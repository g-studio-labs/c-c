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

#[path = "support/gated.rs"]
mod gated;
use gated::{Gate, GatedProxy};

#[path = "support/by_hand.rs"]
mod by_hand;

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
    /// Por donde habla el core: deja pasar todo, y retiene lo que una
    /// prueba ate a una [`Gate`].
    _proxy: GatedProxy,
    core: Core,
    recorder: Arc<Recorder>,
    /// Lo que el servidor cuenta sin leer en el canal `a`: tres, hasta que
    /// se marca leído. Y si ya se marcó leído el directo `c1`.
    unread_a: Arc<AtomicI64>,
    read_c1: Arc<AtomicBool>,
    /// Si la hay, la cuenta de sin leer se calcula al pedirla y no sale
    /// hasta que se abre esta puerta.
    held_counts: Arc<Mutex<Option<Gate>>>,
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

fn org(id: &str, role: &str) -> Value {
    json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": role, "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" })
}

fn space(id: &str, org_id: &str, name: &str, kind: Option<&str>) -> Value {
    json!({ "id": id, "orgId": org_id, "name": name, "color": "#888", "kind": kind, "folders": [], "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 0, "taskCount": 0 }], "people": [] })
}

/// Dos organizaciones. `o1`: la sala general `g`, `a` «Producto» y `b`
/// «Diseño», y un directo con Luis (`c1`). `o2`: `z` «Soporte» y un directo
/// con Zoe (`c9`). El canal `a` tiene tres sin leer y el directo `c1` dos,
/// hasta que se marcan leídos.
async fn rig() -> Rig {
    rig_with("member").await
}

/// Lo mismo, con ese papel en las dos organizaciones.
async fn rig_with(role: &str) -> Rig {
    let server = MockServer::start().await;
    let proxy = GatedProxy::start(server.uri());
    let recorder = Arc::new(Recorder::default());
    let mut config = Config::new(proxy.uri());
    config.stream_backoff = Duration::from_millis(40);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();

    get("/api/v1/organizations/").respond_with(ok(json!([org("o1", role), org("o2", role)]))).mount(&server).await;
    let o1 = json!([space("a", "o1", "Producto", None), space("b", "o1", "Diseño", None), space("g", "o1", "General", Some("general"))]);
    get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(o1)).mount(&server).await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2", "Soporte", None)]))).mount(&server).await;
    get("/api/v1/chat/latest").respond_with(ok(json!([]))).mount(&server).await;

    let (unread_a, read_c1) = (Arc::new(AtomicI64::new(3)), Arc::new(AtomicBool::new(false)));
    let (unread, held_counts) = (unread_a.clone(), Arc::new(Mutex::new(None::<Gate>)));
    let held = held_counts.clone();
    get("/api/v1/chat/unread")
        .respond_with(move |_: &Request| {
            let count = ok(json!([{ "spaceId": "a", "count": unread.load(Ordering::SeqCst) }, { "spaceId": "z", "count": 70 }]));
            match held.lock().unwrap().as_ref() {
                Some(gate) => gate.hold(count),
                None => count,
            }
        })
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
    Rig { server, _proxy: proxy, core, recorder, unread_a, read_c1, held_counts }
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
    get(route).respond_with(move |request: &Request| ok(json!(page_of(&history, request))).set_delay(delay)).mount(server).await;
}

/// La página que contesta el servidor a esa petición.
fn page_of(history: &History, request: &Request) -> Vec<Value> {
    let before = request.url.query_pairs().find(|(name, _)| name == "before").and_then(|(_, value)| DateTime::parse_from_rfc3339(&value).ok());
    let all = history.lock().unwrap().clone();
    let visible: Vec<Value> = all.into_iter().filter(|message| before.is_none_or(|before| instant(message) < before)).collect();
    visible[visible.len().saturating_sub(50)..].to_vec()
}

/// Lo mismo, con cada respuesta calculada al llegar la petición y retenida
/// por esa puerta.
async fn mount_history_held(server: &MockServer, route: &str, history: &History, gate: &Gate) {
    let (history, gate) = (history.clone(), gate.clone());
    get(route).respond_with(move |request: &Request| gate.hold(ok(json!(page_of(&history, request))))).mount(server).await;
}

/// Un stream que entrega esos eventos una vez, cuando la prueba abre la
/// puerta que devuelve, y se acaba. Con un plazo de llegada, en una máquina
/// cargada los eventos llegaban antes de lo que la prueba preparaba para
/// ellos.
async fn mount_events(server: &MockServer, frames: &str) -> Gate {
    let said = Gate::default();
    get("/api/v1/events")
        .respond_with(said.hold(ResponseTemplate::new(200).set_body_raw(frames.as_bytes().to_vec(), "text/event-stream")))
        .up_to_n_times(1)
        .mount(server)
        .await;
    said
}

/// Un stream que se queda abierto sin decir nada: ninguna vuelta de un corte
/// relee por su cuenta.
async fn mount_quiet_stream(server: &MockServer) {
    get("/api/v1/events").respond_with(Gate::default().hold(ResponseTemplate::new(200).set_body_raw(Vec::new(), "text/event-stream"))).mount(server).await;
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
    let said = mount_events(&rig.server, frames).await;
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
    said.open();
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
    let late = Gate::default();
    get(CHAT_A).respond_with(late.hold(ok(json!([line("a1", "u2", "de A", 1), line("a2", "u2", "de A, tarde", 2)])))).mount(&rig.server).await;
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
    until("A va en camino", || late.held() == 1).await;
    assert!(rig.core.comms().select(channel("b")));
    rig.core.chat().follow().await;
    late.open();
    slow.await.unwrap();

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
    // Una ráfaga de cinco.
    let burst = mount_events(&rig.server, &said("m060").repeat(5)).await;
    // Y luego, una retirada (el mismo evento, sin `edited`) y una reacción.
    let later = format!("{}event: chat:reaction\ndata: {{\"spaceId\":\"a\",\"messageId\":\"m058\",\"actorId\":\"u2\"}}\n\n", said("m059"));
    let then = mount_events(&rig.server, &later).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    rig.core.chat().load_older().await;
    assert_eq!(rig.core.chat().state().messages.len(), 60);
    let asked = hits(&rig.server, "GET", CHAT_A).await;

    talk.lock().unwrap().push(line("m060", "u2", "recién llegado", 60));
    burst.open();
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
    then.open();
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
    let said = mount_events(&rig.server, frames).await;
    mount_quiet_stream(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    said.open();
    until("los eventos salen", || said.passed() == 1).await;
    // Lo que se mira es que no pase nada: se le da tiempo de sobra a la
    // relectura que no debe salir.
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
    // Un stream que se acaba, y otro que vuelve.
    let cut = mount_events(&rig.server, ": hola\n\n").await;
    get("/api/v1/events").respond_with(ResponseTemplate::new(200).set_body_raw(b": de vuelta\n\n".to_vec(), "text/event-stream")).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert_eq!(ids(&rig.core.chat().state()), ["h000", "h001", "h002"]);

    // Durante el corte se dicen ciento veinte cosas.
    talk.lock().unwrap().extend(numbered("n", 100..220));
    cut.open();
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
    let late = Gate::default();
    mount_history_held(&rig.server, DM_C1, &talk, &late).await;
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
    until("la relectura va en camino", || late.held() == 1).await;
    assert!(rig.core.switch_org("o2").await);
    let state = rig.core.chat().state();
    assert_ne!(state.conversation, Some(direct("c1")));
    assert!(state.messages.is_empty(), "quedó el directo de la otra organización");
    late.open();
    slow.await.unwrap();

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
    // La primera página, retenida hasta que ya se salió.
    let first = Gate::default();
    let held = (talk.clone(), first.clone());
    get(CHAT_A).respond_with(move |request: &Request| held.1.hold(ok(json!(page_of(&held.0, request))))).up_to_n_times(1).mount(&rig.server).await;
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    let frame = "event: chat:message\ndata: {\"spaceId\":\"a\",\"messageId\":\"m2\",\"actorId\":\"u2\",\"kind\":\"user\",\"edited\":false}\n\n";
    let said = mount_events(&rig.server, frame).await;
    mount_quiet_stream(&rig.server).await;
    rig.core.open_workspace().await;

    // Se entra, y antes de que llegue la página se sale.
    rig.core.comms().enter();
    assert!(rig.core.comms().select(channel("a")));
    let opening = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().follow().await }
    });
    until("la primera página va en camino", || first.held() == 1).await;
    rig.core.comms().leave();
    rig.core.chat().follow().await;
    first.open();
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
    said.open();
    until("el evento sale", || said.passed() == 1).await;
    // Lo que se mira es que no pase nada: se le da tiempo de sobra a la
    // relectura que no debe salir.
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

/// Guardar un fichero de un mensaje lo escribe según llega, a un `.part`
/// al lado, y sólo le pone su nombre al terminar entero; con la sesión, y
/// sólo si es de este servidor.
///
/// Roturas que atrapa: juntar el fichero entero en memoria antes de
/// escribirlo; escribirlo con el nombre final a medias; dejar el `.part`;
/// pedirlo sin la sesión; pedir con la sesión una dirección que no es un
/// adjunto de este servidor.
#[tokio::test]
async fn saving_a_file_of_a_message_writes_it_as_it_arrives() {
    let file = "/api/v1/task-spaces/a/chat/attachments/a1/raw";
    let server = by_hand::serve(file, Vec::new()).await;
    let core = Core::new(Config::new(server.base.clone()), Arc::new(MemoryStorage::default()), Arc::new(Recorder::default())).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let foreign = dir.path().join("fuera.bin");
    for href in ["https://otro.example/api/v1/x/attachments/a/raw", "/api/v1/../admin"] {
        assert!(core.chat().save_file(href, &foreign).await.is_err(), "{href}");
    }
    assert!(!foreign.exists() && !by_hand::part_of(&foreign).exists());
    assert!(server.routes.lock().unwrap().is_empty(), "se pidió algo que no es un adjunto de este servidor: {:?}", server.routes.lock().unwrap());

    let target = dir.path().join("video.mp4");
    let half = 1_000_000;
    let whole: Vec<u8> = (0..2 * half).map(|at| (at % 251) as u8).collect();
    let mut first = format!("HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\n\r\n", whole.len()).into_bytes();
    first.extend_from_slice(&whole[..half]);
    server.send.send(first).unwrap();
    let saving = tokio::spawn({
        let (core, target) = (core.clone(), target.clone());
        async move { core.chat().save_file(file, &target).await }
    });

    let partial = by_hand::part_of(&target);
    let mut waited = 0;
    while std::fs::metadata(&partial).map(|meta| meta.len()).unwrap_or(0) < half as u64 {
        assert!(waited < 500, "la primera mitad no llegó a disco mientras faltaba la otra");
        tokio::time::sleep(Duration::from_millis(10)).await;
        waited += 1;
    }
    assert!(!target.exists(), "a medias ya lleva el nombre final");

    server.send.send(whole[half..].to_vec()).unwrap();
    drop(server.send);
    saving.await.unwrap().unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), whole);
    assert!(!partial.exists(), "se quedó el .part");
    let head = server.asked.await.unwrap().to_lowercase();
    assert!(head.contains("authorization: bearer access"), "{head}");
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
    let late = Gate::default();
    get(CHAT_A).respond_with(late.hold(ok(both))).up_to_n_times(1).mount(&rig.server).await;
    get(CHAT_A).respond_with(ok(json!([line("m1", "u2", "hola", 1)]))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert_eq!(ids(&rig.core.chat().state()), ["m1", "m2"]);

    let slow = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().refresh().await }
    });
    until("la lectura lenta va en camino", || late.held() == 1).await;
    rig.core.chat().refresh().await;
    assert_eq!(ids(&rig.core.chat().state()), ["m1"]);
    late.open();
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
    let rig = rig().await;
    let talk = history(vec![line("m1", "u2", "hola", 1)]);
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    let frame = "event: chat:message\ndata: {\"spaceId\":\"a\",\"messageId\":\"m2\",\"actorId\":\"u2\",\"kind\":\"user\",\"edited\":false}\n\n";
    let said = mount_events(&rig.server, frame).await;
    mount_quiet_stream(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    until("abrir lo deja en cero", || unread_of(&rig.core, "a") == 0).await;

    // Llega uno: el servidor lo cuenta sin leer hasta que se marque, y la
    // cuenta que se pide ahora no contesta hasta después de marcarlo.
    let counts = Gate::default();
    *rig.held_counts.lock().unwrap() = Some(counts.clone());
    talk.lock().unwrap().push(line("m2", "u2", "¿sale hoy?", 2));
    rig.unread_a.store(1, Ordering::SeqCst);
    said.open();
    until("se pide la cuenta", || counts.held() >= 1).await;
    until_hits(&rig.server, "POST", READ_A, 2).await;
    let asked = counts.held();
    counts.open();
    // La cuenta vieja puede poner el «1» un momento; lo que no puede es
    // quedarse: tras marcar se vuelve a contar. Se espera a esa cuenta de
    // después, no a un plazo (con 500 ms, en la caja no había salido aún).
    until("se vuelve a contar tras marcar", || counts.passed() > asked).await;
    until("el contador no vuelve con una cuenta de antes de marcar", || unread_of(&rig.core, "a") == 0).await;
    assert_eq!(ids(&rig.core.chat().state()), ["m1", "m2"]);
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
    let late = Gate::default();
    get(CHAT_A)
        .and(|request: &Request| request.url.query_pairs().any(|(name, _)| name == "before"))
        .respond_with(late.hold(ok(json!(numbered("m", 0..10)))))
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
    until("la página anterior va en camino", || late.held() == 1).await;
    // Mientras sube, se retira todo y se dicen sesenta cosas: el refresco
    // ya no puede unir y se queda con lo de ahora.
    *talk.lock().unwrap() = numbered("n", 100..160);
    rig.core.chat().refresh().await;
    assert_eq!(rig.core.chat().state().messages[0].id, "n100");
    late.open();
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
    // La primera página se calcula al pedirla y no sale hasta que la prueba
    // la suelta. Con un plazo de llegada, en una máquina cargada el aviso
    // llegaba antes de empezar a abrir (y se ignoraba, con razón) o la página
    // salía antes que el aviso.
    let page = Gate::default();
    mount_history_held(&rig.server, CHAT_A, &talk, &page).await;
    // El stream no entrega nada por su cuenta: el aviso lo entrega la prueba.
    mount_quiet_stream(&rig.server).await;
    rig.core.open_workspace().await;

    rig.core.comms().enter();
    assert!(rig.core.comms().select(channel("a")));
    let opening = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().follow().await }
    });
    until("la primera página va en camino", || page.held() == 1).await;
    // Con la página en camino, se escribe y llega el evento.
    talk.lock().unwrap().push(line("m2", "u2", "¿estás?", 2));
    rig.core.stream().deliver(cac_core::ServerEvent { event: "chat:message".into(), data: json!({ "spaceId": "a", "messageId": "m2", "actorId": "u2", "kind": "user", "edited": false }).to_string() });
    until("la apertura sabe que tiene que releer", || rig.core.chat().reread_pending()).await;
    page.open();
    opening.await.unwrap();
    assert_eq!(ids(&rig.core.chat().state()), ["m1", "m2"], "lo que se escribió a media apertura no se releyó al abrir");
    rig.core.stream().stop();
}

/// Un evento que llega solo —sin un mensaje nuevo al lado que haga releer—
/// pone al día la conversación que se tiene delante.
async fn one_event_alone_is_read(conversation: Conversation, route: &str, frame: &str, before: Value, after: Value, seen: impl Fn(&ThreadState) -> bool) -> u64 {
    let rig = rig().await;
    let talk = history(vec![before]);
    mount_history(&rig.server, route, &talk, Duration::ZERO).await;
    let said = mount_events(&rig.server, frame).await;
    // Y el stream no vuelve: ninguna vuelta de un corte relee por su cuenta.
    mount_quiet_stream(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, conversation).await;
    *talk.lock().unwrap() = vec![after];
    said.open();
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


// ── Escribir ────────────────────────────────────────────────────────────

/// Un servidor que, al recibir un mensaje en esa ruta, lo guarda en la
/// conversación (como hace el de verdad) y contesta con el mensaje en bruto:
/// sin el nombre de quien escribe ni la cita.
async fn mount_writes(server: &MockServer, route: &str, talk: &History, held: Option<&Gate>) {
    let (talk, held) = (talk.clone(), held.cloned());
    let direct = route.starts_with("/api/v1/dm/");
    post(route)
        .respond_with(move |request: &Request| {
            let sent: Value = serde_json::from_slice(&request.body).unwrap();
            let mut talk = talk.lock().unwrap();
            let next = talk.len() as i64 + 100;
            let mut written = if direct { private_line(&format!("w{next}"), "u1", sent["body"].as_str().unwrap(), next) } else { line(&format!("w{next}"), "u1", sent["body"].as_str().unwrap(), next) };
            written["authorName"] = json!("Ana Ruiz");
            talk.push(written.clone());
            let raw = json!({ "id": written["id"], "body": written["body"], "authorUserId": "u1", "createdAt": written["createdAt"], "updatedAt": written["updatedAt"], "replyToId": sent.get("replyToId") });
            let answer = ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": raw }));
            match &held {
                Some(gate) => gate.hold(answer),
                None => answer,
            }
        })
        .mount(server)
        .await;
}

fn sent_bodies(requests: &[Request]) -> Vec<Value> {
    requests.iter().map(|request| serde_json::from_slice(&request.body).unwrap()).collect()
}

/// Escribir en un canal manda lo escrito, sin espacios alrededor y con la
/// cita, por la ruta del canal; y el mensaje aparece en la conversación con
/// el nombre de quien lo escribió, sin esperar a ningún evento.
///
/// Roturas que atrapa: otra ruta; lo escrito con los espacios o saltos de
/// alrededor; la respuesta sin su `replyToId`; dar por buena la respuesta
/// del servidor (sin nombre ni cita) en vez de releer; no releer y esperar
/// al eco, que puede no llegar.
#[tokio::test(flavor = "multi_thread")]
async fn writing_in_a_channel_sends_the_text_and_the_reply_and_shows_it() {
    let rig = rig().await;
    let talk = history(vec![line("m1", "u2", "¿sale hoy?", 1)]);
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    mount_writes(&rig.server, CHAT_A, &talk, None).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert!(rig.core.chat().state().can_write && rig.core.chat().state().mentions_allowed);

    rig.core.chat().send(channel("a"), "\n  sí, **hoy**  \n".into(), Some("m1".into())).await.unwrap();
    let sent = sent_bodies(&requests(&rig.server, "POST", CHAT_A).await);
    assert_eq!(sent, [json!({ "body": "sí, **hoy**", "replyToId": "m1" })]);
    let state = rig.core.chat().state();
    let written = state.messages.last().expect("el mensaje no apareció");
    assert_eq!((written.body.as_str(), written.author_name.as_str()), ("sí, **hoy**", "Ana Ruiz"));
    assert!(!state.sending);

    // Sin respuesta, el cuerpo va solo.
    rig.core.chat().send(channel("a"), "otro".into(), None).await.unwrap();
    assert_eq!(sent_bodies(&requests(&rig.server, "POST", CHAT_A).await)[1], json!({ "body": "otro" }));
    rig.core.stream().stop();
}

/// Escribir en un directo va por su ruta, y lo escrito aparece aunque a
/// quien escribe no le llegue evento de lo suyo; la lista de directos se
/// vuelve a pedir, que es donde va su última línea. Ahí no se nombra a
/// nadie.
///
/// Roturas que atrapa: escribir un directo por la ruta de un canal; que lo
/// propio no se vea hasta que el otro conteste; la última línea del
/// directo, en la lista, sin lo que se acaba de decir; ofrecer menciones en
/// un directo.
#[tokio::test(flavor = "multi_thread")]
async fn writing_in_a_direct_goes_by_its_route_and_shows_without_an_event() {
    let rig = rig().await;
    let talk = history(vec![private_line("d1", "u2", "¿vienes?", 1)]);
    mount_history(&rig.server, DM_C1, &talk, Duration::ZERO).await;
    mount_writes(&rig.server, DM_C1, &talk, None).await;
    rig.core.open_workspace().await;
    open(&rig, direct("c1")).await;
    let state = rig.core.chat().state();
    assert!(state.can_write && !state.mentions_allowed);
    let lists = hits(&rig.server, "GET", "/api/v1/dm/").await;

    rig.core.chat().send(direct("c1"), "voy".into(), Some("d1".into())).await.unwrap();
    assert_eq!(sent_bodies(&requests(&rig.server, "POST", DM_C1).await), [json!({ "body": "voy", "replyToId": "d1" })]);
    assert_eq!(rig.core.chat().state().messages.last().map(|message| message.body.as_str()), Some("voy"));
    assert!(hits(&rig.server, "GET", "/api/v1/dm/").await > lists, "no se volvió a pedir la lista de directos");
    rig.core.stream().stop();
}

/// Lo que no se puede mandar no se pide: en blanco, o más largo de lo que
/// acepta el servidor, que cuenta caracteres y no bytes.
///
/// Roturas que atrapa: mandar un mensaje en blanco (el servidor lo
/// aceptaría: no recorta); contar bytes, que deja sin mandar un mensaje con
/// acentos que sí cabe; dejar pasar uno que el servidor rechaza con una
/// frase que nadie entiende.
#[tokio::test(flavor = "multi_thread")]
async fn what_cannot_be_sent_is_not_asked_for() {
    let rig = rig().await;
    let talk = history(Vec::new());
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    mount_writes(&rig.server, CHAT_A, &talk, None).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;

    let code = |result: Result<(), cac_core::ApiError>| match result {
        Err(cac_core::ApiError::Server { code, .. }) => code,
        other => panic!("se esperaba un rechazo: {other:?}"),
    };
    assert_eq!(code(rig.core.chat().send(channel("a"), "  \n\t ".into(), None).await), "empty-body");
    assert_eq!(code(rig.core.chat().send(channel("a"), "a".repeat(8001), None).await), "too-long");
    assert_eq!(hits(&rig.server, "POST", CHAT_A).await, 0);
    // Ocho mil letras con acento son dieciséis mil bytes, y caben.
    rig.core.chat().send(channel("a"), "é".repeat(8000), None).await.unwrap();
    assert_eq!(hits(&rig.server, "POST", CHAT_A).await, 1);
    rig.core.stream().stop();
}

/// Mientras un mensaje va de camino no se manda otro en esa conversación:
/// el servidor no reconoce un envío repetido y lo publicaría dos veces. En
/// otra conversación, sí se puede.
///
/// Roturas que atrapa: el mismo mensaje dos veces por un Enter doble con la
/// red lenta; no decir que se está enviando (el botón no se apagaría); que
/// un envío en un canal bloquee los demás.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_on_its_way_is_not_sent_twice() {
    let rig = rig().await;
    let talk = history(Vec::new());
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    let on_its_way = Gate::default();
    mount_writes(&rig.server, CHAT_A, &talk, Some(&on_its_way)).await;
    mount_writes(&rig.server, CHAT_B, &history(Vec::new()), None).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;

    let first = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().send(channel("a"), "hola".into(), None).await }
    });
    until("se dice que se está enviando", || rig.core.chat().state().sending).await;
    let again = rig.core.chat().send(channel("a"), "hola".into(), None).await;
    assert!(matches!(again, Err(cac_core::ApiError::Server { ref code, .. }) if code == "already-sending"), "{again:?}");
    rig.core.chat().send(channel("b"), "en otro canal".into(), None).await.unwrap();
    on_its_way.open();
    first.await.unwrap().unwrap();
    assert_eq!(hits(&rig.server, "POST", CHAT_A).await, 1);
    assert!(!rig.core.chat().state().sending);
    rig.core.stream().stop();
}

/// Un envío que falla devuelve por qué, y no toca la conversación.
///
/// Roturas que atrapa: tragarse el error (lo escrito se borraría de la caja
/// como si hubiera salido); quedarse «enviando» para siempre; releer por un
/// mensaje que no se publicó.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_send_says_why_and_leaves_the_conversation_alone() {
    let rig = rig().await;
    mount_history(&rig.server, CHAT_A, &history(vec![line("m1", "u2", "hola", 1)]), Duration::ZERO).await;
    post(CHAT_A)
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "success": false, "error": "reply-elsewhere", "message": "The message you reply to is not in this channel" })))
        .mount(&rig.server)
        .await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    let reads = hits(&rig.server, "GET", CHAT_A).await;

    let failed = rig.core.chat().send(channel("a"), "a eso".into(), Some("retirado".into())).await;
    assert!(matches!(failed, Err(cac_core::ApiError::Server { ref code, .. }) if code == "reply-elsewhere"), "{failed:?}");
    assert!(!rig.core.chat().state().sending);
    assert_eq!(hits(&rig.server, "GET", CHAT_A).await, reads, "se releyó por un mensaje que no salió");
    rig.core.stream().stop();
}

/// Quien sólo mira la organización no escribe en un canal, y no se le deja
/// intentarlo; en un directo, sí (el servidor no mira el papel ahí).
///
/// Roturas que atrapa: una caja abierta que acaba en «Forbidden» al
/// enviar; cerrarle también los directos.
#[tokio::test(flavor = "multi_thread")]
async fn a_viewer_writes_in_a_direct_but_not_in_a_channel() {
    let rig = rig_with("viewer").await;
    mount_history(&rig.server, CHAT_A, &history(Vec::new()), Duration::ZERO).await;
    let talk = history(Vec::new());
    mount_history(&rig.server, DM_C1, &talk, Duration::ZERO).await;
    mount_writes(&rig.server, DM_C1, &talk, None).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    assert!(!rig.core.chat().state().can_write);
    let refused = rig.core.chat().send(channel("a"), "hola".into(), None).await;
    assert!(matches!(refused, Err(cac_core::ApiError::Server { ref code, .. }) if code == "viewer-read-only"), "{refused:?}");
    assert_eq!(hits(&rig.server, "POST", CHAT_A).await, 0);

    let refused = rig.core.chat().attach(channel("a"), "nota.txt".into(), b"hola".to_vec()).await;
    assert!(matches!(refused, Err(cac_core::ApiError::Server { ref code, .. }) if code == "viewer-read-only"), "{refused:?}");
    assert_eq!(hits(&rig.server, "POST", "/api/v1/task-spaces/a/chat/attachments").await, 0);

    assert!(rig.core.comms().select(direct("c1")));
    rig.core.chat().follow().await;
    assert!(rig.core.chat().state().can_write);
    rig.core.chat().send(direct("c1"), "hola".into(), None).await.unwrap();
    assert_eq!(hits(&rig.server, "POST", DM_C1).await, 1);
    rig.core.stream().stop();
}

/// En un canal, `@` ofrece a la gente de la organización (sin quien
/// escribe), y la ficha que se pone vale el enlace que el servidor busca
/// para avisar y enseña el nombre. En un directo no se ofrece.
///
/// Roturas que atrapa: una ficha cuyo valor no es el enlace (el servidor
/// no avisaría a nadie); enseñar el usuario en vez del nombre; ofrecerse a
/// uno mismo; ofrecer menciones en un directo, donde no avisan.
#[tokio::test(flavor = "multi_thread")]
async fn naming_someone_puts_a_token_that_is_the_link_the_server_reads() {
    let rig = rig().await;
    let luis = "22222222-2222-4222-8222-222222222222";
    get("/api/v1/users/search")
        .respond_with(ok(json!([{ "id": "u1", "username": "ana", "name": "Ana Ruiz" }, { "id": luis, "username": "luis", "name": "Luis Mora" }, { "id": "u3", "username": "bea" }])))
        .mount(&rig.server)
        .await;
    rig.core.open_workspace().await;
    rig.core.comms().load_people().await;

    let menu = rig.core.chat().mention_menu(&channel("a"), "hola @lu", 8, None).expect("no se ofreció a nadie");
    assert_eq!(menu.people.iter().map(|person| person.username.as_str()).collect::<Vec<_>>(), ["luis"]);
    let everyone = rig.core.chat().mention_menu(&channel("a"), "@", 1, None).unwrap();
    assert!(!everyone.people.iter().any(|person| person.username == "ana"), "se ofreció a quien escribe");
    let token = cac_core::mention_token(&menu.people[0]);
    assert_eq!(token.label, "@Luis Mora");
    assert_eq!(token.text, format!("[@luis](cac:user/{luis})"));
    assert_eq!(cac_core::mentioned_user(&format!("cac:user/{luis}")), Some(luis));
    assert_eq!(token.id, luis);

    assert!(rig.core.chat().mention_menu(&direct("c1"), "hola @lu", 8, None).is_none(), "se ofrecieron menciones en un directo");
    rig.core.stream().stop();
}

/// Un fichero se sube por la ruta de su conversación, entero y con su
/// nombre, y su ficha lo cita como lo lee quien recibe el mensaje: una
/// imagen como imagen, lo demás como enlace, por la ruta del servidor y con
/// el nombre a salvo. Lo que no cabe no se sube.
///
/// Roturas que atrapa: subir a un directo por la ruta de un canal; el
/// fichero sin su nombre; una captura citada como enlace (no se vería); un
/// corchete del nombre que cierra el enlace; la cita con el dominio del
/// servidor (no se leería contra otro); subir treinta megas para que el
/// servidor los rechace.
#[tokio::test]
async fn a_file_goes_by_its_conversation_route_and_its_token_cites_it() {
    let rig = rig().await;
    let shot: Value = serde_json::from_str(include_str!("fixtures/chat_attachment.json")).unwrap();
    let notes: Value = serde_json::from_str(include_str!("fixtures/dm_attachment.json")).unwrap();
    post("/api/v1/task-spaces/a/chat/attachments").respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": shot }))).mount(&rig.server).await;
    post("/api/v1/dm/c1/attachments").respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": notes }))).mount(&rig.server).await;
    rig.core.open_workspace().await;

    let png = include_bytes!("fixtures/gray_600x200.png").to_vec();
    let file = rig.core.chat().attach(channel("a"), "captura [final].png".into(), png.clone()).await.unwrap();
    let asked = requests(&rig.server, "POST", "/api/v1/task-spaces/a/chat/attachments").await;
    assert_eq!(asked.len(), 1);
    let kind = asked[0].headers.get("content-type").unwrap().to_str().unwrap().to_string();
    assert!(kind.starts_with("multipart/form-data; boundary="), "{kind}");
    let sent = &asked[0].body;
    assert!(sent.windows(png.len()).any(|window| window == png.as_slice()), "el fichero no fue entero");
    assert!(String::from_utf8_lossy(sent).contains("filename=\"captura [final].png\""));
    let token = cac_core::file_token(&file).unwrap();
    assert_eq!(token.text, "![captura \\[final\\].png](/api/v1/task-spaces/e8bd6d46-a9ab-494e-ac29-de84d80a55f4/chat/attachments/f1/raw)");
    assert_eq!((token.label.as_str(), token.id.as_str()), ("captura [final].png", "f1"));

    let file = rig.core.chat().attach(direct("c1"), "acta.pdf".into(), b"%PDF-1.4".to_vec()).await.unwrap();
    assert_eq!(hits(&rig.server, "POST", "/api/v1/dm/c1/attachments").await, 1);
    assert_eq!(cac_core::file_token(&file).unwrap().text, "[acta.pdf](/api/v1/dm/c1/attachments/f2/raw)");

    // Lo que no cabe no se pide.
    let too_big = rig.core.chat().attach(channel("a"), "grande.zip".into(), vec![0; (30 << 20) + 1]).await;
    assert!(matches!(too_big, Err(cac_core::ApiError::Server { ref code, .. }) if code == "too-large"), "{too_big:?}");
    assert_eq!(hits(&rig.server, "POST", "/api/v1/task-spaces/a/chat/attachments").await, 1);
    // Un fichero que el servidor dice que vive en otro sitio no se cita.
    let elsewhere = cac_core::ChatFile { id: "x".into(), url: "https://otro.example/f.png".into(), file_name: "f.png".into(), ..Default::default() };
    assert!(cac_core::file_token(&elsewhere).is_none());
    rig.core.stream().stop();
}


// ── Corregir, retirar y reaccionar ──────────────────────────────────────

/// Corregir lo propio manda sólo el texto nuevo, por la ruta del mensaje, y
/// lo corregido se ve; lo mismo, en blanco o igual que estaba no se pide.
///
/// Roturas que atrapa: corregir un directo por la ruta de un canal; mandar
/// una corrección que no cambia nada; borrar un mensaje dejándolo en
/// blanco; no releer (se seguiría viendo lo de antes).
#[tokio::test(flavor = "multi_thread")]
async fn editing_sends_only_a_change_and_shows_it() {
    let rig = rig().await;
    let talk = history(vec![line("m1", "u1", "hola", 1)]);
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    let edits = talk.clone();
    Mock::given(method("PATCH"))
        .and(path("/api/v1/task-spaces/a/chat/m1"))
        .respond_with(move |request: &Request| {
            let sent: Value = serde_json::from_slice(&request.body).unwrap();
            let mut talk = edits.lock().unwrap();
            talk[0]["body"] = sent["body"].clone();
            talk[0]["updatedAt"] = json!("2026-10-15T10:05:00Z");
            ok(json!(null))
        })
        .mount(&rig.server)
        .await;
    Mock::given(method("PATCH")).and(path("/api/v1/dm/c1/messages/d1")).respond_with(ok(json!(null))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;

    rig.core.chat().edit(channel("a"), "m1".into(), " hola, corregido ".into()).await.unwrap();
    let patched = requests(&rig.server, "PATCH", "/api/v1/task-spaces/a/chat/m1").await;
    assert_eq!(sent_bodies(&patched), [json!({ "body": "hola, corregido" })]);
    assert_eq!(rig.core.chat().state().messages[0].body, "hola, corregido");

    rig.core.chat().edit(channel("a"), "m1".into(), "hola, corregido".into()).await.unwrap();
    assert!(rig.core.chat().edit(channel("a"), "m1".into(), "  ".into()).await.is_err());
    assert_eq!(hits(&rig.server, "PATCH", "/api/v1/task-spaces/a/chat/m1").await, 1, "se mandó una corrección que no cambia nada");

    rig.core.chat().edit(direct("c1"), "d1".into(), "otro".into()).await.unwrap();
    assert_eq!(hits(&rig.server, "PATCH", "/api/v1/dm/c1/messages/d1").await, 1);
    rig.core.stream().stop();
}

/// Retirar va por la ruta del mensaje y el mensaje deja de verse; antes se
/// pregunta, con lo que implica en un canal y en un directo.
///
/// Roturas que atrapa: retirar por la ruta equivocada; seguir pintando lo
/// retirado; la pregunta de un canal en un directo («para todo el mundo»
/// cuando son dos).
#[tokio::test(flavor = "multi_thread")]
async fn withdrawing_asks_what_it_implies_and_the_message_goes() {
    let rig = rig().await;
    let talk = history(vec![line("m1", "u1", "hola", 1), line("m2", "u1", "esto se va", 2)]);
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    let gone = talk.clone();
    Mock::given(method("DELETE"))
        .and(path("/api/v1/task-spaces/a/chat/m2"))
        .respond_with(move |_: &Request| {
            gone.lock().unwrap().retain(|message| message["id"] != "m2");
            ok(json!(null))
        })
        .mount(&rig.server)
        .await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;

    let channel_question = rig.core.chat().withdraw_question(&channel("a"), cac_core::Locale::Es);
    let direct_question = rig.core.chat().withdraw_question(&direct("c1"), cac_core::Locale::Es);
    assert_eq!(channel_question.title, "¿Retirar este mensaje?");
    assert_eq!(channel_question.body, "Deja de salir en el canal para todo el mundo.");
    assert_eq!(direct_question.body, "Deja de salir para los dos.");
    assert_eq!(channel_question.confirm, "Retirar");

    rig.core.chat().withdraw(channel("a"), "m2".into()).await.unwrap();
    assert_eq!(hits(&rig.server, "DELETE", "/api/v1/task-spaces/a/chat/m2").await, 1);
    assert_eq!(ids(&rig.core.chat().state()), ["m1"], "lo retirado se sigue viendo");
    rig.core.stream().stop();
}

/// Una reacción se ve en el acto, antes de que conteste el servidor; si el
/// servidor dice que no, vuelve a como estaba. Sólo las del juego.
///
/// Roturas que atrapa: esperar al servidor para pintar lo que se pulsó; una
/// reacción rechazada que se queda pintada; mandar el `on` al revés;
/// mandar un emoji que el servidor rechaza.
#[tokio::test(flavor = "multi_thread")]
async fn a_reaction_is_seen_at_once_and_undone_if_refused() {
    let rig = rig().await;
    let mut first = line("m1", "u2", "hola", 1);
    first["reactions"] = json!([{ "emoji": "👍", "count": 1, "mine": false, "names": ["Luis Mora"] }]);
    let talk = history(vec![first]);
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    let (reacted, answer) = (talk.clone(), Gate::default());
    let held = answer.clone();
    Mock::given(method("POST"))
        .and(path("/api/v1/task-spaces/a/chat/m1/reactions"))
        .and(|request: &Request| String::from_utf8_lossy(&request.body).contains("👍"))
        .respond_with(move |_: &Request| {
            reacted.lock().unwrap()[0]["reactions"] = json!([{ "emoji": "👍", "count": 2, "mine": true, "names": ["Luis Mora", "Ana Ruiz"] }]);
            held.hold(ok(json!(null)))
        })
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/task-spaces/a/chat/m1/reactions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": "boom", "message": "Failed" })))
        .mount(&rig.server)
        .await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;

    let reacting = tokio::spawn({
        let core = rig.core.clone();
        async move { core.chat().react(channel("a"), "m1".into(), "👍".into(), true).await }
    });
    until("la reacción se ve en el acto", || rig.core.chat().state().messages[0].reactions.first().is_some_and(|reaction| reaction.count == 2 && reaction.mine)).await;
    assert_eq!(hits(&rig.server, "GET", CHAT_A).await, 1, "se esperó a releer para pintarla");
    answer.open();
    reacting.await.unwrap().unwrap();
    let sent = sent_bodies(&requests(&rig.server, "POST", "/api/v1/task-spaces/a/chat/m1/reactions").await);
    assert_eq!(sent, [json!({ "emoji": "👍", "on": true })]);

    // Una que el servidor rechaza vuelve a como estaba.
    let refused = rig.core.chat().react(channel("a"), "m1".into(), "🔥".into(), true).await;
    assert!(refused.is_err());
    assert_eq!(rig.core.chat().state().messages[0].reactions.len(), 1, "la reacción rechazada se quedó pintada");

    let outside = rig.core.chat().react(channel("a"), "m1".into(), "🦀".into(), true).await;
    assert!(matches!(outside, Err(cac_core::ApiError::Server { ref code, .. }) if code == "bad-emoji"), "{outside:?}");
    assert_eq!(hits(&rig.server, "POST", "/api/v1/task-spaces/a/chat/m1/reactions").await, 2);
    rig.core.stream().stop();
}

/// La cuenta de una reacción hecha aquí: poner suma y marca, quitar resta
/// y desmarca, y la que se queda a cero se va.
///
/// Roturas que atrapa: sumar dos veces la misma; quitar la de otro; dejar
/// una pastilla con cero.
#[test]
fn a_reaction_is_counted_here_like_the_server_would() {
    use cac_core::{apply_reaction, ReactionSummary};
    let one = |emoji: &str, count: i64, mine: bool, names: &[&str]| ReactionSummary { emoji: emoji.into(), count, mine, names: names.iter().map(|name| name.to_string()).collect() };
    let list = vec![one("👍", 1, false, &["Luis"])];
    assert_eq!(apply_reaction(&list, "👍", true, "Ana"), [one("👍", 2, true, &["Luis", "Ana"])]);
    let mine = apply_reaction(&list, "👍", true, "Ana");
    assert_eq!(apply_reaction(&mine, "👍", true, "Ana"), mine, "se sumó dos veces");
    assert_eq!(apply_reaction(&mine, "👍", false, "Ana"), list);
    assert_eq!(apply_reaction(&list, "👍", false, "Ana"), list, "se quitó la de otro");
    assert_eq!(apply_reaction(&list, "🎉", true, "Ana"), [one("👍", 1, false, &["Luis"]), one("🎉", 1, true, &["Ana"])]);
    let only_mine = vec![one("🎉", 1, true, &["Ana"])];
    assert!(apply_reaction(&only_mine, "🎉", false, "Ana").is_empty(), "quedó una pastilla con cero");
}


// ── Lo hablado, a trabajo ───────────────────────────────────────────────

/// Pasar un mensaje a tarea la crea en la lista del espacio del canal, con
/// la primera línea de título y el mensaje entero de cuerpo; una lista de
/// otro espacio no se pide nunca.
///
/// Roturas que atrapa: crearla en una lista de otro canal (aquí `l-b`); un
/// título que arrastra todas las líneas; un cuerpo que pierde las de
/// después de la primera; mandar visibilidad en una lista que no es de
/// cliente.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_becomes_a_task_in_a_list_of_its_own_channel() {
    let rig = rig().await;
    let talk = history(vec![line("m1", "u2", "revisar el contrato\ncon Luis el lunes", 1)]);
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    let created = json!({ "id": "t1", "createdAt": "2026-11-15T10:00:00Z", "updatedAt": "2026-11-15T10:00:00Z", "listId": "l-a", "seq": 1, "title": "x", "status": "pending", "priority": "medium" });
    post("/api/v1/task-lists/l-a/tasks").respond_with(ok(created.clone())).mount(&rig.server).await;
    post("/api/v1/task-lists/l-b/tasks").respond_with(ok(created)).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    let locale = cac_core::Locale::Es;

    let targets = rig.core.chat().work_targets(&channel("a"));
    assert_eq!(targets.iter().map(|target| target.list_id.as_str()).collect::<Vec<_>>(), ["l-a"]);

    let refused = rig.core.chat().task_from_message(channel("a"), "m1".into(), "l-b".into(), false, locale).await;
    assert!(refused.is_err());
    assert_eq!(hits(&rig.server, "POST", "/api/v1/task-lists/l-b/tasks").await, 0, "se creó en la lista de otro canal");

    rig.core.chat().task_from_message(channel("a"), "m1".into(), "l-a".into(), false, locale).await.unwrap();
    let sent = sent_bodies(&requests(&rig.server, "POST", "/api/v1/task-lists/l-a/tasks").await);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["title"], "revisar el contrato");
    let description = sent[0]["description"].as_str().unwrap();
    assert!(description.starts_with("revisar el contrato\ncon Luis el lunes\n\n— "), "{description}");
    assert!(description.contains("Producto"), "{description}");
    assert!(sent[0].get("visibility").is_none(), "{}", sent[0]);
    rig.core.stream().stop();
}

/// Un directo y la sala general no tienen dónde crear nada.
///
/// Rotura que atrapa: ofrecer las listas de toda la organización desde un
/// directo.
#[tokio::test(flavor = "multi_thread")]
async fn a_direct_has_no_lists_to_make_work_in() {
    let rig = rig().await;
    rig.core.open_workspace().await;
    assert!(rig.core.chat().work_targets(&direct("c1")).is_empty());
    assert!(rig.core.chat().work_targets(&channel("g")).is_empty());
    rig.core.stream().stop();
}

/// Guardar en la documentación añade el mensaje, citado, a la pestaña
/// pedida de la lista; la de decisiones no es de párrafos y no se pide.
///
/// Roturas que atrapa: mandarlo a otra pestaña; no citarlo; aceptar
/// `decisions`; una lista de otro espacio.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_is_saved_quoted_into_the_docs_of_a_list_of_its_channel() {
    let rig = rig().await;
    let talk = history(vec![line("m1", "u2", "usar el puerto 8443", 1)]);
    mount_history(&rig.server, CHAT_A, &talk, Duration::ZERO).await;
    post("/api/v1/docs/list/l-a/tabs/runbook/append").respond_with(ok(json!({}))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    open(&rig, channel("a")).await;
    let locale = cac_core::Locale::Es;

    assert!(rig.core.chat().save_message_to_docs(channel("a"), "m1".into(), "l-a".into(), "decisions".into(), locale).await.is_err());
    assert!(rig.core.chat().save_message_to_docs(channel("a"), "m1".into(), "l-b".into(), "runbook".into(), locale).await.is_err());
    assert_eq!(hits(&rig.server, "POST", "/api/v1/docs/list/l-a/tabs/runbook/append").await, 0);

    rig.core.chat().save_message_to_docs(channel("a"), "m1".into(), "l-a".into(), "runbook".into(), locale).await.unwrap();
    let sent = sent_bodies(&requests(&rig.server, "POST", "/api/v1/docs/list/l-a/tabs/runbook/append").await);
    let body = sent[0]["body"].as_str().unwrap();
    assert!(body.starts_with("> usar el puerto 8443\n\n— "), "{body}");
    rig.core.stream().stop();
}


// ── La tarjeta de una llamada ───────────────────────────────────────────

/// La tarjeta sale de lo que el servidor cuenta de esa llamada, por su ruta;
/// y algo que no es una llamada de verdad es un fallo, no una tarjeta vacía.
///
/// Roturas que atrapa: pedir otra ruta; pintar una tarjeta de una respuesta
/// sin id (un servidor de antes); perder la grabación.
#[tokio::test(flavor = "multi_thread")]
async fn a_call_summary_is_built_from_what_the_server_says_of_the_call() {
    let rig = rig().await;
    let id = "0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8";
    get(&format!("/api/v1/call-sessions/{id}"))
        .respond_with(ok(json!({
            "id": id, "startedAt": "2026-11-25T10:02:00Z", "endedAt": "2026-11-25T10:14:00Z", "durationMs": 720000, "messages": 0,
            "participants": [{ "identity": "u1", "name": "Ana Ruiz", "guest": false }], "recordingId": "r1"
        })))
        .mount(&rig.server)
        .await;
    get("/api/v1/call-sessions/viejo").respond_with(ok(json!({ "id": "" }))).mount(&rig.server).await;

    let card = rig.core.chat().call_card(id, cac_core::Locale::Es).await.unwrap();
    assert_eq!(card.recording_id.as_deref(), Some("r1"));
    assert_eq!(card.faces.len(), 1);
    assert!(card.when.starts_with("12 min"), "{}", card.when);
    assert!(rig.core.chat().call_card("viejo", cac_core::Locale::Es).await.is_err());
    rig.core.stream().stop();
}
