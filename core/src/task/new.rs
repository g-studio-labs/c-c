//! Levantar una tarea: dónde puede nacer y con qué.
//!
//! Vino de `app/src/components/tasks/NewTaskRow.tsx`. Todo se decide antes de
//! que exista: crear primero y editar después la pondría un momento en el
//! tablero de todos sin responsable ni fecha, y eso lo ve gente que puede
//! actuar sobre ella mientras está mal.

use super::edit::shown_name;
use super::{Choice, NewTask, Task, Tasks, UserSummary};
use crate::mywork::{ScopeKind, WorkScope};
use crate::tree::{find_folder, FolderTree, SpaceTree};
use crate::ApiError;

/// Una lista donde puede nacer una tarea.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Destination {
    pub list_id: String,
    /// Con el camino entero (`Espacio / Carpeta / Lista`): casi todas las
    /// listas se llaman igual, y el camino es lo que las distingue.
    pub label: String,
    /// Si es la lista de un cliente: lo que nazca ahí lo lee alguien de fuera
    /// salvo que se diga que no.
    pub client: bool,
}

/// Dónde puede nacer una tarea desde lo que se está mirando.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Destinations {
    pub options: Vec<Destination>,
    /// Se está mirando una lista: la tarea va ahí, y se dice en vez de
    /// preguntarse.
    pub fixed: bool,
}

impl Destinations {
    /// La lista en la que nacería: la pedida si se ofrece, y si no la
    /// primera. Al cambiar de vista, la de antes puede no estar ya: quedarse
    /// con ella es crear en una lista que no se está mirando.
    pub fn chosen(&self, wanted: &str) -> Option<&Destination> {
        self.options.iter().find(|option| option.list_id == wanted).or_else(|| self.options.first())
    }
}

/// `only` es la carpeta a la vista, si la hay: hasta llegar a ella no se
/// ofrece nada, y de ella para abajo todo. El camino se sigue llevando desde
/// el espacio, se ofrezca o no lo de en medio.
fn folder_lists(folders: &[FolderTree], prefix: &str, only: Option<&str>, out: &mut Vec<Destination>) {
    for folder in folders {
        let here = format!("{prefix} / {}", folder.name);
        let only = only.filter(|only| *only != folder.id);
        if only.is_none() {
            for list in &folder.lists {
                out.push(Destination {
                    list_id: list.id.clone(),
                    label: format!("{here} / {}", list.name),
                    client: list.project_id.is_some(),
                });
            }
        }
        folder_lists(&folder.folders, &here, only, out);
    }
}

/// Las listas de esa organización donde puede nacer una tarea.
///
/// Con una lista a la vista, sólo ésa; con un espacio, las suyas; con una
/// carpeta, las que cuelgan de ella a cualquier profundidad (una carpeta sin
/// listas no ofrece ninguna: lo que se crease en otra no saldría en lo que
/// se está mirando). Si la lista o la carpeta a la vista no está en el
/// árbol, se ofrecen todas: no hay a cuál fijarla.
pub fn destinations(spaces: &[SpaceTree], org_id: Option<&str>, scope: Option<&WorkScope>) -> Destinations {
    let folder = scope
        .filter(|scope| scope.kind == ScopeKind::Folder && find_folder(spaces, &scope.id).is_some())
        .map(|scope| scope.id.as_str());
    let mut options = Vec::new();
    for space in spaces {
        if org_id.is_some_and(|org_id| space.org_id != org_id) {
            continue;
        }
        if scope.is_some_and(|scope| scope.kind == ScopeKind::Space && scope.id != space.id) {
            continue;
        }
        // Las listas sueltas del espacio no cuelgan de ninguna carpeta.
        if folder.is_none() {
            for list in &space.lists {
                options.push(Destination {
                    list_id: list.id.clone(),
                    label: format!("{} / {}", space.name, list.name),
                    client: list.project_id.is_some(),
                });
            }
        }
        folder_lists(&space.folders, &space.name, folder, &mut options);
    }
    // Sin mirar si lo acotado es una lista: con un espacio a la vista sólo
    // quedan sus listas, y ninguna lleva el id de su espacio. Con una carpeta
    // sí se mira: no se fija a una lista por compartir id con su carpeta.
    if let Some(at) = scope.filter(|scope| scope.kind != ScopeKind::Folder).and_then(|scope| options.iter().position(|option| option.list_id == scope.id)) {
        return Destinations { options: vec![options.swap_remove(at)], fixed: true };
    }
    Destinations { options, fixed: false }
}

/// Lo que alguien escribió y eligió para una tarea que aún no existe.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TaskDraft {
    pub list_id: String,
    pub title: String,
    pub priority: String,
    /// `YYYY-MM-DD`.
    pub due_day: Option<String>,
    pub assignee_id: Option<String>,
    /// Que el cliente no la lea. Fuera de la lista de un cliente no cambia
    /// nada: ahí todo nace interno.
    pub internal: bool,
}

/// Lo que se le manda al servidor por ese borrador.
///
/// La visibilidad sólo va cuando alguien eligió «interna»: sin ella decide el
/// servidor, y así su regla (en la lista de un cliente, la ve) no se
/// contradice por accidente.
pub fn drafted(draft: &TaskDraft) -> Result<NewTask, ApiError> {
    let title = draft.title.trim();
    if title.is_empty() {
        return Err(super::refused("empty-title", "A task needs a title"));
    }
    if draft.list_id.is_empty() {
        return Err(super::refused("no-list", "Choose where the task goes"));
    }
    Ok(NewTask {
        title: title.to_string(),
        priority: Some(draft.priority.clone()).filter(|priority| !priority.is_empty()),
        due_day: draft.due_day.clone(),
        assignee_ids: draft.assignee_id.iter().cloned().collect(),
        visibility: draft.internal.then(|| "internal".to_string()),
        ..NewTask::default()
    })
}

impl Tasks {
    /// Dónde puede nacer una tarea ahora: según el árbol cargado, la
    /// organización que se mira y a qué está acotado «Mi trabajo».
    pub fn destinations(&self) -> Destinations {
        let tree = self.core.tree().state();
        let org_id = self.core.orgs().state().current_org_id;
        let scope = self.core.my_work().state().scope;
        destinations(&tree.spaces, org_id.as_deref(), scope.as_ref())
    }

    /// A quién se le puede dar una tarea nueva: el equipo de la organización
    /// que se mira, por nombre. Sin organización no se pregunta (el servidor
    /// leería «todas las mías»), y si no contesta no hay a quién ofrecer.
    pub async fn people_to_assign(&self) -> Vec<Choice> {
        let Some(org_id) = self.core.orgs().state().current_org_id.filter(|org_id| !org_id.is_empty()) else {
            return Vec::new();
        };
        let path = format!("/api/v1/users/search?orgId={org_id}&limit={}", super::edit::TEAM_LIMIT);
        let people = self.core.get::<Vec<UserSummary>>(&path).await.unwrap_or_default();
        let mut choices: Vec<Choice> =
            people.iter().map(|person| Choice { id: person.id.clone(), name: shown_name(person), chosen: false }).collect();
        choices.sort_by_key(|choice| choice.name.to_lowercase());
        choices
    }

    /// Crea la tarea del borrador. Un título en blanco o sin lista no se
    /// manda. Después se vuelve a pedir el árbol, que lleva la cuenta de
    /// tareas de cada lista.
    pub async fn raise(&self, draft: TaskDraft) -> Result<Task, ApiError> {
        let task = drafted(&draft)?;
        let created = self.create(&draft.list_id, task).await?;
        if let Some(org_id) = self.core.orgs().state().current_org_id {
            self.core.tree().load(&org_id).await;
        }
        Ok(created)
    }
}
