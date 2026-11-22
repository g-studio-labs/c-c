//! Un servidor abierto: su cabecera, sus pestañas y el resumen
//! (`ServerLayout.tsx` y `ServerOverview.tsx`). Las demás pestañas llegan con
//! las siguientes partes de la fase 1 y con las fases 2–4; mientras, dicen que
//! todavía no están.

use cac_core::{server_tabs, time_ago, AgentNote, PlatformState, Server, ServerKind, ServerOverview, ServerTab};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Las pestañas que son una vista con su propio estado: las da quien las
/// guarda, una por servidor.
pub(crate) struct Panels {
    pub services: Option<AnyElement>,
    pub stats: Option<AnyElement>,
    pub k8s: Option<AnyElement>,
    pub deploy: Option<AnyElement>,
    pub secrets: Option<AnyElement>,
}

pub(crate) fn page(server: &Server, state: &PlatformState, agent_actions: Option<AnyElement>, panels: Panels, cx: &App) -> AnyElement {
    let Panels { services, stats, k8s, deploy, secrets } = panels;
    let theme = cx.theme();
    let (background, foreground) = (theme.background, theme.foreground);
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(background)
        .text_color(foreground)
        .line_height(relative(1.5))
        .child(header(server, cx))
        .child(tabs(server, state.tab, cx))
        .child(
            div()
                .id("server-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .test_support()
                .child(match state.tab {
                    ServerTab::Overview => overview(server, state.overview.as_ref(), agent_actions, k8s, cx),
                    ServerTab::Services => services.unwrap_or_else(|| pending(cx)),
                    ServerTab::Nodes => super::nodes::nodes(state, cx),
                    ServerTab::Stats => stats.unwrap_or_else(|| pending(cx)),
                    ServerTab::Deploy => deploy.unwrap_or_else(|| pending(cx)),
                    ServerTab::Secrets => secrets.unwrap_or_else(|| pending(cx)),
                    _ => pending(cx),
                }),
        )
        .into_any_element()
}

fn header(server: &Server, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (border, muted, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
    let state = div()
        .flex()
        .items_center()
        .gap_1p5()
        .leading_xs()
        .text_color(muted)
        .when(server.kind == ServerKind::Kubernetes, |row| row.child("kubernetes"))
        .when(server.kind != ServerKind::Kubernetes, |row| {
            row.child(div().size_2().rounded_full().bg(super::health_color(server.status, cx))).child(tr(cx, server.status.label_key()))
        });
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .px_4()
        .py_2()
        .min_h(px(52.))
        .border_b_1()
        .border_color(border)
        .child(
            Button::new("server-back")
                .ghost()
                .small()
                .icon(Icon::new(Lucide::ChevronLeft))
                .tooltip(tr(cx, "common:servers.back"))
                .on_click(|_, _, cx| Backend::core(cx).platform().close_server()),
        )
        .child(div().id("server-title").aria_label(server.name.clone()).leading_lg().font_weight(FontWeight::SEMIBOLD).child(server.name.clone()).test_support())
        .child(div().font_family(mono).leading_xs().text_color(muted).child(server.address()))
        .child(state)
}

fn tabs(server: &Server, current: ServerTab, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (border, primary, muted, foreground) = (theme.border, theme.primary, theme.muted_foreground, theme.foreground);
    div().flex().flex_none().px_4().border_b_1().border_color(border).children(server_tabs(server.kind).into_iter().map(|tab| {
        let active = tab == current;
        div()
            .id(SharedString::from(format!("server-tab-{}", tab.as_str())))
            .px_3()
            .py_2()
            .border_b_2()
            .border_color(if active { primary } else { transparent_black() })
            .leading_sm()
            .font_weight(FontWeight::MEDIUM)
            .text_color(if active { foreground } else { muted })
            .cursor_pointer()
            .hover(move |style| style.text_color(foreground))
            .on_click(move |_, _, cx| Backend::run(cx, move |core| async move { core.platform().open_tab(tab).await }))
            .child(tr(cx, tab.label_key()))
            .test_support()
    }))
}

fn pending(cx: &App) -> AnyElement {
    div().id("server-tab-pending").py_8().text_center().leading_sm().text_color(cx.theme().muted_foreground).child(tr(cx, "common:misc.notInThisApp")).test_support().into_any_element()
}

/// Cada tarjeta sin datos enseña cómo empezar, nunca un error: un servidor
/// que todavía no despliega nada desde cac está bien, sólo no ha optado.
fn overview(server: &Server, overview: Option<&ServerOverview>, agent_actions: Option<AnyElement>, k8s: Option<AnyElement>, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let Some(overview) = overview.filter(|overview| overview.server_id == server.id) else {
        return div().id("server-overview-loading").leading_sm().text_color(muted).child(tr(cx, "common:servers.loading")).test_support().into_any_element();
    };
    let wide = card("server-incidents", Lucide::CircleAlert, "work:incidents.serverTitle", incidents(overview, cx), cx);
    let mut halves = Vec::new();
    // Uno de kubernetes no lleva este agente: su consola llega con el hub.
    if server.kind != ServerKind::Kubernetes {
        halves.push(card("server-agent", Lucide::Bot, "common:servers.overview.agent", agent(server, agent_actions, cx), cx));
        halves.push(card("server-connection", Lucide::Link2, "common:servers.overview.connection", connection(server, cx), cx));
        halves.push(card("server-deploys", Lucide::Rocket, "common:servers.overview.deploys", deploys(overview, cx), cx));
    }
    halves.push(card("server-provisioning", Lucide::ClipboardList, "common:servers.overview.provisioning", provisioning(overview, cx), cx));
    let mut rows = vec![wide];
    let mut halves = halves.into_iter();
    while let Some(left) = halves.next() {
        let right = halves.next().unwrap_or_else(|| div().flex_1().into_any_element());
        rows.push(div().flex().gap_4().child(left).child(right).into_any_element());
    }
    // La consola de un clúster va debajo de lo que ha mordido en él.
    div().id("server-overview").flex().flex_col().gap_4().children(rows).children(k8s).test_support().into_any_element()
}

fn card(id: &'static str, icon: Lucide, title_key: &str, body: AnyElement, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded(px(10.))
        .border_1()
        .border_color(theme.border)
        .child(div().flex().items_center().gap_2().leading_sm().font_weight(FontWeight::MEDIUM).child(Icon::new(icon).size_4()).child(tr(cx, title_key)))
        .child(body)
        .test_support()
        .into_any_element()
}

fn muted_line(id: &'static str, text: String, cx: &App) -> AnyElement {
    div().id(id).leading_sm().text_color(cx.theme().muted_foreground).child(text).test_support().into_any_element()
}

fn incidents(overview: &ServerOverview, cx: &App) -> AnyElement {
    if overview.incidents.is_empty() {
        return muted_line("server-incidents-none", tr(cx, "work:incidents.none"), cx);
    }
    let theme = cx.theme();
    let (border, muted, warning, success) = (theme.border, theme.muted_foreground, theme.warning, theme.success);
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(overview.incidents.iter().map(|incident| {
            let open = incident.is_open();
            let day = incident.when().get(..10).unwrap_or_default().to_string();
            let tags = incident.servers.iter().map(|server| server.name.clone()).chain(incident.areas.iter().cloned());
            div()
                .id(SharedString::from(format!("server-incident-{}", incident.id)))
                .flex()
                .items_start()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .child(Icon::new(if open { Lucide::CircleAlert } else { Lucide::CircleCheck }).size_4().text_color(if open { warning } else { success }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(incident.title.clone()))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1p5()
                                .leading_xs()
                                .text_color(muted)
                                .child(tr(cx, if open { "work:incidents.open" } else { "work:incidents.fixed" }))
                                .children(tags.map(|tag| format!("· {tag}")))
                                .child(format!("· {day}")),
                        ),
                )
                .aria_label(incident.title.clone())
                .test_support()
        }))
        .into_any_element()
}

fn agent(server: &Server, actions: Option<AnyElement>, cx: &App) -> AnyElement {
    let locale = locale(cx);
    let muted = cx.theme().muted_foreground;
    let mut facts = vec![tr(cx, server.status.label_key())];
    if server.has_agent_token && server.agent_version > 0 {
        facts.push(cac_core::text_with(locale, "common:servers.overview.agentVersion", &[("version", &server.agent_version.to_string())]));
    }
    if let Some(seen) = &server.agent_seen_at {
        let when = time_ago(seen, &chrono::Local::now(), locale);
        facts.push(cac_core::text_with(locale, "common:servers.overview.agentSeen", &[("when", &when)]));
    }
    let note = AgentNote::of(server).map(|note| match note {
        AgentNote::NoIdentity => tr(cx, "common:servers.overview.agentNoIdentity"),
        AgentNote::Old { version } => cac_core::text_with(locale, "common:servers.overview.agentOld", &[("version", &version.to_string())]),
    });
    div()
        .flex()
        .flex_col()
        .gap_1()
        .leading_sm()
        .child(div().flex().items_center().gap_1p5().child(div().size_2().rounded_full().bg(super::health_color(server.status, cx))).child(facts.join(" · ")))
        .children(note.map(|note| div().id("server-agent-note").text_color(muted).child(note).test_support()))
        .children(actions)
        .into_any_element()
}

fn connection(server: &Server, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let port = cac_core::text_with(locale(cx), "common:servers.overview.agentPort", &[("port", &server.agent_port.to_string())]);
    div()
        .flex()
        .flex_col()
        .gap_0p5()
        .font_family(theme.mono_font_family.clone())
        .leading_xs()
        .text_color(theme.muted_foreground)
        .child(div().id("server-ssh").child(format!("ssh {}@{} -p {}", server.ssh_user, server.host, server.ssh_port)).test_support())
        .child(port)
        .into_any_element()
}

fn deploys(overview: &ServerOverview, cx: &App) -> AnyElement {
    if overview.deployables.is_empty() {
        return muted_line("server-deploys-empty", tr(cx, "common:servers.overview.deploysEmpty"), cx);
    }
    let theme = cx.theme();
    let (muted, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(overview.deployables.iter().map(|deployable| {
            let image = deployable.image_tag().unwrap_or_else(|| tr(cx, "common:servers.overview.notDeployedYet"));
            let mode = if deployable.on_ci_notify.is_empty() { String::new() } else { tr(cx, &format!("common:deploy.ci.modes.{}", deployable.on_ci_notify)) };
            div()
                .id(SharedString::from(format!("server-deployable-{}", deployable.id)))
                .flex()
                .items_center()
                .gap_2()
                .leading_sm()
                .child(div().font_weight(FontWeight::MEDIUM).child(deployable.name.clone()))
                .when(!deployable.environment.is_empty(), |row| row.child(div().leading_xs().text_color(muted).child(deployable.environment.clone())))
                .child(div().flex_1().min_w_0().truncate().text_right().font_family(mono.clone()).leading_xs().text_color(muted).child(image))
                .child(div().leading_xs().text_color(muted).child(mode))
                .test_support()
        }))
        .into_any_element()
}

fn provisioning(overview: &ServerOverview, cx: &App) -> AnyElement {
    let Some(run) = &overview.last_run else {
        return muted_line("server-provisioning-empty", tr(cx, "common:servers.overview.provisioningEmpty"), cx);
    };
    let theme = cx.theme();
    let (muted, mono, danger, success) = (theme.muted_foreground, theme.mono_font_family.clone(), theme.danger, theme.success);
    let color = match run.status.as_str() {
        "succeeded" => success,
        "failed" => danger,
        _ => muted,
    };
    let when = time_ago(&run.started_at, &chrono::Local::now(), locale(cx));
    div()
        .id("server-last-run")
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .leading_xs()
                .child(div().font_family(mono.clone()).child(run.playbook.clone()))
                .child(div().text_color(color).child(run.status.clone()))
                .child(div().text_color(muted).child(when)),
        )
        .when(!run.summary.is_empty(), |column| column.child(div().font_family(mono).text_size(px(11.)).text_color(muted).child(run.summary.clone())))
        .test_support()
        .into_any_element()
}
