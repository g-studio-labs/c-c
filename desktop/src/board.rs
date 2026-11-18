//! «Mi trabajo» en tablero: cuatro columnas por estado, y arrastrar una
//! tarjeta a otra para cambiárselo.
//!
//! Qué columnas hay, qué lleva cada tarjeta, si una puede soltarse en una
//! columna y qué pasa al soltarla lo dice el core (`mywork/board.rs`). Aquí
//! se pinta, con las medidas de `KanbanBoard.tsx` y `TaskCardMini.tsx`, y se
//! le pasa el gesto.

use cac_core::{BoardCard, BoardColumn, BoardStatus, DropVerdict, TaskFilter};
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::locale;
use crate::mywork::{due_tone, priority_chip, Leading, MyWorkView};

/// El ancho de una columna y de la tarjeta que va en el aire (`w-72`).
const COLUMN_WIDTH: f32 = 288.;
/// `rounded-lg` en la app de siempre: su `--radius`, 10 px, no los 8 de
/// fábrica.
const RADIUS: f32 = 10.;

/// Una sombra de CSS (`0 y blur spread rgb(0 0 0 / alpha)`).
fn shadow(y: f32, blur: f32, spread: f32, alpha: f32) -> BoxShadow {
    BoxShadow { color: gpui_kit::black().opacity(alpha), offset: point(px(0.), px(y)), blur_radius: px(blur), spread_radius: px(spread), inset: false }
}

/// Un anillo por dentro (`ring-1 ring-inset`): no ocupa, como el de CSS.
fn inset_ring(color: Hsla) -> BoxShadow {
    BoxShadow { color, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: true }
}

/// La tarjeta que va en el aire: de cuál se trata, para preguntarle al core
/// dónde puede caer, y lo que se pinta bajo el puntero.
#[derive(Clone)]
pub(crate) struct LiftedCard {
    card: BoardCard,
}

impl Render for LiftedCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // `w-72 rotate-2 scale-[1.03] rounded-md opacity-95 shadow-2xl` en
        // `KanbanBoard.tsx`: más ancha que en su columna y con la sombra
        // grande. Girarla no se puede: GPUI sólo gira iconos, no cajas.
        div().w(px(COLUMN_WIDTH)).opacity(0.95).child(card_face(&self.card, cx).shadow_2xl())
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

/// Lo que se ve de una tarjeta, esté en su columna o en el aire
/// (`TaskCardMini.tsx`: `rounded-lg border bg-raised px-2.5 py-2 shadow-sm`).
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
        .rounded(px(RADIUS))
        .border_1()
        .border_color(border)
        .bg(raised)
        // El 1,5 del cuerpo, también cuando va en el aire, fuera de la
        // pantalla que se lo da a las demás.
        .line_height(relative(1.5))
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
        .child(div().mt_0p5().leading_sm().line_clamp(2).text_ellipsis().child(card.title.clone()))
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
    /// Suelta la tarjeta en esa columna, y **todo «no» se dice**: lo rechace
    /// el core sin pedir nada (su máquina no lo deja, o tiene subtareas
    /// abiertas) o lo rechace el servidor, sale el motivo en el aviso del
    /// tablero, con la tarjeta en su columna. Hasta el 17-nov-2026 lo que
    /// prohibía la máquina no se avisaba, «porque la columna ya salió en
    /// rojo»: jose decidió ese día que un movimiento bloqueado nunca es
    /// silencioso (command-and-control-387) — el rojo dice que no, no por qué.
    ///
    /// Qué se puede y cómo se dice lo decide el core (`drop_on`,
    /// `drop_error_phrase`); aquí sólo se le pasa el gesto y se pinta lo que
    /// conteste. El aviso se queda hasta el siguiente gesto.
    fn drop_card(&mut self, card_id: String, status: BoardStatus, cx: &mut Context<Self>) {
        self.board_error = None;
        cx.notify();
        let asked = card_id.clone();
        let answer = Backend::ask(cx, move |core| async move { core.my_work().drop_on(&asked, status).await });
        cx.spawn(async move |this, cx| {
            let Ok(Err(error)) = answer.await else { return };
            let _ = this.update(cx, |this, cx| {
                let phrase = Backend::core(cx).my_work().drop_error_phrase(locale(cx), &error, &card_id, status, "common:misc.errSave");
                this.board_error = Some(phrase);
                cx.notify();
            });
        })
        .detach();
    }

    fn render_column(&self, column: BoardColumn, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, danger, primary, hover) = (theme.border, theme.muted_foreground, theme.danger, theme.primary, theme.primary.opacity(0.4));
        // Al pasar por encima, la tarjeta se levanta (`.lift` en
        // `index.css`): la sombra aparece sobre la suya de siempre.
        let lifted_shadow = vec![shadow(1., 3., 0., 0.1), shadow(1., 2., -1., 0.1), shadow(8., 20., -8., 0.35)];
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
        let empty = column.cards.is_empty().then(|| div().px_1().py_6().leading_xs().text_center().text_color(muted).child(column.empty_hint.clone()));
        let weak = cx.weak_entity();
        let cards = column.cards.into_iter().map(|card| {
            let (open_id, lifted, weak) = (card.id.clone(), LiftedCard { card: card.clone() }, weak.clone());
            let in_the_air = self.lifted.as_deref() == Some(card.id.as_str());
            let lifted_shadow = lifted_shadow.clone();
            // La tarjeta misma es lo que se pulsa: el borde que cambia al
            // pasar por encima es el suyo.
            card_face(&card, cx)
                .id(SharedString::from(format!("card-{}", card.id)))
                .flex_none()
                .cursor_pointer()
                .when(in_the_air, |this| this.opacity(0.4))
                .hover(move |style| style.border_color(hover).shadow(lifted_shadow.clone()))
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
                .test_support()
        });
        div()
            .id(SharedString::from(format!("column-{key}")))
            .relative()
            .w(px(COLUMN_WIDTH))
            .h_full()
            .flex()
            .flex_none()
            .flex_col()
            .rounded(px(RADIUS))
            .border_1()
            .border_color(if refused { danger.opacity(0.4) } else { border })
            .border_t_2()
            .bg(sunken)
            .overflow_hidden()
            // El filo de arriba en el color de su estado (`border-t-2
            // border-t-status-*`): GPUI da un solo color a los cuatro lados,
            // así que el de arriba se pinta encima, sólo ese lado y con las
            // mismas esquinas (lo absoluto se mide aquí desde el borde de
            // fuera, no desde dentro de él como en CSS).
            .child(div().absolute().top_0().left_0().right_0().h(px(RADIUS + 4.)).rounded_t(px(RADIUS)).border_t_2().border_color(tone))
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
                    .child(div().truncate().leading_xs().font_weight(FontWeight::SEMIBOLD).child(column.title.to_uppercase()))
                    .child(
                        div()
                            .id(SharedString::from(format!("column-count-{key}")))
                            .px_1p5()
                            .rounded_full()
                            .bg(tone.opacity(0.15))
                            .leading_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(tone)
                            .child(count.to_string())
                            .test_support(),
                    )
                    // Terminadas y cerradas sin pedir: una raya, que no es
                    // lo mismo que «no hay ninguna». Va a la letra del
                    // cuerpo (16/24): en `MyWork.tsx` es un `<span>` sin
                    // tamaño, y por eso esas dos cabeceras salen seis
                    // píxeles más altas también en la app de siempre.
                    .when(!column.asked, |this| this.child(div().ml_auto().flex_none().text_base().line_height(px(24.)).text_color(muted).child("—"))),
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
                            // `bg-destructive/15 ring-1 ring-inset ring-destructive/50`.
                            Some(DropVerdict::Refused { .. }) => style.bg(danger.opacity(0.15)).shadow(vec![inset_ring(danger.opacity(0.5))]).opacity(1.),
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
            // El relleno va dos veces, como en la app de siempre: el `p-4`
            // del cuerpo de `MyWork.tsx` y el del propio tablero
            // (`flex h-full gap-3 overflow-x-auto p-4`), que es el que se
            // desliza de lado.
            .p_4()
            // El aviso de un movimiento que no se hizo: una caja roja con
            // su icono, no un renglón suelto (`border-error/40 bg-error/10
            // text-error` de los avisos de la app de siempre). Tiene que
            // verse: es lo único que dice por qué la tarjeta sigue donde
            // estaba.
            .children(error.map(|error| {
                div().px_4().pb_2().child(
                    div()
                        .id("board-error")
                        .aria_label(error.clone())
                        .flex()
                        .items_start()
                        .gap_1p5()
                        .px_2()
                        .py_1p5()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(danger.opacity(0.4))
                        .bg(danger.opacity(0.1))
                        .leading_sm()
                        .text_color(danger)
                        .child(Icon::new(gpui_kit::assets::IconName::CircleAlert).size_3p5().flex_none().mt(px(3.)))
                        .child(div().min_w_0().child(error))
                        .test_support(),
                )
            }))
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

    /// Lo que dice el aviso del tablero.
    fn said(window: AnyWindowHandle, cx: &mut TestAppContext) -> String {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find(gpui_kit::SharedString::from("board-error")).label().unwrap_or_default().to_string()
        })
        .unwrap()
    }

    /// Lo que el flujo del cliente no deja (de pendiente a terminada, sin
    /// pasar por en curso) no se pide, la tarjeta se queda, y **se dice por
    /// qué**: la columna en rojo dice que no, no el motivo.
    ///
    /// Roturas que atrapa: mandar al servidor un movimiento que va a
    /// rechazar; un «no» mudo (lo que había: la tarjeta volvía y nadie sabía
    /// por qué); un aviso sin el motivo; la tarjeta en la columna a la que
    /// no podía ir.
    #[gpui_kit::test]
    fn a_drop_the_flow_forbids_asks_nothing_and_says_why(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t3/move")).respond_with(ok(json!(null))));
        let window = show_board(&server, cx);

        drag(window, "t3", "done", cx);
        wait_until(cx, "se dice el rechazo", |cx| on_screen(window, "board-error", cx));
        assert_eq!(said(window, cx), "Ese estado no puede ir detrás de éste.");
        assert_eq!(server.hits("/api/v1/tasks/t3/move"), 0, "se pidió un movimiento prohibido");
        assert!(in_column(window, "t3", "open", cx) && !in_column(window, "t3", "done", cx));
        stop(cx);
    }

    /// Un padre con subtareas abiertas no cae ni en «Cerrada» ni en «Hecha»:
    /// no se pide nada, la tarjeta sigue en su columna y el aviso dice el
    /// motivo con la cuenta (command-and-control-387). Y el aviso se va con
    /// el siguiente movimiento que sí se hace.
    ///
    /// (La tarjeta de la fixture del contrato tiene tres subtareas y una
    /// terminada: dos abiertas.)
    ///
    /// Roturas que atrapa: soltar y que no pase ni se diga nada; pedirle al
    /// servidor lo que se sabe rechazado; un aviso genérico («No se pudo
    /// guardar») o sin cuántas quedan; decir «cerrar» a quien soltó en
    /// «Hecha»; la tarjeta pintada en la columna de destino; y un aviso que
    /// se queda puesto después de un movimiento que salió bien.
    #[gpui_kit::test]
    fn a_parent_with_open_subtasks_dropped_on_done_or_closed_stays_and_says_how_many(cx: &mut TestAppContext) {
        let server = server();
        let moved = contract_task("t1", "a", |task| task["status"] = json!("in_progress"));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t1/move")).respond_with(ok(moved)));
        let window = show_board(&server, cx);

        drag(window, "t1", "closed", cx);
        wait_until(cx, "se dice el rechazo", |cx| on_screen(window, "board-error", cx));
        assert_eq!(said(window, cx), "No se puede cerrar: tiene 2 subtareas abiertas");
        assert!(in_column(window, "t1", "open", cx) && !in_column(window, "t1", "closed", cx), "la tarjeta no está en su columna");

        drag(window, "t1", "done", cx);
        wait_until(cx, "se dice el otro rechazo", |cx| said(window, cx) == "No se puede marcar como hecha: tiene 2 subtareas abiertas");
        assert!(in_column(window, "t1", "open", cx) && !in_column(window, "t1", "done", cx));
        assert_eq!(server.hits("/api/v1/tasks/t1/move"), 0, "se pidió un movimiento que se sabía rechazado");

        // Lo que sí se puede se hace, y el aviso de antes ya no pinta nada.
        drag(window, "t1", "in_progress", cx);
        wait_until(cx, "se pide el cambio", |_| server.hits("/api/v1/tasks/t1/move") == 1);
        assert!(!on_screen(window, "board-error", cx), "el aviso sigue puesto tras un movimiento que salió bien");
        stop(cx);
    }

    /// Si quien rechaza es el servidor —la tarjeta no sabía de sus subtareas
    /// abiertas—, se dice el mismo motivo y la tarjeta vuelve.
    ///
    /// Roturas que atrapa: un rechazo que se traga en silencio (la tarjeta
    /// vuelve sola a su sitio y nadie sabe por qué); decirlo como un fallo
    /// cualquiera («No se pudo guardar»), sin el motivo; la tarjeta en
    /// «Hecha».
    #[gpui_kit::test]
    fn a_move_the_server_refuses_is_said(cx: &mut TestAppContext) {
        let alone = contract_task("t1", "a", |task| {
            task["status"] = json!("in_progress");
            task["flow"] = json!("internal");
            task["subtasksTotal"] = json!(0);
            task["subtasksDone"] = json!(0);
        });
        let server = server_with_tasks(vec![alone]);
        let columns = json!([
            { "id": "col-b", "listId": "l-a", "name": "In progress", "kind": "active", "status": "in_progress" },
            { "id": "col-c", "listId": "l-a", "name": "Done", "kind": "done", "status": "resolved" },
        ]);
        server.mount(Mock::given(method("GET")).and(path("/api/v1/task-lists/l-a/statuses")).respond_with(ok(columns)));
        let refusal = ResponseTemplate::new(409).set_body_json(json!({
            "success": false, "error": "subtasks-open",
            "message": "cannot mark this task done: 2 subtasks are still open — finish or close them first",
        }));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t1/move")).respond_with(refusal));
        let window = show_board(&server, cx);

        drag(window, "t1", "done", cx);
        wait_until(cx, "se dice el rechazo", |cx| on_screen(window, "board-error", cx));
        assert_eq!(server.hits("/api/v1/tasks/t1/move"), 1);
        assert_eq!(said(window, cx), "No se puede terminar ni cerrar: tiene subtareas abiertas. Termínalas o ciérralas primero.");
        wait_until(cx, "la tarjeta vuelve a su columna", |cx| in_column(window, "t1", "in_progress", cx) && !in_column(window, "t1", "done", cx));
        stop(cx);
    }
}
