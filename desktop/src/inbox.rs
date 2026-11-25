//! El panel de la campana: lo que pasó mientras no mirabas
//! (`NotificationsPanel.tsx`).
//!
//! Cuelga de la campana, arriba a la derecha, y no es un cajón de altura
//! completa: se consulta de pasada —¿me habló alguien?— y un panel que tapa
//! la pantalla obliga a cerrarlo para volver a lo que se hacía. El fondo se
//! oscurece y cierra al pulsarlo.
//!
//! Qué se agrupa con qué, en qué orden y qué dice cada fila lo decide el core
//! (`cac_core::inbox_panel`); aquí se pinta y se pulsa. Lo único propio de
//! esta pantalla es qué pestaña se mira y qué conversaciones se abrieron.

use std::collections::HashSet;

use cac_core::{InboxGroup, InboxIcon, InboxRow, InboxTab, InboxTarget, InboxTone, INBOX_TABS};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;
use crate::workspace::Workspace;

/// Lo que el panel recuerda mientras está abierto.
#[derive(Default)]
pub(crate) struct InboxPanelState {
    pub(crate) open: bool,
    tab: InboxTab,
    /// Las conversaciones desplegadas, por su clave: la bandeja se vuelve a
    /// pedir con cada evento, y cualquier otra llave cerraría la que se
    /// acaba de abrir.
    expanded: HashSet<String>,
    /// Lo que se dice tras pulsar algo que esta app todavía no tiene.
    said: Option<String>,
}

fn icon_of(icon: InboxIcon) -> Lucide {
    match icon {
        InboxIcon::Mention => Lucide::AtSign,
        InboxIcon::Direct => Lucide::MessageSquare,
        InboxIcon::Channel => Lucide::Hash,
        InboxIcon::Post => Lucide::Megaphone,
        InboxIcon::Task => Lucide::SquareCheck,
        InboxIcon::Assigned => Lucide::UserPlus,
        InboxIcon::Report => Lucide::Zap,
        InboxIcon::Review => Lucide::BellRing,
        InboxIcon::Agent => Lucide::Bot,
        InboxIcon::Meeting => Lucide::CalendarClock,
        InboxIcon::Ci => Lucide::Workflow,
        InboxIcon::Deploy => Lucide::Rocket,
        InboxIcon::Telemetry => Lucide::Activity,
        InboxIcon::Backup => Lucide::Archive,
        InboxIcon::Unknown => Lucide::Info,
    }
}

fn color_of(tone: InboxTone, cx: &App) -> Hsla {
    let theme = cx.theme();
    match tone {
        InboxTone::Primary => theme.primary,
        InboxTone::Muted => theme.muted_foreground,
        InboxTone::Foreground => theme.foreground,
        InboxTone::Warning => theme.warning,
        InboxTone::Danger => theme.danger,
    }
}

impl Workspace {
    /// Abre o cierra el panel. Al abrirlo se pide la bandeja: hasta entonces
    /// la campana sólo sabía cuántos.
    pub(crate) fn toggle_inbox(&mut self, cx: &mut Context<Self>) {
        self.inbox.open = !self.inbox.open;
        self.inbox.said = None;
        if self.inbox.open {
            Backend::run(cx, |core| async move { core.inbox().load().await });
        }
        cx.notify();
    }

    /// Lo cierra; Escape también (lo oye `workspace.rs`, que tiene el foco).
    pub(crate) fn close_inbox(&mut self, cx: &mut Context<Self>) {
        self.inbox.open = false;
        cx.notify();
    }

    /// Pulsar un aviso o una conversación: leída, y a su sitio. Si es un
    /// sitio que esta app todavía no tiene, se queda en el panel y lo dice.
    fn open_from_inbox(&mut self, read: InboxRead, link: String, org_id: String, cx: &mut Context<Self>) {
        let here = cac_core::link_target(&link) != InboxTarget::NotHere;
        Backend::run(cx, move |core| async move {
            let _ = match read {
                InboxRead::One(id) => core.inbox().mark_read(vec![id]).await,
                InboxRead::Group(key) => core.inbox().mark_group_read(key).await,
            };
            if here {
                core.inbox().open(&link, &org_id).await;
            }
        });
        if here {
            self.close_inbox(cx);
        } else {
            self.inbox.said = Some(tr(cx, "common:misc.notInThisApp"));
            cx.notify();
        }
    }

    pub(crate) fn render_inbox(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.inbox.open {
            return None;
        }
        let language = locale(cx);
        let snapshot = Backend::snapshot(cx).read(cx);
        let panel = cac_core::inbox_panel(&snapshot.inbox, self.inbox.tab, &self.inbox.expanded, language, &chrono::Utc::now());
        let sound = Backend::core(cx).notify_sound();
        let theme = cx.theme();
        let (border, card, muted, primary, on_primary, accent, foreground) =
            (theme.border, theme.popover, theme.muted_foreground, theme.primary, theme.primary_foreground, theme.accent, theme.foreground);

        let header = div()
            .flex()
            .items_center()
            .gap_2p5()
            .px_3p5()
            .py_2p5()
            .border_b_1()
            .border_color(border)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "notifications:title")))
            .children(panel.badge.clone().map(|badge| {
                div()
                    .id("inbox-badge")
                    .rounded_full()
                    .bg(primary)
                    .px_1p5()
                    .text_size(px(10.))
                    .line_height(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(on_primary)
                    .aria_label(badge.clone())
                    .child(badge)
                    .test_support()
            }))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .text_size(px(11.5))
                    .text_color(muted)
                    .when(panel.can_mark_all, |this| {
                        let label = tr(cx, "notifications:markAllRead");
                        this.child(
                            div()
                                .id("inbox-mark-all")
                                .cursor_pointer()
                                .hover(move |style| style.text_color(foreground))
                                .aria_label(label.clone())
                                .child(label)
                                .on_click(|_, _, cx| Backend::run(cx, |core| async move { let _ = core.inbox().mark_all_read().await; }))
                                .test_support(),
                        )
                    })
                    // El sonido de los avisos, en este equipo. Vive aquí porque
                    // es lo único de las preferencias de avisos que esta app
                    // tiene.
                    .child({
                        let hint = tr(cx, "notifications:prefs.sound");
                        div()
                            .id("inbox-sound")
                            .cursor_pointer()
                            .hover(move |style| style.text_color(foreground))
                            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                            .aria_label(tr(cx, "notifications:prefs.sound"))
                            .aria_selected(sound)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let _ = Backend::core(cx).set_notify_sound(!sound);
                                cx.notify();
                            }))
                            .child(Icon::new(if sound { Lucide::Volume2 } else { Lucide::VolumeX }).size_3p5())
                            .test_support()
                    }),
            );

        let tabs = div().flex().gap_0p5().px_2p5().py_2().border_b_1().border_color(border).children(INBOX_TABS.into_iter().map(|tab| {
            let chosen = tab == self.inbox.tab;
            let label = tab.label(language);
            div()
                .id(SharedString::from(format!("inbox-tab-{tab:?}").to_lowercase()))
                .cursor_pointer()
                .rounded_md()
                .px_2p5()
                .py_1()
                .leading_xs()
                .map(|this| if chosen { this.bg(accent).text_color(foreground) } else { this.text_color(muted) })
                .aria_label(label.clone())
                .aria_selected(chosen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.inbox.tab = tab;
                    cx.notify();
                }))
                .child(label)
                .test_support()
        }));

        let mut list = div().id("inbox-list").flex_1().min_h_0().overflow_y_scroll().px_2().pb_2p5().pt_1p5().flex().flex_col();
        if panel.empty {
            let said = tr(cx, "notifications:empty");
            list = list.child(div().id("inbox-empty").px_2p5().py_5().text_center().leading_xs().text_color(muted).aria_label(said.clone()).child(said).test_support());
        }
        for group in &panel.unread {
            list = list.child(self.render_inbox_group(group, false, cx));
        }
        if !panel.read.is_empty() {
            let said = tr(cx, "notifications:read");
            list = list.child(div().id("inbox-read").px_2().pb_0p5().pt_2().text_size(px(10.5)).text_color(muted).aria_label(said.clone()).child(said.to_uppercase()).test_support());
            for group in &panel.read {
                list = list.child(self.render_inbox_group(group, true, cx));
            }
        }

        let said = self.inbox.said.clone().map(|said| {
            div().id("inbox-said").px_3p5().py_2().border_t_1().border_color(border).leading_xs().text_color(muted).aria_label(said.clone()).child(said).test_support()
        });
        let footer = div()
            .id("inbox-footer")
            .flex()
            .items_start()
            .gap_2p5()
            .border_t_1()
            .border_color(border)
            .px_3p5()
            .py_2p5()
            .text_size(px(11.5))
            .text_color(muted)
            .child(Icon::new(Lucide::Info).size_3p5().flex_none().text_color(primary))
            .child(tr(cx, "notifications:footer"));

        Some(
            div()
                .id("inbox")
                .absolute()
                .inset_0()
                .occlude()
                .child(
                    div()
                        .id("inbox-backdrop")
                        .absolute()
                        .inset_0()
                        .bg(gpui_kit::black().opacity(0.35))
                        .on_click(cx.listener(|this, _, _, cx| this.close_inbox(cx)))
                        .test_support(),
                )
                .child(
                    div()
                        .id("inbox-panel")
                        .absolute()
                        .right(px(12.))
                        .top(px(44.))
                        .bottom(px(12.))
                        .w(px(392.))
                        .flex()
                        .flex_col()
                        .justify_start()
                        .child(
                            div()
                                .id("inbox-card")
                                // Lo que se pulsa dentro no le llega al fondo
                                // de detrás, que cerraría el panel.
                                .occlude()
                                .max_h_full()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(border)
                                .bg(card)
                                .shadow_2xl()
                                .child(header)
                                .child(tabs)
                                .child(list)
                                .children(said)
                                .child(footer),
                        )
                        .test_support(),
                )
                .test_support()
                .into_any_element(),
        )
    }

    fn render_inbox_group(&self, group: &InboxGroup, read: bool, cx: &mut Context<Self>) -> AnyElement {
        if group.alone {
            return self.render_inbox_row(&group.rows[0], cx);
        }
        let theme = cx.theme();
        let (muted, primary, hover, surface) = (theme.muted_foreground, theme.primary, theme.accent.opacity(0.6), theme.muted);
        let key = group.key.clone();
        let (link, org) = (group.link.clone(), group.org_id.clone());
        let expanded = group.expanded;
        let toggle_label = cac_core::text_with(locale(cx), if expanded { "notifications:collapse" } else { "notifications:expand" }, &[("name", &group.title)]);
        let toggle = {
            let key = key.clone();
            div()
                .id(SharedString::from(format!("inbox-toggle-{key}")))
                .mt_2()
                .size_5()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .text_color(muted)
                .aria_label(toggle_label)
                .aria_expanded(expanded)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.inbox.expanded.remove(&key) {
                        this.inbox.expanded.insert(key.clone());
                    }
                    cx.notify();
                }))
                .child(Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }).size_3p5())
                .test_support()
        };
        let summary = {
            let key = key.clone();
            div()
                .id(SharedString::from(format!("inbox-group-{key}")))
                .flex_1()
                .min_w_0()
                .flex()
                .items_start()
                .gap_2()
                .py_2()
                .pr_2()
                .cursor_pointer()
                .aria_label(format!("{} · {}", group.title, group.detail))
                .on_click(cx.listener(move |this, _, _, cx| this.open_from_inbox(InboxRead::Group(key.clone()), link.clone(), org.clone(), cx)))
                .child(Icon::new(icon_of(group.icon)).size_4().mt_0p5().flex_none().text_color(color_of(group.tone, cx)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .child(div().min_w_0().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(group.title.clone()))
                                .child(
                                    div()
                                        .id(SharedString::from(format!("inbox-count-{}", group.key)))
                                        .flex_none()
                                        .rounded_full()
                                        .bg(primary.opacity(0.15))
                                        .px_1p5()
                                        .text_size(px(10.))
                                        .line_height(px(16.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(primary)
                                        .aria_label(group.count.to_string())
                                        .child(group.count.to_string())
                                        .test_support(),
                                )
                                .children(group.tag.clone().map(|tag| div().flex_none().rounded_sm().bg(surface).px_1().text_size(px(10.)).line_height(px(16.)).text_color(muted).child(tag)))
                                .when(group.agent, |this| {
                                    this.child(
                                        div()
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .gap_0p5()
                                            .rounded_sm()
                                            .bg(surface)
                                            .px_1()
                                            .text_size(px(10.))
                                            .text_color(muted)
                                            .child(Icon::new(Lucide::Bot).size_2p5())
                                            .child(tr(cx, "notifications:agent")),
                                    )
                                })
                                .child(div().ml_auto().flex_none().text_size(px(10.5)).text_color(muted).child(group.when.clone())),
                        )
                        .child(div().mt_0p5().truncate().leading_xs().text_color(muted).child(group.detail.clone())),
                )
                .child(div().mt_1p5().size_2().flex_none().rounded_full().when(!read, |this| this.bg(primary)))
                .test_support()
        };
        let rows = expanded.then(|| {
            div()
                .ml_6()
                .pl_1()
                .border_l_1()
                .border_color(cx.theme().border)
                .children(group.rows.iter().map(|row| self.render_inbox_row(row, cx)))
                .children(group.showing.clone().map(|said| div().id("inbox-showing").px_2().py_1().text_size(px(10.5)).text_color(muted).aria_label(said.clone()).child(said).test_support()))
        });
        div()
            .when(read, |this| this.opacity(0.55))
            .child(div().flex().items_start().gap_1().rounded(px(8.)).hover(move |style| style.bg(hover)).child(toggle).child(summary))
            .children(rows)
            .into_any_element()
    }

    fn render_inbox_row(&self, row: &InboxRow, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, primary, accent, border) = (theme.muted_foreground, theme.primary, theme.accent, theme.border);
        let (id, link, org) = (row.id.clone(), row.link.clone(), row.org_id.clone());
        let unread = row.unread;
        div()
            .id(SharedString::from(format!("inbox-row-{}", row.id)))
            .w_full()
            .flex()
            .items_center()
            .gap_2p5()
            .rounded(px(8.))
            .p_2()
            .cursor_pointer()
            .hover(move |style| style.bg(accent.opacity(0.4)))
            .when(!unread, |this| this.opacity(0.55))
            .aria_label(row.title.clone())
            .aria_selected(unread)
            .on_click(cx.listener(move |this, _, _, cx| this.open_from_inbox(InboxRead::One(id.clone()), link.clone(), org.clone(), cx)))
            .child(
                div()
                    .size(px(26.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .bg(accent)
                    .child(Icon::new(icon_of(row.icon)).size_3p5().text_color(color_of(row.tone, cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_1p5()
                            .child(div().min_w_0().truncate().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).child(row.title.clone()))
                            .children(row.tag.clone().map(|tag| div().flex_none().rounded_sm().border_1().border_color(border).px_1().text_size(px(10.)).text_color(muted).child(tag)))
                            .when(row.agent, |this| {
                                let hint = tr(cx, "notifications:byAgent");
                                this.child(
                                    div()
                                        .id(SharedString::from(format!("inbox-agent-{}", row.id)))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap_0p5()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(primary.opacity(0.4))
                                        .px_1()
                                        .text_size(px(10.))
                                        .text_color(primary)
                                        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                                        .child(Icon::new(Lucide::Bot).size_2p5())
                                        .child(tr(cx, "notifications:agent")),
                                )
                            })
                            .child(div().ml_auto().flex_none().text_size(px(10.5)).text_color(muted).child(row.when.clone())),
                    )
                    .when(!row.body.is_empty(), |this| this.child(div().mt_0p5().leading_xs().text_color(muted).child(row.body.clone()))),
            )
            .child(div().size_1p5().flex_none().rounded_full().when(unread, |this| this.bg(primary)))
            .test_support()
            .into_any_element()
    }
}

/// Qué se marca leído al pulsar: un aviso, o una conversación entera.
enum InboxRead {
    One(String),
    Group(String),
}

#[cfg(test)]
mod tests {
    use crate::backend::Backend;
    use crate::testing::{ok, server_with_tasks, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use cac_core::Conversation;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::Mock;

    fn get(route: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET")).and(path(route.to_string()))
    }

    fn notice(id: &str, kind: &str, link: &str, at: &str, group: Option<&str>) -> Value {
        json!({ "id": id, "orgId": "o1", "kind": kind, "title": format!("título {id}"), "body": format!("cuerpo {id}"), "link": link,
                "createdAt": at, "groupKey": group, "groupLabel": group.map(|_| "#Espacio a") })
    }

    /// Una organización con dos espacios y una bandeja: dos avisos de `#Espacio
    /// a` (de los cuarenta que cuenta el servidor), uno del muro (que esta app
    /// no tiene) y uno de una tarea, ya leído.
    fn server(items: Value) -> FakeServer {
        let server = server_with_tasks(vec![]);
        let feed = json!({ "items": items, "unread": 41, "groups": [{ "key": "chat:a", "label": "#Espacio a", "total": 40, "unread": 40 }] });
        server.mount(get("/api/v1/notifications/").and(query_param("limit", "50")).respond_with(ok(feed)).with_priority(1));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([]))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/notifications/read")).respond_with(ok(json!(null))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/notifications/read-all")).respond_with(ok(json!(null))));
        server
    }

    fn full_inbox() -> Value {
        json!([
            notice("n1", "chat:message", "/chat?space=a", "2026-11-18T10:00:00Z", Some("chat:a")),
            notice("n2", "chat:mention", "/chat?space=a", "2026-11-18T11:00:00Z", Some("chat:a")),
            notice("n3", "post:feedback", "/servers/x/backups?target=b", "2026-11-18T09:00:00Z", None),
            json!({ "id": "n4", "orgId": "o1", "kind": "task:assigned", "title": "Te asignaron", "body": "Arreglar login", "link": "/tasks?task=t1",
                    "createdAt": "2026-11-17T09:00:00Z", "readAt": "2026-11-17T10:00:00Z" }),
        ])
    }

    fn open(cx: &mut TestAppContext) -> AnyWindowHandle {
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(760.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window").0
        })
    }

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn click(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    fn label(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).and_then(|found| found.label().map(str::to_string)).unwrap_or_default()
        })
        .unwrap()
    }

    fn posted(server: &FakeServer, route: &str) -> Vec<(String, Value)> {
        server
            .requests()
            .into_iter()
            .filter(|request| request.method.as_str() == "POST" && request.url.path() == route)
            .map(|request| (request.url.query().unwrap_or_default().to_string(), serde_json::from_slice(&request.body).unwrap_or(Value::Null)))
            .collect()
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    fn open_panel(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "notifications-bell", cx);
        wait_until(cx, "el panel enseña la bandeja", |cx| on_screen(window, "inbox-group-chat:a", cx));
    }

    /// La campana abre su panel con la bandeja plegada por conversación, la
    /// cuenta del servidor, lo leído aparte y sus pestañas; desplegar enseña
    /// los avisos de dentro y cuántos faltan.
    ///
    /// Roturas que atrapa: una campana que no abre nada; no pedir la bandeja
    /// al abrir; contar lo cargado en vez de lo del servidor; una pestaña que
    /// no filtra; desplegar sin efecto; perder de vista lo leído.
    #[gpui_kit::test]
    fn the_bell_opens_the_inbox_folded_by_conversation(cx: &mut TestAppContext) {
        let server = server(full_inbox());
        server.install(cx);
        let window = open(cx);
        open_panel(window, cx);

        assert_eq!(label(window, "inbox-badge", cx), "41");
        assert_eq!(label(window, "inbox-count-chat:a", cx), "40", "la cuenta del servidor");
        assert!(on_screen(window, "inbox-row-n3", cx), "un aviso suelto es una fila");
        assert!(on_screen(window, "inbox-read", cx) && on_screen(window, "inbox-row-n4", cx), "lo leído, aparte");
        assert!(!on_screen(window, "inbox-row-n1", cx), "la conversación, plegada");

        click(window, "inbox-toggle-chat:a", cx);
        wait_until(cx, "se despliega", |cx| on_screen(window, "inbox-row-n1", cx) && on_screen(window, "inbox-row-n2", cx));
        assert_eq!(label(window, "inbox-showing", cx), "Se ven 2 de 40. El resto sigue en la conversación.");

        click(window, "inbox-tab-tasks", cx);
        wait_until(cx, "la pestaña filtra", |cx| !on_screen(window, "inbox-group-chat:a", cx));
        assert!(on_screen(window, "inbox-row-n4", cx) && !on_screen(window, "inbox-row-n3", cx));
        stop(cx);
    }

    /// Pulsar una conversación la marca leída entera y lleva a ella; pulsar
    /// algo que esta app no tiene lo marca leído, se queda en el panel y lo
    /// dice.
    ///
    /// Roturas que atrapa: marcar una conversación por los ids que hay a mano;
    /// no llevar a ella; cerrar el panel y no ir a ningún sitio (un clic
    /// perdido); no marcar leído lo que no se puede abrir.
    #[gpui_kit::test]
    fn clicking_goes_where_it_says_or_says_why_not(cx: &mut TestAppContext) {
        let server = server(full_inbox());
        server.install(cx);
        let window = open(cx);
        open_panel(window, cx);

        click(window, "inbox-row-n3", cx);
        wait_until(cx, "dice que no está en esta app", |cx| label(window, "inbox-said", cx) == "Todavía no está en esta app");
        assert!(on_screen(window, "inbox", cx), "y el panel sigue abierto");
        wait_until(cx, "se marca leído", |_| posted(&server, "/api/v1/notifications/read").iter().any(|(_, body)| *body == json!({ "ids": ["n3"] })));

        click(window, "inbox-group-chat:a", cx);
        wait_until(cx, "el panel se cierra", |cx| !on_screen(window, "inbox", cx));
        wait_until(cx, "y lleva a la conversación", |cx| {
            cx.update(|cx| {
                let comms = &Backend::snapshot(cx).read(cx).comms;
                comms.on_screen && comms.selected == Some(Conversation::Channel { space_id: "a".into() })
            })
        });
        assert!(posted(&server, "/api/v1/notifications/read").iter().any(|(_, body)| *body == json!({ "group": "chat:a", "orgId": "o1" })));
        stop(cx);
    }

    /// «Marcar todo leído» lo marca en esta organización y desaparece; el
    /// altavoz apaga y enciende el sonido de los avisos; el fondo cierra.
    ///
    /// Roturas que atrapa: marcar todo sin la organización; un botón de
    /// marcar todo sin nada que marcar; un altavoz que no cambia la
    /// preferencia; un fondo que no cierra; Escape que no cierra.
    #[gpui_kit::test]
    fn mark_all_sound_and_closing(cx: &mut TestAppContext) {
        let server = server(full_inbox());
        server.install(cx);
        let window = open(cx);
        open_panel(window, cx);

        click(window, "inbox-mark-all", cx);
        wait_until(cx, "se pide", |_| !posted(&server, "/api/v1/notifications/read-all").is_empty());
        assert_eq!(posted(&server, "/api/v1/notifications/read-all")[0].0, "orgId=o1");
        wait_until(cx, "ya no hay nada que marcar", |cx| !on_screen(window, "inbox-mark-all", cx) && !on_screen(window, "inbox-badge", cx));

        assert!(cx.update(|cx| Backend::core(cx).notify_sound()));
        click(window, "inbox-sound", cx);
        assert!(!cx.update(|cx| Backend::core(cx).notify_sound()), "apagado");
        click(window, "inbox-sound", cx);
        assert!(cx.update(|cx| Backend::core(cx).notify_sound()), "y encendido otra vez");

        click(window, "inbox-backdrop", cx);
        wait_until(cx, "el fondo cierra", |cx| !on_screen(window, "inbox", cx));
        click(window, "notifications-bell", cx);
        wait_until(cx, "se vuelve a abrir", |cx| on_screen(window, "inbox", cx));
        cx.simulate_keystrokes(window, "escape");
        wait_until(cx, "Escape cierra", |cx| !on_screen(window, "inbox", cx));
        stop(cx);
    }

    /// La fila «Notificaciones» del menú de la cuenta abre el mismo panel y
    /// cierra el menú. Rotura que atrapa: una fila que sigue diciendo
    /// «todavía no».
    #[gpui_kit::test]
    fn the_account_menu_opens_the_same_panel(cx: &mut TestAppContext) {
        let server = server(full_inbox());
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "account-menu", cx);
        wait_until(cx, "el menú se abre", |cx| on_screen(window, "account-notifications", cx));
        click(window, "account-notifications", cx);
        wait_until(cx, "el panel se abre", |cx| on_screen(window, "inbox-group-chat:a", cx));
        assert!(!on_screen(window, "account-notifications", cx), "y el menú se cierra");
        stop(cx);
    }

    /// Sin nada, lo dice. Rotura que atrapa: un panel en blanco.
    #[gpui_kit::test]
    fn an_empty_inbox_says_so(cx: &mut TestAppContext) {
        let server = server_with_tasks(vec![]);
        server.mount(get("/api/v1/notifications/").and(query_param("limit", "50")).respond_with(ok(json!({ "items": [], "unread": 0, "groups": [] }))).with_priority(1));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "notifications-bell", cx);
        wait_until(cx, "dice que no hay nada", |cx| label(window, "inbox-empty", cx) == "Nada pendiente en esta organización.");
        assert!(!on_screen(window, "inbox-mark-all", cx));
        stop(cx);
    }
}
