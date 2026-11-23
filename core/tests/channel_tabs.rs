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
