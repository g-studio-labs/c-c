//! Publicar un vídeo propio: tu cámara y tu pantalla (command-and-control-445).
//!
//! Dos piezas, una a cada lado del hilo que captura:
//!
//! - **`Outlet`**, en el hilo de captura: cada fotograma va a la fuente que se
//!   publica y, si alguien mira la sala, a tu vista previa.
//! - **`run`**, una tarea en el runtime: publica la fuente y la retira al
//!   acabar.
//!
//! **Nada se publica hasta el primer fotograma**: la fuente nace con él, así
//! que nadie ve una pista en negro mientras la cámara arranca o alguien
//! contesta al diálogo del sistema.
//!
//! **Una fuente por publicación, aunque cambie el tamaño.** Medido contra un
//! LiveKit local (20-nov, command-and-control-446): una `NativeVideoSource`
//! creada a 1280×720 acepta fotogramas de 1000×700, 1600×900 y 700×1000, y
//! quien recibe ve el tamaño nuevo en 100–150 ms. La prueba de viabilidad
//! republicaba en cada cambio, y cada republicación es una pista nueva para
//! todos y para la grabación; una ventana cambia de tamaño a menudo. (El
//! fallo de Tauri que la prueba creía arreglar era otro: creaba una fuente
//! nueva y seguía publicando la vieja.)
//!
//! Lo que pasa con la sala va detrás de `TrackRoom`, para probar las reglas
//! sin red; la sala de verdad es `LiveRoom`.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use livekit::options::TrackPublishOptions;
use livekit::track::{LocalTrack, LocalVideoTrack, TrackSource};
use livekit::webrtc::prelude::{I420Buffer, VideoBuffer, VideoFrame as RtcFrame, VideoResolution, VideoRotation};
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::webrtc::video_source::RtcVideoSource;
use livekit::Room;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::error::VoiceError;
use crate::events::{EngineEvent, EventSink};
use crate::video::{self, Feed, VideoFeeds, VideoFrame, VideoSource};

/// Lo que la captura le dice a quien publica.
pub(crate) enum Change<S> {
    /// Llegó el primer fotograma: ésta es la fuente, con sus medidas.
    Publish(S, (u32, u32)),
    /// Se acabó. `None`: lo apagaste tú; `Some`: se rindió sola.
    Stop(Option<VoiceError>),
}

/// La sala, para quien publica.
pub(crate) trait TrackRoom<S>: Send + 'static {
    type Published: Send;
    fn publish(&self, source: S) -> impl Future<Output = Result<Self::Published, String>> + Send;
    fn unpublish(&self, published: Self::Published) -> impl Future<Output = ()> + Send;
}

/// Lo que `run` cuenta después de haberse visto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    On,
    Off(Option<VoiceError>),
}

/// Publica la fuente cuando llega y la retira cuando se acaba.
///
/// `started` se contesta una vez: `Ok` al publicarse (o si lo apagaste antes:
/// no falló nada), o el fallo si no llegó a publicarse; en ese caso no se
/// cuenta ni «encendida» ni «apagada», porque nunca se vio. Lo de después va
/// por `tell`. `died` es el fallo si la captura desaparece sin avisar.
pub(crate) async fn run<S, R: TrackRoom<S>>(room: R, mut changes: mpsc::UnboundedReceiver<Change<S>>, started: oneshot::Sender<Result<(), VoiceError>>, died: VoiceError, tell: impl Fn(Lifecycle)) {
    let failure = match changes.recv().await {
        Some(Change::Publish(source, (width, height))) => match room.publish(source).await {
            Ok(published) => {
                log::info!("vídeo propio: publicado a {width}x{height}");
                let _ = started.send(Ok(()));
                tell(Lifecycle::On);
                let ending = next_stop(&mut changes, died).await;
                room.unpublish(published).await;
                tell(Lifecycle::Off(ending));
                return;
            }
            Err(e) => Some(VoiceError::Publish(e)),
        },
        Some(Change::Stop(error)) => error,
        None => Some(died),
    };
    let _ = started.send(failure.map_or(Ok(()), Err));
}

/// Espera a que se acabe. `Outlet` sólo publica una vez, así que aquí no
/// puede llegar otra fuente.
async fn next_stop<S>(changes: &mut mpsc::UnboundedReceiver<Change<S>>, died: VoiceError) -> Option<VoiceError> {
    match changes.recv().await {
        Some(Change::Stop(error)) => error,
        Some(Change::Publish(..)) => unreachable!("una publicación recibe una sola fuente"),
        None => Some(died),
    }
}

/// Algo que recibe fotogramas para publicarlos.
pub(crate) trait FrameSink: Clone + Send + 'static {
    fn capture(&self, buffer: &I420Buffer);
}

impl FrameSink for NativeVideoSource {
    fn capture(&self, buffer: &I420Buffer) {
        let frame = RtcFrame { rotation: VideoRotation::VideoRotation0, timestamp_us: now_us(), frame_metadata: None, buffer };
        self.capture_frame(&frame);
    }
}

fn now_us() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as i64).unwrap_or(0)
}

/// Tu vista previa: la casilla de tu identidad en `VideoFeeds`, la misma por
/// la que llega el vídeo de los demás, así que quien pinta no distingue.
pub(crate) struct Preview {
    pub feeds: VideoFeeds,
    pub feed: Arc<Feed>,
    pub kind: VideoSource,
}

/// Tu cámara se ve **en espejo**, como en un espejo de verdad; tu pantalla
/// nunca, que el texto se leería al revés. Se voltea en los píxeles porque
/// GPUI no sabe voltear una imagen. Sólo la vista previa: lo publicado va
/// derecho.
pub(crate) fn preview_pixels(kind: VideoSource, frame: VideoFrame) -> VideoFrame {
    match kind {
        VideoSource::Camera => mirror(frame),
        VideoSource::Screen => frame,
    }
}

fn mirror(mut frame: VideoFrame) -> VideoFrame {
    let row = frame.width as usize * 4;
    if row == 0 {
        return frame;
    }
    for line in frame.bgra.chunks_exact_mut(row) {
        let pixels = line.len() / 4;
        for x in 0..pixels / 2 {
            let (left, right) = (x * 4, (pixels - 1 - x) * 4);
            for byte in 0..4 {
                line.swap(left + byte, right + byte);
            }
        }
    }
    frame
}

/// Crea la fuente con las medidas del primer fotograma.
type MakeSource<S> = Box<dyn FnOnce((u32, u32)) -> S + Send>;

/// Adónde va cada fotograma capturado. Vive en el hilo de captura.
pub(crate) struct Outlet<S> {
    changes: mpsc::UnboundedSender<Change<S>>,
    make: Option<MakeSource<S>>,
    source: Option<S>,
    preview: Preview,
}

impl<S: FrameSink> Outlet<S> {
    pub(crate) fn new(changes: mpsc::UnboundedSender<Change<S>>, make: impl FnOnce((u32, u32)) -> S + Send + 'static, preview: Preview) -> Self {
        Outlet { changes, make: Some(Box::new(make)), source: None, preview }
    }

    /// Un fotograma: a la fuente (creada con el primero) y a la vista previa
    /// si alguien mira. Con la sala minimizada **se sigue publicando**: sólo
    /// deja de convertirse lo que nadie ve (`offer` no llama). `false` si ya
    /// nadie publica: el hilo debe acabar.
    pub(crate) fn deliver(&mut self, buffer: &I420Buffer) -> bool {
        if self.changes.is_closed() {
            return false;
        }
        let dims = (buffer.width(), buffer.height());
        if dims.0 == 0 || dims.1 == 0 {
            return true;
        }
        if let Some(make) = self.make.take() {
            let source = make(dims);
            if self.changes.send(Change::Publish(source.clone(), dims)).is_err() {
                return false;
            }
            self.source = Some(source);
        }
        if let Some(source) = &self.source {
            source.capture(buffer);
        }
        let kind = self.preview.kind;
        self.preview.feeds.offer(&self.preview.feed, |(box_w, box_h)| preview_pixels(kind, video::convert(buffer, VideoRotation::VideoRotation0, box_w, box_h)));
        true
    }

    /// La captura se rinde: que se retire lo publicado y se diga por qué.
    pub(crate) fn fail(self, error: VoiceError) {
        let _ = self.changes.send(Change::Stop(Some(error)));
    }
}

/// La sala de verdad. Débil: colgar no espera a que acabe una publicación.
pub(crate) struct LiveRoom {
    room: Weak<Room>,
    kind: TrackSource,
    name: &'static str,
}

impl TrackRoom<NativeVideoSource> for LiveRoom {
    type Published = livekit::id::TrackSid;

    async fn publish(&self, source: NativeVideoSource) -> Result<Self::Published, String> {
        let room = self.room.upgrade().ok_or("la sala ya se cerró")?;
        let track = LocalVideoTrack::create_video_track(self.name, RtcVideoSource::Native(source));
        let options = TrackPublishOptions { source: self.kind, ..Default::default() };
        let publication = room.local_participant().publish_track(LocalTrack::Video(track), options).await.map_err(|e| e.to_string())?;
        Ok(publication.sid())
    }

    async fn unpublish(&self, sid: Self::Published) {
        let Some(room) = self.room.upgrade() else { return };
        if let Err(e) = room.local_participant().unpublish_track(&sid).await {
            log::warn!("vídeo propio: no se pudo retirar la pista {sid}: {e}");
        }
    }
}

/// Lo que se le da al hilo que captura, y lo que se queda quien lo arranca.
pub(crate) struct Publishing {
    kind: VideoSource,
    outlet: Outlet<NativeVideoSource>,
    stop: mpsc::UnboundedSender<Change<NativeVideoSource>>,
    started: oneshot::Receiver<Result<(), VoiceError>>,
}

/// El fallo de un hilo de captura que muere, con el texto de su fuente.
fn thread_died(kind: VideoSource, detail: &str) -> VoiceError {
    match kind {
        VideoSource::Camera => VoiceError::CameraFailed(detail.into()),
        VideoSource::Screen => VoiceError::ScreenFailed(detail.into()),
    }
}

/// Un hilo que captura y publica. Soltarlo lo apaga y retira lo publicado
/// **ya**, sin esperar al hilo, que puede estar bloqueado en el controlador
/// de la cámara o esperando al diálogo del sistema.
pub(crate) struct Capture {
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    publisher: mpsc::UnboundedSender<Change<NativeVideoSource>>,
}

impl Capture {
    /// Baja a `false` cuando su hilo acaba de verdad (soltó el dispositivo).
    pub(crate) fn alive_flag(&self) -> Arc<AtomicBool> {
        self.alive.clone()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.publisher.send(Change::Stop(None));
    }
}

struct AliveGuard(Arc<AtomicBool>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// La respuesta a «enciende»: la primera publicación o el fallo.
pub(crate) type Started = oneshot::Receiver<Result<(), VoiceError>>;

/// Lanza `body` en su propio hilo, que captura y entrega a su `Outlet` hasta
/// que la bandera diga que pare.
pub(crate) fn spawn_capture(name: &str, handle: Handle, publishing: Publishing, body: impl FnOnce(Outlet<NativeVideoSource>, &AtomicBool) + Send + 'static) -> Result<(Capture, Started), VoiceError> {
    let Publishing { kind, outlet, stop: publisher, started } = publishing;
    let stop = Arc::new(AtomicBool::new(false));
    let alive = Arc::new(AtomicBool::new(true));
    let (thread_stop, thread_alive) = (stop.clone(), alive.clone());
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let _alive = AliveGuard(thread_alive);
            // `NativeVideoSource::new` lanza tareas de tokio: desde un hilo
            // suelto sin el runtime entrado, la captura moría en el primer
            // fotograma (voice.rs de Tauri, `enter_runtime`).
            let _runtime = handle.enter();
            body(outlet, &thread_stop);
        })
        .map_err(|e| thread_died(kind, &e.to_string()))?;
    Ok((Capture { stop, alive, publisher }, started))
}

/// Arranca quien publica tu `kind` en la sala y prepara tu vista previa. Los
/// encendidos y apagados se cuentan como `EngineEvent::Video` con tu
/// identidad; un fallo después de verse, además, como
/// `EngineEvent::VideoFailed`.
pub(crate) fn spawn(handle: &Handle, room: Weak<Room>, identity: String, kind: VideoSource, feeds: VideoFeeds, sink: EventSink) -> Publishing {
    let (changes, receiver) = mpsc::unbounded_channel();
    let (answer, started) = oneshot::channel();
    let (track_source, name, screencast) = match kind {
        VideoSource::Camera => (TrackSource::Camera, "camara", false),
        VideoSource::Screen => (TrackSource::Screenshare, "pantalla", true),
    };
    let feed = feeds.open(&identity, kind);
    let preview = Preview { feeds: feeds.clone(), feed, kind };
    let tell = move |lifecycle: Lifecycle| match lifecycle {
        Lifecycle::On => sink(EngineEvent::Video { identity: identity.clone(), source: kind, enabled: true }),
        Lifecycle::Off(error) => {
            feeds.close(&identity, kind);
            if let Some(error) = error {
                log::warn!("vídeo propio: se apagó solo: {error}");
                sink(EngineEvent::VideoFailed { source: kind, error: error.label().into() });
            }
            sink(EngineEvent::Video { identity: identity.clone(), source: kind, enabled: false });
        }
    };
    handle.spawn(run(LiveRoom { room, kind: track_source, name }, receiver, answer, thread_died(kind, "la captura terminó sin avisar"), tell));
    let make = move |(width, height): (u32, u32)| NativeVideoSource::new(VideoResolution { width, height }, screencast);
    Publishing { kind, outlet: Outlet::new(changes.clone(), make, preview), stop: changes, started }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Una sala que apunta lo que se le pide, en orden.
    #[derive(Clone, Default)]
    struct FakeRoom {
        log: Arc<Mutex<Vec<String>>>,
        refuse: bool,
    }

    impl TrackRoom<&'static str> for FakeRoom {
        type Published = &'static str;
        async fn publish(&self, source: &'static str) -> Result<&'static str, String> {
            self.log.lock().unwrap().push(format!("publish {source}"));
            if self.refuse {
                return Err("no".into());
            }
            Ok(source)
        }
        async fn unpublish(&self, published: &'static str) {
            self.log.lock().unwrap().push(format!("unpublish {published}"));
        }
    }

    /// Lo que pasa con una serie de cambios: lo que vio la sala, la
    /// respuesta a quien lo pidió y lo que se contó.
    async fn play(room: FakeRoom, changes: Vec<Change<&'static str>>) -> (Vec<String>, Result<(), VoiceError>, Vec<Lifecycle>) {
        let (tx, rx) = mpsc::unbounded_channel();
        for change in changes {
            tx.send(change).unwrap();
        }
        drop(tx);
        let (answer, started) = oneshot::channel();
        let told = Arc::new(Mutex::new(Vec::new()));
        let seen = told.clone();
        run(room.clone(), rx, answer, VoiceError::CameraNoFrames, move |l| seen.lock().unwrap().push(l)).await;
        let log = room.log.lock().unwrap().clone();
        let told = told.lock().unwrap().clone();
        (log, started.await.unwrap(), told)
    }

    /// Se publica con el primer fotograma, se cuenta encendida y, al
    /// apagar, se retira y se cuenta apagada.
    ///
    /// Roturas que atrapa: no retirar la pista al apagar (los demás verían
    /// tu última imagen congelada); contestar antes de publicar; no contar
    /// que se encendió o que se apagó.
    #[tokio::test]
    async fn publishing_and_stopping() {
        let (log, started, told) = play(FakeRoom::default(), vec![Change::Publish("720p", (1280, 720)), Change::Stop(None)]).await;
        assert_eq!(log, ["publish 720p", "unpublish 720p"]);
        assert_eq!(started, Ok(()));
        assert_eq!(told, [Lifecycle::On, Lifecycle::Off(None)]);
    }

    /// Lo que falla antes de verse es la respuesta, sin «encendida» ni
    /// «apagada»; lo que falla después se cuenta con su fallo; y apagar
    /// antes de que arranque no es un fallo.
    ///
    /// Roturas que atrapa: contestar `Ok` a una cámara que no publicó (el
    /// botón se quedaría girando); contar una apagada de algo que nunca se
    /// vio; perder el porqué de una cámara que se rinde a media llamada;
    /// decir que falló algo que apagaste tú.
    #[tokio::test]
    async fn failures_before_and_after_being_seen() {
        let (_, started, told) = play(FakeRoom::default(), vec![Change::Stop(Some(VoiceError::CameraBusy))]).await;
        assert_eq!((started, told), (Err(VoiceError::CameraBusy), vec![]));

        let refusing = FakeRoom { refuse: true, ..FakeRoom::default() };
        let (_, started, _) = play(refusing, vec![Change::Publish("720p", (1280, 720))]).await;
        assert_eq!(started.map_err(|e| e.label()), Err("voice-publish"));

        let (log, started, told) = play(FakeRoom::default(), vec![Change::Publish("720p", (1280, 720)), Change::Stop(Some(VoiceError::CameraNoFrames))]).await;
        assert_eq!((started, told), (Ok(()), vec![Lifecycle::On, Lifecycle::Off(Some(VoiceError::CameraNoFrames))]));
        assert_eq!(log.last().unwrap(), "unpublish 720p");

        let (_, started, told) = play(FakeRoom::default(), vec![Change::Stop(None)]).await;
        assert_eq!((started, told), (Ok(()), vec![]));
    }

    /// Si la captura desaparece sin avisar (su hilo murió), lo publicado se
    /// retira igual y se dice.
    ///
    /// Roturas que atrapa: dejar publicada para siempre una pista que ya
    /// nadie alimenta (los demás verían tu última imagen congelada); callar
    /// que se apagó.
    #[tokio::test]
    async fn a_capture_that_vanishes_is_unpublished() {
        let (log, _, told) = play(FakeRoom::default(), vec![Change::Publish("720p", (1280, 720))]).await;
        assert_eq!(log, ["publish 720p", "unpublish 720p"]);
        assert_eq!(told, [Lifecycle::On, Lifecycle::Off(Some(VoiceError::CameraNoFrames))]);
    }

    fn frame(width: u32, height: u32, bgra: Vec<u8>) -> VideoFrame {
        VideoFrame { width, height, bgra }
    }

    /// El espejo voltea cada fila, y a la pantalla no se le aplica.
    ///
    /// Roturas que atrapa: voltear en vertical; voltear sólo la primera
    /// fila; voltear bytes sueltos en vez de píxeles (los colores cambian);
    /// voltear la pantalla compartida (el texto al revés).
    #[test]
    fn the_camera_preview_is_mirrored_and_the_screen_is_not() {
        // 3×2: fila 0 = A B C, fila 1 = D E F, cada píxel con sus 4 bytes.
        let px = |n: u8| [n, n + 100, n + 200, 255];
        let source: Vec<u8> = [1, 2, 3, 4, 5, 6].iter().flat_map(|&n| px(n)).collect();
        let mirrored = preview_pixels(VideoSource::Camera, frame(3, 2, source.clone()));
        let expected: Vec<u8> = [3, 2, 1, 6, 5, 4].iter().flat_map(|&n| px(n)).collect();
        assert_eq!(mirrored, frame(3, 2, expected));
        assert_eq!(preview_pixels(VideoSource::Screen, frame(3, 2, source.clone())), frame(3, 2, source));
    }

    /// Una fuente de mentira que cuenta lo que recibe.
    #[derive(Clone)]
    struct CountingSink {
        captured: Captured,
    }

    impl FrameSink for CountingSink {
        fn capture(&self, buffer: &I420Buffer) {
            self.captured.lock().unwrap().push((buffer.width(), buffer.height()));
        }
    }

    type Captured = Arc<Mutex<Vec<(u32, u32)>>>;

    fn outlet(feeds: &VideoFeeds) -> (Outlet<CountingSink>, mpsc::UnboundedReceiver<Change<CountingSink>>, Captured) {
        let (tx, rx) = mpsc::unbounded_channel();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let shared = captured.clone();
        let preview = Preview { feeds: feeds.clone(), feed: feeds.open("u-yo", VideoSource::Camera), kind: VideoSource::Camera };
        (Outlet::new(tx, move |_| CountingSink { captured: shared }, preview), rx, captured)
    }

    /// Con la sala minimizada no se convierte tu vista previa, pero la
    /// captura sigue entregando a lo que se publica; y un tamaño nuevo va a
    /// la misma fuente, sin publicar otra.
    ///
    /// Roturas que atrapa: dejar de publicar al minimizar (los demás te
    /// pierden); seguir convirtiendo la vista previa sin nadie mirando;
    /// publicar una pista nueva en cada fotograma o en cada cambio de tamaño
    /// (una ventana que se redimensiona sería una pista nueva para todos y
    /// para la grabación); perder el fotograma de otro tamaño.
    #[test]
    fn minimized_stops_the_preview_but_not_the_publishing() {
        let feeds = VideoFeeds::default();
        let (mut outlet, mut changes, captured) = outlet(&feeds);
        assert!(outlet.deliver(&I420Buffer::new(4, 2)));
        assert!(outlet.deliver(&I420Buffer::new(4, 2)));
        assert_eq!(feeds.converted(), 2);
        assert!(feeds.take("u-yo", VideoSource::Camera).is_some(), "tu vista previa");

        feeds.set_watching(false);
        assert!(outlet.deliver(&I420Buffer::new(4, 2)));
        assert_eq!(feeds.converted(), 2, "nadie mira: no se convierte");
        assert_eq!(captured.lock().unwrap().len(), 3, "pero se sigue publicando");

        assert!(outlet.deliver(&I420Buffer::new(2, 2)));
        assert_eq!(captured.lock().unwrap().last(), Some(&(2, 2)), "el tamaño nuevo, a la misma fuente");
        let sources: Vec<(u32, u32)> = std::iter::from_fn(|| changes.try_recv().ok())
            .map(|change| match change {
                Change::Publish(_, dims) => dims,
                Change::Stop(_) => panic!("nadie lo paró"),
            })
            .collect();
        assert_eq!(sources, [(4, 2)], "una sola publicación, con las medidas del primero");

        drop(changes);
        assert!(!outlet.deliver(&I420Buffer::new(8, 8)), "sin nadie que publique, el hilo acaba");
    }
}
