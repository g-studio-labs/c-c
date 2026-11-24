//! Las pestañas de un canal además de la conversación: Multimedia,
//! Grabaciones y Enlaces; buscar dentro de él; y las llamadas con gente de
//! fuera que cuelgan de él.
//!
//! Vino de `ChannelView.tsx`, `tab-page.ts`, `ThreadSearch.tsx`,
//! `recordings.store.ts`, `calls.store.ts` y `call-link.ts`. Qué se pide, en
//! qué orden se pegan las páginas y qué enseña una pestaña sin filas lo
//! decide esto; la pantalla sólo pinta.

use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::chat::query_escaped;
use crate::comms_text::instant;
use crate::{date_and_time, short_date, ApiError, ChatMessage, Core, Locale};

/// Cuántos aciertos trae cada página de «Buscar en #canal» (`PAGE` en
/// `ThreadSearch.tsx`).
pub const SEARCH_PAGE: usize = 30;

/// Lo más largo que puede ser el título de una llamada con invitados
/// (`max=120` en `CreateCallInviteRequest`).
pub const CALL_TITLE_MAX: usize = 120;

/// Cuánto puede valer un enlace de invitado, en horas, y cómo se dice: las
/// tres de `GuestInviteDialog.tsx`. La de en medio es la que viene puesta.
pub const GUEST_TTLS: [(u32, &str); 3] = [(1, "calls:ttl1"), (24, "calls:ttl24"), (168, "calls:ttl168")];

/// Una pestaña de un canal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ChannelTab {
    Thread,
    Media,
    Recordings,
    Links,
}

impl ChannelTab {
    /// Su id en pantalla y en las pruebas.
    pub fn id(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Media => "media",
            Self::Recordings => "recordings",
            Self::Links => "links",
        }
    }

    /// Su nombre, como clave de los catálogos.
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Thread => "chat:tabConversation",
            Self::Media => "chat:tabMedia",
            Self::Recordings => "recordings:panelTitle",
            Self::Links => "chat:tabLinks",
        }
    }

    /// Si se ofrece buscar en ella. Las grabaciones no: son cuatro filas con
    /// fecha y quién grabó.
    pub fn searchable(self) -> bool {
        self != Self::Recordings
    }
}

/// Las pestañas de un canal, en su orden. Grabaciones sólo si el servidor
/// graba: una instalación sin grabación montada no enseña una lista que
/// siempre va a estar vacía.
pub fn channel_tabs(recordings_enabled: bool) -> Vec<ChannelTab> {
    let mut tabs = vec![ChannelTab::Thread, ChannelTab::Media];
    if recordings_enabled {
        tabs.push(ChannelTab::Recordings);
    }
    tabs.push(ChannelTab::Links);
    tabs
}

// ── Lo que viaja ────────────────────────────────────────────────────────

/// Un fichero que enseña un mensaje vivo del canal. Espejo de
/// `domain.ChatMediaItem`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ChatMediaItem {
    pub id: String,
    pub url: String,
    pub file_name: String,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub bytes: Option<i64>,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub posted_at: String,
    #[serde(default)]
    pub author_name: Option<String>,
}

impl ChatMediaItem {
    /// Si se puede enseñar como imagen. Lo demás va con un clip: pintar un
    /// PDF como imagen da un hueco roto, que se lee como «esto se perdió».
    pub fn is_image(&self) -> bool {
        self.content_type.as_deref().is_some_and(|kind| kind.starts_with("image/"))
    }
}

/// Un enlace que se pegó en el canal, con las palabras que le pusieron.
/// Espejo de `domain.ChatLinkItem`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ChatLinkItem {
    pub url: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub posted_at: String,
    #[serde(default)]
    pub author_name: Option<String>,
}

/// Una página de Multimedia o de Enlaces, y por dónde seguir. Espejo de
/// `domain.ChatMediaPage` / `ChatLinkPage`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "T: Deserialize<'de>"))]
pub struct TabPage<T> {
    #[serde(default)]
    pub items: Vec<T>,
    #[serde(default)]
    pub before: Option<String>,
}

/// Una grabación de una llamada del canal. Espejo de
/// `domain.RecordingResponse`, con lo que la lista usa.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub id: String,
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub started_by: String,
    #[serde(default)]
    pub started_by_name: Option<String>,
    /// `recording`, `finalizing`, `ready`, `partial` o `failed`.
    pub status: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    /// `video/mp4` o `audio/mp4`.
    #[serde(default)]
    pub final_content_type: Option<String>,
    #[serde(default)]
    pub final_bytes: Option<i64>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub has_screen: bool,
}

impl Recording {
    /// Si hay fichero que ver. `partial` también: hay vídeo, y esconderlo
    /// por haber perdido una pista sería tirar lo que sí se salvó.
    pub fn playable(&self) -> bool {
        matches!(self.status.as_str(), "ready" | "partial")
    }

    pub fn failed(&self) -> bool {
        self.status == "failed"
    }

    /// Cómo se dice su estado, como clave de los catálogos.
    pub fn status_key(&self) -> &'static str {
        match self.status.as_str() {
            "recording" => "recordings:statusRecording",
            "finalizing" => "recordings:statusFinalizing",
            "ready" => "recordings:statusReady",
            "partial" => "recordings:statusPartial",
            _ => "recordings:statusFailed",
        }
    }

    fn is_audio(&self) -> bool {
        self.final_content_type.as_deref().is_some_and(|kind| kind.starts_with("audio/"))
    }
}

/// Lo que se pregunta antes de enseñar Grabaciones. Espejo de
/// `domain.RecordingPolicy`, sin la activa, que esta lista no usa.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Alguien dentro de una llamada.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CallOccupant {
    pub identity: String,
    #[serde(default)]
    pub name: String,
}

/// Una llamada con invitados, para un miembro. Espejo de
/// `domain.CallInviteResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct CallInvite {
    pub id: String,
    #[serde(default)]
    pub org_id: String,
    #[serde(default)]
    pub space_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub expires_at: String,
    /// El token del enlace. Sólo lo ve un miembro.
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub occupants: Vec<CallOccupant>,
}

// ── Una pestaña paginada ────────────────────────────────────────────────

/// Con qué se reconoce una fila al pegar páginas.
pub trait TabItem: Clone {
    fn key(&self) -> &str;
}

impl TabItem for ChatMediaItem {
    fn key(&self) -> &str {
        &self.id
    }
}

impl TabItem for ChatLinkItem {
    fn key(&self) -> &str {
        &self.url
    }
}

impl TabItem for ChatMessage {
    fn key(&self) -> &str {
        &self.id
    }
}

/// Qué enseña una pestaña.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabShows {
    /// No se pudo leer. Va **antes** que todo: una pestaña que no pudo leer
    /// y dice «todavía no hay nada» miente, e invita a no volver a mirar.
    Failed,
    Loading,
    Empty,
    Rows,
}

/// Una pestaña que se lee por páginas: Multimedia, Enlaces o los aciertos
/// de una búsqueda (`useTabPage`).
///
/// - **El cursor es el que manda el servidor**, no la fecha de la última
///   fila: una página es una ventana de canal y puede volver corta, o vacía,
///   con historial detrás.
/// - **Se deduplica al pegar**: el mismo enlace pegado dos veces con semanas
///   de diferencia cae en dos ventanas.
/// - **Cada lectura lleva su turno**: lo que llega de una búsqueda que ya se
///   cambió, o de otro canal, no se pega.
#[derive(Debug, Clone)]
pub struct PagedTab<T> {
    rows: Vec<T>,
    before: Option<String>,
    loading: bool,
    failed: bool,
    turn: u64,
}

impl<T> Default for PagedTab<T> {
    fn default() -> Self {
        Self { rows: Vec::new(), before: None, loading: false, failed: false, turn: 0 }
    }
}

impl<T: TabItem> PagedTab<T> {
    /// Empieza de cero —otro canal, otra búsqueda— y da el turno de la
    /// primera página.
    pub fn restart(&mut self) -> u64 {
        self.turn += 1;
        self.rows.clear();
        self.before = None;
        self.loading = true;
        self.failed = false;
        self.turn
    }

    /// El turno y el cursor de la página siguiente, si queda canal detrás y
    /// no hay ya una de camino.
    pub fn next(&mut self) -> Option<(u64, String)> {
        if self.loading || self.failed {
            return None;
        }
        let before = self.before.clone()?;
        self.turn += 1;
        self.loading = true;
        Some((self.turn, before))
    }

    /// Lo que contestó el servidor a la lectura de ese turno. Devuelve si se
    /// tuvo en cuenta.
    pub fn landed(&mut self, turn: u64, answer: Result<TabPage<T>, ApiError>) -> bool {
        if turn != self.turn {
            return false;
        }
        self.loading = false;
        match answer {
            Ok(page) => {
                for row in page.items {
                    if !self.rows.iter().any(|held| held.key() == row.key()) {
                        self.rows.push(row);
                    }
                }
                // Sin cursor de vuelta no queda canal detrás: es la única
                // señal de final, porque una página vacía no lo es.
                self.before = page.before;
                self.failed = false;
            }
            Err(_) => {
                self.before = None;
                self.failed = true;
            }
        }
        true
    }

    pub fn rows(&self) -> &[T] {
        &self.rows
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    /// Si se ofrece «Cargar lo anterior».
    pub fn has_more(&self) -> bool {
        self.before.is_some() && !self.failed
    }

    pub fn shows(&self) -> TabShows {
        if !self.rows.is_empty() {
            TabShows::Rows
        } else if self.failed {
            TabShows::Failed
        } else if self.loading {
            TabShows::Loading
        } else {
            TabShows::Empty
        }
    }
}

// ── Rutas y nombres ─────────────────────────────────────────────────────

/// Lo buscado, tal como viaja: sin los espacios de los bordes.
fn query_part(query: &str) -> Option<String> {
    let query = query.trim();
    (!query.is_empty()).then(|| query_escaped(query))
}

/// La ruta de una página de Multimedia (`media`) o de Enlaces (`links`). Lo
/// buscado viaja al servidor: filtrar lo cargado sólo miraría la última
/// ventana, y una búsqueda que no encuentra dice «no está».
pub fn tab_path(space_id: &str, which: &str, query: &str, before: Option<&str>) -> String {
    let mut params = Vec::new();
    if let Some(before) = before {
        params.push(format!("before={}", query_escaped(before)));
    }
    if let Some(query) = query_part(query) {
        params.push(format!("q={query}"));
    }
    let mut path = format!("/api/v1/task-spaces/{space_id}/chat/{which}");
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    path
}

/// La ruta de una página de aciertos de «Buscar en #canal».
pub fn search_path(space_id: &str, query: &str, before: Option<&str>) -> String {
    let mut path = format!("/api/v1/task-spaces/{space_id}/chat?q={}&limit={SEARCH_PAGE}", query_part(query).unwrap_or_default());
    if let Some(before) = before {
        path.push_str("&before=");
        path.push_str(&query_escaped(before));
    }
    path
}

/// Una página de aciertos como la pinta la lista: lo más reciente arriba
/// (el servidor la manda en el orden del hilo, lo viejo primero), y por
/// dónde seguir si vino llena.
pub fn search_page(mut messages: Vec<ChatMessage>) -> TabPage<ChatMessage> {
    messages.reverse();
    let before = (messages.len() >= SEARCH_PAGE).then(|| messages.last().map(|message| message.created_at.clone())).flatten();
    TabPage { items: messages, before }
}

/// El enlace que se le manda a alguien de fuera (`guestLinkFor`): a la web,
/// que es lo único que tiene quien no tiene la app, y el token **detrás de
/// `#`**, que no sale del navegador ni acaba en los logs.
pub fn guest_link(base_url: &str, token: &str) -> String {
    format!("{}/app/join#{token}", base_url.trim_end_matches('/'))
}

/// Cuánto duró, `m:ss`. Vacío si no se sabe.
pub fn recording_duration(ms: Option<i64>) -> String {
    let Some(ms) = ms.filter(|ms| *ms > 0) else { return String::new() };
    let total = (ms + 500) / 1000;
    format!("{}:{:02}", total / 60, total % 60)
}

/// `cac-grabacion-2026-10-10-2253.m4a`: la fecha y la hora de quien mira,
/// de cuando empezó, que es como se busca luego en Descargas.
pub fn recording_file_name<Viewer: TimeZone>(recording: &Recording, zone: &Viewer) -> String
where
    Viewer::Offset: std::fmt::Display,
{
    let extension = if recording.is_audio() { "m4a" } else { "mp4" };
    let stamp = DateTime::parse_from_rfc3339(&recording.started_at)
        .map(|at| at.with_timezone(zone).format("%Y-%m-%d-%H%M").to_string())
        .unwrap_or_else(|_| recording.id.clone());
    format!("cac-grabacion-{stamp}.{extension}")
}

/// Lo que va debajo de una fila —quién, qué día y, si hace falta, la
/// dirección—, sin huecos por lo que falte: `Luis · 5/1/26`.
pub fn row_meta<Viewer: TimeZone>(author: Option<&str>, posted_at: &str, extra: Option<&str>, locale: Locale, zone: &Viewer) -> String {
    let day = instant(posted_at).map(|at| short_date(locale, at.with_timezone(zone).date_naive()));
    [author.map(str::to_string), day, extra.map(str::to_string)]
        .into_iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Un instante del servidor, con fecha y hora de quien mira. Vacío si no se
/// entiende.
pub fn local_moment<Viewer: TimeZone>(iso: &str, locale: Locale, zone: &Viewer) -> String {
    instant(iso).map(|at| date_and_time(locale, at.with_timezone(zone).naive_local())).unwrap_or_default()
}

/// El título de una llamada nueva, o por qué no vale.
pub fn call_title(written: &str) -> Option<String> {
    let title = written.trim();
    (!title.is_empty() && title.chars().count() <= CALL_TITLE_MAX).then(|| title.to_string())
}

#[derive(Deserialize)]
struct UrlTicket {
    ticket: String,
}

/// Las pestañas de los canales de este core.
pub struct ChannelTabs {
    core: Core,
}

impl Core {
    pub fn channel_tabs(&self) -> ChannelTabs {
        ChannelTabs { core: self.clone() }
    }
}

impl ChannelTabs {
    /// Una página de Multimedia.
    pub async fn media(&self, space_id: &str, query: &str, before: Option<&str>) -> Result<TabPage<ChatMediaItem>, ApiError> {
        self.core.get(&tab_path(space_id, "media", query, before)).await
    }

    /// Una página de Enlaces.
    pub async fn links(&self, space_id: &str, query: &str, before: Option<&str>) -> Result<TabPage<ChatLinkItem>, ApiError> {
        self.core.get(&tab_path(space_id, "links", query, before)).await
    }

    /// Una página de lo dicho en el canal que contiene lo buscado. La filtra
    /// el servidor: el hilo va paginado, y filtrar lo cargado contestaría
    /// «no está» a algo dicho hace tres pantallas.
    pub async fn search(&self, space_id: &str, query: &str, before: Option<&str>) -> Result<TabPage<ChatMessage>, ApiError> {
        let messages = self.core.get_list(&search_path(space_id, query, before)).await?;
        Ok(search_page(messages))
    }

    /// Si este canal tiene Grabaciones. Un fallo la esconde, igual que
    /// `enabled: false`: no se pidió nada, no hay nada que decir.
    pub async fn recordings_enabled(&self, space_id: &str) -> bool {
        self.core
            .get::<RecordingPolicy>(&format!("/api/v1/task-spaces/{space_id}/recordings/policy"))
            .await
            .is_ok_and(|policy| policy.enabled)
    }

    /// Lo grabado en este canal.
    pub async fn recordings(&self, space_id: &str) -> Result<Vec<Recording>, ApiError> {
        self.core.get_list(&format!("/api/v1/task-spaces/{space_id}/recordings")).await
    }

    /// Borra una grabación: el vídeo y todas sus pistas.
    pub async fn delete_recording(&self, recording_id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("/api/v1/recordings/{recording_id}"), None).await
    }

    /// Guarda una grabación en `target`, con la sesión en la cabecera. Va a
    /// disco según llega y sólo toma el nombre al terminar entera.
    pub async fn save_recording(&self, recording_id: &str, target: &std::path::Path) -> Result<(), ApiError> {
        self.core.download_to(&format!("/api/v1/recordings/{recording_id}/media"), target).await
    }

    /// Una dirección para ver la grabación fuera de la app. **Por el proxy de
    /// cac, nunca por el bucket**, y con un pase de adjuntos en vez del token
    /// de acceso: una URL acaba en los logs y el pase caduca en media hora.
    pub async fn recording_watch_url(&self, recording_id: &str) -> Result<String, ApiError> {
        let ticket: UrlTicket = self.core.call("POST", "/api/v1/auth/url-ticket?scope=media", Some(serde_json::json!({}))).await?;
        Ok(format!("{}/api/v1/recordings/{recording_id}/media?token={}", self.core.base_url(), query_escaped(&ticket.ticket)))
    }

    /// Las llamadas con invitados abiertas desde este canal.
    pub async fn call_invites(&self, org_id: &str, space_id: &str) -> Result<Vec<CallInvite>, ApiError> {
        self.core.get_list(&format!("/api/v1/call-invites?orgId={}&spaceId={}", query_escaped(org_id), query_escaped(space_id))).await
    }

    /// Abre una llamada con invitados que cuelga de este canal.
    pub async fn create_call_invite(&self, org_id: &str, space_id: &str, title: &str, ttl_hours: u32) -> Result<CallInvite, ApiError> {
        let title = call_title(title).ok_or_else(|| ApiError::Server { status: 400, code: "invite-invalid".into(), message: "A call needs a title".into() })?;
        let body = serde_json::json!({ "orgId": org_id, "spaceId": space_id, "title": title, "ttlHours": ttl_hours });
        self.core.call("POST", "/api/v1/call-invites", Some(body)).await
    }

    /// Cierra una llamada con invitados: el enlace deja de valer.
    pub async fn close_call_invite(&self, invite_id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("/api/v1/call-invites/{invite_id}"), None).await
    }

    /// El enlace de invitado de esa llamada, a la web de este servidor.
    pub fn guest_link(&self, invite: &CallInvite) -> String {
        guest_link(self.core.base_url(), &invite.link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn link(url: &str) -> ChatLinkItem {
        ChatLinkItem { url: url.into(), ..Default::default() }
    }

    fn page(urls: &[&str], before: Option<&str>) -> TabPage<ChatLinkItem> {
        TabPage { items: urls.iter().map(|url| link(url)).collect(), before: before.map(str::to_string) }
    }

    fn urls(tab: &PagedTab<ChatLinkItem>) -> Vec<&str> {
        tab.rows().iter().map(|row| row.url.as_str()).collect()
    }

    /// Las pestañas salen en su orden, y Grabaciones sólo si el servidor
    /// graba; buscar se ofrece en todas menos ahí.
    ///
    /// Roturas que atrapa: Grabaciones siempre, o nunca; las pestañas en
    /// otro orden que en la app de siempre; la caja de buscar en
    /// Grabaciones.
    #[test]
    fn the_tabs_follow_the_recording_policy() {
        use ChannelTab::*;
        assert_eq!(channel_tabs(false), [Thread, Media, Links]);
        assert_eq!(channel_tabs(true), [Thread, Media, Recordings, Links]);
        assert!(Thread.searchable() && Media.searchable() && Links.searchable() && !Recordings.searchable());
    }

    /// Las páginas se pegan sin repetir, el final lo dice el cursor del
    /// servidor y no una página corta, y lo que llega de un turno viejo no
    /// se pega.
    ///
    /// Roturas que atrapa: el mismo enlace dos veces; parar en una ventana
    /// vacía con canal detrás; seguir pidiendo sin cursor; pegar la página
    /// de una búsqueda ya cambiada; pedir dos veces la misma página.
    #[test]
    fn pages_are_glued_by_the_servers_cursor() {
        let mut tab = PagedTab::<ChatLinkItem>::default();
        let first = tab.restart();
        assert_eq!(tab.shows(), TabShows::Loading);
        assert!(tab.next().is_none(), "pide otra página con la primera de camino");
        assert!(tab.landed(first, Ok(page(&["a", "b"], Some("t1")))));
        assert_eq!(urls(&tab), ["a", "b"]);
        assert!(tab.has_more());

        let (second, before) = tab.next().expect("queda canal detrás");
        assert_eq!(before, "t1");
        assert!(tab.next().is_none(), "dos veces la misma página");
        // Una ventana vacía con cursor no es el final.
        assert!(tab.landed(second, Ok(page(&[], Some("t2")))));
        assert!(tab.has_more(), "una ventana vacía paró la lista");
        let (third, _) = tab.next().unwrap();
        assert!(tab.landed(third, Ok(page(&["b", "c"], None))));
        assert_eq!(urls(&tab), ["a", "b", "c"]);
        assert!(!tab.has_more() && tab.next().is_none(), "sin cursor no queda nada detrás");

        // Otra búsqueda: lo que llegue de la anterior no se pega.
        let stale = tab.restart();
        let fresh = tab.restart();
        assert!(!tab.landed(stale, Ok(page(&["viejo"], None))));
        assert_eq!(tab.shows(), TabShows::Loading);
        assert!(tab.landed(fresh, Ok(page(&[], None))));
        assert_eq!(tab.shows(), TabShows::Empty);
    }

    /// Una pestaña que no pudo leer lo dice, y no ofrece seguir.
    ///
    /// Roturas que atrapa: «todavía no hay nada» cuando no se pudo leer;
    /// «Cargar lo anterior» después de un fallo.
    #[test]
    fn a_tab_that_could_not_read_says_so() {
        let mut tab = PagedTab::<ChatLinkItem>::default();
        let turn = tab.restart();
        tab.landed(turn, Err(ApiError::Network("down".into())));
        assert_eq!(tab.shows(), TabShows::Failed);
        assert!(!tab.has_more() && tab.next().is_none());
    }

    /// Lo buscado viaja al servidor, recortado y escapado, y el cursor
    /// también.
    ///
    /// Roturas que atrapa: buscar sin mandar lo buscado; un `&` o un `+` sin
    /// escapar que parte la consulta; los espacios de los bordes; el cursor
    /// sin escapar; un `?q=` vacío cuando no se busca nada.
    #[test]
    fn the_query_travels_to_the_server() {
        assert_eq!(tab_path("s1", "media", "  ", None), "/api/v1/task-spaces/s1/chat/media");
        assert_eq!(
            tab_path("s1", "links", " a&b+c ", Some("2026-01-05T15:00:00+01:00")),
            "/api/v1/task-spaces/s1/chat/links?before=2026-01-05T15%3A00%3A00%2B01%3A00&q=a%26b%2Bc"
        );
        assert_eq!(search_path("s1", " factura nueva ", None), "/api/v1/task-spaces/s1/chat?q=factura%20nueva&limit=30");
        assert_eq!(search_path("s1", "x", Some("t+1")), "/api/v1/task-spaces/s1/chat?q=x&limit=30&before=t%2B1");
    }

    fn message(id: &str, at: &str) -> ChatMessage {
        ChatMessage { id: id.into(), created_at: at.into(), ..Default::default() }
    }

    /// Los aciertos salen con lo más reciente arriba, y sólo una página
    /// llena dice que puede haber más, desde el más viejo de ella.
    ///
    /// Roturas que atrapa: los aciertos en el orden del hilo; seguir tras
    /// una página corta; seguir desde el más nuevo, que repetiría la página.
    #[test]
    fn search_hits_come_newest_first() {
        let short = search_page(vec![message("1", "t1"), message("2", "t2")]);
        assert_eq!(short.items.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["2", "1"]);
        assert_eq!(short.before, None);
        let full = search_page((0..SEARCH_PAGE).map(|i| message(&i.to_string(), &format!("t{i:02}"))).collect());
        assert_eq!(full.items[0].id, (SEARCH_PAGE - 1).to_string());
        assert_eq!(full.before.as_deref(), Some("t00"));
    }

    /// El enlace de invitado va a la web y lleva el token detrás de `#`.
    ///
    /// Roturas que atrapa: el token en la ruta o en la consulta, donde acaba
    /// en los logs; una barra doble con una base que termina en `/`; un
    /// enlace a la app, que el invitado no tiene.
    #[test]
    fn the_guest_link_goes_to_the_web_with_the_token_after_the_hash() {
        assert_eq!(guest_link("https://cac.guz-studio.dev/", "tok"), "https://cac.guz-studio.dev/app/join#tok");
    }

    /// Una grabación dice si se puede ver, cuánto duró y con qué nombre se
    /// guarda, en la hora de quien mira.
    ///
    /// Roturas que atrapa: esconder una grabación parcial, que sí tiene
    /// vídeo; ofrecer ver una que aún se procesa; los segundos sin el cero;
    /// el nombre en UTC; `.mp4` para una de sólo audio.
    #[test]
    fn a_recording_says_what_can_be_done_with_it() {
        let mut recording = Recording { id: "r1".into(), status: "partial".into(), started_at: "2026-10-11T04:53:00Z".into(), ..Default::default() };
        assert!(recording.playable());
        recording.status = "finalizing".into();
        assert!(!recording.playable());
        assert_eq!(recording.status_key(), "recordings:statusFinalizing");
        assert_eq!(recording_duration(Some(65_000)), "1:05");
        assert_eq!(recording_duration(None), "");
        let mexico = FixedOffset::west_opt(6 * 3600).unwrap();
        assert_eq!(recording_file_name(&recording, &mexico), "cac-grabacion-2026-10-10-2253.mp4");
        recording.final_content_type = Some("audio/mp4".into());
        assert!(recording_file_name(&recording, &mexico).ends_with(".m4a"));
    }

    /// Debajo de una fila va quién y qué día en la zona de quien mira, y lo
    /// que falta no deja un separador colgando.
    ///
    /// Roturas que atrapa: el día en UTC, que corre un día al oeste; un
    /// «· ·» por un autor sin nombre; perder la dirección de un enlace con
    /// rótulo.
    #[test]
    fn a_rows_meta_line_has_no_holes() {
        let mexico = FixedOffset::west_opt(6 * 3600).unwrap();
        assert_eq!(row_meta(Some("Luis"), "2026-01-06T03:00:00Z", None, Locale::Es, &mexico), "Luis · 5/1/26");
        assert_eq!(row_meta(Some(" "), "2026-01-06T03:00:00Z", Some("https://x.dev"), Locale::Es, &mexico), "5/1/26 · https://x.dev");
        assert_eq!(row_meta(None, "", None, Locale::Es, &mexico), "");
    }

    /// El título de una llamada se recorta y no puede ir vacío ni pasarse.
    ///
    /// Roturas que atrapa: crear una llamada sin título; uno de espacios;
    /// uno más largo de lo que el servidor acepta.
    #[test]
    fn a_call_needs_a_title() {
        assert_eq!(call_title("  Revisión  ").as_deref(), Some("Revisión"));
        assert_eq!(call_title("   "), None);
        assert_eq!(call_title(&"a".repeat(CALL_TITLE_MAX)).map(|t| t.len()), Some(CALL_TITLE_MAX));
        assert_eq!(call_title(&"a".repeat(CALL_TITLE_MAX + 1)), None);
    }
}
