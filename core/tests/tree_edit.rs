//! Crear, renombrar y borrar nodos del árbol, contra un servidor de mentira.
//!
//! Las formas de cada petición son las de `app/src/store/tasks.store.ts` y
//! las de los handlers de `backend/internal/adapters/handler/task.go`. Cada
//! prueba dice la rotura que atrapa, comprobada rompiendo el código
//! (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{
    can_edit_tree, clean_node_name, delete_notice, node_menu, text, tree_edit_phrase, ApiError, Config, Core, Locale, Looking, NoListener, NodeAction,
    NodeKind, ScopeKind, SpaceTree, Storage, TreeEditError, MAX_NODE_NAME,
};
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

/// Un espacio con una lista suelta (que ve un cliente), y una carpeta con
/// una lista y una subcarpeta con otra.
fn space(id: &str) -> Value {
    json!({
        "id": id, "orgId": "o1", "name": format!("Espacio {id}"), "color": "#20D9E8", "people": [],
        "folders": [{
            "id": format!("f-{id}"), "name": "Carpeta",
            "folders": [{
                "id": format!("ff-{id}"), "name": "Dentro", "folders": [],
                "lists": [{ "id": format!("deep-{id}"), "name": "Honda", "openCount": 4, "taskCount": 4 }],
            }],
            "lists": [{ "id": format!("in-{id}"), "name": "En carpeta", "openCount": 2, "taskCount": 5 }],
        }],
        "lists": [{ "id": format!("l-{id}"), "name": "Suelta", "openCount": 1, "taskCount": 2, "projectId": "p1" }],
    })
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

struct Rig {
    server: MockServer,
    core: Core,
    storage: Arc<MemoryStorage>,
}

impl Rig {
    /// Lo que se le pidió al servidor que no fue leer: verbo, ruta y cuerpo.
    async fn writes(&self) -> Vec<(String, String, Option<Value>)> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method.as_str() != "GET")
            .map(|request| (request.method.to_string(), request.url.path().to_string(), serde_json::from_slice(&request.body).ok()))
            .collect()
    }

    /// Cuántas veces se pidió esa ruta.
    async fn reads(&self, route: &str) -> usize {
        self.server.received_requests().await.unwrap().iter().filter(|request| request.method.as_str() == "GET" && request.url.path() == route).count()
    }
}

/// Ana dentro, con ese papel en la organización `o1`, sus organizaciones
/// cargadas y un servidor que acepta cualquier cambio en el árbol.
async fn rig_as(role: &str, superadmin: bool) -> Rig {
    let server = MockServer::start().await;
    let org = json!({ "id": "o1", "name": "Org", "slug": "o1", "role": role, "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org]))).mount(&server).await;
    for verb in ["POST", "PATCH", "DELETE"] {
        Mock::given(method(verb))
            .and(path_regex("^/api/v1/task-(spaces|folders|lists)/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "ok" })))
            .mount(&server)
            .await;
    }
    let storage = Arc::new(MemoryStorage::default());
    let core = Core::new(Config::new(server.uri()), storage.clone(), Arc::new(NoListener)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "superadmin": superadmin })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.orgs().load().await;
    Rig { server, core, storage }
}

/// Lo mismo, como miembro y con el árbol ya cargado: primero `before`, y a
/// partir de la segunda vez que se pida, `after`.
async fn rig_with(before: Vec<Value>, after: Vec<Value>) -> Rig {
    let rig = rig_as("member", false).await;
    mount_trees(&rig.server, before, after).await;
    rig.core.tree().load("o1").await;
    rig
}

async fn mount_trees(server: &MockServer, before: Vec<Value>, after: Vec<Value>) {
    let tree = || Mock::given(method("GET")).and(path("/api/v1/task-spaces/"));
    tree().respond_with(ok(Value::Array(before))).up_to_n_times(1).mount(server).await;
    tree().respond_with(ok(Value::Array(after))).mount(server).await;
}

fn names(core: &Core) -> Vec<String> {
    core.tree().state().spaces.iter().map(|space| space.name.clone()).collect()
}

fn spaces(tree: Value) -> Vec<SpaceTree> {
    serde_json::from_value(tree).unwrap()
}

// ── Las reglas ──────────────────────────────────────────────────────────

/// El menú de una fila ofrece lo que se puede hacer con esa clase de nodo, y
/// a quien sólo mira, sólo la documentación.
///
/// Roturas que atrapa: ofrecer «carpeta nueva» dentro de una lista, que
/// guarda tareas y no nodos; no dejar crear dentro de una carpeta; ofrecer
/// borrar o renombrar a quien el servidor se lo va a negar; dejar a un
/// superadmin sin poder tocar una organización de la que no es miembro;
/// quitarle la documentación a quien sólo mira; tomar por bueno un papel que
/// no se conoce.
#[test]
fn the_menu_offers_what_each_node_and_role_can_do() {
    use NodeAction::{Delete, NewFolder, NewList, OpenDocs, Rename};
    assert_eq!(node_menu(NodeKind::Space, true), [OpenDocs, NewFolder, NewList, Rename, Delete]);
    assert_eq!(node_menu(NodeKind::Folder, true), [OpenDocs, NewFolder, NewList, Rename, Delete]);
    assert_eq!(node_menu(NodeKind::List, true), [OpenDocs, Rename, Delete]);
    for kind in [NodeKind::Space, NodeKind::Folder, NodeKind::List] {
        assert_eq!(node_menu(kind, false), [OpenDocs]);
    }

    assert!(can_edit_tree("admin", false) && can_edit_tree("member", false));
    assert!(!can_edit_tree("viewer", false));
    assert!(!can_edit_tree("", false) && !can_edit_tree("owner", false));
    assert!(can_edit_tree("viewer", true) && can_edit_tree("", true));
}

/// Un nombre se manda limpio, o no se manda.
///
/// Roturas que atrapa: crear un nodo sin nombre, o con uno de espacios;
/// mandar los espacios que se colaron a los lados; mandar un nombre que el
/// servidor va a rechazar por largo; contar bytes y no letras, y rechazar
/// uno con acentos que sí cabe.
#[test]
fn a_name_is_sent_clean_or_not_at_all() {
    assert_eq!(clean_node_name("  Diseño  "), Ok("Diseño".to_string()));
    assert_eq!(clean_node_name(""), Err(TreeEditError::EmptyName));
    assert_eq!(clean_node_name(" \t "), Err(TreeEditError::EmptyName));
    let longest = "ñ".repeat(MAX_NODE_NAME);
    assert_eq!(clean_node_name(&longest), Ok(longest.clone()));
    assert_eq!(clean_node_name(&format!("{longest}x")), Err(TreeEditError::NameTooLong));
    assert_eq!(MAX_NODE_NAME, 120);
}

/// Antes de borrar se pregunta nombrando el nodo y diciendo qué se va con
/// él; y lo que no se puede borrar, se dice por qué.
///
/// Roturas que atrapa: una pregunta que no dice qué se borra; callar cuántas
/// carpetas, listas y tareas se lleva un espacio (o contar sólo las de
/// arriba, sin las de las carpetas); «1 tareas»; decir de una carpeta que
/// borra tareas, o de una lista que no; no avisar de que lo ve un cliente;
/// dejar borrar una carpeta con carpetas dentro, que se quedarían fuera del
/// árbol; preguntar por un nodo que ya no está, o por una lista tomándola
/// por una carpeta.
#[test]
fn the_delete_question_names_the_node_and_what_goes_with_it() {
    let mut bound = space("a");
    bound["projectId"] = json!("p1");
    let empty = json!({ "id": "e", "orgId": "o1", "name": "Vacío", "color": "", "folders": [], "lists": [], "people": [] });
    let tree = spaces(json!([bound, space("b"), empty]));
    let ask = |kind, id: &str| delete_notice(&tree, kind, id, Locale::Es);

    let space_a = ask(NodeKind::Space, "a").unwrap();
    assert_eq!(space_a.title, "¿Borrar el espacio «Espacio a»?");
    assert_eq!(
        space_a.body,
        "Borra sus carpetas, sus listas y todas las tareas de dentro. No se puede deshacer.\n2 carpetas · 3 listas · 11 tareas\nUn cliente ve este espacio"
    );
    assert_eq!((space_a.confirm.as_str(), space_a.allowed), ("Borrar el espacio", true));
    assert!(!ask(NodeKind::Space, "b").unwrap().body.contains("cliente"));
    assert!(ask(NodeKind::Space, "e").unwrap().body.ends_with("No se puede deshacer.\nVacío"));

    // Una carpeta sin carpetas dentro: sus listas suben, no se borra nada.
    let inner = ask(NodeKind::Folder, "ff-a").unwrap();
    assert_eq!(inner.title, "¿Borrar la carpeta «Dentro»?");
    assert_eq!(inner.body, "Sus listas suben al espacio — no se borra ninguna tarea.\n1 lista");
    assert_eq!((inner.confirm.as_str(), inner.allowed), ("Borrar la carpeta", true));
    // Con carpetas dentro no se deja.
    let outer = ask(NodeKind::Folder, "f-a").unwrap();
    assert!(!outer.allowed);
    assert_eq!(outer.body, text(Locale::Es, "work:tree.folderHasFolders"));
    assert_ne!(outer.body, "work:tree.folderHasFolders");

    let seen = ask(NodeKind::List, "l-a").unwrap();
    assert_eq!(seen.title, "¿Borrar la lista «Suelta»?");
    assert_eq!(seen.body, "Borra sus 2 tareas y sus columnas. No se puede deshacer.\nUn cliente ve esta lista");
    assert_eq!((seen.confirm.as_str(), seen.allowed), ("Borrar la lista", true));
    assert_eq!(ask(NodeKind::List, "deep-a").unwrap().body, "Borra sus 4 tareas y sus columnas. No se puede deshacer.");

    assert_eq!(ask(NodeKind::List, "no-existe"), None);
    assert_eq!(ask(NodeKind::Folder, "l-a"), None);
    assert_eq!(ask(NodeKind::Space, "f-a"), None);
    // Y en el idioma de quien mira.
    assert_eq!(delete_notice(&tree, NodeKind::Folder, "ff-a", Locale::En).unwrap().body, "Its lists move up to the space — no tasks are deleted.\n1 list");
}

/// Cada motivo para no hacer un cambio se dice con una frase del catálogo.
///
/// Roturas que atrapa: enseñar una clave cruda, o la etiqueta del error;
/// decir «no se pudo» sin más cuando el servidor explicó por qué; no caer a
/// la frase de quien llamaba cuando el servidor manda un código que el
/// catálogo no conoce.
#[test]
fn every_refusal_is_said_in_words() {
    let said = |error: &TreeEditError| tree_edit_phrase(Locale::Es, error, "common:admin.errDelete");
    assert_eq!(said(&TreeEditError::NameTooLong), "El nombre no puede pasar de 120 caracteres.");
    assert_eq!(said(&TreeEditError::ReadOnly), "Aquí puedes mirar, pero no cambiar nada.");
    assert_eq!(said(&TreeEditError::NoOrganization), "Elige antes una organización");
    assert_eq!(said(&TreeEditError::NodeGone), "Eso ya no está en el árbol.");
    assert_eq!(said(&TreeEditError::FolderHasFolders), text(Locale::Es, "work:tree.folderHasFolders"));
    assert_eq!(said(&TreeEditError::EmptyName), "");
    let server = |code: &str| TreeEditError::Api(ApiError::Server { status: 409, code: code.into(), message: "List in use".into() });
    assert_eq!(said(&server("list-in-use-by-channel")), "Hay un canal apuntando a esta lista.");
    assert_eq!(said(&server("a report project delivers into this list")), "No se pudo borrar");
    assert_eq!(tree_edit_phrase(Locale::En, &TreeEditError::NodeGone, "common:admin.errDelete"), "That is no longer in the tree.");
}

// ── Las llamadas ────────────────────────────────────────────────────────

/// Un espacio nuevo se crea en la organización que se mira, con el nombre
/// limpio, y el árbol se vuelve a pedir.
///
/// Roturas que atrapa: crearlo sin decir en qué organización, o en otra;
/// mandar el nombre con sus espacios; no recargar (el espacio existe y no se
/// ve); pintar uno inventado aquí en vez de lo que el servidor guardó.
#[tokio::test]
async fn a_new_space_goes_to_the_current_organization_and_the_tree_reloads() {
    let mut created = space("n");
    created["name"] = json!("Como lo guardó el servidor");
    let rig = rig_with(vec![space("a")], vec![space("a"), created]).await;

    rig.core.tree().create_space("  Diseño  ").await.unwrap();

    assert_eq!(rig.writes().await, [("POST".to_string(), "/api/v1/task-spaces/".to_string(), Some(json!({ "orgId": "o1", "name": "Diseño" })))]);
    assert_eq!(rig.reads("/api/v1/task-spaces/").await, 2, "el árbol no se volvió a pedir");
    assert_eq!(names(&rig.core), ["Espacio a", "Como lo guardó el servidor"]);
}

/// Una carpeta o una lista nueva se crea por la ruta de su **espacio**,
/// diciendo en el cuerpo de qué carpeta cuelga (o `null`); y dentro de una
/// lista no se crea nada.
///
/// Roturas que atrapa: pedirle la ruta a la carpeta, que no la tiene; crear
/// en el espacio lo que se pidió dentro de una carpeta (o al revés); no
/// encontrar el espacio de una carpeta que cuelga de otra; confundir el
/// campo de la carpeta (`parentFolderId`) con el de la lista (`folderId`);
/// omitir el campo en vez de mandar `null`; mandar algo cuando el sitio ya no
/// existe o es una lista; no recargar tras cada una.
#[tokio::test]
async fn a_new_folder_or_list_goes_through_its_space() {
    let rig = rig_with(vec![space("a"), space("b")], vec![space("a"), space("b")]).await;
    let tree = rig.core.tree();

    tree.create_in(NodeKind::Space, "b", NodeKind::Folder, "Diseño").await.unwrap();
    tree.create_in(NodeKind::Space, "b", NodeKind::List, " Pendientes ").await.unwrap();
    tree.create_in(NodeKind::Folder, "ff-b", NodeKind::Folder, "Más dentro").await.unwrap();
    tree.create_in(NodeKind::Folder, "ff-b", NodeKind::List, "Al fondo").await.unwrap();

    let post = |route: &str, body: Value| ("POST".to_string(), route.to_string(), Some(body));
    assert_eq!(
        rig.writes().await,
        [
            post("/api/v1/task-spaces/b/folders", json!({ "name": "Diseño", "parentFolderId": null })),
            post("/api/v1/task-spaces/b/lists", json!({ "name": "Pendientes", "folderId": null })),
            post("/api/v1/task-spaces/b/folders", json!({ "name": "Más dentro", "parentFolderId": "ff-b" })),
            post("/api/v1/task-spaces/b/lists", json!({ "name": "Al fondo", "folderId": "ff-b" })),
        ]
    );
    assert_eq!(rig.reads("/api/v1/task-spaces/").await, 5, "no se recargó tras cada una");

    assert_eq!(tree.create_in(NodeKind::List, "l-b", NodeKind::List, "Dentro de una lista").await, Err(TreeEditError::NodeGone));
    assert_eq!(tree.create_in(NodeKind::Space, "b", NodeKind::Space, "Un espacio dentro de otro").await, Err(TreeEditError::NodeGone));
    assert_eq!(tree.create_in(NodeKind::Folder, "no-existe", NodeKind::List, "Huérfana").await, Err(TreeEditError::NodeGone));
    // Un id de lista no vale como carpeta.
    assert_eq!(tree.create_in(NodeKind::Folder, "l-b", NodeKind::List, "Huérfana").await, Err(TreeEditError::NodeGone));
    assert_eq!(tree.create_in(NodeKind::Space, "b", NodeKind::List, "   ").await, Err(TreeEditError::EmptyName));
    assert_eq!(rig.writes().await.len(), 4, "se mandó algo que la regla ya había rechazado");
}

/// Renombrar manda sólo el nombre al nodo de su clase, recarga, y la
/// pastilla de «Mi trabajo» pasa a decir el nombre nuevo; el mismo nombre no
/// se manda.
///
/// Roturas que atrapa: renombrar una lista por la ruta de las carpetas;
/// mandar además el color o el canal, que el servidor entonces pisaría;
/// gastar una petición para dejarlo igual; seguir pintando el nombre viejo
/// en el árbol, en la pastilla o en la cabecera de su documentación abierta;
/// mandar un nombre en blanco, o el de un nodo que ya no está.
#[tokio::test]
async fn renaming_sends_only_the_name_and_everything_follows() {
    let mut renamed = space("a");
    renamed["lists"][0]["name"] = json!("Entrada");
    let rig = rig_with(vec![space("a")], vec![renamed]).await;
    let doc = json!({ "doc": null, "orgId": "o1", "attachments": [], "tabs": [{ "key": "overview", "body": "Algo" }], "decisions": [] });
    Mock::given(method("GET")).and(path("/api/v1/docs/list/l-a")).respond_with(ok(doc)).mount(&rig.server).await;
    let tree = rig.core.tree();
    tree.click("l-a", Looking::Tasks).unwrap();
    rig.core.docs().open("l-a").await;
    assert_eq!(rig.core.my_work().state().scope.unwrap().name, "Suelta");

    tree.rename(NodeKind::List, "l-a", " Entrada ").await.unwrap();

    let patch = |route: &str, body: Value| ("PATCH".to_string(), route.to_string(), Some(body));
    assert_eq!(rig.writes().await, [patch("/api/v1/task-lists/l-a", json!({ "name": "Entrada" }))]);
    assert_eq!(rig.reads("/api/v1/task-spaces/").await, 2, "el árbol no se volvió a pedir");
    assert_eq!(tree.rows().iter().find(|row| row.id == "l-a").unwrap().name, "Entrada");
    let scope = rig.core.my_work().state().scope.unwrap();
    assert_eq!((scope.kind, scope.name.as_str()), (ScopeKind::List, "Entrada"));
    assert_eq!(rig.core.docs().state().target.map(|target| target.name).as_deref(), Some("Entrada"), "la documentación abierta sigue con el nombre viejo");

    tree.rename(NodeKind::Folder, "f-a", "Clientes").await.unwrap();
    tree.rename(NodeKind::Space, "a", "Producto").await.unwrap();
    assert_eq!(
        rig.writes().await[1..],
        [patch("/api/v1/task-folders/f-a", json!({ "name": "Clientes" })), patch("/api/v1/task-spaces/a", json!({ "name": "Producto" }))]
    );

    // El que ya tiene, con espacios de más o sin ellos: nada que mandar.
    tree.rename(NodeKind::List, "l-a", " Entrada ").await.unwrap();
    assert_eq!(tree.rename(NodeKind::List, "l-a", "  ").await, Err(TreeEditError::EmptyName));
    assert_eq!(tree.rename(NodeKind::List, "no-existe", "Otro").await, Err(TreeEditError::NodeGone));
    assert_eq!(tree.rename(NodeKind::Folder, "l-a", "Otro").await, Err(TreeEditError::NodeGone));
    assert_eq!(rig.writes().await.len(), 3, "se mandó algo que no había que mandar");
}

/// Borrar va al nodo de su clase, recarga el árbol y se lleva lo que
/// colgaba de él: el ámbito de «Mi trabajo», sus tareas ya cargadas y la
/// documentación que se estuviera leyendo.
///
/// Roturas que atrapa: borrar una lista por la ruta de los espacios (¡se
/// llevaría el espacio de ese id!); seguir pintando el nodo borrado; quedarse
/// acotado a él (pantalla vacía con su pastilla); seguir enseñando las
/// tareas de la lista borrada; seguir leyendo la documentación de algo que
/// ya no existe; volver a pedir «Mi trabajo» por una carpeta, que no borra
/// tareas.
#[tokio::test]
async fn deleting_goes_to_the_node_and_takes_what_hung_from_it() {
    let mut without_list = space("a");
    without_list["lists"] = json!([]);
    let rig = rig_as("member", false).await;
    mount_trees(&rig.server, vec![space("a")], vec![without_list]).await;
    Mock::given(method("GET")).and(path("/api/v1/tasks/")).respond_with(ok(json!([]))).mount(&rig.server).await;
    let doc = json!({ "doc": null, "orgId": "o1", "attachments": [], "tabs": [{ "key": "overview", "body": "Algo" }], "decisions": [] });
    Mock::given(method("GET")).and(path("/api/v1/docs/list/l-a")).respond_with(ok(doc)).mount(&rig.server).await;
    let (tree, work) = (rig.core.tree(), rig.core.my_work());
    tree.load("o1").await;
    work.load(Some("o1".into())).await;
    tree.click("l-a", Looking::Tasks).unwrap();
    rig.core.docs().open("l-a").await;
    assert!(rig.core.docs().state().target.is_some() && work.state().scope.is_some());

    tree.delete(NodeKind::List, "l-a").await.unwrap();

    assert_eq!(rig.writes().await, [("DELETE".to_string(), "/api/v1/task-lists/l-a".to_string(), None)]);
    assert!(tree.rows().iter().all(|row| row.id != "l-a"), "la lista borrada sigue en el árbol");
    assert_eq!(work.state().scope, None, "sigue acotado a la lista borrada");
    assert_eq!(rig.core.docs().state().target, None, "sigue abierta la documentación de la lista borrada");
    assert_eq!(rig.reads("/api/v1/tasks/").await, 2, "no se volvieron a pedir las tareas");

    // Una carpeta sin carpetas dentro: no borra tareas, y no se vuelven a pedir.
    tree.delete(NodeKind::Folder, "ff-a").await.unwrap();
    tree.delete(NodeKind::Space, "a").await.unwrap();
    let delete = |route: &str| ("DELETE".to_string(), route.to_string(), None);
    assert_eq!(rig.writes().await[1..], [delete("/api/v1/task-folders/ff-a"), delete("/api/v1/task-spaces/a")]);
    assert_eq!(rig.reads("/api/v1/tasks/").await, 3, "una carpeta borrada volvió a pedir las tareas, o un espacio no");
    assert_eq!(rig.reads("/api/v1/task-spaces/").await, 4);
}

/// Una carpeta con carpetas dentro no se manda a borrar, ni un nodo que ya
/// no está.
///
/// Roturas que atrapa: mandar el borrado de una carpeta cuyas subcarpetas el
/// servidor dejaría colgando de nada, fuera del árbol con sus listas; mandar
/// el de un id que no es de esa clase.
#[tokio::test]
async fn a_folder_with_folders_inside_is_not_sent_for_deletion() {
    let rig = rig_with(vec![space("a")], vec![space("a")]).await;
    let tree = rig.core.tree();

    assert_eq!(tree.delete(NodeKind::Folder, "f-a").await, Err(TreeEditError::FolderHasFolders));
    assert_eq!(tree.delete(NodeKind::Folder, "l-a").await, Err(TreeEditError::NodeGone));
    assert_eq!(tree.delete(NodeKind::Space, "no-existe").await, Err(TreeEditError::NodeGone));

    assert!(rig.writes().await.is_empty(), "se mandó un borrado que la regla ya había rechazado");
    assert_eq!(rig.reads("/api/v1/task-spaces/").await, 1);
}

/// Quien sólo puede mirar no manda nada; un superadmin sí puede, sea cual
/// sea su papel; y sin árbol cargado no hay dónde crear.
///
/// Roturas que atrapa: mandar un cambio que el servidor va a rechazar con un
/// 403 (y enseñar el menú que lo permite); dejar a un superadmin sin poder
/// arreglar una organización en la que sólo mira; crear un espacio sin
/// saber en qué organización.
#[tokio::test]
async fn a_viewer_sends_nothing() {
    let rig = rig_as("viewer", false).await;
    mount_trees(&rig.server, vec![space("a")], vec![space("a")]).await;
    let tree = rig.core.tree();
    tree.load("o1").await;
    assert!(!tree.can_edit());

    assert_eq!(tree.create_space("Nuevo").await, Err(TreeEditError::ReadOnly));
    assert_eq!(tree.create_in(NodeKind::Space, "a", NodeKind::List, "Nueva").await, Err(TreeEditError::ReadOnly));
    assert_eq!(tree.rename(NodeKind::List, "l-a", "Otra").await, Err(TreeEditError::ReadOnly));
    assert_eq!(tree.delete(NodeKind::List, "l-a").await, Err(TreeEditError::ReadOnly));
    assert!(rig.writes().await.is_empty(), "quien sólo mira mandó un cambio");

    let root = rig_as("viewer", true).await;
    assert!(root.core.tree().can_edit());
    // Con permiso pero sin árbol cargado: no se sabe en qué organización.
    assert_eq!(root.core.tree().create_space("Nuevo").await, Err(TreeEditError::NoOrganization));
    assert!(root.writes().await.is_empty());
    mount_trees(&root.server, vec![space("a")], vec![space("a")]).await;
    root.core.tree().load("o1").await;
    root.core.tree().create_space("Nuevo").await.unwrap();
    assert_eq!(root.writes().await.len(), 1);
}

/// Si el servidor dice que no, se devuelve su motivo y no se recarga nada.
///
/// Roturas que atrapa: dar por hecho un cambio que el servidor rechazó;
/// perder el código con el que se dice el motivo; recargar el árbol (y
/// «Mi trabajo») por un cambio que no ocurrió.
#[tokio::test]
async fn a_refusal_from_the_server_comes_back_with_its_reason() {
    let rig = rig_as("member", false).await;
    // Montado después: wiremock contesta con el último que casa.
    let refusal = ResponseTemplate::new(409).set_body_json(json!({ "success": false, "error": "list-in-use-by-channel", "message": "List in use" }));
    rig.server.reset().await;
    Mock::given(method("DELETE")).and(path("/api/v1/task-lists/l-a")).respond_with(refusal).mount(&rig.server).await;
    mount_trees(&rig.server, vec![space("a")], vec![]).await;
    rig.core.tree().load("o1").await;

    let answer = rig.core.tree().delete(NodeKind::List, "l-a").await;

    let Err(TreeEditError::Api(ApiError::Server { status, code, .. })) = &answer else { panic!("no es el rechazo del servidor: {answer:?}") };
    assert_eq!((*status, code.as_str()), (409, "list-in-use-by-channel"));
    assert_eq!(tree_edit_phrase(Locale::Es, &answer.unwrap_err(), "common:admin.errDelete"), "Hay un canal apuntando a esta lista.");
    assert_eq!(rig.reads("/api/v1/task-spaces/").await, 1, "se recargó el árbol por un cambio que no ocurrió");
    assert_eq!(names(&rig.core), ["Espacio a"]);
}

/// Abrir una rama para crear dentro la deja abierta, también si está vacía,
/// y se recuerda como lo abierto a mano.
///
/// Roturas que atrapa: crear dentro de una carpeta plegada sin ver dónde
/// nace; no poder abrir un nodo vacío (no tiene chevrón); que «abrir» cierre
/// lo que ya estaba abierto; olvidarlo al volver a arrancar.
#[tokio::test]
async fn opening_a_branch_to_create_inside_is_remembered() {
    let empty = json!({ "id": "e", "orgId": "o1", "name": "Vacío", "color": "", "folders": [{ "id": "fe", "name": "Hueca", "folders": [], "lists": [] }], "lists": [], "people": [] });
    let rig = rig_with(vec![space("a"), empty.clone()], vec![space("a"), empty]).await;
    let tree = rig.core.tree();
    let open = |core: &Core, id: &str| core.tree().rows().iter().find(|row| row.id == id).map(|row| row.open);
    assert_eq!(open(&rig.core, "f-a"), Some(false));
    assert_eq!(open(&rig.core, "fe"), Some(false));

    tree.open("f-a");
    tree.open("fe");
    tree.open("a");

    assert_eq!(open(&rig.core, "f-a"), Some(true));
    assert!(tree.rows().iter().any(|row| row.id == "in-a"), "lo de dentro no se ve");
    assert_eq!(open(&rig.core, "fe"), Some(true), "un nodo vacío no se abrió");
    assert_eq!(open(&rig.core, "a"), Some(true), "abrir cerró lo que ya estaba abierto");

    let again = Core::new(Config::new(rig.server.uri()), rig.storage.clone(), Arc::new(NoListener)).unwrap();
    again.tree().load("o1").await;
    assert_eq!(open(&again, "f-a"), Some(true), "al volver a arrancar la carpeta está plegada");
}
