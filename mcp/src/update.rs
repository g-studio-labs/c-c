//! Instalar y actualizar `cac-mcp`: saber qué versión hay, bajarla, comprobar
//! que es la que se publicó y ponerla en su sitio.
//!
//! Una sola copia de esta lógica. La usan el propio binario (`cac-mcp update`,
//! el aviso de «hay una versión nueva» al agente) y la app de escritorio
//! nativa (`desktop/`), que instala y actualiza el binario desde su menú; los
//! scripts de una línea (`scripts/install-mcp.sh` y `.ps1`) hacen lo mismo
//! para quien todavía no tiene nada, y `tests/install_script.rs` los prueba
//! contra los mismos nombres.
//!
//! De dónde sale cada cosa:
//!
//! - **La última versión**, del fichero `mcp/VERSION` de `main`, por
//!   `raw.githubusercontent.com`. No de la API de GitHub (tiene límite por IP
//!   y pide cabeceras) ni de `releases/latest`: esa etiqueta es de la app de
//!   Tauri, cuyo actualizador lee `releases/latest/download/latest.json`, y
//!   una release del MCP **nunca** se marca como la última (guardián:
//!   `tests/release.rs`).
//! - **Los binarios**, de la release `mcp-vX.Y.Z` del mismo repo: un fichero
//!   por plataforma, sin empaquetar, y `SHA256SUMS` al lado.
//!
//! Nada se instala solo. Se avisa, y actualiza quien lo pide.
//!
//! Todo aquí es síncrono y arranca su propio runtime de tokio, como el resto
//! del servidor: quien lo llame desde dentro de otro runtime tiene que hacerlo
//! en un hilo aparte.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// El fichero de `main` que dice cuál es la última versión publicada.
pub const VERSION_URL: &str = "https://raw.githubusercontent.com/g-studio-labs/c-c/main/mcp/VERSION";
/// De dónde se bajan los ficheros de una release: `<esto>/mcp-vX.Y.Z/<fichero>`.
pub const DOWNLOAD_BASE: &str = "https://github.com/g-studio-labs/c-c/releases/download";
/// El fichero de sumas de cada release, en el formato de `sha256sum`.
pub const CHECKSUMS: &str = "SHA256SUMS";
/// Instalar sin tener nada, en Linux y macOS.
pub const INSTALL_SH: &str = "curl -fsSL https://raw.githubusercontent.com/g-studio-labs/c-c/main/scripts/install-mcp.sh | sh";
/// Lo mismo en Windows (PowerShell).
pub const INSTALL_PS: &str = "irm https://raw.githubusercontent.com/g-studio-labs/c-c/main/scripts/install-mcp.ps1 | iex";

/// No preguntar por versiones nuevas (cualquier valor que no sea vacío ni `0`).
pub const ENV_NO_CHECK: &str = "CAC_MCP_NO_UPDATE_CHECK";
/// Otra dirección para `mcp/VERSION` (un espejo, las pruebas).
pub const ENV_VERSION_URL: &str = "CAC_MCP_VERSION_URL";
/// Otra base para los ficheros de las releases.
pub const ENV_DOWNLOAD_BASE: &str = "CAC_MCP_DOWNLOAD_BASE";
/// Otra carpeta para lo que este programa recuerda entre arranques.
pub const ENV_STATE_DIR: &str = "CAC_MCP_STATE_DIR";
/// Otro plazo, en segundos, para preguntar por la última versión. Lo usan las
/// pruebas: con un plazo que no se acaba, esperar a la red es colgarse, y eso
/// se distingue de ir lento sin cronometrar nada.
pub const ENV_CHECK_TIMEOUT: &str = "CAC_MCP_VERSION_TIMEOUT";

/// Cada cuánto se vuelve a preguntar si la última vez contestaron.
const CHECK_EVERY: u64 = 24 * 60 * 60;
/// Y si no contestaron (sin red, GitHub caído): antes, pero no en cada arranque.
const RETRY_AFTER: u64 = 60 * 60;
/// Un binario de `cac-mcp` ronda los 15 MB. Más que esto no es uno, y no se
/// guarda entero en memoria lo que mande cualquiera.
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;

/// De dónde se lee la versión y de dónde se baja.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub version_url: String,
    pub download_base: String,
}

impl Default for Source {
    fn default() -> Self {
        Source { version_url: VERSION_URL.into(), download_base: DOWNLOAD_BASE.into() }
    }
}

impl Source {
    /// Las direcciones de siempre, salvo lo que diga el entorno.
    pub fn from_env() -> Self {
        let or = |name: &str, default: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default.into());
        Source { version_url: or(ENV_VERSION_URL, VERSION_URL), download_base: or(ENV_DOWNLOAD_BASE, DOWNLOAD_BASE) }
    }

    /// Dónde está un fichero de la release de esa versión.
    pub fn asset_url(&self, version: &str, file: &str) -> String {
        format!("{}/{}/{file}", self.download_base.trim_end_matches('/'), tag(version))
    }
}

/// La etiqueta de la release de una versión. Con prefijo propio: las `vX.Y.Z`
/// a secas son de la app.
pub fn tag(version: &str) -> String {
    format!("mcp-v{version}")
}

/// El fichero de la release para un sistema y una arquitectura, con los
/// nombres de `std::env::consts`. `None`: para eso no se publica binario.
///
/// Los mismos nombres están escritos en `scripts/install-mcp.sh`, en
/// `install-mcp.ps1` y en `mcp-release.yml`; `tests/release.rs` comprueba que
/// los cuatro sitios dicen lo mismo.
pub fn asset_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("cac-mcp-linux-x86_64"),
        ("macos", "aarch64") => Some("cac-mcp-macos-arm64"),
        ("macos", "x86_64") => Some("cac-mcp-macos-x86_64"),
        ("windows", "x86_64") => Some("cac-mcp-windows-x86_64.exe"),
        _ => None,
    }
}

/// Todo lo que lleva una release, además de `SHA256SUMS`.
pub const ASSETS: [&str; 4] = ["cac-mcp-linux-x86_64", "cac-mcp-macos-arm64", "cac-mcp-macos-x86_64", "cac-mcp-windows-x86_64.exe"];

/// El fichero para la máquina en la que corre esto.
pub fn this_platform_asset() -> Result<&'static str, String> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    asset_name(os, arch).ok_or_else(|| format!("there is no cac-mcp binary for {os} {arch} (published: Linux x86_64, macOS arm64 and x86_64, Windows x86_64)"))
}

/// `X.Y.Z`, y nada más: lo que no se parece a una versión no lo es. Importa
/// porque esto se lee de la red, y un portal cautivo contesta 200 con HTML.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().split('.');
    let mut next = || {
        let part = parts.next()?;
        (!part.is_empty() && part.len() <= 9 && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok()).flatten()
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Si `available` es más nueva que `installed`. Lo que no se entiende, de un
/// lado o del otro, no es «más nuevo»: ante la duda no se ofrece nada.
pub fn is_newer(available: &str, installed: &str) -> bool {
    match (parse_version(available), parse_version(installed)) {
        (Some(a), Some(i)) => a > i,
        _ => false,
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10).min(timeout))
        .timeout(timeout)
        .user_agent(concat!("cac-mcp/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// La última versión publicada, preguntada ahora.
pub fn fetch_latest(source: &Source, timeout: Duration) -> Result<String, String> {
    let text = runtime()?.block_on(async {
        let res = client(timeout)?.get(&source.version_url).send().await.map_err(|e| format!("could not reach {}: {e}", source.version_url))?;
        if !res.status().is_success() {
            return Err(format!("{} answered {}", source.version_url, res.status()));
        }
        res.text().await.map_err(|e| e.to_string())
    })?;
    let version = text.trim();
    parse_version(version).map(|_| version.to_string()).ok_or_else(|| format!("{} did not answer with a version", source.version_url))
}

/// La suma publicada de un fichero, de un `SHA256SUMS` (`<hex>  <nombre>`, o
/// `<hex> *<nombre>` en modo binario).
pub fn checksum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let (sum, name) = (parts.next()?, parts.next()?);
        (name.trim_start_matches('*') == file && sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit())).then(|| sum.to_ascii_lowercase())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Por dónde va una instalación, para quien la enseña.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// Bajando: lo recibido y, si el servidor lo dijo, el total.
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Installing,
}

/// Baja el binario de esa versión y lo devuelve **sólo si su SHA-256 es el
/// publicado**. Sin suma publicada para ese fichero tampoco se devuelve: un
/// binario que no se puede comprobar no se instala.
pub fn download_verified(source: &Source, version: &str, file: &str, progress: &mut dyn FnMut(Progress)) -> Result<Vec<u8>, String> {
    if parse_version(version).is_none() {
        return Err(format!("`{version}` is not a version (expected X.Y.Z)"));
    }
    let (sums_url, file_url) = (source.asset_url(version, CHECKSUMS), source.asset_url(version, file));
    let (sums, bytes) = runtime()?.block_on(async {
        let client = client(Duration::from_secs(600))?;
        let get = |url: String| {
            let client = client.clone();
            async move {
                let res = client.get(&url).send().await.map_err(|e| format!("could not download {url}: {e}"))?;
                if !res.status().is_success() {
                    return Err(format!("{url} answered {} (is version {version} published?)", res.status()));
                }
                Ok(res)
            }
        };
        let sums = get(sums_url.clone()).await?.text().await.map_err(|e| e.to_string())?;
        let mut res = get(file_url.clone()).await?;
        let total = res.content_length();
        let mut bytes = Vec::new();
        progress(Progress::Downloading { received: 0, total });
        while let Some(chunk) = res.chunk().await.map_err(|e| format!("the download of {file_url} was cut: {e}"))? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > MAX_DOWNLOAD {
                return Err(format!("{file_url} is larger than a cac-mcp binary can be; nothing was installed"));
            }
            progress(Progress::Downloading { received: bytes.len() as u64, total });
        }
        Ok((sums, bytes))
    })?;
    progress(Progress::Verifying);
    let published = checksum_for(&sums, file).ok_or_else(|| format!("{sums_url} has no checksum for {file}; nothing was installed"))?;
    let got = sha256_hex(&bytes);
    if got != published {
        return Err(format!("checksum mismatch for {file}: published {published}, downloaded {got}. Nothing was installed."));
    }
    Ok(bytes)
}

/// Pone `bytes` en `dest` de una vez: o queda el binario nuevo entero, o queda
/// lo que había. Se escribe al lado y se renombra, que dentro de una carpeta es
/// atómico; nunca se escribe sobre el fichero que puede estar corriendo.
pub fn replace(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    replace_as(dest, bytes, cfg!(windows))
}

/// `replace`, eligiendo la estrategia. `aside` es la de Windows, donde un
/// `.exe` en marcha no se puede pisar ni borrar pero **sí renombrar**: el
/// viejo se aparta a `<nombre>.old`, el nuevo ocupa su sitio, y si eso falla
/// el viejo vuelve. El `.old` se borra cuando se puede (la vez siguiente, si
/// ahora está en uso). Público sólo para probar esa rama fuera de Windows.
#[doc(hidden)]
pub fn replace_as(dest: &Path, bytes: &[u8], aside: bool) -> Result<(), String> {
    let dir = dest.parent().filter(|d| !d.as_os_str().is_empty()).ok_or_else(|| format!("{} has no folder", dest.display()))?;
    let name = dest.file_name().ok_or_else(|| format!("{} is not a file path", dest.display()))?.to_string_lossy().to_string();
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let staged = dir.join(format!(".{name}.new-{}", std::process::id()));
    let put = || -> Result<(), String> {
        write_executable(&staged, bytes).map_err(|e| format!("could not write to {}: {e}", dir.display()))?;
        if !aside {
            return std::fs::rename(&staged, dest).map_err(|e| format!("could not replace {}: {e}", dest.display()));
        }
        let old = dir.join(format!("{name}.old"));
        let _ = std::fs::remove_file(&old);
        let had = dest.exists();
        if had {
            std::fs::rename(dest, &old).map_err(|e| format!("could not move the running {} aside: {e}", dest.display()))?;
        }
        if let Err(e) = std::fs::rename(&staged, dest) {
            if had {
                let _ = std::fs::rename(&old, dest);
            }
            return Err(format!("could not put the new binary at {}: {e}", dest.display()));
        }
        let _ = std::fs::remove_file(&old);
        Ok(())
    };
    let done = put();
    if done.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    done
}

fn write_executable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o755);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    // En disco antes de renombrar: un corte de luz no deja un binario a medias
    // con el nombre bueno.
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // `mode` de arriba pasa por la umask, y el fichero podía existir.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Lo que quedó de una actualización en Windows: el binario viejo, apartado
/// mientras corría. Al arrancar el nuevo ya se puede borrar.
pub fn clear_leftovers(exe: &Path) {
    if let Some(name) = exe.file_name() {
        let _ = std::fs::remove_file(exe.with_file_name(format!("{}.old", name.to_string_lossy())));
    }
}

/// Baja esa versión para esta máquina, la comprueba y la deja en `dest`.
pub fn install(source: &Source, version: &str, dest: &Path, progress: &mut dyn FnMut(Progress)) -> Result<(), String> {
    let bytes = download_verified(source, version, this_platform_asset()?, progress)?;
    progress(Progress::Installing);
    replace(dest, &bytes)
}

/// Dónde se instala `cac-mcp` cuando no está: una carpeta de la persona, sin
/// permisos de administrador. La misma que usan los scripts.
pub fn default_install_path() -> Option<PathBuf> {
    install_path_from(std::env::consts::OS, &|name| std::env::var_os(name).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()))
}

fn install_path_from(os: &str, env: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if os == "windows" {
        return Some(env("LOCALAPPDATA")?.join("Programs").join("cac-mcp").join("cac-mcp.exe"));
    }
    Some(env("HOME")?.join(".local").join("bin").join("cac-mcp"))
}

/// Un `cac-mcp` que ya está en la máquina.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub path: PathBuf,
    pub version: String,
}

/// La versión que dice un binario con `--version` (`cac-mcp X.Y.Z` en la
/// primera línea). `None` si no arranca o contesta otra cosa: no es `cac-mcp`.
pub fn version_of(binary: &Path) -> Option<String> {
    let out = crate::host::command(binary).arg("--version").stdin(std::process::Stdio::null()).output().ok()?;
    version_line(&String::from_utf8_lossy(&out.stdout))
}

fn version_line(output: &str) -> Option<String> {
    let version = output.lines().next()?.strip_prefix("cac-mcp ")?.trim();
    parse_version(version).map(|_| version.to_string())
}

/// Dónde puede estar `cac-mcp`: primero donde lo instalan los scripts y la
/// app, luego en las carpetas del `PATH`.
fn usual_places() -> Vec<PathBuf> {
    let name = if cfg!(windows) { "cac-mcp.exe" } else { "cac-mcp" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    default_install_path().into_iter().chain(std::env::split_paths(&path).map(|dir| dir.join(name))).collect()
}

/// El primero de esos sitios donde hay un binario que contesta con su versión.
fn find_among(candidates: impl Iterator<Item = PathBuf>) -> Option<Installed> {
    candidates.filter(|path| path.is_file()).find_map(|path| version_of(&path).map(|version| Installed { path, version }))
}

// ── Lo que se recuerda entre arranques ──────────────────────────────────────

/// La carpeta de este programa para lo que recuerda (hoy, cuándo preguntó por
/// la última versión y qué le dijeron).
pub fn state_dir() -> Option<PathBuf> {
    state_dir_from(std::env::consts::OS, &|name| std::env::var_os(name).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()))
}

fn state_dir_from(os: &str, env: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = env(ENV_STATE_DIR) {
        return Some(dir);
    }
    match os {
        "windows" => Some(env("LOCALAPPDATA")?.join("cac-mcp")),
        "macos" => Some(env("HOME")?.join("Library").join("Application Support").join("cac-mcp")),
        _ => Some(env("XDG_STATE_HOME").or_else(|| env("HOME").map(|home| home.join(".local").join("state")))?.join("cac-mcp")),
    }
}

/// Lo que se supo la última vez que se preguntó.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    /// La última versión que se llegó a saber. Sobrevive a una pregunta sin
    /// respuesta: lo que se supo ayer sigue siendo lo mejor que se sabe.
    pub latest: Option<String>,
    /// Cuándo se preguntó, en segundos desde 1970.
    pub at: u64,
    /// Si esa vez contestaron.
    pub answered: bool,
}

fn cache_file(dir: &Path) -> PathBuf {
    dir.join("update.json")
}

pub fn read_checked(dir: &Path) -> Option<Checked> {
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(cache_file(dir)).ok()?).ok()?;
    Some(Checked {
        latest: value["latest"].as_str().filter(|v| parse_version(v).is_some()).map(str::to_string),
        at: value["checkedAt"].as_u64()?,
        answered: value["answered"].as_bool().unwrap_or(false),
    })
}

fn write_checked(dir: &Path, checked: &Checked) {
    // Lo mejor que se pueda: sin carpeta donde escribir se pregunta otra vez
    // la próxima, y ya.
    let _ = std::fs::create_dir_all(dir);
    let staged = dir.join(format!(".update.json.{}", std::process::id()));
    let text = serde_json::json!({ "latest": checked.latest, "checkedAt": checked.at, "answered": checked.answered }).to_string();
    if std::fs::write(&staged, text).is_ok() && std::fs::rename(&staged, cache_file(dir)).is_err() {
        let _ = std::fs::remove_file(&staged);
    }
}

/// Si toca volver a preguntar: una vez al día, o a la hora si la última no
/// contestaron. Un reloj que fue hacia atrás cuenta como «toca».
pub fn check_is_due(checked: Option<&Checked>, now: u64) -> bool {
    check_is_due_within(checked, now, CHECK_EVERY)
}

/// Lo mismo, con otro plazo que el día; sin respuesta se espera lo de siempre
/// o ese plazo, lo que sea menos.
fn check_is_due_within(checked: Option<&Checked>, now: u64, max_age: u64) -> bool {
    match checked {
        None => true,
        Some(checked) => now < checked.at || now - checked.at >= if checked.answered { max_age } else { RETRY_AFTER.min(max_age) },
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Pregunta ahora por la última versión y lo apunta, conteste o no. Devuelve
/// la respuesta de la red tal cual; lo que queda apuntado conserva la última
/// versión que se supo.
pub fn check_now(source: &Source, dir: Option<&Path>, timeout: Duration) -> Result<String, String> {
    let answer = fetch_latest(source, timeout);
    if let Some(dir) = dir {
        let before = read_checked(dir).and_then(|checked| checked.latest);
        write_checked(dir, &Checked { latest: answer.clone().ok().or(before), at: now(), answered: answer.is_ok() });
    }
    answer
}

/// La última versión que se sabe, preguntando sólo si toca. Sin red devuelve
/// lo que se supiera, o nada. Es lo que corre de fondo al arrancar el
/// servidor: puede tardar lo que tarde la red, así que nunca en el hilo que
/// contesta al cliente.
pub fn latest_known(source: &Source, dir: &Path, now: u64) -> Option<String> {
    latest_known_within(source, dir, now, CHECK_EVERY)
}

/// Lo mismo, para quien quiere saberlo antes de un día: la app de escritorio,
/// que lo enseña en una ventana que alguien abrió a propósito.
pub fn latest_known_within(source: &Source, dir: &Path, now: u64, max_age: u64) -> Option<String> {
    let checked = read_checked(dir);
    if !check_is_due_within(checked.as_ref(), now, max_age) {
        return checked.and_then(|checked| checked.latest);
    }
    let before = checked.and_then(|checked| checked.latest);
    check_now(source, Some(dir), check_timeout()).ok().or(before)
}

/// El plazo de preguntar por la última versión: ocho segundos, salvo lo que
/// diga el entorno.
fn check_timeout() -> Duration {
    let secs = std::env::var(ENV_CHECK_TIMEOUT).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(8);
    Duration::from_secs(secs)
}

/// Si se pidió no preguntar por versiones nuevas.
pub fn checks_disabled() -> bool {
    std::env::var(ENV_NO_CHECK).is_ok_and(|v| !v.trim().is_empty() && v.trim() != "0")
}

// ── Lo que decide quien lo enseña ───────────────────────────────────────────

/// Qué se le ofrece a la persona.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// No está: instalarlo.
    Install,
    /// Está y hay una más nueva: actualizarlo.
    Update,
    /// Está y no se sabe de ninguna más nueva: nada que hacer.
    Current,
}

/// Lo que se sabe de `cac-mcp` en esta máquina.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub installed: Option<Installed>,
    /// La última publicada, si se pudo saber.
    pub latest: Option<String>,
}

impl Status {
    pub fn step(&self) -> Step {
        match (&self.installed, &self.latest) {
            (None, _) => Step::Install,
            (Some(installed), Some(latest)) if is_newer(latest, &installed.version) => Step::Update,
            _ => Step::Current,
        }
    }
}

/// Una máquina, para quien instala desde fuera del binario (la app de
/// escritorio): dónde buscar `cac-mcp`, dónde ponerlo, de dónde bajarlo y
/// dónde se recuerda lo que se sabe. Quien pinta sólo guarda una de éstas y
/// pregunta; no decide nada.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub source: Source,
    pub state_dir: Option<PathBuf>,
    /// Dónde puede estar ya, en orden.
    pub look_in: Vec<PathBuf>,
    /// Dónde se instala si no está en ninguno de esos sitios.
    pub install_to: Option<PathBuf>,
}

impl Machine {
    /// Ésta, con lo que diga el entorno.
    pub fn from_env() -> Self {
        Machine { source: Source::from_env(), state_dir: state_dir(), look_in: usual_places(), install_to: default_install_path() }
    }

    /// Mira si está instalado y, si lo último que se sabe tiene más de
    /// `max_age` segundos, pregunta por la última versión. Bloquea: lanza el
    /// binario y puede salir a la red.
    pub fn look(&self, max_age: u64) -> Status {
        Status {
            installed: find_among(self.look_in.iter().cloned()),
            latest: self.state_dir.as_deref().and_then(|dir| latest_known_within(&self.source, dir, now(), max_age)),
        }
    }

    /// Dónde va el binario al instalar o actualizar: donde ya está, o donde lo
    /// ponen los scripts.
    pub fn target(&self, status: &Status) -> Option<PathBuf> {
        status.installed.as_ref().map(|installed| installed.path.clone()).or_else(|| self.install_to.clone())
    }

    /// Instala o actualiza, lo que toque según `status`, y devuelve lo que
    /// quedó instalado, preguntándoselo al binario nuevo. Bloquea.
    pub fn install(&self, status: &Status, progress: &mut dyn FnMut(Progress)) -> Result<Installed, String> {
        let dest = self.target(status).ok_or("this machine does not say where the user's programs go (no HOME)")?;
        // Sin saber la última (no hubo red al mirar), se pregunta ahora: para
        // instalar hace falta red de todos modos, y así el fallo lo dice.
        let version = match &status.latest {
            Some(latest) => latest.clone(),
            None => check_now(&self.source, self.state_dir.as_deref(), Duration::from_secs(20))?,
        };
        install(&self.source, &version, &dest, progress)?;
        match version_of(&dest) {
            Some(version) => Ok(Installed { path: dest, version }),
            None => Err(format!("cac-mcp {version} was installed at {} but it does not run on this machine", dest.display())),
        }
    }
}

/// El marcador que va en la orden donde iría el token. Esta lógica no ve
/// tokens: ni los crea, ni los lee, ni los imprime.
pub const TOKEN_PLACEHOLDER: &str = "PASTE_YOUR_TOKEN";

/// La orden que registra el binario en Claude Code, con el sitio del token
/// marcado. La misma forma que enseña la app de siempre para `cac --mcp`.
pub fn register_command(cac_url: &str, binary: &Path) -> String {
    format!("claude mcp add cac -e CAC_URL={} -e CAC_TOKEN={TOKEN_PLACEHOLDER} -- \"{}\"", cac_url.trim_end_matches('/'), binary.display())
}

/// Lo que se le dice al agente cuando hay una versión más nueva que la que
/// sirve. `None` si no la hay. `exe` es el binario que corre, para que la
/// orden valga aunque no esté en el `PATH`.
pub fn update_note(serving: &str, latest: Option<&str>, exe: Option<&Path>) -> Option<String> {
    let latest = latest.filter(|latest| is_newer(latest, serving))?;
    let command = exe.map(|exe| format!("\"{}\" update", exe.display())).unwrap_or_else(|| "cac-mcp update".into());
    Some(format!(
        "NOTE: cac-mcp {latest} is published and this MCP server is {serving}. At a natural pause, tell the user in one \
         sentence. Nothing updates by itself: to update, run `{command}` in a terminal (ask first), which downloads the \
         new binary, verifies its checksum and replaces this one; it takes effect in the next session (/mcp → cac → \
         reconnect, or a new session). Mention it once."
    ))
}

/// Un servidor de mentira con una release, para las pruebas de este crate, de
/// los scripts y de la app: sirve por HTTP en esta máquina lo que se le ponga
/// y apunta lo que le piden. Ninguna prueba sale a internet.
#[doc(hidden)]
pub mod fake {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Condvar, Mutex};

    use super::Source;

    #[derive(Clone)]
    pub struct FakeReleases {
        pub base: String,
        files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        asked: Arc<Mutex<Vec<String>>>,
        /// Mientras está puesto, lo que se pide se apunta y no se contesta.
        paused: Arc<(Mutex<bool>, Condvar)>,
    }

    impl FakeReleases {
        pub fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("un puerto libre en esta máquina");
            let base = format!("http://{}", listener.local_addr().unwrap());
            let files: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
            let asked: Arc<Mutex<Vec<String>>> = Arc::default();
            let paused: Arc<(Mutex<bool>, Condvar)> = Arc::default();
            let (served, seen, held) = (files.clone(), asked.clone(), paused.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut first = String::new();
                    if reader.read_line(&mut first).is_err() {
                        continue;
                    }
                    // El resto de la cabecera, hasta la línea en blanco.
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                        line.clear();
                    }
                    let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                    seen.lock().unwrap().push(path.clone());
                    let (lock, resumed) = &*held;
                    drop(resumed.wait_while(lock.lock().unwrap(), |paused| *paused).unwrap());
                    let body = served.lock().unwrap().get(&path).cloned();
                    let (status, body) = match body {
                        Some(body) => ("200 OK", body),
                        None => ("404 Not Found", b"not found".to_vec()),
                    };
                    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = stream.write_all(&body);
                }
            });
            FakeReleases { base, files, asked, paused }
        }

        /// Deja de contestar hasta `resume`: quien pregunta espera, como con
        /// una red lenta, pero lo que va antes y lo que va después lo decide
        /// la prueba y no el reloj.
        pub fn pause(&self) {
            *self.paused.0.lock().unwrap() = true;
        }

        pub fn resume(&self) {
            *self.paused.0.lock().unwrap() = false;
            self.paused.1.notify_all();
        }

        pub fn source(&self) -> Source {
            Source { version_url: format!("{}/VERSION", self.base), download_base: format!("{}/download", self.base) }
        }

        pub fn put(&self, path: &str, body: impl Into<Vec<u8>>) {
            self.files.lock().unwrap().insert(path.to_string(), body.into());
        }

        pub fn remove(&self, path: &str) {
            self.files.lock().unwrap().remove(path);
        }

        /// La versión que `mcp/VERSION` dice que es la última.
        pub fn announce(&self, version: &str) {
            self.put("/VERSION", format!("{version}\n"));
        }

        /// Publica una release con ese binario para todas las plataformas y
        /// su `SHA256SUMS`.
        pub fn publish(&self, version: &str, binary: &[u8]) {
            let mut sums = String::new();
            for asset in super::ASSETS {
                self.put(&format!("/download/mcp-v{version}/{asset}"), binary);
                sums.push_str(&format!("{}  {asset}\n", super::sha256_hex(binary)));
            }
            self.put(&format!("/download/mcp-v{version}/SHA256SUMS"), sums);
        }

        /// Las rutas que se pidieron, en orden.
        pub fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeReleases;
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-mcp-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Roturas que atrapa: comparar versiones como texto («1.9.0» ganaría a
    /// «1.10.0»); ofrecer como «más nueva» la misma o una más vieja; tomar por
    /// versión lo que contesta un portal cautivo.
    #[test]
    fn a_version_is_newer_by_its_numbers_and_only_if_it_is_one() {
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(is_newer("2.0.0", "1.99.99"));
        assert!(is_newer("1.33.1", "1.33.0"));
        assert!(!is_newer("1.33.0", "1.33.0"));
        assert!(!is_newer("1.32.9", "1.33.0"));
        assert!(!is_newer("<html>login</html>", "1.33.0"));
        assert!(!is_newer("1.34", "1.33.0"));
        assert!(!is_newer("1.34.0.1", "1.33.0"));
        assert!(!is_newer("1.34.0", "desconocida"));
        assert_eq!(parse_version(" 1.33.0\n"), Some((1, 33, 0)));
        assert_eq!(parse_version("v1.33.0"), None);
        assert_eq!(parse_version("1..0"), None);
    }

    /// Roturas que atrapa: tomar la suma de otro fichero (la primera línea, o
    /// una cuyo nombre sólo empieza igual); no entender el modo binario de
    /// `sha256sum` (`*nombre`); aceptar algo que no es un SHA-256.
    #[test]
    fn the_published_checksum_is_the_one_of_that_file() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a}  cac-mcp-linux-x86_64.sig\n{b} *cac-mcp-linux-x86_64\nabc  cac-mcp-macos-arm64\n");
        assert_eq!(checksum_for(&sums, "cac-mcp-linux-x86_64"), Some("b".repeat(64)));
        assert_eq!(checksum_for(&sums, "cac-mcp-macos-arm64"), None);
        assert_eq!(checksum_for(&sums, "cac-mcp-windows-x86_64.exe"), None);
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    /// Un binario cuya suma no es la publicada no se devuelve, y sin suma
    /// publicada tampoco. Roturas que atrapa: no comparar la suma; instalar
    /// cuando `SHA256SUMS` no nombra el fichero; bajar de otra etiqueta que la
    /// de la versión pedida.
    #[test]
    fn a_download_is_only_handed_over_if_its_checksum_is_the_published_one() {
        let releases = FakeReleases::start();
        releases.publish("1.34.0", b"el binario bueno");
        let mut steps = Vec::new();
        let asset = "cac-mcp-linux-x86_64";
        let got = download_verified(&releases.source(), "1.34.0", asset, &mut |step| steps.push(step)).unwrap();
        assert_eq!(got, b"el binario bueno");
        assert_eq!(steps.last(), Some(&Progress::Verifying));
        assert!(steps.contains(&Progress::Downloading { received: 16, total: Some(16) }), "{steps:?}");
        assert!(releases.asked().contains(&"/download/mcp-v1.34.0/cac-mcp-linux-x86_64".to_string()), "{:?}", releases.asked());

        // Alguien cambió el binario y no la suma.
        releases.put("/download/mcp-v1.34.0/cac-mcp-linux-x86_64", "otro binario");
        let refused = download_verified(&releases.source(), "1.34.0", asset, &mut |_| {}).unwrap_err();
        assert!(refused.contains("checksum mismatch"), "{refused}");

        // La suma de ese fichero no está publicada.
        releases.put("/download/mcp-v1.34.0/SHA256SUMS", format!("{}  otro-fichero\n", sha256_hex(b"otro binario")));
        let refused = download_verified(&releases.source(), "1.34.0", asset, &mut |_| {}).unwrap_err();
        assert!(refused.contains("no checksum"), "{refused}");

        // Una versión que no se publicó.
        let missing = download_verified(&releases.source(), "9.9.9", asset, &mut |_| {}).unwrap_err();
        assert!(missing.contains("404"), "{missing}");
        assert!(download_verified(&releases.source(), "../1.34.0", asset, &mut |_| {}).is_err());
    }

    /// Instalar deja el binario nuevo, ejecutable, y nada más en la carpeta; y
    /// una instalación rechazada deja lo que había, entero. Roturas que
    /// atrapa: pisar el binario antes de comprobar la suma; dejar el temporal
    /// tirado; instalar sin permiso de ejecución; no crear la carpeta.
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn installing_replaces_the_binary_whole_or_leaves_the_old_one() {
        use std::os::unix::fs::PermissionsExt;
        let releases = FakeReleases::start();
        releases.publish("1.34.0", b"nuevo");
        let dir = scratch("install");
        let dest = dir.join("bin").join("cac-mcp");

        install(&releases.source(), "1.34.0", &dest, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"nuevo");
        assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o755);

        releases.publish("1.35.0", b"mas nuevo");
        releases.put(&format!("/download/mcp-v1.35.0/{}", this_platform_asset().unwrap()), "manipulado");
        let refused = install(&releases.source(), "1.35.0", &dest, &mut |_| {}).unwrap_err();
        assert!(refused.contains("checksum mismatch"), "{refused}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"nuevo", "una descarga rechazada tocó el binario instalado");
        let left: Vec<_> = std::fs::read_dir(dest.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["cac-mcp"], "quedó algo más que el binario");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// La estrategia de Windows: el binario en marcha se aparta, el nuevo
    /// ocupa su sitio, y el apartado se borra (o se borra la vez siguiente).
    /// Roturas que atrapa: perder el binario viejo sin poner el nuevo; dejar
    /// el `.old` para siempre; fallar porque quedó un `.old` de antes; no
    /// poder instalar donde no había nada.
    #[test]
    fn where_a_running_binary_cannot_be_overwritten_it_is_moved_aside() {
        let dir = scratch("aside");
        let dest = dir.join("cac-mcp.exe");
        replace_as(&dest, b"primero", true).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"primero");

        // Queda un `.old` de una actualización anterior que no se pudo borrar.
        std::fs::write(dir.join("cac-mcp.exe.old"), b"de antes").unwrap();
        replace_as(&dest, b"segundo", true).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"segundo");
        let mut left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, ["cac-mcp.exe"]);

        // Un destino que no se puede ocupar (una carpeta con algo dentro):
        // falla, no toca lo que había y no deja el temporal.
        let blocked = dir.join("ocupado");
        std::fs::create_dir_all(blocked.join("dentro")).unwrap();
        assert!(replace_as(&blocked, b"x", false).is_err());
        assert!(blocked.join("dentro").is_dir());
        assert!(!dir.join(format!(".ocupado.new-{}", std::process::id())).exists(), "quedó el temporal");

        std::fs::write(dir.join("cac-mcp.exe.old"), b"apartado").unwrap();
        clear_leftovers(&dest);
        assert!(!dir.join("cac-mcp.exe.old").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Se pregunta una vez al día; sin respuesta, a la hora; y lo que se supo
    /// no se olvida porque una vez no contesten. Roturas que atrapa: preguntar
    /// en cada arranque; no volver a preguntar nunca; olvidar la versión
    /// conocida al quedarse sin red; preguntar a la red sin que toque.
    #[test]
    fn the_latest_version_is_asked_once_a_day_and_survives_being_offline() {
        let releases = FakeReleases::start();
        releases.announce("1.34.0");
        let dir = scratch("cache");
        let source = releases.source();
        let asks = || releases.asked().iter().filter(|path| *path == "/VERSION").count();

        assert_eq!(latest_known(&source, &dir, 1_000_000).as_deref(), Some("1.34.0"));
        assert_eq!(asks(), 1);
        let checked = read_checked(&dir).unwrap();
        assert!(checked.answered && checked.latest.as_deref() == Some("1.34.0"));
        let asked_at = checked.at;

        // Dentro del día: se contesta de memoria, aunque ya haya otra.
        releases.announce("1.35.0");
        assert_eq!(latest_known(&source, &dir, asked_at + CHECK_EVERY - 1).as_deref(), Some("1.34.0"));
        assert_eq!(asks(), 1, "preguntó a la red sin que tocara");

        // Pasado el día: se pregunta.
        assert_eq!(latest_known(&source, &dir, asked_at + CHECK_EVERY).as_deref(), Some("1.35.0"));
        assert_eq!(asks(), 2);

        // Sin red (un puerto cerrado): no falla, y dice lo que sabía.
        let offline = Source { version_url: "http://127.0.0.1:9/VERSION".into(), download_base: source.download_base.clone() };
        let later = read_checked(&dir).unwrap().at + CHECK_EVERY;
        assert_eq!(latest_known(&offline, &dir, later).as_deref(), Some("1.35.0"));
        let checked = read_checked(&dir).unwrap();
        assert!(!checked.answered);
        assert_eq!(checked.latest.as_deref(), Some("1.35.0"), "olvidó la versión al quedarse sin red");
        assert!(!check_is_due(Some(&checked), checked.at + RETRY_AFTER - 1));
        assert!(check_is_due(Some(&checked), checked.at + RETRY_AFTER));

        // La primera vez, sin red: nada que decir, y nada que rompa.
        let empty = scratch("cache-empty");
        assert_eq!(latest_known(&offline, &empty, 5), None);

        // Lo que contesta no es una versión: no se apunta como tal.
        releases.put("/VERSION", "<html>Sign in to the hotel WiFi</html>");
        assert!(check_now(&source, Some(&empty), Duration::from_secs(5)).is_err());
        assert_eq!(read_checked(&empty).unwrap().latest, None);
        for dir in [dir, empty] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Roturas que atrapa: ofrecer «Actualizar» sin tenerlo instalado, o
    /// «Instalar» teniéndolo; ofrecer una actualización a la misma versión o
    /// sin saber cuál es la última; instalar en otro sitio un binario que ya
    /// está en uno.
    #[test]
    fn what_is_offered_depends_on_what_is_installed_and_what_is_published() {
        let at = |version: &str| Some(Installed { path: "/opt/cac/cac-mcp".into(), version: version.into() });
        let status = |installed, latest: Option<&str>| Status { installed, latest: latest.map(str::to_string) };
        assert_eq!(status(None, Some("1.34.0")).step(), Step::Install);
        assert_eq!(status(None, None).step(), Step::Install);
        assert_eq!(status(at("1.33.0"), Some("1.34.0")).step(), Step::Update);
        assert_eq!(status(at("1.34.0"), Some("1.34.0")).step(), Step::Current);
        assert_eq!(status(at("1.35.0"), Some("1.34.0")).step(), Step::Current);
        assert_eq!(status(at("1.33.0"), None).step(), Step::Current);
    }

    /// La app instala lo mismo que el script, donde el script, y actualiza el
    /// binario donde ya esté; y lo que devuelve es lo que el binario nuevo
    /// dice de sí mismo. Roturas que atrapa: instalar una segunda copia en
    /// otra carpeta en vez de actualizar la que hay; dar por instalada una
    /// versión sin preguntársela al binario; no poder instalar la primera vez
    /// sin haber sabido antes cuál es la última; preguntar a la red cada vez
    /// que se mira dentro del plazo.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_machine_installs_where_the_script_does_and_updates_in_place() {
        let releases = FakeReleases::start();
        releases.announce("1.34.0");
        releases.publish("1.34.0", b"#!/bin/sh\necho 'cac-mcp 1.34.0'\n");
        let dir = scratch("machine");
        let (default, elsewhere) = (dir.join("home/.local/bin/cac-mcp"), dir.join("opt/cac-mcp"));
        let machine = Machine {
            source: releases.source(),
            state_dir: Some(dir.join("state")),
            look_in: vec![default.clone(), elsewhere.clone()],
            install_to: Some(default.clone()),
        };
        let asks = || releases.asked().iter().filter(|path| *path == "/VERSION").count();

        let before = machine.look(3600);
        assert_eq!(before, Status { installed: None, latest: Some("1.34.0".into()) });
        assert_eq!(machine.target(&before), Some(default.clone()));
        let installed = machine.install(&before, &mut |_| {}).unwrap();
        assert_eq!(installed, Installed { path: default.clone(), version: "1.34.0".into() });
        assert_eq!(machine.look(3600).step(), Step::Current);
        assert_eq!(asks(), 1, "dentro del plazo se contesta de memoria");

        // Sin saber la última (se miró sin red): al instalar se pregunta.
        std::fs::remove_file(&default).unwrap();
        let unknown = Status { installed: None, latest: None };
        assert_eq!(machine.install(&unknown, &mut |_| {}).unwrap().version, "1.34.0");
        assert_eq!(asks(), 2);

        // Ya está en otro sitio, más viejo: se actualiza ahí.
        std::fs::remove_file(&default).unwrap();
        std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        install(&releases.source(), "1.34.0", &elsewhere, &mut |_| {}).unwrap();
        releases.announce("1.35.0");
        releases.publish("1.35.0", b"#!/bin/sh\necho 'cac-mcp 1.35.0'\n");
        let stale = machine.look(0);
        assert_eq!(stale.step(), Step::Update);
        assert_eq!(machine.install(&stale, &mut |_| {}).unwrap(), Installed { path: elsewhere.clone(), version: "1.35.0".into() });
        assert!(!default.exists(), "instaló una segunda copia en vez de actualizar la que había");

        // Lo publicado no es un binario que arranque aquí.
        releases.announce("1.36.0");
        releases.publish("1.36.0", b"esto no arranca");
        let broken = machine.install(&machine.look(0), &mut |_| {}).unwrap_err();
        assert!(broken.contains("does not run on this machine"), "{broken}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Roturas que atrapa: instalar en una carpeta que pide administrador;
    /// que el script y la app instalen en sitios distintos; guardar el estado
    /// en la carpeta de otro programa; no respetar la carpeta pedida.
    #[test]
    fn it_installs_and_remembers_inside_the_persons_own_folders() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| PathBuf::from(v));
        // Las rutas se montan por trozos: estas pruebas también corren en
        // Windows (`rust-platforms.yml`), donde el separador es otro.
        let under = |root: &str, parts: &[&str]| Some(parts.iter().fold(PathBuf::from(root), |path, part| path.join(part)));
        let unix = env(&[("HOME", "/home/ana")]);
        assert_eq!(install_path_from("linux", &unix), under("/home/ana", &[".local", "bin", "cac-mcp"]));
        assert_eq!(install_path_from("macos", &unix), under("/home/ana", &[".local", "bin", "cac-mcp"]));
        assert_eq!(state_dir_from("linux", &unix), under("/home/ana", &[".local", "state", "cac-mcp"]));
        assert_eq!(state_dir_from("macos", &unix), under("/home/ana", &["Library", "Application Support", "cac-mcp"]));
        let xdg = env(&[("HOME", "/home/ana"), ("XDG_STATE_HOME", "/var/ana/state")]);
        assert_eq!(state_dir_from("linux", &xdg), under("/var/ana/state", &["cac-mcp"]));
        let windows = env(&[("LOCALAPPDATA", "C:/Users/ana/AppData/Local")]);
        assert_eq!(install_path_from("windows", &windows), Some(PathBuf::from("C:/Users/ana/AppData/Local").join("Programs").join("cac-mcp").join("cac-mcp.exe")));
        assert_eq!(state_dir_from("windows", &windows), Some(PathBuf::from("C:/Users/ana/AppData/Local").join("cac-mcp")));
        let asked = env(&[("HOME", "/home/ana"), ("CAC_MCP_STATE_DIR", "/tmp/otra")]);
        assert_eq!(state_dir_from("linux", &asked), Some("/tmp/otra".into()));
        assert_eq!(install_path_from("linux", &env(&[])), None);
    }

    /// Roturas que atrapa: avisar de una versión que no es más nueva; un
    /// aviso sin la orden exacta; que la orden no valga cuando el binario no
    /// está en el `PATH`; un aviso que prometa que se actualiza solo.
    #[test]
    fn the_agent_is_told_of_a_newer_version_with_the_exact_command() {
        assert_eq!(update_note("1.33.0", None, None), None);
        assert_eq!(update_note("1.33.0", Some("1.33.0"), None), None);
        assert_eq!(update_note("1.34.0", Some("1.33.0"), None), None);
        let note = update_note("1.33.0", Some("1.34.0"), Some(Path::new("/home/ana/.local/bin/cac-mcp"))).unwrap();
        assert!(note.contains("cac-mcp 1.34.0") && note.contains("is 1.33.0"), "{note}");
        assert!(note.contains("`\"/home/ana/.local/bin/cac-mcp\" update`"), "{note}");
        assert!(note.contains("Nothing updates by itself"), "{note}");
        assert!(update_note("1.33.0", Some("1.34.0"), None).unwrap().contains("`cac-mcp update`"));
    }

    /// Roturas que atrapa: una orden con un token dentro (esta lógica no ve
    /// ninguno); una orden que no es la que entiende Claude Code; tomar por
    /// `cac-mcp` un programa que contesta otra cosa a `--version`.
    #[test]
    fn the_register_command_marks_where_the_token_goes_and_never_carries_one() {
        let command = register_command("https://cac.example/", Path::new("/home/ana/.local/bin/cac-mcp"));
        assert_eq!(command, "claude mcp add cac -e CAC_URL=https://cac.example -e CAC_TOKEN=PASTE_YOUR_TOKEN -- \"/home/ana/.local/bin/cac-mcp\"");
        assert_eq!(version_line("cac-mcp 1.33.0\navailable: 1.34.0\n").as_deref(), Some("1.33.0"));
        assert_eq!(version_line("git version 2.50.0\n"), None);
        assert_eq!(version_line("cac-mcp desconocida\n"), None);
        assert_eq!(version_line(""), None);
    }

    /// Busca el binario preguntándole la versión, y se salta lo que no es
    /// `cac-mcp`. Roturas que atrapa: dar por instalado un fichero que no
    /// arranca; quedarse con el primero del `PATH` aunque no conteste.
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn an_installed_binary_is_one_that_answers_with_its_version() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("find");
        let script = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        let other = script("otro", "echo 'otro programa 3.0'");
        let ours = script("cac-mcp", "echo 'cac-mcp 1.33.0'");
        let plain = dir.join("no-ejecutable");
        std::fs::write(&plain, "cac-mcp 9.9.9").unwrap();
        let found = find_among([dir.join("no-existe"), plain, other, ours.clone()].into_iter());
        assert_eq!(found, Some(Installed { path: ours, version: "1.33.0".into() }));
        assert_eq!(find_among([dir.join("no-existe")].into_iter()), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
