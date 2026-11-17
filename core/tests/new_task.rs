//! Levantar una tarea: dónde puede nacer y qué se manda. Cada prueba dice la
//! rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{
    destinations, drafted, ApiError, Config, Core, Destinations, NoListener, ScopeKind, SpaceTree, Storage, TaskDraft,
    WorkScope,
};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
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

/// Dos espacios de `o1` (el segundo con una carpeta dentro de otra y la
/// lista de un cliente) y uno de otra organización.
fn spaces() -> Vec<SpaceTree> {
    serde_json::from_value(json!([
        { "id": "a", "orgId": "o1", "name": "Interno", "folders": [], "lists": [{ "id": "l-a", "name": "tasks" }] },
        {
            "id": "b", "orgId": "o1", "name": "Proteus",
            "lists": [{ "id": "l-b", "name": "tasks", "projectId": "p1" }],
            "folders": [{
                "id": "f1", "name": "Plataforma", "lists": [{ "id": "l-f1", "name": "tasks" }],
                "folders": [{ "id": "f2", "name": "Infra", "folders": [], "lists": [{ "id": "l-f2", "name": "ops" }] }],
            }],
        },
        { "id": "z", "orgId": "o9", "name": "Ajeno", "folders": [], "lists": [{ "id": "l-z", "name": "tasks" }] },
    ]))
    .unwrap()
}

fn scope(kind: ScopeKind, id: &str) -> WorkScope {
    WorkScope { kind, id: id.into(), name: "x".into(), org_id: Some("o1".into()) }
}

fn said(destinations: &Destinations) -> Vec<(&str, &str, bool)> {
    destinations.options.iter().map(|option| (option.list_id.as_str(), option.label.as_str(), option.client)).collect()
}

/// Se ofrecen las listas de la organización, cada una con su camino, y se
/// sabe cuál es de un cliente.
///
/// Roturas que atrapa: ofrecer una lista de otra organización; dos listas
/// «tasks» que no se distinguen; perder las de una carpeta dentro de otra; no
/// saber que una lista la lee un cliente (no se preguntaría quién la ve).
#[test]
fn every_list_of_the_organization_is_offered_with_its_path() {
    let offered = destinations(&spaces(), Some("o1"), None);

    assert!(!offered.fixed);
    assert_eq!(
        said(&offered),
        [
            ("l-a", "Interno / tasks", false),
            ("l-b", "Proteus / tasks", true),
            ("l-f1", "Proteus / Plataforma / tasks", false),
            ("l-f2", "Proteus / Plataforma / Infra / ops", false),
        ]
    );
}

/// Con un espacio a la vista sólo se ofrecen sus listas; con una lista, va
/// ahí y no se pregunta.
///
/// Roturas que atrapa: crear desde tu lista y que la tarea vaya a la primera
/// del árbol (jose, 11-nov-2026); ofrecer listas de otro espacio; dar por
/// fija una lista que no está en el árbol y quedarse sin dónde crear.
#[test]
fn what_is_looked_at_narrows_where_it_goes() {
    let in_space = destinations(&spaces(), Some("o1"), Some(&scope(ScopeKind::Space, "b")));
    assert!(!in_space.fixed);
    assert_eq!(in_space.options.iter().map(|option| option.list_id.as_str()).collect::<Vec<_>>(), ["l-b", "l-f1", "l-f2"]);

    let in_list = destinations(&spaces(), Some("o1"), Some(&scope(ScopeKind::List, "l-f1")));
    assert!(in_list.fixed);
    assert_eq!(said(&in_list), [("l-f1", "Proteus / Plataforma / tasks", false)]);

    let gone = destinations(&spaces(), Some("o1"), Some(&scope(ScopeKind::List, "no-existe")));
    assert!(!gone.fixed);
    assert_eq!(gone.options.len(), 4);
}

/// Con una carpeta a la vista se ofrecen las listas que cuelgan de ella, a
/// cualquier profundidad, con su camino entero, y se pregunta cuál
/// (command-and-control-382).
///
/// Roturas que atrapa: ofrecer el árbol entero con una carpeta delante (la
/// tarea nacería donde no se está mirando y no saldría en pantalla); perder
/// las de una subcarpeta; colar las listas sueltas de su espacio; contar el
/// camino desde la carpeta y no desde el espacio; quedarse sin dónde crear
/// porque la carpeta ya no está; fijar una lista por compartir id con su
/// carpeta.
#[test]
fn a_folder_in_sight_offers_only_the_lists_under_it() {
    let in_folder = destinations(&spaces(), Some("o1"), Some(&scope(ScopeKind::Folder, "f1")));
    assert!(!in_folder.fixed);
    assert_eq!(
        said(&in_folder),
        [("l-f1", "Proteus / Plataforma / tasks", false), ("l-f2", "Proteus / Plataforma / Infra / ops", false)]
    );

    let in_subfolder = destinations(&spaces(), Some("o1"), Some(&scope(ScopeKind::Folder, "f2")));
    assert_eq!(said(&in_subfolder), [("l-f2", "Proteus / Plataforma / Infra / ops", false)]);
    assert!(!in_subfolder.fixed, "una carpeta con una sola lista no es una lista a la vista");

    let gone = destinations(&spaces(), Some("o1"), Some(&scope(ScopeKind::Folder, "no-existe")));
    assert_eq!(gone.options.len(), 4);

    // Una carpeta y una de sus listas con el mismo id: son de tablas
    // distintas y nada impide que coincidan.
    let twins: Vec<SpaceTree> = serde_json::from_value(json!([{
        "id": "a", "orgId": "o1", "name": "Uno", "lists": [],
        "folders": [{ "id": "x", "name": "Carpeta", "folders": [], "lists": [{ "id": "x", "name": "gemela" }, { "id": "y", "name": "otra" }] }],
    }]))
    .unwrap();
    let offered = destinations(&twins, Some("o1"), Some(&scope(ScopeKind::Folder, "x")));
    assert!(!offered.fixed);
    assert_eq!(offered.options.len(), 2);
}

/// Nace en la lista pedida si se ofrece, y si no en la primera.
///
/// Roturas que atrapa: seguir con la lista de la vista anterior, que ya no se
/// mira; no tener ninguna elegida al abrir la fila.
#[test]
fn the_list_is_the_wanted_one_or_the_first() {
    let offered = destinations(&spaces(), Some("o1"), None);

    assert_eq!(offered.chosen("l-f1").map(|option| option.list_id.as_str()), Some("l-f1"));
    assert_eq!(offered.chosen("").map(|option| option.list_id.as_str()), Some("l-a"));
    assert_eq!(offered.chosen("l-z").map(|option| option.list_id.as_str()), Some("l-a"));
    assert!(Destinations::default().chosen("l-a").is_none());
}

fn code(error: ApiError) -> String {
    match error {
        ApiError::Server { code, .. } => code,
        other => other.to_string(),
    }
}

/// Un borrador manda lo elegido, limpio; sin título o sin lista no hay nada
/// que mandar; y la visibilidad sólo va cuando alguien eligió «interna».
///
/// Roturas que atrapa: crear una tarea sin título; mandar una visibilidad
/// que nadie eligió, contradiciendo al servidor en la lista de un cliente;
/// **no** mandar «interna» cuando se eligió, y que el cliente lea lo que no
/// era para él; perder el responsable.
#[test]
fn a_draft_sends_what_was_chosen() {
    let draft = TaskDraft {
        list_id: "l-b".into(),
        title: "  Subir el backend ".into(),
        priority: "high".into(),
        due_day: Some("2026-12-01".into()),
        assignee_id: Some("u2".into()),
        internal: false,
    };
    let task = drafted(&draft).unwrap();
    assert_eq!(task.title, "Subir el backend");
    assert_eq!(task.priority.as_deref(), Some("high"));
    assert_eq!(task.due_day.as_deref(), Some("2026-12-01"));
    assert_eq!(task.assignee_ids, ["u2"]);
    assert_eq!(task.visibility, None);

    let internal = drafted(&TaskDraft { internal: true, ..draft.clone() }).unwrap();
    assert_eq!(internal.visibility.as_deref(), Some("internal"));

    let bare = drafted(&TaskDraft { list_id: "l-a".into(), title: "x".into(), ..TaskDraft::default() }).unwrap();
    assert_eq!((bare.priority, bare.assignee_ids.len()), (None, 0));

    assert_eq!(code(drafted(&TaskDraft { title: "   ".into(), ..draft.clone() }).unwrap_err()), "empty-title");
    assert_eq!(code(drafted(&TaskDraft { list_id: String::new(), ..draft }).unwrap_err()), "no-list");
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

async fn rig() -> (MockServer, Core) {
    let server = MockServer::start().await;
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    (server, core)
}

async fn mount_org(server: &MockServer) {
    let org = json!({ "id": "o1", "name": "G Studio", "slug": "g", "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" });
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org]))).mount(server).await;
}

async fn asked(server: &MockServer, verb: &str, route: &str) -> Vec<wiremock::Request> {
    let all = server.received_requests().await.unwrap();
    all.into_iter().filter(|request| request.method.as_str() == verb && request.url.path() == route).collect()
}

/// Levantar una tarea la crea en su lista con lo elegido y vuelve a pedir el
/// árbol; una sin título no llega al servidor.
///
/// Roturas que atrapa: crear en otra lista; no mandar lo elegido; que la
/// cuenta de la lista en el árbol se quede con una tarea de menos; pedir el
/// árbol de otra organización; mandar al servidor una tarea sin título.
#[tokio::test]
async fn raising_creates_it_in_its_list_and_counts_again() {
    let (server, core) = rig().await;
    mount_org(&server).await;
    core.orgs().load().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/task-lists/l-b/tasks"))
        .respond_with(ok(json!({ "id": "t9", "createdAt": "2026-11-15T10:00:00Z", "updatedAt": "2026-11-15T10:00:00Z", "listId": "l-b", "seq": 40, "title": "Subir el backend", "status": "pending", "priority": "medium" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/task-spaces/"))
        .and(query_param("orgId", "o1"))
        .respond_with(ok(json!([{ "id": "b", "orgId": "o1", "name": "Proteus", "lists": [{ "id": "l-b", "name": "tasks", "taskCount": 3 }] }])))
        .mount(&server)
        .await;

    let blank = core.tasks().raise(TaskDraft { list_id: "l-b".into(), title: " ".into(), ..TaskDraft::default() }).await;
    assert_eq!(code(blank.unwrap_err()), "empty-title");
    assert!(asked(&server, "POST", "/api/v1/task-lists/l-b/tasks").await.is_empty(), "mandó una tarea sin título");

    let draft = TaskDraft { list_id: "l-b".into(), title: "Subir el backend".into(), priority: "normal".into(), internal: true, ..TaskDraft::default() };
    let created = core.tasks().raise(draft).await.unwrap();

    assert_eq!((created.id.as_str(), created.priority.as_str()), ("t9", "normal"));
    let sent = asked(&server, "POST", "/api/v1/task-lists/l-b/tasks").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(
        serde_json::from_slice::<Value>(&sent[0].body).unwrap(),
        json!({ "title": "Subir el backend", "priority": "normal", "visibility": "internal" })
    );
    assert_eq!(core.tree().state().spaces[0].lists[0].task_count, 3, "el árbol no se volvió a pedir");
}

/// A quién se le puede dar una tarea nueva es el equipo de la organización
/// que se mira, por nombre; sin organización no se pregunta.
///
/// Roturas que atrapa: preguntar sin `orgId` (el servidor lo lee como «todas
/// las mías» y ofrecería gente de otro cliente); quedarse en las diez
/// personas que da sin tope; una lista sin orden; enseñar el usuario de quien
/// tiene nombre.
#[tokio::test]
async fn the_people_offered_are_the_current_organizations() {
    let (server, core) = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/search"))
        .and(query_param("orgId", "o1"))
        .and(query_param("limit", "50"))
        .respond_with(ok(json!([{ "id": "u2", "username": "luis" }, { "id": "u1", "username": "ana", "name": "Ana Pérez" }])))
        .mount(&server)
        .await;

    assert!(core.tasks().people_to_assign().await.is_empty());
    assert!(asked(&server, "GET", "/api/v1/users/search").await.is_empty(), "preguntó sin organización");

    mount_org(&server).await;
    core.orgs().load().await;
    let people = core.tasks().people_to_assign().await;

    let said: Vec<(&str, &str, bool)> = people.iter().map(|person| (person.id.as_str(), person.name.as_str(), person.chosen)).collect();
    assert_eq!(said, [("u1", "Ana Pérez", false), ("u2", "luis", false)]);
}

/// Lo que se ofrece sale del árbol cargado, de la organización que se mira y
/// de a qué está acotado «Mi trabajo».
///
/// Rotura que atrapa: una fila de «tarea nueva» que ignora la lista que se
/// está mirando.
#[tokio::test]
async fn where_it_can_go_follows_what_is_on_screen() {
    let (server, core) = rig().await;
    mount_org(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/task-spaces/"))
        .respond_with(ok(serde_json::to_value(spaces()).unwrap()))
        .mount(&server)
        .await;
    core.orgs().load().await;
    core.tree().load("o1").await;

    assert_eq!(core.tasks().destinations().options.len(), 4);

    core.my_work().set_scope(Some(scope(ScopeKind::List, "l-b")));
    let offered = core.tasks().destinations();
    assert!(offered.fixed);
    assert_eq!(said(&offered), [("l-b", "Proteus / tasks", true)]);

    core.my_work().set_scope(Some(scope(ScopeKind::Folder, "f2")));
    assert_eq!(said(&core.tasks().destinations()), [("l-f2", "Proteus / Plataforma / Infra / ops", false)]);
}
