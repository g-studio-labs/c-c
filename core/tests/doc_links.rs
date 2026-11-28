//! Seguir los enlaces de un documento, contra un servidor de mentira con dos
//! espacios documentados y una tarea.
//!
//! Cada prueba dice la rotura que atrapa, comprobada rompiendo el código
//! (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{Config, Core, DocAsk, DocJump, DocLeave, NoListener, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

const TASK: &str = "0d6bf322-2267-4259-9b81-337575aca792";

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn missing() -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_json(json!({ "success": false, "error": "not-found", "message": "not found" }))
}

fn space(id: &str) -> Value {
    json!({
        "id": id, "orgId": "o1", "name": format!("Espacio {id}"), "color": "#888", "people": [], "folders": [],
        "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 1, "taskCount": 2 }],
    })
}

fn doc(overview: &str, runbook: &str) -> Value {
    json!({
        "doc": null, "orgId": "o1", "decisions": [], "attachments": [],
        "tabs": [
            { "key": "overview", "body": overview, "bodyHash": "h1" },
            { "key": "runbook", "body": runbook, "bodyHash": "h1", "sections": [{ "id": "s1", "heading": "Desplegar", "level": 2, "index": 0 }] },
        ],
    })
}

fn page(id: &str, title: &str) -> Value {
    json!({ "page": { "id": id, "title": title, "body": "## Capas\n\nTres.", "bodyHash": "h1" }, "breadcrumb": [], "children": [], "referencedFrom": [] })
}

struct Rig {
    server: MockServer,
    core: Core,
}

impl Rig {
    async fn reads(&self, route: &str) -> usize {
        self.server.received_requests().await.unwrap().iter().filter(|request| request.url.path() == route).count()
    }

    async fn requests(&self) -> usize {
        self.server.received_requests().await.unwrap().len()
    }

    fn target(&self) -> Option<String> {
        self.core.docs().state().target.map(|target| target.id)
    }

    fn failed(&self) -> Option<String> {
        self.core.docs().state().link_failed.map(|failure| failure.href)
    }

    /// Abre el editor de lo que se lee y escribe eso en la caja.
    fn write(&self, body: &str) {
        assert!(self.core.docs().start_edit(), "no dejó empezar a escribir");
        self.core.docs().draft_body(body);
    }
}

/// Ana dentro, con la documentación del espacio `a` abierta. `a` tiene una
/// página (`p1`) y `b` otra (`q1`); la lista `l-a` no tiene nada escrito.
/// Hay una tarea, que fuera se llama `command-and-control-12`.
async fn rig() -> Rig {
    let server = MockServer::start().await;
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    let org = json!({ "id": "o1", "name": "Org", "slug": "o1", "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
    get("/api/v1/organizations/").respond_with(ok(json!([org]))).mount(&server).await;
    get("/api/v1/task-spaces/").respond_with(ok(json!([space("a"), space("b")]))).mount(&server).await;
    get("/api/v1/docs/").respond_with(ok(json!({}))).mount(&server).await;
    get("/api/v1/tasks/").respond_with(ok(json!([]))).mount(&server).await;

    get("/api/v1/docs/space/a").respond_with(ok(doc("## Qué es\n\nEl espacio a.", "## Desplegar\n\nCon cuidado."))).mount(&server).await;
    get("/api/v1/docs/space/b").respond_with(ok(doc("El espacio b.", "## Desplegar\n\nEl de b."))).mount(&server).await;
    get("/api/v1/docs/list/l-a").respond_with(ok(doc("", ""))).mount(&server).await;
    get("/api/v1/docs/space/a/pages").respond_with(ok(json!([{ "id": "p1", "title": "Arquitectura", "hasBody": true }]))).mount(&server).await;
    get("/api/v1/docs/space/b/pages").respond_with(ok(json!([{ "id": "q1", "title": "Red", "hasBody": true }]))).mount(&server).await;
    get("/api/v1/docs/list/l-a/pages").respond_with(ok(json!([]))).mount(&server).await;
    get("/api/v1/docs/space/a/pages/p1").respond_with(ok(page("p1", "Arquitectura"))).mount(&server).await;
    get("/api/v1/docs/space/b/pages/q1").respond_with(ok(page("q1", "Red"))).mount(&server).await;
    Mock::given(method("GET")).and(path_regex("^/api/v1/docs/[a-z]+/[a-z-]+/pages/")).respond_with(missing()).mount(&server).await;

    let mut detail: Value = serde_json::from_str(include_str!("fixtures/task_detail.json")).unwrap();
    detail["task"]["id"] = json!(TASK);
    get(&format!("/api/v1/tasks/{TASK}")).respond_with(ok(detail.clone())).mount(&server).await;
    get("/api/v1/tasks/by-ref/command-and-control-12").respond_with(ok(detail)).mount(&server).await;
    Mock::given(method("GET")).and(path_regex("^/api/v1/tasks/by-ref/")).respond_with(missing()).mount(&server).await;

    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.orgs().load().await;
    core.tree().load("o1").await;
    core.docs().open("a").await;
    Rig { server, core }
}

/// Un enlace a una tarea la abre: por su id tal cual, y por su nombre de
/// fuera preguntándole al servidor cuál es. La que no existe se dice.
///
/// Roturas que atrapa: un enlace a una tarea que no abre nada (lo que
/// pasaba: se le daba al navegador); pedir `tasks/command-and-control-12`
/// como si fuera un id; un nombre que no es de ninguna tarea dejando una
/// página de tarea en blanco, o no diciendo nada.
#[tokio::test]
async fn a_link_to_a_task_opens_its_page() {
    let rig = rig().await;
    let docs = rig.core.docs();

    docs.follow_link(&format!("/tasks?task={TASK}")).await;
    assert_eq!(rig.core.tasks().state().task_id.as_deref(), Some(TASK));
    assert_eq!(rig.reads("/api/v1/tasks/by-ref/command-and-control-12").await, 0);
    assert_eq!(rig.target().as_deref(), Some("a"), "el documento sigue debajo");
    rig.core.tasks().close();

    docs.follow_link("/tasks?task=command-and-control-12").await;
    assert_eq!(rig.core.tasks().state().task_id.as_deref(), Some(TASK));
    assert_eq!(rig.reads("/api/v1/tasks/command-and-control-12").await, 0, "pidió el nombre como si fuera un id");
    assert_eq!(rig.failed(), None);
    rig.core.tasks().close();

    docs.follow_link("/tasks?task=no-existe-7").await;
    assert_eq!(rig.core.tasks().state().task_id, None);
    assert_eq!(rig.failed().as_deref(), Some("/tasks?task=no-existe-7"));
}

/// Abrir una tarea desde lo que se escribe no pregunta ni toca el borrador:
/// la tarea se abre encima y el editor sigue debajo.
///
/// Rotura que atrapa: preguntar «¿descartar?» —o descartar— por mirar una
/// tarea citada en el texto que se está escribiendo.
#[tokio::test]
async fn opening_a_task_from_a_draft_keeps_the_draft() {
    let rig = rig().await;
    rig.write("a medias");
    rig.core.docs().follow_link(&format!("/tasks?task={TASK}")).await;
    assert_eq!(rig.core.tasks().state().task_id.as_deref(), Some(TASK));
    let edit = rig.core.docs().state().edit.expect("el editor se cerró");
    assert_eq!((edit.body.as_str(), edit.asking), ("a medias", None));
}

/// Un enlace a este mismo documento se abre aquí: su página; su pestaña,
/// volviendo de la página; y la sección que diga queda pedida para saltar.
///
/// Roturas que atrapa: un enlace a «Runbook» que se queda en «Resumen»;
/// seguir en la página al pedir la portada; perder la sección; volver a
/// pedir el documento entero para moverse dentro de él.
#[tokio::test]
async fn a_link_into_this_document_moves_inside_it() {
    let rig = rig().await;
    let docs = rig.core.docs();
    let loads = rig.reads("/api/v1/docs/space/a").await;

    docs.follow_link("/tasks?doc=space:a&page=p1").await;
    let state = docs.state();
    assert_eq!(state.page_id.as_deref(), Some("p1"));
    assert_eq!(state.page.map(|view| view.page.title), Some("Arquitectura".to_string()));

    docs.follow_link("/tasks?doc=space:a&tab=runbook&section=s1").await;
    let state = docs.state();
    assert_eq!((state.page_id, state.tab.as_str()), (None, "runbook"));
    assert_eq!(state.jump, Some(DocJump::Section { id: "s1".into() }));
    assert_eq!(rig.reads("/api/v1/docs/space/a").await, loads, "volvió a pedir el documento");

    // Sin pestaña, la portada donde estuviera.
    docs.follow_link("/tasks?doc=space:a&page=p1").await;
    docs.follow_link("/tasks?doc=space:a").await;
    let state = docs.state();
    assert_eq!((state.page_id, state.tab.as_str(), state.jump), (None, "runbook", None));
    assert_eq!(rig.failed(), None);
}

/// Un enlace al documento de otro nodo lo abre, ya en su pestaña o en su
/// página.
///
/// Roturas que atrapa: un enlace a otro documento que no hace nada (lo que
/// pasaba); abrirlo en «Resumen» diga lo que diga el enlace; abrir su
/// portada cuando pedía una página.
#[tokio::test]
async fn a_link_to_another_document_opens_it_where_it_points() {
    let rig = rig().await;
    let docs = rig.core.docs();

    docs.follow_link("/tasks?doc=space:b&tab=runbook&section=s1").await;
    let state = docs.state();
    assert_eq!(rig.target().as_deref(), Some("b"));
    assert_eq!((state.tab.as_str(), state.body()), ("runbook", "## Desplegar\n\nEl de b."));
    assert_eq!(state.jump, Some(DocJump::Section { id: "s1".into() }));

    docs.follow_link("/tasks?doc=space:a&page=p1").await;
    let state = docs.state();
    assert_eq!((rig.target().as_deref(), state.page_id.as_deref()), (Some("a"), Some("p1")));
    assert_eq!(state.page.map(|view| view.page.title), Some("Arquitectura".to_string()));
    assert_eq!(state.jump, None, "saltó a la sección del enlace anterior");

    docs.follow_link("/tasks?doc=list:l-a").await;
    assert_eq!(rig.target().as_deref(), Some("l-a"));
    assert_eq!(rig.failed(), None);
}

/// Lo que apunta a algo que no está se dice, y no mueve nada: un nodo que
/// no es de este árbol, uno de otra clase que la que dice el enlace, una
/// página que el documento ya no tiene, y un enlace que no se entiende.
///
/// Roturas que atrapa: un clic que no hace nada ni dice nada; abrir el
/// documento de un nodo como si fuera de otra clase (el servidor contesta
/// con uno vacío y se lee «no hay nada escrito»); quedarse en la portada
/// de otro documento sin saber por qué.
#[tokio::test]
async fn what_points_at_nothing_is_said() {
    let rig = rig().await;
    let docs = rig.core.docs();
    let before = rig.requests().await;

    for href in ["/tasks?doc=space:zz", "/tasks?doc=folder:b", "/notes/n1", "cac:recording/11111111-1111-4111-8111-111111111111"] {
        docs.follow_link(href).await;
        assert_eq!(rig.failed().as_deref(), Some(href));
        assert_eq!(rig.target().as_deref(), Some("a"), "{href} movió algo");
    }
    assert_eq!(rig.requests().await, before, "pidió algo al servidor por un enlace que no lleva a nada");

    // El siguiente que sí se abre quita el aviso.
    docs.follow_link("/tasks?doc=space:a&tab=runbook").await;
    assert_eq!(rig.failed(), None);

    // Cada fallo es otro, aunque sea del mismo enlace.
    docs.follow_link("/notes/n1").await;
    let first = docs.state().link_failed.unwrap().serial;
    docs.follow_link("/notes/n1").await;
    assert_ne!(docs.state().link_failed.unwrap().serial, first);
    docs.dismiss_link_failure();
    assert_eq!(rig.failed(), None);

    docs.follow_link("/tasks?doc=space:b&page=ya-no-esta").await;
    assert_eq!(rig.failed().as_deref(), Some("/tasks?doc=space:b&page=ya-no-esta"));
    assert_eq!(docs.state().page_id, None);
}

/// Lo de fuera y una mención no son cosa del core: ni abre nada, ni dice
/// que falló, ni pide nada.
///
/// Rotura que atrapa: «No se pudo abrir el enlace» encima de una página
/// que el navegador sí abrió, o al pulsar el nombre de alguien.
#[tokio::test]
async fn what_is_outside_is_left_to_the_platform() {
    let rig = rig().await;
    let before = rig.requests().await;
    for href in ["https://example.com/a", "mailto:ana@example.com", "cac:user/11111111-1111-4111-8111-111111111111"] {
        rig.core.docs().follow_link(href).await;
    }
    assert_eq!((rig.failed(), rig.requests().await), (None, before));
    assert_eq!(rig.target().as_deref(), Some("a"));
}

/// Un ancla pide saltar a ese título de lo que se lee; mientras se escribe
/// no hay texto leído al que saltar.
///
/// Roturas que atrapa: un `#titulo` tomado por un enlace roto; un salto
/// pendiente que se cumple al cerrar el editor, mucho después del clic.
#[tokio::test]
async fn an_anchor_asks_to_jump_to_that_heading() {
    let rig = rig().await;
    let docs = rig.core.docs();
    docs.follow_link("#que-es").await;
    assert_eq!(docs.state().jump, Some(DocJump::Anchor { slug: "que-es".into() }));
    assert_eq!(rig.failed(), None);
    docs.jump_done();
    assert_eq!(docs.state().jump, None);

    // Irse a otra parte deja el salto sin efecto: era de lo que se leía.
    let leave: [(&str, Box<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>>); 3] = [
        ("otra pestaña", Box::new(|| { let docs = rig.core.docs(); Box::pin(async move { docs.show_tab("runbook") }) })),
        ("una página", Box::new(|| { let docs = rig.core.docs(); Box::pin(async move { docs.open_page("p1").await }) })),
        ("la portada", Box::new(|| { let docs = rig.core.docs(); Box::pin(async move { docs.close_page() }) })),
    ];
    for (what, go) in leave {
        docs.follow_link("#que-es").await;
        assert!(docs.state().jump.is_some());
        go().await;
        assert_eq!(docs.state().jump, None, "el salto sobrevivió a irse a {what}");
    }

    rig.write("a medias");
    docs.follow_link("#que-es").await;
    assert_eq!(docs.state().jump, None);
    assert_eq!(docs.state().edit.map(|edit| edit.asking), Some(None));
}

/// Salir de lo que se escribe por un enlace pregunta antes: con un no, todo
/// sigue como estaba; con un sí, se va a donde llevaba el enlace entero (su
/// nodo, su pestaña). Y un enlace que no lleva a nada no pregunta.
///
/// Roturas que atrapa: perder el texto por pulsar un enlace de la vista
/// previa; preguntar y luego abrir sólo el nodo, en «Resumen»; preguntar
/// «¿descartar?» para acabar diciendo que el enlace no se pudo abrir.
#[tokio::test]
async fn leaving_a_draft_through_a_link_asks_first() {
    let rig = rig().await;
    let docs = rig.core.docs();
    rig.write("a medias");
    let href = "/tasks?doc=space:b&tab=runbook";

    docs.follow_link("/tasks?doc=space:zz").await;
    assert_eq!(docs.state().edit.map(|edit| edit.asking), Some(None), "preguntó por un enlace que no lleva a nada");

    docs.follow_link(href).await;
    let asking = docs.state().edit.and_then(|edit| edit.asking);
    assert_eq!(asking, Some(DocAsk::Leave { to: DocLeave::Link { href: href.into() } }));
    assert_eq!(rig.target().as_deref(), Some("a"), "se fue sin esperar la respuesta");

    docs.answer(false).await;
    let edit = docs.state().edit.expect("un no cerró el editor");
    assert_eq!((edit.body.as_str(), edit.asking, rig.target().as_deref()), ("a medias", None, Some("a")));

    docs.follow_link("/tasks?doc=space:a&page=p1").await;
    assert!(docs.state().edit.is_some_and(|edit| edit.asking.is_some()), "una página de este documento también saca del editor");
    assert_eq!(docs.state().page_id, None);
    docs.answer(false).await;

    docs.follow_link(href).await;
    docs.answer(true).await;
    let state = docs.state();
    assert_eq!((rig.target().as_deref(), state.tab.as_str(), state.edit), (Some("b"), "runbook", None));
}

/// Rotura que atrapa: abrir un documento estando en Plataforma (desde la
/// campana) y que no se vea, porque Plataforma se pinta por delante.
#[tokio::test]
async fn opening_a_doc_leaves_platform() {
    let rig = rig().await;
    rig.core.platform().enter();
    assert!(rig.core.platform().state().on_screen);
    rig.core.docs().open("b").await;
    assert_eq!(rig.target().as_deref(), Some("b"));
    assert!(!rig.core.platform().state().on_screen);
}

/// Un enlace a una página o pestaña de un documento, dado por la bandeja («Por
/// atender», la campana), abre esa página o esa pestaña y no la portada.
///
/// Roturas que atrapa: la bandeja tirando `page`, `tab` o `section` y abriendo
/// la portada en «Resumen» (lo que pasaba); callar cuando la página ya no
/// existe.
#[tokio::test]
async fn an_inbox_link_to_a_doc_page_opens_the_page() {
    let rig = rig().await;
    let inbox = rig.core.inbox();

    assert!(inbox.open("/tasks?doc=space:b&page=q1", "o1").await);
    let state = rig.core.docs().state();
    assert_eq!((rig.target().as_deref(), state.page_id.as_deref()), (Some("b"), Some("q1")));

    assert!(inbox.open("/tasks?doc=space:a&tab=runbook&section=s1", "o1").await);
    let state = rig.core.docs().state();
    assert_eq!((rig.target().as_deref(), state.page_id, state.tab.as_str()), (Some("a"), None, "runbook"));
    assert_eq!(state.jump, Some(DocJump::Section { id: "s1".into() }));

    // La página borrada se dice, en vez de dejar la portada sin explicación.
    inbox.open("/tasks?doc=space:b&page=no-existe", "o1").await;
    assert_eq!(rig.failed().as_deref(), Some("/tasks?doc=space:b&page=no-existe"));
}

/// Un enlace a un documento con una página (o una sección) que no es un id
/// simple no se sigue: ni por el cuerpo de un documento ni por la bandeja.
/// `doc_link` descodifica, y la página acaba pegada a una ruta de la API:
/// `page=..%2F..%2Fx` pediría otra cosa con la sesión de quien pulsa.
///
/// Rotura que atrapa: quitar el filtro de `Docs::follow_link` (hace un GET a
/// `/api/v1/docs/space/b/pages/../../x`, o abre `b` por una sección
/// imposible) o que la bandeja lo siga por otro camino sin filtro.
#[tokio::test]
async fn a_doc_link_with_something_that_is_not_an_id_asks_the_server_nothing() {
    let rig = rig().await;
    let docs = rig.core.docs();
    let inbox = rig.core.inbox();
    let before = rig.requests().await;

    for href in [
        "/tasks?doc=space:b&page=..%2F..%2Fx",
        "/tasks?doc=space:b&page=a/b",
        "/tasks?doc=space:b&page=%2e%2e",
        "/tasks?doc=space:b&section=..%2Fx",
    ] {
        docs.follow_link(href).await;
        assert_eq!(rig.failed().as_deref(), Some(href), "{href} no se dijo desde un documento");
        assert_eq!(rig.target().as_deref(), Some("a"), "{href} movió algo desde un documento");

        docs.dismiss_link_failure();
        assert!(!inbox.open(href, "o1").await, "{href}: la bandeja dice que lo abrió");
        assert_eq!(rig.failed().as_deref(), Some(href), "{href} no se dijo desde la bandeja");
        assert_eq!(rig.target().as_deref(), Some("a"), "{href} movió algo desde la bandeja");
    }
    assert_eq!(rig.requests().await, before, "pidió algo al servidor por un enlace con algo que no es un id");
}

/// La bandeja lee el nodo con el mismo traductor que el resto: descodificado
/// y sin el ancla, así que lo que abre es lo que dice que abrió.
///
/// Rotura que atrapa: un segundo parser en la bandeja que compara `b#x` con
/// `b` y contesta `false` tras abrir el documento, o un `open` que contesta
/// `true` sin haber llegado (a otro nodo, o al mismo con una página que no
/// se pudo abrir).
#[tokio::test]
async fn the_inbox_says_truly_whether_it_opened_a_doc() {
    let rig = rig().await;
    let inbox = rig.core.inbox();

    assert!(inbox.open("/tasks?doc=space:b#x", "o1").await);
    assert_eq!(rig.target().as_deref(), Some("b"));

    assert!(!inbox.open("/tasks?doc=space:zz", "o1").await, "un nodo que no existe no se abrió");
    assert_eq!(rig.target().as_deref(), Some("b"));

    // Ya en `b`: una página que no es un id no se abrió, aunque se siga en `b`.
    assert!(!inbox.open("/tasks?doc=space:b&page=..%2Fx", "o1").await);
    assert_eq!(rig.target().as_deref(), Some("b"));

    // Ni una que ya no está: se llega al documento, pero no a lo que se pidió.
    assert!(!inbox.open("/tasks?doc=space:a&page=ya-no-esta", "o1").await);
    assert_eq!(rig.target().as_deref(), Some("a"));
}
