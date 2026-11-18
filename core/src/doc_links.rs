//! Adónde lleva un enlace escrito en un documento.
//!
//! Un documento se enlaza con lo de dentro —una tarea, otra página, la
//! pestaña de otro nodo, una sección— y con lo de fuera. Qué es cada enlace
//! lo dice [`doc_link`], con las reglas de la app de siempre
//! (`taskIdFromHref` y `docRefFromHref` en `card-menu.ts`, `openInternalRef`
//! en `open-ref.ts`); qué se hace con él, [`Docs::follow_link`]. La pantalla
//! sólo le da al navegador del sistema lo que es de fuera.
//!
//! Dos reglas:
//!
//! - **El prefijo es exacto.** Un enlace de dentro empieza por `/tasks?`; la
//!   versión laxa («acaba en /tasks») reclamaba `https://ejemplo.com/tasks?task=1`
//!   y un enlace externo perfectamente bueno dejaba de abrir el navegador.
//! - **Lo que no se sabe abrir se dice.** Un enlace de dentro que esta app
//!   no entiende, o que apunta a algo que ya no está, no se le da al sistema
//!   (que contestaría con un aviso suyo, o con nada): deja dicho «No se pudo
//!   abrir el enlace».

use std::sync::atomic::{AtomicU64, Ordering};

use crate::doc_edit::DocLeave;
use crate::docs::{kind_segment, Docs, DocsState, DOC_TABS};
use crate::nav::NodeKind;
use crate::task::TaskDetail;
use crate::tree_rows::find_node;

/// Lo que es un enlace de un documento.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DocLink {
    /// Una tarea, por su id o por como se nombra fuera (`proteus-12`).
    Task { id: String },
    /// El documento de un nodo: su portada (en esa pestaña, si dice una) o
    /// una página suya, y dentro, si la dice, una sección.
    Doc { kind: NodeKind, id: String, tab: Option<String>, page: Option<String>, section: Option<String> },
    /// Un título de lo que se está leyendo (`#como-se-despliega`).
    Anchor { slug: String },
    /// Algo de fuera: se abre con lo que el sistema tenga para ello.
    Web { url: String },
    /// Nombra algo sin llevar a ningún sitio: una mención.
    Inert,
    /// De dentro, pero esta app no sabe abrirlo.
    Unknown,
}

/// Un enlace que no se pudo abrir, para decirlo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocLinkFailure {
    /// Cada fallo es otro, aunque sea del mismo enlace: con este número la
    /// pantalla sabe que tiene que volver a decirlo.
    pub serial: u64,
    pub href: String,
}

/// A qué parte de lo que se lee hay que saltar cuando esté pintado.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DocJump {
    /// Una sección por su id del servidor (`&section=`): renombrarla no le
    /// cambia el id, así que el enlace sigue llegando.
    Section { id: String },
    /// Un título por su ancla (`#que-es-esto`).
    Anchor { slug: String },
}

static FAILURES: AtomicU64 = AtomicU64::new(1);

/// `%C3%B3` → `ó`. Lo que no es un escape válido se deja como está.
fn percent_decoded(value: &str) -> String {
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

/// El valor de un parámetro de la consulta de un enlace (`a=1&b=2`), el
/// primero que haya con ese nombre.
fn param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| percent_decoded(value))
    })
}

fn node_kind(segment: &str) -> Option<NodeKind> {
    [NodeKind::Space, NodeKind::Folder, NodeKind::List].into_iter().find(|kind| kind_segment(*kind) == segment)
}

/// Qué es ese enlace.
pub fn doc_link(href: &str) -> DocLink {
    let href = href.trim();
    if let Some(rest) = href.strip_prefix("/tasks?") {
        // Lo que va tras `#` es un ancla, no parte del último valor.
        let query = rest.split('#').next().unwrap_or(rest);
        if let Some(id) = param(query, "task").filter(|id| !id.is_empty()) {
            return DocLink::Task { id };
        }
        let Some(doc) = param(query, "doc") else { return DocLink::Unknown };
        let Some((kind, id)) = doc.split_once(':') else { return DocLink::Unknown };
        let Some(kind) = node_kind(kind).filter(|_| !id.is_empty()) else { return DocLink::Unknown };
        let filled = |name: &str| param(query, name).filter(|value| !value.is_empty());
        return DocLink::Doc {
            kind,
            id: id.to_string(),
            // Una pestaña que no es de las cuatro no es ninguna.
            tab: filled("tab").filter(|tab| DOC_TABS.contains(&tab.as_str())),
            page: filled("page"),
            section: filled("section"),
        };
    }
    if let Some(slug) = href.strip_prefix('#') {
        return if slug.is_empty() { DocLink::Unknown } else { DocLink::Anchor { slug: percent_decoded(slug) } };
    }
    if crate::task::mentioned_user(href).is_some() {
        return DocLink::Inert;
    }
    let scheme = href.split_once(':').map(|(scheme, _)| scheme.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("http" | "https") if href.contains("://") => DocLink::Web { url: href.to_string() },
        Some("mailto") => DocLink::Web { url: href.to_string() },
        _ => DocLink::Unknown,
    }
}

/// Si eso es el id de una tarea y no su nombre de fuera: un uuid.
fn is_task_id(written: &str) -> bool {
    written.len() == 36 && written.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// «¿Qué es esto?» → `que-es-esto`: el ancla de un título, como la pone
/// `slugify` en `app/src/lib/headings.ts`. Sin acentos, para que
/// «Configuración» y «Configuracion» den la misma.
pub fn heading_slug(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut slug = String::new();
    for c in text.to_lowercase().nfd() {
        // Las marcas que la normalización separa de su letra se van.
        if ('\u{0300}'..='\u{036f}').contains(&c) {
            continue;
        }
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

impl DocsState {
    /// La página de **este** documento a la que lleva un enlace interno
    /// (`/tasks?doc=folder:f1&page=p2`), si lleva a una. Es lo que se puede
    /// abrir sin salir de lo que se lee.
    pub fn page_for_link(&self, link: &str) -> Option<String> {
        let target = self.target.as_ref()?;
        match doc_link(link) {
            DocLink::Doc { kind, id, page, .. } if kind == target.kind && id == target.id => page,
            _ => None,
        }
    }
}

impl Docs {
    fn say_link_failed(&self, href: &str) {
        let failure = DocLinkFailure { serial: FAILURES.fetch_add(1, Ordering::Relaxed), href: href.to_string() };
        self.update(|state| state.link_failed = Some(failure));
    }

    /// Quita el aviso de que un enlace no se pudo abrir.
    pub fn dismiss_link_failure(&self) {
        if self.state().link_failed.is_some() {
            self.update(|state| state.link_failed = None);
        }
    }

    /// Se llegó a donde había que saltar (o ya no hay a dónde).
    pub fn jump_done(&self) {
        if self.state().jump.is_some() {
            self.update(|state| state.jump = None);
        }
    }

    /// Sigue un enlace de un documento.
    ///
    /// - Una tarea se abre en su página, sobre lo que se lea. Lo que se
    ///   esté escribiendo sigue debajo: no se pierde nada, y no se pregunta.
    /// - Una página, una pestaña o una sección —de este documento o del de
    ///   otro nodo— se abre aquí. Si hay texto sin guardar, se pregunta
    ///   antes (`doc_edit`), y con un sí se sigue el enlace.
    /// - Un ancla salta a ese título de lo que se lee.
    /// - Lo de fuera y una mención no son de aquí: no hace nada (lo de
    ///   fuera lo abre la plataforma).
    /// - Lo demás, y lo que apunta a algo que no está, se dice.
    pub async fn follow_link(&self, href: &str) {
        self.dismiss_link_failure();
        match doc_link(href) {
            DocLink::Web { .. } | DocLink::Inert => {}
            DocLink::Unknown => self.say_link_failed(href),
            DocLink::Task { id } => self.open_linked_task(href, &id).await,
            DocLink::Anchor { slug } => {
                // Mientras se escribe no hay texto leído al que saltar.
                if self.state().edit.is_none() {
                    self.update(|state| state.jump = Some(DocJump::Anchor { slug }));
                }
            }
            DocLink::Doc { kind, id, tab, page, section } => {
                // Un nodo que no está en el árbol de esta organización (se
                // borró, o es de otra) no se abre: se dice antes de
                // preguntar nada.
                if find_node(&self.core.tree().state().spaces, &id).map(|(found, _)| found) != Some(kind) {
                    self.say_link_failed(href);
                    return;
                }
                if self.held_back(DocLeave::Link { href: href.to_string() }) {
                    return;
                }
                self.open_linked_doc(href, &id, tab, page, section).await;
            }
        }
    }

    async fn open_linked_task(&self, href: &str, written: &str) {
        if is_task_id(written) {
            self.core.tasks().open(written).await;
            return;
        }
        // Por su nombre de fuera: el servidor dice cuál es.
        let named: String = written.bytes().map(|byte| if byte.is_ascii_alphanumeric() || byte == b'-' { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect();
        match self.core.get::<TaskDetail>(&format!("/api/v1/tasks/by-ref/{named}")).await {
            Ok(detail) => self.core.tasks().open(&detail.task.id).await,
            Err(_) => self.say_link_failed(href),
        }
    }

    async fn open_linked_doc(&self, href: &str, id: &str, tab: Option<String>, page: Option<String>, section: Option<String>) {
        let on_it = |state: &DocsState| state.target.as_ref().map(|target| target.id.as_str()) == Some(id);
        if on_it(&self.state()) {
            // La página primero; sin página se vuelve a la portada, en la
            // pestaña que diga el enlace si dice alguna.
            match &page {
                Some(page) => self.open_page(page).await,
                None => {
                    self.close_page();
                    if let Some(tab) = &tab {
                        self.show_tab(tab);
                    }
                }
            }
        } else {
            self.open_at(id, tab.as_deref(), page.as_deref()).await;
        }
        let state = self.state();
        // Otro enlace, u otro clic, llegó antes que éste.
        if !on_it(&state) {
            return;
        }
        // La página que pedía ya no está en el documento: el árbol volvió a
        // la portada. Se dice, en vez de dejar a quien pulsó en otro sitio
        // sin saber por qué.
        if page.is_some() && state.page_id != page {
            self.say_link_failed(href);
            return;
        }
        if let Some(id) = section {
            self.update(|state| state.jump = Some(DocJump::Section { id }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(kind: NodeKind, id: &str, tab: Option<&str>, page: Option<&str>, section: Option<&str>) -> DocLink {
        let owned = |value: Option<&str>| value.map(str::to_string);
        DocLink::Doc { kind, id: id.into(), tab: owned(tab), page: owned(page), section: owned(section) }
    }

    /// Cada forma de enlace que la app y el servidor escriben se lee como lo
    /// que es.
    ///
    /// Roturas que atrapa: una tarea tomada por un documento (o al revés);
    /// perder la página, la pestaña o la sección de un enlace (abre la
    /// portada en «Resumen» diga lo que diga); aceptar una pestaña que no
    /// existe; una sección cuyo id se queda con el ancla pegada.
    #[test]
    fn each_shape_of_link_is_read_as_what_it_is() {
        assert_eq!(doc_link("/tasks?task=0d6bf322-2267-4259-9b81-337575aca792"), DocLink::Task { id: "0d6bf322-2267-4259-9b81-337575aca792".into() });
        assert_eq!(doc_link("/tasks?task=command-and-control-12"), DocLink::Task { id: "command-and-control-12".into() });
        assert_eq!(doc_link("/tasks?doc=space:s1"), doc(NodeKind::Space, "s1", None, None, None));
        assert_eq!(doc_link("/tasks?doc=folder:f1&tab=runbook"), doc(NodeKind::Folder, "f1", Some("runbook"), None, None));
        assert_eq!(doc_link("/tasks?doc=list:l1&page=p2"), doc(NodeKind::List, "l1", None, Some("p2"), None));
        assert_eq!(doc_link("/tasks?doc=list%3Al1&tab=runbook&section=s9#x"), doc(NodeKind::List, "l1", Some("runbook"), None, Some("s9")));
        assert_eq!(doc_link("/tasks?doc=space:s1&tab=inventada"), doc(NodeKind::Space, "s1", None, None, None));
        assert_eq!(doc_link("#como-se-despliega"), DocLink::Anchor { slug: "como-se-despliega".into() });
        assert_eq!(doc_link("#configuraci%C3%B3n"), DocLink::Anchor { slug: "configuración".into() });
    }

    /// Lo de fuera va fuera, una mención no va a ningún sitio, y lo de
    /// dentro que no se entiende no se le da al sistema.
    ///
    /// Roturas que atrapa: reclamar `https://ejemplo.com/tasks?task=1` como
    /// una tarea de aquí (deja de abrir el navegador); mandar al navegador
    /// una ruta de la app o un `cac:recording/…`, que fuera no abren nada;
    /// decir «no se pudo abrir» de una mención; un `/tasks?` sin nada, o un
    /// documento de una clase que no existe, abriendo algo.
    #[test]
    fn what_is_outside_goes_out_and_what_is_not_understood_is_not_guessed() {
        assert_eq!(doc_link("https://ejemplo.com/tasks?task=1"), DocLink::Web { url: "https://ejemplo.com/tasks?task=1".into() });
        assert_eq!(doc_link(" HTTP://ejemplo.com "), DocLink::Web { url: "HTTP://ejemplo.com".into() });
        assert_eq!(doc_link("mailto:ana@example.com"), DocLink::Web { url: "mailto:ana@example.com".into() });
        assert_eq!(doc_link("cac:user/11111111-1111-4111-8111-111111111111"), DocLink::Inert);
        for inside in [
            "cac:recording/11111111-1111-4111-8111-111111111111",
            "/notes/n1",
            "/api/v1/docs/space/s1/attachments/a1",
            "/tasks?",
            "/tasks?task=",
            "/tasks?doc=project:p1",
            "/tasks?doc=space:",
            "/tasks?doc=space",
            "#",
            "otra-pagina.md",
            "javascript:alert(1)",
            "http:sin-barras",
            "",
        ] {
            assert_eq!(doc_link(inside), DocLink::Unknown, "{inside:?}");
        }
    }

    /// El ancla de un título es la que pone la app de siempre.
    ///
    /// Roturas que atrapa: un acento que cambia el ancla (el índice de un
    /// documento apuntaría a un título que no existe); guiones repetidos o
    /// en las puntas.
    #[test]
    fn a_heading_has_the_anchor_the_other_app_gives_it() {
        assert_eq!(heading_slug("¿Qué es esto?"), "que-es-esto");
        assert_eq!(heading_slug("Configuración"), heading_slug("Configuracion"));
        assert_eq!(heading_slug("  Paso 2: `make`  — ya  "), "paso-2-make-ya");
        assert_eq!(heading_slug("ñandú"), "nandu");
        assert_eq!(heading_slug("¿?"), "");
    }

    /// Un uuid es el id de una tarea; lo demás es su nombre de fuera.
    ///
    /// Rotura que atrapa: pedir `/tasks/proteus-12` como si fuera un id
    /// (404), o buscar un uuid por nombre.
    #[test]
    fn a_task_is_told_by_id_or_by_name() {
        assert!(is_task_id("0d6bf322-2267-4259-9b81-337575aca792"));
        assert!(!is_task_id("command-and-control-12"));
        assert!(!is_task_id("0d6bf322-2267-4259-9b81-337575aca79"));
    }
}
