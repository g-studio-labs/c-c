//! Plataforma: el cuarto módulo del riel. Las reglas y el estado viven en
//! `cac_core::platform`; aquí sólo se pinta, un fichero por pantalla, como
//! en `app/src/pages/` de la app de Tauri. Cada fase del diseño («Diseño ·
//! Plataforma en la app nativa», en cac) añade el suyo.

pub mod panel;
pub mod server_dialog;
pub mod servers;
pub mod ssh_key;

use cac_core::ServerHealth;
use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Hsla};

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
