//! `cac-voice`: el motor de voz de la app nativa de cac (command-and-control-427).
//!
//! Entra a una sala de LiveKit, publica el micrófono procesado (eco, paso
//! alto, ruido), mezcla a los demás en una sola salida —que es también la
//! referencia del cancelador de eco— y cuenta lo que pasa por un
//! `Fn(EngineEvent)`. Recibe el vídeo de los demás (cámara y pantalla) y lo
//! deja listo para pintar; no publica vídeo. Sin interfaz.
//!
//! ```ignore
//! let engine = Engine::new();
//! let call = engine.join(&url, &token, Arc::new(|event| println!("{event:?}"))).await?;
//! call.set_mic(false);
//! call.leave().await;
//! ```
//!
//! **Todo el estado vive en la instancia.** El spike y el motor de Tauri
//! usaban globales (`TRACKS`, `RENDER_REFERENCE`, `DEAFENED`…); aquí son de
//! `Engine` (lo que sobrevive a una llamada) o de `Call`. Lo guarda
//! `tests::no_global_state`.
//!
//! De dónde sale cada cosa: el audio, de `spikes/calls-native/src/call.rs`
//! (ya en LiveKit 0.12); silenciar, ensordecer, dispositivos, medidor, quién
//! habla, latencia y grabación, de `app/src-tauri/src/voice.rs` (en 0.8.3).

mod capture;
mod devices;
mod dsp;
mod engine;
mod error;
mod events;
mod mixer;
mod output;
mod recording;
mod video;

pub use capture::InputFormat;
pub use devices::{Device, DeviceKind, Devices};
pub use engine::{Call, Engine, MicLevel};
pub use error::{JoinError, VoiceError};
pub use events::{EngineEvent, EventSink};
pub use recording::{recording_from_metadata, RecordingSignal};
pub use video::{FrameWake, VideoFeeds, VideoFrame, VideoSource};

#[cfg(test)]
mod tests {
    /// Las fuentes del crate, para leerlas.
    const SOURCES: [(&str, &str); 11] = [
        ("lib.rs", include_str!("lib.rs")),
        ("capture.rs", include_str!("capture.rs")),
        ("devices.rs", include_str!("devices.rs")),
        ("dsp.rs", include_str!("dsp.rs")),
        ("engine.rs", include_str!("engine.rs")),
        ("error.rs", include_str!("error.rs")),
        ("events.rs", include_str!("events.rs")),
        ("mixer.rs", include_str!("mixer.rs")),
        ("output.rs", include_str!("output.rs")),
        ("recording.rs", include_str!("recording.rs")),
        ("video.rs", include_str!("video.rs")),
    ];

    /// Ni un global: ni `static` (las constantes sí), ni `thread_local!`, ni
    /// `LazyLock`/`OnceLock`. Con un global, dos `Engine` volverían a
    /// compartir pistas o referencia del eco sin que ninguna otra prueba lo
    /// notara si cada una usa un solo motor.
    #[test]
    fn no_global_state() {
        // Las palabras se arman por partes para que la prueba no se encuentre
        // a sí misma.
        let keyword = concat!("sta", "tic");
        let others = [concat!("thread", "_local!"), concat!("Lazy", "Lock"), concat!("Once", "Lock"), concat!("lazy", "_static")];
        for (file, source) in SOURCES {
            for (n, line) in source.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                // `'static` es un tiempo de vida, no un global: el apóstrofo
                // se queda pegado a la palabra.
                let mut words = code.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\''));
                let global = words.any(|w| w == keyword) || others.iter().any(|o| code.contains(o));
                assert!(!global, "{file}:{}: estado global: {line}", n + 1);
            }
        }
    }
}
