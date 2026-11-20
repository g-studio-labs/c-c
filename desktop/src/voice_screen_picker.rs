//! Qué pantalla o ventana compartir, cuando hay más de una.
//!
//! El `ScreenPicker.tsx` de Tauri, con las ventanas además de las pantallas
//! (allí sólo se elegía pantalla). Sólo sale donde elige la app —X11 y, más
//! adelante, Windows y macOS—: en Wayland pregunta el diálogo del sistema y
//! el core ni lo abre (`ScreenSources::without_asking`). Qué hay lo dice el
//! core (`VoiceState::picking`); aquí sólo se pinta y se le pasa lo elegido.
//! Escape y la equis lo cierran sin compartir nada.

use cac_core::{ScreenSource, ScreenSources};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Una fila: pulsarla comparte esa fuente.
fn row(source: &ScreenSource, shown: String, icon: Lucide, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (border, accent, muted) = (theme.border, theme.accent, theme.muted_foreground);
    let picked = source.clone();
    div()
        .id(SharedString::from(format!("voice-pick-{}", source.id)))
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .rounded(px(6.))
        .border_1()
        .border_color(border)
        .px_3()
        .py_2()
        .text_sm()
        .hover(move |style| style.bg(accent))
        .cursor_pointer()
        .on_click(move |_, _, cx| Backend::core(cx).voice().pick_screen(picked.clone()))
        .aria_label(shown.clone())
        .child(Icon::new(icon).size_4().flex_none().text_color(muted))
        .child(div().min_w_0().truncate().child(shown))
        .test_support()
        .into_any_element()
}

fn group(title: Option<String>, rows: Vec<AnyElement>, cx: &App) -> Option<Div> {
    if rows.is_empty() {
        return None;
    }
    let muted = cx.theme().muted_foreground;
    Some(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(title.map(|title| div().text_size(px(11.)).text_color(muted).child(title.to_uppercase())))
            .children(rows),
    )
}

/// El selector, sobre toda la ventana. `focus` es su foco: con él, Escape lo
/// cierra; quien lo pone se lo da al abrirlo.
pub fn screen_picker(sources: &ScreenSources, focus: &FocusHandle, window: &mut Window, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (background, foreground, muted) = (theme.background, theme.foreground, theme.muted_foreground);
    let lang = locale(cx);
    let with_windows = !sources.windows.is_empty();
    let (title, lead) = if with_windows {
        (tr(cx, "common:voice.pickSourceTitle"), tr(cx, "common:voice.pickSourceLead"))
    } else {
        (tr(cx, "common:voice.pickScreenTitle"), tr(cx, "common:voice.pickScreenLead"))
    };
    // Las pantallas, siempre por su número: el título que da el capturador
    // no es un nombre (en X11 todas se llaman «screen»).
    let screens: Vec<AnyElement> = sources
        .screens
        .iter()
        .enumerate()
        .map(|(index, screen)| row(screen, cac_core::text_with(lang, "common:voice.screenN", &[("n", &(index + 1).to_string())]), Lucide::Monitor, cx))
        .collect();
    let windows: Vec<AnyElement> = sources.windows.iter().map(|one| row(one, one.title.clone(), Lucide::AppWindow, cx)).collect();
    // Con un solo grupo, el título del diálogo ya dice qué es.
    let heading = |key: &str| with_windows.then(|| tr(cx, key));
    let card = div()
        .id("voice-picker")
        .test_support()
        .track_focus(focus)
        .on_key_down(|event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" {
                Backend::core(cx).voice().cancel_pick();
            }
            cx.stop_propagation();
        })
        .relative()
        .w_full()
        .max_w(px(448.))
        .max_h(relative(0.8))
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .rounded(px(14.))
        .border_1()
        .border_color(foreground.opacity(0.1))
        .bg(background)
        .shadow_2xl()
        .leading_sm()
        .text_color(foreground)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .pr_6()
                .child(div().id("voice-picker-title").aria_label(title.clone()).text_size(rems(1.)).line_height(rems(1.25)).font_weight(FontWeight::MEDIUM).child(title).test_support())
                .child(div().text_color(muted).child(lead)),
        )
        .child(
            div()
                .id("voice-picker-sources")
                .min_h_0()
                .flex()
                .flex_col()
                .gap_4()
                .overflow_y_scroll()
                .children(group(heading("common:voice.screens"), screens, cx))
                .children(group(heading("common:voice.windows"), windows, cx)),
        )
        .child(
            div().absolute().top_2().right_2().child(
                Button::new("voice-picker-close")
                    .ghost()
                    .xsmall()
                    .size_7()
                    .p_0()
                    .child(Icon::new(Lucide::X).size_4())
                    .on_click(|_, _, cx| Backend::core(cx).voice().cancel_pick()),
            ),
        );
    let size = window.viewport_size();
    deferred(
        anchored()
            .position(Point::default())
            .child(div().id("voice-picker-veil").w(size.width).h(size.height).occlude().flex().items_center().justify_center().p_4().bg(gpui_kit::black().opacity(0.35)).child(card)),
    )
    .with_priority(2)
    .into_any_element()
}
