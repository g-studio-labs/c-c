//! Actividad: cada ejecución de GitHub Actions de los repos de la
//! organización y cada despliegue de cac, mezclados por tiempo, lo más nuevo
//! primero (`app/src/pages/Activity.tsx`, `store/activity.store.ts`).
//!
//! Lo que llega por el stream (`ci:run`, `deploy:status`) se funde encima de
//! lo cargado sin volver a pedir: un run que termina mientras se mira cambia
//! de color solo.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use super::{Deployment, DeployStatus, Platform, PlatformSection};
use crate::stream::ServerEvent;

/// Cuántas entradas se piden por página.
pub const ACTIVITY_PAGE: usize = 50;

/// Un intento de una ejecución de GitHub Actions (`domain.WorkflowRun`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowRun {
    pub id: String,
    pub org_id: String,
    pub repo_full_name: String,
    pub run_id: i64,
    pub run_attempt: i64,
    pub run_number: i64,
    pub workflow_name: String,
    /// `.github/workflows/prod.yml`.
    pub path: String,
    pub event: String,
    /// requested | queued | in_progress | waiting | pending | completed.
    pub status: String,
    /// Sólo con `completed`.
    pub conclusion: String,
    pub head_sha: String,
    pub head_branch: String,
    pub commit_title: String,
    pub actor: String,
    pub html_url: String,
    /// El `created_at` del run en GitHub: ordena el feed y no se mueve.
    pub occurred_at: String,
}

/// De qué color va la etiqueta de un run o un despliegue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ActivityTone {
    Success,
    Failure,
    Running,
    Muted,
}

impl WorkflowRun {
    /// La clave del catálogo que dice cómo va: cómo acabó si terminó, su
    /// estado si no.
    pub fn label_key(&self) -> String {
        if self.status == "completed" && !self.conclusion.is_empty() {
            format!("activity:run.conclusion.{}", self.conclusion)
        } else {
            format!("activity:run.status.{}", self.status)
        }
    }

    pub fn tone(&self) -> ActivityTone {
        if self.status != "completed" {
            return if self.status == "in_progress" { ActivityTone::Running } else { ActivityTone::Muted };
        }
        match self.conclusion.as_str() {
            "success" => ActivityTone::Success,
            "failure" | "timed_out" | "startup_failure" => ActivityTone::Failure,
            _ => ActivityTone::Muted,
        }
    }

    /// Sólo se abre lo que es de verdad una página de GitHub: el enlace lo
    /// escribe un webhook.
    pub fn opens_on_github(&self) -> bool {
        self.html_url.starts_with("https://github.com/")
    }
}

pub fn deploy_tone(status: DeployStatus) -> ActivityTone {
    match status {
        DeployStatus::Succeeded => ActivityTone::Success,
        DeployStatus::Failed => ActivityTone::Failure,
        DeployStatus::Running => ActivityTone::Running,
        _ => ActivityTone::Muted,
    }
}

/// Un despliegue como lo enseña la actividad: con el servicio al que
/// pertenece.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ActivityDeployment {
    #[serde(flatten)]
    pub deployment: Deployment,
    pub org_id: String,
    pub deployable_name: String,
    pub deployable_env: String,
    /// El run que lo disparó, si vino de la GitHub App.
    pub workflow_run_id: String,
}

/// Una línea del feed: o un run (con los despliegues que disparó) o un
/// despliegue suelto. `at` es por lo que van ordenadas.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ActivityEntry {
    /// `run` o `deployment`.
    pub kind: String,
    pub at: String,
    pub run: Option<WorkflowRun>,
    pub deployments: Vec<ActivityDeployment>,
    pub deployment: Option<ActivityDeployment>,
}

impl ActivityEntry {
    pub fn is_run(&self) -> bool {
        self.kind == "run"
    }

    /// El id de la fila, sea run o despliegue: el desempate del cursor.
    pub fn id(&self) -> &str {
        match (&self.run, &self.deployment) {
            (Some(run), _) if self.is_run() => &run.id,
            (_, Some(deployment)) => &deployment.deployment.id,
            _ => "",
        }
    }
}

/// Qué parte de la actividad se pide. Vacío es toda la organización.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ActivityFilter {
    /// `owner/repo`.
    pub repo: String,
    pub deployable_id: String,
}

impl ActivityFilter {
    pub fn is_empty(&self) -> bool {
        self.repo.is_empty() && self.deployable_id.is_empty()
    }
}

/// Lo que se pinta de Actividad.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ActivityState {
    pub org_id: String,
    pub filter: ActivityFilter,
    pub entries: Vec<ActivityEntry>,
    pub has_more: bool,
    pub loading: bool,
    /// Si ya llegó la primera página: sin ella, «nada todavía» sería mentira.
    pub loaded: bool,
    pub error: Option<String>,
}

impl ActivityState {
    /// El nombre del servicio del filtro, para su chip: va en sus despliegues.
    pub fn deployable_name(&self) -> String {
        let id = &self.filter.deployable_id;
        if id.is_empty() {
            return String::new();
        }
        self.entries
            .iter()
            .flat_map(|entry| entry.deployment.iter().chain(entry.deployments.iter()))
            .find(|deployment| &deployment.deployment.deployable_id == id)
            .map(|deployment| deployment.deployable_name.clone())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct ActivityPage {
    #[serde(default)]
    items: Vec<ActivityEntry>,
    #[serde(default, rename = "hasMore")]
    has_more: bool,
}

fn instant(at: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(at).ok()
}

/// Si `a` va antes que `b` en pantalla: más nuevo primero, y a igual hora el
/// id mayor. Por el instante y no por el texto: Go escribe los segundos con
/// los decimales que tenga, y «10:00:00Z» quedaba detrás de «10:00:00.5Z».
fn goes_before(a_at: &str, a_id: &str, b_at: &str, b_id: &str) -> bool {
    let order = match (instant(a_at), instant(b_at)) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => a_at.cmp(b_at),
    };
    order.is_gt() || (order.is_eq() && a_id > b_id)
}

/// Si una entrada cae dentro del filtro de la página. Lo que llega en vivo es
/// de toda la organización. Qué run es de qué servicio lo sabe el servidor
/// (por su workflow de build): con ese filtro un run nuevo no entra en vivo, y
/// un despliegue en vivo no dice de qué repo es su servicio.
pub fn matches_filter(entry: &ActivityEntry, filter: &ActivityFilter) -> bool {
    if entry.is_run() {
        let Some(run) = &entry.run else { return false };
        if !filter.deployable_id.is_empty() {
            return false;
        }
        return filter.repo.is_empty() || run.repo_full_name.eq_ignore_ascii_case(&filter.repo);
    }
    let Some(deployment) = &entry.deployment else { return false };
    if !filter.deployable_id.is_empty() && deployment.deployment.deployable_id != filter.deployable_id {
        return false;
    }
    filter.repo.is_empty()
}

/// Un evento no trae el nombre del servicio, su entorno ni el de quien lo
/// pidió: no se los puede borrar a la fila.
fn merge_deployment(before: &ActivityDeployment, next: &ActivityDeployment) -> ActivityDeployment {
    let mut merged = next.clone();
    if merged.deployable_name.is_empty() {
        merged.deployable_name = before.deployable_name.clone();
    }
    if merged.deployable_env.is_empty() {
        merged.deployable_env = before.deployable_env.clone();
    }
    if merged.deployment.requested_by_name.as_deref().unwrap_or_default().is_empty() {
        merged.deployment.requested_by_name = before.deployment.requested_by_name.clone();
    }
    if merged.org_id.is_empty() {
        merged.org_id = before.org_id.clone();
    }
    if merged.workflow_run_id.is_empty() {
        merged.workflow_run_id = before.workflow_run_id.clone();
    }
    merged
}

/// Funde una entrada: si ya estaba, la pisa conservando lo que el evento no
/// trae; si no, entra en su sitio por `at`.
pub fn upsert_entry(entries: &mut Vec<ActivityEntry>, entry: ActivityEntry) {
    let id = entry.id().to_string();
    if let Some(slot) = entries.iter_mut().find(|known| known.kind == entry.kind && known.id() == id) {
        if entry.is_run() {
            let deployments = if entry.deployments.is_empty() { std::mem::take(&mut slot.deployments) } else { entry.deployments };
            *slot = ActivityEntry { deployments, ..entry };
        } else if let (Some(before), Some(next)) = (&slot.deployment, &entry.deployment) {
            slot.deployment = Some(merge_deployment(before, next));
        } else {
            *slot = entry;
        }
        return;
    }
    let at = entries.iter().position(|known| goes_before(&entry.at, &id, &known.at, known.id())).unwrap_or(entries.len());
    entries.insert(at, entry);
}

/// Cuelga un despliegue de su run, si el run está en la página.
fn hang_on_run(entries: &mut [ActivityEntry], deployment: &ActivityDeployment) {
    if deployment.workflow_run_id.is_empty() {
        return;
    }
    for entry in entries.iter_mut().filter(|entry| entry.is_run() && entry.run.as_ref().is_some_and(|run| run.id == deployment.workflow_run_id)) {
        match entry.deployments.iter_mut().find(|known| known.deployment.id == deployment.deployment.id) {
            Some(known) => *known = merge_deployment(known, deployment),
            None => entry.deployments.push(deployment.clone()),
        }
    }
}

/// Una línea tal como se pinta: los intentos de un mismo run, juntos.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ActivityRow {
    pub entry: ActivityEntry,
    /// Los intentos anteriores del mismo run, del primero al penúltimo.
    pub earlier: Vec<WorkflowRun>,
}

/// Los intentos de un run en una sola línea, la del **último** (jose,
/// 7-oct-2026): tres filas sueltas hacían leer el intento 2 fallido como un
/// fallo sin resolver cuando el 3 había pasado. Va en el sitio del primero que
/// aparece (`lib/run-attempts.ts`).
pub fn group_attempts(entries: &[ActivityEntry]) -> Vec<ActivityRow> {
    let mut rows: Vec<ActivityRow> = Vec::new();
    for entry in entries {
        let Some(run) = entry.run.as_ref().filter(|_| entry.is_run()) else {
            rows.push(ActivityRow { entry: entry.clone(), earlier: Vec::new() });
            continue;
        };
        let group = rows.iter_mut().find(|row| row.entry.is_run() && row.entry.run.as_ref().is_some_and(|known| known.run_id == run.run_id));
        let Some(group) = group else {
            rows.push(ActivityRow { entry: entry.clone(), earlier: Vec::new() });
            continue;
        };
        group.entry.deployments.extend(entry.deployments.iter().cloned());
        let shown = group.entry.run.clone().unwrap_or_default();
        if run.run_attempt > shown.run_attempt {
            group.earlier.push(shown);
            group.entry.run = Some(run.clone());
            group.entry.at = entry.at.clone();
        } else {
            group.earlier.push(run.clone());
        }
    }
    for row in &mut rows {
        row.earlier.sort_by_key(|run| run.run_attempt);
    }
    rows
}

/// La ruta de una página de la actividad.
pub fn activity_path(org_id: &str, filter: &ActivityFilter, cursor: Option<&ActivityEntry>) -> String {
    use crate::chat::query_escaped as q;
    let mut query = Vec::new();
    if !filter.repo.is_empty() {
        query.push(format!("repo={}", q(&filter.repo)));
    }
    if !filter.deployable_id.is_empty() {
        query.push(format!("deployableId={}", q(&filter.deployable_id)));
    }
    query.push(format!("limit={ACTIVITY_PAGE}"));
    if let Some(cursor) = cursor {
        query.push(format!("before={}", q(&cursor.at)));
        query.push(format!("beforeId={}", q(cursor.id())));
    }
    format!("/api/v1/organizations/{org_id}/activity/?{}", query.join("&"))
}

impl Platform {
    /// Pone en pantalla otra fila del panel. Usuarios no se abre a quien no
    /// administra la plataforma, aunque se pida. Dejar los servidores cierra
    /// el que estuviera abierto, como cambiar de ruta en Tauri.
    pub fn show_section(&self, section: PlatformSection) -> bool {
        if section == PlatformSection::Users && !self.superadmin() {
            return false;
        }
        self.change(|data| {
            if section != PlatformSection::Servers {
                super::close(data);
            }
            data.section = section;
        });
        true
    }

    pub(crate) fn superadmin(&self) -> bool {
        self.core.session().is_some_and(|session| session.superadmin)
    }

    /// Abre Actividad con ese filtro y pide su primera página.
    pub async fn open_activity(&self, filter: ActivityFilter) {
        if !self.show_section(PlatformSection::Activity) {
            return;
        }
        self.change(|data| {
            let activity = data.activity.get_or_insert_with(ActivityState::default);
            activity.filter = filter;
        });
        self.load_activity().await;
    }

    /// La primera página, para la organización que se mira y el filtro que
    /// haya. Lo que llega cuando ya se pidió otra cosa no se pinta.
    pub async fn load_activity(&self) {
        let Some(org_id) = self.core.orgs().state().current_org_id else { return };
        let mut filter = ActivityFilter::default();
        self.change(|data| {
            let activity = data.activity.get_or_insert_with(ActivityState::default);
            if activity.org_id != org_id {
                *activity = ActivityState { filter: activity.filter.clone(), ..ActivityState::default() };
                activity.org_id = org_id.clone();
            }
            activity.loading = true;
            activity.error = None;
            filter = activity.filter.clone();
        });
        let answer = self.core.get::<ActivityPage>(&activity_path(&org_id, &filter, None)).await;
        self.change(|data| {
            let Some(activity) = data.activity.as_mut().filter(|activity| activity.org_id == org_id && activity.filter == filter) else { return };
            activity.loading = false;
            match answer {
                Ok(page) => {
                    activity.entries = page.items;
                    activity.has_more = page.has_more;
                    activity.loaded = true;
                }
                Err(error) => activity.error = Some(error.to_string()),
            }
        });
    }

    /// La página siguiente, por el cursor de la última entrada.
    pub async fn load_more_activity(&self) {
        let Some(state) = self.data().activity.clone() else { return };
        let Some(last) = state.entries.last().cloned() else { return };
        if !state.has_more || state.loading {
            return;
        }
        self.change(|data| {
            if let Some(activity) = data.activity.as_mut() {
                activity.loading = true;
            }
        });
        let answer = self.core.get::<ActivityPage>(&activity_path(&state.org_id, &state.filter, Some(&last))).await;
        self.change(|data| {
            let Some(activity) = data.activity.as_mut().filter(|activity| activity.org_id == state.org_id && activity.filter == state.filter) else { return };
            activity.loading = false;
            match answer {
                Ok(page) => {
                    for entry in page.items {
                        upsert_entry(&mut activity.entries, entry);
                    }
                    activity.has_more = page.has_more;
                }
                Err(error) => activity.error = Some(error.to_string()),
            }
        });
    }

    /// Lo que cuenta el stream de los runs y los despliegues.
    pub(crate) fn activity_event(&self, event: &ServerEvent) {
        match event.event.as_str() {
            "ci:run" => {
                let Ok(run) = serde_json::from_str::<WorkflowRun>(&event.data) else { return };
                if run.id.is_empty() {
                    return;
                }
                self.change(|data| {
                    let Some(activity) = data.activity.as_mut() else { return };
                    if activity.org_id.is_empty() || run.org_id != activity.org_id {
                        return;
                    }
                    let entry = ActivityEntry { kind: "run".into(), at: run.occurred_at.clone(), run: Some(run.clone()), ..ActivityEntry::default() };
                    let known = activity.entries.iter().any(|known| known.is_run() && known.id() == run.id);
                    if known || matches_filter(&entry, &activity.filter) {
                        upsert_entry(&mut activity.entries, entry);
                    }
                });
            }
            "deploy:status" => {
                let Ok(deployment) = serde_json::from_str::<ActivityDeployment>(&event.data) else { return };
                if deployment.deployment.id.is_empty() {
                    return;
                }
                self.change(|data| {
                    let Some(activity) = data.activity.as_mut() else { return };
                    if activity.org_id.is_empty() {
                        return;
                    }
                    hang_on_run(&mut activity.entries, &deployment);
                    let entry = ActivityEntry { kind: "deployment".into(), at: deployment.deployment.created_at.clone(), deployment: Some(deployment.clone()), ..ActivityEntry::default() };
                    let known = activity.entries.iter().any(|known| !known.is_run() && known.id() == deployment.deployment.id);
                    let ours = deployment.org_id.is_empty() || deployment.org_id == activity.org_id;
                    if known || (ours && matches_filter(&entry, &activity.filter)) {
                        upsert_entry(&mut activity.entries, entry);
                    }
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, run_id: i64, attempt: i64, at: &str, repo: &str) -> ActivityEntry {
        let run = WorkflowRun { id: id.into(), run_id, run_attempt: attempt, occurred_at: at.into(), repo_full_name: repo.into(), ..WorkflowRun::default() };
        ActivityEntry { kind: "run".into(), at: at.into(), run: Some(run), ..ActivityEntry::default() }
    }

    fn deploy(id: &str, deployable: &str, at: &str) -> ActivityEntry {
        let deployment = ActivityDeployment { deployment: Deployment { id: id.into(), deployable_id: deployable.into(), created_at: at.into(), ..Deployment::default() }, ..ActivityDeployment::default() };
        ActivityEntry { kind: "deployment".into(), at: at.into(), deployment: Some(deployment), ..ActivityEntry::default() }
    }

    fn ids(entries: &[ActivityEntry]) -> Vec<&str> {
        entries.iter().map(ActivityEntry::id).collect()
    }

    /// Rotura que atrapa: que con un filtro por repo se cuele el run de otro
    /// (o que no entre el del repo por una mayúscula), o que con filtro por
    /// servicio entre un run o el despliegue de otro servicio.
    #[test]
    fn the_filter_keeps_out_what_is_not_its() {
        let by_repo = ActivityFilter { repo: "guz/cac".into(), ..ActivityFilter::default() };
        assert!(matches_filter(&run("r1", 1, 1, "2026-11-20T10:00:00Z", "Guz/CAC"), &by_repo));
        assert!(!matches_filter(&run("r2", 2, 1, "2026-11-20T10:00:00Z", "guz/otro"), &by_repo));
        assert!(!matches_filter(&deploy("d1", "x", "2026-11-20T10:00:00Z"), &by_repo), "un despliegue en vivo no dice su repo");
        let by_service = ActivityFilter { deployable_id: "web".into(), ..ActivityFilter::default() };
        assert!(matches_filter(&deploy("d1", "web", "2026-11-20T10:00:00Z"), &by_service));
        assert!(!matches_filter(&deploy("d2", "api", "2026-11-20T10:00:00Z"), &by_service));
        assert!(!matches_filter(&run("r1", 1, 1, "2026-11-20T10:00:00Z", "guz/cac"), &by_service));
        assert!(matches_filter(&run("r1", 1, 1, "2026-11-20T10:00:00Z", "guz/cac"), &ActivityFilter::default()));
    }

    /// Rotura que atrapa: meter lo nuevo al final o al principio sin mirar
    /// su hora; ordenar por el texto, que deja «10:00:00Z» detrás de
    /// «10:00:00.5Z»; o duplicar una fila que ya estaba en vez de pisarla.
    #[test]
    fn a_new_entry_goes_in_its_place_and_a_known_one_is_replaced() {
        let mut entries = vec![run("r3", 3, 1, "2026-11-20T12:00:00Z", "a/b"), run("r1", 1, 1, "2026-11-20T10:00:00Z", "a/b")];
        upsert_entry(&mut entries, run("r2", 2, 1, "2026-11-20T11:00:00.5Z", "a/b"));
        upsert_entry(&mut entries, run("r0", 9, 1, "2026-11-20T10:00:00.250Z", "a/b"));
        assert_eq!(ids(&entries), ["r3", "r2", "r0", "r1"]);
        let mut done = run("r2", 2, 1, "2026-11-20T11:00:00.5Z", "a/b");
        done.run.as_mut().unwrap().status = "completed".into();
        upsert_entry(&mut entries, done);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[1].run.as_ref().unwrap().status, "completed");
    }

    /// Rotura que atrapa: que el evento crudo de un despliegue le borre a la
    /// fila el nombre del servicio o el de quien lo pidió.
    #[test]
    fn an_event_does_not_erase_what_it_does_not_bring() {
        let mut known = deploy("d1", "web", "2026-11-20T10:00:00Z");
        let row = known.deployment.as_mut().unwrap();
        row.deployable_name = "shop-web".into();
        row.deployment.requested_by_name = Some("Ana".into());
        let mut entries = vec![known];
        let mut event = deploy("d1", "web", "2026-11-20T10:00:00Z");
        event.deployment.as_mut().unwrap().deployment.status = DeployStatus::Succeeded;
        upsert_entry(&mut entries, event);
        let row = entries[0].deployment.as_ref().unwrap();
        assert_eq!((row.deployable_name.as_str(), row.deployment.requested_by_name.as_deref(), row.deployment.status), ("shop-web", Some("Ana"), DeployStatus::Succeeded));
    }

    /// Rotura que atrapa: enseñar cada intento como fila suelta, quedarse con
    /// el primero en vez del último, o perder el orden de los anteriores.
    #[test]
    fn the_attempts_of_a_run_go_in_one_row_under_the_last() {
        let entries = vec![run("a3", 7, 3, "2026-11-20T12:00:00Z", "a/b"), deploy("d1", "web", "2026-11-20T11:30:00Z"), run("a1", 7, 1, "2026-11-20T10:00:00Z", "a/b"), run("a2", 7, 2, "2026-11-20T11:00:00Z", "a/b")];
        let rows = group_attempts(&entries);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].entry.id(), "a3");
        assert_eq!(rows[0].earlier.iter().map(|run| run.id.as_str()).collect::<Vec<_>>(), ["a1", "a2"]);
    }

    /// Rotura que atrapa: que el filtro no llegue al servidor (se pediría
    /// toda la organización) o el cursor vaya sin su desempate.
    #[test]
    fn the_filter_and_the_cursor_travel_in_the_query() {
        let filter = ActivityFilter { repo: "guz/cac".into(), deployable_id: "d 1".into() };
        let cursor = run("r1", 1, 1, "2026-11-20T10:00:00+01:00", "a/b");
        assert_eq!(
            activity_path("o1", &filter, Some(&cursor)),
            "/api/v1/organizations/o1/activity/?repo=guz%2Fcac&deployableId=d%201&limit=50&before=2026-11-20T10%3A00%3A00%2B01%3A00&beforeId=r1"
        );
    }
}
