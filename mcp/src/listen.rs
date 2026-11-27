//! `cac-mcp listen`: escuchar a cac sin vencer ni perder eventos
//! (command-and-control-558).
//!
//! - `listen run` abre el stream de eventos de la persona (SSE, con su token) y
//!   escribe cada evento como una línea JSON en `~/.cache/cac/events.jsonl`. Es
//!   lo que corre el servicio de usuario.
//! - `listen follow --consumer <nombre>` imprime lo que ese consumidor no ha
//!   confirmado y sigue el fichero: es lo que arma el Monitor de la sesión.
//! - `listen ack --consumer <nombre> <seq>` confirma hasta ese evento.
//!
//! Todo va por `seq`, el número del diario de la persona en el backend: sin
//! huecos y creciente. El fichero se escribe en orden de `seq`, una línea por
//! evento, y el cursor de quien escribe (`listen.json`) y el de cada
//! consumidor (`consumers/<nombre>.json`) son un `seq`, nunca una posición en
//! bytes: así la compactación puede reescribir el fichero sin que nadie lea
//! dos veces ni se salte nada.
//!
//! El token no se escribe en ningún sitio: ni en el fichero, ni en el estado,
//! ni en la salida. Se relee en cada conexión (del entorno o del
//! `.claude.json`), así que un token cambiado se toma sin reinstalar nada.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// El tipo de la línea que dice «aquí faltan eventos que ya no se pueden
/// recuperar»: la primera vez (se empieza en el presente, sin el pasado) o
/// cuando el diario del backend ya no guarda lo que falta. Pide un barrido a
/// mano.
pub const GAP: &str = "cac:gap";

const KEEP_DAYS: u64 = 14;
const KEEP_BYTES: u64 = 20 * 1024 * 1024;
const BACKLOG_PAGE: u64 = 500;
/// El backend late cada 25 s; sin un byte en este rato, la conexión está
/// muerta aunque el socket no lo diga.
const IDLE: Duration = Duration::from_secs(75);
const DEFAULT_URL: &str = "https://cac.guz-studio.dev";
/// El evento con el que el backend avisa, antes de colgar, de que el token
/// dejó de valer.
const TOKEN_REVOKED: &str = "token:revoked";
/// La razón de un `cac:gap` local que dice que el diario del backend volvió a
/// empezar: el `seq` baja, y lo confirmado de cada consumidor también.
pub const RESET: &str = "reset";

const USAGE: &str = "Usage: cac-mcp listen run | follow --consumer <name> [--project <id|name>] [--type <type>] [--mine] [--once] | ack --consumer <name> <seq>";

// ── Dónde vive cada cosa ────────────────────────────────────────────────────

/// El fichero de eventos: `$XDG_CACHE_HOME/cac/events.jsonl`, o
/// `~/.cache/cac/events.jsonl`; en Windows, en `%LOCALAPPDATA%\cac`.
pub fn events_file() -> Option<PathBuf> {
    events_file_from(std::env::consts::OS, &env_path)
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

fn events_file_from(os: &str, env: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let base = match os {
        "windows" => env("LOCALAPPDATA")?,
        _ => env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|home| home.join(".cache")))?,
    };
    Some(base.join("cac").join("events.jsonl"))
}

// ── Credenciales ────────────────────────────────────────────────────────────

/// Con quién se habla y con qué token. `Debug` no lo enseña: un `{:?}` en un
/// mensaje de error no puede soltarlo.
#[derive(Clone, PartialEq)]
pub struct Credentials {
    pub base: String,
    pub token: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").field("base", &self.base).field("token", &"<hidden>").finish()
    }
}

/// `CAC_TOKEN` y `CAC_URL` del entorno si el token está; si no, los de
/// `mcpServers.cac.env` del `.claude.json` (el de `CLAUDE_CONFIG_DIR` y, si no
/// lo tiene, el de casa), igual que los hooks (`cac_hooks.py`, `cac_config`).
/// Un servicio de systemd no hereda el entorno de la sesión: por eso el
/// fichero, y no una variable en la unidad.
pub fn credentials() -> Result<Credentials, String> {
    credentials_from(&|name| std::env::var(name).ok().filter(|v| !v.is_empty()), &|path| fs::read_to_string(path).ok())
}

fn credentials_from(env: &dyn Fn(&str) -> Option<String>, read: &dyn Fn(&Path) -> Option<String>) -> Result<Credentials, String> {
    let clean = |url: String| url.trim_end_matches('/').to_string();
    if let Some(token) = env("CAC_TOKEN") {
        return Ok(Credentials { base: clean(env("CAC_URL").unwrap_or_else(|| DEFAULT_URL.into())), token });
    }
    let dirs = [env("CLAUDE_CONFIG_DIR"), env("HOME")];
    for dir in dirs.into_iter().flatten() {
        let Some(text) = read(&Path::new(&dir).join(".claude.json")) else { continue };
        let Ok(config) = serde_json::from_str::<Value>(&text) else { continue };
        let vars = &config["mcpServers"]["cac"]["env"];
        if let Some(token) = vars["CAC_TOKEN"].as_str().filter(|t| !t.is_empty()) {
            let base = vars["CAC_URL"].as_str().filter(|u| !u.is_empty()).unwrap_or(DEFAULT_URL);
            return Ok(Credentials { base: clean(base.to_string()), token: token.to_string() });
        }
    }
    Err("no cac token: set CAC_TOKEN, or connect Claude Code to cac (mcpServers.cac in .claude.json)".into())
}

// ── El diario local ─────────────────────────────────────────────────────────

/// El fichero de eventos y el cursor de quien lo escribe.
pub struct Journal {
    events: PathBuf,
    state_dir: PathBuf,
}

impl Journal {
    pub fn new(events: PathBuf, state_dir: PathBuf) -> Self {
        Journal { events, state_dir }
    }

    pub fn events_path(&self) -> &Path {
        &self.events
    }

    fn cursor_path(&self) -> PathBuf {
        self.state_dir.join("listen.json")
    }

    pub fn saved_cursor(&self) -> Option<u64> {
        let text = fs::read_to_string(self.cursor_path()).ok()?;
        serde_json::from_str::<Value>(&text).ok()?["cursor"].as_u64()
    }

    /// El último `seq` que ya está a salvo, al arrancar. Primero quita una
    /// línea a medias (el proceso murió escribiéndola: no se entregó, y el
    /// stream la vuelve a mandar). Después, la última línea manda sobre el
    /// cursor guardado: la línea se escribe antes que el cursor, así que si
    /// murió entre las dos, la línea es lo último que pasó. Y no vale el mayor
    /// de los dos: tras un reinicio del diario del backend (`reset`) la línea
    /// está por **debajo** del cursor viejo, y subirlo otra vez se tragaría
    /// todo lo que llegue hasta alcanzarlo. Sin líneas (la compactación lo
    /// vació todo), el cursor guardado. `None` es que no hay ni fichero ni
    /// estado: la primera vez.
    pub fn recover(&self) -> io::Result<Option<u64>> {
        let from_file = self.trim_and_last_seq()?;
        Ok(from_file.or_else(|| self.saved_cursor()))
    }

    /// El `seq` de la última línea entera, sin tocar el fichero. Lee sólo el
    /// final: una línea cabe de sobra en 256 KB (el backend corta `data` a
    /// 32 KB).
    pub fn last_seq(&self) -> io::Result<Option<u64>> {
        let mut file = match File::open(&self.events) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let len = file.metadata()?.len();
        file.seek(SeekFrom::Start(len.saturating_sub(256 * 1024)))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        let whole = tail.iter().rposition(|b| *b == b'\n').map(|i| &tail[..i]).unwrap_or(&[]);
        Ok(whole.split(|b| *b == b'\n').rev().find_map(|line| serde_json::from_slice::<Value>(line).ok().and_then(|v| v["seq"].as_u64())))
    }

    fn trim_and_last_seq(&self) -> io::Result<Option<u64>> {
        let mut file = match OpenOptions::new().read(true).write(true).open(&self.events) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let len = file.metadata()?.len();
        // Una línea cabe de sobra: el backend corta `data` a 32 KB.
        let start = len.saturating_sub(4 * 1024 * 1024);
        file.seek(SeekFrom::Start(start))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        if !tail.is_empty() && !tail.ends_with(b"\n") {
            let keep = tail.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
            tail.truncate(keep);
            file.set_len(start + keep as u64)?;
            file.sync_all()?;
        }
        Ok(tail.split(|b| *b == b'\n').rev().find_map(|line| serde_json::from_slice::<Value>(line).ok().and_then(|v| v["seq"].as_u64())))
    }

    /// Añade el evento como una línea y la lleva al disco. El cursor va
    /// aparte, después (`save_cursor`): ese orden es el que hace que una caída
    /// en medio no pierda ni repita (ver `recover`).
    pub fn append(&self, event: &Value) -> io::Result<()> {
        self.append_with(event, &mut |file, line| {
            file.write_all(line)?;
            file.sync_data()
        })
    }

    /// `append` con la escritura de fuera, para poder hacerla fallar a mitad.
    /// Si falla (el disco lleno), el fichero vuelve a su largo de antes: el
    /// reintento sigue en este proceso, sin pasar por `recover`, y una línea
    /// a medias se pegaría a la siguiente y la dejaría ilegible.
    fn append_with(&self, event: &Value, write: &mut dyn FnMut(&mut File, &[u8]) -> io::Result<()>) -> io::Result<()> {
        if let Some(dir) = self.events.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut line = serde_json::to_vec(event).map_err(io::Error::other)?;
        line.push(b'\n');
        let mut file = private_options().append(true).create(true).open(&self.events)?;
        let before = file.metadata()?.len();
        write(&mut file, &line).inspect_err(|_| {
            let _ = file.set_len(before);
        })
    }

    /// Empieza el fichero de nuevo con sólo esta línea (el diario del backend
    /// se reinició: lo de antes lleva números de otro diario). Escribe al lado
    /// y renombra, como la compactación: quien sigue el fichero ve otro inodo.
    fn restart_with(&self, event: &Value) -> io::Result<()> {
        if let Some(dir) = self.events.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.events.with_extension("jsonl.reset");
        {
            let mut out = private_options().write(true).create(true).truncate(true).open(&tmp)?;
            let mut line = serde_json::to_vec(event).map_err(io::Error::other)?;
            line.push(b'\n');
            out.write_all(&line)?;
            out.sync_all()?;
        }
        fs::rename(&tmp, &self.events)
    }

    pub fn save_cursor(&self, seq: u64) -> io::Result<()> {
        write_atomic(&self.cursor_path(), &json!({ "cursor": seq }))
    }

    /// Lo que hace con cada evento quien escucha.
    pub fn record(&self, event: &Value) -> io::Result<()> {
        let seq = event["seq"].as_u64().ok_or_else(|| io::Error::other("an event without seq"))?;
        self.append(event)?;
        self.save_cursor(seq)
    }

    /// Deja fuera lo de más de 14 días y, si aun así pasa de 20 MB, lo más
    /// viejo. Reescribe y renombra: quien sigue el fichero ve otro inodo y lo
    /// relee por `seq`, sin repetir.
    pub fn compact(&self, now: u64) -> io::Result<()> {
        compact_file(&self.events, now, KEEP_DAYS * 86_400, KEEP_BYTES)
    }
}

fn compact_file(path: &Path, now: u64, max_age: u64, max_bytes: u64) -> io::Result<()> {
    let text = match fs::read(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let lines: Vec<&[u8]> = text.split_inclusive(|b| *b == b'\n').collect();
    let oldest = now.saturating_sub(max_age);
    let mut kept: Vec<&[u8]> = lines
        .iter()
        .copied()
        .filter(|line| {
            let at = serde_json::from_slice::<Value>(line).ok().and_then(|v| v["at"].as_str().and_then(unix_of_iso));
            at.is_none_or(|at| at >= oldest)
        })
        .collect();
    let mut size: u64 = kept.iter().map(|l| l.len() as u64).sum();
    let mut drop = 0;
    while size > max_bytes && drop < kept.len() {
        size -= kept[drop].len() as u64;
        drop += 1;
    }
    kept.drain(..drop);
    if kept.len() == lines.len() {
        return Ok(());
    }
    let tmp = path.with_extension("jsonl.compact");
    {
        let mut out = private_options().write(true).create(true).truncate(true).open(&tmp)?;
        for line in &kept {
            out.write_all(line)?;
        }
        out.sync_all()?;
    }
    fs::rename(&tmp, path)
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Escribe al lado y renombra: quien lee ve el estado viejo o el nuevo, nunca
/// uno a medias.
fn write_atomic(path: &Path, value: &Value) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = private_options().write(true).create(true).truncate(true).open(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

// ── Fechas (sin dependencias) ───────────────────────────────────────────────

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// Howard Hinnant, «chrono-Compatible Low-Level Date Algorithms».
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn iso_of_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Los segundos de un `2026-11-27T09:13:17.78Z`. Lo que no lo es, `None`.
fn unix_of_iso(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| text.get(r)?.parse::<i64>().ok();
    let days = days_from_civil(num(0..4)?, num(5..7)?, num(8..10)?);
    let secs = days * 86_400 + num(11..13)? * 3600 + num(14..16)? * 60 + num(17..19)?;
    u64::try_from(secs).ok()
}

// ── El stream ───────────────────────────────────────────────────────────────

/// Un mensaje de SSE ya entero.
#[derive(Debug, Default, PartialEq)]
pub struct SseMessage {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
}

/// Junta los trozos que llegan por la red en mensajes de SSE. Un trozo puede
/// cortar una línea, o un carácter, por la mitad.
#[derive(Default)]
pub struct SseParser {
    pending: Vec<u8>,
    current: SseMessage,
    has_data: bool,
}

impl SseParser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseMessage> {
        self.pending.extend_from_slice(bytes);
        let mut done = Vec::new();
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&raw[..raw.len() - 1]);
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.is_empty() {
                if self.has_data {
                    done.push(std::mem::take(&mut self.current));
                } else {
                    self.current = SseMessage::default();
                }
                self.has_data = false;
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').map(|(f, v)| (f, v.strip_prefix(' ').unwrap_or(v))).unwrap_or((line, ""));
            match field {
                "id" => self.current.id = Some(value.to_string()),
                "event" => self.current.event = Some(value.to_string()),
                "data" => {
                    if self.has_data {
                        self.current.data.push('\n');
                    }
                    self.current.data.push_str(value);
                    self.has_data = true;
                }
                _ => {}
            }
        }
        done
    }
}

/// Por qué se cortó una conexión.
#[derive(Debug, PartialEq)]
pub enum Failure {
    /// 401 o 403: el token no vale o no tiene `events:read`. Reintentar
    /// enseguida no lo arregla; sólo lo arregla una persona.
    Auth { status: u16, code: String },
    /// Todo lo demás: la red, un 5xx, un stream cerrado.
    Transient(String),
    /// No hay token que usar.
    NoCredentials(String),
}

/// Cuánto esperar antes del intento `attempt` (0 es el primero tras fallar).
/// Un «no» del servidor espera minutos: un token revocado no se reintenta en
/// bucle (el incidente del 401 a media sesión, 385a98f3).
pub fn retry_delay(failure: &Failure, attempt: u32) -> Duration {
    let grow = |base: u64, max: u64| Duration::from_secs(base.saturating_mul(1u64 << attempt.min(16)).min(max));
    match failure {
        Failure::Auth { .. } | Failure::NoCredentials(_) => grow(300, 1800),
        Failure::Transient(_) => grow(1, 60),
    }
}

/// Lo que `listen status` (C2) lee: cómo está quien escucha, sin el token.
struct Status {
    path: PathBuf,
    last_connected: Option<u64>,
    last_event: Option<u64>,
}

impl Status {
    fn write(&self, state: &str, reason: Option<&str>, cursor: Option<u64>, retry_in: Option<Duration>) {
        let _ = write_atomic(
            &self.path,
            &json!({
                "state": state,
                "reason": reason,
                "cursor": cursor,
                "lastConnectedAt": self.last_connected.map(iso_of_unix),
                "lastEventAt": self.last_event.map(iso_of_unix),
                "retryInSeconds": retry_in.map(|d| d.as_secs()),
                "updatedAt": iso_of_unix(now()),
                "pid": std::process::id(),
            }),
        );
    }
}

/// Quien escucha: el diario local y por dónde va.
pub struct Listener {
    journal: Journal,
    status: Status,
    /// `None` hasta saber dónde empezar: sin fichero ni estado, en el presente.
    cursor: Option<u64>,
    client: reqwest::Client,
}

/// Una página del diario del backend. Su `gap` no hace falta: lo perdido
/// viene dentro de `events`, como un `cac:gap` que ocupa su último número.
struct Backlog {
    events: Vec<Value>,
    head: u64,
}

impl Listener {
    pub fn new(journal: Journal) -> io::Result<Self> {
        let cursor = journal.recover()?;
        if let Some(cursor) = cursor {
            // Lo recuperado de la línea, a salvo antes de compactar: si la
            // compactación deja el fichero vacío, el cursor sigue sabiéndose.
            journal.save_cursor(cursor)?;
        }
        journal.compact(now())?;
        let status = Status { path: journal.state_dir.join("listen.status.json"), last_connected: None, last_event: None };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("cac-mcp/", env!("CARGO_PKG_VERSION"), " listen"))
            .build()
            .map_err(io::Error::other)?;
        Ok(Listener { journal, status, cursor, client })
    }

    /// Escucha para siempre, reconectando. Lo único que la para es matar el
    /// proceso, y eso no pierde nada (ver `Journal::recover`).
    pub async fn run(mut self, credentials: &dyn Fn() -> Result<Credentials, String>) {
        let mut attempt = 0u32;
        loop {
            let failure = match credentials() {
                Err(e) => Failure::NoCredentials(e),
                Ok(creds) => match self.session(&creds).await {
                    // Conectó y luego se cortó: se vuelve a empezar de cero.
                    (true, failure) => {
                        attempt = 0;
                        failure
                    }
                    (false, failure) => failure,
                },
            };
            let wait = retry_delay(&failure, attempt);
            let (state, reason) = match &failure {
                Failure::Auth { status, code } => ("unauthorized", format!("cac answered {status} {code}: the token is not valid or lacks events:read")),
                Failure::NoCredentials(e) => ("unauthorized", e.clone()),
                Failure::Transient(e) => ("retrying", e.clone()),
            };
            eprintln!("cac-mcp listen: {reason}; retrying in {} s", wait.as_secs());
            self.status.write(state, Some(&reason), self.cursor, Some(wait));
            attempt = attempt.saturating_add(1);
            tokio::time::sleep(wait).await;
        }
    }

    /// Una conexión: ponerse al día con el diario, y después en vivo. Dice si
    /// llegó a conectar y por qué terminó.
    async fn session(&mut self, creds: &Credentials) -> (bool, Failure) {
        if let Err(f) = self.catch_up(creds).await {
            return (false, f);
        }
        let mut request = self.client.get(format!("{}/api/v1/events", creds.base)).bearer_auth(&creds.token).header("Accept", "text/event-stream");
        if let Some(cursor) = self.cursor {
            request = request.header("Last-Event-ID", cursor.to_string());
        }
        let mut response = match request.send().await {
            Ok(r) => r,
            Err(e) => return (false, Failure::Transient(format!("could not connect: {}", without_url(&e)))),
        };
        if let Some(f) = refused(response.status().as_u16()) {
            return (false, f(code_of(&response.text().await.unwrap_or_default())));
        }
        self.status.last_connected = Some(now());
        self.status.write("connected", None, self.cursor, None);
        let mut parser = SseParser::default();
        loop {
            let chunk = match tokio::time::timeout(IDLE, response.chunk()).await {
                Err(_) => return (true, Failure::Transient("no heartbeat from cac".into())),
                Ok(Err(e)) => return (true, Failure::Transient(format!("the stream broke: {}", without_url(&e)))),
                Ok(Ok(None)) => return (true, Failure::Transient("cac closed the stream".into())),
                Ok(Ok(Some(chunk))) => chunk,
            };
            for message in parser.feed(&chunk) {
                // El backend lo manda justo antes de colgar, al ver en un
                // latido que el token ya no vale: es un «no», no un corte.
                if message.event.as_deref() == Some(TOKEN_REVOKED) {
                    return (true, Failure::Auth { status: 401, code: TOKEN_REVOKED.into() });
                }
                if let Err(f) = self.deliver(creds, &message).await {
                    return (true, f);
                }
            }
        }
    }

    /// Un evento del stream. El que sigue al cursor se escribe; uno ya escrito
    /// se salta; uno que se adelanta tiene detrás un hueco (el hub del backend
    /// tiró algo), que se rellena del diario antes de seguir. Si el diario
    /// tampoco lo tiene, se reconecta desde el cursor.
    async fn deliver(&mut self, creds: &Credentials, message: &SseMessage) -> Result<(), Failure> {
        let Ok(event) = serde_json::from_str::<Value>(&message.data) else { return Ok(()) };
        if self.behind_the_head(&event) {
            return self.reset(&event);
        }
        let Some(seq) = event["seq"].as_u64().filter(|s| *s > 0) else { return Ok(()) };
        if seq <= self.cursor.unwrap_or(0) {
            return Ok(());
        }
        if seq > self.cursor.unwrap_or(0) + 1 {
            self.catch_up(creds).await?;
            if seq <= self.cursor.unwrap_or(0) {
                return Ok(());
            }
            if seq != self.cursor.unwrap_or(0) + 1 {
                return Err(Failure::Transient(format!("event {seq} arrived out of order; reconnecting from {}", self.cursor.unwrap_or(0))));
            }
        }
        self.write(&event, seq)
    }

    fn write(&mut self, event: &Value, seq: u64) -> Result<(), Failure> {
        self.journal.record(event).map_err(|e| Failure::Transient(format!("could not write {}: {e}", self.journal.events.display())))?;
        self.cursor = Some(seq);
        self.status.last_event = Some(now());
        Ok(())
    }

    /// Una línea `cac:gap`: de `from` a `to` no hay nada y ya no se puede
    /// recuperar. Lleva `seq = to`, para que quien lee la vea una vez y pueda
    /// confirmarla como cualquier otra.
    fn write_gap(&mut self, from: u64, to: u64, reason: &str) -> Result<(), Failure> {
        let line = gap_line(format!("{GAP}:{to}"), from, to, json!({ "reason": reason }));
        self.write(&line, to)
    }

    /// Un `cac:gap` del backend con un número por detrás del cursor: el
    /// diario del backend volvió a empezar (se borró, se restauró de un
    /// respaldo, o el token es de otra persona) y su cabecera es ese número.
    /// Igual al cursor no: eso es el mismo hueco que ya se escribió.
    fn behind_the_head(&self, event: &Value) -> bool {
        event["type"] == GAP && event["seq"].as_u64().is_some_and(|seq| seq < self.cursor.unwrap_or(0))
    }

    /// El diario del backend volvió a empezar en `seq`. El cursor **baja**
    /// allí (si no, todo lo que llegue hasta el número viejo se saltaría como
    /// ya escrito) y el fichero vuelve a empezar con una línea `cac:gap` de
    /// razón `reset`: lo de antes lleva números del diario viejo. Su `id` es
    /// único, porque es lo que `follow` usa para saber si ya bajó lo
    /// confirmado de un consumidor por este reinicio.
    fn reset(&mut self, event: &Value) -> Result<(), Failure> {
        let seq = event["seq"].as_u64().unwrap_or(0);
        let previous = self.cursor.unwrap_or(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let line = gap_line(format!("{GAP}:reset:{seq}:{nanos}"), seq, seq, json!({ "reason": RESET, "previousCursor": previous }));
        let failed = |e: io::Error| Failure::Transient(format!("could not write {}: {e}", self.journal.events.display()));
        self.journal.restart_with(&line).map_err(failed)?;
        self.journal.save_cursor(seq).map_err(failed)?;
        eprintln!("cac-mcp listen: cac's event journal starts again at {seq} (we were at {previous})");
        self.cursor = Some(seq);
        self.status.last_event = Some(now());
        Ok(())
    }

    /// Lee del diario del backend lo que falta desde el cursor. La primera vez
    /// (sin cursor) no trae el pasado: empieza en el presente y lo dice con
    /// una línea `cac:gap`.
    async fn catch_up(&mut self, creds: &Credentials) -> Result<(), Failure> {
        let Some(mut cursor) = self.cursor else {
            let page = self.backlog(creds, 0, 1).await?;
            if page.head > 0 {
                self.write_gap(1, page.head, "first-run")?;
            } else {
                self.journal.save_cursor(0).map_err(|e| Failure::Transient(e.to_string()))?;
                self.cursor = Some(0);
            }
            return Ok(());
        };
        loop {
            let page = self.backlog(creds, cursor, BACKLOG_PAGE).await?;
            let mut events = page.events;
            events.sort_by_key(|e| e["seq"].as_u64().unwrap_or(0));
            let before = cursor;
            for event in events {
                if self.behind_the_head(&event) {
                    self.reset(&event)?;
                    cursor = self.cursor.unwrap_or(0);
                    continue;
                }
                let Some(seq) = event["seq"].as_u64().filter(|s| *s > cursor) else { continue };
                // Lo que el diario ya no guarda llega como un `cac:gap` que
                // ocupa el último número perdido: ése es el hueco, y no se
                // escribe otro delante.
                if seq > cursor + 1 && event["type"] != GAP {
                    self.write_gap(cursor + 1, seq - 1, "missing")?;
                }
                self.write(&event, seq)?;
                cursor = seq;
            }
            if cursor == before || cursor >= page.head {
                return Ok(());
            }
        }
    }

    async fn backlog(&self, creds: &Credentials, after: u64, limit: u64) -> Result<Backlog, Failure> {
        let url = format!("{}/api/v1/events/backlog?after={after}&limit={limit}", creds.base);
        let response = self
            .client
            .get(url)
            .bearer_auth(&creds.token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| Failure::Transient(format!("could not read the backlog: {}", without_url(&e))))?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if let Some(f) = refused(status) {
            return Err(f(code_of(&text)));
        }
        if status != 200 {
            return Err(Failure::Transient(format!("the backlog answered {status}")));
        }
        let body: Value = serde_json::from_str(&text).map_err(|_| Failure::Transient("the backlog did not answer JSON".into()))?;
        // La API envuelve en `data`; se acepta también sin sobre.
        let body = if body.get("data").is_some_and(Value::is_object) { &body["data"] } else { &body };
        Ok(Backlog {
            events: body["events"].as_array().cloned().unwrap_or_default(),
            head: body["head"].as_u64().unwrap_or(0),
        })
    }
}

/// Una línea `cac:gap` escrita aquí, con la forma del sobre del backend
/// (`orgId` y `key` vacíos, nunca nulos).
fn gap_line(id: String, from: u64, to: u64, extra: Value) -> Value {
    let mut data = json!({ "from": from, "to": to });
    if let (Some(data), Value::Object(extra)) = (data.as_object_mut(), extra) {
        data.extend(extra);
    }
    json!({
        "seq": to,
        "id": id,
        "type": GAP,
        "at": iso_of_unix(now()),
        "orgId": "",
        "key": "",
        "mine": false,
        "external": false,
        "data": data,
    })
}

fn refused(status: u16) -> Option<impl FnOnce(String) -> Failure> {
    matches!(status, 401 | 403).then_some(move |code| Failure::Auth { status, code })
}

/// El código de error de una respuesta (`missing-scope:events:read`), sólo si
/// tiene forma de código: nada de lo que contesta el servidor se copia entero.
fn code_of(body: &str) -> String {
    let body: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = ["error", "code"].iter().find_map(|k| body[*k].as_str().or_else(|| body["error"][*k].as_str())).unwrap_or("");
    if !code.is_empty() && code.len() <= 80 && code.chars().all(|c| c.is_ascii_alphanumeric() || "-_:.".contains(c)) {
        code.to_string()
    } else {
        String::new()
    }
}

/// El error de reqwest por su clase, sin la dirección ni nada de la consulta.
fn without_url(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "connection failed"
    } else if e.is_body() || e.is_decode() {
        "the body broke"
    } else {
        "request failed"
    }
}

// ── Leer: follow y ack ──────────────────────────────────────────────────────

/// Qué quiere ver quien lee. El fichero guarda todo; filtra quien lee.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub projects: Vec<String>,
    pub types: Vec<String>,
    pub mine: bool,
}

impl Filter {
    /// Una línea `cac:gap` pasa siempre: le importa a todo el que lee.
    pub fn passes(&self, event: &Value) -> bool {
        let kind = event["type"].as_str().unwrap_or("");
        if kind == GAP {
            return true;
        }
        if self.mine && event["mine"].as_bool() != Some(true) {
            return false;
        }
        if !self.types.is_empty() && !self.types.iter().any(|t| t == kind) {
            return false;
        }
        if !self.projects.is_empty() {
            let project = &event["data"]["project"];
            let names: Vec<String> = [&project["id"], &project["name"], &project["spaceId"], &event["data"]["spaceId"]]
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.to_lowercase()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect();
            if !self.projects.iter().any(|p| names.contains(&p.to_lowercase())) {
                return false;
            }
        }
        true
    }
}

/// Un nombre de consumidor es el nombre de un fichero: nada de rutas.
pub fn valid_consumer(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && !name.starts_with('.') && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn consumer_path(state_dir: &Path, consumer: &str) -> PathBuf {
    state_dir.join("consumers").join(format!("{consumer}.json"))
}

/// Por dónde va un consumidor: hasta qué `seq` confirmó (`None`: nada desde
/// el último reinicio, así que hasta el `seq` 0 se le entrega) y el `id` de
/// la última línea `cac:gap` de reinicio que ya le bajó lo confirmado.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Progress {
    pub acked: Option<u64>,
    pub epoch: Option<String>,
}

pub fn progress(state_dir: &Path, consumer: &str) -> Progress {
    let value = fs::read_to_string(consumer_path(state_dir, consumer)).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()).unwrap_or(Value::Null);
    Progress { acked: value["acked"].as_u64(), epoch: value["epoch"].as_str().map(String::from) }
}

fn save_progress(state_dir: &Path, consumer: &str, progress: &Progress) -> io::Result<()> {
    write_atomic(&consumer_path(state_dir, consumer), &json!({ "acked": progress.acked, "epoch": progress.epoch }))
}

pub fn acked(state_dir: &Path, consumer: &str) -> u64 {
    progress(state_dir, consumer).acked.unwrap_or(0)
}

/// Confirma hasta `seq`. Nunca retrocede: confirmar uno viejo (un Monitor que
/// llega tarde) deja lo que había. Devuelve lo que queda confirmado.
pub fn ack(state_dir: &Path, consumer: &str, seq: u64) -> io::Result<u64> {
    let mut progress = progress(state_dir, consumer);
    let kept = progress.acked.map_or(seq, |acked| acked.max(seq));
    progress.acked = Some(kept);
    save_progress(state_dir, consumer, &progress)?;
    Ok(kept)
}

/// Una línea `cac:gap` de reinicio (ver `Listener::reset`).
fn is_reset(event: &Value) -> bool {
    event["type"] == GAP && event["data"]["reason"] == RESET
}

/// Por dónde va la lectura del fichero: qué fichero (su inodo) y hasta qué
/// byte. Si el fichero cambia de inodo o se encoge (la compactación), se
/// vuelve a leer desde el principio: lo que no repite es el `seq`.
struct Tail {
    file: Option<File>,
    id: u64,
    pos: u64,
    partial: Vec<u8>,
}

#[cfg(unix)]
fn file_id(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino()
}

#[cfg(not(unix))]
fn file_id(_: &fs::Metadata) -> u64 {
    0
}

impl Tail {
    /// Las líneas enteras nuevas desde la última vez.
    fn read(&mut self, path: &Path) -> io::Result<Vec<Vec<u8>>> {
        let meta = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        // Otro inodo es otro fichero (la compactación o un reinicio lo
        // renombran encima), aunque ya sea tan largo como `pos`: seguir desde
        // ese byte caería a mitad de una línea.
        if self.file.is_none() || file_id(&meta) != self.id || meta.len() < self.pos {
            self.file = Some(File::open(path)?);
            self.id = file_id(&meta);
            self.pos = 0;
            self.partial.clear();
        }
        let file = self.file.as_mut().expect("abierto arriba");
        // Media línea leída que ya no está: `recover` la cortó (el que
        // escribía murió) y se escribió otra encima, en el mismo inodo. El
        // corte cae justo donde empezaba, así que se vuelve a leer desde ahí.
        if !self.partial.is_empty() {
            let start = self.pos - self.partial.len() as u64;
            let mut there = vec![0; self.partial.len()];
            file.seek(SeekFrom::Start(start))?;
            if file.read_exact(&mut there).is_err() || there != self.partial {
                self.pos = start;
                self.partial.clear();
            }
        }
        file.seek(SeekFrom::Start(self.pos))?;
        let mut fresh = Vec::new();
        file.read_to_end(&mut fresh)?;
        self.pos += fresh.len() as u64;
        self.partial.extend_from_slice(&fresh);
        let mut lines = Vec::new();
        while let Some(end) = self.partial.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            lines.push(line[..line.len() - 1].to_vec());
        }
        Ok(lines)
    }
}

/// Imprime, una por línea, las líneas con `seq` mayor que lo que ese
/// consumidor confirmó y que pasan el filtro, y sigue el fichero. Con `once`,
/// imprime lo que hay y vuelve. `keep_going` la para (las pruebas).
///
/// Una línea `cac:gap` de reinicio que este consumidor aún no vio se entrega
/// aunque su `seq` no pase de lo confirmado, y **baja** lo confirmado: los
/// números vuelven a empezar, y sin bajarlo todo lo que llegue hasta el
/// número viejo se daría por visto.
pub fn follow(events: &Path, state_dir: &Path, consumer: &str, filter: &Filter, once: bool, out: &mut dyn Write, poll: Duration, keep_going: &dyn Fn() -> bool) -> io::Result<()> {
    let mut progress = progress(state_dir, consumer);
    let mut seen = progress.acked;
    let mut tail = Tail { file: None, id: 0, pos: 0, partial: Vec::new() };
    loop {
        for line in tail.read(events)? {
            let Ok(event) = serde_json::from_slice::<Value>(&line) else { continue };
            let Some(seq) = event["seq"].as_u64() else { continue };
            let new_epoch = is_reset(&event).then(|| event["id"].as_str().unwrap_or("").to_string()).filter(|id| progress.epoch.as_ref() != Some(id));
            if let Some(epoch) = new_epoch {
                // Lo confirmado queda en «nada»: la línea de reinicio sigue
                // pendiente hasta que el consumidor la confirme.
                progress = Progress { acked: None, epoch: Some(epoch) };
                save_progress(state_dir, consumer, &progress)?;
            } else if seen.is_some_and(|seen| seq <= seen) {
                continue;
            }
            seen = Some(seq);
            if filter.passes(&event) {
                out.write_all(&line)?;
                out.write_all(b"\n")?;
            }
        }
        out.flush()?;
        if once || !keep_going() {
            return Ok(());
        }
        std::thread::sleep(poll);
    }
}

// ── La orden ────────────────────────────────────────────────────────────────

/// `cac-mcp listen …`. Devuelve el código de salida.
pub fn main(args: &[&str]) -> i32 {
    match args {
        ["run"] => run_command(),
        ["follow", rest @ ..] => follow_command(rest),
        ["ack", rest @ ..] => ack_command(rest),
        _ => {
            eprintln!("cac-mcp listen: unknown arguments `{}`. {USAGE}", args.join(" "));
            2
        }
    }
}

fn dirs() -> Result<(PathBuf, PathBuf), String> {
    let events = events_file().ok_or("cac-mcp listen: no home folder to keep the events in")?;
    let state = crate::update::state_dir().ok_or("cac-mcp listen: no home folder to keep the state in")?;
    Ok((events, state))
}

fn run_command() -> i32 {
    let (events, state) = match dirs() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let listener = match Listener::new(Journal::new(events.clone(), state)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cac-mcp listen: could not open {}: {e}", events.display());
            return 1;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cac-mcp listen: {e}");
            return 1;
        }
    };
    eprintln!("cac-mcp listen: writing events to {}", events.display());
    runtime.block_on(listener.run(&credentials));
    0
}

/// `--consumer <nombre>`, obligatorio y con forma de nombre de fichero.
fn consumer_of(args: &[&str]) -> Result<String, String> {
    let at = args.iter().position(|a| *a == "--consumer").ok_or("--consumer <name> is required")?;
    let name = args.get(at + 1).ok_or("--consumer needs a name")?;
    if !valid_consumer(name) {
        return Err(format!("`{name}` is not a consumer name: letters, digits, dot, dash and underscore, up to 64"));
    }
    Ok(name.to_string())
}

fn follow_command(args: &[&str]) -> i32 {
    let mut filter = Filter::default();
    let mut once = false;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--consumer" => i += 1,
            "--project" | "--type" => {
                let Some(value) = args.get(i + 1) else {
                    eprintln!("cac-mcp listen: {} needs a value. {USAGE}", args[i]);
                    return 2;
                };
                let list = if args[i] == "--project" { &mut filter.projects } else { &mut filter.types };
                list.extend(value.split(',').map(str::trim).filter(|v| !v.is_empty()).map(String::from));
                i += 1;
            }
            "--mine" => filter.mine = true,
            "--once" => once = true,
            other => {
                eprintln!("cac-mcp listen: unknown argument `{other}`. {USAGE}");
                return 2;
            }
        }
        i += 1;
    }
    let consumer = match consumer_of(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cac-mcp listen: {e}. {USAGE}");
            return 2;
        }
    };
    let (events, state) = match dirs() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match follow(&events, &state, &consumer, &filter, once, &mut out, Duration::from_millis(500), &|| true) {
        Ok(()) => 0,
        // Quien leía se fue (el Monitor venció): no es un fallo.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(e) => {
            eprintln!("cac-mcp listen: could not read {}: {e}", events.display());
            1
        }
    }
}

fn ack_command(args: &[&str]) -> i32 {
    let consumer = match consumer_of(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cac-mcp listen: {e}. {USAGE}");
            return 2;
        }
    };
    let rest: Vec<&str> = args.iter().copied().filter(|a| *a != "--consumer" && *a != consumer).collect();
    let [seq] = rest.as_slice() else {
        eprintln!("cac-mcp listen: ack needs one seq. {USAGE}");
        return 2;
    };
    let Ok(seq) = seq.parse::<u64>() else {
        eprintln!("cac-mcp listen: `{seq}` is not a seq. {USAGE}");
        return 2;
    };
    let (events, state) = match dirs() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    // Nada por encima de lo que hay en el fichero: tras un reinicio, el `ack`
    // tardío de un número del diario viejo dejaría al consumidor sordo hasta
    // alcanzarlo. Se deja lo que había.
    if let Ok(Some(last)) = Journal::new(events, state.clone()).last_seq() {
        if seq > last {
            println!("{consumer}: {seq} is not in the events file (it ends at {last}); acknowledged stays at {}", progress(&state, &consumer).acked.map_or("nothing".into(), |a| a.to_string()));
            return 0;
        }
    }
    match ack(&state, &consumer, seq) {
        Ok(kept) => {
            println!("{consumer}: acknowledged up to {kept}");
            0
        }
        Err(e) => {
            eprintln!("cac-mcp listen: could not save the acknowledgement: {e}");
            1
        }
    }
}

// ── Un cac de mentira ───────────────────────────────────────────────────────

/// Un backend de mentira con el contrato de command-and-control-558 (el
/// stream y el diario de una persona), para las pruebas: sirve por HTTP en
/// esta máquina y apunta lo que le piden. Ninguna prueba sale a internet.
#[doc(hidden)]
pub mod fake {
    //! Contesta como el backend de producción: el backlog como
    //! `EventJournal.Read` (`service/event_journal.go`) y el stream como
    //! `streamToken` (`handler/events_backlog.go`). Si cambian allí, cambia
    //! aquí: lo que el fake no dice, las pruebas no lo ven.
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    use serde_json::{json, Value};

    /// Una petición que llegó: la ruta, su `Last-Event-ID` y si traía el token
    /// bueno.
    #[derive(Clone, Debug)]
    pub struct Asked {
        pub path: String,
        pub last_event_id: Option<String>,
        pub authorized: bool,
    }

    #[derive(Default)]
    struct State {
        /// El diario de la persona: lo que el backlog sabe.
        journal: BTreeMap<u64, Value>,
        /// El último número dado (`user_event_heads`): la retención no lo
        /// baja; sólo un diario que vuelve a empezar.
        head: u64,
        /// Los números de una organización que la persona dejó: se entregan
        /// como `cac:skipped`.
        hidden: BTreeSet<u64>,
        /// Lo que el hub reparte en vivo, en el orden en que se reparte.
        live: Vec<Value>,
        /// Lo que se le ha pedido, en orden.
        asked: Vec<Asked>,
        /// Si está puesto, todo contesta ese estado.
        refuse: Option<u16>,
        /// Cada vez que sube, se cierran los streams abiertos.
        generation: u64,
    }

    #[derive(Clone)]
    pub struct FakeCac {
        pub base: String,
        pub token: String,
        state: Arc<(Mutex<State>, Condvar)>,
    }

    /// El sobre de un evento como lo manda el backend.
    pub fn envelope(seq: u64, kind: &str, at: &str, data: Value) -> Value {
        json!({
            "seq": seq,
            "id": format!("00000000-0000-4000-8000-{seq:012}"),
            "type": kind,
            "at": at,
            "orgId": "org-1",
            "key": format!("k:{seq}"),
            "mine": false,
            "external": false,
            "data": data,
        })
    }

    /// `domain.GapEnvelope`: se perdió de `from` a `to`, y ocupa `to`.
    pub fn gap_envelope(from: u64, to: u64) -> Value {
        json!({
            "seq": to, "id": "gap", "type": super::GAP, "at": "2026-11-27T00:00:00Z",
            "orgId": "", "key": "", "mine": false, "external": false,
            "data": { "from": from, "to": to },
        })
    }

    /// `EventEnvelope.Skipped`: el número y nada más.
    fn skipped(event: &Value) -> Value {
        json!({
            "seq": event["seq"], "id": event["id"], "type": "cac:skipped", "at": event["at"],
            "orgId": "", "key": "", "mine": false, "external": false, "data": null,
        })
    }

    impl State {
        fn visible(&self, event: &Value) -> Value {
            if self.hidden.contains(&event["seq"].as_u64().unwrap_or(0)) {
                skipped(event)
            } else {
                event.clone()
            }
        }

        /// `EventJournal.Read`: `(events, head, gap)`.
        fn read(&self, after: u64, limit: usize) -> (Vec<Value>, u64, bool) {
            let limit = if limit == 0 || limit > 500 { 500 } else { limit };
            let head = self.head;
            if after > head {
                return (vec![gap_envelope(head, head)], head, true);
            }
            if after == head {
                return (Vec::new(), head, false);
            }
            let mut events = Vec::new();
            let mut gap = false;
            let first_kept = self.journal.keys().next().copied().unwrap_or(head + 1);
            if after + 1 < first_kept {
                gap = true;
                events.push(gap_envelope(after + 1, first_kept - 1));
            }
            events.extend(self.journal.range(after + 1..).take(limit).map(|(_, v)| self.visible(v)));
            (events, head, gap)
        }
    }

    impl FakeCac {
        pub fn start(token: &str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("un puerto libre en esta máquina");
            let base = format!("http://{}", listener.local_addr().unwrap());
            let fake = FakeCac { base, token: token.to_string(), state: Arc::default() };
            let server = fake.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let server = server.clone();
                    std::thread::spawn(move || server.answer(stream));
                }
            });
            fake
        }

        /// Un evento que se apunta en el diario y se publica en el hub, en
        /// ese orden (`EventJournal.Append`).
        pub fn push(&self, event: Value) {
            let (lock, wake) = &*self.state;
            let mut state = lock.lock().unwrap();
            let seq = event["seq"].as_u64().unwrap();
            state.head = state.head.max(seq);
            state.journal.insert(seq, event.clone());
            state.live.push(event);
            wake.notify_all();
        }

        /// Un evento que se apunta en el diario pero que el hub tiró (un
        /// consumidor lento, el bus caído): sólo el backlog lo sabe.
        pub fn record_only(&self, event: Value) {
            let (lock, _) = &*self.state;
            let mut state = lock.lock().unwrap();
            let seq = event["seq"].as_u64().unwrap();
            state.head = state.head.max(seq);
            state.journal.insert(seq, event);
        }

        /// La persona dejó la organización de este evento: desde ahora se
        /// entrega como `cac:skipped`.
        pub fn hide(&self, seq: u64) {
            self.state.0.lock().unwrap().hidden.insert(seq);
        }

        /// El diario olvida todo lo anterior a `seq` (la retención). La
        /// cabecera no baja.
        pub fn forget_before(&self, seq: u64) {
            let (lock, _) = &*self.state;
            lock.lock().unwrap().journal.retain(|s, _| *s >= seq);
        }

        /// El diario vuelve a empezar vacío, como tras un `DROP TABLE
        /// user_events, user_event_heads` o un Postgres restaurado: la
        /// cabecera vuelve a 0. Corta los streams (un despliegue).
        pub fn restart_journal(&self) {
            let (lock, wake) = &*self.state;
            let mut state = lock.lock().unwrap();
            state.journal.clear();
            state.hidden.clear();
            state.head = 0;
            state.generation += 1;
            wake.notify_all();
        }

        /// Todo contesta este estado (401, 403…) hasta `accept`. Un stream
        /// abierto lo ve en su siguiente latido: manda `token:revoked` y
        /// cuelga.
        pub fn refuse(&self, status: u16) {
            let (lock, wake) = &*self.state;
            let mut state = lock.lock().unwrap();
            state.refuse = Some(status);
            state.generation += 1;
            wake.notify_all();
        }

        pub fn accept(&self) {
            let (lock, _) = &*self.state;
            lock.lock().unwrap().refuse = None;
        }

        /// Corta los streams abiertos, como un despliegue del backend.
        pub fn drop_streams(&self) {
            let (lock, wake) = &*self.state;
            lock.lock().unwrap().generation += 1;
            wake.notify_all();
        }

        pub fn asked(&self) -> Vec<Asked> {
            self.state.0.lock().unwrap().asked.clone()
        }

        fn answer(&self, stream: TcpStream) {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).is_err() {
                return;
            }
            let mut last_event_id = None;
            let mut authorization = None;
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                if let Some((name, value)) = line.trim_end().split_once(':') {
                    match name.to_ascii_lowercase().as_str() {
                        "last-event-id" => last_event_id = Some(value.trim().to_string()),
                        "authorization" => authorization = Some(value.trim().to_string()),
                        _ => {}
                    }
                }
                line.clear();
            }
            let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
            let authorized = authorization.as_deref() == Some(&format!("Bearer {}", self.token));
            let (lock, wake) = &*self.state;
            let mut state = lock.lock().unwrap();
            state.asked.push(Asked { path: path.clone(), last_event_id: last_event_id.clone(), authorized });
            wake.notify_all();
            let mut out = stream;
            let status = state.refuse.or((!authorized).then_some(401));
            if let Some(status) = status {
                drop(state);
                let body = json!({ "error": if status == 403 { "missing-scope:events:read" } else { "invalid-token" } }).to_string();
                let _ = write!(out, "HTTP/1.1 {status} No\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                return;
            }
            if let Some(query) = path.strip_prefix("/api/v1/events/backlog?") {
                let param = |name: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{name}="))).and_then(|v| v.parse::<u64>().ok());
                let (events, head, gap) = state.read(param("after").unwrap_or(0), param("limit").unwrap_or(200) as usize);
                let mut page = json!({ "events": events, "head": head });
                if gap {
                    page["gap"] = json!(true);
                }
                let body = json!({ "success": true, "data": page }).to_string();
                drop(state);
                let _ = write!(out, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                return;
            }
            if path != "/api/v1/events" {
                drop(state);
                let _ = write!(out, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                return;
            }
            // `streamToken`: sin `Last-Event-ID` se empieza en la cabecera; uno
            // por delante de ella es un `cac:gap` con `seq = head` y se sigue
            // desde la cabecera; después, ponerse al día con el diario y en
            // vivo, sólo lo que pasa del cursor.
            let generation = state.generation;
            let head = state.head;
            let resume: Option<u64> = last_event_id.as_deref().and_then(|id| id.parse().ok());
            let mut cursor = resume.unwrap_or(head);
            let mut next_live = state.live.len();
            drop(state);
            let send = |out: &mut TcpStream, event: &Value| -> bool {
                let text = format!("id: {}\nevent: {}\ndata: {}\n\n", event["seq"], event["type"].as_str().unwrap_or(""), event);
                out.write_all(text.as_bytes()).and_then(|_| out.flush()).is_ok()
            };
            if write!(out, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n").is_err() {
                return;
            }
            if resume.is_some() && cursor > head {
                cursor = head;
                if !send(&mut out, &gap_envelope(head, head)) {
                    return;
                }
            }
            let catch_up = |out: &mut TcpStream, cursor: &mut u64| -> bool {
                loop {
                    let (events, head, _) = lock.lock().unwrap().read(*cursor, 500);
                    if events.is_empty() {
                        return true;
                    }
                    for event in &events {
                        let seq = event["seq"].as_u64().unwrap_or(0);
                        if seq <= *cursor {
                            continue;
                        }
                        if !send(out, event) {
                            return false;
                        }
                        *cursor = seq;
                    }
                    if *cursor >= head {
                        return true;
                    }
                }
            };
            if out.write_all(b": connected\n\n").is_err() || !catch_up(&mut out, &mut cursor) {
                return;
            }
            loop {
                let pending: Vec<Value> = {
                    let state = lock.lock().unwrap();
                    let (state, _) = wake.wait_timeout_while(state, Duration::from_millis(200), |s| s.live.len() == next_live && s.generation == generation).unwrap();
                    if state.generation != generation {
                        if state.refuse.is_some() {
                            let _ = out.write_all(b"event: token:revoked\ndata: {}\n\n");
                        }
                        return;
                    }
                    let pending: Vec<Value> = state.live[next_live..].iter().map(|e| state.visible(e)).collect();
                    next_live = state.live.len();
                    pending
                };
                if pending.is_empty() {
                    // El latido: además vuelve a mirar el diario, por si el
                    // hub tiró algo.
                    if out.write_all(b"event: ping\ndata: {}\n\n").is_err() || !catch_up(&mut out, &mut cursor) {
                        return;
                    }
                    continue;
                }
                for event in &pending {
                    let seq = event["seq"].as_u64().unwrap_or(0);
                    if seq <= cursor {
                        continue;
                    }
                    if seq > cursor + 1 {
                        if !catch_up(&mut out, &mut cursor) {
                            return;
                        }
                        continue;
                    }
                    if !send(&mut out, event) {
                        return;
                    }
                    cursor = seq;
                }
            }
        }

        /// Espera a que se cumpla algo de lo pedido, hasta `deadline`.
        pub fn wait_until(&self, deadline: Duration, done: impl Fn(&[Asked]) -> bool) -> bool {
            let (lock, wake) = &*self.state;
            let state = lock.lock().unwrap();
            let (state, _) = wake.wait_timeout_while(state, deadline, |s| !done(&s.asked)).unwrap();
            done(&state.asked)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{envelope, FakeCac};
    use super::*;
    use std::collections::HashMap;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-mcp-listen-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn journal(dir: &Path) -> Journal {
        Journal::new(dir.join("cache").join("cac").join("events.jsonl"), dir.join("state"))
    }

    fn ev(seq: u64) -> Value {
        envelope(seq, "notification:new", &iso_of_unix(now()), json!({ "n": seq }))
    }

    fn seqs(path: &Path) -> Vec<u64> {
        fs::read_to_string(path).unwrap_or_default().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["seq"].as_u64().unwrap()).collect()
    }

    /// Roturas que atrapa: no respetar `XDG_CACHE_HOME`, o no caer en
    /// `~/.cache` sin ella.
    #[test]
    fn the_events_file_lives_in_the_cache_folder() {
        let env = |vars: &'static [(&'static str, &'static str)]| move |name: &str| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| PathBuf::from(v));
        assert_eq!(events_file_from("linux", &env(&[("HOME", "/home/ana")])), Some(PathBuf::from("/home/ana/.cache/cac/events.jsonl")));
        assert_eq!(events_file_from("linux", &env(&[("HOME", "/home/ana"), ("XDG_CACHE_HOME", "/c")])), Some(PathBuf::from("/c/cac/events.jsonl")));
        assert_eq!(events_file_from("windows", &env(&[("LOCALAPPDATA", "C:/L")])), Some(PathBuf::from("C:/L/cac/events.jsonl")));
    }

    /// Roturas que atrapa: no leer el `.claude.json` (el servicio de systemd
    /// no tiene `CAC_TOKEN`), no preferir el entorno, o enseñar el token con
    /// `{:?}`.
    #[test]
    fn the_token_comes_from_the_environment_or_the_claude_config() {
        let files: HashMap<PathBuf, String> = HashMap::from([(
            PathBuf::from("/cfg/.claude.json"),
            json!({ "mcpServers": { "cac": { "env": { "CAC_URL": "https://cac.example/", "CAC_TOKEN": "cac_pat_FROMFILE" } } } }).to_string(),
        )]);
        let read = |p: &Path| files.get(p).cloned();
        let from_file = credentials_from(&|n| (n == "CLAUDE_CONFIG_DIR").then(|| "/cfg".to_string()), &read).unwrap();
        assert_eq!(from_file, Credentials { base: "https://cac.example".into(), token: "cac_pat_FROMFILE".into() });
        let env = |n: &str| match n {
            "CAC_TOKEN" => Some("cac_pat_ENV".to_string()),
            "CLAUDE_CONFIG_DIR" => Some("/cfg".to_string()),
            _ => None,
        };
        let from_env = credentials_from(&env, &read).unwrap();
        assert_eq!(from_env.token, "cac_pat_ENV");
        assert_eq!(from_env.base, DEFAULT_URL);
        assert!(!format!("{from_env:?}").contains("cac_pat_ENV"));
        assert!(credentials_from(&|_| None, &read).is_err());
    }

    /// Rotura que atrapa: tomar sólo el cursor guardado al arrancar. Si el
    /// proceso murió después de escribir la línea y antes de guardar el
    /// cursor, el stream la volvería a mandar y quedaría dos veces.
    #[test]
    fn dying_between_the_line_and_the_cursor_repeats_nothing() {
        let dir = scratch("crash");
        let j = journal(&dir);
        j.record(&ev(1)).unwrap();
        j.record(&ev(2)).unwrap();
        j.append(&ev(3)).unwrap(); // y aquí se muere
        assert_eq!(j.saved_cursor(), Some(2));
        assert_eq!(j.recover().unwrap(), Some(3));
    }

    /// Roturas que atrapa: dejar una línea a medias (quien sigue el fichero
    /// leería basura pegada a la siguiente), o no saber que no hay nada.
    #[test]
    fn a_half_written_line_is_cut_off() {
        let dir = scratch("half");
        let j = journal(&dir);
        assert_eq!(j.recover().unwrap(), None);
        j.record(&ev(1)).unwrap();
        let mut file = OpenOptions::new().append(true).open(j.events_path()).unwrap();
        file.write_all(br#"{"seq":2,"type":"notif"#).unwrap();
        drop(file);
        assert_eq!(j.recover().unwrap(), Some(1));
        let text = fs::read_to_string(j.events_path()).unwrap();
        assert!(text.ends_with("}\n"), "{text}");
        assert_eq!(seqs(j.events_path()), vec![1]);
    }

    /// Roturas que atrapa: perder el cursor cuando la compactación vacía el
    /// fichero, o no compactar por fecha ni por tamaño.
    #[test]
    fn compacting_keeps_the_recent_and_the_cursor() {
        let dir = scratch("compact");
        let j = journal(&dir);
        let day = 86_400;
        let t = 1_800_000_000;
        for (seq, at) in [(1, t - 20 * day), (2, t - 15 * day), (3, t - day), (4, t)] {
            j.record(&envelope(seq, "notification:new", &iso_of_unix(at), json!({}))).unwrap();
        }
        j.compact(t).unwrap();
        assert_eq!(seqs(j.events_path()), vec![3, 4]);
        compact_file(j.events_path(), t, 30 * day, 1).unwrap();
        assert_eq!(seqs(j.events_path()), Vec::<u64>::new());
        assert_eq!(j.recover().unwrap(), Some(4));

        // Murió con el cursor un evento por detrás y todo es viejo: arrancar
        // compacta hasta vaciar, y el cursor que sólo sabía la línea sigue.
        let dir = scratch("compact-crash");
        let j = journal(&dir);
        let old = iso_of_unix(now() - 20 * day);
        j.record(&envelope(1, "notification:new", &old, json!({}))).unwrap();
        j.append(&envelope(2, "notification:new", &old, json!({}))).unwrap();
        let listener = Listener::new(j).unwrap();
        assert_eq!(seqs(listener.journal.events_path()), Vec::<u64>::new());
        assert_eq!(listener.journal.recover().unwrap(), Some(2));
    }

    #[test]
    fn dates_go_both_ways() {
        assert_eq!(iso_of_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_of_unix(1_792_627_200), "2026-10-22T00:00:00Z");
        assert_eq!(unix_of_iso("2026-11-27T09:13:17.783602Z"), Some(1_795_770_797));
        assert_eq!(unix_of_iso(&iso_of_unix(1_795_770_797)), Some(1_795_770_797));
        assert_eq!(unix_of_iso("ayer"), None);
    }

    /// Roturas que atrapa: perder un mensaje cortado entre dos trozos, tomar
    /// un comentario (`: ping`) por evento, o no juntar varias líneas `data`.
    #[test]
    fn the_sse_parser_survives_any_split() {
        let text = ": connected\n\nid: 7\r\nevent: notification:new\r\ndata: {\"seq\":7}\r\n\r\n: ping\n\ndata: a\ndata: b\n\n";
        for cut in 0..text.len() {
            let mut parser = SseParser::default();
            let mut got = parser.feed(&text.as_bytes()[..cut]);
            got.extend(parser.feed(&text.as_bytes()[cut..]));
            assert_eq!(
                got,
                vec![
                    SseMessage { id: Some("7".into()), event: Some("notification:new".into()), data: "{\"seq\":7}".into() },
                    SseMessage { id: None, event: None, data: "a\nb".into() },
                ],
                "cortado en {cut}"
            );
        }
    }

    /// Rotura que atrapa: reintentar un 401 o un 403 con el retroceso corto
    /// de un fallo de red (un token revocado martillearía a cac en bucle).
    #[test]
    fn a_refused_token_waits_minutes_not_seconds() {
        let auth = Failure::Auth { status: 401, code: "invalid-token".into() };
        assert!(retry_delay(&auth, 0) >= Duration::from_secs(300));
        assert!(retry_delay(&Failure::NoCredentials("x".into()), 0) >= Duration::from_secs(300));
        assert!(retry_delay(&auth, 20) <= Duration::from_secs(1800));
        assert_eq!(retry_delay(&Failure::Transient("x".into()), 0), Duration::from_secs(1));
        assert_eq!(retry_delay(&Failure::Transient("x".into()), 30), Duration::from_secs(60));
    }

    /// Roturas que atrapa: retroceder el confirmado con un `ack` viejo, o
    /// aceptar un consumidor que es una ruta.
    #[test]
    fn an_ack_never_goes_back() {
        let dir = scratch("ack");
        assert_eq!(ack(&dir, "tcg", 5).unwrap(), 5);
        assert_eq!(ack(&dir, "tcg", 3).unwrap(), 5);
        assert_eq!(acked(&dir, "tcg"), 5);
        assert_eq!(acked(&dir, "otro"), 0);
        for bad in ["", "../x", "a/b", ".hidden", &"x".repeat(65)] {
            assert!(!valid_consumer(bad), "{bad}");
        }
        assert!(valid_consumer("command-and-control_2.x"));
    }

    fn follow_once(j: &Journal, state: &Path, consumer: &str, filter: &Filter) -> Vec<u64> {
        let mut out = Vec::new();
        follow(j.events_path(), state, consumer, filter, true, &mut out, Duration::ZERO, &|| false).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["seq"].as_u64().unwrap()).collect()
    }

    /// Roturas que atrapa: volver a imprimir lo confirmado, o seguir por byte
    /// y repetir (o saltarse) lo que la compactación movió de sitio.
    #[test]
    fn follow_skips_what_was_acked_and_survives_compaction() {
        let dir = scratch("follow");
        let j = journal(&dir);
        let state = dir.join("state");
        for seq in 1..=4 {
            j.record(&ev(seq)).unwrap();
        }
        ack(&state, "s", 2).unwrap();
        assert_eq!(follow_once(&j, &state, "s", &Filter::default()), vec![3, 4]);

        // Siguiendo de verdad, por vueltas y sin hilos: en la primera lee 3 y
        // 4; tras ella la compactación reescribe el fichero (otro inodo, más
        // corto); tras la segunda llega el 5; la tercera lo lee y para.
        let round = std::cell::Cell::new(0);
        let mut out = Vec::new();
        follow(j.events_path(), &state, "s", &Filter::default(), false, &mut out, Duration::ZERO, &|| {
            round.set(round.get() + 1);
            match round.get() {
                1 => {
                    let last = fs::read_to_string(j.events_path()).unwrap().lines().last().unwrap().len() + 1;
                    compact_file(j.events_path(), now(), 365 * 86_400, last as u64).unwrap();
                    assert_eq!(seqs(j.events_path()), vec![4]);
                    true
                }
                2 => {
                    j.record(&ev(5)).unwrap();
                    true
                }
                _ => false,
            }
        })
        .unwrap();
        let got: Vec<u64> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["seq"].as_u64().unwrap()).collect();
        assert_eq!(got, vec![3, 4, 5]);
    }

    /// Roturas que atrapa: que `--mine`, `--type` o `--project` dejen pasar
    /// lo que no es, o que escondan una línea `cac:gap`.
    #[test]
    fn the_filters_are_the_readers() {
        let dir = scratch("filters");
        let j = journal(&dir);
        let state = dir.join("state");
        let mut mine = envelope(1, "basecamp:event", "2026-11-27T00:00:00Z", json!({ "project": { "id": 42, "name": "TCG", "spaceId": "sp-1" } }));
        mine["mine"] = json!(true);
        j.record(&mine).unwrap();
        j.record(&envelope(2, "basecamp:event", "2026-11-27T00:00:00Z", json!({ "project": { "id": 7, "name": "Otro" } }))).unwrap();
        j.record(&envelope(3, "notification:new", "2026-11-27T00:00:00Z", json!({}))).unwrap();
        j.record(&envelope(4, GAP, "2026-11-27T00:00:00Z", json!({}))).unwrap();
        let f = |projects: &[&str], types: &[&str], mine: bool| Filter { projects: projects.iter().map(|s| s.to_string()).collect(), types: types.iter().map(|s| s.to_string()).collect(), mine };
        assert_eq!(follow_once(&j, &state, "a", &f(&[], &[], false)), vec![1, 2, 3, 4]);
        assert_eq!(follow_once(&j, &state, "a", &f(&[], &[], true)), vec![1, 4]);
        assert_eq!(follow_once(&j, &state, "a", &f(&[], &["notification:new"], false)), vec![3, 4]);
        assert_eq!(follow_once(&j, &state, "a", &f(&["tcg"], &[], false)), vec![1, 4]);
        assert_eq!(follow_once(&j, &state, "a", &f(&["42"], &[], false)), vec![1, 4]);
        assert_eq!(follow_once(&j, &state, "a", &f(&["sp-1"], &["basecamp:event"], false)), vec![1, 4]);
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    fn creds(fake: &FakeCac) -> Credentials {
        Credentials { base: fake.base.clone(), token: fake.token.clone() }
    }

    /// Roturas que atrapa: escribir el evento que se adelanta sin rellenar el
    /// hueco (se perdería el 2), o escribirlo dos veces.
    #[test]
    fn a_skipped_seq_is_filled_from_the_backlog() {
        let dir = scratch("skip");
        let fake = FakeCac::start("cac_pat_T");
        let mut listener = Listener::new(journal(&dir)).unwrap();
        listener.cursor = Some(1);
        listener.journal.record(&ev(1)).unwrap();
        fake.record_only(ev(1));
        fake.record_only(ev(2));
        fake.record_only(ev(3));
        let c = creds(&fake);
        rt().block_on(async {
            let msg = |seq: u64| SseMessage { id: Some(seq.to_string()), event: None, data: ev(seq).to_string() };
            listener.deliver(&c, &msg(3)).await.unwrap();
            listener.deliver(&c, &msg(2)).await.unwrap();
            listener.deliver(&c, &msg(3)).await.unwrap();
        });
        assert_eq!(seqs(listener.journal.events_path()), vec![1, 2, 3]);
        assert_eq!(listener.journal.saved_cursor(), Some(3));
    }

    /// Rotura que atrapa: escribir un evento con un hueco que el diario
    /// tampoco tiene. Se reconecta desde el cursor, sin escribirlo.
    #[test]
    fn a_hole_the_backlog_cannot_fill_reconnects() {
        let dir = scratch("hole");
        let fake = FakeCac::start("cac_pat_T");
        let mut listener = Listener::new(journal(&dir)).unwrap();
        listener.cursor = Some(1);
        fake.record_only(ev(1));
        let c = creds(&fake);
        let got = rt().block_on(listener.deliver(&c, &SseMessage { id: None, event: None, data: ev(3).to_string() }));
        assert!(matches!(got, Err(Failure::Transient(_))), "{got:?}");
        assert_eq!(seqs(listener.journal.events_path()), Vec::<u64>::new());
        assert_eq!(listener.cursor, Some(1));
    }

    /// Roturas que atrapa: traer todo el pasado la primera vez, empezar sin
    /// decirlo, o no saltar con una línea `cac:gap` lo que el diario ya
    /// olvidó (se quedaría pidiendo el mismo hueco para siempre).
    #[test]
    fn the_first_run_and_an_expired_cursor_say_there_is_a_gap() {
        let fake = FakeCac::start("cac_pat_T");
        for seq in 1..=5 {
            fake.record_only(ev(seq));
        }
        let c = creds(&fake);

        let dir = scratch("first");
        let mut listener = Listener::new(journal(&dir)).unwrap();
        rt().block_on(listener.catch_up(&c)).unwrap();
        let lines: Vec<Value> = fs::read_to_string(listener.journal.events_path()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["type"], GAP);
        assert_eq!(lines[0]["seq"], 5);
        assert_eq!(lines[0]["data"]["reason"], "first-run");
        assert_eq!((&lines[0]["key"], &lines[0]["orgId"]), (&json!(""), &json!("")));

        let dir = scratch("expired");
        let mut listener = Listener::new(journal(&dir)).unwrap();
        listener.cursor = Some(1);
        fake.forget_before(4);
        rt().block_on(listener.catch_up(&c)).unwrap();
        let lines: Vec<Value> = fs::read_to_string(listener.journal.events_path()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let shape: Vec<(u64, &str)> = lines.iter().map(|l| (l["seq"].as_u64().unwrap(), l["type"].as_str().unwrap())).collect();
        assert_eq!(shape, vec![(3, GAP), (4, "notification:new"), (5, "notification:new")]);
        assert_eq!(lines[0]["data"]["from"], 2);
        assert_eq!(lines[0]["id"], "gap", "el hueco es el del backend, sin otro local delante");
    }

    fn lines_of(path: &Path) -> Vec<Value> {
        fs::read_to_string(path).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    /// Roturas que atrapa: descartar un `cac:gap` del backend por estar por
    /// detrás del cursor (todo lo que llegue hasta el número viejo se
    /// saltaría como ya escrito), no bajar el cursor guardado, o que
    /// `recover` lo vuelva a subir con el mayor de la línea y el estado.
    #[test]
    fn a_journal_that_starts_again_lowers_the_cursor() {
        let dir = scratch("reset");
        let fake = FakeCac::start("cac_pat_T");
        for seq in 1..=3 {
            fake.record_only(ev(seq));
        }
        let mut listener = Listener::new(journal(&dir)).unwrap();
        listener.journal.record(&ev(799)).unwrap();
        listener.journal.record(&ev(800)).unwrap();
        listener.cursor = Some(800);
        let c = creds(&fake);
        rt().block_on(listener.catch_up(&c)).unwrap();
        let lines = lines_of(listener.journal.events_path());
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!((lines[0]["seq"].as_u64(), lines[0]["type"].as_str(), lines[0]["data"]["reason"].as_str()), (Some(3), Some(GAP), Some(RESET)));
        assert_eq!(lines[0]["data"]["previousCursor"], 800);
        assert_eq!(listener.cursor, Some(3));
        assert_eq!(listener.journal.saved_cursor(), Some(3));

        // Lo siguiente del diario nuevo se escribe.
        rt().block_on(listener.deliver(&c, &SseMessage { id: None, event: None, data: ev(4).to_string() })).unwrap();
        assert_eq!(seqs(listener.journal.events_path()), vec![3, 4]);

        // Murió entre reescribir el fichero y guardar el cursor: manda la
        // línea, no el mayor.
        listener.journal.save_cursor(800).unwrap();
        assert_eq!(listener.journal.recover().unwrap(), Some(4));
    }

    /// Rotura que atrapa: descartar el `cac:gap` que manda el stream cuando
    /// `Last-Event-ID` está por delante de la cabecera.
    #[test]
    fn a_gap_behind_the_cursor_on_the_stream_starts_again() {
        let dir = scratch("reset-sse");
        let fake = FakeCac::start("cac_pat_T");
        let mut listener = Listener::new(journal(&dir)).unwrap();
        listener.cursor = Some(800);
        let c = creds(&fake);
        let gap = fake::gap_envelope(0, 0);
        rt().block_on(listener.deliver(&c, &SseMessage { id: Some("0".into()), event: Some(GAP.into()), data: gap.to_string() })).unwrap();
        assert_eq!(listener.cursor, Some(0));
        assert_eq!(seqs(listener.journal.events_path()), vec![0]);
        // Un hueco que ya se escribió (su número es el cursor) y que llega
        // otra vez no es otro reinicio: no se lleva lo que hay.
        let expired = fake::gap_envelope(1, 1);
        rt().block_on(listener.deliver(&c, &SseMessage { id: Some("1".into()), event: Some(GAP.into()), data: expired.to_string() })).unwrap();
        rt().block_on(listener.deliver(&c, &SseMessage { id: Some("1".into()), event: Some(GAP.into()), data: expired.to_string() })).unwrap();
        assert_eq!(seqs(listener.journal.events_path()), vec![0, 1]);
        assert_eq!(listener.cursor, Some(1));
    }

    /// Rotura que atrapa: que `cac:skipped` no se escriba (el cursor se
    /// quedaría atrás y el hueco se pediría para siempre).
    #[test]
    fn a_skipped_event_keeps_the_numbers_together() {
        let dir = scratch("skipped");
        let fake = FakeCac::start("cac_pat_T");
        for seq in 1..=3 {
            fake.record_only(ev(seq));
        }
        fake.hide(2);
        let mut listener = Listener::new(journal(&dir)).unwrap();
        listener.cursor = Some(0);
        rt().block_on(listener.catch_up(&creds(&fake))).unwrap();
        let kinds: Vec<String> = lines_of(listener.journal.events_path()).iter().map(|l| l["type"].as_str().unwrap().to_string()).collect();
        assert_eq!(kinds, vec!["notification:new", "cac:skipped", "notification:new"]);
        assert_eq!(listener.cursor, Some(3));
    }

    /// Rotura que atrapa: tomar `token:revoked` por un corte cualquiera
    /// (reintentaría en un segundo y diría «retrying» en vez de por qué).
    #[test]
    fn a_revoked_token_is_a_no_not_a_cut() {
        let dir = scratch("revoked");
        let fake = FakeCac::start("cac_pat_T");
        let mut listener = Listener::new(journal(&dir)).unwrap();
        listener.cursor = Some(0);
        let c = creds(&fake);
        let revoker = fake.clone();
        let thread = std::thread::spawn(move || {
            assert!(revoker.wait_until(Duration::from_secs(30), |asked| asked.iter().any(|a| a.path == "/api/v1/events")));
            revoker.refuse(401);
        });
        let (connected, failure) = rt().block_on(listener.session(&c));
        thread.join().unwrap();
        assert!(connected);
        assert_eq!(failure, Failure::Auth { status: 401, code: TOKEN_REVOKED.into() });
    }

    /// Rotura que atrapa: crear cualquiera de los cuatro ficheros legible por
    /// otros. El de eventos lleva el texto de mensajes directos.
    #[cfg(unix)]
    #[test]
    fn everything_it_writes_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("modes");
        let listener = Listener::new(journal(&dir)).unwrap();
        listener.journal.record(&ev(1)).unwrap();
        listener.status.write("connected", None, Some(1), None);
        let state = dir.join("state");
        ack(&state, "tcg", 1).unwrap();
        let mut listener = listener;
        listener.cursor = Some(5);
        listener.reset(&fake::gap_envelope(2, 2)).unwrap();
        for path in [listener.journal.events_path().to_path_buf(), state.join("listen.json"), state.join("listen.status.json"), state.join("consumers").join("tcg.json")] {
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", path.display());
        }
    }

    /// Rotura que atrapa: no devolver el fichero a su largo cuando una
    /// escritura falla a mitad (el disco lleno): la línea rota se pegaría a
    /// la siguiente y ese evento quedaría ilegible.
    #[test]
    fn a_write_that_fails_halfway_leaves_nothing() {
        let dir = scratch("halfway");
        let j = journal(&dir);
        j.record(&ev(1)).unwrap();
        let failed = j.append_with(&ev(2), &mut |file, line| {
            file.write_all(&line[..line.len() / 2])?;
            Err(io::Error::other("no space left on device"))
        });
        assert!(failed.is_err());
        j.record(&ev(2)).unwrap();
        assert_eq!(seqs(j.events_path()), vec![1, 2]);
    }

    fn follow_rounds(j: &Journal, state: &Path, consumer: &str, between: &[&dyn Fn()]) -> Vec<u64> {
        let round = std::cell::Cell::new(0);
        let mut out = Vec::new();
        follow(j.events_path(), state, consumer, &Filter::default(), false, &mut out, Duration::ZERO, &|| {
            let at = round.get();
            round.set(at + 1);
            between.get(at).map(|f| f()).is_some()
        })
        .unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["seq"].as_u64().unwrap()).collect()
    }

    /// Rotura que atrapa: reabrir sólo si el fichero encoge. Al arrancar, el
    /// servicio compacta y escribe enseguida lo pendiente; si antes del
    /// siguiente sondeo el fichero nuevo ya pasa de donde iba la lectura,
    /// seguir desde ese byte cae a mitad de una línea y la pierde.
    #[test]
    fn follow_notices_a_new_file_that_is_already_longer() {
        let dir = scratch("inode");
        let j = journal(&dir);
        let state = dir.join("state");
        for seq in 1..=4 {
            j.record(&ev(seq)).unwrap();
        }
        let got = follow_rounds(&j, &state, "s", &[&|| {
            let before = fs::metadata(j.events_path()).unwrap().len();
            let last = fs::read_to_string(j.events_path()).unwrap().lines().last().unwrap().len() + 1;
            compact_file(j.events_path(), now(), 365 * 86_400, last as u64).unwrap();
            let mut seq = 5;
            while fs::metadata(j.events_path()).unwrap().len() <= before {
                j.record(&ev(seq)).unwrap();
                seq += 1;
            }
        }]);
        let last = *seqs(j.events_path()).last().unwrap();
        assert_eq!(got, (1..=last).collect::<Vec<u64>>());
    }

    /// Rotura que atrapa: pegar la media línea leída antes de que el que
    /// escribía muriera con el resto de la línea que se escribió encima en
    /// el mismo inodo (`recover` la corta).
    #[test]
    fn follow_drops_a_half_line_that_was_cut_away() {
        let dir = scratch("partial");
        let j = journal(&dir);
        let state = dir.join("state");
        j.record(&ev(1)).unwrap();
        let mut file = OpenOptions::new().append(true).open(j.events_path()).unwrap();
        file.write_all(br#"{"at":"2026-11-27T00:00:00Z","data":{"n":2},"external":false,"id":"lo-que-se"#).unwrap();
        drop(file);
        let got = follow_rounds(&j, &state, "s", &[&|| {
            j.recover().unwrap();
            j.record(&envelope(2, GAP, &iso_of_unix(now()), json!({ "from": 2, "to": 2 }))).unwrap();
            j.record(&ev(3)).unwrap();
        }]);
        assert_eq!(got, vec![1, 2, 3]);
    }

    /// Roturas que atrapa: que `follow` no entregue la línea de reinicio a
    /// quien ya confirmó más allá de su número, que no le baje lo
    /// confirmado (lo siguiente se daría por visto), o que la entregue otra
    /// vez a quien ya la confirmó.
    #[test]
    fn follow_delivers_a_reset_once_and_lowers_the_ack() {
        let dir = scratch("follow-reset");
        let state = dir.join("state");
        let fake = FakeCac::start("cac_pat_T");
        fake.record_only(ev(1));
        let mut listener = Listener::new(journal(&dir)).unwrap();
        for seq in 799..=800 {
            listener.journal.record(&ev(seq)).unwrap();
        }
        listener.cursor = Some(800);
        ack(&state, "tcg", 800).unwrap();
        rt().block_on(listener.catch_up(&creds(&fake))).unwrap();
        listener.journal.record(&ev(2)).unwrap();
        assert_eq!(follow_once(&listener.journal, &state, "tcg", &Filter::default()), vec![1, 2]);
        assert_eq!(progress(&state, "tcg").acked, None);
        assert_eq!(follow_once(&listener.journal, &state, "tcg", &Filter { types: vec!["nada".into()], ..Filter::default() }), vec![1]);
        ack(&state, "tcg", 1).unwrap();
        assert_eq!(follow_once(&listener.journal, &state, "tcg", &Filter::default()), vec![2]);
        ack(&state, "tcg", 2).unwrap();
        assert_eq!(follow_once(&listener.journal, &state, "tcg", &Filter::default()), Vec::<u64>::new());
    }
}
