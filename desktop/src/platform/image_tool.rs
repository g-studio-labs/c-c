//! El compresor de imágenes de las DevTools (`app/src/pages/ImageTool.tsx`).
//! Comprimir, nombrar el resultado y decir cuánto cambió es de
//! `cac_ops::image_tool`; aquí se eligen los ficheros y se guardan.

use std::path::PathBuf;
use std::sync::Arc;

use cac_ops::image_tool::{compress_file, format_size, size_change, Compressed, OUTPUT_FORMATS};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::lang::{locale, tr};
use crate::theme::Leading as _;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Saving {
    Idle,
    Busy,
    Done,
}

struct Item {
    id: u64,
    result: Compressed,
    /// Lo que se pinta en la tabla; `None` si esta app no sabe pintar ese
    /// formato (AVIF).
    preview: Option<Arc<Image>>,
    saving: Saving,
}

pub struct ImageToolView {
    format: &'static str,
    quality: u8,
    quality_slider: Entity<SliderState>,
    max_width: Entity<InputState>,
    items: Vec<Item>,
    next_id: u64,
    processing: bool,
    error: Option<String>,
    saving_all: Saving,
    _on_quality: Subscription,
}

impl ImageToolView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let quality_slider = cx.new(|_| SliderState::new().min(1.).max(100.).step(1.).default_value(90.));
        let on_quality = cx.subscribe(&quality_slider, |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.quality = value.start().round().clamp(1., 100.) as u8;
                cx.notify();
            }
        });
        let placeholder = tr(cx, "common:devtools.image.original");
        let max_width = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        ImageToolView { format: "webp", quality: 90, quality_slider, max_width, items: Vec::new(), next_id: 0, processing: false, error: None, saving_all: Saving::Idle, _on_quality: on_quality }
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            this.update(cx, |this, cx| this.take(paths, cx)).ok();
        })
        .detach();
    }

    /// Comprime lo elegido fuera del hilo que pinta, con los ajustes de
    /// ahora. Lo nuevo va arriba; lo que falle se dice con su nombre.
    fn take(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let (format, quality) = (self.format, self.quality);
        let max_width = self.max_width.read(cx).value().trim().parse::<u32>().ok().filter(|width| *width > 0);
        self.processing = true;
        self.error = None;
        cx.notify();
        let work = cx.background_executor().spawn(async move { paths.iter().map(|path| compress_file(path, format, quality, max_width)).collect::<Vec<_>>() });
        cx.spawn(async move |this, cx| {
            let results = work.await;
            this.update(cx, |this, cx| {
                this.processing = false;
                let mut fresh = Vec::new();
                for result in results {
                    match result {
                        Ok(result) => {
                            let preview = ImageFormat::from_mime_type(&result.mime).filter(|_| result.format != "avif").map(|format| Arc::new(Image::from_bytes(format, result.data.clone())));
                            this.next_id += 1;
                            fresh.push(Item { id: this.next_id, result, preview, saving: Saving::Idle });
                        }
                        Err(error) => this.error = Some(error),
                    }
                }
                fresh.append(&mut this.items);
                this.items = fresh;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Pregunta dónde y lo escribe. Cancelar no es un fallo.
    fn save(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else { return };
        item.saving = Saving::Busy;
        let (name, data) = (item.result.name.clone(), item.result.data.clone());
        cx.notify();
        let home = std::env::home_dir().unwrap_or_else(std::env::temp_dir);
        let chosen = cx.prompt_for_new_path(&home, Some(&name));
        cx.spawn(async move |this, cx| {
            let outcome = match chosen.await {
                Ok(Ok(Some(target))) => Some(cx.background_executor().spawn(async move { std::fs::write(target, data).map_err(|error| error.to_string()) }).await),
                _ => None,
            };
            this.update(cx, |this, cx| {
                let language = locale(cx);
                if let Some(item) = this.items.iter_mut().find(|item| item.id == id) {
                    item.saving = if matches!(outcome, Some(Ok(()))) { Saving::Done } else { Saving::Idle };
                }
                if let Some(Err(error)) = outcome {
                    this.error = Some(cac_core::text_with(language, "common:devtools.image.saveFailed", &[("name", &name), ("error", &error)]));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Una carpeta y todos dentro, cada uno con su nombre.
    fn save_all(&mut self, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        self.saving_all = Saving::Busy;
        cx.notify();
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        let files: Vec<(u64, String, Vec<u8>)> = self.items.iter().map(|item| (item.id, item.result.name.clone(), item.result.data.clone())).collect();
        cx.spawn(async move |this, cx| {
            let folder = match chosen.await {
                Ok(Ok(Some(mut folders))) if !folders.is_empty() => folders.remove(0),
                _ => {
                    this.update(cx, |this, cx| {
                        this.saving_all = Saving::Idle;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let written = cx
                .background_executor()
                .spawn(async move { files.into_iter().map(|(id, name, data)| (id, name.clone(), std::fs::write(folder.join(&name), data).map_err(|error| error.to_string()))).collect::<Vec<_>>() })
                .await;
            this.update(cx, |this, cx| {
                let language = locale(cx);
                let mut any = false;
                for (id, name, result) in written {
                    let ok = result.is_ok();
                    any |= ok;
                    if let Some(item) = this.items.iter_mut().find(|item| item.id == id) {
                        item.saving = if ok { Saving::Done } else { Saving::Idle };
                    }
                    if let Err(error) = result {
                        this.error = Some(cac_core::text_with(language, "common:devtools.image.saveFailed", &[("name", &name), ("error", &error)]));
                    }
                }
                this.saving_all = if any { Saving::Done } else { Saving::Idle };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn card(id: &'static str, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        div().id(id).flex().flex_col().gap_3().p_4().rounded(px(10.)).border_1().border_color(theme.border).bg(theme.background)
    }

    fn results(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.items.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let (muted, border, success, danger, mono, secondary, secondary_foreground) = (theme.muted_foreground, theme.border, theme.success, theme.danger, theme.mono_font_family.clone(), theme.secondary, theme.secondary_foreground);
        let original: usize = self.items.iter().map(|item| item.result.original_bytes).sum();
        let compressed: usize = self.items.iter().map(|item| item.result.compressed_bytes).sum();
        // El lote entero puede salir más grande, y decirlo en verde con un
        // menos delante sería mentir en lo único que la herramienta mide.
        let total = div()
            .flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .rounded(px(4.))
            .bg(secondary)
            .text_color(secondary_foreground)
            .font_family(mono.clone())
            .text_xs()
            .child(format_size(original))
            .child(Icon::new(Lucide::ArrowRight).size_3())
            .child(format_size(compressed))
            .child(div().id("image-total-change").text_color(if compressed <= original { success } else { danger }).child(format!("({})", size_change(original, compressed))).test_support());
        let saving_all = self.saving_all;
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .font_weight(FontWeight::MEDIUM)
                    .child(Icon::new(Lucide::Image).size_4())
                    .child(cac_core::text_with(locale(cx), "common:devtools.image.results", &[("count", &self.items.len().to_string())]))
                    .child(total),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("image-download-all")
                            .outline()
                            .small()
                            .icon(Icon::new(if saving_all == Saving::Done { Lucide::Check } else { Lucide::Download }))
                            .label(tr(cx, match saving_all {
                                Saving::Busy => "common:devtools.image.saving",
                                Saving::Done => "common:devtools.image.saved",
                                Saving::Idle => "common:devtools.image.downloadAll",
                            }))
                            .disabled(saving_all == Saving::Busy)
                            .on_click(cx.listener(|this, _, _, cx| this.save_all(cx))),
                    )
                    .child(Button::new("image-clear").ghost().small().icon(Icon::new(Lucide::Trash)).label(tr(cx, "common:devtools.image.clear")).on_click(cx.listener(|this, _, _, cx| {
                        this.items.clear();
                        this.saving_all = Saving::Idle;
                        cx.notify();
                    }))),
            );
        let columns: [(&str, f32); 7] = [
            ("common:devtools.image.preview", 0.7),
            ("common:devtools.image.name", 3.),
            ("common:devtools.image.dimensions", 1.5),
            ("common:devtools.image.original", 1.2),
            ("common:devtools.image.compressed", 1.2),
            ("common:devtools.image.change", 1.),
            ("common:devtools.image.actions", 1.),
        ];
        let row = |cells: Vec<AnyElement>| div().flex().items_center().gap_3().px_2().children(cells.into_iter().zip(columns).map(|(cell, (_, share))| div().flex_grow(share).flex_basis(px(0.)).min_w_0().child(cell)));
        let heading = row(columns.iter().map(|(key, _)| div().text_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, key)).into_any_element()).collect()).h(px(32.));
        let rows: Vec<AnyElement> = self
            .items
            .iter()
            .map(|item| {
                let result = &item.result;
                let smaller = result.compressed_bytes < result.original_bytes;
                let preview = match &item.preview {
                    Some(image) => img(image.clone()).size_8().rounded(px(4.)).object_fit(ObjectFit::Cover).into_any_element(),
                    None => div().size_8().flex().items_center().justify_center().child(Icon::new(Lucide::Image).size_4().text_color(muted)).into_any_element(),
                };
                let id = item.id;
                let saving = item.saving;
                let actions = div()
                    .flex()
                    .justify_end()
                    .gap_0p5()
                    .child(
                        Button::new(SharedString::from(format!("image-download-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(if saving == Saving::Done { Lucide::Check } else { Lucide::Download }))
                            .tooltip(tr(cx, "common:devtools.image.download"))
                            .disabled(saving == Saving::Busy)
                            .on_click(cx.listener(move |this, _, _, cx| this.save(id, cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("image-remove-{id}")))
                            .ghost()
                            .xsmall()
                            .text_color(danger)
                            .icon(Icon::new(Lucide::Trash))
                            .tooltip(tr(cx, "common:devtools.image.remove"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.items.retain(|item| item.id != id);
                                cx.notify();
                            })),
                    );
                row(vec![
                    preview,
                    div().font_family(mono.clone()).text_xs().truncate().child(result.name.clone()).into_any_element(),
                    div().text_xs().text_color(muted).child(format!("{} x {}", result.width, result.height)).into_any_element(),
                    div().text_xs().text_color(muted).child(format_size(result.original_bytes)).into_any_element(),
                    div().text_xs().text_color(muted).child(format_size(result.compressed_bytes)).into_any_element(),
                    super::services::pill(size_change(result.original_bytes, result.compressed_bytes), if smaller { success.opacity(0.15) } else { danger.opacity(0.15) }, if smaller { success } else { danger }).into_any_element(),
                    actions.into_any_element(),
                ])
                .id(SharedString::from(format!("image-row-{id}")))
                .h(px(44.))
                .border_t_1()
                .border_color(border)
                .test_support()
                .into_any_element()
            })
            .collect();
        Some(Self::card("image-results", cx).child(header).child(div().flex().flex_col().child(heading).children(rows)).into_any_element())
    }
}

impl Render for ImageToolView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, primary, accent, danger) = (theme.muted_foreground, theme.border, theme.primary, theme.accent, theme.danger);
        let language = locale(cx);
        let formats = div().flex().flex_wrap().gap_1p5().children(OUTPUT_FORMATS.iter().map(|(value, label)| {
            let chosen = self.format == *value;
            let value: &'static str = value;
            Button::new(SharedString::from(format!("image-format-{value}")))
                .xsmall()
                .rounded_full()
                .when(chosen, |chip| chip.primary())
                .when(!chosen, |chip| chip.ghost())
                .label(*label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.format = value;
                    cx.notify();
                }))
        }));
        let field = |label: String, body: AnyElement| div().flex_1().min_w_0().flex().flex_col().gap_1p5().child(div().text_xs().font_weight(FontWeight::MEDIUM).child(label)).child(body);
        let settings = Self::card("image-settings", cx).child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:devtools.image.settings"))).child(
            div()
                .flex()
                .gap_4()
                .child(field(tr(cx, "common:devtools.image.format"), formats.into_any_element()))
                .child(field(
                    cac_core::text_with(language, "common:devtools.image.quality", &[("value", &self.quality.to_string())]),
                    div().h(px(36.)).flex().items_center().child(Slider::new(&self.quality_slider)).into_any_element(),
                ))
                .child(field(tr(cx, "common:devtools.image.maxWidth"), div().id("image-max-width").child(Input::new(&self.max_width).small()).into_any_element())),
        );
        let drop = Self::card("image-drop-card", cx).child(
            div()
                .id("image-drop")
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .p_10()
                .rounded(px(8.))
                .border_2()
                .border_dashed()
                .border_color(border)
                .cursor_pointer()
                .hover(move |style| style.border_color(primary).bg(accent.opacity(0.3)))
                .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(primary))
                .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.take(paths.paths().to_vec(), cx)))
                .on_click(cx.listener(|this, _, window, cx| this.choose(window, cx)))
                .child(Icon::new(Lucide::Upload).size_8().text_color(muted))
                .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, if self.processing { "common:devtools.image.processing" } else { "common:devtools.image.drop" })))
                .child(div().text_xs().text_color(muted).child(tr(cx, "common:devtools.image.accepted")))
                .test_support(),
        )
        .children(self.error.clone().map(|error| div().id("image-error").text_color(danger).child(error).test_support()));
        let results = self.results(cx);
        div().id("image-tool").size_full().overflow_y_scroll().child(
            div()
                .mx_auto()
                .w_full()
                .max_w(px(1024.))
                .p_6()
                .flex()
                .flex_col()
                .gap_4()
                .leading_sm()
                .child(div().flex().items_center().gap_3().text_size(rems(1.25)).font_weight(FontWeight::SEMIBOLD).child(Icon::new(Lucide::ImageDown).size_6().text_color(muted)).child(tr(cx, "common:devtools.image.title")))
                .child(settings)
                .child(drop)
                .children(results),
        )
    }
}
