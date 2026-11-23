//! Plataforma: el cuarto módulo del riel. Las reglas y el estado viven en
//! `cac_core::platform`; aquí sólo se pinta, un fichero por pantalla, como
//! en `app/src/pages/` de la app de Tauri. Cada fase del diseño («Diseño ·
//! Plataforma en la app nativa», en cac) añade el suyo.

pub mod activity;
pub mod agent_install;
pub mod backup_form;
pub mod backup_local;
pub mod backups;
pub mod deploy;
pub mod deploy_settings;
pub mod secret_refs;
pub mod stack_secrets;
pub mod panel;
pub mod server_dialog;
pub mod server_page;
pub mod servers;
pub mod k8s_hub;
pub mod nodes;
pub mod provision;
pub mod provision_config;
pub mod provision_engine;
pub mod provision_requests;
pub mod provision_template;
pub mod services;
pub mod stats;
pub mod ssh_key;

use cac_core::ServerHealth;
use gpui_kit::component::ActiveTheme;
use gpui_kit::{anchored, deferred, div, App, Hsla, InteractiveElement, IntoElement, ParentElement, Point, Styled, Window};

/// El color del punto de estado de un servidor, el de `STATUS_DOT` en
/// `PlatformPanel.tsx`: verde en línea, rojo caído, gris si nadie lo sabe.
pub(crate) fn health_color(health: ServerHealth, cx: &App) -> Hsla {
    let theme = cx.theme();
    match health {
        ServerHealth::Online => theme.success,
        ServerHealth::Offline | ServerHealth::Error => theme.danger,
        ServerHealth::Pending | ServerHealth::Unknown => theme.muted_foreground.opacity(0.5),
    }
}

/// El fondo de un diálogo, sobre la ventana entera: anclado a su origen y
/// diferido, para tapar también el riel y el panel, como el `fixed inset-0`
/// de los diálogos de la app de Tauri.
pub(crate) fn veil(id: &'static str, card: impl IntoElement, window: &Window) -> impl IntoElement {
    let size = window.viewport_size();
    deferred(
        anchored().position(Point::default()).child(
            div()
                .id(id)
                .w(size.width)
                .h(size.height)
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .bg(gpui_kit::black().opacity(0.35))
                .child(card),
        ),
    )
    .with_priority(1)
}
