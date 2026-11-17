//! «Mi trabajo» en calendario: el mes, la semana y «Por programar».
//!
//! Qué cae en cada día, qué es hoy, qué venció, cómo se titula un mes y qué
//! pasa al soltar una tarjeta en un día lo dice el core
//! (`mywork/calendar.rs`). Aquí se pinta, con las medidas de
//! `components/calendar/*.tsx`, y se le pasa el gesto.
//!
//! Lo que es de esta pantalla y de nadie más: si se mira el mes o la semana,
//! si el carril está abierto y qué día tiene su detalle a la vista.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use cac_core::{
    lane_open_by_default, BoardStatus, CalendarChip, CalendarCursor, CalendarDay, CalendarView, MonthView, TaskFilter, ToneCount, WeekView,
    CALENDAR_TONES,
};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::mywork::{Leading, MyWorkView};

/// El ancho del carril y el de la tarjeta que va en el aire.
const LANE_WIDTH: f32 = 220.;
const LIFTED_WIDTH: f32 = 200.;
/// Lo menos que mide de ancho un día de la semana de lunes a viernes: lo que
/// una tarjeta pide para llevar en un renglón su `#número`, «Vencida» y el
/// avatar, y dos palabras de título por renglón. Decidido con jose
/// (17-nov-2026): a 900 px con el carril y el panel abiertos las siete
/// columnas quedaban en 47 px y no se leía nada (el CSS de la app de siempre
/// da lo mismo); un día no se estrecha más que esto, y si los siete no caben
/// la semana se desliza de lado dentro de su hueco.
// 140 y no 150: en una ventana de 1500 con el carril y el panel abiertos
// la semana tiene 932 de ancho, y con 150 ya se deslizaba unos 50 px sin
// necesidad.
const DAY_MIN_WIDTH: f32 = 140.;
/// Lo que mide el fin de semana frente a un día de diario: de lunes a
/// viernes `1fr`, sábado y domingo `.62fr`. También en su mínimo.
const WEEKEND_SHARE: f32 = 0.62;
/// El hueco entre dos días de la semana (`gap-2`).
const WEEK_GAP: f32 = 8.;
/// Lo menos que mide la semana entera: sus siete mínimos y los seis huecos.
const WEEK_MIN_WIDTH: f32 = 5. * DAY_MIN_WIDTH + 2. * DAY_MIN_WIDTH * WEEKEND_SHARE + 6. * WEEK_GAP;
/// El detalle de un día (`w-[340px]`), y cuánto se sale de su celda para
/// taparle el borde.
const POPOVER_WIDTH: f32 = 340.;
const POPOVER_OVERHANG: f32 = -8.;
/// Cuánto tiene que quedarse el ratón sobre un día para que su detalle se
/// asome (`HOVER_DELAY` en `MonthGrid.tsx`): menos, y cruzar el mes abre uno
/// por celda.
const HOVER_DELAY: Duration = Duration::from_millis(220);
/// El margen para pasar de la celda al detalle que la tapa sin que se cierre.
const LEAVE_GRACE: Duration = Duration::from_millis(80);
/// El interlineado del calendario (`leading-[normal]` en `ItemCalendar.tsx`):
/// el de la propia letra, 1,21 en Inter, y no el 1,5 del resto de la app.
/// Las medidas del diseño están tomadas con él.
const LEADING: f32 = 1.21;
/// Cómo se cuenta lo que hay en un día: «3 tareas».
const COUNT_KEY: &str = "common:count.tasks";

/// Lo que esta pantalla recuerda del calendario mientras está abierta.
pub(crate) struct CalendarState {
    /// Dónde se mira. Vacío hasta el primer pintado: «hoy» lo dice el core.
    cursor: Option<CalendarCursor>,
    /// Si el carril está abierto, por vista (mes, semana). Sin elegir, lo
    /// que diga el core para esa vista.
    lane_open: [Option<bool>; 2],
    /// La tarjeta que va en el aire, por su id.
    lifted: Option<String>,
    /// Lo que el servidor contestó al rechazar un cambio de fecha.
    error: Option<String>,
    /// El día del mes que tiene su detalle fijado (con un clic).
    day_open: Option<String>,
    /// El día cuyo detalle se asoma porque el ratón lleva un rato encima;
    /// se va al salir. Con uno fijado no se asoma ningún otro.
    day_peek: Option<String>,
    /// El ratón está sobre el detalle asomado: tapa su celda, así que para
    /// la celda el ratón «se fue» en cuanto el detalle salió.
    over_detail: bool,
    /// La espera antes de asomar el detalle; soltarla la cancela.
    peek_wait: Option<Task<()>>,
    /// El foco del detalle: sin él, Esc no llega a nadie.
    focus: FocusHandle,
    /// Dónde quedó pintado el día que tiene el detalle abierto. Se sabe al
    /// pintar, no antes.
    day_place: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Hacia dónde se abre el detalle de ese día, medido: `true` es hacia
    /// arriba. Vacío hasta que se pinta una vez, que es cuando se sabe cuánto
    /// mide y cuánto sitio tiene.
    day_opens_up: Rc<RefCell<Option<(String, bool)>>>,
}

impl CalendarState {
    /// Cierra el detalle del día, fijado o asomado.
    fn forget_day(&mut self) {
        self.day_open = None;
        self.day_peek = None;
        self.over_detail = false;
        self.peek_wait = None;
    }

    /// El día cuyo detalle se enseña: el fijado manda sobre el asomado, y
    /// con una tarjeta en el aire ninguno (taparía dónde soltar).
    fn shown_day(&self) -> Option<&str> {
        if self.lifted.is_some() {
            return None;
        }
        self.day_open.as_deref().or(self.day_peek.as_deref())
    }

    pub(crate) fn new(cx: &mut App) -> Self {
        Self {
            cursor: None,
            lane_open: [None, None],
            lifted: None,
            error: None,
            day_open: None,
            day_peek: None,
            over_detail: false,
            peek_wait: None,
            focus: cx.focus_handle(),
            day_place: Rc::default(),
            day_opens_up: Rc::default(),
        }
    }
}

fn view_index(view: CalendarView) -> usize {
    match view {
        CalendarView::Month => 0,
        CalendarView::Week => 1,
    }
}

/// Un anillo por dentro (`ring-N ring-inset`): no ocupa, como el de CSS.
fn inset_ring(color: Hsla, width: f32) -> Vec<BoxShadow> {
    vec![BoxShadow { color, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(width), inset: true }]
}

/// Los colores del calendario, leídos una vez por pintado: los del tema y
/// los `--nombre` de `index.css` que usan sus clases.
#[derive(Clone, Copy)]
struct Paint {
    foreground: Hsla,
    border: Hsla,
    border_strong: Hsla,
    muted: Hsla,
    muted_foreground: Hsla,
    muted_soft: Hsla,
    accent: Hsla,
    primary: Hsla,
    primary_foreground: Hsla,
    error: Hsla,
    warning: Hsla,
    info: Hsla,
    success: Hsla,
    raised: Hsla,
    track: Hsla,
    popover: Hsla,
    popover_foreground: Hsla,
    today_column: Hsla,
    heat: [Hsla; 4],
    heat_hover: Hsla,
}

impl Paint {
    fn of(cx: &App) -> Self {
        let theme = cx.theme();
        let css = |token: &str| crate::theme::css_color(token, cx);
        Self {
            foreground: theme.foreground,
            border: theme.border,
            border_strong: css("border-strong"),
            muted: css("muted"),
            muted_foreground: theme.muted_foreground,
            muted_soft: css("muted-soft"),
            accent: theme.accent,
            primary: theme.primary,
            primary_foreground: css("primary-foreground"),
            error: css("error"),
            warning: css("warning"),
            info: css("info"),
            success: css("success"),
            raised: css("raised"),
            track: css("track"),
            popover: css("popover"),
            popover_foreground: css("popover-foreground"),
            today_column: css("today-column"),
            heat: [theme.background, css("heat-1"), css("heat-2"), css("heat-3")],
            heat_hover: css("heat-hover"),
        }
    }

    /// El color de un tono (`priorityTones` en `calendar/tones.ts`). «Sin
    /// prioridad» tiene dos grises: el suave en el punto y la barra, el
    /// fuerte en el filo de la tarjeta.
    fn tone(&self, tone: &str, edge: bool) -> Hsla {
        match tone {
            "urgent" => self.error,
            "high" => self.warning,
            "normal" => self.info,
            "low" => self.muted_foreground,
            _ if edge => self.border_strong,
            _ => self.muted_soft,
        }
    }
}

/// El color del punto de un estado; uno que esta versión no conoce, gris.
fn status_dot(status: &str, paint: &Paint, cx: &App) -> Hsla {
    BoardStatus::of(status).map(|status| crate::board::status_tone(status, cx)).unwrap_or(paint.muted_foreground)
}

/// Cómo se llama un tono: el nombre de su prioridad.
fn tone_name(tone: &str, cx: &App) -> String {
    tr(cx, &format!("work:taskPriority.{tone}"))
}

/// La barra segmentada por tono: un segmento por tono con tareas, de ancho
/// proporcional. `track` le pone la pista que llevan la semana y el resumen.
fn tone_bar(tones: &[ToneCount], height: f32, track: bool, paint: &Paint) -> Div {
    div()
        .w_full()
        .h(px(height))
        .flex()
        .flex_none()
        .gap(px(2.))
        .rounded_full()
        .overflow_hidden()
        .when(track, |this| this.bg(paint.track))
        .children(tones.iter().map(|segment| {
            div().h_full().flex_grow(segment.count as f32).flex_shrink(1.).flex_basis(relative(0.)).bg(paint.tone(&segment.tone, false))
        }))
}

/// «2 alta», con el punto de su color: en el detalle del día y en el
/// resumen de la semana.
fn tone_tally(segment: &ToneCount, dot: f32, paint: &Paint, cx: &App) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .child(div().size(px(dot)).flex_none().rounded_full().bg(paint.tone(&segment.tone, false)))
        .child(format!("{} {}", segment.count, tone_name(&segment.tone, cx).to_lowercase()))
}

/// Las iniciales de quien la lleva sobre su color, que sale de su nombre.
fn avatar(chip: &CalendarChip, size: f32, paint: &Paint, cx: &App) -> Option<Div> {
    let (name, initials) = (chip.assignee.as_ref()?, chip.assignee_initials.clone()?);
    Some(
        div()
            .size(px(size))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(crate::theme::space_color("", cac_core::hue_index(name), cx))
            // `INITIALS_SIZE` en `parts.tsx`: 8,5 px en el de 18 y 9 en el de 20.
            .text_size(px(if size <= 18. { 8.5 } else { 9. }))
            .font_weight(FontWeight::BOLD)
            .text_color(paint.primary_foreground)
            .child(initials),
    )
}

/// El chip «Vencida» de una tarjeta.
fn overdue_badge(paint: &Paint, cx: &App) -> Div {
    div()
        .flex_none()
        .px(px(5.))
        .rounded(px(4.))
        .bg(paint.error.opacity(0.12))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(paint.error)
        .child(tr(cx, "common:calendar.overdue"))
}

/// Lo que se ve de una tarjeta (`CardBody`), esté en su día, en el carril o
/// en el aire. La del carril no enseña el estado ni se tacha —lo que no
/// tiene fecha está por hacer— y lleva delante el asa y debajo los atajos.
fn card_face(chip: &CalendarChip, lane: bool, footer: Option<AnyElement>, paint: &Paint, cx: &App) -> Div {
    let mono = cx.theme().mono_font_family.clone();
    let struck = !lane && chip.done;
    let (accent, border_strong) = (paint.accent, paint.border_strong);
    div()
        .relative()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(6.))
        .rounded(px(7.))
        .border_1()
        .border_color(if chip.overdue { paint.error.opacity(0.55) } else { paint.border })
        .bg(paint.raised)
        .py_2()
        .pr_2()
        .pl(px(10.))
        .hover(move |style| if lane { style.border_color(border_strong) } else { style.bg(accent) })
        // El filo de prioridad, encima del borde izquierdo.
        .child(div().absolute().top_0().bottom_0().left_0().w(px(3.)).rounded_l(px(7.)).bg(paint.tone(&chip.tone, true)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .when(lane, |this| this.child(Icon::new(Lucide::GripVertical).size_3().text_color(paint.muted_foreground)))
                .child(div().flex_none().font_family(mono).text_size(px(11.)).text_color(paint.muted_foreground).child(chip.label.clone()))
                .when(chip.overdue, |this| this.child(overdue_badge(paint, cx)))
                .children(avatar(chip, 18., paint, cx).map(|avatar| div().ml_auto().flex_none().child(avatar))),
        )
        .child(
            div()
                .text_size(px(12.5))
                .line_height(px(17.))
                .text_color(if struck { paint.muted_foreground } else { paint.foreground })
                .when(struck, |this| this.line_through())
                .child(chip.title.clone()),
        )
        .when(!lane && !chip.status_name.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_size(px(11.))
                    .text_color(paint.muted_foreground)
                    .child(div().size(px(6.)).flex_none().rounded_full().bg(status_dot(&chip.status, paint, cx)))
                    .child(chip.status_name.clone()),
            )
        })
        .children(footer)
}

/// La tarjeta que va en el aire: de cuál se trata, para decirle al core
/// dónde cayó, y lo que se pinta bajo el puntero.
#[derive(Clone)]
pub(crate) struct LiftedChip {
    chip: CalendarChip,
}

impl Render for LiftedChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let paint = Paint::of(cx);
        div().w(px(LIFTED_WIDTH)).text_color(paint.foreground).line_height(relative(LEADING)).child(card_face(&self.chip, false, None, &paint, cx).shadow_lg())
    }
}

/// Hace arrastrable lo que enseña una tarea: la tarjeta de la semana, la del
/// carril y la fila del detalle de un día. Empezar a arrastrar cierra el
/// detalle abierto, que taparía los días donde soltar.
fn liftable(element: Stateful<Div>, chip: &CalendarChip, view: WeakEntity<MyWorkView>) -> Stateful<Div> {
    element.on_drag(LiftedChip { chip: chip.clone() }, move |lifted, _, _, cx| {
        let id = lifted.chip.id.clone();
        let _ = view.update(cx, |this, cx| {
            this.calendar.lifted = Some(id);
            this.calendar.forget_day();
            cx.notify();
        });
        let lifted = lifted.clone();
        cx.new(|_| lifted)
    })
}

/// Pulsar lo que enseña una tarea la abre.
fn opens_task(element: Stateful<Div>, task_id: &str) -> Stateful<Div> {
    let task_id = task_id.to_string();
    element.cursor_pointer().on_click(move |_, _, cx| {
        cx.stop_propagation();
        let id = task_id.clone();
        Backend::run(cx, move |core| async move { core.tasks().open(&id).await });
    })
}

/// Una caja de la navegación (‹ y ›).
fn nav_box(id: &'static str, icon: Lucide, paint: &Paint) -> Stateful<Div> {
    let accent = paint.accent;
    div()
        .id(id)
        .size(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .border_1()
        .border_color(paint.border)
        .cursor_pointer()
        .hover(move |style| style.bg(accent))
        .child(Icon::new(icon).size_4())
}

impl MyWorkView {
    /// Dónde se mira. La primera vez, el mes de hoy.
    fn calendar_cursor(&mut self, cx: &App) -> CalendarCursor {
        self.calendar
            .cursor
            .get_or_insert_with(|| CalendarCursor::starting(Backend::core(cx).my_work().today(), CalendarView::Month))
            .clone()
    }

    fn move_calendar(&mut self, cursor: CalendarCursor, cx: &mut Context<Self>) {
        self.calendar.cursor = Some(cursor);
        self.calendar.forget_day();
        cx.notify();
    }

    fn lane_is_open(&self, view: CalendarView) -> bool {
        self.calendar.lane_open[view_index(view)].unwrap_or_else(|| lane_open_by_default(view))
    }

    fn set_lane_open(&mut self, view: CalendarView, open: bool, cx: &mut Context<Self>) {
        self.calendar.lane_open[view_index(view)] = Some(open);
        cx.notify();
    }

    /// El ratón entra en un día o sale de él. Encima un rato, su detalle se
    /// asoma; al salir se va, salvo que el ratón esté ya sobre el detalle.
    fn hover_day(&mut self, day: &str, over: bool, cx: &mut Context<Self>) {
        if self.calendar.day_open.is_some() || self.calendar.lifted.is_some() {
            return;
        }
        let day = day.to_string();
        if over {
            self.calendar.peek_wait = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(HOVER_DELAY).await;
                let _ = this.update(cx, |this, cx| {
                    if this.calendar.day_open.is_none() {
                        this.calendar.day_peek = Some(day);
                        cx.notify();
                    }
                });
            }));
        } else if self.calendar.day_peek.as_deref() == Some(day.as_str()) {
            self.calendar.peek_wait = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(LEAVE_GRACE).await;
                let _ = this.update(cx, |this, cx| {
                    if !this.calendar.over_detail && this.calendar.day_peek.as_deref() == Some(day.as_str()) {
                        this.calendar.day_peek = None;
                        cx.notify();
                    }
                });
            }));
        } else {
            self.calendar.peek_wait = None;
        }
    }

    /// El ratón entra en el detalle asomado o sale de él: al salir se va.
    fn hover_detail(&mut self, over: bool, cx: &mut Context<Self>) {
        self.calendar.over_detail = over;
        if !over && self.calendar.day_open.is_none() {
            self.calendar.day_peek = None;
            self.calendar.peek_wait = None;
        }
        cx.notify();
    }

    /// Un clic en un día fija su detalle; otro en el mismo lo suelta.
    fn pin_day(&mut self, day: &str, window: &mut Window, cx: &mut Context<Self>) {
        let already = self.calendar.day_open.as_deref() == Some(day);
        self.calendar.forget_day();
        if !already {
            self.calendar.day_open = Some(day.to_string());
            // Con el foco en el detalle, Esc lo cierra.
            self.calendar.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn close_day(&mut self, cx: &mut Context<Self>) {
        self.calendar.forget_day();
        cx.notify();
    }

    /// Le pone ese día a la tarea, o se lo quita con `None`. El core mueve
    /// la tarjeta al instante y la devuelve si el servidor no lo acepta; aquí
    /// sólo se dice por qué.
    fn set_due(&mut self, task_id: String, day: Option<String>, cx: &mut Context<Self>) {
        self.calendar.lifted = None;
        self.calendar.error = None;
        let answer = Backend::ask(cx, move |core| async move { core.my_work().set_due_day(&task_id, day.as_deref()).await });
        cx.spawn(async move |this, cx| {
            let Ok(Err(error)) = answer.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.calendar.error = Some(crate::lang::error_text(cx, &error, "common:misc.errSave"));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Abre la fila de «tarea nueva» con ese día ya puesto.
    fn new_task_for_day(&mut self, day: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.calendar.forget_day();
        if self.new_task.is_none() {
            self.toggle_new_task(window, cx);
        }
        let day = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok();
        if let (Some(row), Some(day)) = (self.new_task.clone(), day) {
            row.update(cx, |row, cx| row.set_due_day(day, window, cx));
        }
        cx.notify();
    }

    /// La cabecera (`CalendarHeader.tsx`): título, «Por programar», Mes /
    /// Semana; a la derecha la leyenda y ‹ Hoy ›.
    fn render_calendar_header(&self, cursor: &CalendarCursor, title: String, unscheduled: usize, paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cursor.view;
        let lane_open = self.lane_is_open(view);
        let accent = paint.accent;

        let lane = div()
            .id("cal-lane-toggle")
            .h(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.))
            .pl_2()
            .pr(px(10.))
            .rounded(px(6.))
            .border_1()
            .border_color(if lane_open { paint.border_strong } else { paint.border })
            .when(lane_open, |this| this.bg(accent))
            .text_size(px(13.))
            .cursor_pointer()
            .hover(move |style| style.bg(accent))
            .on_click(cx.listener(move |this, _, _, cx| this.set_lane_open(view, !lane_open, cx)))
            .child(Icon::new(Lucide::PanelLeft).size_4())
            .child(tr(cx, "common:calendar.lane"))
            .child(
                div()
                    .h(px(18.))
                    .min_w(px(18.))
                    .px(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(if lane_open { paint.border } else { paint.primary })
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if lane_open { paint.foreground } else { paint.primary_foreground })
                    .child(unscheduled.to_string()),
            )
            .test_support();

        let views = [(CalendarView::Month, "month"), (CalendarView::Week, "week")].map(|(choice, name)| {
            let (active, cursor) = (view == choice, cursor.clone());
            div()
                .id(SharedString::from(format!("cal-view-{name}")))
                .px_3()
                .py(px(3.))
                .rounded(px(6.))
                .cursor_pointer()
                .when(active, |this| this.bg(accent).font_weight(FontWeight::SEMIBOLD))
                .when(!active, |this| this.text_color(paint.muted_foreground))
                .on_click(cx.listener(move |this, _, _, cx| this.move_calendar(cursor.viewed_as(choice), cx)))
                .child(tr(cx, &format!("common:calendar.{name}")))
                .test_support()
        });

        let legend = CALENDAR_TONES.map(|tone| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.))
                .child(div().size_2().flex_none().rounded_full().bg(paint.tone(tone, false)))
                .child(tone_name(tone, cx))
        });

        let (back, forward) = (cursor.stepped(-1), cursor.stepped(1));
        let view_cursor = cursor.clone();
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(14.))
                    .child(div().id("cal-title").min_w(px(190.)).text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).aria_label(title.clone()).child(title).test_support())
                    .child(lane)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(2.))
                            .p(px(2.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(paint.border)
                            .bg(paint.muted)
                            .text_size(px(13.))
                            .children(views),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(div().flex().gap(px(10.)).text_size(px(12.)).text_color(paint.muted_foreground).children(legend))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap_1()
                            .child(
                                nav_box("cal-prev", Lucide::ChevronLeft, paint)
                                    .on_click(cx.listener(move |this, _, _, cx| this.move_calendar(back.clone(), cx)))
                                    .test_support(),
                            )
                            .child(
                                div()
                                    .id("cal-today")
                                    .h(px(28.))
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .px(px(10.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(paint.border)
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(accent))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let today = Backend::core(cx).my_work().today();
                                        this.move_calendar(view_cursor.at_today(today), cx);
                                    }))
                                    .child(tr(cx, "common:calendar.today"))
                                    .test_support(),
                            )
                            .child(
                                nav_box("cal-next", Lucide::ChevronRight, paint)
                                    .on_click(cx.listener(move |this, _, _, cx| this.move_calendar(forward.clone(), cx)))
                                    .test_support(),
                            ),
                    ),
            )
    }

    /// Una fila del detalle de un día (`PopoverRow`).
    fn render_day_row(&self, chip: &CalendarChip, paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = cx.theme().mono_font_family.clone();
        let accent = paint.accent;
        let row = div()
            .id(SharedString::from(format!("cal-row-{}", chip.id)))
            .w_full()
            .flex()
            .gap_2()
            .p(px(6.))
            .rounded(px(8.))
            .hover(move |style| style.bg(accent))
            .child(div().w(px(3.)).flex_none().rounded(px(2.)).bg(paint.tone(&chip.tone, true)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .text_color(if chip.done { paint.muted_foreground } else { paint.foreground })
                            .when(chip.done, |this| this.line_through())
                            .child(chip.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.))
                            .text_color(paint.muted_foreground)
                            .child(div().flex_none().font_family(mono).child(chip.label.clone()))
                            .when(!chip.status_name.is_empty(), |this| {
                                this.child(div().size(px(5.)).flex_none().rounded_full().bg(status_dot(&chip.status, paint, cx)))
                                    .child(chip.status_name.clone())
                            })
                            .when(chip.overdue, |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(10.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(paint.error)
                                        .child(format!("· {}", tr(cx, "common:calendar.overdue"))),
                                )
                            }),
                    ),
            )
            .children(avatar(chip, 20., paint, cx).map(|avatar| div().flex_none().child(avatar)));
        liftable(opens_task(row, &chip.id), chip, cx.weak_entity()).test_support()
    }

    /// El detalle de un día (`DayPopover`), anclado a su celda: hacia la
    /// derecha de lunes a jueves y hacia la izquierda de viernes a domingo;
    /// y hacia abajo, salvo que ahí no quepa en lo que se ve y hacia arriba
    /// se esconda menos de la ventana (lo decide el core).
    ///
    /// En la app de siempre va por filas: hacia arriba en las dos últimas
    /// del mes. Decidido con jose (17-nov-2026): en una ventana baja el de
    /// un día de en medio se salía por abajo y no se alcanzaba. Cuánto mide
    /// y cuánto sitio tiene sólo se sabe al pintarlo, así que la primera
    /// pintada de cada día lo mide sin enseñarlo (transparente, donde iría
    /// por filas) y la siguiente lo pone en su sitio: un fotograma.
    ///
    /// Se asoma con el ratón un rato encima de su día y se va al salir; un
    /// clic lo fija (y entonces lleva la ×). Fijado se cierra con la ×, con
    /// Esc, con un clic fuera y al empezar a arrastrar.
    fn render_day_popover(&self, day: &CalendarDay, column: usize, row: usize, rows: usize, paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        let here = locale(cx);
        let weekday = cac_core::weekday_name(here, day.weekday, false).to_uppercase();
        let heading = if day.today { format!("{weekday} · {}", tr(cx, "common:calendar.todayLower").to_uppercase()) } else { weekday };
        let title = chrono::NaiveDate::parse_from_str(&day.day, "%Y-%m-%d").map(|date| cac_core::day_title(here, date)).unwrap_or_else(|_| day.day.clone());
        let count = cac_core::text_count(here, COUNT_KEY, day.chips.len() as i64);
        let (accent, foreground) = (paint.accent, paint.foreground);
        let iso = day.day.clone();
        let pinned = self.calendar.day_open.as_deref() == Some(day.day.as_str());
        let pin = day.day.clone();
        let measured = self.calendar.day_opens_up.borrow().as_ref().filter(|(of, _)| *of == day.day).map(|(_, up)| *up);
        let up = measured.unwrap_or(row + 2 >= rows);
        // Mide el detalle ya pintado y decide con el sitio que tiene. Si la
        // respuesta cambia (la primera vez, o al deslizar el mes o cambiar
        // de tamaño la ventana), se vuelve a pintar.
        let measure = {
            let (place, decided, of, view) = (self.calendar.day_place.clone(), self.calendar.day_opens_up.clone(), day.day.clone(), cx.entity_id());
            canvas(
                move |detail: Bounds<Pixels>, window, cx| {
                    let Some(cell) = place.get() else { return };
                    let up = cac_core::day_detail_opens_up(
                        f32::from(cell.top()) + POPOVER_OVERHANG,
                        f32::from(cell.bottom()) - POPOVER_OVERHANG,
                        f32::from(detail.size.height),
                        // Lo que se ve es la ventana entera: el detalle
                        // se pinta encima de todo, también de la cabecera
                        // y los filtros, y el hueco del mes no lo recorta.
                        0.,
                        f32::from(window.viewport_size().height),
                    );
                    let now = Some((of, up));
                    if *decided.borrow() != now {
                        decided.replace(now);
                        cx.defer(move |cx| cx.notify(view));
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0()
        };

        let tallies = (!day.tones.is_empty()).then(|| {
            div().flex().flex_wrap().gap(px(6.)).children(day.tones.iter().map(|segment| {
                tone_tally(segment, 6., paint, cx).px_2().py(px(1.)).rounded_full().border_1().border_color(paint.border).bg(cx.theme().group_box).text_size(px(11.))
            }))
        });
        let rows_list = if day.chips.is_empty() {
            div().px(px(2.)).py_1().text_size(px(13.)).text_color(paint.muted_foreground).child(tr(cx, "common:calendar.nothingDue")).into_any_element()
        } else {
            div()
                .id("cal-day-rows")
                // `-mx-1`: las filas se salen cuatro píxeles del relleno
                // del detalle, y su fondo al pasar llega más cerca del borde.
                .mx(px(-4.))
                .max_h(px(320.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .overflow_y_scroll()
                .children(day.chips.iter().map(|chip| self.render_day_row(chip, paint, cx)))
                .into_any_element()
        };

        let panel = div()
            .id("cal-day-popover")
            .absolute()
            .when(column >= 4, |this| this.right(px(POPOVER_OVERHANG)))
            .when(column < 4, |this| this.left(px(POPOVER_OVERHANG)))
            .when(up, |this| this.bottom(px(POPOVER_OVERHANG)))
            .when(!up, |this| this.top(px(POPOVER_OVERHANG)))
            .when(measured.is_none(), |this| this.opacity(0.))
            .child(measure)
            .w(px(POPOVER_WIDTH))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(12.))
            .border_1()
            .border_color(paint.border_strong)
            .bg(paint.popover)
            .text_color(paint.popover_foreground)
            // `shadow-[0_18px_40px_rgba(0,0,0,.55)]`.
            .shadow(vec![BoxShadow { color: gpui_kit::black().opacity(0.55), offset: point(px(0.), px(18.)), blur_radius: px(40.), spread_radius: px(0.), inset: false }])
            .cursor_default()
            // Lo que se pulsa dentro no llega a la celda de debajo.
            .occlude()
            .track_focus(&self.calendar.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close_day(cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_day(cx)))
            .on_hover(cx.listener(|this, over: &bool, _, cx| this.hover_detail(*over, cx)))
            // Asomado, un clic en él lo fija, como el clic en su día.
            .when(!pinned, |this| this.on_click(cx.listener(move |this, _, window, cx| this.pin_day(&pin, window, cx))))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(div().text_size(px(11.)).text_color(paint.muted_foreground).child(heading))
                            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(title)),
                    )
                    .child(
                        div().flex().items_center().gap(px(6.)).child(div().text_size(px(12.)).text_color(paint.muted_foreground).child(count)).when(pinned, |this| this.child(
                            div()
                                .id("cal-day-close")
                                .size(px(26.))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .text_color(paint.muted_foreground)
                                .cursor_pointer()
                                .hover(move |style| style.bg(accent).text_color(foreground))
                                .on_click(cx.listener(|this, _, _, cx| this.close_day(cx)))
                                .child(Icon::new(Lucide::X).size(px(15.)))
                                .test_support(),
                        )),
                    ),
            )
            .children(tallies)
            .child(rows_list)
            .child(
                div().flex().items_center().justify_between().border_t_1().border_color(paint.border).pt_2().text_size(px(12.)).child(
                    div()
                        .id("cal-new-for-day")
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_color(paint.primary)
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| this.new_task_for_day(&iso, window, cx)))
                        .child(Icon::new(Lucide::Plus).size_3p5())
                        .child(tr(cx, "common:calendar.newForDay"))
                        .test_support(),
                )
                .when(!pinned, |this| this.child(div().id("cal-click-to-pin").text_color(paint.muted_soft).child(tr(cx, "common:calendar.clickToPin")).test_support())),
            )
            .test_support();
        // Diferido: se pinta después de todo el mes, encima de las celdas
        // que vienen detrás de la suya.
        deferred(panel).with_priority(1)
    }

    /// Un día del mes (`DayCell`): cuánto hay y de qué tono. Sin títulos y
    /// sin «+N»: el detalle sale en su popover.
    fn render_day_cell(&self, day: &CalendarDay, column: usize, row: usize, rows: usize, paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        let here = locale(cx);
        let count = day.chips.len();
        let open = self.calendar.shown_day() == Some(day.day.as_str());
        let hover_day = day.day.clone();
        let (primary, heat_hover) = (paint.primary, paint.heat_hover);
        let date = chrono::NaiveDate::parse_from_str(&day.day, "%Y-%m-%d").map(|date| cac_core::day_title(here, date)).unwrap_or_else(|_| day.day.clone());
        let label = format!("{date} · {}", cac_core::text_count(here, COUNT_KEY, count as i64));
        let (open_day, drop_day) = (day.day.clone(), day.day.clone());
        let last = row + 1 == rows;

        div()
            .id(SharedString::from(format!("cal-day-{}", day.day)))
            .relative()
            .min_h(px(96.))
            .px_2()
            .py(px(6.))
            .bg(paint.heat[usize::from(day.heat).min(3)])
            .when(last && column == 0, |this| this.rounded_bl(px(9.)))
            .when(last && column == 6, |this| this.rounded_br(px(9.)))
            .cursor_pointer()
            .hover(move |style| style.bg(heat_hover))
            // Con su detalle a la vista, `ring-1 ring-primary/70 ring-inset`;
            // con una tarjeta encima, `bg-primary/10 ring-2 ring-primary`.
            .when(open, |this| this.shadow(inset_ring(primary.opacity(0.7), 1.)))
            .aria_label(label)
            .drag_over::<LiftedChip>(move |style, _, _, _| style.bg(primary.opacity(0.1)).shadow(inset_ring(primary, 2.)))
            .on_drop(cx.listener(move |this, lifted: &LiftedChip, _, cx| this.set_due(lifted.chip.id.clone(), Some(drop_day.clone()), cx)))
            .on_hover(cx.listener(move |this, over: &bool, _, cx| this.hover_day(&hover_day, *over, cx)))
            .on_click(cx.listener(move |this, _, window, cx| this.pin_day(&open_day, window, cx)))
            .child(
                div()
                    .min_h(px(84.))
                    .flex()
                    .flex_col()
                    .justify_between()
                    .when(!day.in_month, |this| this.opacity(0.4))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .size(px(22.))
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .text_size(px(13.))
                                    .when(day.today, |this| this.bg(primary).font_weight(FontWeight::SEMIBOLD).text_color(paint.primary_foreground))
                                    .when(!day.today, |this| this.text_color(paint.muted_foreground))
                                    .child(day.number.to_string()),
                            )
                            .when(day.overdue, |this| this.child(div().size(px(7.)).flex_none().rounded_full().bg(paint.error))),
                    )
                    .when(count > 0, |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(5.))
                                .child(
                                    div()
                                        .flex()
                                        .items_baseline()
                                        .gap_1()
                                        .child(div().text_size(px(20.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child(count.to_string()))
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(paint.muted_foreground)
                                                .child(cac_core::text_count_unit(here, COUNT_KEY, count as i64)),
                                        ),
                                )
                                .child(tone_bar(&day.tones, 5., false, paint)),
                        )
                    }),
            )
            .when(open, |this| {
                // Dónde está este día: lo que el detalle necesita, con el
                // alto de la ventana, para saber hacia dónde abrirse.
                // Con sus cuatro lados puestos: sin ellos una pieza absoluta
                // se queda donde le tocaría en el flujo, debajo del número.
                let place = self.calendar.day_place.clone();
                this.child(canvas(move |cell: Bounds<Pixels>, _, _| place.set(Some(cell)), |_, _, _, _| {}).absolute().inset_0())
                    .child(self.render_day_popover(day, column, row, rows, paint, cx))
            })
            .test_support()
    }

    /// El mes (`MonthGrid`): siete columnas de lunes a domingo y las semanas
    /// justas. Las rayas entre días son el fondo que asoma por el hueco de
    /// un píxel; sin recorte en la rejilla, que se llevaría el detalle del
    /// día, así que las esquinas se redondean celda a celda.
    fn render_month(&self, month: &MonthView, paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        let here = locale(cx);
        let rows = month.weeks.len();
        let weekdays = (0..7u32).map(|weekday| {
            div()
                .px_2()
                .py_1()
                .bg(paint.muted)
                .when(weekday == 0, |this| this.rounded_tl(px(9.)))
                .when(weekday == 6, |this| this.rounded_tr(px(9.)))
                .flex()
                .justify_center()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(paint.muted_foreground)
                .child(cac_core::weekday_name(here, weekday, true))
        });
        let mut cells = Vec::with_capacity(rows * 7);
        for (row, week) in month.weeks.iter().enumerate() {
            for (column, day) in week.days.iter().enumerate() {
                cells.push(self.render_day_cell(day, column, row, rows, paint, cx).into_any_element());
            }
        }
        div()
            .id("cal-month")
            .grid()
            .grid_cols(7)
            .gap(px(1.))
            .rounded(px(10.))
            .border_1()
            .border_color(paint.border)
            .bg(paint.border)
            .children(weekdays)
            .children(cells)
            .test_support()
    }

    /// La franja de encima de la semana (`WeekSummary`).
    fn render_week_summary(&self, week: &WeekView, paint: &Paint, cx: &App) -> impl IntoElement {
        let here = locale(cx);
        let summary = &week.summary;
        let block = |label: String, value: String, tone: Option<Hsla>| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .px(px(14.))
                .py(px(10.))
                .bg(paint.muted)
                .child(div().text_size(px(11.)).text_color(paint.muted_foreground).child(label))
                .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).when_some(tone, |this, tone| this.text_color(tone)).child(value))
        };
        div()
            .id("cal-week-summary")
            .flex()
            .flex_none()
            .gap(px(1.))
            .rounded(px(10.))
            .border_1()
            .border_color(paint.border)
            .bg(paint.border)
            .overflow_hidden()
            .child(block(tr(cx, "common:calendar.thisWeek"), cac_core::text_count(here, COUNT_KEY, i64::from(summary.total)), None))
            .child(block(tr(cx, "common:calendar.highPriority"), summary.high.to_string(), Some(paint.warning)))
            .child(block(tr(cx, "common:calendar.overdueOpen"), summary.overdue.to_string(), Some(paint.error)))
            .child(block(tr(cx, "common:calendar.done"), summary.done.to_string(), Some(paint.success)))
            .child(
                div()
                    .flex_grow(2.)
                    .flex_shrink(1.)
                    .flex_basis(relative(0.))
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap_2()
                    .px(px(14.))
                    .py(px(10.))
                    .bg(paint.muted)
                    .child(div().text_size(px(11.)).text_color(paint.muted_foreground).child(tr(cx, "common:calendar.priorityMix")))
                    .child(tone_bar(&summary.tones, 6., true, paint))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(10.))
                            .text_size(px(11.))
                            .text_color(paint.muted_foreground)
                            .children(summary.tones.iter().map(|segment| tone_tally(segment, 6., paint, cx))),
                    ),
            )
            .test_support()
    }

    /// La tarjeta de una tarea en su día o en el carril: se abre con un clic
    /// y se arrastra entera.
    fn render_card(&self, chip: &CalendarChip, lane: bool, dimmed: bool, paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        let in_the_air = self.calendar.lifted.as_deref() == Some(chip.id.as_str());
        let footer = lane.then(|| self.render_lane_shortcuts(chip, paint, cx).into_any_element());
        let card = div()
            .id(SharedString::from(format!("cal-card-{}", chip.id)))
            .w_full()
            .flex_none()
            // La de origen se queda al 40% mientras su copia va en el aire;
            // la de un día que ya pasó, al 75%.
            .when(in_the_air, |this| this.opacity(0.4))
            .when(!in_the_air && dimmed, |this| this.opacity(0.75))
            .child(card_face(chip, lane, footer, paint, cx));
        liftable(opens_task(card, &chip.id), chip, cx.weak_entity()).test_support()
    }

    /// Los atajos de una tarjeta del carril: «→ Hoy» y «Mañana». Pulsarlos
    /// no abre la tarjeta.
    fn render_lane_shortcuts(&self, chip: &CalendarChip, paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        let (primary, primary_foreground, accent) = (paint.primary, paint.primary_foreground, paint.accent);
        let shortcut = |name: &str| {
            div()
                .id(SharedString::from(format!("cal-lane-{name}-{}", chip.id)))
                .flex_none()
                .px_2()
                .py(px(2.))
                .rounded(px(5.))
                .border_1()
                .border_color(paint.border)
                .text_size(px(11.))
                .cursor_pointer()
        };
        let (today_id, tomorrow_id) = (chip.id.clone(), chip.id.clone());
        div()
            .flex()
            .gap_1()
            .child(
                shortcut("today")
                    .hover(move |style| style.border_color(primary).bg(primary).text_color(primary_foreground))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let day = Backend::core(cx).my_work().today().to_string();
                        this.set_due(today_id.clone(), Some(day), cx);
                    }))
                    .child(format!("→ {}", tr(cx, "common:calendar.today")))
                    .test_support(),
            )
            .child(
                shortcut("tomorrow")
                    .hover(move |style| style.bg(accent))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let day = Backend::core(cx).my_work().tomorrow().to_string();
                        this.set_due(tomorrow_id.clone(), Some(day), cx);
                    }))
                    .child(tr(cx, "common:calendar.tomorrow"))
                    .test_support(),
            )
    }

    /// Un día de la semana (`DayColumn`), con sus tarjetas enteras.
    fn render_week_column(&self, day: &CalendarDay, paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        let here = locale(cx);
        let primary = paint.primary;
        let drop_day = day.day.clone();
        // El fin de semana, angosto: de lunes a viernes 1fr, sábado y
        // domingo .62fr.
        let share = if day.weekday >= 5 { WEEKEND_SHARE } else { 1. };
        let empty = day.chips.is_empty().then(|| {
            div()
                .min_h(px(80.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .border_1()
                .border_dashed()
                .border_color(paint.border)
                .text_size(px(12.))
                .text_color(paint.muted_soft)
                .child(tr(cx, "common:calendar.free"))
        });
        div()
            .id(SharedString::from(format!("cal-col-{}", day.day)))
            .flex_grow(share)
            .flex_shrink(1.)
            .flex_basis(relative(0.))
            // Con sitio, se reparten el ancho; sin él, ninguno baja de su
            // mínimo y la semana se desliza.
            .min_w(px(DAY_MIN_WIDTH * share))
            .min_h(px(480.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p_2()
            .rounded(px(10.))
            .border_1()
            .border_color(if day.today { primary.opacity(0.45) } else { paint.border })
            .bg(if day.today { paint.today_column } else { paint.muted })
            .drag_over::<LiftedChip>(move |style, _, _, _| style.border_color(primary).bg(primary.opacity(0.08)))
            .on_drop(cx.listener(move |this, lifted: &LiftedChip, _, cx| this.set_due(lifted.chip.id.clone(), Some(drop_day.clone()), cx)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .gap(px(6.))
                    .px(px(2.))
                    .pt(px(2.))
                    .pb_2()
                    .border_b_1()
                    .border_color(paint.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(paint.muted_foreground)
                                    .child(cac_core::weekday_name(here, day.weekday, true).to_uppercase()),
                            )
                            .child(
                                div()
                                    .h(px(24.))
                                    .min_w(px(24.))
                                    .px_1()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .when(day.today, |this| this.bg(primary).text_color(paint.primary_foreground))
                                    .when(!day.today && day.past, |this| this.text_color(paint.muted_foreground))
                                    .child(day.number.to_string()),
                            )
                            .when(!day.chips.is_empty(), |this| {
                                this.child(
                                    div()
                                        .ml_auto()
                                        .flex_none()
                                        .text_size(px(11.))
                                        .text_color(paint.muted_foreground)
                                        .child(cac_core::text_count(here, "common:calendar.openCount", i64::from(day.open))),
                                )
                            }),
                    )
                    .child(tone_bar(&day.tones, 4., true, paint)),
            )
            .children(day.chips.iter().map(|chip| self.render_card(chip, false, day.past, paint, cx)))
            .children(empty)
            .test_support()
    }

    /// La semana (`WeekView`): una columna por día.
    fn render_week(&self, week: &WeekView, paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        // Sin `items_start`: las siete columnas llegan a la misma altura.
        // El mínimo va también en la fila: es lo que le dice a quien la
        // desliza hasta dónde llega, que los días se salgan no basta.
        div().id("cal-week").min_w(px(WEEK_MIN_WIDTH)).flex().gap(px(WEEK_GAP)).children(week.days.iter().map(|day| self.render_week_column(day, paint, cx))).test_support()
    }

    /// «Por programar» (`ScheduleLane`): lo que no tiene fecha, a un lado,
    /// para dársela arrastrando o con un atajo. Soltar aquí una con fecha se
    /// la quita.
    fn render_lane(&self, view: CalendarView, chips: &[CalendarChip], paint: &Paint, cx: &mut Context<Self>) -> impl IntoElement {
        let (primary, accent, foreground) = (paint.primary, paint.accent, paint.foreground);
        let empty = chips
            .is_empty()
            .then(|| div().px(px(2.)).py(px(10.)).text_size(px(12.)).text_color(paint.success).child(tr(cx, "common:calendar.allDated")));
        div()
            .id("cal-lane")
            .w(px(LANE_WIDTH))
            .max_h_full()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(6.))
            .p_2()
            .rounded(px(10.))
            .border_1()
            .border_dashed()
            .border_color(primary.opacity(0.4))
            .bg(primary.opacity(0.03))
            .drag_over::<LiftedChip>(move |style, _, _, _| style.border_color(primary).bg(primary.opacity(0.08)))
            // Una que ya no tenía fecha se queda como está: el core no pide
            // nada si el día no cambia.
            .on_drop(cx.listener(|this, lifted: &LiftedChip, _, cx| this.set_due(lifted.chip.id.clone(), None, cx)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .px(px(2.))
                    .pt(px(2.))
                    .pb(px(6.))
                    .border_b_1()
                    .border_color(paint.border)
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(primary).child(tr(cx, "common:calendar.lane")))
                    .child(
                        div()
                            .px(px(6.))
                            .rounded_full()
                            .bg(paint.track)
                            .text_size(px(11.))
                            .text_color(paint.muted_foreground)
                            .child(chips.len().to_string()),
                    )
                    .child(
                        div()
                            .id("cal-lane-close")
                            .ml_auto()
                            .size(px(22.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .text_color(paint.muted_foreground)
                            .cursor_pointer()
                            .hover(move |style| style.bg(accent).text_color(foreground))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_lane_open(view, false, cx)))
                            .child(Icon::new(Lucide::ChevronLeft).size_3p5())
                            .test_support(),
                    ),
            )
            // Las tarjetas se deslizan dentro del carril: con muchas sin
            // fecha, el carril no estira la página ni se lleva el
            // calendario hacia arriba.
            .child(
                div()
                    .id("cal-lane-cards")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .overflow_y_scroll()
                    .children(chips.iter().map(|chip| self.render_card(chip, true, false, paint, cx)))
                    .children(empty)
                    .test_support(),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(2.))
                    .pt_1()
                    .pb(px(2.))
                    .text_size(px(11.))
                    .line_height(px(15.))
                    .text_color(paint.muted_soft)
                    .child(tr(cx, "common:calendar.laneHelp")),
            )
            .test_support()
    }

    /// El calendario entero (`ItemCalendar.tsx`): la cabecera, el resumen si
    /// se mira la semana, y el carril al lado del mes o de la semana.
    pub(crate) fn render_calendar(&mut self, filter: &TaskFilter, cx: &mut Context<Self>) -> impl IntoElement {
        // Soltar fuera de todo día no avisa a nadie: sin nada en el aire,
        // ninguna tarjeta sigue levantada.
        if self.calendar.lifted.is_some() && !cx.has_active_drag() {
            self.calendar.lifted = None;
        }
        // Con una tarea abierta, el detalle de un día se cierra: se pinta por
        // encima de todo, y se quedaba flotando sobre la página de la tarea
        // que se acababa de abrir desde él (jose, 17-nov-2026).
        if Backend::snapshot(cx).read(cx).task.task_id.is_some() {
            self.calendar.forget_day();
        }
        let paint = Paint::of(cx);
        let here = locale(cx);
        let cursor = self.calendar_cursor(cx);
        let work = Backend::core(cx).my_work();
        let today = work.today();
        let unscheduled = work.unscheduled(filter);
        let lane_open = self.lane_is_open(cursor.view);
        let error = self.calendar.error.clone();

        let (title, summary, body) = match cursor.view {
            CalendarView::Month => {
                let month = work.month_view(filter, here, cursor.month_day().unwrap_or(today));
                (month.title.clone(), None, self.render_month(&month, &paint, cx).into_any_element())
            }
            CalendarView::Week => {
                let week = work.week_view(filter, here, cursor.week_day().unwrap_or(today));
                let summary = self.render_week_summary(&week, &paint, cx).into_any_element();
                (week.title.clone(), Some(summary), self.render_week(&week, &paint, cx).into_any_element())
            }
        };
        let week = cursor.view == CalendarView::Week;
        let header = self.render_calendar_header(&cursor, title, unscheduled.len(), &paint, cx);
        let lane = lane_open.then(|| self.render_lane(cursor.view, &unscheduled, &paint, cx));

        div()
            .id("work-calendar")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p_4()
            .line_height(relative(LEADING))
            .children(error.map(|error| div().id("calendar-error").flex_none().leading_sm().text_color(paint.error).child(error).test_support()))
            .child(header)
            .children(summary)
            // El carril y el calendario se deslizan cada uno por su lado,
            // dentro de lo que queda de pantalla. La semana, además, de
            // lado cuando sus siete días no caben: el resumen de encima y
            // el carril se quedan donde están, y la cabecera de cada día
            // va con su columna.
            .child(
                div().flex_1().min_h_0().flex().items_start().gap_3().min_w_0().children(lane).child(
                    div()
                        .id("cal-body")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .when(week, |this| this.overflow_x_scroll())
                        .child(body)
                        .test_support(),
                ),
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
    use chrono::{Datelike, Days, Months, NaiveDate};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    /// Si eso está pintado dentro de aquello.
    fn inside(window: AnyWindowHandle, child: &str, parent: &str, cx: &mut TestAppContext) -> bool {
        let (child, parent) = (gpui_kit::SharedString::from(child.to_string()), gpui_kit::SharedString::from(parent.to_string()));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within(parent).try_find(child).is_some()
        })
        .unwrap()
    }

    /// Lo que dice un elemento de sí mismo: el día de un mes dice su fecha y
    /// cuánto tiene.
    fn label(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).label().unwrap_or_default().to_string()
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

    fn drag(window: AnyWindowHandle, from: &str, to: &str, cx: &mut TestAppContext) {
        let (from, to) = (gpui_kit::SharedString::from(from.to_string()), gpui_kit::SharedString::from(to.to_string()));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.drag_to(from, to, cx);
        })
        .unwrap();
    }

    fn press(window: AnyWindowHandle, key: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| window.press(key, cx)).unwrap();
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Deja pasar un rato para lo que **no** tiene que ocurrir.
    fn settle(cx: &mut TestAppContext) {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(150));
        cx.run_until_parked();
    }

    fn opened_task(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.clone())
    }

    /// Lo que se le pidió cambiar a esa tarea, en orden.
    fn patches(server: &FakeServer, id: &str) -> Vec<Value> {
        let route = format!("/api/v1/tasks/{id}");
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() == "PATCH" && request.url.path() == route)
            .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
            .collect()
    }

    /// Los días con los que se arman las pruebas: hoy, que es donde vence
    /// `t1`, y otro de la misma semana donde soltarla.
    struct Days2 {
        today: NaiveDate,
        other: NaiveDate,
    }

    impl Days2 {
        fn col(day: NaiveDate) -> String {
            format!("cal-col-{day}")
        }
        fn cell(day: NaiveDate) -> String {
            format!("cal-day-{day}")
        }
    }

    /// El calendario abierto, en el mes, con `t1` venciendo hoy (de Ana,
    /// urgente) y `t2` sin fecha. «Hoy» es el del core, que es el que pinta;
    /// las tareas se montan después de saberlo y por delante de la lista
    /// vacía del servidor de base.
    fn show_calendar(cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle, Days2) {
        show_calendar_with(vec![], cx)
    }

    /// Lo mismo, con más tareas además de las dos de siempre.
    fn show_calendar_with(more: Vec<Value>, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle, Days2) {
        show_calendar_sized(more, 1700., 1000., cx)
    }

    /// Lo mismo, en una ventana de esa medida.
    fn show_calendar_sized(more: Vec<Value>, width: f32, height: f32, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle, Days2) {
        let server = server_with_tasks(vec![]);
        server.install(cx);
        let today = cx.update(|cx| Backend::core(cx).my_work().today());
        let monday = cac_core::week_start(today);
        let other = if today == monday { monday + Days::new(1) } else { monday };
        let mut tasks = vec![
            contract_task("t1", "a", |task| {
                task["title"] = json!("Con fecha");
                task["priority"] = json!("urgent");
                task["dueAt"] = json!(format!("{today}T00:00:00Z"));
            }),
            contract_task("t2", "a", |task| {
                task["title"] = json!("Sin fecha");
                task["dueAt"] = Value::Null;
            }),
        ];
        tasks.extend(more);
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/")).respond_with(ok(Value::Array(tasks))).with_priority(1));
        sign_in(cx);
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(width), px(height)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window").0
        });
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "layout-calendar", cx);
        wait_until(cx, "sale el calendario", |cx| on_screen(window, "work-calendar", cx));
        (server, window, Days2 { today, other })
    }

    fn show_week(cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle, Days2) {
        let shown = show_calendar(cx);
        click(shown.1, "cal-view-week", cx);
        assert!(on_screen(shown.1, "cal-week", cx));
        shown
    }

    /// Dónde está pintado eso: izquierda, derecha, arriba y abajo.
    fn edges(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> (f32, f32, f32, f32) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let bounds = window.find(id).bounds();
            (f32::from(bounds.left()), f32::from(bounds.right()), f32::from(bounds.top()), f32::from(bounds.bottom()))
        })
        .unwrap()
    }

    /// En una ventana estrecha los días de la semana no bajan de su mínimo:
    /// la semana se desliza de lado dentro de su hueco, hasta el domingo. En
    /// una ancha se reparten el ancho entero, como siempre.
    ///
    /// Roturas que atrapa: siete columnas de 47 px en las que no se lee nada
    /// (lo que jose encontró el 17-nov-2026); una semana que se sale por la
    /// derecha sin forma de llegar al domingo; unas columnas que, con sitio
    /// de sobra, se quedan en su mínimo y dejan media pantalla vacía.
    #[gpui_kit::test]
    fn a_day_of_the_week_is_never_narrower_than_it_can_be_read(cx: &mut TestAppContext) {
        let (_server, window, days) = show_calendar_sized(vec![], 900., 800., cx);
        click(window, "cal-view-week", cx);
        let monday = cac_core::week_start(days.today);
        let (first, saturday, last) = (Days2::col(monday), Days2::col(monday + Days::new(5)), Days2::col(monday + Days::new(6)));
        let width = |id: &str, cx: &mut TestAppContext| {
            let (left, right, ..) = edges(window, id, cx);
            right - left
        };

        assert!(width(&first, cx) >= 140., "el lunes mide {} de ancho", width(&first, cx));
        assert!(width(&saturday, cx) >= 86., "el sábado mide {} de ancho", width(&saturday, cx));
        let (body_left, body_right, ..) = edges(window, "cal-body", cx);
        assert!(body_right - body_left < 984., "la ventana de la prueba no es estrecha: la semana cabe");
        assert!(edges(window, &last, cx).1 > body_right, "el domingo cabe sin deslizar");
        // El resumen y el carril no se van con ella.
        let (summary, lane) = (edges(window, "cal-week-summary", cx), edges(window, "cal-lane", cx));

        cx.update_window(window, |_, window, cx| {
            window.scroll("cal-body", gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(-2000.), px(0.))), cx);
        })
        .unwrap();
        let sunday = edges(window, &last, cx);
        assert!((sunday.1 - body_right).abs() < 1., "deslizando hasta el final el domingo acaba en {} y el hueco en {body_right}", sunday.1);
        assert_eq!((edges(window, "cal-week-summary", cx), edges(window, "cal-lane", cx)), (summary, lane), "el resumen o el carril se deslizaron con la semana");
        stop(cx);
    }

    /// Ver el comentario de la prueba anterior: ésta es la mitad ancha.
    #[gpui_kit::test]
    fn with_room_the_days_of_the_week_share_the_whole_width(cx: &mut TestAppContext) {
        let (_server, window, days) = show_week(cx);
        let monday = cac_core::week_start(days.today);
        let (first, last) = (edges(window, &Days2::col(monday), cx), edges(window, &Days2::col(monday + Days::new(6)), cx));
        let body = edges(window, "cal-body", cx);

        assert!((first.0 - body.0).abs() < 1. && (last.1 - body.1).abs() < 1., "la semana va de {} a {} y su hueco de {} a {}", first.0, last.1, body.0, body.1);
        assert!(first.1 - first.0 > 140., "con sitio, el lunes se queda en su mínimo");
        // El fin de semana, angosto: 0,62 de un día de diario (los dos
        // llevan además su relleno y su borde, que no se reparten).
        let share = (last.1 - last.0) / (first.1 - first.0);
        assert!((0.6..0.7).contains(&share), "el domingo mide {share} de lo que mide el lunes");
        stop(cx);
    }

    /// Abre el detalle de ese día y dice hacia dónde quedó: `true`, hacia
    /// arriba (acaba ocho píxeles por debajo de su día); `false`, hacia
    /// abajo (empieza ocho por encima).
    fn detail_opens_up(window: AnyWindowHandle, day: NaiveDate, cx: &mut TestAppContext) -> bool {
        click(window, &Days2::cell(day), cx);
        // La primera pintada lo mide; la siguiente lo pone en su sitio.
        for _ in 0..3 {
            on_screen(window, "cal-day-popover", cx);
            cx.run_until_parked();
        }
        let (cell, detail) = (edges(window, &Days2::cell(day), cx), edges(window, "cal-day-popover", cx));
        let (down, up) = ((detail.2 - (cell.2 - 8.)).abs() < 1., (detail.3 - (cell.3 + 8.)).abs() < 1.);
        assert!(down != up, "el detalle va de {} a {} y su día de {} a {}: no está anclado a él", detail.2, detail.3, cell.2, cell.3);
        up
    }

    /// Cuatro tareas que vencen cada uno de esos días de este mes: con ellas
    /// el detalle del día mide unos 315 px de alto.
    fn four_due_on(numbers: &[u32]) -> Vec<Value> {
        due_on(numbers, 4)
    }

    /// Tantas tareas que vencen cada uno de esos días de este mes.
    fn due_on(numbers: &[u32], each: usize) -> Vec<Value> {
        let today = chrono::Local::now().date_naive();
        let mut tasks = Vec::new();
        for number in numbers {
            let day = today.with_day(*number).unwrap();
            for n in 0..each {
                tasks.push(contract_task(&format!("d{number}-{n}"), "a", |task| {
                    task["title"] = json!(format!("Vence el {number}, {n}"));
                    task["dueAt"] = json!(format!("{day}T00:00:00Z"));
                }));
            }
        }
        tasks
    }

    /// En una ventana baja, el detalle de un día de en medio del mes se abre
    /// hacia arriba, que es donde cabe.
    ///
    /// Rotura que atrapa: decidirlo por la fila del mes (hacia arriba sólo
    /// en las dos últimas), que en una ventana baja deja el detalle de un
    /// día de en medio saliéndose por abajo, donde no se alcanza.
    #[gpui_kit::test]
    fn in_a_short_window_the_detail_of_a_middle_day_opens_upward(cx: &mut TestAppContext) {
        let (_server, window, days) = show_calendar_sized(four_due_on(&[8]), 1700., 470., cx);
        // El día 8 cae siempre en la segunda fila del mes.
        let day = days.today.with_day(8).unwrap();
        let (cell, body) = (edges(window, &Days2::cell(day), cx), edges(window, "cal-body", cx));
        assert!(cell.2 > body.2 && cell.3 < body.3, "la ventana de la prueba no deja ver entero el día 8: va de {} a {} y lo que se ve de {} a {}", cell.2, cell.3, body.2, body.3);

        assert!(detail_opens_up(window, day, cx), "se abre hacia abajo sin caber");
        let detail = edges(window, "cal-day-popover", cx);
        assert!(detail.2 >= 0. && detail.3 <= 470., "el detalle va de {} a {} y la ventana mide 470 de alto", detail.2, detail.3);
        stop(cx);
    }

    /// Fila a fila del mes, el detalle de un día se abre hacia donde el core
    /// dice con lo que de verdad miden su día, él y la ventana.
    ///
    /// Roturas que atrapa: medir mal dónde está el día (noventa píxeles más
    /// abajo, que es donde cae una pieza de medir sin sus cuatro lados
    /// puestos: pasó al escribirlo), o cuánto mide el detalle, y abrirlo
    /// hacia arriba cuando aún cabía abajo.
    #[gpui_kit::test]
    fn the_detail_of_each_row_opens_where_its_real_measures_say(cx: &mut TestAppContext) {
        const HEIGHT: f32 = 680.;
        let (_server, window, days) = show_calendar_sized(four_due_on(&[1, 8, 15, 22]), 1700., HEIGHT, cx);
        let mut seen = Vec::new();
        // Los días 1, 8, 15 y 22 caen siempre en las cuatro primeras filas.
        for number in [1, 8, 15, 22] {
            let day = days.today.with_day(number).unwrap();
            let up = detail_opens_up(window, day, cx);
            let (cell, detail) = (edges(window, &Days2::cell(day), cx), edges(window, "cal-day-popover", cx));
            let expected = cac_core::day_detail_opens_up(cell.2 - 8., cell.3 + 8., detail.3 - detail.2, 0., HEIGHT);
            assert_eq!(up, expected, "el día {number} va de {} a {} y su detalle mide {}", cell.2, cell.3, detail.3 - detail.2);
            seen.push((number, up, cell.2));
            click(window, "cal-day-close", cx);
        }
        assert!(seen.iter().any(|(_, up, _)| *up) && seen.iter().any(|(_, up, _)| !*up), "la ventana de la prueba no pone los dos casos: {seen:?}");
        stop(cx);
    }

    /// Un detalle que no cabe ni hacia abajo ni hacia arriba se abre hacia
    /// donde menos se le esconde, contando desde donde de verdad acabaría.
    ///
    /// Rotura que atrapa: medir «hacia arriba» desde el borde de arriba del
    /// día y no desde el de abajo, que es donde acaba el detalle: se le
    /// calculan cien píxeles escondidos de más y se queda hacia abajo,
    /// tapando más de lo necesario.
    #[gpui_kit::test]
    fn a_detail_that_fits_nowhere_opens_where_less_of_it_is_hidden(cx: &mut TestAppContext) {
        const HEIGHT: f32 = 540.;
        let (_server, window, days) = show_calendar_sized(due_on(&[1], 6), 1700., HEIGHT, cx);
        let day = days.today.with_day(1).unwrap();

        let up = detail_opens_up(window, day, cx);
        let (cell, detail) = (edges(window, &Days2::cell(day), cx), edges(window, "cal-day-popover", cx));
        let height = detail.3 - detail.2;
        let (down_hidden, up_hidden) = (cell.2 - 8. + height - HEIGHT, height - (cell.3 + 8.));
        assert!(up_hidden > 0. && down_hidden > up_hidden && down_hidden < up_hidden + 90., "la ventana de la prueba ya no pone el caso: hacia abajo se esconden {down_hidden} y hacia arriba {up_hidden}");
        assert!(up, "se abre hacia abajo, donde se le esconde más");
        stop(cx);
    }

    /// En una ventana alta, el detalle de un día de la última fila se abre
    /// hacia abajo: cabe.
    ///
    /// Rotura que atrapa: abrirlo hacia arriba sin necesidad (por la fila, o
    /// por medir mal el sitio), tapando las semanas de encima.
    #[gpui_kit::test]
    fn in_a_tall_window_the_detail_of_a_day_of_the_last_row_opens_downward(cx: &mut TestAppContext) {
        let (_server, window, days) = show_calendar_sized(vec![], 1700., 1400., cx);
        let last = days.today.with_day(1).unwrap() + Months::new(1) - Days::new(1);

        assert!(!detail_opens_up(window, last, cx), "se abre hacia arriba cabiendo hacia abajo");
        stop(cx);
    }

    /// Elegir «Calendario» cambia la lista por el mes, con cada tarea
    /// contada en el día en que vence y sin sus títulos dentro de la celda.
    ///
    /// Roturas que atrapa: un selector que sigue enseñando la lista; un mes
    /// que no cuenta lo que vence (o lo cuenta en todos los días); el carril
    /// abierto en el mes, que necesita el ancho.
    #[gpui_kit::test]
    fn the_calendar_counts_each_task_on_the_day_it_is_due(cx: &mut TestAppContext) {
        let (_server, window, days) = show_calendar(cx);

        assert!(!on_screen(window, "work-row-t1", cx), "la lista sigue debajo del calendario");
        assert!(on_screen(window, "cal-month", cx));
        let today = label(window, &Days2::cell(days.today), cx);
        assert!(today.ends_with("· 1 tarea"), "hoy dice «{today}»");
        let other = label(window, &Days2::cell(days.other), cx);
        assert!(other.ends_with("· 0 tareas"), "el otro día dice «{other}»");
        assert!(!on_screen(window, "cal-card-t1", cx), "el mes pinta tarjetas dentro de un día");
        assert!(!on_screen(window, "cal-lane", cx), "el carril sale abierto en el mes");
        stop(cx);
    }

    /// En la semana cada tarea es una tarjeta en la columna de su día, y lo
    /// que no tiene fecha está en «Por programar», que ahí sale abierto y se
    /// puede cerrar.
    ///
    /// Roturas que atrapa: Mes / Semana que no cambia nada; todas las
    /// tarjetas en la primera columna; lo que no tiene fecha perdido; un
    /// botón del carril que no lo abre ni lo cierra.
    #[gpui_kit::test]
    fn the_week_puts_each_card_in_the_column_of_its_day(cx: &mut TestAppContext) {
        let (_server, window, days) = show_week(cx);

        assert!(!on_screen(window, "cal-month", cx), "el mes sigue debajo de la semana");
        assert!(on_screen(window, "cal-week-summary", cx));
        assert!(inside(window, "cal-card-t1", &Days2::col(days.today), cx));
        assert!(!inside(window, "cal-card-t1", &Days2::col(days.other), cx), "la tarjeta sale en un día que no es el suyo");
        assert!(inside(window, "cal-card-t2", "cal-lane", cx), "lo que no tiene fecha no está en el carril");
        assert!(!inside(window, "cal-card-t2", "cal-week", cx));

        click(window, "cal-lane-close", cx);
        assert!(!on_screen(window, "cal-lane", cx), "el carril no se cierra");
        click(window, "cal-lane-toggle", cx);
        assert!(on_screen(window, "cal-lane", cx), "el carril no se vuelve a abrir");

        // Lo elegido es de la semana: el mes sigue con el suyo, cerrado.
        click(window, "cal-view-month", cx);
        assert!(on_screen(window, "cal-month", cx) && !on_screen(window, "cal-lane", cx));
        click(window, "cal-lane-toggle", cx);
        assert!(inside(window, "cal-card-t2", "cal-lane", cx), "el botón del carril no lo abre en el mes");
        stop(cx);
    }

    /// ‹ y › cambian el mes o la semana que se mira, y «Hoy» vuelve.
    ///
    /// Roturas que atrapa: flechas que no mueven nada, o que van las dos
    /// hacia el mismo lado; un «Hoy» que se queda donde estaba; flechas que
    /// en la semana saltan un mes.
    #[gpui_kit::test]
    fn the_arrows_and_today_move_what_is_shown(cx: &mut TestAppContext) {
        let (_server, window, days) = show_calendar(cx);
        let mid = |month: NaiveDate| Days2::cell(month.with_day(15).unwrap());
        let (this, next, previous) =
            (mid(days.today), mid(days.today.with_day(1).unwrap() + Months::new(1)), mid(days.today.with_day(1).unwrap() - Months::new(1)));
        assert!(on_screen(window, &this, cx));

        click(window, "cal-next", cx);
        assert!(on_screen(window, &next, cx) && !on_screen(window, &this, cx), "› no pasó al mes siguiente");
        click(window, "cal-prev", cx);
        click(window, "cal-prev", cx);
        assert!(on_screen(window, &previous, cx) && !on_screen(window, &this, cx), "‹ no pasó al mes anterior");
        click(window, "cal-today", cx);
        assert!(on_screen(window, &this, cx) && !on_screen(window, &previous, cx), "«Hoy» no volvió al mes de hoy");

        click(window, "cal-view-week", cx);
        let (now, after, before) = (Days2::col(days.today), Days2::col(days.today + Days::new(7)), Days2::col(days.today - Days::new(7)));
        assert!(on_screen(window, &now, cx));
        click(window, "cal-next", cx);
        assert!(on_screen(window, &after, cx) && !on_screen(window, &now, cx), "› no pasó a la semana siguiente");
        click(window, "cal-prev", cx);
        click(window, "cal-prev", cx);
        assert!(on_screen(window, &before, cx) && !on_screen(window, &now, cx), "‹ no pasó a la semana anterior");
        click(window, "cal-today", cx);
        assert!(on_screen(window, &now, cx) && !on_screen(window, &before, cx), "«Hoy» no volvió a esta semana");
        stop(cx);
    }

    /// Arrastrar una tarjeta a otro día le pone ese día a esa tarea.
    ///
    /// Roturas que atrapa: una columna que no recibe lo que se suelta; soltar
    /// que no pide nada; pedir el cambio para otra tarea o con otro día.
    #[gpui_kit::test]
    fn dragging_a_card_to_another_day_sets_that_day(cx: &mut TestAppContext) {
        let (server, window, days) = show_week(cx);
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(json!({}))));

        drag(window, "cal-card-t1", &Days2::col(days.other), cx);
        wait_until(cx, "se pide el cambio", |_| !patches(&server, "t1").is_empty());
        assert_eq!(patches(&server, "t1"), [json!({ "dueAt": format!("{}T00:00:00Z", days.other) })]);
        assert!(inside(window, "cal-card-t1", &Days2::col(days.other), cx), "la tarjeta no está en el día donde se soltó");
        assert!(!on_screen(window, "calendar-error", cx));
        assert!(patches(&server, "t2").is_empty());
        stop(cx);
    }

    /// Soltar una tarjeta en «Por programar» le quita la fecha; una que ya
    /// no la tenía se queda como está, sin pedir nada.
    ///
    /// Roturas que atrapa: un carril que no recibe; quitar la fecha con
    /// `dueAt: null`, que el servidor lee como «no tocar»; un PATCH por
    /// soltar en el carril lo que ya estaba en él.
    #[gpui_kit::test]
    fn dropping_on_the_lane_clears_the_date(cx: &mut TestAppContext) {
        let (server, window, _days) = show_week(cx);
        for id in ["t1", "t2"] {
            server.mount(Mock::given(method("PATCH")).and(path(format!("/api/v1/tasks/{id}"))).respond_with(ok(json!({}))));
        }

        drag(window, "cal-card-t1", "cal-lane", cx);
        wait_until(cx, "se pide quitar la fecha", |_| !patches(&server, "t1").is_empty());
        assert_eq!(patches(&server, "t1"), [json!({ "clearDueAt": true })]);
        assert!(inside(window, "cal-card-t1", "cal-lane", cx), "la tarjeta no pasó al carril");

        drag(window, "cal-card-t2", "cal-lane", cx);
        settle(cx);
        assert!(patches(&server, "t2").is_empty(), "se pidió quitar una fecha que no había");
        assert!(!on_screen(window, "calendar-error", cx));
        stop(cx);
    }

    /// Si el servidor no acepta el cambio, se dice y la tarjeta vuelve a su
    /// día.
    ///
    /// Roturas que atrapa: un rechazo que se traga en silencio (la tarjeta
    /// vuelve sola y nadie sabe por qué).
    #[gpui_kit::test]
    fn a_change_the_server_refuses_is_said(cx: &mut TestAppContext) {
        let (server, window, days) = show_week(cx);
        let refusal = ResponseTemplate::new(409).set_body_json(json!({ "success": false, "error": "forbidden", "message": "No puedes" }));
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(refusal));

        drag(window, "cal-card-t1", &Days2::col(days.other), cx);
        wait_until(cx, "se dice el rechazo", |cx| on_screen(window, "calendar-error", cx));
        assert!(inside(window, "cal-card-t1", &Days2::col(days.today), cx), "la tarjeta no volvió a su día");
        stop(cx);
    }

    /// Pulsar una tarjeta abre su tarea, en la semana y en el carril.
    ///
    /// Rotura que atrapa: una tarjeta que sólo se deja arrastrar.
    #[gpui_kit::test]
    fn clicking_a_card_opens_the_task(cx: &mut TestAppContext) {
        let (_server, window, _days) = show_week(cx);

        click(window, "cal-card-t1", cx);
        wait_until(cx, "se abre la tarea", |cx| opened_task(cx).as_deref() == Some("t1"));
        stop(cx);
    }

    /// Aparte de la anterior: una tarea abierta ocupa la pantalla.
    #[gpui_kit::test]
    fn clicking_a_card_of_the_lane_opens_the_task(cx: &mut TestAppContext) {
        let (_server, window, _days) = show_week(cx);

        click(window, "cal-card-t2", cx);
        wait_until(cx, "se abre la del carril", |cx| opened_task(cx).as_deref() == Some("t2"));
        stop(cx);
    }

    /// «Nueva tarea para este día», en el detalle de un día, abre la fila de
    /// tarea nueva con ese día puesto: lo que se crea nace con él.
    ///
    /// Roturas que atrapa: un enlace que no abre nada; una fila que nace sin
    /// fecha (o con la de otro día); un detalle que se queda abierto encima.
    #[gpui_kit::test]
    fn a_new_task_from_a_day_is_born_due_that_day(cx: &mut TestAppContext) {
        let (server, window, days) = show_calendar(cx);
        let created = json!({ "id": "t9", "createdAt": "2026-11-15T10:00:00Z", "updatedAt": "2026-11-15T10:00:00Z",
            "listId": "l-a", "seq": 40, "title": "x", "status": "pending", "priority": "medium" });
        server.mount(Mock::given(method("POST")).and(wiremock::matchers::path_regex("^/api/v1/task-lists/[^/]+/tasks$")).respond_with(ok(created)));
        let posted = |server: &FakeServer| -> Vec<Value> {
            server
                .requests()
                .iter()
                .filter(|request| request.method.as_str() == "POST" && request.url.path().ends_with("/tasks"))
                .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
                .collect()
        };

        click(window, &Days2::cell(days.other), cx);
        click(window, "cal-new-for-day", cx);
        assert!(on_screen(window, "new-task-row", cx), "no se abre la fila de tarea nueva");
        assert!(!on_screen(window, "cal-day-popover", cx), "el detalle sigue abierto");
        cx.update_window(window, |_, window, cx| {
            window.click("new-task-title", cx);
            window.input("Para ese día", cx);
        })
        .unwrap();
        press(window, "enter", cx);
        wait_until(cx, "se crea", |_| !posted(&server).is_empty());
        assert_eq!(posted(&server)[0]["dueAt"], json!(format!("{}T00:00:00Z", days.other)));
        stop(cx);
    }

    /// Pulsar un día del mes abre su detalle, con una fila por tarea; Esc lo
    /// cierra, y arrastrar una fila a otro día le pone ese día.
    ///
    /// Roturas que atrapa: un día que no enseña lo que tiene; un detalle que
    /// no se puede cerrar con el teclado; filas que no se pueden arrastrar,
    /// o un detalle que se queda abierto tapando los días donde soltar; un
    /// día del mes que no recibe lo que se suelta.
    #[gpui_kit::test]
    fn a_day_of_the_month_opens_its_detail(cx: &mut TestAppContext) {
        let (server, window, days) = show_calendar(cx);
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(json!({}))));
        assert!(!on_screen(window, "cal-day-popover", cx));

        click(window, &Days2::cell(days.today), cx);
        assert!(inside(window, "cal-row-t1", "cal-day-popover", cx), "el detalle no trae la tarea del día");
        press(window, "escape", cx);
        assert!(!on_screen(window, "cal-day-popover", cx), "Esc no cierra el detalle");

        click(window, &Days2::cell(days.today), cx);
        click(window, "cal-day-close", cx);
        assert!(!on_screen(window, "cal-day-popover", cx), "la × no cierra el detalle");

        click(window, &Days2::cell(days.today), cx);
        drag(window, "cal-row-t1", &Days2::cell(days.other), cx);
        wait_until(cx, "se pide el cambio", |_| !patches(&server, "t1").is_empty());
        assert_eq!(patches(&server, "t1"), [json!({ "dueAt": format!("{}T00:00:00Z", days.other) })]);
        assert!(!on_screen(window, "cal-day-popover", cx), "el detalle sigue abierto tras arrastrar");
        assert!(label(window, &Days2::cell(days.other), cx).ends_with("· 1 tarea"));
        stop(cx);
    }

    fn hover(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.hover(id, cx);
        })
        .unwrap();
    }

    /// Deja pasar ese rato del reloj de la app.
    fn wait(cx: &mut TestAppContext, millis: u64) {
        cx.executor().advance_clock(std::time::Duration::from_millis(millis));
        cx.run_until_parked();
    }

    /// El detalle de un día se asoma con el ratón un rato encima y se va al
    /// salir; un clic lo fija, y fijado ya no se va con el ratón.
    ///
    /// Roturas que atrapa: un detalle que sólo sale con clic (jose, 16-nov:
    /// «el hover no abre el quick del día»); uno que sale al rozar la celda,
    /// sin esperar; uno asomado que se queda al irse el ratón; uno que se
    /// cierra solo al pasar de la celda a él; uno fijado que se va con el
    /// ratón; el asomado con la × del fijado.
    #[gpui_kit::test]
    fn hovering_a_day_peeks_its_detail_and_a_click_pins_it(cx: &mut TestAppContext) {
        let (_server, window, days) = show_calendar(cx);
        let cell = Days2::cell(days.today);

        hover(window, &cell, cx);
        wait(cx, 100);
        assert!(!on_screen(window, "cal-day-popover", cx), "el detalle sale sin esperar");
        wait(cx, 200);
        assert!(inside(window, "cal-row-t1", "cal-day-popover", cx), "el ratón encima no asoma el detalle");
        assert!(on_screen(window, "cal-click-to-pin", cx));
        assert!(!on_screen(window, "cal-day-close", cx), "asomado y ya con la ×");

        // El detalle tapa su celda: el ratón pasa a estar sobre él.
        hover(window, "cal-day-popover", cx);
        wait(cx, 300);
        assert!(on_screen(window, "cal-day-popover", cx), "se cierra al pasar de la celda al detalle");

        hover(window, "cal-view-week", cx);
        wait(cx, 300);
        assert!(!on_screen(window, "cal-day-popover", cx), "el asomado se queda al irse el ratón");

        hover(window, &cell, cx);
        wait(cx, 300);
        // El clic en el día cae ya sobre el detalle, que lo tapa.
        click(window, &cell, cx);
        assert!(on_screen(window, "cal-day-close", cx), "un clic no lo fija");
        hover(window, "cal-view-week", cx);
        wait(cx, 300);
        assert!(on_screen(window, "cal-day-popover", cx), "fijado y se va con el ratón");
        stop(cx);
    }

    /// Con muchas tareas sin fecha, el carril se desliza por dentro: no se
    /// sale de la pantalla ni estira la página (jose, 16-nov: al bajar por
    /// el carril el calendario se quedaba arriba).
    ///
    /// Rotura que atrapa: un carril tan alto como todas sus tarjetas.
    #[gpui_kit::test]
    fn a_long_lane_scrolls_inside_itself(cx: &mut TestAppContext) {
        let undated = (0..40)
            .map(|at| {
                contract_task(&format!("u{at}"), "a", |task| {
                    task["title"] = json!(format!("Sin fecha {at}"));
                    task["dueAt"] = Value::Null;
                })
            })
            .collect();
        let (_server, window, _) = show_calendar_with(undated, cx);
        click(window, "cal-lane-toggle", cx);
        assert!(on_screen(window, "cal-lane", cx));

        let bottom = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("cal-lane").bounds().bottom()
            })
            .unwrap();
        assert!(bottom <= px(1000.), "el carril se sale de la ventana: acaba en {bottom:?}");
        stop(cx);
    }

    /// Pulsar una fila del detalle abre su tarea, y pulsar fuera lo cierra.
    ///
    /// Roturas que atrapa: una fila que no abre nada; un detalle que sólo
    /// se va con la × o con Esc; un detalle que se queda flotando sobre la
    /// página de la tarea que se abrió desde él.
    #[gpui_kit::test]
    fn a_row_of_the_detail_opens_the_task_and_a_click_outside_closes_it(cx: &mut TestAppContext) {
        let (_server, window, days) = show_calendar(cx);

        click(window, &Days2::cell(days.today), cx);
        click(window, "cal-title", cx);
        assert!(!on_screen(window, "cal-day-popover", cx), "pulsar fuera no cierra el detalle");

        click(window, &Days2::cell(days.today), cx);
        click(window, "cal-row-t1", cx);
        wait_until(cx, "se abre la tarea", |cx| opened_task(cx).as_deref() == Some("t1"));
        assert!(!on_screen(window, "cal-day-popover", cx), "el detalle del día sigue abierto sobre la tarea");
        stop(cx);
    }

    /// «→ Hoy» y «Mañana» le ponen fecha a una tarjeta del carril sin
    /// abrirla.
    ///
    /// Roturas que atrapa: un atajo que no pide nada; «Mañana» que pone
    /// hoy; un atajo que además abre la tarea.
    #[gpui_kit::test]
    fn the_lane_shortcuts_set_today_and_tomorrow(cx: &mut TestAppContext) {
        let (server, window, days) = show_week(cx);
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/tasks/t2")).respond_with(ok(json!({}))));

        click(window, "cal-lane-tomorrow-t2", cx);
        wait_until(cx, "se pide mañana", |_| patches(&server, "t2").len() == 1);
        assert_eq!(patches(&server, "t2"), [json!({ "dueAt": format!("{}T00:00:00Z", days.today + Days::new(1)) })]);
        assert!(!inside(window, "cal-card-t2", "cal-lane", cx), "la tarjeta con fecha sigue en el carril");

        // De vuelta al carril, para el otro atajo.
        cx.update(|cx| Backend::run(cx, |core| async move { let _ = core.my_work().set_due_day("t2", None).await; }));
        wait_until(cx, "vuelve al carril", |cx| inside(window, "cal-card-t2", "cal-lane", cx));
        click(window, "cal-lane-today-t2", cx);
        wait_until(cx, "se pide hoy", |_| patches(&server, "t2").len() == 3);
        assert_eq!(patches(&server, "t2")[2], json!({ "dueAt": format!("{}T00:00:00Z", days.today) }));
        assert!(inside(window, "cal-card-t2", &Days2::col(days.today), cx));
        settle(cx);
        assert_eq!(opened_task(cx), None, "el atajo abrió la tarea");
        stop(cx);
    }
}
