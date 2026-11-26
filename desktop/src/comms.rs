//! Comunicación: la lista de canales y directos en el panel, la cabecera de
//! la conversación que se tiene delante y «Nueva conversación».
//!
//! Qué filas hay, en qué orden, qué dice cada una, cuál está marcada y a
//! quién se le puede escribir lo decide el core (`cac_core::comms`). Aquí se
//! pinta con las medidas de la app de siempre (`CommsPanel.tsx`,
//! `PanelRow.tsx`, `PanelSection.tsx`, `NewConversation.tsx`) y se le pasa
//! cada gesto.
//!
//! La conversación en sí —sus mensajes— la pinta `chat.rs`; aquí está su
//! cabecera, que sale de la misma lista.

use cac_core::{CommsRow, CommsRows, CommsSection, Conversation, Person};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{error_text, locale, tr};
use crate::theme::Leading as _;

/// El ancho de «Nueva conversación»: el `sm:max-w-sm` de `dialog.tsx`.
const PICKER_WIDTH: f32 = 384.;
/// Hasta dónde crece la lista de gente antes de deslizar: `max-h-72`.
const PICKER_LIST_HEIGHT: f32 = 288.;
/// La cabecera de una conversación: el `h-12` de `ChannelView.tsx`.
const HEADER_HEIGHT: f32 = 48.;

/// El id de una fila de la lista: `comms-channel-<espacio>` o
/// `comms-direct-<conversación>`.
fn row_id(conversation: &Conversation) -> String {
    match conversation {
        Conversation::Channel { space_id } => format!("comms-channel-{space_id}"),
        Conversation::Direct { conversation_id } => format!("comms-direct-{conversation_id}"),
    }
}

/// El punto de «anda por aquí» (`PresenceDot.tsx`: `size-2 rounded-full`,
/// verde o apagado), en la caja de un icono de fila.
fn presence_dot(present: bool, cx: &App) -> Div {
    let theme = cx.theme();
    let color = if present { theme.success } else { theme.muted_foreground.opacity(0.3) };
    div().size_4().flex().flex_none().items_center().justify_center().child(div().size_2().rounded_full().bg(color))
}

/// El contador de una fila (`PanelRow.tsx`: `h-5 min-w-5 rounded-full
/// bg-primary px-1.5 text-xs font-medium text-primary-foreground`).
fn pill(id: String, text: String, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .id(SharedString::from(id))
        .h_5()
        .min_w_5()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.primary)
        .px_1p5()
        .leading_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.primary_foreground)
        .aria_label(text.clone())
        .child(text)
        .test_support()
}

/// Una fila sencilla del panel (`PanelRow.tsx` sin segunda línea): su icono
/// y su nombre, apagada hasta que pasa el ratón.
fn plain_row(id: &'static str, icon: Lucide, label: String, small: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let (muted, hover, foreground) = (theme.muted_foreground, theme.sidebar_accent.opacity(0.5), theme.sidebar_foreground);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .pl(px(4.))
        .pr_1()
        .py_1p5()
        .rounded(px(8.))
        .map(|this| if small { this.leading_xs() } else { this.leading_sm() })
        .text_color(muted)
        .hover(move |style| style.bg(hover).text_color(foreground))
        .child(Icon::new(icon).size_4().flex_none())
        .child(div().min_w_0().flex_1().truncate().child(label.clone()))
        .aria_label(label)
}

/// La lista de comunicación, en el panel.
pub struct CommsView {
    /// Lo escrito para recortar la lista. Es de esta pantalla: qué casa con
    /// ello lo dice el core.
    filter: Entity<InputState>,
    /// «Nueva conversación», abierta. La pinta quien tiene la ventana
    /// entera, encima de todo.
    pub(crate) picker: Option<Entity<NewConversation>>,
    _on_picker: Option<Subscription>,
}

impl CommsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "nav:comms.filter");
        Self { filter: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)), picker: None, _on_picker: None }
    }

    pub(crate) fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.new(|cx| NewConversation::new(window, cx));
        self._on_picker = Some(cx.subscribe(&picker, |this, _, _: &NewConversationClosed, cx| {
            this.picker = None;
            this._on_picker = None;
            cx.notify();
        }));
        self.picker = Some(picker);
        cx.notify();
    }

    /// Una fila de canal o de directo (`PanelRow.tsx` con `preview` y
    /// `meta`): el icono, el nombre con la hora a su derecha y, debajo, la
    /// última línea con el contador.
    fn render_row(row: &CommsRow, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (muted, hover, foreground, active_bg, active_ink, primary, border) = (
            theme.muted_foreground,
            theme.sidebar_accent.opacity(0.5),
            theme.sidebar_foreground,
            theme.sidebar_accent,
            theme.sidebar_accent_foreground,
            theme.primary,
            theme.sidebar_border,
        );
        let id = row_id(&row.conversation);
        let icon = match row.present {
            // Dice lo que `PresenceDot.tsx`: nunca «en línea», que el dato no lo sostiene.
            Some(present) => presence_dot(present, cx)
                .id(SharedString::from(format!("{id}-presence")))
                .aria_label(tr(cx, if present { "common:presence.activeRecently" } else { "common:presence.notActiveRecently" }))
                .test_support()
                .into_any_element(),
            None => Icon::new(if row.general { Lucide::Megaphone } else { Lucide::Hash }).size_4().flex_none().into_any_element(),
        };
        let badge = row.badge.clone().map(|text| pill(format!("{id}-badge"), text, cx));
        // Quién anda por la voz de un canal: el altavoz con cuántos, antes del
        // contador, y sus nombres colgando de la fila.
        let space_id = match &row.conversation {
            Conversation::Channel { space_id } => Some(space_id.clone()),
            Conversation::Direct { .. } => None,
        };
        let in_voice = space_id.as_deref().and_then(|space| crate::voice::occupancy_count(space, cx));
        let voice_people = space_id.as_deref().and_then(|space| crate::voice::occupancy_list(space, cx));
        let badge = (in_voice.is_some() || badge.is_some())
            .then(|| div().ml_auto().flex().flex_none().items_center().gap_1p5().children(in_voice).children(badge));
        let two_lines = row.preview.is_some() || row.time.is_some();
        // Las palomitas de lo propio, delante del texto, como en WhatsApp.
        let ticks = row.ticks.map(|read| {
            let label = tr(cx, if read { "chat:readByOther" } else { "chat:sentNotRead" });
            div().id(SharedString::from(format!("{id}-ticks"))).flex_none().aria_label(label).test_support().child(
                Icon::new(if read { Lucide::CheckCheck } else { Lucide::Check }).size_3p5().text_color(if read { primary } else { muted }),
            )
        });
        let name = div().min_w_0().flex_1().truncate().child(row.name.clone());
        let text = if two_lines {
            let preview = row.preview.clone().unwrap_or_default();
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .child(div().min_w_0().flex().items_baseline().gap_2().child(name).children(row.time.clone().map(|time| {
                    div()
                        .id(SharedString::from(format!("{id}-time")))
                        .flex_none()
                        .text_size(px(10.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(muted)
                        .aria_label(time.clone())
                        .child(time)
                        .test_support()
                })))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div().min_w_0().flex_1().flex().items_center().gap_1().children(ticks).child(
                                div()
                                    .id(SharedString::from(format!("{id}-preview")))
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .leading_xs()
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(muted)
                                    .aria_label(preview.clone())
                                    .child(preview)
                                    .test_support(),
                            ),
                        )
                        .children(badge),
                )
                .into_any_element()
        } else {
            div().min_w_0().flex_1().flex().items_center().gap_2().child(name).children(badge).into_any_element()
        };
        let conversation = row.conversation.clone();
        let line = div()
            .id(SharedString::from(id))
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .pl(px(4.))
            .pr_1()
            .py_1p5()
            .rounded(px(8.))
            .leading_sm()
            .when(row.active, |this| this.bg(active_bg).font_weight(FontWeight::MEDIUM).text_color(active_ink))
            .when(!row.active, |this| this.text_color(muted).hover(move |style| style.bg(hover).text_color(foreground)))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                // Con la sala abierta, elegir una conversación es ir a
                // leerla: la sala se minimiza y la llamada sigue.
                let core = Backend::core(cx);
                core.voice().close_stage();
                core.comms().select(conversation.clone());
            })
            .aria_label(row.name.clone())
            .aria_selected(row.active)
            .child(icon)
            .child(text)
            .test_support();
        // La sala general, separada de los canales de los espacios
        // (`mb-1 border-b border-sidebar-border pb-1`).
        let line = match voice_people {
            Some(people) => div().flex().flex_col().child(line).child(people).into_any_element(),
            None => line.into_any_element(),
        };
        if row.set_apart {
            div().flex().flex_col().mb_1().pb_1().border_b_1().border_color(border).child(line).into_any_element()
        } else {
            line
        }
    }

    /// El muro de feedback (`CommsPanel.tsx`), con un punto si hay algo
    /// publicado desde que se abrió. Abrirlo es haberlo visto.
    fn wall_row(active: bool, has_new: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (active_bg, active_ink, primary) = (theme.sidebar_accent, theme.sidebar_accent_foreground, theme.primary);
        let new = tr(cx, "common:feedback.new");
        plain_row("comms-wall", Lucide::Megaphone, tr(cx, "nav:item.feedback"), false, cx)
            .when(active, |this| this.bg(active_bg).font_weight(FontWeight::MEDIUM).text_color(active_ink))
            .cursor_pointer()
            .aria_selected(active)
            .on_click(|_, _, cx| {
                // Con la sala abierta, el muro se pone delante de ella: la
                // sala se minimiza y la llamada sigue.
                Backend::core(cx).voice().close_stage();
                Backend::core(cx).wall().show();
                Backend::run(cx, |core| async move { core.wall().opened().await });
            })
            .when(has_new, |this| this.child(div().id("comms-wall-new").size_2().flex_none().rounded_full().bg(primary).aria_label(new).test_support()))
            .test_support()
    }

    /// Una sección (`PanelSection.tsx`): la cabecera en mayúsculas, que la
    /// pliega y sigue diciendo cuánto hay sin leer, y sus filas.
    fn render_section(section: CommsSection, rows: &CommsRows, extra: Vec<AnyElement>, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (muted, foreground, primary, on_primary) = (theme.muted_foreground, theme.sidebar_foreground, theme.primary, theme.primary_foreground);
        let (id, key) = match section {
            CommsSection::Channels => ("comms-channels", "nav:comms.channels"),
            CommsSection::Directs => ("comms-directs", "nav:comms.directs"),
        };
        let label = tr(cx, key);
        let count = rows.badge.clone().map(|text| {
            div()
                .id(SharedString::from(format!("{id}-count")))
                .ml_auto()
                .h_4()
                .min_w_4()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(primary)
                .px_1()
                .text_size(px(10.))
                .line_height(px(15.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(on_primary)
                .aria_label(text.clone())
                .child(text)
                .test_support()
        });
        let body: Vec<AnyElement> =
            rows.rows.iter().map(|row| Self::render_row(row, cx)).chain(if rows.open { extra } else { Vec::new() }).collect();
        div()
            .flex()
            .flex_col()
            .py_1()
            .child(
                div().h_7().flex().flex_none().items_center().gap_1().pr_1().child(
                    div()
                        .id(id)
                        .min_w_0()
                        .flex_1()
                        .h_full()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded(px(8.))
                        .px_2()
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(muted)
                        .hover(move |style| style.text_color(foreground))
                        .cursor_pointer()
                        .on_click(move |_, _, cx| Backend::core(cx).comms().toggle_section(section))
                        .aria_label(label.clone())
                        .aria_expanded(rows.open)
                        .child(Icon::new(if rows.open { IconName::ChevronDown } else { IconName::ChevronRight }).size_3().flex_none())
                        .child(div().truncate().child(label.to_uppercase()))
                        .children(count)
                        .test_support(),
                ),
            )
            .child(div().flex().flex_col().px_1().children(body))
            .into_any_element()
    }
}

impl Render for CommsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Leer el estado es lo que hace que la lista se repinte cuando cambia.
        let _ = Backend::snapshot(cx).read(cx).comms.org_id.is_some();
        let query = self.filter.read(cx).value().to_string();
        let panel = Backend::core(cx).comms().panel(&query, locale(cx), &chrono::Local::now());
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let (on_wall, wall_new) = {
            let snapshot = Backend::snapshot(cx).read(cx);
            (snapshot.comms.on_screen && snapshot.comms.on_wall, snapshot.wall.has_new)
        };

        let mut channel_extras: Vec<AnyElement> = Vec::new();
        if panel.no_channels {
            channel_extras.push(
                div().id("comms-no-channels").px_2().py_2().leading_xs().text_color(muted).child(tr(cx, "common:last.channelsInSpaces")).test_support().into_any_element(),
            );
        }
        // «Abrir una sala general» (`panel.offers_general`) no se pinta hasta
        // que haga algo: un botón que sólo avisa en su tooltip de que no
        // está en esta app es un botón que miente.
        let mut direct_extras: Vec<AnyElement> = Vec::new();
        if panel.offers_new_conversation {
            direct_extras.push(
                plain_row("comms-new-conversation", Lucide::UserPlus, tr(cx, "nav:comms.newConversation"), true, cx)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.open_picker(window, cx)))
                    .test_support()
                    .into_any_element(),
            );
        }

        div()
            .id("comms-panel")
            .test_support()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            // Recorta lo que ya está en el panel; no es el buscador.
            .child(
                div().flex_none().px_2().py_1().child(
                    Input::new(&self.filter)
                        .id("comms-filter")
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).size_3p5().text_color(muted)),
                ),
            )
            .child(
                div()
                    .id("comms-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(div().flex().flex_col().px_1().child(Self::wall_row(on_wall, wall_new, cx)))
                    .when(panel.channels.shown, |this| this.child(Self::render_section(CommsSection::Channels, &panel.channels, channel_extras, cx)))
                    .when(panel.directs.shown, |this| this.child(Self::render_section(CommsSection::Directs, &panel.directs, direct_extras, cx)))
                    .when(panel.nothing_matches, |this| {
                        this.child(div().id("comms-no-matches").px_3().py_2().leading_xs().text_color(muted).child(tr(cx, "nav:panel.noMatches")).test_support())
                    }),
            )
    }
}

/// Seguir o silenciar el canal que se tiene delante (`ChannelView.tsx`).
/// Vive en la cabecera y no en preferencias porque es una decisión por
/// canal: los que importan se saben estando dentro. Silenciado, aquí sólo
/// avisan las menciones y las respuestas. Cambia en el acto; si el servidor
/// dice que no, vuelve solo y la conversación dice por qué.
fn follow_button(space_id: String, on: bool, thread: Entity<crate::chat::ChatView>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (primary, muted, border) = (theme.primary, theme.muted_foreground, theme.border);
    let (icon, label, hint) = if on {
        (Lucide::Bell, tr(cx, "chat:following"), tr(cx, "chat:followingTitle"))
    } else {
        (Lucide::BellOff, tr(cx, "chat:follow"), tr(cx, "chat:notFollowingTitle"))
    };
    div()
        .id("conversation-follow")
        .ml_auto()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .rounded_md()
        .border_1()
        .px_2()
        .py_1()
        .leading_xs()
        .cursor_pointer()
        .map(|this| if on { this.border_color(primary.opacity(0.4)).text_color(primary) } else { this.border_color(border).text_color(muted) })
        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
        .aria_label(label.clone())
        .aria_selected(on)
        .on_click(move |_, _, cx| {
            let space = space_id.clone();
            let answer = Backend::ask(cx, move |core| async move { core.comms().set_following(&space, !on).await });
            let thread = thread.clone();
            cx.spawn(async move |cx| {
                let Ok(Err(error)) = answer.await else { return };
                cx.update(|cx| {
                    let said = error_text(cx, &error, "common:misc.errSave");
                    thread.update(cx, |view, cx| view.say(said, cx));
                });
            })
            .detach();
        })
        .child(Icon::new(icon).size_3().flex_none())
        .child(label)
        .test_support()
}

/// «Llamar con invitados» (`ChannelView.tsx`: `h-8 rounded-lg border px-2.5
/// text-[13px] font-semibold`).
fn guest_button(tabs: Entity<crate::channel_tabs::ChannelTabsView>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (muted, border, hover, foreground) = (theme.muted_foreground, theme.border, theme.accent, theme.foreground);
    let (label, hint) = (tr(cx, "calls:inviteButton"), tr(cx, "calls:dialogTitle"));
    div()
        .id("conversation-guest-call")
        .h_8()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .rounded(px(8.))
        .border_1()
        .border_color(border)
        .px(px(10.))
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(muted)
        .cursor_pointer()
        .hover(move |style| style.bg(hover).text_color(foreground))
        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
        .aria_label(label.clone())
        .on_click(move |_, window, cx| tabs.update(cx, |tabs, cx| tabs.open_guest(window, cx)))
        .child(Icon::new(Lucide::UserPlus).size_3p5().flex_none())
        .child(label)
        .test_support()
}

/// La conversación que se tiene delante: su cabecera —el nombre del canal
/// con quién anda por aquí, o el de la persona— y, debajo, sus mensajes
/// (`chat.rs`).
pub fn conversation(thread: &Entity<crate::chat::ChatView>, tabs: &Entity<crate::channel_tabs::ChannelTabsView>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (border, muted, success) = (theme.border, theme.muted_foreground, theme.success);
    let now = chrono::Utc::now();
    let title = cac_core::conversation_title(&Backend::snapshot(cx).read(cx).comms, now);
    // Sin conversación —una organización sin espacios— no hay cabecera, y se
    // dice lo que dice `Channels.tsx`.
    let Some(title) = title else {
        let said = tr(cx, "nav:comms.nothingToRead");
        return div()
            .id("conversation")
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("conversation-not-yet")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .px_4()
                    .leading_sm()
                    .text_color(muted)
                    .aria_label(said.clone())
                    .child(Icon::new(Lucide::MessagesSquare).size_4().flex_none())
                    .child(said)
                    .test_support(),
            )
            .test_support()
            .into_any_element();
    };
    let direct = title.direct;
    let follow = {
        let comms = &Backend::snapshot(cx).read(cx).comms;
        match (&comms.selected, comms.selected_followed) {
            // Un directo no trae «se sigue»: sólo los canales se silencian.
            (Some(Conversation::Channel { space_id }), Some(on)) => Some(follow_button(space_id.clone(), on, thread.clone(), cx)),
            _ => None,
        }
    };
    // Quién de la organización anda por aquí (`QuienAnda`): tres nombres y
    // cuántos más, con todos al pasar el ratón. Sólo en un canal.
    let around = (!direct).then(|| Backend::core(cx).comms().around(now)).filter(|names| !names.is_empty());
    let around = around.and_then(|names| {
        let label = cac_core::around_label(&names)?;
        let everyone = cac_core::text_with(locale(cx), "chat:aroundTitle", &[("names", &names.join(", "))]);
        Some(
            div()
                .id("conversation-around")
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .leading_xs()
                .text_color(muted)
                .tooltip(move |window, cx| Tooltip::new(everyone.clone()).build(window, cx))
                .aria_label(label.clone())
                .child(div().size_1p5().flex_none().rounded_full().bg(success))
                .child(label)
                .test_support(),
        )
    });
    let guest = (!direct && Backend::snapshot(cx).read(cx).comms.org_id.is_some()).then(|| guest_button(tabs.clone(), cx));
    let header = div()
        .id("conversation-header")
        .h(px(HEADER_HEIGHT))
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(border)
        .when(direct, |this| this.child(presence_dot(title.present, cx)))
        .child(
            div()
                .id("conversation-title")
                .min_w_0()
                .truncate()
                .leading_sm()
                .font_weight(FontWeight::MEDIUM)
                .aria_label(title.title.clone())
                .child(title.title)
                .test_support(),
        )
        .children(around)
        // La puerta de la voz, al lado del nombre como en `ChannelView.tsx`:
        // la conversación hablada es del canal, no otra pantalla.
        .when_some(match &Backend::snapshot(cx).read(cx).comms.selected {
            Some(Conversation::Channel { space_id }) => Some(space_id.clone()),
            _ => None,
        }, |this, space_id| this.child(crate::voice::voice_bar(&space_id, cx)))
        // Una reunión aparte para gente de fuera, al lado de la voz del canal
        // y no dentro de ella: son dos salas distintas.
        .children(guest)
        .children(follow)
        // `ml-auto text-xs text-muted-foreground` (`DMThread.tsx`).
        .when(direct, |this| this.child(div().id("conversation-private").ml_auto().flex_none().leading_xs().text_color(muted).child(tr(cx, "chat:private")).test_support()))
        .test_support();
    // Un canal lleva sus pestañas, con la conversación dentro; un directo,
    // sólo la conversación.
    let body: AnyElement = if direct { thread.clone().into_any_element() } else { tabs.clone().into_any_element() };
    div().id("conversation").size_full().flex().flex_col().child(header).child(div().flex_1().min_h_0().flex().flex_col().child(body)).test_support().into_any_element()
}

/// «Nueva conversación» se cerró: quien la tiene la quita.
pub struct NewConversationClosed;

/// Escribirle a alguien de la organización: un buscador y la gente
/// (`NewConversation.tsx`). A quién se ofrece lo dice el core.
pub struct NewConversation {
    search: Entity<InputState>,
    /// Abriendo la conversación: otro clic no abre otra.
    busy: bool,
    /// Por qué no se abrió, dicho en el diálogo.
    problem: Option<String>,
    /// La gente ya llegó: hasta entonces no se dice «no hay nadie más».
    loaded: bool,
    focus: FocusHandle,
}

impl EventEmitter<NewConversationClosed> for NewConversation {}

impl NewConversation {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "common:last.findSomebody");
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        search.update(cx, |search, cx| search.focus(window, cx));
        let people = Backend::ask(cx, |core| async move { core.comms().load_people().await });
        cx.spawn(async move |this, cx| {
            let _ = people.await;
            this.update(cx, |this, cx| {
                this.loaded = true;
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self { search, busy: false, problem: None, loaded: false, focus: cx.focus_handle() }
    }

    /// Abre la conversación con esa persona. Si se abrió, el diálogo se
    /// cierra y el core ya la dejó delante; si no, se dice por qué.
    fn start(&mut self, user_id: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.problem = None;
        cx.notify();
        let opened = Backend::ask(cx, move |core| async move { core.comms().open_with(&user_id).await });
        cx.spawn(async move |this, cx| {
            let Ok(opened) = opened.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match opened {
                    Ok(_) => cx.emit(NewConversationClosed),
                    Err(error) => this.problem = Some(error_text(cx, &error, "chat:couldNotOpenConversation")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_person(person: &Person, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (hover, muted) = (theme.accent, theme.muted_foreground);
        let present = cac_core::seen_recently(person.last_seen_at.as_deref(), chrono::Utc::now());
        let user_id = person.id.clone();
        let name = person.shown_name().to_string();
        // El usuario al lado sólo si hay un nombre que enseñar delante.
        let handle = (name != person.username).then(|| format!("@{}", person.username));
        div()
            .id(SharedString::from(format!("new-conversation-{}", person.id)))
            .w_full()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .rounded(px(4.))
            .px_2()
            .py_1p5()
            .leading_sm()
            .hover(move |style| style.bg(hover))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.start(user_id.clone(), cx)))
            .aria_label(name.clone())
            .child(presence_dot(present, cx))
            .child(div().min_w_0().truncate().child(name))
            .children(handle.map(|handle| div().min_w_0().truncate().leading_xs().text_color(muted).child(handle)))
            .test_support()
    }
}

impl Focusable for NewConversation {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for NewConversation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_string();
        let people = Backend::core(cx).comms().people(&query);
        let theme = cx.theme();
        let (background, foreground, muted, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.danger);
        let list: Vec<AnyElement> = people.iter().map(|person| Self::render_person(person, cx).into_any_element()).collect();
        let nobody = self.loaded && list.is_empty();
        let dialog = div()
            .id("new-conversation")
            .test_support()
            .relative()
            .w(px(PICKER_WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .rounded(px(12.))
            .border_1()
            .border_color(foreground.opacity(0.1))
            .bg(background)
            .text_color(foreground)
            .shadow_lg()
            // Un clic dentro no es un clic fuera.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().w_full().text_size(px(16.)).line_height(px(16.)).font_weight(FontWeight::MEDIUM).child(tr(cx, "nav:comms.newConversation")))
            .child(
                div().absolute().top_2().right_2().child(
                    Button::new("new-conversation-close")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::X))
                        .tooltip(tr(cx, "common:action.close"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(NewConversationClosed))),
                ),
            )
            .child(
                div().w_full().flex().items_center().gap_1p5().child(
                    div().min_w_0().flex_1().child(
                        Input::new(&self.search)
                            .id("new-conversation-search")
                            .prefix(Icon::new(IconName::Search).size_3p5().text_color(muted)),
                    ),
                )
                .when(self.busy, |this| this.child(div().id("new-conversation-busy").flex_none().child(Spinner::new().color(muted)).test_support())),
            )
            .children(self.problem.clone().map(|problem| {
                div().id("new-conversation-problem").w_full().leading_xs().text_color(danger).aria_label(problem.clone()).child(problem).test_support()
            }))
            .when(nobody, |this| {
                this.child(div().id("new-conversation-nobody").px_2().py_1().leading_xs().text_color(muted).child(tr(cx, "common:misc.nobodyElse")).test_support())
            })
            .when(!list.is_empty(), |this| {
                this.child(div().id("new-conversation-people").max_h(px(PICKER_LIST_HEIGHT)).overflow_y_scroll().flex().flex_col().gap_0p5().children(list))
            });
        div()
            .id("new-conversation-backdrop")
            .size_full()
            .track_focus(&self.focus)
            // Tapa lo de debajo: mientras está abierto no se pulsa nada más.
            .occlude()
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(NewConversationClosed);
                    cx.stop_propagation();
                }
            }))
            // Fuera del diálogo se cierra, como el de la app de siempre.
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(NewConversationClosed)))
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(black().opacity(0.35))
            .child(dialog)
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer, Gate};
    use crate::workspace::Workspace;
    use cac_core::Conversation;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, SystemNotificationResponse, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, ResponseTemplate};

    fn get(route: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET")).and(path(route.to_string()))
    }

    fn org(id: &str, name: &str) -> Value {
        json!({ "id": id, "name": name, "slug": id, "role": "member", "memberCount": 3, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" })
    }

    fn space(id: &str, org_id: &str, name: &str, kind: Option<&str>) -> Value {
        json!({ "id": id, "orgId": org_id, "name": name, "color": "#888", "kind": kind, "folders": [],
            "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 1, "taskCount": 1 }], "people": [] })
    }

    fn latest(space: &str, author: &str, name: &str, kind: &str, body: &str) -> Value {
        // De enero: la hora de la fila es una fecha, la misma se corra cuando se corra.
        json!({ "spaceId": space, "authorUserId": author, "authorName": name, "kind": kind, "body": body, "createdAt": "2026-01-05T15:00:00Z" })
    }

    fn directs() -> Value {
        json!([
            { "conversationId": "c1", "orgId": "o1", "userId": "u2", "username": "luis", "unread": 2, "lastMessageAt": "2026-01-05T15:00:00Z",
              "lastBody": "¿vienes?", "lastAuthorId": "u2", "lastSeenAt": "2020-01-01T00:00:00Z" },
            { "conversationId": "c3", "orgId": "o1", "userId": "u4", "username": "caro", "unread": 0, "lastMessageAt": "2026-01-04T15:00:00Z",
              "lastBody": "**hecho**", "lastAuthorId": "u1", "otherReadAt": "2026-01-04T16:00:00Z" },
            { "conversationId": "c9", "orgId": "o2", "userId": "u9", "username": "zoe", "unread": 40, "lastBody": "de otra organización", "lastAuthorId": "u9" },
        ])
    }

    /// Dos organizaciones. `o1`: la sala general, «Producto» (`a`) y «Diseño»
    /// (`b`); `o2`: «Soporte» (`z`). Y lo de las dos a la vez, como lo manda
    /// el servidor: lo sin leer, las últimas líneas y los directos.
    fn server() -> FakeServer {
        let server = FakeServer::start();
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org("o1", "G Studio"), org("o2", "Cliente")]))));
        let o1 = json!([space("a", "o1", "Producto", None), space("b", "o1", "Diseño", None), space("g", "o1", "General", Some("general"))]);
        server.mount(get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(o1)));
        server.mount(get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2", "Soporte", None)]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/unread").respond_with(ok(json!([{ "spaceId": "a", "count": 3 }, { "spaceId": "g", "count": 1 }, { "spaceId": "z", "count": 70 }]))));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([
            latest("a", "u2", "Luis Mora", "user", "¿sale **hoy**?"),
            latest("g", "u1", "Ana Ruiz", "system", "Grabación lista"),
            latest("z", "u9", "Zoe", "user", "de otra organización"),
        ]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(directs())));
        server
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

    /// Lo que dice un elemento. Vacío si no está a la vista.
    fn label(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).and_then(|found| found.label().map(str::to_string)).unwrap_or_default()
        })
        .unwrap()
    }

    /// Si esa fila está marcada como la que se tiene delante.
    fn marked(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).selected() == Some(true)
        })
        .unwrap()
    }

    fn top(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> f32 {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            f32::from(window.find(id).bounds().origin.y)
        })
        .unwrap()
    }

    fn type_in(window: AnyWindowHandle, field: &str, text: &str, cx: &mut TestAppContext) {
        let (field, text) = (gpui_kit::SharedString::from(field.to_string()), text.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(field, cx);
            window.input(&text, cx);
        })
        .unwrap();
    }

    fn selected(cx: &mut TestAppContext) -> Option<Conversation> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).comms.selected.clone())
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Entra a comunicación y espera a que su lista esté pintada.
    fn enter(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "rail-comms", cx);
        wait_until(cx, "el panel es la lista de comunicación, con sus filas", |cx| {
            on_screen(window, "comms-channel-a-preview", cx) && on_screen(window, "comms-direct-c1", cx)
        });
    }

    /// Pulsar «Comunicación» en el riel cambia el panel por su lista y la
    /// vista por la conversación, y «Trabajo» devuelve lo que había.
    ///
    /// Roturas que atrapa: un botón del riel que no lleva a ningún sitio; el
    /// panel con el árbol y la lista a la vez, o sin ninguna; la vista de
    /// tareas debajo de la lista de canales; volver a «Trabajo» y quedarse
    /// sin árbol; que el segundo clic en «Trabajo» ya no pliegue el panel;
    /// pintar el canal o el directo de otra organización.
    #[gpui_kit::test]
    fn the_rail_switches_between_work_and_communication(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol y las tareas llegan", |cx| on_screen(window, "tree-a", cx) && on_screen(window, "work-empty", cx));
        assert!(!on_screen(window, "comms-panel", cx) && !on_screen(window, "conversation", cx));
        // Lo pendiente llega después del árbol: leerlo de golpe daba «(2)» en
        // un CI lento (command-and-control-436).
        wait_until(cx, "el riel cuenta lo pendiente de comunicación", |cx| label(window, "rail-comms", cx) == "Comunicación (6)");

        enter(window, cx);
        assert!(!on_screen(window, "tree-everything", cx) && !on_screen(window, "tree-a", cx), "el árbol sigue debajo de la lista");
        assert!(on_screen(window, "conversation", cx) && !on_screen(window, "work-empty", cx), "la vista sigue siendo la de tareas");
        for id in ["comms-filter", "comms-wall", "comms-channel-g", "comms-channel-a", "comms-channel-b", "comms-direct-c3", "comms-new-conversation"] {
            assert!(on_screen(window, id, cx), "falta {id}");
        }
        assert!(!on_screen(window, "comms-channel-z", cx) && !on_screen(window, "comms-direct-c9", cx), "se pinta lo de otra organización");
        // Sin haber elegido nada, la sala general.
        assert_eq!(label(window, "conversation-title", cx), "#General");
        // Y debajo de su cabecera, la conversación (la pinta `chat.rs`).
        assert!(on_screen(window, "thread", cx) && !on_screen(window, "conversation-not-yet", cx));

        click(window, "rail-work", cx);
        wait_until(cx, "vuelve el árbol", |cx| on_screen(window, "tree-everything", cx));
        assert!(on_screen(window, "tree-a", cx) && on_screen(window, "work-empty", cx));
        assert!(!on_screen(window, "comms-panel", cx) && !on_screen(window, "conversation", cx));
        // Y en el módulo en el que se está, el botón pliega el panel, como antes.
        click(window, "rail-work", cx);
        assert!(!on_screen(window, "tree-everything", cx), "pulsar «Trabajo» estando en él ya no pliega el panel");
        click(window, "rail-work", cx);
        assert!(on_screen(window, "tree-everything", cx));
        // Lo mismo en comunicación: el segundo clic pliega, no sale.
        click(window, "rail-comms", cx);
        wait_until(cx, "vuelve la lista", |cx| on_screen(window, "comms-panel", cx));
        click(window, "rail-comms", cx);
        assert!(!on_screen(window, "comms-panel", cx) && on_screen(window, "conversation", cx), "pulsar «Comunicación» estando en ella no pliega el panel");
        click(window, "rail-comms", cx);
        assert!(on_screen(window, "comms-panel", cx));
        stop(cx);
    }

    /// Cada fila dice cómo se llama, lo último que se dijo, cuándo y cuánto
    /// hay sin leer; un directo, además, si lo propio se leyó y si el otro
    /// anda por aquí.
    ///
    /// Roturas que atrapa: una fila sin su última línea, sin su hora o sin
    /// su cuenta; la cuenta de un canal en otro; la cabecera de una sección
    /// sin lo suyo sin leer; la sala general debajo de los demás canales;
    /// los directos encima de los canales; las palomitas en lo que escribió
    /// el otro, o ninguna en lo propio; «activo» para quien no lo está.
    #[gpui_kit::test]
    fn a_row_says_its_name_its_last_line_its_time_and_what_is_unread(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        enter(window, cx);

        assert_eq!(label(window, "comms-channel-a", cx), "Producto");
        assert_eq!(label(window, "comms-channel-a-preview", cx), "Luis Mora: ¿sale hoy?");
        assert_eq!(label(window, "comms-channel-a-time", cx), "5/1/26");
        assert_eq!(label(window, "comms-channel-a-badge", cx), "3");
        // La puso cac por algo que hizo Ana, que es quien mira: no la firma nadie.
        assert_eq!(label(window, "comms-channel-g-preview", cx), "Grabación lista");
        assert_eq!(label(window, "comms-channel-g-badge", cx), "1");
        assert!(!on_screen(window, "comms-channel-b-preview", cx) && !on_screen(window, "comms-channel-b-badge", cx));
        assert_eq!(label(window, "comms-channels-count", cx), "4");
        assert_eq!(label(window, "comms-directs-count", cx), "2");

        assert_eq!(label(window, "comms-direct-c1", cx), "luis");
        assert_eq!(label(window, "comms-direct-c1-preview", cx), "¿vienes?");
        assert_eq!(label(window, "comms-direct-c1-badge", cx), "2");
        assert!(!on_screen(window, "comms-direct-c1-ticks", cx), "palomitas en lo que escribió el otro");
        assert_eq!(label(window, "comms-direct-c1-presence", cx), "sin actividad reciente");
        assert_eq!(label(window, "comms-direct-c3-preview", cx), "hecho");
        assert_eq!(label(window, "comms-direct-c3-ticks", cx), "Leído");
        assert!(!on_screen(window, "comms-direct-c3-badge", cx));

        let order: Vec<f32> = ["comms-wall", "comms-channels", "comms-channel-g", "comms-channel-a", "comms-channel-b", "comms-directs", "comms-direct-c1", "comms-direct-c3", "comms-new-conversation"]
            .iter()
            .map(|id| top(window, id, cx))
            .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "la lista no está en su orden: {order:?}");
        stop(cx);
    }

    /// Pulsar una fila deja esa conversación delante: la fila marcada y su
    /// nombre en la cabecera.
    ///
    /// Roturas que atrapa: un clic que no elige; marcar otra fila, o dos; la
    /// cabecera de la conversación anterior; un directo titulado como canal.
    #[gpui_kit::test]
    fn clicking_a_row_puts_that_conversation_in_front(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        enter(window, cx);
        let which = |cx: &mut TestAppContext| {
            ["comms-channel-g", "comms-channel-a", "comms-channel-b", "comms-direct-c1", "comms-direct-c3"]
                .into_iter()
                .filter(|id| marked(window, id, cx))
                .collect::<Vec<_>>()
        };
        assert_eq!(which(cx), ["comms-channel-g"]);

        click(window, "comms-channel-a", cx);
        wait_until(cx, "el canal elegido está delante", |cx| label(window, "conversation-title", cx) == "#Producto");
        assert_eq!(which(cx), ["comms-channel-a"]);
        assert_eq!(selected(cx), Some(Conversation::Channel { space_id: "a".into() }));

        click(window, "comms-direct-c1", cx);
        wait_until(cx, "el directo elegido está delante", |cx| label(window, "conversation-title", cx) == "luis");
        assert_eq!(which(cx), ["comms-direct-c1"]);

        // Fuera de comunicación no hay conversación delante que marcar; al
        // volver, la misma.
        click(window, "rail-work", cx);
        wait_until(cx, "vuelve el árbol", |cx| on_screen(window, "tree-everything", cx));
        click(window, "rail-comms", cx);
        wait_until(cx, "vuelve la lista", |cx| on_screen(window, "comms-direct-c1", cx));
        assert_eq!(which(cx), ["comms-direct-c1"]);
        assert_eq!(label(window, "conversation-title", cx), "luis");
        stop(cx);
    }

    /// Un stream que entrega ese evento cuando la prueba suelta la puerta
    /// que devuelve; luego, el de siempre del servidor de mentira, abierto
    /// sin decir nada.
    fn held_event(server: &FakeServer, frame: &str) -> Gate {
        let said = Gate::default();
        server.mount(get("/api/v1/events").respond_with(said.hold(ResponseTemplate::new(200).set_body_raw(frame.as_bytes().to_vec(), "text/event-stream"))).up_to_n_times(1));
        said
    }

    /// Si ya llegó la primera carga de la lista (las últimas líneas y, tras
    /// ellas, lo que se sigue): antes, el core no sabe si un mensaje es nuevo.
    fn loaded(cx: &mut TestAppContext) -> bool {
        cx.update(|cx| Backend::core(cx).comms().following().is_some())
    }

    /// Un mensaje que llega mueve su fila en pantalla, sin tocar nada.
    ///
    /// Roturas que atrapa: los avisos del core sobre la lista que no cruzan
    /// a la pantalla (habría que salir y volver a entrar para ver el
    /// mensaje); la fila que se repinta con lo de antes.
    #[gpui_kit::test]
    fn a_message_that_arrives_moves_its_row_on_screen(cx: &mut TestAppContext) {
        let server = server();
        let frame = "event: chat:message\ndata: {\"spaceId\":\"b\",\"messageId\":\"m9\",\"actorId\":\"u2\",\"kind\":\"user\",\"authorName\":\"Luis Mora\",\"preview\":\"ya está subido\",\"mentions\":null,\"repliedTo\":\"\",\"edited\":false}\n\n";
        let said = held_event(&server, frame);
        // Como el servidor de verdad: desde que se dijo, las últimas líneas
        // lo traen. Tras el evento la lista las vuelve a pedir; si el
        // servidor de mentira contestara siempre lo de antes, esa relectura
        // borraba la fila nueva.
        let told = said.clone();
        server.mount(
            get("/api/v1/chat/latest")
                .respond_with(move |_: &wiremock::Request| {
                    let mut lines = vec![latest("a", "u2", "Luis Mora", "user", "¿sale **hoy**?"), latest("g", "u1", "Ana Ruiz", "system", "Grabación lista")];
                    if told.is_open() {
                        lines.push(latest("b", "u2", "Luis Mora", "user", "ya está subido"));
                    }
                    ok(json!(lines))
                })
                .with_priority(1),
        );
        server.install(cx);
        let window = open(cx);
        enter(window, cx);
        wait_until(cx, "la lista ha cargado", |cx| label(window, "comms-channel-a-preview", cx) == "Luis Mora: ¿sale hoy?");
        assert_ne!(label(window, "comms-channel-b-preview", cx), "Luis Mora: ya está subido");
        said.open();

        wait_until(cx, "la fila enseña el mensaje que llegó", |cx| label(window, "comms-channel-b-preview", cx) == "Luis Mora: ya está subido");
        assert_eq!(label(window, "comms-channel-a-preview", cx), "Luis Mora: ¿sale hoy?");
        stop(cx);
    }

    /// Un mensaje de un canal que se sigue sale como aviso del sistema, y
    /// pulsarlo lleva a su organización y a ese canal; un aviso que no es
    /// de esta app no mueve nada.
    ///
    /// El mensaje es de otra organización que la que se mira: es el caso
    /// que obliga a cambiar antes de elegir. Roturas que atrapa: el aviso
    /// del core que no llega a enseñarse; título o cuerpo sin el canal o sin
    /// el autor; pulsar sin efecto (la etiqueta que no se apunta); que un
    /// aviso ajeno lleve a algún sitio; no cambiar de organización; no
    /// buscar el canal en las demás organizaciones; no entrar en
    /// comunicación.
    #[gpui_kit::test]
    fn a_notice_shows_and_clicking_it_opens_its_conversation(cx: &mut TestAppContext) {
        let server = server();
        server.mount(get("/api/v1/chat/following").respond_with(ok(json!(["a", "b", "g", "z"]))).with_priority(1));
        let frame = "event: chat:message\ndata: {\"spaceId\":\"z\",\"messageId\":\"m9\",\"actorId\":\"u9\",\"kind\":\"user\",\"spaceName\":\"Soporte\",\"authorName\":\"Zoe\",\"preview\":\"¿alguien?\",\"mentions\":null,\"repliedTo\":\"\",\"edited\":false}\n\n";
        let said = held_event(&server, frame);
        // Desde que se dice, la última línea de `z` es más nueva: es lo que
        // le dice al core que el mensaje es nuevo y no una corrección. Con un
        // plazo de reloj, en una máquina cargada la primera carga llegaba ya
        // con la línea nueva y el mensaje pasaba por corregido.
        let told = said.clone();
        server.mount(
            get("/api/v1/chat/latest")
                .respond_with(move |_: &wiremock::Request| {
                    let at = if told.is_open() { "2026-01-05T16:00:00Z" } else { "2026-01-05T15:00:00Z" };
                    ok(json!([{ "spaceId": "z", "authorUserId": "u9", "authorName": "Zoe", "kind": "user", "body": "…", "createdAt": at }]))
                })
                .with_priority(1),
        );
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        wait_until(cx, "la lista ha cargado", loaded);
        said.open();

        wait_until(cx, "sale el aviso", |cx| !cx.shown_system_notifications().is_empty());
        let shown = cx.shown_system_notifications();
        assert_eq!(shown.len(), 1);
        assert_eq!((shown[0].tag.as_ref(), shown[0].title.as_ref(), shown[0].body.as_ref()), ("chat:z", "#Soporte", "Zoe: ¿alguien?"));

        cx.simulate_system_notification_response(SystemNotificationResponse { tag: "chat:otra-cosa".into(), action_id: None });
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(200));
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).orgs.current_org_id.clone()).as_deref(), Some("o1"), "un aviso ajeno no mueve nada");

        cx.simulate_system_notification_response(SystemNotificationResponse { tag: "chat:z".into(), action_id: None });
        wait_until(cx, "el canal del aviso está delante", |cx| label(window, "conversation-title", cx) == "#Soporte");
        assert_eq!(selected(cx), Some(Conversation::Channel { space_id: "z".into() }));
        assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).orgs.current_org_id.clone()).as_deref(), Some("o2"));
        stop(cx);
    }

    /// El botón de seguir de la cabecera de un canal: dice si se sigue,
    /// cambia en el acto al pulsarlo y se lo pide al servidor; si el
    /// servidor dice que no, vuelve solo y la conversación dice por qué. En
    /// un directo no está.
    ///
    /// Roturas que atrapa: un botón que no cambia; pedir lo contrario (POST
    /// para silenciar); quedarse silenciado tras un fallo, o callarse el
    /// fallo; el botón en un directo; que la lista que se pidió tras un
    /// cambio, llegando después de otro, lo deshaga.
    #[gpui_kit::test]
    fn the_follow_button_mutes_a_channel_and_says_when_it_cannot(cx: &mut TestAppContext) {
        let server = server();
        // Como el servidor de verdad: silenciar quita el canal de lo que se
        // sigue, y es lo que contesta la lista que se vuelve a pedir.
        let following = std::sync::Arc::new(std::sync::Mutex::new(vec!["a", "b", "g"]));
        let listed = following.clone();
        // La lista que se vuelve a pedir tras silenciar `a` se retiene, con
        // lo que el servidor sabía entonces: es la que, llegando tarde, no
        // puede deshacer lo que se pulsó después. Con un plazo, sólo
        // llegaba tarde en una máquina cargada.
        let stale = Gate::default();
        let held = stale.clone();
        server.mount(
            get("/api/v1/chat/following")
                .respond_with(move |_: &wiremock::Request| {
                    let list = listed.lock().unwrap().clone();
                    if list.contains(&"a") { ok(json!(list)) } else { held.hold(ok(json!(list))) }
                })
                .with_priority(1),
        );
        let follow = |space: &str| path(format!("/api/v1/task-spaces/{space}/chat/follow"));
        let muted = following.clone();
        server.mount(Mock::given(method("DELETE")).and(follow("a")).respond_with(move |_: &wiremock::Request| {
            muted.lock().unwrap().retain(|id| *id != "a");
            ok(json!(null))
        }));
        // El fallo se retiene hasta haber visto el cambio: con un plazo, en
        // una máquina cargada llegaba antes de que la prueba mirara.
        let refusal = Gate::default();
        server.mount(Mock::given(method("DELETE")).and(follow("b")).respond_with(refusal.hold(ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": "boom" })))));
        server.install(cx);
        let window = open(cx);
        enter(window, cx);

        click(window, "comms-channel-a", cx);
        wait_until(cx, "el canal dice que se sigue", |cx| label(window, "conversation-follow", cx) == "Siguiendo");
        click(window, "conversation-follow", cx);
        wait_until(cx, "pasa a «Seguir»", |cx| label(window, "conversation-follow", cx) == "Seguir");
        wait_until(cx, "se le pide al servidor", |_| server.requests().iter().any(|request| request.method.as_str() == "DELETE" && request.url.path() == "/api/v1/task-spaces/a/chat/follow"));
        wait_until(cx, "se vuelve a pedir la lista", |_| stale.held() == 1);

        click(window, "comms-channel-b", cx);
        wait_until(cx, "el otro canal dice que se sigue", |cx| label(window, "conversation-title", cx) == "#Diseño" && label(window, "conversation-follow", cx) == "Siguiendo");
        click(window, "conversation-follow", cx);
        wait_until(cx, "cambia en el acto", |cx| label(window, "conversation-follow", cx) == "Seguir");
        wait_until(cx, "se le pide al servidor", |_| refusal.held() == 1);
        assert_eq!(label(window, "conversation-follow", cx), "Seguir", "volvió antes de que el servidor contestara");
        // La lista de antes llega ahora, ya con `b` como seguido: no manda
        // sobre lo que se pulsó después. Tras ella el core vuelve a contar lo
        // sin leer, que es cuando ya la aplicó.
        let counted = server.hits("/api/v1/chat/unread");
        stale.open();
        wait_until(cx, "llega la lista de antes", |_| server.hits("/api/v1/chat/unread") > counted);
        cx.run_until_parked();
        assert_eq!(label(window, "conversation-follow", cx), "Seguir", "la lista pedida antes deshizo el cambio");
        refusal.open();
        wait_until(cx, "el servidor dice que no y vuelve", |cx| label(window, "conversation-follow", cx) == "Siguiendo");
        wait_until(cx, "y se dice", |cx| on_screen(window, "thread-notice", cx));

        click(window, "comms-direct-c1", cx);
        wait_until(cx, "el directo está delante", |cx| label(window, "conversation-title", cx) == "luis");
        assert!(!on_screen(window, "conversation-follow", cx), "un directo no se sigue");
        stop(cx);
    }

    /// El filtro recorta la lista, plegar una sección deja lo que importa, y
    /// cambiar de organización cambia la lista entera.
    ///
    /// Roturas que atrapa: un filtro que no filtra; una lista en blanco sin
    /// decir por qué; una cabecera que no pliega; plegada, esconder el canal
    /// con algo sin leer; la lista de la organización anterior tras cambiar.
    #[gpui_kit::test]
    fn the_filter_narrows_folding_hides_and_another_organization_has_its_own(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        enter(window, cx);

        click(window, "comms-channels", cx);
        wait_until(cx, "la sección se pliega", |cx| !on_screen(window, "comms-channel-b", cx));
        // La que se tiene delante y la que tiene algo sin leer siguen a la vista.
        assert!(on_screen(window, "comms-channel-g", cx) && on_screen(window, "comms-channel-a", cx));
        assert_eq!(label(window, "comms-channels-count", cx), "4");

        type_in(window, "comms-filter", "dise", cx);
        wait_until(cx, "el filtro abre la sección y deja lo que casa", |cx| on_screen(window, "comms-channel-b", cx));
        assert!(!on_screen(window, "comms-channel-a", cx) && !on_screen(window, "comms-directs", cx) && !on_screen(window, "comms-new-conversation", cx));
        assert!(!on_screen(window, "comms-no-matches", cx));
        type_in(window, "comms-filter", "zzz", cx);
        wait_until(cx, "dice que nada casa", |cx| on_screen(window, "comms-no-matches", cx));
        assert!(!on_screen(window, "comms-channels", cx) && !on_screen(window, "comms-channel-b", cx));

        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.switch_org("o2").await;
            })
        });
        wait_until(cx, "la lista es la de la otra organización", |cx| label(window, "conversation-title", cx) == "#Soporte");
        assert!(!on_screen(window, "comms-channel-a", cx) && !on_screen(window, "comms-channel-g", cx) && !on_screen(window, "comms-direct-c1", cx));
        stop(cx);
    }

    /// «Nueva conversación»: la gente de la organización menos uno mismo, un
    /// buscador, y elegir a alguien abre su conversación y la deja delante.
    ///
    /// Roturas que atrapa: un botón que no abre nada; ofrecerse a uno mismo;
    /// un buscador que no recorta; elegir a alguien y que no pase nada, o
    /// que el diálogo se quede abierto; abrir con otra persona o sin decir
    /// la organización; Escape que no cierra, o que abre una conversación.
    #[gpui_kit::test]
    fn a_new_conversation_is_opened_by_picking_someone(cx: &mut TestAppContext) {
        let server = server();
        server.mount(get("/api/v1/users/search").and(query_param("orgId", "o1")).respond_with(ok(json!([
            { "id": "u1", "username": "ana", "name": "Ana Ruiz" },
            { "id": "u2", "username": "luis", "name": "Luis Mora" },
            { "id": "u3", "username": "bea" },
            { "id": "u4", "username": "caro", "name": "Carolina Paz" },
        ]))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/dm/open")).respond_with(ok(json!({ "id": "c3", "orgId": "o1" }))));
        server.install(cx);
        let window = open(cx);
        enter(window, cx);
        let opened = |server: &FakeServer| -> Vec<Value> {
            server.requests().iter().filter(|request| request.url.path() == "/api/v1/dm/open").filter_map(|request| serde_json::from_slice(&request.body).ok()).collect()
        };

        click(window, "comms-new-conversation", cx);
        wait_until(cx, "sale la gente", |cx| on_screen(window, "new-conversation-u2", cx));
        assert!(on_screen(window, "new-conversation-u3", cx) && !on_screen(window, "new-conversation-u1", cx), "se ofrece a uno mismo");
        assert_eq!(label(window, "new-conversation-u2", cx), "Luis Mora");

        cx.update_window(window, |_, window, cx| window.press("escape", cx)).unwrap();
        wait_until(cx, "Escape lo cierra", |cx| !on_screen(window, "new-conversation", cx));
        // Y un clic fuera del diálogo, en lo oscurecido, también; dentro, no.
        click(window, "comms-new-conversation", cx);
        wait_until(cx, "sale otra vez", |cx| on_screen(window, "new-conversation-u2", cx));
        cx.update_window(window, |_, window, cx| window.click_at("new-conversation", gpui_kit::point(px(200.), px(8.)), cx)).unwrap();
        assert!(on_screen(window, "new-conversation", cx), "un clic dentro del diálogo lo cierra");
        cx.update_window(window, |_, window, cx| window.click_at("new-conversation-backdrop", gpui_kit::point(px(12.), px(12.)), cx)).unwrap();
        wait_until(cx, "un clic fuera lo cierra", |cx| !on_screen(window, "new-conversation", cx));
        assert!(opened(&server).is_empty());

        click(window, "comms-new-conversation", cx);
        wait_until(cx, "sale la gente otra vez", |cx| on_screen(window, "new-conversation-u2", cx));
        // Por su nombre, sin acentos ni mayúsculas.
        type_in(window, "new-conversation-search", "carolina p", cx);
        wait_until(cx, "el buscador recorta", |cx| !on_screen(window, "new-conversation-u2", cx));
        assert!(!on_screen(window, "new-conversation-u3", cx));
        // Con ella ya había hilo: abrir es volver a él, no hacer otro.
        click(window, "new-conversation-u4", cx);
        wait_until(cx, "se cierra y la conversación queda delante", |cx| !on_screen(window, "new-conversation", cx) && label(window, "conversation-title", cx) == "caro");

        assert_eq!(opened(&server), [json!({ "orgId": "o1", "userId": "u4" })]);
        assert!(marked(window, "comms-direct-c3", cx));
        stop(cx);
    }

    /// Si el servidor no deja abrir la conversación, se dice en el diálogo
    /// con la frase del catálogo, y se puede elegir a otra persona.
    ///
    /// Roturas que atrapa: un rechazo que no se ve (el diálogo se queda como
    /// si nada, o se cierra sin haber abierto nada); el diálogo atascado en
    /// «abriendo» tras un fallo, sin dejar elegir otra vez.
    #[gpui_kit::test]
    fn a_conversation_the_server_refuses_is_said(cx: &mut TestAppContext) {
        let server = server();
        server.mount(get("/api/v1/users/search").respond_with(ok(json!([{ "id": "u2", "username": "luis" }, { "id": "u4", "username": "caro" }]))));
        let refusal = json!({ "success": false, "message": "You don't share that organization", "error": "not-colleagues" });
        server.mount(Mock::given(method("POST")).and(path("/api/v1/dm/open")).respond_with(ResponseTemplate::new(403).set_body_json(refusal)).up_to_n_times(1));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/dm/open")).respond_with(ok(json!({ "id": "c3", "orgId": "o1" }))));
        server.install(cx);
        let window = open(cx);
        enter(window, cx);

        click(window, "comms-new-conversation", cx);
        wait_until(cx, "sale la gente", |cx| on_screen(window, "new-conversation-u2", cx));
        click(window, "new-conversation-u2", cx);
        wait_until(cx, "se dice por qué no", |cx| on_screen(window, "new-conversation-problem", cx));
        assert_eq!(label(window, "new-conversation-problem", cx), "No estáis en la misma organización.");
        assert!(on_screen(window, "new-conversation", cx) && !on_screen(window, "new-conversation-busy", cx));
        assert_eq!(label(window, "conversation-title", cx), "#General");

        click(window, "new-conversation-u4", cx);
        wait_until(cx, "con otra persona sí se abre", |cx| !on_screen(window, "new-conversation", cx) && label(window, "conversation-title", cx) == "caro");
        stop(cx);
    }

    /// Lo que la app de siempre tiene en esta lista y aquí todavía no se
    /// hace está pintado en su sitio, y una organización sin espacios dice
    /// dónde nace un canal en vez de quedarse en blanco.
    ///
    /// Roturas que atrapa: dejar fuera el muro por no tener aún qué hacer
    /// con él; una lista vacía sin explicación; una cabecera de conversación
    /// sin conversación.
    #[gpui_kit::test]
    fn what_is_not_here_yet_is_painted_and_an_empty_organization_says_so(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        let mut admin = org("o1", "G Studio");
        admin["role"] = json!("admin");
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([admin]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/unread").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([]))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "llega la organización", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).tree.org_id.is_some() && !Backend::snapshot(cx).read(cx).tree.loading));
        click(window, "rail-comms", cx);
        wait_until(cx, "sale la lista", |cx| on_screen(window, "comms-panel", cx));

        assert_eq!(label(window, "comms-wall", cx), "Feedback");
        assert!(on_screen(window, "comms-no-channels", cx));
        assert!(!on_screen(window, "comms-open-general", cx), "se pinta un botón que no hace nada");
        assert!(on_screen(window, "comms-new-conversation", cx));
        assert!(!on_screen(window, "conversation-header", cx));
        assert_eq!(label(window, "conversation-not-yet", cx), "Todavía no hay nada que leer.");
        assert!(!on_screen(window, "comms-channels-count", cx) && !on_screen(window, "comms-directs-count", cx));
        stop(cx);
    }
}
