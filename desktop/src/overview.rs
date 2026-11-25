//! «Resumen»: con qué te encuentras al abrir la app (`pages/Overview.tsx`,
//! command-and-control-511).
//!
//! Qué cuenta cada tarjeta, en qué orden y con qué palabras lo decide el core
//! (`cac_core::overview_view`); aquí se pinta con las medidas de la página de
//! siempre y se le pasa cada pulsación.

use cac_core::{OverviewList, OverviewRow, UnreadRow};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::locale;
use crate::theme::Leading as _;

/// Lo mínimo de una tarjeta que pinta esta pantalla: el icono y su color.
struct Card {
    id: &'static str,
    icon: Lucide,
    tint: Hsla,
    title: String,
    count: i64,
    link: String,
}

/// La pantalla.
pub struct OverviewScreen;

impl OverviewScreen {
    pub fn new() -> Self {
        Self
    }

    /// `rounded-xl border bg-card` con su cabecera (`Tarjeta`).
    fn card(card: Card, grow: bool, on_link: impl Fn(&mut Window, &mut App) + 'static, body: AnyElement, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, primary) = (theme.border, theme.muted_foreground, theme.primary);
        let Card { id, icon, tint, title, count, link } = card;
        div()
            .id(id)
            .test_support()
            .when(grow, |this| this.flex_1().min_w(px(320.)))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .border_1()
            .border_color(border)
            .bg(theme.group_box)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(border)
                    .px(px(14.))
                    .py_3()
                    .child(Icon::new(icon).size_3p5().text_color(tint))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(div().id(SharedString::from(format!("{id}-count"))).leading_xs().text_color(muted).aria_label(format!("({count})")).child(format!("({count})")).test_support())
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-link")))
                            .ml_auto()
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(primary)
                            .cursor_pointer()
                            .hover(|style| style.underline())
                            .on_click(move |_, window, cx| on_link(window, cx))
                            .child(link)
                            .test_support(),
                    ),
            )
            .child(div().px_2().pb(px(10.)).pt(px(6.)).child(body))
            .into_any_element()
    }

    /// El texto de una tarjeta vacía.
    fn empty(id: String, text: String, cx: &App) -> AnyElement {
        div().id(SharedString::from(id)).px_2().py_3().leading_xs().text_color(cx.theme().muted_foreground).child(text).test_support().into_any_element()
    }

    /// Una pastilla de estado o de prioridad (`rounded border px-1.5 py-0.5
    /// text-[10px] text-muted-foreground`).
    fn chip(text: String, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex_none()
            .rounded(px(4.))
            .border_1()
            .border_color(theme.border)
            .px_1p5()
            .py_0p5()
            .text_size(px(10.))
            .line_height(px(14.))
            .text_color(theme.muted_foreground)
            .child(text)
    }

    /// Una fila de reportes o de tareas (`Fila`).
    fn row(prefix: &str, row: &OverviewRow, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (muted, hover) = (theme.muted_foreground, theme.accent.opacity(0.4));
        let id = row.id.clone();
        div()
            .id(SharedString::from(format!("{prefix}-{}", row.id)))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(8.))
            .px_2()
            .py_1p5()
            .leading_sm()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .aria_label(row.title.clone())
            .on_click(move |_, _, cx| {
                let id = id.clone();
                Backend::run(cx, move |core| async move {
                    core.overview().open_item(&id).await;
                });
            })
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(div().truncate().child(row.title.clone()))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .line_height(px(16.))
                            .text_color(muted)
                            .when(row.mono, |this| this.font_family(cx.theme().mono_font_family.clone()))
                            .child(row.meta.clone()),
                    ),
            )
            .child(Self::chip(row.status.clone(), cx))
            .child(Self::chip(row.priority.clone(), cx))
            .test_support()
            .into_any_element()
    }

    fn list_body(prefix: &str, list: &OverviewList, cx: &App) -> AnyElement {
        if list.rows.is_empty() {
            return Self::empty(format!("{prefix}-empty"), list.empty.clone(), cx);
        }
        div().flex().flex_col().children(list.rows.iter().map(|row| Self::row(&format!("{prefix}-row"), row, cx))).into_any_element()
    }

    /// Una conversación con gente esperando.
    fn unread_row(row: &UnreadRow, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (muted, hover, primary, on_primary, chip) = (theme.muted_foreground, theme.accent.opacity(0.4), theme.primary, theme.primary_foreground, theme.muted);
        let conversation = row.conversation.clone();
        div()
            .id(SharedString::from(format!("overview-unread-{}", row.key)))
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(8.))
            .px_2()
            .py_1p5()
            .leading_sm()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .aria_label(row.place.clone())
            .on_click(move |_, _, cx| {
                let conversation = conversation.clone();
                Backend::run(cx, move |core| async move {
                    core.overview().open_conversation(conversation).await;
                });
            })
            .child(
                div()
                    .size_6()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(chip)
                    .text_size(px(10.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(row.initials.clone()),
            )
            .child(div().min_w_0().flex_1().truncate().child(row.place.clone()))
            .when(!row.when.is_empty(), |this| this.child(div().flex_none().text_size(px(11.)).line_height(px(16.)).text_color(muted).child(row.when.clone())))
            .child(
                div()
                    .id(SharedString::from(format!("overview-unread-{}-count", row.key)))
                    .flex_none()
                    .rounded_full()
                    .bg(primary)
                    .px_1p5()
                    .text_size(px(10.))
                    .line_height(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(on_primary)
                    .aria_label(row.count.clone())
                    .child(row.count.clone())
                    .test_support(),
            )
            .test_support()
            .into_any_element()
    }
}

impl Render for OverviewScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = Backend::snapshot(cx).read(cx);
        let org_name = snapshot.orgs.current().map(|org| org.name.clone()).unwrap_or_default();
        let view = cac_core::overview_view(
            &Backend::core(cx).overview().state(),
            &snapshot.comms,
            &snapshot.platform.servers,
            &org_name,
            chrono::Local::now().date_naive(),
            &chrono::Local::now(),
            locale(cx),
        );
        let theme = cx.theme();
        let (background, foreground, muted, warning, primary, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.warning, theme.primary, theme.danger);

        let reports = Self::card(
            Card { id: "overview-reports", icon: Lucide::CalendarClock, tint: warning, title: view.reports.title.clone(), count: view.reports.count, link: view.reports.link.clone() },
            true,
            |_, cx| Backend::core(cx).show_all_work(),
            Self::list_body("overview-reports", &view.reports, cx),
            cx,
        );
        let tasks = Self::card(
            Card { id: "overview-tasks", icon: Lucide::SquareCheck, tint: primary, title: view.tasks.title.clone(), count: view.tasks.count, link: view.tasks.link.clone() },
            true,
            |_, cx| Backend::core(cx).show_all_work(),
            Self::list_body("overview-tasks", &view.tasks, cx),
            cx,
        );
        let unread_body = if view.unread.rows.is_empty() {
            Self::empty("overview-unread-empty".into(), view.unread.empty.clone(), cx)
        } else {
            div().flex().flex_col().children(view.unread.rows.iter().map(|row| Self::unread_row(row, cx))).into_any_element()
        };
        let unread = Self::card(
            Card { id: "overview-unread", icon: Lucide::MessageSquare, tint: danger, title: view.unread.title.clone(), count: view.unread.count, link: view.unread.link.clone() },
            false,
            |_, cx| Backend::core(cx).comms().enter(),
            unread_body,
            cx,
        );
        let infra_body = if view.infra.count == 0 {
            Self::empty("overview-infra-empty".into(), view.infra.empty.clone(), cx)
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap_x_6()
                .gap_y_2()
                .px_2()
                .py_1p5()
                .leading_sm()
                .children(view.infra.figures.iter().enumerate().map(|(index, figure)| {
                    div()
                        .id(SharedString::from(format!("overview-figure-{index}")))
                        .flex()
                        .items_baseline()
                        .gap_1p5()
                        .aria_label(format!("{} {}", figure.value, figure.label))
                        .child(div().text_size(px(18.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).child(figure.value.clone()))
                        .child(div().leading_xs().text_color(muted).child(figure.label.clone()))
                        .test_support()
                }))
                .into_any_element()
        };
        let infra = Self::card(
            Card { id: "overview-infra", icon: Lucide::Server, tint: muted, title: view.infra.title.clone(), count: view.infra.count, link: view.infra.link.clone() },
            false,
            |_, cx| {
                Backend::core(cx).platform().enter();
                Backend::run(cx, |core| async move {
                    core.platform().start_stats();
                    core.platform().refresh().await
                });
            },
            infra_body,
            cx,
        );

        div()
            .id("overview")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .bg(background)
            .text_color(foreground)
            .px_6()
            .py_5()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(10.))
                            .child(div().id("overview-title").aria_label(view.title.clone()).text_size(px(19.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).child(view.title.clone()).test_support())
                            .child(div().id("overview-subtitle").aria_label(view.subtitle.clone()).leading_xs().text_color(muted).child(view.subtitle.clone()).test_support()),
                    )
                    .child(div().flex().flex_wrap().gap_3().child(reports).child(tasks))
                    .child(unread)
                    .child(infra),
            )
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::Mock;

    fn get(route: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET")).and(path(route.to_string()))
    }

    fn task(id: &str) -> Value {
        json!({
            "id": id, "seq": 1, "title": format!("t-{id}"), "priority": "normal", "status": "in_progress", "statusName": "En curso", "statusKind": "active",
            "listId": "l1", "listName": "Sprint", "spaceId": "g", "spaceName": "App", "updatedAt": "2026-11-25T10:00:00Z"
        })
    }

    fn server() -> FakeServer {
        let server = FakeServer::start();
        let org = json!({ "id": "o1", "name": "G Studio", "slug": "o1", "role": "member", "memberCount": 3, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org]))));
        let general = json!({ "id": "g", "orgId": "o1", "name": "General", "color": "#888", "kind": "general", "folders": [], "lists": [], "people": [] });
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([general]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").and(query_param("limit", "50")).respond_with(ok(json!([task("t1"), task("t2"), task("t3")]))).with_priority(1));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        let report = |id: &str, status: &str| json!({ "id": id, "title": format!("r-{id}"), "folio": "DW-1", "projectName": "Portal", "status": status, "priority": "high" });
        server.mount(get("/api/v1/reports/").respond_with(ok(json!({ "items": [report("r1", "open"), report("r2", "done")], "total": 2 }))));
        server.mount(get("/api/v1/chat/unread").respond_with(ok(json!([{ "spaceId": "g", "count": 4 }]))));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        let dm = json!({ "conversationId": "c1", "orgId": "o1", "userId": "u2", "username": "luis", "unread": 2, "lastMessageAt": "2026-01-05T15:00:00Z" });
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([dm]))));
        let server_row = |id: &str, status: &str, kind: &str| json!({ "id": id, "orgId": "o1", "name": id, "host": "h", "sshPort": 22, "sshUser": "u", "type": kind, "agentPort": 0, "status": status, "hasAgentToken": true });
        server.mount(get("/api/v1/servers/").respond_with(ok(json!([server_row("s1", "online", "docker-swarm"), server_row("s2", "offline", "kubernetes")]))));
        server
    }

    fn open(cx: &mut TestAppContext) -> AnyWindowHandle {
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(900.)) })),
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

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El resumen pone delante sus cuatro tarjetas con lo de la organización,
    /// y sus enlaces llevan a donde dicen.
    ///
    /// Roturas que atrapa: una fila del panel que no lleva a nada; «Mi
    /// trabajo» marcado a la vez; contar un reporte hecho como abierto; lo
    /// sin leer sin los directos o sin los canales; las cifras de los
    /// servidores; un enlace que no cambia de pantalla.
    #[gpui_kit::test]
    fn the_overview_shows_four_cards_and_its_links_go_somewhere(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "panel-overview", cx));
        click(window, "panel-overview", cx);
        wait_until(cx, "llegan las tarjetas", |cx| label(window, "overview-infra-count", cx) == "(2)" && label(window, "overview-tasks-count", cx) == "(3)");
        assert_eq!(label(window, "overview-title", cx), "Resumen");
        assert!(label(window, "overview-subtitle", cx).starts_with("G Studio · "));
        assert_eq!(label(window, "overview-reports-count", cx), "(1)", "el reporte hecho no cuenta");
        assert!(on_screen(window, "overview-reports-row-r1", cx) && !on_screen(window, "overview-reports-row-r2", cx));
        assert!(on_screen(window, "overview-tasks-row-t1", cx));
        assert_eq!(label(window, "overview-unread-count", cx), "(6)");
        assert_eq!(label(window, "overview-unread-d-c1-count", cx), "2");
        assert_eq!(label(window, "overview-unread-c-g-count", cx), "4");
        assert_eq!(label(window, "overview-figure-1", cx), "1/2 en línea");
        assert!(!on_screen(window, "work-empty", cx), "no es «Mi trabajo»");

        click(window, "overview-unread-link", cx);
        wait_until(cx, "Comunicación", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).comms.on_screen) && !on_screen(window, "overview", cx));
        click(window, "rail-work", cx);
        wait_until(cx, "vuelve el árbol", |cx| on_screen(window, "panel-overview", cx));
        assert!(!on_screen(window, "overview", cx), "comunicación lo dejó atrás");
        click(window, "panel-overview", cx);
        wait_until(cx, "vuelve el resumen", |cx| on_screen(window, "overview", cx));
        click(window, "overview-tasks-link", cx);
        wait_until(cx, "Mi trabajo", |cx| !on_screen(window, "overview", cx));
        stop(cx);
    }
}
