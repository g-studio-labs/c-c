//! «Mi trabajo» como tablero: cuatro columnas por estado, y soltar una tarjeta
//! en otra.
//!
//! Vino de `app/src/pages/MyWork.tsx` (las columnas, `moveToColumn`,
//! `puedeSoltar`) y de `TaskCardMini.tsx` (la tarjeta). Las columnas de aquí
//! son **transversales**: las tarjetas vienen de listas y espacios distintos, y
//! «Hecha» no es la columna de ningún tablero concreto sino el mismo estado
//! visto en todos a la vez. Por eso soltar una tarjeta le cambia el estado **en
//! su propia lista**; no la mueve de lista.

use chrono::{NaiveDate, Utc};
use serde_json::json;

use crate::dates::today_in;
use crate::error::ApiError;
use crate::hue::hue_index;
use crate::mywork::list::{due_label, parent_crumb, DueLabel};
use crate::mywork::{MyWork, OpenTask, TaskFilter};
use crate::task::{canonical_priority, canonical_status, status_refusal, status_refusal_phrase, Flow, TaskStatus, SUBTASKS_OPEN};
use crate::texts::{error_phrase, text, Locale};

pub use crate::task::BAD_TRANSITION;

/// La lista de la tarjeta no tiene columna para ese estado.
pub const COLUMN_MISSING: &str = "column-missing";

/// Las cuatro columnas, por **estado** y no por clase.
///
/// La clase (`status_kind`) junta lo hecho y lo cerrado, y cerrada no es lo
/// mismo que hecha: un reporte se puede cerrar sin arreglarlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum BoardStatus {
    Open,
    InProgress,
    Done,
    Closed,
}

impl BoardStatus {
    /// En el orden en que se pintan.
    pub const ALL: [BoardStatus; 4] =
        [BoardStatus::Open, BoardStatus::InProgress, BoardStatus::Done, BoardStatus::Closed];

    /// La columna de un estado crudo, llegue con el nombre guardado
    /// (`pending`, `resolved`) o con el nuevo (`open`, `done`). `None` si es
    /// un estado que esta versión no conoce.
    pub fn of(status: &str) -> Option<BoardStatus> {
        match canonical_status(status) {
            "pending" => Some(BoardStatus::Open),
            "in_progress" => Some(BoardStatus::InProgress),
            "resolved" => Some(BoardStatus::Done),
            "closed" => Some(BoardStatus::Closed),
            _ => None,
        }
    }

    /// Su nombre en la app: el de las claves del catálogo (`work:status.*`) y
    /// el de los colores de estado (`--status-*`).
    pub fn key(self) -> &'static str {
        match self {
            BoardStatus::Open => "open",
            BoardStatus::InProgress => "in_progress",
            BoardStatus::Done => "done",
            BoardStatus::Closed => "closed",
        }
    }

    /// Lo terminado: sólo se pide al servidor con «todos los estados».
    pub fn is_finished(self) -> bool {
        matches!(self, BoardStatus::Done | BoardStatus::Closed)
    }
}

/// A quién está asignada una tarjeta, como se pinta: sus iniciales en su color.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BoardAssignee {
    pub name: String,
    /// Las dos primeras letras del nombre, en mayúsculas.
    pub initials: String,
    /// Su tono, de 0 a `HUES - 1`. Sale del nombre, que es lo único que traen
    /// las tarjetas de «Mi trabajo».
    pub hue: u32,
}

/// Una tarjeta del tablero.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BoardCard {
    pub id: String,
    pub seq: i64,
    pub title: String,
    /// De dónde es: «Espacio / Lista», y si es una subtarea, de cuál
    /// («› #289 Título»). Un tablero que cruza seis listas no dice de quién es
    /// el problema sólo con el título.
    pub path: String,
    /// Con el nombre de la API (`normal`, no `medium`).
    pub priority: String,
    /// El checklist, que sólo se pinta si tiene líneas (`subtasks_total > 0`).
    pub subtasks_done: i64,
    pub subtasks_total: i64,
    /// Sin vencimiento la tarjeta pinta un guion.
    pub due: Option<DueLabel>,
    pub assignee: Option<BoardAssignee>,
    /// Si la lee un cliente: es la que va por la máquina estricta.
    pub client_reads: bool,
}

/// Una columna con sus tarjetas, en el orden en que llegaron.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BoardColumn {
    pub status: BoardStatus,
    pub title: String,
    pub cards: Vec<BoardCard>,
    /// Si esta columna entra en lo que se preguntó. Con «sólo abiertas» lo
    /// terminado ni se pide, así que esa columna no está vacía: está fuera de
    /// la pregunta. Pintar «0» ahí es afirmar que no hay ninguna.
    pub asked: bool,
    /// Qué decir cuando no tiene tarjetas: «Nada», o que no se pidió.
    pub empty_hint: String,
}

/// Qué pasa si se suelta una tarjeta en una columna.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DropVerdict {
    /// Se puede: cambia de estado.
    Move,
    /// Ya está ahí: no pasa nada, y no es un rechazo (la columna no se pinta
    /// en rojo).
    Stay,
    /// No se puede: su máquina no lo deja (`bad-transition`), o tiene
    /// subtareas abiertas y la columna es de terminar (`subtasks-open`).
    /// `code` es el del servidor; la frase la da `MyWork::drop_error_phrase`.
    Refused { code: String },
}

/// Qué máquina gobierna una tarea de «Mi trabajo». Sin decirlo, la estricta:
/// proteger de más no rompe nada, y es lo que hace el servidor con todo lo que
/// no es interno.
fn flow_of(task: &OpenTask) -> Flow {
    Flow::named(task.flow.as_deref().unwrap_or(""))
}

/// ¿Puede esta tarjeta acabar en esa columna?
///
/// Se le pregunta a la máquina de **esa** tarea: aquí se cruzan listas, y en
/// la misma columna hay trabajo interno y tickets de cliente. Con una sola
/// máquina para todas habría que elegir a quién mentirle.
pub fn can_drop(task: &OpenTask, column: BoardStatus) -> DropVerdict {
    if BoardStatus::of(&task.status) == Some(column) {
        return DropVerdict::Stay;
    }
    match status_refusal(flow_of(task), &task.status, column.key(), open_subtasks(task)) {
        None => DropVerdict::Move,
        Some(code) => DropVerdict::Refused { code: code.to_string() },
    }
}

/// Cuántas subtareas de la tarjeta siguen abiertas. El servidor manda cuántas
/// tiene y cuántas están terminadas (hechas o cerradas): la diferencia.
fn open_subtasks(task: &OpenTask) -> i64 {
    (task.subtasks_total - task.subtasks_done).max(0)
}

fn assignee(name: &str) -> BoardAssignee {
    BoardAssignee {
        name: name.to_string(),
        initials: name.chars().take(2).collect::<String>().to_uppercase(),
        hue: hue_index(name),
    }
}

fn card(task: &OpenTask, locale: Locale, today: NaiveDate) -> BoardCard {
    let mut path = format!("{} / {}", task.space_name, task.list_name);
    if let Some(parent) = parent_crumb(task) {
        path = format!("{path} {parent}");
    }
    BoardCard {
        id: task.id.clone(),
        seq: task.seq,
        title: task.title.clone(),
        path,
        priority: canonical_priority(&task.priority).to_string(),
        subtasks_done: task.subtasks_done,
        subtasks_total: task.subtasks_total,
        due: due_label(locale, task.due_at.as_deref(), today),
        assignee: task.assignee.as_deref().filter(|name| !name.is_empty()).map(assignee),
        client_reads: flow_of(task) == Flow::Client,
    }
}

/// Reparte las tareas en las cuatro columnas, cada una en el orden en que
/// llegó (el del servidor: no hay rango dentro de una columna que cruza
/// listas).
///
/// Una tarea con un estado que esta versión no conoce no sale en ninguna:
/// ponerla en una columna sería decir de ella algo que no se sabe.
pub fn work_board(tasks: &[OpenTask], include_closed: bool, locale: Locale, today: NaiveDate) -> Vec<BoardColumn> {
    BoardStatus::ALL
        .into_iter()
        .map(|status| {
            let asked = include_closed || !status.is_finished();
            let hint = if asked { "work:myWork.nothing" } else { "work:myWork.notAsked" };
            BoardColumn {
                status,
                title: text(locale, &format!("work:status.{}", status.key())),
                cards: tasks
                    .iter()
                    .filter(|task| BoardStatus::of(&task.status) == Some(status))
                    .map(|task| card(task, locale, today))
                    .collect(),
                asked,
                empty_hint: text(locale, hint),
            }
        })
        .collect()
}

fn refused(code: &str, message: &str) -> ApiError {
    ApiError::Server { status: 0, code: code.to_string(), message: message.to_string() }
}

impl MyWork {
    /// El tablero que se pinta ahora: lo visible que pasa el filtro, en sus
    /// columnas. «Hoy» es el de la organización que se mira.
    pub fn board(&self, filter: &TaskFilter, locale: Locale) -> Vec<BoardColumn> {
        let orgs = self.core.orgs().state();
        let today = today_in(orgs.current_zone(), Utc::now(), &chrono::Local);
        work_board(&self.filtered(filter), self.state().include_closed, locale, today)
    }

    /// Qué pasaría al soltar esa tarjeta en esa columna, para pintar la
    /// columna mientras se arrastra. `None` si la tarjeta ya no está.
    pub fn drop_verdict(&self, task_id: &str, column: BoardStatus) -> Option<DropVerdict> {
        self.state().tasks.iter().find(|task| task.id == task_id).map(|task| can_drop(task, column))
    }

    /// Un fallo al soltar esa tarjeta en esa columna, dicho para quien lo lee.
    ///
    /// Un rechazo de cambio de estado dice el motivo, y el de las subtareas
    /// abiertas con su cuenta («No se puede cerrar: tiene 2 subtareas
    /// abiertas»), lo haya decidido el core antes de pedir nada o lo haya
    /// contestado el servidor. Lo demás, como cualquier otro fallo.
    pub fn drop_error_phrase(&self, locale: Locale, error: &ApiError, task_id: &str, column: BoardStatus, fallback_key: &str) -> String {
        if let ApiError::Server { code, .. } = error {
            let open = self.state().tasks.iter().find(|task| task.id == task_id).map(open_subtasks).unwrap_or(0);
            if let Some(phrase) = status_refusal_phrase(locale, code, column.key(), open) {
                return phrase;
            }
        }
        error_phrase(locale, error, Some(fallback_key))
    }

    /// Pone un estado a una tarea de las cargadas, aquí y sin preguntar.
    fn set_status_local(&self, task_id: &str, status: &str) {
        self.update(|state| {
            if let Some(task) = state.tasks.iter_mut().find(|task| task.id == task_id) {
                task.status = status.to_string();
            }
        });
    }

    /// Suelta una tarjeta en una columna: le cambia el estado en su lista.
    ///
    /// `Ok(true)` si se movió; `Ok(false)` si no había nada que hacer (ya
    /// estaba ahí, o la tarjeta ya no existe). Si su máquina no lo deja, se
    /// rechaza **sin pedir nada**: el servidor contestaría 409 y aquí ya se
    /// sabe.
    ///
    /// La tarjeta cambia de columna antes de que conteste el servidor —
    /// soltarla y verla volver a su sitio mientras llega la respuesta se lee
    /// como que el arrastre no funcionó—, y si él no la acepta vuelve a donde
    /// estaba y sale su error.
    pub async fn drop_on(&self, task_id: &str, column: BoardStatus) -> Result<bool, ApiError> {
        let Some(task) = self.state().tasks.into_iter().find(|task| task.id == task_id) else {
            return Ok(false);
        };
        match can_drop(&task, column) {
            DropVerdict::Stay => return Ok(false),
            DropVerdict::Refused { code } => {
                return Err(refused(&code, "This status change is not allowed"));
            }
            DropVerdict::Move => {}
        }

        self.set_status_local(task_id, column.key());
        if let Err(error) = self.move_to_column(&task, column).await {
            self.set_status_local(task_id, &task.status);
            // Lo rechazó el servidor por subtareas que la tarjeta no sabía
            // que tenía abiertas: se vuelve a preguntar, para que la columna
            // ya la rechace en el aire y el motivo se diga con su cuenta.
            if matches!(&error, ApiError::Server { code, .. } if code == SUBTASKS_OPEN) {
                self.load(self.state().org_id).await;
            }
            return Err(error);
        }
        // Se vuelve a preguntar: lo que la tarjeta dice de su columna (el
        // nombre, la clase) lo sabe el servidor, y con «sólo abiertas» una que
        // se terminó deja de estar en la respuesta.
        self.load(self.state().org_id).await;
        Ok(true)
    }

    /// El id de la columna de destino se le **pregunta al servidor**. Se
    /// podría componer (`<lista>/<estado>`), pero esa forma es una regla suya,
    /// y copiarla aquí es cómo se acaba con dos versiones de la misma verdad.
    ///
    /// Y se elige por estado, no por clase: «Hecha» y «Cerrada» son las dos
    /// `done`, y por clase todo iría a la primera.
    async fn move_to_column(&self, task: &OpenTask, column: BoardStatus) -> Result<(), ApiError> {
        let columns: Vec<TaskStatus> =
            self.core.get(&format!("/api/v1/task-lists/{}/statuses", task.list_id)).await?;
        let target = columns
            .into_iter()
            .find(|candidate| BoardStatus::of(&candidate.status) == Some(column))
            .ok_or_else(|| refused(COLUMN_MISSING, "This list has no column for that status"))?;
        // Sin vecinos: va al final de su columna. Es el sitio menos
        // sorprendente cuando la columna de la que viene ni siquiera es del
        // mismo tablero.
        let path = format!("/api/v1/tasks/{}/move", task.id);
        self.core.call_ok("POST", &path, Some(json!({ "statusId": target.id }))).await
    }
}
