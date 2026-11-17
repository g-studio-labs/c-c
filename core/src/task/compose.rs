//! Lo que hace cada botón de la barra del editor con el markdown que se
//! escribe.
//!
//! En la app de siempre el editor es tiptap (`MarkdownEditor.tsx`): cada
//! botón cambia un documento y el markdown sale al guardar. Aquí se escribe
//! el markdown tal cual (decidido con jose el 17-nov-2026), así que cada
//! botón le hace al texto lo que el de tiptap le hace al documento. Lo que se
//! guarda es lo mismo.
//!
//! **Las posiciones son bytes de UTF-8** desde el principio del texto, como
//! en `mention.rs` y como las da el campo de texto del escritorio
//! (`selected_range`). Una que cae en mitad de una letra (una «ñ» son dos
//! bytes, un emoji cuatro) se lleva al principio de esa letra: partirla
//! sería un pánico, o un texto roto.

/// Un botón de la barra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ComposeAction {
    Bold,
    Italic,
    Code,
    /// Un encabezado de segundo nivel, el único que la barra ofrece.
    Heading,
    BulletList,
    OrderedList,
    TaskList,
    Quote,
    Table,
    Link,
}

/// El texto ya cambiado, y lo que queda seleccionado (en bytes). Con
/// `start == end` es el cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ComposeEdit {
    pub text: String,
    pub start: u32,
    pub end: u32,
}

fn edit(text: String, start: usize, end: usize) -> ComposeEdit {
    ComposeEdit { text, start: start as u32, end: end as u32 }
}

/// Esa posición, dentro del texto y al principio de su letra.
fn floor(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// La selección en orden y en posiciones que se pueden cortar.
fn selection(text: &str, start: usize, end: usize) -> (usize, usize) {
    let (start, end) = (floor(text, start), floor(text, end));
    (start.min(end), start.max(end))
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |newline| newline + 1)
}

fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |newline| at + newline)
}

/// Qué clase de bloque es una línea, por cómo empieza.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    Plain,
    Heading(usize),
    Bullet,
    Ordered(u32),
    /// Con su casilla marcada o no.
    Task(bool),
}

/// Una línea partida en su sangría, si es cita, su bloque y lo que dice.
#[derive(Debug, Clone, Copy)]
struct Line<'a> {
    indent: &'a str,
    quoted: bool,
    block: Block,
    content: &'a str,
    /// Cuánto ocupa todo lo que va antes de lo que dice.
    prefix: usize,
}

impl Line<'_> {
    fn is_blank(&self) -> bool {
        !self.quoted && self.block == Block::Plain && self.content.trim().is_empty()
    }
}

/// Lo que queda tras `marker` si va seguido de un espacio o acaba la línea.
fn after_marker<'a>(rest: &'a str, marker: &str) -> Option<&'a str> {
    let rest = rest.strip_prefix(marker)?;
    if rest.is_empty() {
        Some(rest)
    } else {
        rest.strip_prefix(' ')
    }
}

fn parse_block(rest: &str) -> (Block, &str) {
    let hashes = rest.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hashes) {
        if let Some(content) = after_marker(rest, &rest[..hashes]) {
            return (Block::Heading(hashes), content);
        }
    }
    for bullet in ["- ", "* ", "+ "] {
        let Some(item) = rest.strip_prefix(bullet) else { continue };
        for (box_, checked) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
            if let Some(content) = after_marker(item, box_) {
                return (Block::Task(checked), content);
            }
        }
        return (Block::Bullet, item);
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let content = rest[digits..].strip_prefix(". ").or_else(|| rest[digits..].strip_prefix(") "));
        if let (Some(content), Ok(number)) = (content, rest[..digits].parse()) {
            return (Block::Ordered(number), content);
        }
    }
    (Block::Plain, rest)
}

fn parse_line(line: &str) -> Line<'_> {
    let rest = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - rest.len()];
    let (quoted, rest) = match rest.strip_prefix('>') {
        Some(inside) => (true, inside.strip_prefix(' ').unwrap_or(inside)),
        None => (false, rest),
    };
    let (block, content) = parse_block(rest);
    Line { indent, quoted, block, content, prefix: line.len() - content.len() }
}

fn prefix_of(indent: &str, quoted: bool, block: Block) -> String {
    let marker = match block {
        Block::Plain => String::new(),
        Block::Heading(level) => format!("{} ", "#".repeat(level)),
        Block::Bullet => "- ".to_string(),
        Block::Ordered(number) => format!("{number}. "),
        Block::Task(false) => "- [ ] ".to_string(),
        Block::Task(true) => "- [x] ".to_string(),
    };
    format!("{indent}{}{marker}", if quoted { "> " } else { "" })
}

/// Si esa posición cae dentro de un bloque de código entre vallas: ahí una
/// línea que empieza por `- ` es código, no una lista.
fn in_fence(text: &str, at: usize) -> bool {
    text[..at].lines().filter(|line| line.trim_start().starts_with("```")).count() % 2 == 1
}

/// Un cambio pequeño del texto: en `at`, se quitan `remove` bytes y se pone
/// `insert`.
struct Patch {
    at: usize,
    remove: usize,
    insert: String,
}

/// El texto con esos cambios, que vienen en orden y sin pisarse.
fn patched(text: &str, patches: &[Patch]) -> String {
    let mut out = String::with_capacity(text.len() + patches.iter().map(|patch| patch.insert.len()).sum::<usize>());
    let mut done = 0;
    for patch in patches {
        out.push_str(&text[done..patch.at]);
        out.push_str(&patch.insert);
        done = patch.at + patch.remove;
    }
    out.push_str(&text[done..]);
    out
}

/// Dónde queda una posición del texto de antes. `after` dice de qué lado de
/// un cambio que está justo en ella: detrás (lo que abre una selección) o
/// delante (lo que la cierra).
fn moved(patches: &[Patch], at: usize, after: bool) -> usize {
    let mut out = at as isize;
    for patch in patches {
        let counts = if after { patch.at + patch.remove <= at } else { patch.at < at };
        if counts {
            out += patch.insert.len() as isize - patch.remove as isize;
        }
    }
    out.max(0) as usize
}

// ---- Negrita, cursiva y código: marcas que envuelven ----

/// Cuántas veces seguidas está `mark` justo antes de `at`, y justo después.
fn run_before(text: &str, at: usize, mark: char) -> usize {
    text[..at].chars().rev().take_while(|c| *c == mark).count()
}

fn run_after(text: &str, at: usize, mark: char) -> usize {
    text[at..].chars().take_while(|c| *c == mark).count()
}

/// Si con tantas marcas a cada lado, ésta está puesta. Los asteriscos se
/// cuentan: `**` es negrita, `*` cursiva y `***` las dos, así que la cursiva
/// está con uno o con tres y la negrita con dos o más.
fn is_on(marker: &str, count: usize) -> bool {
    match marker {
        "**" => count >= 2,
        "*" => count % 2 == 1,
        _ => count >= 1,
    }
}

/// Dónde tiene un trozo sus marcas, a cada lado: dentro de lo seleccionado
/// (se seleccionó con ellas) o fuera. No tienen por qué coincidir: de una
/// selección de varias líneas, la primera las lleva fuera por delante y
/// dentro por detrás.
struct Wrapped {
    left_inside: bool,
    right_inside: bool,
}

fn wrapped(text: &str, from: usize, to: usize, marker: &str) -> Option<Wrapped> {
    let mark = marker.chars().next()?;
    let piece = &text[from..to];
    let (before, after) = (run_before(text, from, mark), run_after(text, to, mark));
    let (leading, trailing) = (run_after(piece, 0, mark), run_before(piece, piece.len(), mark));
    let (left, right) = (if before > 0 { before } else { leading }, if after > 0 { after } else { trailing });
    let state = Wrapped { left_inside: before == 0, right_inside: after == 0 };
    // Un trozo que es sólo marcas no tiene nada envuelto.
    let fits = !(state.left_inside && state.right_inside) || leading + trailing <= piece.len();
    (fits && is_on(marker, left.min(right))).then_some(state)
}

/// Lo seleccionado, línea a línea y sin lo que no es texto: ni el espacio de
/// los lados (`** hola**` no es negrita) ni el `- ` de una lista. Una marca
/// no cruza de un párrafo a otro, así que cada línea lleva las suyas.
fn pieces(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = start;
    while at < end {
        let stop = line_end(text, at).min(end);
        let mut from = at;
        if from == line_start(text, from) {
            let whole = &text[from..line_end(text, from)];
            from = (from + parse_line(whole).prefix).min(stop);
        }
        let piece = &text[from..stop];
        let from = from + (piece.len() - piece.trim_start().len());
        let to = from + piece.trim().len();
        if from < to {
            out.push((from, to));
        }
        at = stop + 1;
    }
    out
}

fn toggle_inline(text: &str, start: usize, end: usize, marker: &str) -> ComposeEdit {
    let size = marker.len();
    let pieces = pieces(text, start, end);
    if pieces.is_empty() {
        // Sin nada seleccionado, las marcas con el cursor en medio: lo que se
        // escriba sale con ella, como al pulsar el botón en tiptap. Y si el
        // cursor ya está entre dos recién puestas, se quitan.
        let mark = marker.chars().next().unwrap_or('*');
        if is_on(marker, run_before(text, start, mark).min(run_after(text, start, mark))) {
            let out = format!("{}{}", &text[..start - size], &text[start + size..]);
            return edit(out, start - size, start - size);
        }
        let out = format!("{}{marker}{marker}{}", &text[..start], &text[start..]);
        return edit(out, start + size, start + size);
    }
    let states: Vec<Option<Wrapped>> = pieces.iter().map(|(from, to)| wrapped(text, *from, *to, marker)).collect();
    let remove = states.iter().all(Option::is_some);
    let mut patches = Vec::new();
    // Lo que queda seleccionado: lo de dentro de las marcas, de la primera
    // línea a la última.
    let (mut first, mut last) = (None, 0);
    for ((from, to), state) in pieces.iter().copied().zip(&states) {
        let (inner_from, inner_to) = match (remove, state) {
            (true, Some(state)) => {
                let inner_from = if state.left_inside { from + size } else { from };
                let inner_to = if state.right_inside { to - size } else { to };
                patches.push(Patch { at: inner_from - size, remove: size, insert: String::new() });
                patches.push(Patch { at: inner_to, remove: size, insert: String::new() });
                (inner_from, inner_to)
            }
            (false, None) => {
                patches.push(Patch { at: from, remove: 0, insert: marker.to_string() });
                patches.push(Patch { at: to, remove: 0, insert: marker.to_string() });
                (from, to)
            }
            // Ya la tenía (o no hay nada que quitar): se queda como está.
            _ => (from, to),
        };
        first.get_or_insert(inner_from);
        last = inner_to;
    }
    let first = first.unwrap_or(start);
    edit(patched(text, &patches), moved(&patches, first, true), moved(&patches, last, false))
}

// ---- Encabezado, listas y cita: lo que va al principio de la línea ----

fn toggle_lines(text: &str, start: usize, end: usize, action: ComposeAction) -> ComposeEdit {
    let first = line_start(text, start);
    // Una selección que acaba justo donde empieza una línea no la incluye:
    // es lo que queda al seleccionar líneas enteras.
    let last = if end > start && end > first && end == line_start(text, end) { end - 1 } else { line_end(text, end) };
    let mut lines = Vec::new();
    let mut at = first;
    for line in text[first..last].split('\n') {
        lines.push((at, parse_line(line)));
        at += line.len() + 1;
    }
    // Las líneas en blanco de en medio no se tocan; si todo es blanco (una
    // línea vacía, lo normal al empezar), se pone ahí.
    let all_blank = lines.iter().all(|(_, line)| line.is_blank());
    let touched = |line: &Line| all_blank || !line.is_blank();
    let has = |line: &Line| match action {
        ComposeAction::Heading => line.block == Block::Heading(2),
        ComposeAction::BulletList => line.block == Block::Bullet,
        ComposeAction::OrderedList => matches!(line.block, Block::Ordered(_)),
        ComposeAction::TaskList => matches!(line.block, Block::Task(_)),
        _ => line.quoted,
    };
    let remove = lines.iter().filter(|(_, line)| touched(line)).all(|(_, line)| has(line));
    // Una lista numerada sigue la cuenta de la línea de encima, si es suya.
    let mut number = 1;
    if let (Some((_, top)), true) = (lines.iter().find(|(_, line)| touched(line)), first > 0) {
        let above = parse_line(&text[line_start(text, first - 1)..first - 1]);
        if let (Block::Ordered(before), true) = (above.block, above.indent == top.indent && above.quoted == top.quoted) {
            number = before + 1;
        }
    }
    let mut patches = Vec::new();
    for (at, line) in lines.iter().filter(|(_, line)| touched(line)) {
        let (quoted, block) = match (action, remove) {
            (ComposeAction::Quote, _) => (!remove, line.block),
            (_, true) => (line.quoted, Block::Plain),
            (ComposeAction::Heading, _) => (line.quoted, Block::Heading(2)),
            (ComposeAction::BulletList, _) => (line.quoted, Block::Bullet),
            (ComposeAction::TaskList, _) => (line.quoted, Block::Task(matches!(line.block, Block::Task(true)))),
            _ => {
                number += 1;
                (line.quoted, Block::Ordered(number - 1))
            }
        };
        patches.push(Patch { at: *at, remove: line.prefix, insert: prefix_of(line.indent, quoted, block) });
    }
    // Lo seleccionado sigue a su texto: lo que estaba detrás del principio
    // de la línea se corre con él, y lo que estaba dentro no se sale.
    let follow = |offset: usize| {
        let mut shift = 0isize;
        for patch in &patches {
            if offset < patch.at {
                break;
            }
            let column = offset - patch.at;
            if column >= patch.remove {
                shift += patch.insert.len() as isize - patch.remove as isize;
            } else {
                return (patch.at as isize + shift) as usize + column.min(patch.insert.len());
            }
        }
        (offset as isize + shift) as usize
    };
    edit(patched(text, &patches), follow(start), follow(end))
}

// ---- Tabla ----

/// Tres columnas, el encabezado y dos filas: la de `insertTable({ rows: 3,
/// cols: 3, withHeaderRow: true })` de la app de siempre.
const TABLE: &str = "|   |   |   |\n| --- | --- | --- |\n|   |   |   |\n|   |   |   |";

fn insert_table(text: &str, end: usize) -> ComposeEdit {
    let (from, to) = (line_start(text, end), line_end(text, end));
    let blank = |line: &str| line.trim().is_empty();
    // Una tabla va en sus líneas, con una en blanco antes y otra después:
    // pegada a un párrafo, hay quien la lee como parte de él.
    let below = if to < text.len() && !blank(&text[to + 1..line_end(text, to + 1)]) { "\n" } else { "" };
    if blank(&text[from..to]) {
        let above = if from > 0 && !blank(&text[line_start(text, from - 1)..from - 1]) { "\n" } else { "" };
        let out = format!("{}{above}{TABLE}{below}{}", &text[..from], &text[to..]);
        let cursor = from + above.len() + 2;
        return edit(out, cursor, cursor);
    }
    let out = format!("{}\n\n{TABLE}{below}{}", &text[..to], &text[to..]);
    edit(out, to + 4, to + 4)
}

// ---- Enlace ----

/// Si eso es una dirección a la que ir, y no un texto que enlazar.
fn is_url(text: &str) -> bool {
    !text.contains(char::is_whitespace) && ["http://", "https://", "mailto:"].iter().any(|scheme| text.len() > scheme.len() && text.starts_with(scheme))
}

/// La dirección del enlace `[texto](dirección)` de esa línea que contiene lo
/// seleccionado, si lo hay.
fn link_target_around(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let (from, to) = (line_start(text, start), line_end(text, start));
    let line = &text[from..to];
    line.match_indices('[').find_map(|(open, _)| {
        let middle = open + line[open..].find("](")?;
        let close = middle + line[middle..].find(')')?;
        let (link_from, link_to) = (from + open, from + close + 1);
        (start >= link_from && end <= link_to).then_some((from + middle + 2, from + close))
    })
}

fn toggle_link(text: &str, start: usize, end: usize) -> ComposeEdit {
    // Sobre un enlace que ya está, el botón es «editar el enlace»: deja
    // seleccionada su dirección para escribir otra.
    if let Some((from, to)) = link_target_around(text, start, end) {
        return edit(text.to_string(), from, to);
    }
    let chosen = &text[start..end];
    let from = start + (chosen.len() - chosen.trim_start().len());
    let label = chosen.trim();
    let to = from + label.len();
    let (before, after) = (&text[..from], &text[to..]);
    if is_url(label) {
        // Lo seleccionado ya es la dirección: falta cómo se llama.
        return edit(format!("{before}[]({label}){after}"), from + 1, from + 1);
    }
    let out = format!("{before}[{label}](){after}");
    // Con texto, el cursor va a donde falta la dirección; sin nada, a donde
    // falta el texto.
    let cursor = if label.is_empty() { from + 1 } else { to + 3 };
    edit(out, cursor, cursor)
}

/// Lo que hace ese botón de la barra con el texto y lo seleccionado en él.
///
/// - **Negrita, cursiva, código**: envuelven lo seleccionado, o lo
///   desenvuelven si ya lo estaba; sin nada seleccionado, ponen las marcas
///   con el cursor en medio.
/// - **Encabezado, listas, cita**: ponen su marca al principio de cada línea
///   seleccionada, o la quitan si todas la tenían. Una lista cambia a otra:
///   no se apilan. La cita sí se suma a lo que haya (`> - algo`).
/// - **Tabla**: una vacía en sus propias líneas, con el cursor en su primera
///   celda.
/// - **Enlace**: `[lo seleccionado]()` con el cursor donde va la dirección;
///   si lo seleccionado es una dirección, pasa a serlo y el cursor va al texto.
pub fn compose(text: &str, start: usize, end: usize, action: ComposeAction) -> ComposeEdit {
    let (start, end) = selection(text, start, end);
    match action {
        ComposeAction::Bold => toggle_inline(text, start, end, "**"),
        ComposeAction::Italic => toggle_inline(text, start, end, "*"),
        ComposeAction::Code => toggle_inline(text, start, end, "`"),
        ComposeAction::Heading | ComposeAction::BulletList | ComposeAction::OrderedList | ComposeAction::TaskList | ComposeAction::Quote => {
            toggle_lines(text, start, end, action)
        }
        ComposeAction::Table => insert_table(text, end),
        ComposeAction::Link => toggle_link(text, start, end),
    }
}

/// Lo que hace Enter dentro de una lista o una cita: la línea nueva sigue
/// siéndolo (el número siguiente, una casilla sin marcar), y en un elemento
/// vacío se sale de ella. `None` si ahí Enter es un salto de línea sin más:
/// fuera de una lista, con algo seleccionado, o dentro de un bloque de código.
pub fn continue_list(text: &str, start: usize, end: usize) -> Option<ComposeEdit> {
    let (cursor, end) = selection(text, start, end);
    if cursor != end || in_fence(text, cursor) {
        return None;
    }
    let (from, to) = (line_start(text, cursor), line_end(text, cursor));
    let line = parse_line(&text[from..to]);
    let next = match line.block {
        Block::Bullet => Block::Bullet,
        Block::Task(_) => Block::Task(false),
        Block::Ordered(number) => Block::Ordered(number + 1),
        Block::Plain | Block::Heading(_) => Block::Plain,
    };
    // Un encabezado no sigue en la línea de abajo, y con el cursor dentro de
    // la marca todavía no se está escribiendo el elemento.
    if (next == Block::Plain && !line.quoted) || cursor - from < line.prefix {
        return None;
    }
    // Enter en un elemento vacío sale de la lista: se queda la línea limpia.
    if line.content.trim().is_empty() {
        return Some(edit(format!("{}{}", &text[..from], &text[to..]), from, from));
    }
    let put = format!("\n{}", prefix_of(line.indent, line.quoted, next));
    let out = format!("{}{put}{}", &text[..cursor], &text[cursor..]);
    Some(edit(out, cursor + put.len(), cursor + put.len()))
}
