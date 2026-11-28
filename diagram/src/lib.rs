//! Los diagramas mermaid de cac, dibujados como PNG dentro del proceso
//! (command-and-control-573).
//!
//! El texto de un diagrama lo escribe cualquiera, así que lo que sale de aquí
//! es una imagen de píxeles y nada más: ni enlaces, ni scripts, ni recursos
//! que cargar. Ése es el saneado, y es estructural. `merman` hace el SVG y
//! resvg lo rasteriza con las fuentes que pasa quien llama (la Inter de la
//! app), nunca las del sistema: así el diagrama mide y se ve igual en Linux,
//! macOS y Windows.

mod cache;

pub use cache::{Cache, CACHE_LIMIT};

use std::fmt;
use std::sync::Arc;

use merman::render::SvgEnvironment;
use merman::svg::SvgPipeline;
use merman::{
    MermaidEngine, OperationControl, ParseOptions, RenderError, RenderOutput, RenderRequest,
    Renderer, SourcePresentationConfig, SvgRequest,
};
use merman_export::NativeFontContext;
use resvg::tiny_skia;
use resvg::usvg::{self, fontdb};

/// La versión de `merman` fijada en `Cargo.toml`. Entra en la clave de la
/// caché: un `merman` nuevo dibuja distinto y no debe servir PNG viejos.
pub const MERMAN_VERSION: &str = "0.11.0";

/// Más que esto no se intenta dibujar. Un doc de verdad no pasa de unos pocos
/// KB por diagrama.
pub const MAX_SOURCE_BYTES: usize = 20_000;

/// El lado más largo de la imagen, en píxeles reales. Un viewBox gigante se
/// encoge hasta caber: reservar el pixmap sin tope es pedir memoria sin
/// límite (100 000 × 100 000 son 40 GB).
pub const MAX_SIDE: u32 = 4096;

/// La familia con la que `merman` mide y resvg pinta. Tiene que estar entre
/// las fuentes que recibe `Painter::new`.
const FONT_FAMILY: &str = "Inter";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    fn mermaid_name(self) -> &'static str {
        match self {
            Theme::Light => "default",
            Theme::Dark => "dark",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

/// Un diagrama dibujado. `png` mide `pixel_width × pixel_height`; en pantalla
/// ocupa eso entre `scale` (ver `width` y `height`). `scale` es siempre la
/// pedida, también si el dibujo se encogió para caber en `MAX_SIDE`: así un
/// diagrama encogido sale más pequeño, un píxel del PNG por píxel de la
/// pantalla, y no estirado y borroso; y la caché, que sólo tiene el PNG,
/// puede dar el mismo tamaño que el primer dibujo.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub png: Vec<u8>,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale: f32,
}

impl Picture {
    /// El ancho lógico, en los píxeles de la interfaz.
    pub fn width(&self) -> f32 {
        self.pixel_width as f32 / self.scale
    }

    pub fn height(&self) -> f32 {
        self.pixel_height as f32 / self.scale
    }
}

/// Por qué no se dibujó. El texto que lee una persona lo pone quien pinta,
/// desde su catálogo; `Invalid` trae el motivo del parser tal cual, que es lo
/// que la web enseña también.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagramError {
    TooLong {
        bytes: usize,
    },
    /// Las fuentes que se pasaron a `Painter::new` no se pudieron leer.
    Fonts(String),
    /// Un tipo de diagrama que no va compilado en la app.
    Unsupported,
    Invalid(String),
}

impl fmt::Display for DiagramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagramError::TooLong { bytes } => {
                write!(
                    f,
                    "diagram source is {bytes} bytes, the limit is {MAX_SOURCE_BYTES}"
                )
            }
            DiagramError::Fonts(reason) => write!(f, "diagram fonts: {reason}"),
            DiagramError::Unsupported => write!(f, "this kind of diagram is not supported"),
            DiagramError::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for DiagramError {}

/// Lo que dibuja un diagrama. Es un trait para que la caché se pueda probar
/// contando cuántas veces se dibuja de verdad.
pub trait Draw {
    fn draw(&self, source: &str, theme: Theme, scale: f32) -> Result<Picture, DiagramError>;
}

pub struct Painter {
    /// Con lo que pinta resvg.
    fonts: Arc<fontdb::Database>,
    /// Con lo que mide merman: las mismas fuentes.
    measure: NativeFontContext,
}

impl Painter {
    /// `fonts` son ficheros de fuente enteros (TTF/OTF). Entre ellos tiene que
    /// estar la Inter: es la familia que se pide para todo el texto.
    pub fn new(fonts: Vec<Vec<u8>>) -> Result<Self, DiagramError> {
        let measure = NativeFontContext::from_font_bytes(
            fonts.clone(),
            Default::default(),
            &OperationControl::new(),
        )
        .map_err(|error| DiagramError::Fonts(error.to_string()))?;
        let mut db = fontdb::Database::new();
        for font in fonts {
            db.load_font_data(font);
        }
        // Cualquier familia que pida el SVG (merman pone `"trebuchet ms",
        // verdana, arial, sans-serif` en algunos sitios) acaba en la Inter, y
        // nunca en una fuente del sistema.
        db.set_sans_serif_family(FONT_FAMILY);
        db.set_serif_family(FONT_FAMILY);
        db.set_monospace_family(FONT_FAMILY);
        db.set_cursive_family(FONT_FAMILY);
        db.set_fantasy_family(FONT_FAMILY);
        Ok(Painter {
            fonts: Arc::new(db),
            measure,
        })
    }

    pub fn render(&self, source: &str, theme: Theme, scale: f32) -> Result<Picture, DiagramError> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(DiagramError::TooLong {
                bytes: source.len(),
            });
        }
        let source = strip_directives(source);
        let svg = self.svg(&source, theme)?;
        self.rasterize(&svg, scale)
    }

    fn svg(&self, source: &str, theme: Theme) -> Result<String, DiagramError> {
        let config = SourcePresentationConfig::from_value(serde_json::json!({
            "theme": theme.mermaid_name(),
            "fontFamily": FONT_FAMILY,
        }));
        let renderer = Renderer::new()
            .with_parse_options(ParseOptions::strict())
            .with_presentation_engine(MermaidEngine::new().with_site_config(config));
        // Sin `resvg_safe` las etiquetas van en `foreignObject` (HTML), que
        // resvg no pinta: las cajas salen vacías.
        let environment = SvgEnvironment::deterministic()
            .with_native_font_context(self.measure.clone())
            .map_err(|error| DiagramError::Fonts(error.to_string()))?;
        let request = SvgRequest {
            environment,
            pipeline: Some(SvgPipeline::resvg_safe()),
            ..Default::default()
        };
        let output = renderer
            .render(
                RenderRequest::svg(source, OperationControl::new(), request)
                    .with_parse_options(ParseOptions::strict()),
            )
            .map_err(classify)?;
        match output {
            RenderOutput::Svg(Some(svg)) => Ok(svg.svg().to_string()),
            _ => Err(DiagramError::Unsupported),
        }
    }

    pub(crate) fn rasterize(&self, svg: &str, scale: f32) -> Result<Picture, DiagramError> {
        let options = usvg::Options {
            resources_dir: None,
            font_family: FONT_FAMILY.to_string(),
            fontdb: self.fonts.clone(),
            // Ni `data:`, ni ficheros, ni URLs: un `<image>` del diagrama no
            // carga nada. Con `Options::default()` resvg lee rutas locales.
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(svg, &options)
            .map_err(|error| DiagramError::Invalid(error.to_string()))?;
        let size = tree.size();
        let requested = requested_scale(scale);
        let fit = fit_scale(size.width(), size.height(), requested);
        let width = ((size.width() * fit).ceil() as u32).clamp(1, MAX_SIDE);
        let height = ((size.height() * fit).ceil() as u32).clamp(1, MAX_SIDE);
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| DiagramError::Invalid("empty diagram".into()))?;
        // Fondo transparente: el de la app es el que se ve detrás.
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(fit, fit),
            &mut pixmap.as_mut(),
        );
        let png = pixmap
            .encode_png()
            .map_err(|error| DiagramError::Invalid(error.to_string()))?;
        Ok(Picture {
            png,
            pixel_width: width,
            pixel_height: height,
            scale: requested,
        })
    }
}

impl Draw for Painter {
    fn draw(&self, source: &str, theme: Theme, scale: f32) -> Result<Picture, DiagramError> {
        self.render(source, theme, scale)
    }
}

/// La escala con la que se pide dibujar, o 1 si no es un número que sirva.
pub(crate) fn requested_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// La escala pedida, o menos si así el lado más largo pasaría de `MAX_SIDE`.
fn fit_scale(width: f32, height: f32, scale: f32) -> f32 {
    let longest = width.max(height) * scale;
    if longest > MAX_SIDE as f32 {
        scale * MAX_SIDE as f32 / longest
    } else {
        scale
    }
}

fn classify(error: RenderError) -> DiagramError {
    let reason = error.to_string();
    // Un tipo que no va compilado llega como error de parseo, y su texto
    // habla de features de Cargo: a quien lee el doc no le dice nada.
    if matches!(error, RenderError::NoDiagram) || reason.starts_with("Unsupported diagram type") {
        DiagramError::Unsupported
    } else {
        DiagramError::Invalid(reason)
    }
}

/// El texto sin directivas `%%{…}%%` ni frontmatter `---…---`. Una directiva
/// fija el tema (y la configuración entera): en la app manda su tema, no el
/// del texto. Medido: con `%%{init:{"theme":"forest"}}%%` delante, el diagrama
/// salía igual en claro que en oscuro.
pub fn strip_directives(source: &str) -> String {
    let mut text = source.trim_start();
    if let Some(rest) = text.strip_prefix("---") {
        if rest.starts_with(['\n', '\r']) {
            match rest.find("\n---") {
                Some(end) => {
                    let after = &rest[end + 4..];
                    text = after.split_once('\n').map(|(_, tail)| tail).unwrap_or("");
                }
                None => text = "",
            }
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("%%{") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 3..];
        rest = match tail.find("}%%") {
            Some(end) => &tail[end + 3..],
            // Sin cierre: se va el resto de la línea, nunca entra a medias.
            None => tail.split_once('\n').map(|(_, next)| next).unwrap_or(""),
        };
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;
