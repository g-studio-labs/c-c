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

    /// Revoca un enlace: deja de abrir la página al momento.
    pub async fn revoke_share(&self, id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("/api/v1/doc-shares/{id}"), None).await
    }

    /// El enlace web de ese enlace vivo, a este servidor.
    pub fn link(&self, share: &DocShare) -> String {
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
}
