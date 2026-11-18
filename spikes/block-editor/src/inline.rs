//! El markdown de línea de un bloque, para pintarlo: qué bytes van en
//! negrita, cuáles son un enlace, y cuáles son **marcas** (`**`, `` ` ``,
//! `](url)`) que sólo se enseñan mientras el bloque tiene el cursor.
//!
//! El texto de un bloque es markdown: aquí no se convierte a otra cosa ni se
//! vuelve a escribir. Sólo se decide cómo se ve cada byte, y —cuando las
//! marcas se esconden— a qué byte del texto corresponde cada byte pintado.

use std::ops::Range;

use markdown::mdast::Node;

/// Cómo se pinta un tramo. Se combinan (negrita dentro de un enlace).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: bool,
    pub strike: bool,
    /// Una marca de markdown, no contenido.
    pub marker: bool,
}

/// Un bloque listo para pintar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    pub text: String,
    pub runs: Vec<(Range<usize>, Look)>,
    /// Tramos `(pintado, fuente, largo)` que van a la par. Vacío cuando lo
    /// pintado es el texto tal cual.
    map: Vec<(usize, usize, usize)>,
    exact: bool,
    source_len: usize,
}

impl Shown {
    /// El texto tal cual, sin estilos: código y bloques opacos.
    pub fn plain(text: &str) -> Shown {
        Shown { text: text.to_string(), runs: Vec::new(), map: Vec::new(), exact: true, source_len: text.len() }
    }

    /// A qué byte del texto corresponde un byte de lo pintado.
    pub fn to_source(&self, shown: usize) -> usize {
        if self.exact {
            return shown.min(self.source_len);
        }
        let mut last = 0;
        for &(painted, source, len) in &self.map {
            if shown < painted {
                return source;
            }
            if shown <= painted + len {
                return source + (shown - painted);
            }
            last = source + len;
        }
        last
    }

    /// Y al revés: dónde se pinta un byte del texto.
    pub fn to_shown(&self, source_ix: usize) -> usize {
        if self.exact {
            return source_ix.min(self.text.len());
        }
        let mut last = 0;
        for &(painted, source, len) in &self.map {
            if source_ix < source {
                return painted;
            }
            if source_ix <= source + len {
                return painted + (source_ix - source);
            }
            last = painted + len;
        }
        last
    }
}

/// Cómo se pinta el texto de un bloque. Con `reveal`, las marcas se enseñan
/// (apagadas) y lo pintado es el texto byte a byte; sin él se esconden, y un
/// salto blando se pinta como el espacio que es.
pub fn shown(text: &str, reveal: bool) -> Shown {
    let mut looks = vec![Look::default(); text.len()];
    let mut soft_breaks = Vec::new();
    if let Ok(root) = markdown::to_mdast(text, &markdown::ParseOptions::gfm()) {
        walk(&root, text, &mut looks, &mut soft_breaks);
    }

    let mut out = Shown { exact: reveal, source_len: text.len(), ..Shown::default() };
    let mut run_start = 0;
    let mut segment: Option<(usize, usize, usize)> = None;
    let mut current: Option<Look> = None;
    for (ix, ch) in text.char_indices() {
        let look = looks[ix];
        if look.marker && !reveal {
            out.map.extend(segment.take());
            continue;
        }
        if current != Some(look) {
            if let Some(previous) = current.filter(|look| *look != Look::default()) {
                out.runs.push((run_start..out.text.len(), previous));
            }
            run_start = out.text.len();
            current = Some(look);
        }
        match &mut segment {
            Some((_, _, len)) => *len += ch.len_utf8(),
            None => segment = Some((out.text.len(), ix, ch.len_utf8())),
        }
        out.text.push(if !reveal && soft_breaks.contains(&ix) { ' ' } else { ch });
    }
    if let Some(previous) = current.filter(|look| *look != Look::default()) {
        out.runs.push((run_start..out.text.len(), previous));
    }
    out.map.extend(segment);
    out
}

fn span(node: &Node, text: &str) -> Option<Range<usize>> {
    let position = node.position()?;
    let range = position.start.offset..position.end.offset;
    (range.start <= range.end && range.end <= text.len() && text.is_char_boundary(range.start) && text.is_char_boundary(range.end)).then_some(range)
}

fn walk(node: &Node, text: &str, looks: &mut [Look], soft_breaks: &mut Vec<usize>) {
    let Some(range) = span(node, text) else { return };
    let mut paint = |range: Range<usize>, set: &dyn Fn(&mut Look)| looks[range].iter_mut().for_each(set);
    // Lo que queda de un nodo fuera de sus hijos son sus marcas: los `**`
    // de una negrita, el `[` y el `](url)` de un enlace.
    let markers_outside_children = |paint: &mut dyn FnMut(Range<usize>, &dyn Fn(&mut Look))| {
        let inner: Vec<Range<usize>> = node.children().into_iter().flatten().filter_map(|child| span(child, text)).collect();
        let (Some(first), Some(last)) = (inner.first(), inner.last()) else { return };
        paint(range.start..first.start.max(range.start), &|look| look.marker = true);
        paint(last.end.min(range.end)..range.end, &|look| look.marker = true);
    };
    match node {
        Node::Strong(_) => {
            paint(range.clone(), &|look| look.bold = true);
            markers_outside_children(&mut paint);
        }
        Node::Emphasis(_) => {
            paint(range.clone(), &|look| look.italic = true);
            markers_outside_children(&mut paint);
        }
        Node::Delete(_) => {
            paint(range.clone(), &|look| look.strike = true);
            markers_outside_children(&mut paint);
        }
        Node::Link(_) | Node::LinkReference(_) => {
            paint(range.clone(), &|look| look.link = true);
            markers_outside_children(&mut paint);
        }
        Node::InlineCode(_) => {
            paint(range.clone(), &|look| look.code = true);
            let slice = &text[range.clone()];
            let ticks = slice.bytes().take_while(|b| *b == b'`').count();
            if ticks * 2 < slice.len() {
                paint(range.start..range.start + ticks, &|look| look.marker = true);
                paint(range.end - ticks..range.end, &|look| look.marker = true);
            }
        }
        Node::Image(_) | Node::ImageReference(_) => {
            // `![texto](ruta)`: se enseña el texto, como un enlace.
            paint(range.clone(), &|look| look.link = true);
            let slice = &text[range.clone()];
            if let Some(close) = slice.rfind("](").or_else(|| slice.rfind("][")).filter(|close| slice.starts_with("![") && *close > 2) {
                paint(range.start..range.start + 2, &|look| look.marker = true);
                paint(range.start + close..range.end, &|look| look.marker = true);
            }
        }
        Node::FootnoteReference(_) => paint(range.clone(), &|look| look.link = true),
        Node::Html(_) => paint(range.clone(), &|look| look.code = true),
        Node::Break(_) => {
            // El salto duro: sus dos espacios o su barra son la marca.
            let slice = &text[range.clone()];
            let marker = slice.find(['\r', '\n']).unwrap_or(slice.len());
            paint(range.start..range.start + marker, &|look| look.marker = true);
        }
        Node::Text(_) => soft_breaks.extend(text[range.clone()].match_indices('\n').map(|(ix, _)| range.start + ix)),
        _ => {}
    }
    for child in node.children().into_iter().flatten() {
        walk(child, text, looks, soft_breaks);
    }
}
