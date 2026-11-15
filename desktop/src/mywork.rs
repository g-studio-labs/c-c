//! «Mi trabajo» en lista: la cabecera (ámbito, lentes, sólo lo abierto,
//! buscador) y las tareas agrupadas por espacio.
//!
//! Qué tareas hay, cómo se agrupan, cómo se dice un vencimiento y qué pide
//! cada lente lo decide el core. Aquí está el buscador, que es lo único que
//! esta pantalla tiene propio, y la pintura.

use cac_core::{DueState, ScopeKind, TaskFilter, WorkGroup, WorkLens, WorkRow, NOBODY};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};

/// Las lentes, en el orden de la app de siempre, con su nombre en el catálogo.
const LENSES: [(WorkLens, &str); 5] = [
    (WorkLens::Assigned, "assigned"),
    (WorkLens::Created, "created"),
    (WorkLens::Watching, "watching"),
    (WorkLens::Clients, "clients"),
    (WorkLens::All, "all"),
];

pub struct MyWorkView {
    search: Entity<InputState>,
    /// Lo elegido en los desplegables; vacío es «cualquiera».
    assignee: String,
    priority: String,
}

impl MyWorkView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "work:board.filter.search");
        Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)),
            assignee: String::new(),
            priority: String::new(),
        }
    }

    fn filter(&self, cx: &App) -> TaskFilter {
        TaskFilter {
            query: self.search.read(cx).value().to_string(),
            assignee: self.assignee.clone(),
            priority: self.priority.clone(),
        }
    }

    pub(crate) fn choose_assignee(&mut self, assignee: String, cx: &mut Context<Self>) {
        self.assignee = assignee;
        cx.notify();
    }

    pub(crate) fn choose_priority(&mut self, priority: String, cx: &mut Context<Self>) {
        self.priority = priority;
        cx.notify();
    }

    /// Quita todo lo que filtra, también lo escrito en el buscador: un
    /// «quitar filtros» que deja la lista a medias no ha quitado los filtros.
    pub(crate) fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.assignee.clear();
        self.priority.clear();
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        cx.notify();
    }

    /// Los desplegables de filtro y, si algo filtra, cómo quitarlo. Qué se
    /// ofrece en cada uno lo decide el core con las tareas que hay.
    fn render_filters(&self, filter: &TaskFilter, cx: &mut Context<Self>) -> impl IntoElement {
        let options = Backend::core(cx).my_work().filter_options(filter);
        let here = locale(cx);
        let phrase = move |key: &str| cac_core::text(here, key);
        let view = cx.entity().downgrade();

        let assignee = (!options.assignees.is_empty()).then(|| {
            let chosen = self.assignee.clone();
            let label = match chosen.as_str() {
                "" => phrase("work:board.filter.anyone"),
                NOBODY => phrase("work:board.filter.nobody"),
                name => name.to_string(),
            };
            let mut choices = vec![(String::new(), phrase("work:board.filter.anyone"))];
            if options.unassigned {
                choices.push((NOBODY.to_string(), phrase("work:board.filter.nobody")));
            }
            choices.extend(options.assignees.iter().map(|name| (name.clone(), name.clone())));
            let view = view.clone();
            Button::new("filter-assignee").ghost().small().label(label).dropdown_caret(true).dropdown_menu(move |mut menu, _, _| {
                for (value, label) in choices.clone() {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(chosen == value).on_click(move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| this.choose_assignee(value.clone(), cx));
                    }));
                }
                menu
            })
        });

        let priority = (!options.priorities.is_empty()).then(|| {
            let chosen = self.priority.clone();
            let label = match chosen.as_str() {
                "" => phrase("work:board.filter.anyPriority"),
                name => phrase(&format!("work:taskPriority.{name}")),
            };
            let mut choices = vec![(String::new(), phrase("work:board.filter.anyPriority"))];
            choices.extend(options.priorities.iter().map(|name| (name.clone(), phrase(&format!("work:taskPriority.{name}")))));
            let view = view.clone();
            Button::new("filter-priority").ghost().small().label(label).dropdown_caret(true).dropdown_menu(move |mut menu, _, _| {
                for (value, label) in choices.clone() {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(chosen == value).on_click(move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| this.choose_priority(value.clone(), cx));
                    }));
                }
                menu
            })
        });

        let clear = filter.is_filtering().then(|| {
            Button::new("filter-clear")
                .ghost()
                .small()
                .icon(Icon::new(Lucide::X))
                .label(phrase("work:board.filter.clear"))
                .on_click(cx.listener(|this, _, window, cx| this.clear_filters(window, cx)))
        });

        div().flex().items_center().gap_1().pb_1().children(assignee).children(priority).children(clear)
    }

    fn show_lens(lens: WorkLens, cx: &App) {
        Backend::run(cx, move |core| async move { core.show_lens(lens).await });
    }

    fn show_closed(on: bool, cx: &App) {
        Backend::run(cx, move |core| async move { core.show_closed(on).await });
    }

    fn render_header(&self, shown: usize, filter: &TaskFilter, cx: &mut Context<Self>) -> impl IntoElement {
        let filters = self.render_filters(filter, cx);
        let theme = cx.theme();
        let (border, muted, foreground, primary, accent) =
            (theme.border, theme.muted_foreground, theme.foreground, theme.primary, theme.accent);
        let my_work = Backend::snapshot(cx).read(cx).my_work.clone();
        let here = locale(cx);
        let title = my_work.scope.as_ref().map(|scope| scope.name.clone()).unwrap_or_else(|| tr(cx, "work:myWork.title"));
        let count = if my_work.loading { "…".to_string() } else { cac_core::text_count(here, "work:myWork.visible", shown as i64) };
        let scope_kind = my_work.scope.as_ref().map(|scope| match scope.kind {
            ScopeKind::Space => tr(cx, "work:myWork.scopeSpace"),
            ScopeKind::List => tr(cx, "work:myWork.scopeList"),
        });
        let closed = my_work.include_closed;
        let docs = cac_core::docs_reachable_from_scope(my_work.scope.as_ref().map(|scope| scope.kind));

        div()
            .flex()
            .flex_none()
            .flex_col()
            .gap_3()
            .px_4()
            .pt_3()
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().id("view-title").test_support().text_lg().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(div().id("view-count").test_support().text_xs().text_color(muted).child(count))
                    // A qué está acotado, y cómo dejar de estarlo.
                    .children(scope_kind.map(|kind| {
                        div()
                            .id("scope-clear")
                            .test_support()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .rounded_md()
                            .bg(accent)
                            .text_xs()
                            .text_color(muted)
                            .cursor_pointer()
                            .on_click(|_, _, cx| Backend::core(cx).my_work().set_scope(None))
                            .child(kind)
                            .child(Icon::new(Lucide::X).xsmall())
                    }))
                    .child(div().flex_1())
                    // La documentación del nodo, a un clic con cualquier
                    // ámbito, sea lista o espacio (command-and-control-372).
                    .when(docs, |this| {
                        this.child(
                            Button::new("open-docs")
                                .ghost()
                                .small()
                                .label(tr(cx, "work:board.view.docs"))
                                .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.open_scope_docs().await })),
                        )
                    })
                    .child(div().w(px(240.)).child(Input::new(&self.search).id("work-search").small().cleanable(true)))
                    .child(
                        Button::new("toggle-closed")
                            .ghost()
                            .small()
                            .label(tr(cx, if closed { "work:myWork.allStates" } else { "work:myWork.openOnly" }))
                            .tooltip(tr(cx, if closed { "work:myWork.hideFinished" } else { "work:myWork.showFinished" }))
                            .on_click(move |_, _, cx| Self::show_closed(!closed, cx)),
                    ),
            )
            .child(div().flex().items_center().gap_4().children(LENSES.map(|(lens, name)| {
                let active = my_work.lens == lens;
                div()
                    .id(SharedString::from(format!("lens-{name}")))
                    .test_support()
                    .pb_2()
                    .border_b_2()
                    .border_color(if active { primary } else { primary.opacity(0.) })
                    .text_sm()
                    .text_color(if active { foreground } else { muted })
                    .when(active, |this| this.font_weight(FontWeight::MEDIUM))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| Self::show_lens(lens, cx))
                    .child(tr(cx, &format!("work:myWork.lens.{name}")))
            })).child(div().flex_1()).child(filters))
    }

    fn render_row(&self, row: WorkRow, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, warning, info, hover, border) =
            (theme.muted_foreground, theme.danger, theme.warning, theme.info, theme.accent, theme.border);
        let priority = (row.priority != "none").then(|| {
            let tone = match row.priority.as_str() {
                "urgent" => danger,
                "high" => warning,
                "normal" => info,
                _ => muted,
            };
            div()
                .flex_none()
                .px_2()
                .rounded_md()
                .bg(tone.opacity(0.15))
                .text_xs()
                .text_color(tone)
                .child(tr(cx, &format!("work:taskPriority.{}", row.priority)))
        });
        let due = row.due.map(|due| {
            // Rojo si ya pasó, ámbar si es hoy o mañana; hoy, además, con peso.
            let (tone, strong) = match due.state {
                DueState::Overdue | DueState::Yesterday => (danger, true),
                DueState::Today => (warning, true),
                DueState::Tomorrow => (warning, false),
                DueState::Later => (muted, false),
            };
            div()
                .w(px(72.))
                .flex_none()
                .text_xs()
                .text_right()
                .text_color(tone)
                .when(strong, |this| this.font_weight(FontWeight::MEDIUM))
                .child(due.text)
        });

        div()
            .id(SharedString::from(format!("work-row-{}", row.id)))
            .test_support()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .hover(move |style| style.bg(hover.opacity(0.4)))
            .cursor_pointer()
            .on_click({
                let id = row.id.clone();
                move |_, _, cx| {
                    let id = id.clone();
                    Backend::run(cx, move |core| async move { core.tasks().open(&id).await });
                }
            })
            .child(div().w(px(44.)).flex_none().text_xs().text_color(muted).child(format!("#{}", row.seq)))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_baseline()
                    .gap_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_sm()
                    .child(div().overflow_hidden().text_ellipsis().child(row.title))
                    .children(row.parent.map(|parent| div().flex_none().text_xs().text_color(muted).child(parent))),
            )
            .children(priority)
            .children(due)
    }

    fn render_group(&self, group: WorkGroup, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, card, radius) = (theme.muted_foreground, theme.border, theme.group_box, theme.radius);
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(muted)
                    .child(format!("{} · {}", group.name.to_uppercase(), group.rows.len())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .bg(card)
                    .overflow_hidden()
                    .children(group.rows.into_iter().map(|row| self.render_row(row, cx))),
            )
    }
}

impl Render for MyWorkView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted) = (theme.background, theme.foreground, theme.muted_foreground);
        let filter = self.filter(cx);
        let groups = Backend::core(cx).my_work().groups(&filter, locale(cx));
        let shown: usize = groups.iter().map(|group| group.rows.len()).sum();
        // Por qué está vacía lo dice el core; aquí sólo se enseña.
        let empty = (shown == 0).then(|| tr(cx, Backend::core(cx).my_work().empty_reason()));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .child(self.render_header(shown, &filter, cx))
            .child(
                div()
                    .id("work-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .p_4()
                    .children(empty.map(|phrase| div().id("work-empty").test_support().text_sm().text_color(muted).child(phrase)))
                    .children(groups.into_iter().map(|group| self.render_group(group, cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::MyWorkView;
    use crate::backend::Backend;
    use crate::testing::{contract_task, server_with_tasks, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;

    fn open(server: &FakeServer, cx: &mut TestAppContext) -> AnyWindowHandle {
        open_with_view(server, cx).0
    }

    fn open_with_view(server: &FakeServer, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<MyWorkView>) {
        server.install(cx);
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(760.)),
                })),
                ..WindowOptions::default()
            };
            let (window, workspace) =
                gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx)))
                    .expect("open test window");
            (window, workspace.read(cx).my_work.clone())
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

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    fn tasks() -> Vec<serde_json::Value> {
        vec![
            contract_task("t1", "b", |task| task["title"] = json!("Subir el backend")),
            contract_task("t2", "a", |task| task["title"] = json!("Revisión del diseño")),
        ]
    }

    /// Las tareas que trae el servidor salen en la lista.
    ///
    /// Roturas que atrapa: una pantalla que sigue en el marcador; una lista
    /// que dice «aquí no hay nada» con tareas cargadas.
    #[gpui_kit::test]
    fn the_tasks_the_server_brings_are_listed(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);

        wait_until(cx, "las tareas llegan a la lista", |cx| on_screen(window, "work-row-t1", cx));

        assert!(on_screen(window, "work-row-t2", cx));
        assert!(!on_screen(window, "work-empty", cx), "dice que no hay nada con dos tareas en pantalla");
        stop(cx);
    }

    /// Sin tareas se dice que no hay nada, y no se queda en «cargando».
    ///
    /// Rotura que atrapa: una lista en blanco, que no distingue «no tienes
    /// nada» de «no ha llegado» ni de «se rompió».
    #[gpui_kit::test]
    fn an_empty_list_says_so(cx: &mut TestAppContext) {
        let server = server_with_tasks(vec![]);
        let window = open(&server, cx);

        wait_until(cx, "termina de cargar", |cx| {
            cx.update(|cx| {
                let my_work = &Backend::snapshot(cx).read(cx).my_work;
                !my_work.loading && my_work.loaded_key.is_some()
            })
        });

        assert!(on_screen(window, "work-empty", cx));
        stop(cx);
    }

    /// Escribir en el buscador deja sólo lo que coincide, sin acentos de por
    /// medio.
    ///
    /// Roturas que atrapa: un buscador que no filtra; que lo escrito no
    /// llegue al filtro del core.
    #[gpui_kit::test]
    fn typing_in_the_search_narrows_the_list(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));

        cx.update_window(window, |_, window, cx| {
            window.click("work-search", cx);
            window.input("revision", cx);
        })
        .unwrap();

        assert!(on_screen(window, "work-row-t2", cx));
        assert!(!on_screen(window, "work-row-t1", cx), "el buscador no quitó lo que no coincide");
        stop(cx);
    }

    fn mixed_tasks() -> Vec<serde_json::Value> {
        vec![
            contract_task("t1", "b", |task| {
                task["priority"] = json!("urgent");
                task["assignee"] = json!("ana");
            }),
            contract_task("t2", "a", |task| {
                task["priority"] = json!("low");
                task["assignee"] = json!("luis");
            }),
        ]
    }

    /// Los desplegables salen cuando hay algo que separar, y elegir en uno
    /// deja sólo lo que coincide.
    ///
    /// Roturas que atrapa: un desplegable que elige y no filtra; que el de
    /// responsable filtre por prioridad o al revés; desplegables que no salen.
    #[gpui_kit::test]
    fn choosing_in_a_picker_narrows_the_list(cx: &mut TestAppContext) {
        let server = server_with_tasks(mixed_tasks());
        let (window, view) = open_with_view(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        assert!(on_screen(window, "filter-assignee", cx) && on_screen(window, "filter-priority", cx));
        assert!(!on_screen(window, "filter-clear", cx), "ofrece quitar filtros sin ninguno puesto");

        cx.update(|cx| view.update(cx, |view, cx| view.choose_priority("low".into(), cx)));
        assert!(on_screen(window, "work-row-t2", cx));
        assert!(!on_screen(window, "work-row-t1", cx), "la prioridad elegida no filtró");

        cx.update(|cx| view.update(cx, |view, cx| view.choose_priority(String::new(), cx)));
        cx.update(|cx| view.update(cx, |view, cx| view.choose_assignee("ana".into(), cx)));
        assert!(on_screen(window, "work-row-t1", cx));
        assert!(!on_screen(window, "work-row-t2", cx), "el responsable elegido no filtró");
        stop(cx);
    }

    /// Sin nada que separar no hay desplegables: una sola persona con una
    /// sola prioridad.
    ///
    /// Rotura que atrapa: desplegables de una opción ocupando la cabecera.
    #[gpui_kit::test]
    fn pickers_with_nothing_to_separate_are_not_shown(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));

        assert!(!on_screen(window, "filter-assignee", cx));
        assert!(!on_screen(window, "filter-priority", cx));
        stop(cx);
    }

    /// «Quitar filtros» lo quita todo, también lo escrito en el buscador.
    ///
    /// Roturas que atrapa: un botón que deja un filtro puesto; uno que
    /// limpia los desplegables y deja la búsqueda escondiendo tareas.
    #[gpui_kit::test]
    fn clearing_the_filters_brings_everything_back(cx: &mut TestAppContext) {
        let server = server_with_tasks(mixed_tasks());
        let (window, view) = open_with_view(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        cx.update_window(window, |_, window, cx| {
            window.click("work-search", cx);
            window.input("nada que exista", cx);
        })
        .unwrap();
        cx.update(|cx| view.update(cx, |view, cx| view.choose_assignee("ana".into(), cx)));
        cx.update(|cx| view.update(cx, |view, cx| view.choose_priority("urgent".into(), cx)));
        assert!(!on_screen(window, "work-row-t1", cx) && !on_screen(window, "work-row-t2", cx));

        click(window, "filter-clear", cx);

        assert!(on_screen(window, "work-row-t1", cx) && on_screen(window, "work-row-t2", cx), "quedó algún filtro puesto");
        assert!(!on_screen(window, "filter-clear", cx));
        stop(cx);
    }

    /// Pulsar una lente le hace al servidor la pregunta de esa lente, y
    /// «sólo lo abierto» pide también lo terminado.
    ///
    /// Roturas que atrapa: pestañas que cambian de color y enseñan lo mismo;
    /// un interruptor que no pide nada.
    #[gpui_kit::test]
    fn a_lens_and_the_open_only_switch_ask_the_server(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        let asked = |server: &FakeServer, what: &str| server.queries("/api/v1/tasks/").iter().any(|query| query.contains(what));
        assert!(asked(&server, "assignee=me") && !asked(&server, "watcher=me") && !asked(&server, "status=all"));

        click(window, "lens-watching", cx);
        wait_until(cx, "pregunta por lo que sigue", |_| asked(&server, "watcher=me"));

        click(window, "toggle-closed", cx);
        wait_until(cx, "pide también lo terminado", |_| asked(&server, "status=all"));
        stop(cx);
    }

    /// Con un ámbito puesto, la cabecera lo dice y su × lo quita.
    ///
    /// Roturas que atrapa: quedarse acotado sin saberlo; no tener cómo
    /// volver a verlo todo desde la propia lista.
    #[gpui_kit::test]
    fn the_scope_shows_in_the_header_and_can_be_cleared(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);
        wait_until(cx, "llega el árbol", |cx| on_screen(window, "tree-a", cx));
        assert!(!on_screen(window, "scope-clear", cx));

        click(window, "tree-a", cx);
        wait_until(cx, "sólo las de ese espacio", |cx| on_screen(window, "scope-clear", cx) && !on_screen(window, "work-row-t1", cx));
        assert!(on_screen(window, "work-row-t2", cx));

        click(window, "scope-clear", cx);
        wait_until(cx, "vuelven todas", |cx| on_screen(window, "work-row-t1", cx));
        assert!(!on_screen(window, "scope-clear", cx));
        stop(cx);
    }
}
