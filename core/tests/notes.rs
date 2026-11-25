//! Notas contra un servidor de mentira que guarda como el de verdad: cada
//! texto tiene su versión (`bodyHash`) y un guardado que no trae la vigente
//! no se aplica: queda como una hija «en conflicto»
//! (`service.NoteService.Update`). Cada prueba dice la rotura que atrapa,
//! comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{Config, Core, Locale, NoListener, NoteAsk, NoteLeave, Storage};
use serde_json::{json, Value};
use wiremock::matchers::path_regex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

#[path = "support/gated.rs"]
mod gated;
use gated::{front, until, Gate};

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

#[derive(Clone)]
struct Stored {
    id: String,
    parent: Option<String>,
    position: i64,
    title: String,
    body: String,
    version: u32,
    favorite: bool,
}

impl Stored {
    fn hash(&self) -> String {
        format!("h{}", self.version)
    }

    fn note(&self) -> Value {
        json!({ "id": self.id, "parentId": self.parent, "title": self.title, "body": self.body, "favorite": self.favorite, "bodyHash": self.hash() })
    }
}

#[derive(Default)]
struct World {
    notes: Vec<Stored>,
    trashed: Vec<String>,
    failing: Option<(u16, &'static str)>,
    /// Con esto, la lectura de esa nota no contesta hasta que se abre.
    gate_read: Option<(String, Gate)>,
    /// Con esto, un guardado no contesta hasta que se abre.
    gate_write: Option<Gate>,
    /// Con esto, la búsqueda de ese texto no contesta hasta que se abre.
    gate_search: Option<(String, Gate)>,
    next: u32,
}

impl World {
    fn add(&mut self, id: &str, parent: Option<&str>, position: i64, title: &str, body: &str) {
        self.notes.push(Stored { id: id.into(), parent: parent.map(str::to_string), position, title: title.into(), body: body.into(), version: 1, favorite: false });
    }

    fn someone_saves(&mut self, id: &str, body: &str) {
        let stored = self.notes.iter_mut().find(|stored| stored.id == id).unwrap();
        stored.body = body.into();
        stored.version += 1;
    }
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn refused(status: u16, code: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({ "success": false, "error": code, "message": code }))
}

fn answer(world: &Mutex<World>, request: &Request) -> ResponseTemplate {
    let mut world = world.lock().unwrap();
    let route = request.url.path().trim_start_matches("/api/v1/notes").to_string();
    let parts: Vec<&str> = route.split('/').filter(|part| !part.is_empty()).collect();
    let sent: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    let verb = request.method.as_str();
    if verb != "GET" {
        if let Some((status, code)) = world.failing {
            return refused(status, code);
        }
    }
    match (verb, parts.as_slice()) {
        ("GET", []) => {
            let live: Vec<Value> = world
                .notes
                .iter()
                .filter(|stored| !world.trashed.contains(&stored.id))
                .map(|stored| json!({ "id": stored.id, "parentId": stored.parent, "position": stored.position, "title": stored.title, "hasBody": !stored.body.is_empty(), "favorite": stored.favorite }))
                .collect();
            ok(Value::Array(live))
        }
        ("GET", ["search"]) => {
            let wanted = request.url.query_pairs().find(|(key, _)| key == "q").map(|(_, value)| value.to_lowercase()).unwrap_or_default();
            let found: Vec<Value> = world
                .notes
                .iter()
                .filter(|stored| stored.title.to_lowercase().contains(&wanted) || stored.body.to_lowercase().contains(&wanted))
                .map(|stored| json!({ "id": stored.id, "title": stored.title, "excerpt": stored.body }))
                .collect();
            let reply = ok(Value::Array(found));
            match &world.gate_search {
                Some((gated, gate)) if *gated == wanted => gate.hold(reply),
                _ => reply,
            }
        }
        ("GET", ["trash"]) => {
            let items: Vec<Value> = world
                .trashed
                .iter()
                .map(|id| {
                    let stored = world.notes.iter().find(|stored| &stored.id == id).unwrap();
                    json!({ "id": id, "title": stored.title, "deletedAt": "2026-11-25T10:00:00Z", "subpages": 0 })
                })
                .collect();
            ok(Value::Array(items))
        }
        ("GET", [id]) => match world.notes.iter().find(|stored| &stored.id == id) {
            Some(stored) => {
                let reply = ok(json!({ "note": stored.note(), "attachments": null, "backlinks": null }));
                match &world.gate_read {
                    Some((gated, gate)) if gated == id => gate.hold(reply),
                    _ => reply,
                }
            }
            None => refused(404, "not-found"),
        },
        ("POST", []) => {
            world.next += 1;
            let id = format!("n{}", world.next);
            let (parent, title) = (sent["parentId"].as_str().map(str::to_string), sent["title"].as_str().unwrap_or_default().to_string());
            world.add(&id, parent.as_deref(), 99, &title, "");
            ok(world.notes.last().unwrap().note())
        }
        ("PATCH", [id]) => {
            let Some(at) = world.notes.iter().position(|stored| &stored.id == id) else { return refused(404, "not-found") };
            let late = sent.get("body").is_some() && sent.get("baseHash").and_then(Value::as_str) != Some(world.notes[at].hash().as_str());
            if late {
                // Como el servidor de verdad: la nota se queda, lo escrito cuelga como hija.
                world.next += 1;
                let child = format!("n{}", world.next);
                let body = sent["body"].as_str().unwrap_or_default().to_string();
                world.add(&child, Some(id), 99, "(conflicting edit)", &body);
                return ok(json!({ "note": world.notes[at].note(), "conflict": { "id": child, "title": "(conflicting edit)" } }));
            }
            let stored = &mut world.notes[at];
            if let Some(title) = sent.get("title").and_then(Value::as_str) {
                stored.title = title.into();
            }
            if let Some(favorite) = sent.get("favorite").and_then(Value::as_bool) {
                stored.favorite = favorite;
            }
            if let Some(body) = sent.get("body").and_then(Value::as_str) {
                stored.body = body.into();
                stored.version += 1;
            }
            let reply = ok(json!({ "note": stored.note() }));
            match &world.gate_write {
                Some(gate) => gate.hold(reply),
                None => reply,
            }
        }
        ("DELETE", [id]) => {
            let mut doomed = vec![id.to_string()];
            let mut next = 0;
            while next < doomed.len() {
                let parent = doomed[next].clone();
                doomed.extend(world.notes.iter().filter(|stored| stored.parent.as_deref() == Some(parent.as_str())).map(|stored| stored.id.clone()));
                next += 1;
            }
            world.trashed.extend(doomed.clone());
            ok(json!({ "deleted": doomed.len() }))
        }
        ("PUT", ["tree"]) => {
            for placed in sent.as_array().cloned().unwrap_or_default() {
                let stored = world.notes.iter_mut().find(|stored| Some(stored.id.as_str()) == placed["id"].as_str()).unwrap();
                stored.position = placed["position"].as_i64().unwrap();
                stored.parent = placed["parentId"].as_str().map(str::to_string);
            }
            ok(json!({}))
        }
        ("DELETE", ["trash", id]) => {
            world.trashed.retain(|trashed| trashed != id);
            world.notes.retain(|stored| &stored.id != id);
            ok(json!({ "deleted": 1 }))
        }
        ("POST", ["trash", id, "restore"]) => {
            world.trashed.retain(|trashed| trashed != id);
            ok(json!({ "restored": 1 }))
        }
        _ => refused(404, "not-found"),
    }
}

struct Rig {
    server: MockServer,
    core: Core,
    world: Arc<Mutex<World>>,
}

impl Rig {
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

    fn edit(&self) -> cac_core::NoteEdit {
        self.core.notes().state().edit.expect("no se está escribiendo nada")
    }

    /// Abre la nota, empieza a escribir y deja esto en la caja.
    async fn write(&self, id: &str, body: &str) {
        self.core.notes().open(id).await;
        assert!(self.core.notes().start_edit(), "no dejó empezar a escribir");
        self.core.notes().draft_body(body);
    }
}

/// Ana dentro y Notas delante: `a` (con `b` dentro y `c` dentro de `b`), `d`
/// y `e`, todas con algo escrito.
async fn rig() -> Rig {
    let server = MockServer::start().await;
    let mut world = World::default();
    world.add("a", None, 1, "Alfa", "Primera.");
    world.add("b", Some("a"), 1, "Beta", "Segunda.");
    world.add("c", Some("b"), 1, "Gamma", "");
    world.add("d", None, 2, "Delta", "Cuarta.");
    world.add("e", None, 3, "Épsilon", "Quinta.");
    let world = Arc::new(Mutex::new(world));
    let seen = world.clone();
    Mock::given(path_regex("^/api/v1/notes(/|$)")).respond_with(move |request: &Request| answer(&seen, request)).mount(&server).await;
    let core = Core::new(Config::new(front(&server.uri())), Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.notes().enter().await;
    Rig { server, core, world }
}

/// Entrar en Notas trae el árbol; abrir una trae su texto, y las listas que
/// Go escribe `null` no la rompen.
///
/// Roturas que atrapa: no pedir el árbol al entrar; leer mal el sobre de una
/// nota (sin `attachments` ni `backlinks`); dejar delante lo que se leía
/// antes mientras llega la otra.
#[tokio::test]
async fn entering_brings_the_tree_and_opening_reads_the_note() {
    let rig = rig().await;
    let state = rig.core.notes().state();
    assert!(state.on_screen && state.tree_loaded);
    assert_eq!(state.tree.len(), 5);
    assert_eq!(cac_core::note_rows(&state).iter().map(|row| row.title.as_str()).collect::<Vec<_>>(), ["Alfa", "Beta", "Gamma", "Delta", "Épsilon"]);

    rig.core.notes().open("d").await;
    let state = rig.core.notes().state();
    assert_eq!(state.open_id.as_deref(), Some("d"));
    assert_eq!(state.detail.as_ref().map(|detail| detail.note.body.as_str()), Some("Cuarta."));
    assert!(state.detail.unwrap().backlinks.is_empty());
}

/// Una lectura que llega tarde no pisa a la nota que se abrió después.
///
/// Rotura que atrapa: poner en pantalla la respuesta de la nota anterior
/// (se leería «Alfa» bajo la que se abrió).
#[tokio::test]
async fn a_late_read_does_not_cover_the_note_opened_after_it() {
    let rig = rig().await;
    let gate = Gate::default();
    rig.core.notes().open("e").await;
    rig.world.lock().unwrap().gate_read = Some(("a".into(), gate.clone()));
    let slow = tokio::spawn({
        let core = rig.core.clone();
        async move { core.notes().open("a").await }
    });
    until("la lectura de `a` salió", || gate.held() == 1).await;
    assert!(rig.core.notes().state().detail.is_none(), "mientras llega `a` no se enseña `e` bajo su nombre");
    rig.core.notes().open("d").await;
    gate.open();
    slow.await.unwrap();
    let state = rig.core.notes().state();
    assert_eq!(state.open_id.as_deref(), Some("d"));
    assert_eq!(state.detail.map(|detail| detail.note.id), Some("d".to_string()));
}

/// Guardar manda sólo lo que cambió, con la versión de la que salió; el
/// siguiente guardado sale de la nueva; renombrar no manda el texto.
///
/// Roturas que atrapa: guardar sin `baseHash` (un servidor permisivo
/// pisaría a quien escribió en medio); mandar el hash de antes en el segundo
/// guardado (chocaría con uno mismo); mandar el texto al renombrar (chocaría
/// con quien lo escribe); dejar el editor abierto o la nota con el texto
/// viejo tras guardar.
#[tokio::test]
async fn saving_sends_only_what_changed_and_the_version_it_came_from() {
    let rig = rig().await;
    rig.write("d", "Cuarta, corregida.").await;
    assert!(rig.writes().await.is_empty(), "escribir no guarda: se guarda con su botón");
    rig.core.notes().save().await;
    assert_eq!(rig.writes().await, [("PATCH".to_string(), "/api/v1/notes/d".to_string(), json!({ "body": "Cuarta, corregida.", "baseHash": "h1" }))]);
    let state = rig.core.notes().state();
    assert!(state.edit.is_none());
    assert_eq!(state.detail.unwrap().note.body, "Cuarta, corregida.");

    rig.write("d", "Otra vez.").await;
    rig.core.notes().save().await;
    assert_eq!(rig.writes().await[1].2["baseHash"], "h2");

    assert!(rig.core.notes().start_edit());
    rig.core.notes().draft_title("  Delta nueva ");
    rig.core.notes().save().await;
    assert_eq!(rig.writes().await[2].2, json!({ "title": "Delta nueva" }));
    assert_eq!(rig.core.notes().state().tree.iter().find(|item| item.id == "d").unwrap().title, "Delta nueva", "el árbol se pone al día");
}

/// Lo que se escribe mientras un guardado viaja no se pierde, y el siguiente
/// guardado sale de la versión que dejó el primero.
///
/// Roturas que atrapa: cerrar el editor al volver el guardado (se tiraría lo
/// escrito entretanto); no avanzar la versión de la que sale lo siguiente
/// (el segundo guardado chocaría con el primero y lo escrito acabaría en una
/// copia «en conflicto»).
#[tokio::test]
async fn what_is_typed_while_saving_stays_and_saves_from_the_new_version() {
    let rig = rig().await;
    let gate = Gate::default();
    rig.world.lock().unwrap().gate_write = Some(gate.clone());
    rig.write("d", "Uno.").await;
    let saving = tokio::spawn({
        let core = rig.core.clone();
        async move { core.notes().save().await }
    });
    until("el guardado salió", || gate.held() == 1).await;
    rig.core.notes().draft_body("Uno. Y dos.");
    gate.open();
    saving.await.unwrap();
    let edit = rig.edit();
    assert_eq!((edit.body.as_str(), edit.base_body.as_str(), edit.base_hash.as_deref()), ("Uno. Y dos.", "Uno.", Some("h2")));

    rig.core.notes().save().await;
    assert!(rig.core.notes().state().conflict.is_none(), "el segundo guardado chocó con el primero");
    assert_eq!(rig.writes().await[1].2, json!({ "body": "Uno. Y dos.", "baseHash": "h2" }));
    assert!(rig.core.notes().state().edit.is_none());
}

/// Si otro guardó en medio, lo escrito no se pierde: queda como hija y se
/// dice.
///
/// Roturas que atrapa: tratarlo como un fallo y dejar el editor abierto
/// con un texto que ya está guardado en otra parte; no decir dónde quedó lo
/// escrito; no traer al árbol la hija nueva.
#[tokio::test]
async fn a_save_that_lost_the_race_says_where_the_text_went() {
    let rig = rig().await;
    rig.write("d", "Lo mío.").await;
    rig.world.lock().unwrap().someone_saves("d", "Lo suyo.");
    rig.core.notes().save().await;
    let state = rig.core.notes().state();
    assert_eq!(state.conflict.as_ref().map(|conflict| conflict.title.as_str()), Some("(conflicting edit)"));
    assert!(state.edit.is_none());
    assert_eq!(state.detail.unwrap().note.body, "Lo suyo.", "se lee lo que hay guardado");
    assert!(state.tree.iter().any(|item| item.parent_id.as_deref() == Some("d")), "la copia cuelga de la nota");
    rig.core.notes().dismiss_conflict();
    assert!(rig.core.notes().state().conflict.is_none());
}

/// Un guardado que falla deja el texto donde estaba, con su motivo.
///
/// Roturas que atrapa: cerrar el editor al fallar (se perdería lo escrito);
/// no decir por qué.
#[tokio::test]
async fn a_failed_save_keeps_the_text_and_says_why() {
    let rig = rig().await;
    rig.write("d", "Algo nuevo.").await;
    rig.world.lock().unwrap().failing = Some((500, "internal"));
    rig.core.notes().save().await;
    let edit = rig.edit();
    assert_eq!(edit.body, "Algo nuevo.");
    assert!(!edit.saving && edit.is_dirty());
    assert!(edit.error_phrase(Locale::Es).is_some());
}

/// Con texto sin guardar no se cambia de nota ni se cierra: se pregunta, y
/// sólo un sí lo hace; con el editor limpio no se pregunta nada.
///
/// Roturas que atrapa: perder lo escrito al abrir otra nota, al cancelar o
/// al crear otra; preguntar a quien abrió el editor y no tocó nada; que un
/// no cambie de nota igual; no abrir la nota a la que se iba tras el sí.
#[tokio::test]
async fn leaving_unsaved_text_asks_and_only_a_yes_goes() {
    let rig = rig().await;
    rig.core.notes().open("d").await;
    assert!(rig.core.notes().start_edit());
    rig.core.notes().open("e").await;
    let state = rig.core.notes().state();
    assert_eq!(state.open_id.as_deref(), Some("e"), "limpio no pregunta");
    assert!(state.edit.is_none());

    rig.write("d", "Sin guardar.").await;
    rig.core.notes().open("a").await;
    let state = rig.core.notes().state();
    assert_eq!(state.asking, Some(NoteAsk::Leave { to: NoteLeave::Open { id: "a".into() } }));
    assert_eq!(state.open_id.as_deref(), Some("d"));
    assert!(rig.core.unsaved_notes());

    rig.core.notes().answer(false).await;
    assert_eq!(rig.core.notes().state().open_id.as_deref(), Some("d"));
    assert_eq!(rig.edit().body, "Sin guardar.");

    rig.core.notes().open("a").await;
    rig.core.notes().answer(true).await;
    let state = rig.core.notes().state();
    assert_eq!((state.open_id.as_deref(), state.edit.is_none()), (Some("a"), true));
    assert!(!rig.core.unsaved_notes());

    // «Cancelar» y «Nueva» se rigen por lo mismo.
    assert!(rig.core.notes().start_edit());
    rig.core.notes().cancel_edit();
    assert!(rig.core.notes().state().edit.is_none(), "limpio, cancelar cierra sin preguntar");
    rig.write("a", "Otro texto.").await;
    rig.core.notes().cancel_edit();
    assert_eq!(rig.core.notes().state().asking, Some(NoteAsk::Leave { to: NoteLeave::Cancel }));
    rig.core.notes().answer(false).await;
    assert_eq!(rig.edit().body, "Otro texto.");
    rig.core.notes().create(None, "Sin título").await;
    assert_eq!(rig.core.notes().state().asking, Some(NoteAsk::Leave { to: NoteLeave::Create { parent_id: None, title: "Sin título".into() } }));
    assert!(rig.writes().await.is_empty(), "creó sin preguntar");
    rig.core.notes().answer(true).await;
    assert_eq!(rig.writes().await.len(), 1);
    assert_eq!(rig.core.notes().state().open_id.as_deref(), Some("n1"));
}

/// Salir de la pantalla no pierde lo escrito, y cerrar la sesión pregunta.
///
/// Roturas que atrapa: tirar el borrador al salir de Notas (se perdería sin
/// preguntar); salir de la sesión sin preguntar con texto pendiente.
#[tokio::test]
async fn leaving_the_screen_keeps_the_draft_and_logging_out_asks() {
    let rig = rig().await;
    rig.write("d", "Pendiente.").await;
    rig.core.notes().leave();
    assert!(!rig.core.notes().state().on_screen);
    assert_eq!(rig.edit().body, "Pendiente.");
    assert!(rig.core.unsaved_notes());

    rig.core.logout().await;
    assert!(rig.core.session().is_some(), "no salió");
    assert_eq!(rig.core.notes().state().asking, Some(NoteAsk::Leave { to: NoteLeave::LogOut }));
    rig.core.notes().answer(true).await;
    assert!(rig.core.session().is_none());
    assert_eq!(rig.core.notes().state(), Default::default(), "las notas de una persona no son de la siguiente");
}

/// Una nota nueva se crea en el servidor, se abre y queda lista para
/// titularla; una colgada de otra lleva su madre.
///
/// Roturas que atrapa: no mandar la madre (la subpágina nacería en la raíz);
/// no abrirla; abrirla sin editor (habría que pulsar «Editar» antes de
/// titular).
#[tokio::test]
async fn a_new_note_is_created_opened_and_ready_for_its_title() {
    let rig = rig().await;
    rig.core.notes().create(Some("d"), "Sin título").await;
    assert_eq!(rig.writes().await, [("POST".to_string(), "/api/v1/notes/".to_string(), json!({ "title": "Sin título", "parentId": "d" }))]);
    let state = rig.core.notes().state();
    assert_eq!(state.open_id.as_deref(), Some("n1"));
    let edit = state.edit.unwrap();
    assert!(edit.focus_title && !edit.is_dirty());
    assert!(state.tree.iter().any(|item| item.id == "n1" && item.parent_id.as_deref() == Some("d")));
}

/// Borrar pregunta diciendo cuántas páginas se van con ella, y mandarla a la
/// papelera cierra la nota si era la abierta o colgaba de ella.
///
/// Roturas que atrapa: borrar sin preguntar; contar mal lo que cuelga; dejar
/// a la vista una nota que ya no está; un no que borra igual.
#[tokio::test]
async fn deleting_asks_first_and_closes_what_went_with_it() {
    let rig = rig().await;
    rig.core.notes().open("b").await;
    rig.core.notes().ask_delete("a");
    let question = rig.core.notes().state().question(Locale::Es).unwrap();
    assert!(question.title.contains("Alfa"));
    assert!(question.body.contains('2'), "dos subpáginas: {}", question.body);

    rig.core.notes().answer(false).await;
    assert!(rig.writes().await.is_empty());

    rig.core.notes().ask_delete("a");
    rig.core.notes().answer(true).await;
    assert_eq!(rig.writes().await, [("DELETE".to_string(), "/api/v1/notes/a".to_string(), Value::Null)]);
    let state = rig.core.notes().state();
    assert_eq!(state.open_id, None, "la abierta colgaba de la borrada");
    assert!(state.detail.is_none());
    assert_eq!(state.tree.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["d", "e"]);
}

/// La estrella se ve en el acto y se deshace si el servidor dice que no;
/// subir una nota renumera a sus hermanas.
///
/// Roturas que atrapa: esperar al servidor para pintarla; dejarla puesta
/// cuando falló; mandar sólo dos posiciones al mover.
#[tokio::test]
async fn favorites_and_order_change_at_once_and_revert_on_failure() {
    let rig = rig().await;
    rig.core.notes().toggle_favorite("d").await;
    assert!(rig.core.notes().state().tree.iter().find(|item| item.id == "d").unwrap().favorite);
    assert_eq!(rig.writes().await[0], ("PATCH".to_string(), "/api/v1/notes/d".to_string(), json!({ "favorite": true })));

    rig.world.lock().unwrap().failing = Some((500, "internal"));
    rig.core.notes().toggle_favorite("d").await;
    assert!(rig.core.notes().state().tree.iter().find(|item| item.id == "d").unwrap().favorite, "el no del servidor la deja como estaba");
    assert!(rig.core.notes().state().problem.is_some());
    rig.core.notes().dismiss_problem();
    rig.world.lock().unwrap().failing = None;

    rig.core.notes().move_by_one("e", true).await;
    let (verb, route, sent) = rig.writes().await.pop().unwrap();
    assert_eq!((verb.as_str(), route.as_str()), ("PUT", "/api/v1/notes/tree"));
    assert_eq!(sent, json!([{ "id": "a", "parentId": null, "position": 0 }, { "id": "e", "parentId": null, "position": 1 }, { "id": "d", "parentId": null, "position": 2 }]));
    let rows = cac_core::note_rows(&rig.core.notes().state());
    assert_eq!(rows.iter().filter(|row| row.depth == 0).map(|row| row.id.as_str()).collect::<Vec<_>>(), ["a", "e", "d"]);
}

/// La búsqueda enseña lo que el servidor encuentra, y vaciar la caja la
/// limpia.
///
/// Roturas que atrapa: que una respuesta lenta pise la de lo último que se
/// escribió; no limpiar al vaciar la caja (seguirían resultados
/// de lo que ya no se busca); pedir una búsqueda con el texto sin escapar;
/// no cerrar el diálogo con su estado.
#[tokio::test]
async fn search_shows_what_the_server_finds_and_clears_with_the_box() {
    let rig = rig().await;
    rig.core.notes().open_search();
    rig.core.notes().search("segunda").await;
    assert_eq!(rig.core.notes().state().search.results.iter().map(|found| found.id.as_str()).collect::<Vec<_>>(), ["b"]);
    // Una respuesta lenta no pisa a la de lo último que se escribió.
    let gate = Gate::default();
    rig.world.lock().unwrap().gate_search = Some(("al".into(), gate.clone()));
    let slow = tokio::spawn({
        let core = rig.core.clone();
        async move { core.notes().search("al").await }
    });
    until("la búsqueda lenta salió", || gate.held() == 1).await;
    rig.core.notes().search("delta").await;
    gate.open();
    slow.await.unwrap();
    let state = rig.core.notes().state();
    assert_eq!((state.search.query.as_str(), state.search.results.iter().map(|found| found.id.as_str()).collect::<Vec<_>>()), ("delta", vec!["d"]));
    rig.core.notes().search("  ").await;
    assert!(rig.core.notes().state().search.results.is_empty());
    rig.core.notes().close_search();
    assert!(!rig.core.notes().state().search.open);
}

/// La papelera: lo borrado aparece en ella, se recupera, y borrar para
/// siempre o vaciar preguntan.
///
/// Roturas que atrapa: purgar sin preguntar (es lo único irreversible); no
/// volver a traer el árbol al recuperar.
#[tokio::test]
async fn the_trash_lists_restores_and_asks_before_purging() {
    let rig = rig().await;
    rig.core.notes().ask_delete("d");
    rig.core.notes().answer(true).await;
    rig.core.notes().open_trash().await;
    assert_eq!(rig.core.notes().state().trash.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["d"]);

    rig.core.notes().ask_purge("d");
    let question = rig.core.notes().state().question(Locale::Es).unwrap();
    assert!(question.title.contains("Delta"));
    rig.core.notes().answer(false).await;
    assert_eq!(rig.writes().await.len(), 1, "sólo el borrado");

    rig.core.notes().restore("d").await;
    let state = rig.core.notes().state();
    assert!(state.trash.items.is_empty());
    assert!(state.tree.iter().any(|item| item.id == "d"));

    rig.core.notes().ask_delete("e");
    rig.core.notes().answer(true).await;
    rig.core.notes().load_tree().await;
    rig.core.notes().ask_empty_trash();
    assert_eq!(rig.core.notes().state().asking, Some(NoteAsk::EmptyTrash));
    rig.core.notes().answer(true).await;
    assert!(rig.core.notes().state().trash.items.is_empty());
}
