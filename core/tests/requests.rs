//! «Por atender» contra un servidor de mentira. Cada prueba dice la rotura
//! que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{AnswerDraft, Config, Core, Listener, RequestScope, ServerEvent, Session, Storage};
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
    shown: Mutex<Vec<String>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn show_request(&self, code: String) {
        self.shown.lock().unwrap().push(code);
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

fn request(code: &str, task: &str, of: (&str, &str), mine: bool) -> Value {
    json!({
        "id": format!("id-{code}"), "code": code, "orgId": "o1", "kind": "approve_message", "summary": format!("resumen {code}"),
        "body": { "draft": "Hola equipo", "destination": "Basecamp" }, "version": 1, "status": "open",
        "task": { "id": task, "ref": "cac-1", "title": "Tarea", "priority": "high", "status": "in_progress", "link": format!("/tasks?task={task}") },
        "requestedOf": { "id": of.0, "name": of.1 }, "requestedBy": { "id": "bot", "name": "Agente" }, "via": "mcp",
        "createdAt": "2026-11-24T10:00:00Z", "updatedAt": "2026-11-24T10:00:00Z", "canAnswer": mine, "link": format!("/requests?open={code}"),
    })
}

fn list(scope: &str, status: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path("/api/v1/requests")).and(query_param("scope", scope)).and(query_param("status", status))
}

/// Ana tiene dos abiertos (en `t1` y `t2`); Luis, uno en `t1`. Resueltos, ninguno.
async fn rig() -> Rig {
    let server = MockServer::start().await;
    let ana = ("u1", "Ana Ruiz");
    let team = json!([request("req-a1", "t1", ana, true), request("req-l1", "t1", ("u2", "Luis"), false), request("req-a2", "t2", ana, true)]);
    list("team", "open").respond_with(ok(team)).mount(&server).await;
    list("mine", "open").respond_with(ok(json!([request("req-a1", "t1", ana, true), request("req-a2", "t2", ana, true)]))).mount(&server).await;
    list("mine", "closed").respond_with(ok(json!([]))).mount(&server).await;
    list("team", "closed").respond_with(ok(json!([]))).mount(&server).await;
    let recorder = Arc::new(Recorder::default());
    let core = Core::new(Config::new(front(&server.uri())), Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    Rig { server, core, recorder }
}

async fn posted(server: &MockServer, route: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == "POST" && request.url.path() == route)
        .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
        .collect()
}

async fn team_loads(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().into_iter().filter(|request| request.url.query().unwrap_or_default().contains("scope=team&status=open")).count()
}

/// El número y las marcas salen de los abiertos de todos, y el número sigue
/// siendo tuyo mirando «Equipo».
///
/// Roturas que atrapa: contar los de los demás en tu número; dejar que el
/// filtro «Equipo» lo cambie; una marca por pedido en vez de por persona.
#[tokio::test]
async fn the_count_is_yours_and_each_task_says_who_it_waits_on() {
    let rig = rig().await;
    rig.core.requests().refresh_marks().await;
    let state = rig.core.requests().state();
    assert_eq!(state.mine_count, 2, "dos de Ana, no los tres abiertos");
    assert_eq!(state.waiting_on.get("t1"), Some(&vec!["Ana Ruiz".to_string(), "Luis".to_string()]));
    assert_eq!(state.waiting_on.get("t2"), Some(&vec!["Ana Ruiz".to_string()]));

    rig.core.requests().load().await;
    assert_eq!(rig.core.requests().state().open.len(), 2);
    rig.core.requests().set_scope(RequestScope::Team).await;
    let state = rig.core.requests().state();
    assert_eq!((state.open.len(), state.mine_count), (3, 2), "«Equipo» enseña los tres; el número sigue siendo de Ana");
}

/// Contestar lo pasa a «Resueltos» en el acto y baja el número y la marca;
/// lo que el tipo exige y falta no se manda.
///
/// Roturas que atrapa: mandar un rechazo sin motivo; dejarlo en abiertos
/// hasta el evento; no bajar el número o la marca de su tarea.
#[tokio::test]
async fn answering_moves_it_to_resolved_and_lowers_the_count() {
    let rig = rig().await;
    let mut done = request("req-a1", "t1", ("u1", "Ana Ruiz"), false);
    done["status"] = json!("answered");
    done["answer"] = json!({ "approved": false, "reason": "el tono" });
    Mock::given(method("POST")).and(path("/api/v1/requests/req-a1/answer")).respond_with(ok(done)).mount(&rig.server).await;
    rig.core.requests().refresh_marks().await;
    rig.core.requests().load().await;

    assert!(rig.core.requests().answer("req-a1", AnswerDraft::Reject { reason: "  ".into() }).await.is_err());
    assert!(posted(&rig.server, "/api/v1/requests/req-a1/answer").await.is_empty(), "sin motivo no sale");

    rig.core.requests().answer("req-a1", AnswerDraft::Reject { reason: " el tono ".into() }).await.unwrap();
    assert_eq!(posted(&rig.server, "/api/v1/requests/req-a1/answer").await, [json!({ "approved": false, "reason": "el tono" })]);
    let state = rig.core.requests().state();
    assert_eq!(state.open.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-a2"]);
    assert_eq!(state.closed.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-a1"]);
    assert_eq!(state.mine_count, 1);
    assert_eq!(state.waiting_on.get("t1"), Some(&vec!["Luis".to_string()]), "Ana ya no espera en t1; Luis sí");
}

/// Una lista que llega tarde no pisa a la de después: un evento pide otra
/// mientras la primera sigue en camino.
///
/// Rotura que atrapa: poner lo que llegue sin mirar si es la carga vigente
/// (la bandeja enseñaría un pedido ya contestado).
#[tokio::test]
async fn a_late_list_does_not_overwrite_a_newer_one() {
    let rig = rig().await;
    let gate = Gate::default();
    let stale = json!([request("req-a1", "t1", ("u1", "Ana Ruiz"), true), request("req-a2", "t2", ("u1", "Ana Ruiz"), true)]);
    list("mine", "open").respond_with(gate.hold(ok(stale))).up_to_n_times(1).with_priority(1).mount(&rig.server).await;
    list("mine", "open").respond_with(ok(json!([request("req-a2", "t2", ("u1", "Ana Ruiz"), true)]))).with_priority(2).mount(&rig.server).await;

    let first = rig.core.requests();
    let first = tokio::spawn(async move { first.load().await });
    until("la primera lista, retenida", || gate.held() == 1).await;
    rig.core.requests().load().await;
    assert_eq!(rig.core.requests().state().open.len(), 1);
    gate.open();
    first.await.unwrap();
    assert_eq!(rig.core.requests().state().open.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-a2"], "la vieja no vuelve");
}

/// `request:open`: «Por atender» delante con el pedido resaltado, la
/// plataforma avisada para traer la ventana, y el «lo enseñé» con su nonce.
/// Un `request:answered` vuelve a contar.
///
/// Roturas que atrapa: no decirle al servidor que se enseñó (el MCP diría
/// que no hay app abierta); mandar otro nonce; no avisar a la plataforma;
/// dejar delante comunicación o Plataforma; no volver a contar con los
/// eventos de los pedidos.
#[tokio::test]
async fn an_agent_asking_to_open_brings_it_and_says_it_was_shown() {
    let rig = rig().await;
    Mock::given(method("POST")).and(path("/api/v1/requests/req-a2/opened")).respond_with(ok(json!(null))).mount(&rig.server).await;
    rig.core.comms().enter();

    // En mayúsculas, como lo puede escribir quien lo copia: se resalta igual.
    rig.core.stream().deliver(ServerEvent { event: "request:open".into(), data: json!({ "id": "id-a2", "code": "REQ-A2", "nonce": "n-77" }).to_string() });
    // El «lo enseñé» sale antes de traer la lista: con ella aquí, ya salió.
    until("la lista, traída", || rig.core.requests().state().loaded).await;
    assert_eq!(posted(&rig.server, "/api/v1/requests/req-a2/opened").await, [json!({ "nonce": "n-77" })]);
    let state = rig.core.requests().state();
    assert!(state.on_screen);
    assert_eq!(state.highlight.as_deref(), Some("req-a2"));
    assert_eq!(*rig.recorder.shown.lock().unwrap(), ["req-a2"]);
    assert!(!rig.core.comms().state().on_screen, "comunicación ya no está delante");

    // Nadie lo ha contado aún; un pedido contestado en otro sitio lo cuenta.
    until("la cuenta del «lo enseñé»", || rig.core.requests().state().mine_count == 2).await;
    let before = team_loads(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/requests")).and(query_param("scope", "team")).and(query_param("status", "open"))
        .respond_with(ok(json!([request("req-a2", "t2", ("u1", "Ana Ruiz"), true)]))).with_priority(1).mount(&rig.server).await;
    rig.core.stream().deliver(ServerEvent { event: "request:answered".into(), data: json!({ "code": "req-a1" }).to_string() });
    until("la cuenta, de nuevo", || rig.core.requests().state().mine_count == 1).await;
    assert!(team_loads(&rig.server).await > before);
}
