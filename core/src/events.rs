//! Cómo se entera la pantalla de que algo cambió.
//!
//! El core no sabe qué pinta: avisa por este trait, y cada plataforma lo
//! convierte en lo suyo (un evento de Tauri, un `StateFlow`, un `@Published`).

use crate::session::Session;

pub trait Listener: Send + Sync {
    /// La sesión cambió: alguien entró, salió, o el servidor la dio por
    /// terminada. `None` es «no hay nadie dentro».
    fn session_changed(&self, session: Option<Session>);
}

/// Para quien no necesita enterarse (el MCP, una prueba).
pub struct NoListener;

impl Listener for NoListener {
    fn session_changed(&self, _session: Option<Session>) {}
}
