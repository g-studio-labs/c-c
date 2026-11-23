//! Actividad, Diagnóstico y Usuarios de Plataforma contra un servidor de
//! mentira. Aparte de `platform.rs` para no pisarse con las otras fases, con
//! su propio arnés (el mismo, en pequeño). Cada prueba dice la rotura que
//! atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{ActivityFilter, Config, Core, Listener, PlatformSection, PlatformState, ServerEvent, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
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
struct Recorder;

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn platform_changed(&self, _state: PlatformState) {}
}

struct Rig {
    server: MockServer,
    core: Core,
}

/// Ana dentro, de la organización `o1`; superadmin si se pide.
async fn rig(superadmin: bool) -> Rig {
    let server = MockServer::start().await;
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(Recorder)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "superadmin": superadmin })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    let org = json!({ "id": "o1", "name": "Org", "slug": "o1", "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" });
    get("/api/v1/organizations/").respond_with(ok(json!([org]))).mount(&server).await;
    core.orgs().load().await;
    Rig { server, core }
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn get(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path(route.to_string()))
}

async fn hits(rig: &Rig, route: &str) -> Vec<Request> {
    rig.server.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == route).collect()
}

fn event(name: &str, data: Value) -> ServerEvent {
    ServerEvent { event: name.into(), data: data.to_string() }
}

// ─── Actividad ───────────────────────────────────────────────────────────────

fn run_row(id: &str, org: &str, repo: &str, status: &str, at: &str) -> Value {
    json!({ "id": id, "orgId": org, "repoFullName": repo, "runId": id.len() as i64 * 100, "runAttempt": 1, "runNumber": 4,
            "workflowName": "CI", "path": ".github/workflows/ci.yml", "event": "push", "status": status, "conclusion": "",
            "headSha": "abcdef1234567", "headBranch": "main", "commitTitle": "fix", "actor": "ana", "htmlUrl": "https://github.com/x", "occurredAt": at })
}

const ACTIVITY: &str = "/api/v1/organizations/o1/activity/";

fn ids(rig: &Rig) -> Vec<String> {
    rig.core.platform().state().activity.unwrap().entries.iter().map(|entry| entry.id().to_string()).collect()
}

/// Un run nuevo de la organización entra en su sitio sin volver a pedir la
/// página, y uno que avanza cambia en la misma fila. Lo de otra
/// organización, o lo que el filtro deja fuera, no entra.
///
/// Roturas que atrapa: no escuchar `ci:run` (la lista sólo cambiaría al
/// recargar); recargar por cada evento; colar el run de otra organización o
/// de otro repo con la página filtrada; duplicar el run que avanza.
#[tokio::test]
async fn a_new_ci_run_enters_without_reloading() {
    let rig = rig(false).await;
    let page = json!({ "items": [{ "kind": "run", "at": "2026-11-20T10:00:00Z", "run": run_row("r1", "o1", "guz/cac", "completed", "2026-11-20T10:00:00Z") }], "hasMore": false });
    get(ACTIVITY).respond_with(ok(page)).mount(&rig.server).await;
    rig.core.platform().open_activity(ActivityFilter::default()).await;
    assert_eq!(rig.core.platform().state().section, PlatformSection::Activity);
    assert_eq!(ids(&rig), ["r1"]);

    rig.core.stream().deliver(event("ci:run", run_row("r22", "o1", "guz/cac", "in_progress", "2026-11-20T11:00:00Z")));
    rig.core.stream().deliver(event("ci:run", run_row("r333", "o2", "guz/cac", "queued", "2026-11-20T12:00:00Z")));
    assert_eq!(ids(&rig), ["r22", "r1"]);
    rig.core.stream().deliver(event("ci:run", run_row("r22", "o1", "guz/cac", "completed", "2026-11-20T11:00:00Z")));
    let state = rig.core.platform().state().activity.unwrap();
    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.entries[0].run.as_ref().unwrap().status, "completed");
    assert_eq!(hits(&rig, ACTIVITY).await.len(), 1, "volvió a pedir la página");

    rig.core.platform().open_activity(ActivityFilter { repo: "guz/otro".into(), ..ActivityFilter::default() }).await;
    let before = ids(&rig);
    rig.core.stream().deliver(event("ci:run", run_row("r4444", "o1", "guz/cac", "queued", "2026-11-20T13:00:00Z")));
    assert_eq!(ids(&rig), before, "el run de otro repo entró en la página filtrada");
}

/// El filtro va al servidor, y quitarlo pide la organización entera.
///
/// Roturas que atrapa: filtrar sólo lo que llega en vivo y pedir la
/// organización entera; quedarse con el filtro viejo al abrir sin él.
#[tokio::test]
async fn the_activity_filter_is_asked_to_the_server() {
    let rig = rig(false).await;
    get(ACTIVITY).respond_with(ok(json!({ "items": [], "hasMore": false }))).mount(&rig.server).await;
    rig.core.platform().open_activity(ActivityFilter { deployable_id: "d1".into(), ..ActivityFilter::default() }).await;
    rig.core.platform().open_activity(ActivityFilter::default()).await;
    let queries: Vec<String> = hits(&rig, ACTIVITY).await.iter().map(|r| r.url.query().unwrap_or_default().to_string()).collect();
    assert_eq!(queries, ["deployableId=d1&limit=50", "limit=50"]);
}

/// Un despliegue que dispara un run se cuelga de él, y su estado avanza en
/// vivo.
///
/// Rotura que atrapa: no escuchar `deploy:status` en la actividad, o no
/// colgar el despliegue de su run.
#[tokio::test]
async fn a_deployment_event_hangs_on_its_run() {
    let rig = rig(false).await;
    let page = json!({ "items": [{ "kind": "run", "at": "2026-11-20T10:00:00Z", "run": run_row("r1", "o1", "guz/cac", "completed", "2026-11-20T10:00:00Z"),
                                   "deployments": [{ "id": "x1", "deployableId": "d1", "serverId": "s1", "status": "running", "orgId": "o1", "deployableName": "web", "deployableEnv": "prod", "workflowRunId": "r1", "createdAt": "2026-11-20T10:05:00Z" }] }],
                       "hasMore": false });
    get(ACTIVITY).respond_with(ok(page)).mount(&rig.server).await;
    rig.core.platform().open_activity(ActivityFilter::default()).await;
    rig.core.stream().deliver(event("deploy:status", json!({ "id": "x1", "deployableId": "d1", "serverId": "s1", "status": "succeeded", "orgId": "o1", "workflowRunId": "r1", "createdAt": "2026-11-20T10:05:00Z" })));
    let state = rig.core.platform().state().activity.unwrap();
    let hung = &state.entries.iter().find(|entry| entry.is_run()).unwrap().deployments[0];
    assert_eq!((hung.deployment.status, hung.deployable_name.as_str()), (cac_core::DeployStatus::Succeeded, "web"));
}

// ─── Diagnóstico ─────────────────────────────────────────────────────────────

/// Elegir un dispositivo trae su ficha y su timeline; filtrar por gravedad
/// vuelve a pedir con el filtro, y buscar texto en los eventos no.
///
/// Roturas que atrapa: pedir la ficha de otro dispositivo (sólo por id, sin
/// el proyecto); no volver a pedir al cambiar la gravedad; ir al servidor por
/// cada letra del texto, que se filtra en pantalla.
#[tokio::test]
async fn picking_a_device_brings_its_sheet_and_its_timeline() {
    let rig = rig(false).await;
    let device = json!({ "deviceId": "dev-1", "projectId": "p1", "projectName": "Rastreo", "label": "moto g54", "platform": "android", "appVersion": "2.3.0",
                         "batches": 3, "reqCount": 10, "errorCount": 1, "warnCount": 0, "lastSeen": "2026-11-20T10:00:00Z", "alerts": null });
    get("/api/v1/telemetry/devices").respond_with(ok(json!([device]))).mount(&rig.server).await;
    let mut detail = device.clone();
    detail["firstSeen"] = json!("2026-11-01T10:00:00Z");
    detail["device"] = json!({ "battery": { "level": 4 } });
    detail["heartbeats"] = json!(["2026-11-20T09:00:00Z"]);
    detail["gaps"] = json!(null);
    get("/api/v1/telemetry/devices/p1/dev-1").respond_with(ok(detail)).mount(&rig.server).await;
    let batch = json!({ "id": "b1", "sessionId": "s1", "receivedAt": "2026-11-20T10:00:00Z", "breadcrumbs": [{ "type": "error", "message": "boom" }], "severities": ["error"] });
    get("/api/v1/telemetry/timeline").respond_with(ok(json!([batch]))).mount(&rig.server).await;

    rig.core.platform().open_diagnostics().await;
    let state = rig.core.platform().state();
    assert_eq!(state.section, PlatformSection::Diagnostics);
    let devices = state.diagnostics.unwrap().devices;
    assert_eq!(devices[0].title(), "moto g54");

    rig.core.platform().select_device(devices[0].reference()).await;
    let state = rig.core.platform().state().diagnostics.unwrap();
    assert_eq!(state.detail.as_ref().unwrap().device, r#"{"battery":{"level":4}}"#);
    assert_eq!(state.timeline.len(), 1);

    let mut filters = state.filters.clone();
    filters.text = "boo".into();
    rig.core.platform().set_timeline_filters(filters.clone()).await;
    filters.min_severity = "error".into();
    rig.core.platform().set_timeline_filters(filters).await;
    let queries: Vec<String> = hits(&rig, "/api/v1/telemetry/timeline").await.iter().map(|r| r.url.query().unwrap_or_default().to_string()).collect();
    assert_eq!(queries, ["deviceId=dev-1&projectId=p1&limit=100", "deviceId=dev-1&projectId=p1&limit=100&minSeverity=error"]);
}
