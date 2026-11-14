//! El puente entre la pantalla y el core (`cac-core`).
//!
//! El core no sabe de Tauri: aquí se le da dónde guardar (un fichero en la
//! carpeta de datos de la app), se le traduce el aviso de «cambió la sesión»
//! a un evento que la pantalla escucha, y se exponen sus funciones como
//! comandos. No hay lógica: si algo de aquí decide algo, está en el sitio
//! equivocado y Android no lo va a tener.

use std::sync::{Arc, OnceLock};

use cac_core::{Config, Core, FileStorage, Listener, Session};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// El evento con el que la pantalla se entera de un cambio de sesión.
pub const SESSION_EVENT: &str = "core://session";

/// El core de este proceso. Vacío hasta `core_init`: la URL del servidor la
/// sabe la pantalla (`VITE_API_URL`), y hasta que no la dice no hay con quién
/// hablar.
#[derive(Default)]
pub struct CoreSlot(OnceLock<Core>);

impl CoreSlot {
    pub fn get(&self) -> Result<&Core, String> {
        self.0.get().ok_or_else(|| "core-not-ready".to_string())
    }
}

/// El token de acceso y el servidor, para lo que descarga por su cuenta
/// (adjuntos, exportar notas). `None` si no hay nadie dentro.
pub fn credentials(slot: &CoreSlot) -> Option<(String, String)> {
    let core = slot.get().ok()?;
    Some((core.access_token()?, core.base_url().to_string()))
}

/// Lo que la pantalla necesita saber de la sesión.
///
/// Lleva el token de acceso —dura minutos, y la pantalla lo usa para las
/// subidas y para el stream de eventos— y **nunca el de refresh**: ése no
/// sale de Rust. Guardián en la app: `auth.store.test.ts`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    session: Option<Session>,
    access_token: Option<String>,
}

fn view(core: &Core) -> SessionView {
    SessionView { session: core.session(), access_token: core.access_token() }
}

#[derive(Serialize)]
pub struct ReplyView {
    status: u16,
    body: String,
}

/// Convierte el aviso del core en un evento de Tauri.
struct Forward {
    app: AppHandle,
}

impl Listener for Forward {
    fn session_changed(&self, _session: Option<Session>) {
        // Se vuelve a leer del core en vez de usar el argumento: el evento
        // lleva también el token, y así los dos salen de la misma foto.
        if let Ok(core) = self.app.state::<CoreSlot>().get() {
            let _ = self.app.emit(SESSION_EVENT, view(core));
        }
    }
}

/// Arranca el core y devuelve la sesión que hubiera guardada.
///
/// La pantalla lo espera **antes de pintar**: sin eso, el primer render no
/// sabría si hay alguien dentro y enseñaría el login un instante.
#[tauri::command]
pub fn core_init(app: AppHandle, slot: State<'_, CoreSlot>, base_url: String) -> Result<SessionView, String> {
    if slot.0.get().is_none() {
        let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let mut config = Config::new(base_url);
        config.client = Some("desktop".to_string());
        let core = Core::new(
            config,
            Arc::new(FileStorage::new(dir)),
            Arc::new(Forward { app: app.clone() }),
        )
        .map_err(|e| e.to_string())?;
        // Si otro `core_init` llegó antes, gana el suyo: son la misma cosa.
        let _ = slot.0.set(core);
    }
    Ok(view(slot.get()?))
}

#[tauri::command]
pub fn core_session(slot: State<'_, CoreSlot>) -> Result<SessionView, String> {
    Ok(view(slot.get()?))
}

/// Cualquier llamada al backend, con la sesión puesta y renovada si caducó.
#[tauri::command]
pub async fn core_request(
    slot: State<'_, CoreSlot>,
    method: String,
    path: String,
    body: Option<String>,
    auth: bool,
) -> Result<ReplyView, String> {
    let core = slot.get()?.clone();
    let reply = core.request(&method, &path, body, auth).await.map_err(|e| e.to_string())?;
    Ok(ReplyView { status: reply.status, body: reply.body })
}

#[tauri::command]
pub fn core_sign_in(
    slot: State<'_, CoreSlot>,
    session: Session,
    access_token: String,
    refresh_token: String,
) -> Result<(), String> {
    slot.get()?.sign_in(session, access_token, refresh_token).map_err(|e| e.to_string())
}

/// La sesión que una versión anterior guardaba en `localStorage`. Sólo entra
/// si el core no tiene ninguna; devuelve si entró.
#[tauri::command]
pub fn core_adopt_session(
    slot: State<'_, CoreSlot>,
    session: Session,
    access_token: String,
    refresh_token: String,
) -> Result<bool, String> {
    slot.get()?
        .adopt_legacy_session(session, access_token, refresh_token)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn core_update_session(slot: State<'_, CoreSlot>, session: Session) -> Result<(), String> {
    slot.get()?.update_session(session).map_err(|e| e.to_string())
}

/// Fuerza un token de acceso nuevo; `None` si no se pudo.
#[tauri::command]
pub async fn core_refresh(slot: State<'_, CoreSlot>) -> Result<Option<String>, String> {
    let core = slot.get()?.clone();
    Ok(core.refresh_access_token().await)
}

#[tauri::command]
pub async fn core_logout(slot: State<'_, CoreSlot>) -> Result<(), String> {
    let core = slot.get()?.clone();
    core.logout().await;
    Ok(())
}
