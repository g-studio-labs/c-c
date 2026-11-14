//! Las dos preguntas de fecha que ya han mordido, contestadas una sola vez.
//!
//! En qué día cae un vencimiento, y cuándo es «hoy» para decidir si venció.
//! Estaban en `app/src/lib/month.ts` y `due.ts`, y cada pantalla nueva era una
//! oportunidad de volver a leer un vencimiento con el reloj de la máquina.
//! Desde aquí salen como **días** (`NaiveDate`): un día no tiene zona, así que
//! no hay captador local con el que equivocarse.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
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
}
