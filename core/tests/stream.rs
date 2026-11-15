//! El stream de eventos y el estado de la conexión, contra un servidor de
//! mentira. Cada prueba dice la rotura que atrapa, comprobada rompiendo el
//! código (`/mutar`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Config, Connection, ConnectionState, Core, Listener, ServerEvent, Session, Storage};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::matchers::{header, method, path};
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
    connection: Mutex<Vec<ConnectionState>>,
    events: Mutex<Vec<ServerEvent>>,
}

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn connection_changed(&self, state: ConnectionState) {
        self.connection.lock().unwrap().push(state);
    }
    fn server_event(&self, event: ServerEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl Recorder {
    fn statuses(&self) -> Vec<Connection> {
        self.connection.lock().unwrap().iter().map(|state| state.status).collect()
    }
    fn opened(&self) -> usize {
        self.statuses().iter().filter(|status| **status == Connection::Open).count()
    }
}

struct Rig {
    server: MockServer,
    core: Core,
    recorder: Arc<Recorder>,
}

/// Un core con sesión contra `uri`, con esperas de prueba: reintenta a los
/// 40 ms y da el stream por muerto tras 250 ms de silencio.
fn core_for(uri: &str, recorder: Arc<Recorder>) -> Core {
    let mut config = Config::new(uri);
    config.stream_backoff = Duration::from_millis(40);
    config.stream_silence = Duration::from_millis(250);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), recorder).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core
}

async fn rig() -> Rig {
    let server = MockServer::start().await;
    let recorder = Arc::new(Recorder::default());
    let core = core_for(&server.uri(), recorder.clone());
    Rig { server, core, recorder }
}

fn stream_of(frames: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(frames.as_bytes().to_vec(), "text/event-stream")
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no llegó a pasar: {what}");
}

async fn hits(server: &MockServer, verb: &str, route: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method.as_str() == verb && request.url.path() == route)
        .count()
}

/// Espera a que «Mi trabajo» se haya pedido más veces que `before`.
async fn my_work_is_asked_again(server: &MockServer, before: usize, when: &str) {
    for _ in 0..200 {
        if hits(server, "GET", "/api/v1/tasks/").await > before {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{when} no se volvió a pedir Mi trabajo");
}

fn my_work_row(id: &str) -> serde_json::Value {
    json!({
        "id": id, "seq": 1, "title": format!("Tarea {id}"), "priority": "normal",
        "status": "pending", "statusName": "Open", "statusKind": "open",
        "listId": "l1", "listName": "Lista", "spaceId": "s1", "spaceName": "Espacio",
        "updatedAt": "2026-11-14T00:00:00Z",
    })
}

async fn mount_my_work(server: &MockServer, rows: Vec<serde_json::Value>) {
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": rows })))
        .mount(server)
        .await;
}

/// El token viaja en una cabecera, nunca en la URL, y los eventos llegan a
/// quien escucha sin los pings.
///
/// Roturas que atrapa: poner el token en la URL (acaba en el log de accesos
/// del servidor); entregar los pings como si fueran eventos.
#[tokio::test]
async fn events_arrive_and_the_token_stays_out_of_the_url() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/events"))
        .and(header("authorization", "Bearer access"))
        .respond_with(stream_of("event: ping\ndata: {}\n\nevent: report:new\ndata: {\"id\":\"r1\"}\n\n"))
        .mount(&rig.server)
        .await;

    rig.core.stream().start();
    until("llega el evento", || !rig.recorder.events.lock().unwrap().is_empty()).await;
    rig.core.stream().stop();

    let events = rig.recorder.events.lock().unwrap().clone();
    assert!(events.iter().all(|event| event.event == "report:new"), "{events:?}");
    assert_eq!(events[0].data, "{\"id\":\"r1\"}");
    let asked = rig.server.received_requests().await.unwrap();
    assert!(asked.iter().all(|request| request.url.query().is_none()), "el token no va en la URL");
    assert_eq!(rig.recorder.statuses()[..2], [Connection::Connecting, Connection::Open]);
}

/// Un stream que se corta vuelve a abrirse solo, y al volver se pide otra vez
/// lo que hay en pantalla.
///
/// Roturas que atrapa: quedarse caído tras el primer corte; volver sin pedir
/// lo que pasó mientras tanto («Mi trabajo» con lo de antes del corte).
#[tokio::test]
async fn a_dropped_stream_comes_back_and_catches_up() {
    let rig = rig().await;
    mount_my_work(&rig.server, vec![my_work_row("t1")]).await;
    rig.core.my_work().load(None).await;
    let loads_before = hits(&rig.server, "GET", "/api/v1/tasks/").await;
    // Un cuerpo que se acaba: para el core, un stream que se cortó.
    Mock::given(method("GET")).and(path("/api/v1/events")).respond_with(stream_of("")).mount(&rig.server).await;

    rig.core.stream().start();
    until("se abre dos veces", || rig.recorder.opened() >= 2).await;
    my_work_is_asked_again(&rig.server, loads_before, "al volver el stream").await;
    rig.core.stream().stop();

    let statuses = rig.recorder.statuses();
    assert!(statuses.contains(&Connection::Down), "{statuses:?}");
}

/// Un 401 se arregla renovando la sesión, y el stream vuelve con el token
/// nuevo sin que nadie de fuera haga nada.
///
/// Rotura que atrapa: reintentar para siempre con el token caducado, o
/// rendirse y dejar la app sin eventos hasta reiniciar.
#[tokio::test]
async fn an_expired_token_is_renewed_and_the_stream_returns() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/events"))
        .and(header("authorization", "Bearer access"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "success": false, "error": "expired-token" })))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/events"))
        .and(header("authorization", "Bearer fresh"))
        .respond_with(stream_of("event: task:new\ndata: {\"taskId\":\"t9\"}\n\n"))
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true, "data": { "accessToken": "fresh", "refreshToken": "refresh-2" },
        })))
        .mount(&rig.server)
        .await;

    rig.core.stream().start();
    until("llega un evento con el token nuevo", || !rig.recorder.events.lock().unwrap().is_empty()).await;
    rig.core.stream().stop();

    assert_eq!(hits(&rig.server, "POST", "/api/v1/auth/refresh").await, 1, "una sola renovación");
    assert_eq!(rig.core.access_token().as_deref(), Some("fresh"));
}

/// Si la sesión terminó, el stream se retira en vez de insistir.
///
/// Rotura que atrapa: golpear al servidor sin parar con una sesión que ya no
/// existe.
#[tokio::test]
async fn a_finished_session_stops_the_stream() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/events"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "success": false, "error": "expired-token" })))
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "success": false, "error": "invalid-token" })))
        .mount(&rig.server)
        .await;

    rig.core.stream().start();
    until("la sesión termina", || rig.core.session().is_none()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(hits(&rig.server, "GET", "/api/v1/events").await, 1, "no se insiste sin sesión");
}

/// Un servidor que rechaza el stream también con el token recién renovado no
/// se lleva una renovación tras otra sin pausa.
///
/// Rotura que atrapa: un bucle sin espera de «renueva y reintenta», que gasta
/// un refresh por vuelta contra un servidor que no va a ceder.
#[tokio::test]
async fn a_stream_refused_with_a_fresh_token_waits_before_trying_again() {
    let rig = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/events"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "success": false, "error": "expired-token" })))
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true, "data": { "accessToken": "fresh", "refreshToken": "refresh-2" },
        })))
        .mount(&rig.server)
        .await;

    rig.core.stream().start();
    tokio::time::sleep(Duration::from_millis(300)).await;
    rig.core.stream().stop();

    // Con la espera de prueba (40 ms, y el doble cada vez) caben cuatro o
    // cinco; sin espera serían decenas.
    let renewals = hits(&rig.server, "POST", "/api/v1/auth/refresh").await;
    assert!((2..=6).contains(&renewals), "{renewals} renovaciones en 300 ms");
}

/// Parar el stream lo para: no vuelve a conectarse.
///
/// Rotura que atrapa: un stream que sigue reconectando tras salir de la
/// cuenta o cerrar la ventana.
#[tokio::test]
async fn a_stopped_stream_does_not_come_back() {
    let rig = rig().await;
    Mock::given(method("GET")).and(path("/api/v1/events")).respond_with(stream_of("")).mount(&rig.server).await;

    rig.core.stream().start();
    until("se abre", || rig.recorder.opened() >= 1).await;
    rig.core.stream().stop();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let asked = hits(&rig.server, "GET", "/api/v1/events").await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(hits(&rig.server, "GET", "/api/v1/events").await, asked);
    assert_eq!(rig.core.stream().state().status, Connection::Idle);
}

/// Un evento de una tarea pone al día «Mi trabajo»; uno de borrado quita la
/// fila y cierra la tarea si era la abierta.
///
/// Roturas que atrapa: que la fila de una tarea que otro movió se quede con
/// lo de antes; que una tarea borrada siga pulsable y abierta.
#[tokio::test]
async fn a_task_event_updates_what_is_on_screen() {
    let rig = rig().await;
    mount_my_work(&rig.server, vec![my_work_row("t1"), my_work_row("t2")]).await;
    rig.core.my_work().load(None).await;
    let loads_before = hits(&rig.server, "GET", "/api/v1/tasks/").await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/t2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": {
            "task": { "id": "t2", "createdAt": "", "updatedAt": "", "listId": "l1", "seq": 2, "title": "Dos", "status": "pending" },
            "status": { "id": "s1", "name": "Open" },
        }})))
        .mount(&rig.server)
        .await;
    rig.core.tasks().open("t2").await;
    assert!(rig.core.tasks().state().detail.is_some());
    Mock::given(method("GET"))
        .and(path("/api/v1/events"))
        .respond_with(
            stream_of("event: task:update\ndata: {\"taskId\":\"t1\"}\n\nevent: task:delete\ndata: {\"taskId\":\"t2\"}\n\n")
                .set_delay(Duration::from_millis(20)),
        )
        .up_to_n_times(1)
        .mount(&rig.server)
        .await;

    rig.core.stream().start();
    until("la tarea borrada se cierra", || rig.core.tasks().state().task_id.is_none()).await;
    rig.core.stream().stop();

    let ids: Vec<String> = rig.core.my_work().state().tasks.into_iter().map(|task| task.id).collect();
    assert_eq!(ids, ["t1"], "la borrada ya no está");
    my_work_is_asked_again(&rig.server, loads_before, "tras el evento de t1").await;
}

/// Un stream abierto que se queda callado se da por muerto y se abre otro.
///
/// El caso que hacía parecer la app congelada: el socket sigue abierto pero
/// por él ya no llega nada. Aquí el servidor contesta las cabeceras y calla.
/// Rotura que atrapa: esperar para siempre a un socket medio muerto.
#[tokio::test]
async fn a_silent_stream_is_dropped_and_reopened() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let seen = connections.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            seen.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut request = [0u8; 2048];
                let _ = socket.read(&mut request).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n")
                    .await;
                // Y ni una palabra más, con el socket abierto.
                tokio::time::sleep(Duration::from_secs(30)).await;
            });
        }
    });
    let recorder = Arc::new(Recorder::default());
    let core = core_for(&format!("http://{address}"), recorder.clone());

    core.stream().start();
    until("se abre otro socket", || connections.load(Ordering::SeqCst) >= 2).await;
    core.stream().stop();

    let reasons: Vec<String> =
        recorder.connection.lock().unwrap().iter().filter_map(|state| state.detail.clone()).collect();
    assert!(reasons.iter().any(|why| why.starts_with("no data for")), "{reasons:?}");
}

/// Al salir de la cuenta, lo que siga llegando por el stream ya no se entrega.
///
/// El socket no se corta en el acto: se suelta cuando llega el siguiente
/// trozo. Rotura que atrapa: que entre el «salir» y ese momento los eventos de
/// la cuenta que se fue sigan moviendo la pantalla.
#[tokio::test]
async fn after_signing_out_nothing_more_is_delivered() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let speak = Arc::new(tokio::sync::Notify::new());
    let server_speak = speak.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let speak = server_speak.clone();
            tokio::spawn(async move {
                let mut request = [0u8; 2048];
                let read = socket.read(&mut request).await.unwrap_or(0);
                if !request[..read].starts_with(b"GET /api/v1/events") {
                    // El «salir»: se contesta y se cierra.
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 16\r\nconnection: close\r\n\r\n{\"success\":true}")
                        .await;
                    return;
                }
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n")
                    .await;
                speak.notified().await;
                let frame = "event: task:new\ndata: {\"taskId\":\"t1\"}\n\n";
                let _ = socket.write_all(format!("{:x}\r\n{frame}\r\n", frame.len()).as_bytes()).await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            });
        }
    });
    let recorder = Arc::new(Recorder::default());
    let mut config = Config::new(format!("http://{address}"));
    config.stream_silence = Duration::from_secs(20);
    let core = Core::new(config, Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();

    core.stream().start();
    until("se abre", || recorder.opened() >= 1).await;
    core.logout().await;
    speak.notify_one();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(recorder.events.lock().unwrap().is_empty(), "llegó un evento de la cuenta que ya salió");
    assert_eq!(core.stream().state().status, Connection::Idle);
}
