//! «Mi trabajo» en tablero: cuatro columnas por estado, y arrastrar una
//! tarjeta a otra para cambiárselo.
//!
//! Qué columnas hay, qué lleva cada tarjeta, si una puede soltarse en una
//! columna y qué pasa al soltarla lo dice el core (`mywork/board.rs`). Aquí
//! se pinta, con las medidas de `KanbanBoard.tsx` y `TaskCardMini.tsx`, y se
//! le pasa el gesto.

use cac_core::{BoardCard, BoardColumn, BoardStatus, DropVerdict, TaskFilter};
use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::locale;
use crate::mywork::{due_tone, priority_chip, MyWorkView};

/// El ancho de una columna y de la tarjeta que va en el aire (`w-72`).
const COLUMN_WIDTH: f32 = 288.;

/// La tarjeta que va en el aire: de cuál se trata, para preguntarle al core
/// dónde puede caer, y lo que se pinta bajo el puntero.
#[derive(Clone)]
pub(crate) struct LiftedCard {
    card: BoardCard,
}

impl Render for LiftedCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(COLUMN_WIDTH - 18.)).opacity(0.95).child(card_face(&self.card, cx).shadow_lg())
    }
}

/// El color de un estado: los `--status-*` del tema puesto.
pub(crate) fn status_tone(status: BoardStatus, cx: &App) -> Hsla {
    let token = match status {
        BoardStatus::Open => "status-open",
        BoardStatus::InProgress => "status-progress",
        BoardStatus::Done => "status-done",
        BoardStatus::Closed => "status-closed",
    };
    crate::theme::css_color(token, cx)
}

/// Lo que se ve de una tarjeta, esté en su columna o en el aire.
fn card_face(card: &BoardCard, cx: &App) -> Div {
    let theme = cx.theme();
    let (muted, border, mono) = (theme.muted_foreground, theme.border, theme.mono_font_family.clone());
    let raised = crate::theme::css_color("raised", cx);
    let due = card.due.as_ref().map(|due| {
        let (tone, strong) = due_tone(due.state, cx);
        div().flex_none().text_color(tone).when(strong, |this| this.font_weight(FontWeight::MEDIUM)).child(due.text.clone())
    });
    let assignee = card.assignee.as_ref().map(|person| {
        let tint = crate::theme::space_color("", person.hue, cx);
        div()
            .ml_auto()
            .size(px(20.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(tint.opacity(0.15))
            .text_size(px(9.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(tint)
            .child(person.initials.clone())
    });
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded_lg()
        .border_1()
        .border_color(border)
        .bg(raised)
        .px_2p5()
        .py_2()
        .shadow_sm()
        .child(
            div()
                .flex()
                .items_baseline()
                .gap_1p5()
                .text_size(px(11.))
                .text_color(muted)
                .child(div().flex_none().font_family(mono.clone()).child(format!("#{}", card.seq)))
                .child(div().min_w_0().truncate().child(card.path.clone())),
        )
        .child(div().mt_0p5().text_sm().line_clamp(2).child(card.title.clone()))
        .child(
            div()
                .mt_1p5()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(11.))
                .children(priority_chip(&card.priority, cx))
                .when(card.subtasks_total > 0, |this| {
                    this.child(div().flex_none().font_family(mono).text_color(muted).child(format!("{}/{}", card.subtasks_done, card.subtasks_total)))
                })
                .child(due.unwrap_or_else(|| div().flex_none().text_color(muted).child("—")))
                .children(assignee),
        )
}

impl MyWorkView {
    /// Suelta la tarjeta en esa columna. Lo que el flujo prohíbe no se pide
    /// ni se avisa (la columna ya salió en rojo mientras iba en el aire); lo
    /// que el servidor rechaza se dice, con la tarjeta ya de vuelta.
    fn drop_card(&mut self, card_id: String, status: BoardStatus, cx: &mut Context<Self>) {
        self.board_error = None;
        cx.notify();
        if !matches!(Backend::core(cx).my_work().drop_verdict(&card_id, status), Some(DropVerdict::Move)) {
            return;
        }
        let answer = Backend::ask(cx, move |core| async move { core.my_work().drop_on(&card_id, status).await });
        cx.spawn(async move |this, cx| {
            let Ok(Err(error)) = answer.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.board_error = Some(crate::lang::error_text(cx, &error, "common:misc.errSave"));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_column(&self, column: BoardColumn, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, danger, primary, hover) = (theme.border, theme.muted_foreground, theme.danger, theme.primary, theme.primary.opacity(0.4));
        let sunken = crate::theme::css_color("sunken", cx);
        let tone = status_tone(column.status, cx);
        let status = column.status;
        let key = status.key();
        // Con una tarjeta en el aire, la columna donde su flujo no la deja
        // caer se apaga y se bordea en rojo: se ve antes de soltar.
        let refused = self
            .lifted
            .as_ref()
            .is_some_and(|id| matches!(Backend::core(cx).my_work().drop_verdict(id, status), Some(DropVerdict::Refused { .. })));
        let count = column.cards.len();
        let empty = column.cards.is_empty().then(|| div().px_1().py_6().text_xs().text_center().text_color(muted).child(column.empty_hint.clone()));
        let weak = cx.weak_entity();
        let cards = column.cards.into_iter().map(|card| {
            let (open_id, lifted, weak) = (card.id.clone(), LiftedCard { card: card.clone() }, weak.clone());
            let in_the_air = self.lifted.as_deref() == Some(card.id.as_str());
            div()
                .id(SharedString::from(format!("card-{}", card.id)))
                .cursor_pointer()
                .when(in_the_air, |this| this.opacity(0.4))
                .hover(move |style| style.border_color(hover))
                .on_click(move |_, _, cx| {
                    let id = open_id.clone();
                    Backend::run(cx, move |core| async move { core.tasks().open(&id).await });
                })
                .on_drag(lifted, move |lifted, _, _, cx| {
                    let id = lifted.card.id.clone();
                    let _ = weak.update(cx, |this, cx| {
                        this.lifted = Some(id);
                        cx.notify();
                    });
                    let lifted = lifted.clone();
                    cx.new(|_| lifted)
                })
                .child(card_face(&card, cx))
                .test_support()
        });
        div()
            .id(SharedString::from(format!("column-{key}")))
            .w(px(COLUMN_WIDTH))
            .h_full()
            .flex()
            .flex_none()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(if refused { danger.opacity(0.4) } else { border })
            .border_t_2()
            .bg(sunken)
            .overflow_hidden()
            .child(div().h(px(2.)).flex_none().bg(tone))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .child(div().size(px(8.)).flex_none().rounded_full().bg(tone))
                    .child(div().truncate().text_xs().font_weight(FontWeight::SEMIBOLD).child(column.title.to_uppercase()))
                    .child(
                        div()
                            .id(SharedString::from(format!("column-count-{key}")))
                            .px_1p5()
                            .rounded_full()
                            .bg(tone.opacity(0.15))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(tone)
                            .child(count.to_string())
                            .test_support(),
                    )
                    // Terminadas y cerradas sin pedir: una raya, que no es
                    // lo mismo que «no hay ninguna».
                    .when(!column.asked, |this| this.child(div().ml_auto().text_xs().text_color(muted).child("—"))),
            )
            .child(
                div()
                    .id(SharedString::from(format!("column-cards-{key}")))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .overflow_y_scroll()
                    .when(refused, |this| this.opacity(0.5))
                    .drag_over::<LiftedCard>(move |style, lifted, _, cx| {
                        match Backend::core(cx).my_work().drop_verdict(&lifted.card.id, status) {
                            Some(DropVerdict::Refused { .. }) => style.bg(danger.opacity(0.15)).opacity(1.),
                            _ => style.bg(primary.opacity(0.05)),
                        }
                    })
                    .on_drop(cx.listener(move |this, lifted: &LiftedCard, _, cx| {
                        this.lifted = None;
                        this.drop_card(lifted.card.id.clone(), status, cx);
                    }))
                    .children(cards)
                    .children(empty)
                    .test_support(),
            )
            .test_support()
    }

    /// El tablero entero: las columnas que dice el core, una al lado de otra.
    pub(crate) fn render_board(&mut self, filter: &TaskFilter, cx: &mut Context<Self>) -> impl IntoElement {
        // Soltar fuera de toda columna no avisa a nadie: sin nada en el
        // aire, ninguna tarjeta sigue levantada.
        if self.lifted.is_some() && !cx.has_active_drag() {
            self.lifted = None;
        }
        let danger = cx.theme().danger;
        let columns = Backend::core(cx).my_work().board(filter, locale(cx));
        let error = self.board_error.clone();
        div()
            .id("work-board")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .children(error.map(|error| div().id("board-error").px_4().pt_3().text_sm().text_color(danger).child(error).test_support()))
            .child(
                div()
                    .id("board-columns")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_3()
                    .p_4()
                    .overflow_x_scroll()
                    .children(columns.into_iter().map(|column| self.render_column(column, cx))),
            )
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use crate::backend::Backend;
    use crate::testing::{contract_task, ok, server_with_tasks, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    /// Ancha: las cuatro columnas tienen que caber para poder soltar en
    /// cualquiera.
    fn open(server: &FakeServer, cx: &mut TestAppContext) -> AnyWindowHandle {
        server.install(cx);
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1700.), px(800.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window").0
        })
    }

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    /// Si esa tarjeta está pintada dentro de esa columna.
    fn in_column(window: AnyWindowHandle, card: &str, column: &str, cx: &mut TestAppContext) -> bool {
        let (card, column) = (gpui_kit::SharedString::from(format!("card-{card}")), gpui_kit::SharedString::from(format!("column-{column}")));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within(column).try_find(card).is_some()
        })
        .unwrap()
    }

    fn click(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    fn drag(window: AnyWindowHandle, card: &str, column: &str, cx: &mut TestAppContext) {
        let (card, column) =
            (gpui_kit::SharedString::from(format!("card-{card}")), gpui_kit::SharedString::from(format!("column-cards-{column}")));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.drag_to(card, column, cx);
        })
        .unwrap();
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Una interna pendiente, una en curso y una del cliente pendiente.
    fn server() -> FakeServer {
        let task = |id: &str, status: &str, flow: &str| {
            contract_task(id, "a", |task| {
                task["status"] = json!(status);
                task["flow"] = json!(flow);
                task["title"] = json!(format!("Tarea {id}"));
            })
        };
        let server =
            server_with_tasks(vec![task("t1", "pending", "internal"), task("t2", "in_progress", "internal"), task("t3", "pending", "client")]);
        let columns = json!([
            { "id": "col-a", "listId": "l-a", "name": "Open", "kind": "open", "status": "pending" },
            { "id": "col-b", "listId": "l-a", "name": "In progress", "kind": "active", "status": "in_progress" },
            { "id": "col-c", "listId": "l-a", "name": "Done", "kind": "done", "status": "resolved" },
            { "id": "col-d", "listId": "l-a", "name": "Closed", "kind": "done", "status": "closed" },
        ]);
        server.mount(Mock::given(method("GET")).and(path("/api/v1/task-lists/l-a/statuses")).respond_with(ok(columns)));
        server
    }

    fn show_board(server: &FakeServer, cx: &mut TestAppContext) -> AnyWindowHandle {
        let window = open(server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "layout-board", cx);
        wait_until(cx, "sale el tablero", |cx| on_screen(window, "work-board", cx));
        window
    }

    /// Elegir «Tablero» cambia la lista por cuatro columnas, con cada
    /// tarjeta en la de su estado; pulsar una tarjeta abre la tarea.
    ///
    /// Roturas que atrapa: un selector de vista que no cambia nada; todas las
    /// tarjetas en la primera columna; una tarjeta que no abre su tarea.
    #[gpui_kit::test]
    fn the_board_puts_each_card_in_the_column_of_its_status(cx: &mut TestAppContext) {
        let server = server();
        let window = show_board(&server, cx);

        assert!(!on_screen(window, "work-row-t1", cx), "la lista sigue debajo del tablero");
        for column in ["open", "in_progress", "done", "closed"] {
            assert!(on_screen(window, &format!("column-{column}"), cx), "falta la columna {column}");
        }
        assert!(in_column(window, "t1", "open", cx));
        assert!(in_column(window, "t2", "in_progress", cx));
        assert!(!in_column(window, "t2", "open", cx), "la tarjeta en curso sale entre las pendientes");

        click(window, "card-t2", cx);
        wait_until(cx, "se abre la tarea", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.as_deref() == Some("t2")));
        stop(cx);
    }

    /// Arrastrar una tarjeta a otra columna le cambia el estado en su lista.
    ///
    /// Roturas que atrapa: una columna que no recibe lo que se suelta; soltar
    /// que no pide nada al servidor; pedir el cambio para otra tarea.
    #[gpui_kit::test]
    fn dragging_a_card_to_another_column_moves_the_task(cx: &mut TestAppContext) {
        let server = server();
        let moved = contract_task("t1", "a", |task| task["status"] = json!("in_progress"));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t1/move")).respond_with(ok(moved)));
        let window = show_board(&server, cx);

        drag(window, "t1", "in_progress", cx);
        wait_until(cx, "se pide el cambio", |_| server.hits("/api/v1/tasks/t1/move") == 1);
        assert!(!on_screen(window, "board-error", cx));
        stop(cx);
    }

    /// Lo que el flujo del cliente no deja (de pendiente a terminada, sin
    /// pasar por en curso) no se pide ni se avisa: la tarjeta se queda.
    ///
    /// Roturas que atrapa: mandar al servidor un movimiento que va a
    /// rechazar; enseñar un error por algo que la columna ya dijo en rojo.
    #[gpui_kit::test]
    fn a_drop_the_flow_forbids_asks_nothing(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t3/move")).respond_with(ok(json!(null))));
        let window = show_board(&server, cx);

        drag(window, "t3", "done", cx);
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(150));
        cx.run_until_parked();
        assert_eq!(server.hits("/api/v1/tasks/t3/move"), 0, "se pidió un movimiento prohibido");
        assert!(in_column(window, "t3", "open", cx));
        assert!(!on_screen(window, "board-error", cx), "avisa de un error por soltar donde no se puede");
        stop(cx);
    }

    /// Si el servidor rechaza el movimiento, se dice y la tarjeta vuelve.
    ///
    /// Roturas que atrapa: un rechazo que se traga en silencio (la tarjeta
    /// vuelve sola a su sitio y nadie sabe por qué).
    #[gpui_kit::test]
    fn a_move_the_server_refuses_is_said(cx: &mut TestAppContext) {
        let server = server();
        let refusal = ResponseTemplate::new(409).set_body_json(json!({ "success": false, "error": "subtasks-open", "message": "Close its subtasks first" }));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t1/move")).respond_with(refusal));
        let window = show_board(&server, cx);

        drag(window, "t1", "in_progress", cx);
        wait_until(cx, "se dice el rechazo", |cx| on_screen(window, "board-error", cx));
        assert!(in_column(window, "t1", "open", cx), "la tarjeta no volvió a su columna");
        stop(cx);
    }
}
