//! La cabecera de un documento: quién responde de él y cuándo se confirmó
//! por última vez.
//!
//! Son las dos preguntas que un markdown suelto no sabe contestar, y las que
//! deciden si alguien se fía de lo que lee (`DocHeader.tsx`). Aquí se dice
//! qué se pinta; cambiar de responsable, fijar una línea o firmar la
//! revisión siguen en la app de siempre.

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::dates::short_date;
use crate::docs::DocsState;
use crate::texts::{text, text_with, Locale};

/// El documento en sí, sin sus pestañas. Espejo de `domain.Doc`, con lo que
/// se lee de él.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Quién responde de él. `maintainer` y no `owner` porque el «owner» del
    /// modelo es el nodo del que cuelga; en pantalla es su responsable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintainer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintainer_name: Option<String>,
    /// Cuándo alguien confirmó que sigue siendo verdad. Editar no es revisar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_by_name: Option<String>,
    /// Una revisión pedida y todavía sin firmar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_requested_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_requested_by_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_request_note: Option<String>,
    /// Una línea sobre el tablero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_line: Option<String>,
    /// Lo calcula el servidor: la regla de los 90 días vive en un solo sitio.
    #[serde(default)]
    pub stale: bool,
}

/// La cabecera, dicha.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocHeader {
    /// Quién responde, o «Sin responsable».
    pub owner: String,
    pub has_owner: bool,
    /// «Revisado el 3/7/26», o que nunca se revisó. La fecha y no «hace
    /// tanto»: una fecha se puede comprobar contra lo que uno recuerda.
    pub reviewed: String,
    /// Lleva demasiado sin confirmar: el aviso va en ámbar.
    pub stale: bool,
    /// «Revisión pedida por Luis · 16/11/26», mientras nadie firme.
    pub review_requested: Option<String>,
    /// Qué cambió y qué mirar, si quien la pidió lo dijo.
    pub review_note: Option<String>,
    /// La línea fijada sobre el tablero, o la invitación a fijar una.
    pub pinned: String,
    pub has_pinned: bool,
    /// Por qué está sin revisar, no sólo cuántos días lleva: un número no
    /// le pide nada a nadie.
    pub stale_notice: Option<String>,
    /// Si a quien mira le toca firmar la revisión: hoy, un superadmin.
    pub can_sign: bool,
}

fn filled(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|value| !value.is_empty())
}

fn instant(value: &Option<String>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(filled(value)?).ok().map(|at| at.with_timezone(&Utc))
}

impl DocInfo {
    /// La cabecera en ese momento, para quien mira desde esa zona. Los días
    /// del aviso cuentan desde la última revisión y, si nunca la hubo, desde
    /// que se escribió — igual que la regla del servidor, para que el número
    /// que se lee y el color que se ve digan lo mismo.
    pub fn header_at<Viewer: TimeZone>(&self, locale: Locale, superadmin: bool, now: DateTime<Utc>, viewer: &Viewer) -> DocHeader {
        let day = |at: DateTime<Utc>| short_date(locale, at.with_timezone(viewer).date_naive());
        let reviewed_at = instant(&self.reviewed_at);
        let owner = filled(&self.maintainer_name);
        let pinned = filled(&self.pinned_line);
        let since = reviewed_at.or_else(|| instant(&self.updated_at));
        let days = since.map(|since| (now - since).num_days().max(0)).unwrap_or(0).to_string();
        DocHeader {
            owner: owner.map(str::to_string).unwrap_or_else(|| text(locale, "work:docs.noOwner")),
            has_owner: filled(&self.maintainer_id).is_some(),
            reviewed: match reviewed_at {
                Some(at) => text_with(locale, "work:docs.reviewedOn", &[("date", &day(at))]),
                None => text(locale, "work:docs.neverReviewed"),
            },
            stale: self.stale,
            review_requested: instant(&self.review_requested_at).map(|at| {
                let who = filled(&self.review_requested_by_name).map(str::to_string).unwrap_or_else(|| text(locale, "work:docs.someone"));
                text_with(locale, "work:docs.reviewRequested", &[("who", &who), ("date", &day(at))])
            }),
            review_note: filled(&self.review_request_note).map(str::to_string),
            pinned: pinned.map(str::to_string).unwrap_or_else(|| text(locale, "work:docs.pinEmpty")),
            has_pinned: pinned.is_some(),
            stale_notice: self.stale.then(|| {
                let key = if reviewed_at.is_some() { "work:docs.staleSinceReview" } else { "work:docs.staleNeverReviewed" };
                text_with(locale, key, &[("days", &days)])
            }),
            can_sign: superadmin,
        }
    }
}

impl DocsState {
    /// La cabecera del documento que se lee, si ya existe: en un nodo sin
    /// nada escrito todavía no hay nada que revisar. Es del documento
    /// entero, así que va sobre las pestañas y no sobre una página.
    pub fn header(&self, locale: Locale, superadmin: bool) -> Option<DocHeader> {
        if self.on_page() {
            return None;
        }
        Some(self.info.as_ref()?.header_at(locale, superadmin, Utc::now(), &Local))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(day: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(day).unwrap().with_timezone(&Utc)
    }

    fn info() -> DocInfo {
        DocInfo {
            id: "d1".into(),
            updated_at: Some("2026-06-01T12:00:00Z".into()),
            maintainer_id: Some("u1".into()),
            maintainer_name: Some("Ana Ruiz".into()),
            reviewed_at: Some("2026-07-03T12:00:00Z".into()),
            pinned_line: Some("Backend antes que app".into()),
            ..DocInfo::default()
        }
    }

    /// Un documento con dueño y revisado lo dice con su nombre y su fecha;
    /// uno sin nada de eso dice que no lo tiene, en vez de dejar un hueco.
    ///
    /// Roturas que atrapa: una pastilla en blanco donde no hay responsable;
    /// «Revisado el» sin fecha en uno que nunca se revisó; la línea fijada
    /// vacía sin decir que se puede fijar una; el instante crudo del
    /// servidor en vez de una fecha.
    #[test]
    fn the_header_says_who_answers_and_when_it_was_reviewed() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let now = at("2026-07-10T12:00:00Z");
        let full = info().header_at(Locale::Es, false, now, &utc);
        assert_eq!((full.owner.as_str(), full.has_owner), ("Ana Ruiz", true));
        assert_eq!(full.reviewed, "Revisado el 3/7/26");
        assert_eq!((full.pinned.as_str(), full.has_pinned), ("Backend antes que app", true));
        assert_eq!((full.stale, full.stale_notice, full.review_requested, full.review_note), (false, None, None, None));
        assert_eq!(info().header_at(Locale::En, false, now, &utc).reviewed, "Reviewed 7/3/26");

        let bare = DocInfo { id: "d1".into(), pinned_line: Some("  ".into()), ..DocInfo::default() }.header_at(Locale::Es, false, now, &utc);
        assert_eq!((bare.owner.as_str(), bare.has_owner), ("Sin responsable", false));
        assert_eq!(bare.reviewed, "Sin revisar nunca");
        assert_eq!((bare.pinned.as_str(), bare.has_pinned), ("Fijar una línea sobre el tablero", false));
    }

    /// El día de una fecha es el de quien la lee.
    ///
    /// Rotura que atrapa: «revisado el 4» para quien lo firmó el 3 por la
    /// tarde, al oeste de Greenwich.
    #[test]
    fn the_day_is_the_readers() {
        let mexico = FixedOffset::west_opt(6 * 3600).unwrap();
        let late = DocInfo { reviewed_at: Some("2026-07-04T02:00:00Z".into()), ..info() };
        assert_eq!(late.header_at(Locale::Es, false, at("2026-07-10T12:00:00Z"), &mexico).reviewed, "Revisado el 3/7/26");
    }

    /// El aviso de que está sin revisar dice por qué y cuántos días lleva:
    /// desde la última revisión o, si nunca la hubo, desde que se escribió.
    ///
    /// Roturas que atrapa: un aviso en un documento al día; contar desde que
    /// se editó cuando sí hubo revisión (editar no es revisar); decir «nadie
    /// lo ha confirmado en N días» de uno que nunca se confirmó.
    #[test]
    fn the_stale_notice_says_why_and_since_when() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let now = at("2026-11-18T12:00:00Z");
        let reviewed = DocInfo { stale: true, updated_at: Some("2026-11-01T12:00:00Z".into()), ..info() }.header_at(Locale::Es, false, now, &utc);
        assert!(reviewed.stale);
        assert_eq!(reviewed.stale_notice.as_deref(), Some(text_with(Locale::Es, "work:docs.staleSinceReview", &[("days", "138")]).as_str()));
        let never = DocInfo { stale: true, reviewed_at: None, ..info() }.header_at(Locale::Es, false, now, &utc);
        assert_eq!(never.stale_notice.as_deref(), Some(text_with(Locale::Es, "work:docs.staleNeverReviewed", &[("days", "170")]).as_str()));
        assert!(never.stale_notice.unwrap().starts_with("Escrito hace 170 días"));
    }

    /// Una revisión pedida dice quién la pidió y cuándo, con su nota; sin
    /// nombre, «alguien». Y firmarla sólo se le ofrece a quien puede.
    ///
    /// Roturas que atrapa: una petición de revisión que no se ve (si el
    /// aviso de la campana se pierde, se pierde con él); «pedida por » sin
    /// nadie; el botón de firmar a quien el servidor se lo va a rechazar.
    #[test]
    fn a_requested_review_says_who_asked() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let now = at("2026-11-18T12:00:00Z");
        let asked = DocInfo {
            review_requested_at: Some("2026-11-16T09:00:00Z".into()),
            review_requested_by_name: Some("Luis Gil".into()),
            review_request_note: Some(" Cambié el runbook ".into()),
            ..info()
        };
        let header = asked.header_at(Locale::Es, false, now, &utc);
        assert_eq!(header.review_requested.as_deref(), Some("Revisión pedida por Luis Gil · 16/11/26"));
        assert_eq!(header.review_note.as_deref(), Some("Cambié el runbook"));
        assert!(!header.can_sign && asked.header_at(Locale::Es, true, now, &utc).can_sign);
        let nameless = DocInfo { review_requested_by_name: None, ..asked }.header_at(Locale::Es, false, now, &utc);
        assert_eq!(nameless.review_requested.as_deref(), Some("Revisión pedida por alguien · 16/11/26"));
    }

    /// La cabecera es del documento entero: va sobre las pestañas, no sobre
    /// una página, y no la hay donde no hay documento.
    ///
    /// Rotura que atrapa: responsable y frescura sobre un nodo sin nada
    /// escrito, o repetidos encima de cada página.
    #[test]
    fn there_is_a_header_only_over_the_tabs_of_a_document_that_exists() {
        let with = DocsState { info: Some(info()), ..DocsState::default() };
        assert!(with.header(Locale::Es, false).is_some());
        assert!(DocsState::default().header(Locale::Es, false).is_none());
        assert!(DocsState { page_id: Some("p1".into()), ..with }.header(Locale::Es, false).is_none());
    }
}
