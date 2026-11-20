//! Publicar un vídeo propio: tu cámara (y, en la subtarea B, tu pantalla)
//! (command-and-control-445).
//!
//! Dos piezas, una a cada lado del hilo que captura:
//!
//! - **`Outlet`**, en el hilo de captura: cada fotograma va a la fuente que se
//!   publica y, si alguien mira la sala, a tu vista previa. Si cambian las
//!   medidas, pide una fuente nueva.
//! - **`run`**, una tarea en el runtime: publica cada fuente nueva y retira la
//!   anterior. Es el arreglo de un fallo del motor de Tauri: si la cámara
//!   cambiaba de resolución a media llamada, el hilo alimentaba una fuente
//!   nueva y la pista publicada seguía atada a la vieja, que ya nadie
//!   alimentaba: los demás veían la imagen congelada. Una fuente nueva
//!   necesita una pista nueva.
//!
//! **Nada se publica hasta el primer fotograma**: la fuente nace con él (y con
//! sus medidas), así que nadie ve una pista en negro mientras la cámara
//! arranca.
//!
//! Lo que pasa con la sala va detrás de `TrackRoom`, para probar las reglas
//! sin red; la sala de verdad es `LiveRoom`. Portado de `spawn_publisher` y
//! `start_camera` de `spikes/calls-native/src/call.rs`.

use std::future::Future;
use std::sync::{Arc, Weak};

use livekit::options::TrackPublishOptions;
use livekit::track::{LocalTrack, LocalVideoTrack, TrackSource};
use livekit::webrtc::prelude::{I420Buffer, VideoBuffer, VideoFrame as RtcFrame, VideoRotation};
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
    /// Una fuente nueva con sus medidas: la primera, o la de después de un
    /// cambio de tamaño.
    Source(S, (u32, u32)),
    /// Se acabó. `None`: lo apagaste tú; `Some`: se rindió sola.
    Stopped(Option<VoiceError>),
}

/// La sala, para quien publica. Detrás de un rasgo para probar las reglas de
/// `run` sin red.
pub(crate) trait TrackRoom<S>: Send + 'static {
    /// Lo que hace falta para retirar lo publicado.
    type Published: Send;
    fn publish(&self, source: S) -> impl Future<Output = Result<Self::Published, String>> + Send;
    fn unpublish(&self, published: Self::Published) -> impl Future<Output = ()> + Send;
}

/// Lo que `run` cuenta de cada publicación, para el resto del motor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    /// La primera fuente ya se publicó: se ve.
    On,
    /// Se dejó de publicar, con el fallo si lo hubo.
    Off(Option<VoiceError>),
}

/// Publica cada fuente que llega y retira la anterior, **siempre en ese
/// orden**: retirar primero deja un instante sin vídeo; publicar primero
/// dejaría dos pistas tuyas a la vez, y los demás verían dos cámaras.
///
/// `started` se contesta una vez: `Ok` con la primera publicación (o si lo
/// apagaste antes: no falló nada), o el fallo si no llegó a publicarse. Lo
/// que pase después se cuenta por `tell`. `died` es el fallo si la captura
/// desaparece sin avisar (su hilo murió).
pub(crate) async fn run<S, R: TrackRoom<S>>(room: R, mut changes: mpsc::UnboundedReceiver<Change<S>>, started: oneshot::Sender<Result<(), VoiceError>>, died: VoiceError, tell: impl Fn(Lifecycle)) {
    let mut started = Some(started);
    let mut published: Option<R::Published> = None;
    let ending = loop {
        let Some(change) = changes.recv().await else { break Some(died) };
        if let Some(previous) = published.take() {
            room.unpublish(previous).await;
        }
        match change {
            Change::Source(source, (width, height)) => match room.publish(source).await {
                Ok(fresh) => {
                    log::info!("vídeo propio: publicado a {width}x{height}");
                    published = Some(fresh);
                    if let Some(started) = started.take() {
                        let _ = started.send(Ok(()));
                        tell(Lifecycle::On);
                    }
                }
                Err(e) => break Some(VoiceError::Publish(e)),
            },
            Change::Stopped(error) => break error,
        }
    };
    if let Some(previous) = published.take() {
        room.unpublish(previous).await;
    }
    match started.take() {
        // No llegó a verse: el fallo es la respuesta a quien lo pidió, y no
        // hay nada que apagar.
        Some(started) => {
            let _ = started.send(ending.map_or(Ok(()), Err));
        }
        None => tell(Lifecycle::Off(ending)),
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

/// Lo que ve quien publica de sí mismo: la cámara **en espejo**, como en un
/// espejo de verdad (levantar la mano derecha la levanta a la derecha); la
/// pantalla, nunca, que el texto se leería al revés. Se voltea en los
/// píxeles porque GPUI no sabe voltear una imagen. Sólo la vista previa: lo
/// que se publica va derecho.
pub(crate) fn preview_pixels(kind: VideoSource, frame: VideoFrame) -> VideoFrame {
    match kind {
        VideoSource::Camera => mirror(frame),
        VideoSource::Screen => frame,
    }
}

/// Voltea en horizontal: cada fila, al revés, píxel a píxel (4 bytes).
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

/// Adónde va cada fotograma capturado. Vive en el hilo de captura.
pub(crate) struct Outlet<S> {
    changes: mpsc::UnboundedSender<Change<S>>,
    make: Box<dyn Fn((u32, u32)) -> S + Send>,
    current: Option<(S, (u32, u32))>,
    preview: Preview,
}

impl<S: FrameSink> Outlet<S> {
    /// `make` crea una fuente de esas medidas.
    pub(crate) fn new(changes: mpsc::UnboundedSender<Change<S>>, make: impl Fn((u32, u32)) -> S + Send + 'static, preview: Preview) -> Self {
        Outlet { changes, make: Box::new(make), current: None, preview }
    }

    /// Un fotograma: a la fuente (nueva si cambió de tamaño) y a la vista
    /// previa, si alguien mira. Con la sala minimizada **se sigue
    /// publicando**: sólo deja de convertirse lo que nadie ve (`offer` no
    /// llama). `false` si ya nadie publica: el hilo debe acabar.
    pub(crate) fn deliver(&mut self, buffer: &I420Buffer) -> bool {
        let dims = (buffer.width(), buffer.height());
        if dims.0 == 0 || dims.1 == 0 {
            return true;
        }
        if self.current.as_ref().map(|(_, current)| *current) != Some(dims) {
            let source = (self.make)(dims);
            if self.changes.send(Change::Source(source.clone(), dims)).is_err() {
                return false;
            }
            self.current = Some((source, dims));
        }
        if let Some((source, _)) = &self.current {
            source.capture(buffer);
        }
        let kind = self.preview.kind;
        self.preview.feeds.offer(&self.preview.feed, |(box_w, box_h)| preview_pixels(kind, video::convert(buffer, VideoRotation::VideoRotation0, box_w, box_h)));
        true
    }

    /// La captura se rinde: que se retire lo publicado y se diga por qué.
    pub(crate) fn fail(self, error: VoiceError) {
        let _ = self.changes.send(Change::Stopped(Some(error)));
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
    /// Para el hilo: por aquí manda los fotogramas.
    pub outlet: Outlet<NativeVideoSource>,
    /// Para quien lo apaga: avisar de que se acabó sin esperar al hilo (que
    /// puede estar bloqueado en el controlador).
    pub stop: mpsc::UnboundedSender<Change<NativeVideoSource>>,
    /// Se contesta con la primera publicación o con el fallo.
    pub started: oneshot::Receiver<Result<(), VoiceError>>,
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
    let died = VoiceError::CameraFailed("la captura terminó sin avisar".into());
    handle.spawn(run(LiveRoom { room, kind: track_source, name }, receiver, answer, died, tell));
    let make = move |(width, height): (u32, u32)| NativeVideoSource::new(livekit::webrtc::prelude::VideoResolution { width, height }, screencast);
    Publishing { outlet: Outlet::new(changes.clone(), make, preview), stop: changes, started }
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

    /// Una fuente de otro tamaño retira la pista anterior **antes** de
    /// publicar la nueva, y apagar retira la última.
    ///
    /// Roturas que atrapa: publicar la nueva sin retirar la vieja (dos
    /// cámaras tuyas a la vez); retirarla después de publicar; dejar la
    /// última publicada al apagar; contar «encendida» en cada cambio.
    #[tokio::test]
    async fn a_new_size_unpublishes_the_old_track_first() {
        let changes = vec![Change::Source("720p", (1280, 720)), Change::Source("480p", (640, 480)), Change::Stopped(None)];
        let (log, started, told) = play(FakeRoom::default(), changes).await;
        assert_eq!(log, ["publish 720p", "unpublish 720p", "publish 480p", "unpublish 480p"]);
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
        let (_, started, told) = play(FakeRoom::default(), vec![Change::Stopped(Some(VoiceError::CameraBusy))]).await;
        assert_eq!((started, told), (Err(VoiceError::CameraBusy), vec![]));

        let refusing = FakeRoom { refuse: true, ..FakeRoom::default() };
        let (_, started, _) = play(refusing, vec![Change::Source("720p", (1280, 720))]).await;
        assert_eq!(started.map_err(|e| e.label()), Err("voice-publish"));

        let (log, started, told) = play(FakeRoom::default(), vec![Change::Source("720p", (1280, 720)), Change::Stopped(Some(VoiceError::CameraNoFrames))]).await;
        assert_eq!((started, told), (Ok(()), vec![Lifecycle::On, Lifecycle::Off(Some(VoiceError::CameraNoFrames))]));
        assert_eq!(log.last().unwrap(), "unpublish 720p");

        let (_, started, told) = play(FakeRoom::default(), vec![Change::Stopped(None)]).await;
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
        let (log, _, told) = play(FakeRoom::default(), vec![Change::Source("720p", (1280, 720))]).await;
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
        dims: (u32, u32),
        captured: Captured,
    }

    impl FrameSink for CountingSink {
        fn capture(&self, buffer: &I420Buffer) {
            assert_eq!((buffer.width(), buffer.height()), self.dims, "un fotograma a una fuente de otras medidas");
            self.captured.lock().unwrap().push(self.dims);
        }
    }

    type Captured = Arc<Mutex<Vec<(u32, u32)>>>;

    fn outlet(feeds: &VideoFeeds) -> (Outlet<CountingSink>, mpsc::UnboundedReceiver<Change<CountingSink>>, Captured) {
        let (tx, rx) = mpsc::unbounded_channel();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let shared = captured.clone();
        let preview = Preview { feeds: feeds.clone(), feed: feeds.open("u-yo", VideoSource::Camera), kind: VideoSource::Camera };
        (Outlet::new(tx, move |dims| CountingSink { dims, captured: shared.clone() }, preview), rx, captured)
    }

    /// Con la sala minimizada no se convierte tu vista previa, pero la
    /// captura sigue entregando a lo que se publica; y una fuente nueva sólo
    /// cuando cambian las medidas.
    ///
    /// Roturas que atrapa: dejar de publicar al minimizar (los demás te
    /// pierden); seguir convirtiendo la vista previa sin nadie mirando;
    /// pedir una fuente (una pista nueva) en cada fotograma; mandar el
    /// fotograma nuevo a la fuente vieja.
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
        let sources: Vec<(u32, u32)> = std::iter::from_fn(|| changes.try_recv().ok())
            .map(|change| match change {
                Change::Source(_, dims) => dims,
                Change::Stopped(_) => panic!("nadie lo paró"),
            })
            .collect();
        assert_eq!(sources, [(4, 2), (2, 2)], "una fuente por tamaño, no por fotograma");

        drop(changes);
        assert!(!outlet.deliver(&I420Buffer::new(8, 8)), "sin nadie que publique, el hilo acaba");
    }
}
