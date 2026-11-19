//! El timbre: llamar a alguien para que entre a la sala donde estás, y que te
//! llamen a ti. Y la mudanza de una llamada a una reunión.
//!
//! Vino de `app/src/store/voice.store.ts` (`timbrar`, `alTimbrar`…) y de
//! `hooks/use-report-events.ts`.
//!
//! **El servidor no guarda el timbre**: es un evento con un plazo
//! (`expiresAt`, veinte segundos), no un registro. Así que cada lado lleva su
//! reloj y se rinde por su cuenta, y por eso un timbre deja de sonar aunque la
//! app de quien llamaba se cierre de golpe y nadie mande la cancelación.
//!
//! Suena en todos los dispositivos de la persona y aceptar en uno no calla los
//! demás: es como lo hace Tauri, y jose decidió copiarlo tal cual (19-nov). El
//! arreglo es del backend: command-and-control-435.

use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::stream::ServerEvent;
use crate::voice::{IncomingRing, OutgoingRing, Voice, VoiceError, VoicePhase, VoiceState};

/// `voice.ring.cancel` (`domain.VoiceRingCancel`).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RingCancel {
    from: String,
}

/// `voice.moved`: un mapa suelto en el servidor (`service/call_invite.go`).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Moved {
    space_id: String,
    invite_id: String,
    title: String,
    actor_id: String,
}

/// A dónde va el timbre de la sala en la que estás: la del canal o la de la
/// reunión. Llamar a alguien «al canal» desde una reunión le llevaría a otra
/// parte.
fn ring_route(state: &VoiceState) -> Option<String> {
    if let Some(meet) = &state.meet_id {
        return Some(format!("/api/v1/call-invites/{meet}/ring"));
    }
    state.space_id.as_ref().map(|space| format!("/api/v1/task-spaces/{space}/voice/ring"))
}

/// Qué tono tiene que estar sonando ahora: el de te llaman, el de estás
/// llamando, o ninguno. Te llaman va antes (es lo que pide una respuesta), y
/// ninguno suena si estás sordo: quien se ensordece ha pedido que el
/// ordenador se calle. El de estás llamando se calla al rendirse.
pub fn ring_tone_now(state: &VoiceState) -> Option<crate::ring_tone::RingTone> {
    use crate::ring_tone::RingTone;
    if state.deaf {
        return None;
    }
    if state.incoming.is_some() {
        return Some(RingTone::Incoming);
    }
    state.outgoing.as_ref().filter(|ring| !ring.unanswered).map(|_| RingTone::Outgoing)
}

/// Cuánto lleva sonando un timbre, como un reloj: «0:07», «1:12».
pub fn ring_clock(started_at: &str, now: DateTime<Utc>) -> String {
    let seconds = DateTime::parse_from_rfc3339(started_at).map(|at| (now - at.with_timezone(&Utc)).num_seconds().max(0)).unwrap_or(0);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Cuánto falta para una hora RFC 3339; nada si ya pasó o no se entiende.
fn until(at: &str) -> Duration {
    DateTime::parse_from_rfc3339(at).ok().and_then(|at| (at.with_timezone(&Utc) - Utc::now()).to_std().ok()).unwrap_or_default()
}

impl Voice {
    /// A quién se puede llamar a esta sala: la gente de la organización que
    /// no está ya dentro. Llamar a quien te está oyendo es un pitido gratis
    /// para los dos.
    pub fn invitable(&self) -> Vec<crate::comms_wire::Person> {
        let state = self.state();
        let inside: Vec<&str> = state.people.iter().map(|peer| peer.identity.as_str()).chain(state.me.as_deref()).collect();
        self.core.comms().people("").into_iter().filter(|person| !inside.contains(&person.id.as_str())).collect()
    }

    fn next_ring_turn(&self) -> u64 {
        self.core.inner.ring_turn.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Hace sonar el escritorio de alguien de la organización para que entre
    /// a esta sala.
    pub async fn ring(&self, user_id: &str, name: &str) {
        let Some(route) = ring_route(&self.state()) else { return };
        let turn = self.next_ring_turn();
        self.change(|state| {
            state.outgoing = Some(OutgoingRing { user_id: user_id.into(), name: name.into(), started_at: Utc::now().to_rfc3339(), unanswered: false });
        });
        if let Err(error) = self.core.call_ok("POST", &route, Some(serde_json::json!({ "userId": user_id }))).await {
            // Una llamada que el servidor rechazó no está sonando en ninguna
            // parte, y enseñarla como si sonara es la peor mentira posible.
            if self.core.inner.ring_turn.load(Ordering::SeqCst) == turn {
                self.change(|state| {
                    state.outgoing = None;
                    state.error = Some(match &error {
                        crate::ApiError::Server { code, message, .. } => VoiceError { code: code.clone(), message: message.clone(), space_id: state.space_id.clone() },
                        other => VoiceError { code: String::new(), message: other.to_string(), space_id: state.space_id.clone() },
                    });
                });
            }
            return;
        }
        let voice = self.core.voice();
        let ring_for = self.core.inner.config.ring_for;
        tokio::spawn(async move {
            tokio::time::sleep(ring_for).await;
            if voice.core.inner.ring_turn.load(Ordering::SeqCst) == turn {
                voice.change(|state| {
                    if let Some(outgoing) = state.outgoing.as_mut() {
                        outgoing.unanswered = true;
                    }
                });
            }
        });
    }

    /// Deja de llamar. También quita el «no contestó» de en medio.
    pub async fn cancel_ring(&self) {
        self.next_ring_turn();
        let state = self.state();
        self.change(|state| state.outgoing = None);
        let (Some(route), Some(outgoing)) = (ring_route(&state), state.outgoing) else { return };
        let _ = self.core.call_ok("DELETE", &format!("{route}/{}", outgoing.user_id), None).await;
    }

    /// Entra a donde te llaman: al canal (aunque sea de otra organización) o
    /// a la reunión.
    pub async fn accept_ring(&self) {
        let Some(ring) = self.state().incoming else { return };
        self.change(|state| state.incoming = None);
        match ring.invite_id {
            Some(invite_id) => self.join_meeting(&invite_id).await,
            None => {
                let org = Some(ring.org_id).filter(|id| !id.is_empty());
                let name = Some(ring.space_name).filter(|name| !name.is_empty());
                self.join_channel(&ring.space_id, org, name).await
            }
        }
    }

    /// Dice que no, en vez de dejar que caduque: quien llamaba se entera ya y
    /// no dentro de veinte segundos.
    pub async fn decline_ring(&self) {
        let Some(ring) = self.state().incoming else { return };
        self.change(|state| state.incoming = None);
        let route = match &ring.invite_id {
            Some(invite) => format!("/api/v1/call-invites/{invite}/ring"),
            None => format!("/api/v1/task-spaces/{}/voice/ring", ring.space_id),
        };
        let _ = self.core.call_ok("DELETE", &format!("{route}/{}", ring.from.id), None).await;
    }

    /// Te llaman.
    fn rung(&self, ring: IncomingRing) {
        let state = self.state();
        // Ya estás en esa sala: la llamada llegó tarde o cruzada, y una
        // tarjeta que te invita a donde ya estás sólo tapa la conversación.
        let already = match &ring.invite_id {
            Some(invite) => state.meet_id.as_deref() == Some(invite.as_str()),
            None => state.meet_id.is_none() && state.space_id.as_deref() == Some(ring.space_id.as_str()),
        };
        if already && state.phase != VoicePhase::Out {
            return;
        }
        let ring_id = ring.ring_id.clone();
        let wait = until(&ring.expires_at);
        self.change(|state| state.incoming = Some(ring));
        let voice = self.core.voice();
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            if voice.state().incoming.is_some_and(|ring| ring.ring_id == ring_id) {
                voice.change(|state| state.incoming = None);
            }
        });
    }

    /// Colgó quien te llamaba, o dijo que no a quien llamabas tú: su rechazo
    /// vuelve por el mismo camino. Lo tuyo se queda en «no contestó» en vez
    /// de desaparecer: una fila que se esfuma no dice si te rechazaron o si
    /// el botón nunca hizo nada.
    fn hung_up(&self, from: &str) {
        let state = self.state();
        let incoming = state.incoming.as_ref().is_some_and(|ring| ring.from.id == from);
        let outgoing = state.outgoing.as_ref().is_some_and(|ring| ring.user_id == from);
        if outgoing {
            self.next_ring_turn();
        }
        if incoming || outgoing {
            self.change(|state| {
                if incoming {
                    state.incoming = None;
                }
                if let Some(ring) = state.outgoing.as_mut().filter(|_| outgoing) {
                    ring.unanswered = true;
                }
            });
        }
    }

    /// La llamada de un canal se mudó a una reunión. Si estás en ella (o
    /// entrando), te vas detrás; quien la mudó ya va por su cuenta.
    async fn follow_move(&self, moved: Moved) {
        let me = self.core.session().map(|session| session.id);
        if moved.space_id.is_empty() || moved.invite_id.is_empty() || me.as_deref() == Some(moved.actor_id.as_str()) {
            return;
        }
        if !self.state().in_channel(&moved.space_id) {
            return;
        }
        let invite = moved.invite_id.clone();
        self.change(|state| state.moved = Some(crate::voice::MovedCall { invite_id: moved.invite_id, title: moved.title }));
        self.join_meeting(&invite).await;
    }

    /// Los eventos del servidor que son de la voz.
    pub(crate) fn server_event(&self, event: &ServerEvent) {
        match event.event.as_str() {
            "voice.ring" => {
                if let Ok(ring) = serde_json::from_str::<IncomingRing>(&event.data) {
                    if !ring.ring_id.is_empty() {
                        self.rung(ring);
                    }
                }
            }
            "voice.ring.cancel" => {
                if let Ok(cancel) = serde_json::from_str::<RingCancel>(&event.data) {
                    if !cancel.from.is_empty() {
                        self.hung_up(&cancel.from);
                    }
                }
            }
            "voice.moved" => {
                if let Ok(moved) = serde_json::from_str::<Moved>(&event.data) {
                    let voice = self.core.voice();
                    tokio::spawn(async move { voice.follow_move(moved).await });
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Qué tono suena. Roturas que atrapa: sonar ensordecido; seguir
    /// sonando «llamando» tras rendirse; que el de llamar tape al de te
    /// llaman.
    #[test]
    fn which_ring_sounds() {
        use crate::ring_tone::RingTone;
        let out = OutgoingRing { user_id: "u2".into(), name: "Luis".into(), started_at: String::new(), unanswered: false };
        let mut state = VoiceState { outgoing: Some(out.clone()), ..VoiceState::default() };
        assert_eq!(ring_tone_now(&state), Some(RingTone::Outgoing));
        state.incoming = Some(IncomingRing::default());
        assert_eq!(ring_tone_now(&state), Some(RingTone::Incoming));
        state.deaf = true;
        assert_eq!(ring_tone_now(&state), None);
        let gave_up = VoiceState { outgoing: Some(OutgoingRing { unanswered: true, ..out }), ..VoiceState::default() };
        assert_eq!(ring_tone_now(&gave_up), None);
        assert_eq!(ring_tone_now(&VoiceState::default()), None);
    }

    /// El reloj del timbre. Rotura que atrapa: segundos sin el cero delante.
    #[test]
    fn the_ring_clock() {
        let now = DateTime::parse_from_rfc3339("2026-11-19T10:01:12Z").unwrap().with_timezone(&Utc);
        assert_eq!(ring_clock("2026-11-19T10:01:05Z", now), "0:07");
        assert_eq!(ring_clock("2026-11-19T10:00:00Z", now), "1:12");
        assert_eq!(ring_clock("no es una hora", now), "0:00");
    }

    /// A dónde va el timbre. Rotura que atrapa: llamar «al canal» desde una
    /// reunión que cuelga de él.
    #[test]
    fn a_meeting_rings_to_the_meeting() {
        let channel = VoiceState { space_id: Some("s1".into()), ..VoiceState::default() };
        assert_eq!(ring_route(&channel).as_deref(), Some("/api/v1/task-spaces/s1/voice/ring"));
        let meeting = VoiceState { meet_id: Some("m1".into()), meet_space_id: Some("s1".into()), ..VoiceState::default() };
        assert_eq!(ring_route(&meeting).as_deref(), Some("/api/v1/call-invites/m1/ring"));
        assert_eq!(ring_route(&VoiceState::default()), None);
    }
}
