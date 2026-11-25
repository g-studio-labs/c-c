//! «Resumen» contra un servidor de mentira. Cada prueba dice la rotura que
//! atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{Config, Core, NoListener, ServerEvent, Storage};
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

struct Rig {
    server: MockServer,
    core: Core,
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn get(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path(route.to_string()))
}

fn org(id: &str) -> Value {
    json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" })
}

fn report(id: &str, status: &str) -> Value {
    json!({ "id": id, "title": format!("r-{id}"), "folio": "", "projectName": "Portal", "status": status, "priority": "high" })
}

fn task(id: &str) -> Value {
    json!({
        "id": id, "seq": 1, "title": format!("t-{id}"), "priority": "normal", "status": "in_progress", "statusName": "En curso", "statusKind": "active",
        "listId": "l1", "listName": "Sprint", "spaceId": "s1", "spaceName": "App", "updatedAt": "2026-11-25T10:00:00Z"
    })
}

/// Dos organizaciones; la que se mira es `o1`.
async fn rig() -> Rig {
    let server = MockServer::start().await;
    get("/api/v1/organizations/").respond_with(ok(json!([org("o1"), org("o2")]))).mount(&server).await;
    get("/api/v1/servers/").respond_with(ok(json!([]))).mount(&server).await;
    get("/api/v1/reports/")
        .and(query_param("orgId", "o1"))
        .respond_with(ok(json!({ "items": [report("r1", "open"), report("r2", "done"), report("r3", "resolved"), report("r4", "closed"), report("r5", "in_progress")], "total": 5 })))
        .mount(&server)
        .await;
    get("/api/v1/tasks/").and(query_param("orgId", "o1")).respond_with(ok(json!([task("t1"), task("t2")]))).mount(&server).await;
    let core = Core::new(Config::new(front(&server.uri())), Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    core.sign_in(serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap(), "access".into(), "refresh".into()).unwrap();
    core.orgs().load().await;
    Rig { server, core }
}

async fn asked(server: &MockServer, route: &str) -> Vec<String> {
    server.received_requests().await.unwrap().iter().filter(|r| r.url.path() == route).map(|r| r.url.query().unwrap_or_default().to_string()).collect()
}

/// Lo que se pide y lo que se cuenta.
///
/// Roturas que atrapa: pedir sin `orgId` (el servidor contesta con todas tus
/// organizaciones y la tarjeta se sale del selector); contar como abierto un
/// reporte hecho o cerrado; no entender el nombre viejo `resolved`.
#[tokio::test]
async fn it_asks_for_this_organization_and_counts_only_open_reports() {
    let rig = rig().await;
    rig.core.overview().load().await;

    let state = rig.core.overview().state();
    assert_eq!(state.reports.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r1", "r5"]);
    assert_eq!(state.tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t1", "t2"]);
    assert_eq!(asked(&rig.server, "/api/v1/reports/").await, ["limit=50&orgId=o1"]);
    assert_eq!(asked(&rig.server, "/api/v1/tasks/").await, ["limit=50&orgId=o1"]);
    assert_eq!(asked(&rig.server, "/api/v1/servers/").await.len(), 1, "los servidores también se leen");
}

/// Lo que llega de otra organización no se pinta.
///
/// Roturas que atrapa: poner lo que contestó el servidor sin mirar si la
/// organización sigue siendo la misma (al volver a ella saldría lo de una
/// lectura vieja, o lo de otra organización en la nueva).
#[tokio::test]
async fn an_answer_that_arrives_after_switching_organization_is_dropped() {
    let rig = rig().await;
    let gate = Gate::default();
    get("/api/v1/tasks/")
        .and(query_param("orgId", "o1"))
        .respond_with(gate.hold(ok(json!([task("viejo")]))))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&rig.server)
        .await;
    let overview = rig.core.overview();
    let reading = tokio::spawn(async move { overview.load().await });
    until("la lectura de o1, retenida", || gate.held() == 1).await;

    assert!(rig.core.orgs().set_current("o2"));
    gate.open();
    reading.await.unwrap();
    assert!(rig.core.orgs().set_current("o1"));
    let state = rig.core.overview().state();
    assert!(state.tasks.is_empty() && state.reports.is_empty(), "{state:?}");
}

/// Un solo módulo a la vez dentro de «Trabajo».
///
/// Roturas que atrapa: dejar «Por atender» o comunicación delante al abrir el
/// resumen; dejar el resumen delante al abrir lo otro.
#[tokio::test]
async fn it_shares_the_screen_with_the_others() {
    let rig = rig().await;
    rig.core.comms().enter();
    assert!(rig.core.overview().show());
    assert!(rig.core.overview().state().on_screen && !rig.core.comms().state().on_screen);

    assert!(rig.core.requests().show(None));
    assert!(!rig.core.overview().state().on_screen && rig.core.requests().state().on_screen);

    assert!(rig.core.overview().show());
    assert!(!rig.core.requests().state().on_screen);
    rig.core.comms().enter();
    assert!(!rig.core.overview().state().on_screen, "comunicación lo deja atrás");

    rig.core.overview().show();
    rig.core.platform().enter();
    assert!(!rig.core.overview().state().on_screen, "Plataforma también");

    rig.core.overview().show();
    rig.core.show_all_work();
    assert!(!rig.core.overview().state().on_screen, "«Mi trabajo» también");
}

/// Mirando el resumen, lo que cambia fuera lo cambia a él (qué eventos lo
/// hacen, y que sin estar delante no, lo prueba `rereads_for` en el core).
///
/// Roturas que atrapa: no releer estando delante; una lectura por cada evento
/// de una ráfaga.
#[tokio::test]
async fn it_rereads_on_task_events_while_in_front() {
    let rig = rig().await;
    rig.core.overview().show();
    rig.core.stream().deliver(ServerEvent { event: "report:new".into(), data: "{}".into() });
    rig.core.stream().deliver(ServerEvent { event: "task:new".into(), data: json!({ "taskId": "t9" }).to_string() });
    until("se vuelve a leer", || rig.core.overview().state().reports.len() == 2).await;
    assert_eq!(asked(&rig.server, "/api/v1/reports/").await.len(), 1, "una ráfaga es una lectura");
}
