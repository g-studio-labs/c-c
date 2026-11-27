//! `cac-mcp listen install | uninstall | status`: el servicio de usuario que
//! corre `listen run` (command-and-control-558).
//!
//! Es una unidad de systemd de la persona (`~/.config/systemd/user`), no del
//! sistema: no pide root y muere con ella. **No lleva el token ni
//! `Environment=`**: `listen run` lo lee del `.claude.json` en cada conexión,
//! así que un token cambiado se toma sin reinstalar y la unidad no guarda
//! ningún secreto. Tampoco activa *linger*: con la sesión cerrada no escucha, y
//! al volver se pone al día con el diario del backend.
//!
//! Sólo para el binario suelto. La ruta de una AppImage cambia en cada
//! actualización (se monta en `/tmp/.mount_…`), y una unidad que apunte a ella
//! se rompe sola; por eso `install` se niega ahí y dice que se instale
//! `cac-mcp`. En macOS y Windows `install` contesta que aún no se puede (launchd
//! y el Programador de tareas van en otra tarea) y cómo correrlo a mano.
//!
//! Todo `systemctl` pasa por `host::command`, como cualquier programa del
//! sistema: dentro de una AppImage el entorno apunta a las bibliotecas de la
//! imagen y el programa del sistema moriría con ellas.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;

use crate::{host, listen, update};

pub const UNIT: &str = "cac-listen.service";
const NOT_SUPPORTED: &str = "not supported on this system yet";

/// Lo que dijo un `systemctl`.
pub struct Ran {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

pub type Systemctl<'a> = &'a dyn Fn(&[&str]) -> io::Result<Ran>;

/// El `systemctl --user` de verdad.
pub fn systemctl(args: &[&str]) -> io::Result<Ran> {
    let out = host::command("systemctl").arg("--user").args(args).stdin(Stdio::null()).output()?;
    Ok(Ran { ok: out.status.success(), stdout: String::from_utf8_lossy(&out.stdout).trim().into(), stderr: String::from_utf8_lossy(&out.stderr).trim().into() })
}

/// Dónde está cada cosa y quién corre esto.
pub struct Setup {
    pub os: &'static str,
    pub unit_dir: PathBuf,
    pub exe: PathBuf,
    pub appdir: Option<PathBuf>,
    pub appimage: Option<PathBuf>,
    pub events: PathBuf,
    pub state: PathBuf,
}

impl Setup {
    fn unit_path(&self) -> PathBuf {
        self.unit_dir.join(UNIT)
    }
}

/// `$XDG_CONFIG_HOME/systemd/user`, o `~/.config/systemd/user`.
fn unit_dir_from(env: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let base = env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|home| home.join(".config")))?;
    Some(base.join("systemd").join("user"))
}

fn unit_path_here() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty());
    unit_dir_from(&env).map(|dir| dir.join(UNIT))
}

fn setup_here() -> Result<Setup, String> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty());
    let unit_dir = unit_dir_from(&env).ok_or("no home folder to keep the unit in")?;
    // El fichero de verdad, no un enlace: la actualización reemplaza el
    // binario por renombrado y el enlace sigue valiendo, pero un enlace
    // dentro de una carpeta de la que se borra la dejaría colgando.
    let exe = std::env::current_exe().and_then(|exe| exe.canonicalize()).map_err(|e| format!("could not find its own file: {e}"))?;
    Ok(Setup {
        os: std::env::consts::OS,
        unit_dir,
        exe,
        appdir: env("APPDIR"),
        appimage: env("APPIMAGE"),
        events: listen::events_file().ok_or("no home folder to keep the events in")?,
        state: update::state_dir().ok_or("no home folder to keep the state in")?,
    })
}

/// Si el binario que corre está dentro de una AppImage. Mirar sólo las
/// variables no vale: al binario suelto le llegan heredadas de otro programa
/// (un terminal de la app), y lo que importa es dónde vive **él**.
pub fn lives_in_an_appimage(exe: &Path, appdir: Option<&Path>, appimage: Option<&Path>) -> bool {
    appdir.is_some_and(|dir| exe.starts_with(dir))
        || appimage.is_some_and(|image| exe == image)
        || exe.components().any(|c| c.as_os_str().to_string_lossy().starts_with(".mount_"))
        || exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("appimage"))
}

/// Lo que va en la unidad para `ExecStart`: la ruta entre comillas, y `%` y
/// `$` duplicados (systemd los interpreta aun entre comillas).
fn quote(arg: &str) -> String {
    let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$");
    format!("\"{escaped}\"")
}

/// El texto de la unidad. Sin token y sin `Environment=`.
pub fn unit_text(exe: &Path) -> String {
    format!(
        "[Unit]\nDescription=cac events listener (cac-mcp listen run)\n\n[Service]\nExecStart={} listen run\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
        quote(&exe.to_string_lossy())
    )
}

pub struct Report {
    pub code: i32,
    pub out: String,
    pub err: String,
}

impl Report {
    fn ok(out: impl Into<String>) -> Self {
        Report { code: 0, out: out.into(), err: String::new() }
    }
    fn fail(code: i32, err: impl Into<String>) -> Self {
        Report { code, out: String::new(), err: err.into() }
    }
}

fn by_hand(os: &str) -> String {
    format!(
        "cac-mcp listen: a user service is {NOT_SUPPORTED} ({os}). Run it by hand instead: `cac-mcp listen run` (leave it open in a terminal), and read with `cac-mcp listen follow --consumer <name>`."
    )
}

pub fn install(setup: &Setup, sys: Systemctl) -> Report {
    if setup.os != "linux" {
        return Report::fail(2, by_hand(setup.os));
    }
    if lives_in_an_appimage(&setup.exe, setup.appdir.as_deref(), setup.appimage.as_deref()) {
        return Report::fail(
            2,
            format!(
                "cac-mcp listen: {} is inside an AppImage, whose path changes with every update, so a service pointing at it would break by itself. Install the standalone `cac-mcp` (scripts/install-mcp.sh) and run `cac-mcp listen install` from it.",
                setup.exe.display()
            ),
        );
    }
    let unit = setup.unit_path();
    let text = unit_text(&setup.exe);
    let before = fs::read_to_string(&unit).ok();
    if let Err(e) = fs::create_dir_all(&setup.unit_dir).and_then(|()| fs::write(&unit, &text)) {
        return Report::fail(1, format!("cac-mcp listen: could not write {}: {e}", unit.display()));
    }
    for args in [&["daemon-reload"][..], &["enable", "--now", UNIT]] {
        match sys(args) {
            Ok(ran) if ran.ok => {}
            Ok(ran) => return Report::fail(1, format!("cac-mcp listen: `systemctl --user {}` failed: {}. The unit is written at {}.", args.join(" "), ran.stderr, unit.display())),
            Err(e) => return Report::fail(1, format!("cac-mcp listen: could not run systemctl: {e}. The unit is written at {}.", unit.display())),
        }
    }
    // Una unidad que ya corría con otra ruta no la toma hasta reiniciar.
    if before.as_deref().is_some_and(|old| old != text) {
        let _ = sys(&["try-restart", UNIT]);
    }
    let mut out = format!("Installed {} and started it.\nIt runs `{} listen run` and keeps the events in {}.\n", unit.display(), setup.exe.display(), setup.events.display());
    if listen::credentials().is_err() {
        out.push_str("Note: there is no cac token yet, so it will wait; connect Claude Code to cac (mcpServers.cac in .claude.json) and it picks it up by itself.\n");
    }
    out.push_str("It listens while you are logged in (no linger); when you come back it catches up from cac's journal.\nCheck it with `cac-mcp listen status`; remove it with `cac-mcp listen uninstall`.\n");
    Report::ok(out)
}

pub fn uninstall(setup: &Setup, sys: Systemctl) -> Report {
    if setup.os != "linux" {
        return Report::fail(2, by_hand(setup.os));
    }
    let unit = setup.unit_path();
    if !unit.exists() {
        return Report::ok(format!("Nothing to remove: {} is not installed. The events file is at {}.\n", UNIT, setup.events.display()));
    }
    let mut notes = String::new();
    match sys(&["disable", "--now", UNIT]) {
        Ok(ran) if ran.ok => {}
        Ok(ran) => notes.push_str(&format!("Note: `systemctl --user disable --now` said: {}\n", ran.stderr)),
        Err(e) => notes.push_str(&format!("Note: could not run systemctl: {e}\n")),
    }
    if let Err(e) = fs::remove_file(&unit) {
        return Report::fail(1, format!("cac-mcp listen: could not remove {}: {e}", unit.display()));
    }
    let _ = sys(&["daemon-reload"]);
    Report::ok(format!("{notes}Removed {UNIT}. The events file stays at {} (delete it if you do not want it).\n", setup.events.display()))
}

fn read_json(path: &Path) -> Value {
    fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null)
}

pub fn status(setup: &Setup, sys: Systemctl) -> Report {
    let mut out = String::new();
    let mut running = false;
    if setup.os != "linux" {
        out.push_str(&format!("unit: {NOT_SUPPORTED} ({}); run `cac-mcp listen run` by hand\n", setup.os));
    } else if !setup.unit_path().exists() {
        out.push_str("unit: not installed (`cac-mcp listen install`)\n");
    } else {
        let word = |args: &[&str]| sys(args).map(|ran| ran.stdout).unwrap_or_default();
        let active = word(&["is-active", UNIT]);
        let enabled = word(&["is-enabled", UNIT]);
        running = active == "active";
        out.push_str(&format!("unit: {} ({}, {})\n", setup.unit_path().display(), if enabled.is_empty() { "unknown" } else { &enabled }, if active.is_empty() { "unknown" } else { &active }));
        if !running {
            out.push_str(&format!("stopped: the unit is `{}`; see `journalctl --user -u {UNIT}`\n", if active.is_empty() { "unknown" } else { &active }));
        }
    }
    let listener = read_json(&listen::status_file(&setup.state));
    let field = |name: &str| listener[name].as_str().map(String::from);
    if let Some(state) = field("state") {
        out.push_str(&format!("listener: {state}{}\n", field("updatedAt").map(|at| format!(" (as of {at})")).unwrap_or_default()));
    }
    if let Some(reason) = field("reason") {
        out.push_str(&format!("reason: {reason}\n"));
    }
    if let Some(wait) = listener["retryInSeconds"].as_u64() {
        out.push_str(&format!("retrying in: {wait} s\n"));
    }
    let cursor = listen::Journal::new(setup.events.clone(), setup.state.clone()).saved_cursor();
    out.push_str(&format!("cursor: {}\n", cursor.map_or("none yet".to_string(), |c| c.to_string())));
    out.push_str(&format!("last connected: {}\n", field("lastConnectedAt").unwrap_or_else(|| "never".into())));
    out.push_str(&format!("last event: {}\n", field("lastEventAt").unwrap_or_else(|| "none".into())));
    out.push_str(&format!("events file: {}\n", setup.events.display()));
    Report { code: if running { 0 } else { 1 }, out, err: String::new() }
}

/// `cac-mcp update` acabó bien: si el servicio está instalado, lo reinicia
/// para que corra la versión nueva (lo pidió quien actualiza). Devuelve lo que
/// hay que decir, o nada si no hay servicio.
pub fn restart_after_update(unit_path: Option<&Path>, sys: Systemctl) -> Option<String> {
    if !unit_path?.exists() {
        return None;
    }
    Some(match sys(&["try-restart", UNIT]) {
        Ok(ran) if ran.ok => format!("The listener service ({UNIT}) was restarted on the new version."),
        Ok(ran) => format!("Could not restart {UNIT}: {}. Run `systemctl --user restart {UNIT}`.", ran.stderr),
        Err(e) => format!("Could not restart {UNIT}: {e}. Run `systemctl --user restart {UNIT}`."),
    })
}

/// Lo que `cac-mcp update` llama al terminar.
pub fn restart_after_update_here() -> Option<String> {
    restart_after_update(unit_path_here().as_deref(), &systemctl)
}

/// `cac-mcp listen install|uninstall|status`.
pub fn main(args: &[&str]) -> i32 {
    let setup = match setup_here() {
        Ok(setup) => setup,
        Err(e) => {
            eprintln!("cac-mcp listen: {e}");
            return 1;
        }
    };
    let report = match args {
        ["install"] => install(&setup, &systemctl),
        ["uninstall"] => uninstall(&setup, &systemctl),
        ["status"] => status(&setup, &systemctl),
        _ => Report::fail(2, format!("cac-mcp listen: unknown arguments `{}`. Usage: cac-mcp listen install | uninstall | status", args.join(" "))),
    };
    print!("{}", report.out);
    if !report.err.is_empty() {
        eprintln!("{}", report.err);
    }
    report.code
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-mcp-service-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup(dir: &Path) -> Setup {
        Setup {
            os: "linux",
            unit_dir: dir.join("config").join("systemd").join("user"),
            exe: dir.join("bin").join("cac-mcp"),
            appdir: None,
            appimage: None,
            events: dir.join("cache").join("cac").join("events.jsonl"),
            state: dir.join("state"),
        }
    }

    /// Un `systemctl` de mentira: apunta lo que le piden y contesta lo que se
    /// le diga por programa. Nada toca el systemd de esta máquina.
    struct Fake {
        calls: RefCell<Vec<String>>,
        answers: Vec<(&'static str, bool, &'static str)>,
    }

    impl Fake {
        fn new(answers: Vec<(&'static str, bool, &'static str)>) -> Self {
            Fake { calls: RefCell::new(vec![]), answers }
        }
        fn run(&self, args: &[&str]) -> io::Result<Ran> {
            let line = args.join(" ");
            self.calls.borrow_mut().push(line.clone());
            let (ok, stdout) = self.answers.iter().find(|(prefix, _, _)| line.starts_with(prefix)).map_or((true, ""), |(_, ok, out)| (*ok, *out));
            Ok(Ran { ok, stdout: stdout.into(), stderr: if ok { String::new() } else { "boom".into() } })
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    /// Roturas que atrapa: meter el token o un `Environment=` en la unidad,
    /// no reiniciar (`Restart=always`), una ruta sin comillas (con espacios se
    /// parte) o un `%` sin duplicar (systemd lo toma por un especificador).
    #[test]
    fn the_unit_has_no_secret_and_quotes_the_path() {
        let text = unit_text(Path::new("/home/ana/my bin/cac%mcp"));
        assert!(text.contains("ExecStart=\"/home/ana/my bin/cac%%mcp\" listen run\n"), "{text}");
        assert!(text.contains("Restart=always"));
        assert!(!text.contains("Environment") && !text.to_lowercase().contains("token"), "{text}");
        assert!(text.contains("WantedBy=default.target"));
    }

    /// Roturas que atrapa: escribir otra cosa que la unidad en su sitio, no
    /// recargar o no habilitar con `--now` (no arrancaría), llamar al
    /// `systemctl` del sistema en vez de `--user`, y no reiniciar una unidad
    /// que ya corría con otra ruta.
    #[test]
    fn install_writes_the_unit_reloads_and_enables_it_now() {
        let dir = scratch("install");
        let s = setup(&dir);
        let sys = Fake::new(vec![]);
        let report = install(&s, &|args| sys.run(args));
        assert_eq!(report.code, 0, "{}", report.err);
        assert_eq!(fs::read_to_string(s.unit_path()).unwrap(), unit_text(&s.exe));
        assert_eq!(sys.calls(), vec!["daemon-reload".to_string(), format!("enable --now {UNIT}")]);
        assert!(report.out.contains("no linger") && report.out.contains(&s.events.display().to_string()), "{}", report.out);

        // Otra vez, igual: no hay por qué reiniciar.
        let sys = Fake::new(vec![]);
        assert_eq!(install(&s, &|args| sys.run(args)).code, 0);
        assert_eq!(sys.calls().len(), 2);
        // Con otra ruta, sí.
        let moved = Setup { exe: dir.join("other").join("cac-mcp"), ..setup(&dir) };
        let sys = Fake::new(vec![]);
        assert_eq!(install(&moved, &|args| sys.run(args)).code, 0);
        assert_eq!(sys.calls().last().unwrap(), &format!("try-restart {UNIT}"));
    }

    /// Roturas que atrapa: decir «instalado» cuando `enable` falló, o perder
    /// el motivo.
    #[test]
    fn a_failing_systemctl_is_a_failure_with_its_reason() {
        let dir = scratch("fails");
        let sys = Fake::new(vec![("enable", false, "")]);
        let report = install(&setup(&dir), &|args| sys.run(args));
        assert_eq!(report.code, 1);
        assert!(report.err.contains("enable --now") && report.err.contains("boom"), "{}", report.err);
    }

    /// Roturas que atrapa: instalar desde una AppImage (por su montaje, por
    /// `APPDIR` o por `APPIMAGE`), o negarse al binario suelto sólo porque
    /// heredó esas variables de otro programa.
    #[test]
    fn an_appimage_is_refused_but_a_standalone_binary_that_inherited_its_variables_is_not() {
        let dir = scratch("appimage");
        for (exe, appdir, image) in [
            ("/tmp/.mount_cacAbC123/usr/bin/cac", None, None),
            ("/opt/x/cac", Some("/opt/x"), None),
            ("/home/ana/cac.AppImage", None, Some("/home/ana/cac.AppImage")),
        ] {
            let s = Setup { exe: exe.into(), appdir: appdir.map(PathBuf::from), appimage: image.map(PathBuf::from), ..setup(&dir) };
            let sys = Fake::new(vec![]);
            let report = install(&s, &|args| sys.run(args));
            assert_eq!(report.code, 2, "{exe}");
            assert!(report.err.contains("standalone"), "{}", report.err);
            assert!(sys.calls().is_empty() && !s.unit_path().exists(), "no se toca nada: {exe}");
        }
        let inherited = Setup { appdir: Some("/tmp/.mount_other".into()), appimage: Some("/tmp/other.AppImage".into()), ..setup(&dir) };
        let sys = Fake::new(vec![]);
        assert_eq!(install(&inherited, &|args| sys.run(args)).code, 0);
    }

    /// Roturas que atrapa: instalar en macOS o Windows, otro código que 2, o
    /// no decir cómo correrlo a mano.
    #[test]
    fn on_macos_and_windows_install_says_no_with_code_two() {
        let dir = scratch("other-os");
        for os in ["macos", "windows"] {
            let s = Setup { os, ..setup(&dir) };
            let sys = Fake::new(vec![]);
            for report in [install(&s, &|a| sys.run(a)), uninstall(&s, &|a| sys.run(a))] {
                assert_eq!(report.code, 2);
                assert!(report.err.contains("not supported on this system yet") && report.err.contains("cac-mcp listen run"), "{}", report.err);
            }
            assert!(sys.calls().is_empty() && !s.unit_path().exists());
        }
    }

    /// Roturas que atrapa: que `uninstall` borre el fichero de eventos, que no
    /// diga dónde queda, que deje la unidad o que no recargue.
    #[test]
    fn uninstall_removes_the_unit_and_leaves_the_events() {
        let dir = scratch("uninstall");
        let s = setup(&dir);
        install(&s, &|_| Ok(Ran { ok: true, stdout: String::new(), stderr: String::new() }));
        fs::create_dir_all(s.events.parent().unwrap()).unwrap();
        fs::write(&s.events, "{\"seq\":1}\n").unwrap();
        let sys = Fake::new(vec![]);
        let report = uninstall(&s, &|args| sys.run(args));
        assert_eq!(report.code, 0, "{}", report.err);
        assert!(!s.unit_path().exists());
        assert_eq!(fs::read_to_string(&s.events).unwrap(), "{\"seq\":1}\n");
        assert!(report.out.contains(&s.events.display().to_string()), "{}", report.out);
        assert_eq!(sys.calls(), vec![format!("disable --now {UNIT}"), "daemon-reload".to_string()]);
        // Sin unidad no hay nada que parar.
        let sys = Fake::new(vec![]);
        assert_eq!(uninstall(&s, &|args| sys.run(args)).code, 0);
        assert!(sys.calls().is_empty());
    }

    /// Roturas que atrapa: no decir el cursor ni la última conexión, no dar el
    /// motivo cuando está parado (el de su fichero de estado y el de la
    /// unidad), o salir con 0 estando parado.
    #[test]
    fn status_says_the_cursor_the_last_connection_and_why_it_is_stopped() {
        let dir = scratch("status");
        let s = setup(&dir);
        let none = Fake::new(vec![]);
        let report = status(&s, &|args| none.run(args));
        assert_eq!(report.code, 1);
        assert!(report.out.contains("not installed") && report.out.contains("cursor: none yet") && report.out.contains("last connected: never"), "{}", report.out);

        install(&s, &|_| Ok(Ran { ok: true, stdout: String::new(), stderr: String::new() }));
        fs::create_dir_all(&s.state).unwrap();
        fs::write(s.state.join("listen.json"), r#"{"cursor":812}"#).unwrap();
        fs::write(
            listen::status_file(&s.state),
            r#"{"state":"unauthorized","reason":"cac answered 401 invalid-token: the token is not valid","lastConnectedAt":"2026-11-27T10:00:00Z","lastEventAt":"2026-11-27T10:05:00Z","retryInSeconds":300,"updatedAt":"2026-11-27T11:00:00Z"}"#,
        )
        .unwrap();
        let sys = Fake::new(vec![("is-active", true, "activating"), ("is-enabled", true, "enabled")]);
        let report = status(&s, &|args| sys.run(args));
        assert_eq!(report.code, 1, "{}", report.out);
        for needle in ["enabled, activating", "stopped: the unit is `activating`", "listener: unauthorized", "reason: cac answered 401 invalid-token", "retrying in: 300 s", "cursor: 812", "last connected: 2026-11-27T10:00:00Z", "last event: 2026-11-27T10:05:00Z"] {
            assert!(report.out.contains(needle), "{needle}\n{}", report.out);
        }
        let up = Fake::new(vec![("is-active", true, "active"), ("is-enabled", true, "enabled")]);
        let report = status(&s, &|args| up.run(args));
        assert_eq!(report.code, 0);
        assert!(!report.out.contains("stopped:"), "{}", report.out);
    }

    /// Roturas que atrapa: reiniciar sin que haya servicio (`try-restart` de
    /// una unidad que no existe da ruido), o no reiniciar el que sí está.
    #[test]
    fn an_update_restarts_the_service_only_if_it_exists() {
        let dir = scratch("restart");
        let s = setup(&dir);
        let sys = Fake::new(vec![]);
        assert_eq!(restart_after_update(Some(&s.unit_path()), &|args| sys.run(args)), None);
        assert_eq!(restart_after_update(None, &|args| sys.run(args)), None);
        assert!(sys.calls().is_empty());
        fs::create_dir_all(&s.unit_dir).unwrap();
        fs::write(s.unit_path(), "x").unwrap();
        let said = restart_after_update(Some(&s.unit_path()), &|args| sys.run(args)).unwrap();
        assert_eq!(sys.calls(), vec![format!("try-restart {UNIT}")]);
        assert!(said.contains("restarted"), "{said}");
    }
}
