//! El muro de feedback contra un servidor de mentira. Cada prueba dice la
//! rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{Config, Conversation, Core, NewPost, NoListener, PostAudience, ServerEvent, Storage};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[path = "support/gated.rs"]
mod gated;
use gated::{front, until, Gate};

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

struct Rig {
    server: MockServer,
    core: Core,
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn get(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(path(route.to_string()))
}

fn post(route: &str) -> wiremock::MockBuilder {
    Mock::given(method("POST")).and(path(route.to_string()))
}

fn card(id: &str, over: Value) -> Value {
    let mut card = json!({
        "id": id, "kind": "feedback", "audience": "platform", "orgId": "o1", "orgName": "Org o1", "authorId": "u9", "authorName": "Bea",
        "title": format!("t-{id}"), "body": "cuerpo", "createdAt": "2026-11-25T10:00:00Z", "comments": 0, "reactions": [],
        "unseen": false, "canDelete": false, "canEdit": false, "canPin": false, "pinned": false, "draft": false, "canPublish": false,
    });
    for (key, value) in over.as_object().cloned().unwrap_or_default() {
        card[key] = value;
    }
    card
}

fn detail(id: &str, over: Value, comments: Value) -> Value {
    let mut detail = card(id, over);
    detail["commentList"] = comments;
    detail
}

fn comment(id: &str, post: &str) -> Value {
    json!({ "id": id, "postId": post, "authorId": "u9", "authorName": "Bea", "body": format!("c-{id}"), "createdAt": "2026-11-25T11:00:00Z", "canDelete": true, "reactions": [] })
}

fn org(id: &str, role: &str) -> Value {
    json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": role, "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" })
}

/// Ana (`u1`), de `o1` con ese papel; el muro, con esas tarjetas.
async fn rig(role: &str, posts: Value) -> Rig {
    let server = MockServer::start().await;
    get("/api/v1/organizations/").respond_with(ok(json!([org("o1", role)]))).mount(&server).await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": posts }))).mount(&server).await;
    post("/api/v1/posts/seen").respond_with(ok(json!(null))).mount(&server).await;
    let core = Core::new(Config::new(front(&server.uri())), Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.orgs().load().await;
    Rig { server, core }
}

async fn sent(server: &MockServer, verb: &str, route: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == verb && request.url.path() == route)
        .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
        .collect()
}

async fn listings(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().iter().filter(|r| r.method.as_str() == "GET" && r.url.path() == "/api/v1/posts").count()
}

fn event(kind: &str, post: &str) -> ServerEvent {
    ServerEvent { event: kind.into(), data: json!({ "postId": post }).to_string() }
}

/// Abrir el muro lo pone delante de la conversación, lo pide, y sólo después
/// avisa de que se vio.
///
/// Roturas que atrapa: avisar de «visto» antes de pedirlo (el «nuevo» se
/// perdería en esta misma visita); no dejar el muro delante de la
/// conversación; apagar el «nuevo» de las tarjetas al abrirlo.
#[tokio::test]
async fn opening_the_wall_loads_it_then_says_it_was_seen() {
    let rig = rig("member", json!([card("a", json!({ "unseen": true })), card("b", json!({}))])).await;
    rig.core.wall().show();
    assert!(rig.core.comms().state().on_wall && rig.core.comms().state().on_screen);
    rig.core.wall().opened().await;

    let state = rig.core.wall().state();
    assert_eq!(state.posts.iter().map(|post| post.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert!(state.posts[0].unseen, "las tarjetas siguen marcando lo nuevo en esta visita");
    assert!(!state.has_new, "con el muro delante no hay punto");
    let order: Vec<String> = rig.server.received_requests().await.unwrap().iter().map(|r| format!("{} {}", r.method, r.url.path())).collect();
    let (list, seen) = (order.iter().position(|r| r == "GET /api/v1/posts").unwrap(), order.iter().position(|r| r == "POST /api/v1/posts/seen").unwrap());
    assert!(list < seen, "{order:?}");
}

/// Lo nuevo que llega por el stream. (Cuándo se enciende el punto lo prueba
/// `new_post_effect`, en el core.)
///
/// Roturas que atrapa: no encender el punto por lo que publica otra persona;
/// con el muro delante, no subir lo nuevo arriba o perder lo que había
/// cargado debajo; encender el punto con el muro delante.
#[tokio::test]
async fn the_dot_lights_for_what_others_post_and_the_wall_in_front_merges_it() {
    let rig = rig("member", json!([card("a", json!({}))])).await;
    rig.core.wall().load().await;
    assert!(!rig.core.wall().state().has_new);

    get("/api/v1/posts/theirs").respond_with(ok(detail("theirs", json!({}), json!([])))).mount(&rig.server).await;
    rig.core.stream().deliver(event("post:new", "theirs"));
    until("se enciende", || rig.core.wall().state().has_new).await;

    rig.core.wall().show();
    assert!(!rig.core.wall().state().has_new);
    rig.server.reset().await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": [card("theirs", json!({})), card("a", json!({}))] }))).mount(&rig.server).await;
    get("/api/v1/posts/theirs").respond_with(ok(detail("theirs", json!({}), json!([])))).mount(&rig.server).await;
    rig.core.stream().deliver(event("post:new", "theirs"));
    until("sube arriba", || rig.core.wall().state().posts.first().is_some_and(|post| post.id == "theirs")).await;
    assert_eq!(rig.core.wall().state().posts.len(), 2);
    assert!(!rig.core.wall().state().has_new);
}

/// Una reacción se ve al momento, en la tarjeta y en el modal, y se deshace si
/// el servidor la rechaza.
///
/// Roturas que atrapa: esperar al servidor para pintarla; ponerla sólo en la
/// tarjeta o sólo en el modal; no deshacerla al fallar; aceptar un emoji que
/// el servidor no conoce.
#[tokio::test]
async fn a_reaction_shows_at_once_and_is_undone_when_refused() {
    let rig = rig("member", json!([card("a", json!({}))])).await;
    get("/api/v1/posts/a").respond_with(ok(detail("a", json!({}), json!([])))).mount(&rig.server).await;
    rig.core.wall().load().await;
    rig.core.wall().open_post("a").await.unwrap();

    let gate = Gate::default();
    post("/api/v1/posts/a/reactions").respond_with(gate.hold(ResponseTemplate::new(400).set_body_json(json!({ "success": false, "error": "bad-request" })))).mount(&rig.server).await;
    let wall = rig.core.wall();
    let pending = tokio::spawn(async move { wall.react("a", "🎉", true).await });
    until("el servidor aún no contestó", || gate.held() == 1).await;
    let state = rig.core.wall().state();
    for reactions in [&state.posts[0].reactions, &state.open.as_ref().unwrap().reactions] {
        assert_eq!((reactions[0].emoji.as_str(), reactions[0].mine, reactions[0].names.clone()), ("🎉", true, vec!["Ana Ruiz".to_string()]));
    }
    gate.open();
    assert!(pending.await.unwrap().is_err());
    let state = rig.core.wall().state();
    assert!(state.posts[0].reactions.is_empty() && state.open.unwrap().reactions.is_empty(), "se deshace en las dos");
    assert!(rig.core.wall().react("a", "💩", true).await.is_err());
    assert_eq!(sent(&rig.server, "POST", "/api/v1/posts/a/reactions").await, [json!({ "emoji": "🎉", "on": true })]);
}

/// Comentar y borrar un comentario mantienen la cuenta de la tarjeta.
///
/// Roturas que atrapa: dejar la tarjeta con la cuenta vieja; mandar el
/// comentario sin recortar o uno en blanco; borrar el comentario de otra
/// publicación.
#[tokio::test]
async fn commenting_and_deleting_a_comment_keep_the_card_count() {
    let rig = rig("member", json!([card("a", json!({})), card("b", json!({}))])).await;
    get("/api/v1/posts/a").respond_with(ok(detail("a", json!({}), json!([comment("c1", "a")])))).mount(&rig.server).await;
    post("/api/v1/posts/a/comments").respond_with(ok(detail("a", json!({}), json!([comment("c1", "a"), comment("c2", "a")])))).mount(&rig.server).await;
    Mock::given(method("DELETE")).and(path("/api/v1/posts/a/comments/c1")).respond_with(ok(json!(null))).mount(&rig.server).await;
    rig.core.wall().load().await;
    rig.core.wall().open_post("a").await.unwrap();

    assert!(rig.core.wall().comment("a", "  \n").await.is_err());
    assert!(sent(&rig.server, "POST", "/api/v1/posts/a/comments").await.is_empty(), "en blanco no sale");
    rig.core.wall().comment("a", "  +1 \n").await.unwrap();
    assert_eq!(sent(&rig.server, "POST", "/api/v1/posts/a/comments").await, [json!({ "body": "+1" })]);
    let state = rig.core.wall().state();
    assert_eq!((state.posts[0].comments, state.posts[1].comments, state.open.as_ref().unwrap().comment_list.len()), (2, 0, 2));

    rig.core.wall().remove_comment("a", "c1").await.unwrap();
    let state = rig.core.wall().state();
    assert_eq!((state.posts[0].comments, state.open.unwrap().comment_list[0].id.clone()), (1, "c2".to_string()));
}

/// Publicar.
///
/// Roturas que atrapa: mandar un texto en blanco; que un viewer publique; no
/// volver a pedir el muro tras publicar (el orden lo da el servidor); mandar
/// la organización o la audiencia equivocadas.
#[tokio::test]
async fn posting_checks_first_sends_what_was_chosen_and_reloads() {
    let rig = rig("member", json!([])).await;
    post("/api/v1/posts").respond_with(ok(detail("n", json!({}), json!([])))).mount(&rig.server).await;
    let blank = NewPost { body: "  ".into(), ..Default::default() };
    assert!(rig.core.wall().create(blank).await.is_err());
    let idea = NewPost { announce: false, audience: PostAudience::Everyone, title: " Encuestas ".into(), body: "que haya".into() };
    rig.core.wall().create(idea).await.unwrap();
    assert_eq!(
        sent(&rig.server, "POST", "/api/v1/posts").await,
        [json!({ "kind": "feedback", "orgId": "o1", "audience": "everyone", "title": "Encuestas", "body": "que haya" })]
    );
    assert_eq!(listings(&rig.server).await, 1, "tras publicar se vuelve a pedir");

    let viewer = self::rig("viewer", json!([])).await;
    post("/api/v1/posts").respond_with(ok(detail("n", json!({}), json!([])))).mount(&viewer.server).await;
    assert!(!viewer.core.wall().permissions().can_publish());
    assert!(viewer.core.wall().create(NewPost { body: "hola".into(), ..Default::default() }).await.is_err());
    assert!(sent(&viewer.server, "POST", "/api/v1/posts").await.is_empty());
}

async fn gets(server: &MockServer, route: &str) -> usize {
    server.received_requests().await.unwrap().iter().filter(|r| r.method.as_str() == "GET" && r.url.path() == route).count()
}

/// Un respiro más largo que el de agrupar eventos: lo que iba a pedirse ya se
/// pidió, y contar después es contar todo.
async fn past_the_coalescing_window() {
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
}

/// Las reacciones y los comentarios no se piden evento a evento.
///
/// Con el muro fuera, se apuntan y se piden al abrirlo (sólo la publicación
/// abierta: la lista viene entera). Con el muro delante, una ráfaga es una
/// lectura por publicación, y lo que no está en pantalla no se pide.
///
/// Roturas que atrapa: una petición por evento con el muro fuera, o con el
/// muro delante; pedir lo que no se tiene; no releer la publicación abierta
/// al abrir el muro; no quitar lo borrado.
#[tokio::test]
async fn events_are_read_once_and_only_for_what_is_on_screen() {
    let rig = rig("member", json!([card("a", json!({})), card("b", json!({}))])).await;
    get("/api/v1/posts/a").respond_with(ok(detail("a", json!({ "comments": 3, "title": "nuevo" }), json!([])))).mount(&rig.server).await;
    get("/api/v1/posts/b").respond_with(ok(detail("b", json!({ "comments": 1 }), json!([])))).mount(&rig.server).await;
    get("/api/v1/posts/zzz").respond_with(ok(detail("zzz", json!({}), json!([])))).mount(&rig.server).await;
    rig.core.wall().load().await;
    rig.core.wall().open_post("a").await.unwrap();
    assert_eq!(gets(&rig.server, "/api/v1/posts/a").await, 1);

    // Con el muro fuera: nada se pide por cada evento.
    for _ in 0..3 {
        rig.core.stream().deliver(event("post:comment", "a"));
    }
    rig.core.stream().deliver(event("post:reaction", "zzz"));
    // Que los eventos se atiendan con el muro todavía fuera.
    tokio::task::yield_now().await;
    past_the_coalescing_window().await;
    assert_eq!(gets(&rig.server, "/api/v1/posts/a").await, 1, "con el muro fuera no se pide nada por los eventos");
    rig.core.wall().show();
    rig.server.reset().await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": [card("a", json!({})), card("b", json!({}))] }))).mount(&rig.server).await;
    get("/api/v1/posts/a").respond_with(ok(detail("a", json!({ "comments": 3, "title": "nuevo" }), json!([])))).mount(&rig.server).await;
    post("/api/v1/posts/seen").respond_with(ok(json!(null))).mount(&rig.server).await;
    rig.core.wall().opened().await;
    past_the_coalescing_window().await;
    assert_eq!(gets(&rig.server, "/api/v1/posts/a").await, 1, "una sola, al abrir el muro");
    assert_eq!(rig.core.wall().state().open.unwrap().title, "nuevo");
    assert_eq!(gets(&rig.server, "/api/v1/posts/zzz").await, 0, "lo que no está en pantalla no se pide");

    // Con el muro delante, la ráfaga es una lectura por publicación.
    get("/api/v1/posts/b").respond_with(ok(detail("b", json!({ "comments": 1 }), json!([])))).mount(&rig.server).await;
    for _ in 0..3 {
        rig.core.stream().deliver(event("post:reaction", "a"));
    }
    rig.core.stream().deliver(event("post:comment", "b"));
    rig.core.stream().deliver(event("post:comment", "zzz"));
    until("se lee b", || rig.core.wall().state().posts[1].comments == 1).await;
    past_the_coalescing_window().await;
    assert_eq!((gets(&rig.server, "/api/v1/posts/a").await, gets(&rig.server, "/api/v1/posts/b").await), (2, 1));
    assert_eq!(gets(&rig.server, "/api/v1/posts/zzz").await, 0);

    rig.core.stream().deliver(event("post:deleted", "b"));
    until("se quitó lo borrado", || rig.core.wall().state().posts.len() == 1).await;
}

/// Una publicación sólo se quita cuando el servidor dice que ya no existe.
///
/// Roturas que atrapa: quitar la tarjeta por un fallo de red o un 500; no
/// quitarla con un 404.
#[tokio::test]
async fn a_card_is_dropped_only_when_the_server_says_it_is_gone() {
    let rig = rig("member", json!([card("a", json!({})), card("b", json!({}))])).await;
    rig.core.wall().show();
    rig.core.wall().load().await;
    get("/api/v1/posts/a").respond_with(ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": "boom" }))).mount(&rig.server).await;
    get("/api/v1/posts/b").respond_with(ResponseTemplate::new(404).set_body_json(json!({ "success": false, "error": "not-found" }))).mount(&rig.server).await;

    rig.core.stream().deliver(event("post:updated", "a"));
    until_asked(&rig.server, "/api/v1/posts/a").await;
    // Su lectura es anterior a la de b (otro respiro): con b quitada, la de a
    // ya contestó.
    rig.core.stream().deliver(event("post:updated", "b"));
    until("b se quita", || rig.core.wall().state().posts.len() == 1).await;
    assert_eq!(rig.core.wall().state().posts[0].id, "a", "con un 500 la tarjeta se queda");
}

async fn until_asked(server: &MockServer, route: &str) {
    for _ in 0..300 {
        if gets(server, route).await > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no llegó a pedirse {route}");
}

/// Una página siguiente en camino no se pisa con una publicación nueva.
///
/// Roturas que atrapa: que subir lo nuevo con el muro delante le cambie el
/// cursor a «Cargar más» mientras la página siguiente viene; repetir una
/// publicación que la página siguiente trae otra vez (la nueva empujó a las
/// demás).
#[tokio::test]
async fn a_new_post_does_not_trample_a_next_page_in_flight() {
    let rig = rig("member", json!([])).await;
    let gate = Gate::default();
    get("/api/v1/posts").and(query_param("cursor", "c1")).respond_with(gate.hold(ok(json!({ "posts": [card("b", json!({})), card("c", json!({}))] })))).with_priority(1).mount(&rig.server).await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": [card("a", json!({})), card("b", json!({}))], "nextCursor": "c1" }))).up_to_n_times(1).with_priority(2).mount(&rig.server).await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": [card("n", json!({})), card("a", json!({})), card("b", json!({}))], "nextCursor": "otro" }))).with_priority(3).mount(&rig.server).await;
    get("/api/v1/posts/n").respond_with(ok(detail("n", json!({}), json!([])))).mount(&rig.server).await;
    rig.core.wall().show();
    rig.core.wall().load().await;
    assert_eq!(rig.core.wall().state().next_cursor.as_deref(), Some("c1"));

    let wall = rig.core.wall();
    let more = tokio::spawn(async move { wall.load_more().await });
    until("la página siguiente, retenida", || gate.held() == 1).await;

    rig.core.stream().deliver(event("post:new", "n"));
    until("lo nuevo sube", || rig.core.wall().state().posts.first().is_some_and(|post| post.id == "n")).await;
    assert_eq!(rig.core.wall().state().next_cursor.as_deref(), Some("c1"), "el cursor es de la página en camino");

    gate.open();
    more.await.unwrap();
    let state = rig.core.wall().state();
    assert_eq!(state.posts.iter().map(|post| post.id.as_str()).collect::<Vec<_>>(), ["n", "a", "b", "c"]);
    assert_eq!(state.next_cursor, None);
}

/// Una página siguiente que contesta después de volver a pedir la primera ya
/// no es la que sigue.
///
/// Roturas que atrapa: pegar debajo de la lista nueva lo que era la
/// continuación de la vieja.
#[tokio::test]
async fn a_next_page_that_answers_after_a_reload_is_dropped() {
    let rig = rig("member", json!([])).await;
    let gate = Gate::default();
    get("/api/v1/posts").and(query_param("cursor", "c1")).respond_with(gate.hold(ok(json!({ "posts": [card("viejo", json!({}))] })))).with_priority(1).mount(&rig.server).await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": [card("a", json!({}))], "nextCursor": "c1" }))).up_to_n_times(1).with_priority(2).mount(&rig.server).await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": [card("z", json!({}))] }))).with_priority(3).mount(&rig.server).await;
    rig.core.wall().load().await;
    let wall = rig.core.wall();
    let more = tokio::spawn(async move { wall.load_more().await });
    until("la página siguiente, retenida", || gate.held() == 1).await;

    rig.core.wall().load().await;
    gate.open();
    more.await.unwrap();
    let state = rig.core.wall().state();
    assert_eq!(state.posts.iter().map(|post| post.id.as_str()).collect::<Vec<_>>(), ["z"]);
    assert!(!state.loading);
}

/// El punto de «nuevo» tras volver de una caída, y lo nuevo que se ve con el
/// muro delante se da por visto.
///
/// Roturas que atrapa: que releer tras reconectar no encienda el punto por lo
/// no visto; encenderlo con el muro delante; no avisar al servidor de que lo
/// que subió ya se vio.
#[tokio::test]
async fn after_a_reconnect_the_dot_is_recomputed_and_what_is_merged_in_front_is_seen() {
    let rig = rig("member", json!([card("a", json!({}))])).await;
    rig.core.wall().load().await;
    assert!(!rig.core.wall().state().has_new);

    rig.server.reset().await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": [card("x", json!({ "unseen": true })), card("a", json!({}))] }))).mount(&rig.server).await;
    post("/api/v1/posts/seen").respond_with(ok(json!(null))).mount(&rig.server).await;
    rig.core.wall().refresh().await;
    assert!(rig.core.wall().state().has_new, "lo no visto tras reconectar enciende el punto");
    assert!(sent(&rig.server, "POST", "/api/v1/posts/seen").await.is_empty(), "fuera, no se da por visto");

    rig.core.wall().show();
    rig.core.wall().refresh().await;
    assert!(!rig.core.wall().state().has_new);
    assert_eq!(sent(&rig.server, "POST", "/api/v1/posts/seen").await.len(), 1, "delante, sí");
}

/// «Cargar más» pone la página que sigue debajo, con el cursor tal cual.
///
/// Roturas que atrapa: sustituir lo cargado en vez de añadir; no mandar el
/// cursor (o mandarlo sin codificar); seguir ofreciendo más al llegar al final.
#[tokio::test]
async fn load_more_appends_the_next_page() {
    let server = MockServer::start().await;
    get("/api/v1/posts").and(query_param("cursor", "a b&c")).respond_with(ok(json!({ "posts": [card("c", json!({}))] }))).with_priority(1).mount(&server).await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": [card("a", json!({})), card("b", json!({}))], "nextCursor": "a b&c" }))).with_priority(2).mount(&server).await;
    let core = Core::new(Config::new(front(&server.uri())), Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    core.sign_in(serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap(), "access".into(), "refresh".into()).unwrap();

    core.wall().load().await;
    assert_eq!(core.wall().state().next_cursor.as_deref(), Some("a b&c"));
    core.wall().load_more().await;
    let state = core.wall().state();
    assert_eq!(state.posts.iter().map(|post| post.id.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    assert_eq!(state.next_cursor, None);
}

/// El enlace de un aviso lleva al muro con esa publicación abierta, y elegir
/// una conversación después quita el muro.
///
/// Roturas que atrapa: que el enlace siga siendo «no está en esta app»; abrir
/// la publicación sin poner el muro delante; dejar el muro delante al elegir
/// un canal.
#[tokio::test]
async fn a_notice_link_opens_the_post_and_choosing_a_channel_takes_the_wall_away() {
    let rig = rig("member", json!([card("p1", json!({}))])).await;
    get("/api/v1/posts/p1").respond_with(ok(detail("p1", json!({}), json!([])))).mount(&rig.server).await;
    get("/api/v1/task-spaces/")
        .and(query_param("orgId", "o1"))
        .respond_with(ok(json!([{ "id": "g", "orgId": "o1", "name": "General", "color": "#888", "kind": "general", "folders": [], "lists": [], "people": [] }])))
        .mount(&rig.server)
        .await;
    rig.core.tree().load("o1").await;

    assert!(rig.core.inbox().open("/feedback?post=p1", "o1").await);
    let (comms, wall) = (rig.core.comms().state(), rig.core.wall().state());
    assert!(comms.on_screen && comms.on_wall);
    assert_eq!(wall.open.map(|post| post.id), Some("p1".to_string()));

    assert!(rig.core.comms().select(Conversation::Channel { space_id: "g".into() }));
    assert!(!rig.core.comms().state().on_wall);
}

/// Una primera página que contesta tarde no pisa a la de después.
///
/// Roturas que atrapa: poner lo que llegue sin mirar si es la última
/// petición (el muro volvería a lo de antes de la publicación nueva).
#[tokio::test]
async fn a_late_first_page_does_not_overwrite_a_newer_one() {
    let rig = rig("member", json!([])).await;
    let gate = Gate::default();
    get("/api/v1/posts").respond_with(gate.hold(ok(json!({ "posts": [card("viejo", json!({}))] })))).up_to_n_times(1).with_priority(1).mount(&rig.server).await;
    get("/api/v1/posts").respond_with(ok(json!({ "posts": [card("nuevo", json!({}))] }))).with_priority(2).mount(&rig.server).await;
    let wall = rig.core.wall();
    let first = tokio::spawn(async move { wall.load().await });
    until("la primera, retenida", || gate.held() == 1).await;
    rig.core.wall().load().await;
    gate.open();
    first.await.unwrap();
    assert_eq!(rig.core.wall().state().posts.iter().map(|post| post.id.as_str()).collect::<Vec<_>>(), ["nuevo"]);
}
