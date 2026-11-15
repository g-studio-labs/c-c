//! Organizaciones y árbol, contra un servidor de mentira. Cada prueba dice la
//! rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{
    doc_key, Config, Connection, Core, Listener, Looking, NodeKind, OrgsState, ScopeKind, Session, Storage, TreeClick, TreeRow,
    TreeState,
};
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

#[derive(Default)]
struct Recorder {
    orgs: Mutex<Vec<OrgsState>>,
    trees: Mutex<Vec<TreeState>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn orgs_changed(&self, state: OrgsState) {
        self.orgs.lock().unwrap().push(state);
    }
    fn tree_changed(&self, state: TreeState) {
        self.trees.lock().unwrap().push(state);
    }
}

fn org(id: &str, zone: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "name": format!("Org {id}"), "slug": id, "role": "member", "memberCount": 2,
        "createdAt": "2026-06-09T12:00:00Z", "clientsSeeOnlyTheirSpace": false,
        "guestsCanUseDevTools": false, "doneNeedsSubtasksDone": false, "timezone": zone,
    })
}

fn space(id: &str, kind: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "id": id, "orgId": "o1", "name": format!("Espacio {id}"), "color": "#888", "kind": kind,
        "folders": [], "lists": [{ "id": format!("l-{id}"), "name": "tasks", "openCount": 1, "taskCount": 2 }],
        "people": [],
    })
}

struct Rig {
    server: MockServer,
    core: Core,
    storage: Arc<MemoryStorage>,
    recorder: Arc<Recorder>,
}

async fn rig() -> Rig {
    let server = MockServer::start().await;
    let storage = Arc::new(MemoryStorage::default());
    let recorder = Arc::new(Recorder::default());
    let core = Core::new(Config::new(server.uri()), storage.clone(), recorder.clone()).unwrap();
    let ana = serde_json::from_value(serde_json::json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    Rig { server, core, storage, recorder }
}

async fn mount_orgs(server: &MockServer, orgs: Vec<serde_json::Value>) {
    Mock::given(method("GET"))
        .and(path("/api/v1/organizations/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": orgs })))
        .mount(server)
        .await;
}

/// La primera vez se mira la primera organización, y se recuerda.
///
/// Rotura que atrapa: no elegir ninguna (todas las pantallas pedirían sin
/// organización); no guardar la elegida.
#[tokio::test]
async fn the_first_organization_is_chosen_and_remembered() {
    let rig = rig().await;
    mount_orgs(&rig.server, vec![org("o1", ""), org("o2", "")]).await;

    rig.core.orgs().load().await;

    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o1"));
    assert!(rig.storage.get("orgs").unwrap().contains("o1"));
}

/// Al volver a abrir se sigue en la que se estaba.
///
/// Rotura que atrapa: no leer la guardada; saltar a la primera en cada carga.
#[tokio::test]
async fn reopening_stays_on_the_organization_that_was_open() {
    let rig = rig().await;
    mount_orgs(&rig.server, vec![org("o1", ""), org("o2", "")]).await;
    rig.core.orgs().load().await;
    assert!(rig.core.orgs().set_current("o2"));

    let reopened = Core::new(Config::new(rig.server.uri()), rig.storage.clone(), Arc::new(Recorder::default())).unwrap();
    assert_eq!(reopened.orgs().state().current_org_id.as_deref(), Some("o2"), "antes de pedir nada");
    reopened.orgs().load().await;

    assert_eq!(reopened.orgs().state().current_org_id.as_deref(), Some("o2"));
}

/// Una organización que ya no es de esta persona no se queda puesta.
///
/// Rotura que atrapa: conservar la guardada aunque el servidor ya no la dé
/// (todas las pantallas pidiendo datos de una organización que se le niega).
#[tokio::test]
async fn an_organization_that_is_no_longer_theirs_is_dropped() {
    let rig = rig().await;
    rig.storage.set("orgs", r#"{"currentOrgId":"se-fue"}"#).unwrap();
    let core = Core::new(Config::new(rig.server.uri()), rig.storage.clone(), Arc::new(Recorder::default())).unwrap();
    mount_orgs(&rig.server, vec![org("o1", "")]).await;

    core.orgs().load().await;

    assert_eq!(core.orgs().state().current_org_id.as_deref(), Some("o1"));
}

/// No se puede cambiar a una organización que no está en la lista.
///
/// Rotura que atrapa: aceptar cualquier id.
#[tokio::test]
async fn switching_only_accepts_their_organizations() {
    let rig = rig().await;
    mount_orgs(&rig.server, vec![org("o1", "")]).await;
    rig.core.orgs().load().await;

    assert!(!rig.core.orgs().set_current("ajena"));
    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o1"));
}

/// La zona es la de la organización que se mira; sin poner, no hay.
///
/// Rotura que atrapa: dar la zona de otra organización; tratar la cadena vacía
/// como una zona.
#[tokio::test]
async fn the_zone_is_the_current_organizations() {
    let rig = rig().await;
    mount_orgs(&rig.server, vec![org("o1", ""), org("o2", "Pacific/Auckland")]).await;
    rig.core.orgs().load().await;

    assert_eq!(rig.core.orgs().state().current_zone(), None);
    rig.core.orgs().set_current("o2");
    assert_eq!(rig.core.orgs().state().current_zone(), Some("Pacific/Auckland"));
}

/// Un fallo al pedirlas no borra las que ya había, y lo dice.
///
/// Rotura que atrapa: vaciar la lista ante un fallo de red (la app se quedaría
/// «sin organizaciones» por un hipo del servidor).
#[tokio::test]
async fn a_failed_load_keeps_what_was_there() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/organizations/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": [org("o1", "")] })))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/organizations/"))
        .respond_with(ResponseTemplate::new(500).set_body_string(r#"{"success":false,"error":"internal","message":"Something broke"}"#))
        .mount(&rig.server)
        .await;

    rig.core.orgs().load().await;
    rig.core.orgs().load().await;

    let state = rig.core.orgs().state();
    assert_eq!(state.orgs.len(), 1);
    assert_eq!(state.error.as_deref(), Some("Something broke"));
    assert!(!state.loading);
    assert!(rig.recorder.orgs.lock().unwrap().len() >= 4, "cada cambio se anuncia");
}

/// El árbol se pide para una organización, y la sala general no sale.
///
/// Rotura que atrapa: pedirlo sin `orgId`; pintar la sala general, que es un
/// canal y no tiene tareas.
#[tokio::test]
async fn the_tree_is_asked_per_organization_without_the_general_room() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/task-spaces/"))
        .and(query_param("orgId", "o1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true, "data": [space("general", Some("general")), space("s1", None), space("s2", None)],
        })))
        .mount(&rig.server)
        .await;

    rig.core.tree().load("o1").await;

    let state = rig.core.tree().state();
    assert_eq!(state.spaces.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
    assert_eq!(state.spaces[0].lists[0].open_count, 1);
    assert_eq!(state.error, None);
}

/// Al cambiar de organización, el árbol de la anterior deja de enseñarse ya.
///
/// Rotura que atrapa: dejar el árbol viejo mientras llega el nuevo (un clic
/// ahí acotaría «Mi trabajo» a una lista de otra organización).
#[tokio::test]
async fn switching_organization_clears_the_old_tree_at_once() {
    let rig = rig().await;
    for (org, id) in [("o1", "s1"), ("o2", "s9")] {
        Mock::given(method("GET"))
            .and(path("/api/v1/task-spaces/"))
            .and(query_param("orgId", org))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": [space(id, None)] })))
            .mount(&rig.server)
            .await;
    }

    rig.core.tree().load("o1").await;
    rig.core.tree().load("o2").await;

    let seen = rig.recorder.trees.lock().unwrap();
    // El primer aviso de la segunda carga: cargando, y ya sin el árbol de o1.
    let switching = &seen[2];
    assert!(switching.loading);
    assert_eq!(switching.org_id.as_deref(), Some("o2"));
    assert!(switching.spaces.is_empty());
    assert_eq!(seen.last().unwrap().spaces[0].id, "s9");
}

/// Recargar la misma organización no parpadea: el árbol sigue mientras llega.
///
/// Rotura que atrapa: vaciar siempre al cargar.
#[tokio::test]
async fn reloading_the_same_organization_keeps_the_tree_meanwhile() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/task-spaces/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": [space("s1", None)] })))
        .mount(&rig.server)
        .await;

    rig.core.tree().load("o1").await;
    rig.core.tree().load("o1").await;

    let seen = rig.recorder.trees.lock().unwrap();
    assert!(seen[2].loading);
    assert_eq!(seen[2].spaces.len(), 1);
}

async fn mount_tree(server: &MockServer, org: &str, spaces: Vec<serde_json::Value>) {
    Mock::given(method("GET"))
        .and(path("/api/v1/task-spaces/"))
        .and(query_param("orgId", org))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": spaces })))
        .mount(server)
        .await;
}

async fn mount_doc_marks(server: &MockServer, org: &str, marks: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/api/v1/docs/"))
        .and(query_param("orgId", org))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": marks })))
        .mount(server)
        .await;
}

/// El árbol sabe qué nodos tienen documentación escrita, cada uno por su clase.
///
/// Roturas que atrapa: marcar la lista con la marca de un espacio del mismo
/// id (la clave lleva la clase); dar por escrito un documento que el servidor
/// dice que no lo está.
#[tokio::test]
async fn the_tree_knows_which_nodes_have_docs() {
    let rig = rig().await;
    mount_tree(&rig.server, "o1", vec![space("s1", None)]).await;
    mount_doc_marks(
        &rig.server,
        "o1",
        serde_json::json!({ "list:n1": { "written": true, "pages": 2 }, "space:n2": { "written": false } }),
    )
    .await;
    rig.core.tree().load("o1").await;

    rig.core.tree().load_doc_marks("o1").await;

    let state = rig.core.tree().state();
    assert_eq!(doc_key(NodeKind::Folder, "x"), "folder:x");
    assert_eq!(state.written_doc(NodeKind::List, "n1").and_then(|mark| mark.pages), Some(2));
    assert!(state.written_doc(NodeKind::Space, "n1").is_none(), "la marca es de la lista, no del espacio");
    assert!(state.written_doc(NodeKind::Space, "n2").is_none(), "está, pero sin nada escrito");
}

/// Las marcas son de una organización: al cambiar se quitan, las de una que
/// ya no se mira no se ponen, y si la petición falla se queda lo que había.
///
/// Roturas que atrapa: enseñar las marcas de la organización anterior sobre
/// el árbol de la nueva; que una respuesta tardía de la anterior pise a la
/// nueva; perder las marcas por un fallo pasajero de red.
#[tokio::test]
async fn doc_marks_belong_to_the_organization_on_screen() {
    let rig = rig().await;
    mount_tree(&rig.server, "o1", vec![space("s1", None)]).await;
    mount_tree(&rig.server, "o2", vec![space("s2", None)]).await;
    let marks = serde_json::json!({ "list:n1": { "written": true } });
    // Contesta una vez; la segunda petición ya no encuentra a nadie (404).
    Mock::given(method("GET"))
        .and(path("/api/v1/docs/"))
        .and(query_param("orgId", "o1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": marks })))
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;
    rig.core.tree().load("o1").await;
    rig.core.tree().load_doc_marks("o1").await;

    rig.core.tree().load_doc_marks("o1").await;
    assert_eq!(rig.core.tree().state().doc_marks.len(), 1, "un fallo deja lo que había");

    rig.core.tree().load("o2").await;
    assert!(rig.core.tree().state().doc_marks.is_empty(), "al cambiar de organización");

    mount_doc_marks(&rig.server, "o1", marks).await;
    rig.core.tree().load_doc_marks("o1").await;
    assert!(rig.core.tree().state().doc_marks.is_empty(), "la respuesta de la anterior no se pone");
}

// ── El árbol tal como se pinta ──────────────────────────────────────────

/// Un espacio con una carpeta dentro (que tiene una lista y otra carpeta con
/// su lista) y una lista suelta que es el canal de un cliente.
fn nested_space(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "orgId": "o1", "name": format!("Espacio {id}"), "color": "#20D9E8",
        "folders": [{
            "id": format!("f-{id}"), "name": "Carpeta",
            "folders": [{
                "id": format!("ff-{id}"), "name": "Dentro", "folders": [],
                "lists": [{ "id": format!("deep-{id}"), "name": "Honda", "openCount": 4, "taskCount": 4 }],
            }],
            "lists": [{ "id": format!("in-{id}"), "name": "En carpeta", "openCount": 2, "taskCount": 5 }],
        }],
        "lists": [{ "id": format!("l-{id}"), "name": "Suelta", "openCount": 1, "taskCount": 2, "projectId": "p1" }],
        "people": [],
    })
}

fn shape(rows: &[TreeRow]) -> Vec<(String, u32, bool)> {
    rows.iter().map(|row| (row.id.clone(), row.depth, row.open)).collect()
}

fn at(id: &str, depth: u32, open: bool) -> (String, u32, bool) {
    (id.to_string(), depth, open)
}

async fn loaded_tree(spaces: Vec<serde_json::Value>) -> Rig {
    let rig = rig().await;
    mount_tree(&rig.server, "o1", spaces).await;
    rig.core.tree().load("o1").await;
    rig
}

/// Sin nada elegido, los espacios van abiertos y las carpetas cerradas, con
/// las carpetas antes que las listas.
///
/// Roturas que atrapa: desplegarlo todo (lo que se busca queda tres pantallas
/// abajo); no enseñar ni las listas de un espacio; pintar una lista a la
/// altura de su espacio; contar como abiertas sólo las tareas de las listas
/// sueltas.
#[tokio::test]
async fn with_nothing_chosen_spaces_are_open_and_folders_closed() {
    let rig = loaded_tree(vec![nested_space("a")]).await;

    let rows = rig.core.tree().rows();

    assert_eq!(shape(&rows), [at("a", 0, true), at("f-a", 1, false), at("l-a", 1, false)]);
    assert_eq!(rows[0].open_count, 7, "las de todo lo que cuelga del espacio");
    assert_eq!(rows[1].open_count, 6, "las de todo lo que cuelga de la carpeta");
    assert!(rows[0].expandable && rows[1].expandable && !rows[2].expandable);
    assert!(rows[2].client && !rows[0].client, "la lista es el canal de un cliente");
    assert_eq!(rows[2].color, "#20D9E8");
}

/// Con «Mi trabajo» acotado a un nodo, va abierta su rama y sólo ésa.
///
/// Roturas que atrapa: dejar plegado el camino hasta lo que se está mirando
/// (no se ve dónde se está); abrir además los demás espacios; no marcar cuál
/// es el nodo activo, o marcar un espacio que comparte id con una lista.
#[tokio::test]
async fn the_branch_of_what_is_being_looked_at_is_the_open_one() {
    let rig = loaded_tree(vec![nested_space("a"), nested_space("b")]).await;

    rig.core.tree().click("deep-b", Looking::Tasks).unwrap();
    let rows = rig.core.tree().rows();

    assert_eq!(
        shape(&rows),
        [
            at("a", 0, false),
            at("b", 0, true),
            at("f-b", 1, true),
            at("ff-b", 2, true),
            at("deep-b", 3, false),
            at("in-b", 2, false),
            at("l-b", 1, false),
        ]
    );
    let active: Vec<&str> = rows.iter().filter(|row| row.active).map(|row| row.id.as_str()).collect();
    assert_eq!(active, ["deep-b"]);

    rig.core.tree().click("a", Looking::Tasks).unwrap();
    let rows = rig.core.tree().rows();
    assert_eq!(shape(&rows)[..3], [at("a", 0, true), at("f-a", 1, false), at("l-a", 1, false)]);
    assert!(rows[0].active);
    assert!(!rows.iter().any(|row| row.id == "b" && row.open));

    // Y al revés: acotado al segundo espacio, el primero se pliega.
    rig.core.tree().click("b", Looking::Tasks).unwrap();
    let open: Vec<String> = rig.core.tree().rows().into_iter().filter(|row| row.open).map(|row| row.id).collect();
    assert_eq!(open, ["b"]);
}

/// La rama abierta es sólo el camino hasta lo que se mira: ni las carpetas
/// de al lado, ni los otros espacios cuando lo acotado es una lista suelta.
///
/// Roturas que atrapa: abrir las carpetas hermanas por las que se pasó al
/// buscar; no reconocer como activa la rama de una lista que cuelga directa
/// del espacio (y dejar entonces todos los espacios abiertos); dar por activo
/// un espacio que comparte id con la lista acotada.
#[tokio::test]
async fn only_the_path_to_what_is_looked_at_opens() {
    let mut space = nested_space("a");
    space["folders"].as_array_mut().unwrap().insert(
        0,
        serde_json::json!({
            "id": "side", "name": "De al lado", "folders": [],
            "lists": [{ "id": "side-list", "name": "Otra", "openCount": 0, "taskCount": 0 }],
        }),
    );
    // Una lista con el mismo id que su espacio: los ids son de tablas
    // distintas y nada impide que coincidan.
    let mut twin = nested_space("b");
    twin["lists"][0]["id"] = serde_json::json!("b");
    let rig = loaded_tree(vec![space, twin]).await;
    let tree = rig.core.tree();

    tree.click("deep-a", Looking::Tasks).unwrap();
    let open: Vec<String> = tree.rows().into_iter().filter(|row| row.open).map(|row| row.id).collect();
    assert_eq!(open, ["a", "f-a", "ff-a"]);

    // Una lista suelta del segundo espacio, la que comparte id con él.
    rig.core.my_work().set_scope(Some(cac_core::WorkScope {
        kind: ScopeKind::List,
        id: "b".into(),
        name: "Suelta".into(),
        org_id: Some("o1".into()),
    }));
    let rows = tree.rows();
    let open: Vec<&str> = rows.iter().filter(|row| row.open).map(|row| row.id.as_str()).collect();
    assert_eq!(open, ["b"]);
    let active: Vec<NodeKind> = rows.iter().filter(|row| row.active).map(|row| row.kind).collect();
    assert_eq!(active, [NodeKind::List]);
}

/// Lo que se abre o se cierra a mano manda sobre la regla, y se recuerda por
/// organización.
///
/// Roturas que atrapa: un chevrón que no hace nada; que la rama activa se
/// vuelva a abrir sola después de plegarla; olvidarlo al reiniciar; que
/// plegar un espacio en una organización pliegue el del mismo id en otra.
#[tokio::test]
async fn what_is_opened_or_closed_by_hand_wins_and_is_remembered() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    let tree = rig.core.tree();

    tree.toggle("f-a");
    assert_eq!(shape(&tree.rows())[1], at("f-a", 1, true));
    tree.toggle("a");
    assert_eq!(shape(&tree.rows()), [at("a", 0, false)]);
    // Acotar a algo de dentro no lo vuelve a abrir: se cerró a mano.
    tree.click("l-a", Looking::Tasks).unwrap();
    assert_eq!(shape(&tree.rows()), [at("a", 0, false)]);
    tree.toggle("no-existe");
    assert_eq!(rig.core.tree().state().decided.len(), 2);
    // Una lista no se abre ni se cierra, aunque esté a la vista.
    tree.toggle("a");
    assert!(tree.rows().iter().any(|row| row.id == "l-a"));
    tree.toggle("l-a");
    assert!(!tree.state().decided.contains_key("l-a"));
    tree.toggle("a");

    // Otra organización con un espacio del mismo id: no hereda lo plegado.
    mount_tree(&rig.server, "o2", vec![nested_space("a")]).await;
    tree.load("o2").await;
    assert!(tree.rows()[0].open);

    // Y al volver —o al reiniciar la app— sigue como se dejó.
    let again = Core::new(Config::new(rig.server.uri()), rig.storage.clone(), rig.recorder.clone()).unwrap();
    again.tree().load("o1").await;
    assert_eq!(shape(&again.tree().rows()), [at("a", 0, false)]);
}

/// Un clic acota «Mi trabajo» a ese nodo, con su nombre y su organización, y
/// el árbol avisa para que se pinte cuál está activo.
///
/// Roturas que atrapa: acotar sin sello de organización (el ámbito no se
/// tiraría al cambiar de organización); acotar a una carpeta; no encontrar un
/// nodo que está bajo una rama plegada; que quien pinta el árbol no se entere
/// de que cambió el nodo activo.
#[tokio::test]
async fn a_click_scopes_my_work_to_that_node() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    let tree = rig.core.tree();
    let notices = rig.recorder.trees.lock().unwrap().len();

    // Bajo una carpeta que está cerrada.
    assert_eq!(tree.click("deep-a", Looking::Tasks), Some(TreeClick::ScopeTasks { kind: ScopeKind::List }));
    let scope = rig.core.my_work().state().scope.unwrap();
    assert_eq!((scope.kind, scope.id.as_str(), scope.name.as_str()), (ScopeKind::List, "deep-a", "Honda"));
    assert_eq!(scope.org_id.as_deref(), Some("o1"));
    assert!(rig.recorder.trees.lock().unwrap().len() > notices, "el árbol no avisó");

    assert_eq!(tree.click("f-a", Looking::Tasks), Some(TreeClick::OpenDocs));
    assert_eq!(rig.core.my_work().state().scope.unwrap().id, "deep-a", "una carpeta no acota");
    assert_eq!(tree.click("a", Looking::Docs), Some(TreeClick::OpenDocs));
    assert_eq!(tree.click("no-existe", Looking::Tasks), None);
}

/// Un nodo con documento escrito lo dice en su fila.
///
/// Rotura que atrapa: marcar una carpeta con la marca de una lista.
#[tokio::test]
async fn a_row_says_whether_its_node_has_docs() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    mount_doc_marks(
        &rig.server,
        "o1",
        serde_json::json!({ "list:l-a": { "written": true }, "folder:x": { "written": true } }),
    )
    .await;
    rig.core.tree().load_doc_marks("o1").await;

    let with_docs: Vec<String> = rig.core.tree().rows().into_iter().filter(|row| row.has_doc).map(|row| row.id).collect();
    assert_eq!(with_docs, ["l-a"]);
}

// ── Entrar a trabajar ───────────────────────────────────────────────────

async fn mount_tasks(server: &MockServer, org: &str, expect: u64) {
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/"))
        .and(query_param("orgId", org))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": [] })))
        .expect(expect)
        .mount(server)
        .await;
}

/// Entrar trae las organizaciones y, de la que se mira, el árbol, sus marcas
/// y «Mi trabajo».
///
/// Roturas que atrapa: una app que abre vacía hasta que alguien pulsa algo;
/// pedir el árbol de una organización que no es la que se mira; quedarse sin
/// las marcas de documentación.
#[tokio::test]
async fn entering_brings_the_organizations_and_what_belongs_to_the_current_one() {
    let rig = rig().await;
    mount_orgs(&rig.server, vec![org("o1", ""), org("o2", "")]).await;
    mount_tree(&rig.server, "o1", vec![space("a", None)]).await;
    mount_doc_marks(&rig.server, "o1", serde_json::json!({ "space:a": { "written": true } })).await;
    mount_tasks(&rig.server, "o1", 1).await;

    rig.core.open_workspace().await;

    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o1"));
    let tree = rig.core.tree().state();
    assert_eq!(tree.org_id.as_deref(), Some("o1"));
    assert_eq!(tree.spaces.len(), 1);
    assert!(tree.written_doc(NodeKind::Space, "a").is_some());
    assert_eq!(rig.core.my_work().state().org_id.as_deref(), Some("o1"));
    // Y deja el stream de eventos en marcha: sin él nada se pone al día solo.
    assert_ne!(rig.core.stream().state().status, Connection::Idle);
    rig.core.stream().stop();
}

/// Cambiar de organización trae lo de la nueva; una que no es suya no cambia
/// nada ni pide nada.
///
/// Roturas que atrapa: cambiar el nombre de arriba y seguir enseñando el
/// árbol y las tareas de la anterior; pedir datos de una organización ajena.
#[tokio::test]
async fn switching_organization_brings_what_belongs_to_the_new_one() {
    let rig = rig().await;
    mount_orgs(&rig.server, vec![org("o1", ""), org("o2", "")]).await;
    mount_tree(&rig.server, "o1", vec![space("a", None)]).await;
    mount_tree(&rig.server, "o2", vec![space("b", None), space("c", None)]).await;
    mount_tasks(&rig.server, "o1", 1).await;
    mount_tasks(&rig.server, "o2", 1).await;
    rig.core.open_workspace().await;
    rig.core.stream().stop();

    assert!(rig.core.switch_org("o2").await);

    assert_eq!(rig.core.orgs().state().current_org_id.as_deref(), Some("o2"));
    assert_eq!(rig.core.tree().state().spaces.len(), 2);
    assert_eq!(rig.core.my_work().state().org_id.as_deref(), Some("o2"));

    assert!(!rig.core.switch_org("o9").await);
    assert_eq!(rig.core.tree().state().org_id.as_deref(), Some("o2"));
    let asked_o9 = rig.server.received_requests().await.unwrap().iter().any(|request| request.url.query().unwrap_or("").contains("o9"));
    assert!(!asked_o9, "se pidió algo de una organización ajena");
}
