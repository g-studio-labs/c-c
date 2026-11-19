//! La caja de escribir de una conversación (canal o directo).
//!
//! Es el campo de varias líneas del kit con sus fichas dentro del texto: una
//! mención se ve como «@Ana Ruiz» y vale `[@ana](cac:user/<uuid>)`, que es lo
//! que el servidor busca para avisar. Así lo que vale la caja ya es el
//! markdown que se manda, y el kit se ocupa de que la ficha se borre entera,
//! se salte con el cursor y vuelva al deshacer (probado en
//! `spike::tests::the_kit_field_keeps_a_mention_as_one_token`).
//!
//! Lo que se manda, cuándo Enter manda y a quién se puede nombrar lo decide
//! el core (`Chat::send`, `enter_sends`, `Chat::mention_menu`). Aquí queda
//! lo de una pantalla: un borrador por conversación —con sus fichas y su
//! respuesta—, la lista de menciones y lo que se dice si algo falla.
//!
//! Un fichero pegado, soltado o elegido con el clip se sube a la
//! conversación y queda como ficha con su nombre; vale su cita
//! (`![captura.png](…/raw)`), que es lo único que une el fichero al mensaje.
//!
//! Distinto de la app de siempre, a propósito: el borrador no se pierde al
//! cambiar de canal ni pasa a otro directo, al mandar se baja al final, y no
//! se manda con un fichero a medio subir (allí su enlace caía en el
//! borrador siguiente).

use std::rc::Rc;

use cac_core::{ApiError, ComposerToken, Conversation, MentionMenu, MentionPerson, ThreadLine};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter, Escape, IndentInline, InlineToken, InlineTokenError, InputContent, InputEvent, MoveDown, MoveUp, Textarea, TextareaState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::chat::ChatView;
use crate::editor::{pasted, read_incoming, Incoming};
use crate::lang::{error_text, locale, tr};
use crate::mentions::{render_mentions, MentionList};
use crate::theme::Leading as _;

/// Cuántas líneas crece la caja antes de desplazarse por dentro.
const MAX_LINES: usize = 8;

/// A qué mensaje se responde: lo que dice la barra encima de la caja.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyTo {
    pub id: String,
    /// Quién lo escribió, como lo dice la conversación («Tú» si es propio).
    pub author: String,
    /// Lo que decía, en una línea.
    pub text: String,
}

/// Un fichero que se está subiendo, y a qué conversación.
struct Upload {
    serial: u64,
    conversation: Conversation,
    name: String,
}

/// La ficha del kit para una del core. El id dice qué es, para pintarla:
/// `user-…` una persona, `file-…` un fichero.
fn inline_token(token: ComposerToken, kind: &str) -> InlineToken {
    InlineToken::new(format!("{kind}-{}", token.id), token.text).with_label(token.label)
}

/// Pone esa ficha donde está el cursor, y un espacio detrás para seguir
/// escribiendo. Con una tecla muerta a medio componer el kit no pone fichas:
/// se cierra la composición antes.
fn put_token(field: &mut TextareaState, range: Option<std::ops::Range<usize>>, token: InlineToken, window: &mut Window, cx: &mut Context<TextareaState>) {
    let put = |field: &mut TextareaState, window: &mut Window, cx: &mut Context<TextareaState>| match &range {
        Some(range) => field.replace_range_with_token(range.clone(), token.clone(), window, cx),
        None => field.replace_with_token(token.clone(), window, cx),
    };
    // Un fichero pegado a una palabra se separa de ella.
    if range.is_none() {
        let (text, cursor) = (field.value(), field.cursor());
        if text[..cursor].chars().next_back().is_some_and(|before| !before.is_whitespace()) {
            field.replace(" ", window, cx);
        }
    }
    if put(field, window, cx) == Err(InlineTokenError::CompositionActive) {
        field.unmark_text(window, cx);
        let _ = put(field, window, cx);
    }
    field.replace(" ", window, cx);
}

/// Cómo se llama lo que llegó, antes de leerlo.
fn incoming_name(file: &Incoming) -> String {
    match file {
        Incoming::Path(path) => path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
        Incoming::Picture { name, .. } => name.clone(),
    }
}

/// Lo que se deja en una conversación al irse a otra.
struct Draft {
    conversation: Conversation,
    content: InputContent,
    reply: Option<ReplyTo>,
}

/// La caja y lo que recuerda.
pub struct Composer {
    pub(crate) field: Entity<TextareaState>,
    /// De qué conversación es lo que hay ahora en la caja.
    of: Option<Conversation>,
    /// De quién es todo esto: al salir de la sesión no queda nada.
    owner: Option<String>,
    /// Lo escrito en las demás conversaciones.
    drafts: Vec<Draft>,
    /// A qué se responde en la conversación de la caja.
    pub(crate) reply: Option<ReplyTo>,
    mentions: MentionList,
    /// Lo que salió mal al mandar en la conversación de la caja.
    error: Option<String>,
    /// El texto de ayuda puesto: cambia con la conversación.
    placeholder: String,
    /// Los ficheros que se están subiendo, de esta conversación o de otras.
    uploading: Vec<Upload>,
    /// Los que ya subieron y esperan a que su conversación esté en la caja
    /// para ponerse como ficha.
    landed: Vec<(Conversation, InlineToken)>,
    uploads: u64,
    _on_change: Subscription,
}

impl Composer {
    pub fn new(window: &mut Window, cx: &mut Context<ChatView>) -> Self {
        let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, MAX_LINES));
        // Lo escrito mueve la lista de menciones y el botón de mandar.
        let on_change = cx.subscribe(&field, |this: &mut ChatView, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let (text, cursor) = this.composer.written(cx);
                this.composer.mentions.keep(&text, cursor);
                cx.notify();
            }
        });
        Self { field, of: None, owner: None, drafts: Vec::new(), reply: None, mentions: MentionList::default(), error: None, placeholder: String::new(), uploading: Vec::new(), landed: Vec::new(), uploads: 0, _on_change: on_change }
    }

    /// Lo escrito y dónde está el cursor, en bytes.
    fn written(&self, cx: &App) -> (String, usize) {
        let field = self.field.read(cx);
        (field.value().to_string(), field.cursor())
    }

    /// Pone en la caja lo de esa conversación y guarda lo de la anterior. Al
    /// cambiar de sesión se olvida todo.
    pub(crate) fn follow(&mut self, to: Option<Conversation>, owner: Option<String>, window: &mut Window, cx: &mut App) {
        if self.owner != owner {
            self.owner = owner;
            self.drafts.clear();
            self.uploading.clear();
            self.landed.clear();
            self.of = None;
            self.reply = None;
            self.error = None;
            self.mentions.reset();
            self.field.update(cx, |field, cx| field.set_value("", window, cx));
        }
        if self.of == to {
            return;
        }
        let content = self.field.read(cx).content();
        if let Some(left) = self.of.take() {
            self.drafts.retain(|draft| draft.conversation != left);
            if !content.text().trim().is_empty() || self.reply.is_some() {
                self.drafts.push(Draft { conversation: left, content, reply: self.reply.take() });
            }
        }
        let found = to.as_ref().and_then(|to| self.drafts.iter().position(|draft| &draft.conversation == to)).map(|at| self.drafts.remove(at));
        let (content, reply) = match found {
            Some(draft) => (draft.content, draft.reply),
            None => (InputContent::new(""), None),
        };
        self.reply = reply;
        self.error = None;
        self.mentions.reset();
        // Se vuelve a escribir donde se acaba lo escrito.
        self.field.update(cx, |field, cx| {
            field.set_value(content, window, cx);
            let end = field.value().len();
            field.set_selected_range(end..end, cx);
        });
        self.of = to;
    }
}

impl ChatView {
    /// De qué conversación es la caja: la que se tiene delante.
    pub(crate) fn composer_conversation(&self) -> Option<Conversation> {
        self.composer.of.clone()
    }

    /// La lista de a quién nombrar, si se está escribiendo un `@algo` donde
    /// se puede.
    fn mention_menu(&self, cx: &App) -> Option<MentionMenu> {
        let conversation = self.composer.of.as_ref()?;
        let (text, cursor) = self.composer.written(cx);
        Backend::core(cx).chat().mention_menu(conversation, &text, cursor as u32, self.composer.mentions.mark.clone())
    }

    /// Cambia el `@algo` que se escribía por la ficha de esa persona, y un
    /// espacio detrás para seguir escribiendo.
    pub(crate) fn pick_mention(&mut self, person: &MentionPerson, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.mention_menu(cx) else { return };
        let (_, cursor) = self.composer.written(cx);
        let token = inline_token(cac_core::mention_token(person), "user");
        let range = menu.start as usize..cursor;
        self.composer.field.update(cx, |field, cx| put_token(field, Some(range), token, window, cx));
        self.composer.mentions.reset();
        cx.notify();
    }

    fn move_mention(&mut self, by: i32, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.mention_menu(cx) else { return false };
        self.composer.mentions.moved(&menu, by);
        cx.notify();
        true
    }

    fn pick_highlighted_mention(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.mention_menu(cx) else { return false };
        let Some(person) = cac_core::mention_highlighted(&menu).cloned() else { return false };
        self.pick_mention(&person, window, cx);
        true
    }

    fn close_mentions(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.mention_menu(cx) else { return false };
        self.composer.mentions.closed(&menu);
        cx.notify();
        true
    }

    /// Sube lo que llegó (pegado, soltado o elegido con el clip) a la
    /// conversación de la caja. Cada fichero, por su lado: uno que falla no
    /// para a los demás, y lo dice.
    pub(crate) fn take_files(&mut self, files: Vec<Incoming>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.composer.of.clone() else { return };
        self.composer.error = None;
        for file in files {
            self.composer.uploads += 1;
            let serial = self.composer.uploads;
            let name = incoming_name(&file);
            self.composer.uploading.push(Upload { serial, conversation: conversation.clone(), name: name.clone() });
            let to = conversation.clone();
            let answer = Backend::ask(cx, move |core| async move {
                let (name, read) = read_incoming(file);
                match read {
                    Ok(bytes) => core.chat().attach(to, name, bytes).await.map_err(Some),
                    Err(error) => Err(error),
                }
            });
            let conversation = conversation.clone();
            cx.spawn_in(window, async move |this, cx| {
                let Ok(result) = answer.await else { return };
                this.update(cx, |this, cx| {
                    this.composer.uploading.retain(|upload| upload.serial != serial);
                    match result {
                        Ok(file) => {
                            if let Some(token) = cac_core::file_token(&file) {
                                this.composer.landed.push((conversation, inline_token(token, "file")));
                            }
                        }
                        Err(error) if this.composer.of.as_ref() == Some(&conversation) => {
                            let title = cac_core::text_with(locale(cx), "common:editor.errAttach", &[("name", &name)]);
                            let detail = match &error {
                                Some(error) => error_text(cx, error, "common:editor.errAttachBody"),
                                None => tr(cx, "common:editor.errAttachBody"),
                            };
                            this.composer.error = Some(format!("{title}: {detail}"));
                        }
                        Err(_) => {}
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// El clip: elegir ficheros del disco.
    fn choose_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            this.update_in(cx, |this, window, cx| this.take_files(paths.into_iter().map(Incoming::Path).collect(), window, cx)).ok();
        })
        .detach();
    }

    /// Responde a ese mensaje: la barra sale encima de la caja y el foco va
    /// a ella.
    pub(crate) fn reply_to(&mut self, line: &ThreadLine, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self.messages.iter().find(|message| message.id == line.id) else { return };
        self.composer.reply = Some(ReplyTo { id: line.id.clone(), author: line.author.clone(), text: cac_core::quote_text(&message.body) });
        self.composer.field.update(cx, |field, cx| field.focus(window, cx));
        cx.notify();
    }

    fn cancel_reply(&mut self, cx: &mut Context<Self>) -> bool {
        if self.composer.reply.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    /// Manda lo escrito. Si sale bien se vacía la caja —si lo escrito sigue
    /// siendo lo que se mandó: lo que se añadió mientras viajaba se queda—,
    /// se quita la respuesta y se baja al final. Si falla, lo escrito se
    /// queda y se dice por qué junto a la caja.
    pub(crate) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.composer.of.clone() else { return };
        // Con un fichero a medio subir no se manda: su cita llegaría tarde.
        if self.composer.uploading.iter().any(|upload| upload.conversation == conversation) {
            return;
        }
        let written = self.composer.field.read(cx).value().to_string();
        if cac_core::message_to_send(&written).is_err() {
            return;
        }
        let reply = self.composer.reply.as_ref().map(|reply| reply.id.clone());
        self.composer.error = None;
        let sent_to = conversation.clone();
        let sent = written.clone();
        let answer = Backend::ask(cx, move |core| async move { core.chat().send(sent_to, sent, reply).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                let here = this.composer.of.as_ref() == Some(&conversation);
                match result {
                    Ok(()) if here => {
                        if this.composer.field.read(cx).value().as_ref() == written {
                            this.composer.field.update(cx, |field, cx| field.set_value("", window, cx));
                        }
                        this.composer.reply = None;
                        this.to_the_end(cx);
                    }
                    // Se mandó desde otra conversación: su borrador ya salió.
                    Ok(()) => this.composer.drafts.retain(|draft| draft.conversation != conversation || draft.content.text().as_ref() != written),
                    // Otro Enter con el mensaje de camino: no es un fallo.
                    Err(ApiError::Server { ref code, .. }) if code == "already-sending" => {}
                    Err(error) if here => this.composer.error = Some(error_text(cx, &error, "common:last.couldNotSend")),
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Enter manda, salvo con la lista de menciones abierta (elige) o dentro
    /// de un bloque de código (salta de línea). Mayús+Enter salta siempre.
    fn pressed_enter(&mut self, enter: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if enter.shift || enter.secondary {
            return;
        }
        if self.pick_highlighted_mention(window, cx) {
            cx.stop_propagation();
            return;
        }
        let (text, cursor) = self.composer.written(cx);
        if cac_core::enter_sends(&text, cursor as u32) {
            cx.stop_propagation();
            self.send(window, cx);
        }
    }

    /// La caja, con lo que va encima (a qué se responde, qué falló, a quién
    /// nombrar) y el botón de mandar.
    pub(crate) fn render_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (conversation, can_write, sending, direct_name, me) = {
            let snapshot = Backend::snapshot(cx).read(cx);
            let thread = &snapshot.thread;
            let direct_name = match &thread.conversation {
                Some(Conversation::Direct { conversation_id }) => {
                    snapshot.comms.directs.iter().find(|direct| &direct.conversation_id == conversation_id).map(|direct| direct.username.clone())
                }
                _ => None,
            };
            (thread.conversation.clone(), thread.can_write, thread.sending, direct_name, snapshot.session.as_ref().map(|session| session.id.clone()))
        };
        self.composer.follow(conversation.clone(), me, window, cx);
        // Lo que subió para la conversación de la caja se pone como ficha.
        if let Some(of) = self.composer.of.clone() {
            let landed = std::mem::take(&mut self.composer.landed);
            let (here, elsewhere): (Vec<_>, Vec<_>) = landed.into_iter().partition(|(conversation, _)| conversation == &of);
            self.composer.landed = elsewhere;
            for (_, token) in here {
                self.composer.field.update(cx, |field, cx| put_token(field, None, token, window, cx));
            }
        }
        let placeholder = match direct_name {
            Some(name) => cac_core::text_with(locale(cx), "common:misc.messageTo", &[("name", &name)]),
            None => tr(cx, "chat:composerPlaceholder"),
        };
        let theme = cx.theme();
        let (muted_ink, border, primary, danger, background, quote_bg) = (theme.muted_foreground, theme.border, theme.primary, theme.danger, theme.background, theme.muted.opacity(0.4));

        // Quien sólo mira la organización no escribe en un canal: la caja
        // está, apagada, y dice por qué.
        if !can_write {
            let why = tr(cx, "errors:viewer-read-only");
            return div()
                .id("thread-composer")
                .flex_none()
                .border_t_1()
                .border_color(border)
                .p_2()
                .aria_label(why.clone())
                .child(
                    div()
                        .min_h(px(40.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(border)
                        .bg(crate::theme::field_background(cx))
                        .px_3()
                        .py_2()
                        .leading_sm()
                        .text_color(muted_ink)
                        .child(why),
                )
                .test_support()
                .into_any_element();
        }

        let field = self.composer.field.clone();
        let view = cx.entity().downgrade();
        if self.composer.placeholder != placeholder {
            self.composer.placeholder = placeholder.clone();
            field.update(cx, |field, cx| field.set_placeholder(placeholder.clone(), window, cx));
        }
        let uploading: Vec<(u64, String)> =
            self.composer.uploading.iter().filter(|upload| conversation.as_ref() == Some(&upload.conversation)).map(|upload| (upload.serial, upload.name.clone())).collect();
        let can_send = !sending && uploading.is_empty() && cac_core::message_to_send(&field.read(cx).value()).is_ok();
        let menu = self.mention_menu(cx);
        let menu_open = menu.is_some();

        let replying = self.composer.reply.clone().map(|reply| {
            let said = cac_core::text_with(locale(cx), "chat:replyingTo", &[("name", &reply.author)]);
            div()
                .id("thread-replying")
                .mb_1()
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(6.))
                .border_l_2()
                .border_color(primary)
                .bg(quote_bg)
                .px_2()
                .py_1()
                .leading_xs()
                .aria_label(format!("{said}: {}", reply.text))
                .child(Icon::new(Lucide::CornerUpLeft).size_3().text_color(muted_ink))
                .child(div().flex_none().font_weight(FontWeight::MEDIUM).child(said))
                .child(div().flex_1().min_w_0().truncate().text_color(muted_ink).child(reply.text.clone()))
                .child(
                    Button::new("thread-reply-cancel")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::Close))
                        .tooltip(tr(cx, "chat:cancelReply"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_reply(cx);
                        })),
                )
                .test_support()
        });
        let uploading = uploading.into_iter().map(|(serial, name)| {
            let said = cac_core::text_with(locale(cx), "chat:uploading", &[("name", &name)]);
            div()
                .id(SharedString::from(format!("thread-uploading-{serial}")))
                .mb_1()
                .flex()
                .items_center()
                .gap_2()
                .leading_xs()
                .text_color(muted_ink)
                .aria_label(said.clone())
                .child(Spinner::new().xsmall().color(muted_ink))
                .child(said)
                .test_support()
        }).collect::<Vec<_>>();
        let failed = self.composer.error.clone().map(|said| {
            div().id("thread-send-failed").mb_1().leading_xs().text_color(danger).aria_label(said.clone()).child(said).test_support()
        });
        let mentions = menu.map(|menu| {
            let list = render_mentions(menu, &self.composer.mentions, Rc::new(|this: &mut Self, person, window, cx| this.pick_mention(person, window, cx)), cx);
            // Encima de la caja, que está al pie de la ventana.
            div().absolute().bottom_full().left_2().w(px(320.)).mb_1().bg(background).shadow_lg().child(list)
        });

        div()
            .id("thread-composer")
            .aria_label(placeholder)
            .relative()
            .flex_none()
            .border_t_1()
            .border_color(border)
            .p_2()
            .flex()
            .flex_col()
            // Un fichero soltado en cualquier parte de la caja se adjunta.
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(primary))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.take_files(paths.paths().iter().cloned().map(Incoming::Path).collect(), window, cx)))
            .children(replying)
            .children(uploading)
            .children(failed)
            // En una línea, como WhatsApp: el clip a la izquierda, la caja en
            // medio y Enviar a la derecha. Al crecer la caja, los botones se
            // quedan abajo, junto a la última línea.
            .child(
                div().flex().items_end().gap_2().child(
                    Button::new("thread-attach")
                        .ghost()
                        .icon(Icon::new(Lucide::Paperclip))
                        .tooltip(tr(cx, "common:editor.attachFile"))
                        .on_click(cx.listener(|this, _, window, cx| this.choose_files(window, cx))),
                ).child(
                div()
                    .id("thread-field")
                    .flex_1()
                    .min_w_0()
                    // Enter y Escape se cogen antes de que lleguen a la caja.
                    // Con la lista abierta, las flechas y Tab son suyas.
                    .capture_action(cx.listener(|this, enter: &Enter, window, cx| this.pressed_enter(enter, window, cx)))
                    .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                        if this.close_mentions(cx) || this.cancel_reply(cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .when(menu_open, |this| {
                        this.capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                            if this.move_mention(-1, cx) {
                                cx.stop_propagation();
                            }
                        }))
                        .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                            if this.move_mention(1, cx) {
                                cx.stop_propagation();
                            }
                        }))
                        .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                            if this.pick_highlighted_mention(window, cx) {
                                cx.stop_propagation();
                            }
                        }))
                    })
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .bg(crate::theme::field_background(cx))
                    .px_1()
                    .child(
                        Textarea::new(&field)
                            .appearance(false)
                            // Una imagen pegada o un fichero copiado se
                            // adjuntan; el texto pegado sigue siendo texto.
                            .on_paste(move |item, window, cx| {
                                let files = pasted(item);
                                if files.is_empty() {
                                    return false;
                                }
                                view.update(cx, |this, cx| this.take_files(files, window, cx)).is_ok()
                            })
                            .token(move |token, _, _| {
                                let file = token.token().id().starts_with("file-");
                                div()
                                    .id(SharedString::from(format!("composer-token-{}", token.token().id())))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .rounded(px(4.))
                                    .px_0p5()
                                    .bg(primary.opacity(0.12))
                                    .text_color(primary)
                                    .font_weight(FontWeight::MEDIUM)
                                    .aria_label(token.token().label().clone())
                                    .when(file, |this| this.child(Icon::new(Lucide::Paperclip).size_3()))
                                    .child(token.token().label().clone())
                                    .test_support()
                            }),
                    )
                    .test_support(),
                ).child(
                    Button::new("thread-send")
                        .primary()
                        .map(|this| if sending { this.child(Spinner::new().xsmall()) } else { this.icon(Icon::new(Lucide::Send)) })
                        .label(tr(cx, "chat:send"))
                        .disabled(!can_send)
                        .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                ),
            )
            .children(mentions)
            .test_support()
            .into_any_element()
    }
}
