//! El detalle de una tarea y lo que se le puede cambiar.
//!
//! Vino de `openTask`, `updateTask`, `moveTask`, `addComment` y
//! `createSubtask` en `app/src/store/tasks.store.ts`. Hay **una** tarea
//! abierta a la vez (la del panel de detalle); el tablero, que mueve tarjetas
//! sin abrirlas, llega en su fase.

mod actions;
mod compose;
mod edit;
mod files;
mod flow;
mod mention;
mod new;
mod sheet;
mod types;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub use actions::{is_pull_url, move_question, move_targets, MoveGroup, MoveTarget, Question, MAX_HANDOFF, MAX_TASK_TITLE};
pub use compose::{compose, continue_list, ComposeAction, ComposeEdit};
pub use edit::{assignee_choices, retitled, shown_name, tag_choices, tag_color_for, title_line, Choice, MAX_TAG_NAME, TAG_PALETTE};
pub use mention::{
    comment_placeholder, mention_link, mention_matches, mention_query, mentioned_user, mentions_allowed, offered_mentions, with_mention,
    mention_closed, mention_highlighted, mention_mark_kept, mention_menu, mention_moved, MentionEdit, MentionMark, MentionMenu, MentionPerson, MentionQuery,
};
pub use new::{destinations, drafted, Destination, Destinations, TaskDraft};
pub use files::{attachment_fits, attachment_path, file_citation, image_type, is_image, multipart, safe_file_name, size_label, thumbnail_size,
    with_citation, Cited, Picture,
    MAX_ATTACHMENT_BYTES, THUMBNAIL_MAX_HEIGHT, THUMBNAIL_MAX_WIDTH,
};
pub use flow::{can_transition, canonical_priority, canonical_status, flow_for, Flow};
pub use sheet::{client_reads, can_change_comment, is_agent_note, says_something, task_sheet, SheetActivity, SheetBlocker, SheetComment, SheetFile, SheetSubtask, SheetTag, TaskSheet};
pub use types::{
    CommentAuthor, DecisionRef, GitSummary, ReleaseRef, Task, TaskAttachment, TaskBlocker, TaskCard,
    TaskComment, TaskDetail, TaskGitLink, TaskGitLinks, TaskStatus, TaskTag, UserSummary,
};

use crate::dates::due_day;
use crate::{ApiError, Core};

/// Una columna a la que se podría mover la tarea abierta, y si se puede.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct StatusOption {
    pub status: TaskStatus,
    /// Si la máquina de la tarea deja ir ahí desde donde está. Una que no se
    /// puede se enseña apagada: ofrecerla es un botón que contesta 409.
    pub allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskState {
    /// La tarea que está abierta, aunque todavía no haya llegado.
    pub task_id: Option<String>,
    pub detail: Option<TaskDetail>,
    /// Las columnas de su lista, en su orden.
    pub statuses: Vec<StatusOption>,
    /// Las etiquetas de su organización: las que se le pueden poner.
    pub tags: Vec<TaskTag>,
    /// El equipo de su organización: a quién se le puede dar.
    pub team: Vec<UserSummary>,
    pub loading: bool,
    pub error: Option<String>,
}

/// Qué se hace con el vencimiento en un cambio.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DueChange {
    #[default]
    Keep,
    /// A ese día, `YYYY-MM-DD`. Un día, no una hora.
    Set { day: String },
    Clear,
}

/// Lo que se cambia de una tarea. Sólo viaja lo que se puso.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub due: DueChange,
    /// La lista entera de etiquetas; vacía las quita todas.
    pub tag_ids: Option<Vec<String>>,
    /// La lista entera de responsables; vacía los quita todos.
    pub assignee_ids: Option<Vec<String>>,
    /// Llevarla a otra lista.
    pub list_id: Option<String>,
    /// El «dónde quedó», entero; vacío lo quita.
    pub handoff: Option<String>,
    /// `internal` la retira de la vista del cliente; `public` se la enseña.
    pub visibility: Option<String>,
}

/// Una tarea por crear.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NewTask {
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub due_day: Option<String>,
    pub assignee_ids: Vec<String>,
    /// Con esto nace como subtarea de ésa.
    pub parent_id: Option<String>,
    /// `internal` o `public`, sólo cuando alguien eligió: sin esto decide el
    /// servidor, que en la lista de un cliente es «la ve».
    pub visibility: Option<String>,
}

fn refused(code: &str, message: &str) -> ApiError {
    ApiError::Server { status: 0, code: code.to_string(), message: message.to_string() }
}

/// El instante con el que se guarda un día de vencimiento: su medianoche UTC,
/// que es como lo guarda la app. `None` si no es un día.
pub fn due_instant(day: &str) -> Option<String> {
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok().map(|day| format!("{day}T00:00:00Z"))
}

/// El cuerpo del `PATCH` de una tarea.
///
/// Quitar el vencimiento es `clearDueAt`, no `dueAt: null`: el servidor lee un
/// `null` como «no tocar», y la fecha se quedaba puesta sin error (#95).
pub fn patch_body(patch: &TaskPatch) -> Result<Value, ApiError> {
    let mut body = Map::new();
    if let Some(title) = &patch.title {
        body.insert("title".into(), json!(title));
    }
    if let Some(description) = &patch.description {
        body.insert("description".into(), json!(description));
    }
    if let Some(priority) = &patch.priority {
        body.insert("priority".into(), json!(canonical_priority(priority)));
    }
    match &patch.due {
        DueChange::Keep => {}
        DueChange::Set { day } => {
            let at = due_instant(day).ok_or_else(|| refused("invalid-due-day", "Invalid due date"))?;
            body.insert("dueAt".into(), json!(at));
        }
        DueChange::Clear => {
            body.insert("clearDueAt".into(), json!(true));
        }
    }
    if let Some(tag_ids) = &patch.tag_ids {
        body.insert("tagIds".into(), json!(tag_ids));
    }
    if let Some(assignee_ids) = &patch.assignee_ids {
        body.insert("assigneeIds".into(), json!(assignee_ids));
    }
    if let Some(list_id) = &patch.list_id {
        body.insert("listId".into(), json!(list_id));
    }
    if let Some(handoff) = &patch.handoff {
        body.insert("handoff".into(), json!(handoff));
    }
    if let Some(visibility) = &patch.visibility {
        body.insert("visibility".into(), json!(visibility));
    }
    Ok(Value::Object(body))
}

/// El cuerpo del `POST` que crea una tarea.
pub fn new_task_body(task: &NewTask) -> Result<Value, ApiError> {
    let mut body = Map::new();
    body.insert("title".into(), json!(task.title.trim()));
    if let Some(description) = &task.description {
        body.insert("description".into(), json!(description));
    }
    if let Some(priority) = &task.priority {
        body.insert("priority".into(), json!(canonical_priority(priority)));
    }
    if let Some(day) = &task.due_day {
        let at = due_instant(day).ok_or_else(|| refused("invalid-due-day", "Invalid due date"))?;
        body.insert("dueAt".into(), json!(at));
    }
    if !task.assignee_ids.is_empty() {
        body.insert("assigneeIds".into(), json!(task.assignee_ids));
    }
    if let Some(parent_id) = &task.parent_id {
        body.insert("parentId".into(), json!(parent_id));
    }
    if let Some(visibility) = &task.visibility {
        body.insert("visibility".into(), json!(visibility));
    }
    Ok(Value::Object(body))
}

/// Deja un detalle recién llegado como lo lee el resto del core: la prioridad
/// con un solo nombre y el día del vencimiento ya calculado.
fn tidy(mut detail: TaskDetail) -> TaskDetail {
    detail.task.priority = canonical_priority(&detail.task.priority).to_string();
    detail.task.due_day = due_day(detail.task.due_at.as_deref()).map(|day| day.to_string());
    for card in detail.subtasks.iter_mut().chain(detail.parent.iter_mut()) {
        card.priority = canonical_priority(&card.priority).to_string();
    }
    detail
}

/// Las columnas, con lo que la máquina de esta tarea deja hacer desde donde
/// está.
fn options_for(detail: &TaskDetail, statuses: Vec<TaskStatus>) -> Vec<StatusOption> {
    let flow = flow_for(&detail.task.project_id, &detail.task.visibility);
    statuses
        .into_iter()
        .map(|status| StatusOption {
            allowed: can_transition(flow, &detail.task.status, &status.status),
            status,
        })
        .collect()
}

/// El detalle de este core.
pub struct Tasks {
    core: Core,
}

impl Core {
    pub fn tasks(&self) -> Tasks {
        Tasks { core: self.clone() }
    }
}

impl Tasks {
    pub fn state(&self) -> TaskState {
        self.core.inner.task.lock().expect("task lock").clone()
    }

    fn update_state(&self, change: impl FnOnce(&mut TaskState)) {
        let state = {
            let mut state = self.core.inner.task.lock().expect("task lock");
            change(&mut state);
            state.clone()
        };
        self.core.inner.listener.task_changed(state);
    }

    fn is_open(&self, task_id: &str) -> bool {
        self.state().task_id.as_deref() == Some(task_id)
    }

    fn open_id(&self) -> Result<String, ApiError> {
        self.state().task_id.ok_or_else(|| refused("no-task-open", "No task is open"))
    }

    /// Pone un detalle que acaba de llegar, si sigue siendo el de la tarea
    /// abierta: la respuesta de una que ya se cerró, o de la anterior cuando
    /// ya se abrió otra, no pisa lo que hay.
    fn install(&self, task_id: &str, detail: TaskDetail) {
        if !self.is_open(task_id) {
            return;
        }
        let detail = tidy(detail);
        self.update_state(|state| {
            let columns: Vec<TaskStatus> = state.statuses.drain(..).map(|option| option.status).collect();
            // Las columnas son de una lista: si la tarea cambió de lista, las
            // que había ya no son las suyas.
            let same_list = columns.iter().all(|column| column.list_id == detail.task.list_id);
            state.statuses = if same_list { options_for(&detail, columns) } else { Vec::new() };
            state.detail = Some(detail);
            state.loading = false;
            state.error = None;
        });
    }

    async fn fetch(&self, task_id: &str) -> Result<(), ApiError> {
        let detail: TaskDetail = self.core.get(&format!("/api/v1/tasks/{task_id}")).await?;
        let (list_id, org_id) = (detail.task.list_id.clone(), detail.task.org_id.clone());
        self.install(task_id, detail);

        self.ensure_columns(task_id, &list_id).await;
        self.ensure_choices(task_id, &org_id).await;
        Ok(())
    }

    /// Trae las columnas de la lista de la tarea abierta, si no están.
    async fn ensure_columns(&self, task_id: &str, list_id: &str) {
        if !self.is_open(task_id) || !self.state().statuses.is_empty() {
            return;
        }
        let path = format!("/api/v1/task-lists/{list_id}/statuses");
        // Sin columnas se puede leer la tarea; sólo no se puede moverla.
        let Ok(columns) = self.core.get::<Vec<TaskStatus>>(&path).await else {
            return;
        };
        self.update_state(|state| {
            if state.task_id.as_deref() != Some(task_id) {
                return;
            }
            if let Some(detail) = &state.detail {
                state.statuses = options_for(detail, columns);
            }
        });
    }

    /// Abre una tarea: lo que hubiera de otra se quita ya, para no enseñar el
    /// detalle de la anterior con el título de ésta mientras llega.
    pub async fn open(&self, task_id: &str) {
        self.update_state(|state| {
            *state = TaskState { task_id: Some(task_id.to_string()), loading: true, ..TaskState::default() };
        });
        if let Err(error) = self.fetch(task_id).await {
            self.update_state(|state| {
                if state.task_id.as_deref() == Some(task_id) {
                    state.loading = false;
                    state.error = Some(error.to_string());
                }
            });
        }
    }

    /// Vuelve a pedir la tarea abierta **sin** vaciarla: lo que se está
    /// leyendo no parpadea porque alguien escribió un comentario.
    pub async fn refresh(&self) {
        if let Some(task_id) = self.state().task_id {
            // Un fallo aquí deja lo que había: sigue siendo lo último que se supo.
            let _ = self.fetch(&task_id).await;
        }
    }

    pub fn close(&self) {
        self.update_state(|state| *state = TaskState::default());
    }

    /// Algo pasó con una tarea (un evento del servidor). Si es la abierta, se
    /// vuelve a pedir.
    pub async fn task_event(&self, task_id: &str) {
        if self.is_open(task_id) {
            self.refresh().await;
        }
    }

    /// Esa tarea se borró. Si es la abierta, se cierra: no hay nada que leer.
    pub fn task_deleted(&self, task_id: &str) {
        if self.is_open(task_id) {
            self.close();
        }
    }

    /// Avisa a «Mi trabajo» de que esta tarea cambió, para que su fila no se
    /// quede con lo de antes.
    fn tell_my_work(&self, task_id: &str, new: bool) {
        self.core.my_work().task_event(task_id, new);
    }

    /// Cambia campos de la tarea abierta.
    pub async fn update(&self, patch: TaskPatch) -> Result<(), ApiError> {
        let task_id = self.open_id()?;
        let body = patch_body(&patch)?;
        let detail: TaskDetail =
            self.core.call("PATCH", &format!("/api/v1/tasks/{task_id}"), Some(body)).await?;
        let list_id = detail.task.list_id.clone();
        self.install(&task_id, detail);
        // Si cambió de lista, las columnas se fueron con la anterior.
        self.ensure_columns(&task_id, &list_id).await;
        self.tell_my_work(&task_id, false);
        Ok(())
    }

    /// Mueve la tarea abierta a otra columna.
    ///
    /// Si su máquina no lo deja, no se pide: el servidor contestaría 409, y
    /// aquí ya se sabe. Con las columnas sin cargar se pide igual, y decide él.
    pub async fn set_status(&self, status_id: &str) -> Result<(), ApiError> {
        let task_id = self.open_id()?;
        let option = self.state().statuses.into_iter().find(|option| option.status.id == status_id);
        if matches!(option, Some(StatusOption { allowed: false, .. })) {
            // Con el código con que lo diría el servidor: es el mismo
            // rechazo, y así quien lo lee encuentra la misma frase.
            return Err(refused("bad-transition", "This status change is not allowed"));
        }
        self.move_to(&task_id, status_id).await?;
        self.refresh().await;
        self.tell_my_work(&task_id, false);
        Ok(())
    }

    async fn move_to(&self, task_id: &str, status_id: &str) -> Result<(), ApiError> {
        let path = format!("/api/v1/tasks/{task_id}/move");
        self.core.call_ok("POST", &path, Some(json!({ "statusId": status_id }))).await
    }

    /// Escribe un comentario en la tarea abierta. `visibility` sólo cuando
    /// alguien eligió (`internal` o `public`): sin ella decide el servidor.
    pub async fn add_comment(&self, body: &str, visibility: Option<&str>) -> Result<(), ApiError> {
        let task_id = self.open_id()?;
        let mut payload = json!({ "body": body });
        if let Some(visibility) = visibility {
            payload["visibility"] = json!(visibility);
        }
        let detail: TaskDetail =
            self.core.call("POST", &format!("/api/v1/tasks/{task_id}/comments"), Some(payload)).await?;
        self.install(&task_id, detail);
        Ok(())
    }

    /// Cambia lo que dice un comentario de la tarea abierta. En blanco no se
    /// manda (para eso está borrarlo), y si dice lo mismo no se pide nada.
    pub async fn edit_comment(&self, comment_id: &str, body: &str) -> Result<(), ApiError> {
        let task_id = self.open_id()?;
        let body = body.trim();
        if body.is_empty() {
            return Err(refused("empty-comment", "A comment needs some text"));
        }
        let says = self.state().detail.and_then(|detail| detail.comments.into_iter().find(|comment| comment.id == comment_id));
        if says.is_some_and(|comment| comment.body.trim() == body) {
            return Ok(());
        }
        let path = format!("/api/v1/tasks/{task_id}/comments/{comment_id}");
        self.core.call_ok("PATCH", &path, Some(json!({ "body": body }))).await?;
        self.refresh().await;
        Ok(())
    }

    pub async fn delete_comment(&self, comment_id: &str) -> Result<(), ApiError> {
        let task_id = self.open_id()?;
        let path = format!("/api/v1/tasks/{task_id}/comments/{comment_id}");
        self.core.call_ok("DELETE", &path, None).await?;
        self.refresh().await;
        Ok(())
    }

    /// Crea una tarea en una lista y la devuelve.
    pub async fn create(&self, list_id: &str, task: NewTask) -> Result<Task, ApiError> {
        let body = new_task_body(&task)?;
        let path = format!("/api/v1/task-lists/{list_id}/tasks");
        let mut created: Task = self.core.call("POST", &path, Some(body)).await?;
        created.priority = canonical_priority(&created.priority).to_string();
        created.due_day = due_day(created.due_at.as_deref()).map(|day| day.to_string());
        self.tell_my_work(&created.id, true);
        Ok(created)
    }

    /// Añade una línea al checklist de la tarea abierta.
    ///
    /// Vino del «+ Añadir» de `TaskDetailDrawer.tsx`, que manda `{ title,
    /// parentId }` a la lista de la madre. Sin título no se pide nada (allí
    /// el diálogo no deja crear), ni con uno que el servidor no guardaría
    /// (`CreateTaskRequest`, 300 letras). No lleva visibilidad ni proyecto:
    /// una subtarea es interna siempre, porque el servidor se lo quita.
    pub async fn add_subtask(&self, title: &str) -> Result<(), ApiError> {
        let title = title_line(title);
        let title = title.trim();
        if title.is_empty() {
            return Err(refused("empty-title", "A task needs a title"));
        }
        if title.chars().count() > actions::MAX_TASK_TITLE {
            return Err(refused("title-too-long", "A task title can be 300 characters at most"));
        }
        let task_id = self.open_id()?;
        let list_id = self
            .state()
            .detail
            .map(|detail| detail.task.list_id)
            .ok_or_else(|| refused("no-task-open", "No task is open"))?;
        let subtask = NewTask { title: title.to_string(), parent_id: Some(task_id), ..NewTask::default() };
        self.create(&list_id, subtask).await?;
        self.refresh().await;
        Ok(())
    }

    /// Si una subtarea está hecha: lo dice la columna en la que está.
    pub fn subtask_is_done(&self, subtask: &TaskCard) -> bool {
        self.state()
            .statuses
            .iter()
            .any(|option| option.status.id == subtask.status_id && option.status.kind == "done")
    }

    /// Marca o desmarca una línea del checklist.
    ///
    /// Va directa a «resuelta» (o de vuelta a «pendiente»), que sólo es legal
    /// en la máquina interna; una subtarea lo es siempre, porque el servidor
    /// le quita el proyecto al crearla. Por eso no se pregunta por la máquina
    /// de la madre: aplicarle la del cliente es como se murió este check.
    pub async fn set_subtask_done(&self, subtask_id: &str, done: bool) -> Result<(), ApiError> {
        let wanted = if done { "resolved" } else { "pending" };
        let column = self
            .state()
            .statuses
            .into_iter()
            .find(|option| canonical_status(&option.status.status) == wanted)
            .ok_or_else(|| refused("no-such-column", "This list has no column for that status"))?;
        self.move_to(subtask_id, &column.status.id).await?;
        self.refresh().await;
        Ok(())
    }
}
