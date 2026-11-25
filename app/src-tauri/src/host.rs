//! Lanzar programas del sistema desde la app sin el entorno de la AppImage.
//!
//! Dentro de una AppImage, la app lleva una veintena de variables que apuntan
//! a la imagen montada (`LD_LIBRARY_PATH`, `GTK_PATH`, `PYTHONHOME`, las de
//! GStreamer…), y cualquier programa que lance las hereda. El del sistema carga
//! entonces las bibliotecas de la imagen, de Ubuntu 22.04, en vez de las suyas:
//!
//! ```text
//! /bin/sh: symbol lookup error: /bin/sh: undefined symbol: rl_trim_arg_from_keyseq
//! ```
//!
//! Es el `bash` del sistema con la `libreadline` vieja de la imagen. Ya mordió
//! dos veces por separado: el relanzado tras actualizar (`relaunch.rs`) y el
//! Python de Ansible (`ansible.rs`). La tercera, el 21-oct-2026: tras un
//! `pacman -Syu`, ningún enlace externo abría («antes abría bien», jose). El
//! plugin de Tauri abre con `xdg-open`, que es un script de shell, y moría sin
//! decir nada. Medido lanzando `xdg-open` con el entorno de la app en marcha:
//! con él, el error de arriba; sin él, Firefox abre.
//!
//! Así que todo lo que la app lanza del sistema pasa por aquí: `command` en vez
//! de `Command::new`, y `open_url` / `open_path` en vez del plugin. Fuera de una
//! AppImage no hay nada que limpiar y esto no cambia nada.

use std::process::Command;

// Limpiar el entorno no necesita Tauri y lo usa también el MCP: vive en su
// crate (`mcp/src/host.rs`). Aquí queda lo que sí es de la app: el pty de la
// terminal y abrir enlaces y ficheros.
pub use cac_mcp::host::appimage_cleanup;
use cac_mcp::host::clean_with;

/// Lo mismo para un `CommandBuilder` de un pty (la terminal).
pub fn clean_pty(cmd: &mut portable_pty::CommandBuilder) {
    let env: Vec<(String, String)> = std::env::vars().collect();
    let (remove, path) = appimage_cleanup(&env);
    for k in remove {
        cmd.env_remove(k);
    }
    if let Some(p) = path {
        cmd.env("PATH", p);
    }
}

/// Qué enlaces se abren fuera: los de la web y el correo. Un `file://` o un
/// esquema raro que llegara de una página no se le pasa al sistema.
pub fn is_openable_url(url: &str) -> bool {
    let u = url.trim();
    u.starts_with("https://") || u.starts_with("http://") || u.starts_with("mailto:")
}

/// Con qué programas se intenta abrir `target`, en orden: los mismos que el
/// plugin `opener` (`open::commands`: `wslview` dentro de WSL, `xdg-open`,
/// `gio open`, `gnome-open`, `kde-open`), y al final el `xdg-open` que la
/// AppImage lleva dentro. Ese último es el que tiene una distro sin
/// `xdg-utils`: el plugin lo encontraba por el `PATH` de la imagen, y aquí ese
/// `PATH` se limpia, así que se nombra por su ruta.
#[cfg(target_os = "linux")]
fn launchers(target: &str, env: &[(String, String)]) -> Vec<Command> {
    let mut out = open::commands(target);
    let appdir = env.iter().find(|(k, _)| k == "APPDIR").map(|(_, v)| v.as_str());
    if let Some(dir) = appdir.filter(|d| !d.is_empty()) {
        let bundled = std::path::Path::new(dir).join("usr/bin/xdg-open");
        if bundled.exists() {
            let mut c = Command::new(bundled);
            c.arg(target);
            out.push(c);
        }
    }
    for c in &mut out {
        clean_with(c, env);
    }
    out
}

/// Lanza el primero de `launchers` que exista y espera un momento a ver si
/// muere al arrancar, que es como fallaba con el entorno de la imagen, para que
/// el error llegue a la página en vez de perderse. Si sigue vivo, ya está: con
/// Firefox cerrado, `xdg-open` no vuelve hasta que se cierre el navegador.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn launch_first(launchers: Vec<Command>) -> Result<(), String> {
    let mut missing = vec![];
    for mut cmd in launchers {
        let name = cmd.get_program().to_string_lossy().to_string();
        let child = cmd
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing.push(name);
                continue;
            }
            Err(e) => return Err(format!("Could not run {name}: {e}")),
        };
        for _ in 0..30 {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    let mut err = String::new();
                    if let Some(mut e) = child.stderr.take() {
                        use std::io::Read;
                        let _ = e.read_to_string(&mut err);
                    }
                    return Err(format!("{name} failed ({status}): {}", err.trim()));
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(e) => return Err(e.to_string()),
            }
        }
        // Sigue abriendo: que acabe solo, sin dejar un proceso zombi.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }
    Err(format!("Nothing to open it with: none of {} is installed", missing.join(", ")))
}

#[cfg(target_os = "linux")]
fn open_with_system(target: &str) -> Result<(), String> {
    let env: Vec<(String, String)> = std::env::vars().collect();
    launch_first(launchers(target, &env))
}

#[cfg(not(target_os = "linux"))]
fn open_with_system(target: &str) -> Result<(), String> {
    tauri_plugin_opener::open_path(target, None::<&str>).map_err(|e| e.to_string())
}

/// Abre un enlace en el navegador del sistema. Desde la página, en vez del
/// `openUrl` del plugin (ver arriba). En macOS y Windows sigue el plugin: allí
/// no hay AppImage, y nada cambia.
#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    if !is_openable_url(&url) {
        return Err("Only http(s) and mailto links are opened outside the app".into());
    }
    #[cfg(not(target_os = "linux"))]
    {
        return tauri_plugin_opener::open_url(url.trim(), None::<&str>).map_err(|e| e.to_string());
    }
    #[cfg(target_os = "linux")]
    tauri::async_runtime::spawn_blocking(move || open_with_system(url.trim()))
        .await
        .map_err(|e| e.to_string())?
}

/// Abre una ruta que escribió un agente en un pedido. Si no existe contesta
/// `not-found`, que la página convierte en un aviso en su idioma.
#[tauri::command]
pub async fn open_local_path(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !cac_mcp::host::expand_home(path.trim()).exists() {
            return Err("not-found".to_string());
        }
        cac_mcp::host::open_path(path.trim())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Abre un fichero con la app que el sistema tenga para él.
pub fn open_path(path: &std::path::Path) -> Result<(), String> {
    open_with_system(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nadie lanza un programa del sistema por fuera de `host`, ni aquí ni en
    /// el crate del MCP (que corre dentro de la misma AppImage con
    /// `cac --mcp`), ni en `ops/` (SSH y llavero, que la app enlaza). Rotura
    /// que detecta: un `Command::new("programa")` nuevo, que heredaría el
    /// entorno de la imagen y moriría igual que `xdg-open`.
    /// La misma prueba, con el mismo nombre, vive en `mcp/src/host.rs`, y la de
    /// `ops/` en su `lib.rs`: aquéllas son las que corren en el CI.
    #[test]
    fn every_system_program_is_launched_through_here() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut bad = cac_mcp::host::unclean_launches(&here.join("src"));
        bad.extend(cac_mcp::host::unclean_launches(&here.join("../../mcp/src")));
        bad.extend(cac_mcp::host::unclean_launches(&here.join("../../ops/src")));
        assert!(bad.is_empty(), "lanzan un programa sin limpiar el entorno: {bad:#?}");
    }

    /// Roturas que detectan: rendirse con el primer programa que no está
    /// (una distro sin `xdg-open` pero con `gio` dejaría de abrir), dar por
    /// bueno uno que muere al arrancar (el fallo que no se veía), o no decir
    /// nada cuando no hay ninguno.
    #[test]
    #[cfg(unix)]
    fn the_first_launcher_that_exists_is_used_and_its_failure_is_reported() {
        let cmd = |p: &str, args: &[&str]| {
            let mut c = Command::new(p);
            c.args(args);
            c
        };
        assert!(launch_first(vec![cmd("cac-no-existe-1", &[]), cmd("true", &[])]).is_ok());
        let err = launch_first(vec![cmd("cac-no-existe-1", &[]), cmd("sh", &["-c", "echo roto >&2; exit 127"])]).unwrap_err();
        assert!(err.contains("127") && err.contains("roto"), "{err}");
        let none = launch_first(vec![cmd("cac-no-existe-1", &[]), cmd("cac-no-existe-2", &[])]).unwrap_err();
        assert!(none.contains("cac-no-existe-1") && none.contains("cac-no-existe-2"), "{none}");
    }

    /// Roturas que detectan: perder los programas que probaba el plugin, o el
    /// `xdg-open` de la imagen (el único en una distro sin `xdg-utils`), o
    /// lanzarlos con el entorno de la imagen.
    #[test]
    #[cfg(target_os = "linux")]
    fn the_launchers_are_the_plugins_plus_the_one_in_the_image() {
        let dir = std::env::temp_dir().join(format!("cac-launchers-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("usr/bin")).unwrap();
        std::fs::write(dir.join("usr/bin/xdg-open"), "#!/bin/sh\n").unwrap();
        let appdir = dir.to_string_lossy().to_string();
        let env = vec![
            ("APPDIR".to_string(), appdir.clone()),
            ("LD_LIBRARY_PATH".to_string(), format!("{appdir}/usr/lib")),
        ];
        let all = launchers("https://x", &env);
        let names: Vec<String> = all.iter().map(|c| c.get_program().to_string_lossy().to_string()).collect();
        // A cada uno se le quita lo de la imagen.
        for c in &all {
            assert!(
                c.get_envs().any(|(k, v)| k == "LD_LIBRARY_PATH" && v.is_none()),
                "{:?} hereda el LD_LIBRARY_PATH de la imagen",
                c.get_program()
            );
        }
        for p in ["xdg-open", "gio", "gnome-open", "kde-open"] {
            assert!(names.iter().any(|n| n == p), "falta {p}: {names:?}");
        }
        assert_eq!(names.last().unwrap(), &dir.join("usr/bin/xdg-open").to_string_lossy().to_string());
        // Sin AppImage, sólo los del plugin.
        assert!(!launchers("https://x", &[]).iter().any(|c| c.get_program().to_string_lossy().contains("usr/bin")));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Rotura que detecta: pasarle al sistema un `file://` o un esquema
    /// cualquiera que llegue de una página.
    #[test]
    fn only_web_and_mail_links_are_opened() {
        for ok in ["https://github.com/x", "http://localhost:1420", "mailto:a@b.c", "  https://x "] {
            assert!(is_openable_url(ok), "{ok}");
        }
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "/home/rv/.ssh", "smb://x", ""] {
            assert!(!is_openable_url(bad), "{bad}");
        }
    }
}
