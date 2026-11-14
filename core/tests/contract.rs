//! El contrato con el backend.
//!
//! Los tipos del core están escritos a mano: no hay OpenAPI que los genere.
//! Lo que los ata al servidor son los ficheros de `fixtures/`, que escribe una
//! prueba de Go (`backend/internal/core/domain/core_contract_test.go`) con
//! los structs de verdad y **todos los campos puestos**. Aquí se leen con los
//! tipos del core.
//!
//! Roturas que atrapa: un campo renombrado en Go (el core ya no lo encuentra,
//! o encuentra uno que no conoce); un campo nuevo en Go que el core todavía no
//! lee; un cambio de tipo.

use std::collections::BTreeSet;

use cac_core::{OpenTask, Organization, Session, SpaceTree};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn keys(value: &Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

/// Lee `written` con el tipo del core y comprueba que no se dejó ningún campo
/// por el camino: lo que el core vuelve a escribir tiene, como poco, todas las
/// claves que escribió el servidor.
fn reads_every_field<T: DeserializeOwned + Serialize>(written: &Value) -> T {
    let read: T = serde_json::from_value(written.clone()).expect("el core no sabe leer esta forma");
    let rewritten = serde_json::to_value(&read).unwrap();
    let unknown: Vec<_> = keys(written).difference(&keys(&rewritten)).cloned().collect();
    assert!(unknown.is_empty(), "el servidor manda campos que el core no lee: {unknown:?}");
    read
}

#[test]
fn an_open_task_is_read_whole() {
    let task: OpenTask = reads_every_field(&fixture("open_task.json"));

    assert_eq!(task.seq, 12);
    assert_eq!(task.priority, "medium", "la prioridad llega con su nombre guardado");
    assert_eq!(task.flow.as_deref(), Some("client"));
    assert_eq!(task.due_at.as_deref(), Some("2026-09-30T00:00:00Z"));
}

/// Lo que guarda la sesión del core, con la forma que manda el servidor.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    session: Value,
}

#[test]
fn a_login_answer_is_read_whole() {
    let written = fixture("auth_response.json");
    let answer: AuthResponse = reads_every_field(&written);
    let session: Session = reads_every_field(&answer.session);

    assert_eq!(session.username, "ana");
    assert!(session.superadmin && session.must_change_password);
    assert_eq!(session.scopes, ["tasks:write"]);
    assert_eq!((answer.access_token.as_str(), answer.refresh_token.as_str()), ("access", "refresh"));
    assert_eq!(answer.expires_in, 900);
}

#[test]
fn a_refresh_answer_carries_the_new_pair() {
    let written = fixture("auth_refresh_response.json");
    assert_eq!(keys(&written), BTreeSet::from(["accessToken".to_string(), "refreshToken".to_string()]));
}

#[test]
fn an_organization_is_read_whole() {
    let org: Organization = reads_every_field(&fixture("organization.json"));

    assert_eq!(org.role, "admin");
    assert_eq!(org.timezone, "America/Mexico_City");
    assert!(org.done_needs_subtasks_done);
}

/// El árbol se lee entero **en cada nivel**: el espacio, sus carpetas (también
/// las de dentro), sus listas y su gente.
#[test]
fn a_space_tree_is_read_whole_at_every_level() {
    let written = fixture("space_tree.json");
    let space: SpaceTree = reads_every_field(&written);
    let rewritten = serde_json::to_value(&space).unwrap();

    for pointer in ["/folders/0", "/folders/0/folders/0", "/folders/0/lists/0", "/lists/0", "/people/0"] {
        let (server, core) = (written.pointer(pointer).unwrap(), rewritten.pointer(pointer).unwrap());
        let unknown: Vec<_> = keys(server).difference(&keys(core)).cloned().collect();
        assert!(unknown.is_empty(), "en {pointer} el core no lee: {unknown:?}");
    }
    assert!(space.is_general());
    assert_eq!(space.folders[0].lists[0].open_count, 6);
    assert_eq!(space.people[0].name.as_deref(), Some("Ana Pérez"));
}
