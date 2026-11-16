//! El tablero de «Mi trabajo», contra un servidor de mentira.
//!
//! Viene de `app/src/pages/MyWork.tsx` y sus pruebas (`MyWork.drag.test.tsx`,
//! `KanbanBoard.rechazo.test.tsx`); cada prueba dice la rotura que atrapa, y
//! se ha comprobado rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{
    can_drop, hue_index, work_board, ApiError, BoardStatus, Config, Core, DropVerdict, DueState, Listener, Locale, MyWorkState,
    OpenTask, ScopeKind, Session, Storage, TaskFilter, WorkScope, BAD_TRANSITION, COLUMN_MISSING,
};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
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

/// Apunta cada estado que el core anuncia.
#[derive(Default)]
struct Recorder(Mutex<Vec<MyWorkState>>);

impl Listener for Recorder {
    fn session_changed(&self, _session: Option<Session>) {}
    fn my_work_changed(&self, state: MyWorkState) {
        self.0.lock().unwrap().push(state);
    }
}

/// Una tarea interna, como la manda `GET /api/v1/tasks/`.
fn raw(id: &str, status: &str, list: &str) -> Value {
    json!({
        "id": id, "seq": 1, "title": format!("Tarea {id}"), "priority": "medium",
        "status": status, "statusName": status, "statusKind": "open", "flow": "internal",
        "listId": list, "listName": "Lista", "spaceId": "s1", "spaceName": "Espacio",
        "updatedAt": "2026-11-14T00:00:00Z", "subtasksDone": 0, "subtasksTotal": 0,
    })
}

fn open_task(id: &str, status: &str, edit: impl FnOnce(&mut Value)) -> OpenTask {
    let mut value = raw(id, status, "l1");
    edit(&mut value);
    serde_json::from_value(value).unwrap()
}

fn with_flow(status: &str, flow: Option<&str>) -> OpenTask {
    open_task("t", status, |value| value["flow"] = json!(flow))
}

fn day(year: i32, month: u32, day: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// Las columnas de una lista, con ids que **no** se pueden adivinar desde
/// fuera: quien los componga a mano en vez de leerlos no acierta.
fn columns_of(list: &str) -> Value {
    json!([
        { "id": format!("col-{list}-a"), "listId": list, "name": "Open", "kind": "open", "status": "pending" },
        { "id": format!("col-{list}-b"), "listId": list, "name": "In progress", "kind": "active", "status": "in_progress" },
        { "id": format!("col-{list}-c"), "listId": list, "name": "Done", "kind": "done", "status": "resolved" },
        { "id": format!("col-{list}-d"), "listId": list, "name": "Closed", "kind": "done", "status": "closed" },
    ])
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

struct Rig {
    server: MockServer,
    core: Core,
    recorder: Arc<Recorder>,
}

/// Un core con sesión y «Mi trabajo» ya cargado con `tasks`.
async fn rig(tasks: Vec<Value>) -> Rig {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v1/tasks/")).respond_with(ok(json!(tasks))).mount(&server).await;
    let recorder = Arc::new(Recorder::default());
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), recorder.clone()).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.my_work().load(Some("o1".into())).await;
    Rig { server, core, recorder }
}

async fn mount_columns(server: &MockServer, list: &str, reply: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/task-lists/{list}/statuses")))
        .respond_with(reply)
        .mount(server)
        .await;
}

async fn mount_move(server: &MockServer, task: &str, reply: ResponseTemplate) {
    Mock::given(method("POST")).and(path(format!("/api/v1/tasks/{task}/move"))).respond_with(reply).mount(server).await;
}

/// Lo que se pidió desde que se cargó «Mi trabajo»: verbo, ruta y cuerpo.
async fn asked(server: &MockServer) -> Vec<(String, String, Value)> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .skip(1)
        .map(|request| {
            let body = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            (request.method.to_string(), request.url.path().to_string(), body)
        })
        .collect()
}

fn status_of(state: &MyWorkState, id: &str) -> String {
    state.tasks.iter().find(|task| task.id == id).map(|task| task.status.clone()).unwrap_or_default()
}

/// Las cuatro columnas salen siempre, en su orden y con su nombre, y cada
/// tarjeta en la de su estado —se llame como se llame— y en el orden en que
/// llegó.
///
/// Roturas que atrapa: otro orden de columnas; no entender `pending` o
/// `resolved` (el nombre guardado) y dejar esas tarjetas fuera; juntar lo
/// hecho y lo cerrado, que por clase son lo mismo; barajar las tarjetas de
/// una columna; meter en alguna columna un estado que no se conoce; no pintar
/// una columna porque está vacía.
#[test]
fn the_board_has_four_columns_by_status() {
    let tasks = vec![
        open_task("t1", "resolved", |_| {}),
        open_task("t2", "open", |_| {}),
        open_task("t3", "closed", |_| {}),
        open_task("t4", "pending", |_| {}),
        open_task("t5", "done", |_| {}),
        open_task("t6", "archived", |_| {}),
        open_task("t7", "open", |_| {}),
    ];

    let board = work_board(&tasks, true, Locale::Es, day(2026, 9, 30));

    let shape: Vec<(BoardStatus, &str, Vec<&str>)> = board
        .iter()
        .map(|column| (column.status, column.title.as_str(), column.cards.iter().map(|card| card.id.as_str()).collect()))
        .collect();
    assert_eq!(
        shape,
        [
            (BoardStatus::Open, "Abierta", vec!["t2", "t4", "t7"]),
            (BoardStatus::InProgress, "En curso", vec![]),
            (BoardStatus::Done, "Hecha", vec!["t1", "t5"]),
            (BoardStatus::Closed, "Cerrada", vec!["t3"]),
        ]
    );
    assert_eq!(work_board(&[], true, Locale::En, day(2026, 9, 30))[1].title, "In progress");
}

/// Con «sólo abiertas», lo terminado no está vacío: no se preguntó.
///
/// Roturas que atrapa: decir «Nada» de una columna que ni se pidió (afirma
/// que no hay ninguna terminada); marcar como no pedidas las abiertas; seguir
/// diciendo «no lo pediste» cuando sí se pidió todo.
#[test]
fn a_finished_column_that_was_not_asked_for_says_so() {
    let today = day(2026, 9, 30);
    let hints = |include_closed| -> Vec<(bool, String)> {
        work_board(&[], include_closed, Locale::Es, today).into_iter().map(|column| (column.asked, column.empty_hint)).collect()
    };
    let nothing = "Nada".to_string();
    let not_asked = "No lo pediste — se enseña sólo lo abierto".to_string();

    assert_eq!(
        hints(false),
        [(true, nothing.clone()), (true, nothing.clone()), (false, not_asked.clone()), (false, not_asked)]
    );
    assert!(hints(true).into_iter().all(|hint| hint == (true, nothing.clone())));
}

/// Una tarjeta dice de dónde es, su prioridad, su checklist, cuándo vence y
/// de quién es.
///
/// Roturas que atrapa: una tarjeta sin ruta (en un tablero que cruza listas
/// no se sabe de quién es); una subtarea que parece una tarea suelta;
/// «medium» donde la pantalla espera «normal»; perder el checklist; un
/// vencimiento sin decir; iniciales en minúsculas, de más de dos letras o con
/// otro color que el de la app de siempre; iniciales de nadie; no saber que la
/// lee un cliente.
#[test]
fn a_card_says_where_it_is_from_and_whose_it_is() {
    let tasks = vec![
        open_task("full", "open", |value| {
            value["spaceName"] = json!("Boaty");
            value["listName"] = json!("Tickets");
            value["parentSeq"] = json!(289);
            value["parentTitle"] = json!("La madre");
            value["dueAt"] = json!("2026-09-30T00:00:00Z");
            value["subtasksDone"] = json!(1);
            value["subtasksTotal"] = json!(3);
            value["assignee"] = json!("ana maría");
            value["flow"] = json!("client");
            value["seq"] = json!(12);
        }),
        open_task("bare", "open", |_| {}),
        open_task("no-flow", "open", |value| {
            value["flow"] = Value::Null;
            value["assignee"] = json!("");
        }),
    ];

    let board = work_board(&tasks, false, Locale::Es, day(2026, 9, 30));
    let cards = &board[0].cards;

    let full = &cards[0];
    assert_eq!((full.seq, full.title.as_str()), (12, "Tarea full"));
    assert_eq!(full.path, "Boaty / Tickets › #289 La madre");
    assert_eq!(full.priority, "normal");
    assert_eq!((full.subtasks_done, full.subtasks_total), (1, 3));
    let due = full.due.as_ref().unwrap();
    assert_eq!((due.text.as_str(), due.state), ("hoy", DueState::Today));
    let assignee = full.assignee.as_ref().unwrap();
    assert_eq!((assignee.name.as_str(), assignee.initials.as_str()), ("ana maría", "AN"));
    assert_eq!(assignee.hue, hue_index("ana maría"));
    assert!(full.client_reads);

    let bare = &cards[1];
    assert_eq!(bare.path, "Espacio / Lista");
    assert_eq!((bare.due.clone(), bare.assignee.clone(), bare.client_reads), (None, None, false));

    // Sin máquina dicha es la estricta, la del cliente; y un nombre vacío no
    // es nadie.
    assert!(cards[2].client_reads);
    assert_eq!(cards[2].assignee, None);
}

/// Si una tarjeta puede acabar en una columna lo decide **su** máquina.
///
/// Roturas que atrapa: aplicar la estricta a una tarea interna (columnas en
/// rojo que sí se pueden: así se murió el check de las subtareas); aplicar la
/// interna a un ticket de cliente (se ofrece «Hecha» desde «Abierta» y el
/// servidor contesta 409); tratar una tarea sin máquina dicha como interna;
/// decir que una tarjeta no puede quedarse en su columna porque el servidor
/// la llama `resolved` y la columna `done`; un rechazo sin el código con el
/// que se dice por qué.
#[test]
fn whether_a_card_can_be_dropped_is_up_to_its_own_flow() {
    let refused = DropVerdict::Refused { code: "bad-transition".into() };
    assert_eq!(BAD_TRANSITION, "bad-transition");

    // La del cliente: de «Abierta» a «Hecha» hay que pasar por «En curso».
    let client = |status| with_flow(status, Some("client"));
    assert_eq!(can_drop(&client("pending"), BoardStatus::InProgress), DropVerdict::Move);
    assert_eq!(can_drop(&client("pending"), BoardStatus::Done), refused);
    assert_eq!(can_drop(&client("open"), BoardStatus::Done), refused);
    assert_eq!(can_drop(&client("in_progress"), BoardStatus::Done), DropVerdict::Move);
    assert_eq!(can_drop(&client("closed"), BoardStatus::InProgress), refused);

    // La interna: de cualquier sitio a cualquier sitio, y cerrado no es
    // terminal.
    let internal = |status| with_flow(status, Some("internal"));
    assert_eq!(can_drop(&internal("pending"), BoardStatus::Done), DropVerdict::Move);
    assert_eq!(can_drop(&internal("closed"), BoardStatus::Open), DropVerdict::Move);

    // Sin máquina dicha, la estricta.
    assert_eq!(can_drop(&with_flow("pending", None), BoardStatus::Done), refused);

    // Donde ya está: ni se mueve ni se rechaza, con los dos nombres.
    assert_eq!(can_drop(&client("resolved"), BoardStatus::Done), DropVerdict::Stay);
    assert_eq!(can_drop(&client("done"), BoardStatus::Done), DropVerdict::Stay);
    assert_eq!(can_drop(&client("closed"), BoardStatus::Closed), DropVerdict::Stay);
    assert_eq!(can_drop(&internal("open"), BoardStatus::Open), DropVerdict::Stay);
}

/// Soltar una tarjeta la mueve a la columna equivalente **de su lista**, con
/// el id que dice el servidor, y luego se vuelve a preguntar.
///
/// Roturas que atrapa: componer el id de la columna a mano; preguntar por las
/// columnas de otra lista; elegir por clase y mandar a «Hecha» lo que se
/// soltó en «Cerrada»; no volver a cargar (la tarjeta se quedaría con lo que
/// se le puso aquí y no con lo que dice el servidor).
#[tokio::test]
async fn a_drop_moves_the_card_to_that_column_of_its_own_list() {
    let rig = rig(vec![raw("t1", "open", "l1"), raw("t2", "open", "other-list")]).await;
    mount_columns(&rig.server, "l1", ok(columns_of("l1"))).await;
    mount_columns(&rig.server, "other-list", ok(columns_of("other-list"))).await;
    mount_move(&rig.server, "t2", ok(Value::Null)).await;

    let moved = rig.core.my_work().drop_on("t2", BoardStatus::Closed).await;

    assert_eq!(moved.unwrap(), true);
    assert_eq!(
        asked(&rig.server).await,
        [
            ("GET".to_string(), "/api/v1/task-lists/other-list/statuses".to_string(), Value::Null),
            ("POST".to_string(), "/api/v1/tasks/t2/move".to_string(), json!({ "statusId": "col-other-list-d" })),
            ("GET".to_string(), "/api/v1/tasks/".to_string(), Value::Null),
        ]
    );
    // Lo que queda es lo que contestó el servidor al volver a preguntar, de
    // la misma organización.
    let state = rig.core.my_work().state();
    assert_eq!(status_of(&state, "t2"), "open");
    let reload = rig.server.received_requests().await.unwrap().pop().unwrap();
    assert!(reload.url.query().unwrap().contains("orgId=o1"), "{:?}", reload.url.query());
}

/// La tarjeta cambia de columna antes de que conteste el servidor.
///
/// Rotura que atrapa: esperar la respuesta para moverla — la tarjeta vuelve a
/// su sitio al soltarla y se lee como que el arrastre no funcionó.
#[tokio::test]
async fn the_card_changes_column_before_the_server_answers() {
    let rig = rig(vec![raw("t1", "open", "l1")]).await;
    mount_columns(&rig.server, "l1", ok(columns_of("l1")).set_delay(Duration::from_millis(400))).await;
    mount_move(&rig.server, "t1", ok(Value::Null)).await;
    let work = rig.core.my_work();

    let meanwhile = async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let state = rig.core.my_work().state();
        let board = work_board(&state.tasks, false, Locale::Es, day(2026, 9, 30));
        (asked(&rig.server).await.len(), board[0].cards.len(), board[1].cards.len())
    };
    let (moved, seen) = tokio::join!(work.drop_on("t1", BoardStatus::InProgress), meanwhile);

    assert_eq!(moved.unwrap(), true);
    // A media petición: sólo se ha preguntado por las columnas, y la tarjeta
    // ya está en «En curso».
    assert_eq!(seen, (1, 0, 1));
}

/// Soltar donde la máquina no deja no pide nada, y dice por qué.
///
/// Roturas que atrapa: pedir el movimiento y dejar que conteste 409 el
/// servidor (la tarjeta se va, se queda un instante donde no puede estar y
/// vuelve); mover la tarjeta aquí aunque no se pueda; un rechazo sin el
/// código del catálogo.
#[tokio::test]
async fn a_forbidden_drop_asks_nothing_and_says_why() {
    let mut ticket = raw("t1", "pending", "l1");
    ticket["flow"] = json!("client");
    let rig = rig(vec![ticket]).await;
    let announced = rig.recorder.0.lock().unwrap().len();

    let refused = rig.core.my_work().drop_on("t1", BoardStatus::Done).await;

    match refused {
        Err(ApiError::Server { code, .. }) => assert_eq!(code, "bad-transition"),
        other => panic!("tenía que rechazarse: {other:?}"),
    }
    assert!(asked(&rig.server).await.is_empty());
    assert_eq!(status_of(&rig.core.my_work().state(), "t1"), "pending");
    assert_eq!(rig.recorder.0.lock().unwrap().len(), announced, "nada cambió, nada que anunciar");
    assert_eq!(
        rig.core.my_work().drop_verdict("t1", BoardStatus::Done),
        Some(DropVerdict::Refused { code: "bad-transition".into() })
    );
    assert_eq!(rig.core.my_work().drop_verdict("t1", BoardStatus::InProgress), Some(DropVerdict::Move));
    assert_eq!(rig.core.my_work().drop_verdict("ghost", BoardStatus::Done), None);
}

/// Soltarla donde ya estaba no cuesta una petición, y una tarjeta que ya no
/// existe tampoco.
///
/// Roturas que atrapa: comparar el estado crudo con la columna (`resolved`
/// no es `done`, y se pediría mover a donde ya está); pedir algo por una
/// tarjeta que se borró mientras se arrastraba.
#[tokio::test]
async fn a_drop_on_its_own_column_does_nothing() {
    let rig = rig(vec![raw("t1", "resolved", "l1")]).await;
    let work = rig.core.my_work();

    assert_eq!(work.drop_on("t1", BoardStatus::Done).await.unwrap(), false);
    assert_eq!(work.drop_on("ghost", BoardStatus::Done).await.unwrap(), false);

    assert!(asked(&rig.server).await.is_empty());
    assert_eq!(status_of(&work.state(), "t1"), "resolved");
}

/// Si el servidor no acepta el movimiento, la tarjeta vuelve a donde estaba y
/// sale su error.
///
/// Roturas que atrapa: dejar la tarjeta en la columna a la que no llegó;
/// tragarse el error; volver a cargar como si hubiera salido bien; devolverla
/// a otra columna que no es la suya; llevarse de paso las demás tarjetas.
#[tokio::test]
async fn a_drop_the_server_refuses_puts_the_card_back() {
    let rig = rig(vec![raw("t1", "in_progress", "l1"), raw("t2", "open", "l1")]).await;
    mount_columns(&rig.server, "l1", ok(columns_of("l1"))).await;
    let no = ResponseTemplate::new(409)
        .set_body_json(json!({ "success": false, "error": "subtasks-open", "message": "Close its subtasks first" }));
    mount_move(&rig.server, "t1", no).await;
    let announced = rig.recorder.0.lock().unwrap().len();

    let refused = rig.core.my_work().drop_on("t1", BoardStatus::Done).await;

    match refused {
        Err(ApiError::Server { status, code, .. }) => assert_eq!((status, code.as_str()), (409, "subtasks-open")),
        other => panic!("tenía que salir el error del servidor: {other:?}"),
    }
    let since: Vec<String> =
        rig.recorder.0.lock().unwrap()[announced..].iter().map(|state| status_of(state, "t1")).collect();
    assert_eq!(since, ["done", "in_progress"], "fue a «Hecha» y volvió");
    let neighbour: Vec<String> =
        rig.recorder.0.lock().unwrap()[announced..].iter().map(|state| status_of(state, "t2")).collect();
    assert_eq!(neighbour, ["open", "open"], "la de al lado no se mueve");
    let paths: Vec<String> = asked(&rig.server).await.into_iter().map(|(_, path, _)| path).collect();
    assert_eq!(paths, ["/api/v1/task-lists/l1/statuses", "/api/v1/tasks/t1/move"]);
}

/// Una lista sin columna para ese estado no mueve nada, y lo dice.
///
/// Roturas que atrapa: mandar la tarjeta a otra columna cualquiera de la
/// lista; dejarla puesta donde no hay columna; no decir qué pasó. Y si las
/// columnas no llegan, lo mismo.
#[tokio::test]
async fn a_list_without_that_column_moves_nothing() {
    let rig = rig(vec![raw("t1", "open", "l1"), raw("t2", "open", "l2")]).await;
    let only_open = json!([{ "id": "col-a", "listId": "l1", "name": "Open", "kind": "open", "status": "pending" }]);
    mount_columns(&rig.server, "l1", ok(only_open)).await;
    mount_columns(&rig.server, "l2", ResponseTemplate::new(500).set_body_json(json!({ "error": "boom", "message": "Boom" }))).await;
    let work = rig.core.my_work();

    match work.drop_on("t1", BoardStatus::Closed).await {
        Err(ApiError::Server { code, .. }) => assert_eq!(code, COLUMN_MISSING),
        other => panic!("tenía que decir que falta la columna: {other:?}"),
    }
    assert!(work.drop_on("t2", BoardStatus::InProgress).await.is_err());

    assert!(asked(&rig.server).await.iter().all(|(verb, _, _)| verb == "GET"));
    let state = work.state();
    assert_eq!((status_of(&state, "t1"), status_of(&state, "t2")), ("open".to_string(), "open".to_string()));
}

/// El tablero que se pinta respeta el ámbito, el filtro y el interruptor de
/// lo terminado.
///
/// Roturas que atrapa: pintar todo lo cargado ignorando a qué se acotó;
/// ignorar lo escrito en el buscador; decir «no lo pediste» de lo terminado
/// cuando sí se pidió.
#[tokio::test]
async fn what_the_board_paints_respects_scope_filter_and_the_closed_switch() {
    let mut other = raw("t2", "in_progress", "l2");
    other["title"] = json!("Revisión del diseño");
    let rig = rig(vec![raw("t1", "open", "l1"), other]).await;
    let work = rig.core.my_work();
    let ids = |filter: &TaskFilter| -> Vec<Vec<String>> {
        work.board(filter, Locale::Es)
            .into_iter()
            .map(|column| column.cards.into_iter().map(|card| card.id).collect())
            .collect()
    };
    let none: Vec<String> = Vec::new();

    assert_eq!(ids(&TaskFilter::default()), [vec!["t1".to_string()], vec!["t2".to_string()], none.clone(), none.clone()]);
    assert!(!work.board(&TaskFilter::default(), Locale::Es)[2].asked);

    let filter = TaskFilter { query: "revision".into(), ..TaskFilter::default() };
    assert_eq!(ids(&filter), [none.clone(), vec!["t2".to_string()], none.clone(), none.clone()]);

    work.set_scope(Some(WorkScope { kind: ScopeKind::List, id: "l1".into(), name: "Lista".into(), org_id: Some("o1".into()) }));
    assert_eq!(ids(&TaskFilter::default()), [vec!["t1".to_string()], none.clone(), none.clone(), none]);

    work.set_include_closed(true);
    assert!(work.board(&TaskFilter::default(), Locale::Es)[2].asked);
}
