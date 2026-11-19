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

use cac_core::{Conversation, ThreadLine, REACTION_EMOJIS};
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

    /// La barra de acciones de un mensaje, en su esquina: se ve al pasar
    /// por él, o mientras se elige una reacción. Una línea de cac no tiene;
    /// quien no puede escribir, tampoco.
    pub(crate) fn render_actions(&self, line: &ThreadLine, group: SharedString, can_write: bool, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if line.system || !can_write {
            return None;
        }
        let id = line.id.clone();
        let picking = self.picking.as_deref() == Some(id.as_str());
        let (popover, border) = (cx.theme().popover, cx.theme().border);
        let (replied, reacted, edited, withdrawn) = (line.clone(), id.clone(), id.clone(), id.clone());
        Some(
            div()
                // Flota sobre el borde de arriba del mensaje: dentro de un
                // mensaje corto taparía lo que dice.
                .absolute()
                .top(px(-16.))
                .right_2()
                .flex()
                .gap_0p5()
                .p_0p5()
                .rounded_md()
                .border_1()
                .border_color(border)
                .bg(popover)
                .shadow_sm()
                .map(|this| if picking { this.opacity(1.) } else { this.opacity(0.).group_hover(group, |style| style.opacity(1.)) })
                .child(
                    Button::new(SharedString::from(format!("thread-reply-{id}")))
                        .ghost()
                        .small()
                        .icon(Icon::new(Lucide::CornerUpLeft).size_4())
                        .tooltip(tr(cx, "chat:reply"))
                        .on_click(cx.listener(move |this, _, window, cx| this.reply_to(&replied, window, cx))),
                )
                .child(
                    Button::new(SharedString::from(format!("thread-react-{id}")))
                        .ghost()
                        .small()
                        .icon(Icon::new(Lucide::HeartPlus).size_4())
                        .tooltip(tr(cx, "chat:react"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.picking = if this.picking.as_deref() == Some(reacted.as_str()) { None } else { Some(reacted.clone()) };
                            cx.notify();
                        })),
                )
                .when(line.can_edit, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("thread-edit-{id}")))
                            .ghost()
                            .small()
                            .icon(Icon::new(Lucide::Pencil).size_4())
                            .tooltip(tr(cx, "chat:edit"))
                            .on_click(cx.listener(move |this, _, window, cx| this.start_edit(edited.clone(), window, cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("thread-withdraw-{id}")))
                            .ghost()
                            .small()
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
                        .label(emoji.clone())
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
