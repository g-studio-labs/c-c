//! Provisioning contra un servidor de mentira: la config, las plantillas y la
//! bandeja de peticiones. Ningún playbook corre aquí: el motor de Ansible es
//! uno de mentira que apunta lo que le piden. Cada prueba dice la rotura que
//! atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Config, Core, Listener, PlatformState, ProvisionError, SecretRef, ServerTab, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, method, path};
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
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn platform_changed(&self, state: PlatformState) {
        self.platform.lock().unwrap().push(state);
    }
}

pub struct Rig {
    pub server: MockServer,
    pub core: Core,
    recorder: Arc<Recorder>,
}

impl Rig {
    /// Todo lo que la pantalla llegó a oír, como texto: lo que no puede
    /// aparecer ahí no aparece en ningún aviso.
    pub fn heard(&self) -> String {
        format!("{:?}", self.recorder.platform.lock().unwrap())
    }
}

pub fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

pub fn get(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path(route.to_string()))
}

pub fn request_row(id: &str, risk: &str, status: &str, flags: Value) -> Value {
    json!({
        "id": id, "createdAt": "2026-11-21T10:00:00Z", "serverId": "s", "playbook": "swarm", "project": "ansible",
        "risk": risk, "verify": "swarm-check", "rollback": "swarm-down", "configSeq": 4, "reason": "", "requestedBy": "u2",
        "source": "mcp", "status": status, "flags": flags, "autoApproved": false, "requestedByName": "Luis",
        "check": { "hosts": ["gstd"], "ok": 3, "changed": 1, "unreachable": 0, "failed": 0, "changedTasks": ["install docker"], "paths": null, "diff": "", "exitCode": 0, "failures": null, "logTail": "" }
    })
}

pub fn effective() -> Value {
    json!({
        "serverId": "s", "configSeq": 4, "templateId": "t1", "templateName": "Swarm", "templateSeq": 2, "playbooks": ["baseline", "swarm"],
        "vars": { "domain": "x.dev" }, "opRefs": { "OPS_PASSWORD": "op://Infra/gstd/password" }, "requires": ["OPS_PASSWORD"], "perServer": ["OPS_PASSWORD"], "missing": [],
        "connection": { "name": "gstd", "host": "203.0.113.9", "sshPort": 22, "sshUser": "ops" }
    })
}

/// Un servidor `s` (`gstd`) con Provisioning abierto, mirado con ese papel.
pub async fn provision_as(role: &str) -> Rig {
    let server = MockServer::start().await;
    let recorder = Arc::new(Recorder::default());
    let mut config = Config::new(server.uri());
    config.stream_backoff = Duration::from_millis(5);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    let rig = Rig { server, core, recorder };
    let org = json!({ "id": "o1", "name": "Org", "slug": "o1", "role": role, "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" });
    get("/api/v1/organizations/").respond_with(ok(json!([org]))).mount(&rig.server).await;
    rig.core.orgs().load().await;
    let row = json!({ "id": "s", "orgId": "o1", "name": "gstd", "host": "203.0.113.9", "sshPort": 22, "sshUser": "ops", "type": "kubernetes", "agentPort": 9090, "status": "online" });
    get("/api/v1/servers/").respond_with(ok(json!([row]))).mount(&rig.server).await;
    rig.core.platform().refresh().await;
    let version = json!({ "id": "v4", "serverId": "s", "seq": 4, "templateId": "t1", "templateSeq": 2, "vars": { "domain": "x.dev" }, "opRefs": { "OPS_PASSWORD": "op://Infra/gstd/password" }, "source": "app", "createdByName": "Ana" });
    get("/api/v1/servers/s/config").respond_with(ok(json!({ "current": version, "versions": [version] }))).mount(&rig.server).await;
    get("/api/v1/servers/s/config/effective").respond_with(ok(effective())).mount(&rig.server).await;
    get("/api/v1/organizations/o1/server-templates/")
        .respond_with(ok(json!([{ "id": "t1", "orgId": "o1", "name": "Swarm", "current": { "id": "tv2", "templateId": "t1", "seq": 2, "playbooks": ["baseline", "swarm"], "vars": {}, "requires": ["OPS_PASSWORD"], "perServer": ["OPS_PASSWORD"] } }])))
        .mount(&rig.server)
        .await;
    get("/api/v1/servers/s/provisioning-runs").respond_with(ok(json!([{ "id": "x1", "playbook": "swarm", "status": "succeeded", "startedAt": "2026-11-20T10:00:00Z", "configSeq": 3, "applied": { "vars": { "domain": "x.dev" }, "opRefs": {} } }]))).mount(&rig.server).await;
    get("/api/v1/servers/s/provisioning-requests")
        .respond_with(ok(json!([
            request_row("r1", "destructive", "awaiting_approval", json!([{ "code": "destructive", "level": "red" }])),
            request_row("r2", "change", "checking", json!(null)),
        ])))
        .mount(&rig.server)
        .await;
    rig.core.platform().open_server("s");
    rig.core.platform().open_tab(ServerTab::Provision).await;
    rig
}

pub async fn writes(rig: &Rig) -> Vec<Request> {
    rig.server.received_requests().await.unwrap().into_iter().filter(|r| r.method.as_str() != "GET").collect()
}

/// Rotura que atrapa: una pestaña que no pide lo suyo (config, efectiva,
/// plantillas, historial y bandeja), o que lo lee mal.
#[tokio::test]
async fn the_tab_reads_the_config_the_templates_and_the_inbox() {
    let rig = provision_as("viewer").await;
    let provision = rig.core.platform().state().provision.expect("la pestaña no abrió Provisioning");
    assert_eq!(provision.config.unwrap().current.unwrap().seq, 4);
    assert_eq!(provision.effective.unwrap().connection.name, "gstd");
    assert_eq!(provision.templates[0].current.as_ref().unwrap().playbooks, ["baseline", "swarm"]);
    assert_eq!(provision.runs[0].applied_text().unwrap().contains("x.dev"), true);
    assert_eq!(provision.requests.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r1", "r2"]);
    assert!(!provision.permissions.decide && !provision.permissions.edit_config && !provision.permissions.run);
    assert_eq!(provision.me, "u1");
}

/// Rotura que atrapa: dejar que quien no administra guarde la config, o
/// mandar un valor pegado en vez de su referencia; o guardar algo distinto de
/// lo escrito.
#[tokio::test]
async fn only_an_admin_saves_the_config_and_only_references_travel() {
    let member = provision_as("member").await;
    let refs = [SecretRef { name: "OPS_PASSWORD".into(), reference: "op://Infra/gstd/password".into() }];
    assert_eq!(member.core.platform().save_config("t1", "{}", &refs, "").await, Err(ProvisionError::AdminOnly));
    assert!(writes(&member).await.is_empty(), "un miembro llegó a guardar");

    let rig = provision_as("admin").await;
    let pasted = [SecretRef { name: "OPS_PASSWORD".into(), reference: "hunter2".into() }];
    assert_eq!(rig.core.platform().save_config("t1", "{}", &pasted, "").await, Err(ProvisionError::Config(cac_core::ConfigProblem::BadRefs)));
    assert!(writes(&rig).await.is_empty(), "un valor pegado llegó a viajar");
    assert!(!rig.heard().contains("hunter2"));

    Mock::given(method("PUT"))
        .and(path("/api/v1/servers/s/config"))
        .and(body_json(json!({ "templateId": "t1", "vars": { "domain": "y.dev" }, "opRefs": { "OPS_PASSWORD": "op://Infra/gstd/password" }, "note": "cambio" })))
        .respond_with(ok(json!({ "id": "v5", "seq": 5 })))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.core.platform().save_config("t1", "{\"domain\": \"y.dev\"}", &refs, " cambio ").await.unwrap();
}

/// Rotura que atrapa: dejar que un miembro apruebe, aprobar una destructiva
/// sin el nombre exacto del servidor o con una bandera roja sin reconocer, o
/// mandar otra decisión que la tomada.
#[tokio::test]
async fn approving_needs_an_admin_the_exact_name_and_the_red_flags_seen() {
    let member = provision_as("member").await;
    assert_eq!(member.core.platform().decide_request("r1", true, "", "gstd", true).await, Err(ProvisionError::AdminOnly));
    assert!(writes(&member).await.is_empty());

    let rig = provision_as("admin").await;
    assert_eq!(rig.core.platform().decide_request("r1", true, "", "GSTD", true).await, Err(ProvisionError::ConfirmServerName));
    assert_eq!(rig.core.platform().decide_request("r1", true, "", "gstd", false).await, Err(ProvisionError::AcknowledgeFlags));
    assert!(writes(&rig).await.is_empty(), "se mandó una aprobación que el protocolo no deja");

    let mut approved = request_row("r1", "destructive", "approved", json!([{ "code": "destructive", "level": "red" }]));
    approved["grantExpiresAt"] = json!("2026-11-21T10:15:00Z");
    Mock::given(method("POST"))
        .and(path("/api/v1/servers/s/provisioning-requests/r1/decision"))
        .and(body_json(json!({ "approve": true, "note": "adelante", "confirmServer": "gstd", "acknowledgeFlags": true })))
        .respond_with(ok(approved))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.core.platform().decide_request("r1", true, " adelante ", "gstd", true).await.unwrap();
    let request = rig.core.platform().state().provision.unwrap().requests.into_iter().find(|r| r.id == "r1").unwrap();
    assert_eq!(request.status, "approved");
}

/// Rotura que atrapa: dejar que otro miembro descarte la petición de Luis, o
/// no dejar descartar a un admin.
#[tokio::test]
async fn a_stale_request_is_discarded_by_who_asked_or_an_admin() {
    let member = provision_as("member").await;
    assert_eq!(member.core.platform().discard_request("r2").await, Err(ProvisionError::NotYours));
    assert_eq!(member.core.platform().reverify_request("r2").await, Err(ProvisionError::NotYours));
    assert!(writes(&member).await.is_empty());

    let rig = provision_as("admin").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/servers/s/provisioning-requests/r2/discard"))
        .respond_with(ok(request_row("r2", "change", "discarded", json!([]))))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.core.platform().discard_request("r2").await.unwrap();
    let request = rig.core.platform().state().provision.unwrap().requests.into_iter().find(|r| r.id == "r2").unwrap();
    assert_eq!(request.status, "discarded");
}

// ─── Correr ──────────────────────────────────────────────────────────────────

use cac_core::{AnsibleEngine, AnsibleManifest, AnsibleTools, EngineEvent, EngineSink, InventoryHost, ManualInputs, ManualRun, PlanGaps};

/// El motor de mentira: apunta lo que le piden y cuenta una salida. Con
/// `fail`, Ansible no llega a arrancar (1Password no contesta).
#[derive(Default)]
struct FakeEngine {
    manifest: Mutex<Option<Value>>,
    fail: Option<String>,
    /// Cada ejecución: el playbook y si era un ensayo.
    runs: Mutex<Vec<(String, bool)>>,
    /// Lo que dijo cada ejecución a mano, para ver qué se tachó.
    manual: Mutex<Vec<(Option<String>, Vec<String>)>>,
    reports: Mutex<Vec<(Vec<String>, i32)>>,
    /// Cómo acaba cada ejecución, por playbook (si no, bien).
    exits: Mutex<HashMap<String, i32>>,
    /// Cuándo se lanzó cada ejecución.
    started: Mutex<Vec<std::time::Instant>>,
}

fn v2_manifest() -> Value {
    json!({ "version": 2, "playbooks": [
        { "id": "baseline", "file": "baseline.yml", "name": "Baseline", "risk": "change", "verify": "baseline-check" },
        { "id": "swarm", "file": "swarm.yml", "name": "Swarm", "risk": "change", "verify": "swarm-check" },
        { "id": "swarm-check", "file": "swarm-check.yml", "name": "Comprobar", "risk": "read" },
    ] })
}

impl FakeEngine {
    fn with(manifest: Value) -> Arc<FakeEngine> {
        Arc::new(FakeEngine { manifest: Mutex::new(Some(manifest)), ..Default::default() })
    }
    fn failing(error: &str) -> Arc<FakeEngine> {
        Arc::new(FakeEngine { manifest: Mutex::new(Some(v2_manifest())), fail: Some(error.into()), ..Default::default() })
    }
    fn say(sink: EngineSink, lines: Vec<String>, code: i32) {
        std::thread::spawn(move || {
            for line in lines {
                sink(EngineEvent::Line(line));
            }
            sink(EngineEvent::Exit { code: Some(code), cancelled: false });
        });
    }
}

impl AnsibleEngine for FakeEngine {
    fn manifest(&self, _project_dir: &str) -> Result<AnsibleManifest, String> {
        Ok(serde_json::from_value(self.manifest.lock().unwrap().clone().unwrap()).unwrap())
    }
    fn tools(&self, _project_dir: &str, _venv: Option<&str>) -> AnsibleTools {
        AnsibleTools { platform_supported: true, reason: String::new(), ansible_playbook: Some("/usr/bin/ansible-playbook".into()) }
    }
    fn inventory(&self, _project_dir: &str, _inventory: &str, _venv: Option<&str>) -> Result<Vec<InventoryHost>, String> {
        Ok(vec![InventoryHost { name: "gstd".into(), groups: vec!["swarm".into()], ansible_host: None }])
    }
    fn plan_gaps(&self, _project_dir: &str, _playbook_id: &str, _effective: &str) -> Result<PlanGaps, String> {
        Ok(PlanGaps::default())
    }
    fn run_with_config(&self, _project_dir: &str, playbook_id: &str, _effective: &str, check: bool, sink: EngineSink) -> Result<String, String> {
        self.runs.lock().unwrap().push((playbook_id.to_string(), check));
        self.started.lock().unwrap().push(std::time::Instant::now());
        if let Some(error) = &self.fail {
            return Err(error.clone());
        }
        let code = self.exits.lock().unwrap().get(playbook_id).copied().unwrap_or(0);
        FakeEngine::say(sink, vec![format!("TASK [{playbook_id}]"), "PLAY RECAP ***".into(), "gstd : ok=1 changed=0".into()], code);
        Ok(format!("run-{playbook_id}"))
    }
    fn run_manual(&self, spec: ManualRun, sink: EngineSink) -> Result<String, String> {
        let password = spec.become_password.clone();
        // Un playbook que se equivoca y escribe lo que le dieron.
        let said = vec![format!("sudo: {}", password.clone().unwrap_or_default()), format!("token={}", spec.vars.iter().find(|v| v.name == "api_token").and_then(|v| v.value.clone()).unwrap_or_default())];
        self.manual.lock().unwrap().push((password, said.clone()));
        FakeEngine::say(sink, said, 0);
        Ok("run-manual".into())
    }
    fn cancel(&self, _run_id: &str) -> Result<(), String> {
        Ok(())
    }
    fn check_report(&self, lines: &[String], exit_code: i32) -> Value {
        self.reports.lock().unwrap().push((lines.to_vec(), exit_code));
        json!({ "hosts": ["gstd"], "ok": 1, "changed": 0, "exitCode": exit_code, "logTail": lines.join("\n") })
    }
}

async fn with_project(rig: &Rig, engine: &Arc<FakeEngine>) {
    rig.core.platform().set_project_dir("/home/ana/infra/ansible");
    rig.core.platform().load_project(engine.clone()).await;
    assert!(rig.core.platform().state().provision.unwrap().project.unwrap().manifest.is_some(), "no leyó el manifiesto");
}

fn requests_in_order(requests: &[Request]) -> Vec<String> {
    requests.iter().filter(|r| r.method.as_str() != "GET").map(|r| format!("{} {}", r.method, r.url.path())).collect()
}

/// Rotura que atrapa: ensayar sin `--check` (aplicaría de verdad), no seguir
/// el orden de la plantilla, abrir una ejecución en cac por un ensayo, no
/// mandar su informe, o seguir con la etapa siguiente tras un ensayo que
/// falla.
#[tokio::test]
async fn rehearsing_never_runs_without_check() {
    let rig = provision_as("member").await;
    let engine = FakeEngine::with(v2_manifest());
    with_project(&rig, &engine).await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/provisioning-requests")).respond_with(ok(request_row("r9", "change", "checking", json!([])))).mount(&rig.server).await;
    Mock::given(method("PUT")).and(path("/api/v1/servers/s/provisioning-requests/r9/check")).respond_with(ok(request_row("r9", "change", "awaiting_approval", json!([])))).mount(&rig.server).await;
    rig.core.platform().rehearse(engine.clone(), &["baseline".into(), "swarm".into()]).await.unwrap();
    assert_eq!(*engine.runs.lock().unwrap(), [("baseline".to_string(), true), ("swarm".to_string(), true)]);
    let sent = writes(&rig).await;
    assert!(!sent.iter().any(|r| r.url.path().ends_with("/provisioning-runs")), "un ensayo abrió una ejecución en cac");
    let asked: Vec<Value> = sent.iter().filter(|r| r.method.as_str() == "POST").map(|r| serde_json::from_slice(&r.body).unwrap()).collect();
    assert_eq!(asked[0], json!({ "playbook": "baseline", "project": "ansible", "risk": "change", "verify": "baseline-check", "rollback": "", "configSeq": 4, "reason": "" }));
    assert_eq!(sent.iter().filter(|r| r.method.as_str() == "PUT").count(), 2, "no mandó el informe de cada ensayo");
    let output = rig.core.platform().state().provision_output.unwrap();
    assert!(!output.running);

    // El ensayo de una etapa que falla para la plantilla: la siguiente
    // puede fallar sólo porque ésta no está aplicada.
    engine.runs.lock().unwrap().clear();
    engine.exits.lock().unwrap().insert("baseline".into(), 2);
    rig.core.platform().rehearse(engine.clone(), &["baseline".into(), "swarm".into()]).await.unwrap();
    assert_eq!(*engine.runs.lock().unwrap(), [("baseline".to_string(), true)], "siguió ensayando tras un ensayo que falló");
}

/// Rotura que atrapa: dejar la petición «ensayando» hasta caducar cuando el
/// ensayo no llega a arrancar, en vez de cerrarla como ensayo fallido.
#[tokio::test]
async fn a_rehearsal_that_does_not_start_closes_its_request() {
    let rig = provision_as("member").await;
    let engine = FakeEngine::failing("[ERROR] op: no contesta");
    with_project(&rig, &engine).await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/provisioning-requests")).respond_with(ok(request_row("r9", "change", "checking", json!([])))).mount(&rig.server).await;
    Mock::given(method("PUT")).and(path("/api/v1/servers/s/provisioning-requests/r9/check")).respond_with(ok(request_row("r9", "change", "awaiting_approval", json!([])))).expect(1).mount(&rig.server).await;
    let failed = rig.core.platform().rehearse(engine.clone(), &["swarm".into()]).await;
    assert_eq!(failed, Err(ProvisionError::Engine("[ERROR] op: no contesta".into())));
    assert_eq!(*engine.reports.lock().unwrap(), [(vec!["[ERROR] op: no contesta".to_string()], -1)]);
    assert!(!rig.core.platform().state().provision_output.unwrap().running, "se quedó «aplicando»");
}

fn approved(id: &str, risk: &str) -> Value {
    let mut row = request_row(id, risk, "approved", json!([]));
    row["grantExpiresAt"] = json!("2026-11-21T10:15:00Z");
    row
}

const CLOSING: Duration = Duration::from_millis(300);

async fn grant_rig(fail: Option<&str>) -> (Rig, Arc<FakeEngine>) {
    let rig = provision_as("admin").await;
    rig.server.reset().await;
    let engine = match fail {
        Some(error) => FakeEngine::failing(error),
        None => FakeEngine::with(v2_manifest()),
    };
    get("/api/v1/servers/").respond_with(ok(json!([{ "id": "s", "orgId": "o1", "name": "gstd", "host": "203.0.113.9", "sshPort": 22, "sshUser": "ops", "type": "kubernetes", "agentPort": 9090, "status": "online" }]))).mount(&rig.server).await;
    get("/api/v1/servers/s/config/effective").respond_with(ok(effective())).mount(&rig.server).await;
    get("/api/v1/servers/s/provisioning-runs").respond_with(ok(json!([]))).mount(&rig.server).await;
    let mut read = approved("r4", "read");
    read["playbook"] = json!("swarm-check");
    get("/api/v1/servers/s/provisioning-requests").respond_with(ok(json!([approved("r3", "change"), read]))).mount(&rig.server).await;
    Mock::given(method("POST")).and(path("/api/v1/servers/s/provisioning-runs")).respond_with(ok(json!({ "id": "run1" }))).mount(&rig.server).await;
    // Cerrar tarda: la verificación tiene que esperar a que cac conteste.
    Mock::given(method("PATCH")).and(path("/api/v1/servers/s/provisioning-runs/run1")).respond_with(ok(json!(null)).set_delay(CLOSING)).mount(&rig.server).await;
    rig.core.platform().reload_requests().await;
    with_project(&rig, &engine).await;
    (rig, engine)
}

/// Rotura que atrapa: gastar la autorización cuando Ansible no llegó a
/// arrancar (sin `notStarted`, cac obligaría a ensayar y aprobar otra vez), o
/// correr la verificación de algo que no se aplicó.
#[tokio::test]
async fn a_failure_before_starting_sends_not_started() {
    let (rig, engine) = grant_rig(Some("op: 1Password no contesta")).await;
    let failed = rig.core.platform().run_grant(engine.clone(), "r3").await;
    assert_eq!(failed, Err(ProvisionError::Engine("op: 1Password no contesta".into())));
    assert_eq!(*engine.runs.lock().unwrap(), [("swarm".to_string(), false)], "corrió la verificación de algo que no se aplicó");
    let closed: Vec<Value> = writes(&rig).await.iter().filter(|r| r.method.as_str() == "PATCH").map(|r| serde_json::from_slice(&r.body).unwrap()).collect();
    assert_eq!(closed, [json!({ "status": "failed", "summary": "op: 1Password no contesta", "logTail": "", "notStarted": true })]);
}

/// Rotura que atrapa: arrancar la verificación antes de cerrar el playbook en
/// cac (la petición se quedaba «verificando» para siempre, gstd, 2-nov), no
/// gastar la autorización al abrir, o correr en ensayo lo autorizado.
#[tokio::test]
async fn the_verification_runs_after_the_playbook_is_closed() {
    let (rig, engine) = grant_rig(None).await;
    rig.core.platform().run_grant(engine.clone(), "r3").await.unwrap();
    assert_eq!(*engine.runs.lock().unwrap(), [("swarm".to_string(), false), ("swarm-check".to_string(), false)]);
    let sent = rig.server.received_requests().await.unwrap();
    assert_eq!(
        requests_in_order(&sent),
        ["POST /api/v1/servers/s/provisioning-runs", "PATCH /api/v1/servers/s/provisioning-runs/run1", "POST /api/v1/servers/s/provisioning-runs", "PATCH /api/v1/servers/s/provisioning-runs/run1"]
    );
    let opened: Value = serde_json::from_slice(&sent.iter().find(|r| r.method.as_str() == "POST").unwrap().body).unwrap();
    assert_eq!((opened["playbook"].clone(), opened["requestId"].clone(), opened["configSeq"].clone(), opened["target"].clone()), (json!("swarm"), json!("r3"), json!(4), json!("gstd")));
    let first_close: Value = serde_json::from_slice(&sent.iter().find(|r| r.method.as_str() == "PATCH").unwrap().body).unwrap();
    assert_eq!((first_close["status"].clone(), first_close["summary"].clone()), (json!("succeeded"), json!("PLAY RECAP ***\ngstd : ok=1 changed=0")));
    let started = engine.started.lock().unwrap().clone();
    assert!(started[1] - started[0] >= CLOSING, "la verificación arrancó sin esperar a que cac cerrara el playbook");
}

/// Rotura que atrapa: verificar algo que no se aplicó bien, o «verificar»
/// una lectura (no cambió nada que comprobar).
#[tokio::test]
async fn only_a_change_that_went_well_is_verified() {
    let (rig, engine) = grant_rig(None).await;
    engine.exits.lock().unwrap().insert("swarm".into(), 2);
    rig.core.platform().run_grant(engine.clone(), "r3").await.unwrap();
    assert_eq!(*engine.runs.lock().unwrap(), [("swarm".to_string(), false)], "verificó lo que falló");
    engine.runs.lock().unwrap().clear();
    rig.core.platform().run_grant(engine.clone(), "r4").await.unwrap();
    assert_eq!(*engine.runs.lock().unwrap(), [("swarm-check".to_string(), false)], "verificó una lectura");
}

/// Rotura que atrapa: correr lo aprobado como `change` cuando el manifiesto
/// ya dice otra cosa (lo aprobado ya no es esto).
#[tokio::test]
async fn a_risk_that_changed_is_asked_again() {
    let (rig, engine) = grant_rig(None).await;
    *engine.manifest.lock().unwrap() = Some(json!({ "version": 2, "playbooks": [{ "id": "swarm", "file": "swarm.yml", "name": "Swarm", "risk": "destructive" }] }));
    with_project(&rig, &engine).await;
    assert_eq!(rig.core.platform().run_grant(engine.clone(), "r3").await, Err(ProvisionError::RiskChanged("swarm".into())));
    assert!(engine.runs.lock().unwrap().is_empty());
    assert!(writes(&rig).await.is_empty());
}

/// Rotura que atrapa: guardar la contraseña de sudo (o un secreto escrito a
/// mano) en el estado que se pinta, mandarla a cac, o dejarla en la salida
/// cuando un playbook la escribe.
#[tokio::test]
async fn the_sudo_password_stays_out_of_the_state_and_the_output() {
    let rig = provision_as("member").await;
    let engine = FakeEngine::with(json!({ "version": 1, "inventory": "hosts.ini", "playbooks": [
        { "id": "tds", "file": "tds.yml", "name": "TDS", "become": true, "vars": [{ "name": "api_token", "secret": true, "required": true }] }
    ] }));
    with_project(&rig, &engine).await;
    rig.core.platform().set_limit("gstd");
    Mock::given(method("POST")).and(path("/api/v1/servers/s/provisioning-runs")).respond_with(ok(json!({ "id": "run1" }))).mount(&rig.server).await;
    Mock::given(method("PATCH")).and(path("/api/v1/servers/s/provisioning-runs/run1")).respond_with(ok(json!(null))).mount(&rig.server).await;
    let inputs = ManualInputs { values: vec![("api_token".into(), "tok-SECRETO".into())], extra: "region=eu".into(), become_password: "sudo-SECRETA".into() };
    rig.core.platform().run_manual(engine.clone(), inputs).await.unwrap();
    assert_eq!(engine.manual.lock().unwrap()[0].0.as_deref(), Some("sudo-SECRETA"), "el motor no recibió la contraseña");
    let output = rig.core.platform().state().provision_output.unwrap();
    let painted: Vec<String> = output.lines.iter().map(|line| line.spans.iter().map(|span| span.text.as_str()).collect()).collect();
    assert_eq!(painted, ["sudo: ***", "token=***"]);
    let everything = format!("{}{:?}", rig.heard(), rig.server.received_requests().await.unwrap().iter().map(|r| String::from_utf8_lossy(&r.body).to_string()).collect::<Vec<_>>());
    assert!(!everything.contains("sudo-SECRETA") && !everything.contains("tok-SECRETO"), "un secreto llegó al estado o a cac");
    let opened: Value = serde_json::from_slice(&writes(&rig).await[0].body).unwrap();
    assert_eq!(opened, json!({ "kind": "playbook", "project": "ansible", "playbook": "tds.yml", "target": "gstd", "varNames": ["api_token", "region"] }));
}
