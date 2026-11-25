//! Lo que la organización recibe de fuera (`OrgIntegrations.tsx`): sus
//! proyectos de reportes. Son integraciones de servidor a servidor: otro
//! sistema guarda una llave de ingesta y empuja trabajo, y todo lo que manda
//! cae en una lista de aquí.

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::orgs::Orgs;
use crate::tree::{FolderTree, SpaceTree};

pub use crate::org_admin::ReportProject;

/// Una lista dicha por su ruta: «Boaty · web · Tasks». Una bandeja se guarda
/// como un uuid, y un uuid no le dice nada a nadie.
///
/// La ruta lleva el espacio a propósito: dos clientes con una lista «Bugs» son
/// indistinguibles sin él, y elegir la del cliente equivocado es enseñarle su
/// trabajo a otro.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListPath {
    pub id: String,
    pub space: String,
    /// «carpeta · lista», o sólo la lista si cuelga del espacio.
    pub rest: String,
}

impl ListPath {
    pub fn path(&self) -> String {
        format!("{} · {}", self.space, self.rest)
    }
}

fn collect(space: &SpaceTree, folders: &[FolderTree], prefix: &str, out: &mut Vec<ListPath>) {
    for folder in folders {
        let here = if prefix.is_empty() { folder.name.clone() } else { format!("{prefix} · {}", folder.name) };
        out.extend(folder.lists.iter().map(|list| ListPath { id: list.id.clone(), space: space.name.clone(), rest: format!("{here} · {}", list.name) }));
        collect(space, &folder.folders, &here, out);
    }
}

/// Las listas del árbol de **esta** organización. No es un filtro de
/// comodidad: ofrecer como bandeja la lista de otra sería enseñarle el
/// trabajo de un cliente a gente que no tiene nada que ver con él. El
/// servidor lo rechaza (`inbox-other-org`), pero llegar a que lo rechace ya
/// es un fallo de la pantalla.
pub fn inbox_lists(spaces: &[SpaceTree], org_id: &str) -> Vec<ListPath> {
    let mut out = Vec::new();
    for space in spaces.iter().filter(|space| space.org_id == org_id) {
        out.extend(space.lists.iter().map(|list| ListPath { id: list.id.clone(), space: space.name.clone(), rest: list.name.clone() }));
        collect(space, &space.folders, "", &mut out);
    }
    out
}

/// La ruta de una lista, o `None` si no está en este árbol. `None` no es «no
/// hay bandeja»: es «hay una y no es de aquí» (de otra organización, o
/// borrada); quien llame decide cómo lo cuenta, y lo que no debe hacer es
/// pintar el uuid.
pub fn list_path(spaces: &[SpaceTree], org_id: &str, list_id: &str) -> Option<String> {
    inbox_lists(spaces, org_id).into_iter().find(|list| list.id == list_id).map(|list| list.path())
}

/// Las listas agrupadas por espacio, para un menú: el espacio es la
/// cabecera y cada opción dice sólo lo que sigue.
pub fn lists_by_space(lists: Vec<ListPath>) -> Vec<(String, Vec<ListPath>)> {
    let mut groups: Vec<(String, Vec<ListPath>)> = Vec::new();
    for list in lists {
        match groups.iter_mut().find(|(space, _)| *space == list.space) {
            Some((_, group)) => group.push(list),
            None => groups.push((list.space.clone(), vec![list])),
        }
    }
    groups
}

/// Lo que el formulario de una integración dice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectForm {
    pub name: String,
    pub per_hour: String,
    pub per_reporter: String,
    pub webhook_url: String,
    pub webhook_secret: String,
    pub assignee: String,
    pub list_id: String,
}

/// Lo que se manda al guardar. Omitir un campo lo deja como está.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_hour: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_reporter_per_hour: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_assignee_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
}

impl ProjectPatch {
    pub fn is_empty(&self) -> bool {
        *self == ProjectPatch::default()
    }
}

impl ProjectForm {
    pub fn of(project: &ReportProject) -> ProjectForm {
        ProjectForm {
            name: project.name.clone(),
            per_hour: project.rate_limit_per_hour.to_string(),
            per_reporter: project.rate_limit_per_reporter_per_hour.to_string(),
            webhook_url: project.webhook_url.clone(),
            webhook_secret: String::new(),
            assignee: project.default_assignee_user_id.clone().unwrap_or_default(),
            list_id: project.list_id.clone().unwrap_or_default(),
        }
    }

    /// **Sólo lo que cambió.** Antes iba el formulario entero, porque el
    /// servidor borraba lo que no viajara; eso hacía que dos personas
    /// editando a la vez se pisaran campos que ninguna había tocado.
    ///
    /// Borrar sigue siendo posible y explícito: vaciar el webhook manda `""`,
    /// que sí lo retira. Lo que no puede pasar es borrar algo sin haberlo
    /// pedido.
    pub fn patch(&self, project: &ReportProject) -> ProjectPatch {
        let mut patch = ProjectPatch::default();
        let name = self.name.trim();
        if !name.is_empty() && name != project.name {
            patch.name = Some(name.to_string());
        }
        let per_hour = self.per_hour.trim().parse::<i64>().unwrap_or(0);
        if per_hour > 0 && per_hour != project.rate_limit_per_hour {
            patch.rate_limit_per_hour = Some(per_hour);
        }
        let per_reporter = self.per_reporter.trim().parse::<i64>().unwrap_or(0);
        if per_reporter > 0 && per_reporter != project.rate_limit_per_reporter_per_hour {
            patch.rate_limit_per_reporter_per_hour = Some(per_reporter);
        }
        if self.webhook_url.trim() != project.webhook_url {
            patch.webhook_url = Some(self.webhook_url.trim().to_string());
        }
        // El secreto sólo viaja cuando hay uno nuevo: en blanco significa
        // «deja el que hay», no «deja de firmar».
        if !self.webhook_secret.is_empty() {
            patch.webhook_secret = Some(self.webhook_secret.clone());
        }
        if self.assignee != project.default_assignee_user_id.clone().unwrap_or_default() {
            patch.default_assignee_user_id = Some(self.assignee.clone());
        }
        // La bandeja no se puede vaciar —el servidor lo rechaza, porque un
        // canal sin lista pierde todo lo que le manden—: sólo se cambia por otra.
        if !self.list_id.is_empty() && self.list_id != project.list_id.clone().unwrap_or_default() {
            patch.list_id = Some(self.list_id.clone());
        }
        patch
    }
}

impl Orgs {
    /// Los proyectos de reportes de la organización, con lo que hace falta
    /// para pintarlos.
    pub async fn integrations(&self, org_id: &str) -> Result<Vec<ReportProject>, ApiError> {
        self.report_projects(org_id).await
    }

    /// Crea una y devuelve su llave de ingesta, que el servidor entrega aquí
    /// y no vuelve a dar: guarda un hash y nada más.
    pub async fn create_report_project(&self, org_id: &str, name: &str) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Created {
            ingest_key: String,
        }
        let created: Created = self.core.call("POST", "/api/v1/report-projects/", Some(serde_json::json!({ "orgId": org_id, "name": name.trim() }))).await?;
        Ok(created.ingest_key)
    }

    /// Una llave nueva; la anterior deja de valer en ese momento.
    pub async fn rotate_report_key(&self, id: &str) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Rotated {
            ingest_key: String,
        }
        let rotated: Rotated = self.core.call("POST", &format!("/api/v1/report-projects/{id}/rotate-key"), Some(serde_json::json!({}))).await?;
        Ok(rotated.ingest_key)
    }

    pub async fn update_report_project(&self, id: &str, patch: &ProjectPatch) -> Result<(), ApiError> {
        self.core.call_ok("PATCH", &format!("/api/v1/report-projects/{id}"), Some(serde_json::json!(patch))).await
    }

    /// Pausar y reanudar mandan sólo la bandera.
    pub async fn set_report_project_active(&self, id: &str, active: bool) -> Result<(), ApiError> {
        self.update_report_project(id, &ProjectPatch { is_active: Some(active), ..Default::default() }).await
    }

    pub async fn delete_report_project(&self, id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("/api/v1/report-projects/{id}"), None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ReportProject {
        ReportProject {
            id: "p".into(),
            name: "boaty".into(),
            org_id: "org-1".into(),
            rate_limit_per_hour: 500,
            rate_limit_per_reporter_per_hour: 7,
            is_active: true,
            webhook_url: "https://boaty.app/hooks/cac".into(),
            webhook_configured: true,
            list_id: Some("li-honda".into()),
            ..Default::default()
        }
    }

    fn tree() -> Vec<SpaceTree> {
        serde_json::from_value(serde_json::json!([
            { "id": "esp-1", "orgId": "org-1", "name": "Boaty", "lists": [{ "id": "li-root", "name": "Inbox" }],
              "folders": [{ "id": "car-1", "name": "web", "lists": [{ "id": "li-honda", "name": "Tasks" }], "folders": [] }] },
            { "id": "esp-9", "orgId": "org-otra", "name": "Latinet", "lists": [{ "id": "li-ajena", "name": "salud en casa" }], "folders": [] },
        ]))
        .unwrap()
    }

    /// Roturas que atrapa: ofrecer las listas de otra organización; olvidar
    /// las de las carpetas (o las que cuelgan del espacio); decir una ruta
    /// sin su espacio; pintar la ruta de una lista que no es de aquí.
    #[test]
    fn the_lists_offered_are_this_organizations_and_say_their_path() {
        let lists = inbox_lists(&tree(), "org-1");
        assert_eq!(lists.iter().map(ListPath::path).collect::<Vec<_>>(), ["Boaty · Inbox", "Boaty · web · Tasks"]);
        assert_eq!(list_path(&tree(), "org-1", "li-honda").as_deref(), Some("Boaty · web · Tasks"));
        assert_eq!(list_path(&tree(), "org-1", "li-ajena"), None);
        assert_eq!(list_path(&tree(), "org-1", "li-borrada"), None);
        let groups = lists_by_space(lists);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "Boaty");
        assert_eq!(groups[0].1[1].rest, "web · Tasks");
    }

    /// Roturas que atrapa: mandar el formulario entero (cambiar el nombre
    /// borraba el webhook de un cliente); no mandar el `""` que retira el
    /// webhook; mandar el secreto en blanco (dejaría de firmar); poder dejar
    /// la integración sin lista; un límite que no es un número.
    #[test]
    fn saving_sends_only_what_changed() {
        let p = project();
        assert!(ProjectForm::of(&p).patch(&p).is_empty());
        let renamed = ProjectForm { name: " boaty v2 ".into(), ..ProjectForm::of(&p) };
        assert_eq!(serde_json::to_value(renamed.patch(&p)).unwrap(), serde_json::json!({ "name": "boaty v2" }));
        let cleared = ProjectForm { webhook_url: String::new(), ..ProjectForm::of(&p) };
        assert_eq!(serde_json::to_value(cleared.patch(&p)).unwrap(), serde_json::json!({ "webhookUrl": "" }));
        let secret = ProjectForm { webhook_secret: "s3".into(), ..ProjectForm::of(&p) };
        assert_eq!(serde_json::to_value(secret.patch(&p)).unwrap(), serde_json::json!({ "webhookSecret": "s3" }));
        let no_list = ProjectForm { list_id: String::new(), ..ProjectForm::of(&p) };
        assert!(no_list.patch(&p).is_empty(), "dejarla sin lista perdería lo que le manden");
        let other = ProjectForm { list_id: "li-root".into(), assignee: "u2".into(), per_hour: "900".into(), per_reporter: "abc".into(), ..ProjectForm::of(&p) };
        assert_eq!(serde_json::to_value(other.patch(&p)).unwrap(), serde_json::json!({ "rateLimitPerHour": 900, "defaultAssigneeUserId": "u2", "listId": "li-root" }));
        let nobody = ReportProject { default_assignee_user_id: Some("u2".into()), ..p.clone() };
        let cleared = ProjectForm { assignee: String::new(), ..ProjectForm::of(&nobody) };
        assert_eq!(serde_json::to_value(cleared.patch(&nobody)).unwrap(), serde_json::json!({ "defaultAssigneeUserId": "" }));
    }
}
