//! La lista de servidores, en lectura: nombre, dirección, tipo y estado
//! (la tabla de `app/src/pages/Dashboard.tsx`). Dar de alta, editar y lo
//! demás llegan con la fase 1.

use cac_core::{PlatformState, Server};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::tr;
use crate::theme::Leading as _;

/// Las columnas, con su parte del ancho: el nombre y la dirección son lo
/// que se lee; tipo y estado caben en poco.
const COLUMNS: [(&str, f32); 4] = [("common:admin.thName", 3.), ("common:admin.thHost", 3.), ("common:admin.thType", 2.), ("common:admin.thStatus", 2.)];

/// La página entera.
pub(crate) fn page(cx: &App) -> AnyElement {
    let state = Backend::snapshot(cx).read(cx).platform.clone();
    let theme = cx.theme();
    let (background, foreground, border, danger) = (theme.background, theme.foreground, theme.border, theme.danger);

    let header = div()
        .flex()
        .flex_none()
        .items_baseline()
        .gap_2()
        .px_4()
        .pt_3()
        .pb_3()
        .border_b_1()
        .border_color(border)
        .child(div().id("platform-title").leading_lg().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:admin.servers")).test_support());

    let body = div()
        .id("platform-body")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .children(state.error.clone().map(|error| {
            div()
                .id("platform-error")
                .flex()
                .items_center()
                .gap_1p5()
                .leading_xs()
                .text_color(danger)
                .child(Icon::new(Lucide::CircleAlert).size_3())
                .child(error)
                .test_support()
        }))
        .child(content(&state, cx));

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(background)
        .text_color(foreground)
        .line_height(relative(1.5))
        .child(header)
        .child(body)
        .into_any_element()
}

/// Lo que va bajo la cabecera: cargando, vacía, o la tabla.
fn content(state: &PlatformState, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    if !state.loaded {
        let phrase = if state.loading { tr(cx, "common:admin.loading") } else { String::new() };
        return div().id("platform-loading").leading_sm().text_color(muted).child(phrase).test_support().into_any_element();
    }
    if state.servers.is_empty() {
        return div().id("platform-empty").py_8().text_center().leading_sm().text_color(muted).child(tr(cx, "common:admin.noServers")).test_support().into_any_element();
    }
    let border = cx.theme().border;
    div()
        .id("platform-table")
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(border)
        .child(
            cells(COLUMNS.iter().map(|(key, _)| div().leading_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, key)).into_any_element()).collect())
                .h(px(36.)),
        )
        .children(state.servers.iter().map(|server| server_row(server, state.open_server.as_deref() == Some(&server.id), cx)))
        .test_support()
        .into_any_element()
}

fn server_row(server: &Server, open: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (border, muted, accent, mono) = (theme.border, theme.muted_foreground, theme.accent, theme.mono_font_family.clone());
    let status = div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(div().size_2().flex_none().rounded_full().bg(super::health_color(server.status, cx)))
        .child(tr(cx, server.status.label_key()));
    cells(vec![
        div().font_weight(FontWeight::MEDIUM).truncate().child(server.name.clone()).into_any_element(),
        div().font_family(mono).text_color(muted).truncate().child(server.address()).into_any_element(),
        div().text_color(muted).child(server.kind.as_str()).into_any_element(),
        status.into_any_element(),
    ])
    .id(SharedString::from(format!("platform-row-{}", server.id)))
    .h(px(44.))
    .border_t_1()
    .border_color(border)
    .leading_sm()
    .when(open, |this| this.bg(accent))
    .test_support()
    .into_any_element()
}

/// Una fila de la tabla, con cada celda en su parte del ancho.
fn cells(cells: Vec<AnyElement>) -> Div {
    div().flex().items_center().px_3().gap_3().children(
        cells.into_iter().zip(COLUMNS).map(|(cell, (_, share))| div().flex_grow(share).flex_basis(px(0.)).min_w_0().child(cell)),
    )
}
