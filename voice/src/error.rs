//! Los fallos del motor, como etiquetas estables.
//!
//! Como hace el motor de Tauri (`voice-no-mic`, `voice-not-in-room`…): la
//! etiqueta es lo que la interfaz traduce a una frase en su catálogo; el
//! detalle es para el diario, no para enseñarlo tal cual.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceError {
    /// `join` se llamó fuera de un runtime de tokio: LiveKit lo necesita.
    NoRuntime,
    /// No se pudo entrar a la sala. El detalle va sin el token.
    Connect(String),
    /// Dentro, pero el SDK no aceptó la pista del micrófono.
    Publish(String),
    NoMic,
    MicOpen(String),
    /// Abrir un dispositivo de audio puede colgarse (Windows reenumerando):
    /// se abandona a los quince segundos.
    MicTimeout,
    MicThreadDied,
    NoOutput,
    OutputOpen(String),
    OutputTimeout,
    NoCamera,
    /// La tiene otro programa (EBUSY).
    CameraBusy,
    /// Abrió pero no entrega imagen, o dejó de entregarla.
    CameraNoFrames,
    CameraFailed(String),
    /// Este sistema no trae capturador de pantalla.
    NoScreenCapture,
    /// El sistema (o el portal) dijo que no.
    ScreenDenied,
    /// El diálogo del sistema sigue sin contestar al minuto.
    ShareUnanswered,
    /// La captura no contestó ni para decir que no.
    ScreenTimeout,
    ScreenFailed(String),
    /// Ya se está pidiendo una pantalla y no ha contestado nadie.
    SharePending,
}

/// El nombre que pide la tarea; es el mismo tipo.
pub type JoinError = VoiceError;

impl VoiceError {
    pub fn label(&self) -> &'static str {
        match self {
            VoiceError::NoRuntime => "voice-no-runtime",
            VoiceError::Connect(_) => "voice-connect",
            VoiceError::Publish(_) => "voice-publish",
            VoiceError::NoMic => "voice-no-mic",
            VoiceError::MicOpen(_) => "voice-mic-open",
            VoiceError::MicTimeout => "voice-mic-timeout",
            VoiceError::MicThreadDied => "voice-mic-thread-died",
            VoiceError::NoOutput => "voice-no-output",
            VoiceError::OutputOpen(_) => "voice-output-open",
            VoiceError::OutputTimeout => "voice-output-timeout",
            VoiceError::NoCamera => "voice-no-camera",
            VoiceError::CameraBusy => "voice-camera-busy",
            VoiceError::CameraNoFrames => "voice-camera-no-frames",
            VoiceError::CameraFailed(_) => "voice-camera-thread-died",
            VoiceError::NoScreenCapture => "voice-no-screen-capture",
            VoiceError::ScreenDenied => "voice-screen-denied",
            VoiceError::ShareUnanswered => "voice-share-unanswered",
            VoiceError::ScreenTimeout => "voice-screen-timeout",
            VoiceError::ScreenFailed(_) => "voice-screen-thread-died",
            VoiceError::SharePending => "voice-share-pending",
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            VoiceError::Connect(d) | VoiceError::Publish(d) | VoiceError::MicOpen(d) | VoiceError::OutputOpen(d) | VoiceError::CameraFailed(d) | VoiceError::ScreenFailed(d) => Some(d),
            _ => None,
        }
    }
}

impl fmt::Display for VoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.detail() {
            Some(detail) => write!(f, "{}: {detail}", self.label()),
            None => f.write_str(self.label()),
        }
    }
}

impl std::error::Error for VoiceError {}

/// Quita un secreto de un texto antes de guardarlo: los errores del SDK pueden
/// llevar la URL de señalización, y ésa lleva el token de la sala.
pub(crate) fn redact(text: String, secret: &str) -> String {
    if secret.is_empty() {
        return text;
    }
    text.replace(secret, "<token>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_never_reaches_the_error() {
        let err = VoiceError::Connect(redact("wss://sfu/rtc?access_token=eyJ.abc.def falló".into(), "eyJ.abc.def"));
        assert_eq!(err.to_string(), "voice-connect: wss://sfu/rtc?access_token=<token> falló");
        assert_eq!(VoiceError::NoMic.to_string(), "voice-no-mic");
    }
}
