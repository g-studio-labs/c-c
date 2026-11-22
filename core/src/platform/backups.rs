//! Los backups de bases de un servidor (`backups.store.ts` y `lib/backups.ts`,
//! #166): qué bases se respaldan, su historial y el log en vivo de cada uno, y
//! las llaves age para las que se cifran.
//!
//! El backup lo hace el agente dentro del contenedor de la base, lo cifra y lo
//! sube al bucket; lo que llega aquí es lo que va contando por el stream
//! (`backup:status`, `backup:log`). Como con los despliegues, el historial se
//! **funde** con lo que llega y no se vuelve a pedir.

use serde::{Deserialize, Serialize};

use super::{Platform, ServerEditError, SwarmService};
use crate::stream::ServerEvent;

/// La zona del horario cuando la organización no tiene una: la del backend.
pub const DEFAULT_BACKUP_TIMEZONE: &str = "America/Mexico_City";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum BackupEngine {
    #[default]
    Postgres,
    Mssql,
    #[serde(other)]
    Unknown,
}

impl BackupEngine {
    pub fn label_key(self) -> &'static str {
        match self {
            BackupEngine::Mssql => "common:backups.engine.mssql",
            BackupEngine::Postgres | BackupEngine::Unknown => "common:backups.engine.postgres",
        }
    }

    /// La variable de la imagen oficial: la contraseña de `sa`, o el usuario
    /// de Postgres.
    pub fn default_credential_env(self) -> &'static str {
        match self {
            BackupEngine::Mssql => "MSSQL_SA_PASSWORD",
            _ => "POSTGRES_USER",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BackupStatus {
    #[default]
    Queued,
    Running,
    Succeeded,
    Failed,
    #[serde(other)]
    Unknown,
}

impl BackupStatus {
    pub fn label_key(self) -> &'static str {
        match self {
            BackupStatus::Queued | BackupStatus::Unknown => "common:backups.status.queued",
            BackupStatus::Running => "common:backups.status.running",
            BackupStatus::Succeeded => "common:backups.status.succeeded",
            BackupStatus::Failed => "common:backups.status.failed",
        }
    }

    /// Todavía puede cambiar: mientras, no se lanza otro de la misma base
    /// (el backend contestaría 409).
    pub fn live(self) -> bool {
        matches!(self, BackupStatus::Queued | BackupStatus::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BackupKind {
    Scheduled,
    #[default]
    Manual,
    RestoreTest,
    #[serde(other)]
    Unknown,
}

impl BackupKind {
    pub fn label_key(self) -> &'static str {
        match self {
            BackupKind::Scheduled => "common:backups.kind.scheduled",
            BackupKind::Manual | BackupKind::Unknown => "common:backups.kind.manual",
            BackupKind::RestoreTest => "common:backups.kind.restore_test",
        }
    }
}

/// Una base que se respalda (`domain.BackupTarget`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct BackupTarget {
    pub id: String,
    pub org_id: String,
    pub server_id: String,
    /// El servicio de Swarm de la base, o un contenedor suelto por su nombre.
    pub service_name: String,
    pub engine: BackupEngine,
    pub database: String,
    /// El **nombre** de la variable del contenedor con la credencial.
    pub credential_env: String,
    /// Postgres: con usuario, la variable es la contraseña (o su `*_FILE`).
    pub username: String,
    pub every_hours: i64,
    pub anchor_time: String,
    pub timezone: String,
    pub keep_days: i64,
    pub keep_weekly: i64,
    pub keep_monthly: i64,
    pub restore_test_every_days: i64,
    pub enabled: bool,
    pub next_run_at: Option<String>,
    pub last_success_at: Option<String>,
    pub next_restore_test_at: Option<String>,
    pub last_restore_test_at: Option<String>,
    pub last_restore_test_ok: Option<bool>,
}

impl BackupTarget {
    /// Lo que se edita de ella.
    pub fn form(&self) -> BackupTargetForm {
        BackupTargetForm {
            service_name: self.service_name.clone(),
            engine: self.engine,
            database: self.database.clone(),
            credential_env: self.credential_env.clone(),
            username: self.username.clone(),
            every_hours: self.every_hours,
            anchor_time: self.anchor_time.clone(),
            timezone: self.timezone.clone(),
            keep_days: self.keep_days,
            keep_weekly: self.keep_weekly,
            keep_monthly: self.keep_monthly,
            restore_test_every_days: self.restore_test_every_days,
            enabled: self.enabled,
        }
    }
}

/// Una ejecución (`domain.Backup`). El log no viene en el historial: llega
/// por el stream o al pedir uno.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct Backup {
    pub id: String,
    pub created_at: String,
    pub server_id: String,
    pub target_id: String,
    pub kind: BackupKind,
    pub status: BackupStatus,
    /// `regular`, `weekly`, `monthly` o vacío.
    pub retention_class: String,
    pub size_bytes: i64,
    pub error: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    /// Lo borró el conserje de S3, vencido: la fila se queda como historia.
    pub purged_at: Option<String>,
}

impl Backup {
    /// Hay un fichero que bajar: uno bueno, que no es una prueba de
    /// restauración y que sigue en el bucket.
    pub fn downloadable(&self) -> bool {
        self.status == BackupStatus::Succeeded && self.kind != BackupKind::RestoreTest && self.purged_at.is_none()
    }
}

/// Una llave pública age para la que se cifran los backups.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct BackupRecipient {
    pub id: String,
    pub label: String,
    pub public_key: String,
}

/// Si los backups están encendidos en este cac y si el agente sabe hacerlos.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct BackupsStatus {
    pub enabled: bool,
    pub agent_ready: bool,
    pub agent_version: i64,
    pub min_agent: i64,
    pub has_recipients: bool,
    /// Los intervalos que se pueden elegir.
    pub every_hours: Vec<i64>,
}

/// El formulario de una base: es el cuerpo de `BackupTargetRequest` tal cual.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct BackupTargetForm {
    pub service_name: String,
    pub engine: BackupEngine,
    pub database: String,
    pub credential_env: String,
    pub username: String,
    pub every_hours: i64,
    pub anchor_time: String,
    pub timezone: String,
    pub keep_days: i64,
    pub keep_weekly: i64,
    pub keep_monthly: i64,
    pub restore_test_every_days: i64,
    pub enabled: bool,
}

impl BackupTargetForm {
    /// Lo que trae al abrirlo, para una base detectada o de cero.
    pub fn new(detected: Option<&DetectedDatabase>, timezone: &str) -> BackupTargetForm {
        let engine = detected.map(|found| found.engine).unwrap_or_default();
        BackupTargetForm {
            service_name: detected.map(|found| found.service_name.clone()).unwrap_or_default(),
            engine,
            database: String::new(),
            credential_env: engine.default_credential_env().to_string(),
            username: String::new(),
            every_hours: 24,
            anchor_time: "03:30".into(),
            timezone: timezone.to_string(),
            keep_days: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            restore_test_every_days: 7,
            enabled: true,
        }
    }

    /// Otro motor: la variable vuelve a la de su imagen y el usuario, que
    /// sólo es de Postgres, se va.
    pub fn set_engine(&mut self, engine: BackupEngine) {
        self.engine = engine;
        self.credential_env = engine.default_credential_env().to_string();
        self.username.clear();
    }

    /// Con usuario, la variable pasa a ser la contraseña: se propone la de la
    /// imagen oficial, en fichero; sin él, vuelve la del usuario. Una
    /// variable escrita a mano no se toca.
    pub fn set_username(&mut self, username: &str) {
        self.username = username.trim().to_string();
        if !self.username.is_empty() && self.credential_env == "POSTGRES_USER" {
            self.credential_env = "POSTGRES_PASSWORD_FILE".into();
        }
        if self.username.is_empty() && self.credential_env == "POSTGRES_PASSWORD_FILE" {
            self.credential_env = "POSTGRES_USER".into();
        }
    }

    /// Lo mínimo para mandarlo; el resto lo comprueba el backend.
    pub fn ready(&self) -> bool {
        !self.service_name.trim().is_empty() && !self.database.trim().is_empty() && !self.credential_env.trim().is_empty()
    }

    /// Cuántos backups se guardan como mucho, para decirlo en el formulario:
    /// los días, más una semanal por semana guardada y una mensual por mes.
    pub fn retained_count(&self) -> i64 {
        let every = self.every_hours.max(1);
        (self.keep_days * 24 + every - 1) / every + self.keep_weekly + self.keep_monthly
    }

    fn body(&self) -> serde_json::Value {
        let mut trimmed = self.clone();
        for field in [&mut trimmed.service_name, &mut trimmed.database, &mut trimmed.credential_env, &mut trimmed.username, &mut trimmed.timezone] {
            *field = field.trim().to_string();
        }
        serde_json::to_value(trimmed).unwrap_or_default()
    }
}

/// Una base que se ve entre los servicios, por su imagen.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DetectedDatabase {
    pub service_name: String,
    pub engine: BackupEngine,
    pub credential_env: String,
}

/// Las bases de los servicios que aún no se respaldan. Sólo las imágenes
/// oficiales de SQL Server y Postgres (también con registro delante, o las de
/// bitnami); lo demás no se adivina y se añade a mano.
pub fn detect_databases(services: &[SwarmService], already: &[BackupTarget]) -> Vec<DetectedDatabase> {
    services
        .iter()
        .filter(|service| !already.iter().any(|target| target.service_name == service.name))
        .filter_map(|service| {
            let image = service.image.to_lowercase();
            let image = image.split('@').next().unwrap_or_default();
            let last = image.rsplit('/').next().unwrap_or_default();
            let name = last.split(':').next().unwrap_or_default();
            let engine = if image.contains("mssql/server") {
                BackupEngine::Mssql
            } else if matches!(name, "postgres" | "postgresql" | "postgis") || image.contains("bitnami/postgresql") {
                BackupEngine::Postgres
            } else {
                return None;
            };
            Some(DetectedDatabase { service_name: service.name.clone(), engine, credential_env: engine.default_credential_env().to_string() })
        })
        .collect()
}

/// Una llave pública age X25519 bien formada, como la pide el backend:
/// `age1` y 58 caracteres de bech32. Una privada (`AGE-SECRET-KEY-1…`) no lo
/// es, ni en minúsculas.
pub fn is_age_public_key(key: &str) -> bool {
    const BECH32: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let key = key.trim();
    key.len() == 62 && key.starts_with("age1") && key[4..].chars().all(|c| BECH32.contains(c))
}

/// Unas líneas más del log de un backup (`backup:log`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupLogEvent {
    pub backup_id: String,
    pub target_id: String,
    pub lines: Vec<String>,
}

/// Los backups del servidor abierto.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct BackupsState {
    pub server_id: String,
    pub status: Option<BackupsStatus>,
    pub targets: Option<Vec<BackupTarget>>,
    pub recipients: Option<Vec<BackupRecipient>>,
    /// Por qué no llegó lo de arriba. Lo que hubiera se queda.
    pub error: Option<String>,
    /// El historial de todas sus bases, lo más nuevo primero.
    pub runs: Vec<Backup>,
    /// El backup cuyo log se mira, y su log.
    pub log_of: Option<String>,
    pub log: String,
}

impl BackupsState {
    /// Los de esa base, lo más nuevo primero.
    pub fn runs_of(&self, target_id: &str) -> Vec<Backup> {
        self.runs.iter().filter(|run| run.target_id == target_id).cloned().collect()
    }

    /// Hay uno de esa base en cola o en curso.
    pub fn busy(&self, target_id: &str) -> bool {
        self.runs.iter().any(|run| run.target_id == target_id && run.status.live())
    }

    /// Un backup que avanza, del stream o de la respuesta a lanzarlo. Lo que
    /// cambia de su base al acabar (último bueno, última prueba) se apunta
    /// también: el backend no manda la base de nuevo, y sin eso «Probar
    /// restauración» seguía apagado tras el primer backup bueno.
    pub fn on_status(&mut self, backup: Backup) {
        if backup.id.is_empty() {
            return;
        }
        let when = backup.finished_at.clone().unwrap_or_else(|| backup.created_at.clone());
        if let Some(target) = self.targets.iter_mut().flatten().find(|target| target.id == backup.target_id) {
            match (backup.kind, backup.status) {
                (BackupKind::RestoreTest, BackupStatus::Succeeded | BackupStatus::Failed) => {
                    target.last_restore_test_at = Some(when);
                    target.last_restore_test_ok = Some(backup.status == BackupStatus::Succeeded);
                }
                (BackupKind::RestoreTest, _) => {}
                (_, BackupStatus::Succeeded) => target.last_success_at = Some(when),
                _ => {}
            }
        }
        match self.runs.iter_mut().find(|known| known.id == backup.id) {
            Some(known) => *known = backup,
            None => self.runs.insert(0, backup),
        }
    }

    /// Unas líneas más, si son del log que se mira.
    pub fn on_log(&mut self, event: BackupLogEvent) {
        if self.log_of.as_deref() != Some(event.backup_id.as_str()) {
            return;
        }
        for line in event.lines {
            self.log.push_str(&line);
            self.log.push('\n');
        }
    }

    fn merge_runs(&mut self, fetched: Vec<Backup>) {
        // Lo que llegó por el stream mientras se pedía es más nuevo.
        for run in fetched {
            if !self.runs.iter().any(|known| known.id == run.id) {
                self.runs.push(run);
            }
        }
        self.runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    }
}

/// Un backup pedido de uno en uno: trae su log entero.
#[derive(Deserialize, Default)]
#[serde(default)]
struct BackupWithLog {
    log: String,
}

#[derive(Deserialize)]
struct Presigned {
    url: String,
}

impl Platform {
    fn backups_route(&self, rest: &str) -> Result<(String, String), ServerEditError> {
        let server = self.open().ok_or(ServerEditError::ServerGone)?;
        let route = format!("/api/v1/servers/{}/backups{rest}", server.id);
        Ok((server.id, route))
    }

    fn edit_backups(&self, server_id: &str, edit: impl FnOnce(&mut BackupsState)) {
        self.change(|data| {
            if let Some(backups) = data.backups.as_mut().filter(|backups| backups.server_id == server_id) {
                edit(backups);
            }
        });
    }

    fn manages_backups(&self) -> Result<(), ServerEditError> {
        if self.permissions().manage_backups {
            Ok(())
        } else {
            Err(ServerEditError::AdminOnly)
        }
    }

    /// El formulario de una base nueva, con la zona de la organización.
    pub fn new_backup_form(&self, detected: Option<&DetectedDatabase>) -> BackupTargetForm {
        let timezone = self.core.orgs().state().current().map(|org| org.timezone.clone()).filter(|zone| !zone.trim().is_empty());
        BackupTargetForm::new(detected, timezone.as_deref().unwrap_or(DEFAULT_BACKUP_TIMEZONE))
    }

    /// Pide si los backups están encendidos, las bases, las llaves y el
    /// historial de cada base. Lo que falle deja lo que hubiera.
    pub async fn load_backups(&self) {
        let Some(server) = self.open() else { return };
        self.change(|data| {
            if data.backups.as_ref().is_none_or(|backups| backups.server_id != server.id) {
                data.backups = Some(BackupsState { server_id: server.id.clone(), ..Default::default() });
            }
        });
        let route = |rest: &str| format!("/api/v1/servers/{}/backups/{rest}", server.id);
        let (core, path) = (self.core.clone(), route("status"));
        let status = tokio::spawn(async move { core.get::<BackupsStatus>(&path).await });
        let (core, path) = (self.core.clone(), route("targets"));
        let targets = tokio::spawn(async move { core.get_list::<BackupTarget>(&path).await });
        let (core, path) = (self.core.clone(), route("recipients"));
        let recipients = tokio::spawn(async move { core.get_list::<BackupRecipient>(&path).await });
        let (status, targets, recipients) = (super::flatten(status.await), super::flatten(targets.await), super::flatten(recipients.await));
        let error = [status.as_ref().err(), targets.as_ref().err(), recipients.as_ref().err()].into_iter().flatten().next().cloned();
        let ids: Vec<String> = targets.iter().flatten().map(|target| target.id.clone()).collect();
        self.edit_backups(&server.id, |backups| {
            if let Ok(status) = status {
                backups.status = Some(status);
            }
            if let Ok(targets) = targets {
                backups.targets = Some(targets);
            }
            if let Ok(recipients) = recipients {
                backups.recipients = Some(recipients);
            }
            backups.error = error;
        });
        let histories: Vec<_> = ids
            .into_iter()
            .map(|id| {
                let (core, path) = (self.core.clone(), route(&format!("targets/{id}/history")));
                tokio::spawn(async move { core.get_list::<Backup>(&path).await })
            })
            .collect();
        for history in histories {
            if let Ok(rows) = super::flatten(history.await) {
                self.edit_backups(&server.id, |backups| backups.merge_runs(rows));
            }
        }
    }

    /// Da de alta una base (`target_id` vacío) o la cambia; la fila vuelve
    /// como la deja el backend, ya con su siguiente hora.
    pub async fn save_backup_target(&self, target_id: Option<&str>, form: &BackupTargetForm) -> Result<BackupTarget, ServerEditError> {
        self.manages_backups()?;
        if !form.ready() {
            return Err(ServerEditError::BackupIncomplete);
        }
        let (server_id, route) = self.backups_route(&match target_id {
            Some(id) => format!("/targets/{id}"),
            None => "/targets".into(),
        })?;
        let saved: BackupTarget = self.core.call(if target_id.is_some() { "PATCH" } else { "POST" }, &route, Some(form.body())).await?;
        let row = saved.clone();
        self.edit_backups(&server_id, |backups| {
            let targets = backups.targets.get_or_insert_with(Vec::new);
            match targets.iter_mut().find(|known| known.id == row.id) {
                Some(slot) => *slot = row,
                None => targets.push(row),
            }
        });
        Ok(saved)
    }

    /// Deja de respaldar una base; lo que ya hay se queda en el bucket.
    /// **Quien llama ya preguntó.**
    pub async fn delete_backup_target(&self, target_id: &str) -> Result<(), ServerEditError> {
        self.manages_backups()?;
        let (server_id, route) = self.backups_route(&format!("/targets/{target_id}"))?;
        self.core.call_ok("DELETE", &route, None).await?;
        self.edit_backups(&server_id, |backups| {
            if let Some(targets) = backups.targets.as_mut() {
                targets.retain(|known| known.id != target_id);
            }
            backups.runs.retain(|run| run.target_id != target_id);
        });
        Ok(())
    }

    /// Un backup ahora, fuera de su horario.
    pub async fn run_backup(&self, target_id: &str) -> Result<Backup, ServerEditError> {
        self.launch_backup(target_id, "run").await
    }

    /// Restaura el último bueno en una base temporal y la revisa. Pesa en la
    /// base de la web: **quien llama ya preguntó.**
    pub async fn test_backup_restore(&self, target_id: &str) -> Result<Backup, ServerEditError> {
        self.launch_backup(target_id, "test").await
    }

    async fn launch_backup(&self, target_id: &str, what: &str) -> Result<Backup, ServerEditError> {
        self.manages_backups()?;
        let (server_id, route) = self.backups_route(&format!("/targets/{target_id}/{what}"))?;
        let started: Backup = self.core.call("POST", &route, Some(serde_json::json!({}))).await?;
        let row = started.clone();
        self.edit_backups(&server_id, |backups| backups.on_status(row));
        Ok(started)
    }

    /// Mira el log de un backup: lo que lleve, y lo que vaya llegando.
    pub async fn open_backup_log(&self, backup_id: &str) {
        let Ok((server_id, route)) = self.backups_route(&format!("/runs/{backup_id}")) else { return };
        self.edit_backups(&server_id, |backups| {
            backups.log_of = Some(backup_id.to_string());
            backups.log.clear();
        });
        if let Ok(full) = self.core.get::<BackupWithLog>(&route).await {
            self.edit_backups(&server_id, |backups| {
                if backups.log_of.as_deref() == Some(backup_id) && full.log.len() >= backups.log.len() {
                    backups.log = full.log;
                }
            });
        }
    }

    pub fn close_backup_log(&self) {
        let server_id = self.open().map(|server| server.id).unwrap_or_default();
        self.edit_backups(&server_id, |backups| {
            backups.log_of = None;
            backups.log.clear();
        });
    }

    /// Una URL de cinco minutos al fichero **cifrado**. Se devuelve a quien
    /// la pidió para bajarlo; no se guarda en el estado.
    pub async fn backup_download_url(&self, backup_id: &str) -> Result<String, ServerEditError> {
        self.manages_backups()?;
        let (_, route) = self.backups_route(&format!("/runs/{backup_id}/download"))?;
        let presigned: Presigned = self.core.call("POST", &route, Some(serde_json::json!({}))).await?;
        Ok(presigned.url)
    }

    /// Añade una llave pública. Se comprueba antes de mandarla: una privada
    /// pegada por error no sale de esta máquina.
    pub async fn add_backup_recipient(&self, label: &str, public_key: &str) -> Result<(), ServerEditError> {
        self.manages_backups()?;
        let (label, public_key) = (label.trim(), public_key.trim());
        if !is_age_public_key(public_key) {
            return Err(ServerEditError::BadAgeKey);
        }
        if label.is_empty() {
            return Err(ServerEditError::BackupIncomplete);
        }
        let (server_id, route) = self.backups_route("/recipients")?;
        let added: BackupRecipient = self.core.call("POST", &route, Some(serde_json::json!({ "label": label, "publicKey": public_key }))).await?;
        self.edit_backups(&server_id, |backups| {
            backups.recipients.get_or_insert_with(Vec::new).push(added);
            if let Some(status) = backups.status.as_mut() {
                status.has_recipients = true;
            }
        });
        Ok(())
    }

    /// Quita una llave: los backups nuevos ya no se cifran para ella.
    /// **Quien llama ya preguntó.**
    pub async fn delete_backup_recipient(&self, recipient_id: &str) -> Result<(), ServerEditError> {
        self.manages_backups()?;
        let (server_id, route) = self.backups_route(&format!("/recipients/{recipient_id}"))?;
        self.core.call_ok("DELETE", &route, None).await?;
        self.edit_backups(&server_id, |backups| {
            let recipients = backups.recipients.get_or_insert_with(Vec::new);
            recipients.retain(|known| known.id != recipient_id);
            let any = !recipients.is_empty();
            if let Some(status) = backups.status.as_mut() {
                status.has_recipients = any;
            }
        });
        Ok(())
    }

    /// Lo que cuenta el stream de los backups.
    pub(crate) fn backup_event(&self, event: &ServerEvent) {
        match event.event.as_str() {
            "backup:status" => {
                if let Ok(backup) = serde_json::from_str::<Backup>(&event.data) {
                    let server_id = backup.server_id.clone();
                    self.edit_backups(&server_id, |backups| backups.on_status(backup));
                }
            }
            "backup:log" => {
                if let Ok(lines) = serde_json::from_str::<BackupLogEvent>(&event.data) {
                    self.change(|data| {
                        if let Some(backups) = data.backups.as_mut() {
                            backups.on_log(lines);
                        }
                    });
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, target: &str, kind: BackupKind, status: BackupStatus) -> Backup {
        Backup { id: id.into(), target_id: target.into(), kind, status, created_at: format!("2026-11-21T10:00:0{}Z", id.len()), ..Default::default() }
    }

    fn with_target() -> BackupsState {
        BackupsState { targets: Some(vec![BackupTarget { id: "t1".into(), ..Default::default() }]), ..Default::default() }
    }

    /// Rotura que atrapa: duplicar la fila en vez de fundirla, no apuntar el
    /// último bueno (y dejar «Probar restauración» apagado), tomar una prueba
    /// de restauración por un backup bueno, o no apuntar si la prueba falló.
    #[test]
    fn a_status_merges_and_moves_its_target() {
        let mut state = with_target();
        state.on_status(run("b1", "t1", BackupKind::Manual, BackupStatus::Queued));
        assert!(state.busy("t1"));
        state.on_status(Backup { finished_at: Some("2026-11-21T10:05:00Z".into()), ..run("b1", "t1", BackupKind::Manual, BackupStatus::Succeeded) });
        assert_eq!(state.runs.len(), 1);
        assert!(!state.busy("t1"));
        let target = &state.targets.as_ref().unwrap()[0];
        assert_eq!(target.last_success_at.as_deref(), Some("2026-11-21T10:05:00Z"));

        state.on_status(Backup { finished_at: Some("2026-11-21T11:00:00Z".into()), ..run("r1", "t1", BackupKind::RestoreTest, BackupStatus::Failed) });
        let target = &state.targets.as_ref().unwrap()[0];
        assert_eq!(target.last_success_at.as_deref(), Some("2026-11-21T10:05:00Z"), "una prueba no es un backup bueno");
        assert_eq!((target.last_restore_test_at.as_deref(), target.last_restore_test_ok), (Some("2026-11-21T11:00:00Z"), Some(false)));
        assert_eq!(state.runs_of("t1").iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r1", "b1"]);
    }

    /// Rotura que atrapa: pegar al log que se mira las líneas de otro
    /// backup, o juntarlas sin salto.
    #[test]
    fn log_lines_land_only_on_their_backup() {
        let mut state = BackupsState { log_of: Some("b1".into()), ..Default::default() };
        state.on_log(BackupLogEvent { backup_id: "b1".into(), lines: vec!["dump".into(), "encrypt".into()], ..Default::default() });
        state.on_log(BackupLogEvent { backup_id: "b2".into(), lines: vec!["ajeno".into()], ..Default::default() });
        assert_eq!(state.log, "dump\nencrypt\n");
    }

    /// Rotura que atrapa: aceptar una llave privada (o una pública con
    /// letras que bech32 no usa) como destinatario.
    #[test]
    fn only_an_age_public_key_is_a_recipient() {
        let public = format!("age1{}", "q".repeat(58));
        assert!(is_age_public_key(&public));
        assert!(is_age_public_key(&format!(" {public}\n")));
        assert!(!is_age_public_key(&format!("age1{}", "b".repeat(58))), "la b no es de bech32");
        assert!(!is_age_public_key(&format!("age1{}", "q".repeat(57))));
        assert!(!is_age_public_key(&format!("AGE-SECRET-KEY-1{}", "Q".repeat(58))));
        assert!(!is_age_public_key(&format!("age-secret-key-1{}", "q".repeat(58))));
    }

    fn service(name: &str, image: &str) -> SwarmService {
        SwarmService { id: name.into(), name: name.into(), image: image.into(), stack: String::new(), replicas: Default::default(), updated_at: String::new() }
    }

    /// Rotura que atrapa: tomar por base cualquier imagen que mencione
    /// postgres (un exportador), no reconocerla con registro o digest
    /// delante, o volver a ofrecer una que ya se respalda.
    #[test]
    fn databases_are_detected_by_their_official_image() {
        let services = [
            service("shop_db", "docker.io/library/postgres:16@sha256:ab"),
            service("sql", "mcr.microsoft.com/mssql/server:2022-latest"),
            service("exporter", "prometheuscommunity/postgres-exporter:v0.15"),
            service("legacy", "bitnami/postgresql:15"),
            service("done", "postgres:15"),
        ];
        let done = [BackupTarget { service_name: "done".into(), ..Default::default() }];
        let found = detect_databases(&services, &done);
        assert_eq!(
            found.iter().map(|d| (d.service_name.as_str(), d.engine, d.credential_env.as_str())).collect::<Vec<_>>(),
            [("shop_db", BackupEngine::Postgres, "POSTGRES_USER"), ("sql", BackupEngine::Mssql, "MSSQL_SA_PASSWORD"), ("legacy", BackupEngine::Postgres, "POSTGRES_USER")]
        );
    }

    /// Rotura que atrapa: dejar la variable del motor anterior al cambiarlo,
    /// no pasar a la contraseña al poner un usuario (el agente tomaría el
    /// usuario por contraseña), o pisar una variable escrita a mano; y contar
    /// mal lo que se guarda.
    #[test]
    fn the_form_follows_engine_and_username() {
        let mut form = BackupTargetForm::new(None, "America/Mexico_City");
        assert!(!form.ready());
        form.set_username("postgres");
        assert_eq!(form.credential_env, "POSTGRES_PASSWORD_FILE");
        form.set_username(" ");
        assert_eq!(form.credential_env, "POSTGRES_USER");
        form.credential_env = "DB_PASS".into();
        form.set_username("app");
        assert_eq!(form.credential_env, "DB_PASS");
        form.set_engine(BackupEngine::Mssql);
        assert_eq!((form.credential_env.as_str(), form.username.as_str()), ("MSSQL_SA_PASSWORD", ""));
        assert_eq!(form.retained_count(), 7 + 4 + 6);
        form.every_hours = 6;
        assert_eq!(form.retained_count(), 28 + 4 + 6);
        form.every_hours = 48;
        assert_eq!(form.retained_count(), 4 + 4 + 6);
    }
}
