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
    let mut config = Config::new(server.uri());
    config.stream_backoff = Duration::from_millis(5);
    config.stats_every = Duration::from_millis(20);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
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

/// Rotura que atrapa: avisar de un cambio con el candado ya suelto, de modo
/// que dos cambios a la vez lleguen a la pantalla en orden inverso y se
/// quede pintado el viejo (un log cerrado que sigue ahí).
#[tokio::test]
async fn the_last_change_is_the_last_one_painted() {
    let rig = rig().await;
    let mut stale = 0;
    for _ in 0..2000 {
        let (a, b) = (rig.core.clone(), rig.core.clone());
        let start = Arc::new(std::sync::Barrier::new(2));
        let (sa, sb) = (start.clone(), start.clone());
        let enter = std::thread::spawn(move || {
            sa.wait();
            a.platform().enter();
        });
        let leave = std::thread::spawn(move || {
            sb.wait();
            b.platform().leave();
        });
        enter.join().unwrap();
        leave.join().unwrap();
        let painted = rig.recorder.platform.lock().unwrap().last().map(|state| state.on_screen);
        if painted != Some(rig.core.platform().state().on_screen) {
            stale += 1;
        }
    }
    assert_eq!(stale, 0, "veces que quedó pintado un estado viejo");
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
    let port = rig.server.address().port();
    let body = json!({ "name": "vps-nuevo", "host": "127.0.0.1", "sshUser": "deploy", "sshPort": 22, "agentPort": port, "type": "docker-swarm" });
    Mock::given(method("PATCH")).and(path("/api/v1/servers/a")).and(body_json(body)).respond_with(ok(renamed_row)).expect(1).mount(&rig.server).await;
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

// ─── Instalar el agente ──────────────────────────────────────────────────────

use cac_core::{AgentInstallOutcome, AgentInstallPhase, AgentWait};
use std::time::Duration;

const QUICK: AgentWait = AgentWait { total: Duration::from_millis(300), every: Duration::from_millis(10) };

fn seen(mut row: Value, at: &str, version: i64) -> Value {
    row["agentSeenAt"] = json!(at);
    row["agentVersion"] = json!(version);
    row
}

async fn admin_with_agent(rig: &Rig) -> Value {
    let current = seen(row(rig, "a", "o1", "online", true), "2026-11-20T10:00:00Z", 2);
    as_role(rig, "admin", vec![current.clone()]).await;
    current
}

async fn mount_mint(rig: &Rig) {
    Mock::given(method("POST"))
        .and(path("/api/v1/servers/a/agent-token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": { "token": "cac_agent_NEW", "sessionKey": "cd".repeat(32) } })))
        .expect(1)
        .mount(&rig.server)
        .await;
}

/// Rotura que atrapa: llevar por SSH otra identidad que la recién acuñada,
/// dar por vuelto al agente viejo (su latido de antes), o no poner la
/// versión nueva en la lista.
#[tokio::test]
async fn installing_carries_the_new_identity_and_waits_for_a_new_heartbeat() {
    let rig = rig().await;
    let current = admin_with_agent(&rig).await;
    mount_mint(&rig).await;
    Mock::given(method("GET")).and(path("/api/v1/servers/a")).respond_with(ok(current.clone())).up_to_n_times(2).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/servers/a")).respond_with(ok(seen(current, "2026-11-20T10:05:00Z", 14))).mount(&rig.server).await;

    let carried = Arc::new(Mutex::new(Vec::new()));
    let sink = carried.clone();
    let outcome = rig
        .core
        .platform()
        .install_agent("a", AgentWait { total: Duration::from_secs(5), ..QUICK }, move |server, identity| {
            sink.lock().unwrap().push((server.id.clone(), identity.token, identity.session_key));
            Ok(())
        })
        .await;

    assert_eq!(outcome, Ok(AgentInstallOutcome::Changed { from: 2, to: 14 }));
    assert_eq!(*carried.lock().unwrap(), [("a".to_string(), "cac_agent_NEW".to_string(), "cd".repeat(32))]);
    let state = rig.core.platform().state();
    assert_eq!(state.servers[0].agent_version, 14);
    assert_eq!(state.agent_job, None);
    let phases: Vec<_> = rig.recorder.platform.lock().unwrap().iter().filter_map(|s| s.agent_job.as_ref().map(|job| job.phase)).collect();
    assert_eq!(phases.first(), Some(&AgentInstallPhase::Installing));
    assert_eq!(phases.last(), Some(&AgentInstallPhase::Waiting));
}

/// Rotura que atrapa: dejar que un miembro reacuñe la identidad (tumba el
/// agente) o lance el SSH.
#[tokio::test]
async fn a_member_cannot_install_and_nothing_is_minted() {
    let rig = rig().await;
    as_role(&rig, "member", vec![row(&rig, "a", "o1", "online", true)]).await;
    assert!(!rig.core.platform().state().permissions.install_agent);
    let outcome = rig.core.platform().install_agent("a", QUICK, |_, _| panic!("no debe llegar al SSH")).await;
    assert_eq!(outcome, Err(ServerEditError::AdminOnly));
    assert!(writes(&rig).await.is_empty());
}

/// Un servidor sin identidad todavía, que tras acuñar el backend da con
/// ella.
async fn admin_without_agent(rig: &Rig) {
    let first = row(rig, "a", "o1", "pending", false);
    as_role(rig, "admin", vec![first.clone()]).await;
    let mut minted = first;
    minted["hasAgentToken"] = json!(true);
    Mock::given(method("GET")).and(path("/api/v1/servers/a")).respond_with(ok(minted)).mount(&rig.server).await;
}

/// Rotura que atrapa: tragarse el fallo del SSH y quedarse esperando un
/// latido que no va a llegar, o dejar la instalación en curso; o que la
/// fila no se entere de que ya hay identidad acuñada (Servicios iría sin
/// pase).
#[tokio::test]
async fn a_failed_ssh_says_why_and_does_not_wait() {
    let rig = rig().await;
    admin_without_agent(&rig).await;
    mount_mint(&rig).await;
    let outcome = rig.core.platform().install_agent("a", QUICK, |_, _| Err("Permission denied (publickey)".into())).await;
    assert_eq!(outcome, Err(ServerEditError::Ssh("Permission denied (publickey)".into())));
    assert_eq!(hits(&rig, "/api/v1/servers/a").await.len(), 1, "una lectura de la fila, sin esperar latidos");
    let state = rig.core.platform().state();
    assert_eq!(state.agent_job, None);
    assert!(state.servers[0].has_agent_token);
}

/// Rotura que atrapa: esperar para siempre a un agente que no arranca, o
/// darlo por vuelto.
#[tokio::test]
async fn an_agent_that_never_speaks_is_silent() {
    let rig = rig().await;
    let current = admin_with_agent(&rig).await;
    mount_mint(&rig).await;
    Mock::given(method("GET")).and(path("/api/v1/servers/a")).respond_with(ok(current)).mount(&rig.server).await;
    let started = std::time::Instant::now();
    let outcome = rig.core.platform().install_agent("a", QUICK, |_, _| Ok(())).await;
    assert_eq!(outcome, Ok(AgentInstallOutcome::Silent));
    assert!(started.elapsed() < QUICK.total * 5, "esperó más de lo que se le dio: {:?}", started.elapsed());
    assert_eq!(rig.core.platform().state().servers[0].agent_version, 2);
}

/// Rotura que atrapa: que tras un agente que no vuelve la fila siga sin
/// identidad, aunque ya se acuñó.
#[tokio::test]
async fn a_silent_install_still_marks_the_identity() {
    let rig = rig().await;
    admin_without_agent(&rig).await;
    mount_mint(&rig).await;
    assert_eq!(rig.core.platform().install_agent("a", QUICK, |_, _| Ok(())).await, Ok(AgentInstallOutcome::Silent));
    assert!(rig.core.platform().state().servers[0].has_agent_token);
}

/// Rotura que atrapa: dejar empezar una segunda instalación mientras la
/// primera sigue: acuñaría otra identidad y tumbaría la que se está llevando.
#[tokio::test]
async fn a_second_install_waits_for_the_first() {
    let rig = rig().await;
    admin_with_agent(&rig).await;
    mount_mint(&rig).await;
    let (release, held) = std::sync::mpsc::channel::<()>();
    let platform = rig.core.platform();
    let first = tokio::spawn(async move {
        platform
            .install_agent("a", QUICK, move |_, _| {
                held.recv().ok();
                Err("cortado".into())
            })
            .await
    });
    while rig.core.platform().state().agent_job.is_none() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let second = rig.core.platform().install_agent("a", QUICK, |_, _| panic!("la segunda no debe llegar al SSH")).await;
    assert_eq!(second, Err(ServerEditError::AgentBusy));
    release.send(()).unwrap();
    assert_eq!(first.await.unwrap(), Err(ServerEditError::Ssh("cortado".into())));
}

// ─── Servicios y logs ────────────────────────────────────────────────────────

use cac_core::{LogStatus, SwarmService};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn web_service() -> Value {
    json!({ "id": "svc1", "name": "shop_web", "image": "ghcr.io/o/web:v3", "stack": "shop", "replicas": { "running": 1, "desired": 2 }, "updatedAt": "2026-11-20T10:00:00Z" })
}

/// Rotura que atrapa: pedir los servicios sin el pase del agente, o leerlos
/// mal; y que un agente caído borre la lista que había.
#[tokio::test]
async fn the_services_tab_asks_the_agent_and_keeps_them_when_it_fails() {
    let rig = rig().await;
    as_role(&rig, "viewer", vec![row(&rig, "s", "o1", "online", true)]).await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p", 600)).mount(&rig.server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/services"))
        .and(header("authorization", "Bearer p"))
        .respond_with(ok(json!([web_service()])))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    Mock::given(method("GET")).and(path("/api/v1/services")).respond_with(ResponseTemplate::new(502)).mount(&rig.server).await;
    rig.core.platform().open_server("s");
    rig.core.platform().open_tab(ServerTab::Services).await;
    let state = rig.core.platform().state();
    let services = state.services.unwrap();
    assert_eq!((services[0].name.as_str(), services[0].replicas.running, services[0].replicas.desired), ("shop_web", 1, 2));
    rig.core.platform().load_services().await;
    let state = rig.core.platform().state();
    assert_eq!(state.services.map(|s| s.len()), Some(1));
    assert!(state.services_error.is_some());
    assert!(!state.services_loading);
}

/// Rotura que atrapa: dejar que un lector reinicie (su pase es de
/// lectura), o que un miembro mande el reinicio sin pase o a otro camino.
#[tokio::test]
async fn a_reader_does_not_restart_and_a_member_does() {
    let rig = rig().await;
    as_role(&rig, "viewer", vec![row(&rig, "s", "o1", "online", true)]).await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p", 600)).mount(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/services/svc1/force-update"))
        .and(header("authorization", "Bearer p"))
        .respond_with(ok(json!({ "message": "service update triggered" })))
        .mount(&rig.server)
        .await;
    rig.core.platform().open_server("s");
    assert!(!rig.core.platform().state().permissions.restart_service);
    assert_eq!(rig.core.platform().restart_service("svc1").await, Err(ServerEditError::ReadOnly));
    assert!(writes(&rig).await.is_empty());

    let rig = rig_as_member_with_open("s").await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p", 600)).mount(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/services/svc1/force-update"))
        .and(header("authorization", "Bearer p"))
        .respond_with(ok(json!({ "message": "service update triggered" })))
        .expect(1)
        .mount(&rig.server)
        .await;
    Mock::given(method("POST")).and(path("/api/v1/services/gone/force-update")).respond_with(ResponseTemplate::new(500)).mount(&rig.server).await;
    assert_eq!(rig.core.platform().restart_service("svc1").await, Ok(()));
    assert!(rig.core.platform().restart_service("gone").await.is_err());
}

async fn rig_as_member_with_open(id: &str) -> Rig {
    let rig = rig().await;
    as_role(&rig, "member", vec![row(&rig, id, "o1", "online", true)]).await;
    rig.core.platform().open_server(id);
    rig
}

/// Un agente de verdad por TCP, que manda el cuerpo del log en estos trozos
/// y con pausas entre ellos, para que lleguen separados. A lo demás (la
/// prueba de vida de la lista) contesta 404.
async fn chunked_agent(chunks: Vec<Vec<u8>>) -> u16 {
    watched_agent(chunks).await.0
}

/// Lo mismo, contando las conexiones de log que la app cerró.
async fn watched_agent(chunks: Vec<Vec<u8>>) -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
    let closed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = closed.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let read = socket.read(&mut request).await.unwrap_or(0);
            if !String::from_utf8_lossy(&request[..read]).contains("/logs") {
                let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            }
            let chunks = chunks.clone();
            let counter = counter.clone();
            tokio::spawn(async move {
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
                for chunk in chunks {
                    socket.write_all(format!("{:x}\r\n", chunk.len()).as_bytes()).await.unwrap();
                    socket.write_all(&chunk).await.unwrap();
                    socket.write_all(b"\r\n").await.unwrap();
                    socket.flush().await.unwrap();
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                // Se queda abierto, como un servicio callado, hasta que la
                // app cuelga.
                let mut rest = [0u8; 64];
                let hung_up = tokio::time::timeout(Duration::from_secs(30), async {
                    while matches!(socket.read(&mut rest).await, Ok(n) if n > 0) {}
                })
                .await;
                if hung_up.is_ok() {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }
    });
    (port, closed)
}

async fn hung_up(closed: &std::sync::atomic::AtomicUsize) -> bool {
    for _ in 0..200 {
        if closed.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

async fn until(rig: &Rig, done: impl Fn(&PlatformState) -> bool) -> PlatformState {
    for _ in 0..200 {
        let state = rig.core.platform().state();
        if done(&state) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no llegó: {:?}", rig.core.platform().state().logs);
}

/// Rotura que atrapa: pasar cada trozo de la red a texto por su cuenta (la
/// «ó» partida sale «��») o pintar crudo el código de color partido entre
/// dos trozos.
#[tokio::test]
async fn a_log_cut_mid_character_is_painted_whole() {
    let rig = rig().await;
    let body = "data: \u{1b}[32mconexión\u{1b}[0m lista\n\n".as_bytes().to_vec();
    let accent = body.iter().position(|&b| b == 0xC3).unwrap();
    let escape = body.iter().position(|&b| b == b'[').unwrap();
    let chunks = vec![body[..escape].to_vec(), body[escape..accent + 1].to_vec(), body[accent + 1..].to_vec()];
    let mut vps = row(&rig, "s", "o1", "online", false);
    vps["agentPort"] = json!(chunked_agent(chunks).await);
    as_role(&rig, "viewer", vec![vps]).await;
    rig.core.platform().open_server("s");
    rig.core.platform().follow_logs(&SwarmService { id: "svc1".into(), name: "shop_web".into(), ..Default::default() });
    let state = until(&rig, |state| state.logs.as_ref().is_some_and(|logs| !logs.lines.is_empty())).await;
    let logs = state.logs.unwrap();
    assert_eq!(logs.status, LogStatus::Streaming);
    let spans = &logs.lines[0].spans;
    assert_eq!(spans.iter().map(|s| s.text.as_str()).collect::<String>(), "conexión lista");
    assert_eq!(spans[0].text, "conexión");
    assert!(spans[0].fg.is_some() && spans[1].fg.is_none());

    rig.core.platform().close_logs();
    assert_eq!(rig.core.platform().state().logs, None);
}

/// Rotura que atrapa: reintentar para siempre contra un agente que no da
/// el log, o dejarlo de golpe al primer fallo; y no decir por qué.
#[tokio::test]
async fn a_log_that_never_opens_gives_up_after_three_tries() {
    let rig = rig().await;
    as_role(&rig, "viewer", vec![row(&rig, "s", "o1", "online", false)]).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/services/svc1/logs"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "success": false, "message": "no such service" })))
        .mount(&rig.server)
        .await;
    rig.core.platform().open_server("s");
    rig.core.platform().follow_logs(&SwarmService { id: "svc1".into(), name: "shop_web".into(), ..Default::default() });
    let state = until(&rig, |state| state.logs.as_ref().is_some_and(|logs| logs.status == LogStatus::Failed)).await;
    assert!(state.logs.unwrap().error.unwrap().contains("no such service"));
    let statuses: Vec<_> = rig.recorder.platform.lock().unwrap().iter().filter_map(|s| s.logs.as_ref().map(|l| l.status)).collect();
    assert!(statuses.contains(&LogStatus::Reconnecting));
    assert_eq!(hits(&rig, "/api/v1/services/svc1/logs").await.len(), 3);
}

/// Un log abierto en la pestaña de servicios de `s`, con su agente.
async fn following(rig: &Rig, chunks: Vec<Vec<u8>>) -> Arc<std::sync::atomic::AtomicUsize> {
    let (port, closed) = watched_agent(chunks).await;
    let mut vps = row(rig, "s", "o1", "online", false);
    vps["agentPort"] = json!(port);
    as_role(rig, "viewer", vec![vps]).await;
    rig.core.platform().open_server("s");
    rig.core.platform().select_tab(ServerTab::Services);
    rig.core.platform().follow_logs(&SwarmService { id: "svc1".into(), name: "shop_web".into(), ..Default::default() });
    until(rig, |state| state.logs.as_ref().is_some_and(|logs| !logs.lines.is_empty())).await;
    closed
}

/// Rotura que atrapa: quitar el log de la pantalla y dejar su conexión
/// abierta al cambiar de pestaña; o cortarlo al pulsar la pestaña en la que
/// ya se está.
#[tokio::test]
async fn leaving_the_tab_stops_the_log() {
    let rig = rig().await;
    let closed = following(&rig, vec![b"data: uno\n\n".to_vec()]).await;
    rig.core.platform().select_tab(ServerTab::Services);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(rig.core.platform().state().logs.is_some(), "la misma pestaña cortó el log");
    assert_eq!(closed.load(std::sync::atomic::Ordering::SeqCst), 0);

    rig.core.platform().select_tab(ServerTab::Overview);
    assert_eq!(rig.core.platform().state().logs, None);
    assert!(hung_up(&closed).await, "la conexión del log siguió abierta");
    rig.core.platform().clear_logs();
    assert_eq!(rig.core.platform().state().logs, None);
}

/// Rotura que atrapa: seguir leyendo el log en segundo plano al irse a
/// Trabajo o a Comunicación.
#[tokio::test]
async fn leaving_platform_stops_the_log() {
    let rig = rig().await;
    rig.core.platform().enter();
    let closed = following(&rig, vec![b"data: uno\n\n".to_vec()]).await;
    rig.core.comms().enter();
    assert_eq!(rig.core.platform().state().logs, None);
    assert!(hung_up(&closed).await, "la conexión del log siguió abierta");
}

/// Rotura que atrapa: avisar (y repintar con una copia del estado entero)
/// por cada línea de un servicio hablador, en vez de por tandas.
#[tokio::test]
async fn a_burst_of_lines_is_painted_at_once() {
    let rig = rig().await;
    let burst: String = (0..20).map(|n| format!("data: línea {n}\n\n")).collect();
    following(&rig, vec![burst.into_bytes()]).await;
    let state = until(&rig, |state| state.logs.as_ref().is_some_and(|logs| logs.lines.len() == 20)).await;
    assert_eq!(state.logs.unwrap().lines.len(), 20);
    let counts: std::collections::BTreeSet<usize> = rig.recorder.platform.lock().unwrap().iter().filter_map(|s| s.logs.as_ref().map(|l| l.lines.len())).collect();
    assert_eq!(counts, [0, 20].into(), "se pintó línea a línea");
}

/// Rotura que atrapa: dar cada apertura por buena y reconectar sin fin a
/// un agente que abre y cierra en el acto, cada vez con sus últimas líneas
/// otra vez (así se comió la memoria la primera versión); o pintar como
/// línea del servicio el aviso de error del agente, o perderlo.
#[tokio::test]
async fn a_log_that_closes_at_once_is_not_reopened_forever() {
    let rig = rig().await;
    as_role(&rig, "viewer", vec![row(&rig, "s", "o1", "online", false)]).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/services/svc1/logs"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: uno\n\nevent: error\ndata: docker: no such service\n\n", "text/event-stream"))
        .mount(&rig.server)
        .await;
    rig.core.platform().open_server("s");
    rig.core.platform().follow_logs(&SwarmService { id: "svc1".into(), name: "shop_web".into(), ..Default::default() });
    let state = until(&rig, |state| state.logs.as_ref().is_some_and(|logs| logs.status == LogStatus::Failed)).await;
    assert_eq!(hits(&rig, "/api/v1/services/svc1/logs").await.len(), 3);
    let logs = state.logs.unwrap();
    assert_eq!(logs.lines.len(), 3, "el aviso de error del agente no es una línea del servicio");
    assert_eq!(logs.error.as_deref(), Some("docker: no such service"));
}

async fn stats_hits(rig: &Rig) -> usize {
    rig.server.received_requests().await.unwrap().iter().filter(|request| request.url.path() == "/api/v1/stats").count()
}

fn container(id: &str, cpu: f64) -> Value {
    json!({ "containerId": id, "nodeId": "n1", "serviceName": "shop_web", "stack": "shop", "state": "running", "cpuPercent": cpu, "onlineCpus": 2, "memUsage": 10, "memLimit": 100 })
}

async fn rig_on_stats() -> Rig {
    let rig = rig_as_member_with_open("s").await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p", 600)).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/stats")).respond_with(ok(json!([container("abc", 12.5)]))).mount(&rig.server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/nodes"))
        .respond_with(ok(json!([{ "id": "n1", "hostname": "manager-1", "role": "manager", "status": "ready", "availability": "active", "engineVersion": "27.3.1" }])))
        .mount(&rig.server)
        .await;
    rig.core.platform().open_tab(ServerTab::Stats).await;
    rig
}

/// Rotura que atrapa: seguir preguntando al agente con la pestaña cerrada
/// (cada lectura recorre todos los contenedores de la máquina), o leer una
/// sola vez y no volver a preguntar.
#[tokio::test]
async fn the_resources_tab_polls_until_you_leave_it() {
    let rig = rig_on_stats().await;
    let state = until(&rig, |state| state.stats.is_some() && state.nodes.is_some()).await;
    assert_eq!(state.stats.unwrap()[0].cpu_percent, 12.5);
    assert_eq!(state.nodes.unwrap()[0].hostname, "manager-1");
    assert!(state.stats_fetched_at.is_some());
    for _ in 0..100 {
        if stats_hits(&rig).await >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(stats_hits(&rig).await >= 3, "no volvió a preguntar");
    rig.core.platform().select_tab(ServerTab::Nodes);
    tokio::time::sleep(Duration::from_millis(30)).await;
    let left_at = stats_hits(&rig).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(stats_hits(&rig).await, left_at, "siguió preguntando fuera de la pestaña");
}

/// Rotura que atrapa: seguir sondeando al irse de Plataforma, o no volver
/// a hacerlo al regresar a la pestaña.
#[tokio::test]
async fn leaving_platform_pauses_the_polling_until_you_come_back() {
    let rig = rig_on_stats().await;
    rig.core.platform().enter();
    until(&rig, |state| state.stats.is_some()).await;
    rig.core.platform().leave();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let left_at = stats_hits(&rig).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(stats_hits(&rig).await, left_at, "siguió preguntando fuera de Plataforma");
    rig.core.platform().enter();
    rig.core.platform().start_stats();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(stats_hits(&rig).await > left_at, "no volvió a preguntar al regresar");
}

/// Rotura que atrapa: enseñar un error ante un swarm sin servicios, nodos
/// o contenedores: el agente omite el `data` vacío.
#[tokio::test]
async fn an_empty_swarm_is_empty_not_an_error() {
    let rig = rig_as_member_with_open("s").await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p", 600)).mount(&rig.server).await;
    for route in ["/api/v1/services", "/api/v1/nodes", "/api/v1/stats"] {
        Mock::given(method("GET")).and(path(route)).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true }))).mount(&rig.server).await;
    }
    rig.core.platform().open_tab(ServerTab::Services).await;
    rig.core.platform().open_tab(ServerTab::Stats).await;
    let state = until(&rig, |state| state.stats.is_some()).await;
    assert_eq!((state.services, state.services_error), (Some(vec![]), None));
    assert_eq!((state.nodes, state.nodes_error), (Some(vec![]), None));
    assert_eq!((state.stats, state.stats_error), (Some(vec![]), None));
}

/// Rotura que atrapa: que pausar no pare, que se lleve lo leído, que volver
/// a la pestaña se salte la pausa, o que reanudar no vuelva a empezar; y que cerrar el servidor deje el sondeo.
#[tokio::test]
async fn pausing_keeps_what_was_read_and_closing_stops_it() {
    let rig = rig_on_stats().await;
    until(&rig, |state| state.stats.is_some()).await;
    rig.core.platform().pause_stats(true);
    tokio::time::sleep(Duration::from_millis(30)).await;
    let paused_at = stats_hits(&rig).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(stats_hits(&rig).await, paused_at, "siguió con la pausa puesta");
    let state = rig.core.platform().state();
    assert!(state.stats_paused);
    assert_eq!(state.stats.map(|stats| stats.len()), Some(1));
    // Volver a la pestaña no quita la pausa.
    rig.core.platform().select_tab(ServerTab::Nodes);
    rig.core.platform().open_tab(ServerTab::Stats).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(stats_hits(&rig).await, paused_at, "volver a la pestaña se saltó la pausa");

    rig.core.platform().refresh_stats().await;
    assert_eq!(stats_hits(&rig).await, paused_at + 1, "refrescar lee una vez aunque esté pausado");

    rig.core.platform().pause_stats(false);
    for _ in 0..100 {
        if stats_hits(&rig).await >= paused_at + 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(stats_hits(&rig).await >= paused_at + 3, "reanudar no volvió a preguntar");

    rig.core.platform().close_server();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let closed_at = stats_hits(&rig).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(stats_hits(&rig).await, closed_at, "siguió con el servidor cerrado");
    assert!(rig.core.platform().state().stats.is_none());
}

/// Rotura que atrapa: no pedir los nodos al abrir su pestaña, o que un
/// agente caído borre los que había.
#[tokio::test]
async fn the_nodes_tab_asks_the_agent_and_keeps_them_when_it_fails() {
    let rig = rig_as_member_with_open("s").await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p", 600)).mount(&rig.server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/nodes"))
        .and(header("authorization", "Bearer p"))
        .respond_with(ok(json!([{ "id": "n1", "hostname": "manager-1", "role": "manager", "status": "ready", "availability": "active", "engineVersion": "27.3.1" }])))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    Mock::given(method("GET")).and(path("/api/v1/nodes")).respond_with(ResponseTemplate::new(502)).mount(&rig.server).await;
    rig.core.platform().open_tab(ServerTab::Nodes).await;
    let nodes = rig.core.platform().state().nodes.unwrap();
    assert_eq!((nodes[0].hostname.as_str(), nodes[0].engine_version.as_str()), ("manager-1", "27.3.1"));
    rig.core.platform().load_nodes().await;
    let state = rig.core.platform().state();
    assert_eq!(state.nodes.map(|nodes| nodes.len()), Some(1));
    assert!(state.nodes_error.is_some());
}

use cac_core::ClusterStatus;

fn cluster(rig: &Rig, id: &str) -> Value {
    let mut cluster = row(rig, id, "o1", "pending", false);
    (cluster["type"], cluster["agentPort"]) = (json!("kubernetes"), json!(0));
    cluster
}

fn health(ready: bool) -> Value {
    json!({
        "nodes": [{ "name": "vmi1", "kubeletVersion": "v1.35.10", "ready": ready }],
        "workloads": [{ "name": "web", "namespace": "cac", "ready": 1, "desired": 2, "healthy": false }],
        "certs": [{ "name": "cac-tls", "namespace": "cac", "dnsNames": ["cac.dev"], "daysLeft": 12, "ready": true }],
        "datastores": [],
    })
}

async fn mount_console(rig: &Rig, id: &str) {
    let get = |what: &str| Mock::given(method("GET")).and(path(format!("/api/v1/servers/{id}/{what}")));
    get("k8s/routes")
        .respond_with(ok(json!({ "gateways": [{ "name": "gw", "namespace": "default", "address": "203.0.113.7", "programmed": true }], "routes": [{ "name": "cac", "namespace": "cac", "gateway": "gw", "hostnames": ["cac.dev"], "links": ["https://cac.dev"], "certReady": true }] })))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    get("k8s/routes").respond_with(ResponseTemplate::new(502)).mount(&rig.server).await;
    get("k8s/health").respond_with(ok(health(true))).mount(&rig.server).await;
    get("integrations")
        .respond_with(ok(json!([
            { "id": "g", "serverId": id, "kind": "grafana", "name": "Grafana", "url": "http://grafana.monitoring:3000", "authMethod": "header", "hasSecret": false },
            { "id": "p", "serverId": id, "kind": "pgadmin", "name": "pgAdmin", "url": "https://pg.dev", "authMethod": "none", "hasSecret": true },
        ])))
        .mount(&rig.server)
        .await;
    for what in ["incidents", "deployables", "provisioning-runs"] {
        get(what).respond_with(ok(json!([]))).mount(&rig.server).await;
    }
}

/// Rotura que atrapa: no leer la consola al abrir un clúster, leerla a
/// medias, o que un fallo al volver a leer borre lo que había.
#[tokio::test]
async fn opening_a_cluster_reads_its_console_and_a_failure_keeps_it() {
    let rig = rig().await;
    mount_console(&rig, "k").await;
    as_role(&rig, "viewer", vec![cluster(&rig, "k")]).await;
    rig.core.platform().open_server_and_load("k").await;
    let hub = rig.core.platform().state().k8s.unwrap();
    assert_eq!(hub.server_id, "k");
    assert_eq!(hub.routes.as_ref().unwrap().routes[0].hostnames, ["cac.dev"]);
    assert_eq!(hub.health.as_ref().unwrap().certs[0].days_left, Some(12));
    assert_eq!(hub.integrations.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Grafana", "pgAdmin"]);
    assert!(hub.error.is_none());
    assert!(!rig.core.platform().state().k8s_loading);

    rig.core.platform().load_k8s().await;
    let hub = rig.core.platform().state().k8s.unwrap();
    assert_eq!(hub.routes.map(|routes| routes.gateways.len()), Some(1), "un fallo borró las rutas");
    assert!(hub.error.is_some());
    rig.core.platform().close_server();
    assert!(rig.core.platform().state().k8s.is_none());
}

/// Rotura que atrapa: dejar un clúster en «pendiente» (la columna del
/// agente de swarm), dar por caído uno que no contesta, o preguntar por la
/// salud de un swarm.
#[tokio::test]
async fn each_cluster_in_the_list_gets_its_badge() {
    let rig = rig().await;
    Mock::given(method("GET")).and(path("/api/v1/servers/k1/k8s/health")).respond_with(ok(health(true))).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/servers/k2/k8s/health")).respond_with(ok(health(false))).mount(&rig.server).await;
    Mock::given(method("GET")).and(path("/api/v1/servers/k3/k8s/health")).respond_with(ResponseTemplate::new(503)).mount(&rig.server).await;
    as_role(&rig, "viewer", vec![cluster(&rig, "k1"), cluster(&rig, "k2"), cluster(&rig, "k3"), row(&rig, "s", "o1", "online", true)]).await;
    let badges = rig.core.platform().state().cluster_badges;
    let of = |id: &str| badges.iter().find(|badge| badge.server_id == id).map(|badge| badge.status.clone());
    assert_eq!(of("k1"), Some(ClusterStatus::Issues { count: 1, detail: "cac/web 1/2".into() }));
    assert_eq!(of("k2"), Some(ClusterStatus::Down { detail: "vmi1".into() }));
    assert!(matches!(of("k3"), Some(ClusterStatus::Unknown { .. })));
    assert_eq!(of("s"), None);
    let asked_swarm = rig.server.received_requests().await.unwrap().iter().any(|request| request.url.path() == "/api/v1/servers/s/k8s/health");
    assert!(!asked_swarm);
}

/// Rotura que atrapa: que un lector vea el secreto guardado, o que un
/// miembro no lo pueda leer.
#[tokio::test]
async fn a_reader_cannot_reveal_and_a_member_reads_the_secret() {
    let rig = rig().await;
    mount_console(&rig, "k").await;
    Mock::given(method("POST")).and(path("/api/v1/servers/k/integrations/p/reveal")).respond_with(ok(json!({ "secret": "admin:hunter2" }))).mount(&rig.server).await;
    as_role(&rig, "viewer", vec![cluster(&rig, "k")]).await;
    rig.core.platform().open_server("k");
    assert_eq!(rig.core.platform().reveal_integration("p").await, Err(ServerEditError::ReadOnly));
    assert!(writes(&rig).await.is_empty(), "un lector llegó a pedir el secreto");

    let rig = rig_with_cluster_as("member").await;
    assert_eq!(rig.core.platform().reveal_integration("p").await.unwrap(), "admin:hunter2");
}

async fn rig_with_cluster_as(role: &str) -> Rig {
    let rig = rig().await;
    mount_console(&rig, "k").await;
    Mock::given(method("POST")).and(path("/api/v1/servers/k/integrations/p/reveal")).respond_with(ok(json!({ "secret": "admin:hunter2" }))).mount(&rig.server).await;
    as_role(&rig, role, vec![cluster(&rig, "k")]).await;
    rig.core.platform().open_server("k");
    rig
}

/// Rotura que atrapa: abrir Grafana en su dirección del clúster (no se
/// alcanza, y entrarías sin cac), o pedirle enlace al proxy para una que
/// abre directa.
#[tokio::test]
async fn an_integration_through_cac_opens_the_proxy_link() {
    let rig = rig_with_cluster_as("viewer").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/servers/k/integrations/g/launch"))
        .respond_with(ok(json!({ "path": "/t/abc", "url": "https://tools.guz-studio.dev/t/abc" })))
        .mount(&rig.server)
        .await;
    rig.core.platform().load_k8s().await;
    let hub = rig.core.platform().state().k8s.unwrap();
    let (grafana, pgadmin) = (&hub.integrations[0], &hub.integrations[1]);
    assert_eq!(rig.core.platform().integration_link(grafana).await.unwrap(), "https://tools.guz-studio.dev/t/abc");
    let before = writes(&rig).await.len();
    assert_eq!(rig.core.platform().integration_link(pgadmin).await.unwrap(), "https://pg.dev");
    assert_eq!(writes(&rig).await.len(), before, "pidió enlace al proxy para una que abre directa");
}

// ─── Despliegues ─────────────────────────────────────────────────────────────

use cac_core::{CiNotify, DeployStatus, NewDeployable, ServerEvent};

fn deployable_row() -> Value {
    json!({ "id": "d1", "orgId": "o1", "serverId": "s", "name": "web", "stack": "shop", "serviceName": "shop_web", "imageRepo": "ghcr.io/o/web", "onCINotify": "record", "ciKeyPreview": "", "currentImage": "ghcr.io/o/web:abc1234" })
}

fn deployment_row(id: &str, status: &str) -> Value {
    json!({ "id": id, "createdAt": "2026-11-21T10:00:00Z", "deployableId": "d1", "serverId": "s", "image": "ghcr.io/o/web:abc1234", "requestedBy": "user", "status": status })
}

/// Un servidor `s` abierto con un desplegable `d1` mirado: su historial
/// tiene un despliegue de Ana.
async fn deploys_as(role: &str) -> Rig {
    let rig = rig().await;
    as_role(&rig, role, vec![row(&rig, "s", "o1", "online", true)]).await;
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    get("/api/v1/servers/s/deployables").respond_with(ok(json!([deployable_row()]))).mount(&rig.server).await;
    let mut first = deployment_row("x1", "succeeded");
    first["requestedByName"] = json!("Ana");
    get("/api/v1/servers/s/deployables/d1/deployments").respond_with(ok(json!([first]))).mount(&rig.server).await;
    get("/api/v1/servers/s/deployables/d1/builds").respond_with(ok(json!([]))).mount(&rig.server).await;
    get("/api/v1/servers/s/deployables/d1/commits").respond_with(ok(json!([{ "sha": "def5678", "message": "arregla", "author": "ana", "date": "2026-11-21" }]))).mount(&rig.server).await;
    rig.core.platform().open_server("s");
    Mock::given(method("GET")).and(path("/api/v1/services")).respond_with(ok(json!([web_service()]))).mount(&rig.server).await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(pass("p", 600)).mount(&rig.server).await;
    rig.core.platform().open_tab(ServerTab::Deploy).await;
    assert_eq!(rig.core.platform().state().services.map(|s| s.len()), Some(1), "la pestaña pide los servicios del swarm");
    rig.core.platform().choose_service("shop_web").await;
    assert_eq!(rig.core.platform().state().deploys.unwrap().selected.as_deref(), Some("d1"), "elegir un servicio registrado mira su desplegable");
    rig
}

fn event(name: &str, data: Value) -> ServerEvent {
    ServerEvent { event: name.into(), data: data.to_string() }
}

/// Rotura que atrapa: volver a pedir el historial por cada evento (un
/// despliegue que avanza no puede depender de refrescar), sustituir la fila
/// y perder quién lo pidió, o pegar al log que se mira las líneas de otro
/// despliegue.
#[tokio::test]
async fn the_stream_moves_a_deployment_without_asking_again() {
    let rig = deploys_as("viewer").await;
    let deploys = rig.core.platform().state().deploys.unwrap();
    assert_eq!(deploys.deployables.unwrap()[0].on_ci_notify, CiNotify::Record);
    assert_eq!(deploys.commits.unwrap()[0].sha, "def5678");

    rig.core.stream().deliver(event("deploy:status", deployment_row("x1", "running")));
    rig.core.stream().deliver(event("deploy:status", deployment_row("x2", "queued")));
    let history = rig.core.platform().state().deploys.unwrap().history;
    assert_eq!(history.iter().map(|d| (d.id.as_str(), d.status)).collect::<Vec<_>>(), [("x2", DeployStatus::Queued), ("x1", DeployStatus::Running)]);
    assert_eq!(history[1].requested_by_name.as_deref(), Some("Ana"));
    assert_eq!(hits(&rig, "/api/v1/servers/s/deployables/d1/deployments").await.len(), 1);

    Mock::given(method("GET")).and(path("/api/v1/servers/s/deployables/d1/deployments/x2")).respond_with(ok(json!({ "id": "x2", "log": "pull\n" }))).mount(&rig.server).await;
    rig.core.platform().open_deploy_log("x2").await;
    rig.core.stream().deliver(event("deploy:log", json!({ "deploymentId": "x2", "deployableId": "d1", "lines": ["update"] })));
    rig.core.stream().deliver(event("deploy:log", json!({ "deploymentId": "x1", "deployableId": "d1", "lines": ["ajeno"] })));
    assert_eq!(rig.core.platform().state().deploys.unwrap().log, "pull\nupdate\n");

    rig.core.stream().deliver(event("deploy:build", json!({ "id": "b1", "deployableId": "d1", "sha": "fff0000" })));
    let deploys = rig.core.platform().state().deploys.unwrap();
    assert_eq!(deploys.builds.len(), 1);
    assert!(deploys.commits_stale);
}

/// Rotura que atrapa: dejar que un lector despliegue o vuelva atrás, o que
/// un miembro mande otro sha u otro despliegue que el elegido.
#[tokio::test]
async fn a_member_deploys_and_rolls_back_what_was_chosen() {
    let reader = deploys_as("viewer").await;
    assert_eq!(reader.core.platform().deploy("d1", "def5678").await, Err(ServerEditError::ReadOnly));
    assert_eq!(reader.core.platform().rollback("d1", "x1").await, Err(ServerEditError::ReadOnly));
    assert!(writes(&reader).await.iter().all(|r| !r.url.path().contains("deployables")));

    let rig = deploys_as("member").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/servers/s/deployables/d1/deploy"))
        .and(body_json(json!({ "sha": "def5678" })))
        .respond_with(ok(deployment_row("x2", "queued")))
        .expect(1)
        .mount(&rig.server)
        .await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/deployables/d1/deployments/x1/rollback")).respond_with(ok(deployment_row("x3", "queued"))).expect(1).mount(&rig.server).await;
    rig.core.platform().deploy("d1", "def5678").await.unwrap();
    rig.core.platform().rollback("d1", "x1").await.unwrap();
    let history = rig.core.platform().state().deploys.unwrap().history;
    assert_eq!(history.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["x3", "x2", "x1"]);
}

/// Rotura que atrapa: dejar la llave del CI entera en el estado (se pinta
/// y viaja a cada aviso), o que un miembro registre o ajuste un servicio
/// (los ajustes llevan un `sh -c` que corre en el servidor).
#[tokio::test]
async fn the_ci_key_is_shown_once_and_only_an_admin_manages() {
    let member = deploys_as("member").await;
    assert_eq!(member.core.platform().mint_ci_key("d1").await, Err(ServerEditError::AdminOnly));
    assert_eq!(member.core.platform().create_deployable(&NewDeployable::default()).await.err(), Some(ServerEditError::AdminOnly));

    let rig = deploys_as("admin").await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/deployables/d1/ci-key")).respond_with(ok(json!({ "key": "cac_dk_SECRETO", "preview": "cac_dk_SE" }))).mount(&rig.server).await;
    let key = rig.core.platform().mint_ci_key("d1").await.unwrap();
    assert_eq!(key.key, "cac_dk_SECRETO");
    let painted = format!("{:?}", rig.recorder.platform.lock().unwrap());
    assert!(!painted.contains("cac_dk_SECRETO"));
    assert_eq!(rig.core.platform().state().deploys.unwrap().deployables.unwrap()[0].ci_key_preview, "cac_dk_SE");

    let mut settings = rig.core.platform().state().deploys.unwrap().deployables.unwrap()[0].settings();
    settings.migrate_command = "npx prisma migrate deploy".into();
    let mut back = deployable_row();
    back["migrateCommand"] = json!("npx prisma migrate deploy");
    Mock::given(method("PATCH"))
        .and(path("/api/v1/servers/s/deployables/d1"))
        .and(body_json(json!({ "name": "web", "environment": "", "repoFullName": "", "onCINotify": "record", "buildWorkflow": "", "shortTags": false, "migrateCommand": "npx prisma migrate deploy", "rotateCommand": "", "branch": "", "buildOnHost": false, "dockerfile": "", "buildContext": "" })))
        .respond_with(ok(back))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.core.platform().update_deployable("d1", &settings).await.unwrap();
    assert_eq!(rig.core.platform().state().deploys.unwrap().deployables.unwrap()[0].migrate_command, "npx prisma migrate deploy");
}

// ─── Secrets por referencia ──────────────────────────────────────────────────

use cac_core::{SecretRefRow, SecretUsage};

/// Rotura que atrapa: dejar que un miembro cambie de dónde salen los
/// secrets, mandar las filas sin recortar o sin su uso, o apuntar una
/// rotación con algo más que nombres y versiones.
#[tokio::test]
async fn refs_are_saved_by_an_admin_and_a_rotation_records_only_names() {
    let member = deploys_as("member").await;
    let rows = [SecretRefRow { name: " DATABASE_URL ".into(), op_ref: "op://Prod/api/url ".into(), usage: SecretUsage::Both }];
    assert_eq!(member.core.platform().save_secret_refs("d1", &rows).await, Err(ServerEditError::AdminOnly));

    let rig = deploys_as("admin").await;
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    get("/api/v1/servers/s/deployables/d1/secret-refs").respond_with(ok(json!([{ "name": "DATABASE_URL", "opRef": "op://Prod/api/url", "usage": "both" }]))).mount(&rig.server).await;
    get("/api/v1/servers/s/deployables/d1/secret-rotations").respond_with(ok(json!([]))).mount(&rig.server).await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/servers/s/deployables/d1/secret-refs"))
        .and(body_json(json!({ "refs": [{ "name": "DATABASE_URL", "opRef": "op://Prod/api/url", "usage": "both" }] })))
        .respond_with(ok(json!(null)))
        .expect(1)
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/servers/s/deployables/d1/secret-rotations"))
        .and(body_json(json!({ "names": ["DATABASE_URL"], "versions": { "DATABASE_URL": "DATABASE_URL_a1b2" }, "status": "succeeded", "error": "" })))
        .respond_with(ok(json!(null)))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.core.platform().save_secret_refs("d1", &rows).await.unwrap();
    assert_eq!(rig.core.platform().state().secrets.unwrap().refs.unwrap()[0].usage, SecretUsage::Both);
    rig.core.platform().record_rotation("d1", &["DATABASE_URL".into()], &[("DATABASE_URL".into(), "DATABASE_URL_a1b2".into())], None).await.unwrap();

    Mock::given(method("POST"))
        .and(path("/api/v1/servers/s/deployables/d1/secret-rotations"))
        .and(body_json(json!({ "names": ["DATABASE_URL"], "versions": {}, "status": "failed", "error": "op: locked" })))
        .respond_with(ok(json!(null)))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.core.platform().record_rotation("d1", &["DATABASE_URL".into()], &[], Some("op: locked")).await.unwrap();
}
