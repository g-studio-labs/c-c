//! Las reglas de «Mi trabajo», contra un servidor de mentira.
//!
//! Vienen de `app/src/store/mywork.store.ts`; cada prueba dice la rotura que
//! atrapa, y se ha comprobado rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{
    day_and_month, due_label, work_groups, Config, Core, DueState, Listener, Locale, MyWorkState, OpenTask, ScopeKind,
    Session, Storage, TaskFilter, WorkLens, WorkScope,
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

// ── La lista ────────────────────────────────────────────────────────────

fn open_task(id: &str, space: &str, edit: impl FnOnce(&mut serde_json::Value)) -> OpenTask {
    let mut raw = task(id, "l1", space);
    raw["spaceName"] = serde_json::json!(format!("Espacio {space}"));
    edit(&mut raw);
    serde_json::from_value(raw).unwrap()
}

fn day(year: i32, month: u32, day: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// La lista se agrupa por espacio en el orden del árbol, y lo que el árbol
/// no trae va al final.
///
/// Roturas que atrapa: ordenar por orden de llegada (el panel y la lista
/// dejarían de leerse igual); perder las tareas de un espacio que el árbol no
/// enseña; mezclar tareas de dos espacios; barajar las de dentro de un grupo.
#[test]
fn the_list_is_grouped_by_space_in_the_trees_order() {
    let tasks = vec![
        open_task("t1", "c", |_| {}),
        open_task("t2", "ghost", |_| {}),
        open_task("t3", "a", |_| {}),
        open_task("t4", "c", |_| {}),
        open_task("t5", "other-ghost", |_| {}),
    ];
    let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];

    let groups = work_groups(&tasks, &order, Locale::Es, day(2026, 9, 30));

    let shape: Vec<(&str, Vec<&str>)> =
        groups.iter().map(|group| (group.space_id.as_str(), group.rows.iter().map(|row| row.id.as_str()).collect())).collect();
    assert_eq!(
        shape,
        [("a", vec!["t3"]), ("c", vec!["t1", "t4"]), ("ghost", vec!["t2"]), ("other-ghost", vec!["t5"])]
    );
    assert_eq!(groups[1].name, "Espacio c");
}

/// Cada fila trae su vencimiento ya dicho, su prioridad con el nombre de la
/// API y de qué tarea es subtarea.
///
/// Roturas que atrapa: «medium» donde la pantalla espera «normal» (la
/// etiqueta de prioridad saldría vacía); una subtarea que parece una tarea
/// suelta; decir «30 sept» de lo que vence hoy.
#[test]
fn a_row_says_its_due_date_priority_and_parent() {
    let tasks = vec![
        open_task("hoy", "a", |_| {}),
        open_task("sub", "a", |raw| {
            raw["parentSeq"] = serde_json::json!(289);
            raw["parentTitle"] = serde_json::json!("La madre");
            raw["dueAt"] = serde_json::Value::Null;
            raw["priority"] = serde_json::json!("urgent");
        }),
        open_task("sub-sin-titulo", "a", |raw| raw["parentSeq"] = serde_json::json!(7)),
    ];

    let groups = work_groups(&tasks, &[], Locale::Es, day(2026, 9, 30));
    let rows = &groups[0].rows;

    assert_eq!(rows[0].priority, "normal");
    assert_eq!(rows[0].parent, None);
    let due = rows[0].due.as_ref().unwrap();
    assert_eq!((due.text.as_str(), due.state), ("hoy", DueState::Today));
    assert_eq!(rows[1].priority, "urgent");
    assert_eq!(rows[1].parent.as_deref(), Some("› #289 La madre"));
    assert_eq!(rows[1].due, None);
    assert_eq!(rows[2].parent.as_deref(), Some("› #7"));
}

/// Un vencimiento se dice con una palabra cuando está cerca y con su día y
/// su mes cuando no, en el idioma de quien lee.
///
/// Roturas que atrapa: «30 sept» en inglés o «Sep 30» en castellano; el mes
/// corrido uno (enero por febrero); «ayer» para lo de hace una semana.
#[test]
fn a_due_date_is_said_in_words_or_as_day_and_month() {
    let due = "2026-09-30T00:00:00Z";
    let said = |locale, today| due_label(locale, Some(due), today).map(|label| (label.text, label.state)).unwrap();

    assert_eq!(said(Locale::Es, day(2026, 9, 30)), ("hoy".into(), DueState::Today));
    assert_eq!(said(Locale::Es, day(2026, 10, 1)), ("ayer".into(), DueState::Yesterday));
    assert_eq!(said(Locale::Es, day(2026, 9, 29)), ("mañana".into(), DueState::Tomorrow));
    assert_eq!(said(Locale::Es, day(2026, 10, 8)), ("30 sept".into(), DueState::Overdue));
    assert_eq!(said(Locale::Es, day(2026, 9, 1)), ("30 sept".into(), DueState::Later));
    assert_eq!(said(Locale::En, day(2026, 9, 1)), ("Sep 30".into(), DueState::Later));
    assert_eq!(said(Locale::En, day(2026, 9, 30)), ("today".into(), DueState::Today));
    assert_eq!(due_label(Locale::Es, None, day(2026, 9, 30)), None);

    assert_eq!(day_and_month(Locale::Es, day(2026, 1, 5)), "5 ene");
    assert_eq!(day_and_month(Locale::En, day(2026, 12, 25)), "Dec 25");
}

/// Lo que la lista pinta respeta el ámbito y el filtro, con el «hoy» de la
/// organización.
///
/// Roturas que atrapa: agrupar todo lo cargado ignorando a qué se acotó;
/// ignorar lo escrito en el buscador.
#[tokio::test]
async fn what_the_list_paints_respects_scope_and_filter() {
    let mut other = task("t2", "l2", "s2");
    other["title"] = serde_json::json!("Revisión del diseño");
    let rig = rig(vec![task("t1", "l1", "s1"), other]).await;
    rig.core.my_work().load(Some("o1".into())).await;
    let my_work = rig.core.my_work();

    assert_eq!(my_work.groups(&TaskFilter::default(), Locale::Es).len(), 2);

    // En el orden del árbol que hay cargado, no en el de llegada.
    let spaces = serde_json::json!([
        { "id": "s2", "orgId": "o1", "name": "Dos", "color": "", "folders": [], "lists": [], "people": [] },
        { "id": "s1", "orgId": "o1", "name": "Uno", "color": "", "folders": [], "lists": [], "people": [] },
    ]);
    Mock::given(method("GET"))
        .and(path("/api/v1/task-spaces/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": spaces })))
        .mount(&rig.server)
        .await;
    rig.core.tree().load("o1").await;
    let order: Vec<String> =
        my_work.groups(&TaskFilter::default(), Locale::Es).into_iter().map(|group| group.space_id).collect();
    assert_eq!(order, ["s2", "s1"]);

    my_work.set_scope(Some(scope(ScopeKind::Space, "s2", "o1")));
    let groups = my_work.groups(&TaskFilter::default(), Locale::Es);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].rows[0].id, "t2");

    my_work.set_scope(None);
    let filter = TaskFilter { query: "revision".into(), ..TaskFilter::default() };
    let groups = my_work.groups(&filter, Locale::Es);
    assert_eq!(groups.iter().map(|group| group.rows.len()).sum::<usize>(), 1);
}

/// Cambiar de lente o pedir lo terminado trae las tareas de esa pregunta, y
/// volver a lo mismo no pregunta dos veces.
///
/// Roturas que atrapa: cambiar la pestaña y seguir enseñando las tareas de la
/// anterior; pedir la lista entera en cada clic aunque no cambie nada.
#[tokio::test]
async fn changing_lens_or_asking_for_closed_brings_that_question() {
    let rig = rig(vec![task("t1", "l1", "s1")]).await;

    rig.core.show_lens(WorkLens::Watching).await;
    rig.core.show_lens(WorkLens::Watching).await;
    rig.core.show_closed(true).await;

    let asked = queries(&rig.server).await;
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(asked[0].contains("watcher=me") && !asked[0].contains("status=all"));
    assert!(asked[1].contains("watcher=me") && asked[1].contains("status=all"));
    assert_eq!(rig.core.my_work().state().tasks.len(), 1);
}
