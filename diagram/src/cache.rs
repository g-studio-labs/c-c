//! Lo ya dibujado, en disco: un diagrama visto una vez se sigue viendo sin
//! red y sin volver a dibujarlo.
//!
//! Un fichero `<clave>.png` por diagrama, donde la clave es el sha256 de la
//! versión de `merman`, el tema, la escala y el texto sin directivas. El PNG
//! es él mismo el registro: su tamaño en píxeles sale de su cabecera, y la
//! escala va en la clave. Los errores no se guardan: un `merman` nuevo puede
//! dibujar lo que éste no.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::{
    strip_directives, DiagramError, Draw, Picture, Theme, MAX_SIDE, MAX_SOURCE_BYTES,
    MERMAN_VERSION,
};

/// Cuántos diagramas se guardan. Al pasar, se borran los menos usados.
pub const CACHE_LIMIT: usize = 300;

pub struct Cache {
    dir: PathBuf,
}

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

impl Cache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Cache { dir: dir.into() }
    }

    /// El diagrama de la caché o, si no está, dibujado con `painter` y
    /// guardado. Que no se pueda escribir en disco no impide enseñarlo.
    pub fn get_or_draw(
        &self,
        painter: &impl Draw,
        source: &str,
        theme: Theme,
        scale: f32,
    ) -> Result<Picture, DiagramError> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(DiagramError::TooLong {
                bytes: source.len(),
            });
        }
        let path = self.dir.join(format!("{}.png", key(source, theme, scale)));
        if let Some(picture) = read(&path, scale) {
            return Ok(picture);
        }
        let picture = painter.draw(source, theme, scale)?;
        let _ = self.store(&path, &picture.png);
        Ok(picture)
    }

    /// Se escribe a un temporal con nombre propio y se renombra: dos hilos
    /// (o dos ventanas) que dibujan lo mismo a la vez no dejan nunca un PNG a
    /// medias, y el que llega segundo reemplaza uno idéntico. Que no se
    /// escriba directo sobre el PNG no lo caza ninguna prueba: un lector a
    /// media escritura es cuestión de tiempos, y no se cronometra.
    fn store(&self, path: &Path, png: &[u8]) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let temp = self.temp_path();
        let written = fs::File::create(&temp).and_then(|mut file| {
            file.write_all(png)?;
            file.sync_all()
        });
        if let Err(error) = written.and_then(|_| fs::rename(&temp, path)) {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        self.purge();
        Ok(())
    }

    pub(crate) fn temp_path(&self) -> PathBuf {
        self.dir.join(format!(
            ".{}-{}.tmp",
            std::process::id(),
            TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn purge(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut pictures: Vec<(SystemTime, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        if pictures.len() <= CACHE_LIMIT {
            return;
        }
        pictures.sort();
        let extra = pictures.len() - CACHE_LIMIT;
        // Otro proceso puede estar purgando a la vez: lo que ya no está, no
        // es un error.
        for (_, path) in pictures.into_iter().take(extra) {
            let _ = fs::remove_file(path);
        }
    }
}

fn key(source: &str, theme: Theme, scale: f32) -> String {
    let mut hash = Sha256::new();
    for part in [
        MERMAN_VERSION,
        theme.key(),
        &scale.to_bits().to_string(),
        &strip_directives(source),
    ] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Un PNG de la caché, o nada si no está o no se entiende (se vuelve a
/// dibujar). Al leerlo se le pone la hora: la purga borra los menos usados,
/// no los más viejos de escribir.
fn read(path: &Path, scale: f32) -> Option<Picture> {
    let png = fs::read(path).ok()?;
    let (pixel_width, pixel_height) = png_size(&png)?;
    if let Ok(file) = fs::File::options().append(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
    Some(Picture {
        png,
        pixel_width,
        pixel_height,
        scale,
    })
}

fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < 24 || &png[..8] != SIGNATURE || &png[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(png[20..24].try_into().ok()?);
    // Un fichero de la caché que diga medir más de lo que aquí se dibuja no
    // es nuestro: decodificarlo sería reservar lo que él quiera.
    let fits = |side: u32| (1..=MAX_SIDE).contains(&side);
    (fits(width) && fits(height)).then_some((width, height))
}
