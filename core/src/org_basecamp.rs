//! Basecamp en la organización (Organización → Basecamp): conectar la cuenta
//! por OAuth, elegir qué proyectos de Basecamp van a qué espacio y asignar a
//! mano las personas que el servidor no pudo casar por correo.
//!
//! El servidor hace todo el trabajo (OAuth, webhooks, casar por correo); la
//! app sólo lo pide y dice cómo fue. Lo que se ofrece y a quién lo decide
//! este módulo, para que las apps no lo vuelvan a decidir cada una.

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::org_admin::{member_face, OrgMember};
use crate::orgs::Orgs;

/// Un proyecto de Basecamp ya enlazado a un espacio de cac.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasecampLinked {
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub project_id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub app_url: String,
    /// 0: el servidor no pudo dar de alta el webhook, y sin él Basecamp no
    /// avisa de nada.
    #[serde(default)]
    pub webhook_id: i64,
}

impl BasecampLinked {
    pub fn has_webhook(&self) -> bool {
        self.webhook_id != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasecampStatus {
    /// Si el servidor tiene la app de 37signals. Sin ella no hay nada que
    /// conectar.
    #[serde(default)]
    pub configured: bool,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub account_name: String,
    #[serde(default)]
    pub identity_name: String,
    /// Desde cuándo falla renovar el permiso: hay que volver a conectar.
    #[serde(default)]
    pub refresh_failed_at: Option<String>,
    #[serde(default)]
    pub projects: Vec<BasecampLinked>,
    /// Cuántas personas de Basecamp siguen sin usuario de cac.
    #[serde(default)]
    pub unmatched: i64,
}

impl BasecampStatus {
    /// Conectar se ofrece a quien administra, con la app puesta en el
    /// servidor y mientras no haya una cuenta: con una conectada, lo que hay
    /// que ofrecer es desconectar, no conectar otra encima.
    pub fn offers_connect(&self, can_manage: bool) -> bool {
        can_manage && self.configured && !self.connected
    }

    pub fn offers_disconnect(&self, can_manage: bool) -> bool {
        can_manage && self.connected
    }

    /// El permiso ya no se renueva: lo conectado sigue a la vista pero no
    /// llega nada, y la salida es desconectar y volver a conectar.
    pub fn needs_reconnect(&self) -> bool {
        self.connected && self.refresh_failed_at.is_some()
    }
}

/// Un proyecto de la cuenta de Basecamp, enlazado o no.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasecampAvailable {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub app_url: String,
    /// Vacío: sin enlazar.
    #[serde(default)]
    pub space_id: String,
}

/// Una persona de la cuenta de Basecamp y, si casó, su usuario de cac.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasecampPerson {
    pub person_id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    /// De la empresa cliente: lo normal es que no case con nadie del equipo.
    #[serde(default)]
    pub client: bool,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub matched_by: String,
}

/// Los proyectos de la cuenta con los enlazados primero, que son los que se
/// vienen a tocar, y por nombre dentro de cada grupo.
pub fn ordered_projects(projects: &[BasecampAvailable]) -> Vec<BasecampAvailable> {
    let mut ordered = projects.to_vec();
    ordered.sort_by(|a, b| a.space_id.is_empty().cmp(&b.space_id.is_empty()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    ordered
}

/// Las personas que esperan a que un admin las asigne: las del equipo antes
/// que las de la empresa cliente, que casi nunca tienen usuario de cac.
pub fn unmatched_people(people: &[BasecampPerson]) -> Vec<BasecampPerson> {
    let mut waiting: Vec<BasecampPerson> = people.iter().filter(|person| person.user_id.is_empty()).cloned().collect();
    waiting.sort_by(|a, b| a.client.cmp(&b.client).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    waiting
}

/// Los miembros a quien se puede asignar una persona, buscando por cómo se
/// les llama (su nombre visible, no su usuario) o por su correo. Quien no
/// terminó el registro se encuentra por su correo, que es lo que se le ve.
pub fn assignable_members(members: &[OrgMember], query: &str) -> Vec<OrgMember> {
    let query = query.trim().to_lowercase();
    let mut found: Vec<OrgMember> = members
        .iter()
        .filter(|member| {
            let (name, _) = member_face(member);
            query.is_empty() || name.to_lowercase().contains(&query) || member.email.as_deref().unwrap_or("").to_lowercase().contains(&query)
        })
        .cloned()
        .collect();
    found.sort_by_key(|member| member_face(member).0.to_lowercase());
    found
}

impl Orgs {
    fn basecamp_base(org_id: &str) -> String {
        format!("/api/v1/organizations/{org_id}/basecamp")
    }

    pub async fn basecamp_status(&self, org_id: &str) -> Result<BasecampStatus, ApiError> {
        self.core.get(&Self::basecamp_base(org_id)).await
    }

    /// La dirección de Launchpad donde se autoriza, que se abre en el
    /// navegador. Con el servidor sin la app contesta `basecamp-off`.
    pub async fn basecamp_connect_url(&self, org_id: &str) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        struct Link {
            url: String,
        }
        let link: Link = self.core.call("POST", &format!("{}/connect", Self::basecamp_base(org_id)), Some(serde_json::json!({}))).await?;
        Ok(link.url)
    }

    /// Quita los webhooks y los proyectos enlazados; las personas se quedan.
    pub async fn basecamp_disconnect(&self, org_id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &Self::basecamp_base(org_id), None).await
    }

    pub async fn basecamp_projects(&self, org_id: &str) -> Result<Vec<BasecampAvailable>, ApiError> {
        self.core.get_list(&format!("{}/projects", Self::basecamp_base(org_id))).await
    }

    /// Enlaza un proyecto a un espacio de esta organización (o lo cambia de
    /// espacio). Devuelve el enlace como quedó, con su webhook.
    pub async fn basecamp_link_project(&self, org_id: &str, project_id: i64, space_id: &str) -> Result<BasecampLinked, ApiError> {
        self.core.call("PUT", &format!("{}/projects/{project_id}", Self::basecamp_base(org_id)), Some(serde_json::json!({ "spaceId": space_id }))).await
    }

    pub async fn basecamp_unlink_project(&self, org_id: &str, project_id: i64) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("{}/projects/{project_id}", Self::basecamp_base(org_id)), None).await
    }

    pub async fn basecamp_people(&self, org_id: &str) -> Result<Vec<BasecampPerson>, ApiError> {
        self.core.get_list(&format!("{}/people", Self::basecamp_base(org_id))).await
    }

    /// Asigna una persona de Basecamp a un miembro; con `user_id` vacío la
    /// suelta. Devuelve la persona como quedó.
    pub async fn basecamp_match_person(&self, org_id: &str, person_id: i64, user_id: &str) -> Result<BasecampPerson, ApiError> {
        self.core.call("PUT", &format!("{}/people/{person_id}", Self::basecamp_base(org_id)), Some(serde_json::json!({ "userId": user_id }))).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available(name: &str, space: &str) -> BasecampAvailable {
        BasecampAvailable { id: 1, name: name.into(), space_id: space.into(), ..Default::default() }
    }

    fn person(id: i64, name: &str, client: bool, user: &str) -> BasecampPerson {
        BasecampPerson { person_id: id, name: name.into(), client, user_id: user.into(), ..Default::default() }
    }

    fn member(id: &str, username: &str, name: Option<&str>, email: &str) -> OrgMember {
        serde_json::from_value(serde_json::json!({ "userId": id, "username": username, "name": name, "email": email, "role": "member" })).unwrap()
    }

    /// Conectar sólo a quien administra, con la app puesta y sin cuenta
    /// conectada; desconectar sólo a quien administra y con una cuenta.
    ///
    /// Roturas que atrapa: ofrecer conectar con el servidor sin la app (un
    /// botón que contesta 503), a quien no administra o encima de otra
    /// cuenta; ofrecer desconectar sin cuenta.
    #[test]
    fn connecting_is_offered_only_when_it_can_work() {
        let off = BasecampStatus { configured: false, ..Default::default() };
        let ready = BasecampStatus { configured: true, ..Default::default() };
        let connected = BasecampStatus { configured: true, connected: true, ..Default::default() };
        assert!(!off.offers_connect(true), "ofrece conectar sin la app en el servidor");
        assert!(!ready.offers_connect(false), "ofrece conectar a quien no administra");
        assert!(ready.offers_connect(true));
        assert!(!connected.offers_connect(true), "ofrece conectar otra cuenta encima");
        assert!(connected.offers_disconnect(true));
        assert!(!connected.offers_disconnect(false), "ofrece desconectar a quien no administra");
        assert!(!ready.offers_disconnect(true), "ofrece desconectar sin cuenta");
    }

    /// Un permiso que no se renueva se dice sólo con una cuenta conectada.
    ///
    /// Rotura que atrapa: avisar de reconectar sin cuenta, o no avisar con
    /// el fallo a la vista.
    #[test]
    fn a_failed_renewal_asks_to_reconnect_only_when_connected() {
        let failed = Some("2026-11-26T10:00:00Z".to_string());
        assert!(BasecampStatus { connected: true, refresh_failed_at: failed.clone(), ..Default::default() }.needs_reconnect());
        assert!(!BasecampStatus { connected: true, ..Default::default() }.needs_reconnect());
        assert!(!BasecampStatus { connected: false, refresh_failed_at: failed, ..Default::default() }.needs_reconnect());
    }

    /// Los enlazados primero y por nombre sin mirar mayúsculas.
    ///
    /// Roturas que atrapa: los sin enlazar delante; el orden que separa
    /// «alfa» de «Beta» por mayúscula.
    #[test]
    fn linked_projects_come_first_and_by_name() {
        let all = [available("zeta", ""), available("Beta", "sp"), available("alfa", "sp"), available("Mu", "")];
        let names: Vec<String> = ordered_projects(&all).into_iter().map(|project| project.name).collect();
        assert_eq!(names, ["alfa", "Beta", "Mu", "zeta"]);
    }

    /// Esperan sólo las que no tienen usuario, las del equipo antes que las
    /// del cliente.
    ///
    /// Roturas que atrapa: listar una persona ya asignada; las del cliente
    /// delante; no ordenar por nombre.
    #[test]
    fn only_the_unassigned_wait_and_the_team_comes_before_the_client() {
        let all = [person(1, "Zoe", false, ""), person(2, "Cliente Uno", true, ""), person(3, "Ana", false, ""), person(4, "Ya Casada", false, "u1")];
        let ids: Vec<i64> = unmatched_people(&all).into_iter().map(|p| p.person_id).collect();
        assert_eq!(ids, [3, 1, 2]);
    }

    /// Se busca por el nombre que se enseña, no por el usuario de acceso:
    /// quien no tiene nombre se encuentra por su usuario porque es lo que se
    /// le ve, pero a quien sí lo tiene no se le encuentra por su usuario.
    ///
    /// Roturas que atrapa: buscar por `username` a mano (la regla de
    /// `nombreVisible`); no buscar por correo; distinguir mayúsculas.
    #[test]
    fn members_are_found_by_the_name_that_is_shown() {
        let members = [member("u1", "aruiz", Some("Ana Ruiz"), "ana@x.com"), member("u2", "bob", None, "bob@acme.io")];
        let ids = |query: &str| assignable_members(&members, query).into_iter().map(|m| m.user_id).collect::<Vec<_>>();
        assert_eq!(ids(" ANA r"), ["u1"]);
        assert!(ids("aruiz").is_empty(), "encontró a Ana por su usuario de acceso");
        assert_eq!(ids("acme"), ["u2"]);
        assert_eq!(ids("bob"), ["u2"]);
        assert_eq!(ids(""), ["u1", "u2"]);
    }
}
