//! Un servidor abierto: sus pestañas y su resumen (`ServerLayout.tsx` y
//! `ServerOverview.tsx`).
//!
//! Las formas de abajo son las de los tipos de la app de Tauri
//! (`types/incident.ts`, `types/deploy.ts`, `ServerOverview.tsx`): el
//! backend no escribe todavía fixtures de contrato para ellas, y esta tarea
//! no toca `backend/`.

use serde::{Deserialize, Serialize};

use super::servers::{Server, ServerKind};

/// Una pestaña de un servidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ServerTab {
    #[default]
    Overview,
    Services,
    Nodes,
    Stats,
    Deploy,
    Secrets,
    Backups,
    Provision,
}

impl ServerTab {
    pub fn label_key(self) -> &'static str {
        match self {
            ServerTab::Overview => "common:servers.tabs.overview",
            ServerTab::Services => "common:servers.tabs.services",
            ServerTab::Nodes => "common:servers.tabs.nodes",
            ServerTab::Stats => "common:servers.tabs.stats",
            ServerTab::Deploy => "common:servers.tabs.deploy",
            ServerTab::Secrets => "common:servers.tabs.secrets",
            ServerTab::Backups => "common:servers.tabs.backups",
            ServerTab::Provision => "common:servers.tabs.provision",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ServerTab::Overview => "overview",
            ServerTab::Services => "services",
            ServerTab::Nodes => "nodes",
            ServerTab::Stats => "stats",
            ServerTab::Deploy => "deploy",
            ServerTab::Secrets => "secrets",
            ServerTab::Backups => "backups",
            ServerTab::Provision => "provision",
        }
    }
}

/// Las pestañas de un servidor, en el orden de la app de Tauri. Uno de
/// kubernetes no lleva el agente de swarm: sólo su consola y provisioning.
pub fn server_tabs(kind: ServerKind) -> Vec<ServerTab> {
    use ServerTab::*;
    match kind {
        ServerKind::Kubernetes => vec![Overview, Provision],
        _ => vec![Overview, Services, Nodes, Stats, Deploy, Secrets, Backups, Provision],
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct IncidentServerRef {
    pub id: String,
    pub name: String,
}

/// Lo que ya mordió en un servidor, sin cuerpo.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct IncidentSummary {
    pub id: String,
    pub title: String,
    /// `open` o `fixed`.
    pub status: String,
    #[serde(default)]
    pub servers: Vec<IncidentServerRef>,
    #[serde(default)]
    pub areas: Vec<String>,
    pub recorded_at: String,
    #[serde(default)]
    pub fixed_at: Option<String>,
}

impl IncidentSummary {
    pub fn is_open(&self) -> bool {
        self.status == "open"
    }

    /// El día que cuenta: el del arreglo si lo hay.
    pub fn when(&self) -> &str {
        self.fixed_at.as_deref().unwrap_or(&self.recorded_at)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct IncidentPage {
    #[serde(default)]
    pub incidents: Vec<IncidentSummary>,
}

/// Un servicio que se despliega desde cac.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DeployableSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub current_image: String,
    /// `record` o `deploy`: qué hace cac cuando avisa el CI. En camelCase
    /// saldría `onCiNotify`; el backend lo escribe con las siglas enteras.
    #[serde(default, rename = "onCINotify")]
    pub on_ci_notify: String,
}

impl DeployableSummary {
    /// El tag de la imagen que corre (`shortRef` en `lib/deploy.ts`): sin
    /// tag es `latest`; un digest no cuenta como tag. Sin imagen, `None`.
    pub fn image_tag(&self) -> Option<String> {
        if self.current_image.is_empty() {
            return None;
        }
        let without_digest = self.current_image.split('@').next().unwrap_or_default();
        let slash = without_digest.rfind('/');
        match without_digest.rfind(':') {
            Some(colon) if slash.is_none_or(|slash| colon > slash) => Some(without_digest[colon + 1..].to_string()),
            _ => Some("latest".to_string()),
        }
    }
}

/// Lo último que se le aplicó.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ProvisioningRun {
    pub id: String,
    pub playbook: String,
    pub status: String,
    pub started_at: String,
    #[serde(default)]
    pub summary: String,
}

/// El resumen de un servidor abierto. Cada parte que falle al pedirla se
/// queda vacía: un servidor que no despliega nada desde cac está bien, sólo
/// no ha optado, y un endpoint caído no tumba la pantalla.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverview {
    pub server_id: String,
    pub incidents: Vec<IncidentSummary>,
    pub deployables: Vec<DeployableSummary>,
    pub last_run: Option<ProvisioningRun>,
}

/// La versión del agente desde la que sabe desplegar
/// (`AGENT_VERSION_DEPLOYS` en `lib/deploy.ts`).
pub const AGENT_VERSION_DEPLOYS: i64 = 3;

/// Lo que hay que decir de su agente, si algo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgentNote {
    /// Sin identidad: su estado no es su latido y no despliega desde cac.
    NoIdentity,
    /// Con identidad pero de una versión que no sabe desplegar.
    Old { version: i64 },
}

impl AgentNote {
    pub fn of(server: &Server) -> Option<AgentNote> {
        if !server.has_agent_token {
            Some(AgentNote::NoIdentity)
        } else if server.agent_version < AGENT_VERSION_DEPLOYS {
            Some(AgentNote::Old { version: server.agent_version })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que atrapa: enseñar las pestañas de swarm a un kubernetes, o
    /// quitarle provisioning (así se quedó gstd dado de alta como swarm).
    #[test]
    fn a_kubernetes_server_has_only_its_console_and_provisioning() {
        assert_eq!(server_tabs(ServerKind::Kubernetes), [ServerTab::Overview, ServerTab::Provision]);
        assert_eq!(server_tabs(ServerKind::DockerSwarm).len(), 8);
        assert_eq!(server_tabs(ServerKind::DockerSwarm)[0], ServerTab::Overview);
    }

    /// Rotura que atrapa: leer el puerto del registro como tag, o un digest
    /// como versión.
    #[test]
    fn the_image_tag_is_read_like_the_tauri_app() {
        let tag = |image: &str| DeployableSummary { current_image: image.into(), ..Default::default() }.image_tag();
        assert_eq!(tag("ghcr.io/o/r:abc1234"), Some("abc1234".into()));
        assert_eq!(tag("registry:5000/o/r"), Some("latest".into()));
        assert_eq!(tag("registry:5000/o/r:v2@sha256:ff"), Some("v2".into()));
        assert_eq!(tag("r"), Some("latest".into()));
        assert_eq!(tag(""), None);
    }

    /// Rotura que atrapa: no avisar de un agente sin identidad o viejo, o
    /// avisar de uno al día.
    #[test]
    fn the_agent_note_says_what_is_missing() {
        let server = |token, version| Server { has_agent_token: token, agent_version: version, ..Server::default() };
        assert_eq!(AgentNote::of(&server(false, 9)), Some(AgentNote::NoIdentity));
        assert_eq!(AgentNote::of(&server(true, 2)), Some(AgentNote::Old { version: 2 }));
        assert_eq!(AgentNote::of(&server(true, AGENT_VERSION_DEPLOYS)), None);
    }
}
