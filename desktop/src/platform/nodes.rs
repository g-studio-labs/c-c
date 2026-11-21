//! La pestaña de nodos de un swarm (`ServerNodes.tsx`).

use cac_core::{PlatformState, SwarmNode};
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use crate::lang::tr;
use crate::theme::Leading as _;

use super::services::{empty, pill};

const COLUMNS: [&str; 5] = ["common:servers.thHostname", "common:servers.thRole", "common:servers.thStatus", "common:servers.thAvailability", "common:servers.thEngine"];

pub(crate) fn nodes(state: &PlatformState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (muted, danger) = (theme.muted_foreground, theme.danger);
    let problem = state.nodes_error.clone().map(|error| div().id("nodes-error").leading_xs().text_color(danger).child(error).test_support());
    let body = match state.nodes.as_deref() {
        None if state.nodes_error.is_some() => div().into_any_element(),
        None => empty("nodes-loading", tr(cx, "common:servers.loading"), muted),
        Some([]) => empty("nodes-empty", tr(cx, "common:servers.noNodes"), muted),
        Some(nodes) => table(nodes, cx),
    };
    div().id("server-nodes").flex().flex_col().gap_3().children(problem).child(body).test_support().into_any_element()
}

fn table(nodes: &[SwarmNode], cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (border, muted, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
    let heading = row(COLUMNS.iter().map(|key| div().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, key)).into_any_element()).collect()).h(px(36.));
    let rows = nodes.iter().map(|node| {
        // Como los `Badge` de Tauri: lo normal en el color de la app, lo raro
        // en gris o en rojo.
        let (primary, secondary, danger) = ((theme.primary.opacity(0.15), theme.primary), (theme.secondary, theme.secondary_foreground), (theme.danger.opacity(0.15), theme.danger));
        let role = if node.role == "manager" { primary } else { secondary };
        let status = if node.status == "ready" { primary } else { danger };
        let availability = if node.availability == "active" { primary } else { secondary };
        row(vec![
            div().font_weight(FontWeight::MEDIUM).truncate().child(node.hostname.clone()).into_any_element(),
            pill(node.role.clone(), role.0, role.1).into_any_element(),
            pill(node.status.clone(), status.0, status.1).into_any_element(),
            pill(node.availability.clone(), availability.0, availability.1).into_any_element(),
            div().font_family(mono.clone()).text_xs().leading_xs().text_color(muted).truncate().child(node.engine_version.clone()).into_any_element(),
        ])
        .id(SharedString::from(format!("node-row-{}", node.id)))
        .aria_label(node.hostname.clone())
        .h(px(44.))
        .border_t_1()
        .border_color(border)
        .test_support()
        .into_any_element()
    });
    div().id("nodes-table").flex().flex_col().rounded(px(8.)).border_1().border_color(border).leading_sm().child(heading).children(rows).test_support().into_any_element()
}

fn row(cells: Vec<AnyElement>) -> Div {
    div().flex().items_center().px_3().gap_3().children(cells.into_iter().map(|cell| div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().child(cell)))
}
