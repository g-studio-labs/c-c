//! El árbol tal como se pinta: una fila por nodo visible, ya con su sangría.
//!
//! Qué ramas están abiertas es una regla, y vino repartida entre
//! `SpacesNavigator.tsx` (`openByDefault`) y `useTreeExpansion.ts`:
//!
//! - lo que alguien abrió o cerró **a mano** manda, y se recuerda por
//!   organización;
//! - si no decidió nada, va abierta la rama de lo que se está mirando (el
//!   camino hasta el nodo al que «Mi trabajo» está acotado), y sólo esa: con
//!   veinte espacios desplegados lo que se busca queda tres pantallas abajo;
//! - sin nada acotado, los espacios abiertos y las carpetas cerradas;
//! - con un filtro por título puesto no manda nada de eso: se enseña lo que
//!   casa, con su camino abierto (vino de `filterTree`, en
//!   `app/src/lib/tree-path.ts`).

use std::collections::{HashMap, HashSet};

use crate::mywork::filter::fold;
use crate::mywork::{ScopeKind, WorkScope};
use crate::nav::{tree_click, Looking, NodeKind, TreeClick};
use crate::tree::{FolderTree, SpaceTree, Tree, TreeState};

/// La clave bajo la que se guarda lo abierto y cerrado a mano.
const STORAGE_KEY: &str = "tree-open";

/// Una fila del árbol.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TreeRow {
    pub kind: NodeKind,
    pub id: String,
    pub name: String,
    /// Cuánto cuelga: 0 es un espacio.
    pub depth: u32,
    /// Si tiene algo dentro que abrir (una lista no).
    pub expandable: bool,
    pub open: bool,
    /// Si «Mi trabajo» está acotado a este nodo.
    pub active: bool,
    /// Si su documento tiene algo escrito.
    pub has_doc: bool,
    /// Las tareas abiertas de una lista; de un espacio o una carpeta, las de
    /// todo lo que cuelga.
    pub open_count: i64,
    /// Si lo ve un cliente: la lista o el espacio es su canal.
    pub client: bool,
    /// El color que alguien le puso al espacio del que cuelga; vacío si
    /// nadie le puso ninguno, que es lo normal.
    pub color: String,
    /// El tono que le toca a ese espacio cuando no tiene color propio
    /// (`hue_index` de su id): el mismo que en la app de siempre.
    pub hue: u32,
}

/// Los ids del camino hasta el nodo acotado, él incluido. `None` si no hay
/// nada acotado o no es de este árbol.
fn active_path(spaces: &[SpaceTree], scope: Option<&WorkScope>) -> Option<HashSet<String>> {
    let scope = scope?;
    fn in_folder(folder: &FolderTree, list_id: &str, path: &mut Vec<String>) -> bool {
        path.push(folder.id.clone());
        if folder.lists.iter().any(|list| list.id == list_id)
            || folder.folders.iter().any(|inner| in_folder(inner, list_id, path))
        {
            return true;
        }
        path.pop();
        false
    }
    for space in spaces {
        let mut path = vec![space.id.clone()];
        let found = match scope.kind {
            ScopeKind::Space => space.id == scope.id,
            ScopeKind::List => {
                space.lists.iter().any(|list| list.id == scope.id)
                    || space.folders.iter().any(|folder| in_folder(folder, &scope.id, &mut path))
            }
        };
        if found {
            path.push(scope.id.clone());
            return Some(path.into_iter().collect());
        }
    }
    None
}

fn folder_open_count(folder: &FolderTree) -> i64 {
    folder.lists.iter().map(|list| list.open_count).sum::<i64>()
        + folder.folders.iter().map(folder_open_count).sum::<i64>()
}

/// Qué nodos se enseñan con ese filtro por título: los que casan, sus
/// antecesores (para que se vea de dónde cuelgan) y lo que tienen dentro
/// (buscar un espacio por su nombre enseña sus listas). `None` sin filtro:
/// todo.
///
/// Sin distinguir mayúsculas ni acentos, como el buscador de «Mi trabajo»
/// (`fold`): «operacion» encuentra «Operación». Filtra lo que ya está en
/// pantalla, no es el buscador.
pub fn filter_tree(spaces: &[SpaceTree], query: &str) -> Option<HashSet<String>> {
    let query = fold(query.trim());
    if query.is_empty() {
        return None;
    }
    let matches = |name: &str| fold(name).contains(&query);
    let mut visible = HashSet::new();

    fn everything_in(folder: &FolderTree, visible: &mut HashSet<String>) {
        visible.insert(folder.id.clone());
        visible.extend(folder.lists.iter().map(|list| list.id.clone()));
        for inner in &folder.folders {
            everything_in(inner, visible);
        }
    }
    /// `above` es el camino hasta la carpeta, sin ella.
    fn walk(folder: &FolderTree, above: &mut Vec<String>, matches: &dyn Fn(&str) -> bool, visible: &mut HashSet<String>) {
        if matches(&folder.name) {
            visible.extend(above.iter().cloned());
            everything_in(folder, visible);
        }
        above.push(folder.id.clone());
        for list in folder.lists.iter().filter(|list| matches(&list.name)) {
            visible.extend(above.iter().cloned());
            visible.insert(list.id.clone());
        }
        for inner in &folder.folders {
            walk(inner, above, matches, visible);
        }
        above.pop();
    }

    for space in spaces {
        if matches(&space.name) {
            visible.insert(space.id.clone());
            visible.extend(space.lists.iter().map(|list| list.id.clone()));
            for folder in &space.folders {
                everything_in(folder, &mut visible);
            }
        }
        let mut above = vec![space.id.clone()];
        for list in space.lists.iter().filter(|list| matches(&list.name)) {
            visible.insert(space.id.clone());
            visible.insert(list.id.clone());
        }
        for folder in &space.folders {
            walk(folder, &mut above, &matches, &mut visible);
        }
    }
    Some(visible)
}

struct Walk<'a> {
    state: &'a TreeState,
    scope: Option<&'a WorkScope>,
    path: Option<HashSet<String>>,
    /// Con un filtro puesto, lo que se enseña.
    visible: Option<HashSet<String>>,
    rows: Vec<TreeRow>,
}

impl Walk<'_> {
    fn shows(&self, id: &str) -> bool {
        self.visible.as_ref().is_none_or(|visible| visible.contains(id))
    }

    fn is_open(&self, id: &str, kind: NodeKind) -> bool {
        // Filtrando va todo abierto: lo que casa está dentro, y plegado no
        // se vería.
        if self.visible.is_some() {
            return true;
        }
        if let Some(decided) = self.state.decided.get(id) {
            return *decided;
        }
        match &self.path {
            Some(path) => path.contains(id),
            None => kind == NodeKind::Space,
        }
    }

    /// Filtrando no se pliega nada, así que no se ofrece: un chevrón que no
    /// hace nada no dice por qué.
    fn can_fold(&self) -> bool {
        self.visible.is_none()
    }

    fn is_active(&self, id: &str, kind: ScopeKind) -> bool {
        self.scope.is_some_and(|scope| scope.kind == kind && scope.id == id)
    }

    fn folder(&mut self, folder: &FolderTree, depth: u32, color: &str, hue: u32) {
        if !self.shows(&folder.id) {
            return;
        }
        let open = self.is_open(&folder.id, NodeKind::Folder);
        self.rows.push(TreeRow {
            kind: NodeKind::Folder,
            id: folder.id.clone(),
            name: folder.name.clone(),
            depth,
            expandable: self.can_fold() && (!folder.folders.is_empty() || !folder.lists.is_empty()),
            open,
            active: false,
            has_doc: self.state.written_doc(NodeKind::Folder, &folder.id).is_some(),
            open_count: folder_open_count(folder),
            client: false,
            color: color.to_string(),
                hue,
        });
        if open {
            self.children(&folder.folders, &folder.lists, depth + 1, color, hue);
        }
    }

    /// Las carpetas primero y las listas después, como las da el servidor.
    fn children(&mut self, folders: &[FolderTree], lists: &[crate::tree::ListSummary], depth: u32, color: &str, hue: u32) {
        for folder in folders {
            self.folder(folder, depth, color, hue);
        }
        for list in lists {
            if !self.shows(&list.id) {
                continue;
            }
            self.rows.push(TreeRow {
                kind: NodeKind::List,
                id: list.id.clone(),
                name: list.name.clone(),
                depth,
                expandable: false,
                open: false,
                active: self.is_active(&list.id, ScopeKind::List),
                has_doc: self.state.written_doc(NodeKind::List, &list.id).is_some(),
                open_count: list.open_count,
                client: list.project_id.is_some(),
                color: color.to_string(),
                hue,
            });
        }
    }
}

/// El árbol aplanado, con lo que está abierto.
pub fn tree_rows(state: &TreeState, scope: Option<&WorkScope>) -> Vec<TreeRow> {
    tree_rows_matching(state, scope, "")
}

/// El árbol aplanado con un filtro por título: sólo lo que casa, su camino
/// y lo que tiene dentro, todo abierto. Con el filtro en blanco, el árbol de
/// siempre.
pub fn tree_rows_matching(state: &TreeState, scope: Option<&WorkScope>, query: &str) -> Vec<TreeRow> {
    let mut walk = Walk {
        state,
        scope,
        path: active_path(&state.spaces, scope),
        visible: filter_tree(&state.spaces, query),
        rows: Vec::new(),
    };
    for space in &state.spaces {
        if !walk.shows(&space.id) {
            continue;
        }
        let open = walk.is_open(&space.id, NodeKind::Space);
        walk.rows.push(TreeRow {
            kind: NodeKind::Space,
            id: space.id.clone(),
            name: space.name.clone(),
            depth: 0,
            expandable: walk.can_fold() && (!space.folders.is_empty() || !space.lists.is_empty()),
            open,
            active: walk.is_active(&space.id, ScopeKind::Space),
            has_doc: state.written_doc(NodeKind::Space, &space.id).is_some(),
            open_count: space.lists.iter().map(|list| list.open_count).sum::<i64>()
                + space.folders.iter().map(folder_open_count).sum::<i64>(),
            client: space.project_id.is_some(),
            color: space.color.clone(),
            hue: crate::hue::hue_index(&space.id),
        });
        if open {
            walk.children(&space.folders, &space.lists, 1, &space.color, crate::hue::hue_index(&space.id));
        }
    }
    walk.rows
}

/// Lo decidido a mano, por organización.
type Decided = HashMap<String, HashMap<String, bool>>;

impl Tree {
    fn scope(&self) -> Option<WorkScope> {
        self.core.my_work().state().scope
    }

    /// Las filas que se pintan ahora.
    pub fn rows(&self) -> Vec<TreeRow> {
        tree_rows(&self.state(), self.scope().as_ref())
    }

    /// Las filas que se pintan con ese filtro por título puesto.
    pub fn rows_matching(&self, query: &str) -> Vec<TreeRow> {
        tree_rows_matching(&self.state(), self.scope().as_ref(), query)
    }

    /// Lo que se decidió a mano en una organización, de lo guardado.
    pub(crate) fn decided_for(&self, org_id: &str) -> HashMap<String, bool> {
        self.saved().remove(org_id).unwrap_or_default()
    }

    fn saved(&self) -> Decided {
        self.core
            .inner
            .storage
            .get(STORAGE_KEY)
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Abre lo cerrado y cierra lo abierto, y lo recuerda para esta
    /// organización. Una lista, o un nodo que no está, no hace nada.
    pub fn toggle(&self, id: &str) {
        let Some(row) = self.rows().into_iter().find(|row| row.id == id && row.kind != NodeKind::List) else {
            return;
        };
        self.update(|state| {
            state.decided.insert(row.id.clone(), !row.open);
        });
        let state = self.state();
        let Some(org_id) = state.org_id else { return };
        let mut saved = self.saved();
        saved.insert(org_id, state.decided);
        if let Ok(raw) = serde_json::to_string(&saved) {
            let _ = self.core.inner.storage.set(STORAGE_KEY, &raw);
        }
    }

    /// Un clic en un nodo: aplica la regla de `tree_click` y, si toca acotar,
    /// acota «Mi trabajo» a ese nodo. Devuelve qué hizo, para que la pantalla
    /// abra la documentación cuando eso es lo que toca.
    pub fn click(&self, id: &str, looking: Looking) -> Option<TreeClick> {
        let state = self.state();
        // En el árbol entero, no en las filas: puede estar bajo una rama
        // plegada (se llega a él desde una tarea, no sólo desde el árbol).
        let (kind, name) = find_node(&state.spaces, id)?;
        let click = tree_click(looking, kind);
        if let TreeClick::ScopeTasks { kind } = click {
            self.core.my_work().set_scope(Some(WorkScope { kind, id: id.to_string(), name, org_id: state.org_id }));
            // El árbol pinta cuál está activo: quien lo observa tiene que
            // enterarse aunque el árbol en sí no haya cambiado.
            self.update(|_| {});
        }
        Some(click)
    }
}

/// Qué es un nodo y cómo se llama, esté donde esté del árbol.
pub(crate) fn find_node(spaces: &[SpaceTree], id: &str) -> Option<(NodeKind, String)> {
    fn in_folder(folder: &FolderTree, id: &str) -> Option<(NodeKind, String)> {
        if folder.id == id {
            return Some((NodeKind::Folder, folder.name.clone()));
        }
        if let Some(list) = folder.lists.iter().find(|list| list.id == id) {
            return Some((NodeKind::List, list.name.clone()));
        }
        folder.folders.iter().find_map(|inner| in_folder(inner, id))
    }
    spaces.iter().find_map(|space| {
        if space.id == id {
            return Some((NodeKind::Space, space.name.clone()));
        }
        if let Some(list) = space.lists.iter().find(|list| list.id == id) {
            return Some((NodeKind::List, list.name.clone()));
        }
        space.folders.iter().find_map(|folder| in_folder(folder, id))
    })
}
