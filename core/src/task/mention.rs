//! Nombrar a alguien con `@` en un comentario.
//!
//! Vino de `app/src/components/markdown/mention-menu.ts` y
//! `mention-scope.ts`. Una mención es un enlace de markdown,
//! `[@jose](cac:user/<uuid>)`: es lo que el servidor busca en el texto para
//! avisar (`domain/mention.go`). Un `@jose` suelto no avisa a nadie.

use super::{client_reads, shown_name, Tasks, UserSummary};
use crate::mywork::filter::fold;

/// Cuántos se ofrecen como mucho.
const OFFERED: usize = 20;

/// Si ahí se ofrece nombrar a alguien.
///
/// En lo que lee un cliente no: pondría el nombre de un compañero delante de
/// quien no tenía que verlo, y quien escribe no tiene por qué notarlo. Ahí
/// `@` es una letra más.
pub fn mentions_allowed(client_reads: bool, internal: bool) -> bool {
    !client_reads || internal
}

/// Lo que la caja de comentar promete, según quién vaya a leerlo.
pub fn comment_placeholder(client_reads: bool, internal: bool) -> &'static str {
    if mentions_allowed(client_reads, internal) {
        "work:task.writeComment"
    } else {
        "work:task.writeCommentClient"
    }
}

/// El `@algo` que se está escribiendo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MentionQuery {
    /// Dónde está su `@`, en bytes desde el principio del texto.
    pub start: u32,
    /// Lo escrito detrás del `@`.
    pub query: String,
}

/// Alguien a quien se puede nombrar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MentionPerson {
    pub id: String,
    pub username: String,
    /// Cómo se le llama; el usuario si no ha puesto nombre.
    pub name: String,
}

/// El texto con la mención puesta, y dónde queda el cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MentionEdit {
    pub text: String,
    pub cursor: u32,
}

fn in_a_name(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// Si esa posición cae dentro de código: en un bloque entre vallas, o entre
/// comillas invertidas en su línea. Ahí se escribe una dirección de correo o
/// una variable, no se nombra a nadie.
fn in_code(text: &str, at: usize) -> bool {
    let before = &text[..at];
    let fences = before.lines().filter(|line| line.trim_start().starts_with("```")).count();
    if fences % 2 == 1 {
        return true;
    }
    let line = before.rsplit('\n').next().unwrap_or("");
    line.matches('`').count() % 2 == 1
}

/// El `@algo` que hay justo antes del cursor, si lo hay.
///
/// El `@` tiene que abrir palabra (principio de línea o tras un espacio): el
/// de `ana@correo.com` no nombra a nadie, ni el de una mención ya puesta.
pub fn mention_query(text: &str, cursor: usize) -> Option<MentionQuery> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let before = &text[..cursor];
    let start = before.rfind('@')?;
    let query = &before[start + 1..];
    if !query.chars().all(in_a_name) {
        return None;
    }
    if before[..start].chars().next_back().is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    if in_code(text, start) {
        return None;
    }
    Some(MentionQuery { start: start as u32, query: query.to_string() })
}

/// Del equipo, quién casa con lo escrito: por su usuario o por su nombre,
/// sin mayúsculas ni acentos. Con nada escrito, los primeros.
pub fn mention_matches(team: &[UserSummary], query: &str) -> Vec<MentionPerson> {
    let query = fold(query.trim());
    team.iter()
        .filter(|person| query.is_empty() || fold(&person.username).contains(&query) || fold(&shown_name(person)).contains(&query))
        .take(OFFERED)
        .map(|person| MentionPerson { id: person.id.clone(), username: person.username.clone(), name: shown_name(person) })
        .collect()
}

/// A quién se ofrece nombrar con ese texto y el cursor ahí. Nadie si ahí no
/// se nombra, o si no se está escribiendo un `@algo`.
pub fn offered_mentions(team: &[UserSummary], text: &str, cursor: usize, allowed: bool) -> Vec<MentionPerson> {
    if !allowed {
        return Vec::new();
    }
    mention_query(text, cursor).map(|at| mention_matches(team, &at.query)).unwrap_or_default()
}

/// La lista de a quién nombrar, abierta: quién sale y quién va marcado.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MentionMenu {
    /// Dónde está el `@` del que sale, en bytes: es lo que la distingue de
    /// la lista de otro `@`.
    pub start: u32,
    pub people: Vec<MentionPerson>,
    /// Cuál va marcado: el que Enter elige.
    pub highlighted: u32,
}

/// Lo que la pantalla recuerda de la lista entre una tecla y la siguiente.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MentionMark {
    /// De qué `@` es lo recordado. Con otro `@` no cuenta.
    pub start: u32,
    pub highlighted: u32,
    /// Se cerró con Escape: ese `@` ya no ofrece a nadie, se escriba lo que
    /// se escriba detrás. Quien la cerró quería escribir un `@`, no nombrar.
    pub closed: bool,
}

/// La lista que se enseña con ese texto y el cursor ahí, o `None` si no hay
/// a quién ofrecer o se cerró para ese `@`.
///
/// El marcado se queda donde estaba mientras sea el mismo `@`; si al seguir
/// escribiendo quedan menos, baja al último. Un `@` nuevo empieza por el
/// primero: es como lo hace el menú de la app de siempre (`Popup.update`).
pub fn mention_menu(team: &[UserSummary], text: &str, cursor: usize, allowed: bool, mark: Option<&MentionMark>) -> Option<MentionMenu> {
    if !allowed {
        return None;
    }
    let at = mention_query(text, cursor)?;
    let mark = mark.filter(|mark| mark.start == at.start);
    if mark.is_some_and(|mark| mark.closed) {
        return None;
    }
    let people = mention_matches(team, &at.query);
    if people.is_empty() {
        return None;
    }
    let last = people.len() as u32 - 1;
    let highlighted = mark.map_or(0, |mark| mark.highlighted.min(last));
    Some(MentionMenu { start: at.start, people, highlighted })
}

/// Lo recordado, mientras se siga escribiendo ese `@`. En cuanto deja de
/// escribirse (se borró, o el cursor se fue) se olvida: un `@` que se borra y
/// se vuelve a poner en el mismo sitio es otro, y vuelve a ofrecer.
pub fn mention_mark_kept(text: &str, cursor: usize, mark: Option<MentionMark>) -> Option<MentionMark> {
    let at = mention_query(text, cursor)?;
    mark.filter(|mark| mark.start == at.start)
}

/// La lista con el marcado movido tantos puestos (abajo, positivo). Da la
/// vuelta: del último se pasa al primero y del primero al último.
pub fn mention_moved(menu: &MentionMenu, by: i32) -> MentionMark {
    let count = menu.people.len().max(1) as i64;
    let highlighted = (menu.highlighted as i64 + by as i64).rem_euclid(count) as u32;
    MentionMark { start: menu.start, highlighted, closed: false }
}

/// La lista cerrada sin elegir a nadie.
pub fn mention_closed(menu: &MentionMenu) -> MentionMark {
    MentionMark { start: menu.start, highlighted: menu.highlighted, closed: true }
}

/// A quién elige Enter.
pub fn mention_highlighted(menu: &MentionMenu) -> Option<&MentionPerson> {
    menu.people.get(menu.highlighted as usize)
}

/// La mención tal como se guarda. Se menciona por el usuario, no por el
/// nombre: es el que identifica.
pub fn mention_link(person: &MentionPerson) -> String {
    format!("[@{}](cac:user/{})", person.username, person.id)
}

/// A quién nombra ese enlace, si es una mención. Con la misma forma que
/// busca el servidor: el prefijo exacto y un uuid, no «lo que venga detrás».
/// Quien pinta lo usa para no tratar una mención como un sitio al que ir.
pub fn mentioned_user(href: &str) -> Option<&str> {
    let id = href.strip_prefix("cac:user/")?;
    (id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).then_some(id)
}

/// Cambia el `@algo` que se escribía por la mención de esa persona, con un
/// espacio detrás para seguir escribiendo. `None` si en el cursor ya no hay
/// un `@algo` (se movió entre que se ofreció y se eligió).
pub fn with_mention(text: &str, cursor: usize, person: &MentionPerson) -> Option<MentionEdit> {
    let at = mention_query(text, cursor)?;
    let start = at.start as usize;
    let put = format!("{} ", mention_link(person));
    let out = format!("{}{put}{}", &text[..start], &text[cursor..]);
    Some(MentionEdit { text: out, cursor: (start + put.len()) as u32 })
}

impl Tasks {
    /// Si en un comentario de la tarea abierta se puede nombrar a alguien,
    /// según vaya a ser interno o no.
    pub fn mentions_allowed(&self, internal: bool) -> bool {
        self.state().detail.as_ref().is_some_and(|detail| mentions_allowed(client_reads(detail), internal))
    }

    /// A quién se ofrece nombrar en el comentario que se escribe en la tarea
    /// abierta: del equipo de su organización.
    pub fn offered_mentions(&self, text: &str, cursor: usize, internal: bool) -> Vec<MentionPerson> {
        offered_mentions(&self.state().team, text, cursor, self.mentions_allowed(internal))
    }

    /// La lista de a quién nombrar en el comentario que se escribe en la
    /// tarea abierta, con lo que la pantalla recuerda de ella.
    pub fn mention_menu(&self, text: &str, cursor: usize, internal: bool, mark: Option<&MentionMark>) -> Option<MentionMenu> {
        mention_menu(&self.state().team, text, cursor, self.mentions_allowed(internal), mark)
    }
}
