//! La lógica de cac que comparten todas sus apps.
//!
//! Una sola base para el escritorio, el móvil y el MCP: lo que decide algo
//! —la sesión, qué se pide al servidor, en qué día cae un vencimiento— vive
//! aquí, y cada plataforma sólo pinta y aporta lo suyo del sistema. El diseño
//! entero está en cac, en la página «Diseño · Core en Rust compartido».

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

pub mod dates;
pub mod error;
pub mod events;
pub mod mywork;
pub mod session;
pub mod storage;
pub mod transport;

use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use error::ApiError;
pub use events::{Listener, NoListener};
pub use mywork::{MyWork, MyWorkState, OpenTask, ScopeKind, TaskFilter, WorkLens, WorkScope};
pub use session::Session;
pub use storage::{FileStorage, Storage};
pub use transport::Reply;

/// Con qué servidor habla el core y cómo se presenta.
#[derive(Debug, Clone)]
pub struct Config {
    /// Sin barra final: `https://cac.guz-studio.dev`.
    pub base_url: String,
    /// Qué cliente es, tal como lo espera el login (`desktop`, `web`). El
    /// servidor decide con eso qué rutas le abre.
    pub client: Option<String>,
    /// El plazo de cada petición.
    pub request_timeout: Duration,
}

impl Config {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: None,
            request_timeout: Duration::from_secs(12),
        }
    }
}

/// El core. Uno por proceso; clonarlo es barato y todos los clones son el
/// mismo.
#[derive(Clone)]
pub struct Core {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) config: Config,
    pub(crate) http: reqwest::Client,
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) listener: Arc<dyn Listener>,
    pub(crate) held: Mutex<Option<session::Held>>,
    /// El turno para renovar la sesión: ver `Core::refresh`.
    pub(crate) refreshing: tokio::sync::Mutex<()>,
    pub(crate) my_work: Mutex<MyWorkState>,
    /// Cuántas recargas de «Mi trabajo» se han pedido: la que espera su
    /// respiro sólo carga si sigue siendo la última.
    pub(crate) my_work_reloads: std::sync::atomic::AtomicU64,
}

impl Core {
    /// Arranca con la sesión que hubiera guardada.
    pub fn new(
        config: Config,
        storage: Arc<dyn Storage>,
        listener: Arc<dyn Listener>,
    ) -> Result<Self, ApiError> {
        let core = Self {
            inner: Arc::new(Inner {
                config,
                http: transport::new_client()?,
                storage,
                listener,
                held: Mutex::new(None),
                refreshing: tokio::sync::Mutex::new(()),
                my_work: Mutex::new(MyWorkState::default()),
                my_work_reloads: std::sync::atomic::AtomicU64::new(0),
            }),
        };
        let held = core.load_held();
        *core.inner.held.lock().expect("held lock") = held;
        core.my_work().restore_preferences();
        Ok(core)
    }
}

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
