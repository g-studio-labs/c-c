//! Los ajustes de audio de la sala: elegir micrófono y altavoz, la
//! supresión de ruido y el medidor del micrófono.
//!
//! Como `DeviceSettings.tsx` y `MicLevel.tsx`, con una cosa más: aquí se
//! elige también el altavoz, que en Tauri no se podía. Qué se elige se
//! recuerda en el core (`Voice::choose_device`); quién lo abre es el motor
//! (`voice_engine.rs`), que sólo hay en Linux por ahora. Sin motor no hay
//! botón de ajustes.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::tr;
use crate::theme::Leading as _;
use crate::workspace::Workspace;

/// Cada cuánto se mira el micrófono con el panel abierto: el de Tauri.
const LEVEL_EVERY: std::time::Duration = std::time::Duration::from_millis(120);

/// Un dispositivo, como lo pinta el panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceRow {
    pub id: String,
    pub name: String,
    pub current: bool,
}

/// Lo que hay, por sistema: micrófonos y altavoces.
#[derive(Debug, Clone, Default)]
pub(crate) struct DeviceLists {
    pub inputs: Vec<DeviceRow>,
    pub outputs: Vec<DeviceRow>,
}

/// Lo que el panel sabe mientras está abierto.
#[derive(Default)]
pub(crate) struct SettingsState {
    pub open: bool,
    pub lists: Option<DeviceLists>,
    /// El que se está abriendo ahora.
    pub busy: Option<String>,
    pub error: Option<String>,
    /// El pico del micrófono, con caída lenta: sin ella la barra parpadea.
    pub level: f32,
    pub input: Option<String>,
    pub mic_error: Option<String>,
    /// Cuántas veces se ha abierto: el reloj del medidor de una apertura
    /// anterior se para solo.
    pub turn: u64,
}

/// Si esta app tiene motor de voz (hoy, sólo en Linux).
pub(crate) fn available(cx: &App) -> bool {
    #[cfg(target_os = "linux")]
    {
        cx.has_global::<crate::voice_engine::Devices>()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cx;
        false
    }
}

#[cfg(target_os = "linux")]
fn engine(cx: &App) -> Option<cac_voice::Engine> {
    cx.try_global::<crate::voice_engine::Devices>().map(|devices| devices.0.clone())
}

#[cfg(target_os = "linux")]
fn rows(devices: &[cac_voice::Device]) -> Vec<DeviceRow> {
    devices.iter().map(|device| DeviceRow { id: device.id.clone(), name: device.name.clone(), current: device.current }).collect()
}

/// Lo que se eligió en esta máquina, al motor: al arrancar, antes de entrar
/// a ninguna sala.
#[cfg(target_os = "linux")]
pub(crate) fn restore_choices(cx: &App) {
    let Some(engine) = engine(cx) else { return };
    Backend::run(cx, move |core| async move {
        let choices = core.voice().device_choices();
        let _ = engine.set_device(cac_voice::DeviceKind::Input, choices.input).await;
        let _ = engine.set_device(cac_voice::DeviceKind::Output, choices.output).await;
    });
}

impl Workspace {
    pub(crate) fn toggle_voice_settings(&mut self, cx: &mut Context<Self>) {
        self.voice_settings.open = !self.voice_settings.open;
        if self.voice_settings.open {
            self.voice_settings.turn += 1;
            self.voice_settings.error = None;
            self.load_devices(cx);
            self.watch_level(cx);
        }
        cx.notify();
    }

    /// La lista se vuelve a pedir cada vez que se abre: un micrófono que se
    /// enchufa con la llamada en marcha tiene que salir.
    fn load_devices(&mut self, cx: &mut Context<Self>) {
        #[cfg(target_os = "linux")]
        {
            let Some(engine) = engine(cx) else { return };
            let found = cx.background_executor().spawn(async move { engine.devices() });
            cx.spawn(async move |this, cx| {
                let devices = found.await;
                let _ = this.update(cx, |this, cx| {
                    this.voice_settings.lists = Some(DeviceLists { inputs: rows(&devices.inputs), outputs: rows(&devices.outputs) });
                    cx.notify();
                });
            })
            .detach();
        }
        #[cfg(not(target_os = "linux"))]
        let _ = cx;
    }

    /// El medidor, mientras el panel está abierto.
    fn watch_level(&mut self, cx: &mut Context<Self>) {
        #[cfg(target_os = "linux")]
        {
            let Some(engine) = engine(cx) else { return };
            let turn = self.voice_settings.turn;
            cx.spawn(async move |this, cx| loop {
                let level = engine.current_call().map(|call| call.mic_level());
                let still = this
                    .update(cx, |this, cx| {
                        let settings = &mut this.voice_settings;
                        if !settings.open || settings.turn != turn {
                            return false;
                        }
                        if let Some(level) = &level {
                            let now = level.peak_permille as f32 / 1000.0;
                            settings.level = now.max(settings.level * 0.82);
                            settings.input = level.input.as_ref().map(|input| format!("{} · {} Hz · {}", input.device, input.rate_hz, if input.channels == 1 { "mono".to_string() } else { format!("{} ch", input.channels) }));
                            settings.mic_error = level.error.clone();
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !still {
                    break;
                }
                cx.background_executor().timer(LEVEL_EVERY).await;
            })
            .detach();
        }
        #[cfg(not(target_os = "linux"))]
        let _ = cx;
    }

    /// Elige uno: se recuerda, y el motor lo abre (en caliente si hay
    /// llamada). Si no abre, la llamada sigue con el que tenía y se dice.
    fn choose_device(&mut self, output: bool, id: String, cx: &mut Context<Self>) {
        Backend::core(cx).voice().choose_device(output, Some(id.clone()));
        #[cfg(target_os = "linux")]
        {
            let Some(engine) = engine(cx) else { return };
            self.voice_settings.busy = Some(id.clone());
            cx.notify();
            let kind = if output { cac_voice::DeviceKind::Output } else { cac_voice::DeviceKind::Input };
            let answer = Backend::ask(cx, move |_| async move { engine.set_device(kind, Some(id)).await.map_err(|error| error.label().to_string()) });
            cx.spawn(async move |this, cx| {
                let result = answer.await.unwrap_or(Ok(()));
                let _ = this.update(cx, |this, cx| {
                    this.voice_settings.busy = None;
                    if let Err(code) = result {
                        this.voice_settings.error = Some(cac_core::voice_error_phrase(&cac_core::VoiceError { message: code.clone(), code, space_id: None }, crate::lang::locale(cx)));
                    }
                    this.load_devices(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// El panel, encima de la barra de mandos.
    pub(crate) fn render_voice_settings(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.voice_settings.open {
            return None;
        }
        let theme = cx.theme();
        let (popover, border, muted, danger, success, foreground, primary, background) =
            (theme.popover, theme.border, theme.muted_foreground, theme.danger, theme.success, theme.foreground, theme.primary, theme.background);
        let settings = &self.voice_settings;
        let noise = Backend::snapshot(cx).read(cx).voice.noise_suppression;
        let group_title = |icon: Lucide, title: String| {
            div().flex().items_center().gap_1p5().px_3().pb_1().text_size(px(11.)).text_color(muted).child(Icon::new(icon).size_3()).child(title.to_uppercase())
        };
        let row = |output: bool, device: &DeviceRow, busy: bool, cx: &mut Context<Self>| {
            let id = device.id.clone();
            let current = device.current;
            div()
                .id(SharedString::from(format!("voice-device-{}-{}", if output { "out" } else { "in" }, device.id)))
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(4.))
                .px_3()
                .py_1p5()
                .text_size(px(13.))
                .text_color(if current { foreground } else { muted })
                .when(!current && !busy, |this| {
                    this.cursor_pointer().hover(move |style| style.text_color(foreground)).on_click(cx.listener(move |this, _, _, cx| this.choose_device(output, id.clone(), cx)))
                })
                .aria_label(device.name.clone())
                .aria_selected(current)
                .child(div().w(px(14.)).flex_none().map(|this| {
                    if busy {
                        this.child(Spinner::new().color(muted).with_size(px(14.)))
                    } else if current {
                        this.child(Icon::new(Lucide::Check).size_3p5().text_color(success))
                    } else {
                        this
                    }
                }))
                .child(div().min_w_0().truncate().child(device.name.clone()))
                .test_support()
        };
        let body: AnyElement = match &settings.lists {
            None => div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_4()
                .leading_xs()
                .text_color(muted)
                .child(Spinner::new().color(muted).with_size(px(14.)))
                .child(tr(cx, "common:last.lookingForDevices"))
                .into_any_element(),
            Some(lists) => {
                let width = (settings.level.sqrt() * 100.).clamp(2., 100.);
                let meter = div()
                    .px_3()
                    .pb_2()
                    .pt_1()
                    .child(
                        div()
                            .id("voice-mic-level")
                            .h_1p5()
                            .w_full()
                            .overflow_hidden()
                            .rounded_full()
                            .bg(muted.opacity(0.2))
                            .aria_label(tr(cx, "common:last.micLevel"))
                            .child(div().h_full().rounded_full().w(relative(width / 100.)).bg(if width > 2. { success } else { muted.opacity(0.3) }))
                            .test_support(),
                    )
                    .children(settings.input.clone().map(|input| div().mt_1().truncate().text_size(px(11.)).text_color(muted).child(input)))
                    .children(settings.mic_error.clone().map(|error| div().mt(px(2.)).text_size(px(11.)).text_color(danger).child(error)));
                let inputs: Vec<AnyElement> = lists.inputs.iter().map(|device| row(false, device, settings.busy.as_deref() == Some(device.id.as_str()), cx).into_any_element()).collect();
                let outputs: Vec<AnyElement> = lists.outputs.iter().map(|device| row(true, device, settings.busy.as_deref() == Some(device.id.as_str()), cx).into_any_element()).collect();
                let empty = |key: &str, cx: &App| div().px_3().py_1().leading_xs().text_color(muted).child(tr(cx, key));
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .py_1()
                            .child(group_title(Lucide::Mic, tr(cx, "common:last.microphone")))
                            .child(meter)
                            .when(inputs.is_empty(), |this| this.child(empty("common:last.noMicrophone", cx)))
                            .children(inputs),
                    )
                    .child(
                        div()
                            .id("voice-noise")
                            .flex()
                            .items_start()
                            .gap_2()
                            .rounded(px(4.))
                            .px_3()
                            .py_2()
                            .leading_xs()
                            .cursor_pointer()
                            .on_click(move |_, _, cx| Backend::core(cx).voice().set_noise_suppression(!noise))
                            .aria_label(tr(cx, "common:last.noiseSuppression"))
                            .aria_selected(noise)
                            .child(Icon::new(Lucide::AudioLines).size_3p5().mt(px(2.)).flex_none().text_color(muted))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .child(div().text_color(foreground).child(tr(cx, "common:last.noiseSuppression")))
                                    .child(div().text_color(muted).child(tr(cx, "common:last.noiseSuppressionHint"))),
                            )
                            .child(
                                div()
                                    .mt(px(2.))
                                    .h_4()
                                    .w_7()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .rounded_full()
                                    .p(px(2.))
                                    .bg(if noise { primary } else { muted.opacity(0.3) })
                                    .when(noise, |this| this.justify_end())
                                    .child(div().size_3().rounded_full().bg(background)),
                            )
                            .test_support(),
                    )
                    .child(
                        div()
                            .py_1()
                            .child(group_title(Lucide::Volume2, tr(cx, "common:last.speaker")))
                            .when(outputs.is_empty(), |this| this.child(empty("common:last.noSpeaker", cx)))
                            .children(outputs),
                    )
                    .into_any_element()
            }
        };
        Some(
            div()
                .absolute()
                .bottom(px(96.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("voice-settings")
                        .occlude()
                        .w(px(288.))
                        .max_h(px(420.))
                        .overflow_y_scroll()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(border)
                        .bg(popover)
                        .p_1()
                        .shadow_lg()
                        .children(settings.error.clone().map(|error| div().id("voice-settings-error").px_3().py_2().leading_xs().text_color(danger).aria_label(error.clone()).child(error).test_support()))
                        .child(body)
                        .test_support(),
                )
                .into_any_element(),
        )
    }
}
