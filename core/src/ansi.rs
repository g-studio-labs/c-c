//! Los colores de terminal de un log, traducidos a tramos con estilo.
//!
//! La app de Tauri los pasaba a HTML con `ansi-to-html`; aquí el core los
//! traduce y quien pinta sólo pone cada tramo con su color. Lo usa cualquier
//! visor de logs (servicios, despliegues, backups, provisioning). Lo que no es
//! color (mover el cursor, borrar la línea, títulos de ventana) se quita: en
//! un log guardado no significa nada y pintado crudo es basura.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AnsiColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Los dieciséis de `ansi-to-html`, para que un log se vea como en la app de
/// Tauri.
const BASIC: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0xAA, 0x00, 0x00),
    (0x00, 0xAA, 0x00),
    (0xAA, 0x55, 0x00),
    (0x00, 0x00, 0xAA),
    (0xAA, 0x00, 0xAA),
    (0x00, 0xAA, 0xAA),
    (0xAA, 0xAA, 0xAA),
    (0x55, 0x55, 0x55),
    (0xFF, 0x55, 0x55),
    (0x55, 0xFF, 0x55),
    (0xFF, 0xFF, 0x55),
    (0x55, 0x55, 0xFF),
    (0xFF, 0x55, 0xFF),
    (0x55, 0xFF, 0xFF),
    (0xFF, 0xFF, 0xFF),
];

impl AnsiColor {
    /// Uno de los 256 de xterm: los dieciséis, el cubo de 6×6×6 y los grises.
    pub fn indexed(index: u8) -> AnsiColor {
        let (r, g, b) = match index {
            0..=15 => BASIC[index as usize],
            16..=231 => {
                let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
                let i = index - 16;
                (level(i / 36), level(i / 6 % 6), level(i % 6))
            }
            _ => {
                let gray = 8 + (index - 232) * 10;
                (gray, gray, gray)
            }
        };
        AnsiColor { r, g, b }
    }
}

/// Un trozo de texto con un mismo estilo.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AnsiSpan {
    pub text: String,
    pub fg: Option<AnsiColor>,
    pub bg: Option<AnsiColor>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Style {
    fg: Option<AnsiColor>,
    bg: Option<AnsiColor>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}

impl Style {
    fn apply(&mut self, params: &str) {
        let codes: Vec<u16> = params.split([';', ':']).map(|code| code.parse().unwrap_or(0)).collect();
        let mut codes = codes.into_iter();
        while let Some(code) = codes.next() {
            match code {
                0 => *self = Style::default(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                22 => (self.bold, self.dim) = (false, false),
                23 => self.italic = false,
                24 => self.underline = false,
                30..=37 => self.fg = Some(AnsiColor::indexed((code - 30) as u8)),
                90..=97 => self.fg = Some(AnsiColor::indexed((code - 90 + 8) as u8)),
                40..=47 => self.bg = Some(AnsiColor::indexed((code - 40) as u8)),
                100..=107 => self.bg = Some(AnsiColor::indexed((code - 100 + 8) as u8)),
                39 => self.fg = None,
                49 => self.bg = None,
                38 => self.fg = extended(&mut codes),
                48 => self.bg = extended(&mut codes),
                _ => {}
            }
        }
    }

    fn span(self, text: String) -> AnsiSpan {
        AnsiSpan { text, fg: self.fg, bg: self.bg, bold: self.bold, dim: self.dim, italic: self.italic, underline: self.underline }
    }
}

/// `38;5;n` o `38;2;r;g;b`. Uno a medias no pinta nada.
fn extended(codes: &mut impl Iterator<Item = u16>) -> Option<AnsiColor> {
    let byte = |code: Option<u16>| code.map(|code| code.min(255) as u8);
    match codes.next()? {
        5 => byte(codes.next()).map(AnsiColor::indexed),
        2 => Some(AnsiColor { r: byte(codes.next())?, g: byte(codes.next())?, b: byte(codes.next())? }),
        _ => None,
    }
}

/// Una línea de log en tramos. Un tabulador se lleva a su columna (de ocho en
/// ocho), porque los logs que se alinean en columnas se leen así.
pub fn ansi_spans(line: &str) -> Vec<AnsiSpan> {
    let mut runs: Vec<(Style, String)> = Vec::new();
    let mut style = Style::default();
    let mut column = 0usize;
    let mut chars = line.chars().peekable();
    let mut push = |style: Style, piece: &str| match runs.last_mut() {
        Some((last, text)) if *last == style => text.push_str(piece),
        _ => runs.push((style, piece.to_string())),
    };

    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    let mut finished = None;
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            finished = Some(c);
                            break;
                        }
                        params.push(c);
                    }
                    if finished == Some('m') {
                        style.apply(&params);
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                // Elegir juego de caracteres lleva uno más: `ESC ( B` es lo
                // que suelta `tput sgr0`, y sin esto la «B» se pintaba.
                Some('(' | ')' | '*' | '+') => {
                    chars.next();
                }
                _ => {}
            },
            '\t' => {
                let spaces = 8 - column % 8;
                push(style, &" ".repeat(spaces));
                column += spaces;
            }
            c if c.is_control() => {}
            c => {
                push(style, c.encode_utf8(&mut [0; 4]));
                column += 1;
            }
        }
    }
    runs.into_iter().map(|(style, text)| style.span(text)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(spans: &[AnsiSpan]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// Rotura que atrapa: dejar en pantalla los códigos crudos (`[32m`, un
    /// borrado de línea, el título de una ventana) o perder el texto de
    /// alrededor.
    #[test]
    fn no_raw_codes_reach_the_screen() {
        let line = "\u{1b}]0;title\u{7}\u{1b}[2K\u{1b}[1;32mINFO\u{1b}[0m listo \u{1b}[38;5;208mnaranja\u{1b}[39m\r";
        let spans = ansi_spans(line);
        assert_eq!(plain(&spans), "INFO listo naranja");
        let green = AnsiColor { r: 0, g: 0xAA, b: 0 };
        assert_eq!(spans[0], AnsiSpan { text: "INFO".into(), fg: Some(green), bold: true, ..Default::default() }, "el borrado de línea (2K) no es «atenuado» (2)");
        assert!(spans.iter().all(|span| !span.text.contains('\u{1b}') && !span.text.contains('[')));
    }

    /// Rotura que atrapa: pintar un color que no es el que pidió el log, o
    /// que un color siga después de su reinicio.
    #[test]
    fn each_span_carries_its_color() {
        let spans = ansi_spans("\u{1b}[1;31merror\u{1b}[0m: \u{1b}[92mok\u{1b}[38;2;1;2;3mrgb\u{1b}[48;5;21;39mfondo");
        let red = AnsiColor { r: 0xAA, g: 0, b: 0 };
        assert_eq!(spans[0], AnsiSpan { text: "error".into(), fg: Some(red), bold: true, ..Default::default() });
        assert_eq!(spans[1], AnsiSpan { text: ": ".into(), ..Default::default() });
        assert_eq!(spans[2].fg, Some(AnsiColor { r: 0x55, g: 0xFF, b: 0x55 }));
        assert_eq!(spans[3].fg, Some(AnsiColor { r: 1, g: 2, b: 3 }));
        assert_eq!(ansi_spans("\u{1b}[38;2;1;2mx")[0].fg, None, "un RGB a medias no se completa con ceros");
        assert_eq!((spans[4].fg, spans[4].bg), (None, Some(AnsiColor { r: 0, g: 0, b: 255 })));
        assert_eq!(AnsiColor::indexed(232), AnsiColor { r: 8, g: 8, b: 8 });
        assert_eq!(AnsiColor::indexed(16 + 36 * 5 + 6 + 2), AnsiColor { r: 255, g: 95, b: 135 });
    }

    /// Rotura que atrapa: partir en tramos lo que tiene un mismo estilo
    /// (cada tramo es un elemento que pintar) o comerse la línea con un
    /// código a medias.
    #[test]
    fn same_style_is_one_span_and_a_cut_code_drops_only_itself() {
        let spans = ansi_spans("a\u{1b}[1mb\u{1b}[0m\u{1b}[0mc\u{1b}[22md");
        assert_eq!(spans.iter().map(|span| span.text.as_str()).collect::<Vec<_>>(), ["a", "b", "cd"]);
        assert_eq!(plain(&ansi_spans("antes \u{1b}[3")), "antes ");
        assert_eq!(ansi_spans("\u{1b}[38;5m").len(), 0);
    }

    /// Rotura que atrapa: una «B» suelta tras cada `tput sgr0`.
    #[test]
    fn a_charset_switch_leaves_nothing_behind() {
        assert_eq!(plain(&ansi_spans("\u{1b}[1mok\u{1b}(B\u{1b}[m listo\u{1b})0")), "ok listo");
    }

    /// Rotura que atrapa: pintar el tabulador como un cuadro o como un
    /// espacio, que desalinea las columnas.
    #[test]
    fn a_tab_reaches_its_column() {
        assert_eq!(plain(&ansi_spans("ab\tc\td")), "ab      c       d");
    }
}
