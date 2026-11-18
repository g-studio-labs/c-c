//! Las dos máquinas de estados de una tarea, y con qué nombre viaja cada cosa.
//!
//! Espejo de `backend/internal/core/domain/report.go` (sección «Dos máquinas,
//! y por qué»). Quien decide es el servidor: esto está para que la pantalla no
//! ofrezca un cambio que va a contestar 409, que es un botón que no hace nada
//! y no dice por qué. Lo ata al servidor `state_machines.json`, que escribe
//! una prueba de Go con sus tablas de verdad.

use serde::{Deserialize, Serialize};

use crate::texts::{text, text_count, Locale};

/// Qué máquina gobierna una tarea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum Flow {
    /// La del cliente: estricta a propósito, y cerrado es terminal.
    Client,
    /// La interna: de cualquier sitio a cualquier sitio.
    Internal,
}

impl Flow {
    /// Por su nombre en el JSON. Uno desconocido es el del cliente: es la
    /// estricta, y es lo que hace el servidor con todo lo que no es interno.
    pub fn named(name: &str) -> Flow {
        if name == "internal" {
            Flow::Internal
        } else {
            Flow::Client
        }
    }
}

/// Qué máquina le toca a una tarea: la del cliente sólo si es de un canal
/// **y** el cliente la ve. Una tarea interna en la lista de un cliente es
/// interna (y una subtarea lo es siempre: nace sin proyecto).
pub fn flow_for(project_id: &str, visibility: &str) -> Flow {
    if !project_id.is_empty() && visibility != "internal" {
        Flow::Client
    } else {
        Flow::Internal
    }
}

/// El nombre guardado de un estado. El servidor acepta también `open` y
/// `done`, los del vocabulario al que se está cambiando.
pub fn canonical_status(status: &str) -> &str {
    match status {
        "open" => "pending",
        "done" => "resolved",
        other => other,
    }
}

const STATUSES: [&str; 4] = ["pending", "in_progress", "resolved", "closed"];

/// ¿Se puede pasar de `from` a `to` en esa máquina? Quedarse donde se está
/// siempre se puede.
pub fn can_transition(flow: Flow, from: &str, to: &str) -> bool {
    let (from, to) = (canonical_status(from), canonical_status(to));
    if from == to {
        return true;
    }
    if !STATUSES.contains(&from) || !STATUSES.contains(&to) {
        return false;
    }
    match flow {
        Flow::Internal => true,
        Flow::Client => matches!(
            (from, to),
            ("pending", "in_progress")
                | ("pending", "closed")
                | ("in_progress", "pending")
                | ("in_progress", "resolved")
                | ("in_progress", "closed")
                | ("resolved", "in_progress")
                | ("resolved", "closed")
        ),
    }
}

/// El código con que se rechaza un cambio de estado que la máquina de la tarea
/// no deja. Es el del servidor, a propósito: así el rechazo se dice igual
/// venga de quien venga.
pub const BAD_TRANSITION: &str = "bad-transition";

/// El código con que se rechaza terminar o cerrar una tarea con subtareas
/// abiertas. También es el del servidor (`service.ErrSubtasksOpen`).
pub const SUBTASKS_OPEN: &str = "subtasks-open";

/// ¿Impiden sus subtareas abiertas que la tarea pase a ese estado?
///
/// Espejo de `domain.OpenSubtasksBlock` (command-and-control-387): un padre no
/// se termina ni se cierra con subtareas abiertas, sea de la máquina que sea.
/// `open` son las que no están ni hechas ni cerradas. Reabrir y quedarse donde
/// se está no se impiden nunca. Lo ata al servidor la tabla `subtaskBlocks` de
/// `state_machines.json`.
pub fn open_subtasks_block(from: &str, to: &str, open: i64) -> bool {
    let (from, to) = (canonical_status(from), canonical_status(to));
    open > 0 && from != to && matches!(to, "resolved" | "closed")
}

/// Por qué no se puede ir de `from` a `to`, con el código con que lo diría el
/// servidor; `None` si se puede.
///
/// Primero la máquina y luego las subtareas, que es el orden en que pregunta
/// el servidor: a un ticket de cliente pendiente con subtareas abiertas lo
/// primero que le pasa al ir a «hecha» es que ese salto no existe.
pub fn status_refusal(flow: Flow, from: &str, to: &str, open_subtasks: i64) -> Option<&'static str> {
    if !can_transition(flow, from, to) {
        Some(BAD_TRANSITION)
    } else if open_subtasks_block(from, to, open_subtasks) {
        Some(SUBTASKS_OPEN)
    } else {
        None
    }
}

/// Un rechazo de cambio de estado, dicho para quien lo lee: qué se quiso
/// hacer, por qué no se puede y, si se sabe, cuántas subtareas quedan
/// («No se puede cerrar: tiene 2 subtareas abiertas»).
///
/// `open` es cero cuando no se sabe cuántas (lo rechazó el servidor y aquí
/// aún no han llegado): entonces sale la frase del catálogo de errores, que
/// dice el motivo sin la cuenta. Un código que no es de un cambio de estado
/// no tiene frase aquí.
pub fn status_refusal_phrase(locale: Locale, code: &str, to: &str, open: i64) -> Option<String> {
    match code {
        BAD_TRANSITION => Some(text(locale, "errors:bad-transition")),
        SUBTASKS_OPEN if open > 0 => {
            let key = if canonical_status(to) == "closed" { "work:task.subtasksOpenClose" } else { "work:task.subtasksOpenDone" };
            Some(text_count(locale, key, open))
        }
        SUBTASKS_OPEN => Some(text(locale, "errors:subtasks-open")),
        _ => None,
    }
}

/// La prioridad con el nombre que habla la API de tareas.
///
/// La media se guarda `medium` y la API la llama `normal`. Dentro del core hay
/// un solo nombre, el de la API: es el que se manda al escribir.
pub fn canonical_priority(priority: &str) -> &str {
    match priority {
        "medium" => "normal",
        "" => "none",
        other => other,
    }
}
