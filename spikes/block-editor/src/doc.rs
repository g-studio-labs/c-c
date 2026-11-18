//! El documento: markdown partido en bloques **sin perder un byte**.
//!
//! La idea que sostiene todo lo demás: un bloque no guarda «lo que significa»
//! sino los trozos de texto del fichero que lo forman —lo que va delante
//! (`lead`), lo que se edita (`text`), lo que va detrás (`tail`) y la
//! separación hasta el siguiente (`gap`)—, y guardar es **pegarlos en orden**.
//! No hay un «escritor de markdown» que pueda reescribir lo que no se tocó:
//! lo que no se tocó vuelve a salir porque nunca dejó de ser el mismo texto.
//!
//! Y lo que no se sabe editar no se interpreta: al leer, cada bloque se parte
//! y se vuelve a pegar; si no sale idéntico al original, o es una
//! construcción que aquí no se edita (tabla, HTML, cita, lista anidada…), se
//! queda **opaco**: su texto entero, tal cual. Es justo lo que el componente
//! rechazado el 17-nov-2026 no hacía (reescribía saltos y juntaba líneas).
//!
//! Aquí no hay GPUI: todo esto se prueba sin ventana.

use markdown::mdast::Node;

/// Qué es un bloque para quien lo edita.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    /// Un título `#`…`######`, con su nivel.
    Heading(u8),
    Bullet,
    Numbered,
    /// Un bloque de código con valla (```` ``` ````).
    Code,
    /// Markdown que aquí no se edita por bloques: se guarda y se enseña tal
    /// cual, y se puede retocar como texto.
    Opaque,
}

impl Kind {
    /// Los que llevan markdown de línea dentro (negrita, enlaces…).
    pub fn has_inline(self) -> bool {
        !matches!(self, Kind::Code | Kind::Opaque)
    }

    /// Los que guardan sus saltos de línea como texto: Enter no los parte.
    pub fn is_raw(self) -> bool {
        matches!(self, Kind::Code | Kind::Opaque)
    }

    fn is_list(self) -> bool {
        matches!(self, Kind::Bullet | Kind::Numbered)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// Estable mientras el documento está abierto: sobrevive a partir y unir.
    pub id: u64,
    pub kind: Kind,
    /// Lo que va delante del texto, tal como venía: `## `, `- `, `3. `, o la
    /// línea que abre el código con su salto.
    pub lead: String,
    pub text: String,
    /// Lo que va detrás: el cierre del código.
    pub tail: String,
    /// La separación hasta el bloque siguiente: saltos y blancos, tal cual.
    pub gap: String,
}

impl Block {
    /// El bloque como va en el fichero, sin su separación.
    pub fn source(&self) -> String {
        format!("{}{}{}", self.lead, self.text, self.tail)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doc {
    /// Lo que hubiera antes del primer bloque (líneas en blanco).
    pub head: String,
    pub blocks: Vec<Block>,
    next_id: u64,
}

/// Dónde queda el cursor tras una operación: bloque y byte dentro de su texto.
pub type Place = (usize, usize);

impl Doc {
    /// Parte el markdown en bloques. Nunca falla y nunca pierde nada:
    /// `Doc::parse(s).to_markdown() == s` para cualquier `s`.
    pub fn parse(source: &str) -> Doc {
        let mut doc = Doc::default();
        let Ok(Node::Root(root)) = markdown::to_mdast(source, &markdown::ParseOptions::gfm()) else {
            // El parser sólo falla con MDX, que no se le pide. Si un día
            // fallara, el documento entero es un bloque opaco: no se pierde.
            doc.push_span(source, 0, source.len(), None);
            return doc;
        };
        let mut at = 0;
        for node in &root.children {
            let Some(position) = node.position() else { continue };
            let (start, end) = (position.start.offset, position.end.offset);
            if start < at || end > source.len() || !source.is_char_boundary(start) || !source.is_char_boundary(end) {
                continue;
            }
            // Algo entre dos nodos que no es blanco y que el parser no dio
            // como nodo: no se sabe qué es, así que se guarda opaco.
            doc.absorb(source, at, start);
            doc.push_span(source, start, end, Some(node));
            at = end;
        }
        doc.absorb(source, at, source.len());
        doc
    }

    /// Lo que hay entre dos nodos: blancos (van a la separación del bloque de
    /// antes) y, si hubiera algo más, un bloque opaco con ello.
    fn absorb(&mut self, source: &str, from: usize, to: usize) {
        let between = &source[from..to];
        if between.trim().is_empty() {
            self.trailing(between);
            return;
        }
        let blank = between.len() - between.trim_start().len();
        self.trailing(&between[..blank]);
        self.push_span(source, from + blank, to, None);
    }

    fn trailing(&mut self, blank: &str) {
        match self.blocks.last_mut() {
            Some(last) => last.gap.push_str(blank),
            None => self.head.push_str(blank),
        }
    }

    /// Mete `source[start..end]` como uno o más bloques. Los blancos del
    /// final van a la separación.
    fn push_span(&mut self, source: &str, start: usize, end: usize, node: Option<&Node>) {
        let span = &source[start..end];
        let raw = span.trim_end();
        let gap = &span[raw.len()..];
        let parts = node.and_then(|node| understand(raw, node)).unwrap_or_else(|| vec![(Kind::Opaque, String::new(), raw.to_string(), String::new())]);
        let last = parts.len() - 1;
        for (ix, (kind, lead, text, tail)) in parts.into_iter().enumerate() {
            // Entre los elementos de una lista sencilla hay un salto y nada más.
            let gap = if ix == last { gap.to_string() } else { "\n".to_string() };
            let id = self.fresh_id();
            self.blocks.push(Block { id, kind, lead, text, tail, gap });
        }
    }

    fn fresh_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// El documento de vuelta a markdown: los trozos, pegados en orden.
    pub fn to_markdown(&self) -> String {
        let mut out = String::with_capacity(self.head.len() + self.blocks.iter().map(|b| b.lead.len() + b.text.len() + b.tail.len() + b.gap.len()).sum::<usize>());
        out.push_str(&self.head);
        for block in &self.blocks {
            out.push_str(&block.lead);
            out.push_str(&block.text);
            out.push_str(&block.tail);
            out.push_str(&block.gap);
        }
        out
    }

    // ---- Editar ----------------------------------------------------------

    /// Un documento vacío no tiene dónde poner el cursor: se le da un párrafo.
    pub fn ensure_not_empty(&mut self) {
        if self.blocks.is_empty() {
            let id = self.fresh_id();
            self.blocks.push(Block { id, kind: Kind::Paragraph, lead: String::new(), text: String::new(), tail: String::new(), gap: String::new() });
        }
    }

    /// Enter: parte el bloque en `at` y dice dónde queda el cursor.
    ///
    /// Un título se parte en título y párrafo; un elemento de lista en dos
    /// elementos; un elemento de lista vacío deja de ser lista (así se sale
    /// de ella, como en Notion). En código y en opaco Enter es un salto de
    /// línea dentro del bloque.
    pub fn split(&mut self, ix: usize, at: usize) -> Place {
        let block = &self.blocks[ix];
        let kind = block.kind;
        if kind.is_raw() {
            self.blocks[ix].text.insert(at, '\n');
            return (ix, at + 1);
        }
        if kind.is_list() && block.text.is_empty() {
            self.convert(ix, Kind::Paragraph);
            return (ix, 0);
        }
        // Enter al principio de un título con texto: el hueco se abre encima
        // y el título sigue siendo el título.
        if matches!(kind, Kind::Heading(_)) && at == 0 && !block.text.is_empty() {
            self.insert(ix, Kind::Paragraph, String::new());
            return (ix + 1, 0);
        }
        let rest = self.blocks[ix].text.split_off(at);
        let new_kind = if kind.is_list() { kind } else { Kind::Paragraph };
        self.insert(ix + 1, new_kind, rest);
        (ix + 1, 0)
    }

    /// Un bloque nuevo en la posición `ix`, con la forma normal de su tipo.
    pub fn insert(&mut self, ix: usize, kind: Kind, text: String) {
        let (lead, tail) = self.canonical(ix, kind);
        let id = self.fresh_id();
        // La separación que había antes del que ahora queda detrás se la
        // lleva el nuevo; `mend` arregla las dos costuras.
        let gap = if ix > 0 { std::mem::take(&mut self.blocks[ix - 1].gap) } else { String::new() };
        self.blocks.insert(ix, Block { id, kind, lead, text, tail, gap });
        self.mend(ix);
    }

    /// Lo que lleva delante y detrás un bloque de ese tipo recién hecho en
    /// esa posición: copia la viñeta del vecino y sigue su numeración.
    fn canonical(&self, ix: usize, kind: Kind) -> (String, String) {
        let before = ix.checked_sub(1).and_then(|ix| self.blocks.get(ix));
        match kind {
            Kind::Paragraph | Kind::Opaque => (String::new(), String::new()),
            Kind::Heading(level) => (format!("{} ", "#".repeat(level.clamp(1, 6) as usize)), String::new()),
            Kind::Bullet => (before.filter(|b| b.kind == Kind::Bullet).map(|b| b.lead.clone()).unwrap_or_else(|| "- ".to_string()), String::new()),
            Kind::Numbered => {
                let previous = before.filter(|b| b.kind == Kind::Numbered).and_then(|b| number_of(&b.lead));
                match previous {
                    Some((number, delimiter)) => (format!("{}{delimiter} ", number + 1), String::new()),
                    None => ("1. ".to_string(), String::new()),
                }
            }
            Kind::Code => ("```\n".to_string(), "\n```".to_string()),
        }
    }

    /// Cambia el tipo de un bloque conservando su texto.
    pub fn convert(&mut self, ix: usize, kind: Kind) {
        let (lead, tail) = self.canonical(ix, kind);
        let block = &mut self.blocks[ix];
        if block.kind == Kind::Code && kind != Kind::Code {
            // El código pierde su valla: el texto se queda.
        }
        block.kind = kind;
        block.lead = lead;
        block.tail = tail;
        self.mend(ix);
    }

    /// Lo escrito al principio de un párrafo que lo convierte en otra cosa:
    /// `# `, `## `…, `- `, `* `, `1. `, y la valla de código. Devuelve el tipo
    /// y cuántos bytes del principio son la marca.
    pub fn shortcut(text: &str) -> Option<(Kind, usize)> {
        if text.starts_with("```") {
            return Some((Kind::Code, 3));
        }
        let (marker, _) = text.split_once(' ')?;
        let len = marker.len() + 1;
        if !marker.is_empty() && marker.len() <= 6 && marker.bytes().all(|b| b == b'#') {
            return Some((Kind::Heading(marker.len() as u8), len));
        }
        if matches!(marker, "-" | "*" | "+") {
            return Some((Kind::Bullet, len));
        }
        let digits = marker.strip_suffix('.').or_else(|| marker.strip_suffix(')'))?;
        (!digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit())).then_some((Kind::Numbered, len))
    }

    /// Si el párrafo `ix` empieza por una marca y el cursor está justo
    /// detrás, lo convierte y devuelve dónde queda el cursor.
    pub fn apply_shortcut(&mut self, ix: usize, cursor: usize) -> Option<Place> {
        let block = &self.blocks[ix];
        if block.kind != Kind::Paragraph {
            return None;
        }
        let (kind, len) = Doc::shortcut(&block.text)?;
        if cursor != len {
            return None;
        }
        let marker = block.text[..len].to_string();
        self.blocks[ix].text.drain(..len);
        self.convert(ix, kind);
        // La viñeta y el número, como se escribieron.
        if kind.is_list() {
            self.blocks[ix].lead = marker;
        }
        Some((ix, 0))
    }

    /// Retroceso al principio de un bloque.
    ///
    /// Si no es un párrafo, primero deja de ser lo que era (un título o un
    /// elemento de lista se quedan en párrafo). Si lo es, se une al de
    /// arriba y el cursor queda en la costura. Con algo opaco o código
    /// arriba no se mezcla el texto: el cursor sube, y un párrafo vacío se
    /// borra.
    pub fn join_backwards(&mut self, ix: usize) -> Option<Place> {
        let kind = self.blocks[ix].kind;
        if kind != Kind::Paragraph && kind != Kind::Opaque {
            self.convert(ix, Kind::Paragraph);
            return Some((ix, 0));
        }
        if ix == 0 {
            return None;
        }
        let above = ix - 1;
        let seam = self.blocks[above].text.len();
        let mixes = self.blocks[above].kind.has_inline() && kind == Kind::Paragraph;
        if !mixes {
            if self.blocks[ix].text.is_empty() {
                self.remove(ix);
            }
            return Some((above, seam));
        }
        let gone = self.blocks.remove(ix);
        let above_block = &mut self.blocks[above];
        above_block.text.push_str(&gone.text);
        above_block.gap = gone.gap;
        self.mend(above);
        Some((above, seam))
    }

    /// Quita un bloque: su separación la hereda el de antes.
    pub fn remove(&mut self, ix: usize) {
        let gone = self.blocks.remove(ix);
        if ix > 0 {
            self.blocks[ix - 1].gap = gone.gap;
            self.mend(ix - 1);
        }
        self.ensure_not_empty();
    }

    /// Deja bien las dos costuras del bloque `ix`: una línea en blanco entre
    /// bloques, un solo salto entre elementos de la misma lista. Sólo toca
    /// separaciones que son blancos y se quedaron cortas; las demás, y todas
    /// las que no rodean a `ix`, ni las mira.
    fn mend(&mut self, ix: usize) {
        for first in [ix.wrapping_sub(1), ix] {
            let (Some(a), Some(b)) = (self.blocks.get(first), self.blocks.get(first.wrapping_add(1))) else { continue };
            let same_list = a.kind.is_list() && a.kind == b.kind;
            let needed = if same_list { 1 } else { 2 };
            if a.gap.bytes().filter(|b| *b == b'\n').count() < needed {
                self.blocks[first].gap = "\n".repeat(needed);
            }
        }
    }
}

/// El número y el delimitador (`.` o `)`) de la marca de una lista numerada.
fn number_of(lead: &str) -> Option<(u64, char)> {
    let marker = lead.trim_end();
    let delimiter = marker.chars().last().filter(|c| matches!(c, '.' | ')'))?;
    Some((marker[..marker.len() - 1].parse().ok()?, delimiter))
}

type Parts = Vec<(Kind, String, String, String)>;

/// Parte un nodo en bloques editables, o dice que no sabe (`None`, opaco).
///
/// La regla que guarda la fidelidad: lo que salga de aquí, pegado, tiene que
/// ser **idéntico** a `raw`. Se comprueba al final; si no lo es, opaco.
fn understand(raw: &str, node: &Node) -> Option<Parts> {
    let parts: Parts = match node {
        Node::Paragraph(_) => vec![(Kind::Paragraph, String::new(), raw.to_string(), String::new())],
        Node::Heading(heading) => {
            let hashes = raw.bytes().take_while(|b| *b == b'#').count();
            let text = raw.get(hashes..)?.strip_prefix(' ')?;
            // Un título de una línea, `# ` y el texto: sin cierre de
            // almohadillas, sin espacios de más, sin subrayado (setext).
            if hashes != heading.depth as usize || raw.contains('\n') || text.is_empty() || text.starts_with(char::is_whitespace) || text.ends_with('#') {
                return None;
            }
            vec![(Kind::Heading(heading.depth), raw[..hashes + 1].to_string(), text.to_string(), String::new())]
        }
        Node::Code(_) => {
            let (opening, rest) = raw.split_once('\n')?;
            let fence = opening.bytes().next().filter(|b| matches!(b, b'`' | b'~'))?;
            let width = opening.bytes().take_while(|b| *b == fence).count();
            let (body, closing) = rest.rsplit_once('\n')?;
            if width < 3 || closing.len() < width || !closing.bytes().all(|b| b == fence) {
                return None;
            }
            vec![(Kind::Code, format!("{opening}\n"), body.to_string(), format!("\n{closing}"))]
        }
        Node::List(list) => {
            // Sólo la lista sencilla: un elemento por línea, sin anidar, sin
            // casillas, sin líneas en blanco entre medias.
            let simple = |item: &Node| matches!(item, Node::ListItem(item) if item.checked.is_none() && item.children.len() == 1 && matches!(item.children[0], Node::Paragraph(_)));
            if list.spread || raw.lines().count() != list.children.len() || !list.children.iter().all(simple) {
                return None;
            }
            let kind = if list.ordered { Kind::Numbered } else { Kind::Bullet };
            let mut items = Parts::new();
            for line in raw.split('\n') {
                let (marker, text) = line.split_once(' ')?;
                let fits = match Doc::shortcut(line) {
                    Some((found, len)) => found == kind && len == marker.len() + 1,
                    None => false,
                };
                if !fits || text.is_empty() || text.starts_with(char::is_whitespace) {
                    return None;
                }
                items.push((kind, format!("{marker} "), text.to_string(), String::new()));
            }
            items
        }
        _ => return None,
    };
    // Una red, no una regla probada: con lo que hoy se entiende, cada parte
    // es un trozo de `raw` y esto no puede fallar (quitarlo no rompe ninguna
    // prueba). Está para quien añada aquí una construcción y se equivoque.
    let rebuilt = parts.iter().map(|(_, lead, text, tail)| format!("{lead}{text}{tail}")).collect::<Vec<_>>().join("\n");
    (rebuilt == raw).then_some(parts)
}
