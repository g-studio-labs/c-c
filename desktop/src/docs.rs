//! La documentación de un nodo: sus cuatro pestañas (la portada), en la de
//! decisiones el registro vigente, y las páginas que cuelgan de ella. Se lee
//! y se escribe.
//!
//! Qué documento hay abierto, qué pestaña o qué página, qué se lee, qué
//! parte es prosa, cómo es el árbol de páginas y por qué está vacío lo dice
//! el core. Aquí sólo se pinta, con las medidas de
//! `app/src/components/docs/DocTabs.tsx`, `DocPagesSheet.tsx`,
//! `DocPagesBranch.tsx`, `PageRow.tsx` y `DocPageView.tsx`.
//!
//! Escribir es el editor de markdown de la app (`editor.rs`) a todo el alto,
//! con el documento ya pintado al lado, que cambia con cada tecla; en una
//! ventana estrecha se ve uno u otro. No es el tiptap de la app de siempre:
//! se escribe el markdown tal cual (decidido con jose, 18-nov-2026). El
//! borrador, si hay algo sin guardar, qué se manda, qué pasa cuando alguien
//! guardó en medio y qué se pregunta antes de perder texto lo dice el core
//! (`cac_core::doc_edit`): aquí están las cajas, que le cuentan cada tecla.

use cac_core::task::Question;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use cac_core::{
    DocBacklinkRow, DocDecision, DocEdit, DocEditWhat, DocEntryPlace, DocHeading, DocPageLink, DocPageRow, DocPiece, DocPieceEnd, DocsState, WorkLayout, DOC_TABS,
};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter, InputEvent, Textarea, TextareaState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::doc_share::{PageShare, PageShareClosed};
use crate::editor::MarkdownEditor;
use crate::theme::Leading as _;
use crate::lang::{locale, tr};

/// La medida de lectura: las 68 letras (`max-w-[68ch]`) de `.prose-doc`, a
/// los 15 px de su texto. Es para la prosa; una tabla o un bloque de código
/// usan el ancho del panel (ver `CLAUDE.md`).
const READING_MEASURE: f32 = 640.;
/// El ancho de lo que se lee (`max-w-4xl`), y el de una ventana grande
/// (`xl:max-w-5xl`), que además lleva el índice al lado.
const PAGE_WIDTH: f32 = 896.;
const WIDE_PAGE_WIDTH: f32 = 1024.;
/// Desde qué ancho de ventana es «grande» (`xl`) y desde cuál caben las
/// pistas de las pestañas (`md`).
const WIDE_WINDOW: f32 = 1280.;
const HINTS_FROM: f32 = 768.;
/// El índice «En esta página» (`w-54`).
const CONTENTS_WIDTH: f32 = 216.;

/// Las vistas de una lista, en el orden de `ViewSwitch.tsx`. La de
/// documentación es en la que se está.
const VIEWS: [(&str, Option<WorkLayout>); 4] =
    [("board", Some(WorkLayout::Board)), ("list", Some(WorkLayout::List)), ("calendar", Some(WorkLayout::Calendar)), ("docs", None)];

/// La hoja «Páginas» (`w-80`).
const PAGES_SHEET_WIDTH: f32 = 320.;
/// Las plantillas de una página vacía, con su icono (`PLANTILLAS` en
/// `DocPageView.tsx`).
const PAGE_TEMPLATES: [(&str, Lucide); 2] = [("component", Lucide::Puzzle), ("procedure", Lucide::ClipboardList)];

/// El aire alrededor de lo que se lee (`p-6`).
const READING_PAD: f32 = 24.;
/// El hueco entre un trozo de prosa y uno ancho (`gap-3`).
const PIECE_GAP: f32 = 12.;
/// El tamaño del texto que se lee (`text-[15px]` de `.prose-doc .md-body`).
const READING_SIZE: f32 = 15.;

/// A qué tamaño se pinta un título de lo que se lee, por su nivel:
/// `text-lg`, y los 17 y 13 px de `.prose-doc .md-body h2` y `h3`. El de
/// tercero se le da al pintor como de sexto (`cac_core::prose_for_painting`:
/// es como sale apagado y en mayúsculas), así que el sexto mide lo mismo.
fn heading_size(level: u8) -> f32 {
    match level {
        1 => 18.,
        2 => 17.,
        3 | 6 => 13.,
        _ => READING_SIZE,
    }
}

/// El aire que lleva encima un trozo de lo que se lee.
///
/// Un trozo que empieza por un título venía pintado dentro del mismo texto
/// que el párrafo de antes, y el aire entre los dos lo ponía el pintor de
/// markdown de GPUI Kit. Ahora cada título empieza su trozo (así el índice
/// salta a él y sólo se maqueta lo que cabe), y ese aire se pone aquí **con
/// la misma cuenta**, para que se vea igual: lo que mide el título por lo
/// que pide su nivel (1,1 un `#`; 1,6 un `##`; 1,4 los demás), y tras una
/// raya nunca menos que el aire de la raya (1,6 veces el texto). Dos
/// títulos seguidos van juntos: 0,35 del de arriba más 0,25 del de abajo.
/// Los números son los de `heading_metrics` y `HEADING_BOTTOM_GAP` de
/// `gpui-base` 0.9.1 (`text/node.rs`): si al subir de versión cambian, el
/// salto entre secciones se nota aquí.
/// → Guardián: `split_at_its_headings_the_text_takes_the_room_it_took_whole`.
///
/// (Allí tampoco baja del hueco de un párrafo, 0,8 rem; con estos tamaños
/// ningún título pide menos, así que aquí no se mira.)
fn piece_lead(before: Option<&DocPiece>, piece: &DocPiece) -> f32 {
    let Some(before) = before else { return 0. };
    let Some(level) = piece.heading.filter(|_| !before.wide && !piece.wide) else { return PIECE_GAP };
    let size = heading_size(level);
    let wanted = size
        * match level {
            1 => 1.1,
            2 => 1.6,
            _ => 1.4,
        };
    match before.end {
        DocPieceEnd::Heading => heading_size(before.heading.unwrap_or(level)) * 0.35 + size * 0.25,
        DocPieceEnd::Rule => wanted.max(READING_SIZE * 1.6),
        DocPieceEnd::Text => wanted,
    }
}

/// Por dónde va la lectura de una pestaña o de una página: su lista, que
/// guarda lo que se ha bajado y sólo maqueta las filas que caben.
struct Reading {
    list: ListState,
    /// De qué texto y de cuántas filas es lo que la lista tiene medido.
    stamp: u64,
    rows: usize,
}

/// Una fila de lo que se lee.
#[derive(Clone, Copy)]
enum ReadingRow {
    /// Las migas y el título de una página.
    PageHead,
    /// Por qué no hay nada que leer.
    Nothing,
    Piece(usize),
    Decision(usize),
    /// Lo que cuelga de una página y lo que la enlaza.
    PageTail,
}

/// Lo que se lee, tal como lo pide la lista: fila a fila, cuando le toca
/// pintarse.
struct ReadingRows {
    rows: Vec<ReadingRow>,
    pieces: Vec<DocPiece>,
    /// El aire sobre cada trozo (`piece_lead`).
    leads: Vec<f32>,
    decisions: Vec<DocDecision>,
    node: String,
    name: String,
    /// De qué pestaña o página es el texto: el de una no le presta su
    /// estado al de la otra.
    key: String,
    empty: Option<&'static str>,
    editable: bool,
    can_add: bool,
    /// De una página: sus migas, su título y, cuando ya llegó, lo que
    /// cuelga de ella y lo que la enlaza.
    trail: Vec<DocPageLink>,
    title: Option<String>,
    tail: Option<(String, Vec<DocPageLink>, Vec<DocBacklinkRow>)>,
    wide: bool,
    /// Si a la derecha va el índice: la fila le deja su sitio.
    with_contents: bool,
}

impl ReadingRows {
    /// El aire de arriba de una fila antes de lo que se lee en ella: a eso
    /// hay que bajar para dejar su título arriba del todo.
    fn lead_of(&self, row: usize) -> f32 {
        let first = if row == 0 { READING_PAD } else { 0. };
        match self.rows.get(row) {
            Some(ReadingRow::Piece(at)) => first + self.leads[*at],
            _ => first,
        }
    }

    fn row_of_piece(&self, piece: usize) -> Option<usize> {
        self.rows.iter().position(|row| matches!(row, ReadingRow::Piece(at) if *at == piece))
    }

    /// La marca junto al título de una sección que sostiene alguna decisión
    /// (`SectionDecisionsMark.tsx`): cuántas y, al abrirla, cuáles; cada una
    /// lleva a su registro, que es donde está el porqué. Va tras el título,
    /// así que hay que saber cuánto mide: se mide con su letra.
    fn render_decisions_mark(&self, at: usize, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let piece = &self.pieces[at];
        let level = piece.heading.filter(|_| !piece.decisions.is_empty())?;
        let theme = cx.theme();
        let (muted, border, soft) = (theme.muted_foreground, theme.border, theme.muted);
        let said = cac_core::text_count(locale(cx), "work:decisions.sectionMark", piece.decisions.len() as i64);

        let title = piece.text.lines().next().unwrap_or("").trim_start_matches('#').replace(['*', '_', '`'], "").trim().to_string();
        // El de tercer nivel se pinta en mayúsculas: se mide como se pinta.
        let title: SharedString = if level == 3 { title.to_uppercase() } else { title }.into();
        let size = heading_size(level);
        let mut font = window.text_style().font();
        font.weight = FontWeight::SEMIBOLD;
        let run = TextRun { len: title.len(), font, color: muted, background_color: None, underline: None, strikethrough: None };
        let width = f32::from(window.text_system().shape_line(title, px(size), &[run], None).width);
        // `ml-2`, y centrada en el renglón del título, dos píxeles arriba.
        const MARK_HEIGHT: f32 = 20.;
        let left = (width + 8.).min(READING_MEASURE - 44.);
        let top = (size * 1.35 - MARK_HEIGHT) / 2. - 2.;

        let trigger = Button::new(SharedString::from(format!("doc-section-decisions-{at}")))
            .ghost()
            .h(px(MARK_HEIGHT))
            .px_1p5()
            .rounded_full()
            .border_1()
            .border_color(border)
            .text_size(px(11.))
            .line_height(px(16.))
            .font_weight(FontWeight::NORMAL)
            .text_color(muted)
            .tooltip(said.clone())
            .child(div().flex().items_center().gap_0p5().child(Icon::new(Lucide::Gavel).size_3()).child(piece.decisions.len().to_string()));
        let (decisions, heading) = (piece.decisions.clone(), said.clone());
        let popover = Popover::new(SharedString::from(format!("doc-section-decisions-list-{at}"))).trigger(trigger).content(move |_, _, _| {
            div()
                .w(px(288.))
                .flex()
                .flex_col()
                .child(div().px_2().py_1().leading_xs().text_color(muted).child(heading.clone()))
                .children(decisions.iter().enumerate().map(|(index, decision)| {
                    let link = cac_core::decision_record_link(decision);
                    div()
                        .id(("doc-section-decision", index))
                        .w_full()
                        .rounded(px(8.))
                        .px_2()
                        .py_1p5()
                        .leading_sm()
                        .hover(move |style| style.bg(soft))
                        .child(decision.title.clone())
                        .when_some(link, |this, link| this.cursor_pointer().on_click(move |_, _, cx| DocsView::follow_link(&link, cx)))
                }))
        });
        Some(div().id(SharedString::from(format!("doc-section-mark-{at}"))).aria_label(said).absolute().top(px(top)).left(px(left)).child(popover).test_support().into_any_element())
    }

    fn render_row(&self, row: usize, window: &mut Window, cx: &mut App) -> AnyElement {
        #[cfg(test)]
        tests::ROWS_BUILT.with(|built| built.borrow_mut().insert(row));
        let inner: AnyElement = match self.rows[row] {
            ReadingRow::PageHead => DocsView::render_page_head(self.name.clone(), self.trail.clone(), self.title.clone(), cx).into_any_element(),
            ReadingRow::Nothing => DocsView::render_nothing(self.empty.unwrap_or("work:docs.emptyTab"), self.editable, cx).into_any_element(),
            ReadingRow::Piece(at) => {
                let piece = &self.pieces[at];
                // Un id por nodo, pestaña y trozo: el del texto anterior no
                // debe prestarle su estado a éste.
                let id = SharedString::from(format!("doc-text-{}-{}-{at}", self.node, self.key));
                let kind = if piece.wide { "wide" } else { "prose" };
                let mark = self.render_decisions_mark(at, window, cx);
                let text = div()
                    .id(SharedString::from(format!("doc-{kind}-{at}")))
                    .relative()
                    .w_full()
                    .min_w_0()
                    .text_size(px(READING_SIZE))
                    .line_height(relative(1.75))
                    // La prosa en su medida y centrada; lo ancho (tablas,
                    // código), a todo lo que da la página.
                    .when(!piece.wide, |this| this.max_w(px(READING_MEASURE)).mx_auto())
                    .child(DocsView::doc_text(id, piece.text.clone(), reading_style(cx)))
                    .children(mark)
                    .test_support();
                // El primero dice que hay texto que leer.
                if at == 0 {
                    div().id("doc-body").w_full().min_w_0().child(text).test_support().into_any_element()
                } else {
                    div().w_full().min_w_0().pt(px(self.leads[at])).child(text).into_any_element()
                }
            }
            ReadingRow::Decision(at) => div()
                .w_full()
                .max_w(px(READING_MEASURE))
                .mx_auto()
                .when(at > 0, |this| this.pt_2())
                .child(DocsView::render_decision(self.decisions[at].clone(), cx))
                .into_any_element(),
            ReadingRow::PageTail => match &self.tail {
                Some((page_id, children, backlinks)) => {
                    DocsView::render_page_tail(page_id.clone(), self.can_add, children.clone(), backlinks.clone(), cx).into_any_element()
                }
                None => div().into_any_element(),
            },
        };
        // La caja de siempre, fila a fila: `p-6` alrededor, el ancho de la
        // página centrado y, en una ventana grande, el sitio del índice.
        div()
            .w_full()
            .px(px(READING_PAD))
            .when(row == 0, |this| this.pt(px(READING_PAD)))
            .when(row + 1 == self.rows.len(), |this| this.pb(px(READING_PAD)))
            .child(
                div()
                    .w_full()
                    .max_w(px(if self.wide { WIDE_PAGE_WIDTH } else { PAGE_WIDTH }))
                    .mx_auto()
                    .flex()
                    .when(self.wide, |this| this.gap_8())
                    .child(div().min_w_0().flex_1().flex().flex_col().child(inner))
                    .when(self.with_contents, |this| this.child(div().w(px(CONTENTS_WIDTH)).flex_none())),
            )
            .into_any_element()
    }
}

/// Las cajas donde se escribe. Se crean la primera vez que se abre el
/// editor: hacen falta con la ventana delante.
struct EditBoxes {
    title: Entity<TextareaState>,
    body: Entity<TextareaState>,
    /// Lo que cuenta cada tecla al core. Vive lo que vivan las cajas.
    _heard: [Subscription; 2],
}

pub struct DocsView {
    /// El foco de la hoja «Páginas»: con él, Escape la cierra.
    sheet_focus: FocusHandle,
    /// Si la hoja ya estaba abierta en el pintado anterior: el foco se le da
    /// una vez, al abrirse.
    sheet_was_shown: bool,
    boxes: Option<EditBoxes>,
    /// De qué sesión de escritura (`DocEdit::session`) es el texto que hay
    /// en las cajas: se pone una vez al abrirse el editor, no en cada tecla.
    loaded_session: Option<u64>,
    /// El foco de la pregunta (¿descartar?, ¿guardar encima?): con él,
    /// Escape la contesta que no.
    question_focus: FocusHandle,
    was_asking: bool,
    /// De qué enlace fallido (`DocLinkFailure::serial`) es el aviso que ya
    /// tiene puesto su reloj para quitarse solo.
    timed_failure: Option<u64>,
    /// Por dónde va la lectura de cada pestaña del documento abierto, y la
    /// de la página que se lee: cada una guarda lo suyo.
    readings: HashMap<String, Reading>,
    /// De qué nodo son.
    readings_of: String,
    /// La entrada del índice marcada en el último pintado.
    active_entry: Option<u32>,
    /// «Compartir con enlace» de la página que se lee, mientras está abierto.
    share_link: Option<(Entity<PageShare>, Subscription)>,
}

/// Cuánto se queda a la vista el aviso de un enlace que no se pudo abrir:
/// lo que dura un aviso de la app de siempre (sonner, 4 s).
const FAILURE_SHOWN_FOR: std::time::Duration = std::time::Duration::from_secs(4);

/// `text-xs font-semibold uppercase text-muted-foreground`: el título de
/// una sección al pie de una página.
fn section_title(text: String, muted: Hsla) -> Div {
    div().leading_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted).child(text.to_uppercase())
}

/// Lo que dice al pasar el ratón una pieza que la app de siempre tiene y
/// ésta todavía no.
fn not_yet(cx: &App) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let phrase = tr(cx, "common:misc.notInThisApp");
    move |window, cx| Tooltip::new(phrase.clone()).build(window, cx)
}

/// Un botón fantasma pequeño con icono y texto (`Button size="sm"
/// variant="ghost"`): los del pie y el de compartir.
fn ghost_button(id: &'static str, icon: Lucide, label: String, cx: &App) -> Stateful<Div> {
    let hover = crate::theme::ghost_hover(cx);
    div()
        .id(id)
        .h_7()
        .px_2p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .rounded(px(8.))
        // `text-[0.8rem]` del `size="sm"`.
        .text_size(rems(0.8))
        .font_weight(FontWeight::MEDIUM)
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size_3())
        .child(label)
}

/// Cómo se lee un documento: la tipografía de `.prose-doc .md-body` (15 px
/// con aire entre líneas; los títulos a 18, 17 y 13) y, de `.md-body`, el
/// bloque de código sobre fondo apagado y las celdas con su borde.
pub(crate) fn reading_style(cx: &App) -> TextViewStyle {
    let muted = cx.theme().muted;
    let mut table = StyleRefinement::default();
    // Una tabla que no cabe se desliza por dentro en vez de estrujarse.
    table.overflow.x = Some(Overflow::Scroll);
    TextViewStyle::default()
        .paragraph_gap(rems(0.8))
        .heading_font_size(|level, _| px(heading_size(level)))
        .code_block(StyleRefinement::default().bg(muted).rounded(px(8.)).p_3().leading_xs())
        .table(table)
        .table_head(StyleRefinement::default().bg(muted.opacity(0.5)).font_weight(FontWeight::MEDIUM))
        .table_cell(StyleRefinement::default().px_2().py_1().leading_xs())
}

impl DocsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            sheet_focus: cx.focus_handle(),
            sheet_was_shown: false,
            boxes: None,
            loaded_session: None,
            question_focus: cx.focus_handle(),
            was_asking: false,
            timed_failure: None,
            readings: HashMap::new(),
            readings_of: String::new(),
            active_entry: None,
            share_link: None,
        }
    }

    /// Abre «Compartir con enlace» para la página que se lee.
    fn open_share_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let docs = Backend::snapshot(cx).read(cx).docs.clone();
        let (Some(target), Some(page)) = (docs.target, docs.page) else { return };
        let share = cx.new(|cx| PageShare::new(target.kind, target.id, page.page.id, window, cx));
        let closed = cx.subscribe(&share, |this, _, _: &PageShareClosed, cx| {
            this.share_link = None;
            cx.notify();
        });
        self.share_link = Some((share, closed));
        cx.notify();
    }

    fn answer(yes: bool, cx: &App) {
        Backend::run(cx, move |core| async move { core.docs().answer(yes).await });
    }

    fn save(cx: &App) {
        Backend::run(cx, |core| async move { core.docs().save().await });
    }

    /// Las cajas del editor, con el texto de esta sesión de escritura. Lo
    /// que hay que escribir en ellas lo dice el core una vez, al abrirse;
    /// desde ahí son ellas las que le cuentan cada tecla.
    fn boxes_for(&mut self, edit: &DocEdit, window: &mut Window, cx: &mut Context<Self>) -> (Entity<TextareaState>, Entity<TextareaState>) {
        let boxes = self.boxes.get_or_insert_with(|| {
            let untitled = tr(cx, "work:docs.untitled");
            let title = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 3).placeholder(untitled));
            // De filas fijas: llena el alto que le den y se desliza dentro.
            let body = cx.new(|cx| TextareaState::new(window, cx));
            let heard = [
                cx.subscribe(&title, |_, title, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        Backend::core(cx).docs().draft_title(&title.read(cx).value());
                    }
                }),
                cx.subscribe(&body, |_, body, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        Backend::core(cx).docs().draft_body(&body.read(cx).value());
                    }
                }),
            ];
            EditBoxes { title, body, _heard: heard }
        });
        let (title, body) = (boxes.title.clone(), boxes.body.clone());
        if self.loaded_session != Some(edit.session) {
            self.loaded_session = Some(edit.session);
            title.update(cx, |input, cx| input.set_value(edit.title.clone(), window, cx));
            body.update(cx, |input, cx| input.set_value(edit.body.clone(), window, cx));
            // Una página nueva empieza por su título; lo demás, por el texto.
            let first = if matches!(edit.what, DocEditWhat::NewPage { .. }) { &title } else { &body };
            first.update(cx, |input, cx| input.focus(window, cx));
        }
        (title, body)
    }

    /// Un botón pequeño con borde (`Button size="sm" variant="outline"
    /// className="h-6 text-xs"`): los del aviso de choque y el conmutador.
    fn outline_button(id: &'static str, label: String, active: bool, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        let (border, accent, foreground, muted) = (theme.border, theme.accent, theme.foreground, theme.muted_foreground);
        div()
            .id(id)
            .h_6()
            .px_2()
            .flex()
            .flex_none()
            .items_center()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .leading_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(if active { foreground } else { muted })
            .when(active, |this| this.bg(accent))
            .cursor_pointer()
            .hover(move |style| style.bg(accent).text_color(foreground))
            .child(label)
    }

    /// El aviso de que alguien guardó en medio (`DocTabs.tsx`: `rounded-md
    /// border border-destructive/40 bg-destructive/[.07] px-3 py-2
    /// text-xs`). Lo escrito sigue en la caja; aquí se decide qué pasa con
    /// lo que hay guardado, y las dos salidas que pierden algo preguntan.
    fn render_conflict(edit: &DocEdit, cx: &App) -> Option<impl IntoElement> {
        let said = edit.conflict_phrase(locale(cx))?;
        let theme = cx.theme();
        let (danger, foreground, muted) = (theme.danger, theme.foreground, theme.muted_foreground);
        let showing = edit.showing_theirs;
        Some(
            div()
                .id("doc-conflict")
                .flex_none()
                .mx_6()
                .mb_2()
                .px_3()
                .py_2()
                .rounded(px(8.))
                .border_1()
                .border_color(danger.opacity(0.4))
                .bg(danger.opacity(0.07))
                .leading_xs()
                .child(div().font_weight(FontWeight::MEDIUM).text_color(foreground).child(tr(cx, "work:docs.conflict")))
                .child(div().mt_0p5().text_color(muted).child(said))
                .child(
                    div()
                        .mt_2()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Self::outline_button("doc-conflict-see", tr(cx, if showing { "work:docs.seeMine" } else { "work:docs.seeTheirs" }), showing, cx)
                                .on_click(move |_, _, cx| Backend::core(cx).docs().show_theirs(!showing))
                                .test_support(),
                        )
                        .child(
                            Self::outline_button("doc-conflict-load", tr(cx, "work:docs.discardAndReload"), false, cx)
                                .on_click(|_, _, cx| Backend::core(cx).docs().ask_load_theirs())
                                .test_support(),
                        )
                        .child(
                            Self::outline_button("doc-conflict-overwrite", tr(cx, "work:docs.overwrite"), false, cx)
                                .on_click(|_, _, cx| Backend::core(cx).docs().ask_overwrite())
                                .test_support(),
                        ),
                )
                .test_support(),
        )
    }

    /// El editor: arriba qué se escribe (y, en una página, su título); en
    /// medio la caja y, al lado, el documento ya pintado; abajo, guardar y
    /// cancelar donde estaba «Editar».
    fn render_editor(&mut self, docs: &DocsState, edit: &DocEdit, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (title_box, body_box) = self.boxes_for(edit, window, cx);
        let theme = cx.theme();
        let (muted, border, danger, warning) = (theme.muted_foreground, theme.border, theme.danger, theme.warning);
        let here = locale(cx);
        let node = docs.target.as_ref().map(|target| target.id.clone()).unwrap_or_default();
        // Lado a lado si caben los dos; si no, uno u otro.
        let side = cac_core::doc_editor_side_by_side(f32::from(window.viewport_size().width));
        let (shows_box, shows_preview) = (side || !edit.previewing, side || edit.previewing);

        let switch = (!side).then(|| {
            let previewing = edit.previewing;
            div()
                .id("doc-edit-switch")
                .ml_auto()
                .flex()
                .flex_none()
                .gap_1()
                .child(
                    Self::outline_button("doc-edit-show-box", tr(cx, "work:docs.write"), !previewing, cx)
                        .on_click(|_, _, cx| Backend::core(cx).docs().show_preview(false))
                        .test_support(),
                )
                .child(
                    Self::outline_button("doc-edit-show-preview", tr(cx, "work:docs.preview"), previewing, cx)
                        .on_click(|_, _, cx| Backend::core(cx).docs().show_preview(true))
                        .test_support(),
                )
                .test_support()
        });
        // Qué se escribe: una página nueva lo dice; una que ya existe, con
        // sus migas; una pestaña, con su pestaña marcada en la fila de arriba.
        let heading = docs.edit_heading(here).map(|heading| div().id("doc-editing-what").aria_label(heading.clone()).leading_xs().text_color(muted).child(heading).test_support());
        let trail = matches!(edit.what, DocEditWhat::Page { .. }).then(|| {
            let name = docs.target.as_ref().map(|target| target.name.clone()).unwrap_or_default();
            Self::render_page_head(name, docs.page_trail(here), None, cx)
        });
        let for_enter = body_box.clone();
        let title = edit.has_title().then(|| {
            div()
                .id("doc-edit-title")
                // Enter en el título baja al texto: un título es una línea.
                .capture_action(move |_: &Enter, window, cx| {
                    for_enter.update(cx, |input, cx| input.focus(window, cx));
                    cx.stop_propagation();
                })
                // El campo trae 4 px de margen por dentro: sin esto el título
                // no cae en la vertical de las migas y de la caja.
                .ml(px(-4.))
                .child(
                    Textarea::new(&title_box)
                        .appearance(false)
                        .bordered(false)
                        .xsmall()
                        // `text-2xl font-semibold`, como el título que se lee.
                        .text_size(px(24.))
                        .line_height(px(32.))
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .test_support()
        });
        // Sin nada que decir encima (una pestaña, en una ventana ancha) no
        // hay cabecera: las cajas suben.
        let head = (heading.is_some() || trail.is_some() || switch.is_some()).then(|| {
            div()
                .flex_none()
                .px_6()
                .pb_2()
                .flex()
                .flex_col()
                .child(div().flex().items_center().gap_2().child(div().min_w_0().flex_1().children(heading).children(trail)).children(switch))
                .children(title)
        });
        // Encima de cada caja, qué es (`text-[11px] uppercase`): lo que se
        // escribe y cómo queda. Así las dos empiezan a la misma altura.
        let pane_label = move |id: &'static str, said: String, tone: Hsla| {
            div().id(id).aria_label(said.clone()).flex_none().text_size(px(11.)).text_color(tone).child(said.to_uppercase()).test_support()
        };

        let placeholder = if edit.has_title() { tr(cx, "common:editor.writeInMarkdown") } else { tr(cx, "work:docs.placeholder") };
        // Lo que se pinta al lado es un solo texto que se desliza por
        // dentro, y no los trozos de lo que se lee: así sólo se maqueta lo
        // que cabe en la caja. Con los trozos, cada tecla volvía a maquetar
        // el documento entero, y en uno de 3.000 líneas eso son 115 ms por
        // cuadro (medido el 18-nov-2026): se escribía a tirones.
        let previewed = edit.preview().trim().to_string();
        let theirs = edit.showing_theirs && edit.conflict.is_some();
        let preview_label = tr(cx, if theirs { "work:docs.theirsLabel" } else { "work:docs.preview" });
        let preview = div()
            .id("doc-edit-preview")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(pane_label("doc-edit-preview-label", preview_label, if theirs { danger } else { muted }))
            .child(
                div()
                    .id("doc-edit-preview-frame")
                    .flex_1()
                    .min_h_0()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if theirs { danger.opacity(0.4) } else { border })
                    .p_4()
                    .map(|this| {
                        if previewed.is_empty() {
                            this.child(div().id("doc-edit-preview-empty").leading_sm().text_color(muted).child(tr(cx, "work:docs.emptyTab")).test_support())
                        } else {
                            // Su propio id: el estado de lo que se lee no es
                            // el de lo que se escribe, ni lo propio el de lo
                            // que guardó otro.
                            let id = SharedString::from(format!("doc-edit-preview-{node}-{}", if theirs { "theirs" } else { "draft" }));
                            this.child(
                                div()
                                    .id("doc-edit-preview-text")
                                    .size_full()
                                    .text_size(px(15.))
                                    .line_height(relative(1.75))
                                    .child(Self::doc_text(id, previewed, reading_style(cx)).scrollable(true))
                                    .test_support(),
                            )
                        }
                    })
                    .test_support(),
            )
            .test_support();
        let panes = div()
            .flex_1()
            .min_h_0()
            .flex()
            .gap_4()
            .px_6()
            .pb_4()
            .when(shows_box, |this| {
                this.child(
                    div()
                        .id("doc-edit-box")
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(pane_label("doc-edit-box-label", tr(cx, "work:docs.write"), muted))
                        .child(MarkdownEditor::new("doc-edit", &body_box).fill().placeholder(placeholder))
                        .test_support(),
                )
            })
            .when(shows_preview, |this| this.child(preview));

        // El pie, donde estaba «Editar»: guardar, cancelar y en qué está lo
        // escrito. Con un choque a la vista «Guardar» se apaga: primero hay
        // que decidir arriba qué pasa con lo que hay guardado.
        let status = edit.status(here).map(|said| {
            div().id("doc-edit-status").aria_label(said.clone()).leading_xs().text_color(if edit.saving { warning } else { muted }).child(said).test_support()
        });
        let failed = edit.error_phrase(here).map(|said| {
            div()
                .id("doc-edit-error")
                .aria_label(said.clone())
                .min_w_0()
                .flex()
                .items_center()
                .gap_1()
                .leading_xs()
                .text_color(danger)
                .child(Icon::new(Lucide::CircleAlert).size_3().flex_none())
                .child(said)
                .test_support()
        });
        let foot = div()
            .id("doc-foot")
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(border)
            .child(
                Button::new("doc-save")
                    .primary()
                    .small()
                    .h_7()
                    .px_2p5()
                    .label(tr(cx, "work:docs.save"))
                    .disabled(!edit.can_save())
                    .loading(edit.saving)
                    .on_click(|_, _, cx| Self::save(cx)),
            )
            .child(ghost_button("doc-cancel", Lucide::X, tr(cx, "work:docs.cancel"), cx).cursor_pointer().on_click(|_, _, cx| Backend::core(cx).docs().cancel_edit()).test_support())
            .children(status)
            .children(failed);

        div()
            .id("doc-editor")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            // Ctrl+S (Cmd en un Mac) guarda, se esté en la caja que se esté.
            .on_key_down(|event: &KeyDownEvent, _, cx| {
                let held = event.keystroke.modifiers;
                if held.secondary(cx) && !held.shift && !held.alt && event.keystroke.key == "s" {
                    Self::save(cx);
                    cx.stop_propagation();
                }
            })
            .pt_4()
            .children(head)
            .children(Self::render_conflict(edit, cx))
            .child(panes)
            .child(foot)
            .test_support()
            .into_any_element()
    }

    /// La pregunta antes de perder o pisar texto, sobre toda la ventana
    /// (`ask.rs`). El foco se le da una vez, al salir: con él, Escape la
    /// contesta que no.
    fn render_question(&mut self, question: Question, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.was_asking {
            self.question_focus.focus(window, cx);
        }
        crate::ask::question_dialog("doc-ask", question, &self.question_focus, |yes, _, cx| Self::answer(yes, cx), window, cx)
    }

    fn open_page(id: String, cx: &App) {
        Backend::run(cx, move |core| async move { core.docs().open_page(&id).await });
    }

    /// Un botón de una fila del árbol de páginas que sólo sale al pasar el
    /// ratón por ella (`hidden size-5 … group-hover:grid`). Con `run` hace
    /// eso al pulsarlo; sin él todavía no hace nada aquí, y lo dice al pasar
    /// por él.
    fn row_action(
        id: SharedString,
        icon: impl Into<Icon>,
        label: String,
        group: SharedString,
        run: Option<Box<dyn Fn(&mut App)>>,
        cx: &App,
    ) -> impl IntoElement {
        let background = cx.theme().background;
        let hint = if run.is_some() { label } else { format!("{label} · {}", tr(cx, "common:misc.notInThisApp")) };
        div()
            .id(id)
            .size_5()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .opacity(0.)
            .group_hover(group, |style| style.opacity(1.))
            .hover(move |style| style.bg(background))
            .child(icon.into().size_3())
            .when_some(run, |this, run| {
                this.cursor_pointer().on_click(move |_, _, cx| {
                    // El botón no es la fila: pulsarlo no abre la página.
                    cx.stop_propagation();
                    run(cx);
                })
            })
            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
            .test_support()
    }

    /// Lo que hace «+» en una fila: empezar una página nueva dentro de ésa
    /// (o en la portada). No crea nada: la página existe al guardarla.
    fn new_page_in(parent: Option<String>) -> Option<Box<dyn Fn(&mut App)>> {
        Some(Box::new(move |cx| Backend::core(cx).docs().new_page(parent.as_deref())))
    }

    /// La caja de una fila de la hoja (`group flex h-7 items-center gap-1
    /// rounded-md pr-1 text-[13px]`): marcada si es lo que se lee, apagada
    /// si no.
    fn sheet_row(id: SharedString, group: SharedString, active: bool, indent: f32, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        let (accent, accent_foreground, muted, foreground) = (theme.sidebar_accent, theme.sidebar_accent_foreground, theme.muted_foreground, theme.foreground);
        div()
            .id(id)
            .group(group)
            .h_7()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .rounded(px(8.))
            .pl(px(indent))
            .pr_1()
            .text_size(px(13.))
            .line_height(px(18.))
            .cursor_pointer()
            .map(|this| {
                if active {
                    this.bg(accent).font_weight(FontWeight::MEDIUM).text_color(accent_foreground)
                } else {
                    this.text_color(muted).hover(move |style| style.bg(accent.opacity(0.6)).text_color(foreground))
                }
            })
    }

    /// Una página en el árbol (`PageRow.tsx`): el chevrón si tiene algo
    /// dentro, su icono y su título, y al pasar el ratón «+» y su menú.
    fn render_page_row(row: DocPageRow, can_add: bool, cx: &App) -> impl IntoElement {
        let group = SharedString::from(format!("doc-page-group-{}", row.id));
        let (open_id, toggle_id) = (row.id.clone(), row.id.clone());
        let menu_label = cac_core::text_with(locale(cx), "work:docs.pageMenu", &[("title", &row.title)]);
        Self::sheet_row(SharedString::from(format!("doc-page-row-{}", row.id)), group.clone(), row.active, 4. + 12. * row.depth as f32, cx)
            .on_click(move |_, _, cx| Self::open_page(open_id.clone(), cx))
            .child(
                div()
                    .id(SharedString::from(format!("doc-page-toggle-{}", row.id)))
                    .size_4()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .when(row.has_children, |this| {
                        this.child(Icon::new(if row.open { IconName::ChevronDown } else { IconName::ChevronRight }).size_3()).on_click(move |_, _, cx| {
                            // El chevrón pliega y despliega; no abre la página.
                            cx.stop_propagation();
                            Backend::core(cx).docs().toggle_page(&toggle_id);
                        })
                    })
                    .test_support(),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(Icon::new(Lucide::FileText).size_3p5().flex_none())
                    .child(div().min_w_0().truncate().child(row.title)),
            )
            // Quien sólo puede mirar no ve cómo crear.
            .when(can_add, |this| {
                this.child(Self::row_action(
                    SharedString::from(format!("doc-page-new-{}", row.id)),
                    Lucide::Plus,
                    tr(cx, "work:docs.newSubpage"),
                    group.clone(),
                    Self::new_page_in(Some(row.id.clone())),
                    cx,
                ))
            })
            .child(Self::row_action(SharedString::from(format!("doc-page-menu-{}", row.id)), IconName::Ellipsis, menu_label, group, None, cx))
            .test_support()
    }

    /// La hoja «Páginas» (`DocPagesSheet.tsx`): sale por la derecha sobre
    /// toda la ventana, con la portada arriba y el árbol debajo. Pulsar
    /// fuera, la equis o Escape la cierran; elegir algo, también.
    fn render_pages_sheet(&mut self, docs: &DocsState, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (sidebar, sidebar_foreground, border, muted) = (theme.sidebar, theme.sidebar_foreground, theme.border, theme.muted_foreground);
        let hover = crate::theme::ghost_hover(cx);
        let rows = docs.page_rows(locale(cx));
        let can_add = docs.can_add_pages();
        let home_group = SharedString::from("doc-pages-home-group");
        let (home_hint, close_hint) = (tr(cx, "work:docs.homeHint"), tr(cx, "work:docs.close"));

        // La portada: las cuatro pestañas de siempre, con «+» y la papelera.
        let home = Self::sheet_row("doc-pages-home".into(), home_group.clone(), !docs.on_page(), 24., cx)
            .on_click(|_, _, cx| Backend::core(cx).docs().close_page())
            .tooltip(move |window, cx| Tooltip::new(home_hint.clone()).build(window, cx))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(Icon::new(Lucide::House).size_3p5().flex_none())
                    .child(div().min_w_0().truncate().child(tr(cx, "work:docs.home"))),
            )
            .when(can_add, |this| {
                this.child(Self::row_action("doc-pages-new".into(), Lucide::Plus, tr(cx, "work:docs.newPage"), home_group.clone(), Self::new_page_in(None), cx))
            })
            .child(Self::row_action("doc-pages-trash".into(), Lucide::Trash, tr(cx, "work:docs.trash"), home_group, None, cx))
            .test_support();

        let panel = div()
            .id("doc-pages-sheet")
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .w(px(PAGES_SHEET_WIDTH))
            .flex()
            .flex_col()
            .p_2()
            .border_l_1()
            .border_color(border)
            .bg(sidebar)
            .text_color(sidebar_foreground)
            .shadow_xl()
            .overflow_y_scroll()
            // Lo que se pulsa dentro no llega al velo, que la cerraría.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // `SheetHeader` con `px-1 pb-1` encima de su `p-4`, y el título
            // a `text-base font-medium`.
            .child(div().flex_none().pt_4().px_1().pb_1().text_size(px(16.)).line_height(px(24.)).font_weight(FontWeight::MEDIUM).child(tr(cx, "work:docs.pages")))
            .child(
                // La equis (`absolute top-3 right-3`, `size="icon-sm"`).
                div()
                    .id("doc-pages-close")
                    .absolute()
                    .top_3()
                    .right_3()
                    .size_7()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(Icon::new(Lucide::X).size_4())
                    .on_click(|_, _, cx| Backend::core(cx).docs().hide_pages())
                    .tooltip(move |window, cx| Tooltip::new(close_hint.clone()).build(window, cx))
                    .test_support(),
            )
            .child(home)
            .children(rows.into_iter().map(|row| Self::render_page_row(row, can_add, cx)))
            // El árbol no llegó: se dice, en vez de una hoja vacía que
            // parece un documento sin páginas.
            .when(docs.pages_error.is_some(), |this| {
                this.child(div().id("doc-pages-error").test_support().px_2().py_3().leading_xs().text_color(muted).child(tr(cx, "common:last.requestsFailing")))
            })
            .test_support();

        if !self.sheet_was_shown {
            self.sheet_focus.focus(window, cx);
        }
        let size = window.viewport_size();
        // Anclada al origen de la ventana y diferida: tapa también el panel
        // lateral, como la hoja de la app de siempre (`fixed inset-0`).
        deferred(
            anchored().position(Point::default()).child(
                div()
                    .id("doc-pages-veil")
                    .w(size.width)
                    .h(size.height)
                    .relative()
                    .occlude()
                    .track_focus(&self.sheet_focus)
                    .on_key_down(|event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" {
                            Backend::core(cx).docs().hide_pages();
                            cx.stop_propagation();
                        }
                    })
                    .bg(gpui_kit::black().opacity(0.35))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| Backend::core(cx).docs().hide_pages())
                    .child(panel)
                    .test_support(),
            ),
        )
        .with_priority(1)
    }

    /// Lo que va encima del texto de una página (`DocPageView.tsx`): las
    /// migas de pan —del nodo, que lleva a la portada, a la madre— y su
    /// título a `text-2xl font-semibold`.
    fn render_page_head(name: String, trail: Vec<DocPageLink>, title: Option<String>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground) = (theme.muted_foreground, theme.foreground);
        let crumb = move |id: SharedString, text: String| {
            div().id(id).cursor_pointer().hover(move |style| style.text_color(foreground).underline()).child(text)
        };
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("doc-page-trail")
                    .mb_2()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .leading_xs()
                    .text_color(muted)
                    .child(crumb("doc-page-crumb-home".into(), name).on_click(|_, _, cx| Backend::core(cx).docs().close_page()).test_support())
                    .children(trail.into_iter().map(|step| {
                        let id = step.id.clone();
                        div().flex().items_center().gap_1().child(Icon::new(IconName::ChevronRight).size_3()).child(
                            crumb(SharedString::from(format!("doc-page-crumb-{}", step.id)), step.title)
                                .on_click(move |_, _, cx| Self::open_page(id.clone(), cx))
                                .test_support(),
                        )
                    }))
                    .test_support(),
            )
            .children(title.map(|title| div().id("doc-page-title").test_support().mb_4().text_size(px(24.)).line_height(px(32.)).font_weight(FontWeight::SEMIBOLD).child(title)))
    }

    /// Una página vacía ofrece por dónde empezar: dos plantillas y
    /// «Escribirlo». Una plantilla deja su texto en la caja; no se guarda
    /// hasta que se pulse guardar (la app de siempre la guarda al elegirla).
    /// A quien sólo puede mirar no se le ofrece nada: sólo se le dice que
    /// está vacía.
    fn render_empty_page(said: impl IntoElement, can_write: bool, cx: &App) -> Div {
        let theme = cx.theme();
        let (muted, border, accent) = (theme.muted_foreground, theme.border, theme.accent);
        let block = div().py_8().flex().flex_col().items_center().child(said);
        if !can_write {
            return block;
        }
        block
            .child(div().mt_4().w_full().max_w(px(512.)).flex().gap_2().children(PAGE_TEMPLATES.map(|(key, icon)| {
                div()
                    .id(SharedString::from(format!("doc-page-template-{key}")))
                    .flex_1()
                    .min_w_0()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .hover(move |style| style.bg(accent))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .leading_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(Icon::new(icon).size_4().flex_none().text_color(muted))
                            .child(tr(cx, &format!("work:pageTemplates.{key}.name"))),
                    )
                    .child(div().mt_1().leading_xs().text_color(muted).child(tr(cx, &format!("work:pageTemplates.{key}.hint"))))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        Backend::core(cx).docs().start_from_template(key, locale(cx));
                    })
                    .test_support()
            })))
            .child(
                ghost_button("doc-write", Lucide::Pencil, tr(cx, "work:docs.writeIt"), cx)
                    .mt_3()
                    .cursor_pointer()
                    .on_click(|_, _, cx| {
                        Backend::core(cx).docs().start_edit();
                    })
                    .test_support(),
            )
    }

    /// Lo que va bajo el texto de una página: las que tiene dentro, siempre
    /// (dice qué hay sin mirar el árbol), y lo que la enlaza, si algo lo
    /// hace.
    fn render_page_tail(page_id: String, can_add: bool, children: Vec<DocPageLink>, backlinks: Vec<DocBacklinkRow>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, primary) = (theme.muted_foreground, theme.border, theme.primary);
        let hover = crate::theme::ghost_hover(cx);
        let new_hint = tr(cx, "work:docs.newSubpage");
        let inside = div()
            .id("doc-page-children")
            .mt_10()
            .pt_4()
            .border_t_1()
            .border_color(border)
            .flex()
            .flex_col()
            .child(
                div().mb_2().flex().items_center().gap_2().child(section_title(tr(cx, "work:docs.childPages"), muted)).when(can_add, |this| {
                    this.child(
                        div()
                            .id("doc-page-new-child")
                            .size_6()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .child(Icon::new(Lucide::Plus).size_3p5())
                            .on_click(move |_, _, cx| Backend::core(cx).docs().new_page(Some(&page_id)))
                            .tooltip(move |window, cx| Tooltip::new(new_hint.clone()).build(window, cx))
                            .test_support(),
                    )
                }),
            )
            .child(div().flex().flex_col().items_start().gap_1().children(children.into_iter().map(|child| {
                let id = child.id.clone();
                div()
                    .id(SharedString::from(format!("doc-page-child-{}", child.id)))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .leading_sm()
                    .text_color(primary)
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .on_click(move |_, _, cx| Self::open_page(id.clone(), cx))
                    .child(Icon::new(Lucide::FileText).size_3p5().flex_none())
                    .child(child.title)
                    .test_support()
            })))
            .test_support();

        let linked = (!backlinks.is_empty()).then(|| {
            div()
                .id("doc-page-backlinks")
                .mt_6()
                .flex()
                .flex_col()
                .child(div().mb_2().child(section_title(tr(cx, "work:docs.referencedFrom"), muted)))
                .child(div().flex().flex_col().gap_1().children(backlinks.into_iter().enumerate().map(|(index, link)| {
                    let icon = if link.from_task { Lucide::SquareKanban } else { Lucide::FileText };
                    div()
                        .id(SharedString::from(format!("doc-page-backlink-{index}")))
                        .w_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .leading_sm()
                        .child(Icon::new(icon).size_3p5().flex_none().text_color(muted))
                        .child(div().min_w_0().truncate().text_color(primary).child(link.title))
                        .children(link.place.map(|place| div().flex_none().leading_xs().text_color(muted).child(place)))
                        // Lo que la nombra se abre: otra página, una tarea,
                        // o la pestaña de otro documento.
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(move |_, _, cx| Self::follow_link(&link.link, cx))
                        .test_support()
                })))
                .test_support()
        });
        div().flex().flex_col().child(inside).children(linked)
    }

    /// Una entrada del registro, con las medidas de `DecisionList.tsx`
    /// (`rounded-[11px] border p-3`): arriba cuándo, quién y su etiqueta;
    /// el título; el porqué, tal cual se escribió; y de dónde salió.
    fn render_decision(decision: DocDecision, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, chip) = (theme.muted_foreground, theme.border, theme.muted);
        let when = decision.decided_on(locale(cx));
        let (from_task, origin) = decision.origin(locale(cx));
        let body = (!decision.body.trim().is_empty()).then(|| div().mt_1().leading_sm().text_color(muted).child(decision.body.trim().to_string()));

        div()
            .id(SharedString::from(format!("doc-decision-{}", decision.id)))
            .test_support()
            .flex()
            .flex_col()
            .p_3()
            .rounded(px(11.))
            .border_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .leading_xs()
                    .text_color(muted)
                    .child(div().flex_none().child(when))
                    .children(decision.author_name.clone().filter(|name| !name.is_empty()).map(|name| div().min_w_0().truncate().child(name)))
                    .children(decision.tag.clone().filter(|tag| !tag.is_empty()).map(|tag| div().ml_auto().flex_none().px_2().py_0p5().rounded_full().bg(chip).child(tag))),
            )
            .child(div().mt_1().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(decision.title))
            .children(body)
            .child(
                div()
                    .mt_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(border)
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .leading_xs()
                    .text_color(muted)
                    .when(from_task, |this| this.child(Icon::new(Lucide::SquareCheck).size_3()))
                    .child(origin),
            )
    }

    /// Sigue un enlace de un documento. Qué es lo dice el core
    /// (`cac_core::doc_link`): lo de fuera se le da al sistema; lo de dentro
    /// —una tarea, una página, la pestaña de otro nodo, una sección— lo abre
    /// el core, que pregunta antes si hay texto sin guardar y deja dicho lo
    /// que no se pudo abrir.
    fn follow_link(url: &str, cx: &mut App) {
        match cac_core::doc_link(url) {
            cac_core::DocLink::Web { url } => cx.open_url(&url),
            _ => {
                let href = url.to_string();
                Backend::run(cx, move |core| async move { core.docs().follow_link(&href).await });
            }
        }
    }

    /// Un texto del documento, pintado, con sus enlaces seguidos por
    /// [`Self::follow_link`].
    fn doc_text(id: SharedString, markdown: String, style: TextViewStyle) -> TextView {
        // Un título de tercer nivel, como etiqueta de sección (`.md-body h3`).
        TextView::markdown(id, cac_core::prose_for_painting(&markdown)).style(style).on_link_click(|url, _, _, cx| Self::follow_link(url, cx))
    }

    /// El aviso de que un enlace no se pudo abrir: el de la app de siempre
    /// (`toast.error` de sonner con `richColors closeButton`, abajo a la
    /// derecha: 356 px, `p-4`, 13 px, radio 8), con sus colores de error. Se
    /// quita solo a los cuatro segundos, o con su equis.
    fn render_link_failure(&mut self, failure: &cac_core::DocLinkFailure, cx: &mut Context<Self>) -> impl IntoElement {
        if self.timed_failure != Some(failure.serial) {
            self.timed_failure = Some(failure.serial);
            let serial = failure.serial;
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(FAILURE_SHOWN_FOR).await;
                cx.update(|cx| {
                    // Sólo el suyo: uno más nuevo tiene su propio reloj.
                    let core = Backend::core(cx);
                    if core.docs().state().link_failed.is_some_and(|failure| failure.serial == serial) {
                        core.docs().dismiss_link_failure();
                    }
                });
            })
            .detach();
        }
        // Los de `[data-rich-colors] [data-type=error]` de sonner.
        let (background, border, text) = if cx.theme().is_dark() {
            (hsla(358. / 360., 0.76, 0.10, 1.), hsla(357. / 360., 0.89, 0.16, 1.), hsla(358. / 360., 1., 0.81, 1.))
        } else {
            (hsla(359. / 360., 1., 0.97, 1.), hsla(359. / 360., 1., 0.94, 1.), hsla(360. / 360., 1., 0.45, 1.))
        };
        let said = tr(cx, "common:last.openLinkFailed");
        let close_hint = tr(cx, "common:last.close");
        div()
            .id("doc-link-failed")
            .aria_label(said.clone())
            .absolute()
            .bottom_6()
            .right_6()
            .w(px(356.))
            .max_w_full()
            .occlude()
            .flex()
            .items_center()
            .gap_1p5()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .bg(background)
            .shadow_lg()
            .text_size(px(13.))
            .line_height(px(19.5))
            .text_color(text)
            .child(Icon::new(Lucide::CircleAlert).size_4().flex_none())
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(div().font_weight(FontWeight::MEDIUM).child(said))
                    // Cuál: lo que llevaba escrito el enlace.
                    .child(div().id("doc-link-failed-href").aria_label(failure.href.clone()).truncate().child(failure.href.clone()).test_support()),
            )
            .child(
                div()
                    .id("doc-link-failed-close")
                    .size_5()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .hover(move |style| style.bg(border))
                    .child(Icon::new(Lucide::X).size_3())
                    .on_click(|_, _, cx| Backend::core(cx).docs().dismiss_link_failure())
                    .tooltip(move |window, cx| Tooltip::new(close_hint.clone()).build(window, cx))
                    .test_support(),
            )
            .test_support()
    }

    /// La cabecera del documento (`DocHeader.tsx`: `border-b px-4 py-2`, y
    /// dentro una fila a `text-xs gap-2`): quién responde de él, cuándo se
    /// confirmó por última vez, si hay una revisión pedida y la línea
    /// fijada sobre el tablero; y, si lleva demasiado sin revisar, el aviso
    /// de por qué. Aquí se lee: cambiar de responsable, fijar la línea y
    /// firmar la revisión están pintados en su sitio y dicen que todavía no
    /// están en esta app.
    fn render_header(header: cac_core::DocHeader, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground, border, primary) = (theme.muted_foreground, theme.foreground, theme.border, theme.primary);
        let hover = crate::theme::ghost_hover(cx);
        // `amber-500` y `emerald-500` de Tailwind 4.
        let amber = Hsla::from(rgb(0xfe9a00));
        let tone = if header.stale { amber } else { Hsla::from(rgb(0x00bc7d)) };
        let chip = |id: &'static str, said: &str| div().id(id).aria_label(said.to_string()).flex().flex_none().items_center().gap_1p5().rounded_full().px_2().py_0p5();

        let owner = chip("doc-owner", &header.owner)
            .border_1()
            .border_color(border)
            .map(|this| if header.has_owner { this.text_color(foreground) } else { this.border_dashed().text_color(muted) })
            .child(Icon::new(Lucide::UserRound).size_3())
            .child(header.owner.clone())
            .tooltip(not_yet(cx))
            .test_support();
        let reviewed = chip("doc-reviewed", &header.reviewed)
            .bg(tone.opacity(0.1))
            .text_color(tone)
            .child(Icon::new(if header.stale { Lucide::Clock } else { Lucide::CircleCheck }).size_3())
            .child(header.reviewed.clone())
            .test_support();
        // Una revisión pedida, al lado de la frescura, hasta que alguien
        // firme. La nota —qué cambió, qué mirar— sale al pasar por ella.
        let requested = header.review_requested.clone().map(|said| {
            let note = header.review_note.clone();
            chip("doc-review-requested", &said)
                .bg(primary.opacity(0.1))
                .text_color(primary)
                .child(Icon::new(Lucide::BellRing).size_3())
                .child(said)
                .when_some(note, |this, note| this.tooltip(move |window, cx| Tooltip::new(note.clone()).build(window, cx)))
                .test_support()
        });
        let pinned = div()
            .id("doc-pinned")
            .aria_label(header.pinned.clone())
            .ml_auto()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1p5()
            .text_color(muted)
            .hover(move |style| style.text_color(foreground))
            .child(Icon::new(Lucide::Pin).size_3().flex_none())
            .child(div().min_w_0().truncate().child(header.pinned.clone()))
            .tooltip(not_yet(cx))
            .test_support();
        // Los dos botones del aviso (`Button size="sm" className="h-6
        // text-xs"`, uno con borde y otro fantasma).
        let small = |id: &'static str, label: String| {
            div().id(id).h_6().px_2().flex().flex_none().items_center().rounded(px(8.)).leading_xs().font_weight(FontWeight::MEDIUM).hover(move |style| style.bg(hover)).child(label)
        };
        let notice = header.stale_notice.clone().map(|said| {
            div()
                .id("doc-stale")
                .aria_label(said.clone())
                .mt_2()
                .px_3()
                .py_2()
                .rounded(px(8.))
                .border_1()
                .border_color(amber.opacity(0.3))
                .bg(amber.opacity(0.07))
                .leading_xs()
                .child(div().text_color(foreground).child(said))
                .child(
                    div()
                        .mt_2()
                        .flex()
                        .gap_2()
                        // Firmar dice que una persona confirmó que sigue
                        // siendo verdad: sólo se le ofrece a quien puede.
                        .when(header.can_sign, |this| {
                            this.child(small("doc-mark-reviewed", tr(cx, "work:docs.markReviewed")).border_1().border_color(border).tooltip(not_yet(cx)).test_support())
                        })
                        .child(small("doc-reassign-owner", tr(cx, "work:docs.reassignOwner")).tooltip(not_yet(cx)).test_support()),
                )
                .test_support()
        });
        div()
            .id("doc-header")
            .flex_none()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(div().flex().flex_wrap().items_center().gap_2().leading_xs().child(owner).child(reviewed).children(requested).child(pinned))
            .children(notice)
            .test_support()
    }

    /// Sin nada que leer: por qué, en el centro (`py-12 text-center`), y
    /// cómo empezar a escribirlo, a quien puede.
    fn render_nothing(key: &'static str, editable: bool, cx: &App) -> Div {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let said = div().id("doc-empty").test_support().leading_sm().text_color(muted).child(tr(cx, key));
        let block = div().py_12().flex().flex_col().items_center().text_center();
        match key {
            "work:decisions.empty" => block
                .child(Icon::new(Lucide::Gavel).size_5().mb_2().text_color(muted))
                .child(said)
                .child(div().mt_1().max_w(px(384.)).leading_xs().text_color(muted).child(tr(cx, "work:decisions.emptyWhy"))),
            "work:docs.emptyPage" => Self::render_empty_page(said, editable, cx),
            "work:docs.emptyTab" => block.child(said).when(editable, |this| {
                this.child(
                    ghost_button("doc-write", Lucide::Pencil, tr(cx, "work:docs.writeIt"), cx)
                        .mt_3()
                        .border_1()
                        .border_color(border)
                        .cursor_pointer()
                        .on_click(|_, _, cx| {
                            Backend::core(cx).docs().start_edit();
                        })
                        .test_support(),
                )
            }),
            _ => block.child(said),
        }
    }

    /// La lista de lo que se lee en esa pestaña o esa página, con lo que
    /// se había bajado en ella. La de otra pestaña se guarda; la de una
    /// página sólo vive mientras se lee, así que cada página empieza arriba.
    fn reading_for(&mut self, node: &str, key: &str, stamp: u64, rows: usize, cx: &mut Context<Self>) -> ListState {
        if self.readings_of != node {
            self.readings.clear();
            self.readings_of = node.to_string();
        }
        self.readings.retain(|other, _| other == key || !other.starts_with("page-"));
        let reading = self.readings.entry(key.to_string()).or_insert_with(|| {
            // Medio alto de ventana de más por arriba y por abajo: lo que
            // está a punto de asomar ya está maquetado.
            let list = ListState::new(rows, ListAlignment::Top, px(480.));
            let view = cx.entity().downgrade();
            // El índice marca por dónde se va: al bajar hay que repintarlo.
            list.set_scroll_handler(move |_, _, cx| {
                let view = view.clone();
                cx.defer(move |cx| {
                    let _ = view.update(cx, |_, cx| cx.notify());
                });
            });
            Reading { list, stamp, rows }
        });
        if reading.stamp != stamp || reading.rows != rows {
            // El texto cambió debajo (se guardó, o alguien lo cambió): lo
            // medido ya no vale, pero se sigue por donde se iba.
            let top = reading.list.logical_scroll_top();
            reading.list.reset(rows);
            reading.list.scroll_to(top);
            (reading.stamp, reading.rows) = (stamp, rows);
        }
        reading.list.clone()
    }

    /// Por dónde va la lectura de esa pestaña o esa página: en qué fila
    /// y cuánto dentro de ella.
    #[cfg(test)]
    fn read_down_to(&self, key: &str) -> Option<(usize, f32)> {
        let top = self.readings.get(key)?.list.logical_scroll_top();
        Some((top.item_ix, f32::from(top.offset_in_item)))
    }

    /// «En esta página» (`DocToc.tsx`): las secciones de lo que se lee, al
    /// lado en una ventana grande. Pulsar una salta a ella, y va marcada
    /// la de la sección por la que va la lectura.
    fn render_contents(contents: Vec<DocHeading>, active: Option<u32>, list: ListState, rows: Rc<ReadingRows>, window: &Window, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground, border, primary) = (theme.muted_foreground, theme.foreground, theme.border, theme.primary);
        div()
            .id("doc-contents")
            .w(px(CONTENTS_WIDTH))
            // Un índice más alto que la ventana se desliza por dentro
            // (`max-h-[calc(100dvh-14rem)] overflow-y-auto`).
            .max_h((window.viewport_size().height - px(224.)).max(px(120.)))
            .overflow_y_scroll()
            .flex_none()
            .flex()
            .flex_col()
            .child(div().mb_2().text_size(px(11.)).text_color(muted).child(tr(cx, "work:docs.onThisPage").to_uppercase()))
            .child(div().flex().flex_col().gap_0p5().border_l_1().border_color(border).children(contents.into_iter().enumerate().map(|(index, heading)| {
                let on = active == Some(index as u32);
                let (list, rows) = (list.clone(), rows.clone());
                div()
                    .id(("doc-contents-entry", index))
                    // `-ml-px border-l-2`: su raya, encima de la del índice.
                    .ml(px(-1.))
                    .border_l_2()
                    .border_color(if on { primary } else { primary.opacity(0.) })
                    .py_1()
                    .pr_2()
                    .map(|this| if heading.level == 3 { this.pl_5() } else { this.pl_3() })
                    .leading_xs()
                    .text_color(if on { foreground } else { muted })
                    .when(on, |this| this.font_weight(FontWeight::MEDIUM))
                    .cursor_pointer()
                    .hover(move |style| style.text_color(foreground))
                    .on_click(move |_, window, _| {
                        let Some(row) = rows.row_of_piece(heading.piece as usize) else { return };
                        // Su título, arriba del todo (`scrollIntoView`).
                        list.scroll_to(ListOffset { item_ix: row, offset_in_item: px(rows.lead_of(row)) });
                        window.refresh();
                    })
                    .child(heading.text)
                    .test_support()
            })))
            .test_support()
    }
}

impl Render for DocsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted, border, primary) = (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.primary);
        let hover = crate::theme::ghost_hover(cx);
        let docs = Backend::snapshot(cx).read(cx).docs.clone();
        let here = locale(cx);
        let on_page = docs.on_page();
        let name = docs.target.as_ref().map(|target| target.name.clone()).unwrap_or_default();
        let node = docs.target.as_ref().map(|target| target.id.clone()).unwrap_or_default();
        let empty = docs.empty_reason();
        let has_text = !docs.body().is_empty();
        let decisions = docs.standing_decisions();
        let on_decisions = docs.tab == "decisions";
        let width = window.viewport_size().width;
        let (wide, hints) = (width >= px(WIDE_WINDOW), width >= px(HINTS_FROM));
        // Lo que se escribe, si se escribe algo: entonces el editor ocupa el
        // sitio de lo que se lee, con su propio pie.
        let editing = docs.edit.clone();
        let (editable, can_add) = (docs.editable(), docs.can_add_pages());
        // Quién responde del documento y cuándo se revisó, sobre las
        // pestañas: es del documento entero, no de lo que se esté mirando.
        let superadmin = Backend::snapshot(cx).read(cx).session.as_ref().is_some_and(|session| session.superadmin);
        let doc_header = docs.header(here, superadmin).map(|header| Self::render_header(header, cx));
        let contents = if wide && editing.is_none() { docs.contents() } else { Vec::new() };

        // La cabecera (`h-12 gap-2 px-4`): el icono, el nombre del nodo, su
        // id para copiar y, al final, las páginas, compartir y cerrar.
        let short: String = node.chars().take(8).collect();
        let (copy, close_hint, pages_hint) = (node.clone(), tr(cx, "work:docs.cancel"), tr(cx, "work:docs.pages"));
        let icon_button = move |id: &'static str, icon: Lucide| {
            div()
                .id(id)
                .size_6()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .hover(move |style| style.bg(hover))
                .child(Icon::new(icon).size_3p5())
        };
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(48.))
            .px_4()
            // Con una lista, la raya va bajo la fila de vistas.
            .child(Icon::new(Lucide::FileText).size_4().flex_none().text_color(muted))
            .child(div().id("doc-title").test_support().min_w_0().truncate().leading_sm().font_weight(FontWeight::MEDIUM).child(name.clone()))
            .child(
                crate::theme::id_chip("doc-id", short, false, cx)
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())))
                    .test_support(),
            )
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    // Las páginas que cuelgan de la portada, en su hoja.
                    .child(
                        icon_button("doc-pages", Lucide::ListTree)
                            .cursor_pointer()
                            .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.docs().show_pages().await }))
                            .tooltip(move |window, cx| Tooltip::new(pages_hint.clone()).build(window, cx))
                            .test_support(),
                    )
                    // Compartir, cuando hay documento que compartir.
                    .when(!docs.tabs.is_empty(), |this| {
                        this.child(ghost_button("doc-share", Lucide::Share2, tr(cx, "work:docs.share"), cx).h_6().gap_1p5().tooltip(not_yet(cx)).test_support())
                    })
                    .child(
                        icon_button("doc-close", Lucide::X)
                            .cursor_pointer()
                            // Con texto sin guardar, el core pregunta antes.
                            .on_click(|_, _, cx| Backend::core(cx).back_to_tasks())
                            .tooltip(move |window, cx| Tooltip::new(close_hint.clone()).build(window, cx))
                            .test_support(),
                    ),
            );

        // Las vistas de la lista, en su fila bajo el nombre (`ListToolbar`:
        // `h-9 gap-4 border-b px-4`), con la documentación tras un divisor.
        // En la de cualquier nodo, no sólo en la de una lista como en la app
        // de siempre: sin ellas, quien abría la documentación de un espacio
        // o de una carpeta no tenía cómo cambiar de vista más que cerrándola
        // (jose, 17-nov-2026: «nos abre la doc sin opciones de cambiar»).
        // A dónde vuelve cada una lo dice el core (`back_to_tasks`): a las
        // tareas de ese nodo, también si es una carpeta
        // (command-and-control-382).
        let views = Some({
            div().id("doc-views").h_9().flex().flex_none().items_stretch().gap_4().px_4().border_b_1().border_color(border).children(VIEWS.map(|(view, layout)| {
                let active = layout.is_none();
                div()
                    .flex()
                    .items_stretch()
                    .gap_4()
                    .when(active, |this| this.child(div().my_2p5().w(px(1.)).bg(border)))
                    .child(
                        div()
                            .id(SharedString::from(format!("view-{view}")))
                            .mb(px(-1.))
                            .flex()
                            .items_center()
                            .border_b_2()
                            .border_color(if active { primary } else { primary.opacity(0.) })
                            .leading_sm()
                            .whitespace_nowrap()
                            .text_color(if active { foreground } else { muted })
                            .when_some(layout, |this, layout| {
                                this.cursor_pointer().hover(move |style| style.text_color(foreground)).on_click(move |_, _, cx| {
                                    // De vuelta a las tareas del mismo nodo,
                                    // en la vista que se pulsó. Con texto
                                    // sin guardar, el core pregunta antes y
                                    // la vista no cambia hasta contestar.
                                    Backend::core(cx).back_to_tasks_in(layout);
                                })
                            })
                            .child(tr(cx, &format!("work:board.view.{view}")))
                            .test_support(),
                    )
            }))
        });

        // Las cuatro pestañas, siempre (`gap-4 border-b px-4 text-sm`, cada
        // una `border-b-2 py-2`): la vacía apagada, y su pista al lado
        // mientras quepa.
        let tabs = div().id("doc-tabs").flex().flex_none().gap_4().px_4().border_b_1().border_color(border).leading_sm().overflow_x_scroll().children(DOC_TABS.map(|key| {
            let active = docs.tab == key;
            let empty = docs.tab_is_empty(key);
            div()
                .id(SharedString::from(format!("doc-tab-{key}")))
                .test_support()
                .flex()
                .flex_none()
                .items_baseline()
                .gap_1p5()
                .whitespace_nowrap()
                .py_2()
                .border_b_2()
                .border_color(if active { primary } else { primary.opacity(0.) })
                .text_color(if active { foreground } else if empty { muted.opacity(0.6) } else { muted })
                .when(active, |this| this.font_weight(FontWeight::MEDIUM))
                .cursor_pointer()
                .on_click(move |_, _, cx| Backend::core(cx).docs().show_tab(key))
                .child(tr(cx, &format!("work:docs.{key}")))
                .when(hints, |this| {
                    this.child(div().text_size(px(11.)).font_weight(FontWeight::NORMAL).text_color(muted.opacity(0.7)).child(tr(cx, &format!("work:docs.{key}Hint"))))
                })
        }));

        // De qué es el texto: de una pestaña o de una página. El de una no
        // le presta su estado al de la otra.
        let reading_key = docs.page_id.as_ref().map(|id| format!("page-{id}")).unwrap_or_else(|| docs.tab.clone());
        // Lo que se lee, fila a fila. Una página lleva encima sus migas y
        // su título, y debajo —cuando ya llegó— lo que tiene dentro y lo
        // que la enlaza.
        let pieces = docs.pieces();
        let mut rows = Vec::new();
        if on_page {
            rows.push(ReadingRow::PageHead);
        }
        if empty.is_some() {
            rows.push(ReadingRow::Nothing);
        }
        rows.extend((0..pieces.len()).map(ReadingRow::Piece));
        rows.extend((0..decisions.len()).map(ReadingRow::Decision));
        if docs.page.is_some() {
            rows.push(ReadingRow::PageTail);
        }
        let leads = pieces.iter().enumerate().map(|(at, piece)| piece_lead(at.checked_sub(1).map(|before| &pieces[before]), piece)).collect();
        let reading = Rc::new(ReadingRows {
            rows,
            pieces,
            leads,
            decisions,
            node: node.clone(),
            name: name.clone(),
            key: reading_key.clone(),
            empty,
            editable,
            can_add,
            trail: docs.page_trail(here),
            title: docs.page_title(here),
            tail: docs.page.as_ref().map(|view| (view.page.id.clone(), docs.page_children(here), docs.page_backlinks(here))),
            wide,
            with_contents: !contents.is_empty(),
        });

        // El pie (`border-t px-4 py-2`): en el registro, apuntar una
        // decisión, que todavía no está en esta app; en lo demás, si hay
        // texto y quien mira puede escribir, editarlo.
        let edit_button = || {
            ghost_button("doc-edit", Lucide::Pencil, tr(cx, "work:docs.edit"), cx)
                .cursor_pointer()
                .on_click(|_, _, cx| {
                    Backend::core(cx).docs().start_edit();
                })
                .test_support()
        };
        let foot_bar = || div().id("doc-foot").flex().flex_none().items_center().gap_2().px_4().py_2().border_t_1().border_color(border);
        let foot = if docs.page.is_some() {
            // El de una página (`DocPageView.tsx`): editar si hay texto, su
            // historial y, al fondo, mandarla a la papelera.
            Some(
                foot_bar()
                    .when(has_text && editable, |this| this.child(edit_button()))
                    .child(
                        ghost_button("doc-page-history", Lucide::RotateCcwClock, tr(cx, "work:docs.history"), cx).h_6().gap_1p5().leading_xs().tooltip(not_yet(cx)).test_support(),
                    )
                    .child(
                        ghost_button("doc-page-share-link", Lucide::Link2, tr(cx, "work:docShare.action"), cx)
                            .h_6()
                            .gap_1p5()
                            .leading_xs()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.open_share_link(window, cx)))
                            .test_support(),
                    )
                    .child(ghost_button("doc-page-trash", Lucide::Trash, tr(cx, "work:docs.trashPage"), cx).ml_auto().text_color(muted).tooltip(not_yet(cx)).test_support()),
            )
        } else if on_page {
            None
        } else if on_decisions {
            Some(foot_bar().child(ghost_button("doc-record-decision", Lucide::Gavel, tr(cx, "work:decisions.record"), cx).tooltip(not_yet(cx)).test_support()))
        } else if has_text && editable {
            Some(foot_bar().child(edit_button()))
        } else {
            None
        };

        // La hoja «Páginas», encima de todo mientras está abierta.
        let sheet = docs.pages_shown.then(|| self.render_pages_sheet(&docs, window, cx));
        self.sheet_was_shown = docs.pages_shown;
        // El editor, en el sitio de lo que se lee y de su pie.
        let editor = editing.as_ref().map(|edit| self.render_editor(&docs, edit, window, cx));
        // Y la pregunta antes de perder texto, encima de todo.
        let question = editing.as_ref().and_then(|edit| edit.question(here));
        let asking = question.is_some();
        let question = question.map(|question| self.render_question(question, window, cx));
        if self.was_asking && !asking {
            // Contestada: el foco vuelve a la caja, si se sigue escribiendo.
            if let (Some(boxes), true) = (&self.boxes, editing.is_some()) {
                boxes.body.update(cx, |input, cx| input.focus(window, cx));
            }
        }
        self.was_asking = asking;
        // La lista de lo que se lee aquí, con lo que se había bajado. Sólo
        // mientras se lee: con el editor delante no se toca.
        let read = editing.is_none().then(|| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            docs.body().hash(&mut hasher);
            let list = self.reading_for(&node, &reading_key, hasher.finish(), reading.rows.len(), cx);
            // A donde lleva un enlace (una sección, un ancla), si lo que se
            // lee lo tiene. El enlace ya esperó a que llegara el texto: si
            // no está aquí, no va a estar, y el salto se da por dado.
            if docs.jump.is_some() {
                if let Some(row) = docs.jump_piece().and_then(|piece| reading.row_of_piece(piece as usize)) {
                    list.scroll_to(ListOffset { item_ix: row, offset_in_item: px(reading.lead_of(row)) });
                }
                Backend::core(cx).docs().jump_done();
            }
            // Qué sección se marca en el índice: dónde cae cada título
            // respecto a lo que se ve lo sabe la lista; cuál es, el core.
            let top = list.logical_scroll_top();
            let seen = list.viewport_bounds();
            let places: Vec<DocEntryPlace> = contents
                .iter()
                .map(|heading| match reading.row_of_piece(heading.piece as usize) {
                    Some(row) if row < top.item_ix => DocEntryPlace::Above,
                    Some(row) => match list.bounds_for_item(row) {
                        Some(bounds) => DocEntryPlace::At { top: f32::from(bounds.origin.y - seen.origin.y) + reading.lead_of(row) },
                        None => DocEntryPlace::Below,
                    },
                    None => DocEntryPlace::Below,
                })
                .collect();
            self.active_entry = cac_core::doc_active_entry(&places, f32::from(seen.size.height));
            // El índice se queda arriba mientras se baja (`sticky top-0`):
            // primero se va el aire de arriba, y él ya no sube más.
            let gone = if top.item_ix == 0 { f32::from(top.offset_in_item) } else { READING_PAD };
            (list, (READING_PAD - gone).max(0.))
        });
        // Los enlaces son de una página: si ya no se lee ésa, se cierra.
        if self.share_link.as_ref().is_some_and(|(share, _)| docs.page_id.as_deref() != Some(share.read(cx).page_id())) {
            self.share_link = None;
        }
        let share_link = self.share_link.as_ref().map(|(share, _)| div().id("doc-share-link-over").absolute().inset_0().child(share.clone()));
        // Un enlace que no se pudo abrir lo dice, abajo a la derecha.
        let link_failure = docs.link_failed.as_ref().map(|failure| self.render_link_failure(failure, cx));
        if link_failure.is_none() {
            self.timed_failure = None;
        }

        div()
            .id("docs")
            .test_support()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .child(header)
            .children(views)
            // Las pestañas son de la portada: una página no las lleva, ni
            // una página nueva que se escribe sobre ella.
            .when(!on_page && !editing.as_ref().is_some_and(DocEdit::has_title), |this| this.children(doc_header).child(tabs))
            .map(|this| match (editor, read) {
                (Some(editor), _) => this.child(editor),
                (None, Some((scroll, index_top))) => {
                    let rows = reading.clone();
                    let scroll_id = docs.page_id.as_ref().map(|id| SharedString::from(format!("doc-scroll-{id}"))).unwrap_or_else(|| "doc-scroll".into());
                    let index = (!contents.is_empty()).then(|| {
                        // Encima de la lista y en su sitio de siempre: la
                        // columna de la derecha del ancho de la página.
                        div().absolute().top(px(index_top)).left_0().right_0().px(px(READING_PAD)).child(
                            div()
                                .w_full()
                                .max_w(px(WIDE_PAGE_WIDTH))
                                .mx_auto()
                                .flex()
                                .justify_end()
                                .child(Self::render_contents(contents, self.active_entry, scroll.clone(), reading.clone(), window, cx)),
                        )
                    });
                    this.child(
                        div()
                            .id(scroll_id)
                            .flex_1()
                            .min_h_0()
                            .relative()
                            .child(list(scroll, move |row, window, cx| rows.render_row(row, window, cx)).size_full())
                            .children(index)
                            .test_support(),
                    )
                    .children(foot)
                }
                (None, None) => this,
            })
            .children(link_failure)
            .children(sheet)
            .children(question)
            .children(share_link)
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::DocsView;
    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, Request, ResponseTemplate};

    /// El resumen del espacio `a` tal como lo tiene el servidor: su texto,
    /// su versión y quién lo dejó así.
    #[derive(Default)]
    struct Kept {
        body: String,
        version: u32,
        by: &'static str,
        /// El runbook, que sólo se lee, y las secciones que el servidor
        /// reconoce en él.
        runbook: String,
        runbook_sections: Value,
        /// El documento en sí (su cabecera); `null` si no la tiene.
        head: Value,
    }

    fn doc(kept: &Kept) -> Value {
        json!({
            "doc": kept.head, "orgId": "o1", "decisions": [], "attachments": [],
            "tabs": [
                { "key": "overview", "body": kept.body, "bodyHash": format!("h{}", kept.version), "updatedByName": kept.by },
                { "key": "runbook", "body": kept.runbook, "sections": kept.runbook_sections },
            ],
        })
    }

    /// La tarea que nombra a la página `p1`.
    const TASK: &str = "0d6bf322-2267-4259-9b81-337575aca792";

    fn get_task() -> Mock {
        let mut detail: Value = serde_json::from_str(include_str!("../../core/tests/fixtures/task_detail.json")).unwrap();
        detail["task"]["id"] = json!(TASK);
        Mock::given(method("GET")).and(path(format!("/api/v1/tasks/{TASK}"))).respond_with(ok(detail))
    }

    fn page(id: &str, parent: Option<&str>, title: &str, body: &str) -> Value {
        json!({ "id": id, "parentId": parent, "title": title, "body": body, "bodyHash": "h1" })
    }

    /// Un servidor con la documentación del espacio `a`, que guarda como el
    /// de verdad: rechaza con 409, y lo que hay dentro, el guardado que no
    /// trae la versión vigente. Dos páginas: `p1`, escrita, y dentro `p2`,
    /// en blanco. Quien entra tiene ese papel en la organización.
    fn server(role: &str) -> (FakeServer, Arc<Mutex<Kept>>) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        let org = json!({ "id": "o1", "name": "G Studio", "slug": "g", "role": role, "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
        let space = json!({
            "id": "a", "orgId": "o1", "name": "Producto", "color": "#888", "folders": [], "people": [],
            "lists": [{ "id": "l-a", "name": "Lista", "openCount": 1, "taskCount": 1 }],
        });
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));

        let kept = Arc::new(Mutex::new(Kept { body: "## Qué es\n\nEl espacio.".into(), version: 1, by: "Luis Gil", ..Kept::default() }));
        let (read, written) = (kept.clone(), kept.clone());
        server.mount(get("/api/v1/docs/space/a").respond_with(move |_: &Request| ok(doc(&read.lock().unwrap()))));
        server.mount(Mock::given(method("PUT")).and(path("/api/v1/docs/space/a/tabs/overview")).respond_with(move |request: &Request| {
            let sent: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let mut kept = written.lock().unwrap();
            if sent["baseHash"].as_str() != Some(format!("h{}", kept.version).as_str()) {
                return ResponseTemplate::new(409).set_body_json(json!({ "success": false, "error": "doc-conflict", "data": doc(&kept) }));
            }
            (kept.body, kept.version, kept.by) = (sent["body"].as_str().unwrap_or_default().into(), kept.version + 1, "Ana Ruiz");
            ok(doc(&kept))
        }));
        let tree = json!([
            { "id": "p1", "title": "Arquitectura", "hasBody": true },
            { "id": "p2", "parentId": "p1", "title": "", "hasBody": false },
        ]);
        server.mount(get("/api/v1/docs/space/a/pages").respond_with(ok(tree)));
        let view = |page: Value| ok(json!({ "page": page, "breadcrumb": [], "children": [], "referencedFrom": [] }));
        // A `p1` la nombra una tarea.
        let cited = json!({
            "page": page("p1", None, "Arquitectura", "## Capas\n\nTres."), "breadcrumb": [], "children": [],
            "referencedFrom": [{ "kind": "task", "title": "Subir el backend", "where": "tasks", "link": format!("/tasks?task={TASK}") }],
        });
        server.mount(get("/api/v1/docs/space/a/pages/p1").respond_with(ok(cited)));
        server.mount(get("/api/v1/docs/space/a/pages/p2").respond_with(view(page("p2", Some("p1"), "", ""))));
        server.mount(get("/api/v1/docs/space/a/pages/new1").respond_with(view(page("new1", Some("p1"), "Los tipos", ""))));
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/docs/space/a/pages"))
                .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": page("new1", Some("p1"), "Los tipos", "") }))),
        );
        server.mount(
            Mock::given(method("PUT")).and(path_regex("^/api/v1/docs/space/a/pages/")).respond_with(ok(page("p2", Some("p1"), "", "guardada"))),
        );
        (server, kept)
    }

    /// La documentación del espacio `a` abierta en una ventana de ese ancho,
    /// con la vista sola: lo de alrededor (el árbol, el riel) no entra aquí.
    fn open(server: &FakeServer, width: f32, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<DocsView>) {
        server.install(cx);
        sign_in(cx);
        let (window, view) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(width), px(760.)) })),
                ..WindowOptions::default()
            };
            let (window, view) = gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|cx| {
                    // Quien pone esta vista (el espacio de trabajo) es quien
                    // la repinta cuando el core cambia; aquí va sola.
                    cx.observe(&Backend::snapshot(cx), |_, _, cx| cx.notify()).detach();
                    DocsView::new(cx)
                })
            })
            .expect("open test window");
            (gpui_kit::AnyWindowHandle::from(window), view)
        });
        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.orgs().load().await;
                core.tree().load("o1").await;
                core.docs().open("a").await;
            })
        });
        wait_until(cx, "llega la documentación del espacio", |cx| on_screen(window, "doc-body", cx));
        (window, view)
    }

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
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

    /// Pulsa dentro de una pieza, a esa distancia de su esquina: un enlace
    /// del texto no tiene id propio.
    fn click_at(window: AnyWindowHandle, id: &str, x: f32, y: f32, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click_at(id, gpui_kit::point(px(x), px(y)), cx);
        })
        .unwrap();
    }

    fn label_of(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).label().unwrap_or_default().to_string()
        })
        .unwrap()
    }

    /// Dónde está una pieza: su borde izquierdo y su ancho.
    fn span(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> (f32, f32) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let bounds = window.find(id).bounds();
            (f32::from(bounds.origin.x), f32::from(bounds.size.width))
        })
        .unwrap()
    }

    /// Escribe eso en la caja (o en el título), como quien la pulsa y teclea.
    fn type_in(window: AnyWindowHandle, id: &str, text: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
            window.input(text, cx);
        })
        .unwrap();
    }

    /// Lo que hay en la caja del texto y en la del título.
    fn boxes(view: &Entity<DocsView>, cx: &mut TestAppContext) -> (String, String) {
        cx.update(|cx| {
            let boxes = view.read(cx).boxes.as_ref().expect("el editor no se ha abierto nunca");
            (boxes.title.read(cx).value().to_string(), boxes.body.read(cx).value().to_string())
        })
    }

    /// El borrador que tiene el core, si se escribe algo.
    fn draft(cx: &mut TestAppContext) -> Option<cac_core::DocEdit> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).docs.edit.clone())
    }

    /// Lo que se le mandó al servidor que no fue leer: verbo, ruta y cuerpo.
    fn writes(server: &FakeServer) -> Vec<(String, String, Value)> {
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() != "GET")
            .map(|request| (request.method.to_string(), request.url.path().to_string(), serde_json::from_slice(&request.body).unwrap_or(Value::Null)))
            .collect()
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// «Compartir con enlace» de una página enseña sus enlaces vivos, crea
    /// uno con la duración elegida para esa página, deja en el portapapeles
    /// el enlace web y revoca preguntando antes.
    ///
    /// Roturas que atrapa: crear con otra duración, para otra página o en
    /// el documento de otro nodo; el enlace a la app, al de las llamadas
    /// o con el token fuera del `#`; no copiar nada; no decir que abre
    /// también las subpáginas; revocar sin preguntar o al cancelar; un enlace revocado
    /// que sigue en la lista, o que se lleva a los demás.
    #[gpui_kit::test]
    fn a_page_is_shared_by_a_web_link_and_revoked_after_asking(cx: &mut TestAppContext) {
        use wiremock::matchers::body_json;
        let (server, _) = server("member");
        let share = |id: &str, by: &str| {
            json!({ "id": id, "pageId": "p1", "expiresAt": "2026-12-24T15:00:00Z", "createdBy": "u2", "createdByName": by, "link": format!("{id}.firma") })
        };
        let shares = "/api/v1/docs/space/a/pages/p1/shares";
        server.mount(Mock::given(method("GET")).and(path(shares)).respond_with(ok(json!([share("s0", "Luis Gil")]))));
        server.mount(
            Mock::given(method("POST"))
                .and(path(shares))
                .and(body_json(json!({ "ttlHours": 720 })))
                .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": share("s1", "Ana Ruiz") }))),
        );
        server.mount(Mock::given(method("DELETE")).and(path("/api/v1/doc-shares/s1")).respond_with(ResponseTemplate::new(204)));
        let (window, _) = open(&server, 1400., cx);
        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open_page("p1").await }));
        wait_until(cx, "llega la página", |cx| on_screen(window, "doc-page-share-link", cx));

        click(window, "doc-page-share-link", cx);
        wait_until(cx, "sale el diálogo con el enlace vivo", |cx| on_screen(window, "doc-share-link-s0", cx));
        assert!(on_screen(window, "doc-share-scope", cx), "no dice que abre también las subpáginas");
        click(window, "doc-share-ttl-720", cx);
        click(window, "doc-share-create", cx);
        wait_until(cx, "el enlace nuevo sale en la lista", |cx| on_screen(window, "doc-share-link-s1", cx));
        let copied = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        assert_eq!(copied, format!("http://127.0.0.1:{}/app/shared#s1.firma", server.port()));
        assert!(on_screen(window, "doc-share-copied", cx));

        // Cancelar no revoca. Se le da tiempo a llegar a lo que se hubiera
        // mandado: lo que corre tras la pregunta sólo avanza si la prueba
        // mueve el ejecutor, y la petición sale por otro hilo.
        click(window, "doc-share-revoke-s1", cx);
        assert!(cx.has_pending_prompt(), "revocó sin preguntar");
        cx.simulate_prompt_answer("Cancelar");
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(300));
        cx.run_until_parked();
        assert_eq!(server.hits("/api/v1/doc-shares/s1"), 0, "revocó sin que se confirmara");
        assert!(on_screen(window, "doc-share-link-s1", cx));

        click(window, "doc-share-revoke-s1", cx);
        assert!(cx.has_pending_prompt(), "revocó sin preguntar");
        cx.simulate_prompt_answer("Revocar enlace");
        wait_until(cx, "se va de la lista", |cx| !on_screen(window, "doc-share-link-s1", cx));
        assert_eq!(server.hits("/api/v1/doc-shares/s1"), 1);
        assert!(on_screen(window, "doc-share-link-s0", cx), "se llevó también el otro enlace");
        stop(cx);
    }

    thread_local! {
        /// Las filas de lo que se lee que se han construido desde que se
        /// vació: lo que la lista maqueta, se vea o no.
        pub(super) static ROWS_BUILT: std::cell::RefCell<std::collections::HashSet<usize>> = Default::default();
    }

    /// De un documento largo sólo se construye lo que cabe en la ventana y
    /// un poco más, esté donde esté la lectura.
    ///
    /// Rotura que atrapa: volver a maquetar el documento entero en cada
    /// pintado, que en uno de 3.000 líneas eran 115 ms por cuadro.
    #[gpui_kit::test]
    fn only_what_fits_is_laid_out(cx: &mut TestAppContext) {
        ROWS_BUILT.with(|built| built.borrow_mut().clear());
        let (server, kept) = server("member");
        kept.lock().unwrap().body = (1..=80).map(|n| format!("## Sección {n}\n\n{}", filler("Texto", 3))).collect::<Vec<_>>().join("\n\n");
        let (window, _) = open(&server, 1400., cx);
        // Las que se han construido desde que se abrió: una fila ya medida
        // no se vuelve a construir en cada pintado, así que se cuentan todas.
        let built = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
            ROWS_BUILT.with(|built| built.borrow().len())
        };
        let at_the_top = built(cx);
        assert!((1..30).contains(&at_the_top), "arriba se construyen {at_the_top} de 80 secciones");
        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().follow_link("#seccion-41").await }));
        wait_until(cx, "se llega a la sección 41", |cx| on_screen(window, "doc-prose-40", cx));
        let in_the_middle = built(cx);
        assert!(in_the_middle < 60, "tras saltar a media altura se han construido {in_the_middle} de 80 secciones");
        stop(cx);
    }

    /// Unos cuantos párrafos, para que lo que se lee no quepa en la ventana.
    fn filler(what: &str, paragraphs: usize) -> String {
        (1..=paragraphs).map(|n| format!("{what} {n}, con algo de texto para que ocupe su renglón.")).collect::<Vec<_>>().join("\n\n")
    }

    /// Un resumen más alto que la ventana, con tres secciones.
    fn tall() -> String {
        format!(
            "{}\n\n## Uno\n\n{}\n\n## Dos\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n{}\n\n### Tres\n\n{}",
            filler("Antes", 9),
            filler("En uno", 14),
            filler("En dos", 14),
            filler("En tres", 30)
        )
    }

    /// A qué altura está una pieza y a cuál la caja en la que se lee.
    fn top_of(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> f32 {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            f32::from(window.find(id).bounds().origin.y)
        })
        .unwrap()
    }

    fn wheel(window: AnyWindowHandle, down: f32, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.scroll("doc-scroll", gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-down))), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn active(view: &Entity<DocsView>, window: AnyWindowHandle, cx: &mut TestAppContext) -> Option<u32> {
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
        cx.update(|cx| view.read(cx).active_entry)
    }

    /// Pulsar una entrada de «En esta página» deja su sección arriba del
    /// todo, y el índice marca la sección por la que va la lectura mientras
    /// se baja y se sube.
    ///
    /// Roturas que atrapa: una entrada que no hace nada al pulsarla (lo que
    /// había); saltar a otra sección que la pulsada, o dejar su título a
    /// media pantalla; un índice que no marca ninguna, o que se queda en la
    /// primera aunque se lea la última; marcar una sección a la que todavía
    /// no se ha llegado.
    #[gpui_kit::test]
    fn the_index_jumps_to_its_section_and_marks_where_the_reading_is(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        kept.lock().unwrap().body = tall();
        let (window, view) = open(&server, 1400., cx);
        assert!(on_screen(window, "doc-contents", cx));
        // Las piezas: lo de antes (0), «Uno» (1), «Dos» (2), su tabla (3),
        // su texto (4) y «Tres» (5).
        assert_eq!(active(&view, window, cx), None, "marca una sección antes de llegar a ninguna");
        assert!(!on_screen(window, "doc-prose-5", cx), "se pinta lo que queda muy por debajo de la ventana");

        let entry = |at: usize| gpui_kit::ElementId::NamedInteger("doc-contents-entry".into(), at as u64);
        cx.update_window(window, |_, window, cx| window.click(entry(1), cx)).unwrap();
        wait_until(cx, "salta a «Dos»", |cx| active(&view, window, cx) == Some(1));
        let (text, frame) = (top_of(window, "doc-prose-2", cx), top_of(window, "doc-scroll", cx));
        assert!((text - frame).abs() <= 1., "el título de «Dos» no quedó arriba del todo: a {} de la caja", text - frame);
        assert!(!on_screen(window, "doc-prose-0", cx), "lo que quedó muy arriba se sigue pintando");

        // Bajando por el texto de «Dos», con su título ya fuera de la
        // vista, sigue marcada «Dos».
        wheel(window, 320., cx);
        assert_eq!(cx.update(|cx| view.read(cx).read_down_to("overview")).map(|(row, _)| row), Some(4), "no se bajó al texto de «Dos»");
        assert_eq!(active(&view, window, cx), Some(1), "con el título fuera de la vista, su sección dejó de estar marcada");

        // Se sigue bajando con la rueda: al llegar «Tres» arriba, se marca.
        for _ in 0..40 {
            if active(&view, window, cx) == Some(2) {
                break;
            }
            wheel(window, 60., cx);
        }
        assert_eq!(active(&view, window, cx), Some(2), "bajando no llegó a marcar «Tres»");
        cx.update_window(window, |_, window, cx| window.click(entry(0), cx)).unwrap();
        wait_until(cx, "vuelve a «Uno»", |cx| active(&view, window, cx) == Some(0));
        let (text, frame) = (top_of(window, "doc-prose-1", cx), top_of(window, "doc-scroll", cx));
        assert!((text - frame).abs() <= 1.);
        // Y subiendo del todo, ninguna.
        wheel(window, -100_000., cx);
        assert_eq!(active(&view, window, cx), None);
        assert_eq!(cx.update(|cx| view.read(cx).read_down_to("overview")), Some((0, 0.)));
        stop(cx);
    }

    /// Cada pestaña guarda lo que se había bajado en ella, y una página
    /// empieza arriba.
    ///
    /// Roturas que atrapa: bajar en «Resumen», ir a «Runbook» y encontrarlo
    /// ya bajado (las cuatro compartían la posición); volver a «Resumen» y
    /// encontrarlo arriba; abrir una página a media altura por lo que se
    /// bajó en la portada.
    #[gpui_kit::test]
    fn each_tab_keeps_how_far_it_was_read(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        {
            let mut kept = kept.lock().unwrap();
            kept.body = tall();
            kept.runbook = format!("## Desplegar\n\n{}", filler("Paso", 60));
        }
        let (window, view) = open(&server, 1400., cx);
        let down = |key: &'static str, cx: &mut TestAppContext| cx.update(|cx| view.read(cx).read_down_to(key));

        wheel(window, 700., cx);
        let overview = down("overview", cx).expect("no se lee el resumen");
        assert_ne!(overview, (0, 0.), "la rueda no bajó nada");

        click(window, "doc-tab-runbook", cx);
        wait_until(cx, "se lee el runbook", |cx| on_screen(window, "doc-prose-0", cx) && down("runbook", cx).is_some());
        assert_eq!(down("runbook", cx), Some((0, 0.)), "el runbook heredó lo que se bajó en el resumen");
        wheel(window, 300., cx);
        let runbook = down("runbook", cx).unwrap();
        assert_ne!(runbook, (0, 0.));

        click(window, "doc-tab-overview", cx);
        wait_until(cx, "vuelve el resumen", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.tab == "overview"));
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
        assert_eq!(down("overview", cx), Some(overview), "el resumen no está donde se dejó");
        click(window, "doc-tab-runbook", cx);
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
        assert_eq!(down("runbook", cx), Some(runbook), "el runbook no está donde se dejó");

        // Una página empieza arriba, y al volver la portada sigue donde iba.
        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open_page("p1").await }));
        wait_until(cx, "llega la página", |cx| on_screen(window, "doc-page-children", cx));
        assert_eq!(down("page-p1", cx), Some((0, 0.)));
        cx.update(|cx| Backend::core(cx).docs().close_page());
        wait_until(cx, "vuelve la portada", |cx| on_screen(window, "doc-tab-runbook", cx));
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
        assert_eq!(down("runbook", cx), Some(runbook));
        assert_eq!(down("page-p1", cx), None, "la página guarda por dónde iba: al reabrirla no empezaría arriba");
        stop(cx);
    }

    /// Un enlace a una sección de otra pestaña cambia de pestaña y deja la
    /// sección arriba; un ancla del propio texto, igual. El salto se da una
    /// vez: después se puede seguir bajando.
    ///
    /// Roturas que atrapa: un enlace a una sección que abre su pestaña y se
    /// queda arriba; un salto que se repite en cada pintado y no deja
    /// moverse de ahí.
    #[gpui_kit::test]
    fn a_link_to_a_section_lands_on_it(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        {
            let mut kept = kept.lock().unwrap();
            kept.body = tall();
            kept.runbook = format!("{}\n\n## Restaurar\n\n{}", filler("Paso", 30), filler("Otro", 30));
            kept.runbook_sections = json!([{ "id": "s1", "heading": "Restaurar", "level": 2, "index": 0 }]);
        }
        let (window, view) = open(&server, 1400., cx);
        let follow = |href: &'static str, cx: &mut TestAppContext| cx.update(|cx| Backend::run(cx, move |core| async move { core.docs().follow_link(href).await }));
        let docs = |cx: &mut TestAppContext| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.clone());
        let down = |key: &'static str, cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
            cx.update(|cx| view.read(cx).read_down_to(key))
        };

        follow("/tasks?doc=space:a&tab=runbook&section=s1", cx);
        wait_until(cx, "se lee el runbook, en «Restaurar»", |cx| docs(cx).tab == "runbook" && down("runbook", cx).is_some_and(|(row, _)| row == 1));
        let (text, frame) = (top_of(window, "doc-prose-1", cx), top_of(window, "doc-scroll", cx));
        assert!((text - frame).abs() <= 1., "«Restaurar» no quedó arriba: a {}", text - frame);
        wait_until(cx, "el salto queda dado", |cx| docs(cx).jump.is_none());
        wheel(window, 200., cx);
        let after = down("runbook", cx).unwrap();
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
        assert_eq!(down("runbook", cx), Some(after), "el salto se repite y no deja bajar");
        assert_ne!(after.0 as f32 + after.1, 1., "la rueda no bajó de «Restaurar»");

        click(window, "doc-tab-overview", cx);
        follow("#dos", cx);
        wait_until(cx, "el ancla lleva a «Dos»", |cx| down("overview", cx).is_some_and(|(row, _)| row == 2));
        stop(cx);
    }

    /// Lo que se lee sigue al texto cuando cambia debajo, más corto o más
    /// largo.
    ///
    /// Rotura que atrapa: tras un cambio, la lista con las filas del texto
    /// de antes (faltan trozos, o se pide uno que ya no existe y la app se
    /// cae).
    #[gpui_kit::test]
    fn what_is_read_follows_the_text_when_it_changes(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        kept.lock().unwrap().body = tall();
        let (window, view) = open(&server, 1400., cx);
        let reopen = |cx: &mut TestAppContext| cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open("a").await }));
        let body = |cx: &mut TestAppContext| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.body().to_string());
        wheel(window, 900., cx);

        kept.lock().unwrap().body = "Ahora es corto.".into();
        reopen(cx);
        wait_until(cx, "llega el texto corto", |cx| body(cx) == "Ahora es corto.");
        assert!(on_screen(window, "doc-prose-0", cx) && !on_screen(window, "doc-prose-1", cx));

        kept.lock().unwrap().body = format!("{}\n\n## Al final\n\nLo último.", tall());
        reopen(cx);
        wait_until(cx, "llega el texto largo", |cx| body(cx).ends_with("Lo último."));
        let last = gpui_kit::ElementId::NamedInteger("doc-contents-entry".into(), 3);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(last, cx);
        })
        .unwrap();
        wait_until(cx, "se llega al último trozo", |cx| on_screen(window, "doc-prose-6", cx));
        assert_ne!(cx.update(|cx| view.read(cx).read_down_to("overview")), Some((0, 0.)));

        stop(cx);
    }

    /// La lectura de un nodo no es la de otro, aunque la pestaña se llame
    /// igual: la del nuevo empieza arriba. Y la del mismo, cuando su texto
    /// cambia, sigue por donde iba.
    ///
    /// Rotura que atrapa: abrir la documentación de otro nodo a la altura
    /// a la que se leía la del anterior (pasa cuando el documento nuevo
    /// llega sin que medie un pintado).
    #[gpui_kit::test]
    fn another_nodes_reading_starts_at_the_top(cx: &mut TestAppContext) {
        let (server, _) = server("member");
        let (_, view) = open(&server, 1400., cx);
        let top = |list: &gpui_kit::ListState| list.logical_scroll_top().item_ix;
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                let first = view.reading_for("a", "overview", 1, 9, cx);
                first.scroll_to(gpui_kit::ListOffset { item_ix: 5, offset_in_item: px(0.) });
                assert_eq!(top(&view.reading_for("a", "overview", 2, 9, cx)), 5, "al cambiar el texto no se siguió por donde se iba");
                assert_eq!(top(&view.reading_for("b", "overview", 3, 9, cx)), 0, "el otro nodo heredó lo bajado");
            })
        });
        stop(cx);
    }

    /// Entre un trozo de prosa y uno ancho va el hueco de siempre, y el
    /// primero no lleva nada encima.
    ///
    /// Rotura que atrapa: una tabla pegada al párrafo de antes, o el título
    /// que la sigue separado como de un párrafo.
    #[test]
    fn a_wide_piece_keeps_its_gap() {
        let pieces = cac_core::doc_pieces("Texto.\n\n| a |\n| - |\n\n## Título\n\nMás.", &[]);
        let leads: Vec<f32> = pieces.iter().enumerate().map(|(at, piece)| super::piece_lead(at.checked_sub(1).map(|before| &pieces[before]), piece)).collect();
        assert_eq!(leads, [0., 12., 12.]);
    }

    /// Lo que se lee partido en trozos ocupa lo mismo que pintado de una
    /// pieza: el aire entre un título y lo de antes es el que le ponía el
    /// pintor de markdown.
    ///
    /// Roturas que atrapa: títulos pegados al párrafo de antes, o con más
    /// aire que antes, al partir el texto en cada título; y un cambio de
    /// esas medidas en GPUI Kit al subir de versión, que aquí habría que
    /// seguir.
    #[gpui_kit::test]
    fn split_at_its_headings_the_text_takes_the_room_it_took_whole(cx: &mut TestAppContext) {
        struct Both(String);
        impl gpui_kit::Render for Both {
            fn render(&mut self, _: &mut gpui_kit::Window, cx: &mut gpui_kit::Context<Self>) -> impl gpui_kit::IntoElement {
                use gpui_kit::*;
                let column = |id: &'static str| div().id(id).w(px(640.)).flex().flex_col().text_size(px(15.)).line_height(relative(1.75));
                let pieces = cac_core::doc_pieces(&self.0, &[]);
                let split = pieces.iter().enumerate().map(|(at, piece)| {
                    let lead = super::piece_lead(at.checked_sub(1).map(|before| &pieces[before]), piece);
                    div().pt(px(lead)).child(DocsView::doc_text(format!("split-{at}").into(), piece.text.clone(), super::reading_style(cx))).into_any_element()
                });
                div()
                    .flex()
                    .items_start()
                    .gap_4()
                    .child(column("whole").child(DocsView::doc_text("whole-text".into(), self.0.clone(), super::reading_style(cx))).test_support())
                    .child(column("split").children(split).test_support())
            }
        }
        let server = FakeServer::start();
        server.install(cx);
        let text = "Un párrafo.\n\n# Uno\n\nTexto.\n\n## Dos\n### Tres\n\n- un punto\n- otro\n\n### Cuatro\n\nMás texto.\n\n---\n\n## Cinco\n\n---\n\n### Seis\n\n#### Siete\n\nY el final.";
        assert!(cac_core::doc_pieces(text, &[]).len() >= 8, "el texto no se partió");
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1400.), px(1600.)) })),
                ..WindowOptions::default()
            };
            let (window, _) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Both(text.to_string()))).expect("open test window");
            gpui_kit::AnyWindowHandle::from(window)
        });
        let tall = |id: &'static str, cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                f32::from(window.find(id).bounds().size.height)
            })
            .unwrap()
        };
        let (whole, split) = (tall("whole", cx), tall("split", cx));
        assert!(whole > 400., "el texto entero mide {whole}");
        assert!((whole - split).abs() <= 0.5, "de una pieza mide {whole} y a trozos {split}");
    }

    /// Sobre las pestañas va quién responde del documento, cuándo se
    /// revisó, la revisión pedida y la línea fijada; si lleva sin revisar,
    /// el aviso. Firmar sólo se le pinta a quien puede. Una página no la
    /// lleva, ni un nodo sin documento.
    ///
    /// Roturas que atrapa: un documento sin su cabecera; el botón de firmar
    /// a quien el servidor se lo va a rechazar; la cabecera del documento
    /// repetida sobre cada página.
    #[gpui_kit::test]
    fn the_header_of_the_document_goes_over_its_tabs(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        let (window, _) = open(&server, 1400., cx);
        assert!(!on_screen(window, "doc-header", cx), "un documento sin cabecera pinta una");

        kept.lock().unwrap().head = json!({
            "id": "d1", "updatedAt": "2026-06-01T12:00:00Z", "maintainerId": "u1", "maintainerName": "Ana Ruiz", "stale": true,
            "reviewRequestedAt": "2026-11-16T12:00:00Z", "reviewRequestedByName": "Luis Gil", "reviewRequestNote": "Cambié el runbook",
            "pinnedLine": "Backend antes que app",
        });
        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open("a").await }));
        wait_until(cx, "llega la cabecera", |cx| on_screen(window, "doc-header", cx));
        assert_eq!(label_of(window, "doc-owner", cx), "Ana Ruiz");
        assert_eq!(label_of(window, "doc-reviewed", cx), "Sin revisar nunca");
        assert_eq!(label_of(window, "doc-review-requested", cx), "Revisión pedida por Luis Gil · 16/11/26");
        assert_eq!(label_of(window, "doc-pinned", cx), "Backend antes que app");
        assert!(label_of(window, "doc-stale", cx).starts_with("Escrito hace"));
        // Encima de las pestañas, no debajo.
        let (header, tabs) = (top_of(window, "doc-header", cx), top_of(window, "doc-tab-overview", cx));
        assert!(header < tabs, "la cabecera está a {header} y las pestañas a {tabs}");
        assert!(on_screen(window, "doc-reassign-owner", cx));
        assert!(!on_screen(window, "doc-mark-reviewed", cx), "se le ofrece firmar a quien no puede");

        // A un superadmin sí se le pinta firmar.
        cx.update(|cx| {
            let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "superadmin": true })).unwrap();
            Backend::core(cx).sign_in(ana, "access".into(), "refresh".into()).unwrap();
        });
        wait_until(cx, "se pinta firmar", |cx| on_screen(window, "doc-mark-reviewed", cx));

        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open_page("p1").await }));
        wait_until(cx, "llega la página", |cx| on_screen(window, "doc-page-children", cx));
        assert!(!on_screen(window, "doc-header", cx), "la cabecera del documento sigue sobre una página");
        stop(cx);
    }

    /// Junto al título de una sección que sostiene alguna decisión va su
    /// marca, con cuántas; las demás no la llevan.
    ///
    /// Roturas que atrapa: una sección con decisiones sin su marca; una
    /// marca en cada título, tenga o no decisiones.
    #[gpui_kit::test]
    fn a_section_backed_by_decisions_carries_its_mark(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        {
            let mut kept = kept.lock().unwrap();
            kept.runbook = "## Desplegar\n\nCon cuidado.\n\n## Restaurar\n\nDel respaldo.".into();
            let decision = |id: &str| json!({ "id": id, "title": "Sin colas", "ownerKind": "space", "ownerId": "a", "decidedAt": "2026-11-14T18:00:00Z" });
            kept.runbook_sections = json!([
                { "id": "s1", "heading": "Desplegar", "level": 2, "index": 0, "decisions": [decision("d1"), decision("d2")] },
                { "id": "s2", "heading": "Restaurar", "level": 2, "index": 1 },
            ]);
        }
        let (window, _) = open(&server, 1400., cx);
        click(window, "doc-tab-runbook", cx);
        wait_until(cx, "se lee el runbook", |cx| on_screen(window, "doc-prose-1", cx));
        assert_eq!(label_of(window, "doc-section-mark-0", cx), "2 decisiones sostienen esta sección");
        assert!(on_screen(window, "doc-section-decisions-0", cx));
        assert!(!on_screen(window, "doc-section-mark-1", cx), "una sección sin decisiones lleva marca");
        // Tras el título, en su renglón.
        let (mark, text) = (span(window, "doc-section-mark-0", cx), span(window, "doc-prose-0", cx));
        assert!(mark.0 > text.0 + 60. && mark.0 < text.0 + 200., "la marca está a {} del borde del texto", mark.0 - text.0);
        stop(cx);
    }

    /// Un enlace escrito en el texto, al pulsarlo, va a donde apunta: el de
    /// una página de este documento la abre, y el que no se sabe abrir lo
    /// dice en pantalla, con su equis para quitarlo. Y lo que nombra a una
    /// página, al pie, se abre también: aquí, una tarea.
    ///
    /// Roturas que atrapa: un enlace del texto que no hace nada al pulsarlo
    /// (el clic no llega al core); uno roto que tampoco dice nada; un aviso
    /// que no se puede quitar; una tarea que enlaza a la página pintada
    /// como algo que no se pulsa.
    #[gpui_kit::test]
    fn a_link_in_the_text_goes_where_it_points(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        kept.lock().unwrap().body = "[Arquitectura](/tasks?doc=space:a&page=p1)\n\n[Notas](/notes/n1)".into();
        let (window, _) = open(&server, 1400., cx);
        let docs = |cx: &mut TestAppContext| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.clone());

        // El segundo párrafo: un renglón (26 px) y su hueco más abajo.
        assert!(!on_screen(window, "doc-link-failed", cx));
        click_at(window, "doc-prose-0", 10., 52., cx);
        wait_until(cx, "se dice que no se pudo abrir", |cx| on_screen(window, "doc-link-failed", cx));
        assert_eq!(label_of(window, "doc-link-failed", cx), "No se pudo abrir el enlace");
        assert_eq!(label_of(window, "doc-link-failed-href", cx), "/notes/n1");
        assert!(!docs(cx).on_page());
        click(window, "doc-link-failed-close", cx);
        wait_until(cx, "la equis quita el aviso", |cx| !on_screen(window, "doc-link-failed", cx));

        click_at(window, "doc-prose-0", 10., 13., cx);
        wait_until(cx, "se abre la página", |cx| docs(cx).page_id.as_deref() == Some("p1") && on_screen(window, "doc-page-backlinks", cx));
        assert!(!on_screen(window, "doc-link-failed", cx));

        server.mount(get_task());
        click(window, "doc-page-backlink-0", cx);
        wait_until(cx, "se abre la tarea que la nombra", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.as_deref() == Some(TASK)));
        assert_eq!(docs(cx).page_id.as_deref(), Some("p1"), "el documento sigue debajo");
        stop(cx);
    }

    /// «Editar» abre la caja con el texto que había y, al lado, el
    /// documento pintado; lo que se teclea llega al borrador (que es lo que
    /// se pinta al lado) y «Guardar» lo manda con su versión y vuelve a leer.
    ///
    /// Roturas que atrapa: un «Editar» que no hace nada; una caja en blanco
    /// sobre un documento escrito (guardarla lo borraría); teclear sin que
    /// el borrador se entere (la vista previa no cambia y «Guardar» sigue
    /// apagado); la caja y la vista previa una encima de otra en una
    /// ventana ancha; un «Guardar» que no guarda.
    #[gpui_kit::test]
    fn editing_shows_the_box_beside_the_document_and_saving_sends_what_was_typed(cx: &mut TestAppContext) {
        let (server, _) = server("member");
        let (window, view) = open(&server, 1400., cx);

        click(window, "doc-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-editor", cx));
        assert!(!on_screen(window, "doc-body", cx), "lo que se lee sigue debajo del editor");
        assert_eq!(boxes(&view, cx).1, "## Qué es\n\nEl espacio.");
        assert!(!on_screen(window, "doc-editing-what", cx), "una pestaña ya dice cuál es en su fila");
        assert!(!on_screen(window, "doc-edit-status", cx), "dice «sin guardar» sin haber tocado nada");

        // Lado a lado: la caja a la izquierda, lo pintado a la derecha.
        let (box_left, box_width) = span(window, "doc-edit-box", cx);
        let (preview_left, preview_width) = span(window, "doc-edit-preview", cx);
        assert!(box_left + box_width <= preview_left + 1., "la vista previa no está al lado de la caja");
        assert!(box_width > 300. && preview_width > 300., "uno de los dos se quedó sin sitio: {box_width} y {preview_width}");
        assert!(on_screen(window, "doc-edit-preview-text", cx), "al lado no hay nada pintado");
        assert!(!on_screen(window, "doc-edit-switch", cx), "con sitio para los dos no hace falta elegir");
        // La caja llena el alto: es para escribir un documento, no una nota.
        // Se mide el campo, no su marco: un campo de dos filas dentro de un
        // marco alto enseñaría dos renglones del documento.
        let tall = cx.update(|cx| f32::from(view.read(cx).boxes.as_ref().unwrap().body.read(cx).input_bounds().size.height));
        assert!(tall > 400., "el campo mide {tall} de alto en una ventana de 760");

        type_in(window, "doc-edit-input", "X", cx);
        wait_until(cx, "lo tecleado llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.body.contains('X')));
        assert_eq!(label_of(window, "doc-edit-status", cx), "Cambios sin guardar");
        // La siguiente tecla cae detrás de la anterior: pintar no vuelve a
        // poner el texto en la caja ni le mueve el cursor.
        cx.update_window(window, |_, window, cx| window.input("Y", cx)).unwrap();
        wait_until(cx, "la segunda tecla llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.body.contains('Y')));
        let typed = boxes(&view, cx).1;
        assert!(typed.contains("XY"), "la segunda tecla no cayó detrás de la primera: {typed:?}");
        assert_eq!(draft(cx).map(|edit| edit.body), Some(typed.clone()), "el borrador no es lo que hay en la caja");

        click(window, "doc-save", cx);
        wait_until(cx, "guardado, se vuelve a leer", |cx| draft(cx).is_none() && on_screen(window, "doc-body", cx));
        assert_eq!(writes(&server), [("PUT".to_string(), "/api/v1/docs/space/a/tabs/overview".to_string(), json!({ "body": typed, "baseHash": "h1" }))]);
        assert!(!on_screen(window, "doc-editor", cx));
        stop(cx);
    }

    /// En una ventana estrecha se ve la caja o la vista previa, con su
    /// conmutador; lo escrito sigue ahí al volver.
    ///
    /// Roturas que atrapa: partir en dos una ventana donde no caben; un
    /// conmutador que no cambia nada; perder lo tecleado al ir a mirar cómo
    /// queda.
    #[gpui_kit::test]
    fn a_narrow_window_shows_the_box_or_the_preview(cx: &mut TestAppContext) {
        let (server, _) = server("member");
        let (window, view) = open(&server, 800., cx);
        click(window, "doc-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-editor", cx));

        assert!(on_screen(window, "doc-edit-box", cx) && !on_screen(window, "doc-edit-preview", cx));
        type_in(window, "doc-edit-input", "X", cx);
        wait_until(cx, "lo tecleado llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.is_dirty()));
        let typed = boxes(&view, cx).1;

        click(window, "doc-edit-show-preview", cx);
        wait_until(cx, "se ve la vista previa", |cx| on_screen(window, "doc-edit-preview", cx));
        assert!(!on_screen(window, "doc-edit-box", cx));
        assert!(on_screen(window, "doc-edit-preview-text", cx));

        click(window, "doc-edit-show-box", cx);
        wait_until(cx, "vuelve la caja", |cx| on_screen(window, "doc-edit-box", cx));
        assert!(!on_screen(window, "doc-edit-preview", cx));
        assert_eq!(boxes(&view, cx).1, typed);

        // Ctrl+S (Cmd en un Mac) guarda desde la caja.
        cx.update_window(window, |_, window, cx| {
            window.click("doc-edit-input", cx);
            window.press("secondary-s", cx);
        })
        .unwrap();
        wait_until(cx, "guardado con el teclado", |cx| draft(cx).is_none());
        assert_eq!(writes(&server).len(), 1);
        assert_eq!(writes(&server)[0].2["body"], typed);
        stop(cx);
    }

    /// Salir con texto sin guardar pregunta en pantalla: «Seguir editando»
    /// y Escape dejan todo como estaba; «Descartar» se va. Sin cambios,
    /// «Cancelar» cierra sin preguntar.
    ///
    /// Roturas que atrapa: cambiar de pestaña y perder lo escrito sin
    /// aviso; una pregunta que no se pinta (el clic parece no hacer nada);
    /// un Escape que descarta; un «Descartar» que no se va; preguntar a
    /// quien no tocó nada.
    #[gpui_kit::test]
    fn leaving_with_unsaved_text_asks_on_screen(cx: &mut TestAppContext) {
        let (server, _) = server("member");
        let (window, view) = open(&server, 1400., cx);
        let tab = |cx: &mut TestAppContext| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.tab.clone());

        click(window, "doc-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-editor", cx));
        click(window, "doc-cancel", cx);
        wait_until(cx, "sin cambios, cancelar cierra", |cx| !on_screen(window, "doc-editor", cx));
        assert!(!on_screen(window, "doc-ask", cx), "preguntó a quien no tocó nada");

        click(window, "doc-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-editor", cx));
        type_in(window, "doc-edit-input", "X", cx);
        wait_until(cx, "lo tecleado llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.is_dirty()));
        let typed = boxes(&view, cx).1;

        click(window, "doc-tab-runbook", cx);
        wait_until(cx, "pregunta antes de irse", |cx| on_screen(window, "doc-ask", cx));
        assert_eq!(label_of(window, "doc-ask-title", cx), "¿Descartar lo que has escrito?");
        assert_eq!(tab(cx), "overview", "se fue sin esperar la respuesta");
        click(window, "doc-ask-no", cx);
        wait_until(cx, "la pregunta se quita", |cx| !on_screen(window, "doc-ask", cx));
        assert_eq!((tab(cx), boxes(&view, cx).1), ("overview".to_string(), typed.clone()));
        // Y se sigue escribiendo sin tener que pulsar la caja otra vez.
        cx.update_window(window, |_, window, cx| window.input("Z", cx)).unwrap();
        wait_until(cx, "lo siguiente cae en la caja", |cx| draft(cx).is_some_and(|edit| edit.body.contains('Z')));
        let typed = boxes(&view, cx).1;

        // La equis de la pregunta también es un no.
        click(window, "doc-cancel", cx);
        wait_until(cx, "cancelar pregunta", |cx| on_screen(window, "doc-ask", cx));
        click(window, "doc-ask-close", cx);
        wait_until(cx, "la equis la quita", |cx| !on_screen(window, "doc-ask", cx));
        assert_eq!(boxes(&view, cx).1, typed, "la equis descartó lo escrito");

        click(window, "doc-cancel", cx);
        wait_until(cx, "cancelar también pregunta", |cx| on_screen(window, "doc-ask", cx));
        cx.update_window(window, |_, window, cx| window.press("enter", cx)).unwrap();
        cx.run_until_parked();
        assert!(on_screen(window, "doc-ask", cx) && draft(cx).is_some(), "Enter descartó lo escrito");
        cx.update_window(window, |_, window, cx| window.press("escape", cx)).unwrap();
        wait_until(cx, "Escape contesta que no", |cx| !on_screen(window, "doc-ask", cx));
        assert_eq!(boxes(&view, cx).1, typed, "Escape descartó lo escrito");
        assert!(on_screen(window, "doc-editor", cx));

        click(window, "doc-tab-runbook", cx);
        wait_until(cx, "pregunta otra vez", |cx| on_screen(window, "doc-ask", cx));
        click(window, "doc-ask-yes", cx);
        wait_until(cx, "con un sí, se va", |cx| tab(cx) == "runbook" && !on_screen(window, "doc-editor", cx));
        assert!(writes(&server).is_empty(), "descartar guardó algo");
        stop(cx);
    }

    /// Si alguien guardó en medio, se dice en pantalla, lo escrito sigue en
    /// la caja, al lado se puede mirar lo que hay ahora, y guardar encima
    /// pregunta antes.
    ///
    /// Roturas que atrapa: un choque que no se ve (parece guardado); perder
    /// lo tecleado; un «ver lo que hay ahora» que sigue enseñando lo propio
    /// sin decir de quién es; pisar al otro sin preguntar.
    #[gpui_kit::test]
    fn a_conflict_is_said_and_the_text_stays(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        let (window, view) = open(&server, 1400., cx);
        click(window, "doc-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-editor", cx));
        type_in(window, "doc-edit-input", "X", cx);
        wait_until(cx, "lo tecleado llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.is_dirty()));
        let typed = boxes(&view, cx).1;
        *kept.lock().unwrap() = Kept { body: "lo de Luis".into(), version: 2, by: "Luis Gil", ..Kept::default() };

        assert!(!on_screen(window, "doc-conflict", cx));
        click(window, "doc-save", cx);
        wait_until(cx, "se dice el choque", |cx| on_screen(window, "doc-conflict", cx));
        assert_eq!(boxes(&view, cx).1, typed, "lo tecleado no sobrevivió al choque");
        assert_eq!(kept.lock().unwrap().body, "lo de Luis");
        assert_eq!(label_of(window, "doc-edit-preview-label", cx), "Vista previa");

        // «Guardar» está apagado: pulsarlo no manda nada más.
        click(window, "doc-save", cx);
        cx.run_until_parked();
        assert_eq!(writes(&server).len(), 1);

        click(window, "doc-conflict-see", cx);
        wait_until(cx, "al lado, lo que hay guardado", |cx| label_of(window, "doc-edit-preview-label", cx) == "Lo que hay guardado ahora");
        assert_eq!(boxes(&view, cx).1, typed);

        click(window, "doc-conflict-overwrite", cx);
        wait_until(cx, "guardar encima pregunta", |cx| on_screen(window, "doc-ask", cx));
        assert_eq!(label_of(window, "doc-ask-title", cx), "¿Guardar tu texto encima del que hay?");
        assert_eq!(writes(&server).len(), 1, "pisó al otro sin esperar la respuesta");
        click(window, "doc-ask-yes", cx);
        wait_until(cx, "guardado encima", |cx| draft(cx).is_none());
        assert_eq!(writes(&server)[1].2, json!({ "body": typed, "baseHash": "h2" }));
        assert_eq!(kept.lock().unwrap().body, typed);
        stop(cx);
    }

    /// Si el documento cambia debajo mientras se escribe —lo guardó otra
    /// persona, o un agente—, al ponerse al día sale el aviso de choque y lo
    /// que hay en la caja no se toca.
    ///
    /// Roturas que atrapa: el texto de la caja cambiado por el del servidor
    /// con el cursor dentro; ningún aviso hasta que se intenta guardar.
    #[gpui_kit::test]
    fn a_change_underneath_is_said_without_touching_the_box(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        let (window, view) = open(&server, 1400., cx);
        click(window, "doc-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-editor", cx));
        type_in(window, "doc-edit-input", "X", cx);
        wait_until(cx, "lo tecleado llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.is_dirty()));
        let typed = boxes(&view, cx).1;

        *kept.lock().unwrap() = Kept { body: "lo de Luis".into(), version: 2, by: "Luis Gil", ..Kept::default() };
        assert!(!on_screen(window, "doc-conflict", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().refresh().await }));
        wait_until(cx, "se dice que cambió debajo", |cx| on_screen(window, "doc-conflict", cx));
        assert_eq!(boxes(&view, cx).1, typed, "ponerse al día tocó lo escrito");
        assert!(on_screen(window, "doc-editor", cx));
        assert!(writes(&server).is_empty());
        // Y lo que hay guardado se puede mirar al lado.
        click(window, "doc-conflict-see", cx);
        wait_until(cx, "al lado, lo que hay guardado", |cx| label_of(window, "doc-edit-preview-label", cx) == "Lo que hay guardado ahora");
        assert_eq!(boxes(&view, cx).1, typed);
        stop(cx);
    }

    /// Tras un choque, «descartar el mío y recargar» pregunta y, con un sí,
    /// deja a la vista lo que hay guardado.
    ///
    /// Rotura que atrapa: un botón que tira lo escrito sin preguntar, o que
    /// pregunta y luego no hace nada.
    #[gpui_kit::test]
    fn loading_theirs_asks_and_then_reads_what_is_saved(cx: &mut TestAppContext) {
        let (server, kept) = server("member");
        let (window, view) = open(&server, 1400., cx);
        click(window, "doc-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-editor", cx));
        type_in(window, "doc-edit-input", "X", cx);
        wait_until(cx, "lo tecleado llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.is_dirty()));
        let typed = boxes(&view, cx).1;
        *kept.lock().unwrap() = Kept { body: "lo de Luis".into(), version: 2, by: "Luis Gil", ..Kept::default() };
        click(window, "doc-save", cx);
        wait_until(cx, "se dice el choque", |cx| on_screen(window, "doc-conflict", cx));

        click(window, "doc-conflict-load", cx);
        wait_until(cx, "descartar pregunta", |cx| on_screen(window, "doc-ask", cx));
        assert_eq!(boxes(&view, cx).1, typed, "lo tiró sin esperar la respuesta");
        click(window, "doc-ask-yes", cx);
        // Cerrar el editor es en el acto; lo guardado llega después, de otro
        // hilo: se espera a ello y no a lo primero.
        wait_until(cx, "se lee lo que hay guardado", |cx| {
            draft(cx).is_none() && on_screen(window, "doc-body", cx) && cx.update(|cx| Backend::snapshot(cx).read(cx).docs.body() == "lo de Luis")
        });
        stop(cx);
    }

    /// Un guardado que falla lo dice junto a los botones y deja la caja
    /// como estaba.
    ///
    /// Rotura que atrapa: un fallo mudo, con el editor abierto y sin saber
    /// si se guardó.
    #[gpui_kit::test]
    fn a_failed_save_is_said_next_to_the_buttons(cx: &mut TestAppContext) {
        let (server, _) = server("member");
        let (window, view) = open(&server, 1400., cx);
        click(window, "doc-tab-runbook", cx);
        wait_until(cx, "el runbook vacío ofrece escribirlo", |cx| on_screen(window, "doc-write", cx));
        click(window, "doc-write", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-editor", cx));
        type_in(window, "doc-edit-input", "pasos", cx);
        wait_until(cx, "lo tecleado llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.body == "pasos"));

        // El servidor de mentira no sabe guardar el runbook: contesta 404.
        assert!(!on_screen(window, "doc-edit-error", cx));
        click(window, "doc-save", cx);
        wait_until(cx, "se dice el fallo", |cx| on_screen(window, "doc-edit-error", cx));
        assert_eq!(label_of(window, "doc-edit-error", cx), "No se pudo guardar esta sección");
        assert_eq!(boxes(&view, cx).1, "pasos");
        assert!(on_screen(window, "doc-editor", cx));
        stop(cx);
    }

    /// Quien sólo puede mirar no ve cómo escribir: ni «Editar», ni
    /// «Escribirlo», ni las plantillas, ni los «+» de las páginas.
    ///
    /// Rotura que atrapa: ofrecerle a un `viewer` un editor cuyo guardado
    /// no le corresponde.
    #[gpui_kit::test]
    fn a_viewer_sees_no_way_to_write(cx: &mut TestAppContext) {
        let (server, _) = server("viewer");
        let (window, _) = open(&server, 1400., cx);
        assert!(!on_screen(window, "doc-edit", cx));
        click(window, "doc-tab-runbook", cx);
        wait_until(cx, "el runbook vacío lo dice", |cx| on_screen(window, "doc-empty", cx));
        assert!(!on_screen(window, "doc-write", cx));

        click(window, "doc-pages", cx);
        wait_until(cx, "sale la hoja con sus páginas", |cx| on_screen(window, "doc-page-row-p1", cx));
        assert!(!on_screen(window, "doc-pages-new", cx) && !on_screen(window, "doc-page-new-p1", cx));
        assert!(on_screen(window, "doc-page-menu-p1", cx), "lo que sólo está pintado no se quita");

        // Una página escrita, sin «Editar»; lo que sólo está pintado sigue.
        click(window, "doc-page-row-p1", cx);
        wait_until(cx, "llega la página escrita", |cx| on_screen(window, "doc-page-children", cx) && on_screen(window, "doc-body", cx));
        assert!(!on_screen(window, "doc-edit", cx) && !on_screen(window, "doc-page-new-child", cx));
        assert!(on_screen(window, "doc-page-history", cx));

        click(window, "doc-pages", cx);
        wait_until(cx, "sale la hoja otra vez", |cx| on_screen(window, "doc-page-row-p1", cx));
        click(window, "doc-page-toggle-p1", cx);
        wait_until(cx, "se despliega", |cx| on_screen(window, "doc-page-row-p2", cx));
        click(window, "doc-page-row-p2", cx);
        wait_until(cx, "la página vacía lo dice", |cx| on_screen(window, "doc-page-children", cx) && on_screen(window, "doc-empty", cx));
        for id in ["doc-write", "doc-page-template-component", "doc-page-new-child", "doc-edit"] {
            assert!(!on_screen(window, id, cx), "un viewer ve {id}");
        }
        stop(cx);
    }

    /// El «+» de una página en la hoja abre el editor de una página nueva
    /// dentro de ella, sin crear nada; al guardar se crea con su título y
    /// de quién cuelga.
    ///
    /// Roturas que atrapa: un «+» que abre la página en vez de crear dentro;
    /// crearla al pulsar; la hoja encima del editor; un título que no llega
    /// al servidor.
    #[gpui_kit::test]
    fn the_plus_of_a_page_starts_a_new_one_inside_it(cx: &mut TestAppContext) {
        let (server, _) = server("member");
        let (window, view) = open(&server, 1400., cx);
        click(window, "doc-pages", cx);
        wait_until(cx, "sale la hoja con sus páginas", |cx| on_screen(window, "doc-page-row-p1", cx));

        // El «+» sale al pasar el ratón por su fila.
        cx.update_window(window, |_, window, cx| {
            window.hover("doc-page-row-p1", cx);
            window.click("doc-page-new-p1", cx);
        })
        .unwrap();
        wait_until(cx, "sale el editor de la página nueva", |cx| on_screen(window, "doc-edit-title", cx));
        assert!(!on_screen(window, "doc-pages-sheet", cx), "la hoja sigue encima");
        assert_eq!(label_of(window, "doc-editing-what", cx), "Página nueva dentro");
        assert_eq!(draft(cx).map(|edit| edit.what), Some(cac_core::DocEditWhat::NewPage { parent_id: Some("p1".into()) }));
        assert!(cx.update(|cx| !Backend::snapshot(cx).read(cx).docs.on_page()), "el «+» abrió la página en vez de empezar una dentro");
        assert!(!on_screen(window, "doc-tab-overview", cx), "una página nueva no lleva las pestañas de la portada");
        assert!(writes(&server).is_empty(), "se creó al pulsar");

        // El título tiene el foco: se escribe sin pulsar nada antes.
        cx.update_window(window, |_, window, cx| window.input("Los tipos", cx)).unwrap();
        wait_until(cx, "el título llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.title == "Los tipos"));
        assert_eq!(boxes(&view, cx).0, "Los tipos");
        // Enter en el título baja al texto, no parte el título.
        cx.update_window(window, |_, window, cx| {
            window.press("enter", cx);
            window.input("Qué son", cx);
        })
        .unwrap();
        wait_until(cx, "el texto llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.body == "Qué son"));
        assert_eq!(boxes(&view, cx).0, "Los tipos");
        click(window, "doc-save", cx);
        wait_until(cx, "creada", |_| !writes(&server).is_empty());
        assert_eq!(
            writes(&server),
            [("POST".to_string(), "/api/v1/docs/space/a/pages".to_string(), json!({ "title": "Los tipos", "parentId": "p1", "body": "Qué son" }))]
        );
        wait_until(cx, "se abre la nueva", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.page_id.as_deref() == Some("new1")));
        stop(cx);
    }

    /// «Editar» en una página trae su título y su texto a las cajas, con
    /// sus migas encima, y cambiarle el título manda sólo el título.
    ///
    /// Roturas que atrapa: abrir la caja del título en blanco sobre una
    /// página que tiene uno (guardar se lo quitaría); un cambio de título
    /// que no llega al servidor, o que arrastra el texto consigo.
    #[gpui_kit::test]
    fn editing_a_page_brings_its_title_and_its_text(cx: &mut TestAppContext) {
        let (server, _) = server("member");
        let (window, view) = open(&server, 1400., cx);
        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open_page("p1").await }));
        wait_until(cx, "llega la página", |cx| on_screen(window, "doc-page-children", cx));

        click(window, "doc-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-editor", cx));
        assert_eq!(boxes(&view, cx), ("Arquitectura".to_string(), "## Capas\n\nTres.".to_string()));
        assert!(on_screen(window, "doc-page-trail", cx), "no dice de dónde cuelga lo que se escribe");
        assert!(!on_screen(window, "doc-editing-what", cx));

        type_in(window, "doc-edit-title", "X", cx);
        wait_until(cx, "el título llega al borrador", |cx| draft(cx).is_some_and(|edit| edit.title.contains('X')));
        let title = boxes(&view, cx).0;
        click(window, "doc-save", cx);
        wait_until(cx, "guardado", |_| !writes(&server).is_empty());
        assert_eq!(writes(&server), [("PUT".to_string(), "/api/v1/docs/space/a/pages/p1".to_string(), json!({ "title": title }))]);
        stop(cx);
    }

    /// En una página en blanco, una plantilla abre el editor con su texto
    /// sin guardar nada, y «Escribirlo» lo abre vacío; «Página nueva
    /// dentro», al pie, empieza una dentro de la que se lee.
    ///
    /// Roturas que atrapa: una plantilla que no hace nada, o que se guarda
    /// sola; el «+» del pie creando en la portada.
    #[gpui_kit::test]
    fn an_empty_page_offers_where_to_start(cx: &mut TestAppContext) {
        let (server, _) = server("member");
        let (window, view) = open(&server, 1400., cx);
        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open_page("p2").await }));
        wait_until(cx, "la página vacía ofrece por dónde empezar", |cx| on_screen(window, "doc-page-template-procedure", cx));

        click(window, "doc-page-template-procedure", cx);
        wait_until(cx, "sale el editor con la plantilla", |cx| on_screen(window, "doc-editor", cx));
        let (title, body) = boxes(&view, cx);
        assert_eq!(title, "");
        assert!(body.starts_with("## Cuándo se hace"), "la plantilla no está en la caja: {body:?}");
        // Al lado se pinta lo que hay en la caja, no lo guardado (nada).
        assert!(on_screen(window, "doc-edit-preview-text", cx) && !on_screen(window, "doc-edit-preview-empty", cx));
        assert_eq!(label_of(window, "doc-edit-status", cx), "Cambios sin guardar");
        assert!(writes(&server).is_empty(), "la plantilla se guardó sola");
        click(window, "doc-cancel", cx);
        wait_until(cx, "una plantilla sin guardar también se pregunta", |cx| on_screen(window, "doc-ask", cx));
        click(window, "doc-ask-yes", cx);
        wait_until(cx, "descartada", |cx| !on_screen(window, "doc-editor", cx));

        click(window, "doc-write", cx);
        wait_until(cx, "sale el editor en blanco", |cx| on_screen(window, "doc-editor", cx));
        assert_eq!(boxes(&view, cx).1, "");
        assert!(on_screen(window, "doc-edit-preview-empty", cx), "sin nada escrito, al lado se dice que no hay nada");
        click(window, "doc-cancel", cx);
        wait_until(cx, "sin nada escrito, cancelar cierra", |cx| !on_screen(window, "doc-editor", cx));

        click(window, "doc-page-new-child", cx);
        wait_until(cx, "sale el editor de la página nueva", |cx| on_screen(window, "doc-edit-title", cx));
        assert_eq!(draft(cx).map(|edit| edit.what), Some(cac_core::DocEditWhat::NewPage { parent_id: Some("p2".into()) }));
        click(window, "doc-cancel", cx);
        wait_until(cx, "sin nada escrito, cancelar cierra", |cx| !on_screen(window, "doc-editor", cx));

        // Y el «+» de la portada, en la hoja, empieza una sin madre.
        click(window, "doc-pages", cx);
        wait_until(cx, "sale la hoja", |cx| on_screen(window, "doc-pages-home", cx));
        cx.update_window(window, |_, window, cx| {
            window.hover("doc-pages-home", cx);
            window.click("doc-pages-new", cx);
        })
        .unwrap();
        wait_until(cx, "sale el editor de la página nueva", |cx| on_screen(window, "doc-edit-title", cx));
        assert_eq!(draft(cx).map(|edit| edit.what), Some(cac_core::DocEditWhat::NewPage { parent_id: None }));
        assert_eq!(label_of(window, "doc-editing-what", cx), "Página nueva");
        stop(cx);
    }
}
