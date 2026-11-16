//! Organizaciones y árbol, contra un servidor de mentira. Cada prueba dice la
//! rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{
    doc_key, DocsState, Config, Connection, Core, Listener, Looking, NodeKind, OrgsState, ScopeKind, Session, Storage, TreeClick, TreeRow,
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
    docs: Mutex<Vec<DocsState>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn orgs_changed(&self, state: OrgsState) {
        self.orgs.lock().unwrap().push(state);
    }
    fn tree_changed(&self, state: TreeState) {
        self.trees.lock().unwrap().push(state);
    }
    fn docs_changed(&self, state: DocsState) {
        self.docs.lock().unwrap().push(state);
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
    // Y el tono de su espacio, por si no tuviera color propio: el de «a».
    assert!(rows.iter().all(|row| row.hue == cac_core::hue_index("a")), "cada fila con el tono de su espacio");
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

// ── La documentación de un nodo ─────────────────────────────────────────

fn doc(tabs: &[(&str, &str)], decisions: serde_json::Value) -> serde_json::Value {
    let tabs: Vec<_> = tabs.iter().map(|(key, body)| serde_json::json!({ "key": key, "body": body })).collect();
    serde_json::json!({ "doc": null, "orgId": "o1", "tabs": tabs, "decisions": decisions, "attachments": [] })
}

async fn mount_doc(server: &MockServer, kind: &str, id: &str, doc: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/docs/{kind}/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "data": doc })))
        .mount(server)
        .await;
}

/// Abrir la documentación de un nodo la pide por su clase y su id, y la deja
/// para leer en su primera pestaña.
///
/// Roturas que atrapa: pedir la de una carpeta como si fuera una lista (el
/// servidor contesta la de otro nodo, o ninguna); abrir en una pestaña que no
/// es la primera; no avisar a quien pinta; abrir algo que no está en el árbol.
#[tokio::test]
async fn opening_a_nodes_docs_asks_for_them_by_kind_and_id() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    mount_doc(&rig.server, "folder", "f-a", doc(&[("overview", "  # La carpeta\n"), ("runbook", "pasos")], serde_json::json!([]))).await;

    rig.core.docs().open("f-a").await;

    let state = rig.core.docs().state();
    let target = state.target.clone().unwrap();
    assert_eq!((target.kind, target.id.as_str(), target.name.as_str()), (NodeKind::Folder, "f-a", "Carpeta"));
    assert_eq!(state.tab, "overview");
    assert_eq!(state.body(), "# La carpeta");
    assert!(!state.loading && state.error.is_none());
    assert_eq!(state.empty_reason(), None);
    assert_eq!(rig.recorder.docs.lock().unwrap().last(), Some(&state), "quien pinta no se enteró");

    rig.core.docs().open("no-existe").await;
    assert_eq!(rig.core.docs().state().target.unwrap().id, "f-a");
}

/// Cambiar de pestaña enseña su texto, y sólo existen las cuatro.
///
/// Roturas que atrapa: una pestaña que cambia de color y enseña lo mismo;
/// aceptar una clave inventada y quedarse leyendo nada.
#[tokio::test]
async fn a_tab_shows_its_own_text() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    mount_doc(&rig.server, "space", "a", doc(&[("overview", "qué es"), ("runbook", "cómo se levanta")], serde_json::json!([]))).await;
    rig.core.docs().open("a").await;

    rig.core.docs().show_tab("runbook");
    assert_eq!(rig.core.docs().state().body(), "cómo se levanta");

    rig.core.docs().show_tab("inventada");
    assert_eq!(rig.core.docs().state().tab, "runbook");
}

/// Las decisiones salen en su pestaña, y sólo las que siguen vigentes.
///
/// Roturas que atrapa: enseñar como vigente una decisión que otra dejó sin
/// efecto; repetir el registro bajo las cuatro pestañas.
#[tokio::test]
async fn only_standing_decisions_show_and_only_in_their_tab() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    let decisions = serde_json::json!([
        { "id": "d1", "title": "La vieja", "body": "", "decidedAt": "2026-09-30T00:00:00Z", "supersededById": "d2" },
        { "id": "d2", "title": "La nueva", "body": "porque sí", "decidedAt": "2026-10-30T00:00:00Z" },
    ]);
    mount_doc(&rig.server, "space", "a", doc(&[("overview", "qué es")], decisions)).await;
    rig.core.docs().open("a").await;
    assert!(rig.core.docs().state().standing_decisions().is_empty());

    rig.core.docs().show_tab("decisions");

    let standing: Vec<String> = rig.core.docs().state().standing_decisions().into_iter().map(|decision| decision.title).collect();
    assert_eq!(standing, ["La nueva"]);
    assert_eq!(rig.core.docs().state().empty_reason(), None, "con una decisión vigente, la pestaña no está vacía");
}

/// Una pestaña sin nada dice que no hay nada, y un fallo dice que falló.
///
/// Roturas que atrapa: una página en blanco que no distingue «vacío» de
/// «roto»; decir «no hay nada escrito» de un documento que no se pudo traer.
#[tokio::test]
async fn an_empty_tab_and_a_failed_load_say_which_they_are() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    mount_doc(&rig.server, "space", "a", doc(&[("overview", "   ")], serde_json::json!([]))).await;
    rig.core.docs().open("a").await;
    assert_eq!(rig.core.docs().state().empty_reason(), Some("work:docs.emptyTab"));
    rig.core.docs().show_tab("decisions");
    assert_eq!(rig.core.docs().state().empty_reason(), Some("work:decisions.empty"));

    // La de esta lista no está montada: el servidor contesta 404.
    rig.core.docs().open("l-a").await;
    let state = rig.core.docs().state();
    assert!(state.error.is_some() && !state.loading);
    assert_eq!(state.empty_reason(), Some("common:last.requestsFailing"));

    // Al volver a pedirla y llegar, el fallo de antes no se queda puesto.
    mount_doc(&rig.server, "list", "l-a", doc(&[("overview", "ya está")], serde_json::json!([]))).await;
    rig.core.docs().open("l-a").await;
    let state = rig.core.docs().state();
    assert_eq!((state.error.as_deref(), state.body()), (None, "ya está"));
}

/// Una respuesta que llega tarde, de un documento que ya no es el que se
/// mira, no se pone.
///
/// Rotura que atrapa: pulsar dos nodos seguidos y acabar leyendo el texto
/// del primero bajo el nombre del segundo.
#[tokio::test]
async fn a_late_answer_for_another_node_is_dropped() {
    let rig = loaded_tree(vec![nested_space("a"), nested_space("b")]).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/docs/space/a"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(300))
                .set_body_json(serde_json::json!({ "success": true, "data": doc(&[("overview", "lo de a")], serde_json::json!([])) })),
        )
        .mount(&rig.server)
        .await;
    mount_doc(&rig.server, "space", "b", doc(&[("overview", "lo de b")], serde_json::json!([]))).await;

    let slow = {
        let core = rig.core.clone();
        tokio::spawn(async move { core.docs().open("a").await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    rig.core.docs().open("b").await;
    slow.await.unwrap();

    let state = rig.core.docs().state();
    assert_eq!((state.target.as_ref().unwrap().id.as_str(), state.body()), ("b", "lo de b"));
}

/// Al abrir la de otro nodo no se queda a la vista la del anterior.
///
/// Rotura que atrapa: leer el runbook de un espacio bajo el nombre de otro
/// mientras llega el suyo (o para siempre, si el suyo falla).
#[tokio::test]
async fn another_nodes_docs_are_not_shown_under_this_ones_name() {
    let rig = loaded_tree(vec![nested_space("a"), nested_space("b")]).await;
    mount_doc(&rig.server, "space", "a", doc(&[("overview", "lo de a")], serde_json::json!([]))).await;
    rig.core.docs().open("a").await;
    rig.core.docs().show_tab("runbook");

    // La de `b` no está montada: falla, y lo de `a` no puede seguir ahí.
    rig.core.docs().open("b").await;

    let state = rig.core.docs().state();
    assert_eq!(state.target.as_ref().unwrap().id, "b");
    assert!(state.tabs.is_empty());
    assert_eq!(state.tab, "overview", "la pestaña elegida en otro documento no se hereda");
}

/// Desde las tareas, un espacio o una lista acotan y una carpeta abre su
/// documentación; con documentación abierta, cualquier nodo abre la suya.
///
/// Roturas que atrapa: sacar de la documentación a quien la lee; una carpeta
/// cuyo clic no hace nada; abrir documentación al acotar.
#[tokio::test]
async fn a_click_follows_what_is_being_looked_at() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    mount_doc(&rig.server, "folder", "f-a", doc(&[("overview", "carpeta")], serde_json::json!([]))).await;
    mount_doc(&rig.server, "list", "l-a", doc(&[("overview", "lista")], serde_json::json!([]))).await;
    assert_eq!(rig.core.looking(), Looking::Tasks);

    assert_eq!(rig.core.click_node("l-a").await, Some(TreeClick::ScopeTasks { kind: ScopeKind::List }));
    assert!(rig.core.docs().state().target.is_none(), "acotar abrió documentación");

    assert_eq!(rig.core.click_node("f-a").await, Some(TreeClick::OpenDocs));
    assert_eq!(rig.core.looking(), Looking::Docs);
    assert_eq!(rig.core.docs().state().body(), "carpeta");

    // Ya leyendo documentación, la lista abre la suya en vez de acotar.
    assert_eq!(rig.core.click_node("l-a").await, Some(TreeClick::OpenDocs));
    assert_eq!(rig.core.docs().state().body(), "lista");
}

/// El botón de documentación abre la del nodo acotado, sea espacio o lista, y
/// volver deja en las tareas de ese mismo nodo (command-and-control-372).
///
/// Roturas que atrapa: ofrecer la documentación sólo con una lista delante;
/// volver de la documentación de un nodo a las tareas de otro; que una
/// carpeta, que no acota, se lleve el ámbito que había.
#[tokio::test]
async fn the_docs_button_opens_the_scoped_nodes_docs_and_back_returns_to_its_tasks() {
    let rig = loaded_tree(vec![nested_space("a"), nested_space("b")]).await;
    for (kind, id) in [("space", "a"), ("space", "b"), ("folder", "f-b")] {
        mount_doc(&rig.server, kind, id, doc(&[("overview", id)], serde_json::json!([]))).await;
    }

    // Sin acotar no hay nodo del que colgar.
    rig.core.open_scope_docs().await;
    assert!(rig.core.docs().state().target.is_none());

    rig.core.click_node("a").await;
    rig.core.open_scope_docs().await;
    assert_eq!(rig.core.docs().state().body(), "a");

    // Leyendo, se pasa a la de otro espacio, y al volver las tareas son suyas.
    rig.core.click_node("b").await;
    rig.core.back_to_tasks();
    assert!(rig.core.docs().state().target.is_none());
    assert_eq!(rig.core.my_work().state().scope.unwrap().id, "b");

    rig.core.click_node("f-b").await;
    rig.core.back_to_tasks();
    assert_eq!(rig.core.looking(), Looking::Tasks);
    assert_eq!(rig.core.my_work().state().scope.unwrap().id, "b", "una carpeta no acota");
}

/// «Mi trabajo» entero quita el ámbito y cierra el documento, y cambiar de
/// organización también lo cierra.
///
/// Roturas que atrapa: pulsar «Mi trabajo» y seguir en un documento; quedarse
/// leyendo el documento de una organización dentro de otra.
#[tokio::test]
async fn showing_all_work_or_switching_organization_closes_the_docs() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    mount_orgs(&rig.server, vec![org("o1", ""), org("o2", "")]).await;
    mount_tree(&rig.server, "o2", vec![]).await;
    mount_doc(&rig.server, "space", "a", doc(&[("overview", "a")], serde_json::json!([]))).await;
    rig.core.orgs().load().await;
    rig.core.click_node("a").await;
    rig.core.open_scope_docs().await;

    rig.core.show_all_work();
    assert!(rig.core.docs().state().target.is_none());
    assert!(rig.core.my_work().state().scope.is_none());

    rig.core.click_node("a").await;
    rig.core.open_scope_docs().await;
    // Y una tarea abierta (no hace falta que llegue: basta con que lo esté).
    rig.core.tasks().open("t1").await;
    assert!(rig.core.switch_org("o2").await);
    assert!(rig.core.docs().state().target.is_none(), "el documento de la otra organización sigue abierto");
    assert!(rig.core.tasks().state().task_id.is_none(), "la tarea de la otra organización sigue abierta");
}

// ── El filtro por título del panel ──────────────────────────────────────

fn ids(rows: &[TreeRow]) -> Vec<&str> {
    rows.iter().map(|row| row.id.as_str()).collect()
}

/// Escribir parte del nombre de una lista honda la enseña con su camino
/// entero abierto, y nada de lo demás.
///
/// Roturas que atrapa: enseñar la lista sin las carpetas de las que cuelga
/// (no se sabe de qué espacio es); dejar su carpeta plegada, con lo que casa
/// dentro y sin verse; seguir pintando las listas que no casan; ofrecer un
/// chevrón que filtrando no pliega nada; distinguir mayúsculas; casar
/// sólo por el principio del nombre.
#[tokio::test]
async fn a_title_filter_shows_what_matches_with_its_path_open() {
    let rig = loaded_tree(vec![nested_space("a"), nested_space("b")]).await;
    let tree = rig.core.tree();
    // Plegado a mano: filtrando no manda.
    tree.toggle("a");

    let rows = tree.rows_matching(" HOND ");

    assert_eq!(
        shape(&rows),
        [
            at("a", 0, true),
            at("f-a", 1, true),
            at("ff-a", 2, true),
            at("deep-a", 3, false),
            at("b", 0, true),
            at("f-b", 1, true),
            at("ff-b", 2, true),
            at("deep-b", 3, false),
        ]
    );
    assert!(rows.iter().all(|row| !row.expandable), "ofrece plegar lo que filtrando va siempre abierto");
    // A mitad del nombre también: se escribe lo que se recuerda de él.
    assert_eq!(ids(&tree.rows_matching("uelt")), ["a", "l-a", "b", "l-b"]);
    assert!(tree.rows_matching("no existe").is_empty());
}

/// Los acentos no cuentan, ni en lo escrito ni en el nombre.
///
/// Rotura que atrapa: que «operacion» no encuentre «Operación», o al revés.
#[tokio::test]
async fn a_title_filter_ignores_accents() {
    let mut space = nested_space("a");
    space["lists"][0]["name"] = serde_json::json!("Operación");
    let rig = loaded_tree(vec![space]).await;
    let tree = rig.core.tree();

    assert_eq!(ids(&tree.rows_matching("operacion")), ["a", "l-a"]);
    assert_eq!(ids(&tree.rows_matching("ESPÁCIO")), ["a", "f-a", "ff-a", "deep-a", "in-a", "l-a"]);
}

/// Lo que casa enseña también lo que tiene dentro: un espacio, todo lo suyo;
/// una carpeta, sus listas y sus carpetas, bajo el espacio del que cuelga.
///
/// Roturas que atrapa: buscar un espacio por su nombre y verlo vacío; que
/// case una carpeta y no salgan sus listas, o salga sin su espacio; colar
/// las listas sueltas del espacio cuando lo que casó fue una carpeta.
#[tokio::test]
async fn what_matches_shows_what_it_holds() {
    let rig = loaded_tree(vec![nested_space("a"), nested_space("b")]).await;
    let tree = rig.core.tree();

    assert_eq!(ids(&tree.rows_matching("espacio b")), ["b", "f-b", "ff-b", "deep-b", "in-b", "l-b"]);
    assert_eq!(ids(&tree.rows_matching("dentro")), ["a", "f-a", "ff-a", "deep-a", "b", "f-b", "ff-b", "deep-b"]);
    // «Carpeta» casa la carpeta y la lista «En carpeta»: una vez cada una.
    assert_eq!(ids(&tree.rows_matching("carpeta"))[..5], ["a", "f-a", "ff-a", "deep-a", "in-a"]);
}

/// Con el filtro en blanco, el árbol de siempre: con lo plegado a mano.
///
/// Rotura que atrapa: que borrar lo escrito deje el árbol entero desplegado.
#[tokio::test]
async fn a_blank_filter_is_the_usual_tree() {
    let rig = loaded_tree(vec![nested_space("a")]).await;
    let tree = rig.core.tree();
    tree.toggle("a");

    assert_eq!(tree.rows_matching("   "), tree.rows());
    assert_eq!(shape(&tree.rows_matching("")), [at("a", 0, false)]);
    assert!(tree.rows()[0].expandable);
}
