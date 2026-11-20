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
//!
//! Una cámara se ve **entera**, nunca recortada: un móvil en vertical en un
//! recuadro ancho, recortado, era una nariz en pantalla completa (jose, 19-nov,
//! con su teléfono). Lo que sobra a los lados lo llena el mismo vídeo en
//! pequeño y desenfocado (`backdrop`), como en Meet.

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
    /// El fondo desenfocado de una cámara, hecho del mismo fotograma.
    blurred: Option<Arc<RenderImage>>,
    /// Cuántas imágenes se han tirado del atlas. Para las pruebas: tiene que
    /// ir una detrás de cada imagen nueva.
    pub(crate) dropped: usize,
    /// Lo mismo, de los fondos.
    pub(crate) blurred_dropped: usize,
    pub(crate) painted: usize,
    _release: Subscription,
}

impl VideoTile {
    fn new(identity: String, source: VideoSource, cx: &mut Context<Self>) -> Self {
        // Al quitarse el recuadro (se apagó la cámara, se colgó), su última
        // imagen sale del atlas: si no, se queda allí para siempre.
        let release = cx.on_release(|tile: &mut VideoTile, cx| {
            for image in [tile.shown.take(), tile.blurred.take()].into_iter().flatten() {
                cx.drop_image(image, None);
            }
        });
        VideoTile { identity, source, shown: None, blurred: None, dropped: 0, blurred_dropped: 0, painted: 0, _release: release }
    }

    #[cfg(test)]
    pub fn has_picture(&self) -> bool {
        self.shown.is_some()
    }

    #[cfg(test)]
    pub fn has_backdrop(&self) -> bool {
        self.blurred.is_some()
    }
}

impl Render for VideoTile {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let feeds = cx.try_global::<VideoFeeds>().cloned();
        if let Some(frame) = feeds.as_ref().and_then(|feeds| feeds.0.take(&self.identity, self.source)) {
            // Una pantalla va sobre negro: de ella interesa leerla, y un fondo
            // con sus colores sólo distrae.
            let blurred = (self.source == VideoSource::Camera).then(|| backdrop(&frame)).flatten();
            if let Some(blurred) = blurred.and_then(|small| image::RgbaImage::from_raw(small.width, small.height, small.bgra)) {
                let fresh = Arc::new(RenderImage::new(vec![image::Frame::new(blurred)]));
                if let Some(old) = self.blurred.replace(fresh) {
                    let _ = window.drop_image(old);
                    self.blurred_dropped += 1;
                }
            }
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
        // El fondo se estira con filtro lineal desde unas decenas de píxeles,
        // y eso ya lo desenfoca. Sobresale del recuadro (que lo recorta) porque
        // el atlas no deja margen entre imágenes: el borde de algo tan
        // estirado mezclaría la imagen vecina en una franja visible.
        let backdrop = self.blurred.clone().map(|image| {
            div()
                .absolute()
                .top(px(-BACKDROP_BLEED))
                .left(px(-BACKDROP_BLEED))
                .right(px(-BACKDROP_BLEED))
                .bottom(px(-BACKDROP_BLEED))
                .child(img(image).absolute().inset_0().size_full().object_fit(ObjectFit::Cover))
                .child(div().absolute().inset_0().bg(hsla(0., 0., 0., 0.45)))
        });
        div()
            .id(SharedString::from(format!("voice-video-{}-{}", self.identity, source_name(self.source))))
            .relative()
            .size_full()
            .overflow_hidden()
            .child(measure)
            .children(backdrop)
            // Clavada a los bordes y no con `size_full`: `img` le pone al
            // estilo la proporción de la imagen, y con altura en porcentaje
            // crecía de alto hasta tenerla. Un móvil en vertical medía tres
            // veces el recuadro y éste lo recortaba: «entero» se veía igual
            // que «recortado» (19-nov, lo encontró jose a pantalla completa).
            .children(self.shown.clone().map(|image| {
                img(image)
                    .id(SharedString::from(format!("voice-video-{}-{}-picture", self.identity, source_name(self.source))))
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(video_fit())
                    .test_support()
            }))
            .test_support()
    }
}

/// Cómo cabe un vídeo, cámara o pantalla, en su recuadro: entero, nunca
/// recortado. Lo que sobra lo llena el fondo (cámara) o el negro (pantalla).
pub(crate) fn video_fit() -> ObjectFit {
    ObjectFit::Contain
}

/// El lado largo del fondo desenfocado, en píxeles.
const BACKDROP_SIDE: u32 = 48;
/// Cuánto sobresale el fondo de su recuadro por cada lado.
const BACKDROP_BLEED: f32 = 64.;

/// El fondo de una cámara: el fotograma en pequeño (lado largo de
/// `BACKDROP_SIDE` como mucho) y suavizado. `None` si el fotograma no cuadra
/// con su tamaño.
///
/// Cada píxel del fondo es la media de 4×4 muestras de su trozo del
/// fotograma, no de todos sus píxeles: para algo que se ve borroso basta, y
/// se hace en cada fotograma de cada cámara.
pub(crate) fn backdrop(frame: &Frame) -> Option<Frame> {
    let (source_w, source_h) = (frame.width as usize, frame.height as usize);
    if source_w == 0 || source_h == 0 || frame.bgra.len() != source_w * source_h * 4 {
        return None;
    }
    let scale = (BACKDROP_SIDE as f32 / source_w.max(source_h) as f32).min(1.0);
    let width = ((source_w as f32 * scale).round() as usize).max(1);
    let height = ((source_h as f32 * scale).round() as usize).max(1);
    const SAMPLES: usize = 4;
    let mut small = vec![0u8; width * height * 4];
    for y in 0..height {
        let (top, bottom) = (y * source_h / height, ((y + 1) * source_h / height).max(y * source_h / height + 1));
        for x in 0..width {
            let (left, right) = (x * source_w / width, ((x + 1) * source_w / width).max(x * source_w / width + 1));
            let mut sum = [0u32; 3];
            for sy in 0..SAMPLES {
                let row = top + (bottom - top) * (2 * sy + 1) / (2 * SAMPLES);
                for sx in 0..SAMPLES {
                    let column = left + (right - left) * (2 * sx + 1) / (2 * SAMPLES);
                    let at = (row * source_w + column) * 4;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += frame.bgra[at + channel] as u32;
                    }
                }
            }
            let at = (y * width + x) * 4;
            for (channel, total) in sum.iter().enumerate() {
                small[at + channel] = (total / (SAMPLES * SAMPLES) as u32) as u8;
            }
            small[at + 3] = 255;
        }
    }
    // Dos pasadas de caja en cada sentido: lo pequeño, estirado, dibujaría
    // cuadros; suavizado, manchas.
    for _ in 0..2 {
        box_blur(&mut small, width, height, true);
        box_blur(&mut small, width, height, false);
    }
    Some(Frame { width: width as u32, height: height as u32, bgra: small })
}

/// Una pasada de caja de radio 2, en horizontal o en vertical. En los bordes
/// se repite el último píxel.
fn box_blur(pixels: &mut [u8], width: usize, height: usize, horizontal: bool) {
    const RADIUS: isize = 2;
    let source = pixels.to_vec();
    let (lines, length) = if horizontal { (height, width) } else { (width, height) };
    let at = |line: usize, step: usize| if horizontal { (line * width + step) * 4 } else { (step * width + line) * 4 };
    for line in 0..lines {
        for step in 0..length {
            let mut sum = [0u32; 3];
            for offset in -RADIUS..=RADIUS {
                let near = (step as isize + offset).clamp(0, length as isize - 1) as usize;
                for (channel, total) in sum.iter_mut().enumerate() {
                    *total += source[at(line, near) + channel] as u32;
                }
            }
            for (channel, total) in sum.iter().enumerate() {
                pixels[at(line, step) + channel] = (total / (2 * RADIUS as u32 + 1)) as u8;
            }
        }
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

#[cfg(test)]
mod tests {
    // Sin `super::*`: traería el `test` de gpui_kit en vez del de siempre.
    use super::{backdrop, video_fit, Frame};
    use gpui_kit::ObjectFit;

    /// Un vídeo se ve entero. Recortado para llenar, un móvil en vertical en
    /// el escenario a pantalla completa era sólo una nariz (19-nov).
    #[test]
    fn a_video_is_never_cropped() {
        assert!(matches!(video_fit(), ObjectFit::Contain));
    }

    fn solid(width: u32, height: u32, bgr: [u8; 3]) -> Frame {
        let pixel = [bgr[0], bgr[1], bgr[2], 255];
        Frame { width, height, bgra: pixel.repeat((width * height) as usize) }
    }

    /// El fondo es pequeño y con la forma del vídeo: un 720p vertical da
    /// uno vertical de lado largo `BACKDROP_SIDE`, y uno ya pequeño no crece.
    ///
    /// Roturas que atrapa: no reducirlo (estirar un 720p entero cuesta
    /// atlas y no desenfoca); perder la proporción (el fondo deformado).
    #[test]
    fn the_backdrop_is_small_and_keeps_the_shape() {
        let tall = backdrop(&solid(720, 1280, [10, 20, 30])).unwrap();
        assert_eq!((tall.width, tall.height), (27, 48));
        let tiny = backdrop(&solid(4, 2, [10, 20, 30])).unwrap();
        assert_eq!((tiny.width, tiny.height), (4, 2));
        assert_eq!(tall.bgra.len(), 27 * 48 * 4);
    }

    /// Un color liso sigue siendo ese color, opaco, y un punto brillante se
    /// reparte por sus vecinos: está desenfocado de verdad.
    ///
    /// Roturas que atrapa: cambiar el orden de los canales (el fondo de otro
    /// color que el vídeo); dejarlo transparente; no suavizarlo (cuadros
    /// estirados en vez de manchas).
    #[test]
    fn the_backdrop_keeps_the_colours_and_softens_them() {
        let plain = backdrop(&solid(96, 48, [10, 20, 30])).unwrap();
        assert!(plain.bgra.chunks(4).all(|pixel| pixel == [10, 20, 30, 255]), "{:?}", &plain.bgra[..8]);

        let mut dot = solid(48, 48, [0, 0, 0]);
        let middle = (24 * 48 + 24) * 4;
        dot.bgra[middle..middle + 3].copy_from_slice(&[255, 255, 255]);
        let soft = backdrop(&dot).unwrap();
        let at = |x: usize, y: usize| soft.bgra[(y * 48 + x) * 4];
        assert!(at(24, 24) < 255, "el punto se repartió");
        assert!(at(26, 24) > 0 && at(24, 26) > 0, "a sus vecinos, en los dos sentidos");
    }

    /// Un fotograma que no cuadra con su tamaño no da fondo (ni revienta).
    #[test]
    fn a_broken_frame_has_no_backdrop() {
        assert!(backdrop(&Frame { width: 4, height: 2, bgra: vec![0; 3] }).is_none());
        assert!(backdrop(&Frame { width: 0, height: 0, bgra: vec![] }).is_none());
    }
}
