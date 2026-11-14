//! Lo que la prueba de viabilidad pone en pantalla.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{v_virtual_list, ActiveTheme, Icon, IconName, Theme, ThemeMode};
use gpui_kit::*;

/// Cuántas filas tiene la lista de prueba. «Mi trabajo» pide 200 como mucho;
/// con mil se ve si el scroll aguanta de sobra.
pub const ROWS: usize = 1000;
const ROW_HEIGHT: f32 = 32.;

/// Los colores de cac, tomados del tema oscuro de `app/src/index.css` (`.dark`).
///
/// Sólo los que esta ventana enseña: el tema entero, claro y oscuro, es de la
/// Fase 2. Aquí se comprueba que el tema de GPUI Kit se deja vestir de cac.
pub fn apply_cac_theme(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    let colors = &mut theme.colors;
    colors.background = rgb(0x101013).into();
    colors.foreground = rgb(0xECECEE).into();
    colors.primary = rgb(0x20D9E8).into();
    colors.primary_foreground = rgb(0x0B0B0E).into();
    colors.muted = rgb(0x131317).into();
    colors.muted_foreground = rgb(0x9A9AA4).into();
    colors.accent = rgb(0x212128).into();
    colors.accent_foreground = rgb(0xECECEE).into();
    colors.border = rgb(0x2B2B33).into();
    colors.input = rgb(0x2B2B33).into();
    colors.ring = rgb(0x20D9E8).into();
    colors.sidebar = rgb(0x0A0A0D).into();
    colors.sidebar_border = rgb(0x23232A).into();
    colors.list_hover = rgb(0x212128).into();
    // La base de cac son 14px, no los 16 de GPUI Kit.
    theme.font_size = px(14.);
    theme.radius = px(8.);
}

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

        let rows_built = self.rows_built.clone();
        let list = v_virtual_list(cx.entity(), "rows", self.row_sizes.clone(), move |_, range, _, _| {
            rows_built.set(range.len());
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
        });

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
                    .child(div().flex_1().overflow_hidden().child(list)),
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
    use super::{apply_cac_theme, Spike, ROWS};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds,
        WindowOptions,
    };

    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Spike>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply_cac_theme(cx);
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
}
