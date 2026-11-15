//! Organizaciones y árbol, contra un servidor de mentira. Cada prueba dice la
//! rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{doc_key, Config, Core, Listener, NodeKind, OrgsState, Session, Storage, TreeState};
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
