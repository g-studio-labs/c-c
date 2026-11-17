//! Los botones de la barra del editor: qué le hace cada uno al markdown.
//!
//! Los textos de estas pruebas llevan dibujado lo seleccionado: `«así»`, o
//! `¦` donde está el cursor. Las marcas no son parte del texto; se quitan
//! antes de llamar y se vuelven a poner en lo que sale, así que cada prueba
//! comprueba a la vez el texto y dónde queda la selección.

use cac_core::{compose, continue_list, ComposeAction, ComposeEdit};

/// El texto sin las marcas, y la selección que dibujaban (en bytes).
fn read(marked: &str) -> (String, usize, usize) {
    if let Some(cursor) = marked.find('¦') {
        return (marked.replace('¦', ""), cursor, cursor);
    }
    let start = marked.find('«').expect("falta « o ¦");
    let plain = marked.replace('«', "");
    let end = plain.find('»').expect("falta »");
    (plain.replace('»', ""), start, end)
}

/// Lo que salió, con su selección dibujada.
fn drawn(edit: &ComposeEdit) -> String {
    let (start, end) = (edit.start as usize, edit.end as usize);
    if start == end {
        format!("{}¦{}", &edit.text[..start], &edit.text[start..])
    } else {
        format!("{}«{}»{}", &edit.text[..start], &edit.text[start..end], &edit.text[end..])
    }
}

fn press(marked: &str, action: ComposeAction) -> String {
    let (text, start, end) = read(marked);
    drawn(&compose(&text, start, end, action))
}

fn enter(marked: &str) -> Option<String> {
    let (text, start, end) = read(marked);
    continue_list(&text, start, end).as_ref().map(drawn)
}

use ComposeAction::*;

/// Negrita, cursiva y código envuelven lo seleccionado y lo dejan
/// seleccionado, sin el espacio de los lados.
///
/// Roturas que atrapa: un botón que no pone nada, o pone la marca de otro;
/// envolver el espacio (`** hola**` no es negrita); perder la selección, o
/// dejarla sobre las marcas, con lo que el segundo clic no desharía el
/// primero.
#[test]
fn a_mark_wraps_what_is_selected() {
    assert_eq!(press("di «hola» ya", Bold), "di **«hola»** ya");
    assert_eq!(press("di «hola» ya", Italic), "di *«hola»* ya");
    assert_eq!(press("di «hola» ya", Code), "di `«hola»` ya");
    assert_eq!(press("di« hola »ya", Bold), "di **«hola»** ya");
}

/// El mismo botón otra vez quita la marca: se seleccionara lo de dentro o
/// con las marcas.
///
/// Roturas que atrapa: un botón que sólo sabe poner (`****hola****`);
/// desenvolver sólo si se seleccionó de una de las dos maneras; quitar las
/// marcas y dejar la selección corrida.
#[test]
fn the_same_button_takes_the_mark_off() {
    assert_eq!(press("di **«hola»** ya", Bold), "di «hola» ya");
    assert_eq!(press("di «**hola**» ya", Bold), "di «hola» ya");
    assert_eq!(press("di `«hola»` ya", Code), "di «hola» ya");
    assert_eq!(press("di «*hola*» ya", Italic), "di «hola» ya");
}

/// Negrita y cursiva comparten el asterisco y no se confunden: la cursiva
/// sobre una negrita se suma, y cada una se quita sin llevarse a la otra.
///
/// Roturas que atrapa: tomar los `**` de una negrita por una cursiva puesta
/// (y quitarle un asterisco a cada lado, que la rompe); no poder quitar la
/// cursiva de algo que es las dos cosas.
#[test]
fn bold_and_italic_do_not_take_each_other_for_themselves() {
    assert_eq!(press("**«hola»**", Italic), "***«hola»***");
    assert_eq!(press("***«hola»***", Italic), "**«hola»**");
    assert_eq!(press("***«hola»***", Bold), "*«hola»*");
    assert_eq!(press("*«hola»*", Bold), "***«hola»***");
    // Unas marcas solas no envuelven nada: no hay qué quitarles.
    assert_eq!(press("di «**» ya", Bold), "di **«**»** ya");
}

/// Sin nada seleccionado, el botón pone las marcas con el cursor en medio;
/// pulsarlo otra vez ahí las quita.
///
/// Roturas que atrapa: un botón que sin selección no hace nada; el cursor
/// detrás de las marcas, donde lo que se escribe ya no sale en negrita;
/// acumular marcas vacías a cada clic.
#[test]
fn with_nothing_selected_the_marks_go_around_the_cursor() {
    assert_eq!(press("hola ¦", Bold), "hola **¦**");
    assert_eq!(press("hola **¦**", Bold), "hola ¦");
    assert_eq!(press("¦", Code), "`¦`");
    assert_eq!(press("`¦`", Code), "¦");
    assert_eq!(press("hola **¦**", Italic), "hola ***¦***");
}

/// Varias líneas seleccionadas llevan cada una sus marcas, y la de una lista
/// va detrás de su guion.
///
/// Roturas que atrapa: una negrita abierta en un párrafo y cerrada en otro,
/// que no es negrita en ninguno; `**- uno**`, que deja de ser una lista;
/// marcas en una línea en blanco; quitar sólo cuando lo están todas.
#[test]
fn each_selected_line_gets_its_own_marks() {
    assert_eq!(press("«- uno\n\n- dos»", Bold), "- **«uno**\n\n- **dos»**");
    assert_eq!(press("- **«uno**\n\n- **dos»**", Bold), "- «uno\n\n- dos»");
    assert_eq!(press("**«uno**\ndos»", Bold), "**«uno**\n**dos»**");
}

/// Las posiciones son bytes, y los acentos y los emoji ocupan varios: el
/// texto sale entero y la selección sobre las mismas letras.
///
/// Roturas que atrapa: contar letras donde hay que contar bytes (la
/// selección se corre una letra por cada acento que haya antes); cortar una
/// letra por la mitad, que es un pánico.
#[test]
fn accents_and_emoji_keep_their_place() {
    assert_eq!(press("ñandú «añejo» 🎉 fin", Bold), "ñandú **«añejo»** 🎉 fin");
    assert_eq!(press("ñandú **«añejo»** 🎉 fin", Bold), "ñandú «añejo» 🎉 fin");
    assert_eq!(press("🎉 «día»", Link), "🎉 [día](¦)");
    assert_eq!(press("«🎉 más»\nñu", BulletList), "- «🎉 más»\nñu");
    assert_eq!(press("- ñu 🎉¦", OrderedList), "1. ñu 🎉¦");
    // Una posición en mitad de una letra (la «ñ» son dos bytes) se lleva a
    // su principio, y una de más allá del final, al final.
    let cut = compose("ñu", 1, 1, Bold);
    assert_eq!((cut.text.as_str(), cut.start, cut.end), ("****ñu", 2, 2));
    let past = compose("ñu", 9, 40, Code);
    assert_eq!((past.text.as_str(), past.start, past.end), ("ñu``", 4, 4));
    // Y al revés (se seleccionó de derecha a izquierda) es lo mismo.
    assert_eq!(compose("di hola", 7, 3, Bold), compose("di hola", 3, 7, Bold));
}

/// El encabezado, las listas y la cita ponen su marca al principio de la
/// línea del cursor, y el cursor sigue a su texto.
///
/// Roturas que atrapa: un botón que pone la marca de otro; ponerla donde
/// está el cursor y no al principio de la línea; dejar el cursor delante de
/// la marca en una línea vacía, con lo que se escribe antes del guion.
#[test]
fn a_block_button_marks_the_line_of_the_cursor() {
    assert_eq!(press("uno\ndos ¦tres\ncuatro", Heading), "uno\n## dos ¦tres\ncuatro");
    assert_eq!(press("dos ¦tres", BulletList), "- dos ¦tres");
    assert_eq!(press("dos ¦tres", OrderedList), "1. dos ¦tres");
    assert_eq!(press("dos ¦tres", TaskList), "- [ ] dos ¦tres");
    assert_eq!(press("dos ¦tres", Quote), "> dos ¦tres");
    assert_eq!(press("¦", TaskList), "- [ ] ¦");
    assert_eq!(press("uno\n¦", BulletList), "uno\n- ¦");
}

/// El mismo botón otra vez la quita.
///
/// Roturas que atrapa: `- - algo` al segundo clic; quitar la marca y dejar
/// el cursor corrido, o fuera de la línea si estaba dentro de la marca.
#[test]
fn the_same_block_button_takes_its_mark_off() {
    assert_eq!(press("## dos ¦tres", Heading), "dos ¦tres");
    assert_eq!(press("- dos ¦tres", BulletList), "dos ¦tres");
    assert_eq!(press("7. dos ¦tres", OrderedList), "dos ¦tres");
    assert_eq!(press("- [x] dos ¦tres", TaskList), "dos ¦tres");
    assert_eq!(press("> dos ¦tres", Quote), "dos ¦tres");
    assert_eq!(press("- [¦ ] dos", TaskList), "¦dos");
}

/// Una lista se cambia por otra en vez de apilarse, y un encabezado de otro
/// nivel pasa al de la barra. La cita sí se suma a lo que haya.
///
/// Roturas que atrapa: `1. - algo`, que es una lista dentro de otra; perder
/// la casilla marcada al volver a pulsar; `## ### título`; una cita que se
/// lleva la lista, o una lista que se lleva la cita.
#[test]
fn one_kind_of_block_replaces_another() {
    assert_eq!(press("- dos¦", OrderedList), "1. dos¦");
    assert_eq!(press("1. dos¦", TaskList), "- [ ] dos¦");
    assert_eq!(press("- [ ] dos¦", BulletList), "- dos¦");
    assert_eq!(press("* dos¦", Heading), "## dos¦");
    assert_eq!(press("### dos¦", Heading), "## dos¦");
    assert_eq!(press("- dos¦", Quote), "> - dos¦");
    assert_eq!(press("> - dos¦", OrderedList), "> 1. dos¦");
    assert_eq!(press("> - dos¦", Quote), "- dos¦");
    assert_eq!(press("  - dos¦", TaskList), "  - [ ] dos¦");
    // `1)` también numera, y una almohadilla pegada al texto no es encabezado.
    assert_eq!(press("1) dos¦", BulletList), "- dos¦");
    assert_eq!(press("#uno¦", Heading), "## #uno¦");
    assert_eq!(press("> 1. dos¦", BulletList), "> - dos¦");
    assert_eq!(press("«- [x] uno\ndos»", TaskList), "«- [x] uno\n- [ ] dos»");
}

/// Con varias líneas seleccionadas van todas, numeradas en orden; las que
/// están en blanco no, y la que sólo se roza al final tampoco.
///
/// Roturas que atrapa: marcar sólo la primera línea; `1.` en todas; un guion
/// en cada línea en blanco de en medio; marcar la línea siguiente por
/// haber seleccionado hasta su principio; quitar la marca cuando sólo la
/// tenían algunas (en vez de ponérsela a las demás).
#[test]
fn every_selected_line_is_marked() {
    assert_eq!(press("«uno\ndos\n\ntres»\ncuatro", OrderedList), "1. «uno\n2. dos\n\n3. tres»\ncuatro");
    assert_eq!(press("«uno\ndos\n»tres", BulletList), "- «uno\n- dos\n»tres");
    assert_eq!(press("- «uno\ndos»", BulletList), "- «uno\n- dos»");
    assert_eq!(press("- «uno\n- dos»", BulletList), "«uno\ndos»");
    assert_eq!(press("u«no\ndo»s", Quote), "> u«no\n> do»s");
}

/// Una lista numerada sigue la cuenta de la línea de encima.
///
/// Roturas que atrapa: volver a empezar por el 1 debajo de una lista; seguir
/// la cuenta de una lista que no es la suya (otra sangría).
#[test]
fn a_numbered_list_carries_on_from_the_line_above() {
    assert_eq!(press("1. uno\n2. dos\ntres¦", OrderedList), "1. uno\n2. dos\n3. tres¦");
    assert_eq!(press("  4. uno\ndos¦", OrderedList), "  4. uno\n1. dos¦");
    assert_eq!(press("uno\ndos¦", OrderedList), "uno\n1. dos¦");
}

/// La tabla va en sus propias líneas, separada de lo que haya, con el
/// cursor en su primera celda.
///
/// Roturas que atrapa: una tabla pegada al párrafo, que se lee como texto;
/// partir la línea por donde estaba el cursor; el cursor al final de la
/// tabla y no donde se empieza a rellenar; líneas en blanco de más cuando
/// ya las había.
#[test]
fn a_table_goes_on_its_own_lines() {
    let table = "|   |   |   |\n| --- | --- | --- |\n|   |   |   |\n|   |   |   |";
    let ready = table.replacen("| ", "| ¦", 1);
    assert_eq!(press("¦", Table), ready);
    assert_eq!(press("uno d¦os\ntres", Table), format!("uno dos\n\n{ready}\n\ntres"));
    assert_eq!(press("uno\n¦\ntres", Table), format!("uno\n\n{ready}\n\ntres"));
    assert_eq!(press("uno\n\n¦\n\ntres", Table), format!("uno\n\n{ready}\n\ntres"));
    assert_eq!(press("uno¦", Table), format!("uno\n\n{ready}"));
}

/// El enlace envuelve lo seleccionado y deja el cursor donde va la
/// dirección; si lo seleccionado es una dirección, pasa a serlo y el cursor
/// va a donde falta el texto.
///
/// Roturas que atrapa: el cursor al final, con la dirección por escribir
/// atrás; una dirección metida como texto del enlace (`[https://…]()`), que
/// no lleva a ningún sitio; tomar por dirección una frase que empieza por
/// «https»; un botón que sin selección no hace nada.
#[test]
fn a_link_wraps_what_is_selected() {
    assert_eq!(press("ver «la guía» hoy", Link), "ver [la guía](¦) hoy");
    assert_eq!(press("ver «https://cac.dev/a?b=1» hoy", Link), "ver [¦](https://cac.dev/a?b=1) hoy");
    assert_eq!(press("«mailto:ana@correo.com»", Link), "[¦](mailto:ana@correo.com)");
    assert_eq!(press("«https://cac.dev y más»", Link), "[https://cac.dev y más](¦)");
    assert_eq!(press("«https://»", Link), "[https://](¦)");
    assert_eq!(press("ver ¦", Link), "ver [¦]()");
}

/// Sobre un enlace que ya está, el botón deja seleccionada su dirección.
///
/// Roturas que atrapa: un enlace dentro de otro (`[[texto]()](dirección)`)
/// al pulsar el botón sobre uno hecho; seleccionar la dirección de otro
/// enlace de la línea.
#[test]
fn on_a_link_the_button_selects_its_target() {
    assert_eq!(press("ver [la g¦uía](https://a.dev) y [otra](https://b.dev)", Link), "ver [la guía](«https://a.dev») y [otra](https://b.dev)");
    assert_eq!(press("ver [la guía](https://a.dev) y [«otra»](https://b.dev)", Link), "ver [la guía](https://a.dev) y [otra](«https://b.dev»)");
    assert_eq!(press("ver [la guía](https://a.dev) «y» más", Link), "ver [la guía](https://a.dev) [y](¦) más");
}

/// Enter dentro de una lista o una cita sigue con otro elemento: el número
/// siguiente, una casilla sin marcar, la misma sangría.
///
/// Roturas que atrapa: una línea nueva sin marca; repetir el número; heredar
/// la casilla marcada; perder la sangría o la cita; llevarse a la línea
/// nueva lo de antes del cursor.
#[test]
fn enter_carries_the_list_on() {
    assert_eq!(enter("- uno¦").as_deref(), Some("- uno\n- ¦"));
    assert_eq!(enter("9. uno¦").as_deref(), Some("9. uno\n10. ¦"));
    assert_eq!(enter("- [x] uno¦").as_deref(), Some("- [x] uno\n- [ ] ¦"));
    assert_eq!(enter("  - uno ¦dos").as_deref(), Some("  - uno \n  - ¦dos"));
    assert_eq!(enter("> uno¦").as_deref(), Some("> uno\n> ¦"));
    assert_eq!(enter("> 1. añejo 🎉¦\nfin").as_deref(), Some("> 1. añejo 🎉\n> 2. ¦\nfin"));
}

/// En un elemento vacío, Enter sale de la lista; y donde no hay lista, o no
/// es sitio, Enter es de la caja.
///
/// Roturas que atrapa: no poder salir de una lista (cada Enter añade otro
/// guion); seguir un encabezado en la línea de abajo; tratar como lista una
/// línea de un bloque de código; tragarse lo seleccionado; partir la marca
/// con el cursor dentro de ella.
#[test]
fn enter_leaves_an_empty_item_and_is_plain_elsewhere() {
    assert_eq!(enter("- uno\n- ¦\nfin").as_deref(), Some("- uno\n¦\nfin"));
    assert_eq!(enter("- [ ] ¦").as_deref(), Some("¦"));
    assert_eq!(enter("> ¦").as_deref(), Some("¦"));
    assert_eq!(enter("uno¦"), None);
    assert_eq!(enter("## uno¦"), None);
    assert_eq!(enter("```\n- uno¦"), None);
    assert_eq!(enter("```\nx\n```\n- uno¦").as_deref(), Some("```\nx\n```\n- uno\n- ¦"));
    assert_eq!(enter("- «uno»"), None);
    assert_eq!(enter("-¦ uno"), None);
}
