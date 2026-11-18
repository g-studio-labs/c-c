//! La propiedad por la que se rechazó el componente del 17-nov: lo que no se
//! toca no cambia. Sin ventana: es todo del modelo (`doc.rs`).
//!
//! Cada prueba dice qué rotura atrapa, y cada una se comprobó rompiendo el
//! código a propósito (el README dice cuáles).

use block_editor_spike::doc::{Doc, Kind};

const CORPUS: [(&str, &str); 6] = [
    ("runbook", include_str!("../corpus/runbook.md")),
    ("notes", include_str!("../corpus/notes.md")),
    ("crlf", include_str!("../corpus/crlf.md")),
    ("edges", include_str!("../corpus/edges.md")),
    ("empty", include_str!("../corpus/empty.md")),
    ("blank", include_str!("../corpus/blank.md")),
];

/// Qué bloques hay y qué dicen, sin los blancos de los bordes (al reabrir,
/// los del final de un bloque pasan a ser separación).
fn kinds(doc: &Doc) -> Vec<(Kind, String)> {
    doc.blocks.iter().map(|block| (block.kind, block.text.trim().to_string())).collect()
}

/// El bloque cuyo texto empieza así.
fn find(doc: &Doc, start: &str) -> usize {
    doc.blocks.iter().position(|block| block.text.starts_with(start)).unwrap_or_else(|| panic!("no hay un bloque que empiece por {start:?}"))
}

/// Abrir y guardar sin tocar nada devuelve el fichero byte a byte.
///
/// Roturas que atrapa: normalizar algo al leer o al escribir (un salto duro,
/// los blancos de una separación, el fin de línea de Windows, el final sin
/// salto); perder lo que hay antes del primer bloque o después del último.
#[test]
fn opening_and_saving_returns_the_same_bytes() {
    for (name, source) in CORPUS {
        assert_eq!(Doc::parse(source).to_markdown(), source, "{name}: leer y guardar cambió el texto");
    }
}

/// Lo mismo con trozos arbitrarios del corpus: cortado por cualquier línea,
/// con las líneas barajadas, o cortado en mitad de una. Markdown roto
/// también es de alguien.
///
/// Rotura que atrapa: un caso que el corpus entero no pisa (una valla que no
/// cierra, una lista cortada a la mitad) y que el modelo no devuelve igual.
#[test]
fn any_piece_of_the_corpus_comes_back_the_same() {
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut next = move |below: usize| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % below.max(1)
    };
    let mut tried = 0;
    for (name, source) in CORPUS {
        let lines: Vec<&str> = source.split_inclusive('\n').collect();
        for _ in 0..400 {
            if lines.is_empty() {
                break;
            }
            let piece: String = match next(3) {
                0 => {
                    let from = next(lines.len());
                    lines[from..from + 1 + next(lines.len() - from)].concat()
                }
                1 => (0..1 + next(12)).map(|_| lines[next(lines.len())]).collect(),
                _ => {
                    let mut cut = next(source.len() + 1);
                    while !source.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    source[..cut].to_string()
                }
            };
            assert_eq!(Doc::parse(&piece).to_markdown(), piece, "{name}: este trozo no volvió igual:\n{piece:?}");
            tried += 1;
        }
    }
    assert!(tried > 1500, "sólo se probaron {tried} trozos");
}

/// Lo que el prototipo no edita por bloques se queda opaco, entero y tal
/// cual; y lo que sí edita, se entiende.
///
/// Roturas que atrapa: que una tabla, un HTML, una cita, una lista anidada o
/// con casillas, una nota al pie o un título con cierre se «entiendan» como
/// párrafo o lista y se puedan partir por dentro; y al revés, que todo se
/// quede opaco y no haya nada que editar pintado.
#[test]
fn what_is_not_understood_stays_opaque_and_whole() {
    let runbook = Doc::parse(CORPUS[0].1);
    let notes = Doc::parse(CORPUS[1].1);
    let opaque = [
        (&runbook, "| Variable"),
        (&runbook, "> **Ojo:**"),
        (&runbook, "<details>"),
        (&runbook, "<pre>"),
        (&runbook, "- [x] Backend"),
        (&runbook, "[^egress]:"),
        (&runbook, "[viejo]:"),
        (&runbook, "* * *"),
        (&notes, "Notas de la reunión\n====="),
        (&notes, "## Título con cierre ##"),
        (&notes, "* Fruta\n  * Manzana"),
        (&notes, "+ Un elemento\n\n+ Otro"),
        (&notes, "- Elemento de varias\n  líneas"),
        (&notes, "<!-- un comentario"),
        (&notes, "| a | b |"),
    ];
    for (doc, start) in opaque {
        let block = &doc.blocks[find(doc, start)];
        assert_eq!(block.kind, Kind::Opaque, "se entendió como otra cosa: {start:?}");
        assert!(block.lead.is_empty() && block.tail.is_empty(), "un bloque opaco no se parte: {start:?}");
    }
    // La lista de casillas es un solo bloque con sus tres líneas.
    assert_eq!(runbook.blocks[find(&runbook, "- [x] Backend")].text.lines().count(), 3);

    let understood = [
        (&runbook, "Runbook de despliegue", Kind::Heading(1), "# "),
        (&runbook, "Antes de empezar", Kind::Heading(2), "## "),
        (&runbook, "Variables", Kind::Heading(3), "### "),
        (&runbook, "El backend despliega", Kind::Paragraph, ""),
        (&runbook, "Comprueba que", Kind::Numbered, "1. "),
        (&runbook, "Avisa en el canal", Kind::Numbered, "3. "),
        (&runbook, "Re-cortar", Kind::Bullet, "- "),
        (&runbook, "(cd backend", Kind::Code, "```sh\n"),
        (&notes, "El once", Kind::Numbered, "11. "),
        (&notes, "También", Kind::Numbered, "2) "),
        (&notes, "def hola", Kind::Code, "~~~python\n"),
        (&notes, "```rust\nfn main", Kind::Code, "````md\n"),
    ];
    for (doc, start, kind, lead) in understood {
        let block = &doc.blocks[find(doc, start)];
        assert_eq!((block.kind, block.lead.as_str()), (kind, lead), "{start:?}");
    }
    // El párrafo guarda sus saltos como venían: el duro de dos espacios, el
    // de barra y el blando.
    let breaks = &runbook.blocks[find(&runbook, "Una línea con salto duro")];
    assert!(breaks.text.contains("espacios)  \ny otra") && breaks.text.contains("invertida\\\ny la") && breaks.text.contains("abajo\nsin más"));
}

/// Escribir en un bloque cambia ese bloque y nada más: el resto del fichero
/// queda byte a byte, antes y después.
///
/// Roturas que atrapa: volver a escribir el documento entero al guardar (y
/// con él las separaciones, las viñetas o los números de los demás); que el
/// texto escrito caiga en otro sitio.
#[test]
fn typing_in_one_block_changes_only_that_block() {
    for (name, source) in CORPUS {
        let pristine = Doc::parse(source);
        for ix in 0..pristine.blocks.len() {
            let mut doc = pristine.clone();
            let block = &doc.blocks[ix];
            let before: usize = doc.head.len() + doc.blocks[..ix].iter().map(|b| b.source().len() + b.gap.len()).sum::<usize>() + block.lead.len() + block.text.len();
            doc.blocks[ix].text.push_str("ñ¡YA!");
            let expected = format!("{}ñ¡YA!{}", &source[..before], &source[before..]);
            assert_eq!(doc.to_markdown(), expected, "{name}, bloque {ix}");
        }
    }
}

/// Partir, unir y convertir sólo tocan el bloque y sus dos costuras: todo lo
/// que hay antes del bloque de arriba y después del de abajo queda igual.
///
/// Roturas que atrapa: renumerar la lista entera al insertar; «arreglar»
/// separaciones de bloques que no son vecinos; perder texto al unir.
#[test]
fn structural_edits_leave_the_rest_of_the_file_alone() {
    let source = CORPUS[0].1;
    let pristine = Doc::parse(source);
    for ix in 0..pristine.blocks.len() {
        let start: usize = pristine.head.len() + pristine.blocks[..ix.saturating_sub(1)].iter().map(|b| b.source().len() + b.gap.len()).sum::<usize>();
        let untouched_before = &source[..start];
        let after_ix = (ix + 2).min(pristine.blocks.len());
        let tail_len: usize = pristine.blocks[after_ix..].iter().map(|b| b.source().len() + b.gap.len()).sum();
        let untouched_after = &source[source.len() - tail_len..];

        let middle = {
            let text = &pristine.blocks[ix].text;
            let mut at = text.len() / 2;
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            at
        };
        let mut split = pristine.clone();
        split.split(ix, middle);
        let mut joined = pristine.clone();
        joined.join_backwards(ix);
        let mut converted = pristine.clone();
        converted.convert(ix, Kind::Heading(2));
        for (what, doc) in [("partir", split), ("unir", joined), ("convertir", converted)] {
            let out = doc.to_markdown();
            assert!(out.starts_with(untouched_before), "{what} el bloque {ix} cambió algo de más arriba");
            assert!(out.ends_with(untouched_after), "{what} el bloque {ix} cambió algo de más abajo:\n{out}");
        }
    }
}

/// Lo que se ve tras editar es lo que se lee al volver a abrir: el markdown
/// que sale se vuelve a partir en los mismos bloques.
///
/// Roturas que atrapa: dos párrafos guardados sin línea en blanco entre
/// ellos (al reabrir son uno); un párrafo pegado debajo de una lista (se lo
/// come el último elemento); un elemento nuevo con un número que no sigue.
#[test]
fn what_is_on_screen_after_editing_is_what_reopens() {
    let mut doc = Doc::parse("# Título\n\nUno dos\n\n- a\n- b\n\n1. x\n2. y\n");
    let reopened = |doc: &Doc| kinds(&Doc::parse(&doc.to_markdown()));

    // Partir un párrafo: dos párrafos, con su línea en blanco.
    assert_eq!(doc.split(1, 3), (2, 0));
    assert_eq!(doc.to_markdown(), "# Título\n\nUno\n\n dos\n\n- a\n- b\n\n1. x\n2. y\n");
    // Enter al final de un elemento: otro elemento, pegado al de arriba.
    let b = find(&doc, "b");
    assert_eq!(doc.split(b, 1), (b + 1, 0));
    doc.blocks[b + 1].text.push('c');
    assert_eq!(doc.to_markdown(), "# Título\n\nUno\n\n dos\n\n- a\n- b\n- c\n\n1. x\n2. y\n");
    // En una lista numerada el nuevo sigue la cuenta, y los demás no se tocan.
    let x = find(&doc, "x");
    doc.split(x, 1);
    doc.blocks[x + 1].text.push_str("nuevo");
    assert_eq!(doc.to_markdown(), "# Título\n\nUno\n\n dos\n\n- a\n- b\n- c\n\n1. x\n2. nuevo\n2. y\n");
    assert_eq!(reopened(&doc)[x + 1], (Kind::Numbered, "nuevo".to_string()));
    // Un elemento del medio que deja de ser lista queda separado por arriba
    // y por abajo: si no, al reabrir se lo come el elemento de arriba.
    let b = find(&doc, "b");
    doc.join_backwards(b);
    assert_eq!(doc.blocks[b].kind, Kind::Paragraph);
    assert_eq!(doc.to_markdown(), "# Título\n\nUno\n\n dos\n\n- a\n\nb\n\n- c\n\n1. x\n2. nuevo\n2. y\n");
    assert_eq!(reopened(&doc), kinds(&doc));
    // Y unir dos párrafos junta su texto sin perder ni añadir nada.
    let dos = find(&doc, " dos");
    assert_eq!(doc.join_backwards(dos), Some((dos - 1, 3)));
    assert!(doc.to_markdown().starts_with("# Título\n\nUno dos\n\n- a\n"));
}

/// Las marcas que convierten un párrafo, y sólo con el cursor justo detrás.
///
/// Roturas que atrapa: que `#texto` (sin espacio) sea un título; que siete
/// almohadillas lo sean; que escribir en medio de un párrafo que ya empezaba
/// por `- ` lo convierta; que la marca se quede en el texto; que una lista
/// numerada pierda el número que se escribió.
#[test]
fn a_marker_at_the_start_converts_the_paragraph() {
    for (typed, kind, lead) in [
        ("# ", Kind::Heading(1), "# "),
        ("## ", Kind::Heading(2), "## "),
        ("### ", Kind::Heading(3), "### "),
        ("- ", Kind::Bullet, "- "),
        ("* ", Kind::Bullet, "* "),
        ("1. ", Kind::Numbered, "1. "),
        ("7) ", Kind::Numbered, "7) "),
        ("```", Kind::Code, "```\n"),
    ] {
        let mut doc = Doc::parse("antes\n\nresto\n");
        doc.blocks[1].text.insert_str(0, typed);
        assert_eq!(doc.apply_shortcut(1, typed.len()), Some((1, 0)), "{typed:?}");
        assert_eq!((doc.blocks[1].kind, doc.blocks[1].lead.as_str(), doc.blocks[1].text.as_str()), (kind, lead, "resto"), "{typed:?}");
        // Y al reabrir es lo mismo que se ve.
        let reopened = Doc::parse(&doc.to_markdown());
        assert_eq!((reopened.blocks[1].kind, reopened.blocks[1].text.as_str()), (kind, "resto"), "{typed:?} no se reabre igual: {:?}", doc.to_markdown());
    }
    for typed in ["#", "#x ", "####### ", "-", "1.", "a. ", "``"] {
        let mut doc = Doc::parse("resto\n");
        doc.blocks[0].text.insert_str(0, typed);
        assert_eq!(doc.apply_shortcut(0, typed.len()), None, "{typed:?} no es una marca");
    }
    // Con el cursor en otro sitio no pasa nada: el párrafo ya era así.
    let mut doc = Doc::parse("antes\n\n\\- resto\n");
    doc.blocks[1].text = "- resto".to_string();
    assert_eq!(doc.apply_shortcut(1, 5), None);
    // Y un título no se convierte en lista por empezar por `- `.
    let mut doc = Doc::parse("# - resto\n");
    assert_eq!(doc.apply_shortcut(0, 2), None);
}

/// Enter y retroceso en los bordes.
///
/// Roturas que atrapa: Enter en un bloque de código que lo parte en dos (y
/// rompe la valla); un elemento vacío que no sale de la lista; retroceso al
/// principio de un párrafo que mezcla su texto dentro de una tabla o de un
/// bloque de código; retroceso en el primer bloque que borra algo.
#[test]
fn enter_and_backspace_at_the_edges() {
    let mut doc = Doc::parse("```sh\nuno\n```\n\n- a\n- \n\n| t |\n|---|\n\npárrafo\n");
    assert_eq!(doc.split(0, 3), (0, 4));
    assert_eq!(doc.blocks[0].text, "uno\n");
    assert_eq!(doc.blocks[0].kind, Kind::Code);

    let table = find(&doc, "| t |");
    let paragraph = find(&doc, "párrafo");
    assert_eq!(doc.join_backwards(paragraph), Some((table, doc.blocks[table].text.len())));
    assert_eq!(doc.blocks[table].text, "| t |\n|---|", "el párrafo se metió en la tabla");
    assert_eq!(doc.blocks[paragraph].text, "párrafo");

    let mut doc = Doc::parse("- a\n- b\n");
    doc.split(1, 1);
    assert_eq!(doc.blocks.len(), 3);
    assert_eq!(doc.split(2, 0), (2, 0), "un elemento vacío sale de la lista");
    assert_eq!(doc.blocks[2].kind, Kind::Paragraph);
    assert_eq!(doc.blocks.len(), 3);

    let mut doc = Doc::parse("primero\n\nsegundo\n");
    assert_eq!(doc.join_backwards(0), None);
    assert_eq!(doc.to_markdown(), "primero\n\nsegundo\n");
}
