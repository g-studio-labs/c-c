//! La prueba de viabilidad de cac de escritorio en GPUI (Fase 0).
//!
//! No es la app: es una ventana con lo mínimo para **medir** si GPUI Kit sirve
//! en las máquinas del equipo antes de construir nada encima. Lo que hay que
//! mirar está escrito en la propia ventana, y los resultados van a la tarea
//! command-and-control-374.

mod spike;

use gpui_kit::*;

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        spike::apply_cac_theme(cx);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1100.), px(760.)),
                cx,
            ))),
            ..WindowOptions::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| spike::Spike::new(window, cx)))
            .expect("no se pudo abrir la ventana");
    });
}
