//! La documentación de un nodo, para leerla: sus cuatro pestañas (la
//! portada), en la de decisiones el registro vigente, y las páginas que
//! cuelgan de ella.
//!
//! Qué documento hay abierto, qué pestaña o qué página, qué se lee, qué
//! parte es prosa, cómo es el árbol de páginas y por qué está vacío lo dice
//! el core. Aquí sólo se pinta, con las medidas de
//! `app/src/components/docs/DocTabs.tsx`, `DocPagesSheet.tsx`,
//! `DocPagesBranch.tsx`, `PageRow.tsx` y `DocPageView.tsx`. Escribirla sigue
//! en la app de siempre.

use cac_core::{DocBacklinkRow, DocBlock, DocDecision, DocHeading, DocPageLink, DocPageRow, DocsState, WorkLayout, DOC_TABS};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
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

pub struct DocsView {
    /// El foco de la hoja «Páginas»: con él, Escape la cierra.
    sheet_focus: FocusHandle,
    /// Si la hoja ya estaba abierta en el pintado anterior: el foco se le da
    /// una vez, al abrirse.
    sheet_was_shown: bool,
}

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
fn reading_style(cx: &App) -> TextViewStyle {
    let muted = cx.theme().muted;
    let mut table = StyleRefinement::default();
    // Una tabla que no cabe se desliza por dentro en vez de estrujarse.
    table.overflow.x = Some(Overflow::Scroll);
    TextViewStyle::default()
        .paragraph_gap(rems(0.8))
        .heading_font_size(|level, _| {
            px(match level {
                1 => 18.,
                2 => 17.,
                3 => 13.,
                _ => 15.,
            })
        })
        .code_block(StyleRefinement::default().bg(muted).rounded(px(8.)).p_3().leading_xs())
        .table(table)
        .table_head(StyleRefinement::default().bg(muted.opacity(0.5)).font_weight(FontWeight::MEDIUM))
        .table_cell(StyleRefinement::default().px_2().py_1().leading_xs())
}

impl DocsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { sheet_focus: cx.focus_handle(), sheet_was_shown: false }
    }

    fn open_page(id: String, cx: &App) {
        Backend::run(cx, move |core| async move { core.docs().open_page(&id).await });
    }

    /// Un botón de una fila del árbol de páginas que sólo sale al pasar el
    /// ratón por ella (`hidden size-5 … group-hover:grid`), y que todavía no
    /// hace nada aquí: lo dice al pasar por él.
    fn row_action(id: SharedString, icon: impl Into<Icon>, label: String, group: SharedString, cx: &App) -> impl IntoElement {
        let background = cx.theme().background;
        let hint = format!("{label} · {}", tr(cx, "common:misc.notInThisApp"));
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
            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
            .test_support()
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
    fn render_page_row(row: DocPageRow, cx: &App) -> impl IntoElement {
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
            .child(Self::row_action(SharedString::from(format!("doc-page-new-{}", row.id)), Lucide::Plus, tr(cx, "work:docs.newSubpage"), group.clone(), cx))
            .child(Self::row_action(SharedString::from(format!("doc-page-menu-{}", row.id)), IconName::Ellipsis, menu_label, group, cx))
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
            .child(Self::row_action("doc-pages-new".into(), Lucide::Plus, tr(cx, "work:docs.newPage"), home_group.clone(), cx))
            .child(Self::row_action("doc-pages-trash".into(), Lucide::Trash, tr(cx, "work:docs.trash"), home_group, cx))
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
            .children(rows.into_iter().map(|row| Self::render_page_row(row, cx)))
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
    /// «Escribirlo». Ninguna de las tres está todavía en esta app.
    fn render_empty_page(said: impl IntoElement, cx: &App) -> Div {
        let theme = cx.theme();
        let (muted, border, accent) = (theme.muted_foreground, theme.border, theme.accent);
        div()
            .py_8()
            .flex()
            .flex_col()
            .items_center()
            .child(said)
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
                    .tooltip(not_yet(cx))
                    .test_support()
            })))
            .child(ghost_button("doc-write", Lucide::Pencil, tr(cx, "work:docs.writeIt"), cx).mt_3().tooltip(not_yet(cx)).test_support())
    }

    /// Lo que va bajo el texto de una página: las que tiene dentro, siempre
    /// (dice qué hay sin mirar el árbol), y lo que la enlaza, si algo lo
    /// hace.
    fn render_page_tail(children: Vec<DocPageLink>, backlinks: Vec<DocBacklinkRow>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, primary) = (theme.muted_foreground, theme.border, theme.primary);
        let hover = crate::theme::ghost_hover(cx);
        let new_hint = format!("{} · {}", tr(cx, "work:docs.newSubpage"), tr(cx, "common:misc.notInThisApp"));
        let inside = div()
            .id("doc-page-children")
            .mt_10()
            .pt_4()
            .border_t_1()
            .border_color(border)
            .flex()
            .flex_col()
            .child(
                div().mb_2().flex().items_center().gap_2().child(section_title(tr(cx, "work:docs.childPages"), muted)).child(
                    div()
                        .id("doc-page-new-child")
                        .size_6()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(px(8.))
                        .hover(move |style| style.bg(hover))
                        .child(Icon::new(Lucide::Plus).size_3p5())
                        .tooltip(move |window, cx| Tooltip::new(new_hint.clone()).build(window, cx))
                        .test_support(),
                ),
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
                        // Otra página de este documento se abre aquí; una
                        // tarea u otro documento, todavía no.
                        .map(|this| match link.page_id {
                            Some(id) => this.cursor_pointer().hover(|style| style.underline()).on_click(move |_, _, cx| Self::open_page(id.clone(), cx)),
                            None => this.tooltip(not_yet(cx)),
                        })
                        .test_support()
                })))
                .test_support()
        });
        div().flex().flex_col().child(inside).children(linked)
    }

    /// Una entrada del registro, con las medidas de `DecisionList.tsx`
    /// (`rounded-[11px] border p-3`): arriba cuándo, quién y su etiqueta;
    /// el título; el porqué, tal cual se escribió; y de dónde salió.
    fn render_decision(&self, decision: DocDecision, cx: &mut Context<Self>) -> impl IntoElement {
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

    /// Lo que se lee, trozo a trozo: la prosa en su medida y centrada, y lo
    /// ancho (tablas, código) a todo lo que da la página.
    fn render_blocks(blocks: Vec<DocBlock>, node: &str, tab: &str, cx: &App) -> impl IntoElement {
        let style = reading_style(cx);
        div().id("doc-body").flex().flex_col().gap_3().text_size(px(15.)).line_height(relative(1.75)).children(blocks.into_iter().enumerate().map(
            |(index, block)| {
                // Un id por nodo, pestaña y trozo: el del texto anterior no
                // debe prestarle su estado a éste.
                let id = SharedString::from(format!("doc-text-{node}-{tab}-{index}"));
                let text = TextView::markdown(id, block.text).style(style.clone()).on_link_click(|url, _, _, cx| {
                    // Un enlace a otra página de este documento la abre
                    // aquí; lo demás, como siempre.
                    if let Some(page) = Backend::snapshot(cx).read(cx).docs.page_for_link(url) {
                        Self::open_page(page, cx);
                    } else if crate::theme::leads_somewhere(url) {
                        cx.open_url(url);
                    }
                });
                let kind = if block.wide { "wide" } else { "prose" };
                div()
                    .id(SharedString::from(format!("doc-{kind}-{index}")))
                    .w_full()
                    .min_w_0()
                    .when(!block.wide, |this| this.max_w(px(READING_MEASURE)).mx_auto())
                    .child(text)
                    .test_support()
            },
        ))
        .test_support()
    }

    /// «En esta página»: las secciones de lo que se lee, al lado en una
    /// ventana grande (`DocToc.tsx`). Saltar a una todavía no está aquí.
    fn render_contents(contents: Vec<DocHeading>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground, border) = (theme.muted_foreground, theme.foreground, theme.border);
        div()
            .id("doc-contents")
            .w(px(CONTENTS_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .child(div().mb_2().text_size(px(11.)).text_color(muted).child(tr(cx, "work:docs.onThisPage").to_uppercase()))
            .child(div().flex().flex_col().gap_0p5().border_l_1().border_color(border).children(contents.into_iter().enumerate().map(|(index, heading)| {
                div()
                    .id(("doc-contents-entry", index))
                    .py_1()
                    .pr_2()
                    .map(|this| if heading.level == 3 { this.pl_5() } else { this.pl_3() })
                    .leading_xs()
                    .text_color(muted)
                    .hover(move |style| style.text_color(foreground))
                    .child(heading.text)
                    .tooltip(not_yet(cx))
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
        let blocks = docs.blocks();
        let decisions = docs.standing_decisions();
        let on_decisions = docs.tab == "decisions";
        let width = window.viewport_size().width;
        let (wide, hints) = (width >= px(WIDE_WINDOW), width >= px(HINTS_FROM));
        let contents = if wide { docs.contents() } else { Vec::new() };

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
                                    // en la vista que se pulsó.
                                    let _ = Backend::core(cx).set_work_layout(layout);
                                    Backend::core(cx).back_to_tasks();
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

        // Sin nada que leer: por qué, en el centro (`py-12 text-center`), y
        // cómo empezar a escribirlo, que todavía no está aquí.
        let nothing = empty.map(|key| {
            let said = div().id("doc-empty").test_support().leading_sm().text_color(muted).child(tr(cx, key));
            let block = div().py_12().flex().flex_col().items_center().text_center();
            match key {
                "work:decisions.empty" => block
                    .child(Icon::new(Lucide::Gavel).size_5().mb_2().text_color(muted))
                    .child(said)
                    .child(div().mt_1().max_w(px(384.)).leading_xs().text_color(muted).child(tr(cx, "work:decisions.emptyWhy"))),
                "work:docs.emptyPage" => Self::render_empty_page(said, cx),
                "work:docs.emptyTab" => block.child(said).child(
                    ghost_button("doc-write", Lucide::Pencil, tr(cx, "work:docs.writeIt"), cx).mt_3().border_1().border_color(border).tooltip(not_yet(cx)).test_support(),
                ),
                _ => block.child(said),
            }
        });

        // De qué es el texto: de una pestaña o de una página. El de una no
        // le presta su estado al de la otra.
        let reading_key = docs.page_id.as_ref().map(|id| format!("page-{id}")).unwrap_or_else(|| docs.tab.clone());
        // Una página lleva encima sus migas y su título, y debajo —cuando
        // ya llegó— lo que tiene dentro y lo que la enlaza.
        let page_head = on_page.then(|| Self::render_page_head(name.clone(), docs.page_trail(here), docs.page_title(here), cx));
        let page_tail = docs.page.is_some().then(|| Self::render_page_tail(docs.page_children(here), docs.page_backlinks(here), cx));
        let reading = div()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .children(page_head)
            .children(nothing)
            .when(!blocks.is_empty(), |this| this.child(Self::render_blocks(blocks.clone(), &node, &reading_key, cx)))
            .when(!decisions.is_empty(), |this| {
                this.child(
                    div()
                        .w_full()
                        .max_w(px(READING_MEASURE))
                        .mx_auto()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(decisions.into_iter().map(|decision| self.render_decision(decision, cx))),
                )
            })
            .children(page_tail);

        // El pie (`border-t px-4 py-2`): en el registro, apuntar una
        // decisión; en lo demás, si hay texto, editarlo. Ninguna de las dos
        // está todavía en esta app.
        let foot_bar = || div().id("doc-foot").flex().flex_none().items_center().gap_2().px_4().py_2().border_t_1().border_color(border);
        let foot = if docs.page.is_some() {
            // El de una página (`DocPageView.tsx`): editar si hay texto, su
            // historial y, al fondo, mandarla a la papelera.
            Some(
                foot_bar()
                    .when(!blocks.is_empty(), |this| this.child(ghost_button("doc-edit", Lucide::Pencil, tr(cx, "work:docs.edit"), cx).tooltip(not_yet(cx)).test_support()))
                    .child(
                        ghost_button("doc-page-history", Lucide::RotateCcwClock, tr(cx, "work:docs.history"), cx).h_6().gap_1p5().leading_xs().tooltip(not_yet(cx)).test_support(),
                    )
                    .child(ghost_button("doc-page-trash", Lucide::Trash, tr(cx, "work:docs.trashPage"), cx).ml_auto().text_color(muted).tooltip(not_yet(cx)).test_support()),
            )
        } else if on_page {
            None
        } else if on_decisions {
            Some(foot_bar().child(ghost_button("doc-record-decision", Lucide::Gavel, tr(cx, "work:decisions.record"), cx).tooltip(not_yet(cx)).test_support()))
        } else if !blocks.is_empty() {
            Some(foot_bar().child(ghost_button("doc-edit", Lucide::Pencil, tr(cx, "work:docs.edit"), cx).tooltip(not_yet(cx)).test_support()))
        } else {
            None
        };

        // La hoja «Páginas», encima de todo mientras está abierta.
        let sheet = docs.pages_shown.then(|| self.render_pages_sheet(&docs, window, cx));
        self.sheet_was_shown = docs.pages_shown;
        // Cada página empieza arriba: la que se abre desde el pie de otra no
        // hereda lo que se había bajado en ella.
        let scroll_id = docs.page_id.as_ref().map(|id| SharedString::from(format!("doc-scroll-{id}"))).unwrap_or_else(|| "doc-scroll".into());

        div()
            .id("docs")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .child(header)
            .children(views)
            // Las pestañas son de la portada: una página no las lleva.
            .when(!on_page, |this| this.child(tabs))
            .child(
                div().id(scroll_id).flex_1().min_h_0().overflow_y_scroll().p_6().child(
                    div()
                        .w_full()
                        .max_w(px(if wide { WIDE_PAGE_WIDTH } else { PAGE_WIDTH }))
                        .mx_auto()
                        .flex()
                        .when(wide, |this| this.gap_8())
                        .child(reading)
                        .when(!contents.is_empty(), |this| this.child(Self::render_contents(contents, cx))),
                )
                .test_support(),
            )
            .children(foot)
            .children(sheet)
    }
}
