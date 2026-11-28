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
    /// El `PATH` con el que se busca `xdg-mime` (uno de mentira). Sin él, uno
    /// vacío: ninguna prueba depende del `xdg-mime` de la máquina.
    pub(crate) static MIME_PATH: std::cell::RefCell<Option<std::ffi::OsString>> = const { std::cell::RefCell::new(None) };
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
/// real. Una carpeta sí se abre, que es lo que se quiere ver (salvo en macOS: ver `opens_folders`). `open_dir` abre
/// la carpeta cuando el gestor no sabe seleccionar (en Linux, sin D-Bus).
pub fn reveal_path(path: &str, open_dir: &dyn Fn(&Path) -> Result<(), String>) -> Result<(), RevealError> {
    if is_network_path(path.trim()) {
        return Err(RevealError::Refused);
    }
    let canonical = std::fs::canonicalize(expand_home(path.trim())).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => RevealError::NotFound,
        _ => RevealError::Failed(e.to_string()),
    })?;
    let canonical = without_verbatim_prefix(&canonical);
    if canonical.is_dir() && opens_folders(THIS_OS) {
        return open_dir(&canonical).map_err(RevealError::Failed);
    }
    let (program, args) = reveal_command(THIS_OS, &canonical);
    match run(program, &args, THIS_OS != Os::Windows) {
        Ok(()) => Ok(()),
        Err(_) if THIS_OS == Os::Linux => open_dir(canonical.parent().unwrap_or(Path::new("/"))).map_err(RevealError::Failed),
        Err(e) => Err(RevealError::Failed(e)),
    }
}

/// En Windows, `canonicalize` devuelve `\\?\C:\…`, que `explorer` no
/// entiende: se le quita ese prefijo (salvo a una ruta de red, `\\?\UNC\…`).
fn without_verbatim_prefix(path: &Path) -> std::path::PathBuf {
    let shown = path.to_string_lossy();
    match shown.strip_prefix(r"\\?\") {
        Some(plain) if !plain.starts_with("UNC") => std::path::PathBuf::from(plain),
        _ => path.to_path_buf(),
    }
}

/// Lo que se abre con su programa al pulsar su ruta en un pedido
/// (command-and-control-567: jose quiere el `apply.sql` en su editor, no en
/// Thunar). Sólo documentos: texto, PDF, imágenes. Lo demás (un `.jar`, un
/// `.sh`, un `.desktop`, algo sin extensión) se sigue mostrando, porque para el
/// sistema abrirlo es ejecutarlo; y nada que abra un navegador (`.html`,
/// `.htm`, `.xhtml`, `.xml`, `.svg`), que corre el `<script>` que lleve.
const OPENS_WITH_ITS_PROGRAM: &[&str] = &[
    "sql", "txt", "md", "markdown", "json", "yaml", "yml", "toml", "csv", "tsv", "log", "ini", "conf", "diff", "patch",
    "pdf", "png", "jpg", "jpeg", "gif", "webp",
];

/// Si un fichero (ya resuelto) se abre con su programa: un documento por su
/// extensión, y que no se pueda ejecutar (un `notas.txt` con permiso de
/// ejecución lo correría un gestor que lo trate como script).
pub fn opens_with_its_program(real: &Path) -> bool {
    let document = real.extension().and_then(|e| e.to_str()).is_some_and(|e| OPENS_WITH_ITS_PROGRAM.contains(&e.to_ascii_lowercase().as_str()));
    #[cfg(unix)]
    let runnable = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(real).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(true)
    };
    #[cfg(not(unix))]
    let runnable = false;
    document && real.is_file() && !runnable
}

/// El programa que abre `target` con lo que el sistema le asocia.
pub fn open_command(os: Os, target: &Path) -> (&'static str, Vec<String>) {
    let target = without_verbatim_prefix(target).to_string_lossy().into_owned();
    match os {
        Os::Linux => ("xdg-open", vec![target]),
        Os::Mac => ("open", vec![target]),
        Os::Windows => ("explorer", vec![target]),
    }
}

/// Qué hizo pulsar una ruta.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    WithItsProgram,
    /// Se mostró en la carpeta. Con motivo cuando era un documento por su
    /// extensión pero no se pudo fiar de su contenido; sin él, cuando no era
    /// un documento (un ejecutable, un `.html`), que es lo de siempre.
    Revealed(Option<NotOpened>),
}

/// Por qué un documento por su extensión se mostró en vez de abrirse.
#[derive(Debug, PartialEq, Eq)]
pub enum NotOpened {
    /// Por su contenido es de otro tipo (`text/html` en un `.sql`).
    ContentType(String),
    /// No se pudo preguntar el tipo (sin `xdg-mime`, o falló).
    TypeUnknown,
}

/// Los tipos de texto que se abren con su programa: los de un editor. Nada
/// de `text/html`, `application/xhtml+xml`, `image/svg+xml` ni XML, que van
/// al navegador y corren su `<script>`, ni de scripts.
const TEXT_TYPES: &[&str] = &[
    "text/plain", "text/markdown", "text/x-markdown", "text/csv", "text/x-csv", "text/tab-separated-values", "text/x-log",
    "text/x-patch", "text/x-diff", "text/x-sql", "application/sql", "application/x-sql", "application/json",
    "application/yaml", "application/x-yaml", "application/toml", "application/x-toml",
];

/// Si el tipo que da el contenido es el de un documento de esa extensión.
pub fn content_matches_extension(ext: &str, mime: &str) -> bool {
    match ext.to_ascii_lowercase().as_str() {
        "pdf" => mime == "application/pdf",
        "png" => mime == "image/png",
        "jpg" | "jpeg" => mime == "image/jpeg",
        "gif" => mime == "image/gif",
        "webp" => mime == "image/webp",
        // Un fichero vacío no tiene nada que ejecutar: se abre si su extensión es de texto.
        _ => TEXT_TYPES.contains(&mime) || mime == "inode/x-empty",
    }
}

/// El tipo que le da `xdg-mime` a un fichero. Mira el contenido, igual que
/// `xdg-open` al abrir (en un escritorio sin GNOME ni KDE, como el bspwm de
/// jose, va por esa rama): un `.sql` que empieza con `<html…>` es
/// `text/html`, y `xdg-open` se lo da a Firefox. `None` si no se pudo
/// preguntar.
fn mime_by_content(target: &Path) -> Option<String> {
    let mut cmd = command("xdg-mime");
    #[cfg(test)]
    cmd.env("PATH", MIME_PATH.with(|m| m.borrow().clone()).unwrap_or_default());
    let out = cmd
        .args(["query", "filetype"])
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mime = String::from_utf8(out.stdout).ok()?.trim().to_ascii_lowercase();
    (!mime.is_empty() && !mime.contains(char::is_whitespace)).then_some(mime)
}

/// Si un documento (por su extensión) se puede abrir. Sólo en Linux se
/// pregunta por el contenido: `open` (macOS) y `explorer` (Windows) eligen
/// el programa por la extensión, así que un `.sql` va al editor del `.sql`
/// aunque dentro lleve HTML. Sin respuesta, no se abre: cerrado por defecto.
fn content_refusal(os: Os, real: &Path) -> Option<NotOpened> {
    if os != Os::Linux {
        return None;
    }
    let ext = real.extension().and_then(|e| e.to_str()).unwrap_or_default();
    match mime_by_content(real) {
        Some(mime) if content_matches_extension(ext, &mime) => None,
        Some(mime) => Some(NotOpened::ContentType(mime)),
        None => Some(NotOpened::TypeUnknown),
    }
}

/// Pulsar una ruta de un pedido: un documento se abre con su programa (el
/// destino real, no el enlace); una carpeta, como carpeta; lo demás se muestra
/// en la carpeta (`reveal_path`), nunca se abre. En Linux, un documento cuyo
/// contenido es de otro tipo también se muestra, con el motivo. Queda el
/// hueco entre mirar y abrir (alguien que reescriba el fichero en ese
/// instante): para eso ya tiene que poder escribir en el disco de quien pulsa.
pub fn open_path_with(path: &str, open_dir: &dyn Fn(&Path) -> Result<(), String>) -> Result<Opened, RevealError> {
    if is_network_path(path.trim()) {
        return Err(RevealError::Refused);
    }
    let canonical = std::fs::canonicalize(expand_home(path.trim())).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => RevealError::NotFound,
        _ => RevealError::Failed(e.to_string()),
    })?;
    if !opens_with_its_program(&canonical) {
        return reveal_path(path, open_dir).map(|()| Opened::Revealed(None));
    }
    if let Some(why) = content_refusal(THIS_OS, &canonical) {
        return reveal_path(path, open_dir).map(|()| Opened::Revealed(Some(why)));
    }
    let (program, args) = open_command(THIS_OS, &canonical);
    run(program, &args, false).map_err(RevealError::Failed)?;
    Ok(Opened::WithItsProgram)
}

/// `open_path_with` con el gestor de esta máquina.
pub fn open_path(path: &str) -> Result<Opened, RevealError> {
    open_path_with(path, &open_dir)
}

/// Si una carpeta se puede abrir ella misma. En macOS no: una `.app` es una
/// carpeta y `open` la lanzaría, así que allí también se muestra con `-R`.
pub fn opens_folders(os: Os) -> bool {
    os != Os::Mac
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

/// El texto de un fichero sin su módulo de pruebas (`#[cfg(test)]` + `mod x {`
/// hasta su `}` en columna 0), con las líneas quitadas en blanco para que los
/// números no se muevan. Un `#[cfg(test)]` suelto (un `use`, un `const`, un
/// `mod x;`) no corta nada: cortar en el primero dejaba sin mirar casi todo
/// el fichero de `main.rs`, `theme.rs`… Lo usan los guardianes que leen las
/// fuentes.
#[doc(hidden)]
pub fn production_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut skipping = false;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if skipping {
            skipping = line != "}";
            out.push('\n');
            continue;
        }
        if line.trim_end() == "#[cfg(test)]" && lines.peek().is_some_and(|next| next.starts_with("mod ") && next.trim_end().ends_with('{')) {
            skipping = true;
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Las líneas de código de `dir` que lanzan un programa por su nombre sin
/// pasar por aquí. Sólo mira el código, no el módulo de pruebas, y se salta `host.rs`. Es la mitad que comparten los dos
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
        let code = production_code(&text);
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

    /// Rotura que detecta: cortar el fichero en el primer `#[cfg(test)]` (el
    /// código que sigue a un `use` de pruebas dejaría de mirarse), o no quitar
    /// el módulo de pruebas (sus rutas falsas darían falsos avisos).
    #[test]
    fn production_code_drops_only_the_test_module() {
        let text = "#[cfg(test)]\nuse x;\nlet a = 1;\n#[cfg(test)]\nmod tests {\n    let bad = 2;\n}\nlet after = 3;\n";
        let code = production_code(text);
        assert!(code.contains("let a = 1;") && code.contains("let after = 3;"), "{code}");
        assert!(!code.contains("bad"), "{code}");
        assert_eq!(code.lines().count(), text.lines().count(), "las líneas se mueven");
    }

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

    /// Pulsar la ruta de un `.sql` lo abre con su programa (el destino real);
    /// un ejecutable, aunque se llame `.sql` o lo apunte un enlace con nombre
    /// de documento, se sigue mostrando.
    ///
    /// Roturas que detecta: revelar el documento en vez de abrirlo; abrir el
    /// enlace en vez de su destino; abrir un `.sh`, uno sin extensión o un
    /// `.sql` ejecutable.
    #[test]
    #[cfg(unix)]
    fn a_document_is_opened_with_its_program_and_anything_runnable_is_only_revealed() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        reset();
        let dir = scratch("open");
        let sql = dir.join("apply.sql");
        std::fs::write(&sql, "select 1;").unwrap();
        let alias = dir.join("alias.sql");
        symlink(&sql, &alias).unwrap();
        fake_xdg_mime(&dir, FAKE_XDG_MIME);
        assert_eq!(open_path_with(&alias.to_string_lossy(), &|_| Ok(())), Ok(Opened::WithItsProgram));
        let real = std::fs::canonicalize(&sql).unwrap();
        let (program, args) = open_command(THIS_OS, &real);
        assert_eq!(launched(), vec![(program.to_string(), args)]);

        let script = dir.join("run.sh");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        let bare = dir.join("Makefile");
        std::fs::write(&bare, "all:\n").unwrap();
        let runnable_sql = dir.join("tricky.sql");
        std::fs::write(&runnable_sql, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&runnable_sql, std::fs::Permissions::from_mode(0o755)).unwrap();
        let doc_link = dir.join("notes.txt");
        symlink(&script, &doc_link).unwrap();
        // Lo que abre un navegador corre su `<script>`: se muestra.
        let mut browser = vec![];
        for name in ["page.xml", "icon.svg", "page.xhtml", "page.htm", "page.html", "PAGE.XML"] {
            let p = dir.join(name);
            std::fs::write(&p, "<html xmlns=\"http://www.w3.org/1999/xhtml\"><script>x()</script></html>").unwrap();
            browser.push(p);
        }
        for target in [&script, &bare, &runnable_sql, &doc_link].into_iter().chain(browser.iter()) {
            reset();
            assert_eq!(open_path_with(&target.to_string_lossy(), &|_| Ok(())), Ok(Opened::Revealed(None)), "{}", target.display());
            let real = std::fs::canonicalize(target).unwrap();
            assert_eq!(launched(), vec![{
                let (p, a) = reveal_command(THIS_OS, &real);
                (p.to_string(), a)
            }], "{} se muestra, no se abre", target.display());
        }
        assert_eq!(open_path_with("//srv/x.sql", &|_| Ok(())), Err(RevealError::Refused));
    }

    /// Un `xdg-mime` de mentira, que decide por el contenido como el de
    /// verdad: HTML, SVG o XHTML por lo que lleva dentro, aunque se llame
    /// `.sql`; un `.sql` es `application/sql`, y lo demás, `text/plain`.
    const FAKE_XDG_MIME: &str = r#"#!/bin/sh
[ "$1 $2" = "query filetype" ] || exit 2
if grep -q '<svg' "$3"; then echo image/svg+xml
elif grep -q 'xhtml' "$3"; then echo application/xhtml+xml
elif grep -q '<html' "$3"; then echo text/html
elif [ "${3##*.}" = sql ]; then echo application/sql
else echo text/plain; fi
"#;

    /// Pone `script` como `xdg-mime` en una carpeta propia y lo deja primero
    /// en el `PATH` con el que se busca (`MIME_PATH`, de este hilo).
    #[cfg(unix)]
    fn fake_xdg_mime(dir: &Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("fake-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let program = bin.join("xdg-mime");
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        MIME_PATH.with(|m| *m.borrow_mut() = Some(format!("{}:/usr/bin:/bin", bin.display()).into()));
    }

    /// En Linux, `xdg-open` elige el programa por el contenido: un `.sql`,
    /// `.txt` o `.md` que lleva `<html…><script>` lo abre el navegador. Antes
    /// de abrir se pregunta a `xdg-mime`, y lo que no es un documento de su
    /// extensión sólo se muestra, con el tipo que dio. Sin `xdg-mime`, o si
    /// falla, también se muestra.
    ///
    /// Roturas que detecta: abrir sin preguntar el tipo; abrir un HTML, un
    /// XHTML o un SVG disfrazado; aceptar cualquier `text/*` (`text/html`
    /// lo es); abrir una imagen cuyo contenido es texto; abrir cuando
    /// `xdg-mime` no está o falla (aunque escriba un tipo); y, al revés,
    /// mostrar un `.sql` o un `.txt` de verdad.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_document_whose_content_is_a_web_page_is_only_revealed() {
        reset();
        let dir = scratch("mime");
        fake_xdg_mime(&dir, FAKE_XDG_MIME);
        let write = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        let page = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><script>x()</script></html>";
        for (file, mime) in [
            (write("apply.sql", "<html><script>x()</script></html>"), "text/html"),
            (write("notes.txt", "<!doctype html><html><script>x()</script>"), "text/html"),
            (write("README.md", page), "application/xhtml+xml"),
            (write("data.csv", "<svg onload=\"x()\"></svg>"), "image/svg+xml"),
            (write("photo.png", "hola"), "text/plain"),
        ] {
            reset();
            assert_eq!(
                open_path_with(&file.to_string_lossy(), &|_| Ok(())),
                Ok(Opened::Revealed(Some(NotOpened::ContentType(mime.into())))),
                "{}",
                file.display()
            );
            let real = std::fs::canonicalize(&file).unwrap();
            let (p, a) = reveal_command(THIS_OS, &real);
            assert_eq!(launched(), vec![(p.to_string(), a)], "{} se muestra", file.display());
        }

        for file in [write("restore.sql", "update t set x = 1;"), write("todo.txt", "comprar pan")] {
            reset();
            assert_eq!(open_path_with(&file.to_string_lossy(), &|_| Ok(())), Ok(Opened::WithItsProgram), "{}", file.display());
            let real = std::fs::canonicalize(&file).unwrap();
            let (p, a) = open_command(THIS_OS, &real);
            assert_eq!(launched(), vec![(p.to_string(), a)]);
        }

        let sql = write("plain.sql", "select 1;");
        // Uno que falla aunque escriba algo: no se le cree.
        fake_xdg_mime(&dir, "#!/bin/sh\necho text/plain\nexit 1\n");
        reset();
        assert_eq!(open_path_with(&sql.to_string_lossy(), &|_| Ok(())), Ok(Opened::Revealed(Some(NotOpened::TypeUnknown))));
        let empty = dir.join("no-bin");
        std::fs::create_dir_all(&empty).unwrap();
        MIME_PATH.with(|m| *m.borrow_mut() = Some(empty.into_os_string()));
        reset();
        assert_eq!(open_path_with(&sql.to_string_lossy(), &|_| Ok(())), Ok(Opened::Revealed(Some(NotOpened::TypeUnknown))));
        assert!(!launched().iter().any(|(p, _)| p == "xdg-open"), "sin tipo no se abre");
    }

    /// macOS y Windows eligen el programa por la extensión: allí no se
    /// pregunta el contenido. Y un tipo de documento casa con su extensión.
    ///
    /// Roturas que detecta: tomar por buenos `text/html`, `application/xml`
    /// o `image/svg+xml`; una imagen que no casa con su extensión; dejar de
    /// abrir un `.sql` (`application/sql`) o un `.md` (`text/markdown`).
    #[test]
    fn a_document_type_matches_its_extension() {
        assert!(content_refusal(Os::Mac, Path::new("/x/a.sql")).is_none());
        assert!(content_refusal(Os::Windows, Path::new("/x/a.sql")).is_none());
        for (ext, mime) in [("sql", "application/sql"), ("md", "text/markdown"), ("txt", "text/plain"), ("json", "application/json"), ("pdf", "application/pdf"), ("PNG", "image/png"), ("jpg", "image/jpeg"), ("sql", "inode/x-empty")] {
            assert!(content_matches_extension(ext, mime), "{ext} {mime}");
        }
        for (ext, mime) in [("sql", "text/html"), ("txt", "application/xhtml+xml"), ("md", "image/svg+xml"), ("txt", "application/xml"), ("txt", "text/xml"), ("png", "image/jpeg"), ("pdf", "text/plain"), ("txt", "application/x-shellscript"), ("txt", "application/x-desktop"), ("sql", "application/octet-stream"), ("md", "application/x-shellscript"), ("csv", "application/x-executable"), ("png", "inode/x-empty"), ("pdf", "inode/x-empty")] {
            assert!(!content_matches_extension(ext, mime), "{ext} {mime}");
        }
    }

    /// Una `.app` de macOS es una carpeta, y abrirla la lanza: allí ninguna
    /// carpeta se abre, se muestra.
    ///
    /// Rotura que detecta: abrir carpetas en macOS, que ejecutaría una `.app`.
    #[test]
    fn on_macos_a_folder_is_revealed_because_an_app_is_a_folder() {
        assert!(!opens_folders(Os::Mac));
        assert!(opens_folders(Os::Linux) && opens_folders(Os::Windows));
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

    /// En Windows, `explorer` no entiende la ruta `\\?\C:\…` que da
    /// `canonicalize`: abrir un documento le pasa la ruta sin el prefijo, y
    /// una de red (`\\?\UNC\…`) la deja como está.
    ///
    /// Rotura que detecta: pasarle a `explorer` el `canonicalize` crudo, con lo
    /// que en Windows pulsar la ruta de un `.sql` no abre nada.
    #[test]
    fn on_windows_a_document_is_opened_without_the_verbatim_prefix() {
        assert_eq!(open_command(Os::Windows, Path::new(r"\\?\C:\tcg\apply.sql")), ("explorer", vec![r"C:\tcg\apply.sql".to_string()]));
        assert_eq!(open_command(Os::Windows, Path::new(r"\\?\UNC\srv\x.sql")), ("explorer", vec![r"\\?\UNC\srv\x.sql".to_string()]));
        assert_eq!(open_command(Os::Linux, Path::new("/tmp/apply.sql")), ("xdg-open", vec!["/tmp/apply.sql".to_string()]));
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
