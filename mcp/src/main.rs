//! `cac-mcp`: el servidor MCP de cac, solo.
//!
//! Sin argumentos sirve MCP por stdio, igual que `cac --mcp`: desde ahí stdout
//! lleva sólo JSON-RPC. La identidad es la de siempre, `CAC_TOKEN` y `CAC_URL`.
//!
//! `cac-mcp update` lo actualiza (nunca se actualiza solo) y `--version` dice
//! cuál es y, si ya se sabe de una más nueva, cuál hay.

use std::time::Duration;

use cac_mcp::update::{self, Progress, Source};

const USAGE: &str = "Run it with no arguments to serve MCP over stdio. Other uses: --version, update, update --check.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = match args.as_slice() {
        [] => {
            // Lo que una actualización anterior no pudo borrar (Windows).
            if let Ok(exe) = std::env::current_exe() {
                update::clear_leftovers(&exe);
            }
            cac_mcp::serve(cac_mcp::Host::standalone());
            0
        }
        ["--version" | "-V"] => {
            print!("{}", version_text(update::state_dir().and_then(|dir| update::read_checked(&dir)).and_then(|checked| checked.latest).as_deref()));
            0
        }
        ["update"] => run_update(false),
        ["update", "--check"] => run_update(true),
        // Un argumento que no se entiende no puede dejar el proceso esperando
        // JSON-RPC por stdin: quien se equivocó al escribirlo no vería nada.
        other => {
            eprintln!("cac-mcp: unknown arguments `{}`. {USAGE}", other.join(" "));
            2
        }
    };
    std::process::exit(code);
}

const INSTALLED: &str = env!("CARGO_PKG_VERSION");

/// La primera línea es siempre `cac-mcp X.Y.Z`: es lo que lee quien pregunta
/// qué versión hay instalada (`update::version_of`). La de «hay otra» sólo
/// sale de lo ya sabido: `--version` no sale a la red.
fn version_text(latest: Option<&str>) -> String {
    let mut text = format!("cac-mcp {INSTALLED}\n");
    if let Some(latest) = latest.filter(|latest| update::is_newer(latest, INSTALLED)) {
        text.push_str(&format!("available: {latest} (run `cac-mcp update`)\n"));
    }
    text
}

/// `update` y `update --check`. Aquí sí se pregunta a la red, ahora: lo pidió
/// una persona. Lo que dice va por stdout; lo que falla, por stderr y con
/// código 1.
fn run_update(only_check: bool) -> i32 {
    let source = Source::from_env();
    let latest = match update::check_now(&source, update::state_dir().as_deref(), Duration::from_secs(20)) {
        Ok(latest) => latest,
        Err(e) => {
            eprintln!("cac-mcp: could not find out the latest version: {e}");
            return 1;
        }
    };
    if !update::is_newer(&latest, INSTALLED) {
        println!("cac-mcp {INSTALLED} is the latest version.");
        return 0;
    }
    if only_check {
        println!("cac-mcp {latest} is available (installed: {INSTALLED}). Run `cac-mcp update` to install it.");
        return 0;
    }
    // El fichero de verdad, no un enlace que apunte a él: se reemplaza el
    // binario, y el enlace sigue valiendo.
    let exe = match std::env::current_exe().and_then(|exe| exe.canonicalize()) {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("cac-mcp: could not find its own file: {e}");
            return 1;
        }
    };
    println!("Updating {} from {INSTALLED} to {latest}…", exe.display());
    let mut said_verifying = false;
    let done = update::install(&source, &latest, &exe, &mut |step| {
        if step == Progress::Verifying && !said_verifying {
            said_verifying = true;
            println!("Downloaded. Verifying the checksum…");
        }
    });
    match done {
        Ok(()) => {
            println!("cac-mcp {latest} is installed. It takes effect in the next session: in Claude Code, /mcp → cac → reconnect, or open a new one.");
            0
        }
        Err(e) => {
            eprintln!("cac-mcp: not updated, {INSTALLED} is still installed. {e}");
            1
        }
    }
}
