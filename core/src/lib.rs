//! La lógica de cac que comparten todas sus apps.
//!
//! Una sola base para el escritorio, el móvil y el MCP: lo que decide algo
//! —la sesión, qué se pide al servidor, en qué día cae un vencimiento— vive
//! aquí, y cada plataforma sólo pinta y aporta lo suyo del sistema. El diseño
//! entero está en cac, en la página «Diseño · Core en Rust compartido».

#[cfg(test)]
mod tests {
    /// Las dependencias que declara el manifiesto, por su nombre.
    fn dependency_names(manifest: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut in_dependencies = false;
        for line in manifest.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                // También `[target.'cfg(…)'.dependencies]` y `[dev-dependencies]`:
                // una interfaz que sólo se enlaza en una plataforma o en las
                // pruebas sigue siendo una interfaz dentro del core.
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

    /// El core no enlaza ninguna interfaz.
    ///
    /// Es lo que lo hace compartible: con `tauri` o `gtk` dentro dejaría de
    /// compilar para Android, y el día que el escritorio cambie de interfaz se
    /// llevaría la lógica detrás. Nada más lo avisaría hasta ese día.
    #[test]
    fn the_core_depends_on_no_ui() {
        const UI: [&str; 7] = ["tauri", "wry", "tao", "gtk", "webkit", "gpui", "objc2"];
        let names = dependency_names(include_str!("../Cargo.toml"));
        let offending: Vec<&String> = names
            .iter()
            .filter(|name| UI.iter().any(|ui| name.starts_with(ui)))
            .collect();
        assert!(offending.is_empty(), "el core no puede depender de {offending:?}");
    }

    /// El lector de arriba ve de verdad las dependencias: sin esto, uno que no
    /// encontrara ninguna daría la prueba anterior por buena para siempre.
    #[test]
    fn the_manifest_reader_finds_dependencies() {
        let manifest = "[package]\nname = \"x\"\n\n[dependencies]\n# nota\nserde = \"1\"\n\
                        tauri = { version = \"2\" }\n\n[target.'cfg(unix)'.dependencies]\ngtk = \"0.18\"\n";
        assert_eq!(dependency_names(manifest), ["serde", "tauri", "gtk"]);
    }
}
