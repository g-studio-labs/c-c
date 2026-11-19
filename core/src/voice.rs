//! La voz: la sala en la que estás, quién está contigo y quién anda por los
//! canales.
//!
//! Vino de `app/src/store/voice.store.ts` y `voice-occupancy.ts`. Aquí no hay
//! audio: el motor (LiveKit, el micrófono, los altavoces) lo pone cada
//! plataforma por el rasgo `VoiceEngine`, y lo que pasa dentro de la sala
//! vuelve por `Voice::engine_event`. Lo que se decide —a qué sala se entra,
//! qué hace ensordecer, cuándo se pregunta quién hay— vive aquí, para que el
//! escritorio y el móvil no lo decidan cada uno a su manera.
//!
//! - **Una sala a la vez.** Entrar en otra sale de la anterior: dos
//!   micrófonos abiertos es un fallo que sólo se nota cuando alguien te oye
//!   desde donde no estabas.
//! - **Cada entrada lleva su turno.** Si mientras conecta se sale, o se entra
//!   a otra, la conexión que llega tarde se cuelga en vez de dejar un
//!   micrófono abierto donde nadie lo ve.
//! - **Estar dentro y estar mirando son dos cosas**: minimizar no cuelga.
//! - **La ocupación se pregunta**, cada quince segundos y sólo mientras
//!   alguien la mira, en vez de llevar la cuenta con eventos: un recuento
//!   propio se desincroniza con el primer evento perdido y entonces miente sin
//!   que nada falle.
//!
//! El timbre está en `voice_ring.rs`; los tonos, en `ring_tone.rs`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::texts::{has_text, text, Locale};
use crate::Core;

/// Bajo qué clave se guarda si se quiere la supresión de ruido.
const NOISE_KEY: &str = "voice-noise-suppression";
/// Y el micrófono y el altavoz elegidos a mano.
const DEVICES_KEY: &str = "voice-devices";

/// El micrófono y el altavoz elegidos a mano en esta máquina, por el id que
/// les da el motor. `None` es «el del sistema».
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeviceChoices {
    pub input: Option<String>,
    pub output: Option<String>,
}

/// Lo que devuelve el motor cuando tarda.
pub type VoiceFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// El motor de voz de cada plataforma. El core lo define y no lo implementa:
/// en el escritorio es `cac-voice` (LiveKit), en las pruebas uno de mentira.
///
/// Los fallos van como **etiquetas** (`voice-no-mic`), no como frases: la
/// frase la pone quien la lee, en su idioma (`voice_error_phrase`).
pub trait VoiceEngine: Send + Sync {
    /// Conecta con la sala. Lo que pase dentro llega por
    /// `Voice::engine_event`. Devuelve la identidad propia en la sala.
    fn join(&self, url: String, token: String, noise_suppression: bool) -> VoiceFuture<Result<String, String>>;
    /// Sale de la sala, si hay alguna. No falla: salir de nada es nada.
    fn leave(&self) -> VoiceFuture<()>;
    /// Abre o cierra el micrófono.
    fn set_mic(&self, on: bool);
    /// Ensordece: no se reproduce nada de lo que llega.
    fn set_deaf(&self, on: bool);
    fn set_noise_suppression(&self, on: bool);
}

/// Lo que cuenta el motor de lo que pasa en la sala. Los mismos que el motor
/// de Tauri (`VoiceEvent` en `app/src-tauri/src/voice.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum VoiceEvent {
    /// Ya dentro, con esta identidad.
    Connected { identity: String },
    /// Alguien está en la sala. Al conectar llega uno por cada uno que ya
    /// estaba.
    Joined { identity: String, name: String },
    Left { identity: String },
    /// Quién habla ahora, **de los demás**: la lista entera, no un cambio.
    Speaking { identities: Vec<String> },
    /// Si hablas tú, medido de tu micrófono y no por el servidor.
    SelfSpeaking { speaking: bool },
    Muted { identity: String, muted: bool },
    Latency { ms: u32 },
    /// Alguien graba la sala (lo dice el metadata de la sala), o dejó de
    /// hacerlo.
    Recording { active: bool, id: String, by: String, since: String },
    /// Se cortó, con el motivo de LiveKit (`ParticipantRemoved`…).
    Disconnected { reason: String },
}

/// En qué punto está la llamada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum VoicePhase {
    #[default]
    Out,
    /// Pidiendo el pase y conectando.
    Joining,
    In,
}

/// Alguien en una sala.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct VoicePeer {
    pub identity: String,
    pub name: String,
}

/// Quién graba la llamada, para el chip REC.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct VoiceRecording {
    pub id: String,
    pub by: String,
    pub since: String,
}

/// Algo salió mal. `code` es la etiqueta (`voice-no-mic`); `message`, lo que
/// llegó, para cuando la etiqueta no está en el catálogo.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct VoiceError {
    pub code: String,
    pub message: String,
    /// De qué canal es: el botón de entrar de cada canal enseña lo suyo y
    /// calla lo ajeno.
    pub space_id: Option<String>,
}

/// La reunión a la que se mudó una llamada.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct MovedCall {
    pub invite_id: String,
    pub title: String,
}

/// Una llamada tuya que todavía no ha contestado nadie.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OutgoingRing {
    pub user_id: String,
    pub name: String,
    /// Cuándo empezó a sonar (RFC 3339), para el contador.
    pub started_at: String,
    /// Se rindió: veinte segundos sin respuesta, o el otro dijo que no.
    pub unanswered: bool,
}

/// Quién llama (`domain.VoiceCaller`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct VoiceCaller {
    pub id: String,
    pub name: String,
}

/// Te llaman (`domain.VoiceRing`, el evento `voice.ring`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct IncomingRing {
    pub ring_id: String,
    pub space_id: String,
    /// La organización del canal: te puede llamar alguien de otra.
    pub org_id: String,
    pub space_name: String,
    pub from: VoiceCaller,
    /// RFC 3339. Pasada esa hora la tarjeta se va sola.
    pub expires_at: String,
    /// Te llaman a una reunión, no a la sala del canal: aceptar lleva a ella.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_id: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
}

/// La voz, entera: la sala en la que estás, el timbre y quién anda por los
/// canales.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct VoiceState {
    pub phase: VoicePhase,
    /// El canal cuya sala está abierta. **En una reunión es `None`**, aunque
    /// la reunión cuelgue de un canal: el botón de entrar de ese canal tiene
    /// que ver que no estás en él.
    pub space_id: Option<String>,
    /// La reunión en la que estás.
    pub meet_id: Option<String>,
    /// El canal del que cuelga la reunión.
    pub meet_space_id: Option<String>,
    /// La llamada guardada de la sala, de la que cuelga su chat.
    pub session_id: Option<String>,
    /// El título de la reunión.
    pub title: Option<String>,
    /// De qué organización y canal es la sala: la llamada sigue al cambiar
    /// de organización, y entonces el árbol ya no sabe cómo se llama.
    pub org_id: Option<String>,
    pub space_name: Option<String>,
    /// La sala ocupa la pantalla. Minimizar sólo quita esto.
    pub stage_open: bool,
    /// Quién está dentro, tú incluido.
    pub people: Vec<VoicePeer>,
    /// Quién habla, de los demás.
    pub speaking: Vec<String>,
    pub self_speaking: bool,
    /// Quién tiene el micrófono cerrado. Un mapa: «no sé» y «abierto» no son
    /// lo mismo.
    pub muted: HashMap<String, bool>,
    pub latency_ms: Option<u32>,
    /// Tu identidad en la sala.
    pub me: Option<String>,
    pub mic: bool,
    pub deaf: bool,
    pub recording: Option<VoiceRecording>,
    pub error: Option<VoiceError>,
    /// A quién estás llamando.
    pub outgoing: Option<OutgoingRing>,
    /// Quién te llama.
    pub incoming: Option<IncomingRing>,
    /// La llamada se mudó a una reunión y te fuiste con ella. Quien pinta lo
    /// dice una vez y lo limpia (`clear_moved`).
    pub moved: Option<MovedCall>,
    /// Quién está en la voz de cada canal de la organización que se mira,
    /// sin haber entrado.
    pub occupancy: HashMap<String, Vec<VoicePeer>>,
    /// Si se quiere la supresión de ruido. Se recuerda en esta máquina.
    pub noise_suppression: bool,
}

impl Default for VoiceState {
    fn default() -> Self {
        VoiceState {
            phase: VoicePhase::Out,
            space_id: None,
            meet_id: None,
            meet_space_id: None,
            session_id: None,
            title: None,
            org_id: None,
            space_name: None,
            stage_open: false,
            people: Vec::new(),
            speaking: Vec::new(),
            self_speaking: false,
            muted: HashMap::new(),
            latency_ms: None,
            me: None,
            mic: true,
            deaf: false,
            recording: None,
            error: None,
            outgoing: None,
            incoming: None,
            moved: None,
            occupancy: HashMap::new(),
            noise_suppression: true,
        }
    }
}

impl VoiceState {
    /// Sin sala: todo lo de la llamada vuelve a cero, y se queda lo que no es
    /// de ella (quién te llama, la mudanza, quién anda por los canales, la
    /// preferencia de ruido). La grabación también se va: el punto rojo de
    /// la última llamada encendido en la siguiente es la clase de mentira que
    /// nadie se para a comprobar.
    fn outside(&self) -> VoiceState {
        VoiceState {
            incoming: self.incoming.clone(),
            moved: self.moved.clone(),
            occupancy: self.occupancy.clone(),
            noise_suppression: self.noise_suppression,
            ..VoiceState::default()
        }
    }

    /// Si estás en la sala de este canal (o entrando).
    pub fn in_channel(&self, space_id: &str) -> bool {
        self.phase != VoicePhase::Out && self.space_id.as_deref() == Some(space_id)
    }
}

/// El pase de un canal (`domain.VoiceTokenResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VoiceToken {
    pub url: String,
    pub token: String,
    pub room: String,
    pub org_id: String,
    pub space_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<MovedCall>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session_id: String,
}

/// El pase de una reunión (`domain.MeetTokenResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeetToken {
    pub url: String,
    pub token: String,
    pub room: String,
    pub org_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    pub invite_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session_id: String,
}

/// La frase de un fallo de la voz, en el idioma de quien lo lee. Una
/// etiqueta que esta versión no conozca se queda con lo que llegó.
pub fn voice_error_phrase(error: &VoiceError, locale: Locale) -> String {
    let key = format!("errors:{}", error.code);
    if !error.code.is_empty() && has_text(&key) {
        text(locale, &key)
    } else if !error.message.is_empty() {
        error.message.clone()
    } else {
        text(locale, "errors:voice-failed")
    }
}

/// Los motivos de corte que se le dicen a alguien. Los que no son un fallo
/// —salir tú, o no saber por qué— no se dicen (`None`); uno que no se conoce
/// se dice tal cual, porque callarlo escondería un fallo nuevo.
pub fn disconnect_reason(reason: &str) -> Option<String> {
    match reason {
        "Unknown" | "UnknownReason" | "ClientInitiated" | "" => None,
        "ParticipantRemoved" => Some("voice-removed".into()),
        "RoomDeleted" => Some("voice-room-closed".into()),
        "DuplicateIdentity" => Some("voice-joined-elsewhere".into()),
        other => Some(other.into()),
    }
}

/// Las iniciales de una persona, como `iniciales` de `app/src/lib/desde.ts`:
/// la primera letra de las dos primeras palabras, o las dos primeras letras
/// de un nombre de una sola palabra («Luis Mora» → «LM», «ana» → «AN»).
pub fn person_initials(name: &str) -> String {
    let clean = name.trim().trim_start_matches('@').trim();
    if clean.is_empty() {
        return "?".into();
    }
    let parts: Vec<&str> = clean.split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-')).filter(|part| !part.is_empty()).collect();
    let picked: String = if parts.len() >= 2 {
        parts[0].chars().take(1).chain(parts[1].chars().take(1)).collect()
    } else {
        clean.chars().take(2).collect()
    };
    picked.to_uppercase()
}

/// Unos nombres en una frase: «Ana», «Ana y Luis», «Ana, Luis y Eva» (en
/// inglés con la coma de serie). Son las personas que están, todas a la vez.
pub fn join_names(names: &[String], locale: Locale) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => {
            let and = match locale {
                Locale::Es => " y ",
                _ if rest.len() > 1 => ", and ",
                _ => " and ",
            };
            format!("{}{and}{last}", rest.join(", "))
        }
    }
}

/// Un recuadro de la sala.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct VoiceTile {
    pub identity: String,
    pub name: String,
    pub initials: String,
    /// El borde verde: el único indicador de que alguien habla.
    pub speaking: bool,
    pub muted: bool,
    pub me: bool,
}

/// Los recuadros de la sala: tú primero aunque el motor sólo cuente a los
/// demás. Lo tuyo sale de ti —si hablas, de tu micrófono; si estás mudo, de
/// tu botón, que es optimista—; lo de los demás, del servidor.
pub fn voice_tiles(state: &VoiceState, locale: Locale) -> Vec<VoiceTile> {
    let mut tiles = Vec::new();
    if let Some(me) = &state.me {
        let name = text(locale, "calls:you");
        tiles.push(VoiceTile { identity: me.clone(), initials: person_initials(&name), name, speaking: state.self_speaking, muted: !state.mic, me: true });
    }
    for peer in state.people.iter().filter(|peer| Some(&peer.identity) != state.me.as_ref()) {
        let name = if peer.name.is_empty() { peer.identity.clone() } else { peer.name.clone() };
        tiles.push(VoiceTile {
            identity: peer.identity.clone(),
            initials: person_initials(&name),
            name,
            speaking: state.speaking.contains(&peer.identity),
            muted: state.muted.get(&peer.identity).copied().unwrap_or(false),
            me: false,
        });
    }
    tiles
}

/// Cómo se llama la sala: el título de la reunión, o el canal.
pub fn voice_room_name(state: &VoiceState, locale: Locale) -> String {
    if state.meet_id.is_some() {
        return state.title.clone().filter(|title| !title.is_empty()).unwrap_or_else(|| text(locale, "calls:untitled"));
    }
    state.space_name.clone().filter(|name| !name.is_empty()).unwrap_or_else(|| text(locale, "common:servers.voice"))
}

/// La línea de al lado del nombre: «conectando…», «nadie más todavía» o
/// «3 en la llamada», y la latencia sólo cuando se sabe (un «0 ms» mientras
/// conecta se leería como una llamada perfecta justo cuando todavía no lo es).
pub fn voice_status(state: &VoiceState, locale: Locale) -> String {
    let inside = voice_tiles(state, locale).len() as i64;
    let mut line = if state.phase == VoicePhase::Joining {
        text(locale, "calls:connecting")
    } else if inside <= 1 {
        text(locale, "calls:nobodyYet")
    } else {
        crate::texts::text_count(locale, "calls:inVoice", inside)
    };
    if let (VoicePhase::In, Some(ms)) = (state.phase, state.latency_ms) {
        line.push_str(&format!(" · {ms} ms"));
    }
    line
}

fn api_failure(error: &ApiError, space_id: Option<String>) -> VoiceError {
    match error {
        ApiError::Server { code, message, .. } => VoiceError { code: code.clone(), message: message.clone(), space_id },
        ApiError::Network(_) => VoiceError { code: "voice-offline".into(), message: String::new(), space_id },
        other => VoiceError { code: String::new(), message: other.to_string(), space_id },
    }
}

/// La voz de este core.
pub struct Voice {
    pub(crate) core: Core,
}

impl Core {
    pub fn voice(&self) -> Voice {
        Voice { core: self.clone() }
    }

    /// Pone el motor de voz de la plataforma. Sin él, entrar dice que esta
    /// app todavía no tiene llamadas.
    pub fn set_voice_engine(&self, engine: Arc<dyn VoiceEngine>) {
        self.inner.voice.lock().expect("voice lock").engine = Some(engine);
    }

    /// Si un aviso puede sonar ahora: encendido en esta máquina y fuera de
    /// una llamada. En una llamada un tono de aviso se mete en lo que oyes
    /// (`app/src/lib/notify-sound.ts`).
    pub fn notice_sound_allowed(&self) -> bool {
        self.notify_sound() && self.voice().state().phase == VoicePhase::Out
    }
}

/// Lo que hay detrás de la voz.
#[derive(Default)]
pub(crate) struct VoiceData {
    pub(crate) state: VoiceState,
    pub(crate) engine: Option<Arc<dyn VoiceEngine>>,
    /// De qué organización es la ocupación que se tiene.
    pub(crate) occupancy_org: Option<String>,
    /// Cuántos miran la ocupación ahora.
    pub(crate) watchers: usize,
    /// Si se ha leído la preferencia de ruido.
    pub(crate) restored: bool,
}

/// Mientras vive, se pregunta quién anda por los canales. Al soltarlo deja de
/// preguntarse, si nadie más mira.
pub struct OccupancyWatch {
    voice: Voice,
}

impl Drop for OccupancyWatch {
    fn drop(&mut self) {
        self.voice.stop_watching();
    }
}

impl Voice {
    pub(crate) fn data(&self) -> std::sync::MutexGuard<'_, VoiceData> {
        self.core.inner.voice.lock().expect("voice lock")
    }

    pub fn state(&self) -> VoiceState {
        let current = self.core.orgs().state().current_org_id;
        let mut data = self.data();
        if !data.restored {
            data.restored = true;
            data.state.noise_suppression = self.core.inner.storage.get(NOISE_KEY).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(true);
        }
        let mut state = data.state.clone();
        // La ocupación de otra organización no se pinta.
        if data.occupancy_org != current {
            state.occupancy = HashMap::new();
        }
        state
    }

    pub(crate) fn change(&self, edit: impl FnOnce(&mut VoiceState)) {
        let _ = self.state();
        edit(&mut self.data().state);
        self.core.inner.listener.voice_changed(self.state());
    }

    fn engine(&self) -> Option<Arc<dyn VoiceEngine>> {
        self.data().engine.clone()
    }

    /// Un turno nuevo: la entrada que estuviera en camino deja de ser la
    /// vigente.
    fn next_turn(&self) -> u64 {
        self.core.inner.voice_turn.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_turn(&self, turn: u64) -> bool {
        self.core.inner.voice_turn.load(Ordering::SeqCst) == turn
    }

    /// Entra a la voz de un canal. `org_id` y `space_name` hacen falta cuando
    /// el canal no es de la organización que se mira (aceptar un timbre de
    /// otra); sin ellos se toman la de ahora y el nombre de su canal.
    pub async fn join_channel(&self, space_id: &str, org_id: Option<String>, space_name: Option<String>) {
        let state = self.state();
        // Ya dentro: volver a entrar cortaría la conversación para dejarla
        // igual.
        if state.in_channel(space_id) {
            return;
        }
        if state.phase != VoicePhase::Out {
            self.leave().await;
        }
        let turn = self.next_turn();
        let org_id = org_id.or_else(|| self.core.orgs().state().current_org_id);
        let space_name = space_name.or_else(|| self.core.comms().state().channels.into_iter().find(|channel| channel.space_id == space_id).map(|channel| channel.name));
        // La sala se abre ya, mientras conecta: entrar lleva un segundo largo,
        // y sin nada que mirar parece que el botón no hizo nada.
        self.change(|state| {
            *state = VoiceState { space_id: Some(space_id.into()), org_id, space_name, phase: VoicePhase::Joining, stage_open: true, ..state.outside() };
        });
        let entry = self.core.call::<VoiceToken>("POST", &format!("/api/v1/task-spaces/{space_id}/voice/token"), Some(serde_json::json!({}))).await;
        if !self.is_turn(turn) {
            return;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                self.change(|state| *state = VoiceState { error: Some(api_failure(&error, Some(space_id.into()))), ..state.outside() });
                return;
            }
        };
        // Llegas tarde: la llamada se acaba de mudar a una reunión. Entrar
        // aquí sería quedarte solo en el canal.
        if let Some(moved) = entry.moved_to {
            let invite_id = moved.invite_id.clone();
            self.change(|state| state.moved = Some(moved));
            Box::pin(self.join_meeting(&invite_id)).await;
            return;
        }
        self.change(|state| {
            state.session_id = Some(entry.session_id.clone()).filter(|id| !id.is_empty());
            if state.space_name.is_none() && !entry.space_name.is_empty() {
                state.space_name = Some(entry.space_name.clone());
            }
            if state.org_id.is_none() && !entry.org_id.is_empty() {
                state.org_id = Some(entry.org_id.clone());
            }
        });
        self.connect(entry.url, entry.token, turn, Some(space_id.into())).await;
    }

    /// Entra a una reunión con invitados, como miembro.
    pub async fn join_meeting(&self, invite_id: &str) {
        let state = self.state();
        if state.phase != VoicePhase::Out && state.meet_id.as_deref() == Some(invite_id) {
            return;
        }
        if state.phase != VoicePhase::Out {
            self.leave().await;
        }
        let turn = self.next_turn();
        self.change(|state| *state = VoiceState { meet_id: Some(invite_id.into()), phase: VoicePhase::Joining, stage_open: true, ..state.outside() });
        let entry = self.core.call::<MeetToken>("POST", &format!("/api/v1/call-invites/{invite_id}/voice/token"), Some(serde_json::json!({}))).await;
        if !self.is_turn(turn) {
            return;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                self.change(|state| *state = VoiceState { error: Some(api_failure(&error, None)), ..state.outside() });
                return;
            }
        };
        self.change(|state| {
            state.org_id = Some(entry.org_id.clone()).filter(|id| !id.is_empty());
            state.title = Some(entry.title.clone());
            state.meet_space_id = entry.space_id.clone();
            state.session_id = Some(entry.session_id.clone()).filter(|id| !id.is_empty());
        });
        self.connect(entry.url, entry.token, turn, None).await;
    }

    /// Conecta con un pase ya pedido. Común a canal y reunión.
    async fn connect(&self, url: String, token: String, turn: u64, space_id: Option<String>) {
        let Some(engine) = self.engine() else {
            self.change(|state| {
                *state = VoiceState { error: Some(VoiceError { code: "voice-not-in-this-app".into(), message: String::new(), space_id }), ..state.outside() }
            });
            return;
        };
        let noise = self.state().noise_suppression;
        let joined = engine.join(url, token, noise).await;
        if !self.is_turn(turn) {
            engine.leave().await;
            return;
        }
        match joined {
            Ok(identity) => {
                self.change(|state| {
                    state.phase = VoicePhase::In;
                    state.me = Some(identity);
                    state.mic = true;
                });
                self.occupancy_soon();
            }
            Err(code) => {
                self.change(|state| *state = VoiceState { error: Some(VoiceError { code: code.clone(), message: code, space_id }), ..state.outside() });
                engine.leave().await;
            }
        }
    }

    /// Cuelga. Si estabas llamando a alguien, primero le calla el teléfono:
    /// si no, le seguiría sonando una invitación a una sala vacía.
    pub async fn leave(&self) {
        if self.state().outgoing.is_some() {
            self.cancel_ring().await;
        }
        self.next_turn();
        let was_in = self.state().phase != VoicePhase::Out;
        self.change(|state| *state = state.outside());
        if let Some(engine) = self.engine() {
            engine.leave().await;
        }
        if was_in {
            self.occupancy_soon();
        }
    }

    /// Vuelve a la sala sin reconectar. Sin sala no hay nada que enseñar.
    pub fn open_stage(&self) {
        if self.state().phase == VoicePhase::Out {
            return;
        }
        self.change(|state| state.stage_open = true);
    }

    /// Minimiza. No cuelga: el audio sigue.
    pub fn close_stage(&self) {
        self.change(|state| state.stage_open = false);
    }

    /// Abre o cierra el micrófono. Optimista: silenciarse tiene que sentirse
    /// instantáneo, y el motor no puede fallar en apagar algo que ya tiene
    /// abierto. Tu recuadro sale del mismo mapa que el de los demás.
    pub fn toggle_mic(&self) {
        let next = !self.state().mic;
        self.change(|state| {
            state.mic = next;
            if let Some(me) = state.me.clone() {
                state.muted.insert(me, !next);
            }
            state.error = None;
        });
        if let Some(engine) = self.engine() {
            engine.set_mic(next);
        }
    }

    /// Ensordece, o lo quita. Ensordecer silencia también el micrófono, y
    /// quitarlo **no** lo devuelve: quien se pone sordo casi siempre se está
    /// apartando, y volver hablando sin querer es el accidente que este botón
    /// existe para evitar. Reabrir el micrófono es un gesto aparte.
    pub fn toggle_deaf(&self) {
        let next = !self.state().deaf;
        self.change(|state| {
            state.deaf = next;
            if next {
                state.mic = false;
                if let Some(me) = state.me.clone() {
                    state.muted.insert(me, true);
                }
            }
        });
        if let Some(engine) = self.engine() {
            engine.set_deaf(next);
            if next {
                engine.set_mic(false);
            }
        }
    }

    /// La supresión de ruido: se recuerda, y se aplica ya si hay llamada.
    pub fn set_noise_suppression(&self, on: bool) {
        if let Ok(raw) = serde_json::to_string(&on) {
            let _ = self.core.inner.storage.set(NOISE_KEY, &raw);
        }
        self.change(|state| state.noise_suppression = on);
        if self.state().phase != VoicePhase::Out {
            if let Some(engine) = self.engine() {
                engine.set_noise_suppression(on);
            }
        }
    }

    /// Lo elegido en esta máquina, para dárselo al motor al arrancar: quien
    /// corrigió el micrófono una vez no tiene que hacerlo cada día.
    pub fn device_choices(&self) -> DeviceChoices {
        self.core.inner.storage.get(DEVICES_KEY).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
    }

    /// Apunta un micrófono (`output: false`) o un altavoz elegido a mano.
    pub fn choose_device(&self, output: bool, id: Option<String>) {
        let mut choices = self.device_choices();
        if output {
            choices.output = id;
        } else {
            choices.input = id;
        }
        if let Ok(raw) = serde_json::to_string(&choices) {
            let _ = self.core.inner.storage.set(DEVICES_KEY, &raw);
        }
    }

    pub fn clear_error(&self) {
        self.change(|state| state.error = None);
    }

    pub fn clear_moved(&self) {
        self.change(|state| state.moved = None);
    }

    /// Lo que cuenta el motor. Fuera de una sala no se escucha: lo que llegue
    /// es de una que ya se dejó.
    pub fn engine_event(&self, event: VoiceEvent) {
        if self.state().phase == VoicePhase::Out {
            return;
        }
        match event {
            VoiceEvent::Connected { identity } => self.change(|state| {
                state.me = Some(identity);
                state.phase = VoicePhase::In;
            }),
            VoiceEvent::Joined { identity, name } => self.change(|state| {
                if !state.people.iter().any(|peer| peer.identity == identity) {
                    state.people.push(VoicePeer { identity, name });
                }
            }),
            VoiceEvent::Left { identity } => self.change(|state| {
                state.people.retain(|peer| peer.identity != identity);
                // Fuera de los que hablan: quien se va hablando dejaría su
                // punto encendido para siempre. Y fuera de los silenciados:
                // quien se fue mudo y vuelve abierto saldría silenciado.
                state.speaking.retain(|id| *id != identity);
                state.muted.remove(&identity);
            }),
            VoiceEvent::Speaking { identities } => self.change(|state| state.speaking = identities),
            VoiceEvent::SelfSpeaking { speaking } => self.change(|state| state.self_speaking = speaking),
            VoiceEvent::Muted { identity, muted } => self.change(|state| {
                state.muted.insert(identity, muted);
            }),
            VoiceEvent::Latency { ms } => self.change(|state| state.latency_ms = Some(ms)),
            VoiceEvent::Recording { active, id, by, since } => self.change(|state| {
                state.recording = active.then_some(VoiceRecording { id, by, since });
            }),
            VoiceEvent::Disconnected { reason } => {
                self.next_turn();
                self.change(|state| {
                    let space_id = state.space_id.clone();
                    let error = disconnect_reason(&reason).map(|code| VoiceError { message: code.clone(), code, space_id });
                    *state = VoiceState { error, ..state.outside() };
                });
                // La sala soltó, pero el micrófono del motor sigue abierto
                // hasta que se le diga: fuera de una sala no se publica nada.
                if let (Some(engine), Ok(runtime)) = (self.engine(), tokio::runtime::Handle::try_current()) {
                    runtime.spawn(async move { engine.leave().await });
                }
                self.occupancy_soon();
            }
        }
    }

    /// Empieza a mirar quién anda por los canales. Lo que devuelve deja de
    /// hacerlo al soltarse.
    pub fn watch_occupancy(&self) -> OccupancyWatch {
        self.start_watching();
        OccupancyWatch { voice: self.core.voice() }
    }

    /// Lo mismo, contado a mano, para quien no puede soltar un guardián (el
    /// móvil).
    pub fn start_watching(&self) {
        let first = {
            let mut data = self.data();
            data.watchers += 1;
            data.watchers == 1
        };
        if first {
            let turn = self.core.inner.occupancy_turn.fetch_add(1, Ordering::SeqCst) + 1;
            let voice = self.core.voice();
            let every = self.core.inner.config.voice_presence_every;
            tokio::spawn(async move {
                while voice.core.inner.occupancy_turn.load(Ordering::SeqCst) == turn {
                    voice.refresh_occupancy().await;
                    tokio::time::sleep(every).await;
                }
            });
        }
    }

    pub fn stop_watching(&self) {
        let last = {
            let mut data = self.data();
            data.watchers = data.watchers.saturating_sub(1);
            data.watchers == 0
        };
        if last {
            self.core.inner.occupancy_turn.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Pregunta quién anda por los canales de la organización que se mira.
    /// Si falla, se queda lo que había: es informativo y se reintenta solo.
    pub async fn refresh_occupancy(&self) {
        let Some(org_id) = self.core.orgs().state().current_org_id.filter(|id| !id.is_empty()) else { return };
        let path = format!("/api/v1/chat/voice-presence?orgId={org_id}");
        let Ok(found) = self.core.get::<HashMap<String, Vec<VoicePeer>>>(&path).await else { return };
        if self.core.orgs().state().current_org_id.as_deref() != Some(org_id.as_str()) {
            return;
        }
        self.data().occupancy_org = Some(org_id);
        self.change(|state| state.occupancy = found);
    }

    /// Una pregunta suelta, si alguien mira: al entrar o salir uno mismo, o
    /// al cambiar de organización.
    pub(crate) fn occupancy_soon(&self) {
        if self.data().watchers == 0 {
            return;
        }
        let voice = self.core.voice();
        tokio::spawn(async move { voice.refresh_occupancy().await });
    }

    /// Al salir de la sesión: se cuelga, y lo de una persona no es de la
    /// siguiente.
    pub(crate) fn clear(&self) {
        self.next_turn();
        self.core.inner.occupancy_turn.fetch_add(1, Ordering::SeqCst);
        let engine = {
            let mut data = self.data();
            let engine = data.engine.take();
            *data = VoiceData { engine: engine.clone(), ..VoiceData::default() };
            engine
        };
        self.core.inner.listener.voice_changed(self.state());
        if let Some(engine) = engine {
            if tokio::runtime::Handle::try_current().is_ok() {
                tokio::spawn(async move { engine.leave().await });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Qué cortes se le dicen a alguien. Roturas que atrapa: decir «saliste»
    /// cuando colgaste tú; callar un motivo nuevo que no se conoce; enseñar
    /// el motivo crudo de LiveKit en vez de su etiqueta.
    #[test]
    fn which_disconnections_are_told() {
        assert_eq!(disconnect_reason("ClientInitiated"), None);
        assert_eq!(disconnect_reason("UnknownReason"), None);
        assert_eq!(disconnect_reason("DuplicateIdentity").as_deref(), Some("voice-joined-elsewhere"));
        assert_eq!(disconnect_reason("ParticipantRemoved").as_deref(), Some("voice-removed"));
        assert_eq!(disconnect_reason("SignalClose").as_deref(), Some("SignalClose"));
    }

    /// Las iniciales, como Tauri. Roturas que atrapa: tomar las dos primeras
    /// letras de un nombre con apellido; romperse con un nombre vacío.
    #[test]
    fn initials_like_the_other_app() {
        assert_eq!(person_initials("Luis Mora"), "LM");
        assert_eq!(person_initials("ana"), "AN");
        assert_eq!(person_initials("@jose.guzman"), "JG");
        assert_eq!(person_initials("  "), "?");
    }

    /// Los nombres en una frase. Rotura que atrapa: «Ana, Luis, Eva» sin la
    /// conjunción, o la coma de serie en castellano.
    #[test]
    fn names_read_as_a_sentence() {
        let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        assert_eq!(join_names(&names(&["Ana"]), Locale::Es), "Ana");
        assert_eq!(join_names(&names(&["Ana", "Luis"]), Locale::Es), "Ana y Luis");
        assert_eq!(join_names(&names(&["Ana", "Luis", "Eva"]), Locale::Es), "Ana, Luis y Eva");
        assert_eq!(join_names(&names(&["Ana", "Luis", "Eva"]), Locale::En), "Ana, Luis, and Eva");
        assert_eq!(join_names(&names(&["Ana", "Luis"]), Locale::En), "Ana and Luis");
    }

    /// Los recuadros y la línea de la cabecera. Roturas que atrapa: no
    /// contarte a ti (la sala diría «nadie» con alguien dentro); tu mudo
    /// sacado del mapa del servidor y no de tu botón; tu «hablando» sacado
    /// de la lista del servidor; la latencia mientras conecta; repetirte si
    /// el motor también te cuenta.
    #[test]
    fn the_room_counts_you_first() {
        let mut state = VoiceState { phase: VoicePhase::In, me: Some("u1".into()), mic: false, self_speaking: true, latency_ms: Some(40), ..VoiceState::default() };
        state.people = vec![VoicePeer { identity: "u1".into(), name: "Ana".into() }, VoicePeer { identity: "u2".into(), name: "Luis Mora".into() }];
        state.speaking = vec!["u1".into()];
        state.muted.insert("u1".into(), false);
        state.muted.insert("u2".into(), true);
        let tiles = voice_tiles(&state, Locale::Es);
        assert_eq!(tiles.len(), 2);
        assert_eq!((tiles[0].me, tiles[0].name.as_str(), tiles[0].muted, tiles[0].speaking), (true, "Tú", true, true));
        assert_eq!((tiles[1].initials.as_str(), tiles[1].muted, tiles[1].speaking), ("LM", true, false));
        assert_eq!(voice_status(&state, Locale::Es), "2 en la llamada · 40 ms");

        state.people.truncate(1);
        assert_eq!(voice_status(&state, Locale::Es), "nadie más todavía · 40 ms");
        state.phase = VoicePhase::Joining;
        assert_eq!(voice_status(&state, Locale::Es), "conectando…");
    }

    /// El nombre de la sala. Rotura que atrapa: llamar a una reunión por el
    /// canal del que cuelga.
    #[test]
    fn a_meeting_is_called_by_its_title() {
        let channel = VoiceState { space_name: Some("producto".into()), ..VoiceState::default() };
        assert_eq!(voice_room_name(&channel, Locale::Es), "producto");
        let meeting = VoiceState { meet_id: Some("m1".into()), space_name: Some("producto".into()), title: Some("Con el cliente".into()), ..VoiceState::default() };
        assert_eq!(voice_room_name(&meeting, Locale::Es), "Con el cliente");
    }

    /// La frase de un fallo. Roturas que atrapa: enseñar la etiqueta cruda;
    /// perder lo que llegó cuando la etiqueta no está en el catálogo.
    #[test]
    fn a_failure_reads_in_your_language() {
        let known = VoiceError { code: "voice-no-mic".into(), message: "voice-no-mic".into(), space_id: None };
        assert_eq!(voice_error_phrase(&known, Locale::Es), "No se encontró ningún micrófono.");
        let unknown = VoiceError { code: "voice-something-new".into(), message: "algo nuevo".into(), space_id: None };
        assert_eq!(voice_error_phrase(&unknown, Locale::Es), "algo nuevo");
    }
}
