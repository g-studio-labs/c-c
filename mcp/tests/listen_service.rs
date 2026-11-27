//! `cac-mcp listen install | uninstall | status` y el reinicio al actualizar,
//! con el binario de verdad y un `systemctl` de mentira en el `PATH`: nada
//! toca el systemd de esta máquina (command-and-control-558).
#![cfg(unix)]

use cac_mcp::update::fake::FakeReleases;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const TOKEN: &str = "cac_pat_SERVICESECRET0123456789";
const UNIT: &str = "cac-listen.service";

/// Una prueba a la vez: al copiar el binario, el descriptor abierto para
/// escribir se hereda en el `fork` de otra prueba que lance un proceso a la
/// vez, y el `exec` de la copia da «Text file busy». No se arregla con un
/// plazo: se evita que dos pruebas lancen procesos a la vez.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cac-mcp-service-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Un `systemctl` que sólo apunta, en `systemctl.log`, sus argumentos y qué
/// valía `LD_LIBRARY_PATH` cuando lo lanzaron. Sólo usa la shell: el `PATH`
/// de la prueba no tiene nada más.
fn fake_systemctl(dir: &Path) {
    let bin = dir.join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\nprintf '%s | LD=[%s]\\n' \"$*\" \"$LD_LIBRARY_PATH\" >> '{}'\ncase \"$*\" in\n  *is-active*) echo active;;\n  *is-enabled*) echo enabled;;\nesac\nexit 0\n",
        dir.join("systemctl.log").display()
    );
    std::fs::write(bin.join("systemctl"), script).unwrap();
    std::fs::set_permissions(bin.join("systemctl"), std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("systemctl.log")).unwrap_or_default().lines().map(String::from).collect()
}

/// El binario copiado a `at`, con todo lo suyo dentro de `dir`, sin token en
/// el entorno y con las variables de una AppImage heredadas de otro programa.
fn binary(at: &Path, dir: &Path) -> Command {
    let mut cmd = Command::new(at);
    cmd.env_clear()
        .env("PATH", dir.join("fakebin"))
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("CLAUDE_CONFIG_DIR", dir.join("claude"))
        .env("CAC_MCP_STATE_DIR", dir.join("state"))
        .env("CAC_MCP_NO_UPDATE_CHECK", "1")
        .env("APPDIR", "/tmp/.mount_cacINHERITED")
        .env("LD_LIBRARY_PATH", "/tmp/.mount_cacINHERITED/usr/lib")
        .stdin(Stdio::null());
    cmd
}

fn copy_binary(to: &Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_cac-mcp"), to).unwrap();
}

fn said(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn unit_file(dir: &Path) -> PathBuf {
    dir.join("config").join("systemd").join("user").join(UNIT)
}

/// Roturas que atrapa: una unidad con el token o con `Environment=`; apuntar
/// a un enlace en vez de al binario de verdad; no recargar ni habilitar con
/// `--now`; lanzar `systemctl` sin pasar por `host::command` (heredaría el
/// `LD_LIBRARY_PATH` de la imagen); y soltar el token en la salida.
#[test]
fn install_status_and_uninstall_through_the_binary() {
    let _one = serial();
    let dir = scratch("flow");
    fake_systemctl(&dir);
    let real = dir.join("real").join("cac-mcp");
    copy_binary(&real);
    let link = dir.join("bin").join("cac-mcp");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    // El token está en el `.claude.json`, como en la máquina de verdad.
    std::fs::create_dir_all(dir.join("claude")).unwrap();
    std::fs::write(dir.join("claude").join(".claude.json"), json!({ "mcpServers": { "cac": { "env": { "CAC_TOKEN": TOKEN } } } }).to_string()).unwrap();

    let out = binary(&link, &dir).args(["listen", "install"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    let unit = std::fs::read_to_string(unit_file(&dir)).unwrap();
    let real = real.canonicalize().unwrap();
    assert!(unit.contains(&format!("ExecStart=\"{}\" listen run\n", real.display())), "{unit}");
    assert!(!unit.contains("Environment") && !unit.contains(TOKEN), "{unit}");
    assert_eq!(calls(&dir), [format!("--user daemon-reload | LD=[]"), format!("--user enable --now {UNIT} | LD=[]")]);
    assert!(!said(&out).contains(TOKEN));

    // Un fichero de eventos que ya existe y un cursor: `status` los dice.
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(dir.join("state").join("listen.json"), r#"{"cursor":42}"#).unwrap();
    let out = binary(&link, &dir).args(["listen", "status"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    let text = said(&out);
    assert!(text.contains("enabled, active") && text.contains("cursor: 42"), "{text}");
    assert!(!text.contains(TOKEN));

    let events = dir.join("cache").join("cac").join("events.jsonl");
    std::fs::create_dir_all(events.parent().unwrap()).unwrap();
    std::fs::write(&events, "{\"seq\":42}\n").unwrap();
    let out = binary(&link, &dir).args(["listen", "uninstall"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert!(!unit_file(&dir).exists());
    assert_eq!(std::fs::read_to_string(&events).unwrap(), "{\"seq\":42}\n", "uninstall borró los eventos");
    assert!(said(&out).contains(&events.display().to_string()), "{}", said(&out));
    assert!(calls(&dir).iter().any(|c| c.starts_with(&format!("--user disable --now {UNIT}"))), "{:?}", calls(&dir));
    for call in calls(&dir) {
        assert!(call.starts_with("--user ") && call.ends_with("LD=[]"), "{call}");
    }
}

/// Roturas que atrapa: instalar desde dentro de una AppImage (el binario
/// vive en un montaje `.mount_…`): escribiría una unidad que se rompe en la
/// siguiente actualización.
#[test]
fn an_appimage_is_refused_with_nothing_written() {
    let _one = serial();
    let dir = scratch("appimage");
    fake_systemctl(&dir);
    let inside = dir.join(".mount_cacXYZ").join("usr").join("bin").join("cac-mcp");
    copy_binary(&inside);
    let out = binary(&inside, &dir).args(["listen", "install"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", said(&out));
    assert!(said(&out).contains("standalone"), "{}", said(&out));
    assert!(!unit_file(&dir).exists() && calls(&dir).is_empty());
}

/// `cac-mcp update` reinicia el servicio si existe y no toca systemd si no.
///
/// Roturas que atrapa: no reiniciar tras actualizar; llamar a `systemctl`
/// sin que haya servicio; reiniciar cuando la actualización falló.
#[test]
fn an_update_restarts_the_service_only_when_it_is_installed() {
    let _one = serial();
    let releases = FakeReleases::start();
    releases.announce("999.0.0");
    releases.publish("999.0.0", b"#!/bin/sh\necho 'cac-mcp 999.0.0'\n");
    let source = releases.source();
    let run = |dir: &Path, program: &Path| {
        binary(program, dir).env("CAC_MCP_VERSION_URL", &source.version_url).env("CAC_MCP_DOWNLOAD_BASE", &source.download_base).arg("update").output().unwrap()
    };

    let dir = scratch("update-none");
    fake_systemctl(&dir);
    let at = dir.join("bin").join("cac-mcp");
    copy_binary(&at);
    let out = run(&dir, &at);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert!(calls(&dir).is_empty(), "sin servicio no se toca systemd: {:?}", calls(&dir));

    let dir = scratch("update-some");
    fake_systemctl(&dir);
    let at = dir.join("bin").join("cac-mcp");
    copy_binary(&at);
    std::fs::create_dir_all(unit_file(&dir).parent().unwrap()).unwrap();
    std::fs::write(unit_file(&dir), "[Service]\n").unwrap();
    let out = run(&dir, &at);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert_eq!(calls(&dir), [format!("--user try-restart {UNIT} | LD=[]")]);

    // Una actualización que falla no reinicia nada.
    let dir = scratch("update-failed");
    fake_systemctl(&dir);
    let at = dir.join("bin").join("cac-mcp");
    copy_binary(&at);
    std::fs::create_dir_all(unit_file(&dir).parent().unwrap()).unwrap();
    std::fs::write(unit_file(&dir), "[Service]\n").unwrap();
    let asset = cac_mcp::update::this_platform_asset().unwrap();
    releases.put(&format!("/download/mcp-v999.0.0/{asset}"), "#!/bin/sh\necho robado\n");
    let out = run(&dir, &at);
    assert_eq!(out.status.code(), Some(1), "{}", said(&out));
    assert!(calls(&dir).is_empty(), "{:?}", calls(&dir));
}
