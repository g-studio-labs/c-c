//! La documentación de un nodo, para leerla.
//!
//! Cuelga de un nodo del árbol (espacio, carpeta o lista) y tiene cuatro
//! pestañas fijas —la portada— más las páginas que cuelgan de ella (ver
//! `doc_pages`). Aquí se lee; escribirla es de `doc_edit`.
//!
//! Qué documento hay abierto es también lo que contesta «¿qué está mirando?»
//! a la regla del clic (`nav::tree_click`): con uno abierto se mira
//! documentación, y un clic en el árbol abre la de ese nodo.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::doc_edit::{DocEdit, DocEditWhat, DocLeave};
use crate::doc_header::DocInfo;
use crate::doc_links::{DocJump, DocLinkFailure};
use crate::doc_pages::{DocPageItem, DocPageView};
use crate::task::DecisionRef;

use crate::mywork::day_and_month;
use crate::nav::{docs_reachable_from_scope, Looking, NodeKind, TreeClick};
use crate::texts::Locale;
use crate::tree_rows::find_node;
use crate::Core;

/// Las cuatro pestañas, en el orden en que se pintan. Ninguna se esconde por
/// estar vacía: que un proyecto no tenga runbook es un dato sobre el proyecto.
pub const DOC_TABS: [&str; 4] = ["overview", "runbook", "decisions", "links"];

/// Una sección (un título) de una pestaña o de una página, con identidad
/// propia en el servidor: renombrarla o moverla no le cambia el id, así que
/// un enlace a ella, o una decisión que la sostiene, siguen llegando
/// (command-and-control-91). Espejo de `domain.DocSectionView`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocSection {
    pub id: String,
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub level: u8,
    /// Su puesto entre los títulos de la pestaña o la página.
    #[serde(default)]
    pub index: u32,
    /// Las decisiones vigentes que la sostienen: por qué dice lo que dice.
    #[serde(default, deserialize_with = "crate::doc_pages::or_empty", skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<DecisionRef>,
}

/// Una pestaña del documento. Espejo de `domain.DocTab`, con lo que se lee.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocTab {
    pub key: String,
    /// Markdown.
    #[serde(default)]
    pub body: String,
    /// De qué versión es lo que se lee: quien guarda lo devuelve
    /// (`baseHash`), y si el servidor ya tiene otra, no se pisa.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by_name: Option<String>,
    /// Sus títulos, con el id que les da el servidor.
    #[serde(default, deserialize_with = "crate::doc_pages::or_empty", skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<DocSection>,
}

/// Una entrada del registro de decisiones. Espejo de `domain.Decision`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocDecision {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    pub decided_at: String,
    /// El título de la tarea de la que salió.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_title: Option<String>,
    /// La decisión que dejó a ésta sin efecto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_id: Option<String>,
}

impl DocDecision {
    /// De dónde salió, dicho: «De <la tarea>», o que se escribió en el
    /// propio registro (`Procedencia` en `DecisionList.tsx`). El `bool` es si
    /// salió de una tarea, que lleva su icono.
    pub fn origin(&self, locale: Locale) -> (bool, String) {
        match self.origin_title.as_deref().filter(|title| !title.trim().is_empty()) {
            Some(title) => (true, crate::texts::text_with(locale, "work:decisions.fromTask", &[("what", title)])),
            None => (false, crate::texts::text(locale, "work:decisions.writtenHere")),
        }
    }

    /// Cuándo se decidió, dicho: «30 sept 2026». Es un instante, así que el
    /// día es el de quien lo lee.
    pub fn decided_on(&self, locale: Locale) -> String {
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(&self.decided_at) else { return String::new() };
        let day = at.with_timezone(&chrono::Local).date_naive();
        format!("{} {}", day_and_month(locale, day), chrono::Datelike::year(&day))
    }
}

/// El sobre con que el servidor contesta `GET /api/v1/docs/{kind}/{id}`. Un
/// nodo sin documento contesta igual, con las listas vacías.
#[derive(Debug, Default, Deserialize)]
struct DocAnswer {
    /// El documento en sí; `null` si en el nodo no hay nada escrito.
    #[serde(default)]
    doc: Option<DocInfo>,
    #[serde(default)]
    tabs: Vec<DocTab>,
    #[serde(default)]
    decisions: Vec<DocDecision>,
}

/// De qué nodo es el documento abierto.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocTarget {
    pub kind: NodeKind,
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocsState {
    /// `None` es «no se mira documentación».
    pub target: Option<DocTarget>,
    /// El documento en sí —quién responde de él, cuándo se revisó—, si ya
    /// existe (ver `doc_header`).
    pub info: Option<DocInfo>,
    /// La pestaña que se lee, por su clave (`DOC_TABS`).
    pub tab: String,
    pub loading: bool,
    pub error: Option<String>,
    pub tabs: Vec<DocTab>,
    pub decisions: Vec<DocDecision>,
    /// Las páginas del documento, en el orden del servidor (por rango).
    pub pages: Vec<DocPageItem>,
    /// Por qué no llegó el árbol de páginas, si no llegó.
    pub pages_error: Option<String>,
    /// Si la hoja «Páginas» está abierta.
    pub pages_shown: bool,
    /// La página que se lee; `None` es la portada.
    pub page_id: Option<String>,
    pub page: Option<DocPageView>,
    pub page_loading: bool,
    pub page_error: Option<String>,
    /// Las ramas del árbol de páginas plegadas o desplegadas a mano.
    pub page_folds: HashMap<String, bool>,
    /// Si quien mira puede escribir en este documento: ver `Docs::open`.
    pub can_edit: bool,
    /// Lo que se está escribiendo, si se escribe algo (ver `doc_edit`).
    pub edit: Option<DocEdit>,
    /// A qué parte de lo que se lee hay que saltar cuando esté pintado: lo
    /// pidió un enlace (ver `doc_links`).
    pub jump: Option<DocJump>,
    /// El último enlace que no se pudo abrir, hasta que se quite el aviso.
    pub link_failed: Option<DocLinkFailure>,
    /// Si se está en el módulo Documentación (su panel y su tabla), y no
    /// leyendo la documentación de un nodo desde Trabajo. Abrir o cerrar un
    /// documento no lo cambia: es de dónde se está, no de qué se lee.
    pub in_module: bool,
    /// Los nodos del árbol de Documentación plegados o desplegados a mano,
    /// por su clave (`doc_key`). Tampoco cambian al abrir otro documento,
    /// salvo su camino (`docs_panel::reveal`).
    pub node_folds: HashMap<String, bool>,
    /// Las páginas de otros documentos, pedidas al desplegar su nodo en el
    /// árbol, por la clave del nodo.
    pub peeked: HashMap<String, Vec<DocPageItem>>,
}

impl Default for DocsState {
    fn default() -> Self {
        Self {
            target: None,
            info: None,
            tab: DOC_TABS[0].to_string(),
            loading: false,
            error: None,
            tabs: Vec::new(),
            decisions: Vec::new(),
            pages: Vec::new(),
            pages_error: None,
            pages_shown: false,
            page_id: None,
            page: None,
            page_loading: false,
            page_error: None,
            page_folds: HashMap::new(),
            can_edit: false,
            edit: None,
            jump: None,
            link_failed: None,
            in_module: false,
            node_folds: HashMap::new(),
            peeked: HashMap::new(),
        }
    }
}

impl DocsState {
    /// Un estado sin documento que conserva lo que es del módulo y no de lo
    /// que se lee: dónde se está y cómo va el árbol.
    fn emptied(&mut self) -> DocsState {
        DocsState {
            in_module: self.in_module,
            node_folds: std::mem::take(&mut self.node_folds),
            peeked: std::mem::take(&mut self.peeked),
            ..DocsState::default()
        }
    }
}

/// Un trozo del texto de un documento, para pintarlo con su ancho.
///
/// La medida de lectura (68 letras) es para **la prosa**: puesta al cuerpo
/// entero, una tabla ancha «cabe» partiendo las palabras a mitad y su propio
/// deslizamiento no se activa nunca (ver `.prose-doc` en `app/src/index.css`).
/// Lo ancho —tablas, bloques de código, una imagen sola— usa el ancho del panel.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocBlock {
    pub wide: bool,
    /// Markdown.
    pub text: String,
}

/// Parte un markdown en prosa y en lo ancho, en su orden.
///
/// Un bloque de código es lo que hay entre sus vallas (` ``` ` o `~~~`); una
/// tabla, las líneas seguidas que empiezan por `|`. Lo que va sangrado dentro
/// de una lista se queda con su lista: sacarlo de ella la rompería. Y lo que
/// hay dentro de una valla es código, empiece por lo que empiece.
pub fn doc_blocks(markdown: &str) -> Vec<DocBlock> {
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Prose,
        Table,
        Code,
        Image,
    }
    let mut blocks: Vec<DocBlock> = Vec::new();
    let mut flush = |kind: Kind, lines: &mut Vec<&str>| {
        let text = lines.join("\n");
        lines.clear();
        if !text.trim().is_empty() {
            blocks.push(DocBlock { wide: kind != Kind::Prose, text: text.trim_matches('\n').to_string() });
        }
    };
    let (mut kind, mut lines) = (Kind::Prose, Vec::new());
    // La valla abierta: con qué letra y cuántas; se cierra con las mismas o más.
    let mut fence: Option<(char, usize)> = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some((mark, length)) = fence {
            lines.push(line);
            let closing = trimmed.chars().take_while(|c| *c == mark).count();
            if closing >= length && trimmed[closing..].trim().is_empty() {
                fence = None;
                flush(Kind::Code, &mut lines);
                kind = Kind::Prose;
            }
            continue;
        }
        let opening = ['`', '~'].into_iter().find(|mark| trimmed.starts_with(&mark.to_string().repeat(3)));
        let next = match opening {
            Some(_) if indent < 2 => Kind::Code,
            _ if indent < 2 && trimmed.starts_with('|') => Kind::Table,
            _ if indent < 2 && trimmed.starts_with("![") && trimmed.trim_end().ends_with(')') => Kind::Image,
            _ => Kind::Prose,
        };
        // Una tabla o un párrafo siguen mientras sigan sus líneas. Un bloque
        // de código ya va solo: al cerrarse su valla se suelta.
        if next != kind {
            flush(kind, &mut lines);
            kind = next;
        }
        lines.push(line);
        if let (Kind::Code, Some(mark)) = (next, opening) {
            fence = Some((mark, trimmed.chars().take_while(|c| *c == mark).count()));
        }
    }
    flush(kind, &mut lines);
    blocks
}

impl DocsState {
    /// Si esa pestaña no tiene nada: sale apagada, para saber dónde hay algo
    /// escrito sin abrirlas una a una. La de decisiones no se mide por su
    /// texto —no tiene—, sino por si hay entradas.
    pub fn tab_is_empty(&self, key: &str) -> bool {
        if key == "decisions" {
            self.decisions.is_empty()
        } else {
            self.tabs.iter().find(|tab| tab.key == key).is_none_or(|tab| tab.body.trim().is_empty())
        }
    }

    /// El texto de lo que se lee: el de la página abierta o, en la portada,
    /// el de su pestaña. Vacío si no tiene nada escrito, o si todavía no
    /// ha llegado.
    pub fn body(&self) -> &str {
        if self.on_page() {
            return self.page.as_ref().map(|view| view.page.body.trim()).unwrap_or("");
        }
        self.tabs.iter().find(|tab| tab.key == self.tab).map(|tab| tab.body.trim()).unwrap_or("")
    }

    /// Las decisiones que siguen vigentes, y sólo en su pestaña. Una que otra
    /// dejó sin efecto no se enseña: leerla como vigente es peor que no verla.
    pub fn standing_decisions(&self) -> Vec<DocDecision> {
        // El registro es de la portada: una página no lo repite.
        if self.on_page() || self.tab != "decisions" {
            return Vec::new();
        }
        self.decisions.iter().filter(|decision| decision.superseded_by_id.is_none()).cloned().collect()
    }

    /// Por qué no hay nada que leer, como clave del catálogo; `None` si lo
    /// hay. No es lo mismo «no ha llegado», «se rompió» y «no hay nada
    /// escrito», y una página en blanco no distingue ninguna.
    pub fn empty_reason(&self) -> Option<&'static str> {
        if self.on_page() {
            // Una página tiene sus propios motivos: no se pudo abrir, no ha
            // llegado, o está en blanco.
            return if self.page_error.is_some() {
                Some("work:docs.errPage")
            } else if self.page.is_none() {
                Some("work:tree.loading")
            } else if self.body().is_empty() {
                Some("work:docs.emptyPage")
            } else {
                None
            };
        }
        if self.loading && self.tabs.is_empty() {
            Some("work:tree.loading")
        } else if self.error.is_some() {
            Some("common:last.requestsFailing")
        } else if self.body().is_empty() && self.standing_decisions().is_empty() {
            Some(if self.tab == "decisions" { "work:decisions.empty" } else { "work:docs.emptyTab" })
        } else {
            None
        }
    }
}

pub(crate) fn kind_segment(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Space => "space",
        NodeKind::Folder => "folder",
        NodeKind::List => "list",
    }
}

/// La documentación de este core.
pub struct Docs {
    pub(crate) core: Core,
}

impl Core {
    pub fn docs(&self) -> Docs {
        Docs { core: self.clone() }
    }

    /// Qué se está mirando: documentación si hay un documento abierto.
    pub fn looking(&self) -> Looking {
        if self.docs().state().target.is_some() {
            Looking::Docs
        } else {
            Looking::Tasks
        }
    }

    /// Un clic en un nodo del árbol, entero: aplica la regla de `tree_click`
    /// con lo que se está mirando y, si toca, abre la documentación.
    pub async fn click_node(&self, id: &str) -> Option<TreeClick> {
        let click = self.tree().click(id, self.looking())?;
        // Acotar es pedir tareas: «Por atender» deja de estar delante.
        if matches!(click, TreeClick::ScopeTasks { .. }) {
            self.requests().leave();
            self.overview().leave();
            self.notes().leave();
        }
        if click == TreeClick::OpenDocs {
            self.docs().open(id).await;
        }
        Some(click)
    }

    /// El botón de documentación de «Mi trabajo»: abre la del nodo al que
    /// está acotado, sea lista, carpeta o espacio (command-and-control-372). Sin acotar
    /// no hay nodo del que colgar, y no hace nada.
    pub async fn open_scope_docs(&self) {
        let Some(scope) = self.my_work().state().scope else { return };
        if docs_reachable_from_scope(Some(scope.kind)) {
            self.docs().open(&scope.id).await;
        }
    }

    /// De la documentación a las tareas **del mismo nodo**: cambiar de vista
    /// no cambia de qué nodo se habla. También desde la de una carpeta, que
    /// desde el 17-nov-2026 acota como las demás (command-and-control-382):
    /// antes se volvía a lo que hubiera puesto, fuera de quien fuera.
    ///
    /// Con texto sin guardar no se va: se pregunta (`doc_edit`).
    pub fn back_to_tasks(&self) {
        let Some(target) = self.docs().state().target else { return };
        if self.docs().held_back(DocLeave::Tasks { layout: None }) {
            return;
        }
        self.docs().close_now();
        self.docs().leave_module();
        self.tree().click(&target.id, Looking::Tasks);
    }

    /// «Mi trabajo» entero: sin ámbito y sin documento abierto.
    pub fn show_all_work(&self) {
        if self.docs().held_back(DocLeave::AllWork) {
            return;
        }
        self.docs().close_now();
        self.docs().leave_module();
        self.requests().leave();
        self.overview().leave();
        self.notes().leave();
        self.my_work().set_scope(None);
    }
}

impl Docs {
    pub fn state(&self) -> DocsState {
        self.core.inner.docs.lock().expect("docs lock").clone()
    }

    pub(crate) fn update(&self, change: impl FnOnce(&mut DocsState)) {
        let state = {
            let mut state = self.core.inner.docs.lock().expect("docs lock");
            change(&mut state);
            state.clone()
        };
        self.core.inner.listener.docs_changed(state);
    }

    /// Abre la documentación de un nodo y la trae, con su árbol de páginas.
    /// Un id que no está en el árbol no abre nada.
    ///
    /// Otro nodo empieza en su portada: la página que se leía es del
    /// documento que se deja. El mismo nodo otra vez se queda donde estaba
    /// —la página abierta, las ramas plegadas, lo que se esté escribiendo— y
    /// sólo se pone al día.
    ///
    /// Con texto sin guardar no se cambia de nodo: se pregunta (`doc_edit`).
    pub async fn open(&self, id: &str) {
        self.open_at(id, None, None).await;
    }

    /// Lo mismo, llegando ya a esa pestaña o a esa página: es a donde lleva
    /// un enlace (`doc_links`). Así no se enseña «Resumen» un instante antes
    /// de lo que se pidió.
    pub(crate) async fn open_at(&self, id: &str, tab: Option<&str>, page: Option<&str>) {
        let Some((kind, name)) = find_node(&self.core.tree().state().spaces, id) else { return };
        let other_node = self.state().target.as_ref().map(|target| target.id.as_str()) != Some(id);
        if other_node && self.held_back(DocLeave::Node { id: id.to_string() }) {
            return;
        }
        let target = DocTarget { kind, id: id.to_string(), name };
        // Plataforma se pinta por delante de la documentación: un documento
        // abierto desde la campana estando en ella no se veía.
        self.core.platform().leave();
        // Quién puede escribir aquí: quien puede cambiar el árbol de la
        // organización que se mira (`can_edit_tree`). Un `viewer` lee.
        let can_edit = self.core.tree().can_edit();
        let spaces = self.core.tree().state().spaces;
        self.update(|state| {
            // El documento de otro nodo no se enseña mientras llega éste:
            // se leería bajo un nombre que no es el suyo.
            if state.target.as_ref() != Some(&target) {
                // El mismo nodo con otro nombre (se renombró) no se lleva
                // lo que se estaba escribiendo en él. De otro nodo no queda
                // nada que llevarse: lo de arriba ya preguntó.
                let edit = state.edit.take();
                // El aviso de un enlace que no se abrió no es del documento.
                let link_failed = state.link_failed.take();
                crate::docs_panel::reveal(state, &spaces, target.kind, &target.id);
                *state = DocsState { target: Some(target.clone()), edit, link_failed, ..state.emptied() };
            }
            if let Some(tab) = tab.filter(|tab| DOC_TABS.contains(tab)) {
                state.tab = tab.to_string();
            }
            if let Some(page) = page {
                // La de antes no se enseña mientras llega ésta.
                if state.page_id.as_deref() != Some(page) {
                    state.page = None;
                }
                state.page_id = Some(page.to_string());
                state.page_loading = true;
                state.page_error = None;
            }
            state.can_edit = can_edit;
            state.loading = true;
            state.error = None;
        });
        let path = format!("/api/v1/docs/{}/{id}", kind_segment(kind));
        let saves = self.core.inner.doc_saves.load(Ordering::SeqCst);
        let answer = self.core.get::<DocAnswer>(&path).await;
        // Si mientras tanto se abrió otro, o se cerró, esta respuesta ya no
        // es de lo que se mira.
        if self.state().target.as_ref().map(|target| target.id.as_str()) != Some(id) {
            return;
        }
        match answer {
            Ok(answer) => self.install(answer, saves),
            Err(error) => self.update(|state| {
                state.error = Some(error.to_string());
                state.loading = false;
            }),
        }
        self.reload_pages().await;
        // La página que se leía, al día también; si ya no está en el árbol,
        // lo de arriba ya volvió a la portada.
        if let Some(page_id) = self.state().page_id {
            if self.state().target.as_ref().map(|target| target.id.as_str()) == Some(id) {
                self.load_page(&page_id).await;
            }
        }
        self.peek_path(kind, id).await;
    }

    /// Pone en el estado un documento que acaba de llegar.
    ///
    /// **Lo que se está escribiendo no se toca.** Si la pestaña que se
    /// escribe ya no es la que se leyó, se dice con el aviso de choque
    /// (`DocEdit::changed_underneath`) y el texto se queda donde estaba.
    ///
    /// `saves` es cuántos guardados iban cuando se pidió: si desde entonces
    /// salió otro, esta lectura trae lo de antes de él y no se pone.
    fn install(&self, answer: DocAnswer, saves: u64) {
        if self.core.inner.doc_saves.load(Ordering::SeqCst) != saves {
            self.update(|state| state.loading = false);
            return;
        }
        *self.core.inner.doc_read_at.lock().expect("doc read lock") = Some(Instant::now());
        self.update(|state| {
            if let Some(edit) = &mut state.edit {
                if let DocEditWhat::Tab { key } = &edit.what {
                    let theirs = answer.tabs.iter().find(|tab| &tab.key == key).cloned().unwrap_or_default();
                    edit.changed_underneath(&theirs.body, theirs.body_hash, theirs.updated_by_name);
                }
            }
            state.info = answer.doc;
            state.tabs = answer.tabs;
            state.decisions = answer.decisions;
            state.loading = false;
            state.error = None;
        });
    }

    /// Vuelve a pedir el documento abierto **sin** vaciarlo ni moverse de
    /// donde se lee: su texto, su cabecera, su árbol de páginas y la página
    /// abierta. Es lo que lo mantiene al día cuando lo cambia otro —una
    /// persona, o un agente por MCP—.
    ///
    /// Un fallo deja lo que había: sigue siendo lo último que se supo. Y lo
    /// que se está escribiendo no se toca nunca (ver `install`).
    ///
    /// El servidor no avisa de que un documento cambió (no hay evento para
    /// ello, 18-nov-2026), así que esto se llama al recuperar el stream y
    /// al volver a la ventana (`came_back`).
    pub async fn refresh(&self) {
        let Some(target) = self.state().target else { return };
        let path = format!("/api/v1/docs/{}/{}", kind_segment(target.kind), target.id);
        let saves = self.core.inner.doc_saves.load(Ordering::SeqCst);
        let Ok(answer) = self.core.get::<DocAnswer>(&path).await else { return };
        if self.state().target.as_ref().map(|now| now.id.as_str()) != Some(target.id.as_str()) {
            return;
        }
        self.install(answer, saves);
        self.reload_pages().await;
        if let Some(page_id) = self.state().page_id {
            if self.state().target.as_ref().map(|now| now.id.as_str()) == Some(target.id.as_str()) {
                self.load_page(&page_id).await;
            }
        }
    }

    /// Se volvió a la ventana: el documento abierto se pone al día, salvo
    /// que se acabe de leer (`Config::doc_fresh_for`). Mientras se estaba en
    /// otra ventana pudo cambiarlo cualquiera, y nadie lo avisa.
    pub async fn came_back(&self) {
        let read_at = *self.core.inner.doc_read_at.lock().expect("doc read lock");
        if read_at.is_some_and(|at| at.elapsed() < self.core.inner.config.doc_fresh_for) {
            return;
        }
        self.refresh().await;
    }

    /// Cierra la documentación. Con texto sin guardar, pregunta antes.
    pub fn close(&self) {
        if !self.held_back(DocLeave::Close) {
            self.close_now();
        }
    }

    /// La cierra sin preguntar: para cuando ya se preguntó, o cuando el
    /// nodo del que era ya no existe.
    pub(crate) fn close_now(&self) {
        if self.state().target.is_some() {
            self.update(|state| *state = state.emptied());
        }
    }

    /// Cambia de pestaña. Una clave que no es de las cuatro no hace nada, y
    /// la que ya se lee tampoco. Con texto sin guardar, pregunta antes.
    pub fn show_tab(&self, key: &str) {
        if !DOC_TABS.contains(&key) || self.state().tab == key {
            return;
        }
        if self.held_back(DocLeave::Tab { key: key.to_string() }) {
            return;
        }
        self.update(|state| {
            state.tab = key.to_string();
            // A donde se iba a saltar era de la otra pestaña.
            state.jump = None;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mientras llega no se dice «vacío», y al recargar lo que ya se leía no
    /// se cambia por un «cargando».
    ///
    /// Roturas que atrapa: «todavía no hay nada aquí» durante la carga de un
    /// documento que sí tiene; un parpadeo a «cargando» en cada recarga.
    #[test]
    fn loading_is_said_only_while_there_is_nothing_to_read() {
        let arriving = DocsState { loading: true, ..DocsState::default() };
        assert_eq!(arriving.empty_reason(), Some("work:tree.loading"));

        let reloading = DocsState {
            loading: true,
            tabs: vec![DocTab { key: "overview".into(), body: "qué es".into(), ..DocTab::default() }],
            ..DocsState::default()
        };
        assert_eq!(reloading.empty_reason(), None);
    }

    /// Rotura que atrapa: enseñar el instante crudo, o una fecha sin año en
    /// un registro que se lee años después.
    #[test]
    fn a_decision_says_the_day_it_was_made() {
        // A mediodía de Greenwich es el mismo día en casi cualquier zona.
        let decision = DocDecision { decided_at: "2026-09-30T12:00:00Z".into(), ..DocDecision::default() };
        assert_eq!(decision.decided_on(Locale::Es), "30 sept 2026");
        assert_eq!(decision.decided_on(Locale::En), "Sep 30 2026");
        assert_eq!(DocDecision { decided_at: "ayer".into(), ..DocDecision::default() }.decided_on(Locale::Es), "");
    }

    fn shape(markdown: &str) -> Vec<(bool, String)> {
        doc_blocks(markdown).into_iter().map(|block| (block.wide, block.text)).collect()
    }

    /// La prosa y lo ancho salen separados y en su orden, cada tabla y cada
    /// bloque de código enteros.
    ///
    /// Roturas que atrapa: una tabla dentro de la medida de lectura (se
    /// estruja y parte las palabras); un párrafo a todo el ancho del panel;
    /// un bloque de código partido en dos; perder o desordenar texto.
    #[test]
    fn prose_and_what_is_wide_come_apart_in_order() {
        let text = "# Título\n\nUn párrafo.\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nOtro párrafo.\n\n```sh\ncargo test\n\n# no es un título\n```\nY el final.";
        assert_eq!(
            shape(text),
            [
                (false, "# Título\n\nUn párrafo.".to_string()),
                (true, "| a | b |\n| - | - |\n| 1 | 2 |".to_string()),
                (false, "Otro párrafo.".to_string()),
                (true, "```sh\ncargo test\n\n# no es un título\n```".to_string()),
                (false, "Y el final.".to_string()),
            ]
        );
        assert_eq!(shape(""), []);
        assert_eq!(shape("\n  \n"), []);
    }

    /// Lo que hay dentro de una valla es código, empiece por lo que empiece,
    /// y la valla la cierra otra igual o más larga, no una más corta ni de
    /// otra letra.
    ///
    /// Roturas que atrapa: una línea con `|` dentro de un bloque de código
    /// tomada por una tabla (parte el bloque en tres); cerrar un bloque de
    /// cuatro comillas con tres, o uno de `~~~` con comillas.
    #[test]
    fn what_is_inside_a_fence_is_code() {
        let piped = "```\n| no | tabla |\n```";
        assert_eq!(shape(piped), [(true, piped.to_string())]);

        let nested = "````md\n```\ndentro\n```\n````\nfuera";
        assert_eq!(shape(nested), [(true, "````md\n```\ndentro\n```\n````".to_string()), (false, "fuera".to_string())]);

        let tildes = "~~~\n```\n~~~\nfuera";
        assert_eq!(shape(tildes), [(true, "~~~\n```\n~~~".to_string()), (false, "fuera".to_string())]);

        // Una valla sin cerrar se lleva lo que queda, como en markdown.
        assert_eq!(shape("antes\n```\ncódigo"), [(false, "antes".to_string()), (true, "```\ncódigo".to_string())]);
    }

    /// Dos bloques de código seguidos son dos; una imagen sola en su línea
    /// usa el ancho del panel; y lo sangrado dentro de una lista se queda
    /// con su lista.
    ///
    /// Roturas que atrapa: fundir dos bloques en uno (el segundo se pinta
    /// como texto del primero); sacar de su lista el código de un paso, que
    /// la corta en dos listas que vuelven a numerar desde uno.
    #[test]
    fn each_wide_thing_stands_alone_and_a_list_keeps_what_is_its_own() {
        let two = "```\na\n```\n```\nb\n```";
        assert_eq!(shape(two), [(true, "```\na\n```".to_string()), (true, "```\nb\n```".to_string())]);

        let picture = "Mira:\n![captura](https://example.com/a.png)\nY sigue.";
        assert_eq!(
            shape(picture),
            [(false, "Mira:".to_string()), (true, "![captura](https://example.com/a.png)".to_string()), (false, "Y sigue.".to_string())]
        );

        let list = "1. Paso uno:\n   ```sh\n   make\n   ```\n2. Paso dos\n   | a | b |";
        assert_eq!(shape(list), [(false, list.to_string())]);
    }

    /// Una pestaña sin nada escrito sale apagada; la de decisiones se mide
    /// por sus entradas, no por su texto.
    ///
    /// Roturas que atrapa: todas las pestañas iguales, haya o no algo; la
    /// de decisiones siempre apagada aunque el registro esté lleno.
    #[test]
    fn a_tab_with_nothing_written_is_empty() {
        let docs = DocsState {
            tabs: vec![
                DocTab { key: "overview".into(), body: "qué es".into(), ..DocTab::default() },
                DocTab { key: "runbook".into(), body: "  \n".into(), ..DocTab::default() },
            ],
            decisions: vec![DocDecision::default()],
            ..DocsState::default()
        };
        assert!(!docs.tab_is_empty("overview"));
        assert!(docs.tab_is_empty("runbook"));
        assert!(docs.tab_is_empty("links"), "una pestaña que el documento no trae no está escrita");
        assert!(!docs.tab_is_empty("decisions"));
        assert!(DocsState::default().tab_is_empty("decisions"));
    }

    /// Una decisión dice de qué tarea salió, o que se escribió en el registro.
    ///
    /// Roturas que atrapa: «De » sin nombre cuando no salió de ninguna
    /// tarea; decir «escrita aquí» de la que salió de una.
    #[test]
    fn a_decision_says_where_it_came_from() {
        let from_task = DocDecision { origin_title: Some("Subir el backend".into()), ..DocDecision::default() };
        assert_eq!(from_task.origin(Locale::Es), (true, "De Subir el backend".to_string()));
        assert_eq!(DocDecision::default().origin(Locale::Es), (false, "Escrita aquí".to_string()));
        let blank = DocDecision { origin_title: Some("  ".into()), ..DocDecision::default() };
        assert_eq!(blank.origin(Locale::Es), (false, "Escrita aquí".to_string()));
    }
}
