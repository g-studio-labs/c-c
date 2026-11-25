//! Que soltar un fichero llegue siempre, también tras teclear.
//!
//! GPUI apaga el «hover» mientras lo último que entró fue una tecla, y un
//! `FileDrop` no cuenta como movimiento del ratón: tras escribir, el destino
//! de lo soltado nunca está «bajo el ratón» y el `drop` no llega hasta que
//! se mueva. Al entrar un arrastre de fuera con el modo de teclado puesto, se
//! reenvía el mismo movimiento como uno de ratón, que devuelve el hover.

use gpui_kit::*;

/// Un elemento invisible que hay que poner una vez en la raíz de la ventana.
pub fn drop_wake() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |_, _, window, _| {
            window.on_mouse_event(|event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || event.pressed_button.is_none() || !cx.has_active_drag() || !window.last_input_was_keyboard() {
                    return;
                }
                let moved = event.clone();
                // Dentro del despacho de este mismo evento no se puede lanzar otro.
                window.defer(cx, move |window, cx| {
                    window.dispatch_event(PlatformInput::MouseMove(moved), cx);
                });
            });
        },
    )
    .absolute()
    .size_0()
}

#[cfg(test)]
mod tests {
    use super::drop_wake;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{div, px, size, AppContext, Bounds, Context, ExternalPaths, FileDropEvent, InteractiveElement, IntoElement, ParentElement, PlatformInput, Point, Render, StatefulInteractiveElement, Styled, TestAppContext, TestSupportExt, Window, WindowBounds, WindowOptions};
    use std::path::PathBuf;

    struct Target {
        wake: bool,
        got: Vec<PathBuf>,
    }

    impl Render for Target {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .children(self.wake.then(drop_wake))
                .child(
                    div()
                        .id("target")
                        .aria_label("target")
                        .size_full()
                        .on_drop(cx.listener(|this, paths: &ExternalPaths, _, _| this.got.extend(paths.paths().iter().cloned())))
                        .test_support(),
                )
        }
    }

    fn drop_after_typing(wake: bool, cx: &mut TestAppContext) -> Vec<PathBuf> {
        let (window, view) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(400.), px(300.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Target { wake, got: vec![] })).unwrap()
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("a", cx);
            let position = window.find("target").bounds().center();
            let paths = ExternalPaths(vec![PathBuf::from("/tmp/notes.txt")].into());
            // Sin ningún movimiento del ratón antes: es el caso del fallo.
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position, paths }), cx);
        })
        .unwrap();
        // El gestor de ficheros tarda en soltar: el reenvío tiene tiempo de correr.
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            let position = window.find("target").bounds().center();
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Submit { position }), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| view.read(cx).got.clone())
    }

    /// Soltar un fichero justo después de teclear, sin haber movido el
    /// ratón, llega al destino.
    ///
    /// Rotura que atrapa: quitar el reenvío del movimiento (el `drop`
    /// vuelve a perderse); la segunda mitad prueba que sin él se pierde de
    /// verdad, o sea que la prueba mide lo que dice.
    #[gpui_kit::test]
    fn a_file_dropped_right_after_typing_arrives(cx: &mut TestAppContext) {
        assert_eq!(drop_after_typing(true, cx), [PathBuf::from("/tmp/notes.txt")]);
    }

    #[gpui_kit::test]
    fn without_the_wake_the_drop_is_lost_after_typing(cx: &mut TestAppContext) {
        assert!(drop_after_typing(false, cx).is_empty(), "GPUI ya entrega el drop: sobra el reenvío");
    }
}
