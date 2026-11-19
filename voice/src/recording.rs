//! El aviso de grabación, leído del metadata de la sala.
//!
//! Sale del **metadata** y no de un mensaje aparte: LiveKit se lo entrega a
//! cualquiera que se conecte, así que quien entra tarde ve el REC sin que nadie
//! se lo repita. Lo escribe el backend (`markRoom` en
//! `backend/internal/core/service/recording.go`): `{"recording":{…}}` mientras
//! se graba y `{}` al parar.

use crate::events::EngineEvent;

/// Lo que dice el metadata cuando hay grabación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSignal {
    pub id: String,
    /// El id de quien la empezó.
    pub by: String,
    /// Cuándo empezó, como lo escribe el backend (RFC 3339). Puede venir vacío.
    pub since: String,
}

/// Lee el metadata de la sala y dice si hay grabación.
///
/// El metadata lo escribe otro proceso y llega de todas las formas: vacío (sala
/// recién creada), `{}` (se acaba de parar), roto. Ninguna es un error.
///
/// **Se exigen `id` y `by`.** Un objeto a medias no enciende el aviso:
/// preferimos no avisar a avisar de una grabación que no se sabe de quién es.
pub fn recording_from_metadata(raw: &str) -> Option<RecordingSignal> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let recording = value.get("recording")?;
    let id = recording.get("id")?.as_str()?;
    let by = recording.get("by")?.as_str()?;
    if id.is_empty() || by.is_empty() {
        return None;
    }
    Some(RecordingSignal {
        id: id.to_string(),
        by: by.to_string(),
        since: recording.get("since").and_then(|s| s.as_str()).unwrap_or("").to_string(),
    })
}

/// El evento que corresponde a un metadata, encendido o apagado.
pub(crate) fn recording_event(raw: &str) -> EngineEvent {
    match recording_from_metadata(raw) {
        Some(r) => EngineEvent::Recording { active: true, id: r.id, by: r.by, since: r.since },
        None => EngineEvent::Recording { active: false, id: String::new(), by: String::new(), since: String::new() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lo que escribe `markRoom` de verdad: `json.Marshal` de
    /// `{"recording": domain.RecordingSignal}`, con `since` como lo serializa
    /// Go un `time.Time` (RFC 3339 con nanosegundos) y las claves opcionales
    /// (`spaceId`, `room`, `inviteId`) presentes.
    const BACKEND_WHILE_RECORDING: &str = r#"{"recording":{"id":"0b7c3e2a-5f1d-4c47-9a0e-2d1f6b8c9e10","by":"34bc78c8-cca2-4a06-a788-5e70c63dcec5","since":"2026-11-19T18:25:09.226586Z","spaceId":"e8bd6d46-a9ab-494e-ac29-de84d80a55f4","room":"meet-e8bd6d46-a9ab-494e-ac29-de84d80a55f4-7","inviteId":"5a1c9b0e-3d2f-4e6a-8b7c-1f0e9d8c7b6a"}}"#;

    /// Y lo que deja al parar: `payload := "{}"`.
    const BACKEND_STOPPED: &str = "{}";

    #[test]
    fn the_backend_signal_lights_the_warning() {
        assert_eq!(
            recording_from_metadata(BACKEND_WHILE_RECORDING),
            Some(RecordingSignal {
                id: "0b7c3e2a-5f1d-4c47-9a0e-2d1f6b8c9e10".into(),
                by: "34bc78c8-cca2-4a06-a788-5e70c63dcec5".into(),
                since: "2026-11-19T18:25:09.226586Z".into(),
            })
        );
        assert_eq!(
            recording_event(BACKEND_WHILE_RECORDING),
            EngineEvent::Recording {
                active: true,
                id: "0b7c3e2a-5f1d-4c47-9a0e-2d1f6b8c9e10".into(),
                by: "34bc78c8-cca2-4a06-a788-5e70c63dcec5".into(),
                since: "2026-11-19T18:25:09.226586Z".into(),
            }
        );
    }

    /// Un canal (sin invitación) no lleva `inviteId`: Go lo omite.
    #[test]
    fn a_channel_recording_without_invite_counts() {
        let raw = r#"{"recording":{"id":"rec-1","by":"u-ana","since":"2026-11-19T18:25:09Z","spaceId":"s-1","room":"space-s-1"}}"#;
        assert_eq!(recording_from_metadata(raw).map(|r| r.id), Some("rec-1".into()));
    }

    #[test]
    fn stopped_or_unreadable_is_off() {
        for raw in [BACKEND_STOPPED, "", "no soy json", "null", "[]", r#"{"recording":null}"#, r#"{"otra":"cosa"}"#] {
            assert_eq!(recording_from_metadata(raw), None, "con {raw:?}");
            assert!(matches!(recording_event(raw), EngineEvent::Recording { active: false, .. }), "con {raw:?}");
        }
    }

    /// A medias no cuenta: sin dueño no hay aviso.
    #[test]
    fn half_a_signal_is_off() {
        for raw in [
            r#"{"recording":{}}"#,
            r#"{"recording":{"id":"rec-1"}}"#,
            r#"{"recording":{"by":"u-ana"}}"#,
            r#"{"recording":{"id":"","by":"u-ana"}}"#,
            r#"{"recording":{"id":"rec-1","by":""}}"#,
            r#"{"recording":{"id":1,"by":"u-ana"}}"#,
        ] {
            assert_eq!(recording_from_metadata(raw), None, "con {raw}");
        }
    }

    /// Sin `since` el aviso sigue valiendo.
    #[test]
    fn without_since_the_warning_stays_on() {
        let r = recording_from_metadata(r#"{"recording":{"id":"rec-1","by":"u-ana"}}"#).unwrap();
        assert_eq!(r.since, "");
    }
}
