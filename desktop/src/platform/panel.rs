//! El panel de Plataforma: «Servidores» con una fila por servidor y su punto
//! de estado, y debajo lo que todavía no está en esta app
//! (`app/src/components/sidebar/panels/PlatformPanel.tsx`).

use cac_core::{platform_sections, PlatformSection, Server};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::tr;
use crate::theme::Leading as _;
use crate::workspace::{Workspace, INDENT, PANEL_ROW_HEIGHT};

pub(crate) fn panel(cx: &App) -> AnyElement {
    let snapshot = Backend::snapshot(cx).read(cx);
    let state = snapshot.platform.clone();
    let superadmin = snapshot.session.as_ref().is_some_and(|session| session.superadmin);
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let on_list = state.open_server.is_none();

    let servers: Vec<AnyElement> = if state.loaded && state.servers.is_empty() {
        vec![div().id("platform-no-servers").px_4().pb_1().leading_xs().text_color(muted).child(tr(cx, "nav:panel.noServers")).test_support().into_any_element()]
    } else {
        state.servers.iter().map(|server| server_row(server, state.open_server.as_deref() == Some(&server.id), cx)).collect()
    };

    div()
        .flex()
        .flex_col()
        .px_1()
        .child(
            row("platform-servers", Icon::new(Lucide::Server), tr(cx, "nav:item.servers"), on_list, 0., cx)
                .on_click(|_, _, cx| Backend::core(cx).platform().close_server())
                .test_support(),
        )
        .children(servers)
        .children(platform_sections(superadmin).into_iter().filter_map(|section| {
            let (id, icon) = match section {
                PlatformSection::Servers => return None,
                PlatformSection::Activity => ("platform-activity", Lucide::Workflow),
                PlatformSection::Diagnostics => ("platform-diagnostics", Lucide::Activity),
                PlatformSection::Users => ("platform-users", Lucide::Users),
                PlatformSection::DevTools => ("platform-devtools", Lucide::Wrench),
            };
            Some(Workspace::pending_panel_row(id, icon, section.label_key(), cx))
        }))
        .into_any_element()
}

fn server_row(server: &Server, active: bool, cx: &App) -> AnyElement {
    let status = tr(cx, server.status.label_key());
    // El estado lo dice el punto, también a quien no lo ve (`aria-label` en
    // `PlatformPanel.tsx`).
    let dot = div()
        .id(SharedString::from(format!("platform-dot-{}", server.id)))
        .size_2()
        .rounded_full()
        .bg(super::health_color(server.status, cx))
        .aria_label(status.clone())
        .test_support();
    let id = server.id.clone();
    row(SharedString::from(format!("platform-server-{}", server.id)), dot, server.name.clone(), active, INDENT, cx)
        .tooltip(move |window, cx| Tooltip::new(status.clone()).build(window, cx))
        .on_click(move |_, _, cx| {
            let id = id.clone();
            Backend::run(cx, move |core| async move { core.platform().open_server_and_load(&id).await });
        })
        .test_support()
        .into_any_element()
}

fn row(id: impl Into<ElementId>, icon: impl IntoElement, label: String, active: bool, indent: f32, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let (hover, muted, foreground) = (theme.sidebar_accent, theme.muted_foreground, theme.sidebar_foreground);
    div()
        .id(id)
        .h(px(PANEL_ROW_HEIGHT))
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .px_1()
        .mx_1()
        .pl(px(4. + indent))
        .rounded(px(8.))
        .leading_sm()
        .cursor_pointer()
        .when(active, |this| this.bg(hover).font_weight(FontWeight::MEDIUM))
        .when(!active, |this| this.text_color(muted).hover(move |style| style.bg(hover.opacity(0.5)).text_color(foreground)))
        .child(div().size_4().flex().flex_none().items_center().justify_center().child(icon))
        .child(div().flex_1().min_w_0().truncate().child(label))
}
