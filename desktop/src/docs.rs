//! La documentación de un nodo, para leerla: sus cuatro pestañas y, en la de
//! decisiones, el registro vigente.
//!
//! Qué documento hay abierto, qué pestaña, qué se lee, qué parte es prosa y
//! por qué está vacío lo dice el core. Aquí sólo se pinta, con las medidas
//! de `app/src/components/docs/DocTabs.tsx`. Escribirla sigue en la app de
//! siempre.

use cac_core::{DocBlock, DocDecision, DocHeading, WorkLayout, DOC_TABS};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon};
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

pub struct DocsView;

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
                    if crate::theme::leads_somewhere(url) {
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
        let (copy, close_hint, pages_hint) = (node.clone(), tr(cx, "work:docs.cancel"), format!("{} · {}", tr(cx, "work:docs.pages"), tr(cx, "common:misc.notInThisApp")));
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
            .child(div().id("doc-title").test_support().min_w_0().truncate().leading_sm().font_weight(FontWeight::MEDIUM).child(name))
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
                    .child(icon_button("doc-pages", Lucide::ListTree).tooltip(move |window, cx| Tooltip::new(pages_hint.clone()).build(window, cx)).test_support())
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
                "work:docs.emptyTab" => block.child(said).child(
                    ghost_button("doc-write", Lucide::Pencil, tr(cx, "work:docs.writeIt"), cx).mt_3().border_1().border_color(border).tooltip(not_yet(cx)).test_support(),
                ),
                _ => block.child(said),
            }
        });

        let reading = div()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .children(nothing)
            .when(!blocks.is_empty(), |this| this.child(Self::render_blocks(blocks.clone(), &node, &docs.tab, cx)))
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
            });

        // El pie (`border-t px-4 py-2`): en el registro, apuntar una
        // decisión; en lo demás, si hay texto, editarlo. Ninguna de las dos
        // está todavía en esta app.
        let foot = if on_decisions {
            Some(ghost_button("doc-record-decision", Lucide::Gavel, tr(cx, "work:decisions.record"), cx))
        } else if !blocks.is_empty() {
            Some(ghost_button("doc-edit", Lucide::Pencil, tr(cx, "work:docs.edit"), cx))
        } else {
            None
        }
        .map(|button| div().id("doc-foot").flex().flex_none().items_center().gap_2().px_4().py_2().border_t_1().border_color(border).child(button.tooltip(not_yet(cx)).test_support()));

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
            .child(tabs)
            .child(
                div().id("doc-scroll").flex_1().min_h_0().overflow_y_scroll().p_6().child(
                    div()
                        .w_full()
                        .max_w(px(if wide { WIDE_PAGE_WIDTH } else { PAGE_WIDTH }))
                        .mx_auto()
                        .flex()
                        .when(wide, |this| this.gap_8())
                        .child(reading)
                        .when(!contents.is_empty(), |this| this.child(Self::render_contents(contents, cx))),
                ),
            )
            .children(foot)
    }
}
