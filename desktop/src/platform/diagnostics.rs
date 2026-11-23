//! Diagnóstico (`app/src/pages/Diagnostics.tsx` y
//! `components/diagnostics/*`): a la izquierda los dispositivos; a la derecha
//! la ficha del elegido (lo que va mal, sus latidos, su estado) y su
//! timeline. Las reglas —qué es un hueco, en qué orden va el timeline, cómo se
//! despliega un JSON— son del core (`cac_core::platform::diagnostics`).

use std::time::Duration;

use cac_core::{crumb_types, flatten_timeline, gap_duration, heartbeat_strip, kv_rows, matches_text, short_id, text_count, text_with, time_ago, DeviceDetail, DeviceSummary, KvKind, KvRow, StripKind, TimelineFilters, TimelineRow};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Cuánto se espera a que se deje de teclear antes de buscar: una petición
/// por letra sería pedir la lista entera diez veces para escribir un nombre.
const SEARCH_PAUSE: Duration = Duration::from_millis(250);

pub struct DiagnosticsView {
    search: Entity<InputState>,
    text: Entity<InputState>,
    /// Las ramas de la ficha que se han pulsado.
    sheet_toggled: Vec<String>,
    /// Los eventos abiertos, y las ramas pulsadas dentro de cada uno.
    open_rows: Vec<String>,
    row_toggled: Vec<String>,
    /// Lo último que se mandó a buscar: cada letra lo cambia, y sólo la
    /// búsqueda de la última sale.
    typed: u64,
    _subscriptions: [Subscription; 2],
}

impl DiagnosticsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "diagnostics:search");
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let placeholder = tr(cx, "diagnostics:timeline.filterText");
        let text = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let on_search = cx.subscribe_in(&search, window, |this, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.typed += 1;
                let typed = this.typed;
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(SEARCH_PAUSE).await;
                    let Ok(still) = this.update(cx, |this, _| this.typed == typed) else { return };
                    if still {
                        cx.update(|_, cx| Backend::run(cx, move |core| async move { core.platform().search_devices(&query).await })).ok();
                    }
                })
                .detach();
            }
        });
        let on_text = cx.subscribe_in(&text, window, |_, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                let mut filters = Backend::snapshot(cx).read(cx).platform.diagnostics.clone().unwrap_or_default().filters;
                filters.text = text;
                Backend::run(cx, move |core| async move { core.platform().set_timeline_filters(filters).await });
            }
        });
        DiagnosticsView { search, text, sheet_toggled: Vec::new(), open_rows: Vec::new(), row_toggled: Vec::new(), typed: 0, _subscriptions: [on_search, on_text] }
    }

    fn device_row(&self, device: &DeviceSummary, active: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, accent, primary, danger, warning, mono) = (theme.muted_foreground, theme.border, theme.accent, theme.primary, theme.danger, theme.warning, theme.mono_font_family.clone());
        let language = locale(cx);
        let worst = device.worst();
        let reference = device.reference();
        let silent = device.silent_since.as_ref().map(|since| text_with(language, "diagnostics:silentSince", &[("when", &time_ago(since, &chrono::Local::now(), language))]));
        let alerts = device.alerts.iter().map(|alert| alert.message.clone()).collect::<Vec<_>>().join(" · ");
        let id = format!("diag-device-{}-{}", device.project_id, device.device_id);
        div()
            .id(SharedString::from(id))
            .w_full()
            .flex()
            .flex_col()
            .gap_0p5()
            .p_2()
            .rounded(px(6.))
            .border_1()
            .border_color(if active { primary } else if worst == Some("error") { danger.opacity(0.5) } else { border })
            .when(active, |row| row.bg(accent))
            .cursor_pointer()
            .hover(move |style| style.bg(accent))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sheet_toggled.clear();
                this.open_rows.clear();
                this.row_toggled.clear();
                let reference = reference.clone();
                Backend::run(cx, move |core| async move { core.platform().select_device(reference).await });
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(Icon::new(Lucide::Smartphone).size_3p5().text_color(muted))
                    .child(div().flex_1().min_w_0().truncate().when(device.label.trim().is_empty(), |title| title.font_family(mono.clone()).text_xs()).child(device.title()))
                    .children(silent.map(|silent| div().id("diag-silent").child(Icon::new(Lucide::ZapOff).size_3p5().text_color(danger)).tooltip(move |window, cx| Tooltip::new(silent.clone()).build(window, cx))))
                    .children(worst.map(|worst| {
                        let tip = alerts.clone();
                        div().id("diag-worst").child(Icon::new(Lucide::TriangleAlert).size_3p5().text_color(if worst == "error" { danger } else { warning })).tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    })),
            )
            .when(!device.subject.is_empty(), |row| row.child(div().truncate().text_xs().child(device.subject.clone())))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .text_color(muted)
                    .child(div().truncate().child(device.project_name.clone()))
                    .child("·")
                    .child(if device.platform.is_empty() { "?".to_string() } else { device.platform.clone() })
                    .when(!device.app_version.is_empty(), |line| line.child(div().truncate().child(format!("· v{}", device.app_version)))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(muted)
                    .when(device.error_count > 0, |line| line.child(super::services::pill(text_count(language, "diagnostics:errors", device.error_count), danger, gpui_kit::white())))
                    .when(device.warn_count > 0, |line| line.child(super::services::pill(text_count(language, "diagnostics:warnings", device.warn_count), warning.opacity(0.15), warning)))
                    .child(div().ml_auto().child(text_with(language, "diagnostics:lastSeen", &[("when", &time_ago(&device.last_seen, &chrono::Local::now(), language))]))),
            )
            .when(!device.alerts.is_empty(), |row| {
                row.child(div().flex().flex_col().text_xs().children(device.alerts.iter().take(3).map(|alert| div().text_color(if alert.is_error() { danger } else { warning }).child(alert.message.clone()))))
            })
            .test_support()
            .into_any_element()
    }

    fn device_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.diagnostics.clone().unwrap_or_default();
        let theme = cx.theme();
        let (muted, border, danger) = (theme.muted_foreground, theme.border, theme.danger);
        let selected = state.selected.clone();
        let note = if let Some(error) = state.error.clone() {
            Some(div().id("diag-devices-error").flex().items_center().gap_1p5().px_2().py_2().text_xs().text_color(danger).child(Icon::new(Lucide::CircleAlert).size_3()).child(error).test_support().into_any_element())
        } else if state.devices.is_empty() {
            let key = if state.loading_devices || !state.loaded {
                "diagnostics:loading"
            } else if state.query.trim().is_empty() {
                "diagnostics:noDevices"
            } else {
                "diagnostics:noMatches"
            };
            Some(div().id("diag-devices-empty").px_2().py_2().text_xs().text_color(muted).child(tr(cx, key)).test_support().into_any_element())
        } else {
            None
        };
        let rows: Vec<AnyElement> = state.devices.iter().map(|device| self.device_row(device, selected.as_ref() == Some(&device.reference()), cx)).collect();
        let more = state.devices_cursor.is_some().then(|| {
            Button::new("diag-devices-more")
                .ghost()
                .small()
                .w_full()
                .label(tr(cx, "diagnostics:loadMore"))
                .disabled(state.loading_devices)
                .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.platform().load_devices(true).await }))
        });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(div().flex_none().p_2().border_b_1().border_color(border).child(div().id("diag-search").child(Input::new(&self.search).small().prefix(Icon::new(Lucide::Search).size_3p5().text_color(muted))).test_support()))
            .child(div().id("diag-devices").flex_1().min_h_0().overflow_y_scroll().p_2().flex().flex_col().gap_1p5().children(note).children(rows).children(more).test_support())
    }

    fn sheet(&self, detail: &DeviceDetail, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger, warning, success, border, mono, panel) = (theme.muted_foreground, theme.danger, theme.warning, theme.success, theme.border, theme.mono_font_family.clone(), theme.muted);
        let language = locale(cx);
        let now = chrono::Local::now();
        let summary = &detail.summary;
        let byline = [summary.subject.clone(), summary.project_name.clone(), summary.platform.clone(), if summary.app_version.is_empty() { String::new() } else { format!("v{}", summary.app_version) }]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let seen = format!(
            "{} · {}",
            text_with(language, "diagnostics:lastSeen", &[("when", &time_ago(&summary.last_seen, &now, language))]),
            text_with(language, "diagnostics:sheet.firstSeen", &[("when", &time_ago(&detail.first_seen, &now, language))])
        );
        let alerts = if summary.alerts.is_empty() {
            div().id("diag-healthy").flex().items_center().gap_1p5().text_xs().text_color(muted).child(Icon::new(Lucide::CircleCheck).size_3p5().text_color(success)).child(tr(cx, "diagnostics:sheet.healthy")).test_support().into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .children(summary.alerts.iter().enumerate().map(|(index, alert)| {
                    let tone = if alert.is_error() { danger } else { warning };
                    div()
                        .id(SharedString::from(format!("diag-alert-{index}")))
                        .flex()
                        .items_start()
                        .gap_1p5()
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(tone.opacity(0.4))
                        .bg(tone.opacity(0.1))
                        .text_xs()
                        .text_color(tone)
                        .child(Icon::new(Lucide::TriangleAlert).size_3p5())
                        .child(div().flex_1().child(alert.message.clone()))
                        .child(div().font_family(mono.clone()).opacity(0.7).child(format!("{} = {}", alert.path, alert.actual)))
                        .test_support()
                }))
                .into_any_element()
        };
        let highlight: Vec<(String, String)> = summary.alerts.iter().map(|alert| (alert.path.clone(), if alert.is_error() { "error".into() } else { "warn".into() })).collect();
        let state_tree = if detail.device.is_empty() {
            div().text_xs().text_color(muted).child(tr(cx, "diagnostics:sheet.noSnapshot")).into_any_element()
        } else {
            let rows = kv_rows(&detail.device, &highlight, 1, &self.sheet_toggled);
            div()
                .id("diag-state")
                .max_h(px(320.))
                .overflow_y_scroll()
                .rounded(px(4.))
                .border_1()
                .border_color(border)
                .bg(panel.opacity(0.2))
                .p_2()
                .child(tree(rows, "diag-kv", false, cx))
                .test_support()
                .into_any_element()
        };
        let heading = |key: &str, cx: &App| div().text_xs().font_weight(FontWeight::MEDIUM).text_color(cx.theme().muted_foreground).child(tr(cx, key).to_uppercase());
        div()
            .id("diag-sheet")
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().id("diag-sheet-title").text_size(rems(1.)).font_weight(FontWeight::MEDIUM).child(summary.title()).test_support())
                    .child(div().text_xs().text_color(muted).child(byline))
                    .child(div().font_family(mono).text_size(px(11.)).text_color(muted.opacity(0.7)).child(summary.device_id.clone()))
                    .child(div().text_xs().text_color(muted).child(seen)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(alerts)
                    .children(detail.unhealthy_since.as_ref().map(|since| div().text_xs().text_color(danger).child(text_with(language, "diagnostics:sheet.unhealthySince", &[("when", &time_ago(since, &now, language))])))),
            )
            .child(div().flex().flex_col().gap_1().child(heading("diagnostics:beats.title", cx)).child(strip(detail, cx)))
            .child(div().flex().flex_col().gap_1().child(heading("diagnostics:sheet.title", cx)).child(state_tree))
            .test_support()
            .into_any_element()
    }

    fn timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = Backend::snapshot(cx).read(cx).platform.diagnostics.clone().unwrap_or_default();
        let theme = cx.theme();
        let (muted, border, danger, warning) = (theme.muted_foreground, theme.border, theme.danger, theme.warning);
        let filters = state.filters.clone();
        let rows: Vec<TimelineRow> = flatten_timeline(&state.timeline).into_iter().filter(|row| matches_text(row, &filters.text)).collect();
        // Los tipos que han salido, más el elegido aunque el filtro lo haya
        // hecho desaparecer.
        let mut types = crumb_types(&state.timeline);
        for kind in &filters.types {
            if !types.contains(kind) {
                types.push(kind.clone());
            }
        }
        types.sort();
        let broken = state.timeline.iter().filter(|batch| batch.undecryptable).count();
        let chip = |id: String, label: String, on: bool, next: TimelineFilters| {
            Button::new(SharedString::from(id))
                .xsmall()
                .when(on, |chip| chip.primary())
                .when(!on, |chip| chip.outline())
                .label(label)
                .on_click(move |_, _, cx| {
                    let next = next.clone();
                    Backend::run(cx, move |core| async move { core.platform().set_timeline_filters(next).await });
                })
        };
        let with = |edit: &dyn Fn(&mut TimelineFilters)| {
            let mut next = filters.clone();
            edit(&mut next);
            next
        };
        let severity = [("info", "diagnostics:timeline.all"), ("warn", "diagnostics:timeline.warnUp"), ("error", "diagnostics:timeline.errorOnly")]
            .map(|(value, key)| chip(format!("diag-severity-{value}"), tr(cx, key), filters.min_severity == value, with(&|next| next.min_severity = value.to_string())));
        let language = locale(cx);
        let since = [0u32, 1, 6, 24, 72].map(|hours| {
            let label = if hours == 0 { tr(cx, "diagnostics:timeline.always") } else { text_count(language, "diagnostics:timeline.lastHours", i64::from(hours)) };
            chip(format!("diag-since-{hours}"), label, filters.since_hours == hours, with(&|next| next.since_hours = hours))
        });
        let mut kinds = vec![chip("diag-type-all".into(), tr(cx, "diagnostics:timeline.allTypes"), filters.types.is_empty(), with(&|next| next.types.clear()))];
        kinds.extend(types.iter().map(|kind| {
            let picked = kind.clone();
            chip(format!("diag-type-{kind}"), kind.clone(), filters.types.first() == Some(kind), with(&|next| next.types = vec![picked.clone()]))
        }));
        let session = (!filters.session_id.is_empty()).then(|| chip("diag-session-clear".into(), format!("{} · {}", tr(cx, "diagnostics:timeline.clearSession"), short_id(&filters.session_id)), false, with(&|next| next.session_id.clear())));
        let group = |label_key: &str, chips: Vec<Button>, cx: &App| div().flex().flex_wrap().items_center().gap_1().child(div().text_xs().text_color(cx.theme().muted_foreground).mr_1().child(tr(cx, label_key))).children(chips);

        let body = if state.loading_timeline && state.timeline.is_empty() {
            div().id("diag-timeline-loading").text_xs().text_color(muted).child(tr(cx, "diagnostics:loading")).test_support().into_any_element()
        } else if rows.is_empty() && state.timeline_error.is_none() {
            div().id("diag-timeline-empty").text_xs().text_color(muted).child(tr(cx, "diagnostics:timeline.empty")).test_support().into_any_element()
        } else {
            div()
                .id("diag-timeline")
                .flex()
                .flex_col()
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .children(rows.iter().enumerate().map(|(index, row)| self.timeline_row(row, index > 0, cx)))
                .test_support()
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().w(px(192.)).child(div().id("diag-text").child(Input::new(&self.text).small()).test_support()))
            .child(group("diagnostics:timeline.severity", severity.into_iter().collect(), cx))
            .child(group("diagnostics:timeline.types", kinds, cx))
            .child(group("diagnostics:timeline.since", since.into_iter().collect(), cx))
            .children(session)
            .children(state.timeline_error.clone().map(|error| div().id("diag-timeline-error").flex().items_center().gap_1p5().text_xs().text_color(danger).child(Icon::new(Lucide::CircleAlert).size_3()).child(error).test_support()))
            .when(broken > 0, |column| column.child(div().text_xs().text_color(warning).child(tr(cx, "diagnostics:timeline.undecryptable"))))
            .child(body)
            .children(state.timeline_before.is_some().then(|| {
                Button::new("diag-timeline-more")
                    .ghost()
                    .small()
                    .w_full()
                    .label(tr(cx, "diagnostics:loadMore"))
                    .disabled(state.loading_timeline)
                    .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.platform().load_more_timeline().await }))
            }))
            .into_any_element()
    }

    fn timeline_row(&self, row: &TimelineRow, separated: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, danger, warning, success, accent, panel, mono) = (theme.muted_foreground, theme.border, theme.danger, theme.warning, theme.success, theme.accent, theme.muted, theme.mono_font_family.clone());
        let tone = match row.severity.as_str() {
            "error" => Some(danger),
            "warn" => Some(warning),
            _ => None,
        };
        let open = self.open_rows.contains(&row.key);
        let at = chrono::DateTime::from_timestamp_millis(row.at).map(|at| cac_core::date_and_time(locale(cx), at.with_timezone(&chrono::Local).naive_local())).unwrap_or_default();
        let new_session = row.session_start.then(|| {
            let session = row.session_id.clone();
            let mut next = Backend::snapshot(cx).read(cx).platform.diagnostics.clone().unwrap_or_default().filters;
            next.session_id = session.clone();
            div()
                .id(SharedString::from(format!("diag-session-{}", row.key)))
                .w_full()
                .px_3()
                .py_0p5()
                .border_b_1()
                .border_dashed()
                .border_color(border)
                .bg(panel.opacity(0.3))
                .text_size(px(11.))
                .text_color(muted)
                .cursor_pointer()
                .tooltip({
                    let tip = tr(cx, "diagnostics:timeline.sessionFilter");
                    move |window, cx| Tooltip::new(tip.clone()).build(window, cx)
                })
                .on_click(move |_, _, cx| {
                    let next = next.clone();
                    Backend::run(cx, move |core| async move { core.platform().set_timeline_filters(next).await });
                })
                .child(text_with(locale(cx), "diagnostics:timeline.newSession", &[("id", &short_id(if session.is_empty() { "—" } else { &session }))]))
        });
        let key = row.key.clone();
        let line = div()
            .id(SharedString::from(format!("diag-event-{}", row.key)))
            .aria_label(row.title.clone())
            .w_full()
            .flex()
            .items_start()
            .gap_2()
            .px_3()
            .py_1p5()
            .text_xs()
            .cursor_pointer()
            .hover(move |style| style.bg(accent.opacity(0.4)))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(at) = this.open_rows.iter().position(|open| *open == key) {
                    this.open_rows.remove(at);
                } else {
                    this.open_rows.push(key.clone());
                }
                cx.notify();
            }))
            .child(div().mt_1().size_1p5().flex_none().rounded_full().bg(tone.unwrap_or(muted.opacity(0.5))))
            .child(div().flex_none().text_color(muted).child(at))
            .child(div().flex_none().px_1().rounded(px(3.)).bg(panel).text_size(px(10.)).text_color(muted).child(row.kind.to_uppercase()))
            .child(div().flex_1().min_w_0().truncate().when_some(tone, |title, tone| title.text_color(tone)).child(row.title.clone()))
            .children(row.status.map(|status| div().flex_none().font_family(mono.clone()).text_color(if status == 0 || status >= 400 { danger } else { success }).child(status.to_string())))
            .children(tone.map(|tone| div().flex_none().text_color(tone).child(tr(cx, &format!("diagnostics:severity.{}", row.severity)))))
            .test_support();
        let content = open.then(|| {
            // Lo que el evento ya enseña en su línea no se repite dentro.
            let fields: serde_json::Map<String, serde_json::Value> = serde_json::from_str::<serde_json::Value>(&row.crumb)
                .ok()
                .and_then(|crumb| crumb.as_object().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter(|(key, _)| !["type", "ts", "level", "message", "name", "eventName", "category", "method", "url", "status"].contains(&key.as_str()))
                .collect();
            let json = if fields.is_empty() { row.crumb.clone() } else { serde_json::Value::Object(fields).to_string() };
            let toggled: Vec<String> = self.row_toggled.iter().filter_map(|path| path.strip_prefix(&format!("{}|", row.key)).map(str::to_string)).collect();
            div().ml_6().mr_3().mb_2().p_2().rounded(px(4.)).bg(panel.opacity(0.3)).child(tree(kv_rows(&json, &[], 2, &toggled), &format!("diag-event-kv-{}", row.key), true, cx))
        });
        div()
            .flex()
            .flex_col()
            .when(separated, |row| row.border_t_1().border_color(border))
            .children(new_session)
            .child(line)
            .children(content)
            .into_any_element()
    }
}

/// La tira de latidos: un punto por latido y una banda roja por hueco, del
/// primer latido guardado a ahora; el hueco que sigue abierto, sólido.
fn strip(detail: &DeviceDetail, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (muted, danger, success, panel) = (theme.muted_foreground, theme.danger, theme.success, theme.muted);
    let language = locale(cx);
    if detail.heartbeats.is_empty() {
        return div().id("diag-beats-none").text_xs().text_color(muted).child(tr(cx, "diagnostics:beats.none")).test_support().into_any_element();
    }
    let now = chrono::Utc::now();
    let first = &detail.heartbeats[0];
    let from = chrono::DateTime::parse_from_rfc3339(first).map(|at| at.timestamp_millis()).unwrap_or_default();
    let marks = heartbeat_strip(&detail.heartbeats, &detail.gaps, from, now.timestamp_millis());
    let local = |iso: &str| chrono::DateTime::parse_from_rfc3339(iso).map(|at| cac_core::date_and_time(language, at.with_timezone(&chrono::Local).naive_local())).unwrap_or_default();
    let mut count = text_count(language, "diagnostics:beats.count", detail.heartbeats.len() as i64);
    if let Some(beat) = &detail.heartbeat {
        count = format!("{count} · {}", text_with(language, "diagnostics:beats.every", &[("duration", &gap_duration(beat.interval_seconds))]));
    }
    let gap_phrase = |open: bool, seconds: i64| text_with(language, if open { "diagnostics:beats.gapOpen" } else { "diagnostics:beats.gap" }, &[("duration", &gap_duration(seconds))]);
    let bar = div()
        .id("diag-strip")
        .aria_label(tr(cx, "diagnostics:beats.title"))
        .relative()
        .h_4()
        .rounded(px(4.))
        .bg(panel.opacity(0.5))
        .children(marks.iter().enumerate().map(|(index, mark)| {
            let tip = match mark.kind {
                StripKind::Gap => {
                    let seconds = chrono::DateTime::parse_from_rfc3339(&mark.to).ok().zip(chrono::DateTime::parse_from_rfc3339(&mark.from).ok()).map(|(to, from)| (to - from).num_seconds()).unwrap_or_default();
                    gap_phrase(mark.open, seconds)
                }
                StripKind::Beat => local(&mark.from),
            };
            let id = SharedString::from(format!("diag-strip-{}-{index}", if mark.kind == StripKind::Gap { "gap" } else { "beat" }));
            let shape = match mark.kind {
                StripKind::Gap => div().top_0().bottom_0().w(relative(mark.width.max(0.005))).rounded(px(2.)).bg(if mark.open { danger.opacity(0.6) } else { danger.opacity(0.25) }),
                StripKind::Beat => div().top_1().bottom_1().w(px(1.)).bg(success),
            };
            shape.id(id).absolute().left(relative(mark.left)).tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)).test_support()
        }))
        .test_support();
    let gaps = match &detail.heartbeat {
        None => Some(div().text_xs().text_color(muted).child(tr(cx, "diagnostics:beats.notConfigured")).into_any_element()),
        Some(_) if !detail.gaps.is_empty() => Some(
            div()
                .flex()
                .flex_col()
                .text_xs()
                .children(detail.gaps.iter().rev().take(5).map(|gap| div().text_color(if gap.open { danger } else { muted }).child(format!("{} · {}", gap_phrase(gap.open, gap.seconds), local(&gap.from)))))
                .into_any_element(),
        ),
        Some(_) => None,
    };
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(div().flex().justify_between().text_xs().text_color(muted).child(count).child(format!("{} → {}", local(first), cac_core::date_and_time(language, now.with_timezone(&chrono::Local).naive_local()))))
        .child(bar)
        .children(gaps)
        .into_any_element()
}

/// Un JSON como árbol de clave y valor. Pulsar una rama la abre o la cierra.
fn tree(rows: Vec<KvRow>, id: &str, in_event: bool, cx: &mut Context<DiagnosticsView>) -> AnyElement {
    let theme = cx.theme();
    let (muted, danger, warning, success, info, accent, mono) = (theme.muted_foreground, theme.danger, theme.warning, theme.success, theme.info, theme.accent, theme.mono_font_family.clone());
    let owner = id.to_string();
    div()
        .flex()
        .flex_col()
        .font_family(mono)
        .text_xs()
        .children(rows.into_iter().map(|row| {
            let tone = match row.tone.as_deref() {
                Some("error") => Some(danger),
                Some("warn") => Some(warning),
                _ => None,
            };
            let indent = px(row.depth as f32 * 12.);
            let line = div().id(SharedString::from(format!("{owner}-{}", row.path))).flex().gap_2().rounded(px(3.)).px_1().when_some(tone, |line, tone| line.bg(tone.opacity(0.1)).text_color(tone));
            if row.kind == KvKind::Branch {
                let (path, event) = (row.path.clone(), owner.strip_prefix("diag-event-kv-").map(str::to_string));
                line.pl(indent)
                    .items_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(accent.opacity(0.5)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let (list, path) = match (&event, in_event) {
                            (Some(event), true) => (&mut this.row_toggled, format!("{event}|{path}")),
                            _ => (&mut this.sheet_toggled, path.clone()),
                        };
                        if let Some(at) = list.iter().position(|known| *known == path) {
                            list.remove(at);
                        } else {
                            list.push(path);
                        }
                        cx.notify();
                    }))
                    .child(Icon::new(if row.open { IconName::ChevronDown } else { IconName::ChevronRight }).size_3())
                    .child(div().text_color(muted).child(row.key))
                    .child(div().text_color(muted.opacity(0.6)).child(row.value))
                    .into_any_element()
            } else {
                let color = match row.kind {
                    KvKind::Null => muted.opacity(0.6),
                    KvKind::True => success,
                    KvKind::False => muted,
                    KvKind::Number => info,
                    _ => cx.theme().foreground,
                };
                line.pl(indent + px(16.))
                    .child(div().flex_none().text_color(muted).child(row.key))
                    .child(div().min_w_0().when(tone.is_none(), |value| value.text_color(color)).child(row.value))
                    .into_any_element()
            }
        }))
        .into_any_element()
}

impl Render for DiagnosticsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.diagnostics.clone().unwrap_or_default();
        let theme = cx.theme();
        let (background, foreground, muted, border, panel, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.muted, theme.danger);
        let right = match (&state.selected, &state.detail, &state.detail_error) {
            (None, _, _) => div().size_full().flex().items_center().justify_center().p_4().child(div().id("diag-pick").text_color(muted).child(tr(cx, "diagnostics:pick")).test_support()).into_any_element(),
            (Some(_), detail, error) => {
                let sheet = match (detail, error) {
                    (_, Some(error)) => div().id("diag-sheet-error").flex().items_center().gap_1p5().text_xs().text_color(danger).child(Icon::new(Lucide::CircleAlert).size_3()).child(error.clone()).test_support().into_any_element(),
                    (Some(detail), None) => self.sheet(detail, cx),
                    (None, None) => div().text_xs().text_color(muted).child(tr(cx, "diagnostics:loading")).into_any_element(),
                };
                div().mx_auto().w_full().max_w(px(896.)).p_4().flex().flex_col().gap_6().child(sheet).child(self.timeline(cx)).into_any_element()
            }
        };
        div()
            .size_full()
            .flex()
            .bg(background)
            .text_color(foreground)
            .leading_sm()
            .child(
                div()
                    .w(px(320.))
                    .flex_none()
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(border)
                    .bg(panel.opacity(0.1))
                    .child(
                        div()
                            .h(px(48.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_3()
                            .border_b_1()
                            .border_color(border)
                            .child(div().id("diag-title").flex().items_center().gap_2().font_weight(FontWeight::MEDIUM).child(Icon::new(Lucide::Activity).size_4()).child(tr(cx, "diagnostics:title")).test_support())
                            .child(
                                Button::new("diag-refresh")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(Lucide::RefreshCw))
                                    .tooltip(tr(cx, "common:last.refresh"))
                                    .disabled(state.loading_devices)
                                    .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.platform().load_devices(false).await })),
                            ),
                    )
                    .child(self.device_list(cx)),
            )
            .child(div().id("diag-main").flex_1().min_w_0().h_full().overflow_y_scroll().child(right))
    }
}
