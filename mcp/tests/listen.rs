//! `cac-mcp listen`, con el binario de verdad contra un cac de mentira
//! (`cac_mcp::listen::fake`): escribir cada evento una vez aunque se muera,
//! retomar desde donde iba, no dejar el token en ningún sitio, y leer con
//! `follow` y `ack`. Nada sale a internet.

use cac_mcp::listen::fake::{envelope, FakeCac};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "cac_pat_LISTENSECRET0123456789";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cac-mcp-listen-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// El binario con todo lo suyo dentro de `dir`, y sin token en el entorno.
fn binary(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cac-mcp"));
    cmd.env_remove("CAC_TOKEN")
        .env_remove("CAC_URL")
        .env("HOME", dir)
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("CLAUDE_CONFIG_DIR", dir.join("claude"))
        .env("CAC_MCP_STATE_DIR", dir.join("state"))
        .env("CAC_MCP_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn events_path(dir: &Path) -> PathBuf {
    dir.join("cache").join("cac").join("events.jsonl")
}

fn lines(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(events_path(dir)).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn seqs(dir: &Path) -> Vec<u64> {
    lines(dir).iter().map(|l| l["seq"].as_u64().unwrap()).collect()
}

/// Espera a que se cumpla algo, con un plazo generoso: lo que se comprueba es
/// que pasa, no cuánto tarda.
/// Si no pasa, enseña lo que dijo el binario: sin eso, un rojo aquí no dice
/// nada de por qué.
fn eventually(what: &str, child: &mut Child, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let status = child.wait();
            let mut said = String::new();
            let _ = child.stdout.take().unwrap().read_to_string(&mut said);
            let _ = child.stderr.take().unwrap().read_to_string(&mut said);
            panic!("no pasó: {what}; el binario salió con {status:?} y dijo:\n{said}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ev(seq: u64) -> Value {
    envelope(seq, "basecamp:event", "2099-01-01T00:00:00Z", json!({ "project": { "id": 1, "name": "TCG" }, "text": format!("evento {seq}") }))
}

fn stop(mut child: Child) -> String {
    let _ = child.kill();
    let _ = child.wait();
    let mut text = String::new();
    child.stdout.take().unwrap().read_to_string(&mut text).unwrap();
    child.stderr.take().unwrap().read_to_string(&mut text).unwrap();
    text
}

fn streams_opened(fake: &FakeCac) -> Vec<Option<String>> {
    fake.asked().into_iter().filter(|a| a.path == "/api/v1/events").map(|a| a.last_event_id).collect()
}

/// Todo lo que el binario dejó en su carpeta, junto, para buscar el token.
fn everything_written(dir: &Path) -> String {
    fn walk(dir: &Path, out: &mut String) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if !path.ends_with(".claude.json") {
                out.push_str(&String::from_utf8_lossy(&std::fs::read(&path).unwrap_or_default()));
            }
        }
    }
    let mut out = String::new();
    walk(dir, &mut out);
    out
}

/// Roturas que atrapa: perder lo que pasó con el proceso muerto (no pedir el
/// diario al volver), repetirlo (no leer el cursor de la última línea), no
/// mandar `Last-Event-ID`, o no leer el token del `.claude.json` (el servicio
/// de systemd no tiene `CAC_TOKEN`). Y el token no queda en ningún fichero ni
/// en la salida.
#[test]
fn every_event_lands_once_across_a_restart() {
    let dir = scratch("restart");
    let fake = FakeCac::start(TOKEN);

    // Primera vez, con el token en el entorno: el diario está vacío.
    let mut first = binary(&dir).env("CAC_TOKEN", TOKEN).env("CAC_URL", &fake.base).args(["listen", "run"]).spawn().unwrap();
    eventually("abre el stream", &mut first, || streams_opened(&fake).len() == 1);
    for seq in 1..=3 {
        fake.push(ev(seq));
    }
    eventually("escribe 1, 2 y 3", &mut first, || seqs(&dir) == vec![1, 2, 3]);
    let said = stop(first);

    // Con el proceso muerto pasan dos cosas; la siguiente vez el token sale
    // del `.claude.json`, no del entorno.
    fake.record_only(ev(4));
    fake.record_only(ev(5));
    std::fs::create_dir_all(dir.join("claude")).unwrap();
    std::fs::write(dir.join("claude").join(".claude.json"), json!({ "mcpServers": { "cac": { "env": { "CAC_URL": fake.base, "CAC_TOKEN": TOKEN } } } }).to_string()).unwrap();
    let mut second = binary(&dir).args(["listen", "run"]).spawn().unwrap();
    eventually("vuelve a abrir el stream", &mut second, || streams_opened(&fake).len() == 2);
    fake.push(ev(6));
    eventually("escribe 4, 5 y 6", &mut second, || seqs(&dir).len() >= 6);
    let said = said + &stop(second);

    assert_eq!(seqs(&dir), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(streams_opened(&fake), vec![Some("0".to_string()), Some("5".to_string())]);
    assert!(fake.asked().iter().all(|a| a.authorized), "{:?}", fake.asked());
    assert!(!said.contains(TOKEN), "{said}");
    assert!(!everything_written(&dir).contains(TOKEN));
    let status: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("state").join("listen.status.json")).unwrap()).unwrap();
    assert_eq!(status["state"], "connected");
}

/// Roturas que atrapa: no dejar dicho por qué no escucha (quien mira
/// `status` no lo sabría), o volver a preguntar enseguida con un token que
/// cac ya rechazó.
#[test]
fn a_refused_token_is_said_and_not_hammered() {
    let dir = scratch("refused");
    let fake = FakeCac::start(TOKEN);
    fake.refuse(403);
    let mut child = binary(&dir).env("CAC_TOKEN", TOKEN).env("CAC_URL", &fake.base).args(["listen", "run"]).spawn().unwrap();
    let status_path = dir.join("state").join("listen.status.json");
    let status = || std::fs::read_to_string(&status_path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or(Value::Null);
    eventually("dice que no lo dejan", &mut child, || status()["state"] == "unauthorized");
    let asked = fake.asked().len();
    let said = stop(child);
    assert_eq!(asked, 1, "{:?}", fake.asked());
    assert!(status()["reason"].as_str().unwrap().contains("missing-scope:events:read"), "{}", status());
    assert!(status()["retryInSeconds"].as_u64().unwrap() >= 300);
    assert!(!said.contains(TOKEN) && !everything_written(&dir).contains(TOKEN));
}

fn run(cmd: &mut Command) -> Output {
    cmd.output().unwrap()
}

fn printed(out: &Output) -> Vec<u64> {
    String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["seq"].as_u64().unwrap()).collect()
}

/// Roturas que atrapa: que `follow` vuelva a dar lo confirmado (la sesión lo
/// trataría dos veces cada vez que el Monitor vence), que `ack` retroceda, o
/// que un consumidor sea una ruta fuera de su carpeta.
#[test]
fn follow_gives_each_event_once_per_consumer() {
    let dir = scratch("follow");
    std::fs::create_dir_all(events_path(&dir).parent().unwrap()).unwrap();
    let text: String = (1..=3).map(|s| format!("{}\n", ev(s))).collect();
    std::fs::write(events_path(&dir), text).unwrap();

    let follow = |consumer: &str| run(binary(&dir).args(["listen", "follow", "--consumer", consumer, "--once"]));
    assert_eq!(printed(&follow("tcg")), vec![1, 2, 3]);
    let acked = run(binary(&dir).args(["listen", "ack", "--consumer", "tcg", "2"]));
    assert!(acked.status.success(), "{acked:?}");
    assert_eq!(printed(&follow("tcg")), vec![3]);
    assert!(run(binary(&dir).args(["listen", "ack", "--consumer", "tcg", "1"])).status.success());
    assert_eq!(printed(&follow("tcg")), vec![3]);
    assert_eq!(printed(&follow("otro")), vec![1, 2, 3]);

    let bad = run(binary(&dir).args(["listen", "ack", "--consumer", "../x", "3"]));
    assert_eq!(bad.status.code(), Some(2));
    assert_eq!(run(binary(&dir).args(["listen", "follow"])).status.code(), Some(2));
    assert_eq!(run(binary(&dir).args(["listen", "nada"])).status.code(), Some(2));
}

/// Roturas que atrapa: que el cursor no baje cuando el diario de cac vuelve a
/// empezar (cursor 800, cabecera 3: todo lo nuevo se saltaría como ya
/// escrito), que `recover` lo vuelva a subir, que `follow` no entregue la
/// línea de reinicio a quien confirmó hasta 800 o no le baje lo confirmado, o
/// que un `ack` tardío del diario viejo lo vuelva a dejar sordo.
#[test]
fn a_journal_that_starts_again_is_followed_from_its_new_head() {
    let dir = scratch("reset");
    let fake = FakeCac::start(TOKEN);
    for seq in 1..=3 {
        fake.record_only(ev(seq));
    }
    std::fs::create_dir_all(events_path(&dir).parent().unwrap()).unwrap();
    std::fs::write(events_path(&dir), format!("{}\n{}\n", ev(799), ev(800))).unwrap();
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(dir.join("state").join("listen.json"), json!({ "cursor": 800 }).to_string()).unwrap();
    assert!(run(binary(&dir).args(["listen", "ack", "--consumer", "tcg", "800"])).status.success());

    let mut child = binary(&dir).env("CAC_TOKEN", TOKEN).env("CAC_URL", &fake.base).args(["listen", "run"]).spawn().unwrap();
    eventually("abre el stream desde la cabecera nueva", &mut child, || streams_opened(&fake) == vec![Some("3".to_string())]);
    fake.push(ev(4));
    eventually("escribe el 4", &mut child, || seqs(&dir) == vec![3, 4]);
    let said = stop(child);
    assert!(!said.contains(TOKEN), "{said}");
    assert_eq!(lines(&dir)[0]["data"]["reason"], "reset");

    // Arranca otra vez: la línea manda sobre el estado, aunque éste diga más.
    std::fs::write(dir.join("state").join("listen.json"), json!({ "cursor": 800 }).to_string()).unwrap();
    let mut again = binary(&dir).env("CAC_TOKEN", TOKEN).env("CAC_URL", &fake.base).args(["listen", "run"]).spawn().unwrap();
    eventually("vuelve desde el 4", &mut again, || streams_opened(&fake).len() == 2);
    fake.push(ev(5));
    eventually("escribe el 5", &mut again, || seqs(&dir) == vec![3, 4, 5]);
    stop(again);
    assert_eq!(streams_opened(&fake)[1], Some("4".to_string()));

    let follow = |consumer: &str| run(binary(&dir).args(["listen", "follow", "--consumer", consumer, "--once"]));
    assert_eq!(printed(&follow("tcg")), vec![3, 4, 5]);
    assert!(run(binary(&dir).args(["listen", "ack", "--consumer", "tcg", "4"])).status.success());
    assert!(run(binary(&dir).args(["listen", "ack", "--consumer", "tcg", "800"])).status.success());
    assert_eq!(printed(&follow("tcg")), vec![5]);
    assert_eq!(printed(&follow("otro")), vec![3, 4, 5]);
}
