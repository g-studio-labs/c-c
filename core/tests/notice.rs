//! Qué mensajes se avisan, de punta a punta: el stream trae el evento, la
//! recarga dice si era nuevo, y la pantalla recibe el aviso. Contra un
//! servidor de mentira. Cada prueba dice la rotura que atrapa, comprobada
//! rompiendo el código (`/mutar`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Announcement, Config, Conversation, Core, Listener, Locale, NoticeKind, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

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
    announced: Mutex<Vec<Announcement>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn announce(&self, announcement: Announcement) {
        self.announced.lock().unwrap().push(announcement);
    }
}

impl Recorder {
    fn announced(&self) -> Vec<Announcement> {
        self.announced.lock().unwrap().clone()
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
    let mut config = Config::new(&server.uri());
    config.stream_backoff = Duration::from_millis(40);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    Rig { server, core, recorder }
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn get(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path(route.to_string()))
}

/// Dos organizaciones: `o1`, la que se mira al entrar, con dos canales, `a`
/// «Producto» y `b` «Diseño»; y `o2` con `z` «Soporte». Los tres seguidos.
async fn mount_org(server: &MockServer) {
    let org = |id: &str| json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": "member", "memberCount": 3, "createdAt": "2026-06-09T12:00:00Z" });
    get("/api/v1/organizations/").respond_with(ok(json!([org("o1"), org("o2")]))).mount(server).await;
    let space = |id: &str, org: &str, name: &str| json!({ "id": id, "orgId": org, "name": name, "color": "#888", "kind": null, "folders": [], "lists": [], "people": [] });
    get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(json!([space("a", "o1", "Producto"), space("b", "o1", "Diseño")]))).mount(server).await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2", "Soporte")]))).mount(server).await;
    get("/api/v1/chat/unread").respond_with(ok(json!([]))).mount(server).await;
}

/// Lo que el servidor contesta **ahora**: la prueba lo cambia cuando quiere.
/// Así da igual cuántas veces se pregunte (cada vuelta del stream vuelve a
/// pedirlo todo).
#[derive(Clone)]
struct Live(Arc<Mutex<Value>>, Duration);

impl Live {
    fn new(value: Value) -> Self {
        Live(Arc::new(Mutex::new(value)), Duration::ZERO)
    }
    /// Uno que lee lo que hay al llegar la petición y contesta tarde: su
    /// respuesta puede llegar después de que haya cambiado.
    fn slow(value: Value, delay: Duration) -> Self {
        Live(Arc::new(Mutex::new(value)), delay)
    }
    fn set(&self, value: Value) {
        *self.0.lock().unwrap() = value;
    }
}

impl Respond for Live {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        ok(self.0.lock().unwrap().clone()).set_delay(self.1)
    }
}

/// Los eventos por entregar: cada conexión del stream se lleva el siguiente
/// y se acaba; sin ninguno, espera un poco, se acaba y vuelve a preguntar.
///
/// Con cada tanda va lo que cambia en el servidor, y se cambia **al
/// entregarla**: en el de verdad el mensaje se guarda y el evento sale a la
/// vez. Cambiarlo antes deja que la recarga de una vuelta del stream vea el
/// mensaje antes que su evento, y eso no pasa fuera de la prueba.
#[derive(Clone, Default)]
struct Events(Arc<Mutex<VecDeque<(String, Vec<(Live, Value)>)>>>);

impl Events {
    fn push(&self, frames: String, changes: Vec<(Live, Value)>) {
        self.0.lock().unwrap().push_back((frames, changes));
    }
    fn delivered(&self) -> bool {
        self.0.lock().unwrap().is_empty()
    }
}

impl Respond for Events {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let Some((frames, changes)) = self.0.lock().unwrap().pop_front() else {
            return ResponseTemplate::new(200).set_body_raw(b": nada\n\n".to_vec(), "text/event-stream").set_delay(Duration::from_millis(300));
        };
        for (live, value) in changes {
            live.set(value);
        }
        ResponseTemplate::new(200).set_body_raw(frames.into_bytes(), "text/event-stream")
    }
}

struct World {
    latest: Live,
    dms: Live,
    following: Live,
    events: Events,
}

/// Seguir o silenciar, como el servidor: cambia lo que contesta
/// `/chat/following`.
struct Follow {
    following: Live,
    on: bool,
}

impl Respond for Follow {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let space = request.url.path().split('/').nth(4).unwrap_or_default().to_string();
        let mut now: Vec<String> = serde_json::from_value(self.following.0.lock().unwrap().clone()).unwrap();
        now.retain(|id| id != &space);
        if self.on {
            now.push(space);
        }
        self.following.set(json!(now));
        ok(json!(null))
    }
}

async fn world(server: &MockServer, latest: Value, dms: Value) -> World {
    world_with(server, Live::new(latest), dms).await
}

async fn world_with(server: &MockServer, latest: Live, dms: Value) -> World {
    mount_org(server).await;
    let world = World { latest, dms: Live::new(dms), following: Live::new(json!(["a", "b", "z"])), events: Events::default() };
    get("/api/v1/chat/following").respond_with(world.following.clone()).mount(server).await;
    let follow = || wiremock::matchers::path_regex(r"^/api/v1/task-spaces/[^/]+/chat/follow$");
    Mock::given(method("POST")).and(follow()).respond_with(Follow { following: world.following.clone(), on: true }).mount(server).await;
    Mock::given(method("DELETE")).and(follow()).respond_with(Follow { following: world.following.clone(), on: false }).mount(server).await;
    get("/api/v1/chat/latest").respond_with(world.latest.clone()).mount(server).await;
    get("/api/v1/dm/").respond_with(world.dms.clone()).mount(server).await;
    get("/api/v1/events").respond_with(world.events.clone()).mount(server).await;
    world
}

/// La última línea de un canal, a esa hora.
fn line(space: &str, at: &str) -> Value {
    json!({ "spaceId": space, "authorUserId": "u2", "authorName": "Luis Mora", "kind": "user", "body": "…", "createdAt": at })
}

fn chat_message(space: &str, id: &str, actor: &str, extra: Value) -> String {
    let mut data = json!({
        "spaceId": space, "messageId": id, "actorId": actor, "kind": "user",
        "spaceName": match space { "a" => "Producto", "b" => "Diseño", _ => "Soporte" },
        "authorName": "Luis Mora", "preview": "¿sale hoy?", "mentions": null, "repliedTo": "", "edited": false,
    });
    for (key, value) in extra.as_object().unwrap() {
        data[key] = value.clone();
    }
    format!("event: chat:message\ndata: {data}\n\n")
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..400 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no llegó a pasar: {what}");
}

/// Entrega unos eventos, con lo que cambia en el servidor a la vez, y espera
/// a que se hayan leído y a que la recarga que provocan (400 ms después)
/// haya contestado.
async fn deliver(world: &World, frames: String, changes: Vec<(&Live, Value)>) {
    world.events.push(frames, changes.into_iter().map(|(live, value)| (live.clone(), value)).collect());
    until("se entregan los eventos", || world.events.delivered()).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
}

/// Entra y espera a que la primera carga haya sembrado las horas.
async fn enter(rig: &Rig) {
    rig.core.open_workspace().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
}

const T0: &str = "2026-11-18T10:00:00Z";
const T1: &str = "2026-11-18T10:05:00.5Z";
const T2: &str = "2026-11-18T10:06:00.25Z";

/// Un mensaje nuevo en un canal que se sigue y no se tiene delante se avisa,
/// con lo que dice y a dónde lleva; uno que se corrige, no.
///
/// La corrección es de un mensaje de antes de abrir la app: su id no se ha
/// visto nunca, y lo único que la delata es que la última línea del canal
/// no avanza. Roturas que atrapa: avisar sin esperar a la recarga (la
/// corrección sale como mensaje nuevo, que es lo que hace Tauri); no avisar
/// de nada; el aviso sin su organización o sin su canal.
#[tokio::test]
async fn a_new_message_is_announced_and_an_edit_is_not() {
    let rig = rig().await;
    let world = world(&rig.server, json!([line("a", T0)]), json!([])).await;
    enter(&rig).await;

    deliver(&world, chat_message("a", "m5", "u2", json!({})), vec![(&world.latest, json!([line("a", T1)]))]).await;
    let announced = rig.recorder.announced();
    assert_eq!(announced.len(), 1, "se avisa el mensaje nuevo");
    let said = &announced[0];
    assert_eq!((said.kind, said.tag.as_str()), (NoticeKind::ChannelMessage, "chat:a"));
    assert_eq!(said.target.conversation, Conversation::Channel { space_id: "a".into() });
    assert_eq!(said.target.org_id.as_deref(), Some("o1"));
    assert_eq!((said.title(Locale::Es), said.body(Locale::Es)), ("#Producto".to_string(), "Luis Mora: ¿sale hoy?".to_string()));

    deliver(&world, chat_message("a", "m1", "u2", json!({ "preview": "corregido" })), vec![]).await;
    assert_eq!(rig.recorder.announced().len(), 1, "la corrección no se avisa");
    rig.core.stream().stop();
}

/// Un mensaje que llegó en esta sesión y luego se corrige trae el mismo id:
/// no se vuelve a avisar aunque, entre tanto, el canal haya avanzado por
/// otro mensaje.
///
/// Rotura que atrapa: no recordar los ids vistos (la hora sola no basta si
/// otra línea movió el canal).
#[tokio::test]
async fn a_message_seen_this_session_is_not_announced_again() {
    let rig = rig().await;
    let world = world(&rig.server, json!([line("a", T0)]), json!([])).await;
    enter(&rig).await;

    deliver(&world, chat_message("a", "m5", "u2", json!({})), vec![(&world.latest, json!([line("a", T1)]))]).await;
    assert_eq!(rig.recorder.announced().len(), 1);
    deliver(&world, chat_message("a", "m5", "u2", json!({ "preview": "corregido" })), vec![(&world.latest, json!([line("a", T2)]))]).await;
    assert_eq!(rig.recorder.announced().len(), 1, "el mismo id, otra vez, no");
    rig.core.stream().stop();
}

/// Retirar el último mensaje no se avisa, y el siguiente de verdad sí,
/// aunque sea más viejo que el retirado.
///
/// Roturas que atrapa: tomar el retiro por un mensaje; quedarse con la hora
/// más alta en vez de la del servidor, con lo que el mensaje siguiente no
/// cuenta como nuevo.
#[tokio::test]
async fn withdrawing_the_last_message_is_silent_and_the_next_one_is_not() {
    let rig = rig().await;
    // Se entra con el último a las 10:06; se retira y vuelve a ser el de las
    // 10:00; luego llega uno de las 10:05 (basta un reloj de servidor que
    // escribió en ese orden).
    let world = world(&rig.server, json!([line("a", T2)]), json!([])).await;
    enter(&rig).await;

    deliver(&world, chat_message("a", "m9", "u2", json!({ "preview": "" })), vec![(&world.latest, json!([line("a", T0)]))]).await;
    assert!(rig.recorder.announced().is_empty(), "el retiro no se avisa");
    deliver(&world, chat_message("a", "m10", "u2", json!({})), vec![(&world.latest, json!([line("a", T1)]))]).await;
    assert_eq!(rig.recorder.announced().len(), 1, "el siguiente, sí");
    rig.core.stream().stop();
}

/// Lo propio no se avisa; la línea del sistema que provocó otro, sí. Y con
/// el canal delante, un mensaje normal no se avisa y una mención sí.
///
/// Roturas que atrapa: avisar de lo que uno mismo escribe; callar las líneas
/// del sistema ajenas; avisar de lo normal teniéndolo delante; callar una
/// mención por tener el canal delante.
#[tokio::test]
async fn own_lines_are_silent_and_a_mention_gets_through_the_open_channel() {
    let rig = rig().await;
    let world = world(&rig.server, json!([line("a", T0), line("b", T0)]), json!([])).await;
    enter(&rig).await;
    rig.core.comms().enter();
    assert!(rig.core.comms().select(Conversation::Channel { space_id: "a".into() }));

    let system = json!({ "kind": "system", "authorName": "", "preview": "Grabación lista" });
    deliver(&world, chat_message("b", "m1", "u1", json!({})) + &chat_message("b", "m2", "u2", system), vec![(&world.latest, json!([line("a", T0), line("b", T1)]))]).await;
    let announced = rig.recorder.announced();
    assert_eq!(announced.len(), 1);
    assert_eq!((announced[0].tag.as_str(), announced[0].body(Locale::Es)), ("chat:b", "Grabación lista".to_string()), "la del sistema, y no lo mío");

    deliver(&world, chat_message("a", "m3", "u2", json!({})), vec![(&world.latest, json!([line("a", T1), line("b", T1)]))]).await;
    assert_eq!(rig.recorder.announced().len(), 1, "lo normal en el canal que se tiene delante, no");

    deliver(&world, chat_message("a", "m4", "u2", json!({ "mentions": ["u1"], "preview": "@ana mira" })), vec![(&world.latest, json!([line("a", T2), line("b", T1)]))]).await;
    let announced = rig.recorder.announced();
    assert_eq!(announced.len(), 2, "la mención, sí");
    assert_eq!((announced[1].kind, announced[1].title(Locale::Es)), (NoticeKind::Mention, "Te mencionaron en #Producto".to_string()));
    rig.core.stream().stop();
}

/// Varios mensajes seguidos en un canal salen en un aviso, y gana la
/// mención aunque no sea el último.
///
/// Roturas que atrapa: un globo por mensaje (en Linux no se sustituyen);
/// quedarse con el último en vez del que más pesa.
#[tokio::test]
async fn a_burst_is_one_notice_and_the_mention_wins() {
    let rig = rig().await;
    let world = world(&rig.server, json!([line("b", T0)]), json!([])).await;
    enter(&rig).await;

    let burst = chat_message("b", "m1", "u2", json!({}))
        + &chat_message("b", "m2", "u3", json!({ "mentions": ["u1"], "authorName": "Bea Soto", "preview": "@ana ¿lo ves?" }))
        + &chat_message("b", "m3", "u2", json!({ "preview": "y otra cosa" }));
    deliver(&world, burst, vec![(&world.latest, json!([line("b", T1)]))]).await;
    let announced = rig.recorder.announced();
    assert_eq!(announced.len(), 1, "un aviso por la ráfaga");
    assert_eq!((announced[0].kind, announced[0].body(Locale::Es)), (NoticeKind::Mention, "Bea Soto: @ana ¿lo ves?".to_string()));
    rig.core.stream().stop();
}

fn direct(at: &str, last_author: &str) -> Value {
    json!({ "conversationId": "c1", "orgId": "o1", "userId": "u2", "username": "luis", "unread": 1, "lastMessageAt": at, "lastBody": "secreto", "lastAuthorId": last_author })
}

fn dm_message(id: &str) -> String {
    format!("event: dm:message\ndata: {}\n\n", json!({ "conversationId": "c1", "messageId": id, "actorId": "u2", "authorName": "Luis Mora" }))
}

/// Un directo nuevo se avisa sin nada de lo que dice, con su organización;
/// uno corregido (la hora no avanza) no, y tampoco uno al que uno mismo ya
/// contestó.
///
/// Roturas que atrapa: enseñar el texto de un directo; avisar de una
/// corrección; el aviso sin la organización a la que llevar; avisar cuando
/// la última línea ya es de uno.
#[tokio::test]
async fn a_direct_is_announced_without_its_text_and_an_edit_is_not() {
    let rig = rig().await;
    let world = world(&rig.server, json!([]), json!([direct(T0, "u2")])).await;
    enter(&rig).await;

    deliver(&world, dm_message("d5"), vec![(&world.dms, json!([direct(T1, "u2")]))]).await;
    let announced = rig.recorder.announced();
    assert_eq!(announced.len(), 1, "se avisa el directo");
    let said = &announced[0];
    assert_eq!((said.kind, said.title(Locale::Es), said.body(Locale::Es)), (NoticeKind::Direct, "Luis Mora te escribió".to_string(), String::new()));
    assert_eq!(said.target.org_id.as_deref(), Some("o1"));
    assert!(!format!("{said:?}").contains("secreto"), "nada del texto viaja en el aviso");

    deliver(&world, dm_message("d1"), vec![]).await;
    assert_eq!(rig.recorder.announced().len(), 1, "la corrección no se avisa");

    deliver(&world, dm_message("d6"), vec![(&world.dms, json!([direct(T2, "u1")]))]).await;
    assert_eq!(rig.recorder.announced().len(), 1, "si lo último ya es mío, no");
    rig.core.stream().stop();
}

/// Una carga que salió antes que el mensaje y vuelve después, sin él, no lo
/// da por corregido: espera a la siguiente.
///
/// Pasa siempre que el stream vuelve a conectar (cada vuelta lo recarga
/// todo) con un servidor que tarda en contestar. Rotura que atrapa: dar por
/// corregido un mensaje con cualquier carga que no lo trae, con lo que un
/// mensaje nuevo se pierde según cómo caigan las respuestas.
#[tokio::test]
async fn a_load_that_left_before_the_message_does_not_decide_it() {
    let rig = rig().await;
    let world = world_with(&rig.server, Live::slow(json!([line("a", T0)]), Duration::from_millis(300)), json!([])).await;
    enter(&rig).await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    world.events.push(chat_message("a", "m5", "u2", json!({})), vec![(world.latest.clone(), json!([line("a", T1)]))]);
    until("se avisa aunque una carga vieja vuelva sin él", || rig.recorder.announced().len() == 1).await;
    rig.core.stream().stop();
}

/// Pulsar el aviso de un canal de otra organización lleva a esa
/// organización, a comunicación y a ese canal; el de un directo, a ese
/// directo.
///
/// El canal de otra organización no se sabe de quién es al llegar (sólo
/// está el árbol de la que se mira): se averigua al pulsar. Roturas que
/// atrapa: no cambiar de organización (el canal no se puede elegir); elegir
/// antes de cambiar (se elige en la anterior y se pierde); no entrar en
/// comunicación; el directo sin su organización.
#[tokio::test]
async fn opening_a_notice_goes_to_its_organization_and_its_conversation() {
    let rig = rig().await;
    let world = world(&rig.server, json!([line("z", T0)]), json!([direct(T0, "u2")])).await;
    enter(&rig).await;

    deliver(&world, chat_message("z", "m5", "u2", json!({})), vec![(&world.latest, json!([line("z", T1)]))]).await;
    let announced = rig.recorder.announced();
    assert_eq!(announced.len(), 1, "se avisa lo de la otra organización");
    assert_eq!(announced[0].title(Locale::Es), "#Soporte");

    assert!(rig.core.notices().open(announced[0].target.clone()).await);
    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o2"));
    let comms = rig.core.comms().state();
    assert!(comms.on_screen, "en comunicación");
    assert_eq!(comms.selected, Some(Conversation::Channel { space_id: "z".into() }));

    // Y de vuelta, por un directo de la primera.
    rig.core.comms().leave();
    deliver(&world, dm_message("d5"), vec![(&world.dms, json!([direct(T1, "u2")]))]).await;
    let direct = rig.recorder.announced()[1].clone();
    assert!(rig.core.notices().open(direct.target).await);
    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o1"));
    let comms = rig.core.comms().state();
    assert!(comms.on_screen);
    assert_eq!(comms.selected, Some(Conversation::Direct { conversation_id: "c1".into() }));
    rig.core.stream().stop();
}

/// Un canal silenciado no avisa de lo corriente, y una mención sí; volver a
/// seguirlo lo devuelve.
///
/// Roturas que atrapa: silenciar sin efecto en los avisos (como en Tauri);
/// que el silencio calle también las menciones; que seguir no deshaga el
/// silencio; no pedir el cambio al servidor.
#[tokio::test]
async fn a_muted_channel_is_silent_but_a_mention_gets_through() {
    let rig = rig().await;
    let world = world(&rig.server, json!([line("b", T0)]), json!([])).await;
    enter(&rig).await;

    rig.core.comms().set_following("b", false).await.unwrap();
    assert_eq!(*world.following.0.lock().unwrap(), json!(["a", "z"]), "se le pide al servidor");
    deliver(&world, chat_message("b", "m1", "u2", json!({})), vec![(&world.latest, json!([line("b", T1)]))]).await;
    assert!(rig.recorder.announced().is_empty(), "silenciado, lo corriente no avisa");

    deliver(&world, chat_message("b", "m2", "u2", json!({ "mentions": ["u1"] })), vec![(&world.latest, json!([line("b", T2)]))]).await;
    assert_eq!(rig.recorder.announced().len(), 1, "la mención, sí");
    assert_eq!(rig.recorder.announced()[0].kind, NoticeKind::Mention);

    rig.core.comms().set_following("b", true).await.unwrap();
    deliver(&world, chat_message("b", "m3", "u2", json!({})), vec![(&world.latest, json!([line("b", "2026-11-18T10:07:00Z")]))]).await;
    assert_eq!(rig.recorder.announced().len(), 2, "seguido otra vez, avisa");
    rig.core.stream().stop();
}

/// El botón cambia en el acto y, si el servidor dice que no, vuelve a como
/// estaba y se dice por qué, aunque tampoco se pueda volver a pedir la lista
/// (la red se cayó). Un directo no se sigue ni se silencia.
///
/// Roturas que atrapa: esperar al servidor para cambiar (se pulsa dos
/// veces); quedarse silenciado tras un fallo; tragarse el error; decir de
/// un directo que se sigue.
#[tokio::test]
async fn following_changes_at_once_and_goes_back_if_the_server_says_no() {
    let rig = rig().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/task-spaces/a/chat/follow"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": "boom" })).set_delay(Duration::from_millis(300)))
        .with_priority(1)
        .mount(&rig.server)
        .await;
    let _world = world(&rig.server, json!([line("a", T0)]), json!([direct(T0, "u2")])).await;
    enter(&rig).await;
    get("/api/v1/chat/following").respond_with(ResponseTemplate::new(503)).with_priority(1).mount(&rig.server).await;
    rig.core.comms().enter();
    assert!(rig.core.comms().select(Conversation::Channel { space_id: "a".into() }));
    assert_eq!(rig.core.comms().state().selected_followed, Some(true));

    let core = rig.core.clone();
    let asked = tokio::spawn(async move { core.comms().set_following("a", false).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.core.comms().state().selected_followed, Some(false), "cambia en el acto");
    assert!(asked.await.unwrap().is_err(), "el fallo se dice");
    assert_eq!(rig.core.comms().state().selected_followed, Some(true), "y vuelve a como estaba");

    assert!(rig.core.comms().select(Conversation::Direct { conversation_id: "c1".into() }));
    assert_eq!(rig.core.comms().state().selected_followed, None, "un directo no se sigue");
    rig.core.stream().stop();
}
