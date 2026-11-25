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

/// Por qué no se pudo mostrar una ruta.
#[derive(Debug, PartialEq, Eq)]
pub enum RevealError {
    NotFound,
    /// Una ruta de red (`//host/…`, `\\host\…`): ni mirar si existe, que ya
    /// contacta al servidor.
    Refused,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Mac,
    Windows,
}

pub const THIS_OS: Os = if cfg!(windows) {
    Os::Windows
} else if cfg!(target_os = "macos") {
    Os::Mac
} else {
    Os::Linux
};

/// Una ruta que empieza con dos separadores es de red en Windows (UNC), venga
/// escrita como venga; mirarla ya habla con el servidor (y le regala el hash
/// NTLM de quien la mira).
pub fn is_network_path(path: &str) -> bool {
    let mut chars = path.chars();
    matches!((chars.next(), chars.next()), (Some('/' | '\\'), Some('/' | '\\')))
}

fn file_url(path: &Path) -> String {
    let mut out = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// El programa y los argumentos que muestran `target` en el gestor de ficheros,
/// con el fichero seleccionado. Nunca lo abren: lo que el sistema asocia a un
/// `.jar`, un `.command` o un `.exe` es ejecutarlo.
pub fn reveal_command(os: Os, target: &Path) -> (&'static str, Vec<String>) {
    match os {
        Os::Linux => (
            "dbus-send",
            vec![
                "--session".into(),
                "--reply-timeout=3000".into(),
                "--dest=org.freedesktop.FileManager1".into(),
                "--type=method_call".into(),
                "/org/freedesktop/FileManager1".into(),
                "org.freedesktop.FileManager1.ShowItems".into(),
                format!("array:string:{}", file_url(target)),
                "string:".into(),
            ],
        ),
        Os::Mac => ("open", vec!["-R".into(), target.to_string_lossy().into_owned()]),
        Os::Windows => ("explorer", vec![format!("/select,{}", target.to_string_lossy())]),
    }
}

#[cfg(test)]
thread_local! {
    /// Lo que se habría lanzado, y qué programas fallan, sin lanzar nada.
    pub(crate) static LAUNCHED: std::cell::RefCell<Vec<(String, Vec<String>)>> = const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static FAILING: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Lanza `program`. `check`: esperar su salida y fallar si no fue buena (el
/// `explorer` de Windows sale con 1 aunque lo haya mostrado).
fn run(program: &str, args: &[String], check: bool) -> Result<(), String> {
    #[cfg(test)]
    {
        let _ = check;
        LAUNCHED.with(|l| l.borrow_mut().push((program.to_string(), args.to_vec())));
        return if FAILING.with(|f| f.borrow().iter().any(|p| p == program)) { Err(format!("{program} failed")) } else { Ok(()) };
    }
    #[cfg(not(test))]
    {
        let mut cmd = command(program);
        cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().map_err(|e| format!("{program}: {e}"))?;
        if check {
            let status = child.wait().map_err(|e| e.to_string())?;
            return if status.success() { Ok(()) } else { Err(format!("{program} exited with {status}")) };
        }
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

/// Abrir una carpeta con el gestor de ficheros del sistema (la de esta
/// máquina; la app de Tauri pasa la suya, que ya sabe qué probar).
pub fn open_dir(dir: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(windows)]
    let program = "explorer";
    run(program, &[dir.to_string_lossy().into_owned()], false)
}

/// Muestra una ruta de esta máquina en el gestor de ficheros, con el fichero
/// seleccionado. **Nunca la abre**: el texto de un pedido lo escribe un agente
/// y puede venir dirigido por contenido ajeno, y abrir un `.jar`, un `.exe` o
/// un enlace a un ejecutable lo ejecutaría con un clic. Primero se resuelven
/// los enlaces simbólicos (como `read_dropped_file`) y se muestra el destino
/// real. Una carpeta sí se abre, que es lo que se quiere ver. `open_dir` abre
/// la carpeta cuando el gestor no sabe seleccionar (en Linux, sin D-Bus).
pub fn reveal_path(path: &str, open_dir: &dyn Fn(&Path) -> Result<(), String>) -> Result<(), RevealError> {
    if is_network_path(path.trim()) {
        return Err(RevealError::Refused);
    }
    let canonical = std::fs::canonicalize(expand_home(path.trim())).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => RevealError::NotFound,
        _ => RevealError::Failed(e.to_string()),
    })?;
    let shown = canonical.to_string_lossy();
    let canonical = match shown.strip_prefix(r"\\?\") {
        Some(plain) if !plain.starts_with("UNC") => std::path::PathBuf::from(plain),
        _ => canonical.clone(),
    };
    if canonical.is_dir() {
        return open_dir(&canonical).map_err(RevealError::Failed);
    }
    let (program, args) = reveal_command(THIS_OS, &canonical);
    match run(program, &args, THIS_OS != Os::Windows) {
        Ok(()) => Ok(()),
        Err(_) if THIS_OS == Os::Linux => open_dir(canonical.parent().unwrap_or(Path::new("/"))).map_err(RevealError::Failed),
        Err(e) => Err(RevealError::Failed(e)),
    }
}

/// `reveal_path` con el gestor de esta máquina, para quien no es la app de Tauri.
pub fn reveal(path: &str) -> Result<(), RevealError> {
    reveal_path(path, &open_dir)
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

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-reveal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn reset() {
        LAUNCHED.with(|l| l.borrow_mut().clear());
        FAILING.with(|f| f.borrow_mut().clear());
    }

    fn launched() -> Vec<(String, Vec<String>)> {
        LAUNCHED.with(|l| l.borrow().clone())
    }

    /// Un enlace simbólico a un ejecutable se muestra, no se abre: lo que se
    /// lanza es el comando de mostrar con el destino real, y el abridor de
    /// carpetas no recibe ni el fichero ni el enlace.
    ///
    /// Roturas que detecta: pasarle el fichero (o el enlace) a `xdg-open`,
    /// `open` o `explorer` sin más, que lo ejecutaría; mostrar el enlace en
    /// vez de su destino; abrir la carpeta cuando el gestor sí supo mostrarlo.
    #[test]
    #[cfg(unix)]
    fn a_symlink_to_an_executable_is_revealed_never_opened() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        reset();
        let dir = scratch("link");
        let program = dir.join("run.sh");
        std::fs::write(&program, "#!/bin/sh\necho pwned\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = dir.join("innocent.txt");
        symlink(&program, &link).unwrap();

        let opened: std::cell::RefCell<Vec<std::path::PathBuf>> = Default::default();
        reveal_path(&link.to_string_lossy(), &|d| {
            opened.borrow_mut().push(d.to_path_buf());
            Ok(())
        })
        .unwrap();

        let real = std::fs::canonicalize(&program).unwrap();
        let (expected, args) = reveal_command(THIS_OS, &real);
        assert_eq!(launched(), vec![(expected.to_string(), args)]);
        assert!(opened.borrow().is_empty(), "el gestor sí supo mostrarlo: no se abre nada más");
        assert!(!launched().iter().any(|(_, args)| args.iter().any(|a| a.contains("innocent.txt"))), "el enlace no se muestra: se muestra su destino");
    }

    /// Sin D-Bus (o sin gestor que sepa seleccionar) se abre la carpeta que lo
    /// contiene, nunca el fichero. Una carpeta se abre ella misma.
    ///
    /// Roturas que detecta: caer a abrir el fichero cuando falla mostrarlo;
    /// abrir una carpeta que no es la del fichero; no abrir una carpeta.
    #[test]
    #[cfg(target_os = "linux")]
    fn when_the_file_manager_cannot_select_the_parent_folder_is_opened() {
        reset();
        FAILING.with(|f| f.borrow_mut().push("dbus-send".into()));
        let dir = scratch("fallback");
        let file = dir.join("a.jar");
        std::fs::write(&file, "x").unwrap();
        let opened: std::cell::RefCell<Vec<std::path::PathBuf>> = Default::default();
        let record = |d: &Path| {
            opened.borrow_mut().push(d.to_path_buf());
            Ok(())
        };
        reveal_path(&file.to_string_lossy(), &record).unwrap();
        let real_dir = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(*opened.borrow(), vec![real_dir.clone()]);
        reveal_path(&dir.to_string_lossy(), &record).unwrap();
        assert_eq!(*opened.borrow(), vec![real_dir.clone(), real_dir], "una carpeta se abre ella misma");
    }

    /// El comando de mostrar de cada sistema selecciona el fichero y no lo abre.
    ///
    /// Roturas que detecta: perder `-R` en macOS o `/select,` en Windows (sin
    /// ellos, `open` y `explorer` lo ejecutan); un D-Bus que no es ShowItems;
    /// una ruta con espacios o acentos sin escapar en la URL.
    #[test]
    fn each_system_selects_the_file_instead_of_opening_it() {
        let target = Path::new("/home/ana/mi carpeta/á.jar");
        assert_eq!(reveal_command(Os::Mac, target), ("open", vec!["-R".to_string(), "/home/ana/mi carpeta/á.jar".to_string()]));
        assert_eq!(reveal_command(Os::Windows, Path::new(r"C:\x\a.exe")), ("explorer", vec![r"/select,C:\x\a.exe".to_string()]));
        let (program, args) = reveal_command(Os::Linux, target);
        assert_eq!(program, "dbus-send");
        assert!(args.contains(&"org.freedesktop.FileManager1.ShowItems".to_string()));
        assert!(args.contains(&"array:string:file:///home/ana/mi%20carpeta/%C3%A1.jar".to_string()), "{args:?}");
    }

    /// Una ruta de red no se mira ni se muestra, y una que no existe se dice.
    ///
    /// Roturas que detecta: dejar pasar `//host/…` o `\\host\…` (mirar si
    /// existe ya contacta al servidor); confundir «no existe» con un fallo.
    #[test]
    fn network_paths_are_refused_and_missing_ones_are_reported() {
        reset();
        for path in ["//srv/share/x", r"\\srv\share", r"/\srv/x"] {
            assert_eq!(reveal_path(path, &|_| Ok(())), Err(RevealError::Refused), "{path}");
        }
        assert_eq!(reveal_path("/no/existe/cac-504.txt", &|_| Ok(())), Err(RevealError::NotFound));
        assert!(launched().is_empty());
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
