//! Compartir una página de un documento —con todo lo que cuelga de ella—
//! por un enlace firmado que caduca y se puede revocar.
//!
//! El enlace abre la página **en la web** (`/app/shared`), sin cuenta. El
//! token va detrás del `#`, como en `guest_link`: lo que va en la ruta o en
//! la consulta acaba en los logs de cualquier proxy. La caducidad y la
//! revocación viven en el servidor, no en el token: por eso un enlace vivo se
//! puede volver a copiar cuando haga falta.

use serde::{Deserialize, Serialize};

use crate::docs::kind_segment;
use crate::error::ApiError;
use crate::nav::NodeKind;
use crate::Core;

/// Cuánto puede valer un enlace, con la clave de cómo se dice. Lo decidió
/// jose: un día, una semana o un mes, y nada más.
pub const DOC_SHARE_TTLS: [(u32, &str); 3] = [(24, "work:docShare.ttl24"), (168, "work:docShare.ttl168"), (720, "work:docShare.ttl720")];

/// Un enlace vivo a una página. Espejo de `domain.DocShareResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocShare {
    pub id: String,
    #[serde(default)]
    pub page_id: String,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub created_by_name: String,
    /// El token del enlace. Sólo lo ve un miembro.
    #[serde(default)]
    pub link: String,
}

/// Un enlace vivo de la organización, con la página y el espacio ya
/// resueltos por el servidor. Espejo de `domain.OrgDocShareResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct OrgDocShare {
    pub id: String,
    #[serde(default)]
    pub page_id: String,
    #[serde(default)]
    pub page_title: String,
    /// `space`, `folder` o `list`: la clase del nodo del que cuelga el doc.
    #[serde(default)]
    pub doc_kind: String,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub owner_name: String,
    /// Vacío si el nodo del doc ya no existe: el enlace sigue abriendo.
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub space_name: String,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub created_by_name: String,
    /// `read` hasta que haya enlaces de responder.
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub response_count: u32,
    /// El token del enlace. Sólo lo ve un miembro.
    #[serde(default)]
    pub link: String,
}

impl OrgDocShare {
    /// El enlace de cac que abre esta página en la app, por el nodo del que
    /// cuelga su documento.
    pub fn page_route(&self) -> String {
        format!("cac://tasks?doc={}:{}&page={}", self.doc_kind, self.owner_id, self.page_id)
    }
}

/// Los espacios que salen en una lista de enlaces, sin repetir y con el
/// primero en aparecer primero. Un enlace sin espacio no cuenta.
pub fn share_spaces(shares: &[OrgDocShare]) -> Vec<(String, String)> {
    let mut spaces: Vec<(String, String)> = Vec::new();
    for share in shares.iter().filter(|share| !share.space_id.is_empty()) {
        if !spaces.iter().any(|(id, _)| *id == share.space_id) {
            spaces.push((share.space_id.clone(), share.space_name.clone()));
        }
    }
    spaces
}

/// Los enlaces de ese espacio; sin espacio, todos.
pub fn shares_in_space<'a>(shares: &'a [OrgDocShare], space: Option<&str>) -> Vec<&'a OrgDocShare> {
    shares.iter().filter(|share| space.is_none_or(|space| share.space_id == space)).collect()
}

fn org_shares_path(org_id: &str, space: Option<&str>) -> String {
    let base = format!("/api/v1/organizations/{org_id}/doc-shares");
    match space.filter(|space| !space.is_empty()) {
        Some(space) => format!("{base}?space={}", encoded(space)),
        None => base,
    }
}

fn encoded(value: &str) -> String {
    value.bytes().map(|byte| if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect()
}

/// El enlace web de una página compartida, en este servidor.
pub fn shared_doc_link(base_url: &str, token: &str) -> String {
    format!("{}/app/shared#{token}", base_url.trim_end_matches('/'))
}

fn shares_path(kind: NodeKind, owner_id: &str, page_id: &str) -> String {
    format!("/api/v1/docs/{}/{owner_id}/pages/{page_id}/shares", kind_segment(kind))
}

/// Los enlaces públicos de las páginas, con la sesión de este core.
pub struct DocShares {
    core: Core,
}

impl Core {
    pub fn doc_shares(&self) -> DocShares {
        DocShares { core: self.clone() }
    }
}

impl DocShares {
    /// Crea un enlace a esa página que vale `ttl_hours` horas. La página se
    /// nombra como en el resto de rutas de páginas: por el nodo del que
    /// cuelga el documento.
    pub async fn create_share(&self, kind: NodeKind, owner_id: &str, page_id: &str, ttl_hours: u32) -> Result<DocShare, ApiError> {
        let body = serde_json::json!({ "ttlHours": ttl_hours });
        self.core.call("POST", &shares_path(kind, owner_id, page_id), Some(body)).await
    }

    /// Los enlaces vivos de esa página.
    pub async fn list_shares(&self, kind: NodeKind, owner_id: &str, page_id: &str) -> Result<Vec<DocShare>, ApiError> {
        self.core.get_list(&shares_path(kind, owner_id, page_id)).await
    }

    /// Todos los enlaces vivos de la organización (los que caducan antes,
    /// primero), o sólo los de un espacio.
    pub async fn list_org_shares(&self, org_id: &str, space: Option<&str>) -> Result<Vec<OrgDocShare>, ApiError> {
        self.core.get_list(&org_shares_path(org_id, space)).await
    }

    /// Revoca un enlace: deja de abrir la página al momento.
    pub async fn revoke_share(&self, id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("/api/v1/doc-shares/{id}"), None).await
    }

    /// El enlace web de ese enlace vivo, a este servidor.
    pub fn link(&self, share: &DocShare) -> String {
        shared_doc_link(self.core.base_url(), &share.link)
    }

    /// Lo mismo para un enlace de la lista de la organización.
    pub fn org_link(&self, share: &OrgDocShare) -> String {
        shared_doc_link(self.core.base_url(), &share.link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El enlace va a la página pública de la web, con el token detrás del
    /// `#`.
    ///
    /// Roturas que atrapa: el token en la ruta o en la consulta, donde acaba
    /// en los logs; una barra doble con una base que termina en `/`; el
    /// enlace de las llamadas (`/app/join`) en vez del de los documentos.
    #[test]
    fn the_shared_link_goes_to_the_web_with_the_token_after_the_hash() {
        assert_eq!(shared_doc_link("https://cac.guz-studio.dev/", "s1.firma"), "https://cac.guz-studio.dev/app/shared#s1.firma");
    }

    fn share(id: &str, space: &str, name: &str) -> OrgDocShare {
        OrgDocShare { id: id.into(), space_id: space.into(), space_name: name.into(), ..Default::default() }
    }

    /// La página se abre en la app por el nodo de su documento, no por el
    /// espacio.
    ///
    /// Roturas que atrapa: abrir por el espacio en vez del dueño; perder la
    /// página (se abriría la portada del doc); la clase del nodo cambiada.
    #[test]
    fn a_shared_page_opens_in_the_app_by_its_owner() {
        let share = OrgDocShare { doc_kind: "folder".into(), owner_id: "f1".into(), page_id: "p9".into(), space_id: "s1".into(), ..Default::default() };
        assert_eq!(share.page_route(), "cac://tasks?doc=folder:f1&page=p9");
    }

    /// Los chips de espacio salen una vez cada uno, en orden de aparición, y
    /// un enlace sin espacio no inventa uno; filtrar deja sólo ese espacio.
    ///
    /// Roturas que atrapa: un espacio repetido por cada enlace; un chip
    /// vacío para el nodo borrado; el filtro que deja pasar otros espacios o
    /// que, sin espacio elegido, quita los enlaces sin espacio.
    #[test]
    fn the_space_chips_are_each_space_once_and_the_filter_keeps_only_that_space() {
        let shares = [share("a", "s1", "Producto"), share("b", "s2", "Galena"), share("c", "s1", "Producto"), share("d", "", "")];
        assert_eq!(share_spaces(&shares), [("s1".to_string(), "Producto".to_string()), ("s2".to_string(), "Galena".to_string())]);
        let ids = |space| shares_in_space(&shares, space).iter().map(|share| share.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(Some("s1")), ["a", "c"]);
        assert_eq!(ids(None), ["a", "b", "c", "d"]);
    }

    /// El espacio va en la consulta, codificado; sin espacio no hay consulta.
    ///
    /// Roturas que atrapa: la ruta de `/orgs/`, que no existe; el espacio sin
    /// codificar; una consulta vacía (`?space=`) cuando no se filtra.
    #[test]
    fn the_org_list_asks_the_organizations_route_with_the_space_encoded() {
        assert_eq!(org_shares_path("o1", None), "/api/v1/organizations/o1/doc-shares");
        assert_eq!(org_shares_path("o1", Some("")), "/api/v1/organizations/o1/doc-shares");
        assert_eq!(org_shares_path("o1", Some("a b&c")), "/api/v1/organizations/o1/doc-shares?space=a%20b%26c");
    }
}
