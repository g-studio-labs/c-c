//! Los tokens personales: qué permisos puede llevar uno y crearlo.
//!
//! La lista de permisos es la de `app/src/components/mcp-scopes.tsx`, en el
//! mismo orden, y cada `id` tiene que ser una constante de
//! `backend/internal/core/domain/token.go`: `ValidScope` tira en silencio lo
//! que no reconoce, y un permiso con una errata daría un token que no puede
//! lo que dice. Lo guarda `tokens::every_scope_is_one_the_server_knows`.
//!
//! El valor del token sólo existe en la respuesta de crearlo: el servidor no
//! lo guarda. Aquí va envuelto en [`TokenValue`], que no se deja imprimir.

use serde::Deserialize;

use crate::error::ApiError;
use crate::Core;

/// Un permiso que se le puede dar a un token, con sus claves del catálogo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenScope {
    pub id: &'static str,
    pub label_key: &'static str,
    pub detail_key: &'static str,
}

const fn scope(id: &'static str, label_key: &'static str, detail_key: &'static str) -> TokenScope {
    TokenScope { id, label_key, detail_key }
}

pub const TOKEN_SCOPES: &[TokenScope] = &[
    scope("tasks:write", "common:scopes.tasksWrite", "common:scopes.tasksWriteDetail"),
    scope("tasks:manage", "common:scopes.tasksManage", "common:scopes.tasksManageDetail"),
    scope("notes:write", "common:scopes.notesWrite", "common:scopes.notesWriteDetail"),
    scope("notes:manage", "common:scopes.notesManage", "common:scopes.notesManageDetail"),
    scope("reports:write", "common:scopes.reportsWrite", "common:scopes.reportsWriteDetail"),
    scope("reports:manage", "common:scopes.reportsManage", "common:scopes.reportsManageDetail"),
    scope("docs:write", "common:scopes.docsWrite", "common:scopes.docsWriteDetail"),
    scope("docs:manage", "common:scopes.docsManage", "common:scopes.docsManageDetail"),
    scope("docs:share", "common:scopes.docsShare", "common:scopes.docsShareDetail"),
    scope("collections:write", "common:scopes.collectionsWrite", "common:scopes.collectionsWriteDetail"),
    scope("deploys:manage", "common:scopes.deploysManage", "common:scopes.deploysManageDetail"),
    scope("servers:manage", "common:scopes.serversManage", "common:scopes.serversManageDetail"),
    scope("notify:me", "common:scopes.notifyMe", "common:scopes.notifyMeDetail"),
    scope("posts:write", "common:scopes.postsWrite", "common:scopes.postsWriteDetail"),
    scope("chat:write", "common:scopes.chatWrite", "common:scopes.chatWriteDetail"),
];

/// Si están marcados todos los permisos que se pueden dar.
pub fn all_scopes_selected(selected: &[String]) -> bool {
    TOKEN_SCOPES.iter().all(|scope| selected.iter().any(|id| id == scope.id))
}

/// «Seleccionar todos»: con todos marcados, los desmarca; si falta alguno,
/// los marca todos, en el orden de la lista.
pub fn toggle_all_scopes(selected: &[String]) -> Vec<String> {
    if all_scopes_selected(selected) {
        Vec::new()
    } else {
        TOKEN_SCOPES.iter().map(|scope| scope.id.to_string()).collect()
    }
}

/// Marca o desmarca uno, sin repetirlo.
pub fn toggle_scope(selected: &[String], id: &str, on: bool) -> Vec<String> {
    let mut next: Vec<String> = selected.iter().filter(|known| *known != id).cloned().collect();
    if on {
        next.push(id.to_string());
    }
    next
}

/// El valor de un token. No se imprime con `{:?}`: un `log` o un mensaje de
/// error que lo arrastre dice `TokenValue(…)` y no la credencial.
#[derive(Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(transparent)]
pub struct TokenValue(String);

impl TokenValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// El valor, para quien tiene que usarlo (el comando que registra, el
    /// «Copiar» que pulsa la persona). Con nombre largo a propósito.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for TokenValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenValue(…)")
    }
}

/// Un token recién creado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedToken {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub value: TokenValue,
}

#[derive(Deserialize, Default)]
struct CreatedWire {
    #[serde(default)]
    token: TokenWire,
    #[serde(default)]
    value: TokenValue,
}

#[derive(Deserialize, Default)]
struct TokenWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

/// El nombre que lleva un token si no se escribe otro, como en la otra app.
pub const DEFAULT_TOKEN_NAME: &str = "Claude Code";

impl Core {
    /// Crea un token personal de quien está dentro, con esos permisos.
    ///
    /// Un permiso que la app no conoce no se manda: el servidor lo tiraría
    /// en silencio y el token diría menos de lo que se marcó.
    pub async fn create_personal_token(&self, name: &str, scopes: &[String]) -> Result<CreatedToken, ApiError> {
        let name = match name.trim() {
            "" => DEFAULT_TOKEN_NAME,
            name => name,
        };
        let scopes: Vec<&str> = TOKEN_SCOPES.iter().map(|scope| scope.id).filter(|id| scopes.iter().any(|picked| picked == id)).collect();
        let body = serde_json::json!({ "name": name, "scopes": scopes });
        let reply = self.request("POST", "/api/v1/auth/tokens", Some(body.to_string()), true).await?;
        if reply.status >= 400 {
            return Err(crate::session::server_error(&reply));
        }
        let created: CreatedWire = crate::session::data_or_default(&reply);
        // Sin valor no hay nada que registrar ni que enseñar: se dice que
        // falló en vez de dar un token vacío por bueno.
        if created.value.expose().is_empty() {
            return Err(ApiError::Server { status: reply.status, code: "token-missing".into(), message: "the server did not return the token".into() });
        }
        Ok(CreatedToken { id: created.token.id, name: created.token.name, scopes: created.token.scopes.unwrap_or_default(), value: created.value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    /// «Seleccionar todos» marca todos los permisos, y pulsado otra vez los
    /// desmarca; con alguno ya marcado, completa.
    ///
    /// Roturas que atrapa: marcar sólo los que faltaban dejándose alguno;
    /// no desmarcar al volver a pulsar; tomar «uno marcado» por «todos».
    #[test]
    fn select_all_marks_every_scope_and_then_clears_them() {
        let all = toggle_all_scopes(&[]);
        assert_eq!(all.len(), TOKEN_SCOPES.len());
        assert!(all_scopes_selected(&all));
        assert!(TOKEN_SCOPES.iter().all(|scope| all.iter().any(|id| id == scope.id)));
        assert!(toggle_all_scopes(&all).is_empty(), "pulsado con todos marcados, no los desmarca");
        let some = ids(&["tasks:write"]);
        assert!(!all_scopes_selected(&some));
        assert_eq!(toggle_all_scopes(&some).len(), TOKEN_SCOPES.len());
    }

    /// Marcar uno ya marcado no lo repite, y desmarcarlo lo quita.
    #[test]
    fn toggling_one_scope_never_repeats_it() {
        let once = toggle_scope(&[], "docs:write", true);
        assert_eq!(toggle_scope(&once, "docs:write", true), ids(&["docs:write"]));
        assert!(toggle_scope(&once, "docs:write", false).is_empty());
    }

    /// Cada permiso de la lista es uno que el servidor reconoce, y el
    /// servidor no tiene ninguno que la lista no ofrezca.
    ///
    /// Roturas que atrapa: una errata en un `id` (el servidor lo tira en
    /// silencio); un permiso nuevo en el backend que la nativa no deja dar
    /// («Seleccionar todos» no sería todos).
    #[test]
    fn every_scope_is_one_the_server_knows() {
        let go = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../backend/internal/core/domain/token.go")).unwrap();
        let mut server: Vec<String> = go
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix("Scope")?;
                let (_, value) = rest.split_once("= \"")?;
                Some(value.trim_end_matches('"').to_string())
            })
            .collect();
        server.sort();
        let mut ours: Vec<String> = TOKEN_SCOPES.iter().map(|scope| scope.id.to_string()).collect();
        ours.sort();
        assert!(!server.is_empty(), "no se encontraron las constantes Scope… en token.go");
        assert_eq!(ours, server);
    }

    /// Ningún `{:?}` enseña el valor de un token.
    ///
    /// Rotura que atrapa: un `#[derive(Debug)]` en `TokenValue`, que haría
    /// que un `log::warn!("{created:?}")` escribiera la credencial.
    #[test]
    fn a_token_value_never_prints() {
        let created = CreatedToken { id: "t1".into(), name: "Claude Code".into(), scopes: vec![], value: TokenValue::new("cac_pat_SECRET") };
        assert!(!format!("{created:?}").contains("SECRET"));
        assert!(!format!("{:#?}", created.value).contains("SECRET"));
    }
}
