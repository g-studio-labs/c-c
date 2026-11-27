//! `cac-mcp` y sus versiones, con el binario de verdad: avisar de que hay una
//! nueva sin estorbar, y actualizarse cuando se le pide.
//!
//! Nada sale a internet. `mcp/VERSION` y la release los sirve un servidor de
//! mentira en esta máquina (`cac_mcp::update::fake`), y el binario los busca
//! ahí porque las direcciones se le dan por el entorno.

use cac_mcp::update::fake::FakeReleases;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const INSTALLED: &str = env!("CARGO_PKG_VERSION");
/// Una versión que siempre es más nueva que la de este crate.
const NEWER: &str = "999.0.0";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cac-mcp-updates-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// El binario, con todo lo suyo dentro de `dir` y hablando con `version_url`.
fn binary_at(program: &Path, dir: &Path, version_url: &str, download_base: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_remove("CAC_TOKEN")
        .env_remove("CAC_MCP_NO_UPDATE_CHECK")
        .env("CAC_URL", "http://127.0.0.1:9")
        .env("CLAUDE_CONFIG_DIR", dir.join("claude"))
        .env("HOME", dir)
        .env("CAC_MCP_STATE_DIR", dir.join("state"))
        .env("CAC_MCP_VERSION_URL", version_url)
        .env("CAC_MCP_DOWNLOAD_BASE", download_base)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn binary(dir: &Path, releases: &FakeReleases) -> Command {
    let source = releases.source();
    binary_at(Path::new(env!("CARGO_BIN_EXE_cac-mcp")), dir, &source.version_url, &source.download_base)
}

/// Una sesión abierta: se le pregunta y se lee la respuesta, de una en una.
struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Session {
    fn open(mut cmd: Command) -> Self {
        let mut child = cmd.spawn().expect("cac-mcp arranca");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Session { child, stdin, stdout }
    }

    fn ask(&mut self, request: Value) -> Value {
        writeln!(self.stdin.as_mut().unwrap(), "{request}").unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("no es JSON ({e}): {line}"))
    }

    fn instructions(&mut self) -> String {
        let init = self.ask(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }));
        init["result"]["instructions"].as_str().unwrap().to_string()
    }

    /// Lo que dice una llamada a una herramienta, todos sus bloques juntos.
    /// `list_projects` sin token falla sin salir a ningún sitio: sirve igual.
    fn call(&mut self) -> String {
        let answer = self.ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "list_projects", "arguments": {} } }));
        let blocks = answer["result"]["content"].as_array().unwrap();
        blocks.iter().map(|block| block["text"].as_str().unwrap()).collect::<Vec<_>>().join("\n")
    }

    fn close(mut self) {
        drop(self.stdin.take());
        self.child.wait().unwrap();
    }
}

fn wait_for(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "no llegó a pasar: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn version_asks(releases: &FakeReleases) -> usize {
    releases.asked().iter().filter(|path| *path == "/VERSION").count()
}

/// Hay una versión nueva: la primera sesión no lo sabe al conectar y lo dice
/// en una llamada, una vez; la siguiente lo dice al conectar, con la orden
/// exacta, y no vuelve a preguntar a la red ese día.
///
/// Roturas que atrapa: no avisar nunca; repetir el aviso en cada llamada; no
/// recordar lo sabido entre sesiones; preguntar a la red en cada arranque; un
/// aviso sin la orden para actualizar.
#[test]
fn a_newer_version_is_told_once_and_asked_for_once_a_day() {
    let releases = FakeReleases::start();
    releases.announce(NEWER);
    releases.publish(NEWER, b"binary");
    let dir = scratch("told");

    // La red no contesta hasta haber conectado: si contestaba antes (una
    // máquina cargada), el primer arranque ya lo sabía y la prueba no miraba
    // lo que dice.
    releases.pause();
    let mut first = Session::open(binary(&dir, &releases));
    let at_start = first.instructions();
    assert!(!at_start.contains(NEWER), "en el primer arranque todavía no se sabe: {at_start}");
    releases.resume();
    wait_for("el hilo de fondo apunta lo que le contestaron", || dir.join("state").join("update.json").exists());
    // El fichero se escribe un instante antes de que el servidor lo sepa, y
    // no hay más señal que preguntarle: se pregunta hasta que lo dice.
    let mut told = String::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        told = first.call();
        if told.contains(NEWER) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(told.contains(&format!("cac-mcp {NEWER} is published and this MCP server is {INSTALLED}")), "{told}");
    assert!(told.contains("update`"), "sin la orden: {told}");
    assert!(!first.call().contains(NEWER), "el aviso se repite en cada llamada");
    first.close();

    let mut second = Session::open(binary(&dir, &releases));
    let at_start = second.instructions();
    assert!(at_start.contains(&format!("cac-mcp {NEWER} is published")), "la segunda sesión no lo dice al conectar: {at_start}");
    assert!(at_start.contains("get_context"), "el aviso sustituyó a las instrucciones");
    assert!(!second.call().contains(NEWER), "ya se dijo al conectar");
    second.close();
    assert_eq!(version_asks(&releases), 1, "preguntó a la red más de una vez en el mismo día");
    let _ = std::fs::remove_dir_all(dir);
}

/// Una versión anunciada cuya release aún no está publicada (`mcp/VERSION`
/// sube a `main` antes de que salga el workflow) no se avisa ni se ofrece:
/// ni el aviso al agente, ni `update --check`, ni `update`, que no toca el
/// binario ni falla. Al publicarse, se avisa.
///
/// Roturas que atrapa: avisar al agente de una versión que da 404; que
/// `update --check` diga «available» sin poder bajarla; que `update` falle o
/// reemplace con una descarga rota en vez de decir que no hay nada que
/// actualizar; que `--version` anuncie la no publicada.
#[test]
#[cfg(unix)]
fn a_version_announced_before_its_release_is_neither_told_nor_offered() {
    let releases = FakeReleases::start();
    releases.announce(NEWER);
    let dir = scratch("unpublished");
    let copy = dir.join("bin").join("cac-mcp");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_cac-mcp"), &copy).unwrap();
    let original = std::fs::read(&copy).unwrap();
    let source = releases.source();
    let run = |args: &[&str]| {
        let out = binary_at(&copy, &dir, &source.version_url, &source.download_base).args(args).stdin(Stdio::null()).output().unwrap();
        (out.status.code(), String::from_utf8_lossy(&out.stdout).to_string())
    };

    // El aviso al agente: se espera a que el hilo de fondo haya preguntado.
    let mut session = Session::open(binary(&dir, &releases));
    session.instructions();
    wait_for("el hilo de fondo mira la release", || releases.asked().iter().any(|path| path.ends_with("/SHA256SUMS")));
    std::thread::sleep(Duration::from_millis(200));
    assert!(!session.call().contains(NEWER), "avisó de una versión sin publicar");
    session.close();
    // La siguiente sesión lo sabe de lo apuntado (la última anunciada), y
    // tampoco lo dice al conectar.
    let mut next = Session::open(binary(&dir, &releases));
    assert!(!next.instructions().contains(NEWER), "al conectar avisó de lo apuntado, que no está publicada");
    next.close();

    let (code, out) = run(&["update", "--check"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("announced, but its release isn't published yet") && !out.contains("is available"), "{out}");
    let (code, out) = run(&["update"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("Nothing to update"), "{out}");
    assert_eq!(std::fs::read(&copy).unwrap(), original, "`update` tocó el binario sin release");
    assert!(!run(&["--version"]).1.contains("available"), "`--version` anuncia lo que no se puede bajar");

    releases.publish(NEWER, NEW_BINARY);
    let (_, out) = run(&["update", "--check"]);
    assert!(out.contains("is available"), "{out}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Sin versión nueva no se dice nada, y con la variable puesta ni se pregunta.
///
/// Roturas que atrapa: avisar de la misma versión; ignorar
/// `CAC_MCP_NO_UPDATE_CHECK`.
#[test]
fn nothing_is_said_without_a_newer_version_and_nothing_is_asked_when_disabled() {
    let releases = FakeReleases::start();
    releases.announce(INSTALLED);
    let dir = scratch("same");
    let mut session = Session::open(binary(&dir, &releases));
    session.instructions();
    wait_for("se pregunta", || dir.join("state").join("update.json").exists());
    std::thread::sleep(Duration::from_millis(100));
    let said = session.call();
    assert!(!said.contains("is published"), "{said}");
    session.close();
    assert_eq!(version_asks(&releases), 1);

    let quiet = FakeReleases::start();
    quiet.announce(NEWER);
    let dir2 = scratch("disabled");
    let mut cmd = binary(&dir2, &quiet);
    cmd.env("CAC_MCP_NO_UPDATE_CHECK", "1");
    let mut session = Session::open(cmd);
    assert!(!session.instructions().contains(NEWER));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!session.call().contains(NEWER));
    session.close();
    assert_eq!(version_asks(&quiet), 0, "preguntó aunque se pidió que no");
    assert!(!dir2.join("state").exists(), "apuntó algo aunque se pidió que no");
    for dir in [dir, dir2] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Con un servidor que acepta la conexión y no contesta nunca (peor que no
/// tener red), el servidor arranca, contesta y termina sin esperarlo.
///
/// Rotura que atrapa: preguntar por la versión en el hilo que atiende al
/// cliente, al arrancar o en una llamada: la sesión esperaría los 8 segundos
/// del tiempo límite, o para siempre.
#[test]
fn a_server_that_never_answers_delays_nothing() {
    let hung = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/VERSION", hung.local_addr().unwrap());
    let dir = scratch("hung");
    // Con un plazo que no se acaba, una sesión que esperara a la red no
    // terminaría nunca: terminar es no haberla esperado. Medir cuánto tardaba
    // dependía de lo cargada que estuviera la máquina.
    let mut cmd = binary_at(Path::new(env!("CARGO_BIN_EXE_cac-mcp")), &dir, &url, "http://127.0.0.1:9");
    cmd.env("CAC_MCP_VERSION_TIMEOUT", "86400");
    let (done, finished) = std::sync::mpsc::channel();
    let (close, closing) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut session = Session::open(cmd);
        let said = (session.instructions(), session.call());
        let _ = done.send(said);
        let _ = closing.recv();
        session.close();
    });
    // Sólo para que una rotura no cuelgue la prueba un día entero.
    let (instructions, called) = finished.recv_timeout(Duration::from_secs(120)).expect("la sesión esperó a la red");
    assert!(instructions.contains("get_context"));
    assert!(called.contains("CAC_TOKEN is not set"));
    // Y sí que lo intentó, con la sesión aún abierta: el hilo de fondo
    // conecta cuando puede, y cerrar antes lo mataba sin haberlo hecho.
    hung.set_nonblocking(true).unwrap();
    wait_for("llega a preguntar", || hung.accept().is_ok());
    let _ = close.send(());
    let _ = std::fs::remove_dir_all(dir);
}

/// Un «binario nuevo» que se puede ejecutar aquí: dice su versión, como el de
/// verdad.
#[cfg(unix)]
const NEW_BINARY: &[u8] = b"#!/bin/sh\necho 'cac-mcp 999.0.0'\n";

/// `cac-mcp update` baja la versión nueva, la comprueba y se reemplaza a sí
/// mismo; `update --check` y `--version` sólo informan; y una descarga cuya
/// suma no es la publicada no toca el binario.
///
/// Roturas que atrapa: un `update --check` que instala; un `update` que no
/// reemplaza, o reemplaza otro fichero; instalar con la suma cambiada; salir
/// con éxito sin haber actualizado; un `--version` que sale a la red o que
/// cambia su primera línea.
#[test]
#[cfg(unix)]
fn update_replaces_the_binary_itself_only_when_asked_and_only_if_verified() {
    let releases = FakeReleases::start();
    releases.announce(NEWER);
    releases.publish(NEWER, NEW_BINARY);
    let dir = scratch("self");
    let copy = dir.join("bin").join("cac-mcp");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_cac-mcp"), &copy).unwrap();
    let source = releases.source();
    let run = |args: &[&str]| {
        let out = binary_at(&copy, &dir, &source.version_url, &source.download_base).args(args).stdin(Stdio::null()).output().unwrap();
        (out.status.code(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
    };
    let original = std::fs::read(&copy).unwrap();

    // Sin haber preguntado nunca, `--version` sólo sabe la suya y no pregunta.
    assert_eq!(run(&["--version"]), (Some(0), format!("cac-mcp {INSTALLED}\n"), String::new()));
    assert_eq!(version_asks(&releases), 0, "--version salió a la red");

    let (code, out, _) = run(&["update", "--check"]);
    assert_eq!(code, Some(0));
    assert!(out.contains(&format!("cac-mcp {NEWER} is available (installed: {INSTALLED})")), "{out}");
    assert_eq!(std::fs::read(&copy).unwrap(), original, "--check instaló");
    // Y ahora `--version` lo sabe, de lo apuntado.
    assert_eq!(run(&["--version"]).1, format!("cac-mcp {INSTALLED}\navailable: {NEWER} (run `cac-mcp update`)\n"));

    // La suma publicada no es la de lo que se baja.
    let asset = cac_mcp::update::this_platform_asset().unwrap();
    releases.put(&format!("/download/mcp-v{NEWER}/{asset}"), "#!/bin/sh\necho 'cac-mcp 999.0.0'\necho robado\n");
    let (code, _, err) = run(&["update"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("checksum mismatch") && err.contains("is still installed"), "{err}");
    assert_eq!(std::fs::read(&copy).unwrap(), original, "una descarga rechazada tocó el binario");

    releases.publish(NEWER, NEW_BINARY);
    let (code, out, err) = run(&["update"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("takes effect in the next session"), "{out}");
    assert_eq!(std::fs::read(&copy).unwrap(), NEW_BINARY);
    assert_eq!(run(&["--version"]).1, "cac-mcp 999.0.0\n");
    let left: Vec<_> = std::fs::read_dir(copy.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, ["cac-mcp"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Ya en la última: `update` lo dice y no baja nada. Y sin red, `update`
/// falla diciéndolo, en vez de dar por bueno lo instalado.
///
/// Roturas que atrapa: reinstalar la misma versión cada vez; un `update` sin
/// red que sale con éxito.
#[test]
fn update_does_nothing_when_current_and_fails_aloud_when_offline() {
    let releases = FakeReleases::start();
    releases.announce(INSTALLED);
    let dir = scratch("current");
    let out = binary(&dir, &releases).arg("update").stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("is the latest version"));
    assert!(!releases.asked().iter().any(|path| path.starts_with("/download/")), "{:?}", releases.asked());

    let offline = binary_at(Path::new(env!("CARGO_BIN_EXE_cac-mcp")), &dir, "http://127.0.0.1:9/VERSION", "http://127.0.0.1:9")
        .args(["update", "--check"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(offline.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&offline.stderr).contains("could not find out the latest version"));
    let _ = std::fs::remove_dir_all(dir);
}
