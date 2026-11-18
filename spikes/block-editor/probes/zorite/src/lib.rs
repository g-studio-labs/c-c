//! `zorite-editor` 0.11.3 puesto sobre el GPUI de `desktop/` (gpui-kit
//! 0.9.1, gpui-pre 0.3.12): un editor de markdown que pinta mientras se
//! escribe, sobre el texto fuente. Aquí sólo lo mínimo para verlo, medirlo y
//! probarle la fidelidad y los acentos.

use std::time::{Duration, Instant};

use gpui_kit::*;
use zorite_editor::{EditorState, SyntaxStyle};

fn style() -> SyntaxStyle {
    SyntaxStyle {
        block_label: None,
        block_label_gen: 0,
        block_ref_count: None,
        marker: hsla(0., 0., 0.5, 0.55),
        code: hsla(0.09, 0.6, 0.72, 1.),
        code_bg: hsla(0., 0., 1., 0.06),
        link: hsla(0.58, 0.75, 0.66, 1.),
        tag: hsla(0.33, 0.45, 0.62, 1.),
        quote: hsla(0., 0., 0.6, 1.),
        alert_note: hsla(0.58, 0.9, 0.62, 1.),
        alert_tip: hsla(0.36, 0.5, 0.48, 1.),
        alert_important: hsla(0.74, 0.85, 0.73, 1.),
        alert_warning: hsla(0.12, 0.7, 0.48, 1.),
        alert_caution: hsla(0.01, 0.9, 0.63, 1.),
        alert_icons: None,
        rule: hsla(0., 0., 1., 0.18),
        mark_bg: hsla(0.13, 1., 0.5, 0.4),
        popover_bg: hsla(0., 0., 0.16, 1.),
        popover_border: hsla(0., 0., 0.28, 1.),
        popover_fg: hsla(0., 0., 0.9, 1.),
        popover_hover: hsla(0.58, 0.75, 0.66, 0.16),
        popover_divider: hsla(0., 0., 1., 0.18),
        popover_danger: rgb(0xE5484D).into(),
        mono: font("Iosevka Term Slab"),
        property_icon: None,
    }
}

/// Las letras de `desktop/` y las teclas del editor.
pub fn install(cx: &mut App) {
    let faces: [&'static [u8]; 7] = [
        include_bytes!("../../../../../desktop/assets/fonts/Inter-Regular.ttf"),
        include_bytes!("../../../../../desktop/assets/fonts/Inter-Medium.ttf"),
        include_bytes!("../../../../../desktop/assets/fonts/Inter-SemiBold.ttf"),
        include_bytes!("../../../../../desktop/assets/fonts/Inter-Bold.ttf"),
        include_bytes!("../../../../../desktop/assets/fonts/Inter-Italic.ttf"),
        include_bytes!("../../../../../desktop/assets/fonts/IosevkaTermSlab-Regular.ttf"),
        include_bytes!("../../../../../desktop/assets/fonts/IosevkaTermSlab-Bold.ttf"),
    ];
    let _ = cx.text_system().add_fonts(faces.into_iter().map(std::borrow::Cow::Borrowed).collect());
    zorite_editor::bind_keys(cx);
}

pub struct Probe {
    pub editor: Entity<EditorState>,
    scroll: ScrollHandle,
    frame_started: Option<Instant>,
    pub frames: Vec<Duration>,
}

impl Probe {
    pub fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| EditorState::new(window, cx).with_text(text));
        editor.update(cx, |editor, cx| editor.set_markdown_style(style(), cx));
        window.focus(&editor.read(cx).focus_handle(cx), cx);
        Self { editor, scroll: ScrollHandle::new(), frame_started: None, frames: Vec::new() }
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame_started = Some(Instant::now());
        let entity = cx.entity();
        let stopwatch = canvas(
            |_, _, _| (),
            move |_, _, _, cx| {
                entity.update(cx, |this, _| {
                    if let Some(started) = this.frame_started.take() {
                        this.frames.push(started.elapsed());
                    }
                })
            },
        )
        .absolute()
        .size_0();
        div()
            .relative()
            .size_full()
            .bg(rgb(0x0a0a0a))
            .text_color(rgb(0xe6e6e6))
            .font_family("Inter")
            .text_size(px(15.))
            .child(div().id("scroll").size_full().overflow_y_scroll().track_scroll(&self.scroll).px(px(32.)).py(px(16.)).child(self.editor.clone()))
            .child(stopwatch)
    }
}
