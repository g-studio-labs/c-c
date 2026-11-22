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
