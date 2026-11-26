//! El resumen de una llamada terminada, en el canal: quiénes estuvieron,
//! cuánto duró y la grabación (`CallSummaryCard.tsx`).
//!
//! El servidor deja una línea de cac con `cac:call/<id>` al vaciarse la
//! sala; esto es lo que se pinta a partir de ese id. Mientras no llega, o si
//! no llega, se queda la frase del servidor: es lo mismo en una línea.

use chrono::{DateTime, TimeZone};
use serde::Deserialize;

use crate::chat::{short_time, Chat};
use crate::error::ApiError;
use crate::orgs::initials;
use crate::texts::{text, text_with, Locale};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSession {
    pub id: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub duration_ms: i64,
    /// Cuántos mensajes tiene su chat.
    #[serde(default)]
    pub messages: i64,
    #[serde(default)]
    pub participants: Vec<CallParticipant>,
    #[serde(default)]
    pub recording_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallParticipant {
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub guest: bool,
}

/// Quién estuvo, para pintar su cara.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFace {
    pub initials: String,
    /// Su nombre, con «(invitado)» si lo era: es lo que dice al pasar el ratón.
    pub name: String,
}

/// Lo que dice la tarjeta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallCard {
    pub title: String,
    /// «12 min · 10:02–10:14».
    pub when: String,
    /// Las primeras seis caras.
    pub faces: Vec<CallFace>,
    /// «3 personas: Ana, Luis, Zoe».
    pub people: String,
    /// La grabación, si se hizo: lo que abre «Ver la grabación».
    pub recording_id: Option<String>,
}

/// «45 s», «12 min», «1 h 05 min» (`callLength`, y `domain.CallLength` en el
/// servidor).
pub fn call_length(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    if ms < 60_000 {
        return format!("{} s", ms / 1000);
    }
    let minutes = (ms + 30_000) / 60_000;
    if minutes < 60 {
        return format!("{minutes} min");
    }
    format!("{} h {:02} min", minutes / 60, minutes % 60)
}

/// La tarjeta de esa llamada, en la zona de quien mira.
pub fn call_card<Viewer: TimeZone>(session: &CallSession, locale: Locale, now: &DateTime<Viewer>) -> CallCard {
    let zone = now.timezone();
    let span = session.ended_at.as_deref().map(|end| format!("{}–{}", short_time(&session.started_at, &zone, locale), short_time(end, &zone, locale)));
    let when = [call_length(session.duration_ms), span.unwrap_or_default()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
    let faces = session
        .participants
        .iter()
        .take(6)
        .map(|person| CallFace {
            initials: initials(Some(&person.name), ""),
            name: if person.guest { text_with(locale, "chat:callGuest", &[("name", &person.name)]) } else { person.name.clone() },
        })
        .collect();
    let names = session.participants.iter().map(|person| person.name.as_str()).collect::<Vec<_>>().join(", ");
    let people = if session.participants.is_empty() {
        String::new()
    } else {
        text_with(locale, &format!("chat:callPeople_{}", if session.participants.len() == 1 { "one" } else { "other" }), &[("count", &session.participants.len().to_string()), ("names", &names)])
    };
    CallCard { title: text(locale, "chat:callEnded"), when, faces, people, recording_id: session.recording_id.clone().filter(|id| !id.is_empty()) }
}

impl Chat {
    /// Pide la llamada y arma su tarjeta. Una respuesta que no es una llamada
    /// de verdad (un servidor de antes) es un fallo: se queda la frase.
    pub async fn call_card(&self, call_id: &str, locale: Locale) -> Result<CallCard, ApiError> {
        let session: CallSession = self.core.get(&format!("/api/v1/call-sessions/{call_id}")).await?;
        if session.id.is_empty() {
            return Err(ApiError::Server { status: 0, code: "not-a-call".into(), message: "That is not a call".into() });
        }
        Ok(call_card(&session, locale, &chrono::Local::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn a_call_is_measured_like_the_server_does() {
        assert_eq!(call_length(0), "");
        assert_eq!(call_length(45_000), "45 s");
        assert_eq!(call_length(12 * 60_000 + 29_000), "12 min");
        assert_eq!(call_length(12 * 60_000 + 30_000), "13 min");
        assert_eq!(call_length(65 * 60_000), "1 h 05 min");
    }

    fn session(guest: bool) -> CallSession {
        serde_json::from_value(serde_json::json!({
            "id": "0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8", "startedAt": "2026-11-25T10:02:00Z", "endedAt": "2026-11-25T10:14:00Z",
            "durationMs": 720000, "messages": 2,
            "participants": [
                { "identity": "u1", "name": "Ana Ruiz", "guest": false },
                { "identity": "g1", "name": "Luis Mora", "guest": guest },
            ],
            "recordingId": "r1"
        }))
        .unwrap()
    }

    #[test]
    fn the_card_says_who_came_how_long_and_where_the_recording_is() {
        let card = call_card(&session(true), Locale::Es, &Utc.with_ymd_and_hms(2026, 11, 25, 12, 0, 0).unwrap());
        assert_eq!(card.title, "Llamada terminada");
        assert_eq!(card.when, "12 min · 10:02–10:14");
        assert_eq!(card.faces.iter().map(|face| face.name.as_str()).collect::<Vec<_>>(), ["Ana Ruiz", "Luis Mora (invitado)"]);
        assert_eq!(card.people, "2 personas: Ana Ruiz, Luis Mora");
        assert_eq!(card.recording_id.as_deref(), Some("r1"));
    }

    #[test]
    fn without_a_recording_there_is_no_button() {
        let mut call = session(false);
        call.recording_id = Some(String::new());
        assert_eq!(call_card(&call, Locale::Es, &Utc::now()).recording_id, None);
    }
}
