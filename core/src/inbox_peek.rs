//! Ver qué dice un mensaje de la campana sin salir de la página.
//!
//! Doce filas de «Diego te escribió» no dicen nada, y abrir cada conversación
//! obliga a dejar lo que se hacía. Dos cosas lo arreglan:
//!
//! - **La vista rápida**: pulsar un aviso de una conversación enseña sus
//!   últimos mensajes encima de la página, con una caja para contestar.
//! - **El inicio del mensaje en la fila**: con la campana abierta, cada fila
//!   de un directo que se ve (suelta, o dentro de un grupo desplegado) lleva
//!   las primeras palabras de su mensaje.
//!
//! Lo que **no** cambia: el aviso de un directo no guarda el texto
//! (`backend/internal/core/service/dm.go`), para que la bandeja no destape
//! una conversación privada a quien mira una pantalla compartida. Aquí el
//! texto se le pide a la conversación, que ya es de quien mira, sólo con la
//! campana abierta y nunca se guarda en el aviso. Y para compartir pantalla
//! hay un ajuste de esta máquina que lo apaga (`hide_direct_previews`); la
//! vista rápida sólo se abre con un clic.
//!
//! Los canales no necesitan nada de esto: su aviso ya trae el texto, porque
//! en un canal lo lee toda la organización.

use std::collections::HashMap;

use chrono::{DateTime, TimeZone};

use crate::chat::{read_path, thread_lines, ThreadMessage, ThreadState};
use crate::comms::Conversation;
use crate::comms_text::{instant, quote_text};
use crate::comms_wire::{ChatMessage, DmMessage};
use crate::error::ApiError;
use crate::inbox::{link_target, Inbox, InboxPanel, InboxTarget};
use crate::texts::Locale;
use crate::Core;

/// Cuántos mensajes trae la vista rápida: los últimos, lo que cabe en un
/// vistazo. Para más está la conversación.
pub const PEEK_LIMIT: usize = 12;

/// La clave bajo la que se guarda el ajuste de ocultar el inicio de los
/// directos.
const HIDE_PREVIEWS_KEY: &str = "inbox-hide-direct-previews";

/// Cuánto puede ir un aviso por detrás de su mensaje. El servidor guarda el
/// mensaje y luego avisa, así que el aviso nunca va por delante; dos minutos
/// es de sobra y no alcanza a un mensaje de otra tanda.
const MATCH_WINDOW_MS: i64 = 120_000;

/// La ruta de los últimos mensajes de esa conversación.
pub fn peek_path(conversation: &Conversation) -> String {
    match conversation {
        Conversation::Channel { space_id } => format!("/api/v1/task-spaces/{space_id}/chat?limit={PEEK_LIMIT}"),
        Conversation::Direct { conversation_id } => format!("/api/v1/dm/{conversation_id}/messages?limit={PEEK_LIMIT}"),
    }
}

/// Un directo de la campana cuyo inicio hay que pedir: la conversación y el
/// aviso más nuevo suyo a la vista. Si llega uno más nuevo, cambia, y se
/// vuelve a pedir: lo pedido antes no lo tiene.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreviewWanted {
    pub conversation: Conversation,
    pub newest: String,
}

/// La conversación de un directo a la que lleva ese enlace.
fn direct_of(link: &str) -> Option<Conversation> {
    match link_target(link) {
        InboxTarget::Conversation { conversation: conversation @ Conversation::Direct { .. } } => Some(conversation),
        _ => None,
    }
}

/// Qué directos hay que leer para el panel tal como se pinta: los de las
/// filas que se ven (un aviso suelto, o los de un grupo desplegado). Un grupo
/// plegado no enseña filas, y no pide nada. Con el ajuste de ocultar, nada.
pub fn previews_wanted(panel: &InboxPanel, hide: bool) -> Vec<PreviewWanted> {
    if hide {
        return Vec::new();
    }
    let mut wanted: Vec<PreviewWanted> = Vec::new();
    for row in panel.unread.iter().chain(&panel.read).flat_map(|group| &group.rows) {
        let Some(conversation) = direct_of(&row.link) else { continue };
        match wanted.iter_mut().find(|wanted| wanted.conversation == conversation) {
            Some(wanted) if row.created_at > wanted.newest => wanted.newest = row.created_at.clone(),
            Some(_) => {}
            None => wanted.push(PreviewWanted { conversation, newest: row.created_at.clone() }),
        }
    }
    wanted
}

/// El mensaje del que avisa un aviso de directo, por la hora: el aviso no
/// dice qué mensaje es. Es el último de la otra persona que no es posterior
/// al aviso y le va a menos de dos minutos por detrás. Devuelve su inicio en
/// una línea, o nada si no se encuentra (ya no está entre los últimos, o se
/// retiró).
pub fn row_preview(notice_at: &str, messages: &[ThreadMessage], me: Option<&str>) -> Option<String> {
    let at = instant(notice_at)?;
    messages
        .iter()
        .filter(|message| me != Some(message.author_user_id.as_str()))
        .filter_map(|message| instant(&message.created_at).map(|written| (written, message)))
        .filter(|(written, _)| {
            let behind = (at - *written).num_milliseconds();
            (0..=MATCH_WINDOW_MS).contains(&behind)
        })
        .max_by_key(|(written, _)| *written)
        .map(|(_, message)| quote_text(&message.body))
        .filter(|text| !text.is_empty())
}

/// El panel con el inicio de cada directo que se ve, sacado de lo que se
/// leyó de su conversación. Con el ajuste de ocultar, las filas de un directo
/// se quedan sin texto, como las manda el servidor.
pub fn with_previews(mut panel: InboxPanel, read: &HashMap<Conversation, Vec<ThreadMessage>>, me: Option<&str>, hide: bool) -> InboxPanel {
    for row in panel.unread.iter_mut().chain(panel.read.iter_mut()).flat_map(|group| group.rows.iter_mut()) {
        let Some(conversation) = direct_of(&row.link) else { continue };
        row.body = if hide { String::new() } else { read.get(&conversation).and_then(|messages| row_preview(&row.created_at, messages, me)).unwrap_or_default() };
    }
    panel
}

/// Una línea de la vista rápida.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct QuickLine {
    pub id: String,
    /// «Tú», el nombre, o «cac».
    pub author: String,
    pub mine: bool,
    pub time: String,
    /// Lo que dice, sin markdown y en una línea: es un vistazo.
    pub text: String,
}

/// Las líneas de la vista rápida, de la más vieja a la más nueva. Quién firma
/// y la hora salen de lo mismo que la conversación (`thread_lines`).
pub fn quick_lines<Viewer: TimeZone>(conversation: &Conversation, messages: &[ThreadMessage], me: Option<&str>, locale: Locale, now: &DateTime<Viewer>) -> Vec<QuickLine> {
    let state = ThreadState { conversation: Some(conversation.clone()), messages: messages.to_vec(), ..ThreadState::default() };
    thread_lines(&state, me, locale, now)
        .into_iter()
        .zip(messages)
        .map(|(line, message)| QuickLine { id: line.id, author: line.author, mine: line.mine, time: line.time, text: quote_text(&message.body) })
        .collect()
}

impl Inbox {
    /// Los últimos mensajes de esa conversación, del más viejo al más nuevo.
    /// No la marca leída: es lo que pide el inicio de las filas, y eso no es
    /// haberla leído.
    pub async fn peek(&self, conversation: &Conversation) -> Result<Vec<ThreadMessage>, ApiError> {
        let path = peek_path(conversation);
        let mut messages: Vec<ThreadMessage> = match conversation {
            Conversation::Channel { .. } => self.core.get_list::<ChatMessage>(&path).await?.into_iter().map(ThreadMessage::from).collect(),
            Conversation::Direct { .. } => self.core.get_list::<DmMessage>(&path).await?.into_iter().map(ThreadMessage::from).collect(),
        };
        messages.sort_by(|left, right| match (instant(&left.created_at), instant(&right.created_at)) {
            (Some(left), Some(right)) => left.cmp(&right),
            _ => left.created_at.cmp(&right.created_at),
        });
        Ok(messages)
    }

    /// La vista rápida: los últimos mensajes, y la conversación leída, que
    /// es lo que son una vez a la vista. Si no se pudo leer, no se marca.
    pub async fn quick_view(&self, conversation: &Conversation) -> Result<Vec<ThreadMessage>, ApiError> {
        let messages = self.peek(conversation).await?;
        if self.core.call_ok("POST", &read_path(conversation), Some(serde_json::json!({}))).await.is_ok() {
            self.core.counters().conversation_read(conversation);
        }
        Ok(messages)
    }
}

impl Core {
    /// Si las filas de un directo se quedan sin su inicio en esta máquina:
    /// para compartir pantalla. Por omisión, se enseña.
    pub fn hide_direct_previews(&self) -> bool {
        self.inner.storage.get(HIDE_PREVIEWS_KEY).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(false)
    }

    pub fn set_hide_direct_previews(&self, hide: bool) -> Result<(), ApiError> {
        let raw = serde_json::to_string(&hide).map_err(|error| ApiError::Storage(error.to_string()))?;
        self.inner.storage.set(HIDE_PREVIEWS_KEY, &raw).map_err(ApiError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::Utc;

    use super::*;
    use crate::inbox::{inbox_panel, InboxFeed, InboxItem, InboxState, InboxTab};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-26T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn message(id: &str, author: &str, at: &str, body: &str) -> ThreadMessage {
        ThreadMessage { id: id.into(), author_user_id: author.into(), author_name: format!("{author} nombre"), body: body.into(), created_at: at.into(), ..Default::default() }
    }

    fn notice(id: &str, kind: &str, link: &str, group: &str, at: &str) -> InboxItem {
        InboxItem {
            id: id.into(),
            org_id: "o1".into(),
            kind: kind.into(),
            title: format!("t-{id}"),
            body: if kind.starts_with("dm:") { String::new() } else { format!("b-{id}") },
            link: link.into(),
            created_at: at.into(),
            group_key: Some(group.into()),
            group_label: Some(group.into()),
            ..Default::default()
        }
    }

    fn direct(id: &str) -> Conversation {
        Conversation::Direct { conversation_id: id.into() }
    }

    /// Dos de Diego (c1), uno suelto de Bea (c2), dos de un canal y uno de
    /// una tarea.
    fn state() -> InboxState {
        let items = vec![
            notice("d1", "dm:message", "/dm?c=c1", "dm:c1", "2026-11-26T11:00:05Z"),
            notice("d2", "dm:message", "/dm?c=c1", "dm:c1", "2026-11-26T11:30:02Z"),
            notice("b1", "dm:message", "/dm?c=c2", "dm:c2", "2026-11-26T10:00:01Z"),
            notice("k1", "chat:message", "/chat?space=s1", "chat:s1", "2026-11-26T09:00:00Z"),
            notice("k2", "chat:message", "/chat?space=s1", "chat:s1", "2026-11-26T09:10:00Z"),
            notice("t1", "task:comment", "/tasks?task=t1", "item:t1", "2026-11-26T08:00:00Z"),
        ];
        InboxState { org_id: Some("o1".into()), loaded: true, failed: false, feed: InboxFeed { items, unread: 6, groups: Vec::new() } }
    }

    fn panel(expanded: &[&str]) -> InboxPanel {
        let expanded: HashSet<String> = expanded.iter().map(|key| key.to_string()).collect();
        inbox_panel(&state(), InboxTab::All, &expanded, Locale::Es, &now())
    }

    /// Qué se pide con la campana abierta. Roturas que atrapa: pedir el
    /// directo de un grupo plegado (su texto no se ve, y no hay que traerlo);
    /// pedir también los canales (su aviso ya trae el texto); quedarse con
    /// el aviso más viejo (un mensaje nuevo no volvería a pedir); pedir algo
    /// con el ajuste de ocultar.
    #[test]
    fn only_the_directs_on_screen_are_read() {
        assert_eq!(previews_wanted(&panel(&[]), false), [PreviewWanted { conversation: direct("c2"), newest: "2026-11-26T10:00:01Z".into() }], "plegado, Diego no se pide; Bea va suelta y sí");
        let open = previews_wanted(&panel(&["dm:c1", "chat:s1"]), false);
        assert_eq!(
            open,
            [PreviewWanted { conversation: direct("c1"), newest: "2026-11-26T11:30:02Z".into() }, PreviewWanted { conversation: direct("c2"), newest: "2026-11-26T10:00:01Z".into() }],
            "desplegado, Diego con su aviso más nuevo; el canal no"
        );
        assert!(previews_wanted(&panel(&["dm:c1"]), true).is_empty(), "con el ajuste, nada");
    }

    /// Qué mensaje es el de cada aviso. Roturas que atrapa: tomar el
    /// propio; tomar uno posterior al aviso (el de la siguiente tanda); tomar
    /// el más viejo de la ventana en vez del más cercano; tomar uno de hace
    /// una hora (el suyo ya no está entre los últimos); enseñar el markdown.
    #[test]
    fn each_notice_finds_its_message_by_the_time() {
        let messages = vec![
            message("m0", "diego", "2026-11-26T09:58:00Z", "de hace rato"),
            message("m1", "diego", "2026-11-26T11:00:03Z", "¿Has visto **el** deploy?"),
            message("m2", "diego", "2026-11-26T11:00:04Z", "el de `api`"),
            message("me", "u1", "2026-11-26T11:00:05Z", "mío"),
            message("m3", "diego", "2026-11-26T11:30:01Z", "otra cosa"),
        ];
        let me = Some("u1");
        assert_eq!(row_preview("2026-11-26T11:00:05Z", &messages, me).as_deref(), Some("el de api"), "el más cercano, no el propio ni uno posterior");
        assert_eq!(row_preview("2026-11-26T11:00:03Z", &messages, me).as_deref(), Some("¿Has visto el deploy?"));
        assert_eq!(row_preview("2026-11-26T11:30:02Z", &messages, me).as_deref(), Some("otra cosa"));
        assert_eq!(row_preview("2026-11-26T11:05:00Z", &messages, me), None, "a cinco minutos ya no es suyo");
        assert_eq!(row_preview("2026-11-26T10:00:00Z", &messages, None).as_deref(), Some("de hace rato"), "dos minutos por detrás todavía sí");
        assert_eq!(row_preview("no es una fecha", &messages, me), None);
    }

    /// Las filas de un directo enseñan su inicio, salvo con el ajuste; las de
    /// un canal siguen con el texto de su aviso. Roturas que atrapa: un
    /// ajuste que no apaga nada; pisar el texto de un canal; enseñar el
    /// mensaje de otra conversación.
    #[test]
    fn the_setting_turns_the_previews_off() {
        let mut read = HashMap::new();
        read.insert(direct("c1"), vec![message("m1", "diego", "2026-11-26T11:00:04Z", "hola"), message("m2", "diego", "2026-11-26T11:30:00Z", "¿vienes?")]);
        read.insert(direct("c2"), vec![message("x", "bea", "2026-11-26T10:00:00Z", "de Bea")]);
        let bodies = |panel: &InboxPanel| -> Vec<(String, String)> {
            panel.unread.iter().flat_map(|group| &group.rows).map(|row| (row.id.clone(), row.body.clone())).collect()
        };
        let shown = with_previews(panel(&["dm:c1", "chat:s1"]), &read, Some("u1"), false);
        let expected: Vec<(String, String)> =
            [("d2", "¿vienes?"), ("d1", "hola"), ("b1", "de Bea"), ("k2", "b-k2"), ("k1", "b-k1"), ("t1", "b-t1")].iter().map(|(id, body)| (id.to_string(), body.to_string())).collect();
        let mut got = bodies(&shown);
        got.sort();
        let mut want = expected;
        want.sort();
        assert_eq!(got, want);
        let hidden = with_previews(panel(&["dm:c1", "chat:s1"]), &read, Some("u1"), true);
        for (id, body) in bodies(&hidden) {
            if id.starts_with('d') || id.starts_with('b') {
                assert_eq!(body, "", "{id}: con el ajuste, sin texto");
            } else {
                assert_eq!(body, format!("b-{id}"), "{id}: el canal no cambia");
            }
        }
    }

    /// La ruta de la vista rápida. Rotura que atrapa: confundir un canal con
    /// un directo, o pedir la página entera de la conversación.
    #[test]
    fn the_quick_view_asks_for_the_last_messages() {
        assert_eq!(peek_path(&direct("c1")), "/api/v1/dm/c1/messages?limit=12");
        assert_eq!(peek_path(&Conversation::Channel { space_id: "s1".into() }), "/api/v1/task-spaces/s1/chat?limit=12");
    }

    /// Cada línea lleva su firma y su texto. Rotura que atrapa: líneas
    /// desemparejadas de sus mensajes, o lo propio sin «Tú».
    #[test]
    fn quick_lines_say_who_and_what() {
        let messages = vec![message("m1", "diego", "2026-11-26T11:00:00Z", "**hola**"), message("m2", "u1", "2026-11-26T11:01:00Z", "dime")];
        let lines = quick_lines(&direct("c1"), &messages, Some("u1"), Locale::Es, &now());
        let said: Vec<(&str, &str, bool)> = lines.iter().map(|line| (line.author.as_str(), line.text.as_str(), line.mine)).collect();
        assert_eq!(said, [("diego nombre", "hola", false), ("Tú", "dime", true)]);
    }
}
