//! «Resumen»: con qué te encuentras al abrir la app (command-and-control-511).
//!
//! Vino de `app/src/pages/Overview.tsx`. Cuatro preguntas y sus respuestas, sin
//! tener que ir a cuatro sitios: qué reportó la gente, qué queda por hacer,
//! quién te habló y si la infraestructura está en pie. Cada tarjeta lleva a la
//! pantalla que la contesta entera.
//!
//! - **Lee por su cuenta** los reportes y las tareas, y no reutiliza «Mi
//!   trabajo»: aquella aplica la lente y los filtros que haya puestos, y un
//!   resumen que encoge porque alguien dejó un filtro en otra pantalla estaría
//!   mintiendo sobre cuánto hay.
//! - **Las dos peticiones llevan `orgId`.** Sin él el servidor contesta con
//!   todas tus organizaciones, y «ver más» lleva a «Mi trabajo», que sí acota:
//!   las tareas de otra organización aparecían aquí y desaparecían al pulsar.
//! - **Un reporte hecho no está abierto.** La app de Tauri sólo descartaba
//!   `resolved` y `closed`, pero el estado llega ya normalizado (`resolved` es
//!   `done`), así que lo hecho se contaba como abierto.
//! - Lo sin leer y los servidores no se piden aquí: son los de comunicación y
//!   Plataforma, que ya los tienen.

use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveDate, TimeZone};
use serde::Deserialize;

use crate::comms::{CommsState, Conversation};
use crate::doc_edit::DocLeave;
use crate::inbox::time_ago;
use crate::mywork::{day_title, weekday_name, OpenTask};
use crate::notice::NoticeTarget;
use crate::org_admin::username_initials;
use crate::platform::{Server, ServerHealth, ServerKind};
use crate::stream::ServerEvent;
use crate::texts::{text, Locale};
use crate::Core;

/// Cuántos se piden de cada cosa: los que pide Tauri.
const PAGE: usize = 50;
/// Cuántos se enseñan de cada tarjeta.
const SHOWN: usize = 5;
const SHOWN_UNREAD: usize = 6;

/// Un respiro antes de volver a pedir tras un evento: una ráfaga de cambios es
/// una sola lectura.
const COALESCE: Duration = Duration::from_millis(400);

/// Un reporte de la lista (`domain.ReportListItem`), con lo que el resumen lee.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct OverviewReport {
    pub id: String,
    pub title: String,
    pub folio: String,
    pub project_name: String,
    pub status: String,
    pub priority: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ReportPage {
    items: Vec<OverviewReport>,
}

/// Los nombres con los que el servidor llamó antes a los estados: un servidor
/// sin renombrar todavía contesta `pending` y `resolved`.
fn report_status(raw: &str) -> &str {
    match raw {
        "pending" => "open",
        "resolved" => "done",
        other => other,
    }
}

/// Un reporte sigue abierto mientras no está hecho ni cerrado.
pub fn is_open_report(report: &OverviewReport) -> bool {
    !matches!(report_status(&report.status), "done" | "closed")
}

/// Lo leído del servidor, de una organización.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OverviewState {
    /// Si es la pantalla que se tiene delante.
    pub on_screen: bool,
    /// De qué organización es lo de abajo.
    pub org_id: Option<String>,
    /// Los abiertos, entre los primeros que contestó el servidor.
    pub reports: Vec<OverviewReport>,
    pub tasks: Vec<OpenTask>,
    /// Ya contestó el servidor (bien o mal) para esta organización: hasta
    /// entonces «no entró nada» sería mentira.
    pub reports_loaded: bool,
    pub tasks_loaded: bool,
}

#[derive(Default)]
pub(crate) struct OverviewData {
    state: OverviewState,
}

pub struct Overview {
    core: Core,
}

impl Core {
    pub fn overview(&self) -> Overview {
        Overview { core: self.clone() }
    }
}

impl Overview {
    fn data(&self) -> std::sync::MutexGuard<'_, OverviewData> {
        self.core.inner.overview.lock().expect("overview lock")
    }

    /// Lo de la organización que se mira **ahora**: lo que llegó de otra no se
    /// pinta.
    pub fn state(&self) -> OverviewState {
        let current = self.core.orgs().state().current_org_id.filter(|id| !id.is_empty());
        let state = self.data().state.clone();
        if state.org_id == current {
            state
        } else {
            OverviewState { on_screen: state.on_screen, org_id: current, ..OverviewState::default() }
        }
    }

    fn change(&self, edit: impl FnOnce(&mut OverviewState)) {
        let state = {
            let mut data = self.data();
            edit(&mut data.state);
            data.state.clone()
        };
        self.core.inner.listener.overview_changed(state);
    }

    /// Pone el resumen delante. Es de «Trabajo»: deja comunicación,
    /// Plataforma, la documentación y «Por atender». Con texto sin guardar en
    /// un documento no se va (se pregunta, `doc_edit`), y devuelve `false`.
    pub fn show(&self) -> bool {
        if self.core.docs().held_back(DocLeave::AllWork) {
            return false;
        }
        self.core.docs().close_now();
        self.core.docs().leave_module();
        self.core.comms().leave();
        self.core.platform().leave();
        self.core.requests().leave();
        self.change(|state| state.on_screen = true);
        true
    }

    /// Volver a lo demás. Lo cargado se queda para la vuelta.
    pub fn leave(&self) {
        if self.data().state.on_screen {
            self.change(|state| state.on_screen = false);
        }
    }

    /// Trae lo de la organización que se mira, y los servidores. Lo que falle
    /// deja lo que hubiera: una tarjeta que no carga no es un cero.
    pub async fn load(&self) {
        self.read(true).await;
    }

    /// `servers` también vuelve a preguntar por los servidores: al entrar, no
    /// con cada evento de una tarea.
    async fn read(&self, servers: bool) {
        let Some(org_id) = self.core.orgs().state().current_org_id.filter(|id| !id.is_empty()) else { return };
        let turn = self.core.inner.overview_reads.fetch_add(1, Ordering::SeqCst) + 1;
        let platform = servers.then(|| {
            let core = self.core.clone();
            tokio::spawn(async move { core.platform().refresh().await })
        });
        let (reports_path, tasks_path) = (format!("/api/v1/reports/?limit={PAGE}&orgId={org_id}"), format!("/api/v1/tasks/?limit={PAGE}&orgId={org_id}"));
        let (reports, tasks) = tokio::join!(self.core.get::<ReportPage>(&reports_path), self.core.get_list::<OpenTask>(&tasks_path));
        if let Some(platform) = platform {
            let _ = platform.await;
        }
        // Una lectura que se pidió antes de otra, o de otra organización, no
        // pisa a la de después.
        if self.core.inner.overview_reads.load(Ordering::SeqCst) != turn || self.core.orgs().state().current_org_id.as_deref() != Some(org_id.as_str()) {
            return;
        }
        self.change(|state| {
            if state.org_id.as_deref() != Some(org_id.as_str()) {
                *state = OverviewState { on_screen: state.on_screen, org_id: Some(org_id.clone()), ..OverviewState::default() };
            }
            if let Ok(page) = reports {
                state.reports = page.items.into_iter().filter(is_open_report).collect();
            }
            if let Ok(tasks) = tasks {
                state.tasks = tasks;
            }
            state.reports_loaded = true;
            state.tasks_loaded = true;
        });
    }

    /// Al entrar o al volver de una caída, si se está mirando.
    pub(crate) async fn refresh(&self) {
        if self.data().state.on_screen {
            self.load().await;
        }
    }

    pub(crate) fn server_event(&self, event: &ServerEvent) {
        if !rereads_for(&event.event, self.data().state.on_screen) {
            return;
        }
        let turn = self.core.inner.overview_reloads.fetch_add(1, Ordering::SeqCst) + 1;
        let overview = self.core.overview();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            if overview.core.inner.overview_reloads.load(Ordering::SeqCst) == turn && overview.data().state.on_screen {
                overview.read(false).await;
            }
        });
    }

    /// Al salir: lo de una persona no es de la siguiente.
    pub(crate) fn clear(&self) {
        *self.data() = OverviewData::default();
    }

    /// Abre una tarea o un reporte —el id de un reporte es el de su tarea—.
    pub async fn open_item(&self, id: &str) -> bool {
        let Some(org_id) = self.core.orgs().state().current_org_id else { return false };
        self.core.inbox().open(&format!("/tasks?task={id}"), &org_id).await
    }

    /// Abre una conversación sin leer, en la organización que se mira.
    pub async fn open_conversation(&self, conversation: Conversation) -> bool {
        let org_id = self.core.orgs().state().current_org_id;
        self.core.notices().open(NoticeTarget { conversation, org_id }).await
    }
}

/// Si un evento del servidor pide releer el resumen: sólo los de tareas y
/// reportes, y sólo mientras se mira. Sin estar delante se lee al volver.
fn rereads_for(event: &str, on_screen: bool) -> bool {
    on_screen && (event.starts_with("task:") || event.starts_with("report:"))
}

/// Una fila de «Reportes abiertos» o «Tareas abiertas».
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OverviewRow {
    /// El id de la tarea (el de un reporte es el mismo).
    pub id: String,
    pub title: String,
    pub meta: String,
    /// La meta es un folio: se escribe en monoespaciada.
    pub mono: bool,
    pub status: String,
    pub priority: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OverviewList {
    /// Todavía no contestó el servidor: se dice «cargando», no «vacío».
    pub loading: bool,
    pub title: String,
    pub count: i64,
    pub link: String,
    pub empty: String,
    pub rows: Vec<OverviewRow>,
}

/// Una conversación con gente esperando.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UnreadRow {
    pub key: String,
    pub place: String,
    pub initials: String,
    pub when: String,
    pub count: String,
    pub conversation: Conversation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UnreadCard {
    pub title: String,
    pub count: i64,
    pub link: String,
    pub empty: String,
    pub rows: Vec<UnreadRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Figure {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InfraCard {
    pub loading: bool,
    pub title: String,
    pub count: i64,
    pub link: String,
    pub empty: String,
    pub figures: Vec<Figure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OverviewView {
    pub title: String,
    /// «Dwit México · lunes, 1 de septiembre».
    pub subtitle: String,
    pub reports: OverviewList,
    pub tasks: OverviewList,
    pub unread: UnreadCard,
    pub infra: InfraCard,
}

/// «lunes, 1 de septiembre» (`fechaLarga`).
pub fn long_date(locale: Locale, day: NaiveDate) -> String {
    format!("{}, {}", weekday_name(locale, day.weekday().num_days_from_monday(), false), day_title(locale, day))
}

fn priority_label(locale: Locale, raw: &str) -> String {
    // La API de tareas dice `normal` a lo que se guarda `medium`.
    match if raw == "normal" { "medium" } else { raw } {
        key @ ("low" | "medium" | "high" | "urgent") => text(locale, &format!("work:priority.{key}")),
        _ => raw.to_string(),
    }
}

fn report_status_label(locale: Locale, raw: &str) -> String {
    match report_status(raw) {
        key @ ("open" | "in_progress" | "done" | "closed") => text(locale, &format!("work:status.{key}")),
        other => other.to_string(),
    }
}

/// Lo sin leer, directos primero: al mirarlo no importa por qué puerta entró,
/// sino quién espera respuesta. `comms` ya es de la organización que se mira.
fn unread_rows<Z: TimeZone>(comms: &CommsState, now: &DateTime<Z>, locale: Locale) -> (i64, Vec<UnreadRow>) {
    let directs = comms.directs.iter().filter(|direct| direct.unread > 0).map(|direct| {
        (
            direct.unread,
            UnreadRow {
                key: format!("d-{}", direct.conversation_id),
                place: format!("@{}", direct.username),
                initials: username_initials(&direct.username),
                when: direct.last_message_at.as_deref().map(|at| time_ago(at, now, locale)).unwrap_or_default(),
                count: String::new(),
                conversation: Conversation::Direct { conversation_id: direct.conversation_id.clone() },
            },
        )
    });
    let channels = comms.channels.iter().filter(|channel| channel.unread > 0).map(|channel| {
        (
            channel.unread,
            UnreadRow {
                key: format!("c-{}", channel.space_id),
                place: format!("#{}", channel.name),
                initials: username_initials("channel"),
                when: String::new(),
                count: String::new(),
                conversation: Conversation::Channel { space_id: channel.space_id.clone() },
            },
        )
    });
    let all: Vec<(i64, UnreadRow)> = directs.chain(channels).collect();
    let total = all.iter().map(|(count, _)| count).sum();
    let rows = all
        .into_iter()
        .take(SHOWN_UNREAD)
        .map(|(count, row)| UnreadRow { count: if count > 99 { "99+".into() } else { count.to_string() }, ..row })
        .collect();
    (total, rows)
}

/// La pantalla entera, lista para pintar.
pub fn overview_view<Z: TimeZone>(
    state: &OverviewState,
    comms: &CommsState,
    servers: &[Server],
    servers_loaded: bool,
    org_name: &str,
    today: NaiveDate,
    now: &DateTime<Z>,
    locale: Locale,
) -> OverviewView {
    let see_all = text(locale, "common:admin.seeAll");
    let reports = OverviewList {
        loading: !state.reports_loaded,
        title: text(locale, "common:admin.openReports"),
        count: state.reports.len() as i64,
        link: see_all.clone(),
        empty: text(locale, "common:admin.nothingCameIn"),
        rows: state
            .reports
            .iter()
            .take(SHOWN)
            .map(|report| OverviewRow {
                id: report.id.clone(),
                title: report.title.clone(),
                meta: if report.folio.is_empty() { report.project_name.clone() } else { report.folio.clone() },
                mono: true,
                status: report_status_label(locale, &report.status),
                priority: priority_label(locale, &report.priority),
            })
            .collect(),
    };
    let tasks = OverviewList {
        loading: !state.tasks_loaded,
        title: text(locale, "common:admin.openTasks"),
        count: state.tasks.len() as i64,
        link: see_all,
        empty: text(locale, "common:admin.nothingPending"),
        rows: state
            .tasks
            .iter()
            .take(SHOWN)
            .map(|task| OverviewRow {
                id: task.id.clone(),
                title: task.title.clone(),
                meta: format!("{} · {}", task.space_name, task.list_name),
                mono: false,
                status: task.status_name.clone(),
                priority: priority_label(locale, &task.priority),
            })
            .collect(),
    };
    let (unread_total, unread_rows) = unread_rows(comms, now, locale);
    let online = servers.iter().filter(|server| server.status == ServerHealth::Online).count();
    let orchestrators = servers.iter().filter(|server| server.kind == ServerKind::Kubernetes).count();
    OverviewView {
        title: text(locale, "common:admin.overview"),
        subtitle: if org_name.is_empty() { long_date(locale, today) } else { format!("{org_name} · {}", long_date(locale, today)) },
        reports,
        tasks,
        unread: UnreadCard {
            title: text(locale, "common:admin.unread"),
            count: unread_total,
            link: text(locale, "common:admin.goToTalk"),
            empty: text(locale, "common:admin.nothingWaiting"),
            rows: unread_rows,
        },
        infra: InfraCard {
            loading: !servers_loaded,
            title: text(locale, "common:admin.infrastructure"),
            count: servers.len() as i64,
            link: text(locale, "common:admin.openLink"),
            empty: text(locale, "common:admin.noServersRegistered"),
            figures: vec![
                Figure { value: servers.len().to_string(), label: text(locale, "common:admin.figServers") },
                Figure { value: format!("{online}/{}", servers.len()), label: text(locale, "common:admin.figOnline") },
                Figure { value: orchestrators.to_string(), label: text(locale, "common:admin.figOrchestrators") },
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comms::Channel;
    use crate::comms_wire::DmSummary;
    use chrono::Utc;

    fn report(id: &str, status: &str) -> OverviewReport {
        OverviewReport { id: id.into(), title: format!("r-{id}"), status: status.into(), priority: "high".into(), ..Default::default() }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-25T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 25).unwrap()
    }

    fn view(state: &OverviewState, comms: &CommsState, servers: &[Server]) -> OverviewView {
        overview_view(state, comms, servers, true, "Dwit", today(), &now(), Locale::Es)
    }

    fn direct(id: &str, name: &str, unread: i64) -> DmSummary {
        DmSummary { conversation_id: id.into(), username: name.into(), unread, last_message_at: Some("2026-11-25T11:00:00Z".into()), ..Default::default() }
    }

    fn channel(id: &str, name: &str, unread: i64) -> Channel {
        Channel { space_id: id.into(), name: name.into(), general: false, unread, last: None }
    }

    /// Qué reportes cuentan como abiertos. Roturas que atrapa: contar uno
    /// hecho (lo que hacía la app de Tauri, que normaliza `resolved` a `done` y
    /// luego sólo descarta `resolved`); contar uno cerrado; no entender los
    /// nombres viejos del servidor.
    #[test]
    fn a_done_report_is_not_open() {
        for (status, open) in [("open", true), ("in_progress", true), ("pending", true), ("done", false), ("resolved", false), ("closed", false)] {
            assert_eq!(is_open_report(&report("r", status)), open, "{status}");
        }
    }

    /// Qué eventos releen el resumen. Roturas que atrapa: releer sin estar
    /// delante (una petición por cada evento de tarea, para nada); no releer
    /// los de reportes; releer por un mensaje de chat.
    #[test]
    fn only_task_and_report_events_reread_and_only_in_front() {
        assert!(rereads_for("task:new", true) && rereads_for("report:comment", true));
        assert!(!rereads_for("task:new", false) && !rereads_for("report:new", false));
        assert!(!rereads_for("chat:message", true) && !rereads_for("post:new", true));
    }

    /// Las tarjetas de reportes y tareas. Roturas que atrapa: enseñar más de
    /// cinco; contar sólo lo que se enseña; el folio en vez del proyecto (o al
    /// revés); el espacio y la lista de una tarea; la prioridad cruda
    /// (`normal`) en vez de su frase; el nombre de la organización y la fecha
    /// larga del encabezado.
    #[test]
    fn the_lists_show_five_and_count_all() {
        let mut state = OverviewState { org_id: Some("o1".into()), ..Default::default() };
        state.reports = (1..=7).map(|n| report(&n.to_string(), "open")).collect();
        state.reports[0].folio = "DWIT-12".into();
        state.reports[1].project_name = "Portal".into();
        state.tasks = vec![serde_json::from_value(serde_json::json!({
            "id": "t1", "seq": 1, "title": "Subir", "priority": "normal", "status": "in_progress", "statusName": "En curso", "statusKind": "active",
            "listId": "l1", "listName": "Sprint", "spaceId": "s1", "spaceName": "App", "updatedAt": "2026-11-25T10:00:00Z"
        }))
        .unwrap()];
        let view = view(&state, &CommsState::default(), &[]);

        assert_eq!(view.subtitle, "Dwit · miércoles, 25 de noviembre");
        assert_eq!((view.reports.count, view.reports.rows.len()), (7, 5));
        assert_eq!((view.reports.rows[0].meta.as_str(), view.reports.rows[1].meta.as_str()), ("DWIT-12", "Portal"));
        assert_eq!((view.reports.rows[0].status.as_str(), view.reports.rows[0].priority.as_str()), ("Abierta", "Alta"));
        let task = &view.tasks.rows[0];
        assert_eq!((task.meta.as_str(), task.status.as_str(), task.priority.as_str()), ("App · Sprint", "En curso", "Media"));
        assert_eq!((view.tasks.empty.as_str(), view.reports.empty.as_str()), ("No hay nada pendiente.", "No entró nada."));
    }

    /// Lo sin leer. Roturas que atrapa: poner los canales antes que los
    /// directos; contar sólo lo que se enseña; no cortar a seis filas; escribir
    /// más de 99; enseñar conversaciones sin nada pendiente; el nombre del
    /// canal en un directo.
    #[test]
    fn unread_lists_directs_first_and_caps_the_rows() {
        let mut comms = CommsState::default();
        comms.directs = vec![direct("c1", "ana.ruiz", 2), direct("c2", "bea", 0), direct("c3", "luis", 150)];
        comms.channels = (1..=6).map(|n| channel(&format!("s{n}"), &format!("canal{n}"), n)).collect();
        let view = view(&OverviewState::default(), &comms, &[]);

        assert_eq!(view.unread.count, 2 + 150 + (1 + 2 + 3 + 4 + 5 + 6));
        let places: Vec<&str> = view.unread.rows.iter().map(|row| row.place.as_str()).collect();
        assert_eq!(places, ["@ana.ruiz", "@luis", "#canal1", "#canal2", "#canal3", "#canal4"]);
        assert_eq!(view.unread.rows[0].initials, "AR");
        assert_eq!(view.unread.rows[1].count, "99+");
        assert_eq!(view.unread.rows[0].when, "hace 1 hora");
        assert_eq!(view.unread.rows[2].when, "");
        assert_eq!(view.unread.rows[2].conversation, Conversation::Channel { space_id: "s1".into() });
    }

    /// Antes de que conteste el servidor se dice «cargando», no «vacío».
    /// Roturas que atrapa: pintar «No entró nada» en la organización que
    /// acaba de elegirse; seguir en «cargando» cuando ya contestó.
    #[test]
    fn nothing_is_said_to_be_empty_until_the_server_answered() {
        let mut state = OverviewState::default();
        let before = overview_view(&state, &CommsState::default(), &[], false, "Dwit", today(), &now(), Locale::Es);
        assert!(before.reports.loading && before.tasks.loading && before.infra.loading);
        state.reports_loaded = true;
        let after = overview_view(&state, &CommsState::default(), &[], true, "Dwit", today(), &now(), Locale::Es);
        assert!(!after.reports.loading && after.tasks.loading && !after.infra.loading);
    }

    /// La infraestructura. Roturas que atrapa: contar como en línea lo que no
    /// lo está; contar como orquestador un servidor de swarm.
    #[test]
    fn infrastructure_counts_what_is_online_and_what_orchestrates() {
        let server = |health, kind| Server { status: health, kind, ..Default::default() };
        let servers = [
            server(ServerHealth::Online, ServerKind::DockerSwarm),
            server(ServerHealth::Offline, ServerKind::DockerSwarm),
            server(ServerHealth::Online, ServerKind::Kubernetes),
            server(ServerHealth::Pending, ServerKind::Kubernetes),
        ];
        let view = view(&OverviewState::default(), &CommsState::default(), &servers);
        let figures: Vec<(&str, &str)> = view.infra.figures.iter().map(|figure| (figure.value.as_str(), figure.label.as_str())).collect();
        assert_eq!(figures, [("4", "servidores"), ("2/4", "en línea"), ("2", "orquestadores")]);
        assert_eq!(view.infra.count, 4);
    }
}
