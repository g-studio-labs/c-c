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

#[path = "../../core/tests/support/gated.rs"]
mod gated;
/// Retiene una respuesta hasta que la prueba la suelta (`gate.hold(…)` al
/// montarla, `gate.open()` para soltarla), sin parar al servidor: lo demás
/// sigue contestando.
pub use gated::Gate;

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
    /// Por donde habla la app: deja pasar todo y retiene lo que una prueba
    /// ate a una [`Gate`].
    proxy: gated::GatedProxy,
}

impl FakeServer {
    pub fn start() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let server = runtime.block_on(MockServer::start());
        let proxy = gated::GatedProxy::start(server.uri());
        // Como el servidor de verdad: un stream de eventos abierto que no dice
        // nada. Sin él, el stream caía en cada intento y a los 45 s la app
        // avisaba del corte con una franja encima de la pantalla: una prueba
        // que en una máquina lenta pasaba de ese minuto veía moverse todo lo
        // de debajo. Con menos prioridad que la de cualquier prueba, que
        // monta el suyo cuando quiere eventos (o un corte).
        let quiet = Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/v1/events"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(Vec::new(), "text/event-stream").set_delay(Duration::from_secs(3600)))
            .with_priority(10);
        runtime.block_on(quiet.mount(&server));
        Self { runtime, server, proxy }
    }

    pub fn mount(&self, mock: Mock) {
        self.runtime.block_on(mock.mount(&self.server));
    }

    /// Lo que llegó a pedirse, en orden.
    pub fn requests(&self) -> Vec<Request> {
        self.runtime.block_on(self.server.received_requests()).unwrap_or_default()
    }

    /// El puerto donde escucha: un servidor de la lista que apunte aquí
    /// tiene aquí también su agente.
    pub fn port(&self) -> u16 {
        self.proxy.port()
    }

    /// Cuántas peticiones llegaron a esa ruta.
    pub fn hits(&self, path: &str) -> usize {
        self.requests().iter().filter(|request| request.url.path() == path).count()
    }

    /// El core de la app, hablando con este servidor, en castellano.
    pub fn install(&self, cx: &mut TestAppContext) {
        self.install_with(cx, |_| {});
    }

    /// Lo mismo, retocando la configuración (las esperas del stream).
    pub fn install_with(&self, cx: &mut TestAppContext, edit: impl FnOnce(&mut Config)) {
        let mut config = Config::new(self.proxy.uri());
        config.client = Some("desktop".into());
        config.stream_backoff = Duration::from_millis(20);
        edit(&mut config);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply(gpui_kit::component::ThemeMode::Dark, cx);
            cx.set_global(SystemLanguage(Some("es_MX.UTF-8".into())));
            // Como `main.rs`: sin quién es la app, el sistema no enseña sus
            // avisos (y la plataforma de pruebas tampoco).
            cx.set_app_identity(crate::identity::APP_ID, crate::identity::APP_NAME);
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

/// Deja a Ana dentro, sin pasar por el login.
pub fn sign_in(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let ana = serde_json::from_value(serde_json::json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" }));
        Backend::core(cx).sign_in(ana.unwrap(), "access".into(), "refresh".into()).unwrap();
    });
}

/// Una respuesta buena del servidor con ese `data`.
pub fn ok(data: serde_json::Value) -> wiremock::ResponseTemplate {
    wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": data }))
}

/// Una tarea como la manda el servidor: la del contrato del core, retocada.
pub fn contract_task(id: &str, space: &str, edit: impl FnOnce(&mut serde_json::Value)) -> serde_json::Value {
    let mut task: serde_json::Value =
        serde_json::from_str(include_str!("../../core/tests/fixtures/open_task.json")).unwrap();
    task["id"] = serde_json::json!(id);
    task["spaceId"] = serde_json::json!(space);
    task["spaceName"] = serde_json::json!(format!("Espacio {space}"));
    task["listId"] = serde_json::json!(format!("l-{space}"));
    edit(&mut task);
    task
}

/// Un servidor con una organización, dos espacios (`a` y `b`, en ese orden)
/// y las tareas que se le den.
pub fn server_with_tasks(tasks: Vec<serde_json::Value>) -> FakeServer {
    use wiremock::matchers::{method, path};
    let server = FakeServer::start();
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    let org = serde_json::json!({
        "id": "o1", "name": "G Studio", "slug": "g", "role": "member", "memberCount": 2,
        "createdAt": "2026-06-09T12:00:00Z", "timezone": "",
    });
    let space = |id: &str| serde_json::json!({
        "id": id, "orgId": "o1", "name": format!("Espacio {id}"), "color": "#888", "folders": [],
        "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 1, "taskCount": 1 }], "people": [],
    });
    server.mount(get("/api/v1/organizations/").respond_with(ok(serde_json::json!([org]))));
    server.mount(get("/api/v1/task-spaces/").respond_with(ok(serde_json::json!([space("a"), space("b")]))));
    server.mount(get("/api/v1/docs/").respond_with(ok(serde_json::json!({}))));
    server.mount(get("/api/v1/tasks/").respond_with(ok(serde_json::Value::Array(tasks))));
    server
}

impl FakeServer {
    /// Las consultas que llegaron a esa ruta, en orden.
    pub fn queries(&self, path: &str) -> Vec<String> {
        self.requests()
            .iter()
            .filter(|request| request.url.path() == path)
            .map(|request| request.url.query().unwrap_or("").to_string())
            .collect()
    }
}

