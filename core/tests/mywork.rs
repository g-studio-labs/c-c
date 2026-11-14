//! Las reglas de «Mi trabajo», contra un servidor de mentira.
//!
//! Vienen de `app/src/store/mywork.store.ts`; cada prueba dice la rotura que
//! atrapa, y se ha comprobado rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{
    Config, Core, Listener, MyWorkState, ScopeKind, Session, Storage, TaskFilter, WorkLens, WorkScope,
};
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

/// Apunta cada estado que el core anuncia.
#[derive(Default)]
struct Recorder(Mutex<Vec<MyWorkState>>);

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn my_work_changed(&self, state: MyWorkState) {
        self.0.lock().unwrap().push(state);
    }
}

fn task(id: &str, list: &str, space: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "seq": 1, "title": format!("Tarea {id}"), "priority": "medium",
        "dueAt": "2026-09-30T00:00:00Z",
        "status": "open", "statusName": "Open", "statusKind": "open",
        "listId": list, "listName": "Lista", "spaceId": space, "spaceName": "Espacio",
        "updatedAt": "2026-11-14T00:00:00Z", "subtasksDone": 0, "subtasksTotal": 0,
    })
}

struct Rig {
    server: MockServer,
    core: Core,
    storage: Arc<MemoryStorage>,
    recorder: Arc<Recorder>,
}

/// Un core con sesión y un servidor que contesta `/api/v1/tasks/` con `tasks`.
async fn rig(tasks: Vec<serde_json::Value>) -> Rig {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": tasks })))
        .mount(&server)
        .await;
    let storage = Arc::new(MemoryStorage::default());
    let recorder = Arc::new(Recorder::default());
    let core = Core::new(Config::new(server.uri()), storage.clone(), recorder.clone()).unwrap();
    let ana = serde_json::from_value(serde_json::json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    Rig { server, core, storage, recorder }
}

/// Las consultas que recibió `/api/v1/tasks/`, en orden.
async fn queries(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request: &&Request| request.url.path() == "/api/v1/tasks/")
        .map(|request| request.url.query().unwrap_or("").to_string())
        .collect()
}

fn scope(kind: ScopeKind, id: &str, org: &str) -> WorkScope {
    WorkScope { kind, id: id.into(), name: "Nombre".into(), org_id: Some(org.into()) }
}

/// Cada lente pregunta lo suyo, y todas menos la de clientes piden también lo
/// que entró por un canal de cliente.
///
/// Rotura que atrapa: quitar `origin=any`, que deja fuera de «asignadas a mí»
/// un ticket asignado a mí porque lo levantó un cliente.
#[tokio::test]
async fn every_lens_asks_its_own_question() {
    let rig = rig(vec![]).await;
    let work = rig.core.my_work();
    for lens in [WorkLens::Assigned, WorkLens::Created, WorkLens::Watching, WorkLens::Clients, WorkLens::All] {
        work.set_lens(lens);
        work.load(Some("org1".into())).await;
    }

    assert_eq!(
        queries(&rig.server).await,
        [
            "orgId=org1&assignee=me&origin=any&limit=200",
            "orgId=org1&creator=me&origin=any&limit=200",
            "orgId=org1&watcher=me&origin=any&limit=200",
            "orgId=org1&origin=clients&limit=200",
            "orgId=org1&origin=any&limit=200",
        ]
    );
}

/// «Incluir cerradas» lo pide al servidor.
///
/// Rotura que atrapa: no mandar `status=all`.
#[tokio::test]
async fn including_closed_asks_for_every_status() {
    let rig = rig(vec![]).await;
    let work = rig.core.my_work();
    work.set_include_closed(true);
    work.load(None).await;

    assert_eq!(queries(&rig.server).await, ["assignee=me&origin=any&status=all&limit=200"]);
}

/// Cada tarea sale con su día de vencimiento ya resuelto.
///
/// Rotura que atrapa: dejar que la pantalla lea `dueAt`, que al oeste de
/// Greenwich es el día anterior.
#[tokio::test]
async fn a_task_arrives_with_its_due_day() {
    let rig = rig(vec![task("t1", "l1", "s1")]).await;
    let work = rig.core.my_work();
    work.load(None).await;

    assert_eq!(work.state().tasks[0].due_day.as_deref(), Some("2026-09-30"));
}

/// Un ámbito de otra organización se tira al cargar.
///
/// Rotura que atrapa: no tirarlo (la pantalla sale vacía y parece que no hay
/// trabajo), o tirar también el de la organización que se está cargando (los
/// dos clics para filtrar).
#[tokio::test]
async fn a_scope_from_another_organization_is_dropped() {
    let rig = rig(vec![]).await;
    let work = rig.core.my_work();

    work.set_scope(Some(scope(ScopeKind::List, "l1", "org1")));
    work.load(Some("org1".into())).await;
    assert!(work.state().scope.is_some(), "el de esta organización se queda");

    work.load(Some("org2".into())).await;
    assert_eq!(work.state().scope, None);
}

/// El ámbito acota lo que se pinta, por lista o por espacio.
///
/// Rotura que atrapa: comparar la lista con el espacio, o no acotar.
#[tokio::test]
async fn a_scope_narrows_what_is_shown() {
    let rig = rig(vec![task("a", "l1", "s1"), task("b", "l2", "s1"), task("c", "l3", "s2")]).await;
    let work = rig.core.my_work();
    work.load(Some("org1".into())).await;
    let ids = |tasks: Vec<cac_core::OpenTask>| tasks.into_iter().map(|t| t.id).collect::<Vec<_>>();

    assert_eq!(ids(work.visible()), ["a", "b", "c"]);
    work.set_scope(Some(scope(ScopeKind::List, "l2", "org1")));
    assert_eq!(ids(work.visible()), ["b"]);
    work.set_scope(Some(scope(ScopeKind::Space, "s1", "org1")));
    assert_eq!(ids(work.visible()), ["a", "b"]);

    let filter = TaskFilter { query: "tarea b".into(), ..TaskFilter::default() };
    assert_eq!(ids(work.filtered(&filter)), ["b"]);
}

/// Volver a la pantalla no vuelve a pedir lo que ya responde a la pregunta.
///
/// Rotura que atrapa: cargar siempre; o no cargar cuando cambió la lente.
#[tokio::test]
async fn coming_back_does_not_ask_again() {
    let rig = rig(vec![]).await;
    let work = rig.core.my_work();

    work.ensure_loaded(Some("org1".into())).await;
    work.ensure_loaded(Some("org1".into())).await;
    assert_eq!(queries(&rig.server).await.len(), 1);

    work.set_lens(WorkLens::Created);
    work.ensure_loaded(Some("org1".into())).await;
    assert_eq!(queries(&rig.server).await.len(), 2);

    work.ensure_loaded(Some("org2".into())).await;
    assert_eq!(queries(&rig.server).await.len(), 3);
}

/// Una carga que falló lo dice, y al volver se intenta otra vez aunque la
/// pregunta sea la misma que ya se contestó bien antes.
///
/// Es la recarga tras un evento la que falla: la clave de la última carga
/// buena sigue casando. Rotura que atrapa: dar por buena una carga fallida;
/// callar el fallo.
#[tokio::test]
async fn a_failed_load_says_so_and_is_tried_again() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"success":true,"data":[]}"#))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/"))
        .respond_with(ResponseTemplate::new(500).set_body_string(r#"{"success":false,"error":"internal","message":"Something broke"}"#))
        .mount(&server)
        .await;
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(Recorder::default())).unwrap();
    let work = core.my_work();

    work.ensure_loaded(None).await;
    assert_eq!(work.state().error, None);

    work.load(None).await;
    assert_eq!(work.state().error.as_deref(), Some("Something broke"));
    assert!(!work.state().loading);

    work.ensure_loaded(None).await;
    assert_eq!(queries(&server).await.len(), 3, "la buena, la que falló y el reintento");
}

/// Dejar de seguir quita la fila en la lente «siguiendo», y sólo en ésa.
///
/// Rotura que atrapa: no quitarla (sigue en una lista que ya no la contiene),
/// o quitarla en cualquier lente.
#[tokio::test]
async fn unfollowing_takes_the_row_away_only_where_it_no_longer_belongs() {
    let rig = rig(vec![task("a", "l1", "s1"), task("b", "l1", "s1")]).await;
    Mock::given(method("DELETE")).and(path("/api/v1/tasks/a/watch")).respond_with(ResponseTemplate::new(200)).expect(2).mount(&rig.server).await;
    let work = rig.core.my_work();

    work.load(None).await;
    work.set_watching("a", false).await.unwrap();
    assert_eq!(work.state().tasks.len(), 2, "en «asignadas» se queda");

    work.set_lens(WorkLens::Watching);
    work.set_watching("a", false).await.unwrap();
    assert_eq!(work.state().tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b"]);
}

/// Seguir es un POST; y un rechazo del servidor llega como error.
///
/// Rotura que atrapa: mandar siempre DELETE; tragarse el rechazo.
#[tokio::test]
async fn following_posts_and_a_refusal_is_an_error() {
    let rig = rig(vec![]).await;
    Mock::given(method("POST")).and(path("/api/v1/tasks/a/watch")).respond_with(ResponseTemplate::new(200)).expect(1).mount(&rig.server).await;
    Mock::given(method("POST")).and(path("/api/v1/tasks/x/watch")).respond_with(ResponseTemplate::new(404).set_body_string(r#"{"success":false,"error":"not-found","message":"Task not found"}"#)).mount(&rig.server).await;
    let work = rig.core.my_work();

    work.set_watching("a", true).await.unwrap();
    assert!(work.set_watching("x", true).await.is_err());
}

/// Un vencimiento se cambia aquí antes de que conteste el servidor, con su día,
/// y devuelve el anterior para deshacer.
///
/// Rotura que atrapa: no devolver el anterior; no recalcular el día.
#[tokio::test]
async fn a_due_date_moves_here_first_and_can_be_undone() {
    let rig = rig(vec![task("a", "l1", "s1")]).await;
    let work = rig.core.my_work();
    work.load(None).await;

    let previous = work.set_due_local("a", Some("2026-10-02T00:00:00Z".into()));

    assert_eq!(previous.as_deref(), Some("2026-09-30T00:00:00Z"));
    assert_eq!(work.state().tasks[0].due_day.as_deref(), Some("2026-10-02"));
    work.set_due_local("a", None);
    assert_eq!(work.state().tasks[0].due_day, None);
}

/// Cinco eventos seguidos de una tarea que tenemos son una sola recarga.
///
/// Rotura que atrapa: quitar el respiro (cinco peticiones para llegar al
/// mismo sitio).
#[tokio::test]
async fn a_burst_of_events_is_one_reload() {
    let rig = rig(vec![task("a", "l1", "s1")]).await;
    let work = rig.core.my_work();
    work.load(Some("org1".into())).await;

    for _ in 0..5 {
        work.task_event("a", false);
    }
    tokio::time::sleep(Duration::from_millis(700)).await;

    assert_eq!(queries(&rig.server).await.len(), 2, "la carga inicial y una recarga");
}

/// Un evento de una tarea que no tenemos no recarga; uno de una nueva, sí, pero
/// sólo si ya se había cargado algo.
///
/// Rotura que atrapa: recargar por cada evento de la organización; no
/// enterarse nunca de una tarea nueva.
#[tokio::test]
async fn only_our_tasks_and_new_ones_reload() {
    let rig = rig(vec![task("a", "l1", "s1")]).await;
    let work = rig.core.my_work();

    work.task_event("nueva", true);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(queries(&rig.server).await.len(), 0, "sin nada cargado no hay lista que poner al día");

    work.load(None).await;
    work.task_event("otra", false);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(queries(&rig.server).await.len(), 1, "una que no es nuestra no recarga");

    work.task_event("nueva", true);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(queries(&rig.server).await.len(), 2, "una nueva sí");
}

/// La lente y el interruptor sobreviven a cerrar la app; el ámbito y las tareas no.
///
/// Rotura que atrapa: no guardar la preferencia; guardar también el ámbito
/// (abrir la app acotada a una lista que ya no recuerdas haber elegido).
#[tokio::test]
async fn the_lens_is_remembered_and_the_scope_is_not() {
    let rig = rig(vec![task("a", "l1", "s1")]).await;
    let work = rig.core.my_work();
    let reopen = || {
        Core::new(Config::new(rig.server.uri()), rig.storage.clone(), Arc::new(Recorder::default()))
            .unwrap()
            .my_work()
            .state()
    };

    // Cada preferencia se guarda al cambiarla, no al cambiar la otra.
    work.set_lens(WorkLens::Watching);
    assert_eq!(reopen().lens, WorkLens::Watching);
    work.set_include_closed(true);
    work.set_scope(Some(scope(ScopeKind::List, "l1", "org1")));
    work.load(Some("org1".into())).await;

    let state = reopen();

    assert_eq!(state.lens, WorkLens::Watching);
    assert!(state.include_closed);
    assert_eq!(state.scope, None);
    assert!(state.tasks.is_empty());
}

/// Cada cambio se anuncia con el estado entero.
///
/// Rotura que atrapa: cambiar sin avisar (la pantalla se queda con lo viejo).
#[tokio::test]
async fn every_change_is_announced() {
    let rig = rig(vec![task("a", "l1", "s1")]).await;
    let work = rig.core.my_work();

    work.load(None).await;
    work.forget("a");

    let seen = rig.recorder.0.lock().unwrap();
    assert!(seen.first().unwrap().loading, "primero dice que está cargando");
    assert_eq!(seen[1].tasks.len(), 1);
    assert!(seen.last().unwrap().tasks.is_empty());
}
