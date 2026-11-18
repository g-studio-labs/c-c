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

/// Si un fichero de ese tamaño cabe. Quien lo lee del disco pregunta antes
/// de leerlo: cinco gigas no se cargan en memoria para que digan que no.
pub fn attachment_fits(bytes: u64) -> Result<(), ApiError> {
    if bytes > MAX_ATTACHMENT_BYTES as u64 {
        return Err(super::refused("too-large", "File exceeds 30 MB"));
    }
    Ok(())
}

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
    named_image(file.content_type.as_deref(), &file.file_name)
}

/// Si un fichero subido es una imagen, por el tipo que dijo el servidor o,
/// sin él, por su nombre. Lo mismo para un fichero de una tarea que de una
/// conversación.
pub fn named_image(content_type: Option<&str>, file_name: &str) -> bool {
    if content_type.is_some_and(|kind| kind.starts_with("image/")) {
        return true;
    }
    let name = file_name.to_lowercase();
    ["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp"].iter().any(|ext| name.ends_with(&format!(".{ext}")))
}

/// Qué imagen es, por cómo empieza el fichero; `None` si no es ninguna de
/// las que se enseñan.
///
/// Se mira el fichero y no su tipo ni su nombre: el tipo falta en todo lo
/// que entra por la integración, y un nombre lo pone quien sube el fichero
/// —puede ser un cliente—. Lo que no empieza como una imagen no se le da a
/// quien las decodifica.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    let at = |from: usize, magic: &[u8]| bytes.get(from..from + magic.len()) == Some(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if starts(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some("image/gif")
    } else if starts(b"RIFF") && at(8, b"WEBP") {
        Some("image/webp")
    } else if at(4, b"ftypavif") {
        Some("image/avif")
    } else if starts(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

/// El hueco de una miniatura: el de la app de siempre (`max-w-64 max-h-40`).
pub const THUMBNAIL_MAX_WIDTH: u32 = 256;
pub const THUMBNAIL_MAX_HEIGHT: u32 = 160;

/// De qué tamaño se enseña una imagen de ese tamaño como miniatura: entera,
/// sin deformarla, dentro del hueco; y nunca más grande de lo que es, que un
/// icono de 16 px ampliado a 160 es una mancha.
pub fn thumbnail_size(width: u32, height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let (w, h) = (width as f64, height as f64);
    let scale = (THUMBNAIL_MAX_WIDTH as f64 / w).min(THUMBNAIL_MAX_HEIGHT as f64 / h).min(1.);
    (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

/// Una imagen de la tarea, bajada para enseñarla.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Picture {
    /// Qué imagen es (`image/png`…), mirado en el fichero.
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Un texto con algo puesto dentro, y dónde queda el cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Cited {
    pub text: String,
    pub cursor: u32,
}

/// Cómo se cita un fichero de la tarea en un comentario o en la descripción:
/// una imagen como imagen y lo demás como enlace, con su nombre.
///
/// Vino de `insertUpload` en `app/src/components/markdown/MarkdownEditor.tsx`.
/// Se cita por la ruta del servidor, sin dominio ni credencial (la forma
/// canónica de `attachmentPath` en `app/src/lib/media.ts`): lo escrito contra
/// un servidor se lee en otro, y un token no acaba guardado en un texto.
/// `None` si el fichero no vive en este servidor: no hay con qué citarlo.
pub fn file_citation(file: &TaskAttachment) -> Option<String> {
    citation(&file.url, &file.file_name, is_image(file))
}

/// La cita de un fichero subido, de una tarea o de una conversación: una
/// imagen como imagen y lo demás como enlace, por la ruta del servidor.
pub fn citation(url: &str, file_name: &str, image: bool) -> Option<String> {
    let path = attachment_path(url)?;
    // El nombre lo puso quien subió el fichero: un corchete suyo no cierra
    // el enlace, ni un salto de línea lo parte.
    let name: String = file_name
        .chars()
        .flat_map(|c| match c {
            '[' | ']' | '\\' => vec!['\\', c],
            '\n' | '\r' => vec![' '],
            c => vec![c],
        })
        .collect();
    Some(if image { format!("![{name}]({path})") } else { format!("[{name}]({path})") })
}

/// El texto con ese fichero citado donde está el cursor, y el cursor detrás
/// para seguir escribiendo.
///
/// Una imagen es un bloque y va en su línea; un enlace va en la frase, con
/// un espacio a cada lado si no lo hay. Un cursor que no es de este texto
/// (cambió mientras subía el fichero) lo deja al final.
pub fn with_citation(text: &str, cursor: usize, file: &TaskAttachment) -> Option<Cited> {
    let citation = file_citation(file)?;
    // (`is_char_boundary` ya dice que no a un cursor de más allá del final.)
    let cursor = if text.is_char_boundary(cursor) { cursor } else { text.len() };
    let (before, after) = text.split_at(cursor);
    let put = if is_image(file) {
        let lead = if before.is_empty() || before.ends_with('\n') { "" } else { "\n" };
        format!("{lead}{citation}\n")
    } else {
        let lead = if before.chars().next_back().is_none_or(char::is_whitespace) { "" } else { " " };
        format!("{lead}{citation} ")
    };
    Some(Cited { text: format!("{before}{put}{after}"), cursor: (before.len() + put.len()) as u32 })
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

    /// Una imagen de la tarea abierta, para enseñarla. Viene por donde
    /// vienen todos sus ficheros —por su id, de este servidor, con la
    /// sesión en la cabecera—, y lo que llega y no es una imagen no se da
    /// por una.
    pub async fn picture(&self, attachment_id: &str) -> Result<Picture, ApiError> {
        let bytes = self.attachment_bytes(attachment_id).await?;
        let mime = image_type(&bytes).ok_or_else(|| super::refused("not-an-image", "That file is not an image"))?;
        Ok(Picture { mime: mime.to_string(), bytes })
    }

    /// Sube un fichero a la tarea abierta y la vuelve a pedir: la sección de
    /// adjuntos lee el detalle, y sin eso el fichero no aparece y subirlo
    /// parece haber fallado. Devuelve el adjunto que quedó, para citarlo.
    ///
    /// Un fichero que se adjunta al comentar también es de la tarea, como en
    /// la app de siempre: se sube igual, y el comentario lo cita.
    pub async fn attach(&self, file_name: &str, bytes: Vec<u8>) -> Result<TaskAttachment, ApiError> {
        let task_id = self.open_id()?;
        attachment_fits(bytes.len() as u64)?;
        let (content_type, body) = multipart(file_name, &bytes);
        let file: TaskAttachment = self.core.upload(&format!("/api/v1/tasks/{task_id}/attachments"), &content_type, body).await?;
        self.refresh().await;
        Ok(file)
    }

    /// El fichero de la tarea abierta al que lleva ese enlace, si lleva a
    /// uno: por su ruta, o por su dirección entera en este servidor (así la
    /// escribe la app de siempre). Su dirección pide sesión, así que quien
    /// pinta no se lo da al navegador: lo baja por su id.
    pub fn linked_attachment(&self, href: &str) -> Option<TaskAttachment> {
        let path = href.strip_prefix(self.core.inner.config.base_url.as_str()).unwrap_or(href);
        let path = attachment_path(path)?;
        let detail = self.state().detail?;
        detail
            .attachments
            .into_iter()
            .chain(detail.comments.into_iter().flat_map(|comment| comment.attachments))
            .find(|file| file.url == path)
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
