//! Lo mismo que se le pide al prototipo, pedido a zorite-editor: que no
//! cambie lo que no se toca y que los acentos del teclado español entren.

use gpui_kit::test::{TestInput, TestWindowExt};
use gpui_kit::{px, size, AnyWindowHandle, App, AppContext as _, Bounds, ElementInputHandler, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
use zorite_editor::EditorState;
use zorite_probe::Probe;

const CORPUS: [(&str, &str); 4] = [
    ("runbook", include_str!("../../../corpus/runbook.md")),
    ("notes", include_str!("../../../corpus/notes.md")),
    ("crlf", include_str!("../../../corpus/crlf.md")),
    ("edges", include_str!("../../../corpus/edges.md")),
];

fn open(source: &str, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<EditorState>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        zorite_probe::install(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(900.), px(700.)) })),
            ..WindowOptions::default()
        };
        let (window, probe) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Probe::new(source, window, cx))).expect("open test window");
        (window, probe.read(cx).editor.clone())
    })
}

fn act(window: AnyWindowHandle, cx: &mut TestAppContext, act: impl FnOnce(&mut gpui_kit::Window, &mut App)) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        act(window, cx);
        window.render_frame(cx);
    })
    .unwrap();
}

/// Cargar, pintar, pasear el cursor y leer el texto: byte a byte el mismo.
#[gpui_kit::test]
fn loading_painting_and_moving_changes_nothing(cx: &mut TestAppContext) {
    for (name, source) in CORPUS {
        let (window, editor) = open(source, cx);
        act(window, cx, |window, cx| {
            for _ in 0..60 {
                window.press("down", cx);
                window.press("right", cx);
            }
        });
        let text = editor.read_with(cx, |editor, _| editor.text().to_string());
        assert_eq!(text, source, "{name}: cambió sin escribir nada");
    }
}

/// Escribir una letra cambia esa letra y nada más.
#[gpui_kit::test]
fn typing_changes_only_what_was_typed(cx: &mut TestAppContext) {
    for (name, source) in CORPUS {
        let (window, editor) = open(source, cx);
        let at = source.find("línea").or_else(|| source.find("ítulo")).unwrap();
        editor.update(cx, |editor, cx| editor.set_cursor(at, cx));
        act(window, cx, |window, cx| window.input("ñ¡", cx));
        let text = editor.read_with(cx, |editor, _| editor.text().to_string());
        assert_eq!(text, format!("{}ñ¡{}", &source[..at], &source[at..]), "{name}");
    }
}

/// La tecla muerta de X11 (`´` a medio componer, luego `á`) deja una `á`.
#[gpui_kit::test]
fn spanish_dead_keys_compose(cx: &mut TestAppContext) {
    let (window, editor) = open("ctón\n", cx);
    editor.update(cx, |editor, cx| editor.set_cursor(1, cx));
    let handle = editor.clone();
    act(window, cx, |window, cx| {
        let mut input = TestInput::new(ElementInputHandler::new(Bounds::default(), handle), window);
        input.compose_text(None, "´", None, cx);
        assert_eq!(input.marked_text_range(cx), Some(1..2));
    });
    assert_eq!(editor.read_with(cx, |editor, _| editor.text().to_string()), "c´tón\n");
    act(window, cx, |window, cx| window.input("á", cx));
    assert_eq!(editor.read_with(cx, |editor, _| editor.text().to_string()), "cátón\n");
}
