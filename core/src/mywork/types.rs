//! Las formas de «Mi trabajo». Espejo de `domain.OpenTask` del backend; lo
//! guarda `tests/contract.rs` contra un JSON que escribe una prueba de Go.

use serde::{Deserialize, Serialize};

/// Una tarea abierta, tal como la cuenta `GET /api/v1/tasks/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct OpenTask {
    pub id: String,
    pub seq: i64,
    pub title: String,
    /// `medium` es como se guarda la prioridad media; la API de tareas la
    /// llama `normal`. Aquí llega la guardada.
    pub priority: String,
    /// Una fecha guardada como instante. **No se lee**: para saber en qué día
    /// cae está `due_day`, ya calculado.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    /// El día del vencimiento, `YYYY-MM-DD`. Lo pone el core
    /// (`dates::due_day`); el servidor no lo manda.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_day: Option<String>,
    /// El estado crudo. Con éste se agrupa, no con `status_kind`: la clase
    /// mete `done` y `closed` en el mismo saco.
    pub status: String,
    /// Qué máquina de estados la gobierna (cliente o interna).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    pub status_name: String,
    pub status_kind: String,
    pub list_id: String,
    pub list_name: String,
    pub space_id: String,
    pub space_name: String,
    pub updated_at: String,
    #[serde(default)]
    pub subtasks_done: i64,
    #[serde(default)]
    pub subtasks_total: i64,
    /// Si es una subtarea, de qué tarea es.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_title: Option<String>,
    /// El responsable principal, por nombre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// El número que ve el cliente, cuando entró por su canal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folio: Option<String>,
}

/// Las preguntas que se le hacen a «Mi trabajo».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum WorkLens {
    #[default]
    Assigned,
    Created,
    Watching,
    Clients,
    All,
}

impl WorkLens {
    /// La consulta de cada lente.
    ///
    /// Todas mandan `origin=any` menos la de clientes. Sin decir nada, el
    /// servidor deja fuera lo que entró por un canal de cliente, y aquí eso
    /// mentía dos veces: una lista de cliente con siete tareas salía vacía
    /// bajo «Todas», y «asignadas a mí» callaba un ticket asignado a mí sólo
    /// porque lo levantó un cliente. Quién lo levantó es otra pregunta, y
    /// tiene su lente.
    pub fn query(self) -> &'static str {
        match self {
            WorkLens::Assigned => "assignee=me&origin=any",
            WorkLens::Created => "creator=me&origin=any",
            WorkLens::Watching => "watcher=me&origin=any",
            WorkLens::Clients => "origin=clients",
            WorkLens::All => "origin=any",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum ScopeKind {
    Space,
    List,
}

/// A qué ha acotado el árbol esta vista, si a algo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct WorkScope {
    pub kind: ScopeKind,
    pub id: String,
    pub name: String,
    /// De qué organización es esta lista o espacio. Viaja con el filtro
    /// porque la pregunta al cargar es «¿lo que hay puesto es de aquí?», y
    /// sin sello un ámbito no se tiraría nunca al cambiar de organización.
    pub org_id: Option<String>,
}

/// «Mi trabajo», entero: lo que la pantalla pinta.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct MyWorkState {
    pub lens: WorkLens,
    pub scope: Option<WorkScope>,
    /// De qué organización son las tareas cargadas, para que un evento pueda
    /// pedir la misma lista otra vez.
    pub org_id: Option<String>,
    pub include_closed: bool,
    pub tasks: Vec<OpenTask>,
    pub loading: bool,
    pub error: Option<String>,
    /// Lo que pidió la última carga buena. Con eso `ensure_loaded` sabe si lo
    /// que hay ya responde a la pregunta.
    pub loaded_key: Option<String>,
}
