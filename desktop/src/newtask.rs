//! La fila de «tarea nueva»: levantar una tarea sin salir de la lista.
//!
//! Una fila y no un diálogo, y se queda abierta tras cada una: el trabajo
//! llega en tandas —termina una llamada y hay cuatro cosas que apuntar—, y un
//! diálogo entre cada una son cuatro cambios de contexto para cuatro frases.
//!
//! Dónde puede nacer, qué se manda y a quién se le puede dar lo decide el
//! core (`cac_core::task::new`). Aquí está lo escrito y lo elegido.

use cac_core::{Choice, Destination, TaskDraft, PRIORITIES};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{error_text, tr};

/// La fila se cerró: quien la tiene la quita.
pub struct NewTaskClosed;

pub struct NewTaskRow {
    title: Entity<InputState>,
    /// La lista elegida. Vacía, o una que ya no se ofrece, es «la primera»:
    /// lo resuelve el core con lo que se mira en cada momento.
    list_id: String,
    priority: String,
    due: Entity<DatePickerState>,
    assignee: Option<String>,
    /// Que el cliente no la lea. Sólo se pregunta en la lista de un cliente.
    internal: bool,
    busy: bool,
    /// Por qué no se creó la última, dicho en la fila.
    problem: Option<String>,
    /// A quién se le puede dar.
    people: Vec<Choice>,
    _on_enter: Subscription,
}

impl EventEmitter<NewTaskClosed> for NewTaskRow {}

impl NewTaskRow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "work:board.whatNeedsDoing");
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let on_enter = cx.subscribe_in(&title, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create(window, cx);
            }
        });
        title.update(cx, |title, cx| title.focus(window, cx));

        let people = Backend::ask(cx, |core| async move { core.tasks().people_to_assign().await });
        cx.spawn(async move |this, cx| {
            let Ok(people) = people.await else { return };
            this.update(cx, |this, cx| {
                this.people = people;
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            title,
            list_id: String::new(),
            priority: "normal".into(),
            due: cx.new(|cx| DatePickerState::new(window, cx).date_format("%d/%m/%Y")),
            assignee: None,
            internal: false,
            busy: false,
            problem: None,
            people: Vec::new(),
            _on_enter: on_enter,
        }
    }

    pub(crate) fn choose_list(&mut self, list_id: String, cx: &mut Context<Self>) {
        self.list_id = list_id;
        cx.notify();
    }

    pub(crate) fn choose_priority(&mut self, priority: String, cx: &mut Context<Self>) {
        self.priority = priority;
        cx.notify();
    }

    pub(crate) fn choose_assignee(&mut self, assignee: Option<String>, cx: &mut Context<Self>) {
        self.assignee = assignee;
        cx.notify();
    }

    pub(crate) fn set_internal(&mut self, internal: bool, cx: &mut Context<Self>) {
        self.internal = internal;
        cx.notify();
    }

    /// Le pone ese día de vencimiento: la fila que se abre desde un día del
    /// calendario nace con él.
    pub(crate) fn set_due_day(&mut self, day: chrono::NaiveDate, window: &mut Window, cx: &mut Context<Self>) {
        self.due.update(cx, |due, cx| due.set_date(Date::Single(Some(day)), window, cx));
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(NewTaskClosed);
    }

    /// Dónde nacería ahora mismo.
    fn destination(&self, cx: &App) -> Option<Destination> {
        Backend::core(cx).tasks().destinations().chosen(&self.list_id).cloned()
    }

    /// Crea la tarea con lo escrito y lo elegido. Si se creó, sólo se vacía
    /// el título: lo siguiente que se apunta casi siempre va a la misma lista
    /// con la misma urgencia. Si no, lo escrito se queda y se dice por qué.
    pub(crate) fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.title.read(cx).value().to_string();
        let Some(destination) = self.destination(cx) else { return };
        if self.busy || title.trim().is_empty() {
            return;
        }
        let due_day = match self.due.read(cx).date() {
            Date::Single(Some(day)) => Some(day.format("%Y-%m-%d").to_string()),
            _ => None,
        };
        let draft = TaskDraft {
            list_id: destination.list_id,
            title,
            priority: self.priority.clone(),
            due_day,
            assignee_id: self.assignee.clone(),
            // Lo elegido en la lista de un cliente no se arrastra a otra
            // donde ya no se pregunta.
            internal: destination.client && self.internal,
        };
        self.busy = true;
        self.problem = None;
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move { core.tasks().raise(draft).await.map(|_| ()) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => this.title.update(cx, |title, cx| title.set_value("", window, cx)),
                    Err(error) => this.problem = Some(error_text(cx, &error, "work:board.errCreate")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Un desplegable de una sola elección.
    fn render_picker<T: Clone + PartialEq + 'static>(
        &self,
        id: &'static str,
        chosen: T,
        choices: Vec<(T, String)>,
        choose: fn(&mut Self, T, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let label = choices.iter().find(|(value, _)| *value == chosen).map(|(_, label)| label.clone()).unwrap_or_default();
        // El `Picker` de la app de siempre en su tamaño `sm`: con borde y
        // su chevrón, no un texto suelto.
        Button::new(id).outline().small().label(label).dropdown_caret(true).dropdown_menu(move |mut menu, _, _| {
            for (value, label) in choices.clone() {
                let view = view.clone();
                menu = menu.item(PopupMenuItem::new(label).checked(value == chosen).on_click(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| choose(this, value.clone(), cx));
                }));
            }
            menu
        })
    }
}

impl Render for NewTaskRow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground, primary, card, danger, mono) =
            (theme.muted_foreground, theme.foreground, theme.primary, theme.group_box, theme.danger, theme.mono_font_family.clone());
        let close_hint = tr(cx, "work:board.closeComposer");
        let destinations = Backend::core(cx).tasks().destinations();
        let destination = destinations.chosen(&self.list_id).cloned();

        let place = destination.as_ref().map(|destination| {
            if destinations.fixed {
                // Ya se sabe dónde va: se dice, no se pregunta.
                div().id("new-task-list").test_support().max_w(px(256.)).truncate().px_1().text_color(muted).child(destination.label.clone()).into_any_element()
            } else {
                let choices = destinations.options.iter().map(|option| (option.list_id.clone(), option.label.clone())).collect();
                self.render_picker("new-task-list", destination.list_id.clone(), choices, Self::choose_list, cx).into_any_element()
            }
        });
        let priorities = PRIORITIES.iter().map(|name| (name.to_string(), tr(cx, &format!("work:taskPriority.{name}")))).collect();
        let priority = self.render_picker("new-task-priority", self.priority.clone(), priorities, Self::choose_priority, cx);
        let mut people = vec![(None, tr(cx, "work:board.nobodyYet"))];
        people.extend(self.people.iter().map(|person| (Some(person.id.clone()), person.name.clone())));
        let assignee = self.render_picker("new-task-assignee", self.assignee.clone(), people, Self::choose_assignee, cx);
        // Quién la ve sólo se pregunta donde un cliente podría leerla.
        let audience = destination.as_ref().filter(|destination| destination.client).map(|_| {
            let choices = vec![(false, tr(cx, "work:board.clientSeesIt")), (true, tr(cx, "work:board.internalOnly"))];
            self.render_picker("new-task-audience", self.internal, choices, Self::set_internal, cx)
        });

        // Las medidas son las de `app/src/components/tasks/NewTaskRow.tsx`:
        // `rounded-md border border-primary/40 bg-card p-2`, el título solo
        // con su × (crear es Enter, y lo dice la esquina), y debajo lo
        // elegido a `gap-1.5` en letra pequeña.
        div()
            .id("new-task-row")
            .test_support()
            .flex()
            .flex_col()
            .p_2()
            .rounded(px(8.))
            .border_1()
            .border_color(primary.opacity(0.4))
            .bg(card)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close(cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.title).id("new-task-title").appearance(false)))
                    .when(self.busy, |this| this.child(div().id("new-task-busy").flex_none().child(Spinner::new().color(muted)).test_support()))
                    .child(
                        div()
                            .id("new-task-close")
                            .flex_none()
                            .text_color(muted)
                            .cursor_pointer()
                            .hover(move |style| style.text_color(foreground))
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                            .child(Icon::new(Lucide::X).size_4())
                            .tooltip(move |window, cx| Tooltip::new(close_hint.clone()).build(window, cx))
                            .test_support(),
                    ),
            )
            .child(
                div()
                    .mt_2()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .children(place)
                    .child(priority)
                    .child(div().id("new-task-due").test_support().w(px(150.)).child(
                        DatePicker::new(&self.due).small().cleanable(true).placeholder(tr(cx, "work:board.dueDate")),
                    ))
                    .child(assignee)
                    .children(audience)
                    .child(div().ml_auto().font_family(mono).text_size(px(10.)).text_color(muted).child(tr(cx, "work:board.composerKeys"))),
            )
            .children(self.problem.clone().map(|problem| div().id("new-task-problem").test_support().mt_2().text_xs().text_color(danger).child(problem)))
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::NewTaskRow;
    use crate::backend::Backend;
    use crate::mywork::MyWorkView;
    use crate::testing::{contract_task, ok, server_with_tasks, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use cac_core::{ScopeKind, WorkScope};
    use chrono::NaiveDate;
    use gpui_kit::component::calendar::Date;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, ResponseTemplate};

    const CREATED: &str = r#"{ "id": "t9", "createdAt": "2026-11-15T10:00:00Z", "updatedAt": "2026-11-15T10:00:00Z",
        "listId": "l-a", "seq": 40, "title": "x", "status": "pending", "priority": "medium" }"#;

    /// Dos espacios; la lista del segundo (`l-b`) es la de un cliente. Y un
    /// equipo de dos personas.
    fn server() -> FakeServer {
        let server = server_with_tasks(vec![contract_task("t1", "a", |_| {})]);
        let space = |id: &str, project: Value| json!({
            "id": id, "orgId": "o1", "name": format!("Espacio {id}"), "color": "#888", "folders": [], "people": [],
            "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 1, "taskCount": 1, "projectId": project }],
        });
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/task-spaces/"))
                .respond_with(ok(json!([space("a", Value::Null), space("b", json!("p1"))])))
                .with_priority(1),
        );
        server.mount(Mock::given(method("GET")).and(path("/api/v1/users/search")).respond_with(ok(json!([
            { "id": "u2", "username": "luis" },
            { "id": "u1", "username": "ana", "name": "Ana Ruiz" },
        ]))));
        server
    }

    fn creating(server: &FakeServer) {
        server.mount(
            Mock::given(method("POST"))
                .and(path_regex("^/api/v1/task-lists/[^/]+/tasks$"))
                .respond_with(ok(serde_json::from_str(CREATED).unwrap())),
        );
    }

    fn open(server: &FakeServer, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<MyWorkView>) {
        server.install(cx);
        sign_in(cx);
        let opened = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(760.)) })),
                ..WindowOptions::default()
            };
            let (window, workspace) =
                gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window");
            (window, workspace.read(cx).my_work.clone())
        });
        wait_until(cx, "llega el árbol", |cx| on_screen(opened.0, "tree-b", cx));
        opened
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

    fn press(window: AnyWindowHandle, key: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
    }

    fn write(window: AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input(text, cx);
        })
        .unwrap();
    }

    /// La fila abierta.
    fn row(view: &Entity<MyWorkView>, cx: &mut TestAppContext) -> Entity<NewTaskRow> {
        cx.update(|cx| view.read(cx).new_task.clone()).expect("la fila no está abierta")
    }

    fn titled(view: &Entity<MyWorkView>, cx: &mut TestAppContext) -> String {
        let row = row(view, cx);
        cx.update(|cx| row.read(cx).title.read(cx).value().to_string())
    }

    /// Lo que se mandó a crear: en qué lista y con qué.
    fn created(server: &FakeServer) -> Vec<(String, Value)> {
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() == "POST" && request.url.path().ends_with("/tasks"))
            .map(|request| (request.url.path().to_string(), serde_json::from_slice(&request.body).unwrap()))
            .collect()
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// «Tarea nueva» abre la fila; su ×, Escape y el propio botón la cierran.
    ///
    /// Roturas que atrapa: un botón que no abre nada; una fila que no se
    /// puede quitar de encima de la lista.
    #[gpui_kit::test]
    fn the_button_opens_the_row_and_it_can_be_closed(cx: &mut TestAppContext) {
        let server = server();
        let (window, _) = open(&server, cx);
        assert!(!on_screen(window, "new-task-row", cx));

        click(window, "new-task", cx);
        assert!(on_screen(window, "new-task-row", cx));
        click(window, "new-task-close", cx);
        assert!(!on_screen(window, "new-task-row", cx), "la × no cerró la fila");

        click(window, "new-task", cx);
        press(window, "escape", cx);
        assert!(!on_screen(window, "new-task-row", cx), "Escape no cerró la fila");

        click(window, "new-task", cx);
        click(window, "new-task", cx);
        assert!(!on_screen(window, "new-task-row", cx), "el botón no cerró la fila que abrió");
        stop(cx);
    }

    /// Se escribe nada más abrir, Enter crea con lo elegido en la primera
    /// lista, y la fila sigue abierta con el título vacío y lo demás igual.
    ///
    /// Roturas que atrapa: tener que pulsar en el campo para escribir; un
    /// Enter que no crea; crear sin la prioridad, la fecha o la persona
    /// elegidas; cerrar la fila tras la primera; dejar el título escrito y
    /// crear la misma dos veces; decir quién la ve donde nadie eligió.
    #[gpui_kit::test]
    fn enter_creates_with_what_was_chosen_and_the_row_stays(cx: &mut TestAppContext) {
        let server = server();
        creating(&server);
        let (window, view) = open(&server, cx);
        click(window, "new-task", cx);
        let row = row(&view, cx);
        wait_until(cx, "llega el equipo", |cx| cx.update(|cx| row.read(cx).people.len()) == 2);
        let names: Vec<String> = cx.update(|cx| row.read(cx).people.iter().map(|person| person.name.clone()).collect());
        assert_eq!(names, ["Ana Ruiz", "luis"]);

        write(window, "Subir el backend", cx);
        cx.update_window(window, |_, window, cx| {
            row.update(cx, |row, cx| {
                row.choose_priority("high".into(), cx);
                row.choose_assignee(Some("u2".into()), cx);
                row.set_internal(true, cx);
                let day = NaiveDate::from_ymd_opt(2026, 12, 1);
                row.due.update(cx, |due, cx| due.set_date(Date::Single(day), window, cx));
            })
        })
        .unwrap();
        press(window, "enter", cx);

        wait_until(cx, "se crea y el título se vacía", |cx| !created(&server).is_empty() && titled(&view, cx).is_empty());
        assert_eq!(
            created(&server),
            [(
                "/api/v1/task-lists/l-a/tasks".to_string(),
                json!({ "title": "Subir el backend", "priority": "high", "dueAt": "2026-12-01T00:00:00Z", "assigneeIds": ["u2"] })
            )]
        );
        assert!(on_screen(window, "new-task-row", cx), "la fila se cerró tras crear");
        assert_eq!(cx.update(|cx| row.read(cx).priority.clone()), "high");
        stop(cx);
    }

    /// Con una lista a la vista la tarea va ahí y se dice; sin ella va a la
    /// que se elija.
    ///
    /// Roturas que atrapa: crear desde tu lista y que la tarea acabe en la
    /// primera del árbol; un desplegable que elige y no cambia el destino;
    /// que una tarea nazca con otra prioridad que la normal sin elegirla.
    #[gpui_kit::test]
    fn it_goes_to_the_list_looked_at_or_to_the_chosen_one(cx: &mut TestAppContext) {
        let server = server();
        creating(&server);
        let (window, view) = open(&server, cx);
        click(window, "new-task", cx);
        let row = row(&view, cx);

        cx.update(|cx| row.update(cx, |row, cx| row.choose_list("l-b".into(), cx)));
        write(window, "Una", cx);
        press(window, "enter", cx);
        wait_until(cx, "se crea la primera", |cx| created(&server).len() == 1 && titled(&view, cx).is_empty());

        cx.update(|cx| {
            row.update(cx, |row, cx| row.choose_list(String::new(), cx));
            let scope = WorkScope { kind: ScopeKind::List, id: "l-b".into(), name: "Lista".into(), org_id: Some("o1".into()) };
            Backend::core(cx).my_work().set_scope(Some(scope));
        });
        write(window, "Otra", cx);
        press(window, "enter", cx);
        wait_until(cx, "se crea la segunda", |_| created(&server).len() == 2);

        let lists: Vec<String> = created(&server).iter().map(|(route, _)| route.clone()).collect();
        assert_eq!(lists, ["/api/v1/task-lists/l-b/tasks", "/api/v1/task-lists/l-b/tasks"]);
        // Sin elegir prioridad nace normal, como en la app de siempre.
        assert_eq!(created(&server)[0].1, json!({ "title": "Una", "priority": "normal" }));
        stop(cx);
    }

    /// Quién la ve se pregunta sólo en la lista de un cliente, y lo elegido
    /// ahí no viaja a otra lista.
    ///
    /// Roturas que atrapa: no poder dejar interna una tarea que el cliente
    /// leería; un interruptor que no cambia lo que se manda; preguntar por
    /// una distinción que en una lista interna no existe.
    #[gpui_kit::test]
    fn who_reads_it_is_asked_only_in_a_clients_list(cx: &mut TestAppContext) {
        let server = server();
        creating(&server);
        let (window, view) = open(&server, cx);
        click(window, "new-task", cx);
        let row = row(&view, cx);
        assert!(!on_screen(window, "new-task-audience", cx), "pregunta quién la ve en una lista interna");

        cx.update(|cx| row.update(cx, |row, cx| row.choose_list("l-b".into(), cx)));
        assert!(on_screen(window, "new-task-audience", cx));
        write(window, "La ve", cx);
        press(window, "enter", cx);
        wait_until(cx, "se crea a la vista del cliente", |cx| created(&server).len() == 1 && titled(&view, cx).is_empty());

        cx.update(|cx| row.update(cx, |row, cx| row.set_internal(true, cx)));
        write(window, "Interna", cx);
        press(window, "enter", cx);
        wait_until(cx, "se crea interna", |cx| created(&server).len() == 2 && titled(&view, cx).is_empty());

        cx.update(|cx| row.update(cx, |row, cx| row.choose_list("l-a".into(), cx)));
        write(window, "En otra lista", cx);
        press(window, "enter", cx);
        wait_until(cx, "se crea en la interna", |_| created(&server).len() == 3);

        let said: Vec<Value> = created(&server).into_iter().map(|(_, body)| body["visibility"].clone()).collect();
        assert_eq!(said, [Value::Null, json!("internal"), Value::Null]);
        stop(cx);
    }

    /// Si no se crea, se dice por qué y lo escrito se queda; sin título no
    /// se pide nada.
    ///
    /// Roturas que atrapa: perder lo escrito por un fallo de red; un botón
    /// que parece no hacer nada; crear tareas sin título; decir «no se pudo
    /// crear» por pulsar Enter con la fila vacía.
    #[gpui_kit::test]
    fn a_task_that_is_not_created_keeps_what_was_written(cx: &mut TestAppContext) {
        let server = server();
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/task-lists/l-a/tasks"))
                .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "success": false, "message": "boom", "error": "x" }))),
        );
        let (window, view) = open(&server, cx);
        click(window, "new-task", cx);

        write(window, "   ", cx);
        press(window, "enter", cx);
        cx.run_until_parked();
        assert!(created(&server).is_empty(), "pidió crear una tarea sin título");
        assert!(!on_screen(window, "new-task-problem", cx), "regaña por un Enter con la fila vacía");

        write(window, "Importante", cx);
        press(window, "enter", cx);
        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "new-task-problem", cx));

        assert_eq!(titled(&view, cx), "   Importante");
        stop(cx);
    }
}
