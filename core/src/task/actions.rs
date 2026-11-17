//! Lo que se le hace a una tarea entera desde su página: dejar escrito dónde
//! quedó y llevarla a otra lista.
//!
//! Vino de `app/src/components/TaskDetailDrawer.tsx` (`HandoffSection`,
//! `MoveToList`) y de `app/src/lib/move-targets.ts`.
//!
//! Mover una tarea la saca de donde la mira más gente, así que trae **su
//! pregunta** ([`Question`]): qué se va a hacer y a dónde va, con las frases
//! de la app de siempre. La pantalla la enseña y sólo pide el cambio si
//! contestan que sí; qué dice la pregunta se decide aquí, una vez, para
//! todas las apps.

use super::{refused, TaskDetail, TaskPatch, Tasks};
use crate::texts::{text, text_with, Locale};
use crate::tree::{FolderTree, SpaceTree};
use crate::ApiError;

/// Lo más que mide un «dónde quedó», en letras: el tope del servidor
/// (`UpdateTaskRequest.Handoff`). Es un resumen para leer de un vistazo.
pub const MAX_HANDOFF: usize = 2000;

/// Lo más que mide el título de una tarea, en letras (`CreateTaskRequest`).
pub const MAX_TASK_TITLE: usize = 300;

/// Una lista a la que se puede llevar la tarea abierta.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MoveTarget {
    pub list_id: String,
    /// «Carpeta · Lista», sin el espacio: el espacio es la cabecera del grupo.
    pub label: String,
    /// Es de otro espacio, y allí una tarea interna toma el número siguiente:
    /// su ref cambia, y quien elige tiene que saberlo antes.
    pub crosses_space: bool,
    /// Es la lista en la que ya está.
    pub current: bool,
}

/// Las listas de un espacio a las que se puede llevar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MoveGroup {
    pub space: String,
    pub targets: Vec<MoveTarget>,
}

/// Lo que se pregunta antes de hacer algo que no se deshace o que ve más
/// gente: qué se va a hacer, qué implica y los dos botones.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Question {
    pub title: String,
    pub body: String,
    pub confirm: String,
    pub cancel: String,
}

struct Flat {
    id: String,
    label: String,
    project_id: Option<String>,
}

// Las carpetas anidan: un recorrido de un solo nivel se dejaba fuera las
// listas de una subcarpeta (`walk` en `move-targets.ts`).
fn walk(folder: &FolderTree, prefix: &str, out: &mut Vec<Flat>) {
    let here = if prefix.is_empty() { folder.name.clone() } else { format!("{prefix} · {}", folder.name) };
    for list in &folder.lists {
        out.push(Flat { id: list.id.clone(), label: format!("{here} · {}", list.name), project_id: list.project_id.clone() });
    }
    for inner in &folder.folders {
        walk(inner, &here, out);
    }
}

fn lists_of(space: &SpaceTree) -> Vec<Flat> {
    let mut out: Vec<Flat> =
        space.lists.iter().map(|list| Flat { id: list.id.clone(), label: list.name.clone(), project_id: list.project_id.clone() }).collect();
    for folder in &space.folders {
        walk(folder, "", &mut out);
    }
    out
}

/// El canal de una lista, o nada: uno vacío es «de nadie de fuera».
fn channel(project_id: Option<&str>) -> &str {
    project_id.unwrap_or("")
}

/// Las listas a las que se puede llevar esa tarea, agrupadas por espacio.
///
/// Lo que se ofrece es lo que el servidor acepta (`MoveTaskToList` en
/// `backend/internal/core/repository/item_migration.go`), para que el menú no
/// proponga algo que luego conteste con un 409:
///
/// - **una subtarea, ninguna**: vive en la lista de su madre y viaja con
///   ella (`ErrMoveSubtask`);
/// - **misma organización**: cruzarla enseñaría la tarjeta a quien no ve de
///   dónde viene (`ErrChannelOtherOrg`);
/// - **una tarjeta de cliente sólo entre listas de su canal**: gastó un folio
///   de ese cliente (`ErrMoveLeavesChannel`). Una interna, a cualquiera.
///
/// La lista en la que está sale también, marcada: es donde se lee en cuál vive.
pub fn move_targets(spaces: &[SpaceTree], detail: &TaskDetail) -> Vec<MoveGroup> {
    let task = &detail.task;
    if task.parent_id.as_deref().is_some_and(|parent| !parent.is_empty()) {
        return Vec::new();
    }
    // El espacio de la tarea lo dice ella; un servidor que no lo mande, el
    // árbol, por la lista en la que está.
    let home = Some(task.space_id.clone())
        .filter(|space| !space.is_empty())
        .or_else(|| spaces.iter().find(|space| lists_of(space).iter().any(|list| list.id == task.list_id)).map(|space| space.id.clone()));
    let mut groups = Vec::new();
    for space in spaces {
        if !task.org_id.is_empty() && space.org_id != task.org_id {
            continue;
        }
        let targets: Vec<MoveTarget> = lists_of(space)
            .into_iter()
            .filter(|list| task.project_id.is_empty() || channel(list.project_id.as_deref()) == task.project_id)
            .map(|list| MoveTarget {
                current: list.id == task.list_id,
                list_id: list.id,
                label: list.label,
                // Una de cliente conserva su folio aunque cambie de espacio:
                // el número es del proyecto, no del espacio.
                crosses_space: task.project_id.is_empty() && home.as_deref().is_some_and(|home| home != space.id),
            })
            .collect();
        if !targets.is_empty() {
            groups.push(MoveGroup { space: space.name.clone(), targets });
        }
    }
    groups
}

/// Lo que se pregunta antes de llevar la tarea a esa lista, o `None` si no
/// hay nada que preguntar: no se le ofrece, o ya está en ella.
///
/// Siempre se pregunta (jose, 13-nov-2026, en `TaskDetailDrawer.tsx`):
/// elegir en un desplegable se hace sin querer, y mover saca la tarea de
/// donde se miraba. El título dice a cuál va, con el espacio delante cuando
/// es otro («App» puede haber una en cada uno); y a otro espacio, además, le
/// cambia la ref: eso es lo que dice el cuerpo.
pub fn move_question(locale: Locale, detail: &TaskDetail, groups: &[MoveGroup], list_id: &str) -> Option<Question> {
    let (group, target) =
        groups.iter().find_map(|group| group.targets.iter().find(|target| target.list_id == list_id).map(|target| (group, target)))?;
    if target.current {
        return None;
    }
    let (label, body) = if target.crosses_space {
        // Sin referencia (un servidor de antes), su número a secas.
        let reference = detail.reference.clone().filter(|reference| !reference.is_empty()).unwrap_or_else(|| format!("#{}", detail.task.seq));
        (format!("{} › {}", group.space, target.label), text_with(locale, "work:task.moveListRefChanges", &[("ref", &reference)]))
    } else {
        (target.label.clone(), text(locale, "work:task.moveListSameSpace"))
    };
    Some(Question {
        title: text_with(locale, "work:task.moveListTitle", &[("list", &label)]),
        body,
        confirm: text(locale, "work:task.moveListConfirm"),
        cancel: text(locale, "work:task.cancel"),
    })
}

impl Tasks {
    fn detail_or_refuse(&self) -> Result<TaskDetail, ApiError> {
        self.state().detail.ok_or_else(|| refused("no-task-open", "No task is open"))
    }

    /// Deja escrito dónde quedó la tarea abierta; en blanco lo quita.
    ///
    /// Se reemplaza entero, y el servidor lo firma con quién y cuándo: de su
    /// respuesta sale la firma que se enseña, no de aquí. Si dice lo mismo
    /// que ya decía no se pide nada (la app de siempre sí lo manda, y eso le
    /// cambia la fecha a un texto que nadie tocó). Uno más largo de lo que el
    /// servidor guarda no se manda.
    pub async fn set_handoff(&self, text: &str) -> Result<(), ApiError> {
        let text = text.trim();
        if text.chars().count() > MAX_HANDOFF {
            return Err(refused("handoff-too-long", "A handoff can be 2000 characters at most"));
        }
        let has = self.detail_or_refuse()?.task.handoff.unwrap_or_default();
        if has.trim() == text {
            return Ok(());
        }
        self.update(TaskPatch { handoff: Some(text.to_string()), ..TaskPatch::default() }).await
    }

    /// Las listas a las que se puede llevar la tarea abierta, con el árbol
    /// que haya cargado. Sin árbol no hay a dónde ofrecer, no un error.
    pub fn move_targets(&self) -> Vec<MoveGroup> {
        let spaces = self.core.tree().state().spaces;
        self.state().detail.map(|detail| move_targets(&spaces, &detail)).unwrap_or_default()
    }

    /// Lo que se pregunta antes de llevar la tarea abierta a esa lista.
    pub fn move_question(&self, locale: Locale, list_id: &str) -> Option<Question> {
        let detail = self.state().detail?;
        move_question(locale, &detail, &self.move_targets(), list_id)
    }

    /// Lleva la tarea abierta a otra lista: `PATCH` con `listId`, y la
    /// respuesta es la tarea en su sitio nuevo (su lista, su espacio y, si
    /// cruzó de espacio siendo interna, su ref nueva).
    ///
    /// Lo que el servidor contestaría con un 409 no se le pide, y se rechaza
    /// con su mismo código para que se diga con su misma frase: una subtarea,
    /// o la tarjeta de un cliente a una lista que no es de su canal. A una
    /// lista que el árbol no conoce se pide igual, y decide él. A la lista en
    /// la que ya está, nada.
    pub async fn move_to_list(&self, list_id: &str) -> Result<(), ApiError> {
        let detail = self.detail_or_refuse()?;
        let task = &detail.task;
        if task.list_id == list_id {
            return Ok(());
        }
        if task.parent_id.as_deref().is_some_and(|parent| !parent.is_empty()) {
            return Err(refused("move-subtask", "A subtask lives in its parent's list"));
        }
        let spaces = self.core.tree().state().spaces;
        let known = spaces.iter().find_map(|space| lists_of(space).into_iter().find(|list| list.id == list_id).map(|list| (space, list)));
        if let Some((space, list)) = known {
            if !task.org_id.is_empty() && space.org_id != task.org_id {
                return Err(refused("channel-other-org", "That list belongs to another organization"));
            }
            if !task.project_id.is_empty() && channel(list.project_id.as_deref()) != task.project_id {
                return Err(refused("move-leaves-channel", "A client's card can only move to another list of that same client"));
            }
        }
        self.update(TaskPatch { list_id: Some(list_id.to_string()), ..TaskPatch::default() }).await
    }
}
