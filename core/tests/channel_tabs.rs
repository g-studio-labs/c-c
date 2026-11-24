//! Las pestañas de un canal, las grabaciones y las llamadas con invitados
//! contra un servidor de mentira: qué se pide y qué se manda. Cada prueba
//! dice la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{Config, Core, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, header, method, path, query_param};
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

impl cac_core::Listener for Quiet {
    fn session_changed(&self, _session: Option<Session>) {}
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

async fn rig() -> (MockServer, Core) {
    let server = MockServer::start().await;
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(Quiet)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    (server, core)
}

/// Multimedia y Enlaces leen su ruta con lo buscado y el cursor, y traen el
/// cursor del servidor.
///
/// Roturas que atrapa: Enlaces leyendo la ruta de Multimedia; perder el
/// cursor que manda el servidor; no mandar lo buscado.
#[tokio::test]
async fn media_and_links_read_their_own_pages() {
    let (server, core) = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/task-spaces/s1/chat/media"))
        .and(query_param("q", "plano"))
        .respond_with(ok(json!({ "items": [{ "id": "f1", "url": "/api/v1/files/f1", "fileName": "plano.png", "contentType": "image/png",
            "messageId": "m1", "postedAt": "2026-01-05T15:00:00Z", "authorName": "Luis" }], "before": "2026-01-01T00:00:00Z" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/task-spaces/s1/chat/links"))
        .respond_with(ok(json!({ "items": [{ "url": "https://example.com", "label": "el runbook", "messageId": "m2", "postedAt": "2026-01-05T15:00:00Z" }] })))
        .mount(&server)
        .await;

    let media = core.channel_tabs().media("s1", "plano", None).await.unwrap();
    assert_eq!(media.items[0].file_name, "plano.png");
    assert!(media.items[0].is_image());
    assert_eq!(media.before.as_deref(), Some("2026-01-01T00:00:00Z"));
    let links = core.channel_tabs().links("s1", "", None).await.unwrap();
    assert_eq!(links.items[0].label.as_deref(), Some("el runbook"));
    assert_eq!(links.before, None);
}

/// Grabaciones sólo existe si la política lo dice, y un fallo al
/// preguntarla la esconde en vez de dar error.
///
/// Roturas que atrapa: enseñar Grabaciones en un servidor que no graba;
/// enseñarla porque la pregunta falló.
#[tokio::test]
async fn recordings_exist_only_when_the_server_records() {
    let (server, core) = rig().await;
    Mock::given(method("GET")).and(path("/api/v1/task-spaces/on/recordings/policy")).respond_with(ok(json!({ "enabled": true, "active": null }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/task-spaces/off/recordings/policy")).respond_with(ok(json!({ "enabled": false, "active": null }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/task-spaces/broken/recordings/policy")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
    let tabs = core.channel_tabs();
    assert!(tabs.recordings_enabled("on").await);
    assert!(!tabs.recordings_enabled("off").await);
    assert!(!tabs.recordings_enabled("broken").await);
}

/// Ver una grabación fuera de la app pide un pase de adjuntos y lo pone en
/// la dirección del proxy de cac, nunca el token de acceso; borrarla manda
/// el DELETE a su ruta.
///
/// Roturas que atrapa: el token de acceso en una URL que acaba en los
/// logs; una dirección que no pasa por el proxy; el pase sin escapar;
/// borrar otra grabación.
#[tokio::test]
async fn watching_a_recording_uses_a_short_ticket() {
    let (server, core) = rig().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/url-ticket"))
        .and(query_param("scope", "media"))
        .and(header("Authorization", "Bearer access"))
        .respond_with(ok(json!({ "ticket": "t+1", "expiresAt": "2026-01-05T15:30:00Z" })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE")).and(path("/api/v1/recordings/r1")).respond_with(ok(json!(null))).expect(1).mount(&server).await;

    let url = core.channel_tabs().recording_watch_url("r1").await.unwrap();
    assert_eq!(url, format!("{}/api/v1/recordings/r1/media?token=t%2B1", server.uri()));
    assert!(!url.contains("access"));
    core.channel_tabs().delete_recording("r1").await.unwrap();
}

/// Un servidor a mano para una sola petición: contesta con lo que la prueba
/// le vaya mandando, cuando lo mande, y cierra al soltar el emisor. Con
/// wiremock la respuesta sale entera de golpe, y aquí lo que se mira es qué
/// pasa con lo que va llegando.
async fn by_hand() -> (String, tokio::sync::mpsc::UnboundedSender<Vec<u8>>, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (send, mut parts) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let served = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        while let Some(part) = parts.recv().await {
            socket.write_all(&part).await.unwrap();
            socket.flush().await.unwrap();
        }
        String::from_utf8(head).unwrap()
    });
    (base, send, served)
}

fn signed_in_at(base: &str) -> Core {
    let core = Core::new(Config::new(base.to_string()), Arc::new(MemoryStorage::default()), Arc::new(Quiet)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core
}

fn part_of(target: &std::path::Path) -> std::path::PathBuf {
    let mut partial = target.as_os_str().to_owned();
    partial.push(".part");
    partial.into()
}

/// Una grabación se guarda según llega, a un `.part` al lado, y sólo toma
/// su nombre al terminar entera; la pide con la sesión en la cabecera.
///
/// Roturas que atrapa: juntar la grabación entera en memoria antes de
/// escribirla (cientos de megas); escribir directamente con el nombre
/// final, que a mitad ya parece el fichero; dejar el `.part` al terminar;
/// pedirla sin la sesión.
#[tokio::test]
async fn a_recording_is_written_to_disk_as_it_arrives() {
    let (base, send, served) = by_hand().await;
    let core = signed_in_at(&base);
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("llamada.mp4");
    let half = 1_000_000;
    let whole: Vec<u8> = (0..2 * half).map(|at| (at % 251) as u8).collect();

    let mut first = format!("HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\n\r\n", whole.len()).into_bytes();
    first.extend_from_slice(&whole[..half]);
    send.send(first).unwrap();
    let saving = tokio::spawn({
        let (core, target) = (core.clone(), target.clone());
        async move { core.channel_tabs().save_recording("r1", &target).await }
    });

    let partial = part_of(&target);
    let mut waited = 0;
    while std::fs::metadata(&partial).map(|meta| meta.len()).unwrap_or(0) < half as u64 {
        assert!(waited < 500, "la primera mitad no llegó a disco mientras faltaba la otra");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        waited += 1;
    }
    assert!(!target.exists(), "a medias ya lleva el nombre final");

    send.send(whole[half..].to_vec()).unwrap();
    drop(send);
    saving.await.unwrap().unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), whole);
    assert!(!partial.exists(), "se quedó el .part");
    let head = served.await.unwrap().to_lowercase();
    assert!(head.starts_with("get /api/v1/recordings/r1/media "), "{head}");
    assert!(head.contains("authorization: bearer access"), "{head}");
}

/// Una descarga que se corta a mitad, o que el servidor rechaza, no deja
/// nada con el nombre elegido, ni su `.part`, ni se lleva el fichero que
/// ya había con ese nombre.
///
/// Roturas que atrapa: dar por buena una descarga cortada (un vídeo
/// truncado con su nombre); dejar el `.part` tirado; guardar el cuerpo de
/// un error como si fuera la grabación; pisar el fichero de antes cuando
/// la nueva no llegó.
#[tokio::test]
async fn a_download_cut_short_leaves_nothing_under_its_name() {
    let (base, send, served) = by_hand().await;
    let core = signed_in_at(&base);
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("llamada.mp4");
    std::fs::write(&target, b"la de antes").unwrap();

    let mut cut = b"HTTP/1.1 200 OK\r\nContent-Length: 2000\r\n\r\n".to_vec();
    cut.extend_from_slice(&[7u8; 1000]);
    send.send(cut).unwrap();
    drop(send);
    assert!(core.channel_tabs().save_recording("r1", &target).await.is_err(), "una descarga cortada se dio por buena");
    served.await.unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"la de antes");
    assert!(!part_of(&target).exists(), "se quedó el .part de una descarga cortada");

    let (server, core) = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/recordings/r2/media"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": { "code": "boom", "message": "boom" } })))
        .mount(&server)
        .await;
    let refused = dir.path().join("otra.mp4");
    assert!(core.channel_tabs().save_recording("r2", &refused).await.is_err());
    assert!(!refused.exists() && !part_of(&refused).exists(), "el cuerpo de un error se guardó como grabación");
}

/// Crear una llamada con invitados manda la organización, el canal, el
/// título recortado y cuánto vale; el enlace sale de su token; y cerrarla
/// la borra.
///
/// Roturas que atrapa: una llamada sin canal (no se lista en él); el
/// título con sus espacios; otra duración que la elegida; mudar la llamada
/// del canal, que desde aquí no se pide; un título vacío que llega al
/// servidor; cerrar otra.
#[tokio::test]
async fn a_guest_call_is_created_with_what_was_chosen() {
    let (server, core) = rig().await;
    let invite = json!({ "id": "i1", "orgId": "o1", "spaceId": "s1", "title": "Revisión", "expiresAt": "2026-01-06T15:00:00Z", "link": "tok", "occupants": [] });
    Mock::given(method("POST"))
        .and(path("/api/v1/call-invites"))
        .and(body_json(json!({ "orgId": "o1", "spaceId": "s1", "title": "Revisión", "ttlHours": 168 })))
        .respond_with(ok(invite.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/call-invites"))
        .and(query_param("orgId", "o1"))
        .and(query_param("spaceId", "s1"))
        .respond_with(ok(json!([invite])))
        .mount(&server)
        .await;
    Mock::given(method("DELETE")).and(path("/api/v1/call-invites/i1")).respond_with(ok(json!(null))).expect(1).mount(&server).await;

    let tabs = core.channel_tabs();
    assert!(tabs.create_call_invite("o1", "s1", "   ", 24).await.is_err());
    let created = tabs.create_call_invite("o1", "s1", "  Revisión ", 168).await.unwrap();
    assert_eq!(tabs.guest_link(&created), format!("{}/app/join#tok", server.uri()));
    assert_eq!(tabs.call_invites("o1", "s1").await.unwrap()[0].id, "i1");
    tabs.close_call_invite("i1").await.unwrap();
}
