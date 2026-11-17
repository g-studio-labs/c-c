//! El detalle de una tarea y sus cambios, contra un servidor de mentira. Cada
//! prueba dice la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{
    new_task_body, patch_body, Config, Core, DueChange, Listener, NewTask, Session, Storage, TaskPatch, TaskState,
};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, method, path, query_param};
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

/// Comentar manda «interno» sólo donde un cliente lo leería y alguien lo
/// pidió; y un comentario en blanco no se manda.
///
/// Roturas que atrapa: una nota para el equipo que acaba en el tablero del
/// cliente; marcar como interno lo de una tarea que nunca sale del equipo;
/// mandar espacios como si fueran un comentario.
#[tokio::test]
async fn saying_something_keeps_it_internal_only_where_a_client_would_read_it() {
    let rig = rig().await;
    for (id, project, visibility) in [("client", "p1", "public"), ("team", "", "")] {
        mount_task(&rig.server, id, detail(id, "Tarea", "pending", project, visibility)).await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/tasks/{id}/comments")))
            .respond_with(ResponseTemplate::new(201).set_body_json(
                json!({ "success": true, "data": detail(id, "Tarea", "pending", project, visibility) }),
            ))
            .mount(&rig.server)
            .await;
    }
    mount_columns(&rig.server).await;

    rig.core.tasks().open("client").await;
    assert!(rig.core.tasks().sheet(cac_core::Locale::Es).unwrap().client_reads);
    rig.core.tasks().say("  para el cliente  ", false).await.expect("se escribió");
    rig.core.tasks().say("entre nosotros", true).await.expect("se escribió");
    assert!(rig.core.tasks().say("   ", false).await.is_err());

    rig.core.tasks().open("team").await;
    assert!(!rig.core.tasks().sheet(cac_core::Locale::Es).unwrap().client_reads);
    rig.core.tasks().say("del equipo", true).await.expect("se escribió");

    assert_eq!(
        requests_to(&rig.server, "POST", "/comments").await,
        [
            json!({ "body": "para el cliente" }),
            json!({ "body": "entre nosotros", "visibility": "internal" }),
            json!({ "body": "del equipo" }),
        ]
    );
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

/// Una tarea de la organización `o1`, con una etiqueta y un responsable ya
/// puestos, y lo que su organización ofrece: otra etiqueta y otra persona.
async fn rig_with_choices() -> Rig {
    let rig = rig().await;
    let mut task = detail("t1", "Tarea", "pending", "", "");
    task["task"]["orgId"] = json!("o1");
    task["tags"] = json!([{ "id": "tag-old", "orgId": "o9", "name": "Vieja", "color": "#f00" }]);
    task["assignees"] = json!([{ "id": "u1", "username": "ana", "name": "Ana Pérez" }]);
    mount_task(&rig.server, "t1", task).await;
    mount_columns(&rig.server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/task-tags/"))
        .and(query_param("orgId", "o1"))
        .respond_with(ok(json!([{ "id": "tag-new", "orgId": "o1", "name": "backend", "color": "#0f0" }])))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/search"))
        .and(query_param("orgId", "o1"))
        // Sin el tope, el servidor contesta sólo diez.
        .and(query_param("limit", "50"))
        .respond_with(ok(json!([
            { "id": "u1", "username": "ana", "name": "Ana Pérez" },
            { "id": "u2", "username": "luis" },
            { "id": "u3", "username": "zoe", "name": "  " },
        ])))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;
    rig
}

fn said(choices: Vec<cac_core::Choice>) -> Vec<(String, String, bool)> {
    choices.into_iter().map(|choice| (choice.id, choice.name, choice.chosen)).collect()
}

/// Abrir una tarea trae lo que se le puede poner: las etiquetas de su
/// organización y su equipo, con lo que ya tiene marcado.
///
/// Roturas que atrapa: pedir las etiquetas o el equipo de otra organización
/// (o de todas); ofrecer dos veces a quien ya la lleva; dejar fuera lo que
/// tiene y nadie ofrece, que entonces no habría cómo quitarlo; marcar lo que
/// no tiene; llamar a alguien por su usuario teniendo nombre.
#[tokio::test]
async fn opening_a_task_brings_what_can_be_put_on_it() {
    let rig = rig_with_choices().await;
    let tasks = rig.core.tasks();

    assert_eq!(
        said(tasks.tag_choices()),
        [("tag-new".into(), "backend".into(), false), ("tag-old".into(), "Vieja".into(), true)]
    );
    assert_eq!(
        said(tasks.assignee_choices()),
        [
            ("u1".into(), "Ana Pérez".into(), true),
            ("u2".into(), "luis".into(), false),
            // Un nombre en blanco no es un nombre.
            ("u3".into(), "zoe".into(), false),
        ]
    );
}

/// Una tarea que no dice de qué organización es no pide etiquetas ni equipo.
///
/// Rotura que atrapa: preguntar con la organización vacía, que el servidor
/// lee como «las de todas las mías» y ofrecería etiquetas de otro cliente.
#[tokio::test]
async fn a_task_without_an_organization_asks_for_no_choices() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Tarea", "pending", "", "")).await;
    mount_columns(&rig.server).await;

    rig.core.tasks().open("t1").await;

    let asked: Vec<String> =
        rig.server.received_requests().await.unwrap().iter().map(|request| request.url.path().to_string()).collect();
    assert_eq!(asked, ["/api/v1/tasks/t1", "/api/v1/task-lists/l1/statuses"]);
}

/// Las etiquetas de una tarea que ya no es la abierta no se le ofrecen a la
/// que sí.
///
/// Rotura que atrapa: abrir una tarea de un cliente, saltar a otra, y que al
/// llegar tarde las etiquetas del primero se ofrezcan en la del segundo.
#[tokio::test]
async fn late_choices_are_not_offered_on_another_task() {
    let rig = rig().await;
    let mut first = detail("t1", "Primera", "pending", "", "");
    first["task"]["orgId"] = json!("o1");
    mount_task(&rig.server, "t1", first).await;
    mount_task(&rig.server, "t2", detail("t2", "Segunda", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/task-tags/"))
        .respond_with(
            ok(json!([{ "id": "tag1", "orgId": "o1", "name": "del otro", "color": "#0f0" }]))
                .set_delay(Duration::from_millis(250)),
        )
        .mount(&rig.server)
        .await;

    let slow = rig.core.clone();
    let opening = tokio::spawn(async move { slow.tasks().open("t1").await });
    tokio::time::sleep(Duration::from_millis(120)).await;
    rig.core.tasks().open("t2").await;
    opening.await.unwrap();

    let state = rig.core.tasks().state();
    assert_eq!(state.task_id.as_deref(), Some("t2"));
    assert!(state.tags.is_empty(), "ofrece {:?}", state.tags);
}

/// Poner o quitar una etiqueta o un responsable manda la lista entera, que
/// es lo que el servidor espera.
///
/// Roturas que atrapa: mandar sólo lo nuevo (el servidor quitaría lo que
/// había); no quitar al pulsar lo que ya estaba; cruzar etiquetas con
/// responsables.
#[tokio::test]
async fn a_touch_sends_the_whole_list() {
    let rig = rig_with_choices().await;
    let answer = rig.core.tasks().state().detail.unwrap();
    Mock::given(method("PATCH"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ok(serde_json::to_value(&answer).unwrap()))
        .mount(&rig.server)
        .await;
    let tasks = rig.core.tasks();

    tasks.toggle_tag("tag-new").await.expect("se puso");
    tasks.toggle_tag("tag-old").await.expect("se quitó");
    tasks.toggle_assignee("u2").await.expect("se le dio");
    tasks.toggle_assignee("u1").await.expect("se le quitó");

    assert_eq!(
        requests_to(&rig.server, "PATCH", "/tasks/t1").await,
        [
            json!({ "tagIds": ["tag-old", "tag-new"] }),
            json!({ "tagIds": [] }),
            json!({ "assigneeIds": ["u1", "u2"] }),
            json!({ "assigneeIds": [] }),
        ]
    );
}

/// La prioridad y el vencimiento se mandan cuando cambian, y sólo entonces.
///
/// Roturas que atrapa: guardar otra vez lo que ya tenía (un selector de
/// fecha que avisa al recibir su propio valor guardaría en bucle); tomar
/// `medium` y `normal` por prioridades distintas; quitar la fecha con un
/// `dueAt` vacío en vez de `clearDueAt`.
#[tokio::test]
async fn priority_and_due_are_sent_only_when_they_change() {
    let rig = rig_with_choices().await;
    let answer = rig.core.tasks().state().detail.unwrap();
    Mock::given(method("PATCH"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ok(serde_json::to_value(&answer).unwrap()))
        .mount(&rig.server)
        .await;
    let tasks = rig.core.tasks();

    tasks.set_priority("normal").await.expect("ya la tenía");
    tasks.set_priority("medium").await.expect("ya la tenía, con su otro nombre");
    tasks.set_due(Some("2026-09-30")).await.expect("ya vencía ese día");
    assert!(requests_to(&rig.server, "PATCH", "/tasks/t1").await.is_empty());

    tasks.set_priority("high").await.expect("se guardó");
    tasks.set_due(Some("2026-10-02")).await.expect("se guardó");
    tasks.set_due(None).await.expect("se quitó");

    assert_eq!(
        requests_to(&rig.server, "PATCH", "/tasks/t1").await,
        [json!({ "priority": "high" }), json!({ "dueAt": "2026-10-02T00:00:00Z" }), json!({ "clearDueAt": true })]
    );
}

/// Sin tarea abierta no hay nada que cambiar ni que ofrecer.
///
/// Rotura que atrapa: mandar un cambio a ninguna tarea.
#[tokio::test]
async fn nothing_is_changed_without_an_open_task() {
    let rig = rig().await;
    let tasks = rig.core.tasks();

    assert!(tasks.set_priority("high").await.is_err() && tasks.set_due(None).await.is_err());
    assert!(tasks.set_title("x").await.is_err() && tasks.set_description("x").await.is_err());
    assert!(tasks.toggle_tag("x").await.is_err() && tasks.toggle_assignee("x").await.is_err());
    assert!(tasks.tag_choices().is_empty() && tasks.assignee_choices().is_empty());
    assert!(rig.server.received_requests().await.unwrap().is_empty());
}

/// El título y la descripción se mandan limpios, y sólo si cambian; un
/// título en blanco no se manda nunca.
///
/// Roturas que atrapa: dejar una tarea sin título; guardar lo que ya decía;
/// no poder quitar la descripción; mandar el título como descripción.
#[tokio::test]
async fn title_and_description_are_sent_clean_and_only_when_they_change() {
    let rig = rig().await;
    let mut task = detail("t1", "Tarea", "pending", "", "");
    task["task"]["description"] = json!("Lo que hay.\n");
    mount_task(&rig.server, "t1", task.clone()).await;
    mount_columns(&rig.server).await;
    Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(task)).mount(&rig.server).await;
    let tasks = rig.core.tasks();
    tasks.open("t1").await;

    assert!(tasks.set_title("   ").await.is_err());
    tasks.set_title(" Tarea ").await.expect("es el que tiene");
    tasks.set_description("Lo que hay.").await.expect("es la que tiene");
    assert!(requests_to(&rig.server, "PATCH", "/tasks/t1").await.is_empty());

    tasks.set_title("  Otra  ").await.expect("se guardó");
    tasks.set_description(" Nueva\n").await.expect("se guardó");
    tasks.set_description("  ").await.expect("se quitó");
    // Un título pegado con saltos de línea se guarda en un renglón; y uno
    // que sólo son saltos está en blanco.
    assert!(tasks.set_title("\n\r\n").await.is_err());
    tasks.set_title("Pegado de\nun correo\r\ncon saltos\n").await.expect("se guardó");

    assert_eq!(
        requests_to(&rig.server, "PATCH", "/tasks/t1").await,
        [
            json!({ "title": "Otra" }),
            json!({ "description": "Nueva" }),
            json!({ "description": "" }),
            json!({ "title": "Pegado de un correo con saltos" })
        ]
    );
}

/// Lo escrito en el campo del título: qué se guarda y qué no.
///
/// Roturas que atrapa: guardar un título partido en renglones; mandar el
/// que ya tiene (o uno en blanco) cada vez que se sale del campo; que un
/// retorno de carro de Windows deje dos espacios.
#[test]
fn what_was_typed_in_the_title_is_saved_on_one_line_or_not_at_all() {
    assert_eq!(cac_core::title_line("uno\ndos\r\ntres\rcuatro"), "uno dos tres cuatro");
    assert_eq!(cac_core::retitled("  Otra\ncosa \n", "Tarea").as_deref(), Some("Otra cosa"));
    assert_eq!(cac_core::retitled(" Tarea ", "Tarea"), None);
    assert_eq!(cac_core::retitled(" \n ", "Tarea"), None);
}

/// Editar un comentario manda lo escrito, limpio, y vuelve a pedir la tarea;
/// en blanco o igual que estaba no se pide nada. Borrarlo lo pide por su id.
///
/// Roturas que atrapa: dejar un comentario en blanco en vez de borrarlo;
/// marcar «editado» un comentario que no cambió; editar o borrar otro
/// comentario; que el hilo siga enseñando lo de antes; callar una edición
/// porque **otro** comentario ya decía eso; no ofrecer editar a nadie por no
/// mirar quién está dentro.
#[tokio::test]
async fn a_comment_is_edited_clean_and_only_when_it_changes() {
    let rig = rig().await;
    let mut task = detail("t1", "Tarea", "pending", "", "");
    // Dos comentarios, y el primero no es el que se edita: lo que se compara
    // es lo que dice **ése**. El de Ana es `u1`, quien está dentro.
    task["comments"] = json!([
        { "id": "c0", "body": "hola a todos", "createdAt": "2026-11-15T09:00:00Z", "authorUserId": "u7" },
        { "id": "c1", "body": "hola ", "createdAt": "2026-11-15T10:00:00Z", "authorUserId": "u1" },
    ]);
    mount_task(&rig.server, "t1", task).await;
    mount_columns(&rig.server).await;
    for verb in ["PATCH", "DELETE"] {
        Mock::given(method(verb))
            .and(path("/api/v1/tasks/t1/comments/c1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "ok" })))
            .mount(&rig.server)
            .await;
    }
    rig.core.tasks().open("t1").await;
    let fetched = |requests: Vec<Value>| requests.len();
    // Quién puede cambiarlo sale de quien está dentro.
    let mine: Vec<bool> = rig.core.tasks().sheet(cac_core::Locale::Es).unwrap().talk.iter().map(|comment| comment.mine).collect();
    assert_eq!(mine, [false, true]);

    let blank = rig.core.tasks().edit_comment("c1", "   ").await.unwrap_err();
    assert!(matches!(blank, cac_core::ApiError::Server { ref code, .. } if code == "empty-comment"), "{blank:?}");
    rig.core.tasks().edit_comment("c1", " hola").await.unwrap();
    assert!(requests_to(&rig.server, "PATCH", "/comments/c1").await.is_empty(), "pidió sin nada que cambiar");

    rig.core.tasks().edit_comment("c1", "  hola a todos \n").await.unwrap();
    assert_eq!(requests_to(&rig.server, "PATCH", "/comments/c1").await, [json!({ "body": "hola a todos" })]);
    assert_eq!(fetched(requests_to(&rig.server, "GET", "/tasks/t1").await), 2, "no volvió a pedir la tarea");

    rig.core.tasks().delete_comment("c1").await.unwrap();
    assert_eq!(requests_to(&rig.server, "DELETE", "/comments/c1").await.len(), 1);
    assert_eq!(fetched(requests_to(&rig.server, "GET", "/tasks/t1").await), 3, "no volvió a pedir la tarea");
}

fn refusal(result: Result<(), cac_core::ApiError>) -> String {
    match result {
        Err(cac_core::ApiError::Server { code, .. }) => code,
        other => format!("{other:?}"),
    }
}

/// `rig_with_choices`, con un servidor que crea la etiqueta que se le pida
/// (la contesta como `tag-made`) y guarda los cambios de la tarea.
async fn rig_that_makes_tags(made: &str) -> Rig {
    let rig = rig_with_choices().await;
    let answer = rig.core.tasks().state().detail.unwrap();
    Mock::given(method("PATCH"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ok(serde_json::to_value(&answer).unwrap()))
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/task-tags/"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "success": true,
            "data": { "id": "tag-made", "orgId": "o1", "name": made, "color": "#06B6D4" },
        })))
        .mount(&rig.server)
        .await;
    rig
}

/// Una etiqueta que no existe se crea en la organización de la tarea, con
/// su color, y se le pone; desde entonces se ofrece.
///
/// Roturas que atrapa: crearla en otra organización, o sin decir en cuál;
/// crearla y no ponérsela; ponérsela quitando las que tenía; mandar el
/// nombre con los espacios de alrededor; que no salga en la lista hasta
/// volver a abrir la tarea.
#[tokio::test]
async fn a_tag_that_does_not_exist_is_made_in_the_tasks_organization() {
    let rig = rig_that_makes_tags("Diseño").await;
    let tasks = rig.core.tasks();

    tasks.add_tag_named("  Diseño ").await.expect("se creó y se puso");

    assert_eq!(
        requests_to(&rig.server, "POST", "/task-tags/").await,
        [json!({ "orgId": "o1", "name": "Diseño", "color": "#06B6D4" })]
    );
    assert_eq!(requests_to(&rig.server, "PATCH", "/tasks/t1").await, [json!({ "tagIds": ["tag-old", "tag-made"] })]);
    let offered: Vec<String> = tasks.tag_choices().into_iter().map(|choice| choice.id).collect();
    assert!(offered.contains(&"tag-made".to_string()), "no se ofrece: {offered:?}");
}

/// Con el nombre de una etiqueta que ya hay no se crea otra: se pone ésa; y
/// si la tarea ya la tiene, no se pide nada.
///
/// Roturas que atrapa: dos «backend» en la lista, que no se distinguen;
/// tomar «BACKEND» o «Diseno» por otra etiqueta; guardar otra vez la tarea
/// por una etiqueta que ya tenía (o ponérsela dos veces).
#[tokio::test]
async fn a_tag_that_exists_is_put_and_not_made_again() {
    let rig = rig_that_makes_tags("x").await;
    let tasks = rig.core.tasks();

    tasks.add_tag_named(" BÁCKEND").await.expect("se puso la que había");
    assert_eq!(requests_to(&rig.server, "PATCH", "/tasks/t1").await, [json!({ "tagIds": ["tag-old", "tag-new"] })]);

    // «Vieja» ya la tiene, aunque sea de otra organización y nadie la ofrezca.
    tasks.add_tag_named("vieja").await.expect("ya la tenía");
    assert_eq!(requests_to(&rig.server, "PATCH", "/tasks/t1").await.len(), 1);
    assert!(requests_to(&rig.server, "POST", "/task-tags/").await.is_empty(), "creó una que ya existía");
}

/// Una etiqueta sin nombre, con uno que el servidor no admite, o para una
/// tarea que no dice de qué organización es, no se pide.
///
/// Roturas que atrapa: crear una etiqueta en blanco; mandar un nombre de más
/// de sesenta letras para que el servidor conteste un error que nadie
/// entiende (o contar bytes, y rechazar uno de sesenta con acentos); crear
/// la etiqueta con la organización vacía.
#[tokio::test]
async fn a_tag_needs_a_name_and_an_organization() {
    let made = rig_that_makes_tags("x").await;
    let tasks = made.core.tasks();
    assert_eq!(refusal(tasks.add_tag_named("   ").await), "empty-tag");
    assert_eq!(refusal(tasks.add_tag_named(&"ñ".repeat(61)).await), "tag-too-long");
    assert!(requests_to(&made.server, "POST", "/task-tags/").await.is_empty());
    tasks.add_tag_named(&"ñ".repeat(60)).await.expect("sesenta letras caben, aunque sean más bytes");
    assert_eq!(requests_to(&made.server, "POST", "/task-tags/").await.len(), 1);

    // Otra tarea, que no dice su organización.
    let bare = rig().await;
    mount_task(&bare.server, "t1", detail("t1", "Tarea", "pending", "", "")).await;
    mount_columns(&bare.server).await;
    Mock::given(method("POST")).and(path("/api/v1/task-tags/")).respond_with(ok(json!({ "id": "x", "name": "x" }))).mount(&bare.server).await;
    bare.core.tasks().open("t1").await;
    assert_eq!(refusal(bare.core.tasks().add_tag_named("Diseño").await), "no-organization");
    assert!(requests_to(&bare.server, "POST", "/task-tags/").await.is_empty(), "pidió crear una etiqueta sin organización");

    bare.core.tasks().close();
    assert_eq!(refusal(bare.core.tasks().add_tag_named("Diseño").await), "no-task-open");
}

/// Una etiqueta nueva nace del color que le daría la app de siempre.
///
/// Roturas que atrapa: todas las etiquetas del mismo color; la misma
/// etiqueta de otro color según desde qué app se cree (los valores son los
/// de `paletteFor`, sacados con node); un color de fuera de la paleta;
/// reventar con un nombre cuya cuenta desborda o sale negativa.
#[test]
fn a_new_tag_gets_the_colour_the_other_app_would_give_it() {
    use cac_core::{tag_color_for, TAG_PALETTE};
    for (name, colour) in [
        ("backend", "#EAB308"),
        ("Diseño", "#06B6D4"),
        ("urgente", "#8B5CF6"),
        ("🚀 lanzamiento", "#EC4899"),
        ("cliente importante", "#64748B"),
        ("bug", "#8B5CF6"),
        ("a", "#EC4899"),
        ("", "#3B82F6"),
    ] {
        assert_eq!(tag_color_for(name), colour, "{name:?}");
        assert!(TAG_PALETTE.contains(&tag_color_for(name)));
    }
}

/// Los `PATCH` que llegaron a la tarea `t1`.
async fn patches(server: &MockServer) -> Vec<Value> {
    requests_to(server, "PATCH", "/tasks/t1").await
}

/// La tarea abierta, tal como quedó en el estado.
fn open_task(rig: &Rig) -> cac_core::TaskDetail {
    rig.core.tasks().state().detail.expect("hay tarea abierta")
}

/// Escribir dónde quedó manda el texto limpio, y lo que se enseña después
/// (el texto, quién y cuándo) es lo que contestó el servidor.
///
/// Roturas que atrapa: mandarlo en otro campo, o con los blancos de los
/// lados; firmarlo aquí en vez de leer la firma de la respuesta; no poner en
/// pantalla lo que el servidor contestó.
#[tokio::test]
async fn writing_where_it_was_left_shows_what_the_server_signed() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Tarea", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    let mut signed = detail("t1", "Tarea", "pending", "", "");
    signed["task"]["handoff"] = json!("Falta desplegar.");
    signed["task"]["handoffAt"] = json!("2026-11-17T10:00:00Z");
    signed["task"]["handoffById"] = json!("u1");
    signed["handoffBy"] = json!("Ana Ruiz");
    Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(signed)).mount(&rig.server).await;
    rig.core.tasks().open("t1").await;
    assert_eq!(open_task(&rig).task.handoff, None);

    rig.core.tasks().set_handoff("  Falta desplegar.\n").await.expect("se guardó");

    assert_eq!(patches(&rig.server).await, [json!({ "handoff": "Falta desplegar." })]);
    let now = open_task(&rig);
    assert_eq!(now.task.handoff.as_deref(), Some("Falta desplegar."));
    assert_eq!((now.handoff_by.as_deref(), now.task.handoff_at.as_deref()), (Some("Ana Ruiz"), Some("2026-11-17T10:00:00Z")));
}

/// En blanco lo quita: se manda vacío, que es como el servidor lo borra.
///
/// Roturas que atrapa: no mandar nada al vaciarlo (no habría cómo quitarlo);
/// tratar «sólo espacios» como un texto.
#[tokio::test]
async fn an_empty_handoff_clears_it() {
    let rig = rig().await;
    let mut written = detail("t1", "Tarea", "pending", "", "");
    written["task"]["handoff"] = json!("Lo de antes.");
    mount_task(&rig.server, "t1", written).await;
    mount_columns(&rig.server).await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ok(detail("t1", "Tarea", "pending", "", "")))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;

    rig.core.tasks().set_handoff("   \n ").await.expect("se quitó");

    assert_eq!(patches(&rig.server).await, [json!({ "handoff": "" })]);
    assert_eq!(open_task(&rig).task.handoff, None);
}

/// Lo que no cambia nada, o no cabe, no se le pide al servidor.
///
/// Roturas que atrapa: volver a firmar con la fecha de hoy un texto que nadie
/// tocó; mandar uno de más de 2000 letras, que el servidor rechaza con un 400
/// sin frase; contar bytes en vez de letras (2000 eñes caben).
#[tokio::test]
async fn a_handoff_that_says_the_same_or_does_not_fit_is_not_sent() {
    let rig = rig().await;
    let mut written = detail("t1", "Tarea", "pending", "", "");
    written["task"]["handoff"] = json!("Lo de antes.");
    mount_task(&rig.server, "t1", written.clone()).await;
    mount_columns(&rig.server).await;
    Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(written)).mount(&rig.server).await;
    rig.core.tasks().open("t1").await;

    rig.core.tasks().set_handoff(" Lo de antes. ").await.expect("no hay nada que guardar");
    assert_eq!(refusal(rig.core.tasks().set_handoff(&"ñ".repeat(2001)).await), "handoff-too-long");
    assert_eq!(patches(&rig.server).await, Vec::<Value>::new());

    rig.core.tasks().set_handoff(&"ñ".repeat(2000)).await.expect("2000 letras caben");
    assert_eq!(patches(&rig.server).await.len(), 1);
}

/// Si el servidor no lo guarda, se dice y la tarea sigue diciendo lo de antes.
///
/// Roturas que atrapa: dar por guardado lo que no se guardó; poner en la
/// tarea el texto que se quería sin que el servidor lo aceptara.
#[tokio::test]
async fn a_handoff_the_server_refuses_leaves_the_task_as_it_was() {
    let rig = rig().await;
    let mut written = detail("t1", "Tarea", "pending", "", "");
    written["task"]["handoff"] = json!("Lo de antes.");
    mount_task(&rig.server, "t1", written).await;
    mount_columns(&rig.server).await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "success": false, "message": "Forbidden", "error": "insufficient-role" })))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;

    assert_eq!(refusal(rig.core.tasks().set_handoff("Otra cosa.").await), "insufficient-role");
    assert_eq!(open_task(&rig).task.handoff.as_deref(), Some("Lo de antes."));
}

/// Un servidor que crea la subtarea que se le pida en la lista `l1`.
async fn mount_subtask_maker(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/v1/task-lists/l1/tasks"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": {
            "id": "sub2", "createdAt": "2026-11-15T10:00:00Z", "updatedAt": "2026-11-15T10:00:00Z",
            "listId": "l1", "seq": 14, "title": "Otra línea", "status": "pending", "priority": "none",
        }})))
        .mount(server)
        .await;
}

/// La subtarea nueva sale en la tarea abierta, con su número, y la de un
/// cliente no manda nada que la haga suya: va el título en un renglón y de
/// quién cuelga, y nada más.
///
/// Roturas que atrapa: crearla y no volver a pedir la madre (no aparece
/// hasta reabrir); mandarle la visibilidad o el proyecto de la madre (una
/// subtarea es interna siempre: gastaría un folio del cliente en una línea
/// de checklist); un título pegado con saltos de línea, o con sus blancos.
#[tokio::test]
async fn a_new_subtask_shows_in_the_open_task_with_its_number() {
    let rig = rig().await;
    let mother = detail("t1", "Del cliente", "pending", "p1", "public");
    Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(mother.clone())).up_to_n_times(1).mount(&rig.server).await;
    mount_columns(&rig.server).await;
    mount_subtask_maker(&rig.server).await;
    rig.core.tasks().open("t1").await;
    let mut grown = mother;
    grown["subtasks"] = json!([
        { "id": "sub1", "seq": 13, "title": "Una línea", "priority": "medium", "statusId": "s-pending" },
        { "id": "sub2", "seq": 14, "title": "Otra línea", "priority": "none", "statusId": "s-pending" },
    ]);
    mount_task(&rig.server, "t1", grown).await;

    rig.core.tasks().add_subtask("  Otra\nlínea ").await.expect("se creó");

    assert_eq!(requests_to(&rig.server, "POST", "/l1/tasks").await, [json!({ "title": "Otra línea", "parentId": "t1" })]);
    let listed: Vec<(String, i64)> = open_task(&rig).subtasks.into_iter().map(|card| (card.id, card.seq)).collect();
    assert_eq!(listed, [("sub1".to_string(), 13), ("sub2".to_string(), 14)]);
}

/// Sin título, o con uno que no cabe, no se le pide nada al servidor.
///
/// Roturas que atrapa: crear una subtarea sin nombre (o sólo de espacios),
/// que en la lista es una casilla suelta; mandar un título de más de 300
/// letras, que el servidor rechaza con un 400 sin frase.
#[tokio::test]
async fn a_subtask_without_a_title_or_with_one_too_long_is_not_sent() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Madre", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    mount_subtask_maker(&rig.server).await;
    rig.core.tasks().open("t1").await;

    assert_eq!(refusal(rig.core.tasks().add_subtask("  \n ").await), "empty-title");
    assert_eq!(refusal(rig.core.tasks().add_subtask(&"ñ".repeat(301)).await), "title-too-long");
    assert_eq!(requests_to(&rig.server, "POST", "/l1/tasks").await, Vec::<Value>::new());

    rig.core.tasks().add_subtask(&"ñ".repeat(300)).await.expect("300 letras caben");
    assert_eq!(requests_to(&rig.server, "POST", "/l1/tasks").await.len(), 1);
}

/// Si el servidor no la crea, se dice y la lista sigue como estaba.
///
/// Rotura que atrapa: dar por creada una subtarea que no existe.
#[tokio::test]
async fn a_subtask_the_server_refuses_is_not_listed() {
    let rig = rig().await;
    mount_task(&rig.server, "t1", detail("t1", "Madre", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/task-lists/l1/tasks"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "success": false, "message": "Forbidden", "error": "insufficient-role" })))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t1").await;

    assert_eq!(refusal(rig.core.tasks().add_subtask("Otra línea").await), "insufficient-role");
    assert_eq!(open_task(&rig).subtasks.len(), 1);
}

/// Un árbol con dos espacios de la organización `o1` y uno de otra: listas
/// sueltas, una carpeta con otra dentro, y dos listas del canal del cliente
/// `p1`, una en cada espacio.
fn places() -> Value {
    json!([
        { "id": "sp1", "orgId": "o1", "name": "Interno",
          "lists": [{ "id": "l1", "name": "tasks" }],
          "folders": [{ "id": "f1", "name": "Clientes",
              "lists": [{ "id": "l3", "name": "Acme", "projectId": "p1" }],
              "folders": [{ "id": "f2", "name": "Viejos", "folders": [], "lists": [{ "id": "l4", "name": "Archivo" }] }] }] },
        { "id": "sp2", "orgId": "o1", "name": "Proteus", "folders": [],
          "lists": [{ "id": "l2", "name": "App" }, { "id": "l5", "name": "Soporte", "projectId": "p1" }] },
        { "id": "sp3", "orgId": "o2", "name": "De otra organización", "folders": [], "lists": [{ "id": "l9", "name": "Ajena" }] },
    ])
}

/// Una tarea de la organización `o1`, en la lista `l1` del espacio `sp1`.
fn placed(project: &str, visibility: &str) -> Value {
    let mut task = detail("t1", "Tarea", "pending", project, visibility);
    task["task"]["orgId"] = json!("o1");
    task["task"]["spaceId"] = json!("sp1");
    task["ref"] = json!("interno-12");
    task
}

fn as_detail(task: &Value) -> cac_core::TaskDetail {
    serde_json::from_value(task.clone()).unwrap()
}

/// Lo ofrecido, como `Espacio: lista` y sus marcas (`*` la suya, `!` cambia la ref).
fn offered(task: &Value) -> Vec<String> {
    let spaces: Vec<cac_core::SpaceTree> = serde_json::from_value(places()).unwrap();
    cac_core::task::move_targets(&spaces, &as_detail(task))
        .into_iter()
        .flat_map(|group| {
            group.targets.into_iter().map(move |target| {
                let marks = format!("{}{}", if target.current { "*" } else { "" }, if target.crosses_space { "!" } else { "" });
                format!("{}: {}{marks}", group.space, target.label)
            })
        })
        .collect()
}

/// Una tarea del equipo puede ir a cualquier lista de su organización, y las
/// de otro espacio avisan de que allí cambia de ref.
///
/// Roturas que atrapa: ofrecer listas de otra organización (el servidor
/// contesta 403); dejarse las de una subcarpeta; no marcar la suya, o marcar
/// como «cambia la ref» una de su mismo espacio; callar que cambia en otro.
#[test]
fn a_team_task_can_go_to_any_list_of_its_organization() {
    assert_eq!(
        offered(&placed("", "")),
        [
            "Interno: tasks*",
            "Interno: Clientes · Acme",
            "Interno: Clientes · Viejos · Archivo",
            "Proteus: App!",
            "Proteus: Soporte!",
        ]
    );
    // Un servidor que no diga el espacio de la tarea: se lee del árbol, por
    // la lista en la que está.
    let mut unplaced = placed("", "");
    unplaced["task"].as_object_mut().unwrap().remove("spaceId");
    assert_eq!(offered(&unplaced)[3], "Proteus: App!");
}

/// La tarjeta de un cliente sólo se ofrece a las listas de su canal, y
/// conserva su folio aunque cambie de espacio. Una subtarea, a ninguna.
///
/// Roturas que atrapa: ofrecerle una lista de otro canal o del equipo (el
/// servidor contesta 409, `move-leaves-channel`); decir que su ref cambia
/// (su número es del proyecto, no del espacio); ofrecer mover una subtarea,
/// que vive en la lista de su madre; dejar de aplicar la regla a una
/// tarjeta retirada, que sigue gastando su folio.
#[test]
fn a_clients_card_can_only_go_to_lists_of_that_client() {
    let mut card = placed("p1", "public");
    card["task"]["listId"] = json!("l3");
    assert_eq!(offered(&card), ["Interno: Clientes · Acme*", "Proteus: Soporte"]);
    card["task"]["visibility"] = json!("internal");
    assert_eq!(offered(&card), ["Interno: Clientes · Acme*", "Proteus: Soporte"]);

    let mut subtask = placed("", "");
    subtask["task"]["parentId"] = json!("t0");
    assert_eq!(offered(&subtask), Vec::<String>::new());
}

/// Antes de mover se pregunta diciendo a cuál va; a otro espacio, con el
/// espacio delante y avisando de que la ref de ahora deja de serlo.
///
/// Roturas que atrapa: una pregunta que no nombra el destino, o que nombra
/// «App» sin decir de qué espacio; callar que la ref cambia, o decirlo
/// cuando no cambia; preguntar por la lista en la que ya está o por una que
/// no se le ofrece.
#[test]
fn the_question_before_moving_says_where_it_goes_and_when_the_ref_changes() {
    use cac_core::task::{move_question, move_targets};
    use cac_core::texts::Locale;
    let spaces: Vec<cac_core::SpaceTree> = serde_json::from_value(places()).unwrap();
    let task = as_detail(&placed("", ""));
    let groups = move_targets(&spaces, &task);

    let near = move_question(Locale::Es, &task, &groups, "l4").expect("hay que preguntar");
    assert_eq!(near.title, "¿Mover a «Clientes · Viejos · Archivo»?");
    assert!(near.body.contains("Su ref no cambia"), "{}", near.body);
    assert_eq!((near.confirm.as_str(), near.cancel.as_str()), ("Mover", "Cancelar"));

    let far = move_question(Locale::Es, &task, &groups, "l2").expect("hay que preguntar");
    assert_eq!(far.title, "¿Mover a «Proteus › App»?");
    assert!(far.body.contains("interno-12 deja de ser su ref"), "{}", far.body);

    assert_eq!(move_question(Locale::Es, &task, &groups, "l1"), None, "preguntó por la lista en la que ya está");
    assert_eq!(move_question(Locale::Es, &task, &groups, "l9"), None, "preguntó por una lista de otra organización");

    // Sin referencia (un servidor de antes), se nombra por su número.
    let mut bare = placed("", "");
    bare.as_object_mut().unwrap().remove("ref");
    let bare = as_detail(&bare);
    assert!(move_question(Locale::Es, &bare, &groups, "l2").unwrap().body.contains("#12 deja de ser su ref"));
}

/// `rig`, con el árbol de `places` cargado y esa tarea abierta.
async fn rig_with_places(task: Value) -> Rig {
    let rig = rig().await;
    Mock::given(method("GET")).and(path("/api/v1/task-spaces/")).respond_with(ok(places())).mount(&rig.server).await;
    mount_task(&rig.server, "t1", task).await;
    mount_columns(&rig.server).await;
    rig.core.tree().load("o1").await;
    rig.core.tasks().open("t1").await;
    rig
}

/// Mover a otra lista manda `listId` en un `PATCH`, y la tarea queda como la
/// contesta el servidor: en su lista nueva y con su ref nueva.
///
/// Roturas que atrapa: mandar otra cosa que la lista elegida; seguir
/// enseñando la lista y la ref de antes después de mover.
#[tokio::test]
async fn moving_to_another_list_shows_the_task_in_its_new_place() {
    let rig = rig_with_places(placed("", "")).await;
    let mut moved = placed("", "");
    moved["task"]["listId"] = json!("l2");
    moved["task"]["spaceId"] = json!("sp2");
    moved["task"]["seq"] = json!(301);
    moved["listName"] = json!("App");
    moved["spaceName"] = json!("Proteus");
    moved["ref"] = json!("proteus-301");
    Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(moved)).mount(&rig.server).await;
    assert_eq!(rig.core.tasks().move_question(cac_core::texts::Locale::Es, "l2").map(|question| question.title).as_deref(), Some("¿Mover a «Proteus › App»?"));

    rig.core.tasks().move_to_list("l2").await.expect("se movió");

    assert_eq!(patches(&rig.server).await, [json!({ "listId": "l2" })]);
    let now = open_task(&rig);
    assert_eq!((now.task.list_id.as_str(), now.list_name.as_str(), now.space_name.as_str()), ("l2", "App", "Proteus"));
    assert_eq!(now.reference.as_deref(), Some("proteus-301"));
    // Y lo que se le ofrece ya cuenta desde donde está ahora.
    let current: Vec<String> = rig.core.tasks().move_targets().into_iter().flat_map(|group| group.targets).filter(|target| target.current).map(|target| target.list_id).collect();
    assert_eq!(current, ["l2"]);
}

/// Lo que el servidor rechazaría no se le pide: se rechaza aquí, con su
/// mismo código. Y a la lista en la que ya está no se pide nada.
///
/// Roturas que atrapa: mandar un movimiento que va a contestar 409 (una
/// subtarea; la tarjeta de un cliente fuera de su canal) o 403 (a otra
/// organización); un `PATCH` por elegir la lista en la que ya está.
#[tokio::test]
async fn a_move_the_server_would_refuse_is_not_sent() {
    let mut card = placed("p1", "public");
    card["task"]["listId"] = json!("l3");
    let rig = rig_with_places(card).await;
    assert_eq!(refusal(rig.core.tasks().move_to_list("l2").await), "move-leaves-channel");
    assert_eq!(refusal(rig.core.tasks().move_to_list("l9").await), "channel-other-org");
    rig.core.tasks().move_to_list("l3").await.expect("ya está ahí");
    assert_eq!(patches(&rig.server).await, Vec::<Value>::new());

    let mut subtask = placed("", "");
    subtask["task"]["parentId"] = json!("t0");
    let rig = rig_with_places(subtask).await;
    assert_eq!(refusal(rig.core.tasks().move_to_list("l2").await), "move-subtask");
    assert_eq!(patches(&rig.server).await, Vec::<Value>::new());
}

/// A una lista que el árbol no conoce se pide igual, y decide el servidor:
/// si la rechaza, se dice con su código y la tarea sigue donde estaba.
///
/// Roturas que atrapa: no poder mover a una lista recién creada que el
/// árbol aún no trae; dar por movida una tarea que el servidor no movió.
#[tokio::test]
async fn the_server_has_the_last_word_on_a_move() {
    let rig = rig_with_places(placed("", "")).await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({ "success": false, "message": "No", "error": "move-leaves-channel" })))
        .mount(&rig.server)
        .await;

    assert_eq!(refusal(rig.core.tasks().move_to_list("l-nueva").await), "move-leaves-channel");

    assert_eq!(patches(&rig.server).await, [json!({ "listId": "l-nueva" })]);
    assert_eq!(open_task(&rig).task.list_id, "l1");
}

/// `rig`, con «Mi trabajo» cargado con las tareas `t1` y `t2` y la `t1` abierta.
async fn rig_with_my_work() -> Rig {
    let rig = rig().await;
    let row = |id: &str| {
        let mut row: Value = serde_json::from_str(include_str!("fixtures/open_task.json")).unwrap();
        row["id"] = json!(id);
        row
    };
    Mock::given(method("GET")).and(path("/api/v1/tasks/")).respond_with(ok(json!([row("t1"), row("t2")]))).mount(&rig.server).await;
    mount_task(&rig.server, "t1", detail("t1", "Subir el backend", "pending", "", "")).await;
    mount_columns(&rig.server).await;
    rig.core.my_work().load(None).await;
    rig.core.tasks().open("t1").await;
    rig
}

fn my_work_ids(rig: &Rig) -> Vec<String> {
    rig.core.my_work().state().tasks.into_iter().map(|task| task.id).collect()
}

/// Antes de borrar se pregunta nombrando la tarea y diciendo qué se va con
/// ella; si tiene subtareas, que ésas no se borran.
///
/// Roturas que atrapa: una pregunta que no dice qué tarea se borra; callar
/// que las subtareas se quedan (el servidor no las borra, y quien borra una
/// tarea partida en pasos da por hecho que se van); decirlo de una que no
/// tiene ninguna.
#[tokio::test]
async fn the_question_before_deleting_names_the_task_and_what_goes_with_it() {
    use cac_core::texts::Locale;
    let rig = rig_with_my_work().await;
    assert_eq!(rig.core.tasks().state().detail.unwrap().subtasks.len(), 1);

    let question = rig.core.tasks().delete_question(Locale::Es).expect("hay tarea abierta");
    assert_eq!(question.title, "¿Borrar la tarea «Subir el backend»?");
    assert_eq!(question.body, "La quita con sus comentarios y sus adjuntos. No se puede deshacer. Sus subtareas no se borran con ella.");
    assert_eq!((question.confirm.as_str(), question.cancel.as_str()), ("Borrar", "Cancelar"));

    let mut alone = detail("t9", "Suelta", "pending", "", "");
    alone["subtasks"] = json!([]);
    mount_task(&rig.server, "t9", alone).await;
    rig.core.tasks().open("t9").await;
    assert_eq!(rig.core.tasks().delete_question(Locale::Es).unwrap().body, "La quita con sus comentarios y sus adjuntos. No se puede deshacer.");

    rig.core.tasks().close();
    assert_eq!(rig.core.tasks().delete_question(Locale::Es), None);
}

/// Borrar pide el `DELETE` de la tarea abierta, la cierra y la quita de «Mi
/// trabajo» sin esperar a recargarlo.
///
/// Roturas que atrapa: borrar otra tarea, o por otra ruta; dejar abierta la
/// página de una tarea que ya no existe; dejar su fila en «Mi trabajo»,
/// pulsable (el servidor de mentira la sigue listando: sólo se va si se
/// quita aquí); llevarse también las demás filas.
#[tokio::test]
async fn deleting_closes_the_task_and_takes_it_out_of_my_work() {
    let rig = rig_with_my_work().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "Task deleted" })))
        .mount(&rig.server)
        .await;
    assert_eq!(my_work_ids(&rig), ["t1", "t2"]);

    rig.core.tasks().delete().await.expect("se borró");

    assert_eq!(requests_to(&rig.server, "DELETE", "/api/v1/tasks/t1").await.len(), 1);
    assert_eq!(rig.core.tasks().state().task_id, None);
    assert_eq!(my_work_ids(&rig), ["t2"]);
    // Sin árbol cargado no hay cuentas que poner al día.
    assert_eq!(requests_to(&rig.server, "GET", "/api/v1/task-spaces/").await.len(), 0);
}

/// Con el árbol cargado, borrar lo vuelve a pedir: sus cuentas llevaban esa
/// tarea (`fetchTree` tras `deleteTask` en la app de siempre).
///
/// Rotura que atrapa: una lista que sigue diciendo «3 abiertas» con dos.
#[tokio::test]
async fn deleting_brings_the_tree_again() {
    let rig = rig_with_my_work().await;
    Mock::given(method("GET")).and(path("/api/v1/task-spaces/")).respond_with(ok(places())).mount(&rig.server).await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
        .mount(&rig.server)
        .await;
    rig.core.tree().load("o1").await;

    rig.core.tasks().delete().await.expect("se borró");

    let asked = rig.server.received_requests().await.unwrap();
    let trees: Vec<&str> = asked.iter().filter(|request| request.url.path() == "/api/v1/task-spaces/").map(|request| request.url.query().unwrap_or("")).collect();
    assert_eq!(trees, ["orgId=o1", "orgId=o1"]);
}

/// Si el servidor no la borra, se dice y todo sigue como estaba; y sin tarea
/// abierta no se pide nada.
///
/// Roturas que atrapa: cerrar la página y quitar la fila de una tarea que
/// sigue existiendo; un `DELETE` sin tarea (a `/tasks/`).
#[tokio::test]
async fn a_delete_the_server_refuses_changes_nothing() {
    let rig = rig_with_my_work().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/t1"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "success": false, "message": "Forbidden", "error": "insufficient-role" })))
        .mount(&rig.server)
        .await;

    assert_eq!(refusal(rig.core.tasks().delete().await), "insufficient-role");
    assert_eq!(rig.core.tasks().state().task_id.as_deref(), Some("t1"));
    assert_eq!(my_work_ids(&rig), ["t1", "t2"]);

    rig.core.tasks().close();
    assert_eq!(refusal(rig.core.tasks().delete().await), "no-task-open");
    assert_eq!(rig.server.received_requests().await.unwrap().iter().filter(|request| request.method.as_str() == "DELETE").count(), 1);
}
