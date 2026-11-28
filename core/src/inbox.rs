//! La bandeja de la campana: lo que pasó mientras no mirabas.
//!
//! Vino de `app/src/store/inbox.store.ts`, `lib/notification-groups.ts` y
//! `components/NotificationsPanel.tsx`. Es el registro del servidor
//! (`/api/v1/notifications/`), no los avisos del sistema de esta sesión
//! (`notice.rs`): el número de la campana quiere decir «desde que lo leíste»,
//! no «desde que abriste la app».
//!
//! - **Una fila por conversación**, no una por mensaje: diez mensajes de un
//!   canal se pliegan en una con el último y un contador, como un teléfono.
//!   Qué va con qué lo dice el servidor (`groupKey`); lo anterior a esa
//!   columna se deduce del enlace, pero la familia sale siempre de la clase:
//!   `/chat?space=X` es tanto un mensaje del canal como un recordatorio de una
//!   reunión en esa sala.
//! - **Lo leído y lo no leído se separan antes de agrupar**: un grupo con las
//!   dos cosas no se puede colocar.
//! - **La cuenta del servidor manda** sobre lo que llegó: la página trae 50 y
//!   un canal puede tener trescientos.
//! - **Leer no espera al servidor**: nadie quiere esperar para marcar algo
//!   leído. Y una conversación se marca por su clave, no por los ids que se
//!   tienen a mano, por lo mismo que la cuenta.
//! - **En un directo nunca se enseña el texto**: el servidor lo manda vacío a
//!   propósito, y el detalle es un recuento.
//!
//! El servidor contesta de una organización si se le dice cuál (`?orgId=`),
//! y sin ella de todas: por eso no se le pregunta nunca sin ella.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::comms::Conversation;
use crate::counters::{count_badge, rings_bell};
use crate::doc_links::{doc_link, DocLink};
use crate::error::ApiError;
use crate::notice::NoticeTarget;
use crate::stream::ServerEvent;
use crate::texts::{text, text_count, text_with, Locale};
use crate::Core;

/// Cuántos se piden: los que pide Tauri. El servidor no pagina.
const PAGE: usize = 50;

/// Un respiro antes de volver a pedir la bandeja tras un evento.
const COALESCE: Duration = Duration::from_millis(400);

/// Un aviso de la bandeja (`domain.Notification`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct InboxItem {
    pub id: String,
    pub org_id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub link: String,
    pub read_at: Option<String>,
    pub created_at: String,
    /// Quién lo causó: vacío la app, `mcp` un agente. Lo declara el cliente
    /// que escribió: informa, no acredita.
    pub via: Option<String>,
    /// De qué conversación es, para plegarlo con los suyos.
    pub group_key: Option<String>,
    /// Cómo se llama esa conversación: «#portento», «Ana», el título de la
    /// tarea.
    pub group_label: Option<String>,
}

/// Cuántos hay de una conversación en **toda** la bandeja.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct GroupTally {
    pub key: String,
    pub label: String,
    pub total: i64,
    pub unread: i64,
}

/// Lo que contesta el servidor (`domain.NotificationFeed`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct InboxFeed {
    pub items: Vec<InboxItem>,
    pub unread: i64,
    pub groups: Vec<GroupTally>,
}

/// La bandeja de la organización que se mira.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct InboxState {
    /// De qué organización es lo de abajo.
    pub org_id: Option<String>,
    /// `false` mientras no ha llegado nada.
    pub loaded: bool,
    /// La primera lectura de esta organización falló y sigue sin haber nada
    /// que enseñar. Una relectura que falla con la bandeja ya llegada no lo
    /// pone: se queda lo que había.
    pub failed: bool,
    pub feed: InboxFeed,
}

/// Las pestañas del panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum InboxTab {
    #[default]
    All,
    /// «Conversación» y no «Menciones»: aquí caen también los directos y los
    /// mensajes de los canales que sigues, que no te nombran.
    Talk,
    Tasks,
    System,
}

pub const INBOX_TABS: [InboxTab; 4] = [InboxTab::All, InboxTab::Talk, InboxTab::Tasks, InboxTab::System];

impl InboxTab {
    pub fn label(self, locale: Locale) -> String {
        let key = match self {
            InboxTab::All => "notifications:tab.all",
            InboxTab::Talk => "notifications:tab.talk",
            InboxTab::Tasks => "notifications:tab.tasks",
            InboxTab::System => "notifications:tab.system",
        };
        text(locale, key)
    }
}

/// Con qué dibujo se pinta cada clase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum InboxIcon {
    Mention,
    Direct,
    Channel,
    Post,
    Task,
    Assigned,
    Report,
    Review,
    Agent,
    Meeting,
    Ci,
    Deploy,
    Telemetry,
    Backup,
    Unknown,
}

/// De qué color va el dibujo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum InboxTone {
    Primary,
    Muted,
    Foreground,
    Warning,
    Danger,
}

/// Lo que se sabe de cada clase: su pestaña, su etiqueta y su cara
/// (`KINDS` en `NotificationsPanel.tsx`). Lo que no se reconoce va a
/// «Sistema» sin etiqueta: no hay palabra honesta para «no sé qué es esto».
fn kind_of(kind: &str) -> (InboxTab, Option<&'static str>, InboxIcon, InboxTone) {
    use InboxIcon as I;
    use InboxTab as T;
    use InboxTone as C;
    match kind {
        "chat:mention" => (T::Talk, Some("mention"), I::Mention, C::Primary),
        "dm:message" => (T::Talk, Some("direct"), I::Direct, C::Primary),
        "chat:message" => (T::Talk, Some("channel"), I::Channel, C::Muted),
        // Alguien corrigió un mensaje que te avisó (command-and-control-547):
        // de la misma conversación que el original, apagado como lo corriente.
        "chat:edited" => (T::Talk, Some("edited"), I::Channel, C::Muted),
        "dm:edited" => (T::Talk, Some("edited"), I::Direct, C::Muted),
        "post:feedback" => (T::Talk, Some("post"), I::Post, C::Primary),
        "post:comment" => (T::Talk, Some("postComment"), I::Post, C::Muted),
        "post:draft" => (T::Tasks, Some("postDraft"), I::Post, C::Primary),
        "task:comment" => (T::Tasks, Some("comment"), I::Task, C::Foreground),
        "task:assigned" => (T::Tasks, Some("assigned"), I::Assigned, C::Primary),
        "task:status" => (T::Tasks, Some("status"), I::Task, C::Muted),
        "report:new" => (T::System, Some("report"), I::Report, C::Warning),
        "doc:review" => (T::Tasks, Some("review"), I::Review, C::Primary),
        "agent:notice" => (T::Tasks, Some("agent"), I::Agent, C::Primary),
        "request:new" => (T::Tasks, Some("agent"), I::Agent, C::Primary),
        "meeting:reminder" => (T::Talk, Some("meeting"), I::Meeting, C::Primary),
        "ci:run" => (T::System, Some("ci"), I::Ci, C::Muted),
        "deploy:done" => (T::System, Some("deploy"), I::Deploy, C::Primary),
        "telemetry:alert" => (T::System, Some("telemetry"), I::Telemetry, C::Warning),
        "backup:failed" | "backup:stale" | "backup:restore_failed" => (T::System, Some("backup"), I::Backup, C::Danger),
        _ => (T::System, None, I::Unknown, C::Muted),
    }
}

/// Las familias que se pliegan. Sale de la clase, nunca del enlace.
#[derive(PartialEq, Eq)]
enum Family {
    Chat,
    Direct,
    Item,
    Ci,
    /// Los recordatorios de reunión no se pliegan: su enlace es el de la
    /// sala, y agruparlos juntaría la daily y la retro del mismo canal.
    None,
}

fn family_of(kind: &str) -> Family {
    if kind.starts_with("chat:") {
        Family::Chat
    } else if kind.starts_with("dm:") {
        Family::Direct
    } else if kind.starts_with("task:") || kind.starts_with("report:") {
        Family::Item
    } else if kind.starts_with("ci:") || kind.starts_with("deploy:") {
        Family::Ci
    } else {
        Family::None
    }
}

/// El valor de `name` en la consulta de un enlace (`/chat?space=X`).
fn query_param<'a>(link: &'a str, name: &str) -> Option<&'a str> {
    let (_, query) = link.split_once('?')?;
    query.split('&').filter_map(|pair| pair.split_once('=')).find(|(key, _)| *key == name).map(|(_, value)| value).filter(|value| !value.is_empty())
}

/// De qué conversación es un aviso, o `""` si no se puede saber (y entonces
/// va suelto, nunca junto con otros sin clave).
pub fn group_key_of(item: &InboxItem) -> String {
    if let Some(key) = item.group_key.as_deref().filter(|key| !key.is_empty()) {
        return key.to_string();
    }
    let found = match family_of(&item.kind) {
        Family::Chat => query_param(&item.link, "space").map(|id| format!("space:{id}")),
        Family::Direct => query_param(&item.link, "c").map(|id| format!("dm:{id}")),
        // `open` es el formato viejo de la pantalla de reportes.
        Family::Item => query_param(&item.link, "task").or_else(|| query_param(&item.link, "open")).map(|id| format!("item:{id}")),
        Family::Ci | Family::None => None,
    };
    found.unwrap_or_default()
}

/// El nombre de una conversación, del aviso más nuevo (un canal renombrado
/// se cura con el siguiente mensaje). Lo normal es el `groupLabel` del
/// servidor; lo de abajo es para avisos de antes de esa columna.
fn label_of(item: &InboxItem) -> String {
    if let Some(label) = item.group_label.as_deref().filter(|label| !label.is_empty()) {
        return label.to_string();
    }
    match family_of(&item.kind) {
        Family::Chat => item.title.trim_start_matches("Mentioned in ").to_string(),
        Family::Direct => item.title.trim_end_matches(" te escribió").to_string(),
        Family::Item if !item.body.is_empty() => item.body.clone(),
        _ => item.title.clone(),
    }
}

/// A dónde lleva un aviso.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum InboxTarget {
    Conversation { conversation: Conversation },
    Task { task_id: String },
    /// La documentación de un nodo (`/tasks?doc=<clase>:<id>`). Lleva el
    /// enlace entero, porque la página, la pestaña y la sección las lee quien
    /// lo sigue (`Docs::follow_link`), no un segundo parser.
    Doc { node_id: String, route: String },
    /// Un pedido de un agente, en «Por atender» (`/requests?open=req-…`).
    Request { code: String },
    /// Una publicación del muro (`/feedback?post=<id>`).
    Post { post_id: String },
    /// Un sitio que esta app todavía no tiene (los servidores, una llamada), o
    /// ninguno.
    NotHere,
}

/// A dónde lleva un enlace de la bandeja (los que escribe el servidor).
pub fn link_target(link: &str) -> InboxTarget {
    let path = link.split('?').next().unwrap_or_default();
    match path {
        "/chat" => query_param(link, "space").map(|id| InboxTarget::Conversation { conversation: Conversation::Channel { space_id: id.into() } }),
        "/dm" => query_param(link, "c").map(|id| InboxTarget::Conversation { conversation: Conversation::Direct { conversation_id: id.into() } }),
        // El mismo traductor que el resto de enlaces de la app: descodifica,
        // quita el ancla y sólo acepta las clases de nodo que existen.
        "/tasks" => match doc_link(link) {
            DocLink::Task { id } => Some(InboxTarget::Task { task_id: id }),
            DocLink::Doc { id, .. } => Some(InboxTarget::Doc { node_id: id, route: link.into() }),
            _ => None,
        },
        "/requests" => query_param(link, "open").map(|code| InboxTarget::Request { code: code.into() }),
        "/feedback" => query_param(link, "post").map(|id| InboxTarget::Post { post_id: id.into() }),
        _ => None,
    }
    .unwrap_or(InboxTarget::NotHere)
}

/// Cuánto hace, como lo diría alguien: «ahora», «hace 5 minutos», «hace 2
/// horas», «hace 3 días» (`desde` en `app/src/lib/desde.ts`).
pub fn time_ago<Viewer: TimeZone>(iso: &str, now: &DateTime<Viewer>, locale: Locale) -> String {
    let Ok(at) = DateTime::parse_from_rfc3339(iso) else { return text(locale, "common:time.never") };
    let minutes = (now.with_timezone(&Utc) - at.with_timezone(&Utc)).num_minutes();
    // Por debajo de dos minutos la cifra no aporta nada.
    if minutes < 2 {
        return text(locale, "common:time.now");
    }
    if minutes < 60 {
        return text_count(locale, "common:time.minutesAgo", minutes);
    }
    let hours = minutes / 60;
    if hours < 24 {
        return text_count(locale, "common:time.hoursAgo", hours);
    }
    text_count(locale, "common:time.daysAgo", hours / 24)
}

/// Un aviso tal como se pinta.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxRow {
    pub id: String,
    pub title: String,
    pub body: String,
    pub tag: Option<String>,
    pub icon: InboxIcon,
    pub tone: InboxTone,
    pub agent: bool,
    pub when: String,
    pub unread: bool,
    /// A dónde lleva, y de qué organización es.
    pub link: String,
    pub org_id: String,
    /// Tal como llegó: con él se encuentra el mensaje de un directo, cuyo
    /// aviso no dice cuál es (`inbox_peek::row_preview`).
    pub created_at: String,
}

/// Una conversación de la bandeja tal como se pinta: su resumen plegado y,
/// si está abierta, sus avisos.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxGroup {
    /// La clave, o el id del aviso si va suelto.
    pub key: String,
    /// Un aviso solo: se pinta como una fila, sin contador ni flecha.
    pub alone: bool,
    pub title: String,
    pub detail: String,
    /// Cuántos: lo que cuenta el servidor si lo sabe.
    pub count: i64,
    pub tag: Option<String>,
    /// El de la familia, salvo que dentro haya una mención: eso es lo que
    /// decide si hay que abrirlo ya.
    pub icon: InboxIcon,
    pub tone: InboxTone,
    pub agent: bool,
    pub when: String,
    pub unread: bool,
    pub expanded: bool,
    /// Los avisos de dentro, si está abierta (o el único, si va solo).
    pub rows: Vec<InboxRow>,
    /// «Se ven 12 de 47»: si el servidor tiene más de los que llegaron.
    pub showing: Option<String>,
    /// A dónde lleva pulsarla: donde ocurrió lo más reciente.
    pub link: String,
    pub org_id: String,
}

/// El panel entero.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxPanel {
    /// El número junto al título; nada si no hay nada sin leer.
    pub badge: Option<String>,
    pub can_mark_all: bool,
    pub unread: Vec<InboxGroup>,
    pub read: Vec<InboxGroup>,
    /// Sólo si ya llegó la bandeja y no hay nada: antes de eso no se sabe.
    pub empty: bool,
    /// Todavía no ha llegado nada y no ha fallado.
    pub loading: bool,
    /// No ha llegado nada y falló: se enseña el error, no «Nada pendiente».
    pub failed: bool,
}

fn row_of<Viewer: TimeZone>(item: &InboxItem, now: &DateTime<Viewer>, locale: Locale) -> InboxRow {
    let (_, tag, icon, tone) = kind_of(&item.kind);
    InboxRow {
        id: item.id.clone(),
        title: item.title.clone(),
        body: item.body.clone(),
        tag: tag.map(|tag| text(locale, &format!("notifications:kind.{tag}"))),
        icon,
        tone,
        agent: item.via.as_deref() == Some("mcp"),
        when: time_ago(&item.created_at, now, locale),
        unread: item.read_at.is_none(),
        link: item.link.clone(),
        org_id: item.org_id.clone(),
        created_at: item.created_at.clone(),
    }
}

/// Pliega una lista ya separada en leídos o no leídos. El orden entre grupos
/// es el del aviso más nuevo de cada uno, nunca el tamaño: un canal charlatán
/// y viejo se quedaría arriba para siempre.
fn groups_of<Viewer: TimeZone>(items: &[&InboxItem], state: &InboxState, read: bool, expanded: &HashSet<String>, locale: Locale, now: &DateTime<Viewer>) -> Vec<InboxGroup> {
    // Sin clave, cada uno va suelto con su propio id: dos avisos inconexos
    // no acaban en el mismo saco.
    let mut keyed: Vec<(String, Vec<&InboxItem>)> = Vec::new();
    let mut loose: Vec<(String, Vec<&InboxItem>)> = Vec::new();
    for item in items {
        let key = group_key_of(item);
        if key.is_empty() {
            loose.push((item.id.clone(), vec![item]));
            continue;
        }
        match keyed.iter_mut().find(|(other, _)| *other == key) {
            Some((_, members)) => members.push(item),
            None => keyed.push((key, vec![item])),
        }
    }
    keyed.extend(loose);
    let newest = |members: &[&InboxItem]| members.iter().map(|item| item.created_at.clone()).max().unwrap_or_default();
    keyed.sort_by(|(_, a), (_, b)| newest(b).cmp(&newest(a)));
    keyed
        .into_iter()
        .map(|(key, members)| {
            let newest = *members.iter().max_by(|a, b| a.created_at.cmp(&b.created_at)).expect("un grupo tiene avisos");
            let alone = members.len() == 1;
            let (_, tag, icon, tone) = kind_of(&members[0].kind);
            let mention = members.iter().any(|item| item.kind == "chat:mention");
            let tally = state.feed.groups.iter().find(|tally| tally.key == key);
            let loaded = members.len() as i64;
            let count = if alone { 0 } else if read { tally.map_or(loaded, |tally| tally.total) } else { tally.map_or(loaded, |tally| tally.unread) };
            let detail = match family_of(&newest.kind) {
                Family::Direct => text_count(locale, "notifications:directCount", loaded),
                Family::Item | Family::Ci => newest.title.clone(),
                _ => newest.body.clone(),
            };
            let is_expanded = !alone && expanded.contains(&key);
            let hidden = tally.map_or(0, |tally| tally.total - loaded);
            InboxGroup {
                alone,
                title: if alone { newest.title.clone() } else { label_of(newest) },
                detail: if alone { newest.body.clone() } else { detail },
                count,
                tag: tag.map(|tag| text(locale, &format!("notifications:kind.{tag}"))),
                icon: if mention { InboxIcon::Mention } else { icon },
                tone: if mention { InboxTone::Primary } else { tone },
                agent: members.iter().any(|item| item.via.as_deref() == Some("mcp")),
                when: time_ago(&newest.created_at, now, locale),
                unread: !read,
                expanded: is_expanded,
                rows: if alone || is_expanded { members.iter().map(|item| row_of(item, now, locale)).collect() } else { Vec::new() },
                showing: (is_expanded && hidden > 0).then(|| {
                    let total = tally.map_or(loaded, |tally| tally.total).to_string();
                    text_with(locale, "notifications:showing", &[("shown", &loaded.to_string()), ("total", &total)])
                }),
                link: newest.link.clone(),
                org_id: newest.org_id.clone(),
                key,
            }
        })
        .collect()
}

/// El panel de la campana, con la pestaña y los grupos abiertos que diga
/// quien lo pinta. Lo que se tiene abierto va por clave, y no por posición:
/// la bandeja se vuelve a pedir con cada evento, y cualquier otra llave
/// cerraría el grupo que se acaba de abrir.
pub fn inbox_panel<Viewer: TimeZone>(state: &InboxState, tab: InboxTab, expanded: &HashSet<String>, locale: Locale, now: &DateTime<Viewer>) -> InboxPanel {
    let mine: Vec<&InboxItem> = state.feed.items.iter().filter(|item| tab == InboxTab::All || kind_of(&item.kind).0 == tab).collect();
    let (read, unread): (Vec<&InboxItem>, Vec<&InboxItem>) = mine.into_iter().partition(|item| item.read_at.is_some());
    let unread = groups_of(&unread, state, false, expanded, locale, now);
    let read = groups_of(&read, state, true, expanded, locale, now);
    InboxPanel {
        badge: count_badge(Some(state.feed.unread), 99),
        can_mark_all: state.feed.unread > 0,
        empty: state.loaded && unread.is_empty() && read.is_empty(),
        loading: !state.loaded && !state.failed,
        failed: !state.loaded && state.failed,
        unread,
        read,
    }
}

/// La bandeja de este core.
pub struct Inbox {
    pub(crate) core: Core,
}

impl Core {
    pub fn inbox(&self) -> Inbox {
        Inbox { core: self.clone() }
    }
}

/// Lo que hay detrás de la bandeja.
#[derive(Default)]
pub(crate) struct InboxData {
    state: InboxState,
    /// Si se ha abierto alguna vez: hasta entonces no se pide (la campana ya
    /// tiene su número por `counters`).
    wanted: bool,
}

impl Inbox {
    fn data(&self) -> std::sync::MutexGuard<'_, InboxData> {
        self.core.inner.inbox.lock().expect("inbox lock")
    }

    pub fn state(&self) -> InboxState {
        let current = self.core.orgs().state().current_org_id;
        let state = self.data().state.clone();
        // La de otra organización no se pinta.
        if state.org_id == current {
            state
        } else {
            InboxState { org_id: current, ..InboxState::default() }
        }
    }

    fn change(&self, edit: impl FnOnce(&mut InboxState)) {
        let state = {
            let mut data = self.data();
            edit(&mut data.state);
            data.state.clone()
        };
        if let Some(org_id) = state.org_id.as_deref() {
            self.core.counters().set_notifications(org_id, state.feed.unread);
        }
        self.core.inner.listener.inbox_changed(self.state());
    }

    /// Trae la bandeja de la organización que se mira. Si falla se queda lo
    /// que hubiera: una bandeja que no carga es un número que no cambia, no
    /// algo por lo que interrumpir a nadie. Salvo que no hubiera nada: entonces
    /// el panel tiene que poder decir que no lo sabe, y no «Nada pendiente».
    pub async fn load(&self) {
        self.data().wanted = true;
        let Some(org_id) = self.core.orgs().state().current_org_id.filter(|id| !id.is_empty()) else { return };
        let path = format!("/api/v1/notifications/?orgId={org_id}&limit={PAGE}");
        let answer = self.core.get::<InboxFeed>(&path).await;
        // La respuesta de una organización que ya no es la que se mira no se
        // pone.
        if self.core.orgs().state().current_org_id.as_deref() != Some(org_id.as_str()) {
            return;
        }
        match answer {
            Ok(feed) => self.change(|state| *state = InboxState { org_id: Some(org_id), loaded: true, failed: false, feed }),
            Err(_) => self.change(|state| {
                if state.org_id.as_deref() != Some(org_id.as_str()) {
                    *state = InboxState { org_id: Some(org_id), ..InboxState::default() };
                }
                state.failed = !state.loaded;
            }),
        }
    }

    /// Unos avisos leídos. El número baja por los que de verdad no lo
    /// estaban: un doble clic no lo lleva por debajo de cero.
    pub async fn mark_read(&self, ids: Vec<String>) -> Result<(), ApiError> {
        if ids.is_empty() {
            return Ok(());
        }
        self.change(|state| {
            let mut fresh = 0;
            for item in state.feed.items.iter_mut().filter(|item| ids.contains(&item.id) && item.read_at.is_none()) {
                item.read_at = Some(Utc::now().to_rfc3339());
                fresh += 1;
            }
            state.feed.unread = (state.feed.unread - fresh).max(0);
        });
        self.core.call_ok("POST", "/api/v1/notifications/read", Some(serde_json::json!({ "ids": ids }))).await
    }

    /// Toda una conversación de una vez, por su clave: la cuenta del servidor
    /// sabe cuántos hay, aunque aquí sólo hayan llegado unos pocos.
    pub async fn mark_group_read(&self, key: String) -> Result<(), ApiError> {
        let mut org = String::new();
        self.change(|state| {
            org = state.org_id.clone().unwrap_or_default();
            let mut fresh = 0;
            for item in state.feed.items.iter_mut().filter(|item| group_key_of(item) == key && item.read_at.is_none()) {
                item.read_at = Some(Utc::now().to_rfc3339());
                fresh += 1;
            }
            let tallied = state.feed.groups.iter().find(|tally| tally.key == key).map_or(fresh, |tally| tally.unread);
            state.feed.unread = (state.feed.unread - tallied).max(0);
            for tally in state.feed.groups.iter_mut().filter(|tally| tally.key == key) {
                tally.unread = 0;
            }
        });
        self.core.call_ok("POST", "/api/v1/notifications/read", Some(serde_json::json!({ "group": key, "orgId": org }))).await
    }

    /// Todo leído, en la organización que se mira.
    pub async fn mark_all_read(&self) -> Result<(), ApiError> {
        let Some(org_id) = self.state().org_id else { return Ok(()) };
        self.change(|state| {
            for item in state.feed.items.iter_mut().filter(|item| item.read_at.is_none()) {
                item.read_at = Some(Utc::now().to_rfc3339());
            }
            state.feed.unread = 0;
            for tally in state.feed.groups.iter_mut() {
                tally.unread = 0;
            }
        });
        let result = self.core.call_ok("POST", &format!("/api/v1/notifications/read-all?orgId={org_id}"), Some(serde_json::json!({}))).await;
        // El marcado ya se pintó: si el servidor no lo aceptó, el número y las
        // filas vuelven a lo que él dice.
        if result.is_err() {
            self.load().await;
        }
        result
    }

    /// Lleva a donde dice un aviso: a su organización si es otra, y a su
    /// conversación, su tarea o su documentación. Devuelve si había a dónde
    /// ir en esta app.
    pub async fn open(&self, link: &str, org_id: &str) -> bool {
        let target = link_target(link);
        if target == InboxTarget::NotHere {
            return false;
        }
        // Los pedidos no son de una organización: no se cambia de ninguna.
        if let InboxTarget::Request { code } = target {
            if !self.core.requests().show(Some(code)) {
                return false;
            }
            self.core.requests().load().await;
            return true;
        }
        // Las publicaciones son de la persona, no de una organización.
        if let InboxTarget::Post { post_id } = target {
            return self.core.wall().open_from_link(&post_id).await;
        }
        if let InboxTarget::Conversation { conversation } = target {
            return self.core.notices().open(NoticeTarget { conversation, org_id: Some(org_id.to_string()) }).await;
        }
        if self.core.orgs().state().current_org_id.as_deref() != Some(org_id) && !self.core.switch_org(org_id).await {
            return false;
        }
        // Una tarea o un documento es de «Trabajo»: el resumen y «Por atender»
        // dejan de estar delante, que si no seguirían encendidos debajo.
        if matches!(target, InboxTarget::Task { .. } | InboxTarget::Doc { .. }) {
            self.core.overview().leave();
            self.core.requests().leave();
        }
        match target {
            InboxTarget::Task { task_id } => self.core.tasks().open(&task_id).await,
            InboxTarget::Doc { node_id, route } => {
                self.core.comms().leave();
                self.core.docs().follow_link(&route).await;
                let docs = self.core.docs().state();
                return docs.link_failed.is_none() && docs.target.is_some_and(|target| target.id == node_id);
            }
            _ => {}
        }
        true
    }

    /// Vuelve a pedirla tras un respiro, y sólo la última vez que se pidió.
    fn reload_soon(&self) {
        let turn = self.core.inner.inbox_reloads.fetch_add(1, Ordering::SeqCst) + 1;
        let inbox = self.core.inbox();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            if inbox.core.inner.inbox_reloads.load(Ordering::SeqCst) != turn {
                return;
            }
            inbox.load().await;
        });
    }

    /// Un evento que deja fila en la campana: si alguna vez se abrió, se
    /// vuelve a pedir.
    pub(crate) fn server_event(&self, event: &ServerEvent) {
        if self.data().wanted && rings_bell(&event.event, &event.data) {
            self.reload_soon();
        }
    }

    /// Al volver de una caída o al cambiar de organización, si se quiere.
    pub(crate) async fn refresh(&self) {
        if self.data().wanted {
            self.load().await;
        }
    }

    /// Al salir: la bandeja de una persona no es de la siguiente.
    pub(crate) fn clear(&self) {
        *self.data() = InboxData::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: &str, at: &str, read: bool) -> InboxItem {
        InboxItem {
            id: id.into(),
            org_id: "o1".into(),
            kind: kind.into(),
            title: format!("t-{id}"),
            body: format!("b-{id}"),
            link: String::new(),
            read_at: read.then(|| "2026-11-18T10:00:00Z".into()),
            created_at: at.into(),
            ..Default::default()
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-18T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    /// Qué conversación es cada aviso. Roturas que atrapa: sacar la familia
    /// del enlace (un recordatorio de reunión en la sala se plegaría con sus
    /// mensajes); juntar dos avisos sin clave; no entender el formato viejo
    /// de reportes.
    #[test]
    fn what_goes_with_what() {
        let with = |kind: &str, link: &str| InboxItem { kind: kind.into(), link: link.into(), ..Default::default() };
        assert_eq!(group_key_of(&InboxItem { group_key: Some("chat:x".into()), ..with("chat:message", "/chat?space=y") }), "chat:x", "manda la del servidor");
        assert_eq!(group_key_of(&with("chat:message", "/chat?space=s1")), "space:s1");
        assert_eq!(group_key_of(&with("dm:message", "/dm?c=c1")), "dm:c1");
        assert_eq!(group_key_of(&with("task:comment", "/tasks?task=t1")), "item:t1");
        assert_eq!(group_key_of(&with("report:new", "/reports?open=r1")), "item:r1");
        assert_eq!(group_key_of(&with("meeting:reminder", "/chat?space=s1")), "", "una reunión no se pliega con su sala");
        assert_eq!(group_key_of(&with("chat:message", "/chat")), "");
    }

    /// El panel. Roturas que atrapa: mezclar leídos y no leídos en un grupo;
    /// ordenar por tamaño o por el aviso más viejo; un contador en un aviso
    /// suelto; contar lo cargado y no lo del servidor; enseñar el texto de un
    /// directo; el detalle de una tarea en el sitio del de un canal; una
    /// mención que no cambia el dibujo; filtrar mal una pestaña.
    #[test]
    fn the_panel_folds_by_conversation() {
        let mut items = vec![
            InboxItem { group_key: Some("chat:a".into()), group_label: Some("#producto".into()), ..item("1", "chat:message", "2026-11-18T09:00:00Z", false) },
            InboxItem { group_key: Some("chat:a".into()), group_label: Some("#producto".into()), ..item("2", "chat:mention", "2026-11-18T11:00:00Z", false) },
            InboxItem { group_key: Some("dm:c".into()), group_label: Some("Bea".into()), body: String::new(), ..item("3", "dm:message", "2026-11-18T10:00:00Z", false) },
            InboxItem { group_key: Some("dm:c".into()), group_label: Some("Bea".into()), body: String::new(), ..item("4", "dm:message", "2026-11-18T10:30:00Z", false) },
            InboxItem { group_key: Some("item:t".into()), group_label: Some("Arreglar login".into()), ..item("5", "task:comment", "2026-11-18T11:30:00Z", false) },
            InboxItem { group_key: Some("item:t".into()), group_label: Some("Arreglar login".into()), ..item("6", "task:comment", "2026-11-18T08:00:00Z", true) },
            item("7", "ci:run", "2026-11-18T11:58:00Z", false),
            item("8", "algo:nuevo", "2026-11-17T08:00:00Z", true),
        ];
        items[4].title = "Bea respondió".into();
        let state = InboxState {
            org_id: Some("o1".into()),
            loaded: true,
            failed: false,
            feed: InboxFeed {
                items,
                unread: 60,
                groups: vec![GroupTally { key: "chat:a".into(), label: "#producto".into(), total: 52, unread: 50 }],
            },
        };
        let panel = inbox_panel(&state, InboxTab::All, &HashSet::new(), Locale::Es, &now());
        let keys = |groups: &[InboxGroup]| groups.iter().map(|group| group.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&panel.unread), ["7", "item:t", "chat:a", "dm:c"], "el más nuevo de cada uno manda el orden");
        assert_eq!(keys(&panel.read), ["item:t", "8"], "lo leído de la tarea va aparte");

        let chat = &panel.unread[2];
        assert_eq!((chat.title.as_str(), chat.detail.as_str(), chat.count), ("#producto", "b-2", 50), "la cuenta del servidor");
        assert_eq!((chat.icon, chat.tone), (InboxIcon::Mention, InboxTone::Primary), "con una mención dentro, se ve");
        let dm = &panel.unread[3];
        assert_eq!((dm.title.as_str(), dm.detail.as_str(), dm.count), ("Bea", "2 mensajes nuevos", 2), "un directo no enseña texto");
        let task = &panel.unread[1];
        assert!(task.alone, "en lo no leído la tarea es un aviso solo");
        assert_eq!((task.title.as_str(), task.count, task.rows.len()), ("Bea respondió", 0, 1), "sin contador, como una fila");
        assert_eq!(panel.unread[0].when, "hace 2 minutos");
        assert_eq!(panel.read[1].tag, None, "lo desconocido va sin etiqueta");
        assert_eq!((panel.badge.as_deref(), panel.can_mark_all, panel.empty), (Some("60"), true, false));

        let talk = inbox_panel(&state, InboxTab::Talk, &HashSet::new(), Locale::Es, &now());
        assert_eq!(keys(&talk.unread), ["chat:a", "dm:c"]);
        assert!(talk.read.is_empty());
        let system = inbox_panel(&state, InboxTab::System, &HashSet::new(), Locale::Es, &now());
        assert_eq!((keys(&system.unread), keys(&system.read)), (vec!["7".to_string()], vec!["8".to_string()]), "lo desconocido cae en sistema");

        // Abierta, con lo que el servidor tiene de más.
        let open = inbox_panel(&state, InboxTab::All, &HashSet::from(["chat:a".to_string()]), Locale::Es, &now());
        let chat = &open.unread[2];
        assert_eq!(chat.rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["1", "2"]);
        assert_eq!(chat.showing.as_deref(), Some("Se ven 2 de 52. El resto sigue en la conversación."));
        assert!(open.unread[3].rows.is_empty(), "las demás siguen plegadas");
    }

    /// Antes de que llegue la bandeja el panel no sabe, y no dice «Nada
    /// pendiente»; si la lectura falló, lo dice; con la bandeja llegada y
    /// vacía, sí está vacío.
    ///
    /// Roturas que atrapa: `empty` sin mirar `loaded` (un falso «Nada
    /// pendiente» mientras carga o tras un fallo); `loading` que sigue
    /// encendido tras un fallo; `failed` que se enciende con la bandeja ya
    /// llegada.
    #[test]
    fn the_panel_tells_loading_failed_and_empty_apart() {
        let panel = |loaded, failed| inbox_panel(&InboxState { org_id: Some("o1".into()), loaded, failed, feed: InboxFeed::default() }, InboxTab::All, &HashSet::new(), Locale::Es, &now());
        let flags = |panel: InboxPanel| (panel.loading, panel.failed, panel.empty);
        assert_eq!(flags(panel(false, false)), (true, false, false), "cargando: no se sabe");
        assert_eq!(flags(panel(false, true)), (false, true, false), "falló: no es «nada pendiente»");
        assert_eq!(flags(panel(true, false)), (false, false, true), "llegó y está vacía");
        assert_eq!(flags(panel(true, true)), (false, false, true), "una relectura fallida no tapa lo que llegó");
    }

    /// «Ana editó su mensaje» (command-and-control-547). Roturas que atrapa:
    /// que caiga en «Sistema» sin etiqueta, como una clase desconocida; que no
    /// se pliegue con su canal o su directo; o que no lleve a la conversación.
    #[test]
    fn an_edited_message_reads_as_talk_and_leads_to_its_conversation() {
        let edited = |kind: &str, link: &str| InboxItem { link: link.into(), ..item("e", kind, "2026-11-18T11:00:00Z", false) };
        let channel = edited("chat:edited", "/chat?space=s1");
        let direct = edited("dm:edited", "/dm?c=c1");
        for (it, icon) in [(&channel, InboxIcon::Channel), (&direct, InboxIcon::Direct)] {
            let (tab, _, _, _) = kind_of(&it.kind);
            assert_eq!(tab, InboxTab::Talk, "{}", it.kind);
            let row = row_of(it, &now(), Locale::Es);
            assert_eq!((row.tag.as_deref(), row.icon), (Some("editado"), icon), "{}", it.kind);
            assert_eq!(row_of(it, &now(), Locale::En).tag.as_deref(), Some("edited"));
        }
        assert_eq!(group_key_of(&channel), "space:s1");
        assert_eq!(group_key_of(&direct), "dm:c1");
        assert_eq!(link_target(&channel.link), InboxTarget::Conversation { conversation: Conversation::Channel { space_id: "s1".into() } });
        assert_eq!(link_target(&direct.link), InboxTarget::Conversation { conversation: Conversation::Direct { conversation_id: "c1".into() } });
    }

    /// A dónde lleva cada enlace. Roturas que atrapa: confundir un canal con
    /// un directo; no entender la documentación; llevar a un sitio que esta
    /// app no tiene.
    #[test]
    fn where_a_link_goes() {
        assert_eq!(link_target("/chat?space=s1"), InboxTarget::Conversation { conversation: Conversation::Channel { space_id: "s1".into() } });
        assert_eq!(link_target("/dm?c=c1"), InboxTarget::Conversation { conversation: Conversation::Direct { conversation_id: "c1".into() } });
        assert_eq!(link_target("/tasks?task=t1"), InboxTarget::Task { task_id: "t1".into() });
        assert_eq!(link_target("/tasks?doc=list:l1"), InboxTarget::Doc { node_id: "l1".into(), route: "/tasks?doc=list:l1".into() });
        assert_eq!(link_target("/requests?open=req-ab12"), InboxTarget::Request { code: "req-ab12".into() });
        assert_eq!(link_target("/feedback?post=p1"), InboxTarget::Post { post_id: "p1".into() });
        for elsewhere in ["/feedback", "/servers/x/backups?target=b", "/call/abc", "", "/chat"] {
            assert_eq!(link_target(elsewhere), InboxTarget::NotHere, "{elsewhere}");
        }
    }

    /// Cuánto hace. Roturas que atrapa: «hace 0 minutos»; el plural mal;
    /// pasar de minutos a horas o a días tarde o pronto.
    #[test]
    fn how_long_ago() {
        let ago = |iso: &str| time_ago(iso, &now(), Locale::Es);
        assert_eq!(ago("2026-11-18T11:59:00Z"), "ahora");
        assert_eq!(ago("2026-11-18T11:58:00Z"), "hace 2 minutos");
        assert_eq!(ago("2026-11-18T11:01:00Z"), "hace 59 minutos");
        assert_eq!(ago("2026-11-18T11:00:00Z"), "hace 1 hora");
        assert_eq!(ago("2026-11-17T12:00:00Z"), "hace 1 día");
        assert_eq!(ago("2026-11-15T12:00:00Z"), "hace 3 días");
        assert_eq!(time_ago("2026-11-18T09:00:00Z", &now(), Locale::En), "3 hours ago");
        assert_eq!(ago("no es una fecha"), "nunca");
    }
}
