//! La conversación que se tiene delante —los mensajes de un canal o de un
//! directo— contra un servidor de mentira. Cada prueba dice la rotura que
//! atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Config, Conversation, Core, Listener, Session, Storage, ThreadLink, ThreadState};
use chrono::{DateTime, FixedOffset, TimeZone};
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

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
    threads: Mutex<Vec<ThreadState>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn thread_changed(&self, state: ThreadState) {
        self.threads.lock().unwrap().push(state);
    }
}

impl Recorder {
    /// Si alguna vez se avisó de una conversación que cumple eso.
    fn ever(&self, seen: impl Fn(&ThreadState) -> bool) -> bool {
        self.threads.lock().unwrap().iter().any(seen)
    }
}

/// Lo que el servidor de mentira guarda de una conversación: sus mensajes,
/// del más viejo al más nuevo. Contesta como el de verdad: los cincuenta
/// últimos, o los cincuenta anteriores a `before` (estrictamente), y la
/// primera página si no entiende el cursor.
type History = Arc<Mutex<Vec<Value>>>;

struct Rig {
    server: MockServer,
    core: Core,
    recorder: Arc<Recorder>,
    /// Lo que el servidor cuenta sin leer en el canal `a`: tres, hasta que
    /// se marca leído. Y si ya se marcó leído el directo `c1`.
    unread_a: Arc<AtomicI64>,
    read_c1: Arc<AtomicBool>,
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn get(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path(route.to_string()))
}

fn post(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("POST")).and(path(route.to_string()))
}

fn org(id: &str) -> Value {
    json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" })
}

fn space(id: &str, org_id: &str, name: &str, kind: Option<&str>) -> Value {
    json!({ "id": id, "orgId": org_id, "name": name, "color": "#888", "kind": kind, "folders": [], "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 0, "taskCount": 0 }], "people": [] })
}

/// Dos organizaciones. `o1`: la sala general `g`, `a` «Producto» y `b`
/// «Diseño», y un directo con Luis (`c1`). `o2`: `z` «Soporte» y un directo
/// con Zoe (`c9`). El canal `a` tiene tres sin leer y el directo `c1` dos,
/// hasta que se marcan leídos.
async fn rig() -> Rig {
    rig_counting_slowly(Duration::ZERO).await
}

/// Lo mismo, con un servidor que tarda eso en contestar cuánto hay sin leer
/// en los canales: lo cuenta al recibir la pregunta y lo dice después.
async fn rig_counting_slowly(unread_delay: Duration) -> Rig {
    let server = MockServer::start().await;
    let recorder = Arc::new(Recorder::default());
    let mut config = Config::new(server.uri());
    config.stream_backoff = Duration::from_millis(40);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();

    get("/api/v1/organizations/").respond_with(ok(json!([org("o1"), org("o2")]))).mount(&server).await;
    let o1 = json!([space("a", "o1", "Producto", None), space("b", "o1", "Diseño", None), space("g", "o1", "General", Some("general"))]);
    get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(o1)).mount(&server).await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2", "Soporte", None)]))).mount(&server).await;
    get("/api/v1/chat/latest").respond_with(ok(json!([]))).mount(&server).await;

    let (unread_a, read_c1) = (Arc::new(AtomicI64::new(3)), Arc::new(AtomicBool::new(false)));
    let unread = unread_a.clone();
    get("/api/v1/chat/unread")
        .respond_with(move |_: &Request| ok(json!([{ "spaceId": "a", "count": unread.load(Ordering::SeqCst) }, { "spaceId": "z", "count": 70 }])).set_delay(unread_delay))
        .mount(&server)
        .await;
    let read = read_c1.clone();
    get("/api/v1/dm/")
        .respond_with(move |_: &Request| {
            ok(json!([
                { "conversationId": "c1", "orgId": "o1", "userId": "u2", "username": "luis", "unread": if read.load(Ordering::SeqCst) { 0 } else { 2 },
                  "lastMessageAt": "2026-10-15T15:00:00Z", "lastBody": "¿vienes?", "lastAuthorId": "u2" },
                { "conversationId": "c9", "orgId": "o2", "userId": "u9", "username": "zoe", "unread": 40, "lastBody": "de otra organización", "lastAuthorId": "u9" },
            ]))
        })
        .mount(&server)
        .await;
    let unread = unread_a.clone();
    post("/api/v1/task-spaces/a/chat/read")
        .respond_with(move |_: &Request| {
            unread.store(0, Ordering::SeqCst);
            ok(json!(null))
        })
        .mount(&server)
        .await;
    let read = read_c1.clone();
    post("/api/v1/dm/c1/read")
        .respond_with(move |_: &Request| {
            read.store(true, Ordering::SeqCst);
            ok(json!(null))
        })
        .mount(&server)
        .await;
    post("/api/v1/task-spaces/b/chat/read").respond_with(ok(json!(null))).mount(&server).await;
    Rig { server, core, recorder, unread_a, read_c1 }
}

fn at(second: i64) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(0).unwrap().with_ymd_and_hms(2026, 10, 15, 10, 0, 0).unwrap() + chrono::Duration::seconds(second)
}

/// Un mensaje de canal, escrito a tantos segundos de las diez.
fn line(id: &str, author: &str, body: &str, second: i64) -> Value {
    let when = at(second).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    json!({ "id": id, "spaceId": "a", "authorUserId": author, "authorName": format!("N-{author}"), "kind": "user", "body": body, "createdAt": when, "updatedAt": when })
}

/// Lo mismo en un directo: sin `kind`.
fn private_line(id: &str, author: &str, body: &str, second: i64) -> Value {
    let when = at(second).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    json!({ "id": id, "conversationId": "c1", "authorUserId": author, "authorName": format!("N-{author}"), "body": body, "createdAt": when, "updatedAt": when })
}

fn numbered(prefix: &str, range: std::ops::Range<i64>) -> Vec<Value> {
    range.map(|index| line(&format!("{prefix}{index:03}"), "u2", "hola", index)).collect()
}

fn history(messages: Vec<Value>) -> History {
    Arc::new(Mutex::new(messages))
}

fn instant(message: &Value) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(message["createdAt"].as_str().unwrap()).unwrap()
}

/// Sirve esa conversación en esa ruta, como el servidor de verdad.
async fn mount_history(server: &MockServer, route: &str, history: &History, delay: Duration) {
    let history = history.clone();
    get(route)
        .respond_with(move |request: &Request| {
            let before = request.url.query_pairs().find(|(name, _)| name == "before").and_then(|(_, value)| DateTime::parse_from_rfc3339(&value).ok());
            let all = history.lock().unwrap().clone();
            let visible: Vec<Value> = all.into_iter().filter(|message| before.is_none_or(|before| instant(message) < before)).collect();
            let page = visible[visible.len().saturating_sub(50)..].to_vec();
            ok(json!(page)).set_delay(delay)
        })
        .mount(server)
        .await;
}

/// Un stream que entrega esos eventos una vez, tras `delay`, y se acaba.
async fn mount_events(server: &MockServer, frames: &str, delay: Duration) {
    get("/api/v1/events")
        .respond_with(ResponseTemplate::new(200).set_body_raw(frames.as_bytes().to_vec(), "text/event-stream").set_delay(delay))
        .up_to_n_times(1)
        .mount(server)
        .await;
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

async fn until_hits(server: &MockServer, verb: &str, route: &str, wanted: usize) {
    for _ in 0..400 {
        if hits(server, verb, route).await >= wanted {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{verb} {route} no llegó a pedirse {wanted} veces");
}

async fn requests(server: &MockServer, verb: &str, route: &str) -> Vec<Request> {
    server.received_requests().await.unwrap().into_iter().filter(|request| request.method.as_str() == verb && request.url.path() == route).collect()
}

async fn hits(server: &MockServer, verb: &str, route: &str) -> usize {
    requests(server, verb, route).await.len()
}

fn channel(id: &str) -> Conversation {
    Conversation::Channel { space_id: id.into() }
}

fn direct(id: &str) -> Conversation {
    Conversation::Direct { conversation_id: id.into() }
}

fn ids(state: &ThreadState) -> Vec<String> {
    state.messages.iter().map(|message| message.id.clone()).collect()
}

fn unread_of(core: &Core, space: &str) -> i64 {
    core.comms().state().channels.iter().find(|channel| channel.space_id == space).map(|channel| channel.unread).unwrap_or(-1)
}

/// Entra a comunicación con esa conversación delante, y la trae.
async fn open(rig: &Rig, conversation: Conversation) {
    rig.core.comms().enter();
    assert!(rig.core.comms().select(conversation), "esa conversación no es de esta organización");
    rig.core.chat().follow().await;
}

const CHAT_A: &str = "/api/v1/task-spaces/a/chat";
const CHAT_B: &str = "/api/v1/task-spaces/b/chat";
const READ_A: &str = "/api/v1/task-spaces/a/chat/read";
const DM_C1: &str = "/api/v1/dm/c1/messages";
const READ_C1: &str = "/api/v1/dm/c1/read";

/// Abrir un canal trae su última página y lo marca leído; su contador se
/// va de la lista y del riel.
///
/// Roturas que atrapa: pedir los mensajes por otra ruta, o sin el tope de
/// cincuenta; no marcar leído al abrir (el contador se queda para siempre);
/// marcarlo antes de haber leído nada; el contador de la fila o el número
/// del riel sin bajar; decir que queda historia cuando la página vino corta.
#[tokio::test]
async fn opening_a_channel_reads_its_last_page_and_marks_it_read() {
    let rig = rig().await;
    mount_history(&rig.server, CHAT_A, &history(vec![line("m1", "u2", "hola", 1), line("m2", "u1", "buenas", 2), line("m3", "u2", "¿sale hoy?", 3)]), Duration::ZERO).await;
    rig.core.open_workspace().await;
    assert_eq!(unread_of(&rig.core, "a"), 3);
    assert_eq!(rig.core.counters().state().comms, Some(5));

    open(&rig, channel("a")).await;
    let state = rig.core.chat().state();
    assert_eq!(state.conversation, Some(channel("a")));
    assert_eq!(ids(&state), ["m1", "m2", "m3"]);
    assert!(!state.loading && !state.failed && !state.has_more);
    assert_eq!(state.messages[1].body, "buenas");

    let asked = requests(&rig.server, "GET", CHAT_A).await;
    assert_eq!(asked.iter().map(|request| request.url.query().unwrap_or("").to_string()).collect::<Vec<_>>(), ["limit=50"]);
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 1);
    // En el acto, sin esperar a que el servidor vuelva a contar.
    assert_eq!(unread_of(&rig.core, "a"), 0);
    assert_eq!(rig.core.counters().state().comms, Some(2));
    // Y cuando el servidor vuelve a contar, sigue en cero.
    assert_eq!(rig.unread_a.load(Ordering::SeqCst), 0);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(unread_of(&rig.core, "a"), 0);
    // Quien pinta se enteró de la conversación con sus mensajes.
    assert!(rig.recorder.ever(|state| state.conversation == Some(channel("a")) && state.messages.len() == 3 && !state.loading));
    rig.core.stream().stop();
}

/// Un directo se lee por sus rutas, se marca leído y sus palomitas salen de
/// hasta dónde leyó el otro, que llega por evento sin volver a pedir nada.
///
/// Roturas que atrapa: pedir un directo por la ruta de un canal; no marcarlo
/// leído, o no bajar su contador; un mensaje que llega al directo abierto y
/// no se ve hasta salir y entrar; no avisar a quien pinta de que el otro
/// leyó (las palomitas no cambian hasta el siguiente mensaje); leer un
/// directo como si llevara `kind`.
#[tokio::test]
async fn a_direct_is_read_by_its_own_routes_and_follows_its_events() {
    let rig = rig().await;
    let talk = history(vec![private_line("d1", "u1", "¿quedamos?", 1), private_line("d2", "u2", "¿vienes?", 2)]);
    mount_history(&rig.server, DM_C1, &talk, Duration::ZERO).await;
    let frames = concat!(
        "event: dm:read\ndata: {\"conversationId\":\"c1\",\"userId\":\"u2\",\"at\":\"2026-10-15T10:00:01Z\"}\n\n",
        "event: dm:message\ndata: {\"conversationId\":\"c1\",\"messageId\":\"d3\",\"actorId\":\"u2\",\"authorName\":\"Luis\"}\n\n",
    );
    mount_events(&rig.server, frames, Duration::from_millis(500)).await;
    rig.core.open_workspace().await;
    assert_eq!(rig.core.comms().state().directs[0].unread, 2);

    open(&rig, direct("c1")).await;
    let state = rig.core.chat().state();
    assert_eq!(ids(&state), ["d1", "d2"]);
    assert!(state.messages.iter().all(|message| !message.system));
    assert_eq!(state.other_read_at, None);
    assert_eq!(hits(&rig.server, "POST", READ_C1).await, 1);
    assert_eq!(rig.core.comms().state().directs[0].unread, 0);
    let asked = hits(&rig.server, "GET", DM_C1).await;
    assert_eq!(asked, 1);

    // El otro escribe y lee: el mensaje llega y las palomitas se mueven.
    talk.lock().unwrap().push(private_line("d3", "u2", "ya estoy", 3));
    until("se avisa de hasta dónde leyó el otro", || rig.recorder.ever(|state| state.other_read_at.as_deref() == Some("2026-10-15T10:00:01Z") && state.messages.len() == 2)).await;
    until("llega lo que escribió", || ids(&rig.core.chat().state()) == ["d1", "d2", "d3"]).await;
    assert_eq!(rig.core.chat().state().arrivals, 1);
    until_hits(&rig.server, "POST", READ_C1, 2).await;
    assert!(rig.read_c1.load(Ordering::SeqCst));
    rig.core.stream().stop();
}

/// Al elegir otra conversación no queda nada de la anterior bajo su nombre,
/// ni antes de que lleguen sus mensajes ni si los de la anterior llegan
/// tarde.
///
/// Roturas que atrapa: los mensajes del canal anterior bajo la cabecera del
/// nuevo mientras carga; la respuesta lenta del canal que se dejó pintada
/// encima del nuevo; marcar leído el canal que se dejó.
#[tokio::test(flavor = "multi_thread")]
async fn another_conversation_never_shows_the_previous_ones_messages() {
    let rig = rig().await;
    // El canal `a` contesta rápido la primera vez y muy tarde la segunda.
    get(CHAT_A).respond_with(ok(json!([line("a1", "u2", "de A", 1)]))).up_to_n_times(1).mount(&rig.server).await;
    get(CHAT_A).respond_with(ok(json!([line("a1", "u2", "de A", 1), line("a2", "u2", "de A, tarde", 2)])).set_delay(Duration::from_millis(500))).mount(&rig.server).await;
    mount_history(&rig.server, CHAT_B, &history(vec![line("b1", "u3", "de B", 1)]), Duration::from_millis(200)).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert_eq!(ids(&rig.core.chat().state()), ["a1"]);

    // Se elige B: desde ese instante, antes de pedir nada, lo de A no se ve.
    assert!(rig.core.comms().select(channel("b")));
    let state = rig.core.chat().state();
    assert_eq!((state.conversation.clone(), state.messages.len(), state.loading), (Some(channel("b")), 0, true));
    rig.core.chat().follow().await;
    assert_eq!(ids(&rig.core.chat().state()), ["b1"]);

    // Se vuelve a A, que ahora tarda, y antes de que conteste se elige B.
    assert!(rig.core.comms().select(channel("a")));
    let slow = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().follow().await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rig.core.comms().select(channel("b")));
    rig.core.chat().follow().await;
    slow.await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let state = rig.core.chat().state();
    assert_eq!((state.conversation.clone(), ids(&state)), (Some(channel("b")), vec!["b1".to_string()]));
    assert!(!rig.recorder.ever(|state| state.conversation == Some(channel("b")) && state.messages.iter().any(|message| message.id.starts_with('a'))), "se avisó de mensajes de A bajo B");
    // A se marcó leído una vez, al abrirlo de verdad; la respuesta tardía no.
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 1);
    rig.core.stream().stop();
}

/// Una conversación que no se puede leer lo dice, y su contador se queda:
/// nadie vio esos mensajes. Reintentar la trae.
///
/// Roturas que atrapa: marcar leído lo que no se llegó a leer; una pantalla
/// en «cargando» para siempre, o vacía sin decir por qué; un reintento que
/// no reintenta.
#[tokio::test]
async fn a_conversation_that_cannot_be_read_says_so_and_keeps_its_badge() {
    let rig = rig().await;
    get(CHAT_A).respond_with(ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": "boom", "message": "Failed to read the channel" }))).up_to_n_times(1).mount(&rig.server).await;
    mount_history(&rig.server, CHAT_A, &history(vec![line("m1", "u2", "hola", 1)]), Duration::ZERO).await;
    rig.core.open_workspace().await;

    open(&rig, channel("a")).await;
    let state = rig.core.chat().state();
    assert!(state.failed && !state.loading && state.messages.is_empty());
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 0, "se marcó leído sin haber leído");
    assert_eq!(unread_of(&rig.core, "a"), 3);

    rig.core.chat().retry().await;
    let state = rig.core.chat().state();
    assert!(!state.failed);
    assert_eq!(ids(&state), ["m1"]);
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 1);
    assert_eq!(unread_of(&rig.core, "a"), 0);
    rig.core.stream().stop();
}

/// La página anterior se pide con el instante del mensaje más viejo **tal
/// como llegó**, se pone delante sin repetir nada, y «queda más» es que la
/// página vino llena.
///
/// Roturas que atrapa: un cursor reescrito (sin sus nanosegundos, o en otra
/// zona: el servidor compara con `<` estricto y repetiría o se saltaría el
/// mensaje del corte); el `+` de la zona sin escapar, que llega como un
/// espacio y hace que el servidor conteste otra vez con la primera página;
/// la página puesta al final; mensajes repetidos; seguir pidiendo cuando ya
/// no queda nada; pedir dos veces la misma página.
#[tokio::test(flavor = "multi_thread")]
async fn the_older_page_is_asked_with_the_oldest_instant_exactly_as_received() {
    let rig = rig().await;
    let mut all = numbered("m", 0..110);
    // El del corte, con nanosegundos y en otra zona, como lo mandaría Go.
    all[60]["createdAt"] = json!("2026-10-15T12:01:00.123456789+02:00");
    let talk = history(all);
    mount_history(&rig.server, CHAT_A, &talk, Duration::from_millis(60)).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    let state = rig.core.chat().state();
    assert_eq!((state.messages.len(), state.has_more), (50, true));
    assert_eq!(state.messages[0].id, "m060");

    // Dos veces a la vez: la segunda no pide.
    let (one, other) = (rig.core.chat(), rig.core.chat());
    tokio::join!(one.load_older(), other.load_older());
    let state = rig.core.chat().state();
    assert_eq!((state.messages.len(), state.has_more, state.loading_older), (100, true, false));
    assert_eq!(ids(&state), (10..110).map(|index| format!("m{index:03}")).collect::<Vec<_>>());
    let cursors: Vec<String> = requests(&rig.server, "GET", CHAT_A)
        .await
        .iter()
        .filter_map(|request| request.url.query_pairs().find(|(name, _)| name == "before").map(|(_, value)| value.to_string()))
        .collect();
    assert_eq!(cursors, ["2026-10-15T12:01:00.123456789+02:00"]);

    rig.core.chat().load_older().await;
    let state = rig.core.chat().state();
    assert_eq!((state.messages.len(), state.has_more), (110, false));
    assert_eq!(state.messages[0].id, "m000");
    // Ya no queda nada: no se pide más.
    let before = hits(&rig.server, "GET", CHAT_A).await;
    rig.core.chat().load_older().await;
    assert_eq!(hits(&rig.server, "GET", CHAT_A).await, before);
    rig.core.stream().stop();
}

/// Una página anterior que trae algo que ya se tiene no lo repite.
///
/// Rotura que atrapa: el mismo mensaje dos veces en pantalla cuando el
/// servidor contesta con solape (dos mensajes del mismo instante en el
/// corte, o una página pedida a la vez que otra lectura).
#[tokio::test]
async fn an_older_page_that_overlaps_what_is_held_repeats_nothing() {
    let rig = rig().await;
    get(CHAT_A).and(wiremock::matchers::query_param_is_missing("before")).respond_with(ok(json!(numbered("m", 10..60)))).mount(&rig.server).await;
    get(CHAT_A).respond_with(ok(json!(numbered("m", 5..13)))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    rig.core.chat().load_older().await;
    assert_eq!(ids(&rig.core.chat().state()), (5..60).map(|index| format!("m{index:03}")).collect::<Vec<_>>());
    rig.core.stream().stop();
}

/// Un mensaje que llega al canal abierto se lee solo —una ráfaga, de una
/// vez—, conserva las páginas viejas ya cargadas y queda leído; uno que se
/// retira desaparece sin anunciarse como nuevo.
///
/// Roturas que atrapa: un refresco que tira las páginas cargadas subiendo;
/// una petición por cada evento de una ráfaga; el mensaje retirado todavía
/// pintado, o contado como llegada (saldría «mensajes nuevos» por un mensaje
/// que se fue); una reacción que no se ve hasta el siguiente mensaje; no
/// marcar leído lo que llega mientras se mira.
#[tokio::test(flavor = "multi_thread")]
async fn what_arrives_is_read_at_once_and_what_is_withdrawn_is_not_news() {
    let rig = rig().await;
    let talk = history(numbered("m", 0..60));
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    let said = |message: &str| format!("event: chat:message\ndata: {{\"spaceId\":\"a\",\"messageId\":\"{message}\",\"actorId\":\"u2\",\"kind\":\"user\",\"preview\":\"x\",\"mentions\":null,\"repliedTo\":\"\",\"edited\":false}}\n\n");
    // Una ráfaga de cinco, a los 600 ms.
    mount_events(&rig.server, &said("m060").repeat(5), Duration::from_millis(600)).await;
    // Y luego, una retirada (el mismo evento, sin `edited`) y una reacción.
    let later = format!("{}event: chat:reaction\ndata: {{\"spaceId\":\"a\",\"messageId\":\"m058\",\"actorId\":\"u2\"}}\n\n", said("m059"));
    get("/api/v1/events")
        .respond_with(ResponseTemplate::new(200).set_body_raw(later.into_bytes(), "text/event-stream").set_delay(Duration::from_millis(1500)))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    rig.core.chat().load_older().await;
    assert_eq!(rig.core.chat().state().messages.len(), 60);
    let asked = hits(&rig.server, "GET", CHAT_A).await;

    talk.lock().unwrap().push(line("m060", "u2", "recién llegado", 60));
    until("llega el mensaje nuevo", || rig.core.chat().state().messages.last().is_some_and(|message| message.id == "m060")).await;
    let state = rig.core.chat().state();
    assert_eq!((state.messages.len(), state.arrivals), (61, 1));
    assert_eq!(state.messages[0].id, "m000", "el refresco tiró las páginas viejas");
    until_hits(&rig.server, "POST", READ_A, 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // La ráfaga, y la vuelta del stream, que también relee: dos como mucho,
    // no cinco.
    let after_burst = hits(&rig.server, "GET", CHAT_A).await;
    assert!(after_burst - asked <= 2, "una ráfaga de cinco eventos pidió {} veces", after_burst - asked);

    // Se retira m059 y alguien reacciona a m058.
    {
        let mut talk = talk.lock().unwrap();
        talk.retain(|message| message["id"] != "m059");
        let reacted = talk.iter_mut().find(|message| message["id"] == "m058").unwrap();
        reacted["reactions"] = json!([{ "emoji": "👍", "count": 1, "mine": false, "names": ["Luis Mora"] }]);
    }
    until("el retirado desaparece", || !ids(&rig.core.chat().state()).contains(&"m059".to_string())).await;
    let state = rig.core.chat().state();
    assert_eq!((state.messages.len(), state.arrivals), (60, 1), "un mensaje retirado se anunció como nuevo");
    assert_eq!(state.messages.iter().find(|message| message.id == "m058").map(|message| message.reactions.len()), Some(1));
    assert_eq!(state.messages[0].id, "m000");
    rig.core.stream().stop();
}

/// Lo que pasa en otra conversación no hace releer la que se tiene delante.
///
/// Rotura que atrapa: pedir el canal abierto por cada mensaje de cualquier
/// canal o directo de la organización.
#[tokio::test(flavor = "multi_thread")]
async fn an_event_of_another_conversation_reads_nothing() {
    let rig = rig().await;
    mount_history(&rig.server, CHAT_A, &history(vec![line("m1", "u2", "hola", 1)]), Duration::ZERO).await;
    let frames = concat!(
        "event: chat:message\ndata: {\"spaceId\":\"b\",\"messageId\":\"x\",\"actorId\":\"u2\",\"kind\":\"user\",\"edited\":false}\n\n",
        "event: dm:message\ndata: {\"conversationId\":\"c1\",\"messageId\":\"d9\",\"actorId\":\"u2\"}\n\n",
        "event: chat:reaction\ndata: {\"spaceId\":\"g\",\"messageId\":\"y\",\"actorId\":\"u2\"}\n\n",
    );
    // El stream se queda abierto tras entregarlos: sin vuelta que relea.
    get("/api/v1/events")
        .respond_with(ResponseTemplate::new(200).set_body_raw(frames.as_bytes().to_vec(), "text/event-stream").set_delay(Duration::from_millis(300)))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    get("/api/v1/events").respond_with(ResponseTemplate::new(200).set_body_raw(Vec::new(), "text/event-stream").set_delay(Duration::from_secs(30))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let asked = hits(&rig.server, "GET", CHAT_A).await;
    assert_eq!(asked, 1, "se releyó por eventos de otras conversaciones");
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 1);
    rig.core.stream().stop();
}

/// Al volver el stream de un corte se relee lo que hay delante, y si en el
/// corte se dijo más de lo que cabe en una página, se sigue hacia atrás
/// hasta unirlo con lo que se tenía: sin agujero.
///
/// Roturas que atrapa: quedarse con lo de antes del corte hasta que alguien
/// escriba otra vez; pintar lo de antes y lo último con un hueco de
/// mensajes en medio, sin nada que lo diga.
#[tokio::test(flavor = "multi_thread")]
async fn after_a_cut_what_is_in_front_is_read_again_without_a_gap() {
    let rig = rig().await;
    let talk = history(numbered("h", 0..3));
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    // Un stream que se acaba, y otro que vuelve al rato.
    mount_events(&rig.server, ": hola\n\n", Duration::from_millis(400)).await;
    get("/api/v1/events").respond_with(ResponseTemplate::new(200).set_body_raw(b": de vuelta\n\n".to_vec(), "text/event-stream").set_delay(Duration::from_millis(50))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert_eq!(ids(&rig.core.chat().state()), ["h000", "h001", "h002"]);

    // Durante el corte se dicen ciento veinte cosas.
    talk.lock().unwrap().extend(numbered("n", 100..220));
    until("se relee al volver", || rig.core.chat().state().messages.len() > 3).await;
    let state = rig.core.chat().state();
    let expected: Vec<String> = (0..3).map(|index| format!("h{index:03}")).chain((100..220).map(|index| format!("n{index:03}"))).collect();
    assert_eq!(ids(&state), expected, "quedó un agujero entre lo que se tenía y lo último");
    assert_eq!(state.arrivals, 1);
    assert!(!state.has_more);
    rig.core.stream().stop();
}

/// Al cambiar de organización no queda nada del directo de la anterior, ni
/// lo pone una respuesta suya que llegue tarde, ni se marca leído.
///
/// Roturas que atrapa: el directo de un cliente pintado dentro de otra
/// organización; su respuesta tardía puesta en pantalla; marcar leída una
/// conversación que ya no se tiene delante.
#[tokio::test(flavor = "multi_thread")]
async fn a_direct_of_another_organization_is_not_shown_nor_marked() {
    let rig = rig().await;
    let talk = history(vec![private_line("d1", "u2", "esto es privado", 1)]);
    get(DM_C1).respond_with(ok(json!([private_line("d1", "u2", "esto es privado", 1)]))).up_to_n_times(1).mount(&rig.server).await;
    mount_history(&rig.server, DM_C1, &talk, Duration::from_millis(400)).await;
    mount_history(&rig.server, "/api/v1/task-spaces/z/chat", &history(Vec::new()), Duration::ZERO).await;
    rig.core.open_workspace().await;
    open(&rig, direct("c1")).await;
    assert_eq!(ids(&rig.core.chat().state()), ["d1"]);
    assert_eq!(hits(&rig.server, "POST", READ_C1).await, 1);

    // Llega algo al directo y se relee, despacio; mientras, se cambia.
    talk.lock().unwrap().push(private_line("d2", "u2", "y esto también", 2));
    let slow = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().refresh().await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rig.core.switch_org("o2").await);
    let state = rig.core.chat().state();
    assert_ne!(state.conversation, Some(direct("c1")));
    assert!(state.messages.is_empty(), "quedó el directo de la otra organización");
    slow.await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(rig.core.chat().state().messages.is_empty());
    assert!(!rig.core.comms().select(direct("c1")), "se eligió un directo de otra organización");
    assert_eq!(hits(&rig.server, "POST", READ_C1).await, 1, "se marcó leída una conversación que ya no está delante");
    // Y al seguir a la lista, lo que se abre es lo de la nueva.
    rig.core.chat().follow().await;
    assert_eq!(rig.core.chat().state().conversation, Some(channel("z")));
    rig.core.stream().stop();
}

/// Fuera de comunicación la conversación no se relee ni se marca leída; al
/// volver se pone al día y entonces sí.
///
/// Roturas que atrapa: marcar leído lo que llega mientras se mira «Mi
/// trabajo» (el contador no avisaría nunca); marcar leída una conversación
/// cuya primera página llegó cuando ya se había salido; volver y encontrar
/// lo de antes de irse; volver y que el contador se quede.
#[tokio::test(flavor = "multi_thread")]
async fn out_of_communication_nothing_is_read_or_marked_until_coming_back() {
    let rig = rig().await;
    let talk = history(vec![line("m1", "u2", "hola", 1)]);
    mount_history(&rig.server, CHAT_A, &talk, Duration::from_millis(250)).await;
    let frame = "event: chat:message\ndata: {\"spaceId\":\"a\",\"messageId\":\"m2\",\"actorId\":\"u2\",\"kind\":\"user\",\"edited\":false}\n\n";
    get("/api/v1/events")
        .respond_with(ResponseTemplate::new(200).set_body_raw(frame.as_bytes().to_vec(), "text/event-stream").set_delay(Duration::from_millis(700)))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    get("/api/v1/events").respond_with(ResponseTemplate::new(200).set_body_raw(Vec::new(), "text/event-stream").set_delay(Duration::from_secs(30))).mount(&rig.server).await;
    rig.core.open_workspace().await;

    // Se entra, y antes de que llegue la página se sale.
    rig.core.comms().enter();
    assert!(rig.core.comms().select(channel("a")));
    let opening = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().follow().await }
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    rig.core.comms().leave();
    rig.core.chat().follow().await;
    opening.await.unwrap();
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 0, "se marcó leída una conversación que no se llegó a mirar");

    // Se vuelve sin que haya llegado nada: lo que no se pudo marcar antes
    // se marca ahora, que es cuando se mira.
    rig.core.comms().enter();
    rig.core.chat().follow().await;
    assert_eq!(ids(&rig.core.chat().state()), ["m1"]);
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 1, "al volver no se marcó leída");
    assert_eq!(unread_of(&rig.core, "a"), 0);

    // Se sale otra vez, y llega un mensaje: ni se relee ni se marca.
    rig.core.comms().leave();
    rig.core.chat().follow().await;
    talk.lock().unwrap().push(line("m2", "u2", "¿estás?", 2));
    let asked = hits(&rig.server, "GET", CHAT_A).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // Ni por el evento, ni por la vuelta de un corte (que llama a esto).
    rig.core.chat().refresh().await;
    assert_eq!(hits(&rig.server, "GET", CHAT_A).await, asked, "se releyó una conversación que no se mira");
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 1, "se marcó leído lo que llegó sin mirarlo");

    // Al volver, se pone al día y se marca.
    rig.core.comms().enter();
    rig.core.chat().follow().await;
    assert_eq!(ids(&rig.core.chat().state()), ["m1", "m2"]);
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 2);
    rig.core.stream().stop();
}

/// Una imagen de un mensaje se baja con la sesión, y sólo de este servidor.
///
/// Roturas que atrapa: pedir la imagen sin la sesión (el servidor contesta
/// que no existe, y el mensaje sale sin su captura); pedir con la sesión una
/// dirección que no es un adjunto de este servidor; dar por imagen lo que
/// no lo es.
#[tokio::test]
async fn a_picture_in_a_message_is_fetched_with_the_session_and_only_from_here() {
    let rig = rig().await;
    let png = include_bytes!("fixtures/gray_600x200.png").to_vec();
    let shot = "/api/v1/task-spaces/a/chat/attachments/a1/raw";
    get(shot).and(header("Authorization", "Bearer access")).respond_with(ResponseTemplate::new(200).set_body_raw(png.clone(), "image/png")).mount(&rig.server).await;
    let notes = "/api/v1/dm/c1/attachments/a2/raw";
    get(notes).and(header("Authorization", "Bearer access")).respond_with(ResponseTemplate::new(200).set_body_raw(b"hola".to_vec(), "text/plain")).mount(&rig.server).await;

    let picture = rig.core.chat().picture(shot).await.unwrap();
    assert_eq!((picture.mime.as_str(), picture.bytes.len()), ("image/png", png.len()));
    assert_eq!(rig.core.chat().file_bytes(notes).await.unwrap(), b"hola");
    assert!(rig.core.chat().picture(notes).await.is_err(), "un texto se dio por imagen");

    let asked = rig.server.received_requests().await.unwrap().len();
    for foreign in ["https://otro.example/api/v1/x/attachments/a/raw", "/api/v1/../admin", "/otra/cosa"] {
        assert!(rig.core.chat().file_bytes(foreign).await.is_err(), "{foreign}");
    }
    assert_eq!(rig.server.received_requests().await.unwrap().len(), asked, "se pidió algo que no es un adjunto de este servidor");
}

/// Un enlace de un mensaje va a donde apunta: una tarea se abre en su
/// página, también por su nombre de fuera, y un documento saca de
/// comunicación para leerlo.
///
/// Roturas que atrapa: una tarea citada en el canal que no abre nada; un
/// nombre que no existe dado por bueno; un documento abierto debajo de la
/// conversación, donde no se ve; lo de fuera tratado como de dentro.
#[tokio::test]
async fn a_link_in_a_message_opens_what_it_points_to() {
    let rig = rig().await;
    let detail: Value = serde_json::from_str(include_str!("fixtures/task_detail.json")).unwrap();
    let task_id = detail["task"]["id"].as_str().unwrap().to_string();
    get("/api/v1/tasks/by-ref/cac-12").respond_with(ok(detail.clone())).mount(&rig.server).await;
    get(&format!("/api/v1/tasks/{task_id}")).respond_with(ok(detail)).mount(&rig.server).await;
    rig.core.open_workspace().await;
    rig.core.comms().enter();

    assert_eq!(rig.core.chat().follow_link("/tasks?task=cac-12").await, ThreadLink::Task { id: "cac-12".into() });
    assert_eq!(rig.core.tasks().state().task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(rig.core.chat().follow_link("/tasks?task=no-existe-9").await, ThreadLink::Unknown);
    assert_eq!(rig.core.chat().follow_link("https://example.com").await, ThreadLink::Web { url: "https://example.com".into() });
    assert!(rig.core.comms().state().on_screen);

    assert_eq!(rig.core.chat().follow_link("/tasks?doc=space:borrado").await, ThreadLink::Unknown);
    assert!(rig.core.comms().state().on_screen, "un documento que no existe sacó de comunicación");
    assert_eq!(rig.core.chat().follow_link("/tasks?doc=space:a").await, ThreadLink::Doc);
    assert!(!rig.core.comms().state().on_screen, "el documento se abrió debajo de la conversación");
    assert_eq!(rig.core.docs().state().target.map(|target| target.id), Some("a".to_string()));
    rig.core.stream().stop();
}

/// Al salir de la sesión no queda la conversación de quien estaba.
///
/// Rotura que atrapa: los mensajes de una persona a la vista de la que entra
/// después en la misma máquina.
#[tokio::test]
async fn signing_out_leaves_no_conversation_behind() {
    let rig = rig().await;
    mount_history(&rig.server, CHAT_A, &history(vec![line("m1", "u2", "hola", 1)]), Duration::ZERO).await;
    post("/api/v1/auth/logout").respond_with(ok(json!(null))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert_eq!(rig.core.chat().state().messages.len(), 1);

    rig.core.logout().await;
    assert!(rig.core.chat().state().messages.is_empty());
    assert!(rig.recorder.threads.lock().unwrap().last().is_some_and(|state| state.messages.is_empty()));

    // Entra otra persona y llega al mismo canal: antes de pedir nada, no
    // tiene delante lo que se le trajo a la anterior.
    let bea = serde_json::from_value(json!({ "id": "u3", "username": "bea" })).unwrap();
    rig.core.sign_in(bea, "access".into(), "refresh".into()).unwrap();
    rig.core.open_workspace().await;
    rig.core.comms().enter();
    let state = rig.core.chat().state();
    assert_eq!(state.conversation, Some(channel("a")));
    assert!(state.messages.is_empty() && state.loading, "quedó la conversación de quien salió");
    rig.core.stream().stop();
}

/// Una lectura que se pidió antes que otra y llega después trae lo de
/// antes: no se pone.
///
/// Rotura que atrapa: un mensaje retirado que vuelve a aparecer porque la
/// respuesta de una lectura vieja llegó la última.
#[tokio::test(flavor = "multi_thread")]
async fn a_reading_asked_earlier_does_not_bring_back_what_was_withdrawn() {
    let rig = rig().await;
    let both = json!([line("m1", "u2", "hola", 1), line("m2", "u2", "esto se retira", 2)]);
    get(CHAT_A).respond_with(ok(both.clone())).up_to_n_times(1).mount(&rig.server).await;
    // La lectura lenta vio los dos; la rápida, pedida después, ya sólo uno.
    get(CHAT_A).respond_with(ok(both).set_delay(Duration::from_millis(500))).up_to_n_times(1).mount(&rig.server).await;
    get(CHAT_A).respond_with(ok(json!([line("m1", "u2", "hola", 1)]))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert_eq!(ids(&rig.core.chat().state()), ["m1", "m2"]);

    let slow = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().refresh().await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    rig.core.chat().refresh().await;
    assert_eq!(ids(&rig.core.chat().state()), ["m1"]);
    slow.await.unwrap();
    assert_eq!(ids(&rig.core.chat().state()), ["m1"], "una lectura vieja devolvió el mensaje retirado");
    rig.core.stream().stop();
}

/// Lo que llega mientras se mira queda leído aunque la cuenta de sin leer
/// se hubiera pedido antes de marcarlo y conteste después.
///
/// Rotura que atrapa: un «1» en la fila del canal que se tiene delante,
/// con el mensaje ya leído, hasta que alguien escribe otra vez.
#[tokio::test(flavor = "multi_thread")]
async fn a_count_asked_before_marking_does_not_bring_the_badge_back() {
    let rig = rig_counting_slowly(Duration::from_millis(350)).await;
    let talk = history(vec![line("m1", "u2", "hola", 1)]);
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    let frame = "event: chat:message\ndata: {\"spaceId\":\"a\",\"messageId\":\"m2\",\"actorId\":\"u2\",\"kind\":\"user\",\"edited\":false}\n\n";
    get("/api/v1/events")
        .respond_with(ResponseTemplate::new(200).set_body_raw(frame.as_bytes().to_vec(), "text/event-stream").set_delay(Duration::from_millis(1300)))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    get("/api/v1/events").respond_with(ResponseTemplate::new(200).set_body_raw(Vec::new(), "text/event-stream").set_delay(Duration::from_secs(30))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(unread_of(&rig.core, "a"), 0);

    // Llega uno: el servidor lo cuenta sin leer hasta que se marque.
    talk.lock().unwrap().push(line("m2", "u2", "¿sale hoy?", 2));
    rig.unread_a.store(1, Ordering::SeqCst);
    until_hits(&rig.server, "POST", READ_A, 2).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(ids(&rig.core.chat().state()), ["m1", "m2"]);
    assert_eq!(unread_of(&rig.core, "a"), 0, "el contador volvió con una cuenta de antes de marcar");
    rig.core.stream().stop();
}

/// Si tras un corte ya no queda en el servidor nada de lo que se tenía, o
/// queda demasiado lejos para unirlo, lo que se pinta es lo que hay ahora:
/// entero y sin restos.
///
/// Roturas que atrapa: mensajes retirados durante el corte que siguen
/// pintados por ser «páginas viejas»; decir que queda historia cuando se
/// leyó hasta el principio; pedir páginas sin fin para unir dos extremos de
/// un canal con miles de mensajes nuevos, o dejar pintado lo viejo pegado a
/// lo último como si fueran seguidos.
#[tokio::test(flavor = "multi_thread")]
async fn what_can_no_longer_be_joined_is_replaced_by_what_there_is_now() {
    let rig = rig().await;
    let talk = history(numbered("h", 0..3));
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert_eq!(rig.core.chat().state().messages.len(), 3);

    // Se retira todo lo que se tenía y se dicen sesenta cosas.
    *talk.lock().unwrap() = numbered("n", 100..160);
    rig.core.chat().refresh().await;
    let state = rig.core.chat().state();
    assert_eq!(ids(&state), (100..160).map(|index| format!("n{index:03}")).collect::<Vec<_>>());
    assert!(!state.has_more, "se leyó hasta el principio");

    // Y luego, setecientas más: demasiadas para ir a buscar el empalme.
    talk.lock().unwrap().extend(numbered("z", 200..900));
    let asked = hits(&rig.server, "GET", CHAT_A).await;
    rig.core.chat().refresh().await;
    let state = rig.core.chat().state();
    assert_eq!(hits(&rig.server, "GET", CHAT_A).await - asked, 11);
    assert!(state.has_more);
    assert_eq!(state.messages.len(), 550);
    assert!(state.messages.iter().all(|message| message.id.starts_with('z')), "lo viejo quedó pegado a lo último");
    assert_eq!(state.messages.last().map(|message| message.id.as_str()), Some("z899"));
    rig.core.stream().stop();
}

/// Una página anterior que llega cuando lo que había debajo ya se cambió
/// por otra cosa no se pega delante: era «lo de antes» de una lista que ya
/// no está.
///
/// Rotura que atrapa: tras volver de un corte en el que se retiró todo lo
/// que se tenía, la página vieja que se pidió subiendo aparece pegada encima
/// de lo de ahora, con mensajes que ya no existen.
#[tokio::test(flavor = "multi_thread")]
async fn an_older_page_for_a_list_that_was_replaced_is_dropped() {
    let rig = rig().await;
    let talk = history(numbered("m", 0..60));
    // La primera página anterior que se pida tarda, y trae lo viejo.
    get(CHAT_A)
        .and(|request: &Request| request.url.query_pairs().any(|(name, _)| name == "before"))
        .respond_with(ok(json!(numbered("m", 0..10))).set_delay(Duration::from_millis(600)))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert_eq!(rig.core.chat().state().messages[0].id, "m010");

    let older = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().load_older().await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Mientras sube, se retira todo y se dicen sesenta cosas: el refresco
    // ya no puede unir y se queda con lo de ahora.
    *talk.lock().unwrap() = numbered("n", 100..160);
    rig.core.chat().refresh().await;
    assert_eq!(rig.core.chat().state().messages[0].id, "n100");
    older.await.unwrap();

    let state = rig.core.chat().state();
    assert!(state.messages.iter().all(|message| message.id.starts_with('n')), "la página vieja se pegó encima de lo de ahora");
    assert_eq!(state.messages.len(), 60);
    assert!(!state.loading_older);
    rig.core.stream().stop();
}

/// Lo que llega mientras la conversación se está abriendo no se pierde: si
/// hubo que releer a media apertura, se relee al acabar de abrir.
///
/// Rotura que atrapa: un mensaje que llega con la primera página en camino
/// (red lenta, recién elegido el canal) y que no se ve hasta que alguien
/// escribe otra vez.
#[tokio::test(flavor = "multi_thread")]
async fn what_arrives_while_opening_is_not_lost() {
    let rig = rig().await;
    let talk = history(vec![line("m1", "u2", "hola", 1)]);
    // La primera página se calcula al pedirla y llega mucho después.
    mount_history(&rig.server, CHAT_A, &talk, Duration::from_millis(1200)).await;
    let frame = "event: chat:message\ndata: {\"spaceId\":\"a\",\"messageId\":\"m2\",\"actorId\":\"u2\",\"kind\":\"user\",\"edited\":false}\n\n";
    get("/api/v1/events")
        .respond_with(ResponseTemplate::new(200).set_body_raw(frame.as_bytes().to_vec(), "text/event-stream").set_delay(Duration::from_millis(300)))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    get("/api/v1/events").respond_with(ResponseTemplate::new(200).set_body_raw(Vec::new(), "text/event-stream").set_delay(Duration::from_secs(30))).mount(&rig.server).await;
    rig.core.open_workspace().await;

    rig.core.comms().enter();
    assert!(rig.core.comms().select(channel("a")));
    let opening = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().follow().await }
    });
    // Con la página en camino, se escribe y llega el evento.
    tokio::time::sleep(Duration::from_millis(50)).await;
    talk.lock().unwrap().push(line("m2", "u2", "¿estás?", 2));
    opening.await.unwrap();
    until("llega lo que se escribió a media apertura", || ids(&rig.core.chat().state()) == ["m1", "m2"]).await;
    rig.core.stream().stop();
}

/// Un evento que llega solo —sin un mensaje nuevo al lado que haga releer—
/// pone al día la conversación que se tiene delante.
async fn one_event_alone_is_read(conversation: Conversation, route: &str, frame: &str, before: Value, after: Value, seen: impl Fn(&ThreadState) -> bool) -> u64 {
    let rig = rig().await;
    let talk = history(vec![before]);
    mount_history(&rig.server, route, &talk, Duration::ZERO).await;
    get("/api/v1/events")
        .respond_with(ResponseTemplate::new(200).set_body_raw(frame.as_bytes().to_vec(), "text/event-stream").set_delay(Duration::from_millis(400)))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    // Y el stream no vuelve: ninguna vuelta de un corte relee por su cuenta.
    get("/api/v1/events").respond_with(ResponseTemplate::new(200).set_body_raw(Vec::new(), "text/event-stream").set_delay(Duration::from_secs(30))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, conversation).await;
    *talk.lock().unwrap() = vec![after];
    until("se ve lo que cambió", || seen(&rig.core.chat().state())).await;
    let arrivals = rig.core.chat().state().arrivals;
    rig.core.stream().stop();
    arrivals
}

/// Una reacción en el canal abierto se ve sin esperar a otro mensaje.
///
/// Rotura que atrapa: `chat:reaction` fuera de los eventos que releen (la
/// reacción sólo aparece cuando alguien escribe).
#[tokio::test(flavor = "multi_thread")]
async fn a_reaction_in_a_channel_is_seen_on_its_own() {
    let mut reacted = line("m1", "u2", "hola", 1);
    reacted["reactions"] = json!([{ "emoji": "❤️", "count": 1, "mine": false, "names": ["Luis Mora"] }]);
    let frame = "event: chat:reaction\ndata: {\"spaceId\":\"a\",\"messageId\":\"m1\",\"actorId\":\"u2\"}\n\n";
    let arrivals = one_event_alone_is_read(channel("a"), CHAT_A, frame, line("m1", "u2", "hola", 1), reacted, |state| state.messages.first().is_some_and(|message| message.reactions.len() == 1)).await;
    assert_eq!(arrivals, 0, "una reacción se anunció como mensaje nuevo");
}

/// Lo mismo en un directo, que tiene su propio evento.
///
/// Rotura que atrapa: `dm:reaction` fuera de los eventos que releen.
#[tokio::test(flavor = "multi_thread")]
async fn a_reaction_in_a_direct_is_seen_on_its_own() {
    let mut reacted = private_line("d1", "u2", "¿vienes?", 1);
    reacted["reactions"] = json!([{ "emoji": "👍", "count": 1, "mine": false, "names": ["Luis Mora"] }]);
    let frame = "event: dm:reaction\ndata: {\"conversationId\":\"c1\",\"messageId\":\"d1\",\"actorId\":\"u2\"}\n\n";
    let arrivals = one_event_alone_is_read(direct("c1"), DM_C1, frame, private_line("d1", "u2", "¿vienes?", 1), reacted, |state| state.messages.first().is_some_and(|message| message.reactions.len() == 1)).await;
    assert_eq!(arrivals, 0);
}

/// Un mensaje editado cambia su texto donde está y no es una llegada.
///
/// Roturas que atrapa: el texto viejo pintado tras editarlo; «mensajes
/// nuevos» por una corrección de algo que ya se leyó.
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_changes_the_text_and_is_not_news() {
    let mut edited = line("m1", "u2", "hola, corregido", 1);
    edited["updatedAt"] = json!("2026-10-15T10:05:00Z");
    let frame = "event: chat:message\ndata: {\"spaceId\":\"a\",\"messageId\":\"m1\",\"actorId\":\"u2\",\"kind\":\"user\",\"edited\":true}\n\n";
    let arrivals = one_event_alone_is_read(channel("a"), CHAT_A, frame, line("m1", "u2", "hola", 1), edited, |state| state.messages.first().is_some_and(|message| message.body == "hola, corregido")).await;
    assert_eq!(arrivals, 0, "una edición se anunció como mensaje nuevo");
}

/// Lo que llega mientras se lee más arriba no se marca leído; al bajar al
/// final, sí. Abrir sigue marcando: se abre por el final.
///
/// Roturas que atrapa: marcar leído un mensaje que llegó fuera de la vista
/// (el contador se va sin que nadie lo haya visto); no marcarlo al bajar
/// (el contador se queda con todo leído); marcar dos veces lo mismo al
/// subir y bajar sin que llegue nada.
#[tokio::test(flavor = "multi_thread")]
async fn what_arrives_while_reading_above_is_marked_only_on_reaching_the_end() {
    let rig = rig().await;
    let talk = history(numbered("m", 0..60));
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 1);

    // Se sube a leer, y se baja sin que llegue nada: nada que marcar.
    rig.core.chat().at_tail(false).await;
    rig.core.chat().at_tail(true).await;
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 1, "se marcó otra vez sin que llegara nada");

    // Se sube, y llega algo.
    rig.core.chat().at_tail(false).await;
    talk.lock().unwrap().push(line("m060", "u2", "recién llegado", 60));
    rig.core.chat().refresh().await;
    assert_eq!(rig.core.chat().state().arrivals, 1);
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 1, "se marcó leído lo que llegó leyendo más arriba");

    // Al bajar, se marca.
    rig.core.chat().at_tail(true).await;
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 2, "al bajar al final no se marcó leído");
    // Y lo que llega estando abajo se marca solo.
    talk.lock().unwrap().push(line("m061", "u2", "otro", 61));
    rig.core.chat().refresh().await;
    assert_eq!(hits(&rig.server, "POST", READ_A).await, 3);
    rig.core.stream().stop();
}
