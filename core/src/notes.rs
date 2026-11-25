//! Notas: las páginas personales de cada quien, en un árbol
//! (command-and-control-511).
//!
//! Vino de `app/src/pages/Notes.tsx` y `store/notes.store.ts`. El contrato es
//! el del backend (`domain/note.go`): aquí se describe, se pide y se decide
//! qué se manda al guardar y qué se pregunta antes de perder texto.
//!
//! - **Son de una persona, no de una organización**: ni siquiera un
//!   superadmin las ve. Cambiar de organización no las toca.
//! - **Lo escrito no se pierde sin preguntar.** Cambiar de nota, cancelar o
//!   cerrar la sesión con texto sin guardar se queda en una pregunta. Salir de
//!   la pantalla no pregunta, porque no pierde nada: el borrador vive aquí y
//!   está donde se dejó al volver. Cerrar la ventana sí pregunta
//!   (`Core::unsaved_notes`).
//! - **No se pisa a nadie, a su manera.** Cada guardado dice de qué versión
//!   salió (`baseHash`). Si el servidor ya tenía otra no contesta 409: deja
//!   la nota como estaba y cuelga lo escrito como una hija «en conflicto»
//!   (`NoteConflict`). En la app de siempre se guarda solo; aquí con su
//!   botón, como la documentación (decidido con jose, 18-nov-2026).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::doc_edit::{next_session, DocEditError};
use crate::doc_pages::or_empty;
use crate::error::ApiError;
use crate::session::unwrap_data;
use crate::task::Question;
use crate::texts::{error_phrase, text, text_count, text_with, Locale};
use crate::Core;

/// Cuántos resultados se enseñan de una búsqueda sin red, por título.
const OFFLINE_MATCHES: usize = 40;

/// Una fila del navegador: sin cuerpo, para que listar todo el árbol cueste
/// poco. Espejo de `domain.NoteTreeItem`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct NoteTreeItem {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Su puesto entre hermanas. Puede haber huecos.
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub has_body: bool,
    #[serde(default)]
    pub favorite: bool,
}

/// Una nota, con su texto. Espejo de `domain.Note`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub title: String,
    /// Markdown.
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub favorite: bool,
    /// De qué versión es el texto: quien guarda lo devuelve (`baseHash`).
    #[serde(default)]
    pub body_hash: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Un fichero adjunto a una nota.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct NoteAttachment {
    pub id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub bytes: i64,
}

/// Una nota que sale en una búsqueda o en «Enlazada desde».
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NoteLink {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub excerpt: String,
}

/// Lo que contesta `GET /api/v1/notes/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NoteDetail {
    pub note: Note,
    #[serde(default, deserialize_with = "or_empty")]
    pub attachments: Vec<NoteAttachment>,
    /// Las que la citan. Go la escribe `null` si no hay ninguna.
    #[serde(default, deserialize_with = "or_empty")]
    pub backlinks: Vec<NoteLink>,
}

/// Una página de la papelera, con cuántas vuelven con ella.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct NoteTrashItem {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub deleted_at: String,
    #[serde(default)]
    pub subpages: u32,
}

impl NoteTrashItem {
    /// Cuándo se borró, dicho: el instante es de quien lo lee, así que el día
    /// y la hora son los suyos. Vacío si el servidor no dio una fecha legible.
    pub fn deleted_on(&self, locale: Locale) -> String {
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(&self.deleted_at) else { return String::new() };
        crate::dates::date_and_time(locale, at.with_timezone(&chrono::Local).naive_local())
    }
}

/// Dónde está lo que se pierde si se sigue: a dónde se iba.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum NoteLeave {
    /// «Cancelar».
    Cancel,
    /// Cerrar la nota.
    Close,
    /// Otra nota.
    Open { id: String },
    /// Una nota nueva, colgada de otra si se dice.
    Create { parent_id: Option<String>, title: String },
    /// Cerrar la sesión.
    LogOut,
}

/// Lo que se pregunta antes de hacerlo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum NoteAsk {
    /// Irse y perder lo escrito.
    Leave { to: NoteLeave },
    /// Mandar una nota (y lo que cuelga de ella) a la papelera.
    Delete { id: String },
    /// Borrar de la papelera para siempre.
    Purge { id: String },
    EmptyTrash,
}

/// Por qué no salió lo que se pidió, con la frase de reserva de lo que se
/// intentaba.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NoteProblem {
    pub fallback_key: String,
    pub error: DocEditError,
}

impl NoteProblem {
    fn of(fallback_key: &str, error: &ApiError) -> Self {
        Self { fallback_key: fallback_key.to_string(), error: DocEditError::from(error) }
    }

    pub fn phrase(&self, locale: Locale) -> String {
        error_phrase(locale, &api_error(&self.error), Some(&self.fallback_key))
    }
}

fn api_error(error: &DocEditError) -> ApiError {
    if error.offline {
        ApiError::Network(String::new())
    } else {
        ApiError::Server { status: 0, code: error.code.clone(), message: error.message.clone() }
    }
}

/// Lo que se está escribiendo de una nota.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NoteEdit {
    /// Cada vez que se abre el editor es una sesión distinta: con este número
    /// la pantalla sabe que tiene que poner en sus cajas el texto del
    /// borrador, y no hacerlo en cada tecla.
    pub session: u64,
    pub id: String,
    pub base_title: String,
    pub base_body: String,
    pub base_hash: Option<String>,
    pub title: String,
    pub body: String,
    pub saving: bool,
    pub error: Option<DocEditError>,
    /// Empieza por el título (se llegó a renombrar, o la nota es nueva).
    pub focus_title: bool,
    /// En una ventana estrecha: si se mira la vista previa en vez de la caja.
    pub previewing: bool,
}

impl NoteEdit {
    fn begin(note: &Note, focus_title: bool) -> Self {
        Self {
            session: next_session(),
            id: note.id.clone(),
            base_title: note.title.clone(),
            base_body: note.body.clone(),
            base_hash: note.body_hash.clone(),
            title: note.title.clone(),
            body: note.body.clone(),
            saving: false,
            error: None,
            focus_title,
            previewing: false,
        }
    }

    /// Si hay algo escrito que el servidor no tiene.
    pub fn is_dirty(&self) -> bool {
        self.body != self.base_body || self.title != self.base_title
    }

    pub fn can_save(&self) -> bool {
        self.is_dirty() && !self.saving
    }

    /// En qué está lo escrito, para decirlo junto a los botones.
    pub fn status(&self, locale: Locale) -> Option<String> {
        if self.saving {
            Some(text(locale, "work:docs.save_guardando"))
        } else if self.is_dirty() {
            Some(text(locale, "work:docs.unsaved"))
        } else {
            None
        }
    }

    pub fn error_phrase(&self, locale: Locale) -> Option<String> {
        let error = self.error.as_ref()?;
        Some(error_phrase(locale, &api_error(error), Some("work:notes.errSaveNote")))
    }
}

/// Una nota que se guardó como hija porque otro dispositivo llegó antes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NoteConflict {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NoteSearch {
    pub open: bool,
    pub query: String,
    pub results: Vec<NoteLink>,
    pub searching: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NoteTrash {
    pub open: bool,
    pub loading: bool,
    pub items: Vec<NoteTrashItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NotesState {
    /// Si la pantalla de Notas es la que se tiene delante.
    pub on_screen: bool,
    pub tree: Vec<NoteTreeItem>,
    pub tree_loading: bool,
    /// Si el árbol llegó alguna vez: sin esto, «no hay páginas» se diría
    /// antes de saberlo.
    pub tree_loaded: bool,
    pub open_id: Option<String>,
    pub detail: Option<NoteDetail>,
    pub detail_loading: bool,
    pub detail_failed: bool,
    /// Las ramas plegadas a mano. Todas empiezan abiertas.
    pub folded: HashMap<String, bool>,
    pub edit: Option<NoteEdit>,
    pub asking: Option<NoteAsk>,
    pub conflict: Option<NoteConflict>,
    pub problem: Option<NoteProblem>,
    pub search: NoteSearch,
    pub trash: NoteTrash,
}

/// Una fila del navegador, ya con lo que hay que pintar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NoteRow {
    pub id: String,
    pub title: String,
    pub depth: u32,
    pub expandable: bool,
    pub open: bool,
    pub favorite: bool,
    pub active: bool,
}

fn by_position(items: &mut [&NoteTreeItem]) {
    items.sort_by_key(|item| item.position);
}

fn children_of<'a>(tree: &'a [NoteTreeItem], parent: Option<&str>) -> Vec<&'a NoteTreeItem> {
    let mut children: Vec<&NoteTreeItem> = tree.iter().filter(|item| item.parent_id.as_deref() == parent).collect();
    by_position(&mut children);
    children
}

/// Las filas del árbol, en el orden en que se pintan: cada nota seguida de
/// lo que cuelga de ella, salvo en las ramas plegadas.
pub fn note_rows(state: &NotesState) -> Vec<NoteRow> {
    fn walk(state: &NotesState, parent: Option<&str>, depth: u32, rows: &mut Vec<NoteRow>) {
        for item in children_of(&state.tree, parent) {
            let expandable = state.tree.iter().any(|other| other.parent_id.as_deref() == Some(&item.id));
            let open = !state.folded.get(&item.id).copied().unwrap_or(false);
            rows.push(NoteRow {
                id: item.id.clone(),
                title: item.title.clone(),
                depth,
                expandable,
                open,
                favorite: item.favorite,
                active: state.open_id.as_deref() == Some(&item.id),
            });
            if expandable && open {
                walk(state, Some(&item.id), depth + 1, rows);
            }
        }
    }
    let mut rows = Vec::new();
    walk(state, None, 0, &mut rows);
    rows
}

/// A qué nota lleva un enlace de dentro de una nota (`/notes/{id}`).
pub fn note_link_target(href: &str) -> Option<String> {
    let rest = href.strip_prefix("/notes/")?;
    let id: String = rest.chars().take_while(|c| !matches!(c, '/' | '?' | '#')).collect();
    (!id.is_empty()).then_some(id)
}

impl NotesState {
    /// Las favoritas, por título: un atajo plano, no un segundo árbol.
    pub fn favorites(&self) -> Vec<&NoteTreeItem> {
        let mut favorites: Vec<&NoteTreeItem> = self.tree.iter().filter(|item| item.favorite).collect();
        favorites.sort_by_key(|item| item.title.to_lowercase());
        favorites
    }

    /// Las hijas de la nota abierta que su texto no enlaza (creadas desde el
    /// árbol): al pie, para que una página enseñe todo lo que cuelga de ella.
    pub fn unlinked_children(&self) -> Vec<&NoteTreeItem> {
        let Some(id) = self.open_id.as_deref() else { return Vec::new() };
        let body = match &self.edit {
            Some(edit) if edit.id == id => edit.body.as_str(),
            _ => self.detail.as_ref().filter(|detail| detail.note.id == id).map(|detail| detail.note.body.as_str()).unwrap_or(""),
        };
        children_of(&self.tree, Some(id)).into_iter().filter(|item| !body.contains(&item.id)).collect()
    }

    /// La nota y todo lo que cuelga de ella.
    pub fn subtree_of(&self, id: &str) -> Vec<String> {
        let mut found = vec![id.to_string()];
        let mut next = 0;
        while next < found.len() {
            let parent = found[next].clone();
            found.extend(self.tree.iter().filter(|item| item.parent_id.as_deref() == Some(&parent)).map(|item| item.id.clone()));
            next += 1;
        }
        found
    }

    /// Cuántas páginas cuelgan de ella.
    pub fn subpage_count(&self, id: &str) -> usize {
        self.subtree_of(id).len() - 1
    }

    /// Si hay una nota abierta y quien mira puede escribirla.
    pub fn editable(&self) -> bool {
        self.edit.is_none() && self.detail.as_ref().is_some_and(|detail| Some(&detail.note.id) == self.open_id.as_ref())
    }

    /// El título que se enseña de una nota: el suyo, o «Sin título».
    pub fn shown_title(title: &str, locale: Locale) -> String {
        if title.trim().is_empty() {
            text(locale, "work:notes.untitled")
        } else {
            title.to_string()
        }
    }

    fn title_of(&self, id: &str) -> String {
        self.tree.iter().find(|item| item.id == id).map(|item| item.title.clone()).unwrap_or_default()
    }

    /// Lo que se pregunta ahora mismo, con sus dos botones. El primero es
    /// siempre el que pierde algo.
    pub fn question(&self, locale: Locale) -> Option<Question> {
        let keep = text(locale, "work:docs.keepEditing");
        let cancel = text(locale, "work:docs.cancel");
        Some(match self.asking.as_ref()? {
            NoteAsk::Leave { .. } => Question {
                title: text(locale, "work:docs.discardTitle"),
                body: text(locale, "work:docs.discardWhy"),
                warning: None,
                confirm: text(locale, "work:docs.discard"),
                cancel: keep,
            },
            NoteAsk::Delete { id } => {
                let name = Self::shown_title(&self.title_of(id), locale);
                let count = self.subpage_count(id) as i64;
                Question {
                    title: text_with(locale, "work:notes.deleteTitle", &[("name", &name)]),
                    body: if count > 0 { text_count(locale, "common:count.removesSubpages", count) } else { text(locale, "work:notes.restoreFromTrash") },
                    warning: None,
                    confirm: text(locale, "work:notes.delete"),
                    cancel,
                }
            }
            NoteAsk::Purge { id } => {
                let item = self.trash.items.iter().find(|item| &item.id == id);
                let name = Self::shown_title(item.map(|item| item.title.as_str()).unwrap_or(""), locale);
                let subpages = item.map(|item| item.subpages).unwrap_or(0) as i64;
                Question {
                    title: text_with(locale, "work:notes.purgeTitle", &[("name", &name)]),
                    body: if subpages > 0 { text_count(locale, "common:count.deletesSubpages", subpages) } else { text(locale, "work:notes.cannotUndo") },
                    warning: None,
                    confirm: text(locale, "work:notes.deleteForever"),
                    cancel,
                }
            }
            NoteAsk::EmptyTrash => Question {
                title: text(locale, "work:notes.emptyTrashTitle"),
                body: text(locale, "work:notes.emptyTrashBody"),
                warning: None,
                confirm: text(locale, "work:notes.emptyTrash"),
                cancel,
            },
        })
    }
}

/// Cómo quedan las hermanas de `id` tras subirla o bajarla un puesto,
/// numeradas de cero. `None` si ya está en el borde.
fn swapped(tree: &[NoteTreeItem], id: &str, up: bool) -> Option<Vec<(String, Option<String>, i64)>> {
    let parent = tree.iter().find(|item| item.id == id)?.parent_id.clone();
    let mut order: Vec<String> = children_of(tree, parent.as_deref()).into_iter().map(|item| item.id.clone()).collect();
    let at = order.iter().position(|other| other == id)?;
    let to = if up { at.checked_sub(1)? } else { at + 1 };
    if to >= order.len() {
        return None;
    }
    order.swap(at, to);
    Some(order.into_iter().enumerate().map(|(position, id)| (id, parent.clone(), position as i64)).collect())
}

#[derive(Default)]
pub(crate) struct NotesData {
    state: NotesState,
    /// Qué lectura de una nota es la vigente: una que llega tarde no se pone.
    open_turn: u64,
    search_turn: u64,
}

#[derive(Clone)]
pub struct Notes {
    core: Core,
}

impl Core {
    pub fn notes(&self) -> Notes {
        Notes { core: self.clone() }
    }

    /// Si en Notas hay algo escrito que el servidor no tiene: cerrar la
    /// ventana ahora lo perdería.
    pub fn unsaved_notes(&self) -> bool {
        self.notes().state().edit.is_some_and(|edit| edit.is_dirty())
    }
}

impl Notes {
    fn data(&self) -> std::sync::MutexGuard<'_, NotesData> {
        self.core.inner.notes.lock().expect("notes lock")
    }

    pub fn state(&self) -> NotesState {
        self.data().state.clone()
    }

    fn change<R>(&self, edit: impl FnOnce(&mut NotesData) -> R) -> R {
        let (out, state) = {
            let mut data = self.data();
            let out = edit(&mut data);
            (out, data.state.clone())
        };
        self.core.inner.listener.notes_changed(state);
        out
    }

    fn update(&self, edit: impl FnOnce(&mut NotesState)) {
        self.change(|data| edit(&mut data.state));
    }

    /// Pone Notas delante y trae el árbol. Es de «Trabajo»: deja
    /// comunicación, Plataforma, «Por atender» y la documentación. Con
    /// texto sin guardar en un documento no se va (se pregunta, `doc_edit`).
    pub async fn enter(&self) {
        if !self.show() {
            return;
        }
        self.load_tree().await;
        // Volver a la nota que se tenía abierta: se pone al día.
        if let Some(id) = self.state().open_id {
            self.reload(&id).await;
        }
    }

    pub fn show(&self) -> bool {
        if self.core.docs().held_back(crate::doc_edit::DocLeave::AllWork) {
            return false;
        }
        self.core.docs().close_now();
        self.core.docs().leave_module();
        self.core.comms().leave();
        self.core.platform().leave();
        self.core.requests().leave();
        self.update(|state| state.on_screen = true);
        true
    }

    /// Volver a lo demás. El borrador se queda: no se pierde nada.
    pub fn leave(&self) {
        if self.state().on_screen {
            self.update(|state| {
                state.on_screen = false;
                state.search.open = false;
                state.trash.open = false;
            });
        }
    }

    /// Al salir: las notas de una persona no son de la siguiente.
    pub(crate) fn clear(&self) {
        self.change(|data| {
            data.open_turn += 1;
            data.search_turn += 1;
            data.state = NotesState::default();
        });
    }

    /// Si esto que se iba a hacer se queda en una pregunta, porque hay texto
    /// sin guardar. Sin él, el editor (limpio) se cierra y se sigue.
    pub(crate) fn held_back(&self, to: NoteLeave) -> bool {
        let Some(edit) = self.state().edit else { return false };
        if edit.is_dirty() {
            self.update(|state| state.asking = Some(NoteAsk::Leave { to }));
            return true;
        }
        self.update(|state| state.edit = None);
        false
    }

    fn change_edit(&self, change: impl FnOnce(&mut NoteEdit)) {
        if self.state().edit.is_some() {
            self.update(|state| {
                if let Some(edit) = &mut state.edit {
                    change(edit);
                }
            });
        }
    }

    fn fail(&self, key: &str, error: &ApiError) {
        let problem = NoteProblem::of(key, error);
        self.update(|state| state.problem = Some(problem));
    }

    pub fn dismiss_problem(&self) {
        self.update(|state| state.problem = None);
    }

    pub fn dismiss_conflict(&self) {
        self.update(|state| state.conflict = None);
    }

    /// Trae el árbol. Si falla, el que había se queda a la vista.
    pub async fn load_tree(&self) {
        self.update(|state| state.tree_loading = true);
        match self.core.get_list::<NoteTreeItem>("/api/v1/notes/").await {
            Ok(tree) => self.update(|state| {
                state.tree = tree;
                state.tree_loading = false;
                state.tree_loaded = true;
                state.problem = None;
            }),
            Err(error) => {
                self.update(|state| state.tree_loading = false);
                self.fail("work:notes.couldNotLoadPage", &error);
            }
        }
    }

    pub fn toggle_fold(&self, id: &str) {
        self.update(|state| {
            let folded = state.folded.get(id).copied().unwrap_or(false);
            state.folded.insert(id.to_string(), !folded);
        });
    }

    /// Abre una nota y la lee. La que se leía no se enseña mientras llega
    /// ésta. Con texto sin guardar en otra no se cambia: se pregunta.
    pub async fn open(&self, id: &str) {
        let same = self.state().open_id.as_deref() == Some(id);
        if !same && self.held_back(NoteLeave::Open { id: id.to_string() }) {
            return;
        }
        self.reload(id).await;
    }

    async fn reload(&self, id: &str) {
        let turn = self.change(|data| {
            data.open_turn += 1;
            let state = &mut data.state;
            if state.detail.as_ref().is_some_and(|detail| detail.note.id != id) {
                state.detail = None;
            }
            state.open_id = Some(id.to_string());
            state.detail_loading = true;
            state.detail_failed = false;
            data.open_turn
        });
        let answer = self.core.get::<NoteDetail>(&format!("/api/v1/notes/{id}")).await;
        if self.data().open_turn != turn {
            return;
        }
        match answer {
            Ok(detail) => self.update(|state| {
                state.detail = Some(detail);
                state.detail_loading = false;
            }),
            Err(error) => {
                self.update(|state| {
                    state.detail_loading = false;
                    state.detail_failed = state.detail.is_none();
                });
                self.fail("work:notes.couldNotLoadPage", &error);
            }
        }
    }

    /// Cierra la nota abierta. Con texto sin guardar, pregunta.
    pub fn close(&self) {
        if self.held_back(NoteLeave::Close) {
            return;
        }
        self.close_now();
    }

    fn close_now(&self) {
        self.change(|data| {
            data.open_turn += 1;
            let state = &mut data.state;
            state.open_id = None;
            state.detail = None;
            state.detail_loading = false;
            state.detail_failed = false;
            state.edit = None;
            state.conflict = None;
        });
    }

    /// Empieza a escribir la nota que se lee. Dice si empezó.
    pub fn start_edit(&self) -> bool {
        self.start_edit_titled(false)
    }

    fn start_edit_titled(&self, focus_title: bool) -> bool {
        let state = self.state();
        if !state.editable() {
            return false;
        }
        let Some(detail) = state.detail else { return false };
        self.update(|state| state.edit = Some(NoteEdit::begin(&detail.note, focus_title)));
        true
    }

    /// «Renombrar»: se abre la nota y se escribe, con el título primero.
    pub async fn begin_rename(&self, id: &str) {
        self.open(id).await;
        if self.state().open_id.as_deref() == Some(id) {
            self.start_edit_titled(true);
        }
    }

    pub fn draft_title(&self, title: &str) {
        self.change_edit(|edit| edit.title = title.to_string());
    }

    pub fn draft_body(&self, body: &str) {
        self.change_edit(|edit| edit.body = body.to_string());
    }

    pub fn show_preview(&self, on: bool) {
        self.change_edit(|edit| edit.previewing = on);
    }

    /// «Cancelar»: sin cambios cierra el editor; con ellos, pregunta.
    pub fn cancel_edit(&self) {
        self.held_back(NoteLeave::Cancel);
    }

    /// Una nota nueva, vacía, y a escribirla. Existe en el servidor desde el
    /// primer momento, como en la app de siempre.
    pub async fn create(&self, parent_id: Option<&str>, title: &str) {
        let parent = parent_id.map(str::to_string);
        if self.held_back(NoteLeave::Create { parent_id: parent.clone(), title: title.to_string() }) {
            return;
        }
        self.create_now(parent, title).await;
    }

    async fn create_now(&self, parent: Option<String>, title: &str) {
        let mut body = json!({ "title": title });
        if let Some(parent) = &parent {
            body["parentId"] = json!(parent);
        }
        match self.core.call::<Note>("POST", "/api/v1/notes/", Some(body)).await {
            Ok(note) => {
                self.load_tree().await;
                self.reload(&note.id).await;
                self.start_edit_titled(true);
            }
            Err(error) => self.fail("work:notes.errCreatePage", &error),
        }
    }

    /// Guarda lo escrito, mandando sólo lo que cambió: renombrar no manda el
    /// texto, y así no puede chocar con quien lo esté escribiendo.
    pub async fn save(&self) {
        let Some(edit) = self.state().edit else { return };
        if edit.saving {
            return;
        }
        if !edit.is_dirty() {
            self.update(|state| state.edit = None);
            return;
        }
        self.change_edit(|edit| {
            edit.saving = true;
            edit.error = None;
        });
        let mut body = json!({});
        let renamed = edit.title != edit.base_title;
        if renamed {
            body["title"] = json!(edit.title.trim());
        }
        if edit.body != edit.base_body {
            body["body"] = json!(edit.body);
            if let Some(hash) = &edit.base_hash {
                body["baseHash"] = json!(hash);
            }
        }
        let path = format!("/api/v1/notes/{}", edit.id);
        let outcome = match self.core.request("PATCH", &path, Some(body.to_string()), true).await {
            Ok(reply) => unwrap_data::<SaveResult>(&reply),
            Err(error) => Err(error),
        };
        // Si el editor ya no es éste, la respuesta no es de nadie.
        if self.state().edit.as_ref().map(|current| current.session) != Some(edit.session) {
            return;
        }
        match outcome {
            Ok(saved) => {
                let (sent_title, sent_body) = (edit.title.clone(), edit.body.clone());
                let has_conflict = saved.conflict.is_some();
                self.update(|state| {
                    if let Some(conflict) = saved.conflict {
                        state.conflict = Some(NoteConflict { id: conflict.id, title: conflict.title });
                    }
                    if state.open_id.as_deref() == Some(&saved.note.id) {
                        let attachments = state.detail.as_ref().map(|detail| detail.attachments.clone()).unwrap_or_default();
                        let backlinks = state.detail.as_ref().map(|detail| detail.backlinks.clone()).unwrap_or_default();
                        state.detail = Some(NoteDetail { note: saved.note.clone(), attachments, backlinks });
                    }
                    let Some(edit) = &mut state.edit else { return };
                    edit.saving = false;
                    let still_writing = edit.body != sent_body || edit.title != sent_title;
                    edit.base_hash = saved.note.body_hash.clone();
                    // Con choque, lo escrito no se aplicó: el servidor
                    // conserva lo suyo.
                    edit.base_body = if has_conflict { saved.note.body.clone() } else { sent_body.clone() };
                    edit.base_title = if has_conflict { saved.note.title.clone() } else { sent_title.clone() };
                    if !still_writing {
                        state.edit = None;
                    }
                });
                // El árbol lleva títulos, si tienen texto y las hijas: se
                // vuelve a pedir.
                self.load_tree().await;
            }
            Err(error) => self.update(|state| {
                if let Some(edit) = &mut state.edit {
                    edit.saving = false;
                    edit.error = Some(DocEditError::from(&error));
                }
            }),
        }
    }

    /// Contesta la pregunta que haya. Con un no todo sigue como estaba.
    pub async fn answer(&self, yes: bool) {
        let Some(asking) = self.state().asking else { return };
        self.update(|state| state.asking = None);
        if !yes {
            return;
        }
        match asking {
            NoteAsk::Leave { to } => {
                self.update(|state| state.edit = None);
                match to {
                    NoteLeave::Cancel => {}
                    NoteLeave::Close => self.close_now(),
                    NoteLeave::Open { id } => self.reload(&id).await,
                    NoteLeave::Create { parent_id, title } => self.create_now(parent_id, &title).await,
                    NoteLeave::LogOut => self.core.logout().await,
                }
            }
            NoteAsk::Delete { id } => self.delete_now(&id).await,
            NoteAsk::Purge { id } => self.purge_now(&id).await,
            NoteAsk::EmptyTrash => self.empty_trash_now().await,
        }
    }

    pub fn ask_delete(&self, id: &str) {
        self.update(|state| state.asking = Some(NoteAsk::Delete { id: id.to_string() }));
    }

    async fn delete_now(&self, id: &str) {
        let state = self.state();
        let doomed = state.subtree_of(id);
        let reply = self.core.request("DELETE", &format!("/api/v1/notes/{id}"), None, true).await;
        match reply {
            Ok(reply) if reply.status < 400 => {
                if state.open_id.as_ref().is_some_and(|open| doomed.contains(open)) {
                    self.close_now();
                }
                self.load_tree().await;
            }
            Ok(reply) => self.fail("work:notes.errDelete", &crate::session::server_error(&reply)),
            Err(error) => self.fail("work:notes.errDelete", &error),
        }
    }

    /// Estrella o desestrella, y se ve en el acto: es un bit que quien mira
    /// ve cambiar, y esperar al servidor se leería como un clic perdido.
    pub async fn toggle_favorite(&self, id: &str) {
        let Some(now) = self.state().tree.iter().find(|item| item.id == id).map(|item| item.favorite) else { return };
        let next = !now;
        let mark = |state: &mut NotesState, value: bool| {
            for item in state.tree.iter_mut().filter(|item| item.id == id) {
                item.favorite = value;
            }
            if let Some(detail) = state.detail.as_mut().filter(|detail| detail.note.id == id) {
                detail.note.favorite = value;
            }
        };
        self.update(|state| mark(state, next));
        if let Err(error) = self.core.call::<serde_json::Value>("PATCH", &format!("/api/v1/notes/{id}"), Some(json!({ "favorite": next }))).await {
            self.update(|state| mark(state, now));
            self.fail("work:notes.errMove", &error);
        }
    }

    /// Sube o baja una nota un puesto entre sus hermanas.
    pub async fn move_by_one(&self, id: &str, up: bool) {
        let Some(moves) = swapped(&self.state().tree, id, up) else { return };
        let before = self.state().tree;
        self.update(|state| {
            for (moved, parent, position) in &moves {
                if let Some(item) = state.tree.iter_mut().find(|item| &item.id == moved) {
                    item.parent_id = parent.clone();
                    item.position = *position;
                }
            }
        });
        let body: Vec<serde_json::Value> = moves.iter().map(|(id, parent, position)| json!({ "id": id, "parentId": parent, "position": position })).collect();
        if let Err(error) = self.core.call_ok("PUT", "/api/v1/notes/tree", Some(json!(body))).await {
            self.update(|state| state.tree = before);
            self.fail("work:notes.errMove", &error);
        }
    }

    pub fn open_search(&self) {
        self.update(|state| state.search = NoteSearch { open: true, ..NoteSearch::default() });
    }

    pub fn close_search(&self) {
        self.change(|data| {
            data.search_turn += 1;
            data.state.search = NoteSearch::default();
        });
    }

    /// Busca por título y contenido; sin red, por título entre lo que ya se
    /// tiene.
    pub async fn search(&self, query: &str) {
        let turn = self.change(|data| {
            data.search_turn += 1;
            data.state.search.query = query.to_string();
            data.state.search.searching = !query.trim().is_empty();
            if query.trim().is_empty() {
                data.state.search.results.clear();
            }
            data.search_turn
        });
        if query.trim().is_empty() {
            return;
        }
        let path = format!("/api/v1/notes/search?q={}", encode_query(query));
        let found = match self.core.get_list::<NoteLink>(&path).await {
            Ok(found) => found,
            Err(ApiError::Network(_)) => titles_matching(&self.state().tree, query),
            Err(_) => Vec::new(),
        };
        if self.data().search_turn != turn {
            return;
        }
        self.update(|state| {
            state.search.results = found;
            state.search.searching = false;
        });
    }

    pub async fn open_trash(&self) {
        self.update(|state| state.trash.open = true);
        self.load_trash().await;
    }

    pub fn close_trash(&self) {
        self.update(|state| state.trash.open = false);
    }

    async fn load_trash(&self) {
        self.update(|state| state.trash.loading = true);
        match self.core.get_list::<NoteTrashItem>("/api/v1/notes/trash").await {
            Ok(items) => self.update(|state| {
                state.trash.items = items;
                state.trash.loading = false;
            }),
            Err(error) => {
                self.update(|state| state.trash.loading = false);
                self.fail("work:notes.errRestore", &error);
            }
        }
    }

    /// Recupera una página y lo que colgaba de ella.
    pub async fn restore(&self, id: &str) {
        match self.core.call_ok("POST", &format!("/api/v1/notes/trash/{id}/restore"), Some(json!({}))).await {
            Ok(()) => {
                self.load_trash().await;
                self.load_tree().await;
            }
            Err(error) => self.fail("work:notes.errRestore", &error),
        }
    }

    pub fn ask_purge(&self, id: &str) {
        self.update(|state| state.asking = Some(NoteAsk::Purge { id: id.to_string() }));
    }

    pub fn ask_empty_trash(&self) {
        self.update(|state| state.asking = Some(NoteAsk::EmptyTrash));
    }

    async fn purge_now(&self, id: &str) {
        match self.core.call_ok("DELETE", &format!("/api/v1/notes/trash/{id}"), None).await {
            Ok(()) => self.load_trash().await,
            Err(error) => self.fail("work:notes.errDelete", &error),
        }
    }

    async fn empty_trash_now(&self) {
        match self.core.call_ok("DELETE", "/api/v1/notes/trash", None).await {
            Ok(()) => self.load_trash().await,
            Err(error) => self.fail("work:notes.errEmptyTrash", &error),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SaveResult {
    note: Note,
    #[serde(default)]
    conflict: Option<ConflictInfo>,
}

#[derive(Debug, Deserialize)]
struct ConflictInfo {
    id: String,
    #[serde(default)]
    title: String,
}

/// Sin red, la búsqueda cae a los títulos que ya se tienen.
fn titles_matching(tree: &[NoteTreeItem], query: &str) -> Vec<NoteLink> {
    let wanted = query.to_lowercase();
    tree.iter()
        .filter(|item| item.title.to_lowercase().contains(&wanted))
        .take(OFFLINE_MATCHES)
        .map(|item| NoteLink { id: item.id.clone(), title: item.title.clone(), excerpt: String::new() })
        .collect()
}

/// Lo que va tras `?q=`: todo menos lo que una dirección admite tal cual.
fn encode_query(query: &str) -> String {
    let mut out = String::new();
    for byte in query.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, parent: Option<&str>, position: i64, title: &str) -> NoteTreeItem {
        NoteTreeItem { id: id.into(), parent_id: parent.map(str::to_string), position, title: title.into(), ..Default::default() }
    }

    fn state(tree: Vec<NoteTreeItem>) -> NotesState {
        NotesState { tree, ..Default::default() }
    }

    fn titles(rows: &[NoteRow]) -> Vec<(String, u32)> {
        rows.iter().map(|row| (row.title.clone(), row.depth)).collect()
    }

    /// El árbol se pinta por posición, cada nota seguida de sus hijas, y lo
    /// plegado esconde lo suyo.
    ///
    /// Roturas que atrapa: ordenar por título o por llegada en vez de por
    /// posición; pintar las hijas lejos de su madre; que plegar no esconda
    /// nada (o esconda también a las hermanas); marcar como desplegable una
    /// nota sin hijas.
    #[test]
    fn the_tree_is_laid_out_by_position_with_children_under_their_parent() {
        let mut notes = state(vec![item("b", None, 2, "Beta"), item("a", None, 1, "Alfa"), item("a2", Some("a"), 5, "Alfa dos"), item("a1", Some("a"), 3, "Alfa uno")]);
        let rows = note_rows(&notes);
        assert_eq!(titles(&rows), [("Alfa".to_string(), 0), ("Alfa uno".into(), 1), ("Alfa dos".into(), 1), ("Beta".into(), 0)]);
        assert_eq!(rows.iter().map(|row| row.expandable).collect::<Vec<_>>(), [true, false, false, false]);
        notes.folded.insert("a".into(), true);
        let rows = note_rows(&notes);
        assert_eq!(titles(&rows), [("Alfa".to_string(), 0), ("Beta".into(), 0)]);
        assert!(!rows[0].open);
    }

    /// Lo que cuelga de una nota, a cualquier profundidad, y sin ella misma
    /// al contarlo.
    ///
    /// Roturas que atrapa: contarse a sí misma entre sus subpáginas; quedarse
    /// en el primer nivel (el aviso de borrado diría menos de lo que se va).
    #[test]
    fn the_subtree_counts_every_level_but_not_itself() {
        let notes = state(vec![item("a", None, 0, ""), item("b", Some("a"), 0, ""), item("c", Some("b"), 0, ""), item("d", None, 1, "")]);
        assert_eq!(notes.subtree_of("a"), ["a", "b", "c"]);
        assert_eq!(notes.subpage_count("a"), 2);
        assert_eq!(notes.subpage_count("d"), 0);
    }

    /// Las hijas que el texto no enlaza salen al pie; las que sí, no.
    ///
    /// Roturas que atrapa: enseñar dos veces la que el texto ya enlaza;
    /// esconder la que no; mirar el texto guardado y no el que se escribe.
    #[test]
    fn children_the_text_does_not_link_are_listed_below() {
        let mut notes = state(vec![item("p", None, 0, "P"), item("x1", Some("p"), 0, "Uno"), item("x2", Some("p"), 1, "Dos")]);
        notes.open_id = Some("p".into());
        notes.detail = Some(NoteDetail { note: Note { id: "p".into(), body: "mira [Uno](/notes/x1)".into(), ..Default::default() }, ..Default::default() });
        assert_eq!(notes.unlinked_children().iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["x2"]);
        notes.edit = Some(NoteEdit::begin(&notes.detail.as_ref().unwrap().note, false));
        notes.edit.as_mut().unwrap().body = "mira [Dos](/notes/x2)".into();
        assert_eq!(notes.unlinked_children().iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["x1"]);
    }

    /// Un enlace a otra nota se reconoce por su ruta, con o sin lo que le
    /// siga.
    ///
    /// Roturas que atrapa: abrir como nota cualquier enlace; llevarse la
    /// cola de `?x` o `#y` en el id; tomar `/notes/` vacío por una nota.
    #[test]
    fn a_link_to_another_note_is_known_by_its_path() {
        assert_eq!(note_link_target("/notes/abc"), Some("abc".into()));
        assert_eq!(note_link_target("/notes/abc?x=1#y"), Some("abc".into()));
        assert_eq!(note_link_target("/notes/"), None);
        assert_eq!(note_link_target("https://example.com/notes/abc"), None);
        assert_eq!(note_link_target("/tasks/abc"), None);
    }

    /// Subir o bajar renumera a todas las hermanas, y en el borde no hace
    /// nada.
    ///
    /// Roturas que atrapa: intercambiar sólo dos posiciones (con huecos o
    /// repetidas no cambia el orden); moverla en el sentido contrario;
    /// mezclar hermanas de otra rama; salirse del borde.
    #[test]
    fn moving_by_one_renumbers_the_siblings() {
        let tree = vec![item("a", None, 7, ""), item("b", None, 7, ""), item("c", None, 9, ""), item("z", Some("a"), 0, "")];
        let order = |moves: Vec<(String, Option<String>, i64)>| moves.into_iter().map(|(id, _, position)| (id, position)).collect::<Vec<_>>();
        assert_eq!(order(swapped(&tree, "c", true).unwrap()), [("a".to_string(), 0), ("c".into(), 1), ("b".into(), 2)]);
        assert_eq!(order(swapped(&tree, "a", false).unwrap()), [("b".to_string(), 0), ("a".into(), 1), ("c".into(), 2)]);
        assert!(swapped(&tree, "a", true).is_none());
        assert!(swapped(&tree, "c", false).is_none());
        assert!(swapped(&tree, "z", true).is_none(), "sola entre sus hermanas");
    }

    /// Lo que se busca llega a la dirección con lo que no cabe en ella
    /// escapado.
    ///
    /// Rotura que atrapa: mandar `&` o `#` sin escapar (cortaría la búsqueda)
    /// o los acentos como bytes sueltos.
    #[test]
    fn the_search_text_is_escaped_for_the_address() {
        assert_eq!(encode_query("a b&c#é"), "a%20b%26c%23%C3%A9");
        assert_eq!(encode_query("plan-1_v2.x~"), "plan-1_v2.x~");
    }

    /// Sin red se busca por título entre lo que ya se tiene, sin distinguir
    /// mayúsculas, y con tope.
    ///
    /// Roturas que atrapa: distinguir mayúsculas (no se encontraría «alfa»
    /// teniendo «Alfa»); buscar en lo que no es el título; no poner tope.
    #[test]
    fn offline_search_matches_titles_without_case_and_is_capped() {
        let tree: Vec<NoteTreeItem> = (0..60).map(|n| item(&format!("n{n}"), None, n, &format!("Plan {n}"))).chain([item("x", None, 0, "Otra")]).collect();
        assert_eq!(titles_matching(&tree, "PLAN 1").iter().map(|found| found.id.as_str()).collect::<Vec<_>>(), ["n1", "n10", "n11", "n12", "n13", "n14", "n15", "n16", "n17", "n18", "n19"]);
        assert_eq!(titles_matching(&tree, "plan").len(), OFFLINE_MATCHES);
        assert!(titles_matching(&tree, "zzz").is_empty());
    }

    /// Cuándo se borró sale con el día de quien mira; sin fecha legible, nada.
    ///
    /// Roturas que atrapa: pintar la cadena cruda del servidor; romper con
    /// una fecha ilegible en vez de callar.
    #[test]
    fn the_deletion_time_reads_as_a_date_or_says_nothing() {
        let item = |at: &str| NoteTrashItem { deleted_at: at.into(), ..Default::default() };
        let said = item("2026-11-25T12:00:00Z").deleted_on(Locale::En);
        assert!(!said.is_empty() && !said.contains("2026-11-25T"), "{said}");
        assert_eq!(item("ayer").deleted_on(Locale::En), "");
    }
}
