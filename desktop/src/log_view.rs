//! Una caja de log: las líneas que tradujo el core, cada tramo con su color.
//! La usan los servicios, y la usarán despliegues, backups y provisioning.
//!
//! Baja sola con cada línea nueva, como `tail -f`, sólo mientras se está
//! abajo: quien sube a leer una línea de antes no pierde la vista con la
//! siguiente que llega (`use-follow-log.ts`).

use cac_core::{AnsiColor, LogLine};
use gpui_kit::*;

use crate::theme::Leading as _;

/// Lo que se considera «abajo»: un par de líneas de holgura.
const NEAR_BOTTOM: f32 = 32.;

/// Siempre oscura, como una terminal, y con el verde de la app de Tauri:
/// los colores de un log están pensados para ese fondo.
const BACKGROUND: u32 = 0x27272a;
const DEFAULT_TEXT: u32 = 0x4ade80;

fn color(color: AnsiColor) -> Hsla {
    rgb(((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32).into()
}

fn line_text(line: &LogLine) -> (SharedString, Vec<(std::ops::Range<usize>, HighlightStyle)>) {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for span in &line.spans {
        let start = text.len();
        text.push_str(&span.text);
        let style = HighlightStyle {
            color: span.fg.map(color),
            background_color: span.bg.map(color),
            font_weight: span.bold.then_some(FontWeight::BOLD),
            font_style: span.italic.then_some(FontStyle::Italic),
            underline: span.underline.then(|| UnderlineStyle { thickness: px(1.), ..Default::default() }),
            fade_out: span.dim.then_some(0.4),
            ..Default::default()
        };
        if style != HighlightStyle::default() {
            highlights.push((start..text.len(), style));
        }
    }
    (text.into(), highlights)
}

/// El texto de una línea, sin estilo: lo que se lee en voz alta y lo que
/// miran las pruebas.
pub(crate) fn plain(line: &LogLine) -> String {
    line.spans.iter().map(|span| span.text.as_str()).collect()
}

pub(crate) struct LogBox {
    scroll: ScrollHandle,
    /// Cuántas líneas había en el último pintado: sólo se baja cuando llegan
    /// más. Pedirlo en cada pintado provoca otro pintado, sin fin.
    shown: std::cell::Cell<usize>,
}

impl LogBox {
    pub(crate) fn new() -> Self {
        LogBox { scroll: ScrollHandle::new(), shown: std::cell::Cell::new(0) }
    }

    /// `id` nombra la caja; cada línea es `{id}-line-{n}`.
    pub(crate) fn render(&self, id: &str, lines: &[LogLine], empty: String, mono: SharedString) -> AnyElement {
        let (offset, max) = (self.scroll.offset(), self.scroll.max_offset());
        if self.shown.replace(lines.len()) != lines.len() && -offset.y >= max.y - px(NEAR_BOTTOM) {
            self.scroll.scroll_to_bottom();
        }
        let rows: Vec<AnyElement> = if lines.is_empty() {
            vec![div().text_color(rgb(0xa1a1aa)).child(empty).into_any_element()]
        } else {
            lines
                .iter()
                .enumerate()
                .map(|(n, line)| {
                    let (text, highlights) = line_text(line);
                    div()
                        .id(SharedString::from(format!("{id}-line-{n}")))
                        .aria_label(plain(line))
                        .whitespace_nowrap()
                        .child(StyledText::new(text).with_highlights(highlights))
                        .test_support()
                        .into_any_element()
                })
                .collect()
        };
        div()
            .id(SharedString::from(id.to_string()))
            .track_scroll(&self.scroll)
            .overflow_scroll()
            .min_h(px(128.))
            .max_h(px(400.))
            .p_3()
            .rounded(px(6.))
            .bg(rgb(BACKGROUND))
            .text_color(rgb(DEFAULT_TEXT))
            .font_family(mono)
            .leading_sm()
            .children(rows)
            .test_support()
            .into_any_element()
    }
}
