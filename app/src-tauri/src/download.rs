//! Bajar un backup a donde diga quien lo pide. Lo que baja, el `.part` y la
//! comprobación de https viven en `cac_ops::download` (con sus pruebas); aquí
//! sólo el diálogo y el aviso de progreso a la ventana.

use tauri::Emitter;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    id: String,
    done: u64,
    total: Option<u64>,
}

/// Pregunta dónde guardar y baja ahí. `None` si se cancela el diálogo, para
/// que la app calle en vez de contarlo como un fallo. El progreso llega como
/// `download://progress` con el `id` que se pasó.
#[tauri::command]
pub async fn download_to_file(app: tauri::AppHandle, id: String, url: String, file_name: String) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    if !cac_ops::download::allowed_url(&url) {
        return Err("Only https links can be downloaded".into());
    }
    let mut dialog = app.dialog().file().set_file_name(&file_name);
    if let Some(dir) = crate::dialog_dir(&app) {
        dialog = dialog.set_directory(dir);
    }
    let Some(chosen) = dialog.blocking_save_file() else {
        return Ok(None);
    };
    let target = chosen.into_path().map_err(|e| e.to_string())?;
    crate::remember_dialog_dir(target.parent());

    cac_ops::download::download_to(&url, &target, |done, total| {
        let _ = app.emit("download://progress", Progress { id: id.clone(), done, total });
    })
    .await?;
    Ok(Some(target.to_string_lossy().into_owned()))
}
