//! El muro de feedback y comunicados (command-and-control-511).
//!
//! Vino de `app/src/store/posts.store.ts` y de `pages/Feedback.tsx` con sus
//! tarjetas (`components/feedback/`). El contrato es el del backend
//! (`domain/post.go`).
//!
//! - **Quién ve qué lo decide el servidor**: aquí sólo se pinta lo que llega.
//!   Los eventos `post:*` traen ids, así que al recibir uno se vuelve a pedir
//!   **lo que hay en pantalla**, no el muro entero: se perderían las páginas de
//!   «Cargar más», y con una reacción a algo «para todos» cada cliente pediría
//!   la primera página.
//! - **Abrirlo es haberlo visto**: se avisa al servidor (el punto de «nuevo»
//!   se apaga) después de pedirlo, que si no el «nuevo» se perdería en esta
//!   misma visita. Las tarjetas siguen marcando lo nuevo durante ella.
//! - **Reaccionar se ve al momento** y se deshace si el servidor dice que no.

use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::chat::apply_reaction;
use crate::comms_wire::ReactionSummary;
use crate::error::ApiError;
use crate::inbox::time_ago;
use crate::stream::ServerEvent;
use crate::texts::{text, text_count, text_with, Locale};
use crate::Core;

/// Lo que cabe en cada cosa (`validate` de `CreatePostRequest`).
pub const POST_TITLE_MAX: usize = 140;
pub const POST_BODY_MAX: usize = 20_000;
pub const POST_COMMENT_MAX: usize = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum PostKind {
    Announcement,
    #[default]
    #[serde(other)]
    Feedback,
}

/// `platform`: su organización y el equipo de cac. `everyone`: todas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum PostAudience {
    Everyone,
    #[default]
    #[serde(other)]
    Platform,
}

impl PostAudience {
    pub fn as_str(self) -> &'static str {
        match self {
            PostAudience::Platform => "platform",
            PostAudience::Everyone => "everyone",
        }
    }
}

/// Una publicación (`domain.PostResponse`). Con `comment_list` si viene del
/// detalle; la lista del muro no la trae.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct WallPost {
    pub id: String,
    pub kind: PostKind,
    pub audience: PostAudience,
    pub org_id: Option<String>,
    pub org_name: Option<String>,
    pub author_id: String,
    pub author_name: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub comments: i64,
    pub reactions: Vec<ReactionSummary>,
    /// Publicado después de la última vez que abriste el muro.
    pub unseen: bool,
    pub can_delete: bool,
    /// Sólo su autor la edita.
    pub can_edit: bool,
    /// Sólo un superadmin fija.
    pub can_pin: bool,
    pub pinned: bool,
    pub edited_at: Option<String>,
    /// `mcp`: la publicó un agente con el token de su autor.
    pub via: Option<String>,
    /// Redactado y sin publicar: sólo lo ve su autor.
    pub draft: bool,
    /// Es un borrador tuyo y lo puedes publicar.
    pub can_publish: bool,
    pub comment_list: Vec<WallComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct WallComment {
    pub id: String,
    pub post_id: String,
    pub author_id: String,
    pub author_name: String,
    pub body: String,
    pub created_at: String,
    pub can_delete: bool,
    pub reactions: Vec<ReactionSummary>,
    pub via: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Page {
    posts: Vec<WallPost>,
    next_cursor: Option<String>,
}

/// El muro, como lo pinta una pantalla.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WallState {
    pub posts: Vec<WallPost>,
    pub next_cursor: Option<String>,
    /// `false` mientras la primera página no ha llegado.
    pub loaded: bool,
    pub loading: bool,
    /// La última carga falló: lo que hubiera se queda.
    pub failed: bool,
    /// La publicación abierta, con sus comentarios.
    pub open: Option<WallPost>,
    /// Hay algo publicado desde la última vez que se abrió el muro: el punto
    /// de «nuevo» de su fila.
    pub has_new: bool,
}

/// Qué puede publicar quien mira, y a nombre de quién.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WallPermissions {
    /// Feedback de la organización que se mira: admin o member.
    pub can_feedback: bool,
    /// Comunicados: sólo un superadmin.
    pub can_announce: bool,
    pub org_id: Option<String>,
    pub org_name: Option<String>,
}

impl WallPermissions {
    /// Un viewer lee, reacciona y comenta, pero no publica.
    pub fn can_publish(&self) -> bool {
        self.can_feedback || self.can_announce
    }
}

/// Lo que se escribe en «Publicar».
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NewPost {
    pub announce: bool,
    pub audience: PostAudience,
    pub title: String,
    pub body: String,
}

/// Lo que se manda al publicar, o `None` si no se puede: sin texto, de más, o
/// sin permiso para lo que se pide.
///
/// Quien no puede feedback sólo publica comunicados (un superadmin que no es de
/// la organización que mira), y quien no es superadmin no los publica aunque
/// lo pida.
pub fn post_payload(new: &NewPost, permissions: &WallPermissions) -> Option<serde_json::Value> {
    if new.body.trim().is_empty() || new.body.chars().count() > POST_BODY_MAX || new.title.trim().chars().count() > POST_TITLE_MAX {
        return None;
    }
    let title = new.title.trim();
    if new.announce || !permissions.can_feedback {
        return permissions.can_announce.then(|| serde_json::json!({ "kind": "announcement", "title": title, "body": new.body }));
    }
    let org_id = permissions.org_id.as_deref()?;
    Some(serde_json::json!({ "kind": "feedback", "orgId": org_id, "audience": new.audience.as_str(), "title": title, "body": new.body }))
}

/// Lo que se manda al guardar una edición: el título y el texto, y la
/// audiencia sólo en un feedback (un comunicado es de todos).
pub fn edit_payload(post: &WallPost, title: &str, body: &str, audience: PostAudience) -> Option<serde_json::Value> {
    if body.trim().is_empty() || body.chars().count() > POST_BODY_MAX || title.trim().chars().count() > POST_TITLE_MAX {
        return None;
    }
    let mut payload = serde_json::json!({ "title": title.trim(), "body": body });
    if post.kind != PostKind::Announcement {
        payload["audience"] = serde_json::Value::String(audience.as_str().to_string());
    }
    Some(payload)
}

/// Para quién es, en una palabra (`PostCard.tsx`): un comunicado es un
/// comunicado; un feedback es para todos, para su organización o, sin ella,
/// para cac.
pub fn audience_label(post: &WallPost, locale: Locale) -> String {
    audience_text(post.kind, post.audience, post.org_name.as_deref(), locale)
}

/// Lo mismo para una audiencia que se está eligiendo.
pub fn audience_text(kind: PostKind, audience: PostAudience, org_name: Option<&str>, locale: Locale) -> String {
    if kind == PostKind::Announcement {
        return text(locale, "common:feedback.announcement");
    }
    match (audience, org_name.filter(|name| !name.is_empty())) {
        (PostAudience::Everyone, _) => text(locale, "common:feedback.audienceEveryone"),
        (_, Some(org)) => text_with(locale, "common:feedback.audienceOrg", &[("org", org)]),
        (_, None) => text(locale, "common:feedback.audiencePlatform"),
    }
}

/// La línea de debajo del autor: de qué organización, cuánto hace, si lo
/// escribió un agente y si se editó.
pub fn post_meta<Viewer: TimeZone>(post: &WallPost, now: &DateTime<Viewer>, locale: Locale) -> String {
    let parts = [
        post.org_name.clone().unwrap_or_default(),
        time_ago(&post.created_at, now, locale),
        if post.via.as_deref() == Some("mcp") { text(locale, "common:feedback.viaAgent") } else { String::new() },
        if post.edited_at.is_some() { text(locale, "common:feedback.edited") } else { String::new() },
    ];
    parts.into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// La línea del modal: lo mismo, con el nombre del autor delante.
pub fn post_detail_meta<Viewer: TimeZone>(post: &WallPost, now: &DateTime<Viewer>, locale: Locale) -> String {
    let rest = post_meta(post, now, locale);
    if rest.is_empty() {
        post.author_name.clone()
    } else {
        format!("{} · {rest}", post.author_name)
    }
}

/// La línea de debajo del autor de un comentario.
pub fn comment_meta<Viewer: TimeZone>(comment: &WallComment, now: &DateTime<Viewer>, locale: Locale) -> String {
    let when = time_ago(&comment.created_at, now, locale);
    if comment.via.as_deref() == Some("mcp") {
        format!("{when} · {}", text(locale, "common:feedback.viaAgent"))
    } else {
        when
    }
}

pub fn comments_label(count: i64, locale: Locale) -> String {
    text_count(locale, "common:feedback.comments", count)
}

/// Qué hace una publicación nueva que llega por el stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewPostEffect {
    Nothing,
    /// Subirla al muro que se está mirando.
    Merge,
    /// Encender el punto de «nuevo».
    Dot,
}

/// `seen` es lo que contestó pedirla: `None` si no se puede abrir. Un evento
/// «para todos» llega también a quien no la ve (sin organización, por
/// ejemplo), y sólo cuenta si se abre. Lo propio no enciende el punto.
fn new_post_effect(seen: Option<&WallPost>, me: Option<&str>, in_front: bool) -> NewPostEffect {
    let Some(post) = seen else { return NewPostEffect::Nothing };
    if in_front {
        NewPostEffect::Merge
    } else if me.is_some_and(|me| me != post.author_id) {
        NewPostEffect::Dot
    } else {
        NewPostEffect::Nothing
    }
}

fn local(code: &str, message: &str) -> ApiError {
    ApiError::Server { status: 0, code: code.to_string(), message: message.to_string() }
}

/// Una tarjeta del muro: lo mismo que el detalle, sin los comentarios.
fn card_of(post: &WallPost) -> WallPost {
    WallPost { comment_list: Vec::new(), ..post.clone() }
}

#[derive(Default)]
pub(crate) struct WallData {
    state: WallState,
}

pub struct Wall {
    core: Core,
}

impl Core {
    pub fn wall(&self) -> Wall {
        Wall { core: self.clone() }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostEvent {
    post_id: Option<String>,
}

impl Wall {
    fn data(&self) -> std::sync::MutexGuard<'_, WallData> {
        self.core.inner.wall.lock().expect("wall lock")
    }

    pub fn state(&self) -> WallState {
        self.data().state.clone()
    }

    fn change(&self, edit: impl FnOnce(&mut WallState)) {
        let state = {
            let mut data = self.data();
            edit(&mut data.state);
            data.state.clone()
        };
        self.core.inner.listener.wall_changed(state);
    }

    /// Qué puede publicar quien mira, en la organización que mira.
    pub fn permissions(&self) -> WallPermissions {
        let orgs = self.core.orgs().state();
        let org = orgs.current();
        let superadmin = self.core.session().is_some_and(|session| session.superadmin);
        WallPermissions {
            can_feedback: org.is_some_and(|org| org.role == "admin" || org.role == "member"),
            can_announce: superadmin,
            org_id: org.map(|org| org.id.clone()),
            org_name: org.map(|org| org.name.clone()),
        }
    }

    /// Cómo se llama quien mira, para ponerlo en lo que escribe al momento.
    fn me(&self) -> String {
        self.core.session().map(|session| session.name.filter(|name| !name.trim().is_empty()).unwrap_or(session.username)).unwrap_or_default()
    }

    /// Pone el muro delante de la conversación. Sólo dentro de comunicación.
    pub fn show(&self) {
        self.core.comms().show_wall();
        self.change(|state| state.has_new = false);
    }

    /// Abrir el muro es haberlo visto: se pide y, **después**, se avisa.
    pub async fn opened(&self) {
        self.load().await;
        let _ = self.mark_seen().await;
    }

    /// El enlace de un aviso (`/feedback?post=<id>`): el muro delante con esa
    /// publicación abierta.
    pub async fn open_from_link(&self, post_id: &str) -> bool {
        self.show();
        self.load().await;
        let opened = self.open_post(post_id).await.is_ok();
        let _ = self.mark_seen().await;
        opened
    }

    /// La primera página, sustituyendo lo que hubiera.
    pub async fn load(&self) {
        self.change(|state| state.loading = true);
        match self.core.get::<Page>("/api/v1/posts").await {
            Ok(page) => {
                let viewing = self.core.comms().wall_in_front();
                self.change(|state| {
                    state.has_new = !viewing && page.posts.iter().any(|post| post.unseen);
                    state.posts = page.posts;
                    state.next_cursor = page.next_cursor.filter(|cursor| !cursor.is_empty());
                    state.loaded = true;
                    state.loading = false;
                    state.failed = false;
                });
            }
            Err(_) => self.change(|state| {
                state.loading = false;
                state.failed = true;
            }),
        }
    }

    /// La página que sigue, debajo de lo ya cargado.
    pub async fn load_more(&self) {
        let Some(cursor) = self.state().next_cursor.filter(|_| !self.state().loading) else { return };
        self.change(|state| state.loading = true);
        let path = format!("/api/v1/posts?cursor={}", encode(&cursor));
        match self.core.get::<Page>(&path).await {
            Ok(page) => self.change(|state| {
                state.posts.extend(page.posts);
                state.next_cursor = page.next_cursor.filter(|cursor| !cursor.is_empty());
                state.loading = false;
            }),
            Err(_) => self.change(|state| {
                state.loading = false;
                state.failed = true;
            }),
        }
    }

    /// Abre una publicación entera.
    pub async fn open_post(&self, id: &str) -> Result<(), ApiError> {
        let detail: WallPost = self.core.get(&format!("/api/v1/posts/{id}")).await?;
        self.change(|state| state.open = Some(detail));
        Ok(())
    }

    pub fn close_post(&self) {
        if self.state().open.is_some() {
            self.change(|state| state.open = None);
        }
    }

    /// Publica y vuelve a pedir el muro: el orden lo da el servidor.
    pub async fn create(&self, new: NewPost) -> Result<WallPost, ApiError> {
        let Some(payload) = post_payload(&new, &self.permissions()) else { return Err(local("post-invalid", "That cannot be posted")) };
        let created: WallPost = self.core.call("POST", "/api/v1/posts", Some(payload)).await?;
        self.load().await;
        Ok(created)
    }

    pub async fn remove(&self, id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("/api/v1/posts/{id}"), None).await?;
        self.change(|state| {
            state.posts.retain(|post| post.id != id);
            if state.open.as_ref().is_some_and(|open| open.id == id) {
                state.open = None;
            }
        });
        Ok(())
    }

    /// Guarda lo editado de la publicación abierta.
    pub async fn update(&self, id: &str, title: &str, body: &str, audience: PostAudience) -> Result<(), ApiError> {
        let post = self.state().open.filter(|open| open.id == id);
        let Some(payload) = post.as_ref().and_then(|post| edit_payload(post, title, body, audience)) else {
            return Err(local("post-invalid", "That cannot be saved"));
        };
        let detail: WallPost = self.core.call("PATCH", &format!("/api/v1/posts/{id}"), Some(payload)).await?;
        self.replace(id, detail);
        Ok(())
    }

    /// Lo que el servidor contestó sobre una publicación, puesto donde esté:
    /// el detalle si es la abierta, y su tarjeta.
    fn replace(&self, id: &str, detail: WallPost) {
        self.change(|state| {
            let card = card_of(&detail);
            for post in state.posts.iter_mut().filter(|post| post.id == id) {
                *post = card.clone();
            }
            if state.open.as_ref().is_some_and(|open| open.id == id) {
                state.open = Some(detail);
            }
        });
    }

    /// Un borrador deja de serlo y sale como nuevo para todos.
    pub async fn publish(&self, id: &str) -> Result<(), ApiError> {
        let detail: WallPost = self.core.call("POST", &format!("/api/v1/posts/{id}/publish"), Some(serde_json::json!({}))).await?;
        self.change(|state| {
            if state.open.as_ref().is_some_and(|open| open.id == id) {
                state.open = Some(detail);
            }
        });
        self.load().await;
        Ok(())
    }

    /// Fija o suelta. Mueve la publicación de sitio: se vuelve a pedir.
    pub async fn pin(&self, id: &str, pinned: bool) -> Result<(), ApiError> {
        let detail: WallPost = self.core.call("POST", &format!("/api/v1/posts/{id}/pin"), Some(serde_json::json!({ "pinned": pinned }))).await?;
        self.change(|state| {
            if state.open.as_ref().is_some_and(|open| open.id == id) {
                state.open = Some(detail);
            }
        });
        self.load().await;
        Ok(())
    }

    /// Pone o quita la reacción de quien mira, en la tarjeta y en el modal a
    /// la vez; si el servidor no la acepta, vuelve a como estaba.
    pub async fn react(&self, id: &str, emoji: &str, on: bool) -> Result<(), ApiError> {
        if !crate::comms_wire::REACTION_EMOJIS.contains(&emoji) {
            return Err(local("bad-emoji", "That reaction is not one of the set"));
        }
        let me = self.me();
        let (before_card, before_open) = {
            let state = self.state();
            (
                state.posts.iter().find(|post| post.id == id).map(|post| post.reactions.clone()),
                state.open.as_ref().filter(|open| open.id == id).map(|open| open.reactions.clone()),
            )
        };
        self.change(|state| {
            for post in state.posts.iter_mut().filter(|post| post.id == id) {
                post.reactions = apply_reaction(&post.reactions, emoji, on, &me);
            }
            if let Some(open) = state.open.as_mut().filter(|open| open.id == id) {
                open.reactions = apply_reaction(&open.reactions, emoji, on, &me);
            }
        });
        let result = self.core.call_ok("POST", &format!("/api/v1/posts/{id}/reactions"), Some(serde_json::json!({ "emoji": emoji, "on": on }))).await;
        if result.is_err() {
            self.change(|state| {
                if let Some(before) = before_card {
                    for post in state.posts.iter_mut().filter(|post| post.id == id) {
                        post.reactions = before.clone();
                    }
                }
                if let Some(before) = before_open {
                    if let Some(open) = state.open.as_mut().filter(|open| open.id == id) {
                        open.reactions = before;
                    }
                }
            });
        }
        result
    }

    /// Lo mismo para un comentario: sólo se ve en el modal.
    pub async fn react_comment(&self, post_id: &str, comment_id: &str, emoji: &str, on: bool) -> Result<(), ApiError> {
        if !crate::comms_wire::REACTION_EMOJIS.contains(&emoji) {
            return Err(local("bad-emoji", "That reaction is not one of the set"));
        }
        let me = self.me();
        let before = self.state().open.filter(|open| open.id == post_id).and_then(|open| open.comment_list.iter().find(|comment| comment.id == comment_id).map(|comment| comment.reactions.clone()));
        let put = |wall: &Wall, edit: &dyn Fn(&[ReactionSummary]) -> Vec<ReactionSummary>| {
            wall.change(|state| {
                if let Some(comment) = state.open.as_mut().filter(|open| open.id == post_id).and_then(|open| open.comment_list.iter_mut().find(|comment| comment.id == comment_id)) {
                    comment.reactions = edit(&comment.reactions);
                }
            });
        };
        put(self, &|reactions| apply_reaction(reactions, emoji, on, &me));
        let path = format!("/api/v1/posts/{post_id}/comments/{comment_id}/reactions");
        let result = self.core.call_ok("POST", &path, Some(serde_json::json!({ "emoji": emoji, "on": on }))).await;
        if result.is_err() {
            if let Some(before) = before {
                put(self, &|_| before.clone());
            }
        }
        result
    }

    /// Comenta la publicación. El servidor contesta con ella entera.
    pub async fn comment(&self, id: &str, body: &str) -> Result<(), ApiError> {
        let body = body.trim();
        if body.is_empty() || body.chars().count() > POST_COMMENT_MAX {
            return Err(local("post-invalid", "That cannot be sent"));
        }
        let detail: WallPost = self.core.call("POST", &format!("/api/v1/posts/{id}/comments"), Some(serde_json::json!({ "body": body }))).await?;
        self.change(|state| {
            let count = detail.comment_list.len() as i64;
            for post in state.posts.iter_mut().filter(|post| post.id == id) {
                post.comments = count;
            }
            if state.open.as_ref().is_some_and(|open| open.id == id) {
                state.open = Some(detail);
            }
        });
        Ok(())
    }

    pub async fn remove_comment(&self, post_id: &str, comment_id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("/api/v1/posts/{post_id}/comments/{comment_id}"), None).await?;
        self.change(|state| {
            let Some(open) = state.open.as_mut().filter(|open| open.id == post_id) else { return };
            open.comment_list.retain(|comment| comment.id != comment_id);
            let count = open.comment_list.len() as i64;
            for post in state.posts.iter_mut().filter(|post| post.id == post_id) {
                post.comments = count;
            }
        });
        Ok(())
    }

    /// Le dice al servidor que se miró el muro, y apaga el punto.
    pub async fn mark_seen(&self) -> Result<(), ApiError> {
        self.change(|state| state.has_new = false);
        self.core.call_ok("POST", "/api/v1/posts/seen", Some(serde_json::json!({}))).await
    }

    /// Un `post:*` del stream.
    pub(crate) fn server_event(&self, event: &ServerEvent) {
        if !matches!(event.event.as_str(), "post:new" | "post:updated" | "post:comment" | "post:reaction" | "post:deleted") {
            return;
        }
        let Some(id) = serde_json::from_str::<PostEvent>(&event.data).ok().and_then(|event| event.post_id) else { return };
        let (wall, kind) = (self.core.wall(), event.event.clone());
        tokio::spawn(async move { wall.on_event(&kind, &id).await });
    }

    async fn on_event(&self, kind: &str, id: &str) {
        if kind == "post:deleted" {
            self.change(|state| {
                state.posts.retain(|post| post.id != id);
                if state.open.as_ref().is_some_and(|open| open.id == id) {
                    state.open = None;
                }
            });
            return;
        }
        if kind == "post:new" {
            let seen = self.core.get::<WallPost>(&format!("/api/v1/posts/{id}")).await.ok();
            let me = self.core.session().map(|session| session.id);
            match new_post_effect(seen.as_ref(), me.as_deref(), self.core.comms().wall_in_front()) {
                NewPostEffect::Nothing => {}
                NewPostEffect::Merge => self.merge_first_page().await,
                NewPostEffect::Dot => self.change(|state| state.has_new = true),
            }
            return;
        }
        let shown = {
            let state = self.state();
            state.posts.iter().any(|post| post.id == id) || state.open.as_ref().is_some_and(|open| open.id == id)
        };
        if shown {
            self.refresh_one(id).await;
        }
    }

    /// Vuelve a pedir una publicación que está en pantalla; si ya no se puede
    /// ver, se quita.
    async fn refresh_one(&self, id: &str) {
        match self.core.get::<WallPost>(&format!("/api/v1/posts/{id}")).await {
            Ok(detail) => self.replace(id, detail),
            Err(_) => self.change(|state| {
                state.posts.retain(|post| post.id != id);
                if state.open.as_ref().is_some_and(|open| open.id == id) {
                    state.open = None;
                }
            }),
        }
    }

    /// Lo nuevo arriba, sin perder lo que ya se había cargado debajo.
    async fn merge_first_page(&self) {
        let Ok(page) = self.core.get::<Page>("/api/v1/posts").await else { return };
        self.change(|state| {
            let rest: Vec<WallPost> = state.posts.iter().filter(|old| !page.posts.iter().any(|fresh| fresh.id == old.id)).cloned().collect();
            let no_rest = rest.is_empty();
            state.posts = page.posts.into_iter().chain(rest).collect();
            if no_rest {
                state.next_cursor = page.next_cursor.filter(|cursor| !cursor.is_empty());
            }
        });
    }

    /// Al entrar, y al volver de una caída: lo publicado mientras no había
    /// stream no llegó por evento.
    pub(crate) async fn refresh(&self) {
        if self.state().loaded {
            self.merge_first_page().await;
        } else {
            self.load().await;
        }
    }

    /// Al salir: el muro de una persona no es de la siguiente.
    pub(crate) fn clear(&self) {
        *self.data() = WallData::default();
    }
}

/// Un valor para una consulta: lo que no es seguro, por su porcentaje.
fn encode(value: &str) -> String {
    value.bytes().map(|byte| if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn permissions(can_feedback: bool, can_announce: bool, org: Option<&str>) -> WallPermissions {
        WallPermissions { can_feedback, can_announce, org_id: org.map(str::to_string), org_name: org.map(|id| format!("Org {id}")) }
    }

    fn new_post(announce: bool, audience: PostAudience, title: &str, body: &str) -> NewPost {
        NewPost { announce, audience, title: title.into(), body: body.into() }
    }

    /// Lo que se manda al publicar.
    ///
    /// Roturas que atrapa: mandar feedback sin organización; un viewer que
    /// publica; un member que publica un comunicado; un superadmin ajeno a la
    /// organización que no puede publicar el suyo; olvidar la audiencia
    /// elegida o mandar la audiencia en un comunicado; publicar un texto en
    /// blanco o uno que no cabe; mandar el título sin recortar.
    #[test]
    fn what_is_posted_depends_on_who_posts() {
        let member = permissions(true, false, Some("o1"));
        let payload = post_payload(&new_post(false, PostAudience::Everyone, "  Idea ", "texto\n"), &member).unwrap();
        assert_eq!(payload, serde_json::json!({ "kind": "feedback", "orgId": "o1", "audience": "everyone", "title": "Idea", "body": "texto\n" }));
        assert_eq!(post_payload(&new_post(false, PostAudience::Platform, "", "hola"), &member).unwrap()["audience"], "platform");

        assert_eq!(post_payload(&new_post(true, PostAudience::Platform, "", "hola"), &member), None, "un member no publica comunicados");
        assert_eq!(post_payload(&new_post(false, PostAudience::Platform, "", "hola"), &permissions(false, false, Some("o1"))), None, "un viewer no publica");
        assert_eq!(post_payload(&new_post(false, PostAudience::Platform, "", "hola"), &permissions(true, false, None)), None, "feedback sin organización");

        let both = permissions(true, true, Some("o1"));
        assert_eq!(post_payload(&new_post(true, PostAudience::Everyone, "", "aviso"), &both).unwrap(), serde_json::json!({ "kind": "announcement", "title": "", "body": "aviso" }));
        let outsider = permissions(false, true, None);
        assert_eq!(post_payload(&new_post(false, PostAudience::Platform, "", "aviso"), &outsider).unwrap()["kind"], "announcement", "sin feedback, sólo comunicados");

        assert_eq!(post_payload(&new_post(false, PostAudience::Platform, "", "  \n "), &member), None);
        assert_eq!(post_payload(&new_post(false, PostAudience::Platform, "", &"x".repeat(POST_BODY_MAX + 1)), &member), None);
        assert!(post_payload(&new_post(false, PostAudience::Platform, "", &"x".repeat(POST_BODY_MAX)), &member).is_some());
        assert_eq!(post_payload(&new_post(false, PostAudience::Platform, &"t".repeat(POST_TITLE_MAX + 1), "hola"), &member), None);
    }

    /// Lo que se manda al editar. Roturas que atrapa: mandar la audiencia de
    /// un comunicado; no mandarla en un feedback; guardar un texto en blanco.
    #[test]
    fn an_edit_sends_the_audience_only_for_feedback() {
        let feedback = WallPost { kind: PostKind::Feedback, ..Default::default() };
        let announcement = WallPost { kind: PostKind::Announcement, ..Default::default() };
        assert_eq!(edit_payload(&feedback, " nuevo ", "cuerpo", PostAudience::Everyone).unwrap(), serde_json::json!({ "title": "nuevo", "body": "cuerpo", "audience": "everyone" }));
        assert_eq!(edit_payload(&announcement, "n", "cuerpo", PostAudience::Everyone).unwrap(), serde_json::json!({ "title": "n", "body": "cuerpo" }));
        assert_eq!(edit_payload(&feedback, "n", "  ", PostAudience::Platform), None);
    }

    /// Para quién es y la línea del autor. Roturas que atrapa: nombrar a
    /// «cac» en una publicación de una organización; olvidar el «vía agente»
    /// o el «editada»; dejar un separador suelto cuando falta una pieza.
    #[test]
    fn a_post_says_who_it_is_for_and_where_it_comes_from() {
        let now = DateTime::parse_from_rfc3339("2026-11-25T12:00:00Z").unwrap().with_timezone(&Utc);
        let mut post = WallPost { org_name: Some("Uno".into()), created_at: "2026-11-25T11:00:00Z".into(), ..Default::default() };
        assert_eq!(audience_label(&post, Locale::Es), "Solo Uno");
        post.audience = PostAudience::Everyone;
        assert_eq!(audience_label(&post, Locale::Es), "Para todos");
        post.audience = PostAudience::Platform;
        post.org_name = None;
        assert_eq!(audience_label(&post, Locale::Es), "Solo para cac");
        post.kind = PostKind::Announcement;
        assert_eq!(audience_label(&post, Locale::Es), "Comunicado");

        post.org_name = Some("Uno".into());
        assert_eq!(post_meta(&post, &now, Locale::Es), "Uno · hace 1 hora");
        post.via = Some("mcp".into());
        post.edited_at = Some("2026-11-25T11:30:00Z".into());
        assert_eq!(post_meta(&post, &now, Locale::Es), "Uno · hace 1 hora · vía agente · editada");
        post.org_name = None;
        post.author_name = "Ana".into();
        assert_eq!(post_detail_meta(&post, &now, Locale::Es), "Ana · hace 1 hora · vía agente · editada");
        assert_eq!(comments_label(1, Locale::Es), "1 comentario");
        assert_eq!(comments_label(3, Locale::En), "3 comments");
    }

    /// La forma del cable. Roturas que atrapa: un tipo de publicación o una
    /// audiencia que esta versión no conoce y rompe el muro entero; perder el
    /// detalle de los comentarios.
    #[test]
    fn the_wire_reads_unknown_kinds_and_comments() {
        let raw = serde_json::json!({
            "id": "p1", "kind": "poll", "audience": "staff", "authorId": "u1", "authorName": "Ana", "title": "t", "body": "b",
            "createdAt": "2026-11-25T11:00:00Z", "comments": 1, "reactions": [{ "emoji": "👍", "count": 1, "mine": true, "names": ["Ana"] }],
            "commentList": [{ "id": "c1", "postId": "p1", "authorId": "u2", "authorName": "Bea", "body": "sí", "createdAt": "2026-11-25T11:05:00Z", "canDelete": true, "reactions": [] }],
        });
        let post: WallPost = serde_json::from_value(raw).unwrap();
        assert_eq!((post.kind, post.audience), (PostKind::Feedback, PostAudience::Platform));
        assert_eq!((post.comment_list.len(), post.comment_list[0].can_delete, post.reactions[0].mine), (1, true, true));
        assert_eq!(serde_json::from_str::<Page>(r#"{"posts":[]}"#).unwrap().next_cursor, None);
    }

    /// Qué hace una publicación nueva. Roturas que atrapa: encender el punto
    /// por lo que uno mismo publicó o por algo que no se puede abrir; no subirla
    /// con el muro delante; encenderlo con el muro delante.
    #[test]
    fn a_new_post_lights_the_dot_only_for_what_others_posted_and_can_be_seen() {
        let post = |author: &str| WallPost { author_id: author.into(), ..Default::default() };
        assert_eq!(new_post_effect(Some(&post("u2")), Some("u1"), false), NewPostEffect::Dot);
        assert_eq!(new_post_effect(Some(&post("u1")), Some("u1"), false), NewPostEffect::Nothing);
        assert_eq!(new_post_effect(None, Some("u1"), false), NewPostEffect::Nothing);
        assert_eq!(new_post_effect(None, Some("u1"), true), NewPostEffect::Nothing);
        assert_eq!(new_post_effect(Some(&post("u2")), Some("u1"), true), NewPostEffect::Merge);
        assert_eq!(new_post_effect(Some(&post("u2")), None, false), NewPostEffect::Nothing, "sin sesión no hay a quién");
    }

    #[test]
    fn a_cursor_travels_encoded() {
        assert_eq!(encode("abc-1_2.3~"), "abc-1_2.3~");
        assert_eq!(encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }
}
