//! Lo que comparten las pruebas de las pantallas: un servidor de mentira, un
//! almacén en memoria y cómo esperar a lo que viene de otro hilo.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Config, Storage};
use gpui_kit::TestAppContext;
use wiremock::{Mock, MockServer, Request};

use crate::backend::Backend;
use crate::lang::SystemLanguage;

#[derive(Default)]
pub struct MemoryStorage(Mutex<HashMap<String, String>>);

impl Storage for MemoryStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn remove(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}

/// Un servidor de mentira, con su propio runtime: las pruebas de GPUI no
/// corren sobre tokio.
pub struct FakeServer {
    runtime: tokio::runtime::Runtime,
    server: MockServer,
}

impl FakeServer {
    pub fn start() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let server = runtime.block_on(MockServer::start());
        Self { runtime, server }
    }

    pub fn mount(&self, mock: Mock) {
        self.runtime.block_on(mock.mount(&self.server));
    }

    /// Lo que llegó a pedirse, en orden.
    pub fn requests(&self) -> Vec<Request> {
        self.runtime.block_on(self.server.received_requests()).unwrap_or_default()
    }

    /// Cuántas peticiones llegaron a esa ruta.
    pub fn hits(&self, path: &str) -> usize {
        self.requests().iter().filter(|request| request.url.path() == path).count()
    }

    /// El core de la app, hablando con este servidor, en castellano.
    pub fn install(&self, cx: &mut TestAppContext) {
        let mut config = Config::new(self.server.uri());
        config.client = Some("desktop".into());
        config.stream_backoff = Duration::from_millis(20);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply(gpui_kit::component::ThemeMode::Dark, cx);
            cx.set_global(SystemLanguage(Some("es_MX.UTF-8".into())));
            Backend::install(config, Arc::new(MemoryStorage::default()), cx).unwrap();
        });
    }
}

/// Espera a algo que viene de otro hilo (el runtime del core).
pub fn wait_until(cx: &mut TestAppContext, what: &str, done: impl Fn(&mut TestAppContext) -> bool) {
    cx.executor().allow_parking();
    for _ in 0..300 {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no llegó a pasar: {what}");
}
