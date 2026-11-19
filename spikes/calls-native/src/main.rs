//! La prueba de llamadas en la app nativa (command-and-control-419).
//!
//! `calls-native-spike` sin argumentos es el paso 0; `paint` es el paso 1
//! (`paint.rs`); `call` es el paso 2, una llamada de verdad como invitado
//! (`call.rs`).
//!
//! Paso 0: ¿compila y enlaza `livekit` 0.12 dentro de un binario con GPUI?
//!
//! Abre una ventana GPUI y, en un runtime de tokio propio (como el de
//! `desktop/src/backend.rs`), toca las piezas de libwebrtc que la llamada va a
//! usar —una fuente de vídeo, el procesador de audio, una conversión de
//! fotograma— para que el enlazador tenga que resolverlas de verdad. No se
//! conecta a ningún sitio.

mod call;
mod paint;

use std::sync::Arc;

use gpui_kit::*;
use livekit::webrtc::native::apm::AudioProcessingModule;
use livekit::webrtc::prelude::*;
use livekit::webrtc::video_frame::native::VideoFrameBufferExt as _;
use livekit::webrtc::video_source::native::NativeVideoSource;

/// Lo que el arranque comprobó, para pintarlo.
struct Probe {
    said: Vec<String>,
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .bg(rgb(0x111111))
            .text_color(rgb(0xeeeeee))
            .children(self.said.iter().map(|line| div().child(line.clone())))
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("call") {
        call::run();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("paint") {
        let setup = paint::Setup::from_env();
        application().run(move |cx| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1280.), px(720.)), cx))),
                ..WindowOptions::default()
            };
            open_window(options, cx, |_, cx| cx.new(|cx| paint::Wall::new(setup, cx))).expect("ventana");
        });
        return;
    }
    let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime"));
    let said = runtime.block_on(async {
        let mut said = Vec::new();
        // Una fuente de vídeo: crearla lanza tareas de tokio por dentro.
        let source = NativeVideoSource::new(VideoResolution { width: 1280, height: 720 }, false);
        said.push(format!("fuente de vídeo: {:?}", source.video_resolution()));
        // El procesador de audio: eco, ganancia, paso alto y ruido.
        let _apm = AudioProcessingModule::new(true, false, true, true);
        said.push("procesador de audio: creado".to_string());
        // Un fotograma I420 negro convertido al formato que pinta GPUI.
        let buffer = I420Buffer::new(64, 64);
        let mut argb = vec![0u8; 64 * 64 * 4];
        buffer.to_argb(VideoFormatType::ARGB, &mut argb, 64 * 4, 64, 64);
        said.push(format!("conversión I420 → ARGB: {} bytes", argb.len()));
        said
    });
    for line in &said {
        println!("{line}");
    }
    if std::env::var_os("CALLS_PROBE_NO_WINDOW").is_some() {
        return;
    }
    application().run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(520.), px(240.)), cx))),
            ..WindowOptions::default()
        };
        open_window(options, cx, |_, cx| cx.new(|_| Probe { said: said.clone() })).expect("ventana");
        let _keep = runtime.clone();
    });
}
