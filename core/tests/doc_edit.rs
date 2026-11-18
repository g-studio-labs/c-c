//! Escribir la documentación, contra un servidor de mentira que guarda como
//! el de verdad: cada texto tiene su versión (`bodyHash`), y un guardado
//! que no trae la vigente se rechaza con 409 y lo que hay ahora dentro
//! (`domain.DocSaveConflicts`, `handler/doc.go`, `handler/doc_page.go`).
//!
//! Cada prueba dice la rotura que atrapa, comprobada rompiendo el código
//! (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Config, Core, DocAsk, DocEditWhat, DocLeave, Locale, NoListener, Storage, WorkLayout};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, path_regex};
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

/// Un texto guardado, con su versión y quién lo dejó así.
#[derive(Clone)]
struct Kept {
    title: String,
    parent: Option<String>,
    body: String,
    version: u32,
    by: String,
}

impl Kept {
    fn hash(&self) -> String {
        format!("h{}", self.version)
    }
}

/// Lo que el servidor de mentira tiene guardado del documento del espacio
/// `a`: sus pestañas y sus páginas.
#[derive(Default)]
struct World {
    tabs: HashMap<String, Kept>,
    pages: Vec<(String, Kept)>,
    /// Con esto, todo guardado falla con ese estado y esa etiqueta.
    failing: Option<(u16, &'static str)>,
    /// Cuánto tarda en contestar un guardado.
    delay: Duration,
    /// Con esto, un guardado contesta 200 sin nada dentro.
    garbled: bool,
    /// El documento en sí (`doc` en el sobre). No existe hasta que se guarda
    /// algo en el nodo.
    head: Option<Value>,
}

impl World {
    fn doc(&self) -> Value {
        let tabs: Vec<Value> = ["overview", "runbook", "decisions", "links"]
            .into_iter()
            .map(|key| match self.tabs.get(key) {
                Some(kept) => json!({ "key": key, "body": kept.body, "bodyHash": kept.hash(), "updatedByName": kept.by }),
                None => json!({ "key": key, "body": "" }),
            })
            .collect();
        json!({ "doc": self.head, "orgId": "o1", "tabs": tabs, "decisions": [], "attachments": [] })
    }

    fn page(&self, id: &str) -> Option<Value> {
        let (_, kept) = self.pages.iter().find(|(page, _)| page == id)?;
        Some(json!({
            "id": id, "parentId": kept.parent, "title": kept.title, "body": kept.body, "bodyHash": kept.hash(), "updatedByName": kept.by,
        }))
    }

    fn tree(&self) -> Value {
        Value::Array(self.pages.iter().map(|(id, kept)| json!({ "id": id, "parentId": kept.parent, "title": kept.title, "hasBody": !kept.body.is_empty() })).collect())
    }

    /// Otra persona guarda esa pestaña mientras tanto.
    fn someone_saves_tab(&mut self, key: &str, body: &str, by: &str) {
        let version = self.tabs.get(key).map(|kept| kept.version + 1).unwrap_or(1);
        self.tabs.insert(key.into(), Kept { title: String::new(), parent: None, body: body.into(), version, by: by.into() });
    }

    fn someone_saves_page(&mut self, id: &str, body: &str, by: &str) {
        let (_, kept) = self.pages.iter_mut().find(|(page, _)| page == id).unwrap();
        kept.body = body.into();
        kept.version += 1;
        kept.by = by.into();
    }
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn refused(status: u16, code: &str, data: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({ "success": false, "error": code, "message": code, "data": data }))
}

/// La regla del servidor: llega tarde quien no trae la versión vigente de
/// algo que ya tiene una.
fn late(kept: Option<&Kept>, sent: &Value) -> bool {
    kept.is_some_and(|kept| sent.get("baseHash").and_then(Value::as_str) != Some(kept.hash().as_str()))
}

/// Contesta a todo lo de `/api/v1/docs/space/a` con lo que haya en `world`.
fn answer(world: &Mutex<World>, request: &Request) -> ResponseTemplate {
    let mut world = world.lock().unwrap();
    let route = request.url.path().trim_start_matches("/api/v1/docs/space/a").to_string();
    let parts: Vec<&str> = route.split('/').filter(|part| !part.is_empty()).collect();
    let sent: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    let verb = request.method.as_str();
    if verb != "GET" {
        if let Some((status, code)) = world.failing {
            return refused(status, code, Value::Null).set_delay(world.delay);
        }
    }
    let delay = world.delay;
    if verb != "GET" && world.garbled {
        return ResponseTemplate::new(200).set_body_json(json!({ "success": true }));
    }
    let reply = match (verb, parts.as_slice()) {
        ("GET", []) => ok(world.doc()),
        ("PUT", ["tabs", key]) => {
            if late(world.tabs.get(*key), &sent) {
                refused(409, "doc-conflict", world.doc())
            } else {
                world.someone_saves_tab(key, sent["body"].as_str().unwrap_or_default(), "Ana Ruiz");
                world.head = Some(json!({ "id": "d1", "maintainerId": "u1", "maintainerName": "Ana Ruiz", "stale": false }));
                ok(world.doc())
            }
        }
        ("GET", ["pages"]) => ok(world.tree()),
        ("POST", ["pages"]) => {
            let id = format!("new{}", world.pages.len() + 1);
            let kept = Kept {
                title: sent["title"].as_str().unwrap_or_default().into(),
                parent: sent["parentId"].as_str().map(str::to_string),
                body: sent["body"].as_str().unwrap_or_default().into(),
                version: 1,
                by: "Ana Ruiz".into(),
            };
            world.pages.push((id.clone(), kept));
            ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": world.page(&id) }))
        }
        ("GET", ["pages", id]) => match world.page(id) {
            Some(page) => ok(json!({ "page": page, "breadcrumb": [], "children": [], "referencedFrom": [] })),
            None => refused(404, "not-found", Value::Null),
        },
        ("PUT", ["pages", id]) => {
            let Some(at) = world.pages.iter().position(|(page, _)| page == id) else { return refused(404, "not-found", Value::Null) };
            if sent.get("body").is_some() && late(Some(&world.pages[at].1), &sent) {
                refused(409, "doc-page-conflict", world.page(id).unwrap())
            } else {
                let kept = &mut world.pages[at].1;
                if let Some(title) = sent.get("title").and_then(Value::as_str) {
                    kept.title = title.into();
                }
                if let Some(body) = sent.get("body").and_then(Value::as_str) {
                    kept.body = body.into();
                    kept.version += 1;
                    kept.by = "Ana Ruiz".into();
                }
                ok(world.page(id).unwrap())
            }
        }
        _ => refused(404, "not-found", Value::Null),
    };
    if verb == "GET" {
        reply
    } else {
        reply.set_delay(delay)
    }
}

struct Rig {
    server: MockServer,
    core: Core,
    world: Arc<Mutex<World>>,
}

impl Rig {
    /// Lo que se le pidió al servidor que no fue leer: verbo, ruta y cuerpo.
    async fn writes(&self) -> Vec<(String, String, Value)> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method.as_str() != "GET")
            .map(|request| (request.method.to_string(), request.url.path().to_string(), serde_json::from_slice(&request.body).unwrap_or(Value::Null)))
            .collect()
    }

    async fn reads(&self, route: &str) -> usize {
        self.server.received_requests().await.unwrap().iter().filter(|request| request.method.as_str() == "GET" && request.url.path() == route).count()
    }

    fn edit(&self) -> cac_core::DocEdit {
        self.core.docs().state().edit.expect("no se está escribiendo nada")
    }

    fn asking(&self) -> Option<DocAsk> {
        self.core.docs().state().edit.and_then(|edit| edit.asking)
    }

    /// Abre el editor de lo que se lee y escribe eso en la caja.
    fn write(&self, body: &str) {
        assert!(self.core.docs().start_edit(), "no dejó empezar a escribir");
        self.core.docs().draft_body(body);
    }
}

fn space(id: &str) -> Value {
    json!({
        "id": id, "orgId": "o1", "name": format!("Espacio {id}"), "color": "#888", "people": [], "folders": [],
        "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 1, "taskCount": 2 }],
    })
}

/// Ana dentro, con ese papel, y la documentación del espacio `a` abierta:
/// un resumen ya escrito (versión `h1`) y dos páginas, `p1` y dentro `p2`.
async fn rig_as(role: &str, superadmin: bool) -> Rig {
    let server = MockServer::start().await;
    let org = |id: &str| json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": role, "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org("o1"), org("o2")]))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/task-spaces/")).respond_with(ok(json!([space("a"), space("b")]))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/docs/")).respond_with(ok(json!({}))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/tasks/")).respond_with(ok(json!([]))).mount(&server).await;

    let mut world = World::default();
    world.someone_saves_tab("overview", "## Qué es\n\nEl espacio.", "Luis Gil");
    let page = |title: &str, parent: Option<&str>, body: &str| Kept { title: title.into(), parent: parent.map(str::to_string), body: body.into(), version: 1, by: "Luis Gil".into() };
    world.pages = vec![("p1".into(), page("Arquitectura", None, "## Capas\n\nTres.")), ("p2".into(), page("", Some("p1"), ""))];
    let world = Arc::new(Mutex::new(world));
    let seen = world.clone();
    Mock::given(path_regex("^/api/v1/docs/space/a(/|$)")).respond_with(move |request: &Request| answer(&seen, request)).mount(&server).await;
    // El otro espacio no tiene nada escrito.
    Mock::given(method("GET"))
        .and(path_regex("^/api/v1/docs/(space/b|list/l-a)$"))
        .respond_with(ok(json!({ "doc": null, "orgId": "o1", "tabs": [], "decisions": [], "attachments": [] })))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path_regex("^/api/v1/docs/(space/b|list/l-a)/pages$")).respond_with(ok(json!([]))).mount(&server).await;

    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "superadmin": superadmin })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.orgs().load().await;
    core.tree().load("o1").await;
    core.docs().open("a").await;
    Rig { server, core, world }
}

async fn rig() -> Rig {
    rig_as("member", false).await
}

/// Guardar una pestaña manda su texto a su ruta con la versión de la que
/// salió, cierra el editor y deja a la vista lo guardado; y el siguiente
/// guardado sale ya de la versión nueva.
///
/// Roturas que atrapa: guardar sin `baseHash` (el servidor lo rechaza
/// siempre que ya haya algo, o —peor— un servidor permisivo pisaría a
/// quien escribió en medio); mandar el hash de antes en el segundo guardado
/// (chocaría con uno mismo); escribir en la pestaña que no es; dejar el
/// editor abierto o la lectura con el texto viejo tras guardar.
#[tokio::test]
async fn saving_a_tab_sends_its_text_and_the_version_it_came_from() {
    let rig = rig().await;
    let docs = rig.core.docs();

    rig.write("## Qué es\n\nEl espacio de todos.");
    assert_eq!(rig.edit().what, DocEditWhat::Tab { key: "overview".into() });
    assert_eq!(docs.state().edit_heading(Locale::Es), None, "una pestaña ya dice cuál es en su fila");
    assert!(rig.writes().await.is_empty(), "escribir no guarda: se guarda con su botón");
    docs.save().await;

    assert_eq!(
        rig.writes().await,
        [("PUT".to_string(), "/api/v1/docs/space/a/tabs/overview".to_string(), json!({ "body": "## Qué es\n\nEl espacio de todos.", "baseHash": "h1" }))]
    );
    let state = docs.state();
    assert!(state.edit.is_none(), "el editor sigue abierto tras guardar");
    assert_eq!(state.body(), "## Qué es\n\nEl espacio de todos.");

    rig.write("otra vez");
    docs.save().await;
    assert_eq!(rig.writes().await[1].2["baseHash"], "h2", "el segundo guardado no sale de la versión que dejó el primero");
    assert!(docs.state().edit.is_none(), "chocó consigo mismo");

    // Una pestaña que nunca se guardó no tiene versión que decir.
    docs.show_tab("runbook");
    rig.write("pasos");
    docs.save().await;
    let (_, route, sent) = rig.writes().await.pop().unwrap();
    assert_eq!(route, "/api/v1/docs/space/a/tabs/runbook");
    assert_eq!(sent, json!({ "body": "pasos" }));
    assert_eq!(docs.state().body(), "pasos");
}

/// Un texto guardado con líneas en blanco delante se abre en el editor por
/// su primera línea escrita, y eso solo no cuenta como un cambio.
///
/// Roturas que atrapa: abrir el editor con renglones vacíos encima del
/// texto; dar por cambiado —y preguntar «¿descartar?» al salir— un texto
/// que nadie tocó.
#[tokio::test]
async fn text_saved_with_blank_lines_in_front_opens_at_its_first_line() {
    let rig = rig().await;
    rig.world.lock().unwrap().someone_saves_tab("overview", "\n\n## Qué es\n\nEl espacio.", "Luis Gil");
    rig.world.lock().unwrap().someone_saves_page("p1", "\n\n## Capas\n\nTres.", "Luis Gil");
    let docs = rig.core.docs();
    docs.open("a").await;
    assert!(docs.start_edit());
    let edit = rig.edit();
    assert_eq!(edit.body, "## Qué es\n\nEl espacio.");
    assert!(!edit.is_dirty(), "abrir el editor ya cuenta como un cambio");
    docs.cancel_edit();
    assert_eq!(docs.state().edit, None, "salir sin tocar nada preguntó");

    docs.open_page("p1").await;
    assert!(docs.start_edit());
    assert_eq!(rig.edit().body, "## Capas\n\nTres.");
    assert!(!rig.edit().is_dirty());
}

/// Hay texto que perder cuando el borrador ya no es lo que se leyó; un
/// editor abierto y sin tocar, o ya guardado, no tiene nada que perder. Y
/// la pregunta de cerrar la ventana dice lo que se pierde.
///
/// Roturas que atrapa: cerrar la ventana con medio documento escrito sin
/// que nadie pregunte; preguntar «¿perder lo escrito?» a quien no escribió
/// nada; una pregunta cuyo botón rojo no dice que cierra.
#[tokio::test]
async fn unsaved_text_is_what_closing_the_window_would_lose() {
    let rig = rig().await;
    assert!(!rig.core.unsaved_docs());
    assert!(rig.core.docs().start_edit());
    assert!(!rig.core.unsaved_docs(), "un editor sin tocar no tiene nada que perder");
    rig.core.docs().draft_body("a medias");
    assert!(rig.core.unsaved_docs());
    rig.core.docs().save().await;
    assert!(!rig.core.unsaved_docs(), "lo guardado ya no se pierde");

    let asked = cac_core::close_window_question(Locale::Es);
    assert_eq!((asked.confirm.as_str(), asked.cancel.as_str()), ("Cerrar sin guardar", "Seguir aquí"));
    assert!(asked.title.starts_with('¿') && asked.body.contains("sin guardar"));
    assert_eq!(cac_core::close_window_question(Locale::En).confirm, "Close without saving");
}

/// El primer guardado crea el documento, y con él llega su cabecera; al
/// volver a abrirlo sigue ahí, y la documentación de otro nodo no la hereda.
///
/// Roturas que atrapa: escribir en un nodo y seguir sin ver quién responde
/// de él hasta reabrirlo; leer el documento sin su cabecera; ver sobre un
/// nodo sin nada escrito el responsable del que se miraba antes.
#[tokio::test]
async fn the_header_arrives_with_the_document() {
    let rig = rig().await;
    let docs = rig.core.docs();
    assert_eq!(docs.state().info, None, "un nodo en el que nadie guardó nada no tiene cabecera");

    rig.write("## Qué es\n\nOtra cosa.");
    docs.save().await;
    let owner = |core: &Core| core.docs().state().info.and_then(|info| info.maintainer_name);
    assert_eq!(owner(&rig.core).as_deref(), Some("Ana Ruiz"));
    assert!(docs.state().header(Locale::Es, false).is_some_and(|header| header.owner == "Ana Ruiz"));

    // Otro nodo no la hereda, y al volver llega con el documento.
    docs.open("b").await;
    assert_eq!(docs.state().info, None);
    docs.open("a").await;
    assert_eq!(owner(&rig.core).as_deref(), Some("Ana Ruiz"));
}

/// Si alguien guardó mientras se escribía, el guardado no se aplica, lo
/// escrito se queda en la caja y se dice quién llegó antes; «Guardar» ya no
/// manda nada hasta decidir.
///
/// Roturas que atrapa: perder lo escrito al chocar (cerrar el editor, o
/// cambiar la caja por lo del otro); no enseñar el choque (parece guardado
/// y no lo está); seguir mandando el mismo guardado, que choca en bucle.
#[tokio::test]
async fn a_late_save_keeps_the_text_and_shows_the_conflict() {
    let rig = rig().await;
    let docs = rig.core.docs();
    rig.write("lo mío");
    // Sin choque no hay nada que decidir.
    docs.ask_overwrite();
    docs.ask_load_theirs();
    assert!(rig.asking().is_none(), "preguntó por un choque que no hay");
    // Un fallo antes, para ver que su motivo no se queda junto al choque.
    rig.world.lock().unwrap().failing = Some((500, ""));
    docs.save().await;
    assert!(rig.edit().error.is_some());
    rig.world.lock().unwrap().failing = None;
    rig.world.lock().unwrap().someone_saves_tab("overview", "lo de Luis", "Luis Gil");

    docs.save().await;

    let edit = rig.edit();
    assert_eq!(edit.body, "lo mío", "lo escrito no sobrevivió al choque");
    assert!(!edit.saving);
    let conflict = edit.conflict.clone().expect("no se enteró del choque");
    assert_eq!((conflict.body.as_str(), conflict.hash.as_deref(), conflict.by_name.as_deref()), ("lo de Luis", Some("h2"), Some("Luis Gil")));
    assert!(edit.conflict_phrase(Locale::Es).unwrap().starts_with("Luis Gil guardó"));
    assert!(edit.error.is_none(), "un choque no es un fallo: tiene su propio aviso");
    assert_eq!(rig.world.lock().unwrap().tabs["overview"].body, "lo de Luis", "se pisó lo del otro");

    assert!(!edit.can_save());
    docs.save().await;
    assert_eq!(rig.writes().await.len(), 2, "volvió a mandar lo mismo con el choque a la vista");

    // Al lado se puede mirar lo que hay ahora, sin soltar lo propio.
    docs.show_theirs(true);
    assert_eq!(rig.edit().preview(), "lo de Luis");
    assert_eq!(rig.edit().body, "lo mío");
    docs.show_theirs(false);
    assert_eq!(rig.edit().preview(), "lo mío");
}

/// Tras un choque, «guardar el mío encima» pregunta, y sólo si contestan
/// que sí guarda, saliendo de la versión que había visto.
///
/// Roturas que atrapa: pisar al otro sin preguntar; preguntar y guardar
/// también con un «no»; guardar encima con la versión vieja (vuelve a
/// chocar para siempre) o sin ninguna.
#[tokio::test]
async fn saving_over_theirs_asks_first_and_starts_from_what_was_seen() {
    let rig = rig().await;
    let docs = rig.core.docs();
    rig.write("lo mío");
    rig.world.lock().unwrap().someone_saves_tab("overview", "lo de Luis", "Luis Gil");
    docs.save().await;

    docs.ask_overwrite();
    assert_eq!(rig.asking(), Some(DocAsk::Overwrite));
    assert_eq!(rig.writes().await.len(), 1, "guardó encima sin esperar la respuesta");
    docs.answer(false).await;
    assert_eq!(rig.writes().await.len(), 1, "guardó encima con un «no»");
    assert!(rig.edit().conflict.is_some() && rig.asking().is_none());

    docs.ask_overwrite();
    docs.answer(true).await;
    let (_, _, sent) = rig.writes().await.pop().unwrap();
    assert_eq!(sent, json!({ "body": "lo mío", "baseHash": "h2" }));
    assert!(docs.state().edit.is_none());
    assert_eq!(docs.state().body(), "lo mío");
}

/// Si entre ver el choque y guardar encima vuelve a cambiar, vuelve a
/// chocar: no se pisa a un tercero que nadie vio.
///
/// Rotura que atrapa: guardar encima sin versión, o con la que haya en el
/// servidor en ese momento, que se lleva por delante lo que nadie leyó.
#[tokio::test]
async fn saving_over_theirs_does_not_run_over_a_third() {
    let rig = rig().await;
    let docs = rig.core.docs();
    rig.write("lo mío");
    rig.world.lock().unwrap().someone_saves_tab("overview", "lo de Luis", "Luis Gil");
    docs.save().await;
    rig.world.lock().unwrap().someone_saves_tab("overview", "lo de Eva", "Eva Sanz");

    docs.ask_overwrite();
    docs.answer(true).await;

    let edit = rig.edit();
    assert_eq!(edit.conflict.as_ref().map(|conflict| conflict.body.as_str()), Some("lo de Eva"));
    assert_eq!(edit.body, "lo mío");
    assert_eq!(rig.world.lock().unwrap().tabs["overview"].body, "lo de Eva");
}

/// Tras un choque, «descartar el mío y recargar» pregunta, y con un sí
/// cierra el editor y deja a la vista lo que hay guardado, vuelto a pedir.
///
/// Roturas que atrapa: tirar lo escrito sin preguntar; tirarlo con un
/// «no»; cerrar el editor y seguir leyendo la versión de antes del choque.
#[tokio::test]
async fn loading_theirs_asks_first_and_then_reads_what_is_saved() {
    let rig = rig().await;
    let docs = rig.core.docs();
    rig.write("lo mío");
    rig.world.lock().unwrap().someone_saves_tab("overview", "lo de Luis", "Luis Gil");
    docs.save().await;

    docs.ask_load_theirs();
    assert_eq!(rig.asking(), Some(DocAsk::LoadTheirs));
    assert_eq!(rig.edit().body, "lo mío", "lo tiró sin esperar la respuesta");
    docs.answer(false).await;
    assert_eq!(rig.edit().body, "lo mío", "lo tiró con un «no»");

    let before = rig.reads("/api/v1/docs/space/a").await;
    docs.ask_load_theirs();
    docs.answer(true).await;
    assert!(docs.state().edit.is_none());
    assert_eq!(docs.state().body(), "lo de Luis");
    assert_eq!(rig.reads("/api/v1/docs/space/a").await, before + 1, "no volvió a pedir el documento");
    assert_eq!(rig.writes().await.len(), 1, "descartar no escribe nada");
}

/// Un guardado que falla —el servidor dice que no, o no contesta— deja el
/// texto en la caja, dice por qué y se puede volver a intentar.
///
/// Roturas que atrapa: cerrar el editor como si se hubiera guardado; dejar
/// «guardando…» para siempre (no se puede reintentar); un fallo sin motivo;
/// el motivo del intento anterior a la vista tras uno que sí salió.
#[tokio::test]
async fn a_failed_save_keeps_the_text_and_says_why() {
    let rig = rig().await;
    let docs = rig.core.docs();
    rig.write("un texto enorme");
    rig.world.lock().unwrap().failing = Some((413, "body-too-large"));

    docs.save().await;

    let edit = rig.edit();
    assert_eq!(edit.body, "un texto enorme");
    assert!(!edit.saving && edit.conflict.is_none());
    assert!(edit.error_phrase(Locale::Es).unwrap().contains("demasiado largo"));
    assert!(edit.can_save(), "no deja reintentar");
    assert_eq!(docs.state().body(), "## Qué es\n\nEl espacio.", "lo que se lee detrás cambió sin haberse guardado");

    rig.world.lock().unwrap().failing = Some((500, ""));
    docs.save().await;
    assert_eq!(rig.edit().error_phrase(Locale::Es).as_deref(), Some("No se pudo guardar esta sección"));

    // Un 409 que no es «alguien llegó antes» es un fallo más, no un choque.
    rig.world.lock().unwrap().failing = Some((409, "page-cycle"));
    docs.save().await;
    assert!(rig.edit().conflict.is_none() && rig.edit().error.is_some());

    // Y un 200 que no trae lo guardado no se da por guardado.
    rig.world.lock().unwrap().failing = None;
    rig.world.lock().unwrap().garbled = true;
    docs.save().await;
    assert_eq!(rig.edit().body, "un texto enorme", "dio por guardado lo que no se sabe si se guardó");
    assert!(rig.edit().error.is_some());

    rig.world.lock().unwrap().garbled = false;
    docs.save().await;
    assert!(docs.state().edit.is_none());
    assert_eq!(docs.state().body(), "un texto enorme");
}

/// Lo que se escribe mientras viaja un guardado no se pierde: al volver, el
/// editor sigue abierto con ello, y ya sale de la versión recién guardada.
///
/// Roturas que atrapa: cerrar el editor al llegar la respuesta, con lo
/// tecleado entre tanto dentro; dar por guardado lo último que hay en la
/// caja cuando lo que se mandó fue lo de antes.
#[tokio::test]
async fn what_is_typed_while_a_save_travels_is_not_lost() {
    let rig = rig().await;
    let docs = rig.core.docs();
    rig.write("primera frase");
    rig.world.lock().unwrap().delay = Duration::from_millis(150);

    let saving = {
        let docs = rig.core.docs();
        tokio::spawn(async move { docs.save().await })
    };
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(rig.edit().saving);
    // Pulsar guardar otra vez mientras viaja no manda otro.
    docs.save().await;
    docs.draft_body("primera frase y otra");
    saving.await.unwrap();
    assert_eq!(rig.writes().await.len(), 1, "dos guardados a la vez del mismo texto");

    let edit = rig.edit();
    assert_eq!(edit.body, "primera frase y otra");
    assert_eq!((edit.base_body.as_str(), edit.base_hash.as_deref()), ("primera frase", Some("h2")));
    assert!(edit.is_dirty() && !edit.saving);
    assert_eq!(rig.writes().await[0].2["body"], "primera frase");
}

/// Todo lo que saca del editor con cambios sin guardar pregunta antes; con
/// un «no» no pasa nada y el texto sigue; con un «sí» se hace lo que se iba
/// a hacer.
///
/// Roturas que atrapa, por cada salida (otra pestaña, una página, la
/// portada, cerrar, las tareas, «Mi trabajo», otro nodo, otra
/// organización, cerrar la sesión, una página nueva, «Cancelar»): irse sin preguntar y
/// perder lo escrito; preguntar y no ir con un «sí»; ir con un «no».
#[tokio::test]
async fn leaving_with_unsaved_text_asks_first() {
    type Go = Box<dyn Fn(Core) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send>;
    fn go<F, Fut>(run: F) -> Go
    where
        F: Fn(Core) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        Box::new(move |core| Box::pin(run(core)))
    }
    // Cada salida: cómo se pide, qué se pregunta y cómo se sabe que se fue.
    let exits: Vec<(&str, Go, DocLeave, fn(&Core) -> bool)> = vec![
        ("otra pestaña", go(|core| async move { core.docs().show_tab("runbook") }), DocLeave::Tab { key: "runbook".into() }, |core| {
            core.docs().state().tab == "runbook"
        }),
        ("una página", go(|core| async move { core.docs().open_page("p1").await }), DocLeave::Page { id: "p1".into() }, |core| {
            core.docs().state().page_id.as_deref() == Some("p1")
        }),
        ("cerrar", go(|core| async move { core.docs().close() }), DocLeave::Close, |core| core.docs().state().target.is_none()),
        ("las tareas", go(|core| async move { core.back_to_tasks() }), DocLeave::Tasks { layout: None }, |core| {
            core.docs().state().target.is_none() && core.my_work().state().scope.is_some()
        }),
        (
            "las tareas, en lista",
            go(|core| async move { core.back_to_tasks_in(WorkLayout::List) }),
            DocLeave::Tasks { layout: Some(WorkLayout::List) },
            |core| core.docs().state().target.is_none() && core.work_layout() == WorkLayout::List,
        ),
        ("«Mi trabajo»", go(|core| async move { core.show_all_work() }), DocLeave::AllWork, |core| core.docs().state().target.is_none()),
        ("otro nodo", go(|core| async move { core.docs().open("b").await }), DocLeave::Node { id: "b".into() }, |core| {
            core.docs().state().target.is_some_and(|target| target.id == "b")
        }),
        (
            "otra organización",
            go(|core| async move {
                core.switch_org("o2").await;
            }),
            DocLeave::Org { id: "o2".into() },
            |core| core.orgs().state().current_org_id.as_deref() == Some("o2") && core.docs().state().target.is_none(),
        ),
        ("cerrar la sesión", go(|core| async move { core.logout().await }), DocLeave::LogOut, |core| core.session().is_none()),
        ("una página nueva", go(|core| async move { core.docs().new_page(None) }), DocLeave::NewPage { parent_id: None }, |core| {
            core.docs().state().edit.is_some_and(|edit| edit.what == DocEditWhat::NewPage { parent_id: None } && edit.body.is_empty())
        }),
        ("«Cancelar»", go(|core| async move { core.docs().cancel_edit() }), DocLeave::Cancel, |core| {
            core.docs().state().edit.is_none() && core.docs().state().target.is_some()
        }),
    ];

    for (name, leave, asked, left) in exits {
        let rig = rig().await;
        rig.core.set_work_layout(WorkLayout::Board).unwrap();
        rig.write("a medias");

        leave(rig.core.clone()).await;
        assert_eq!(rig.asking(), Some(DocAsk::Leave { to: asked.clone() }), "{name}: no preguntó");
        assert!(!left(&rig.core), "{name}: se fue sin esperar la respuesta");
        assert_eq!(rig.edit().body, "a medias", "{name}: perdió lo escrito");

        rig.core.docs().answer(false).await;
        assert!(!left(&rig.core), "{name}: se fue con un «no»");
        assert_eq!(rig.edit().body, "a medias", "{name}: un «no» perdió lo escrito");
        assert!(rig.asking().is_none(), "{name}: la pregunta sigue puesta");

        leave(rig.core.clone()).await;
        rig.core.docs().answer(true).await;
        assert!(left(&rig.core), "{name}: con un «sí» no se fue");
        let saved = rig.writes().await.into_iter().filter(|(_, route, _)| route.starts_with("/api/v1/docs")).count();
        assert_eq!(saved, 0, "{name}: descartar guardó algo");
    }
}

/// De una página a la portada también se pregunta; cerrar la hoja de
/// páginas mientras se escribe una pestaña, no: no se va a ningún sitio.
///
/// Roturas que atrapa: volver a la portada y perder lo escrito en la
/// página; preguntar «¿descartar?» por cerrar una hoja.
#[tokio::test]
async fn going_home_from_a_page_asks_and_closing_the_sheet_does_not() {
    let rig = rig().await;
    let docs = rig.core.docs();
    rig.write("a medias");
    docs.show_pages().await;
    docs.close_page();
    assert!(rig.asking().is_none() && !docs.state().pages_shown);
    docs.answer(true).await;
    assert_eq!(rig.edit().body, "a medias", "un «sí» sin pregunta hizo algo");
    docs.cancel_edit();
    docs.answer(true).await;

    docs.open_page("p1").await;
    rig.write("a medias en la página");
    docs.close_page();
    assert_eq!(rig.asking(), Some(DocAsk::Leave { to: DocLeave::Home }));
    assert!(docs.state().on_page());
    docs.answer(true).await;
    assert!(!docs.state().on_page() && docs.state().edit.is_none());
}

/// La respuesta de un guardado que se dejó atrás no cae sobre otro
/// borrador.
///
/// Rotura que atrapa: salir de una pestaña con el guardado en vuelo,
/// empezar a escribir otra, y que la respuesta de la primera le cambie a
/// la segunda de qué versión sale (o la cierre con el texto dentro).
#[tokio::test]
async fn a_late_answer_does_not_land_on_another_draft() {
    let rig = rig().await;
    let docs = rig.core.docs();
    rig.write("el resumen nuevo");
    rig.world.lock().unwrap().delay = Duration::from_millis(150);
    let saving = {
        let docs = rig.core.docs();
        tokio::spawn(async move { docs.save().await })
    };
    tokio::time::sleep(Duration::from_millis(40)).await;
    docs.show_tab("runbook");
    docs.answer(true).await;
    rig.write("el runbook");
    let other = rig.edit();
    saving.await.unwrap();

    assert_eq!(rig.edit(), other, "la respuesta de otro guardado tocó este borrador");
}

/// Sin cambios no hay nada que preguntar: se sale sin más, y el editor
/// vacío no se queda puesto encima de lo siguiente.
///
/// Roturas que atrapa: preguntar «¿descartar?» a quien no escribió nada;
/// dejar el editor de una pestaña abierto sobre otra (se guardaría el
/// texto de una en la otra).
#[tokio::test]
async fn leaving_without_changes_just_leaves() {
    let rig = rig().await;
    let docs = rig.core.docs();
    assert!(docs.start_edit());

    docs.show_tab("runbook");
    let state = docs.state();
    assert_eq!(state.tab, "runbook");
    assert!(state.edit.is_none());

    // «Guardar» sin cambios cierra el editor y no manda nada: no hay por
    // qué escribir en el historial una versión igual a la anterior.
    assert!(docs.start_edit());
    docs.save().await;
    assert!(docs.state().edit.is_none());
    assert!(rig.writes().await.is_empty(), "guardó un texto que no cambió");

    // Y «Cancelar» sin cambios cierra el editor sin preguntar.
    assert!(docs.start_edit());
    docs.cancel_edit();
    assert!(docs.state().edit.is_none());

    // Escribir y volver a dejarlo como estaba tampoco es un cambio.
    docs.show_tab("overview");
    rig.write("otra cosa");
    docs.draft_body("## Qué es\n\nEl espacio.");
    docs.cancel_edit();
    assert!(docs.state().edit.is_none(), "preguntó por un texto que es el que había");
}

/// La documentación que se vuelve a abrir del mismo nodo —se puso al día, o
/// el nodo cambió de nombre— no se lleva lo que se estaba escribiendo.
///
/// Rotura que atrapa: vaciar el estado al reabrir y, con él, el borrador
/// (lo hacía con un nodo renombrado, que es otro `target`).
#[tokio::test]
async fn reopening_the_same_node_keeps_the_draft() {
    let rig = rig().await;
    rig.write("a medias");

    rig.core.docs().open("a").await;
    assert_eq!(rig.edit().body, "a medias");
    assert!(rig.asking().is_none(), "reabrir el mismo nodo no es irse");

    // El espacio cambia de nombre y el árbol se vuelve a traer.
    rig.server.reset().await;
    let mut renamed = space("a");
    renamed["name"] = json!("Otro nombre");
    Mock::given(method("GET")).and(path("/api/v1/task-spaces/")).respond_with(ok(json!([renamed]))).mount(&rig.server).await;
    let seen = rig.world.clone();
    Mock::given(path_regex("^/api/v1/docs/space/a(/|$)")).respond_with(move |request: &Request| answer(&seen, request)).mount(&rig.server).await;
    rig.core.tree().load("o1").await;
    rig.core.docs().open("a").await;

    assert_eq!(rig.core.docs().state().target.unwrap().name, "Otro nombre");
    assert_eq!(rig.edit().body, "a medias");
}

/// Guardar una página manda sólo lo que cambió: renombrar no manda el
/// texto, y el texto va con su versión. Después se lee lo guardado y el
/// árbol dice el título nuevo.
///
/// Roturas que atrapa: mandar el texto al renombrar (choca con quien lo
/// esté escribiendo, o lo pisa); mandar el texto sin `baseHash`; no volver
/// a pedir el árbol (sigue con el título viejo); un título con espacios a
/// los lados guardado tal cual.
#[tokio::test]
async fn saving_a_page_sends_only_what_changed() {
    let rig = rig().await;
    let docs = rig.core.docs();
    docs.open_page("p1").await;

    assert!(docs.start_edit());
    assert_eq!((rig.edit().title.as_str(), rig.edit().body.as_str()), ("Arquitectura", "## Capas\n\nTres."));
    docs.draft_title("  Arquitectura nueva ");
    let trees = rig.reads("/api/v1/docs/space/a/pages").await;
    docs.save().await;

    assert_eq!(rig.writes().await, [("PUT".to_string(), "/api/v1/docs/space/a/pages/p1".to_string(), json!({ "title": "Arquitectura nueva" }))]);
    let state = docs.state();
    assert!(state.edit.is_none());
    assert_eq!(state.page_title(Locale::Es).as_deref(), Some("Arquitectura nueva"));
    assert_eq!(rig.reads("/api/v1/docs/space/a/pages").await, trees + 1, "el árbol no se volvió a pedir");
    assert_eq!(state.page_rows(Locale::Es)[0].title, "Arquitectura nueva");

    assert!(docs.start_edit());
    docs.draft_body("## Capas\n\nCuatro.");
    docs.save().await;
    assert_eq!(rig.writes().await[1].2, json!({ "body": "## Capas\n\nCuatro.", "baseHash": "h1" }));
    assert_eq!(docs.state().body(), "## Capas\n\nCuatro.");
}

/// Lo que se teclea mientras viaja el guardado de una página tampoco se
/// pierde, y el siguiente guardado sale de la versión recién guardada.
///
/// Rotura que atrapa: seguir con la versión de antes tras guardar una
/// página, y chocar con uno mismo en el guardado siguiente.
#[tokio::test]
async fn a_page_saved_while_typing_goes_on_from_the_new_version() {
    let rig = rig().await;
    let docs = rig.core.docs();
    docs.open_page("p1").await;
    rig.write("uno");
    rig.world.lock().unwrap().delay = Duration::from_millis(150);
    let saving = {
        let docs = rig.core.docs();
        tokio::spawn(async move { docs.save().await })
    };
    tokio::time::sleep(Duration::from_millis(40)).await;
    docs.draft_body("uno y dos");
    saving.await.unwrap();
    assert_eq!(rig.edit().body, "uno y dos");

    rig.world.lock().unwrap().delay = Duration::ZERO;
    docs.save().await;
    assert_eq!(rig.writes().await[1].2, json!({ "body": "uno y dos", "baseHash": "h2" }));
    assert!(docs.state().edit.is_none(), "chocó consigo mismo");
    assert_eq!(docs.state().body(), "uno y dos");
}

/// Una página también choca: lo escrito se queda y se enseña lo que hay.
///
/// Rotura que atrapa: leer el 409 de una página como el de una pestaña (el
/// choque sale vacío, y «ver lo que hay ahora» no enseña nada).
#[tokio::test]
async fn a_page_conflicts_like_a_tab() {
    let rig = rig().await;
    let docs = rig.core.docs();
    docs.open_page("p1").await;
    rig.write("lo mío");
    rig.world.lock().unwrap().someone_saves_page("p1", "lo de Luis", "Luis Gil");

    docs.save().await;

    let edit = rig.edit();
    assert_eq!(edit.body, "lo mío");
    let conflict = edit.conflict.expect("no se enteró del choque");
    assert_eq!((conflict.body.as_str(), conflict.hash.as_deref(), conflict.by_name.as_deref()), ("lo de Luis", Some("h2"), Some("Luis Gil")));
}

/// Una página nueva no existe hasta que se guarda: abrirla no pide nada, y
/// guardarla la crea con su título, su texto y de quién cuelga, la abre y
/// vuelve a pedir el árbol.
///
/// Roturas que atrapa: crear la página al pulsar «+» (queda una «Sin
/// título» por cada clic, a la vista de todos); crearla en la portada
/// cuando se pidió dentro de otra; no abrirla tras crearla; que lo que se
/// escriba después cree otra en vez de editar ésa.
#[tokio::test]
async fn a_new_page_is_not_created_until_saved() {
    let rig = rig().await;
    let docs = rig.core.docs();
    docs.show_pages().await;

    docs.new_page(Some("p1"));
    let state = docs.state();
    assert_eq!(state.edit.as_ref().map(|edit| edit.what.clone()), Some(DocEditWhat::NewPage { parent_id: Some("p1".into()) }));
    assert!(!state.pages_shown, "la hoja se queda encima del editor");
    assert_eq!(state.edit_heading(Locale::Es).as_deref(), Some("Página nueva dentro"));
    assert!(rig.writes().await.is_empty(), "se creó sin guardar");

    // Sin nada escrito, cancelar no pregunta: no hay nada que perder.
    docs.cancel_edit();
    assert!(docs.state().edit.is_none());
    assert!(rig.writes().await.is_empty());

    docs.new_page(Some("p1"));
    docs.draft_title(" Los tipos ");
    docs.draft_body("## Qué son");
    docs.save().await;

    assert_eq!(
        rig.writes().await,
        [("POST".to_string(), "/api/v1/docs/space/a/pages".to_string(), json!({ "title": "Los tipos", "parentId": "p1", "body": "## Qué son" }))]
    );
    let state = docs.state();
    assert!(state.edit.is_none());
    assert_eq!(state.page_id.as_deref(), Some("new3"));
    assert_eq!(state.body(), "## Qué son");
    assert!(state.pages.iter().any(|page| page.id == "new3" && page.parent_id.as_deref() == Some("p1")), "el árbol no trae la nueva");

    // En la portada va sin madre.
    docs.new_page(None);
    assert_eq!(docs.state().edit_heading(Locale::Es).as_deref(), Some("Página nueva"));
    docs.draft_title("Suelta");
    docs.save().await;
    assert_eq!(rig.writes().await[1].2, json!({ "title": "Suelta", "body": "" }));
}

/// Si se sigue escribiendo mientras se crea la página, lo siguiente es
/// editar esa página, no crear otra.
///
/// Rotura que atrapa: dos páginas con el mismo título, una por guardado.
#[tokio::test]
async fn typing_while_a_page_is_created_edits_that_page() {
    let rig = rig().await;
    let docs = rig.core.docs();
    docs.new_page(None);
    docs.draft_title("Nueva");
    rig.world.lock().unwrap().delay = Duration::from_millis(150);
    let saving = {
        let docs = rig.core.docs();
        tokio::spawn(async move { docs.save().await })
    };
    tokio::time::sleep(Duration::from_millis(40)).await;
    docs.draft_body("y su texto");
    saving.await.unwrap();

    assert_eq!(rig.edit().what, DocEditWhat::Page { id: "new3".into() });
    rig.world.lock().unwrap().delay = Duration::ZERO;
    docs.save().await;

    let writes = rig.writes().await;
    assert_eq!(writes.len(), 2);
    assert_eq!((writes[1].0.as_str(), writes[1].1.as_str()), ("PUT", "/api/v1/docs/space/a/pages/new3"));
    assert_eq!(writes[1].2, json!({ "body": "y su texto", "baseHash": "h1" }));
    assert_eq!(rig.world.lock().unwrap().pages.len(), 3);
}

/// Una plantilla de página deja su texto en la caja y no guarda nada; sólo
/// en una página en blanco.
///
/// Roturas que atrapa: guardar la plantilla al elegirla (la app de siempre
/// lo hace; aquí nada se escribe sin su botón); echarla encima de una
/// página que ya tiene texto; escribir en el documento la clave de una
/// plantilla que no existe.
#[tokio::test]
async fn a_template_fills_the_box_without_saving() {
    let rig = rig().await;
    let docs = rig.core.docs();
    docs.open_page("p2").await;

    assert!(!docs.start_from_template("inventada", Locale::Es));
    assert!(docs.state().edit.is_none());

    assert!(docs.start_from_template("component", Locale::Es));
    let edit = rig.edit();
    assert!(edit.body.starts_with("## Qué es"));
    assert!(edit.is_dirty() && edit.base_body.is_empty());
    assert!(rig.writes().await.is_empty());
    docs.cancel_edit();
    assert!(rig.asking().is_some(), "una plantilla sin guardar también es texto que se pierde");
    docs.answer(true).await;

    docs.open_page("p1").await;
    assert!(!docs.start_from_template("component", Locale::Es), "echó la plantilla encima de una página escrita");
}

/// Escribe quien puede cambiar el árbol de la organización: un `viewer`
/// lee. Y el registro de decisiones no se escribe como texto.
///
/// Roturas que atrapa: ofrecerle «Editar» o «Página nueva» a quien sólo
/// puede mirar; abrir el editor de markdown sobre el registro de
/// decisiones, que sólo crece entrada a entrada.
#[tokio::test]
async fn a_viewer_reads_and_the_decision_log_is_not_text() {
    let viewer = rig_as("viewer", false).await;
    let state = viewer.core.docs().state();
    assert!(!state.can_edit && !state.editable() && !state.can_add_pages());
    assert!(!viewer.core.docs().start_edit());
    viewer.core.docs().new_page(None);
    assert!(viewer.core.docs().state().edit.is_none());

    // Un superadmin escribe en cualquier organización.
    assert!(rig_as("viewer", true).await.core.docs().state().editable());

    let member = rig().await;
    let docs = member.core.docs();
    assert!(docs.state().editable() && docs.state().can_add_pages());
    docs.show_tab("decisions");
    assert!(!docs.state().editable());
    assert!(!docs.start_edit());
    // Mientras se escribe no se ofrece escribir otra vez.
    docs.show_tab("links");
    assert!(docs.start_edit());
    assert!(!docs.state().editable());
}
