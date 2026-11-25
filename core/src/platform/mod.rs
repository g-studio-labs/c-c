//! Plataforma: los servidores de la organización que se mira y lo que se
//! hace con ellos.
//!
//! Es el cuarto módulo del riel. Aquí van las reglas y el estado; la pintura,
//! en `desktop/src/platform/`. El diseño, en cac: «Diseño · Plataforma en la
//! app nativa». Cada fase añade su fichero junto a éstos.

pub mod activity;
pub mod agent;
pub mod backups;
pub mod deploys;
pub mod detail;
pub mod diagnostics;
pub mod edit;
pub mod install;
pub mod k8s;
pub mod secret_refs;
pub mod provision;
pub mod provision_run;
pub mod servers;
pub mod services;
pub mod swarm;
pub mod users;

use serde::{Deserialize, Serialize};

use crate::stream::ServerEvent;
use crate::Core;
pub use activity::{activity_path, deploy_tone, group_attempts, matches_filter, upsert_entry, ActivityDeployment, ActivityEntry, ActivityFilter, ActivityRow, ActivityState, ActivityTone, WorkflowRun, ACTIVITY_PAGE};
pub use agent::{agent_base, Agent, AgentStream, AGENT_TIMEOUT, PASS_MARGIN, PROBE_TIMEOUT};
pub use backups::{detect_databases, is_age_public_key, Backup, BackupEngine, BackupKind, BackupRecipient, BackupStatus, BackupTarget, BackupTargetForm, BackupsState, BackupsStatus, DetectedDatabase, DEFAULT_BACKUP_TIMEZONE};
pub use deploys::{commit_pick, infer_owner_repo, repo_of_deployable, CommitPick, CommitRow, ci_notice_step, guess_environment, is_sha, repo_from_image, runs_commit, short_ref, short_service_name, uses_short_sha, CiKey, CiNotify, DeployStatus, Deployable, DeployableSettings, Deployment, DeploysState, ImageBuild, NewDeployable, RepoCommit, AGENT_VERSION_BUILDS, AGENT_VERSION_MIGRATES, LIST_PREVIEW, SECRETS_ENTRYPOINT_SNIPPET};
pub use secret_refs::{append_rows, label_to_name, merge_build_args, ref_problems, valid_secret_name, valid_secret_ref, RefProblem, SecretRefRow, SecretRotation, SecretUsage, SecretsState};
pub use detail::{server_tabs, AgentNote, DeployableSummary, IncidentServerRef, IncidentSummary, ProvisioningRun, ServerOverview, ServerTab, AGENT_VERSION_DEPLOYS};
pub use edit::{server_edit_phrase, server_permissions, ServerEditError, ServerField, ServerForm, ServerPermissions, MAX_SERVER_NAME};
pub use install::{agent_update_outcome, AgentAction, AgentIdentity, AgentInstallOutcome, AgentInstallPhase, AgentJob, AgentWait, AGENT_VERSION_LATEST};
pub use servers::{platform_sections, servers_of, PlatformSection, Server, ServerHealth, ServerKind};
pub use services::{filter_services, LogLine, LogStatus, ServiceHealth, ServiceLogs, ServiceReplicas, SwarmService, LOG_LINES_KEPT};
pub use k8s::{cert_warns, cluster_status, days_until, route_links, workloads_failing_first, ClusterBadge, ClusterStatus, Integration, K8sCert, K8sDatastore, K8sGateway, K8sHealth, K8sHub, K8sNode, K8sRoute, K8sRoutes, K8sWorkload, CERT_WARN_DAYS};
pub use provision::{can_approve, config_body, config_changed, from_input, from_items, grant_command, is_stale, json_object, line_diff, ordered_playbooks, provision_permissions, provision_phrase, sorted_json, stable_json, stage_playbooks, to_input, to_items, valid_op_ref, var_fields, visible_requests, with_required, AnsibleManifest, AnsibleTools, ConfigOrigin, ConfigProblem, DiffRow, EffectiveConfig, FlagLevel, InventoryHost, PlanGaps, PlaybookInfo, PlaybookVar, ProjectInfo, ProvisionError, ProvisionPermissions, ProvisionRun, ProvisionState, ProvisioningCheck, ProvisioningFailure, ProvisioningRequest, Risk, RiskFlag, SecretField, SecretRef, ServerConfigSummary, ServerConfigVersion, ServerConnection, ServerTemplate, ServerTemplateVersion, TemplateDraft, VarField, VarFields, GRANT_MINUTES, LIVE_STATUSES, RECENT_CLOSED, STALE_CHECK_SECONDS};
pub use provision_run::{final_status, log_tail, parse_extra_vars, play_recap, strip_ansi, AnsibleEngine, EngineEvent, EngineSink, ManualInputs, ManualRun, ManualVar, ProvisionOutput, RunEnd, RunKind, MAX_LINES, TAIL_LINES};
pub use swarm::{cpu_load, format_bytes, mem_load, stats_view, ContainerStats, Load, StatsColumn, StatsSort, StatsView, SwarmNode};

/// Lo que se pinta de Plataforma.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct PlatformState {
    /// Se está en Plataforma: el panel son sus filas y la vista, sus páginas.
    pub on_screen: bool,
    /// Qué fila del panel está en pantalla.
    pub section: PlatformSection,
    /// De qué organización son los servidores de abajo.
    pub org_id: Option<String>,
    /// Los servidores de esa organización, en el orden del backend.
    pub servers: Vec<Server>,
    /// Si ya llegó la lista alguna vez: sin ella, «no hay servidores» sería
    /// mentira.
    pub loaded: bool,
    pub loading: bool,
    /// La frase del último fallo al pedirla. La lista que hubiera se queda.
    pub error: Option<String>,
    /// El servidor abierto, si se entró en uno.
    pub open_server: Option<String>,
    /// La pestaña del servidor abierto.
    pub tab: ServerTab,
    /// Su resumen, cuando llegó.
    pub overview: Option<ServerOverview>,
    pub permissions: ServerPermissions,
    /// La instalación del agente en curso, si hay una.
    pub agent_job: Option<AgentJob>,
    /// Los servicios del servidor abierto, cuando contestó su agente.
    pub services: Option<Vec<SwarmService>>,
    pub services_loading: bool,
    /// Por qué no contestó. Los que hubiera se quedan.
    pub services_error: Option<String>,
    /// El log que se está siguiendo.
    pub logs: Option<ServiceLogs>,
    /// Los nodos del swarm, cuando contestó su agente.
    pub nodes: Option<Vec<SwarmNode>>,
    pub nodes_error: Option<String>,
    /// Lo que gasta cada contenedor, en la última lectura. Se queda al
    /// pausar y si falla la siguiente.
    pub stats: Option<Vec<ContainerStats>>,
    pub stats_loading: bool,
    pub stats_error: Option<String>,
    /// Pausado a mano: lo último leído se queda en pantalla.
    pub stats_paused: bool,
    /// Cuándo llegó la última lectura (RFC 3339).
    pub stats_fetched_at: Option<String>,
    /// La consola del servidor kubernetes abierto.
    pub k8s: Option<K8sHub>,
    pub k8s_loading: bool,
    /// La etiqueta de cada clúster de la lista, cuando se leyó su salud.
    pub cluster_badges: Vec<ClusterBadge>,
    /// Los despliegues del servidor abierto, desde que se miró su pestaña.
    pub deploys: Option<DeploysState>,
    /// Los secrets del desplegable que se mira, por referencia.
    pub secrets: Option<SecretsState>,
    /// Provisioning del servidor abierto, desde que se miró su pestaña.
    pub provision: Option<ProvisionState>,
    /// La salida de lo que corre desde esta app, o de lo último. Sobrevive a
    /// salir del servidor: una ejecución sigue aunque nadie la mire.
    pub provision_output: Option<ProvisionOutput>,
    /// Los backups del servidor abierto, desde que se miró su pestaña.
    pub backups: Option<BackupsState>,
    /// Actividad, desde que se abrió.
    pub activity: Option<activity::ActivityState>,
    /// Usuarios, desde que se abrió (sólo superadmin).
    pub users: Option<users::UsersState>,
    /// Diagnóstico, desde que se abrió.
    pub diagnostics: Option<diagnostics::DiagnosticsState>,
}

#[derive(Default)]
pub(crate) struct PlatformData {
    on_screen: bool,
    section: PlatformSection,
    /// Los de todas las organizaciones de la persona, como llegan.
    all: Option<Vec<Server>>,
    loading: bool,
    error: Option<String>,
    open_server: Option<String>,
    tab: ServerTab,
    overview: Option<ServerOverview>,
    agent_job: Option<AgentJob>,
    services: Option<Vec<SwarmService>>,
    services_loading: bool,
    services_error: Option<String>,
    logs: Option<ServiceLogs>,
    /// Quien sigue el log. Se corta al cerrarlo: un servicio callado deja la
    /// lectura esperando para siempre, y no se puede esperar a que acabe.
    logs_task: Option<tokio::task::AbortHandle>,
    nodes: Option<Vec<SwarmNode>>,
    nodes_error: Option<String>,
    stats: Option<Vec<ContainerStats>>,
    stats_loading: bool,
    stats_error: Option<String>,
    stats_paused: bool,
    stats_fetched_at: Option<String>,
    /// El sondeo de la pestaña de recursos: vive mientras se mira, sin
    /// pausa. Cada lectura recorre todos los contenedores de la máquina.
    stats_task: Option<tokio::task::AbortHandle>,
    k8s: Option<K8sHub>,
    k8s_loading: bool,
    cluster_badges: Vec<ClusterBadge>,
    deploys: Option<DeploysState>,
    secrets: Option<SecretsState>,
    provision: Option<ProvisionState>,
    provision_output: Option<provision_run::OutputData>,
    backups: Option<BackupsState>,
    activity: Option<activity::ActivityState>,
    users: Option<users::UsersState>,
    diagnostics: Option<diagnostics::DiagnosticsState>,
}

/// Plataforma en este core.
pub struct Platform {
    core: Core,
}

impl Core {
    pub fn platform(&self) -> Platform {
        Platform { core: self.clone() }
    }
}

impl Platform {
    pub fn state(&self) -> PlatformState {
        let org_id = self.core.orgs().state().current_org_id;
        let data = self.data();
        // Una sesión que deja de ser superadmin no se queda mirando Usuarios.
        let section = if data.section == PlatformSection::Users && !self.superadmin() { PlatformSection::Servers } else { data.section };
        PlatformState {
            on_screen: data.on_screen,
            section,
            servers: data.all.as_deref().map(|all| servers_of(all, org_id.as_deref())).unwrap_or_default(),
            org_id,
            loaded: data.all.is_some(),
            loading: data.loading,
            error: data.error.clone(),
            open_server: data.open_server.clone(),
            tab: data.tab,
            overview: data.overview.clone(),
            permissions: self.permissions(),
            agent_job: data.agent_job.clone(),
            services: data.services.clone(),
            services_loading: data.services_loading,
            services_error: data.services_error.clone(),
            logs: data.logs.clone(),
            nodes: data.nodes.clone(),
            nodes_error: data.nodes_error.clone(),
            stats: data.stats.clone(),
            stats_loading: data.stats_loading,
            stats_error: data.stats_error.clone(),
            stats_paused: data.stats_paused,
            stats_fetched_at: data.stats_fetched_at.clone(),
            k8s: data.k8s.clone(),
            k8s_loading: data.k8s_loading,
            cluster_badges: data.cluster_badges.clone(),
            deploys: data.deploys.clone(),
            secrets: data.secrets.clone(),
            provision: data.provision.clone().map(|mut provision| {
                provision.permissions = self.provision_permissions();
                provision.me = self.me();
                provision
            }),
            provision_output: data.provision_output.as_ref().map(|out| out.output.clone()),
            backups: data.backups.clone(),
            activity: data.activity.clone(),
            users: data.users.clone().filter(|_| self.superadmin()),
            diagnostics: data.diagnostics.clone(),
        }
    }

    fn permissions(&self) -> ServerPermissions {
        let role = self.core.orgs().state().current().map(|org| org.role.clone()).unwrap_or_default();
        let superadmin = self.core.session().is_some_and(|session| session.superadmin);
        server_permissions(&role, superadmin)
    }

    /// Da de alta un servidor en la organización que se mira.
    pub async fn create_server(&self, form: &ServerForm) -> Result<(), ServerEditError> {
        let permissions = self.permissions();
        if !permissions.create {
            return Err(ServerEditError::ReadOnly);
        }
        let fields = form.validate()?;
        if fields.kind == ServerKind::Kubernetes && !permissions.kubernetes {
            return Err(ServerEditError::KubernetesSuperadminOnly);
        }
        let org_id = self.core.orgs().state().current_org_id.ok_or(ServerEditError::NoOrganization)?;
        let mut body = fields.body();
        body["orgId"] = serde_json::Value::String(org_id);
        let created: Server = self.core.call("POST", "/api/v1/servers/", Some(body)).await?;
        self.edit_list(|all| all.push(created));
        Ok(())
    }

    /// Edita un servidor de la organización que se mira. Si nada cambió, no
    /// se manda.
    pub async fn update_server(&self, server_id: &str, form: &ServerForm) -> Result<(), ServerEditError> {
        let permissions = self.permissions();
        if !permissions.edit {
            return Err(ServerEditError::ReadOnly);
        }
        let server = self.server(server_id)?;
        let fields = form.validate()?;
        if fields.unchanged(&server) {
            return Ok(());
        }
        let touches_kubernetes = fields.kind != server.kind && (fields.kind == ServerKind::Kubernetes || server.kind == ServerKind::Kubernetes);
        if touches_kubernetes && !permissions.kubernetes {
            return Err(ServerEditError::KubernetesSuperadminOnly);
        }
        if !fields.same_connection(&server) && !permissions.edit_connection {
            return Err(ServerEditError::ConnectionAdminOnly);
        }
        let updated: Server = self.core.call("PATCH", &format!("/api/v1/servers/{server_id}"), Some(fields.body())).await?;
        self.edit_list(|all| {
            if let Some(slot) = all.iter_mut().find(|server| server.id == updated.id) {
                *slot = updated;
            }
        });
        Ok(())
    }

    /// Borra un servidor de cac (la máquina no se toca). **Quien llama ya
    /// preguntó.**
    pub async fn delete_server(&self, server_id: &str) -> Result<(), ServerEditError> {
        if !self.permissions().delete {
            return Err(ServerEditError::AdminOnly);
        }
        self.server(server_id)?;
        self.core.call_ok("DELETE", &format!("/api/v1/servers/{server_id}"), None).await?;
        self.edit_list(|all| all.retain(|server| server.id != server_id));
        if self.data().open_server.as_deref() == Some(server_id) {
            self.close_server();
        }
        Ok(())
    }

    /// Instala o actualiza el agente de un servidor: el backend acuña su
    /// identidad, `ssh` la lleva a la máquina (en un hilo aparte: bloquea) y
    /// se espera a que el agente nuevo lata. **Quien llama ya preguntó**: la
    /// identidad nueva tumba la anterior, y si el SSH falla el agente se
    /// queda sin hablar hasta que se reinstale.
    pub async fn install_agent<F>(&self, server_id: &str, wait: AgentWait, ssh: F) -> Result<AgentInstallOutcome, ServerEditError>
    where
        F: FnOnce(&Server, AgentIdentity) -> Result<(), String> + Send + 'static,
    {
        if !self.permissions().install_agent {
            return Err(ServerEditError::AdminOnly);
        }
        let before = self.server(server_id)?;
        let mut started = false;
        self.change(|data| {
            if data.agent_job.is_none() {
                data.agent_job = Some(AgentJob { server_id: server_id.to_string(), phase: AgentInstallPhase::Installing });
                started = true;
            }
        });
        if !started {
            return Err(ServerEditError::AgentBusy);
        }
        let result = self.run_install(before, wait, ssh).await;
        self.change(|data| data.agent_job = None);
        result
    }

    async fn run_install<F>(&self, before: Server, wait: AgentWait, ssh: F) -> Result<AgentInstallOutcome, ServerEditError>
    where
        F: FnOnce(&Server, AgentIdentity) -> Result<(), String> + Send + 'static,
    {
        let id = before.id.clone();
        let identity: AgentIdentity = self.core.call("POST", &format!("/api/v1/servers/{id}/agent-token"), Some(serde_json::json!({}))).await?;
        let target = before.clone();
        let carried = tokio::task::spawn_blocking(move || ssh(&target, identity))
            .await
            .map_err(|error| ServerEditError::Ssh(error.to_string()))
            .and_then(|carried| carried.map_err(ServerEditError::Ssh));
        if let Err(error) = carried {
            self.reload_server(&id).await;
            return Err(error);
        }
        self.change(|data| {
            if let Some(job) = data.agent_job.as_mut() {
                job.phase = AgentInstallPhase::Waiting;
            }
        });
        let deadline = tokio::time::Instant::now() + wait.total;
        loop {
            tokio::time::sleep(wait.every).await;
            if let Ok(now) = self.core.get::<Server>(&format!("/api/v1/servers/{id}")).await {
                if let Some(outcome) = agent_update_outcome(&before, &now) {
                    self.edit_list(|all| {
                        if let Some(slot) = all.iter_mut().find(|server| server.id == now.id) {
                            *slot = now;
                        }
                    });
                    return Ok(outcome);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                self.reload_server(&id).await;
                return Ok(AgentInstallOutcome::Silent);
            }
        }
    }

    /// Ya acuñada la identidad, la fila tiene que saberlo aunque el agente
    /// no haya vuelto: sin `has_agent_token`, Servicios iba sin pase, el
    /// agente nuevo contestaba 401 y no se reintentaba hasta salir y volver.
    async fn reload_server(&self, server_id: &str) {
        if let Ok(now) = self.core.get::<Server>(&format!("/api/v1/servers/{server_id}")).await {
            self.edit_list(|all| {
                if let Some(slot) = all.iter_mut().find(|server| server.id == now.id) {
                    *slot = now;
                }
            });
        }
    }

    fn server(&self, server_id: &str) -> Result<Server, ServerEditError> {
        self.state().servers.into_iter().find(|server| server.id == server_id).ok_or(ServerEditError::ServerGone)
    }

    /// Lo que contestó el servidor, a la lista que se tiene, sin volver a
    /// pedirla entera; y el número del riel, que cuenta de ella.
    fn edit_list(&self, edit: impl FnOnce(&mut Vec<Server>)) {
        self.change(|data| {
            if let Some(all) = data.all.as_mut() {
                edit(all);
            }
        });
        if let Some(all) = self.data().all.clone() {
            self.core.counters().servers_seen(&all);
        }
    }

    /// Entrar a Plataforma. Sólo un módulo está en pantalla a la vez: deja
    /// comunicación si se estaba en ella.
    pub fn enter(&self) {
        self.core.comms().leave();
        self.core.overview().leave();
        self.change(|data| data.on_screen = true);
    }

    /// Volver a lo demás. El servidor abierto se queda para la vuelta; el
    /// log y el sondeo de recursos no, que nadie los mira: el sondeo vuelve
    /// con `start_stats` al entrar.
    pub fn leave(&self) {
        if self.data().on_screen {
            self.change(|data| {
                data.on_screen = false;
                stop_logs(data);
                stop_stats(data);
            });
        }
    }

    /// Pide los servidores y, a los agentes de los de antes, si contestan.
    ///
    /// Lo que falle deja la lista que hubiera: un fallo de red no es «no
    /// tienes servidores».
    pub async fn refresh(&self) {
        self.change(|data| {
            data.loading = true;
            data.error = None;
        });
        match self.core.get_list::<Server>("/api/v1/servers/").await {
            Ok(servers) => {
                self.core.counters().servers_seen(&servers);
                self.change(|data| {
                    data.all = Some(servers);
                    data.loading = false;
                });
                self.probe_old_agents().await;
                self.load_cluster_badges().await;
            }
            Err(error) => self.change(|data| {
                data.error = Some(error.to_string());
                data.loading = false;
            }),
        }
    }

    /// Entra en un servidor, por su resumen. Uno que no es de la
    /// organización que se mira no se abre: cambiar de organización con un
    /// servidor abierto dejaba en pantalla el de la otra, y era fácil
    /// desplegar en el equivocado (`ServerLayout.tsx`).
    pub fn open_server(&self, server_id: &str) -> bool {
        if !self.state().servers.iter().any(|server| server.id == server_id) {
            return false;
        }
        self.change(|data| {
            close(data);
            data.section = PlatformSection::Servers;
            data.open_server = Some(server_id.to_string());
        });
        true
    }

    /// Abre un servidor y pide su resumen.
    pub async fn open_server_and_load(&self, server_id: &str) {
        if !self.open_server(server_id) {
            return;
        }
        if self.open().is_some_and(|server| server.kind == ServerKind::Kubernetes) {
            let platform = self.core.platform();
            let console = tokio::spawn(async move { platform.load_k8s().await });
            self.load_overview().await;
            let _ = console.await;
        } else {
            self.load_overview().await;
        }
    }

    /// Lee la consola del servidor kubernetes abierto: rutas, salud e
    /// integraciones, a la vez. Lo que falle se queda vacío y lo dice
    /// `error`; lo que hubiera de antes no se borra.
    pub async fn load_k8s(&self) {
        let Some(server_id) = self.data().open_server.clone() else { return };
        self.change(|data| data.k8s_loading = true);
        let route = |what: &str| format!("/api/v1/servers/{server_id}/{what}");
        let (core, path) = (self.core.clone(), route("k8s/routes"));
        let routes = tokio::spawn(async move { core.get::<K8sRoutes>(&path).await });
        let (core, path) = (self.core.clone(), route("k8s/health"));
        let health = tokio::spawn(async move { core.get::<K8sHealth>(&path).await });
        let (core, path) = (self.core.clone(), route("integrations"));
        let integrations = tokio::spawn(async move { core.get_list::<Integration>(&path).await });
        let (routes, health, integrations) = (flatten(routes.await), flatten(health.await), flatten(integrations.await));
        let error = [routes.as_ref().err(), health.as_ref().err(), integrations.as_ref().err()].into_iter().flatten().next().cloned();
        if let Ok(health) = &health {
            let badge = ClusterBadge { server_id: server_id.clone(), status: cluster_status(health) };
            self.change(|data| set_badge(data, badge));
        }
        self.change(|data| {
            if data.open_server.as_deref() != Some(server_id.as_str()) {
                return;
            }
            data.k8s_loading = false;
            let before = data.k8s.take().filter(|hub| hub.server_id == server_id).unwrap_or_default();
            data.k8s = Some(K8sHub {
                server_id: server_id.clone(),
                routes: routes.ok().or(before.routes),
                health: health.ok().or(before.health),
                integrations: integrations.unwrap_or(before.integrations),
                error,
            });
        });
    }

    /// La salud de cada clúster de la lista, para su etiqueta. Uno que no
    /// contesta queda «desconocido», no caído.
    async fn load_cluster_badges(&self) {
        let clusters: Vec<String> = self.data().all.iter().flatten().filter(|server| server.kind == ServerKind::Kubernetes).map(|server| server.id.clone()).collect();
        let reads: Vec<_> = clusters
            .into_iter()
            .map(|id| {
                let core = self.core.clone();
                tokio::spawn(async move {
                    let status = match core.get::<K8sHealth>(&format!("/api/v1/servers/{id}/k8s/health")).await {
                        Ok(health) => cluster_status(&health),
                        Err(error) => ClusterStatus::Unknown { detail: error.to_string() },
                    };
                    ClusterBadge { server_id: id, status }
                })
            })
            .collect();
        for read in reads {
            if let Ok(badge) = read.await {
                self.change(|data| set_badge(data, badge));
            }
        }
    }

    /// El secreto de una integración, para quien puede verlo. No se guarda
    /// en el estado: lo enseña quien lo pidió, y nada más.
    pub async fn reveal_integration(&self, integration_id: &str) -> Result<String, ServerEditError> {
        if !self.permissions().reveal_integrations {
            return Err(ServerEditError::ReadOnly);
        }
        let server = self.open().ok_or(ServerEditError::ServerGone)?;
        #[derive(Deserialize)]
        struct Revealed {
            secret: String,
        }
        let revealed: Revealed = self.core.call("POST", &format!("/api/v1/servers/{}/integrations/{integration_id}/reveal", server.id), Some(serde_json::json!({}))).await?;
        Ok(revealed.secret)
    }

    /// A dónde se abre una integración. Las que entran por cac piden su
    /// enlace al proxy (`tools.guz-studio.dev`), nunca al origen de cac; las
    /// demás abren su propia dirección.
    pub async fn integration_link(&self, integration: &Integration) -> Result<String, crate::ApiError> {
        if !integration.through_cac() {
            return Ok(integration.url.clone());
        }
        #[derive(Deserialize)]
        struct Launch {
            url: Option<String>,
        }
        let launch: Launch = self.core.call("POST", &format!("/api/v1/servers/{}/integrations/{}/launch", integration.server_id, integration.id), Some(serde_json::json!({}))).await?;
        Ok(launch.url.unwrap_or_else(|| integration.url.clone()))
    }

    /// Vuelve a la lista de servidores.
    pub fn close_server(&self) {
        self.change(|data| {
            close(data);
            data.section = PlatformSection::Servers;
        });
    }

    /// Cambia de pestaña. Una que ese servidor no tiene no se elige. El log
    /// que se siguiera es de la pestaña que se deja.
    pub fn select_tab(&self, tab: ServerTab) -> bool {
        let Some(server) = self.open() else { return false };
        if !server_tabs(server.kind).contains(&tab) {
            return false;
        }
        // La misma pestaña otra vez no es dejarla: no corta el log.
        if self.data().tab == tab {
            return true;
        }
        self.change(|data| {
            data.tab = tab;
            stop_logs(data);
            stop_stats(data);
        });
        true
    }

    /// Cambia de pestaña y pide lo que ésa enseña.
    pub async fn open_tab(&self, tab: ServerTab) {
        if !self.select_tab(tab) {
            return;
        }
        match tab {
            ServerTab::Services => self.load_services().await,
            ServerTab::Nodes => self.load_nodes().await,
            // De la imagen de sus servicios sale el repo de GitHub.
            ServerTab::Secrets if self.data().services.is_none() => self.load_services().await,
            ServerTab::Deploy => {
                // La lista de la izquierda son los servicios del swarm.
                if self.data().services.is_none() {
                    let platform = self.core.platform();
                    let services = tokio::spawn(async move { platform.load_services().await });
                    self.load_deployables().await;
                    let _ = services.await;
                } else {
                    self.load_deployables().await;
                }
            }
            ServerTab::Provision => self.load_provision().await,
            ServerTab::Backups => {
                // Las bases detectadas salen de los servicios del swarm.
                if self.data().services.is_none() {
                    let platform = self.core.platform();
                    let services = tokio::spawn(async move { platform.load_services().await });
                    self.load_backups().await;
                    let _ = services.await;
                } else {
                    self.load_backups().await;
                }
            }
            ServerTab::Stats => {
                self.start_stats();
                // Los nodos ponen nombre a la columna de nodo.
                if self.data().nodes.is_none() {
                    self.load_nodes().await;
                }
            }
            _ => {}
        }
    }

    /// Pide al agente del servidor abierto sus nodos.
    pub async fn load_nodes(&self) {
        let Some(server) = self.open() else { return };
        let answer = self.core.agent().get_list::<SwarmNode>(&server, "/api/v1/nodes").await;
        self.change(|data| {
            if data.open_server.as_deref() != Some(server.id.as_str()) {
                return;
            }
            match answer {
                Ok(nodes) => {
                    data.nodes = Some(nodes);
                    data.nodes_error = None;
                }
                Err(error) => data.nodes_error = Some(error.to_string()),
            }
        });
    }

    /// Empieza a leer lo que gastan los contenedores, una vez ahora y luego
    /// cada `stats_every`, salvo que esté pausado. Necesita el runtime de
    /// tokio: el escritorio lo llama dentro de `Backend::run`.
    pub fn start_stats(&self) {
        let Some(server) = self.open() else { return };
        let ready = {
            let data = self.data();
            !data.stats_paused && data.tab == ServerTab::Stats
        };
        if !ready {
            return;
        }
        let server_id = server.id.clone();
        let platform = self.core.platform();
        let every = self.core.inner.config.stats_every;
        self.change(stop_stats);
        let task = tokio::spawn(async move {
            loop {
                platform.load_stats(&server).await;
                tokio::time::sleep(every).await;
            }
        });
        let mut data = self.data();
        // Si mientras arrancaba se salió de la pestaña o se pausó, sobra.
        if data.tab == ServerTab::Stats && !data.stats_paused && data.stats_task.is_none() && data.open_server.as_deref() == Some(server_id.as_str()) {
            data.stats_task = Some(task.abort_handle());
        } else {
            task.abort();
        }
    }

    /// Pausa o reanuda el sondeo. Lo leído se queda.
    pub fn pause_stats(&self, paused: bool) {
        self.change(|data| {
            data.stats_paused = paused;
            if paused {
                stop_stats(data);
            }
        });
        if !paused {
            self.start_stats();
        }
    }

    /// Una lectura ahora, aunque esté pausado.
    pub async fn refresh_stats(&self) {
        if let Some(server) = self.open() {
            self.load_stats(&server).await;
        }
    }

    async fn load_stats(&self, server: &Server) {
        // Una lectura que no vuelve no puede tapar las siguientes: el plazo
        // del agente la corta, y mientras tanto no se pide otra.
        let mut busy = false;
        self.change(|data| busy = std::mem::replace(&mut data.stats_loading, true));
        if busy {
            return;
        }
        let answer = self.core.agent().get_list::<ContainerStats>(server, "/api/v1/stats").await;
        self.change(|data| {
            data.stats_loading = false;
            if data.open_server.as_deref() != Some(server.id.as_str()) {
                return;
            }
            match answer {
                Ok(stats) => {
                    data.stats = Some(stats);
                    data.stats_error = None;
                    data.stats_fetched_at = Some(chrono::Utc::now().to_rfc3339());
                }
                Err(error) => data.stats_error = Some(error.to_string()),
            }
        });
    }

    /// Pide al agente del servidor abierto sus servicios.
    pub async fn load_services(&self) {
        let Some(server) = self.open() else { return };
        self.change(|data| {
            data.services_loading = true;
            data.services_error = None;
        });
        let answer = self.core.agent().get_list::<SwarmService>(&server, "/api/v1/services").await;
        self.change(|data| {
            if data.open_server.as_deref() != Some(server.id.as_str()) {
                return;
            }
            data.services_loading = false;
            match answer {
                Ok(services) => data.services = Some(services),
                Err(error) => data.services_error = Some(error.to_string()),
            }
        });
    }

    /// Reinicia un servicio del servidor abierto (`force-update`: Swarm
    /// cambia sus contenedores uno a uno). Un lector no puede: su pase al
    /// agente es de lectura.
    pub async fn restart_service(&self, service_id: &str) -> Result<(), ServerEditError> {
        if !self.permissions().restart_service {
            return Err(ServerEditError::ReadOnly);
        }
        let server = self.open().ok_or(ServerEditError::ServerGone)?;
        let reply = self.core.agent().post(&server, &format!("/api/v1/services/{service_id}/force-update"), None).await?;
        if reply.status >= 400 {
            return Err(crate::session::server_error(&reply).into());
        }
        Ok(())
    }

    /// Sigue el log de un servicio del servidor abierto, en vez del que se
    /// siguiera. Llega a `logs` línea a línea, ya en tramos con su color.
    pub fn follow_logs(&self, service: &SwarmService) {
        let Some(server) = self.open() else { return };
        let platform = self.core.platform();
        let service_id = service.id.clone();
        self.change(|data| {
            stop_logs(data);
            data.logs = Some(ServiceLogs::new(service));
        });
        let task = tokio::spawn(async move { platform.run_logs(server, service_id).await });
        let mut data = self.data();
        // Si mientras arrancaba se cerró o se abrió otro, éste sobra.
        if data.logs.as_ref().is_some_and(|logs| logs.service_id == service.id) && data.logs_task.is_none() {
            data.logs_task = Some(task.abort_handle());
        } else {
            task.abort();
        }
    }

    /// Deja de seguir el log.
    pub fn close_logs(&self) {
        self.change(stop_logs);
    }

    /// Vacía lo que hay en pantalla sin cortar: para ver qué pasa a partir
    /// de ahora.
    pub fn clear_logs(&self) {
        self.change(|data| {
            if let Some(logs) = data.logs.as_mut() {
                logs.lines.clear();
            }
        });
    }

    /// Abre el stream y lo reabre si se corta: tres cortes seguidos y se
    /// deja. Sólo uno que aguantó abierto (`LOG_STEADY`) pone la cuenta a
    /// cero: contando cada apertura, un agente que abre y cierra en el acto
    /// se reconectaba sin fin, cada vez con sus cien últimas líneas otra vez.
    async fn run_logs(&self, server: Server, service_id: String) {
        let path = format!("/api/v1/services/{service_id}/logs");
        let mut failures: u32 = 0;
        loop {
            match self.core.agent().stream(&server, &path).await {
                Ok(mut stream) => {
                    let opened = tokio::time::Instant::now();
                    self.edit_logs(&service_id, |logs| logs.status = LogStatus::Streaming);
                    // Las líneas se juntan en tandas de `LOG_FLUSH`: cada aviso
                    // copia el estado entero y repinta, y un servicio que
                    // escribe mucho lo hacía cientos de veces por segundo.
                    let mut pending: Vec<ServerEvent> = Vec::new();
                    let mut flush_at = tokio::time::Instant::now();
                    loop {
                        let next = if pending.is_empty() {
                            stream.next().await
                        } else {
                            match tokio::time::timeout_at(flush_at, stream.next()).await {
                                Ok(next) => next,
                                Err(_) => {
                                    self.push_logs(&service_id, std::mem::take(&mut pending));
                                    continue;
                                }
                            }
                        };
                        let Some(event) = next else { break };
                        if pending.is_empty() {
                            flush_at = tokio::time::Instant::now() + services::LOG_FLUSH;
                        }
                        pending.push(event);
                    }
                    self.push_logs(&service_id, pending);
                    if opened.elapsed() >= services::LOG_STEADY {
                        failures = 0;
                    }
                }
                Err(error) => self.edit_logs(&service_id, |logs| logs.error = Some(error.to_string())),
            }
            failures += 1;
            if failures >= 3 {
                self.edit_logs(&service_id, |logs| logs.status = LogStatus::Failed);
                return;
            }
            self.edit_logs(&service_id, |logs| logs.status = LogStatus::Reconnecting);
            tokio::time::sleep(crate::stream::backoff(self.core.inner.config.stream_backoff, failures - 1)).await;
        }
    }

    fn push_logs(&self, service_id: &str, events: Vec<ServerEvent>) {
        if events.is_empty() {
            return;
        }
        self.edit_logs(service_id, |logs| {
            for event in events {
                match event.event.as_str() {
                    "error" => logs.error = Some(event.data),
                    _ => logs.push(&event.data),
                }
            }
        });
    }

    fn edit_logs(&self, service_id: &str, edit: impl FnOnce(&mut ServiceLogs)) {
        self.change(|data| {
            if let Some(logs) = data.logs.as_mut().filter(|logs| logs.service_id == service_id) {
                edit(logs);
            }
        });
    }

    /// El servidor abierto, si sigue en la lista.
    pub fn open(&self) -> Option<Server> {
        let state = self.state();
        let id = state.open_server?;
        state.servers.into_iter().find(|server| server.id == id)
    }

    /// Pide el resumen del servidor abierto: lo que ha mordido en él, lo que
    /// se despliega desde cac y lo último que se le aplicó. Lo que falle se
    /// queda vacío. Si mientras llegaba se abrió otro, no se pisa.
    pub async fn load_overview(&self) {
        let Some(server_id) = self.data().open_server.clone() else { return };
        let route = |what: &str| format!("/api/v1/servers/{server_id}/{what}");
        // A la vez, cada una en su tarea: el core no trae las macros de tokio.
        let (core, path) = (self.core.clone(), route("incidents"));
        let incidents = tokio::spawn(async move { core.get::<detail::IncidentPage>(&path).await });
        let (core, path) = (self.core.clone(), route("deployables"));
        let deployables = tokio::spawn(async move { core.get_list::<DeployableSummary>(&path).await });
        let (core, path) = (self.core.clone(), route("provisioning-runs"));
        let runs = tokio::spawn(async move { core.get_list::<ProvisioningRun>(&path).await });
        let overview = ServerOverview {
            server_id: server_id.clone(),
            incidents: incidents.await.ok().and_then(Result::ok).map(|page| page.incidents).unwrap_or_default(),
            deployables: deployables.await.ok().and_then(Result::ok).unwrap_or_default(),
            last_run: runs.await.ok().and_then(Result::ok).and_then(|runs| runs.into_iter().next()),
        };
        self.change(|data| {
            if data.open_server.as_deref() == Some(server_id.as_str()) {
                data.overview = Some(overview);
            }
        });
    }

    /// Se cambió de organización: la lista es la misma respuesta, filtrada
    /// por otra; el servidor abierto era de la anterior.
    pub(crate) fn org_changed(&self) {
        let reload = {
            let data = self.data();
            data.on_screen && data.section == PlatformSection::Activity
        };
        self.change(|data| {
            close(data);
            data.activity = None;
        });
        if reload {
            let platform = self.core.platform();
            tokio::spawn(async move { platform.load_activity().await });
        }
    }

    /// Al salir: los servidores y los pases de una persona no son de la
    /// siguiente.
    pub(crate) fn clear(&self) {
        self.core.agent().clear();
        self.change(|data| {
            stop_logs(data);
            stop_stats(data);
            *data = PlatformData::default();
        });
    }

    /// Pregunta a los agentes sin identidad si contestan, y lo que cambie lo
    /// escribe en el backend (`agent-status`).
    ///
    /// Lo hace la app y no el servidor porque el agente vive en la VPS del
    /// cliente, y quien alcanza esa red es este escritorio. El estado es,
    /// dicho tal cual, «lo que la última app que miró pudo alcanzar». Sin
    /// esto, uno dado de alta se quedaba «pendiente» para siempre aunque su
    /// agente estuviera en pie (`use-servers.ts`).
    async fn probe_old_agents(&self) {
        let candidates: Vec<Server> = self.data().all.iter().flatten().filter(|server| server.needs_probe()).cloned().collect();
        let probes: Vec<_> = candidates
            .into_iter()
            .map(|server| {
                let core = self.core.clone();
                tokio::spawn(async move {
                    let seen = if core.agent().alive(&server).await { ServerHealth::Online } else { ServerHealth::Offline };
                    (server, seen)
                })
            })
            .collect();
        let mut changed = Vec::new();
        for probe in probes {
            if let Ok((server, seen)) = probe.await {
                if server.status != seen {
                    changed.push((server.id, seen));
                }
            }
        }
        if changed.is_empty() {
            return;
        }
        self.edit_list(|all| {
            for server in all.iter_mut() {
                if let Some((_, seen)) = changed.iter().find(|(id, _)| *id == server.id) {
                    server.status = *seen;
                }
            }
        });
        for (id, seen) in changed {
            let Some(status) = seen.as_reported() else { continue };
            // Si no se pudo escribir, la próxima app que mire lo vuelve a
            // intentar: no es algo que enseñar.
            let _ = self.core.call_ok("POST", &format!("/api/v1/servers/{id}/agent-status"), Some(serde_json::json!({ "status": status }))).await;
        }
    }

    fn data(&self) -> std::sync::MutexGuard<'_, PlatformData> {
        self.core.inner.platform.lock().expect("platform lock")
    }

    /// Cada cambio y su aviso van juntos y en fila: si no, dos a la vez
    /// (una línea de log y el cierre del log) podían avisar en orden inverso
    /// y dejar en pantalla el estado viejo.
    fn change(&self, edit: impl FnOnce(&mut PlatformData)) {
        let _order = self.core.inner.platform_order.lock().expect("platform order lock");
        edit(&mut self.data());
        self.core.inner.listener.platform_changed(self.state());
    }
}

pub(super) fn close(data: &mut PlatformData) {
    data.open_server = None;
    data.tab = ServerTab::Overview;
    data.overview = None;
    data.services = None;
    data.services_loading = false;
    data.services_error = None;
    stop_logs(data);
    stop_stats(data);
    data.nodes = None;
    data.nodes_error = None;
    data.stats = None;
    data.stats_error = None;
    data.stats_fetched_at = None;
    data.k8s = None;
    data.k8s_loading = false;
    data.deploys = None;
    data.secrets = None;
    data.provision = None;
    data.backups = None;
}

/// Lo que contestó una tarea de lectura, con su fallo como frase.
fn flatten<T>(joined: Result<Result<T, crate::ApiError>, tokio::task::JoinError>) -> Result<T, String> {
    joined.map_err(|error| error.to_string()).and_then(|answer| answer.map_err(|error| error.to_string()))
}

fn set_badge(data: &mut PlatformData, badge: ClusterBadge) {
    data.cluster_badges.retain(|known| known.server_id != badge.server_id);
    data.cluster_badges.push(badge);
}

fn stop_stats(data: &mut PlatformData) {
    if let Some(task) = data.stats_task.take() {
        task.abort();
        // Abortar a mitad de una lectura se salta el `stats_loading = false`
        // de `load_stats`, y las siguientes se creerían otra en curso.
        data.stats_loading = false;
    }
}

fn stop_logs(data: &mut PlatformData) {
    if let Some(task) = data.logs_task.take() {
        task.abort();
    }
    data.logs = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que atrapa: leer el contrato a mano y que se separe del
    /// backend. El fixture lo escribe `domain.TestCoreContract`.
    #[test]
    fn a_server_is_read_as_the_backend_writes_it() {
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/server_response.json")).unwrap();
        let server: Server = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            (server.name.as_str(), server.address(), server.ssh_port, server.ssh_user.as_str()),
            ("pixel-vps", "203.0.113.7:8080".to_string(), 22, "deploy")
        );
        assert_eq!((server.kind, server.status), (ServerKind::DockerSwarm, ServerHealth::Offline));
        assert!(server.has_agent_token);
        assert_eq!(server.agent_version, 2);
        assert_eq!(server.agent_seen_at.as_deref(), Some("2026-09-30T00:00:00Z"));
    }

    /// Rotura que atrapa: que un tipo o un estado nuevo del backend tire la
    /// lista entera en vez de enseñarse como desconocido.
    #[test]
    fn an_unknown_kind_or_status_does_not_break_the_list() {
        let server: Server = serde_json::from_value(serde_json::json!({
            "id": "s", "orgId": "o", "name": "n", "host": "h", "sshPort": 22, "sshUser": "u",
            "type": "nomad", "agentPort": 1, "status": "rebooting", "agentVersion": 0
        }))
        .unwrap();
        assert_eq!((server.kind, server.status), (ServerKind::Unknown, ServerHealth::Unknown));
        assert!(!server.needs_probe());
    }
}
