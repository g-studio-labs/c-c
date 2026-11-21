//! Plataforma contra un servidor de mentira: la lista de servidores y el
//! cliente del agente. El agente de mentira vive en el mismo `MockServer` que
//! el backend (otro camino, mismo puerto): el servidor de la lista apunta a
//! él. Cada prueba dice la rotura que atrapa, comprobada rompiendo el código
//! (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{Config, Core, CountersState, Listener, PlatformState, Server, ServerHealth, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, header, method, path};
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
    platform: Mutex<Vec<PlatformState>>,
    counters: Mutex<Vec<CountersState>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn platform_changed(&self, state: PlatformState) {
        self.platform.lock().unwrap().push(state);
    }
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
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    Rig { server, core, recorder }
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn org(id: &str) -> Value {
    json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" })
}

/// Un servidor cuyo agente es este mismo `MockServer`.
fn row(rig: &Rig, id: &str, org_id: &str, status: &str, identity: bool) -> Value {
    json!({
        "id": id, "orgId": org_id, "name": format!("vps-{id}"), "host": "127.0.0.1", "sshPort": 22, "sshUser": "deploy",
        "type": "docker-swarm", "agentPort": rig.server.address().port(), "status": status,
        "hasAgentToken": identity, "agentVersion": if identity { 2 } else { 0 },
    })
}

async fn mount_servers(rig: &Rig, rows: Vec<Value>) {
    Mock::given(method("GET")).and(path("/api/v1/servers/")).respond_with(ok(Value::Array(rows))).mount(&rig.server).await;
}

async fn mount_orgs(rig: &Rig, ids: &[&str]) {
    Mock::given(method("GET"))
        .and(path("/api/v1/organizations/"))
        .respond_with(ok(ids.iter().map(|id| org(id)).collect()))
        .mount(&rig.server)
        .await;
    rig.core.orgs().load().await;
}

/// Un pase que dura `seconds` desde ahora.
fn pass(token: &str, seconds: i64) -> ResponseTemplate {
    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339();
    ok(json!({ "token": token, "expiresAt": expires_at }))
}

fn server_of(row: Value) -> Server {
    serde_json::from_value(row).unwrap()
}

/// Cuántas peticiones llegaron a este camino.
async fn hits(rig: &Rig, route: &str) -> Vec<Request> {
    rig.server.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == route).collect()
}

// ─── La lista ────────────────────────────────────────────────────────────────

/// Rotura que atrapa: enseñar los servidores de todas las organizaciones a
/// la vez, o seguir enseñando los de la anterior al cambiar.
#[tokio::test]
async fn switching_organization_switches_the_list() {
    let rig = rig().await;
    mount_orgs(&rig, &["o1", "o2"]).await;
    let rows = vec![row(&rig, "a", "o1", "online", true), row(&rig, "b", "o2", "online", true), row(&rig, "c", "o1", "online", true)];
    mount_servers(&rig, rows).await;

    rig.core.platform().refresh().await;
    let ids = |state: PlatformState| state.servers.into_iter().map(|s| s.id).collect::<Vec<_>>();
    assert_eq!(ids(rig.core.platform().state()), ["a", "c"]);
    assert!(rig.core.platform().open_server("a"));

    assert!(rig.core.switch_org("o2").await);
    let state = rig.core.platform().state();
    assert_eq!(state.org_id.as_deref(), Some("o2"));
    assert_eq!(state.open_server, None, "el servidor abierto era de la otra organización");
    assert_eq!(ids(state), ["b"]);
    // Y lo que oye la pantalla también, no sólo lo que se le pregunta.
    let heard = rig.recorder.platform.lock().unwrap().last().cloned().unwrap();
    assert_eq!(ids(heard), ["b"]);
}

/// Rotura que atrapa: abrir un servidor de otra organización (o uno que no
/// existe) porque llegó su id.
#[tokio::test]
async fn a_server_of_another_organization_does_not_open() {
    let rig = rig().await;
    mount_orgs(&rig, &["o1", "o2"]).await;
    mount_servers(&rig, vec![row(&rig, "a", "o1", "online", true), row(&rig, "b", "o2", "online", true)]).await;
    rig.core.platform().refresh().await;
    assert!(!rig.core.platform().open_server("b"));
    assert!(!rig.core.platform().open_server("zz"));
    assert_eq!(rig.core.platform().state().open_server, None);
}

/// Rotura que atrapa: que un fallo al pedir la lista la vacíe («no tienes
/// servidores») en vez de dejar la que había y decir el fallo.
#[tokio::test]
async fn a_failed_refresh_keeps_the_list() {
    let rig = rig().await;
    mount_orgs(&rig, &["o1"]).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/servers/"))
        .respond_with(ok(json!([row(&rig, "a", "o1", "online", true)])))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    rig.core.platform().refresh().await;
    rig.core.platform().refresh().await; // ya sin respuesta: 404
    let state = rig.core.platform().state();
    assert_eq!(state.servers.len(), 1);
    assert!(state.loaded && !state.loading);
    assert!(state.error.is_some());
}

/// Rotura que atrapa: dos módulos en pantalla a la vez (el panel no sabría
/// cuál pintar).
#[tokio::test]
async fn only_one_module_is_on_screen() {
    let rig = rig().await;
    rig.core.comms().enter();
    rig.core.platform().enter();
    assert!(rig.core.platform().state().on_screen);
    assert!(!rig.core.comms().state().on_screen);
    rig.core.comms().enter();
    assert!(!rig.core.platform().state().on_screen);
}

// ─── Los agentes de los de antes ─────────────────────────────────────────────

/// Rotura que atrapa: dejar «pendiente» para siempre un agente que contesta,
/// escribir en el backend lo que no cambió, o sondear a uno con identidad
/// (pisaría su latido).
#[tokio::test]
async fn an_old_agent_is_probed_and_what_changed_is_written() {
    let rig = rig().await;
    mount_orgs(&rig, &["o1"]).await;
    let rows = vec![row(&rig, "old", "o1", "pending", false), row(&rig, "same", "o1", "online", false), row(&rig, "new", "o1", "offline", true)];
    mount_servers(&rig, rows).await;
    Mock::given(method("GET")).and(path("/api/v1/nodes")).respond_with(ok(json!([]))).mount(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/servers/old/agent-status"))
        .and(body_json(json!({ "status": "online" })))
        .respond_with(ok(json!(null)))
        .expect(1)
        .mount(&rig.server)
        .await;
    // Los contadores, de la organización que se mira: es como arranca.
    rig.core.counters().load(Some("o1".into())).await;

    rig.core.platform().refresh().await;

    let state = rig.core.platform().state();
    let status = |id: &str| state.servers.iter().find(|s| s.id == id).unwrap().status;
    assert_eq!(status("old"), ServerHealth::Online);
    assert_eq!(status("new"), ServerHealth::Offline, "el latido de uno con identidad no se toca");
    assert!(hits(&rig, "/api/v1/servers/same/agent-status").await.is_empty());
    assert!(hits(&rig, "/api/v1/servers/new/agent-session").await.is_empty(), "a uno con identidad no se le sondea");
    assert_eq!(hits(&rig, "/api/v1/nodes").await.len(), 2);
    // Y el riel cuenta con lo que se vio: sólo «new» no está en línea.
    let counted = rig.recorder.counters.lock().unwrap().last().cloned().unwrap();
    assert_eq!(counted.servers, Some(1));
}

// ─── El pase ─────────────────────────────────────────────────────────────────

/// Rotura que atrapa: pedir un pase en cada llamada (el backend firma uno
/// por petición), o guardarlo hasta el final y mandarlo caducado.
#[tokio::test]
async fn a_pass_is_reused_until_a_minute_before_it_ends() {
    let rig = rig().await;
    let long = server_of(row(&rig, "long", "o1", "online", true));
    let short = server_of(row(&rig, "short", "o1", "online", true));
    Mock::given(method("POST")).and(path("/api/v1/servers/long/agent-session")).respond_with(pass("p-long", 600)).mount(&rig.server).await;
    Mock::given(method("POST")).and(path("/api/v1/servers/short/agent-session")).respond_with(pass("p-short", 50)).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/nodes")).respond_with(ok(json!([]))).mount(&rig.server).await;

    for _ in 0..3 {
        assert_eq!(rig.core.agent().get(&long, "/api/v1/nodes").await.unwrap().status, 200);
        assert_eq!(rig.core.agent().get(&short, "/api/v1/nodes").await.unwrap().status, 200);
    }
    assert_eq!(hits(&rig, "/api/v1/servers/long/agent-session").await.len(), 1);
    assert_eq!(hits(&rig, "/api/v1/servers/short/agent-session").await.len(), 3);
    let carried: Vec<String> = hits(&rig, "/api/v1/nodes")
        .await
        .iter()
        .map(|r| r.headers.get("authorization").unwrap().to_str().unwrap().to_string())
        .collect();
    assert_eq!(carried.iter().filter(|h| *h == "Bearer p-long").count(), 3);
}

/// Rotura que atrapa: no reintentar con un pase nuevo tras un 401 (un
/// agente con la identidad reacuñada quedaría mudo hasta reiniciar), o
/// reintentar sin fin.
#[tokio::test]
async fn a_rejected_pass_is_renewed_and_retried_once() {
    let rig = rig().await;
    let vps = server_of(row(&rig, "s", "o1", "online", true));
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p1", 600)).up_to_n_times(1).mount(&rig.server).await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p2", 600)).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/services")).and(header("authorization", "Bearer p2")).respond_with(ok(json!(["web"]))).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/services")).respond_with(ResponseTemplate::new(401)).mount(&rig.server).await;

    let services: Vec<String> = rig.core.agent().get_data(&vps, "/api/v1/services").await.unwrap();
    assert_eq!(services, ["web"]);
    assert_eq!(hits(&rig, "/api/v1/servers/s/agent-session").await.len(), 2);

    // Si el nuevo tampoco vale, el 401 es la respuesta: un intento más, no
    // un bucle.
    let deaf = server_of(row(&rig, "d", "o1", "online", true));
    Mock::given(method("POST")).and(path("/api/v1/servers/d/agent-session")).respond_with(pass("pd", 600)).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/stats")).respond_with(ResponseTemplate::new(401)).mount(&rig.server).await;
    assert_eq!(rig.core.agent().get(&deaf, "/api/v1/stats").await.unwrap().status, 401);
    assert_eq!(hits(&rig, "/api/v1/stats").await.len(), 2);
}

/// Rotura que atrapa: pedir un pase para un agente de los de antes (el
/// backend no tiene con qué firmarlo y la llamada fallaría), o mandarle una
/// cabecera que no espera.
#[tokio::test]
async fn an_agent_without_identity_gets_no_pass() {
    let rig = rig().await;
    let old = server_of(row(&rig, "old", "o1", "online", false));
    Mock::given(method("GET")).and(path("/api/v1/nodes")).respond_with(ResponseTemplate::new(401)).mount(&rig.server).await;
    assert_eq!(rig.core.agent().get(&old, "/api/v1/nodes").await.unwrap().status, 401);
    assert!(hits(&rig, "/api/v1/servers/old/agent-session").await.is_empty());
    let calls = hits(&rig, "/api/v1/nodes").await;
    assert_eq!(calls.len(), 1, "sin pase no hay nada que renovar");
    assert!(calls[0].headers.get("authorization").is_none());
}

/// Rotura que atrapa: que el pase de una persona le sirva a la siguiente
/// que entra en la misma app.
#[tokio::test]
async fn signing_out_forgets_the_passes() {
    let rig = rig().await;
    let vps = server_of(row(&rig, "s", "o1", "online", true));
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p", 600)).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/nodes")).respond_with(ok(json!([]))).mount(&rig.server).await;
    Mock::given(method("POST")).and(path("/api/v1/auth/logout")).respond_with(ok(json!(null))).mount(&rig.server).await;

    rig.core.agent().get(&vps, "/api/v1/nodes").await.unwrap();
    rig.core.logout().await;
    let luis = serde_json::from_value(json!({ "id": "u2", "username": "luis" })).unwrap();
    rig.core.sign_in(luis, "access2".into(), "refresh2".into()).unwrap();
    rig.core.agent().get(&vps, "/api/v1/nodes").await.unwrap();
    assert_eq!(hits(&rig, "/api/v1/servers/s/agent-session").await.len(), 2);
}

/// Rotura que atrapa: un stream que pierde los eventos que llegan juntos en
/// un mismo trozo, o que no lleva el pase.
#[tokio::test]
async fn a_stream_brings_each_event_with_the_pass() {
    let rig = rig().await;
    let vps = server_of(row(&rig, "s", "o1", "online", true));
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p", 600)).mount(&rig.server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/services/web/logs"))
        .and(header("authorization", "Bearer p"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: uno\n\ndata: dos\n\n", "text/event-stream"))
        .mount(&rig.server)
        .await;
    let mut stream = rig.core.agent().stream(&vps, "/api/v1/services/web/logs").await.unwrap();
    assert_eq!(stream.next().await.map(|e| e.data), Some("uno".into()));
    assert_eq!(stream.next().await.map(|e| e.data), Some("dos".into()));
    assert_eq!(stream.next().await, None);
}

// ─── Altas, edición y borrado ────────────────────────────────────────────────

use cac_core::{ServerEditError, ServerForm, ServerKind};

/// Una organización `o1` donde la persona tiene ese papel, y su lista.
async fn as_role(rig: &Rig, role: &str, rows: Vec<Value>) {
    let mut org = org("o1");
    org["role"] = json!(role);
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org]))).mount(&rig.server).await;
    rig.core.orgs().load().await;
    mount_servers(rig, rows).await;
    rig.core.platform().refresh().await;
}

fn new_form() -> ServerForm {
    ServerForm { name: " prod-02 ".into(), host: "10.0.0.2".into(), ssh_user: "root".into(), ..ServerForm::default() }
}

async fn writes(rig: &Rig) -> Vec<Request> {
    rig.server.received_requests().await.unwrap().into_iter().filter(|r| r.method.as_str() != "GET" && !r.url.path().ends_with("/agent-session")).collect()
}

/// Rotura que atrapa: no mandar la organización que se mira, mandar el
/// formulario sin limpiar, o no poner el nuevo en la lista ni en el número
/// del riel.
#[tokio::test]
async fn adding_a_server_sends_the_org_and_joins_the_list() {
    let rig = rig().await;
    as_role(&rig, "member", vec![row(&rig, "a", "o1", "online", true)]).await;
    let created = json!({ "id": "n", "orgId": "o1", "name": "prod-02", "host": "10.0.0.2", "sshPort": 22, "sshUser": "root",
        "type": "docker-swarm", "agentPort": 9090, "status": "pending", "agentVersion": 0 });
    Mock::given(method("POST"))
        .and(path("/api/v1/servers/"))
        .and(body_json(json!({ "orgId": "o1", "name": "prod-02", "host": "10.0.0.2", "sshUser": "root", "sshPort": 22, "agentPort": 9090, "type": "docker-swarm" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": created })))
        .expect(1)
        .mount(&rig.server)
        .await;

    rig.core.counters().load(Some("o1".into())).await;
    rig.core.platform().create_server(&new_form()).await.unwrap();

    let ids: Vec<_> = rig.core.platform().state().servers.into_iter().map(|s| s.id).collect();
    assert_eq!(ids, ["a", "n"]);
    assert_eq!(rig.core.counters().state().servers, Some(1), "el nuevo está pendiente: cuenta en el riel");
}

/// Rotura que atrapa: dejar que un lector dé de alta o edite, o que alguien
/// que no es superadmin cree un kubernetes. No llega ni a mandarse.
#[tokio::test]
async fn who_cannot_add_or_edit_sends_nothing() {
    let rig = rig().await;
    as_role(&rig, "viewer", vec![row(&rig, "a", "o1", "online", true)]).await;
    assert!(!rig.core.platform().state().permissions.create);
    assert_eq!(rig.core.platform().create_server(&new_form()).await, Err(ServerEditError::ReadOnly));
    let server = rig.core.platform().state().servers[0].clone();
    let renamed = ServerForm { name: "otro".into(), ..ServerForm::from(&server) };
    assert_eq!(rig.core.platform().update_server("a", &renamed).await, Err(ServerEditError::ReadOnly));
    assert!(writes(&rig).await.is_empty());

    let rig = rig_admin_without_superadmin().await;
    let kubernetes = ServerForm { kind: ServerKind::Kubernetes, ..new_form() };
    assert_eq!(rig.core.platform().create_server(&kubernetes).await, Err(ServerEditError::KubernetesSuperadminOnly));
    assert!(writes(&rig).await.is_empty());
}

async fn rig_admin_without_superadmin() -> Rig {
    let rig = rig().await;
    as_role(&rig, "admin", vec![row(&rig, "a", "o1", "online", true)]).await;
    rig
}

/// Rotura que atrapa: dejar que un miembro mueva a dónde se conecta (su SSH
/// y los pases de un admin irían a otra máquina) o lo vuelva kubernetes, o
/// impedirle el nombre.
#[tokio::test]
async fn a_member_renames_but_does_not_move_the_connection() {
    let rig = rig().await;
    let current = row(&rig, "a", "o1", "online", true);
    as_role(&rig, "member", vec![current.clone()]).await;
    let server = server_of(current.clone());

    let moved = ServerForm { host: "10.9.9.9".into(), ..ServerForm::from(&server) };
    assert_eq!(rig.core.platform().update_server("a", &moved).await, Err(ServerEditError::ConnectionAdminOnly));
    let kubernetes = ServerForm { kind: ServerKind::Kubernetes, ..ServerForm::from(&server) };
    assert_eq!(rig.core.platform().update_server("a", &kubernetes).await, Err(ServerEditError::KubernetesSuperadminOnly));
    assert!(writes(&rig).await.is_empty());

    let mut renamed_row = current;
    renamed_row["name"] = json!("vps-nuevo");
    Mock::given(method("PATCH")).and(path("/api/v1/servers/a")).respond_with(ok(renamed_row)).expect(1).mount(&rig.server).await;
    let renamed = ServerForm { name: "vps-nuevo".into(), ..ServerForm::from(&server) };
    rig.core.platform().update_server("a", &renamed).await.unwrap();
    assert_eq!(rig.core.platform().state().servers[0].name, "vps-nuevo");
}

/// Rotura que atrapa: mandar una edición que no cambia nada.
#[tokio::test]
async fn an_unchanged_edit_is_not_sent() {
    let rig = rig_admin_without_superadmin().await;
    let server = rig.core.platform().state().servers[0].clone();
    rig.core.platform().update_server("a", &ServerForm::from(&server)).await.unwrap();
    assert!(writes(&rig).await.is_empty());
}

/// Rotura que atrapa: dejar el borrado en la lista, o abierto; o dejar que
/// un miembro borre.
#[tokio::test]
async fn deleting_takes_it_out_and_closes_it() {
    let rig = rig().await;
    as_role(&rig, "member", vec![row(&rig, "a", "o1", "online", true)]).await;
    assert_eq!(rig.core.platform().delete_server("a").await, Err(ServerEditError::AdminOnly));
    assert!(writes(&rig).await.is_empty());

    let rig = rig_admin_without_superadmin().await;
    rig.core.platform().open_server("a");
    Mock::given(method("DELETE")).and(path("/api/v1/servers/a")).respond_with(ok(json!(null))).expect(1).mount(&rig.server).await;
    rig.core.platform().delete_server("a").await.unwrap();
    let state = rig.core.platform().state();
    assert!(state.servers.is_empty());
    assert_eq!(state.open_server, None);
}

// ─── Un servidor abierto ─────────────────────────────────────────────────────

use cac_core::ServerTab;

async fn mount_overview(rig: &Rig, id: &str) {
    let route = |what: &str| format!("/api/v1/servers/{id}/{what}");
    Mock::given(method("GET"))
        .and(path(route("incidents")))
        .respond_with(ok(json!({ "incidents": [
            { "id": "i1", "title": "cert-manager no renueva", "status": "open", "servers": [{ "id": id, "name": "vps" }], "areas": ["cert-manager"], "recordedAt": "2026-11-01T10:00:00Z" }
        ] })))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(route("deployables")))
        .respond_with(ok(json!([{ "id": "d1", "name": "api", "environment": "prod", "currentImage": "ghcr.io/o/api:abc1234", "onCINotify": "deploy" }])))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path(route("provisioning-runs")))
        .respond_with(ok(json!([
            { "id": "r2", "playbook": "docker.yml", "status": "succeeded", "startedAt": "2026-11-02T10:00:00Z", "summary": "ok=3" },
            { "id": "r1", "playbook": "base.yml", "status": "failed", "startedAt": "2026-11-01T10:00:00Z" }
        ])))
        .mount(&rig.server)
        .await;
}

/// Rotura que atrapa: no pedir una de las tres partes, quedarse con la
/// ejecución más vieja, o abrir en otra pestaña que el resumen.
#[tokio::test]
async fn opening_a_server_brings_its_overview() {
    let rig = rig().await;
    as_role(&rig, "member", vec![row(&rig, "a", "o1", "online", true)]).await;
    mount_overview(&rig, "a").await;
    rig.core.platform().select_tab(ServerTab::Services);

    rig.core.platform().open_server_and_load("a").await;
    let state = rig.core.platform().state();
    assert_eq!(state.tab, ServerTab::Overview);
    let overview = state.overview.expect("sin resumen");
    assert_eq!(overview.incidents.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["cert-manager no renueva"]);
    assert!(overview.incidents[0].is_open());
    assert_eq!(overview.deployables[0].image_tag().as_deref(), Some("abc1234"));
    assert_eq!(overview.deployables[0].on_ci_notify, "deploy");
    assert_eq!(overview.last_run.map(|run| run.playbook), Some("docker.yml".into()));
}

/// Rotura que atrapa: que un endpoint caído tumbe el resumen entero.
#[tokio::test]
async fn a_failing_part_of_the_overview_stays_empty() {
    let rig = rig().await;
    as_role(&rig, "member", vec![row(&rig, "a", "o1", "online", true)]).await;
    Mock::given(method("GET")).and(path("/api/v1/servers/a/incidents")).respond_with(ResponseTemplate::new(500)).mount(&rig.server).await;
    mount_overview(&rig, "a").await;

    rig.core.platform().open_server_and_load("a").await;
    let overview = rig.core.platform().state().overview.expect("sin resumen");
    assert!(overview.incidents.is_empty());
    assert_eq!(overview.deployables.len(), 1);
}

/// Rotura que atrapa: pintar en un servidor el resumen que llegó tarde de
/// otro.
#[tokio::test]
async fn a_late_overview_does_not_land_on_another_server() {
    let rig = rig().await;
    as_role(&rig, "member", vec![row(&rig, "a", "o1", "online", true), row(&rig, "b", "o1", "online", true)]).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/servers/a/incidents"))
        .respond_with(ok(json!({ "incidents": [] })).set_delay(std::time::Duration::from_millis(300)))
        .mount(&rig.server)
        .await;
    rig.core.platform().open_server("a");
    let platform = rig.core.platform();
    let late = tokio::spawn(async move { platform.load_overview().await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    rig.core.platform().open_server("b");
    late.await.unwrap();
    let state = rig.core.platform().state();
    assert_eq!(state.open_server.as_deref(), Some("b"));
    assert_eq!(state.overview, None);
}

/// Rotura que atrapa: dejar elegir una pestaña de swarm en un kubernetes,
/// o volver a un servidor en la pestaña que tenía otro.
#[tokio::test]
async fn tabs_follow_the_kind_of_server() {
    let rig = rig().await;
    let mut kubernetes = row(&rig, "k", "o1", "online", false);
    kubernetes["type"] = json!("kubernetes");
    as_role(&rig, "member", vec![row(&rig, "a", "o1", "online", true), kubernetes]).await;
    let platform = rig.core.platform();

    platform.open_server("k");
    assert!(!platform.select_tab(ServerTab::Services));
    assert!(platform.select_tab(ServerTab::Provision));
    assert_eq!(platform.state().tab, ServerTab::Provision);

    platform.open_server("a");
    assert_eq!(platform.state().tab, ServerTab::Overview, "abrir otro hereda la pestaña del anterior");
    assert!(platform.select_tab(ServerTab::Services));
    platform.close_server();
    assert_eq!(platform.state().tab, ServerTab::Overview, "cerrar deja la pestaña del que se cerró");
}
