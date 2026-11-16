//! Lo que se cambia de una tarea con un toque: prioridad, vencimiento,
//! etiquetas y responsables.
//!
//! Vino de la columna lateral de `app/src/components/TaskDetailDrawer.tsx`.
//! El servidor recibe **la lista entera** de etiquetas y de responsables, así
//! que poner o quitar una es calcular la lista nueva desde la que hay: eso se
//! hace aquí, una vez, y no en cada pantalla.

use serde_json::json;

use super::{DueChange, TaskDetail, TaskPatch, TaskTag, Tasks, UserSummary};
use crate::mywork::filter::fold;
use crate::ApiError;

/// Cuánta gente del equipo se pide para ofrecerla: el tope del servidor.
pub(super) const TEAM_LIMIT: usize = 50;

/// Los colores de una etiqueta: la paleta de la app de siempre (`PALETTE` en
/// `app/src/components/ui/color-swatches.tsx`), en su orden. Tonos medios,
/// que se ven igual sobre el fondo claro que sobre el oscuro.
pub const TAG_PALETTE: [&str; 10] =
    ["#3B82F6", "#06B6D4", "#14B8A6", "#22C55E", "#EAB308", "#F97316", "#EF4444", "#EC4899", "#8B5CF6", "#64748B"];

/// Lo más que puede medir el nombre de una etiqueta: el tope del servidor
/// (`CreateTagRequest`), en letras.
pub const MAX_TAG_NAME: usize = 60;

/// El color con que nace una etiqueta: uno de la paleta, estable para su
/// nombre. Es la cuenta de `paletteFor` en la app de siempre, letra a letra,
/// para que la misma etiqueta nazca del mismo color desde cualquiera de las
/// dos apps.
pub fn tag_color_for(name: &str) -> &'static str {
    // Como en JavaScript: sobre unidades UTF-16 y con el desborde de 32 bits.
    let hash = name.encode_utf16().fold(0i32, |hash, unit| hash.wrapping_mul(31).wrapping_add(unit as i32));
    TAG_PALETTE[(hash as i64).unsigned_abs() as usize % TAG_PALETTE.len()]
}

/// Algo que se puede poner o quitar en la tarea abierta.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Choice {
    pub id: String,
    pub name: String,
    /// Si la tarea ya lo tiene.
    pub chosen: bool,
}

/// Cómo se le llama a alguien: por su nombre, y por su usuario sólo si no
/// lo ha puesto.
pub fn shown_name(person: &UserSummary) -> String {
    person.name.clone().filter(|name| !name.trim().is_empty()).unwrap_or_else(|| person.username.clone())
}

/// Lo que tiene la tarea más lo que se le podría poner, sin repetir.
///
/// Lo que ya tiene va siempre, esté o no entre lo ofrecido: una etiqueta de
/// otra organización o alguien que ya no está en el equipo tiene que poder
/// quitarse, y sin salir en la lista no habría cómo.
fn choices<T>(has: &[T], offered: &[T], id: impl Fn(&T) -> &str, name: impl Fn(&T) -> String) -> Vec<Choice> {
    let mut out: Vec<Choice> =
        has.iter().map(|item| Choice { id: id(item).to_string(), name: name(item), chosen: true }).collect();
    for item in offered {
        if !out.iter().any(|choice| choice.id == id(item)) {
            out.push(Choice { id: id(item).to_string(), name: name(item), chosen: false });
        }
    }
    // Por nombre, y no «lo puesto primero»: al marcar una, la lista no salta
    // bajo el puntero.
    out.sort_by_key(|choice| choice.name.to_lowercase());
    out
}

/// Las etiquetas de la tarea y las de su organización.
pub fn tag_choices(detail: &TaskDetail, offered: &[TaskTag]) -> Vec<Choice> {
    choices(&detail.tags, offered, |tag| &tag.id, |tag| tag.name.clone())
}

/// Quién lleva la tarea y quién más podría, del equipo.
pub fn assignee_choices(detail: &TaskDetail, team: &[UserSummary]) -> Vec<Choice> {
    choices(&detail.assignees, team, |person| &person.id, shown_name)
}

/// La lista con eso puesto si no estaba, o quitado si estaba.
fn toggled(mut ids: Vec<String>, id: &str) -> Vec<String> {
    match ids.iter().position(|have| have == id) {
        Some(at) => {
            ids.remove(at);
        }
        None => ids.push(id.to_string()),
    }
    ids
}

impl Tasks {
    fn open_detail(&self) -> Result<TaskDetail, ApiError> {
        self.state().detail.ok_or_else(|| super::refused("no-task-open", "No task is open"))
    }

    /// Trae las etiquetas de la organización de la tarea abierta y su equipo,
    /// si no están. Sin ellas la tarea se lee igual; sólo no hay qué ofrecer.
    pub(super) async fn ensure_choices(&self, task_id: &str, org_id: &str) {
        if !self.is_open(task_id) || org_id.is_empty() {
            return;
        }
        let state = self.state();
        if state.tags.is_empty() {
            if let Ok(tags) = self.core.get::<Vec<TaskTag>>(&format!("/api/v1/task-tags/?orgId={org_id}")).await {
                self.update_state(|state| {
                    if state.task_id.as_deref() == Some(task_id) {
                        state.tags = tags;
                    }
                });
            }
        }
        if state.team.is_empty() {
            let path = format!("/api/v1/users/search?orgId={org_id}&limit={TEAM_LIMIT}");
            if let Ok(team) = self.core.get::<Vec<UserSummary>>(&path).await {
                self.update_state(|state| {
                    if state.task_id.as_deref() == Some(task_id) {
                        state.team = team;
                    }
                });
            }
        }
    }

    /// Las etiquetas que se pueden poner o quitar en la tarea abierta.
    pub fn tag_choices(&self) -> Vec<Choice> {
        let state = self.state();
        state.detail.as_ref().map(|detail| tag_choices(detail, &state.tags)).unwrap_or_default()
    }

    /// A quién se le puede dar o quitar la tarea abierta.
    pub fn assignee_choices(&self) -> Vec<Choice> {
        let state = self.state();
        state.detail.as_ref().map(|detail| assignee_choices(detail, &state.team)).unwrap_or_default()
    }

    /// Le cambia el título. Uno en blanco no se manda: una tarea sin título
    /// no se encuentra en ninguna lista. Si es el que tiene, no se pide nada.
    pub async fn set_title(&self, title: &str) -> Result<(), ApiError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(super::refused("empty-title", "A task needs a title"));
        }
        if self.open_detail()?.task.title == title {
            return Ok(());
        }
        self.update(TaskPatch { title: Some(title.to_string()), ..TaskPatch::default() }).await
    }

    /// Le cambia la descripción (markdown); vacía se la quita. Si es la que
    /// tiene, no se pide nada.
    pub async fn set_description(&self, description: &str) -> Result<(), ApiError> {
        let description = description.trim();
        if self.open_detail()?.task.description.trim() == description {
            return Ok(());
        }
        self.update(TaskPatch { description: Some(description.to_string()), ..TaskPatch::default() }).await
    }

    /// Le pone esa prioridad. Si ya la tiene, no se pide nada.
    pub async fn set_priority(&self, priority: &str) -> Result<(), ApiError> {
        let priority = super::canonical_priority(priority);
        if self.open_detail()?.task.priority == priority {
            return Ok(());
        }
        self.update(TaskPatch { priority: Some(priority.to_string()), ..TaskPatch::default() }).await
    }

    /// Le pone ese día de vencimiento (`YYYY-MM-DD`), o se lo quita con
    /// `None`. Si ya es ése, no se pide nada: un selector de fecha que avisa
    /// al ponerle el valor que ya tenía no acaba guardando en bucle.
    pub async fn set_due(&self, day: Option<&str>) -> Result<(), ApiError> {
        if self.open_detail()?.task.due_day.as_deref() == day {
            return Ok(());
        }
        let due = match day {
            Some(day) => DueChange::Set { day: day.to_string() },
            None => DueChange::Clear,
        };
        self.update(TaskPatch { due, ..TaskPatch::default() }).await
    }

    /// Le pone esa etiqueta, o se la quita si ya la tenía.
    pub async fn toggle_tag(&self, tag_id: &str) -> Result<(), ApiError> {
        let has = self.open_detail()?.tags.into_iter().map(|tag| tag.id).collect();
        self.update(TaskPatch { tag_ids: Some(toggled(has, tag_id)), ..TaskPatch::default() }).await
    }

    /// Le pone a la tarea abierta la etiqueta que se llama así: la que ya
    /// haya con ese nombre, o una nueva de su organización.
    ///
    /// Vino del «+ Etiqueta nueva» de `TaskDetailDrawer.tsx`, que crea
    /// siempre. Aquí no se crea otra que se llame igual (sin mirar mayúsculas
    /// ni acentos): dos «Backend» en la lista no se distinguen. Y la
    /// organización es la de la tarea, que tiene que decirla: con ella vacía
    /// no se pide nada.
    pub async fn add_tag_named(&self, name: &str) -> Result<(), ApiError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(super::refused("empty-tag", "A tag needs a name"));
        }
        if name.chars().count() > MAX_TAG_NAME {
            return Err(super::refused("tag-too-long", "A tag name can be 60 characters at most"));
        }
        let task_id = self.open_id()?;
        let detail = self.open_detail()?;
        let wanted = fold(name);
        let known = detail.tags.iter().chain(self.state().tags.iter()).find(|tag| fold(tag.name.trim()) == wanted).cloned();
        let tag = match known {
            Some(tag) => tag,
            None => {
                let org_id = detail.task.org_id.as_str();
                if org_id.is_empty() {
                    return Err(super::refused("no-organization", "This task does not say which organization it belongs to"));
                }
                let body = json!({ "orgId": org_id, "name": name, "color": tag_color_for(name) });
                let created: TaskTag = self.core.call("POST", "/api/v1/task-tags/", Some(body)).await?;
                // Entra en lo que se ofrece: aunque ponérsela a la tarea
                // falle después, ya existe y tiene que poder elegirse.
                self.update_state(|state| {
                    if state.task_id.as_deref() == Some(task_id.as_str()) {
                        state.tags.push(created.clone());
                    }
                });
                created
            }
        };
        if detail.tags.iter().any(|has| has.id == tag.id) {
            return Ok(());
        }
        let mut ids: Vec<String> = detail.tags.into_iter().map(|has| has.id).collect();
        ids.push(tag.id);
        self.update(TaskPatch { tag_ids: Some(ids), ..TaskPatch::default() }).await
    }

    /// Le da la tarea a esa persona, o se la quita si ya la llevaba.
    pub async fn toggle_assignee(&self, user_id: &str) -> Result<(), ApiError> {
        let has = self.open_detail()?.assignees.into_iter().map(|person| person.id).collect();
        self.update(TaskPatch { assignee_ids: Some(toggled(has, user_id)), ..TaskPatch::default() }).await
    }
}
