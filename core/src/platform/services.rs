//! Los servicios de un swarm y sus logs (`ServerServices.tsx`).
//!
//! Se le piden al agente del servidor, no al backend: la lista es la de
//! Docker en ese momento.

use serde::{Deserialize, Serialize};

use crate::ansi::{ansi_spans, AnsiSpan};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ServiceReplicas {
    pub running: i64,
    pub desired: i64,
}

/// Un servicio, como lo escribe el agente (`domain.Service` de
/// `swarm-manage`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct SwarmService {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub stack: String,
    #[serde(default)]
    pub replicas: ServiceReplicas,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ServiceHealth {
    Healthy,
    /// Corre, pero con menos réplicas de las que pide.
    Degraded,
    Down,
}

impl ServiceHealth {
    pub fn label(self) -> &'static str {
        match self {
            ServiceHealth::Healthy => "healthy",
            ServiceHealth::Degraded => "degraded",
            ServiceHealth::Down => "down",
        }
    }
}

impl SwarmService {
    pub fn health(&self) -> ServiceHealth {
        let ServiceReplicas { running, desired } = self.replicas;
        if running == 0 {
            ServiceHealth::Down
        } else if running < desired {
            ServiceHealth::Degraded
        } else {
            ServiceHealth::Healthy
        }
    }
}

/// Los que casan con lo escrito, por nombre, imagen o stack, sin mirar
/// mayúsculas.
pub fn filter_services(services: &[SwarmService], filter: &str) -> Vec<SwarmService> {
    let needle = filter.trim().to_lowercase();
    services
        .iter()
        .filter(|service| needle.is_empty() || format!("{} {} {}", service.name, service.image, service.stack).to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

/// Lo que se guarda de un log: un servicio hablador llena esto en un minuto,
/// y más no se lee.
pub const LOG_LINES_KEPT: usize = 500;

/// Lo que tiene que aguantar abierto un log para que su corte cuente como
/// uno nuevo y no como el mismo fallo repetido.
pub const LOG_STEADY: std::time::Duration = std::time::Duration::from_secs(10);

/// Cuánto se juntan las líneas de log antes de pintarlas: menos de lo que
/// se nota, y bastante para no repintar por cada una.
pub const LOG_FLUSH: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LogLine {
    pub spans: Vec<AnsiSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum LogStatus {
    Connecting,
    Streaming,
    Reconnecting,
    /// Tres intentos seguidos sin abrirlo: se deja.
    Failed,
}

impl LogStatus {
    pub fn label_key(self) -> &'static str {
        match self {
            LogStatus::Connecting => "common:servers.connecting",
            LogStatus::Streaming => "common:servers.streaming",
            LogStatus::Reconnecting => "common:servers.reconnecting",
            LogStatus::Failed => "common:servers.connectionFailed",
        }
    }
}

/// El log que se está siguiendo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ServiceLogs {
    pub service_id: String,
    pub service_name: String,
    pub lines: Vec<LogLine>,
    pub status: LogStatus,
    /// Lo último que dijo el agente al fallar, para quien quiera saberlo.
    pub error: Option<String>,
}

impl ServiceLogs {
    pub(crate) fn new(service: &SwarmService) -> Self {
        ServiceLogs { service_id: service.id.clone(), service_name: service.name.clone(), lines: Vec::new(), status: LogStatus::Connecting, error: None }
    }

    /// Un evento de datos del agente: casi siempre una línea, pero un trozo
    /// de Docker puede traer varias.
    pub(crate) fn push(&mut self, data: &str) {
        self.lines.extend(data.split('\n').map(|line| LogLine { spans: ansi_spans(line) }));
        let over = self.lines.len().saturating_sub(LOG_LINES_KEPT);
        self.lines.drain(..over);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str, image: &str, stack: &str, running: i64, desired: i64) -> SwarmService {
        SwarmService { id: name.into(), name: name.into(), image: image.into(), stack: stack.into(), replicas: ServiceReplicas { running, desired }, ..Default::default() }
    }

    /// Rotura que atrapa: dar por sano uno que corre a medias, o por caído
    /// uno que corre.
    #[test]
    fn health_follows_the_replicas() {
        assert_eq!(service("a", "", "", 0, 1).health(), ServiceHealth::Down);
        assert_eq!(service("a", "", "", 1, 2).health(), ServiceHealth::Degraded);
        assert_eq!(service("a", "", "", 2, 2).health(), ServiceHealth::Healthy);
        assert_eq!(service("a", "", "", 0, 0).health(), ServiceHealth::Down);
    }

    /// Rotura que atrapa: buscar sólo en el nombre, distinguir mayúsculas,
    /// o no aguantar los espacios de alrededor.
    #[test]
    fn the_filter_reads_name_image_and_stack() {
        let all = [service("web", "ghcr.io/o/web:v1", "shop", 1, 1), service("db", "postgres:16", "", 1, 1)];
        let names = |filter: &str| filter_services(&all, filter).into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names(" POSTGRES "), ["db"]);
        assert_eq!(names("shop"), ["web"]);
        assert_eq!(names("web"), ["web"]);
        assert_eq!(names(""), ["web", "db"]);
        assert!(names("nada").is_empty());
    }

    /// Rotura que atrapa: crecer sin fin con un servicio hablador, o tirar
    /// las líneas nuevas en vez de las viejas.
    #[test]
    fn the_log_keeps_the_newest_lines() {
        let mut logs = ServiceLogs::new(&service("web", "", "", 1, 1));
        for n in 0..LOG_LINES_KEPT + 10 {
            logs.push(&format!("línea {n}"));
        }
        logs.push("a\nb");
        assert_eq!(logs.lines.len(), LOG_LINES_KEPT);
        let text = |line: &LogLine| line.spans.iter().map(|s| s.text.clone()).collect::<String>();
        assert_eq!(text(&logs.lines[0]), "línea 12");
        assert_eq!(text(logs.lines.last().unwrap()), "b");
    }
}
