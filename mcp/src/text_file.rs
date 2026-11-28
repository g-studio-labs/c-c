//! Un fichero de texto de esta máquina que va con un pedido
//! (command-and-control-567): el `apply.sql` que el agente adjunta a un
//! «Ejecutar» y el que la persona copia desde la ruta que el pedido nombra.
//!
//! Lo comparten el MCP (`create_request` con `files`) y la app nativa
//! («Copiar contenido» de una ruta): las dos leen un fichero por una ruta que
//! escribió un agente, y un agente puede venir dirigido por lo que leyó. Así
//! que, como `dropped::read_path`, se mira la ruta **real**: un enlace
//! simbólico que salga de su carpeta no pasa (`apply.sql` → `~/.ssh/id_rsa`),
//! ni un nombre o una carpeta de secretos, ni lo que no es texto, ni lo que
//! pasa del tope que acepta el servidor.

use std::path::{Path, PathBuf};

/// El tope por fichero de un pedido (`MaxRequestFileBytes` en el backend).
pub const MAX_BYTES: u64 = 256 * 1024;

/// Por qué un fichero no se lee. `code` es estable (lo traduce la app);
/// `Display` es para el agente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NotAbsolute,
    NotFound,
    NotAFile,
    /// Un enlace simbólico que apunta fuera de la carpeta donde se nombra.
    LinkOutside,
    /// Por el nombre o la carpeta parece una llave, un `.env`, credenciales.
    Secret,
    TooLarge,
    NotText,
    Unreadable,
}

impl Refusal {
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::NotAbsolute => "not-absolute",
            Refusal::NotFound => "not-found",
            Refusal::NotAFile => "not-a-file",
            Refusal::LinkOutside => "link-outside",
            Refusal::Secret => "secret",
            Refusal::TooLarge => "too-large",
            Refusal::NotText => "not-text",
            Refusal::Unreadable => "unreadable",
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let why = match self {
            Refusal::NotAbsolute => "the path must be absolute (/home/…, ~/…): the person does not know your working directory",
            Refusal::NotFound => "no such file",
            Refusal::NotAFile => "not a regular file",
            Refusal::LinkOutside => "it is a symbolic link that points outside its folder",
            Refusal::Secret => "its name or folder looks like a secret (.env, a key, credentials); never attach those",
            Refusal::TooLarge => "it is larger than 256 KB",
            Refusal::NotText => "it is not UTF-8 text",
            Refusal::Unreadable => "it could not be read",
        };
        f.write_str(why)
    }
}

/// Un fichero leído: su nombre (sin la ruta) y su contenido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFile {
    pub name: String,
    pub content: String,
}

/// Lee un fichero de texto por su ruta absoluta (o `~/…`).
pub fn read(path: &str) -> Result<TextFile, Refusal> {
    let real = check(path)?;
    let bytes = std::fs::read(&real).map_err(|_| Refusal::Unreadable)?;
    // Lo mismo que el backend: texto UTF-8 y sin NUL.
    let content = String::from_utf8(bytes).map_err(|_| Refusal::NotText)?;
    if content.contains('\0') {
        return Err(Refusal::NotText);
    }
    let name = Path::new(path.trim()).file_name().map(|n| n.to_string_lossy().into_owned()).ok_or(Refusal::NotAFile)?;
    Ok(TextFile { name, content })
}

/// Si se podría leer, sin leerlo entero: lo que mira la app antes de ofrecer
/// el botón. Devuelve la ruta real.
pub fn check(path: &str) -> Result<PathBuf, Refusal> {
    let path = path.trim();
    if crate::host::is_network_path(path) {
        return Err(Refusal::NotAbsolute);
    }
    let given = crate::host::expand_home(path);
    if !given.is_absolute() {
        return Err(Refusal::NotAbsolute);
    }
    if looks_secret(&given) {
        return Err(Refusal::Secret);
    }
    let real = std::fs::canonicalize(&given).map_err(|_| Refusal::NotFound)?;
    let folder = given.parent().and_then(|p| std::fs::canonicalize(p).ok()).ok_or(Refusal::NotFound)?;
    if !real.starts_with(&folder) {
        return Err(Refusal::LinkOutside);
    }
    if looks_secret(&real) {
        return Err(Refusal::Secret);
    }
    let meta = std::fs::metadata(&real).map_err(|_| Refusal::NotFound)?;
    if !meta.is_file() {
        return Err(Refusal::NotAFile);
    }
    if meta.len() > MAX_BYTES {
        return Err(Refusal::TooLarge);
    }
    Ok(real)
}

/// Carpetas donde sólo hay llaves (`.docker` guarda `config.json` con los
/// tokens de los registros).
const SECRET_DIRS: &[&str] = &[".ssh", ".gnupg", ".aws", ".kube", ".password-store", ".docker"];

/// Los tipos de llave que `ssh-keygen` pone tras `id_`: `id_rsa.bak` o
/// `id_ed25519-old` siguen siendo la llave.
const KEY_TYPES: &[&str] = &["rsa", "dsa", "ecdsa", "ed25519"];

/// Por el nombre: lo que no se adjunta ni se copia aunque sea texto. Todo en
/// minúsculas: el disco de macOS y el de Windows no distinguen, y allí
/// `~/.SSH/ID_RSA` es la llave.
pub fn looks_secret(path: &Path) -> bool {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    if parts.iter().any(|c| SECRET_DIRS.contains(&c.as_str())) {
        return true;
    }
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()) else {
        return false;
    };
    // El token de `gh`.
    if name == "hosts.yml" && parts.iter().rev().nth(1).is_some_and(|d| d == "gh") {
        return true;
    }
    let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().into_owned());
    let ext = ext.as_deref();
    let key_file = name.strip_prefix("id_").is_some_and(|rest| matches!(ext, None | Some("pub")) || KEY_TYPES.iter().any(|t| rest.starts_with(t)));
    name == ".env"
        || name.starts_with(".env.")
        || ext == Some("env")
        || matches!(ext, Some("pem" | "key" | "ppk" | "p12" | "pfx" | "kdbx" | "keystore" | "jks" | "asc" | "gpg"))
        || key_file
        || name.starts_with("credentials")
        || name.starts_with("secret")
        || matches!(name.as_str(), ".netrc" | "_netrc" | ".pgpass" | ".npmrc" | ".pypirc" | ".git-credentials")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-text-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_sql_file_is_read_whole_with_its_name() {
        let dir = temp("sql");
        let sql = "update pagos set moneda = 'USD'\n where id in (1, 2);\n";
        std::fs::write(dir.join("apply.sql"), sql).unwrap();
        let got = read(&dir.join("apply.sql").to_string_lossy()).unwrap();
        assert_eq!(got, TextFile { name: "apply.sql".into(), content: sql.into() });
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_link_that_leaves_its_folder_does_not_pass_even_with_an_innocent_target() {
        let dir = temp("link");
        let outside = temp("link-outside");
        std::fs::write(outside.join("notes.txt"), "fuera").unwrap();
        std::os::unix::fs::symlink(outside.join("notes.txt"), dir.join("apply.sql")).unwrap();
        assert_eq!(read(&dir.join("apply.sql").to_string_lossy()), Err(Refusal::LinkOutside));
        // Dentro de la misma carpeta, un enlace sí vale.
        std::fs::write(dir.join("real.sql"), "select 1;").unwrap();
        std::os::unix::fs::symlink(dir.join("real.sql"), dir.join("alias.sql")).unwrap();
        assert_eq!(read(&dir.join("alias.sql").to_string_lossy()).unwrap().content, "select 1;");
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn a_link_to_a_key_in_the_same_folder_is_a_secret() {
        let dir = temp("key");
        std::fs::write(dir.join("id_ed25519"), "-----BEGIN").unwrap();
        std::os::unix::fs::symlink(dir.join("id_ed25519"), dir.join("apply.sql")).unwrap();
        assert_eq!(read(&dir.join("apply.sql").to_string_lossy()), Err(Refusal::Secret));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn secrets_by_name_or_folder_are_refused_before_reading() {
        let dir = temp("env");
        for name in [".env", ".env.production", "prod.env", "server.pem", "tls.key", "id_rsa", "id_rsa.pub", "credentials.json", "secrets.yaml", ".pgpass"] {
            std::fs::write(dir.join(name), "X=1").unwrap();
            assert_eq!(read(&dir.join(name).to_string_lossy()), Err(Refusal::Secret), "{name}");
        }
        for fine in ["apply.sql", "id_mapping.sql", "environment.md", "keys.sql"] {
            assert!(!looks_secret(&dir.join(fine)), "{fine}");
        }
        // Por el nombre que se pidió, sin mirar el disco: un `.env` que no
        // existe o que enlaza a algo inocente se rechaza igual, por secreto.
        assert_eq!(read(&dir.join("missing").join(".env").to_string_lossy()), Err(Refusal::Secret));
        std::fs::write(dir.join("plain.txt"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("plain.txt"), dir.join(".env.local")).unwrap();
        assert_eq!(read(&dir.join(".env.local").to_string_lossy()), Err(Refusal::Secret));
        assert!(looks_secret(Path::new("/home/ana/.ssh/config")));
        assert!(looks_secret(Path::new("/home/ana/.aws/config")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// En macOS y Windows el disco no distingue mayúsculas: `.ENV`, `ID_RSA`
    /// o `~/.SSH/config` son el mismo secreto.
    ///
    /// Roturas que detecta: comparar el nombre o la carpeta sin pasarlos a
    /// minúsculas.
    #[test]
    fn a_secret_in_capitals_is_still_a_secret() {
        let dir = temp("caps");
        for name in [".ENV", "ID_RSA", "Server.PEM", "Prod.Env", "CREDENTIALS.json"] {
            std::fs::write(dir.join(name), "X=1").unwrap();
            assert_eq!(read(&dir.join(name).to_string_lossy()), Err(Refusal::Secret), "{name}");
        }
        for path in ["/Users/ana/.SSH/config", "/Users/ana/.Aws/credentials", "C:/Users/ana/.Docker/config.json", "/Users/ana/.config/GH/Hosts.yml"] {
            assert!(looks_secret(Path::new(path)), "{path}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Lo que guarda una llave o un token aunque no se llame `.env`.
    ///
    /// Roturas que detecta: dejar pasar una copia de una llave (`id_rsa.bak`),
    /// una llave de PuTTY, un almacén de certificados o de contraseñas, el
    /// token de `gh`, el de Docker o los ficheros de credenciales de npm, pip,
    /// git y Postgres. Y, al revés, tomar por llave un `id_mapping.sql` o
    /// cualquier `hosts.yml` que no es el de `gh`.
    #[test]
    fn keys_tokens_and_their_copies_are_secrets() {
        for path in [
            "/home/ana/keys/id_rsa.bak",
            "/home/ana/keys/id_ed25519.old",
            "/home/ana/keys/id_ecdsa-2024",
            "/home/ana/keys/server.ppk",
            "/home/ana/certs/client.p12",
            "/home/ana/certs/client.pfx",
            "/home/ana/vault.kdbx",
            "/home/ana/.netrc",
            "/home/ana/.pgpass",
            "/home/ana/.npmrc",
            "/home/ana/.pypirc",
            "/home/ana/.git-credentials",
            "/home/ana/.docker/config.json",
            "/home/ana/.config/gh/hosts.yml",
        ] {
            assert!(looks_secret(Path::new(path)), "{path}");
        }
        for fine in ["/home/ana/tcg/id_mapping.sql", "/home/ana/tcg/id_rsvp.csv", "/home/ana/ansible/hosts.yml", "/home/ana/tcg/apply.sql"] {
            assert!(!looks_secret(Path::new(fine)), "{fine}");
        }
    }

    #[test]
    fn too_large_not_text_relative_and_missing_are_refused() {
        let dir = temp("misc");
        let big = dir.join("big.sql");
        std::fs::File::create(&big).unwrap().set_len(MAX_BYTES + 1).unwrap();
        assert_eq!(read(&big.to_string_lossy()), Err(Refusal::TooLarge));
        let exact = dir.join("exact.sql");
        std::fs::write(&exact, "x".repeat(MAX_BYTES as usize)).unwrap();
        assert!(read(&exact.to_string_lossy()).is_ok());
        std::fs::write(dir.join("blob.sql"), b"abc\xff\xfe").unwrap();
        assert_eq!(read(&dir.join("blob.sql").to_string_lossy()), Err(Refusal::NotText));
        std::fs::write(dir.join("nul.sql"), b"abc\0def").unwrap();
        assert_eq!(read(&dir.join("nul.sql").to_string_lossy()), Err(Refusal::NotText));
        assert_eq!(read("apply.sql"), Err(Refusal::NotAbsolute));
        assert_eq!(read("tcg/apply.sql"), Err(Refusal::NotAbsolute));
        assert_eq!(read(&dir.join("nope.sql").to_string_lossy()), Err(Refusal::NotFound));
        assert_eq!(read(&dir.to_string_lossy()), Err(Refusal::NotAFile));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
