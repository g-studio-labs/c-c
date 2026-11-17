//! Las dos preguntas de fecha que ya han mordido, contestadas una sola vez.
//!
//! En qué día cae un vencimiento, y cuándo es «hoy» para decidir si venció.
//! Estaban en `app/src/lib/month.ts` y `due.ts`, y cada pantalla nueva era una
//! oportunidad de volver a leer un vencimiento con el reloj de la máquina.
//! Desde aquí salen como **días** (`NaiveDate`): un día no tiene zona, así que
//! no hay captador local con el que equivocarse.

use chrono::{DateTime, NaiveDate, Offset, TimeZone, Utc};

/// El día que alguien eligió como vencimiento, leído tal cual.
///
/// `dueAt` es **una fecha, no un instante**, pero se guarda como uno: el día a
/// medianoche UTC. Leído en la zona de quien mira, se corre un día entero al
/// oeste de Greenwich —eliges el 7 y en México se ve el 6; así pintó el
/// calendario en el 29 lo que se soltó en el 30—. El día es el de UTC, siempre.
pub fn due_day(due_at: Option<&str>) -> Option<NaiveDate> {
    let instant = DateTime::parse_from_rfc3339(due_at?).ok()?;
    Some(instant.with_timezone(&Utc).date_naive())
}

/// Qué día es hoy para un vencimiento: en la zona de la organización, no en
/// la de quien mira (decidido con jose el 13-nov-2026).
///
/// El día 15 es el 15 para todos; lo que depende de una zona es **cuándo se
/// acaba**. Medido con el reloj de cada uno, la misma tarea estaba vencida
/// para quien va una hora por delante y vigente para los demás.
///
/// Sin zona en la organización, o con una que no existe, es el reloj de quien
/// mira (`viewer`).
pub fn today_in<Viewer: TimeZone>(zone: Option<&str>, now: DateTime<Utc>, viewer: &Viewer) -> NaiveDate {
    match zone.and_then(|name| name.parse::<chrono_tz::Tz>().ok()) {
        Some(zone) => now.with_timezone(&zone).date_naive(),
        None => now.with_timezone(viewer).date_naive(),
    }
}

/// «hora de Mexico City», sólo para quien **no** está en la hora de la
/// organización: a los demás no hay nada que aclararles. Es lo que sale al
/// pasar el cursor por un vencimiento (`zoneHint` en `app/src/lib/due.ts`).
///
/// Se compara el desfase de ahora y no el nombre: Mérida y Ciudad de México
/// son la misma hora. Sin zona, o con una que no existe, no hay aviso.
pub fn zone_hint<Viewer: TimeZone>(locale: crate::texts::Locale, zone: Option<&str>, now: DateTime<Utc>, viewer: &Viewer) -> Option<String> {
    let name = zone?;
    let zone = name.parse::<chrono_tz::Tz>().ok()?;
    let offset = |seconds: chrono::FixedOffset| seconds.local_minus_utc();
    if offset(now.with_timezone(&zone).offset().fix()) == offset(now.with_timezone(viewer).offset().fix()) {
        return None;
    }
    // La ciudad es el último tramo del nombre, con espacios.
    let city = name.rsplit('/').next().unwrap_or(name).replace('_', " ");
    Some(crate::texts::text_with(locale, "work:due.zone", &[("city", &city)]))
}

/// Un día con números: «30/10/26». Es `fecha()` de `app/src/lib/fechas.ts`
/// (`dateStyle: "short"`): día, mes y año de dos cifras, sin ceros delante;
/// en inglés, el mes primero.
pub fn short_date(locale: crate::texts::Locale, day: NaiveDate) -> String {
    use chrono::Datelike;
    let year = day.year().rem_euclid(100);
    match locale {
        crate::texts::Locale::Es => format!("{}/{}/{year:02}", day.day(), day.month()),
        crate::texts::Locale::En => format!("{}/{}/{year:02}", day.month(), day.day()),
    }
}

/// Un momento con números y segundos: «11/11/26, 10:38:42». Es `fechaYHora()`
/// de `app/src/lib/fechas.ts` (`dateStyle: "short", timeStyle: "medium"`),
/// para sellos de tiempo: quién dijo qué y cuándo. En castellano el reloj es
/// de 24 horas; en inglés, de 12 con su «AM» o «PM».
///
/// El momento viene ya en la hora de quien mira: de eso se ocupa quien llama.
pub fn date_and_time(locale: crate::texts::Locale, at: chrono::NaiveDateTime) -> String {
    use chrono::Timelike;
    let day = short_date(locale, at.date());
    match locale {
        crate::texts::Locale::Es => format!("{day}, {}:{:02}:{:02}", at.hour(), at.minute(), at.second()),
        crate::texts::Locale::En => {
            let (afternoon, hour) = at.hour12();
            format!("{day}, {hour}:{:02}:{:02} {}", at.minute(), at.second(), if afternoon { "PM" } else { "AM" })
        }
    }
}

/// Cómo está un vencimiento respecto a hoy.
///
/// Vino de `cuando()` en `app/src/components/tasks/TaskCardMini.tsx`. El color
/// y el texto los pone cada interfaz; **cuál de los cinco casos es** se decide
/// aquí, para que una tarea no esté vencida en el escritorio y vigente en el
/// teléfono.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DueState {
    /// Venció antes de ayer o más atrás: se enseña su fecha, en rojo.
    Overdue,
    /// Venció ayer: «ayer», en rojo.
    Yesterday,
    /// Vence hoy: ámbar, y con peso.
    Today,
    /// Vence mañana: ámbar.
    Tomorrow,
    /// Más adelante: su fecha, sin color.
    Later,
}

impl DueState {
    /// Si ya pasó su día. Lo que vence hoy **no** está vencido: tiene el día
    /// entero.
    pub fn is_overdue(self) -> bool {
        matches!(self, DueState::Overdue | DueState::Yesterday)
    }
}

/// Clasifica un vencimiento, dado qué día es hoy (`today_in`).
pub fn due_state(due_at: Option<&str>, today: NaiveDate) -> Option<DueState> {
    let days = (due_day(due_at)? - today).num_days();
    Some(match days {
        0 => DueState::Today,
        -1 => DueState::Yesterday,
        1 => DueState::Tomorrow,
        days if days < -1 => DueState::Overdue,
        _ => DueState::Later,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Un día y un sello de tiempo salen con números, como en la app de
    /// siempre: `30/10/26` y `11/11/26, 10:38:42`.
    ///
    /// Roturas que atrapa: el mes con letras («11 nov»), que es otro formato;
    /// el año entero o sin su cero; ceros delante del día o del mes; el sello
    /// sin segundos, o con la hora rellenada a dos cifras; en inglés, el día
    /// antes del mes o el reloj de 24 horas; las doce de la noche como «0 AM».
    #[test]
    fn dates_are_said_with_numbers() {
        use crate::texts::Locale;
        let at = |h, m, s| day(2026, 11, 5).and_hms_opt(h, m, s).unwrap();
        assert_eq!(short_date(Locale::Es, day(2026, 10, 30)), "30/10/26");
        assert_eq!(short_date(Locale::Es, day(2005, 3, 1)), "1/3/05");
        assert_eq!(short_date(Locale::En, day(2026, 10, 30)), "10/30/26");
        assert_eq!(date_and_time(Locale::Es, day(2026, 11, 11).and_hms_opt(10, 38, 42).unwrap()), "11/11/26, 10:38:42");
        assert_eq!(date_and_time(Locale::Es, at(21, 7, 6)), "5/11/26, 21:07:06");
        assert_eq!(date_and_time(Locale::Es, at(8, 0, 0)), "5/11/26, 8:00:00");
        assert_eq!(date_and_time(Locale::En, at(21, 7, 6)), "11/5/26, 9:07:06 PM");
        assert_eq!(date_and_time(Locale::En, at(0, 30, 0)), "11/5/26, 12:30:00 AM");
        assert_eq!(date_and_time(Locale::En, at(12, 0, 1)), "11/5/26, 12:00:01 PM");
    }

    /// Los casos de `app/src/lib/month.test.ts`, «el día de vencimiento».
    ///
    /// Rotura que atrapa: leer el instante en otra zona que no sea UTC. Aquí
    /// no hay zona de la máquina que pueda colarse —por eso sale un día y no
    /// una fecha con hora—, así que la prueba escribe el instante de las tres
    /// maneras en que puede llegar.
    #[test]
    fn a_due_date_is_the_day_that_was_chosen() {
        assert_eq!(due_day(Some("2026-09-07T00:00:00.000Z")), Some(day(2026, 9, 7)));
        assert_eq!(due_day(Some("2026-09-30T00:00:00Z")), Some(day(2026, 9, 30)));
        // El mismo instante escrito desde México sigue siendo el 30.
        assert_eq!(due_day(Some("2026-09-29T18:00:00-06:00")), Some(day(2026, 9, 30)));
    }

    /// Rotura que atrapa: inventar una fecha cuando no la hay.
    #[test]
    fn without_a_due_date_there_is_no_day() {
        assert_eq!(due_day(None), None);
        assert_eq!(due_day(Some("")), None);
        assert_eq!(due_day(Some("no soy una fecha")), None);
    }

    /// Los casos de `app/src/lib/due.test.ts`: a las 03:00 UTC del 13 todavía
    /// es 12 en México y ya es 13 en Auckland.
    ///
    /// Rotura que atrapa: medir «hoy» con el reloj de quien mira cuando la
    /// organización tiene zona.
    #[test]
    fn today_is_the_organizations_day() {
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 3, 0, 0).unwrap();
        let viewer = FixedOffset::east_opt(13 * 3600).unwrap();
        assert_eq!(today_in(Some("America/Mexico_City"), now, &viewer), day(2026, 11, 12));
        assert_eq!(today_in(Some("Pacific/Auckland"), now, &viewer), day(2026, 11, 13));
        assert_eq!(today_in(Some("UTC"), now, &viewer), day(2026, 11, 13));
    }

    /// Rotura que atrapa: no caer al reloj de quien mira (por ejemplo, caer a
    /// UTC) cuando la organización no tiene zona o tiene una que no existe.
    #[test]
    fn without_a_zone_today_is_the_viewers_day() {
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 3, 0, 0).unwrap();
        let mexico = FixedOffset::west_opt(6 * 3600).unwrap();
        assert_eq!(today_in(None, now, &mexico), day(2026, 11, 12));
        assert_eq!(today_in(Some("Marte/Olympus"), now, &mexico), day(2026, 11, 12));
    }

    /// Los cinco casos, alrededor del 12 de noviembre.
    ///
    /// Roturas que atrapa: dar por vencido lo que vence hoy; contar «ayer»
    /// como una fecha más; pintar de «mañana» lo que vence pasado mañana.
    #[test]
    fn a_due_date_is_one_of_five_things() {
        let today = day(2026, 11, 12);
        let state = |iso: &str| due_state(Some(iso), today);

        assert_eq!(state("2026-11-10T00:00:00Z"), Some(DueState::Overdue));
        assert_eq!(state("2026-11-11T00:00:00Z"), Some(DueState::Yesterday));
        assert_eq!(state("2026-11-12T00:00:00Z"), Some(DueState::Today));
        assert_eq!(state("2026-11-13T00:00:00Z"), Some(DueState::Tomorrow));
        assert_eq!(state("2026-11-14T00:00:00Z"), Some(DueState::Later));
        assert_eq!(due_state(None, today), None);

        assert!(DueState::Overdue.is_overdue() && DueState::Yesterday.is_overdue());
        assert!(!DueState::Today.is_overdue(), "lo que vence hoy tiene el día entero");
    }

    /// El caso de `due.test.ts`: lo que vence el 12 vence hoy en México y ya
    /// venció en Auckland, en el mismo instante.
    ///
    /// Rotura que atrapa: clasificar con el día de quien mira y no con el de
    /// la organización.
    #[test]
    fn the_same_task_is_due_today_in_mexico_and_overdue_in_auckland() {
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 3, 0, 0).unwrap();
        let due = Some("2026-11-12T00:00:00.000Z");

        let mexico = today_in(Some("America/Mexico_City"), now, &Utc);
        let auckland = today_in(Some("Pacific/Auckland"), now, &Utc);

        assert_eq!(due_state(due, mexico), Some(DueState::Today));
        assert_eq!(due_state(due, auckland), Some(DueState::Yesterday));
    }

    /// El aviso de zona sólo sale para quien no está en la hora de la
    /// organización, y nombra su ciudad.
    ///
    /// Roturas que atrapa: avisar a quien ya está en esa hora (también desde
    /// otra ciudad con el mismo desfase); callar ante quien está en otra;
    /// enseñar «America/Mexico_City» en vez de la ciudad; inventar un aviso
    /// sin zona o con una que no existe.
    #[test]
    fn the_zone_hint_is_only_for_who_is_in_another_hour() {
        use crate::texts::Locale;
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 3, 0, 0).unwrap();
        let merida = FixedOffset::west_opt(6 * 3600).unwrap();
        let madrid = FixedOffset::east_opt(3600).unwrap();
        assert_eq!(zone_hint(Locale::Es, Some("America/Mexico_City"), now, &merida), None);
        assert_eq!(zone_hint(Locale::Es, Some("America/Mexico_City"), now, &madrid).as_deref(), Some("hora de Mexico City"));
        assert_eq!(zone_hint(Locale::Es, None, now, &madrid), None);
        assert_eq!(zone_hint(Locale::Es, Some("Marte/Olympus"), now, &madrid), None);
    }
}
