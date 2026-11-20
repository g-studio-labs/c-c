//! Compartir una pantalla o una ventana (command-and-control-446).
//!
//! Con el capturador de escritorio de libwebrtc. En X11 se listan pantallas y
//! ventanas y se elige en la app; en Wayland la lista viene vacía y elige el
//! diálogo del sistema (el portal), sin probar todavía en este motor. Portado
//! de `start_screen`, `GlibContext` y `the_system_chooses` de
//! `spikes/calls-native/src/call.rs`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use livekit::webrtc::desktop_capturer::{CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions, DesktopFrame};
use livekit::webrtc::native::yuv_helper;
use livekit::webrtc::prelude::I420Buffer;
use serde::Serialize;
use tokio::runtime::Handle;

use crate::error::VoiceError;
use crate::publish::{self, Capture, Outlet, Publishing, Started};
use livekit::webrtc::video_source::native::NativeVideoSource;

/// Lo que se espera a que alguien conteste al diálogo del sistema: entre que
/// aparece, se busca la ventana y se acepta pasa más de lo que parece (Tauri
/// empezó con diez segundos).
pub(crate) const PORTAL_WAIT: Duration = Duration::from_secs(60);
/// A ritmo de vídeo y nunca en bucle cerrado: mientras el sistema pregunta,
/// el capturador contesta «todavía no» sin parar.
const FRAME_EVERY: Duration = Duration::from_millis(1000 / 30);

/// Una pantalla o una ventana que se puede compartir. `id` sólo vale para
/// `Call::share` en esta sesión.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScreenSource {
    pub id: String,
    pub title: String,
}

/// Lo que se puede compartir, en dos grupos. Vacío cuando elige el sistema.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ScreenSources {
    pub screens: Vec<ScreenSource>,
    pub windows: Vec<ScreenSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceKind {
    Screen,
    Window,
}

/// El id lleva el tipo delante: un capturador de pantallas no encuentra una
/// ventana, y el número solo no dice cuál abrir.
fn source_id(kind: SourceKind, id: u64) -> String {
    match kind {
        SourceKind::Screen => format!("screen:{id}"),
        SourceKind::Window => format!("window:{id}"),
    }
}

pub(crate) fn parse_source_id(id: &str) -> Option<(SourceKind, u64)> {
    let (kind, number) = id.split_once(':')?;
    let kind = match kind {
        "screen" => SourceKind::Screen,
        "window" => SourceKind::Window,
        _ => return None,
    };
    Some((kind, number.parse().ok()?))
}

/// Si elige el sistema (el portal de Wayland) y no la app. Ahí la lista trae
/// una entrada de relleno, y preguntar en la app sería preguntar dos veces.
fn the_system_chooses(session_type: Option<&str>, wayland_display: Option<&str>) -> bool {
    session_type == Some("wayland") || wayland_display.is_some_and(|d| !d.is_empty())
}

/// Las dos listas, como se enseñan. Una ventana sin título fuera: en X11 se
/// listan también las de ayuda de cada programa, y nadie sabría cuál es.
fn arrange(screens: Vec<(u64, String)>, windows: Vec<(u64, String)>) -> ScreenSources {
    let as_source = |kind| move |(id, title): (u64, String)| ScreenSource { id: source_id(kind, id), title };
    ScreenSources {
        screens: screens.into_iter().map(as_source(SourceKind::Screen)).collect(),
        windows: windows.into_iter().filter(|(_, title)| !title.trim().is_empty()).map(as_source(SourceKind::Window)).collect(),
    }
}

fn options(kind: SourceKind) -> DesktopCapturerOptions {
    let mut options = DesktopCapturerOptions::new(match kind {
        SourceKind::Screen => DesktopCaptureSourceType::Screen,
        SourceKind::Window => DesktopCaptureSourceType::Window,
    });
    options.set_include_cursor(true);
    options
}

fn list(kind: SourceKind) -> Vec<(u64, String)> {
    DesktopCapturer::new(options(kind)).map(|capturer| capturer.get_source_list().iter().map(|s| (s.id(), s.title())).collect()).unwrap_or_default()
}

/// Lo que se puede compartir ahora. Habla con el servidor gráfico: unos
/// milisegundos.
pub(crate) fn sources() -> ScreenSources {
    if the_system_chooses(std::env::var("XDG_SESSION_TYPE").ok().as_deref(), std::env::var("WAYLAND_DISPLAY").ok().as_deref()) {
        return ScreenSources::default();
    }
    arrange(list(SourceKind::Screen), list(SourceKind::Window))
}

/// Lo que dio una vuelta del capturador, ya sin libwebrtc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shot {
    Frame,
    /// Todavía no: el sistema pregunta, o la ventana está minimizada.
    NotYet,
    /// No va a dar más: denegada, o la ventana se cerró.
    Refused,
}

/// Lo que se decide en cada vuelta, separado del capturador para probarlo
/// sin pantalla.
struct Watch {
    seen: bool,
    deadline: Instant,
}

impl Watch {
    fn new(deadline: Instant) -> Self {
        Watch { seen: false, deadline }
    }

    /// `None` mientras haya que seguir. Un «no» antes del primer fotograma es
    /// el sistema denegándola; después, que la captura murió (la ventana se
    /// cerró): se dice, y quien publica la retira. En Tauri la interfaz no
    /// se enteraba y seguía diciendo «compartiendo».
    fn judge(&mut self, shot: Option<Shot>, now: Instant) -> Option<VoiceError> {
        match shot {
            Some(Shot::Frame) => {
                self.seen = true;
                None
            }
            Some(Shot::Refused) if self.seen => Some(VoiceError::ScreenFailed("la captura se cortó".into())),
            Some(Shot::Refused) => Some(VoiceError::ScreenDenied),
            Some(Shot::NotYet) | None if !self.seen && now > self.deadline => Some(VoiceError::ShareUnanswered),
            Some(Shot::NotYet) | None => None,
        }
    }
}

/// Quién comparte ahora, para que pedirlo dos veces seguidas no lance dos
/// diálogos del sistema. Genérico en lo que guarda para probarlo sin hilos.
pub(crate) struct ShareSlot<H> {
    next: u64,
    current: Option<ShareEntry<H>>,
}

struct ShareEntry<H> {
    ticket: u64,
    live: bool,
    handle: Option<H>,
}

impl<H> Default for ShareSlot<H> {
    fn default() -> Self {
        ShareSlot { next: 0, current: None }
    }
}

impl<H> ShareSlot<H> {
    /// Empieza a pedir una fuente. Con otra todavía arrancando, no: el
    /// primer diálogo sigue abierto. Con otra ya compartiéndose, la suelta
    /// (cambiar de fuente).
    pub(crate) fn begin(&mut self) -> Result<u64, VoiceError> {
        if self.current.as_ref().is_some_and(|entry| !entry.live) {
            return Err(VoiceError::SharePending);
        }
        self.next += 1;
        self.current = Some(ShareEntry { ticket: self.next, live: false, handle: None });
        Ok(self.next)
    }

    /// Guarda la captura de ese turno. Si mientras tanto se paró, se la
    /// devuelve para que quien llama la suelte.
    pub(crate) fn attach(&mut self, ticket: u64, handle: H) -> Result<(), H> {
        match &mut self.current {
            Some(entry) if entry.ticket == ticket => {
                entry.handle = Some(handle);
                Ok(())
            }
            _ => Err(handle),
        }
    }

    /// Ese turno ya se ve, o falló (y se suelta).
    pub(crate) fn settle(&mut self, ticket: u64, live: bool) {
        if self.current.as_ref().is_some_and(|entry| entry.ticket == ticket) {
            if live {
                self.current.as_mut().unwrap().live = true;
            } else {
                self.current = None;
            }
        }
    }

    pub(crate) fn stop(&mut self) {
        self.current = None;
    }
}

/// Comparte la fuente pedida (`None`: la que elija el sistema) en su hilo.
pub(crate) fn start(handle: Handle, requested: Option<(SourceKind, u64)>, publishing: Publishing) -> Result<(Capture, Started), VoiceError> {
    publish::spawn_capture("pantalla", handle, publishing, move |outlet, stop| capture(requested, outlet, stop))
}

fn capture(requested: Option<(SourceKind, u64)>, mut outlet: Outlet<NativeVideoSource>, stop: &AtomicBool) {
    // Antes de crear el capturador: GIO se queda con el contexto que esté
    // puesto en el momento de la llamada.
    let glib = GlibContext::fresh_one();
    let kind = requested.map_or(SourceKind::Screen, |(kind, _)| kind);
    let Some(mut capturer) = DesktopCapturer::new(options(kind)) else { return outlet.fail(VoiceError::NoScreenCapture) };
    let source = match requested {
        None => None,
        // Nunca otra en su lugar: compartir una ventana que no elegiste es
        // justo lo que no puede pasar.
        Some((_, id)) => match capturer.get_source_list().into_iter().find(|s| s.id() == id) {
            Some(source) => Some(source),
            None => return outlet.fail(VoiceError::ScreenFailed("esa pantalla o ventana ya no está".into())),
        },
    };
    let (sent, received) = std::sync::mpsc::channel();
    capturer.start_capture(source, move |result| {
        let _ = sent.send(match result {
            Ok(frame) => to_i420(&frame).ok_or(Shot::NotYet),
            Err(CaptureError::Temporary) => Err(Shot::NotYet),
            Err(CaptureError::Permanent) => Err(Shot::Refused),
        });
    });
    let mut watch = Watch::new(Instant::now() + PORTAL_WAIT);
    let mut next = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        glib.pump();
        capturer.capture_frame();
        let shot = match received.try_iter().last() {
            Some(Ok(buffer)) => {
                if !outlet.deliver(&buffer) {
                    return;
                }
                Some(Shot::Frame)
            }
            Some(Err(shot)) => Some(shot),
            None => None,
        };
        if let Some(error) = watch.judge(shot, Instant::now()) {
            log::warn!("pantalla: {error}");
            return outlet.fail(error);
        }
        next += FRAME_EVERY;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}

/// BGRA del capturador → I420 con libyuv («ARGB» en libyuv es B, G, R, A en
/// memoria), con el `stride` de verdad. `None` si no cabe en lo que dice
/// medir.
fn to_i420(frame: &DesktopFrame) -> Option<I420Buffer> {
    let (width, height, stride) = (frame.width(), frame.height(), frame.stride());
    let data = frame.data();
    if width <= 0 || height <= 0 || (stride as usize) < width as usize * 4 || data.len() < stride as usize * height as usize {
        return None;
    }
    let mut buffer = I420Buffer::new(width as u32, height as u32);
    let (stride_y, stride_u, stride_v) = buffer.strides();
    let (y, u, v) = buffer.data_mut();
    yuv_helper::argb_to_i420(data, stride, y, stride_y, u, stride_u, v, stride_v, width, height);
    Some(buffer)
}

/// Un contexto de GLib **propio** para el hilo que habla con el portal: GIO
/// entrega sus respuestas al contexto por defecto del hilo que llamó, y sólo
/// si alguien lo itera. Sin él, el diálogo salía y la captura contestaba
/// «todavía no» para siempre. Y no la característica `glib-main-loop` de
/// LiveKit, que itera el contexto global desde un hilo suelto (ver
/// `app/src-tauri/Cargo.toml`). En X11 no hace nada, pero no estorba.
struct GlibContext(*mut std::ffi::c_void);

extern "C" {
    fn g_main_context_new() -> *mut std::ffi::c_void;
    fn g_main_context_push_thread_default(context: *mut std::ffi::c_void);
    fn g_main_context_pop_thread_default(context: *mut std::ffi::c_void);
    fn g_main_context_iteration(context: *mut std::ffi::c_void, may_block: i32) -> i32;
    fn g_main_context_unref(context: *mut std::ffi::c_void);
}

impl GlibContext {
    fn fresh_one() -> Self {
        // SAFETY: un contexto recién creado, empujado en este hilo y soltado
        // en `drop` en el mismo hilo.
        unsafe {
            let context = g_main_context_new();
            g_main_context_push_thread_default(context);
            Self(context)
        }
    }

    /// Despacha lo pendiente sin bloquear, con tope.
    fn pump(&self) {
        // SAFETY: el contexto es de este hilo y sigue vivo.
        unsafe {
            for _ in 0..64 {
                if g_main_context_iteration(self.0, 0) == 0 {
                    break;
                }
            }
        }
    }
}

impl Drop for GlibContext {
    fn drop(&mut self) {
        // SAFETY: el mismo hilo que lo empujó en `fresh_one`.
        unsafe {
            g_main_context_pop_thread_default(self.0);
            g_main_context_unref(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(list: &[(u64, &str)]) -> Vec<(u64, String)> {
        list.iter().map(|(id, title)| (*id, title.to_string())).collect()
    }

    /// Pantallas y ventanas van cada una en su grupo, con un id que dice cuál
    /// es; las ventanas sin título no salen.
    ///
    /// Roturas que atrapa: mezclar los dos grupos; un id sin el tipo (una
    /// ventana se buscaría entre las pantallas); enseñar las ventanas de
    /// ayuda sin nombre.
    #[test]
    fn screens_and_windows_are_listed_apart() {
        let listed = arrange(entries(&[(0, "")]), entries(&[(41, "Firefox"), (42, " "), (43, "Terminal")]));
        assert_eq!(listed.screens, [ScreenSource { id: "screen:0".into(), title: String::new() }]);
        let windows: Vec<(&str, &str)> = listed.windows.iter().map(|w| (w.id.as_str(), w.title.as_str())).collect();
        assert_eq!(windows, [("window:41", "Firefox"), ("window:43", "Terminal")]);
        assert_eq!(parse_source_id("window:41"), Some((SourceKind::Window, 41)));
        assert_eq!(parse_source_id("screen:0"), Some((SourceKind::Screen, 0)));
        assert_eq!(parse_source_id("41"), None);
        assert_eq!(parse_source_id("tab:1"), None);
    }

    /// En Wayland elige el sistema.
    /// Rotura que atrapa: preguntar en la app y luego otra vez en el portal.
    #[test]
    fn on_wayland_the_system_chooses() {
        assert!(the_system_chooses(Some("wayland"), None));
        assert!(the_system_chooses(Some("tty"), Some("wayland-0")));
        assert!(!the_system_chooses(Some("x11"), Some("")));
        assert!(!the_system_chooses(None, None));
    }

    /// Un «no» antes del primer fotograma es una denegación; después, que la
    /// captura murió, y se dice. Sin respuesta al minuto, nadie contestó;
    /// pero con algo ya compartido, un «todavía no» (ventana minimizada) no
    /// corta nada.
    ///
    /// Roturas que atrapa: seguir «compartiendo» una ventana cerrada;
    /// llamarlo denegación cuando ya se veía; esperar para siempre al
    /// diálogo; cortar una ventana minimizada al minuto.
    #[test]
    fn what_each_turn_of_the_capturer_means() {
        let start = Instant::now();
        let late = start + PORTAL_WAIT + Duration::from_secs(1);
        let mut watch = Watch::new(start + PORTAL_WAIT);
        assert_eq!(watch.judge(Some(Shot::NotYet), start), None);
        assert_eq!(watch.judge(None, late), Some(VoiceError::ShareUnanswered));
        assert_eq!(Watch::new(start + PORTAL_WAIT).judge(Some(Shot::Refused), start), Some(VoiceError::ScreenDenied));

        let mut sharing = Watch::new(start + PORTAL_WAIT);
        assert_eq!(sharing.judge(Some(Shot::Frame), start), None);
        assert_eq!(sharing.judge(Some(Shot::NotYet), late), None, "minimizada no es muerta");
        assert_eq!(sharing.judge(None, late), None);
        assert_eq!(sharing.judge(Some(Shot::Refused), late).map(|e| e.label()), Some("voice-screen-thread-died"));
    }

    /// Pedir dos veces seguidas no lanza dos diálogos; parar mientras se
    /// pide suelta lo que llegue tarde; un fallo deja pedir otra vez; y con
    /// una ya compartiéndose se puede cambiar de fuente.
    ///
    /// Roturas que atrapa: dos diálogos del sistema a la vez; quedarse con
    /// una captura que llegó después de parar (o colarla en el turno
    /// siguiente); un fallo que bloquea compartir
    /// para siempre; no poder cambiar de ventana.
    #[test]
    fn sharing_twice_in_a_row_is_pending() {
        let mut slot: ShareSlot<&str> = ShareSlot::default();
        let first = slot.begin().unwrap();
        assert_eq!(slot.begin(), Err(VoiceError::SharePending));

        slot.stop();
        assert_eq!(slot.attach(first, "tarde"), Err("tarde"), "se paró mientras se pedía");
        let second = slot.begin().unwrap();
        assert_eq!(slot.attach(first, "tarde"), Err("tarde"), "ni en el turno siguiente");
        slot.attach(second, "pantalla").unwrap();
        slot.settle(second, false);
        let third = slot.begin().expect("un fallo deja pedir otra vez");
        slot.attach(third, "ventana").unwrap();
        slot.settle(first, false);
        assert_eq!(slot.begin(), Err(VoiceError::SharePending), "un turno viejo no suelta el de ahora");
        slot.settle(third, true);
        assert!(slot.begin().is_ok(), "compartiendo, se puede cambiar de fuente");
    }
}
