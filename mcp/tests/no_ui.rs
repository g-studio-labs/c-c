//! Este crate no enlaza ninguna interfaz.
//!
//! Es lo que hace que `cac-mcp` sea un binario que se instala solo, en una
//! máquina sin escritorio: con `tauri`, `gtk` o `webkit` dentro pediría las
//! bibliotecas del sistema que se sacó de la app para no pedir. Y es un fallo
//! que no avisa: en la máquina de quien desarrolla esas bibliotecas están, y
//! todo compila y corre igual.
//!
//! Se miran dos sitios. El manifiesto, lo que este crate pide por su nombre; y
//! `Cargo.lock`, todo lo que acaba dentro, porque una interfaz puede llegar de
//! la mano de otra dependencia sin que aquí se nombre.

const UI: [&str; 7] = ["tauri", "wry", "tao", "gtk", "webkit", "gpui", "objc2"];

fn is_ui(name: &str) -> bool {
    UI.iter().any(|ui| name.starts_with(ui))
}

/// Las dependencias que declara un manifiesto, por su nombre. También las de
/// `[target.'cfg(…)'.dependencies]` y `[dev-dependencies]`: una interfaz que
/// sólo se enlaza en una plataforma o en las pruebas sigue estando dentro.
fn declared(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_dependencies = line.trim_end_matches(']').ends_with("dependencies");
            continue;
        }
        if !in_dependencies || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            names.push(name.trim().trim_matches('"').to_string());
        }
    }
    names
}

/// Los paquetes de un `Cargo.lock`, por su nombre.
fn locked(lock: &str) -> Vec<String> {
    lock.lines()
        .filter_map(|l| l.strip_prefix("name = \""))
        .map(|rest| rest.trim_end_matches('"').to_string())
        .collect()
}

fn read(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_mcp_depends_on_no_ui() {
    let asked: Vec<String> = declared(&read("Cargo.toml")).into_iter().filter(|n| is_ui(n)).collect();
    assert!(asked.is_empty(), "el MCP no puede depender de {asked:?}");
    let linked: Vec<String> = locked(&read("Cargo.lock")).into_iter().filter(|n| is_ui(n)).collect();
    assert!(linked.is_empty(), "al MCP le llega una interfaz por otra dependencia: {linked:?}");
}

/// Los dos lectores ven de verdad lo que hay: sin esto, uno que no encontrara
/// nada daría la prueba de arriba por buena para siempre.
#[test]
fn the_readers_find_what_is_there() {
    let manifest = "[package]\nname = \"x\"\n\n[dependencies]\n# nota\nserde = \"1\"\n\
                    tauri = { version = \"2\" }\n\n[target.'cfg(unix)'.dependencies]\ngtk = \"0.18\"\n";
    assert_eq!(declared(manifest), ["serde", "tauri", "gtk"]);
    let lock = "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"webkit2gtk\"\nversion = \"2.0.1\"\n";
    assert_eq!(locked(lock), ["serde", "webkit2gtk"]);
    // Y sobre los ficheros de verdad: lo que este crate sí usa, está.
    assert!(declared(&read("Cargo.toml")).contains(&"reqwest".to_string()));
    assert!(locked(&read("Cargo.lock")).contains(&"reqwest".to_string()));
}
