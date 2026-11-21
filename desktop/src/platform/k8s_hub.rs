//! La consola de un servidor kubernetes (`K8sHub.tsx`): sus integraciones,
//! gateways, rutas, la salud del clúster, sus cargas de trabajo y sus
//! certificados. Sólo se mira: dar de alta o editar integraciones llega
//! después. Y la etiqueta de cada clúster en la lista (`K8sStatusBadge.tsx`).

use std::collections::HashMap;

use cac_core::{cert_warns, days_until, route_links, workloads_failing_first, ClusterStatus, Integration, K8sHub, PlatformState};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

use super::services::pill;

/// La etiqueta de un clúster: en vez del estado del agente de swarm, que un
/// clúster no lleva, lo que dice su salud. El detalle, al pasar por encima.
pub(crate) fn cluster_badge(server_id: &str, state: &PlatformState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let language = locale(cx);
    let status = state.cluster_badges.iter().find(|badge| badge.server_id == server_id).map(|badge| badge.status.clone());
    let (label, detail, colors) = match status {
        None => (tr(cx, "common:servers.checking"), String::new(), None),
        Some(ClusterStatus::Healthy { nodes }) => (tr(cx, "common:servers.clusterHealthy"), cac_core::text_count(language, "common:count.nodesReady", nodes as i64), Some((theme.primary.opacity(0.15), theme.primary))),
        Some(ClusterStatus::Issues { count, detail }) => (cac_core::text_count(language, "common:count.issues", count as i64), detail, Some((theme.secondary, theme.secondary_foreground))),
        Some(ClusterStatus::Down { detail }) => (tr(cx, "common:servers.clusterDown"), detail, Some((theme.danger.opacity(0.15), theme.danger))),
        Some(ClusterStatus::Unknown { detail }) => (tr(cx, "common:servers.clusterUnknown"), detail, Some((transparent_black(), theme.muted_foreground))),
    };
    let body = match colors {
        Some((background, color)) => pill(label.clone(), background, color).into_any_element(),
        None => div().text_xs().leading_xs().text_color(theme.muted_foreground).child(label.clone()).into_any_element(),
    };
    div()
        .id(SharedString::from(format!("cluster-badge-{server_id}")))
        .aria_label(label)
        .when(!detail.is_empty(), |badge| badge.tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(detail.clone()).build(window, cx)))
        .child(body)
        .test_support()
        .into_any_element()
}

/// El punto de un clúster en el panel, con lo que dice al pasar por encima:
/// verde sano, rojo caído, ámbar con problemas, gris si no se sabe.
pub(crate) fn cluster_dot(server_id: &str, state: &PlatformState, cx: &App) -> (Hsla, String) {
    let theme = cx.theme();
    let language = locale(cx);
    match state.cluster_badges.iter().find(|badge| badge.server_id == server_id).map(|badge| &badge.status) {
        Some(ClusterStatus::Healthy { .. }) => (theme.success, tr(cx, "common:servers.clusterHealthy")),
        Some(ClusterStatus::Issues { count, .. }) => (theme.warning, cac_core::text_count(language, "common:count.issues", *count as i64)),
        Some(ClusterStatus::Down { .. }) => (theme.danger, tr(cx, "common:servers.clusterDown")),
        Some(ClusterStatus::Unknown { .. }) => (theme.muted_foreground.opacity(0.5), tr(cx, "common:servers.clusterUnknown")),
        None => (theme.muted_foreground.opacity(0.5), tr(cx, "common:servers.checking")),
    }
}

pub struct K8sConsole {
    /// Los secretos que se pidieron ver, por integración. Viven aquí y no en
    /// el core: se enseñan a quien los pidió, y se van al cerrar.
    revealed: HashMap<String, String>,
    copied: Option<String>,
    problem: Option<String>,
}

impl K8sConsole {
    pub fn new() -> Self {
        K8sConsole { revealed: HashMap::new(), copied: None, problem: None }
    }

    fn open(&mut self, integration: Integration, cx: &mut Context<Self>) {
        self.problem = None;
        let done = Backend::ask(cx, move |core| async move { core.platform().integration_link(&integration).await });
        cx.spawn(async move |this, cx| {
            let Ok(link) = done.await else { return };
            this.update(cx, |this, cx| {
                match link {
                    Ok(url) => cx.open_url(&url),
                    Err(error) => this.problem = Some(format!("{}: {error}", tr(cx, "common:admin.errOpenThrough"))),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn reveal(&mut self, id: String, cx: &mut Context<Self>) {
        self.problem = None;
        let asked = id.clone();
        let done = Backend::ask(cx, move |core| async move { core.platform().reveal_integration(&asked).await });
        cx.spawn(async move |this, cx| {
            let Ok(secret) = done.await else { return };
            this.update(cx, |this, cx| {
                match secret {
                    Ok(secret) => {
                        this.revealed.insert(id, secret);
                    }
                    Err(error) => this.problem = Some(cac_core::server_edit_phrase(locale(cx), &error, "common:admin.errReveal")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn integrations(&self, hub: &K8sHub, can_reveal: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
        if hub.integrations.is_empty() {
            return div().id("k8s-integrations-empty").text_xs().leading_xs().text_color(muted).child(tr(cx, "common:admin.noIntegrations")).test_support().into_any_element();
        }
        let tiles = hub.integrations.iter().map(|integration| {
            let opened = integration.clone();
            let id = integration.id.clone();
            let secret = self.revealed.get(&integration.id).cloned();
            let secret_line = (integration.has_secret && can_reveal).then(|| match secret {
                // En una fila propia: suelto en la columna, el botón se estira y
                // se centra.
                None => div()
                    .flex()
                    .child(
                        Button::new(SharedString::from(format!("k8s-reveal-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::KeyRound))
                            .label(tr(cx, "common:servers.revealCredentials"))
                            .on_click(cx.listener(move |this, _, _, cx| this.reveal(id.clone(), cx))),
                    )
                    .into_any_element(),
                Some(secret) => {
                    let shown = if secret.is_empty() { tr(cx, "common:servers.emptySecret") } else { secret.clone() };
                    let copied = self.copied.as_deref() == Some(integration.id.as_str());
                    let copy_id = integration.id.clone();
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .id(SharedString::from(format!("k8s-secret-{}", integration.id)))
                                .aria_label(shown.clone())
                                .flex_1()
                                .min_w_0()
                                .px_2()
                                .py_1()
                                .rounded(px(4.))
                                .bg(theme.muted)
                                .font_family(mono.clone())
                                .text_xs()
                                .leading_xs()
                                .child(shown)
                                .test_support(),
                        )
                        .child(
                            Button::new(SharedString::from(format!("k8s-copy-{}", integration.id)))
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(if copied { Lucide::Check } else { Lucide::Copy }))
                                .tooltip(tr(cx, "common:admin.copy"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(secret.clone()));
                                    this.copied = Some(copy_id.clone());
                                    cx.notify();
                                })),
                        )
                        .into_any_element()
                }
            });
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .leading_sm()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id(SharedString::from(format!("k8s-open-{}", integration.id)))
                                .aria_label(integration.name.clone())
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .items_center()
                                .gap_1p5()
                                .cursor_pointer()
                                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(integration.name.clone()))
                                .child(Icon::new(Lucide::ExternalLink).size_3().text_color(muted))
                                .on_click(cx.listener(move |this, _, _, cx| this.open(opened.clone(), cx)))
                                .test_support(),
                        )
                        .child(pill(integration.kind.clone(), theme.secondary, theme.secondary_foreground))
                        .when(integration.through_cac(), |row| row.child(pill("SSO".into(), theme.primary.opacity(0.15), theme.primary))),
                )
                .child(div().font_family(mono.clone()).text_xs().leading_xs().text_color(muted).truncate().child(integration.url.clone()))
                .children(secret_line)
                .into_any_element()
        });
        grid("k8s-integrations", tiles.collect())
    }

    fn routes(&self, hub: &K8sHub, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let (border, muted, danger, success, mono) = (theme.border, theme.muted_foreground, theme.danger, theme.success, theme.mono_font_family.clone());
        let Some(routes) = hub.routes.as_ref() else { return Vec::new() };
        let mut sections = Vec::new();
        if !routes.gateways.is_empty() {
            let tiles = routes.gateways.iter().map(|gateway| {
                let (label, colors) = if gateway.programmed { (tr(cx, "common:servers.programmed"), (theme.primary.opacity(0.15), theme.primary)) } else { (tr(cx, "common:servers.notProgrammed"), (danger.opacity(0.15), danger)) };
                div()
                    .id(SharedString::from(format!("k8s-gateway-{}-{}", gateway.namespace, gateway.name)))
                    .aria_label(format!("{} {label}", gateway.name))
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if gateway.programmed { border } else { danger.opacity(0.5) })
                    .when(!gateway.programmed, |tile| tile.bg(danger.opacity(0.05)))
                    .leading_sm()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().font_weight(FontWeight::MEDIUM).child(gateway.name.clone()))
                            .child(div().text_xs().text_color(muted).child(gateway.namespace.clone()))
                            .child(div().flex_1())
                            .child(pill(label, colors.0, colors.1)),
                    )
                    .child(div().font_family(mono.clone()).text_xs().leading_xs().text_color(muted).child(if gateway.address.is_empty() { tr(cx, "common:servers.noAddress") } else { gateway.address.clone() }))
                    .test_support()
                    .into_any_element()
            });
            sections.push(section(Lucide::Globe, tr(cx, "common:misc.gateways"), grid("k8s-gateways", tiles.collect()), cx));
        }
        let now = chrono::Utc::now();
        let tiles: Vec<AnyElement> = routes
            .routes
            .iter()
            .flat_map(|route| {
                route_links(route).into_iter().map(move |(host, url)| (route.clone(), host, url))
            })
            .map(|(route, host, url)| {
                let shield = route.cert_ready.map(|ready| {
                    let days = route.cert_expiry.as_deref().and_then(|expiry| days_until(expiry, now));
                    let warn = cert_warns(ready, days);
                    Icon::new(if warn { Lucide::ShieldAlert } else { Lucide::ShieldCheck }).size_3p5().text_color(if warn { danger } else { success })
                });
                let gateway = if route.gateway.is_empty() { tr(cx, "common:servers.noGateway") } else { route.gateway.clone() };
                div()
                    .id(SharedString::from(format!("k8s-route-{host}")))
                    .aria_label(host.clone())
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .cursor_pointer()
                    .leading_sm()
                    .child(Icon::new(Lucide::Globe).size_4().text_color(muted))
                    .child(div().flex_1().min_w_0().child(div().truncate().font_weight(FontWeight::MEDIUM).child(host.clone())).child(div().truncate().text_xs().leading_xs().text_color(muted).child(format!("{} · {gateway}", route.name))))
                    .children(shield)
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let title = cac_core::text_with(locale(cx), "common:servers.routesCount", &[("count", &routes.routes.len().to_string())]);
        sections.push(section(Lucide::ExternalLink, title, grid("k8s-routes", tiles), cx));
        sections
    }

    fn health(&self, hub: &K8sHub, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let (border, muted, danger, success) = (theme.border, theme.muted_foreground, theme.danger, theme.success);
        let Some(health) = hub.health.as_ref() else { return Vec::new() };
        let language = locale(cx);
        let mut sections = Vec::new();
        let nodes = div().flex().flex_wrap().gap_2().children(health.nodes.iter().map(|node| {
            let colors = if node.ready { (theme.primary.opacity(0.15), theme.primary) } else { (danger.opacity(0.15), danger) };
            div()
                .id(SharedString::from(format!("k8s-node-{}", node.name)))
                .aria_label(format!("{} · {}", node.name, node.kubelet_version))
                .child(pill(format!("{} · {}", node.name, node.kubelet_version), colors.0, colors.1))
                .test_support()
        }));
        let stores = health.datastores.iter().map(|store| {
            let ready = cac_core::text_with(language, "common:servers.readyOf", &[("ready", &store.ready.to_string()), ("total", &store.instances.to_string())]);
            div()
                .id(SharedString::from(format!("k8s-store-{}-{}", store.namespace, store.name)))
                .aria_label(store.name.clone())
                .flex()
                .items_center()
                .gap_2()
                .p_2p5()
                .rounded(px(8.))
                .border_1()
                .border_color(if store.healthy { border } else { danger.opacity(0.5) })
                .when(!store.healthy, |row| row.bg(danger.opacity(0.05)))
                .leading_sm()
                .child(Icon::new(Lucide::Database).size_4().text_color(muted))
                .child(div().font_weight(FontWeight::MEDIUM).child(store.name.clone()))
                .child(div().text_xs().text_color(muted).child(store.namespace.clone()))
                .child(div().flex_1())
                .child(div().text_xs().child(ready))
                .when(!store.healthy, |row| row.child(div().max_w(px(240.)).truncate().text_xs().text_color(danger).child(store.phase.clone())))
                .test_support()
        });
        sections.push(section(Lucide::Server, tr(cx, "common:misc.clusterHealth"), div().flex().flex_col().gap_3().child(nodes).child(div().flex().flex_col().gap_1p5().children(stores)).into_any_element(), cx));
        if !health.workloads.is_empty() {
            let tiles = workloads_failing_first(&health.workloads).into_iter().map(|workload| {
                div()
                    .id(SharedString::from(format!("k8s-workload-{}-{}", workload.namespace, workload.name)))
                    .aria_label(workload.name.clone())
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2p5()
                    .py_1p5()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if workload.healthy { border } else { danger.opacity(0.5) })
                    .when(!workload.healthy, |row| row.bg(danger.opacity(0.05)))
                    .text_xs()
                    .leading_xs()
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(workload.name.clone()))
                    .child(div().text_color(muted).child(workload.namespace.clone()))
                    .child(div().flex_1())
                    .child(div().when(!workload.healthy, |count| count.text_color(danger)).child(format!("{}/{}", workload.ready, workload.desired)))
                    .test_support()
                    .into_any_element()
            });
            sections.push(section(Lucide::Boxes, tr(cx, "common:misc.workloads"), grid("k8s-workloads", tiles.collect()), cx));
        }
        if !health.certs.is_empty() {
            let rows = health.certs.iter().map(|cert| {
                let warn = cert_warns(cert.ready, cert.days_left);
                div()
                    .id(SharedString::from(format!("k8s-cert-{}-{}", cert.namespace, cert.name)))
                    .aria_label(format!("{}{}", cert.name, if warn { " !" } else { "" }))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2p5()
                    .py_1p5()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if warn { danger.opacity(0.5) } else { border })
                    .when(warn, |row| row.bg(danger.opacity(0.05)))
                    .text_xs()
                    .leading_xs()
                    .child(Icon::new(if warn { Lucide::ShieldAlert } else { Lucide::ShieldCheck }).size_3p5().text_color(if warn { danger } else { success }))
                    .child(div().font_weight(FontWeight::MEDIUM).child(cert.name.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_color(muted).child(cert.dns_names.join(", ")))
                    .children(cert.days_left.map(|days| div().when(warn, |left| left.text_color(danger)).child(format!("{days}d"))))
                    .test_support()
            });
            sections.push(section(Lucide::ShieldCheck, tr(cx, "common:misc.certificates"), div().flex().flex_col().gap_1p5().children(rows).into_any_element(), cx));
        }
        sections
    }
}

impl Render for K8sConsole {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let refresh = Button::new("k8s-refresh")
            .outline()
            .small()
            .icon(Icon::new(Lucide::RefreshCw))
            .tooltip(tr(cx, "common:servers.refresh"))
            .disabled(state.k8s_loading)
            .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.platform().load_k8s().await }));
        let hub = state.k8s.clone().filter(|hub| Some(&hub.server_id) == state.open_server.as_ref());
        let problems: Vec<String> = hub.as_ref().and_then(|hub| hub.error.clone()).into_iter().chain(self.problem.clone()).collect();
        let body: Vec<AnyElement> = match hub.as_ref() {
            None => vec![div().id("k8s-loading").leading_sm().text_color(muted).child(tr(cx, "common:servers.readingCluster")).test_support().into_any_element()],
            Some(hub) => {
                let integrations = self.integrations(hub, state.permissions.reveal_integrations, cx);
                let mut all = vec![section(Lucide::LayoutGrid, tr(cx, "common:servers.integrations"), integrations, cx)];
                all.extend(self.routes(hub, cx));
                all.extend(self.health(hub, cx));
                all
            }
        };
        div()
            .id("k8s-console")
            .flex()
            .flex_col()
            .gap_6()
            .child(div().flex().justify_end().child(refresh))
            .children(problems.into_iter().map(|problem| {
                div().id("k8s-error").flex().items_center().gap_2().p_3().rounded(px(8.)).border_1().border_color(danger.opacity(0.4)).bg(danger.opacity(0.05)).leading_sm().text_color(danger).child(Icon::new(Lucide::CircleAlert).size_4()).child(problem).test_support()
            }))
            .children(body)
            .test_support()
    }
}

fn section(icon: Lucide, title: String, body: AnyElement, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().flex().items_center().gap_2().leading_sm().font_weight(FontWeight::MEDIUM).text_color(muted).child(Icon::new(icon).size_4()).child(title))
        .child(body)
        .into_any_element()
}

/// Dos por fila, como la rejilla de Tauri.
fn grid(id: &'static str, tiles: Vec<AnyElement>) -> AnyElement {
    let mut rows = Vec::new();
    let mut tiles = tiles.into_iter();
    while let Some(left) = tiles.next() {
        let right = tiles.next().map(|right| div().flex_1().min_w_0().child(right).into_any_element()).unwrap_or_else(|| div().flex_1().into_any_element());
        rows.push(div().flex().gap_2().child(div().flex_1().min_w_0().child(left)).child(right).into_any_element());
    }
    div().id(id).flex().flex_col().gap_2().children(rows).test_support().into_any_element()
}
