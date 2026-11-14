//! El árbol de espacios, carpetas y listas de una organización.
//!
//! Vino de `fetchTree` en `app/src/store/tasks.store.ts` y de los tipos de
//! `app/src/types/task.ts`.

use serde::{Deserialize, Serialize};

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
}

/// El árbol de este core.
pub struct Tree {
    core: Core,
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

    fn update(&self, change: impl FnOnce(&mut TreeState)) {
        let state = {
            let mut state = self.core.inner.tree.lock().expect("tree lock");
            change(&mut state);
            state.clone()
        };
        self.core.inner.listener.tree_changed(state);
    }

    pub async fn load(&self, org_id: &str) {
        self.update(|state| {
            // Al cambiar de organización, el árbol de la anterior no se
            // enseña mientras llega el nuevo: sus listas no existen aquí, y
            // un clic en una acotaría «Mi trabajo» a algo de otra organización.
            if state.org_id.as_deref() != Some(org_id) {
                state.spaces = Vec::new();
            }
            state.org_id = Some(org_id.to_string());
            state.loading = true;
            state.error = None;
        });
        let path = format!("/api/v1/task-spaces/?orgId={org_id}");
        match self.core.get::<Vec<SpaceTree>>(&path).await {
            Ok(spaces) => self.update(|state| {
                state.spaces = spaces.into_iter().filter(|space| !space.is_general()).collect();
                state.loading = false;
            }),
            Err(error) => self.update(|state| {
                state.error = Some(error.to_string());
                state.loading = false;
            }),
        }
    }
}
