//! Escribir la documentación: una pestaña de la portada, una página, o una
//! página nueva.
//!
//! El borrador vive aquí, no en la pantalla: qué se escribe, si hay algo sin
//! guardar, qué se manda, qué pasa si alguien guardó en medio y qué se
//! pregunta antes de perder texto lo decide este fichero. La pantalla pone
//! las cajas y cuenta cada tecla.
//!
//! Tres reglas que no se negocian:
//!
//! - **Lo escrito no se pierde sin preguntar.** Todo lo que saca del editor
//!   con cambios sin guardar —otra pestaña, otra página, otro nodo, volver a
//!   las tareas, cambiar de organización, cerrar la sesión, «Cancelar»— se
//!   queda en una pregunta, y sólo se hace si contestan que sí. (Cerrar la
//!   ventana no pasa por aquí: eso es de cada plataforma.)
//! - **Un guardado que falla deja el texto donde estaba**, con el motivo al
//!   lado. También el que llega tarde.
//! - **No se pisa a nadie.** Cada guardado dice de qué versión salió
//!   (`baseHash`, el `bodyHash` que se leyó); si el servidor ya tiene otra,
//!   contesta 409 con la suya y aquí se enseña el choque en vez de escribir
//!   encima (`domain.DocSaveConflicts`).
//!
//! En la app de siempre esto se guarda solo, cada pocos segundos
//! (`use-autoguardado`). Aquí se guarda con su botón: decidido con jose el
//! 18-nov-2026, junto con que salir sin guardar pregunta.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::doc_header::DocInfo;
use crate::doc_pages::DocPage;
use crate::docs::{kind_segment, DocDecision, DocTab, Docs, DocsState};
use crate::error::ApiError;
use crate::prefs::WorkLayout;
use crate::session::server_error;
use crate::task::Question;
use crate::texts::{error_phrase, text, text_with, Locale};
use crate::Core;

/// La pestaña que no es texto: el registro de decisiones sólo crece, entrada
/// a entrada, y no se edita como un markdown.
const LOG_TAB: &str = "decisions";

/// Desde qué ancho de ventana caben la caja y su vista previa lado a lado,
/// con el árbol puesto al lado (que se lleva unos 340 px): a cada una le
/// quedan más de 400. Por debajo se ve una u otra, con su conmutador.
pub const SIDE_BY_SIDE_FROM: f32 = 1200.;

/// Si en una ventana de ese ancho la caja y la vista previa van lado a lado.
pub fn doc_editor_side_by_side(width: f32) -> bool {
    width >= SIDE_BY_SIDE_FROM
}

/// Cada vez que se abre el editor es una sesión distinta: con este número
/// la pantalla sabe que tiene que poner en sus cajas el texto del borrador
/// (y no hacerlo en cada tecla). Es del proceso y no del documento: abrir
/// otro nodo vacía el estado, y un contador que empezara de nuevo repetiría
/// número.
static SESSIONS: AtomicU64 = AtomicU64::new(1);

/// Qué se está escribiendo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DocEditWhat {
    /// Una pestaña de la portada, por su clave.
    Tab { key: String },
    /// Una página que ya existe: su título y su texto.
    Page { id: String },
    /// Una página que todavía no existe: no se crea hasta guardar.
    NewPage { parent_id: Option<String> },
}

/// A dónde se iba cuando había texto sin guardar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DocLeave {
    /// «Cancelar»: se deja de escribir y se queda donde estaba.
    Cancel,
    /// Cerrar la documentación.
    Close,
    /// Volver a las tareas del nodo, en esa vista si se eligió una.
    Tasks { layout: Option<WorkLayout> },
    /// «Mi trabajo» entero.
    AllWork,
    /// Otra organización.
    Org { id: String },
    /// Cerrar la sesión.
    LogOut,
    /// La documentación de otro nodo.
    Node { id: String },
    /// Otra pestaña de la portada.
    Tab { key: String },
    /// Otra página.
    Page { id: String },
    /// La portada.
    Home,
    /// Una página nueva.
    NewPage { parent_id: Option<String> },
    /// A donde lleve un enlace del propio texto (ver `doc_links`).
    Link { href: String },
}

/// Lo que se está preguntando sobre lo que se escribe.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DocAsk {
    /// Irse y perder lo escrito.
    Leave { to: DocLeave },
    /// Tras un choque: tirar lo propio y quedarse con lo que hay guardado.
    LoadTheirs,
    /// Tras un choque: guardar lo propio encima de lo que hay guardado.
    Overwrite,
}

/// Lo que el servidor tiene ahora, cuando no es lo que se leyó.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocConflict {
    /// Markdown.
    pub body: String,
    /// De qué versión es: con él, guardar encima es una decisión y no un
    /// accidente.
    pub hash: Option<String>,
    /// Quién lo guardó, si el servidor lo dice.
    pub by_name: Option<String>,
}

/// Por qué no se guardó.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocEditError {
    /// No hubo respuesta.
    pub offline: bool,
    /// La etiqueta del servidor (`body-too-large`).
    pub code: String,
    /// Su frase, por si el catálogo no conoce la etiqueta.
    pub message: String,
}

impl DocEditError {
    fn from(error: &ApiError) -> Self {
        match error {
            ApiError::Server { code, message, .. } => Self { offline: false, code: code.clone(), message: message.clone() },
            ApiError::Network(_) => Self { offline: true, ..Self::default() },
            other => Self { offline: false, code: String::new(), message: other.to_string() },
        }
    }
}

/// Lo que se está escribiendo, con de dónde salió.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocEdit {
    /// Ver `SESSIONS`.
    pub session: u64,
    pub what: DocEditWhat,
    /// Lo que había al empezar (o tras el último guardado): con qué se
    /// compara para saber si hay algo sin guardar.
    pub base_title: String,
    pub base_body: String,
    /// El `bodyHash` de lo que se leyó. `None` en lo que nunca se guardó.
    pub base_hash: Option<String>,
    /// El borrador. El título sólo cuenta en una página.
    pub title: String,
    pub body: String,
    pub saving: bool,
    pub error: Option<DocEditError>,
    pub conflict: Option<DocConflict>,
    /// Tras un choque: si al lado se enseña lo que hay guardado en vez de
    /// la vista previa de lo propio.
    pub showing_theirs: bool,
    /// En una ventana estrecha: si se mira la vista previa en vez de la caja.
    pub previewing: bool,
    pub asking: Option<DocAsk>,
}

impl DocEdit {
    fn begin(what: DocEditWhat, title: &str, body: &str, hash: Option<String>) -> Self {
        Self {
            session: SESSIONS.fetch_add(1, Ordering::Relaxed),
            what,
            base_title: title.to_string(),
            base_body: body.to_string(),
            base_hash: hash,
            title: title.to_string(),
            body: body.to_string(),
            saving: false,
            error: None,
            conflict: None,
            showing_theirs: false,
            previewing: false,
            asking: None,
        }
    }

    /// Llegó del servidor lo que hay ahora de esto que se escribe. Si ya no
    /// es la versión de la que salió el borrador, alguien guardó en medio:
    /// se dice con el aviso de choque —lo suyo queda a mano para mirarlo— y
    /// **el texto no se toca**, se haya escrito algo o no. Mientras viaja un
    /// guardado propio no se mira: su respuesta es la que lo dice.
    pub(crate) fn changed_underneath(&mut self, body: &str, hash: Option<String>, by_name: Option<String>) {
        if self.saving || hash == self.base_hash {
            return;
        }
        // Si ya había choque y volvió a cambiar, el aviso enseña lo último.
        self.conflict = Some(DocConflict { body: body.to_string(), hash, by_name });
    }

    /// Si se escribe una página (que lleva título) y no una pestaña.
    pub fn has_title(&self) -> bool {
        !matches!(self.what, DocEditWhat::Tab { .. })
    }

    /// Si hay algo escrito que el servidor no tiene.
    pub fn is_dirty(&self) -> bool {
        self.body != self.base_body || (self.has_title() && self.title != self.base_title)
    }

    /// Si «Guardar» hace algo. Con un choque a la vista no: primero hay que
    /// decidir qué pasa con lo que hay guardado.
    pub fn can_save(&self) -> bool {
        self.is_dirty() && !self.saving && self.conflict.is_none()
    }

    /// Lo que se enseña al lado de la caja: lo propio, ya pintado; o, tras
    /// un choque y si se pidió, lo que hay guardado.
    pub fn preview(&self) -> &str {
        match (&self.conflict, self.showing_theirs) {
            (Some(conflict), true) => &conflict.body,
            _ => &self.body,
        }
    }

    /// En qué está lo escrito, para decirlo junto a los botones: guardando,
    /// o con cambios sin guardar. `None` si no hay nada que decir.
    pub fn status(&self, locale: Locale) -> Option<String> {
        if self.saving {
            Some(text(locale, "work:docs.save_guardando"))
        } else if self.is_dirty() {
            Some(text(locale, "work:docs.unsaved"))
        } else {
            None
        }
    }

    /// Por qué no se guardó, dicho.
    pub fn error_phrase(&self, locale: Locale) -> Option<String> {
        let error = self.error.as_ref()?;
        let fallback = if matches!(self.what, DocEditWhat::NewPage { .. }) { "work:docs.errCreatePage" } else { "work:docs.errSave" };
        let error = if error.offline {
            ApiError::Network(String::new())
        } else {
            ApiError::Server { status: 0, code: error.code.clone(), message: error.message.clone() }
        };
        Some(error_phrase(locale, &error, Some(fallback)))
    }

    /// El choque, dicho: quién llegó antes si se sabe, y que lo escrito
    /// sigue aquí.
    pub fn conflict_phrase(&self, locale: Locale) -> Option<String> {
        let conflict = self.conflict.as_ref()?;
        Some(match conflict.by_name.as_deref().filter(|name| !name.trim().is_empty()) {
            Some(who) => text_with(locale, "work:docs.conflictKeptBy", &[("who", who)]),
            None => text(locale, "work:docs.conflictKept"),
        })
    }

    /// Lo que se pregunta ahora mismo, con sus dos botones. El primero es
    /// siempre el que pierde o pisa algo.
    pub fn question(&self, locale: Locale) -> Option<Question> {
        let (title, body, confirm) = match self.asking.as_ref()? {
            DocAsk::Leave { .. } => ("work:docs.discardTitle", "work:docs.discardWhy", "work:docs.discard"),
            DocAsk::LoadTheirs => ("work:docs.loadTheirsTitle", "work:docs.discardWhy", "work:docs.discardAndReload"),
            DocAsk::Overwrite => ("work:docs.overwriteTitle", "work:docs.overwriteWhy", "work:docs.overwrite"),
        };
        Some(Question {
            title: text(locale, title),
            body: text(locale, body),
            warning: None,
            confirm: text(locale, confirm),
            cancel: text(locale, "work:docs.keepEditing"),
        })
    }
}

impl DocsState {
    /// Si quien mira puede escribir **esto** que tiene delante, ahora: su
    /// papel le deja, no se está escribiendo ya, y lo que hay es texto que
    /// llegó (una pestaña que no es el registro, o una página abierta).
    pub fn editable(&self) -> bool {
        if !self.can_edit || self.edit.is_some() || self.target.is_none() {
            return false;
        }
        if self.on_page() {
            return self.page.is_some();
        }
        // Un documento que no llegó no se edita: no se sabe qué hay.
        self.tab != LOG_TAB && !(self.loading && self.tabs.is_empty()) && self.error.is_none()
    }

    /// Si quien mira puede crear páginas en este documento.
    pub fn can_add_pages(&self) -> bool {
        self.can_edit && self.target.is_some()
    }

    /// Lo que se dice encima de una página que todavía no existe: «Página
    /// nueva», o «Página nueva dentro» si va a colgar de otra. Lo demás ya
    /// dice qué es: una pestaña, con su pestaña marcada; una página, con
    /// sus migas.
    pub fn edit_heading(&self, locale: Locale) -> Option<String> {
        match &self.edit.as_ref()?.what {
            DocEditWhat::NewPage { parent_id: None } => Some(text(locale, "work:docs.newPage")),
            DocEditWhat::NewPage { parent_id: Some(_) } => Some(text(locale, "work:docs.newSubpage")),
            DocEditWhat::Tab { .. } | DocEditWhat::Page { .. } => None,
        }
    }
}

/// Lo que contesta el servidor al guardar una pestaña: el documento entero.
/// También dentro de un 409, con lo que hay ahora.
#[derive(Debug, Default, Deserialize)]
struct DocAnswer {
    /// El documento en sí: el primer guardado lo crea.
    #[serde(default)]
    doc: Option<DocInfo>,
    #[serde(default)]
    tabs: Vec<DocTab>,
    #[serde(default)]
    decisions: Vec<DocDecision>,
}

/// Cómo acabó un guardado.
enum Saved {
    Tab(DocAnswer),
    Page(DocPage),
    Created(DocPage),
}

/// El `data` de una respuesta, leído con su tipo. `None` si no viene o no
/// encaja.
fn data_of<T: for<'de> Deserialize<'de>>(body: &str) -> Option<T> {
    let mut envelope: Value = serde_json::from_str(body).ok()?;
    serde_json::from_value(envelope.get_mut("data")?.take()).ok()
}

impl Core {
    /// De la documentación a las tareas del mismo nodo, en esa vista. Si
    /// hay texto sin guardar se pregunta antes, y la vista no se cambia
    /// hasta que contesten.
    pub fn back_to_tasks_in(&self, layout: WorkLayout) {
        if self.docs().held_back(DocLeave::Tasks { layout: Some(layout) }) {
            return;
        }
        let _ = self.set_work_layout(layout);
        self.back_to_tasks();
    }
}

impl Docs {
    /// Si esto que se iba a hacer se queda en una pregunta, porque hay texto
    /// sin guardar. Si no lo hay, el editor (limpio) se cierra y se sigue.
    pub(crate) fn held_back(&self, to: DocLeave) -> bool {
        let Some(edit) = self.state().edit else { return false };
        if edit.is_dirty() {
            self.update(|state| {
                if let Some(edit) = &mut state.edit {
                    edit.asking = Some(DocAsk::Leave { to });
                }
            });
            return true;
        }
        self.update(|state| state.edit = None);
        false
    }

    fn change_edit(&self, change: impl FnOnce(&mut DocEdit)) {
        if self.state().edit.is_some() {
            self.update(|state| {
                if let Some(edit) = &mut state.edit {
                    change(edit);
                }
            });
        }
    }

    /// Empieza a escribir lo que se tiene delante: la pestaña que se lee o
    /// la página abierta. Dice si empezó.
    pub fn start_edit(&self) -> bool {
        self.start_edit_with(None)
    }

    /// Lo mismo, partiendo de una plantilla de página (`component`,
    /// `procedure`) si la página está en blanco: su texto queda escrito en
    /// la caja y **no se guarda** hasta que se pulse guardar.
    pub fn start_from_template(&self, template: &str, locale: Locale) -> bool {
        let key = format!("work:pageTemplates.{template}.body");
        let body = text(locale, &key);
        // Una plantilla que el catálogo no tiene devuelve su clave: no se
        // escribe eso en un documento.
        if body == key || !self.state().body().is_empty() {
            return false;
        }
        self.start_edit_with(Some(body))
    }

    fn start_edit_with(&self, draft: Option<String>) -> bool {
        let state = self.state();
        if !state.editable() {
            return false;
        }
        // `editable` ya dijo que, si se lee una página, llegó.
        let mut edit = match &state.page {
            Some(view) if state.on_page() => {
                DocEdit::begin(DocEditWhat::Page { id: view.page.id.clone() }, &view.page.title, &view.page.body, view.page.body_hash.clone())
            }
            _ => {
                let tab = state.tabs.iter().find(|tab| tab.key == state.tab);
                let (body, hash) = tab.map(|tab| (tab.body.as_str(), tab.body_hash.clone())).unwrap_or(("", None));
                DocEdit::begin(DocEditWhat::Tab { key: state.tab.clone() }, "", body, hash)
            }
        };
        if let Some(draft) = draft {
            edit.body = draft;
        }
        self.update(|state| state.edit = Some(edit));
        true
    }

    /// Empieza una página nueva, colgada de la portada o de otra página.
    /// **No crea nada**: la página existe cuando se guarda, con el título y
    /// el texto que se le hayan puesto.
    pub fn new_page(&self, parent_id: Option<&str>) {
        let state = self.state();
        if !state.can_add_pages() {
            return;
        }
        let parent_id = parent_id.map(str::to_string);
        if self.held_back(DocLeave::NewPage { parent_id: parent_id.clone() }) {
            return;
        }
        self.begin_new_page(parent_id);
    }

    fn begin_new_page(&self, parent_id: Option<String>) {
        self.update(|state| {
            state.edit = Some(DocEdit::begin(DocEditWhat::NewPage { parent_id }, "", "", None));
            // Se eligió desde la hoja: ya no hace falta encima.
            state.pages_shown = false;
        });
    }

    /// Lo que la caja tiene escrito ahora.
    pub fn draft_body(&self, body: &str) {
        self.change_edit(|edit| edit.body = body.to_string());
    }

    /// Lo que la caja del título tiene escrito ahora.
    pub fn draft_title(&self, title: &str) {
        self.change_edit(|edit| edit.title = title.to_string());
    }

    /// En una ventana estrecha: la vista previa, o la caja.
    pub fn show_preview(&self, on: bool) {
        self.change_edit(|edit| edit.previewing = on);
    }

    /// Tras un choque: al lado, lo que hay guardado; o lo propio.
    pub fn show_theirs(&self, on: bool) {
        self.change_edit(|edit| edit.showing_theirs = on);
    }

    /// «Cancelar»: sin cambios, cierra el editor; con ellos, pregunta.
    pub fn cancel_edit(&self) {
        self.held_back(DocLeave::Cancel);
    }

    /// Tras un choque, pregunta si se tira lo propio para quedarse con lo
    /// que hay guardado.
    pub fn ask_load_theirs(&self) {
        self.change_edit(|edit| {
            if edit.conflict.is_some() {
                edit.asking = Some(DocAsk::LoadTheirs);
            }
        });
    }

    /// Tras un choque, pregunta si se guarda lo propio encima.
    pub fn ask_overwrite(&self) {
        self.change_edit(|edit| {
            if edit.conflict.is_some() {
                edit.asking = Some(DocAsk::Overwrite);
            }
        });
    }

    /// Contesta la pregunta que haya. Con un no, todo sigue como estaba:
    /// el editor abierto, con su texto.
    pub async fn answer(&self, yes: bool) {
        let Some(edit) = self.state().edit else { return };
        let Some(asking) = edit.asking else { return };
        if !yes {
            self.change_edit(|edit| edit.asking = None);
            return;
        }
        match asking {
            DocAsk::Leave { to } => {
                self.update(|state| state.edit = None);
                self.leave(to).await;
            }
            DocAsk::LoadTheirs => {
                self.update(|state| state.edit = None);
                self.reload_what_is_read().await;
            }
            DocAsk::Overwrite => {
                // Lo propio sale ahora de la versión que hay: si entre
                // tanto vuelve a cambiar, vuelve a chocar, y no se pisa a
                // un tercero sin verlo.
                self.change_edit(|edit| {
                    edit.asking = None;
                    if let Some(conflict) = edit.conflict.take() {
                        edit.base_hash = conflict.hash;
                    }
                });
                self.save().await;
            }
        }
    }

    /// Hace lo que se iba a hacer, ya sin editor.
    async fn leave(&self, to: DocLeave) {
        let core = &self.core;
        match to {
            DocLeave::Cancel => {}
            DocLeave::Close => self.close(),
            DocLeave::Tasks { layout } => {
                if let Some(layout) = layout {
                    let _ = core.set_work_layout(layout);
                }
                core.back_to_tasks();
            }
            DocLeave::AllWork => core.show_all_work(),
            DocLeave::Org { id } => {
                core.switch_org(&id).await;
            }
            DocLeave::LogOut => core.logout().await,
            DocLeave::Node { id } => self.open(&id).await,
            DocLeave::Tab { key } => self.show_tab(&key),
            DocLeave::Page { id } => self.open_page(&id).await,
            DocLeave::Home => self.close_page(),
            DocLeave::NewPage { parent_id } => self.begin_new_page(parent_id),
            DocLeave::Link { href } => self.follow_link(&href).await,
        }
    }

    /// Lo que se lee, otra vez del servidor.
    async fn reload_what_is_read(&self) {
        let state = self.state();
        let Some(target) = state.target else { return };
        self.open(&target.id).await;
    }

    /// Guarda lo escrito.
    ///
    /// - Una pestaña: `PUT /api/v1/docs/{kind}/{id}/tabs/{tab}` con
    ///   `{body, baseHash}`.
    /// - Una página: `PUT …/pages/{pageId}` con sólo lo que cambió: `title`,
    ///   y `body` con su `baseHash`.
    /// - Una página nueva: `POST …/pages` con `{title, parentId, body}`.
    ///
    /// Si sale bien y no se escribió nada más mientras viajaba, el editor
    /// se cierra y se lee lo guardado. Si se siguió escribiendo, se queda
    /// abierto con eso. Si el servidor ya tenía otra versión (409), el
    /// texto se queda y se enseña el choque. Cualquier otro fallo, lo
    /// mismo, con su motivo.
    pub async fn save(&self) {
        let state = self.state();
        let (Some(target), Some(edit)) = (state.target, state.edit) else { return };
        if edit.saving || edit.conflict.is_some() {
            return;
        }
        if !edit.is_dirty() {
            // Nada que mandar: guardar lo que ya está guardado es cerrar.
            self.update(|state| state.edit = None);
            return;
        }
        self.change_edit(|edit| {
            edit.saving = true;
            edit.error = None;
        });

        let root = format!("/api/v1/docs/{}/{}", kind_segment(target.kind), target.id);
        let (method, path, body) = match &edit.what {
            DocEditWhat::Tab { key } => {
                let mut body = json!({ "body": edit.body });
                if let Some(hash) = &edit.base_hash {
                    body["baseHash"] = json!(hash);
                }
                ("PUT", format!("{root}/tabs/{key}"), body)
            }
            DocEditWhat::Page { id } => {
                // Lo que no va no se toca: renombrar no manda el texto, y
                // así no puede chocar con quien lo esté escribiendo.
                let mut body = json!({});
                if edit.title != edit.base_title {
                    body["title"] = json!(edit.title.trim());
                }
                if edit.body != edit.base_body {
                    body["body"] = json!(edit.body);
                    if let Some(hash) = &edit.base_hash {
                        body["baseHash"] = json!(hash);
                    }
                }
                ("PUT", format!("{root}/pages/{id}"), body)
            }
            DocEditWhat::NewPage { parent_id } => {
                let mut body = json!({ "title": edit.title.trim(), "body": edit.body });
                if let Some(parent) = parent_id {
                    body["parentId"] = json!(parent);
                }
                ("POST", format!("{root}/pages"), body)
            }
        };

        let outcome = match self.core.request(method, &path, Some(body.to_string()), true).await {
            Err(error) => Err((error, None)),
            Ok(reply) if reply.status < 400 => {
                let saved = match &edit.what {
                    // Sin la pestaña que se guardó dentro, no se sabe de qué
                    // versión es lo que quedó.
                    DocEditWhat::Tab { key } => {
                        data_of::<DocAnswer>(&reply.body).filter(|answer| answer.tabs.iter().any(|tab| &tab.key == key)).map(Saved::Tab)
                    }
                    DocEditWhat::Page { .. } => data_of::<DocPage>(&reply.body).map(Saved::Page),
                    DocEditWhat::NewPage { .. } => data_of::<DocPage>(&reply.body).map(Saved::Created),
                };
                // Un 200 que no se entiende no se da por guardado.
                saved.ok_or_else(|| (server_error(&reply), None))
            }
            Ok(reply) => {
                let error = server_error(&reply);
                let late = matches!(&error, ApiError::Server { code, .. } if code == "doc-conflict" || code == "doc-page-conflict");
                let conflict = (reply.status == 409 && late).then(|| match &edit.what {
                    DocEditWhat::Tab { key } => {
                        let theirs = data_of::<DocAnswer>(&reply.body).unwrap_or_default();
                        let tab = theirs.tabs.iter().find(|tab| &tab.key == key).cloned().unwrap_or_default();
                        DocConflict { body: tab.body, hash: tab.body_hash, by_name: tab.updated_by_name }
                    }
                    _ => {
                        let page = data_of::<DocPage>(&reply.body).unwrap_or_default();
                        DocConflict { body: page.body, hash: page.body_hash, by_name: page.updated_by_name }
                    }
                });
                Err((error, conflict))
            }
        };

        // Lo que se mandó, para saber si se siguió escribiendo entre tanto.
        let (sent_title, sent_body, session) = (edit.title.clone(), edit.body.clone(), edit.session);
        // Si el editor ya no es éste, la respuesta no es de nadie.
        if self.state().edit.as_ref().map(|edit| edit.session) != Some(session) {
            return;
        }
        match outcome {
            Ok(saved) => {
                // Lo que se leyera antes de este guardado ya es viejo.
                self.core.inner.doc_saves.fetch_add(1, Ordering::SeqCst);
                let mut created = None;
                self.update(|state| {
                    let Some(edit) = &mut state.edit else { return };
                    edit.saving = false;
                    let still_writing = edit.body != sent_body || (edit.has_title() && edit.title != sent_title);
                    match saved {
                        Saved::Tab(answer) => {
                            let key = match &edit.what {
                                DocEditWhat::Tab { key } => key.clone(),
                                _ => String::new(),
                            };
                            edit.base_hash = answer.tabs.iter().find(|tab| tab.key == key).and_then(|tab| tab.body_hash.clone());
                            edit.base_body = sent_body.clone();
                            // Guardar es lo que crea el documento: desde
                            // ahora tiene cabecera.
                            if answer.doc.is_some() {
                                state.info = answer.doc;
                            }
                            state.tabs = answer.tabs;
                            state.decisions = answer.decisions;
                            state.error = None;
                        }
                        Saved::Page(page) => {
                            edit.base_hash = page.body_hash.clone();
                            edit.base_body = sent_body.clone();
                            edit.base_title = sent_title.clone();
                            if let Some(view) = &mut state.page {
                                if view.page.id == page.id {
                                    view.page = page;
                                }
                            }
                        }
                        Saved::Created(page) => {
                            // Ya existe: lo que se siga escribiendo es
                            // editar esa página, no crear otra.
                            edit.what = DocEditWhat::Page { id: page.id.clone() };
                            edit.base_hash = page.body_hash.clone();
                            edit.base_body = sent_body.clone();
                            edit.base_title = sent_title.clone();
                            created = Some(page.id);
                        }
                    }
                    if !still_writing {
                        state.edit = None;
                    }
                });
                // El árbol lleva títulos: tras crear o renombrar, se vuelve
                // a pedir. Y la página nueva se abre.
                match created {
                    Some(id) => {
                        self.reload_pages().await;
                        self.show_page(&id).await;
                    }
                    None if matches!(edit.what, DocEditWhat::Page { .. }) => self.reload_pages().await,
                    None => {}
                }
            }
            Err((error, conflict)) => self.update(|state| {
                let Some(edit) = &mut state.edit else { return };
                edit.saving = false;
                match conflict {
                    Some(conflict) => edit.conflict = Some(conflict),
                    None => edit.error = Some(DocEditError::from(&error)),
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editing(what: DocEditWhat, title: &str, body: &str) -> DocEdit {
        DocEdit::begin(what, title, body, Some("h1".into()))
    }

    /// Hay algo sin guardar cuando el texto ya no es el que se leyó; el
    /// título sólo cuenta donde hay título.
    ///
    /// Roturas que atrapa: preguntar «¿descartar?» a quien abrió el editor
    /// y no tocó nada; no preguntar a quien sólo cambió el título de una
    /// página; dar por cambiada una pestaña por un título que no tiene.
    #[test]
    fn dirty_is_what_differs_from_what_was_read() {
        let mut tab = editing(DocEditWhat::Tab { key: "overview".into() }, "", "qué es");
        assert!(!tab.is_dirty());
        tab.title = "no cuenta".into();
        assert!(!tab.is_dirty(), "una pestaña no tiene título");
        tab.body = "qué es esto".into();
        assert!(tab.is_dirty());
        tab.body = "qué es".into();
        assert!(!tab.is_dirty(), "volver a lo que había no es un cambio");

        let mut page = editing(DocEditWhat::Page { id: "p1".into() }, "Arquitectura", "texto");
        page.title = "Arquitectura nueva".into();
        assert!(page.is_dirty());
    }

    /// «Guardar» no hace nada sin cambios, mientras viaja otro guardado, ni
    /// con un choque a la vista.
    ///
    /// Roturas que atrapa: dos guardados a la vez del mismo texto; un
    /// «Guardar» encendido bajo el aviso de choque, que mandaría lo mismo
    /// para chocar otra vez.
    #[test]
    fn saving_is_offered_only_when_it_would_do_something() {
        let mut edit = editing(DocEditWhat::Tab { key: "runbook".into() }, "", "a");
        assert!(!edit.can_save());
        edit.body = "b".into();
        assert!(edit.can_save());
        edit.saving = true;
        assert!(!edit.can_save());
        edit.saving = false;
        edit.conflict = Some(DocConflict::default());
        assert!(!edit.can_save());
    }

    /// Al lado se ve lo propio; lo que hay guardado, sólo tras un choque y
    /// si se pidió.
    ///
    /// Rotura que atrapa: enseñar como vista previa de lo que se escribe el
    /// texto de otro (o al revés), que es justo lo que hay que distinguir
    /// para fundir a mano.
    #[test]
    fn the_preview_is_mine_unless_theirs_was_asked_for() {
        let mut edit = editing(DocEditWhat::Tab { key: "runbook".into() }, "", "a");
        edit.body = "lo mío".into();
        edit.showing_theirs = true;
        assert_eq!(edit.preview(), "lo mío", "sin choque no hay «lo suyo» que enseñar");
        edit.conflict = Some(DocConflict { body: "lo suyo".into(), ..DocConflict::default() });
        assert_eq!(edit.preview(), "lo suyo");
        edit.showing_theirs = false;
        assert_eq!(edit.preview(), "lo mío");
    }

    /// Cada pregunta dice lo suyo, y la que pisa a otro no se confunde con
    /// la que tira lo propio.
    ///
    /// Roturas que atrapa: el botón de «guardar encima» bajo la pregunta de
    /// descartar (o al revés); un choque que no dice quién llegó antes
    /// sabiéndolo; un fallo sin frase.
    #[test]
    fn each_question_and_each_failure_says_its_own() {
        let mut edit = editing(DocEditWhat::Tab { key: "runbook".into() }, "", "a");
        assert_eq!(edit.question(Locale::Es), None);
        edit.asking = Some(DocAsk::Leave { to: DocLeave::Cancel });
        let leave = edit.question(Locale::Es).unwrap();
        assert_eq!((leave.confirm.as_str(), leave.cancel.as_str()), ("Descartar", "Seguir editando"));
        edit.asking = Some(DocAsk::Overwrite);
        let over = edit.question(Locale::Es).unwrap();
        assert_eq!(over.confirm, "Guardar el mío encima");
        assert_ne!(over.title, leave.title);
        edit.asking = Some(DocAsk::LoadTheirs);
        assert_eq!(edit.question(Locale::Es).unwrap().confirm, "Descartar el mío y recargar");

        assert_eq!(edit.conflict_phrase(Locale::Es), None);
        edit.conflict = Some(DocConflict { by_name: Some("Ana Pérez".into()), ..DocConflict::default() });
        assert!(edit.conflict_phrase(Locale::Es).unwrap().starts_with("Ana Pérez"));
        edit.conflict = Some(DocConflict { by_name: Some("  ".into()), ..DocConflict::default() });
        assert!(!edit.conflict_phrase(Locale::Es).unwrap().contains("  "));

        edit.error = Some(DocEditError { offline: false, code: "body-too-large".into(), message: "This text is too long".into() });
        assert!(edit.error_phrase(Locale::Es).unwrap().contains("demasiado largo"));
        edit.error = Some(DocEditError { offline: false, code: "nueva-etiqueta".into(), message: "x".into() });
        assert_eq!(edit.error_phrase(Locale::Es).unwrap(), "No se pudo guardar esta sección");
        edit.what = DocEditWhat::NewPage { parent_id: None };
        assert_eq!(edit.error_phrase(Locale::Es).unwrap(), "No se pudo crear la página");
    }

    /// Un fallo de red se dice como «sin conexión», no como un fallo del
    /// servidor; y junto a los botones se dice si se está guardando o si
    /// hay cambios sin guardar, y nada si no los hay.
    ///
    /// Roturas que atrapa: «No se pudo guardar» sin más cuando lo que pasa
    /// es que no hay red; «cambios sin guardar» en un editor recién
    /// abierto; no decir que se está guardando.
    #[test]
    fn a_draft_says_what_state_it_is_in() {
        let mut edit = editing(DocEditWhat::Tab { key: "runbook".into() }, "", "a");
        assert_eq!(edit.status(Locale::Es), None);
        edit.body = "b".into();
        assert_eq!(edit.status(Locale::Es).as_deref(), Some("Cambios sin guardar"));
        edit.saving = true;
        assert_eq!(edit.status(Locale::Es).as_deref(), Some("Guardando…"));

        edit.error = Some(DocEditError::from(&ApiError::Network("sin red".into())));
        assert_eq!(edit.error_phrase(Locale::Es), Some(text(Locale::Es, "nav:panel.serverStatus.offline")));
        let refused = ApiError::Server { status: 413, code: "body-too-large".into(), message: "x".into() };
        assert_eq!(DocEditError::from(&refused), DocEditError { offline: false, code: "body-too-large".into(), message: "x".into() });
    }

    /// No se ofrece escribir lo que no llegó: un documento que falló al
    /// cargar, uno que todavía viene, o una página que no se ha abierto.
    /// Uno que sólo se está poniendo al día sí.
    ///
    /// Rotura que atrapa: abrir el editor en blanco sobre un documento que
    /// no se pudo leer, y guardar ese blanco encima de lo que tenía.
    #[test]
    fn what_did_not_arrive_is_not_offered_for_writing() {
        use crate::docs::DocTarget;
        use crate::nav::NodeKind;
        let open = || DocsState { can_edit: true, target: Some(DocTarget { kind: NodeKind::Space, id: "a".into(), name: "A".into() }), ..DocsState::default() };
        assert!(open().editable(), "un nodo sin nada escrito se puede empezar a escribir");
        assert!(!DocsState { can_edit: true, ..DocsState::default() }.editable(), "sin documento abierto");
        assert!(!DocsState { error: Some("500".into()), ..open() }.editable());
        assert!(!DocsState { loading: true, ..open() }.editable());
        let tabs = vec![DocTab { key: "overview".into(), body: "qué es".into(), ..DocTab::default() }];
        assert!(DocsState { loading: true, tabs, ..open() }.editable());
        assert!(!DocsState { page_id: Some("p1".into()), ..open() }.editable());
    }

    /// Lado a lado sólo donde caben los dos.
    ///
    /// Rotura que atrapa: partir en dos una ventana estrecha, con la caja y
    /// la vista previa a media anchura cada una.
    #[test]
    fn side_by_side_needs_room() {
        assert!(!doc_editor_side_by_side(1199.));
        assert!(doc_editor_side_by_side(1200.));
    }
}
