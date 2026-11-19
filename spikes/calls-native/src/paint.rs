//! Paso 1: pintar vídeo en GPUI, sin servidor (command-and-control-421).
//!
//! La incógnita de la prueba. En Linux GPUI 0.3.12 sólo tiene un camino para
//! pintar píxeles que cambian: `img(ImageSource::Render(Arc<RenderImage>))`,
//! que sube cada fotograma al atlas de la gráfica. El atlas no olvida nada solo:
//! el fotograma anterior se tira a mano con `window.drop_image`, o la memoria
//! de la gráfica crece sin fin. Es lo que hace Zed fuera de macOS
//! (`crates/livekit_client/src/remote_video_track_view.rs`).
//!
//! Aquí cada «pista» es un hilo que hace de red: genera un fotograma I420 al
//! ritmo pedido —como los que entrega `NativeVideoStream`—, lo convierte con
//! `to_argb` de libwebrtc (el mismo libyuv que usará la llamada) y deja sólo el
//! último en su casilla. La vista lo recoge, tira el anterior y lo pinta.
//!
//! Lo que se mide, una línea por segundo y en un CSV:
//! - fotogramas pintados por segundo de cada pista, y los que se perdieron
//!   porque llegó otro antes de pintar el anterior;
//! - cuánto tarda generar y convertir uno;
//! - CPU del proceso (en núcleos) y memoria residente: con lavapipe la memoria
//!   «de la gráfica» es RAM, así que una fuga se ve aquí.
//!
//! Variables: `CALLS_TRACKS` (1), `CALLS_W`×`CALLS_H` (1280×720), `CALLS_FPS`
//! (30), `CALLS_SECONDS` (30; luego se cierra), `CALLS_CSV` (fichero),
//! `CALLS_LEAK=1` (no tirar el fotograma anterior: para ver la fuga).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use livekit::webrtc::prelude::*;

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

/// Lo que se pide medir.
#[derive(Clone, Copy, Debug)]
pub struct Setup {
    pub tracks: usize,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub seconds: u64,
    pub leak: bool,
    pub spin: bool,
}

impl Setup {
    pub fn from_env() -> Self {
        Setup {
            tracks: env_or("CALLS_TRACKS", 1),
            width: env_or("CALLS_W", 1280),
            height: env_or("CALLS_H", 720),
            fps: env_or("CALLS_FPS", 30),
            seconds: env_or("CALLS_SECONDS", 30),
            leak: std::env::var_os("CALLS_LEAK").is_some(),
            spin: std::env::var_os("CALLS_SPIN").is_some(),
        }
    }
}

/// La casilla de una pista: el último fotograma listo para pintar y sus
/// cuentas. La escribe el hilo de la pista, la vacía la vista.
///
/// La usa también la llamada (`call.rs`): ahí la escribe la tarea que recibe
/// la pista y `made` cuenta los fotogramas que llegaron.
#[derive(Default)]
pub struct Slot {
    ready: Mutex<Option<Arc<RenderImage>>>,
    pub made: AtomicU64,
    pub lost: AtomicU64,
    pub gen_us: AtomicU64,
    pub conv_us: AtomicU64,
}

impl Slot {
    /// Deja un fotograma listo, en lugar del que hubiera. Si había uno, no
    /// llegó a pintarse y se cuenta como perdido; como nunca subió al atlas,
    /// tirarlo así no fuga nada.
    pub fn offer(&self, image: Arc<RenderImage>) {
        self.made.fetch_add(1, Ordering::Relaxed);
        if self.ready.lock().unwrap().replace(image).is_some() {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// El fotograma pendiente, si lo hay. Lo llama la vista al pintar.
    pub fn take(&self) -> Option<Arc<RenderImage>> {
        self.ready.lock().unwrap().take()
    }
}

/// Un fotograma I420 de prueba: barras de color, una franja roja y una azul
/// (para ver que los canales no se cruzan) y una barra blanca que se mueve
/// (para ver que avanza).
pub fn test_frame(width: u32, height: u32, n: u64) -> I420Buffer {
    let mut buffer = I420Buffer::new(width, height);
    let (stride_y, stride_u, stride_v) = buffer.strides();
    let (y, u, v) = buffer.data_mut();
    // (Y, U, V) en BT.601: gris, rojo, verde, azul, blanco, negro.
    const BARS: [(u8, u8, u8); 6] = [(126, 128, 128), (82, 90, 240), (145, 54, 34), (41, 240, 110), (235, 128, 128), (16, 128, 128)];
    let moving = (n * 8 % width as u64) as u32;
    for row in 0..height {
        for col in 0..width {
            let bar = if row < height / 8 {
                // La franja de arriba: mitad roja, mitad azul.
                if col < width / 2 { BARS[1] } else { BARS[3] }
            } else if col.abs_diff(moving) < 6 {
                BARS[4]
            } else {
                BARS[(col * 6 / width) as usize]
            };
            y[(row * stride_y + col) as usize] = bar.0;
            if row % 2 == 0 && col % 2 == 0 {
                let chroma = |stride: u32| ((row / 2) * stride + col / 2) as usize;
                u[chroma(stride_u)] = bar.1;
                v[chroma(stride_v)] = bar.2;
            }
        }
    }
    buffer
}

/// I420 a la imagen BGRA que pinta GPUI, por libyuv (`I420ToARGB`, que en
/// memoria deja B, G, R, A).
///
/// Acepta cualquier búfer de libwebrtc y no sólo `I420Buffer`: lo que entrega
/// `NativeVideoStream` es un `Box<dyn VideoBuffer>` y así se convierte directo,
/// sin pasar antes por una copia en I420.
pub fn to_render_image(buffer: &dyn VideoBuffer) -> Arc<RenderImage> {
    let (width, height) = (buffer.width(), buffer.height());
    let mut bgra = vec![0u8; (width * height * 4) as usize];
    buffer.to_argb(VideoFormatType::ARGB, &mut bgra, width * 4, width as i32, height as i32);
    let pixels = image::RgbaImage::from_raw(width, height, bgra).expect("tamaño del fotograma");
    Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
}

/// El hilo de una pista: genera, convierte y deja el último en su casilla, al
/// ritmo pedido.
fn run_track(index: usize, setup: Setup, slot: Arc<Slot>, wake: UnboundedSender<usize>) {
    let period = Duration::from_secs_f64(1.0 / setup.fps as f64);
    // Treinta fotogramas hechos al empezar y repetidos: inventar la imagen no
    // es parte de una llamada (llega ya decodificada), y aquí sólo se quiere
    // medir lo que sí: convertir, subir y pintar.
    let reel: Vec<I420Buffer> = (0..30).map(|n| test_frame(setup.width, setup.height, n + index as u64 * 40)).collect();
    let mut next = Instant::now();
    let mut n = 0u64;
    loop {
        let started = Instant::now();
        let buffer = &reel[(n % 30) as usize];
        let generated = Instant::now();
        let image = to_render_image(buffer);
        let converted = Instant::now();
        slot.gen_us.fetch_add((generated - started).as_micros() as u64, Ordering::Relaxed);
        slot.conv_us.fetch_add((converted - generated).as_micros() as u64, Ordering::Relaxed);
        slot.offer(image);
        if wake.unbounded_send(index).is_err() {
            return;
        }
        n += 1;
        next += period;
        match next.checked_duration_since(Instant::now()) {
            Some(wait) => std::thread::sleep(wait),
            // Va con retraso: no se intenta recuperar, se sigue al ritmo.
            None => next = Instant::now(),
        }
    }
}

/// La vista: una rejilla con lo último de cada pista.
pub struct Wall {
    setup: Setup,
    slots: Vec<Arc<Slot>>,
    shown: Vec<Option<Arc<RenderImage>>>,
    painted: Vec<u64>,
    leaked: Vec<Arc<RenderImage>>,
}

impl Wall {
    pub fn new(setup: Setup, cx: &mut Context<Self>) -> Self {
        let (wake, mut woken) = unbounded::<usize>();
        let slots: Vec<Arc<Slot>> = (0..setup.tracks).map(|_| Arc::new(Slot::default())).collect();
        for (index, slot) in slots.iter().enumerate() {
            let (slot, wake) = (slot.clone(), wake.clone());
            std::thread::Builder::new().name(format!("pista-{index}")).spawn(move || run_track(index, setup, slot, wake)).expect("hilo de pista");
        }
        // Lo que llega despierta a la vista; varios avisos seguidos son un
        // solo repintado.
        cx.spawn(async move |this, cx| {
            while woken.next().await.is_some() {
                while woken.try_recv().is_ok() {}
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        })
        .detach();
        let painted = vec![0; setup.tracks];
        report(setup, slots.clone(), cx);
        Wall { setup, shown: vec![None; setup.tracks], slots, painted, leaked: Vec::new() }
    }
}

impl Render for Wall {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(fresh) = slot.take() else { continue };
            if let Some(old) = self.shown[index].replace(fresh) {
                if self.setup.leak {
                    self.leaked.push(old);
                } else {
                    // El que se pintó la última vez ya no se va a pintar.
                    let _ = window.drop_image(old);
                }
            }
            self.painted[index] += 1;
            PAINTED.fetch_add(1, Ordering::Relaxed);
        }
        RENDERS.fetch_add(1, Ordering::Relaxed);
        if self.setup.spin {
            // Pintar sin parar, haya fotograma nuevo o no: mide cuántas
            // veces por segundo puede presentar esta pantalla.
            window.request_animation_frame();
        }
        let columns = (self.setup.tracks.max(1) as f32).sqrt().ceil() as usize;
        div()
            .size_full()
            .bg(rgb(0x000000))
            .flex()
            .flex_wrap()
            .children(self.shown.iter().map(|image| {
                let tile = div().w(relative(1.0 / columns as f32)).h(relative(1.0 / columns as f32)).p_0p5();
                match image {
                    Some(image) => tile.child(img(ImageSource::Render(image.clone())).size_full().object_fit(ObjectFit::Contain)),
                    None => tile,
                }
            }))
    }
}

/// Fotogramas pintados, de todas las pistas.
static PAINTED: AtomicU64 = AtomicU64::new(0);
/// Veces que se pintó la ventana, con o sin vídeo nuevo.
static RENDERS: AtomicU64 = AtomicU64::new(0);

/// CPU usada por el proceso hasta ahora, en segundos (utime + stime).
pub fn cpu_seconds() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // Tras el nombre entre paréntesis: el campo 14 y el 15 son utime y stime.
    let rest = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or_default();
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let ticks: f64 = fields.get(11).and_then(|v| v.parse().ok()).unwrap_or(0.0) + fields.get(12).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    ticks / 100.0
}

/// Memoria residente, en MB.
pub fn rss_mb() -> f64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: f64 = statm.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    pages * 4096.0 / 1_048_576.0
}

/// Una línea por segundo, y al final un resumen; luego cierra.
fn report(setup: Setup, slots: Vec<Arc<Slot>>, cx: &mut Context<Wall>) {
    let csv_path = std::env::var("CALLS_CSV").ok();
    cx.spawn(async move |_, cx| {
        let mut csv = csv_path.map(|path| std::fs::File::create(path).expect("csv"));
        if let Some(csv) = csv.as_mut() {
            use std::io::Write;
            let _ = writeln!(csv, "segundo,pintados_por_s,hechos_por_s,perdidos_por_s,generar_ms,convertir_ms,cpu_nucleos,rss_mb,ventana_por_s");
        }
        let (mut last_cpu, mut last_painted, mut last_made, mut last_lost, mut last_gen, mut last_conv) = (cpu_seconds(), 0u64, 0u64, 0u64, 0u64, 0u64);
        let mut last_renders = 0u64;
        let mut rows: Vec<[f64; 8]> = Vec::new();
        for second in 1..=setup.seconds {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let sum = |pick: fn(&Slot) -> &AtomicU64| slots.iter().map(|slot| pick(slot).load(Ordering::Relaxed)).sum::<u64>();
            let (made, lost, gen, conv) = (sum(|s| &s.made), sum(|s| &s.lost), sum(|s| &s.gen_us), sum(|s| &s.conv_us));
            let painted = PAINTED.load(Ordering::Relaxed);
            let renders = RENDERS.load(Ordering::Relaxed);
            let cpu = cpu_seconds();
            let made_now = (made - last_made).max(1);
            let row = [
                (painted - last_painted) as f64 / setup.tracks as f64,
                (made - last_made) as f64 / setup.tracks as f64,
                (lost - last_lost) as f64 / setup.tracks as f64,
                (gen - last_gen) as f64 / made_now as f64 / 1000.0,
                (conv - last_conv) as f64 / made_now as f64 / 1000.0,
                cpu - last_cpu,
                rss_mb(),
                (renders - last_renders) as f64,
            ];
            last_renders = renders;
            println!(
                "t={second:>3}s  pintados/s por pista={:5.1}  hechos/s={:5.1}  perdidos/s={:5.1}  generar={:5.2}ms  convertir={:5.2}ms  cpu={:4.2} núcleos  rss={:7.1}MB  ventana/s={:5.1}",
                row[0], row[1], row[2], row[3], row[4], row[5], row[6], row[7]
            );
            if let Some(csv) = csv.as_mut() {
                use std::io::Write;
                let _ = writeln!(csv, "{second},{:.1},{:.1},{:.1},{:.3},{:.3},{:.3},{:.1},{:.1}", row[0], row[1], row[2], row[3], row[4], row[5], row[6], row[7]);
            }
            rows.push(row);
            (last_cpu, last_painted, last_made, last_lost, last_gen, last_conv) = (cpu, painted, made, lost, gen, conv);
        }
        // El resumen, sin los 3 primeros segundos (arranque).
        let steady: Vec<&[f64; 8]> = rows.iter().skip(3).collect();
        let mean = |i: usize| steady.iter().map(|row| row[i]).sum::<f64>() / steady.len().max(1) as f64;
        let rss_first = steady.first().map(|row| row[6]).unwrap_or(0.0);
        let rss_last = steady.last().map(|row| row[6]).unwrap_or(0.0);
        println!(
            "RESUMEN {}x{}@{} pistas={} fuga={}: pintados/s={:.1} perdidos/s={:.1} generar={:.2}ms convertir={:.2}ms cpu={:.2} núcleos rss {:.0}→{:.0}MB ({:+.1}MB/min) ventana/s={:.1}",
            setup.width,
            setup.height,
            setup.fps,
            setup.tracks,
            setup.leak,
            mean(0),
            mean(2),
            mean(3),
            mean(4),
            mean(5),
            rss_first,
            rss_last,
            (rss_last - rss_first) / (steady.len().max(1) as f64 / 60.0),
            mean(7),
        );
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}
