//! Un enlace de cac abre la app en su sitio, contra un servidor de mentira.
//! Cada prueba dice la rotura que atrapa, comprobada rompiendo el código
//! (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{app_link, app_route, launch_landing, Config, Conversation, Core, Landing, LinkOpened, NoListener, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, path_regex, query_param};
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

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn get(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path(route.to_string()))
}

fn space(id: &str, org: &str, folders: Value) -> Value {
    json!({ "id": id, "orgId": org, "name": format!("Espacio {id}"), "color": "#888", "kind": null, "people": [], "folders": folders, "lists": [] })
}

/// Dos organizaciones: la que se mira, `o1` (espacio `a`), y `o2` (espacio
/// `z` con la carpeta `f2`, y el directo `c9`).
async fn rig() -> (MockServer, Core) {
    let server = MockServer::start().await;
    let org = |id: &str| json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" });
    get("/api/v1/organizations/").respond_with(ok(json!([org("o1"), org("o2")]))).mount(&server).await;
    get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(json!([space("a", "o1", json!([]))]))).mount(&server).await;
    let folder = json!([{ "id": "f2", "name": "Carpeta", "folders": [], "lists": [] }]);
    get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2", folder)]))).mount(&server).await;
    get("/api/v1/docs/").respond_with(ok(json!({}))).mount(&server).await;
    get("/api/v1/tasks/").respond_with(ok(json!([]))).mount(&server).await;
    get("/api/v1/chat/latest").respond_with(ok(json!([]))).mount(&server).await;
    let direct = json!({ "conversationId": "c9", "orgId": "o2", "userId": "u9", "username": "zoe", "unread": 0, "lastBody": "hola", "lastAuthorId": "u9" });
    get("/api/v1/dm/").respond_with(ok(json!([direct]))).mount(&server).await;
    let doc = json!({ "doc": null, "orgId": "o2", "decisions": [], "attachments": [], "tabs": [{ "key": "overview", "body": "La carpeta.", "bodyHash": "h1" }] });
    get("/api/v1/docs/folder/f2").respond_with(ok(doc)).mount(&server).await;
    Mock::given(method("GET")).and(path_regex("^/api/v1/docs/folder/f2/pages")).respond_with(ok(json!([]))).mount(&server).await;
    Mock::given(method("GET")).and(path_regex("^/api/v1/requests")).respond_with(ok(json!([]))).mount(&server).await;

    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.open_workspace().await;
    assert_eq!(core.orgs().state().current_org_id.as_deref(), Some("o1"));
    (server, core)
}

fn org(core: &Core) -> Option<String> {
    core.orgs().state().current_org_id
}

/// Un canal, un directo y un documento de **otra** organización se abren
/// en ella, cada uno en su pantalla, venga el enlace como venga.
///
/// Roturas que atrapa: no cambiar de organización (el canal o el directo
/// no se encuentran en la que se mira, el documento se abriría sobre el
/// árbol que no es); no buscar el directo en la lista de todas; no buscar
/// el nodo en los árboles de las demás; dejar comunicación delante al abrir
/// un documento, o no entrar en ella al abrir un canal.
#[tokio::test]
async fn a_link_opens_its_screen_in_its_organization() {
    let (server, core) = rig().await;
    let base = server.uri();

    assert_eq!(core.open_link(&format!("{base}/app/chat?space=z")).await, LinkOpened::Opened);
    assert_eq!(org(&core).as_deref(), Some("o2"));
    let comms = core.comms().state();
    assert!(comms.on_screen);
    assert_eq!(comms.selected, Some(Conversation::Channel { space_id: "z".into() }));

    assert!(core.switch_org("o1").await);
    assert_eq!(core.open_link("cac://dm?c=c9").await, LinkOpened::Opened);
    assert_eq!(org(&core).as_deref(), Some("o2"));
    assert_eq!(core.comms().state().selected, Some(Conversation::Direct { conversation_id: "c9".into() }));

    assert!(core.switch_org("o1").await);
    assert_eq!(core.open_link("/tasks?doc=folder:f2").await, LinkOpened::Opened);
    assert_eq!(org(&core).as_deref(), Some("o2"));
    assert_eq!(core.docs().state().target.map(|target| target.id).as_deref(), Some("f2"));
    assert!(!core.comms().state().on_screen, "la documentación, delante");
    core.stream().stop();
}

/// «Por atender» con su pedido, la organización (que es de la interfaz),
/// lo que la app no tiene (a la web) y lo que no está (un fallo dicho).
///
/// Roturas que atrapa: un pedido que no resalta; abrir algo en vez de
/// dejarle la organización a la interfaz; un enlace de cac que esta app no
/// tiene dicho como fallo en vez de ir a la web; un canal que no es de
/// nadie dado por abierto, o mandado al navegador que tampoco lo tiene.
#[tokio::test]
async fn what_is_not_a_screen_here_goes_to_the_web_or_is_said() {
    let (server, core) = rig().await;
    let base = server.uri();

    assert_eq!(core.open_link("/requests?open=REQ-7").await, LinkOpened::Opened);
    let requests = core.requests().state();
    assert!(requests.on_screen);
    assert_eq!(requests.highlight.as_deref(), Some("req-7"));

    assert_eq!(core.open_link("cac://organization").await, LinkOpened::Organization);
    assert_eq!(core.open_link("cac://servers/x").await, LinkOpened::Browser { url: format!("{base}/app/servers/x") });
    assert_eq!(core.open_link("https://example.com/a").await, LinkOpened::Browser { url: "https://example.com/a".into() });
    assert_eq!(core.open_link("/chat?space=de-nadie").await, LinkOpened::Failed);
    assert_eq!(core.open_link("javascript:alert(1)").await, LinkOpened::Failed);
    assert_eq!(org(&core).as_deref(), Some("o1"), "nada de eso cambia de organización");
    core.stream().stop();
}

/// Un enlace cuyo id, descodificado, sale de su sitio (`a/../../x`) no
/// abre nada y no hace ninguna petición fuera de la ruta que le toca.
///
/// Roturas que atrapa: meter en la ruta de la API el id de una nota o de
/// una publicación del muro tal como llega en el enlace; mirar la barra
/// antes de descodificar.
#[tokio::test]
async fn an_id_from_a_link_never_reaches_another_route() {
    let (server, core) = rig().await;
    for sneaky in ["cac://notes/a%2F..%2F..%2Fx", "cac://feedback?post=..%2F..%2Fx"] {
        assert_eq!(core.open_link(sneaky).await, LinkOpened::Failed, "{sneaky}");
    }
    let asked: Vec<String> = server.received_requests().await.unwrap_or_default().iter().map(|request| request.url.path().to_string()).collect();
    assert!(asked.iter().all(|path| !path.ends_with("/x") && !path.contains("..")), "{asked:?}");
    core.stream().stop();
}

/// La tabla de rutas que la app nativa abre (`fixtures/native_routes.json`) es
/// la verdad de `app_link` y `launch_landing`; la web la lee para decidir
/// dónde ofrece «Abrir en la app» (`open-in-app.test.ts`).
///
/// Roturas que atrapa: dejar de abrir `/dm?u=`; leer el canal de `?channel=`
/// en vez de `?space=`; añadir una pantalla (o quitarla) sin tocar la tabla,
/// con lo que la web ofrecería abrir donde no hay nada o callaría donde sí;
/// mandar `/my-work` al inicio o al frente; perder `/mywork`.
#[test]
fn the_routes_the_native_app_opens_are_the_shared_table() {
    const BASE: &str = "https://cac.guz-studio.dev";
    let table: Value = serde_json::from_str(include_str!("fixtures/native_routes.json")).unwrap();
    let routes = table["routes"].as_array().unwrap();
    assert!(routes.len() > 20, "la tabla no puede vaciarse en silencio");
    for case in routes {
        let route = case["route"].as_str().unwrap();
        let screen = case["screen"].as_str().unwrap();
        for href in [route.to_string(), format!("cac://{}", route.trim_start_matches('/')), format!("{BASE}/app{route}")] {
            let found = format!("{:?}", app_link(&href, BASE));
            let name: String = found.chars().take_while(|c| c.is_alphanumeric()).collect();
            assert_eq!(name, screen, "{href}");
            if screen == "Browser" {
                let landing = match launch_landing(&app_route(&href, BASE).unwrap()) {
                    Landing::Home => "home",
                    Landing::MyWork => "my_work",
                    Landing::Front => "front",
                };
                assert_eq!(Some(landing), case["landing"].as_str(), "{href}");
            }
        }
    }
}

/// Un `cac://` que llega de fuera y no es una pantalla nunca vuelve al
/// navegador: `/tasks` sin tarea y `cac:///` llevan al inicio, `/my-work` y
/// `/mywork` a «Mi trabajo» (lo que cierra el resumen y la documentación), y
/// lo demás deja todo como estaba. Lo pulsado dentro sigue yendo a la web.
///
/// Roturas que atrapa: quitar `overview().show()` (la persona se queda donde
/// estaba); dejar `/my-work` en «al frente» o en el resumen; dejar la
/// documentación abierta al ir a «Mi trabajo»; cambiar de pantalla con los
/// servidores; devolver el navegador (`Browser`) por el esquema.
#[tokio::test]
async fn a_scheme_link_that_is_no_screen_lands_on_home_my_work_or_stays() {
    let (_server, core) = rig().await;

    for home in ["cac://tasks", "cac://", "cac:///", "cac://overview"] {
        core.overview().leave();
        assert_eq!(core.open_launched_link(home).await, LinkOpened::Opened, "{home}");
        assert!(core.overview().state().on_screen, "{home} lleva al inicio");
    }

    for my_work in ["cac://my-work", "cac://mywork", "cac:///my-work"] {
        assert_eq!(core.open_link("/tasks?doc=folder:f2").await, LinkOpened::Opened);
        assert!(core.docs().state().target.is_some());
        assert_eq!(core.open_launched_link(my_work).await, LinkOpened::Opened, "{my_work}");
        assert!(!core.overview().state().on_screen, "{my_work}: no el resumen");
        assert!(core.docs().state().target.is_none(), "{my_work}: no la documentación");
    }

    core.overview().show();
    for front in ["cac://servers/x/backups", "cac://call/abc"] {
        assert_eq!(core.open_launched_link(front).await, LinkOpened::Opened, "{front}");
        assert!(core.overview().state().on_screen, "{front} no cambia de pantalla");
    }

    assert_eq!(core.open_link("https://ejemplo.com/a").await, LinkOpened::Browser { url: "https://ejemplo.com/a".into() });
    assert_eq!(core.open_launched_link("cac://tasks?task=a%2Fb").await, LinkOpened::Failed, "un id que no es un id sigue sin abrir nada");
    core.stream().stop();
}
