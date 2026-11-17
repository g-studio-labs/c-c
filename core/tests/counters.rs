//! Los contadores de la campana y del riel, contra un servidor de mentira.
//! Cada prueba dice la rotura que atrapa, comprobada rompiendo el código
//! (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Config, Core, CountersState, Listener, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct MemoryStorage(Mutex<HashMap<String, String>>);

impl Storage for MemoryStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn remove(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}

#[derive(Default)]
struct Recorder {
    counters: Mutex<Vec<CountersState>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn counters_changed(&self, state: CountersState) {
        self.counters.lock().unwrap().push(state);
    }
}

struct Rig {
    server: MockServer,
    core: Core,
    recorder: Arc<Recorder>,
}

async fn rig() -> Rig {
    let server = MockServer::start().await;
    let recorder = Arc::new(Recorder::default());
    let mut config = Config::new(server.uri());
    config.stream_backoff = Duration::from_millis(40);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    Rig { server, core, recorder }
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn get(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path(route.to_string()))
}

fn org(id: &str) -> Value {
    json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" })
}

fn space(id: &str, org_id: &str, kind: Option<&str>) -> Value {
    json!({ "id": id, "orgId": org_id, "name": format!("Espacio {id}"), "color": "#888", "kind": kind, "folders": [], "lists": [], "people": [] })
}

async fn mount_orgs(server: &MockServer, ids: &[&str]) {
    get("/api/v1/organizations/").respond_with(ok(ids.iter().map(|id| org(id)).collect())).mount(server).await;
}

async fn mount_tree(server: &MockServer, org_id: &str, spaces: Vec<Value>) {
    get("/api/v1/task-spaces/").and(query_param("orgId", org_id)).respond_with(ok(Value::Array(spaces))).mount(server).await;
}

/// La campana de una organización: sólo contesta si se le pregunta por ella.
async fn mount_bell(server: &MockServer, org_id: &str, unread: i64) {
    get("/api/v1/notifications/")
        .and(query_param("orgId", org_id))
        .respond_with(ok(json!({ "items": [], "unread": unread })))
        .mount(server)
        .await;
}

/// Lo sin leer de cada canal, de todas las organizaciones, como lo manda el
/// servidor.
async fn mount_channels(server: &MockServer, rows: &[(&str, i64)]) {
    let rows: Vec<Value> = rows.iter().map(|(space, count)| json!({ "spaceId": space, "count": count })).collect();
    get("/api/v1/chat/unread").respond_with(ok(Value::Array(rows))).mount(server).await;
}

async fn mount_dms(server: &MockServer, rows: &[(&str, i64)]) {
    let rows: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(index, (org_id, unread))| {
            json!({ "conversationId": format!("c{index}"), "orgId": org_id, "userId": "u2", "username": "luis", "unread": unread })
        })
        .collect();
    get("/api/v1/dm/").respond_with(ok(Value::Array(rows))).mount(server).await;
}

async fn mount_servers(server: &MockServer, rows: &[(&str, &str)]) {
    let rows: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(index, (org_id, status))| {
            json!({
                "id": format!("s{index}"), "orgId": org_id, "name": "vps", "host": "203.0.113.7", "sshPort": 22, "sshUser": "deploy",
                "type": "docker-swarm", "agentPort": 8080, "status": status, "hasAgentToken": true, "agentVersion": 2,
            })
        })
        .collect();
    get("/api/v1/servers/").respond_with(ok(Value::Array(rows))).mount(server).await;
}

async fn mount_invitations(server: &MockServer, pending: usize) {
    let rows: Vec<Value> = (0..pending).map(|index| json!({ "id": format!("i{index}"), "orgId": "o9", "orgName": "Otra" })).collect();
    get("/api/v1/invitations/").respond_with(ok(Value::Array(rows))).mount(server).await;
}

/// Una organización `o1` con un espacio `a`, y de todo `each`: avisos, un
/// canal, un directo, servidores caídos e invitaciones.
async fn mount_all(server: &MockServer, bell: i64, channel: i64, dm: i64, offline: usize, invitations: usize) {
    mount_orgs(server, &["o1"]).await;
    mount_tree(server, "o1", vec![space("a", "o1", None)]).await;
    mount_bell(server, "o1", bell).await;
    mount_channels(server, &[("a", channel)]).await;
    mount_dms(server, &[("o1", dm)]).await;
    let down: Vec<(&str, &str)> = (0..offline).map(|_| ("o1", "offline")).collect();
    mount_servers(server, &down).await;
    mount_invitations(server, invitations).await;
}

fn counts(state: &CountersState) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    (state.notifications, state.comms, state.servers, state.invitations)
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..300 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no llegó a pasar: {what}");
}

async fn hits(server: &MockServer, route: &str) -> usize {
    server.received_requests().await.unwrap().iter().filter(|request| request.url.path() == route).count()
}

async fn until_hits(server: &MockServer, route: &str, at_least: usize) {
    for _ in 0..300 {
        if hits(server, route).await >= at_least {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{route} no llegó a pedirse {at_least} veces");
}

/// Un stream que entrega esos eventos una vez y se acaba; lo que venga
/// después no encuentra stream, así que no hay «volver de una caída».
async fn mount_events(server: &MockServer, frames: &str) {
    get("/api/v1/events")
        .respond_with(ResponseTemplate::new(200).set_body_raw(frames.as_bytes().to_vec(), "text/event-stream").set_delay(Duration::from_millis(20)))
        .up_to_n_times(1)
        .mount(server)
        .await;
}

/// Entrar cuenta lo de la organización que se mira, y sólo lo de ella.
///
/// El servidor manda canales, directos y servidores de todas las
/// organizaciones (de toda la plataforma a un superadmin). Roturas que
/// atrapa: sumar lo de otra organización (un mensaje de otro cliente
/// encendía el número y al entrar no había nada que leer); no contar la sala
/// general, que es un canal aunque el árbol no la pinte; contar como caído un
/// servidor en línea, o no contar uno pendiente; no contar los directos, o
/// los canales; no avisar a la pantalla.
#[tokio::test]
async fn entering_counts_what_belongs_to_the_organization_on_screen() {
    let rig = rig().await;
    mount_orgs(&rig.server, &["o1", "o2"]).await;
    mount_tree(&rig.server, "o1", vec![space("a", "o1", None), space("g", "o1", Some("general"))]).await;
    mount_bell(&rig.server, "o1", 4).await;
    mount_bell(&rig.server, "o2", 50).await;
    mount_channels(&rig.server, &[("a", 3), ("g", 2), ("z", 70)]).await;
    mount_dms(&rig.server, &[("o1", 1), ("o2", 500)]).await;
    mount_servers(&rig.server, &[("o1", "online"), ("o1", "offline"), ("o1", "pending"), ("o2", "offline")]).await;
    mount_invitations(&rig.server, 2).await;

    rig.core.open_workspace().await;
    rig.core.stream().stop();

    let state = rig.core.counters().state();
    assert_eq!(state.org_id.as_deref(), Some("o1"));
    assert_eq!(state.notifications, Some(4), "la campana");
    assert_eq!(state.comms, Some(6), "tres del canal, dos de la sala general y un directo");
    assert_eq!(state.servers, Some(2), "el caído y el pendiente");
    assert_eq!(state.invitations, Some(2));
    assert_eq!(rig.recorder.counters.lock().unwrap().last(), Some(&state), "la pantalla no se enteró");
}

/// A la campana no se le pregunta nunca sin organización: contestaría con
/// los avisos de todas. Y quien no es de ninguna ve sus invitaciones, que es
/// lo único que tiene.
///
/// Roturas que atrapa: preguntar con `orgId` vacío o sin él; no enseñarle
/// las invitaciones a quien acaban de invitar; pintar un cero donde no hay
/// organización de la que contar.
#[tokio::test]
async fn without_an_organization_only_the_invitations_are_asked() {
    let rig = rig().await;
    mount_orgs(&rig.server, &[]).await;
    get("/api/v1/notifications/").respond_with(ok(json!({ "items": [], "unread": 31 }))).mount(&rig.server).await;
    mount_channels(&rig.server, &[("a", 3)]).await;
    mount_dms(&rig.server, &[("o1", 1)]).await;
    mount_servers(&rig.server, &[("o1", "offline")]).await;
    mount_invitations(&rig.server, 1).await;

    rig.core.open_workspace().await;
    rig.core.stream().stop();
    rig.core.counters().load(Some(String::new())).await;

    assert_eq!(counts(&rig.core.counters().state()), (None, None, None, Some(1)));
    assert_eq!(hits(&rig.server, "/api/v1/notifications/").await, 0, "se preguntó a la campana sin organización");
}

/// Con organización, la campana se pregunta siempre con ella.
///
/// Roturas que atrapa: una petición sin `orgId` colada entre las buenas (la
/// de un refresco, la de un evento), que traería la cuenta de todas; traerse
/// una página entera de avisos para leer un número.
#[tokio::test]
async fn the_bell_is_always_asked_for_an_organization() {
    let rig = rig().await;
    mount_all(&rig.server, 1, 1, 1, 1, 1).await;
    mount_events(&rig.server, "event: task:assigned\ndata: {}\n\n").await;

    rig.core.open_workspace().await;
    rig.core.counters().refresh().await;
    until_hits(&rig.server, "/api/v1/notifications/", 3).await;
    rig.core.stream().stop();

    let asked = rig.server.received_requests().await.unwrap();
    let bell: Vec<_> = asked.iter().filter(|request| request.url.path() == "/api/v1/notifications/").collect();
    assert!(bell.iter().all(|request| request.url.query_pairs().any(|(key, value)| key == "orgId" && value == "o1")));
    // Y sólo la cuenta: sin `limit` el servidor manda cincuenta filas de una
    // bandeja que aquí no se pinta.
    assert!(bell.iter().all(|request| request.url.query_pairs().any(|(key, value)| key == "limit" && value == "1")));
}

/// Lo cargado con uno de cada; el servidor pasa a tener más de todo, y llega
/// ese evento. Devuelve lo que la pantalla acaba viendo.
async fn after_event(event: &str, data: &str) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    let rig = rig().await;
    mount_all(&rig.server, 1, 1, 1, 1, 1).await;
    rig.core.orgs().load().await;
    rig.core.tree().load("o1").await;
    rig.core.counters().load(Some("o1".into())).await;
    assert_eq!(counts(&rig.core.counters().state()), (Some(1), Some(2), Some(1), Some(1)));

    rig.server.reset().await;
    mount_all(&rig.server, 5, 10, 100, 3, 4).await;
    mount_events(&rig.server, &format!("event: {event}\ndata: {data}\n\n")).await;
    rig.core.stream().start();
    // El respiro de antes de volver a contar, y de sobra para que llegue.
    tokio::time::sleep(Duration::from_millis(900)).await;
    rig.core.stream().stop();
    counts(&rig.core.counters().state())
}

/// Cada evento vuelve a contar lo que mueve, y nada más.
///
/// Roturas que atrapa: un aviso nuevo que no se ve en la campana hasta
/// reiniciar; un mensaje de canal que no mueve la comunicación, o que
/// vuelve a contar los directos en vez de los canales (y al revés); una
/// invitación aceptada en otro sitio que sigue contando; volver a
/// preguntarlo todo por cualquier evento.
#[tokio::test]
async fn each_event_recounts_what_it_moves() {
    // Lo que no cambia se queda en lo cargado: 1 aviso, 1 + 1 de
    // comunicación, 1 servidor, 1 invitación.
    assert_eq!(after_event("task:assigned", "{}").await, (Some(5), Some(2), Some(1), Some(1)), "un aviso");
    assert_eq!(after_event("chat:message", "{}").await, (Some(5), Some(11), Some(1), Some(1)), "un mensaje de canal");
    assert_eq!(after_event("dm:message", "{}").await, (Some(5), Some(101), Some(1), Some(1)), "un directo");
    assert_eq!(after_event("org:membership", "{}").await, (Some(1), Some(2), Some(1), Some(4)), "una membresía");
    assert_eq!(after_event("ci:run", r#"{"status":"completed"}"#).await, (Some(5), Some(2), Some(1), Some(1)), "un run que acaba");
    assert_eq!(after_event("task:move", "{}").await, (Some(1), Some(2), Some(1), Some(1)), "mover una tarjeta no cuenta");
}

/// Diez mensajes seguidos son una sola cuenta nueva.
///
/// Rotura que atrapa: una petición por evento para llegar al mismo número.
#[tokio::test]
async fn a_burst_of_messages_recounts_once() {
    let rig = rig().await;
    mount_all(&rig.server, 1, 1, 1, 1, 1).await;
    rig.core.counters().load(Some("o1".into())).await;
    let before = hits(&rig.server, "/api/v1/chat/unread").await;
    mount_events(&rig.server, &"event: chat:message\ndata: {}\n\n".repeat(10)).await;

    rig.core.stream().start();
    tokio::time::sleep(Duration::from_millis(900)).await;
    rig.core.stream().stop();

    assert_eq!(hits(&rig.server, "/api/v1/chat/unread").await, before + 1);
}

/// Cuando el stream vuelve de una caída se cuenta todo otra vez: lo que pasó
/// mientras no había stream no trajo su evento.
///
/// Rotura que atrapa: volver a conectar con los números de antes del corte,
/// también los servidores, que no tienen evento que los mueva.
#[tokio::test]
async fn a_stream_that_comes_back_recounts_everything() {
    let rig = rig().await;
    mount_all(&rig.server, 1, 1, 1, 1, 1).await;
    rig.core.orgs().load().await;
    rig.core.tree().load("o1").await;
    rig.core.counters().load(Some("o1".into())).await;
    rig.server.reset().await;
    mount_all(&rig.server, 5, 10, 100, 3, 4).await;
    // Un cuerpo que se acaba: para el core, un stream que se cortó y vuelve.
    get("/api/v1/events")
        .respond_with(ResponseTemplate::new(200).set_body_raw(Vec::new(), "text/event-stream"))
        .mount(&rig.server)
        .await;

    rig.core.stream().start();
    until("se vuelve a contar todo", || counts(&rig.core.counters().state()) == (Some(5), Some(110), Some(3), Some(4))).await;
    rig.core.stream().stop();
}

/// Cambiar de organización cuenta lo de la nueva.
///
/// Roturas que atrapa: seguir enseñando la campana de la anterior; contar
/// los canales con los espacios de la anterior; contar los directos o los
/// servidores de la que se dejó.
#[tokio::test]
async fn switching_organization_counts_for_the_new_one() {
    let rig = rig().await;
    mount_orgs(&rig.server, &["o1", "o2"]).await;
    mount_tree(&rig.server, "o1", vec![space("a", "o1", None)]).await;
    mount_tree(&rig.server, "o2", vec![space("z", "o2", None)]).await;
    mount_bell(&rig.server, "o1", 4).await;
    mount_bell(&rig.server, "o2", 8).await;
    mount_channels(&rig.server, &[("a", 3), ("z", 70)]).await;
    mount_dms(&rig.server, &[("o1", 1), ("o2", 500)]).await;
    mount_servers(&rig.server, &[("o1", "offline"), ("o2", "offline"), ("o2", "pending"), ("o2", "online")]).await;
    mount_invitations(&rig.server, 2).await;
    rig.core.open_workspace().await;
    rig.core.stream().stop();
    assert_eq!(counts(&rig.core.counters().state()), (Some(4), Some(4), Some(1), Some(2)));

    assert!(rig.core.switch_org("o2").await);

    let state = rig.core.counters().state();
    assert_eq!(state.org_id.as_deref(), Some("o2"));
    assert_eq!(counts(&state), (Some(8), Some(570), Some(2), Some(2)));
}

/// Si al cambiar de organización la cuenta nueva falla, no queda la de la
/// anterior: no queda nada.
///
/// Roturas que atrapa: la campana de un cliente pintada sobre otro; contar
/// para la organización nueva con listas de antes de entrar en ella, que ya
/// no se sabe si siguen siendo verdad. Las invitaciones sí se quedan: son de
/// la persona, no de la organización.
#[tokio::test]
async fn a_failed_count_after_switching_does_not_show_the_previous_one() {
    let rig = rig().await;
    mount_orgs(&rig.server, &["o1", "o2"]).await;
    mount_tree(&rig.server, "o1", vec![space("a", "o1", None)]).await;
    mount_bell(&rig.server, "o1", 4).await;
    mount_channels(&rig.server, &[("a", 3), ("z", 70)]).await;
    mount_dms(&rig.server, &[("o1", 1), ("o2", 500)]).await;
    mount_servers(&rig.server, &[("o1", "offline"), ("o2", "offline")]).await;
    mount_invitations(&rig.server, 2).await;
    rig.core.open_workspace().await;
    rig.core.stream().stop();
    assert_eq!(counts(&rig.core.counters().state()), (Some(4), Some(4), Some(1), Some(2)));
    // El servidor deja de contestar a todo menos a las organizaciones y al
    // árbol: lo que ya se había traído habla también de la organización
    // nueva (70 en su canal, 500 en sus directos, un servidor caído).
    rig.server.reset().await;
    mount_orgs(&rig.server, &["o1", "o2"]).await;
    mount_tree(&rig.server, "o2", vec![space("z", "o2", None)]).await;

    assert!(rig.core.switch_org("o2").await);

    let state = rig.core.counters().state();
    assert_eq!(state.org_id.as_deref(), Some("o2"));
    assert_eq!(counts(&state), (None, None, None, Some(2)));
    assert_eq!(rig.recorder.counters.lock().unwrap().last(), Some(&state), "la pantalla sigue pintando los de antes");
}

/// Una cuenta que no se pudo cargar no es un cero: no hay número.
///
/// Rotura que atrapa: pintar «0 servidores caídos» porque el servidor no
/// contestó, o porque contestó con otra forma; dar por bueno un cuerpo que
/// dice que falló. Las demás cuentas no dependen de la que falla.
#[tokio::test]
async fn a_count_that_could_not_be_loaded_is_not_a_zero() {
    let rig = rig().await;
    get("/api/v1/notifications/").respond_with(ResponseTemplate::new(500)).mount(&rig.server).await;
    // Otra forma: una lista de textos donde iban filas.
    get("/api/v1/chat/unread").respond_with(ok(json!(["a", "b"]))).mount(&rig.server).await;
    get("/api/v1/dm/").respond_with(ok(json!([{ "conversationId": "c1" }]))).mount(&rig.server).await;
    get("/api/v1/servers/")
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": false, "error": "boom" })))
        .mount(&rig.server)
        .await;
    mount_invitations(&rig.server, 3).await;
    rig.core.tree().load("o1").await;

    rig.core.counters().load(Some("o1".into())).await;

    assert_eq!(counts(&rig.core.counters().state()), (None, None, None, Some(3)));
}

/// Una de las dos mitades de la comunicación basta para decir lo que se
/// sabe.
///
/// Rotura que atrapa: callar tres mensajes de canal porque la lista de
/// directos no cargó (o al revés).
#[tokio::test]
async fn half_of_comms_is_still_said() {
    let rig = rig().await;
    mount_tree(&rig.server, "o1", vec![space("a", "o1", None)]).await;
    mount_channels(&rig.server, &[("a", 3)]).await;
    rig.core.tree().load("o1").await;
    rig.core.counters().load(Some("o1".into())).await;
    assert_eq!(rig.core.counters().state().comms, Some(3), "sin directos");

    let rig = self::rig().await;
    mount_dms(&rig.server, &[("o1", 2)]).await;
    rig.core.counters().load(Some("o1".into())).await;
    assert_eq!(rig.core.counters().state().comms, Some(2), "sin canales");
}

/// Los canales se cuentan con los espacios de la organización que se mira, y
/// mientras no se sepa cuáles son no se cuentan.
///
/// Rotura que atrapa: al cambiar de organización, contar los canales con el
/// árbol de la anterior hasta que llega el nuevo (o para siempre, si falla).
#[tokio::test]
async fn channels_are_not_counted_with_another_organizations_spaces() {
    let rig = rig().await;
    mount_tree(&rig.server, "o1", vec![space("a", "o1", None)]).await;
    mount_channels(&rig.server, &[("a", 3)]).await;
    rig.core.tree().load("o1").await;
    rig.core.counters().load(Some("o1".into())).await;
    assert_eq!(rig.core.counters().state().comms, Some(3));

    rig.core.counters().load(Some("o2".into())).await;

    assert_eq!(rig.core.counters().state().comms, None);
}

/// Una lista vacía sí es un cero: Go no escribe `data` cuando no hay
/// ninguno, y eso es una respuesta buena.
///
/// Rotura que atrapa: tomar por fallo «no tienes servidores» y dejar de
/// saber que no hay ninguno caído; o que un contador con número no vuelva a
/// cero cuando ya no queda nada.
#[tokio::test]
async fn an_empty_list_is_a_zero() {
    let rig = rig().await;
    mount_all(&rig.server, 1, 1, 1, 1, 1).await;
    rig.core.tree().load("o1").await;
    rig.core.counters().load(Some("o1".into())).await;
    assert_eq!(counts(&rig.core.counters().state()), (Some(1), Some(2), Some(1), Some(1)));
    rig.server.reset().await;
    let nothing = || ResponseTemplate::new(200).set_body_json(json!({ "success": true }));
    for route in ["/api/v1/chat/unread", "/api/v1/dm/", "/api/v1/servers/", "/api/v1/invitations/"] {
        get(route).respond_with(nothing()).mount(&rig.server).await;
    }
    mount_bell(&rig.server, "o1", 0).await;

    rig.core.counters().refresh().await;

    assert_eq!(counts(&rig.core.counters().state()), (Some(0), Some(0), Some(0), Some(0)));
}

/// Volver a contar y que falle deja lo último que se supo.
///
/// Rotura que atrapa: que un parpadeo de la red apague todos los números de
/// la organización que se está mirando.
#[tokio::test]
async fn a_recount_that_fails_keeps_what_was_known() {
    let rig = rig().await;
    mount_all(&rig.server, 4, 3, 1, 2, 2).await;
    rig.core.tree().load("o1").await;
    rig.core.counters().load(Some("o1".into())).await;
    rig.server.reset().await;

    rig.core.counters().refresh().await;
    rig.core.counters().load(Some("o1".into())).await;

    assert_eq!(counts(&rig.core.counters().state()), (Some(4), Some(4), Some(2), Some(2)));
}

/// La campana de una organización que ya no es la que se mira no se pone.
///
/// Rotura que atrapa: cambiar de organización deprisa y quedarse con la
/// cuenta de la primera, que contestó más tarde.
#[tokio::test]
async fn a_late_answer_for_another_organization_is_dropped() {
    let rig = rig().await;
    get("/api/v1/notifications/")
        .and(query_param("orgId", "o1"))
        .respond_with(ok(json!({ "items": [], "unread": 4 })).set_delay(Duration::from_millis(300)))
        .mount(&rig.server)
        .await;
    mount_bell(&rig.server, "o2", 8).await;

    let slow = {
        let core = rig.core.clone();
        tokio::spawn(async move { core.counters().load(Some("o1".into())).await })
    };
    tokio::time::sleep(Duration::from_millis(60)).await;
    rig.core.counters().load(Some("o2".into())).await;
    slow.await.unwrap();

    assert_eq!(rig.core.counters().state().notifications, Some(8));
}

/// Al salir, los números se van con quien sale.
///
/// Rotura que atrapa: que quien entre después vea, hasta que carguen los
/// suyos, los avisos y las invitaciones de la persona anterior.
#[tokio::test]
async fn signing_out_takes_the_counts_away() {
    let rig = rig().await;
    mount_all(&rig.server, 4, 3, 1, 2, 2).await;
    rig.core.tree().load("o1").await;
    rig.core.counters().load(Some("o1".into())).await;
    assert_eq!(rig.core.counters().state().invitations, Some(2));

    rig.core.logout().await;

    assert_eq!(rig.core.counters().state(), CountersState::default());
    assert_eq!(rig.recorder.counters.lock().unwrap().last(), Some(&CountersState::default()));
}
