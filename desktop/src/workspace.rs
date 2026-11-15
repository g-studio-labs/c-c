//! La app por dentro: el riel de módulos, el panel con la organización y el
//! árbol de espacios, y el sitio de la vista.
//!
//! Qué filas tiene el árbol, cuáles van abiertas, qué hace un clic y qué se
//! trae al entrar lo dice el core. Aquí se pinta y se le pasa cada gesto.

use cac_core::{Locale, NodeKind, ThemePreference, TreeRow};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::docs::DocsView;
use crate::lang::{locale, tr};
use crate::mywork::MyWorkView;
use crate::task::TaskView;

/// Las medidas de la app de siempre: el riel y el panel.
const RAIL_WIDTH: f32 = 48.;
const PANEL_WIDTH: f32 = 256.;
const ROW_HEIGHT: f32 = 28.;
/// Lo que entra una fila por cada nivel que cuelga.
const INDENT: f32 = 14.;

/// Casi sin estado propio: lo que pinta lo lee del core al pintarse, y GPUI
/// repinta una vista cuando cambia algo que leyó. No hace falta suscribirse.
pub struct Workspace {
    pub(crate) my_work: Entity<MyWorkView>,
    docs: Entity<DocsView>,
    task: Entity<TaskView>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Quien entra, entra a trabajar: organizaciones, árbol, tareas y el
        // stream de eventos.
        Backend::run(cx, |core| async move { core.open_workspace().await });
        Self { my_work: cx.new(|cx| MyWorkView::new(window, cx)), docs: cx.new(|_| DocsView), task: cx.new(|_| TaskView::new()) }
    }

    fn switch_org(org_id: String, cx: &App) {
        Backend::run(cx, move |core| async move {
            core.switch_org(&org_id).await;
        });
    }

    /// Elige el tema: el core lo recuerda y la app se viste ya.
    fn choose_theme(preference: ThemePreference, cx: &mut App) {
        if Backend::core(cx).set_theme(preference).is_ok() {
            crate::theme::apply_preference(cx);
        }
    }

    /// Elige el idioma (`None` es el del sistema). El core lo cambia aquí al
    /// momento y lo guarda en la cuenta; si el servidor no pudo, lo elegido
    /// se queda en esta máquina.
    fn choose_language(preference: Option<Locale>, cx: &App) {
        Backend::run(cx, move |core| async move {
            let _ = core.set_locale(preference).await;
        });
    }

    fn render_account(&self, name: String) -> impl IntoElement {
        Button::new("account-menu").ghost().small().label(name).dropdown_caret(true).dropdown_menu(|menu, _, cx| {
            let core = Backend::core(cx);
            let (theme, language) = (core.theme(), core.locale_preference());
            let here = locale(cx);
            let phrase = |key: &str| cac_core::text(here, key);

            let mut menu = menu.label(phrase("nav:account.theme"));
            for (preference, key) in [
                (ThemePreference::System, "nav:account.themeAuto"),
                (ThemePreference::Light, "nav:account.themeLight"),
                (ThemePreference::Dark, "nav:account.themeDark"),
            ] {
                menu = menu.item(
                    PopupMenuItem::new(phrase(key))
                        .checked(theme == preference)
                        .on_click(move |_, _, cx| Self::choose_theme(preference, cx)),
                );
            }
            menu = menu.separator().label(phrase("nav:account.language"));
            for (preference, key) in [
                (None, "common:language.system"),
                (Some(Locale::En), "common:language.en"),
                (Some(Locale::Es), "common:language.es"),
            ] {
                menu = menu.item(
                    PopupMenuItem::new(phrase(key))
                        .checked(language == preference)
                        .on_click(move |_, _, cx| Self::choose_language(preference, cx)),
                );
            }
            menu
        })
    }

    /// Qué hace el clic (acotar las tareas o abrir la documentación) lo
    /// decide el core con lo que se esté mirando.
    fn click_row(row: &TreeRow, cx: &App) {
        let id = row.id.clone();
        Backend::run(cx, move |core| async move {
            core.click_node(&id).await;
        });
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        div()
            .id("connection-banner")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(danger.opacity(0.3))
            .bg(danger.opacity(0.1))
            .text_sm()
            .child(Icon::new(Lucide::WifiOff).text_color(danger))
            .child(div().font_weight(FontWeight::MEDIUM).text_color(danger).child(tr(cx, "common:last.liveUpdatesStopped")))
            .child(div().flex_1())
            .child(
                Button::new("reconnect")
                    .outline()
                    .small()
                    .icon(Icon::new(Lucide::RefreshCw))
                    .label(tr(cx, "common:last.reconnect"))
                    .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.open_workspace().await })),
            )
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .w(px(RAIL_WIDTH))
            .h_full()
            .flex()
            .flex_none()
            .flex_col()
            .items_center()
            .gap_2()
            .py_3()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(div().text_sm().font_weight(FontWeight::BOLD).text_color(theme.primary).child("cac"))
            // El único módulo de la primera entrega.
            .child(
                div()
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(theme.sidebar_accent)
                    .child(Icon::new(IconName::Inbox).text_color(theme.primary)),
            )
    }

    fn render_org(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let orgs = Backend::snapshot(cx).read(cx).orgs.clone();
        let name = orgs.current().map(|org| org.name.clone()).unwrap_or_else(|| tr(cx, "nav:rail.organization"));
        let current = orgs.current_org_id.clone();
        Button::new("org-switch")
            .ghost()
            .w_full()
            .label(name)
            .dropdown_caret(orgs.orgs.len() > 1)
            .dropdown_menu(move |mut menu, _, _| {
                for org in &orgs.orgs {
                    let id = org.id.clone();
                    menu = menu.item(
                        PopupMenuItem::new(org.name.clone())
                            .checked(current.as_deref() == Some(org.id.as_str()))
                            .on_click(move |_, _, cx| Self::switch_org(id.clone(), cx)),
                    );
                }
                menu
            })
    }

    fn render_row(&self, row: TreeRow, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (hover, active_bg, muted, foreground) =
            (theme.sidebar_accent, theme.sidebar_accent, theme.muted_foreground, theme.sidebar_foreground);
        let chevron = if row.open { IconName::ChevronDown } else { IconName::ChevronRight };
        let toggle_id = row.id.clone();
        let clicked = row.clone();
        let count = (row.kind == NodeKind::List && row.open_count > 0).then(|| row.open_count.to_string());

        div()
            .id(SharedString::from(format!("tree-{}", row.id)))
            .test_support()
            .h(px(ROW_HEIGHT))
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .pl(px(8. + INDENT * row.depth as f32))
            .pr_2()
            .mx_1()
            .rounded_md()
            .text_sm()
            .text_color(if row.kind == NodeKind::List && !row.active { muted } else { foreground })
            .when(row.active, |this| this.bg(active_bg).font_weight(FontWeight::MEDIUM))
            .hover(move |style| style.bg(hover))
            .cursor_pointer()
            .on_click(move |_, _, cx| Self::click_row(&clicked, cx))
            .child(
                div()
                    .id(SharedString::from(format!("tree-toggle-{}", row.id)))
                    .test_support()
                    .size(px(16.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .when(row.expandable, |this| {
                        this.child(Icon::new(chevron).xsmall().text_color(muted)).on_click(move |_, _, cx| {
                            // El chevrón abre y cierra; no es un clic en la fila.
                            cx.stop_propagation();
                            Backend::core(cx).tree().toggle(&toggle_id);
                        })
                    }),
            )
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(row.name))
            .children(count.map(|count| div().flex_none().text_xs().text_color(muted).child(count)))
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (sidebar, border, muted, hover, foreground) =
            (theme.sidebar, theme.sidebar_border, theme.muted_foreground, theme.sidebar_accent, theme.sidebar_foreground);
        let snapshot = Backend::snapshot(cx).read(cx);
        let everything = snapshot.my_work.scope.is_none() && snapshot.docs.target.is_none();
        let session = snapshot.session.clone();
        let rows = Backend::core(cx).tree().rows();
        let name = session.as_ref().map(|s| s.name.clone().unwrap_or_else(|| s.username.clone())).unwrap_or_default();

        div()
            .w(px(PANEL_WIDTH))
            .h_full()
            .flex()
            .flex_none()
            .flex_col()
            .bg(sidebar)
            .text_color(foreground)
            .border_r_1()
            .border_color(border)
            .child(div().p_2().border_b_1().border_color(border).child(self.render_org(cx)))
            .child(
                div()
                    .id("tree-everything")
                    .test_support()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .mx_1()
                    .mt_2()
                    .rounded_md()
                    .text_sm()
                    .when(everything, |this| this.bg(hover).font_weight(FontWeight::MEDIUM))
                    .hover(move |style| style.bg(hover))
                    .cursor_pointer()
                    .on_click(|_, _, cx| Backend::core(cx).show_all_work())
                    .child(Icon::new(IconName::Inbox).small())
                    .child(tr(cx, "nav:item.myWork")),
            )
            .child(div().px_3().pt_3().pb_1().text_xs().text_color(muted).child(tr(cx, "nav:panel.spaces")))
            .child(
                div()
                    .id("tree")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows.into_iter().map(|row| self.render_row(row, cx))),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .border_t_1()
                    .border_color(border)
                    .child(div().flex_1().overflow_hidden().child(self.render_account(name)))
                    .child(
                        Button::new("log-out")
                            .ghost()
                            .small()
                            .icon(Icon::new(Lucide::LogOut))
                            .tooltip(tr(cx, "nav:account.logOut"))
                            .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.logout().await })),
                    ),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = Backend::snapshot(cx).read(cx);
        let lasting = snapshot.connection.lasting;
        // Con un documento abierto se lee documentación; si no, las tareas.
        let main: AnyView = if snapshot.docs.target.is_some() { self.docs.clone().into() } else { self.my_work.clone().into() };
        // La tarea abierta, sobre lo que se estuviera mirando.
        let task = snapshot.task.task_id.is_some().then(|| self.task.clone());
        div()
            .id("open")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .when(lasting, |this| this.child(self.render_banner(cx)))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .overflow_hidden()
                    .child(self.render_rail(cx))
                    .child(self.render_panel(cx))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .relative()
                            .overflow_hidden()
                            .child(main)
                            // Un cajón sobre la vista, no al lado: en una ventana
                            // estrecha, al lado dejaba la lista en nada. Lo que
                            // tapa no recibe el ratón.
                            .children(task.map(|task| {
                                div().absolute().top_0().right_0().bottom_0().max_w_full().occlude().shadow_lg().child(task)
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::Workspace;
    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer};
    use cac_core::ScopeKind;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::Mock;

    fn org(id: &str, name: &str) -> serde_json::Value {
        json!({
            "id": id, "name": name, "slug": id, "role": "member", "memberCount": 2,
            "createdAt": "2026-06-09T12:00:00Z", "timezone": "",
        })
    }

    fn space(id: &str, name: &str) -> serde_json::Value {
        json!({
            "id": id, "orgId": "o1", "name": name, "color": "#20D9E8",
            "folders": [{
                "id": format!("f-{id}"), "name": "Carpeta", "folders": [],
                "lists": [{ "id": format!("in-{id}"), "name": "En carpeta", "openCount": 2, "taskCount": 2 }],
            }],
            "lists": [{ "id": format!("l-{id}"), "name": "Pendientes", "openCount": 3, "taskCount": 5 }],
            "people": [],
        })
    }

    /// Una tarea como la manda el servidor: la del contrato del core, en
    /// otra lista.
    fn task(id: &str, list: &str) -> serde_json::Value {
        let mut task: serde_json::Value =
            serde_json::from_str(include_str!("../../core/tests/fixtures/open_task.json")).unwrap();
        task["id"] = json!(id);
        task["listId"] = json!(list);
        task["spaceId"] = json!("a");
        task
    }

    /// Un servidor con dos organizaciones; la primera con un espacio y dos
    /// tareas, la segunda con otro.
    fn server() -> FakeServer {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org("o1", "G Studio"), org("o2", "Cliente")]))));
        server.mount(
            get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(json!([space("a", "Producto")]))),
        );
        server.mount(
            get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "Soporte")]))),
        );
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(
            get("/api/v1/tasks/")
                .and(query_param("orgId", "o1"))
                .respond_with(ok(json!([task("1", "l-a"), task("2", "in-a")]))),
        );
        server.mount(get("/api/v1/tasks/").and(query_param("orgId", "o2")).respond_with(ok(json!([]))));
        server
    }

    fn open(cx: &mut TestAppContext) -> AnyWindowHandle {
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(760.)),
                })),
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

    fn scope(cx: &mut TestAppContext) -> Option<(ScopeKind, String)> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).my_work.scope.as_ref().map(|scope| (scope.kind, scope.id.clone())))
    }

    /// Entrar trae la organización y su árbol, y los pinta.
    ///
    /// Roturas que atrapa: una app que abre con el panel vacío hasta que se
    /// pulsa algo; pintar lo que hay dentro de una carpeta cerrada.
    #[gpui_kit::test]
    fn entering_paints_the_organization_and_its_tree(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);

        wait_until(cx, "el árbol llega a la pantalla", |cx| on_screen(window, "tree-l-a", cx));

        assert!(on_screen(window, "tree-a", cx) && on_screen(window, "tree-f-a", cx));
        assert!(!on_screen(window, "tree-in-a", cx), "se pinta lo de dentro de una carpeta cerrada");
        assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).orgs.current().map(|org| org.name.clone())).as_deref(), Some("G Studio"));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Pulsar un nodo acota «Mi trabajo» a él; el chevrón sólo abre y cierra;
    /// y «Mi trabajo» quita el filtro.
    ///
    /// Roturas que atrapa: un árbol que no hace nada al pulsarlo; que abrir
    /// una rama cambie de qué se está hablando; no poder volver a verlo todo.
    #[gpui_kit::test]
    fn clicking_the_tree_scopes_and_the_chevron_only_opens(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));

        click(window, "tree-toggle-f-a", cx);
        assert!(on_screen(window, "tree-in-a", cx), "el chevrón no abrió la carpeta");
        assert_eq!(scope(cx), None, "abrir una rama acotó la vista");

        click(window, "tree-in-a", cx);
        wait_until(cx, "acota a la lista", |cx| scope(cx) == Some((ScopeKind::List, "in-a".into())));
        wait_until(cx, "llegan sus tareas", |cx| cx.update(|cx| Backend::core(cx).my_work().visible().len()) == 1);

        click(window, "tree-a", cx);
        wait_until(cx, "acota al espacio", |cx| scope(cx) == Some((ScopeKind::Space, "a".into())));

        click(window, "tree-everything", cx);
        wait_until(cx, "quita el filtro", |cx| scope(cx).is_none());
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El servidor de arriba, que además contesta la documentación de
    /// cualquier nodo: resumen escrito, runbook vacío, y dos decisiones, una
    /// de ellas sin efecto.
    fn server_with_docs() -> FakeServer {
        let server = server();
        let doc = json!({
            "doc": null, "orgId": "o1", "attachments": [],
            "tabs": [{ "key": "overview", "body": "# Qué es\n\nUn párrafo." }, { "key": "runbook", "body": "" }],
            "decisions": [
                { "id": "d0", "title": "La vieja", "body": "", "decidedAt": "2026-09-30T12:00:00Z", "supersededById": "d1" },
                { "id": "d1", "title": "La nueva", "body": "Porque sí.", "decidedAt": "2026-10-30T12:00:00Z", "authorName": "Ana" },
            ],
        });
        server.mount(
            Mock::given(method("GET"))
                .and(wiremock::matchers::path_regex("^/api/v1/docs/(space|folder|list)/[^/]+$"))
                .respond_with(ok(doc)),
        );
        server
    }

    fn doc_open(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).docs.target.as_ref().map(|target| target.id.clone()))
    }

    /// Pulsar una carpeta abre su documentación en el sitio de la vista, y
    /// «Lista» devuelve a las tareas.
    ///
    /// Roturas que atrapa: una carpeta cuyo clic no hace nada; abrir el
    /// documento en el core y seguir pintando la lista; no tener cómo volver.
    #[gpui_kit::test]
    fn a_folder_opens_its_docs_and_the_list_brings_the_tasks_back(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-f-a", cx));
        assert!(on_screen(window, "view-title", cx) && !on_screen(window, "docs", cx));

        click(window, "tree-f-a", cx);
        wait_until(cx, "llega su documentación", |cx| on_screen(window, "doc-body", cx));
        assert!(!on_screen(window, "view-title", cx), "la lista sigue pintada debajo del documento");
        assert_eq!(scope(cx), None, "una carpeta no acota");

        click(window, "view-list", cx);
        wait_until(cx, "vuelven las tareas", |cx| on_screen(window, "view-title", cx));
        assert!(!on_screen(window, "docs", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Con un espacio acotado hay botón de documentación (372); leyendo, el
    /// árbol abre la del nodo que se pulse; y «Mi trabajo» saca de ahí.
    ///
    /// Roturas que atrapa: ofrecer la documentación sólo con una lista
    /// delante, o sin nada acotado; que un clic en el árbol saque de la
    /// documentación a quien la lee; pulsar «Mi trabajo» y seguir en un
    /// documento.
    #[gpui_kit::test]
    fn the_docs_button_opens_the_scoped_nodes_docs(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        assert!(!on_screen(window, "open-docs", cx), "ofrece documentación sin un nodo delante");

        click(window, "tree-a", cx);
        wait_until(cx, "acotado al espacio, ofrece su documentación", |cx| on_screen(window, "open-docs", cx));
        click(window, "open-docs", cx);
        wait_until(cx, "llega la del espacio", |cx| on_screen(window, "doc-body", cx));
        assert_eq!(doc_open(cx).as_deref(), Some("a"));

        click(window, "tree-l-a", cx);
        wait_until(cx, "abre la de la lista", |cx| doc_open(cx).as_deref() == Some("l-a"));
        assert!(on_screen(window, "docs", cx), "el clic en el árbol sacó de la documentación");

        click(window, "tree-everything", cx);
        wait_until(cx, "vuelve a «Mi trabajo» entero", |cx| on_screen(window, "view-title", cx));
        assert!(doc_open(cx).is_none() && scope(cx).is_none());
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Cada pestaña enseña lo suyo: el texto, «no hay nada» si está vacía, y
    /// en la de decisiones sólo las que siguen vigentes.
    ///
    /// Roturas que atrapa: pestañas que cambian de color y enseñan lo mismo;
    /// una pestaña vacía en blanco; el registro repetido bajo cada pestaña;
    /// una decisión sin efecto pintada como vigente.
    #[gpui_kit::test]
    fn each_tab_shows_its_own(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-f-a", cx));
        click(window, "tree-f-a", cx);
        wait_until(cx, "llega su documentación", |cx| on_screen(window, "doc-body", cx));
        assert!(!on_screen(window, "doc-empty", cx) && !on_screen(window, "doc-decision-d1", cx));

        click(window, "doc-tab-runbook", cx);
        wait_until(cx, "el runbook está vacío y lo dice", |cx| on_screen(window, "doc-empty", cx));
        assert!(!on_screen(window, "doc-body", cx), "sigue el texto de la otra pestaña");

        click(window, "doc-tab-decisions", cx);
        wait_until(cx, "sale la decisión vigente", |cx| on_screen(window, "doc-decision-d1", cx));
        assert!(!on_screen(window, "doc-decision-d0", cx), "pinta una decisión que otra dejó sin efecto");
        assert!(!on_screen(window, "doc-empty", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Cambiar de organización cambia el árbol entero.
    ///
    /// Rotura que atrapa: cambiar el nombre de arriba y seguir con el árbol
    /// de la organización anterior.
    #[gpui_kit::test]
    fn switching_organization_changes_the_tree(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));

        cx.update(|cx| Workspace::switch_org("o2".into(), cx));

        wait_until(cx, "llega el árbol de la otra", |cx| on_screen(window, "tree-z", cx));
        assert!(!on_screen(window, "tree-a", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Elegir tema viste la app en el acto y se recuerda para el arranque.
    ///
    /// Roturas que atrapa: un menú de tema que no cambia nada; cambiarlo sólo
    /// hasta cerrar la app; arrancar siempre en oscuro aunque se eligió claro.
    #[gpui_kit::test]
    fn choosing_a_theme_dresses_the_app_and_is_remembered(cx: &mut TestAppContext) {
        use cac_core::ThemePreference;
        use gpui_kit::component::ActiveTheme;
        let server = server();
        server.install(cx);
        let background = |cx: &mut TestAppContext| cx.update(|cx| cx.theme().background);
        let dark = background(cx);

        cx.update(|cx| Workspace::choose_theme(ThemePreference::Light, cx));

        let light = background(cx);
        assert_ne!(light, dark, "el tema no cambió");
        assert_eq!(cx.update(|cx| Backend::core(cx).theme()), ThemePreference::Light);

        // Como al arrancar: otro tema puesto, y se vuelve al elegido.
        cx.update(|cx| crate::theme::apply(gpui_kit::component::ThemeMode::Dark, cx));
        assert_eq!(background(cx), dark);
        cx.update(crate::theme::apply_preference);
        assert_eq!(background(cx), light);
    }

    /// Elegir idioma cambia los textos y se le dice al servidor.
    ///
    /// Roturas que atrapa: un menú de idioma que no hace nada; cambiar el
    /// idioma sólo en esta máquina.
    #[gpui_kit::test]
    fn choosing_a_language_changes_the_texts(cx: &mut TestAppContext) {
        use cac_core::Locale;
        let server = server();
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/auth/locale")).respond_with(ok(json!(null))));
        server.install(cx);
        sign_in(cx);
        assert_eq!(cx.update(|cx| crate::lang::tr(cx, "nav:item.myWork")), "Mi trabajo");

        cx.update(|cx| Workspace::choose_language(Some(Locale::En), cx));

        wait_until(cx, "los textos pasan a inglés", |cx| cx.update(|cx| crate::lang::tr(cx, "nav:item.myWork")) == "My work");
        wait_until(cx, "se le dice al servidor", |_| server.hits("/api/v1/auth/locale") == 1);
    }

    /// Un corte que dura se avisa, y «Reconectar» vuelve a pedirlo todo.
    ///
    /// Roturas que atrapa: una app que deja de ponerse al día sin decirlo;
    /// un aviso puesto desde el primer segundo; un botón de reconectar que
    /// no pide nada.
    #[gpui_kit::test]
    fn an_outage_that_lasts_is_announced(cx: &mut TestAppContext) {
        // Sin ruta de eventos: el stream se cae en cada intento.
        let server = server();
        server.install_with(cx, |config| config.stream_grace = Duration::from_millis(400));
        let window = open(cx);
        // Nada más abrir, sin esperar a nada: el corte todavía no puede durar.
        // (Esperar aquí al árbol hacía la prueba depender de lo cargada que
        // estuviera la máquina.)
        assert!(!on_screen(window, "connection-banner", cx), "avisa antes de que el corte dure");
        // Y el aviso sale exactamente cuando el core dice que dura.
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        let lasting = cx.update(|cx| Backend::snapshot(cx).read(cx).connection.lasting);
        assert_eq!(on_screen(window, "connection-banner", cx), lasting);

        wait_until(cx, "avisa del corte", |cx| on_screen(window, "connection-banner", cx));
        let asked = server.hits("/api/v1/organizations/");
        click(window, "reconnect", cx);
        wait_until(cx, "vuelve a pedirlo todo", |_| server.hits("/api/v1/organizations/") > asked);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }
}
