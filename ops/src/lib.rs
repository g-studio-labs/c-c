//! Lo que Plataforma hace en la máquina local: hablar por SSH con los
//! servidores, guardar en el llavero del sistema lo que es de cada uno, y
//! llevar los secretos de 1Password a los servicios y a GitHub, y bajar un
//! backup a un fichero.
//!
//! Sale de la app de Tauri (`app/src-tauri/src/lib.rs`) para que la app nativa
//! use lo mismo: es código sensible —llaves, tokens, qué viaja como argumento y
//! qué por stdin— y dos copias acaban diciendo cosas distintas. Las apps sólo
//! le ponen encima su capa fina (en Tauri, los `#[tauri::command]`).
//!
//! Sin interfaz (guardián: `tests/no_ui.rs`). Todo programa del sistema se
//! lanza con `cac_mcp::host::command`, nunca con `Command::new` (guardián:
//! `tests::every_system_program_is_launched_through_host`).

pub mod agent_install;
pub mod download;
pub mod github;
pub mod keychain;
pub mod op;
pub mod op_env;
pub mod secrets;
pub mod ssh;

#[cfg(test)]
mod tests {
    /// Nadie en este crate lanza un programa por su nombre sin limpiar el
    /// entorno de la AppImage. Rotura que detecta: un `Command::new("ssh")`
    /// nuevo aquí, que heredaría `LD_LIBRARY_PATH` de la imagen y moriría
    /// como murió `xdg-open` (ver `mcp/src/host.rs`). El guardián del MCP
    /// sólo mira sus fuentes y las de la app; éstas se miran aquí.
    #[test]
    fn every_system_program_is_launched_through_host() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let bad = cac_mcp::host::unclean_launches(&here.join("src"));
        assert!(bad.is_empty(), "lanzan un programa sin limpiar el entorno: {bad:#?}");
    }
}
