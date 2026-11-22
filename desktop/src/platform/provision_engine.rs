//! El motor de Ansible de esta máquina para el core: `cac_mcp::ansible`, el
//! mismo que usan la app de Tauri y `cac provision`. Aquí sólo se traducen
//! los tipos; las pruebas ponen uno de mentira con `LocalEngine`.

use std::sync::Arc;

use cac_core::{AnsibleEngine, AnsibleManifest, AnsibleTools, EngineEvent, EngineSink, InventoryHost, ManualRun, PlanGaps};
use cac_mcp::ansible::{self as engine, AnsibleEvent, RunSpec, RunVar};
use gpui_kit::{App, Global};

struct SystemEngine;

fn to_engine(sink: EngineSink) -> engine::Sink {
    Arc::new(move |event| match event {
        AnsibleEvent::Line { text, .. } => sink(EngineEvent::Line(text)),
        AnsibleEvent::Exit { code, cancelled } => sink(EngineEvent::Exit { code, cancelled }),
    })
}

impl AnsibleEngine for SystemEngine {
    fn manifest(&self, project_dir: &str) -> Result<AnsibleManifest, String> {
        let manifest = engine::ansible_manifest(project_dir.to_string())?;
        serde_json::to_value(manifest).and_then(serde_json::from_value).map_err(|error| error.to_string())
    }

    fn tools(&self, project_dir: &str, venv: Option<&str>) -> AnsibleTools {
        let tools = engine::ansible_tools(project_dir.to_string(), venv.map(str::to_string));
        AnsibleTools { platform_supported: tools.platform_supported, reason: tools.reason, ansible_playbook: tools.ansible_playbook }
    }

    fn inventory(&self, project_dir: &str, inventory: &str, venv: Option<&str>) -> Result<Vec<InventoryHost>, String> {
        let hosts = engine::ansible_inventory(project_dir.to_string(), inventory.to_string(), venv.map(str::to_string))?;
        Ok(hosts.into_iter().map(|host| InventoryHost { name: host.name, groups: host.groups, ansible_host: host.ansible_host }).collect())
    }

    fn plan_gaps(&self, project_dir: &str, playbook_id: &str, effective: &str) -> Result<PlanGaps, String> {
        let effective = serde_json::from_str(effective).map_err(|error| error.to_string())?;
        let gaps = engine::ansible_plan_gaps(project_dir.to_string(), playbook_id.to_string(), effective)?;
        Ok(PlanGaps { vars: gaps.vars, secrets: gaps.secrets })
    }

    fn run_with_config(&self, project_dir: &str, playbook_id: &str, effective: &str, check: bool, sink: EngineSink) -> Result<String, String> {
        let effective = serde_json::from_str(effective).map_err(|error| error.to_string())?;
        engine::ansible_run_with_config(project_dir.to_string(), playbook_id.to_string(), effective, Some(check), to_engine(sink)).map(|started| started.run_id)
    }

    fn run_manual(&self, spec: ManualRun, sink: EngineSink) -> Result<String, String> {
        let spec = RunSpec {
            project_dir: spec.project_dir,
            venv: spec.venv,
            playbook: spec.playbook,
            inventory: spec.inventory,
            host: None,
            limit: spec.limit,
            vars: spec.vars.into_iter().map(|var| RunVar { name: var.name, value: var.value, from: var.from, secret: var.secret }).collect(),
            become_password: spec.become_password,
            become_password_ref: spec.become_password_ref,
            check: false,
        };
        engine::run_playbook(spec, to_engine(sink))
    }

    fn cancel(&self, run_id: &str) -> Result<(), String> {
        engine::ansible_cancel(run_id.to_string())
    }

    fn check_report(&self, lines: &[String], exit_code: i32) -> serde_json::Value {
        cac_mcp::provision_cli::check_report(lines, exit_code)
    }
}

/// El de esta app: el del sistema, salvo que alguien ponga otro.
pub struct LocalEngine(pub Arc<dyn AnsibleEngine>);

impl Global for LocalEngine {}

pub(crate) fn engine(cx: &App) -> Arc<dyn AnsibleEngine> {
    cx.try_global::<LocalEngine>().map(|engine| engine.0.clone()).unwrap_or_else(|| Arc::new(SystemEngine))
}

/// Al cerrar la app: nada sigue tocando un servidor sin nadie mirando.
pub(crate) fn stop_all() {
    engine::close_all();
}
