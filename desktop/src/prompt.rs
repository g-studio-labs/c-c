//! La pregunta de confirmación, pintada por la app.
//!
//! En macOS y Windows `window.prompt` abre el diálogo del sistema. En Linux
//! no hay uno, y GPUI pinta el suyo de reserva: una caja blanca de 288 px
//! —también con el tema oscuro— cuyo detalle **no parte línea**, así que de
//! «Borra sus carpetas, sus listas y todas las tareas de dentro. No se puede
//! deshacer.» se leía hasta «todas las t» (medido el 17-nov-2026, al mirar
//! la confirmación de borrar un espacio). Una confirmación que no se puede
//! leer entera no confirma nada, así que donde el sistema no pone la suya,
//! la pone la app: con sus colores, y con el texto entero.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::theme::Leading as _;

/// El ancho de la caja: el `max-w-sm` de los diálogos de la app de siempre.
const WIDTH: f32 = 384.;

/// Una pregunta con sus respuestas, encima de la ventana.
pub struct Question {
    /// Lo que se va a hacer es destructivo: su botón va en rojo.
    warns: bool,
    title: String,
    detail: Option<String>,
    answers: Vec<String>,
    focus: FocusHandle,
}

impl Question {
    pub fn new(level: PromptLevel, title: &str, detail: Option<&str>, answers: Vec<String>, cx: &mut Context<Self>) -> Self {
        Self {
            warns: !matches!(level, PromptLevel::Info),
            title: title.to_string(),
            detail: detail.map(str::to_string),
            answers,
            focus: cx.focus_handle(),
        }
    }
}

impl EventEmitter<PromptResponse> for Question {}

impl Focusable for Question {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Question {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, popover, border, foreground, muted) = (theme.background, theme.popover, theme.border, theme.foreground, theme.muted_foreground);
        // Escape es la última respuesta: en las preguntas de esta app es
        // «Cancelar» (o «Cerrar», cuando no hay otra).
        let last = self.answers.len().saturating_sub(1);
        let warns = self.warns && self.answers.len() > 1;
        let answers = self.answers.iter().enumerate().map(|(index, answer)| {
            Button::new(SharedString::from(format!("prompt-answer-{index}")))
                .small()
                .label(answer.clone())
                // La primera es lo que se preguntó; las demás, no hacerlo.
                .map(|button| match (index, warns) {
                    (0, true) => button.danger(),
                    (0, false) => button.primary(),
                    _ => button.outline(),
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(PromptResponse(index));
                    cx.stop_propagation();
                }))
        });
        let question = div()
            .id("prompt")
            .test_support()
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded(px(10.))
            .border_1()
            .border_color(border)
            .bg(popover)
            .text_color(foreground)
            .shadow_lg()
            .child(div().id("prompt-title").test_support().w_full().leading_sm().font_weight(FontWeight::SEMIBOLD).child(self.title.clone()))
            // Cada línea en su bloque, a todo el ancho: así parte donde no cabe.
            .children(self.detail.clone().map(|detail| {
                div()
                    .id("prompt-detail")
                    .test_support()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .leading_xs()
                    .text_color(muted)
                    .children(detail.lines().map(|line| div().w_full().child(line.to_string())))
            }))
            .child(div().mt_2().flex().justify_end().gap_2().children(answers));
        div()
            .size_full()
            .track_focus(&self.focus)
            // Tapa lo de debajo: mientras se pregunta, no se pulsa nada más.
            .occlude()
            .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(PromptResponse(last));
                    cx.stop_propagation();
                }
            }))
            .flex()
            .items_center()
            .justify_center()
            .bg(background.opacity(0.7))
            .child(question)
    }
}

/// Desde ahora, donde el sistema no tiene diálogo propio, pregunta la app.
pub fn install(cx: &mut App) {
    cx.set_prompt_builder(|level, title, detail, answers, handle, window, cx| {
        let answers = answers.iter().map(|answer| answer.label().to_string()).collect();
        let question = cx.new(|cx| Question::new(level, title, detail, answers, cx));
        handle.with_view(question, window, cx)
    });
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::Question;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AppContext, Bounds, Focusable, Point, PromptLevel, PromptResponse, TestAppContext, WindowBounds, WindowOptions};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// La pregunta se lee entera —un detalle largo parte línea dentro de su
    /// caja—, cada botón contesta lo suyo y Escape contesta el último.
    ///
    /// Roturas que atrapa: un detalle que se sale de la caja y se corta (lo
    /// que hacía el diálogo de reserva de GPUI); perder una de sus líneas;
    /// un botón que contesta por otro; un Escape que confirma el borrado.
    #[gpui_kit::test]
    fn the_question_is_read_whole_and_each_answer_is_its_own(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply(gpui_kit::component::ThemeMode::Dark, cx);
        });
        let said: Rc<RefCell<Vec<usize>>> = Rc::default();
        let heard = said.clone();
        let detail = "Borra sus carpetas, sus listas y todas las tareas de dentro. No se puede deshacer.\n2 carpetas · 5 listas · 42 tareas";
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(900.), px(600.)) })),
                ..WindowOptions::default()
            };
            let (window, question) = gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|cx| Question::new(PromptLevel::Warning, "¿Borrar el espacio «Producto»?", Some(detail), vec!["Borrar el espacio".into(), "Cancelar".into()], cx))
            })
            .expect("open test window");
            cx.subscribe(&question, move |_, answer: &PromptResponse, _| heard.borrow_mut().push(answer.0)).detach();
            let focus = question.read(cx).focus_handle(cx);
            window.update(cx, |_, window, cx| window.focus(&focus, cx)).unwrap();
            window
        });
        let (boxed, detail_box, one_line) = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                (window.find("prompt").bounds(), window.find("prompt-detail").bounds(), window.find("prompt-title").bounds().size.height)
            })
            .unwrap();

        assert!(detail_box.size.width <= boxed.size.width, "el detalle se sale de su caja");
        assert!(detail_box.size.height > one_line * 2.5, "el detalle no partió línea, o perdió la de las cuentas");

        cx.update_window(window, |_, window, cx| {
            window.click("prompt-answer-1", cx);
            window.click("prompt-answer-0", cx);
            window.press("escape", cx);
        })
        .unwrap();
        assert_eq!(*said.borrow(), [1, 0, 1]);
    }
}
