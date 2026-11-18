//! Los comandos de Tauri de la pestaña Provisioning.
//!
//! Ansible en sí —el manifiesto, el plan, lanzar `ansible-playbook`, tachar
//! secretos— vive en `cac_mcp::ansible` (`mcp/src/ansible.rs`), sin Tauri,
//! porque lo corren también `cac provision` y el servidor MCP solo. Aquí queda
//! únicamente lo que es de la ventana: cada comando con el nombre que invoca la
//! página, y el canal por el que le llega la salida.

use cac_mcp::ansible::{self as engine, AnsibleEvent, AnsibleTools, InventoryHost, Manifest, PlanGaps, RunSpec, StartedRun};
use tauri::ipc::Channel;

pub use engine::close_all;

/// La salida de una ejecución, hacia el canal de la ventana.
fn to_window(on_event: Channel<AnsibleEvent>) -> engine::Sink {
    std::sync::Arc::new(move |e| {
        let _ = on_event.send(e);
    })
}

/// Qué le falta a la config de un servidor para correr un playbook (la
/// pestaña lo enseña antes de dejar apretar).
#[tauri::command]
pub fn ansible_plan_gaps(project_dir: String, playbook_id: String, effective: serde_json::Value) -> Result<PlanGaps, String> {
    engine::ansible_plan_gaps(project_dir, playbook_id, effective)
}

/// Corre un playbook con la config de cac de un servidor; con `check`, sólo
/// lo ensaya.
#[tauri::command]
pub async fn ansible_run_with_config(
    project_dir: String,
    playbook_id: String,
    effective: serde_json::Value,
    check: Option<bool>,
    on_event: Channel<AnsibleEvent>,
) -> Result<StartedRun, String> {
    engine::ansible_run_with_config(project_dir, playbook_id, effective, check, to_window(on_event))
}

/// Lee la salida de un ensayo como lo que cac evalúa.
#[tauri::command]
pub fn ansible_check_report(lines: Vec<String>, exit_code: i32) -> serde_json::Value {
    engine::ansible_check_report(lines, exit_code)
}

#[tauri::command]
pub fn ansible_tools(project_dir: String, venv: Option<String>) -> AnsibleTools {
    engine::ansible_tools(project_dir, venv)
}

/// El manifest del proyecto, o lo que se deduce del repo si no tiene.
#[tauri::command]
pub fn ansible_manifest(project_dir: String) -> Result<Manifest, String> {
    engine::ansible_manifest(project_dir)
}

#[tauri::command]
pub fn ansible_inventory(project_dir: String, inventory: String, venv: Option<String>) -> Result<Vec<InventoryHost>, String> {
    engine::ansible_inventory(project_dir, inventory, venv)
}

/// Corre un playbook y va contando la salida por el canal. Devuelve el id con
/// el que se cancela.
#[tauri::command]
pub async fn ansible_run(spec: RunSpec, on_event: Channel<AnsibleEvent>) -> Result<String, String> {
    engine::run_playbook(spec, to_window(on_event))
}

/// Cancela una ejecución: el grupo entero, con `ssh` y `sudo` dentro.
#[tauri::command]
pub fn ansible_cancel(run_id: String) -> Result<(), String> {
    engine::ansible_cancel(run_id)
}

#[cfg(test)]
mod tests {
    /// «Ensayar» en la app es el mismo comando que aplicar, con `check`. El
    /// motor ya tiene su prueba de que `check` decide (`cac_mcp::ansible`,
    /// `the_app_rehearses_through_the_check_parameter`); ésta guarda la otra
    /// mitad, que desde la extracción vive aquí: que el comando de la ventana
    /// se lo pase. Si lo dejara caer, «Ensayar» aplicaría de verdad.
    #[test]
    fn the_window_command_hands_the_check_parameter_to_the_engine() {
        let src = include_str!("ansible.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let body = code.split("pub async fn ansible_run_with_config(").nth(1).and_then(|r| r.split("\n}\n").next()).unwrap();
        assert!(body.contains("check: Option<bool>,"), "el comando ya no recibe `check`");
        assert!(
            body.contains("engine::ansible_run_with_config(project_dir, playbook_id, effective, check, to_window(on_event))"),
            "el ensayo de la app no llega al motor"
        );
    }
}
