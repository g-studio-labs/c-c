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

use cac_core::{
    can_transition, canonical_priority, flow_for, open_subtasks_block, DocDecision, DocMark, DocPageView, DocTab, Flow, OpenTask, Organization, Session, SpaceTree, TaskDetail,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn keys(value: &Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

/// Las claves que el servidor escribió y el core no devuelve, con su ruta
/// (`comments[0].author.userId`). Mira también dentro de objetos y listas: un
/// campo que falte en una subtarea es tan invisible como uno de arriba.
fn unread(written: &Value, rewritten: &Value, at: &str, out: &mut Vec<String>) {
    match (written, rewritten) {
        (Value::Object(written), Value::Object(rewritten)) => {
            for (key, value) in written {
                let path = if at.is_empty() { key.clone() } else { format!("{at}.{key}") };
                match rewritten.get(key) {
                    Some(read) => unread(value, read, &path, out),
                    None => out.push(path),
                }
            }
        }
        (Value::Array(written), Value::Array(rewritten)) => {
            for (index, value) in written.iter().enumerate() {
                match rewritten.get(index) {
                    Some(read) => unread(value, read, &format!("{at}[{index}]"), out),
                    None => out.push(format!("{at}[{index}]")),
                }
            }
        }
        _ => {}
    }
}

/// Lee `written` con el tipo del core y comprueba que no se dejó ningún campo
/// por el camino, salvo los de `skipped`, que se nombran a propósito.
fn reads_every_field_but<T: DeserializeOwned + Serialize>(written: &Value, skipped: &[&str]) -> T {
    let read: T = serde_json::from_value(written.clone()).expect("el core no sabe leer esta forma");
    let rewritten = serde_json::to_value(&read).unwrap();
    let mut unknown = Vec::new();
    unread(written, &rewritten, "", &mut unknown);
    unknown.retain(|path| !skipped.contains(&path.as_str()));
    assert!(unknown.is_empty(), "el servidor manda campos que el core no lee: {unknown:?}");
    read
}

fn reads_every_field<T: DeserializeOwned + Serialize>(written: &Value) -> T {
    reads_every_field_but(written, &[])
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

/// El detalle de una tarea se lee entero, también lo de dentro.
///
/// Roturas que atrapa: un campo que el servidor manda y el core no lee, en la
/// tarea o en cualquiera de sus comentarios, subtareas, adjuntos o enlaces de
/// git; y leer `ref` (palabra reservada en Rust) con otro nombre.
#[test]
fn a_task_detail_is_read_whole() {
    // `telemetry` es lo único que el core no lee: ver `TaskDetail`.
    let detail: TaskDetail = reads_every_field_but(&fixture("task_detail.json"), &["telemetry"]);

    assert_eq!(detail.task.seq, 12);
    assert_eq!(detail.reference.as_deref(), Some("acme-7"));
    assert_eq!(detail.blocked_by[0].reference.as_deref(), Some("command-and-control-9"));
    assert_eq!(detail.comments[0].attachments[0].file_name, "captura.png");
    assert_eq!(detail.git.unwrap().commits[0].released_in.as_ref().unwrap().tag, "v1.33.0");
}

/// Go escribe `null` donde una lista no se creó, y el core lo lee como vacía.
///
/// Rotura que atrapa: que una tarea sin etiquetas ni comentarios no se pueda
/// abrir («invalid type: null, expected a sequence»).
#[test]
fn a_task_with_empty_lists_written_as_null_is_read() {
    let mut written = fixture("task_detail.json");
    for key in ["tags", "assignees", "comments", "attachments", "subtasks", "blockedBy", "decisions"] {
        written[key] = Value::Null;
    }
    let detail: TaskDetail = serde_json::from_value(written).expect("un null es una lista vacía");
    assert!(detail.tags.is_empty() && detail.comments.is_empty() && detail.blocked_by.is_empty());
}

#[derive(Deserialize)]
struct Transition {
    flow: String,
    from: String,
    to: String,
    allowed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlowOwner {
    project_id: String,
    visibility: String,
    flow: String,
}

#[derive(Deserialize)]
struct PriorityName {
    stored: String,
    wire: String,
}

/// Las dos máquinas de estados del core son las del servidor, par por par.
///
/// Roturas que atrapa: dejar pasar en el flujo del cliente `pendiente →
/// resuelta` (el servidor contesta 409); tratar una tarea interna con la
/// máquina estricta (botones que no hacen nada: así se murió el check de las
/// subtareas); no entender `open` y `done`.
#[test]
fn the_state_machines_are_the_servers() {
    let written = fixture("state_machines.json");
    let transitions: Vec<Transition> = serde_json::from_value(written["transitions"].clone()).unwrap();
    assert_eq!(transitions.len(), 72, "dos máquinas, seis nombres de estado");

    for t in transitions {
        let ours = can_transition(Flow::named(&t.flow), &t.from, &t.to);
        assert_eq!(ours, t.allowed, "{}: {} → {}", t.flow, t.from, t.to);
    }
}

#[derive(Deserialize)]
struct SubtaskBlock {
    from: String,
    to: String,
    open: i64,
    blocked: bool,
}

/// La regla de las subtareas abiertas es la del servidor, caso por caso.
///
/// Roturas que atrapa: dejar terminar o cerrar con subtareas abiertas (el
/// servidor contesta 409 y la pantalla lo habría ofrecido); impedir reabrir, o
/// quedarse en la columna en la que ya se está; bloquear sin ninguna abierta;
/// no entender `open` y `done`.
#[test]
fn open_subtasks_block_what_the_server_blocks() {
    let written = fixture("state_machines.json");
    let blocks: Vec<SubtaskBlock> = serde_json::from_value(written["subtaskBlocks"].clone()).unwrap();
    assert_eq!(blocks.len(), 108, "seis nombres de estado por seis, con cero, una y dos abiertas");
    assert!(blocks.iter().any(|b| b.blocked) && blocks.iter().any(|b| !b.blocked && b.open > 0));

    for b in blocks {
        assert_eq!(open_subtasks_block(&b.from, &b.to, b.open), b.blocked, "{} → {} con {} abiertas", b.from, b.to, b.open);
    }
}

/// A quién le toca cada máquina.
///
/// Rotura que atrapa: darle la del cliente a una tarea interna que vive en la
/// lista de un cliente, o la interna a una que el cliente ve.
#[test]
fn each_task_gets_the_machine_the_server_gives_it() {
    let written = fixture("state_machines.json");
    let owners: Vec<FlowOwner> = serde_json::from_value(written["flows"].clone()).unwrap();
    assert_eq!(owners.len(), 6);

    for o in owners {
        assert_eq!(
            flow_for(&o.project_id, &o.visibility),
            Flow::named(&o.flow),
            "proyecto «{}», visibilidad «{}»",
            o.project_id,
            o.visibility
        );
    }
}

/// La prioridad tiene un solo nombre en el core: el que habla la API.
///
/// Rotura que atrapa: mandar o comparar `medium`, el nombre guardado, que es
/// como convivieron dos nombres para la misma prioridad (#83).
#[test]
fn a_priority_has_the_name_the_api_speaks() {
    let written = fixture("state_machines.json");
    let names: Vec<PriorityName> = serde_json::from_value(written["priorities"].clone()).unwrap();
    assert_eq!(names.len(), 5);

    for name in names {
        assert_eq!(canonical_priority(&name.stored), name.wire);
        assert_eq!(canonical_priority(&name.wire), name.wire);
    }
}

/// La marca de un documento se lee entera.
///
/// De una pestaña y de una decisión el core lee lo que hace falta para
/// leerlas; lo demás es para editarlas, y se nombra aquí campo por campo.
///
/// Rotura que atrapa: renombrar en Go el cuerpo, la clave de la pestaña o
/// quién dejó sin efecto una decisión, y que la pantalla salga vacía o enseñe
/// como vigente lo que ya no lo es.
#[test]
fn a_doc_tab_and_a_decision_are_read_for_reading() {
    let tab: DocTab =
        reads_every_field_but(&fixture("doc_tab.json"), &["id", "createdAt", "docId", "bodyHash", "updatedBy", "sections"]);
    assert_eq!(tab.key, "runbook");
    assert!(tab.body.starts_with("## Desplegar"));
    assert_eq!(tab.updated_by_name.as_deref(), Some("Ana Pérez"));

    let decision: DocDecision = reads_every_field_but(
        &fixture("doc_decision.json"),
        &[
            "createdAt", "updatedAt", "docId", "authorId", "origin", "originTaskId", "sourceUrl", "sourceAuthor", "via",
            "supersedesId", "links",
        ],
    );
    assert_eq!(decision.title, "La interfaz se construye con GPUI Kit");
    assert_eq!(decision.superseded_by_id.as_deref(), Some("dec2"));
    assert_eq!(decision.decided_at, "2026-09-30T00:00:00Z");
}

/// Una página abierta se lee con lo que se pinta de ella: su texto, sus
/// migas, sus hijas (la misma fila que la lista de páginas) y lo que la
/// enlaza. Lo que es de escribirla —el hash, las secciones con identidad—
/// se deja fuera a propósito y se nombra.
///
/// Roturas que atrapa: `parentId`, `breadcrumb`, `children` o
/// `referencedFrom` renombrados en Go (el árbol sale plano, o la página sin
/// migas ni hijas, sin que nada falle); un campo nuevo de la fila del árbol
/// que el core no lee.
#[test]
fn a_doc_page_is_read_with_its_trail_its_children_and_what_links_to_it() {
    let view: DocPageView = reads_every_field_but(
        &fixture("doc_page_view.json"),
        &[
            "orgId", "page.createdAt", "page.docId", "page.orgId", "page.rank", "page.bodyHash", "page.updatedBy", "page.deletedAt",
            "page.sections",
        ],
    );
    assert_eq!((view.page.id.as_str(), view.page.parent_id.as_deref()), ("p2", Some("p1")));
    assert!(view.page.body.starts_with("## Qué es"));
    assert_eq!(view.breadcrumb[0].title, "Arquitectura");
    let child = &view.children[0];
    assert_eq!((child.id.as_str(), child.parent_id.as_deref(), child.has_body, child.subpages), ("p3", Some("p2"), true, 2));
    assert_eq!(view.referenced_from[0].place.as_deref(), Some("runbook"));
    assert!(view.referenced_from[0].link.ends_with("tab=runbook"));
}

/// Rotura que atrapa: un campo que el servidor manda y el core no lee (la
/// línea fijada sobre el tablero, cuántas páginas tiene).
#[test]
fn a_doc_mark_is_read_whole() {
    let mark: DocMark = reads_every_field(&fixture("doc_mark.json"));
    assert!(mark.written);
    assert_eq!(mark.pages, Some(3));
}
