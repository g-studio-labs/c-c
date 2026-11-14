//! Las reglas de la sesión, contra un servidor de mentira.
//!
//! Cada prueba dice qué rotura atrapa, y se ha comprobado rompiendo el código
//! a propósito (`/mutar`): una prueba de sesión que pasa con el cerrojo
//! quitado no estaba probando el cerrojo.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use cac_core::{ApiError, Config, Core, FileStorage, Listener, Session, Storage};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Un almacén en memoria que además deja mirar qué tenía el core en la mano
/// en el momento de guardar.
#[derive(Default)]
struct MemoryStorage {
    values: Mutex<HashMap<String, String>>,
    core: OnceLock<Core>,
    /// El token de acceso que el core tenía en la mano en cada `set`.
    in_hand_at_set: Mutex<Vec<Option<String>>>,
}

impl Storage for MemoryStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.values.lock().unwrap().get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        if let Some(core) = self.core.get() {
            self.in_hand_at_set.lock().unwrap().push(core.access_token());
        }
        self.values.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn remove(&self, key: &str) {
        self.values.lock().unwrap().remove(key);
    }
}

/// Apunta cada aviso de sesión: `Some(usuario)` o `None`.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Option<String>>>,
}

impl Listener for Recorder {
    fn session_changed(&self, session: Option<Session>) {
        self.seen.lock().unwrap().push(session.map(|s| s.username));
    }
}

fn ana() -> Session {
    Session {
        id: "u1".into(),
        username: "ana".into(),
        name: None,
        email: None,
        superadmin: false,
        must_change_password: false,
        locale: None,
    }
}

struct Rig {
    server: MockServer,
    core: Core,
    storage: Arc<MemoryStorage>,
    recorder: Arc<Recorder>,
}

/// Un core con Ana dentro, con el par `old-access` / `old-refresh`.
async fn signed_in() -> Rig {
    let server = MockServer::start().await;
    let storage = Arc::new(MemoryStorage::default());
    let recorder = Arc::new(Recorder::default());
    let core = Core::new(Config::new(server.uri()), storage.clone(), recorder.clone()).unwrap();
    core.sign_in(ana(), "old-access".into(), "old-refresh".into()).unwrap();
    Rig { server, core, storage, recorder }
}

/// `/api/v1/tasks/`: caducado con el token viejo, bien con el nuevo.
async fn mount_tasks(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/"))
        .respond_with(|request: &Request| {
            let bearer = request.headers.get("authorization").and_then(|v| v.to_str().ok());
            if bearer == Some("Bearer new-access") {
                ResponseTemplate::new(200).set_body_string(r#"{"success":true,"data":[]}"#)
            } else {
                ResponseTemplate::new(401).set_body_string(
                    r#"{"success":false,"error":"expired-token","message":"Unauthorized"}"#,
                )
            }
        })
        .mount(server)
        .await;
}

fn refreshed() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_string(
        r#"{"success":true,"data":{"accessToken":"new-access","refreshToken":"new-refresh"}}"#,
    )
}

/// Diez llamadas que caducan a la vez gastan **un** refresh.
///
/// El servidor rota el refresh al usarlo: el segundo que llegue con el mismo
/// es, para él, un refresh robado, y cierra la sesión en todos los
/// dispositivos. Rotura que atrapa: quitar el turno de `Core::refresh`.
#[tokio::test(flavor = "multi_thread")]
async fn calls_that_expire_together_refresh_once() {
    let rig = signed_in().await;
    mount_tasks(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .and(header("authorization", "Bearer old-refresh"))
        // Lento adrede: que las diez estén esperando a la vez.
        .respond_with(refreshed().set_delay(Duration::from_millis(150)))
        .expect(1)
        .mount(&rig.server)
        .await;

    let calls = (0..10).map(|_| {
        let core = rig.core.clone();
        tokio::spawn(async move { core.request("GET", "/api/v1/tasks/", None, true).await })
    });
    for call in calls.collect::<Vec<_>>() {
        assert_eq!(call.await.unwrap().unwrap().status, 200);
    }
    // `expect(1)` se comprueba al soltar el servidor.
}

/// El par nuevo está guardado antes de estar en la mano.
///
/// Rotura que atrapa: poner en la mano primero y guardar después. Con ese
/// orden otra llamada usa el token nuevo antes de que esté a salvo, y un corte
/// ahí deja la app arrancando con un refresh ya gastado (22898591).
#[tokio::test]
async fn a_new_pair_is_stored_before_it_is_used() {
    let rig = signed_in().await;
    mount_tasks(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .respond_with(refreshed())
        .mount(&rig.server)
        .await;
    rig.storage.core.set(rig.core.clone()).ok().unwrap();

    rig.core.request("GET", "/api/v1/tasks/", None, true).await.unwrap();

    assert_eq!(
        *rig.storage.in_hand_at_set.lock().unwrap(),
        [Some("old-access".to_string())],
        "al guardar el par nuevo, el core todavía tenía el viejo en la mano",
    );
    assert!(rig.storage.get("session").unwrap().contains("new-refresh"));
    assert_eq!(rig.core.access_token().as_deref(), Some("new-access"));
}

/// Renovar avisa, aunque la persona sea la misma.
///
/// El escritorio guarda una copia del token de acceso para el stream de
/// eventos y las subidas. Rotura que atrapa: renovar en silencio, que deja
/// esa copia caducada para siempre.
#[tokio::test]
async fn a_renewed_token_is_announced() {
    let rig = signed_in().await;
    mount_tasks(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .respond_with(refreshed())
        .mount(&rig.server)
        .await;

    rig.core.request("GET", "/api/v1/tasks/", None, true).await.unwrap();

    assert_eq!(
        *rig.recorder.seen.lock().unwrap(),
        [Some("ana".to_string()), Some("ana".to_string())],
        "uno al entrar y otro al renovar",
    );
}

/// Un servidor que no pudo mirar no echa a nadie.
///
/// Rotura que atrapa: cerrar la sesión ante cualquier error del refresh.
#[tokio::test]
async fn a_refresh_the_server_could_not_check_keeps_the_session() {
    let rig = signed_in().await;
    mount_tasks(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(500).set_body_string(r#"{"success":false,"error":"internal"}"#))
        .mount(&rig.server)
        .await;

    let result = rig.core.request("GET", "/api/v1/tasks/", None, true).await;

    assert_eq!(result, Err(ApiError::SessionExpired));
    assert_eq!(rig.core.session(), Some(ana()), "la sesión sigue");
    assert!(rig.storage.get("session").unwrap().contains("old-refresh"));
    assert_eq!(*rig.recorder.seen.lock().unwrap(), [Some("ana".to_string())]);
}

/// Un refresh que no vale sí cierra, aquí y en lo guardado, y avisa.
///
/// Rotura que atrapa: no cerrar nunca (tratar el 401 como pasajero).
#[tokio::test]
async fn a_refresh_that_is_refused_ends_the_session() {
    let rig = signed_in().await;
    mount_tasks(&rig.server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"success":false,"error":"invalid-token"}"#))
        .mount(&rig.server)
        .await;

    let result = rig.core.request("GET", "/api/v1/tasks/", None, true).await;

    assert_eq!(result, Err(ApiError::SessionExpired));
    assert_eq!(rig.core.session(), None);
    assert_eq!(rig.storage.get("session"), None);
    assert_eq!(*rig.recorder.seen.lock().unwrap(), [Some("ana".to_string()), None]);
}

/// La sesión de una versión anterior no pisa la que el core ya tiene.
///
/// Rotura que atrapa: adoptar siempre.
#[tokio::test]
async fn a_legacy_session_never_replaces_the_one_in_the_core() {
    let rig = signed_in().await;

    let adopted = rig
        .core
        .adopt_legacy_session(ana(), "stale-access".into(), "stale-refresh".into())
        .unwrap();

    assert!(!adopted);
    assert_eq!(rig.core.access_token().as_deref(), Some("old-access"));
    assert!(rig.storage.get("session").unwrap().contains("old-refresh"));
}

/// …y sí entra cuando el core está vacío: es la migración.
///
/// Rotura que atrapa: no adoptar nunca (todo el mundo a la pantalla de login
/// al actualizar).
#[tokio::test]
async fn a_legacy_session_is_adopted_by_an_empty_core() {
    let server = MockServer::start().await;
    let storage = Arc::new(MemoryStorage::default());
    let core = Core::new(Config::new(server.uri()), storage.clone(), Arc::new(Recorder::default())).unwrap();

    let adopted = core.adopt_legacy_session(ana(), "a".into(), "r".into()).unwrap();

    assert!(adopted);
    assert_eq!(core.session(), Some(ana()));
}

/// Cerrar y abrir la app conserva la sesión.
///
/// Rotura que atrapa: no leer lo guardado al arrancar.
#[tokio::test]
async fn a_new_core_starts_with_the_stored_session() {
    let rig = signed_in().await;

    let reopened = Core::new(Config::new(rig.server.uri()), rig.storage.clone(), Arc::new(Recorder::default())).unwrap();

    assert_eq!(reopened.session(), Some(ana()));
    assert_eq!(reopened.access_token().as_deref(), Some("old-access"));
}

/// Una llamada sin sesión no lleva el token.
///
/// El login y el refresh van sin él, y mandarlo a una ruta pública es
/// enseñarlo donde no hace falta. Rotura que atrapa: ponerlo siempre.
#[tokio::test]
async fn a_call_without_auth_carries_no_token() {
    let rig = signed_in().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/health"))
        .respond_with(|request: &Request| {
            ResponseTemplate::new(if request.headers.contains_key("authorization") { 400 } else { 200 })
        })
        .mount(&rig.server)
        .await;

    assert_eq!(rig.core.request("GET", "/api/v1/health", None, false).await.unwrap().status, 200);
    assert_eq!(rig.core.request("GET", "/api/v1/health", None, true).await.unwrap().status, 400);
}

/// Entrar guarda la sesión y manda el cliente que se configuró.
///
/// Rotura que atrapa: no guardar tras el login; no mandar `client`.
#[tokio::test]
async fn logging_in_stores_the_session() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(|request: &Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            if body["client"] != "desktop" || body["username"] != "ana" {
                return ResponseTemplate::new(400).set_body_string(r#"{"success":false,"error":"bad-request"}"#);
            }
            ResponseTemplate::new(200).set_body_string(
                r#"{"success":true,"data":{"accessToken":"a","refreshToken":"r","expiresIn":900,
                    "session":{"id":"u1","username":"ana"}}}"#,
            )
        })
        .mount(&server)
        .await;
    let storage = Arc::new(MemoryStorage::default());
    let mut config = Config::new(server.uri());
    config.client = Some("desktop".into());
    let core = Core::new(config, storage.clone(), Arc::new(Recorder::default())).unwrap();

    let session = core.login("ana", "secreto-largo").await.unwrap();

    assert_eq!(session, ana());
    assert!(storage.get("session").unwrap().contains("\"refreshToken\":\"r\""));
}

/// Un login rechazado trae el código y la frase por separado.
///
/// Rotura que atrapa: enseñar la etiqueta donde va la frase (el «Error:
/// invalid-credentials» que ya se vio en pantalla).
#[tokio::test]
async fn a_refused_login_says_why() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(ResponseTemplate::new(401).set_body_string(
            r#"{"success":false,"error":"invalid-credentials","message":"Wrong username or password"}"#,
        ))
        .mount(&server)
        .await;
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(Recorder::default())).unwrap();

    let error = core.login("ana", "secreto-largo").await.unwrap_err();

    assert_eq!(
        error,
        ApiError::Server {
            status: 401,
            code: "invalid-credentials".into(),
            message: "Wrong username or password".into(),
        }
    );
    assert_eq!(core.session(), None);
}

/// Salir revoca en el servidor con el refresh, y borra aquí aunque no haya red.
///
/// Rotura que atrapa: salir sólo en local (un refresh copiado seguiría
/// valiendo siete días).
#[tokio::test]
async fn logging_out_revokes_on_the_server() {
    let rig = signed_in().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/logout"))
        .and(header("authorization", "Bearer old-refresh"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&rig.server)
        .await;

    rig.core.logout().await;

    assert_eq!(rig.core.session(), None);
    assert_eq!(rig.storage.get("session"), None);
}

/// Un servidor que no contesta es un fallo de red, con su prefijo.
///
/// Rotura que atrapa: cambiar el prefijo `network:`, que la app usa para
/// separar «no llegó» de «dijo que no».
#[tokio::test]
async fn a_server_that_is_not_there_is_a_network_error() {
    // El puerto 1 rechaza al instante: sin red de por medio, sin intermitencia.
    let core = Core::new(Config::new("http://127.0.0.1:1"), Arc::new(MemoryStorage::default()), Arc::new(Recorder::default())).unwrap();

    let error = core.request("GET", "/api/v1/health", None, false).await.unwrap_err();

    assert!(matches!(error, ApiError::Network(_)), "{error:?}");
    assert!(error.to_string().starts_with("network: "), "{error}");
}

/// El fichero de sesión del escritorio sólo lo lee su dueño, y no deja restos.
///
/// Rotura que atrapa: quitar el modo `0600`; escribir sin renombrar.
#[test]
fn the_desktop_storage_is_private_and_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let storage = FileStorage::new(dir.path().join("cac"));

    storage.set("session", "uno").unwrap();
    storage.set("session", "dos").unwrap();

    assert_eq!(storage.get("session").as_deref(), Some("dos"));
    let names: Vec<String> = std::fs::read_dir(dir.path().join("cac"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["session.json"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("cac/session.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    storage.remove("session");
    assert_eq!(storage.get("session"), None);
}

/// Un socket que se muere a media llamada se cura solo, con una conexión nueva.
///
/// Es el primer clic tras un rato sin usar la app. Rotura que atrapa: quitar
/// el reintento de `Core::send`.
#[tokio::test]
async fn a_dead_connection_is_retried_once() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        // La primera conexión se corta sin contestar.
        let (first, _) = listener.accept().await.unwrap();
        drop(first);
        // La segunda contesta.
        let (mut second, _) = listener.accept().await.unwrap();
        let mut buffer = [0u8; 2048];
        let _ = second.read(&mut buffer).await;
        second
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")
            .await
            .unwrap();
    });
    let core = Core::new(Config::new(format!("http://{address}")), Arc::new(MemoryStorage::default()), Arc::new(Recorder::default())).unwrap();

    let reply = core.request("GET", "/api/v1/health", None, false).await.unwrap();

    assert_eq!(reply.status, 200);
}
