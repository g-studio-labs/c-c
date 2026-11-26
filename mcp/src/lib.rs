//! El servidor MCP de cac, y lo que necesita para servir sin la app de
//! escritorio.
//!
//! Dos programas sirven lo mismo desde aquí: el binario `cac-mcp` de este crate
//! (`src/main.rs`) y `cac --mcp`, la app de Tauri, que depende de este crate
//! por ruta para que las instalaciones que ya apuntan a ella sigan igual. Una
//! sola fuente: la app no lleva copia de nada de esto.
//!
//! Por eso aquí no entra nada que pinte (guardián: `tests/no_ui.rs`). Los
//! módulos que también usa la app —comprimir una imagen o un PDF, leer un
//! fichero soltado, Ansible, 1Password, el agente SSH, limpiar el entorno de la
//! AppImage— son públicos y la app los toma de aquí; lo que es suyo (los
//! comandos de Tauri, abrir un enlace) se queda en `app/src-tauri` como una
//! capa fina.

// El catálogo de herramientas del MCP (`server::tool_defs`) es un solo `json!`,
// y con las de etiquetas, responsables y fechas pasó el límite de 128 niveles
// de macro que trae Rust. Subirlo es lo que pide el propio compilador; partir
// el catálogo en trozos sólo movería el mismo JSON a más sitios. Con
// `install_hooks` (5-nov-2026) pasó también de 256.
#![recursion_limit = "512"]

pub mod ansible;
pub mod claude_code;
pub mod desktop_update;
pub mod dropped;
pub mod files;
pub mod hooks_install;
pub mod host;
pub mod image;
pub mod op;
pub mod pdf;
pub mod provision_cli;
mod server;
pub mod ssh_agent;
pub mod update;

pub use server::{serve, tool_catalog, Host};
