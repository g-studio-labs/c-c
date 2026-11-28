//! Los bloques ```mermaid del markdown, dibujados como diagrama
//! (command-and-control-573).
//!
//! Los dibuja `cac-diagram` dentro del proceso, sin red: un PNG con la Inter
//! de la app, que aquí sólo se enseña. Todo markdown de la app pasa por
//! [`markdown`], que es quien enseña al `TextView` a reconocer el bloque; la
//! prueba `every_markdown_goes_through_the_diagrams_helper` lo vigila.
//!
//! Lo que se ve, como en la web (`MermaidDiagram.tsx`): mientras se dibuja, el
//! código atenuado; dibujado, la imagen, que se desliza en horizontal si no
//! cabe; con un error, el código con el motivo.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use cac_diagram::{Cache, DiagramError, Painter, Theme};
use gpui_kit::component::text::{markdown_ast::Node, MarkdownNode, MarkdownParseContext, TextView};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    div, img, px, AnyElement, App, AppContext as _, ElementId, Global, Image, ImageFormat, InteractiveElement as _,
    IntoElement, ParentElement as _, RenderImage, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window,
};

use crate::lang::tr;

/// El nombre del bloque en el `TextView`.
const BLOCK: &str = "mermaid";

/// Más estados que esto en memoria y se olvidan los ya resueltos: lo dibujado
/// sigue en el disco.
const REMEMBERED: usize = 256;

/// El texto de un bloque ```mermaid.
struct Mermaid(SharedString);

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    source: SharedString,
    theme: Theme,
    scale: u32,
}

#[derive(Clone)]
enum State {
    Pending,
    Ready { image: Arc<RenderImage>, width: f32, height: f32 },
    Failed(DiagramError),
}

pub(crate) struct Diagrams {
    /// Se crea en segundo plano la primera vez que hace falta: cargar las
    /// fuentes para medir no va en el hilo que pinta.
    painter: Arc<OnceLock<Result<Painter, DiagramError>>>,
    cache: Option<Arc<Cache>>,
    states: HashMap<Key, State>,
    /// En las pruebas, retiene el dibujo hasta que la prueba lo suelta.
    #[cfg(test)]
    hold: Option<Arc<tokio::sync::Semaphore>>,
}

impl Global for Diagrams {}

impl Diagrams {
    fn new(cache_dir: Option<PathBuf>) -> Self {
        Diagrams {
            painter: Arc::new(OnceLock::new()),
            cache: cache_dir.map(|dir| Arc::new(Cache::new(dir))),
            states: HashMap::new(),
            #[cfg(test)]
            hold: None,
        }
    }

    /// Con la caché en disco en `cache_dir` (`system::cache_dir`). Sin ella
    /// se dibuja igual, pero cada arranque vuelve a dibujar.
    pub(crate) fn install(cache_dir: Option<PathBuf>, cx: &mut App) {
        cx.set_global(Diagrams::new(cache_dir.map(|dir| dir.join("diagrams"))));
    }

    #[cfg(test)]
    pub(crate) fn install_held(cache_dir: Option<PathBuf>, cx: &mut App) -> Arc<tokio::sync::Semaphore> {
        let hold = Arc::new(tokio::sync::Semaphore::new(0));
        let mut diagrams = Diagrams::new(cache_dir);
        diagrams.hold = Some(hold.clone());
        cx.set_global(diagrams);
        hold
    }

    /// El estado de ese diagrama; si no se había pedido, se pide.
    fn state(key: Key, cx: &mut App) -> State {
        if !cx.has_global::<Diagrams>() {
            Diagrams::install(None, cx);
        }
        if !cx.global::<Diagrams>().states.contains_key(&key) {
            Diagrams::request(key.clone(), cx);
        }
        cx.global::<Diagrams>().states[&key].clone()
    }

    /// Lo apunta como pendiente y lo dibuja en segundo plano.
    fn request(key: Key, cx: &mut App) {
        let diagrams = cx.global_mut::<Diagrams>();
        if diagrams.states.len() >= REMEMBERED {
            diagrams.states.retain(|_, state| matches!(state, State::Pending));
        }
        diagrams.states.insert(key.clone(), State::Pending);
        let (painter, cache) = (diagrams.painter.clone(), diagrams.cache.clone());
        #[cfg(test)]
        let hold = diagrams.hold.clone();
        let svg = cx.svg_renderer();
        let work = key.clone();
        let task = cx.background_spawn(async move {
            #[cfg(test)]
            if let Some(hold) = hold {
                let _ = hold.acquire().await.map(|permit| permit.forget());
            }
            let painter = match painter.get_or_init(|| Painter::new(crate::theme::INTER_FACES.iter().map(|face| face.to_vec()).collect())) {
                Ok(painter) => painter,
                Err(error) => return State::Failed(error.clone()),
            };
            let scale = f32::from_bits(work.scale);
            let drawn = match &cache {
                Some(cache) => cache.get_or_draw(painter, &work.source, work.theme, scale),
                None => painter.render(&work.source, work.theme, scale),
            };
            match drawn {
                // Decodificado aquí, fuera del hilo que pinta.
                Ok(picture) => match Image::from_bytes(ImageFormat::Png, picture.png.clone()).to_image_data(svg) {
                    Ok(image) => State::Ready { image, width: picture.width(), height: picture.height() },
                    Err(error) => State::Failed(DiagramError::Invalid(error.to_string())),
                },
                Err(error) => State::Failed(error),
            }
        });
        cx.spawn(async move |cx| {
            let state = task.await;
            let _ = cx.update(|cx| {
                if cx.has_global::<Diagrams>() {
                    cx.global_mut::<Diagrams>().states.insert(key, state);
                }
                // Sin esto la ventana no se entera y sigue el código
                // atenuado hasta que algo la mueva. Ninguna prueba lo caza:
                // el arnés pinta siempre que mira (`render_frame`), y saber
                // si GPUI pidió un cuadro es `pub(crate)` suyo.
                cx.refresh_windows();
            });
        })
        .detach();
    }
}

/// El markdown de la app, con los diagramas. Todo `TextView::markdown` pasa
/// por aquí.
pub(crate) fn markdown(id: impl Into<ElementId>, text: impl Into<SharedString>) -> TextView {
    TextView::markdown(id, text).markdown_block_parser(parse).markdown_block_renderer(BLOCK, render)
}

fn parse(node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
    let Node::Code(code) = node else { return None };
    if !code.lang.as_deref().is_some_and(|lang| lang.trim().eq_ignore_ascii_case(BLOCK)) {
        return None;
    }
    Some(
        MarkdownNode::new(BLOCK, Mermaid(code.value.clone().into()))
            .text(code.value.clone())
            .markdown(cx.node_source(node).unwrap_or_default().to_string()),
    )
}

fn render(node: &MarkdownNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let Some(Mermaid(source)) = node.data::<Mermaid>() else { return div().into_any_element() };
    let theme = if cx.theme().mode.is_dark() { Theme::Dark } else { Theme::Light };
    let key = Key { source: source.clone(), theme, scale: window.scale_factor().to_bits() };
    // Un id por diagrama: dos en el mismo texto no comparten el desliz.
    let slot = ElementId::from((SharedString::from("mermaid"), hash(&key.source)));
    let body = match Diagrams::state(key, cx) {
        State::Ready { image, width, height } => div()
            .id("mermaid-diagram")
            .aria_label(tr(cx, "common:last.mermaidDiagram"))
            .w_full()
            .overflow_x_scroll()
            .child(img(image).w(px(width)).h(px(height)).flex_none())
            .test_support()
            .into_any_element(),
        State::Pending => code(source, cx).id("mermaid-loading").opacity(0.5).test_support().into_any_element(),
        State::Failed(error) => {
            let reason = match &error {
                DiagramError::TooLong { .. } => tr(cx, "common:last.mermaidTooLong"),
                DiagramError::Unsupported => tr(cx, "common:last.mermaidUnsupported"),
                DiagramError::Invalid(reason) | DiagramError::Fonts(reason) => reason.clone(),
            };
            let message = format!("{} {reason}", tr(cx, "common:last.mermaidError"));
            div()
                .id("mermaid-error")
                .aria_label(message.clone())
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().warning)
                .child(div().text_sm().text_color(cx.theme().warning).child(message))
                .child(code(source, cx))
                .test_support()
                .into_any_element()
        }
    };
    div().id(slot).my_2().child(body).into_any_element()
}

/// El código del diagrama, como un bloque de código.
fn code(source: &SharedString, cx: &App) -> gpui_kit::Div {
    let theme = cx.theme();
    div()
        .p_3()
        .rounded_md()
        .bg(theme.muted)
        .text_xs()
        .font_family(theme.mono_font_family.clone())
        .whitespace_normal()
        .children(source.lines().map(|line| div().min_h(px(16.)).child(SharedString::from(line.to_string()))))
}

fn hash(text: &str) -> usize {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish() as usize
}

#[cfg(test)]
mod tests {
    /// Nadie pinta markdown sin pasar por [`super::markdown`]: un
    /// `TextView::markdown` suelto enseña un ```mermaid como código.
    ///
    /// Roturas que atrapa: una pantalla nueva (o una de las de ahora) que
    /// llama a `TextView::markdown` directamente.
    #[test]
    fn every_markdown_goes_through_the_diagrams_helper() {
        let needle = concat!("TextView", "::markdown(");
        let mut offenders = Vec::new();
        let mut pending = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") && !path.ends_with("diagrams.rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (n, line) in text.lines().enumerate() {
                        if line.contains(needle) {
                            offenders.push(format!("{}:{}", path.display(), n + 1));
                        }
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "markdown painted without diagrams: {offenders:?}");
    }
}
