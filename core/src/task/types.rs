//! Las formas del detalle de una tarea. Espejo de `domain.TaskDetail` y de lo
//! que cuelga de él; lo guarda `tests/contract.rs` contra `task_detail.json`,
//! que escribe una prueba de Go con todos los campos puestos.

use serde::{Deserialize, Deserializer, Serialize};

/// Go escribe `null` donde una lista está sin crear. Aquí es una lista vacía.
fn or_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/// La tarea en sí. Espejo de `domain.Item`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub org_id: String,
    pub list_id: String,
    #[serde(default)]
    pub space_id: String,
    /// Con algo dentro, la tarea vive en el canal de un cliente.
    #[serde(default)]
    pub project_id: String,
    pub seq: i64,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// El estado crudo: `pending`, `in_progress`, `resolved`, `closed`.
    pub status: String,
    #[serde(default)]
    pub category: String,
    /// Con el nombre de la API: la media es `normal`. El core la normaliza al
    /// leer (`canonical_priority`), venga como venga.
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub area: String,
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub viewport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_purge_at: Option<String>,
    #[serde(default)]
    pub reporter_name: String,
    #[serde(default)]
    pub reporter_email: String,
    #[serde(default)]
    pub reporter_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_user_id: Option<String>,
    /// `internal` o `public`: si el cliente del canal la ve.
    #[serde(default)]
    pub visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at: Option<String>,
    /// Una fecha guardada como instante. **No se lee**: está `due_day`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    /// El día del vencimiento, `YYYY-MM-DD`. Lo pone el core; el servidor no
    /// lo manda.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_day: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
    /// El «dónde quedó» de la última sesión que la tocó.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_by_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
}

/// Una columna de una lista. Espejo de `domain.TaskStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub id: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub list_id: String,
    pub name: String,
    #[serde(default)]
    pub color: String,
    /// `open`, `active` o `done`. `done` mete lo resuelto y lo cerrado en el
    /// mismo saco: para decidir se mira `status`.
    #[serde(default)]
    pub kind: String,
    /// El estado crudo al que corresponde la columna.
    #[serde(default)]
    pub status: String,
}

/// Espejo de `domain.TaskTag`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskTag {
    pub id: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub org_id: String,
    pub name: String,
    #[serde(default)]
    pub color: String,
}

/// Una persona, lo justo para enseñarla. Espejo de `domain.UserSummary`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct UserSummary {
    pub id: String,
    /// Para buscar, mencionar e identificar.
    pub username: String,
    /// Para enseñar. Puede faltar: quien pinte cae al usuario.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Quién escribió un comentario: alguien del equipo, el cliente o el sistema.
/// Espejo de `domain.AgentSignature`: lo escribió un agente con el token de
/// `of`, y cuál, si lo dijo (command-and-control-522).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct AgentSignature {
    #[serde(default)]
    pub of: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// Espejo de `domain.CommentAuthor`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct CommentAuthor {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}

/// Espejo de `domain.ItemAttachment`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskAttachment {
    pub id: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub item_id: String,
    /// Sin esto es de la tarea; con esto, del comentario donde se puso.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_id: Option<String>,
    /// Relativa al servidor (`/api/v1/tasks/…/raw`), y pide sesión.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploaded_by: Option<String>,
}

/// Espejo de `domain.TaskCommentResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskComment {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<CommentAuthor>,
    #[serde(default)]
    pub author_user_id: String,
    #[serde(default)]
    pub author_name: String,
    /// `internal` o `public`: quién lo lee.
    #[serde(default)]
    pub visibility: String,
    /// `system` son las líneas que escribió el servidor («status: x → y»).
    #[serde(default)]
    pub kind: String,
    /// De dónde vino una línea de sistema escrita desde fuera (`gh`, `deploy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `mcp` si lo escribió un agente con el token de `author_name`: esas
    /// notas van aparte del hilo (command-and-control-359).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Con `via: mcp`, el agente de quién y cuál.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentSignature>,
    #[serde(default)]
    pub body: String,
    #[serde(default, deserialize_with = "or_empty")]
    pub attachments: Vec<TaskAttachment>,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// Espejo de `domain.GitSummary`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct GitSummary {
    #[serde(default)]
    pub branches: i64,
    #[serde(default)]
    pub prs: i64,
    #[serde(default)]
    pub commits: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_badge: Option<String>,
}

/// Una tarjeta: una subtarea, o la tarea madre. Espejo de `domain.TaskCard`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskCard {
    pub id: String,
    pub seq: i64,
    pub title: String,
    #[serde(default)]
    pub priority: String,
    /// En qué columna está. El estado sale de la columna, no de aquí.
    #[serde(default)]
    pub status_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    #[serde(default)]
    pub has_description: bool,
    #[serde(default)]
    pub comment_count: i64,
    #[serde(default)]
    pub attachment_count: i64,
    #[serde(default)]
    pub subtask_count: i64,
    #[serde(default)]
    pub subtask_done: i64,
    #[serde(default, deserialize_with = "or_empty")]
    pub tags: Vec<TaskTag>,
    #[serde(default, deserialize_with = "or_empty")]
    pub assignees: Vec<UserSummary>,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    #[serde(default)]
    pub created_at: String,
    /// Qué máquina la gobierna: `client` o `internal`.
    #[serde(default)]
    pub flow: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
}

/// De qué tarea depende otra. Espejo de `domain.TaskBlocker`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskBlocker {
    pub id: String,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub title: String,
    #[serde(default)]
    pub status: String,
    /// Si sigue abierta: mientras lo esté, la que depende de ella espera.
    #[serde(default)]
    pub open: bool,
}

/// Una decisión enlazada. Espejo de `domain.DecisionRef`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DecisionRef {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub decided_at: String,
    /// Con algo dentro, otra decisión la sustituyó.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_id: Option<String>,
}

/// Espejo de `domain.ReleaseRef`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ReleaseRef {
    pub tag: String,
    #[serde(default)]
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Una rama, una PR o un commit enlazado. Espejo de `domain.TaskGitLink`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskGitLink {
    pub id: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub repo_id: i64,
    #[serde(default)]
    pub repo_full_name: String,
    /// `branch`, `pr` o `commit`.
    pub kind: String,
    /// El nombre de la rama, el número de la PR o el sha del commit.
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub author_login: String,
    #[serde(default)]
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    #[serde(default)]
    pub via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<String>,
    #[serde(default)]
    pub occurred_at: String,
    /// En un commit, la release en la que salió.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released_in: Option<ReleaseRef>,
}

/// Espejo de `domain.TaskGitLinks`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskGitLinks {
    #[serde(default)]
    pub summary: GitSummary,
    #[serde(default, deserialize_with = "or_empty")]
    pub branches: Vec<TaskGitLink>,
    #[serde(default, deserialize_with = "or_empty")]
    pub prs: Vec<TaskGitLink>,
    #[serde(default, deserialize_with = "or_empty")]
    pub commits: Vec<TaskGitLink>,
}

/// Todo lo que enseña el detalle de una tarea. Espejo de `domain.TaskDetail`.
///
/// Falta `telemetry` a propósito: es el rastro que acompaña a un reporte de
/// fallo, un JSON sin forma fija, y la primera entrega no lo pinta. El
/// contrato lo nombra como lo único que el core no lee.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    pub task: Task,
    #[serde(default)]
    pub list_name: String,
    #[serde(default)]
    pub space_name: String,
    /// La columna en la que está.
    pub status: TaskStatus,
    #[serde(default, deserialize_with = "or_empty")]
    pub tags: Vec<TaskTag>,
    #[serde(default, deserialize_with = "or_empty")]
    pub assignees: Vec<UserSummary>,
    #[serde(default, deserialize_with = "or_empty")]
    pub comments: Vec<TaskComment>,
    #[serde(default, deserialize_with = "or_empty")]
    pub attachments: Vec<TaskAttachment>,
    #[serde(default, deserialize_with = "or_empty")]
    pub subtasks: Vec<TaskCard>,
    /// Si ésta es una subtarea, su madre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TaskCard>,
    /// El número que ve el cliente (`portento-89`), si entró por su canal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_slug: Option<String>,
    /// Cómo se nombra en un commit para que cac la enlace (`proteus-12`).
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Las claves de fuera que nombra (`PROTEUS-165`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
    /// De ésas, las que son su ticket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tickets: Option<Vec<String>>,
    /// Quién escribió el «dónde quedó», con su nombre visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_by: Option<String>,
    /// Si el «dónde quedó» lo escribió un agente con el token de `handoff_by`,
    /// cuál. Sin pintar todavía.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_agent: Option<AgentSignature>,
    #[serde(default, deserialize_with = "or_empty")]
    pub blocked_by: Vec<TaskBlocker>,
    #[serde(default, deserialize_with = "or_empty")]
    pub decisions: Vec<DecisionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<TaskGitLinks>,
}
