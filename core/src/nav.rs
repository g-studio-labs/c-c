//! Qué hace un clic en un nodo del árbol.
//!
//! Una regla, escrita una vez (palabras de jose, 14-nov-2026): **pulsar un
//! nodo sólo cambia de qué nodo se habla, y deja a quien mira en la vista en
//! la que esté**. Quien está leyendo documentación ve la documentación de ese
//! nodo; quien está en las tareas ve las tareas de ese nodo.
//!
//! Estaba repartida entre tres manejadores de clic de la app de Tauri
//! (`SpacesNavigator.tsx`) y `lib/doc-mode.ts`, y de ahí salió
//! command-and-control-372: desde las tareas de un espacio no había cómo
//! llegar a su documentación. Aquí es una función pura, y cada interfaz la
//! pregunta en vez de decidirlo.

use crate::mywork::ScopeKind;

/// Lo que está mirando quien pulsa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Looking {
    /// Tareas, en cualquiera de sus vistas (lista, tablero, calendario).
    Tasks,
    /// La documentación de un nodo.
    Docs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum NodeKind {
    Space,
    Folder,
    List,
}

/// A dónde lleva el clic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TreeClick {
    /// Acotar las tareas a este nodo, sin cambiar de vista.
    ScopeTasks { kind: ScopeKind },
    /// Abrir la documentación de este nodo.
    OpenDocs,
}

/// La regla.
///
/// Una carpeta no acota: «Mi trabajo» se acota a un espacio o a una lista, y
/// una carpeta no es ninguna de las dos. Su clic abre siempre su
/// documentación, que es lo único que una carpeta tiene propio.
pub fn tree_click(looking: Looking, node: NodeKind) -> TreeClick {
    match (looking, node) {
        (Looking::Docs, _) | (_, NodeKind::Folder) => TreeClick::OpenDocs,
        (Looking::Tasks, NodeKind::Space) => TreeClick::ScopeTasks { kind: ScopeKind::Space },
        (Looking::Tasks, NodeKind::List) => TreeClick::ScopeTasks { kind: ScopeKind::List },
    }
}

/// Si con este ámbito hay documentación a un clic.
///
/// La hay con cualquier nodo delante, sea lista o espacio (372): la
/// documentación cuelga de un nodo. Sin acotar no, porque «Mi trabajo» entero
/// cruza todos los nodos y no hay uno del que colgar.
pub fn docs_reachable_from_scope(scope: Option<ScopeKind>) -> bool {
    scope.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que atrapa: sacar de la documentación a quien la está leyendo
    /// (lo de antes del 9-oct-2026: había que volver a pulsar «Documentación»
    /// en cada lista).
    #[test]
    fn from_docs_a_click_opens_that_nodes_docs() {
        for node in [NodeKind::Space, NodeKind::Folder, NodeKind::List] {
            assert_eq!(tree_click(Looking::Docs, node), TreeClick::OpenDocs);
        }
    }

    /// Rotura que atrapa: que un espacio abra su documentación desde las
    /// tareas (lo de antes del 13-nov-2026: no había forma de ver las tareas
    /// de un espacio), o acotar un espacio como si fuera una lista.
    #[test]
    fn from_tasks_a_click_scopes_to_that_node() {
        assert_eq!(
            tree_click(Looking::Tasks, NodeKind::Space),
            TreeClick::ScopeTasks { kind: ScopeKind::Space }
        );
        assert_eq!(
            tree_click(Looking::Tasks, NodeKind::List),
            TreeClick::ScopeTasks { kind: ScopeKind::List }
        );
    }

    /// Rotura que atrapa: intentar acotar a una carpeta, que dejaría la
    /// pantalla filtrando por algo que ninguna tarea tiene.
    #[test]
    fn a_folder_always_opens_its_docs() {
        assert_eq!(tree_click(Looking::Tasks, NodeKind::Folder), TreeClick::OpenDocs);
    }

    /// El requisito de command-and-control-372. Rotura que atrapa: ofrecer la
    /// documentación sólo con una lista delante.
    #[test]
    fn docs_are_one_click_away_from_any_scoped_node() {
        assert!(docs_reachable_from_scope(Some(ScopeKind::Space)));
        assert!(docs_reachable_from_scope(Some(ScopeKind::List)));
        assert!(!docs_reachable_from_scope(None));
    }
}
