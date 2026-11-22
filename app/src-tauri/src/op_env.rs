//! Las variables de un Environment de 1Password viven en `cac_ops::op_env`,
//! compartido con la app nativa; aquí sólo su comando de Tauri.

#[tauri::command]
pub fn env_file_names(path: String) -> Result<Vec<String>, String> {
    cac_ops::op_env::env_file_names(&path)
}
