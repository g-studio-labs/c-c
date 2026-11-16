//! Lo que cada persona elige de cómo ve cac: el tema y el idioma.
//!
//! Vino de `app/src/store/theme.store.ts` y `lib/locale-sync.ts`. No van al
//! mismo sitio, y es a propósito: el tema es de **esta máquina** (una pantalla
//! con luz de día y otra de noche no quieren lo mismo) y se guarda aquí; el
//! idioma es de **la persona** y se guarda en su cuenta, para que la siga a
//! cualquier dispositivo.

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::texts::Locale;
use crate::Core;

/// La clave bajo la que se guarda el tema.
const THEME_KEY: &str = "theme";

/// La clave bajo la que se guarda cómo se mira «Mi trabajo».
const WORK_LAYOUT_KEY: &str = "work-layout";

/// Cómo se mira «Mi trabajo»: las tres leen las mismas tareas ya traídas,
/// ninguna vuelve a pedirlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum WorkLayout {
    /// En lista si nadie eligió: es la que cabe en cualquier ancho.
    #[default]
    List,
    Board,
    Calendar,
}

/// Qué tema quiere quien mira.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// El que diga el sistema.
    System,
    Light,
    /// Oscuro si nadie eligió: cac es una herramienta de logs, estados y
    /// líneas de tiempo.
    #[default]
    Dark,
}

/// Si toca pintar en oscuro. `system_is_dark` es lo que dice el sistema, si
/// lo dice: uno que no contesta cuenta como oscuro.
pub fn is_dark(preference: ThemePreference, system_is_dark: Option<bool>) -> bool {
    match preference {
        ThemePreference::Dark => true,
        ThemePreference::Light => false,
        ThemePreference::System => system_is_dark.unwrap_or(true),
    }
}

impl Core {
    /// El tema elegido en esta máquina.
    pub fn theme(&self) -> ThemePreference {
        self.inner
            .storage
            .get(THEME_KEY)
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Elige el tema, y lo recuerda.
    pub fn set_theme(&self, preference: ThemePreference) -> Result<(), ApiError> {
        let raw = serde_json::to_string(&preference).map_err(|error| ApiError::Storage(error.to_string()))?;
        self.inner.storage.set(THEME_KEY, &raw).map_err(ApiError::Storage)
    }

    /// Cómo se mira «Mi trabajo» en esta máquina. Algo guardado que ya no
    /// se sabe pintar (una vista que se quitó) cuenta como no elegido.
    pub fn work_layout(&self) -> WorkLayout {
        self.inner
            .storage
            .get(WORK_LAYOUT_KEY)
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Elige cómo se mira «Mi trabajo», y lo recuerda.
    pub fn set_work_layout(&self, layout: WorkLayout) -> Result<(), ApiError> {
        let raw = serde_json::to_string(&layout).map_err(|error| ApiError::Storage(error.to_string()))?;
        self.inner.storage.set(WORK_LAYOUT_KEY, &raw).map_err(ApiError::Storage)
    }

    /// El idioma que eligió quien está dentro; `None` es «el del sistema».
    pub fn locale_preference(&self) -> Option<Locale> {
        self.session().and_then(|session| session.locale).as_deref().and_then(Locale::from_tag)
    }

    /// Elige el idioma: cambia ya en esta pantalla y se guarda en la cuenta.
    ///
    /// En ese orden: quien pulsa «Español» lo ve en español aunque el
    /// servidor tarde o no conteste. Si no se pudo guardar se dice, pero lo
    /// elegido aquí se queda: es lo que la persona pidió.
    pub async fn set_locale(&self, preference: Option<Locale>) -> Result<(), ApiError> {
        let Some(mut session) = self.session() else { return Err(ApiError::SessionExpired) };
        session.locale = preference.map(|locale| locale.tag().to_string());
        self.update_session(session)?;

        // Vacío es un valor, no una ausencia: es como se dice «el del sistema».
        let body = serde_json::json!({ "locale": preference.map(Locale::tag).unwrap_or("") });
        self.call_ok("PATCH", "/api/v1/auth/locale", Some(body)).await
    }
}
