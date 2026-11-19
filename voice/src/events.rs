//! Lo que el motor le cuenta a quien lo pinta.
//!
//! El mismo vocabulario que `VoiceEvent` del motor de Tauri
//! (`app/src-tauri/src/voice.rs`), sin los de vídeo, y con la misma forma al
//! serializar (`{"kind":"joined", …}`) por si algún día viaja.

use std::sync::Arc;

use serde::Serialize;

use crate::recording::recording_event;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EngineEvent {
    /// Dentro. `identity` es la que acuñó el token (el id de usuario de cac).
    Connected { identity: String },
    Joined { identity: String, name: String },
    Left { identity: String },
    /// Quién habla ahora, según el servidor. La lista entera y no un delta:
    /// reconstruir el conjunto a base de altas y bajas es cómo se queda un
    /// punto verde encendido para siempre por un evento perdido.
    Speaking { identities: Vec<String> },
    /// **Tú** estás hablando, medido de tu propio micrófono (después del
    /// procesado y del silencio). Aparte de `Speaking` porque el servidor
    /// tarda y puede no incluirte nunca.
    SelfSpeaking { speaking: bool },
    /// El micrófono de alguien, uno a uno: el SDK avisa de la pista que cambia.
    /// El propio también llega, como confirmación del servidor.
    Muted { identity: String, muted: bool },
    /// Ida y vuelta al SFU, en milisegundos, cada cinco segundos.
    Latency { ms: u32 },
    /// Se está grabando, o se dejó de grabar. `id`, `by` y `since` vacíos
    /// cuando `active` es falso.
    Recording { active: bool, id: String, by: String, since: String },
    /// La sala te soltó. El micrófono sigue abierto hasta que quien lo
    /// recibe llama a `Call::leave` (como en Tauri: decide la interfaz).
    /// Colgar con `Call::leave` **no** lo emite.
    Disconnected { reason: String },
}

/// A quién se le cuentan los eventos. Se llama desde las tareas del motor
/// (otro hilo): tiene que volver rápido y no bloquear.
pub type EventSink = Arc<dyn Fn(EngineEvent) + Send + Sync>;

/// Alguien que ya estaba al entrar, reducido a lo que hace falta contar.
pub(crate) struct Peer {
    pub identity: String,
    pub name: String,
    /// `None` si todavía no publica micrófono: no se sabe si está abierto, y
    /// no se afirma ni una cosa ni la otra.
    pub mic_muted: Option<bool>,
}

/// Lo que hay que contar al entrar a una sala que ya tenía gente, en el orden
/// en que se habría oído estando desde el principio (voice.rs:2066-2091).
///
/// El SDK no manda `ParticipantConnected` por quien ya estaba, ni
/// `RoomMetadataChanged` por una grabación que ya estaba en marcha: sin esto,
/// entrar a una conversación en curso enseña una sala vacía y sin REC.
pub(crate) fn arrival_events(peers: &[Peer], metadata: &str) -> Vec<EngineEvent> {
    let mut events = Vec::with_capacity(peers.len() * 2 + 1);
    for peer in peers {
        events.push(EngineEvent::Joined { identity: peer.identity.clone(), name: peer.name.clone() });
        if let Some(muted) = peer.mic_muted {
            events.push(EngineEvent::Muted { identity: peer.identity.clone(), muted });
        }
    }
    events.push(recording_event(metadata));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada uno que ya estaba: entra, y su micro si lo publica; y al final si
    /// se graba.
    #[test]
    fn arriving_tells_who_was_already_there() {
        let peers = [
            Peer { identity: "u-ana".into(), name: "Ana".into(), mic_muted: Some(true) },
            Peer { identity: "u-luis".into(), name: "Luis".into(), mic_muted: None },
        ];
        let events = arrival_events(&peers, r#"{"recording":{"id":"rec-1","by":"u-ana","since":"x"}}"#);
        assert_eq!(
            events,
            vec![
                EngineEvent::Joined { identity: "u-ana".into(), name: "Ana".into() },
                EngineEvent::Muted { identity: "u-ana".into(), muted: true },
                EngineEvent::Joined { identity: "u-luis".into(), name: "Luis".into() },
                EngineEvent::Recording { active: true, id: "rec-1".into(), by: "u-ana".into(), since: "x".into() },
            ]
        );
    }

    /// Sala vacía y sin grabar: sólo el REC apagado, que también es estado.
    #[test]
    fn an_empty_room_still_says_it_is_not_recording() {
        assert_eq!(arrival_events(&[], ""), vec![EngineEvent::Recording { active: false, id: String::new(), by: String::new(), since: String::new() }]);
    }

    /// La forma en el cable, la de Tauri.
    #[test]
    fn events_serialize_like_tauri() {
        let json = serde_json::to_string(&EngineEvent::SelfSpeaking { speaking: true }).unwrap();
        assert_eq!(json, r#"{"kind":"selfSpeaking","speaking":true}"#);
    }
}
