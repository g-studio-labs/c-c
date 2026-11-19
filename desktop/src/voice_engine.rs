//! El motor de voz de esta app: `cac-voice` (LiveKit) puesto detrás del rasgo
//! que el core define (`cac_core::VoiceEngine`).
//!
//! Aquí no se decide nada. El core dice cuándo entrar, salir, silenciar o
//! ensordecer; esto se lo pasa al motor y le devuelve lo que cuenta la sala.
//! Ensordecer: el core manda también «micrófono cerrado», y por eso al quitar
//! la sordera el micrófono sigue cerrado aunque el motor, solo, lo dejaría
//! como estaba (`Call::set_deaf`).
//!
//! Sólo en Linux por ahora (ver `Cargo.toml`). En macOS y Windows no se pone
//! ninguno y el core dice que esta app todavía no tiene llamadas.

use std::sync::{Arc, Mutex};

use cac_core::{Core, VoiceEngine, VoiceEvent, VoiceFuture};
use cac_voice::{Call, EngineEvent, EventSink};

/// Lo que cuenta el motor, en el idioma del core.
fn to_core(event: EngineEvent) -> VoiceEvent {
    match event {
        EngineEvent::Connected { identity } => VoiceEvent::Connected { identity },
        EngineEvent::Joined { identity, name } => VoiceEvent::Joined { identity, name },
        EngineEvent::Left { identity } => VoiceEvent::Left { identity },
        EngineEvent::Speaking { identities } => VoiceEvent::Speaking { identities },
        EngineEvent::SelfSpeaking { speaking } => VoiceEvent::SelfSpeaking { speaking },
        EngineEvent::Muted { identity, muted } => VoiceEvent::Muted { identity, muted },
        EngineEvent::Latency { ms } => VoiceEvent::Latency { ms },
        EngineEvent::Recording { active, id, by, since } => VoiceEvent::Recording { active, id, by, since },
        EngineEvent::Disconnected { reason } => VoiceEvent::Disconnected { reason },
    }
}

/// El motor, para lo que no pasa por el core: elegir micrófono y altavoz.
pub struct Devices(pub cac_voice::Engine);

impl gpui_kit::Global for Devices {}

/// El motor de verdad, con la llamada en curso.
pub struct LiveVoice {
    engine: cac_voice::Engine,
    call: Arc<Mutex<Option<Call>>>,
    core: Core,
}

impl LiveVoice {
    pub fn new(core: Core) -> Self {
        LiveVoice { engine: cac_voice::Engine::new(), call: Arc::default(), core }
    }

    /// El motor, para lo que no pasa por el core: los dispositivos.
    pub fn engine(&self) -> cac_voice::Engine {
        self.engine.clone()
    }

    fn with_call(&self, act: impl FnOnce(&Call)) {
        if let Some(call) = self.call.lock().expect("voice call lock").as_ref() {
            act(call);
        }
    }
}

impl VoiceEngine for LiveVoice {
    fn join(&self, url: String, token: String, noise_suppression: bool) -> VoiceFuture<Result<String, String>> {
        let (engine, slot, core) = (self.engine.clone(), self.call.clone(), self.core.clone());
        Box::pin(async move {
            engine.set_noise_suppression(noise_suppression);
            // Se llama desde las tareas del motor: el core sólo apunta y
            // avisa, no bloquea.
            let sink: EventSink = Arc::new(move |event| core.voice().engine_event(to_core(event)));
            let call = engine.join(&url, &token, sink).await.map_err(|error| error.label().to_string())?;
            let identity = call.identity().to_string();
            *slot.lock().expect("voice call lock") = Some(call);
            Ok(identity)
        })
    }

    fn leave(&self) -> VoiceFuture<()> {
        let call = self.call.lock().expect("voice call lock").take();
        Box::pin(async move {
            if let Some(call) = call {
                call.leave().await;
            }
        })
    }

    fn set_mic(&self, on: bool) {
        self.with_call(|call| call.set_mic(on));
    }

    fn set_deaf(&self, on: bool) {
        self.with_call(|call| call.set_deaf(on));
    }

    fn set_noise_suppression(&self, on: bool) {
        self.engine.set_noise_suppression(on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada evento del motor llega al core como el mismo evento.
    /// Rotura que atrapa: cruzar dos (un silencio que se lee como que se fue).
    #[test]
    fn every_event_keeps_its_meaning() {
        let cases = [
            (EngineEvent::Connected { identity: "u1".into() }, VoiceEvent::Connected { identity: "u1".into() }),
            (EngineEvent::Joined { identity: "u2".into(), name: "Luis".into() }, VoiceEvent::Joined { identity: "u2".into(), name: "Luis".into() }),
            (EngineEvent::Left { identity: "u2".into() }, VoiceEvent::Left { identity: "u2".into() }),
            (EngineEvent::Speaking { identities: vec!["u2".into()] }, VoiceEvent::Speaking { identities: vec!["u2".into()] }),
            (EngineEvent::SelfSpeaking { speaking: true }, VoiceEvent::SelfSpeaking { speaking: true }),
            (EngineEvent::Muted { identity: "u2".into(), muted: true }, VoiceEvent::Muted { identity: "u2".into(), muted: true }),
            (EngineEvent::Latency { ms: 40 }, VoiceEvent::Latency { ms: 40 }),
            (
                EngineEvent::Recording { active: true, id: "r".into(), by: "u2".into(), since: "t".into() },
                VoiceEvent::Recording { active: true, id: "r".into(), by: "u2".into(), since: "t".into() },
            ),
            (EngineEvent::Disconnected { reason: "RoomDeleted".into() }, VoiceEvent::Disconnected { reason: "RoomDeleted".into() }),
        ];
        for (from, to) in cases {
            assert_eq!(to_core(from), to);
        }
    }
}
