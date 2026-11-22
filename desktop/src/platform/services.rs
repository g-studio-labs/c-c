//! La pestaña de servicios de un swarm (`ServerServices.tsx`): la tabla con
//! su filtro, reiniciar, y el log en vivo de uno debajo.

use cac_core::{filter_services, server_edit_phrase, PlatformState, ServiceHealth, SwarmService};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::log_view::LogBox;
use crate::theme::Leading as _;

/// Las columnas: las de texto libre se reparten el ancho; las de largo
/// conocido (réplicas, estado, fecha) van con el suyo fijo, que repartidas se
/// quedan en «h…» en una ventana estrecha. Las acciones, aparte: son botones,
/// y recortar uno lo deja inservible.
#[derive(Clone, Copy)]
enum Width {
    Share(f32),
    Fixed(f32),
}
const COLUMNS: [(&str, Width); 6] = [
    ("common:servers.thName", Width::Share(3.)),
    ("common:servers.thImage", Width::Share(3.)),
    ("common:servers.thStack", Width::Share(2.)),
    ("common:servers.thReplicas", Width::Fixed(68.)),
    ("common:servers.thStatus", Width::Fixed(84.)),
    ("common:servers.thUpdated", Width::Fixed(124.)),
];
const ACTIONS_WIDTH: f32 = 168.;

pub struct ServicesView {
    filter: Entity<InputState>,
    _filter_changed: Subscription,
    logs: LogBox,
    /// Cómo acabó el último reinicio: la frase y si falló.
    restart_line: Option<(String, bool)>,
}

impl ServicesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "common:servers.filterServices");
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let filter_changed = cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        ServicesView { filter, _filter_changed: filter_changed, logs: LogBox::new(), restart_line: None }
    }

    fn restart(&mut self, service: SwarmService, cx: &mut Context<Self>) {
        self.restart_line = None;
        cx.notify();
        let id = service.id.clone();
        let done = Backend::ask(cx, move |core| async move { core.platform().restart_service(&id).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = done.await else { return };
            this.update(cx, |this, cx| {
                let language = locale(cx);
                this.restart_line = Some(match result {
                    Ok(()) => (cac_core::text_with(language, "common:last.restarting", &[("name", &service.name)]), false),
                    Err(error) => (server_edit_phrase(language, &error, "common:servers.restartFailed"), true),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn table(&self, state: &PlatformState, services: &[SwarmService], cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let filter = self.filter.read(cx).value().to_string();
        let shown = filter_services(services, &filter);
        if services.is_empty() {
            return empty("services-empty", tr(cx, "common:servers.noServices"), muted);
        }
        if shown.is_empty() {
            return empty("services-no-match", cac_core::text_with(locale(cx), "common:servers.noMatches", &[("filter", filter.trim())]), muted);
        }
        let border = cx.theme().border;
        let heading = row(
            COLUMNS.iter().map(|(key, _)| div().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, key)).into_any_element()).collect(),
            div().text_right().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, "common:servers.thActions")).into_any_element(),
        )
        .h(px(36.));
        let following = state.logs.as_ref().map(|logs| logs.service_id.clone());
        let rows: Vec<AnyElement> = shown.iter().map(|service| self.service_row(service, state, following.as_deref() == Some(&service.id), cx)).collect();
        let count = (!filter.trim().is_empty()).then(|| {
            let phrase = cac_core::text_with(locale(cx), "common:servers.showingOf", &[("shown", &shown.len().to_string()), ("total", &services.len().to_string())]);
            div().id("services-count").aria_label(phrase.clone()).text_right().leading_xs().text_color(muted).child(phrase).test_support()
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().id("services-table").flex().flex_col().rounded(px(8.)).border_1().border_color(border).leading_sm().child(heading).children(rows).test_support())
            .children(count)
            .into_any_element()
    }

    fn service_row(&self, service: &SwarmService, state: &PlatformState, following: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, accent, mono) = (theme.border, theme.muted_foreground, theme.accent, theme.mono_font_family.clone());
        let (secondary_bg, secondary_fg) = (theme.secondary, theme.secondary_foreground);
        let health = service.health();
        let (replicas_color, badge_color) = match health {
            ServiceHealth::Down => (theme.danger, theme.danger),
            ServiceHealth::Degraded => (theme.warning, theme.warning),
            ServiceHealth::Healthy => (theme.success, theme.success),
        };
        let updated = chrono::DateTime::parse_from_rfc3339(&service.updated_at)
            .map(|at| cac_core::date_and_time(locale(cx), at.with_timezone(&chrono::Local).naive_local()))
            .unwrap_or_default();
        let (logged, restarted, deployed) = (service.clone(), service.clone(), service.name.clone());
        let actions = div()
            .flex()
            .justify_end()
            .gap_0p5()
            .child(
                Button::new(SharedString::from(format!("service-logs-{}", service.id)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Lucide::SquareTerminal))
                    .label(tr(cx, "common:servers.logs"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let service = logged.clone();
                        Backend::run(cx, move |core| async move {
                            let platform = core.platform();
                            // Pulsar el del que ya se sigue lo cierra, como en Tauri.
                            if platform.state().logs.is_some_and(|logs| logs.service_id == service.id) {
                                platform.close_logs();
                            } else {
                                platform.follow_logs(&service);
                            }
                        });
                    })),
            )
            // A su despliegue, con él elegido (`DeployDialog` en Tauri).
            .child(
                Button::new(SharedString::from(format!("service-deploy-{}", service.id)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Lucide::Rocket))
                    .tooltip(tr(cx, "common:deploy.deploy"))
                    .on_click(move |_, _, cx| {
                        let name = deployed.clone();
                        Backend::run(cx, move |core| async move {
                            core.platform().open_tab(cac_core::ServerTab::Deploy).await;
                            core.platform().choose_service(&name).await;
                        });
                    }),
            )
            .when(state.permissions.restart_service, |actions| {
                actions.child(
                    Button::new(SharedString::from(format!("service-restart-{}", service.id)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::RotateCcw))
                        .label(tr(cx, "common:servers.restart"))
                        .on_click(cx.listener(move |this, _, _, cx| this.restart(restarted.clone(), cx))),
                )
            });
        row(
            vec![
                div().font_weight(FontWeight::MEDIUM).truncate().child(service.name.clone()).into_any_element(),
                div().font_family(mono.clone()).leading_xs().text_color(muted).truncate().child(service.image.clone()).into_any_element(),
                if service.stack.is_empty() {
                    div().text_color(muted).child("—").into_any_element()
                } else {
                    pill(service.stack.clone(), secondary_bg, secondary_fg).into_any_element()
                },
                div().font_family(mono).text_color(replicas_color).child(format!("{}/{}", service.replicas.running, service.replicas.desired)).into_any_element(),
                pill(health.label().to_string(), badge_color.opacity(0.15), badge_color).into_any_element(),
                div().text_xs().leading_xs().text_color(muted).truncate().child(updated).into_any_element(),
            ],
            actions.into_any_element(),
        )
        .id(SharedString::from(format!("service-row-{}", service.id)))
        .aria_label(service.name.clone())
        .h(px(44.))
        .border_t_1()
        .border_color(border)
        .when(following, |this| this.bg(accent))
        .test_support()
        .into_any_element()
    }

    fn logs_panel(&self, state: &PlatformState, cx: &mut Context<Self>) -> Option<AnyElement> {
        let logs = state.logs.as_ref()?;
        let theme = cx.theme();
        let (border, muted, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
        let dot = match logs.status {
            cac_core::LogStatus::Streaming => theme.success,
            cac_core::LogStatus::Failed => theme.danger,
            _ => theme.warning,
        };
        let status = tr(cx, logs.status.label_key());
        let title = cac_core::text_with(locale(cx), "common:servers.logsOf", &[("name", &logs.service_name)]);
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(Icon::new(Lucide::Terminal).size_4())
            .child(div().flex_1().leading_sm().font_weight(FontWeight::MEDIUM).child(title))
            .child(
                Button::new("service-logs-clear")
                    .ghost()
                    .xsmall()
                    .label(tr(cx, "common:servers.clearLogs"))
                    .disabled(logs.lines.is_empty())
                    .on_click(|_, _, cx| Backend::core(cx).platform().clear_logs()),
            )
            .child(Button::new("service-logs-close").ghost().xsmall().icon(Icon::new(Lucide::X)).on_click(|_, _, cx| Backend::core(cx).platform().close_logs()));
        let status_line = div()
            .id("service-logs-status")
            .aria_label(status.clone())
            .flex()
            .items_center()
            .gap_2()
            .leading_xs()
            .text_color(muted)
            .child(div().size_2().rounded_full().bg(dot))
            .child(status)
            .test_support();
        Some(
            div()
                .id("service-logs")
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .child(header)
                .child(status_line)
                .child(self.logs.render("service-log", &logs.lines, tr(cx, "common:servers.waitingForLogs"), mono))
                .test_support()
                .into_any_element(),
        )
    }
}

impl Render for ServicesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let filled = !self.filter.read(cx).value().is_empty();
        let filter = div()
            .id("services-filter")
            .child(
                Input::new(&self.filter).small().prefix(Icon::new(Lucide::Search).size_4().text_color(muted)).when(filled, |input| {
                    input.suffix(
                        Button::new("services-filter-clear")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::X))
                            .tooltip(tr(cx, "common:servers.clearFilter"))
                            .on_click(cx.listener(|this, _, window, cx| this.filter.update(cx, |input, cx| input.set_value("", window, cx)))),
                    )
                }),
            )
            .test_support();
        let problem = state.services_error.clone().map(|error| {
            div().id("services-error").flex().items_center().gap_1p5().leading_xs().text_color(danger).child(Icon::new(Lucide::CircleAlert).size_3()).child(error).test_support()
        });
        let restart_line = self.restart_line.clone().map(|(phrase, failed)| {
            div().id("services-restart-result").aria_label(phrase.clone()).leading_xs().text_color(if failed { danger } else { muted }).child(phrase).test_support()
        });
        let body = match state.services.as_deref() {
            Some(services) => self.table(&state, services, cx),
            None if state.services_error.is_some() => div().into_any_element(),
            None => empty("services-loading", tr(cx, "common:servers.loading"), muted),
        };
        div()
            .id("server-services")
            .flex()
            .flex_col()
            .gap_3()
            .child(filter)
            .children(problem)
            .children(restart_line)
            .child(body)
            .children(self.logs_panel(&state, cx))
            .test_support()
    }
}

pub(crate) fn empty(id: &'static str, phrase: String, muted: Hsla) -> AnyElement {
    div().id(id).py_8().text_center().leading_sm().text_color(muted).child(phrase).test_support().into_any_element()
}

fn row(cells: Vec<AnyElement>, actions: AnyElement) -> Div {
    div()
        .flex()
        .items_center()
        .px_3()
        .gap_3()
        // Una celda nunca parte en dos líneas: «Réplicas» o «healthy» cortados
        // a media palabra no se leen. Lo que no cabe se recorta.
        .children(cells.into_iter().zip(COLUMNS).map(|(cell, (_, width))| {
            let column = match width {
                Width::Share(share) => div().flex_grow(share).flex_basis(px(0.)).min_w_0(),
                Width::Fixed(width) => div().w(px(width)).flex_none(),
            };
            column.whitespace_nowrap().overflow_hidden().child(cell)
        }))
        .child(div().w(px(ACTIONS_WIDTH)).flex_none().child(actions))
}

/// Como el `Badge` de Tauri: el stack y el estado se leen de un vistazo.
pub(crate) fn pill(text: String, background: Hsla, color: Hsla) -> Div {
    div().flex().child(div().px_1p5().rounded(px(4.)).bg(background).text_color(color).text_xs().leading_xs().font_weight(FontWeight::MEDIUM).truncate().child(text))
}
