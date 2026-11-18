//! `cac-mcp`: el servidor MCP de cac, solo.
//!
//! Sin argumentos sirve MCP por stdio, igual que `cac --mcp`: desde ahí stdout
//! lleva sólo JSON-RPC. La identidad es la de siempre, `CAC_TOKEN` y `CAC_URL`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => cac_mcp::serve(cac_mcp::Host::standalone()),
        Some("--version" | "-V") => println!("cac-mcp {}", env!("CARGO_PKG_VERSION")),
        // Un argumento que no se entiende no puede dejar el proceso esperando
        // JSON-RPC por stdin: quien se equivocó al escribirlo no vería nada.
        Some(other) => {
            eprintln!("cac-mcp: unknown argument `{other}`. Run it with no arguments to serve MCP over stdio, or with --version.");
            std::process::exit(2);
        }
    }
}
