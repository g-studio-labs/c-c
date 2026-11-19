//! El vídeo de una llamada en pantalla: un recuadro por cámara o pantalla
//! (command-and-control-440).
//!
//! Los fotogramas **no pasan por el core**: el core sabe quién tiene cámara y
//! qué va al escenario (`cac_core::voice_layout`), y los píxeles van del motor
//! a cada `VideoTile` por el rasgo `Feeds`. Si pasaran por el `Snapshot`,
//! cada uno clonaría el estado de la voz y avisaría a todo lo que lo observa.
//!
//! Lo que cuesta un fotograma aquí: el motor ya lo dejó reducido a su recuadro
//! y en BGRA (`cac-voice`), así que sólo se envuelve en una `RenderImage` y
//! se sube. **La anterior se tira del atlas siempre** (`drop_image`): el
//! atlas no olvida nada solo, y sin eso un 720p30 pasó de 565 MB a 2,5 GB en
//! treinta segundos en la prueba de viabilidad (`spikes/calls-native`).

use std::collections::HashMap;
use std::sync::Arc;

use cac_core::{VideoSource, VoiceState};
use gpui_kit::*;

/// Un fotograma listo: B, G, R, A por píxel, sin relleno entre filas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Lo que despierta a la sala cuando llega un fotograma. Se llama desde las
/// tareas del motor.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// De dónde salen los fotogramas: el motor de verdad (`voice_engine.rs`) o
/// uno de mentira en las pruebas.
pub trait Feeds: Send + Sync {
    /// El último fotograma de ese vídeo que no se ha pintado.
    fn take(&self, identity: &str, source: VideoSource) -> Option<Frame>;
    /// El tamaño en que se ve, en píxeles físicos: el motor no convierte más.
    fn set_target(&self, identity: &str, source: VideoSource, width: u32, height: u32);
    /// A quién despertar cuando llega uno.
    fn on_frame(&self, wake: Wake);
}

/// Los fotogramas de esta app. Sin él (macOS y Windows, por ahora) no se
/// pinta vídeo: los recuadros enseñan la cara.
#[derive(Clone)]
pub struct VideoFeeds(pub Arc<dyn Feeds>);

impl Global for VideoFeeds {}

/// Un vídeo en pantalla. Pinta su último fotograma y dice su tamaño.
pub struct VideoTile {
    identity: String,
    source: VideoSource,
    shown: Option<Arc<RenderImage>>,
    /// Cuántas imágenes se han tirado del atlas. Para las pruebas: tiene que
    /// ir una detrás de cada imagen nueva.
    pub(crate) dropped: usize,
    pub(crate) painted: usize,
    _release: Subscription,
}

impl VideoTile {
    fn new(identity: String, source: VideoSource, cx: &mut Context<Self>) -> Self {
        // Al quitarse el recuadro (se apagó la cámara, se colgó), su última
        // imagen sale del atlas: si no, se queda allí para siempre.
        let release = cx.on_release(|tile: &mut VideoTile, cx| {
            if let Some(image) = tile.shown.take() {
                cx.drop_image(image, None);
            }
        });
        VideoTile { identity, source, shown: None, dropped: 0, painted: 0, _release: release }
    }

    #[cfg(test)]
    pub fn has_picture(&self) -> bool {
        self.shown.is_some()
    }
}

impl Render for VideoTile {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let feeds = cx.try_global::<VideoFeeds>().cloned();
        if let Some(frame) = feeds.as_ref().and_then(|feeds| feeds.0.take(&self.identity, self.source)) {
            if let Some(pixels) = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra) {
                let fresh = Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]));
                if let Some(old) = self.shown.replace(fresh) {
                    let _ = window.drop_image(old);
                    self.dropped += 1;
                }
                self.painted += 1;
            }
        }
        let (identity, source) = (self.identity.clone(), self.source);
        // Lo que mide el recuadro, para que el motor no convierta más de lo
        // que se ve.
        let measure = canvas(
            move |bounds, window, cx| {
                if let Some(feeds) = cx.try_global::<VideoFeeds>() {
                    let scale = window.scale_factor();
                    let (width, height) = (f32::from(bounds.size.width) * scale, f32::from(bounds.size.height) * scale);
                    feeds.0.set_target(&identity, source, width.round() as u32, height.round() as u32);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        // La cámara llena su recuadro (se recorta lo que sobra); la pantalla
        // se ve entera, con bandas: de una pantalla interesa todo.
        let fit = if self.source == VideoSource::Screen { ObjectFit::Contain } else { ObjectFit::Cover };
        div()
            .id(SharedString::from(format!("voice-video-{}-{}", self.identity, source_name(self.source))))
            .relative()
            .size_full()
            .child(measure)
            .children(self.shown.clone().map(|image| img(image).size_full().object_fit(fit)))
            .test_support()
    }
}

pub fn source_name(source: VideoSource) -> &'static str {
    match source {
        VideoSource::Camera => "camera",
        VideoSource::Screen => "screen",
    }
}

/// Los vídeos de la sala: crea un `VideoTile` por cada vídeo encendido, suelta
/// los que se apagan y los despierta cuando llega un fotograma.
pub struct VideoWall {
    tiles: HashMap<(String, VideoSource), Entity<VideoTile>>,
    /// Si ya se le dijo al motor a quién despertar.
    listening: bool,
}

impl VideoWall {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        VideoWall { tiles: HashMap::new(), listening: false }
    }

    /// Al día con lo que dice el core: un recuadro por vídeo encendido, y con
    /// la sala cerrada, ninguno.
    pub fn sync(&mut self, state: &VoiceState, shown: bool, cx: &mut Context<Self>) {
        self.listen(cx);
        let mut wanted: Vec<(String, VideoSource)> = Vec::new();
        if shown {
            wanted.extend(state.cameras.iter().map(|id| (id.clone(), VideoSource::Camera)));
            wanted.extend(state.screens.iter().map(|id| (id.clone(), VideoSource::Screen)));
        }
        self.tiles.retain(|key, _| wanted.contains(key));
        for (identity, source) in wanted {
            self.tiles.entry((identity.clone(), source)).or_insert_with(|| cx.new(|cx| VideoTile::new(identity, source, cx)));
        }
    }

    /// El recuadro de ese vídeo, si está encendido.
    pub fn tile(&self, identity: &str, source: VideoSource) -> Option<Entity<VideoTile>> {
        self.tiles.get(&(identity.to_string(), source)).cloned()
    }

    /// Un canal de avisos del motor a la sala. Varios seguidos se juntan en
    /// un solo repintado.
    fn listen(&mut self, cx: &mut Context<Self>) {
        if self.listening {
            return;
        }
        let Some(feeds) = cx.try_global::<VideoFeeds>().cloned() else { return };
        self.listening = true;
        // El de tokio, como `backend.rs`: no necesita su runtime para esperar.
        let (sender, mut woken) = tokio::sync::mpsc::unbounded_channel::<()>();
        feeds.0.on_frame(Arc::new(move || {
            let _ = sender.send(());
        }));
        cx.spawn(async move |wall, cx| {
            while woken.recv().await.is_some() {
                while woken.try_recv().is_ok() {}
                let alive = wall.update(cx, |wall, cx| {
                    for tile in wall.tiles.values() {
                        tile.update(cx, |_, cx| cx.notify());
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }
}
