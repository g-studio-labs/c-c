//! La pantalla de la organización contra un servidor de mentira. Cada prueba
//! dice la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{AddOutcome, ApiError, Config, Core, Listener, OrgPatch, Session, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, header, method, path};
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

fn org(id: &str, role: &str) -> Value {
    json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": role, "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" })
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn refused(status: u16, code: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({ "success": false, "error": code, "message": "No" }))
}

/// Ana dentro, admin de `o1`.
async fn rig(orgs: Value) -> (MockServer, Core) {
    let server = MockServer::start().await;
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(Quiet)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    let listing = Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(orgs)).up_to_n_times(1).mount_as_scoped(&server).await;
    core.orgs().load().await;
    drop(listing);
    (server, core)
}

/// «Añadir directamente» a quien no es de tu gente: el servidor pide
/// invitarla y se la invita, en vez de dejar un error.
///
/// Roturas que atrapa: no caer a la invitación; caer también con cualquier
/// otro error (invitar a alguien a quien el servidor negó por otra razón).
#[tokio::test]
async fn adding_someone_who_is_not_yours_invites_them() {
    let (server, core) = rig(json!([org("o1", "admin")])).await;
    Mock::given(method("POST")).and(path("/api/v1/organizations/o1/members")).respond_with(refused(409, "invite-required")).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/organizations/o1/invitations"))
        .and(body_json(json!({ "userId": "u-dan", "role": "member" })))
        .respond_with(ok(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(core.orgs().add_or_invite("o1", "u-dan", "member").await, Ok(AddOutcome::Invited));

    server.reset().await;
    Mock::given(method("POST")).and(path("/api/v1/organizations/o1/members")).respond_with(refused(403, "forbidden")).mount(&server).await;
    Mock::given(method("POST")).and(path("/api/v1/organizations/o1/invitations")).respond_with(ok(json!({}))).expect(0).mount(&server).await;
    assert!(matches!(core.orgs().add_or_invite("o1", "u-dan", "member").await, Err(ApiError::Server { status: 403, .. })));
}

/// Guardar una regla manda el nombre con ella, y lo que contesta el
/// servidor queda en la organización sin perder el papel de quien mira.
///
/// Roturas que atrapa: tomar el papel de la respuesta (a un superadmin le
/// dice «admin» y pasaría a administrarla como miembro); no ponerla al día
/// (el interruptor vuelve a su sitio).
#[tokio::test]
async fn a_saved_rule_stays_and_keeps_your_role() {
    let (server, core) = rig(json!([org("o1", "member")])).await;
    // Lo que contesta a un superadmin: «admin» aunque no sea miembro.
    let mut answer = org("o1", "admin");
    answer["guestsCanUseDevTools"] = json!(true);
    Mock::given(method("PATCH"))
        .and(path("/api/v1/organizations/o1"))
        .and(body_json(json!({ "name": "Org o1", "guestsCanUseDevTools": true })))
        .respond_with(ok(answer))
        .expect(1)
        .mount(&server)
        .await;
    core.orgs().save("o1", &OrgPatch { guests_can_use_dev_tools: Some(true), ..Default::default() }).await.unwrap();
    let now = core.orgs().state().current().cloned().unwrap();
    assert!(now.guests_can_use_dev_tools);
    assert_eq!(now.role, "member");
}

/// Aceptar renueva la sesión (la pertenencia viaja en el token), trae la
/// organización nueva y baja el número del riel.
///
/// Roturas que atrapa: aceptar sin renovar (la organización nueva se ve
/// vacía); no volver a pedir las organizaciones; dejar el número del riel
/// contando la que ya se aceptó.
#[tokio::test]
async fn accepting_renews_the_session_and_brings_the_new_organization() {
    let (server, core) = rig(json!([org("o1", "member")])).await;
    Mock::given(method("POST")).and(path("/api/v1/invitations/i1/accept")).respond_with(ok(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .and(header("Authorization", "Bearer refresh"))
        .respond_with(ok(json!({ "accessToken": "new-access", "refreshToken": "new-refresh" })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org("o1", "member"), org("o2", "viewer")]))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/invitations/")).respond_with(ok(json!([]))).mount(&server).await;

    assert_eq!(core.orgs().accept_invitation("i1").await, Ok(true));
    assert_eq!(core.access_token().as_deref(), Some("new-access"));
    assert_eq!(core.orgs().state().orgs.len(), 2);
    assert_eq!(core.counters().state().invitations, Some(0));
}

/// Leer las invitaciones que te esperan pone su número en el riel.
///
/// Rotura que atrapa: abrir la pantalla con tres y dejar el riel en otro
/// número hasta la próxima carga.
#[tokio::test]
async fn reading_your_invitations_sets_the_rail_count() {
    let (server, core) = rig(json!([org("o1", "member")])).await;
    let invitation = |id: &str| json!({ "id": id, "orgId": "o9", "orgName": "Dwit", "role": "member", "status": "pending", "invitedBy": "bea", "createdAt": "2026-11-20T10:00:00Z" });
    Mock::given(method("GET")).and(path("/api/v1/invitations/")).respond_with(ok(json!([invitation("a"), invitation("b"), invitation("c")]))).mount(&server).await;
    let mine = core.orgs().my_invitations().await.unwrap();
    assert_eq!(mine[0].org_name, "Dwit");
    assert_eq!(core.counters().state().invitations, Some(3));
}

/// Borrar la organización que se mira pasa a otra.
///
/// Rotura que atrapa: quedarse mirando una organización que ya no existe
/// (toda pantalla pediría datos que el servidor niega).
#[tokio::test]
async fn deleting_the_current_organization_moves_to_another() {
    let (server, core) = rig(json!([org("o1", "admin"), org("o2", "member")])).await;
    assert_eq!(core.orgs().state().current_org_id.as_deref(), Some("o1"));
    Mock::given(method("DELETE")).and(path("/api/v1/organizations/o1")).respond_with(ok(json!({}))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org("o2", "member")]))).mount(&server).await;
    // Lo de la organización nueva: lo que no está montado contesta 404 y no
    // importa aquí.
    core.orgs().delete("o1").await.unwrap();
    assert_eq!(core.orgs().state().current_org_id.as_deref(), Some("o2"));
}

fn space(id: &str) -> Value {
    json!({ "id": id, "orgId": "o1", "name": "Producto", "color": "#888", "folders": [], "lists": [], "people": [] })
}

/// Con un documento abierto de la organización que se mira.
async fn reading_a_doc(server: &MockServer, core: &Core) {
    Mock::given(method("GET")).and(path("/api/v1/task-spaces/")).respond_with(ok(json!([space("a")]))).up_to_n_times(1).mount(server).await;
    core.tree().load("o1").await;
    core.docs().open("a").await;
    assert!(core.docs().state().target.is_some(), "no llegó a abrirse");
}

/// Borrar la que se mira hace lo mismo que elegir otra: cierra su
/// documento; y aunque releer la lista falle, la borrada sale de ella.
///
/// Roturas que atrapa: dejar abierto un documento de la organización
/// borrada; seguir mirándola (o listándola) porque la lista no se pudo
/// releer.
#[tokio::test]
async fn deleting_the_current_organization_closes_its_doc_even_if_the_list_fails() {
    let (server, core) = rig(json!([org("o1", "admin"), org("o2", "member")])).await;
    reading_a_doc(&server, &core).await;
    Mock::given(method("DELETE")).and(path("/api/v1/organizations/o1")).respond_with(ok(json!({}))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
    core.orgs().delete("o1").await.unwrap();
    let state = core.orgs().state();
    assert_eq!(state.current_org_id.as_deref(), Some("o2"));
    assert!(state.orgs.iter().all(|org| org.id != "o1"), "la borrada sigue en la lista");
    assert!(core.docs().state().target.is_none(), "el documento de la borrada sigue abierto");
}

/// Quitarte tú mismo de la que miras te lleva a otra, como si la
/// hubieras dejado.
///
/// Rotura que atrapa: seguir mirando una organización a la que ya no
/// perteneces, con su documento abierto.
#[tokio::test]
async fn leaving_the_current_organization_moves_to_another() {
    let (server, core) = rig(json!([org("o1", "admin"), org("o2", "member")])).await;
    reading_a_doc(&server, &core).await;
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org("o2", "member")]))).mount(&server).await;
    core.orgs().left("o1").await;
    assert_eq!(core.orgs().state().current_org_id.as_deref(), Some("o2"));
    assert!(core.docs().state().target.is_none());
}

/// El aviso `org:membership` (te añadieron o te quitaron) vuelve a leer
/// tus organizaciones, no sólo el número de invitaciones.
///
/// Rotura que atrapa: seguir mirando la organización de la que te sacó
/// otro admin hasta reiniciar.
#[tokio::test]
async fn a_membership_event_reloads_your_organizations() {
    let (server, core) = rig(json!([org("o1", "admin"), org("o2", "member")])).await;
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org("o2", "member")]))).mount(&server).await;
    core.stream().deliver(cac_core::ServerEvent { event: "org:membership".into(), data: json!({ "orgId": "o1", "joined": false }).to_string() });
    for _ in 0..300 {
        if core.orgs().state().current_org_id.as_deref() == Some("o2") {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no volvió a leer las organizaciones");
}
