//! El compresor de imágenes de las DevTools: lo mismo que `compress_image` del
//! MCP (`cac_mcp::image`), con lo que la pantalla necesita encima —el nombre
//! del resultado y cuánto cambió—.

use std::path::Path;

pub use cac_mcp::image::OutputFormat;
use cac_mcp::image::{compress, CompressOptions};

/// Los formatos de salida, en el orden de la app de Tauri.
pub const OUTPUT_FORMATS: [(&str, &str); 6] = [("webp", "WebP"), ("jpeg", "JPEG"), ("png", "PNG"), ("avif", "AVIF"), ("gif", "GIF"), ("bmp", "BMP")];

/// Una imagen ya comprimida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compressed {
    /// El nombre del original con la extensión nueva.
    pub name: String,
    pub data: Vec<u8>,
    /// La extensión (`webp`, `jpg`…).
    pub format: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub original_bytes: usize,
    pub compressed_bytes: usize,
}

/// `format` es uno de [`OUTPUT_FORMATS`]. Un ancho máximo más grande que la
/// imagen no la agranda: sólo se reduce.
pub fn compress_bytes(name: &str, raw: &[u8], format: &str, quality: u8, max_width: Option<u32>) -> Result<Compressed, String> {
    let format: OutputFormat = serde_json::from_value(serde_json::Value::String(format.to_string())).map_err(|error| format!("Invalid format: {error}"))?;
    let result = compress(raw, &CompressOptions { quality: Some(quality), max_width, format })?;
    let stem = Path::new(name).file_stem().and_then(|stem| stem.to_str()).filter(|stem| !stem.is_empty()).unwrap_or("image");
    Ok(Compressed {
        name: format!("{stem}.{}", result.format),
        data: result.data,
        format: result.format,
        mime: result.mime,
        width: result.width,
        height: result.height,
        original_bytes: result.original_bytes,
        compressed_bytes: result.compressed_bytes,
    })
}

/// Lee un fichero y lo comprime.
pub fn compress_file(path: &Path, format: &str, quality: u8, max_width: Option<u32>) -> Result<Compressed, String> {
    let raw = std::fs::read(path).map_err(|error| error.to_string())?;
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("image");
    compress_bytes(name, &raw, format, quality, max_width).map_err(|error| format!("{name}: {error}"))
}

/// Cuánto cambió, con su signo: `-12.0%` si salió más pequeño, `+12.0%` si
/// salió más grande. Comprimir y recibir algo mayor es justo lo que la
/// herramienta tiene que decir sin rodeos; en Tauri salía «+-12.0%».
pub fn size_change(original: usize, compressed: usize) -> String {
    if original == 0 {
        return "0%".into();
    }
    let percent = ((1. - compressed as f64 / original as f64) * 100.).abs();
    let sign = if compressed <= original { "-" } else { "+" };
    format!("{sign}{percent:.1}%")
}

/// Como lo dice la app de Tauri: B, KB o MB.
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024. * 1024.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAY: &[u8] = include_bytes!("../../core/tests/fixtures/gray_600x200.png");

    /// Rotura que atrapa: que un ancho máximo mayor que la imagen la estire
    /// (el compresor no agranda una imagen), o que al reducirla se pierda la
    /// proporción.
    #[test]
    fn the_compressor_never_enlarges_an_image() {
        let kept = compress_bytes("foto.png", GRAY, "png", 80, Some(1200)).unwrap();
        assert_eq!((kept.width, kept.height), (600, 200));
        let smaller = compress_bytes("foto.png", GRAY, "png", 80, Some(300)).unwrap();
        assert_eq!((smaller.width, smaller.height), (300, 100));
        assert_eq!(smaller.name, "foto.png");
        let webp = compress_bytes("vacaciones.final.PNG", GRAY, "webp", 80, None).unwrap();
        assert_eq!((webp.name.as_str(), webp.mime.as_str()), ("vacaciones.final.webp", "image/webp"));
        assert_eq!(webp.original_bytes, GRAY.len());
        assert!(compress_bytes("x.png", b"no es una imagen", "png", 80, None).is_err());
    }

    /// Rotura que atrapa: pintar «-» delante de un fichero que creció (o
    /// «+-», como en Tauri).
    #[test]
    fn a_result_that_grew_says_so() {
        assert_eq!(size_change(1000, 880), "-12.0%");
        assert_eq!(size_change(1000, 1120), "+12.0%");
        assert_eq!(size_change(0, 10), "0%");
        assert_eq!(format_size(2048), "2.0 KB");
    }
}
