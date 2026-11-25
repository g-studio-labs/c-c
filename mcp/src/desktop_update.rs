//! Actualizar la app de escritorio nativa (`desktop/`).
//!
//! La misma mecánica que `cac-mcp` (`update.rs`: versión de un `VERSION` en
//! `main`, descarga, SHA-256 contra `SHA256SUMS`, renombrar en vez de escribir
//! encima), con su propia etiqueta, `desktop-vX.Y.Z`, que **nunca** se marca
//! «latest» (guardián: `tests/desktop_release.rs`): el actualizador de la app
//! de Tauri lee `releases/latest/download/latest.json`.
//!
//! Qué se baja depende del sistema, y es lo que reparte `desktop-release.yml`:
//!
//! - Linux: el binario suelto, que se pone donde está el que corre. Si esa
//!   carpeta no es de la persona (un `.deb` en `/usr/bin`) no se toca nada:
//!   el gestor de paquetes es quien actualiza eso, y se le enseña la página.
//! - Windows: el instalador de NSIS, que se lanza; la app tiene que cerrarse
//!   para que pueda sustituir el `.exe`.
//! - macOS: el `.dmg`, que se abre; arrastrar a Aplicaciones lo hace la persona.
//!
//! Nada se instala solo: esto ofrece, y quien lo llama espera a un clic.
//!
//! Todo es síncrono (ver `update.rs`): en un hilo aparte, nunca en el que pinta.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::update::{self, Progress, Source};

/// El fichero de `main` que dice cuál es la última versión de la app nativa.
pub const VERSION_URL: &str = "https://raw.githubusercontent.com/g-studio-labs/c-c/main/desktop/VERSION";
/// De dónde se bajan los ficheros: el mismo repo que el MCP.
pub const DOWNLOAD_BASE: &str = update::DOWNLOAD_BASE;
/// La página de una release, para quien no se puede actualizar solo.
pub const RELEASES_PAGE: &str = "https://github.com/g-studio-labs/c-c/releases/tag";
/// El nombre del binario, el de `desktop/packaging/identity.env`.
pub const APP_BINARY: &str = "cac-desktop";

/// No preguntar por versiones nuevas (cualquier valor que no sea vacío ni `0`).
pub const ENV_NO_CHECK: &str = "CAC_DESKTOP_NO_UPDATE_CHECK";
pub const ENV_VERSION_URL: &str = "CAC_DESKTOP_VERSION_URL";
pub const ENV_DOWNLOAD_BASE: &str = "CAC_DESKTOP_DOWNLOAD_BASE";
pub const ENV_STATE_DIR: &str = "CAC_DESKTOP_STATE_DIR";

/// Cada cuánto se vuelve a preguntar con la app abierta.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// La etiqueta de la release de una versión.
pub fn tag(version: &str) -> String {
    format!("desktop-v{version}")
}

/// La página de la release de una versión.
pub fn release_page(version: &str) -> String {
    format!("{RELEASES_PAGE}/{}", tag(version))
}

/// Cómo se le entrega una versión a esta máquina.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Package {
    /// El binario suelto, que sustituye al que corre.
    Binary,
    /// El instalador de Windows.
    Installer,
    /// La imagen de disco de macOS.
    Disk,
}

/// El fichero de la release para un sistema y una arquitectura, con los
/// nombres de `std::env::consts`. `None`: para eso no se publica nada.
///
/// Los nombres los fabrican `desktop/packaging/linux/package.sh`, `macos/
/// package.sh` y `windows/package.ps1`, con la versión de `Cargo.toml`;
/// `tests/desktop_release.rs` comprueba que dicen lo mismo que esto.
pub fn asset_name(version: &str, os: &str, arch: &str) -> Option<(String, Package)> {
    let (suffix, package) = match (os, arch) {
        ("linux", "x86_64") => ("linux-x86_64", Package::Binary),
        ("macos", "aarch64") => ("macos-arm64.dmg", Package::Disk),
        ("windows", "x86_64") => ("windows-x86_64-setup.exe", Package::Installer),
        _ => return None,
    };
    Some((format!("{APP_BINARY}-{version}-{suffix}"), package))
}

/// Lo que se hace con la versión nueva en esta máquina.
#[derive(Clone, Debug, PartialEq)]
pub enum Delivery {
    /// Bajarla y ponerla donde está el binario que corre.
    Replace { exe: PathBuf },
    /// Bajarla y lanzar el instalador.
    RunInstaller,
    /// Bajarla y abrir la imagen de disco.
    OpenDisk,
    /// No se puede aquí: enseñar la página de la release.
    Manual,
}

/// Qué entrega toca. `replaceable`: si la carpeta del binario que corre es de
/// quien lo corre (para el binario suelto).
pub fn delivery(package: Package, exe: Option<&Path>, replaceable: bool) -> Delivery {
    match (package, exe) {
        (Package::Binary, Some(exe)) if replaceable => Delivery::Replace { exe: exe.to_path_buf() },
        (Package::Binary, _) => Delivery::Manual,
        (Package::Installer, _) => Delivery::RunInstaller,
        (Package::Disk, _) => Delivery::OpenDisk,
    }
}

/// Un binario que vive dentro de un `target/` de cargo es una compilación de
/// desarrollo: sustituirlo por el publicado deshace lo que se está probando.
fn is_dev_build(exe: &Path) -> bool {
    exe.components().any(|part| part.as_os_str() == "target")
}

/// El prefijo de la carpeta donde se deja lo que se baja y no es el binario.
pub const DOWNLOAD_DIR_PREFIX: &str = "cac-desktop-update-";

/// Si se puede crear un fichero junto a `exe`: lo que hace falta para
/// sustituirlo por renombrado.
fn folder_is_writable(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else { return false };
    let probe = dir.join(format!(".{APP_BINARY}.probe-{}", std::process::id()));
    let made = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    made
}

/// Una versión nueva, y qué se hace con ella.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub version: String,
    pub asset: String,
    pub delivery: Delivery,
}

/// Lo que dejó una instalación.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// El binario nuevo ya está en su sitio; falta reiniciar.
    Replaced { exe: PathBuf },
    /// El instalador o la imagen están bajados y comprobados; falta lanzarlos
    /// (`Outcome::launch`).
    Downloaded { path: PathBuf, delivery: Delivery },
}

impl Outcome {
    /// Lanza el instalador o abre la imagen. Por `host`, nunca a pelo: dentro de
    /// una AppImage el programa heredaría su entorno.
    pub fn launch(&self) -> Result<(), String> {
        let Outcome::Downloaded { path, delivery } = self else { return Ok(()) };
        let mut command = match delivery {
            Delivery::RunInstaller => crate::host::command(path),
            Delivery::OpenDisk => {
                let mut command = crate::host::command("open");
                command.arg(path);
                command
            }
            _ => return Ok(()),
        };
        command.stdin(std::process::Stdio::null()).spawn().map(|_| ()).map_err(|e| format!("could not open {}: {e}", path.display()))
    }
}

/// Esta máquina: de dónde se baja, qué versión es la que corre y dónde está
/// su binario.
#[derive(Clone, Debug, PartialEq)]
pub struct Updater {
    pub source: Source,
    pub state_dir: Option<PathBuf>,
    /// La versión que corre.
    pub current: String,
    /// El binario que corre.
    pub exe: Option<PathBuf>,
    pub os: &'static str,
    pub arch: &'static str,
    /// Dónde se deja lo que se baja y no es el binario.
    pub download_dir: PathBuf,
}

impl Updater {
    /// Ésta, con lo que diga el entorno. `current` es la versión del binario
    /// que llama (`env!("CARGO_PKG_VERSION")` de la app).
    pub fn from_env(current: &str) -> Self {
        let or = |name: &str, default: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default.into());
        Updater {
            source: Source { version_url: or(ENV_VERSION_URL, VERSION_URL), download_base: or(ENV_DOWNLOAD_BASE, DOWNLOAD_BASE) },
            state_dir: update::state_dir_for("cac-desktop", ENV_STATE_DIR),
            current: current.to_string(),
            // Se lee ahora: tras sustituirlo, Linux lo cuenta como «(deleted)».
            exe: std::env::current_exe().ok(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            download_dir: std::env::temp_dir().join(format!("{DOWNLOAD_DIR_PREFIX}{}", std::process::id())),
        }
    }

    /// Si se pidió no preguntar por versiones nuevas.
    pub fn checks_disabled() -> bool {
        std::env::var(ENV_NO_CHECK).is_ok_and(|v| !v.trim().is_empty() && v.trim() != "0")
    }

    /// Si hay una versión más nueva que la que corre. Pregunta a la red sólo
    /// si lo sabido tiene más de `max_age` segundos; sin red, devuelve lo que
    /// se supiera. Bloquea.
    pub fn check(&self, max_age: Duration) -> Option<Offer> {
        let latest = match &self.state_dir {
            Some(dir) => update::latest_known_within(&self.source, dir, update::now(), max_age.as_secs()),
            None => update::check_now(&self.source, None, Duration::from_secs(8)).ok(),
        }?;
        self.offer(&latest).filter(|offer| self.is_published(offer))
    }

    /// Si la release de esa versión ya tiene su fichero y su suma. `VERSION` se
    /// sube a `main` antes de que el workflow publique, y durante la
    /// compilación (o para siempre, si un sistema falla) ofrecer esa versión
    /// acabaría en un 404. Sin respuesta tampoco se ofrece: sin red no se
    /// puede bajar nada de todos modos.
    fn is_published(&self, offer: &Offer) -> bool {
        let url = self.source.asset_url_tagged(&tag(&offer.version), update::CHECKSUMS);
        update::fetch_text(&url, Duration::from_secs(8)).is_ok_and(|sums| update::checksum_for(&sums, &offer.asset).is_some())
    }

    /// Borra lo que dejaron las descargas de otras ejecuciones (el instalador
    /// que se lanzó y ya no hace falta), si tiene más de `older_than`.
    pub fn clean_leftovers(&self, older_than: Duration) {
        let Some(parent) = self.download_dir.parent() else { return };
        let Ok(entries) = std::fs::read_dir(parent) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let ours = entry.file_name().to_string_lossy().starts_with(DOWNLOAD_DIR_PREFIX);
            let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|at| at.elapsed().ok()).is_some_and(|age| age >= older_than);
            if ours && old && path != self.download_dir && path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }

    /// La oferta de esa versión para esta máquina, si es más nueva y hay algo
    /// publicado para ella.
    pub fn offer(&self, latest: &str) -> Option<Offer> {
        if !update::is_newer(latest, &self.current) || self.exe.as_deref().is_some_and(is_dev_build) {
            return None;
        }
        let (asset, package) = asset_name(latest, self.os, self.arch)?;
        let replaceable = self.exe.as_deref().is_some_and(folder_is_writable);
        Some(Offer { version: latest.to_string(), asset, delivery: delivery(package, self.exe.as_deref(), replaceable) })
    }

    /// Baja la versión, comprueba su suma, y la deja lista: el binario en su
    /// sitio, o el instalador bajado. Si algo falla, lo que había sigue igual.
    /// Bloquea.
    pub fn install(&self, offer: &Offer, progress: &mut dyn FnMut(Progress)) -> Result<Outcome, String> {
        if offer.delivery == Delivery::Manual {
            return Err("this install cannot update itself; download the new version from its release page".into());
        }
        let bytes = update::download_verified_tagged(&self.source, &tag(&offer.version), &offer.version, &offer.asset, progress)?;
        progress(Progress::Installing);
        match &offer.delivery {
            Delivery::Replace { exe } => {
                update::replace(exe, &bytes)?;
                Ok(Outcome::Replaced { exe: exe.clone() })
            }
            delivery => {
                let path = self.download_dir.join(&offer.asset);
                update::replace(&path, &bytes)?;
                Ok(Outcome::Downloaded { path, delivery: delivery.clone() })
            }
        }
    }
}

/// Arranca otra vez la app, con los mismos argumentos, por `host`.
pub fn relaunch(exe: &Path) -> Result<(), String> {
    crate::host::command(exe).args(std::env::args_os().skip(1)).stdin(std::process::Stdio::null()).spawn().map(|_| ()).map_err(|e| format!("could not start {}: {e}", exe.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::fake::FakeReleases;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-desktop-update-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const OLD: &[u8] = b"the running binary";

    /// Una máquina de Linux con su binario instalado en una carpeta propia.
    fn machine(releases: &FakeReleases, dir: &Path, current: &str) -> Updater {
        let exe = dir.join("bin").join(APP_BINARY);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, OLD).unwrap();
        Updater {
            source: releases.desktop_source(),
            state_dir: Some(dir.join("state")),
            current: current.into(),
            exe: Some(exe),
            os: "linux",
            arch: "x86_64",
            download_dir: dir.join("downloads"),
        }
    }

    fn publish_linux(releases: &FakeReleases, version: &str, body: &[u8]) {
        let (name, _) = asset_name(version, "linux", "x86_64").unwrap();
        releases.announce_desktop(version);
        releases.publish_desktop(version, &[(&name, body)]);
    }

    /// Una versión más nueva se ofrece; la misma, una más vieja, o una
    /// respuesta que no es una versión, no. Roturas que atrapa: comparar
    /// versiones como texto; ofrecer la que ya corre (avisaría para siempre);
    /// leer el `VERSION` del MCP en vez del de la app.
    #[test]
    fn a_newer_version_is_offered_and_the_same_one_is_not() {
        let releases = FakeReleases::start();
        let dir = scratch("offer");
        releases.announce("999.0.0");
        publish_linux(&releases, "0.10.0", b"new");

        let app = machine(&releases, &dir, "0.9.0");
        let offer = app.check(Duration::ZERO).expect("0.10.0 is newer than 0.9.0");
        assert_eq!(offer.version, "0.10.0");
        assert_eq!(offer.asset, "cac-desktop-0.10.0-linux-x86_64");
        assert_eq!(offer.delivery, Delivery::Replace { exe: dir.join("bin").join(APP_BINARY) });

        assert_eq!(machine(&releases, &dir, "0.10.0").check(Duration::ZERO), None, "the same version");
        assert_eq!(machine(&releases, &dir, "0.11.0").check(Duration::ZERO), None, "an older one");
        releases.put("/desktop/VERSION", "<html>log in to the wifi</html>");
        let fresh = scratch("offer-portal");
        assert_eq!(machine(&releases, &fresh, "0.9.0").check(Duration::ZERO), None, "a captive portal is not a version");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(fresh);
    }

    /// Sin red se ofrece lo que se supo la última vez, y con lo sabido
    /// reciente no se vuelve a preguntar. Roturas que atrapa: preguntar en cada
    /// arranque; olvidar lo sabido cuando la red falla.
    #[test]
    fn what_was_learned_is_remembered_and_asked_once_a_day() {
        let releases = FakeReleases::start();
        let dir = scratch("daily");
        publish_linux(&releases, "0.10.0", b"new");
        let app = machine(&releases, &dir, "0.9.0");

        assert!(app.check(CHECK_EVERY).is_some());
        let versions = || releases.asked().iter().filter(|path| path.as_str() == "/desktop/VERSION").count();
        let asked = versions();
        assert!(app.check(CHECK_EVERY).is_some());
        assert_eq!(versions(), asked, "asked again within the day");

        let offline = Updater { source: Source { version_url: "http://127.0.0.1:9/VERSION".into(), ..app.source.clone() }, ..app.clone() };
        assert!(offline.check(Duration::ZERO).is_some(), "when VERSION does not answer, what was known is still offered");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Instalar pone el binario nuevo, entero y ejecutable, en el sitio del que
    /// corre. Roturas que atrapa: escribir encima en vez de renombrar; bajar de
    /// otra etiqueta; dejar el binario sin permiso de ejecución.
    #[test]
    fn installing_puts_the_verified_binary_where_the_running_one_is() {
        let releases = FakeReleases::start();
        let dir = scratch("replace");
        publish_linux(&releases, "0.10.0", b"the new binary");
        let app = machine(&releases, &dir, "0.9.0");
        let offer = app.check(Duration::ZERO).unwrap();

        let mut steps = Vec::new();
        let outcome = app.install(&offer, &mut |step| steps.push(step)).unwrap();
        let exe = dir.join("bin").join(APP_BINARY);
        assert_eq!(outcome, Outcome::Replaced { exe: exe.clone() });
        assert_eq!(std::fs::read(&exe).unwrap(), b"the new binary");
        assert!(steps.contains(&Progress::Verifying) && steps.last() == Some(&Progress::Installing), "{steps:?}");
        assert!(releases.asked().iter().all(|path| !path.contains("mcp-v")), "{:?}", releases.asked());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o111, 0o111);
        }
        let leftovers: Vec<_> = std::fs::read_dir(exe.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(leftovers, [std::ffi::OsString::from(APP_BINARY)], "a staged file was left behind");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Lo que se baja y no es lo publicado se rechaza, y lo instalado sigue
    /// como estaba. Roturas que atrapa: no comparar la suma; sustituir antes de
    /// comprobar.
    #[test]
    fn a_download_that_is_not_what_was_published_is_rejected_and_the_old_binary_stays() {
        let releases = FakeReleases::start();
        let dir = scratch("mismatch");
        publish_linux(&releases, "0.10.0", b"the new binary");
        let (name, _) = asset_name("0.10.0", "linux", "x86_64").unwrap();
        releases.put(&format!("/download/desktop-v0.10.0/{name}"), "tampered");
        let app = machine(&releases, &dir, "0.9.0");
        let offer = app.check(Duration::ZERO).unwrap();

        let error = app.install(&offer, &mut |_| {}).unwrap_err();
        assert!(error.contains("checksum mismatch"), "{error}");
        assert_eq!(std::fs::read(dir.join("bin").join(APP_BINARY)).unwrap(), OLD);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Una descarga que falla (la release no tiene el fichero, o no tiene
    /// sumas) deja lo instalado como estaba. Roturas que atrapa: borrar o
    /// vaciar el binario antes de tener el nuevo; instalar sin suma publicada.
    #[test]
    fn a_failed_download_keeps_the_old_binary() {
        let releases = FakeReleases::start();
        let dir = scratch("failed");
        publish_linux(&releases, "0.10.0", b"the new binary");
        let app = machine(&releases, &dir, "0.9.0");
        let offer = app.check(Duration::ZERO).unwrap();
        let exe = dir.join("bin").join(APP_BINARY);

        releases.remove(&format!("/download/desktop-v0.10.0/{}", offer.asset));
        let error = app.install(&offer, &mut |_| {}).unwrap_err();
        assert!(error.contains("404"), "{error}");
        assert_eq!(std::fs::read(&exe).unwrap(), OLD);

        publish_linux(&releases, "0.10.0", b"the new binary");
        releases.remove("/download/desktop-v0.10.0/SHA256SUMS");
        assert!(app.install(&offer, &mut |_| {}).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), OLD, "installed with no published checksum");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Cada sistema recibe lo suyo. Roturas que atrapa: ofrecer a Windows el
    /// binario de Linux; sustituir el binario de una instalación de paquete
    /// (carpeta que no es de la persona); ofrecer algo donde no se publica.
    #[test]
    fn each_system_gets_its_own_package_and_a_package_install_is_left_alone() {
        let exe = Some(Path::new("/usr/bin/cac-desktop"));
        assert_eq!(delivery(Package::Binary, exe, true), Delivery::Replace { exe: "/usr/bin/cac-desktop".into() });
        assert_eq!(delivery(Package::Binary, exe, false), Delivery::Manual, "a /usr/bin install is the package manager's");
        assert_eq!(delivery(Package::Binary, None, true), Delivery::Manual);
        assert_eq!(delivery(Package::Installer, exe, false), Delivery::RunInstaller);
        assert_eq!(delivery(Package::Disk, None, false), Delivery::OpenDisk);

        assert_eq!(asset_name("1.2.3", "windows", "x86_64"), Some(("cac-desktop-1.2.3-windows-x86_64-setup.exe".into(), Package::Installer)));
        assert_eq!(asset_name("1.2.3", "macos", "aarch64"), Some(("cac-desktop-1.2.3-macos-arm64.dmg".into(), Package::Disk)));
        assert_eq!(asset_name("1.2.3", "linux", "x86_64"), Some(("cac-desktop-1.2.3-linux-x86_64".into(), Package::Binary)));
        assert_eq!(asset_name("1.2.3", "linux", "aarch64"), None);
        assert_eq!(asset_name("1.2.3", "macos", "x86_64"), None);
    }

    /// En Windows y en macOS se baja el instalador o la imagen, comprobados, y
    /// no se toca el binario que corre; lanzarlos es aparte. Roturas que
    /// atrapa: sustituir el `.exe` en marcha; lanzar lo que no pasó la suma.
    #[test]
    fn the_installer_and_the_disk_image_are_downloaded_verified_and_not_launched_here() {
        let releases = FakeReleases::start();
        for (os, arch, delivery) in [("windows", "x86_64", Delivery::RunInstaller), ("macos", "aarch64", Delivery::OpenDisk)] {
            let dir = scratch(os);
            let (name, _) = asset_name("0.10.0", os, arch).unwrap();
            releases.announce_desktop("0.10.0");
            releases.publish_desktop("0.10.0", &[(&name, b"the installer")]);
            let app = Updater { os, arch, ..machine(&releases, &dir, "0.9.0") };
            let offer = app.check(Duration::ZERO).unwrap();
            assert_eq!(offer.delivery, delivery);

            let outcome = app.install(&offer, &mut |_| {}).unwrap();
            let path = dir.join("downloads").join(&name);
            assert_eq!(outcome, Outcome::Downloaded { path: path.clone(), delivery });
            assert_eq!(std::fs::read(&path).unwrap(), b"the installer");
            assert_eq!(std::fs::read(dir.join("bin").join(APP_BINARY)).unwrap(), OLD, "the running binary was touched");

            releases.put(&format!("/download/desktop-v0.10.0/{name}"), "tampered");
            let error = app.install(&offer, &mut |_| {}).unwrap_err();
            assert!(error.contains("checksum mismatch"), "{error}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Una instalación que no puede actualizarse sola no baja nada. Rotura que
    /// atrapa: bajar y fallar al renombrar en una carpeta ajena, tras gastar la
    /// descarga.
    #[test]
    fn a_manual_install_downloads_nothing() {
        let releases = FakeReleases::start();
        let dir = scratch("manual");
        publish_linux(&releases, "0.10.0", b"new");
        let app = machine(&releases, &dir, "0.9.0");
        let offer = Offer { version: "0.10.0".into(), asset: "cac-desktop-0.10.0-linux-x86_64".into(), delivery: Delivery::Manual };
        let before = releases.asked().len();
        assert!(app.install(&offer, &mut |_| {}).is_err());
        assert_eq!(releases.asked().len(), before);
        assert_eq!(release_page("0.10.0"), "https://github.com/g-studio-labs/c-c/releases/tag/desktop-v0.10.0");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Una versión que `VERSION` anuncia pero que aún no está publicada no se
    /// ofrece: el clic acabaría en un 404. Roturas que atrapa: ofrecer en
    /// cuanto `VERSION` lo dice; contentarse con que exista `SHA256SUMS` sin
    /// que nombre el fichero de este sistema (un sistema que falló al
    /// publicar); dejar de ofrecerla cuando por fin se publica.
    #[test]
    fn a_version_that_is_announced_but_not_published_is_not_offered() {
        let releases = FakeReleases::start();
        let dir = scratch("unpublished");
        let app = machine(&releases, &dir, "0.9.0");
        releases.announce_desktop("0.10.0");
        assert_eq!(app.check(Duration::ZERO), None, "no release at all");

        releases.publish_desktop("0.10.0", &[("cac-desktop-0.10.0-windows-x86_64-setup.exe", b"other system")]);
        assert_eq!(app.check(Duration::ZERO), None, "a release without this system's file");

        publish_linux(&releases, "0.10.0", b"new");
        assert_eq!(app.check(Duration::ZERO).map(|offer| offer.version), Some("0.10.0".into()), "published, and the notice never came");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Una compilación de desarrollo no se ofrece. Rotura que atrapa: pisar con
    /// el publicado el binario de `target/debug` que se está probando.
    #[test]
    fn a_binary_inside_a_cargo_target_folder_is_never_offered_an_update() {
        let releases = FakeReleases::start();
        let dir = scratch("dev");
        publish_linux(&releases, "0.10.0", b"new");
        let mut app = machine(&releases, &dir, "0.9.0");
        assert!(app.check(Duration::ZERO).is_some());
        app.exe = Some(dir.join("repo").join("target").join("release").join(APP_BINARY));
        assert_eq!(app.check(Duration::ZERO), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Lo que dejó una ejecución anterior se borra, y sólo eso. Roturas que
    /// atrapa: borrar carpetas que no son de la app; borrar la de esta misma
    /// ejecución; borrar lo reciente (puede ser de otra instancia que está
    /// bajando ahora).
    #[test]
    fn old_download_folders_are_removed_and_nothing_else() {
        let releases = FakeReleases::start();
        let dir = scratch("leftovers");
        let mut app = machine(&releases, &dir, "0.9.0");
        app.download_dir = dir.join(format!("{DOWNLOAD_DIR_PREFIX}200"));
        for name in [format!("{DOWNLOAD_DIR_PREFIX}100"), format!("{DOWNLOAD_DIR_PREFIX}200"), "somebody-elses".to_string()] {
            std::fs::create_dir_all(dir.join(name).join("inner")).unwrap();
        }
        app.clean_leftovers(Duration::from_secs(3600));
        assert!(dir.join(format!("{DOWNLOAD_DIR_PREFIX}100")).exists(), "removed what is recent");
        app.clean_leftovers(Duration::ZERO);
        assert!(!dir.join(format!("{DOWNLOAD_DIR_PREFIX}100")).exists(), "left an old download folder");
        assert!(dir.join(format!("{DOWNLOAD_DIR_PREFIX}200")).exists(), "removed this run's own folder");
        assert!(dir.join("somebody-elses").exists(), "removed a folder that is not the app's");
        let _ = std::fs::remove_dir_all(dir);
    }
}
