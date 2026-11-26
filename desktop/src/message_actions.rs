//! Lo que se hace con un mensaje: responder, reaccionar, y con lo propio,
//! corregirlo y retirarlo.
//!
//! Qué se puede hacer con cada uno, lo que se manda y la pregunta de
//! retirar los decide el core (`ThreadLine::can_edit`, `Chat::edit`,
//! `Chat::withdraw`, `Chat::react`, `Chat::withdraw_question`). Aquí queda
//! cómo se pinta: la barra de acciones en la esquina del mensaje al pasar
//! por él, el selector de reacciones, y la caja de corregir en su sitio.
//!
//! Al corregir se edita el markdown del mensaje tal cual: una mención sale
//! como su enlace (`[@ana](cac:user/…)`), no como ficha. Rehacer las fichas
//! al corregir queda para después.
//!
//! El botón «más» lleva lo que se hace con lo hablado: una tarea, o un
//! párrafo en la documentación de la lista (`Chat::task_from_message`,
//! `Chat::save_message_to_docs`).

use cac_core::{Conversation, ThreadLine, REACTION_EMOJIS};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter, Escape, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme, Icon, Selectable as _, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::chat::ChatView;
use crate::lang::{error_text, locale, tr};
use crate::theme::Leading as _;

/// Un mensaje que se está corrigiendo.
pub struct Editing {
    pub(crate) id: String,
    field: Entity<TextareaState>,
    /// Lo que salió mal al guardar.
    problem: Option<String>,
}

impl ChatView {
    fn conversation(&self) -> Option<Conversation> {
        self.composer_conversation()
    }

    /// Abre la caja de corregir en el sitio del mensaje, con su texto.
    fn start_edit(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(body) = self.messages.iter().find(|message| message.id == id).map(|message| message.body.clone()) else { return };
        let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 12));
        field.update(cx, |field, cx| {
            field.set_value(body.clone(), window, cx);
            let end = body.len();
            field.set_selected_range(end..end, cx);
            field.focus(window, cx);
        });
        self.picking = None;
        self.editing = Some(Editing { id, field, problem: None });
        cx.notify();
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.editing.take().is_none() {
            return false;
        }
        self.composer.field.update(cx, |field, cx| field.focus(window, cx));
        cx.notify();
        true
    }

    /// Guarda la corrección. Si sale bien se cierra la caja; si no, lo
    /// escrito se queda y se dice por qué.
    fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(editing), Some(conversation)) = (self.editing.as_ref(), self.conversation()) else { return };
        let (id, body) = (editing.id.clone(), editing.field.read(cx).value().to_string());
        let answer = Backend::ask(cx, move |core| async move { core.chat().edit(conversation, id, body).await });
        let edited = editing.id.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                if this.editing.as_ref().map(|editing| editing.id.as_str()) != Some(edited.as_str()) {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.cancel_edit(window, cx);
                    }
                    Err(error) => {
                        let said = error_text(cx, &error, "common:misc.errSave");
                        if let Some(editing) = this.editing.as_mut() {
                            editing.problem = Some(said);
                        }
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Crea una tarea con el mensaje. En la lista de un cliente se pregunta
    /// antes si la ve él: saltarse la pregunta «porque es el chat» es el
    /// único camino que filtraría lo interno.
    fn task_from_message(&mut self, id: String, list_id: String, client: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.conversation() else { return };
        let locale = locale(cx);
        let chat = Backend::core(cx).chat();
        let asked = client.then(|| {
            let question = chat.client_sees_question(locale);
            window.prompt(PromptLevel::Info, &question.title, Some(&question.body), &[question.confirm.as_str(), question.cancel.as_str()], cx)
        });
        cx.spawn_in(window, async move |this, cx| {
            let internal = match asked {
                Some(answer) => answer.await != Ok(0),
                None => false,
            };
            let Ok(done) = cx.update(|_, cx| Backend::ask(cx, move |core| async move { core.chat().task_from_message(conversation, id, list_id, internal, locale).await })) else { return };
            let Ok(result) = done.await else { return };
            this.update(cx, |this, cx| {
                let said = match result {
                    Ok(()) => tr(cx, "chat:taskCreated"),
                    Err(error) => error_text(cx, &error, "work:board.errCreate"),
                };
                this.say(said, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Guarda el mensaje, citado, en una pestaña de la documentación de la lista.
    fn save_to_docs(&mut self, id: String, list_id: String, tab: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.conversation() else { return };
        let locale = locale(cx);
        let done = Backend::ask(cx, move |core| async move { core.chat().save_message_to_docs(conversation, id, list_id, tab.to_string(), locale).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = done.await else { return };
            this.update(cx, |this, cx| {
                let said = match result {
                    Ok(()) => tr(cx, "work:docs.savedToDoc"),
                    Err(error) => error_text(cx, &error, "work:docs.errSave"),
                };
                this.say(said, cx);
            })
            .ok();
        })
        .detach();
    }

    /// El botón «más» de un mensaje de un canal: una tarea o un párrafo de
    /// documentación por cada lista del espacio. Sin ninguna lista, dice por
    /// qué no hay nada que hacer.
    fn render_more(&self, id: &str, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let conversation = self.conversation().filter(|conversation| matches!(conversation, Conversation::Channel { .. }))?;
        let targets = Backend::core(cx).chat().work_targets(&conversation);
        let (create, save, nothing) = (tr(cx, "chat:createTask"), tr(cx, "work:docs.saveToDocs"), tr(cx, "chat:noListInChannel"));
        let tabs = [("overview", tr(cx, "work:docs.overview")), ("runbook", tr(cx, "work:docs.runbook")), ("links", tr(cx, "work:docs.links"))];
        let view = cx.entity().downgrade();
        let message = id.to_string();
        Some(
            Button::new(SharedString::from(format!("thread-more-{id}")))
                .ghost()
                .xsmall()
                .icon(Icon::new(Lucide::Ellipsis).size_4())
                .tooltip(tr(cx, "chat:moreActions"))
                .dropdown_menu(move |mut menu, _, _| {
                    if targets.is_empty() {
                        return menu.label(nothing.clone());
                    }
                    let several = targets.len() > 1;
                    for target in targets.clone() {
                        if several {
                            menu = menu.label(target.label.clone());
                        }
                        let (list_id, client, message_id) = (target.list_id.clone(), target.client, message.clone());
                        let task_view = view.clone();
                        menu = menu.item(PopupMenuItem::new(create.clone()).on_click(move |_, window, cx| {
                            let (list_id, message_id) = (list_id.clone(), message_id.clone());
                            let _ = task_view.update(cx, |this, cx| this.task_from_message(message_id, list_id, client, window, cx));
                        }));
                        for (tab, label) in tabs.clone() {
                            let (list_id, message_id, docs_view) = (target.list_id.clone(), message.clone(), view.clone());
                            menu = menu.item(PopupMenuItem::new(format!("{save} · {label}")).on_click(move |_, window, cx| {
                                let (list_id, message_id) = (list_id.clone(), message_id.clone());
                                let _ = docs_view.update(cx, |this, cx| this.save_to_docs(message_id, list_id, tab, window, cx));
                            }));
                        }
                    }
                    menu
                }),
        )
    }

    /// Retira un mensaje propio, preguntando antes: siempre.
    fn ask_withdraw(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.conversation() else { return };
        let question = Backend::core(cx).chat().withdraw_question(&conversation, locale(cx));
        let answer = window.prompt(PromptLevel::Warning, &question.title, Some(&question.body), &[question.confirm.as_str(), question.cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let Ok(done) = cx.update(|_, cx| Backend::ask(cx, move |core| async move { core.chat().withdraw(conversation, id).await })) else { return };
            let Ok(Err(error)) = done.await else { return };
            this.update(cx, |this, cx| {
                let said = error_text(cx, &error, "common:misc.errSave");
                this.say(said, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Pone o quita la reacción de quien mira. Se ve en el acto (lo hace el
    /// core); si el servidor dice que no, se dice.
    pub(crate) fn react(&mut self, id: String, emoji: String, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.conversation() else { return };
        self.picking = None;
        let answer = Backend::ask(cx, move |core| async move { core.chat().react(conversation, id, emoji, on).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Err(_)) = answer.await else { return };
            this.update(cx, |this, cx| {
                let said = tr(cx, "chat:reactFailed");
                this.say(said, cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Lo que la burbuja deja libre a su derecha para la barra de acciones:
    /// tres botones (responder, reaccionar, más) o cinco si el mensaje es propio.
    pub(crate) fn actions_gutter(line: &ThreadLine, can_write: bool) -> Pixels {
        match (line.system, can_write, line.can_edit) {
            (_, false, _) => px(28.),
            // Sólo la carita: en una línea de cac no se responde ni se corrige.
            (true, true, _) => px(36.),
            (false, true, false) => px(100.),
            (false, true, true) => px(152.),
        }
    }

    /// La barra de acciones de un mensaje, en su esquina: se ve al pasar
    /// por él, o mientras se elige una reacción. En una línea de cac sólo está
    /// la carita; quien no puede escribir no tiene barra.
    pub(crate) fn render_actions(&self, line: &ThreadLine, group: SharedString, can_write: bool, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !can_write {
            return None;
        }
        let id = line.id.clone();
        let picking = self.picking.as_deref() == Some(id.as_str());
        let (popover, border) = (cx.theme().popover, cx.theme().border);
        let (replied, reacted, edited, withdrawn) = (line.clone(), id.clone(), id.clone(), id.clone());
        Some(
            div()
                // Dentro de la burbuja, arriba a la derecha (como en Tauri):
                // sobre el borde de arriba caía en el hueco entre dos
                // mensajes y no se sabía de cuál era. La burbuja reserva a su
                // derecha lo que ocupa (`actions_gutter`) para no tapar el
                // texto de un mensaje corto.
                .absolute()
                .top_1()
                .right_1()
                .flex()
                .gap_0p5()
                .rounded_md()
                .border_1()
                .border_color(border)
                .bg(popover)
                .shadow_sm()
                .map(|this| if picking { this.opacity(1.) } else { this.opacity(0.).group_hover(group, |style| style.opacity(1.)) })
                .when(!line.system, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("thread-reply-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::CornerUpLeft).size_4())
                            .tooltip(tr(cx, "chat:reply"))
                            .on_click(cx.listener(move |this, _, window, cx| this.reply_to(&replied, window, cx))),
                    )
                })
                .child(
                    Button::new(SharedString::from(format!("thread-react-{id}")))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::HeartPlus).size_4())
                        .tooltip(tr(cx, "chat:react"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.picking = if this.picking.as_deref() == Some(reacted.as_str()) { None } else { Some(reacted.clone()) };
                            cx.notify();
                        })),
                )
                .when(!line.system, |this| this.children(self.render_more(&id, cx)))
                .when(line.can_edit, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("thread-edit-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::Pencil).size_4())
                            .tooltip(tr(cx, "chat:edit"))
                            .on_click(cx.listener(move |this, _, window, cx| this.start_edit(edited.clone(), window, cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("thread-withdraw-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::Trash).size_4())
                            .tooltip(tr(cx, "chat:withdraw"))
                            .on_click(cx.listener(move |this, _, window, cx| this.ask_withdraw(withdrawn.clone(), window, cx))),
                    )
                }),
        )
    }

    /// Las ocho reacciones del juego, para elegir una, bajo el mensaje.
    pub(crate) fn render_picker(&self, line: &ThreadLine, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.picking.as_deref() != Some(line.id.as_str()) {
            return None;
        }
        let theme = cx.theme();
        let (border, popover) = (theme.border, theme.popover);
        Some(
            div()
                .id(SharedString::from(format!("thread-picker-{}", line.id)))
                .mt_1()
                // Del ancho de sus ocho, del lado del mensaje.
                .map(|this| if line.mine { this.self_end() } else { this.self_start() })
                .flex()
                .gap_0p5()
                .p_1()
                .rounded_full()
                .border_1()
                .border_color(border)
                .bg(popover)
                .shadow_md()
                .children(REACTION_EMOJIS.iter().enumerate().map(|(index, emoji)| {
                    let mine = line.reactions.iter().any(|reaction| reaction.emoji == *emoji && reaction.mine);
                    let (id, emoji) = (line.id.clone(), emoji.to_string());
                    Button::new(SharedString::from(format!("thread-pick-{}-{index}", line.id)))
                        .ghost()
                        .small()
                        .child(crate::assets::emoji(&emoji, px(20.)))
                        .selected(mine)
                        .on_click(cx.listener(move |this, _, window, cx| this.react(id.clone(), emoji.clone(), !mine, window, cx)))
                }))
                .test_support(),
        )
    }

    /// La caja de corregir, en el sitio del texto del mensaje que se
    /// corrige: Enter guarda, Mayús+Enter salta de línea, Escape deja como
    /// estaba.
    pub(crate) fn render_editor(&self, line: &ThreadLine, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editing = self.editing.as_ref().filter(|editing| editing.id == line.id)?;
        let theme = cx.theme();
        let (border, danger) = (theme.border, theme.danger);
        Some(
            div()
                .id(SharedString::from(format!("thread-editing-{}", line.id)))
                .w(px(480.))
                .max_w_full()
                .flex()
                .flex_col()
                .gap_1()
                .capture_action(cx.listener(|this, enter: &Enter, window, cx| {
                    if !enter.shift && !enter.secondary {
                        cx.stop_propagation();
                        this.save_edit(window, cx);
                    }
                }))
                .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                    if this.cancel_edit(window, cx) {
                        cx.stop_propagation();
                    }
                }))
                .child(div().rounded(px(8.)).border_1().border_color(border).bg(crate::theme::field_background(cx)).px_1().child(Textarea::new(&editing.field).appearance(false)))
                .children(editing.problem.clone().map(|said| div().id("thread-edit-failed").leading_xs().text_color(danger).aria_label(said.clone()).child(said).test_support()))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(Button::new("thread-edit-cancel").ghost().xsmall().label(tr(cx, "chat:cancel")).on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_edit(window, cx);
                        })))
                        .child(Button::new("thread-edit-save").primary().xsmall().label(tr(cx, "chat:save")).on_click(cx.listener(|this, _, window, cx| this.save_edit(window, cx)))),
                )
                .test_support()
                .into_any_element(),
        )
    }
}
