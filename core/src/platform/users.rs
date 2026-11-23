//! Usuarios: alta, edición y baja de las personas de la plataforma, sólo para
//! quien la administra entera (`app/src/pages/Users.tsx`,
//! `store/users.store.ts`). El servidor lo vuelve a decir con un 403; aquí es
//! para no ofrecerlo ni pedirlo.

use serde::{Deserialize, Serialize};

use super::{Platform, PlatformSection};
use crate::error::ApiError;
use crate::texts::{error_phrase, text, text_with};
use crate::Locale;

/// Una persona de la plataforma (`domain.UserResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct AdminUser {
    pub id: String,
    pub username: String,
    pub email: String,
    pub name: String,
    pub is_superadmin: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct UsersState {
    pub users: Vec<AdminUser>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<String>,
}

/// Lo escrito en el alta.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NewUserForm {
    pub username: String,
    pub password: String,
    pub name: String,
    pub email: String,
    pub is_superadmin: bool,
}

/// Lo escrito al editar. Una contraseña vacía no se cambia.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EditUserForm {
    pub name: String,
    pub email: String,
    pub password: String,
    pub is_superadmin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum UserField {
    Username,
    Password,
    Name,
    Email,
}

/// Lo que falta en un campo, dicho.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UserProblem {
    pub field: UserField,
    pub text: String,
}

/// Lo mínimo de un correo: algo, una arroba, algo con un punto.
fn looks_like_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else { return false };
    let Some((host, tld)) = domain.rsplit_once('.') else { return false };
    !email.chars().any(char::is_whitespace) && !local.is_empty() && !host.is_empty() && !tld.is_empty() && !domain.contains('@')
}

/// Lo que falta para poder crear, campo a campo. De la misma lista sale lo
/// que apaga el botón y lo que se le enseña a la persona: un botón apagado
/// que no dice por qué fue lo que hubo en Tauri. Las reglas son las de
/// `CreateUserRequest` en el backend.
pub fn new_user_problems(locale: Locale, form: &NewUserForm) -> Vec<UserProblem> {
    let mut problems = Vec::new();
    let mut add = |field, text: String| problems.push(UserProblem { field, text });
    if form.username.trim().chars().count() < 3 {
        add(UserField::Username, text(locale, "common:admin.usernameShort"));
    }
    if form.password.chars().count() < 8 {
        add(UserField::Password, text_with(locale, "common:admin.passwordShort", &[("count", &form.password.chars().count().to_string())]));
    }
    if form.name.trim().is_empty() {
        add(UserField::Name, text(locale, "common:admin.nameRequired"));
    }
    let email = form.email.trim();
    if email.is_empty() {
        add(UserField::Email, text(locale, "common:admin.emailRequired"));
    } else if !looks_like_email(email) {
        add(UserField::Email, text(locale, "common:admin.emailInvalid"));
    }
    problems
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UserEditError {
    #[error("superadmin only")]
    NotSuperadmin,
    #[error("invalid form")]
    Invalid,
    #[error("cannot delete yourself")]
    Yourself,
    #[error(transparent)]
    Api(#[from] ApiError),
}

pub fn user_edit_phrase(locale: Locale, error: &UserEditError, fallback_key: &str) -> String {
    match error {
        UserEditError::NotSuperadmin => text(locale, "errors:superadmin-required"),
        UserEditError::Invalid => text(locale, fallback_key),
        UserEditError::Yourself => text(locale, "common:admin.cantDeleteYourself"),
        UserEditError::Api(error) => error_phrase(locale, error, Some(fallback_key)),
    }
}

impl Platform {
    /// Abre Usuarios y pide la lista. Quien no es superadmin se queda donde
    /// estaba.
    pub async fn open_users(&self) {
        if self.show_section(PlatformSection::Users) {
            self.load_users().await;
        }
    }

    pub async fn load_users(&self) {
        if !self.superadmin() {
            return;
        }
        self.change(|data| {
            let users = data.users.get_or_insert_with(UsersState::default);
            users.loading = true;
            users.error = None;
        });
        let answer = self.core.get_list::<AdminUser>("/api/v1/users/").await;
        self.change(|data| {
            let users = data.users.get_or_insert_with(UsersState::default);
            users.loading = false;
            match answer {
                Ok(list) => {
                    users.users = list;
                    users.loaded = true;
                }
                Err(error) => users.error = Some(error.to_string()),
            }
        });
    }

    pub async fn create_user(&self, form: &NewUserForm) -> Result<AdminUser, UserEditError> {
        if !self.superadmin() {
            return Err(UserEditError::NotSuperadmin);
        }
        if !new_user_problems(Locale::En, form).is_empty() {
            return Err(UserEditError::Invalid);
        }
        let body = serde_json::json!({
            "username": form.username.trim(),
            "password": form.password,
            "name": form.name.trim(),
            "email": form.email.trim(),
            "isSuperadmin": form.is_superadmin,
        });
        let created: AdminUser = self.core.call("POST", "/api/v1/users/", Some(body)).await?;
        self.change(|data| {
            let users = data.users.get_or_insert_with(UsersState::default);
            users.users.push(created.clone());
            users.users.sort_by(|a, b| a.username.to_lowercase().cmp(&b.username.to_lowercase()));
        });
        Ok(created)
    }

    /// Edita a alguien. El papel propio no se cambia: un superadmin que se
    /// lo quita a sí mismo se queda fuera de esta pantalla sin nadie que lo
    /// devuelva.
    pub async fn update_user(&self, user_id: &str, form: &EditUserForm) -> Result<(), UserEditError> {
        if !self.superadmin() {
            return Err(UserEditError::NotSuperadmin);
        }
        if !form.password.is_empty() && form.password.chars().count() < 8 {
            return Err(UserEditError::Invalid);
        }
        let me = self.core.session().map(|session| session.id).unwrap_or_default();
        let mut body = serde_json::json!({ "name": form.name, "email": form.email });
        if user_id != me {
            body["isSuperadmin"] = serde_json::Value::Bool(form.is_superadmin);
        }
        if !form.password.is_empty() {
            body["password"] = serde_json::Value::String(form.password.clone());
        }
        self.core.call_ok("PATCH", &format!("/api/v1/users/{user_id}"), Some(body)).await?;
        self.change(|data| {
            if let Some(user) = data.users.as_mut().and_then(|users| users.users.iter_mut().find(|user| user.id == user_id)) {
                user.name = form.name.clone();
                user.email = form.email.clone();
                if user_id != me {
                    user.is_superadmin = form.is_superadmin;
                }
            }
        });
        Ok(())
    }

    /// Borra a alguien para siempre. **Quien llama ya preguntó.** A uno mismo
    /// no.
    pub async fn delete_user(&self, user_id: &str) -> Result<(), UserEditError> {
        if !self.superadmin() {
            return Err(UserEditError::NotSuperadmin);
        }
        if self.core.session().is_some_and(|session| session.id == user_id) {
            return Err(UserEditError::Yourself);
        }
        self.core.call_ok("DELETE", &format!("/api/v1/users/{user_id}"), None).await?;
        self.change(|data| {
            if let Some(users) = data.users.as_mut() {
                users.users.retain(|user| user.id != user_id);
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(username: &str, password: &str, name: &str, email: &str) -> NewUserForm {
        NewUserForm { username: username.into(), password: password.into(), name: name.into(), email: email.into(), is_superadmin: false }
    }

    fn fields(form: &NewUserForm) -> Vec<UserField> {
        new_user_problems(Locale::Es, form).into_iter().map(|problem| problem.field).collect()
    }

    /// Rotura que atrapa: dejar crear con un usuario de dos letras, una
    /// contraseña de siete, sin nombre o con algo que no es un correo (el
    /// servidor lo rechazaría con un 400 sin decir qué campo).
    #[test]
    fn a_new_user_says_what_is_missing() {
        assert!(fields(&form("ana", "12345678", "Ana", "ana@x.com")).is_empty());
        assert_eq!(fields(&form("an", "1234567", " ", "ana@x")), [UserField::Username, UserField::Password, UserField::Name, UserField::Email]);
        assert_eq!(fields(&form("ana", "12345678", "Ana", "")), [UserField::Email]);
        assert_eq!(new_user_problems(Locale::Es, &form("ana", "1234", "Ana", "a@b.co"))[0].text, "La contraseña lleva al menos 8 caracteres (van 4)");
    }
}
