//! El vídeo de los demás: cámaras y pantallas que llegan de la sala
//! (command-and-control-438).
//!
//! El motor sólo **recibe**: no publica cámara ni pantalla (fases siguientes).
//! Por cada pista de vídeo suscrita, una tarea convierte cada fotograma a lo
//! que pinta la interfaz (BGRA, reducido a su recuadro) y deja **el último**
//! en una casilla. Quien pinta lo toma cuando puede; lo que llega entre medias
//! se pisa. Sin nada de GPUI: el motor sigue sin interfaz.
//!
//! De dónde sale: la casilla, el ajuste al recuadro y la conversión, de
//! `spikes/calls-native/src/paint.rs`, que midió que reducir antes de subir
//! baja 4 × 1080p60 de 0,88 a 0,25 núcleos.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use livekit::track::{TrackKind, TrackSource};
use livekit::webrtc::prelude::VideoRotation;
use livekit::webrtc::video_frame::{VideoBuffer, VideoFormatType};
use serde::Serialize;

/// De qué es un vídeo. Lo dice la fuente de la publicación, **nunca** el
/// nombre de la pista (cada app nombra las suyas como quiere).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VideoSource {
    Camera,
    Screen,
}

/// Un fotograma listo para pintar: B, G, R, A por píxel, filas seguidas sin
/// relleno, `width * height * 4` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Lo que despierta a quien pinta cuando hay un fotograma nuevo. Se llama
/// desde las tareas del motor: tiene que volver enseguida.
pub type FrameWake = Arc<dyn Fn() + Send + Sync>;

/// Qué le pasó a una pista, tal como lo cuenta la sala.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackChange {
    Subscribed { muted: bool },
    Unsubscribed,
    Unpublished,
    Muted,
    Unmuted,
}

/// Si un cambio de pista enciende o apaga un vídeo, y de qué. `None` si no es
/// vídeo.
///
/// **Silenciar cuenta como apagar.** El navegador (`livekit-client`) apaga la
/// cámara silenciando la pista, sin despublicarla; el motor de Tauri sólo
/// miraba los silencios de audio y dejaba el último fotograma congelado para
/// siempre. Aquí no.
pub(crate) fn video_change(kind: TrackKind, source: TrackSource, change: TrackChange) -> Option<(VideoSource, bool)> {
    if kind != TrackKind::Video {
        return None;
    }
    // Una fuente que no se conoce es una cámara, como en Tauri: es lo que
    // publica cualquier cliente que no la marca.
    let source = if source == TrackSource::Screenshare { VideoSource::Screen } else { VideoSource::Camera };
    let on = match change {
        TrackChange::Subscribed { muted } => !muted,
        TrackChange::Unmuted => true,
        TrackChange::Unsubscribed | TrackChange::Unpublished | TrackChange::Muted => false,
    };
    Some((source, on))
}

/// A qué tamaño convertir un fotograma de `width`×`height` que se ve en un
/// recuadro de `box_w`×`box_h` píxeles físicos, conservando la proporción.
/// `None` si cabe entero o el recuadro aún no se pintó (0): nunca se agranda.
/// Medidas pares, que I420 las pide.
///
/// Se ajusta como `Contain` aunque la cámara se pinte como `Cover`: lo que
/// sobra del `Cover` se recorta, y reducir más perdería nitidez justo en lo
/// que sí se ve. Con `Contain` nunca se baja de lo que el recuadro enseña.
pub(crate) fn fit_size(width: u32, height: u32, box_w: u32, box_h: u32) -> Option<(u32, u32)> {
    if box_w == 0 || box_h == 0 || (width <= box_w && height <= box_h) {
        return None;
    }
    let scale = (box_w as f64 / width as f64).min(box_h as f64 / height as f64);
    let even = |v: f64| ((v as u32) & !1).max(2);
    Some((even(width as f64 * scale), even(height as f64 * scale)))
}

/// Gira un BGRA ya convertido. libwebrtc 0.4.2 no expone girar un búfer, y
/// un móvil manda la imagen tumbada con la rotación aparte. Va después de
/// reducir: se giran los píxeles que se ven, no los que llegan.
pub(crate) fn rotate(frame: VideoFrame, rotation: VideoRotation) -> VideoFrame {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if rotation == VideoRotation::VideoRotation0 || w == 0 || h == 0 {
        return frame;
    }
    let quarter = matches!(rotation, VideoRotation::VideoRotation90 | VideoRotation::VideoRotation270);
    let (out_w, out_h) = if quarter { (h, w) } else { (w, h) };
    let mut out = vec![0u8; frame.bgra.len()];
    for y in 0..h {
        for x in 0..w {
            // Dónde cae el píxel (x, y) una vez girado en el sentido del reloj.
            let (nx, ny) = match rotation {
                VideoRotation::VideoRotation90 => (h - 1 - y, x),
                VideoRotation::VideoRotation180 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            let from = (y * w + x) * 4;
            let to = (ny * out_w + nx) * 4;
            out[to..to + 4].copy_from_slice(&frame.bgra[from..from + 4]);
        }
    }
    VideoFrame { width: out_w as u32, height: out_h as u32, bgra: out }
}

/// Convierte lo que entrega la sala a lo que se pinta: reducido a su recuadro
/// (por libyuv), en BGRA (`I420ToARGB` deja B, G, R, A en memoria) y derecho.
pub(crate) fn convert(buffer: &dyn VideoBuffer, rotation: VideoRotation, box_w: u32, box_h: u32) -> VideoFrame {
    let quarter = matches!(rotation, VideoRotation::VideoRotation90 | VideoRotation::VideoRotation270);
    // El recuadro se compara con la imagen ya derecha.
    let (box_w, box_h) = if quarter { (box_h, box_w) } else { (box_w, box_h) };
    let (width, height) = (buffer.width(), buffer.height());
    let frame = match fit_size(width, height, box_w, box_h) {
        Some((w, h)) => to_bgra(&buffer.to_i420().scale(w as i32, h as i32)),
        None => to_bgra(buffer),
    };
    rotate(frame, rotation)
}

fn to_bgra(buffer: &dyn VideoBuffer) -> VideoFrame {
    let (width, height) = (buffer.width(), buffer.height());
    let mut bgra = vec![0u8; (width * height * 4) as usize];
    buffer.to_argb(VideoFormatType::ARGB, &mut bgra, width * 4, width as i32, height as i32);
    VideoFrame { width, height, bgra }
}

/// La casilla de un vídeo: el último fotograma sin pintar y el tamaño en que
/// se ve.
#[derive(Default)]
pub(crate) struct Feed {
    ready: Mutex<Option<VideoFrame>>,
    /// Ancho en los 32 bits altos, alto en los bajos; 0 hasta que se pinta.
    target: AtomicU64,
}

impl Feed {
    pub(crate) fn target(&self) -> (u32, u32) {
        let packed = self.target.load(Ordering::Relaxed);
        ((packed >> 32) as u32, packed as u32)
    }
}

type FeedKey = (String, VideoSource);

/// Los vídeos de una llamada. Los clones son los mismos vídeos. Vive en la
/// llamada (no hay globales): al colgar, se vacía.
#[derive(Clone, Default)]
pub struct VideoFeeds {
    inner: Arc<FeedsInner>,
}

struct FeedsInner {
    feeds: Mutex<HashMap<FeedKey, Arc<Feed>>>,
    /// Las publicaciones de vídeo suscritas, para dejar de recibirlas con la
    /// sala minimizada. Vacío en las pruebas: no se pueden fabricar.
    publications: Mutex<HashMap<FeedKey, livekit::prelude::RemoteTrackPublication>>,
    watching: AtomicBool,
    wake: Mutex<Option<FrameWake>>,
    /// Cuántos fotogramas se han convertido. Para comprobar que con la sala
    /// minimizada no se convierte nada.
    converted: AtomicU64,
}

impl Default for FeedsInner {
    fn default() -> Self {
        FeedsInner {
            feeds: Mutex::default(),
            publications: Mutex::default(),
            // Mirando hasta que digan lo contrario: quien entra a una llamada
            // está delante de la sala.
            watching: AtomicBool::new(true),
            wake: Mutex::default(),
            converted: AtomicU64::new(0),
        }
    }
}

impl VideoFeeds {
    /// El último fotograma de ese vídeo que aún no se pintó, si lo hay. Lo
    /// llama quien pinta; el siguiente `take` no lo vuelve a dar.
    pub fn take(&self, identity: &str, source: VideoSource) -> Option<VideoFrame> {
        let feed = self.inner.feeds.lock().unwrap().get(&(identity.to_string(), source)).cloned()?;
        let frame = feed.ready.lock().unwrap().take();
        frame
    }

    /// El tamaño en que se ve ese vídeo, en píxeles físicos. Los fotogramas
    /// siguientes se convierten a ese tamaño (nunca más grandes de lo que
    /// llegan). 0 = sin límite.
    pub fn set_target(&self, identity: &str, source: VideoSource, width: u32, height: u32) {
        if let Some(feed) = self.inner.feeds.lock().unwrap().get(&(identity.to_string(), source)) {
            feed.target.store((width as u64) << 32 | height as u64, Ordering::Relaxed);
        }
    }

    /// Si alguien mira la sala. Sin nadie mirando (la sala minimizada) no se
    /// convierte nada **y** se le pide al servidor que deje de mandar el
    /// vídeo: no se gasta red ni CPU en lo que no se ve.
    pub fn set_watching(&self, on: bool) {
        self.inner.watching.store(on, Ordering::SeqCst);
        for publication in self.inner.publications.lock().unwrap().values() {
            publication.set_enabled(on);
        }
        if !on {
            // Lo que quedó sin pintar ya es viejo al volver.
            for feed in self.inner.feeds.lock().unwrap().values() {
                feed.ready.lock().unwrap().take();
            }
        }
    }

    pub fn is_watching(&self) -> bool {
        self.inner.watching.load(Ordering::SeqCst)
    }

    /// A quién avisar cuando llega un fotograma. Uno solo: el último gana.
    pub fn on_frame(&self, wake: FrameWake) {
        *self.inner.wake.lock().unwrap() = Some(wake);
    }

    /// Cuántos fotogramas se han convertido desde que empezó la llamada.
    pub fn converted(&self) -> u64 {
        self.inner.converted.load(Ordering::Relaxed)
    }

    /// Abre la casilla de un vídeo que empieza a llegar. Si ya había una (se
    /// volvió a suscribir), sigue la misma: quien pinta no pierde su tamaño.
    pub(crate) fn open(&self, identity: &str, source: VideoSource) -> Arc<Feed> {
        self.inner.feeds.lock().unwrap().entry((identity.to_string(), source)).or_default().clone()
    }

    /// Guarda la publicación para poder pausarla, y la pausa ya si la sala
    /// está minimizada: lo que se suscribe después hereda.
    pub(crate) fn track_publication(&self, identity: &str, source: VideoSource, publication: livekit::prelude::RemoteTrackPublication) {
        if !self.is_watching() {
            publication.set_enabled(false);
        }
        self.inner.publications.lock().unwrap().insert((identity.to_string(), source), publication);
    }

    /// Ese vídeo se apagó o se fue: fuera su casilla y su último fotograma,
    /// para que nadie pinte una cara congelada.
    pub(crate) fn close(&self, identity: &str, source: VideoSource) {
        let key = (identity.to_string(), source);
        self.inner.feeds.lock().unwrap().remove(&key);
        self.inner.publications.lock().unwrap().remove(&key);
    }

    /// Quitar el fotograma pendiente sin cerrar la casilla: la pista sigue,
    /// pero está en silencio.
    pub(crate) fn blank(&self, identity: &str, source: VideoSource) {
        if let Some(feed) = self.inner.feeds.lock().unwrap().get(&(identity.to_string(), source)) {
            feed.ready.lock().unwrap().take();
        }
    }

    /// Alguien se fue de la sala: todos sus vídeos.
    pub(crate) fn forget_person(&self, identity: &str) {
        self.inner.feeds.lock().unwrap().retain(|(who, _), _| who != identity);
        self.inner.publications.lock().unwrap().retain(|(who, _), _| who != identity);
    }

    /// Al colgar: nada de la sala anterior se queda para la siguiente.
    pub(crate) fn clear(&self) {
        self.inner.feeds.lock().unwrap().clear();
        self.inner.publications.lock().unwrap().clear();
    }

    /// Un fotograma recién llegado a su casilla. Si nadie mira no se
    /// convierte: `make` no se llama.
    pub(crate) fn offer(&self, feed: &Feed, make: impl FnOnce((u32, u32)) -> VideoFrame) {
        if !self.is_watching() {
            return;
        }
        let frame = make(feed.target());
        self.inner.converted.fetch_add(1, Ordering::Relaxed);
        *feed.ready.lock().unwrap() = Some(frame);
        let wake = self.inner.wake.lock().unwrap().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn frame(width: u32, height: u32, fill: u8) -> VideoFrame {
        VideoFrame { width, height, bgra: vec![fill; (width * height * 4) as usize] }
    }

    /// Nunca se agranda; se reduce conservando la proporción y en pares.
    #[test]
    fn a_frame_is_shrunk_to_its_box_but_never_grown() {
        assert_eq!(fit_size(1280, 720, 0, 0), None, "sin recuadro todavía, entero");
        assert_eq!(fit_size(640, 360, 1280, 720), None, "cabe: no se agranda");
        assert_eq!(fit_size(1920, 1080, 640, 640), Some((640, 360)));
        // Proporción rara: el lado que manda es el alto, y las medidas pares.
        assert_eq!(fit_size(1920, 1080, 1000, 301), Some((534, 300)));
        let (w, h) = fit_size(1281, 721, 333, 333).unwrap();
        assert!(w % 2 == 0 && h % 2 == 0, "{w}×{h}");
    }

    /// Silenciar o quitar un vídeo lo apaga; el audio no es cosa de esto; una
    /// pantalla es pantalla y lo que no se sabe, cámara.
    #[test]
    fn muting_a_video_turns_it_off() {
        use TrackChange::*;
        assert_eq!(video_change(TrackKind::Video, TrackSource::Camera, Muted), Some((VideoSource::Camera, false)));
        assert_eq!(video_change(TrackKind::Video, TrackSource::Camera, Unmuted), Some((VideoSource::Camera, true)));
        assert_eq!(video_change(TrackKind::Video, TrackSource::Screenshare, Subscribed { muted: false }), Some((VideoSource::Screen, true)));
        assert_eq!(video_change(TrackKind::Video, TrackSource::Camera, Subscribed { muted: true }), Some((VideoSource::Camera, false)), "suscribirse a una cámara ya apagada no la enciende");
        assert_eq!(video_change(TrackKind::Video, TrackSource::Screenshare, Unpublished), Some((VideoSource::Screen, false)));
        assert_eq!(video_change(TrackKind::Video, TrackSource::Unknown, Unsubscribed), Some((VideoSource::Camera, false)));
        assert_eq!(video_change(TrackKind::Audio, TrackSource::Microphone, Muted), None);
    }

    /// La casilla guarda sólo el último, y lo da una vez.
    #[test]
    fn the_feed_keeps_only_the_latest_frame() {
        let feeds = VideoFeeds::default();
        let feed = feeds.open("u-ana", VideoSource::Camera);
        feeds.offer(&feed, |_| frame(2, 2, 1));
        feeds.offer(&feed, |_| frame(2, 2, 2));
        assert_eq!(feeds.take("u-ana", VideoSource::Camera), Some(frame(2, 2, 2)));
        assert_eq!(feeds.take("u-ana", VideoSource::Camera), None, "ya se dio");
        assert_eq!(feeds.take("u-ana", VideoSource::Screen), None, "otra fuente, otra casilla");
    }

    /// Con la sala minimizada no se convierte nada ni se despierta a nadie, y
    /// lo pendiente se tira.
    #[test]
    fn nothing_is_converted_while_nobody_watches() {
        let feeds = VideoFeeds::default();
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = woken.clone();
        feeds.on_frame(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        let feed = feeds.open("u-ana", VideoSource::Camera);
        feeds.offer(&feed, |_| frame(2, 2, 1));
        feeds.set_watching(false);
        assert_eq!(feeds.take("u-ana", VideoSource::Camera), None, "lo pendiente se tira al dejar de mirar");
        feeds.offer(&feed, |_| panic!("se convirtió sin que nadie mirara"));
        assert_eq!((feeds.converted(), woken.load(Ordering::SeqCst)), (1, 1));
        feeds.set_watching(true);
        feeds.offer(&feed, |_| frame(2, 2, 3));
        assert_eq!((feeds.converted(), woken.load(Ordering::SeqCst)), (2, 2));
    }

    /// Quien pinta dice su tamaño y quien convierte lo lee.
    #[test]
    fn the_box_size_reaches_the_conversion() {
        let feeds = VideoFeeds::default();
        let feed = feeds.open("u-ana", VideoSource::Screen);
        feeds.set_target("u-ana", VideoSource::Screen, 640, 360);
        let mut seen = None;
        feeds.offer(&feed, |target| {
            seen = Some(target);
            frame(2, 2, 0)
        });
        assert_eq!(seen, Some((640, 360)));
    }

    /// Quien se va o apaga no deja su último fotograma.
    #[test]
    fn leaving_or_closing_drops_the_last_frame() {
        let feeds = VideoFeeds::default();
        let camera = feeds.open("u-ana", VideoSource::Camera);
        let screen = feeds.open("u-ana", VideoSource::Screen);
        let other = feeds.open("u-luis", VideoSource::Camera);
        for feed in [&camera, &screen, &other] {
            feeds.offer(feed, |_| frame(2, 2, 9));
        }
        feeds.close("u-luis", VideoSource::Camera);
        assert_eq!(feeds.take("u-luis", VideoSource::Camera), None);
        feeds.forget_person("u-ana");
        assert_eq!(feeds.take("u-ana", VideoSource::Camera), None);
        assert_eq!(feeds.take("u-ana", VideoSource::Screen), None);
    }

    /// Girar un cuarto: lo de arriba a la izquierda acaba arriba a la derecha.
    #[test]
    fn a_quarter_turn_moves_the_corner_clockwise() {
        // 3×2, un píxel marcado arriba a la izquierda.
        let mut source = frame(3, 2, 0);
        source.bgra[0..4].copy_from_slice(&[1, 2, 3, 4]);
        let turned = rotate(source.clone(), VideoRotation::VideoRotation90);
        assert_eq!((turned.width, turned.height), (2, 3));
        assert_eq!(&turned.bgra[4..8], &[1, 2, 3, 4], "arriba a la derecha");
        let half = rotate(source.clone(), VideoRotation::VideoRotation180);
        assert_eq!(&half.bgra[(5 * 4)..(6 * 4)], &[1, 2, 3, 4], "abajo a la derecha");
        let back = rotate(source.clone(), VideoRotation::VideoRotation270);
        assert_eq!((back.width, back.height), (2, 3));
        assert_eq!(&back.bgra[(4 * 4)..(5 * 4)], &[1, 2, 3, 4], "abajo a la izquierda");
        assert_eq!(rotate(source.clone(), VideoRotation::VideoRotation0), source);
    }
}
