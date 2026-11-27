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
//!
//! Pulsar el aviso de una conversación no se va de la página: la tarjeta
//! pasa a ser su **vista rápida**, con los últimos mensajes y una caja para
//! contestar ahí (`cac_core::inbox_peek`). Y con la campana abierta, las
//! filas de un directo que se ven llevan el inicio de su mensaje, pedido a
//! la conversación; al cerrarla se olvida.

use std::collections::{HashMap, HashSet};

use cac_core::{ApiError, Conversation, InboxGroup, InboxIcon, InboxRow, InboxTab, InboxTarget, InboxTone, PreviewWanted, ThreadMessage, INBOX_TABS};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter, InputContent, Textarea, TextareaState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{error_text, locale, tr};
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
    /// Los últimos mensajes de cada directo cuyas filas se ven, para su
    /// inicio. Sólo con la campana abierta: al cerrarla se vacía.
    previews: HashMap<Conversation, Vec<ThreadMessage>>,
    /// Lo ya pedido (o en camino), por conversación y aviso más nuevo.
    asked: HashSet<PreviewWanted>,
    /// Cuántas veces se ha abierto: lo que llega de una apertura anterior no
    /// se pone.
    opened: u64,
    /// Cuántas vistas rápidas se han abierto, para numerarlas.
    quick_serials: u64,
    /// La vista rápida de una conversación, si se pulsó una.
    pub(crate) quick: Option<QuickView>,
}

/// La vista rápida: los últimos mensajes de una conversación y una caja
/// para contestar, encima de la página.
pub(crate) struct QuickView {
    conversation: Conversation,
    title: String,
    link: String,
    org_id: String,
    /// `None` mientras llegan.
    messages: Option<Vec<ThreadMessage>>,
    failed: bool,
    pub(crate) field: Entity<TextareaState>,
    sending: bool,
    error: Option<String>,
    /// Cuál de las vistas rápidas es: lo que llega para otra no se pone.
    serial: u64,
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
    pub(crate) fn toggle_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.inbox.open {
            self.close_inbox(window, cx);
            return;
        }
        self.inbox.open = true;
        self.inbox.opened += 1;
        self.inbox.said = None;
        Backend::run(cx, |core| async move { core.inbox().load().await });
        cx.notify();
    }

    /// «Marcar todo leído». Si el servidor no lo acepta, lo dice: el número
    /// ya bajó aquí, y sin aviso parecería hecho.
    fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        self.inbox.said = None;
        let answer = Backend::ask(cx, |core| async move { core.inbox().mark_all_read().await });
        cx.spawn(async move |this, cx| {
            let Ok(Err(error)) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.inbox.said = Some(error_text(cx, &error, "notifications:markAllFailed"));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Lo cierra; Escape también (lo oye `workspace.rs`, que tiene el foco).
    /// Lo que se leyó de las conversaciones se olvida: con la campana
    /// cerrada no queda texto de nadie a mano.
    pub(crate) fn close_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_quick_view(window, cx);
        self.inbox.open = false;
        self.inbox.previews.clear();
        self.inbox.asked.clear();
        cx.notify();
    }

    /// Pulsar un aviso o una conversación: leída, y a su sitio. Una
    /// conversación se abre en la vista rápida, sin irse de la página. Lo que
    /// esta bandeja no sabe abrir (la organización, un aviso de Basecamp, un
    /// sitio que esta app no tiene) va por `links::open`: a su pantalla, o a
    /// la web en el navegador; el panel se queda.
    fn open_from_inbox(&mut self, read: InboxRead, link: String, org_id: String, title: String, window: &mut Window, cx: &mut Context<Self>) {
        let target = cac_core::link_target(&link);
        let here = target != InboxTarget::NotHere;
        let quick = matches!(target, InboxTarget::Conversation { .. });
        let (go_link, go_org) = (link.clone(), org_id.clone());
        Backend::run(cx, move |core| async move {
            let _ = match read {
                InboxRead::One(id) => core.inbox().mark_read(vec![id]).await,
                InboxRead::Group(key) => core.inbox().mark_group_read(key).await,
            };
            if here && !quick {
                core.inbox().open(&go_link, &go_org).await;
            }
        });
        if let InboxTarget::Conversation { conversation } = target {
            self.open_quick_view(conversation, title, link, org_id, window, cx);
        } else if here {
            self.close_inbox(window, cx);
        } else {
            crate::links::open(&link, cx);
        }
    }

    /// Abre la vista rápida de esa conversación: pide sus últimos mensajes y
    /// trae a la caja lo que hubiera escrito para ella.
    fn open_quick_view(&mut self, conversation: Conversation, title: String, link: String, org_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close_quick_view(window, cx);
        self.inbox.said = None;
        let draft = self.chat.update(cx, |chat, cx| chat.take_draft(&conversation, window, cx));
        let placeholder = tr(cx, "notifications:quick.reply");
        let field = cx.new(|cx| {
            let mut field = TextareaState::new(window, cx).auto_grow(1, 5);
            field.set_placeholder(placeholder, window, cx);
            field.set_value(draft, window, cx);
            let end = field.value().len();
            field.set_selected_range(end..end, cx);
            field
        });
        field.update(cx, |field, cx| field.focus(window, cx));
        self.inbox.quick_serials += 1;
        let serial = self.inbox.quick_serials;
        self.inbox.quick = Some(QuickView { conversation: conversation.clone(), title, link, org_id, messages: None, failed: false, field, sending: false, error: None, serial });
        self.load_quick_view(conversation, serial, cx);
        cx.notify();
    }

    fn load_quick_view(&mut self, conversation: Conversation, serial: u64, cx: &mut Context<Self>) {
        if let Some(quick) = self.inbox.quick.as_mut() {
            quick.failed = false;
        }
        let answer = Backend::ask(cx, move |core| async move { core.inbox().quick_view(&conversation).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                let Some(quick) = this.inbox.quick.as_mut().filter(|quick| quick.serial == serial) else { return };
                match result {
                    Ok(messages) => quick.messages = Some(messages),
                    Err(_) => quick.failed = quick.messages.is_none(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Cierra la vista rápida y devuelve lo escrito a su conversación.
    fn close_quick_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(quick) = self.inbox.quick.take() else { return };
        let content: InputContent = quick.field.read(cx).content();
        let conversation = quick.conversation;
        self.chat.update(cx, |chat, cx| chat.give_draft(conversation, content, window, cx));
        cx.notify();
    }

    /// «Abrir la conversación»: a ella, como antes de la vista rápida, con lo
    /// escrito esperándola en su caja.
    fn open_quick_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((link, org_id)) = self.inbox.quick.as_ref().map(|quick| (quick.link.clone(), quick.org_id.clone())) else { return };
        Backend::run(cx, move |core| async move {
            core.inbox().open(&link, &org_id).await;
        });
        self.close_inbox(window, cx);
    }

    /// Manda lo escrito en la vista rápida a su conversación, por el mismo
    /// camino que la caja de la conversación (`Chat::send`). Si sale bien se
    /// vacía la caja —si sigue diciendo lo mismo— y se releen los últimos.
    fn send_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(quick) = self.inbox.quick.as_mut() else { return };
        if quick.sending {
            return;
        }
        let written = quick.field.read(cx).value().to_string();
        if cac_core::message_to_send(&written).is_err() {
            return;
        }
        quick.sending = true;
        quick.error = None;
        let (to, serial, sent) = (quick.conversation.clone(), quick.serial, written.clone());
        let answer = Backend::ask(cx, move |core| async move {
            let result = core.chat().send(to.clone(), sent, None).await;
            let fresh = match result {
                Ok(()) => core.inbox().peek(&to).await.ok(),
                Err(_) => None,
            };
            (result, fresh)
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok((result, fresh)) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                let Some(quick) = this.inbox.quick.as_mut().filter(|quick| quick.serial == serial) else { return };
                quick.sending = false;
                match result {
                    Ok(()) => {
                        if quick.field.read(cx).value().as_ref() == written {
                            quick.field.update(cx, |field, cx| field.set_value("", window, cx));
                        }
                        if let Some(fresh) = fresh {
                            quick.messages = Some(fresh);
                        }
                    }
                    Err(ApiError::Server { ref code, .. }) if code == "already-sending" => {}
                    Err(error) => quick.error = Some(error_text(cx, &error, "common:last.couldNotSend")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Pide el inicio de los directos que el panel enseña y todavía no se
    /// tienen. Lo que llega tarde, de una campana ya cerrada, no se pone.
    fn ask_previews(&mut self, wanted: Vec<PreviewWanted>, cx: &mut Context<Self>) {
        for want in wanted {
            if !self.inbox.asked.insert(want.clone()) {
                continue;
            }
            let opened = self.inbox.opened;
            let conversation = want.conversation.clone();
            let answer = Backend::ask(cx, move |core| async move { core.inbox().peek(&conversation).await });
            cx.spawn(async move |this, cx| {
                let Ok(Ok(messages)) = answer.await else { return };
                this.update(cx, |this, cx| {
                    if this.inbox.open && this.inbox.opened == opened {
                        this.inbox.previews.insert(want.conversation, messages);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    pub(crate) fn render_inbox(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.inbox.open {
            return None;
        }
        let language = locale(cx);
        let (panel, me) = {
            let snapshot = Backend::snapshot(cx).read(cx);
            (cac_core::inbox_panel(&snapshot.inbox, self.inbox.tab, &self.inbox.expanded, language, &chrono::Utc::now()), snapshot.session.as_ref().map(|session| session.id.clone()))
        };
        let hide = Backend::core(cx).hide_direct_previews();
        self.ask_previews(cac_core::previews_wanted(&panel, hide), cx);
        let panel = cac_core::with_previews(panel, &self.inbox.previews, me.as_deref(), hide);
        if let Some(quick) = self.render_quick_view(me.as_deref(), cx) {
            return Some(self.render_inbox_frame(quick, cx));
        }
        let sound = Backend::core(cx).notify_sound();
        let theme = cx.theme();
        let (border, muted, primary, on_primary, accent, foreground) =
            (theme.border, theme.muted_foreground, theme.primary, theme.primary_foreground, theme.accent, theme.foreground);

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
                                .on_click(cx.listener(|this, _, _, cx| this.mark_all_read(cx)))
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
                    })
                    // Para compartir pantalla: las filas de un directo sin lo
                    // que dice. De esta máquina, como el sonido.
                    .child({
                        let hint = tr(cx, "notifications:prefs.hidePreviewsHint");
                        div()
                            .id("inbox-hide-previews")
                            .cursor_pointer()
                            .hover(move |style| style.text_color(foreground))
                            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                            .aria_label(tr(cx, "notifications:prefs.hidePreviews"))
                            .aria_selected(hide)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = Backend::core(cx).set_hide_direct_previews(!hide);
                                this.inbox.previews.clear();
                                this.inbox.asked.clear();
                                cx.notify();
                            }))
                            .child(Icon::new(if hide { Lucide::EyeOff } else { Lucide::Eye }).size_3p5())
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
        if panel.loading {
            list = list.child(div().id("inbox-loading").flex().flex_col().gap_1p5().px_1().py_1p5().children((0..4).map(|_| div().h(px(44.)).w_full().rounded_md().bg(muted.opacity(0.15)))).test_support());
        }
        if panel.failed {
            let said = tr(cx, "notifications:loadFailed");
            list = list.child(
                div()
                    .id("inbox-error")
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .px_2p5()
                    .py_5()
                    .leading_xs()
                    .text_color(muted)
                    .aria_label(said.clone())
                    .child(said)
                    .child(Button::new("inbox-retry").outline().small().label(tr(cx, "chat:retry")).on_click(|_, _, cx| Backend::run(cx, |core| async move { core.inbox().load().await })))
                    .test_support(),
            );
        }
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

        let content = div().flex().flex_col().min_h_0().child(header).child(tabs).child(list).children(said).child(footer).into_any_element();
        Some(self.render_inbox_frame(content, cx))
    }

    /// El fondo y la tarjeta colgada de la campana, con lo que toque dentro:
    /// la bandeja o una vista rápida.
    fn render_inbox_frame(&self, content: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, card) = (theme.border, theme.popover);
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
                        .on_click(cx.listener(|this, _, window, cx| this.close_inbox(window, cx)))
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
                                .child(content),
                        )
                        .test_support(),
                )
                .test_support()
                .into_any_element()
    }

    /// La vista rápida, si hay una abierta: quién, los últimos mensajes, la
    /// caja para contestar y «Abrir la conversación».
    fn render_quick_view(&self, me: Option<&str>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let quick = self.inbox.quick.as_ref()?;
        let language = locale(cx);
        let theme = cx.theme();
        let (border, muted, foreground, danger, primary) = (theme.border, theme.muted_foreground, theme.foreground, theme.danger, theme.primary);
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_2p5()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(
                Button::new("inbox-quick-back")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Lucide::ArrowLeft))
                    .tooltip(tr(cx, "notifications:quick.back"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_quick_view(window, cx))),
            )
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(quick.title.clone()))
            .child(
                Button::new("inbox-quick-open")
                    .ghost()
                    .small()
                    .label(tr(cx, "notifications:quick.open"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_quick_conversation(window, cx))),
            );

        let mut list = div().id("inbox-quick-lines").flex_1().min_h_0().overflow_y_scroll().px_3().py_2().flex().flex_col().gap_2();
        match &quick.messages {
            Some(messages) if messages.is_empty() => {
                let said = tr(cx, "notifications:quick.empty");
                list = list.child(div().id("inbox-quick-empty").py_3().text_center().text_color(muted).aria_label(said.clone()).child(said).test_support());
            }
            Some(messages) => {
                for line in cac_core::quick_lines(&quick.conversation, messages, me, language, &chrono::Local::now()) {
                    let label = format!("{}: {}", line.author, line.text);
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("inbox-quick-line-{}", line.id)))
                            .flex()
                            .flex_col()
                            .when(line.mine, |this| this.items_end())
                            .aria_label(label)
                            .child(
                                div()
                                    .flex()
                                    .gap_1p5()
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child(div().font_weight(FontWeight::MEDIUM).text_color(if line.mine { primary } else { foreground }).child(line.author.clone()))
                                    .child(line.time.clone()),
                            )
                            .child(div().leading_xs().child(line.text.clone()))
                            .test_support(),
                    );
                }
            }
            None if quick.failed => {
                let said = tr(cx, "notifications:quick.failed");
                let (conversation, serial) = (quick.conversation.clone(), quick.serial);
                list = list.child(
                    div()
                        .id("inbox-quick-failed")
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .py_3()
                        .text_color(muted)
                        .aria_label(said.clone())
                        .child(said)
                        .child(Button::new("inbox-quick-retry").outline().small().label(tr(cx, "chat:retry")).on_click(cx.listener(move |this, _, _, cx| this.load_quick_view(conversation.clone(), serial, cx))))
                        .test_support(),
                );
            }
            None => list = list.child(div().id("inbox-quick-loading").py_3().flex().justify_center().child(Spinner::new().small().color(muted)).test_support()),
        }

        let error = quick.error.clone().map(|said| div().id("inbox-quick-error").px_3().pb_1().leading_xs().text_color(danger).aria_label(said.clone()).child(said).test_support());
        let sending = quick.sending;
        let composer = div()
            .flex()
            .items_end()
            .gap_2()
            .border_t_1()
            .border_color(border)
            .p_2()
            .child(
                div()
                    .id("inbox-quick-field")
                    .flex_1()
                    .min_w_0()
                    // Enter manda, como en la conversación; Mayús+Enter salta
                    // de línea.
                    .capture_action(cx.listener(|this, enter: &Enter, window, cx| {
                        if enter.shift || enter.secondary {
                            return;
                        }
                        let Some(quick) = this.inbox.quick.as_ref() else { return };
                        let (text, cursor) = {
                            let field = quick.field.read(cx);
                            (field.value().to_string(), field.cursor())
                        };
                        if cac_core::enter_sends(&text, cursor as u32) {
                            cx.stop_propagation();
                            this.send_quick(window, cx);
                        }
                    }))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .bg(crate::theme::field_background(cx))
                    .px_1()
                    .child(Textarea::new(&quick.field).appearance(false))
                    .test_support(),
            )
            .child(
                Button::new("inbox-quick-send")
                    .primary()
                    .map(|this| if sending { this.child(Spinner::new().xsmall()) } else { this.icon(Icon::new(Lucide::Send)) })
                    .tooltip(tr(cx, "chat:send"))
                    .on_click(cx.listener(|this, _, window, cx| this.send_quick(window, cx))),
            );
        Some(
            div()
                .id("inbox-quick")
                .flex()
                .flex_col()
                .min_h_0()
                .max_h(px(560.))
                .aria_label(quick.title.clone())
                .child(header)
                .child(list)
                .children(error)
                .child(composer)
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
            let title = group.title.clone();
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
                .on_click(cx.listener(move |this, _, window, cx| this.open_from_inbox(InboxRead::Group(key.clone()), link.clone(), org.clone(), title.clone(), window, cx)))
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
        let (id, link, org, title) = (row.id.clone(), row.link.clone(), row.org_id.clone(), row.title.clone());
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
            .on_click(cx.listener(move |this, _, window, cx| this.open_from_inbox(InboxRead::One(id.clone()), link.clone(), org.clone(), title.clone(), window, cx)))
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
                    .when(!row.body.is_empty(), |this| {
                        this.child(
                            div()
                                .id(SharedString::from(format!("inbox-row-body-{}", row.id)))
                                .mt_0p5()
                                .leading_xs()
                                .text_color(muted)
                                .aria_label(row.body.clone())
                                .child(row.body.clone())
                                .test_support(),
                        )
                    }),
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
        server.mount(get("/api/v1/task-spaces/a/chat").respond_with(ok(json!([
            { "id": "k1", "spaceId": "a", "authorUserId": "u2", "authorName": "Bea", "kind": "user", "body": "hola canal", "createdAt": "2026-11-18T11:00:00Z", "updatedAt": "2026-11-18T11:00:00Z" },
        ]))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/notifications/read")).respond_with(ok(json!(null))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/notifications/read-all")).respond_with(ok(json!(null))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/task-spaces/a/chat/read")).respond_with(ok(json!(null))));
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
        open_with_workspace(cx).0
    }

    fn open_with_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, gpui_kit::Entity<Workspace>) {
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(760.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window")
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

    /// Pulsar una conversación la marca leída entera y abre su vista rápida
    /// sin irse de la página; «Abrir la conversación» lleva a ella. Pulsar
    /// algo que esta app no tiene lo marca leído, se queda en el panel y lo
    /// abre en la web.
    ///
    /// Roturas que atrapa: marcar una conversación por los ids que hay a mano;
    /// irse de la página al pulsarla; un «Abrir la conversación» que no lleva
    /// a ella; cerrar el panel y no ir a ningún sitio (un clic perdido); no
    /// marcar leído lo que no se puede abrir; un sitio que esta app no tiene
    /// que no lleva a ninguna parte (ni a la web).
    #[gpui_kit::test]
    fn clicking_goes_where_it_says_or_says_why_not(cx: &mut TestAppContext) {
        let server = server(full_inbox());
        server.install(cx);
        let window = open(cx);
        open_panel(window, cx);

        click(window, "inbox-row-n3", cx);
        let web = cx.update(|cx| format!("{}/app/servers/x/backups?target=b", Backend::core(cx).base_url()));
        wait_until(cx, "lo abre en la web", |cx| cx.opened_url().as_ref() == Some(&web));
        assert!(on_screen(window, "inbox", cx), "y el panel sigue abierto");
        wait_until(cx, "se marca leído", |_| posted(&server, "/api/v1/notifications/read").iter().any(|(_, body)| *body == json!({ "ids": ["n3"] })));

        click(window, "inbox-group-chat:a", cx);
        wait_until(cx, "la vista rápida enseña el canal", |cx| label(window, "inbox-quick-line-k1", cx) == "Bea: hola canal");
        assert!(cx.update(|cx| !Backend::snapshot(cx).read(cx).comms.on_screen), "la página de debajo no cambia");
        click(window, "inbox-quick-open", cx);
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

    /// Mientras no llega la bandeja no dice «Nada pendiente»: si la lectura
    /// falla, dice que no pudo, con «Reintentar», y se recupera.
    ///
    /// Roturas que atrapa: «Nada pendiente» con la bandeja sin cargar; un
    /// fallo mudo; un «Reintentar» que no vuelve a pedir.
    #[gpui_kit::test]
    fn a_failed_read_is_not_an_empty_inbox(cx: &mut TestAppContext) {
        // El fallo se monta antes que la bandeja: con la misma prioridad gana el primero, y se agota tras una vez.
        let server = server_with_tasks(vec![]);
        let refusal = wiremock::ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": "boom", "message": "x" }));
        server.mount(get("/api/v1/notifications/").and(query_param("limit", "50")).respond_with(refusal).up_to_n_times(1).with_priority(1));
        let feed = json!({ "items": full_inbox(), "unread": 41, "groups": [{ "key": "chat:a", "label": "#Espacio a", "total": 40, "unread": 40 }] });
        server.mount(get("/api/v1/notifications/").and(query_param("limit", "50")).respond_with(ok(feed)).with_priority(1));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([]))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "notifications-bell", cx);
        wait_until(cx, "dice que no pudo", |cx| on_screen(window, "inbox-error", cx));
        assert!(!on_screen(window, "inbox-empty", cx) && !on_screen(window, "inbox-loading", cx));

        click(window, "inbox-retry", cx);
        wait_until(cx, "llega la bandeja", |cx| on_screen(window, "inbox-group-chat:a", cx) && !on_screen(window, "inbox-error", cx));
        stop(cx);
    }

    /// Si «Marcar todo leído» no lo acepta el servidor, el panel lo dice.
    /// Rotura que atrapa: un fallo mudo.
    #[gpui_kit::test]
    fn mark_all_says_why_it_failed(cx: &mut TestAppContext) {
        let server = server(full_inbox());
        let refusal = wiremock::ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": "boom", "message": "Sin servicio" }));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/notifications/read-all")).respond_with(refusal).with_priority(1));
        server.install(cx);
        let window = open(cx);
        open_panel(window, cx);
        click(window, "inbox-mark-all", cx);
        wait_until(cx, "lo dice", |cx| !label(window, "inbox-said", cx).is_empty());
        stop(cx);
    }
    /// Una bandeja con dos directos de Diego (c1, plegados) y uno de Bea
    /// (c2, suelto), sin texto, como los manda el servidor; y las dos
    /// conversaciones con sus últimos mensajes.
    fn server_with_directs() -> FakeServer {
        let dm = |id: &str, conversation: &str, at: &str, who: &str| {
            json!({ "id": id, "orgId": "o1", "kind": "dm:message", "title": format!("{who} te escribió"), "body": "", "link": format!("/dm?c={conversation}"),
                    "createdAt": at, "groupKey": format!("dm:{conversation}"), "groupLabel": who })
        };
        let items = json!([
            dm("d1", "c1", "2026-11-18T11:00:01Z", "Diego"),
            dm("d2", "c1", "2026-11-18T11:30:01Z", "Diego"),
            dm("b1", "c2", "2026-11-18T10:00:01Z", "Bea"),
        ]);
        let server = server_with_tasks(vec![]);
        let feed = json!({ "items": items, "unread": 3, "groups": [] });
        server.mount(get("/api/v1/notifications/").and(query_param("limit", "50")).respond_with(ok(feed)).with_priority(1));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([]))));
        let message = |id: &str, conversation: &str, author: &str, name: &str, at: &str, body: &str| {
            json!({ "id": id, "conversationId": conversation, "authorUserId": author, "authorName": name, "body": body, "createdAt": at, "updatedAt": at })
        };
        server.mount(get("/api/v1/dm/c1/messages").respond_with(ok(json!([
            message("m1", "c1", "u3", "Diego", "2026-11-18T11:00:00Z", "¿Has visto el **deploy**?"),
            message("m2", "c1", "u3", "Diego", "2026-11-18T11:30:00Z", "Ya está, avísame"),
        ]))));
        server.mount(get("/api/v1/dm/c2/messages").respond_with(ok(json!([message("x1", "c2", "u2", "Bea", "2026-11-18T10:00:00Z", "Te paso el acta")]))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/notifications/read")).respond_with(ok(json!(null))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/dm/c1/read")).respond_with(ok(json!(null))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/dm/c1/messages")).respond_with(ok(json!(null))));
        server
    }

    /// Cuántas veces se leyeron los últimos mensajes de esa conversación.
    fn peeks(server: &FakeServer, conversation: &str) -> usize {
        server.requests().iter().filter(|request| request.method.as_str() == "GET" && request.url.path() == format!("/api/v1/dm/{conversation}/messages")).count()
    }

    fn wait_open(window: AnyWindowHandle, cx: &mut TestAppContext) {
        click(window, "notifications-bell", cx);
        wait_until(cx, "el panel enseña la bandeja", |cx| on_screen(window, "inbox-group-dm:c1", cx));
    }

    /// Con la campana abierta, cada fila de un directo que se ve enseña el
    /// inicio de su mensaje, pedido a su conversación; un grupo plegado no
    /// pide nada; al cerrar la campana se olvida; y el ajuste de ocultar lo
    /// apaga.
    ///
    /// Roturas que atrapa: pedir texto con la campana cerrada; pedir el de un
    /// grupo plegado; no pintar el inicio en la fila; guardar lo leído al
    /// cerrar (al volver no se pediría de nuevo); un ajuste que no apaga el
    /// texto, o que lo apaga pero lo sigue pidiendo.
    #[gpui_kit::test]
    fn direct_rows_show_their_start_only_with_the_bell_open(cx: &mut TestAppContext) {
        let server = server_with_directs();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        assert_eq!((peeks(&server, "c1"), peeks(&server, "c2")), (0, 0), "con la campana cerrada no se pide nada");

        wait_open(window, cx);
        wait_until(cx, "la fila suelta de Bea dice lo suyo", |cx| label(window, "inbox-row-body-b1", cx) == "Te paso el acta");
        assert_eq!(peeks(&server, "c1"), 0, "Diego, plegado, no se pide");

        click(window, "inbox-toggle-dm:c1", cx);
        wait_until(cx, "desplegado, cada fila con su mensaje", |cx| {
            label(window, "inbox-row-body-d1", cx) == "¿Has visto el deploy?" && label(window, "inbox-row-body-d2", cx) == "Ya está, avísame"
        });

        click(window, "notifications-bell", cx);
        wait_until(cx, "se cierra", |cx| !on_screen(window, "inbox", cx));
        let before = peeks(&server, "c2");
        wait_open(window, cx);
        wait_until(cx, "al volver se pide otra vez", |cx| peeks(&server, "c2") > before && on_screen(window, "inbox-row-body-b1", cx));

        click(window, "inbox-hide-previews", cx);
        assert!(cx.update(|cx| Backend::core(cx).hide_direct_previews()), "el ajuste se guarda");
        wait_until(cx, "sin texto", |cx| !on_screen(window, "inbox-row-body-b1", cx) && !on_screen(window, "inbox-row-body-d1", cx));
        let asked = (peeks(&server, "c1"), peeks(&server, "c2"));
        click(window, "notifications-bell", cx);
        wait_open(window, cx);
        cx.run_until_parked();
        assert_eq!((peeks(&server, "c1"), peeks(&server, "c2")), asked, "y no se pide");
        assert!(!on_screen(window, "inbox-row-body-b1", cx));
        stop(cx);
    }

    /// Pulsar un directo abre su vista rápida sin cambiar de página: sus
    /// últimos mensajes, la conversación leída y una caja que contesta ahí,
    /// a esa conversación.
    ///
    /// Roturas que atrapa: irse de la página; no pedir los mensajes; no
    /// marcar leída la conversación vista; mandar a otra conversación (o a
    /// ninguna); dejar lo mandado en la caja.
    #[gpui_kit::test]
    fn a_direct_opens_its_quick_view_and_is_answered_there(cx: &mut TestAppContext) {
        let server = server_with_directs();
        server.install(cx);
        let (window, workspace) = open_with_workspace(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        wait_open(window, cx);

        click(window, "inbox-group-dm:c1", cx);
        wait_until(cx, "enseña los últimos", |cx| {
            label(window, "inbox-quick-line-m1", cx) == "Diego: ¿Has visto el deploy?" && label(window, "inbox-quick-line-m2", cx) == "Diego: Ya está, avísame"
        });
        assert_eq!(server.queries("/api/v1/dm/c1/messages").last().map(String::as_str), Some("limit=12"));
        wait_until(cx, "la conversación, leída", |_| !posted(&server, "/api/v1/dm/c1/read").is_empty());
        assert!(cx.update(|cx| !Backend::snapshot(cx).read(cx).comms.on_screen), "la página de debajo no cambia");

        cx.update_window(window, |_, window, cx| {
            let field = workspace.read(cx).inbox.quick.as_ref().unwrap().field.clone();
            field.update(cx, |field, cx| field.focus(window, cx));
            window.input("Voy", cx);
        })
        .unwrap();
        click(window, "inbox-quick-send", cx);
        wait_until(cx, "se manda a esa conversación", |_| posted(&server, "/api/v1/dm/c1/messages").iter().any(|(_, body)| *body == json!({ "body": "Voy" })));
        wait_until(cx, "y la caja se vacía", |cx| cx.update(|cx| workspace.read(cx).inbox.quick.as_ref().is_some_and(|quick| quick.field.read(cx).value().is_empty())));
        assert!(on_screen(window, "inbox-quick", cx), "sigue en la vista rápida");
        assert!(cx.update(|cx| !Backend::snapshot(cx).read(cx).comms.on_screen), "sin cambiar de página");
        stop(cx);
    }

    /// Lo que se escribe en la vista rápida y no se manda es el borrador de
    /// esa conversación: está en su caja al abrirla, y vuelve a la vista
    /// rápida. Rotura que atrapa: un segundo borrador que se pierde al
    /// cerrar.
    #[gpui_kit::test]
    fn the_quick_view_shares_the_conversation_draft(cx: &mut TestAppContext) {
        let server = server_with_directs();
        server.install(cx);
        let (window, workspace) = open_with_workspace(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        wait_open(window, cx);
        click(window, "inbox-group-dm:c1", cx);
        wait_until(cx, "se abre", |cx| on_screen(window, "inbox-quick-line-m1", cx));
        cx.update_window(window, |_, window, cx| {
            let field = workspace.read(cx).inbox.quick.as_ref().unwrap().field.clone();
            field.update(cx, |field, cx| field.focus(window, cx));
            window.input("a medias", cx);
        })
        .unwrap();
        click(window, "inbox-quick-back", cx);
        wait_until(cx, "vuelve a la bandeja", |cx| on_screen(window, "inbox-group-dm:c1", cx));
        click(window, "inbox-group-dm:c1", cx);
        wait_until(cx, "con lo escrito", |cx| cx.update(|cx| workspace.read(cx).inbox.quick.as_ref().is_some_and(|quick| quick.field.read(cx).value().as_ref() == "a medias")));
        stop(cx);
    }
}
