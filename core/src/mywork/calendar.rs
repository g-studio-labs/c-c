//! «Mi trabajo» en calendario: el mes, la semana y «Por programar».
//!
//! Vino de `app/src/lib/calendar.ts`, `components/tasks/due-calendar.ts` y
//! `components/ItemCalendar.tsx`. La pantalla recibe cada día ya resuelto —qué
//! tiene, si es hoy, si algo venció— y sólo pinta y reenvía gestos.
//!
//! Aquí todo son **días** (`NaiveDate`, o `YYYY-MM-DD` en lo que viaja): un
//! vencimiento se lee con `dates::due_day` y «hoy» es el de la organización
//! (`MyWork::today`). No hay captador local con el que pintar en el 29 lo que
//! se soltó en el 30.

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::list::{MONTHS_EN, MONTHS_ES};
use crate::dates::{due_day, today_in};
use crate::error::ApiError;
use crate::mywork::{MyWork, OpenTask, TaskFilter};
use crate::task::{canonical_priority, canonical_status, due_instant};
use crate::texts::Locale;

/// Los tonos de una tarea: su prioridad, de más a menos. Es el orden de los
/// segmentos de la barra y el de la leyenda (`components/calendar/tones.ts`).
pub const CALENDAR_TONES: [&str; 5] = ["urgent", "high", "normal", "low", "none"];

/// El tono de una prioridad tal como llega: `medium` es `normal`, y una que
/// no se conoce —o ninguna— es `none`, no un color de más.
pub fn calendar_tone(priority: &str) -> &'static str {
    let priority = canonical_priority(priority);
    CALENDAR_TONES.into_iter().find(|tone| *tone == priority).unwrap_or("none")
}

/// Si ese tono cuenta como «prioridad alta» en el resumen de la semana: la
/// urgente también lo es.
pub fn tone_is_high(tone: &str) -> bool {
    matches!(tone, "urgent" | "high")
}

/// Mes o semana.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum CalendarView {
    #[default]
    Month,
    Week,
}

/// Si «Por programar» sale abierto en esa vista cuando nadie ha elegido:
/// cerrado en el mes, que necesita el ancho, y abierto en la semana.
pub fn lane_open_by_default(view: CalendarView) -> bool {
    view == CalendarView::Week
}

/// Cuánto de un trozo (de `top` a `bottom`) se sale de lo que se ve.
fn hidden(top: f32, bottom: f32, visible_top: f32, visible_bottom: f32) -> f32 {
    (visible_top - top).max(0.) + (bottom - visible_bottom).max(0.)
}

/// Si el detalle de un día del mes se abre hacia arriba: cuando hacia abajo
/// no cabe en lo que se ve y hacia arriba se le esconde menos. `down_top` es
/// dónde empezaría abriéndose hacia abajo y `up_bottom` dónde acabaría
/// abriéndose hacia arriba; `height`, lo que mide.
///
/// Decidido con jose (17-nov-2026). Antes iba por filas (hacia arriba en las
/// dos últimas del mes, como el `DayPopover` de la app de siempre), y en una
/// ventana baja el detalle de un día de en medio se salía por abajo, donde
/// no se alcanza. Si no cabe de ninguna de las dos maneras, hacia abajo: lo
/// que se pierde es el final, y su cabecera se sigue leyendo.
pub fn day_detail_opens_up(down_top: f32, up_bottom: f32, height: f32, visible_top: f32, visible_bottom: f32) -> bool {
    let down = hidden(down_top, down_top + height, visible_top, visible_bottom);
    let up = hidden(up_bottom - height, up_bottom, visible_top, visible_bottom);
    up < down
}

/// Una tarea como se enseña en el calendario: en la fila del popover del día,
/// en la tarjeta de la semana y en la del carril.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CalendarChip {
    pub id: String,
    /// El folio, como se lee: `#12`.
    pub label: String,
    pub title: String,
    /// Uno de `CALENDAR_TONES`: de qué color es y en qué segmento cuenta.
    pub tone: String,
    /// El día en que vence, `YYYY-MM-DD`. Sin él va a «Por programar».
    pub day: Option<String>,
    /// El estado crudo, para el color del punto.
    pub status: String,
    pub status_name: String,
    /// Quién la lleva, por nombre: es también la clave de su color.
    pub assignee: Option<String>,
    pub assignee_initials: Option<String>,
    /// Terminada o cerrada: se pinta tachada y apagada.
    pub done: bool,
    /// Venció sin cerrarse: el punto rojo del día y el chip «Vencida».
    pub overdue: bool,
}

/// Cuántas hay de un tono.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ToneCount {
    pub tone: String,
    pub count: u32,
}

/// Un día del calendario, con lo suyo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CalendarDay {
    /// `YYYY-MM-DD`. Es también lo que se le pasa a `set_due_day` al soltar
    /// una tarjeta encima.
    pub day: String,
    /// El número que se pinta.
    pub number: u32,
    /// Qué día de la semana es: 0 lunes, 6 domingo.
    pub weekday: u32,
    /// Falso en los días de otro mes que completan la primera y la última
    /// semana (van apagados). En la semana, siempre cierto.
    pub in_month: bool,
    pub today: bool,
    /// Ya pasó: en la semana sus tarjetas van atenuadas.
    pub past: bool,
    /// Alguna de las suyas venció sin cerrarse.
    pub overdue: bool,
    /// Las que no están terminadas: «N abiertas» en la semana.
    pub open: u32,
    /// El mapa de calor de la celda: 0 vacío, 1 una, 2 de dos a tres, 3
    /// cuatro o más.
    pub heat: u8,
    /// La barra de prioridad: por tono, en su orden y sin los vacíos.
    pub tones: Vec<ToneCount>,
    pub chips: Vec<CalendarChip>,
}

/// Una fila del mes: siete días, de lunes a domingo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CalendarWeek {
    pub days: Vec<CalendarDay>,
}

/// Un mes, en las semanas que ocupa.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MonthView {
    pub year: i32,
    /// 1–12.
    pub month: u32,
    /// «Noviembre de 2026».
    pub title: String,
    pub weeks: Vec<CalendarWeek>,
}

/// Lo que dice de una semana la franja de encima.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WeekSummary {
    /// Las que vencen esta semana.
    pub total: u32,
    /// De ésas, las de prioridad alta o urgente.
    pub high: u32,
    /// Vencidas sin cerrar, **de cualquier semana**: lo que se arrastra no
    /// deja de pesar por mirar otra.
    pub overdue: u32,
    /// Las terminadas de esta semana.
    pub done: u32,
    pub tones: Vec<ToneCount>,
}

/// Una semana, de lunes a domingo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WeekView {
    /// Su lunes, `YYYY-MM-DD`.
    pub start: String,
    /// «9 – 15 noviembre 2026».
    pub title: String,
    pub summary: WeekSummary,
    pub days: Vec<CalendarDay>,
}

/// Las iniciales de un avatar sin foto, como `iniciales` en
/// `app/src/lib/desde.ts`: la primera letra de las dos primeras palabras, o
/// las dos primeras letras si sólo hay una.
fn avatar_initials(name: &str) -> String {
    let clean = name.trim_start_matches('@').trim();
    let mut parts = clean.split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-')).filter(|part| !part.is_empty());
    let letters: String = match (parts.next(), parts.next()) {
        (Some(first), Some(second)) => first.chars().take(1).chain(second.chars().take(1)).collect(),
        _ => clean.chars().take(2).collect(),
    };
    letters.to_uppercase()
}

/// Una tarea, como elemento del calendario.
///
/// Vencida se mide contra el **día** de hoy, no contra el instante: lo que
/// vence hoy tiene el día entero. Y una terminada no vence: se tacha.
fn chip(task: &OpenTask, today: NaiveDate) -> CalendarChip {
    let day = due_day(task.due_at.as_deref());
    let done = matches!(canonical_status(&task.status), "resolved" | "closed");
    let assignee = task.assignee.clone().filter(|name| !name.is_empty());
    CalendarChip {
        id: task.id.clone(),
        label: format!("#{}", task.seq),
        title: task.title.clone(),
        tone: calendar_tone(&task.priority).to_string(),
        day: day.map(|day| day.to_string()),
        status: task.status.clone(),
        status_name: task.status_name.clone(),
        assignee_initials: assignee.as_deref().map(avatar_initials),
        assignee,
        done,
        overdue: !done && day.is_some_and(|day| day < today),
    }
}

/// Las tareas como elementos del calendario, en el orden en que llegan. Las
/// que no tienen fecha salen también, sin día: van a «Por programar».
pub fn calendar_chips(tasks: &[OpenTask], today: NaiveDate) -> Vec<CalendarChip> {
    tasks.iter().map(|task| chip(task, today)).collect()
}

/// Lo que no tiene fecha, y por tanto no cabe en ningún día.
///
/// Va a un lado en vez de descartarse: de sesenta y cinco tareas el calendario
/// enseñaba una, mientras la cabecera seguía diciendo sesenta y cinco.
pub fn unscheduled(chips: &[CalendarChip]) -> Vec<CalendarChip> {
    chips.iter().filter(|chip| chip.day.is_none()).cloned().collect()
}

/// Cuántos hay de cada tono, en el orden de los tonos y sin los vacíos.
fn tone_counts<'a>(chips: impl Iterator<Item = &'a CalendarChip> + Clone) -> Vec<ToneCount> {
    CALENDAR_TONES
        .into_iter()
        .map(|tone| ToneCount { tone: tone.to_string(), count: chips.clone().filter(|chip| chip.tone == tone).count() as u32 })
        .filter(|segment| segment.count > 0)
        .collect()
}

/// El mapa de calor de una celda. Los cortes son los del diseño.
fn heat_level(count: usize) -> u8 {
    match count {
        0 => 0,
        1 => 1,
        2 | 3 => 2,
        _ => 3,
    }
}

/// Qué día de la semana es, con el lunes en 0: la semana empieza en lunes.
fn weekday_index(day: NaiveDate) -> u32 {
    day.weekday().num_days_from_monday()
}

fn day_cell(day: NaiveDate, month: Option<u32>, chips: &[CalendarChip], today: NaiveDate) -> CalendarDay {
    let key = day.to_string();
    let chips: Vec<CalendarChip> = chips.iter().filter(|chip| chip.day.as_deref() == Some(key.as_str())).cloned().collect();
    CalendarDay {
        number: day.day(),
        weekday: weekday_index(day),
        in_month: month.is_none_or(|month| day.month() == month),
        today: day == today,
        past: day < today,
        overdue: chips.iter().any(|chip| chip.overdue),
        open: chips.iter().filter(|chip| !chip.done).count() as u32,
        heat: heat_level(chips.len()),
        tones: tone_counts(chips.iter()),
        chips,
        day: key,
    }
}

/// El lunes de la semana de ese día.
pub fn week_start(day: NaiveDate) -> NaiveDate {
    day - Days::new(u64::from(weekday_index(day)))
}

/// El día 1 del mes de ese día.
pub fn month_start(day: NaiveDate) -> NaiveDate {
    day.with_day(1).unwrap_or(day)
}

/// El mes de ese día, en las semanas que ocupa: cuatro, cinco o seis,
/// empezando en lunes.
///
/// Las justas y no siempre seis: aquí la navegación va encima de la rejilla,
/// así que no hay botón que se mueva bajo el cursor al cambiar de alto. Los
/// días de otro mes que completan la primera y la última semana salen con lo
/// suyo, marcados con `in_month` en falso.
pub fn month_view(month: NaiveDate, chips: &[CalendarChip], today: NaiveDate, locale: Locale) -> MonthView {
    let first = month_start(month);
    let next = first.checked_add_months(Months::new(1)).unwrap_or(first);
    let start = week_start(first);
    let weeks = start
        .iter_weeks()
        .take_while(|monday| *monday < next)
        .map(|monday| CalendarWeek {
            days: monday.iter_days().take(7).map(|day| day_cell(day, Some(first.month()), chips, today)).collect(),
        })
        .collect();
    MonthView { year: first.year(), month: first.month(), title: month_title(locale, first), weeks }
}

/// La semana de ese día, de lunes a domingo, con su resumen.
pub fn week_view(day: NaiveDate, chips: &[CalendarChip], today: NaiveDate, locale: Locale) -> WeekView {
    let start = week_start(day);
    let days: Vec<CalendarDay> = start.iter_days().take(7).map(|day| day_cell(day, None, chips, today)).collect();
    let week = || days.iter().flat_map(|day| day.chips.iter());
    let summary = WeekSummary {
        total: week().count() as u32,
        high: week().filter(|chip| tone_is_high(&chip.tone)).count() as u32,
        overdue: chips.iter().filter(|chip| chip.overdue).count() as u32,
        done: week().filter(|chip| chip.done).count() as u32,
        tones: tone_counts(week()),
    };
    WeekView { start: start.to_string(), title: week_title(locale, start), summary, days }
}

const LONG_MONTHS_ES: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
];
const LONG_MONTHS_EN: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
];
const WEEKDAYS_ES: [&str; 7] = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"];
const WEEKDAYS_EN: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

fn month_name(locale: Locale, day: NaiveDate, short: bool) -> &'static str {
    let month = day.month0() as usize;
    match (locale, short) {
        (Locale::Es, false) => LONG_MONTHS_ES[month],
        (Locale::Es, true) => MONTHS_ES[month],
        (Locale::En, false) => LONG_MONTHS_EN[month],
        (Locale::En, true) => MONTHS_EN[month],
    }
}

/// El título de un mes: «Noviembre de 2026», «November 2026». Sólo lleva
/// mayúscula al principio: salía «Noviembre De 2026».
pub fn month_title(locale: Locale, month: NaiveDate) -> String {
    let name = month_name(locale, month, false);
    match locale {
        Locale::Es => {
            let mut letters = name.chars();
            let capital: String = letters.next().map(|first| first.to_uppercase().collect()).unwrap_or_default();
            format!("{capital}{} de {}", letters.as_str(), month.year())
        }
        Locale::En => format!("{name} {}", month.year()),
    }
}

/// El título de una semana, dado su lunes: «9 – 15 noviembre 2026»; si cruza
/// de mes, «30 nov – 6 diciembre 2026»; y de año, «28 dic 2026 – 3 enero
/// 2027». Es el del diseño, no el de un formateador de rangos.
pub fn week_title(locale: Locale, start: NaiveDate) -> String {
    let end = start + Days::new(6);
    let right = format!("{} {} {}", end.day(), month_name(locale, end, false), end.year());
    if start.month() == end.month() {
        return format!("{} – {right}", start.day());
    }
    let left = format!("{} {}", start.day(), month_name(locale, start, true));
    if start.year() == end.year() {
        format!("{left} – {right}")
    } else {
        format!("{left} {} – {right}", start.year())
    }
}

/// Un día dicho entero, para la cabecera de su popover: «11 de noviembre»,
/// «November 11».
pub fn day_title(locale: Locale, day: NaiveDate) -> String {
    let month = month_name(locale, day, false);
    match locale {
        Locale::Es => format!("{} de {month}", day.day()),
        Locale::En => format!("{month} {}", day.day()),
    }
}

/// El nombre de un día de la semana (0 lunes, 6 domingo): «miércoles», o
/// «mié» para la cabecera de la rejilla y de cada columna.
pub fn weekday_name(locale: Locale, weekday: u32, short: bool) -> String {
    let name = match locale {
        Locale::Es => WEEKDAYS_ES,
        Locale::En => WEEKDAYS_EN,
    }[(weekday % 7) as usize];
    if short {
        name.chars().take(3).collect()
    } else {
        name.to_string()
    }
}

/// Dónde está mirando el calendario: qué vista, qué mes y qué semana.
///
/// El mes y la semana van aparte: las flechas mueven sólo lo que se mira,
/// «Hoy» lleva los dos al actual, y pasar de Semana a Mes abre el mes de la
/// semana visible.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CalendarCursor {
    pub view: CalendarView,
    /// El día 1 del mes que se mira, `YYYY-MM-DD`.
    pub month: String,
    /// El lunes de la semana que se mira, `YYYY-MM-DD`.
    pub week: String,
}

fn parse_day(day: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

impl CalendarCursor {
    /// En el mes y la semana de hoy.
    pub fn starting(today: NaiveDate, view: CalendarView) -> Self {
        Self { view, month: month_start(today).to_string(), week: week_start(today).to_string() }
    }

    /// El día 1 del mes que se mira, para `month_view`.
    pub fn month_day(&self) -> Option<NaiveDate> {
        parse_day(&self.month).map(month_start)
    }

    /// El lunes de la semana que se mira, para `week_view`.
    pub fn week_day(&self) -> Option<NaiveDate> {
        parse_day(&self.week).map(week_start)
    }

    /// Las flechas: `1` adelante, `-1` atrás. En el mes cambian de mes; en la
    /// semana, siete días. Lo que no se mira se queda donde estaba.
    pub fn stepped(&self, direction: i32) -> Self {
        let mut next = self.clone();
        let amount = direction.unsigned_abs();
        match self.view {
            CalendarView::Month => {
                let moved = self.month_day().and_then(|month| {
                    if direction < 0 {
                        month.checked_sub_months(Months::new(amount))
                    } else {
                        month.checked_add_months(Months::new(amount))
                    }
                });
                if let Some(month) = moved {
                    next.month = month.to_string();
                }
            }
            CalendarView::Week => {
                let days = Days::new(7 * u64::from(amount));
                let moved = self
                    .week_day()
                    .and_then(|week| if direction < 0 { week.checked_sub_days(days) } else { week.checked_add_days(days) });
                if let Some(week) = moved {
                    next.week = week.to_string();
                }
            }
        }
        next
    }

    /// «Hoy»: el mes y la semana actuales, en la vista que haya.
    pub fn at_today(&self, today: NaiveDate) -> Self {
        Self::starting(today, self.view)
    }

    /// Cambia de vista. De Semana a Mes, el mes de la semana que se miraba
    /// (el de su lunes).
    pub fn viewed_as(&self, view: CalendarView) -> Self {
        let mut next = self.clone();
        if view == CalendarView::Month && self.view == CalendarView::Week {
            if let Some(week) = self.week_day() {
                next.month = month_start(week).to_string();
            }
        }
        next.view = view;
        next
    }
}

fn refused(code: &str, message: &str) -> ApiError {
    ApiError::Server { status: 0, code: code.to_string(), message: message.to_string() }
}

impl MyWork {
    /// Qué día es hoy para el calendario: el de la organización que se mira,
    /// no el de la máquina (`dates::today_in`).
    pub fn today(&self) -> NaiveDate {
        self.today_at(Utc::now())
    }

    /// Lo mismo, en ese instante.
    pub fn today_at(&self, now: DateTime<Utc>) -> NaiveDate {
        let orgs = self.core.orgs().state();
        today_in(orgs.current_zone(), now, &chrono::Local)
    }

    /// Mañana, contado desde ese hoy: el atajo «Mañana» de «Por programar».
    pub fn tomorrow(&self) -> NaiveDate {
        self.tomorrow_at(Utc::now())
    }

    /// Lo mismo, en ese instante.
    pub fn tomorrow_at(&self, now: DateTime<Utc>) -> NaiveDate {
        let today = self.today_at(now);
        today.succ_opt().unwrap_or(today)
    }

    /// Lo que el calendario tiene para pintar ahora: lo visible que pasa el
    /// filtro, con y sin fecha.
    pub fn calendar_chips(&self, filter: &TaskFilter) -> Vec<CalendarChip> {
        calendar_chips(&self.filtered(filter), self.today())
    }

    /// El mes de ese día, con lo que se ve ahora.
    pub fn month_view(&self, filter: &TaskFilter, locale: Locale, month: NaiveDate) -> MonthView {
        month_view(month, &self.calendar_chips(filter), self.today(), locale)
    }

    /// La semana de ese día, con lo que se ve ahora.
    pub fn week_view(&self, filter: &TaskFilter, locale: Locale, day: NaiveDate) -> WeekView {
        week_view(day, &self.calendar_chips(filter), self.today(), locale)
    }

    /// «Por programar», con lo que se ve ahora.
    pub fn unscheduled(&self, filter: &TaskFilter) -> Vec<CalendarChip> {
        unscheduled(&self.calendar_chips(filter))
    }

    /// Le pone a una tarea ese día de vencimiento (`YYYY-MM-DD`: soltarla en
    /// un día), o se lo quita con `None` (soltarla en «Por programar»).
    ///
    /// El cambio se ve al instante y se deshace si el servidor no lo acepta:
    /// arrastrar a un día y esperar la vuelta para ver la tarjeta moverse se
    /// lee como que el arrastre no funcionó. Soltarla en el día que ya tiene
    /// no cambia nada ni pide nada.
    pub async fn set_due_day(&self, task_id: &str, day: Option<&str>) -> Result<(), ApiError> {
        // Quitarlo es `clearDueAt`, no `dueAt: null`: el servidor lee un
        // `null` como «no tocar» y la fecha se quedaba puesta sin error (#95).
        let (instant, body) = match day {
            Some(day) => {
                let instant = due_instant(day).ok_or_else(|| refused("invalid-due-day", "Invalid due date"))?;
                let body = json!({ "dueAt": instant });
                (Some(instant), body)
            }
            None => (None, json!({ "clearDueAt": true })),
        };
        let wanted = due_day(instant.as_deref());
        let Some(task) = self.state().tasks.into_iter().find(|task| task.id == task_id) else {
            // Sin la tarea a la vista no se sabe qué día tiene ni habría qué
            // deshacer: lo que se arrastra en el calendario siempre está.
            return Err(refused("not-found", "Task not found"));
        };
        if due_day(task.due_at.as_deref()) == wanted {
            return Ok(());
        }

        let previous = self.set_due_local(task_id, instant.clone());
        if let Err(error) = self.core.call_ok("PATCH", &format!("/api/v1/tasks/{task_id}"), Some(body)).await {
            // Sólo si sigue puesto lo que puso este gesto: si mientras tanto
            // se soltó en otro día, deshacer éste se llevaría el nuevo.
            let still_ours = self.state().tasks.iter().any(|task| task.id == task_id && task.due_at == instant);
            if still_ours {
                self.set_due_local(task_id, previous);
            }
            return Err(error);
        }

        // Si es la tarea que está abierta, su ficha enseñaba el día de antes.
        let tasks = self.core.tasks();
        if tasks.state().task_id.as_deref() == Some(task_id) {
            tasks.refresh().await;
        }
        Ok(())
    }
}
