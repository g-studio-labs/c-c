//! La voz contra un servidor de mentira y un motor de mentira. Cada prueba
//! dice la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{Config, Core, ServerEvent, Session, Storage, VoiceEngine, VoiceEvent, VoiceFuture, VoicePhase, VoiceState};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, path_regex};
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

/// Un motor que apunta lo que se le pide. `join` tarda lo que diga `delay`.
#[derive(Default)]
struct FakeEngine {
    calls: Mutex<Vec<String>>,
    delay: Mutex<Duration>,
    fail: Mutex<Option<String>>,
}

impl FakeEngine {
    fn log(&self, line: impl Into<String>) {
        self.calls.lock().unwrap().push(line.into());
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl VoiceEngine for FakeEngine {
    fn join(&self, url: String, token: String, noise: bool) -> VoiceFuture<Result<String, String>> {
        self.log(format!("join {url} {token} noise={noise}"));
        let delay = *self.delay.lock().unwrap();
        let fail = self.fail.lock().unwrap().clone();
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            match fail {
                Some(code) => Err(code),
                None => Ok("u1".to_string()),
            }
        })
    }
    fn leave(&self) -> VoiceFuture<()> {
        self.log("leave");
        Box::pin(async {})
    }
    fn set_mic(&self, on: bool) {
        self.log(format!("mic {on}"));
    }
    fn set_deaf(&self, on: bool) {
        self.log(format!("deaf {on}"));
    }
    fn set_noise_suppression(&self, on: bool) {
        self.log(format!("noise {on}"));
    }
}

struct Rig {
    server: MockServer,
    core: Core,
    engine: Arc<FakeEngine>,
    storage: Arc<MemoryStorage>,
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn token(space: &str) -> Value {
    json!({ "url": "wss://sfu", "token": format!("t-{space}"), "room": format!("voice:{space}"), "orgId": "o1", "spaceName": format!("Canal {space}") })
}

async fn rig() -> Rig {
    rig_polling_every(Duration::from_millis(60)).await
}

async fn rig_polling_every(every: Duration) -> Rig {
    let server = MockServer::start().await;
    let org = json!({ "id": "o1", "name": "Org", "slug": "o1", "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" });
    Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org]))).mount(&server).await;
    for space in ["a", "b"] {
        Mock::given(method("POST")).and(path(format!("/api/v1/task-spaces/{space}/voice/token"))).respond_with(ok(token(space))).mount(&server).await;
    }
    let meet = json!({ "url": "wss://sfu", "token": "t-meet", "room": "meet:m1", "orgId": "o1", "spaceId": "a", "inviteId": "m1", "title": "Con el cliente", "sessionId": "cs1" });
    Mock::given(method("POST")).and(path("/api/v1/call-invites/m1/voice/token")).respond_with(ok(meet)).mount(&server).await;
    Mock::given(method("POST")).and(path_regex(r"/voice/ring$")).respond_with(ok(json!({ "ringId": "r1" }))).mount(&server).await;
    Mock::given(method("DELETE")).and(path_regex(r"/ring/")).respond_with(ok(json!(null))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/chat/voice-presence"))
        .respond_with(ok(json!({ "a": [{ "identity": "u2", "name": "Luis" }] })))
        .mount(&server)
        .await;

    let mut config = Config::new(server.uri());
    config.voice_presence_every = every;
    config.ring_for = Duration::from_millis(150);
    let storage = Arc::new(MemoryStorage::default());
    let core = Core::new(config, storage.clone(), Arc::new(Quiet)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.orgs().load().await;
    let engine = Arc::new(FakeEngine::default());
    core.set_voice_engine(engine.clone());
    Rig { server, core, engine, storage }
}

fn state(rig: &Rig) -> VoiceState {
    rig.core.voice().state()
}

async fn requests(server: &MockServer, verb: &str, route: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == verb && request.url.path() == route)
        .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
        .collect()
}

async fn asked(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().iter().filter(|request| request.url.path() == "/api/v1/chat/voice-presence").count()
}

async fn wait_until(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no llegó: {what}");
}

fn event(name: &str, data: Value) -> ServerEvent {
    ServerEvent { event: name.into(), data: data.to_string() }
}

/// Entrar pide el pase del canal, abre la sala mientras conecta y conecta con
/// él; lo que cuenta el motor pone a la gente, quién habla y quién está mudo;
/// quien se va sale de todo eso.
///
/// Roturas que atrapa: no abrir la sala hasta conectar; olvidar a quien se
/// fue en la lista de los que hablan o de los mudos; apuntar dos veces a la
/// misma persona; pasar la supresión de ruido que no es.
#[tokio::test]
async fn joining_a_channel_fills_the_room() {
    let rig = rig().await;
    *rig.engine.delay.lock().unwrap() = Duration::from_millis(80);
    let voice = rig.core.voice();
    let joining = tokio::spawn(async move { voice.join_channel("a", None, None).await });
    wait_until("la sala abierta mientras conecta", || state(&rig).phase == VoicePhase::Joining).await;
    assert!(state(&rig).stage_open);
    joining.await.unwrap();

    let now = state(&rig);
    assert_eq!((now.phase, now.space_id.as_deref(), now.me.as_deref(), now.mic), (VoicePhase::In, Some("a"), Some("u1"), true));
    assert_eq!(now.space_name.as_deref(), Some("Canal a"), "el nombre que da el pase si la lista no lo sabe");
    assert_eq!(rig.engine.calls(), ["join wss://sfu t-a noise=true"]);

    let voice = rig.core.voice();
    voice.engine_event(VoiceEvent::Joined { identity: "u2".into(), name: "Luis".into() });
    voice.engine_event(VoiceEvent::Joined { identity: "u2".into(), name: "Luis".into() });
    voice.engine_event(VoiceEvent::Muted { identity: "u2".into(), muted: true });
    voice.engine_event(VoiceEvent::Speaking { identities: vec!["u2".into()] });
    voice.engine_event(VoiceEvent::Latency { ms: 42 });
    let now = state(&rig);
    assert_eq!(now.people.len(), 1);
    assert_eq!((now.muted.get("u2"), now.speaking.clone(), now.latency_ms), (Some(&true), vec!["u2".to_string()], Some(42)));

    voice.engine_event(VoiceEvent::Left { identity: "u2".into() });
    let now = state(&rig);
    assert!(now.people.is_empty() && now.speaking.is_empty() && now.muted.is_empty());
}

/// Ensordecer silencia también el micrófono, y quitarlo no lo reabre.
///
/// Roturas que atrapa: que quitar la sordera devuelva el micrófono (volver
/// hablando sin querer); que ensordecer deje el micrófono abierto; que tu
/// recuadro no salga mudo.
#[tokio::test]
async fn undeafening_does_not_reopen_the_mic() {
    let rig = rig().await;
    rig.core.voice().join_channel("a", None, None).await;
    rig.core.voice().toggle_deaf();
    let now = state(&rig);
    assert!(now.deaf && !now.mic);
    assert_eq!(now.muted.get("u1"), Some(&true));
    rig.core.voice().toggle_deaf();
    let now = state(&rig);
    assert!(!now.deaf && !now.mic, "el micrófono sigue cerrado");
    assert_eq!(rig.engine.calls()[1..], ["deaf true", "mic false", "deaf false"]);

    rig.core.voice().toggle_mic();
    assert!(state(&rig).mic);
    assert_eq!(rig.engine.calls().last().unwrap(), "mic true");
}

/// Una sala a la vez: entrar en otra sale de la primera, y volver a pulsar
/// la misma no reconecta.
///
/// Roturas que atrapa: dos salas abiertas; reconectar la misma (cortaría la
/// conversación para dejarla igual).
#[tokio::test]
async fn one_room_at_a_time() {
    let rig = rig().await;
    rig.core.voice().join_channel("a", None, None).await;
    rig.core.voice().join_channel("a", None, None).await;
    assert_eq!(rig.engine.calls(), ["join wss://sfu t-a noise=true"]);
    rig.core.voice().join_channel("b", None, None).await;
    assert_eq!(rig.engine.calls(), ["join wss://sfu t-a noise=true", "leave", "join wss://sfu t-b noise=true"]);
    assert_eq!(state(&rig).space_id.as_deref(), Some("b"));
}

/// Si se sale mientras conecta, la conexión que llega tarde se cuelga y no
/// deja la llamada por dentro.
///
/// Rotura que atrapa: quitar el turno de la entrada (quedaría un micrófono
/// abierto en una sala que la pantalla dice que dejaste).
#[tokio::test]
async fn a_late_connection_is_hung_up() {
    let rig = rig().await;
    *rig.engine.delay.lock().unwrap() = Duration::from_millis(150);
    let voice = rig.core.voice();
    let joining = tokio::spawn(async move { voice.join_channel("a", None, None).await });
    wait_until("conectando", || rig.engine.calls().len() == 1).await;
    rig.core.voice().leave().await;
    joining.await.unwrap();
    assert_eq!(state(&rig).phase, VoicePhase::Out);
    assert_eq!(rig.engine.calls().iter().filter(|call| *call == "leave").count(), 2, "salir, y colgar la que llegó tarde");
}

/// Un fallo del motor se dice con su etiqueta y de qué canal es, y un corte
/// del servidor con su motivo.
///
/// Roturas que atrapa: perder de qué canal es el error (el botón de cada
/// canal se pintaría en rojo); seguir «dentro» tras un corte; enseñar el
/// motivo crudo de LiveKit.
#[tokio::test]
async fn failures_say_what_and_where() {
    let rig = rig().await;
    *rig.engine.fail.lock().unwrap() = Some("voice-no-mic".into());
    rig.core.voice().join_channel("a", None, None).await;
    let now = state(&rig);
    assert_eq!(now.phase, VoicePhase::Out);
    let error = now.error.unwrap();
    assert_eq!((error.code.as_str(), error.space_id.as_deref()), ("voice-no-mic", Some("a")));

    *rig.engine.fail.lock().unwrap() = None;
    rig.core.voice().join_channel("b", None, None).await;
    assert!(state(&rig).error.is_none(), "entrar limpia el fallo anterior");
    rig.core.voice().engine_event(VoiceEvent::Recording { active: true, id: "rec1".into(), by: "Luis".into(), since: "2026-11-19T10:00:00Z".into() });
    assert_eq!(state(&rig).recording.unwrap().by, "Luis");
    rig.core.voice().engine_event(VoiceEvent::Disconnected { reason: "DuplicateIdentity".into() });
    let now = state(&rig);
    assert_eq!(now.phase, VoicePhase::Out);
    assert!(now.recording.is_none(), "el punto rojo se va con la llamada");
    assert_eq!(now.error.unwrap().code, "voice-joined-elsewhere");
    rig.core.voice().engine_event(VoiceEvent::Joined { identity: "u9".into(), name: "Nadie".into() });
    assert!(state(&rig).people.is_empty(), "fuera de la sala no se escucha al motor");
}

/// Si la llamada del canal se acaba de mudar a una reunión, el pase lo dice y
/// se entra a la reunión. Y si se muda estando dentro, se va detrás, salvo
/// quien la mudó.
///
/// Roturas que atrapa: entrar al canal vacío; no seguir la mudanza; seguirla
/// también quien la hizo; seguir una de otro canal.
#[tokio::test]
async fn a_moved_call_is_followed() {
    let rig = rig().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/task-spaces/c/voice/token"))
        .respond_with(ok(json!({ "url": "wss://sfu", "token": "t-c", "room": "voice:c", "orgId": "o1", "spaceName": "c", "movedTo": { "inviteId": "m1", "title": "Con el cliente" } })))
        .mount(&rig.server)
        .await;
    rig.core.voice().join_channel("c", None, None).await;
    let now = state(&rig);
    assert_eq!((now.meet_id.as_deref(), now.space_id.as_deref(), now.title.as_deref()), (Some("m1"), None, Some("Con el cliente")));
    assert_eq!(now.meet_space_id.as_deref(), Some("a"));
    assert_eq!(now.moved.unwrap().invite_id, "m1");
    assert_eq!(rig.engine.calls().last().unwrap(), "join wss://sfu t-meet noise=true");

    rig.core.voice().clear_moved();
    rig.core.voice().join_channel("a", None, None).await;
    let moved = |actor: &str, space: &str| event("voice.moved", json!({ "spaceId": space, "inviteId": "m1", "title": "Con el cliente", "actorId": actor }));
    rig.core.stream().deliver(moved("u1", "a"));
    rig.core.stream().deliver(moved("u2", "b"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state(&rig).space_id.as_deref(), Some("a"), "ni la mía ni la de otro canal");
    rig.core.stream().deliver(moved("u2", "a"));
    wait_until("detrás de la mudanza", || state(&rig).meet_id.as_deref() == Some("m1") && state(&rig).phase == VoicePhase::In).await;
}

/// Quién anda por los canales se pregunta sólo mientras alguien mira, de la
/// organización que se mira.
///
/// Roturas que atrapa: preguntar sin nadie mirando; no parar al dejar de
/// mirar; parar cuando aún mira otro; preguntar sin decir de qué
/// organización.
#[tokio::test]
async fn occupancy_is_asked_only_while_watched() {
    let rig = rig().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(asked(&rig.server).await, 0, "nadie mira");

    let first = rig.core.voice().watch_occupancy();
    let second = rig.core.voice().watch_occupancy();
    wait_until("la ocupación", || state(&rig).occupancy.get("a").is_some_and(|peers| peers[0].name == "Luis")).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let while_watching = asked(&rig.server).await;
    assert!(while_watching >= 3, "cada poco: {while_watching}");
    drop(first);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(asked(&rig.server).await > while_watching, "otro sigue mirando");
    drop(second);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let stopped = asked(&rig.server).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(asked(&rig.server).await, stopped, "nadie mira: se para");
    let queries: Vec<String> = rig.server.received_requests().await.unwrap().iter().filter(|r| r.url.path() == "/api/v1/chat/voice-presence").map(|r| r.url.query().unwrap_or_default().to_string()).collect();
    assert!(queries.iter().all(|query| query == "orgId=o1"));
}

/// Llamar a alguien: suena, se rinde a su hora, y colgar le calla el
/// teléfono. Si dice que no, queda «no contestó».
///
/// Roturas que atrapa: no rendirse nunca (el servidor no guarda el timbre);
/// salir de la sala dejándole sonar; llamar al canal desde una reunión; que
/// un rechazo borre la fila en vez de decir «no contestó».
#[tokio::test]
async fn ringing_someone_gives_up_and_hangs_up() {
    let rig = rig().await;
    rig.core.voice().join_channel("a", None, None).await;
    rig.core.voice().ring("u2", "Luis").await;
    assert_eq!(requests(&rig.server, "POST", "/api/v1/task-spaces/a/voice/ring").await, [json!({ "userId": "u2" })]);
    let outgoing = state(&rig).outgoing.unwrap();
    assert!(!outgoing.unanswered && outgoing.name == "Luis");
    wait_until("no contestó", || state(&rig).outgoing.is_some_and(|ring| ring.unanswered)).await;

    rig.core.voice().ring("u2", "Luis").await;
    rig.core.stream().deliver(event("voice.ring.cancel", json!({ "spaceId": "a", "from": "u2" })));
    assert!(state(&rig).outgoing.unwrap().unanswered, "dijo que no");

    rig.core.voice().ring("u3", "Eva").await;
    rig.core.voice().leave().await;
    assert_eq!(requests(&rig.server, "DELETE", "/api/v1/task-spaces/a/voice/ring/u3").await.len(), 1, "salir le calla el teléfono");
    assert!(state(&rig).outgoing.is_none());
}

/// Te llaman: la tarjeta sale, se va sola a su hora, y no sale si ya estás en
/// esa sala. Aceptar entra (aunque sea otra organización); rechazar se lo
/// dice a quien llamaba.
///
/// Roturas que atrapa: una tarjeta que te invita a donde ya estás; una que no
/// caduca si nadie cancela; aceptar sin entrar; rechazar sin avisar; colgar
/// el timbre de otro.
#[tokio::test]
async fn being_rung() {
    let rig = rig().await;
    let ring = |space: &str, from: &str, secs: i64| {
        let expires = (chrono::Utc::now() + chrono::Duration::milliseconds(secs)).to_rfc3339();
        event("voice.ring", json!({ "ringId": format!("r-{space}-{from}"), "spaceId": space, "orgId": "o1", "spaceName": "Canal", "from": { "id": from, "name": "Luis" }, "expiresAt": expires }))
    };
    rig.core.stream().deliver(ring("a", "u2", 150));
    assert_eq!(state(&rig).incoming.unwrap().from.id, "u2");
    wait_until("caduca", || state(&rig).incoming.is_none()).await;

    rig.core.stream().deliver(ring("a", "u2", 5_000));
    rig.core.stream().deliver(event("voice.ring.cancel", json!({ "spaceId": "a", "from": "u3" })));
    assert!(state(&rig).incoming.is_some(), "la cancelación de otro no la calla");
    rig.core.stream().deliver(event("voice.ring.cancel", json!({ "spaceId": "a", "from": "u2" })));
    assert!(state(&rig).incoming.is_none(), "colgó quien llamaba");

    rig.core.stream().deliver(ring("b", "u2", 5_000));
    rig.core.voice().decline_ring().await;
    assert_eq!(requests(&rig.server, "DELETE", "/api/v1/task-spaces/b/voice/ring/u2").await.len(), 1);

    rig.core.stream().deliver(ring("a", "u2", 5_000));
    rig.core.voice().accept_ring().await;
    let now = state(&rig);
    assert_eq!((now.phase, now.space_id.as_deref(), now.incoming.is_none()), (VoicePhase::In, Some("a"), true));

    rig.core.stream().deliver(ring("a", "u3", 5_000));
    assert!(state(&rig).incoming.is_none(), "ya estás en esa sala");

    // A una reunión: aceptar lleva a la reunión, no al canal del que cuelga.
    let expires = (chrono::Utc::now() + chrono::Duration::seconds(5)).to_rfc3339();
    rig.core.stream().deliver(event("voice.ring", json!({ "ringId": "r-meet", "spaceId": "a", "orgId": "o1", "spaceName": "Canal", "from": { "id": "u2", "name": "Luis" }, "expiresAt": expires, "inviteId": "m1", "title": "Con el cliente" })));
    assert!(state(&rig).incoming.is_some(), "estar en el canal no es estar en su reunión");
    rig.core.voice().accept_ring().await;
    let now = state(&rig);
    assert_eq!((now.meet_id.as_deref(), now.space_id.as_deref()), (Some("m1"), None));
}

/// En una llamada no suena ningún aviso: se metería en lo que oyes.
/// Rotura que atrapa: que el tono de un mensaje suene en mitad de la llamada.
#[tokio::test]
async fn notices_keep_quiet_in_a_call() {
    let rig = rig().await;
    assert!(rig.core.notice_sound_allowed());
    rig.core.voice().join_channel("a", None, None).await;
    assert!(!rig.core.notice_sound_allowed());
    rig.core.voice().leave().await;
    assert!(rig.core.notice_sound_allowed());
}

/// La supresión de ruido se recuerda y va con cada entrada; cambiarla en una
/// llamada se aplica ya.
/// Roturas que atrapa: olvidarla al reiniciar; no pasarla al entrar.
#[tokio::test]
async fn noise_suppression_is_remembered() {
    let rig = rig().await;
    rig.core.voice().set_noise_suppression(false);
    let again = Core::new(Config::new(rig.server.uri()), rig.storage.clone(), Arc::new(Quiet)).unwrap();
    assert!(!again.voice().state().noise_suppression, "al volver a abrir la app");
    rig.core.voice().join_channel("a", None, None).await;
    assert_eq!(rig.engine.calls(), ["join wss://sfu t-a noise=false"]);
    rig.core.voice().set_noise_suppression(true);
    assert_eq!(rig.engine.calls().last().unwrap(), "noise true");
}

/// Sin motor (macOS y Windows, hoy) entrar lo dice en vez de quedarse
/// conectando para siempre.
#[tokio::test]
async fn without_an_engine_it_says_so() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/v1/task-spaces/a/voice/token")).respond_with(ok(token("a"))).mount(&server).await;
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(Quiet)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.voice().join_channel("a", None, None).await;
    let now = core.voice().state();
    assert_eq!(now.phase, VoicePhase::Out);
    assert_eq!(now.error.unwrap().code, "voice-not-in-this-app");
}

/// Al entrar o salir uno mismo se vuelve a preguntar quién hay, sin esperar
/// a la siguiente vuelta: la lista del canal tiene que enseñarte dentro, o ya
/// fuera.
/// Rotura que atrapa: esperar quince segundos para verte en tu propio canal.
#[tokio::test]
async fn joining_or_leaving_asks_again_who_is_there() {
    let rig = rig_polling_every(Duration::from_secs(60)).await;
    let _watch = rig.core.voice().watch_occupancy();
    wait_until("la primera pregunta", || state(&rig).occupancy.contains_key("a")).await;
    let before = asked(&rig.server).await;
    rig.core.voice().join_channel("a", None, None).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let joined = asked(&rig.server).await;
    assert_eq!(joined, before + 1, "al entrar");
    rig.core.voice().leave().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(asked(&rig.server).await, joined + 1, "al salir");
}

/// Si el servidor no da el pase, se dice por qué y de qué canal, y no se
/// queda «conectando».
/// Rotura que atrapa: tragarse el rechazo (la sala se cerraría sin decir
/// nada, como un botón que no hace caso).
#[tokio::test]
async fn a_refused_entry_says_why() {
    let rig = rig().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/task-spaces/x/voice/token"))
        .respond_with(ResponseTemplate::new(501).set_body_json(json!({ "success": false, "error": "voice-unconfigured", "message": "Voice is not configured" })))
        .mount(&rig.server)
        .await;
    rig.core.voice().join_channel("x", None, None).await;
    let now = state(&rig);
    assert_eq!(now.phase, VoicePhase::Out);
    let error = now.error.unwrap();
    assert_eq!((error.code.as_str(), error.space_id.as_deref()), ("voice-unconfigured", Some("x")));
    assert_eq!(cac_core::voice_error_phrase(&error, cac_core::Locale::Es), "La voz no está montada en este servidor.");
    assert!(rig.engine.calls().is_empty(), "sin pase no se llama al motor");
}
