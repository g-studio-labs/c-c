//! El editor con una ventana sin pantalla y teclas de verdad: lo que se
//! pulsa pasa por los atajos y por el receptor de texto de GPUI, como en la
//! app. Cada prueba dice qué rotura atrapa.

use block_editor_spike::doc::Kind;
use block_editor_spike::view::{self, BlockEditor};
use gpui_kit::test::{TestInput, TestWindowExt};
use gpui_kit::{px, size, AnyWindowHandle, App, AppContext as _, Bounds, ElementId, ElementInputHandler, Entity, Focusable as _, Point, TestAppContext, WindowBounds, WindowOptions};

struct Harness {
    window: AnyWindowHandle,
    editor: Entity<BlockEditor>,
}

fn open(source: &str, cx: &mut TestAppContext) -> Harness {
    cx.update(|cx| {
        gpui_kit::init(cx);
        view::install(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(900.), px(700.)) })),
            ..WindowOptions::default()
        };
        let (window, editor) = gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|cx| BlockEditor::new(source, cx));
            view.focus_handle(cx).focus(window, cx);
            view
        })
        .expect("open test window");
        Harness { window, editor }
    })
}

impl Harness {
    fn press(&self, keys: &str, cx: &mut TestAppContext) {
        for key in keys.split(' ') {
            cx.update_window(self.window, |_, window, cx| {
                window.render_frame(cx);
                window.press(key, cx);
            })
            .unwrap();
        }
    }

    fn write(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            window.input(text, cx);
        })
        .unwrap();
    }

    fn frame(&self, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx)).unwrap();
    }

    fn place(&self, block: usize, offset: usize, cx: &mut TestAppContext) {
        self.editor.update(cx, |editor, cx| editor.place(block, offset, cx));
        self.frame(cx);
    }

    fn markdown(&self, cx: &mut TestAppContext) -> String {
        self.editor.read_with(cx, |editor, _| editor.to_markdown())
    }

    fn cursor(&self, cx: &mut TestAppContext) -> (usize, usize) {
        self.editor.read_with(cx, |editor, _| editor.cursor())
    }

    fn kinds(&self, cx: &mut TestAppContext) -> Vec<Kind> {
        self.editor.read_with(cx, |editor, _| editor.doc.blocks.iter().map(|block| block.kind).collect())
    }

    /// Lo que el sistema le manda al campo mientras se compone una letra.
    fn ime<R>(&self, cx: &mut TestAppContext, act: impl FnOnce(&mut TestInput, &mut App) -> R) -> R {
        let editor = self.editor.clone();
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            let mut input = TestInput::new(ElementInputHandler::new(Bounds::default(), editor), window);
            act(&mut input, cx)
        })
        .unwrap()
    }
}

/// Enter parte el bloque donde está el cursor y retroceso al principio lo
/// vuelve a unir, con el cursor en la costura.
///
/// Roturas que atrapa: teclas que no llegan al editor (sin atajo, sin foco);
/// Enter que mete un salto en el texto en vez de partir; un retroceso que
/// borra una letra del bloque de arriba o deja el cursor en otro sitio.
#[gpui_kit::test]
fn enter_splits_and_backspace_joins(cx: &mut TestAppContext) {
    let h = open("Hola mundo\n\nFin\n", cx);
    h.place(0, 4, cx);
    h.press("enter", cx);
    assert_eq!(h.markdown(cx), "Hola\n\n mundo\n\nFin\n");
    assert_eq!(h.cursor(cx), (1, 0));
    h.press("backspace", cx);
    assert_eq!(h.markdown(cx), "Hola mundo\n\nFin\n");
    assert_eq!(h.cursor(cx), (0, 4));
    // Dentro del texto, retroceso borra una letra; y una letra con acento
    // es una letra.
    h.write("ñ", cx);
    assert_eq!(h.markdown(cx), "Holañ mundo\n\nFin\n");
    h.press("backspace", cx);
    assert_eq!(h.markdown(cx), "Hola mundo\n\nFin\n");
    // Suprimir al final sube el párrafo de abajo.
    h.press("end delete", cx);
    assert_eq!(h.markdown(cx), "Hola mundoFin\n");
    assert_eq!(h.cursor(cx), (0, 10));
}

/// Escribir `# `, `- `, `1. ` o la valla al principio convierte el bloque, y
/// la lista sigue con Enter y se deja con Enter en un elemento vacío.
///
/// Roturas que atrapa: la marca se queda como texto; el tipo cambia pero el
/// markdown guardado no; la lista no continúa o no se puede salir de ella;
/// Enter dentro del código parte el bloque.
#[gpui_kit::test]
fn typing_a_marker_converts_the_block(cx: &mut TestAppContext) {
    let h = open("", cx);
    h.write("# Título", cx);
    h.press("enter", cx);
    h.write("## Otro", cx);
    h.press("enter", cx);
    h.write("### Tercero", cx);
    h.press("enter", cx);
    h.write("- uno", cx);
    h.press("enter", cx);
    h.write("dos", cx);
    h.press("enter enter", cx);
    h.write("1. primero", cx);
    h.press("enter", cx);
    h.write("segundo", cx);
    h.press("enter enter", cx);
    h.write("```let x = 1;", cx);
    h.press("enter", cx);
    h.write("let y = 2;", cx);
    h.press("ctrl-enter", cx);
    h.write("Fin.", cx);
    assert_eq!(
        h.markdown(cx),
        "# Título\n\n## Otro\n\n### Tercero\n\n- uno\n- dos\n\n1. primero\n2. segundo\n\n```\nlet x = 1;\nlet y = 2;\n```\n\nFin."
    );
    use Kind::*;
    assert_eq!(h.kinds(cx), [Heading(1), Heading(2), Heading(3), Bullet, Bullet, Numbered, Numbered, Code, Paragraph]);
    // Retroceso al principio de un título lo deja en párrafo, sin unirlo.
    h.place(1, 0, cx);
    h.press("backspace", cx);
    assert_eq!(h.kinds(cx)[1], Paragraph);
    assert!(h.markdown(cx).starts_with("# Título\n\nOtro\n\n### Tercero"));
}

/// Los acentos del teclado español en X11: la tecla muerta deja su marca a
/// medio componer y la vocal la sustituye. Es la secuencia que manda
/// `gpui-pre-linux` (`x11/client.rs`): `handle_ime_preedit("´")` y luego la
/// tecla con `key_char = "á"`.
///
/// Roturas que atrapa: que quede «´á» (la marca no se sustituye); que la
/// marca a medias dispare un atajo o mueva el cursor; que lo compuesto se
/// escriba al final del bloque y no en el cursor; que un acento seguido de
/// algo que no compone (´ y espacio) se pierda; contar en bytes lo que el
/// sistema cuenta en UTF-16 (después de «ñ» o de un emoji cae en mal sitio).
#[gpui_kit::test]
fn spanish_dead_keys_compose_in_place(cx: &mut TestAppContext) {
    let h = open("ctón 🚀 fin\n\notro\n", cx);
    h.place(0, 1, cx);
    h.ime(cx, |input, cx| {
        input.compose_text(None, "´", None, cx);
        assert_eq!(input.marked_text_range(cx), Some(1..2));
    });
    assert_eq!(h.markdown(cx), "c´tón 🚀 fin\n\notro\n");
    h.write("á", cx);
    assert_eq!(h.markdown(cx), "cátón 🚀 fin\n\notro\n");
    assert_eq!(h.cursor(cx), (0, 1 + "á".len()));
    assert_eq!(h.ime(cx, |input, cx| input.marked_text_range(cx)), None);

    // La diéresis, detrás del emoji: el sistema habla en UTF-16.
    let after_rocket = "cátón 🚀".len();
    h.place(0, after_rocket, cx);
    let selected = h.ime(cx, |input, cx| input.selected_text_range(cx).unwrap().range);
    assert_eq!(selected, 8..8, "«cátón 🚀» son 8 unidades UTF-16, no {after_rocket} bytes");
    h.ime(cx, |input, cx| input.compose_text(None, "¨", None, cx));
    h.write("ü", cx);
    assert_eq!(h.markdown(cx), "cátón 🚀ü fin\n\notro\n");

    // Acento y algo que no compone: X11 confirma la marca tal cual
    // (`handle_ime_commit`) y sigue.
    h.ime(cx, |input, cx| {
        input.compose_text(None, "´", None, cx);
        input.commit_text("´", cx);
    });
    h.write("x", cx);
    assert_eq!(h.markdown(cx), "cátón 🚀ü´x fin\n\notro\n");

    // Las que no son teclas muertas entran directas, también al principio.
    h.place(1, 0, cx);
    h.write("¿Ñandú, pingüino? ", cx);
    assert_eq!(h.markdown(cx), "cátón 🚀ü´x fin\n\n¿Ñandú, pingüino? otro\n");
}

/// Las flechas pasan de un bloque a otro. Arriba y abajo mantienen la
/// columna aunque por medio haya un bloque más corto; izquierda y derecha
/// saltan por los bordes.
///
/// Roturas que atrapa: quedarse atrapado en un bloque; entrar siempre por el
/// principio; perder la columna al cruzar un bloque corto; contar la columna
/// en bytes (con «ñ» delante cae una letra más allá).
#[gpui_kit::test]
fn arrows_move_between_blocks_keeping_the_column(cx: &mut TestAppContext) {
    let h = open("ñandúes verdes\n\nab\n\nñandúes verdes\n\n- ñandúes verdes\n", cx);
    let column = "ñandúes".len();
    h.place(0, column, cx);
    h.press("down", cx);
    assert_eq!(h.cursor(cx), (1, 2), "en el bloque corto, al final");
    h.press("down", cx);
    assert_eq!(h.cursor(cx), (2, column), "la columna se recupera al salir del corto");
    h.press("up up", cx);
    assert_eq!(h.cursor(cx), (0, column));
    h.press("up", cx);
    assert_eq!(h.cursor(cx), (0, 0), "arriba en el primero: al principio");

    h.place(1, 2, cx);
    h.press("right", cx);
    assert_eq!(h.cursor(cx), (2, 0));
    h.press("left", cx);
    assert_eq!(h.cursor(cx), (1, 2));
    h.press("left left left", cx);
    assert_eq!(h.cursor(cx), (0, "ñandúes verdes".len()));

    // Hacia un elemento de lista, que empieza más a la derecha: la columna
    // es la de la pantalla, así que cae unas letras antes, no al principio
    // ni al final.
    h.place(2, column, cx);
    h.press("down", cx);
    let (block, offset) = h.cursor(cx);
    assert_eq!(block, 3);
    assert!(offset > 0 && offset < column, "cayó en {offset}");
    h.press("down", cx);
    assert_eq!(h.cursor(cx), (3, "ñandúes verdes".len()), "abajo en el último: al final");
}

/// Un párrafo largo ocupa varias líneas pintadas: arriba y abajo se mueven
/// por ellas antes de salir del bloque.
///
/// Rotura que atrapa: tratar el bloque como una sola línea (abajo salta al
/// bloque siguiente desde la primera línea de un párrafo de cinco).
#[gpui_kit::test]
fn arrows_walk_the_wrapped_lines_of_a_long_paragraph(cx: &mut TestAppContext) {
    let long = "palabra ".repeat(60);
    let h = open(&format!("{long}\n\nfin\n"), cx);
    h.place(0, 3, cx);
    h.press("down", cx);
    let (block, offset) = h.cursor(cx);
    assert_eq!(block, 0, "abajo salió del párrafo desde su primera línea");
    assert!(offset > 40, "no bajó de línea: {offset}");
    h.press("up", cx);
    assert_eq!(h.cursor(cx), (0, 3));
}

/// Pulsar con el ratón en un bloque pone el cursor en él.
///
/// Rotura que atrapa: un clic que no mueve el cursor o lo lleva a otro
/// bloque (los bloques no reciben el ratón, o lo reciben con el índice de
/// otro).
#[gpui_kit::test]
fn clicking_a_block_puts_the_cursor_there(cx: &mut TestAppContext) {
    let h = open("uno\n\ndos tres cuatro\n\ntres\n", cx);
    let id = h.editor.read_with(cx, |editor, _| editor.doc.blocks[1].id);
    cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        window.click(ElementId::NamedInteger("block".into(), id), cx);
    })
    .unwrap();
    assert_eq!(h.cursor(cx).0, 1);
    h.write("X", cx);
    assert!(h.markdown(cx).starts_with("uno\n\n") && h.markdown(cx).ends_with("\n\ntres\n") && h.markdown(cx).contains('X'));
}

/// Las marcas de negrita, código o enlace sólo se pintan en el bloque que
/// tiene el cursor; en los demás se ve el resultado.
///
/// Roturas que atrapa: marcas a la vista en todo el documento (no es «ya
/// pintado»); o escondidas también donde se escribe (el cursor y el texto
/// dejan de corresponderse byte a byte).
#[gpui_kit::test]
fn markers_show_only_where_the_cursor_is(cx: &mut TestAppContext) {
    let h = open("Con **negrita** y `código`.\n\nUn [enlace](https://example.com \"título\") aquí.\n", cx);
    h.place(0, 0, cx);
    let painted = |cx: &mut TestAppContext, block| h.editor.read_with(cx, |editor, _| editor.painted_text(block).expect("no se pintó"));
    assert_eq!(painted(cx, 0), "Con **negrita** y `código`.");
    assert_eq!(painted(cx, 1), "Un enlace aquí.");
    h.press("down", cx);
    h.frame(cx);
    assert_eq!(painted(cx, 0), "Con negrita y código.");
    assert_eq!(painted(cx, 1), "Un [enlace](https://example.com \"título\") aquí.");
    // Y nada de eso cambió el documento.
    assert_eq!(h.markdown(cx), "Con **negrita** y `código`.\n\nUn [enlace](https://example.com \"título\") aquí.\n");
}

/// Pasear el cursor por todo un documento de verdad, sin escribir, no
/// cambia un byte; y escribir dentro de una tabla (opaca) la retoca como
/// texto sin tocar lo demás.
///
/// Roturas que atrapa: que pintar o mover el cursor normalice algo; que un
/// bloque opaco no se pueda editar de ninguna manera, o que Enter dentro de
/// él lo parta.
#[gpui_kit::test]
fn walking_through_a_real_document_changes_nothing(cx: &mut TestAppContext) {
    let source = include_str!("../corpus/runbook.md");
    let h = open(source, cx);
    for _ in 0..120 {
        h.press("down", cx);
    }
    let last = h.editor.read_with(cx, |editor, _| editor.doc.blocks.len() - 1);
    assert_eq!(h.cursor(cx).0, last, "las flechas no llegaron al final");
    for _ in 0..40 {
        h.press("up right right left", cx);
    }
    assert_eq!(h.markdown(cx), source);

    let table = h.editor.read_with(cx, |editor, _| editor.doc.blocks.iter().position(|block| block.text.starts_with("| Variable")).unwrap());
    let end = h.editor.read_with(cx, |editor, _| editor.doc.blocks[table].text.len());
    h.place(table, end, cx);
    h.press("enter", cx);
    h.write("| `NUEVA` | Añadida | — |", cx);
    let expected = source.replace("| —           |\n", "| —           |\n| `NUEVA` | Añadida | — |\n");
    assert_ne!(expected, source);
    assert_eq!(h.markdown(cx), expected);
}

/// Con tres mil bloques sólo se pintan los que caben (y un margen), y el
/// cursor se puede llevar al final.
///
/// Roturas que atrapa: pintar el documento entero en cada fotograma (quitar
/// la lista virtual); que el bloque del cursor no se traiga a la vista.
#[gpui_kit::test]
fn a_long_document_paints_only_what_is_in_view(cx: &mut TestAppContext) {
    let source: String = (0..3000).map(|ix| format!("Párrafo {ix} con **algo** de texto.\n\n")).collect();
    let h = open(&source, cx);
    h.frame(cx);
    let painted = h.editor.read_with(cx, |editor, _| editor.painted_count());
    assert!(painted > 5 && painted < 120, "se pintaron {painted} bloques de 3000");
    h.place(2999, 0, cx);
    h.frame(cx);
    assert_eq!(h.editor.read_with(cx, |editor, _| editor.painted_text(2999)).as_deref(), Some("Párrafo 2999 con **algo** de texto."));
    h.write("¡", cx);
    assert!(h.markdown(cx).ends_with("Párrafo 2998 con **algo** de texto.\n\n¡Párrafo 2999 con **algo** de texto.\n\n"));
}
