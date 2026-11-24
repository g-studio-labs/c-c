//! El calendario de «Mi trabajo»: el mes, la semana, «Por programar» y soltar
//! una tarjeta en un día, contra un servidor de mentira.
//!
//! Viene de `app/src/lib/calendar.ts`, `components/tasks/due-calendar.ts` e
//! `ItemCalendar.tsx`, con sus pruebas. Cada prueba dice la rotura que atrapa,
//! y se ha comprobado rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{
    calendar_chips, calendar_tone, day_title, lane_open_by_default, month_title, month_view, today_in, tone_is_high, unscheduled,
    week_start, week_title, week_view, weekday_name, ApiError, CalendarChip, CalendarCursor, CalendarDay, CalendarView, Config, Core,
    Listener, Locale, MonthView, MyWorkState, OpenTask, ScopeKind, Session, Storage, TaskFilter, WorkScope,
};
use chrono::{NaiveDate, TimeZone, Utc};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[path = "support/gated.rs"]
mod gated;
use gated::{front, until, Gate};

#[derive(Default)]
struct MemoryStorage(Mutex<HashMap<String, String>>);

impl Storage for MemoryStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn remove(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}

struct Quiet;

impl Listener for Quiet {
    fn session_changed(&self, _session: Option<Session>) {}
    fn my_work_changed(&self, _state: MyWorkState) {}
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Una tarea como la manda el servidor, con lo que se le cambie encima.
fn wire(id: &str, due: Option<&str>, extra: Value) -> Value {
    let mut task = json!({
        "id": id, "seq": 1, "title": format!("Tarea {id}"), "priority": "medium",
        "status": "open", "statusName": "Open", "statusKind": "open",
        "listId": "l1", "listName": "Lista", "spaceId": "s1", "spaceName": "Espacio",
        "updatedAt": "2026-11-14T00:00:00Z", "subtasksDone": 0, "subtasksTotal": 0,
    });
    if let Some(due) = due {
        task["dueAt"] = json!(due);
    }
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        task[key] = value;
    }
    task
}

fn task(id: &str, due: Option<&str>, extra: Value) -> OpenTask {
    serde_json::from_value(wire(id, due, extra)).unwrap()
}

/// Una tarea que vence ese día (`YYYY-MM-DD`), guardada como la guarda la
/// app: su medianoche UTC.
fn due_on(id: &str, day: &str, extra: Value) -> OpenTask {
    task(id, Some(&format!("{day}T00:00:00Z")), extra)
}

fn all_days(month: &MonthView) -> Vec<&CalendarDay> {
    month.weeks.iter().flat_map(|week| week.days.iter()).collect()
}

fn cell<'a>(month: &'a MonthView, day: &str) -> &'a CalendarDay {
    all_days(month).into_iter().find(|cell| cell.day == day).unwrap_or_else(|| panic!("el {day} no está en la rejilla"))
}

fn ids(chips: &[CalendarChip]) -> Vec<&str> {
    chips.iter().map(|chip| chip.id.as_str()).collect()
}

/// Los casos de `calendar.test.ts`, «un mes ocupa las semanas que necesita».
///
/// Roturas que atrapa: empezar la semana en domingo; pintar siempre seis
/// semanas; no marcar como de fuera los días de otro mes que completan la
/// primera y la última fila.
#[test]
fn a_month_takes_the_weeks_it_needs_starting_on_monday() {
    let today = day(2026, 11, 16);
    // Noviembre de 2026 empieza en domingo: seis semanas, desde el lunes 26
    // de octubre hasta el domingo 6 de diciembre.
    let november = month_view(day(2026, 11, 20), &[], today, Locale::Es);
    assert_eq!((november.year, november.month, november.weeks.len()), (2026, 11, 6));
    assert!(november.weeks.iter().all(|week| week.days.len() == 7));
    let days = all_days(&november);
    assert_eq!((days[0].day.as_str(), days[0].weekday, days[0].number), ("2026-10-26", 0, 26));
    assert_eq!((days[41].day.as_str(), days[41].weekday), ("2026-12-06", 6));
    let outside: Vec<&str> = days.iter().filter(|cell| !cell.in_month).map(|cell| cell.day.as_str()).collect();
    assert_eq!(
        outside,
        [
            "2026-10-26", "2026-10-27", "2026-10-28", "2026-10-29", "2026-10-30", "2026-10-31", "2026-12-01", "2026-12-02",
            "2026-12-03", "2026-12-04", "2026-12-05", "2026-12-06"
        ]
    );

    // Febrero de 2027 empieza en lunes y tiene 28 días: cuatro, y nada de fuera.
    let february = month_view(day(2027, 2, 1), &[], today, Locale::Es);
    assert_eq!(february.weeks.len(), 4);
    assert!(all_days(&february).iter().all(|cell| cell.in_month));
    // Octubre de 2026 empieza en jueves: cinco.
    assert_eq!(month_view(day(2026, 10, 1), &[], today, Locale::Es).weeks.len(), 5);
}

/// Febrero de un año bisiesto tiene 29, y lo que vence el 29 cae ahí.
///
/// Roturas que atrapa: cortar el mes en el 28 (el 29 saldría como de marzo, o
/// no saldría); contar el 1 de marzo como de febrero.
#[test]
fn a_leap_february_has_its_29th() {
    let chips = calendar_chips(&[due_on("a", "2028-02-29", json!({}))], day(2028, 2, 1));
    let february = month_view(day(2028, 2, 10), &chips, day(2028, 2, 1), Locale::Es);

    // Empieza en martes: 1 + 29 días son cinco semanas, hasta el 5 de marzo.
    assert_eq!(february.weeks.len(), 5);
    let leap = cell(&february, "2028-02-29");
    assert!(leap.in_month);
    assert_eq!(ids(&leap.chips), ["a"]);
    assert!(!cell(&february, "2028-03-01").in_month);
    assert_eq!(all_days(&february).last().unwrap().day, "2028-03-05");

    // Y el de 2027 no lo tiene: tras el 28 no hay más rejilla.
    let plain = month_view(day(2027, 2, 1), &[], day(2027, 2, 1), Locale::Es);
    assert_eq!(all_days(&plain).last().unwrap().day, "2027-02-28");
}

/// El fallo del 29-por-30 (`ItemCalendar.test.tsx`, «en qué día cae»): un
/// vencimiento es el día a medianoche UTC, y cae en **ese** día se mire desde
/// donde se mire.
///
/// Rotura que atrapa: leer el instante en una zona al oeste de Greenwich (o en
/// la de la máquina), que lo pinta el día anterior.
#[test]
fn a_due_date_at_midnight_utc_lands_on_its_own_day() {
    let tasks = [
        task("midnight", Some("2026-09-30T00:00:00.000Z"), json!({})),
        // El mismo instante, escrito desde México.
        task("mexico", Some("2026-09-29T18:00:00-06:00"), json!({})),
        // Y el último instante de ese día en UTC sigue siendo ese día.
        task("late", Some("2026-09-30T23:59:59Z"), json!({})),
    ];
    let chips = calendar_chips(&tasks, day(2026, 9, 1));
    assert!(chips.iter().all(|chip| chip.day.as_deref() == Some("2026-09-30")));

    let september = month_view(day(2026, 9, 1), &chips, day(2026, 9, 1), Locale::Es);
    assert_eq!(ids(&cell(&september, "2026-09-30").chips), ["midnight", "mexico", "late"]);
    assert!(cell(&september, "2026-09-29").chips.is_empty());
}

/// A las 03:00 UTC del 13 es todavía 12 en México y ya 13 en Auckland: la
/// misma tarea del 12 vence hoy al oeste y ya venció al este.
///
/// Roturas que atrapa: marcar «hoy» en otro día que el que se le da; dar por
/// vencido lo que vence hoy; no dar por pasado el día anterior a hoy.
#[test]
fn today_and_overdue_follow_the_organizations_day_west_and_east() {
    let now = Utc.with_ymd_and_hms(2026, 11, 13, 3, 0, 0).unwrap();
    let tasks = [due_on("a", "2026-11-12", json!({}))];
    let marked = |month: &MonthView| -> Vec<String> {
        all_days(month).iter().filter(|cell| cell.today).map(|cell| cell.day.clone()).collect()
    };

    let mexico = today_in(Some("America/Mexico_City"), now, &Utc);
    let west = month_view(mexico, &calendar_chips(&tasks, mexico), mexico, Locale::Es);
    assert_eq!(marked(&west), ["2026-11-12"]);
    let twelfth = cell(&west, "2026-11-12");
    assert!(!twelfth.past && !twelfth.overdue && !twelfth.chips[0].overdue, "lo que vence hoy tiene el día entero");
    assert!(cell(&west, "2026-11-11").past);

    let auckland = today_in(Some("Pacific/Auckland"), now, &Utc);
    let east = month_view(auckland, &calendar_chips(&tasks, auckland), auckland, Locale::Es);
    assert_eq!(marked(&east), ["2026-11-13"]);
    let twelfth = cell(&east, "2026-11-12");
    assert!(twelfth.past && twelfth.overdue && twelfth.chips[0].overdue);
    assert!(!cell(&east, "2026-11-13").past);
}

/// En el borde del mes: a las 03:00 UTC del 1 de diciembre es todavía 30 de
/// noviembre en México. Diciembre de 2026 empieza en martes, así que su
/// rejilla arranca el lunes 30 de noviembre: «hoy» es ese día de fuera.
///
/// Rotura que atrapa: no marcar hoy en un día de otro mes (o marcarlo en el 1
/// porque es el del reloj UTC).
#[test]
fn today_can_be_a_day_outside_the_month_at_its_edge() {
    let now = Utc.with_ymd_and_hms(2026, 12, 1, 3, 0, 0).unwrap();
    let mexico = today_in(Some("America/Mexico_City"), now, &Utc);
    let auckland = today_in(Some("Pacific/Auckland"), now, &Utc);

    let december = month_view(day(2026, 12, 1), &[], mexico, Locale::Es);
    let edge = cell(&december, "2026-11-30");
    assert!(edge.today && !edge.in_month);
    assert!(!cell(&december, "2026-12-01").today);
    // El mismo instante, mirando noviembre: es su último día.
    assert!(cell(&month_view(mexico, &[], mexico, Locale::Es), "2026-11-30").in_month);

    let east = month_view(day(2026, 12, 1), &[], auckland, Locale::Es);
    assert!(cell(&east, "2026-12-01").today && !cell(&east, "2026-11-30").today);
}

/// Los casos de `due-calendar.test.ts`: lo que enseña una tarjeta.
///
/// Roturas que atrapa: enseñar el folio sin `#`; el tono con el nombre
/// guardado (`medium`) o un color de más para una prioridad desconocida; dar
/// por vencida la de hoy o una ya terminada; no tachar la cerrada; las
/// iniciales con las dos primeras letras cuando hay nombre y apellido.
#[test]
fn a_chip_says_what_the_card_shows() {
    let today = day(2026, 11, 12);
    let tasks = [
        due_on("yesterday", "2026-11-11", json!({ "seq": 203, "assignee": "Ana López", "statusName": "En curso", "status": "in_progress" })),
        due_on("today", "2026-11-12", json!({ "priority": "urgent", "assignee": "jose" })),
        due_on("done", "2026-11-01", json!({ "status": "done", "priority": "low", "assignee": "@maria.perez" })),
        due_on("closed", "2026-11-01", json!({ "status": "closed", "priority": "", "assignee": "" })),
        due_on("resolved", "2026-11-01", json!({ "status": "resolved", "priority": "rarísima" })),
        task("undated", None, json!({ "priority": "high" })),
    ];
    let chips = calendar_chips(&tasks, today);
    let by = |id: &str| chips.iter().find(|chip| chip.id == id).unwrap();

    let yesterday = by("yesterday");
    assert_eq!((yesterday.label.as_str(), yesterday.title.as_str(), yesterday.tone.as_str()), ("#203", "Tarea yesterday", "normal"));
    assert_eq!((yesterday.status.as_str(), yesterday.status_name.as_str()), ("in_progress", "En curso"));
    assert_eq!((yesterday.assignee.as_deref(), yesterday.assignee_initials.as_deref()), (Some("Ana López"), Some("AL")));
    assert!(yesterday.overdue && !yesterday.done);

    assert!(!by("today").overdue, "lo que vence hoy no está vencido hasta mañana");
    assert_eq!((by("today").tone.as_str(), by("today").assignee_initials.as_deref()), ("urgent", Some("JO")));

    for finished in ["done", "closed", "resolved"] {
        assert!(by(finished).done && !by(finished).overdue, "{finished}: una terminada no vence, se tacha");
    }
    assert_eq!(by("done").assignee_initials.as_deref(), Some("MP"));
    assert_eq!((by("closed").assignee.as_deref(), by("closed").assignee_initials.as_deref()), (None, None), "un nombre vacío no es nadie");
    assert_eq!((by("done").tone.as_str(), by("closed").tone.as_str(), by("resolved").tone.as_str()), ("low", "none", "none"));

    let undated = by("undated");
    assert_eq!((undated.day.as_deref(), undated.overdue, undated.assignee.as_deref(), undated.assignee_initials.as_deref()), (None, false, None, None));

    assert_eq!(calendar_tone("medium"), "normal");
    assert!(tone_is_high("urgent") && tone_is_high("high") && !tone_is_high("normal") && !tone_is_high("none"));
}

/// Lo que dice un día de un vistazo (`ItemCalendar.test.tsx`, «cada día»).
///
/// Roturas que atrapa: la barra en el orden de llegada en vez del de los
/// tonos, o con segmentos vacíos; los cortes del mapa de calor movidos;
/// contar las terminadas como abiertas; no encender el punto de vencidas.
#[test]
fn a_day_says_how_much_it_has_and_of_what() {
    let today = day(2026, 11, 16);
    let mut tasks = vec![
        due_on("a", "2026-11-10", json!({ "priority": "" })),
        due_on("b", "2026-11-10", json!({ "priority": "high" })),
        due_on("c", "2026-11-10", json!({ "priority": "", "status": "done" })),
        due_on("d", "2026-11-20", json!({})),
    ];
    // El 21 con una, el 22 con dos, el 23 con tres, el 24 con cuatro.
    for (date, count) in [(21, 1), (22, 2), (23, 3), (24, 4)] {
        for n in 0..count {
            tasks.push(due_on(&format!("h{date}-{n}"), &format!("2026-11-{date}"), json!({})));
        }
    }
    let month = month_view(today, &calendar_chips(&tasks, today), today, Locale::Es);

    let tenth = cell(&month, "2026-11-10");
    let tones: Vec<(&str, u32)> = tenth.tones.iter().map(|segment| (segment.tone.as_str(), segment.count)).collect();
    assert_eq!(tones, [("high", 1), ("none", 2)]);
    assert_eq!(ids(&tenth.chips), ["a", "b", "c"]);
    assert_eq!((tenth.open, tenth.overdue), (2, true));

    let twentieth = cell(&month, "2026-11-20");
    assert_eq!((twentieth.open, twentieth.overdue), (1, false));

    let heat = |date: &str| cell(&month, date).heat;
    assert_eq!(
        [heat("2026-11-19"), heat("2026-11-21"), heat("2026-11-22"), heat("2026-11-23"), heat("2026-11-24")],
        [0, 1, 2, 2, 3]
    );
}

/// Los casos de `ItemCalendar.week.test.tsx`.
///
/// Roturas que atrapa: empezar la semana en domingo o en el día que se pide;
/// no cruzar de mes; contar «alta» sin la urgente; contar las vencidas sólo
/// de esta semana, o las hechas de cualquiera; no distinguir hoy ni los días
/// pasados; marcar como «de fuera» los días de la semana que caen en otro mes.
#[test]
fn a_week_runs_monday_to_sunday_with_its_summary() {
    let today = day(2026, 12, 2);
    let tasks = [
        due_on("old", "2026-11-03", json!({ "priority": "urgent" })),
        due_on("old-done", "2026-11-03", json!({ "status": "done" })),
        due_on("mon", "2026-11-30", json!({ "priority": "urgent" })),
        due_on("tue", "2026-12-01", json!({ "priority": "high", "status": "done" })),
        due_on("wed", "2026-12-02", json!({})),
        due_on("sun", "2026-12-06", json!({ "priority": "" })),
        due_on("next", "2026-12-07", json!({ "priority": "high" })),
        task("undated", None, json!({ "priority": "high" })),
    ];
    let chips = calendar_chips(&tasks, today);
    // Se pide por el domingo: su semana empezó el lunes 30 de noviembre.
    let week = week_view(day(2026, 12, 6), &chips, today, Locale::Es);

    assert_eq!(week.start, "2026-11-30");
    let days: Vec<(&str, u32, u32)> = week.days.iter().map(|cell| (cell.day.as_str(), cell.weekday, cell.number)).collect();
    assert_eq!(
        days,
        [
            ("2026-11-30", 0, 30), ("2026-12-01", 1, 1), ("2026-12-02", 2, 2), ("2026-12-03", 3, 3), ("2026-12-04", 4, 4),
            ("2026-12-05", 5, 5), ("2026-12-06", 6, 6)
        ]
    );
    assert!(week.days.iter().all(|cell| cell.in_month));
    let per_day: Vec<Vec<&str>> = week.days.iter().map(|cell| ids(&cell.chips)).collect();
    assert_eq!(per_day, [vec!["mon"], vec!["tue"], vec!["wed"], vec![], vec![], vec![], vec!["sun"]]);

    let flags: Vec<(bool, bool)> = week.days.iter().map(|cell| (cell.past, cell.today)).collect();
    assert_eq!(flags[..4], [(true, false), (true, false), (false, true), (false, false)]);
    assert_eq!((week.days[0].open, week.days[1].open), (1, 0));

    // Esta semana: cuatro, dos de ellas altas (la urgente cuenta), una hecha.
    // Vencidas sin cerrar: la del lunes y la vieja de noviembre, no la hecha.
    let summary = &week.summary;
    assert_eq!((summary.total, summary.high, summary.overdue, summary.done), (4, 2, 2, 1));
    let tones: Vec<(&str, u32)> = summary.tones.iter().map(|segment| (segment.tone.as_str(), segment.count)).collect();
    assert_eq!(tones, [("urgent", 1), ("high", 1), ("normal", 1), ("none", 1)]);

    assert_eq!(week_start(day(2026, 11, 15)), day(2026, 11, 9), "desde un domingo, su lunes");
    assert_eq!(week_start(day(2026, 11, 9)), day(2026, 11, 9));
}

/// Los títulos de `calendar.test.ts`: la semana como el diseño, y el mes con
/// mayúscula sólo al principio.
///
/// Roturas que atrapa: «Noviembre De 2026»; el mes abreviado donde va entero
/// (o al revés) al cruzar de mes; callar el año de la izquierda al cruzar de
/// año, o repetirlo cuando no se cruza; el día y el mes en el orden del otro
/// idioma; los días de la semana empezando en domingo.
#[test]
fn titles_read_like_the_design() {
    assert_eq!(week_title(Locale::Es, day(2026, 11, 9)), "9 – 15 noviembre 2026");
    assert_eq!(week_title(Locale::Es, day(2026, 11, 30)), "30 nov – 6 diciembre 2026");
    assert_eq!(week_title(Locale::Es, day(2026, 12, 28)), "28 dic 2026 – 3 enero 2027");
    assert_eq!(week_title(Locale::En, day(2026, 11, 30)), "30 Nov – 6 December 2026");

    assert_eq!(month_title(Locale::Es, day(2026, 11, 20)), "Noviembre de 2026");
    assert_eq!(month_title(Locale::En, day(2026, 11, 20)), "November 2026");
    assert_eq!(month_view(day(2026, 1, 9), &[], day(2026, 1, 9), Locale::Es).title, "Enero de 2026");
    assert_eq!(week_view(day(2026, 11, 11), &[], day(2026, 11, 11), Locale::Es).title, "9 – 15 noviembre 2026");

    assert_eq!(day_title(Locale::Es, day(2026, 11, 11)), "11 de noviembre");
    assert_eq!(day_title(Locale::En, day(2026, 11, 11)), "November 11");

    let short: Vec<String> = (0..7).map(|weekday| weekday_name(Locale::Es, weekday, true)).collect();
    assert_eq!(short, ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"]);
    assert_eq!(weekday_name(Locale::Es, 2, false), "miércoles");
    assert_eq!((weekday_name(Locale::En, 0, true), weekday_name(Locale::En, 6, false)), ("Mon".to_string(), "Sunday".to_string()));
}

/// «Por programar» lleva lo que no tiene fecha, en su orden, y nada más
/// (`ItemCalendar.undated.test.tsx`).
///
/// Roturas que atrapa: tragarse lo que no tiene fecha; meter en el carril lo
/// que sí la tiene.
#[test]
fn the_lane_carries_what_has_no_date() {
    let today = day(2026, 11, 16);
    let tasks = [
        task("b", None, json!({})),
        due_on("dated", "2026-11-20", json!({})),
        task("a", None, json!({})),
        // Una fecha que no se entiende tampoco cabe en ningún día.
        task("broken", Some("no soy una fecha"), json!({})),
    ];
    let chips = calendar_chips(&tasks, today);

    assert_eq!(ids(&unscheduled(&chips)), ["b", "a", "broken"]);
    assert_eq!(chips.len(), 4, "el calendario no pierde ninguna: con día o al carril");
    assert!(lane_open_by_default(CalendarView::Week) && !lane_open_by_default(CalendarView::Month));
}

/// Las flechas, «Hoy» y el cambio de vista (`ItemCalendar.test.tsx`, «la
/// cabecera»; `ItemCalendar.week.test.tsx`, «pasar de una vista a otra»).
///
/// Roturas que atrapa: que las flechas del mes muevan la semana (o al revés);
/// no cruzar de año; mover la semana un día en vez de siete; que «Hoy» sólo
/// devuelva lo que se mira; que pasar a Mes no abra el de la semana visible, o
/// que pasar a Semana mueva el mes.
#[test]
fn the_cursor_moves_what_is_being_looked_at() {
    // Un domingo: su semana empezó el lunes 9.
    let today = day(2026, 11, 15);
    let cursor = CalendarCursor::starting(today, CalendarView::Month);
    assert_eq!((cursor.month.as_str(), cursor.week.as_str()), ("2026-11-01", "2026-11-09"));
    assert_eq!((cursor.month_day(), cursor.week_day()), (Some(day(2026, 11, 1)), Some(day(2026, 11, 9))));

    // En el mes: dos adelante cruza de año, y la semana no se mueve.
    let january = cursor.stepped(1).stepped(1);
    assert_eq!((january.month.as_str(), january.week.as_str()), ("2027-01-01", "2026-11-09"));
    assert_eq!(january.stepped(-1).month, "2026-12-01");
    assert_eq!(cursor.stepped(-1).month, "2026-10-01");

    // En la semana: de siete en siete, cruzando de mes, y el mes no se mueve.
    let week = cursor.viewed_as(CalendarView::Week);
    assert_eq!((week.view, week.month.as_str()), (CalendarView::Week, "2026-11-01"));
    let later = week.stepped(1).stepped(1).stepped(1);
    assert_eq!((later.week.as_str(), later.month.as_str()), ("2026-11-30", "2026-11-01"));
    assert_eq!(week.stepped(-1).stepped(-1).week, "2026-10-26");

    // De Semana a Mes: el mes de la semana que se miraba.
    let december = later.stepped(1).viewed_as(CalendarView::Month);
    assert_eq!((december.view, december.month.as_str(), december.week.as_str()), (CalendarView::Month, "2026-12-01", "2026-12-07"));
    // De Mes a Semana el mes se queda: no es el de la semana.
    assert_eq!(january.viewed_as(CalendarView::Week).month, "2027-01-01");

    // «Hoy» lleva los dos al actual, en la vista que haya.
    let back = december.stepped(1).at_today(today);
    assert_eq!(back, cursor);
    assert_eq!(later.at_today(today), week);
}

struct Rig {
    server: MockServer,
    core: Core,
}

/// Un core con sesión, un servidor que contesta `/api/v1/tasks/` con `tasks`,
/// y «Mi trabajo» ya cargado.
async fn rig(tasks: Vec<Value>) -> Rig {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": tasks })))
        .mount(&server)
        .await;
    let core = Core::new(Config::new(front(&server.uri())), Arc::new(MemoryStorage::default()), Arc::new(Quiet)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    core.my_work().load(None).await;
    Rig { server, core }
}

fn ok() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": {} }))
}

fn refusal() -> ResponseTemplate {
    ResponseTemplate::new(409).set_body_json(json!({ "success": false, "error": "forbidden", "message": "No puedes" }))
}

async fn mount_patch(server: &MockServer, id: &str, reply: ResponseTemplate) {
    Mock::given(method("PATCH")).and(path(format!("/api/v1/tasks/{id}"))).respond_with(reply).mount(server).await;
}

/// Los cuerpos que recibió ese verbo en esa ruta, en orden.
async fn bodies(server: &MockServer, verb: &str, route: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == verb && request.url.path() == route)
        .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
        .collect()
}

/// El día y el instante que tiene una tarea en «Mi trabajo».
fn due_of(core: &Core, id: &str) -> (Option<String>, Option<String>) {
    let task = core.my_work().state().tasks.into_iter().find(|task| task.id == id).unwrap();
    (task.due_day, task.due_at)
}

/// El calendario pinta lo que se ve en «Mi trabajo»: dentro del ámbito y
/// pasado el filtro, con y sin fecha.
///
/// Roturas que atrapa: construirlo con todas las tareas cargadas, saltándose
/// el ámbito o el filtro de la barra.
#[tokio::test]
async fn the_calendar_shows_what_my_work_shows() {
    let rig = rig(vec![
        wire("mine", Some("2026-11-10T00:00:00Z"), json!({ "priority": "high" })),
        wire("mine-undated", None, json!({ "priority": "high" })),
        wire("low", Some("2026-11-10T00:00:00Z"), json!({ "priority": "low" })),
        wire("elsewhere", Some("2026-11-10T00:00:00Z"), json!({ "priority": "high", "listId": "l2" })),
    ])
    .await;
    let work = rig.core.my_work();
    work.set_scope(Some(WorkScope { kind: ScopeKind::List, id: "l1".into(), name: "Lista".into(), org_id: None }));
    let filter = TaskFilter { priority: "high".into(), ..TaskFilter::default() };

    assert_eq!(ids(&work.calendar_chips(&filter)), ["mine", "mine-undated"]);
    let month = work.month_view(&filter, Locale::Es, day(2026, 11, 1));
    assert_eq!(ids(&cell(&month, "2026-11-10").chips), ["mine"]);
    let week = work.week_view(&filter, Locale::Es, day(2026, 11, 10));
    assert_eq!((week.summary.total, ids(&week.days[1].chips)), (1, vec!["mine"]));
    assert_eq!(ids(&work.unscheduled(&filter)), ["mine-undated"]);
}

async fn core_in_zone(zone: &str) -> (MockServer, Core) {
    let rig = rig(vec![]).await;
    let org = json!({
        "id": "o1", "name": "Org", "slug": "o1", "role": "member", "memberCount": 2,
        "createdAt": "2026-06-09T12:00:00Z", "clientsSeeOnlyTheirSpace": false,
        "guestsCanUseDevTools": false, "doneNeedsSubtasksDone": false, "timezone": zone,
    });
    Mock::given(method("GET"))
        .and(path("/api/v1/organizations/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": [org] })))
        .mount(&rig.server)
        .await;
    rig.core.orgs().load().await;
    (rig.server, rig.core)
}

/// «Hoy» en el calendario es el de la organización que se mira. A las 03:00
/// UTC del 13 es 12 en México y 13 en Auckland; la máquina que corre la
/// prueba no puede estar en las dos.
///
/// Rotura que atrapa: medir «hoy» con el reloj de la máquina (o en UTC).
#[tokio::test]
async fn the_calendars_today_is_the_organizations() {
    let now = Utc.with_ymd_and_hms(2026, 11, 13, 3, 0, 0).unwrap();

    let (_west_server, west) = core_in_zone("America/Mexico_City").await;
    assert_eq!(west.my_work().today_at(now), day(2026, 11, 12));

    let (_east_server, east) = core_in_zone("Pacific/Auckland").await;
    assert_eq!(east.my_work().today_at(now), day(2026, 11, 13));
}

/// El número y su palabra se pintan aparte en un día del mes: «5» grande y
/// «tareas» pequeño. La palabra sale del mensaje entero, con su plural.
///
/// Roturas que atrapa: «5 5 tareas» (la palabra con el número dentro); un
/// plural fijo («1 tareas»); comerse una frase que no empieza por el número.
#[test]
fn a_count_gives_its_word_without_the_number() {
    use cac_core::text_count_unit;
    assert_eq!(text_count_unit(Locale::Es, "common:count.tasks", 5), "tareas");
    assert_eq!(text_count_unit(Locale::Es, "common:count.tasks", 1), "tarea");
    assert_eq!(text_count_unit(Locale::En, "common:count.tasks", 12), "tasks");
    assert_eq!(text_count_unit(Locale::Es, "common:count.restoredPages", 2), "Recuperadas 2 páginas");
}

/// «Mañana» es el día siguiente al hoy de la organización, también cuando
/// cruza de mes: a las 03:00 UTC del 1 de diciembre, en México sigue siendo
/// 30 de noviembre y mañana es 1 de diciembre.
///
/// Roturas que atrapa: un «Mañana» que pone la fecha de hoy; uno contado
/// desde el día de la máquina en vez del de la organización.
#[tokio::test]
async fn tomorrow_is_the_day_after_the_organizations_today() {
    let now = Utc.with_ymd_and_hms(2026, 12, 1, 3, 0, 0).unwrap();

    let (_west_server, west) = core_in_zone("America/Mexico_City").await;
    assert_eq!(west.my_work().tomorrow_at(now), day(2026, 12, 1));

    let (_east_server, east) = core_in_zone("Pacific/Auckland").await;
    assert_eq!(east.my_work().tomorrow_at(now), day(2026, 12, 2));
}

/// Soltar una tarjeta en un día: se ve movida antes de que conteste el
/// servidor, y lo que viaja es el día a medianoche UTC.
///
/// Roturas que atrapa: esperar la respuesta para moverla; mandar otro
/// instante que la medianoche UTC del día (el día correría); dejar sin
/// actualizar el día ya resuelto que lee la pantalla.
#[tokio::test]
async fn dropping_on_a_day_moves_the_card_before_the_server_answers() {
    let rig = rig(vec![wire("a", Some("2026-09-30T00:00:00Z"), json!({})), wire("b", None, json!({}))]).await;
    let answer = Gate::default();
    mount_patch(&rig.server, "a", answer.hold(ok())).await;
    mount_patch(&rig.server, "b", ok()).await;

    let work = rig.core.my_work();
    let dropping = tokio::spawn(async move { work.set_due_day("a", Some("2026-10-02")).await });
    until("se le pide al servidor", || answer.held() == 1).await;
    assert!(!dropping.is_finished(), "el servidor todavía no ha contestado");
    assert_eq!(due_of(&rig.core, "a"), (Some("2026-10-02".into()), Some("2026-10-02T00:00:00Z".into())));
    answer.open();
    dropping.await.unwrap().expect("se guardó");
    assert_eq!(due_of(&rig.core, "a").0.as_deref(), Some("2026-10-02"));

    // Y del carril a un día, lo mismo.
    rig.core.my_work().set_due_day("b", Some("2028-02-29")).await.expect("se guardó");
    assert_eq!(due_of(&rig.core, "b").0.as_deref(), Some("2028-02-29"));

    assert_eq!(bodies(&rig.server, "PATCH", "/api/v1/tasks/a").await, [json!({ "dueAt": "2026-10-02T00:00:00Z" })]);
    assert_eq!(bodies(&rig.server, "PATCH", "/api/v1/tasks/b").await, [json!({ "dueAt": "2028-02-29T00:00:00Z" })]);
}

/// Soltarla en «Por programar» le quita la fecha, con `clearDueAt`.
///
/// Roturas que atrapa: quitarla con `dueAt: null`, que el servidor lee como
/// «no tocar» (#95); dejarla con su día en la pantalla.
#[tokio::test]
async fn dropping_on_the_lane_clears_the_date() {
    let rig = rig(vec![wire("a", Some("2026-09-30T00:00:00Z"), json!({}))]).await;
    mount_patch(&rig.server, "a", ok()).await;

    rig.core.my_work().set_due_day("a", None).await.expect("se quitó");

    assert_eq!(due_of(&rig.core, "a"), (None, None));
    assert_eq!(bodies(&rig.server, "PATCH", "/api/v1/tasks/a").await, [json!({ "clearDueAt": true })]);
    assert_eq!(ids(&rig.core.my_work().unscheduled(&TaskFilter::default())), ["a"]);
}

/// Si el servidor no lo acepta, la tarjeta vuelve a donde estaba y el error
/// sale: al día que tenía, o al carril si no tenía.
///
/// Roturas que atrapa: dejar la tarjeta en el día nuevo tras un rechazo (la
/// pantalla diría algo que el servidor no guardó); tragarse el error; volver
/// a «sin fecha» lo que tenía una.
#[tokio::test]
async fn a_refused_drop_goes_back_and_says_why() {
    let rig = rig(vec![wire("dated", Some("2026-09-30T00:00:00Z"), json!({})), wire("undated", None, json!({}))]).await;
    mount_patch(&rig.server, "dated", refusal()).await;
    mount_patch(&rig.server, "undated", refusal()).await;
    let work = rig.core.my_work();

    let error = work.set_due_day("dated", Some("2026-10-02")).await.expect_err("el servidor dijo que no");
    assert!(matches!(&error, ApiError::Server { status: 409, code, .. } if code == "forbidden"), "{error:?}");
    assert_eq!(due_of(&rig.core, "dated"), (Some("2026-09-30".into()), Some("2026-09-30T00:00:00Z".into())));

    assert!(work.set_due_day("dated", None).await.is_err());
    assert_eq!(due_of(&rig.core, "dated").0.as_deref(), Some("2026-09-30"));

    assert!(work.set_due_day("undated", Some("2026-10-02")).await.is_err());
    assert_eq!(due_of(&rig.core, "undated"), (None, None));
}

/// Soltarla donde ya está no pide nada: en el día que tiene —llegue su
/// instante escrito como llegue— o en el carril si no tiene fecha.
///
/// Roturas que atrapa: pedir siempre; comparar el instante en vez del día
/// (el mismo día escrito desde otra zona parecería otro).
#[tokio::test]
async fn dropping_where_it_already_is_asks_for_nothing() {
    let rig = rig(vec![
        wire("a", Some("2026-09-30T00:00:00Z"), json!({})),
        wire("mexico", Some("2026-09-29T18:00:00-06:00"), json!({})),
        wire("undated", None, json!({})),
    ])
    .await;
    mount_patch(&rig.server, "a", ok()).await;
    mount_patch(&rig.server, "mexico", ok()).await;
    mount_patch(&rig.server, "undated", ok()).await;
    let work = rig.core.my_work();

    work.set_due_day("a", Some("2026-09-30")).await.expect("ya vencía ese día");
    work.set_due_day("mexico", Some("2026-09-30")).await.expect("ya vencía ese día");
    work.set_due_day("undated", None).await.expect("ya estaba sin fecha");

    let patches = rig.server.received_requests().await.unwrap().into_iter().filter(|request| request.method.as_str() == "PATCH").count();
    assert_eq!(patches, 0);
    assert_eq!(due_of(&rig.core, "mexico").1.as_deref(), Some("2026-09-29T18:00:00-06:00"), "no se tocó");
}

/// Un día que no existe, o una tarea que no está a la vista, no cambian nada
/// ni piden nada.
///
/// Roturas que atrapa: quitarle la fecha a la tarea porque el día no se
/// entendió (30 de febrero); pedir al servidor por una tarea que aquí no hay.
#[tokio::test]
async fn a_day_that_does_not_exist_changes_nothing() {
    let rig = rig(vec![wire("a", Some("2026-09-30T00:00:00Z"), json!({}))]).await;
    mount_patch(&rig.server, "a", ok()).await;
    mount_patch(&rig.server, "ghost", ok()).await;
    let work = rig.core.my_work();

    let error = work.set_due_day("a", Some("2026-02-30")).await.expect_err("febrero no tiene 30");
    assert!(matches!(&error, ApiError::Server { code, .. } if code == "invalid-due-day"), "{error:?}");
    assert!(work.set_due_day("a", Some("mañana")).await.is_err());
    assert!(work.set_due_day("ghost", Some("2026-10-02")).await.is_err());

    assert_eq!(due_of(&rig.core, "a").0.as_deref(), Some("2026-09-30"));
    let patches = rig.server.received_requests().await.unwrap().into_iter().filter(|request| request.method.as_str() == "PATCH").count();
    assert_eq!(patches, 0);
}

/// Dos gestos seguidos sobre la misma tarjeta: el primero tarda y acaba
/// rechazado, el segundo se guarda. El rechazo tardío no se lleva el día que
/// puso el segundo.
///
/// Rotura que atrapa: deshacer siempre al valor de antes del gesto, aunque
/// ya haya otro más nuevo puesto encima.
#[tokio::test]
async fn a_late_refusal_does_not_undo_a_newer_drop() {
    let rig = rig(vec![wire("a", Some("2026-09-30T00:00:00Z"), json!({}))]).await;
    let late = Gate::default();
    Mock::given(method("PATCH"))
        .and(path("/api/v1/tasks/a"))
        .and(body_json(json!({ "dueAt": "2026-10-02T00:00:00Z" })))
        .respond_with(late.hold(refusal()))
        .mount(&rig.server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/tasks/a"))
        .and(body_json(json!({ "dueAt": "2026-10-05T00:00:00Z" })))
        .respond_with(ok())
        .mount(&rig.server)
        .await;

    let work = rig.core.my_work();
    let first = tokio::spawn(async move { work.set_due_day("a", Some("2026-10-02")).await });
    until("el primero va en camino", || late.held() == 1).await;
    rig.core.my_work().set_due_day("a", Some("2026-10-05")).await.expect("el segundo se guardó");
    late.open();
    assert!(first.await.unwrap().is_err());

    assert_eq!(due_of(&rig.core, "a").0.as_deref(), Some("2026-10-05"));
}

/// Si la tarea que se movió es la que está abierta, su ficha se pone al día;
/// si es otra, no se pide nada de más.
///
/// Rotura que atrapa: dejar la ficha abierta enseñando el vencimiento de
/// antes de arrastrarla.
#[tokio::test]
async fn the_open_task_learns_its_new_day() {
    let rig = rig(vec![wire("a", Some("2026-09-30T00:00:00Z"), json!({})), wire("b", None, json!({}))]).await;
    let detail = json!({
        "task": {
            "id": "a", "createdAt": "2026-11-14T20:00:00Z", "updatedAt": "2026-11-14T20:00:00Z",
            "listId": "l1", "projectId": "", "seq": 1, "title": "Tarea a", "status": "pending",
            "priority": "medium", "visibility": "internal", "dueAt": "2026-10-02T00:00:00Z",
        },
        "listName": "Lista", "spaceName": "Espacio",
        "status": { "id": "s-pending", "listId": "l1", "name": "Open", "kind": "open", "status": "pending" },
        "tags": null, "assignees": null, "comments": null, "attachments": null, "subtasks": null,
        "blockedBy": null, "decisions": null,
    });
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": detail })))
        .mount(&rig.server)
        .await;
    mount_patch(&rig.server, "a", ok()).await;
    mount_patch(&rig.server, "b", ok()).await;
    rig.core.tasks().open("a").await;
    assert_eq!(bodies(&rig.server, "GET", "/api/v1/tasks/a").await.len(), 1);

    rig.core.my_work().set_due_day("b", Some("2026-10-02")).await.expect("se guardó");
    assert_eq!(bodies(&rig.server, "GET", "/api/v1/tasks/a").await.len(), 1, "la abierta es otra");

    rig.core.my_work().set_due_day("a", Some("2026-10-02")).await.expect("se guardó");
    assert_eq!(bodies(&rig.server, "GET", "/api/v1/tasks/a").await.len(), 2);
}

/// El detalle de un día del mes se abre hacia donde menos se esconde: hacia
/// abajo mientras quepa, hacia arriba cuando no cabe y arriba hay más sitio.
///
/// Roturas que atrapa: un detalle que se sale por abajo de una ventana baja
/// teniendo sitio arriba (lo que había: se decidía por la fila del mes, no
/// por el sitio); uno que se abre hacia arriba y se sale por arriba, o cuyo
/// día asoma a medias por abajo y se sale igual; que salte hacia arriba
/// cabiendo abajo.
#[test]
fn the_day_detail_opens_where_less_of_it_is_hidden() {
    use cac_core::day_detail_opens_up as up;
    // Lo que se ve va de 100 a 500, y el detalle mide 200. El día va de
    // `top` a `top + 96`; el detalle se le sale 8 por arriba o por abajo.
    let opens_up = |top: f32, height: f32| up(top - 8., top + 96. + 8., height, 100., 500.);
    assert!(!opens_up(150., 200.), "cabe hacia abajo y se abre hacia arriba");
    assert!(!opens_up(308., 200.), "cabe justo hacia abajo y se abre hacia arriba");
    assert!(opens_up(350., 200.), "no cabe hacia abajo, cabe hacia arriba y se abre hacia abajo");
    // Arriba del todo no hay sitio hacia arriba, aunque tampoco quepa abajo.
    assert!(!opens_up(110., 450.), "se abre hacia arriba y se sale por arriba");
    // No cabe de ninguna manera, pero hacia arriba se esconde menos.
    assert!(opens_up(380., 420.));
    // El día asoma a medias por abajo: hacia arriba se saldría por abajo
    // tanto como hacia abajo, y entonces no se mueve.
    assert!(!opens_up(480., 30.), "se abre hacia arriba para salirse igual");
    // Lo mismo de lado a lado: no depende de dónde empiece lo que se ve.
    assert!(up(-8., 104., 200., -300., 100.) && !up(-8., 104., 200., -300., 200.));
}
