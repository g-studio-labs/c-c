//! La voz contra un servidor de mentira y un motor de mentira. Cada prueba
//! dice la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cac_core::{voice_tiles, Config, Core, Locale, Publishing, ScreenSource, ScreenSources, ServerEvent, Session, Storage, VideoSource, VoiceEngine, VoiceEvent, VoiceFocus, VoiceFuture, VoiceLayout, VoicePhase, VoiceState};
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
    /// Lo que se le dijo de mirar el vídeo, aparte de `calls`.
    video: Mutex<Vec<bool>>,
    /// Lo que contesta a `screen_sources`.
    sources: Mutex<ScreenSources>,
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
    fn watch_video(&self, on: bool) {
        self.video.lock().unwrap().push(on);
    }
    fn set_camera(&self, on: bool) {
        self.log(format!("camera {on}"));
    }
    fn screen_sources(&self) -> ScreenSources {
        self.sources.lock().unwrap().clone()
    }
    fn share_screen(&self, source: Option<ScreenSource>) {
        self.log(format!("share {}", source.map(|s| s.id).unwrap_or_else(|| "system".into())));
    }
    fn stop_share(&self) {
        self.log("stop share");
    }
}

fn source(id: &str) -> ScreenSource {
    ScreenSource { id: id.into(), title: format!("Fuente {id}") }
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
    wait_until("el motor suelta el micrófono", || rig.engine.calls().last().map(String::as_str) == Some("leave")).await;
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

/// A quién se ofrece llamar: la gente de la organización menos tú y quien ya
/// está dentro.
/// Rotura que atrapa: ofrecer llamar a quien te está oyendo, o a ti.
#[tokio::test]
async fn you_can_ring_whoever_is_not_inside() {
    let rig = rig().await;
    let people = json!([{ "id": "u1", "username": "ana" }, { "id": "u2", "username": "luis" }, { "id": "u3", "username": "eva", "name": "Eva Paz" }]);
    Mock::given(method("GET")).and(path("/api/v1/users/search")).respond_with(ok(people)).mount(&rig.server).await;
    rig.core.comms().load_people().await;
    rig.core.voice().join_channel("a", None, None).await;
    rig.core.voice().engine_event(VoiceEvent::Joined { identity: "u2".into(), name: "Luis".into() });
    let ids: Vec<String> = rig.core.voice().invitable().into_iter().map(|person| person.id).collect();
    assert_eq!(ids, ["u3"]);
}

/// El micrófono y el altavoz elegidos se recuerdan en esta máquina, cada uno
/// por su lado.
/// Roturas que atrapa: olvidarlos al reiniciar; que elegir el altavoz borre
/// el micrófono.
#[tokio::test]
async fn chosen_devices_are_remembered() {
    let rig = rig().await;
    rig.core.voice().choose_device(false, Some("alsa:mic".into()));
    rig.core.voice().choose_device(true, Some("alsa:speaker".into()));
    let again = Core::new(Config::new(rig.server.uri()), rig.storage.clone(), Arc::new(Quiet)).unwrap();
    let choices = again.voice().device_choices();
    assert_eq!((choices.input.as_deref(), choices.output.as_deref()), (Some("alsa:mic"), Some("alsa:speaker")));
    again.voice().choose_device(false, None);
    assert_eq!(again.voice().device_choices().output.as_deref(), Some("alsa:speaker"));
    assert_eq!(again.voice().device_choices().input, None);
}

/// Entrar con la sala abierta mira el vídeo, minimizar deja de recibirlo y
/// volver lo pide otra vez; fuera de una sala no se le pide nada al motor.
///
/// Roturas que atrapa: no pedirlo al entrar (la sala en negro); minimizar
/// sin avisar al motor (se sigue recibiendo y convirtiendo vídeo que nadie
/// ve); volver sin pedirlo; avisar con la llamada ya colgada.
#[tokio::test]
async fn minimizing_stops_the_video() {
    let rig = rig().await;
    let voice = rig.core.voice();
    voice.join_channel("a", None, None).await;
    voice.close_stage();
    voice.open_stage();
    voice.leave().await;
    voice.close_stage();
    assert_eq!(*rig.engine.video.lock().unwrap(), [true, false, true]);
}

/// Lo que cuenta el motor del vídeo llega al estado: cámaras, pantallas por
/// orden, y quien se va se lleva lo suyo y el foco si era suyo.
///
/// Roturas que atrapa: no escuchar el evento de vídeo; dejar la pantalla o
/// la cámara de quien se fue; un foco que apunta a alguien que ya no está;
/// colgar y que la siguiente sala empiece con las pantallas de la anterior.
#[tokio::test]
async fn video_follows_what_the_engine_tells() {
    let rig = rig().await;
    let voice = rig.core.voice();
    voice.join_channel("a", None, None).await;
    for (identity, name) in [("u2", "Luis"), ("u3", "Eva")] {
        voice.engine_event(VoiceEvent::Joined { identity: identity.into(), name: name.into() });
    }
    voice.engine_event(VoiceEvent::Video { identity: "u2".into(), source: VideoSource::Camera, enabled: true });
    voice.engine_event(VoiceEvent::Video { identity: "u2".into(), source: VideoSource::Screen, enabled: true });
    voice.engine_event(VoiceEvent::Video { identity: "u3".into(), source: VideoSource::Screen, enabled: true });
    voice.toggle_focus("u2", VideoSource::Camera);
    let now = state(&rig);
    assert_eq!((now.cameras.clone(), now.screens.clone()), (vec!["u2".to_string()], vec!["u2".to_string(), "u3".to_string()]));
    assert_eq!(now.focus, Some(VoiceFocus { identity: "u2".into(), source: VideoSource::Camera }));

    voice.engine_event(VoiceEvent::Left { identity: "u2".into() });
    let now = state(&rig);
    assert!(now.cameras.is_empty(), "su cámara se fue con él");
    assert_eq!(now.screens, ["u3"], "su pantalla también, y pasa la siguiente");
    assert_eq!(now.focus, None, "el foco era suyo");

    voice.leave().await;
    assert!(state(&rig).screens.is_empty(), "nada de esta sala para la siguiente");
}

/// Pulsar un recuadro lo lleva al escenario y pulsarlo otra vez lo devuelve;
/// un recuadro sin vídeo no se lleva.
///
/// Roturas que atrapa: el segundo clic que no devuelve; llevar al escenario
/// una cámara apagada; la tira de caras que no se abre o que sobrevive al
/// escenario.
#[tokio::test]
async fn clicking_a_tile_takes_it_to_the_stage_and_back() {
    let rig = rig().await;
    let voice = rig.core.voice();
    voice.join_channel("a", None, None).await;
    voice.engine_event(VoiceEvent::Joined { identity: "u2".into(), name: "Luis".into() });
    voice.toggle_focus("u2", VideoSource::Camera);
    assert_eq!(state(&rig).focus, None, "sin cámara no hay nada que llevar");
    voice.engine_event(VoiceEvent::Video { identity: "u2".into(), source: VideoSource::Camera, enabled: true });
    voice.toggle_focus("u2", VideoSource::Camera);
    assert!(state(&rig).focus.is_some());
    voice.show_faces();
    assert!(state(&rig).faces_out);
    voice.toggle_focus("u2", VideoSource::Camera);
    let now = state(&rig);
    assert_eq!((now.focus, now.faces_out), (None, false));
}

/// Lo que se le pidió al motor después de entrar.
fn after_join(rig: &Rig) -> Vec<String> {
    rig.engine.calls().into_iter().filter(|call| !call.starts_with("join")).collect()
}

fn own(identity: &str, source: VideoSource, enabled: bool) -> VoiceEvent {
    VoiceEvent::Video { identity: identity.into(), source, enabled }
}

/// Encender la cámara espera al motor: el botón arranca, y sólo su evento lo
/// enciende y te pone en tu recuadro. Apagar es inmediato.
///
/// Roturas que atrapa: el botón encendido antes de que la cámara publique
/// (optimista, como el micro); tu recuadro con vídeo antes de que haya;
/// pulsar apagar sin decírselo al motor; tu recuadro con vídeo después de
/// apagar.
#[tokio::test]
async fn the_camera_is_on_only_when_the_engine_says_so() {
    let rig = rig().await;
    let voice = rig.core.voice();
    voice.join_channel("a", None, None).await;
    voice.toggle_camera();
    let now = state(&rig);
    assert_eq!(now.camera, Publishing::Starting);
    assert!(!voice_tiles(&now, Locale::Es)[0].camera, "sin vídeo todavía");
    assert_eq!(after_join(&rig), ["camera true"]);

    voice.engine_event(own("u1", VideoSource::Camera, true));
    let now = state(&rig);
    assert_eq!(now.camera, Publishing::On);
    assert!(voice_tiles(&now, Locale::Es)[0].camera, "tu cámara sale en tu recuadro");

    voice.toggle_camera();
    let now = state(&rig);
    assert_eq!(now.camera, Publishing::Off);
    assert!(!voice_tiles(&now, Locale::Es)[0].camera);
    assert_eq!(after_join(&rig), ["camera true", "camera false"]);
}

/// Un fallo al encender deja el botón como estaba y dice por qué; un fallo
/// con la cámara ya publicando no la apaga por su cuenta. Y el «apagado»
/// tardío de un intento anterior no apaga el que está arrancando.
///
/// Roturas que atrapa: quedarse girando para siempre tras un fallo; perder
/// la frase; apagar el botón por un aviso que no dice que paró; que apagar y
/// encender seguidos deje el botón apagado con la cámara arrancando.
#[tokio::test]
async fn a_failure_puts_the_button_back_and_says_why() {
    let rig = rig().await;
    let voice = rig.core.voice();
    voice.join_channel("a", None, None).await;
    voice.toggle_camera();
    voice.engine_event(VoiceEvent::VideoFailed { source: VideoSource::Camera, code: "voice-camera-busy".into() });
    let now = state(&rig);
    assert_eq!(now.camera, Publishing::Off);
    assert_eq!(now.error.map(|e| e.code).as_deref(), Some("voice-camera-busy"));

    voice.toggle_share();
    voice.engine_event(own("u1", VideoSource::Screen, true));
    voice.engine_event(VoiceEvent::VideoFailed { source: VideoSource::Screen, code: "voice-screen-thread-died".into() });
    assert_eq!(state(&rig).sharing, Publishing::On, "si paró, lo dirá su propio evento");

    voice.toggle_camera();
    voice.engine_event(own("u1", VideoSource::Camera, true));
    voice.toggle_camera();
    voice.toggle_camera();
    voice.engine_event(own("u1", VideoSource::Camera, false));
    assert_eq!(state(&rig).camera, Publishing::Starting, "ese apagado era del intento anterior");
    voice.engine_event(own("u1", VideoSource::Camera, true));
    assert_eq!(state(&rig).camera, Publishing::On);
}

/// Sin dónde elegir se comparte directo (lo único que hay, o lo que elija el
/// sistema); con varias fuentes se abre el selector y elegir manda ésa.
/// Pulsar compartiendo para.
///
/// Roturas que atrapa: abrir el selector con una sola fuente; compartir
/// cualquiera con varias; elegir y mandar otra; un clic tardío que comparte
/// tras cancelar; no parar al pulsar otra vez.
#[tokio::test]
async fn one_source_shares_at_once_and_several_ask() {
    let rig = rig().await;
    let voice = rig.core.voice();
    voice.join_channel("a", None, None).await;

    voice.toggle_share();
    assert_eq!(state(&rig).sharing, Publishing::Starting);
    voice.toggle_share();
    *rig.engine.sources.lock().unwrap() = ScreenSources { screens: vec![source("s1")], windows: vec![] };
    voice.toggle_share();
    assert_eq!(after_join(&rig), ["share system", "stop share", "share s1"]);
    voice.toggle_share();

    let several = ScreenSources { screens: vec![source("s1")], windows: vec![source("w7")] };
    *rig.engine.sources.lock().unwrap() = several.clone();
    voice.toggle_share();
    let now = state(&rig);
    assert_eq!((now.picking, now.sharing), (Some(several), Publishing::Off), "el selector, y nada pedido todavía");
    voice.cancel_pick();
    voice.pick_screen(source("w7"));
    assert_eq!(after_join(&rig).len(), 4, "cancelado no comparte");

    voice.toggle_share();
    voice.pick_screen(source("w7"));
    let now = state(&rig);
    assert_eq!((now.picking, now.sharing), (None, Publishing::Starting));
    assert_eq!(after_join(&rig).last().unwrap(), "share w7");
}

/// Tu pantalla va al escenario como cualquiera, por orden de llegada: no le
/// quitas el sitio a quien ya compartía, ni otra que llega después te lo
/// quita a ti. Y el escenario sabe que es tuya.
///
/// Roturas que atrapa: tu pantalla fuera del escenario; colarte delante de
/// quien compartía; que otra te quite el sitio; el escenario sin saber que
/// es tuya (no saldría «Estás compartiendo»).
#[tokio::test]
async fn your_screen_takes_its_turn_on_the_stage() {
    let rig = rig().await;
    let voice = rig.core.voice();
    voice.join_channel("a", None, None).await;
    voice.engine_event(VoiceEvent::Joined { identity: "u2".into(), name: "Luis".into() });
    voice.engine_event(VoiceEvent::Joined { identity: "u3".into(), name: "Eva".into() });
    voice.toggle_share();
    voice.engine_event(own("u1", VideoSource::Screen, true));
    voice.engine_event(own("u2", VideoSource::Screen, true));
    let VoiceLayout::Stage { on_stage, .. } = cac_core::voice_layout(&state(&rig), Locale::Es) else { panic!("tu pantalla va al escenario") };
    assert_eq!((on_stage.identity.as_str(), on_stage.me), ("u1", true));

    voice.toggle_share();
    let VoiceLayout::Stage { on_stage, .. } = cac_core::voice_layout(&state(&rig), Locale::Es) else { panic!() };
    assert_eq!((on_stage.identity.as_str(), on_stage.me), ("u2", false), "al parar, pasa la siguiente");

    voice.toggle_share();
    voice.engine_event(own("u1", VideoSource::Screen, true));
    let VoiceLayout::Stage { on_stage, .. } = cac_core::voice_layout(&state(&rig), Locale::Es) else { panic!() };
    assert_eq!(on_stage.identity, "u2", "quien ya compartía se queda el escenario");
}

/// Colgar, o que la sala te suelte, apaga cámara, pantalla y selector; y
/// fuera de una sala los botones no piden nada.
///
/// Roturas que atrapa: la cámara encendida en el botón de la llamada
/// siguiente; el selector abierto sin sala; pedirle al motor que publique
/// sin estar dentro.
#[tokio::test]
async fn leaving_turns_everything_off() {
    let rig = rig().await;
    let voice = rig.core.voice();
    voice.join_channel("a", None, None).await;
    voice.toggle_camera();
    voice.engine_event(own("u1", VideoSource::Camera, true));
    *rig.engine.sources.lock().unwrap() = ScreenSources { screens: vec![source("s1"), source("s2")], windows: vec![] };
    voice.toggle_share();
    voice.leave().await;
    let now = state(&rig);
    assert_eq!((now.camera, now.sharing, now.picking.is_some(), now.cameras.len()), (Publishing::Off, Publishing::Off, false, 0));

    let before = rig.engine.calls().len();
    voice.toggle_camera();
    voice.toggle_share();
    assert_eq!(rig.engine.calls().len(), before, "fuera no se publica nada");
    assert_eq!(state(&rig).camera, Publishing::Off);

    voice.join_channel("a", None, None).await;
    voice.toggle_camera();
    voice.engine_event(own("u1", VideoSource::Camera, true));
    voice.engine_event(VoiceEvent::Disconnected { reason: "SignalClose".into() });
    assert_eq!(state(&rig).camera, Publishing::Off);
}

/// La cámara elegida se recuerda por su nombre y no pisa ni el micrófono ni
/// el altavoz; lo guardado antes de que existiera la cámara se sigue
/// leyendo.
///
/// Roturas que atrapa: olvidarla al reiniciar; que elegirla borre el
/// micrófono; perder las elecciones de siempre al añadir el campo nuevo.
#[tokio::test]
async fn the_chosen_camera_is_remembered() {
    let rig = rig().await;
    rig.storage.set("voice-devices", r#"{"input":"alsa:mic","output":null}"#).unwrap();
    assert_eq!(rig.core.voice().device_choices().input.as_deref(), Some("alsa:mic"), "lo guardado antes de la cámara");
    rig.core.voice().choose_camera(Some("Integrated Camera".into()));
    let again = Core::new(Config::new(rig.server.uri()), rig.storage.clone(), Arc::new(Quiet)).unwrap();
    let choices = again.voice().device_choices();
    assert_eq!((choices.camera.as_deref(), choices.input.as_deref()), (Some("Integrated Camera"), Some("alsa:mic")));
}
