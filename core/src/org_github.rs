//! La GitHub App de la organización (`OrgGitHub.tsx`): qué cuentas de GitHub
//! la tienen instalada y a qué espacio va cada repo.
//!
//! Un repo enlazado a un espacio deja una línea en la tarea que nombre cada
//! commit o PR. Las líneas son internas: el cliente no lee los mensajes de
//! commit del equipo. Un repo sin enlazar no hace nada, y así empieza cada uno.

use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::inbox::time_ago;
use crate::orgs::Orgs;
use crate::texts::{text, text_with, Locale};
use crate::tree::SpaceTree;

/// Una instalación de la GitHub App atada a esta organización.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct GitHubInstallation {
    pub id: String,
    pub installation_id: i64,
    #[serde(default)]
    pub account_login: String,
}

impl GitHubInstallation {
    /// Cómo se le llama: su cuenta, y si no la dijo, su número.
    pub fn label(&self) -> String {
        if self.account_login.is_empty() { self.installation_id.to_string() } else { self.account_login.clone() }
    }
}

/// Una instalación que llegó por el webhook y ninguna organización ató: sólo
/// la ve un superadmin en persona.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct GitHubUnbound {
    pub installation_id: i64,
    #[serde(default)]
    pub account_login: String,
    #[serde(default)]
    pub repos: Vec<String>,
}

impl GitHubUnbound {
    pub fn label(&self) -> String {
        if self.account_login.is_empty() { self.installation_id.to_string() } else { self.account_login.clone() }
    }
}

/// Un repo de una instalación. `space_id` vacío es «sin enlazar»: no comenta
/// nada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct GitHubRepo {
    pub id: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub bare_refs: bool,
    /// Si sus .md salen en Documentación, bajo su espacio.
    #[serde(default)]
    pub show_docs: bool,
    #[serde(default)]
    pub docs_synced_at: Option<String>,
    /// `ok`, `truncated` o `error`; vacío, nunca.
    #[serde(default)]
    pub docs_state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct GitHubStatus {
    /// Si el servidor tiene la App puesta. Sin ella no hay nada que conectar.
    #[serde(default)]
    pub configured: bool,
    #[serde(default)]
    pub installations: Vec<GitHubInstallation>,
    #[serde(default)]
    pub repos: Vec<GitHubRepo>,
    #[serde(default)]
    pub unbound: Vec<GitHubUnbound>,
}

/// Lo que cambia en un repo; lo que no se nombra se queda como estaba.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoChange {
    pub space_id: Option<String>,
    pub bare_refs: Option<bool>,
    pub show_docs: Option<bool>,
}

impl RepoChange {
    /// Lo que ya estaba por mandarse, con esto encima: dos clics seguidos en
    /// casillas distintas del mismo repo son un solo cambio, no uno que pisa
    /// al otro.
    pub fn merged_with(mut self, later: RepoChange) -> RepoChange {
        self.space_id = later.space_id.or(self.space_id);
        self.bare_refs = later.bare_refs.or(self.bare_refs);
        self.show_docs = later.show_docs.or(self.show_docs);
        self
    }

    /// El cuerpo del PATCH: el servidor reemplaza, así que va el repo entero
    /// con lo cambiado encima.
    fn body(&self, repo: &GitHubRepo) -> serde_json::Value {
        serde_json::json!({
            "spaceId": self.space_id.clone().unwrap_or_else(|| repo.space_id.clone()),
            "bareRefs": self.bare_refs.unwrap_or(repo.bare_refs),
            "showDocs": self.show_docs.unwrap_or(repo.show_docs),
        })
    }
}

/// Los espacios a los que se puede enlazar un repo: los de **esta**
/// organización. La sala general no tiene tareas, y la de otra organización
/// sería enseñarle a un cliente los commits de otro.
pub fn linkable_spaces(spaces: &[SpaceTree], org_id: &str) -> Vec<(String, String)> {
    spaces.iter().filter(|space| space.org_id == org_id && !space.is_general()).map(|space| (space.id.clone(), space.name.clone())).collect()
}

/// `#12` a secas no se puede encender en un repo sin enlazar: ¿de qué espacio
/// sería el 12?
pub fn bare_refs_enabled(repo: &GitHubRepo, can_manage: bool) -> bool {
    can_manage && !repo.space_id.is_empty()
}

/// Los repos que casan con lo escrito. Una cuenta entera trae decenas: los
/// atados a un espacio van primero, que son los que se vienen a tocar.
pub fn shown_repos(repos: &[GitHubRepo], query: &str) -> Vec<GitHubRepo> {
    let query = query.trim().to_lowercase();
    let mut shown: Vec<GitHubRepo> = repos.iter().filter(|repo| query.is_empty() || repo.full_name.to_lowercase().contains(&query)).cloned().collect();
    shown.sort_by(|a, b| a.space_id.is_empty().cmp(&b.space_id.is_empty()).then_with(|| a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase())));
    shown
}

/// La línea de cómo va la copia de sus docs.
pub fn docs_line<Viewer: TimeZone>(repo: &GitHubRepo, now: &DateTime<Viewer>, locale: Locale) -> String {
    match (repo.docs_state.as_str(), repo.docs_synced_at.as_deref()) {
        ("truncated", _) => text(locale, "work:repoDocs.state.truncated"),
        ("error", _) => text(locale, "work:repoDocs.state.error"),
        (_, Some(at)) => text_with(locale, "work:repoDocs.synced", &[("when", &time_ago(at, now, locale))]),
        _ => text(locale, "work:repoDocs.never"),
    }
}

impl Orgs {
    fn github_base(org_id: &str) -> String {
        format!("/api/v1/organizations/{org_id}/github")
    }

    pub async fn github_status(&self, org_id: &str) -> Result<GitHubStatus, ApiError> {
        self.core.get(&format!("{}/", Self::github_base(org_id))).await
    }

    /// La dirección de GitHub donde se instala la App, con el `state` que ata
    /// la instalación a esta organización.
    pub async fn github_link(&self, org_id: &str) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        struct Link {
            url: String,
        }
        let link: Link = self.core.call("POST", &format!("{}/link", Self::github_base(org_id)), Some(serde_json::json!({}))).await?;
        Ok(link.url)
    }

    /// Devuelve el repo como quedó.
    pub async fn github_update_repo(&self, org_id: &str, repo: &GitHubRepo, change: &RepoChange) -> Result<GitHubRepo, ApiError> {
        self.core.call("PATCH", &format!("{}/repos/{}", Self::github_base(org_id), repo.id), Some(change.body(repo))).await
    }

    /// Ata a esta organización una instalación sin dueña. Una vez atada no se
    /// la lleva otra.
    pub async fn github_adopt(&self, org_id: &str, installation_id: i64) -> Result<(), ApiError> {
        self.core.call_ok("POST", &format!("{}/installations/{installation_id}/adopt", Self::github_base(org_id)), Some(serde_json::json!({}))).await
    }

    /// Los docs del repo, a mano: por si se perdió un push.
    pub async fn github_sync_docs(&self, org_id: &str, repo_id: &str) -> Result<GitHubRepo, ApiError> {
        self.core.call("POST", &format!("{}/repos/{repo_id}/docs/sync", Self::github_base(org_id)), Some(serde_json::json!({}))).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn repo(name: &str, space: &str) -> GitHubRepo {
        GitHubRepo { id: name.into(), full_name: name.into(), space_id: space.into(), bare_refs: false, show_docs: false, docs_synced_at: None, docs_state: String::new() }
    }

    /// Roturas que atrapa: el filtro sin efecto (o que distingue
    /// mayúsculas); los atados sin ir primero; un empate sin orden por
    /// nombre.
    #[test]
    fn repos_are_found_by_name_and_the_linked_ones_come_first() {
        let all = [repo("dwit/aaa", ""), repo("dwit/zzz", "sp-1"), repo("dwit/mmm", ""), repo("dwit/bbb", "sp-1")];
        let names = |found: Vec<GitHubRepo>| found.into_iter().map(|r| r.full_name).collect::<Vec<_>>();
        assert_eq!(names(shown_repos(&all, "")), ["dwit/bbb", "dwit/zzz", "dwit/aaa", "dwit/mmm"]);
        assert_eq!(names(shown_repos(&all, " MM ")), ["dwit/mmm"]);
        assert!(shown_repos(&all, "nada").is_empty());
    }

    /// Roturas que atrapa: ofrecer la sala general o un espacio de otra
    /// organización; encender `#12` en un repo sin enlazar o a quien no
    /// administra.
    #[test]
    fn only_this_organizations_spaces_with_tasks_are_offered() {
        let space = |id: &str, org: &str, kind: Option<&str>| -> SpaceTree {
            serde_json::from_value(serde_json::json!({ "id": id, "orgId": org, "name": id, "kind": kind })).unwrap()
        };
        let offered = linkable_spaces(&[space("a", "o1", None), space("g", "o1", Some("general")), space("x", "o2", None)], "o1");
        assert_eq!(offered, [("a".to_string(), "a".to_string())]);
        assert!(!bare_refs_enabled(&repo("r", ""), true));
        assert!(!bare_refs_enabled(&repo("r", "sp-1"), false));
        assert!(bare_refs_enabled(&repo("r", "sp-1"), true));
    }

    /// Roturas que atrapa: un cambio que arrastra el resto en falso (apagar
    /// los docs al cambiar el espacio); mandar sólo lo cambiado, que el
    /// servidor reemplaza.
    #[test]
    fn a_change_goes_with_the_rest_of_the_repo() {
        let linked = GitHubRepo { bare_refs: true, show_docs: true, ..repo("dwit/api", "sp-1") };
        assert_eq!(RepoChange { space_id: Some("sp-2".into()), ..Default::default() }.body(&linked), serde_json::json!({ "spaceId": "sp-2", "bareRefs": true, "showDocs": true }));
        assert_eq!(RepoChange { show_docs: Some(false), ..Default::default() }.body(&linked), serde_json::json!({ "spaceId": "sp-1", "bareRefs": true, "showDocs": false }));
    }

    /// Roturas que atrapa: decir «sincronizado» cuando faltaron ficheros o la
    /// última pasada falló; no decir «sin sincronizar» al repo nuevo.
    #[test]
    fn the_docs_line_says_how_the_copy_went() {
        let now = Utc::now();
        let at = (now - chrono::Duration::hours(3)).to_rfc3339();
        let synced = GitHubRepo { docs_state: "ok".into(), docs_synced_at: Some(at.clone()), ..repo("r", "sp") };
        assert_eq!(docs_line(&synced, &now, Locale::En), "Synced 3 hours ago");
        assert_eq!(docs_line(&GitHubRepo { docs_state: "truncated".into(), ..synced.clone() }, &now, Locale::En), "Too many files: only the first 1000 are shown");
        assert_eq!(docs_line(&GitHubRepo { docs_state: "error".into(), ..synced }, &now, Locale::En), "The last sync failed");
        assert_eq!(docs_line(&repo("r", "sp"), &now, Locale::En), "Not synced yet");
    }

    /// Rotura que atrapa: que el segundo clic borre al primero (o que el
    /// primero gane sobre el que vino después).
    #[test]
    fn two_quick_changes_become_one_with_the_later_winning() {
        let first = RepoChange { bare_refs: Some(true), show_docs: Some(false), ..Default::default() };
        let later = RepoChange { show_docs: Some(true), ..Default::default() };
        assert_eq!(first.merged_with(later), RepoChange { space_id: None, bare_refs: Some(true), show_docs: Some(true) });
    }
}
