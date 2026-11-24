//! El módulo Documentación: su panel (el árbol de espacios con las páginas
//! de cada documento colgando de su nodo) y su pantalla de inicio (la tabla
//! de toda la documentación de la organización).
//!
//! Vino de `DocsPanel.tsx`, `DocsTree.tsx`, `lib/docs-panel.ts` y
//! `DocIndex.tsx`. El documento que se abra desde aquí se lee con lo de
//! siempre (`docs`): este módulo sólo decide qué se pinta alrededor.
//!
//! Qué se queda fuera, todavía: los `.md` de los repos de cada espacio
//! (`RepoDocsBranch`) y, en el árbol, crear, mover o tirar páginas.

use std::collections::{HashMap, HashSet};

use crate::doc_pages::DocPageItem;
use crate::docs::{kind_segment, Docs, DocsState};
use crate::nav::NodeKind;
use crate::texts::{text, Locale};
use crate::tree::{doc_key, DocMark, FolderTree, SpaceTree, TreeState};

/// Qué es una fila del árbol de Documentación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DocTreeItem {
    /// Un nodo del trabajo, con su documento.
    Node,
    /// La portada (las cuatro pestañas) del documento abierto.
    Home,
    /// Una página de un documento.
    Page,
}

/// Una fila del árbol de Documentación, lista para pintar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocTreeRow {
    pub item: DocTreeItem,
    /// El id del nodo, o el de la página. La portada lleva el de su nodo.
    pub id: String,
    /// El nodo de cuyo documento es la fila (el propio, en un nodo).
    pub owner_kind: NodeKind,
    pub owner_id: String,
    /// Ya con «Portada» o «Sin título» donde toca.
    pub title: String,
    pub depth: u32,
    /// Si tiene algo que desplegar: lleva chevrón.
    pub expandable: bool,
    pub open: bool,
    /// Es lo que se tiene delante: con fondo.
    pub active: bool,
    /// Lo que se tiene delante cuelga de aquí y se ve debajo: sólo en
    /// negrita, para que no haya dos filas con fondo una encima de otra.
    pub on_path: bool,
    /// Si su documento tiene algo escrito (un nodo).
    pub written: bool,
    /// El color del espacio (un espacio), como en el árbol de Trabajo.
    pub color: String,
    pub hue: u32,
}

/// Un nodo del trabajo con lo que cuelga de él, en el orden del árbol:
/// carpetas y luego listas (`docNodes`).
struct DocNode<'a> {
    kind: NodeKind,
    id: &'a str,
    name: &'a str,
    color: &'a str,
    children: Vec<DocNode<'a>>,
}

fn doc_nodes(spaces: &[SpaceTree]) -> Vec<DocNode<'_>> {
    fn folder(this: &FolderTree) -> DocNode<'_> {
        let mut children: Vec<DocNode> = this.folders.iter().map(folder).collect();
        children.extend(this.lists.iter().map(|list| DocNode { kind: NodeKind::List, id: &list.id, name: &list.name, color: "", children: Vec::new() }));
        DocNode { kind: NodeKind::Folder, id: &this.id, name: &this.name, color: "", children }
    }
    spaces
        .iter()
        .filter(|space| !space.is_general())
        .map(|space| {
            let mut children: Vec<DocNode> = space.folders.iter().map(folder).collect();
            children.extend(space.lists.iter().map(|list| DocNode { kind: NodeKind::List, id: &list.id, name: &list.name, color: "", children: Vec::new() }));
            DocNode { kind: NodeKind::Space, id: &space.id, name: &space.name, color: &space.color, children }
        })
        .collect()
}

/// Las claves desde el espacio hasta el nodo, él incluido; vacío si no está.
fn node_path(nodes: &[DocNode], kind: NodeKind, id: &str) -> Vec<String> {
    for node in nodes {
        if node.kind == kind && node.id == id {
            return vec![doc_key(node.kind, node.id)];
        }
        let below = node_path(&node.children, kind, id);
        if !below.is_empty() {
            let mut path = vec![doc_key(node.kind, node.id)];
            path.extend(below);
            return path;
        }
    }
    Vec::new()
}

/// La clave de una página en los mapas de plegado y de filtro: no choca con
/// la de un nodo (`doc_key`).
fn page_key(id: &str) -> String {
    format!("page:{id}")
}

fn shown_title(locale: Locale, title: &str) -> String {
    if title.trim().is_empty() {
        text(locale, "work:docs.untitled")
    } else {
        title.to_string()
    }
}

impl DocsState {
    /// Las páginas que se saben de un documento: las del abierto, o las que
    /// se pidieron al desplegarlo.
    fn known_pages(&self, kind: NodeKind, id: &str) -> Option<&[DocPageItem]> {
        let open = self.target.as_ref().is_some_and(|target| target.kind == kind && target.id == id);
        if open {
            return Some(&self.pages);
        }
        self.peeked.get(&doc_key(kind, id)).map(Vec::as_slice)
    }

    /// Lo que deja el filtro: las claves de los nodos y las páginas cuyo
    /// título casa, **con todos sus antepasados**, para que lo encontrado
    /// tenga dónde pintarse. `None` sin texto. Sólo mira las páginas que ya
    /// se tienen: filtrar no pide nada por cada documento del árbol.
    fn filter_keys(&self, nodes: &[DocNode], query: &str) -> Option<HashSet<String>> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return None;
        }
        let hit = |title: &str| title.to_lowercase().contains(&query);
        let mut keep = HashSet::new();
        fn pages_hit(pages: &[DocPageItem], parent: Option<&str>, chain: &mut Vec<String>, hit: &dyn Fn(&str) -> bool, keep: &mut HashSet<String>) -> bool {
            let mut any = false;
            for page in pages.iter().filter(|page| page.parent_id.as_deref() == parent) {
                // Un árbol que se muerde la cola no puede colgar a nadie.
                if chain.contains(&page_key(&page.id)) {
                    continue;
                }
                chain.push(page_key(&page.id));
                if hit(&page.title) {
                    any = true;
                    keep.extend(chain.iter().cloned());
                }
                if pages_hit(pages, Some(&page.id), chain, hit, keep) {
                    any = true;
                }
                chain.pop();
            }
            any
        }
        fn visit(state: &DocsState, node: &DocNode, hit: &dyn Fn(&str) -> bool, keep: &mut HashSet<String>) -> bool {
            let mut any = hit(node.name);
            if let Some(pages) = state.known_pages(node.kind, node.id) {
                any |= pages_hit(pages, None, &mut Vec::new(), hit, keep);
            }
            for child in &node.children {
                any |= visit(state, child, hit, keep);
            }
            if any {
                keep.insert(doc_key(node.kind, node.id));
            }
            any
        }
        for node in nodes {
            visit(self, node, &hit, &mut keep);
        }
        Some(keep)
    }

    /// Si un nodo va desplegado sin que nadie lo haya decidido: el camino
    /// hasta el documento abierto, él incluido. Nada más; un árbol con todo
    /// abierto es una lista de doscientas filas.
    fn open_by_default(&self, nodes: &[DocNode]) -> HashSet<String> {
        match &self.target {
            Some(target) => node_path(nodes, target.kind, &target.id).into_iter().collect(),
            None => HashSet::new(),
        }
    }

    /// El árbol de Documentación tal como se pinta, con ese filtro.
    ///
    /// Con el filtro puesto va desplegado todo lo que se ve: lo que se busca
    /// no puede quedar tras un chevrón. Y la portada no sale: no es un
    /// título que buscar.
    pub fn docs_tree_rows(&self, tree: &TreeState, query: &str, locale: Locale) -> Vec<DocTreeRow> {
        let nodes = doc_nodes(&tree.spaces);
        let visible = self.filter_keys(&nodes, query);
        let defaults = self.open_by_default(&nodes);
        let mut rows = Vec::new();
        let walk = Walk { state: self, marks: &tree.doc_marks, visible: visible.as_ref(), defaults: &defaults, locale };
        for node in nodes.iter().filter(|node| walk.shows(&doc_key(node.kind, node.id))) {
            walk.node(node, node, 0, &mut rows);
        }
        rows
    }
}

struct Walk<'a> {
    state: &'a DocsState,
    marks: &'a HashMap<String, DocMark>,
    visible: Option<&'a HashSet<String>>,
    defaults: &'a HashSet<String>,
    locale: Locale,
}

impl Walk<'_> {
    fn shows(&self, key: &str) -> bool {
        self.visible.is_none_or(|visible| visible.contains(key))
    }

    fn node(&self, space: &DocNode, node: &DocNode, depth: u32, rows: &mut Vec<DocTreeRow>) {
        let key = doc_key(node.kind, node.id);
        let state = self.state;
        let filtering = self.visible.is_some();
        let active = state.target.as_ref().is_some_and(|target| target.kind == node.kind && target.id == node.id);
        let mark = self.marks.get(&key);
        let peeked = state.peeked.get(&key).is_some_and(|pages| !pages.is_empty());
        let has_pages = active || mark.and_then(|mark| mark.pages).unwrap_or(0) > 0 || peeked;
        let children: Vec<&DocNode> = node.children.iter().filter(|child| self.shows(&doc_key(child.kind, child.id))).collect();
        let expandable = has_pages || !children.is_empty();
        let open = expandable && (filtering || state.node_folds.get(&key).copied().unwrap_or_else(|| self.defaults.contains(&key)));
        // Con su rama abierta, el fondo va en la portada o la página que se
        // lee, debajo; aquí sólo la negrita.
        let below = active && open && has_pages && !filtering;
        rows.push(DocTreeRow {
            item: DocTreeItem::Node,
            id: node.id.to_string(),
            owner_kind: node.kind,
            owner_id: node.id.to_string(),
            title: node.name.to_string(),
            depth,
            expandable,
            open,
            active: active && !below,
            on_path: below,
            written: mark.is_some_and(|mark| mark.written),
            color: space.color.to_string(),
            hue: crate::hue::hue_index(space.id),
        });
        if !open {
            return;
        }
        if has_pages {
            if active && !filtering {
                rows.push(DocTreeRow {
                    item: DocTreeItem::Home,
                    id: node.id.to_string(),
                    owner_kind: node.kind,
                    owner_id: node.id.to_string(),
                    title: text(self.locale, "work:docs.home"),
                    depth: depth + 1,
                    expandable: false,
                    open: false,
                    active: state.page_id.is_none(),
                    on_path: false,
                    written: false,
                    color: String::new(),
                    hue: 0,
                });
            }
            if let Some(pages) = state.known_pages(node.kind, node.id) {
                self.pages(node, pages, active, depth + 1, rows);
            }
        }
        for child in children {
            self.node(space, child, depth + 1, rows);
        }
    }

    fn pages(&self, owner: &DocNode, pages: &[DocPageItem], active: bool, depth: u32, rows: &mut Vec<DocTreeRow>) {
        let state = self.state;
        let filtering = self.visible.is_some();
        // Las madres de la página que se lee van abiertas, para que se vea
        // dónde está una página honda abierta por un enlace.
        let mut around: HashSet<&str> = HashSet::new();
        if active {
            let mut at = state.page_id.as_deref().and_then(|id| pages.iter().find(|page| page.id == id)).and_then(|page| page.parent_id.as_deref());
            while let Some(parent) = at {
                if !around.insert(parent) {
                    break;
                }
                at = pages.iter().find(|page| page.id == parent).and_then(|page| page.parent_id.as_deref());
            }
        }
        let children_of = |parent: Option<&str>| -> Vec<&DocPageItem> {
            pages.iter().filter(|page| page.parent_id.as_deref() == parent && self.shows(&page_key(&page.id))).collect()
        };
        // Una pila en vez de recursión: el árbol no tiene fondo.
        let mut pending: Vec<(&DocPageItem, u32)> = children_of(None).into_iter().rev().map(|page| (page, depth)).collect();
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some((page, depth)) = pending.pop() {
            if !seen.insert(page.id.as_str()) {
                continue;
            }
            let children = children_of(Some(page.id.as_str()));
            let open = !children.is_empty() && (filtering || state.page_folds.get(&page.id).copied().unwrap_or_else(|| around.contains(page.id.as_str())));
            rows.push(DocTreeRow {
                item: DocTreeItem::Page,
                id: page.id.clone(),
                owner_kind: owner.kind,
                owner_id: owner.id.to_string(),
                title: shown_title(self.locale, &page.title),
                depth,
                expandable: !children.is_empty(),
                open,
                active: active && state.page_id.as_deref() == Some(page.id.as_str()),
                on_path: false,
                written: false,
                color: String::new(),
                hue: 0,
            });
            if open {
                pending.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
            }
        }
    }
}

/// Una fila de la tabla de toda la documentación.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocIndexRow {
    pub kind: NodeKind,
    pub id: String,
    pub name: String,
    /// La ruta hasta él: `Portento › Backend`. Casi todas las listas se
    /// llaman igual («tasks», «bugs»), y el nombre suelto no identifica nada.
    pub path: String,
    /// Las tareas de una lista; nada en un espacio o una carpeta.
    pub tasks: Option<i64>,
    /// Quién responde de él; `None` si nadie, que es lo que hay que ver.
    pub owner: Option<String>,
    /// Cuándo se revisó, ya dicho, o «Sin revisar nunca».
    pub reviewed: String,
    /// Lleva demasiado sin que nadie lo confirme.
    pub stale: bool,
    /// Cuántas páginas cuelgan de su portada; nada si ninguna.
    pub pages: Option<i64>,
}

/// La tabla de toda la documentación de la organización.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocIndex {
    /// Lo documentado, lo más viejo arriba y lo que nunca se revisó primero
    /// de todo: lo que hace falta saber al abrir esto es qué está
    /// desactualizado, y un orden alfabético lo esconde.
    pub written: Vec<DocIndexRow>,
    /// Las listas sin documento, al pie: son las que hay que arreglar. Una
    /// carpeta o un espacio sin documento no es una tarea pendiente.
    pub undocumented: Vec<DocIndexRow>,
    /// Si hay algo documentado en la organización, al margen del filtro:
    /// «nada coincide» y «nada escrito» no son lo mismo.
    pub any_written: bool,
}

/// La tabla, con ese filtro (por nombre o por ruta) y, si se pide, sólo lo
/// que necesita atención: viejo o sin responsable.
pub fn doc_index(spaces: &[SpaceTree], marks: &HashMap<String, DocMark>, query: &str, attention_only: bool, locale: Locale) -> DocIndex {
    struct Entry<'a> {
        row: DocIndexRow,
        mark: Option<&'a DocMark>,
    }
    let reviewed = |mark: Option<&DocMark>| -> String {
        let at = mark.and_then(|mark| mark.reviewed_at.as_deref()).and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok());
        match at {
            Some(at) => crate::dates::short_date(locale, at.with_timezone(&chrono::Local).date_naive()),
            None => text(locale, "work:docs.neverReviewed"),
        }
    };
    let entry = |kind: NodeKind, id: &str, name: &str, path: String, tasks: Option<i64>| {
        let mark = marks.get(&doc_key(kind, id));
        Entry {
            row: DocIndexRow {
                kind,
                id: id.to_string(),
                name: name.to_string(),
                path,
                tasks,
                owner: mark.and_then(|mark| mark.maintainer_name.clone()).filter(|name| !name.trim().is_empty()),
                reviewed: reviewed(mark),
                stale: mark.and_then(|mark| mark.stale).unwrap_or(false),
                pages: mark.and_then(|mark| mark.pages).filter(|pages| *pages > 0),
            },
            mark,
        }
    };
    fn folder<'a>(this: &'a FolderTree, path: &str, out: &mut Vec<(NodeKind, &'a str, &'a str, String, Option<i64>)>) {
        let here = format!("{path} › {}", this.name);
        // Una carpeta sale con la ruta de quien la contiene.
        out.push((NodeKind::Folder, &this.id, &this.name, path.to_string(), None));
        for list in &this.lists {
            out.push((NodeKind::List, &list.id, &list.name, here.clone(), Some(list.task_count)));
        }
        for inner in &this.folders {
            folder(inner, &here, out);
        }
    }
    let mut found: Vec<(NodeKind, &str, &str, String, Option<i64>)> = Vec::new();
    for space in spaces.iter().filter(|space| !space.is_general()) {
        found.push((NodeKind::Space, &space.id, &space.name, "—".to_string(), None));
        for list in &space.lists {
            found.push((NodeKind::List, &list.id, &list.name, space.name.clone(), Some(list.task_count)));
        }
        for inner in &space.folders {
            folder(inner, &space.name, &mut found);
        }
    }
    let query = query.trim().to_lowercase();
    let entries: Vec<Entry> = found
        .into_iter()
        .map(|(kind, id, name, path, tasks)| entry(kind, id, name, path, tasks))
        // Espacios y carpetas sólo cuando tienen documento.
        .filter(|entry| entry.row.kind == NodeKind::List || entry.mark.is_some())
        .filter(|entry| query.is_empty() || entry.row.name.to_lowercase().contains(&query) || entry.row.path.to_lowercase().contains(&query))
        .collect();
    let mut written: Vec<&Entry> = entries
        .iter()
        .filter(|entry| entry.mark.is_some_and(|mark| !attention_only || mark.stale == Some(true) || mark.maintainer_id.as_deref().is_none_or(str::is_empty)))
        .collect();
    written.sort_by(|a, b| {
        let at = |entry: &Entry| entry.mark.and_then(|mark| mark.reviewed_at.clone()).filter(|at| !at.is_empty());
        match (at(a), at(b)) {
            (None, None) => a.row.path.to_lowercase().cmp(&b.row.path.to_lowercase()),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(a), Some(b)) => a.cmp(&b),
        }
    });
    let mut undocumented: Vec<&Entry> = entries.iter().filter(|entry| entry.mark.is_none()).collect();
    undocumented.sort_by(|a, b| a.row.path.to_lowercase().cmp(&b.row.path.to_lowercase()));
    DocIndex {
        written: written.into_iter().map(|entry| entry.row.clone()).collect(),
        undocumented: undocumented.into_iter().map(|entry| entry.row.clone()).collect(),
        any_written: !marks.is_empty(),
    }
}

impl Docs {
    /// Entrar a Documentación. Sólo un módulo está en pantalla a la vez.
    /// Lo abierto se queda: volver al módulo es volver a lo que se leía.
    pub fn enter_module(&self) {
        self.core.comms().leave();
        self.core.platform().leave();
        if !self.state().in_module {
            self.update(|state| state.in_module = true);
        }
    }

    /// Salir de Documentación a otro módulo.
    pub fn leave_module(&self) {
        if self.state().in_module {
            self.update(|state| state.in_module = false);
        }
    }

    /// La pantalla de inicio del módulo (la tabla): cierra lo que se lea,
    /// preguntando antes si hay texto sin guardar.
    pub fn show_index(&self) {
        self.enter_module();
        self.close();
    }

    /// Una fila de nodo del árbol: abre su documento por la portada. Si ya
    /// es el abierto, vuelve a su portada.
    pub async fn open_node(&self, id: &str) {
        if self.state().target.as_ref().is_some_and(|target| target.id == id) {
            self.close_page();
            return;
        }
        self.open(id).await;
    }

    /// Una página del árbol, sea del documento abierto o de otro.
    pub async fn open_tree_page(&self, owner_id: &str, page_id: &str) {
        if self.state().target.as_ref().is_some_and(|target| target.id == owner_id) {
            self.open_page(page_id).await;
        } else {
            self.open_at(owner_id, None, Some(page_id)).await;
        }
    }

    /// Pliega o despliega un nodo del árbol. Lo que se hace a mano manda
    /// sobre lo que va abierto por llevar al documento abierto. Al
    /// desplegar uno con páginas que no es el abierto, se piden: se ven sin
    /// abrirlo.
    pub async fn toggle_node(&self, kind: NodeKind, id: &str) {
        let key = doc_key(kind, id);
        let state = self.state();
        let tree = self.core.tree().state();
        let defaults = state.open_by_default(&doc_nodes(&tree.spaces));
        let open = !state.node_folds.get(&key).copied().unwrap_or_else(|| defaults.contains(&key));
        self.update(|state| {
            state.node_folds.insert(key.clone(), open);
        });
        let is_open_doc = state.target.as_ref().is_some_and(|target| target.kind == kind && target.id == id);
        let has_pages = tree.doc_marks.get(&key).and_then(|mark| mark.pages).unwrap_or(0) > 0;
        if open && !is_open_doc && has_pages {
            self.peek_pages(kind, id).await;
        }
    }

    /// Trae las páginas de un documento que no es el abierto, para verlas
    /// en el árbol. Un fallo deja lo que hubiera: es un adorno.
    pub(crate) async fn peek_pages(&self, kind: NodeKind, id: &str) {
        let path = format!("/api/v1/docs/{}/{id}/pages", kind_segment(kind));
        let Ok(pages) = self.core.get_list::<DocPageItem>(&path).await else { return };
        self.update(|state| {
            state.peeked.insert(doc_key(kind, id), pages);
        });
    }

    /// Las páginas de los nodos por encima del documento abierto, que van
    /// desplegados por llevar a él: sin pedirlas, su rama saldría vacía.
    /// Sólo en el módulo, que es donde se pinta el árbol.
    pub(crate) async fn peek_path(&self, kind: NodeKind, id: &str) {
        let state = self.state();
        if !state.in_module {
            return;
        }
        let tree = self.core.tree().state();
        let nodes = doc_nodes(&tree.spaces);
        let above: Vec<(NodeKind, String)> = {
            fn find(nodes: &[DocNode], kind: NodeKind, id: &str, chain: &mut Vec<(NodeKind, String)>) -> bool {
                for node in nodes {
                    if node.kind == kind && node.id == id {
                        return true;
                    }
                    chain.push((node.kind, node.id.to_string()));
                    if find(&node.children, kind, id, chain) {
                        return true;
                    }
                    chain.pop();
                }
                false
            }
            let mut chain = Vec::new();
            find(&nodes, kind, id, &mut chain);
            chain
        };
        for (kind, id) in above {
            let key = doc_key(kind, &id);
            let has_pages = tree.doc_marks.get(&key).and_then(|mark| mark.pages).unwrap_or(0) > 0;
            if has_pages && !state.peeked.contains_key(&key) {
                self.peek_pages(kind, &id).await;
            }
        }
    }
}

/// Al abrir otro documento se olvida lo plegado a mano en su camino: lo que
/// se lee no puede quedar escondido.
pub(crate) fn reveal(state: &mut DocsState, spaces: &[SpaceTree], kind: NodeKind, id: &str) {
    for key in node_path(&doc_nodes(spaces), kind, id) {
        state.node_folds.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::DocTarget;
    use crate::tree::ListSummary;

    fn list(id: &str, name: &str, tasks: i64) -> ListSummary {
        ListSummary { id: id.into(), name: name.into(), open_count: 0, project_id: None, task_count: tasks }
    }

    /// Producto › (Backend › tasks, bugs) y una lista suelta «roadmap».
    fn spaces() -> Vec<SpaceTree> {
        vec![SpaceTree {
            id: "s".into(),
            key: None,
            org_id: "o".into(),
            name: "Producto".into(),
            color: String::new(),
            project_id: None,
            kind: None,
            folders: vec![FolderTree { id: "f".into(), name: "Backend".into(), folders: vec![], lists: vec![list("t", "tasks", 4), list("b", "bugs", 2)] }],
            lists: vec![list("r", "roadmap", 1)],
            people: vec![],
        }]
    }

    fn mark(reviewed: Option<&str>, owner: Option<&str>, pages: i64) -> DocMark {
        DocMark {
            written: true,
            reviewed_at: reviewed.map(str::to_string),
            maintainer_id: owner.map(|_| "u".to_string()),
            maintainer_name: owner.map(str::to_string),
            pages: Some(pages),
            ..DocMark::default()
        }
    }

    fn page(id: &str, parent: Option<&str>, title: &str) -> DocPageItem {
        DocPageItem { id: id.into(), parent_id: parent.map(str::to_string), title: title.into(), ..DocPageItem::default() }
    }

    fn shape(rows: &[DocTreeRow]) -> Vec<String> {
        rows.iter().map(|row| format!("{}{}", "  ".repeat(row.depth as usize), row.title)).collect()
    }

    /// La tabla pone primero lo que nunca se revisó y luego lo más viejo,
    /// dice la ruta entera, deja las listas sin documento al pie y no
    /// cuenta como pendiente una carpeta sin documento. «Necesita atención»
    /// deja sólo lo viejo o sin responsable.
    ///
    /// Roturas que atrapa: un orden alfabético que esconde lo viejo; la
    /// ruta sin la carpeta (dos «tasks» indistinguibles); una carpeta sin
    /// documento entre las pendientes; el filtro de atención que no filtra.
    #[test]
    fn the_index_puts_what_is_oldest_first_and_the_undocumented_apart() {
        let marks = HashMap::from([
            (doc_key(NodeKind::List, "t"), mark(Some("2026-10-01T12:00:00Z"), Some("Ana"), 3)),
            (doc_key(NodeKind::List, "r"), mark(Some("2026-06-01T12:00:00Z"), Some("Ana"), 0)),
            (doc_key(NodeKind::Space, "s"), DocMark { stale: Some(true), ..mark(None, Some("Ana"), 0) }),
        ]);
        let index = doc_index(&spaces(), &marks, "", false, Locale::Es);
        let written: Vec<(&str, &str)> = index.written.iter().map(|row| (row.name.as_str(), row.path.as_str())).collect();
        assert_eq!(written, [("Producto", "—"), ("roadmap", "Producto"), ("tasks", "Producto › Backend")]);
        assert_eq!(index.written[0].reviewed, "Sin revisar nunca");
        assert_eq!(index.written[2].pages, Some(3));
        assert_eq!(index.written[1].pages, None, "un cero se pinta como nada");
        let undocumented: Vec<&str> = index.undocumented.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(undocumented, ["b"]);
        assert!(index.any_written);

        let mut stale = marks.clone();
        stale.insert(doc_key(NodeKind::List, "t"), mark(Some("2026-10-01T12:00:00Z"), None, 0));
        let attention: Vec<String> = doc_index(&spaces(), &stale, "", true, Locale::Es).written.into_iter().map(|row| row.id).collect();
        assert_eq!(attention, ["s", "t"]);

        let by_path: Vec<String> = doc_index(&spaces(), &marks, "backend", false, Locale::Es).written.into_iter().map(|row| row.id).collect();
        assert_eq!(by_path, ["t"]);
        assert!(!doc_index(&spaces(), &HashMap::new(), "", false, Locale::Es).any_written);
    }

    /// Sin nada abierto el árbol va plegado; con un documento abierto va
    /// desplegado el camino hasta él, con su portada y sus páginas debajo,
    /// y el fondo en lo que se lee, no en el nodo.
    ///
    /// Roturas que atrapa: todo desplegado (doscientas filas); el documento
    /// abierto escondido tras un chevrón; dos filas con fondo (el nodo y la
    /// portada); las páginas colgando del nodo que no es.
    #[test]
    fn the_tree_opens_the_way_to_what_is_read() {
        let tree = TreeState { spaces: spaces(), ..TreeState::default() };
        assert_eq!(shape(&DocsState::default().docs_tree_rows(&tree, "", Locale::Es)), ["Producto"]);

        let reading = DocsState {
            target: Some(DocTarget { kind: NodeKind::List, id: "t".into(), name: "tasks".into() }),
            pages: vec![page("p1", None, "Arquitectura"), page("p2", Some("p1"), "Tipos")],
            ..DocsState::default()
        };
        let rows = reading.docs_tree_rows(&tree, "", Locale::Es);
        assert_eq!(shape(&rows), ["Producto", "  Backend", "    tasks", "      Portada", "      Arquitectura", "    bugs", "  roadmap"]);
        let tasks = &rows[2];
        assert!(tasks.on_path && !tasks.active, "el fondo va en lo que se lee, no en su nodo");
        assert!(rows[3].active && rows[3].item == DocTreeItem::Home);
        assert!(rows[4].expandable && !rows[4].open);

        let on_page = DocsState { page_id: Some("p2".into()), ..reading.clone() };
        let rows = on_page.docs_tree_rows(&tree, "", Locale::Es);
        assert_eq!(shape(&rows)[4..6], ["      Arquitectura".to_string(), "        Tipos".to_string()], "la página abierta queda plegada bajo su madre");
        assert!(rows[5].active && !rows[3].active);

        let folded = DocsState { node_folds: HashMap::from([(doc_key(NodeKind::Folder, "f"), false)]), ..reading };
        assert_eq!(shape(&folded.docs_tree_rows(&tree, "", Locale::Es)), ["Producto", "  Backend", "  roadmap"]);
    }

    /// El filtro deja lo que casa con todo lo que lleva a ello, desplegado,
    /// y busca también en las páginas que ya se tienen, del abierto o de
    /// otro.
    ///
    /// Roturas que atrapa: una página encontrada sin su nodo encima (no se
    /// pinta); lo encontrado tras un chevrón cerrado; buscar sólo en los
    /// nodos; la portada entre los resultados.
    #[test]
    fn the_filter_keeps_what_matches_and_the_way_to_it() {
        let tree = TreeState { spaces: spaces(), ..TreeState::default() };
        let state = DocsState { peeked: HashMap::from([(doc_key(NodeKind::List, "b"), vec![page("q1", None, "Guía"), page("q2", Some("q1"), "Despliegue")])]), ..DocsState::default() };
        assert_eq!(shape(&state.docs_tree_rows(&tree, "desplie", Locale::Es)), ["Producto", "  Backend", "    bugs", "      Guía", "        Despliegue"]);
        assert_eq!(shape(&state.docs_tree_rows(&tree, "ROAD", Locale::Es)), ["Producto", "  roadmap"]);
        assert!(state.docs_tree_rows(&tree, "nada así", Locale::Es).is_empty());
    }
}
