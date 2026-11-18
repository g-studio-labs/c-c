//! El visor de imágenes de una conversación: una imagen sobre toda la
//! ventana, y con las flechas, las demás del chat (como en WhatsApp).
//!
//! Cuáles hay y en qué orden lo dice el core (`thread_pictures`,
//! `picture_step`). Aquí queda lo de una pantalla: la imagen entera, que se
//! baja con la sesión por la misma caché que las miniaturas, y las teclas.

use cac_core::ThreadPicture;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::chat::ChatView;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;
use crate::thumbs::Thumb;

/// Cuánto se deja alrededor de la imagen: arriba la barra, a los lados las
/// flechas.
const MARGIN_X: f32 = 96.;
const MARGIN_Y: f32 = 72.;

/// El visor abierto: qué imagen enseña (por su ruta, que no cambia aunque
/// lleguen mensajes) y su foco, para las teclas.
pub struct PictureViewer {
    path: String,
    focus: FocusHandle,
}

impl ChatView {
    /// Las imágenes que el visor recorre: las de los mensajes cargados.
    fn pictures(&self, cx: &App) -> Vec<ThreadPicture> {
        cac_core::thread_pictures(&self.messages, Backend::core(cx).base_url())
    }

    /// Abre el visor en esa imagen.
    pub(crate) fn open_viewer(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.viewer = Some(PictureViewer { path, focus });
        cx.notify();
    }

    /// Pasa a la anterior (−1) o a la siguiente (1). En los extremos, nada.
    fn step_viewer(&mut self, by: i32, cx: &mut Context<Self>) {
        let pictures = self.pictures(cx);
        let Some(viewer) = self.viewer.as_mut() else { return };
        let Some(at) = pictures.iter().position(|picture| picture.path == viewer.path) else { return };
        if let Some(to) = cac_core::picture_step(pictures.len(), at, by) {
            viewer.path = pictures[to].path.clone();
            cx.notify();
        }
    }

    fn close_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.viewer = None;
        // Se vuelve a escribir donde se estaba.
        self.composer.field.update(cx, |field, cx| field.focus(window, cx));
        cx.notify();
    }

    /// El visor, si está abierto: sobre toda la ventana.
    pub(crate) fn render_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pictures = self.pictures(cx);
        let viewer = self.viewer.as_ref()?;
        // Si la imagen ya no está (se retiró su mensaje), el visor se cierra.
        let Some(at) = pictures.iter().position(|picture| picture.path == viewer.path) else {
            self.viewer = None;
            return None;
        };
        let focus = viewer.focus.clone();
        let picture = pictures[at].clone();
        // La que se ve y las de al lado, para que pasar no espere.
        for near in [Some(at), at.checked_sub(1), Some(at + 1)].into_iter().flatten().filter_map(|index| pictures.get(index)) {
            let path = near.path.clone();
            self.thumbs.request(
                near.path.clone(),
                move |core| async move { core.chat().picture(&path).await },
                |this: &mut Self| &mut this.thumbs,
                |_, _, _| {},
                window,
                cx,
            );
        }
        let has_previous = cac_core::picture_step(pictures.len(), at, -1).is_some();
        let has_next = cac_core::picture_step(pictures.len(), at, 1).is_some();
        let count = cac_core::text_with(locale(cx), "chat:pictureOf", &[("n", &(at + 1).to_string()), ("total", &pictures.len().to_string())]);
        let name = if picture.name.trim().is_empty() { tr(cx, "chat:picture") } else { picture.name.clone() };
        let size = window.viewport_size();
        let (max_width, max_height) = ((size.width - px(2. * MARGIN_X)).max(px(64.)), (size.height - px(2. * MARGIN_Y)).max(px(64.)));
        let white = gpui_kit::white();

        let shown: AnyElement = match self.thumbs.get(&picture.path) {
            Some(Thumb::Ready(image)) => {
                let natural = image.size(0);
                let (width, height) = (px(natural.width.0.max(1) as f32), px(natural.height.0.max(1) as f32));
                // Entera y sin deformar, sin pasar de su tamaño.
                let scale = (max_width / width).min(max_height / height).min(1.);
                div()
                    .id("picture-viewer-image")
                    .aria_label(name.clone())
                    .child(img(image.clone()).w(width * scale).h(height * scale).object_fit(ObjectFit::Contain))
                    .test_support()
                    .into_any_element()
            }
            Some(Thumb::Failed) => div()
                .id("picture-viewer-missing")
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .text_color(white.opacity(0.7))
                .child(Icon::new(Lucide::Image).size_8())
                .child(name.clone())
                .test_support()
                .into_any_element(),
            _ => div().id("picture-viewer-loading").child(Spinner::new().color(white)).test_support().into_any_element(),
        };

        let (save_path, save_name, open_path, open_name) = (picture.path.clone(), name.clone(), picture.path.clone(), name.clone());
        let bar = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(MARGIN_Y - 16.))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .text_color(white)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .leading_sm()
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(name))
                    .child(div().truncate().text_color(white.opacity(0.7)).child(picture.author.clone())),
            )
            .child(div().id("picture-viewer-count").flex_none().text_color(white.opacity(0.7)).aria_label(count.clone()).child(count).test_support())
            .child(
                Button::new("picture-viewer-save")
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::Download))
                    .tooltip(tr(cx, "work:task.saveFile"))
                    .on_click(cx.listener(move |this, _, window, cx| this.save_file(save_path.clone(), save_name.clone(), window, cx))),
            )
            .child(
                Button::new("picture-viewer-open")
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::ExternalLink))
                    .tooltip(tr(cx, "common:last.openWithSystem"))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_file(open_path.clone(), open_name.clone(), window, cx))),
            )
            .child(
                Button::new("picture-viewer-close")
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::X))
                    .tooltip(tr(cx, "common:action.close"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_viewer(window, cx))),
            );
        let arrow = |id: &'static str, icon: Lucide, said: String, by: i32, cx: &mut Context<Self>| {
            div().absolute().top_0().bottom_0().flex().items_center().map(|this| if by < 0 { this.left_4() } else { this.right_4() }).child(
                Button::new(id)
                    .ghost()
                    .large()
                    .icon(Icon::new(icon).size_6())
                    .tooltip(said)
                    .on_click(cx.listener(move |this, _, _, cx| this.step_viewer(by, cx))),
            )
        };
        let previous = has_previous.then(|| arrow("picture-viewer-previous", Lucide::ChevronLeft, tr(cx, "chat:previousPicture"), -1, cx));
        let next = has_next.then(|| arrow("picture-viewer-next", Lucide::ChevronRight, tr(cx, "chat:nextPicture"), 1, cx));

        Some(
            deferred(
                anchored().position(Point::default()).child(
                    div()
                        .id("picture-viewer")
                        .track_focus(&focus)
                        .w(size.width)
                        .h(size.height)
                        .occlude()
                        .bg(gpui_kit::black().opacity(0.96))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| match event.keystroke.key.as_str() {
                            "left" => {
                                this.step_viewer(-1, cx);
                                cx.stop_propagation();
                            }
                            "right" => {
                                this.step_viewer(1, cx);
                                cx.stop_propagation();
                            }
                            "escape" => {
                                this.close_viewer(window, cx);
                                cx.stop_propagation();
                            }
                            _ => {}
                        }))
                        // Pulsar fuera de la imagen cierra.
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_viewer(window, cx)))
                        .child(div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(shown))
                        .child(bar)
                        .children(previous.map(|this| this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())))
                        .children(next.map(|this| this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())))
                        .test_support(),
                ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}
