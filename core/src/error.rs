//! Lo que puede salir mal al hablar con el servidor, con nombre.
//!
//! Hasta ahora todo error del lado Rust era un `String`, y quien lo recibía
//! lo reconocía mirando el texto. Aquí cada caso es una variante: es lo que
//! Kotlin y Swift pueden distinguir con un `when`, y lo que deja de romperse
//! cuando alguien retoca una frase.

/// Un fallo del core.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// No hubo respuesta: sin red, sin conexión, o se agotó el plazo.
    ///
    /// El prefijo `network:` es un contrato con la app de escritorio, que lo
    /// usa para separar «no llegó» de «llegó y dijo que no».
    #[error("network: {0}")]
    Network(String),

    /// La sesión caducó y no se pudo renovar. No dice que se haya cerrado: un
    /// servidor que no pudo mirar deja la sesión donde estaba.
    #[error("session-expired")]
    SessionExpired,

    /// El servidor contestó con un error. `code` es la etiqueta para el código
    /// (`inbox-other-org`) y `message` la frase para leer; no son lo mismo y
    /// no se intercambian.
    #[error("{message}")]
    Server {
        status: u16,
        code: String,
        message: String,
    },

    /// No se pudo guardar o leer lo que la plataforma guarda por nosotros.
    #[error("storage: {0}")]
    Storage(String),
}
