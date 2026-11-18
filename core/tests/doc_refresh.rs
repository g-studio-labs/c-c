//! El documento abierto se pone al día solo, sin pisar lo que se escribe.
//!
//! Contra un servidor de mentira que guarda como el de verdad (cada texto
//! con su versión) y al que «otra persona» le cambia cosas entre medias.
//!
//! Cada prueba dice la rotura que atrapa, comprobada rompiendo el código
//! (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Config, Core, NoListener, Storage};
use serde_json::{json, Value};
use wiremock::matchers::path_regex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

#[derive(Default)]
struct MemoryStorage(Mutex<HashMap<String, String>>);

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

/// Lo que el servidor de mentira tiene del espacio `a`: su resumen y su
/// página `p1`, cada uno con su versión.
struct World {
    overview: (String, u32, &'static str),
    page: (String, u32, &'static str),
    owner: &'static str,
    /// Cuánto tarda en contestar una lectura del documento.
    read_delay: Duration,
    /// Con esto, leer el documento falla.
    broken: bool,
    /// Cuánto tarda en contestar un guardado (que ya quedó hecho).
    write_delay: Duration,
}

impl World {
    fn doc(&self) -> Value {
        json!({
            "doc": { "id": "d1", "maintainerId": "u1", "maintainerName": self.owner, "stale": false },
            "orgId": "o1", "decisions": [], "attachments": [],
            "tabs": [{ "key": "overview", "body": self.overview.0, "bodyHash": format!("h{}", self.overview.1), "updatedByName": self.overview.2 }],
        })
    }

    fn page(&self) -> Value {
        json!({ "id": "p1", "title": "Arquitectura", "body": self.page.0, "bodyHash": format!("h{}", self.page.1), "updatedByName": self.page.2 })
    }
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn refused(status: u16, code: &str, data: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({ "success": false, "error": code, "message": code, "data": data }))
}

fn answer(world: &Mutex<World>, request: &Request) -> ResponseTemplate {
    let mut world = world.lock().unwrap();
    let route = request.url.path().trim_start_matches("/api/v1/docs/space/a").to_string();
    let parts: Vec<&str> = route.split('/').filter(|part| !part.is_empty()).collect();
    let sent: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    match (request.method.as_str(), parts.as_slice()) {
        ("GET", []) if world.broken => refused(500, "boom", Value::Null),
        ("GET", []) => ok(world.doc()).set_delay(world.read_delay),
        ("GET", ["pages"]) => ok(json!([{ "id": "p1", "title": "Arquitectura", "hasBody": true }])),
        ("GET", ["pages", "p1"]) => ok(json!({ "page": world.page(), "breadcrumb": [], "children": [], "referencedFrom": [] })),
        ("PUT", ["tabs", "overview"]) => {
            if sent["baseHash"].as_str() != Some(format!("h{}", world.overview.1).as_str()) {
                return refused(409, "doc-conflict", world.doc());
            }
            world.overview = (sent["body"].as_str().unwrap_or_default().into(), world.overview.1 + 1, "Ana Ruiz");
            ok(world.doc()).set_delay(world.write_delay)
        }
        _ => refused(404, "not-found", Value::Null),
    }
}

struct Rig {
    server: MockServer,
    core: Core,
    world: Arc<Mutex<World>>,
}

impl Rig {
    async fn reads(&self) -> usize {
        self.server.received_requests().await.unwrap().iter().filter(|request| request.method.as_str() == "GET" && request.url.path() == "/api/v1/docs/space/a").count()
    }

    fn luis_saves_the_overview(&self, body: &str) {
        let mut world = self.world.lock().unwrap();
        world.overview = (body.into(), world.overview.1 + 1, "Luis Gil");
    }

    fn body(&self) -> String {
        self.core.docs().state().body().to_string()
    }
}

/// Ana dentro, con la documentación del espacio `a` abierta.
async fn rig() -> Rig {
    let server = MockServer::start().await;
    let org = json!({ "id": "o1", "name": "Org", "slug": "o1", "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
    let space = json!({ "id": "a", "orgId": "o1", "name": "Espacio", "color": "#888", "people": [], "folders": [], "lists": [] });
    for (route, data) in [("^/api/v1/organizations/$", json!([org])), ("^/api/v1/task-spaces/$", json!([space])), ("^/api/v1/docs/$", json!({})), ("^/api/v1/tasks/$", json!([]))] {
        Mock::given(path_regex(route)).respond_with(ok(data)).mount(&server).await;
    }
    let world = Arc::new(Mutex::new(World {
        overview: ("Lo de siempre.".into(), 1, "Ana Ruiz"),
        page: ("## Capas\n\nTres.".into(), 1, "Ana Ruiz"),
        owner: "Ana Ruiz",
        read_delay: Duration::ZERO,
        broken: false,
        write_delay: Duration::ZERO,
    }));
    let seen = world.clone();
    Mock::given(path_regex("^/api/v1/docs/space/a(/|$)")).respond_with(move |request: &Request| answer(&seen, request)).mount(&server).await;

    let mut config = Config::new(server.uri());
    config.doc_fresh_for = Duration::from_millis(80);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.orgs().load().await;
    core.tree().load("o1").await;
    core.docs().open("a").await;
    Rig { server, core, world }
}

/// Ponerse al día trae lo que hay ahora —el texto, la cabecera, la página
/// abierta— sin moverse de donde se lee; y si no se puede, deja lo que había
/// sin decir que se rompió.
///
/// Roturas que atrapa: seguir leyendo lo de antes después de que otro
/// guardara; volver a «Resumen» o a la portada al ponerse al día; un «no se
/// pudo cargar» encima de un documento que se estaba leyendo bien.
#[tokio::test]
async fn refreshing_brings_what_there_is_now_and_stays_where_it_was() {
    let rig = rig().await;
    let docs = rig.core.docs();
    docs.open_page("p1").await;

    rig.luis_saves_the_overview("Lo de Luis.");
    {
        let mut world = rig.world.lock().unwrap();
        world.owner = "Luis Gil";
        world.page = ("## Capas\n\nCuatro.".into(), 2, "Luis Gil");
    }
    docs.refresh().await;
    let state = docs.state();
    assert_eq!(state.page_id.as_deref(), Some("p1"), "ponerse al día cerró la página");
    assert_eq!(state.body(), "## Capas\n\nCuatro.");
    assert_eq!(state.info.and_then(|info| info.maintainer_name).as_deref(), Some("Luis Gil"));
    docs.close_page();
    assert_eq!(rig.body(), "Lo de Luis.");

    rig.world.lock().unwrap().broken = true;
    rig.luis_saves_the_overview("No se llega a leer.");
    docs.refresh().await;
    let state = docs.state();
    assert_eq!((state.body(), state.error.clone(), state.loading), ("Lo de Luis.", None, false));

    // Y uno que no se pudo abrir deja de decirlo en cuanto llega.
    rig.core.docs().close();
    docs.open("a").await;
    assert!(docs.state().error.is_some());
    rig.world.lock().unwrap().broken = false;
    docs.refresh().await;
    let state = docs.state();
    assert_eq!((state.body(), state.error.clone()), ("No se llega a leer.", None));
}

/// Si el documento cambia debajo de lo que se escribe, se dice con el aviso
/// de choque y el texto no se toca; lo que hay guardado queda a mano para
/// mirarlo, y guardar encima sale de esa versión.
///
/// Roturas que atrapa: ponerse al día encima del borrador (se pierde lo
/// escrito); no avisar, y enterarse del choque al guardar, mucho después;
/// un «guardar encima» que vuelve a chocar porque sale de la versión vieja.
#[tokio::test]
async fn a_change_underneath_a_draft_is_said_and_the_text_is_not_touched() {
    let rig = rig().await;
    let docs = rig.core.docs();
    assert!(docs.start_edit());
    docs.draft_body("Lo mío, a medias.");

    // Sin cambios debajo, ponerse al día no dice nada.
    docs.refresh().await;
    let edit = docs.state().edit.unwrap();
    assert_eq!((edit.conflict, edit.body.as_str()), (None, "Lo mío, a medias."));

    rig.luis_saves_the_overview("Lo de Luis.");
    docs.refresh().await;
    let edit = docs.state().edit.expect("ponerse al día cerró el editor");
    assert_eq!(edit.body, "Lo mío, a medias.", "se tocó lo escrito");
    let conflict = edit.conflict.clone().expect("no se avisó de que cambió debajo");
    assert_eq!((conflict.body.as_str(), conflict.hash.as_deref(), conflict.by_name.as_deref()), ("Lo de Luis.", Some("h2"), Some("Luis Gil")));
    assert!(!edit.can_save(), "con el choque a la vista se puede guardar sin decidir");

    // Si vuelve a cambiar, el aviso enseña lo último.
    rig.luis_saves_the_overview("Lo de Luis, otra vez.");
    docs.refresh().await;
    assert_eq!(docs.state().edit.and_then(|edit| edit.conflict).map(|conflict| conflict.hash), Some(Some("h3".to_string())));

    docs.ask_overwrite();
    docs.answer(true).await;
    assert_eq!(docs.state().edit, None, "guardar encima volvió a chocar");
    assert_eq!(rig.world.lock().unwrap().overview.0, "Lo mío, a medias.");
}

/// Un editor abierto y sin tocar tampoco se cambia por debajo: se avisa
/// igual. Y la página que se escribe, como la pestaña.
///
/// Roturas que atrapa: cambiar el texto de la caja mientras alguien tiene el
/// cursor en ella; que el aviso sólo funcione en las pestañas.
#[tokio::test]
async fn an_untouched_editor_and_a_page_are_warned_too() {
    let rig = rig().await;
    let docs = rig.core.docs();
    assert!(docs.start_edit());
    rig.luis_saves_the_overview("Lo de Luis.");
    docs.refresh().await;
    let edit = docs.state().edit.unwrap();
    assert_eq!(edit.body, "Lo de siempre.");
    assert!(edit.conflict.is_some());
    docs.ask_load_theirs();
    docs.answer(true).await;
    assert_eq!(rig.body(), "Lo de Luis.");

    docs.open_page("p1").await;
    assert!(docs.start_edit());
    docs.draft_body("## Capas\n\nCinco.");
    rig.world.lock().unwrap().page = ("## Capas\n\nCuatro.".into(), 2, "Luis Gil");
    docs.refresh().await;
    let edit = docs.state().edit.unwrap();
    assert_eq!(edit.body, "## Capas\n\nCinco.");
    assert_eq!(edit.conflict.map(|conflict| (conflict.body, conflict.by_name)), Some(("## Capas\n\nCuatro.".to_string(), Some("Luis Gil".to_string()))));
}

/// Una lectura que se pidió antes de guardar y llega después no cuenta: ni
/// avisa de un choque que no hay, ni pone a la vista el texto de antes.
///
/// Rotura que atrapa: guardar, y un instante después ver el aviso de «alguien
/// guardó» con el texto viejo como si fuera de otro.
#[tokio::test]
async fn a_reading_that_lands_after_a_save_does_not_count() {
    let rig = rig().await;
    let docs = rig.core.docs();
    assert!(docs.start_edit());
    docs.draft_body("Lo mío.");

    rig.world.lock().unwrap().read_delay = Duration::from_millis(150);
    let slow = rig.core.clone();
    let reading = tokio::spawn(async move { slow.docs().refresh().await });
    tokio::time::sleep(Duration::from_millis(40)).await;
    rig.world.lock().unwrap().read_delay = Duration::ZERO;
    // Se sigue escribiendo mientras viaja el guardado, así el editor no se
    // cierra y la lectura tardía lo encuentra abierto.
    docs.save().await;
    assert!(docs.start_edit());
    docs.draft_body("Lo mío, y más.");
    reading.await.unwrap();

    let state = docs.state();
    let edit = state.edit.clone().expect("el editor se cerró");
    assert_eq!(edit.conflict, None, "avisó de un choque con su propio guardado");
    assert_eq!(state.tabs[0].body, "Lo mío.");
}

/// Mientras viaja un guardado propio, ponerse al día no lo toma por un
/// cambio de otro: lo que el servidor tiene ya es lo que se acaba de mandar.
///
/// Rotura que atrapa: guardar, seguir escribiendo, y que salga «alguien
/// guardó mientras escribías» con el texto propio como si fuera ajeno —y se
/// quede ahí, con «Guardar» apagado—.
#[tokio::test]
async fn my_own_save_in_flight_is_not_taken_for_someone_elses() {
    let rig = rig().await;
    let docs = rig.core.docs();
    assert!(docs.start_edit());
    docs.draft_body("Lo mío.");
    rig.world.lock().unwrap().write_delay = Duration::from_millis(150);
    let saver = rig.core.clone();
    let saving = tokio::spawn(async move { saver.docs().save().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(docs.state().edit.is_some_and(|edit| edit.saving), "el guardado ya no viaja");
    docs.draft_body("Lo mío, y más.");

    docs.refresh().await;
    assert_eq!(docs.state().edit.and_then(|edit| edit.conflict), None, "tomó su propio guardado por el de otro");
    saving.await.unwrap();
    let edit = docs.state().edit.expect("se seguía escribiendo");
    assert_eq!((edit.conflict, edit.base_hash.as_deref(), edit.body.as_str()), (None, Some("h2"), "Lo mío, y más."));
}

/// Volver a la ventana pone al día el documento, pero no si se acaba de
/// leer; y sin documento abierto no pide nada.
///
/// Roturas que atrapa: no enterarse de lo que un agente escribió mientras
/// se estaba en otra ventana; pedir el documento cada vez que la ventana
/// coge el foco, también recién abierto.
#[tokio::test]
async fn coming_back_refreshes_unless_it_was_just_read() {
    let rig = rig().await;
    let docs = rig.core.docs();
    let after_opening = rig.reads().await;

    docs.came_back().await;
    assert_eq!(rig.reads().await, after_opening, "recién leído, lo volvió a pedir");

    // Pasado el rato en que vale por reciente (80 ms en esta prueba).
    rig.luis_saves_the_overview("Lo de Luis.");
    tokio::time::sleep(Duration::from_millis(120)).await;
    docs.came_back().await;
    assert_eq!(rig.reads().await, after_opening + 1);
    assert_eq!(rig.body(), "Lo de Luis.");
    docs.came_back().await;
    assert_eq!(rig.reads().await, after_opening + 1, "ponerse al día no cuenta como haber leído");

    rig.core.docs().close();
    let before = rig.server.received_requests().await.unwrap().len();
    tokio::time::sleep(Duration::from_millis(120)).await;
    docs.came_back().await;
    docs.refresh().await;
    assert_eq!(rig.server.received_requests().await.unwrap().len(), before, "sin documento abierto pidió algo");
}
