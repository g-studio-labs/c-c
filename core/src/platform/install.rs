//! Instalar o actualizar el agente de un servidor (`use-agent-lifecycle.ts`).
//!
//! Instalar y actualizar son lo mismo desde la v2 del agente: el backend
//! acuña una identidad nueva, que tumba la anterior, y la app la lleva al
//! servidor por SSH. Esa parte es de la máquina local (`cac_ops`), así que
//! quien llama la pasa; aquí va el orden y la espera del primer latido, que
//! es lo único que dice que el agente nuevo arrancó.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::servers::Server;

/// La versión del agente que instalan las apps (`AGENT_VERSION_LATEST` en
/// `lib/deploy.ts`, `AGENT_IMAGE` en `cac_ops`).
pub const AGENT_VERSION_LATEST: i64 = 14;

/// La identidad recién acuñada. No se pinta nunca: sin `Debug` derivado, un
/// `{:?}` en un log no la suelta.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentIdentity {
    pub token: String,
    pub session_key: String,
}

impl std::fmt::Debug for AgentIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentIdentity(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgentInstallPhase {
    /// Acuñando la identidad y llevándola por SSH.
    Installing,
    /// Ya se lanzó: esperando a que el agente nuevo lata.
    Waiting,
}

/// Una instalación en curso.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct AgentJob {
    pub server_id: String,
    pub phase: AgentInstallPhase,
}

/// Cómo acabó. Terminar el SSH sólo dice que se lanzó: un contenedor que no
/// arranca (una imagen que no baja, un secret malo) se ve igual que uno que
/// sí, y por eso se espera al latido.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgentInstallOutcome {
    Changed { from: i64, to: i64 },
    Same { version: i64 },
    /// No latió en todo el rato.
    Silent,
}

impl AgentInstallOutcome {
    /// Volvió, pero con una versión anterior: la imagen nueva no bajó y Swarm
    /// se quedó con la que tenía.
    pub fn behind(self) -> bool {
        match self {
            AgentInstallOutcome::Changed { to, .. } | AgentInstallOutcome::Same { version: to } => to < AGENT_VERSION_LATEST,
            AgentInstallOutcome::Silent => false,
        }
    }
}

/// Cuánto se espera el primer latido, y cada cuánto se mira.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentWait {
    pub total: Duration,
    pub every: Duration,
}

impl Default for AgentWait {
    fn default() -> Self {
        AgentWait { total: Duration::from_secs(90), every: Duration::from_secs(3) }
    }
}

/// Si el agente nuevo ya habló. La identidad nueva tumba la del viejo: desde
/// entonces, un latido distinto del de antes sólo puede ser del nuevo, sin
/// comparar relojes de dos máquinas.
pub fn agent_update_outcome(before: &Server, now: &Server) -> Option<AgentInstallOutcome> {
    let seen = now.agent_seen_at.as_deref()?;
    if before.agent_seen_at.as_deref() == Some(seen) {
        return None;
    }
    Some(if before.agent_version == now.agent_version {
        AgentInstallOutcome::Same { version: now.agent_version }
    } else {
        AgentInstallOutcome::Changed { from: before.agent_version, to: now.agent_version }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que atrapa: dar por vuelto al agente viejo (el mismo latido de
    /// antes) o a uno que todavía no latió.
    #[test]
    fn only_a_new_heartbeat_means_the_new_agent_spoke() {
        let server = |seen: Option<&str>, version| Server { agent_seen_at: seen.map(str::to_string), agent_version: version, ..Server::default() };
        assert_eq!(agent_update_outcome(&server(Some("t1"), 4), &server(Some("t1"), 4)), None);
        assert_eq!(agent_update_outcome(&server(Some("t1"), 4), &server(None, 4)), None);
        assert_eq!(agent_update_outcome(&server(None, 0), &server(Some("t2"), 14)), Some(AgentInstallOutcome::Changed { from: 0, to: 14 }));
        assert_eq!(agent_update_outcome(&server(Some("t1"), 14), &server(Some("t2"), 14)), Some(AgentInstallOutcome::Same { version: 14 }));
        assert!(AgentInstallOutcome::Same { version: AGENT_VERSION_LATEST - 1 }.behind());
        assert!(!AgentInstallOutcome::Changed { from: 3, to: AGENT_VERSION_LATEST }.behind());
        assert!(!AgentInstallOutcome::Silent.behind());
    }

    /// Rotura que atrapa: publicar un agente nuevo sin que la app nativa lo
    /// sepa, y avisar de «quedó atrás» a uno que está al día (o al revés).
    #[test]
    fn the_latest_version_is_the_one_published() {
        let version: i64 = include_str!("../../../swarm-manage/VERSION").trim().parse().expect("VERSION");
        assert_eq!(AGENT_VERSION_LATEST, version);
    }

    /// Rotura que atrapa: que un `{:?}` de la identidad suelte el token.
    #[test]
    fn the_identity_never_prints() {
        let identity = AgentIdentity { token: "cac_agent_SECRET".into(), session_key: "ab".repeat(32) };
        let printed = format!("{identity:?}");
        assert!(!printed.contains("SECRET") && !printed.contains("abab"));
    }
}
