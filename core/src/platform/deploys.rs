//! Los despliegues desde cac (`deployments.store.ts` y `lib/deploy.ts`): qué
//! servicios de un servidor se despliegan desde aquí, su historial y el log
//! en vivo de cada despliegue.
//!
//! El despliegue lo hace el agente del servidor; lo que llega aquí es lo que
//! va contando por el stream (`deploy:status`, `deploy:log`, `deploy:build`).
//! Por eso el historial se **funde** con lo que llega y no se vuelve a pedir:
//! un despliegue que avanza no puede depender de que alguien refresque.

use serde::{Deserialize, Serialize};

use super::{Platform, ServerEditError};
use crate::stream::ServerEvent;

/// Un servicio de Swarm que cac sabe desplegar (`domain.Deployable`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct Deployable {
    pub id: String,
    pub org_id: String,
    pub server_id: String,
    pub name: String,
    pub stack: String,
    pub service_name: String,
    /// El repositorio de imágenes, sin tag (`ghcr.io/owner/repo`).
    pub image_repo: String,
    pub environment: String,
    pub repo_full_name: String,
    /// El workflow que publica la imagen (`prod.yml`). Con la GitHub App, que
    /// termine bien cuenta como el aviso del CI.
    pub build_workflow: String,
    /// El CI etiqueta con el sha corto: cac lo recorta.
    pub short_tags: bool,
    /// Lo que corre antes de cada despliegue, con la imagen nueva.
    pub migrate_command: String,
    /// El comando del servidor que recibe los secretos por stdin. Vacío, la
    /// rotación de cac.
    pub rotate_command: String,
    #[serde(rename = "onCINotify")]
    pub on_ci_notify: CiNotify,
    /// Vacía: cualquiera.
    pub branch: String,
    /// El servidor construye la imagen desde el commit (agente v6).
    pub build_on_host: bool,
    pub dockerfile: String,
    pub build_context: String,
    /// El principio de la llave del CI; vacío si no tiene.
    #[serde(rename = "ciKeyPreview")]
    pub ci_key_preview: String,
    pub current_image: String,
    pub previous_image: String,
}

/// Lo que se cambia de un desplegable. El PATCH pide la fila entera: lo que
/// no se toca va como está.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DeployableSettings {
    pub name: String,
    pub environment: String,
    pub repo_full_name: String,
    #[serde(rename = "onCINotify")]
    pub on_ci_notify: CiNotify,
    pub build_workflow: String,
    pub short_tags: bool,
    pub migrate_command: String,
    pub rotate_command: String,
    pub branch: String,
    pub build_on_host: bool,
    pub dockerfile: String,
    pub build_context: String,
}

impl Deployable {
    pub fn settings(&self) -> DeployableSettings {
        DeployableSettings {
            name: self.name.clone(),
            environment: self.environment.clone(),
            repo_full_name: self.repo_full_name.clone(),
            on_ci_notify: self.on_ci_notify,
            build_workflow: self.build_workflow.clone(),
            short_tags: self.short_tags,
            migrate_command: self.migrate_command.clone(),
            rotate_command: self.rotate_command.clone(),
            branch: self.branch.clone(),
            build_on_host: self.build_on_host,
            dockerfile: self.dockerfile.clone(),
            build_context: self.build_context.clone(),
        }
    }
}

/// Qué hace cac cuando el CI avisa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum CiNotify {
    /// Apuntarlo.
    #[default]
    Record,
    /// Apuntarlo y desplegarlo.
    Deploy,
}

impl CiNotify {
    pub fn as_str(self) -> &'static str {
        match self {
            CiNotify::Record => "record",
            CiNotify::Deploy => "deploy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum DeployStatus {
    #[default]
    Queued,
    Running,
    Succeeded,
    Failed,
    RolledBack,
    #[serde(other)]
    Unknown,
}

impl DeployStatus {
    pub fn label_key(self) -> &'static str {
        match self {
            DeployStatus::Queued => "common:deploy.status.queued",
            DeployStatus::Running => "common:deploy.status.running",
            DeployStatus::Succeeded => "common:deploy.status.succeeded",
            DeployStatus::Failed => "common:deploy.status.failed",
            DeployStatus::RolledBack | DeployStatus::Unknown => "common:deploy.status.rolled_back",
        }
    }

    /// Ya no va a cambiar.
    pub fn finished(self) -> bool {
        matches!(self, DeployStatus::Succeeded | DeployStatus::Failed | DeployStatus::RolledBack)
    }
}

/// Un despliegue. El log no viene en el historial: llega por el stream o al
/// pedir uno.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct Deployment {
    pub id: String,
    pub created_at: String,
    pub deployable_id: String,
    pub server_id: String,
    pub image: String,
    pub final_image: String,
    pub previous_image: String,
    /// `user`, `ci` o `github`.
    pub requested_by: String,
    pub requested_by_user_id: String,
    /// Sólo en el historial: el evento no lo trae.
    pub requested_by_name: Option<String>,
    pub status: DeployStatus,
    pub error: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub rollback_of_id: String,
}

/// Lo que hace falta para registrar un servicio.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct NewDeployable {
    pub name: String,
    pub stack: String,
    pub service_name: String,
    pub image_repo: String,
    pub environment: String,
    pub short_tags: bool,
}

impl NewDeployable {
    /// Lo que se deduce de un servicio que corre: su nombre corto, su
    /// repositorio de imágenes, el entorno por el sufijo del stack y si se
    /// etiqueta con el sha corto.
    pub fn from_service(service_name: &str, stack: &str, image: &str) -> NewDeployable {
        NewDeployable {
            name: short_service_name(service_name, stack).to_string(),
            stack: stack.to_string(),
            service_name: service_name.to_string(),
            image_repo: repo_from_image(image).to_string(),
            environment: guess_environment(stack),
            short_tags: uses_short_sha(image),
        }
    }
}

/// Un commit de la rama del servicio (`service.RepoCommit`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct RepoCommit {
    pub sha: String,
    /// Sólo su primera línea.
    pub message: String,
    pub author: String,
    pub date: String,
}

/// Una imagen que el CI dijo haber publicado (`domain.ImageBuild`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ImageBuild {
    pub id: String,
    pub created_at: String,
    pub deployable_id: String,
    pub sha: String,
    pub image: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub actor: String,
    pub run_url: String,
    /// `ci`, `github` o `host` (lo construyó el servidor).
    pub source: String,
}

/// La llave del CI recién acuñada: la única vez que se ve entera. No se
/// guarda en el estado.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CiKey {
    pub key: String,
    pub preview: String,
}

/// Unas líneas más del log de un despliegue (`deploy:log`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeployLogEvent {
    pub deployment_id: String,
    pub deployable_id: String,
    pub lines: Vec<String>,
}

/// Los despliegues del servidor abierto.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DeploysState {
    pub server_id: String,
    /// El servicio del swarm que se mira (por su nombre), esté registrado o
    /// no.
    pub selected_service: Option<String>,
    /// Los servicios que se despliegan desde cac, cuando llegaron.
    pub deployables: Option<Vec<Deployable>>,
    pub error: Option<String>,
    /// El servicio que se mira, y lo suyo.
    pub selected: Option<String>,
    pub history: Vec<Deployment>,
    pub builds: Vec<ImageBuild>,
    pub commits: Option<Vec<RepoCommit>>,
    /// Por qué no llegaron (sin repo, sin GitHub App…).
    pub commits_error: Option<String>,
    /// Llegó un commit nuevo: la lista de commits ya no vale.
    pub commits_stale: bool,
    /// El despliegue cuyo log se mira, y su log.
    pub log_of: Option<String>,
    pub log: String,
}

impl DeploysState {
    /// Un despliegue que avanza, del stream o de la respuesta a pedirlo. Se
    /// funde con la fila que hubiera, no la sustituye: el evento no trae el
    /// nombre de quien lo pidió, y sustituirla le quitaba el autor al
    /// avanzar.
    pub fn on_status(&mut self, deployment: Deployment) {
        if deployment.id.is_empty() || self.selected.as_deref() != Some(deployment.deployable_id.as_str()) {
            return;
        }
        match self.history.iter_mut().find(|known| known.id == deployment.id) {
            Some(known) => {
                let name = known.requested_by_name.take();
                *known = Deployment { requested_by_name: deployment.requested_by_name.clone().or(name), ..deployment };
            }
            None => self.history.insert(0, deployment),
        }
    }

    /// Unas líneas más, si son del log que se mira.
    pub fn on_log(&mut self, event: DeployLogEvent) {
        if self.log_of.as_deref() != Some(event.deployment_id.as_str()) {
            return;
        }
        for line in event.lines {
            self.log.push_str(&line);
            self.log.push('\n');
        }
    }

    /// Un commit nuevo apuntado: a las versiones, y la lista de commits a
    /// refrescar.
    pub fn on_build(&mut self, build: ImageBuild) {
        if build.id.is_empty() || self.selected.as_deref() != Some(build.deployable_id.as_str()) {
            return;
        }
        if !self.builds.iter().any(|known| known.id == build.id) {
            self.builds.insert(0, build);
        }
        self.commits_stale = true;
    }
}

/// Un despliegue pedido de uno en uno: trae su log entero.
#[derive(Deserialize, Default)]
#[serde(default)]
struct DeploymentWithLog {
    log: String,
}

impl Platform {
    fn deploys_route(&self, rest: &str) -> Result<String, ServerEditError> {
        let server = self.open().ok_or(ServerEditError::ServerGone)?;
        Ok(format!("/api/v1/servers/{}/deployables{rest}", server.id))
    }

    fn edit_deploys(&self, server_id: &str, edit: impl FnOnce(&mut DeploysState)) {
        self.change(|data| {
            if let Some(deploys) = data.deploys.as_mut().filter(|deploys| deploys.server_id == server_id) {
                edit(deploys);
            }
        });
    }

    /// Pide los servicios que se despliegan desde cac en el servidor
    /// abierto. Un fallo deja los que hubiera.
    pub async fn load_deployables(&self) {
        let Some(server) = self.open() else { return };
        self.change(|data| {
            if data.deploys.as_ref().is_none_or(|deploys| deploys.server_id != server.id) {
                data.deploys = Some(DeploysState { server_id: server.id.clone(), ..Default::default() });
            }
        });
        let answer = self.core.get_list::<Deployable>(&format!("/api/v1/servers/{}/deployables", server.id)).await;
        self.edit_deploys(&server.id, |deploys| match answer {
            Ok(list) => {
                deploys.deployables = Some(list);
                deploys.error = None;
            }
            Err(error) => deploys.error = Some(error.to_string()),
        });
    }

    /// Registra un servicio que corre para desplegarlo desde cac.
    pub async fn create_deployable(&self, new: &NewDeployable) -> Result<Deployable, ServerEditError> {
        if !self.permissions().manage_deploys {
            return Err(ServerEditError::AdminOnly);
        }
        let route = self.deploys_route("")?;
        let created: Deployable = self.core.call("POST", &route, Some(serde_json::to_value(new).unwrap_or_default())).await?;
        let server_id = created.server_id.clone();
        let row = created.clone();
        self.edit_deploys(&server_id, |deploys| deploys.deployables.get_or_insert_with(Vec::new).push(row));
        Ok(created)
    }

    /// Cambia sus ajustes; la fila vuelve como la deja el backend.
    pub async fn update_deployable(&self, deployable_id: &str, settings: &DeployableSettings) -> Result<(), ServerEditError> {
        if !self.permissions().manage_deploys {
            return Err(ServerEditError::AdminOnly);
        }
        let route = self.deploys_route(&format!("/{deployable_id}"))?;
        let updated: Deployable = self.core.call("PATCH", &route, Some(serde_json::to_value(settings).unwrap_or_default())).await?;
        self.replace_deployable(updated);
        Ok(())
    }

    fn replace_deployable(&self, updated: Deployable) {
        let server_id = updated.server_id.clone();
        self.edit_deploys(&server_id, |deploys| {
            if let Some(slot) = deploys.deployables.iter_mut().flatten().find(|known| known.id == updated.id) {
                *slot = updated;
            }
        });
    }

    /// Acuña la llave del CI; la anterior deja de valer. Se devuelve a quien
    /// la pidió, que la enseña una vez: en el estado sólo queda su principio.
    pub async fn mint_ci_key(&self, deployable_id: &str) -> Result<CiKey, ServerEditError> {
        if !self.permissions().manage_deploys {
            return Err(ServerEditError::AdminOnly);
        }
        let route = self.deploys_route(&format!("/{deployable_id}/ci-key"))?;
        let key: CiKey = self.core.call("POST", &route, Some(serde_json::json!({}))).await?;
        let server_id = self.open().map(|server| server.id).unwrap_or_default();
        let preview = key.preview.clone();
        self.edit_deploys(&server_id, |deploys| {
            if let Some(slot) = deploys.deployables.iter_mut().flatten().find(|known| known.id == deployable_id) {
                slot.ci_key_preview = preview;
            }
        });
        Ok(key)
    }

    /// Mira un servicio: su historial, sus versiones y los commits de su
    /// rama, a la vez. Lo de otro servicio no se queda.
    pub async fn select_deployable(&self, deployable_id: &str) {
        let Some(server) = self.open() else { return };
        self.edit_deploys(&server.id, |deploys| {
            *deploys = DeploysState {
                server_id: deploys.server_id.clone(),
                selected_service: deploys.selected_service.take(),
                deployables: deploys.deployables.take(),
                error: deploys.error.take(),
                selected: Some(deployable_id.to_string()),
                ..Default::default()
            };
        });
        let route = |rest: &str| format!("/api/v1/servers/{}/deployables/{deployable_id}/{rest}", server.id);
        let (core, path) = (self.core.clone(), route("deployments"));
        let history = tokio::spawn(async move { core.get_list::<Deployment>(&path).await });
        let (core, path) = (self.core.clone(), route("builds"));
        let builds = tokio::spawn(async move { core.get_list::<ImageBuild>(&path).await });
        let platform = self.core.platform();
        let id = deployable_id.to_string();
        let commits = tokio::spawn(async move { platform.load_commits_of(&id).await });
        let (history, builds) = (super::flatten(history.await), super::flatten(builds.await));
        let _ = commits.await;
        self.edit_deploys(&server.id, |deploys| {
            if deploys.selected.as_deref() != Some(deployable_id) {
                return;
            }
            // Lo que llegó por el stream mientras tanto es más nuevo.
            if let Ok(history) = history {
                for row in history.into_iter().rev() {
                    if !deploys.history.iter().any(|known| known.id == row.id) {
                        deploys.history.insert(0, row);
                    }
                }
                deploys.history.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            }
            if let Ok(builds) = builds {
                for build in builds {
                    if !deploys.builds.iter().any(|known| known.id == build.id) {
                        deploys.builds.push(build);
                    }
                }
            }
        });
    }

    /// Mira un servicio del swarm: si ya se despliega desde cac, lo suyo; si
    /// no, nada más (se ofrece registrarlo).
    pub async fn choose_service(&self, service_name: &str) {
        let Some(server) = self.open() else { return };
        let mut registered = None;
        self.edit_deploys(&server.id, |deploys| {
            *deploys = DeploysState {
                server_id: deploys.server_id.clone(),
                selected_service: Some(service_name.to_string()),
                deployables: deploys.deployables.take(),
                error: deploys.error.take(),
                ..Default::default()
            };
            registered = deploys.deployables.iter().flatten().find(|known| known.service_name == service_name).map(|known| known.id.clone());
        });
        if let Some(id) = registered {
            self.select_deployable(&id).await;
        }
    }

    /// Vuelve a pedir los commits de la rama del servicio que se mira.
    pub async fn load_commits(&self) {
        let selected = self.data().deploys.as_ref().and_then(|deploys| deploys.selected.clone());
        if let Some(id) = selected {
            self.load_commits_of(&id).await;
        }
    }

    async fn load_commits_of(&self, deployable_id: &str) {
        let Ok(route) = self.deploys_route(&format!("/{deployable_id}/commits")) else { return };
        let server_id = self.open().map(|server| server.id).unwrap_or_default();
        let answer = self.core.get_list::<RepoCommit>(&route).await;
        self.edit_deploys(&server_id, |deploys| {
            if deploys.selected.as_deref() != Some(deployable_id) {
                return;
            }
            match answer {
                Ok(commits) => {
                    deploys.commits = Some(commits);
                    deploys.commits_error = None;
                    deploys.commits_stale = false;
                }
                Err(error) => deploys.commits_error = Some(error.to_string()),
            }
        });
    }

    /// Despliega ese commit del servicio. **Quien llama ya preguntó.**
    pub async fn deploy(&self, deployable_id: &str, sha: &str) -> Result<Deployment, ServerEditError> {
        if !self.permissions().deploy {
            return Err(ServerEditError::ReadOnly);
        }
        let route = self.deploys_route(&format!("/{deployable_id}/deploy"))?;
        let started: Deployment = self.core.call("POST", &route, Some(serde_json::json!({ "sha": sha }))).await?;
        self.deployment_seen(started.clone());
        Ok(started)
    }

    /// Vuelve a la imagen de antes de ese despliegue. **Quien llama ya
    /// preguntó.**
    pub async fn rollback(&self, deployable_id: &str, deployment_id: &str) -> Result<Deployment, ServerEditError> {
        if !self.permissions().deploy {
            return Err(ServerEditError::ReadOnly);
        }
        let route = self.deploys_route(&format!("/{deployable_id}/deployments/{deployment_id}/rollback"))?;
        let started: Deployment = self.core.call("POST", &route, Some(serde_json::json!({}))).await?;
        self.deployment_seen(started.clone());
        Ok(started)
    }

    fn deployment_seen(&self, deployment: Deployment) {
        let server_id = deployment.server_id.clone();
        self.edit_deploys(&server_id, |deploys| deploys.on_status(deployment));
    }

    /// Mira el log de un despliegue: lo que lleve, y lo que vaya llegando.
    pub async fn open_deploy_log(&self, deployment_id: &str) {
        let Some(server) = self.open() else { return };
        let mut deployable = None;
        self.edit_deploys(&server.id, |deploys| {
            deploys.log_of = Some(deployment_id.to_string());
            deploys.log.clear();
            deployable = deploys.selected.clone();
        });
        let Some(deployable) = deployable else { return };
        let route = format!("/api/v1/servers/{}/deployables/{deployable}/deployments/{deployment_id}", server.id);
        if let Ok(full) = self.core.get::<DeploymentWithLog>(&route).await {
            self.edit_deploys(&server.id, |deploys| {
                // Trae todo lo escrito hasta ahora, también lo que llegó
                // por el stream mientras se pedía.
                if deploys.log_of.as_deref() == Some(deployment_id) && full.log.len() >= deploys.log.len() {
                    deploys.log = full.log;
                }
            });
        }
    }

    pub fn close_deploy_log(&self) {
        let server_id = self.open().map(|server| server.id).unwrap_or_default();
        self.edit_deploys(&server_id, |deploys| {
            deploys.log_of = None;
            deploys.log.clear();
        });
    }

    /// Lo que cuenta el stream de los despliegues.
    pub(crate) fn deploy_event(&self, event: &ServerEvent) {
        let apply = |edit: &dyn Fn(&mut DeploysState)| {
            self.change(|data| {
                if let Some(deploys) = data.deploys.as_mut() {
                    edit(deploys);
                }
            });
        };
        match event.event.as_str() {
            "deploy:status" => {
                if let Ok(deployment) = serde_json::from_str::<Deployment>(&event.data) {
                    apply(&|deploys| {
                        if deploys.server_id == deployment.server_id {
                            deploys.on_status(deployment.clone());
                        }
                    });
                }
            }
            "deploy:log" => {
                if let Ok(lines) = serde_json::from_str::<DeployLogEvent>(&event.data) {
                    apply(&|deploys| deploys.on_log(lines.clone()));
                }
            }
            "deploy:build" => {
                if let Ok(build) = serde_json::from_str::<ImageBuild>(&event.data) {
                    apply(&|deploys| deploys.on_build(build.clone()));
                }
            }
            _ => {}
        }
    }
}

/// Un commit de la lista para elegir qué desplegar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRow {
    pub sha: String,
    pub message: String,
    pub author: String,
    /// Es lo que corre en el servidor.
    pub running: bool,
    /// Lo que corre y lo de antes: en segundo plano.
    pub older: bool,
    /// El tag con el que el CI publicó su imagen, si la publicó.
    pub published_tag: Option<String>,
    pub published: bool,
}

/// La lista de commits con lo que se deduce de ella.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitPick {
    pub rows: Vec<CommitRow>,
    /// Cuántos hay por encima del que corre, sin desplegar.
    pub behind: usize,
    /// El del botón: el elegido a mano mientras siga en la lista; si no, lo
    /// más nuevo que se puede desplegar.
    pub chosen: Option<String>,
}

impl CommitPick {
    /// Lo que se manda para desplegar ese commit. Sin build en el servidor,
    /// uno con imagen publicada va con el tag **con el que se publicó**
    /// (corto o entero), que es el que existe.
    pub fn value_of(&self, sha: &str, deployable: &Deployable) -> String {
        if deployable.build_on_host {
            return sha.to_string();
        }
        self.rows.iter().find(|row| row.sha == sha).and_then(|row| row.published_tag.clone()).unwrap_or_else(|| sha.to_string())
    }
}

/// Marca el commit que corre, los que tienen imagen publicada y cuál va en
/// el botón. Construyendo en el servidor, cualquiera se despliega; si la
/// imagen la publica el CI, sólo el que ya la tiene: el más nuevo puede estar
/// construyéndose todavía, y desplegarlo pediría un tag que no existe.
pub fn commit_pick(commits: &[RepoCommit], builds: &[ImageBuild], deployable: &Deployable, picked: Option<&str>) -> CommitPick {
    let published = |sha: &str| builds.iter().find(|build| !build.sha.is_empty() && sha.starts_with(&build.sha)).map(|build| build.sha.clone());
    let running = commits.iter().position(|commit| runs_commit(&deployable.current_image, &commit.sha));
    let rows: Vec<CommitRow> = commits
        .iter()
        .enumerate()
        .map(|(n, commit)| {
            let published_tag = published(&commit.sha);
            CommitRow {
                sha: commit.sha.clone(),
                message: commit.message.clone(),
                author: commit.author.clone(),
                running: running == Some(n),
                older: running.is_some_and(|at| n > at),
                published: !deployable.build_on_host && published_tag.is_some(),
                published_tag,
            }
        })
        .collect();
    let still_there = picked.filter(|sha| rows.iter().any(|row| row.sha == *sha)).map(str::to_string);
    let first = if deployable.build_on_host { rows.first() } else { rows.iter().find(|row| row.published_tag.is_some()) };
    CommitPick { behind: running.unwrap_or(0), chosen: still_there.or_else(|| first.map(|row| row.sha.clone())), rows }
}

/// Cuántas filas enseñan las listas de un despliegue antes de «Ver N más».
pub const LIST_PREVIEW: usize = 5;

/// La primera versión del agente que corre migraciones antes del despliegue
/// (`domain.AgentVersionMigrates`).
pub const AGENT_VERSION_MIGRATES: i64 = 4;

/// La primera que construye la imagen en el servidor
/// (`domain.AgentVersionBuilds`).
pub const AGENT_VERSION_BUILDS: i64 = 6;

/// El repositorio de una imagen, sin tag ni digest. El `:` de un puerto no
/// es un tag.
pub fn repo_from_image(image: &str) -> &str {
    let bare = image.split('@').next().unwrap_or_default();
    match (bare.rfind('/'), bare.rfind(':')) {
        (slash, Some(colon)) if slash.is_none_or(|slash| colon > slash) => &bare[..colon],
        _ => bare,
    }
}

/// El entorno por el sufijo del stack (`beta-api-prod` → `prod`).
pub fn guess_environment(stack: &str) -> String {
    let lower = stack.to_lowercase();
    ["production", "develop", "staging", "prod", "dev", "stage", "test", "qa"]
        .into_iter()
        .find(|env| lower.ends_with(&format!("-{env}")))
        .map(str::to_string)
        .unwrap_or_default()
}

/// El nombre del servicio sin el prefijo de su stack (`api-prod_app` → `app`).
pub fn short_service_name<'a>(service: &'a str, stack: &str) -> &'a str {
    service.strip_prefix(stack).and_then(|rest| rest.strip_prefix('_')).unwrap_or(service)
}

/// La referencia corta de una imagen, para una fila: el tag, sin el digest.
pub fn short_ref(image: &str) -> &str {
    if image.is_empty() {
        return "—";
    }
    let bare = image.split('@').next().unwrap_or_default();
    match (bare.rfind('/'), bare.rfind(':')) {
        (slash, Some(colon)) if slash.is_none_or(|slash| colon > slash) => &bare[colon + 1..],
        _ => "latest",
    }
}

/// Un sha de git, corto o entero. El backend lo vuelve a comprobar.
pub fn is_sha(text: &str) -> bool {
    (7..=40).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Si la imagen que corre es la de ese commit: su tag empieza por el sha.
/// Lo construido en el servidor lleva detrás el nombre del servicio
/// (`f01b62f-rrhh-prod_app`, `Deployable.CommitImage`), y un sha no tiene
/// guiones: lo de antes del primero es el commit.
pub fn runs_commit(image: &str, sha: &str) -> bool {
    let head = short_ref(image).split('-').next().unwrap_or_default();
    is_sha(head) && sha.starts_with(head)
}

/// Si el servicio se etiqueta con el sha corto: siete u ocho hex de tag.
pub fn uses_short_sha(image: &str) -> bool {
    let bare = image.split('@').next().unwrap_or_default();
    let tag = bare.rsplit('/').next().and_then(|last| last.split(':').nth(1)).unwrap_or_default();
    (7..=8).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// El paso que un repo añade a su workflow para avisar a cac tras publicar
/// la imagen. El sha tiene que ser el mismo con el que se etiqueta: cac
/// despliega `imageRepo:sha` y no otra cosa. Con tags cortos se manda el
/// corto: `$GITHUB_SHA` es siempre el entero.
pub fn ci_notice_step(url: &str, short_sha: bool) -> String {
    let sha = if short_sha { "$SHORT_SHA" } else { "$GITHUB_SHA" };
    let mut lines = vec!["      - name: Notify cac".to_string(), "        run: |".to_string()];
    if short_sha {
        lines.push("          SHORT_SHA=$(git rev-parse --short=7 HEAD)".into());
    }
    lines.push(format!("          curl -fsS -X POST {url} \\"));
    lines.push("            -H \"X-Deploy-Key: ${{ secrets.CAC_DEPLOY_KEY }}\" \\".into());
    lines.push("            -H \"Content-Type: application/json\" \\".into());
    lines.push(format!(
        "            -d \"{{\\\"sha\\\":\\\"{sha}\\\",\\\"ref\\\":\\\"$GITHUB_REF\\\",\\\"actor\\\":\\\"$GITHUB_ACTOR\\\",\\\"runUrl\\\":\\\"$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID\\\"}}\""
    ));
    lines.join("\n")
}

/// El puente de los Docker secrets al entorno, para el Dockerfile de un
/// servicio: cada `/run/secrets/NOMBRE` se exporta como `NOMBRE` si no viene
/// ya, y `exec "$@"` deja que el mismo contenedor corra la app o las
/// migraciones.
pub const SECRETS_ENTRYPOINT_SNIPPET: &str = r#"# Docker secrets → entorno (cac). Va en la imagen final, antes de USER.
RUN printf '%s\n' \
  '#!/bin/sh' \
  'for f in /run/secrets/*; do' \
  '  [ -f "$f" ] || continue' \
  '  n=${f##*/}' \
  '  case $n in *[!A-Za-z0-9_]*) continue;; esac' \
  '  printenv "$n" > /dev/null || export "$n=$(cat "$f")"' \
  'done' \
  'exec "$@"' \
  > /usr/local/bin/docker-entrypoint && chmod 755 /usr/local/bin/docker-entrypoint
ENTRYPOINT ["docker-entrypoint"]"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn deployment(id: &str, status: DeployStatus) -> Deployment {
        Deployment { id: id.into(), deployable_id: "d1".into(), status, ..Default::default() }
    }

    fn watching() -> DeploysState {
        DeploysState { selected: Some("d1".into()), ..Default::default() }
    }

    /// Rotura que atrapa: quitarle el autor a la fila al avanzar (el evento
    /// no lo trae), duplicarla en vez de fundirla, o meter en el historial
    /// el despliegue de otro servicio.
    #[test]
    fn a_status_event_merges_into_its_row() {
        let mut state = watching();
        state.history.push(Deployment { requested_by_name: Some("Ana".into()), ..deployment("x", DeployStatus::Queued) });
        state.on_status(deployment("x", DeployStatus::Running));
        assert_eq!(state.history.len(), 1);
        assert_eq!((state.history[0].status, state.history[0].requested_by_name.as_deref()), (DeployStatus::Running, Some("Ana")));
        state.on_status(deployment("y", DeployStatus::Queued));
        assert_eq!(state.history.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["y", "x"]);
        state.on_status(Deployment { deployable_id: "otro".into(), ..deployment("z", DeployStatus::Queued) });
        assert_eq!(state.history.len(), 2);
    }

    /// Rotura que atrapa: pegar al log que se mira las líneas de otro
    /// despliegue, o juntarlas sin salto.
    #[test]
    fn log_lines_land_only_on_their_deployment() {
        let mut state = DeploysState { log_of: Some("x".into()), ..watching() };
        state.on_log(DeployLogEvent { deployment_id: "x".into(), lines: vec!["pull".into(), "update".into()], ..Default::default() });
        state.on_log(DeployLogEvent { deployment_id: "y".into(), lines: vec!["ajeno".into()], ..Default::default() });
        assert_eq!(state.log, "pull\nupdate\n");
    }

    /// Rotura que atrapa: duplicar una versión que ya estaba, o no avisar de
    /// que la lista de commits quedó vieja.
    #[test]
    fn a_new_build_is_listed_once_and_stales_the_commits() {
        let mut state = watching();
        let build = ImageBuild { id: "b1".into(), deployable_id: "d1".into(), ..Default::default() };
        state.on_build(build.clone());
        state.on_build(build);
        assert_eq!(state.builds.len(), 1);
        assert!(state.commits_stale);
    }

    /// Rotura que atrapa: tomar el puerto del registro por un tag, perder el
    /// tag con un digest, o dar por de un commit una imagen `latest`.
    #[test]
    fn images_are_read_as_docker_reads_them() {
        assert_eq!(repo_from_image("registry:5000/o/api:v3@sha256:ab"), "registry:5000/o/api");
        assert_eq!(repo_from_image("registry:5000/o/api"), "registry:5000/o/api");
        assert_eq!(short_ref("ghcr.io/o/api:abc1234@sha256:ff"), "abc1234");
        assert_eq!(short_ref("registry:5000/o/api"), "latest");
        assert_eq!(short_ref(""), "—");
        assert!(runs_commit("ghcr.io/o/api:f01b62f-rrhh-prod_app", "f01b62fdeadbeef"));
        assert!(!runs_commit("ghcr.io/o/api:latest", "f01b62f"));
        assert!(!runs_commit("ghcr.io/o/api:abc1234", "f01b62f"));
        assert!(uses_short_sha("ghcr.io/o/api:abc1234"));
        assert!(!uses_short_sha("ghcr.io/o/api:v1.2.3"));
        assert!(!uses_short_sha("ghcr.io/o/api:0123456789abcdef0123456789abcdef01234567"), "un sha entero no es corto");
        assert!(!is_sha("abc12") && is_sha("abc1234") && !is_sha("ABC1234"));
    }

    /// Rotura que atrapa: prellenar el registro con el nombre largo, el tag
    /// pegado al repositorio o un entorno que no es el del stack.
    #[test]
    fn a_running_service_prefills_its_registration() {
        let new = NewDeployable::from_service("api-prod_app", "api-prod", "ghcr.io/o/api:abc1234");
        assert_eq!((new.name.as_str(), new.image_repo.as_str(), new.environment.as_str(), new.short_tags), ("app", "ghcr.io/o/api", "prod", true));
        assert_eq!(guess_environment("beta-api-PRODUCTION"), "production");
        assert_eq!(guess_environment("tools"), "");
        assert_eq!(short_service_name("otro_app", "api-prod"), "otro_app");
    }

    /// Rotura que atrapa: dejar elegido por defecto un commit cuya imagen el
    /// CI aún no publicó (pediría un tag que no existe), desplegar con el sha
    /// entero lo que se publicó con el corto, no marcar lo que corre, o
    /// conservar un elegido que ya no está en la lista.
    #[test]
    fn the_commit_list_offers_what_can_be_deployed() {
        let commit = |sha: &str| RepoCommit { sha: sha.into(), message: format!("c {sha}"), ..Default::default() };
        let commits = [commit("ccc3333aaaa"), commit("bbb2222aaaa"), commit("aaa1111aaaa")];
        let builds = [ImageBuild { sha: "bbb2222".into(), ..Default::default() }];
        let ci = Deployable { current_image: "ghcr.io/o/web:aaa1111".into(), ..Default::default() };
        let pick = commit_pick(&commits, &builds, &ci, None);
        assert_eq!(pick.chosen.as_deref(), Some("bbb2222aaaa"));
        assert_eq!(pick.value_of("bbb2222aaaa", &ci), "bbb2222");
        assert_eq!(pick.behind, 2);
        assert_eq!(pick.rows.iter().map(|row| (row.running, row.published)).collect::<Vec<_>>(), [(false, false), (false, true), (true, false)]);
        assert_eq!(commit_pick(&commits, &builds, &ci, Some("ccc3333aaaa")).chosen.as_deref(), Some("ccc3333aaaa"));
        assert_eq!(commit_pick(&commits, &builds, &ci, Some("borrado")).chosen.as_deref(), Some("bbb2222aaaa"));

        let host = Deployable { build_on_host: true, ..ci };
        let pick = commit_pick(&commits, &builds, &host, None);
        assert_eq!(pick.chosen.as_deref(), Some("ccc3333aaaa"));
        assert_eq!(pick.value_of("bbb2222aaaa", &host), "bbb2222aaaa");
        assert!(pick.rows.iter().all(|row| !row.published));
    }

    /// Rotura que atrapa: mandar el sha entero a un CI que etiqueta con el
    /// corto (cac desplegaría un tag que no existe).
    #[test]
    fn the_ci_step_sends_the_sha_the_image_carries() {
        let short = ci_notice_step("https://cac/x", true);
        assert!(short.contains("SHORT_SHA=$(git rev-parse --short=7 HEAD)") && short.contains("\\\"sha\\\":\\\"$SHORT_SHA\\\""));
        let long = ci_notice_step("https://cac/x", false);
        assert!(!long.contains("SHORT_SHA") && long.contains("\\\"sha\\\":\\\"$GITHUB_SHA\\\""));
        assert!(long.contains("curl -fsS -X POST https://cac/x"));
    }
}
