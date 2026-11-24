//! La lista de comunicación —canales y directos— contra un servidor de
//! mentira. Cada prueba dice la rotura que atrapa, comprobada rompiendo el
//! código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{CommsSection, CommsState, Config, Conversation, Core, Listener, Locale, NodeKind, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[path = "support/gated.rs"]
mod gated;
use gated::{Gate, GatedProxy};

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
    comms: Mutex<Vec<CommsState>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn comms_changed(&self, state: CommsState) {
        self.comms.lock().unwrap().push(state);
    }
}

impl Recorder {
    /// Si alguna vez se avisó de una lista que cumple eso.
    fn ever(&self, seen: impl Fn(&CommsState) -> bool) -> bool {
        self.comms.lock().unwrap().iter().any(seen)
    }
    fn last(&self) -> Option<CommsState> {
        self.comms.lock().unwrap().last().cloned()
    }
}

struct Rig {
    server: MockServer,
    /// Por donde habla el core: deja pasar todo, y retiene lo que una
    /// prueba ate a una [`Gate`].
    proxy: GatedProxy,
    core: Core,
    recorder: Arc<Recorder>,
    storage: Arc<MemoryStorage>,
}

fn core_for(uri: &str, storage: Arc<MemoryStorage>, recorder: Arc<Recorder>) -> Core {
    let mut config = Config::new(uri);
    config.stream_backoff = Duration::from_millis(40);
    let core = Core::new(config, storage, recorder).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core
}

async fn rig() -> Rig {
    let server = MockServer::start().await;
    let recorder = Arc::new(Recorder::default());
    let storage = Arc::new(MemoryStorage::default());
    let proxy = GatedProxy::start(server.uri());
    let core = core_for(&proxy.uri(), storage.clone(), recorder.clone());
    Rig { server, proxy, core, recorder, storage }
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

fn space(id: &str, org_id: &str, name: &str, kind: Option<&str>) -> Value {
    json!({ "id": id, "orgId": org_id, "name": name, "color": "#888", "kind": kind, "folders": [], "lists": [], "people": [] })
}

fn latest(space: &str, author: &str, name: &str, kind: &str, body: &str) -> Value {
    json!({ "spaceId": space, "authorUserId": author, "authorName": name, "kind": kind, "body": body, "createdAt": "2026-10-15T15:00:00Z" })
}

fn dm(id: &str, org_id: &str, username: &str, unread: i64, body: &str, author: &str) -> Value {
    json!({
        "conversationId": id, "orgId": org_id, "userId": format!("u-{username}"), "username": username, "unread": unread,
        "lastMessageAt": "2026-10-15T15:00:00Z", "lastBody": body, "lastAuthorId": author,
    })
}

/// Dos organizaciones. `o1`: la sala general `g` (que el servidor manda la
/// última), `a` «Producto» y `b` «Diseño». `o2`: `z` «Soporte». Y lo que el
/// servidor contesta de **todas** a la vez: lo sin leer, las últimas líneas
/// y los directos.
async fn mount_two_orgs(server: &MockServer) {
    get("/api/v1/organizations/").respond_with(ok(json!([org("o1"), org("o2")]))).mount(server).await;
    let o1 = json!([space("a", "o1", "Producto", None), space("b", "o1", "Diseño", None), space("g", "o1", "General", Some("general"))]);
    get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(o1)).mount(server).await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2", "Soporte", None)]))).mount(server).await;
    get("/api/v1/chat/unread")
        .respond_with(ok(json!([{ "spaceId": "a", "count": 3 }, { "spaceId": "g", "count": 1 }, { "spaceId": "z", "count": 70 }])))
        .mount(server)
        .await;
}

async fn mount_latest(server: &MockServer, rows: Value) {
    get("/api/v1/chat/latest").respond_with(ok(rows)).mount(server).await;
}

async fn mount_dms(server: &MockServer, rows: Value) {
    get("/api/v1/dm/").respond_with(ok(rows)).mount(server).await;
}

fn both_orgs_latest() -> Value {
    json!([latest("a", "u2", "Luis Mora", "user", "¿sale hoy?"), latest("z", "u7", "Zoe", "user", "de otra organización")])
}

fn both_orgs_dms() -> Value {
    json!([dm("c1", "o1", "luis", 2, "¿vienes?", "u-luis"), dm("c9", "o2", "zoe", 500, "de otra organización", "u-zoe")])
}

/// Un stream que entrega esos eventos una vez, cuando la prueba abre la
/// puerta que devuelve, y se acaba. Con un plazo de llegada, en una máquina
/// cargada los eventos llegaban antes de que la lista estuviera cargada.
async fn mount_events(server: &MockServer, frames: &str) -> Gate {
    let said = Gate::default();
    get("/api/v1/events")
        .respond_with(said.hold(ResponseTemplate::new(200).set_body_raw(frames.as_bytes().to_vec(), "text/event-stream")))
        .up_to_n_times(1)
        .mount(server)
        .await;
    said
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

fn channel_names(state: &CommsState) -> Vec<String> {
    state.channels.iter().map(|channel| channel.name.clone()).collect()
}

fn direct_names(state: &CommsState) -> Vec<String> {
    state.directs.iter().map(|direct| direct.username.clone()).collect()
}

fn channel(id: &str) -> Conversation {
    Conversation::Channel { space_id: id.into() }
}

fn direct(id: &str) -> Conversation {
    Conversation::Direct { conversation_id: id.into() }
}

fn last_body(state: &CommsState, space: &str) -> Option<String> {
    state.channels.iter().find(|channel| channel.space_id == space).and_then(|channel| channel.last.as_ref()).map(|last| last.body.clone())
}

/// Entrar trae los canales y los directos de la organización que se mira, y
/// sólo los de ella.
///
/// El servidor manda lo sin leer, las últimas líneas y los directos de todas
/// las organizaciones. Roturas que atrapa: listar el canal o el directo de
/// otra organización (gente de dos clientes en la misma columna); sumarle a
/// la sección lo sin leer de otra; la sala general en el sitio en que la
/// manda el servidor y no la primera; un canal sin su última línea o sin su
/// cuenta; no avisar a la pantalla; un panel que no coincide con el riel.
#[tokio::test]
async fn the_list_is_the_organizations_and_nobody_elses() {
    let rig = rig().await;
    mount_two_orgs(&rig.server).await;
    mount_latest(&rig.server, both_orgs_latest()).await;
    mount_dms(&rig.server, both_orgs_dms()).await;

    rig.core.open_workspace().await;
    let state = rig.core.comms().state();

    assert_eq!(state.org_id.as_deref(), Some("o1"));
    assert_eq!(channel_names(&state), ["General", "Producto", "Diseño"]);
    assert_eq!(state.channels.iter().map(|channel| channel.unread).collect::<Vec<_>>(), [1, 3, 0]);
    assert_eq!(last_body(&state, "a").as_deref(), Some("¿sale hoy?"));
    assert_eq!(last_body(&state, "b"), None);
    assert_eq!(direct_names(&state), ["luis"]);
    assert_eq!(state.directs[0].unread, 2);
    assert_eq!(rig.recorder.last(), Some(state.clone()), "la pantalla no tiene la última lista");

    // Lo que se pinta: las cuentas de sección son las de esta organización,
    // y juntas dan el número del riel.
    let now = chrono::Utc::now();
    let panel = rig.core.comms().panel("", Locale::Es, &now);
    assert_eq!((panel.channels.badge.as_deref(), panel.directs.badge.as_deref()), (Some("4"), Some("2")));
    assert_eq!(panel.channels.rows[1].preview.as_deref(), Some("Luis Mora: ¿sale hoy?"));
    assert_eq!(rig.core.counters().state().comms, Some(6));
    rig.core.stream().stop();
}

/// Lo propio se dice «Tú», y una línea del sistema no la firma nadie aunque
/// la provocara quien mira: el core sabe quién está dentro.
///
/// Roturas que atrapa: «Tú: Grabación lista» para quien grabó; lo que uno
/// escribió firmado con su nombre como si fuera de otro; «Tú:» en lo de otra
/// persona.
#[tokio::test]
async fn what_i_wrote_is_mine_and_what_cac_wrote_is_nobodys() {
    let rig = rig().await;
    mount_two_orgs(&rig.server).await;
    mount_latest(
        &rig.server,
        json!([
            latest("g", "u1", "Ana Ruiz", "system", "Grabación lista"),
            latest("a", "u1", "Ana Ruiz", "user", "voy yo"),
            latest("b", "u2", "Luis Mora", "user", "vale"),
        ]),
    )
    .await;
    mount_dms(&rig.server, json!([])).await;

    rig.core.open_workspace().await;
    let now = chrono::Utc::now();
    let panel = rig.core.comms().panel("", Locale::Es, &now);
    let previews: Vec<Option<&str>> = panel.channels.rows.iter().map(|row| row.preview.as_deref()).collect();

    assert_eq!(previews, [Some("Grabación lista"), Some("Tú: voy yo"), Some("Luis Mora: vale")]);
    rig.core.stream().stop();
}

/// Cambiar de organización cambia la lista entera, y lo que se eligió en
/// cada una se recuerda en ella, también al volver a arrancar.
///
/// Roturas que atrapa: los canales o los directos de la organización
/// anterior en la nueva; elegir en una organización lo que es de otra;
/// volver a una organización y encontrarse en la sala general en vez de
/// donde se estaba; que lo elegido en una pise lo de la otra; perderlo al
/// cerrar la app; no marcar nada al entrar por primera vez.
#[tokio::test]
async fn each_organization_keeps_its_own_list_and_its_own_place() {
    let rig = rig().await;
    mount_two_orgs(&rig.server).await;
    mount_latest(&rig.server, both_orgs_latest()).await;
    mount_dms(&rig.server, both_orgs_dms()).await;
    rig.core.open_workspace().await;
    let comms = rig.core.comms();

    // Sin haber elegido nada, la sala general.
    assert_eq!(comms.state().selected, Some(channel("g")));
    assert!(comms.select(direct("c1")));
    assert_eq!(comms.state().selected, Some(direct("c1")));
    // Lo de otra organización no se elige desde ésta.
    assert!(!comms.select(channel("z")));
    assert!(!comms.select(direct("c9")));
    assert_eq!(comms.state().selected, Some(direct("c1")));

    assert!(rig.core.switch_org("o2").await);
    let state = comms.state();
    assert_eq!((state.org_id.as_deref(), channel_names(&state), direct_names(&state)), (Some("o2"), vec!["Soporte".to_string()], vec!["zoe".to_string()]));
    assert_eq!(state.channels[0].unread, 70);
    assert_eq!(state.selected, Some(channel("z")), "sin sala general, el primer canal");
    assert!(comms.select(direct("c9")));

    assert!(rig.core.switch_org("o1").await);
    assert_eq!(comms.state().selected, Some(direct("c1")));
    rig.core.stream().stop();

    // Otra vez la app, con lo guardado en esta máquina.
    let again = core_for(&rig.proxy.uri(), rig.storage.clone(), Arc::new(Recorder::default()));
    again.open_workspace().await;
    assert_eq!(again.comms().state().selected, Some(direct("c1")));
    assert!(again.switch_org("o2").await);
    assert_eq!(again.comms().state().selected, Some(direct("c9")));
    again.stream().stop();
}

/// Un espacio que se borra deja de tener canal, y si era el que se tenía
/// delante se cae a la sala general.
///
/// Roturas que atrapa: el canal de un espacio borrado sigue en la lista (y
/// abrirlo da un 404); seguir «en» un canal que ya no existe.
#[tokio::test]
async fn a_deleted_spaces_channel_leaves_the_list() {
    let rig = rig().await;
    get("/api/v1/organizations/").respond_with(ok(json!([org("o1")]))).mount(&rig.server).await;
    let before = json!([space("a", "o1", "Producto", None), space("b", "o1", "Diseño", None), space("g", "o1", "General", Some("general"))]);
    let after = json!([space("b", "o1", "Diseño", None), space("g", "o1", "General", Some("general"))]);
    get("/api/v1/task-spaces/").respond_with(ok(before)).up_to_n_times(1).mount(&rig.server).await;
    get("/api/v1/task-spaces/").respond_with(ok(after)).mount(&rig.server).await;
    Mock::given(method("DELETE")).and(path("/api/v1/task-spaces/a")).respond_with(ok(json!(null))).mount(&rig.server).await;
    mount_latest(&rig.server, json!([latest("a", "u2", "Luis Mora", "user", "¿sale hoy?")])).await;
    mount_dms(&rig.server, json!([])).await;
    rig.core.open_workspace().await;
    let comms = rig.core.comms();
    assert!(comms.select(channel("a")));
    assert_eq!(channel_names(&comms.state()), ["General", "Producto", "Diseño"]);

    rig.core.tree().delete(NodeKind::Space, "a").await.unwrap();

    let state = comms.state();
    assert_eq!(channel_names(&state), ["General", "Diseño"]);
    assert_eq!(state.selected, Some(channel("g")));
    assert_eq!(rig.recorder.last(), Some(state));
    rig.core.stream().stop();
}

/// Un mensaje en un canal mueve su fila en el acto, con lo que trae el
/// evento, y luego se vuelven a pedir las últimas líneas.
///
/// Roturas que atrapa: la fila que no cambia hasta reiniciar; tomar por
/// última una línea que sólo se editó (el resumen de una llamada de ayer
/// subiría como si acabara de llegar); una línea del sistema que llega por
/// evento firmada por quien la provocó; no volver a pedir tras el evento;
/// perder la línea de los demás canales al poner la de uno.
#[tokio::test]
async fn a_channel_message_moves_its_row() {
    let rig = rig().await;
    mount_two_orgs(&rig.server).await;
    // El servidor de mentira no cambia: lo que la fila diga de nuevo sólo
    // puede venir del evento.
    mount_latest(&rig.server, json!([latest("a", "u2", "Luis Mora", "user", "lo de antes"), latest("b", "u2", "Luis Mora", "user", "sin tocar")])).await;
    mount_dms(&rig.server, json!([])).await;
    let frames = concat!(
        "event: chat:message\ndata: {\"spaceId\":\"a\",\"messageId\":\"m9\",\"actorId\":\"u1\",\"kind\":\"system\",\"spaceName\":\"Producto\",",
        "\"authorName\":\"\",\"preview\":\"Grabación lista\",\"mentions\":null,\"repliedTo\":\"\",\"edited\":false}\n\n",
        "event: chat:message\ndata: {\"spaceId\":\"b\",\"messageId\":\"m1\",\"actorId\":\"u2\",\"kind\":\"user\",\"authorName\":\"Luis Mora\",",
        "\"preview\":\"esto ya estaba\",\"mentions\":null,\"repliedTo\":\"\",\"edited\":true}\n\n",
    );
    // Cuando la lista ya esté cargada.
    let said = mount_events(&rig.server, frames).await;

    rig.core.open_workspace().await;
    assert_eq!(last_body(&rig.core.comms().state(), "a").as_deref(), Some("lo de antes"));
    said.open();

    until("la fila del canal enseña lo que trajo el evento", || rig.recorder.ever(|state| last_body(state, "a").as_deref() == Some("Grabación lista"))).await;
    let moved = rig.recorder.comms.lock().unwrap().iter().find(|state| last_body(state, "a").as_deref() == Some("Grabación lista")).cloned().unwrap();
    let line = moved.channels.iter().find(|channel| channel.space_id == "a").and_then(|channel| channel.last.clone()).unwrap();
    assert_eq!((line.kind.as_str(), line.author_user_id.as_str(), line.author_name.as_str()), ("system", "u1", ""));
    assert!(chrono::DateTime::parse_from_rfc3339(&line.created_at).is_ok(), "la línea del evento no lleva una hora que se pueda leer");
    assert_eq!(last_body(&moved, "b").as_deref(), Some("sin tocar"));
    assert!(!rig.recorder.ever(|state| last_body(state, "b").as_deref() == Some("esto ya estaba")), "una línea editada se tomó por la última");

    // Y se vuelve a pedir: una vez al entrar y otra, sola, por los dos eventos.
    until_hits(&rig.server, "/api/v1/chat/latest", 2).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(hits(&rig.server, "/api/v1/chat/latest").await, 2, "una ráfaga pide las últimas líneas una vez");
    assert_eq!(last_body(&rig.core.comms().state(), "a").as_deref(), Some("lo de antes"), "manda lo que dice el servidor");
    rig.core.stream().stop();
}

/// Un directo que llega mueve su fila —el servidor no manda el texto, así
/// que se vuelve a pedir la lista—, y cuando el otro lee, lo propio pasa a
/// leído sin pedir nada.
///
/// Roturas que atrapa: la fila de un directo que no cambia al llegar un
/// mensaje; las palomitas que no pasan a leído hasta recargar; marcar como
/// leída otra conversación; pedir la lista entera por un aviso de lectura.
#[tokio::test]
async fn a_direct_message_and_a_read_receipt_move_their_row() {
    let rig = rig().await;
    mount_two_orgs(&rig.server).await;
    mount_latest(&rig.server, json!([])).await;
    let mine = |id: &str, username: &str| dm(id, "o1", username, 0, "¿quedamos?", "u1");
    get("/api/v1/dm/").respond_with(ok(json!([mine("c1", "luis"), mine("c2", "bea")]))).up_to_n_times(1).mount(&rig.server).await;
    mount_dms(&rig.server, json!([dm("c2", "o1", "bea", 1, "¡sí!", "u-bea"), mine("c1", "luis")])).await;
    // Lee Bea, la segunda de la lista: la primera no se toca.
    let frames = concat!(
        "event: dm:read\ndata: {\"conversationId\":\"c2\",\"userId\":\"u-bea\",\"at\":\"2026-10-15T15:00:05Z\"}\n\n",
        "event: dm:message\ndata: {\"conversationId\":\"c2\",\"messageId\":\"d9\",\"actorId\":\"u-bea\",\"authorName\":\"Bea\"}\n\n",
    );
    let said = mount_events(&rig.server, frames).await;

    rig.core.open_workspace().await;
    let now = chrono::Utc::now();
    let ticks = |core: &Core| core.comms().panel("", Locale::Es, &now).directs.rows.iter().map(|row| (row.name.clone(), row.ticks)).collect::<Vec<_>>();
    assert_eq!(ticks(&rig.core), [("luis".to_string(), Some(false)), ("bea".to_string(), Some(false))]);
    said.open();

    // El aviso de lectura, antes de que la lista se vuelva a pedir.
    until("lo que escribí a Bea pasa a leído, y sólo eso", || {
        rig.recorder.ever(|state| {
            state.directs.iter().map(|direct| direct.other_read_at.as_deref()).collect::<Vec<_>>() == [None, Some("2026-10-15T15:00:05Z")]
        })
    })
    .await;
    assert_eq!(hits(&rig.server, "/api/v1/dm/").await, 1, "un aviso de lectura no pide la lista");

    until("la fila de Bea enseña lo que escribió", || direct_names(&rig.core.comms().state()) == ["bea", "luis"]).await;
    let state = rig.core.comms().state();
    assert_eq!((state.directs[0].last_body.as_deref(), state.directs[0].unread), (Some("¡sí!"), 1));
    assert_eq!(rig.recorder.last(), Some(state));
    rig.core.stream().stop();
}

/// Al volver de una caída se vuelven a pedir las últimas líneas: lo que se
/// dijo mientras no había stream no llegó por evento.
///
/// Rotura que atrapa: la lista con las líneas de antes del corte hasta que
/// alguien escribe otra vez.
#[tokio::test]
async fn a_stream_that_comes_back_asks_for_the_last_lines_again() {
    let rig = rig().await;
    mount_two_orgs(&rig.server).await;
    get("/api/v1/chat/latest").respond_with(ok(json!([latest("a", "u2", "Luis Mora", "user", "antes del corte")]))).up_to_n_times(1).mount(&rig.server).await;
    mount_latest(&rig.server, json!([latest("a", "u2", "Luis Mora", "user", "durante el corte")])).await;
    mount_dms(&rig.server, json!([])).await;
    // Un stream que se acaba, y otro que vuelve.
    let cut = mount_events(&rig.server, ": hola\n\n").await;
    get("/api/v1/events").respond_with(ResponseTemplate::new(200).set_body_raw(b": de vuelta\n\n".to_vec(), "text/event-stream")).mount(&rig.server).await;

    rig.core.open_workspace().await;
    assert_eq!(last_body(&rig.core.comms().state(), "a").as_deref(), Some("antes del corte"));
    cut.open();

    until("la fila enseña lo que se dijo durante el corte", || last_body(&rig.core.comms().state(), "a").as_deref() == Some("durante el corte")).await;
    rig.core.stream().stop();
}

/// Una respuesta que llega cuando ya se cambió de organización no pinta nada
/// de la anterior, ni le quita a la nueva lo suyo.
///
/// Roturas que atrapa: los canales de la organización que se dejó, pintados
/// en la nueva porque su árbol llegó tarde; la lista de la nueva vaciada por
/// esa misma respuesta; la conversación abierta con alguien de la anterior,
/// elegida en la nueva; la gente de la anterior ofrecida en la nueva; el
/// riel sin contar los canales de la nueva (pasaba: `counters` guardaba los
/// espacios de una sola organización, la última en contestar); y el árbol de
/// «Trabajo» con los espacios de la anterior bajo el nombre de la nueva
/// (pasaba también: `Tree::load` pintaba cualquier respuesta).
#[tokio::test]
async fn a_late_answer_does_not_paint_the_organization_that_was_left() {
    let rig = rig().await;
    get("/api/v1/organizations/").respond_with(ok(json!([org("o1"), org("o2")]))).mount(&rig.server).await;
    // Lo de `o1` no contesta hasta que ya se cambió: con un plazo, en una
    // máquina cargada contestaba antes o se pedía después del cambio.
    let late = Gate::default();
    get("/api/v1/task-spaces/")
        .and(query_param("orgId", "o1"))
        .respond_with(late.hold(ok(json!([space("a", "o1", "Producto", None)]))))
        .mount(&rig.server)
        .await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2", "Soporte", None)]))).mount(&rig.server).await;
    mount_latest(&rig.server, both_orgs_latest()).await;
    get("/api/v1/chat/unread").respond_with(ok(json!([{ "spaceId": "a", "count": 3 }, { "spaceId": "z", "count": 5 }]))).mount(&rig.server).await;
    mount_dms(&rig.server, json!([dm("c1", "o1", "luis", 0, "hola", "u-luis"), dm("c9", "o2", "zoe", 0, "hola", "u-zoe")])).await;
    get("/api/v1/users/search")
        .and(query_param("orgId", "o1"))
        .respond_with(late.hold(ok(json!([{ "id": "u2", "username": "luis" }]))))
        .mount(&rig.server)
        .await;
    get("/api/v1/users/search").and(query_param("orgId", "o2")).respond_with(ok(json!([{ "id": "u7", "username": "zoe" }]))).mount(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/dm/open"))
        .respond_with(late.hold(ok(json!({ "id": "c1", "orgId": "o1" }))))
        .mount(&rig.server)
        .await;

    // Todo lo de `o1` en marcha y sin contestar, y se cambia a `o2`.
    let entering = tokio::spawn({
        let core = rig.core.clone();
        async move { core.open_workspace().await }
    });
    until("se sabe en qué organización se está", || rig.core.orgs().state().current_org_id.is_some()).await;
    let people = tokio::spawn({
        let core = rig.core.clone();
        async move { core.comms().load_people().await }
    });
    let opening = tokio::spawn({
        let core = rig.core.clone();
        async move { core.comms().open_with("u2").await }
    });
    until("lo de o1 va en camino", || late.held() == 3).await;
    assert!(rig.core.switch_org("o2").await);
    rig.core.comms().load_people().await;
    let comms = rig.core.comms();
    assert_eq!(channel_names(&comms.state()), ["Soporte"]);

    late.open();
    entering.await.unwrap();
    people.await.unwrap();
    assert_eq!(opening.await.unwrap().unwrap(), "c1");

    let state = comms.state();
    assert_eq!((state.org_id.as_deref(), channel_names(&state), direct_names(&state)), (Some("o2"), vec!["Soporte".to_string()], vec!["zoe".to_string()]));
    assert_eq!(state.selected, Some(channel("z")));
    assert!(!rig.recorder.ever(|state| state.org_id.as_deref() == Some("o2") && channel_names(state).contains(&"Producto".to_string())));
    assert_eq!(comms.people("").into_iter().map(|person| person.username).collect::<Vec<_>>(), ["zoe"]);
    // Y el número del riel sigue siendo el de la lista que se ve.
    assert_eq!(state.channels[0].unread, 5);
    assert_eq!(rig.core.counters().state().comms, Some(5), "el riel no cuenta lo que la lista enseña");
    // Ni el árbol de «Trabajo» enseña los espacios de la que se dejó.
    let tree = rig.core.tree().state();
    assert_eq!((tree.org_id.as_deref(), tree.spaces.iter().map(|space| space.name.as_str()).collect::<Vec<_>>()), (Some("o2"), vec!["Soporte"]));

    // Y al volver, lo que se abrió allí es lo que se tiene delante.
    assert!(rig.core.switch_org("o1").await);
    assert_eq!(comms.state().selected, Some(direct("c1")));
    rig.core.stream().stop();
}

/// Lo mismo cuando lo que llega tarde es un fallo: el árbol de la
/// organización nueva no se queda con el error de la que se dejó.
///
/// Rotura que atrapa: «no se pudo cargar» sobre un árbol que cargó bien,
/// porque el de la organización anterior falló después.
#[tokio::test]
async fn a_late_failure_does_not_mark_the_new_organizations_tree() {
    let rig = rig().await;
    get("/api/v1/organizations/").respond_with(ok(json!([org("o1"), org("o2")]))).mount(&rig.server).await;
    let late = Gate::default();
    get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(late.hold(ResponseTemplate::new(500))).mount(&rig.server).await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2", "Soporte", None)]))).mount(&rig.server).await;
    let entering = tokio::spawn({
        let core = rig.core.clone();
        async move { core.open_workspace().await }
    });
    until("el árbol de o1 va en camino", || late.held() == 1).await;
    assert!(rig.core.switch_org("o2").await);
    late.open();
    entering.await.unwrap();

    let tree = rig.core.tree().state();
    assert_eq!((tree.org_id.as_deref(), tree.error, tree.loading), (Some("o2"), None, false));
    assert_eq!(channel_names(&rig.core.comms().state()), ["Soporte"]);
    rig.core.stream().stop();
}

/// «Nueva conversación»: se ofrece la gente de la organización menos uno
/// mismo, y abrir con alguien deja esa conversación delante, exista ya o no.
///
/// Roturas que atrapa: pedir la gente sin decir de qué organización (el
/// servidor contestaría con la de todas); ofrecerse a uno mismo; abrir sin
/// decir en qué organización, o con otra persona; no elegir la conversación
/// recién abierta; elegirla sin que esté en la lista (no habría fila que
/// marcar); que abrir dos veces con la misma persona deje dos filas; un
/// rechazo del servidor que no se dice.
#[tokio::test]
async fn opening_a_conversation_puts_it_in_front() {
    let rig = rig().await;
    mount_two_orgs(&rig.server).await;
    mount_latest(&rig.server, json!([])).await;
    get("/api/v1/dm/").respond_with(ok(json!([dm("c1", "o1", "luis", 0, "hola", "u-luis")]))).up_to_n_times(1).mount(&rig.server).await;
    mount_dms(&rig.server, json!([dm("c1", "o1", "luis", 0, "hola", "u-luis"), json!({ "conversationId": "c2", "orgId": "o1", "userId": "u3", "username": "bea", "unread": 0 })])).await;
    get("/api/v1/users/search")
        .and(query_param("orgId", "o1"))
        .and(query_param("q", ""))
        .respond_with(ok(json!([
            { "id": "u1", "username": "ana", "name": "Ana Ruiz" },
            { "id": "u2", "username": "luis", "name": "Luis Mora", "lastSeenAt": "2026-10-15T15:00:00Z" },
            { "id": "u3", "username": "bea" },
        ])))
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/dm/open"))
        .and(body_json(json!({ "orgId": "o1", "userId": "u3" })))
        .respond_with(ok(json!({ "id": "c2", "orgId": "o1" })))
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/dm/open"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "success": false, "message": "You don't share that organization", "error": "not-colleagues" })))
        .mount(&rig.server)
        .await;
    rig.core.open_workspace().await;
    let comms = rig.core.comms();

    comms.load_people().await;
    assert_eq!(comms.people("").into_iter().map(|person| person.id).collect::<Vec<_>>(), ["u2", "u3"]);
    assert_eq!(comms.people("mora").into_iter().map(|person| person.id).collect::<Vec<_>>(), ["u2"]);

    assert_eq!(comms.open_with("u3").await.unwrap(), "c2");
    let state = comms.state();
    assert_eq!(direct_names(&state), ["luis", "bea"]);
    assert_eq!(state.selected, Some(direct("c2")));

    assert_eq!(comms.open_with("u3").await.unwrap(), "c2");
    assert_eq!(direct_names(&comms.state()), ["luis", "bea"]);

    // Con quien el servidor no deja: se dice, y lo que había delante se queda.
    assert!(comms.open_with("u9").await.is_err());
    assert_eq!(comms.state().selected, Some(direct("c2")));
    rig.core.stream().stop();
}

/// La sala general se ofrece sólo a quien administra la organización que se
/// mira: el core sabe el papel de quien está dentro en cada una.
///
/// Roturas que atrapa: no ofrecérsela a quien puede abrirla; ofrecérsela en
/// una organización en la que sólo es miembro (el servidor se lo negaría).
#[tokio::test]
async fn the_general_room_is_offered_where_i_administer() {
    let rig = rig().await;
    let mut admin = org("o1");
    admin["role"] = json!("admin");
    get("/api/v1/organizations/").respond_with(ok(json!([admin, org("o2")]))).mount(&rig.server).await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(json!([space("a", "o1", "Producto", None)]))).mount(&rig.server).await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2", "Soporte", None)]))).mount(&rig.server).await;
    mount_latest(&rig.server, json!([])).await;
    mount_dms(&rig.server, json!([])).await;
    rig.core.open_workspace().await;
    let now = chrono::Utc::now();

    assert!(rig.core.comms().panel("", Locale::Es, &now).offers_general);
    assert!(rig.core.switch_org("o2").await);
    assert!(!rig.core.comms().panel("", Locale::Es, &now).offers_general);
    rig.core.stream().stop();
}

/// Entrar y salir de comunicación, plegar una sección y salir de la cuenta.
///
/// Roturas que atrapa: la pantalla que no se entera de que se entró a
/// comunicación (el panel seguiría siendo el árbol); lo plegado en una
/// organización plegado también en otra, o perdido al reiniciar; las líneas
/// y la gente de una persona a la vista de la siguiente que entra.
#[tokio::test]
async fn entering_folding_and_signing_out() {
    let rig = rig().await;
    mount_two_orgs(&rig.server).await;
    mount_latest(&rig.server, both_orgs_latest()).await;
    mount_dms(&rig.server, both_orgs_dms()).await;
    get("/api/v1/users/search").respond_with(ok(json!([{ "id": "u2", "username": "luis" }]))).mount(&rig.server).await;
    Mock::given(method("POST")).and(path("/api/v1/auth/logout")).respond_with(ok(json!(null))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    let comms = rig.core.comms();
    assert!(!comms.state().on_screen);

    comms.enter();
    assert_eq!(rig.recorder.last().map(|state| state.on_screen), Some(true));
    comms.toggle_section(CommsSection::Channels);
    let state = comms.state();
    assert!(state.channels_folded && !state.directs_folded);
    assert_eq!(rig.recorder.last(), Some(state));

    assert!(rig.core.switch_org("o2").await);
    assert!(!comms.state().channels_folded, "lo plegado en una organización no lo está en otra");
    comms.toggle_section(CommsSection::Directs);
    rig.core.stream().stop();

    let again = core_for(&rig.proxy.uri(), rig.storage.clone(), Arc::new(Recorder::default()));
    again.open_workspace().await;
    let state = again.comms().state();
    assert_eq!((state.org_id.as_deref(), state.channels_folded, state.directs_folded), (Some("o2"), false, true));
    assert!(again.switch_org("o1").await);
    assert!(again.comms().state().channels_folded && !again.comms().state().directs_folded);
    again.comms().toggle_section(CommsSection::Channels);
    assert!(!again.comms().state().channels_folded);
    again.stream().stop();

    comms.load_people().await;
    assert_eq!(comms.people("").len(), 1);
    comms.leave();
    assert_eq!(rig.recorder.last().map(|state| state.on_screen), Some(false));
    comms.enter();
    rig.core.logout().await;
    let state = comms.state();
    assert!(!state.on_screen && state.channels.is_empty() && state.directs.is_empty());
    assert!(comms.people("").is_empty());
    assert_eq!(rig.recorder.last(), Some(state));
}
