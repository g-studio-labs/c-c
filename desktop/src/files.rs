//! Los ficheros en el disco de esta máquina: dónde se deja uno para abrirlo
//! y con qué se abre.
//!
//! Qué se baja, de dónde y con qué nombre es seguro escribirlo lo decide el
//! core. Aquí sólo se escribe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cac_core::ApiError;
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

/// Baja un fichero con `save` a donde el sistema pueda abrirlo, y dice
/// dónde. `save` lo escribe según llega (`download_to` del core), así que no
/// pasa entero por la memoria.
///
/// Cada uno en su carpeta nueva, para que conserve su nombre (es lo que
/// enseña el programa que lo abre) sin pisar a otro que se llame igual. La
/// carpeta es sólo de quien usa la app: el fichero puede ser de un cliente,
/// y la carpeta temporal la comparte toda la máquina. Si no llega, la
/// carpeta se va con él.
pub async fn fetch_for_opening<F, Fut>(base: &Path, name: &str, save: F) -> Result<PathBuf, Option<ApiError>>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<(), ApiError>>,
{
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|at| at.as_nanos()).unwrap_or(0);
    let turn = TURN.fetch_add(1, Ordering::Relaxed);
    let dir = base.join(format!("cac-{}-{nanos:x}-{turn}", std::process::id()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    // Sin `recursive`: si ya existe falla, y no se escribe en la carpeta
    // que otro dejó preparada con ese nombre.
    builder.create(&dir).map_err(|_| None)?;
    let path = dir.join(cac_core::safe_file_name(name));
    if let Err(error) = save(path.clone()).await {
        let _ = std::fs::remove_dir(&dir);
        return Err(Some(error));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::fetch_for_opening;
    use cac_core::ApiError;

    /// Una carpeta vacía para esta prueba.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fetch(base: &std::path::Path, name: &str, bytes: &'static [u8]) -> Result<std::path::PathBuf, Option<ApiError>> {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(fetch_for_opening(base, name, |target| async move { std::fs::write(target, bytes).map_err(|e| ApiError::Storage(e.to_string())) }))
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
        let first = fetch(&base, "../../captura.png", b"uno").unwrap();
        let second = fetch(&base, "../../captura.png", b"dos").unwrap();

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

    /// Un fichero que no llega no deja su carpeta vacía en la temporal, y el
    /// fallo vuelve con lo que dijo quien bajaba.
    ///
    /// Roturas que atrapa: una carpeta huérfana por cada fichero que no se
    /// pudo abrir; dar por bajado uno que no llegó; perder por qué falló.
    #[test]
    fn a_file_that_does_not_arrive_leaves_no_folder() {
        let base = scratch("failed");
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let failed = runtime.block_on(fetch_for_opening(&base, "plan.pdf", |_| async { Err(ApiError::Network("cut".into())) }));

        assert!(matches!(failed, Err(Some(ApiError::Network(_)))), "{failed:?}");
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 0, "se quedó la carpeta");
        let _ = std::fs::remove_dir_all(&base);
    }
}
