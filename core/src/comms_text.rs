//! Lo que se lee en la fila de un canal o de un directo: la última línea en
//! limpio, su hora, si el otro la leyó y si anda por aquí.
//!
//! Vino de `app/src/lib/comms-preview.ts`, `lib/quote.ts`,
//! `components/chat/ReadTicks.tsx` y `lib/desde.ts` (`activo`). Son reglas
//! puras: ninguna pregunta al servidor.

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};

use crate::texts::{text, Locale};

/// Cuánto hace, como mucho, que alguien dio señales para decir que anda por
/// aquí: la `VENTANA_MIN` de `lib/desde.ts`.
///
/// No es «en línea ahora»: la marca se escribe como mucho cada cinco minutos.
const PRESENCE_WINDOW_MINUTES: i64 = 10;

/// Si `ch` es de palabra para una expresión regular de JavaScript (`\w`):
/// letra o cifra ASCII, o `_`. Una «á» no lo es, y así se queda: es la regla
/// con la que la app de siempre limpia el énfasis.
fn is_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Quita las vallas de código: de ``` hasta el final de su línea (el
/// lenguaje), con su salto. Lo de dentro se queda.
fn without_fences(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find("```") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 3..];
        rest = match after.find('\n') {
            Some(end) => &after[end + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// `[texto](url)` → `texto`, e `![nombre](url)` → `🖼 nombre`.
fn without_links(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut at = 0;
    while at < chars.len() {
        let image = chars[at] == '!' && chars.get(at + 1) == Some(&'[');
        let open = if image { at + 1 } else { at };
        if chars[open] == '[' {
            // `[` … `]` pegado a `(` … `)`, sin otro `]` ni otro `)` en medio.
            let close = (open + 1..chars.len()).find(|index| chars[*index] == ']');
            let end = close
                .filter(|close| chars.get(close + 1) == Some(&'('))
                .and_then(|close| (close + 2..chars.len()).find(|index| chars[*index] == ')'));
            if let (Some(close), Some(end)) = (close, end) {
                let label: String = chars[open + 1..close].iter().collect();
                if image {
                    out.push('🖼');
                    if !label.is_empty() {
                        out.push(' ');
                    }
                }
                out.push_str(&label);
                at = end + 1;
                continue;
            }
        }
        out.push(chars[at]);
        at += 1;
    }
    out
}

/// Lo que marca el principio de una línea: un encabezado, una cita, una
/// viñeta o un número con punto, tras hasta tres espacios y con uno detrás.
fn without_line_marker(line: &str) -> &str {
    let indent = line.chars().take(3).take_while(|ch| ch.is_whitespace()).count();
    let rest = &line[line.char_indices().nth(indent).map_or(line.len(), |(at, _)| at)..];
    let hashes = rest.chars().take_while(|ch| *ch == '#').count();
    let digits = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
    let marker = if (1..=6).contains(&hashes) {
        hashes
    } else if rest.starts_with(['>', '-', '*', '+']) {
        1
    } else if digits > 0 && rest[digits..].starts_with('.') {
        digits + 1
    } else {
        return line;
    };
    let after = &rest[marker..];
    // Sin espacio detrás no es un marcador: `#42`, `-5`, `*énfasis*`.
    if !after.is_empty() && !after.starts_with(char::is_whitespace) {
        return line;
    }
    after.trim_start()
}

/// `*así*` y `_así_` → `así`, sin tocar los guiones bajos de un nombre
/// (`snake_case_name`): el que abre va tras algo que no es de palabra, y el
/// que cierra, antes.
fn without_single_emphasis(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mark = |ch: char| ch == '*' || ch == '_';
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < chars.len() {
        let opens = mark(chars[at])
            && (at == 0 || !is_word(chars[at - 1]))
            && chars.get(at + 1).is_some_and(|next| !next.is_whitespace());
        if opens {
            let close = (at + 2..chars.len()).find(|index| mark(chars[*index]));
            if let Some(close) = close.filter(|close| chars.get(close + 1).is_none_or(|after| !is_word(*after))) {
                out.extend(&chars[at + 1..close]);
                at = close + 1;
                continue;
            }
        }
        out.push(chars[at]);
        at += 1;
    }
    out
}

/// Las entidades que escribe el editor (`<` se guarda `&lt;`), deshechas de
/// una pasada: `&amp;lt;` es `&lt;`, no `<`.
fn without_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let known = after.find(';').filter(|end| *end <= 8).and_then(|end| {
            let name = &after[..end];
            let shown = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "#39" | "apos" => Some('\''),
                "nbsp" => Some(' '),
                "amp" => Some('&'),
                _ => name.strip_prefix('#').and_then(|code| code.parse::<u32>().ok()).and_then(char::from_u32),
            };
            shown.map(|shown| (shown, end))
        });
        match known {
            Some((shown, end)) => {
                out.push(shown);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Lo que dice un mensaje, en una línea y sin markdown (`quoteText`).
///
/// Es para reconocer de qué se habla, no para leerlo entero: un `**` o un
/// `[texto](url)` crudos en una línea de 13 px son ruido. Una imagen queda
/// como su nombre.
pub fn quote_text(body: &str) -> String {
    let body = without_links(&without_fences(body));
    let lines: Vec<&str> = body.lines().map(without_line_marker).collect();
    let mut plain = lines.join("\n");
    for mark in ["**", "__", "~~", "`"] {
        plain = plain.replace(mark, "");
    }
    let plain = without_single_emphasis(&plain);
    let one_line = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    without_entities(&one_line)
}

/// La llamada que nombra el texto (`(cac:call/<id>)`): es el resumen que cac
/// deja en el canal al colgar (`callIdInBody`).
pub fn call_id_in(body: &str) -> Option<String> {
    const OPEN: &str = "(cac:call/";
    body.match_indices(OPEN).find_map(|(at, _)| {
        let rest = &body[at + OPEN.len()..];
        let id: String = rest.chars().take(36).collect();
        let whole = id.chars().count() == 36 && id.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-') && rest[id.len()..].starts_with(')');
        whole.then_some(id)
    })
}

/// Si el texto lleva la ficha de una llamada.
pub fn mentions_a_call(body: &str) -> bool {
    call_id_in(body).is_some()
}

/// La última línea de una conversación, tal como llega.
#[derive(Debug, Clone, Copy)]
pub struct LastLine<'a> {
    pub body: &'a str,
    /// La puso cac, no una persona.
    pub system: bool,
    pub author_name: &'a str,
    /// La escribió quien mira.
    pub mine: bool,
}

/// Lo que sale debajo del nombre de un canal o un directo (`previewLine`).
///
/// En un canal va el nombre («Bea: …»), porque hablan varios; en un directo
/// no, que ya se sabe quién es. Lo propio es «Tú: …». **Una línea del sistema
/// no la firma nadie**, ni siquiera «Tú»: la columna guarda a quien la
/// provocó, y «Tú: Grabación lista» diría que lo escribiste tú. El resumen
/// de una llamada se dice con palabras en vez de enseñar su markdown.
pub fn preview_line(last: LastLine, with_author: bool, locale: Locale) -> String {
    if last.system {
        return if mentions_a_call(last.body) { text(locale, "chat:callEnded") } else { quote_text(last.body) };
    }
    let said = quote_text(last.body);
    if last.mine {
        return format!("{}: {said}", text(locale, "chat:you"));
    }
    if with_author && !last.author_name.is_empty() {
        return format!("{}: {said}", last.author_name);
    }
    said
}

/// Un instante del servidor (`2026-10-15T10:00:00Z`).
pub(crate) fn instant(iso: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(iso).ok().map(|at| at.with_timezone(&Utc))
}

/// La hora de la fila (`listTime`): la del mensaje si es de hoy, «Ayer», el
/// día de la semana si es de esta semana, y la fecha si es de antes.
///
/// Con el día **de quien mira** (`now` va en su zona), no el de Greenwich:
/// un mensaje de las 23:59 de ayer es de ayer aunque en UTC ya fuera hoy.
/// Algo que no es una fecha no dice nada.
pub fn list_time<Viewer: TimeZone>(iso: &str, now: &DateTime<Viewer>, locale: Locale) -> String {
    let Some(at) = instant(iso) else { return String::new() };
    let at = at.with_timezone(&now.timezone());
    let days = (now.date_naive() - at.date_naive()).num_days();
    if days <= 0 {
        // Dos cifras en la hora y en el minuto, como `horaCorta`.
        return match locale {
            Locale::Es => format!("{:02}:{:02}", at.hour(), at.minute()),
            Locale::En => {
                let (pm, hour) = at.hour12();
                format!("{hour:02}:{:02} {}", at.minute(), if pm { "PM" } else { "AM" })
            }
        };
    }
    if days == 1 {
        return text(locale, "chat:yesterday");
    }
    if days < 7 {
        return crate::mywork::weekday_name(locale, at.weekday().num_days_from_monday(), true);
    }
    crate::dates::short_date(locale, at.date_naive())
}

/// Si esa persona anda por aquí: dio señales hace menos de diez minutos
/// (`activo`). Sin marca, no.
pub fn seen_recently(last_seen_at: Option<&str>, now: DateTime<Utc>) -> bool {
    last_seen_at.and_then(instant).is_some_and(|seen| now - seen < Duration::minutes(PRESENCE_WINDOW_MINUTES))
}

/// Si un mensaje escrito a `at` ya lo leyó quien leyó hasta `read_up_to`
/// (`wasRead`): leído es lo escrito antes de esa marca, o justo en ella.
pub fn was_read(at: Option<&str>, read_up_to: Option<&str>) -> bool {
    match (at.and_then(instant), read_up_to.and_then(instant)) {
        (Some(at), Some(read_up_to)) => at <= read_up_to,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    /// El texto de una cita: lo que dice, en una línea y sin el marcado. Los
    /// casos son los de `app/src/lib/quote.test.ts`.
    ///
    /// Roturas que atrapa: dejar el `[texto](url)` crudo; perder el texto de
    /// un enlace; dejar los `**`; no juntar las líneas; una imagen sin su
    /// nombre; comerse los guiones bajos de `snake_case_name`; dejar las
    /// vallas de un bloque de código o el lenguaje que llevan.
    #[test]
    fn a_message_is_said_in_one_clean_line() {
        assert_eq!(quote_text("**ojo**: mira [el PR](https://github.com/x/y/pull/1)\n\n- uno\n- dos"), "ojo: mira el PR uno dos");
        assert_eq!(quote_text("## Lanzamiento\n![captura.png](cacmedia://x/1)"), "Lanzamiento 🖼 captura.png");
        assert_eq!(quote_text("![](cacmedia://x/1) mira"), "🖼 mira");
        assert_eq!(quote_text("revisa DB_HOST y snake_case_name"), "revisa DB_HOST y snake_case_name");
        assert_eq!(quote_text("la columna user_id_ se queda"), "la columna user_id_ se queda");
        assert_eq!(quote_text("es *muy* importante y _urgente_."), "es muy importante y urgente.");
        assert_eq!(quote_text("```rust\nlet x = 1;\n```\ny ya"), "let x = 1; y ya");
        assert_eq!(quote_text("> cita\n1. primero\n2. segundo"), "cita primero segundo");
        assert_eq!(quote_text("issue #42 y -5 grados, a [medias](sin cerrar"), "issue #42 y -5 grados, a [medias](sin cerrar");
        assert_eq!(quote_text("~~no~~ `sí`"), "no sí");
    }

    /// Rotura que atrapa: no deshacer las entidades («&lt;3» en la lista de
    /// directos), o deshacer `&amp;` primero, que convertiría «&amp;lt;» en
    /// «<» en vez de en «&lt;».
    #[test]
    fn what_the_editor_escapes_is_read_as_what_it_is() {
        assert_eq!(quote_text("&lt;3 &amp; &quot;ok&quot; &#39;ya&#39;"), "<3 & \"ok\" 'ya'");
        assert_eq!(quote_text("escribe &amp;lt; tal cual"), "escribe &lt; tal cual");
        assert_eq!(quote_text("a &#241; b & c &desconocida; d"), "a ñ b & c &desconocida; d");
    }

    fn line<'a>(body: &'a str, system: bool, author_name: &'a str, mine: bool) -> LastLine<'a> {
        LastLine { body, system, author_name, mine }
    }

    /// La línea de debajo del nombre. Los casos son los de
    /// `app/src/lib/comms-preview.test.ts`.
    ///
    /// Roturas que atrapa: firmar en un directo; no decir «Tú» en lo propio;
    /// enseñar el markdown; **«Tú:» (o el nombre de quien la provocó) delante
    /// de una línea del sistema**; el resumen de una llamada con su enlace
    /// crudo; una ficha de llamada reconocida en un texto que no la lleva.
    #[test]
    fn the_line_under_a_name_says_who_said_it_unless_nobody_did() {
        assert_eq!(preview_line(line("**ojo** con [esto](https://x.y)", false, "Bea", false), true, Locale::Es), "Bea: ojo con esto");
        assert_eq!(preview_line(line("voy", false, "Ana", true), true, Locale::Es), "Tú: voy");
        assert_eq!(preview_line(line("voy", false, "Ana", true), true, Locale::En), "You: voy");
        assert_eq!(preview_line(line("¿vienes?", false, "Bea", false), false, Locale::Es), "¿vienes?");
        assert_eq!(preview_line(line("hola", false, "", false), true, Locale::Es), "hola");

        let call = "Call ended · 3 min — [details](cac:call/0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8).";
        assert_eq!(preview_line(line(call, true, "Ana", false), true, Locale::Es), "Llamada terminada");
        assert_eq!(preview_line(line("La grabación está lista", true, "Ana", false), true, Locale::Es), "La grabación está lista");
        // Aunque quien mira sea quien la provocó: no la escribió.
        assert_eq!(preview_line(line("La grabación está lista", true, "Ana", true), true, Locale::Es), "La grabación está lista");
        assert!(!mentions_a_call("mira (cac:call/no-es-un-id) y (cac:task/0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8)"));
        assert_eq!(preview_line(line("mira (cac:call/corto)", true, "", false), true, Locale::Es), "mira (cac:call/corto)");
    }

    /// De una frase con una llamada sale su id, y sólo el de una llamada.
    ///
    /// Roturas que atrapa: devolver el id de una tarea (`cac:task/`); uno
    /// cortado o con letras que no son de un id; el primer paréntesis en
    /// vez del que es una llamada.
    #[test]
    fn the_call_in_a_sentence_is_found_by_its_id() {
        let id = "0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8";
        assert_eq!(call_id_in(&format!("Call ended — [details](cac:call/{id}).")).as_deref(), Some(id));
        assert_eq!(call_id_in(&format!("(cac:call/corto) y (cac:call/{id})")).as_deref(), Some(id));
        assert_eq!(call_id_in(&format!("(cac:task/{id})")), None);
        assert_eq!(call_id_in(&format!("(cac:call/{id}x")), None);
        assert_eq!(call_id_in("(cac:call/zzzzzzzz-4d5e-6f70-8192-a3b4c5d6e7f8)"), None);
    }

    /// La hora de la fila, con el día de quien mira.
    ///
    /// Roturas que atrapa: contar los días en UTC (a un lado y a otro de
    /// Greenwich: un mensaje de ayer a las 23:59 saldría con su hora, o uno
    /// de hoy como «Ayer»); «Ayer» para lo de hace dos días; el día de la
    /// semana para lo de hace siete; la hora en el formato del otro idioma;
    /// algo escrito donde no hay fecha.
    #[test]
    fn the_time_of_a_row_is_counted_in_the_viewers_day() {
        // Las 10:00 de un jueves, 15 de octubre, para quien mira.
        for hours in [-6, 0, 13] {
            let zone = FixedOffset::east_opt(hours * 3600).unwrap();
            let now = zone.with_ymd_and_hms(2026, 10, 15, 10, 0, 0).unwrap();
            let at = |day: u32, hour: u32, minute: u32| zone.with_ymd_and_hms(2026, 10, day, hour, minute, 0).unwrap().with_timezone(&Utc).to_rfc3339();

            assert_eq!(list_time(&at(15, 0, 5), &now, Locale::Es), "00:05", "UTC{hours:+}");
            assert_eq!(list_time(&at(15, 21, 7), &now, Locale::En), "09:07 PM", "UTC{hours:+}");
            assert_eq!(list_time(&at(15, 0, 5), &now, Locale::En), "12:05 AM", "UTC{hours:+}");
            assert_eq!(list_time(&at(14, 23, 59), &now, Locale::Es), "Ayer", "UTC{hours:+}");
            assert_eq!(list_time(&at(14, 0, 1), &now, Locale::En), "Yesterday", "UTC{hours:+}");
            assert_eq!(list_time(&at(13, 12, 0), &now, Locale::Es), "mar", "UTC{hours:+}");
            assert_eq!(list_time(&at(12, 9, 0), &now, Locale::En), "Mon", "UTC{hours:+}");
            assert_eq!(list_time(&at(9, 9, 0), &now, Locale::Es), "vie", "UTC{hours:+}");
            assert_eq!(list_time(&at(8, 9, 0), &now, Locale::Es), "8/10/26", "UTC{hours:+}");
            assert_eq!(list_time(&at(1, 9, 0), &now, Locale::En), "10/1/26", "UTC{hours:+}");
            assert_eq!(list_time("no es fecha", &now, Locale::En), "");
        }
    }

    /// Roturas que atrapa: dar por leído lo escrito después de la marca del
    /// otro (la comparación al revés); leído sin marca; «anda por aquí» para
    /// quien se fue hace un cuarto de hora, o para quien nunca entró.
    #[test]
    fn read_is_what_was_written_before_the_others_mark_and_present_is_ten_minutes() {
        assert!(was_read(Some("2026-10-15T10:00:00Z"), Some("2026-10-15T10:00:05Z")));
        assert!(was_read(Some("2026-10-15T10:00:05Z"), Some("2026-10-15T10:00:05Z")));
        assert!(!was_read(Some("2026-10-15T10:00:10Z"), Some("2026-10-15T10:00:05Z")));
        assert!(!was_read(Some("2026-10-15T10:00:00Z"), None));
        assert!(!was_read(None, Some("2026-10-15T10:00:05Z")));

        let now = Utc.with_ymd_and_hms(2026, 10, 15, 10, 0, 0).unwrap();
        assert!(seen_recently(Some("2026-10-15T09:51:00Z"), now));
        assert!(!seen_recently(Some("2026-10-15T09:50:00Z"), now));
        assert!(!seen_recently(Some("2026-10-15T09:45:00Z"), now));
        assert!(!seen_recently(None, now));
        assert!(!seen_recently(Some("nunca"), now));
    }
}
