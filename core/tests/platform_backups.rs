//! Los backups de Plataforma contra un servidor de mentira. Aparte de
//! `platform.rs` para no pisarse con las otras fases, con su propio arnés
//! (el mismo, en pequeño). Cada prueba dice la rotura que atrapa, comprobada
//! rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{BackupStatus, BackupTargetForm, Config, Core, Listener, PlatformState, ServerEditError, ServerEvent, ServerTab, Session, Storage};
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

fn target_row() -> Value {
    json!({ "id": "t1", "orgId": "o1", "serverId": "s", "serviceName": "shop_db", "engine": "postgres", "database": "shop", "credentialEnv": "POSTGRES_USER",
            "username": "", "everyHours": 24, "anchorTime": "03:30", "timezone": "America/Mexico_City", "keepDays": 7, "keepWeekly": 4, "keepMonthly": 6,
            "restoreTestEveryDays": 7, "enabled": true })
}

fn backup_row(id: &str, status: &str) -> Value {
    json!({ "id": id, "createdAt": format!("2026-11-21T10:00:0{}Z", id.len()), "orgId": "o1", "serverId": "s", "targetId": "t1", "kind": "manual", "status": status, "retentionClass": "", "sizeBytes": 0, "error": "" })
}

/// Un servidor `s` abierto en la pestaña de backups, con la persona en ese
/// papel: una base `t1` con un backup bueno en su historial.
async fn backups_as(role: &str) -> Rig {
    let server = MockServer::start().await;
    let recorder = Arc::new(Recorder::default());
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    let rig = Rig { server, core, recorder };

    let org = json!({ "id": "o1", "name": "Org", "slug": "o1", "role": role, "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "Europe/Madrid" });
    get("/api/v1/organizations/").respond_with(ok(json!([org]))).mount(&rig.server).await;
    rig.core.orgs().load().await;
    let vps = json!({ "id": "s", "orgId": "o1", "name": "vps", "host": "127.0.0.1", "sshPort": 22, "sshUser": "deploy", "type": "docker-swarm",
                      "agentPort": rig.server.address().port(), "status": "online", "hasAgentToken": true, "agentVersion": 14 });
    get("/api/v1/servers/").respond_with(ok(json!([vps]))).mount(&rig.server).await;
    rig.core.platform().refresh().await;

    Mock::given(method("POST")).and(path("/api/v1/servers/s/agent-session")).respond_with(ok(json!({ "token": "p", "expiresAt": (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339() }))).mount(&rig.server).await;
    get("/api/v1/services").respond_with(ok(json!([{ "id": "svc", "name": "shop_db", "image": "postgres:16", "stack": "shop" }]))).mount(&rig.server).await;
    get("/api/v1/servers/s/backups/status").respond_with(ok(json!({ "enabled": true, "agentReady": true, "agentVersion": 14, "minAgent": 9, "hasRecipients": true, "everyHours": [6, 24] }))).mount(&rig.server).await;
    get("/api/v1/servers/s/backups/targets").respond_with(ok(json!([target_row()]))).mount(&rig.server).await;
    get("/api/v1/servers/s/backups/recipients").respond_with(ok(json!([{ "id": "r1", "orgId": "o1", "label": "jose", "publicKey": format!("age1{}", "q".repeat(58)) }]))).mount(&rig.server).await;
    get("/api/v1/servers/s/backups/targets/t1/history").respond_with(ok(json!([backup_row("b1", "succeeded")]))).mount(&rig.server).await;

    assert!(rig.core.platform().open_server("s"));
    rig.core.platform().open_tab(ServerTab::Backups).await;
    rig
}

async fn hits(rig: &Rig, route: &str) -> usize {
    rig.server.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == route).count()
}

async fn backup_writes(rig: &Rig) -> Vec<Request> {
    rig.server.received_requests().await.unwrap().into_iter().filter(|r| r.method.as_str() != "GET" && r.url.path().contains("/backups")).collect()
}

fn event(name: &str, data: Value) -> ServerEvent {
    ServerEvent { event: name.into(), data: data.to_string() }
}

/// Rotura que atrapa: volver a pedir el historial por cada evento (un backup
/// que avanza no puede depender de refrescar), duplicar la fila en vez de
/// fundirla, meter el backup de otro servidor, o pegar al log que se mira
/// las líneas de otro backup.
#[tokio::test]
async fn a_status_event_moves_the_row_without_asking_again() {
    let rig = backups_as("viewer").await;
    let backups = rig.core.platform().state().backups.expect("la pestaña trae los backups");
    assert_eq!(backups.status.as_ref().map(|status| status.every_hours.clone()), Some(vec![6, 24]));
    assert_eq!(backups.targets.as_ref().map(|targets| targets[0].database.clone()).as_deref(), Some("shop"));
    assert_eq!(backups.recipients.as_ref().map(Vec::len), Some(1));
    assert_eq!(backups.runs.iter().map(|run| run.id.as_str()).collect::<Vec<_>>(), ["b1"]);
    assert_eq!(rig.core.platform().state().services.map(|services| services.len()), Some(1), "las bases detectadas salen de los servicios");

    rig.core.stream().deliver(event("backup:status", backup_row("b22", "queued")));
    rig.core.stream().deliver(event("backup:status", backup_row("b22", "running")));
    let mut ajeno = backup_row("b333", "running");
    ajeno["serverId"] = json!("otro");
    rig.core.stream().deliver(event("backup:status", ajeno));
    let backups = rig.core.platform().state().backups.unwrap();
    assert_eq!(backups.runs.iter().map(|run| (run.id.as_str(), run.status)).collect::<Vec<_>>(), [("b22", BackupStatus::Running), ("b1", BackupStatus::Succeeded)]);
    assert!(backups.busy("t1"));
    assert_eq!(hits(&rig, "/api/v1/servers/s/backups/targets/t1/history").await, 1);

    get("/api/v1/servers/s/backups/runs/b22").respond_with(ok(json!({ "id": "b22", "log": "dump\n" }))).mount(&rig.server).await;
    rig.core.platform().open_backup_log("b22").await;
    rig.core.stream().deliver(event("backup:log", json!({ "backupId": "b22", "targetId": "t1", "lines": ["encrypt"] })));
    rig.core.stream().deliver(event("backup:log", json!({ "backupId": "b1", "targetId": "t1", "lines": ["ajeno"] })));
    assert_eq!(rig.core.platform().state().backups.unwrap().log, "dump\nencrypt\n");
}

/// Rotura que atrapa: dejar a quien no administra correr, probar,
/// descargar o tocar bases y llaves (el backend contestaría 403, pero la
/// petición ya habría salido); o dejar la URL de descarga en el estado, que
/// se pinta y viaja en cada aviso.
#[tokio::test]
async fn only_an_admin_runs_downloads_and_edits() {
    let member = backups_as("member").await;
    let platform = member.core.platform();
    let form = platform.state().backups.unwrap().targets.unwrap()[0].form();
    assert_eq!(platform.run_backup("t1").await.err(), Some(ServerEditError::AdminOnly));
    assert_eq!(platform.test_backup_restore("t1").await.err(), Some(ServerEditError::AdminOnly));
    assert_eq!(platform.backup_download_url("b1").await.err(), Some(ServerEditError::AdminOnly));
    assert_eq!(platform.save_backup_target(Some("t1"), &form).await.err(), Some(ServerEditError::AdminOnly));
    assert_eq!(platform.delete_backup_target("t1").await.err(), Some(ServerEditError::AdminOnly));
    assert_eq!(platform.add_backup_recipient("ana", &format!("age1{}", "p".repeat(58))).await.err(), Some(ServerEditError::AdminOnly));
    assert_eq!(platform.delete_backup_recipient("r1").await.err(), Some(ServerEditError::AdminOnly));
    assert!(backup_writes(&member).await.is_empty(), "salió una escritura de quien no administra");

    let rig = backups_as("admin").await;
    let platform = rig.core.platform();
    Mock::given(method("POST")).and(path("/api/v1/servers/s/backups/targets/t1/run")).respond_with(ok(backup_row("b22", "queued"))).expect(1).mount(&rig.server).await;
    platform.run_backup("t1").await.unwrap();
    assert!(platform.state().backups.unwrap().busy("t1"), "lo lanzado no sale en el historial");

    let url = "https://bucket.s3.amazonaws.com/o1/s/t1/b1.dump.age?X-Amz-Signature=FIRMA";
    Mock::given(method("POST")).and(path("/api/v1/servers/s/backups/runs/b1/download")).respond_with(ok(json!({ "method": "GET", "url": url, "headers": {} }))).mount(&rig.server).await;
    assert_eq!(platform.backup_download_url("b1").await.unwrap(), url);
    assert!(!format!("{:?}", rig.recorder.platform.lock().unwrap()).contains("FIRMA"), "la URL firmada quedó en el estado");

    let mut edited = form.clone();
    edited.every_hours = 6;
    edited.database = " shop ".into();
    let mut body = serde_json::to_value(&form).unwrap();
    (body["everyHours"], body["database"]) = (json!(6), json!("shop"));
    let mut back = target_row();
    back["everyHours"] = json!(6);
    Mock::given(method("PATCH")).and(path("/api/v1/servers/s/backups/targets/t1")).and(body_json(body)).respond_with(ok(back)).expect(1).mount(&rig.server).await;
    platform.save_backup_target(Some("t1"), &edited).await.unwrap();
    assert_eq!(platform.state().backups.unwrap().targets.unwrap()[0].every_hours, 6);

    Mock::given(method("DELETE")).and(path("/api/v1/servers/s/backups/targets/t1")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&rig.server).await;
    platform.delete_backup_target("t1").await.unwrap();
    let backups = platform.state().backups.unwrap();
    assert!(backups.targets.unwrap().is_empty());
    assert!(backups.runs.is_empty(), "el historial de una base borrada se quedó");
}

/// Rotura que atrapa: mandar al servidor una llave privada pegada en el
/// campo de la pública (sale de la máquina aunque el backend la rechace) o
/// dejarla en el estado; o que una llave buena no se añada ni encienda
/// «hay destinatarios».
#[tokio::test]
async fn a_private_key_pasted_as_recipient_is_never_sent() {
    let rig = backups_as("admin").await;
    let platform = rig.core.platform();
    let private = format!("AGE-SECRET-KEY-1{}", "Q".repeat(58));
    assert_eq!(platform.add_backup_recipient("jose", &private).await.err(), Some(ServerEditError::BadAgeKey));
    assert_eq!(platform.add_backup_recipient("jose", &private.to_lowercase()).await.err(), Some(ServerEditError::BadAgeKey));
    assert!(backup_writes(&rig).await.is_empty(), "la llave privada salió hacia el servidor");
    let painted = format!("{:?}", rig.recorder.platform.lock().unwrap()).to_lowercase();
    assert!(!painted.contains("age-secret-key"), "la llave privada llegó al estado");

    Mock::given(method("DELETE")).and(path("/api/v1/servers/s/backups/recipients/r1")).respond_with(ResponseTemplate::new(204)).mount(&rig.server).await;
    platform.delete_backup_recipient("r1").await.unwrap();
    assert_eq!(platform.state().backups.unwrap().status.unwrap().has_recipients, false);

    let public = format!("age1{}", "p".repeat(58));
    Mock::given(method("POST"))
        .and(path("/api/v1/servers/s/backups/recipients"))
        .and(body_json(json!({ "label": "jose", "publicKey": public })))
        .respond_with(ok(json!({ "id": "r2", "orgId": "o1", "label": "jose", "publicKey": public })))
        .expect(1)
        .mount(&rig.server)
        .await;
    platform.add_backup_recipient(" jose ", &format!("{public}\n")).await.unwrap();
    let backups = platform.state().backups.unwrap();
    assert_eq!(backups.recipients.unwrap().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r2"]);
    assert!(backups.status.unwrap().has_recipients);
}

/// Rotura que atrapa: mandar una base sin servicio, base o variable (el
/// backend contesta un 400 en inglés), o abrir el formulario de una base
/// nueva con otra zona que la de la organización.
#[tokio::test]
async fn an_incomplete_target_is_not_sent() {
    let rig = backups_as("admin").await;
    let platform = rig.core.platform();
    let form = platform.new_backup_form(None);
    assert_eq!(form.timezone, "Europe/Madrid");
    assert_eq!(platform.save_backup_target(None, &BackupTargetForm { database: "shop".into(), ..form }).await.err(), Some(ServerEditError::BackupIncomplete));
    assert!(backup_writes(&rig).await.is_empty());
}
