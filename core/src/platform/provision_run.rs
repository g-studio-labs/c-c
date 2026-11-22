//! Correr Ansible desde esta máquina, siguiendo el protocolo (decisión
//! 4d28f4ab): ensayar (`--check --diff`) y pedir, correr una petición
//! autorizada con su verificación, y aplicar a mano un repo sin manifiesto
//! v2 (`ServerProvision.tsx`).
//!
//! Ansible no vive aquí: lo pone quien usa el core por `AnsibleEngine` (el
//! escritorio, con `cac_mcp::ansible`), porque lanza programas de la
//! máquina. Aquí van el orden de las llamadas y lo que se apunta en cac:
//! nunca un valor, sólo nombres, el resumen y la cola de la salida, que ya
//! llega tachada y que aquí se vuelve a tachar con lo que se escribió a mano.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::provision::{json_object, AnsibleManifest, AnsibleTools, EffectiveConfig, InventoryHost, PlanGaps, ProjectInfo, ProvisionError, ProvisioningRequest, Risk};
use super::{LogLine, Platform, Server};
use crate::ansi::ansi_spans;

/// Lo que va contando una ejecución.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    Line(String),
    Exit { code: Option<i32>, cancelled: bool },
}

pub type EngineSink = Arc<dyn Fn(EngineEvent) + Send + Sync>;

/// Una variable de una ejecución a mano. Sin `Debug`: puede llevar un valor
/// secreto escrito en la pantalla.
#[derive(Clone)]
pub struct ManualVar {
    pub name: String,
    pub value: Option<String>,
    /// Una referencia `op://` que el motor lee al correr.
    pub from: Option<String>,
    pub secret: bool,
}

/// Una ejecución de un repo con su propio inventario (manifiesto v1). Sin
/// `Debug` ni `Serialize`: lleva la contraseña de sudo, que sólo vive aquí
/// hasta que el motor la escribe en su fichero `0600`.
pub struct ManualRun {
    pub project_dir: String,
    pub venv: Option<String>,
    /// El fichero del playbook, relativo al proyecto.
    pub playbook: String,
    pub inventory: String,
    pub limit: Option<String>,
    pub vars: Vec<ManualVar>,
    pub become_password: Option<String>,
    pub become_password_ref: Option<String>,
}

/// Lo de esta máquina que hace falta para correr Ansible. Todo bloquea
/// (leer 1Password pide la huella): el core lo llama en un hilo aparte.
pub trait AnsibleEngine: Send + Sync {
    fn manifest(&self, project_dir: &str) -> Result<AnsibleManifest, String>;
    fn tools(&self, project_dir: &str, venv: Option<&str>) -> AnsibleTools;
    fn inventory(&self, project_dir: &str, inventory: &str, venv: Option<&str>) -> Result<Vec<InventoryHost>, String>;
    /// `effective` es la config efectiva como la escribió el backend.
    fn plan_gaps(&self, project_dir: &str, playbook_id: &str, effective: &str) -> Result<PlanGaps, String>;
    /// Lanza y contesta en cuanto lanzó, con el id para cancelar. Un `Err`
    /// es que Ansible no llegó a arrancar: el servidor no se tocó.
    fn run_with_config(&self, project_dir: &str, playbook_id: &str, effective: &str, check: bool, sink: EngineSink) -> Result<String, String>;
    fn run_manual(&self, spec: ManualRun, sink: EngineSink) -> Result<String, String>;
    fn cancel(&self, run_id: &str) -> Result<(), String>;
    /// La salida de un ensayo como la evalúa cac (`domain.ProvisioningCheck`).
    fn check_report(&self, lines: &[String], exit_code: i32) -> Value;
}

/// Qué se está corriendo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum RunKind {
    /// El ensayo del protocolo: no cambia nada.
    #[default]
    Rehearse,
    /// Con una autorización, o su verificación.
    Grant,
    /// Un repo sin manifiesto v2.
    Manual,
}

/// Cómo acabó, en las palabras de cac.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum RunEnd {
    Succeeded,
    Failed,
    Cancelled,
}

impl RunEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            RunEnd::Succeeded => "succeeded",
            RunEnd::Failed => "failed",
            RunEnd::Cancelled => "cancelled",
        }
    }
}

pub fn final_status(code: Option<i32>, cancelled: bool) -> RunEnd {
    if cancelled {
        RunEnd::Cancelled
    } else if code == Some(0) {
        RunEnd::Succeeded
    } else {
        RunEnd::Failed
    }
}

/// La salida de lo que corre (o de lo último), para pintarla.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ProvisionOutput {
    pub server_id: String,
    pub playbook: String,
    pub kind: RunKind,
    /// Hay algo en marcha desde esta app: no se lanza otra cosa.
    pub running: bool,
    /// El motor ya lanzó: se puede parar.
    pub cancellable: bool,
    pub lines: Vec<LogLine>,
    pub ended: Option<RunEnd>,
    pub exit_code: Option<i32>,
    /// Por qué no llegó a arrancar.
    pub error: Option<String>,
}

/// Lo de la ejecución que no se pinta. Sin `Debug`: `secrets` es lo que se
/// tacha.
#[derive(Default)]
pub(crate) struct OutputData {
    pub output: ProvisionOutput,
    raw: Vec<String>,
    engine_run: Option<String>,
    secrets: Vec<String>,
}

/// Las líneas que se guardan de una ejecución en curso.
pub const MAX_LINES: usize = 2000;

/// Las que se mandan a cac al acabar (el backend también lo acota).
pub const TAIL_LINES: usize = 200;

/// Cuánto se juntan las líneas antes de pintarlas.
const FLUSH: std::time::Duration = std::time::Duration::from_millis(50);

/// Lo que se apunta para cerrar, tras reabrir, una ejecución que se quedó a
/// medias (la app se cerró con ella corriendo).
const IN_FLIGHT_KEY: &str = "provision-in-flight";

pub fn strip_ansi(line: &str) -> String {
    ansi_spans(line).into_iter().map(|span| span.text).collect()
}

/// El `PLAY RECAP` del final, sin color: cómo acabó cada host.
pub fn play_recap(lines: &[String]) -> String {
    let Some(start) = lines.iter().rposition(|line| strip_ansi(line).starts_with("PLAY RECAP")) else { return String::new() };
    lines[start..].iter().map(|line| strip_ansi(line)).filter(|line| !line.trim().is_empty()).collect::<Vec<_>>().join("\n")
}

pub fn log_tail(lines: &[String], n: usize) -> String {
    lines[lines.len().saturating_sub(n)..].iter().map(|line| strip_ansi(line)).collect::<Vec<_>>().join("\n")
}

fn redact(line: &str, secrets: &[String]) -> String {
    secrets.iter().filter(|secret| !secret.is_empty()).fold(line.to_string(), |line, secret| line.replace(secret.as_str(), "***"))
}

/// Las variables extra escritas a mano, una `nombre=valor` por línea; las que
/// no lo son, aparte.
pub fn parse_extra_vars(text: &str) -> (Vec<(String, String)>, Vec<String>) {
    let (mut vars, mut bad) = (Vec::new(), Vec::new());
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once('=').map(|(name, value)| (name.trim(), value)).unwrap_or(("", ""));
        let mut chars = name.chars();
        let valid = matches!(chars.next(), Some('A'..='Z' | 'a'..='z' | '_')) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if valid {
            vars.push((name.to_string(), value.to_string()));
        } else {
            bad.push(line.to_string());
        }
    }
    (vars, bad)
}

fn project_name(dir: &str) -> String {
    dir.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string()
}

/// Lo escrito para una ejecución a mano. Sin `Debug`.
#[derive(Default, Clone)]
pub struct ManualInputs {
    /// Las variables del playbook escritas a mano, por nombre.
    pub values: Vec<(String, String)>,
    pub extra: String,
    pub become_password: String,
}

/// Mientras vive, hay algo en marcha. Al soltarlo, pase lo que pase (un
/// error, un `?`), se deja de estarlo: una ejecución que falla en el acto no
/// puede dejar la pestaña «aplicando» para siempre.
struct Busy(Platform);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.change(|data| {
            if let Some(out) = data.provision_output.as_mut() {
                out.output.running = false;
                out.output.cancellable = false;
                out.engine_run = None;
                out.secrets.clear();
            }
        });
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InFlight {
    server_id: String,
    backend_run_id: String,
}

impl Platform {
    fn provision_state(&self) -> Option<super::ProvisionState> {
        self.state().provision
    }

    /// La carpeta del proyecto de Ansible de este servidor, en esta máquina.
    pub fn set_project_dir(&self, dir: &str) {
        let Some(server) = self.open() else { return };
        self.edit_provision(&server.id, |provision| {
            if provision.project_dir != dir {
                provision.project_dir = dir.trim().to_string();
                provision.playbook_id.clear();
                provision.project = None;
            }
        });
        self.remember_prefs();
    }

    pub fn choose_playbook(&self, playbook_id: &str) {
        let Some(server) = self.open() else { return };
        self.edit_provision(&server.id, |provision| provision.playbook_id = playbook_id.to_string());
        self.remember_prefs();
    }

    /// Contra qué (v1): un host o grupo del inventario, o vacío.
    pub fn set_limit(&self, limit: &str) {
        let Some(server) = self.open() else { return };
        self.edit_provision(&server.id, |provision| provision.limit = limit.to_string());
        self.remember_prefs();
    }

    fn remember_prefs(&self) {
        if let Some(provision) = self.data().provision.clone() {
            self.keep_prefs(&provision);
        }
    }

    /// Lee la carpeta: el manifiesto, si Ansible está, el inventario (v1) y
    /// lo que le falta a la config para el playbook elegido (v2).
    pub async fn load_project(&self, engine: Arc<dyn AnsibleEngine>) {
        let Some(provision) = self.provision_state() else { return };
        let dir = provision.project_dir.clone();
        if dir.is_empty() {
            self.edit_provision(&provision.server_id, |provision| provision.project = None);
            return;
        }
        let reading = engine.clone();
        let read_dir = dir.clone();
        let info = tokio::task::spawn_blocking(move || {
            let mut info = ProjectInfo { dir: read_dir.clone(), ..Default::default() };
            match reading.manifest(&read_dir) {
                Ok(manifest) => {
                    info.tools = Some(reading.tools(&read_dir, manifest.venv.as_deref()));
                    if !manifest.inventory.is_empty() {
                        info.hosts = reading.inventory(&read_dir, &manifest.inventory, manifest.venv.as_deref()).unwrap_or_default();
                    }
                    info.manifest = Some(manifest);
                }
                Err(error) => info.error = Some(error),
            }
            info
        })
        .await
        .unwrap_or_default();
        self.edit_provision(&provision.server_id, |provision| {
            if provision.project_dir == dir {
                provision.project = Some(info);
            }
        });
        self.refresh_gaps(engine).await;
    }

    /// Qué le falta a la config para el playbook elegido, con la misma
    /// receta con la que correrá.
    pub async fn refresh_gaps(&self, engine: Arc<dyn AnsibleEngine>) {
        let Some(provision) = self.provision_state() else { return };
        let (Some(playbook), Some(effective)) = (provision.playbook().cloned(), provision.effective.clone()) else { return };
        if !provision.project.as_ref().and_then(|p| p.manifest.as_ref()).is_some_and(AnsibleManifest::is_v2) {
            return;
        }
        let (dir, id) = (provision.project_dir.clone(), playbook.id.clone());
        let gaps = tokio::task::spawn_blocking(move || engine.plan_gaps(&dir, &id, &effective.to_json())).await.ok().and_then(Result::ok);
        self.edit_provision(&provision.server_id, |state| {
            if let Some(project) = state.project.as_mut().filter(|project| project.dir == provision.project_dir) {
                project.gaps_of = playbook.id.clone();
                project.gaps = gaps;
            }
        });
    }

    fn begin_run(&self, server: &Server, playbook: &str, kind: RunKind) -> Result<Busy, ProvisionError> {
        let mut taken = false;
        self.change(|data| {
            if data.provision_output.as_ref().is_some_and(|out| out.output.running) {
                return;
            }
            taken = true;
            data.provision_output = Some(OutputData {
                output: ProvisionOutput { server_id: server.id.clone(), playbook: playbook.to_string(), kind, running: true, ..Default::default() },
                ..Default::default()
            });
        });
        if taken {
            Ok(Busy(self.core.platform()))
        } else {
            Err(ProvisionError::Busy)
        }
    }

    fn edit_output(&self, edit: impl FnOnce(&mut OutputData)) {
        self.change(|data| {
            if let Some(out) = data.provision_output.as_mut() {
                edit(out);
            }
        });
    }

    /// Una ejecución, de principio a fin: se abre en cac (si `start`), se
    /// lanza, se va pintando la salida y se cierra en cac. Contesta cuando cac
    /// ya la cerró: cerrarla es lo que pasa una petición a «verificando», y la
    /// verificación que arranca justo después la necesita ahí (con la
    /// respuesta antes, las dos se cruzaban: 409 y «verificando» para
    /// siempre, gstd, 2-nov-2026).
    ///
    /// Si el motor no llega a lanzar (1Password no contestó, el venv…), el
    /// cierre dice `notStarted`: el servidor no se tocó, y cac devuelve la
    /// petición a «aprobada» con su misma autorización en vez de gastarla
    /// (jose, 3-nov-2026).
    async fn execute<F>(&self, engine: &Arc<dyn AnsibleEngine>, server: &Server, start: Option<Value>, playbook: &str, secrets: Vec<String>, launch: F) -> Result<(RunEnd, Option<i32>, Vec<String>), ProvisionError>
    where
        F: FnOnce(&dyn AnsibleEngine, EngineSink) -> Result<String, String> + Send + 'static,
    {
        let runs = format!("/api/v1/servers/{}/provisioning-runs", server.id);
        let backend_run = match start {
            Some(body) => {
                #[derive(Deserialize)]
                struct Opened {
                    id: String,
                }
                let opened: Opened = self.core.call("POST", &runs, Some(body)).await?;
                Some(opened.id)
            }
            None => None,
        };
        let playbook = playbook.to_string();
        self.edit_output(|out| {
            out.output = ProvisionOutput { server_id: server.id.clone(), playbook: playbook.clone(), kind: out.output.kind, running: true, ..Default::default() };
            out.raw.clear();
            out.secrets = secrets.into_iter().filter(|secret| !secret.is_empty()).collect();
        });
        if let Some(id) = &backend_run {
            let keep = InFlight { server_id: server.id.clone(), backend_run_id: id.clone() };
            if let Ok(raw) = serde_json::to_string(&keep) {
                let _ = self.core.inner.storage.set(IN_FLIGHT_KEY, &raw);
            }
        }
        let close = |body: Value| {
            let (core, route) = (self.core.clone(), backend_run.as_ref().map(|id| format!("{runs}/{id}")));
            async move {
                if let Some(route) = route {
                    // Si no se pudo cerrar, la deja como interrumpida la
                    // próxima vez que se abra la pestaña: no es para enseñar.
                    let _ = core.call_ok("PATCH", &route, Some(body)).await;
                    core.inner.storage.remove(IN_FLIGHT_KEY);
                }
            }
        };

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<EngineEvent>();
        let sink: EngineSink = Arc::new(move |event| {
            let _ = tx.send(event);
        });
        let launching = engine.clone();
        let launched = tokio::task::spawn_blocking(move || launch(&*launching, sink)).await.map_err(|error| error.to_string()).and_then(|launched| launched);
        let run_id = match launched {
            Ok(run_id) => run_id,
            Err(error) => {
                close(json!({ "status": "failed", "summary": error, "logTail": "", "notStarted": true })).await;
                self.edit_output(|out| out.output.error = Some(error.clone()));
                if backend_run.is_some() {
                    self.reload_runs().await;
                }
                return Err(ProvisionError::Engine(error));
            }
        };
        self.edit_output(|out| {
            out.engine_run = Some(run_id);
            out.output.cancellable = true;
        });

        let mut pending: Vec<String> = Vec::new();
        let mut flush_at = tokio::time::Instant::now();
        let (code, cancelled) = loop {
            let next = if pending.is_empty() {
                rx.recv().await
            } else {
                match tokio::time::timeout_at(flush_at, rx.recv()).await {
                    Ok(next) => next,
                    Err(_) => {
                        self.push_output(std::mem::take(&mut pending));
                        continue;
                    }
                }
            };
            match next {
                Some(EngineEvent::Line(line)) => {
                    if pending.is_empty() {
                        flush_at = tokio::time::Instant::now() + FLUSH;
                    }
                    pending.push(line);
                }
                Some(EngineEvent::Exit { code, cancelled }) => break (code, cancelled),
                // El motor soltó el canal sin decir cómo acabó.
                None => break (None, false),
            }
        };
        self.push_output(pending);
        let end = final_status(code, cancelled);
        let mut raw = Vec::new();
        self.edit_output(|out| {
            out.output.ended = Some(end);
            out.output.exit_code = code;
            out.output.cancellable = false;
            out.engine_run = None;
            raw = out.raw.clone();
        });
        close(json!({ "status": end.as_str(), "exitCode": code, "summary": play_recap(&raw), "logTail": log_tail(&raw, TAIL_LINES) })).await;
        if backend_run.is_some() {
            self.reload_runs().await;
        }
        Ok((end, code, raw))
    }

    fn push_output(&self, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        self.edit_output(|out| {
            for line in lines {
                let line = redact(&line, &out.secrets);
                out.output.lines.push(LogLine { spans: ansi_spans(&line) });
                out.raw.push(line);
            }
            if out.raw.len() > MAX_LINES {
                let extra = out.raw.len() - MAX_LINES;
                out.raw.drain(..extra);
            }
            if out.output.lines.len() > MAX_LINES {
                let extra = out.output.lines.len() - MAX_LINES;
                out.output.lines.drain(..extra);
            }
        });
    }

    /// Para lo que corre. Si el motor todavía no lanzó (leyendo 1Password),
    /// no hay nada que parar.
    pub async fn cancel_run(&self, engine: Arc<dyn AnsibleEngine>) {
        let run = self.data().provision_output.as_ref().and_then(|out| out.engine_run.clone());
        if let Some(run) = run {
            let _ = tokio::task::spawn_blocking(move || engine.cancel(&run)).await;
        }
    }

    /// Cierra como interrumpida una ejecución que se quedó «en curso» de una
    /// vez anterior: el proceso murió con la app.
    pub(crate) async fn close_orphan(&self) {
        let Some(orphan) = self.core.inner.storage.get(IN_FLIGHT_KEY).and_then(|raw| serde_json::from_str::<InFlight>(&raw).ok()) else { return };
        if self.data().provision_output.as_ref().is_some_and(|out| out.output.running) {
            return;
        }
        let route = format!("/api/v1/servers/{}/provisioning-runs/{}", orphan.server_id, orphan.backend_run_id);
        let _ = self.core.call_ok("PATCH", &route, Some(json!({ "status": "interrupted", "summary": "", "logTail": "" }))).await;
        self.core.inner.storage.remove(IN_FLIGHT_KEY);
    }

    fn v2_project(&self) -> Result<(Server, super::ProvisionState, AnsibleManifest), ProvisionError> {
        if !self.provision_permissions().run {
            return Err(ProvisionError::ReadOnly);
        }
        let server = self.open().ok_or(ProvisionError::ServerGone)?;
        let provision = self.provision_state().filter(|p| p.server_id == server.id).ok_or(ProvisionError::ServerGone)?;
        let manifest = provision.project.as_ref().and_then(|p| p.manifest.clone()).ok_or(ProvisionError::NoProject)?;
        Ok((server, provision, manifest))
    }

    /// Ensaya y pide (manifiesto v2): por cada playbook, en orden, abre una
    /// petición, corre su ensayo y se lo manda a cac, que lo evalúa. Lo
    /// aprueba un admin en la bandeja. Para en el primer ensayo que falla: el
    /// de una etapa puede fallar porque la anterior aún no está aplicada.
    /// **Quien llama ya preguntó.**
    ///
    /// Esta función no tiene camino para correr de verdad: el motor siempre
    /// recibe `check`.
    pub async fn rehearse(&self, engine: Arc<dyn AnsibleEngine>, playbook_ids: &[String]) -> Result<(), ProvisionError> {
        let (server, provision, manifest) = self.v2_project()?;
        let effective = provision.effective.clone().filter(|e| e.config_seq >= 1).ok_or(ProvisionError::NoConfig)?;
        let _busy = self.begin_run(&server, playbook_ids.first().map(String::as_str).unwrap_or_default(), RunKind::Rehearse)?;
        let (dir, effective_json) = (provision.project_dir.clone(), effective.to_json());
        for id in playbook_ids {
            let playbook = manifest.playbook(id).ok_or(ProvisionError::NoProject)?;
            let risk = playbook.risk.as_deref().and_then(Risk::parse).ok_or_else(|| ProvisionError::NoRisk(id.clone()))?;
            let body = json!({
                "playbook": id, "project": project_name(&dir), "risk": risk.as_str(),
                "verify": playbook.verify.clone().unwrap_or_default(), "rollback": playbook.rollback.clone().unwrap_or_default(),
                "configSeq": effective.config_seq, "reason": "",
            });
            let created: ProvisioningRequest = self.core.call("POST", &format!("/api/v1/servers/{}/provisioning-requests", server.id), Some(body)).await?;
            let check = format!("/api/v1/servers/{}/provisioning-requests/{}/check", server.id, created.id);
            self.reload_requests().await;
            let (run_dir, run_id, run_effective) = (dir.clone(), id.clone(), effective_json.clone());
            let ran = self.execute(&engine, &server, None, id, Vec::new(), move |engine, sink| engine.run_with_config(&run_dir, &run_id, &run_effective, true, sink)).await;
            let (end, code, lines) = match ran {
                Ok(ran) => ran,
                Err(ProvisionError::Engine(error)) => {
                    // El ensayo no arrancó: sin su informe, la petición se
                    // quedaba «ensayando» hasta caducar. Se cierra como ensayo
                    // fallido, con el porqué.
                    let report = engine.check_report(std::slice::from_ref(&error), -1);
                    let _ = self.core.call_ok("PUT", &check, Some(report)).await;
                    self.reload_requests().await;
                    return Err(ProvisionError::Engine(error));
                }
                Err(other) => return Err(other),
            };
            if end == RunEnd::Cancelled {
                return Err(ProvisionError::Cancelled);
            }
            let report = engine.check_report(&lines, code.unwrap_or(-1));
            self.core.call_ok("PUT", &check, Some(report)).await?;
            self.reload_requests().await;
            if code != Some(0) {
                break;
            }
        }
        Ok(())
    }

    /// Corre una petición autorizada: gasta la autorización (el backend
    /// comprueba playbook, versión y caducidad) y, si acaba bien, su
    /// verificación. Una que se quedó «verificando» sin verificación sólo
    /// corre ésta. El riesgo aprobado tiene que seguir siendo el del
    /// manifiesto; si cambió, se pide otra vez. **Quien llama ya preguntó.**
    pub async fn run_grant(&self, engine: Arc<dyn AnsibleEngine>, request_id: &str) -> Result<(), ProvisionError> {
        let (server, provision, manifest) = self.v2_project()?;
        let request = provision.request(request_id).cloned().ok_or(ProvisionError::RequestGone)?;
        let same_risk = manifest.playbook(&request.playbook).and_then(|p| p.risk.as_deref()).and_then(Risk::parse) == Some(request.risk);
        if !same_risk {
            return Err(ProvisionError::RiskChanged(request.playbook.clone()));
        }
        let _busy = self.begin_run(&server, &request.playbook, RunKind::Grant)?;
        let raw: Value = self.core.get(&format!("/api/v1/servers/{}/config/effective?seq={}", server.id, request.config_seq)).await?;
        let effective: EffectiveConfig = serde_json::from_value(raw.clone()).map_err(|error| ProvisionError::Engine(error.to_string()))?;
        let step = Grant { server: &server, manifest: &manifest, dir: &provision.project_dir, effective: &effective, effective_json: raw.to_string(), request: &request };
        let outcome = async {
            if request.status == "verifying" {
                if !request.verify.is_empty() {
                    self.grant_step(&engine, &step, &request.verify).await?;
                }
                return Ok(());
            }
            let end = self.grant_step(&engine, &step, &request.playbook).await?;
            if end == RunEnd::Succeeded && request.risk != Risk::Read && !request.verify.is_empty() {
                self.grant_step(&engine, &step, &request.verify).await?;
            }
            Ok(())
        }
        .await;
        self.reload_requests().await;
        outcome
    }

    async fn grant_step(&self, engine: &Arc<dyn AnsibleEngine>, grant: &Grant<'_>, playbook_id: &str) -> Result<RunEnd, ProvisionError> {
        let mut var_names: Vec<String> = grant.manifest.playbook(playbook_id).map(|p| p.vars.iter().map(|v| v.name.clone()).collect()).unwrap_or_default();
        var_names.extend(json_object(&grant.effective.vars).keys().cloned());
        var_names.sort();
        var_names.dedup();
        let body = json!({
            "kind": "playbook", "project": project_name(grant.dir),
            // Con autorización, el backend la casa por el id del playbook.
            "playbook": playbook_id, "target": grant.effective.connection.name, "varNames": var_names,
            "configSeq": grant.request.config_seq, "requestId": grant.request.id,
        });
        let (dir, id, effective) = (grant.dir.to_string(), playbook_id.to_string(), grant.effective_json.clone());
        let (end, _, _) = self.execute(engine, grant.server, Some(body), playbook_id, Vec::new(), move |engine, sink| engine.run_with_config(&dir, &id, &effective, false, sink)).await?;
        Ok(end)
    }

    /// Aplica un playbook de un repo con su propio inventario (manifiesto
    /// v1): una persona desde la app es su propia aprobación. La contraseña
    /// de sudo va al motor y a ningún sitio más; lo que se escribió como
    /// secreto se tacha de la salida. **Quien llama ya preguntó.**
    pub async fn run_manual(&self, engine: Arc<dyn AnsibleEngine>, inputs: ManualInputs) -> Result<(), ProvisionError> {
        let (server, provision, manifest) = self.v2_project()?;
        let playbook = provision.playbook().cloned().ok_or(ProvisionError::NoProject)?;
        let (extra, bad) = parse_extra_vars(&inputs.extra);
        if !bad.is_empty() {
            return Err(ProvisionError::BadVars(bad.join(", ")));
        }
        let value_of = |name: &str| inputs.values.iter().find(|(known, _)| known == name).map(|(_, value)| value.clone()).filter(|value| !value.trim().is_empty());
        let missing: Vec<String> = playbook.vars.iter().filter(|v| v.required && v.from.is_none() && value_of(&v.name).is_none()).map(|v| v.name.clone()).collect();
        if !missing.is_empty() {
            return Err(ProvisionError::MissingVars(missing.join(", ")));
        }
        let needs_password = playbook.become_sudo && playbook.become_password_ref.is_none();
        if needs_password && inputs.become_password.is_empty() {
            return Err(ProvisionError::MissingVars("become".into()));
        }
        let mut secrets: Vec<String> = playbook.vars.iter().filter(|v| v.secret).filter_map(|v| value_of(&v.name)).collect();
        let mut vars: Vec<ManualVar> = playbook
            .vars
            .iter()
            .map(|v| ManualVar { name: v.name.clone(), from: v.from.clone(), value: if v.from.is_some() { None } else { value_of(&v.name) }, secret: v.secret })
            .collect();
        vars.extend(extra.iter().map(|(name, value)| ManualVar { name: name.clone(), value: Some(value.clone()), from: None, secret: false }));
        let var_names: Vec<String> = vars.iter().map(|v| v.name.clone()).collect();
        let become_password = needs_password.then(|| inputs.become_password.clone());
        secrets.extend(become_password.clone());
        let spec = ManualRun {
            project_dir: provision.project_dir.clone(),
            venv: manifest.venv.clone(),
            playbook: playbook.file.clone(),
            inventory: manifest.inventory.clone(),
            limit: (!provision.limit.is_empty()).then(|| provision.limit.clone()),
            vars,
            become_password,
            become_password_ref: playbook.become_password_ref.clone(),
        };
        let body = json!({ "kind": "playbook", "project": project_name(&provision.project_dir), "playbook": playbook.file, "target": provision.limit, "varNames": var_names });
        let _busy = self.begin_run(&server, &playbook.name, RunKind::Manual)?;
        self.execute(&engine, &server, Some(body), &playbook.name, secrets, move |engine, sink| engine.run_manual(spec, sink)).await?;
        Ok(())
    }
}

struct Grant<'a> {
    server: &'a Server,
    manifest: &'a AnsibleManifest,
    dir: &'a str,
    effective: &'a EffectiveConfig,
    effective_json: String,
    request: &'a ProvisioningRequest,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que atrapa: guardar en cac el principio de la salida (la
    /// instalación de paquetes) o el primer recap en vez de cómo acabó, o el
    /// recap con sus colores crudos.
    #[test]
    fn what_is_kept_of_a_run_is_how_it_ended() {
        let lines: Vec<String> = ["PLAY RECAP de un play anterior", "TASK [x]", "ok: [gstd]", "\u{1b}[0;32mPLAY RECAP\u{1b}[0m *****", "", "gstd : ok=3 changed=1"].iter().map(|l| l.to_string()).collect();
        assert_eq!(play_recap(&lines), "PLAY RECAP *****\ngstd : ok=3 changed=1");
        assert_eq!(log_tail(&lines, 2), "\ngstd : ok=3 changed=1");
        assert_eq!(play_recap(&lines[1..3]), "");
    }

    /// Rotura que atrapa: dar por bien una que se paró, o por bien una que
    /// no dijo su código.
    #[test]
    fn how_it_ended_in_cac_words() {
        assert_eq!(final_status(Some(0), true), RunEnd::Cancelled);
        assert_eq!(final_status(Some(0), false), RunEnd::Succeeded);
        assert_eq!(final_status(None, false), RunEnd::Failed);
        assert_eq!(final_status(Some(2), false), RunEnd::Failed);
    }

    /// Rotura que atrapa: aceptar `x y=1` o `=valor` como variable, o cortar
    /// el valor en el segundo `=`.
    #[test]
    fn extra_variables_are_name_equals_value() {
        let (vars, bad) = parse_extra_vars("a=1\n\n b_2 = x=y \n=v\nx y=1\n9a=1");
        assert_eq!(vars, [("a".to_string(), "1".to_string()), ("b_2".to_string(), " x=y".to_string())]);
        assert_eq!(bad, ["=v", "x y=1", "9a=1"]);
    }
}
