//! De un enlace de cac a una pantalla de la app.
//!
//! Un enlace de cac llega de muchos sitios —un mensaje, un comentario, un
//! documento, un aviso, otra app que lanza `cac-desktop <enlace>`— y con
//! varias formas, todas a la misma pantalla:
//!
//! - `https://cac.guz-studio.dev/app/tasks?task=…`, la de la web;
//! - `https://cac.guz-studio.dev/tasks?task=…`, sin `/app`, como las daba el
//!   MCP (el Gateway las manda a la web, que las redirige bajo `/app`:
//!   `TestEveryWebRouteWithoutAppLeadsToIt` en `backend/k8s`);
//! - `/app/tasks?…` y `/tasks?…`, la ruta sola, como la escribe el servidor
//!   en un aviso;
//! - `cac://tasks?task=…`, el esquema propio.
//!
//! [`app_route`] las deja todas en la ruta sola, y [`app_link`] dice a qué
//! pantalla lleva, con las rutas y los parámetros de `app/src/App.tsx`
//! (`?task=`, `?doc=`, `?space=`, `?c=`, `?u=`, `?open=`, `?post=`,
//! `/notes/:id`, `/organization`).
//!
//! **Lo que esta app no sabe abrir va al navegador, nunca a un error mudo**:
//! una ruta de cac que aquí no existe (los servidores, una llamada) es
//! [`AppLink::Browser`] con su dirección en la web. Lo que sí sabe leer pero
//! no encuentra (una tarea que no existe, un documento de un nodo que no es
//! de ninguna de las organizaciones de quien mira) no va al navegador, que
//! tampoco lo tendría: [`Core::open_link`] contesta [`LinkOpened::Failed`] y
//! quien pinta dice que no se pudo abrir.

use crate::comms::Conversation;
use crate::doc_links::{doc_link, DocLink};
use crate::nav::NodeKind;
use crate::notice::NoticeTarget;
use crate::tree::SpaceTree;
use crate::tree_rows::find_node;
use crate::Core;

/// El esquema propio. `cac:` sin las dos barras es otra cosa: una mención
/// (`cac:user/…`) o una grabación (`cac:recording/…`) dentro de un texto.
const SCHEME: &str = "cac://";

/// A qué pantalla lleva un enlace.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AppLink {
    /// Una tarea, por su id o por como se nombra fuera (`proteus-12`).
    Task { id: String },
    /// El documento de un nodo: su portada, una pestaña, una página o una
    /// sección (ver [`DocLink::Doc`]).
    Doc { kind: NodeKind, id: String, tab: Option<String>, page: Option<String>, section: Option<String> },
    /// Un canal, y si lo dice, un mensaje suyo (`&message=`). La web no
    /// enlaza todavía a un mensaje: sólo a su canal.
    Channel { space_id: String, message_id: Option<String> },
    /// Una conversación privada, por su id (`/dm?c=`).
    Direct { conversation_id: String },
    /// La conversación privada con alguien, por su usuario (`/dm?u=`).
    DirectWith { user_id: String },
    /// «Por atender», con ese pedido resaltado si lo dice (`/requests?open=`).
    Requests { code: Option<String> },
    /// El muro, con esa publicación abierta si la dice (`/feedback?post=`).
    Wall { post_id: Option<String> },
    /// Las notas, con esa abierta si la dice (`/notes/:id`).
    Notes { id: Option<String> },
    /// La pantalla de la organización (un aviso de Basecamp lleva aquí).
    Organization,
    /// Lo que esta app no abre: va al navegador, con esta dirección.
    Browser { url: String },
    /// Ni de cac ni de la web (`javascript:`, `file:`, un texto suelto): no
    /// se le da a nadie, y quien pinta dice que no se pudo abrir.
    Unopenable,
}

/// Lo que pasó al abrir un enlace.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum LinkOpened {
    /// Ya está en pantalla.
    Opened,
    /// La pantalla de la organización: es de la interfaz, que la abre ella.
    Organization,
    /// Al navegador, con esta dirección.
    Browser { url: String },
    /// No se encontró, o no es un enlace: se dice que no se pudo abrir.
    Failed,
}

/// El origen (`https://host:puerto`) de una dirección, en minúsculas.
fn origin(url: &str) -> Option<(String, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..end];
    // Con usuario dentro (`https://cac.guz-studio.dev@otro.com/`) lo de
    // delante de `/` no es el host del servidor, y no casa con él: no hace
    // falta mirar la arroba aparte.
    if host.is_empty() {
        return None;
    }
    Some((format!("{scheme}://{}", host.to_ascii_lowercase()), &rest[end..]))
}

/// Quita el `/app` de delante: la web vive ahí, y la ruta es lo de después.
fn without_app(route: &str) -> String {
    match route.strip_prefix("/app") {
        Some("") => "/".to_string(),
        Some(rest) if rest.starts_with(['/', '?', '#']) => {
            if rest.starts_with('/') {
                rest.to_string()
            } else {
                format!("/{rest}")
            }
        }
        _ => route.to_string(),
    }
}

/// La ruta de la app (`/tasks?task=…`) a la que lleva un enlace de cac, en
/// cualquiera de sus formas. `None` si no es de cac: una dirección de otro
/// sitio, o algo que no es un enlace.
///
/// `base_url` es el servidor con el que se habla: un enlace absoluto sólo es
/// de cac si es de ese origen.
pub fn app_route(href: &str, base_url: &str) -> Option<String> {
    let href = href.trim();
    if href.get(..SCHEME.len()).is_some_and(|head| head.eq_ignore_ascii_case(SCHEME)) {
        let rest = href[SCHEME.len()..].trim_start_matches('/');
        return (!rest.is_empty()).then(|| without_app(&format!("/{rest}")));
    }
    if href.starts_with('/') && !href.starts_with("//") {
        return Some(without_app(href));
    }
    let (from, route) = origin(href)?;
    let (ours, _) = origin(base_url)?;
    if from != ours {
        return None;
    }
    let route = if route.is_empty() || !route.starts_with('/') { format!("/{route}") } else { route.to_string() };
    Some(without_app(&route))
}

/// La dirección en la web de una ruta de la app.
pub fn web_url(route: &str, base_url: &str) -> String {
    format!("{}/app{}", base_url.trim_end_matches('/'), route)
}

/// `%C3%B3` → `ó`, y `+` → espacio (lo mismo que `doc_links`).
fn decoded(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = (bytes[at] == b'%').then(|| value.get(at + 1..at + 3)).flatten().and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(if bytes[at] == b'+' { b' ' } else { bytes[at] });
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// El valor no vacío de un parámetro de la consulta.
fn param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| decoded(value)).filter(|value| !value.trim().is_empty())
    })
}

/// A qué pantalla lleva un enlace (ver el comentario del módulo).
pub fn app_link(href: &str, base_url: &str) -> AppLink {
    let href = href.trim();
    let Some(route) = app_route(href, base_url) else {
        return match doc_link(href) {
            DocLink::Web { url } => AppLink::Browser { url },
            _ => AppLink::Unopenable,
        };
    };
    // Lo que la app no abre se abre en la web: la dirección absoluta tal cual
    // si llegó así, y si no, la de la ruta.
    let browser = || AppLink::Browser { url: if origin(href).is_some() { href.to_string() } else { web_url(&route, base_url) } };
    let (path, query) = route.split_once('?').unwrap_or((route.as_str(), ""));
    let query = query.split('#').next().unwrap_or_default();
    let path = path.split('#').next().unwrap_or_default().trim_end_matches('/');
    let found = match path {
        "/tasks" => match doc_link(&format!("/tasks?{query}")) {
            DocLink::Task { id } => Some(AppLink::Task { id }),
            DocLink::Doc { kind, id, tab, page, section } => Some(AppLink::Doc { kind, id, tab, page, section }),
            _ => None,
        },
        "/chat" => param(query, "space").map(|space_id| AppLink::Channel { space_id, message_id: param(query, "message") }),
        "/dm" => param(query, "c")
            .map(|conversation_id| AppLink::Direct { conversation_id })
            .or_else(|| param(query, "u").map(|user_id| AppLink::DirectWith { user_id })),
        "/requests" => Some(AppLink::Requests { code: param(query, "open") }),
        "/feedback" => Some(AppLink::Wall { post_id: param(query, "post") }),
        "/notes" => Some(AppLink::Notes { id: None }),
        "/organization" => Some(AppLink::Organization),
        _ => path.strip_prefix("/notes/").map(|id| AppLink::Notes { id: Some(decoded(id)) }),
    };
    match found {
        Some(screen) if only_plain_ids(&screen) => screen,
        Some(_) => AppLink::Unopenable,
        None => browser(),
    }
}

/// Un id tal como lo da el servidor (un uuid, `req-7f2a`, `proteus-12`).
/// Uno vacío no llega aquí: `param` y la ruta de notas ya lo descartan.
/// Se mira **descodificado**: `a%2F..%2Fx` es `a/../x`, y metido en una ruta
/// de `/api/v1` pediría otra cosa con la sesión de quien pulsó.
pub(crate) fn plain_id(id: &str) -> bool {
    id.len() <= 128 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Todo id de un enlace que acaba en una ruta de la API es un id y nada
/// más. La pestaña y la sección de un documento no van a ninguna ruta.
fn only_plain_ids(screen: &AppLink) -> bool {
    let ids: Vec<Option<&String>> = match screen {
        AppLink::Task { id } => vec![Some(id)],
        AppLink::Doc { id, page, .. } => vec![Some(id), page.as_ref()],
        AppLink::Channel { space_id, message_id } => vec![Some(space_id), message_id.as_ref()],
        AppLink::Direct { conversation_id } => vec![Some(conversation_id)],
        AppLink::DirectWith { user_id } => vec![Some(user_id)],
        AppLink::Requests { code } => vec![code.as_ref()],
        AppLink::Wall { post_id } => vec![post_id.as_ref()],
        AppLink::Notes { id } => vec![id.as_ref()],
        AppLink::Organization | AppLink::Browser { .. } | AppLink::Unopenable => vec![],
    };
    ids.into_iter().flatten().all(|id| plain_id(id))
}

/// Adónde lleva un `cac://` que la app no abre en una pantalla suya.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// El resumen de Trabajo: el inicio mismo y `/tasks` sin tarea ni documento.
    Home,
    /// «Mi trabajo», el panel por defecto de Trabajo (`/my-work`; `/mywork`
    /// como lo escribe quien no sabe el guion).
    MyWork,
    /// Los servidores, una llamada…: sólo traer la ventana al frente.
    Front,
}

/// A qué pantalla lleva una ruta que `app_link` manda al navegador, cuando
/// llegó por el esquema y por tanto no puede ir allí.
pub fn launch_landing(route: &str) -> Landing {
    let path = route.split(['?', '#']).next().unwrap_or_default().trim_end_matches('/');
    match path {
        "" | "/overview" | "/tasks" => Landing::Home,
        "/my-work" | "/mywork" => Landing::MyWork,
        _ => Landing::Front,
    }
}

/// `cac://` y nada más (con las barras que sea): un enlace a la app misma.
fn bare_scheme(href: &str) -> bool {
    let href = href.trim();
    href.get(..SCHEME.len()).is_some_and(|head| head.eq_ignore_ascii_case(SCHEME)) && href[SCHEME.len()..].trim_start_matches('/').is_empty()
}

impl Core {
    /// [`app_link`] con el servidor de esta sesión.
    pub fn app_link(&self, href: &str) -> AppLink {
        app_link(href, self.base_url())
    }

    /// [`app_route`] con el servidor de esta sesión.
    pub fn app_route(&self, href: &str) -> Option<String> {
        app_route(href, self.base_url())
    }

    /// Abre un enlace de cac en su pantalla, en su organización si es otra.
    /// Lo que esta app no tiene lo devuelve para el navegador; lo que no
    /// encuentra, como fallo (ver el comentario del módulo).
    pub async fn open_link(&self, href: &str) -> LinkOpened {
        let opened = match self.app_link(href) {
            AppLink::Browser { url } => return LinkOpened::Browser { url },
            AppLink::Unopenable => return LinkOpened::Failed,
            AppLink::Organization => return LinkOpened::Organization,
            AppLink::Task { id } => self.open_linked_task(&id).await,
            AppLink::Doc { kind, id, .. } => self.open_linked_doc(kind, &id, href).await,
            AppLink::Channel { space_id, .. } => {
                let conversation = Conversation::Channel { space_id };
                self.open_conversation(NoticeTarget { conversation, org_id: None }).await
            }
            AppLink::Direct { conversation_id } => {
                let org_id = self.direct_org(&conversation_id).await;
                org_id.is_some() && self.open_conversation(NoticeTarget { conversation: Conversation::Direct { conversation_id }, org_id }).await
            }
            AppLink::DirectWith { user_id } => match self.comms().open_with(&user_id).await {
                Ok(conversation_id) => {
                    let org_id = self.orgs().state().current_org_id;
                    self.open_conversation(NoticeTarget { conversation: Conversation::Direct { conversation_id }, org_id }).await
                }
                Err(_) => false,
            },
            // Con texto sin guardar en un documento no se va: ya se está
            // preguntando (`doc_edit`), y eso no es un fallo del enlace.
            AppLink::Requests { code } => {
                if self.requests().show(code) {
                    self.requests().load().await;
                }
                true
            }
            AppLink::Wall { post_id: Some(post_id) } => self.wall().open_from_link(&post_id).await,
            AppLink::Wall { post_id: None } => {
                self.wall().show();
                self.wall().opened().await;
                true
            }
            AppLink::Notes { id } => {
                self.notes().enter().await;
                if let Some(id) = id.filter(|_| self.notes().state().on_screen) {
                    self.notes().open(&id).await;
                }
                true
            }
        };
        if opened {
            LinkOpened::Opened
        } else {
            LinkOpened::Failed
        }
    }

    /// Abre un enlace que llegó por el esquema `cac://` (lo lanzó el sistema u
    /// otra instancia). A diferencia de [`Core::open_link`], **nunca** vuelve
    /// al navegador: esa dirección era de la web, que ofrece abrirla en la
    /// app, y rebotarla abría otra pestaña con la barra otra vez. Una ruta
    /// que no se reconoce lleva a la pantalla más cercana: el inicio
    /// (`/`, `/overview`, `/tasks` sin tarea ni documento), «Mi trabajo»
    /// (`/my-work`) o, para el resto, sólo traer la ventana al frente, que ya hizo quien llama.
    pub async fn open_launched_link(&self, href: &str) -> LinkOpened {
        let route = match self.app_link(href) {
            AppLink::Browser { .. } => self.app_route(href).unwrap_or_default(),
            AppLink::Unopenable if bare_scheme(href) => String::new(),
            _ => return self.open_link(href).await,
        };
        match launch_landing(&route) {
            Landing::Home => {
                self.overview().show();
            }
            // `show` deja todo lo demás y `leave` quita el resumen: lo que
            // queda es el panel por defecto.
            Landing::MyWork => {
                if self.overview().show() {
                    self.overview().leave();
                }
            }
            Landing::Front => {}
        }
        LinkOpened::Opened
    }

    /// Una conversación, en su organización, con comunicación delante.
    async fn open_conversation(&self, target: NoticeTarget) -> bool {
        self.requests().leave();
        self.notes().leave();
        self.notices().open(target).await
    }

    /// De qué organización es un directo: lo dice la lista de directos, que
    /// trae los de todas. Si no está (es nuevo), se vuelve a pedir una vez.
    async fn direct_org(&self, conversation_id: &str) -> Option<String> {
        let find = |core: &Core| core.counters().conversations().1.into_iter().flatten().find(|direct| direct.conversation_id == conversation_id).map(|direct| direct.org_id);
        if let Some(org_id) = find(self) {
            return Some(org_id);
        }
        self.counters().reload_directs().await;
        find(self)
    }

    /// Una tarea, en su organización. Abrirla primero dice de cuál es; si es
    /// de otra, se cambia (lo que cierra la tarea abierta) y se vuelve a abrir.
    async fn open_linked_task(&self, written: &str) -> bool {
        self.overview().leave();
        self.requests().leave();
        if !self.tasks().open_written(written).await {
            return false;
        }
        let state = self.tasks().state();
        let Some(detail) = state.detail else { return false };
        let task_id = detail.task.id.clone();
        let org_id = detail.task.org_id.clone();
        if !org_id.is_empty() && self.orgs().state().current_org_id.as_deref() != Some(org_id.as_str()) {
            if !self.switch_org(&org_id).await {
                return false;
            }
            self.tasks().open(&task_id).await;
        }
        true
    }

    /// De qué organización es un nodo (un espacio, una carpeta, una lista):
    /// el árbol que se mira, y si no está ahí, los de las demás.
    async fn node_org(&self, kind: NodeKind, id: &str) -> Option<String> {
        let current = self.orgs().state().current_org_id;
        if find_node(&self.tree().state().spaces, id).map(|(found, _)| found) == Some(kind) {
            return current;
        }
        for org in self.orgs().state().orgs.iter().filter(|org| Some(&org.id) != current.as_ref()) {
            let Ok(spaces) = self.get::<Vec<SpaceTree>>(&format!("/api/v1/task-spaces/?orgId={}", org.id)).await else { continue };
            if find_node(&spaces, id).map(|(found, _)| found) == Some(kind) {
                return Some(org.id.clone());
            }
        }
        None
    }

    /// Un documento, en su organización, con la documentación delante.
    async fn open_linked_doc(&self, kind: NodeKind, id: &str, href: &str) -> bool {
        let Some(org_id) = self.node_org(kind, id).await else { return false };
        if self.orgs().state().current_org_id.as_deref() != Some(org_id.as_str()) && !self.switch_org(&org_id).await {
            return false;
        }
        let Some(route) = self.app_route(href) else { return false };
        self.overview().leave();
        self.requests().leave();
        self.notes().leave();
        self.comms().leave();
        self.docs().follow_link(&route).await;
        self.docs().state().target.is_some_and(|target| target.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://cac.guz-studio.dev";

    fn link(href: &str) -> AppLink {
        app_link(href, BASE)
    }

    fn task(id: &str) -> AppLink {
        AppLink::Task { id: id.into() }
    }

    /// Las cinco formas de un mismo enlace llevan al mismo sitio.
    ///
    /// Roturas que atrapa: no quitar `/app` (la de la web se iría al
    /// navegador); no entender la liga sin `/app` del MCP; no entender
    /// `cac://`; tomar `/application` por `/app` + `lication`.
    #[test]
    fn every_shape_of_a_cac_link_reaches_the_same_place() {
        for href in [
            "https://cac.guz-studio.dev/app/tasks?task=command-and-control-12",
            "https://cac.guz-studio.dev/tasks?task=command-and-control-12",
            "HTTPS://CAC.guz-studio.dev/app/tasks?task=command-and-control-12",
            "/app/tasks?task=command-and-control-12",
            "/tasks?task=command-and-control-12",
            "cac://tasks?task=command-and-control-12",
            "cac:///app/tasks?task=command-and-control-12",
            " CAC://tasks?task=command-and-control-12 ",
        ] {
            assert_eq!(link(href), task("command-and-control-12"), "{href}");
        }
        // El contrato del esquema: `cac://` y la misma ruta y consulta de la
        // web sin `/app` (lo genera la web y lo lanza el sistema).
        assert_eq!(
            link("cac://tasks?doc=space:s1&page=p1"),
            link("https://cac.guz-studio.dev/app/tasks?doc=space:s1&page=p1"),
        );
        assert_eq!(link("cac://dm?c=c1"), link("https://cac.guz-studio.dev/app/dm?c=c1"));
        assert_eq!(app_route("/application/x", BASE).as_deref(), Some("/application/x"));
        assert_eq!(app_route("/app?task=1", BASE).as_deref(), Some("/?task=1"));
    }

    /// Cada pantalla, con lo que dice su enlace.
    ///
    /// Roturas que atrapa: perder la pestaña, la página o la sección de un
    /// documento; confundir un directo por su id con uno por su usuario;
    /// perder el pedido o la publicación que hay que abrir; no ver el
    /// mensaje de un canal; un `%2D` sin descodificar en el id de una nota.
    #[test]
    fn each_screen_is_read_with_what_its_link_says() {
        assert_eq!(link("https://cac.guz-studio.dev/app/tasks?task=0d6bf322-2267-4259-9b81-337575aca792"), task("0d6bf322-2267-4259-9b81-337575aca792"));
        assert_eq!(
            link("https://cac.guz-studio.dev/app/tasks?doc=folder:f1&tab=runbook&section=s9#x"),
            AppLink::Doc { kind: NodeKind::Folder, id: "f1".into(), tab: Some("runbook".into()), page: None, section: Some("s9".into()) }
        );
        assert_eq!(link("cac://tasks?doc=list:l1&page=p2"), AppLink::Doc { kind: NodeKind::List, id: "l1".into(), tab: None, page: Some("p2".into()), section: None });
        assert_eq!(link("/chat?space=s1"), AppLink::Channel { space_id: "s1".into(), message_id: None });
        assert_eq!(link("cac://chat?space=s1&message=m7"), AppLink::Channel { space_id: "s1".into(), message_id: Some("m7".into()) });
        assert_eq!(link("https://cac.guz-studio.dev/app/dm?c=c1"), AppLink::Direct { conversation_id: "c1".into() });
        assert_eq!(link("/dm?u=u9"), AppLink::DirectWith { user_id: "u9".into() });
        assert_eq!(link("/requests?open=req-7f2a"), AppLink::Requests { code: Some("req-7f2a".into()) });
        assert_eq!(link("cac://requests"), AppLink::Requests { code: None });
        assert_eq!(link("/feedback?post=p1"), AppLink::Wall { post_id: Some("p1".into()) });
        assert_eq!(link("/app/notes/n%2D1"), AppLink::Notes { id: Some("n-1".into()) });
        assert_eq!(link("/notes"), AppLink::Notes { id: None });
        assert_eq!(link("https://cac.guz-studio.dev/app/organization"), AppLink::Organization);
        assert_eq!(link("/organization/"), AppLink::Organization);
    }

    /// Lo que la app no abre va al navegador, en la web; lo de fuera, tal
    /// cual; y lo que no es un enlace no se le da a nadie.
    ///
    /// Roturas que atrapa: reclamar una dirección de otro sitio como de cac
    /// (`https://ejemplo.com/tasks?task=1`, o el truco del usuario delante
    /// del host); un error mudo para una ruta de cac que aquí no existe, o
    /// mandarla al navegador sin `/app` ni servidor (no abriría nada); un
    /// enlace de cac a medias (`/tasks?`, `/chat` sin canal) que no va a
    /// ningún sitio; darle al sistema un `javascript:`.
    #[test]
    fn what_this_app_does_not_open_goes_to_the_browser() {
        assert_eq!(link("https://ejemplo.com/tasks?task=1"), AppLink::Browser { url: "https://ejemplo.com/tasks?task=1".into() });
        assert_eq!(link("https://cac.guz-studio.dev@ejemplo.com/tasks?task=1"), AppLink::Browser { url: "https://cac.guz-studio.dev@ejemplo.com/tasks?task=1".into() });
        assert_eq!(link("https://cac.guz-studio.dev.ejemplo.com/tasks?task=1"), AppLink::Browser { url: "https://cac.guz-studio.dev.ejemplo.com/tasks?task=1".into() });
        assert_eq!(link("mailto:ana@example.com"), AppLink::Browser { url: "mailto:ana@example.com".into() });
        assert_eq!(link("cac://servers/x/backups"), AppLink::Browser { url: "https://cac.guz-studio.dev/app/servers/x/backups".into() });
        assert_eq!(link("/call/abc"), AppLink::Browser { url: "https://cac.guz-studio.dev/app/call/abc".into() });
        assert_eq!(link("https://cac.guz-studio.dev/app/servers/x"), AppLink::Browser { url: "https://cac.guz-studio.dev/app/servers/x".into() });
        assert_eq!(link("/tasks?"), AppLink::Browser { url: "https://cac.guz-studio.dev/app/tasks?".into() });
        assert_eq!(link("/chat"), AppLink::Browser { url: "https://cac.guz-studio.dev/app/chat".into() });
        for nothing in ["javascript:alert(1)", "file:///etc/passwd", "otra-pagina.md", "", "cac://", "cac:user/u1"] {
            assert_eq!(link(nothing), AppLink::Unopenable, "{nothing:?}");
        }
    }
    /// Un id de un enlace es un id: lo que, descodificado, lleva una barra,
    /// puntos u otra cosa no se abre, porque iría a una ruta de la API.
    ///
    /// Roturas que atrapa: mirar la barra antes de descodificar (`%2F`
    /// pasa); no mirar el id de la publicación, del canal, del directo, de
    /// la página o del pedido; dejar pasar puntos.
    #[test]
    fn an_id_that_is_not_an_id_opens_nothing() {
        for sneaky in [
            "cac://notes/a%2F..%2F..%2Fx",
            "cac://notes/..",
            "cac://feedback?post=..%2F..%2Fx",
            "cac://chat?space=s1%2F..%2Fx",
            "cac://chat?space=s1&message=..%2Fx",
            "cac://dm?c=..%2Fx",
            "cac://dm?u=a%3Fb",
            "cac://tasks?doc=list:l1&page=..%2Fx",
            "cac://tasks?doc=list:..%2Fx",
            "cac://requests?open=a%2Fb",
            "cac://tasks?task=a%2F..%2Fb",
        ] {
            assert_eq!(link(sneaky), AppLink::Unopenable, "{sneaky}");
        }
        assert_eq!(link("cac://notes/0d6bf322-2267-4259-9b81-337575aca792"), AppLink::Notes { id: Some("0d6bf322-2267-4259-9b81-337575aca792".into()) });
        assert_eq!(link("cac://feedback?post=p_1"), AppLink::Wall { post_id: Some("p_1".into()) });
    }
}
