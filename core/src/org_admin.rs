//! La pantalla de la organización: quién está, a quién se invitó, sus
//! espacios y sus reglas; y las invitaciones que le llegan a quien mira.
//!
//! Vino de `app/src/pages/OrganizationSettings.tsx`, `components/org/*`,
//! `pages/Invitations.tsx` y de los stores `orgs.store.ts` e
//! `invitations.store.ts`. Lo que decide (quién administra, qué se lista, qué
//! se dice de una invitación o de un espacio) es función pura; lo que habla
//! con el servidor devuelve su resultado para que la pantalla diga por qué
//! falló.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::orgs::{Organization, Orgs};
use crate::texts::{text, text_count, text_with, Locale};
use crate::tree::{FolderTree, ListSummary, SpaceTree};

/// Los papeles en una organización, de más a menos.
pub const ORG_ROLES: [&str; 3] = ["admin", "member", "viewer"];

/// Alguien de la organización. Espejo de `domain.MemberResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct OrgMember {
    pub user_id: String,
    pub username: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    pub role: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    /// Dada de alta por correo y sin terminar el registro: su usuario es
    /// provisional y no se enseña.
    #[serde(default)]
    pub setup_pending: bool,
}

/// Una invitación, vista desde la organización (con `invited_user` y su
/// caducidad) o desde quien la recibe. Espejo de `domain.InvitationResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Invitation {
    pub id: String,
    #[serde(default)]
    pub org_id: String,
    #[serde(default)]
    pub org_name: String,
    pub role: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub invited_by: String,
    #[serde(default)]
    pub invited_user: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// Alguien de la plataforma, para invitarlo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct PersonHit {
    pub id: String,
    pub username: String,
}

/// Lo que contesta dar de alta por correo, o reenviar su enlace.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct AccountLink {
    /// `created`: cuenta nueva; `invited`: el correo ya tenía cuenta.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub email_sent: bool,
    /// Sólo si el correo no salió: el enlace para pasarlo a mano.
    #[serde(default)]
    pub link: Option<String>,
}

/// Lo que se dice tras dar de alta por correo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AccountNotice {
    /// Salió bien y basta con decirlo.
    Said { text: String },
    /// El correo no salió: la cuenta está creada y el enlace se enseña para
    /// pasarlo a mano. Sólo entonces: quien lo tiene elige la contraseña de
    /// otra persona.
    ShowLink { lead: String, link: String },
}

/// Qué decir tras dar de alta a `name` <`email`> en `org`.
pub fn account_notice(out: &AccountLink, name: &str, email: &str, org: &str, locale: Locale) -> AccountNotice {
    let email = email.trim();
    if out.status == "invited" {
        return AccountNotice::Said { text: text_with(locale, "common:account.invited", &[("email", email), ("org", org)]) };
    }
    match out.link.as_deref().filter(|link| !out.email_sent && !link.is_empty()) {
        Some(link) => AccountNotice::ShowLink { lead: text_with(locale, "common:account.linkLead", &[("name", name.trim())]), link: link.to_string() },
        None if out.email_sent => AccountNotice::Said { text: text_with(locale, "common:account.created", &[("email", email)]) },
        None => AccountNotice::Said { text: text(locale, "common:account.resent") },
    }
}

/// Lo mismo que pide el servidor (`validate:"email"`), sin pretender más.
pub fn valid_email(email: &str) -> bool {
    let email = email.trim();
    let Some((user, domain)) = email.split_once('@') else { return false };
    let Some((host, tld)) = domain.rsplit_once('.') else { return false };
    !email.chars().any(char::is_whitespace) && !user.is_empty() && !host.is_empty() && !tld.is_empty() && !domain.contains('@')
}

/// Qué pasó al «añadir directamente».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AddOutcome {
    Added,
    /// No era de tu gente: el servidor pide invitarla, y se la invitó.
    Invited,
}

/// Un cambio en la organización: lo que no se nombra no se toca.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OrgPatch {
    pub name: Option<String>,
    pub domain: Option<String>,
    pub default_invite_role: Option<String>,
    pub clients_see_only_their_space: Option<bool>,
    pub guests_can_use_dev_tools: Option<bool>,
    pub done_needs_subtasks_done: Option<bool>,
    pub timezone: Option<String>,
}

impl OrgPatch {
    /// El cuerpo del PATCH. El nombre va siempre: el validador del servidor
    /// lo pide aunque sólo cambie una regla.
    fn body(&self, current_name: &str) -> serde_json::Value {
        let name = self.name.as_deref().map(str::trim).filter(|name| !name.is_empty()).unwrap_or(current_name);
        let mut body = serde_json::json!({ "name": name });
        let fields = [
            ("domain", self.domain.clone().map(serde_json::Value::from)),
            ("defaultInviteRole", self.default_invite_role.clone().map(serde_json::Value::from)),
            ("clientsSeeOnlyTheirSpace", self.clients_see_only_their_space.map(serde_json::Value::from)),
            ("guestsCanUseDevTools", self.guests_can_use_dev_tools.map(serde_json::Value::from)),
            ("doneNeedsSubtasksDone", self.done_needs_subtasks_done.map(serde_json::Value::from)),
            ("timezone", self.timezone.clone().map(|zone| serde_json::Value::from(zone.trim().to_string()))),
        ];
        for (key, value) in fields {
            if let Some(value) = value {
                body[key] = value;
            }
        }
        body
    }
}

/// Las pestañas de la pantalla, en el orden en que se suele venir a ellas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum OrgTab {
    Members,
    Invites,
    Spaces,
    Integrations,
    GitHub,
    Meetings,
    General,
}

impl OrgTab {
    pub fn label_key(self) -> &'static str {
        match self {
            OrgTab::Members => "org:tab.members",
            OrgTab::Invites => "org:tab.invites",
            OrgTab::Spaces => "org:tab.spaces",
            OrgTab::Integrations => "org:tab.integrations",
            OrgTab::GitHub => "org:tab.github",
            OrgTab::Meetings => "org:tab.meetings",
            OrgTab::General => "org:tab.general",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            OrgTab::Members => "members",
            OrgTab::Invites => "invites",
            OrgTab::Spaces => "spaces",
            OrgTab::Integrations => "integrations",
            OrgTab::GitHub => "github",
            OrgTab::Meetings => "meetings",
            OrgTab::General => "general",
        }
    }
}

/// Quién administra la organización: su admin, o un superadmin de la
/// plataforma, que las ve todas sin pertenecer a ninguna.
pub fn can_manage_org(org: &Organization, superadmin: bool) -> bool {
    superadmin || org.role == "admin"
}

/// Las pestañas que se enseñan. Las invitaciones sólo a quien administra: a
/// los demás el servidor no se las da.
pub fn org_tabs(can_manage: bool) -> Vec<OrgTab> {
    [OrgTab::Members, OrgTab::Invites, OrgTab::Spaces, OrgTab::Integrations, OrgTab::GitHub, OrgTab::Meetings, OrgTab::General]
        .into_iter()
        .filter(|tab| *tab != OrgTab::Invites || can_manage)
        .collect()
}

/// Los miembros que casan con lo escrito (en su usuario o su correo) y con el
/// papel elegido. Vacío es «cualquiera».
pub fn members_matching(members: &[OrgMember], query: &str, role: &str) -> Vec<OrgMember> {
    let query = query.trim().to_lowercase();
    members
        .iter()
        .filter(|member| role.is_empty() || member.role == role)
        .filter(|member| {
            // El usuario provisional (`pending-…`) de quien no terminó el
            // registro no se enseña, así que tampoco se busca por él.
            query.is_empty()
                || (!member.setup_pending && member.username.to_lowercase().contains(&query))
                || member.email.as_deref().unwrap_or("").to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

/// Cómo se le llama a un miembro en la tabla, y la línea de debajo.
///
/// Quien no terminó el registro tiene un usuario provisional (`pending-…`)
/// que no eligió nadie: se enseña su nombre o su correo, nunca ese usuario
/// (`MemberName.tsx`).
pub fn member_face(member: &OrgMember) -> (String, Option<String>) {
    let name = member.name.as_deref().map(str::trim).filter(|name| !name.is_empty());
    if member.setup_pending {
        let shown = name.or(member.email.as_deref()).unwrap_or("");
        return (shown.to_string(), None);
    }
    (name.unwrap_or(&member.username).to_string(), Some(format!("@{}", member.username)))
}

/// Cuánto le queda a una invitación, o que ya caducó: «dentro de 12 días»,
/// «caducada». Sin fecha, «pendiente».
pub fn invitation_deadline<Viewer: TimeZone>(expires_at: Option<&str>, now: &DateTime<Viewer>, locale: Locale) -> (bool, String) {
    let Some(at) = expires_at.and_then(|at| DateTime::parse_from_rfc3339(at).ok()) else {
        return (false, text(locale, "org:pendingChip"));
    };
    let hours = (at.with_timezone(&Utc) - now.with_timezone(&Utc)).num_hours();
    let left_ms = (at.with_timezone(&Utc) - now.with_timezone(&Utc)).num_milliseconds();
    if left_ms <= 0 {
        return (true, text(locale, "org:expired"));
    }
    // Al menos una hora: «en 0 horas» suena a que ya venció.
    if hours < 24 {
        return (false, text_count(locale, "common:time.inHours", hours.max(1)));
    }
    (false, text_count(locale, "common:time.inDays", hours / 24))
}

/// Una fecha con su año: «9 jun 2026», «Jun 9, 2026» (`fechaConAno`).
pub fn date_with_year(locale: Locale, iso: &str) -> String {
    use chrono::Datelike;
    let Ok(at) = DateTime::parse_from_rfc3339(iso) else { return String::new() };
    let day = at.date_naive();
    format!(
        "{}{} {}",
        crate::mywork::day_and_month(locale, day),
        if locale == Locale::En { "," } else { "" },
        day.year()
    )
}

/// La línea bajo el nombre de la organización: el papel de quien mira, si
/// es superadmin, la gente, los espacios y desde cuándo existe. Lo que no se
/// sabe (cero) no se dice.
pub fn org_summary(org: &Organization, superadmin: bool, spaces: usize, locale: Locale) -> String {
    let mut parts = vec![text_with(locale, "org:yourRole", &[("role", &org.role)])];
    if superadmin {
        parts.push("superadmin".to_string());
    }
    if org.member_count > 0 {
        parts.push(text_count(locale, "common:count.members", org.member_count));
    }
    if spaces > 0 {
        parts.push(text_count(locale, "common:count.spaces", spaces as i64));
    }
    let created = date_with_year(locale, &org.created_at);
    if !created.is_empty() {
        parts.push(text_with(locale, "org:createdOn", &[("when", &created)]));
    }
    parts.join(" · ")
}

/// Quién ve un espacio.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SpaceAccess {
    /// Toda la organización; ningún cliente.
    WholeOrg,
    /// El espacio es de un cliente: su nombre.
    Client { name: String },
    /// El espacio no, pero alguna de sus listas sí.
    Partly,
}

/// La ficha de un espacio en la pestaña Espacios.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SpaceCard {
    pub id: String,
    pub name: String,
    pub color: String,
    pub tasks: i64,
    pub open: i64,
    pub access: SpaceAccess,
    /// Quienes cargan trabajo abierto, hasta cuatro.
    pub people: Vec<SpaceFace>,
    /// Cuántos más hay que no caben.
    pub more: i64,
}

/// Una cara en la ficha de un espacio.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SpaceFace {
    pub initials: String,
    pub username: String,
}

/// Un proyecto de reportes: a qué cliente pertenece un espacio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ReportProject {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub org_id: String,
}

fn every_list(folders: &[FolderTree], out: &mut Vec<ListSummary>) {
    for folder in folders {
        out.extend(folder.lists.iter().cloned());
        every_list(&folder.folders, out);
    }
}

/// Las iniciales de un usuario: las primeras letras de sus dos primeras
/// partes (`jose.marin` → «JM»), o sus dos primeras (`iniciales` de
/// `desde.ts`).
pub fn username_initials(username: &str) -> String {
    let clean = username.trim().trim_start_matches('@').trim();
    if clean.is_empty() {
        return "?".into();
    }
    let parts: Vec<&str> = clean.split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-')).filter(|part| !part.is_empty()).collect();
    if parts.len() >= 2 {
        return parts[..2].iter().filter_map(|part| part.chars().next()).flat_map(char::to_uppercase).collect();
    }
    clean.chars().take(2).flat_map(char::to_uppercase).collect()
}

/// Las fichas de los espacios.
///
/// Una lista puede ser de un cliente aunque su espacio no lo sea, así que
/// «toda la organización» mira las dos: decir que un espacio es interno
/// cuando una de sus listas no lo es sería el peor error de esta pantalla.
pub fn space_cards(spaces: &[SpaceTree], projects: &[ReportProject]) -> Vec<SpaceCard> {
    spaces
        .iter()
        .map(|space| {
            let mut lists = space.lists.clone();
            every_list(&space.folders, &mut lists);
            let access = match &space.project_id {
                Some(project) => SpaceAccess::Client {
                    name: projects.iter().find(|candidate| &candidate.id == project).map(|found| found.name.clone()).unwrap_or_default(),
                },
                None if lists.iter().any(|list| list.project_id.is_some()) => SpaceAccess::Partly,
                None => SpaceAccess::WholeOrg,
            };
            SpaceCard {
                id: space.id.clone(),
                name: space.name.clone(),
                color: space.color.clone(),
                tasks: lists.iter().map(|list| list.task_count).sum(),
                open: lists.iter().map(|list| list.open_count).sum(),
                access,
                people: space.people.iter().take(4).map(|person| SpaceFace { initials: username_initials(&person.username), username: person.username.clone() }).collect(),
                more: space.people.len().saturating_sub(4) as i64,
            }
        })
        .collect()
}

/// Lo que dice la ficha de quién ve el espacio.
pub fn space_access_text(access: &SpaceAccess, locale: Locale) -> String {
    match access {
        SpaceAccess::WholeOrg => text(locale, "org:spaceAccess.wholeOrg"),
        SpaceAccess::Partly => text(locale, "org:spaceAccess.partly"),
        SpaceAccess::Client { name } if name.is_empty() => text(locale, "org:spaceAccess.someClient"),
        SpaceAccess::Client { name } => name.clone(),
    }
}

/// La frase que hay que escribir para borrar una organización: con el nombre
/// exacto, mayúsculas incluidas. Una confirmación que se pasa por
/// aproximación dejó de confirmar.
pub fn delete_org_phrase(org_name: &str) -> String {
    format!("delete/{org_name}")
}

pub fn delete_org_confirmed(org_name: &str, typed: &str) -> bool {
    typed == delete_org_phrase(org_name)
}

/// Si lo escrito vale como zona del equipo: vacía (la quita) o una zona IANA
/// que existe.
pub fn valid_team_zone(zone: &str) -> bool {
    let zone = zone.trim();
    zone.is_empty() || zone.parse::<chrono_tz::Tz>().is_ok()
}

fn encoded(value: &str) -> String {
    value.bytes().map(|byte| if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect()
}

impl Orgs {
    fn base(org_id: &str) -> String {
        format!("/api/v1/organizations/{org_id}")
    }

    pub async fn members(&self, org_id: &str) -> Result<Vec<OrgMember>, ApiError> {
        self.core.get_list(&format!("{}/members", Self::base(org_id))).await
    }

    pub async fn add_member(&self, org_id: &str, user_id: &str, role: &str) -> Result<(), ApiError> {
        let body = serde_json::json!({ "userId": user_id, "role": role });
        self.core.call_ok("POST", &format!("{}/members", Self::base(org_id)), Some(body)).await
    }

    /// Añadir directamente; y si no es de tu gente (el servidor contesta
    /// `invite-required`: cualquiera puede crearse una organización, así que
    /// meter sin aceptar sólo vale para quien ya administras), invitarla.
    pub async fn add_or_invite(&self, org_id: &str, user_id: &str, role: &str) -> Result<AddOutcome, ApiError> {
        match self.add_member(org_id, user_id, role).await {
            Ok(()) => Ok(AddOutcome::Added),
            Err(ApiError::Server { code, .. }) if code == "invite-required" => {
                self.invite(org_id, user_id, role).await?;
                Ok(AddOutcome::Invited)
            }
            Err(error) => Err(error),
        }
    }

    pub async fn set_member_role(&self, org_id: &str, user_id: &str, role: &str) -> Result<(), ApiError> {
        let body = serde_json::json!({ "role": role });
        self.core.call_ok("PATCH", &format!("{}/members/{user_id}", Self::base(org_id)), Some(body)).await
    }

    pub async fn remove_member(&self, org_id: &str, user_id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("{}/members/{user_id}", Self::base(org_id)), None).await
    }

    /// Las invitaciones que mandó la organización, caducadas incluidas: sin
    /// ellas a la vista, una vencida no se podía ni retirar ni repetir.
    pub async fn sent_invitations(&self, org_id: &str) -> Result<Vec<Invitation>, ApiError> {
        self.core.get_list(&format!("{}/invitations", Self::base(org_id))).await
    }

    pub async fn invite(&self, org_id: &str, user_id: &str, role: &str) -> Result<(), ApiError> {
        let body = serde_json::json!({ "userId": user_id, "role": role });
        self.core.call_ok("POST", &format!("{}/invitations", Self::base(org_id)), Some(body)).await
    }

    /// Le da otros 14 días.
    pub async fn resend_invitation(&self, org_id: &str, invitation_id: &str) -> Result<(), ApiError> {
        self.core.call_ok("POST", &format!("{}/invitations/{invitation_id}/resend", Self::base(org_id)), Some(serde_json::json!({}))).await
    }

    pub async fn revoke_invitation(&self, org_id: &str, invitation_id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("{}/invitations/{invitation_id}", Self::base(org_id)), None).await
    }

    /// Dar de alta por correo: la cuenta nueva, o la invitación si el correo
    /// ya tenía una.
    pub async fn create_account(&self, org_id: &str, name: &str, email: &str, role: &str) -> Result<AccountLink, ApiError> {
        let body = serde_json::json!({ "name": name.trim(), "email": email.trim(), "role": role });
        self.core.call("POST", &format!("{}/accounts", Self::base(org_id)), Some(body)).await
    }

    /// Otro enlace para quien no terminó el registro. El anterior deja de valer.
    pub async fn resend_account_link(&self, org_id: &str, user_id: &str) -> Result<AccountLink, ApiError> {
        self.core.call("POST", &format!("{}/accounts/{user_id}/resend", Self::base(org_id)), Some(serde_json::json!({}))).await
    }

    /// Gente de toda la plataforma, para invitarla. El servidor deja fuera a
    /// quien pregunta.
    pub async fn search_people(&self, query: &str) -> Result<Vec<PersonHit>, ApiError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        self.core.get_list(&format!("/api/v1/users/search?q={}", encoded(query))).await
    }

    /// Los proyectos de reportes de la organización: de quién es cada espacio
    /// de cliente.
    pub async fn report_projects(&self, org_id: &str) -> Result<Vec<ReportProject>, ApiError> {
        let all: Vec<ReportProject> = self.core.get_list("/api/v1/report-projects/").await?;
        Ok(all.into_iter().filter(|project| project.org_id == org_id).collect())
    }

    /// Cambia la organización. Manda lo que contesta el servidor: es quien
    /// sabe qué dejó un cambio parcial.
    pub async fn save(&self, org_id: &str, patch: &OrgPatch) -> Result<(), ApiError> {
        let Some(current) = self.state().orgs.into_iter().find(|org| org.id == org_id) else {
            return Err(ApiError::Server { status: 404, code: String::new(), message: "Organization not found".into() });
        };
        let answer: serde_json::Value = self.core.call("PATCH", &Self::base(org_id), Some(patch.body(&current.name))).await?;
        // Se mezcla sobre lo que había en vez de sustituirlo, y el papel no
        // se toma de ahí: a un superadmin el servidor le contesta «admin»
        // aunque no sea miembro.
        let mut merged = serde_json::to_value(&current).unwrap_or_default();
        if let (Some(target), Some(fields)) = (merged.as_object_mut(), answer.as_object()) {
            for (key, value) in fields {
                if !value.is_null() && key != "role" {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
        if let Ok(updated) = serde_json::from_value::<Organization>(merged) {
            self.replace(updated);
        }
        Ok(())
    }

    /// Borra la organización (sólo un superadmin). Si era la que se miraba,
    /// se pasa a otra con lo suyo.
    pub async fn delete(&self, org_id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &Self::base(org_id), None).await?;
        let before = self.state().current_org_id;
        // Fuera de la lista ya, aunque releerla falle: ya no existe.
        self.forget(org_id);
        self.load().await;
        self.follow_current(before).await;
        Ok(())
    }

    /// Quita una organización de la lista y, si era la que se miraba, pasa a
    /// la primera que quede.
    fn forget(&self, org_id: &str) {
        self.update(|state| {
            state.orgs.retain(|org| org.id != org_id);
            if state.current_org_id.as_deref() == Some(org_id) {
                state.current_org_id = state.orgs.first().map(|org| org.id.clone());
            }
        });
    }

    /// Si la organización que se mira ya no es `before`, se hace lo mismo
    /// que al elegir otra.
    async fn follow_current(&self, before: Option<String>) {
        if self.state().current_org_id != before {
            self.core.current_org_changed().await;
        }
    }

    /// Vuelve a leer a qué organizaciones perteneces y con qué papel: tras
    /// quitarte o cambiarte el papel a ti mismo, o cuando el servidor avisa
    /// de que entraste o saliste de una.
    pub async fn reload_membership(&self) {
        let before = self.state().current_org_id;
        // El token lleva dentro a qué perteneces.
        self.core.refresh_access_token().await;
        self.load().await;
        self.follow_current(before).await;
    }

    /// Te quitaste tú mismo de esta organización: fuera de la lista aunque
    /// releerla falle, y a otra si era la que mirabas.
    pub async fn left(&self, org_id: &str) {
        let before = self.state().current_org_id;
        self.forget(org_id);
        self.core.refresh_access_token().await;
        self.load().await;
        self.follow_current(before).await;
    }

    /// `org:membership` va dirigido a quien entra o sale, y `org:role` a
    /// quien cambia de papel: el token lleva el papel, así que los dos
    /// piden lo mismo.
    pub(crate) fn server_event(&self, event: &crate::stream::ServerEvent) {
        if matches!(event.event.as_str(), "org:membership" | "org:role") {
            let orgs = self.core.orgs();
            tokio::spawn(async move { orgs.reload_membership().await });
        }
    }

    /// Las invitaciones que esperan a quien mira. Pone su número en el riel.
    pub async fn my_invitations(&self) -> Result<Vec<Invitation>, ApiError> {
        let pending: Vec<Invitation> = self.core.get_list("/api/v1/invitations/").await?;
        self.core.counters().invitations_seen(pending.len() as i64);
        Ok(pending)
    }

    /// Acepta y **renueva la sesión**: el token lleva dentro a qué
    /// organizaciones perteneces, y con el de antes la nueva se vería vacía
    /// hasta volver a entrar. Devuelve si se pudo renovar: si no, la
    /// invitación sí quedó aceptada y lo que falta es entrar otra vez.
    pub async fn accept_invitation(&self, invitation_id: &str) -> Result<bool, ApiError> {
        self.core.call_ok("POST", &format!("/api/v1/invitations/{invitation_id}/accept"), Some(serde_json::json!({}))).await?;
        let renewed = self.core.refresh_access_token().await.is_some();
        let before = self.state().current_org_id;
        self.load().await;
        self.follow_current(before).await;
        let _ = self.my_invitations().await;
        Ok(renewed)
    }

    pub async fn decline_invitation(&self, invitation_id: &str) -> Result<(), ApiError> {
        self.core.call_ok("POST", &format!("/api/v1/invitations/{invitation_id}/decline"), Some(serde_json::json!({}))).await?;
        let _ = self.my_invitations().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texts::Locale;

    fn member(username: &str, email: &str, role: &str) -> OrgMember {
        OrgMember { user_id: username.into(), username: username.into(), name: None, email: Some(email.into()), role: role.into(), last_seen_at: None, setup_pending: false }
    }

    /// Roturas que atrapa: enseñar la pestaña de invitaciones a quien no
    /// administra (el servidor le niega la lista); que un superadmin sin
    /// papel en la organización no la administre.
    #[test]
    fn only_who_manages_sees_the_invitations_tab() {
        assert!(!org_tabs(false).contains(&OrgTab::Invites));
        assert_eq!(org_tabs(true)[..3], [OrgTab::Members, OrgTab::Invites, OrgTab::Spaces]);
        let org: Organization = serde_json::from_value(serde_json::json!({
            "id": "o", "name": "O", "slug": "o", "role": "member", "createdAt": "2026-06-09T12:00:00Z",
        }))
        .unwrap();
        assert!(!can_manage_org(&org, false));
        assert!(can_manage_org(&org, true));
        assert!(can_manage_org(&Organization { role: "admin".into(), ..org }, false));
    }

    /// Roturas que atrapa: buscar sólo en el usuario (no en el correo);
    /// distinguir mayúsculas; que el filtro de papel no filtre, o que
    /// «cualquiera» deje fuera a alguien; encontrar a alguien por un usuario
    /// provisional que no se enseña.
    #[test]
    fn members_are_found_by_username_or_email_and_by_role() {
        let all = [member("ana", "ana@x.com", "admin"), member("bob", "BOB@Acme.io", "viewer"), member("caro", "c@x.com", "member")];
        let names = |found: Vec<OrgMember>| found.into_iter().map(|m| m.username).collect::<Vec<_>>();
        assert_eq!(names(members_matching(&all, "acme", "")), ["bob"]);
        assert_eq!(names(members_matching(&all, " AN ", "")), ["ana"]);
        assert_eq!(names(members_matching(&all, "", "member")), ["caro"]);
        assert_eq!(names(members_matching(&all, "", "")).len(), 3);
        assert!(members_matching(&all, "ana", "viewer").is_empty());
        let pending = OrgMember { username: "pending-7f3a".into(), setup_pending: true, ..member("x", "dan@x.com", "member") };
        assert!(members_matching(&[pending.clone()], "pending", "").is_empty(), "se busca por el usuario provisional");
        assert_eq!(members_matching(&[pending], "dan@", "").len(), 1);
    }

    /// Roturas que atrapa: enseñar el usuario provisional `pending-…` de
    /// quien no terminó el registro; no caer al usuario cuando no hay nombre.
    #[test]
    fn a_member_is_shown_by_name_and_a_pending_one_never_by_its_username() {
        let mut ana = member("ana", "ana@x.com", "admin");
        assert_eq!(member_face(&ana), ("ana".into(), Some("@ana".into())));
        ana.name = Some("Ana Ruiz".into());
        assert_eq!(member_face(&ana), ("Ana Ruiz".into(), Some("@ana".into())));
        let pending = OrgMember { username: "pending-7f3a".into(), setup_pending: true, name: None, ..ana };
        assert_eq!(member_face(&pending), ("ana@x.com".into(), None));
    }

    /// Roturas que atrapa: una caducada que dice cuánto le queda (o una viva
    /// que dice «caducada»); «0 horas» para lo que vence en minutos; contar
    /// días por horas.
    #[test]
    fn an_invitation_says_how_long_it_has_or_that_it_expired() {
        let now = DateTime::parse_from_rfc3339("2026-11-24T12:00:00Z").unwrap();
        let es = Locale::Es;
        assert_eq!(invitation_deadline(Some("2026-11-18T12:00:00Z"), &now, es), (true, "caducada".into()));
        assert_eq!(invitation_deadline(Some("2026-12-07T13:00:00Z"), &now, es), (false, "dentro de 13 días".into()));
        assert_eq!(invitation_deadline(Some("2026-11-24T12:40:00Z"), &now, es), (false, "dentro de 1 hora".into()));
        assert_eq!(invitation_deadline(Some("2026-11-24T17:00:00Z"), &now, Locale::En), (false, "in 5 hours".into()));
        assert_eq!(invitation_deadline(None, &now, es), (false, "pendiente".into()));
    }

    /// Roturas que atrapa: no sumar las listas de las carpetas (ni las
    /// anidadas); llamar «de toda la organización» a un espacio con una
    /// lista de cliente; no nombrar al cliente dueño; más de cuatro caras.
    #[test]
    fn a_space_card_counts_nested_lists_and_says_who_sees_it() {
        let space = |id: &str, project: Option<&str>, list_project: Option<&str>, people: usize| -> SpaceTree {
            serde_json::from_value(serde_json::json!({
                "id": id, "orgId": "o", "name": id, "color": "#0ff", "projectId": project,
                "folders": [{ "id": "f", "name": "f", "lists": [{ "id": "l2", "name": "l2", "openCount": 2, "taskCount": 8 }],
                              "folders": [{ "id": "g", "name": "g", "folders": [], "lists": [{ "id": "l3", "name": "l3", "openCount": 1, "taskCount": 3, "projectId": list_project }] }] }],
                "lists": [{ "id": "l1", "name": "l1", "openCount": 7, "taskCount": 40 }],
                "people": (0..people).map(|n| serde_json::json!({ "userId": format!("u{n}"), "username": format!("jose.marin{n}") })).collect::<Vec<_>>(),
            }))
            .unwrap()
        };
        let projects = [ReportProject { id: "p1".into(), name: "Portento".into(), org_id: "o".into() }];
        let cards = space_cards(&[space("a", None, None, 6), space("b", Some("p1"), None, 0), space("c", None, Some("p1"), 0)], &projects);
        assert_eq!((cards[0].tasks, cards[0].open), (51, 10));
        assert_eq!(cards[0].access, SpaceAccess::WholeOrg);
        assert_eq!(cards[0].people.len(), 4);
        assert_eq!(cards[0].more, 2);
        assert_eq!(cards[0].people[0].initials, "JM");
        assert_eq!(cards[1].access, SpaceAccess::Client { name: "Portento".into() });
        assert_eq!(cards[2].access, SpaceAccess::Partly);
        assert_eq!(space_access_text(&SpaceAccess::Client { name: String::new() }, Locale::En), "a client");
    }

    /// Roturas que atrapa: aceptar el nombre a secas, o una aproximación en
    /// minúsculas.
    #[test]
    fn deleting_an_organization_takes_the_exact_phrase() {
        assert!(!delete_org_confirmed("Nuke AI", "Nuke AI"));
        assert!(!delete_org_confirmed("Nuke AI", "delete/nuke ai"));
        assert!(delete_org_confirmed("Nuke AI", "delete/Nuke AI"));
    }

    /// Roturas que atrapa: mandar un PATCH sin nombre (el servidor lo
    /// rechaza); apagar las reglas que no se nombran mandándolas en falso.
    #[test]
    fn a_patch_carries_the_name_and_only_what_changed() {
        let patch = OrgPatch { guests_can_use_dev_tools: Some(true), ..Default::default() };
        assert_eq!(patch.body("Uno"), serde_json::json!({ "name": "Uno", "guestsCanUseDevTools": true }));
        let renamed = OrgPatch { name: Some("  Dos ".into()), timezone: Some(String::new()), ..Default::default() };
        assert_eq!(renamed.body("Uno"), serde_json::json!({ "name": "Dos", "timezone": "" }));
    }

    /// Roturas que atrapa: el resumen sin el papel, con «0 espacios», o sin
    /// la fecha.
    #[test]
    fn the_summary_says_role_size_and_age() {
        let org: Organization = serde_json::from_value(serde_json::json!({
            "id": "o", "name": "O", "slug": "o", "role": "admin", "memberCount": 5, "createdAt": "2026-06-09T12:00:00Z",
        }))
        .unwrap();
        assert_eq!(org_summary(&org, false, 0, Locale::Es), "Tu rol: admin · 5 miembros · creada el 9 jun 2026");
        assert_eq!(org_summary(&org, true, 2, Locale::En), "Your role: admin · superadmin · 5 members · 2 spaces · created Jun 9, 2026");
    }

    /// Roturas que atrapa: enseñar el enlace cuando el correo sí salió
    /// (quien lo tiene elige la contraseña de otro); no enseñarlo cuando no
    /// salió; decir «le mandamos el enlace» a quien ya tenía cuenta.
    #[test]
    fn after_adding_by_email_the_link_shows_only_if_the_email_did_not_go() {
        let out = |status: &str, sent: bool, link: Option<&str>| AccountLink { status: status.into(), user_id: "u".into(), email_sent: sent, link: link.map(Into::into) };
        let en = Locale::En;
        assert_eq!(account_notice(&out("created", true, None), "Caro", "caro@x.io", "Dwit", en), AccountNotice::Said { text: "We sent the link to caro@x.io.".into() });
        assert!(matches!(account_notice(&out("created", true, Some("https://cac.test/app/welcome#t")), "Caro", "caro@x.io", "Dwit", en), AccountNotice::Said { .. }));
        assert!(matches!(account_notice(&out("created", false, Some("https://cac.test/app/welcome#t")), "Caro", "caro@x.io", "Dwit", en), AccountNotice::ShowLink { link, .. } if link == "https://cac.test/app/welcome#t"));
        assert_eq!(
            account_notice(&out("invited", false, None), "Bea", " bea@x.io ", "Dwit", en),
            AccountNotice::Said { text: "bea@x.io already had an account: we invited them to Dwit.".into() }
        );
    }

    /// Roturas que atrapa: dar por bueno un correo sin dominio, sin punto o
    /// con espacios.
    #[test]
    fn an_email_needs_a_user_a_host_and_a_domain() {
        assert!(valid_email(" caro@x.io "));
        for bad in ["caro", "caro@x", "@x.io", "ca ro@x.io", "caro@x.", "a@b@c.io"] {
            assert!(!valid_email(bad), "{bad}");
        }
    }

    /// Roturas que atrapa: aceptar una zona inventada; no dejar quitarla.
    #[test]
    fn the_team_zone_is_empty_or_a_real_zone() {
        assert!(valid_team_zone(""));
        assert!(valid_team_zone(" America/Mexico_City "));
        assert!(!valid_team_zone("Mars/Olympus"));
    }
}
