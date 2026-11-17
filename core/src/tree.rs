//! El árbol de espacios, carpetas y listas de una organización.
//!
//! Vino de `fetchTree` en `app/src/store/tasks.store.ts` y de los tipos de
//! `app/src/types/task.ts`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::nav::NodeKind;
use crate::Core;

/// Una lista de tareas, con sus cuentas. Espejo de `domain.ListSummary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ListSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub open_count: i64,
    /// Si la lista es el canal de un cliente, cuál: lo ve alguien de fuera.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub task_count: i64,
}

/// Una carpeta: listas y más carpetas. Espejo de `domain.FolderTree`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct FolderTree {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub folders: Vec<FolderTree>,
    #[serde(default)]
    pub lists: Vec<ListSummary>,
}

/// Quién está en un espacio. Espejo de `domain.SpacePerson`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct SpacePerson {
    pub user_id: String,
    /// Para buscar, mencionar e identificar.
    pub username: String,
    /// Para enseñar. Puede faltar: quien pinte cae al usuario.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Un espacio entero. Espejo de `domain.SpaceTree`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct SpaceTree {
    pub id: String,
    /// La clave con la que se nombran sus tareas (`command-and-control`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub org_id: String,
    pub name: String,
    #[serde(default)]
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// `general` es la sala general de la organización: un canal, sin tareas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default)]
    pub folders: Vec<FolderTree>,
    #[serde(default)]
    pub lists: Vec<ListSummary>,
    #[serde(default)]
    pub people: Vec<SpacePerson>,
}

impl SpaceTree {
    /// La sala general no sale en el árbol: es un canal, no tiene tareas.
    pub fn is_general(&self) -> bool {
        self.kind.as_deref() == Some("general")
    }
}

/// Lo que se sabe del documento de un nodo sin cargarlo. Espejo de
/// `domain.DocMark`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocMark {
    /// Si hay algo escrito. Una portada vacía con páginas debajo **está**
    /// escrita: lo decide el servidor.
    #[serde(default)]
    pub written: bool,
    /// La línea que el documento fija sobre el tablero de su lista.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_line: Option<String>,
    /// Lleva demasiado sin que nadie lo confirme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintainer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintainer_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<i64>,
}

/// Una carpeta del árbol, cuelgue de donde cuelgue.
pub fn find_folder<'a>(spaces: &'a [SpaceTree], folder_id: &str) -> Option<&'a FolderTree> {
    fn within<'a>(folders: &'a [FolderTree], folder_id: &str) -> Option<&'a FolderTree> {
        folders.iter().find_map(|folder| {
            if folder.id == folder_id {
                return Some(folder);
            }
            within(&folder.folders, folder_id)
        })
    }
    spaces.iter().find_map(|space| within(&space.folders, folder_id))
}

/// Las listas que cuelgan de una carpeta, también las de sus subcarpetas a
/// cualquier profundidad. `None` si la carpeta no está en el árbol, que no es
/// lo mismo que estar vacía.
///
/// Es lo que quiere decir «las tareas de una carpeta»: una tarea sabe su
/// lista y su espacio, no su carpeta.
pub fn folder_list_ids(spaces: &[SpaceTree], folder_id: &str) -> Option<HashSet<String>> {
    fn collect(folder: &FolderTree, out: &mut HashSet<String>) {
        out.extend(folder.lists.iter().map(|list| list.id.clone()));
        for inner in &folder.folders {
            collect(inner, out);
        }
    }
    let folder = find_folder(spaces, folder_id)?;
    let mut ids = HashSet::new();
    collect(folder, &mut ids);
    Some(ids)
}

/// Con qué clave nombra el servidor el documento de un nodo: `list:<id>`.
pub fn doc_key(kind: NodeKind, id: &str) -> String {
    let kind = match kind {
        NodeKind::Space => "space",
        NodeKind::Folder => "folder",
        NodeKind::List => "list",
    };
    format!("{kind}:{id}")
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TreeState {
    /// Los espacios que se pintan: sin la sala general.
    pub spaces: Vec<SpaceTree>,
    /// De qué organización es lo cargado.
    pub org_id: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    /// Qué nodos tienen documento, por su clave (`doc_key`).
    #[serde(default)]
    pub doc_marks: HashMap<String, DocMark>,
    /// Lo que alguien abrió (`true`) o cerró (`false`) a mano en esta
    /// organización, por id de nodo. Lo demás sigue la regla de `tree_rows`.
    #[serde(default)]
    pub decided: HashMap<String, bool>,
}

impl TreeState {
    /// La marca del documento de un nodo, si tiene algo escrito.
    pub fn written_doc(&self, kind: NodeKind, id: &str) -> Option<&DocMark> {
        self.doc_marks.get(&doc_key(kind, id)).filter(|mark| mark.written)
    }
}

/// El árbol de este core.
pub struct Tree {
    pub(crate) core: Core,
}

impl Core {
    pub fn tree(&self) -> Tree {
        Tree { core: self.clone() }
    }
}

impl Tree {
    pub fn state(&self) -> TreeState {
        self.core.inner.tree.lock().expect("tree lock").clone()
    }

    pub(crate) fn update(&self, change: impl FnOnce(&mut TreeState)) {
        let state = {
            let mut state = self.core.inner.tree.lock().expect("tree lock");
            change(&mut state);
            state.clone()
        };
        self.core.inner.listener.tree_changed(state);
    }

    pub async fn load(&self, org_id: &str) {
        let decided = self.decided_for(org_id);
        self.update(|state| {
            state.decided = decided;
            // Al cambiar de organización, el árbol de la anterior no se
            // enseña mientras llega el nuevo: sus listas no existen aquí, y
            // un clic en una acotaría «Mi trabajo» a algo de otra organización.
            if state.org_id.as_deref() != Some(org_id) {
                state.spaces = Vec::new();
                state.doc_marks = HashMap::new();
            }
            state.org_id = Some(org_id.to_string());
            state.loading = true;
            state.error = None;
        });
        let path = format!("/api/v1/task-spaces/?orgId={org_id}");
        match self.core.get::<Vec<SpaceTree>>(&path).await {
            Ok(spaces) => {
                // Antes de quitar la sala general: no se pinta en el árbol,
                // pero es un canal y lo que tenga sin leer cuenta.
                self.core.counters().spaces_loaded(org_id, spaces.iter().map(|space| space.id.clone()).collect());
                self.update(|state| {
                    state.spaces = spaces.into_iter().filter(|space| !space.is_general()).collect();
                    state.loading = false;
                });
                // Un ámbito de carpeta se lee de este árbol: con uno nuevo
                // hay que volver a mirarlo.
                self.core.my_work().tree_reloaded();
            }
            Err(error) => self.update(|state| {
                state.error = Some(error.to_string());
                state.loading = false;
            }),
        }
    }

    /// Trae qué nodos tienen documento.
    ///
    /// La marca es un adorno: si falla, el árbol sigue como estaba y sin
    /// error a la vista. Y la respuesta de una organización que ya no es la
    /// que se mira no se pone.
    pub async fn load_doc_marks(&self, org_id: &str) {
        let path = format!("/api/v1/docs/?orgId={org_id}");
        let Ok(marks) = self.core.get::<HashMap<String, DocMark>>(&path).await else {
            return;
        };
        if self.state().org_id.as_deref() != Some(org_id) {
            return;
        }
        self.update(|state| state.doc_marks = marks);
    }
}
