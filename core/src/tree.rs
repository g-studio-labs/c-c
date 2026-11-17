//! El árbol de espacios, carpetas y listas de una organización.
//!
//! Vino de `fetchTree` en `app/src/store/tasks.store.ts` y de los tipos de
//! `app/src/types/task.ts`. Crear, renombrar y borrar nodos vino de las
//! «tree mutations» del mismo store y de lo que ofrece cada menú en
//! `app/src/components/tree/SpacesNavigator.tsx`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::nav::NodeKind;
use crate::texts::{error_phrase, text, text_count, text_with, Locale};
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

/// Una lista del árbol, cuelgue de donde cuelgue.
pub fn find_list<'a>(spaces: &'a [SpaceTree], list_id: &str) -> Option<&'a ListSummary> {
    fn within<'a>(folders: &'a [FolderTree], lists: &'a [ListSummary], id: &str) -> Option<&'a ListSummary> {
        lists.iter().find(|list| list.id == id).or_else(|| folders.iter().find_map(|folder| within(&folder.folders, &folder.lists, id)))
    }
    spaces.iter().find_map(|space| within(&space.folders, &space.lists, list_id))
}

/// Cómo se llama ahora un nodo de esa clase, esté donde esté del árbol.
/// `None` si ya no está.
///
/// Con la clase además del id: una lista y una carpeta son de tablas
/// distintas y nada impide que compartan id.
pub fn node_name(spaces: &[SpaceTree], kind: NodeKind, id: &str) -> Option<String> {
    match kind {
        NodeKind::Space => spaces.iter().find(|space| space.id == id).map(|space| space.name.clone()),
        NodeKind::Folder => find_folder(spaces, id).map(|folder| folder.name.clone()),
        NodeKind::List => find_list(spaces, id).map(|list| list.name.clone()),
    }
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

// ── Crear, renombrar y borrar nodos ─────────────────────────────────────

/// Lo que el menú de una fila («···») puede ofrecer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum NodeAction {
    OpenDocs,
    NewFolder,
    NewList,
    Rename,
    Delete,
}

/// Si ese papel puede cambiar el árbol: `admin` y `member`, y un
/// superadmin en cualquier organización. Un `viewer` mira.
///
/// Espejo de `OrgRole.CanWrite` (`backend/internal/core/domain/
/// organization.go`) y de `authorizeOrg` (`backend/internal/adapters/handler/
/// task.go`), que es por donde pasan todas las rutas de aquí abajo. El
/// servidor lo vuelve a decir (403 `viewer-read-only`); esto es para no
/// ofrecer lo que va a rechazar.
pub fn can_edit_tree(role: &str, superadmin: bool) -> bool {
    superadmin || matches!(role, "admin" | "member")
}

/// Qué ofrece el menú de un nodo, en su orden, a quien lo abre.
///
/// La documentación, a cualquiera. Lo demás, a quien puede cambiar el árbol:
/// carpetas y listas nuevas dentro de un espacio o de una carpeta (una lista
/// guarda tareas, no nodos), renombrar y borrar. El orden es el de los menús
/// de `SpacesNavigator.tsx`, con borrar al final.
pub fn node_menu(kind: NodeKind, can_edit: bool) -> Vec<NodeAction> {
    let mut actions = vec![NodeAction::OpenDocs];
    if !can_edit {
        return actions;
    }
    if kind != NodeKind::List {
        actions.extend([NodeAction::NewFolder, NodeAction::NewList]);
    }
    actions.extend([NodeAction::Rename, NodeAction::Delete]);
    actions
}

/// Lo más largo que puede ser el nombre de un nodo, en caracteres: el
/// `max=120` de `CreateSpaceRequest`, `CreateFolderRequest`,
/// `CreateListRequest` y `RenameRequest` (`backend/internal/core/domain/
/// task.go`).
pub const MAX_NODE_NAME: usize = 120;

/// Por qué no se hizo un cambio en el árbol.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TreeEditError {
    /// El nombre está en blanco.
    #[error("empty-name")]
    EmptyName,
    #[error("name-too-long")]
    NameTooLong,
    /// Quien lo pide sólo puede mirar en esta organización.
    #[error("viewer-read-only")]
    ReadOnly,
    /// No hay organización cargada en la que crear.
    #[error("no-organization")]
    NoOrganization,
    /// El nodo (o aquél en el que se iba a crear) ya no está en el árbol.
    #[error("node-gone")]
    NodeGone,
    /// La carpeta tiene carpetas dentro: ver `Tree::delete`.
    #[error("folder-has-folders")]
    FolderHasFolders,
    /// Lo que dijo el servidor, o que no contestó.
    #[error(transparent)]
    Api(#[from] ApiError),
}

/// El nombre tal como se va a mandar: sin los espacios de los lados.
/// En blanco o demasiado largo no se manda.
pub fn clean_node_name(raw: &str) -> Result<String, TreeEditError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(TreeEditError::EmptyName);
    }
    if name.chars().count() > MAX_NODE_NAME {
        return Err(TreeEditError::NameTooLong);
    }
    Ok(name.to_string())
}

/// Un fallo al cambiar el árbol, dicho para quien lo lee. `fallback_key` es
/// la frase para un error del servidor que el catálogo no conoce («No se
/// pudo borrar»).
pub fn tree_edit_phrase(locale: Locale, error: &TreeEditError, fallback_key: &str) -> String {
    match error {
        // El campo vacío se queda abierto y ya dice qué espera.
        TreeEditError::EmptyName => String::new(),
        TreeEditError::NameTooLong => text_with(locale, "work:tree.nameTooLong", &[("max", &MAX_NODE_NAME.to_string())]),
        TreeEditError::ReadOnly => text(locale, "errors:viewer-read-only"),
        TreeEditError::NoOrganization => text(locale, "work:tree.pickOrg"),
        TreeEditError::NodeGone => text(locale, "work:tree.nodeGone"),
        TreeEditError::FolderHasFolders => text(locale, "work:tree.folderHasFolders"),
        TreeEditError::Api(error) => error_phrase(locale, error, Some(fallback_key)),
    }
}

/// El espacio del que cuelga un nodo (él mismo, si es un espacio).
fn space_of<'a>(spaces: &'a [SpaceTree], kind: NodeKind, id: &str) -> Option<&'a SpaceTree> {
    spaces.iter().find(|space| node_name(std::slice::from_ref(space), kind, id).is_some())
}

/// Cuántas carpetas, listas y tareas hay bajo unas carpetas y unas listas,
/// a cualquier profundidad.
fn contents(folders: &[FolderTree], lists: &[ListSummary]) -> (i64, i64, i64) {
    let mut total = (folders.len() as i64, lists.len() as i64, lists.iter().map(|list| list.task_count).sum::<i64>());
    for folder in folders {
        let inner = contents(&folder.folders, &folder.lists);
        total = (total.0 + inner.0, total.1 + inner.1, total.2 + inner.2);
    }
    total
}

/// Lo que se le pregunta a quien va a borrar un nodo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeleteNotice {
    /// «¿Borrar la lista «Pendientes»?»: nombra el nodo.
    pub title: String,
    /// Qué se va con él, con sus cuentas. Si no se puede borrar, por qué.
    pub body: String,
    /// Lo que dice el botón que borra.
    pub confirm: String,
    /// `false`: no se puede borrar, y entonces no hay nada que confirmar.
    pub allowed: bool,
}

/// Qué preguntar antes de borrar ese nodo. `None` si ya no está en el árbol.
///
/// Los textos son los de `SpacesNavigator.tsx`, y además se dice **cuánto**
/// se va: «Borra sus carpetas, sus listas y todas las tareas de dentro» no
/// distingue un espacio de pruebas de uno con cuatrocientas tareas. Y si la
/// lista o el espacio lo ve un cliente, se dice.
///
/// Lo que hace el servidor con cada uno (`backend/internal/core/repository/
/// task.go`): un espacio se lleva carpetas, listas y tareas; una lista, sus
/// tareas y sus columnas; una carpeta no borra nada —sus listas suben al
/// espacio—, pero **no mira si tiene carpetas dentro**: se quedarían
/// apuntando a un padre que no existe, y el árbol se construye bajando desde
/// el espacio, así que dejarían de pintarse con todo lo suyo. Por eso una
/// carpeta con carpetas dentro no se deja borrar desde aquí.
pub fn delete_notice(spaces: &[SpaceTree], kind: NodeKind, id: &str, locale: Locale) -> Option<DeleteNotice> {
    let name = node_name(spaces, kind, id)?;
    let named = |key: &str| text_with(locale, key, &[("name", &name)]);
    let counted = |counts: [(&str, i64); 3]| {
        let said: Vec<String> = counts.into_iter().filter(|(_, count)| *count > 0).map(|(key, count)| text_count(locale, key, count)).collect();
        if said.is_empty() { text(locale, "work:tree.empty") } else { said.join(" · ") }
    };
    let (folders_key, lists_key, tasks_key) = ("common:count.folders", "common:count.lists", "common:count.tasks");
    Some(match kind {
        NodeKind::Space => {
            let space = spaces.iter().find(|space| space.id == id)?;
            let (folders, lists, tasks) = contents(&space.folders, &space.lists);
            let mut body = format!("{}\n{}", text(locale, "work:tree.deleteSpaceBody"), counted([(folders_key, folders), (lists_key, lists), (tasks_key, tasks)]));
            if space.project_id.is_some() {
                body = format!("{body}\n{}", text(locale, "work:tree.clientSeesSpace"));
            }
            DeleteNotice { title: named("work:tree.deleteSpaceTitle"), body, confirm: text(locale, "work:tree.deleteSpace"), allowed: true }
        }
        NodeKind::Folder => {
            let folder = find_folder(spaces, id)?;
            if !folder.folders.is_empty() {
                return Some(DeleteNotice {
                    title: named("work:tree.deleteFolderTitle"),
                    body: text(locale, "work:tree.folderHasFolders"),
                    confirm: text(locale, "work:tree.deleteFolder"),
                    allowed: false,
                });
            }
            let lists = folder.lists.len() as i64;
            let body = format!("{}\n{}", text(locale, "work:tree.deleteFolderBody"), counted([(folders_key, 0), (lists_key, lists), (tasks_key, 0)]));
            DeleteNotice { title: named("work:tree.deleteFolderTitle"), body, confirm: text(locale, "work:tree.deleteFolder"), allowed: true }
        }
        NodeKind::List => {
            let list = find_list(spaces, id)?;
            let mut body = text_count(locale, "common:count.deletesTasks", list.task_count);
            if list.project_id.is_some() {
                body = format!("{body}\n{}", text(locale, "work:tree.clientSeesList"));
            }
            DeleteNotice { title: named("work:tree.deleteListTitle"), body, confirm: text(locale, "work:tree.deleteList"), allowed: true }
        }
    })
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
                // El ámbito de «Mi trabajo» es un nodo de este árbol: con
                // uno nuevo hay que volver a mirar si sigue y cómo se llama.
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

    // ── Crear, renombrar y borrar ───────────────────────────────────────

    /// Si quien está dentro puede cambiar el árbol de la organización que
    /// se mira.
    pub fn can_edit(&self) -> bool {
        let superadmin = self.core.session().is_some_and(|session| session.superadmin);
        let orgs = self.core.orgs().state();
        can_edit_tree(orgs.current().map(|org| org.role.as_str()).unwrap_or_default(), superadmin)
    }

    /// Lo que se le pregunta a quien va a borrar ese nodo, en ese idioma.
    pub fn delete_notice(&self, kind: NodeKind, id: &str, locale: Locale) -> Option<DeleteNotice> {
        delete_notice(&self.state().spaces, kind, id, locale)
    }

    /// La organización cargada, para quien puede cambiar su árbol.
    fn editable_org(&self) -> Result<String, TreeEditError> {
        if !self.can_edit() {
            return Err(TreeEditError::ReadOnly);
        }
        self.state().org_id.ok_or(TreeEditError::NoOrganization)
    }

    /// El árbol otra vez, tras un cambio: lo que el servidor guardó es lo
    /// que se pinta (el orden del nodo nuevo lo pone él). La documentación
    /// abierta de un nodo que ya no está se cierra: es de algo borrado. Y
    /// la de uno que cambió de nombre se vuelve a abrir, para que su
    /// cabecera no siga diciendo el de antes.
    async fn reload_after_edit(&self, org_id: &str) {
        self.load(org_id).await;
        let spaces = self.state().spaces;
        let docs = self.core.docs();
        let Some(target) = docs.state().target else { return };
        match node_name(&spaces, target.kind, &target.id) {
            None => docs.close(),
            Some(name) if name != target.name => docs.open(&target.id).await,
            Some(_) => {}
        }
    }

    /// Crea un espacio en la organización que se mira.
    ///
    /// `POST /api/v1/task-spaces/` con `{orgId, name}`, como `createSpace`
    /// en `tasks.store.ts`.
    pub async fn create_space(&self, name: &str) -> Result<(), TreeEditError> {
        let org_id = self.editable_org()?;
        let name = clean_node_name(name)?;
        let body = serde_json::json!({ "orgId": org_id, "name": name });
        self.core.call_ok("POST", "/api/v1/task-spaces/", Some(body)).await?;
        self.reload_after_edit(&org_id).await;
        Ok(())
    }

    /// Crea una carpeta o una lista dentro de un espacio o de una carpeta.
    ///
    /// Las dos rutas cuelgan del **espacio**, también para lo que nace en
    /// una carpeta, que va en el cuerpo: `POST /api/v1/task-spaces/{id}/
    /// folders` con `{name, parentFolderId}` y `…/lists` con `{name,
    /// folderId}`, con `null` cuando cuelga del espacio (`createFolder` y
    /// `createList` en `tasks.store.ts`). Dentro de una lista no se crea
    /// nada, y un espacio no se crea dentro de otro.
    pub async fn create_in(&self, parent: NodeKind, parent_id: &str, child: NodeKind, name: &str) -> Result<(), TreeEditError> {
        let org_id = self.editable_org()?;
        let name = clean_node_name(name)?;
        let spaces = self.state().spaces;
        let space = space_of(&spaces, parent, parent_id).ok_or(TreeEditError::NodeGone)?;
        let folder_id = match parent {
            NodeKind::Space => None,
            NodeKind::Folder => Some(parent_id),
            NodeKind::List => return Err(TreeEditError::NodeGone),
        };
        let (route, body) = match child {
            NodeKind::Folder => ("folders", serde_json::json!({ "name": name, "parentFolderId": folder_id })),
            NodeKind::List => ("lists", serde_json::json!({ "name": name, "folderId": folder_id })),
            NodeKind::Space => return Err(TreeEditError::NodeGone),
        };
        self.core.call_ok("POST", &format!("/api/v1/task-spaces/{}/{route}", space.id), Some(body)).await?;
        self.reload_after_edit(&org_id).await;
        Ok(())
    }

    /// Le cambia el nombre a un nodo. El mismo nombre que ya tiene no se
    /// manda.
    ///
    /// `PATCH /api/v1/task-{spaces,folders,lists}/{id}` con sólo `{name}`:
    /// lo que no se manda (el color, el canal, la clave) no se toca
    /// (`RenameRequest` en `backend/internal/core/domain/task.go`).
    pub async fn rename(&self, kind: NodeKind, id: &str, name: &str) -> Result<(), TreeEditError> {
        let org_id = self.editable_org()?;
        let name = clean_node_name(name)?;
        let current = node_name(&self.state().spaces, kind, id).ok_or(TreeEditError::NodeGone)?;
        if name == current {
            return Ok(());
        }
        self.core.call_ok("PATCH", &node_path(kind, id), Some(serde_json::json!({ "name": name }))).await?;
        self.reload_after_edit(&org_id).await;
        Ok(())
    }

    /// Borra un nodo. **Quien llama ya preguntó** (`delete_notice`).
    ///
    /// `DELETE /api/v1/task-{spaces,folders,lists}/{id}`. Una carpeta con
    /// carpetas dentro no se manda: el servidor la borraría dejando a las
    /// de dentro fuera del árbol (ver `delete_notice`). Tras borrar una
    /// lista o un espacio se vuelve a pedir «Mi trabajo»: sus tareas ya no
    /// existen.
    pub async fn delete(&self, kind: NodeKind, id: &str) -> Result<(), TreeEditError> {
        let org_id = self.editable_org()?;
        let spaces = self.state().spaces;
        node_name(&spaces, kind, id).ok_or(TreeEditError::NodeGone)?;
        if kind == NodeKind::Folder && find_folder(&spaces, id).is_some_and(|folder| !folder.folders.is_empty()) {
            return Err(TreeEditError::FolderHasFolders);
        }
        self.core.call_ok("DELETE", &node_path(kind, id), None).await?;
        self.reload_after_edit(&org_id).await;
        if kind != NodeKind::Folder && self.core.my_work().state().loaded_key.is_some() {
            self.core.my_work().load(Some(org_id)).await;
        }
        Ok(())
    }
}

/// La ruta de un nodo, por su clase.
fn node_path(kind: NodeKind, id: &str) -> String {
    let table = match kind {
        NodeKind::Space => "task-spaces",
        NodeKind::Folder => "task-folders",
        NodeKind::List => "task-lists",
    };
    format!("/api/v1/{table}/{id}")
}
