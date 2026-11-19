//! Qué mensaje se avisa: el aviso del sistema de la app (y su sonido).
//!
//! Vino de `app/src/hooks/use-report-events.ts` (las ramas de `chat:message`
//! y `dm:message`), con tres diferencias que decidió jose el 18-nov-2026:
//!
//! - **Un canal silenciado calla de verdad.** En Tauri el aviso del escritorio
//!   salía igual; aquí sólo lo atraviesan una mención o una respuesta.
//! - **Corregir o retirar no se avisa.** El servidor publica una corrección y
//!   un retiro como un `chat:message` cualquiera, sin `edited`
//!   (`ChatService.Edit` y `Withdraw` en `backend/internal/core/service/chat.go`;
//!   lo mismo `dm:message` en `dm.go`), y Tauri los volvía a anunciar.
//! - **La ventana con el foco no es motivo para callar**: la puerta que había
//!   está quitada a propósito (`CLAUDE.md`, «Un aviso del sistema no le
//!   pregunta al foco»). Lo único que calla un mensaje normal es tener **esa**
//!   conversación delante, y ni eso calla una mención o una respuesta.
//!
//! ## Nuevo o corregido, sin que el servidor lo diga
//!
//! Dos cosas, porque cada una sola deja pasar algo:
//!
//! 1. **Los ids ya vistos.** Un mensaje que llegó en esta sesión y luego se
//!    corrige o se retira trae el mismo `messageId`: el segundo evento se
//!    descarta sin más.
//! 2. **La hora del último mensaje, según el servidor.** Corregir uno de antes
//!    de abrir la app trae un id nunca visto; lo que lo delata es que la hora
//!    del último mensaje de ese canal (`/chat/latest`) o de esa conversación
//!    (`lastMessageAt` de `/dm/`) **no avanza**: corregir no la toca y retirar
//!    la hace retroceder (los retirados no cuentan). Así que un mensaje apunta
//!    la hora que había al llegar y espera a la recarga que ya se hace tras
//!    cada evento: se avisa si la hora que trae la supera. Si no la supera,
//!    sólo cuenta como «no era nuevo» una carga pedida **después** de llegar
//!    el mensaje: una que salió antes puede traer la lista de antes de que se
//!    guardara.
//!
//! La hora se compara como fecha, no como texto: Go escribe un número de
//! decimales distinto según el instante. Y es la del servidor, no la que
//! `comms` le pone a la fila al llegar el evento para pintarla en el acto.
//!
//! Lo que llega junto a una misma conversación sale en **un** aviso, el que
//! más pesa (mención, respuesta, lo demás): en Linux GPUI no sustituye un
//! aviso por otro de la misma etiqueta, y diez mensajes seguidos serían diez
//! globos. Si la recarga falla, lo que esperaba caduca sin avisar: la campana
//! lo cuenta igual.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::comms::Conversation;
use crate::comms_wire::{ChatLatest, ChatMessageEvent, DmMessageEvent, DmSummary};
use crate::stream::ServerEvent;
use crate::texts::{text, text_with, Locale};
use crate::Core;

/// Cuántos ids de mensaje se recuerdan. Es una sesión de chat entera: una
/// corrección de algo más viejo que esto la caza la hora.
const SEEN: usize = 512;

/// Cuánto espera un mensaje a la recarga que dice si era nuevo. La recarga
/// sale a los 400 ms; esto es para cuando falla.
const WAIT: Duration = Duration::from_secs(30);

/// Por qué se avisa. El orden es el peso: en una tanda gana el primero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum NoticeKind {
    /// Te nombraron.
    Mention,
    /// Te contestaron con cita.
    Reply,
    /// Te escribieron en privado.
    Direct,
    /// Se dijo algo en un canal que sigues.
    ChannelMessage,
}

/// A dónde lleva pulsar el aviso.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct NoticeTarget {
    pub conversation: Conversation,
    /// La organización de esa conversación, si se sabe: el aviso puede ser de
    /// otra que la que se mira.
    pub org_id: Option<String>,
}

/// Un aviso del sistema, listo para enseñar.
///
/// Los textos se escriben al enseñarlo, en el idioma de quien lo lee
/// (`title` y `body`), y no al decidirlo: el core no sabe en qué idioma
/// pinta cada plataforma.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    pub kind: NoticeKind,
    /// Uno por conversación: `chat:<espacio>` o `dm:<conversación>`.
    pub tag: String,
    pub target: NoticeTarget,
    /// El nombre del canal, sin la almohadilla. Vacío en un directo, o si no
    /// se sabe.
    pub place: String,
    /// Quien escribió. Vacío en una línea del sistema.
    pub author: String,
    /// El adelanto del texto. Siempre vacío en un directo: lo privado no
    /// viaja en un aviso.
    pub preview: String,
}

impl Announcement {
    /// Dónde: `#Producto`, o «Canal» si no se sabe cuál.
    fn place_label(&self, locale: Locale) -> String {
        if self.place.is_empty() {
            text(locale, "common:misc.channel")
        } else {
            format!("#{}", self.place)
        }
    }

    /// La primera línea del aviso.
    pub fn title(&self, locale: Locale) -> String {
        let place = self.place_label(locale);
        match self.kind {
            NoticeKind::Mention => text_with(locale, "common:last.mentionedIn", &[("where", &place)]),
            NoticeKind::Reply => text_with(locale, "common:last.repliedIn", &[("where", &place)]),
            NoticeKind::ChannelMessage => place,
            NoticeKind::Direct if self.author.is_empty() => text(locale, "notifications:newDirect"),
            NoticeKind::Direct => text_with(locale, "notifications:wroteToYou", &[("who", &self.author)]),
        }
    }

    /// Lo que se dijo: «Luis Mora: ¿sale hoy?». En un directo, nada.
    pub fn body(&self, locale: Locale) -> String {
        if self.kind == NoticeKind::Direct {
            return String::new();
        }
        match (self.author.is_empty(), self.preview.is_empty()) {
            (false, false) => format!("{}: {}", self.author, self.preview),
            (true, false) => self.preview.clone(),
            (false, true) => text_with(locale, "notifications:wrote", &[("who", &self.author)]),
            (true, true) => String::new(),
        }
    }
}

/// Lo que hace falta saber de quien mira para decidir.
pub struct Viewer<'a> {
    /// Quién está dentro.
    pub me: &'a str,
    /// La conversación que tiene delante, si está en comunicación.
    pub open: Option<&'a Conversation>,
    /// Los canales que sigue: los de sus organizaciones menos los que
    /// silenció. `None` mientras no se sabe.
    pub following: Option<&'a HashSet<String>>,
}

/// Si un mensaje de canal se avisa, y por qué. La regla, sin estado.
///
/// - Lo corregido (`edited`) no; lo propio tampoco, ni una línea del sistema
///   que provocó uno mismo (grabar no es enterarse de que se grabó).
/// - Una mención o una respuesta, siempre: con la conversación delante y con
///   el canal silenciado.
/// - Lo demás, sólo en un canal que se sigue y que no se tiene delante.
///   Seguir sale de `/chat/following`, que son los canales de las
///   organizaciones de uno: un superadmin, que oye las de todas, no se entera
///   así de las ajenas. Mientras no se sabe qué se sigue, no se avisa.
pub fn channel_notice(event: &ChatMessageEvent, viewer: &Viewer) -> Option<NoticeKind> {
    if event.edited || event.space_id.is_empty() || event.actor_id == viewer.me {
        return None;
    }
    if event.mentions.iter().flatten().any(|id| id == viewer.me) {
        return Some(NoticeKind::Mention);
    }
    if event.replied_to == viewer.me {
        return Some(NoticeKind::Reply);
    }
    let here = Conversation::Channel { space_id: event.space_id.clone() };
    let followed = viewer.following.is_some_and(|following| following.contains(&event.space_id));
    (followed && viewer.open != Some(&here)).then_some(NoticeKind::ChannelMessage)
}

/// Si un mensaje privado se avisa: si no es una corrección, no es de uno
/// mismo y no se tiene esa conversación delante. (La marca de corrección la
/// manda el servidor desde el 19-nov-2026; antes la delataba sólo la hora.)
pub fn direct_notice(event: &DmMessageEvent, viewer: &Viewer) -> Option<NoticeKind> {
    if event.edited || event.conversation_id.is_empty() || event.actor_id == viewer.me {
        return None;
    }
    let here = Conversation::Direct { conversation_id: event.conversation_id.clone() };
    (viewer.open != Some(&here)).then_some(NoticeKind::Direct)
}

/// Una hora del servidor, como fecha. `None` si no se entiende.
fn instant(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw).ok().map(|at| at.with_timezone(&Utc))
}

/// Un mensaje que espera a saber si era nuevo.
struct Waiting {
    announcement: Announcement,
    /// Quien lo escribió: en un directo, la última línea tiene que ser suya.
    actor_id: String,
    /// La hora del último mensaje de su conversación cuando llegó: lo nuevo
    /// la supera. `None` si no tenía ninguno.
    before: Option<DateTime<Utc>>,
    queued_at: Instant,
}

/// Lo que hay detrás de los avisos.
#[derive(Default)]
pub(crate) struct NoticeData {
    /// Los ids de mensaje que ya llegaron, el más viejo delante.
    seen: VecDeque<String>,
    /// La hora del último mensaje de cada canal, según el servidor. `None`
    /// hasta la primera carga: sin ella no se sabe qué es avanzar.
    channel_marks: Option<HashMap<String, DateTime<Utc>>>,
    /// Lo mismo para cada conversación privada.
    direct_marks: Option<HashMap<String, DateTime<Utc>>>,
    waiting: Vec<Waiting>,
}

impl NoticeData {
    /// Si ese id ya pasó; si no, lo apunta.
    fn already_seen(&mut self, id: &str) -> bool {
        if id.is_empty() {
            return false;
        }
        if self.seen.iter().any(|seen| seen == id) {
            return true;
        }
        if self.seen.len() == SEEN {
            self.seen.pop_front();
        }
        self.seen.push_back(id.to_string());
        false
    }
}

/// Si la hora de una carga es posterior a la que había: un mensaje nuevo.
/// Sin hora antes, cualquiera lo es; sin hora ahora (se retiró todo), no.
fn advanced(before: Option<DateTime<Utc>>, now: Option<DateTime<Utc>>) -> bool {
    match (before, now) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(before), Some(now)) => now > before,
    }
}

/// Los avisos de este core.
pub struct Notices {
    core: Core,
}

impl Core {
    pub fn notices(&self) -> Notices {
        Notices { core: self.clone() }
    }
}

impl Notices {
    fn data(&self) -> std::sync::MutexGuard<'_, NoticeData> {
        self.core.inner.notices.lock().expect("notices lock")
    }

    /// Un evento del servidor: un mensaje que puede avisarse se pone a
    /// esperar la recarga que dirá si era nuevo.
    pub(crate) fn server_event(&self, event: &ServerEvent) {
        let Some(me) = self.core.session().map(|session| session.id) else { return };
        let comms = self.core.comms();
        let state = comms.state();
        let open = state.selected.as_ref().filter(|_| state.on_screen);
        let following = comms.following();
        let viewer = Viewer { me: &me, open, following: following.as_ref() };
        let (announcement, actor_id, message_id) = match event.event.as_str() {
            "chat:message" => {
                let Ok(said) = serde_json::from_str::<ChatMessageEvent>(&event.data) else { return };
                // Lo corregido no pasa ni por los vistos: su id es el de uno
                // que ya estaba.
                if said.edited {
                    return;
                }
                let Some(kind) = channel_notice(&said, &viewer) else {
                    self.data().already_seen(&said.message_id);
                    return;
                };
                let (org_id, name) = comms.channel_place(&said.space_id);
                let place = if said.space_name.is_empty() { name.unwrap_or_default() } else { said.space_name.clone() };
                let announcement = Announcement {
                    kind,
                    tag: format!("chat:{}", said.space_id),
                    target: NoticeTarget { conversation: Conversation::Channel { space_id: said.space_id.clone() }, org_id },
                    place,
                    author: said.author_name.clone(),
                    preview: said.preview.clone(),
                };
                (announcement, said.actor_id, said.message_id)
            }
            "dm:message" => {
                let Ok(said) = serde_json::from_str::<DmMessageEvent>(&event.data) else { return };
                let Some(kind) = direct_notice(&said, &viewer) else {
                    self.data().already_seen(&said.message_id);
                    return;
                };
                let announcement = Announcement {
                    kind,
                    tag: format!("dm:{}", said.conversation_id),
                    // La organización se sabe al llegar la lista: es suya.
                    target: NoticeTarget { conversation: Conversation::Direct { conversation_id: said.conversation_id.clone() }, org_id: None },
                    place: String::new(),
                    author: said.author_name.clone(),
                    preview: String::new(),
                };
                (announcement, said.actor_id, said.message_id)
            }
            _ => return,
        };
        let mut data = self.data();
        if data.already_seen(&message_id) {
            return;
        }
        let id = announcement.target.conversation.id().to_string();
        let marks = match announcement.target.conversation {
            Conversation::Channel { .. } => &data.channel_marks,
            Conversation::Direct { .. } => &data.direct_marks,
        };
        // Sin la primera carga no se sabe qué es avanzar: no se avisa.
        let Some(marks) = marks else { return };
        let before = marks.get(&id).copied();
        data.waiting.push(Waiting { announcement, actor_id, before, queued_at: Instant::now() });
    }

    /// Llegaron las últimas líneas de los canales, pedidas en `asked_at`.
    pub(crate) fn channels_loaded(&self, latest: &[ChatLatest], asked_at: Instant) {
        let now: HashMap<String, DateTime<Utc>> = latest.iter().filter_map(|line| Some((line.space_id.clone(), instant(&line.created_at)?))).collect();
        self.data().channel_marks = Some(now.clone());
        self.settle(asked_at, |announcement, _, before| match &announcement.target.conversation {
            Conversation::Channel { space_id } => Some(advanced(before, now.get(space_id).copied())),
            Conversation::Direct { .. } => None,
        });
    }

    /// Llegaron las conversaciones privadas, pedidas en `asked_at`. Además de
    /// avanzar, la última línea tiene que ser de quien avisó: si después
    /// contestó uno mismo, ya lo leyó.
    pub(crate) fn directs_loaded(&self, directs: &[DmSummary], asked_at: Instant) {
        let now: HashMap<String, DateTime<Utc>> = directs
            .iter()
            .filter_map(|direct| Some((direct.conversation_id.clone(), instant(direct.last_message_at.as_deref()?)?)))
            .collect();
        self.data().direct_marks = Some(now.clone());
        self.settle(asked_at, |announcement, actor_id, before| match &announcement.target.conversation {
            Conversation::Direct { conversation_id } => {
                let direct = directs.iter().find(|direct| &direct.conversation_id == conversation_id);
                let theirs = direct.is_some_and(|direct| direct.last_author_id.as_deref() == Some(actor_id));
                if let Some(direct) = direct {
                    announcement.target.org_id = Some(direct.org_id.clone());
                }
                Some(advanced(before, now.get(conversation_id).copied()) && theirs)
            }
            Conversation::Channel { .. } => None,
        });
    }

    /// Decide lo que esperaba con lo que trae una carga pedida en
    /// `asked_at`. `verdict` dice, de cada uno, si su conversación avanzó
    /// (`None`: esta carga no es de su clase). Avanzar lo da por nuevo con
    /// cualquier carga; no avanzar lo da por corregido sólo con una pedida
    /// después de que llegara. Lo que caducó se tira; lo nuevo sale en un
    /// aviso por conversación, el que más pesa y, a igual peso, el último.
    fn settle(&self, asked_at: Instant, mut verdict: impl FnMut(&mut Announcement, &str, Option<DateTime<Utc>>) -> Option<bool>) {
        let fresh = {
            let mut data = self.data();
            let waiting = std::mem::take(&mut data.waiting);
            let mut fresh: Vec<Announcement> = Vec::new();
            for mut one in waiting {
                if one.queued_at.elapsed() > WAIT {
                    continue;
                }
                match verdict(&mut one.announcement, &one.actor_id, one.before) {
                    None => data.waiting.push(one),
                    // Una carga que salió antes que el mensaje puede no
                    // tenerlo todavía: espera a la siguiente.
                    Some(false) if one.queued_at > asked_at => data.waiting.push(one),
                    Some(false) => {}
                    Some(true) => match fresh.iter_mut().find(|other| other.tag == one.announcement.tag) {
                        Some(other) if one.announcement.kind <= other.kind => *other = one.announcement,
                        Some(_) => {}
                        None => fresh.push(one.announcement),
                    },
                }
            }
            fresh
        };
        for announcement in fresh {
            self.core.inner.listener.announce(announcement);
        }
    }

    /// Lleva a la conversación de un aviso que se pulsó: a su organización
    /// si es otra (como `goInOrg` en `app/src/lib/ir-en-org.ts`), a
    /// comunicación, y a ella. Devuelve si llegó.
    ///
    /// Cambiar de organización puede no hacerse —documentación a medio
    /// escribir, que pregunta antes (`doc_edit`)—, y entonces no se mueve
    /// nada. Una conversación que ya no está (se borró el canal) deja
    /// comunicación abierta con lo que hubiera.
    pub async fn open(&self, target: NoticeTarget) -> bool {
        let org_id = match (&target.org_id, &target.conversation) {
            (Some(org_id), _) => Some(org_id.clone()),
            (None, Conversation::Channel { space_id }) => self.core.comms().channel_org(space_id).await,
            // Un directo sólo se avisa si está en la lista, que trae su
            // organización (`directs_loaded`): sin ella no hay a dónde ir.
            (None, Conversation::Direct { .. }) => None,
        };
        let Some(org_id) = org_id else { return false };
        if self.core.orgs().state().current_org_id.as_deref() != Some(org_id.as_str()) && !self.core.switch_org(&org_id).await {
            return false;
        }
        let comms = self.core.comms();
        let chosen = comms.select(target.conversation);
        comms.enter();
        chosen
    }

    /// Al salir: lo visto y lo que esperaba es de esa persona.
    pub(crate) fn clear(&self) {
        *self.data() = NoticeData::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(space: &str, actor: &str) -> ChatMessageEvent {
        ChatMessageEvent { space_id: space.into(), message_id: "m1".into(), actor_id: actor.into(), kind: "user".into(), ..Default::default() }
    }

    fn following(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    /// La regla de un mensaje de canal.
    ///
    /// Roturas que atrapa: avisar de lo propio o de una línea del sistema que
    /// uno provocó; avisar de una corrección; callar una mención o una
    /// respuesta por tener el canal delante o silenciado; avisar de lo normal
    /// con el canal delante, en uno silenciado o sin saber qué se sigue;
    /// preguntarle al foco (no hay con qué).
    #[test]
    fn what_a_channel_message_announces() {
        let follows = following(&["a", "b"]);
        let open_a = Conversation::Channel { space_id: "a".into() };
        let looking_at_a = Viewer { me: "u1", open: Some(&open_a), following: Some(&follows) };
        let elsewhere = Viewer { me: "u1", open: None, following: Some(&follows) };

        assert_eq!(channel_notice(&said("b", "u2"), &looking_at_a), Some(NoticeKind::ChannelMessage), "otro canal, sí");
        assert_eq!(channel_notice(&said("a", "u2"), &looking_at_a), None, "el que se tiene delante, no");
        assert_eq!(channel_notice(&said("a", "u2"), &elsewhere), Some(NoticeKind::ChannelMessage), "fuera de comunicación, sí");
        assert_eq!(channel_notice(&said("z", "u2"), &elsewhere), None, "uno que no se sigue, no");
        assert_eq!(channel_notice(&said("a", "u2"), &Viewer { me: "u1", open: None, following: None }), None, "sin saber qué se sigue, no");
        assert_eq!(channel_notice(&said("b", "u1"), &elsewhere), None, "lo propio, no");
        let mine_system = ChatMessageEvent { kind: "system".into(), ..said("b", "u1") };
        assert_eq!(channel_notice(&mine_system, &elsewhere), None, "la línea del sistema que provoqué yo, no");
        let theirs_system = ChatMessageEvent { kind: "system".into(), author_name: String::new(), ..said("b", "u2") };
        assert_eq!(channel_notice(&theirs_system, &elsewhere), Some(NoticeKind::ChannelMessage), "la que provocó otro, sí");
        let edited = ChatMessageEvent { edited: true, ..said("b", "u2") };
        assert_eq!(channel_notice(&edited, &elsewhere), None, "corregido, no");

        let mention = ChatMessageEvent { mentions: Some(vec!["u9".into(), "u1".into()]), ..said("a", "u2") };
        assert_eq!(channel_notice(&mention, &looking_at_a), Some(NoticeKind::Mention), "una mención, con el canal delante");
        let muted_mention = ChatMessageEvent { mentions: Some(vec!["u1".into()]), ..said("z", "u2") };
        assert_eq!(channel_notice(&muted_mention, &elsewhere), Some(NoticeKind::Mention), "y en uno silenciado");
        let reply = ChatMessageEvent { replied_to: "u1".into(), ..said("a", "u2") };
        assert_eq!(channel_notice(&reply, &looking_at_a), Some(NoticeKind::Reply));
        let others = ChatMessageEvent { mentions: Some(vec!["u9".into()]), replied_to: "u9".into(), ..said("z", "u2") };
        assert_eq!(channel_notice(&others, &elsewhere), None, "mencionar o responder a otro no es a mí");
    }

    /// La regla de un directo. Roturas que atrapa: avisar de lo propio, de
    /// una corrección, o de la conversación que se tiene delante; callar uno
    /// por tener delante otra.
    #[test]
    fn what_a_direct_announces() {
        let event = DmMessageEvent { conversation_id: "c1".into(), message_id: "d1".into(), actor_id: "u2".into(), author_name: "Bea".into(), edited: false };
        let c1 = Conversation::Direct { conversation_id: "c1".into() };
        let c2 = Conversation::Direct { conversation_id: "c2".into() };
        assert_eq!(direct_notice(&event, &Viewer { me: "u1", open: None, following: None }), Some(NoticeKind::Direct));
        assert_eq!(direct_notice(&event, &Viewer { me: "u1", open: Some(&c2), following: None }), Some(NoticeKind::Direct));
        assert_eq!(direct_notice(&event, &Viewer { me: "u1", open: Some(&c1), following: None }), None);
        assert_eq!(direct_notice(&event, &Viewer { me: "u2", open: None, following: None }), None);
        let edited = DmMessageEvent { edited: true, ..event };
        assert_eq!(direct_notice(&edited, &Viewer { me: "u1", open: None, following: None }), None, "una corrección no");
    }

    fn announcement(kind: NoticeKind, place: &str, author: &str, preview: &str) -> Announcement {
        Announcement {
            kind,
            tag: "chat:a".into(),
            target: NoticeTarget { conversation: Conversation::Channel { space_id: "a".into() }, org_id: None },
            place: place.into(),
            author: author.into(),
            preview: preview.into(),
        }
    }

    /// Lo que dice cada aviso. Roturas que atrapa: un directo que enseña algo
    /// del texto; un canal sin su almohadilla; el título de una mención sin
    /// dónde; un cuerpo «: » sin autor; textos fuera del catálogo.
    #[test]
    fn what_each_notice_says() {
        let plain = announcement(NoticeKind::ChannelMessage, "Producto", "Luis Mora", "¿sale hoy?");
        assert_eq!((plain.title(Locale::Es), plain.body(Locale::Es)), ("#Producto".to_string(), "Luis Mora: ¿sale hoy?".to_string()));
        let mention = announcement(NoticeKind::Mention, "Producto", "Luis Mora", "@ana mira");
        assert_eq!(mention.title(Locale::Es), "Te mencionaron en #Producto");
        assert_eq!(announcement(NoticeKind::Reply, "Producto", "Luis", "sí").title(Locale::En), "Somebody replied to you in #Producto");
        assert_eq!(announcement(NoticeKind::ChannelMessage, "", "", "Grabación lista").title(Locale::Es), "Canal");
        assert_eq!(announcement(NoticeKind::ChannelMessage, "P", "", "Grabación lista").body(Locale::Es), "Grabación lista", "una línea del sistema no firma");
        assert_eq!(announcement(NoticeKind::ChannelMessage, "P", "Luis", "").body(Locale::Es), "Luis escribió");

        let direct = Announcement { kind: NoticeKind::Direct, place: String::new(), ..announcement(NoticeKind::Direct, "", "Bea Soto", "no debería verse") };
        assert_eq!((direct.title(Locale::Es), direct.body(Locale::Es)), ("Bea Soto te escribió".to_string(), String::new()));
        let nameless = Announcement { author: String::new(), ..direct };
        assert_eq!(nameless.title(Locale::Es), "Mensaje directo nuevo");
    }

    /// Avanzar es por fecha, no por texto. Roturas que atrapa: comparar las
    /// horas como texto, con lo que `…:00.5Z` queda por debajo de
    /// `…:00.25Z` y un mensaje nuevo pasa por corrección; dar por nuevo lo
    /// que no tiene hora (se retiró todo).
    #[test]
    fn hours_are_compared_as_dates() {
        let at = |raw: &str| instant(raw);
        assert!(advanced(at("2026-11-18T10:00:00.25Z"), at("2026-11-18T10:00:00.5Z")));
        assert!(!advanced(at("2026-11-18T10:00:00.5Z"), at("2026-11-18T10:00:00.5Z")), "la misma hora no es avanzar");
        assert!(advanced(at("2026-11-18T10:00:00.5Z"), at("2026-11-18T04:00:00.6-06:00")), "en otra zona, y algo después");
        assert!(advanced(None, at("2026-11-18T10:00:00Z")), "el primer mensaje de un canal");
        assert!(!advanced(at("2026-11-18T10:00:00Z"), None), "se retiró el único");
    }

    /// Los ids vistos. Rotura que atrapa: no olvidar nunca (la memoria crece
    /// sin fin), o olvidar el último.
    #[test]
    fn the_seen_ids_are_a_ring() {
        let mut data = NoticeData::default();
        for n in 0..SEEN {
            assert!(!data.already_seen(&n.to_string()));
        }
        assert!(data.already_seen("0"));
        assert!(!data.already_seen("nuevo"));
        assert!(!data.already_seen("0"), "el más viejo se olvidó al entrar uno más");
        assert!(data.already_seen("nuevo"));
        assert_eq!(data.seen.len(), SEEN);
        assert!(!data.already_seen(""), "un id vacío no cuenta como visto");
        assert!(!data.already_seen(""));
    }
}
