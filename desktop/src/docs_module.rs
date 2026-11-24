//! El módulo Documentación: su panel —«Todos los docs», el filtro y el árbol
//! de espacios con las páginas de cada documento colgando de su nodo
//! (`DocsPanel.tsx`, `DocsTree.tsx`)— y su pantalla de inicio, la tabla de
//! toda la documentación (`DocIndex.tsx`). Un documento abierto desde aquí
//! se lee en la vista de siempre (`docs.rs`).
//!
//! Qué filas hay, cuáles van abiertas y en qué orden sale la tabla lo dice
//! el core (`docs_panel`); aquí se pinta y se le pasa cada gesto.

use cac_core::{DocIndexRow, DocTreeItem, DocTreeRow, NodeKind};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Selectable as _, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;
use crate::workspace::PANEL_ROW_HEIGHT;

/// Lo que entra cada nivel del árbol (`PanelRow.tsx`: `4 + depth * 12`).
const STEP: f32 = 12.;
/// El hueco del chevrón de un nodo (`size-5` y su `gap-1`): un nodo sin él
/// entra eso de más, para no quedar a la izquierda de su hermano.
const CHEVRON_SLOT: f32 = 24.;
/// Una fila de página o de portada (`h-7`).
const PAGE_ROW_HEIGHT: f32 = 28.;

/// El panel de Documentación, bajo la cabecera del panel.
pub(crate) fn panel(filter: &Entity<InputState>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (hover, muted, foreground) = (theme.sidebar_accent, theme.muted_foreground, theme.sidebar_foreground);
    let docs = Backend::snapshot(cx).read(cx).docs.clone();
    let tree = Backend::snapshot(cx).read(cx).tree.clone();
    let query = filter.read(cx).value().to_string();
    let rows = docs.docs_tree_rows(&tree, &query, locale(cx));
    let nothing = rows.is_empty() && !query.trim().is_empty();
    // Leyendo un documento, la tabla ya no es lo que se tiene delante.
    let on_index = docs.target.is_none();
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div().px_1().child(
                div()
                    .id("docs-all")
                    .h(px(PANEL_ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .rounded(px(8.))
                    .leading_sm()
                    .cursor_pointer()
                    .when(on_index, |this| this.bg(hover).font_weight(FontWeight::MEDIUM))
                    .when(!on_index, |this| this.text_color(muted).hover(move |style| style.bg(hover.opacity(0.5)).text_color(foreground)))
                    .child(Icon::new(Lucide::FileText).size_4())
                    .child(tr(cx, "nav:item.docs"))
                    .on_click(|_, _, cx| Backend::core(cx).docs().show_index())
                    .test_support(),
            ),
        )
        // Filtra lo que ya está en el panel; no es el buscador.
        .child(div().mt_1().px_2().py_1().child(Input::new(filter).id("docs-filter").cleanable(true).prefix(Icon::new(IconName::Search).size_3p5().text_color(muted))))
        .when(nothing, |this| {
            this.child(div().id("docs-no-matches").test_support().px_3().py_2().leading_xs().text_color(muted).child(tr(cx, "nav:panel.noMatches")))
        })
        .child(div().id("docs-tree").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().px_1().children(rows.into_iter().map(|row| tree_row(row, cx))))
        .into_any_element()
}

/// Una fila del árbol: un nodo (`PanelRow`), la portada o una página
/// (`PageRow`). El chevrón despliega sin navegar; el resto de la fila abre.
fn tree_row(row: DocTreeRow, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (hover, muted, foreground, accent) = (theme.sidebar_accent, theme.muted_foreground, theme.sidebar_foreground, theme.sidebar_accent);
    let node = row.item == DocTreeItem::Node;
    let id = match row.item {
        DocTreeItem::Node => format!("docs-node-{}", row.id),
        DocTreeItem::Home => format!("docs-home-{}", row.id),
        DocTreeItem::Page => format!("docs-page-{}", row.id),
    };
    let indent = 4. + STEP * row.depth as f32
        + match row.item {
            DocTreeItem::Node if !row.expandable => CHEVRON_SLOT,
            DocTreeItem::Home => 20.,
            _ => 0.,
        };
    let chevron_slot = if node { 20. } else { 16. };
    // Una página sin hijas guarda el hueco del chevrón; un nodo sin nada
    // dentro, no: ya entra `CHEVRON_SLOT` de más.
    let chevron = (row.item == DocTreeItem::Page || (node && row.expandable)).then(|| {
        let (open, item, kind, toggled) = (row.open, row.item, row.owner_kind, row.id.clone());
        let label = cac_core::text_with(locale(cx), if open { "work:tree.collapse" } else { "work:tree.expand" }, &[("name", &row.title)]);
        div()
            .id(SharedString::from(format!("{id}-toggle")))
            .size(px(chevron_slot))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .text_color(muted)
            .when(row.expandable, |this| {
                this.child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(if node { 14. } else { 12. })))
                    .aria_label(label)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(foreground))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        let toggled = toggled.clone();
                        match item {
                            DocTreeItem::Page => Backend::core(cx).docs().toggle_page(&toggled),
                            _ => Backend::run(cx, move |core| async move { core.docs().toggle_node(kind, &toggled).await }),
                        }
                    })
            })
            .test_support()
    });
    let mark = match row.item {
        DocTreeItem::Node => match row.owner_kind {
            NodeKind::Space => div().size(px(10.)).rounded(px(2.)).bg(crate::theme::space_color(&row.color, row.hue, cx)).into_any_element(),
            NodeKind::Folder => Icon::new(Lucide::Folder).size_4().into_any_element(),
            NodeKind::List => Icon::new(Lucide::List).size_4().into_any_element(),
        },
        DocTreeItem::Home => Icon::new(Lucide::House).size_3p5().into_any_element(),
        DocTreeItem::Page => Icon::new(Lucide::FileText).size_3p5().into_any_element(),
    };
    let (owner, target, item) = (row.owner_id.clone(), row.id.clone(), row.item);
    div()
        .id(SharedString::from(id.clone()))
        .h(px(if node { PANEL_ROW_HEIGHT } else { PAGE_ROW_HEIGHT }))
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .pl(px(indent))
        .pr_1()
        .rounded(px(6.))
        .map(|this| if node { this.leading_sm() } else { this.text_size(px(13.)) })
        .cursor_pointer()
        .map(|this| {
            if row.active {
                this.bg(accent).font_weight(FontWeight::MEDIUM).text_color(foreground)
            } else if row.on_path {
                this.font_weight(FontWeight::MEDIUM).text_color(foreground).hover(move |style| style.bg(hover.opacity(0.5)))
            } else {
                this.text_color(muted).hover(move |style| style.bg(hover.opacity(0.5)).text_color(foreground))
            }
        })
        .children(chevron)
        .child(div().size_4().flex().flex_none().items_center().justify_center().child(mark))
        .child(div().flex_1().min_w_0().truncate().child(row.title.clone()))
        // Tiene algo escrito.
        .when(row.written, |this| {
            this.child(div().id(SharedString::from(format!("{id}-written"))).flex_none().child(Icon::new(Lucide::FileText).size_3().text_color(muted)).test_support())
        })
        .aria_selected(row.active)
        .on_click(move |_, _, cx| {
            let (owner, target) = (owner.clone(), target.clone());
            match item {
                DocTreeItem::Node => Backend::run(cx, move |core| async move { core.docs().open_node(&target).await }),
                DocTreeItem::Home => Backend::core(cx).docs().close_page(),
                DocTreeItem::Page => Backend::run(cx, move |core| async move { core.docs().open_tree_page(&owner, &target).await }),
            }
        })
        .test_support()
        .into_any_element()
}

/// La tabla de toda la documentación: la pantalla del módulo cuando no se
/// lee nada.
pub struct DocIndexView {
    filter: Entity<InputState>,
    /// «Necesita atención»: sólo lo viejo o sin responsable.
    attention_only: bool,
}

impl DocIndexView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "work:docIndex.filter");
        Self { filter: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)), attention_only: false }
    }
}

/// Las columnas, con el ancho de cada una; `None` reparte lo que sobra.
const COLUMNS: [(&str, Option<f32>, bool); 6] = [
    ("work:docIndex.document", None, false),
    ("work:docIndex.where", None, false),
    ("work:docIndex.owner", Some(160.), false),
    ("work:docIndex.lastReviewed", Some(128.), false),
    // Cuántas páginas cuelgan de la portada: uno con veinte páginas y otro
    // sólo con su resumen no son lo mismo de revisar.
    ("work:docIndex.pages", Some(72.), true),
    ("work:docIndex.tasks", Some(72.), true),
];

fn cell(index: usize, content: impl IntoElement) -> Div {
    let (_, width, right) = COLUMNS[index];
    div()
        .px_2()
        .min_w_0()
        .truncate()
        .map(|this| match width {
            Some(width) => this.w(px(width)).flex_none(),
            None => this.flex_1(),
        })
        .when(right, |this| this.flex().justify_end())
        .child(content)
}

impl DocIndexView {
    fn render_row(row: DocIndexRow, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, danger, hover) = (theme.border, theme.muted_foreground, theme.danger, theme.muted.opacity(0.5));
        // `amber-500` de Tailwind 4, como en la cabecera del documento.
        let amber = Hsla::from(rgb(0xfe9a00));
        let owner = match row.owner.clone() {
            Some(name) => div().child(name).into_any_element(),
            // El único rojo de la tabla: un documento sin dueño no está mal
            // escrito, es que nadie lo va a arreglar cuando deje de servir.
            None => div()
                .id(SharedString::from(format!("doc-index-no-owner-{}", row.id)))
                .flex()
                .items_center()
                .gap_1()
                .text_color(danger)
                .child(Icon::new(Lucide::CircleAlert).size_3())
                .child(tr(cx, "work:docs.noOwner"))
                .test_support()
                .into_any_element(),
        };
        let id = row.id.clone();
        div()
            .id(SharedString::from(format!("doc-index-row-{}", row.id)))
            .flex()
            .items_center()
            .py_2()
            .border_b_1()
            .border_color(border)
            .leading_sm()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(cell(0, div().font_weight(FontWeight::MEDIUM).child(row.name)))
            .child(cell(1, div().text_color(muted).child(row.path)))
            .child(cell(2, owner))
            .child(cell(3, div().when(row.stale, |this| this.text_color(amber)).child(row.reviewed)))
            .child(cell(4, div().text_color(muted).child(row.pages.map(|pages| pages.to_string()).unwrap_or_default())))
            .child(cell(5, div().text_color(muted).child(row.tasks.map(|tasks| tasks.to_string()).unwrap_or_default())))
            .on_click(move |_, _, cx| {
                let id = id.clone();
                Backend::run(cx, move |core| async move { core.docs().open(&id).await });
            })
            .test_support()
            .into_any_element()
    }
}

impl Render for DocIndexView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, foreground, background) = (theme.border, theme.muted_foreground, theme.foreground, theme.background);
        let accent = theme.accent;
        let snapshot = Backend::snapshot(cx).read(cx);
        let query = self.filter.read(cx).value().to_string();
        let index = cac_core::doc_index(&snapshot.tree.spaces, &snapshot.tree.doc_marks, &query, self.attention_only, locale(cx));
        let attention = self.attention_only;
        let rows: Vec<AnyElement> = index.written.iter().cloned().map(|row| Self::render_row(row, cx)).collect();
        let chips: Vec<AnyElement> = index
            .undocumented
            .iter()
            .map(|row| {
                let id = row.id.clone();
                div()
                    .id(SharedString::from(format!("doc-index-undocumented-{}", row.id)))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .border_1()
                    .border_dashed()
                    .border_color(border)
                    .leading_xs()
                    .text_color(muted)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(foreground))
                    .child(Icon::new(Lucide::Plus).size_3())
                    .child(format!("{} › {}", row.path, row.name))
                    .on_click(move |_, _, cx| {
                        let id = id.clone();
                        Backend::run(cx, move |core| async move { core.docs().open(&id).await });
                    })
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let undocumented = cac_core::text_count(locale(cx), "work:docIndex.undocumented", index.undocumented.len() as i64);
        let header_row = div()
            .flex()
            .items_center()
            .h(px(40.))
            .border_b_1()
            .border_color(border)
            .leading_sm()
            .font_weight(FontWeight::MEDIUM)
            .children(COLUMNS.iter().enumerate().map(|(index, (key, _, _))| cell(index, tr(cx, key))));
        div()
            .id("doc-index")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .child(
                div()
                    .h(px(48.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .border_b_1()
                    .border_color(border)
                    .child(Icon::new(Lucide::FileText).size_4().text_color(muted))
                    .child(div().id("doc-index-title").leading_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, "work:docIndex.title")).test_support())
                    .child(div().ml_2().w(px(224.)).child(Input::new(&self.filter).id("doc-index-filter").cleanable(true)))
                    .child(
                        Button::new("doc-index-attention")
                            .ghost()
                            .small()
                            .selected(attention)
                            .when(attention, |this| this.bg(accent))
                            .label(tr(cx, "work:docIndex.needsAttention"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.attention_only = !this.attention_only;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("doc-index-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .child(header_row)
                    .children(rows)
                    // «Nada coincide» y «nada escrito» son dos cosas: con la
                    // organización sin documentar, una tabla vacía parecía un
                    // filtro mal puesto y no una invitación a escribir.
                    .when(index.written.is_empty(), |this| {
                        let key = if index.any_written { "work:docIndex.nothing" } else { "work:docIndex.noneYet" };
                        this.child(div().id("doc-index-empty").py_8().text_center().leading_sm().text_color(muted).child(tr(cx, key)).test_support())
                    })
                    // Al pie y no escondidas: son las que hay que arreglar.
                    .when(!chips.is_empty(), |this| {
                        this.child(
                            div()
                                .mt_8()
                                .child(div().mb_2().leading_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(undocumented))
                                .child(div().flex().flex_wrap().gap_1p5().children(chips)),
                        )
                    }),
            )
    }
}
