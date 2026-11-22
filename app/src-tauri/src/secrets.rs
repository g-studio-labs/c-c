//! Rotar los secrets de un servicio y leer 1Password vive en
//! `cac_ops::secrets`, compartido con la app nativa; aquí sólo sus comandos de
//! Tauri, y el progreso de la rotación como el evento que escucha la página.

use std::collections::HashMap;

use cac_ops::secrets::{self, BackupKey, CaptureResult, ItemField, RefCheck, RotationResult, SecretRef, SshTarget};
use tauri::Emitter;

/// Ver `cac_ops::secrets::rotate_service_secrets`. Cada paso sale como
/// `secrets://rotation`, con la forma de `RotationProgress`.
#[tauri::command]
pub async fn rotate_service_secrets(
    app: tauri::AppHandle,
    target: SshTarget,
    service: String,
    refs: Vec<SecretRef>,
    env_files: Option<HashMap<String, String>>,
    command: Option<String>,
) -> Result<RotationResult, String> {
    secrets::rotate_service_secrets(&target, &service, &refs, &env_files.unwrap_or_default(), command.as_deref(), |p| {
        let _ = app.emit("secrets://rotation", p);
    })
}

#[tauri::command]
pub async fn check_op_refs(refs: Vec<String>, env_files: Option<HashMap<String, String>>) -> Vec<RefCheck> {
    secrets::check_op_refs(refs, &env_files.unwrap_or_default())
}

#[tauri::command]
pub async fn op_item_fields(item: String) -> Result<Vec<ItemField>, String> {
    secrets::op_item_fields(&item)
}

#[tauri::command]
pub async fn capture_service_env(target: SshTarget, service: String, vault: String, title: String) -> Result<CaptureResult, String> {
    secrets::capture_service_env(&target, &service, &vault, &title)
}

#[tauri::command]
pub fn create_backup_key(account: String, vault: String, title: String) -> Result<BackupKey, String> {
    secrets::create_backup_key(&account, &vault, &title)
}

#[cfg(test)]
mod tests {
    /// Lo que espera a 1Password o a un servidor no corre en el hilo de la
    /// ventana. Tauri corre ahí los comandos síncronos, y «Comprobar» —un `op
    /// read` por secret, más de veinte— dejaba la app trabada sin pintar nada
    /// hasta acabar (jose, 23-oct-2026). Y «Ensayar» lee los secretos de un
    /// playbook antes de lanzarlo: síncrono, el modal no se cerraba ni el
    /// botón pintaba su spinner durante ese rato (jose, 2-nov-2026).
    /// Roturas que atrapa: volver a hacer síncrono cualquiera de estos.
    #[test]
    fn what_waits_for_1password_or_a_server_does_not_freeze_the_window() {
        let src = [include_str!("secrets.rs"), include_str!("lib.rs"), include_str!("ansible.rs")].concat();
        for name in ["op_list_vaults", "op_item_create", "rotate_service_secrets", "check_op_refs", "op_item_fields", "capture_service_env", "ansible_run", "ansible_run_with_config"] {
            assert!(
                src.contains(&format!("async fn {name}(")),
                "{name} es síncrono: Tauri lo corre en el hilo de la ventana y la traba mientras espera"
            );
        }
    }
}
