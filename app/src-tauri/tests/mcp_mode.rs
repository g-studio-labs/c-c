//! `cac --mcp` sigue sirviendo el MCP.
//!
//! El servidor vive en su crate (`mcp/`) y esta app lo usa por ruta, pero las
//! instalaciones que ya existen apuntan aquí: en su configuración de Claude
//! Code pone la ruta de la app y `--mcp`. Se arranca el binario de la app de
//! verdad y se le habla por stdio.
//!
//! Sin pantalla (`DISPLAY` y `WAYLAND_DISPLAY` fuera): si `--mcp` dejara de
//! desviarse antes de la interfaz, la prueba falla en vez de abrir una ventana
//! en el escritorio de quien la corre. Sin token, sin red, y con la carpeta de
//! Claude Code en una temporal.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

/// Roturas que detecta: que `--mcp` deje de llegar al servidor (o llegue a
/// otro, con otro catálogo), y que la app pase a anunciar la versión del crate
/// del MCP en vez de la suya.
#[test]
fn the_app_still_serves_the_same_mcp_with_its_own_version() {
    let scratch = std::env::temp_dir().join(format!("cac-app-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .arg("--mcp")
        .env_remove("CAC_TOKEN")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("CAC_URL", "http://127.0.0.1:9")
        .env("CLAUDE_CONFIG_DIR", &scratch)
        .env("HOME", &scratch)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("la app arranca");
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} })).unwrap();
        writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
    }
    let lines: Vec<String> = BufReader::new(child.stdout.take().unwrap()).lines().map(|l| l.unwrap()).collect();
    let status = child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&scratch);

    assert_eq!(lines.len(), 2, "`cac --mcp` no contestó como un servidor MCP: {lines:?}");
    assert!(status.success(), "{status}");
    let init: Value = serde_json::from_str(&lines[0]).unwrap();
    // La versión de la app (la que la release escribe en su Cargo.toml), no la
    // del crate del MCP.
    assert_eq!(init["result"]["serverInfo"], json!({ "name": "cac", "version": env!("CARGO_PKG_VERSION") }));
    let list: Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(list["result"]["tools"], cac_mcp::tool_catalog());
}
