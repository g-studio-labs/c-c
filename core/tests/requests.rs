//! «Por atender» contra un servidor de mentira. Cada prueba dice la rotura
//! que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{elsewhere_label, AnswerDraft, Config, Core, Listener, Locale, RequestScope, ServerEvent, Session, Storage};
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
    mount_orgs(&server).await;
    let recorder = Arc::new(Recorder::default());
    let core = Core::new(Config::new(front(&server.uri())), Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.orgs().load().await;
    Rig { server, core, recorder }
}

/// Dos organizaciones; se mira la primera (`o1`, «DWIT»).
async fn mount_orgs(server: &MockServer) {
    let org = |id: &str, name: &str| {
        json!({ "id": id, "name": name, "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z",
            "clientsSeeOnlyTheirSpace": false, "guestsCanUseDevTools": false, "doneNeedsSubtasksDone": false, "timezone": "" })
    };
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org("o1", "DWIT"), org("o2", "G Studio")]))).mount(server).await;
}

fn in_org(org: &str, mut request: Value) -> Value {
    request["orgId"] = json!(org);
    request
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
/// hasta el evento; no bajar el número o la marca de su tarea; que desplegar
/// «Resueltos» después lo pierda.
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

    // Desplegar «Resueltos» después no se lo lleva, aunque la primera página
    // (aquí, vacía) no lo traiga.
    rig.core.requests().set_resolved_open(true).await;
    assert_eq!(rig.core.requests().state().closed.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-a1"]);
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

/// «Por atender» es de la organización que se mira: ni la lista, ni «Resueltos»
/// ni el número llevan pedidos de otra, y de los tuyos en las otras se avisa
/// por organización. Al cambiar de organización los abiertos se reparten sin
/// volver a pedirlos (command-and-control-527); los resueltos, que van por
/// páginas, se piden de la nueva (command-and-control-569).
///
/// Roturas que atrapa: pintar o contar los de otra organización (en la lista,
/// en «Resueltos» o en el número); no avisar de ellos; contar en el aviso los
/// de otra persona o los de la organización que se mira; no repartir de nuevo
/// al cambiar de organización; decir el aviso sin su organización o sin plural.
#[tokio::test]
async fn only_the_organization_being_looked_at_is_listed_and_the_others_are_announced() {
    let rig = rig().await;
    let ana = ("u1", "Ana Ruiz");
    let g = |code: &str, mine: bool| in_org("o2", request(code, "t9", if mine { ana } else { ("u2", "Luis") }, mine));
    let team = json!([request("req-a1", "t1", ana, true), g("req-g1", true), g("req-g2", true), g("req-g3", false)]);
    list("team", "open").respond_with(ok(team)).with_priority(1).mount(&rig.server).await;
    list("mine", "open").respond_with(ok(json!([request("req-a1", "t1", ana, true), g("req-g1", true), g("req-g2", true)]))).with_priority(1).mount(&rig.server).await;

    // Los de abajo son de una organización: se piden con la suya.
    list("mine", "closed").and(query_param("orgId", "o1")).respond_with(ok(json!({ "requests": [request("req-a9", "t1", ana, false)] }))).with_priority(1).mount(&rig.server).await;
    list("mine", "closed").and(query_param("orgId", "o2")).respond_with(ok(json!({ "requests": [in_org("o2", request("req-g9", "t9", ana, false))] }))).with_priority(1).mount(&rig.server).await;

    rig.core.requests().refresh_marks().await;
    rig.core.requests().load().await;
    rig.core.requests().set_resolved_open(true).await;
    let state = rig.core.requests().state();
    let codes = |list: &[cac_core::AgentRequest]| list.iter().map(|r| r.code.clone()).collect::<Vec<_>>();
    assert_eq!(codes(&state.open), ["req-a1"]);
    assert_eq!(codes(&state.closed), ["req-a9"]);
    assert_eq!(state.mine_count, 1, "el número no cuenta los de G Studio");
    assert_eq!(state.elsewhere.len(), 1);
    assert_eq!((state.elsewhere[0].org_id.as_str(), state.elsewhere[0].org_name.as_str(), state.elsewhere[0].count), ("o2", "G Studio", 2), "los de Luis no son tuyos");
    assert_eq!(elsewhere_label(&state.elsewhere[0], Locale::En), "2 more requests in G Studio");
    assert_eq!(elsewhere_label(&cac_core::RequestsElsewhere { count: 1, ..state.elsewhere[0].clone() }, Locale::Es), "1 pedido más en G Studio");

    let loads = team_loads(&rig.server).await;
    assert!(rig.core.switch_org("o2").await);
    until("los de abajo de G Studio", || codes(&rig.core.requests().state().closed) == ["req-g9"]).await;
    let state = rig.core.requests().state();
    assert_eq!(codes(&state.open), ["req-g1", "req-g2"]);
    assert_eq!(state.mine_count, 2);
    assert_eq!((state.elsewhere.len(), state.elsewhere[0].org_name.as_str(), state.elsewhere[0].count), (1, "DWIT", 1));
    assert_eq!(team_loads(&rig.server).await, loads, "cambiar de organización reparte, no vuelve a pedir");
}

/// Un agente pide abrir un pedido de **otra** organización: se pasa a ella
/// antes de enseñarlo, y sólo entonces se dice «lo enseñé» (command-and-control-527).
///
/// Roturas que atrapa: enseñar y confirmar sin cambiar de organización (la
/// tarjeta no estaría en la lista y el MCP diría `delivered` sin que nadie
/// lo viera); confirmar un pedido que no se encuentra en ninguna lista.
#[tokio::test]
async fn opening_a_request_of_another_organization_switches_to_it_first() {
    let rig = rig().await;
    let ana = ("u1", "Ana Ruiz");
    let g1 = in_org("o2", request("req-g1", "t9", ana, true));
    list("team", "open").respond_with(ok(json!([request("req-a1", "t1", ana, true), g1.clone()]))).with_priority(1).mount(&rig.server).await;
    list("mine", "open").respond_with(ok(json!([request("req-a1", "t1", ana, true), g1]))).with_priority(1).mount(&rig.server).await;
    for code in ["req-g1", "req-zz"] {
        Mock::given(method("POST")).and(path(format!("/api/v1/requests/{code}/opened"))).respond_with(ok(json!(null))).mount(&rig.server).await;
    }
    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o1"));

    rig.core.stream().deliver(ServerEvent { event: "request:open".into(), data: json!({ "code": "req-g1", "nonce": "n-1" }).to_string() });
    until("el pedido, delante", || rig.core.requests().state().highlight.as_deref() == Some("req-g1")).await;
    until("la lista de la otra organización", || rig.core.requests().state().open.iter().any(|r| r.code == "req-g1")).await;
    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o2"), "se cambió a la del pedido");
    assert_eq!(posted(&rig.server, "/api/v1/requests/req-g1/opened").await, [json!({ "nonce": "n-1" })]);

    // Uno que ningún listado conoce no se puede enseñar, y no se confirma.
    rig.core.stream().deliver(ServerEvent { event: "request:open".into(), data: json!({ "code": "req-zz", "nonce": "n-2" }).to_string() });
    until("la búsqueda del desconocido", || rig.recorder.shown.lock().unwrap().len() >= 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(posted(&rig.server, "/api/v1/requests/req-zz/opened").await.is_empty());
    assert_eq!(*rig.recorder.shown.lock().unwrap(), ["req-g1"]);
}

/// Un pedido de una organización a la que no se puede pasar (no es tuya) no se
/// enseña ni se confirma: el MCP no debe oír «lo enseñé» de lo que nadie ve.
///
/// Rotura que atrapa: ignorar que el cambio de organización dijo que no y
/// enseñar y confirmar igual.
#[tokio::test]
async fn a_request_of_an_organization_that_cannot_be_entered_is_neither_shown_nor_confirmed() {
    let rig = rig().await;
    let ana = ("u1", "Ana Ruiz");
    let stranger = in_org("o-unknown", request("req-x1", "t9", ana, true));
    list("team", "open").respond_with(ok(json!([request("req-a1", "t1", ana, true), stranger]))).with_priority(1).mount(&rig.server).await;
    Mock::given(method("POST")).and(path("/api/v1/requests/req-x1/opened")).respond_with(ok(json!(null))).mount(&rig.server).await;

    rig.core.stream().deliver(ServerEvent { event: "request:open".into(), data: json!({ "code": "REQ-X1", "nonce": "n-9" }).to_string() });
    until("la búsqueda del pedido", || rig.core.requests().state().mine_count == 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(posted(&rig.server, "/api/v1/requests/req-x1/opened").await.is_empty());
    assert!(rig.recorder.shown.lock().unwrap().is_empty());
    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o1"));
}

/// Una organización que no está en la lista (o que aún no llegó) no avisa:
/// el aviso nunca dice un id.
///
/// Rotura que atrapa: avisar «2 pedidos más en <uuid>».
#[tokio::test]
async fn an_organization_without_a_known_name_is_not_announced() {
    let rig = rig().await;
    let ana = ("u1", "Ana Ruiz");
    let stranger = in_org("o-unknown", request("req-x1", "t9", ana, true));
    list("team", "open").respond_with(ok(json!([request("req-a1", "t1", ana, true), stranger]))).with_priority(1).mount(&rig.server).await;
    rig.core.requests().refresh_marks().await;
    let state = rig.core.requests().state();
    assert_eq!(state.mine_count, 1);
    assert!(state.elsewhere.is_empty(), "sin nombre no se avisa: {:?}", state.elsewhere);
}

fn closed(code: &str) -> Value {
    let mut done = request(code, "t1", ("u1", "Ana Ruiz"), false);
    done["status"] = json!("answered");
    done["answer"] = json!({ "approved": true });
    done
}

/// Lo que se pidió a `/api/v1/requests` con ese estado, por su query.
async fn asked(server: &MockServer, status: &str) -> Vec<HashMap<String, String>> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.url.path() == "/api/v1/requests")
        .map(|request| request.url.query_pairs().into_owned().collect::<HashMap<String, String>>())
        .filter(|query| query.get("status").map(String::as_str) == Some(status))
        .collect()
}

async fn mine_open(server: &MockServer) -> usize {
    asked(server, "open").await.into_iter().filter(|q| q.get("scope").map(String::as_str) == Some("mine")).count()
}

fn page(requests: Value, next: &str) -> ResponseTemplate {
    ok(json!({ "requests": requests, "next": next }))
}

/// «Resueltos» plegado no pide nada; desplegarlo pide la primera página de
/// veinte, de la organización que se mira, y la siguiente se pide con su
/// cursor. Cuando no hay más, no se pide más.
///
/// Roturas que atrapa: pedir los resueltos al abrir; pedirlos sin cursor (la
/// lista entera, como antes); pedir la siguiente sin el cursor que dio la
/// anterior; seguir pidiendo cuando el servidor dijo que no hay más; perder
/// la primera página al añadir la segunda; pedirlos de todas las
/// organizaciones.
#[tokio::test]
async fn resolved_are_asked_only_when_unfolded_and_page_by_page() {
    let rig = rig().await;
    list("mine", "closed").and(query_param("cursor", "")).respond_with(page(json!([closed("req-c1"), closed("req-c2")]), "CUR-1")).with_priority(1).mount(&rig.server).await;
    list("mine", "closed").and(query_param("cursor", "CUR-1")).respond_with(page(json!([closed("req-c3")]), "")).with_priority(1).mount(&rig.server).await;

    rig.core.requests().load().await;
    assert!(rig.core.requests().state().loaded);
    assert!(asked(&rig.server, "closed").await.is_empty(), "plegado no se pide");
    let open = asked(&rig.server, "open").await;
    assert_eq!((open[0].get("limit").map(String::as_str), open[0].get("cursor").map(String::as_str)), (Some("50"), Some("")), "los abiertos, de cincuenta en cincuenta");

    rig.core.requests().set_resolved_open(true).await;
    let first = asked(&rig.server, "closed").await;
    assert_eq!(first.len(), 1);
    assert_eq!((first[0]["limit"].as_str(), first[0]["cursor"].as_str(), first[0]["orgId"].as_str()), ("20", "", "o1"));
    let state = rig.core.requests().state();
    assert_eq!(state.closed.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-c1", "req-c2"]);
    assert!(state.closed_paging.more && state.closed_paging.loaded && !state.closed_paging.loading);

    rig.core.requests().load_more().await;
    let pages = asked(&rig.server, "closed").await;
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1]["cursor"], "CUR-1");
    let state = rig.core.requests().state();
    assert_eq!(state.closed.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-c1", "req-c2", "req-c3"]);
    assert!(!state.closed_paging.more);

    rig.core.requests().load_more().await;
    assert_eq!(asked(&rig.server, "closed").await.len(), 2, "sin más, no se pide más");

    // Plegar y desplegar no vuelve a pedir lo que ya está.
    rig.core.requests().set_resolved_open(false).await;
    rig.core.requests().set_resolved_open(true).await;
    assert_eq!(asked(&rig.server, "closed").await.len(), 2);
}

/// El scroll pide la siguiente muchas veces seguidas: sale una sola petición
/// mientras la primera no vuelve, y si falla se dice y se reintenta a mano.
///
/// Roturas que atrapa: pedir dos veces la misma página; no decir que está
/// cargando; dar la lista por terminada cuando la página falló (no se podría
/// reintentar).
#[tokio::test]
async fn the_next_page_is_asked_once_at_a_time_and_a_failure_can_be_retried() {
    let rig = rig().await;
    let gate = Gate::default();
    list("mine", "closed").and(query_param("cursor", "")).respond_with(page(json!([closed("req-c1")]), "CUR-1")).with_priority(1).mount(&rig.server).await;
    list("mine", "closed").and(query_param("cursor", "CUR-1")).respond_with(gate.hold(ResponseTemplate::new(500))).up_to_n_times(1).with_priority(1).mount(&rig.server).await;
    list("mine", "closed").and(query_param("cursor", "CUR-1")).respond_with(page(json!([closed("req-c2")]), "")).with_priority(2).mount(&rig.server).await;
    rig.core.requests().load().await;
    rig.core.requests().set_resolved_open(true).await;

    let (a, b) = (rig.core.requests(), rig.core.requests());
    let first = tokio::spawn(async move { a.load_more().await });
    until("la página, retenida", || gate.held() == 1).await;
    assert!(rig.core.requests().state().closed_paging.loading, "se dice que está cargando");
    b.load_more().await;
    rig.core.requests().load_more().await;
    gate.open();
    first.await.unwrap();
    assert_eq!(asked(&rig.server, "closed").await.iter().filter(|q| q["cursor"] == "CUR-1").count(), 1, "una sola petición");
    let state = rig.core.requests().state();
    assert!(state.closed_paging.failed && state.closed_paging.more && !state.closed_paging.loading);

    rig.core.requests().load_more().await;
    let state = rig.core.requests().state();
    assert_eq!(state.closed.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-c1", "req-c2"]);
    assert!(!state.closed_paging.failed && !state.closed_paging.more);
}

/// Un enlace a un pedido resuelto que no está en lo cargado lo encuentra
/// igual: se pide ése por su código y se despliega «Resueltos» con él.
///
/// Roturas que atrapa: no pedirlo suelto (el enlace no llevaría a nada);
/// pedirlo pero no enseñarlo; no desplegar «Resueltos»; volver a pedirlo con
/// cada carga; volver a desplegar «Resueltos» con cada carga después de que
/// la persona lo plegó.
#[tokio::test]
async fn a_link_to_a_resolved_request_outside_the_page_opens_it() {
    let rig = rig().await;
    list("mine", "closed").respond_with(page(json!([closed("req-c1")]), "CUR-1")).with_priority(1).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/requests/req-old")).respond_with(ok(closed("req-old"))).mount(&rig.server).await;

    assert!(rig.core.requests().show(Some("REQ-OLD".into())));
    rig.core.requests().load().await;
    let state = rig.core.requests().state();
    assert!(state.resolved_open, "«Resueltos» se despliega");
    assert!(state.closed.iter().any(|r| r.code == "req-old"), "{:?}", state.closed.iter().map(|r| &r.code).collect::<Vec<_>>());
    assert!(state.closed.iter().any(|r| r.code == "req-c1"), "con la primera página");

    // Si la persona lo pliega, una carga después (un evento, volver de una
    // caída) no se lo vuelve a abrir.
    rig.core.requests().set_resolved_open(false).await;
    rig.core.requests().load().await;
    let singles = rig.server.received_requests().await.unwrap().iter().filter(|r| r.url.path() == "/api/v1/requests/req-old").count();
    assert_eq!(singles, 1, "se busca una vez por enlace");
    assert!(!rig.core.requests().state().resolved_open, "plegado se queda plegado");
}

/// Los filtros van al servidor en la misma consulta que el cursor: la página
/// siguiente es de lo filtrado. Filtrar por un estado cerrado pide esa lista
/// sin pliegue, y no los abiertos.
///
/// Roturas que atrapa: filtrar en local (la página dos traería lo no
/// filtrado); perder el filtro al pedir la siguiente; no reiniciar desde la
/// primera página al cambiarlo; pedir los abiertos filtrando por resueltos.
#[tokio::test]
async fn filters_travel_with_the_cursor() {
    let rig = rig().await;
    let ana = ("u1", "Ana Ruiz");
    list("mine", "open").and(query_param("q", "cac-12")).and(query_param("cursor", "")).respond_with(page(json!([request("req-a1", "t1", ana, true)]), "OPEN-2")).with_priority(1).mount(&rig.server).await;
    list("mine", "open").and(query_param("q", "cac-12")).and(query_param("cursor", "OPEN-2")).respond_with(page(json!([request("req-a3", "t1", ana, true)]), "")).with_priority(1).mount(&rig.server).await;
    list("mine", "closed").respond_with(page(json!([closed("req-c1")]), "")).with_priority(1).mount(&rig.server).await;
    rig.core.requests().load().await;

    let filter = cac_core::RequestFilter { text: " cac-12 ".into(), kinds: vec![cac_core::RequestKind::Decide, cac_core::RequestKind::Run], ..Default::default() };
    rig.core.requests().set_filter(filter.clone()).await;
    rig.core.requests().load_more().await;
    let open = asked(&rig.server, "open").await;
    let filtered: Vec<_> = open.iter().filter(|q| q.contains_key("q")).collect();
    assert_eq!(filtered.len(), 2, "{open:?}");
    for q in &filtered {
        assert_eq!((q["q"].as_str(), q["kind"].as_str()), ("cac-12", "decide,run"));
    }
    assert_eq!((filtered[0]["cursor"].as_str(), filtered[1]["cursor"].as_str()), ("", "OPEN-2"));
    assert_eq!(rig.core.requests().state().open.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-a1", "req-a3"]);

    let before = asked(&rig.server, "open").await.len();
    rig.core.requests().set_filter(cac_core::RequestFilter { status: cac_core::RequestStatusFilter::Resolved, ..Default::default() }).await;
    assert_eq!(asked(&rig.server, "open").await.len(), before, "filtrando por resueltos no se piden abiertos");
    let state = rig.core.requests().state();
    assert!(state.open.is_empty() && state.loaded);
    assert_eq!(state.closed.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-c1"], "sin desplegar nada");
}

/// Lo que llega por eventos no lo recarga todo: un pedido nuevo vuelve a pedir
/// los abiertos y no los resueltos; uno contestado en otro sitio se pide
/// suelto y entra arriba en «Resueltos», sin perder las páginas ya traídas.
///
/// Roturas que atrapa: recargar los resueltos desde la primera página con
/// cada evento (se pierde lo que el scroll trajo); no pedir los abiertos con
/// un pedido nuevo; dejar el contestado en abiertos; no ponerlo en
/// «Resueltos».
#[tokio::test]
async fn events_do_not_reload_everything() {
    let rig = rig().await;
    list("mine", "closed").and(query_param("cursor", "")).respond_with(page(json!([closed("req-c1")]), "CUR-1")).with_priority(1).mount(&rig.server).await;
    list("mine", "closed").and(query_param("cursor", "CUR-1")).respond_with(page(json!([closed("req-c2")]), "")).with_priority(1).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/requests/req-a1")).respond_with(ok(closed("req-a1"))).mount(&rig.server).await;
    rig.core.requests().load().await;
    rig.core.requests().set_resolved_open(true).await;
    rig.core.requests().load_more().await;
    // Los abiertos de la lista; los de «Equipo» son los del número, que se
    // piden con cualquier evento.
    let (opens, closeds) = (mine_open(&rig.server).await, asked(&rig.server, "closed").await.len());

    rig.core.stream().deliver(ServerEvent { event: "request:created".into(), data: json!({ "code": "req-a9", "status": "open" }).to_string() });
    for _ in 0..300 {
        if mine_open(&rig.server).await > opens {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(mine_open(&rig.server).await > opens, "un pedido nuevo vuelve a pedir los abiertos");

    rig.core.stream().deliver(ServerEvent { event: "request:answered".into(), data: json!({ "code": "req-a1", "status": "answered" }).to_string() });
    until("el contestado, arriba", || rig.core.requests().state().closed.first().map(|r| r.code.as_str()) == Some("req-a1")).await;
    let state = rig.core.requests().state();
    assert_eq!(state.closed.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-a1", "req-c1", "req-c2"]);
    assert!(!state.open.iter().any(|r| r.code == "req-a1"));
    assert_eq!(asked(&rig.server, "closed").await.len(), closeds, "los resueltos no se vuelven a pedir");
}

/// Un pedido nuevo que llega mientras la primera carga está en camino (al
/// entrar, o tras cambiar de filtro) no la deja colgada: la respuesta vieja
/// se tira y la carga se vuelve a pedir entera, con el pedido nuevo.
///
/// Roturas que atrapa: subir el turno con la carga en camino sin volver a
/// pedir (se queda «Cargando…» para siempre, el fallo de la primera vuelta);
/// pedir sólo el número en ese caso; poner la respuesta vieja, que no trae el
/// pedido nuevo.
#[tokio::test]
async fn an_event_during_the_first_load_does_not_leave_it_loading() {
    let rig = rig().await;
    let gate = Gate::default();
    let ana = ("u1", "Ana Ruiz");
    list("mine", "open").respond_with(gate.hold(page(json!([request("req-a1", "t1", ana, true)]), ""))).up_to_n_times(1).with_priority(1).mount(&rig.server).await;
    list("mine", "open").respond_with(page(json!([request("req-a1", "t1", ana, true), request("req-a9", "t1", ana, true)]), "")).with_priority(2).mount(&rig.server).await;

    let first = rig.core.requests();
    let first = tokio::spawn(async move { first.load().await });
    until("la primera carga, retenida", || gate.held() == 1).await;
    rig.core.stream().deliver(ServerEvent { event: "request:created".into(), data: json!({ "code": "req-a9", "status": "open" }).to_string() });
    until("la carga, pedida otra vez", || rig.core.requests().state().loaded).await;
    gate.open();
    first.await.unwrap();

    let state = rig.core.requests().state();
    assert!(state.loaded && !state.open_paging.loading && state.open_paging.loaded, "{:?}", state.open_paging);
    assert_eq!(state.open.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-a1", "req-a9"], "la vieja no vuelve");
    assert_eq!(mine_open(&rig.server).await, 2);
}

/// Un pedido nuevo que llega mientras se pide la página siguiente de los
/// abiertos: esa página se tira, y el «cargando más» no se queda puesto; el
/// scroll vuelve a pedirla con el cursor que dio la recarga.
///
/// Roturas que atrapa: tirar la página sin bajar su «cargando» (se queda
/// «Cargando más…» fijo y el scroll no vuelve a pedir, el fallo de la primera
/// vuelta); que la recarga no lo baje; poner la página vieja sobre la recarga.
#[tokio::test]
async fn an_event_while_the_next_open_page_loads_lets_scrolling_ask_again() {
    let rig = rig().await;
    let gate = Gate::default();
    let ana = ("u1", "Ana Ruiz");
    let first_page = || page(json!([request("req-a1", "t1", ana, true), request("req-a2", "t2", ana, true)]), "OPEN-2");
    list("mine", "open").and(query_param("cursor", "")).respond_with(first_page()).with_priority(1).mount(&rig.server).await;
    list("mine", "open").and(query_param("cursor", "OPEN-2")).respond_with(gate.hold(page(json!([request("req-old", "t1", ana, true)]), ""))).up_to_n_times(1).with_priority(1).mount(&rig.server).await;
    list("mine", "open").and(query_param("cursor", "OPEN-2")).respond_with(page(json!([request("req-a3", "t1", ana, true)]), "")).with_priority(2).mount(&rig.server).await;
    rig.core.requests().load().await;
    assert!(rig.core.requests().state().open_paging.more);

    let scroll = rig.core.requests();
    let scroll = tokio::spawn(async move { scroll.load_more().await });
    until("la página siguiente, retenida", || gate.held() == 1).await;
    rig.core.stream().deliver(ServerEvent { event: "request:created".into(), data: json!({ "code": "req-a9", "status": "open" }).to_string() });
    until("la recarga de los abiertos, puesta", || !rig.core.requests().state().open_paging.loading).await;
    gate.open();
    scroll.await.unwrap();

    let state = rig.core.requests().state();
    assert!(!state.open_paging.loading && state.open_paging.more && !state.open_paging.failed, "{:?}", state.open_paging);
    assert!(!state.open.iter().any(|r| r.code == "req-old"), "la página tirada no entra");

    rig.core.requests().load_more().await;
    let again = asked(&rig.server, "open").await.into_iter().filter(|q| q.get("cursor").map(String::as_str) == Some("OPEN-2")).count();
    assert_eq!(again, 2, "el scroll vuelve a pedirla");
    let state = rig.core.requests().state();
    assert_eq!(state.open.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(), ["req-a1", "req-a2", "req-a3"]);
    assert!(!state.open_paging.loading && !state.open_paging.more);
}

/// Un enlace enseña su pedido aunque el filtro de ahora no lo deje pasar: es
/// lo que se pidió ver. Cambiar de filtro lo suelta, también si cambia
/// mientras el pedido llegaba.
///
/// Roturas que atrapa: esconder el pedido del enlace porque el filtro no lo
/// admite; dejarlo pegado al cambiar de filtro; ponerlo cuando llega tarde,
/// con otro filtro ya puesto.
#[tokio::test]
async fn a_link_shows_its_request_under_any_filter_until_the_filter_changes() {
    let rig = rig().await;
    let gate = Gate::default();
    Mock::given(method("GET")).and(path("/api/v1/requests/req-old")).respond_with(gate.hold(ok(closed("req-old")))).up_to_n_times(1).with_priority(1).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/requests/req-old")).respond_with(ok(closed("req-old"))).with_priority(2).mount(&rig.server).await;
    let only_open = cac_core::RequestFilter { status: cac_core::RequestStatusFilter::Open, ..Default::default() };
    let other = cac_core::RequestFilter { text: "cac-1".into(), ..only_open.clone() };

    // Llega tarde, con otro filtro ya puesto: no entra.
    rig.core.requests().set_filter(only_open.clone()).await;
    assert!(rig.core.requests().show(Some("req-old".into())));
    let late = rig.core.requests();
    let late = tokio::spawn(async move { late.load().await });
    until("el pedido del enlace, retenido", || gate.held() == 1).await;
    rig.core.requests().set_filter(other).await;
    gate.open();
    late.await.unwrap();
    assert!(!rig.core.requests().state().closed.iter().any(|r| r.code == "req-old"), "no se pega a un filtro que no lo pidió");

    // Un enlace nuevo con «sólo abiertos»: se enseña igual, aunque esté resuelto.
    rig.core.requests().set_filter(only_open).await;
    rig.core.requests().leave();
    assert!(rig.core.requests().show(Some("req-old".into())));
    rig.core.requests().load().await;
    assert!(rig.core.requests().state().closed.iter().any(|r| r.code == "req-old"), "el enlace lo enseña");

    rig.core.requests().set_filter(cac_core::RequestFilter::default()).await;
    assert!(!rig.core.requests().state().closed.iter().any(|r| r.code == "req-old"), "cambiar de filtro lo suelta");
}
