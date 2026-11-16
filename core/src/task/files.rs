//! Los ficheros de una tarea: verlos, bajarlos, subirlos y quitarlos.
//!
//! Vino de `uploadAttachment` / `deleteAttachment` en
//! `app/src/store/tasks.store.ts` y de `open_attachment` / `save_attachment`
//! en `app/src-tauri/src/lib.rs`. Los bytes viajan por aquí, con la sesión en
//! la cabecera: la dirección de un adjunto pide sesión, así que no se le
//! puede dar tal cual al sistema ni a un navegador.
//!
//! Escribir en el disco no es de aquí: eso es de cada plataforma. Sí lo es
//! con qué nombre se escribe, porque el nombre lo puso quien subió el fichero
//! —puede ser un cliente— y no puede salirse de la carpeta.

use super::{TaskAttachment, Tasks};
use crate::ApiError;

/// El tope del servidor (`maxAttachmentBytes`), para decirlo antes de subir
/// treinta megas que va a rechazar.
pub const MAX_ATTACHMENT_BYTES: usize = 30 << 20;

/// Lo que se le pide al servidor por un adjunto, o `None` si su dirección no
/// es una de las nuestras.
///
/// Sólo rutas de la API de este servidor. Una dirección entera (las filas de
/// antes del proxy guardan la del bucket) o cualquier otra cosa no se pide:
/// la petición lleva la sesión, y la sesión no sale de este servidor.
pub fn attachment_path(url: &str) -> Option<&str> {
    let ours = url.starts_with("/api/v1/")
        && !url.contains("//")
        && !url.split('/').any(|part| part == "..")
        && !url.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '\\' | '@'));
    ours.then_some(url)
}

/// Un nombre de fichero que no se sale de la carpeta donde se escribe.
///
/// Sólo letras, números, punto, guiones y espacios; sin puntos por delante
/// (ni `..` ni un fichero oculto) y nunca vacío.
pub fn safe_file_name(name: &str) -> String {
    let kept: String = name.chars().filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ')).collect();
    // Puntos y espacios de delante, juntos: «. .oculto» no se queda en «.oculto».
    let kept = kept.trim_start_matches(|c: char| c == '.' || c.is_whitespace()).trim_end();
    if kept.is_empty() { "file".to_string() } else { kept.to_string() }
}

/// Si se puede enseñar como imagen. Por el tipo y, si no lo trae, por el
/// nombre: el tipo es opcional y hay filas sin él.
pub fn is_image(file: &TaskAttachment) -> bool {
    if file.content_type.as_deref().is_some_and(|kind| kind.starts_with("image/")) {
        return true;
    }
    let name = file.file_name.to_lowercase();
    ["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp"].iter().any(|ext| name.ends_with(&format!(".{ext}")))
}

/// Cuánto pesa, para leer: «12 KB», «1.4 MB».
pub fn size_label(bytes: i64) -> String {
    if bytes < 1024 * 1024 {
        format!("{} KB", ((bytes.max(0) as f64) / 1024.).round().max(1.) as i64)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024. * 1024.))
    }
}

/// Una frontera que no aparece en lo que separa.
fn boundary_for(bytes: &[u8]) -> String {
    let mut turn = 0u32;
    loop {
        let boundary = format!("----cac-file-{turn:08x}");
        let needle = boundary.as_bytes();
        if !bytes.windows(needle.len()).any(|window| window == needle) {
            return boundary;
        }
        turn += 1;
    }
}

/// El cuerpo `multipart/form-data` con el fichero en el campo `file`, y su
/// tipo de contenido (lleva la frontera).
///
/// En el nombre, las comillas y los saltos de línea van escapados como los
/// manda un navegador: sin eso, un nombre con una comilla corta la cabecera y
/// uno con un salto de línea escribe otra.
pub fn multipart(file_name: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let boundary = boundary_for(bytes);
    let name = file_name.replace('"', "%22").replace('\r', "%0D").replace('\n', "%0A");
    let mut body = Vec::with_capacity(bytes.len() + 256);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(format!("Content-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n").as_bytes());
    // El tipo lo decide el servidor mirando el fichero; el que se diga aquí
    // no llega a quien guarda.
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

impl Tasks {
    /// El adjunto con ese id en la tarea abierta, sea de la tarea o de uno de
    /// sus comentarios.
    fn attachment(&self, attachment_id: &str) -> Result<TaskAttachment, ApiError> {
        let detail = self.state().detail.ok_or_else(|| super::refused("no-task-open", "No task is open"))?;
        detail
            .attachments
            .into_iter()
            .chain(detail.comments.into_iter().flat_map(|comment| comment.attachments))
            .find(|file| file.id == attachment_id)
            .ok_or_else(|| super::refused("attachment-not-found", "That file is not in this task"))
    }

    /// Los bytes de un adjunto de la tarea abierta.
    ///
    /// Se pide por su id y no por una dirección: lo que se baja es siempre
    /// algo que la tarea abierta tiene.
    pub async fn attachment_bytes(&self, attachment_id: &str) -> Result<Vec<u8>, ApiError> {
        let file = self.attachment(attachment_id)?;
        let path = attachment_path(&file.url)
            .ok_or_else(|| super::refused("not-an-attachment", "That file does not live in this server"))?;
        self.core.download(path).await
    }

    /// Sube un fichero a la tarea abierta y la vuelve a pedir: la sección de
    /// adjuntos lee el detalle, y sin eso el fichero no aparece y subirlo
    /// parece haber fallado.
    pub async fn attach(&self, file_name: &str, bytes: Vec<u8>) -> Result<(), ApiError> {
        let task_id = self.open_id()?;
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(super::refused("too-large", "File exceeds 30 MB"));
        }
        let (content_type, body) = multipart(file_name, &bytes);
        self.core.upload(&format!("/api/v1/tasks/{task_id}/attachments"), &content_type, body).await?;
        self.refresh().await;
        Ok(())
    }

    /// Quita un adjunto de la tarea abierta. El fichero se queda guardado en
    /// el servidor: sólo deja de estar en la tarea.
    pub async fn remove_attachment(&self, attachment_id: &str) -> Result<(), ApiError> {
        let task_id = self.open_id()?;
        let file = self.attachment(attachment_id)?;
        self.core.call_ok("DELETE", &format!("/api/v1/tasks/{task_id}/attachments/{}", file.id), None).await?;
        self.refresh().await;
        Ok(())
    }
}
