//! El instalador del agente vive en `cac_ops::agent_install`, compartido con
//! la app nativa; aquí sólo su comando de Tauri.

pub(crate) use cac_ops::agent_install::sh_quote;
use cac_ops::agent_install::{install_agent, AgentInstall};
use cac_ops::ssh::SshOutput;

#[tauri::command]
pub fn install_swarm_manage_agent(install: AgentInstall) -> Result<SshOutput, String> {
    install_agent(&install)
}
