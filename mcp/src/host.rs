//! Lanzar programas del sistema sin el entorno de la AppImage.
//!
//! Dentro de una AppImage, el proceso lleva una veintena de variables que
//! apuntan a la imagen montada (`LD_LIBRARY_PATH`, `GTK_PATH`, `PYTHONHOME`,
//! las de GStreamer…), y cualquier programa que lance las hereda. El del
//! sistema carga entonces las bibliotecas de la imagen, de Ubuntu 22.04, en vez
//! de las suyas:
//!
//! ```text
//! /bin/sh: symbol lookup error: /bin/sh: undefined symbol: rl_trim_arg_from_keyseq
//! ```
//!
//! La historia entera (las tres veces que mordió) está en
//! `app/src-tauri/src/host.rs`, que es donde vive lo que sólo hace la app:
//! abrir enlaces y ficheros. Aquí está lo que no necesita Tauri, porque este
//! servidor también corre dentro de la AppImage (`cac --mcp`) y lanza `git`,
//! `gs`, `op`, `ssh-add` y Ansible. Todo lanzamiento pasa por `command`, nunca
//! por `Command::new("…")`. En el binario `cac-mcp` suelto no hay nada que
//! limpiar y esto no cambia nada.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// Lo que la AppImage mete en el entorno y no puede llegar a un programa del
/// sistema: las variables que apuntan dentro de `$APPDIR` y las de la propia
/// imagen. Devuelve qué quitar y el `PATH` sin carpetas de la imagen. Fuera de
/// una AppImage (sin `APPDIR`) no toca nada.
pub fn appimage_cleanup(env: &[(String, String)]) -> (Vec<String>, Option<String>) {
    let Some(appdir) = env.iter().find(|(k, _)| k == "APPDIR").map(|(_, v)| v.trim_end_matches('/').to_string()) else {
        return (vec![], None);
    };
    if appdir.is_empty() {
        return (vec![], None);
    }
    let mut remove: Vec<String> = env
        .iter()
        .filter(|(k, v)| k != "PATH" && v.contains(&appdir))
        .map(|(k, _)| k.clone())
        .collect();
    for k in ["APPDIR", "APPIMAGE", "ARGV0", "OWD", "PYTHONDONTWRITEBYTECODE"] {
        if env.iter().any(|(x, _)| x == k) && !remove.iter().any(|r| r == k) {
            remove.push(k.to_string());
        }
    }
    let path = env.iter().find(|(k, _)| k == "PATH").map(|(_, v)| {
        v.split(':').filter(|p| !p.is_empty() && !p.starts_with(&appdir)).collect::<Vec<_>>().join(":")
    });
    (remove, path)
}

/// Un programa del sistema, listo para lanzar con el entorno limpio.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    clean(&mut cmd);
    cmd
}

/// A dónde apunta una ruta escrita por una persona o un agente: `~/` es la
/// casa de quien corre esto.
pub fn expand_home(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|home| Path::new(&home).join(rest)).unwrap_or_else(|| path.into()),
        None => path.into(),
    }
}

/// Abre un fichero o carpeta con lo que el sistema tenga para ello. Devuelve
/// el error si no existe o no hay con qué abrirlo; no espera a que termine.
pub fn open_path(path: &str) -> Result<(), String> {
    let target = expand_home(path);
    if !target.exists() {
        return Err(format!("{} does not exist", target.display()));
    }
    #[cfg(target_os = "linux")]
    let mut cmd = command("xdg-open");
    #[cfg(target_os = "macos")]
    let mut cmd = command("open");
    #[cfg(windows)]
    let mut cmd = command("explorer");
    cmd.arg(&target).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("Could not open {}: {e}", target.display()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Limpia el entorno de un `Command` ya creado.
pub fn clean(cmd: &mut Command) {
    let env: Vec<(String, String)> = std::env::vars().collect();
    clean_with(cmd, &env);
}

/// `clean` con el entorno dado, en vez del de este proceso (para probarlo, y
/// para quien ya lo tiene leído).
pub fn clean_with(cmd: &mut Command, env: &[(String, String)]) {
    let (remove, path) = appimage_cleanup(env);
    for k in remove {
        cmd.env_remove(k);
    }
    if let Some(p) = path {
        cmd.env("PATH", p);
    }
}

/// Las líneas de código de `dir` que lanzan un programa por su nombre sin
/// pasar por aquí. Sólo mira el código, no las pruebas (lo que va tras
/// `#[cfg(test)]`), y se salta `host.rs`. Es la mitad que comparten los dos
/// guardianes `every_system_program_is_launched_through_here`, el de este crate
/// y el de la app: cada uno mira las dos carpetas.
#[doc(hidden)]
pub fn unclean_launches(dir: &Path) -> Vec<String> {
    let mut bad = vec![];
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) != Some("rs") || p.ends_with("host.rs") {
            continue;
        }
        seen += 1;
        let text = std::fs::read_to_string(&p).unwrap();
        let code = text.split("#[cfg(test)]").next().unwrap_or("");
        for (i, line) in code.lines().enumerate() {
            if line.contains("Command::new(\"") || line.contains("CommandBuilder::new(\"") && !code.contains("clean_pty") {
                bad.push(format!("{}:{}: {}", p.file_name().unwrap().to_string_lossy(), i + 1, line.trim()));
            }
        }
    }
    // Una carpeta sin fuentes es una ruta equivocada, no un «todo limpio».
    assert!(seen > 0, "{} no tiene fuentes que mirar", dir.display());
    bad
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que detecta: no expandir `~/` (abriría una ruta relativa que no
    /// existe), o expandir una `~` que no abre la ruta.
    #[test]
    fn a_tilde_path_starts_at_home() {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).expect("home");
        assert_eq!(expand_home("~/a/b.txt"), Path::new(&home).join("a/b.txt"));
        assert_eq!(expand_home("/tmp/a"), Path::new("/tmp/a"));
        assert_eq!(expand_home("~x/a"), Path::new("~x/a"));
    }

    /// Rotura que detecta: lanzar el abridor con una ruta que no existe (el
    /// sistema contesta con un aviso suyo o con nada) en vez de decirlo.
    #[test]
    fn a_missing_path_is_reported_instead_of_launched() {
        let error = open_path("/no/existe/cac-504.txt").unwrap_err();
        assert!(error.contains("does not exist"), "{error}");
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// Roturas que detecta: dejar `LD_LIBRARY_PATH` (el bash del sistema
    /// cargaría la readline vieja), dejar las de la imagen, tocar lo que no
    /// apunta a la imagen, o dejar sus carpetas en el PATH.
    #[test]
    fn inside_an_appimage_everything_that_points_at_it_goes() {
        let e = env(&[
            ("APPDIR", "/tmp/.mount_cac.X"),
            ("APPIMAGE", "/home/rv/cac.AppImage"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_cac.X/usr/lib:/tmp/.mount_cac.X/lib"),
            ("XDG_DATA_DIRS", "/tmp/.mount_cac.X/usr/share:/usr/share"),
            ("PATH", "/tmp/.mount_cac.X/usr/bin:/usr/bin:/bin"),
            ("HOME", "/home/rv"),
            ("SSH_AUTH_SOCK", "/home/rv/.1password/agent.sock"),
        ]);
        let (remove, path) = appimage_cleanup(&e);
        for k in ["LD_LIBRARY_PATH", "XDG_DATA_DIRS", "APPDIR", "APPIMAGE"] {
            assert!(remove.iter().any(|r| r == k), "{k} tiene que quitarse");
        }
        for k in ["HOME", "SSH_AUTH_SOCK", "PATH"] {
            assert!(!remove.iter().any(|r| r == k), "{k} no se toca");
        }
        assert_eq!(path.as_deref(), Some("/usr/bin:/bin"));
    }

    #[test]
    fn outside_an_appimage_nothing_changes() {
        assert_eq!(appimage_cleanup(&env(&[("HOME", "/h"), ("PATH", "/usr/bin")])), (vec![], None));
    }

    /// Nadie lanza un programa del sistema por fuera de aquí, ni en este crate
    /// ni en la app de escritorio, que lo usa por ruta. Rotura que detecta: un
    /// `Command::new("programa")` nuevo, que heredaría el entorno de la imagen
    /// y moriría igual que `xdg-open`. Mira las dos carpetas porque esta
    /// prueba es la que corre en el CI (`mcp.yml`); las de la app no corren.
    #[test]
    fn every_system_program_is_launched_through_here() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut bad = unclean_launches(&here.join("src"));
        bad.extend(unclean_launches(&here.join("../app/src-tauri/src")));
        assert!(bad.is_empty(), "lanzan un programa sin limpiar el entorno: {bad:#?}");
    }

    /// De punta a punta, con un fallo como el que se midió: una variable de la
    /// imagen que no deja arrancar a la shell. Limpia, la shell arranca.
    /// Rotura que detecta: que `clean` no le quite al proceso lo de la imagen.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_command_does_not_inherit_the_image_environment() {
        let dir = std::env::temp_dir().join(format!("cac-host-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Una «biblioteca» que no es una biblioteca: con ella en
        // LD_PRELOAD, el cargador se queja al arrancar cualquier programa.
        let fake = dir.join("libroto.so");
        std::fs::write(&fake, b"no soy un ELF").unwrap();
        let appdir = dir.to_string_lossy().to_string();
        let run = |clean_it: bool| {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg("exit 0");
            cmd.env("APPDIR", &appdir).env("LD_PRELOAD", fake.to_string_lossy().to_string());
            if clean_it {
                let env = vec![
                    ("APPDIR".to_string(), appdir.clone()),
                    ("LD_PRELOAD".to_string(), fake.to_string_lossy().to_string()),
                ];
                clean_with(&mut cmd, &env);
            }
            cmd.output().unwrap()
        };
        let dirty = run(false);
        assert!(String::from_utf8_lossy(&dirty.stderr).contains("libroto"), "el entorno sucio tenía que romperla: {dirty:?}");
        let clean = run(true);
        assert!(clean.status.success() && !String::from_utf8_lossy(&clean.stderr).contains("libroto"), "limpia, la shell no vio la imagen: {clean:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
