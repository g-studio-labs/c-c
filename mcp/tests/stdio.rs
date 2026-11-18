//! El binario `cac-mcp`, de punta a punta: se arranca de verdad y se le habla
//! por stdio, como hace un cliente MCP.
//!
//! Sin token y sin red: `initialize` y `tools/list` no llaman a cac, y
//! `CAC_URL` apunta a un puerto cerrado de esta máquina por si algo llamara.
//! La carpeta de Claude Code (`CLAUDE_CONFIG_DIR`) es una temporal: `initialize`
//! mira ahí si las skills y los hooks están instalados, y no tiene por qué
//! mirar los de quien corre las pruebas.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn binary() -> Command {
    let scratch = std::env::temp_dir().join(format!("cac-mcp-stdio-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cac-mcp"));
    cmd.env_remove("CAC_TOKEN")
        .env("CAC_URL", "http://127.0.0.1:9")
        .env("CLAUDE_CONFIG_DIR", &scratch)
        .env("HOME", &scratch)
        // Arrancar el servidor pregunta por la última versión: aquí, a nadie.
        // Las pruebas que miran eso (`tests/updates.rs`) ponen su servidor.
        .env("CAC_MCP_NO_UPDATE_CHECK", "1")
        .env("CAC_MCP_STATE_DIR", scratch.join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Manda las peticiones, cierra stdin y devuelve lo que salió por stdout,
/// línea a línea, y el código de salida.
fn talk(requests: &[Value]) -> (Vec<String>, Option<i32>) {
    let mut child = binary().spawn().expect("cac-mcp arranca");
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            writeln!(stdin, "{r}").unwrap();
        }
        // Al soltarlo se cierra: el servidor termina con el fin de la entrada.
    }
    let lines: Vec<String> = BufReader::new(child.stdout.take().unwrap()).lines().map(|l| l.unwrap()).collect();
    let status = child.wait().unwrap();
    (lines, status.code())
}

/// Sin argumentos, `cac-mcp` es un servidor MCP: contesta `initialize` con su
/// nombre y su versión, y `tools/list` con el catálogo de este crate, sin
/// token. Roturas que detecta: que el binario no sirva (o sirva otra cosa),
/// que anuncie una versión que no es la suya, que escriba por stdout algo que
/// no sea JSON-RPC —un `println!` de más rompe el protocolo para todo
/// cliente—, o que no termine cuando el cliente cierra.
#[test]
fn with_no_arguments_it_serves_mcp_over_stdio_without_a_token() {
    let (lines, code) = talk(&[
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    ]);
    assert_eq!(code, Some(0), "al cerrarse la entrada, termina bien");
    let answers: Vec<Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("por stdout salió algo que no es JSON ({e}): {l}")))
        .collect();
    // Dos respuestas: la notificación no lleva `id` y no se contesta.
    assert_eq!(answers.len(), 2, "{lines:?}");

    let init = &answers[0];
    assert_eq!(init["id"], json!(1));
    assert_eq!(init["result"]["serverInfo"], json!({ "name": "cac", "version": env!("CARGO_PKG_VERSION") }));
    assert!(init["result"]["instructions"].as_str().unwrap().contains("get_context"));

    let tools = &answers[1]["result"]["tools"];
    assert_eq!(answers[1]["id"], json!(2));
    assert_eq!(tools, &cac_mcp::tool_catalog(), "el binario sirve otro catálogo");
    let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in ["get_context", "update_task", "request_provisioning", "install_skill"] {
        assert!(names.contains(&expected), "falta {expected}");
    }
}

/// Una herramienta que llama a cac, sin token, dice qué falta en vez de morir
/// o de quedarse callada. Rotura que detecta: que el servidor exija el token
/// para arrancar (hoy arranca sin él y lo dice en la llamada), o que intente
/// la llamada sin identidad.
#[test]
fn a_tool_that_needs_cac_says_the_token_is_missing() {
    let (lines, _) = talk(&[json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": { "name": "list_projects", "arguments": {} }
    })]);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let answer: Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(answer["result"]["isError"], json!(true));
    assert!(answer["result"]["content"][0]["text"].as_str().unwrap().contains("CAC_TOKEN is not set"), "{answer}");
}

/// `--version` dice la versión y se va; un argumento desconocido falla sin
/// tocar stdout. Roturas que detecta: que `--version` se ponga a servir (quien
/// lo corre en una terminal vería un proceso colgado), que imprima otra
/// versión, o que un argumento mal escrito deje el proceso esperando JSON-RPC.
#[test]
fn version_and_unknown_arguments_do_not_start_a_server() {
    let out = binary().arg("--version").stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), format!("cac-mcp {}\n", env!("CARGO_PKG_VERSION")));

    // Con stdin abierto y sin escribirle: si se pusiera a servir, no volvería.
    let mut child = binary().arg("--mcpp").spawn().unwrap();
    let _keep_open = child.stdin.take().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("con un argumento desconocido se quedó esperando");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(2));
    let mut stdout = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut stdout).unwrap();
    assert!(stdout.is_empty(), "stdout es del protocolo: {stdout}");
}
