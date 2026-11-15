//! Lo que la prueba de viabilidad pone en pantalla.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{
    v_virtual_list, ActiveTheme, Icon, IconName, VirtualListScrollHandle,
};
use gpui_kit::*;

/// Cuántas filas tiene la lista de prueba. «Mi trabajo» pide 200 como mucho;
/// con mil se ve si el scroll aguanta de sobra.
pub const ROWS: usize = 1000;
const ROW_HEIGHT: f32 = 32.;

const MARKDOWN: &str = "\
## Markdown pintado

Un párrafo con **negrita**, *cursiva*, `código` y un [enlace](https://cac.guz-studio.dev).

- Una lista
- con acentos: revisión, diseño, año, ¿qué tal?

```rust
fn main() { println!(\"hola\"); }
```

| Lente | Pregunta |
|---|---|
| Asignadas | assignee=me |
| Siguiendo | watcher=me |
";

pub struct Spike {
    text: Entity<InputState>,
    notes: Entity<TextareaState>,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    /// Cuántas filas se construyeron en el último pintado. Es lo que dice si
    /// la lista es virtual de verdad: mil filas en datos, unas decenas en
    /// pantalla.
    rows_built: Rc<Cell<usize>>,
    /// La primera fila del último pintado: lo que dice si el scroll movió algo.
    first_row: Rc<Cell<usize>>,
    /// Sin esto la lista se pinta pero no se mueve: es lo que la rueda del
    /// ratón y la barra desplazan.
    scroll: VirtualListScrollHandle,
}

impl Spike {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            text: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Escribe: áéíóú ñ ü ¿? — con teclas muertas")
            }),
            notes: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("Varias líneas: pega texto, deshaz, selecciona")
            }),
            row_sizes: Rc::new(vec![size(px(0.), px(ROW_HEIGHT)); ROWS]),
            rows_built: Rc::new(Cell::new(0)),
            first_row: Rc::new(Cell::new(0)),
            scroll: VirtualListScrollHandle::new(),
        }
    }
}

/// El texto de una fila de la lista de prueba.
pub fn row_label(index: usize) -> String {
    format!("#{} · Tarea de prueba número {}", index + 1, index + 1)
}

fn section(title: &'static str, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(title))
}

impl Render for Spike {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, primary) =
            (theme.border, theme.muted_foreground, theme.list_hover, theme.primary);

        let (rows_built, first_row) = (self.rows_built.clone(), self.first_row.clone());
        let list = v_virtual_list(cx.entity(), "rows", self.row_sizes.clone(), move |_, range, _, _| {
            rows_built.set(range.len());
            first_row.set(range.start);
            range
                .map(|index| {
                    div()
                        .id(index)
                        .h(px(ROW_HEIGHT))
                        .w_full()
                        .px_3()
                        .flex()
                        .items_center()
                        .border_b_1()
                        .border_color(border)
                        .hover(move |style| style.bg(hover))
                        .child(row_label(index))
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.scroll);

        div()
            .size_full()
            .flex()
            .bg(theme.background)
            .text_color(theme.foreground)
            // Columna izquierda: la lista virtual.
            .child(
                div()
                    .w(px(360.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(border)
                    .bg(theme.sidebar)
                    .child(
                        div()
                            .p_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::Inbox).text_color(primary))
                            .child(format!("Lista virtual · {ROWS} filas")),
                    )
                    .child(
                        div()
                            .id("rows-viewport")
                            // Para que una prueba pueda apuntar aquí con la
                            // rueda; fuera de las pruebas no hace nada.
                            .test_support()
                            .relative()
                            .flex_1()
                            .overflow_hidden()
                            .child(list)
                            .child(Scrollbar::vertical(&self.scroll)),
                    ),
            )
            // Columna derecha: texto, markdown.
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child("cac · prueba de GPUI"))
                            .child(div().text_color(muted).child(
                                "Mira: que la ventana se pinte bien en bspwm, los acentos al escribir, \
                                 el scroll de la lista y el markdown.",
                            ))
                            // Que el core está enlazado y contesta: la consulta
                            // de la lente «asignadas» sale de `cac-core`.
                            .child(div().text_xs().text_color(muted).child(format!(
                                "El core contesta: {}",
                                cac_core::WorkLens::Assigned.query()
                            ))),
                    )
                    .child(section("CAMPO DE TEXTO", cx).child(Input::new(&self.text).id("spike-text")))
                    .child(section("VARIAS LÍNEAS", cx).child(Textarea::new(&self.notes).h(px(120.))))
                    .child(
                        section("MARKDOWN", cx)
                            .flex_1()
                            .overflow_hidden()
                            .child(TextView::markdown("markdown", MARKDOWN)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: arriba se importa `gpui_kit::*`, que trae una macro
    // `test`, y `#[gpui_kit::test]` se expandiría sobre sí misma sin fin.
    use super::{Spike, ROWS};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        point, px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, ScrollDelta,
        TestAppContext, WindowBounds, WindowOptions,
    };

    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Spike>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply(gpui_kit::component::ThemeMode::Dark, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(760.)),
                })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Spike::new(window, cx)))
                .expect("open test window")
        })
    }

    /// Lo que se teclea en el campo llega a su estado, con acentos.
    ///
    /// Con clic y teclado de verdad, sin ventana: es la prueba de que las
    /// vistas de la app se van a poder probar en el CI. No dice nada del método
    /// de entrada del sistema (las teclas muertas en X11): eso se mira a mano.
    /// Rotura que atrapa: que el campo no esté en pantalla o no reciba el foco.
    #[gpui_kit::test]
    fn typing_reaches_the_field(cx: &mut TestAppContext) {
        let (window, spike) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.click("spike-text", cx);
            window.input("revisión del diseño, año ¿qué?", cx);

            let text = spike.read(cx).text.read(cx).value();
            assert_eq!(text.as_ref(), "revisión del diseño, año ¿qué?");
        })
        .unwrap();
    }

    /// La lista tiene mil filas y construye sólo las que caben.
    ///
    /// Rotura que atrapa: construirlas todas (una lista que no es virtual, que
    /// con los datos de verdad sería una pantalla que tarda en abrir).
    #[gpui_kit::test]
    fn the_list_builds_only_what_fits(cx: &mut TestAppContext) {
        let (window, spike) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let built = spike.read(cx).rows_built.get();
            assert!(built > 0, "no se pintó ninguna fila");
            assert!(built < 100, "se construyeron {built} filas de {ROWS}");
        })
        .unwrap();
    }

    /// La rueda del ratón mueve la lista.
    ///
    /// jose abrió la primera versión y la lista no se movía: se pintaba, pero
    /// no tenía a qué engancharse el scroll (14-nov-2026). Rotura que atrapa:
    /// quitarle a la lista su manejador de scroll.
    #[gpui_kit::test]
    fn the_wheel_moves_the_list(cx: &mut TestAppContext) {
        let (window, spike) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(spike.read(cx).first_row.get(), 0);

            window.scroll("rows-viewport", ScrollDelta::Pixels(point(px(0.), px(-640.))), cx);
            window.render_frame(cx);

            let first = spike.read(cx).first_row.get();
            assert!(first >= 15, "tras bajar 640px la primera fila es la {first}");
        })
        .unwrap();
    }
}
