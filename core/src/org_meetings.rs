//! Las reuniones periódicas de la organización (`OrgMeetings.tsx`,
//! `meetings.store.ts`, `meeting-time.ts` y `timezones.ts`).
//!
//! Lo que se programa aquí **suena**: a la hora exacta le sale a cada miembro
//! una tarjeta con timbre. Por eso la pantalla insiste en a qué hora es *de
//! verdad* para cada quien y a quién le va a sonar, y por eso la hora no es un
//! instante sino una hora de pared en una zona: «todos los lunes a las nueve»
//! no se mueve el día que cambia el horario. `next_fire_at` sí es un instante,
//! ya calculado por el servidor: se pinta, no se calcula aquí.

use chrono::{DateTime, NaiveDate, NaiveTime, Offset, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::mywork::{day_and_month, weekday_name};
use crate::orgs::Orgs;
use crate::texts::{text, text_count, text_with, Locale};

/// Una reunión periódica. Espejo de `Meeting` en `meetings.store.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: String,
    #[serde(default)]
    pub org_id: String,
    pub title: String,
    /// «09:00», hora de pared en `timezone`.
    pub wall_time: String,
    pub timezone: String,
    /// `daily`, `weekly` o `monthly`.
    pub freq: String,
    #[serde(default)]
    pub interval: i64,
    /// Días de la semana (0 domingo) separados por coma: «1,3,5». Sólo semanal.
    #[serde(default)]
    pub weekdays: Option<String>,
    #[serde(default)]
    pub month_day: Option<i64>,
    #[serde(default)]
    pub anchor: Option<String>,
    #[serde(default)]
    pub space_id: Option<String>,
    #[serde(default)]
    pub space_name: Option<String>,
    #[serde(default)]
    pub next_fire_at: String,
    #[serde(default)]
    pub paused: bool,
    /// A quién **no** le llega. Todos los demás están convocados.
    #[serde(default)]
    pub excluded_user_ids: Vec<String>,
}

/// Una sala a la que puede llevar el aviso.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct MeetingRoom {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
}

impl MeetingRoom {
    pub fn is_general(&self) -> bool {
        self.kind.as_deref() == Some("general")
    }
}

/// Lo que se manda al crear o editar. Lo que no se nombra no se toca.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDraft {
    pub title: String,
    pub wall_time: String,
    pub timezone: String,
    pub freq: String,
    pub interval: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekdays: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month_day: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Freq {
    Weekly,
    Monthly,
    /// Sólo para una que ya era «cada N días»: no cabe en días de la semana.
    Daily,
}

impl Freq {
    pub fn as_str(self) -> &'static str {
        match self {
            Freq::Weekly => "weekly",
            Freq::Monthly => "monthly",
            Freq::Daily => "daily",
        }
    }

    fn parse(raw: &str) -> Freq {
        match raw {
            "monthly" => Freq::Monthly,
            "daily" => Freq::Daily,
            _ => Freq::Weekly,
        }
    }
}

pub const WORKWEEK: [u32; 5] = [1, 2, 3, 4, 5];
pub const ALL_DAYS: [u32; 7] = [0, 1, 2, 3, 4, 5, 6];
/// Los días de lunes a domingo, con el número que les da el servidor.
pub const WEEK_ORDER: [u32; 7] = [1, 2, 3, 4, 5, 6, 0];

/// Lo que dice el formulario de una reunión.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingForm {
    pub title: String,
    pub time: String,
    pub zone: String,
    pub freq: Freq,
    pub days: Vec<u32>,
    pub month_day: u32,
    pub every: u32,
    pub room: String,
}

impl MeetingForm {
    /// Lo que dice el formulario de una reunión que ya existe, o el de una
    /// nueva en `default_zone`.
    ///
    /// «Diaria» cada 1 día es lo mismo que semanal con los siete días, y así
    /// se enseña: quitar el viernes de una «diaria» es desmarcarlo (#122).
    /// Una diaria cada N días (N > 1) no cabe en días de la semana y se queda
    /// como diaria.
    pub fn new(existing: Option<&Meeting>, default_zone: &str) -> MeetingForm {
        let Some(meeting) = existing else {
            return MeetingForm { title: String::new(), time: "09:00".into(), zone: default_zone.into(), freq: Freq::Weekly, days: WORKWEEK.to_vec(), month_day: 1, every: 1, room: String::new() };
        };
        let is_daily = meeting.freq == "daily" && meeting.interval <= 1;
        MeetingForm {
            title: meeting.title.clone(),
            time: meeting.wall_time.clone(),
            zone: meeting.timezone.clone(),
            freq: if is_daily { Freq::Weekly } else { Freq::parse(&meeting.freq) },
            days: if is_daily { ALL_DAYS.to_vec() } else { parse_weekdays(meeting.weekdays.as_deref().unwrap_or("")) },
            month_day: meeting.month_day.map_or(1, |day| day.clamp(1, 31) as u32),
            every: meeting.interval.max(1) as u32,
            room: meeting.space_id.clone().unwrap_or_default(),
        }
    }

    pub fn toggle_day(&mut self, day: u32) {
        match self.days.iter().position(|d| *d == day) {
            Some(at) => {
                self.days.remove(at);
            }
            None => self.days.push(day),
        }
    }

    pub fn is_workweek(&self) -> bool {
        same_days(&self.days, &WORKWEEK)
    }

    pub fn is_every_day(&self) -> bool {
        same_days(&self.days, &ALL_DAYS)
    }

    /// «Cada» se queda entre 1 y 52; lo que no es un número es 1.
    pub fn set_every(&mut self, raw: &str) {
        self.every = raw.trim().parse::<u32>().unwrap_or(1).clamp(1, 52);
    }

    pub fn set_month_day(&mut self, raw: &str) {
        self.month_day = raw.trim().parse::<u32>().unwrap_or(1).clamp(1, 31);
    }

    /// Una semanal sin ningún día no sonaría nunca.
    pub fn needs_a_day(&self) -> bool {
        self.freq == Freq::Weekly && self.days.is_empty()
    }

    pub fn can_save(&self) -> bool {
        !self.title.trim().is_empty() && !self.needs_a_day() && normalized_wall_time(&self.time).is_some() && is_zone(&self.zone)
    }

    /// Lo que se manda al guardar, o `None` si todavía no vale.
    pub fn draft(&self, existing: Option<&Meeting>, today: NaiveDate) -> Option<MeetingDraft> {
        if !self.can_save() {
            return None;
        }
        let mut days = self.days.clone();
        days.sort_unstable();
        days.dedup();
        Some(MeetingDraft {
            title: self.title.trim().to_string(),
            wall_time: normalized_wall_time(&self.time)?,
            timezone: self.zone.trim().to_string(),
            freq: self.freq.as_str().into(),
            interval: self.every,
            weekdays: (self.freq == Freq::Weekly).then(|| days.iter().map(u32::to_string).collect::<Vec<_>>().join(",")),
            month_day: (self.freq == Freq::Monthly).then_some(self.month_day),
            // El ancla del ciclo cuando se repite cada N: sin ella «cada dos
            // semanas» no dice cuál de las dos es. Hoy es una respuesta tan
            // buena como otra.
            anchor: (self.every > 1).then(|| existing.and_then(|m| m.anchor.clone()).filter(|anchor| !anchor.is_empty()).unwrap_or_else(|| today.format("%Y-%m-%d").to_string())),
            // Quitar la sala de una que ya la tenía se dice con «».
            space_id: if self.room.is_empty() { existing.map(|_| String::new()) } else { Some(self.room.clone()) },
        })
    }
}

fn same_days(a: &[u32], b: &[u32]) -> bool {
    a.len() == b.len() && b.iter().all(|day| a.contains(day))
}

fn parse_weekdays(raw: &str) -> Vec<u32> {
    raw.split(',').map(str::trim).filter(|part| !part.is_empty()).filter_map(|part| part.parse::<u32>().ok()).filter(|day| *day <= 6).collect()
}

/// «9:30» y «09:30» son la misma hora; lo que no es una hora, no es nada.
pub fn normalized_wall_time(raw: &str) -> Option<String> {
    let time = NaiveTime::parse_from_str(raw.trim(), "%H:%M").ok()?;
    Some(format!("{:02}:{:02}", time.hour(), time.minute()))
}

// ---- Zonas ----

/// La ciudad de una zona IANA: `America/Argentina/Buenos_Aires` → «Buenos Aires».
pub fn zone_city(zone: &str) -> String {
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

/// Si la base de zonas conoce el nombre.
pub fn is_zone(zone: &str) -> bool {
    zone.trim().parse::<Tz>().is_ok()
}

/// «UTC−5», «UTC+5:30», «UTC»: el desfase de la zona **en ese instante**, que
/// cambia con el horario de verano. Nunca aritmética de desfases a mano.
pub fn zone_offset(zone: &str, at: DateTime<Utc>) -> String {
    let Ok(tz) = zone.trim().parse::<Tz>() else { return String::new() };
    let seconds = at.with_timezone(&tz).offset().fix().local_minus_utc();
    if seconds == 0 {
        return "UTC".into();
    }
    let (hours, minutes) = (seconds.abs() / 3600, seconds.abs() % 3600 / 60);
    let sign = if seconds < 0 { "−" } else { "+" };
    if minutes == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{minutes:02}")
    }
}

/// «Cancun (UTC−5)».
pub fn zone_label(zone: &str, at: DateTime<Utc>) -> String {
    let offset = zone_offset(zone, at);
    if offset.is_empty() {
        zone_city(zone)
    } else {
        format!("{} ({offset})", zone_city(zone))
    }
}

/// La zona de la máquina, con el nombre que le da el sistema (`TZ`, o a dónde
/// apunta `/etc/localtime`). Sin saberla, UTC: quien muestre dos horas no
/// enseñará una segunda, que es peor que no enseñarla.
pub fn local_zone_name() -> String {
    let named = |name: &str| name.trim().trim_start_matches(':').parse::<Tz>().is_ok().then(|| name.trim().trim_start_matches(':').to_string());
    if let Some(zone) = std::env::var("TZ").ok().and_then(|tz| named(&tz)) {
        return zone;
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|target| target.to_string_lossy().split_once("zoneinfo/").map(|(_, name)| name.to_string()))
        .and_then(|name| named(&name))
        .unwrap_or_else(|| "UTC".into())
}

/// La zona en la que nace una reunión nueva: la del equipo, y si no hay, la
/// de quien la crea.
pub fn default_meeting_zone(team_zone: Option<&str>, viewer_zone: &str) -> String {
    team_zone.map(str::trim).filter(|zone| is_zone(zone)).unwrap_or(viewer_zone).to_string()
}

/// La hora de un instante en una zona: «09:30» en castellano, «09:30 AM» en
/// inglés.
pub fn hour_in(at: DateTime<Utc>, zone: &str, locale: Locale) -> String {
    let Ok(tz) = zone.trim().parse::<Tz>() else { return String::new() };
    let local = at.with_timezone(&tz);
    match locale {
        Locale::Es => format!("{:02}:{:02}", local.hour(), local.minute()),
        Locale::En => {
            let (afternoon, hour) = local.hour12();
            format!("{hour:02}:{:02} {}", local.minute(), if afternoon { "PM" } else { "AM" })
        }
    }
}

/// El instante en que hoy son las `time` en `zone`. Sirve para enseñar,
/// mientras se escribe la reunión, a qué hora le suena a cada quien.
pub fn wall_time_today(time: &str, zone: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let tz = zone.trim().parse::<Tz>().ok()?;
    let time = NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?;
    let naive = now.with_timezone(&tz).date_naive().and_time(time);
    let local = tz.from_local_datetime(&naive);
    Some(local.earliest().or_else(|| local.latest())?.with_timezone(&Utc))
}

/// La misma hora dicha dos veces: allí, y para quien mira si no coincide.
///
/// Se formatea **el mismo instante** dos veces y se compara el texto: Mérida y
/// Ciudad de México son la misma hora aunque las zonas se llamen distinto.
pub fn dual_time(at: DateTime<Utc>, zone: &str, viewer_zone: &str, locale: Locale) -> (String, Option<String>) {
    let (there, here) = (hour_in(at, zone, locale), hour_in(at, viewer_zone, locale));
    let city = zone_city(zone);
    let said_there = text_with(locale, "org:ringsThere", &[("time", &there), ("city", &city)]);
    let said_here = (zone != viewer_zone && there != here).then(|| text_with(locale, "org:ringsHere", &[("time", &here), ("city", &zone_city(viewer_zone))]));
    (said_there, said_here)
}

/// Lo que dice la ficha de a qué hora suena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingWhen {
    pub there: String,
    /// La hora para quien mira, o «tu zona» si es la misma.
    pub here: Option<String>,
    /// Si no es la zona del equipo, qué decir.
    pub off_team: Option<String>,
}

pub fn meeting_when(meeting: &Meeting, viewer_zone: &str, team_zone: Option<&str>, locale: Locale) -> MeetingWhen {
    let instant = DateTime::parse_from_rfc3339(&meeting.next_fire_at).ok().map(|at| at.with_timezone(&Utc)).filter(|_| is_zone(&meeting.timezone));
    let (there, here) = match instant {
        Some(at) => dual_time(at, &meeting.timezone, viewer_zone, locale),
        None => (text_with(locale, "org:ringsThere", &[("time", &meeting.wall_time), ("city", &zone_city(&meeting.timezone))]), None),
    };
    let here = here.or_else(|| (meeting.timezone == viewer_zone).then(|| text(locale, "org:yourZone")));
    let off_team = team_zone.filter(|team| !team.is_empty() && *team != meeting.timezone).map(|team| text_with(locale, "org:notTeamZone", &[("city", &zone_city(team))]));
    MeetingWhen { there, here, off_team }
}

/// Los días de «1,3,5» dichos como los diría una persona, de lunes a domingo:
/// «lun, mié, vie».
pub fn readable_weekdays(weekdays: &str, locale: Locale) -> String {
    let mut days = parse_weekdays(weekdays);
    days.sort_by_key(|day| (day + 6) % 7);
    days.dedup();
    days.iter().map(|day| weekday_name(locale, (day + 6) % 7, true)).collect::<Vec<_>>().join(", ")
}

/// La regla entera en una línea: «Semanal · lun, mié», «Cada 2 semanas · lun».
/// Cada caso es un mensaje entero con el número dentro: el catálogo decide su
/// forma.
pub fn readable_rule(meeting: &Meeting, locale: Locale) -> String {
    let every = meeting.interval.max(1);
    let mut parts = Vec::new();
    match meeting.freq.as_str() {
        "daily" => parts.push(if every > 1 { text_count(locale, "common:recurrence.everyDays", every) } else { text(locale, "common:recurrence.daily") }),
        "weekly" => {
            parts.push(if every > 1 { text_count(locale, "common:recurrence.everyWeeks", every) } else { text(locale, "common:recurrence.weekly") });
            let days = readable_weekdays(meeting.weekdays.as_deref().unwrap_or(""), locale);
            if !days.is_empty() {
                parts.push(days);
            }
        }
        "monthly" => {
            parts.push(if every > 1 { text_count(locale, "common:recurrence.everyMonths", every) } else { text(locale, "common:recurrence.monthly") });
            if let Some(day) = meeting.month_day.filter(|day| *day > 0) {
                parts.push(text_with(locale, "common:recurrence.onDay", &[("day", &day.to_string())]));
            }
        }
        // Una frecuencia que esta versión no conoce: enseñar el identificador
        // es feo, pero es información; una cadena vacía sería una regla
        // invisible.
        other => return other.to_string(),
    }
    parts.join(" · ")
}

/// «lun, 1 sept»: el día de la próxima vez, en la zona de quien mira.
pub fn next_ring(meeting: &Meeting, viewer: &impl TimeZone, locale: Locale) -> String {
    let Ok(at) = DateTime::parse_from_rfc3339(&meeting.next_fire_at) else { return String::new() };
    let day = at.with_timezone(viewer).date_naive();
    use chrono::Datelike;
    format!("{}, {}", weekday_name(locale, day.weekday().num_days_from_monday(), true), day_and_month(locale, day))
}

/// A cuántos de los miembros les llega.
pub fn reached(meeting: &Meeting, member_ids: &[&str]) -> usize {
    member_ids.iter().filter(|id| !meeting.excluded_user_ids.iter().any(|excluded| excluded == *id)).count()
}

/// La lista de quienes no reciben el aviso, con `user_id` cambiado de lado.
pub fn toggled_exclusion(meeting: &Meeting, user_id: &str) -> Vec<String> {
    if meeting.excluded_user_ids.iter().any(|id| id == user_id) {
        meeting.excluded_user_ids.iter().filter(|id| *id != user_id).cloned().collect()
    } else {
        meeting.excluded_user_ids.iter().cloned().chain(std::iter::once(user_id.to_string())).collect()
    }
}

impl Orgs {
    pub async fn meetings(&self, org_id: &str) -> Result<Vec<Meeting>, ApiError> {
        self.core.get_list(&format!("/api/v1/organizations/{org_id}/meetings/")).await
    }

    /// Las salas a las que puede llevar el aviso, con la general delante.
    pub async fn meeting_rooms(&self, org_id: &str) -> Result<Vec<MeetingRoom>, ApiError> {
        let mut rooms: Vec<MeetingRoom> = self.core.get_list(&format!("/api/v1/task-spaces/?orgId={org_id}")).await?;
        rooms.sort_by_key(|room| !room.is_general());
        Ok(rooms)
    }

    pub async fn create_meeting(&self, org_id: &str, draft: &MeetingDraft) -> Result<(), ApiError> {
        self.core.call_ok("POST", &format!("/api/v1/organizations/{org_id}/meetings/"), Some(serde_json::json!(draft))).await
    }

    pub async fn update_meeting(&self, id: &str, draft: &MeetingDraft) -> Result<(), ApiError> {
        self.core.call_ok("PATCH", &format!("/api/v1/meetings/{id}/"), Some(serde_json::json!(draft))).await
    }

    /// Pausar y reanudar mandan sólo la bandera: el servidor deja como está
    /// lo que no se nombra.
    pub async fn set_meeting_paused(&self, id: &str, paused: bool) -> Result<(), ApiError> {
        self.core.call_ok("PATCH", &format!("/api/v1/meetings/{id}/"), Some(serde_json::json!({ "paused": paused }))).await
    }

    pub async fn delete_meeting(&self, id: &str) -> Result<(), ApiError> {
        self.core.call_ok("DELETE", &format!("/api/v1/meetings/{id}/"), None).await
    }

    /// Reemplaza la lista de quienes no reciben el aviso.
    pub async fn set_meeting_excluded(&self, id: &str, user_ids: &[String]) -> Result<(), ApiError> {
        self.core.call_ok("PUT", &format!("/api/v1/meetings/{id}/recipients"), Some(serde_json::json!({ "excludedUserIds": user_ids }))).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(freq: &str, interval: i64) -> Meeting {
        Meeting {
            id: "m-1".into(),
            org_id: "o".into(),
            title: "Daily".into(),
            wall_time: "09:30".into(),
            timezone: "America/Mexico_City".into(),
            freq: freq.into(),
            interval,
            weekdays: None,
            month_day: None,
            anchor: None,
            space_id: None,
            space_name: None,
            next_fire_at: "2026-10-05T15:30:00Z".into(),
            paused: false,
            excluded_user_ids: vec![],
        }
    }

    fn utc(iso: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(iso).unwrap().with_timezone(&Utc)
    }

    /// Roturas que atrapa: enseñar una «diaria» como diaria (no se le podrían
    /// quitar días); perder el «cada N días» de una que no cabe en la semana;
    /// que la nueva nazca en otra zona que la que se le dio o sin entre
    /// semana.
    #[test]
    fn a_daily_is_edited_as_seven_days_and_a_new_one_is_a_workweek() {
        let daily = MeetingForm::new(Some(&meeting("daily", 1)), "UTC");
        assert_eq!((daily.freq, daily.days.clone(), daily.zone.as_str()), (Freq::Weekly, ALL_DAYS.to_vec(), "America/Mexico_City"));
        assert_eq!(MeetingForm::new(Some(&meeting("daily", 2)), "UTC").freq, Freq::Daily);
        let fresh = MeetingForm::new(None, "America/Cancun");
        assert_eq!((fresh.zone.as_str(), fresh.days.clone(), fresh.every), ("America/Cancun", WORKWEEK.to_vec(), 1));
        assert!(fresh.is_workweek() && !fresh.is_every_day());
    }

    /// Roturas que atrapa: mandar la regla vieja al quitar días de una
    /// diaria; los días sin ordenar; días en una mensual (o el día del mes en
    /// una semanal); la sala que no se puede quitar al editar; el ancla que
    /// se pierde en una «cada dos semanas» que ya la tenía.
    #[test]
    fn the_draft_carries_the_whole_new_rule() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let mut form = MeetingForm::new(Some(&meeting("daily", 1)), "UTC");
        for day in [5, 6, 0] {
            form.toggle_day(day);
        }
        form.zone = "America/Cancun".into();
        let draft = form.draft(Some(&meeting("daily", 1)), today).unwrap();
        assert_eq!(
            serde_json::to_value(&draft).unwrap(),
            serde_json::json!({ "title": "Daily", "wallTime": "09:30", "timezone": "America/Cancun", "freq": "weekly", "interval": 1, "weekdays": "1,2,3,4", "spaceId": "" })
        );

        let mut monthly = MeetingForm::new(None, "UTC");
        monthly.title = "Retro".into();
        monthly.freq = Freq::Monthly;
        monthly.set_month_day("31");
        monthly.room = "sp-1".into();
        let body = serde_json::to_value(monthly.draft(None, today).unwrap()).unwrap();
        assert_eq!(body["monthDay"], 31);
        assert_eq!(body["spaceId"], "sp-1");
        assert!(body.get("weekdays").is_none() && body.get("anchor").is_none() && body.get("spaceId").is_some());
        assert!(serde_json::to_value(MeetingForm { room: String::new(), ..monthly.clone() }.draft(None, today).unwrap()).unwrap().get("spaceId").is_none());

        let anchored = Meeting { anchor: Some("2026-09-21".into()), weekdays: Some("1".into()), ..meeting("weekly", 2) };
        let mut biweekly = MeetingForm::new(Some(&anchored), "UTC");
        assert_eq!(biweekly.draft(Some(&anchored), today).unwrap().anchor.as_deref(), Some("2026-09-21"));
        biweekly.every = 3;
        assert_eq!(biweekly.draft(None, today).unwrap().anchor.as_deref(), Some("2026-10-05"));
    }

    /// Roturas que atrapa: guardar una sin nombre, una semanal sin días (no
    /// sonaría nunca), una hora o una zona que no existen; «Cada» fuera de
    /// 1..52.
    #[test]
    fn what_would_never_ring_cannot_be_saved() {
        let mut form = MeetingForm::new(None, "America/Cancun");
        assert!(!form.can_save(), "sin nombre");
        form.title = "Daily".into();
        assert!(form.can_save());
        form.days.clear();
        assert!(form.needs_a_day() && !form.can_save());
        form.freq = Freq::Monthly;
        assert!(form.can_save(), "una mensual no usa los días");
        form.time = "25:00".into();
        assert!(!form.can_save());
        form.time = "9:05".into();
        assert_eq!(form.draft(None, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()).unwrap().wall_time, "09:05");
        form.zone = "Mars/Olympus".into();
        assert!(!form.can_save());
        form.set_every("0");
        assert_eq!(form.every, 1);
        form.set_every("99");
        assert_eq!(form.every, 52);
        form.set_every("x");
        assert_eq!(form.every, 1);
    }

    /// Roturas que atrapa: «UTC-5» con el signo ASCII; contar el horario de
    /// verano con el desfase de enero; perder los medios husos.
    #[test]
    fn a_zone_is_a_city_and_its_offset_that_day() {
        assert_eq!(zone_city("America/Argentina/Buenos_Aires"), "Buenos Aires");
        assert_eq!(zone_label("America/Cancun", utc("2026-10-05T12:00:00Z")), "Cancun (UTC−5)");
        assert_eq!(zone_offset("Asia/Kolkata", utc("2026-10-05T12:00:00Z")), "UTC+5:30");
        assert_eq!(zone_offset("UTC", utc("2026-10-05T12:00:00Z")), "UTC");
        assert_eq!(zone_offset("Europe/Madrid", utc("2026-07-01T12:00:00Z")), "UTC+2");
        assert_eq!(zone_offset("Europe/Madrid", utc("2026-01-01T12:00:00Z")), "UTC+1");
        assert_eq!(zone_label("Mars/Olympus", utc("2026-01-01T12:00:00Z")), "Olympus");
    }

    /// Roturas que atrapa: las nueve de «hoy» en la zona de quien mira y no
    /// en la de la reunión; el día que corre al cruzar medianoche UTC; la
    /// segunda hora sólo cuando de verdad cambia.
    #[test]
    fn the_preview_says_when_it_rings_there_and_for_you() {
        // 9:30 en Cancún, hoy 5-oct a las 02:00 UTC (en Cancún aún es el 4).
        let now = utc("2026-10-05T02:00:00Z");
        let at = wall_time_today("09:30", "America/Cancun", now).unwrap();
        assert_eq!(at, utc("2026-10-04T14:30:00Z"));
        let (there, here) = dual_time(at, "America/Cancun", "Europe/Madrid", Locale::En);
        assert_eq!(there, "09:30 AM in Cancun");
        assert_eq!(here.as_deref(), Some("04:30 PM for you (Madrid)"));
        let (_, same_hour) = dual_time(at, "America/Cancun", "America/Bogota", Locale::Es);
        assert!(same_hour.is_none(), "Bogotá y Cancún son la misma hora");
        assert!(dual_time(at, "America/Cancun", "America/Cancun", Locale::Es).1.is_none());
        assert!(wall_time_today("09:30", "Mars/Olympus", now).is_none());
    }

    /// Roturas que atrapa: no decir la ciudad; no avisar de que está fuera de
    /// la zona del equipo; «tu zona» cuando sí hay otra hora que enseñar.
    #[test]
    fn a_card_says_the_city_and_warns_when_it_is_not_the_team_zone() {
        let daily = meeting("daily", 1);
        let when = meeting_when(&daily, "Europe/Madrid", Some("America/Cancun"), Locale::En);
        assert_eq!(when.there, "09:30 AM in Mexico City");
        assert_eq!(when.here.as_deref(), Some("05:30 PM for you (Madrid)"));
        assert_eq!(when.off_team.as_deref(), Some("Not in the team time zone (Cancun): edit it if it should be."));
        let mine = meeting_when(&daily, "America/Mexico_City", Some("America/Mexico_City"), Locale::Es);
        assert_eq!((mine.here.as_deref(), mine.off_team), (Some("tu zona"), None));
        assert!(meeting_when(&daily, "UTC", None, Locale::Es).off_team.is_none());
        let broken = meeting_when(&Meeting { next_fire_at: "nope".into(), ..daily }, "UTC", None, Locale::Es);
        assert_eq!(broken.there, "09:30 en Mexico City");
    }

    /// Roturas que atrapa: los días en el orden del servidor (domingo
    /// primero); leer una regla sin días como «domingo»; contar «cada 1
    /// semana».
    #[test]
    fn a_rule_reads_as_a_person_would_say_it() {
        let weekly = Meeting { weekdays: Some("0,3,1".into()), ..meeting("weekly", 1) };
        assert_eq!(readable_rule(&weekly, Locale::Es), "Semanal · lun, mié, dom");
        assert_eq!(readable_rule(&Meeting { interval: 2, weekdays: Some("1".into()), ..weekly.clone() }, Locale::En), "Every 2 weeks · Mon");
        assert_eq!(readable_rule(&Meeting { weekdays: Some(String::new()), ..weekly.clone() }, Locale::Es), "Semanal");
        assert_eq!(readable_rule(&Meeting { month_day: Some(31), ..meeting("monthly", 1) }, Locale::Es), "Mensual · el día 31");
        assert_eq!(readable_rule(&meeting("daily", 3), Locale::Es), "Cada 3 días");
        assert_eq!(readable_rule(&meeting("yearly", 1), Locale::Es), "yearly");
    }

    /// Roturas que atrapa: contar como convocado a quien se quitó; que
    /// marcar y desmarcar no sea simétrico.
    #[test]
    fn who_is_reached_is_everyone_but_the_unticked() {
        let m = Meeting { excluded_user_ids: vec!["u2".into()], ..meeting("daily", 1) };
        assert_eq!(reached(&m, &["u1", "u2", "u3"]), 2);
        assert_eq!(toggled_exclusion(&m, "u2"), Vec::<String>::new());
        assert_eq!(toggled_exclusion(&m, "u3"), ["u2", "u3"]);
    }

    /// Rotura que atrapa: el día de la semana de la próxima vez en la zona
    /// de la máquina y no en la de la reunión.
    #[test]
    fn the_next_ring_is_dated_where_the_viewer_is() {
        let m = meeting("daily", 1);
        assert_eq!(next_ring(&m, &chrono::FixedOffset::east_opt(13 * 3600).unwrap(), Locale::En), "Tue, Oct 6");
        assert_eq!(next_ring(&m, &chrono::FixedOffset::west_opt(6 * 3600).unwrap(), Locale::En), "Mon, Oct 5");
    }
}
