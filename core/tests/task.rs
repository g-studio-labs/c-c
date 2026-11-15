//! El detalle de una tarea y sus cambios, contra un servidor de mentira. Cada
//! prueba dice la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{
    new_task_body, patch_body, Config, Core, DueChange, Listener, NewTask, Session, Storage, TaskPatch, TaskState,
};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
struct Recorder(Mutex<Vec<TaskState>>);

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn task_changed(&self, state: TaskState) {
        self.0.lock().unwrap().push(state);
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
    let core =
        Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    Rig { server, core, recorder }
}

/// Una tarea como la manda el servidor. `project` con algo y sin `internal`
/// es la de un cliente.
fn detail(id: &str, title: &str, status: &str, project: &str, visibility: &str) -> Value {
    json!({
        "task": {
            "id": id, "createdAt": "2026-11-14T20:00:00Z", "updatedAt": "2026-11-14T20:00:00Z",
            "listId": "l1", "projectId": project, "seq": 12, "title": title, "status": status,
            "priority": "medium", "visibility": visibility, "dueAt": "2026-09-30T00:00:00Z",
        },
        "listName": "tasks", "spaceName": "Espacio",
        "status": { "id": format!("s-{status}"), "listId": "l1", "name": status, "kind": "open", "status": status },
        "tags": null, "assignees": null, "comments": null, "attachments": null,
        "subtasks": [{ "id": "sub1", "seq": 13, "title": "Una línea", "priority": "medium", "statusId": "s-pending" }],
        "blockedBy": null, "decisions": null,
    })
}

fn columns() -> Value {
    json!([
        { "id": "s-pending", "listId": "l1", "name": "Open", "kind": "open", "status": "pending" },
        { "id": "s-in_progress", "listId": "l1", "name": "In progress", "kind": "active", "status": "in_progress" },
        { "id": "s-resolved", "listId": "l1", "name": "Done", "kind": "done", "status": "resolved" },
        { "id": "s-closed", "listId": "l1", "name": "Closed", "kind": "done", "status": "closed" },
    ])
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

async fn mount_task(server: &MockServer, id: &str, body: Value) {
    Mock::given(method("GET")).and(path(format!("/api/v1/tasks/{id}"))).respond_with(ok(body)).mount(server).await;
}

async fn mount_columns(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v1/task-lists/l1/statuses"))
        .respond_with(ok(columns()))
        .mount(server)
        .await;
}

async fn requests_to(server: &MockServer, verb: &str, suffix: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == verb && request.url.path().ends_with(suffix))
        .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
        .collect()
}

fn allowed(state: &TaskState) -> Vec<(String, bool)> {
    state.statuses.iter().map(|option| (option.status.status.clone(), option.allowed)).collect()
}

/// Abrir una tarea trae su detalle, con la prioridad por su nombre de la API y
/// el día del vencimiento ya calculado.
///
/// Roturas que atrapa: no instalar lo que llega; enseñar `medium`, el nombre
/// guardado; dejar el día sin calcular (la pantalla lo leería del instante y
/// se correría un día al oeste de Greenwich).
#[tokio::test]
async fn opening_a_task_brings_it_ready_to_show() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Subir el backend", "pending", "", "")).await;
    mount_columns(&rig.server).await;

    rig.core.tasks().open("t1").await;

    let state = rig.core.tasks().state();
    let shown = state.detail.expect("llegó el detalle");
    assert_eq!(shown.task.title, "Subir el backend");
    assert_eq!(shown.task.priority, "normal");
    assert_eq!(shown.subtasks[0].priority, "normal");
    assert_eq!(shown.task.due_day.as_deref(), Some("2026-09-30"));
    assert!(!state.loading);
    assert_eq!(state.error, None);
}

/// A la tarea de un cliente se le ofrecen sólo los cambios que su máquina deja.
///
/// Rotura que atrapa: ofrecer `pendiente → resuelta` a la de un cliente (el
/// servidor contesta 409 y el botón no hace nada).
#[tokio::test]
async fn a_clients_task_is_offered_only_what_its_machine_allows() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Del cliente", "pending", "p1", "public")).await;
    mount_columns(&rig.server).await;

    rig.core.tasks().open("t1").await;

    assert_eq!(
        allowed(&rig.core.tasks().state()),
        [
            ("pending".to_string(), true),
            ("in_progress".to_string(), true),
            ("resolved".to_string(), false),
            ("closed".to_string(), true),
        ]
    );
}

/// Una tarea interna va de cualquier sitio a cualquier sitio, aunque viva en
/// la lista de un cliente.
///
/// Rotura que atrapa: aplicarle la máquina estricta por estar en un canal.
#[tokio::test]
async fn an_internal_task_in_a_clients_list_can_go_anywhere() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Interna", "pending", "p1", "internal")).await;
    mount_columns(&rig.server).await;

    rig.core.tasks().open("t1").await;

    assert!(allowed(&rig.core.tasks().state()).iter().all(|(_, allowed)| *allowed));
}

/// Un cambio de estado que la máquina no deja no se pide.
///
/// Rotura que atrapa: mandarlo igual y enterarse por el 409.
#[tokio::test]
async fn a_move_the_machine_forbids_is_never_sent() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Del cliente", "pending", "p1", "public")).await;
    mount_columns(&rig.server).await;
    rig.core.tasks().open("t1").await;

    let answer = rig.core.tasks().set_status("s-resolved").await;

    // Dicho como lo diría el servidor, para que se lea la misma frase.
    let phrase = cac_core::error_phrase(cac_core::Locale::Es, &answer.unwrap_err(), None);
    assert_eq!(phrase, "Ese estado no puede ir detrás de éste.");
    assert!(requests_to(&rig.server, "POST", "/move").await.is_empty());
}

/// Mover manda la columna, y el detalle se vuelve a pedir.
///
/// Roturas que atrapa: mandar otra columna; quedarse con el estado de antes en
/// pantalla (la ruta de mover no devuelve la tarea).
#[tokio::test]
async fn moving_sends_the_column_and_brings_the_task_again() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ok(detail("t1", "Tarea", "pending", "", "")))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    mount_columns(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/t1/move"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "Task moved" })))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;
    mount_task(&rig.server, "t1", detail("t1", "Tarea", "in_progress", "", "")).await;

    rig.core.tasks().set_status("s-in_progress").await.expect("se movió");

    assert_eq!(requests_to(&rig.server, "POST", "/move").await, [json!({ "statusId": "s-in_progress" })]);
    assert_eq!(rig.core.tasks().state().detail.unwrap().task.status, "in_progress");
}

/// Un cambio manda sólo lo que se puso.
///
/// Roturas que atrapa: quitar el vencimiento con `dueAt: null` (el servidor lo
/// lee como «no tocar» y la fecha se queda, #95); mandar el día sin su
/// medianoche UTC; mandar `medium`; mandar campos que nadie tocó (un título
/// vacío que pisa el bueno).
#[test]
fn a_change_carries_only_what_was_set() {
    let clear = TaskPatch { due: DueChange::Clear, ..TaskPatch::default() };
    assert_eq!(patch_body(&clear).unwrap(), json!({ "clearDueAt": true }));

    let set = TaskPatch {
        priority: Some("medium".into()),
        due: DueChange::Set { day: "2026-09-30".into() },
        assignee_ids: Some(vec![]),
        ..TaskPatch::default()
    };
    assert_eq!(
        patch_body(&set).unwrap(),
        json!({ "priority": "normal", "dueAt": "2026-09-30T00:00:00Z", "assigneeIds": [] })
    );

    assert_eq!(patch_body(&TaskPatch::default()).unwrap(), json!({}));
    let not_a_day = TaskPatch { due: DueChange::Set { day: "30/09/2026".into() }, ..TaskPatch::default() };
    assert!(patch_body(&not_a_day).is_err());
}

/// Editar la tarea abierta la deja con lo que contestó el servidor.
///
/// Rotura que atrapa: no instalar la respuesta (el título viejo en pantalla
/// hasta reabrir).
#[tokio::test]
async fn editing_shows_what_the_server_answered() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Antes", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/tasks/t1"))
        .and(body_json(json!({ "title": "Después" })))
        .respond_with(ok(detail("t1", "Después", "pending", "", "")))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;

    let patch = TaskPatch { title: Some("Después".into()), ..TaskPatch::default() };
    rig.core.tasks().update(patch).await.expect("se guardó");

    let state = rig.core.tasks().state();
    assert_eq!(state.detail.unwrap().task.title, "Después");
    assert_eq!(state.statuses.len(), 4, "las columnas siguen ahí tras editar");
}

/// La respuesta de una tarea que ya no es la abierta no pisa a la que sí.
///
/// Rotura que atrapa: abrir una tarea, saltar a otra, y que al llegar tarde la
/// primera se quede en pantalla bajo el nombre de la segunda.
#[tokio::test]
async fn a_late_answer_does_not_replace_the_task_that_is_open() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/slow"))
        .respond_with(ok(detail("slow", "La lenta", "pending", "", "")).set_delay(Duration::from_millis(300)))
        .mount(&rig.server)
        .await;
    mount_task(&rig.server, "fast", detail("fast", "La rápida", "pending", "", "")).await;
    mount_columns(&rig.server).await;

    let tasks = rig.core.tasks();
    let slow = tokio::spawn({
        let core = rig.core.clone();
        async move { core.tasks().open("slow").await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    tasks.open("fast").await;
    slow.await.unwrap();

    let state = tasks.state();
    assert_eq!(state.task_id.as_deref(), Some("fast"));
    assert_eq!(state.detail.unwrap().task.title, "La rápida");
}

/// Al abrir otra tarea, la anterior deja de enseñarse ya; al refrescar la
/// abierta, no se vacía.
///
/// Roturas que atrapa: ver el cuerpo de la tarea anterior mientras llega la
/// nueva; que la que se está leyendo parpadee con cada comentario ajeno.
#[tokio::test]
async fn opening_blanks_the_previous_task_and_refreshing_does_not() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Una", "pending", "", "")).await;
    mount_task(&rig.server, "t2", detail("t2", "Otra", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    rig.core.tasks().open("t1").await;

    rig.recorder.0.lock().unwrap().clear();
    rig.core.tasks().open("t2").await;
    let first = rig.recorder.0.lock().unwrap().first().cloned().unwrap();
    assert_eq!((first.task_id.as_deref(), first.detail.is_none(), first.loading), (Some("t2"), true, true));

    rig.recorder.0.lock().unwrap().clear();
    rig.core.tasks().refresh().await;
    let seen = rig.recorder.0.lock().unwrap().clone();
    assert!(!seen.is_empty(), "refrescar avisa");
    assert!(seen.iter().all(|state| state.detail.is_some() && !state.loading));
}

/// Una tarea que no se puede leer lo dice, y no se queda «cargando».
///
/// Rotura que atrapa: el panel girando para siempre ante un 403 o un 404.
#[tokio::test]
async fn a_task_that_cannot_be_read_says_so() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/nope"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "success": false, "error": "task-not-found", "message": "Task not found",
        })))
        .mount(&rig.server)
        .await;

    rig.core.tasks().open("nope").await;

    let state = rig.core.tasks().state();
    assert_eq!(state.error.as_deref(), Some("Task not found"));
    assert!(!state.loading);
    assert!(state.detail.is_none());
}

/// Un comentario lleva su visibilidad sólo cuando alguien la eligió, y el hilo
/// que contesta el servidor es el que se enseña.
///
/// Roturas que atrapa: mandar siempre `internal` o `public` (le quita la
/// decisión al servidor, que en una tarea de cliente es «lo lee»); no enseñar
/// el comentario recién escrito.
#[tokio::test]
async fn a_comment_says_who_reads_it_only_when_someone_chose() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Tarea", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    let mut answered = detail("t1", "Tarea", "pending", "", "");
    answered["comments"] = json!([{ "id": "c1", "body": "hola", "createdAt": "2026-11-15T10:00:00Z" }]);
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/t1/comments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": answered })))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;

    rig.core.tasks().add_comment("hola", None).await.expect("se escribió");
    rig.core.tasks().add_comment("entre nosotros", Some("internal")).await.expect("se escribió");

    assert_eq!(
        requests_to(&rig.server, "POST", "/comments").await,
        [json!({ "body": "hola" }), json!({ "body": "entre nosotros", "visibility": "internal" })]
    );
    assert_eq!(rig.core.tasks().state().detail.unwrap().comments[0].body, "hola");
}

/// Marcar una línea del checklist la lleva a «resuelta» aunque la madre sea de
/// un cliente y esté pendiente.
///
/// Rotura que atrapa: preguntarle a la máquina de la madre, que en el flujo
/// del cliente no deja ir de pendiente a resuelta: el check no haría nada. Es
/// la trampa de «hay dos máquinas» de `CLAUDE.md`.
#[tokio::test]
async fn ticking_a_subtask_does_not_ask_the_parents_machine() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Del cliente", "pending", "p1", "public")).await;
    mount_columns(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/sub1/move"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;
    let tasks = rig.core.tasks();
    let subtask = tasks.state().detail.unwrap().subtasks[0].clone();
    assert!(!tasks.subtask_is_done(&subtask));

    tasks.set_subtask_done("sub1", true).await.expect("se marcó");
    tasks.set_subtask_done("sub1", false).await.expect("se desmarcó");

    assert_eq!(
        requests_to(&rig.server, "POST", "/sub1/move").await,
        [json!({ "statusId": "s-resolved" }), json!({ "statusId": "s-pending" })]
    );
}

/// Una línea nueva del checklist nace en la lista de su madre y colgando de
/// ella.
///
/// Rotura que atrapa: crearla como tarea suelta (sin `parentId`), o en otra
/// lista.
#[tokio::test]
async fn a_new_subtask_hangs_from_the_open_task() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Madre", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/task-lists/l1/tasks"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": {
            "id": "sub2", "createdAt": "2026-11-15T10:00:00Z", "updatedAt": "2026-11-15T10:00:00Z",
            "listId": "l1", "seq": 14, "title": "Otra línea", "status": "pending", "priority": "none",
        }})))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;

    rig.core.tasks().add_subtask("Otra línea").await.expect("se creó");

    assert_eq!(
        requests_to(&rig.server, "POST", "/l1/tasks").await,
        [json!({ "title": "Otra línea", "parentId": "t1" })]
    );
}

/// Una tarea nueva lleva todo lo que se eligió en la fila, de una vez.
///
/// Roturas que atrapa: mandar el vencimiento como día suelto (el servidor
/// espera un instante); mandar `medium`; mandar una visibilidad que nadie
/// eligió.
#[test]
fn a_new_task_carries_what_was_chosen() {
    let task = NewTask {
        title: "  Tarea nueva ".into(),
        priority: Some("medium".into()),
        due_day: Some("2026-12-01".into()),
        assignee_ids: vec!["u1".into()],
        ..NewTask::default()
    };
    assert_eq!(
        new_task_body(&task).unwrap(),
        json!({ "title": "Tarea nueva", "priority": "normal", "dueAt": "2026-12-01T00:00:00Z", "assigneeIds": ["u1"] })
    );
}

/// Si la tarea cambia de lista, sus columnas son las de la nueva.
///
/// Rotura que atrapa: seguir ofreciendo las columnas de la lista anterior, que
/// en la nueva no existen.
#[tokio::test]
async fn a_task_moved_to_another_list_gets_that_lists_columns() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Tarea", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    let mut moved = detail("t1", "Tarea", "pending", "", "");
    moved["task"]["listId"] = json!("l2");
    Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(moved)).mount(&rig.server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/task-lists/l2/statuses"))
        .respond_with(ok(json!([{ "id": "x1", "listId": "l2", "name": "Por hacer", "kind": "open", "status": "pending" }])))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;
    assert_eq!(rig.core.tasks().state().statuses.len(), 4);

    let patch = TaskPatch { list_id: Some("l2".into()), ..TaskPatch::default() };
    rig.core.tasks().update(patch).await.expect("se movió de lista");

    let names: Vec<String> =
        rig.core.tasks().state().statuses.into_iter().map(|option| option.status.name).collect();
    assert_eq!(names, ["Por hacer"]);
}
