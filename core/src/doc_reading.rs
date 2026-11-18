//! Lo que se lee de un documento, trozo a trozo, y su índice.
//!
//! El texto se parte dos veces. Primero en prosa y en lo ancho (`doc_blocks`:
//! una tabla o un bloque de código usan el ancho del panel). Y la prosa, en
//! cada título: así cada sección empieza un trozo, y con eso
//!
//! - el índice «En esta página» salta a su sección y sabe por cuál se va;
//! - un enlace a una sección (`&section=`, `#ancla`) llega a ella;
//! - y la pantalla sólo maqueta los trozos que caben: un documento de 3.000
//!   líneas pintado de una pieza eran 115 ms por cuadro (medido el
//!   18-nov-2026).
//!
//! Los títulos se cuentan como los cuenta el servidor (`scanMarkdown` y
//! `markdownHeading` en `domain/search_section.go`): de uno a seis `#`
//! pegados al margen y un espacio, nunca dentro de un bloque de código. De
//! eso depende que el título de puesto `i` sea la sección de puesto `i`.

use unicode_normalization::UnicodeNormalization;

use crate::doc_links::{heading_slug, DocJump};
use crate::docs::{doc_blocks, DocSection, DocsState};
use crate::task::DecisionRef;

/// Con qué acaba un trozo: de eso depende el aire que lleva debajo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DocPieceEnd {
    /// Un párrafo, una lista, una cita: texto corriente.
    Text,
    /// El propio título: el trozo no tiene nada más.
    Heading,
    /// Una raya de separación (`---`).
    Rule,
}

/// Un trozo de lo que se lee.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocPiece {
    /// Una tabla, un bloque de código o una imagen sola: a todo el ancho.
    pub wide: bool,
    /// Markdown.
    pub text: String,
    /// El nivel del título con que empieza (1–6), si empieza por uno.
    pub heading: Option<u8>,
    pub end: DocPieceEnd,
    /// Su puesto en el índice «En esta página», si sale en él.
    pub entry: Option<u32>,
    /// El ancla de su título (`que-es-esto`).
    pub slug: Option<String>,
    /// El id que el servidor le da a su sección, si la reconoce.
    pub section_id: Option<String>,
    /// Las decisiones vigentes que sostienen esa sección.
    pub decisions: Vec<DecisionRef>,
}

/// Una entrada del índice «En esta página».
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocHeading {
    /// 2 o 3: el índice no baja más.
    pub level: u8,
    pub text: String,
    /// En qué trozo de lo que se lee empieza.
    pub piece: u32,
}

/// El título de una línea, como lo lee el servidor: su nivel y lo que dice
/// tras los `#`, tal cual.
fn heading_line(line: &str) -> Option<(u8, &str)> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    let rest = &line[level..];
    ((1..=6).contains(&level) && rest.starts_with(' ')).then_some((level as u8, rest))
}

/// Cómo se comparan dos títulos: sin mayúsculas, sin acentos y sin marcas
/// de markdown, como `domain.SectionKey`. El servidor guarda `**Correo**`
/// y el texto dice «Correo».
fn heading_key(text: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    for c in text.to_lowercase().nfd() {
        if unicode_normalization::char::is_combining_mark(c) {
            continue;
        }
        if c.is_alphanumeric() {
            word.push(c);
        } else if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words.join(" ")
}

/// Si esa línea es una raya de separación: tres o más `-`, `*` o `_`.
fn is_rule(line: &str) -> bool {
    let marks: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && ['-', '*', '_'].contains(&marks[0]) && marks.iter().all(|c| *c == marks[0])
}

/// Parte lo que se lee en trozos: lo ancho aparte, y la prosa en cada
/// título. `sections` son las del servidor para ese texto, si las mandó.
pub fn doc_pieces(markdown: &str, sections: &[DocSection]) -> Vec<DocPiece> {
    let mut pieces: Vec<DocPiece> = Vec::new();
    // Cuántos títulos van, de cualquier nivel (su puesto, para el
    // servidor), y cuántos de los que salen en el índice.
    let (mut seen, mut listed) = (0u32, 0u32);

    for block in doc_blocks(markdown) {
        if block.wide {
            pieces.push(DocPiece { wide: true, text: block.text, heading: None, end: DocPieceEnd::Text, entry: None, slug: None, section_id: None, decisions: Vec::new() });
            continue;
        }
        // La valla abierta dentro de la prosa (el código de un paso de una
        // lista): lo de dentro no son títulos.
        let mut fence = "";
        let mut lines: Vec<&str> = Vec::new();
        let mut start = 0;
        let all: Vec<&str> = block.text.lines().collect();
        let mut cuts: Vec<usize> = Vec::new();
        for (at, line) in all.iter().enumerate() {
            let trimmed = line.trim_start_matches([' ', '\t']);
            let marker = ["```", "~~~"].into_iter().find(|marker| trimmed.starts_with(marker)).unwrap_or("");
            if fence.is_empty() && !marker.is_empty() {
                fence = marker;
            } else if !fence.is_empty() && marker == fence {
                fence = "";
            }
            if fence.is_empty() && marker.is_empty() && heading_line(line).is_some() {
                cuts.push(at);
            }
        }
        cuts.push(all.len());
        for cut in cuts {
            lines.clear();
            lines.extend_from_slice(&all[start..cut]);
            start = cut;
            // Las líneas en blanco de las puntas son el hueco entre trozos.
            while lines.first().is_some_and(|line| line.trim().is_empty()) {
                lines.remove(0);
            }
            while lines.last().is_some_and(|line| line.trim().is_empty()) {
                lines.pop();
            }
            let Some(first) = lines.first() else { continue };
            let mut piece = DocPiece {
                wide: false,
                text: lines.join("\n"),
                heading: None,
                end: DocPieceEnd::Text,
                entry: None,
                slug: None,
                section_id: None,
                decisions: Vec::new(),
            };
            if let Some((level, rest)) = heading_line(first) {
                let index = seen;
                seen += 1;
                piece.heading = Some(level);
                // Lo que se lee del título: sin los signos de énfasis.
                let shown = rest.replace(['*', '_', '`'], "").trim().to_string();
                if (2..=3).contains(&level) && !shown.is_empty() {
                    piece.entry = Some(listed);
                    listed += 1;
                }
                piece.slug = Some(heading_slug(&shown)).filter(|slug| !slug.is_empty());
                // La sección de ese puesto, si su título casa: una que va
                // atrasada se queda sin ella antes que con la de otro.
                if let Some(section) = sections.iter().find(|section| section.index == index) {
                    if section.level == level && heading_key(&section.heading) == heading_key(rest) {
                        piece.section_id = Some(section.id.clone());
                        piece.decisions = section.decisions.clone();
                    }
                }
                if lines.len() == 1 {
                    piece.end = DocPieceEnd::Heading;
                }
            }
            if piece.end == DocPieceEnd::Text && lines.len() > 1 {
                let last = lines[lines.len() - 1];
                // Bajo una línea de texto, `---` la hace título; no es raya.
                if is_rule(last) && lines[lines.len() - 2].trim().is_empty() {
                    piece.end = DocPieceEnd::Rule;
                }
            } else if piece.end == DocPieceEnd::Text && is_rule(first) {
                piece.end = DocPieceEnd::Rule;
            }
            pieces.push(piece);
        }
    }
    pieces
}

/// Las secciones de un markdown: sus títulos de segundo y tercer nivel, sin
/// los signos de énfasis, y en qué trozo empieza cada una. Vino de
/// `headingsOf` en `app/src/lib/headings.ts`. Un `# comentario` dentro de un
/// bloque de código no es una sección.
pub fn doc_headings(markdown: &str) -> Vec<DocHeading> {
    headings_of(&doc_pieces(markdown, &[]))
}

fn headings_of(pieces: &[DocPiece]) -> Vec<DocHeading> {
    pieces
        .iter()
        .enumerate()
        .filter(|(_, piece)| piece.entry.is_some())
        .filter_map(|(at, piece)| {
            let first = piece.text.lines().next()?;
            let (level, rest) = heading_line(first)?;
            Some(DocHeading { level, text: rest.replace(['*', '_', '`'], "").trim().to_string(), piece: at as u32 })
        })
        .collect()
}

/// El enlace al registro donde vive una decisión —la pestaña de decisiones
/// de su nodo—, que es donde está su porqué (`SectionDecisionsMark.tsx`).
/// `None` si el servidor no dijo de qué nodo es.
pub fn decision_record_link(decision: &DecisionRef) -> Option<String> {
    let kind = decision.owner_kind.as_deref().filter(|kind| !kind.is_empty())?;
    let id = decision.owner_id.as_deref().filter(|id| !id.is_empty())?;
    Some(format!("/tasks?doc={kind}:{id}&tab=decisions"))
}

/// Dónde está el título de una entrada del índice respecto a lo que se ve.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DocEntryPlace {
    /// Ya quedó arriba, fuera de la vista.
    Above,
    /// A la vista, a tantos píxeles del borde de arriba.
    At { top: f32 },
    /// Todavía no se ha llegado a él.
    Below,
}

/// Hasta qué parte del alto de la vista cuenta un título como «ya se está
/// en su sección»: el quinto de arriba. Es la franja que mira el índice de
/// la app de siempre (`rootMargin: "-10% 0px -80% 0px"` en `DocToc.tsx`).
const ACTIVE_BAND: f32 = 0.2;

/// Qué entrada del índice se marca: la de la sección por la que va la
/// lectura, que es la del último título que ya llegó al quinto de arriba de
/// la vista. Ninguna si todavía no ha llegado ninguno.
pub fn doc_active_entry(places: &[DocEntryPlace], view_height: f32) -> Option<u32> {
    places
        .iter()
        .rposition(|place| match place {
            DocEntryPlace::Above => true,
            DocEntryPlace::At { top } => *top <= view_height * ACTIVE_BAND,
            DocEntryPlace::Below => false,
        })
        .map(|at| at as u32)
}

/// Las pestañas que llevan índice: en una lista de entradas repetiría lo
/// que se ve (`WITH_TOC` en `DocTabs.tsx`).
const TABS_WITH_CONTENTS: [&str; 2] = ["overview", "runbook"];

impl DocsState {
    /// Las secciones que el servidor reconoce en lo que se lee.
    fn sections(&self) -> &[DocSection] {
        if self.on_page() {
            return self.page.as_ref().map(|view| view.page.sections.as_slice()).unwrap_or(&[]);
        }
        self.tabs.iter().find(|tab| tab.key == self.tab).map(|tab| tab.sections.as_slice()).unwrap_or(&[])
    }

    /// Lo que se lee, en trozos.
    pub fn pieces(&self) -> Vec<DocPiece> {
        doc_pieces(self.body(), self.sections())
    }

    /// El índice de lo que se lee: sólo donde aporta (una página siempre;
    /// de la portada, dos pestañas), y sólo si hay al menos dos secciones
    /// entre las que saltar.
    pub fn contents(&self) -> Vec<DocHeading> {
        if !self.on_page() && !TABS_WITH_CONTENTS.contains(&self.tab.as_str()) {
            return Vec::new();
        }
        let headings = headings_of(&self.pieces());
        if headings.len() < 2 {
            return Vec::new();
        }
        headings
    }

    /// El trozo al que hay que saltar, si un enlace pidió ir a una sección
    /// y ya está aquí. `None` también si lo que se lee no la tiene: ese
    /// enlace llega a su pestaña y se queda arriba. De dos títulos iguales,
    /// el ancla es del primero, como en una página web.
    pub fn jump_piece(&self) -> Option<u32> {
        let pieces = self.pieces();
        let at = match self.jump.as_ref()? {
            DocJump::Section { id } => pieces.iter().position(|piece| piece.section_id.as_deref() == Some(id.as_str())),
            DocJump::Anchor { slug } => {
                let wanted = heading_slug(slug);
                pieces.iter().position(|piece| piece.slug.as_deref() == Some(wanted.as_str()))
            }
        };
        at.map(|at| at as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::{DocTab, DocsState};

    fn shape(markdown: &str) -> Vec<(bool, &'static str, String)> {
        doc_pieces(markdown, &[])
            .into_iter()
            .map(|piece| {
                let end = match piece.end {
                    DocPieceEnd::Text => "texto",
                    DocPieceEnd::Heading => "título",
                    DocPieceEnd::Rule => "raya",
                };
                (piece.wide, end, piece.text)
            })
            .collect()
    }

    /// Cada título empieza un trozo, lo ancho va aparte, y no se pierde ni
    /// se desordena nada.
    ///
    /// Roturas que atrapa: una sección sin trozo propio (el índice no puede
    /// saltar a ella); un `# comentario` de un bloque de código —suelto o
    /// dentro de un paso de una lista— tomado por título y partiendo el
    /// bloque; perder el texto que va antes del primer título; un `#sin
    /// espacio` o un título sangrado partiendo el texto.
    #[test]
    fn every_heading_starts_a_piece() {
        let text = "Antes.\n\n# Doc\n\n## Uno\n\nTexto uno.\n\n| a |\n| - |\n\n### Dos\n## Tres\n\n1. Paso:\n   ```sh\n# no es título\n   ```\n\n#no tampoco\n  ## ni éste\n\n```\n## ni éste\n```";
        assert_eq!(
            shape(text),
            [
                (false, "texto", "Antes.".to_string()),
                (false, "título", "# Doc".to_string()),
                (false, "texto", "## Uno\n\nTexto uno.".to_string()),
                (true, "texto", "| a |\n| - |".to_string()),
                (false, "título", "### Dos".to_string()),
                (false, "texto", "## Tres\n\n1. Paso:\n   ```sh\n# no es título\n   ```\n\n#no tampoco\n  ## ni éste".to_string()),
                (true, "texto", "```\n## ni éste\n```".to_string()),
            ]
        );
        assert_eq!(shape(""), []);
    }

    /// Un trozo dice si acaba en una raya de separación: debajo lleva otro
    /// aire que un párrafo. Un `---` pegado bajo una línea de texto no es
    /// una raya, es un título subrayado.
    ///
    /// Rotura que atrapa: el título que sigue a una raya pegado a ella, o
    /// separado como de un párrafo.
    #[test]
    fn a_piece_says_whether_it_ends_in_a_rule() {
        assert_eq!(shape("Texto.\n\n---\n\n## Dos")[0].1, "raya");
        assert_eq!(shape("* * *\n\n## Dos")[0].1, "raya");
        assert_eq!(shape("Título\n---\n\n## Dos")[0].1, "texto");
        assert_eq!(shape("## Uno\n\n___\n\n## Dos")[0].1, "raya");
        assert_eq!(shape("a -- b\n\n## Dos")[0].1, "texto");
    }

    fn section(id: &str, heading: &str, level: u8, index: u32) -> DocSection {
        DocSection { id: id.into(), heading: heading.into(), level, index, decisions: Vec::new() }
    }

    /// El título de puesto `i` es la sección de puesto `i` del servidor, si
    /// su título y su nivel casan; el puesto cuenta todos los títulos, no
    /// sólo los del índice, y no los de dentro de un bloque de código.
    ///
    /// Roturas que atrapa: contar sólo los títulos de segundo y tercer
    /// nivel (toda sección tras un `#` se corre una); darle a un título la
    /// sección de otro cuando el servidor va atrasado; no reconocer
    /// `**Correo**` como «Correo», ni «Configuración» escrita sin acento;
    /// perder las decisiones de una sección.
    #[test]
    fn a_heading_takes_the_section_of_its_place_if_it_matches() {
        let decision = DecisionRef { id: "d1".into(), title: "Sin colas".into(), ..DecisionRef::default() };
        let sections = [
            section("s0", "Doc", 1, 0),
            DocSection { decisions: vec![decision.clone()], ..section("s1", "**Correo** `smtp`", 2, 1) },
            section("s2", "Otra cosa", 2, 2),
            section("s3", "Configuracion", 3, 3),
            section("s4", "Final", 3, 4),
        ];
        let text = "# Doc\n\n## Correo smtp ##\n\n```\n## no cuenta\n```\n\n## Renombrada\n\n### Configuración\n\n## Final";
        let read: Vec<(Option<String>, usize)> =
            doc_pieces(text, &sections).into_iter().filter(|piece| piece.heading.is_some()).map(|piece| (piece.section_id, piece.decisions.len())).collect();
        assert_eq!(
            read,
            [(Some("s0".to_string()), 0), (Some("s1".to_string()), 1), (None, 0), (Some("s3".to_string()), 0), (None, 0)],
            "la última casa en título pero no en nivel"
        );
    }

    /// El índice son los títulos de segundo y tercer nivel, limpios, con el
    /// trozo donde empieza cada uno, y sólo donde hay entre qué saltar.
    ///
    /// Roturas que atrapa: el título del documento o uno de cuarto nivel en
    /// el índice; un `## comentario` de un bloque de código tomado por
    /// sección; los asteriscos del énfasis a la vista; una entrada que
    /// salta al trozo de otra; un índice de una sola entrada, o uno en la
    /// pestaña de enlaces.
    #[test]
    fn the_contents_are_the_second_and_third_level_headings() {
        let text = "# Doc\n\n## Qué **es**\n\n```sh\n## no es sección\n```\n\n### Cómo `corre`\n\n#### Detalle\n\n##sin espacio\n\n## \n";
        let said: Vec<(u8, String, u32)> = doc_headings(text).into_iter().map(|heading| (heading.level, heading.text, heading.piece)).collect();
        assert_eq!(said, [(2, "Qué es".to_string(), 1), (3, "Cómo corre".to_string(), 3)]);
        let pieces = doc_pieces(text, &[]);
        assert!(pieces[1].text.starts_with("## Qué") && pieces[3].text.starts_with("### Cómo"));

        let with = |tab: &str, body: &str| DocsState {
            tab: tab.into(),
            tabs: vec![DocTab { key: tab.into(), body: body.into(), ..DocTab::default() }],
            ..DocsState::default()
        };
        assert_eq!(with("overview", text).contents().len(), 2);
        assert_eq!(with("runbook", text).contents().len(), 2);
        assert!(with("links", text).contents().is_empty());
        assert!(with("overview", "## Una sola\n\ntexto").contents().is_empty());
    }

    /// Se marca la sección por la que va la lectura: la del último título
    /// que ya llegó al quinto de arriba de la vista.
    ///
    /// Roturas que atrapa: marcar la primera que asoma por abajo (se lee
    /// una sección y el índice dice la siguiente); no marcar ninguna cuando
    /// el título ya se fue por arriba; marcar la primera antes de llegar a
    /// ella; que pulsar una entrada —que deja su título arriba del todo— no
    /// la marque.
    #[test]
    fn the_active_entry_is_the_last_heading_that_reached_the_top() {
        use DocEntryPlace::{Above, At, Below};
        assert_eq!(doc_active_entry(&[], 800.), None);
        assert_eq!(doc_active_entry(&[At { top: 300. }, Below], 800.), None);
        assert_eq!(doc_active_entry(&[At { top: 160. }, At { top: 500. }, Below], 800.), Some(0));
        assert_eq!(doc_active_entry(&[Above, At { top: 161. }, Below], 800.), Some(0));
        assert_eq!(doc_active_entry(&[Above, At { top: 0. }, At { top: 700. }], 800.), Some(1));
        assert_eq!(doc_active_entry(&[Above, Above, Above], 800.), Some(2));
    }

    /// Una decisión que sostiene una sección lleva a su registro: las
    /// decisiones del nodo donde se tomó.
    ///
    /// Roturas que atrapa: abrir el registro del nodo que se lee cuando la
    /// decisión es de otro; un enlace a medias cuando no se sabe de cuál es.
    #[test]
    fn a_decision_leads_to_the_record_where_it_lives() {
        use crate::doc_links::{doc_link, DocLink};
        use crate::nav::NodeKind;
        let decision = DecisionRef { id: "d1".into(), owner_kind: Some("folder".into()), owner_id: Some("f1".into()), ..DecisionRef::default() };
        let link = decision_record_link(&decision).unwrap();
        assert_eq!(doc_link(&link), DocLink::Doc { kind: NodeKind::Folder, id: "f1".into(), tab: Some("decisions".into()), page: None, section: None });
        assert_eq!(decision_record_link(&DecisionRef { owner_id: None, ..decision.clone() }), None);
        assert_eq!(decision_record_link(&DecisionRef { owner_kind: Some(String::new()), ..decision }), None);
    }

    /// Un enlace a una sección llega a su trozo: por el id del servidor, o
    /// por el ancla del título (escrita con o sin acentos). Dos títulos
    /// iguales: el ancla es del primero. Lo que no está no lleva a nada.
    ///
    /// Roturas que atrapa: un enlace a una sección que se queda arriba;
    /// `#configuración` sin encontrar «Configuración»; saltar al segundo de
    /// dos títulos iguales; un salto a un trozo cualquiera cuando la
    /// sección ya no existe.
    #[test]
    fn a_jump_finds_its_piece() {
        let text = "Intro.\n\n## Configuración\n\nuno\n\n## Otra\n\n## Configuración\n\ndos";
        let tab = DocTab { key: "overview".into(), body: text.into(), sections: vec![section("s1", "Otra", 2, 1)], ..DocTab::default() };
        let with = |jump: Option<DocJump>| DocsState { tabs: vec![tab.clone()], jump, ..DocsState::default() };
        assert_eq!(with(None).jump_piece(), None);
        assert_eq!(with(Some(DocJump::Section { id: "s1".into() })).jump_piece(), Some(2));
        assert_eq!(with(Some(DocJump::Section { id: "s9".into() })).jump_piece(), None);
        assert_eq!(with(Some(DocJump::Anchor { slug: "configuracion".into() })).jump_piece(), Some(1));
        assert_eq!(with(Some(DocJump::Anchor { slug: "Configuración".into() })).jump_piece(), Some(1));
        assert_eq!(with(Some(DocJump::Anchor { slug: "no-esta".into() })).jump_piece(), None);

        // En una página, las secciones son las de la página, no las de la
        // pestaña que quedó debajo.
        use crate::doc_pages::{DocPage, DocPageView};
        let page = DocPage { id: "p1".into(), body: "## Capas\n\ntres\n\n## Otra".into(), sections: vec![section("s1", "Capas", 2, 0)], ..DocPage::default() };
        let on_page = DocsState {
            page_id: Some("p1".into()),
            page: Some(DocPageView { page, ..DocPageView::default() }),
            ..with(Some(DocJump::Section { id: "s1".into() }))
        };
        assert_eq!(on_page.jump_piece(), Some(0));
    }
}
