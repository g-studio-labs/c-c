//! Los enlaces públicos de una página de documentación contra un servidor
//! de mentira: qué se pide y qué se manda, tal como dice el contrato de
//! command-and-control-496. Cada prueba dice la rotura que atrapa.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{Config, Core, NodeKind, Session, Storage};
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

/// Crear manda la duración elegida a esa página del documento del nodo, listar
/// lee los de esa página, el enlace es el de la web, y revocar es un DELETE
/// del enlace que contesta 204.
///
/// Roturas que atrapa: crear en la ruta de otra página o de otra clase de
/// nodo; mandar la duración con otro nombre o en días; leer un
/// `DocShareResponse` con otros nombres de campo; el enlace sin el token
/// detrás del `#`; revocar por otra ruta, o tomar un 204 sin cuerpo como
/// fallo.
#[tokio::test]
async fn a_page_share_is_created_listed_and_revoked() {
    let (server, core) = rig().await;
    let share = json!({ "id": "s1", "pageId": "p1", "expiresAt": "2026-12-24T15:00:00Z", "createdBy": "u1", "createdByName": "Ana Ruiz", "link": "s1.firma" });
    Mock::given(method("POST"))
        .and(path("/api/v1/docs/list/l1/pages/p1/shares"))
        .and(body_json(json!({ "ttlHours": 720 })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": share.clone() })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/api/v1/docs/list/l1/pages/p1/shares")).respond_with(ok(json!([share]))).mount(&server).await;
    Mock::given(method("DELETE")).and(path("/api/v1/doc-shares/s1")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;

    let shares = core.doc_shares();
    let created = shares.create_share(NodeKind::List, "l1", "p1", 720).await.unwrap();
    assert_eq!((created.page_id.as_str(), created.created_by_name.as_str(), created.expires_at.as_str()), ("p1", "Ana Ruiz", "2026-12-24T15:00:00Z"));
    assert_eq!(shares.link(&created), format!("{}/app/shared#s1.firma", server.uri()));
    assert_eq!(shares.list_shares(NodeKind::List, "l1", "p1").await.unwrap(), [created]);
    shares.revoke_share("s1").await.unwrap();
}
