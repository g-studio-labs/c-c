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

const LABEL: &str = "dev.guz-studio.cac-listen";

/// Un `launchctl` con estado (el marcador `loaded`) y un `id` que contesta
/// 501, ambos de mentira: el `PATH` de la prueba no tiene los de verdad.
fn fake_launchctl(dir: &Path) {
    let bin = dir.join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\nprintf '%s | LD=[%s]\\n' \"$*\" \"$LD_LIBRARY_PATH\" >> '{log}'\ncase \"$1\" in\n  print) [ -f '{mark}' ] && printf 'state = running\\npid = 7\\n' || exit 113;;\n  bootstrap|load) : > '{mark}';;\n  bootout) rm -f '{mark}';;\nesac\nexit 0\n",
        log = dir.join("launchctl.log").display(),
        mark = dir.join("loaded").display()
    );
    std::fs::write(bin.join("launchctl"), script).unwrap();
    std::fs::write(bin.join("id"), "#!/bin/sh\necho 501\n").unwrap();
    for name in ["launchctl", "id"] {
        std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn launchctl_calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("launchctl.log")).unwrap_or_default().lines().map(String::from).collect()
}

fn plist_file(dir: &Path) -> PathBuf {
    dir.join("Library").join("LaunchAgents").join(format!("{LABEL}.plist"))
}

/// En macOS (forzado, esto corre en Linux) el binario de verdad escribe el
/// LaunchAgent, lo carga en `gui/<uid>`, dice su estado y lo descarga
/// dejando los eventos.
///
/// Roturas que atrapa: un plist con el token; apuntar a un enlace en vez de
/// al binario; cargar en otro dominio que `gui/501`; lanzar `launchctl` sin
/// pasar por `host::command` (heredaría el `LD_LIBRARY_PATH` de la imagen);
/// no descargar al quitar; borrar los eventos; escribir una unidad de
/// systemd en macOS; y soltar el token en la salida.
#[test]
fn on_macos_install_status_and_uninstall_go_through_launchctl() {
    let _one = serial();
    let dir = scratch("mac-flow");
    fake_launchctl(&dir);
    let real = dir.join("real").join("cac-mcp");
    copy_binary(&real);
    let link = dir.join("bin").join("cac-mcp");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    std::fs::create_dir_all(dir.join("claude")).unwrap();
    std::fs::write(dir.join("claude").join(".claude.json"), json!({ "mcpServers": { "cac": { "env": { "CAC_TOKEN": TOKEN } } } }).to_string()).unwrap();
    let mac = |args: &[&str]| {
        let mut cmd = binary(&link, &dir);
        cmd.env("CAC_MCP_SERVICE_OS", "macos").args(args);
        cmd.output().unwrap()
    };

    let out = mac(&["listen", "install"]);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    let plist = std::fs::read_to_string(plist_file(&dir)).unwrap();
    let real = real.canonicalize().unwrap();
    assert!(plist.contains(&format!("<string>{}</string>\n    <string>listen</string>\n    <string>run</string>", real.display())), "{plist}");
    assert!(!plist.contains("EnvironmentVariables") && !plist.contains(TOKEN), "{plist}");
    assert_eq!(
        launchctl_calls(&dir),
        [format!("print gui/501/{LABEL} | LD=[]"), format!("bootstrap gui/501 {} | LD=[]", plist_file(&dir).display())]
    );
    assert!(!said(&out).contains(TOKEN));
    assert!(!unit_file(&dir).exists(), "en macOS no se escribe una unidad de systemd");

    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(dir.join("state").join("listen.json"), r#"{"cursor":42}"#).unwrap();
    let out = mac(&["listen", "status"]);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert!(said(&out).contains("loaded, running") && said(&out).contains("cursor: 42"), "{}", said(&out));

    let events = dir.join("cache").join("cac").join("events.jsonl");
    std::fs::create_dir_all(events.parent().unwrap()).unwrap();
    std::fs::write(&events, "{\"seq\":42}\n").unwrap();
    let out = mac(&["listen", "uninstall"]);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert!(!plist_file(&dir).exists());
    assert_eq!(std::fs::read_to_string(&events).unwrap(), "{\"seq\":42}\n", "uninstall borró los eventos");
    assert!(launchctl_calls(&dir).iter().any(|c| c.starts_with(&format!("bootout gui/501/{LABEL}"))), "{:?}", launchctl_calls(&dir));
    for call in launchctl_calls(&dir) {
        assert!(call.ends_with("LD=[]"), "{call}");
    }
}

/// `--dryRun` de `install_listener` en macOS enseña el plist y no escribe ni
/// carga nada. Habla con el binario por stdio.
///
/// Roturas que atrapa: que el dryRun escriba el plist o llame a `launchctl`,
/// o que enseñe la unidad de systemd en vez del plist.
#[test]
fn the_dry_run_shows_the_plist_and_writes_nothing() {
    use std::io::Write;
    let _one = serial();
    let dir = scratch("mac-dry");
    fake_launchctl(&dir);
    let at = dir.join("bin").join("cac-mcp");
    copy_binary(&at);
    let mut child = binary(&at, &dir).env("CAC_MCP_SERVICE_OS", "macos").stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } } });
    let call = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "install_listener", "arguments": { "dryRun": true } } });
    writeln!(stdin, "{init}").unwrap();
    writeln!(stdin, "{call}").unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains(".plist") && text.contains("ProgramArguments") && !text.contains(TOKEN), "{text}");
    assert!(!plist_file(&dir).exists() && launchctl_calls(&dir).is_empty() && !unit_file(&dir).exists());
}

/// `cac-mcp update` reinicia el agente de macOS si existe, con
/// `kickstart -k`, y no toca launchd si no.
///
/// Roturas que atrapa: no reiniciar tras actualizar; llamar a `launchctl`
/// sin que haya agente; usar `systemctl` en macOS.
#[test]
fn on_macos_an_update_restarts_the_agent_only_when_it_is_installed() {
    let _one = serial();
    let releases = FakeReleases::start();
    releases.announce("999.0.0");
    releases.publish("999.0.0", b"#!/bin/sh\necho 'cac-mcp 999.0.0'\n");
    let source = releases.source();
    let run = |dir: &Path, program: &Path| {
        binary(program, dir)
            .env("CAC_MCP_SERVICE_OS", "macos")
            .env("CAC_MCP_VERSION_URL", &source.version_url)
            .env("CAC_MCP_DOWNLOAD_BASE", &source.download_base)
            .arg("update")
            .output()
            .unwrap()
    };

    let dir = scratch("mac-update-none");
    fake_launchctl(&dir);
    let at = dir.join("bin").join("cac-mcp");
    copy_binary(&at);
    assert_eq!(run(&dir, &at).status.code(), Some(0));
    assert!(launchctl_calls(&dir).is_empty(), "{:?}", launchctl_calls(&dir));

    let dir = scratch("mac-update-some");
    fake_launchctl(&dir);
    let at = dir.join("bin").join("cac-mcp");
    copy_binary(&at);
    std::fs::create_dir_all(plist_file(&dir).parent().unwrap()).unwrap();
    std::fs::write(plist_file(&dir), "<plist/>").unwrap();
    let out = run(&dir, &at);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert_eq!(launchctl_calls(&dir), [format!("kickstart -k gui/501/{LABEL} | LD=[]")]);
    assert!(said(&out).contains("restarted"), "{}", said(&out));
}
