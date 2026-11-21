//! Instalar o actualizar el agente de un servidor desde su resumen
//! (`use-agent-lifecycle.ts`). El orden y la espera del latido son del core;
//! aquí, la pregunta, el SSH de esta máquina y la frase de cómo acabó.

use std::sync::Arc;

use cac_core::{server_edit_phrase, text_with, AgentIdentity, AgentInstallOutcome, AgentWait, Locale, Server, ServerEditError, AGENT_VERSION_LATEST};
use cac_ops::agent_install::AgentInstall;
use gpui_kit::{App, Global};

/// Lo de la máquina local que hace falta aquí. Las pruebas ponen uno de
/// mentira: el de verdad entra por SSH en el servidor.
pub trait AgentInstaller: Send + Sync {
    fn install(&self, install: &AgentInstall) -> Result<(), String>;
    fn wait(&self) -> AgentWait {
        AgentWait::default()
    }
}

struct SystemInstaller;

impl AgentInstaller for SystemInstaller {
    fn install(&self, install: &AgentInstall) -> Result<(), String> {
        cac_ops::agent_install::install_agent(install).map(|_| ())
    }
}

pub struct LocalInstaller(pub Arc<dyn AgentInstaller>);

impl Global for LocalInstaller {}

pub(crate) fn installer(cx: &App) -> Arc<dyn AgentInstaller> {
    cx.try_global::<LocalInstaller>().map(|installer| installer.0.clone()).unwrap_or_else(|| Arc::new(SystemInstaller))
}

/// Lo que se lleva al servidor: su conexión, la llave elegida para él (si
/// no, el agente SSH ofrece todas y el servidor puede cortar antes) y la
/// identidad recién acuñada.
pub(crate) fn install_for(server: &Server, identity: AgentIdentity, identity_key: Option<String>, backend_url: &str) -> AgentInstall {
    AgentInstall {
        host: server.host.clone(),
        ssh_port: server.ssh_port,
        ssh_user: server.ssh_user.clone(),
        agent_port: server.agent_port,
        stack: None,
        identity_key,
        server_id: server.id.clone(),
        backend_url: backend_url.to_string(),
        agent_token: identity.token,
        session_key: identity.session_key,
    }
}

/// La frase de cómo acabó, y si es un aviso.
pub(crate) fn outcome_phrase(locale: Locale, name: &str, outcome: &Result<AgentInstallOutcome, ServerEditError>) -> (String, bool) {
    let outcome = match outcome {
        Ok(outcome) => *outcome,
        Err(error) => return (server_edit_phrase(locale, error, "common:servers.agentUpdate.failed"), true),
    };
    let mut phrase = match outcome {
        AgentInstallOutcome::Changed { from, to } => {
            text_with(locale, "common:servers.agentUpdate.changed", &[("name", name), ("from", &from.to_string()), ("to", &to.to_string())])
        }
        AgentInstallOutcome::Same { version } => text_with(locale, "common:servers.agentUpdate.same", &[("name", name), ("version", &version.to_string())]),
        AgentInstallOutcome::Silent => format!(
            "{} {}",
            text_with(locale, "common:servers.agentUpdate.silent", &[("name", name)]),
            cac_core::text(locale, "common:servers.agentUpdate.silentHint")
        ),
    };
    if outcome.behind() {
        let version = match outcome {
            AgentInstallOutcome::Changed { to, .. } | AgentInstallOutcome::Same { version: to } => to,
            AgentInstallOutcome::Silent => 0,
        };
        phrase = format!(
            "{phrase} {}",
            text_with(locale, "common:servers.agentUpdate.behind", &[("version", &version.to_string()), ("latest", &AGENT_VERSION_LATEST.to_string())])
        );
    }
    let warning = matches!(outcome, AgentInstallOutcome::Silent) || outcome.behind();
    (phrase, warning)
}
