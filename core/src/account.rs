//! Lo que hace alguien con su cuenta: pedir el enlace de «olvidé mi
//! contraseña», cambiar la contraseña y cambiar cómo se le llama.
//!
//! Y las reglas de esos formularios, que antes vivían en cada pantalla
//! (`app/src/pages/Login.tsx`, `ChangePassword.tsx`). El servidor las vuelve a
//! mirar todas: esto está para no mandar lo que ya se sabe que va a rechazar.

use serde::Deserialize;

use crate::error::ApiError;
use crate::session::Session;
use crate::texts::Locale;
use crate::Core;

/// La contraseña más corta que acepta el servidor.
pub const MIN_PASSWORD: usize = 8;

/// Si un formulario se puede mandar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum FormCheck {
    /// Se puede mandar.
    Ready,
    /// Falta algo por escribir: el botón no hace nada, y no hay nada que
    /// reprochar todavía.
    Incomplete,
    /// Lo escrito no vale. La clave del catálogo dice por qué.
    Problem { key: String },
}

fn problem(key: &str) -> FormCheck {
    FormCheck::Problem { key: key.to_string() }
}

/// El formulario de entrar.
pub fn check_sign_in(username: &str, password: &str) -> FormCheck {
    if username.trim().is_empty() || password.is_empty() {
        return FormCheck::Incomplete;
    }
    if password.chars().count() < MIN_PASSWORD {
        return problem("common:misc.passwordMin8");
    }
    FormCheck::Ready
}

/// El formulario de cambiar la contraseña.
///
/// Sólo reprocha lo que ya está escrito: una repetición vacía es que no se ha
/// llegado a ella, no que no coincida.
pub fn check_new_password(current: &str, next: &str, confirm: &str) -> FormCheck {
    if !next.is_empty() && next == current {
        return problem("common:admin.mustDiffer");
    }
    if !confirm.is_empty() && confirm != next {
        return problem("common:admin.passwordsDontMatch");
    }
    if current.is_empty() || confirm.is_empty() || next.chars().count() < MIN_PASSWORD {
        return FormCheck::Incomplete;
    }
    FormCheck::Ready
}

/// Qué puede ver quien abre la app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Gate {
    /// Nadie dentro: la pantalla de entrar.
    SignIn,
    /// Dentro, pero con una contraseña que puso otro (un alta, un
    /// restablecimiento): no pasa de aquí hasta poner la suya.
    NewPassword,
    /// Dentro.
    Open,
}

/// Hasta dónde llega una sesión.
pub fn gate(session: Option<&Session>) -> Gate {
    match session {
        None => Gate::SignIn,
        Some(session) if session.must_change_password => Gate::NewPassword,
        Some(_) => Gate::Open,
    }
}

/// Lo que devuelve cambiar la contraseña. Un servidor de antes del barrido
/// del 6-oct-2026 no devolvía par: por eso son opcionales.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct NewPair {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

impl Core {
    /// Pide el enlace para elegir una contraseña nueva.
    ///
    /// El servidor contesta lo mismo exista o no la cuenta, así que un `Ok`
    /// no dice que la haya. El correo sale en `locale`.
    pub async fn forgot_password(&self, login: &str, locale: Locale) -> Result<(), ApiError> {
        let body = serde_json::json!({ "login": login.trim() }).to_string();
        let reply = self
            .send_with(
                "POST",
                "/api/v1/public/account/forgot",
                Some(body),
                None,
                &[("Accept-Language", locale.tag())],
            )
            .await?;
        if reply.status < 400 {
            return Ok(());
        }
        Err(crate::session::server_error(&reply))
    }

    /// Guarda el nombre y el correo de quien está dentro (`Profile.tsx`) y
    /// vuelve a pedir quién es.
    ///
    /// El usuario no viaja: es el identificador, y cambiarlo rompería las
    /// menciones ya escritas. Y se relee la sesión porque el nombre se pinta
    /// desde ella en el menú de la cuenta y en media app: sin eso se guarda
    /// bien y se sigue viendo el de antes hasta reiniciar.
    pub async fn update_profile(&self, name: &str, email: &str) -> Result<Session, ApiError> {
        let body = serde_json::json!({ "name": name.trim(), "email": email.trim() });
        let reply = self.request("PATCH", "/api/v1/auth/me", Some(body.to_string()), true).await?;
        if reply.status >= 400 {
            return Err(crate::session::server_error(&reply));
        }
        self.me().await
    }

    /// Cambia la contraseña de quien está dentro y lo deja dentro.
    ///
    /// Cambiarla cierra **todas** las sesiones, también ésta: el refresh
    /// guardado queda revocado, y renovar con él sería un 401 que saca a quien
    /// la acaba de cambiar. El servidor devuelve un par nuevo y es el que se
    /// guarda. `/auth/me` va después: es lo que quita el «tiene que cambiarla».
    pub async fn change_password(&self, current: &str, next: &str) -> Result<Session, ApiError> {
        let body = serde_json::json!({ "currentPassword": current, "newPassword": next });
        let reply = self
            .request("POST", "/api/v1/auth/change-password", Some(body.to_string()), true)
            .await?;
        if reply.status >= 400 {
            return Err(crate::session::server_error(&reply));
        }
        let pair: NewPair = crate::session::data_or_default(&reply);
        match (pair.access_token, pair.refresh_token, self.session()) {
            (Some(access), Some(refresh), Some(session)) => self.sign_in(session, access, refresh)?,
            _ => {
                self.refresh_access_token().await;
            }
        }
        self.me().await
    }
}
