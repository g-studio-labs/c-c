//! La pestaña de recursos de un swarm (`ServerStats.tsx`): lo que gasta cada
//! contenedor, leído cada cinco segundos mientras se mira, con pausa, orden y
//! filtro. El sondeo lo lleva el core; aquí sólo se pinta y se elige qué ver.

use std::collections::HashMap;

use cac_core::{cpu_load, format_bytes, mem_load, stats_view, time_ago, ContainerStats, Load, PlatformState, StatsColumn, StatsSort};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

use super::services::{empty, pill};

/// Las columnas: la etiqueta, por qué se ordena (el nodo no se ordena, como
/// en Tauri), el ancho y si los números van a la derecha.
const COLUMNS: [(&str, Option<StatsColumn>, f32, bool); 11] = [
    ("common:servers.thContainer", Some(StatsColumn::Container), 112., false),
    ("common:servers.thService", Some(StatsColumn::Service), 0., false),
    ("common:servers.thStack", Some(StatsColumn::Stack), 0., false),
    ("common:servers.thNode", None, 0., false),
    ("common:servers.thState", Some(StatsColumn::State), 76., false),
    ("CPU", Some(StatsColumn::Cpu), 80., false),
    ("common:servers.thMemory", Some(StatsColumn::Memory), 128., false),
    ("common:servers.thNetRx", Some(StatsColumn::NetRx), 72., true),
    ("common:servers.thNetTx", Some(StatsColumn::NetTx), 72., true),
    ("common:servers.thDiskR", Some(StatsColumn::DiskRead), 72., true),
    ("common:servers.thDiskW", Some(StatsColumn::DiskWrite), 72., true),
];

/// Por debajo de esto las columnas de texto libre se quedan en nada.
const TABLE_MIN_WIDTH: f32 = 1120.;

pub struct StatsPanel {
    filter: Entity<InputState>,
    _filter_changed: Subscription,
    show_stopped: bool,
    sort: StatsSort,
}

impl StatsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "common:servers.filterContainers");
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let filter_changed = cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        StatsPanel { filter, _filter_changed: filter_changed, show_stopped: false, sort: StatsSort::default() }
    }

    fn toolbar(&self, state: &PlatformState, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let polling = if state.stats_paused { tr(cx, "common:servers.paused") } else { tr(cx, "common:servers.pollingEvery") };
        let last = state.stats_fetched_at.as_deref().map(|at| time_ago(at, &chrono::Local::now(), locale(cx))).unwrap_or_else(|| tr(cx, "common:time.never"));
        let status = cac_core::text_with(locale(cx), "common:servers.lastRead", &[("polling", &polling), ("ago", &last)]);
        let paused = state.stats_paused;
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(Icon::new(Lucide::Activity).size_5().text_color(muted))
            .child(div().flex_1().leading_lg().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:servers.resourceMonitor")))
            .child(div().id("stats-status").aria_label(status.clone()).text_xs().leading_xs().text_color(muted).child(status).test_support())
            .child(
                Button::new("stats-refresh")
                    .outline()
                    .small()
                    .icon(Icon::new(Lucide::RefreshCw))
                    .label(tr(cx, "common:servers.refresh"))
                    .disabled(state.stats_loading)
                    .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.platform().refresh_stats().await })),
            )
            .child(
                Button::new("stats-pause")
                    .outline()
                    .small()
                    .icon(Icon::new(if paused { Lucide::Play } else { Lucide::Pause }))
                    .label(tr(cx, if paused { "common:servers.resume" } else { "common:servers.pause" }))
                    // Reanudar arranca el sondeo, que necesita el runtime.
                    .on_click(move |_, _, cx| Backend::run(cx, move |core| async move { core.platform().pause_stats(!paused) })),
            )
    }

    fn table(&self, rows: &[ContainerStats], node_names: &HashMap<String, String>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, foreground) = (theme.border, theme.muted_foreground, theme.foreground);
        let sort = self.sort;
        let heading = row(
            COLUMNS
                .iter()
                .map(|(key, column, _, _)| {
                    let label = if key.contains(':') { tr(cx, key) } else { key.to_string() };
                    let Some(column) = *column else {
                        return div().font_weight(FontWeight::MEDIUM).text_color(muted).child(label).into_any_element();
                    };
                    let active = sort.column == column;
                    let icon = match (active, sort.descending) {
                        (false, _) => Lucide::ArrowUpDown,
                        (true, true) => Lucide::ArrowDown,
                        (true, false) => Lucide::ArrowUp,
                    };
                    div()
                        .id(SharedString::from(format!("stats-sort-{column:?}")))
                        .flex()
                        .items_center()
                        .gap_1()
                        .cursor_pointer()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if active { foreground } else { muted })
                        .child(label)
                        .child(Icon::new(icon).size_3())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sort = this.sort.toggled(column);
                            cx.notify();
                        }))
                        .test_support()
                        .into_any_element()
                })
                .collect(),
        )
        .h(px(36.));
        let lines = rows.iter().map(|stats| self.stats_row(stats, node_names, cx));
        let table = div().id("stats-table").flex().flex_col().min_w(px(TABLE_MIN_WIDTH)).rounded(px(8.)).border_1().border_color(border).leading_sm().child(heading).children(lines.collect::<Vec<_>>()).test_support();
        // Once columnas no caben en una ventana estrecha: la tabla se desliza
        // de lado, como en Tauri, en vez de esconder servicio, stack y nodo.
        div().id("stats-scroll").overflow_x_scroll().child(table).into_any_element()
    }

    fn stats_row(&self, stats: &ContainerStats, node_names: &HashMap<String, String>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, danger, mono) = (theme.border, theme.muted_foreground, theme.danger, theme.mono_font_family.clone());
        let state_colors = match stats.state.as_str() {
            "running" => (theme.primary.opacity(0.15), theme.primary),
            "exited" | "dead" => (danger.opacity(0.15), danger),
            _ => (theme.secondary, theme.secondary_foreground),
        };
        let dash = || div().text_color(muted).child("—").into_any_element();
        let bytes = |n: u64| div().font_family(mono.clone()).text_xs().leading_xs().child(format_bytes(n)).into_any_element();
        let container = div()
            .flex()
            .items_center()
            .gap_1()
            .font_family(mono.clone())
            .text_xs()
            .leading_xs()
            .when(stats.error.is_some(), |id| id.text_color(danger).child(Icon::new(Lucide::CircleAlert).size_3()))
            .child(stats.short_id().to_string());
        let node = node_names.get(&stats.node_id).cloned().unwrap_or_else(|| stats.node_id.chars().take(8).collect());
        row(vec![
            container.into_any_element(),
            if stats.service_name.is_empty() { dash() } else { div().text_xs().leading_xs().truncate().child(stats.service_name.clone()).into_any_element() },
            if stats.stack.is_empty() { dash() } else { pill(stats.stack.clone(), theme.secondary, theme.secondary_foreground).into_any_element() },
            div()
                .id(SharedString::from(format!("stats-node-{}", stats.container_id)))
                .aria_label(node.clone())
                .font_family(mono.clone())
                .text_xs()
                .leading_xs()
                .text_color(muted)
                .truncate()
                .child(node)
                .test_support()
                .into_any_element(),
            pill(stats.state.clone(), state_colors.0, state_colors.1).into_any_element(),
            bar(format!("{:.1}%", stats.cpu_percent), None, stats.cpu_fill(), cpu_load(stats.cpu_fill()), cx),
            bar(format_bytes(stats.mem_usage), Some(format_bytes(stats.mem_limit)), stats.mem_fill(), mem_load(stats.mem_fill()), cx),
            bytes(stats.net_rx),
            bytes(stats.net_tx),
            bytes(stats.block_read),
            bytes(stats.block_write),
        ])
        .id(SharedString::from(format!("stats-row-{}", stats.container_id)))
        .aria_label(stats.service_name.clone())
        .when_some(stats.error.clone(), |row, error| row.tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(error.clone()).build(window, cx)))
        .h(px(48.))
        .border_t_1()
        .border_color(border)
        .test_support()
        .into_any_element()
    }
}

impl Render for StatsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let theme = cx.theme();
        let (muted, danger, border) = (theme.muted_foreground, theme.danger, theme.border);
        let filter_text = self.filter.read(cx).value().to_string();
        let stats = state.stats.clone().unwrap_or_default();
        let view = stats_view(&stats, &filter_text, self.show_stopped, self.sort);
        let node_names: HashMap<String, String> = state.nodes.iter().flatten().map(|node| (node.id.clone(), node.hostname.clone())).collect();
        let problem = state.stats_error.clone().map(|error| {
            div().id("stats-error").px_4().py_3().rounded(px(6.)).border_1().border_color(danger.opacity(0.5)).bg(danger.opacity(0.1)).leading_sm().text_color(danger).child(error).test_support()
        });
        let count = cac_core::text_with(locale(cx), "common:servers.containersOf", &[("shown", &view.rows.len().to_string()), ("total", &stats.len().to_string())]);
        let show_stopped = self.show_stopped;
        let card_header = div()
            .flex()
            .items_center()
            .child(div().id("stats-count").aria_label(count.clone()).flex_1().leading_sm().font_weight(FontWeight::MEDIUM).child(count).test_support())
            .child(
                Checkbox::new("stats-show-stopped").small().checked(show_stopped).label(tr(cx, "common:servers.showStopped")).on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.show_stopped = *checked;
                    cx.notify();
                })),
            );
        let filled = !filter_text.is_empty();
        let filter = div().id("stats-filter").child(
            Input::new(&self.filter).small().prefix(Icon::new(Lucide::Search).size_4().text_color(muted)).when(filled, |input| {
                input.suffix(
                    Button::new("stats-filter-clear")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::X))
                        .tooltip(tr(cx, "common:servers.clearFilter"))
                        .on_click(cx.listener(|this, _, window, cx| this.filter.update(cx, |input, cx| input.set_value("", window, cx)))),
                )
            }),
        ).test_support();
        let hidden = (!show_stopped && view.hidden_stopped > 0).then(|| {
            let phrase = format!("{} — {}", cac_core::text_count(locale(cx), "common:count.hiddenContainers", view.hidden_stopped as i64), tr(cx, "common:servers.showThem"));
            div()
                .id("stats-hidden")
                .aria_label(phrase.clone())
                .text_xs()
                .leading_xs()
                .text_color(muted)
                .underline()
                .cursor_pointer()
                .child(phrase)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_stopped = true;
                    cx.notify();
                }))
                .test_support()
        });
        let body = if stats.is_empty() {
            let key = if state.stats_loading || state.stats.is_none() { "common:servers.loading" } else { "common:servers.noSwarmContainers" };
            empty("stats-empty", tr(cx, key), muted)
        } else if view.rows.is_empty() {
            empty("stats-no-match", cac_core::text_with(locale(cx), "common:servers.noContainerMatches", &[("filter", filter_text.trim())]), muted)
        } else {
            self.table(&view.rows, &node_names, cx)
        };
        div()
            .id("server-stats")
            .flex()
            .flex_col()
            .gap_3()
            .child(self.toolbar(&state, cx))
            .children(problem)
            .child(div().flex().flex_col().gap_3().p_4().rounded(px(8.)).border_1().border_color(border).child(card_header).child(filter).children(hidden).child(body))
            .test_support()
    }
}

/// Una cifra con su barra debajo, del color de lo cargado que va.
fn bar(value: String, limit: Option<String>, fill: f32, load: Load, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let color = match load {
        Load::High => theme.danger,
        Load::Medium => theme.warning,
        Load::Low => theme.success,
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .leading_xs()
        .child(div().flex().justify_between().child(value).children(limit.map(|limit| div().text_color(theme.muted_foreground).child(limit))))
        .child(div().h(px(6.)).w_full().rounded(px(3.)).bg(theme.muted).child(div().h_full().rounded(px(3.)).bg(color).w(relative(fill))))
        .into_any_element()
}

/// Las columnas de ancho fijo con el suyo; servicio, stack y nodo se reparten
/// lo que queda.
fn row(cells: Vec<AnyElement>) -> Div {
    div().flex().items_center().px_3().gap_3().children(cells.into_iter().zip(COLUMNS).map(|(cell, (_, _, width, right))| {
        let column = if width > 0. { div().w(px(width)).flex_none() } else { div().flex_1().min_w(px(88.)) };
        column.whitespace_nowrap().overflow_hidden().when(right, |column| column.flex().justify_end()).child(cell)
    }))
}
