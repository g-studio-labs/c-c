//! El motor de voz de esta app: `cac-voice` (LiveKit) puesto detrás del rasgo
//! que el core define (`cac_core::VoiceEngine`).
//!
//! Aquí no se decide nada. El core dice cuándo entrar, salir, silenciar o
//! ensordecer; esto se lo pasa al motor y le devuelve lo que cuenta la sala.
//! Ensordecer: el core manda también «micrófono cerrado», y por eso al quitar
//! la sordera el micrófono sigue cerrado aunque el motor, solo, lo dejaría
//! como estaba (`Call::set_deaf`).
//!
//! Publicar cámara y pantalla: el core lo pide sin esperar (`set_camera`,
//! `share_screen`), y aquí se espera al motor; un fallo vuelve al core como
//! `VoiceEvent::VideoFailed`, y lo que sí sale lo cuenta el propio motor con
//! su `Video`.
//!
//! Los fotogramas de vídeo tampoco pasan por el core: `LiveVoice` es también
//! el `voice_video::Feeds` de la app, y la sala los toma de la llamada viva.
//!
//! Sólo en Linux por ahora (ver `Cargo.toml`). En macOS y Windows no se pone
//! ninguno y el core dice que esta app todavía no tiene llamadas.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Poll;

use cac_core::{Core, ScreenSource, ScreenSources, VideoSource, VoiceEngine, VoiceEvent, VoiceFuture};
use cac_voice::{Call, EngineEvent, EventSink, VoiceError};
use tokio::sync::mpsc;

use crate::voice_video::{Feeds, Frame, Wake};

fn to_core_source(source: cac_voice::VideoSource) -> VideoSource {
    match source {
        cac_voice::VideoSource::Camera => VideoSource::Camera,
        cac_voice::VideoSource::Screen => VideoSource::Screen,
    }
}

fn to_engine_source(source: VideoSource) -> cac_voice::VideoSource {
    match source {
        VideoSource::Camera => cac_voice::VideoSource::Camera,
        VideoSource::Screen => cac_voice::VideoSource::Screen,
    }
}

/// Lo que cuenta el motor, en el idioma del core.
fn to_core(event: EngineEvent) -> VoiceEvent {
    match event {
        EngineEvent::Connected { identity } => VoiceEvent::Connected { identity },
        EngineEvent::Joined { identity, name } => VoiceEvent::Joined { identity, name },
        EngineEvent::Left { identity } => VoiceEvent::Left { identity },
        EngineEvent::Speaking { identities } => VoiceEvent::Speaking { identities },
        EngineEvent::SelfSpeaking { speaking } => VoiceEvent::SelfSpeaking { speaking },
        EngineEvent::Muted { identity, muted } => VoiceEvent::Muted { identity, muted },
        EngineEvent::Latency { ms } => VoiceEvent::Latency { ms },
        EngineEvent::Recording { active, id, by, since } => VoiceEvent::Recording { active, id, by, since },
        EngineEvent::Disconnected { reason } => VoiceEvent::Disconnected { reason },
        EngineEvent::Video { identity, source, enabled } => VoiceEvent::Video { identity, source: to_core_source(source), enabled },
        EngineEvent::VideoFailed { source, error } => VoiceEvent::VideoFailed { source: to_core_source(source), code: error },
    }
}

fn to_core_sources(sources: cac_voice::ScreenSources) -> ScreenSources {
    let convert = |list: Vec<cac_voice::ScreenSource>| list.into_iter().map(|source| ScreenSource { id: source.id, title: source.title }).collect();
    ScreenSources { screens: convert(sources.screens), windows: convert(sources.windows) }
}

/// Una orden de publicar, ya con su llamada dentro.
type Order = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Empieza cada orden en el orden en que llegó, sin esperar a que acabe la
/// anterior. Las dos cosas hacen falta: apagar mientras la cámara arranca
/// tiene que cancelar ese arranque (`Call::set_camera`), no esperar los 15 s
/// a que termine; y si «enciende» y «apaga» se lanzaran sueltas podrían
/// empezar al revés y dejarla encendida. Por eso cada una da aquí su primera
/// vuelta —donde `Call` aparta lo que había— y sólo después se suelta.
async fn run_in_order(mut orders: mpsc::UnboundedReceiver<Order>) {
    while let Some(mut order) = orders.recv().await {
        let first = std::future::poll_fn(|cx| Poll::Ready(order.as_mut().poll(cx))).await;
        if first.is_pending() {
            tokio::spawn(order);
        }
    }
}

/// La llamada en curso y por dónde se le mandan las órdenes de publicar.
#[derive(Clone)]
struct Live {
    call: Call,
    orders: mpsc::UnboundedSender<Order>,
}

/// El motor, para lo que no pasa por el core: elegir micrófono y altavoz.
pub struct Devices(pub cac_voice::Engine);

impl gpui_kit::Global for Devices {}

/// El motor de verdad, con la llamada en curso.
pub struct LiveVoice {
    engine: cac_voice::Engine,
    live: Arc<Mutex<Option<Live>>>,
    core: Core,
    /// A quién despertar cuando llega un fotograma; se le da a cada llamada.
    wake: Arc<Mutex<Option<Wake>>>,
}

impl LiveVoice {
    pub fn new(core: Core) -> Self {
        LiveVoice { engine: cac_voice::Engine::new(), live: Arc::default(), core, wake: Arc::default() }
    }

    /// El motor, para lo que no pasa por el core: los dispositivos.
    pub fn engine(&self) -> cac_voice::Engine {
        self.engine.clone()
    }

    fn live(&self) -> Option<Live> {
        self.live.lock().expect("voice call lock").clone()
    }

    fn with_call(&self, act: impl FnOnce(&Call)) {
        if let Some(live) = self.live() {
            act(&live.call);
        }
    }

    /// Manda una orden de publicar a la llamada; si no sale, se lo dice al
    /// core, que devuelve el botón a como estaba.
    fn publish(&self, source: VideoSource, act: impl FnOnce(Call) -> Pin<Box<dyn Future<Output = Result<(), VoiceError>> + Send>>) {
        let Some(live) = self.live() else { return };
        let (core, attempt) = (self.core.clone(), act(live.call));
        let _ = live.orders.send(Box::pin(async move {
            if let Err(error) = attempt.await {
                core.voice().engine_event(VoiceEvent::VideoFailed { source, code: error.label().to_string() });
            }
        }));
    }
}

impl VoiceEngine for LiveVoice {
    fn join(&self, url: String, token: String, noise_suppression: bool) -> VoiceFuture<Result<String, String>> {
        let (engine, slot, core, wake) = (self.engine.clone(), self.live.clone(), self.core.clone(), self.wake.clone());
        Box::pin(async move {
            engine.set_noise_suppression(noise_suppression);
            // Se llama desde las tareas del motor: el core sólo apunta y
            // avisa, no bloquea.
            let sink: EventSink = Arc::new(move |event| core.voice().engine_event(to_core(event)));
            let call = engine.join(&url, &token, sink).await.map_err(|error| error.label().to_string())?;
            let identity = call.identity().to_string();
            if let Some(wake) = wake.lock().expect("voice wake lock").clone() {
                call.video().on_frame(wake);
            }
            let (orders, received) = mpsc::unbounded_channel();
            tokio::spawn(run_in_order(received));
            *slot.lock().expect("voice call lock") = Some(Live { call, orders });
            Ok(identity)
        })
    }

    fn leave(&self) -> VoiceFuture<()> {
        // Soltar `Live` cierra su cola de órdenes, y con ella su trabajador.
        let live = self.live.lock().expect("voice call lock").take();
        Box::pin(async move {
            if let Some(live) = live {
                live.call.leave().await;
            }
        })
    }

    fn set_mic(&self, on: bool) {
        self.with_call(|call| call.set_mic(on));
    }

    fn set_deaf(&self, on: bool) {
        self.with_call(|call| call.set_deaf(on));
    }

    fn set_noise_suppression(&self, on: bool) {
        self.engine.set_noise_suppression(on);
    }

    fn watch_video(&self, on: bool) {
        self.with_call(|call| call.video().set_watching(on));
    }

    fn set_camera(&self, on: bool) {
        self.publish(VideoSource::Camera, move |call| Box::pin(async move { call.set_camera(on).await }));
    }

    fn screen_sources(&self) -> ScreenSources {
        self.live().map(|live| to_core_sources(live.call.screen_sources())).unwrap_or_default()
    }

    fn share_screen(&self, source: Option<ScreenSource>) {
        let source = source.map(|source| cac_voice::ScreenSource { id: source.id, title: source.title });
        self.publish(VideoSource::Screen, move |call| Box::pin(async move { call.share(source).await }));
    }

    /// Por la cola, como compartir: parar antes de que la petición haya
    /// empezado no la pararía.
    fn stop_share(&self) {
        self.publish(VideoSource::Screen, |call| {
            Box::pin(async move {
                call.stop_share();
                Ok(())
            })
        });
    }
}

impl Feeds for LiveVoice {
    fn take(&self, identity: &str, source: VideoSource) -> Option<Frame> {
        let live = self.live()?;
        let frame = live.call.video().take(identity, to_engine_source(source))?;
        Some(Frame { width: frame.width, height: frame.height, bgra: frame.bgra })
    }

    fn set_target(&self, identity: &str, source: VideoSource, width: u32, height: u32) {
        self.with_call(|call| call.video().set_target(identity, to_engine_source(source), width, height));
    }

    fn on_frame(&self, wake: Wake) {
        *self.wake.lock().expect("voice wake lock") = Some(wake.clone());
        self.with_call(|call| call.video().on_frame(wake));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada evento del motor llega al core como el mismo evento.
    /// Rotura que atrapa: cruzar dos (un silencio que se lee como que se fue).
    #[test]
    fn every_event_keeps_its_meaning() {
        let cases = [
            (EngineEvent::Connected { identity: "u1".into() }, VoiceEvent::Connected { identity: "u1".into() }),
            (EngineEvent::Joined { identity: "u2".into(), name: "Luis".into() }, VoiceEvent::Joined { identity: "u2".into(), name: "Luis".into() }),
            (EngineEvent::Left { identity: "u2".into() }, VoiceEvent::Left { identity: "u2".into() }),
            (EngineEvent::Speaking { identities: vec!["u2".into()] }, VoiceEvent::Speaking { identities: vec!["u2".into()] }),
            (EngineEvent::SelfSpeaking { speaking: true }, VoiceEvent::SelfSpeaking { speaking: true }),
            (EngineEvent::Muted { identity: "u2".into(), muted: true }, VoiceEvent::Muted { identity: "u2".into(), muted: true }),
            (EngineEvent::Latency { ms: 40 }, VoiceEvent::Latency { ms: 40 }),
            (
                EngineEvent::Recording { active: true, id: "r".into(), by: "u2".into(), since: "t".into() },
                VoiceEvent::Recording { active: true, id: "r".into(), by: "u2".into(), since: "t".into() },
            ),
            (EngineEvent::Disconnected { reason: "RoomDeleted".into() }, VoiceEvent::Disconnected { reason: "RoomDeleted".into() }),
            (
                EngineEvent::Video { identity: "u2".into(), source: cac_voice::VideoSource::Screen, enabled: true },
                VoiceEvent::Video { identity: "u2".into(), source: VideoSource::Screen, enabled: true },
            ),
            (
                EngineEvent::Video { identity: "u2".into(), source: cac_voice::VideoSource::Camera, enabled: false },
                VoiceEvent::Video { identity: "u2".into(), source: VideoSource::Camera, enabled: false },
            ),
            (
                EngineEvent::VideoFailed { source: cac_voice::VideoSource::Camera, error: "voice-camera-no-frames".into() },
                VoiceEvent::VideoFailed { source: VideoSource::Camera, code: "voice-camera-no-frames".into() },
            ),
        ];
        for (from, to) in cases {
            assert_eq!(to_core(from), to);
        }
    }

    /// Una orden empieza después de la anterior aunque la anterior tarde en
    /// su primera vuelta, y sin esperar a que la anterior acabe.
    ///
    /// Roturas que atrapa: esperar cada orden entera (apagar la cámara
    /// esperaría a que termine de encenderse); soltarlas sin su primera
    /// vuelta (la de detrás puede empezar antes y la cámara queda encendida
    /// después de apagarla).
    #[test]
    fn orders_start_in_turn_without_waiting_for_each_other() {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let started = Arc::new(Mutex::new(Vec::new()));
        let (orders, received) = mpsc::unbounded_channel::<Order>();
        runtime.spawn(run_in_order(received));
        let (slow, quick) = (started.clone(), started.clone());
        // Como encender una cámara: su primera vuelta aparta lo que había y
        // lanza el hilo, y luego espera sin fin a que publique.
        orders
            .send(Box::pin(async move {
                std::thread::sleep(std::time::Duration::from_millis(50));
                slow.lock().unwrap().push("enciende");
                std::future::pending::<()>().await;
            }))
            .unwrap();
        orders.send(Box::pin(async move { quick.lock().unwrap().push("apaga") })).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while started.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(*started.lock().unwrap(), ["enciende", "apaga"]);
    }
}
