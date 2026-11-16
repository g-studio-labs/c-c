//! Los ficheros en el disco de esta máquina: dónde se deja uno para abrirlo
//! y con qué se abre.
//!
//! Qué se baja, de dónde y con qué nombre es seguro escribirlo lo decide el
//! core. Aquí sólo se escribe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use gpui_kit::{App, Global};

/// Con qué se abre un fichero. Existe para las pruebas: sin él se le pide
/// al sistema.
pub struct Opener(pub Arc<dyn Fn(&Path) + Send + Sync>);

impl Global for Opener {}

/// Abre un fichero con el programa del sistema que lo entiende.
pub fn open_with_system(path: &Path, cx: &App) {
    match cx.try_global::<Opener>() {
        Some(opener) => (opener.0)(path),
        None => cx.open_with_system(path),
    }
}

static TURN: AtomicU64 = AtomicU64::new(0);

/// Deja un fichero bajado donde el sistema pueda abrirlo, y dice dónde.
///
/// Cada uno en su carpeta nueva, para que conserve su nombre (es lo que
/// enseña el programa que lo abre) sin pisar a otro que se llame igual. La
/// carpeta es sólo de quien usa la app: el fichero puede ser de un cliente,
/// y la carpeta temporal la comparte toda la máquina.
pub fn write_for_opening(base: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|at| at.as_nanos()).unwrap_or(0);
    let turn = TURN.fetch_add(1, Ordering::Relaxed);
    let dir = base.join(format!("cac-{}-{nanos:x}-{turn}", std::process::id()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    // Sin `recursive`: si ya existe falla, y no se escribe en la carpeta
    // que otro dejó preparada con ese nombre.
    builder.create(&dir)?;
    let path = dir.join(cac_core::safe_file_name(name));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Escribe una copia donde se eligió. Primero a un `.part` y al final se
/// renombra: un corte a mitad no deja algo que parece el fichero y no lo es.
pub fn write_copy(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut partial = target.as_os_str().to_owned();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    let written = std::fs::write(&partial, bytes).and_then(|_| std::fs::rename(&partial, target));
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::{write_copy, write_for_opening};

    /// Una carpeta vacía para esta prueba.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Un fichero bajado queda dentro de su carpeta, con su nombre, entero, y
    /// dos que se llaman igual no se pisan.
    ///
    /// Roturas que atrapa: un nombre con `../` escrito fuera de la carpeta;
    /// el segundo «captura.png» machacando al primero; una carpeta que lee
    /// cualquiera de la máquina.
    #[test]
    fn a_downloaded_file_lands_in_its_own_folder() {
        let base = scratch("opening");
        let first = write_for_opening(&base, "../../captura.png", b"uno").unwrap();
        let second = write_for_opening(&base, "../../captura.png", b"dos").unwrap();

        assert_eq!(first.file_name().unwrap(), "captura.png");
        assert_eq!(std::fs::read(&first).unwrap(), b"uno");
        assert_eq!(std::fs::read(&second).unwrap(), b"dos");
        assert_ne!(first, second);
        for path in [&first, &second] {
            let dir = path.parent().unwrap();
            assert_eq!(dir.parent().unwrap(), &base, "se escribió fuera: {path:?}");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(std::fs::metadata(dir).unwrap().permissions().mode() & 0o777, 0o700);
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Una copia queda entera en su sitio y sin restos; si no se puede
    /// escribir, no queda nada a medias.
    ///
    /// Roturas que atrapa: dejar el `.part` al lado de la copia; un `.part`
    /// huérfano tras un fallo.
    #[test]
    fn a_copy_is_whole_or_is_not_there() {
        let base = scratch("copy");
        let target = base.join("plan.pdf");
        write_copy(&target, b"%PDF").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"%PDF");
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 1, "quedó el .part");

        // El destino es una carpeta: renombrar encima falla.
        let blocked = base.join("ocupado");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("x"), b"x").unwrap();
        assert!(write_copy(&blocked, b"%PDF").is_err());
        assert!(!base.join("ocupado.part").exists(), "quedó un .part huérfano");
        let _ = std::fs::remove_dir_all(&base);
    }
}
