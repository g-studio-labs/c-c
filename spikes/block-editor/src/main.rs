//! La ventana de la prueba.
//!
//! `block-editor-spike [fichero.md]` abre el fichero (o el runbook de
//! muestra) y deja editarlo; Ctrl+S lo guarda. Las variables `BLOCKS_*` son
//! ganchos para medir y para capturar sin tocar teclado ni ratón:
//!
//! - `BLOCKS_BENCH=3000`: un documento de ese número de bloques; escribe en
//!   medio, mide cada fotograma y sale. Los números van a `BLOCKS_OUT`;
//!   `BLOCKS_BENCH_KEYS` es cuántas letras (300 si no se dice).
//! - `BLOCKS_NOVIRT=1`: pinta todos los bloques, sin lista virtual.
//! - `BLOCKS_DEMO=1`: escribe un trozo de documento por el mismo camino que
//!   el teclado, para verlo pintado.
//! - `BLOCKS_DUMP=fichero.md` (con `BLOCKS_BENCH`): escribe ese documento
//!   largo en un fichero y sale, sin abrir ventana.
//! - `BLOCKS_HOLD_MS=4000`: se cierra sola pasado ese tiempo.

use std::time::{Duration, Instant};

use block_editor_spike::view::{self, BlockEditor};
use gpui_kit::*;

const SAMPLE: &str = include_str!("../corpus/runbook.md");

/// Un documento largo con pinta de documento: títulos, párrafos con markdown
/// de línea, listas, código y, de vez en cuando, una tabla.
fn long_document(blocks: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut section = 0;
    while count < blocks {
        section += 1;
        out.push_str(&format!("## Sección {section}: despliegue y verificación\n\n"));
        count += 1;
        for paragraph in 0..6 {
            out.push_str(&format!(
                "Párrafo {paragraph} de la sección {section}. El backend despliega **antes** que la app; al revés, una app nueva \
                 habla con un servidor que *no la entiende*. Mira el [tablero](https://example.com/d/{section}) y pasa `go test ./...` \
                 con `DB_HOST` puesto: sin él las pruebas que necesitan Postgres se saltan, y un verde no significa que se hayan corrido todas.\n\n"
            ));
            count += 1;
        }
        for item in 1..=5 {
            out.push_str(&format!("{item}. Paso {item} de la sección {section}, con `código` y **énfasis**.\n"));
            count += 1;
        }
        out.push_str("\n```sh\n(cd backend && go build ./... && go vet ./... && go test ./...)\necho \"hecho\"\n```\n\n");
        count += 1;
        for item in ["Re-cortar una versión publicada.", "Desplegar un viernes.", "Saltarse la firma."] {
            out.push_str(&format!("- {item}\n"));
            count += 1;
        }
        out.push('\n');
        if section % 4 == 0 {
            out.push_str("| Variable | Para qué | Por defecto |\n|:--|:-:|--:|\n| `DB_HOST` | La base de pruebas | *(ninguna)* |\n| `SITE_URL` | El dominio | `localhost` |\n\n");
            count += 1;
        }
    }
    out
}

fn resident_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status.lines().find_map(|line| line.strip_prefix("VmRSS:")).and_then(|rest| rest.split_whitespace().next()?.parse().ok()).unwrap_or(0)
}

fn summary(label: &str, samples: &mut [Duration]) -> String {
    if samples.is_empty() {
        return format!("{label}: sin muestras\n");
    }
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.;
    let at = |q: f64| ms(samples[((samples.len() - 1) as f64 * q).round() as usize]);
    format!("{label}: n={} mediana={:.2}ms p95={:.2}ms max={:.2}ms\n", samples.len(), at(0.5), at(0.95), ms(samples[samples.len() - 1]))
}

/// Espera a que se pinte un fotograma más (o un rato, si no llega).
async fn next_frame(editor: &Entity<BlockEditor>, cx: &mut AsyncApp) {
    let before = editor.read_with(cx, |editor, _| editor.frames.len());
    for _ in 0..20_000 {
        cx.background_executor().timer(Duration::from_micros(500)).await;
        if editor.read_with(cx, |editor, _| editor.frames.len()) > before {
            return;
        }
    }
}

fn main() {
    let var = |name: &str| std::env::var(name).ok();
    let bench: Option<usize> = var("BLOCKS_BENCH").and_then(|value| value.parse().ok());
    let virtualized = var("BLOCKS_NOVIRT").is_none();
    let demo = var("BLOCKS_DEMO").is_some();
    let hold: Option<u64> = var("BLOCKS_HOLD_MS").and_then(|value| value.parse().ok());
    let out = var("BLOCKS_OUT");
    let keys: usize = var("BLOCKS_BENCH_KEYS").and_then(|value| value.parse().ok()).unwrap_or(300);
    let path = std::env::args().nth(1).map(std::path::PathBuf::from);

    let before_source = resident_kb();
    let source = match (&bench, &path) {
        (Some(blocks), _) => long_document(*blocks),
        (None, Some(path)) => std::fs::read_to_string(path).unwrap_or_else(|error| {
            eprintln!("no se pudo leer {}: {error}", path.display());
            std::process::exit(1);
        }),
        (None, None) => SAMPLE.to_string(),
    };
    // El documento largo, a un fichero: para abrirlo con otra cosa.
    if let Some(dump) = var("BLOCKS_DUMP") {
        std::fs::write(&dump, &source).expect("no se pudo escribir el documento");
        return;
    }

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        view::install(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(920.), px(900.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some("block-editor-spike".into()), ..Default::default() }),
            ..Default::default()
        };
        let mut report = String::new();
        let parse_started = Instant::now();
        let parsed = block_editor_spike::doc::Doc::parse(&source);
        let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.;
        assert_eq!(parsed.to_markdown(), source, "leer y guardar no devolvió el mismo texto");
        drop(parsed);
        let mut editor = None;
        let window = gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|cx| {
                let mut view = BlockEditor::new(&source, cx);
                view.virtualized = virtualized;
                view.path = path.clone();
                view
            });
            view.focus_handle(cx).focus(window, cx);
            editor = Some(view.clone());
            view
        })
        .expect("no se pudo abrir la ventana");
        let editor = editor.expect("la ventana no creó el editor");
        report.push_str(&format!(
            "bloques={} bytes={} virtual={virtualized} leer={:.2}ms rss_antes={}kB rss_cargado={}kB\n",
            editor.read(cx).doc.blocks.len(),
            source.len(),
            parse_ms,
            before_source,
            resident_kb()
        ));
        let _ = window;

        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(1200)).await;
            if demo {
                let script = "\n## Escrito por el camino del teclado\nUn párrafo con **negrita**, *cursiva*, `código` y un [enlace](https://example.com). Acentos: áéíóú, ñ, ü, ¿qué tal?\n- primer elemento\nsegundo elemento\n\n1. uno\ndos\n\n```fn main() {}\n";
                editor.update(cx, |editor, cx| {
                    let last = editor.doc.blocks.len() - 1;
                    let end = editor.doc.blocks[last].text.len();
                    editor.place(last, end, cx);
                });
                for ch in script.chars() {
                    editor.update(cx, |editor, cx| editor.type_char(ch, cx));
                    cx.background_executor().timer(Duration::from_millis(2)).await;
                }
            }
            if bench.is_some() {
                let middle = editor.read_with(cx, |editor, _| editor.doc.blocks.len() / 2);
                // Un párrafo de en medio del documento, con el cursor al final.
                editor.update(cx, |editor, cx| {
                    let ix = (middle..editor.doc.blocks.len()).find(|ix| editor.doc.blocks[*ix].kind == block_editor_spike::doc::Kind::Paragraph).unwrap_or(middle);
                    let end = editor.doc.blocks[ix].text.len();
                    editor.place(ix, end, cx);
                });
                next_frame(&editor, cx).await;
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let first_frames = editor.update(cx, |editor, _| std::mem::take(&mut editor.frames));
                let (mut handling, mut frames, mut latency) = (Vec::new(), Vec::new(), Vec::new());
                for ix in 0..keys {
                    let started = Instant::now();
                    let ch = if ix % 7 == 6 { ' ' } else { "añoqué".chars().nth(ix % 6).unwrap() };
                    editor.update(cx, |editor, cx| editor.type_char(ch, cx));
                    handling.push(started.elapsed());
                    next_frame(&editor, cx).await;
                    latency.push(started.elapsed());
                    frames.extend(editor.update(cx, |editor, _| std::mem::take(&mut editor.frames)));
                }
                let (mut split_handling, mut split_frames) = (Vec::new(), Vec::new());
                for _ in 0..keys / 5 {
                    let started = Instant::now();
                    editor.update(cx, |editor, cx| editor.type_char('\n', cx));
                    split_handling.push(started.elapsed());
                    next_frame(&editor, cx).await;
                    split_frames.extend(editor.update(cx, |editor, _| std::mem::take(&mut editor.frames)));
                    editor.update(cx, |editor, cx| editor.type_char('x', cx));
                    next_frame(&editor, cx).await;
                    editor.update(cx, |editor, _| editor.frames.clear());
                }
                let saving = Instant::now();
                let bytes = editor.read_with(cx, |editor, _| editor.to_markdown().len());
                let mut text = String::new();
                text.push_str(&summary("primeros fotogramas (abrir y saltar al medio)", &mut first_frames.clone()));
                text.push_str(&summary("escribir una letra: atender la tecla", &mut handling));
                text.push_str(&summary("escribir una letra: fotograma (render a fin de pintado)", &mut frames));
                text.push_str(&summary("escribir una letra: de la tecla al fin del pintado (incluye esperar al fotograma)", &mut latency));
                text.push_str(&summary("Enter (partir bloque): atender la tecla", &mut split_handling));
                text.push_str(&summary("Enter (partir bloque): fotograma", &mut split_frames));
                text.push_str(&format!("guardar a markdown: {:.2}ms ({bytes} bytes)\nrss_final={}kB\n", saving.elapsed().as_secs_f64() * 1000., resident_kb()));
                report.push_str(&text);
            }
            eprint!("{report}");
            if let Some(out) = &out {
                let _ = std::fs::write(out, &report);
            }
            if let Some(hold) = hold {
                cx.background_executor().timer(Duration::from_millis(hold)).await;
                cx.update(|cx| cx.quit());
            } else if bench.is_some() {
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
    });
}
