//! Los ficheros en el disco de esta máquina: dónde se deja uno para abrirlo
//! y con qué se abre.
//!
//! Qué se baja, de dónde y con qué nombre es seguro escribirlo lo decide el
//! core. Aquí sólo se escribe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cac_core::{ApiError, Locale};
use cac_mcp::host::{NotOpened, Opened, RevealError};
use gpui_kit::{Context, Global};

use crate::lang::locale;

/// Con qué se abre un fichero. Existe para las pruebas: sin él se lanza de
/// verdad, por `cac_mcp::host`.
pub struct Opener(pub Arc<dyn Fn(&Path) + Send + Sync>);

impl Global for Opener {}

/// Abre un fichero o una carpeta con lo que el sistema asocia, por las reglas
/// de `cac_mcp::host::open_path`: un documento con su programa, una carpeta en
/// el gestor de ficheros, y lo ejecutable o dudoso sólo se muestra. Si no se
/// pudo, `say` recibe la línea que lo explica.
///
/// Nunca `cx.open_with_system` de GPUI. En Linux lanza `xdg-open` con el
/// entorno heredado entero (sólo le añade las variables gráficas y no mira el
/// resultado de `status()`): dentro de la AppImage `xdg-open` es un script de
/// shell que carga las bibliotecas de la imagen en vez de las del sistema y
/// muere sin que nadie lo vea, y con el portal gtk de bspwm no hay otro
/// camino que lo recoja. `host` limpia ese entorno (ver CLAUDE.md, «Lo que la
/// app lanza del sistema hereda la AppImage») y dice por qué falla.
/// Guardián: `no_gpui_open_outside_files`.
pub fn open_with_system<V: 'static>(path: &Path, cx: &mut Context<V>, say: impl Fn(&mut V, String, &mut Context<V>) + 'static) {
    launch(path, cx, say, |path| cac_mcp::host::open_path(path));
}

/// Lo mismo, pero mostrando el fichero seleccionado en su carpeta, sin abrirlo
/// (`cac_mcp::host::reveal`). Una carpeta se abre.
pub fn reveal_in_folder<V: 'static>(path: &Path, cx: &mut Context<V>, say: impl Fn(&mut V, String, &mut Context<V>) + 'static) {
    launch(path, cx, say, |path| cac_mcp::host::reveal(path).map(|()| Opened::Revealed(None)));
}

pub(crate) fn launch<V: 'static>(
    path: &Path,
    cx: &mut Context<V>,
    say: impl Fn(&mut V, String, &mut Context<V>) + 'static,
    run: fn(&str) -> Result<Opened, RevealError>,
) {
    if let Some(opener) = cx.try_global::<Opener>().map(|opener| opener.0.clone()) {
        opener(path);
        return;
    }
    let shown = path.to_string_lossy().into_owned();
    cx.spawn(async move |this, cx| {
        let target = shown.clone();
        let result = cx.background_executor().spawn(async move { run(&target) }).await;
        this.update(cx, |this, cx| {
            if let Some(line) = failure_line(&result, &shown, locale(cx)) {
                say(this, line, cx);
            }
        })
        .ok();
    })
    .detach();
}

/// Lo que se le dice a quien pulsó, o nada si se abrió. Un «no» nunca es mudo:
/// ni la ruta que no existe, ni el documento que por dentro es otra cosa, ni el
/// gestor que no arrancó.
pub fn failure_line(result: &Result<Opened, RevealError>, shown: &str, language: Locale) -> Option<String> {
    let name = Path::new(shown).file_name().map_or_else(|| shown.to_string(), |name| name.to_string_lossy().into_owned());
    match result {
        Ok(Opened::WithItsProgram | Opened::Revealed(None)) => None,
        Ok(Opened::Revealed(Some(NotOpened::ContentType(mime)))) => Some(cac_core::text_with(language, "work:requests.revealedByContent", &[("name", &name), ("type", mime)])),
        Ok(Opened::Revealed(Some(NotOpened::TypeUnknown))) => Some(cac_core::text_with(language, "work:requests.revealedTypeUnknown", &[("name", &name)])),
        Err(RevealError::Failed(_)) => Some(cac_core::text(language, "common:last.openLinkFailed")),
        Err(_) => Some(cac_core::text_with(language, "work:requests.pathMissing", &[("path", shown)])),
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
    use super::{failure_line, fetch_for_opening};
    use cac_core::{ApiError, Locale};
    use cac_mcp::host::{NotOpened, Opened, RevealError};

    /// Las fuentes de la app, sin lo que va tras `#[cfg(test)]` ni los
    /// comentarios.
    fn code_lines() -> Vec<(String, usize, String)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<(String, usize, String)>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let code = text.split("#[cfg(test)]").next().unwrap_or("");
                    for (i, line) in code.lines().enumerate().filter(|(_, line)| !line.trim_start().starts_with("//")) {
                        out.push((path.display().to_string(), i + 1, line.to_string()));
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
        assert!(!out.is_empty(), "no hay fuentes que mirar");
        out
    }

    /// Nadie le pide a GPUI que abra algo (`cx.open_with_system`), ni el
    /// propio `files.rs`: en Linux lanza `xdg-open` con el entorno de la
    /// AppImage y sin mirar si falló, y no abre nada sin decirlo. Todo abrir
    /// fichero o carpeta pasa por `files::open_with_system` /
    /// `reveal_in_folder`, que van por `cac_mcp::host`.
    ///
    /// Rotura que atrapa: volver a `cx.open_with_system(...)` en cualquier
    /// fichero de la app.
    #[test]
    fn no_gpui_open_outside_host() {
        let bad: Vec<String> = code_lines().into_iter().filter(|(_, _, line)| line.contains(".open_with_system(")).map(|(file, n, line)| format!("{file}:{n}: {}", line.trim())).collect();
        assert!(bad.is_empty(), "abren con GPUI en vez de por host: {bad:#?}");
    }

    /// Un abrir que no abrió dice por qué, y el que abrió no dice nada.
    ///
    /// Roturas que atrapa: un fallo del gestor sin línea (el «no hace nada»
    /// mudo); dar la ruta que no existe por abierta; no avisar de un
    /// documento que sólo se mostró.
    #[test]
    fn a_failed_open_says_why_and_a_good_one_says_nothing() {
        let es = Locale::Es;
        assert_eq!(failure_line(&Ok(Opened::WithItsProgram), "/a/b.pdf", es), None);
        assert_eq!(failure_line(&Ok(Opened::Revealed(None)), "/a", es), None);
        let failed = failure_line(&Err(RevealError::Failed("xdg-open".into())), "/a", es).unwrap();
        assert!(failed.contains("No se pudo abrir"), "{failed}");
        let missing = failure_line(&Err(RevealError::NotFound), "/a/b.pdf", es).unwrap();
        assert!(missing.contains("/a/b.pdf"), "{missing}");
        let kind = failure_line(&Ok(Opened::Revealed(Some(NotOpened::ContentType("application/x-sh".into())))), "/a/notas.txt", es).unwrap();
        assert!(kind.contains("notas.txt") && kind.contains("application/x-sh"), "{kind}");
    }

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
