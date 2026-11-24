//! La conversación que se tiene delante: sus mensajes, de un canal o de un
//! directo, leídos y al día.
//!
//! Vino de `app/src/store/chat.store.ts` y `dm.store.ts` (abrir, refrescar
//! fundiendo, la página de antes, marcar leído), de `hooks/use-report-events.ts`
//! (qué evento refresca qué) y de `components/chat/ChannelView.tsx`,
//! `DMThread.tsx`, `lib/same-voice.ts`, `ReplyQuote.tsx` y `ReadTicks.tsx`
//! (cómo se lee cada línea). Allí eran dos stores gemelos; aquí es uno, porque
//! un canal y un directo sólo se distinguen en las rutas, en que en un directo
//! no escribe cac y en las palomitas.
//!
//! Las reglas, que son las que ya mordieron en la app de siempre:
//!
//! - **Lo que se pinta es de la conversación que la lista marca**
//!   ([`Chat::state`]): si se eligió otra y sus mensajes aún no llegaron, no
//!   se enseñan los de la anterior bajo su nombre. Y un directo de otra
//!   organización no se enseña nunca.
//! - **Una respuesta que llega tarde no cuenta**: cada apertura tiene su
//!   turno, y lo que vuelve con uno viejo se tira.
//! - **Refrescar funde, no reemplaza** ([`merged`]): las páginas viejas que
//!   alguien cargó subiendo se quedan, y sólo se quita lo que desapareció
//!   dentro de la ventana que se acaba de leer.
//! - **El cursor de la página anterior es el `createdAt` del mensaje más
//!   viejo, tal como llegó**: el servidor compara con `<` estricto sobre
//!   nanosegundos, y un instante reescrito repetiría o se saltaría mensajes.
//! - **Leído se marca cuando se ha visto**: tras una lectura que salió bien
//!   de una conversación que está en pantalla. Una carga que falla no borra
//!   el contador de algo que nadie llegó a ver.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

use crate::comms::Conversation;
use crate::comms_text::{instant, quote_text, was_read};
use crate::comms_wire::{ChatFile, ChatMessage, DmMessage, MessageQuote, ReactionSummary};
use crate::doc_links::{doc_link, DocLink};
use crate::error::ApiError;
use crate::stream::ServerEvent;
use crate::task::{attachment_path, image_type, MentionMark, MentionMenu, MentionPerson, Picture, UserSummary};
use crate::texts::{text, Locale};
use crate::Core;

/// Cuántos mensajes trae una página: la `PAGE` de los dos stores.
pub const THREAD_PAGE: usize = 50;

/// Un respiro antes de volver a leer: una ráfaga de mensajes son otros
/// tantos eventos, y la conversación a la que se llega es la misma.
const COALESCE: Duration = Duration::from_millis(400);

/// Cuántas páginas hacia atrás se piden, como mucho, para unir lo que se
/// tenía con lo que hay ahora tras un corte largo. Más allá se suelta lo
/// viejo: mejor volver a subir que leer una conversación con un agujero.
const GAP_PAGES: usize = 10;

/// Por dónde entra lo que escribe un agente con el token de una persona.
const VIA_AGENT: &str = "mcp";

/// Un mensaje de la conversación, sea de un canal o de un directo.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessage {
    pub id: String,
    /// Siempre una persona, también en una línea del sistema: quien la provocó.
    pub author_user_id: String,
    pub author_name: String,
    /// La puso cac, no una persona. En un directo, nunca.
    pub system: bool,
    pub via: Option<String>,
    pub body: String,
    /// Tal como llegó: es el cursor de la página anterior.
    pub created_at: String,
    pub updated_at: String,
    pub reactions: Vec<ReactionSummary>,
    pub reply_to: Option<MessageQuote>,
}

impl From<ChatMessage> for ThreadMessage {
    fn from(message: ChatMessage) -> Self {
        Self {
            system: message.is_system(),
            id: message.id,
            author_user_id: message.author_user_id,
            author_name: message.author_name,
            via: message.via,
            body: message.body,
            created_at: message.created_at,
            updated_at: message.updated_at,
            reactions: message.reactions,
            reply_to: message.reply_to,
        }
    }
}

impl From<DmMessage> for ThreadMessage {
    fn from(message: DmMessage) -> Self {
        Self {
            system: false,
            id: message.id,
            author_user_id: message.author_user_id,
            author_name: message.author_name,
            via: message.via,
            body: message.body,
            created_at: message.created_at,
            updated_at: message.updated_at,
            reactions: message.reactions,
            reply_to: message.reply_to,
        }
    }
}

/// La conversación que se tiene delante, tal como se pinta.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ThreadState {
    /// De qué conversación es todo lo de abajo. `None` si no hay ninguna.
    pub conversation: Option<Conversation>,
    /// Del más viejo al más nuevo: el orden en que se leen.
    pub messages: Vec<ThreadMessage>,
    /// Se está trayendo la primera página.
    pub loading: bool,
    /// Se está trayendo una página de antes.
    pub loading_older: bool,
    /// Queda historia por encima de lo cargado.
    pub has_more: bool,
    /// La conversación no se pudo leer: se dice, y no se marca leída.
    pub failed: bool,
    /// La página de antes no se pudo traer.
    pub older_failed: bool,
    /// Cuántas veces ha llegado algo nuevo por el final desde que se abrió.
    /// Quien pinta lo compara con el anterior para saber si avisar de
    /// «mensajes nuevos»; un mensaje retirado o editado no lo mueve.
    pub arrivals: u64,
    /// En un directo, hasta dónde leyó el otro.
    pub other_read_at: Option<String>,
    /// Quien mira puede escribir aquí. En un canal, no quien sólo mira la
    /// organización (`viewer`); en un directo, siempre.
    pub can_write: bool,
    /// Se puede nombrar a alguien con `@`: en un canal, que es donde el
    /// servidor avisa a quien se nombra. En un directo sólo hay dos.
    pub mentions_allowed: bool,
    /// Lo escrito en esta conversación está de camino al servidor.
    pub sending: bool,
}

/// La conversación abierta, por dentro.
struct Open {
    conversation: Conversation,
    /// Cuál de las aperturas es: la respuesta de otra no se pone.
    turn: u64,
    /// Se está en comunicación con ella delante. Fuera no se refresca ni se
    /// marca leída: nadie la está mirando.
    on_screen: bool,
    messages: Vec<ThreadMessage>,
    loading: bool,
    loading_older: bool,
    has_more: bool,
    failed: bool,
    older_failed: bool,
    arrivals: u64,
    /// La última lectura que se puso: una pedida antes y llegada después
    /// trae lo de antes, y no pisa.
    applied_read: u64,
    /// Hasta qué llegada se marcó leída (`arrivals` al pedirlo): si no es
    /// la última, hay algo sin marcar. Si su página llegó cuando no estaba
    /// a la vista no se marcó, y se marca al volver a ella.
    marked: Option<u64>,
    /// Quien pinta tiene a la vista el final de la conversación. Lo que
    /// llega mientras se lee más arriba no se ha visto: no se marca hasta
    /// bajar.
    at_tail: bool,
    /// Se pidió releer con la primera página todavía en camino: esa página
    /// pudo salir del servidor antes de lo que hizo pedirlo.
    reread: bool,
}

/// Lo que hay detrás de la conversación.
#[derive(Default)]
pub(crate) struct ChatData {
    open: Option<Open>,
    /// Las conversaciones con un mensaje de camino: una vez cada una.
    sending: Vec<Conversation>,
    /// Lo último que se avisó, para no avisar de lo mismo.
    published: ThreadState,
}

/// La conversación de este core.
pub struct Chat {
    core: Core,
}

impl Core {
    pub fn chat(&self) -> Chat {
        Chat { core: self.clone() }
    }
}

/// Compara dos instantes del servidor. Por lo que valen y no por cómo están
/// escritos: Go no escribe los ceros de la derecha, y «…:00Z» ordenado como
/// texto cae **después** de «…:00.5Z».
fn compare_at(left: &str, right: &str) -> std::cmp::Ordering {
    match (instant(left), instant(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

/// Funde lo que se tenía con la ventana que se acaba de leer (`refrescar` en
/// `chat.store.ts`). Devuelve la conversación y si llegó algo nuevo por el
/// final.
///
/// - Lo fresco pisa a lo que se tenía con su mismo id: así entra una
///   edición, una reacción o una cita que cambió.
/// - Lo que se tenía y es **anterior** a la ventana se queda: son páginas
///   viejas que alguien cargó subiendo, y la ventana no dice nada de ellas.
/// - Lo que se tenía, cae dentro de la ventana y ya no viene, se retiró.
/// - Una ventana vacía es una conversación vacía: la primera página son los
///   últimos mensajes, y si no hay ninguno no queda nada más atrás. (En la
///   app de siempre una ventana vacía no quitaba nada, y el único mensaje de
///   una conversación seguía pintado después de retirarlo.)
pub(crate) fn merged(held: &[ThreadMessage], fresh: Vec<ThreadMessage>) -> (Vec<ThreadMessage>, bool) {
    let Some(oldest_fresh) = fresh.iter().map(|message| message.created_at.as_str()).min_by(|left, right| compare_at(left, right)).map(str::to_string) else {
        return (Vec::new(), false);
    };
    let held_ids: HashSet<&str> = held.iter().map(|message| message.id.as_str()).collect();
    let newest_held = held.iter().map(|message| message.created_at.as_str()).max_by(|left, right| compare_at(left, right));
    // Nuevo es lo que no se tenía y va detrás de todo lo que se tenía. Un id
    // desconocido en mitad de lo cargado no es una llegada.
    let arrived = fresh
        .iter()
        .any(|message| !held_ids.contains(message.id.as_str()) && newest_held.is_none_or(|newest| compare_at(&message.created_at, newest).is_ge()));
    let fresh_ids: HashSet<&str> = fresh.iter().map(|message| message.id.as_str()).collect();
    let mut all: Vec<ThreadMessage> = held
        .iter()
        .filter(|message| compare_at(&message.created_at, &oldest_fresh).is_lt() && !fresh_ids.contains(message.id.as_str()))
        .cloned()
        .collect();
    drop(fresh_ids);
    all.extend(fresh);
    // Estable: dos mensajes del mismo instante se quedan como los mandó el
    // servidor.
    all.sort_by(|left, right| compare_at(&left.created_at, &right.created_at));
    (all, arrived)
}

/// Si entre lo que se tenía y la ventana recién leída puede faltar algo: la
/// ventana vino llena, es toda posterior a lo que se tenía y no comparte
/// ningún mensaje con ello. Pasa al volver de un corte largo en un canal
/// con mucho movimiento.
fn may_leave_a_gap(held: &[ThreadMessage], fresh: &[ThreadMessage]) -> bool {
    if held.is_empty() || fresh.len() < THREAD_PAGE {
        return false;
    }
    let held_ids: HashSet<&str> = held.iter().map(|message| message.id.as_str()).collect();
    !fresh.iter().any(|message| held_ids.contains(message.id.as_str()))
}

/// `encodeURIComponent`: todo lo que no es letra, cifra o `-_.~` va escapado.
/// El `+` de una zona horaria sin escapar llegaría como un espacio, el
/// servidor no entendería el cursor y contestaría con la primera página.
pub(crate) fn query_escaped(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// La ruta de una página de esa conversación. `before` es el `createdAt` del
/// mensaje más viejo que se tiene, **tal como llegó**.
pub(crate) fn page_path(conversation: &Conversation, before: Option<&str>) -> String {
    let mut path = match conversation {
        Conversation::Channel { space_id } => format!("/api/v1/task-spaces/{space_id}/chat?limit={THREAD_PAGE}"),
        Conversation::Direct { conversation_id } => format!("/api/v1/dm/{conversation_id}/messages?limit={THREAD_PAGE}"),
    };
    if let Some(before) = before {
        path.push_str("&before=");
        path.push_str(&query_escaped(before));
    }
    path
}

/// La ruta que marca leída esa conversación.
fn read_path(conversation: &Conversation) -> String {
    match conversation {
        Conversation::Channel { space_id } => format!("/api/v1/task-spaces/{space_id}/chat/read"),
        Conversation::Direct { conversation_id } => format!("/api/v1/dm/{conversation_id}/read"),
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Touched {
    space_id: String,
    conversation_id: String,
}

impl Chat {
    fn data(&self) -> std::sync::MutexGuard<'_, ChatData> {
        self.core.inner.chat.lock().expect("chat lock")
    }

    /// La conversación que se tiene delante **ahora**, acotada a lo que la
    /// lista de comunicación marca.
    ///
    /// Se arma cada vez contra la lista del momento, como hace ella con la
    /// organización: si lo abierto no es lo marcado —se acaba de elegir otra
    /// conversación, o de cambiar de organización— no se enseña nada suyo,
    /// sólo que la marcada se está trayendo.
    pub fn state(&self) -> ThreadState {
        let comms = self.core.comms().state();
        let Some(selected) = comms.selected.clone() else { return ThreadState::default() };
        let other_read_at = match &selected {
            Conversation::Direct { conversation_id } => {
                comms.directs.iter().find(|direct| &direct.conversation_id == conversation_id).and_then(|direct| direct.other_read_at.clone())
            }
            Conversation::Channel { .. } => None,
        };
        let can_write = self.can_write(&selected);
        let mentions_allowed = matches!(selected, Conversation::Channel { .. });
        let data = self.data();
        let sending = data.sending.contains(&selected);
        // Lo marcado es siempre de la organización que se mira (lo dice la
        // lista): con comparar la conversación, un directo de otra no pasa.
        match data.open.as_ref().filter(|open| open.conversation == selected) {
            Some(open) => ThreadState {
                conversation: Some(selected),
                messages: open.messages.clone(),
                loading: open.loading,
                loading_older: open.loading_older,
                has_more: open.has_more,
                failed: open.failed,
                older_failed: open.older_failed,
                arrivals: open.arrivals,
                other_read_at,
                can_write,
                mentions_allowed,
                sending,
            },
            None => ThreadState {
                conversation: Some(selected),
                loading: true,
                has_more: true,
                other_read_at,
                can_write,
                mentions_allowed,
                sending,
                ..ThreadState::default()
            },
        }
    }

    /// Si quien mira puede escribir en esa conversación.
    ///
    /// En un canal manda el papel en la organización, como en el árbol
    /// (`OrgRole.CanWrite`; el servidor contesta 403 `viewer-read-only`). En
    /// un directo el servidor no mira el papel: quien está en él, escribe.
    fn can_write(&self, conversation: &Conversation) -> bool {
        match conversation {
            Conversation::Direct { .. } => true,
            Conversation::Channel { .. } => {
                let role = self.core.orgs().state().current().map(|org| org.role.clone()).unwrap_or_default();
                let superadmin = self.core.session().is_some_and(|session| session.superadmin);
                crate::tree::can_edit_tree(&role, superadmin)
            }
        }
    }

    /// Vuelve a armar lo que se pinta y avisa si cambió. Lo llama lo de
    /// aquí y también la lista de comunicación, que es quien dice cuál está
    /// delante y hasta dónde leyó el otro.
    pub(crate) fn publish(&self) {
        let state = self.state();
        {
            let mut data = self.data();
            if data.published == state {
                return;
            }
            data.published = state.clone();
        }
        self.core.inner.listener.thread_changed(state);
    }

    /// Al salir: la conversación de una persona no es de la siguiente.
    pub(crate) fn clear(&self) {
        self.core.inner.chat_turn.fetch_add(1, Ordering::SeqCst);
        self.data().open = None;
        self.publish();
    }

    /// Si esa apertura sigue vigente **y a la vista**: se está en
    /// comunicación con ella marcada.
    fn in_front(&self, turn: u64) -> bool {
        let comms = self.core.comms().state();
        let data = self.data();
        data.open.as_ref().is_some_and(|open| {
            open.turn == turn && open.on_screen && comms.on_screen && comms.selected.as_ref() == Some(&open.conversation)
        })
    }

    /// Pone delante lo que la lista de comunicación marca.
    ///
    /// Lo llama quien pinta cada vez que la lista cambia; qué hacer lo
    /// decide esto. Una conversación nueva se abre (vacía lo que hubiera y
    /// trae su primera página); la misma, si se vuelve a ella, se relee
    /// fundiendo; y al salir de comunicación deja de estar a la vista, sin
    /// tirar lo cargado.
    pub async fn follow(&self) {
        let comms = self.core.comms().state();
        let target = comms.selected.clone().filter(|_| comms.on_screen);
        enum Step {
            Nothing,
            Hidden,
            Back(bool),
            Open(Conversation),
        }
        let step = {
            let mut data = self.data();
            match (target, data.open.as_mut()) {
                (None, Some(open)) if open.on_screen => {
                    open.on_screen = false;
                    Step::Hidden
                }
                (None, _) => Step::Nothing,
                (Some(conversation), Some(open)) if open.conversation == conversation => {
                    if open.on_screen {
                        Step::Nothing
                    } else {
                        open.on_screen = true;
                        // Si se fue a medio traer, la respuesta la pondrá al
                        // día; si no, se relee.
                        Step::Back(open.loading)
                    }
                }
                (Some(conversation), _) => Step::Open(conversation),
            }
        };
        match step {
            Step::Nothing | Step::Hidden | Step::Back(true) => {}
            Step::Back(false) => self.refresh().await,
            Step::Open(conversation) => {
                // La gente de la organización, para decir quién anda por
                // aquí en la cabecera de un canal: una vez por organización.
                if !self.core.comms().has_people() {
                    let comms = self.core.comms();
                    tokio::spawn(async move { comms.load_people().await });
                }
                self.open(conversation).await;
            }
        }
    }

    /// Abre una conversación: vacía lo que hubiera —la anterior no puede
    /// quedarse pintada bajo el nombre de ésta— y trae su primera página.
    async fn open(&self, conversation: Conversation) {
        let turn = self.core.inner.chat_turn.fetch_add(1, Ordering::SeqCst) + 1;
        self.data().open = Some(Open {
            conversation: conversation.clone(),
            turn,
            on_screen: true,
            messages: Vec::new(),
            loading: true,
            loading_older: false,
            has_more: true,
            failed: false,
            older_failed: false,
            arrivals: 0,
            applied_read: 0,
            marked: None,
            at_tail: true,
            reread: false,
        });
        self.publish();
        let page = self.page(&conversation, None).await;
        let (loaded, reread) = {
            let mut data = self.data();
            // Una respuesta que llega tras elegir otra conversación no se pone.
            let Some(open) = data.open.as_mut().filter(|open| open.turn == turn) else { return };
            open.loading = false;
            let reread = std::mem::take(&mut open.reread);
            match page {
                Ok(messages) => {
                    open.has_more = messages.len() >= THREAD_PAGE;
                    open.messages = messages;
                    (true, reread)
                }
                Err(_) => {
                    open.failed = true;
                    (false, false)
                }
            }
        };
        self.publish();
        // Abrir es leer; pero sólo si se llegó a ver algo.
        if loaded {
            self.mark_read(turn).await;
        }
        // Lo que llegó con la página en camino puede no venir en ella.
        if reread {
            Box::pin(self.refresh()).await;
        }
    }

    /// Si la apertura en curso ya sabe que, al acabar, tiene que releer: algo
    /// cambió con su primera página en camino. Sólo lo mira una prueba, que
    /// no tiene otra forma de saber que el aviso llegó antes que la página
    /// sin cronometrar la espera.
    pub fn reread_pending(&self) -> bool {
        self.data().open.as_ref().is_some_and(|open| open.loading && open.reread)
    }

    /// Vuelve a intentar una conversación que no se pudo leer.
    pub async fn retry(&self) {
        let target = self.data().open.as_ref().filter(|open| open.failed && !open.loading).map(|open| open.conversation.clone());
        if let Some(conversation) = target {
            self.open(conversation).await;
        }
    }

    /// Una página de la conversación, del más viejo al más nuevo.
    async fn page(&self, conversation: &Conversation, before: Option<&str>) -> Result<Vec<ThreadMessage>, ApiError> {
        let path = page_path(conversation, before);
        match conversation {
            Conversation::Channel { .. } => Ok(self.core.get_list::<ChatMessage>(&path).await?.into_iter().map(ThreadMessage::from).collect()),
            Conversation::Direct { .. } => Ok(self.core.get_list::<DmMessage>(&path).await?.into_iter().map(ThreadMessage::from).collect()),
        }
    }

    /// Vuelve a leer la conversación que está a la vista, **sin vaciarla**:
    /// funde su primera página con lo que hay. Si llegó algo nuevo —o si
    /// todavía no se había podido marcar—, se marca leída: se está mirando.
    ///
    /// Una que no se llegó a leer se vuelve a abrir; una que no está a la
    /// vista no se toca (se releerá al volver a ella).
    pub async fn refresh(&self) {
        let (conversation, turn, serial, failed) = {
            let mut data = self.data();
            let Some(open) = data.open.as_mut().filter(|open| open.on_screen) else { return };
            if open.loading {
                // La apertura relee al acabar.
                open.reread = true;
                return;
            }
            (open.conversation.clone(), open.turn, self.core.inner.chat_reads.fetch_add(1, Ordering::SeqCst) + 1, open.failed)
        };
        if failed {
            self.retry().await;
            return;
        }
        let Ok(mut fresh) = self.page(&conversation, None).await else { return };
        let held = match self.data().open.as_ref().filter(|open| open.turn == turn) {
            Some(open) => open.messages.clone(),
            None => return,
        };
        // Tras un corte largo la ventana puede no llegar hasta lo que se
        // tenía: se sigue hacia atrás hasta unirlas, para no dejar un agujero
        // en mitad de la conversación.
        let mut whole_history = false;
        let mut joined = !may_leave_a_gap(&held, &fresh);
        if !joined {
            let held_ids: HashSet<&str> = held.iter().map(|message| message.id.as_str()).collect();
            for _ in 0..GAP_PAGES {
                let cursor = fresh[0].created_at.clone();
                let Ok(older) = self.page(&conversation, Some(&cursor)).await else { return };
                let short = older.len() < THREAD_PAGE;
                let overlaps = older.iter().any(|message| held_ids.contains(message.id.as_str()));
                let known: HashSet<String> = fresh.iter().map(|message| message.id.clone()).collect();
                let mut window: Vec<ThreadMessage> = older.into_iter().filter(|message| !known.contains(&message.id)).collect();
                window.append(&mut fresh);
                fresh = window;
                if overlaps {
                    joined = true;
                    break;
                }
                if short {
                    // Se llegó al principio sin encontrar nada de lo que se
                    // tenía: lo leído es la conversación entera.
                    whole_history = true;
                    break;
                }
            }
        }
        let mark = {
            let mut data = self.data();
            let Some(open) = data.open.as_mut().filter(|open| open.turn == turn) else { return };
            // Una lectura pedida antes que otra que ya se puso trae lo de
            // antes: pondría de vuelta un mensaje retirado entre las dos.
            if serial < open.applied_read {
                return;
            }
            open.applied_read = serial;
            let arrived = if joined {
                let (messages, arrived) = merged(&open.messages, fresh);
                open.messages = messages;
                arrived
            } else {
                // No se pudieron unir: se suelta lo viejo antes que leer con
                // un agujero. Queda historia por encima salvo que se llegara
                // al principio.
                open.messages = fresh;
                open.has_more = !whole_history;
                true
            };
            if arrived {
                open.arrivals += 1;
            }
            open.at_tail && open.marked != Some(open.arrivals)
        };
        self.publish();
        if mark {
            self.mark_read(turn).await;
        }
    }

    /// Vuelve a leer tras un respiro, y sólo la última vez que se pidió
    /// (como `Counters::reload_soon`).
    fn refresh_soon(&self) {
        let turn = self.core.inner.chat_reloads.fetch_add(1, Ordering::SeqCst) + 1;
        let chat = self.core.chat();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            if chat.core.inner.chat_reloads.load(Ordering::SeqCst) != turn {
                return;
            }
            chat.refresh().await;
        });
    }

    /// Trae la página anterior a lo cargado y la pone delante.
    pub async fn load_older(&self) {
        let (conversation, turn, cursor) = {
            let mut data = self.data();
            let Some(open) = data.open.as_mut().filter(|open| open.has_more && !open.loading && !open.loading_older) else { return };
            // El más viejo en pantalla, tal como llegó: «lo de antes de esto».
            let Some(oldest) = open.messages.first() else { return };
            let cursor = (oldest.id.clone(), oldest.created_at.clone());
            open.loading_older = true;
            open.older_failed = false;
            (open.conversation.clone(), open.turn, cursor)
        };
        self.publish();
        let page = self.page(&conversation, Some(&cursor.1)).await;
        {
            let mut data = self.data();
            let Some(open) = data.open.as_mut().filter(|open| open.turn == turn) else { return };
            open.loading_older = false;
            // Mientras subía, un refresco que no pudo unir cambió la lista
            // por la de ahora: esta página era «lo de antes» de otra cosa.
            if open.messages.first().map(|message| &message.id) != Some(&cursor.0) {
                drop(data);
                self.publish();
                return;
            }
            match page {
                Ok(older) => {
                    open.has_more = older.len() >= THREAD_PAGE;
                    let held: HashSet<String> = open.messages.iter().map(|message| message.id.clone()).collect();
                    let mut all: Vec<ThreadMessage> = older.into_iter().filter(|message| !held.contains(&message.id)).collect();
                    all.append(&mut open.messages);
                    open.messages = all;
                }
                Err(_) => open.older_failed = true,
            }
        }
        self.publish();
    }

    /// Marca leída la conversación de esa apertura, si sigue a la vista.
    ///
    /// Como la app de siempre: al abrirla, y cuando llega algo mientras se
    /// tiene delante (`openPanel` y `onIncoming`). El contador de su fila se
    /// quita aquí en vez de volver a contar todos los canales por un cambio
    /// que ya se sabe, y luego se le pregunta al servidor, que es quien
    /// tiene la última palabra.
    async fn mark_read(&self, turn: u64) {
        if !self.in_front(turn) {
            return;
        }
        // Lo que se marca es lo que había al pedirlo: si llega algo con la
        // petición en camino, sigue sin marcar.
        let Some((conversation, arrivals)) = self.data().open.as_ref().filter(|open| open.turn == turn).map(|open| (open.conversation.clone(), open.arrivals)) else { return };
        if self.core.call_ok("POST", &read_path(&conversation), Some(serde_json::json!({}))).await.is_ok() {
            if let Some(open) = self.data().open.as_mut().filter(|open| open.turn == turn) {
                open.marked = Some(open.marked.map_or(arrivals, |marked| marked.max(arrivals)));
            }
            self.core.counters().conversation_read(&conversation);
        }
    }

    /// Quien pinta dice si tiene a la vista el final de la conversación.
    ///
    /// Lo que llega leyendo más arriba no se marca leído —no se ha visto—;
    /// al bajar hasta el final, sí.
    pub async fn at_tail(&self, at_tail: bool) {
        let turn = {
            let mut data = self.data();
            let Some(open) = data.open.as_mut() else { return };
            if open.at_tail == at_tail {
                return;
            }
            open.at_tail = at_tail;
            // Sin página no hay nada que marcar: lo hace la apertura.
            if !at_tail || open.loading || open.failed || open.marked == Some(open.arrivals) {
                return;
            }
            open.turn
        };
        self.mark_read(turn).await;
    }

    /// Un evento del servidor: si toca la conversación que está a la vista,
    /// se vuelve a leer.
    ///
    /// `chat:message` es un mensaje nuevo, uno editado **o uno retirado**
    /// (el servidor manda el mismo evento para los tres): por eso se relee
    /// en vez de añadir nada, y qué llegó de verdad lo dice [`merged`].
    /// Las reacciones tienen su evento. `dm:read` no trae mensajes: mueve
    /// las palomitas, que salen de la lista de directos, y es ella quien
    /// avisa.
    pub(crate) fn server_event(&self, event: &ServerEvent) {
        let touched = |data: &str| serde_json::from_str::<Touched>(data).unwrap_or_default();
        let conversation = match event.event.as_str() {
            "chat:message" | "chat:reaction" => Conversation::Channel { space_id: touched(&event.data).space_id },
            "dm:message" | "dm:reaction" => Conversation::Direct { conversation_id: touched(&event.data).conversation_id },
            _ => return,
        };
        // Si no está a la vista, `refresh` no hace nada: se releerá al volver.
        let open = self.data().open.as_ref().is_some_and(|open| open.conversation == conversation);
        if open {
            self.refresh_soon();
        }
    }

    // ── Escribir ─────────────────────────────────────────────────────────

    /// Manda un mensaje a esa conversación, como respuesta a `reply_to` si
    /// lo hay.
    ///
    /// Lo que no se puede mandar no se pide (en blanco: `empty-body`; más
    /// largo de lo que acepta el servidor: `too-long`), y un mensaje de
    /// camino en esa conversación no deja mandar otro (`already-sending`):
    /// el servidor no reconoce un envío repetido y lo publicaría dos veces.
    ///
    /// Al salir bien se relee la conversación, si es la que se tiene
    /// delante: lo que contesta el servidor no trae el nombre de quien
    /// escribe ni la cita, y en un directo a quien escribe no le llega
    /// evento de lo suyo. En un directo, además, se vuelve a pedir la lista,
    /// que es donde va su última línea. Si falla, no se toca nada: lo
    /// escrito sigue en la caja de quien llama.
    pub async fn send(&self, conversation: Conversation, body: String, reply_to: Option<String>) -> Result<(), ApiError> {
        let body = message_to_send(&body)?;
        if !self.can_write(&conversation) {
            return Err(local("viewer-read-only", "Viewers can look, but not write"));
        }
        {
            let mut data = self.data();
            if data.sending.contains(&conversation) {
                return Err(local("already-sending", "A message is already on its way here"));
            }
            data.sending.push(conversation.clone());
        }
        self.publish();
        let mut payload = serde_json::json!({ "body": body });
        if let Some(reply_to) = reply_to.filter(|id| !id.is_empty()) {
            payload["replyToId"] = serde_json::Value::String(reply_to);
        }
        let result = self.core.call_ok("POST", &send_path(&conversation), Some(payload)).await;
        self.data().sending.retain(|sending| sending != &conversation);
        self.publish();
        if result.is_ok() {
            if self.data().open.as_ref().is_some_and(|open| open.conversation == conversation) {
                self.refresh().await;
            }
            if matches!(conversation, Conversation::Direct { .. }) {
                self.core.counters().reload_directs().await;
            }
        }
        result
    }

    /// Sube un fichero a esa conversación. Lo que no cabe no se sube
    /// (`too-large`). Lo que contesta el servidor se cita en el mensaje
    /// (`file_token`): es lo único que une el fichero al mensaje.
    pub async fn attach(&self, conversation: Conversation, file_name: String, bytes: Vec<u8>) -> Result<ChatFile, ApiError> {
        crate::task::attachment_fits(bytes.len() as u64)?;
        if !self.can_write(&conversation) {
            return Err(local("viewer-read-only", "Viewers can look, but not write"));
        }
        let (content_type, body) = crate::task::multipart(&file_name, &bytes);
        self.core.upload(&upload_path(&conversation), &content_type, body).await
    }

    /// Corrige un mensaje propio. Lo que no se puede mandar no se pide, y si
    /// el texto es el mismo, tampoco: no hay nada que corregir. Al salir
    /// bien se relee la conversación.
    pub async fn edit(&self, conversation: Conversation, message_id: String, body: String) -> Result<(), ApiError> {
        let body = message_to_send(&body)?;
        let same = self.data().open.as_ref().filter(|open| open.conversation == conversation).and_then(|open| open.messages.iter().find(|message| message.id == message_id).map(|message| message.body == body));
        if same == Some(true) {
            return Ok(());
        }
        self.core.call_ok("PATCH", &message_path(&conversation, &message_id), Some(serde_json::json!({ "body": body }))).await?;
        self.reread(&conversation).await;
        Ok(())
    }

    /// Retira un mensaje propio: deja de salir para todos. Se pregunta antes
    /// (`withdraw_question`).
    pub async fn withdraw(&self, conversation: Conversation, message_id: String) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &message_path(&conversation, &message_id), None).await?;
        self.reread(&conversation).await;
        Ok(())
    }

    /// Pone (`on`) o quita la reacción de quien mira. Se ve al momento, con
    /// la cuenta hecha aquí (`apply_reaction`); si el servidor dice que no,
    /// vuelve a como estaba. Al salir bien se relee: la lista exacta de
    /// quiénes la trae el servidor.
    pub async fn react(&self, conversation: Conversation, message_id: String, emoji: String, on: bool) -> Result<(), ApiError> {
        if !crate::comms_wire::REACTION_EMOJIS.contains(&emoji.as_str()) {
            return Err(local("bad-emoji", "That reaction is not one of the set"));
        }
        let me = self.core.session().map(|session| session.name.clone().filter(|name| !name.trim().is_empty()).unwrap_or(session.username)).unwrap_or_default();
        let before = {
            let mut data = self.data();
            let message = data.open.as_mut().filter(|open| open.conversation == conversation).and_then(|open| open.messages.iter_mut().find(|message| message.id == message_id));
            message.map(|message| {
                let before = message.reactions.clone();
                message.reactions = apply_reaction(&before, &emoji, on, &me);
                before
            })
        };
        self.publish();
        let path = format!("{}/reactions", message_path(&conversation, &message_id));
        let result = self.core.call_ok("POST", &path, Some(serde_json::json!({ "emoji": emoji, "on": on }))).await;
        match &result {
            Ok(()) => self.reread(&conversation).await,
            Err(_) => {
                if let Some(before) = before {
                    let mut data = self.data();
                    if let Some(message) = data.open.as_mut().filter(|open| open.conversation == conversation).and_then(|open| open.messages.iter_mut().find(|message| message.id == message_id)) {
                        message.reactions = before;
                    }
                }
                self.publish();
            }
        }
        result
    }

    /// Lo que se pregunta antes de retirar un mensaje de esa conversación.
    pub fn withdraw_question(&self, conversation: &Conversation, locale: Locale) -> crate::task::Question {
        let body = match conversation {
            Conversation::Channel { .. } => "chat:withdrawBody",
            Conversation::Direct { .. } => "common:misc.withdrawDmBody",
        };
        crate::task::Question {
            title: text(locale, "chat:withdrawTitle"),
            body: text(locale, body),
            warning: None,
            confirm: text(locale, "chat:withdraw"),
            cancel: text(locale, "chat:cancel"),
        }
    }

    /// Relee esa conversación si es la que está delante.
    async fn reread(&self, conversation: &Conversation) {
        if self.data().open.as_ref().is_some_and(|open| &open.conversation == conversation) {
            self.refresh().await;
        }
    }

    /// La lista de a quién nombrar con ese texto y el cursor ahí, si se está
    /// escribiendo un `@algo` donde se puede nombrar. Con la gente de la
    /// organización que ya trajo comunicación (`Comms::load_people`).
    pub fn mention_menu(&self, conversation: &Conversation, text: &str, cursor: u32, mark: Option<MentionMark>) -> Option<MentionMenu> {
        let allowed = matches!(conversation, Conversation::Channel { .. });
        let team: Vec<UserSummary> = self.core.comms().people("").into_iter().map(UserSummary::from).collect();
        crate::task::mention_menu(&team, text, cursor as usize, allowed, mark.as_ref())
    }

    // ── Lo que hay dentro de un mensaje ─────────────────────────────────

    /// A qué lleva un enlace de un mensaje.
    pub fn link(&self, href: &str) -> ThreadLink {
        thread_link(href, self.core.base_url())
    }

    /// Sigue un enlace de un mensaje, hasta donde llega el core: una tarea
    /// se abre en su página y un documento, en la documentación (saliendo
    /// de comunicación). Lo demás lo devuelve para quien pinta: un fichero
    /// hay que bajarlo, y lo de fuera es del navegador.
    pub async fn follow_link(&self, href: &str) -> ThreadLink {
        let link = self.link(href);
        match &link {
            ThreadLink::Task { id } => {
                if self.core.tasks().open_written(id).await {
                    link
                } else {
                    ThreadLink::Unknown
                }
            }
            ThreadLink::Doc => {
                let known = match doc_link(href) {
                    DocLink::Doc { kind, id, .. } => crate::tree_rows::find_node(&self.core.tree().state().spaces, &id).map(|(found, _)| found) == Some(kind),
                    _ => false,
                };
                if !known {
                    return ThreadLink::Unknown;
                }
                self.core.comms().leave();
                self.core.docs().follow_link(href).await;
                link
            }
            _ => link,
        }
    }

    /// Los bytes de un fichero citado en un mensaje. Sólo de este servidor,
    /// y con la sesión en la cabecera: su dirección la pide.
    pub async fn file_bytes(&self, path: &str) -> Result<Vec<u8>, ApiError> {
        self.core.download(served_path(path)?).await
    }

    /// Guarda en `target` un fichero citado en un mensaje. Va a disco según
    /// llega y sólo toma el nombre al terminar entero: un vídeo de cientos
    /// de megas no tiene por qué pasar por la memoria. Lo que hay que tener
    /// en memoria para pintarlo (una miniatura) va por [`Self::file_bytes`].
    pub async fn save_file(&self, path: &str, target: &std::path::Path) -> Result<(), ApiError> {
        self.core.download_to(served_path(path)?, target).await
    }

    /// Una imagen citada en un mensaje, para enseñarla. Lo que llega y no es
    /// una imagen no se da por una.
    pub async fn picture(&self, path: &str) -> Result<Picture, ApiError> {
        let bytes = self.file_bytes(path).await?;
        let mime = image_type(&bytes).ok_or_else(|| ApiError::Server { status: 400, code: "not-an-image".into(), message: "That file is not an image".into() })?;
        Ok(Picture { mime: mime.to_string(), bytes })
    }
}

/// La ruta de un fichero de la conversación, si es de este servidor: la
/// sesión sólo viaja ahí.
fn served_path(path: &str) -> Result<&str, ApiError> {
    attachment_path(path).ok_or_else(|| ApiError::Server { status: 400, code: "not-an-attachment".into(), message: "That file does not live in this server".into() })
}

// ── Las imágenes de la conversación ─────────────────────────────────────

/// Una imagen de la conversación, para el visor: de qué mensaje es, dónde
/// se baja y cómo se llama.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ThreadPicture {
    pub message_id: String,
    /// La ruta del servidor: con ella se baja, con la sesión.
    pub path: String,
    pub name: String,
    /// Quién la mandó, como lo firma el mensaje.
    pub author: String,
}

/// Las imágenes de los mensajes cargados, de la más vieja a la más nueva:
/// las que el visor recorre con las flechas. Las de un mismo mensaje, en el
/// orden en que van en él. Las de fuera de este servidor no están: no se
/// bajan (`message_body`).
pub fn thread_pictures(messages: &[ThreadMessage], base_url: &str) -> Vec<ThreadPicture> {
    messages
        .iter()
        .flat_map(|message| {
            message_body(&message.body, base_url).pictures.into_iter().map(move |picture| ThreadPicture {
                message_id: message.id.clone(),
                path: picture.path,
                name: picture.name,
                author: message.author_name.clone(),
            })
        })
        .collect()
}

/// A cuál se pasa desde la de `at` moviéndose `by` (−1 la anterior, 1 la
/// siguiente). Sin dar la vuelta: desde la primera no se va a la última,
/// como en WhatsApp. `None` si no hay a dónde ir.
pub fn picture_step(count: usize, at: usize, by: i32) -> Option<usize> {
    let to = at as i64 + by as i64;
    (to >= 0 && (to as usize) < count && to as usize != at).then_some(to as usize)
}

// ── Escribir ────────────────────────────────────────────────────────────

/// Lo más largo que acepta el servidor, en caracteres: el `max=8000` de
/// `domain.ChatMessageRequest` y `domain.DMMessageRequest`, que en un texto
/// cuenta runas, no bytes.
pub const MESSAGE_MAX_CHARS: u32 = 8000;

/// Un rechazo de aquí, antes de pedir nada: con su código, como los del
/// servidor, para que quien pinta lo diga con la misma frase.
fn local(code: &str, message: &str) -> ApiError {
    ApiError::Server { status: 0, code: code.to_string(), message: message.to_string() }
}

/// Lo que se manda de lo escrito: sin espacios alrededor, y sólo si dice
/// algo y cabe.
pub fn message_to_send(written: &str) -> Result<String, ApiError> {
    let body = written.trim();
    if body.is_empty() {
        return Err(local("empty-body", "The message is empty"));
    }
    if body.chars().count() > MESSAGE_MAX_CHARS as usize {
        return Err(local("too-long", "The message is longer than the server accepts"));
    }
    Ok(body.to_string())
}

/// La ruta por la que se escribe en esa conversación.
fn send_path(conversation: &Conversation) -> String {
    match conversation {
        Conversation::Channel { space_id } => format!("/api/v1/task-spaces/{space_id}/chat"),
        Conversation::Direct { conversation_id } => format!("/api/v1/dm/{conversation_id}/messages"),
    }
}

/// Si Enter, con lo escrito y el cursor ahí, manda el mensaje.
///
/// Dentro de un bloque de código abierto (una valla ```` ``` ```` sin cerrar
/// antes del cursor) no: ahí se está escribiendo código, y Enter es otra
/// línea. En lo demás sí, también en una lista: es un chat, no un documento
/// (Mayús+Enter salta de línea siempre).
pub fn enter_sends(text: &str, cursor: u32) -> bool {
    let cursor = (cursor as usize).min(text.len());
    let Some(before) = text.get(..cursor) else { return true };
    before.lines().filter(|line| line.trim_start().starts_with("```")).count() % 2 == 0
}

/// Una ficha dentro de la caja de escribir: lo que vale (`text`, el
/// markdown que se manda) y lo que se ve (`label`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ComposerToken {
    /// A qué se refiere: la persona o el fichero.
    pub id: String,
    pub text: String,
    pub label: String,
}

/// La ficha de nombrar a esa persona: vale el enlace que el servidor busca
/// para avisar, y enseña su nombre.
pub fn mention_token(person: &MentionPerson) -> ComposerToken {
    ComposerToken { id: person.id.clone(), text: crate::task::mention_link(person), label: format!("@{}", person.name) }
}

/// La ficha de un fichero subido: vale su cita (una imagen como imagen, lo
/// demás como enlace) y enseña su nombre. `None` si no vive en este
/// servidor: no hay con qué citarlo.
pub fn file_token(file: &ChatFile) -> Option<ComposerToken> {
    let image = crate::task::named_image(file.content_type.as_deref(), &file.file_name);
    let text = crate::task::citation(&file.url, &file.file_name, image)?;
    let label = file.file_name.replace(['\n', '\r'], " ");
    Some(ComposerToken { id: file.id.clone(), text, label })
}

/// La ruta de un mensaje de esa conversación: por ella se corrige y se
/// retira, y debajo cuelgan sus reacciones.
fn message_path(conversation: &Conversation, message_id: &str) -> String {
    match conversation {
        Conversation::Channel { space_id } => format!("/api/v1/task-spaces/{space_id}/chat/{message_id}"),
        Conversation::Direct { conversation_id } => format!("/api/v1/dm/{conversation_id}/messages/{message_id}"),
    }
}

/// Las reacciones de un mensaje tras poner (`on`) o quitar la de quien
/// mira, sin esperar al servidor (`applyReaction` en `lib/reactions.ts`). Un
/// emoji que se queda a cero desaparece; uno nuevo va al final, que es donde
/// lo pondrá el servidor (por orden del primero). Al quitar se quita **un**
/// nombre igual al de quien mira: puede coincidir con el de otra persona, y
/// la lista exacta llega al releer.
pub fn apply_reaction(list: &[ReactionSummary], emoji: &str, on: bool, me: &str) -> Vec<ReactionSummary> {
    let mut out = list.to_vec();
    let found = out.iter().position(|reaction| reaction.emoji == emoji);
    if on {
        match found {
            Some(at) if out[at].mine => {}
            Some(at) => {
                out[at].count += 1;
                out[at].mine = true;
                out[at].names.push(me.to_string());
            }
            None => out.push(ReactionSummary { emoji: emoji.to_string(), count: 1, mine: true, names: vec![me.to_string()] }),
        }
        return out;
    }
    let Some(at) = found.filter(|at| out[*at].mine) else { return out };
    if out[at].count <= 1 {
        out.remove(at);
        return out;
    }
    let reaction = &mut out[at];
    reaction.count -= 1;
    reaction.mine = false;
    match reaction.names.iter().rposition(|name| name == me) {
        Some(name) => {
            reaction.names.remove(name);
        }
        None => {
            reaction.names.pop();
        }
    }
    out
}

/// La ruta por la que se sube un fichero a esa conversación.
fn upload_path(conversation: &Conversation) -> String {
    match conversation {
        Conversation::Channel { space_id } => format!("/api/v1/task-spaces/{space_id}/chat/attachments"),
        Conversation::Direct { conversation_id } => format!("/api/v1/dm/{conversation_id}/attachments"),
    }
}

// ── Cómo se lee cada línea ──────────────────────────────────────────────

/// Si un mensaje va pegado al anterior, sin su cabecera (`sameVoice`).
///
/// Sólo si es de la misma persona **y** entró por el mismo sitio: un agente
/// escribe con el token de su dueño, y agrupar por autor pegaba lo del
/// agente debajo de lo de la persona y le quitaba la cabecera, que es donde
/// va «vía agente». Y una línea de cac no se pega a lo de quien la provocó,
/// ni al revés: la columna guarda a esa persona, y pegadas la línea de cac
/// se quedaría sin su marca. (La app de siempre no mira esto último.)
pub(crate) fn same_voice(previous: Option<&ThreadMessage>, message: &ThreadMessage) -> bool {
    previous.is_some_and(|previous| {
        previous.author_user_id == message.author_user_id
            && previous.via.as_deref().unwrap_or("") == message.via.as_deref().unwrap_or("")
            && previous.system == message.system
    })
}

/// La hora de un mensaje (`horaCorta`): dos cifras y dos cifras, en la zona
/// de quien mira.
pub(crate) fn short_time<Viewer: TimeZone>(iso: &str, zone: &Viewer, locale: Locale) -> String {
    let Some(at) = instant(iso) else { return String::new() };
    let at = at.with_timezone(zone);
    match locale {
        Locale::Es => format!("{:02}:{:02}", at.hour(), at.minute()),
        Locale::En => {
            let (pm, hour) = at.hour12();
            format!("{hour:02}:{:02} {}", at.minute(), if pm { "PM" } else { "AM" })
        }
    }
}

/// Cómo se nombra un día en la raya que separa los mensajes de uno y de
/// otro: «Hoy», «Ayer», «5 de enero», y con su año si no es éste.
pub(crate) fn day_label(day: NaiveDate, today: NaiveDate, locale: Locale) -> String {
    use chrono::Datelike;
    match (today - day).num_days() {
        0 => text(locale, "common:calendar.today"),
        1 => text(locale, "chat:yesterday"),
        _ => {
            let title = crate::mywork::day_title(locale, day);
            if day.year() == today.year() {
                title
            } else {
                match locale {
                    Locale::Es => format!("{title} de {}", day.year()),
                    Locale::En => format!("{title}, {}", day.year()),
                }
            }
        }
    }
}

/// La cita encima de una respuesta (`ReplyQuote.tsx`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ThreadQuote {
    /// El mensaje citado: pulsar la cita lleva a él si está cargado.
    pub id: String,
    pub author: String,
    /// Lo que decía, en una línea y sin markdown. Vacío si se retiró.
    pub text: String,
    /// Lo citado se retiró: se dice en vez de enseñar nada.
    pub gone: bool,
}

/// Una línea de la conversación, con lo que la rodea ya decidido.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ThreadLine {
    pub id: String,
    /// Es la primera de su día: encima va la raya con ese nombre.
    pub day: Option<String>,
    /// Va pegada a la anterior, sin cabecera.
    pub grouped: bool,
    /// La escribió quien mira: a la derecha. Una línea de cac, nunca.
    pub mine: bool,
    /// La puso cac: lleva su marca en vez de un nombre.
    pub system: bool,
    /// Quién firma: «Tú», el nombre, o «cac».
    pub author: String,
    /// La escribió un agente con el token de esa persona.
    pub via_agent: bool,
    pub time: String,
    pub edited: bool,
    pub quote: Option<ThreadQuote>,
    /// En un directo, las palomitas de lo propio: leído o sólo enviado.
    pub ticks: Option<bool>,
    /// Quien mira puede corregirlo y retirarlo: es suyo y no es de cac.
    pub can_edit: bool,
    pub reactions: Vec<ReactionSummary>,
}

/// Las líneas de una conversación, para quien mira (`Message` en
/// `ChannelView.tsx` y `DMLine` en `DMThread.tsx`).
///
/// **Que una línea es de cac va antes que de quién es**: su columna de autor
/// guarda a quien la provocó, y sin eso esa persona vería su aviso
/// automático a la derecha, firmado «Tú».
pub fn thread_lines<Viewer: TimeZone>(state: &ThreadState, me: Option<&str>, locale: Locale, now: &DateTime<Viewer>) -> Vec<ThreadLine> {
    let zone = now.timezone();
    let today = now.date_naive();
    let direct = matches!(state.conversation, Some(Conversation::Direct { .. }));
    let mut previous: Option<(&ThreadMessage, Option<NaiveDate>)> = None;
    let mut lines = Vec::with_capacity(state.messages.len());
    for message in &state.messages {
        let day = instant(&message.created_at).map(|at| at.with_timezone(&zone).date_naive());
        // Un día nuevo es un turno nuevo: lleva su raya, y su cabecera.
        let new_day = previous.is_none_or(|(_, before)| before != day);
        let mine = !message.system && me.is_some_and(|me| me == message.author_user_id);
        let author = if message.system {
            text(locale, "chat:systemAuthor")
        } else if mine {
            text(locale, "chat:you")
        } else if message.author_name.is_empty() {
            text(locale, "chat:unknownAuthor")
        } else {
            message.author_name.clone()
        };
        lines.push(ThreadLine {
            id: message.id.clone(),
            day: day.filter(|_| new_day).map(|day| day_label(day, today, locale)),
            grouped: !new_day && same_voice(previous.map(|(before, _)| before), message),
            mine,
            system: message.system,
            author,
            via_agent: message.via.as_deref() == Some(VIA_AGENT),
            time: short_time(&message.created_at, &zone, locale),
            edited: !message.system && !message.updated_at.is_empty() && message.updated_at != message.created_at,
            quote: message.reply_to.as_ref().map(|quote| ThreadQuote {
                id: quote.id.clone(),
                author: if quote.author_name.is_empty() { text(locale, "chat:unknownAuthor") } else { quote.author_name.clone() },
                text: if quote.gone { String::new() } else { quote_text(&quote.body) },
                gone: quote.gone,
            }),
            ticks: (direct && mine).then(|| was_read(Some(&message.created_at), state.other_read_at.as_deref())),
            can_edit: mine,
            reactions: message.reactions.clone(),
        });
        previous = Some((message, day));
    }
    lines
}

/// Qué hacerle a una lista ya pintada para que pase de unas líneas a otras:
/// de `start`, quitar `removed` y poner `inserted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ThreadSplice {
    pub start: u32,
    pub removed: u32,
    pub inserted: u32,
}

/// El cambio más pequeño que lleva de una lista de ids a otra, o `None` si
/// son la misma. Lo que coincide por delante y por detrás no se toca: así
/// una página de antes sólo añade filas arriba y un mensaje nuevo sólo una
/// abajo, y la lista de quien pinta no pierde por dónde iba la lectura.
pub fn thread_splice(before: &[String], after: &[String]) -> Option<ThreadSplice> {
    let head = before.iter().zip(after).take_while(|(left, right)| left == right).count();
    let room = before.len().min(after.len()) - head;
    let tail = before.iter().rev().zip(after.iter().rev()).take(room).take_while(|(left, right)| left == right).count();
    let (removed, inserted) = (before.len() - head - tail, after.len() - head - tail);
    (removed > 0 || inserted > 0).then_some(ThreadSplice { start: head as u32, removed: removed as u32, inserted: inserted as u32 })
}

// ── El cuerpo de un mensaje ─────────────────────────────────────────────

/// Un fichero citado en un mensaje: su ruta en este servidor y su nombre.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BodyFile {
    pub path: String,
    pub name: String,
}

/// Un mensaje partido en lo que se pinta de cada manera.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MessageBody {
    /// El markdown que se lee, ya sin las imágenes ni los ficheros sueltos.
    pub text: String,
    /// Las imágenes subidas a la conversación: van debajo, en miniatura.
    pub pictures: Vec<BodyFile>,
    /// Los ficheros que iban solos en su línea: van debajo, con su nombre.
    pub files: Vec<BodyFile>,
}

/// La ruta de un adjunto de este servidor a la que apunta esa dirección, o
/// `None` si apunta a otra cosa (`attachmentPath` en `lib/media.ts`).
///
/// Vale la ruta sola, que es como se guarda, y la dirección entera de este
/// servidor, que es como la dejaba un viaje de ida y vuelta por el editor;
/// una credencial que viniera pegada se tira. Lo de cualquier otro servidor
/// no: lo que se pide lleva la sesión, y la sesión no sale de éste.
pub(crate) fn body_attachment(url: &str, base_url: &str) -> Option<String> {
    let path = url.strip_prefix(base_url).unwrap_or(url);
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    attachment_path(path).filter(|path| path.contains("/attachments/")).map(str::to_string)
}

/// Un `[texto](destino)` o `![texto](destino)` que empieza en `at`: su
/// texto, su destino y dónde acaba.
fn link_at(chars: &[char], at: usize) -> Option<(String, String, usize)> {
    if chars.get(at) != Some(&'[') {
        return None;
    }
    let close = (at + 1..chars.len()).find(|index| chars[*index] == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len()).find(|index| chars[*index] == ')')?;
    let label: String = chars[at + 1..close].iter().collect();
    let target: String = chars[close + 2..end].iter().collect();
    // `(destino "título")`: el destino es lo de antes del espacio.
    let target = target.split_whitespace().next().unwrap_or("").to_string();
    Some((label, target, end + 1))
}

/// Parte el cuerpo de un mensaje en lo que se lee y lo que se enseña debajo.
///
/// Las imágenes de un mensaje son adjuntos de este servidor, y su dirección
/// pide la sesión: el pintor de markdown las pediría sin ella. Así que se
/// sacan del texto y quien pinta las baja por el core. Un fichero que va
/// solo en su línea —como lo deja subirlo— se saca igual; citado en mitad
/// de una frase se queda como enlace, que al pulsarlo se baja por el mismo
/// camino. Una imagen de fuera no se pide sola: queda como un enlace con su
/// nombre (pedirla diría a ese servidor quién lee el mensaje, y cuándo).
/// Lo de dentro de un bloque de código o entre comillas invertidas no se
/// toca.
pub fn message_body(body: &str, base_url: &str) -> MessageBody {
    let mut out = MessageBody::default();
    let mut lines: Vec<String> = Vec::new();
    let mut fence = "";
    for line in body.split('\n') {
        let trimmed = line.trim_start_matches([' ', '\t']);
        let marker = ["```", "~~~"].into_iter().find(|marker| trimmed.starts_with(marker)).unwrap_or("");
        if !fence.is_empty() || !marker.is_empty() {
            if fence.is_empty() {
                fence = marker;
            } else if marker == fence {
                fence = "";
            }
            lines.push(line.to_string());
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut kept = String::with_capacity(line.len());
        let mut in_code = false;
        let mut took_something = false;
        let mut at = 0;
        while at < chars.len() {
            if chars[at] == '`' {
                in_code = !in_code;
            }
            let image = !in_code && chars[at] == '!' && chars.get(at + 1) == Some(&'[');
            let Some((label, target, end)) = link_at(&chars, if image { at + 1 } else { at }) else {
                kept.push(chars[at]);
                at += 1;
                continue;
            };
            let attachment = body_attachment(&target, base_url);
            match (image, attachment) {
                (true, Some(path)) => {
                    if !out.pictures.iter().any(|picture| picture.path == path) {
                        out.pictures.push(BodyFile { path, name: label });
                    }
                    took_something = true;
                }
                (true, None) => {
                    kept.push_str(&format!("[🖼{}{label}]({target})", if label.is_empty() { "" } else { " " }));
                }
                (false, Some(path)) => {
                    // Solo en su línea: es un fichero subido, no una cita en
                    // una frase.
                    let before: String = chars[..at].iter().collect();
                    let after: String = chars[end..].iter().collect();
                    if before.trim().is_empty() && after.trim().is_empty() {
                        if !out.files.iter().any(|file| file.path == path) {
                            out.files.push(BodyFile { path, name: label });
                        }
                        took_something = true;
                    } else {
                        kept.extend(&chars[at..end]);
                    }
                }
                (false, None) => kept.extend(&chars[at..end]),
            }
            at = end;
        }
        // La línea que sólo llevaba lo que se sacó no deja un hueco.
        if took_something && kept.trim().is_empty() {
            continue;
        }
        lines.push(kept);
    }
    out.text = lines.join("\n").trim().to_string();
    out
}

/// A qué lleva un enlace de un mensaje.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ThreadLink {
    /// Un fichero de este servidor: se baja con la sesión.
    File { path: String },
    /// Una tarea: se abre en su página.
    Task { id: String },
    /// Un documento, una página o una sección.
    Doc,
    /// Una grabación: se abre en la pestaña Grabaciones del canal.
    Recording { id: String },
    /// Algo de fuera: lo abre el sistema.
    Web { url: String },
    /// Nombra a alguien sin llevar a ningún sitio.
    Inert,
    /// De cac, pero esta app todavía no lo abre: una llamada.
    NotHere,
    /// No se sabe abrir, o ya no está.
    Unknown,
}

/// Qué es ese enlace (`openInternalRef` en `lib/open-ref.ts`, con la regla
/// de `doc_links` para lo de dentro).
pub(crate) fn thread_link(href: &str, base_url: &str) -> ThreadLink {
    let href = href.trim();
    if let Some(path) = body_attachment(href, base_url) {
        return ThreadLink::File { path };
    }
    if let Some(id) = href.strip_prefix("cac:recording/") {
        // Lo mismo que `recordingIdFromHref`: sólo un id con forma de uuid
        // llega a la pestaña; lo demás es un enlace roto.
        let well_formed = id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        return if well_formed { ThreadLink::Recording { id: id.to_string() } } else { ThreadLink::Unknown };
    }
    match doc_link(href) {
        DocLink::Task { id } => ThreadLink::Task { id },
        DocLink::Doc { .. } => ThreadLink::Doc,
        DocLink::Web { url } => ThreadLink::Web { url },
        DocLink::Inert => ThreadLink::Inert,
        // Un ancla es de un documento: en un mensaje no hay títulos a los
        // que saltar.
        DocLink::Anchor { .. } => ThreadLink::Unknown,
        DocLink::Unknown if href.starts_with("cac:") => ThreadLink::NotHere,
        DocLink::Unknown => ThreadLink::Unknown,
    }
}

/// Quién anda por aquí, para la cabecera de un canal (`QuienAnda` en
/// `ChannelView.tsx`): tres nombres y cuántos más. `None` si no hay nadie.
pub fn around_label(names: &[String]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let mut label = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > 3 {
        label.push_str(&format!(" +{}", names.len() - 3));
    }
    Some(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El visor recorre las imágenes de todos los mensajes cargados, en el
    /// orden en que se leen, y no da la vuelta.
    ///
    /// Roturas que atrapa: sólo las del mensaje que se pulsó; el orden al
    /// revés (→ iría a la anterior); dar la vuelta de la última a la
    /// primera; contar las de otro servidor, que no se pueden bajar.
    #[test]
    fn the_pictures_of_a_conversation_go_in_reading_order_without_wrapping() {
        let base = "https://cac.example";
        let mut first = message("m1", "u2", "2026-10-15T10:00:00Z");
        first.body = "![uno.png](/api/v1/task-spaces/a/chat/attachments/a1/raw)".into();
        let mut middle = message("m2", "u3", "2026-10-15T10:01:00Z");
        middle.body = "sin imagen".into();
        let mut last = message("m3", "u2", "2026-10-15T10:02:00Z");
        last.body = "dos:\n\n![dos.png](/api/v1/task-spaces/a/chat/attachments/a2/raw)\n\n![fuera.png](https://otro.example/x.png)\n\n![tres.png](/api/v1/task-spaces/a/chat/attachments/a3/raw)".into();
        let pictures = thread_pictures(&[first, middle, last], base);
        let names: Vec<&str> = pictures.iter().map(|picture| picture.name.as_str()).collect();
        assert_eq!(names, ["uno.png", "dos.png", "tres.png"]);
        assert_eq!((pictures[1].message_id.as_str(), pictures[1].author.as_str()), ("m3", "N-u2"));
        assert_eq!(pictures[0].path, "/api/v1/task-spaces/a/chat/attachments/a1/raw");

        assert_eq!(picture_step(3, 0, 1), Some(1));
        assert_eq!(picture_step(3, 1, -1), Some(0));
        assert_eq!(picture_step(3, 2, 1), None, "se dio la vuelta a la primera");
        assert_eq!(picture_step(3, 0, -1), None, "se dio la vuelta a la última");
        assert_eq!(picture_step(0, 0, 1), None);
    }

    /// Enter manda, salvo dentro de un bloque de código abierto antes del
    /// cursor; uno ya cerrado no cuenta, ni las comillas sueltas de una
    /// línea.
    ///
    /// Roturas que atrapa: mandar a medio escribir un bloque de código;
    /// no mandar nunca después de haber cerrado uno; no mandar en una
    /// lista (aquí es un chat).
    #[test]
    fn enter_sends_except_inside_an_open_code_block() {
        assert!(enter_sends("hola", 4));
        assert!(enter_sends("- uno\n- dos", 11));
        let open = "mira:\n```rust\nfn main() {";
        assert!(!enter_sends(open, open.len() as u32));
        let closed = "```\nx\n```\ny ya";
        assert!(enter_sends(closed, closed.len() as u32));
        assert!(enter_sends("con `código` suelto", 19));
        // El cursor antes de la valla manda; después, no.
        let fence = "antes\n```\ndentro";
        assert!(enter_sends(fence, 5));
        assert!(!enter_sends(fence, fence.len() as u32));
    }
    use chrono::FixedOffset;

    fn message(id: &str, author: &str, at: &str) -> ThreadMessage {
        ThreadMessage { id: id.into(), author_user_id: author.into(), author_name: format!("N-{author}"), body: id.into(), created_at: at.into(), updated_at: at.into(), ..ThreadMessage::default() }
    }

    fn ids(messages: &[ThreadMessage]) -> Vec<&str> {
        messages.iter().map(|message| message.id.as_str()).collect()
    }

    fn mexico() -> DateTime<FixedOffset> {
        FixedOffset::west_opt(6 * 3600).unwrap().with_ymd_and_hms(2026, 10, 15, 10, 0, 0).unwrap()
    }

    /// Refrescar funde la ventana con lo que hay.
    ///
    /// Roturas que atrapa: tirar las páginas viejas cargadas subiendo;
    /// dejar pintado un mensaje retirado dentro de la ventana; perder una
    /// edición; dar por nuevo lo que ya estaba, o no dar por nuevo lo que
    /// llegó; el único mensaje de una conversación que sigue ahí después
    /// de retirarlo; el orden por texto, que pone «…:00Z» detrás de
    /// «…:00.5Z».
    #[test]
    fn a_refresh_merges_the_window_into_what_is_held() {
        let held = vec![
            message("viejo-1", "u2", "2026-10-15T09:00:00Z"),
            message("viejo-2", "u2", "2026-10-15T09:30:00Z"),
            message("a", "u2", "2026-10-15T10:00:00Z"),
            message("retirado", "u2", "2026-10-15T10:00:01Z"),
            message("b", "u2", "2026-10-15T10:00:02Z"),
        ];
        let mut edited = message("b", "u2", "2026-10-15T10:00:02Z");
        edited.body = "corregido".into();
        let (all, arrived) = merged(&held, vec![message("a", "u2", "2026-10-15T10:00:00Z"), edited.clone(), message("c", "u3", "2026-10-15T10:00:03Z")]);
        assert_eq!(ids(&all), ["viejo-1", "viejo-2", "a", "b", "c"]);
        assert_eq!(all[3].body, "corregido");
        assert!(arrived);

        // Sólo se retiró uno: nada nuevo.
        let (all, arrived) = merged(&held, vec![message("a", "u2", "2026-10-15T10:00:00Z"), message("b", "u2", "2026-10-15T10:00:02Z")]);
        assert_eq!(ids(&all), ["viejo-1", "viejo-2", "a", "b"]);
        assert!(!arrived, "un mensaje retirado se dio por uno nuevo");

        // Lo mismo que había: nada nuevo.
        assert!(!merged(&held, held.clone()).1);
        // La primera línea de una conversación vacía sí es una llegada.
        assert!(merged(&[], vec![message("a", "u2", "2026-10-15T10:00:00Z")]).1);
        // Una ventana vacía es una conversación vacía.
        assert_eq!(merged(&held[..1], Vec::new()), (Vec::new(), false));

        // Por el instante, no por el texto.
        let (all, _) = merged(&[], vec![message("medio", "u2", "2026-10-15T10:00:00.5Z"), message("en-punto", "u2", "2026-10-15T10:00:00Z")]);
        assert_eq!(ids(&all), ["en-punto", "medio"]);
    }

    /// Cuándo puede quedar un agujero entre lo que se tenía y lo leído.
    ///
    /// Roturas que atrapa: no ver el agujero tras un corte largo; verlo
    /// donde no lo hay (una ventana que comparte un mensaje, una que vino
    /// corta, una conversación recién abierta) y pedir páginas de más en
    /// cada mensaje que llega.
    #[test]
    fn a_full_window_that_shares_nothing_may_leave_a_gap() {
        let held = vec![message("h1", "u2", "2026-10-15T09:00:00Z")];
        let full: Vec<ThreadMessage> = (0..THREAD_PAGE).map(|index| message(&format!("n{index}"), "u2", &format!("2026-10-15T10:{:02}:00Z", index))).collect();
        assert!(may_leave_a_gap(&held, &full));
        assert!(!may_leave_a_gap(&held, &full[1..]));
        assert!(!may_leave_a_gap(&[], &full));
        let mut touching = full.clone();
        touching[0] = held[0].clone();
        assert!(!may_leave_a_gap(&held, &touching));
    }

    /// El cursor viaja tal como llegó, escapado entero.
    ///
    /// Roturas que atrapa: un `+` de zona horaria que llega como espacio
    /// (el servidor no entiende el cursor y repite la primera página); los
    /// nanosegundos recortados; la ruta de un canal para un directo.
    #[test]
    fn the_cursor_is_sent_exactly_as_it_came() {
        let channel = Conversation::Channel { space_id: "s1".into() };
        let direct = Conversation::Direct { conversation_id: "c1".into() };
        assert_eq!(page_path(&channel, None), "/api/v1/task-spaces/s1/chat?limit=50");
        assert_eq!(page_path(&direct, None), "/api/v1/dm/c1/messages?limit=50");
        assert_eq!(
            page_path(&channel, Some("2026-10-15T10:00:00.123456789+02:00")),
            "/api/v1/task-spaces/s1/chat?limit=50&before=2026-10-15T10%3A00%3A00.123456789%2B02%3A00"
        );
        assert_eq!(page_path(&direct, Some("2026-10-15T10:00:00.1Z")), "/api/v1/dm/c1/messages?limit=50&before=2026-10-15T10%3A00%3A00.1Z");
        assert_eq!(read_path(&channel), "/api/v1/task-spaces/s1/chat/read");
        assert_eq!(read_path(&direct), "/api/v1/dm/c1/read");
    }

    fn state(conversation: Conversation, messages: Vec<ThreadMessage>) -> ThreadState {
        ThreadState { conversation: Some(conversation), messages, ..ThreadState::default() }
    }

    /// Los turnos seguidos de una misma voz se pegan, y un día nuevo abre
    /// turno.
    ///
    /// Roturas que atrapa: el mensaje de un agente pegado al de su dueño
    /// (pierde la cabecera, que es donde dice «vía agente»); dos personas
    /// pegadas; una línea de cac pegada a lo de quien la provocó; el primer
    /// mensaje de un día sin su raya ni su cabecera; una raya en cada
    /// mensaje; el día contado en Greenwich.
    #[test]
    fn consecutive_turns_of_one_voice_are_glued_and_a_new_day_starts_one() {
        let mut agent = message("m3", "u2", "2026-10-15T15:02:00Z");
        agent.via = Some("mcp".into());
        let mut system = message("m5", "u2", "2026-10-15T15:04:00Z");
        system.system = true;
        let messages = vec![
            // Las 05:30 UTC son las 23:30 del 13 para quien mira.
            message("m0", "u2", "2026-10-14T05:30:00Z"),
            message("m1", "u2", "2026-10-15T15:00:00Z"),
            message("m2", "u2", "2026-10-15T15:01:00Z"),
            agent,
            message("m4", "u2", "2026-10-15T15:03:00Z"),
            system,
            message("m6", "u3", "2026-10-15T15:05:00Z"),
        ];
        let lines = thread_lines(&state(Conversation::Channel { space_id: "s".into() }, messages), Some("u1"), Locale::Es, &mexico());
        let grouped: Vec<bool> = lines.iter().map(|line| line.grouped).collect();
        assert_eq!(grouped, [false, false, true, false, false, false, false]);
        let days: Vec<Option<&str>> = lines.iter().map(|line| line.day.as_deref()).collect();
        assert_eq!(days, [Some("13 de octubre"), Some("Hoy"), None, None, None, None, None]);
        assert!(lines[3].via_agent && !lines[2].via_agent);
        assert_eq!(lines[1].time, "09:00");
        assert_eq!(short_time("2026-10-15T21:05:00Z", &mexico().timezone(), Locale::En), "03:05 PM");
    }

    /// De quién es cada línea, y cómo firma.
    ///
    /// Roturas que atrapa: una línea de cac pintada como propia de quien la
    /// provocó («Tú», a la derecha, «editado»); lo propio sin marcar; lo de
    /// otro como propio; un autor sin nombre en blanco; «editado» en lo que
    /// no se tocó.
    #[test]
    fn a_system_line_is_never_mine_and_each_line_says_who_signs() {
        let mut system = message("s", "u1", "2026-10-15T15:00:00Z");
        system.system = true;
        system.updated_at = "2026-10-15T15:09:00Z".into();
        let mut edited = message("e", "u1", "2026-10-15T15:01:00Z");
        edited.updated_at = "2026-10-15T15:02:00Z".into();
        let mut nameless = message("n", "u7", "2026-10-15T15:03:00Z");
        nameless.author_name = String::new();
        let lines = thread_lines(
            &state(Conversation::Channel { space_id: "s".into() }, vec![system, edited, message("o", "u2", "2026-10-15T15:02:30Z"), nameless]),
            Some("u1"),
            Locale::Es,
            &mexico(),
        );
        assert_eq!((lines[0].system, lines[0].mine, lines[0].author.as_str(), lines[0].edited), (true, false, "cac", false));
        assert_eq!((lines[1].mine, lines[1].author.as_str(), lines[1].edited), (true, "Tú", true));
        assert_eq!((lines[2].mine, lines[2].author.as_str(), lines[2].edited), (false, "N-u2", false));
        assert_eq!(lines[3].author, "desconocido");
        // Sin nadie dentro, nada es propio.
        assert!(thread_lines(&state(Conversation::Channel { space_id: "s".into() }, vec![message("x", "u1", "2026-10-15T15:00:00Z")]), None, Locale::Es, &mexico()).iter().all(|line| !line.mine));
    }

    /// Las palomitas son de lo propio, en un directo, y la cita dice lo que
    /// citaba en limpio.
    ///
    /// Roturas que atrapa: palomitas en un canal, o en lo que escribió el
    /// otro; leído lo que se escribió después de que el otro leyera; la
    /// cita con su markdown crudo; el texto de un mensaje retirado todavía
    /// en su cita.
    #[test]
    fn ticks_are_for_my_lines_in_a_direct_and_a_quote_reads_clean() {
        let mut reply = message("d3", "u2", "2026-10-15T15:10:00Z");
        reply.reply_to = Some(MessageQuote { id: "d1".into(), author_name: "Ana Ruiz".into(), body: "¿sale **hoy**?".into(), gone: false });
        let mut orphan = message("d4", "u2", "2026-10-15T15:11:00Z");
        orphan.reply_to = Some(MessageQuote { id: "d0".into(), author_name: String::new(), body: "no debería verse".into(), gone: true });
        let messages = vec![message("d1", "u1", "2026-10-15T15:00:00Z"), message("d2", "u1", "2026-10-15T15:06:00Z"), reply, orphan];
        let mut direct = state(Conversation::Direct { conversation_id: "c1".into() }, messages.clone());
        direct.other_read_at = Some("2026-10-15T15:05:00Z".into());
        let lines = thread_lines(&direct, Some("u1"), Locale::Es, &mexico());
        assert_eq!(lines.iter().map(|line| line.ticks).collect::<Vec<_>>(), [Some(true), Some(false), None, None]);
        let quote = lines[2].quote.clone().unwrap();
        assert_eq!((quote.id.as_str(), quote.author.as_str(), quote.text.as_str(), quote.gone), ("d1", "Ana Ruiz", "¿sale hoy?", false));
        let gone = lines[3].quote.clone().unwrap();
        assert_eq!((gone.text.as_str(), gone.gone, gone.author.as_str()), ("", true, "desconocido"));

        let channel = thread_lines(&state(Conversation::Channel { space_id: "s".into() }, messages), Some("u1"), Locale::Es, &mexico());
        assert!(channel.iter().all(|line| line.ticks.is_none()));
    }

    /// El día de la raya.
    ///
    /// Roturas que atrapa: una fecha donde va «Hoy» o «Ayer»; el año
    /// puesto siempre, o nunca; el nombre del mes en el otro idioma.
    #[test]
    fn a_day_is_named_as_people_say_it() {
        let today = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let day = |year, month, day| NaiveDate::from_ymd_opt(year, month, day).unwrap();
        assert_eq!(day_label(today, today, Locale::Es), "Hoy");
        assert_eq!(day_label(day(2026, 1, 4), today, Locale::Es), "Ayer");
        assert_eq!(day_label(day(2026, 1, 3), today, Locale::Es), "3 de enero");
        assert_eq!(day_label(day(2025, 12, 31), today, Locale::Es), "31 de diciembre de 2025");
        assert_eq!(day_label(day(2025, 12, 31), today, Locale::En), "December 31, 2025");
    }

    /// Lo que cambia entre dos listas, lo más pequeño posible.
    ///
    /// Roturas que atrapa: rehacer la lista entera por un mensaje que llega
    /// (quien lee arriba pierde su sitio); anteponer una página como si se
    /// añadiera al final; no ver un mensaje retirado en medio; inventarse
    /// un cambio donde no lo hay.
    #[test]
    fn the_smallest_change_between_two_lists() {
        let list = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        let splice = |start, removed, inserted| Some(ThreadSplice { start, removed, inserted });
        assert_eq!(thread_splice(&list(&["a", "b"]), &list(&["a", "b"])), None);
        assert_eq!(thread_splice(&list(&["a", "b"]), &list(&["a", "b", "c"])), splice(2, 0, 1));
        assert_eq!(thread_splice(&list(&["c", "d"]), &list(&["a", "b", "c", "d"])), splice(0, 0, 2));
        assert_eq!(thread_splice(&list(&["a", "b", "c"]), &list(&["a", "c"])), splice(1, 1, 0));
        assert_eq!(thread_splice(&list(&[]), &list(&["a"])), splice(0, 0, 1));
        assert_eq!(thread_splice(&list(&["a", "b"]), &list(&["x", "y", "z"])), splice(0, 2, 3));
        assert_eq!(thread_splice(&list(&["a", "a"]), &list(&["a"])), splice(1, 1, 0));
    }

    const BASE: &str = "https://cac.example";

    /// Las imágenes y los ficheros de un mensaje salen del texto.
    ///
    /// Roturas que atrapa: una imagen de este servidor que se queda en el
    /// markdown (el pintor la pediría sin sesión); una de fuera que se pide
    /// sola; un fichero citado en mitad de una frase que desaparece de
    /// ella; una línea en blanco donde iba la imagen; tocar lo que hay en
    /// un bloque de código; la misma imagen dos veces.
    #[test]
    fn pictures_and_files_come_out_of_the_text() {
        let body = "Mirad esto:\n\n![captura.png](/api/v1/task-spaces/s1/chat/attachments/a1/raw)\n\ny el [plan.pdf](/api/v1/task-spaces/s1/chat/attachments/a2/raw) de ayer\n\n[acta.pdf](https://cac.example/api/v1/dm/c1/attachments/a3/raw?token=secreto)\n![captura.png](/api/v1/task-spaces/s1/chat/attachments/a1/raw)\n![fuera](https://otro.example/pixel.png)\n\n```\n![no](/api/v1/task-spaces/s1/chat/attachments/a9/raw)\n```\ny `![tampoco](/api/v1/task-spaces/s1/chat/attachments/a8/raw)`";
        let parts = message_body(body, BASE);
        let file = |path: &str, name: &str| BodyFile { path: path.into(), name: name.into() };
        assert_eq!(parts.pictures, [file("/api/v1/task-spaces/s1/chat/attachments/a1/raw", "captura.png")]);
        assert_eq!(parts.files, [file("/api/v1/dm/c1/attachments/a3/raw", "acta.pdf")]);
        assert_eq!(
            parts.text,
            "Mirad esto:\n\n\ny el [plan.pdf](/api/v1/task-spaces/s1/chat/attachments/a2/raw) de ayer\n\n[🖼 fuera](https://otro.example/pixel.png)\n\n```\n![no](/api/v1/task-spaces/s1/chat/attachments/a9/raw)\n```\ny `![tampoco](/api/v1/task-spaces/s1/chat/attachments/a8/raw)`"
        );
        assert!(!parts.text.contains("secreto"));
        // Un mensaje que sólo es una captura no deja texto.
        assert_eq!(message_body("![](/api/v1/dm/c1/attachments/a1/raw)", BASE).text, "");
        assert_eq!(message_body("hola **mundo**", BASE), MessageBody { text: "hola **mundo**".into(), ..MessageBody::default() });
    }

    /// Qué dirección es un adjunto de este servidor.
    ///
    /// Roturas que atrapa: pedir con la sesión algo de otro servidor (la
    /// sesión saldría de éste); una ruta que sube con `..`; una ruta de la
    /// API que no es un adjunto tomada por uno.
    #[test]
    fn only_this_servers_attachments_are_fetched_with_the_session() {
        let ours = "/api/v1/task-spaces/s1/chat/attachments/a1/raw";
        assert_eq!(body_attachment(ours, BASE).as_deref(), Some(ours));
        assert_eq!(body_attachment(&format!("{BASE}{ours}?token=x"), BASE).as_deref(), Some(ours));
        assert_eq!(body_attachment("/api/v1/tasks/t1/attachments/a1/raw", BASE).as_deref(), Some("/api/v1/tasks/t1/attachments/a1/raw"));
        for other in [
            "https://otro.example/api/v1/task-spaces/s1/chat/attachments/a1/raw",
            "/api/v1/task-spaces/s1/chat/attachments/../../../auth/me",
            "/api/v1/auth/me",
            "//otro.example/api/v1/x/attachments/a",
            "cacmedia://x/1",
        ] {
            assert_eq!(body_attachment(other, BASE), None, "{other}");
        }
    }

    /// A qué lleva cada enlace de un mensaje.
    ///
    /// Roturas que atrapa: una tarea citada que sale al navegador; un
    /// fichero de la conversación dado al navegador, que no tiene la
    /// sesión; una mención que intenta abrirse; una llamada tomada por un
    /// enlace roto en vez de decir que aún no está aquí; una grabación que
    /// no lleva a su pestaña, o un id que no es uno llevado a ella.
    #[test]
    fn each_link_in_a_message_goes_where_it_points() {
        let link = |href: &str| thread_link(href, BASE);
        assert_eq!(link("/tasks?task=cac-12"), ThreadLink::Task { id: "cac-12".into() });
        assert_eq!(link("/tasks?doc=list:l1&page=p2"), ThreadLink::Doc);
        assert_eq!(link("https://example.com/a"), ThreadLink::Web { url: "https://example.com/a".into() });
        assert_eq!(link("/api/v1/dm/c1/attachments/a1/raw"), ThreadLink::File { path: "/api/v1/dm/c1/attachments/a1/raw".into() });
        assert_eq!(link("cac:user/11111111-1111-4111-8111-111111111111"), ThreadLink::Inert);
        assert_eq!(link("cac:call/0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8"), ThreadLink::NotHere);
        assert_eq!(link("cac:recording/0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8"), ThreadLink::Recording { id: "0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8".into() });
        assert_eq!(link("cac:recording/r1"), ThreadLink::Unknown);
        assert_eq!(link("cac:recording/0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8/../x"), ThreadLink::Unknown);
        assert_eq!(link("#seccion"), ThreadLink::Unknown);
        assert_eq!(link("javascript:alert(1)"), ThreadLink::Unknown);
    }

    /// Quién anda por aquí, en poco sitio.
    ///
    /// Roturas que atrapa: una fila de veinte nombres tapando el del canal;
    /// «+0»; algo pintado cuando no hay nadie.
    #[test]
    fn who_is_around_fits_in_the_header() {
        let names = |count: usize| (1..=count).map(|index| format!("P{index}")).collect::<Vec<_>>();
        assert_eq!(around_label(&names(0)), None);
        assert_eq!(around_label(&names(2)).as_deref(), Some("P1, P2"));
        assert_eq!(around_label(&names(3)).as_deref(), Some("P1, P2, P3"));
        assert_eq!(around_label(&names(5)).as_deref(), Some("P1, P2, P3 +2"));
    }
}
