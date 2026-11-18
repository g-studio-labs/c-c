//! La pregunta antes de perder o pisar algo, sobre toda la ventana.
//!
//! Es el diálogo de `ConfirmDialog.tsx`, con las medidas de la pregunta de
//! una tarea (`task.rs`). La usan la documentación (salir de lo que se
//! escribe, guardar encima de otro) y la ventana al cerrarse. Qué se
//! pregunta lo dice el core (`Question`); aquí sólo se pinta, siempre igual:
//!
//! - el botón que pierde algo va en rojo;
//! - Escape, la equis y el otro botón dejan todo como estaba;
//! - **Enter no contesta**: lo que se pregunta aquí no se deshace;
//! - mientras se pregunta, ni las teclas ni el ratón llegan a lo de debajo.

use std::rc::Rc;

use cac_core::task::Question;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::*;

use crate::theme::Leading as _;

/// La pregunta, pintada. Los ids de sus piezas empiezan por `prefix`
/// (`doc-ask`, `doc-ask-yes`…). `focus` es su foco: con él, Escape contesta
/// que no; quien la pone se lo da al abrirla. `answer` recibe la respuesta.
pub fn question_dialog(
    prefix: &'static str,
    question: Question,
    focus: &FocusHandle,
    answer: impl Fn(bool, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let (background, foreground, muted, soft, border, danger) =
        (theme.background, theme.foreground, theme.muted_foreground, theme.muted, theme.border, theme.danger);
    let (rest, over) = if theme.is_dark() { (0.2, 0.3) } else { (0.1, 0.2) };
    let id = |piece: &str| SharedString::from(if piece.is_empty() { prefix.to_string() } else { format!("{prefix}-{piece}") });
    let answer = Rc::new(answer);
    let (escape, no, yes, close) = (answer.clone(), answer.clone(), answer.clone(), answer);
    let card = div()
        .id(id(""))
        .test_support()
        .track_focus(focus)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                escape(false, window, cx);
            }
            // Ninguna tecla llega a lo de debajo mientras se pregunta.
            cx.stop_propagation();
        })
        .relative()
        .w_full()
        .max_w(px(384.))
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .rounded(px(14.))
        .border_1()
        .border_color(foreground.opacity(0.1))
        .bg(background)
        .shadow_2xl()
        .leading_sm()
        .text_color(foreground)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .id(id("title"))
                        .aria_label(question.title.clone())
                        .pr_6()
                        .text_size(rems(1.))
                        .line_height(rems(1.25))
                        .font_weight(FontWeight::MEDIUM)
                        .child(question.title)
                        .test_support(),
                )
                .child(div().id(id("body")).aria_label(question.body.clone()).text_color(muted).child(question.body).test_support()),
        )
        .child(
            div()
                .mx(px(-16.))
                .mb(px(-16.))
                .p_4()
                .flex()
                .justify_end()
                .gap_2()
                .rounded_b(px(14.))
                .border_t_1()
                .border_color(border)
                .bg(soft.opacity(0.5))
                .child(Button::new(id("no")).outline().small().h_8().px_2p5().label(question.cancel).on_click(move |_, window, cx| no(false, window, cx)))
                .child(
                    // `variant="destructive"`: lo que pierde algo.
                    div()
                        .id(id("yes"))
                        .h_8()
                        .px_2p5()
                        .flex()
                        .flex_none()
                        .items_center()
                        .rounded(px(8.))
                        .bg(danger.opacity(rest))
                        .leading_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(danger)
                        .cursor_pointer()
                        .hover(move |style| style.bg(danger.opacity(over)))
                        .on_click(move |_, window, cx| yes(true, window, cx))
                        .child(question.confirm)
                        .test_support(),
                ),
        )
        .child(
            div().absolute().top_2().right_2().child(
                Button::new(id("close")).ghost().xsmall().size_7().p_0().child(Icon::new(Lucide::X).size_4()).on_click(move |_, window, cx| close(false, window, cx)),
            ),
        );
    let size = window.viewport_size();
    // Sobre toda la ventana y encima de lo demás: mientras se pregunta
    // tampoco se pulsa el árbol de al lado.
    deferred(
        anchored().position(Point::default()).child(
            div().id(id("veil")).w(size.width).h(size.height).occlude().flex().items_center().justify_center().p_4().bg(gpui_kit::black().opacity(0.35)).child(card),
        ),
    )
    .with_priority(2)
}
