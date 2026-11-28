//! El LaunchAgent de macOS que corre `listen run` (command-and-control-568):
//! lo mismo que la unidad de systemd de `listen_service`, con launchd.
//!
//! Es un agente **de la persona** (`~/Library/LaunchAgents`, dominio
//! `gui/<uid>`), no un demonio del sistema: no pide root. **No lleva el token
//! ni `EnvironmentVariables`**: `listen run` lo lee del `.claude.json` en cada
//! conexión, igual que con systemd. Arranca al iniciar sesión (`RunAtLoad`) y
//! launchd lo levanta si muere (`KeepAlive`).
//!
//! Todo `launchctl` pasa por `host::command` (ver `listen_service`).

use std::fs;
use std::path::Path;

use crate::listen_service::{listener_report, Control, Ran, Report, Setup};

pub const LABEL: &str = "dev.guz-studio.cac-listen";

/// Lo que va dentro de un `<string>`: `&`, `<` y `>` escapados.
fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// El texto del plist. Sin token y sin `EnvironmentVariables`.
pub fn plist_text(exe: &Path, log: &Path) -> String {
    let exe = xml(&exe.to_string_lossy());
    let log = xml(&log.to_string_lossy());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{LABEL}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{exe}</string>\n    <string>listen</string>\n    <string>run</string>\n  </array>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <true/>\n  <key>ThrottleInterval</key>\n  <integer>5</integer>\n  <key>StandardOutPath</key>\n  <string>{log}</string>\n  <key>StandardErrorPath</key>\n  <string>{log}</string>\n</dict>\n</plist>\n"
    )
}

fn domain(setup: &Setup) -> String {
    format!("gui/{}", setup.uid)
}

fn target(setup: &Setup) -> String {
    format!("{}/{LABEL}", domain(setup))
}

fn failed(what: &str, ran: &Ran, plist: &Path) -> Report {
    Report::fail(1, format!("cac-mcp listen: `launchctl {what}` failed: {}. The agent is written at {}.", ran.stderr, plist.display()))
}

pub fn install(setup: &Setup, ctl: Control) -> Report {
    let plist = setup.plist_path();
    let text = plist_text(&setup.exe, &setup.log);
    let before = fs::read_to_string(&plist).ok();
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(&setup.agents_dir)?;
        if let Some(dir) = setup.log.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&plist, &text)
    };
    if let Err(e) = write() {
        return Report::fail(1, format!("cac-mcp listen: could not write {}: {e}", plist.display()));
    }
    let target = target(setup);
    let loaded = ctl(&["print", &target]).is_ok_and(|ran| ran.ok);
    let mut out = String::new();
    if loaded && before.as_deref() == Some(text.as_str()) {
        out.push_str(&format!("{LABEL} was already loaded with this plist; left running.\n"));
    } else {
        // Cargado con otro plist (otra ruta del binario): se descarga para que
        // el nuevo se tome, y `bootstrap` no se queje de que ya está.
        if loaded {
            let _ = ctl(&["bootout", &target]);
        }
        let plist_arg = plist.to_string_lossy().to_string();
        let domain = domain(setup);
        match ctl(&["bootstrap", &domain, &plist_arg]) {
            Ok(ran) if ran.ok => {}
            // launchctl viejo (antes de 10.10) o sin `bootstrap`: `load -w`.
            Ok(first) => match ctl(&["load", "-w", &plist_arg]) {
                Ok(ran) if ran.ok => {}
                _ => return failed(&format!("bootstrap {domain} {plist_arg}"), &first, &plist),
            },
            Err(e) => return Report::fail(1, format!("cac-mcp listen: could not run launchctl: {e}. The agent is written at {}.", plist.display())),
        }
        out.push_str(&format!("Installed {} and loaded it.\n", plist.display()));
    }
    out.push_str(&format!("It runs `{} listen run`, keeps the events in {} and logs to {}.\n", setup.exe.display(), setup.events.display(), setup.log.display()));
    if crate::listen::credentials().is_err() {
        out.push_str("Note: there is no cac token yet, so it will wait; connect Claude Code to cac (mcpServers.cac in .claude.json) and it picks it up by itself.\n");
    }
    out.push_str("It listens while you are logged in; when you come back it catches up from cac's journal.\nCheck it with `cac-mcp listen status`; remove it with `cac-mcp listen uninstall`.\n");
    Report::ok(out)
}

pub fn uninstall(setup: &Setup, ctl: Control) -> Report {
    let plist = setup.plist_path();
    if !plist.exists() {
        return Report::ok(format!("Nothing to remove: {} is not installed. The events file is at {}.\n", plist.display(), setup.events.display()));
    }
    let mut notes = String::new();
    match ctl(&["bootout", &target(setup)]) {
        Ok(ran) if ran.ok => {}
        Ok(ran) => notes.push_str(&format!("Note: `launchctl bootout` said: {}\n", ran.stderr)),
        Err(e) => notes.push_str(&format!("Note: could not run launchctl: {e}\n")),
    }
    if let Err(e) = fs::remove_file(&plist) {
        return Report::fail(1, format!("cac-mcp listen: could not remove {}: {e}", plist.display()));
    }
    Report::ok(format!("{notes}Removed {LABEL}. The events file stays at {} (delete it if you do not want it).\n", setup.events.display()))
}

/// La palabra de `state = running` en la salida de `launchctl print`.
fn state_of(printed: &str) -> Option<String> {
    printed.lines().find_map(|line| line.trim().strip_prefix("state = ").map(|s| s.trim().to_string()))
}

pub fn status(setup: &Setup, ctl: Control) -> Report {
    let plist = setup.plist_path();
    let mut out = String::new();
    let mut running = false;
    if !plist.exists() {
        out.push_str("unit: not installed (`cac-mcp listen install`)\n");
    } else {
        match ctl(&["print", &target(setup)]) {
            Ok(ran) if ran.ok => {
                let state = state_of(&ran.stdout);
                running = state.as_deref() == Some("running");
                out.push_str(&format!("unit: {} (loaded, {})\n", plist.display(), state.as_deref().unwrap_or("unknown")));
                if !running {
                    out.push_str(&format!("stopped: launchd says `{}`; see {}\n", state.as_deref().unwrap_or("unknown"), setup.log.display()));
                }
            }
            _ => out.push_str(&format!("unit: {} (not loaded); load it with `cac-mcp listen install`\n", plist.display())),
        }
    }
    listener_report(setup, out, running)
}

/// `cac-mcp update` acabó bien: si el agente está instalado, lo reinicia para
/// que corra la versión nueva.
/// Sin `Setup`: tras actualizar, el binario que corre ya fue reemplazado y no
/// se puede preguntar por su propia ruta.
pub fn restart_after_update(plist: &Path, uid: &str, ctl: Control) -> Option<String> {
    if !plist.exists() {
        return None;
    }
    let target = format!("gui/{uid}/{LABEL}");
    Some(match ctl(&["kickstart", "-k", &target]) {
        Ok(ran) if ran.ok => format!("The listener agent ({LABEL}) was restarted on the new version."),
        Ok(ran) => format!("Could not restart {LABEL}: {}. Run `launchctl kickstart -k {target}`.", ran.stderr),
        Err(e) => format!("Could not restart {LABEL}: {e}. Run `launchctl kickstart -k {target}`."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen_service::{install as install_any, status as status_any, uninstall as uninstall_any, Setup};
    use std::cell::RefCell;
    use std::io;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-mcp-launchd-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup(dir: &Path) -> Setup {
        Setup {
            os: "macos",
            unit_dir: dir.join("config").join("systemd").join("user"),
            agents_dir: dir.join("home").join("Library").join("LaunchAgents"),
            log: dir.join("home").join("Library").join("Logs").join("cac-listen.log"),
            uid: "501".into(),
            exe: dir.join("bin").join("cac-mcp"),
            appdir: None,
            appimage: None,
            events: dir.join("cache").join("cac").join("events.jsonl"),
            state: dir.join("state"),
        }
    }

    /// Un `launchctl` de mentira con estado: `bootstrap` carga, `bootout`
    /// descarga, `print` contesta según esté cargado. Nada toca el launchd de
    /// esta máquina.
    struct Fake {
        calls: RefCell<Vec<String>>,
        loaded: RefCell<bool>,
        bootstrap_works: bool,
        state: &'static str,
    }

    impl Fake {
        fn new() -> Self {
            Fake { calls: RefCell::new(vec![]), loaded: RefCell::new(false), bootstrap_works: true, state: "running" }
        }
        fn run(&self, args: &[&str]) -> io::Result<Ran> {
            self.calls.borrow_mut().push(args.join(" "));
            let fail = |why: &str| Ok(Ran { ok: false, stdout: String::new(), stderr: why.into() });
            let ok = |out: &str| Ok(Ran { ok: true, stdout: out.into(), stderr: String::new() });
            match args[0] {
                "print" if *self.loaded.borrow() => ok(&format!("gui/501/{LABEL} = {{\n\tstate = {}\n\tpid = 7\n}}", self.state)),
                "print" => fail("Could not find service"),
                "bootstrap" if self.bootstrap_works => {
                    *self.loaded.borrow_mut() = true;
                    ok("")
                }
                "bootstrap" => fail("boom"),
                "load" => {
                    *self.loaded.borrow_mut() = true;
                    ok("")
                }
                "bootout" => {
                    *self.loaded.borrow_mut() = false;
                    ok("")
                }
                _ => ok(""),
            }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    /// Roturas que atrapa: meter el token o `EnvironmentVariables` en el
    /// plist, perder `RunAtLoad` o `KeepAlive`, poner el binario sin los
    /// argumentos `listen run` (o en otro orden), no escapar `&` en una ruta
    /// (el plist dejaría de ser XML), o mandar los logs a ningún sitio.
    #[test]
    fn the_plist_has_the_binary_the_arguments_and_no_secret() {
        let text = plist_text(Path::new("/Users/ana/my & bin/cac-mcp"), Path::new("/Users/ana/Library/Logs/cac-listen.log"));
        let args = "<key>ProgramArguments</key>\n  <array>\n    <string>/Users/ana/my &amp; bin/cac-mcp</string>\n    <string>listen</string>\n    <string>run</string>\n  </array>";
        assert!(text.contains(args), "{text}");
        assert!(text.contains(&format!("<key>Label</key>\n  <string>{LABEL}</string>")));
        assert!(text.contains("<key>RunAtLoad</key>\n  <true/>") && text.contains("<key>KeepAlive</key>\n  <true/>"), "{text}");
        assert_eq!(text.matches("<string>/Users/ana/Library/Logs/cac-listen.log</string>").count(), 2, "stdout y stderr");
        assert!(!text.contains("EnvironmentVariables") && !text.to_lowercase().contains("token"), "{text}");
    }

    /// Roturas que atrapa: escribir el plist en otro sitio o con otro nombre,
    /// cargarlo en otro dominio que el del usuario (`gui/<uid>`), no cargarlo
    /// (sin `bootstrap` no arranca), y no caer a `load -w` cuando `bootstrap`
    /// no existe.
    #[test]
    fn install_writes_the_agent_and_loads_it_in_the_user_domain() {
        let dir = scratch("install");
        let s = setup(&dir);
        let ctl = Fake::new();
        let report = install_any(&s, &|a| ctl.run(a));
        assert_eq!(report.code, 0, "{}", report.err);
        let plist = s.agents_dir.join(format!("{LABEL}.plist"));
        assert_eq!(fs::read_to_string(&plist).unwrap(), plist_text(&s.exe, &s.log));
        assert_eq!(ctl.calls(), vec![format!("print gui/501/{LABEL}"), format!("bootstrap gui/501 {}", plist.display())]);
        assert!(report.out.contains(&s.events.display().to_string()) && report.out.contains(&s.log.display().to_string()), "{}", report.out);

        let old = Fake { bootstrap_works: false, ..Fake::new() };
        let dir = scratch("install-old");
        let s = setup(&dir);
        assert_eq!(install_any(&s, &|a| old.run(a)).code, 0);
        assert_eq!(old.calls().last().unwrap(), &format!("load -w {}", s.plist_path().display()));
    }

    /// Roturas que atrapa: reinstalar con el mismo plist y reiniciar lo que ya
    /// corre, o con otra ruta del binario no descargarlo antes (el nuevo
    /// plist no se tomaría, o `bootstrap` diría que ya está).
    #[test]
    fn installing_again_is_harmless_and_a_moved_binary_reloads() {
        let dir = scratch("again");
        let s = setup(&dir);
        let ctl = Fake::new();
        install_any(&s, &|a| ctl.run(a));
        let again = Fake { loaded: RefCell::new(true), ..Fake::new() };
        let report = install_any(&s, &|a| again.run(a));
        assert_eq!(report.code, 0, "{}", report.err);
        assert_eq!(again.calls(), vec![format!("print gui/501/{LABEL}")], "igual: no se toca");

        let moved = Setup { exe: dir.join("other").join("cac-mcp"), ..setup(&dir) };
        let ctl = Fake { loaded: RefCell::new(true), ..Fake::new() };
        assert_eq!(install_any(&moved, &|a| ctl.run(a)).code, 0);
        let calls = ctl.calls();
        assert_eq!(calls[1], format!("bootout gui/501/{LABEL}"));
        assert!(calls[2].starts_with("bootstrap gui/501 "), "{calls:?}");
        assert!(fs::read_to_string(s.plist_path()).unwrap().contains("/other/cac-mcp"));
    }

    /// Roturas que atrapa: decir «instalado» cuando ni `bootstrap` ni
    /// `load -w` cargaron, o perder el motivo.
    #[test]
    fn a_failing_launchctl_is_a_failure_with_its_reason() {
        let dir = scratch("fails");
        let ctl = Fake { bootstrap_works: false, ..Fake::new() };
        let report = install_any(&setup(&dir), &|a| {
            if a[0] == "load" { Ok(Ran { ok: false, stdout: String::new(), stderr: "no".into() }) } else { ctl.run(a) }
        });
        assert_eq!(report.code, 1);
        assert!(report.err.contains("bootstrap") && report.err.contains("boom"), "{}", report.err);
    }

    /// Roturas que atrapa: no descargar el agente, dejar el plist, borrar el
    /// fichero de eventos, no decir dónde queda, o llamar a `launchctl` cuando
    /// no hay nada instalado.
    #[test]
    fn uninstall_unloads_the_agent_removes_the_plist_and_leaves_the_events() {
        let dir = scratch("uninstall");
        let s = setup(&dir);
        install_any(&s, &|a| Fake::new().run(a));
        fs::create_dir_all(s.events.parent().unwrap()).unwrap();
        fs::write(&s.events, "{\"seq\":1}\n").unwrap();
        let ctl = Fake::new();
        let report = uninstall_any(&s, &|a| ctl.run(a));
        assert_eq!(report.code, 0, "{}", report.err);
        assert!(!s.plist_path().exists());
        assert_eq!(fs::read_to_string(&s.events).unwrap(), "{\"seq\":1}\n");
        assert!(report.out.contains(&s.events.display().to_string()), "{}", report.out);
        assert_eq!(ctl.calls(), vec![format!("bootout gui/501/{LABEL}")]);
        let ctl = Fake::new();
        assert_eq!(uninstall_any(&s, &|a| ctl.run(a)).code, 0);
        assert!(ctl.calls().is_empty());
    }

    /// Roturas que atrapa: dar por vivo un agente cargado pero parado, no decir
    /// el motivo, no distinguir «sin plist» de «con plist sin cargar», y salir
    /// con 0 sin que corra.
    #[test]
    fn status_says_if_the_agent_is_loaded_and_running() {
        let dir = scratch("status");
        let s = setup(&dir);
        let none = Fake::new();
        let report = status_any(&s, &|a| none.run(a));
        assert_eq!(report.code, 1);
        assert!(report.out.contains("not installed") && none.calls().is_empty(), "{}", report.out);

        install_any(&s, &|a| Fake::new().run(a));
        let unloaded = Fake::new();
        let report = status_any(&s, &|a| unloaded.run(a));
        assert_eq!(report.code, 1);
        assert!(report.out.contains("not loaded"), "{}", report.out);

        let up = Fake { loaded: RefCell::new(true), ..Fake::new() };
        let report = status_any(&s, &|a| up.run(a));
        assert_eq!(report.code, 0, "{}", report.out);
        assert!(report.out.contains("loaded, running") && !report.out.contains("stopped:"), "{}", report.out);

        let waiting = Fake { loaded: RefCell::new(true), state: "waiting", ..Fake::new() };
        let report = status_any(&s, &|a| waiting.run(a));
        assert_eq!(report.code, 1);
        assert!(report.out.contains("loaded, waiting") && report.out.contains("stopped: launchd says `waiting`"), "{}", report.out);
    }

    /// Roturas que atrapa: reiniciar sin que haya agente, o no reiniciar (con
    /// `kickstart -k`) el que sí está.
    #[test]
    fn an_update_restarts_the_agent_only_if_it_exists() {
        let dir = scratch("restart");
        let s = setup(&dir);
        let ctl = Fake::new();
        assert_eq!(restart_after_update(&s.plist_path(), &s.uid, &|a| ctl.run(a)), None);
        assert!(ctl.calls().is_empty());
        fs::create_dir_all(&s.agents_dir).unwrap();
        fs::write(s.plist_path(), "x").unwrap();
        let said = restart_after_update(&s.plist_path(), &s.uid, &|a| ctl.run(a)).unwrap();
        assert_eq!(ctl.calls(), vec![format!("kickstart -k gui/501/{LABEL}")]);
        assert!(said.contains("restarted"), "{said}");
    }
}
