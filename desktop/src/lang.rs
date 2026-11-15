//! En qué idioma pinta la app, y el atajo para pedir un texto.
//!
//! El texto y la regla de qué idioma toca son del core. Aquí sólo se junta lo
//! que hace falta para preguntar: lo que dijo el sistema al arrancar y lo que
//! eligió quien está dentro.

use cac_core::{pick_locale, ApiError, Locale};
use gpui_kit::{App, Global};

use crate::backend::Backend;

/// El idioma del sistema, leído una vez al arrancar.
pub struct SystemLanguage(pub Option<String>);

impl Global for SystemLanguage {}

/// El idioma de ahora: el de la sesión, y si no dice nada, el del sistema.
pub fn locale(cx: &App) -> Locale {
    let session = Backend::snapshot(cx).read(cx).session.as_ref().and_then(|session| session.locale.clone());
    let system = cx.try_global::<SystemLanguage>().and_then(|system| system.0.clone());
    pick_locale(session.as_deref(), system.as_deref())
}

/// El texto de una clave del catálogo, en el idioma de ahora.
pub fn tr(cx: &App, key: &str) -> String {
    cac_core::text(locale(cx), key)
}

/// Un fallo, dicho para quien lo lee.
pub fn error_text(cx: &App, error: &ApiError, fallback_key: &str) -> String {
    cac_core::error_phrase(locale(cx), error, Some(fallback_key))
}
