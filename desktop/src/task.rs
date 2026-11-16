//! El detalle de una tarea, para leerlo: un cajón sobre la derecha de la vista.
//!
//! Qué se lee (cada frase, qué comentario es de un agente, qué subtarea está
//! hecha, por qué no hay nada todavía) lo dice el core en su `TaskSheet`.
//! Aquí está qué pestaña del hilo se mira, que es lo único propio, y la
//! pintura. Cada cambio se le pide al core, que es quien sabe qué mandar.

use cac_core::{Choice, DueState, SheetComment, SheetSubtask, StatusOption, TaskSheet, PRIORITIES};
use chrono::NaiveDate;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::text::TextView;
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{error_text, locale, tr};

/// El ancho del panel: el del cajón de la app de siempre.
pub const TASK_PANEL_WIDTH: f32 = 520.;

/// Las dos pestañas del hilo.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Thread {
    Talk,
    Notes,
}

pub struct TaskView {
    thread: Thread,
    /// De qué tarea es la pestaña elegida: al abrir otra se vuelve a la
    /// conversación, que es donde se le habla a quien la abre.
    thread_of: Option<String>,
    /// Lo último que no se pudo guardar, dicho, y de qué tarea: no hay otro
    /// sitio donde enseñarlo, y callarlo deja un botón que parece no hacer nada.
    problem: Option<(String, String)>,
    /// Lo que se está escribiendo para comentar.
    comment: Entity<TextareaState>,
    /// Que el cliente no lo lea. Sólo cuenta donde un cliente podría leerlo.
    internal: bool,
    /// De qué tarea se están editando el título y la descripción. Al abrir
    /// otra no se sigue editando: lo escrito era de aquélla.
    editing: Option<String>,
    title_input: Entity<InputState>,
    body_input: Entity<TextareaState>,
    /// El selector del vencimiento.
    due: Entity<DatePickerState>,
    /// De qué tarea y qué día es lo que el selector enseña: se le pone el de
    /// la tarea cuando cambia, y no en cada pintada, que le pisaría a quien
    /// está eligiendo.
    due_shown: Option<(String, Option<String>)>,
    _due_changed: Subscription,
}

/// Con qué tono se pinta una prioridad.
fn priority_tone(priority: &str, cx: &App) -> Hsla {
    let theme = cx.theme();
    match priority {
        "urgent" => theme.danger,
        "high" => theme.warning,
        "normal" => theme.info,
        _ => theme.muted_foreground,
    }
}

impl TaskView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "work:task.writeComment");
        let due = cx.new(|cx| DatePickerState::new(window, cx).date_format("%d/%m/%Y"));
        // El selector avisa cuando alguien elige un día o lo quita; ponerle
        // desde aquí el de la tarea no avisa.
        let due_changed = cx.subscribe_in(&due, window, |this, _, event: &DatePickerEvent, window, cx| {
            let DatePickerEvent::Change(value) = event;
            this.set_due(value.format("%Y-%m-%d").map(|day| day.to_string()), window, cx);
        });
        Self {
            thread: Thread::Talk,
            thread_of: None,
            problem: None,
            comment: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 8).placeholder(placeholder)),
            internal: false,
            editing: None,
            title_input: cx.new(|cx| InputState::new(window, cx)),
            body_input: cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 24)),
            due,
            due_shown: None,
            _due_changed: due_changed,
        }
    }

    /// Pasa a editar el título y la descripción, con lo que la tarea dice.
    pub(crate) fn start_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = Backend::core(cx).tasks().sheet(locale(cx)) else { return };
        self.title_input.update(cx, |input, cx| input.set_value(sheet.title, window, cx));
        self.body_input.update(cx, |input, cx| input.set_value(sheet.description, window, cx));
        self.editing = Some(sheet.id);
        cx.notify();
    }

    /// Deja de editar sin guardar: lo escrito se tira.
    pub(crate) fn cancel_editing(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        cx.notify();
    }

    /// Guarda el título y la descripción escritos y, si se guardaron, deja de
    /// editar. Si no, se sigue editando con lo escrito.
    pub(crate) fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.title_input.read(cx).value().to_string();
        let body = self.body_input.read(cx).value().to_string();
        self.save(
            "common:misc.errSave",
            move |core| async move {
                core.tasks().set_title(&title).await?;
                core.tasks().set_description(&body).await
            },
            |this, _, _| this.editing = None,
            window,
            cx,
        );
    }

    /// Le pone al selector el día de la tarea, cuando es otra tarea u otro
    /// día que el que enseña.
    fn show_due(&mut self, sheet: &TaskSheet, window: &mut Window, cx: &mut Context<Self>) {
        let now = (sheet.id.clone(), sheet.due_day.clone());
        if self.due_shown.as_ref() == Some(&now) {
            return;
        }
        let day = sheet.due_day.as_deref().and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok());
        self.due.update(cx, |due, cx| due.set_date(Date::Single(day), window, cx));
        self.due_shown = Some(now);
    }

    /// Un cambio de un toque en la tarea abierta.
    fn change<F, Fut>(&mut self, work: F, window: &mut Window, cx: &mut Context<Self>)
    where
        F: FnOnce(cac_core::Core) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), cac_core::ApiError>> + Send + 'static,
    {
        self.save("common:misc.errSave", work, |_, _, _| {}, window, cx);
    }

    pub(crate) fn set_priority(&mut self, priority: String, window: &mut Window, cx: &mut Context<Self>) {
        self.change(move |core| async move { core.tasks().set_priority(&priority).await }, window, cx);
    }

    /// Pone el vencimiento en ese día (`YYYY-MM-DD`), o lo quita.
    pub(crate) fn set_due(&mut self, day: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.change(move |core| async move { core.tasks().set_due(day.as_deref()).await }, window, cx);
    }

    pub(crate) fn toggle_tag(&mut self, tag_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.change(move |core| async move { core.tasks().toggle_tag(&tag_id).await }, window, cx);
    }

    pub(crate) fn toggle_assignee(&mut self, user_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.change(move |core| async move { core.tasks().toggle_assignee(&user_id).await }, window, cx);
    }

    /// Le pide al core algo que se guarda en la tarea abierta y, si no se
    /// pudo, lo deja dicho en el cajón. `done` corre si salió bien.
    fn save<F, Fut>(
        &mut self,
        fallback_key: &'static str,
        work: F,
        done: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(cac_core::Core) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), cac_core::ApiError>> + Send + 'static,
    {
        let Some(task_id) = Backend::snapshot(cx).read(cx).task.task_id.clone() else { return };
        self.problem = None;
        cx.notify();
        let answer = Backend::ask(cx, work);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => done(this, window, cx),
                    Err(error) => {
                        this.problem = Some((task_id, error_text(cx, &error, fallback_key)));
                        // El selector de fecha se quedó con lo que se eligió
                        // y no se guardó: que vuelva a enseñar lo que hay.
                        this.due_shown = None;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Marca o desmarca una línea del checklist.
    pub(crate) fn tick_subtask(&mut self, subtask_id: String, done: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.save(
            "work:task.errMoveList",
            move |core| async move { core.tasks().set_subtask_done(&subtask_id, done).await },
            |_, _, _| {},
            window,
            cx,
        );
    }

    /// Manda lo escrito como comentario y, si se guardó, vacía la caja. Si
    /// no, lo escrito se queda: perderlo por un fallo de red es lo peor.
    pub(crate) fn send_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.comment.read(cx).value().to_string();
        if body.trim().is_empty() {
            return;
        }
        let internal = self.internal;
        self.save(
            "work:task.errComment",
            move |core| async move { core.tasks().say(&body, internal).await },
            |this, window, cx| this.comment.update(cx, |comment, cx| comment.set_value("", window, cx)),
            window,
            cx,
        );
    }

    pub(crate) fn set_internal(&mut self, internal: bool, cx: &mut Context<Self>) {
        self.internal = internal;
        cx.notify();
    }

    /// La caja de comentar. Donde un cliente lee, dice quién va a leerlo y
    /// deja cambiarlo antes de mandar.
    fn render_composer(&self, client_reads: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let internal = client_reads && self.internal;
        let audience = client_reads.then(|| {
            Button::new("comment-audience")
                .ghost()
                .small()
                .label(tr(cx, if internal { "work:task.internalNote" } else { "work:task.theClientReads" }))
                .tooltip(tr(cx, if internal { "work:task.onlyTeamWillSee" } else { "work:task.clientReadsSwitch" }))
                .on_click(cx.listener(move |this, _, _, cx| this.set_internal(!internal, cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().id("comment-input").test_support().child(Textarea::new(&self.comment)))
            .child(
                div().flex().items_center().gap_2().child(div().flex_1()).children(audience).child(
                    Button::new("comment-send")
                        .primary()
                        .small()
                        .label(tr(cx, if internal { "work:task.commentInternally" } else { "work:task.comment" }))
                        .on_click(cx.listener(|this, _, window, cx| this.send_comment(window, cx))),
                ),
            )
    }

    fn show_thread(&mut self, thread: Thread, task_id: String, cx: &mut Context<Self>) {
        self.thread = thread;
        self.thread_of = Some(task_id);
        cx.notify();
    }

    /// Mueve la tarea abierta a otra columna. Si se puede lo decide el core
    /// (y el servidor); aquí sólo se enseña lo que conteste.
    pub(crate) fn move_to(&mut self, status_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.save(
            "work:task.errMoveList",
            move |core| async move { core.tasks().set_status(&status_id).await },
            |_, _, _| {},
            window,
            cx,
        );
    }

    fn section(&self, title: String, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        div().text_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(title.to_uppercase())
    }

    fn render_chips(&self, sheet: &TaskSheet, statuses: Vec<StatusOption>, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let current = sheet.status_name.clone();
        let theme = cx.theme();
        let (muted, danger, warning, accent) = (theme.muted_foreground, theme.danger, theme.warning, theme.accent);
        let chip = |tone: Hsla, label: String| div().flex_none().px_2().rounded_md().bg(tone.opacity(0.15)).text_xs().text_color(tone).child(label);
        let status_tone = gpui_kit::rgb(u32::from_str_radix(sheet.status_color.trim_start_matches('#'), 16).unwrap_or(0x888888));
        let due = sheet.due.clone().map(|due| {
            let tone = match due.state {
                DueState::Overdue | DueState::Yesterday => danger,
                DueState::Today | DueState::Tomorrow => warning,
                DueState::Later => muted,
            };
            chip(tone, due.text)
        });

        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .pl_2()
                    .rounded_md()
                    .bg(accent)
                    .child(div().size(px(8.)).rounded_full().bg(status_tone))
                    // Las columnas de su lista; la que su máquina no deja
                    // sale apagada en vez de ser un botón que contesta 409.
                    .child(
                        Button::new("task-status")
                            .ghost()
                            .xsmall()
                            .label(sheet.status_name.clone())
                            .dropdown_caret(!statuses.is_empty())
                            .dropdown_menu(move |mut menu, _, _| {
                                for option in statuses.clone() {
                                    let (view, id) = (view.clone(), option.status.id.clone());
                                    menu = menu.item(
                                        PopupMenuItem::new(option.status.name.clone())
                                            .checked(option.status.name == current)
                                            .disabled(!option.allowed)
                                            .on_click(move |_, window, cx| {
                                                let _ = view.update(cx, |this, cx| this.move_to(id.clone(), window, cx));
                                            }),
                                    );
                                }
                                menu
                            }),
                    ),
            )
            .children(due)
    }

    /// Un desplegable de cosas que se ponen y se quitan: lo que la tarea
    /// tiene, marcado. Sin nada puesto, el botón dice qué se puede hacer.
    fn render_choices(
        &self,
        id: &'static str,
        empty_key: &str,
        choices: Vec<Choice>,
        toggle: fn(&mut Self, String, &mut Window, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let chosen: Vec<String> = choices.iter().filter(|choice| choice.chosen).map(|choice| choice.name.clone()).collect();
        let label = if chosen.is_empty() { tr(cx, empty_key) } else { chosen.join(", ") };
        Button::new(id).ghost().xsmall().label(label).dropdown_caret(!choices.is_empty()).dropdown_menu(move |mut menu, _, _| {
            for choice in choices.clone() {
                let view = view.clone();
                menu = menu.item(PopupMenuItem::new(choice.name).checked(choice.chosen).on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| toggle(this, choice.id.clone(), window, cx));
                }));
            }
            menu
        })
    }

    /// Lo que se le cambia a la tarea de un toque, cada cosa en su fila.
    fn render_fields(&self, sheet: &TaskSheet, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let view = cx.entity().downgrade();
        let tasks = Backend::core(cx).tasks();
        let current = sheet.priority.clone();
        let row = |name: String, control: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_none().w(px(96.)).text_xs().text_color(muted).child(name))
                .child(div().flex_1().min_w_0().flex().child(control))
        };
        let priority = Button::new("task-priority")
            .ghost()
            .xsmall()
            .label(tr(cx, &format!("work:taskPriority.{current}")))
            .text_color(priority_tone(&current, cx))
            .dropdown_caret(true)
            .dropdown_menu({
                let names: Vec<(String, String)> =
                    PRIORITIES.iter().map(|priority| (priority.to_string(), tr(cx, &format!("work:taskPriority.{priority}")))).collect();
                move |mut menu, _, _| {
                    for (priority, name) in names.clone() {
                        let view = view.clone();
                        menu = menu.item(PopupMenuItem::new(name).checked(priority == current).on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| this.set_priority(priority.clone(), window, cx));
                        }));
                    }
                    menu
                }
            });
        let tags = self.render_choices("task-tags", "work:task.addTags", tasks.tag_choices(), Self::toggle_tag, cx);
        let assignees = self.render_choices("task-assignees", "work:task.assign", tasks.assignee_choices(), Self::toggle_assignee, cx);
        let due = div().id("task-due").test_support().w(px(170.)).child(
            DatePicker::new(&self.due).small().cleanable(true).placeholder(tr(cx, "common:last.clearDate")),
        );

        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(row(tr(cx, "work:board.priority"), priority.into_any_element()))
            .child(row(tr(cx, "work:task.tags"), tags.into_any_element()))
            .child(row(tr(cx, "work:task.assignees"), assignees.into_any_element()))
            .child(row(tr(cx, "work:task.due"), due.into_any_element()))
    }

    fn render_subtask(&self, subtask: &SheetSubtask, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, success) = (theme.muted_foreground, theme.border, theme.success);
        div()
            .id(SharedString::from(format!("subtask-{}", subtask.id)))
            .test_support()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .child(
                div()
                    .id(SharedString::from(format!("subtask-tick-{}", subtask.id)))
                    .test_support()
                    .size(px(14.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(if subtask.done { success } else { border })
                    .cursor_pointer()
                    .on_click({
                        let (id, done) = (subtask.id.clone(), subtask.done);
                        cx.listener(move |this, _, window, cx| this.tick_subtask(id.clone(), !done, window, cx))
                    })
                    .children(subtask.done.then(|| {
                        div()
                            .id(SharedString::from(format!("subtask-done-{}", subtask.id)))
                            .test_support()
                            .child(Icon::new(IconName::Check).xsmall().text_color(success))
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .when(subtask.done, |this| this.text_color(muted).line_through())
                    .child(subtask.title.clone()),
            )
            .child(div().flex_none().text_xs().text_color(muted).child(format!("#{}", subtask.seq)))
    }

    fn render_comment(&self, comment: &SheetComment, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, accent) = (theme.muted_foreground, theme.border, theme.accent);
        let id = SharedString::from(format!("comment-{}", comment.id));
        // Una línea del servidor es un apunte, no alguien hablando.
        if comment.system {
            return div()
                .id(id)
                .test_support()
                .text_xs()
                .text_color(muted)
                .child(format!("{} · {}", comment.body, comment.when))
                .into_any_element();
        }
        let audience = comment.audience.as_ref().map(|key| {
            div().flex_none().px_1().rounded_sm().bg(accent).text_xs().text_color(muted).child(tr(cx, key))
        });
        div()
            .id(id)
            .test_support()
            .flex()
            .flex_col()
            .gap_1()
            .pb_3()
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(comment.author.clone()))
                    .child(div().text_xs().text_color(muted).child(comment.when.clone()))
                    .children(audience),
            )
            .child(TextView::markdown(SharedString::from(format!("comment-body-{}", comment.id)), comment.body.clone()))
            .into_any_element()
    }

    fn render_sheet(&mut self, sheet: TaskSheet, statuses: Vec<StatusOption>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let danger = theme.danger;
        let problem = self.problem.as_ref().filter(|(task_id, _)| *task_id == sheet.id).map(|(_, phrase)| phrase.clone());
        let (muted, border, primary, foreground, card, radius) =
            (theme.muted_foreground, theme.border, theme.primary, theme.foreground, theme.group_box, theme.radius);
        // La pestaña elegida era de otra tarea: en ésta se empieza por la conversación.
        let thread = if self.thread_of.as_deref() == Some(sheet.id.as_str()) { self.thread } else { Thread::Talk };
        let editing = self.editing.as_deref() == Some(sheet.id.as_str());
        let done = sheet.subtasks.iter().filter(|subtask| subtask.done).count();
        let comments = match thread {
            Thread::Talk => &sheet.talk,
            Thread::Notes => &sheet.notes,
        };
        let tabs = [
            (Thread::Talk, "thread-talk", "work:task.thread.talk", sheet.talk.len()),
            (Thread::Notes, "thread-notes", "work:task.thread.agentNotes", sheet.notes.len()),
        ];

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(if editing {
                        div().id("task-title-input").test_support().child(Input::new(&self.title_input)).into_any_element()
                    } else {
                        div().id("task-title").test_support().text_lg().font_weight(FontWeight::SEMIBOLD).child(sheet.title.clone()).into_any_element()
                    })
                    .child(div().text_xs().text_color(muted).child(sheet.place.clone()))
                    .children(sheet.parent.clone().map(|parent| div().text_xs().text_color(muted).child(format!("› {parent}")))),
            )
            .child(self.render_chips(&sheet, statuses, cx))
            .child(self.render_fields(&sheet, cx))
            .children(problem.map(|phrase| div().id("task-problem").test_support().text_sm().text_color(danger).child(phrase)))
            .children(sheet.handoff.clone().map(|handoff| {
                let by = sheet.handoff_by.clone().map(|by| format!(" · {by}")).unwrap_or_default();
                div()
                    .id("task-handoff")
                    .test_support()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_3()
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .bg(card)
                    .child(self.section(format!("{}{by}", tr(cx, "work:task.handoff")), cx))
                    .child(TextView::markdown(SharedString::from(format!("handoff-{}", sheet.id)), handoff))
            }))
            .child(
                div().flex().flex_col().gap_1().child(self.section(tr(cx, "work:task.description"), cx)).child(
                    if editing {
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().id("task-description-input").test_support().child(Textarea::new(&self.body_input)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("task-cancel")
                                            .ghost()
                                            .small()
                                            .label(tr(cx, "work:task.cancel"))
                                            .on_click(cx.listener(|this, _, _, cx| this.cancel_editing(cx))),
                                    )
                                    .child(
                                        Button::new("task-save")
                                            .primary()
                                            .small()
                                            .label(tr(cx, "work:task.save"))
                                            .on_click(cx.listener(|this, _, window, cx| this.save_edit(window, cx))),
                                    ),
                            )
                            .into_any_element()
                    } else if sheet.description.is_empty() {
                        div().id("task-no-description").test_support().text_sm().text_color(muted).child(tr(cx, "work:task.noDescription")).into_any_element()
                    } else {
                        div()
                            .id("task-description")
                            .test_support()
                            .child(TextView::markdown(SharedString::from(format!("description-{}", sheet.id)), sheet.description.clone()))
                            .into_any_element()
                    },
                ),
            )
            .when(!sheet.subtasks.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(self.section(format!("{} · {done}/{}", tr(cx, "work:task.subtasks"), sheet.subtasks.len()), cx))
                        .children(sheet.subtasks.iter().map(|subtask| self.render_subtask(subtask, cx))),
                )
            })
            .when(!sheet.attachments.is_empty(), |this| {
                this.child(
                    div()
                        .id("task-attachments")
                        .test_support()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(self.section(tr(cx, "work:task.attachments"), cx))
                        .children(sheet.attachments.iter().map(|name| div().text_sm().child(name.clone()))),
                )
            })
            .child(div().flex().items_center().gap_4().border_b_1().border_color(border).children(tabs.map(
                |(tab, id, key, count)| {
                    let active = thread == tab;
                    let task_id = sheet.id.clone();
                    div()
                        .id(id)
                        .test_support()
                        .pb_2()
                        .border_b_2()
                        .border_color(if active { primary } else { primary.opacity(0.) })
                        .text_sm()
                        .text_color(if active { foreground } else { muted })
                        .when(active, |this| this.font_weight(FontWeight::MEDIUM))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.show_thread(tab, task_id.clone(), cx)))
                        .child(format!("{} ({count})", tr(cx, key)))
                },
            )))
            .when(thread == Thread::Notes, |this| {
                this.child(div().text_xs().text_color(muted).child(tr(cx, "work:task.thread.agentNotesHint")))
            })
            .child(div().flex().flex_col().gap_3().children(comments.iter().map(|comment| self.render_comment(comment, cx))))
            // Se comenta en la conversación; las notas de agentes son suyas.
            .when(thread == Thread::Talk, |this| this.child(self.render_composer(sheet.client_reads, cx)))
    }
}

impl Render for TaskView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).task.clone();
        let sheet = Backend::core(cx).tasks().sheet(locale(cx));
        if let Some(sheet) = &sheet {
            self.show_due(sheet, window, cx);
        }
        let theme = cx.theme();
        let (background, foreground, muted, border) = (theme.background, theme.foreground, theme.muted_foreground, theme.border);
        let waiting = state.waiting_reason().map(|key| tr(cx, key));
        // Editar se ofrece con la tarea a la vista y mientras no se edita ya.
        let can_edit = sheet.as_ref().is_some_and(|sheet| self.editing.as_deref() != Some(sheet.id.as_str()));
        let edit = can_edit.then(|| {
            Button::new("task-edit")
                .ghost()
                .small()
                .label(tr(cx, "work:task.edit"))
                .on_click(cx.listener(|this, _, window, cx| this.start_editing(window, cx)))
        });
        let name = sheet
            .as_ref()
            .map(|sheet| match &sheet.reference {
                Some(reference) => format!("#{} · {reference}", sheet.seq),
                None => format!("#{}", sheet.seq),
            })
            .unwrap_or_default();

        div()
            .id("task-panel")
            .test_support()
            .w(px(TASK_PANEL_WIDTH))
            .h_full()
            .flex()
            .flex_none()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .border_l_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .child(div().flex_1().text_xs().text_color(muted).child(name))
                    .children(edit)
                    .child(
                        Button::new("task-close")
                            .ghost()
                            .small()
                            .icon(Icon::new(Lucide::X))
                            .tooltip(tr(cx, "work:task.close"))
                            .on_click(|_, _, cx| Backend::core(cx).tasks().close()),
                    ),
            )
            .child(
                div()
                    .id("task-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_4()
                    .children(waiting.map(|phrase| div().id("task-waiting").test_support().text_sm().text_color(muted).child(phrase)))
                    .children(sheet.map(|sheet| self.render_sheet(sheet, state.statuses.clone(), cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::TaskView;
    use crate::backend::Backend;
    use crate::testing::{contract_task, ok, server_with_tasks, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use gpui_kit::component::calendar::Date;
    use gpui_kit::component::date_picker::DatePickerEvent;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::Mock;

    /// El detalle del contrato del core, con otro id y un hilo de dos: lo
    /// que dijo una persona y lo que anotó un agente.
    fn detail(id: &str) -> serde_json::Value {
        let mut detail: serde_json::Value =
            serde_json::from_str(include_str!("../../core/tests/fixtures/task_detail.json")).unwrap();
        let note = detail["comments"][0].clone();
        let mut said = note.clone();
        said["id"] = json!("said");
        said["via"] = serde_json::Value::Null;
        let mut noted = note;
        noted["id"] = json!("noted");
        detail["comments"] = json!([said, noted]);
        detail["task"]["id"] = json!(id);
        detail
    }

    /// Dos tareas en la lista; la primera con su detalle y las columnas de
    /// su lista (la `s1`, donde está su subtarea, es de «hecho»). El detalle
    /// de la segunda no existe: el servidor contesta 404.
    fn server() -> FakeServer {
        server_with_column("done")
    }

    fn server_with_column(kind: &str) -> FakeServer {
        let server = server_with_tasks(vec![contract_task("t1", "a", |_| {}), contract_task("t2", "a", |_| {})]);
        let first = detail("t1");
        let mut column = first["status"].clone();
        column["kind"] = json!(kind);
        column["listId"] = first["task"]["listId"].clone();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(first)));
        server.mount(
            Mock::given(method("GET")).and(path_regex("^/api/v1/task-lists/[^/]+/statuses$")).respond_with(ok(json!([column]))),
        );
        server
    }

    fn open(server: &FakeServer, cx: &mut TestAppContext) -> AnyWindowHandle {
        open_with_view(server, cx).0
    }

    fn open_with_view(server: &FakeServer, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TaskView>) {
        server.install(cx);
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    // Ancha, para que el cajón no tape el centro de las filas
                    // que estas pruebas pulsan.
                    size: size(px(1700.), px(900.)),
                })),
                ..WindowOptions::default()
            };
            let (window, workspace) =
                gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window");
            (window, workspace.read(cx).task.clone())
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

    /// Abre la primera tarea y espera a que estén su detalle y sus columnas.
    fn open_first_task(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llegan el detalle y sus columnas", |cx| {
            cx.update(|cx| !Backend::snapshot(cx).read(cx).task.statuses.is_empty()) && on_screen(window, "task-title", cx)
        });
    }

    fn move_to(window: AnyWindowHandle, view: &Entity<TaskView>, status: &str, cx: &mut TestAppContext) {
        let status = status.to_string();
        cx.update_window(window, |_, window, cx| view.update(cx, |view, cx| view.move_to(status, window, cx))).unwrap();
    }

    fn moves(server: &FakeServer) -> Vec<String> {
        server
            .requests()
            .iter()
            .filter(|request| request.url.path() == "/api/v1/tasks/t1/move")
            .map(|request| String::from_utf8_lossy(&request.body).to_string())
            .collect()
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

    /// Pulsar una tarea abre su detalle sobre la lista, con lo suyo, y la ×
    /// lo cierra.
    ///
    /// Roturas que atrapa: una fila que no abre nada; un detalle que
    /// sustituye a la lista en vez de abrirse encima; una subtarea hecha
    /// pintada sin hacer; no tener cómo cerrarlo.
    #[gpui_kit::test]
    fn clicking_a_task_opens_its_detail_over_the_list(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        assert!(!on_screen(window, "task-panel", cx), "el panel sale sin ninguna tarea abierta");

        click(window, "work-row-t1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "task-title", cx));

        assert!(!on_screen(window, "task-waiting", cx));
        assert!(on_screen(window, "task-description", cx) && on_screen(window, "task-handoff", cx));
        assert!(on_screen(window, "task-status", cx) && on_screen(window, "subtask-t2", cx));
        assert!(on_screen(window, "work-row-t2", cx), "el detalle quitó la lista de la vista");
        wait_until(cx, "llegan las columnas y la subtarea sale hecha", |cx| on_screen(window, "subtask-done-t2", cx));

        click(window, "task-close", cx);
        wait_until(cx, "se cierra", |cx| !on_screen(window, "task-panel", cx));
        stop(cx);
    }

    /// Elegir una columna le pide al servidor mover la tarea a ésa.
    ///
    /// Roturas que atrapa: un desplegable que no manda nada; mandar otra
    /// columna; decir que falló cuando salió bien.
    #[gpui_kit::test]
    fn choosing_a_column_asks_the_server_to_move_the_task(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t1/move")).respond_with(ok(json!(null))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(moves(&server).is_empty());

        move_to(window, &view, "s1", cx);
        wait_until(cx, "llega la petición de mover", |_| !moves(&server).is_empty());

        assert!(moves(&server)[0].contains("\"statusId\":\"s1\""), "mandó {:?}", moves(&server));
        cx.run_until_parked();
        assert!(!on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// Un cambio de estado que el servidor rechaza se dice en el cajón, con
    /// su frase.
    ///
    /// Rotura que atrapa: un desplegable que parece no hacer nada cuando el
    /// cambio no se guardó.
    #[gpui_kit::test]
    fn a_move_that_fails_is_said_in_the_drawer(cx: &mut TestAppContext) {
        let server = server();
        // La primera vez lo rechaza; la segunda lo acepta.
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/tasks/t1/move"))
                .respond_with(
                    wiremock::ResponseTemplate::new(409)
                        .set_body_json(json!({ "success": false, "message": "no", "error": "bad-transition" })),
                )
                .up_to_n_times(1),
        );
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t1/move")).respond_with(ok(json!(null))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(!on_screen(window, "task-problem", cx));

        move_to(window, &view, "s1", cx);
        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));

        // Al volver a intentarlo y salir bien, el aviso de antes se quita.
        move_to(window, &view, "s1", cx);
        wait_until(cx, "el segundo intento sale bien", |cx| moves(&server).len() == 2 && !on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// Lo que no se pudo guardar en una tarea no se dice sobre otra.
    ///
    /// Rotura que atrapa: abrir una tarea y leer el fallo de la anterior.
    #[gpui_kit::test]
    fn a_failure_is_not_said_over_another_task(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        // Sin ruta de mover montada: el servidor contesta 404.
        move_to(window, &view, "s1", cx);
        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));

        click(window, "work-row-t2", cx);
        wait_until(cx, "llega la otra", |cx| {
            cx.update(|cx| Backend::snapshot(cx).read(cx).task.detail.as_ref().is_some_and(|detail| detail.task.id == "t2"))
        });

        assert!(!on_screen(window, "task-problem", cx));
        stop(cx);
    }

    fn bodies(server: &FakeServer, route: &str) -> Vec<String> {
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() == "POST" && request.url.path() == route)
            .map(|request| String::from_utf8_lossy(&request.body).to_string())
            .collect()
    }

    fn typed(view: &Entity<TaskView>, cx: &mut TestAppContext) -> String {
        cx.update(|cx| view.read(cx).comment.read(cx).value().to_string())
    }

    fn write(window: AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("comment-input", cx);
            window.input(text, cx);
        })
        .unwrap();
    }

    /// El servidor de siempre, con lo que la organización de la tarea ofrece
    /// (otra etiqueta y otra persona) y un `PATCH` que contesta la tarea.
    fn server_with_choices() -> FakeServer {
        let server = server();
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/task-tags/"))
                .respond_with(ok(json!([{ "id": "tag2", "orgId": "o1", "name": "backend", "color": "#0f0" }]))),
        );
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/users/search"))
                .respond_with(ok(json!([{ "id": "u2", "username": "luis" }]))),
        );
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(detail("t1"))));
        server
    }

    fn patches(server: &FakeServer) -> Vec<serde_json::Value> {
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() == "PATCH" && request.url.path() == "/api/v1/tasks/t1")
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect()
    }

    /// Hace algo con la vista del cajón, en su ventana.
    fn with_view(
        window: AnyWindowHandle,
        view: &Entity<TaskView>,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut TaskView, &mut gpui_kit::Window, &mut gpui_kit::Context<TaskView>),
    ) {
        cx.update_window(window, |_, window, cx| view.update(cx, |view, cx| act(view, window, cx))).unwrap();
    }

    fn day(year: i32, month: u32, day: u32) -> Date {
        Date::Single(chrono::NaiveDate::from_ymd_opt(year, month, day))
    }

    fn picked(view: &Entity<TaskView>, cx: &mut TestAppContext) -> Date {
        cx.update(|cx| view.read(cx).due.read(cx).date())
    }

    /// Elige un día en el selector, como quien lo pulsa en el calendario.
    fn pick(window: AnyWindowHandle, view: &Entity<TaskView>, date: Date, cx: &mut TestAppContext) {
        let due = cx.update(|cx| view.read(cx).due.clone());
        cx.update_window(window, |_, window, cx| {
            due.update(cx, |due, cx| {
                due.set_date(date, window, cx);
                cx.emit(DatePickerEvent::Change(due.date_time()));
            })
        })
        .unwrap();
    }

    /// Cada selector del cajón le pide al servidor su cambio, y sólo el suyo.
    ///
    /// Roturas que atrapa: un selector que no manda nada; la prioridad que
    /// manda otra; poner una etiqueta como responsable o al revés; quitar la
    /// fecha cuando se quería poner.
    #[gpui_kit::test]
    fn each_field_asks_for_its_own_change(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        wait_until(cx, "llega lo que se puede elegir", |cx| {
            cx.update(|cx| !Backend::snapshot(cx).read(cx).task.team.is_empty())
        });
        for id in ["task-priority", "task-tags", "task-assignees", "task-due"] {
            assert!(on_screen(window, id, cx), "falta {id}");
        }
        assert!(patches(&server).is_empty(), "abrir la tarea ya guardó algo: {:?}", patches(&server));

        let steps: [(Box<dyn FnOnce(&mut TaskView, &mut gpui_kit::Window, &mut gpui_kit::Context<TaskView>)>, serde_json::Value); 5] = [
            (Box::new(|view, window, cx| view.set_priority("urgent".into(), window, cx)), json!({ "priority": "urgent" })),
            (Box::new(|view, window, cx| view.toggle_tag("tag2".into(), window, cx)), json!({ "tagIds": ["tag1", "tag2"] })),
            (Box::new(|view, window, cx| view.toggle_assignee("u2".into(), window, cx)), json!({ "assigneeIds": ["u1", "u2"] })),
            (Box::new(|view, window, cx| view.set_due(Some("2026-10-02".into()), window, cx)), json!({ "dueAt": "2026-10-02T00:00:00Z" })),
            (Box::new(|view, window, cx| view.set_due(None, window, cx)), json!({ "clearDueAt": true })),
        ];
        for (at, (act, sent)) in steps.into_iter().enumerate() {
            with_view(window, &view, cx, act);
            wait_until(cx, "llega el cambio", |_| patches(&server).len() == at + 1);
            assert_eq!(patches(&server)[at], sent);
        }
        cx.run_until_parked();
        assert!(!on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// El selector de fecha enseña el día de la tarea, y elegir otro lo guarda.
    ///
    /// Roturas que atrapa: un selector vacío sobre una tarea con fecha;
    /// elegir un día y que no se guarde; pisarle lo elegido a quien elige
    /// mientras se guarda; enseñar en otra tarea el día que se eligió en
    /// ésta; guardar por el mero hecho de enseñarlo (un bucle de peticiones).
    #[gpui_kit::test]
    fn the_date_picker_shows_the_due_day_and_saves_another(cx: &mut TestAppContext) {
        let server = server_with_choices();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert_eq!(picked(&view, cx), day(2026, 9, 30));

        pick(window, &view, day(2026, 10, 2), cx);
        wait_until(cx, "se guarda el día elegido", |_| !patches(&server).is_empty());
        assert_eq!(patches(&server), [json!({ "dueAt": "2026-10-02T00:00:00Z" })]);
        // El servidor de mentira contesta la tarea como estaba, así que nada
        // nuevo hay que enseñar: lo elegido sigue en el selector.
        cx.run_until_parked();
        on_screen(window, "task-title", cx);
        assert_eq!(picked(&view, cx), day(2026, 10, 2), "se pisó lo que se acababa de elegir");

        // La otra tarea vence el 30: el selector no se queda con lo de ésta.
        click(window, "work-row-t2", cx);
        wait_until(cx, "llega la otra, con su día", |cx| {
            on_screen(window, "task-title", cx) && picked(&view, cx) == day(2026, 9, 30)
        });
        cx.run_until_parked();
        assert_eq!(patches(&server).len(), 1, "enseñar una fecha guardó: {:?}", patches(&server));
        stop(cx);
    }

    /// Si el día de la tarea cambia, el selector lo sigue.
    ///
    /// Rotura que atrapa: un selector que se queda con el día de cuando se
    /// abrió la tarea, aunque alguien se lo haya quitado.
    #[gpui_kit::test]
    fn the_date_picker_follows_the_task(cx: &mut TestAppContext) {
        let server = server();
        let mut without = detail("t1");
        without["task"]["dueAt"] = serde_json::Value::Null;
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(without)));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert_eq!(picked(&view, cx), day(2026, 9, 30));

        with_view(window, &view, cx, |view, window, cx| view.set_priority("urgent".into(), window, cx));

        wait_until(cx, "llega la tarea ya sin fecha", |cx| {
            on_screen(window, "task-title", cx) && picked(&view, cx) == Date::Single(None)
        });
        stop(cx);
    }

    /// Un cambio que no se pudo guardar se dice, y el selector de fecha
    /// vuelve al día que la tarea tiene de verdad.
    ///
    /// Roturas que atrapa: un selector que parece no hacer nada cuando el
    /// cambio falló; un selector que se queda enseñando el día que no se
    /// guardó.
    #[gpui_kit::test]
    fn a_change_that_fails_is_said_and_the_date_goes_back(cx: &mut TestAppContext) {
        // Sin `PATCH` montado: el servidor contesta 404.
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(!on_screen(window, "task-problem", cx));

        pick(window, &view, day(2026, 10, 2), cx);

        wait_until(cx, "dice que no se pudo y vuelve al día guardado", |cx| {
            on_screen(window, "task-problem", cx) && picked(&view, cx) == day(2026, 9, 30)
        });
        stop(cx);
    }

    fn written(view: &Entity<TaskView>, cx: &mut TestAppContext) -> (String, String) {
        cx.update(|cx| {
            let view = view.read(cx);
            (view.title_input.read(cx).value().to_string(), view.body_input.read(cx).value().to_string())
        })
    }

    fn rewrite(window: AnyWindowHandle, view: &Entity<TaskView>, title: &str, body: &str, cx: &mut TestAppContext) {
        let (title, body) = (title.to_string(), body.to_string());
        with_view(window, view, cx, |view, window, cx| {
            view.title_input.update(cx, |input, cx| input.set_value(title, window, cx));
            view.body_input.update(cx, |input, cx| input.set_value(body, window, cx));
        });
    }

    /// «Editar» pone el título y la descripción en sus cajas con lo que la
    /// tarea dice, y «Guardar» manda lo escrito y vuelve a la lectura.
    ///
    /// Roturas que atrapa: unas cajas vacías sobre una tarea con texto (y
    /// guardar la borra); un «Guardar» que no manda, o que manda sólo el
    /// título; quedarse editando después de guardar.
    #[gpui_kit::test]
    fn editing_starts_from_what_the_task_says_and_saves_what_was_written(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(!on_screen(window, "task-title-input", cx) && !on_screen(window, "task-save", cx));

        click(window, "task-edit", cx);
        assert!(on_screen(window, "task-title-input", cx) && on_screen(window, "task-description-input", cx));
        assert!(!on_screen(window, "task-title", cx) && !on_screen(window, "task-edit", cx));
        assert_eq!(written(&view, cx), ("Subir el backend".to_string(), "Con **markdown**.".to_string()));

        rewrite(window, &view, "Otro título", "Otra descripción", cx);
        click(window, "task-save", cx);
        wait_until(cx, "se guardan los dos y vuelve a la lectura", |cx| {
            patches(&server).len() == 2 && on_screen(window, "task-title", cx)
        });

        assert_eq!(patches(&server), [json!({ "title": "Otro título" }), json!({ "description": "Otra descripción" })]);
        assert!(!on_screen(window, "task-title-input", cx) && !on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// «Cancelar» tira lo escrito: no se manda, y al volver a editar las
    /// cajas dicen otra vez lo que dice la tarea.
    ///
    /// Roturas que atrapa: un «Cancelar» que guarda; volver a editar y
    /// encontrarse lo que se había descartado.
    #[gpui_kit::test]
    fn cancelling_throws_away_what_was_written(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        click(window, "task-edit", cx);
        rewrite(window, &view, "No", "Tampoco", cx);

        click(window, "task-cancel", cx);
        cx.run_until_parked();
        assert!(on_screen(window, "task-title", cx) && !on_screen(window, "task-title-input", cx));
        assert!(patches(&server).is_empty(), "cancelar guardó {:?}", patches(&server));

        click(window, "task-edit", cx);
        assert_eq!(written(&view, cx).0, "Subir el backend");
        stop(cx);
    }

    /// Si no se pudo guardar, se dice y se sigue editando con lo escrito.
    ///
    /// Rotura que atrapa: perder una descripción entera por un fallo de red.
    #[gpui_kit::test]
    fn an_edit_that_fails_keeps_what_was_written(cx: &mut TestAppContext) {
        // Sin `PATCH` montado: el servidor contesta 404.
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        click(window, "task-edit", cx);
        rewrite(window, &view, "Otro título", "Mucho texto", cx);

        click(window, "task-save", cx);

        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        assert!(on_screen(window, "task-title-input", cx));
        assert_eq!(written(&view, cx), ("Otro título".to_string(), "Mucho texto".to_string()));
        stop(cx);
    }

    /// Al abrir otra tarea no se sigue editando: lo escrito era de la anterior.
    ///
    /// Rotura que atrapa: guardar en una tarea el título que se escribía
    /// para otra.
    #[gpui_kit::test]
    fn another_task_is_not_opened_in_edit_mode(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        let (window, _view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        click(window, "task-edit", cx);
        assert!(on_screen(window, "task-title-input", cx));

        click(window, "work-row-t2", cx);
        wait_until(cx, "llega la otra", |cx| {
            cx.update(|cx| Backend::snapshot(cx).read(cx).task.detail.as_ref().is_some_and(|detail| detail.task.id == "t2"))
        });

        assert!(on_screen(window, "task-title", cx) && !on_screen(window, "task-title-input", cx));
        assert!(on_screen(window, "task-edit", cx));
        stop(cx);
    }

    /// Pulsar la casilla de una subtarea sin hacer la manda a la columna de
    /// «hecho» de su lista.
    ///
    /// Roturas que atrapa: una casilla que no hace nada; mover la tarea madre
    /// en vez de la subtarea; desmarcar lo que se quería marcar.
    #[gpui_kit::test]
    fn ticking_a_subtask_moves_it_to_done(cx: &mut TestAppContext) {
        let mut done = detail("t1")["status"].clone();
        done["id"] = json!("s-done");
        done["kind"] = json!("done");
        done["status"] = json!("resolved");
        // Dos columnas: la abierta, donde está la subtarea, y la de «hecho».
        let mut open_column = detail("t1")["status"].clone();
        open_column["kind"] = json!("active");
        let server_columns = json!([open_column, done]);
        let server = {
            let server = server_with_tasks(vec![contract_task("t1", "a", |_| {})]);
            server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(detail("t1"))));
            server.mount(
                Mock::given(method("GET")).and(path_regex("^/api/v1/task-lists/[^/]+/statuses$")).respond_with(ok(server_columns)),
            );
            server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t2/move")).respond_with(ok(json!(null))));
            server
        };
        let window = open(&server, cx);
        open_first_task(window, cx);
        assert!(!on_screen(window, "subtask-done-t2", cx));

        click(window, "subtask-tick-t2", cx);
        wait_until(cx, "pide mover la subtarea", |_| !bodies(&server, "/api/v1/tasks/t2/move").is_empty());

        assert!(bodies(&server, "/api/v1/tasks/t2/move")[0].contains("\"statusId\":\"s-done\""));
        assert!(bodies(&server, "/api/v1/tasks/t1/move").is_empty(), "movió la tarea madre");
        stop(cx);
    }

    /// Si la casilla no se puede guardar, se dice.
    ///
    /// Rotura que atrapa: una casilla que parece no hacer nada (aquí la
    /// lista no tiene columna de «pendiente» a la que devolverla).
    #[gpui_kit::test]
    fn a_tick_that_cannot_be_saved_is_said(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        open_first_task(window, cx);
        assert!(on_screen(window, "subtask-done-t2", cx));

        click(window, "subtask-tick-t2", cx);

        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// Lo escrito se manda como comentario y la caja se vacía; donde un
    /// cliente lee, se puede dejar en interno antes de mandar.
    ///
    /// Roturas que atrapa: un botón que no manda; mandar otra cosa que lo
    /// escrito; una caja que se queda con lo ya mandado (y se manda dos
    /// veces); un interruptor de «interno» que no llega al servidor.
    #[gpui_kit::test]
    fn what_is_written_is_sent_and_can_be_kept_internal(cx: &mut TestAppContext) {
        let server = server();
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/tasks/t1/comments"))
                .respond_with(wiremock::ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": detail("t1") }))),
        );
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        let sent = |server: &FakeServer| bodies(server, "/api/v1/tasks/t1/comments");

        write(window, "para todos", cx);
        click(window, "comment-send", cx);
        wait_until(cx, "llega y la caja se vacía", |cx| sent(&server).len() == 1 && typed(&view, cx).is_empty());
        assert_eq!(sent(&server)[0], r#"{"body":"para todos"}"#);

        click(window, "comment-audience", cx);
        write(window, "entre nosotros", cx);
        click(window, "comment-send", cx);
        wait_until(cx, "llega el segundo", |_| sent(&server).len() == 2);
        assert!(sent(&server)[1].contains(r#""visibility":"internal""#), "mandó {:?}", sent(&server)[1]);

        // Con la caja vacía no se manda nada, y tampoco es un fallo que avisar.
        wait_until(cx, "la caja se vacía", |cx| typed(&view, cx).is_empty());
        click(window, "comment-send", cx);
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(150));
        cx.run_until_parked();
        assert_eq!(sent(&server).len(), 2);
        assert!(!on_screen(window, "task-problem", cx), "avisa de un fallo por pulsar con la caja vacía");
        stop(cx);
    }

    /// Un comentario que no se pudo mandar se dice, y lo escrito se queda.
    ///
    /// Rotura que atrapa: perder lo escrito por un fallo de red.
    #[gpui_kit::test]
    fn a_comment_that_fails_keeps_what_was_written(cx: &mut TestAppContext) {
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);

        write(window, "no se pierde", cx);
        click(window, "comment-send", cx);

        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        assert_eq!(typed(&view, cx), "no se pierde");
        stop(cx);
    }

    /// Una subtarea que no está hecha no lleva la marca.
    ///
    /// Rotura que atrapa: un checklist entero marcado, hecho o no.
    #[gpui_kit::test]
    fn a_subtask_that_is_not_done_is_not_ticked(cx: &mut TestAppContext) {
        let server = server_with_column("active");
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llegan el detalle y sus columnas", |cx| {
            cx.update(|cx| !Backend::snapshot(cx).read(cx).task.statuses.is_empty()) && on_screen(window, "subtask-t2", cx)
        });

        assert!(!on_screen(window, "subtask-done-t2", cx));
        stop(cx);
    }

    /// Las notas de agentes están en su pestaña, aparte de la conversación.
    ///
    /// Roturas que atrapa: las dos cosas mezcladas en un hilo; una pestaña
    /// que cambia de color y enseña lo mismo.
    #[gpui_kit::test]
    fn agent_notes_are_behind_their_own_tab(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "comment-said", cx));
        assert!(!on_screen(window, "comment-noted", cx), "la nota del agente sale en la conversación");

        click(window, "thread-notes", cx);

        assert!(on_screen(window, "comment-noted", cx));
        assert!(!on_screen(window, "comment-said", cx), "la conversación sigue bajo la pestaña de notas");
        stop(cx);
    }

    /// Al abrir otra tarea se empieza por su conversación, aunque en la
    /// anterior se estuvieran leyendo las notas de agentes.
    ///
    /// Rotura que atrapa: abrir una tarea y encontrarse las notas de un
    /// agente en vez de lo que le han dicho a uno.
    #[gpui_kit::test]
    fn another_task_starts_on_its_conversation(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "comment-said", cx));
        click(window, "thread-notes", cx);
        assert!(on_screen(window, "comment-noted", cx));

        click(window, "work-row-t2", cx);
        wait_until(cx, "llega la otra", |cx| {
            cx.update(|cx| Backend::snapshot(cx).read(cx).task.detail.as_ref().is_some_and(|detail| detail.task.id == "t2"))
        });

        assert!(on_screen(window, "comment-said", cx) && !on_screen(window, "comment-noted", cx));
        stop(cx);
    }

    /// Una tarea que no llega lo dice, y no deja a la vista la que se leía.
    ///
    /// Roturas que atrapa: un panel en blanco para siempre; el título de la
    /// tarea anterior sobre un detalle que no es el suyo.
    #[gpui_kit::test]
    fn a_task_that_does_not_arrive_says_so(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "task-title", cx));

        click(window, "work-row-t2", cx);
        wait_until(cx, "falla y lo dice", |cx| {
            cx.update(|cx| Backend::snapshot(cx).read(cx).task.error.is_some()) && on_screen(window, "task-waiting", cx)
        });

        assert!(!on_screen(window, "task-title", cx), "sigue a la vista la tarea anterior");
        stop(cx);
    }
}
