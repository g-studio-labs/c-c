//! `scripts/install-mcp.sh`, corrido de verdad contra una release de mentira.
//!
//! Con cada shell POSIX que haya en la máquina (`sh` es dash en Ubuntu, que es
//! donde corre el CI, y bash en otras), y por una tubería, que es como lo usa
//! quien sigue la línea de instalación. `HOME` es una carpeta temporal: nada
//! toca el `~/.local/bin` de quien corre las pruebas, y nada sale a internet.
//!
//! El de PowerShell (`install-mcp.ps1`) no se puede correr aquí; lo único que
//! se comprueba de él, en `tests/release.rs`, es que nombra los mismos
//! ficheros y direcciones.
#![cfg(unix)]

use cac_mcp::update::fake::FakeReleases;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BINARY: &[u8] = b"#!/bin/sh\necho 'cac-mcp 1.34.0'\n";
const NEWER_BINARY: &[u8] = b"#!/bin/sh\necho 'cac-mcp 1.35.0'\n";
const SECRET: &str = "cac_pat_no-debe-salir-nunca";

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/install-mcp.sh")
}

/// Las shells con las que se prueba: las que haya de estas.
fn shells() -> Vec<&'static str> {
    let found: Vec<&str> = ["sh", "dash", "bash"]
        .into_iter()
        .filter(|shell| Command::new(shell).args(["-c", "exit 0"]).status().is_ok_and(|status| status.success()))
        .collect();
    assert!(found.contains(&"sh"), "no hay sh en esta máquina");
    found
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cac-mcp-script-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Run {
    code: Option<i32>,
    out: String,
    err: String,
}

/// Corre el script por una tubería (`… | sh`), con `HOME` en `home` y las
/// direcciones apuntando a la release de mentira.
fn install(shell: &str, home: &Path, releases: &FakeReleases, edit: impl FnOnce(&mut Command)) -> Run {
    let source = releases.source();
    let mut cmd = Command::new(shell);
    cmd.env("HOME", home)
        .env_remove("CAC_MCP_VERSION")
        .env_remove("CAC_MCP_INSTALL_DIR")
        .env("CAC_MCP_VERSION_URL", &source.version_url)
        .env("CAC_MCP_DOWNLOAD_BASE", &source.download_base)
        // Un token en el entorno, como lo tendría quien ya usa cac: el script
        // no tiene por qué mirarlo, y desde luego no enseñarlo.
        .env("CAC_TOKEN", SECRET)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    edit(&mut cmd);
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(&std::fs::read(script()).unwrap()).unwrap();
    let out = child.wait_with_output().unwrap();
    Run { code: out.status.code(), out: String::from_utf8_lossy(&out.stdout).into(), err: String::from_utf8_lossy(&out.stderr).into() }
}

fn files_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut names: Vec<String> = entries.map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    names.sort();
    names
}

/// Con la suma buena instala en `~/.local/bin/cac-mcp`, ejecutable, y dice
/// cómo registrarlo; volver a correrlo con una versión nueva lo actualiza.
///
/// Roturas que atrapa: instalar en otro sitio o sin permiso de ejecución;
/// dejar temporales al lado del binario; una orden de registro que no apunta
/// al binario instalado; enseñar un token; un segundo pase que no reemplaza;
/// no leer la versión de `mcp/VERSION`.
#[test]
fn a_good_checksum_installs_and_a_second_run_upgrades() {
    for shell in shells() {
        let releases = FakeReleases::start();
        releases.announce("1.34.0");
        releases.publish("1.34.0", BINARY);
        let home = home(&format!("good-{shell}"));
        let bin = home.join(".local").join("bin");
        let dest = bin.join("cac-mcp");

        let run = install(shell, &home, &releases, |_| {});
        assert_eq!(run.code, Some(0), "[{shell}] {}", run.err);
        assert_eq!(std::fs::read(&dest).unwrap(), BINARY, "[{shell}]");
        assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o755, "[{shell}]");
        assert_eq!(files_in(&bin), ["cac-mcp"], "[{shell}] quedó algo más que el binario");
        let register = format!("claude mcp add cac -e CAC_URL=https://cac.guz-studio.dev -e CAC_TOKEN=PASTE_YOUR_TOKEN -- \"{}\"", dest.display());
        assert!(run.out.contains(&register), "[{shell}] {}", run.out);
        assert!(run.out.contains("Installed cac-mcp 1.34.0"), "[{shell}] {}", run.out);
        assert!(!run.out.contains(SECRET) && !run.err.contains(SECRET), "[{shell}] enseñó el token");
        // El binario que quedó es el que corre.
        let version = Command::new(&dest).arg("--version").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&version.stdout), "cac-mcp 1.34.0\n");

        releases.announce("1.35.0");
        releases.publish("1.35.0", NEWER_BINARY);
        let again = install(shell, &home, &releases, |_| {});
        assert_eq!(again.code, Some(0), "[{shell}] {}", again.err);
        assert_eq!(std::fs::read(&dest).unwrap(), NEWER_BINARY, "[{shell}] el segundo pase no actualizó");
        assert_eq!(files_in(&bin), ["cac-mcp"], "[{shell}]");
        let _ = std::fs::remove_dir_all(home);
    }
}

/// Una descarga cuya suma no es la publicada no se instala, ni deja nada; y
/// si ya había un binario, sigue siendo el de antes.
///
/// Roturas que atrapa: no comparar la suma; instalar y avisar después;
/// instalar cuando `SHA256SUMS` no nombra el fichero; dejar el temporal con
/// el binario malo, ya ejecutable, en una carpeta del `PATH`; salir con éxito.
#[test]
fn a_bad_checksum_refuses_and_leaves_nothing() {
    for shell in shells() {
        let releases = FakeReleases::start();
        releases.announce("1.34.0");
        releases.publish("1.34.0", BINARY);
        let asset = cac_mcp::update::this_platform_asset().unwrap();
        releases.put(&format!("/download/mcp-v1.34.0/{asset}"), "#!/bin/sh\necho robado\n");
        let home = home(&format!("bad-{shell}"));
        let bin = home.join(".local").join("bin");

        let run = install(shell, &home, &releases, |_| {});
        assert_eq!(run.code, Some(1), "[{shell}] {}{}", run.out, run.err);
        assert!(run.err.contains("checksum mismatch") && run.err.contains("Nothing was installed"), "[{shell}] {}", run.err);
        assert!(!run.out.contains("Installed"), "[{shell}] {}", run.out);
        assert_eq!(files_in(&bin), Vec::<String>::new(), "[{shell}] quedó algo tras rechazar la descarga");

        // Sin suma para ese fichero tampoco.
        releases.put("/download/mcp-v1.34.0/SHA256SUMS", format!("{}  otro-fichero\n", "0".repeat(64)));
        let run = install(shell, &home, &releases, |_| {});
        assert_eq!(run.code, Some(1), "[{shell}]");
        assert!(run.err.contains("no checksum for"), "[{shell}] {}", run.err);
        assert_eq!(files_in(&bin), Vec::<String>::new(), "[{shell}]");

        // Con uno ya instalado: el de antes sigue ahí, intacto.
        releases.publish("1.34.0", BINARY);
        assert_eq!(install(shell, &home, &releases, |_| {}).code, Some(0));
        releases.announce("1.35.0");
        releases.publish("1.35.0", NEWER_BINARY);
        releases.put(&format!("/download/mcp-v1.35.0/{asset}"), "manipulado");
        let run = install(shell, &home, &releases, |_| {});
        assert_eq!(run.code, Some(1), "[{shell}]");
        assert_eq!(std::fs::read(bin.join("cac-mcp")).unwrap(), BINARY, "[{shell}] una descarga rechazada tocó el binario instalado");
        assert_eq!(files_in(&bin), ["cac-mcp"], "[{shell}]");
        let _ = std::fs::remove_dir_all(home);
    }
}

/// Una plataforma para la que no se publica binario lo dice y no baja nada;
/// la versión se puede fijar por el entorno, y la carpeta también; y lo que
/// no es una versión no se instala.
///
/// Roturas que atrapa: bajar el binario de otra arquitectura (no arrancaría);
/// ignorar `CAC_MCP_VERSION` o `CAC_MCP_INSTALL_DIR`; tomar por versión la
/// página de un portal cautivo y pedir una release con ese nombre.
#[test]
fn an_unsupported_platform_says_so_and_the_version_and_folder_can_be_chosen() {
    for shell in shells() {
        let releases = FakeReleases::start();
        releases.announce("1.35.0");
        releases.publish("1.34.0", BINARY);
        let home = home(&format!("misc-{shell}"));

        // Un `uname` que dice ser otra máquina, por delante en el PATH.
        let fake_bin = home.join("fake-bin");
        std::fs::create_dir_all(&fake_bin).unwrap();
        let uname = fake_bin.join("uname");
        std::fs::write(&uname, "#!/bin/sh\ncase \"$1\" in -s) echo Linux ;; -m) echo riscv64 ;; esac\n").unwrap();
        std::fs::set_permissions(&uname, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());
        let run = install(shell, &home, &releases, |cmd| {
            cmd.env("PATH", &path);
        });
        assert_eq!(run.code, Some(1), "[{shell}]");
        assert!(run.err.contains("there is no cac-mcp binary for Linux riscv64"), "[{shell}] {}", run.err);
        assert!(releases.asked().is_empty(), "[{shell}] bajó algo para una plataforma sin binario: {:?}", releases.asked());
        assert!(!home.join(".local").exists(), "[{shell}]");

        // La versión pedida, no la última; y donde se pidió.
        let elsewhere = home.join("otra carpeta");
        let run = install(shell, &home, &releases, |cmd| {
            cmd.env("CAC_MCP_VERSION", "1.34.0").env("CAC_MCP_INSTALL_DIR", &elsewhere);
        });
        assert_eq!(run.code, Some(0), "[{shell}] {}", run.err);
        assert_eq!(std::fs::read(elsewhere.join("cac-mcp")).unwrap(), BINARY, "[{shell}]");
        assert!(!releases.asked().contains(&"/VERSION".to_string()), "[{shell}] con la versión fijada no hay nada que preguntar");
        assert!(!home.join(".local").exists(), "[{shell}] instaló también en la carpeta por defecto");

        releases.put("/VERSION", "<html>Sign in to the hotel WiFi</html>");
        let run = install(shell, &home, &releases, |_| {});
        assert_eq!(run.code, Some(1), "[{shell}]");
        assert!(run.err.contains("is not X.Y.Z"), "[{shell}] {}", run.err);
        assert!(!releases.asked().iter().any(|path| path.contains("html")), "[{shell}] {:?}", releases.asked());
        assert_eq!(files_in(&home.join(".local").join("bin")), Vec::<String>::new(), "[{shell}]");
        let _ = std::fs::remove_dir_all(home);
    }
}
