//! Basecamp en la organización contra un servidor de mentira. Cada prueba
//! dice la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{ApiError, Config, Core, Listener, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, method, path};
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

struct Quiet;

impl Listener for Quiet {
    fn session_changed(&self, _session: Option<Session>) {}
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn refused(status: u16, code: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({ "success": false, "error": code, "message": "No" }))
}

const BASE: &str = "/api/v1/organizations/o1/basecamp";

async fn rig() -> (MockServer, Core) {
    let server = MockServer::start().await;
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(Quiet)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    (server, core)
}

/// El estado se lee tal como lo manda el servidor, con lo de una cuenta
/// conectada; y con el servidor sin la app, `configured` falso y nada más.
///
/// Roturas que atrapa: perder el nombre de la cuenta o de quien conectó, el
/// fallo de renovar, los proyectos o la cuenta de personas sin casar; leer
/// «conectado» donde el servidor dijo que no.
#[tokio::test]
async fn the_status_is_read_as_the_server_sends_it() {
    let (server, core) = rig().await;
    Mock::given(method("GET"))
        .and(path(BASE))
        .respond_with(ok(json!({
            "configured": true, "connected": true, "accountId": 99, "accountName": "DWIT", "identityName": "Ana Ruiz",
            "refreshFailedAt": "2026-11-26T10:00:00Z",
            "projects": [{ "id": "l1", "orgId": "o1", "spaceId": "sp1", "projectId": 7, "name": "Sitio", "appUrl": "https://3.basecamp.com/99/projects/7", "webhookId": 55, "linkedById": "u1" }],
            "unmatched": 3,
        })))
        .mount(&server)
        .await;
    let status = core.orgs().basecamp_status("o1").await.unwrap();
    assert!(status.configured && status.connected);
    assert_eq!((status.account_name.as_str(), status.identity_name.as_str(), status.unmatched), ("DWIT", "Ana Ruiz", 3));
    assert!(status.needs_reconnect());
    assert_eq!((status.projects[0].project_id, status.projects[0].space_id.as_str()), (7, "sp1"));
    assert!(status.projects[0].has_webhook());

    server.reset().await;
    Mock::given(method("GET")).and(path(BASE)).respond_with(ok(json!({ "configured": false, "connected": false, "projects": [], "unmatched": 0 }))).mount(&server).await;
    let off = core.orgs().basecamp_status("o1").await.unwrap();
    assert!(!off.configured && !off.connected && !off.offers_connect(true));
}

/// Conectar pide la dirección al servidor y la devuelve tal cual; sin la app
/// en el servidor el error trae su etiqueta, para decirlo con claridad.
///
/// Roturas que atrapa: una dirección inventada aquí; tragarse el 503 y dar
/// una dirección vacía; cambiar la etiqueta `basecamp-off`.
#[tokio::test]
async fn connecting_returns_the_servers_address_or_says_it_is_off() {
    let (server, core) = rig().await;
    Mock::given(method("POST")).and(path(format!("{BASE}/connect"))).and(body_json(json!({}))).respond_with(ok(json!({ "url": "https://launchpad.37signals.com/authorization/new?state=s" }))).expect(1).mount(&server).await;
    assert_eq!(core.orgs().basecamp_connect_url("o1").await.unwrap(), "https://launchpad.37signals.com/authorization/new?state=s");

    server.reset().await;
    Mock::given(method("POST")).and(path(format!("{BASE}/connect"))).respond_with(refused(503, "basecamp-off")).mount(&server).await;
    let Err(ApiError::Server { status: 503, code, .. }) = core.orgs().basecamp_connect_url("o1").await else { panic!("no dio el 503") };
    assert_eq!(code, "basecamp-off");
}

/// Enlazar manda sólo el espacio, al proyecto de la ruta, y devuelve el
/// enlace con su webhook; quitarlo es un DELETE a ese mismo proyecto; y el
/// error del servidor llega con su etiqueta.
///
/// Roturas que atrapa: el cuerpo con otro nombre de campo o sin el espacio;
/// el proyecto equivocado en la ruta; enlazar con PUT cambiado por POST;
/// quitar sin DELETE; tragarse `space-outside-org`; perder el id del webhook.
#[tokio::test]
async fn linking_a_project_sends_its_space_and_unlinking_deletes_it() {
    let (server, core) = rig().await;
    Mock::given(method("PUT"))
        .and(path(format!("{BASE}/projects/7")))
        .and(body_json(json!({ "spaceId": "sp1" })))
        .respond_with(ok(json!({ "id": "l1", "spaceId": "sp1", "projectId": 7, "name": "Sitio", "appUrl": "https://3.basecamp.com/99/projects/7", "webhookId": 55 })))
        .expect(1)
        .mount(&server)
        .await;
    let linked = core.orgs().basecamp_link_project("o1", 7, "sp1").await.unwrap();
    assert_eq!((linked.project_id, linked.space_id.as_str(), linked.webhook_id), (7, "sp1", 55));

    Mock::given(method("DELETE")).and(path(format!("{BASE}/projects/7"))).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    core.orgs().basecamp_unlink_project("o1", 7).await.unwrap();

    server.reset().await;
    Mock::given(method("PUT")).and(path(format!("{BASE}/projects/8"))).respond_with(refused(400, "space-outside-org")).mount(&server).await;
    let Err(ApiError::Server { code, .. }) = core.orgs().basecamp_link_project("o1", 8, "other").await else { panic!("no falló") };
    assert_eq!(code, "space-outside-org");
}

/// Asignar manda el usuario elegido a la persona de la ruta, y soltarla
/// manda el usuario vacío (no omite el campo: el servidor lo exige).
///
/// Roturas que atrapa: el campo con otro nombre; asignar a la persona
/// equivocada; soltar sin mandar `userId` vacío; tragarse `user-outside-org`.
#[tokio::test]
async fn assigning_a_person_sends_the_user_and_releasing_sends_an_empty_one() {
    let (server, core) = rig().await;
    Mock::given(method("PUT"))
        .and(path(format!("{BASE}/people/31")))
        .and(body_json(json!({ "userId": "u2" })))
        .respond_with(ok(json!({ "id": "x", "personId": 31, "name": "Bob B", "email": "bob@acme.io", "client": false, "userId": "u2", "matchedBy": "manual" })))
        .expect(1)
        .mount(&server)
        .await;
    let assigned = core.orgs().basecamp_match_person("o1", 31, "u2").await.unwrap();
    assert_eq!((assigned.user_id.as_str(), assigned.matched_by.as_str()), ("u2", "manual"));

    server.reset().await;
    Mock::given(method("PUT"))
        .and(path(format!("{BASE}/people/31")))
        .and(body_json(json!({ "userId": "" })))
        .respond_with(ok(json!({ "personId": 31, "name": "Bob B", "userId": "" })))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(core.orgs().basecamp_match_person("o1", 31, "").await.unwrap().user_id, "");

    server.reset().await;
    Mock::given(method("PUT")).and(path(format!("{BASE}/people/32"))).respond_with(refused(400, "user-outside-org")).mount(&server).await;
    let Err(ApiError::Server { code, .. }) = core.orgs().basecamp_match_person("o1", 32, "u9").await else { panic!("no falló") };
    assert_eq!(code, "user-outside-org");
}

/// Los proyectos de la cuenta y las personas llegan como listas, también
/// cuando el servidor no escribe la lista vacía; y desconectar es un DELETE
/// a la raíz de Basecamp.
///
/// Roturas que atrapa: tomar una lista sin `data` por error; desconectar con
/// otro verbo o hacia otra ruta; perder `spaceId` (dice si ya está enlazado)
/// o `client`.
#[tokio::test]
async fn the_lists_arrive_and_disconnecting_deletes_the_root() {
    let (server, core) = rig().await;
    Mock::given(method("GET")).and(path(format!("{BASE}/projects"))).respond_with(ok(json!([{ "id": 7, "name": "Sitio", "appUrl": "https://x", "spaceId": "sp1" }, { "id": 8, "name": "App", "appUrl": "https://y" }]))).mount(&server).await;
    let projects = core.orgs().basecamp_projects("o1").await.unwrap();
    assert_eq!((projects[0].space_id.as_str(), projects[1].space_id.as_str()), ("sp1", ""));

    Mock::given(method("GET")).and(path(format!("{BASE}/people"))).respond_with(ok(json!([{ "id": "a", "personId": 31, "name": "Bob B", "email": "bob@x.io", "client": true, "userId": "" }]))).mount(&server).await;
    let people = core.orgs().basecamp_people("o1").await.unwrap();
    assert!(people[0].client && people[0].user_id.is_empty());

    Mock::given(method("DELETE")).and(path(BASE)).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    core.orgs().basecamp_disconnect("o1").await.unwrap();

    server.reset().await;
    Mock::given(method("GET")).and(path(format!("{BASE}/projects"))).respond_with(refused(409, "basecamp-not-connected")).mount(&server).await;
    let Err(ApiError::Server { code, .. }) = core.orgs().basecamp_projects("o1").await else { panic!("no falló") };
    assert_eq!(code, "basecamp-not-connected");
}
