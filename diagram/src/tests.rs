use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use resvg::tiny_skia::Pixmap;

use super::*;

const JOSE_FLOWCHART: &str = include_str!("../tests/fixtures/jose_flowchart.mmd");
const CLASS: &str =
    "classDiagram\n  class Animal {\n    +String nombre\n    +comer()\n  }\n  Animal <|-- Perro";

fn painter() -> &'static Painter {
    static PAINTER: OnceLock<Painter> = OnceLock::new();
    PAINTER.get_or_init(|| {
        Painter::new(vec![
            include_bytes!("../../desktop/assets/fonts/Inter-Regular.ttf").to_vec(),
            include_bytes!("../../desktop/assets/fonts/Inter-Medium.ttf").to_vec(),
            include_bytes!("../../desktop/assets/fonts/Inter-SemiBold.ttf").to_vec(),
            include_bytes!("../../desktop/assets/fonts/Inter-Bold.ttf").to_vec(),
            include_bytes!("../../desktop/assets/fonts/Inter-Italic.ttf").to_vec(),
        ])
        .expect("the app's Inter loads")
    })
}

fn pixels(picture: &Picture) -> Pixmap {
    let pixmap = Pixmap::decode_png(&picture.png).expect("a PNG");
    assert_eq!(
        (pixmap.width(), pixmap.height()),
        (picture.pixel_width, picture.pixel_height)
    );
    pixmap
}

fn count(picture: &Picture, keep: impl Fn(u8, u8, u8, u8) -> bool) -> usize {
    pixels(picture)
        .pixels()
        .iter()
        .map(|p| p.demultiply())
        .filter(|p| keep(p.red(), p.green(), p.blue(), p.alpha()))
        .count()
}

fn opaque(picture: &Picture) -> usize {
    count(picture, |_, _, _, a| a > 200)
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cac-diagram-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn draws_the_flowchart_jose_wrote() {
    let picture = painter()
        .render(JOSE_FLOWCHART, Theme::Light, 2.0)
        .expect("it draws");
    assert!(
        picture.pixel_width > 400 && picture.pixel_height > 100,
        "{}x{}",
        picture.pixel_width,
        picture.pixel_height
    );
    assert_eq!(picture.width(), picture.pixel_width as f32 / 2.0);
    assert!(
        opaque(&picture) > 10_000,
        "only {} opaque pixels",
        opaque(&picture)
    );
}

#[test]
fn draws_the_four_families_people_write() {
    let sources = [
        JOSE_FLOWCHART,
        // Con un participante implícito (B): la forma habitual de escribirlo.
        "sequenceDiagram\n  participant A as Alicia\n  A->>B: Hola «qué tal»\n  B-->>A: Bien",
        CLASS,
        "stateDiagram-v2\n  [*] --> Abierta\n  Abierta --> EnCurso\n  EnCurso --> Hecha\n  Hecha --> [*]",
    ];
    for source in sources {
        let picture = painter()
            .render(source, Theme::Dark, 1.0)
            .unwrap_or_else(|e| panic!("{e:?} for {source}"));
        assert!(opaque(&picture) > 500, "{source}");
    }
}

#[test]
fn a_kind_not_built_in_says_so() {
    assert_eq!(
        painter().render("gitGraph\n  commit", Theme::Light, 1.0),
        Err(DiagramError::Unsupported)
    );
}

#[test]
fn the_labels_are_painted() {
    // Un solo nodo, sin aristas: lo único oscuro del tema claro es el texto
    // (el borde es lila y el relleno, claro).
    let picture = painter()
        .render(
            "flowchart LR\n  A[\"Etiqueta bastante larga\"]",
            Theme::Light,
            2.0,
        )
        .unwrap();
    let ink = count(&picture, |r, g, b, a| {
        a > 200 && (r as u32 + g as u32 + b as u32) < 3 * 90
    });
    assert!(ink > 300, "only {ink} text pixels");
}

/// El ancho de `text` pintado en Inter, como lo pinta resvg.
fn inter_width(text: &str, weight: u16) -> f32 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="40"><text x="0" y="20" font-family="Inter" font-size="14" font-weight="{weight}">{text}</text></svg>"#
    );
    let options = usvg::Options {
        fontdb: painter().fonts.clone(),
        ..usvg::Options::default()
    };
    usvg::Tree::from_str(&svg, &options)
        .unwrap()
        .root()
        .bounding_box()
        .width()
}

/// El ancho de la caja (`<svg … width="…">`) donde va la etiqueta `text`.
fn label_box_width(svg: &str, text: &str) -> f32 {
    let at = svg
        .find(&format!(">{text}</text>"))
        .unwrap_or_else(|| panic!("no label {text}"));
    let open = svg[..at].rfind("<svg ").unwrap();
    let width = &svg[open..at].split("width=\"").nth(1).unwrap();
    width[..width.find('"').unwrap()].parse().unwrap()
}

#[test]
fn the_class_diagram_labels_fit_their_boxes() {
    // Cada etiqueta va en un `<svg>` con `overflow:hidden` del ancho que mide
    // merman. Con sus tablas fijas, «Animal» medía 39,2 y en Inter mide más:
    // salía «Anima» (command-and-control-573).
    let svg = painter().svg(CLASS, Theme::Light).unwrap();
    for (text, weight) in [("Animal", 700), ("+String nombre", 400), ("+comer()", 400)] {
        let (fits, needs) = (label_box_width(&svg, text), inter_width(text, weight));
        assert!(
            fits + 0.5 >= needs,
            "{text}: the box is {fits} wide, the text {needs}"
        );
    }
}

#[test]
fn a_broken_diagram_says_why() {
    let error = painter()
        .render("flowchart LR\n  A --> \n  [[[", Theme::Light, 1.0)
        .unwrap_err();
    let DiagramError::Invalid(reason) = error else {
        panic!("{error:?}")
    };
    assert!(
        reason.contains("line") || reason.contains("Unterminated") || reason.contains("expected"),
        "{reason}"
    );
}

#[test]
fn the_theme_is_the_apps() {
    let source = "flowchart LR\n  A --> B";
    let light = painter().render(source, Theme::Light, 1.0).unwrap();
    let dark = painter().render(source, Theme::Dark, 1.0).unwrap();
    assert_ne!(light.png, dark.png);
    let with_directive = format!("%%{{init: {{\"theme\": \"forest\"}}}}%%\n{source}");
    assert_eq!(
        painter()
            .render(&with_directive, Theme::Dark, 1.0)
            .unwrap()
            .png,
        dark.png
    );
    let with_frontmatter = format!("---\nconfig:\n  theme: forest\n---\n{source}");
    assert_eq!(
        painter()
            .render(&with_frontmatter, Theme::Dark, 1.0)
            .unwrap()
            .png,
        dark.png
    );
}

#[test]
fn directives_and_frontmatter_are_stripped() {
    assert_eq!(
        strip_directives("%%{init: {\"a\": 1}}%%\nflowchart LR\n A-->B"),
        "\nflowchart LR\n A-->B"
    );
    assert_eq!(
        strip_directives("---\ntitle: x\n---\nflowchart LR"),
        "flowchart LR"
    );
    assert_eq!(
        strip_directives("flowchart LR\n%%{init:\n{}\n}%%\nA"),
        "flowchart LR\n\nA"
    );
    assert_eq!(
        strip_directives("flowchart LR\n%%{ sin cierre\nA-->B"),
        "flowchart LR\nA-->B"
    );
    // Un comentario normal se queda: no cambia nada.
    assert_eq!(
        strip_directives("flowchart LR\n%% nota\nA"),
        "flowchart LR\n%% nota\nA"
    );
}

fn red_png() -> Vec<u8> {
    let mut pixmap = Pixmap::new(8, 8).unwrap();
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    pixmap.encode_png().unwrap()
}

fn reds(picture: &Picture) -> usize {
    count(picture, |r, g, b, a| a > 100 && r > 200 && g < 60 && b < 60)
}

#[test]
fn nothing_outside_the_diagram_is_loaded() {
    let dir = temp_dir("href");
    let file = dir.join("red.png");
    std::fs::write(&file, red_png()).unwrap();
    let data = format!("data:image/png;base64,{}", base64(&red_png()));
    for href in [
        file.to_string_lossy().to_string(),
        format!("file://{}", file.display()),
        data,
    ] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="40" height="40">
                 <rect width="40" height="10" fill="blue"/>
                 <image x="0" y="10" width="40" height="30" href="{href}" xlink:href="{href}"/>
               </svg>"#
        );
        let picture = painter().rasterize(&svg, 1.0).unwrap();
        // El rectángulo sí: se rasterizó de verdad.
        assert!(
            count(&picture, |r, _, b, a| a > 200 && b > 200 && r < 50) > 300,
            "{href}"
        );
        assert_eq!(reds(&picture), 0, "{href} was loaded");
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn a_huge_canvas_is_shrunk_not_cut() {
    for (w, h) in [(100_000, 100_000), (100_000, 50), (50, 100_000)] {
        // Una marca roja en la esquina de abajo a la derecha: si el lienzo se
        // recortara en vez de encogerse, no saldría.
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}">
                 <rect width="{w}" height="{h}" fill="blue"/>
                 <rect x="{}" y="{}" width="{}" height="{}" fill="red"/>
               </svg>"#,
            w / 2,
            h / 2,
            w / 2,
            h / 2
        );
        let picture = painter().rasterize(&svg, 2.0).unwrap();
        assert!(
            picture.pixel_width <= MAX_SIDE && picture.pixel_height <= MAX_SIDE,
            "{w}x{h}"
        );
        let pixmap = pixels(&picture);
        // La última fila o columna puede ser media (el `ceil`): se mira la de antes.
        let corner = pixmap
            .pixel(pixmap.width() - 2, pixmap.height() - 2)
            .unwrap()
            .demultiply();
        assert!(
            corner.red() > 200 && corner.blue() < 60,
            "{w}x{h}: the corner is not there"
        );
    }
}

#[test]
fn too_long_is_refused() {
    let source = format!(
        "flowchart LR\n{}",
        "  A --> B\n".repeat(MAX_SOURCE_BYTES / 10)
    );
    assert!(source.len() > MAX_SOURCE_BYTES);
    assert_eq!(
        painter().render(&source, Theme::Light, 1.0),
        Err(DiagramError::TooLong {
            bytes: source.len()
        })
    );
    let cache = Cache::new(temp_dir("too-long"));
    let counter = Counting::default();
    assert!(matches!(
        cache.get_or_draw(&counter, &source, Theme::Light, 1.0),
        Err(DiagramError::TooLong { .. })
    ));
    assert_eq!(counter.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn too_many_edges_is_refused() {
    // Bajo el límite de bytes, pero por encima del presupuesto de aristas de
    // merman: un grupo `a0 & … --> b0 & …` son 60 × 60 aristas en 700 bytes.
    let side = |p: &str| {
        (0..60)
            .map(|i| format!("{p}{i}"))
            .collect::<Vec<_>>()
            .join(" & ")
    };
    let source = format!("flowchart LR\n  {} --> {}\n", side("a"), side("b"));
    assert!(source.len() <= MAX_SOURCE_BYTES, "{}", source.len());
    match painter().render(&source, Theme::Light, 1.0) {
        Err(DiagramError::Invalid(reason)) => {
            assert!(reason.to_lowercase().contains("limit"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_pinned_merman_is_the_one_in_the_key() {
    let manifest = include_str!("../Cargo.toml");
    assert!(
        manifest.contains(&format!("merman = {{ version = \"={MERMAN_VERSION}\"")),
        "MERMAN_VERSION is not the pinned merman"
    );
}

/// Dibuja un PNG de 2×2 y cuenta cuántas veces se le pidió.
#[derive(Default)]
struct Counting {
    calls: AtomicUsize,
}

impl Draw for Counting {
    fn draw(&self, _: &str, _: Theme, scale: f32) -> Result<Picture, DiagramError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut pixmap = Pixmap::new(2, 2).unwrap();
        pixmap.fill(resvg::tiny_skia::Color::BLACK);
        Ok(Picture {
            png: pixmap.encode_png().unwrap(),
            pixel_width: 2,
            pixel_height: 2,
            scale,
        })
    }
}

#[test]
fn cache_hits_by_content_and_theme() {
    let dir = temp_dir("hits");
    let counter = Counting::default();
    let calls = || counter.calls.load(Ordering::SeqCst);
    let cache = Cache::new(&dir);
    let first = cache
        .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Light, 2.0)
        .unwrap();
    assert_eq!(calls(), 1);
    assert_eq!(
        cache
            .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Light, 2.0)
            .unwrap(),
        first
    );
    assert_eq!(calls(), 1, "the second time is drawn again");
    // Otra caché sobre la misma carpeta: la app reabierta, sin red.
    let reopened = Cache::new(&dir);
    assert_eq!(
        reopened
            .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Light, 2.0)
            .unwrap(),
        first
    );
    assert_eq!(calls(), 1, "what is on disk is not read");
    // Una directiva no cambia el dibujo, así que tampoco la clave.
    reopened
        .get_or_draw(
            &counter,
            "%%{init:{}}%%flowchart LR\n A-->B",
            Theme::Light,
            2.0,
        )
        .unwrap();
    assert_eq!(calls(), 1);
    reopened
        .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Dark, 2.0)
        .unwrap();
    assert_eq!(calls(), 2, "another theme is not drawn");
    reopened
        .get_or_draw(&counter, "flowchart LR\n A-->C", Theme::Dark, 2.0)
        .unwrap();
    assert_eq!(calls(), 3, "another text is not drawn");
    reopened
        .get_or_draw(&counter, "flowchart LR\n A-->C", Theme::Dark, 1.0)
        .unwrap();
    assert_eq!(calls(), 4, "another scale is not drawn");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_shrunk_diagram_has_the_same_size_drawn_and_from_the_cache() {
    // Cuarenta nodos en fila: a 2× pasa de `MAX_SIDE` y se encoge. El primer
    // dibujo y la caché (la misma y una reabierta) tienen que ocupar lo mismo
    // en pantalla; antes el primero salía cuatro veces más ancho.
    let source = format!(
        "flowchart LR\n{}",
        (0..40)
            .map(|i| format!("  N{i} --> N{}\n", i + 1))
            .collect::<String>()
    );
    let dir = temp_dir("shrunk");
    let drawn = Cache::new(&dir)
        .get_or_draw(painter(), &source, Theme::Light, 2.0)
        .unwrap();
    assert_eq!(drawn.pixel_width, MAX_SIDE, "the diagram is not shrunk");
    let cached = Cache::new(&dir)
        .get_or_draw(painter(), &source, Theme::Light, 2.0)
        .unwrap();
    let cache = Cache::new(&dir);
    let again = cache
        .get_or_draw(painter(), &source, Theme::Light, 2.0)
        .unwrap();
    let again_same_cache = cache
        .get_or_draw(painter(), &source, Theme::Light, 2.0)
        .unwrap();
    for (what, picture) in [
        ("reopened", &cached),
        ("another reopened", &again),
        ("same cache", &again_same_cache),
    ] {
        assert_eq!(
            (picture.width(), picture.height()),
            (drawn.width(), drawn.height()),
            "{what}"
        );
    }
    // Sin borrón: a la escala pedida, el diagrama no ocupa más píxeles de
    // pantalla de los que tiene el PNG.
    assert!(drawn.width() * 2.0 <= drawn.pixel_width as f32 + 0.5);
    assert!(drawn.height() * 2.0 <= drawn.pixel_height as f32 + 0.5);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_scale_that_is_not_a_number_draws_at_one_also_from_the_cache() {
    let dir = temp_dir("bad-scale");
    for scale in [0.0, f32::NAN] {
        let drawn = Cache::new(&dir)
            .get_or_draw(painter(), "flowchart LR\n A-->B", Theme::Light, scale)
            .unwrap();
        let cached = Cache::new(&dir)
            .get_or_draw(painter(), "flowchart LR\n A-->B", Theme::Light, scale)
            .unwrap();
        assert_eq!(drawn.scale, 1.0);
        assert_eq!((cached.width(), cached.height()), (drawn.width(), drawn.height()));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_cached_file_that_is_not_ours_is_drawn_again() {
    let dir = temp_dir("not-ours");
    let counter = Counting::default();
    let cache = Cache::new(&dir);
    cache
        .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Light, 1.0)
        .unwrap();
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    // Una cabecera que dice 100 000 × 100 000: decodificarla sería reservar 40 GB.
    let mut huge = std::fs::read(&file).unwrap();
    huge[16..20].copy_from_slice(&100_000u32.to_be_bytes());
    huge[20..24].copy_from_slice(&100_000u32.to_be_bytes());
    std::fs::write(&file, &huge).unwrap();
    cache
        .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Light, 1.0)
        .unwrap();
    std::fs::write(&file, b"not a png").unwrap();
    cache
        .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Light, 1.0)
        .unwrap();
    assert_eq!(counter.calls.load(Ordering::SeqCst), 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn errors_are_not_cached() {
    let dir = temp_dir("errors");
    let cache = Cache::new(&dir);
    assert!(cache
        .get_or_draw(
            painter(),
            "flowchart LR\n  A --> \n  [[[",
            Theme::Light,
            1.0
        )
        .is_err());
    assert_eq!(std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_cache_forgets_the_oldest() {
    let dir = temp_dir("purge");
    let png = Counting::default().draw("", Theme::Light, 1.0).unwrap().png;
    let now = SystemTime::now();
    for i in 0..CACHE_LIMIT {
        let path = dir.join(format!("old-{i:03}.png"));
        std::fs::write(&path, &png).unwrap();
        // old-000 es el más viejo.
        let file = std::fs::File::options().append(true).open(&path).unwrap();
        file.set_modified(now - Duration::from_secs(10_000 - i as u64))
            .unwrap();
    }
    let cache = Cache::new(&dir);
    cache
        .get_or_draw(
            &Counting::default(),
            "flowchart LR\n A-->B",
            Theme::Light,
            1.0,
        )
        .unwrap();
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left.len(), CACHE_LIMIT);
    assert!(
        !left.contains(&"old-000.png".to_string()),
        "the oldest stayed"
    );
    assert!(left.contains(&"old-001.png".to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reading_from_the_cache_makes_it_recent() {
    let dir = temp_dir("recent");
    let counter = Counting::default();
    let cache = Cache::new(&dir);
    cache
        .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Light, 1.0)
        .unwrap();
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let old = SystemTime::now() - Duration::from_secs(100_000);
    std::fs::File::options()
        .append(true)
        .open(&file)
        .unwrap()
        .set_modified(old)
        .unwrap();
    cache
        .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Light, 1.0)
        .unwrap();
    assert!(
        std::fs::metadata(&file).unwrap().modified().unwrap() > old + Duration::from_secs(1000)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_write_has_its_own_temporary_file() {
    // Dos hilos con el mismo temporal escribirían el uno sobre el otro y el
    // `rename` podría dejar el PNG de un diagrama bajo la clave de otro.
    let cache = Cache::new(temp_dir("temp-names"));
    assert_ne!(cache.temp_path(), cache.temp_path());
}

#[test]
fn drawing_the_same_diagram_at_once_leaves_one_whole_file() {
    let dir = temp_dir("race");
    let counter = Counting::default();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let cache = Cache::new(&dir);
                for _ in 0..20 {
                    cache
                        .get_or_draw(&counter, "flowchart LR\n A-->B", Theme::Light, 1.0)
                        .unwrap();
                }
            });
        }
    });
    let files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(Pixmap::decode_png(&std::fs::read(&files[0]).unwrap()).is_ok());
    let _ = std::fs::remove_dir_all(dir);
}
