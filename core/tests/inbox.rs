//! La bandeja de la campana contra un servidor de mentira. Cada prueba dice
//! la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Config, Conversation, Core, InboxState, Listener, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[path = "support/gated.rs"]
mod gated;
use gated::{front, until, Gate};

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
    inbox: Mutex<Vec<InboxState>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn inbox_changed(&self, state: InboxState) {
        self.inbox.lock().unwrap().push(state);
    }
}

struct Rig {
    server: MockServer,
    core: Core,
    recorder: Arc<Recorder>,
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn get(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path(route.to_string()))
}

fn item(id: &str, group: &str, at: &str) -> Value {
    json!({ "id": id, "orgId": "o1", "kind": "chat:message", "title": "#producto", "body": format!("mensaje {id}"), "link": "/chat?space=a",
            "createdAt": at, "groupKey": group, "groupLabel": "#producto" })
}

/// Dos organizaciones; la bandeja de `o1` con dos avisos de `#producto` (de
/// los cuarenta que el servidor cuenta) y uno de una tarea; la de `o2`, uno.
async fn rig() -> Rig {
    rig_with(ok(json!(null))).await
}

async fn rig_with(read_all: ResponseTemplate) -> Rig {
    let server = MockServer::start().await;
    let org = |id: &str| json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" });
    get("/api/v1/organizations/").respond_with(ok(json!([org("o1"), org("o2")]))).mount(&server).await;
    let space = |id: &str, org: &str| json!({ "id": id, "orgId": org, "name": format!("Espacio {id}"), "color": "#888", "kind": null, "folders": [], "lists": [], "people": [] });
    get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(json!([space("a", "o1")]))).mount(&server).await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2")]))).mount(&server).await;
    let o1 = json!({
        "items": [item("n1", "chat:a", "2026-11-18T10:00:00Z"), item("n2", "chat:a", "2026-11-18T11:00:00Z"),
                  { "id": "n3", "orgId": "o1", "kind": "task:assigned", "title": "Te asignaron", "body": "Arreglar login", "link": "/tasks?task=t1", "createdAt": "2026-11-18T09:00:00Z" }],
        "unread": 41,
        "groups": [{ "key": "chat:a", "label": "#producto", "total": 40, "unread": 40 }],
    });
    get("/api/v1/notifications/").and(query_param("orgId", "o1")).respond_with(ok(o1)).mount(&server).await;
    let o2 = json!({ "items": [item("z1", "chat:z", "2026-11-18T10:00:00Z")], "unread": 1, "groups": [] });
    get("/api/v1/notifications/").and(query_param("orgId", "o2")).respond_with(ok(o2)).mount(&server).await;
    Mock::given(method("POST")).and(path("/api/v1/notifications/read")).respond_with(ok(json!(null))).mount(&server).await;
    Mock::given(method("POST")).and(path("/api/v1/notifications/read-all")).respond_with(read_all).mount(&server).await;
    let recorder = Arc::new(Recorder::default());
    let mut config = Config::new(front(&server.uri()));
    config.stream_backoff = Duration::from_millis(40);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    Rig { server, core, recorder }
}

/// Cuántas veces se pidió la bandeja entera (no la cuenta de la campana,
/// que pide `limit=1`).
async fn full_loads(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().into_iter().filter(|request| request.url.path() == "/api/v1/notifications/" && request.url.query().unwrap_or_default().contains("limit=50")).count()
}

async fn posted(server: &MockServer, route: &str) -> Vec<(String, Value)> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == "POST" && request.url.path() == route)
        .map(|request| (request.url.query().unwrap_or_default().to_string(), serde_json::from_slice(&request.body).unwrap_or(Value::Null)))
        .collect()
}

/// Leer baja el número en el acto —el de la bandeja y el de la campana— y se
/// lo dice al servidor: un aviso por su id, una conversación por su clave
/// (con todo lo que el servidor cuenta de ella, no lo cargado), y todo, en
/// la organización que se mira.
///
/// Roturas que atrapa: esperar al servidor para bajar el número; que la
/// campana se quede con el número viejo; marcar una conversación por los ids
/// que hay a mano (bajaría 2 y no 40); un doble clic que baja dos veces;
/// marcar todo sin decir de qué organización (el servidor marcaría las de
/// todas).
#[tokio::test]
async fn reading_lowers_the_count_at_once_and_tells_the_server() {
    let rig = rig().await;
    rig.core.open_workspace().await;
    rig.core.inbox().load().await;
    let unread = |core: &Core| core.inbox().state().feed.unread;
    let bell = |core: &Core| core.counters().state().notifications;
    assert_eq!(unread(&rig.core), 41);

    rig.core.inbox().mark_read(vec!["n3".into()]).await.unwrap();
    assert_eq!((unread(&rig.core), bell(&rig.core)), (40, Some(40)));
    rig.core.inbox().mark_read(vec!["n3".into()]).await.unwrap();
    assert_eq!(unread(&rig.core), 40, "leer otra vez lo leído no baja");

    rig.core.inbox().mark_group_read("chat:a".into()).await.unwrap();
    assert_eq!((unread(&rig.core), bell(&rig.core)), (0, Some(0)), "la conversación entera, la que cuenta el servidor");
    assert!(rig.core.inbox().state().feed.items.iter().all(|item| item.read_at.is_some()));

    rig.core.inbox().mark_all_read().await.unwrap();
    let read = posted(&rig.server, "/api/v1/notifications/read").await;
    assert_eq!(read.iter().map(|(_, body)| body.clone()).collect::<Vec<_>>(), [json!({ "ids": ["n3"] }), json!({ "ids": ["n3"] }), json!({ "group": "chat:a", "orgId": "o1" })]);
    assert_eq!(posted(&rig.server, "/api/v1/notifications/read-all").await[0].0, "orgId=o1");
    assert!(!rig.recorder.inbox.lock().unwrap().is_empty(), "la pantalla se entera");
    rig.core.stream().stop();
}

/// Si el servidor rechaza «todo leído», el número y las filas vuelven a lo que
/// el servidor dice: no se queda un cero que miente.
///
/// Roturas que atrapa: no volver a cargar tras el fallo (el cero y las filas
/// leídas se quedan); devolver Ok ocultando el fallo.
#[tokio::test]
async fn a_refused_mark_all_read_restores_what_the_server_says() {
    let rig = rig_with(ResponseTemplate::new(500)).await;
    rig.core.open_workspace().await;
    rig.core.inbox().load().await;
    assert_eq!(rig.core.inbox().state().feed.unread, 41);

    assert!(rig.core.inbox().mark_all_read().await.is_err());
    let state = rig.core.inbox().state();
    assert_eq!(state.feed.unread, 41);
    assert!(state.feed.items.iter().all(|item| item.read_at.is_none()));
    assert_eq!(full_loads(&rig.server).await, 2, "volvió a pedir la bandeja");
    rig.core.stream().stop();
}

/// La bandeja es de la organización que se mira: al cambiar, la de la nueva;
/// y no se pide hasta que alguien la abre.
///
/// Roturas que atrapa: pedirla sin organización (el servidor contesta con
/// la de todas); enseñar la de la anterior tras cambiar; pedirla en cada
/// arranque aunque nadie la mire.
#[tokio::test]
async fn the_inbox_is_the_organizations_and_only_when_wanted() {
    let rig = rig().await;
    rig.core.open_workspace().await;
    assert_eq!(full_loads(&rig.server).await, 0, "nadie la ha abierto");
    rig.core.inbox().load().await;
    assert_eq!(rig.core.inbox().state().feed.items.len(), 3);

    assert!(rig.core.switch_org("o2").await);
    let state = rig.core.inbox().state();
    assert_eq!((state.org_id.as_deref(), state.feed.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>()), (Some("o2"), vec!["z1"]));
    rig.core.stream().stop();
}

/// Pulsar un aviso lleva a su sitio, en su organización; uno de un sitio que
/// esta app no tiene no mueve nada.
///
/// Roturas que atrapa: no cambiar de organización; abrir algo con un enlace
/// que esta app no sabe seguir.
#[tokio::test]
async fn opening_a_notice_goes_where_it_says() {
    let rig = rig().await;
    get("/api/v1/chat/latest").respond_with(ok(json!([]))).mount(&rig.server).await;
    get("/api/v1/dm/").respond_with(ok(json!([]))).mount(&rig.server).await;
    rig.core.open_workspace().await;

    assert!(!rig.core.inbox().open("/servers/x/backups?target=b", "o2").await);
    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o1"), "un sitio que no está aquí no mueve nada");

    assert!(rig.core.inbox().open("/chat?space=z", "o2").await);
    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o2"));
    let comms = rig.core.comms().state();
    assert!(comms.on_screen);
    assert_eq!(comms.selected, Some(Conversation::Channel { space_id: "z".into() }));
    rig.core.stream().stop();
}

/// Con la bandeja abierta alguna vez, un evento que deja fila la vuelve a
/// pedir; sin abrirla, no.
///
/// Roturas que atrapa: una bandeja que no se mueve hasta cerrarla y abrirla;
/// pedirla con cada evento aunque nadie la mire.
#[tokio::test]
async fn an_event_that_rings_the_bell_reloads_an_open_inbox() {
    let rig = rig().await;
    let frame = |task: &str| format!("event: task:assigned\ndata: {{\"taskId\":\"{task}\"}}\n\n").into_bytes();
    // Un evento antes de abrir la bandeja y otro después, cada uno cuando la
    // prueba lo suelta: con plazos, en una máquina cargada el segundo podía
    // llegar antes de abrirla.
    let (before, after) = (Gate::default(), Gate::default());
    get("/api/v1/events").respond_with(before.hold(ResponseTemplate::new(200).set_body_raw(frame("t8"), "text/event-stream"))).up_to_n_times(1).mount(&rig.server).await;
    get("/api/v1/events").respond_with(after.hold(ResponseTemplate::new(200).set_body_raw(frame("t9"), "text/event-stream"))).up_to_n_times(1).mount(&rig.server).await;
    get("/api/v1/events").respond_with(Gate::default().hold(ResponseTemplate::new(200).set_body_raw(b": nada\n\n".to_vec(), "text/event-stream"))).mount(&rig.server).await;
    rig.core.open_workspace().await;
    before.open();
    until("sale el primer evento", || before.passed() == 1).await;
    // Lo que se mira es que no pase nada: se le da tiempo de sobra a la
    // recarga que no debe salir.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(full_loads(&rig.server).await, 0, "sin abrirla, un evento no la pide");

    rig.core.inbox().load().await;
    assert_eq!(full_loads(&rig.server).await, 1);
    after.open();
    for _ in 0..300 {
        if full_loads(&rig.server).await >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(full_loads(&rig.server).await, 2, "abierta, el evento la vuelve a pedir");
    rig.core.stream().stop();
}
