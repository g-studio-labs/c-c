//! Paso 2: una llamada de verdad en la app nativa (command-and-control-419).
//!
//! `calls-native-spike call` entra a una reunión de cac **como invitado** —sin
//! cuenta, con el enlace de la invitación— y hace lo que hace el motor de la
//! app de Tauri (`app/src-tauri/src/voice.rs`), portado a `livekit` 0.12 y
//! pintando con GPUI:
//!
//! - el micrófono, procesado (eco, paso alto, ruido) y publicado;
//! - lo que dicen los demás, mezclado en una sola salida que es también la
//!   referencia del cancelador de eco;
//! - el vídeo de los demás (cámara o pantalla), un mosaico por pista, con el
//!   mismo camino que el paso 1 (`paint.rs`): convertir en la tarea de la
//!   pista, dejar el último en su casilla, y al pintar tirar el anterior con
//!   `window.drop_image`;
//! - opcionalmente, la cámara y la pantalla propias.
//!
//! Y una línea de medidas por segundo.
//!
//! Variables: `CALLS_INVITE` (el enlace `…/app/join#<token>` o el token solo;
//! obligatoria), `CALLS_API` (`https://cac.guz-studio.dev`), `CALLS_NAME`
//! («Prueba app nativa»), `CALLS_MUTE=1` (publicar silencio), `CALLS_CAMERA=1`,
//! `CALLS_SCREEN=1` y `CALLS_SCREEN_SOURCE=<índice>`, `CALLS_CSV` (fichero).
//!
//! Lo que se dejó fuera a propósito, porque es una prueba: elegir
//! dispositivos, el diario, la latencia y las estadísticas, grabar, el canal
//! de datos y el timbre.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures::channel::mpsc::{unbounded as wake_channel, UnboundedReceiver as WakeReceiver, UnboundedSender as WakeSender};
use futures::StreamExt;
use gpui_kit::*;
use livekit::options::TrackPublishOptions;
use livekit::track::{LocalAudioTrack, LocalTrack, LocalVideoTrack, RemoteTrack, TrackSource};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::native::apm::AudioProcessingModule;
use livekit::webrtc::prelude::{AudioFrame, I420Buffer, RtcAudioTrack, RtcVideoTrack, VideoFrame, VideoResolution, VideoRotation};
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::webrtc::video_source::RtcVideoSource;
use livekit::webrtc::video_stream::native::{NativeVideoStream, NativeVideoStreamOptions};
use livekit::participant::RemoteParticipant;
use livekit::{Room, RoomEvent, RoomOptions};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::paint;

// ─── Lo que se pide ───────────────────────────────────────────────────────────

const DEFAULT_API: &str = "https://cac.guz-studio.dev";
const DEFAULT_NAME: &str = "Prueba app nativa";

struct CallSetup {
    api: String,
    /// El token del enlace. **No se imprime nunca**: con él cualquiera pide
    /// entrar a la reunión.
    invite: String,
    name: String,
    mute: bool,
    camera: bool,
    screen: bool,
    screen_source: Option<usize>,
    csv: Option<String>,
}

fn flag(name: &str) -> bool {
    matches!(std::env::var(name).as_deref(), Ok("1") | Ok("true") | Ok("si") | Ok("sí"))
}

impl CallSetup {
    /// Todo se lee y se comprueba **antes** de tocar la red: sin invitación
    /// buena, el proceso se va sin haber hecho ni una petición.
    fn from_env() -> Result<Self, String> {
        let raw = std::env::var("CALLS_INVITE").unwrap_or_default();
        if raw.trim().is_empty() {
            return Err("falta CALLS_INVITE: el enlace de invitación de la reunión (…/app/join#<token>) o sólo el token".into());
        }
        let invite = invite_token(&raw)
            .ok_or("CALLS_INVITE no parece un enlace de invitación: se espera …/app/join#<id>.<firma> o el token <id>.<firma> solo")?;
        let text = |name: &str, default: &str| {
            std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
        };
        Ok(CallSetup {
            api: text("CALLS_API", DEFAULT_API).trim_end_matches('/').to_string(),
            invite,
            name: text("CALLS_NAME", DEFAULT_NAME),
            mute: flag("CALLS_MUTE"),
            camera: flag("CALLS_CAMERA"),
            screen: flag("CALLS_SCREEN"),
            screen_source: std::env::var("CALLS_SCREEN_SOURCE").ok().and_then(|v| v.trim().parse().ok()),
            csv: std::env::var("CALLS_CSV").ok().filter(|v| !v.is_empty()),
        })
    }
}

/// El token de un enlace de invitado, como lo saca la página del invitado
/// (`tokenFromHash` en `app/src/lib/call-link.ts`): lo que va detrás de `#`,
/// descodificado y sin espacios. Se acepta también el token pelado.
///
/// Y con la forma que comprueba el backend (`VerifyCallInviteLink`):
/// `<id>.<firma>`, con la firma un HMAC-SHA256 en hexadecimal. Comprobar la
/// forma aquí no prueba nada —la firma la valida el servidor—, pero evita
/// mandarle por error una URL entera o un texto cualquiera.
fn invite_token(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let fragment = match raw.rsplit_once('#') {
        Some((_, after)) => after,
        // Una URL sin `#` no lleva token: no adivinar.
        None if raw.contains("://") => return None,
        None => raw,
    };
    let token = percent_decode(fragment)?;
    let token = token.trim();
    let (id, signature) = token.split_once('.')?;
    let well_formed = !id.is_empty() && signature.len() == 64 && signature.bytes().all(|b| b.is_ascii_hexdigit());
    well_formed.then(|| token.to_string())
}

/// `decodeURIComponent` en pequeño: `%XX` a su byte. `None` si está mal.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Quita un secreto de un texto antes de enseñarlo. Los errores del SDK
/// pueden llevar la URL de señalización, y ésa lleva el token de la sala.
fn redact(text: String, secret: &str) -> String {
    if secret.is_empty() {
        return text;
    }
    text.replace(secret, "<token>")
}

// ─── La puerta de invitados ───────────────────────────────────────────────────
//
// Lo mismo que hace `app/src/pages/JoinCall.tsx`: pedir entrar con el enlace y
// un nombre, y mientras se espera, volver a preguntar con el pase que devuelve
// el servidor. **Mientras se espera no hay token de sala**: lo acuña el
// servidor sólo cuando alguien de dentro te deja pasar.

/// Cada cuánto se pregunta en la sala de espera. El servidor deja 1500
/// preguntas por hora a cada invitado (`publicStatusPerGuestPerHour`); cada
/// 3 s son 1200, como la página web.
const STATUS_EVERY: Duration = Duration::from_secs(3);

#[derive(Serialize)]
struct JoinBody<'a> {
    token: &'a str,
    name: &'a str,
    pass: &'a str,
}

#[derive(Serialize)]
struct StatusBody<'a> {
    token: &'a str,
    pass: &'a str,
}

/// El sobre de la API (`domain.APIResponse`).
#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    error: String,
    data: Option<Entry>,
}

/// `domain.GuestJoinResponse`. `url` y `token` sólo vienen al estar admitido.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Entry {
    status: String,
    url: String,
    token: String,
    identity: String,
    name: String,
    pass: String,
    title: String,
}

/// Lo que hace falta para entrar a la sala.
struct Admission {
    url: String,
    token: String,
    identity: String,
    title: String,
}

enum Refusal {
    /// 429: se ha preguntado demasiado. Se espera y se sigue.
    RateLimited,
    Fatal(String),
}

/// Lo que el servidor dice al negarse, en castellano (`callError` en
/// `backend/internal/adapters/handler/call.go`).
fn refusal_text(code: reqwest::StatusCode, error: &str, message: &str) -> String {
    let said = match error {
        "invite-invalid" => "este enlace no abre ninguna reunión",
        "invite-expired" => "la invitación caducó",
        "invite-revoked" => "la invitación se revocó",
        "invite-full" => "la invitación está llena",
        "guest-removed" => "te sacaron de esta reunión",
        "guest-rejected" => "no te dejaron entrar",
        "guest-name-required" => "hace falta un nombre (CALLS_NAME)",
        "voice-unconfigured" => "el servidor no tiene las llamadas configuradas",
        _ => return format!("el servidor dijo {code}: {message} ({error})"),
    };
    said.to_string()
}

async fn post<B: Serialize>(http: &reqwest::Client, url: &str, body: &B) -> Result<Entry, Refusal> {
    let response = http.post(url).json(body).send().await.map_err(|e| Refusal::Fatal(format!("no se pudo hablar con {url}: {e}")))?;
    let code = response.status();
    if code == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(Refusal::RateLimited);
    }
    let envelope: Envelope = response.json().await.map_err(|e| Refusal::Fatal(format!("respuesta ilegible de {url} ({code}): {e}")))?;
    if !code.is_success() || !envelope.success {
        return Err(Refusal::Fatal(refusal_text(code, &envelope.error, &envelope.message)));
    }
    envelope.data.ok_or_else(|| Refusal::Fatal(format!("{url} contestó sin datos")))
}

fn describe_status(status: &str) -> &str {
    match status {
        "waiting" => "esperando a que alguien de dentro te deje pasar",
        "admitted" => "admitido",
        "rejected" => "rechazado",
        other => other,
    }
}

/// Pedir entrar y esperar hasta estar dentro, o hasta que digan que no.
async fn admit(setup: &CallSetup) -> Result<Admission, String> {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| format!("cliente HTTP: {e}"))?;
    let join_url = format!("{}/api/v1/public/calls/join", setup.api);
    let status_url = format!("{}/api/v1/public/calls/status", setup.api);
    println!("pidiendo entrar en {} como «{}»…", setup.api, setup.name);
    let mut entry = match post(&http, &join_url, &JoinBody { token: &setup.invite, name: &setup.name, pass: "" }).await {
        Ok(entry) => entry,
        Err(Refusal::RateLimited) => return Err("el servidor dice que se ha pedido entrar demasiadas veces (429); espera un rato".into()),
        Err(Refusal::Fatal(e)) => return Err(e),
    };
    println!("reunión «{}», en la puerta como «{}»", entry.title, entry.name);
    let mut last_status = String::new();
    loop {
        if entry.status != last_status {
            println!("sala de espera: {}", describe_status(&entry.status));
            last_status = entry.status.clone();
        }
        match entry.status.as_str() {
            "admitted" => {
                if entry.url.is_empty() || entry.token.is_empty() {
                    return Err("admitido, pero el servidor no mandó la sala (sin url o sin token)".into());
                }
                return Ok(Admission { url: entry.url, token: entry.token, identity: entry.identity, title: entry.title });
            }
            "rejected" => return Err("no te dejaron entrar".into()),
            "waiting" => {}
            other => return Err(format!("estado desconocido en la sala de espera: «{other}»")),
        }
        tokio::time::sleep(STATUS_EVERY).await;
        // El pase se renueva en cada respuesta: se pregunta con el último.
        match post(&http, &status_url, &StatusBody { token: &setup.invite, pass: &entry.pass }).await {
            Ok(next) => entry = next,
            Err(Refusal::RateLimited) => {
                println!("sala de espera: el servidor pide calma (429); se espera 30 s");
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Err(Refusal::Fatal(e)) => return Err(e),
        }
    }
}

// ─── La llamada ───────────────────────────────────────────────────────────────

pub fn run() {
    let setup = match CallSetup::from_env() {
        Ok(setup) => setup,
        Err(e) => {
            eprintln!("calls-native-spike call: {e}");
            std::process::exit(2);
        }
    };
    // Un runtime de tokio propio, como el de `desktop/src/backend.rs`: el SDK,
    // la puerta HTTP y las tareas de cada pista viven aquí; GPUI, en el hilo
    // principal con su ejecutor.
    let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread().worker_threads(4).thread_name("llamada").enable_all().build().expect("runtime"));
    let handle = runtime.handle().clone();

    let admission = match runtime.block_on(admit(&setup)) {
        Ok(admission) => admission,
        Err(e) => {
            eprintln!("calls-native-spike call: {e}");
            std::process::exit(1);
        }
    };
    println!("admitido como {}; entrando a la sala…", admission.identity);
    let (room, events) = match runtime.block_on(Room::connect(&admission.url, &admission.token, RoomOptions::default())) {
        Ok(joined) => joined,
        Err(e) => {
            eprintln!("calls-native-spike call: no se pudo entrar a la sala: {}", redact(e.to_string(), &admission.token));
            std::process::exit(1);
        }
    };
    let room = Arc::new(room);
    println!("dentro de «{}» como {}", admission.title, room.local_participant().identity());

    let wall = Arc::new(Wall::default());
    let (wake, woken) = wake_channel::<()>();

    // El micrófono, siempre. Si no se puede publicar o abrir, se sigue: oír y
    // ver también es probar.
    match runtime.block_on(publish_microphone(&room)) {
        Ok(source) => match start_capture(source, handle.clone(), setup.mute) {
            Ok(described) => println!("micrófono: {described}{}", if setup.mute { " (CALLS_MUTE: se publica silencio)" } else { "" }),
            Err(e) => eprintln!("micrófono: {e}"),
        },
        Err(e) => eprintln!("micrófono: {}", redact(e, &admission.token)),
    }

    handle.spawn(listen(events, Arc::downgrade(&room), handle.clone(), wall.clone(), wake.clone()));
    {
        let room = room.clone();
        handle.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                leave(&room, "Ctrl+C").await;
            }
        });
    }

    if setup.camera {
        let preview = Arc::new(Tile::new(LOCAL_CAMERA.into(), "tú · cámara".into()));
        wall.add(preview.clone());
        let publish = spawn_publisher(&handle, room.clone(), TrackSource::Camera, "cámara");
        start_camera(handle.clone(), publish, preview, wake.clone());
    }
    if setup.screen {
        let publish = spawn_publisher(&handle, room.clone(), TrackSource::Screenshare, "pantalla");
        start_screen(handle.clone(), publish, setup.screen_source);
    }

    handle.spawn(report(wall.clone(), setup.csv.clone()));

    let title = admission.title.clone();
    let (room_for_window, runtime_for_window) = (room.clone(), runtime.clone());
    application().run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1280.), px(720.)), cx))),
            ..WindowOptions::default()
        };
        open_window(options, cx, |_, cx| cx.new(|cx| CallView::new(title, wall, woken, cx))).expect("ventana");
        // Cerrar la ventana es colgar: se sale de la sala antes de que el
        // proceso termine, para no dejar un participante fantasma en el SFU.
        cx.on_window_closed(move |_, _| {
            runtime_for_window.block_on(leave(&room_for_window, "ventana cerrada"));
        })
        .detach();
    });
    // Si GPUI terminara por otro camino.
    runtime.block_on(leave(&room, "la app terminó"));
}

static LEAVING: AtomicBool = AtomicBool::new(false);

/// Salir de la sala limpio (`room.close()`) y terminar el proceso.
///
/// Pueden pedirlo a la vez Ctrl+C, la ventana y el SFU; sólo el primero
/// cierra, y los demás esperan a que ése termine el proceso.
async fn leave(room: &Room, why: &str) {
    if LEAVING.swap(true, Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_secs(10)).await;
        return;
    }
    println!("saliendo ({why})…");
    match tokio::time::timeout(Duration::from_secs(5), room.close()).await {
        Ok(Ok(())) => println!("fuera de la sala"),
        Ok(Err(e)) => println!("la sala no cerró limpia: {e}"),
        Err(_) => println!("la sala no contestó en 5 s; se sale igual"),
    }
    // A propósito sin esperar a los hilos: la cámara puede estar bloqueada
    // dentro del driver (ver `start_camera`) y no volvería nunca.
    std::process::exit(0);
}

fn who(participant: &RemoteParticipant) -> String {
    let name = participant.name();
    if name.is_empty() { participant.identity().to_string() } else { name }
}

fn source_text(source: TrackSource) -> &'static str {
    match source {
        TrackSource::Screenshare => "pantalla",
        _ => "cámara",
    }
}

/// Lo que se está recibiendo de una pista remota, para soltarlo al irse.
struct Receiving {
    task: tokio::task::JoinHandle<()>,
    label: String,
    queue: Option<TrackQueue>,
    video: bool,
}

/// Los eventos de la sala: quién entra y sale, y sobre todo qué pistas
/// llegan. Portado de `listen_to_events` (voice.rs), sólo lo que hace sonar y
/// pintar.
async fn listen(mut events: mpsc::UnboundedReceiver<RoomEvent>, room: Weak<Room>, handle: Handle, wall: Arc<Wall>, wake: WakeSender<()>) {
    let mut receiving: HashMap<String, Receiving> = HashMap::new();
    while let Some(event) = events.recv().await {
        match event {
            // Los que ya estaban: el SDK no manda `ParticipantConnected` por
            // ellos. Sus pistas llegan después como `TrackSubscribed`.
            RoomEvent::Connected { participants_with_tracks } => {
                for (participant, _) in &participants_with_tracks {
                    println!("ya estaba: {}", who(participant));
                }
            }
            RoomEvent::ParticipantConnected(participant) => println!("entra: {}", who(&participant)),
            RoomEvent::ParticipantDisconnected(participant) => println!("sale: {}", who(&participant)),
            RoomEvent::TrackSubscribed { track, publication, participant } => {
                let sid = publication.sid().to_string();
                match track {
                    RemoteTrack::Audio(audio) => {
                        let label = format!("{} · micrófono", who(&participant));
                        println!("suena: {label}");
                        let (queue, task) = play_track(&handle, audio.rtc_track());
                        receiving.insert(sid, Receiving { task, label, queue: Some(queue), video: false });
                    }
                    RemoteTrack::Video(video) => {
                        let label = format!("{} · {}", who(&participant), source_text(publication.source()));
                        println!("llega vídeo: {label}");
                        let tile = Arc::new(Tile::new(sid.clone(), label.clone()));
                        wall.add(tile.clone());
                        let task = receive_video(&handle, video.rtc_track(), tile, wake.clone());
                        receiving.insert(sid, Receiving { task, label, queue: None, video: true });
                        let _ = wake.unbounded_send(());
                    }
                }
            }
            RoomEvent::TrackUnsubscribed { publication, .. } => {
                let sid = publication.sid().to_string();
                if let Some(gone) = receiving.remove(&sid) {
                    println!("se va: {}", gone.label);
                    gone.task.abort();
                    if let Some(queue) = gone.queue {
                        TRACKS.lock().unwrap().retain(|q| !Arc::ptr_eq(q, &queue));
                    }
                    if gone.video {
                        wall.remove(&sid);
                        let _ = wake.unbounded_send(());
                    }
                }
            }
            RoomEvent::Disconnected { reason } => {
                let why = format!("la sala te desconectó: {reason:?}");
                match room.upgrade() {
                    Some(room) => leave(&room, &why).await,
                    None => {
                        println!("{why}");
                        std::process::exit(0);
                    }
                }
            }
            _ => {}
        }
    }
}

// ─── El micrófono ─────────────────────────────────────────────────────────────

/// 48 kHz mono: lo que WebRTC usa internamente (voice.rs).
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u32 = 1;
/// 10 ms por trama, que es el tamaño que espera libwebrtc.
const SAMPLES_PER_FRAME: usize = SAMPLE_RATE as usize / 100;

/// Tramas que el SDK aceptó.
static FRAMES_PUBLISHED: AtomicU64 = AtomicU64::new(0);
/// El pico más alto desde la última línea de medidas, en tanto por mil de la
/// escala completa, medido **después** del procesado: lo que de verdad sale.
static PEAK_PERMILLE: AtomicU32 = AtomicU32::new(0);

async fn publish_microphone(room: &Room) -> Result<NativeAudioSource, String> {
    // Como en voice.rs: estas opciones **no se aplican** a una fuente nativa
    // (las tramas van directas a los sumideros), y el procesado de verdad lo
    // hace `Processor`. Sin probar en 0.12 que siga siendo así; el C++ de
    // webrtc-sys 0.3.55 las guarda igual que antes.
    let source = NativeAudioSource::new(AudioSourceOptions { echo_cancellation: true, noise_suppression: true, auto_gain_control: true }, SAMPLE_RATE, CHANNELS, 1_000);
    let track = LocalAudioTrack::create_audio_track("micro", RtcAudioSource::Native(source.clone()));
    room.local_participant()
        .publish_track(LocalTrack::Audio(track), TrackPublishOptions { source: TrackSource::Microphone, ..Default::default() })
        .await
        .map_err(|e| format!("no se pudo publicar: {e}"))?;
    Ok(source)
}

/// El micrófono del sistema por defecto, en su propio hilo. Portado de
/// `start_capture_thread` (voice.rs): cpal entrega bloques del tamaño que
/// quiere y al ritmo del dispositivo; aquí se pasan a mono, a 48 kHz y a
/// tramas de 10 ms, se procesan y se publican.
///
/// Devuelve qué dispositivo se abrió y con qué formato.
///
/// Sin probar: desenchufar el micrófono a media llamada (el flujo de cpal
/// falla y la voz se para; voice.rs tampoco lo recupera solo).
fn start_capture(source: NativeAudioSource, handle: Handle, mute: bool) -> Result<String, String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<String, String>>();
    std::thread::Builder::new()
        .name("micro".into())
        .spawn(move || {
            let host = cpal::default_host();
            let Some(device) = host.default_input_device() else {
                let _ = ready_tx.send(Err("no hay dispositivo de entrada".into()));
                return;
            };
            let config = match device.default_input_config() {
                Ok(config) => config,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("el micrófono no dice su formato: {e}")));
                    return;
                }
            };
            let input_rate = config.sample_rate();
            let input_channels = config.channels() as usize;
            let described = format!("«{device}» a {input_rate} Hz, {} canal(es), {:?}", config.channels(), config.sample_format());

            let (frames_tx, mut frames_rx) = mpsc::unbounded_channel::<Vec<i16>>();
            // Muestras ya remuestreadas a `SAMPLE_RATE`: una trama de
            // `SAMPLES_PER_FRAME` son siempre 10 ms de los que espera la fuente.
            let mut accumulator: Vec<i16> = Vec::with_capacity(SAMPLES_PER_FRAME * 2);
            let mut phase = Phase::default();
            let stream = device.build_input_stream(
                config.config(),
                move |samples: &[f32], _: &cpal::InputCallbackInfo| {
                    // A mono promediando, y al ritmo de la fuente: sin
                    // remuestrear, un micrófono a 44,1 kHz hacía que el SDK
                    // rechazara las tramas y no se te oyera (voice.rs).
                    let mono: Vec<f32> = samples.chunks(input_channels).map(|t| t.iter().sum::<f32>() / t.len() as f32).collect();
                    resample(&mono, input_rate as f32 / SAMPLE_RATE as f32, &mut phase, mute, &mut accumulator);
                    while accumulator.len() >= SAMPLES_PER_FRAME {
                        let rest = accumulator.split_off(SAMPLES_PER_FRAME);
                        let frame = std::mem::replace(&mut accumulator, rest);
                        let _ = frames_tx.send(frame);
                    }
                },
                |e| eprintln!("micrófono: el flujo de captura falló: {e}"),
                None,
            );
            let stream = match stream {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("no se pudo abrir «{device}»: {e}")));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(format!("no se pudo arrancar «{device}»: {e}")));
                return;
            }

            // El procesado y la publicación, en el runtime: `capture_frame`
            // es asíncrono.
            handle.spawn(async move {
                let mut processor = Processor::new();
                let mut warned = false;
                while let Some(mut frame) = frames_rx.recv().await {
                    if let Err(e) = processor.process_frame(&mut frame, take_reference()) {
                        if !warned {
                            eprintln!("micrófono: trama sin procesar: {e}");
                            warned = true;
                        }
                    }
                    PEAK_PERMILLE.fetch_max(peak_permille(&frame), Ordering::Relaxed);
                    let rtc_frame = AudioFrame { data: frame.into(), sample_rate: SAMPLE_RATE, num_channels: CHANNELS, samples_per_channel: SAMPLES_PER_FRAME as u32 };
                    // Un rechazo del SDK no puede pasar en silencio: era el
                    // fallo «micrófono abierto, nadie te oye» de voice.rs.
                    if let Err(e) = source.capture_frame(&rtc_frame).await {
                        eprintln!("micrófono: el SDK rechazó una trama y se deja de publicar: {e}");
                        break;
                    }
                    FRAMES_PUBLISHED.fetch_add(1, Ordering::Relaxed);
                }
            });

            let _ = ready_tx.send(Ok(described));
            // Este hilo sólo existe para mantener vivo el `stream` de cpal,
            // que no es `Send`. Vive lo que el proceso.
            loop {
                std::thread::park();
            }
        })
        .map_err(|e| format!("no se pudo lanzar el hilo del micrófono: {e}"))?;
    // Con plazo, como en voice.rs: abrir un dispositivo de audio puede
    // colgarse.
    match ready_rx.recv_timeout(Duration::from_secs(15)) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err("quince segundos sin abrir el dispositivo".into()),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err("el hilo del micrófono murió sin decir nada".into()),
    }
}

/// Por dónde va el remuestreo entre un bloque de cpal y el siguiente
/// (voice.rs, `Phase`): la fracción pendiente y el último cuadro, para no
/// meter un clic en cada frontera.
#[derive(Default)]
struct Phase {
    pos: f32,
    last_value: f32,
}

/// Lleva un bloque de cuadros mono al ritmo de la fuente, por interpolación
/// lineal. Copiado de voice.rs, donde tiene sus pruebas.
fn resample(mono_frames: &[f32], step: f32, phase: &mut Phase, silent: bool, output: &mut Vec<i16>) {
    let n = mono_frames.len();
    if n == 0 {
        return;
    }
    let mut pos = phase.pos;
    while pos < (n - 1) as f32 {
        let i = pos.floor();
        let frac = pos - i;
        let a = if i < 0.0 { phase.last_value } else { mono_frames[i as usize] };
        let b = mono_frames[(i + 1.0) as usize];
        let mean = a + (b - a) * frac;
        output.push(if silent { 0 } else { (mean.clamp(-1.0, 1.0) * i16::MAX as f32) as i16 });
        pos += step;
    }
    phase.pos = pos - n as f32;
    phase.last_value = mono_frames[n - 1];
}

fn peak_permille(samples: &[i16]) -> u32 {
    let peak = samples.iter().map(|m| m.unsigned_abs() as u32).max().unwrap_or(0);
    peak * 1000 / i16::MAX as u32
}

/// El procesado de la voz (voice.rs, `Processor`): el módulo de WebRTC con
/// cancelación de eco, paso alto y supresión de ruido, y **sin control de
/// ganancia**, que con el cancelador encendido volvía a subir el ruido que el
/// supresor acababa de bajar (medido allí). Siempre encendido: aquí no hay el
/// interruptor de «música».
struct Processor {
    apm: AudioProcessingModule,
}

impl Processor {
    fn new() -> Self {
        // (eco, ganancia, paso alto, ruido).
        Self { apm: AudioProcessingModule::new(true, false, true, true) }
    }

    /// Procesa una trama en su sitio, después de darle al cancelador lo que
    /// sonó desde la anterior. `process_stream` hace panic si la longitud no
    /// es múltiplo de 10 ms: se comprueba antes, y una trama que no cuadra se
    /// deja sin procesar.
    fn process_frame(&mut self, frame: &mut [i16], reference: Vec<Vec<i16>>) -> Result<(), String> {
        if frame.is_empty() || frame.len() % SAMPLES_PER_FRAME != 0 {
            return Err(format!("trama de {} muestras, no múltiplo de 10 ms", frame.len()));
        }
        for mut played in reference {
            self.apm.process_reverse_stream(&mut played, SAMPLE_RATE as i32, CHANNELS as i32).map_err(|e| e.to_string())?;
        }
        self.apm.process_stream(frame, SAMPLE_RATE as i32, CHANNELS as i32).map_err(|e| e.to_string())
    }
}

// ─── Lo que suena: una sola salida, que es la referencia del eco ──────────────

/// Lo que ha sonado y el cancelador todavía no ha visto, a 48 kHz mono. Lo
/// llena la salida y lo vacía la captura.
static RENDER_REFERENCE: Mutex<VecDeque<i16>> = Mutex::new(VecDeque::new());
/// 200 ms como mucho: lo más viejo ya no sirve para cancelar nada.
const RENDER_REFERENCE_MAX: usize = SAMPLES_PER_FRAME * 20;

fn take_reference() -> Vec<Vec<i16>> {
    let mut queue = RENDER_REFERENCE.lock().unwrap();
    let mut out = Vec::with_capacity(queue.len() / SAMPLES_PER_FRAME);
    while queue.len() >= SAMPLES_PER_FRAME {
        out.push(queue.drain(..SAMPLES_PER_FRAME).collect());
    }
    out
}

fn note_reference(queue: &mut VecDeque<i16>, played: &[i16]) {
    queue.extend(played.iter().copied());
    if queue.len() > RENDER_REFERENCE_MAX {
        let excess = queue.len() - RENDER_REFERENCE_MAX;
        queue.drain(..excess);
    }
}

/// Suma una muestra de cada persona, en `i32` y recortada: dar la vuelta
/// suena a chasquido.
fn mix_samples(samples: impl Iterator<Item = i16>) -> i16 {
    samples.map(i32::from).sum::<i32>().clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// 60 ms de colchón antes de que una pista empiece a sonar: sin él, el vaivén
/// de llegada de las tramas vaciaba la cola y sonaban clics (voice.rs).
const JITTER_CUSHION: usize = SAMPLES_PER_FRAME * 6;
/// 200 ms como mucho; pasado eso se tira lo más viejo hasta el colchón.
const QUEUE_MAX: usize = SAMPLES_PER_FRAME * 20;

/// Veces que una pista se quedó sin muestras sonando, de todas: la medida de
/// los clics. Global y no por pista como en voice.rs, para que la cuenta no se
/// pierda cuando una pista se va.
static UNDERRUNS: AtomicU64 = AtomicU64::new(0);

/// Una pista remota en la mezcla (voice.rs, `MixTrack`).
#[derive(Default)]
struct MixTrack {
    queue: VecDeque<i16>,
    playing: bool,
}

impl MixTrack {
    fn push_samples(&mut self, samples: &[i16]) {
        self.queue.extend(samples.iter().copied());
        if self.queue.len() > QUEUE_MAX {
            let excess = self.queue.len() - JITTER_CUSHION;
            self.queue.drain(..excess);
        }
    }

    fn next_sample(&mut self) -> i16 {
        if !self.playing {
            if self.queue.len() < JITTER_CUSHION {
                return 0;
            }
            self.playing = true;
        }
        match self.queue.pop_front() {
            Some(sample) => sample,
            None => {
                self.playing = false;
                UNDERRUNS.fetch_add(1, Ordering::Relaxed);
                0
            }
        }
    }
}

/// Una trama del SDK, a mono. Se pide mono, pero si llegara con más canales
/// se promedian.
fn to_mono(samples: &[i16], channel_count: usize) -> Vec<i16> {
    if channel_count <= 1 {
        return samples.to_vec();
    }
    samples.chunks(channel_count).map(|c| (c.iter().map(|&v| v as i32).sum::<i32>() / c.len() as i32) as i16).collect()
}

type TrackQueue = Arc<Mutex<MixTrack>>;

static TRACKS: Mutex<Vec<TrackQueue>> = Mutex::new(Vec::new());
static OUTPUT_OPEN: AtomicBool = AtomicBool::new(false);

/// Abre la salida por defecto si no está abierta (voice.rs, `ensure_output`):
/// una sola para todas las pistas, que es lo que permite saber qué está
/// sonando y dárselo al cancelador.
///
/// Sin probar: cambiar de altavoz o desenchufarlo a media llamada.
fn ensure_output() {
    if OUTPUT_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new().name("altavoz".into()).spawn(|| {
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            eprintln!("altavoz: no hay dispositivo de salida");
            return;
        };
        let Ok(config) = device.default_output_config() else {
            eprintln!("altavoz: el dispositivo no dice su formato");
            return;
        };
        let output_channels = config.channels() as usize;
        let rate_hz = config.sample_rate();
        println!("altavoz: «{device}» a {rate_hz} Hz, {output_channels} canal(es)");
        // De 48 kHz (la mezcla) al ritmo del dispositivo, por interpolación
        // lineal: `pos` es la fracción entre la muestra anterior y la
        // siguiente de la mezcla.
        let step = SAMPLE_RATE as f32 / rate_hz as f32;
        let (mut pos, mut before, mut after) = (1.0f32, 0i16, 0i16);
        let mut played: Vec<i16> = Vec::with_capacity(1024);
        let stream = device.build_output_stream(
            config.config(),
            move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let tracks = TRACKS.lock().unwrap();
                let mut queues: Vec<_> = tracks.iter().map(|q| q.lock().unwrap()).collect();
                played.clear();
                for chunk in output.chunks_mut(output_channels) {
                    while pos >= 1.0 {
                        let sample = mix_samples(queues.iter_mut().map(|q| q.next_sample()));
                        played.push(sample);
                        before = after;
                        after = sample;
                        pos -= 1.0;
                    }
                    let value = (before as f32 + (after as f32 - before as f32) * pos) / i16::MAX as f32;
                    pos += step;
                    for channel in chunk.iter_mut() {
                        *channel = value;
                    }
                }
                drop(queues);
                drop(tracks);
                note_reference(&mut RENDER_REFERENCE.lock().unwrap(), &played);
            },
            |e| eprintln!("altavoz: error de reproducción: {e}"),
            None,
        );
        let Ok(stream) = stream else {
            eprintln!("altavoz: no se pudo abrir «{device}»");
            return;
        };
        if let Err(e) = stream.play() {
            eprintln!("altavoz: no se pudo arrancar: {e}");
            return;
        }
        // Mantiene vivo el `stream`, que no es `Send`.
        loop {
            std::thread::park();
        }
    });
    if let Err(e) = spawned {
        eprintln!("altavoz: no se pudo lanzar su hilo: {e}");
    }
}

/// Lo que dice otro, a la mezcla (voice.rs, `play_track`). En una tarea del
/// runtime y no en un hilo con su `block_on`: el resultado es el mismo y se
/// puede abortar al irse la pista.
fn play_track(handle: &Handle, track: RtcAudioTrack) -> (TrackQueue, tokio::task::JoinHandle<()>) {
    ensure_output();
    let queue: TrackQueue = Arc::new(Mutex::new(MixTrack::default()));
    TRACKS.lock().unwrap().push(queue.clone());
    let mine = queue.clone();
    let task = handle.spawn(async move {
        // A 48 kHz y mono, que es como mezcla la salida: todas las pistas
        // suman muestra con muestra y la referencia del eco sale al ritmo de
        // la captura sin otro remuestreo.
        let mut incoming = NativeAudioStream::new(track, SAMPLE_RATE as i32, 1);
        while let Some(frame) = incoming.next().await {
            let mono = to_mono(&frame.data, frame.num_channels.max(1) as usize);
            mine.lock().unwrap().push_samples(&mono);
        }
        TRACKS.lock().unwrap().retain(|q| !Arc::ptr_eq(q, &mine));
    });
    (queue, task)
}

// ─── Lo que se ve ─────────────────────────────────────────────────────────────

const LOCAL_CAMERA: &str = "local:cámara";

/// Un mosaico: una pista de vídeo (remota, o la vista previa de la cámara
/// propia) con su casilla del paso 1.
struct Tile {
    /// El sid de la pista, o `LOCAL_CAMERA`.
    id: String,
    label: String,
    slot: paint::Slot,
    painted: AtomicU64,
}

impl Tile {
    fn new(id: String, label: String) -> Self {
        Tile { id, label, slot: paint::Slot::default(), painted: AtomicU64::new(0) }
    }
}

/// Los mosaicos que hay. Lo escriben los eventos de la sala; lo leen la vista
/// y las medidas.
#[derive(Default)]
struct Wall {
    tiles: Mutex<Vec<Arc<Tile>>>,
}

impl Wall {
    fn add(&self, tile: Arc<Tile>) {
        self.tiles.lock().unwrap().push(tile);
    }

    fn remove(&self, id: &str) {
        self.tiles.lock().unwrap().retain(|tile| tile.id != id);
    }

    fn snapshot(&self) -> Vec<Arc<Tile>> {
        self.tiles.lock().unwrap().clone()
    }
}

/// Lo que llega de una pista de vídeo: cada fotograma se convierte **en la
/// tarea de la pista** a lo que pinta GPUI y se deja el último en su casilla.
///
/// Cola sin tope (`queue_size_frames: Some(0)`): se convierte todo lo que
/// entrega el SDK, así «llegan/s» cuenta de verdad lo que llegó. La de por
/// defecto (`NativeVideoStream::new`, la que usa voice.rs) guarda sólo el
/// último fotograma y tira los demás **sin contarlos** —el SDK no expone esa
/// cuenta—, y entonces no se sabría si se pierden en la red o aquí. Si convertir no diera abasto la
/// cola crecería; con 0,7 ms por fotograma de 720p (paso 1) sobra margen, y
/// las medidas lo dirían. Lo que llega y no se pinta sale en «perdidos».
fn receive_video(handle: &Handle, track: RtcVideoTrack, tile: Arc<Tile>, wake: WakeSender<()>) -> tokio::task::JoinHandle<()> {
    handle.spawn(async move {
        let mut incoming = NativeVideoStream::with_options(track, NativeVideoStreamOptions { queue_size_frames: Some(0), ..Default::default() });
        while let Some(frame) = incoming.next().await {
            let started = Instant::now();
            let image = paint::to_render_image(&*frame.buffer);
            tile.slot.conv_us.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
            tile.slot.offer(image);
            if wake.unbounded_send(()).is_err() {
                return;
            }
        }
    })
}

/// La ventana: un mosaico por pista, con quién y qué.
struct CallView {
    title: String,
    wall: Arc<Wall>,
    /// Lo que se pintó la última vez de cada mosaico, que es lo que hay que
    /// tirar del atlas cuando llegue el siguiente o el mosaico se vaya.
    shown: HashMap<String, Arc<RenderImage>>,
}

impl CallView {
    fn new(title: String, wall: Arc<Wall>, mut woken: WakeReceiver<()>, cx: &mut Context<Self>) -> Self {
        // Lo que llega despierta a la vista; varios avisos seguidos son un
        // solo repintado (como `paint::Wall`).
        cx.spawn(async move |this, cx| {
            while woken.next().await.is_some() {
                while woken.try_recv().is_ok() {}
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        })
        .detach();
        CallView { title, wall, shown: HashMap::new() }
    }
}

impl Render for CallView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let tiles = self.wall.snapshot();
        for tile in &tiles {
            let Some(fresh) = tile.slot.take() else { continue };
            if let Some(old) = self.shown.insert(tile.id.clone(), fresh) {
                // Obligatorio: el atlas no olvida nada solo (paso 1).
                let _ = window.drop_image(old);
            }
            tile.painted.fetch_add(1, Ordering::Relaxed);
        }
        // Los mosaicos que se fueron: su última imagen también sale del atlas.
        let gone: Vec<String> = self.shown.keys().filter(|id| !tiles.iter().any(|tile| &tile.id == *id)).cloned().collect();
        for id in gone {
            if let Some(old) = self.shown.remove(&id) {
                let _ = window.drop_image(old);
            }
        }

        let columns = (tiles.len().max(1) as f32).sqrt().ceil() as usize;
        let rows = tiles.len().max(1).div_ceil(columns);
        let header = if tiles.is_empty() { format!("{} — sin vídeo todavía", self.title) } else { format!("{} — {} vídeo(s)", self.title, tiles.len()) };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x000000))
            .text_color(rgb(0xeeeeee))
            .child(div().px_2().py_1().text_sm().child(header))
            .child(div().flex_1().min_h_0().flex().flex_wrap().children(tiles.iter().map(|tile| {
                let mut cell = div().relative().w(relative(1.0 / columns as f32)).h(relative(1.0 / rows as f32)).p_0p5();
                if let Some(image) = self.shown.get(&tile.id) {
                    cell = cell.child(img(ImageSource::Render(image.clone())).size_full().object_fit(ObjectFit::Contain));
                }
                cell.child(div().absolute().left_1().bottom_1().px_1().bg(rgba(0x000000aa)).text_xs().child(tile.label.clone()))
            })))
    }
}

// ─── Cámara y pantalla propias ────────────────────────────────────────────────

/// Lo que la captura le dice a quien publica.
enum VideoChange {
    /// Una fuente nueva, con sus medidas: la primera, o la de después de un
    /// cambio de resolución.
    Source(NativeVideoSource, u32, u32),
    /// La captura se rindió.
    Stopped(String),
}

/// Quien publica la pista de una captura, y la **republica** cuando la
/// captura cambia de fuente.
///
/// Es el arreglo de un fallo del motor de Tauri: allí, si la cámara cambiaba
/// de resolución a media llamada, el hilo creaba una `NativeVideoSource`
/// nueva y le mandaba las tramas, pero la pista publicada seguía atada a la
/// vieja, que ya nadie alimentaba: los demás se quedaban con la imagen
/// congelada. Una fuente nueva necesita una pista nueva: se retira la
/// publicada y se publica otra con la fuente nueva.
fn spawn_publisher(handle: &Handle, room: Arc<Room>, kind: TrackSource, name: &'static str) -> mpsc::UnboundedSender<VideoChange> {
    let (tx, mut rx) = mpsc::unbounded_channel::<VideoChange>();
    handle.spawn(async move {
        let mut published = None;
        while let Some(change) = rx.recv().await {
            if let Some(sid) = published.take() {
                if let Err(e) = room.local_participant().unpublish_track(&sid).await {
                    eprintln!("{name}: no se pudo retirar la pista anterior: {e}");
                }
            }
            match change {
                VideoChange::Source(source, width, height) => {
                    let track = LocalVideoTrack::create_video_track(name, RtcVideoSource::Native(source));
                    let options = TrackPublishOptions { source: kind, ..Default::default() };
                    match room.local_participant().publish_track(LocalTrack::Video(track), options).await {
                        Ok(publication) => {
                            println!("{name}: publicada a {width}x{height}");
                            published = Some(publication.sid());
                        }
                        Err(e) => eprintln!("{name}: no se pudo publicar: {e}"),
                    }
                }
                VideoChange::Stopped(why) => println!("{name}: se apaga — {why}"),
            }
        }
    });
    tx
}

/// 720p: lo que se le pide a la cámara primero (voice.rs).
const VIDEO_WIDTH: u32 = 1280;
const VIDEO_HEIGHT: u32 = 720;

fn now_us() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as i64).unwrap_or(0)
}

/// Las cámaras que pueden capturar (voice.rs, `usable_cameras`): en Linux el
/// nodo de metadatos de la webcam se llama igual y sale antes.
fn usable_cameras() -> Vec<nokhwa::utils::CameraInfo> {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{ApiBackend, RequestedFormat, RequestedFormatType};

    nokhwa::query(ApiBackend::Auto)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| {
            nokhwa::Camera::new(c.index().clone(), RequestedFormat::new::<RgbFormat>(RequestedFormatType::None))
                .and_then(|mut cam| cam.compatible_camera_formats())
                .map(|f| !f.is_empty())
                .unwrap_or(false)
        })
        .collect()
}

/// La cámara por defecto, en su propio hilo. Portado de `start_camera`
/// (voice.rs), sin el selector, el vigía ni el plazo de arranque: aquí nadie
/// espera la respuesta, y lo que pase se imprime.
fn start_camera(handle: Handle, publish: mpsc::UnboundedSender<VideoChange>, preview: Arc<Tile>, wake: WakeSender<()>) {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType, Resolution};
    use nokhwa::Camera;

    let spawned = std::thread::Builder::new().name("cámara".into()).spawn(move || {
        // `NativeVideoSource::new` lanza tareas de tokio: desde un hilo
        // suelto sin el runtime entrado, la captura moría en la primera trama
        // (voice.rs, `enter_runtime`). El contexto es por hilo.
        let _in_runtime = handle.enter();

        // En orden de preferencia y cayendo al siguiente: `HighestResolution`
        // exige la resolución exacta, y pedir sólo 720p dejaba sin cámara a
        // quien no la ofrecía (voice.rs).
        let attempts: [(&str, RequestedFormatType); 4] = [
            ("720p", RequestedFormatType::HighestResolution(Resolution::new(VIDEO_WIDTH, VIDEO_HEIGHT))),
            ("480p", RequestedFormatType::HighestResolution(Resolution::new(640, 480))),
            ("más fps", RequestedFormatType::AbsoluteHighestFrameRate),
            ("la mayor", RequestedFormatType::AbsoluteHighestResolution),
        ];
        let usable = usable_cameras();
        let index = usable.first().map(|c| c.index().clone()).unwrap_or(CameraIndex::Index(0));
        println!("cámara: {} útil(es), se usa {index:?}", usable.len());
        let mut camera = None;
        let mut reasons = Vec::new();
        for (asked, format) in attempts {
            match Camera::new(index.clone(), RequestedFormat::new::<RgbFormat>(format)) {
                Ok(mut c) => match c.open_stream() {
                    Ok(()) => {
                        println!("cámara: abierta pidiendo «{asked}» → {}", c.camera_format());
                        camera = Some(c);
                        break;
                    }
                    Err(e) => reasons.push(format!("{asked}: {e}")),
                },
                Err(e) => reasons.push(format!("{asked}: {e}")),
            }
        }
        let Some(mut camera) = camera else {
            let _ = publish.send(VideoChange::Stopped(format!("no aceptó ningún formato ({})", reasons.join("; "))));
            return;
        };

        // La fuente nace con la primera trama y con **sus** medidas, y se
        // rehace —y se republica— si cambian.
        let mut current: Option<(NativeVideoSource, (u32, u32))> = None;
        const STREAK_MAX: u32 = 30;
        let mut in_a_row = 0u32;
        let mut last_error = String::new();
        // Sin probar: la cámara que otra aplicación te quita a media llamada,
        // y desenchufarla. `frame()` puede bloquear sin plazo dentro del
        // driver (voice.rs, `CAPTURE_ALIVE`); aquí sólo lo resuelve salir.
        while in_a_row < STREAK_MAX {
            let raw = match camera.frame() {
                Ok(raw) => raw,
                Err(e) => {
                    in_a_row += 1;
                    last_error = format!("dejó de dar imagen: {e}");
                    continue;
                }
            };
            let rgb = match raw.decode_image::<RgbFormat>() {
                Ok(rgb) => rgb,
                Err(e) => {
                    in_a_row += 1;
                    last_error = format!("no se pudo descodificar: {e}");
                    continue;
                }
            };
            in_a_row = 0;
            let (width, height) = (rgb.width(), rgb.height());
            if width == 0 || height == 0 {
                continue;
            }
            if current.as_ref().map(|(_, dims)| *dims) != Some((width, height)) {
                let source = NativeVideoSource::new(VideoResolution { width, height }, false);
                match current {
                    None => println!("cámara: primera trama {width}x{height}"),
                    Some(_) => println!("cámara: cambió a {width}x{height}; fuente nueva y se republica la pista"),
                }
                if publish.send(VideoChange::Source(source.clone(), width, height)).is_err() {
                    return;
                }
                current = Some((source, (width, height)));
            }
            let Some((source, _)) = current.as_ref() else { continue };
            let mut buffer = I420Buffer::new(width, height);
            rgb_to_i420(rgb.as_raw(), width, height, &mut buffer);
            let frame = VideoFrame { rotation: VideoRotation::VideoRotation0, timestamp_us: now_us(), frame_metadata: None, buffer };
            source.capture_frame(&frame);
            // La vista previa, por el mismo camino que una pista remota.
            let started = Instant::now();
            let image = paint::to_render_image(&frame.buffer);
            preview.slot.conv_us.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
            preview.slot.offer(image);
            let _ = wake.unbounded_send(());
        }
        let _ = camera.stop_stream();
        let _ = publish.send(VideoChange::Stopped(last_error));
    });
    if let Err(e) = spawned {
        eprintln!("cámara: no se pudo lanzar su hilo: {e}");
    }
}

/// RGB de la cámara → I420, BT.601 a mano (voice.rs, `rgb_a_i420`). Una
/// muestra de croma por bloque de 2×2, sin promediar.
fn rgb_to_i420(rgb: &[u8], width: u32, height: u32, dest: &mut I420Buffer) {
    let (w, h) = (width as usize, height as usize);
    let (stride_y, stride_u, stride_v) = dest.strides();
    let (y_plane, u_plane, v_plane) = dest.data_mut();
    for row in 0..h {
        for col in 0..w {
            let i = (row * w + col) * 3;
            let (r, g, b) = (rgb[i] as f32, rgb[i + 1] as f32, rgb[i + 2] as f32);
            y_plane[row * stride_y as usize + col] = (0.257 * r + 0.504 * g + 0.098 * b + 16.0).clamp(0.0, 255.0) as u8;
            if row % 2 == 0 && col % 2 == 0 {
                u_plane[(row / 2) * stride_u as usize + col / 2] = (-0.148 * r - 0.291 * g + 0.439 * b + 128.0).clamp(0.0, 255.0) as u8;
                v_plane[(row / 2) * stride_v as usize + col / 2] = (0.439 * r - 0.368 * g - 0.071 * b + 128.0).clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// BGRA de la pantalla → I420 (voice.rs, `bgra_to_i420`), con el `stride`
/// de verdad de cada fila: en macOS trae relleno (#98). `false`, sin tocar
/// nada, si la trama no cabe en lo que dice medir.
fn bgra_to_i420(bgra: &[u8], width: u32, height: u32, stride: usize, dest: &mut I420Buffer) -> bool {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || stride < w * 4 || bgra.len() < stride * (h - 1) + w * 4 {
        return false;
    }
    let (stride_y, stride_u, stride_v) = dest.strides();
    let (y_plane, u_plane, v_plane) = dest.data_mut();
    for row in 0..h {
        for col in 0..w {
            let i = row * stride + col * 4;
            let (b, g, r) = (bgra[i] as f32, bgra[i + 1] as f32, bgra[i + 2] as f32);
            y_plane[row * stride_y as usize + col] = (0.257 * r + 0.504 * g + 0.098 * b + 16.0).clamp(0.0, 255.0) as u8;
        }
    }
    for row in (0..h).step_by(2) {
        for col in (0..w).step_by(2) {
            let i = row * stride + col * 4;
            let (b, g, r) = (bgra[i] as f32, bgra[i + 1] as f32, bgra[i + 2] as f32);
            u_plane[(row / 2) * stride_u as usize + col / 2] = (-0.148 * r - 0.291 * g + 0.439 * b + 128.0).clamp(0.0, 255.0) as u8;
            v_plane[(row / 2) * stride_v as usize + col / 2] = (0.439 * r - 0.368 * g - 0.071 * b + 128.0).clamp(0.0, 255.0) as u8;
        }
    }
    true
}

/// Lo que se espera a que el sistema conceda la pantalla (el diálogo del
/// portal en Wayland).
const PORTAL_WAIT: Duration = Duration::from_secs(60);

/// Si elige la pantalla el sistema (el portal de Wayland) y no la app.
fn the_system_chooses(session_type: Option<&str>, wayland_display: Option<&str>) -> bool {
    session_type == Some("wayland") || wayland_display.is_some_and(|d| !d.is_empty())
}

/// Un contexto de GLib **propio** para el hilo que habla con el portal
/// (voice.rs, `GlibContext`): GIO entrega las respuestas del portal al
/// contexto por defecto del hilo que hizo la llamada, y sólo si alguien lo
/// itera. Sin él, el diálogo salía y la captura contestaba `Temporary` para
/// siempre. En X11 no hace nada, pero no estorba.
#[cfg(target_os = "linux")]
struct GlibContext(*mut std::ffi::c_void);

#[cfg(target_os = "linux")]
extern "C" {
    fn g_main_context_new() -> *mut std::ffi::c_void;
    fn g_main_context_push_thread_default(context: *mut std::ffi::c_void);
    fn g_main_context_pop_thread_default(context: *mut std::ffi::c_void);
    fn g_main_context_iteration(context: *mut std::ffi::c_void, may_block: i32) -> i32;
    fn g_main_context_unref(context: *mut std::ffi::c_void);
}

#[cfg(target_os = "linux")]
impl GlibContext {
    fn fresh_one() -> Self {
        unsafe {
            let context = g_main_context_new();
            g_main_context_push_thread_default(context);
            Self(context)
        }
    }

    /// Despacha lo pendiente sin bloquear, con tope.
    fn pump(&self) {
        unsafe {
            for _ in 0..64 {
                if g_main_context_iteration(self.0, 0) == 0 {
                    break;
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for GlibContext {
    fn drop(&mut self) {
        unsafe {
            g_main_context_pop_thread_default(self.0);
            g_main_context_unref(self.0);
        }
    }
}

#[cfg(not(target_os = "linux"))]
struct GlibContext;

#[cfg(not(target_os = "linux"))]
impl GlibContext {
    fn fresh_one() -> Self {
        Self
    }
    fn pump(&self) {}
}

/// La pantalla, en su propio hilo, con el capturador de escritorio de
/// libwebrtc. Portado de `start_screen` (voice.rs).
///
/// En X11 (esta máquina) se lista y se elige aquí: `CALLS_SCREEN_SOURCE` es
/// el índice en la lista que se imprime. Sin probar: Wayland, donde la elige
/// el diálogo del portal y la lista trae una entrada de relleno.
fn start_screen(handle: Handle, publish: mpsc::UnboundedSender<VideoChange>, requested: Option<usize>) {
    use livekit::webrtc::desktop_capturer::{CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions};

    let spawned = std::thread::Builder::new().name("pantalla".into()).spawn(move || {
        let _in_runtime = handle.enter();
        // Antes de crear el capturador: GIO se queda con el contexto que
        // encuentre puesto en el momento de la llamada.
        let glib = GlibContext::fresh_one();
        let mut options = DesktopCapturerOptions::new(DesktopCaptureSourceType::Screen);
        options.set_include_cursor(true);
        let Some(mut capturer) = DesktopCapturer::new(options) else {
            let _ = publish.send(VideoChange::Stopped("este sistema no trae capturador".into()));
            return;
        };
        let sources = capturer.get_source_list();
        for (index, source) in sources.iter().enumerate() {
            println!("pantalla: [{index}] id={} «{}»", source.id(), source.title());
        }
        let system_chooses = the_system_chooses(std::env::var("XDG_SESSION_TYPE").ok().as_deref(), std::env::var("WAYLAND_DISPLAY").ok().as_deref());
        let chosen = if system_chooses {
            println!("pantalla: Wayland, elige el diálogo del sistema");
            None
        } else {
            let index = requested.filter(|i| *i < sources.len()).unwrap_or(0);
            println!("pantalla: se comparte la [{index}]");
            sources.get(index).cloned()
        };
        let (sent, received) = std::sync::mpsc::channel();
        capturer.start_capture(chosen, move |result| {
            let _ = sent.send(result.map(|f| (f.width() as u32, f.height() as u32, f.stride() as usize, f.data().to_vec())));
        });

        let limit = Instant::now() + PORTAL_WAIT;
        let mut current: Option<(NativeVideoSource, (u32, u32))> = None;
        // A ritmo de vídeo, nunca en bucle cerrado: el capturador contesta
        // «todavía no» mientras no le dejan trabajar (voice.rs).
        let each = Duration::from_millis(1000 / 30);
        loop {
            glib.pump();
            capturer.capture_frame();
            let mut last_seen = None;
            while let Ok(result) = received.try_recv() {
                last_seen = Some(result);
            }
            match last_seen {
                Some(Ok((width, height, stride, bgra))) => {
                    if current.as_ref().map(|(_, dims)| *dims) != Some((width, height)) {
                        // Pantalla, no cara: el SFU prioriza el detalle.
                        let source = NativeVideoSource::new(VideoResolution { width, height }, true);
                        match current {
                            None => println!("pantalla: primera trama {width}x{height}"),
                            Some(_) => println!("pantalla: cambió a {width}x{height}; fuente nueva y se republica la pista"),
                        }
                        if publish.send(VideoChange::Source(source.clone(), width, height)).is_err() {
                            return;
                        }
                        current = Some((source, (width, height)));
                    }
                    let mut buffer = I420Buffer::new(width, height);
                    if let Some((source, _)) = current.as_ref() {
                        if bgra_to_i420(&bgra, width, height, stride, &mut buffer) {
                            source.capture_frame(&VideoFrame { rotation: VideoRotation::VideoRotation0, timestamp_us: now_us(), frame_metadata: None, buffer });
                        }
                    }
                }
                // `Temporary` es lo normal mientras el sistema pregunta;
                // `Permanent` es que dijo que no.
                Some(Err(CaptureError::Permanent)) => {
                    let _ = publish.send(VideoChange::Stopped("el sistema la denegó".into()));
                    return;
                }
                Some(Err(CaptureError::Temporary)) | None => {}
            }
            if current.is_none() && Instant::now() > limit {
                let _ = publish.send(VideoChange::Stopped("nadie concedió la pantalla en un minuto".into()));
                return;
            }
            std::thread::sleep(each);
        }
    });
    if let Err(e) = spawned {
        eprintln!("pantalla: no se pudo lanzar su hilo: {e}");
    }
}

// ─── Las medidas ──────────────────────────────────────────────────────────────

/// Una línea por segundo, y en `CALLS_CSV` una fila por mosaico y segundo
/// (o una sin vídeo, con la pista «-»).
async fn report(wall: Arc<Wall>, csv_path: Option<String>) {
    use std::io::Write;

    let mut csv = csv_path.and_then(|path| match std::fs::File::create(&path) {
        Ok(file) => Some(file),
        Err(e) => {
            eprintln!("medidas: no se pudo crear {path}: {e}");
            None
        }
    });
    if let Some(csv) = csv.as_mut() {
        let _ = writeln!(csv, "segundo,pista,llegan_por_s,pintados_por_s,perdidos_por_s,convertir_ms,audio_publicadas_por_s,audio_pico,audio_remotas,huecos_por_s,cpu_nucleos,rss_mb");
    }
    // (llegados, pintados, perdidos, µs convirtiendo) de cada mosaico.
    let mut before: HashMap<String, [u64; 4]> = HashMap::new();
    let mut last_cpu = paint::cpu_seconds();
    let mut last_published = FRAMES_PUBLISHED.load(Ordering::Relaxed);
    let mut last_underruns = UNDERRUNS.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let second = started.elapsed().as_secs_f64().round() as u64;
        let cpu = paint::cpu_seconds();
        let cores = cpu - last_cpu;
        last_cpu = cpu;
        let published = FRAMES_PUBLISHED.load(Ordering::Relaxed);
        let published_now = published - last_published;
        last_published = published;
        let underruns = UNDERRUNS.load(Ordering::Relaxed);
        let underruns_now = underruns - last_underruns;
        last_underruns = underruns;
        let peak = PEAK_PERMILLE.swap(0, Ordering::Relaxed) as f64 / 1000.0;
        let remote_audio = TRACKS.lock().unwrap().len();
        let rss = paint::rss_mb();

        let tiles = wall.snapshot();
        let mut rows = Vec::new();
        for tile in &tiles {
            let now = [
                tile.slot.made.load(Ordering::Relaxed),
                tile.painted.load(Ordering::Relaxed),
                tile.slot.lost.load(Ordering::Relaxed),
                tile.slot.conv_us.load(Ordering::Relaxed),
            ];
            let prev = before.insert(tile.id.clone(), now).unwrap_or([0; 4]);
            let arrived = now[0] - prev[0];
            let convert_ms = (now[3] - prev[3]) as f64 / arrived.max(1) as f64 / 1000.0;
            rows.push((tile.label.clone(), arrived, now[1] - prev[1], now[2] - prev[2], convert_ms));
        }
        before.retain(|id, _| tiles.iter().any(|tile| &tile.id == id));

        let video = if rows.is_empty() {
            "ninguno".to_string()
        } else {
            rows.iter()
                .map(|(label, arrived, painted, lost, convert_ms)| format!("[{label}] llegan={arrived} pintados={painted} perdidos={lost} convertir={convert_ms:.2}ms"))
                .collect::<Vec<_>>()
                .join("  ")
        };
        println!(
            "t={second:>4}s  audio: publicadas/s={published_now} pico={peak:.2} remotas={remote_audio} huecos/s={underruns_now}  cpu={cores:.2} núcleos rss={rss:.0}MB  vídeo: {video}"
        );
        if let Some(csv) = csv.as_mut() {
            let tail = format!("{published_now},{peak:.3},{remote_audio},{underruns_now},{cores:.3},{rss:.1}");
            if rows.is_empty() {
                let _ = writeln!(csv, "{second},-,,,,,{tail}");
            }
            for (label, arrived, painted, lost, convert_ms) in &rows {
                let _ = writeln!(csv, "{second},\"{}\",{arrived},{painted},{lost},{convert_ms:.3},{tail}", label.replace('"', "'"));
            }
        }
    }
}
