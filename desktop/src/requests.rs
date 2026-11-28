//! «Por atender»: lo que los agentes esperan de quien mira, con sus botones
//! (`AgentRequests.tsx` y `RequestCard.tsx`, command-and-control-504).
//!
//! Qué se manda por cada botón, cómo quedó uno cerrado y qué falta para
//! poder mandarlo lo decide el core (`cac_core::build_answer`,
//! `request_outcome`); aquí se pinta y se pulsa. Lo propio de esta pantalla
//! es lo que se está escribiendo en cada tarjeta y qué modo tiene abierto
//! (editar, rechazar, «no funciona»).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use cac_core::{AgentRequest, AnswerDraft, InboxTarget, LinkSegment, RequestFile, RequestFileSlot, RequestFilter, RequestKind, RequestScope, RequestStatus, RequestStatusFilter};
use cac_mcp::host::{NotOpened, Opened};
use cac_mcp::text_file::Refusal;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{error_text, locale, tr};
use crate::theme::Leading as _;

/// Cuánto se espera tras la última tecla antes de buscar.
const SEARCH_AFTER: std::time::Duration = std::time::Duration::from_millis(300);
/// A cuántos píxeles del final visible se pide ya la página siguiente.
const LOAD_BEFORE: f32 = 240.;

/// El texto de un pedido es texto, no markdown: lo que parezca marcado
/// (`*`, `_`, `#`, una lista) se pinta tal cual, y un salto de línea es un
/// salto.
fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("\\\n"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '~' | '|' | '&' | '!' | '-' | '+' | '.' | '(' | ')' | '{' | '}' | '=' | '"' | '$' | '%' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Lo abierto en una tarjeta, además de sus botones.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Idle,
    /// «Editar y aprobar»: el borrador en un campo.
    Edit,
    /// «Rechazar»: el motivo, obligatorio.
    Reject,
    /// «No funciona» se pulsó sin nota: ahora la nota es obligatoria.
    Fails,
}

/// Lo que se sabe de una ruta de esta máquina que nombra un pedido: si se
/// puede copiar su contenido (command-and-control-567). Se mira una vez, fuera
/// del hilo de la ventana, con las reglas de adjuntar (`cac_mcp::text_file`).
#[derive(Clone, PartialEq, Eq)]
enum LocalFile {
    Checking,
    Copyable(String),
    Refused(Refusal),
}

/// Un fichero adjunto con más líneas se enseña plegado.
const FOLDED_LINES: usize = 12;

/// Qué se pulsó en un adjunto cuyo contenido hubo que pedir.
enum FileWanted {
    Copy,
    /// Desplegarlo: el id del bloque en pantalla.
    Show(String),
}

/// La clave del contenido de un adjunto: el pedido, su versión y el índice.
fn file_key(code: &str, version: i64, n: usize) -> String {
    format!("{code}#{version}#{n}")
}

/// Lo escrito en una tarjeta. Vive mientras el pedido siga abierto: la lista
/// se vuelve a pedir con cada evento, y lo escrito no se pierde por eso.
struct Card {
    mode: Mode,
    text: Entity<TextareaState>,
    note: Entity<TextareaState>,
    reason: Entity<TextareaState>,
    output: Entity<TextareaState>,
}

pub struct RequestsView {
    cards: HashMap<String, Card>,
    busy: HashSet<String>,
    /// Por qué no se pudo contestar, por pedido.
    problems: HashMap<String, String>,
    /// El último ID copiado: su botón dice que ya está.
    copied: Option<String>,
    /// Las rutas que nombran los pedidos, por ruta tal como se escribió.
    local_files: HashMap<String, LocalFile>,
    /// Los ficheros adjuntos desplegados (por su id en pantalla).
    unfolded: HashSet<String>,
    /// El contenido de los adjuntos, que la lista no trae: se pide al detalle
    /// la primera vez que hace falta, por `file_key` (rehacer el pedido sube
    /// la versión y lo vuelve a pedir).
    file_contents: HashMap<String, String>,
    /// Lo que se dice tras pulsar algo que esta app todavía no tiene.
    said: Option<String>,
    /// La búsqueda: se manda un rato después de dejar de escribir, no con
    /// cada tecla (`search_turn` dice cuál es la última).
    search: Entity<InputState>,
    search_turn: u64,
    scroll: ScrollHandle,
    /// El pedido resaltado al que todavía no se ha llevado la vista.
    scroll_to: Option<String>,
    /// El resaltado ya atendido: se atiende una sola vez, la primera vez que su
    /// pedido aparece en la lista.
    shown_highlight: Option<String>,
    /// Las tarjetas en el orden en que quedaron pintadas, para saber si la
    /// lista cambió.
    order: Vec<String>,
    /// Dónde quedó pintada cada tarjeta en el último cuadro, que cada una
    /// anota al pintarse (el `ScrollHandle` de esta versión no da las de sus
    /// hijos).
    tops: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    /// La primera tarjeta que se veía y a cuántos píxeles del borde de arriba,
    /// anotada cuando la lista cambió. La corrige esa misma tarjeta al pintarse
    /// en su sitio nuevo.
    anchor: Rc<RefCell<Option<(String, Pixels)>>>,
    _subscriptions: Vec<Subscription>,
}

impl RequestsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr(cx, "work:requests.searchPlaceholder")));
        let typing = cx.subscribe(&search, |view: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.search_soon(cx);
            }
        });
        Self {
            cards: HashMap::new(),
            busy: HashSet::new(),
            problems: HashMap::new(),
            copied: None,
            local_files: HashMap::new(),
            unfolded: HashSet::new(),
            file_contents: HashMap::new(),
            said: None,
            search,
            search_turn: 0,
            scroll: ScrollHandle::new(),
            scroll_to: None,
            shown_highlight: None,
            order: Vec::new(),
            tops: Rc::default(),
            anchor: Rc::default(),
            _subscriptions: vec![typing],
        }
    }

    /// Lo escrito en la búsqueda va al servidor cuando se deja de escribir.
    fn search_soon(&mut self, cx: &mut Context<Self>) {
        self.search_turn += 1;
        let turn = self.search_turn;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_AFTER).await;
            this.update(cx, |view, cx| {
                if view.search_turn != turn {
                    return;
                }
                let text = view.search.read(cx).value().to_string();
                let filter = RequestFilter { text, ..Backend::snapshot(cx).read(cx).requests.filter.clone() };
                Self::filter(filter, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Cambiar un filtro vuelve a la primera página de lo filtrado.
    fn filter(filter: RequestFilter, cx: &mut Context<Self>) {
        Backend::run(cx, move |core| async move { core.requests().set_filter(filter).await });
    }

    fn render_filters(&self, filter: &RequestFilter, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let kinds: Vec<AnyElement> = RequestKind::ALL
            .iter()
            .filter_map(|&kind| {
                let wire = kind.wire()?;
                let on = filter.kinds.contains(&kind);
                let current = filter.clone();
                let button = Button::new(SharedString::from(format!("requests-kind-{wire}"))).small().label(kind.label(locale(cx))).on_click(cx.listener(move |_, _, _, cx| {
                    let mut next = current.clone();
                    if on {
                        next.kinds.retain(|picked| *picked != kind);
                    } else {
                        next.kinds.push(kind);
                    }
                    Self::filter(next, cx);
                }));
                Some(if on { button.secondary() } else { button.ghost() }.into_any_element())
            })
            .collect();
        let statuses: Vec<AnyElement> = [
            (RequestStatusFilter::All, "all"),
            (RequestStatusFilter::Open, "open"),
            (RequestStatusFilter::Resolved, "resolved"),
            (RequestStatusFilter::Withdrawn, "withdrawn"),
        ]
        .into_iter()
        .map(|(status, key)| {
            let current = filter.clone();
            let button = Button::new(SharedString::from(format!("requests-status-{key}")))
                .small()
                .label(tr(cx, &format!("work:requests.statusFilter.{key}")))
                .on_click(cx.listener(move |_, _, _, cx| Self::filter(RequestFilter { status, ..current.clone() }, cx)));
            if filter.status == status { button.secondary() } else { button.ghost() }.into_any_element()
        })
        .collect();
        let group = |id: &'static str, label: String, chips: Vec<AnyElement>| {
            div()
                .id(id)
                .flex()
                .flex_wrap()
                .items_center()
                .gap_0p5()
                .p_0p5()
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .aria_label(label)
                .children(chips)
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Input::new(&self.search).id("requests-search"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(group("requests-kinds", tr(cx, "work:requests.filterKind"), kinds))
                    .child(group("requests-statuses", tr(cx, "work:requests.filterStatus"), statuses)),
            )
    }

    /// El final de una lista: dice si está cargando o por qué falló, y
    /// cuando entra en vista (o queda cerca) pide la página siguiente. Lo
    /// pinta cada cuadro, también al deslizar; que no salgan dos peticiones
    /// lo cuida el core.
    fn render_end(&self, id: &'static str, paging: cac_core::RequestPaging, active: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let wants = active && paging.more && !paging.loading && !paging.failed;
        let scroll = self.scroll.clone();
        div()
            .id(id)
            .relative()
            .min_h(px(1.))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let area = scroll.bounds();
                        if wants && area.size.height > px(0.) && bounds.top() <= area.bottom() + px(LOAD_BEFORE) {
                            Backend::run(cx, |core| async move { core.requests().load_more().await });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .when(active && paging.loading, |this| {
                this.child(div().id(SharedString::from(format!("{id}-loading"))).leading_sm().text_color(muted).child(tr(cx, "work:requests.loadingMore")).test_support())
            })
            .when(active && paging.failed, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(div().id(SharedString::from(format!("{id}-error"))).leading_sm().text_color(danger).child(tr(cx, "work:requests.errLoadMore")).test_support())
                        .child(Button::new(SharedString::from(format!("{id}-retry"))).small().outline().label(tr(cx, "work:requests.retry")).on_click(cx.listener(|_, _, _, cx| {
                            Backend::run(cx, |core| async move { core.requests().load_more().await });
                        }))),
                )
            })
            .test_support()
            .into_any_element()
    }

    fn card(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.cards.contains_key(code) {
            return;
        }
        let field = |placeholder: String, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| TextareaState::new(window, cx).placeholder(placeholder).auto_grow(2, 10));
        let text = field(tr(cx, "work:requests.finalText"), window, cx);
        let note = field(tr(cx, "work:requests.noteOptional"), window, cx);
        let reason = field(tr(cx, "work:requests.rejectReason"), window, cx);
        let output = field(tr(cx, "work:requests.output"), window, cx);
        for input in [&text, &note, &reason, &output] {
            // Los botones que piden texto se encienden al escribirlo.
            self._subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        self.cards.insert(code.to_string(), Card { mode: Mode::Idle, text, note, reason, output });
    }

    fn value(field: &Entity<TextareaState>, cx: &App) -> String {
        field.read(cx).value().to_string()
    }

    fn set_mode(&mut self, code: &str, mode: Mode, cx: &mut Context<Self>) {
        if let Some(card) = self.cards.get_mut(code) {
            card.mode = mode;
        }
        self.problems.remove(code);
        cx.notify();
    }

    /// «Editar y aprobar»: el campo empieza con el borrador.
    fn start_edit(&mut self, request: &AgentRequest, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.cards.get(&request.code) else { return };
        let draft = request.body.draft.clone();
        card.text.update(cx, |field, cx| {
            field.set_value(draft, window, cx);
            field.focus(window, cx);
        });
        self.set_mode(&request.code, Mode::Edit, cx);
    }

    fn send(&mut self, code: String, draft: AnswerDraft, cx: &mut Context<Self>) {
        // Lo que no se puede mandar ni se intenta: el botón ya está apagado,
        // esto es por si llega un clic igual.
        if cac_core::build_answer(&draft).is_none() || !self.busy.insert(code.clone()) {
            return;
        }
        self.problems.remove(&code);
        cx.notify();
        let asked = code.clone();
        let answer = Backend::ask(cx, move |core| async move { core.requests().answer(&asked, draft).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy.remove(&code);
                match result {
                    Ok(_) => {
                        this.cards.remove(&code);
                    }
                    Err(error) => {
                        let said = error_text(cx, &error, "work:requests.errAnswer");
                        this.problems.insert(code, said);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Un enlace del pedido: una ruta de la app, a su sitio aquí en la
    /// organización del pedido; lo demás (una dirección de cac o de fuera,
    /// una ruta que esta app no tiene) lo decide `links::open`.
    fn open_link(&mut self, url: String, org_id: String, cx: &mut Context<Self>) {
        if !url.starts_with('/') || url.starts_with("//") || cac_core::link_target(&url) == InboxTarget::NotHere {
            crate::links::open(&url, cx);
            return;
        }
        Backend::run(cx, move |core| async move {
            core.inbox().open(&url, &org_id).await;
        });
    }

    /// Lo que llega no mueve lo que se lee: si la lista cambia (un pedido
    /// nuevo entra por encima, se contesta uno, se abre «Resueltos»), la primera
    /// tarjeta que se veía se queda a la misma altura. Se anota antes de que se
    /// pinte la lista nueva y esa tarjeta, al pintarse, corrige lo que se
    /// desliza (`AgentRequests.tsx` lo hace con el anclaje del navegador).
    fn keep_reading(&mut self, shown: Vec<String>) {
        let changed = self.order != shown;
        let area = self.scroll.bounds();
        let mut anchor = self.anchor.borrow_mut();
        // Si la tarjeta anotada se fue (se contestó), no hay a quién corregir.
        if anchor.as_ref().is_some_and(|(code, _)| !shown.contains(code)) {
            *anchor = None;
        }
        if self.scroll_to.is_some() {
            *anchor = None;
        } else if changed && anchor.is_none() && area.size.height > px(0.) {
            let tops = self.tops.borrow();
            *anchor = self.order.iter().find_map(|code| {
                let card = tops.get(code)?;
                (card.bottom() > area.top()).then(|| (code.clone(), card.top() - area.top()))
            });
        }
        drop(anchor);
        self.order = shown;
    }

    /// Un enlace dentro de un texto del pedido: la web, al navegador; una
    /// pantalla de la app, dentro; una ruta, con el sistema, y si no existe se
    /// dice.
    fn follow(&mut self, target: LinkSegment, org_id: String, cx: &mut Context<Self>) {
        match target {
            LinkSegment::Url(url) => crate::links::open(&url, cx),
            LinkSegment::AppRoute { route, .. } => self.open_link(route, org_id, cx),
            LinkSegment::Path { path, .. } => {
                // Mirar si existe puede tardar (una ruta de red): fuera del hilo de la ventana.
                let shown = path.clone();
                cx.spawn(async move |this, cx| {
                    let result = cx.background_executor().spawn(async move { open_path(&path) }).await;
                    this.update(cx, |this, cx| {
                        let name = file_name(&shown);
                        match result {
                            Ok(Opened::WithItsProgram | Opened::Revealed(None)) => return,
                            Ok(Opened::Revealed(Some(NotOpened::ContentType(mime)))) => {
                                this.said = Some(cac_core::text_with(locale(cx), "work:requests.revealedByContent", &[("name", &name), ("type", &mime)]))
                            }
                            Ok(Opened::Revealed(Some(NotOpened::TypeUnknown))) => {
                                this.said = Some(cac_core::text_with(locale(cx), "work:requests.revealedTypeUnknown", &[("name", &name)]))
                            }
                            Err(cac_mcp::host::RevealError::Failed(_)) => this.said = Some(tr(cx, "common:last.openLinkFailed")),
                            Err(_) => this.said = Some(cac_core::text_with(locale(cx), "work:requests.pathMissing", &[("path", &shown)])),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            LinkSegment::Text(_) => {}
        }
    }

    /// Texto corriente que se puede seleccionar con el ratón y copiar (cursor
    /// de texto, doble clic para una palabra, Ctrl/⌘+C).
    fn selectable(id: String, text: String) -> AnyElement {
        div().id(SharedString::from(id.clone())).cursor_text().child(gpui_kit::base::SelectableText::new(SharedString::from(format!("{id}-text")), text)).test_support().into_any_element()
    }

    /// Un texto del pedido, seleccionable, con sus enlaces pintados y
    /// pulsables. Sin enlaces, texto corriente; con ellos, markdown pintado
    /// por `TextView` (que sí deja seleccionar) donde cada enlace apunta a
    /// `cac-link:N`, el índice del segmento que se sigue al pulsarlo.
    fn rich(&self, id: String, text: &str, org_id: &str, cx: &mut Context<Self>) -> AnyElement {
        // El markdown se come la sangría del principio (y con cuatro espacios la
        // pinta como bloque de código): se saca y se devuelve como margen.
        let body = text.trim_start();
        let indent: usize = text[..text.len() - body.len()].chars().map(|ch| if ch == '\t' { 4 } else { 1 }).sum();
        if indent > 0 && !body.is_empty() {
            let inner = self.rich(id, body, org_id, cx);
            return div().pl(px(indent as f32 * 7.)).child(inner).into_any_element();
        }
        let segments = cac_core::link_segments(text);
        if segments.iter().all(|segment| matches!(segment, LinkSegment::Text(_))) {
            return Self::selectable(id, text.to_string()).into_any_element();
        }
        let (mut painted, mut targets) = (String::new(), Vec::new());
        for segment in segments {
            match &segment {
                LinkSegment::Text(text) => painted.push_str(&escape_markdown(text)),
                LinkSegment::Url(shown) | LinkSegment::Path { shown, .. } | LinkSegment::AppRoute { shown, .. } => {
                    // Dentro del texto de un enlace el pintor no deshace las barras: sólo se
                    // protegen los corchetes.
                    painted.push_str(&format!("[{}](cac-link:{})", shown.replace('[', "(").replace(']', ")"), targets.len()));
                    targets.push(segment.clone());
                }
            }
        }
        let paths: Vec<bool> = targets.iter().map(|target| matches!(target, LinkSegment::Path { .. })).collect();
        let (view, org_id) = (cx.entity().downgrade(), org_id.to_string());
        let hint = tr(cx, "work:requests.openPath");
        let at = |url: &str| url.strip_prefix("cac-link:").and_then(|n| n.parse::<usize>().ok());
        let follow = move |url: &SharedString, _: &ClickEvent, _: &mut Window, cx: &mut App| {
            if let Some(target) = at(url).and_then(|n| targets.get(n)).cloned() {
                let org_id = org_id.clone();
                view.update(cx, |this, cx| this.follow(target, org_id, cx)).ok();
            }
        };
        let view_text = gpui_kit::component::text::TextView::markdown(SharedString::from(format!("{id}-text")), painted)
            .style(crate::theme::md_body_style(cx))
            .on_link_click(follow)
            .link_tooltip(move |url, window, cx| {
                if at(url).and_then(|n| paths.get(n).copied()).unwrap_or(false) {
                    gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx)
                } else {
                    gpui_kit::component::tooltip::Tooltip::new(url.clone()).build(window, cx)
                }
            });
        div().id(SharedString::from(id)).min_w_0().max_w_full().leading_sm().child(view_text).test_support().into_any_element()
    }

    fn copy(&mut self, text: String, code: Option<String>, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = code;
        cx.notify();
    }

    /// Mira una vez si una ruta se puede copiar; mientras, no se ofrece nada.
    fn check_local_file(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.local_files.contains_key(path) {
            return;
        }
        self.local_files.insert(path.to_string(), LocalFile::Checking);
        let asked = path.to_string();
        cx.spawn(async move |this, cx| {
            let looked = asked.clone();
            let result = cx.background_executor().spawn(async move { cac_mcp::text_file::check(&looked) }).await;
            this.update(cx, |this, cx| {
                let state = match result {
                    Ok(_) => LocalFile::Copyable(file_name(&asked)),
                    Err(why) => LocalFile::Refused(why),
                };
                this.local_files.insert(asked, state);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// «Copiar apply.sql» de una ruta: se lee al pulsar (pudo cambiar desde
    /// que se miró), y si ya no se puede, se dice por qué.
    fn copy_local_file(&mut self, path: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let read = path.clone();
            let result = cx.background_executor().spawn(async move { cac_mcp::text_file::read(&read) }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(file) => {
                        this.copy(file.content, None, cx);
                        this.said = Some(cac_core::text_with(locale(cx), "work:requests.fileCopied", &[("name", &file.name)]));
                    }
                    Err(why) => {
                        this.said = Some(refusal_line(&file_name(&path), &why, cx));
                        this.local_files.insert(path, LocalFile::Refused(why));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Debajo de un texto que nombra ficheros de esta máquina: por cada uno,
    /// «Copiar <nombre>» a la vista (jose: de un botonazo, sin pedírselo al
    /// agente) y «Mostrar en la carpeta» aparte; o por qué no se puede copiar.
    /// Una carpeta no lleva nada: pulsarla ya la abre.
    fn path_actions(&mut self, id: &str, texts: &[&str], cx: &mut Context<Self>) -> Option<AnyElement> {
        let mut paths: Vec<String> = Vec::new();
        for text in texts {
            for segment in cac_core::link_segments(text) {
                if let LinkSegment::Path { path, .. } = segment {
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
        }
        let mut rows: Vec<AnyElement> = Vec::new();
        for (n, path) in paths.into_iter().enumerate() {
            self.check_local_file(&path, cx);
            let state = self.local_files.get(&path).cloned().unwrap_or(LocalFile::Checking);
            let copy: AnyElement = match state {
                LocalFile::Checking | LocalFile::Refused(Refusal::NotAFile) => continue,
                LocalFile::Copyable(name) => {
                    let target = path.clone();
                    Button::new(SharedString::from(format!("{id}-copy-path-{n}")))
                        .outline()
                        .small()
                        .icon(Icon::new(Lucide::Copy))
                        .label(cac_core::text_with(locale(cx), "work:requests.copyFile", &[("name", &name)]))
                        .on_click(cx.listener(move |this, _, _, cx| this.copy_local_file(target.clone(), cx)))
                        .into_any_element()
                }
                LocalFile::Refused(why) => {
                    let line = refusal_line(&file_name(&path), &why, cx);
                    div()
                        .id(SharedString::from(format!("{id}-cannot-copy-{n}")))
                        .aria_label(line.clone())
                        .leading_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(line)
                        .test_support()
                        .into_any_element()
                }
            };
            let shown = path.clone();
            let reveal_button = Button::new(SharedString::from(format!("{id}-reveal-path-{n}")))
                .ghost()
                .small()
                .icon(Icon::new(Lucide::FolderOpen))
                .tooltip(tr(cx, "work:requests.revealPath"))
                .on_click(cx.listener(move |this, _, _, cx| this.reveal_in_folder(shown.clone(), cx)));
            rows.push(div().flex().flex_wrap().items_center().gap_2().child(copy).child(reveal_button).into_any_element());
        }
        (!rows.is_empty()).then(|| div().id(SharedString::from(format!("{id}-paths"))).flex().flex_col().gap_1().children(rows).into_any_element())
    }

    fn reveal_in_folder(&mut self, path: String, cx: &mut Context<Self>) {
        let shown = path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { reveal(&path) }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => return,
                    Err(cac_mcp::host::RevealError::Failed(_)) => this.said = Some(tr(cx, "common:last.openLinkFailed")),
                    Err(_) => this.said = Some(cac_core::text_with(locale(cx), "work:requests.pathMissing", &[("path", &shown)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Un fichero adjunto: «Copiar <nombre>» a la vista y su contenido en mono,
    /// plegado si es largo. El contenido es texto: ni enlaces ni nada que se
    /// pulse (lo escribió un agente). La lista no trae el contenido: copiar o
    /// desplegar lo pide al detalle del pedido.
    fn file_block(&mut self, request: &AgentRequest, n: usize, file: &RequestFile, cx: &mut Context<Self>) -> AnyElement {
        let code = request.code.clone();
        let id = format!("request-file-{code}-{n}");
        let key = file_key(&code, request.version, n);
        let content = if file.has_content() { Some(file.content.clone()) } else { self.file_contents.get(&key).cloned() };
        let unfolded = self.unfolded.contains(&id);
        let name = file.name.clone();
        let copy = {
            let (content, name, code) = (content.clone(), name.clone(), code.clone());
            Button::new(SharedString::from(format!("{id}-copy")))
                .outline()
                .small()
                .icon(Icon::new(Lucide::Copy))
                .label(cac_core::text_with(locale(cx), "work:requests.copyFile", &[("name", &file.name)]))
                .on_click(cx.listener(move |this, _, _, cx| match &content {
                    Some(content) => this.copy_file(content.clone(), &name, cx),
                    None => this.fetch_file(code.clone(), n, name.clone(), FileWanted::Copy, cx),
                }))
        };
        let Some(content) = content else {
            // Sin contenido todavía: «Ver el contenido» lo trae y lo despliega.
            let show = {
                let (code, name, id) = (code.clone(), name.clone(), id.clone());
                Button::new(SharedString::from(format!("{id}-fold")))
                    .ghost()
                    .small()
                    .label(tr(cx, "work:requests.showFile"))
                    .on_click(cx.listener(move |this, _, _, cx| this.fetch_file(code.clone(), n, name.clone(), FileWanted::Show(id.clone()), cx)))
            };
            return div().flex().flex_wrap().items_center().gap_2().child(copy).child(show).into_any_element();
        };
        let total = content.lines().count();
        let shown = if total > FOLDED_LINES && !unfolded { content.lines().take(FOLDED_LINES).collect::<Vec<_>>().join("\n") } else { content.clone() };
        let toggle = (total > FOLDED_LINES).then(|| {
            let key = id.clone();
            let label = if unfolded { tr(cx, "work:requests.fold") } else { cac_core::text_with(locale(cx), "work:requests.unfold", &[("count", &total.to_string())]) };
            Button::new(SharedString::from(format!("{id}-fold")))
                .ghost()
                .small()
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.unfolded.remove(&key) {
                        this.unfolded.insert(key.clone());
                    }
                    cx.notify();
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().flex().flex_wrap().items_center().gap_2().child(copy).children(toggle))
            .child(self.block(format!("{id}-content"), shown, true, None, cx).test_support())
            .into_any_element()
    }

    fn copy_file(&mut self, content: String, name: &str, cx: &mut Context<Self>) {
        self.copy(content, None, cx);
        self.said = Some(cac_core::text_with(locale(cx), "work:requests.fileCopied", &[("name", name)]));
    }

    /// Pide al detalle del pedido el contenido de sus ficheros, lo guarda y
    /// hace lo que se pulsó: copiar el que se pidió o desplegarlo.
    fn fetch_file(&mut self, code: String, n: usize, name: String, wanted: FileWanted, cx: &mut Context<Self>) {
        let asked = code.clone();
        let files = Backend::ask(cx, move |core| async move { core.requests().files_of(&asked).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = files.await else { return };
            this.update(cx, |this, cx| {
                // El detalle trae los ficheros en el mismo orden que la lista:
                // se guardan todos, con la versión que trae (si se rehízo, la
                // lista nueva los encuentra), y el nombre confirma el pulsado.
                let Ok(detail) = result else {
                    this.said = Some(cac_core::text_with(locale(cx), "work:requests.fileFailed", &[("name", &name)]));
                    cx.notify();
                    return;
                };
                for (i, file) in detail.files.iter().enumerate().filter(|(_, file)| file.has_content()) {
                    this.file_contents.insert(file_key(&code, detail.version, i), file.content.clone());
                }
                let files = detail.files;
                let found = files.into_iter().nth(n).filter(|file| file.name == name && file.has_content());
                match found {
                    Some(file) => {
                        match wanted {
                            FileWanted::Copy => this.copy_file(file.content, &name, cx),
                            FileWanted::Show(id) => {
                                this.unfolded.insert(id);
                            }
                        }
                    }
                    None => this.said = Some(cac_core::text_with(locale(cx), "work:requests.fileFailed", &[("name", &name)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Los adjuntos de un sitio de la tarjeta, o nada.
    fn files_in(&mut self, request: &AgentRequest, slot: RequestFileSlot, cx: &mut Context<Self>) -> Vec<AnyElement> {
        request.body.files.iter().enumerate().filter(|(_, file)| file.slot() == slot).map(|(n, file)| self.file_block(request, n, file, cx)).collect()
    }

    fn render_card(&mut self, request: &AgentRequest, highlighted: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let open = request.status == RequestStatus::Open;
        if open && request.can_answer {
            self.card(&request.code, window, cx);
        }
        let language = locale(cx);
        let theme = cx.theme().clone();
        let (border, muted, primary, mono) = (theme.border, theme.muted_foreground, theme.primary, theme.mono_font_family.clone());
        let code = request.code.clone();
        let copied = self.copied.as_deref() == Some(code.as_str());
        let task_request = request.clone();
        let age = cac_core::time_ago(&request.created_at, &chrono::Utc::now(), language);
        let mut asked = cac_core::text_with(language, "work:requests.askedBy", &[("name", &request.requested_by.name)]);
        if request.via == "mcp" {
            asked = format!("{asked} {}", tr(cx, "work:requests.viaAgent"));
        }
        let asked = format!("{asked} · {}", cac_core::text_with(language, "work:requests.waitsOn", &[("name", &request.requested_of.name)]));
        let copy_label = cac_core::text_with(language, "work:requests.copyId", &[("code", &code)]);
        let copy_code = code.clone();

        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id(SharedString::from(format!("request-kind-{code}")))
                    .px_2()
                    .py_0p5()
                    .rounded(px(6.))
                    .bg(theme.secondary)
                    .text_color(theme.secondary_foreground)
                    .leading_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(request.kind.label(language))
                    .test_support(),
            )
            .child(
                div()
                    .id(SharedString::from(format!("request-task-{code}")))
                    .min_w_0()
                    .truncate()
                    .leading_xs()
                    .text_color(muted)
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .tooltip({
                        let hint = tr(cx, "work:requests.openTask");
                        move |window, cx| gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx)
                    })
                    .on_click(move |_, _, cx| {
                        let request = task_request.clone();
                        Backend::run(cx, move |core| async move {
                            core.requests().open_task(&request).await;
                        });
                    })
                    .child(request.task_line())
                    .test_support(),
            )
            .child(div().leading_xs().text_color(muted).child(format!("· {age}")))
            .child(
                Button::new(SharedString::from(format!("request-copy-{code}")))
                    .outline()
                    .xsmall()
                    .ml_auto()
                    .icon(Icon::new(if copied { Lucide::Check } else { Lucide::Copy }))
                    .child(div().font_family(mono.clone()).child(code.clone()))
                    .tooltip(copy_label)
                    .on_click(cx.listener(move |this, _, _, cx| this.copy(copy_code.clone(), Some(copy_code.clone()), cx))),
            );

        let summary = self.rich(format!("request-summary-{code}"), &request.summary, &request.org_id, cx);
        let summary_paths = self.path_actions(&format!("request-summary-{code}"), &[&request.summary], cx);
        let (tops, anchor, scroll, noted) = (self.tops.clone(), self.anchor.clone(), self.scroll.clone(), code.clone());
        let mut column = div()
            .id(SharedString::from(format!("request-{code}")))
            .relative()
            .child(
                canvas(
                    move |bounds, window, _| {
                        tops.borrow_mut().insert(noted.clone(), bounds);
                        // Si esta es la tarjeta que se leía, y la lista nueva
                        // la dejó en otro sitio, se sube o baja lo que se desliza
                        // lo que se movió y se pinta otra vez.
                        let was = anchor.borrow().as_ref().filter(|(code, _)| *code == noted).map(|(_, was)| *was);
                        if let Some(was) = was {
                            anchor.borrow_mut().take();
                            let moved = (bounds.top() - scroll.bounds().top()) - was;
                            if moved.abs() > px(0.5) {
                                let offset = scroll.offset();
                                scroll.set_offset(point(offset.x, offset.y - moved));
                                window.refresh();
                            }
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(px(10.))
            .border_1()
            .border_color(if highlighted { primary } else { border })
            .when(highlighted, |this| {
                this.shadow(vec![BoxShadow { color: primary, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false }])
            })
            .bg(crate::theme::css_color("raised", cx))
            .leading_sm()
            .child(header)
            .child(div().whitespace_normal().child(summary))
            .children(summary_paths)
            .child(div().leading_xs().text_color(muted).child(asked))
            .child(self.render_body(request, cx));
        column = if open {
            if request.can_answer {
                column.child(self.render_controls(request, cx))
            } else {
                column.child(
                    div()
                        .id(SharedString::from(format!("request-not-yours-{code}")))
                        .leading_xs()
                        .text_color(muted)
                        .child(cac_core::text_with(language, "work:requests.notYours", &[("name", &request.requested_of.name)]))
                        .test_support(),
                )
            }
        } else {
            column.child(self.render_outcome(request, cx))
        };
        if let Some(problem) = self.problems.get(&code) {
            column = column.child(div().id(SharedString::from(format!("request-problem-{code}"))).leading_xs().text_color(theme.danger).child(problem.clone()).test_support());
        }
        column.test_support().into_any_element()
    }

    fn field_label(label: String, cx: &App) -> Div {
        div().leading_xs().font_weight(FontWeight::MEDIUM).text_color(cx.theme().muted_foreground).child(label)
    }

    /// Un bloque de texto en caja (el borrador, el comando, lo que salió). Con
    /// enlaces pintados si `org_id` viene: copiar sigue copiando el texto crudo,
    /// que no pasa por aquí.
    fn block(&self, id: String, text: String, mono: bool, org_id: Option<&str>, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let lines: Vec<AnyElement> = text
            .lines()
            .enumerate()
            .map(|(n, line)| {
                let content = match org_id {
                    Some(org) => self.rich(format!("{id}-line-{n}"), line, org, cx),
                    None => Self::selectable(format!("{id}-line-{n}"), line.to_string()).into_any_element(),
                };
                div().min_w_0().child(content).into_any_element()
            })
            .collect();
        div()
            .id(SharedString::from(id))
            .w_full()
            .min_w_0()
            .p_3()
            .rounded(px(6.))
            .bg(theme.muted)
            .when(mono, |this| this.font_family(theme.mono_font_family.clone()))
            .children(lines)
    }

    /// Un campo con su etiqueta, lo que lleva, y debajo los botones de sus
    /// ficheros (los que nombra por su ruta y los adjuntos de su sitio).
    fn field(label: String, content: impl IntoElement, below: Vec<AnyElement>, cx: &App) -> Div {
        div().flex().flex_col().gap_1().child(Self::field_label(label, cx)).child(content).children(below)
    }

    /// Lo que el agente dejó, según el tipo. Los ficheros adjuntos van junto a
    /// lo que se corre («apply») y bajo «Para deshacerlo» («undo»); en un tipo
    /// sin comando, al final.
    fn render_body(&mut self, request: &AgentRequest, cx: &mut Context<Self>) -> AnyElement {
        let code = request.code.clone();
        let body = request.body.clone();
        let primary = cx.theme().primary;
        let org = request.org_id.clone();
        let column = div().flex().flex_col().gap_2();
        let paths = |this: &mut Self, name: &str, texts: &[&str], cx: &mut Context<Self>| this.path_actions(&format!("request-{name}-{code}"), texts, cx).into_iter().collect::<Vec<_>>();
        let column = match request.kind {
            RequestKind::ApproveMessage => {
                let draft_block = self.block(format!("request-draft-{code}"), body.draft.clone(), false, Some(&org), cx);
                let destination: AnyElement = if body.destination_url.is_empty() {
                    Self::selectable(format!("request-destination-{code}"), body.destination.clone()).into_any_element()
                } else {
                    let (url, org) = (body.destination_url.clone(), request.org_id.clone());
                    div()
                        .id(SharedString::from(format!("request-destination-{code}")))
                        .text_color(primary)
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(cx.listener(move |this, _, _, cx| this.open_link(url.clone(), org.clone(), cx)))
                        .child(body.destination.clone())
                        .into_any_element()
                };
                let draft = body.draft.clone();
                let copy_draft = Button::new(SharedString::from(format!("request-copy-draft-{code}")))
                    .outline()
                    .small()
                    .icon(Icon::new(Lucide::Copy))
                    .tooltip(tr(cx, "work:requests.copyDraft"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.copy(draft.clone(), None, cx);
                        this.said = Some(tr(cx, "work:requests.draftCopied"));
                    }));
                let draft_paths = paths(self, "draft", &[&body.draft], cx);
                column
                    .child(Self::field(tr(cx, "work:requests.destination"), destination, Vec::new(), cx))
                    .child(Self::field(tr(cx, "work:requests.draft"), div().flex().items_start().gap_2().child(draft_block.flex_1().test_support()).child(copy_draft), draft_paths, cx))
            }
            RequestKind::Test => {
                let steps: Vec<AnyElement> = body
                    .steps
                    .iter()
                    .enumerate()
                    .map(|(n, step)| div().flex().gap_1p5().child(div().flex_none().child(format!("{}.", n + 1))).child(div().min_w_0().child(self.rich(format!("request-step-{code}-{n}"), step, &org, cx))).into_any_element())
                    .collect();
                let steps_text: Vec<&str> = body.steps.iter().map(String::as_str).collect();
                let step_paths = paths(self, "steps", &steps_text, cx);
                let expected = self.rich(format!("request-expected-{code}"), &body.expected, &org, cx);
                let expected_paths = paths(self, "expected", &[&body.expected], cx);
                let (url, org) = (body.url.clone(), request.org_id.clone());
                column
                    .when(!body.url.is_empty(), |this| {
                        this.child(
                            div().child(
                                Button::new(SharedString::from(format!("request-open-screen-{code}")))
                                    .outline()
                                    .small()
                                    .icon(Icon::new(Lucide::ExternalLink))
                                    .label(tr(cx, "work:requests.openScreen"))
                                    .on_click(cx.listener(move |this, _, _, cx| this.open_link(url.clone(), org.clone(), cx))),
                            ),
                        )
                    })
                    .child(Self::field(tr(cx, "work:requests.steps"), div().flex().flex_col().gap_1().children(steps), step_paths, cx))
                    .child(Self::field(tr(cx, "work:requests.expected"), expected, expected_paths, cx))
            }
            RequestKind::Decide => {
                let question = self.rich(format!("request-question-{code}"), &body.question, &org, cx);
                let question_paths = paths(self, "question", &[&body.question], cx);
                column.when(!body.question.is_empty(), |this| this.child(div().font_weight(FontWeight::MEDIUM).child(question)).children(question_paths))
            }
            RequestKind::Run => {
                let command = body.command.clone();
                let (where_, expected, undo) = (
                    self.rich(format!("request-where-{code}"), &body.where_, &org, cx),
                    self.rich(format!("request-expected-{code}"), &body.expected, &org, cx),
                    self.rich(format!("request-undo-{code}"), &body.undo, &org, cx),
                );
                let where_paths = paths(self, "where", &[&body.where_], cx);
                let expected_paths = paths(self, "expected", &[&body.expected], cx);
                let mut command_below = paths(self, "command", &[&body.command], cx);
                command_below.extend(self.files_in(request, RequestFileSlot::Apply, cx));
                let mut undo_below = paths(self, "undo", &[&body.undo], cx);
                undo_below.extend(self.files_in(request, RequestFileSlot::Undo, cx));
                let has_undo = !body.undo.is_empty() || !undo_below.is_empty();
                let command_block = self.block(format!("request-command-{code}"), body.command.clone(), true, Some(&org), cx);
                let command_row = div().flex().items_start().gap_2().child(command_block.flex_1().test_support()).child(
                    Button::new(SharedString::from(format!("request-copy-command-{code}")))
                        .outline()
                        .small()
                        .icon(Icon::new(Lucide::Copy))
                        .tooltip(tr(cx, "work:requests.copyCommand"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy(command.clone(), None, cx);
                            this.said = Some(tr(cx, "work:requests.commandCopied"));
                        })),
                );
                column
                    .child(Self::field(tr(cx, "work:requests.where"), where_, where_paths, cx))
                    .child(Self::field(tr(cx, "work:requests.command"), command_row, command_below, cx))
                    .child(Self::field(tr(cx, "work:requests.expected"), expected, expected_paths, cx))
                    .when(has_undo, |this| this.child(Self::field(tr(cx, "work:requests.undo"), div().when(!body.undo.is_empty(), |this| this.child(undo)), undo_below, cx)))
            }
            RequestKind::Obtain => {
                let (what, from) = (self.rich(format!("request-what-{code}"), &body.what, &org, cx), self.rich(format!("request-from-{code}"), &body.from, &org, cx));
                let what_paths = paths(self, "what", &[&body.what], cx);
                column.child(Self::field(tr(cx, "work:requests.what"), what, what_paths, cx)).child(Self::field(tr(cx, "work:requests.from"), from, Vec::new(), cx))
            }
            RequestKind::Unknown => column,
        };
        // Lo que el tipo no colocó: sin comando, lo de aplicar va con el resto.
        let mut rest = Vec::new();
        if request.kind != RequestKind::Run {
            rest.extend(self.files_in(request, RequestFileSlot::Apply, cx));
        }
        rest.extend(self.files_in(request, RequestFileSlot::Other, cx));
        let undo = if request.kind != RequestKind::Run { self.files_in(request, RequestFileSlot::Undo, cx) } else { Vec::new() };
        column
            .when(!rest.is_empty(), |this| this.child(Self::field(tr(cx, "work:requests.files"), div().flex().flex_col().gap_2().children(rest), Vec::new(), cx)))
            .when(!undo.is_empty(), |this| this.child(Self::field(tr(cx, "work:requests.undo"), div().flex().flex_col().gap_2().children(undo), Vec::new(), cx)))
            .into_any_element()
    }

    fn textarea(id: String, field: &Entity<TextareaState>, mono: bool, cx: &App) -> Stateful<Div> {
        div()
            .id(SharedString::from(id))
            .when(mono, |this| this.font_family(cx.theme().mono_font_family.clone()))
            .child(Textarea::new(field))
    }

    /// Los botones del tipo.
    fn render_controls(&mut self, request: &AgentRequest, cx: &mut Context<Self>) -> AnyElement {
        let code = request.code.clone();
        let Some(card) = self.cards.get(&code) else { return div().into_any_element() };
        let (mode, text, note, reason, output) = (card.mode, card.text.clone(), card.note.clone(), card.reason.clone(), card.output.clone());
        let busy = self.busy.contains(&code);
        let note_typed = !Self::value(&note, cx).trim().is_empty();
        let row = || div().flex().flex_wrap().gap_2();
        let id = |name: &str| SharedString::from(format!("request-{name}-{code}"));
        // Cuando la nota es obligatoria se dice encima: el campo es el mismo,
        // y lo escrito antes de saberlo no se pierde.
        let note_box = |required: bool, cx: &mut Context<Self>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .when(required, |this| this.child(div().id(SharedString::from(format!("request-note-required-{code}"))).child(Self::field_label(tr(cx, "work:requests.noteRequired"), cx)).test_support()))
                .child(Self::textarea(format!("request-note-{code}"), &note, false, cx).test_support())
        };
        match request.kind {
            RequestKind::ApproveMessage if mode == Mode::Edit => {
                let typed = Self::value(&text, cx);
                let original = request.body.draft.clone();
                let send_code = code.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Self::textarea(format!("request-text-{code}"), &text, false, cx).test_support())
                    .child(
                        row()
                            .child(Button::new(id("approve-text")).primary().small().label(tr(cx, "work:requests.approveThis")).disabled(busy || typed.trim().is_empty()).on_click(cx.listener(move |this, _, _, cx| {
                                let edited = Self::value(&text, cx);
                                this.send(send_code.clone(), AnswerDraft::Approve { edited: Some(edited), original: original.clone() }, cx)
                            })))
                            .child(Button::new(id("cancel")).ghost().small().label(tr(cx, "work:requests.cancel")).on_click(cx.listener({
                                let code = code.clone();
                                move |this, _, _, cx| this.set_mode(&code, Mode::Idle, cx)
                            }))),
                    )
                    .into_any_element()
            }
            RequestKind::ApproveMessage if mode == Mode::Reject => {
                let send_code = code.clone();
                let reason_typed = !Self::value(&reason, cx).trim().is_empty();
                let typed_reason = reason.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Self::textarea(format!("request-reason-{code}"), &reason, false, cx).test_support())
                    .child(
                        row()
                            .child(Button::new(id("send-reject")).danger().small().label(tr(cx, "work:requests.sendReject")).disabled(busy || !reason_typed).on_click(cx.listener(move |this, _, _, cx| {
                                let reason = Self::value(&typed_reason, cx);
                                this.send(send_code.clone(), AnswerDraft::Reject { reason }, cx)
                            })))
                            .child(Button::new(id("cancel")).ghost().small().label(tr(cx, "work:requests.cancel")).on_click(cx.listener({
                                let code = code.clone();
                                move |this, _, _, cx| this.set_mode(&code, Mode::Idle, cx)
                            }))),
                    )
                    .into_any_element()
            }
            RequestKind::ApproveMessage => {
                let (approve_code, original, reject_code, edit_request) = (code.clone(), request.body.draft.clone(), code.clone(), request.clone());
                row()
                    .child(Button::new(id("approve")).primary().small().label(tr(cx, "work:requests.approve")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
                        this.send(approve_code.clone(), AnswerDraft::Approve { edited: None, original: original.clone() }, cx)
                    })))
                    .child(Button::new(id("edit")).outline().small().label(tr(cx, "work:requests.editApprove")).disabled(busy).on_click(cx.listener(move |this, _, window, cx| this.start_edit(&edit_request, window, cx))))
                    .child(Button::new(id("reject")).danger().small().label(tr(cx, "work:requests.reject")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| this.set_mode(&reject_code, Mode::Reject, cx))))
                    .into_any_element()
            }
            RequestKind::Test => {
                let failing = mode == Mode::Fails;
                let (works_code, fails_code, works_note, fails_note) = (code.clone(), code.clone(), note.clone(), note.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(note_box(failing, cx))
                    .child(
                        row()
                            .child(Button::new(id("works")).primary().small().label(tr(cx, "work:requests.works")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
                                let note = Self::value(&works_note, cx);
                                this.send(works_code.clone(), AnswerDraft::Works { note }, cx)
                            })))
                            .child(Button::new(id("fails")).danger().small().label(tr(cx, "work:requests.fails")).disabled(busy || (failing && !note_typed)).on_click(cx.listener(move |this, _, _, cx| {
                                // «No funciona» pide decir qué se vio: el
                                // primer clic sin nota la pide; con ella, manda.
                                let note = Self::value(&fails_note, cx);
                                if note.trim().is_empty() {
                                    this.set_mode(&fails_code, Mode::Fails, cx);
                                    return;
                                }
                                this.send(fails_code.clone(), AnswerDraft::Fails { note }, cx)
                            }))),
                    )
                    .into_any_element()
            }
            RequestKind::Decide => {
                let options = request.body.options.iter().enumerate().map(|(n, option)| {
                    let (choose_code, label, chosen_note) = (code.clone(), option.label.clone(), note.clone());
                    // El texto va fuera del botón: arrastrar para seleccionarlo
                    // terminaría con un mouse-up sobre «Elegir» y elegiría sin querer.
                    let text = div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .py_1()
                        .child(
                            div().flex().items_center().gap_2().child(Self::selectable(format!("request-option-label-{code}-{n}"), option.label.clone())).when(option.recommended, |this| {
                                this.child(div().px_1p5().rounded(px(4.)).border_1().leading_xs().child(tr(cx, "work:requests.recommended")))
                            }),
                        )
                        .child(div().leading_xs().whitespace_normal().opacity(0.8).child(Self::selectable(format!("request-option-consequence-{code}-{n}"), option.consequence.clone())));
                    let button = Button::new(SharedString::from(format!("request-option-{code}-{n}")))
                        .small()
                        .disabled(busy)
                        .label(tr(cx, "work:requests.choose"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let note = Self::value(&chosen_note, cx);
                            this.send(choose_code.clone(), AnswerDraft::Choose { option: label.clone(), note }, cx)
                        }));
                    let button = if option.recommended { button.primary() } else { button.outline() };
                    div().flex().items_center().gap_2().child(text).child(button)
                });
                div().flex().flex_col().gap_2().children(options.collect::<Vec<_>>()).child(note_box(false, cx)).into_any_element()
            }
            RequestKind::Run => {
                let (done_code, pasted) = (code.clone(), output.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Self::textarea(format!("request-output-{code}"), &output, true, cx).test_support())
                    .child(row().child(Button::new(id("done")).primary().small().label(tr(cx, "work:requests.done")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
                        let output = Self::value(&pasted, cx);
                        this.send(done_code.clone(), AnswerDraft::Done { note: String::new(), output }, cx)
                    }))))
                    .into_any_element()
            }
            RequestKind::Obtain => {
                let (done_code, typed) = (code.clone(), note.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(note_box(false, cx))
                    .child(row().child(Button::new(id("ready")).primary().small().label(tr(cx, "work:requests.ready")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
                        let note = Self::value(&typed, cx);
                        this.send(done_code.clone(), AnswerDraft::Done { note, output: String::new() }, cx)
                    }))))
                    .into_any_element()
            }
            RequestKind::Unknown => div().into_any_element(),
        }
    }

    /// Cómo quedó un pedido cerrado.
    fn render_outcome(&mut self, request: &AgentRequest, cx: &mut Context<Self>) -> AnyElement {
        let language = locale(cx);
        let theme = cx.theme().clone();
        let code = request.code.clone();
        let answer = request.answer.clone().unwrap_or_default();
        let by = request.answered_by.as_ref().map(|person| {
            let when = cac_core::time_ago(request.answered_at.as_deref().unwrap_or_default(), &chrono::Utc::now(), language);
            let mut line = cac_core::text_with(language, "work:requests.outcome.by", &[("name", &person.name), ("when", &when)]);
            if request.answered_via == "session" {
                line = format!("{line} · {}", tr(cx, "work:requests.outcome.viaSession"));
            }
            line
        });
        let (published, org) = (request.published_url.clone(), request.org_id.clone());
        let output_block = (!answer.output.is_empty()).then(|| self.block(format!("request-outcome-output-{code}"), answer.output.clone(), true, None, cx));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded(px(6.))
            .bg(theme.muted)
            .child(div().font_weight(FontWeight::MEDIUM).child(Self::selectable(format!("request-outcome-{code}"), cac_core::request_outcome(request, language))))
            .when(!answer.note.is_empty() && request.kind != RequestKind::Test, |this| this.child(Self::selectable(format!("request-outcome-note-{code}"), answer.note.clone())))
            .children(output_block)
            .when(answer.edited && !answer.final_text.is_empty(), |this| this.child(div().leading_xs().child(Self::selectable(format!("request-outcome-final-{code}"), answer.final_text.clone()))))
            .children(by.map(|line| div().leading_xs().text_color(theme.muted_foreground).child(Self::selectable(format!("request-outcome-by-{code}"), line))))
            .when(!published.is_empty(), |this| {
                this.child(
                    div()
                        .id(SharedString::from(format!("request-published-{code}")))
                        .leading_xs()
                        .text_color(theme.primary)
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(cx.listener(move |this, _, _, cx| this.open_link(published.clone(), org.clone(), cx)))
                        .child(tr(cx, "work:requests.outcome.published")),
                )
            })
            .into_any_element()
    }

    fn render_scope(&self, scope: RequestScope, cx: &mut Context<Self>) -> impl IntoElement {
        let chip = |id: &'static str, this: RequestScope, key: &str, cx: &mut Context<Self>| {
            let button = Button::new(id).small().label(tr(cx, key)).on_click(cx.listener(move |view, _, _, cx| {
                view.cards.clear();
                view.problems.clear();
                cx.notify();
                Backend::run(cx, move |core| async move { core.requests().set_scope(this).await });
            }));
            if scope == this { button.secondary() } else { button.ghost() }
        };
        div()
            .flex()
            .gap_0p5()
            .p_0p5()
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().border)
            .child(chip("requests-scope-mine", RequestScope::Mine, "work:requests.scope.mine", cx))
            .child(chip("requests-scope-team", RequestScope::Team, "work:requests.scope.team", cx))
    }
}

/// El nombre de un fichero por su ruta, para el botón.
fn file_name(path: &str) -> String {
    let path = path.trim_end_matches(['/', '\\']);
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// «apply.sql: es un enlace que apunta fuera de su carpeta».
fn refusal_line(name: &str, why: &Refusal, cx: &App) -> String {
    let reason = tr(cx, &format!("work:requests.fileRefused.{}", why.code()));
    cac_core::text_with(locale(cx), "work:requests.cannotCopy", &[("name", name), ("why", &reason)])
}

/// Pulsar una ruta: un documento se abre con su programa, lo demás se
/// muestra en la carpeta (`cac_mcp::host::open_path_with`). En las pruebas se
/// decide igual, con la misma regla, y se anota en vez de lanzar.
fn open_path(path: &str) -> Result<Opened, cac_mcp::host::RevealError> {
    #[cfg(test)]
    {
        let Ok(real) = std::fs::canonicalize(cac_mcp::host::expand_home(path)) else {
            return Err(cac_mcp::host::RevealError::NotFound);
        };
        if !cac_mcp::host::opens_with_its_program(&real) {
            return reveal(path).map(|()| Opened::Revealed(None));
        }
        tests::OPENED_FILES.lock().unwrap().push(path.to_string());
        Ok(Opened::WithItsProgram)
    }
    #[cfg(not(test))]
    cac_mcp::host::open_path(path)
}

/// Mostrar una ruta en el gestor de ficheros, seleccionada: nunca se abre
/// (ver `cac_mcp::host::reveal_path`). En las pruebas sólo se anota.
fn reveal(path: &str) -> Result<(), cac_mcp::host::RevealError> {
    #[cfg(test)]
    {
        if !cac_mcp::host::expand_home(path).exists() {
            return Err(cac_mcp::host::RevealError::NotFound);
        }
        tests::OPENED.lock().unwrap().push(path.to_string());
        Ok(())
    }
    #[cfg(not(test))]
    cac_mcp::host::reveal(path)
}

impl Render for RequestsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).requests.clone();
        let theme = cx.theme().clone();
        let (background, foreground, muted, border, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.danger);
        // El enlace se atiende una vez, cuando su pedido aparece: se lleva la
        // vista a él, y «Resueltos» se abre sólo si ya estaba resuelto. Que
        // luego se conteste (y pase a resueltos) o lleguen otros no lo vuelve
        // a abrir ni a mover.
        if let Some(code) = state.highlight.as_ref().filter(|code| self.shown_highlight.as_ref() != Some(code)) {
            let closed = state.closed.iter().any(|request| &request.code == code);
            if closed || state.open.iter().any(|request| &request.code == code) {
                self.shown_highlight = Some(code.clone());
                self.scroll_to = Some(code.clone());
            }
        } else if state.highlight.is_none() {
            self.shown_highlight = None;
        }
        // Lo escrito en un pedido que ya no está abierto no tiene a dónde ir.
        self.cards.retain(|code, _| state.open.iter().any(|request| &request.code == code));

        let highlight = state.highlight.clone();
        // Con «Todos», los de abajo van plegados en «Resueltos»; filtrando por
        // uno de sus estados, se enseñan sin pliegue y no hay abiertos.
        let status = state.filter.status;
        let shows_open = matches!(status, RequestStatusFilter::All | RequestStatusFilter::Open);
        let has_toggle = status == RequestStatusFilter::All;
        let resolved_open = match status {
            RequestStatusFilter::All => state.resolved_open,
            RequestStatusFilter::Open => false,
            _ => true,
        };
        let open: Vec<AnyElement> = state.open.iter().map(|request| self.render_card(request, highlight.as_deref() == Some(request.code.as_str()), window, cx)).collect();
        let closed: Vec<AnyElement> = if resolved_open {
            state.closed.iter().map(|request| self.render_card(request, highlight.as_deref() == Some(request.code.as_str()), window, cx)).collect()
        } else {
            Vec::new()
        };
        // `scroll_to_item` cuenta hijos directos de lo que se desliza: en qué
        // puesto va la resaltada, contando lo de encima (título, filtros,
        // error, estado), el final de los abiertos y el pliegue «Resueltos».
        let before_open = 3 + usize::from(state.failed) + state.elsewhere.len();
        let shown: Vec<&str> = state.open.iter().chain(state.closed.iter().filter(|_| resolved_open)).map(|request| request.code.as_str()).collect();
        let at = highlight.as_deref().and_then(|code| {
            let n = shown.iter().position(|shown| *shown == code)?;
            Some(before_open + n + if n >= state.open.len() { 1 + usize::from(has_toggle) } else { 0 })
        });
        self.keep_reading(shown.iter().map(|code| code.to_string()).collect());
        if let (Some(_), Some(at)) = (self.scroll_to.as_ref(), at) {
            // Un `ScrollHandle` que aún no se pintó no sabe su alto y tira la
            // petición sin moverse: se pide cuando la lista ya se pintó.
            if self.scroll.bounds().size.height > px(0.) {
                self.scroll.scroll_to_item(at);
                self.scroll_to = None;
            } else {
                window.refresh();
            }
        }

        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(div().id("requests-title").leading_lg().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "work:requests.title")).test_support())
                    .child(div().leading_sm().text_color(muted).child(tr(cx, "work:requests.lead"))),
            )
            .child(self.render_scope(state.scope, cx));
        let filtered = !state.filter.text.trim().is_empty() || !state.filter.kinds.is_empty() || status != RequestStatusFilter::All;
        let empty_key = if filtered {
            "work:requests.noMatches"
        } else if state.scope == RequestScope::Mine {
            "work:requests.empty"
        } else {
            "work:requests.emptyTeam"
        };
        let nothing = if shows_open { state.open.is_empty() } else { state.closed_paging.loaded && state.closed.is_empty() };
        let status_line: AnyElement = if !state.loaded {
            div().id("requests-loading").leading_sm().text_color(muted).child(tr(cx, "work:requests.loading")).test_support().into_any_element()
        } else if nothing {
            div()
                .id("requests-empty")
                .aria_label(tr(cx, empty_key))
                .p_6()
                .rounded(px(10.))
                .border_1()
                .border_dashed()
                .border_color(border)
                .text_center()
                .leading_sm()
                .text_color(muted)
                .child(tr(cx, empty_key))
                .test_support()
                .into_any_element()
        } else {
            div().into_any_element()
        };
        // Sin número: lo cargado no es cuántos hay, y contarlos todos es
        // justo lo que la paginación evita.
        let toggle = (has_toggle && state.loaded).then(|| {
            div()
                .id("requests-resolved")
                .flex()
                .items_center()
                .gap_2()
                .leading_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(muted)
                .cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .on_click(cx.listener(move |_, _, _, cx| {
                    Backend::run(cx, move |core| async move { core.requests().set_resolved_open(!resolved_open).await });
                }))
                .child(Icon::new(if resolved_open { Lucide::ChevronDown } else { Lucide::ChevronRight }).size_4())
                .aria_label(tr(cx, "work:requests.resolvedTitle"))
                .child(tr(cx, "work:requests.resolvedTitle"))
                .test_support()
                .into_any_element()
        });
        let language = locale(cx);
        let elsewhere: Vec<AnyElement> = state
            .elsewhere
            .iter()
            .map(|other| {
                let org_id = other.org_id.clone();
                let text = cac_core::elsewhere_label(other, language);
                div()
                    .id(SharedString::from(format!("requests-elsewhere-{}", other.org_id)))
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .leading_sm()
                    .cursor_pointer()
                    .hover(move |style| style.bg(muted.opacity(0.1)))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let org_id = org_id.clone();
                        Backend::run(cx, move |core| async move {
                            core.switch_org(&org_id).await;
                        });
                    }))
                    .aria_label(text.clone())
                    .child(text)
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let said = self.said.clone();
        let filters = self.render_filters(&state.filter, cx);
        let open_end = self.render_end("requests-open-end", state.open_paging, shows_open, cx);
        let closed_end = self.render_end("requests-closed-end", state.closed_paging, resolved_open, cx);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .line_height(relative(1.5))
            .child(
                div()
                    .id("requests")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .test_support()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w_full()
                    .max_w(rems(48.))
                    .mx_auto()
                    .p_6()
                    .child(header)
                    .child(filters)
                    .children(elsewhere)
                    .when(state.failed, |this| this.child(div().id("requests-error").leading_sm().text_color(danger).child(tr(cx, "work:requests.errLoad")).test_support()))
                    .child(status_line)
                    .children(open)
                    .child(open_end)
                    .children(toggle)
                    .children(closed)
                    .child(closed_end)
                    .children(said.map(|said| {
                        div()
                            .id("requests-said")
                            .aria_label(said.clone())
                            .leading_xs()
                            .text_color(muted)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.said = None;
                                cx.notify();
                            }))
                            .child(said)
                            .test_support()
                    })),
            )
    }
}

/// «Espera a Ana»: la tarea tiene pedidos abiertos a una persona
/// (`WaitingOnChip` de `Tasks.tsx`). A quién, lo sabe el core.
pub(crate) fn waiting_chip(id: String, task_id: &str, cx: &App) -> Option<AnyElement> {
    let names = Backend::snapshot(cx).read(cx).requests.waiting_on.get(task_id).cloned()?;
    let text = cac_core::waiting_label(&names, locale(cx))?;
    let warning = cx.theme().warning;
    Some(
        div()
            .id(SharedString::from(id))
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .px_1p5()
            .py_0p5()
            .rounded(px(4.))
            .bg(warning.opacity(0.15))
            .text_color(warning)
            .leading_xs()
            .aria_label(text.clone())
            .child(Icon::new(Lucide::Hand).size_3())
            .child(text)
            .test_support()
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use crate::backend::Backend;
    use crate::testing::{contract_task, ok, server_with_tasks, sign_in, wait_until, FakeServer, Gate};
    use crate::workspace::Workspace;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{point, px, size, AnyWindowHandle, AppContext, Bounds, Pixels, Point, ScrollDelta, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, ResponseTemplate};

    /// Las rutas que la pantalla habría mostrado en el gestor de ficheros.
    pub(super) static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    /// Los ficheros que habría abierto con su programa.
    pub(super) static OPENED_FILES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn request(code: &str, kind: &str, task: &str, of: (&str, &str), mine: bool, body: Value) -> Value {
        json!({
            "id": format!("id-{code}"), "code": code, "orgId": "o1", "kind": kind, "summary": format!("resumen {code}"), "body": body,
            "version": 1, "status": "open", "requestedOf": { "id": of.0, "name": of.1 }, "requestedBy": { "id": "bot", "name": "Agente" }, "via": "mcp",
            "task": { "id": task, "ref": "cac-1", "title": "Subir el backend", "priority": "high", "status": "in_progress", "link": format!("/tasks?task={task}") },
            "createdAt": "2026-11-24T10:00:00Z", "updatedAt": "2026-11-24T10:00:00Z", "canAnswer": mine, "link": format!("/requests?open={code}"),
        })
    }

    fn mine() -> Vec<Value> {
        let ana = ("u1", "Ana Ruiz");
        vec![
            request("req-a1", "approve_message", "t1", ana, true, json!({ "draft": "Hola equipo", "destination": "Basecamp" })),
            request("req-a2", "test", "t1", ana, true, json!({ "url": "https://staging.example.com", "steps": ["Entra"], "expected": "07:30" })),
        ]
    }

    /// Ana tiene dos pedidos en `t1`; Luis, uno en `t2`.
    fn server() -> FakeServer {
        let server = server_with_tasks(vec![
            contract_task("t1", "a", |task| task["title"] = json!("Subir el backend")),
            contract_task("t2", "a", |task| task["title"] = json!("Revisar el diseño")),
        ]);
        let list = |scope: &str, status: &str| {
            Mock::given(method("GET")).and(path("/api/v1/requests")).and(query_param("scope", scope.to_string())).and(query_param("status", status.to_string()))
        };
        let mut team = mine();
        team.push(request("req-l1", "decide", "t2", ("u2", "Luis"), false, json!({ "question": "¿Cuál?", "options": [{ "label": "A", "consequence": "rápido" }] })));
        server.mount(list("team", "open").respond_with(ok(Value::Array(team))));
        server.mount(list("mine", "open").respond_with(ok(Value::Array(mine()))));
        server.mount(list("mine", "closed").respond_with(ok(json!([]))));
        server.mount(list("team", "closed").respond_with(ok(json!([]))));
        server
    }

    fn open(server: &FakeServer, cx: &mut TestAppContext) -> AnyWindowHandle {
        open_sized(server, 1400., cx)
    }

    fn open_sized(server: &FakeServer, height: f32, cx: &mut TestAppContext) -> AnyWindowHandle {
        server.install(cx);
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1200.), px(height)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window").0
        })
    }

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn label(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).and_then(|found| found.label().map(str::to_string)).unwrap_or_default()
        })
        .unwrap()
    }

    fn click(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    fn type_in(window: AnyWindowHandle, field: &str, text: &str, cx: &mut TestAppContext) {
        let (field, text) = (gpui_kit::SharedString::from(field.to_string()), text.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(field, cx);
            window.input(&text, cx);
        })
        .unwrap();
    }

    fn answers(server: &FakeServer, code: &str) -> Vec<Value> {
        let route = format!("/api/v1/requests/{code}/answer");
        server.requests().iter().filter(|request| request.method.as_str() == "POST" && request.url.path() == route).map(|request| serde_json::from_slice(&request.body).unwrap()).collect()
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    fn show_requests(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "el número de «Por atender»", |cx| on_screen(window, "panel-requests-count", cx));
        click(window, "panel-requests", cx);
        wait_until(cx, "los pedidos de Ana", |cx| on_screen(window, "request-req-a1", cx));
    }

    /// El panel cuenta los tuyos, las tareas dicen a quién esperan, y en
    /// «Equipo» los de otro se leen sin botones mientras el número sigue
    /// siendo tuyo.
    ///
    /// Roturas que atrapa: un número que cuenta los del equipo; la marca
    /// «espera a…» que no llega a la fila; ofrecer contestar el de otro; que
    /// «Mi trabajo» siga delante tras pulsar la fila; que el tablero de
    /// pulsar el árbol no devuelva las tareas.
    #[gpui_kit::test]
    fn the_panel_counts_yours_and_the_team_reads_without_buttons(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        wait_until(cx, "el número de «Por atender»", |cx| on_screen(window, "panel-requests-count", cx));
        assert_eq!(label(window, "panel-requests-count", cx), "2");
        wait_until(cx, "la marca en la fila", |cx| on_screen(window, "work-waiting-t2", cx));
        assert_eq!(label(window, "work-waiting-t1", cx), "Espera a Ana Ruiz");
        assert_eq!(label(window, "work-waiting-t2", cx), "Espera a Luis");

        show_requests(window, cx);
        assert!(!on_screen(window, "work-row-t1", cx), "«Mi trabajo» sigue delante");
        assert!(on_screen(window, "request-approve-req-a1", cx));
        assert!(!on_screen(window, "request-req-l1", cx), "el de Luis no es de Ana");

        click(window, "requests-scope-team", cx);
        wait_until(cx, "los del equipo", |cx| on_screen(window, "request-req-l1", cx));
        assert!(on_screen(window, "request-not-yours-req-l1", cx));
        assert!(!on_screen(window, "request-option-req-l1-0", cx), "el de Luis no se contesta por él");
        assert_eq!(label(window, "panel-requests-count", cx), "2", "el número sigue siendo de Ana");

        click(window, "tree-everything", cx);
        wait_until(cx, "vuelven las tareas", |cx| on_screen(window, "work-row-t1", cx));
        stop(cx);
    }

    /// Los pedidos de otra organización no salen ni cuentan, se avisa de ellos,
    /// y el aviso lleva a esa organización (command-and-control-527).
    ///
    /// Roturas que atrapa: pintar o contar los de la otra organización; no
    /// avisar de ellos; un aviso que no cambia de organización; que tras
    /// cambiar sigan saliendo los de la de antes.
    #[gpui_kit::test]
    fn the_other_organizations_are_announced_not_listed(cx: &mut TestAppContext) {
        let server = server();
        let org = |id: &str, name: &str| json!({ "id": id, "name": name, "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
        server.mount(Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org("o1", "G Studio"), org("o2", "Otra org")]))).with_priority(1));
        let mut elsewhere = request("req-g1", "decide", "t9", ("u1", "Ana Ruiz"), true, json!({ "question": "¿Cuál?", "options": [{ "label": "A", "consequence": "rápido" }] }));
        elsewhere["orgId"] = json!("o2");
        let with_it = |mut list: Vec<Value>| {
            list.push(elsewhere.clone());
            Value::Array(list)
        };
        let list = |scope: &str, status: &str| {
            Mock::given(method("GET")).and(path("/api/v1/requests")).and(query_param("scope", scope.to_string())).and(query_param("status", status.to_string()))
        };
        server.mount(list("team", "open").respond_with(ok(with_it(mine()))).with_priority(1));
        server.mount(list("mine", "open").respond_with(ok(with_it(mine()))).with_priority(1));
        let window = open(&server, cx);
        wait_until(cx, "el número de «Por atender»", |cx| on_screen(window, "panel-requests-count", cx));
        assert_eq!(label(window, "panel-requests-count", cx), "2", "el de la otra organización no cuenta");

        show_requests(window, cx);
        assert!(!on_screen(window, "request-req-g1", cx), "el de la otra organización no se pinta");
        assert_eq!(label(window, "requests-elsewhere-o2", cx), "1 pedido más en Otra org");

        click(window, "requests-elsewhere-o2", cx);
        wait_until(cx, "los de la otra organización", |cx| on_screen(window, "request-req-g1", cx));
        assert!(!on_screen(window, "request-req-a1", cx), "los de la de antes ya no");
        assert_eq!(label(window, "requests-elsewhere-o1", cx), "2 pedidos más en G Studio");
        stop(cx);
    }

    /// Rechazar pide motivo, y mientras el servidor contesta no se puede
    /// mandar otra vez; al contestar pasa a «Resueltos». El ID se copia.
    ///
    /// Roturas que atrapa: rechazar sin motivo; un doble clic que contesta
    /// dos veces; dejarlo en abiertos tras contestarlo; un ID que no copia.
    #[gpui_kit::test]
    fn rejecting_asks_for_a_reason_and_sends_once(cx: &mut TestAppContext) {
        let server = server();
        let gate = Gate::default();
        let mut done = mine()[0].clone();
        done["status"] = json!("answered");
        done["answer"] = json!({ "approved": false, "reason": "el tono" });
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a1/answer")).respond_with(gate.hold(ok(done))));
        let window = open(&server, cx);
        show_requests(window, cx);

        click(window, "request-copy-req-a1", cx);
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("req-a1"));

        click(window, "request-reject-req-a1", cx);
        click(window, "request-send-reject-req-a1", cx);
        assert!(answers(&server, "req-a1").is_empty(), "sin motivo no sale");
        type_in(window, "request-reason-req-a1", "el tono", cx);
        click(window, "request-send-reject-req-a1", cx);
        wait_until(cx, "la respuesta, retenida", |_| gate.held() == 1);
        click(window, "request-send-reject-req-a1", cx);
        assert_eq!(answers(&server, "req-a1"), [json!({ "approved": false, "reason": "el tono" })], "una sola vez");

        gate.open();
        wait_until(cx, "pasa a «Resueltos»", |cx| on_screen(window, "requests-resolved", cx) && !on_screen(window, "request-req-a1", cx));
        click(window, "requests-resolved", cx);
        wait_until(cx, "«Resueltos», desplegado", |cx| on_screen(window, "request-outcome-req-a1", cx));
        stop(cx);
    }

    /// «No funciona» sin nota la pide en vez de mandar; con ella, manda.
    ///
    /// Rotura que atrapa: un «no funciona» que sale sin decir qué se vio.
    #[gpui_kit::test]
    fn doesnt_work_needs_a_note(cx: &mut TestAppContext) {
        let server = server();
        let mut done = mine()[1].clone();
        done["status"] = json!("answered");
        done["answer"] = json!({ "works": false, "note": "sale 18:40" });
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a2/answer")).respond_with(ok(done)));
        let window = open(&server, cx);
        show_requests(window, cx);

        click(window, "request-fails-req-a2", cx);
        assert!(answers(&server, "req-a2").is_empty());
        assert!(on_screen(window, "request-note-required-req-a2", cx), "dice que la nota hace falta");
        type_in(window, "request-note-req-a2", "sale 18:40", cx);
        click(window, "request-fails-req-a2", cx);
        wait_until(cx, "sale la respuesta", |_| !answers(&server, "req-a2").is_empty());
        assert_eq!(answers(&server, "req-a2"), [json!({ "works": false, "note": "sale 18:40" })]);
        stop(cx);
    }

    /// Un agente pide abrir un pedido: «Por atender» delante, aunque se
    /// estuviera en comunicación, y el servidor sabe que se enseñó.
    ///
    /// Roturas que atrapa: que la pantalla no cambie; que el «lo enseñé» no
    /// salga (el MCP diría que no hay app abierta).
    #[gpui_kit::test]
    fn an_agent_asking_to_open_shows_it(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a2/opened")).respond_with(ok(json!(null))));
        let window = open(&server, cx);
        wait_until(cx, "las tareas", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "rail-comms", cx);
        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.stream().deliver(cac_core::ServerEvent { event: "request:open".into(), data: json!({ "code": "req-a2", "nonce": "n1" }).to_string() });
            })
        });
        wait_until(cx, "el pedido delante", |cx| on_screen(window, "request-req-a2", cx));
        wait_until(cx, "el «lo enseñé»", |_| server.hits("/api/v1/requests/req-a2/opened") == 1);
        assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).requests.highlight.clone()).as_deref(), Some("req-a2"));
        stop(cx);
    }

    type Shared = Arc<Mutex<Vec<Value>>>;

    /// Un servidor cuyas listas de «mine» y «team» se cambian en plena prueba.
    fn live_server(open: &Shared, closed: &Shared) -> FakeServer {
        let server = server_with_tasks(vec![contract_task("t1", "a", |task| task["title"] = json!("Subir el backend"))]);
        for scope in ["mine", "team"] {
            for (status, list) in [("open", open), ("closed", closed)] {
                let list = list.clone();
                server.mount(
                    Mock::given(method("GET"))
                        .and(path("/api/v1/requests"))
                        .and(query_param("scope", scope.to_string()))
                        .and(query_param("status", status.to_string()))
                        .respond_with(move |_: &wiremock::Request| ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": list.lock().unwrap().clone() }))),
                );
            }
        }
        server
    }

    fn approve(code: &str, summary: &str) -> Value {
        let mut value = request(code, "approve_message", "t1", ("u1", "Ana Ruiz"), true, json!({ "draft": "Hola", "destination": "Basecamp" }));
        value["summary"] = json!(summary);
        value
    }

    fn bounds(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> Option<Bounds<Pixels>> {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).map(|found| found.bounds())
        })
        .unwrap()
    }

    fn click_start(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click_at(id, point(px(3.), px(3.)), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Una dirección, una ruta que existe, una que no y una pantalla de la app
    /// escritas en el texto de un pedido se pulsan, y cada una hace lo suyo.
    ///
    /// Roturas que atrapa: la dirección que no llega al navegador; la ruta
    /// que no llega al gestor de ficheros, o que se muestra sin existir; una ruta que no
    /// existe y no dice nada; una pantalla de la app que sale al navegador en
    /// vez de abrirse dentro.
    #[gpui_kit::test]
    fn links_in_a_request_open_what_they_point_at(cx: &mut TestAppContext) {
        let here = std::env::temp_dir().to_string_lossy().to_string();
        let listed: Shared = Arc::new(Mutex::new(vec![
            approve("req-u1", "https://supabase.example/sql/new y listo"),
            approve("req-p1", &format!("{here} existe")),
            approve("req-p2", "/no/existe/cac-504.sql no está"),
            approve("req-r1", "/app/tasks?task=t1 es la tarea"),
        ]));
        let server = live_server(&listed, &Arc::new(Mutex::new(Vec::new())));
        let window = open_sized(&server, 2400., cx);
        show_list(window, "req-u1", cx);
        wait_until(cx, "el último pedido", |cx| on_screen(window, "request-summary-req-r1", cx));

        click_start(window, "request-summary-req-u1", cx);
        assert_eq!(cx.opened_url().as_deref(), Some("https://supabase.example/sql/new"));
        assert!(!OPENED.lock().unwrap().iter().any(|p| p.contains("supabase")), "una dirección no es una ruta");

        // Otras pruebas anotan sus rutas a la vez: se cuentan sólo las suyas.
        let mine = |wanted: &str| OPENED.lock().unwrap().iter().filter(|p| *p == wanted || p.starts_with("/no/existe")).count();
        click_start(window, "request-summary-req-p1", cx);
        wait_until(cx, "la ruta mostrada", |_| mine(&here) == 1);

        click_start(window, "request-summary-req-p2", cx);
        assert_eq!(mine(&here), 1, "una ruta que no existe no se abre");
        wait_until(cx, "el aviso de que no existe", |cx| label(window, "requests-said", cx).contains("/no/existe/cac-504.sql"));

        click_start(window, "request-summary-req-r1", cx);
        assert_eq!(cx.opened_url().as_deref(), Some("https://supabase.example/sql/new"), "la pantalla de la app no se le da al navegador");
        wait_until(cx, "la tarea se abre dentro", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.clone()).as_deref() == Some("t1"));
        stop(cx);
    }

    fn clipboard(cx: &mut TestAppContext) -> Option<String> {
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    /// Un «Ejecutar» con `apply.sql` y `restore.sql` adjuntos: cada uno con su
    /// botón «Copiar <nombre>» a la vista, el de aplicar junto al comando y el
    /// de deshacer bajo «Para deshacerlo», y cada botón copia el contenido
    /// exacto de su fichero (command-and-control-567). Uno largo sale plegado.
    ///
    /// Roturas que atrapa: un botón sin el nombre del fichero; copiar el otro
    /// fichero, o el contenido recortado o plegado; el de deshacer junto al
    /// comando; no decir qué se copió; enseñar entero uno largo.
    #[gpui_kit::test]
    fn the_attached_files_are_copied_whole_each_in_its_place(cx: &mut TestAppContext) {
        let apply: String = (1..=20).map(|n| format!("update pagos set moneda = 'USD' where id = {n};\n")).collect();
        let restore = "  update pagos set moneda = 'MXN';\n\n".to_string();
        let run = request("req-f1", "run", "t1", ("u1", "Ana Ruiz"), true, json!({
            "command": "Pegar apply.sql en el SQL Editor", "where": "Supabase", "expected": "UPDATE 20", "undo": "Pegar restore.sql",
            "files": [{ "name": "apply.sql", "content": apply, "role": "apply" }, { "name": "restore.sql", "content": restore, "role": "undo" }],
        }));
        let server = live_server(&Arc::new(Mutex::new(vec![run])), &Arc::new(Mutex::new(Vec::new())));
        let window = open_sized(&server, 2400., cx);
        show_list(window, "req-f1", cx);
        wait_until(cx, "los ficheros", |cx| on_screen(window, "request-file-req-f1-1-copy", cx));

        assert_eq!(label(window, "request-file-req-f1-0-copy", cx), "Copiar apply.sql");
        assert_eq!(label(window, "request-file-req-f1-1-copy", cx), "Copiar restore.sql");
        let at = |id: &str, cx: &mut TestAppContext| bounds(window, id, cx).unwrap_or_else(|| panic!("{id} no está"));
        let (command, expected, undo) = (at("request-command-req-f1", cx), at("request-expected-req-f1", cx), at("request-undo-req-f1", cx));
        let (apply_button, restore_button) = (at("request-file-req-f1-0-copy", cx), at("request-file-req-f1-1-copy", cx));
        assert!(apply_button.top() > command.bottom() && apply_button.bottom() < expected.top(), "apply.sql va junto al comando");
        assert!(restore_button.top() > undo.bottom(), "restore.sql va bajo «Para deshacerlo»");

        assert!(on_screen(window, "request-file-req-f1-0-content-line-11", cx));
        assert!(!on_screen(window, "request-file-req-f1-0-content-line-12", cx), "uno largo sale plegado");
        click(window, "request-file-req-f1-0-copy", cx);
        assert_eq!(clipboard(cx).as_deref(), Some(apply.as_str()), "se copia entero, aunque esté plegado");
        assert_eq!(label(window, "requests-said", cx), "Copiado apply.sql");
        click(window, "request-file-req-f1-1-copy", cx);
        assert_eq!(clipboard(cx).as_deref(), Some(restore.as_str()));
        assert_eq!(label(window, "requests-said", cx), "Copiado restore.sql");

        click(window, "request-file-req-f1-0-fold", cx);
        assert!(on_screen(window, "request-file-req-f1-0-content-line-19", cx), "desplegado se ve entero");
        stop(cx);
    }

    /// La lista de pedidos trae los adjuntos sin contenido (sólo nombre,
    /// papel y tamaño): «Copiar apply.sql» lo pide al detalle del pedido por
    /// su código y deja en el portapapeles el contenido exacto; «Ver el
    /// contenido» lo trae y lo enseña. Lo traído se guarda: otro copiar no
    /// vuelve a pedirlo (command-and-control-567).
    ///
    /// Roturas que atrapa: copiar lo que trae la lista (vacío) en vez de
    /// pedirlo; pedir otro pedido o copiar el otro fichero; no enseñar lo
    /// traído al desplegar; volver a pedir el detalle en cada clic.
    #[gpui_kit::test]
    fn a_file_listed_without_its_content_is_fetched_from_the_request_when_copied(cx: &mut TestAppContext) {
        let apply = "update pagos set moneda = 'USD' where id = 7;\n".to_string();
        let restore: String = (1..=20).map(|n| format!("update pagos set moneda = 'MXN' where id = {n};\n")).collect();
        let run = |code: &str| {
            let listed = request(code, "run", "t1", ("u1", "Ana Ruiz"), true, json!({
                "command": "Pegar apply.sql", "where": "Supabase", "expected": "UPDATE 1", "undo": "Pegar restore.sql",
                "files": [{ "name": "apply.sql", "role": "apply", "size": apply.len() }, { "name": "restore.sql", "role": "undo", "size": restore.len() }],
            }));
            let mut detail = listed.clone();
            detail["body"]["files"] = json!([{ "name": "apply.sql", "content": apply, "role": "apply", "size": apply.len() }, { "name": "restore.sql", "content": restore, "role": "undo", "size": restore.len() }]);
            (listed, detail)
        };
        let ((copied, copied_detail), (shown, shown_detail)) = (run("req-l1"), run("req-l2"));
        let server = live_server(&Arc::new(Mutex::new(vec![copied, shown])), &Arc::new(Mutex::new(Vec::new())));
        for (code, detail) in [("req-l1", copied_detail), ("req-l2", shown_detail)] {
            server.mount(
                Mock::given(method("GET"))
                    .and(path(format!("/api/v1/requests/{code}")))
                    .respond_with(move |_: &wiremock::Request| ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": detail.clone() }))),
            );
        }
        let fetched = |code: &str| server.requests().iter().filter(|r| r.method.as_str() == "GET" && r.url.path() == format!("/api/v1/requests/{code}")).count();
        let window = open_sized(&server, 2400., cx);
        show_list(window, "req-l1", cx);
        wait_until(cx, "los ficheros", |cx| on_screen(window, "request-file-req-l2-1-copy", cx));
        assert!(!on_screen(window, "request-file-req-l1-1-content-line-0", cx), "sin contenido no hay bloque");
        assert_eq!(fetched("req-l1") + fetched("req-l2"), 0, "la lista basta para pintar los botones");

        // Copiar sin haberlo visto: lo pide y copia el suyo, no el otro.
        click(window, "request-file-req-l1-0-copy", cx);
        wait_until(cx, "lo copiado", |cx| label(window, "requests-said", cx) == "Copiado apply.sql");
        assert_eq!(clipboard(cx).as_deref(), Some(apply.as_str()));
        click(window, "request-file-req-l1-1-copy", cx);
        wait_until(cx, "lo copiado", |cx| label(window, "requests-said", cx) == "Copiado restore.sql");
        assert_eq!(clipboard(cx).as_deref(), Some(restore.as_str()));
        assert_eq!(fetched("req-l1"), 1, "lo traído se guarda");

        // «Ver el contenido» lo trae y lo enseña entero.
        click(window, "request-file-req-l2-1-fold", cx);
        wait_until(cx, "el contenido traído", |cx| on_screen(window, "request-file-req-l2-1-content-line-0", cx));
        assert!(on_screen(window, "request-file-req-l2-1-content-line-19", cx), "lo que se pidió ver se ve entero");
        stop(cx);
    }

    /// Si el pedido se rehízo entre la lista y el detalle, lo traído es de la
    /// versión nueva y se guarda con ella, no con la de la tarjeta: copiar
    /// otra vez desde la tarjeta vieja lo vuelve a pedir, y la lista nueva lo
    /// encuentra (command-and-control-567).
    ///
    /// Rotura que atrapa: guardar el contenido nuevo con la versión que tenía
    /// la tarjeta, con lo que la tarjeta vieja copiaría lo de otra versión sin
    /// preguntar.
    #[gpui_kit::test]
    fn a_file_fetched_from_a_newer_version_is_kept_for_that_version(cx: &mut TestAppContext) {
        let listed = request("req-v1", "run", "t1", ("u1", "Ana Ruiz"), true, json!({
            "command": "Pegar apply.sql", "where": "Supabase", "expected": "UPDATE 1",
            "files": [{ "name": "apply.sql", "role": "apply", "size": 10 }],
        }));
        let newer = "update pagos set moneda = 'EUR';\n";
        let mut detail = listed.clone();
        detail["version"] = json!(2);
        detail["body"]["files"] = json!([{ "name": "apply.sql", "content": newer, "role": "apply", "size": newer.len() }]);
        let server = live_server(&Arc::new(Mutex::new(vec![listed])), &Arc::new(Mutex::new(Vec::new())));
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/requests/req-v1"))
                .respond_with(move |_: &wiremock::Request| ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": detail.clone() }))),
        );
        let fetched = || server.requests().iter().filter(|r| r.method.as_str() == "GET" && r.url.path() == "/api/v1/requests/req-v1").count();
        let window = open_sized(&server, 2400., cx);
        show_list(window, "req-v1", cx);
        wait_until(cx, "el fichero", |cx| on_screen(window, "request-file-req-v1-0-copy", cx));
        click(window, "request-file-req-v1-0-copy", cx);
        wait_until(cx, "lo copiado", |cx| label(window, "requests-said", cx) == "Copiado apply.sql");
        assert_eq!(clipboard(cx).as_deref(), Some(newer));
        click(window, "request-file-req-v1-0-copy", cx);
        wait_until(cx, "lo pedido otra vez", |_| fetched() == 2);
        stop(cx);
    }

    /// Un pedido que nombra un `.sql` de esta máquina por su ruta lleva
    /// «Copiar apply.sql», que copia su contenido exacto; pulsar la ruta lo
    /// abre con su programa y «Mostrar en la carpeta» lo muestra. Un `.env` y
    /// un enlace que sale de su carpeta no ofrecen copiar y dicen por qué
    /// (command-and-control-567).
    ///
    /// Roturas que atrapa: no ofrecer copiar una ruta; copiar otra cosa que
    /// el contenido; ofrecer copiar un secreto o lo que apunta un enlace que
    /// sale de su carpeta; callar el motivo; revelar el fichero en vez de
    /// abrirlo al pulsarlo; «Mostrar en la carpeta» que no lo muestra.
    #[gpui_kit::test]
    fn a_path_named_in_a_request_copies_its_content_and_opens_with_its_program(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("cac-567-paths-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("cac-567-outside-{}", std::process::id()));
        for d in [&dir, &outside] {
            let _ = std::fs::remove_dir_all(d);
            std::fs::create_dir_all(d).unwrap();
        }
        let sql = "update pagos set moneda = 'USD';\n-- fin\n";
        std::fs::write(dir.join("apply.sql"), sql).unwrap();
        std::fs::write(dir.join(".env"), "DATABASE_URL=postgres://x").unwrap();
        std::fs::write(outside.join("notes.txt"), "fuera").unwrap();
        std::os::unix::fs::symlink(outside.join("notes.txt"), dir.join("link.sql")).unwrap();
        let path = |name: &str| dir.join(name).to_string_lossy().to_string();
        let run = request("req-p1", "run", "t1", ("u1", "Ana Ruiz"), true, json!({
            "command": format!("{} en el SQL Editor", path("apply.sql")), "where": path(".env"), "expected": "UPDATE 1", "undo": path("link.sql"),
        }));
        let server = live_server(&Arc::new(Mutex::new(vec![run])), &Arc::new(Mutex::new(Vec::new())));
        let window = open_sized(&server, 2400., cx);
        show_list(window, "req-p1", cx);
        wait_until(cx, "el botón de copiar", |cx| on_screen(window, "request-command-req-p1-copy-path-0", cx));
        wait_until(cx, "el motivo del enlace", |cx| on_screen(window, "request-undo-req-p1-cannot-copy-0", cx));
        wait_until(cx, "el motivo del .env", |cx| on_screen(window, "request-where-req-p1-cannot-copy-0", cx));

        assert_eq!(label(window, "request-command-req-p1-copy-path-0", cx), "Copiar apply.sql");
        click(window, "request-command-req-p1-copy-path-0", cx);
        wait_until(cx, "lo copiado", |cx| label(window, "requests-said", cx) == "Copiado apply.sql");
        assert_eq!(clipboard(cx).as_deref(), Some(sql));

        assert!(!on_screen(window, "request-where-req-p1-copy-path-0", cx), "un .env no se ofrece");
        assert!(label(window, "request-where-req-p1-cannot-copy-0", cx).contains("parece un secreto"), "{}", label(window, "request-where-req-p1-cannot-copy-0", cx));
        assert!(!on_screen(window, "request-undo-req-p1-copy-path-0", cx), "un enlace que sale de su carpeta no se ofrece");
        assert!(label(window, "request-undo-req-p1-cannot-copy-0", cx).contains("fuera de su carpeta"));

        // Otras pruebas anotan sus rutas a la vez: se miran sólo las de aquí.
        let here = dir.to_string_lossy().to_string();
        let ours = |list: &Mutex<Vec<String>>| list.lock().unwrap().iter().filter(|p| p.starts_with(&here)).cloned().collect::<Vec<_>>();
        click_start(window, "request-command-req-p1-line-0", cx);
        wait_until(cx, "el fichero abierto", |_| !ours(&OPENED_FILES).is_empty());
        assert_eq!(ours(&OPENED_FILES), [path("apply.sql")]);
        assert!(ours(&OPENED).is_empty(), "pulsar la ruta lo abre, no lo revela");
        click(window, "request-command-req-p1-reveal-path-0", cx);
        wait_until(cx, "la carpeta", |_| !ours(&OPENED).is_empty());
        assert_eq!(ours(&OPENED), [path("apply.sql")]);
        stop(cx);
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    /// Arrastra el ratón de un extremo a otro de un texto y pulsa Ctrl+C.
    fn select_and_copy(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> Option<String> {
        let found = bounds(window, id, cx).unwrap_or_else(|| panic!("{id} no está en pantalla"));
        let y = found.top() + px(8.);
        let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
        visual.simulate_mouse_down(point(found.left() + px(1.), y), gpui_kit::MouseButton::Left, gpui_kit::Modifiers::default());
        visual.simulate_mouse_move(point(found.right() - px(1.), y), Some(gpui_kit::MouseButton::Left), gpui_kit::Modifiers::default());
        visual.simulate_mouse_up(point(found.right() - px(1.), y), gpui_kit::MouseButton::Left, gpui_kit::Modifiers::default());
        visual.simulate_keystrokes("ctrl-c");
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    /// El texto de un pedido se selecciona con el ratón y se copia con Ctrl+C
    /// (también el que lleva enlaces, tal como está escrito), y el borrador
    /// tiene su botón «Copiar» (command-and-control-543).
    ///
    /// Roturas que atrapa: texto del pedido que no deja seleccionar; un texto
    /// con enlaces que se copia con la marca del markdown (`\_`, `[…](…)`) en
    /// vez de lo escrito; el botón de copiar el borrador que no copia.
    #[gpui_kit::test]
    fn the_text_of_a_request_can_be_selected_and_copied(cx: &mut TestAppContext) {
        let listed: Shared = Arc::new(Mutex::new(vec![
            approve("req-u1", "1. ver https://supabase.example/sql/new_x y *listo* #1 (ok)."),
            approve("req-d1", "resumen llano"),
        ]));
        let server = live_server(&listed, &Arc::new(Mutex::new(Vec::new())));
        let window = open_sized(&server, 2400., cx);
        show_list(window, "req-u1", cx);
        wait_until(cx, "los pedidos", |cx| on_screen(window, "request-draft-req-d1", cx));

        assert_eq!(select_and_copy(window, "request-summary-req-d1", cx).as_deref(), Some("resumen llano"));
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(String::new()));
        assert_eq!(select_and_copy(window, "request-summary-req-u1", cx).as_deref(), Some("1. ver https://supabase.example/sql/new_x y *listo* #1 (ok)."));
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(String::new()));
        assert_eq!(select_and_copy(window, "request-draft-req-d1-line-0", cx).as_deref(), Some("Hola"));

        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(String::new()));
        click(window, "request-copy-draft-req-d1", cx);
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("Hola"));
        stop(cx);
    }

    /// Las opciones de un «decide» se seleccionan y se copian sin elegir; sólo
    /// «Elegir» contesta. Lo mismo con la línea del resultado, y una línea de
    /// comando sangrada que lleva un enlace conserva su sangría
    /// (command-and-control-543).
    ///
    /// Roturas que atrapa: el texto de la opción dentro del área de clic (soltar
    /// el arrastre elige); «Elegir» que no contesta; la línea del resultado que no
    /// se deja seleccionar; la sangría que se pierde al pasar por el markdown.
    #[gpui_kit::test]
    fn an_option_is_copied_without_being_chosen_and_choose_answers(cx: &mut TestAppContext) {
        let mut decide = request("req-o1", "decide", "t1", ("u1", "Ana Ruiz"), true, json!({ "question": "¿Cuál?", "options": [{ "label": "Opción A", "consequence": "rápido", "recommended": true }] }));
        decide["summary"] = json!("elige");
        let mut flat = approve("req-i0", "plano");
        flat["body"]["draft"] = json!("sin sangría https://x.example");
        let mut indented = approve("req-i1", "con sangría");
        indented["body"]["draft"] = json!("    con sangría https://x.example");
        let listed: Shared = Arc::new(Mutex::new(vec![decide, flat, indented]));
        let closed: Shared = Arc::new(Mutex::new(Vec::new()));
        let server = live_server(&listed, &closed);
        let mut done = request("req-o1", "decide", "t1", ("u1", "Ana Ruiz"), true, json!({ "question": "¿Cuál?", "options": [{ "label": "Opción A", "consequence": "rápido" }] }));
        done["status"] = json!("answered");
        done["answer"] = json!({ "option": "Opción A" });
        done["answeredBy"] = json!({ "id": "u1", "name": "Ana Ruiz" });
        done["answeredAt"] = json!("2026-11-24T11:00:00Z");
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-o1/answer")).respond_with(ok(done.clone())));
        let window = open_sized(&server, 2400., cx);
        show_list(window, "req-o1", cx);
        wait_until(cx, "los pedidos", |cx| on_screen(window, "request-draft-req-i1", cx));

        assert_eq!(select_and_copy(window, "request-option-label-req-o1-0", cx).as_deref(), Some("Opción A"));
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(String::new()));
        assert_eq!(select_and_copy(window, "request-option-consequence-req-o1-0", cx).as_deref(), Some("rápido"));
        assert!(answers(&server, "req-o1").is_empty(), "seleccionar el texto no elige la opción");

        let left = |id: &str, cx: &mut TestAppContext| f32::from(bounds(window, id, cx).unwrap().left());
        let (plain, shifted) = (left("request-draft-req-i0-line-0", cx), left("request-draft-req-i1-line-0", cx));
        assert!(shifted - plain > 20., "la sangría se conserva: {plain} contra {shifted}");

        click(window, "request-option-req-o1-0", cx);
        wait_until(cx, "la opción elegida", |_| answers(&server, "req-o1").len() == 1);
        assert_eq!(answers(&server, "req-o1")[0]["option"], "Opción A");
        listed.lock().unwrap().retain(|request| request["code"] != "req-o1");
        *closed.lock().unwrap() = vec![done];
        wait_until(cx, "pasa a «Resueltos»", |cx| on_screen(window, "requests-resolved", cx) && !on_screen(window, "request-req-o1", cx));
        click(window, "requests-resolved", cx);
        wait_until(cx, "el resultado", |cx| on_screen(window, "request-outcome-req-o1", cx));
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(String::new()));
        assert!(select_and_copy(window, "request-outcome-req-o1", cx).is_some_and(|text| text.contains("Opción A")));
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(String::new()));
        assert!(select_and_copy(window, "request-outcome-by-req-o1", cx).is_some_and(|text| text.contains("Ana Ruiz")));
        stop(cx);
    }

    /// Lo que entra por encima no mueve lo que se lee: con la lista deslizada,
    /// un pedido nuevo (que ordena primero) deja la primera tarjeta visible
    /// donde estaba.
    ///
    /// Rotura que atrapa: no corregir el desplazamiento tras pintar la lista
    /// nueva (todo baja lo que mide el pedido que entró).
    #[gpui_kit::test]
    fn a_request_arriving_above_does_not_move_what_is_read(cx: &mut TestAppContext) {
        let codes = ["req-b1", "req-b2", "req-b3", "req-b4", "req-b5", "req-b6"];
        let listed: Shared = Arc::new(Mutex::new(codes.iter().map(|code| approve(code, "uno")).collect()));
        let server = live_server(&listed, &Arc::new(Mutex::new(Vec::new())));
        let window = open_sized(&server, 560., cx);
        show_list(window, "req-b1", cx);
        scroll_down(window, 700., cx);
        let (code, was) = first_visible(window, &codes, cx);
        assert_ne!(code, "req-b1", "la lista tenía que estar deslizada");

        listed.lock().unwrap().insert(0, approve("req-b0", "nuevo"));
        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.stream().deliver(cac_core::ServerEvent { event: "request:created".into(), data: json!({ "code": "req-b0" }).to_string() });
            })
        });
        wait_until(cx, "el nuevo en la lista", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).requests.open.len()) == 7);
        wait_until(cx, "la tarjeta de antes, donde estaba", |cx| bounds(window, &format!("request-{code}"), cx).is_some_and(|now| (f32::from(now.top()) - was).abs() < 1.));
        stop(cx);
    }

    /// Contestar el pedido resaltado no abre «Resueltos»: sólo se abre si el
    /// enlace llegó cuando ya estaba resuelto.
    ///
    /// Rotura que atrapa: abrir «Resueltos» cada vez que el pedido resaltado
    /// cae en resueltos, que es lo que desplegaba la lista bajo quien estaba
    /// contestando.
    #[gpui_kit::test]
    fn answering_the_highlighted_request_does_not_open_resolved(cx: &mut TestAppContext) {
        let listed: Shared = Arc::new(Mutex::new(vec![approve("req-a1", "uno"), approve("req-a2", "dos")]));
        let closed: Shared = Arc::new(Mutex::new(Vec::new()));
        let server = live_server(&listed, &closed);
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a1/opened")).respond_with(ok(json!(null))));
        let mut done = approve("req-a1", "uno");
        done["status"] = json!("answered");
        done["answer"] = json!({ "approved": true });
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a1/answer")).respond_with(ok(done.clone())));
        let window = open(&server, cx);
        wait_until(cx, "las tareas", |cx| on_screen(window, "work-row-t1", cx));
        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.stream().deliver(cac_core::ServerEvent { event: "request:open".into(), data: json!({ "code": "req-a1", "nonce": "n1" }).to_string() });
            })
        });
        wait_until(cx, "el pedido resaltado", |cx| on_screen(window, "request-req-a1", cx));

        *listed.lock().unwrap() = vec![approve("req-a2", "dos")];
        *closed.lock().unwrap() = vec![done];
        click(window, "request-approve-req-a1", cx);
        wait_until(cx, "pasa a «Resueltos»", |cx| on_screen(window, "requests-resolved", cx) && !on_screen(window, "request-req-a1", cx));
        for _ in 0..5 {
            cx.run_until_parked();
            assert!(!on_screen(window, "request-outcome-req-a1", cx), "«Resueltos» no se abre solo");
        }
        stop(cx);
    }

    fn show_list(window: AnyWindowHandle, first: &str, cx: &mut TestAppContext) {
        wait_until(cx, "el número de «Por atender»", |cx| on_screen(window, "panel-requests-count", cx));
        click(window, "panel-requests", cx);
        let wanted = format!("request-{first}");
        wait_until(cx, "los pedidos", |cx| on_screen(window, &wanted, cx));
    }

    fn scroll_down(window: AnyWindowHandle, by: f32, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.scroll("requests", ScrollDelta::Pixels(point(px(0.), px(-by))), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// La primera tarjeta que aún se ve, y a qué altura de la ventana.
    fn first_visible(window: AnyWindowHandle, codes: &[&str], cx: &mut TestAppContext) -> (String, f32) {
        let area = bounds(window, "requests", cx).expect("la lista");
        for code in codes {
            if let Some(card) = bounds(window, &format!("request-{code}"), cx) {
                if card.bottom() > area.top() {
                    return (code.to_string(), f32::from(card.top()));
                }
            }
        }
        panic!("no se ve ninguna tarjeta");
    }

    fn resolved(code: &str) -> Value {
        let mut value = approve(code, &format!("resuelto {code}"));
        value["status"] = json!("answered");
        value["canAnswer"] = json!(false);
        value["answer"] = json!({ "approved": true });
        value
    }

    /// Las consultas a `/api/v1/requests` de ese estado, como pares.
    fn asked(server: &FakeServer, status: &str) -> Vec<HashMap<String, String>> {
        server
            .requests()
            .iter()
            .filter(|request| request.url.path() == "/api/v1/requests")
            .map(|request| request.url.query_pairs().into_owned().collect::<HashMap<String, String>>())
            .filter(|query| query.get("status").map(String::as_str) == Some(status))
            .collect()
    }

    fn closed_page(server: &FakeServer, cursor: &str, template: ResponseTemplate) {
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/requests"))
                .and(query_param("scope", "mine"))
                .and(query_param("status", "closed"))
                .and(query_param("cursor", cursor.to_string()))
                .respond_with(template)
                .with_priority(1),
        );
    }

    fn page(requests: Vec<Value>, next: &str) -> ResponseTemplate {
        ok(json!({ "requests": requests, "next": next }))
    }

    /// «Resueltos» plegado no pide nada; desplegarlo pide la primera página, y
    /// llevar el scroll al fondo pide la siguiente una sola vez, con su
    /// cursor. Si falla se dice y se reintenta a mano; cuando no hay más, no
    /// se pide más. Y no hay número: lo cargado no es cuántos hay.
    ///
    /// Roturas que atrapa: pedir los resueltos al abrir; no pedir la
    /// siguiente al llegar al fondo; pedirla sin llegar; pedirla dos veces
    /// mientras se desliza; reintentar sola tras un fallo (un bucle contra un
    /// servidor caído); no ofrecer «Reintentar»; seguir pidiendo sin más.
    #[gpui_kit::test]
    fn scrolling_to_the_end_of_resolved_asks_the_next_page_once(cx: &mut TestAppContext) {
        let server = server();
        let first: Vec<Value> = (0..20).map(|n| resolved(&format!("req-c{n:02}"))).collect();
        closed_page(&server, "", page(first, "CUR-1"));
        let gate = Gate::default();
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/requests"))
                .and(query_param("status", "closed"))
                .and(query_param("cursor", "CUR-1"))
                .respond_with(gate.hold(ResponseTemplate::new(500)))
                .up_to_n_times(1)
                .with_priority(1),
        );
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/requests"))
                .and(query_param("status", "closed"))
                .and(query_param("cursor", "CUR-1"))
                .respond_with(page(vec![resolved("req-c15"), resolved("req-c16")], ""))
                .with_priority(2),
        );
        let window = open_sized(&server, 1000., cx);
        show_requests(window, cx);
        assert!(asked(&server, "closed").is_empty(), "plegado no se pide");
        assert_eq!(label(window, "requests-resolved", cx), "Resueltos");

        click(window, "requests-resolved", cx);
        wait_until(cx, "la primera página", |cx| on_screen(window, "request-req-c00", cx));
        assert_eq!(asked(&server, "closed").len(), 1);
        for _ in 0..3 {
            cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
            cx.run_until_parked();
        }
        assert_eq!(asked(&server, "closed").len(), 1, "sin llegar al fondo no se pide la siguiente");

        for _ in 0..4 {
            scroll_down(window, 4000., cx);
        }
        wait_until(cx, "la siguiente, retenida", |_| gate.held() == 1);
        assert!(on_screen(window, "requests-closed-end-loading", cx), "abajo dice que está cargando");
        for _ in 0..3 {
            scroll_down(window, 4000., cx);
        }
        gate.open();
        wait_until(cx, "el fallo, dicho", |cx| on_screen(window, "requests-closed-end-error", cx));
        for _ in 0..3 {
            scroll_down(window, 4000., cx);
        }
        let next = |server: &FakeServer| asked(server, "closed").iter().filter(|query| query["cursor"] == "CUR-1").count();
        assert_eq!(next(&server), 1, "una sola petición, y tras el fallo no se reintenta sola");

        click(window, "requests-closed-end-retry", cx);
        wait_until(cx, "la segunda página", |cx| on_screen(window, "request-req-c16", cx));
        for _ in 0..3 {
            scroll_down(window, 4000., cx);
        }
        assert_eq!(next(&server), 2);
        assert_eq!(asked(&server, "closed").len(), 3, "sin más, no se pide más");
        assert!(!on_screen(window, "requests-closed-end-error", cx));
        stop(cx);
    }

    /// Un enlace a un pedido resuelto que no está en lo cargado lo abre: se
    /// pide suelto, se despliega «Resueltos» y se enseña cómo quedó.
    ///
    /// Rotura que atrapa: que la pantalla no lo encuentre (sólo buscaba en lo
    /// que ya tenía).
    #[gpui_kit::test]
    fn a_link_to_a_resolved_request_outside_the_page_opens_it(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/requests/req-old")).respond_with(ok(resolved("req-old"))));
        let window = open(&server, cx);
        wait_until(cx, "las tareas", |cx| on_screen(window, "work-row-t1", cx));
        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.open_link("cac://requests?open=req-old").await;
            })
        });
        wait_until(cx, "el pedido resuelto", |cx| on_screen(window, "request-outcome-req-old", cx));
        stop(cx);
    }

    /// La búsqueda va al servidor un rato después de dejar de escribir, y los
    /// chips de tipo y estado también; sin resultados se dice.
    ///
    /// Roturas que atrapa: buscar en local; mandar una petición por tecla;
    /// no mandar el tipo elegido; que filtrar por resueltos siga pidiendo
    /// abiertos; decir «nada te espera» de un filtro sin resultados.
    #[gpui_kit::test]
    fn filters_are_asked_to_the_server(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        show_requests(window, cx);
        let opens = asked(&server, "open").iter().filter(|query| query["scope"] == "mine").count();

        type_in(window, "requests-search", "req-a", cx);
        cx.run_until_parked();
        assert!(asked(&server, "open").iter().all(|query| !query.contains_key("q")), "no con cada tecla");
        cx.executor().advance_clock(std::time::Duration::from_millis(400));
        wait_until(cx, "la búsqueda, en el servidor", |_| asked(&server, "open").iter().any(|query| query.get("q").map(String::as_str) == Some("req-a")));
        assert_eq!(asked(&server, "open").iter().filter(|query| query.contains_key("q")).count(), 1, "una sola petición");

        click(window, "requests-kind-decide", cx);
        wait_until(cx, "el tipo", |_| asked(&server, "open").iter().any(|query| query.get("kind").map(String::as_str) == Some("decide") && query["q"] == "req-a"));

        let before = asked(&server, "open").len();
        click(window, "requests-status-resolved", cx);
        wait_until(cx, "los resueltos, filtrados", |_| asked(&server, "closed").iter().any(|query| query.get("kind").map(String::as_str) == Some("decide")));
        assert_eq!(asked(&server, "open").len(), before, "filtrando por resueltos no se piden abiertos");
        wait_until(cx, "sin resultados", |cx| on_screen(window, "requests-empty", cx));
        assert_eq!(label(window, "requests-empty", cx), "Ningún pedido coincide con el filtro.");
        assert!(opens >= 1);
        stop(cx);
    }

    /// Cada tecla reinicia la espera: teclear, esperar menos que ella y teclear
    /// otra vez no manda nada hasta que pasa la espera de la última tecla, y
    /// entonces una sola petición, la del texto final.
    ///
    /// Roturas que atrapa: quitar la guarda del turno en `search_soon`, de modo
    /// que cada espera vencida mande su petición.
    #[gpui_kit::test]
    fn each_key_restarts_the_search_wait(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        show_requests(window, cx);

        type_in(window, "requests-search", "req-a", cx);
        cx.run_until_parked();
        cx.executor().advance_clock(std::time::Duration::from_millis(200));
        type_in(window, "requests-search", "bc", cx);
        cx.run_until_parked();
        cx.executor().advance_clock(std::time::Duration::from_millis(150));
        cx.run_until_parked();
        // El filtro llega al core por su propio hilo: se le da un rato de pared
        // para que, si la espera vencida lo mandara, se vea. Por el lado seguro:
        // un equipo lento sólo deja pasar la rotura, nunca falla de más.
        cx.executor().allow_parking();
        for _ in 0..30 {
            cx.run_until_parked();
            cx.update(|cx| assert_eq!(Backend::snapshot(cx).read(cx).requests.filter.text, "", "la espera de la primera tecla ya venció, pero la segunda la reinició"));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        cx.executor().advance_clock(std::time::Duration::from_millis(400));
        wait_until(cx, "la búsqueda, en el servidor", |_| asked(&server, "open").iter().any(|query| query.contains_key("q")));
        cx.run_until_parked();
        let searches: Vec<String> = asked(&server, "open").iter().filter_map(|query| query.get("q").cloned()).collect();
        assert_eq!(searches, vec!["req-abc".to_string()], "una sola petición, con el texto final");
        stop(cx);
    }
}

