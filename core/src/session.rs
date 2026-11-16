//! La sesión: quién está dentro y con qué par de tokens.
//!
//! Vivía en la pantalla —`app/src/lib/api.ts` y `auth.store.ts`, con los
//! tokens en `localStorage`— y eso tenía dos problemas. Cada plataforma nueva
//! tendría que volver a escribir las mismas reglas; y un refresh recién rotado
//! que la pantalla no llegaba a guardar antes de un reinicio dejaba la app
//! arrancando con el anterior, que el servidor toma por robado (22898591).
//!
//! Las reglas son las de `api.ts`, una por una, y cada una tiene su prueba en
//! `tests/session.rs`:
//!
//! - se renueva sólo cuando el servidor dice `expired-token`, **una vez por
//!   proceso** aunque fallen diez llamadas a la vez, y la llamada se repite;
//! - sólo un 401 del refresh cierra la sesión: un 500 es el servidor que no
//!   pudo mirar, y echar a alguien por eso convierte un hipo de la base en un
//!   cierre masivo;
//! - el par nuevo se guarda **antes** de usarse.

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use std::time::Duration;

use crate::transport::{self, RawReply, RawRequest, Reply, Request};
use crate::Core;

/// La clave bajo la que se guarda la sesión en el `Storage`.
const STORAGE_KEY: &str = "session";

/// El plazo de un fichero. El de una llamada normal está pensado para JSON
/// pequeño y cortaría treinta megas a medio subir; éste es la ventana que el
/// servidor da a una subida (`attachmentReadWindow`).
const FILE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Quién está dentro. Espejo de `domain.Session` del backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub username: String,
    /// Cómo se le llama. Puede faltar: quien pinte cae al usuario.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default)]
    pub superadmin: bool,
    #[serde(default)]
    pub must_change_password: bool,
    /// Lo que puede hacer, cuando quien entra es un token personal. Una
    /// persona con su contraseña no los trae.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// En qué idioma lee cac; vacío es «pregúntale a la máquina».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

/// Lo que el core tiene en la mano y lo que guarda: la sesión y su par.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Held {
    pub session: Session,
    pub access_token: String,
    pub refresh_token: String,
}

/// El sobre de toda respuesta del backend (`domain.APIResponse`).
#[derive(Deserialize)]
struct Envelope<T> {
    #[serde(default)]
    success: bool,
    data: Option<T>,
    /// La etiqueta para el código.
    error: Option<String>,
    /// La frase para leer.
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthData {
    access_token: String,
    refresh_token: String,
    session: Session,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshData {
    access_token: String,
    refresh_token: String,
}

/// La etiqueta de error que trae un cuerpo, si la trae.
fn error_code(body: &str) -> String {
    serde_json::from_str::<Envelope<serde_json::Value>>(body)
        .ok()
        .and_then(|envelope| envelope.error)
        .unwrap_or_default()
}

/// Saca el `data` de una respuesta, o el error del servidor con sus dos campos.
fn unwrap_data<T: for<'de> Deserialize<'de>>(reply: &Reply) -> Result<T, ApiError> {
    let envelope = serde_json::from_str::<Envelope<T>>(&reply.body).ok();
    if reply.status < 400 {
        if let Some(Envelope { success: true, data: Some(data), .. }) = envelope {
            return Ok(data);
        }
    }
    Err(server_error(reply))
}

/// El error que cuenta una respuesta, con su etiqueta y su frase.
pub(crate) fn server_error(reply: &Reply) -> ApiError {
    // Un cuerpo de error puede no encajar en ningún tipo: se lee sin él.
    let loose = serde_json::from_str::<Envelope<serde_json::Value>>(&reply.body).ok();
    let code = loose.as_ref().and_then(|e| e.error.clone()).unwrap_or_default();
    let message = loose
        .and_then(|e| e.message)
        .unwrap_or_else(|| if code.is_empty() { "Request failed".into() } else { code.clone() });
    ApiError::Server { status: reply.status, code, message }
}

/// El `data` de una respuesta buena que puede no traerlo.
pub(crate) fn data_or_default<T: for<'de> Deserialize<'de> + Default>(reply: &Reply) -> T {
    serde_json::from_str::<Envelope<T>>(&reply.body).ok().and_then(|envelope| envelope.data).unwrap_or_default()
}

impl Core {
    /// Lo guardado en el arranque, si lo hay y se deja leer.
    pub(crate) fn load_held(&self) -> Option<Held> {
        let raw = self.inner.storage.get(STORAGE_KEY)?;
        serde_json::from_str(&raw).ok()
    }

    fn held(&self) -> Option<Held> {
        self.inner.held.lock().expect("held lock").clone()
    }

    /// Guarda y luego pone en la mano, en ese orden.
    ///
    /// Al revés, otra llamada podría usar el token nuevo —y el servidor dar el
    /// viejo por gastado— antes de que esté a salvo: un corte ahí deja la app
    /// arrancando con un refresh que ya no vale.
    fn keep(&self, held: Held) -> Result<(), ApiError> {
        let raw = serde_json::to_string(&held).map_err(|e| ApiError::Storage(e.to_string()))?;
        let stored = self.inner.storage.set(STORAGE_KEY, &raw).map_err(ApiError::Storage);
        *self.inner.held.lock().expect("held lock") = Some(held);
        stored
    }

    fn forget(&self) {
        self.inner.storage.remove(STORAGE_KEY);
        *self.inner.held.lock().expect("held lock") = None;
        // Sin sesión no hay stream: el que hubiera se retira en su próximo turno.
        self.stream().stop();
        self.inner.listener.session_changed(None);
    }

    /// Con qué servidor habla, sin barra final.
    pub fn base_url(&self) -> &str {
        &self.inner.config.base_url
    }

    /// Quién está dentro, si hay alguien.
    pub fn session(&self) -> Option<Session> {
        self.held().map(|held| held.session)
    }

    /// El token de acceso de ahora, para lo que el core todavía no hace por sí
    /// mismo (una subida multipart desde la pantalla). Puede estar caducado:
    /// quien reciba `expired-token` pide `refresh_access_token`.
    pub fn access_token(&self) -> Option<String> {
        self.held().map(|held| held.access_token)
    }

    /// Alguien acaba de entrar por un camino que no es `login` (una
    /// invitación, un cambio de contraseña): ésta es su sesión desde ahora.
    pub fn sign_in(
        &self,
        session: Session,
        access_token: String,
        refresh_token: String,
    ) -> Result<(), ApiError> {
        let result = self.keep(Held { session: session.clone(), access_token, refresh_token });
        self.inner.listener.session_changed(Some(session));
        result
    }

    /// Recoge la sesión que una versión anterior guardaba en otro sitio
    /// (`localStorage`), **sólo si el core no tiene ya una**.
    ///
    /// La que tiene el core es más nueva por definición —es la que se ha ido
    /// renovando—, y pisarla con la copia vieja sería reintroducir a mano el
    /// refresh gastado que esto venía a quitar. Devuelve si la adoptó.
    pub fn adopt_legacy_session(
        &self,
        session: Session,
        access_token: String,
        refresh_token: String,
    ) -> Result<bool, ApiError> {
        if self.held().is_some() {
            return Ok(false);
        }
        self.sign_in(session, access_token, refresh_token)?;
        Ok(true)
    }

    /// Los datos de la persona cambiaron (vienen de `/auth/me`); el par sigue.
    pub fn update_session(&self, session: Session) -> Result<(), ApiError> {
        let Some(held) = self.held() else { return Ok(()) };
        let result = self.keep(Held { session: session.clone(), ..held });
        self.inner.listener.session_changed(Some(session));
        result
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<Session, ApiError> {
        let mut body = serde_json::json!({ "username": username, "password": password });
        if let Some(client) = &self.inner.config.client {
            body["client"] = serde_json::Value::String(client.clone());
        }
        let reply = self.request("POST", "/api/v1/auth/login", Some(body.to_string()), false).await?;
        let data: AuthData = unwrap_data(&reply)?;
        self.sign_in(data.session.clone(), data.access_token, data.refresh_token)?;
        Ok(data.session)
    }

    /// Sale, **también en el servidor**: revoca el refresh y los que salieron
    /// de él. Mejor esfuerzo: sin red se sale igual en local.
    pub async fn logout(&self) {
        if let Some(held) = self.held() {
            let _ = self
                .send("POST", "/api/v1/auth/logout", None, Some(&held.refresh_token))
                .await;
        }
        self.forget();
    }

    /// Vuelve a pedir quién es, para que lleguen los campos que se añadieron
    /// después de entrar.
    pub async fn me(&self) -> Result<Session, ApiError> {
        let reply = self.request("GET", "/api/v1/auth/me", None, true).await?;
        let session: Session = unwrap_data(&reply)?;
        self.update_session(session.clone())?;
        Ok(session)
    }

    /// Una llamada al backend, con la sesión puesta y renovada si hace falta.
    ///
    /// Un estado de error vuelve como respuesta, no como `Err`: quien llama
    /// lee el código y la frase. `Err` es que no hubo respuesta, o que la
    /// sesión caducó y no se pudo renovar.
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<String>,
        auth: bool,
    ) -> Result<Reply, ApiError> {
        let token = if auth { self.access_token() } else { None };
        let reply = self.send(method, path, body.clone(), token.as_deref()).await?;

        if auth && reply.status >= 400 && error_code(&reply.body) == "expired-token" {
            let Some(fresh) = self.refresh(token).await else {
                return Err(ApiError::SessionExpired);
            };
            return self.send(method, path, body, Some(&fresh)).await;
        }
        Ok(reply)
    }

    /// Un GET con sesión que devuelve el `data` del sobre, ya con su tipo.
    pub(crate) async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        let reply = self.request("GET", path, None, true).await?;
        unwrap_data(&reply)
    }

    /// Una petición con sesión y cuerpo JSON que devuelve el `data` del sobre.
    pub(crate) async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, ApiError> {
        let reply = self.request(method, path, body.map(|body| body.to_string()), true).await?;
        unwrap_data(&reply)
    }

    /// Lo mismo para las rutas que contestan sin `data` («Task moved»): sólo
    /// importa si salió bien.
    pub(crate) async fn call_ok(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(), ApiError> {
        let reply = self.request(method, path, body.map(|body| body.to_string()), true).await?;
        if reply.status < 400 {
            return Ok(());
        }
        unwrap_data::<serde_json::Value>(&reply).map(|_| ())
    }

    /// Fuerza un token nuevo. Para quien recibió el rechazo por su cuenta: el
    /// stream de eventos, o una subida que hizo la pantalla.
    pub async fn refresh_access_token(&self) -> Option<String> {
        self.refresh(self.access_token()).await
    }

    /// Renueva el par, una sola vez aunque lo pidan muchos a la vez.
    ///
    /// `stale` es el token con el que falló quien llama. Si al conseguir el
    /// turno el de la mano ya es otro, alguien renovó mientras esperábamos y
    /// ése es el que vale: pedir otro gastaría un refresh que ya está rotado.
    pub(crate) async fn refresh(&self, stale: Option<String>) -> Option<String> {
        let _turn = self.inner.refreshing.lock().await;

        let held = self.held()?;
        if stale.as_deref() != Some(held.access_token.as_str()) {
            return Some(held.access_token);
        }

        let reply = self
            .send("POST", "/api/v1/auth/refresh", None, Some(&held.refresh_token))
            .await
            .ok()?; // red o plazo: pasajero, la sesión se queda

        if reply.status == 401 {
            // Caducado, revocado, o reutilizado y dado por robado.
            self.forget();
            return None;
        }
        let data: RefreshData = unwrap_data(&reply).ok()?;

        let access_token = data.access_token.clone();
        // Si no se pudo guardar, la sesión sigue en la mano: este proceso
        // puede continuar, y la siguiente renovación lo vuelve a intentar.
        let _ = self.keep(Held {
            session: held.session.clone(),
            access_token: data.access_token,
            refresh_token: data.refresh_token,
        });
        // La persona es la misma, pero quien tenga una copia del token de
        // acceso (el stream de eventos, una subida) tiene que enterarse de
        // que cambió: si no, sigue llamando con el caducado.
        self.inner.listener.session_changed(Some(held.session));
        Some(access_token)
    }

    /// El viaje, con un reintento si no llegó.
    ///
    /// Un socket muerto se cura solo con una conexión nueva; sin esto, el
    /// primer clic tras un rato sin usar la app era un error.
    async fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<String>,
        bearer: Option<&str>,
    ) -> Result<Reply, ApiError> {
        self.send_with(method, path, body, bearer, &[]).await
    }

    /// Lo mismo, con cabeceras de más (el idioma de un correo).
    pub(crate) async fn send_with(
        &self,
        method: &str,
        path: &str,
        body: Option<String>,
        bearer: Option<&str>,
        extra: &[(&str, &str)],
    ) -> Result<Reply, ApiError> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if let Some(token) = bearer {
            headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        }
        headers.extend(extra.iter().map(|(name, value)| (name.to_string(), value.to_string())));
        let request = Request {
            method: method.to_string(),
            url: format!("{}{}", self.inner.config.base_url, path),
            headers,
            body,
            timeout: self.inner.config.request_timeout,
        };
        match transport::send(&self.inner.http, request.clone()).await {
            Err(ApiError::Network(_)) => transport::send(&self.inner.http, request).await,
            other => other,
        }
    }

    /// Un viaje de bytes con la sesión puesta. Sin reintento si no llegó: un
    /// fichero que sí llegó y cuya respuesta se perdió quedaría subido dos
    /// veces.
    async fn send_file(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Option<Vec<u8>>,
        bearer: Option<&str>,
    ) -> Result<RawReply, ApiError> {
        let mut headers = Vec::new();
        if let Some(content_type) = content_type {
            headers.push(("Content-Type".to_string(), content_type.to_string()));
        }
        if let Some(token) = bearer {
            headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        }
        let request = RawRequest {
            method: method.to_string(),
            url: format!("{}{}", self.inner.config.base_url, path),
            headers,
            body,
            timeout: FILE_TIMEOUT,
        };
        transport::send_raw(&self.inner.http, request).await
    }

    fn file_error(reply: &RawReply) -> ApiError {
        server_error(&Reply { status: reply.status, body: String::from_utf8_lossy(&reply.body).into_owned() })
    }

    /// Baja un fichero de este servidor, con la sesión en la cabecera.
    ///
    /// La ruta que sirve los bytes de un adjunto contesta **404** a un token
    /// caducado, no «caducó»: a quien no puede verlo no le dice ni que
    /// existe. Así que un 404 (o un 401) renueva la sesión una vez y vuelve a
    /// pedir; si sigue sin estar, es que no está.
    pub(crate) async fn download(&self, path: &str) -> Result<Vec<u8>, ApiError> {
        let token = self.access_token();
        let mut reply = self.send_file("GET", path, None, None, token.as_deref()).await?;
        if matches!(reply.status, 401 | 404) {
            // Si no se pudo renovar, se queda el «no está» que se recibió: la
            // red caída un momento no es una sesión caducada.
            if let Some(fresh) = self.refresh(token).await {
                reply = self.send_file("GET", path, None, None, Some(&fresh)).await?;
            }
        }
        if reply.status >= 400 {
            return Err(Self::file_error(&reply));
        }
        Ok(reply.body)
    }

    /// Sube un cuerpo ya montado (un `multipart`) con la sesión puesta.
    ///
    /// Devuelve lo que el servidor dice que guardó: quien sube un fichero
    /// para citarlo necesita saber con qué dirección quedó.
    pub(crate) async fn upload<T: for<'de> Deserialize<'de>>(&self, path: &str, content_type: &str, body: Vec<u8>) -> Result<T, ApiError> {
        let token = self.access_token();
        let reply = self.send_file("POST", path, Some(content_type), Some(body.clone()), token.as_deref()).await?;
        let expired = reply.status >= 400 && error_code(&String::from_utf8_lossy(&reply.body)) == "expired-token";
        let reply = if expired {
            let Some(fresh) = self.refresh(token).await else {
                return Err(ApiError::SessionExpired);
            };
            self.send_file("POST", path, Some(content_type), Some(body), Some(&fresh)).await?
        } else {
            reply
        };
        if reply.status >= 400 {
            return Err(Self::file_error(&reply));
        }
        unwrap_data(&Reply { status: reply.status, body: String::from_utf8_lossy(&reply.body).into_owned() })
    }
}
