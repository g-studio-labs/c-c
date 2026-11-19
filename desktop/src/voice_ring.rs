//! El timbre en pantalla: llamar a alguien a la sala («Invitar»), la fila de
//! «Llamando a…», la tarjeta de quien te llama, sus tonos y el aviso del
//! sistema.
//!
//! Como `InvitePicker.tsx`, `RingRow.tsx` e `IncomingCall.tsx`. Qué tono
//! suena, cuándo se rinde un timbre y a quién se puede llamar lo decide el
//! core (`cac_core::voice_ring`); aquí se pinta y se hace sonar.

use cac_core::{person_initials, ring_clock, VoiceState};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;
use crate::workspace::Workspace;

/// Lo que el timbre lleva entre pintados.
#[derive(Default)]
pub(crate) struct RingUi {
    /// La lista de a quién llamar, abierta.
    pub invite_open: bool,
    /// El último timbre que entró del que se avisó al sistema: uno por
    /// timbre, no uno por pintado.
    pub announced: Option<String>,
    /// El reloj de «Llamando · 0:07» está andando.
    pub ticking: bool,
}

/// Una cara redonda con iniciales.
fn face(name: &str, size: f32, text: f32, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .size(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.accent)
        .text_size(px(text))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.accent_foreground)
        .child(person_initials(name))
}

impl Workspace {
    /// Lo que el timbre hace en cada pintado: el tono que toca, el aviso del
    /// sistema de uno nuevo, y el reloj de «Llamando».
    pub(crate) fn sync_ring(&mut self, voice: &VoiceState, cx: &mut Context<Self>) {
        crate::sound::ring(cac_core::ring_tone_now(voice), cx);
        let incoming = voice.incoming.as_ref().map(|ring| ring.ring_id.clone());
        if incoming.is_some() && incoming != self.ring.announced {
            if let Some(ring) = &voice.incoming {
                crate::notifier::show_ring(ring, cx);
            }
        }
        self.ring.announced = incoming;
        let ringing = voice.outgoing.as_ref().is_some_and(|ring| !ring.unanswered);
        if ringing && !self.ring.ticking {
            self.ring.ticking = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                let _ = this.update(cx, |this, cx| {
                    this.ring.ticking = false;
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(crate) fn toggle_invite(&mut self, cx: &mut Context<Self>) {
        self.ring.invite_open = !self.ring.invite_open;
        if self.ring.invite_open {
            // La gente de la organización, por si no se había traído.
            Backend::run(cx, |core| async move { core.comms().load_people().await });
        }
        cx.notify();
    }

    /// El botón «Invitar» de la cabecera de la sala: llama a alguien a
    /// **esta** sala.
    pub(crate) fn invite_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, card, accent) = (theme.border, theme.group_box, theme.accent);
        let label = tr(cx, "calls:invite");
        div()
            .id("voice-invite")
            .h_8()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .rounded(px(6.))
            .border_1()
            .border_color(border)
            .bg(card)
            .px(px(10.))
            .text_size(px(13.))
            .hover(move |style| style.bg(accent))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.toggle_invite(cx)))
            .aria_label(label.clone())
            .aria_selected(self.ring.invite_open)
            .child(Icon::new(Lucide::UserPlus).size(px(15.)))
            .child(label)
            .test_support()
            .into_any_element()
    }

    /// A quién llamar: la gente de la organización que no está dentro. Sin
    /// buscador: a quien llamas por voz es casi siempre alguien con quien
    /// ya estabas hablando.
    pub(crate) fn render_invite(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.ring.invite_open {
            return None;
        }
        let theme = cx.theme();
        let (popover, border, muted, accent) = (theme.popover, theme.border, theme.muted_foreground, theme.accent);
        let people = Backend::core(cx).voice().invitable();
        let list: AnyElement = if people.is_empty() {
            div().px_3().py_4().text_center().leading_xs().text_color(muted).child(tr(cx, "common:misc.everyoneHere")).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .children(people.into_iter().map(|person| {
                    let name = person.shown_name().to_string();
                    let (id, called) = (person.id.clone(), name.clone());
                    div()
                        .id(SharedString::from(format!("voice-invite-{}", person.id)))
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .rounded(px(4.))
                        .px_2()
                        .py_1p5()
                        .text_size(px(13.))
                        .hover(move |style| style.bg(accent))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ring.invite_open = false;
                            let (id, called) = (id.clone(), called.clone());
                            Backend::run(cx, move |core| async move { core.voice().ring(&id, &called).await });
                            cx.notify();
                        }))
                        .aria_label(name.clone())
                        .child(face(&name, 24., 10., cx))
                        .child(div().min_w_0().flex_1().truncate().child(name))
                        .child(Icon::new(Lucide::Phone).size_3p5().flex_none().text_color(muted))
                        .test_support()
                }))
                .into_any_element()
        };
        Some(
            div()
                .id("voice-invite-list")
                .absolute()
                .top(px(56.))
                .right_4()
                .occlude()
                .w(px(256.))
                .max_h(px(288.))
                .overflow_y_scroll()
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .bg(popover)
                .p_1()
                .shadow_lg()
                .child(list)
                .test_support()
                .into_any_element(),
        )
    }

    /// La fila de «Llamando a X · 0:07», encima de los mandos: cancelar, y al
    /// rendirse «no contestó» con volver a llamar o quitarla.
    pub(crate) fn render_ring_row(&self, voice: &VoiceState, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ring = voice.outgoing.as_ref()?;
        let theme = cx.theme();
        let (border, sidebar, muted, danger, success, card, accent) =
            (theme.border, theme.sidebar, theme.muted_foreground, theme.danger, theme.success, theme.group_box, theme.accent);
        let lang = locale(cx);
        let (title, detail) = if ring.unanswered {
            (ring.name.clone(), tr(cx, "calls:didNotAnswer"))
        } else {
            (
                cac_core::text_with(lang, "calls:ringing", &[("name", &ring.name)]),
                cac_core::text_with(lang, "calls:callingFor", &[("clock", &ring_clock(&ring.started_at, chrono::Utc::now()))]),
            )
        };
        let (user, name) = (ring.user_id.clone(), ring.name.clone());
        let small = |id: &'static str| div().id(id).h_8().flex().items_center().gap_1p5().rounded(px(6.)).border_1().px_3().leading_xs().cursor_pointer();
        let actions: AnyElement = if ring.unanswered {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    small("voice-ring-again")
                        .border_color(border)
                        .bg(card)
                        .hover(move |style| style.bg(accent))
                        .on_click(move |_, _, cx| {
                            let (user, name) = (user.clone(), name.clone());
                            Backend::run(cx, move |core| async move { core.voice().ring(&user, &name).await })
                        })
                        .child(Icon::new(Lucide::RotateCcw).size_3p5())
                        .child(tr(cx, "calls:ringAgain"))
                        .test_support(),
                )
                .child(
                    div()
                        .id("voice-ring-dismiss")
                        .size_8()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(border)
                        .bg(card)
                        .text_color(muted)
                        .cursor_pointer()
                        .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.voice().cancel_ring().await }))
                        .aria_label(tr(cx, "common:last.dismiss"))
                        .child(Icon::new(Lucide::X).size_3p5())
                        .test_support(),
                )
                .into_any_element()
        } else {
            small("voice-ring-cancel")
                .border_color(danger.opacity(0.4))
                .bg(danger.opacity(0.1))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(danger)
                .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.voice().cancel_ring().await }))
                .child(Icon::new(Lucide::PhoneOff).size_3p5())
                .child(tr(cx, "calls:cancelRing"))
                .test_support()
                .into_any_element()
        };
        Some(
            div()
                .id("voice-ring-row")
                .flex()
                .flex_none()
                .items_center()
                .gap_3()
                .border_t_1()
                .border_color(border)
                .bg(sidebar)
                .px_5()
                .py(px(10.))
                .child(face(&ring.name, 36., 12., cx).map(|this| if ring.unanswered { this.opacity(0.5) } else { this.border_2().border_color(success) }))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().id("voice-ring-title").truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).aria_label(title.clone()).child(title).test_support())
                        .child(div().id("voice-ring-detail").leading_xs().text_color(muted).aria_label(detail.clone()).child(detail).test_support()),
                )
                .child(div().ml_auto().flex_none().child(actions))
                .test_support()
                .into_any_element(),
        )
    }

    /// Quién te llama, encima de todo, estés donde estés.
    pub(crate) fn render_incoming(&self, voice: &VoiceState, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ring = voice.incoming.as_ref()?;
        let theme = cx.theme();
        let (background, card, border, muted, success, danger) = (theme.background, theme.group_box, theme.border, theme.muted_foreground, theme.success, theme.danger);
        let lang = locale(cx);
        let mut detail = match &ring.invite_id {
            Some(_) => cac_core::text_with(lang, "calls:callingYouTo", &[("title", &ring.title)]),
            None => cac_core::text_with(lang, "calls:callingYouToChannel", &[("channel", &ring.space_name)]),
        };
        let inside = voice.occupancy.get(&ring.space_id).map_or(0, Vec::len) as i64;
        if inside > 0 {
            detail.push_str(" · ");
            detail.push_str(&cac_core::text_count(lang, "calls:inVoice", inside));
        }
        Some(
            div()
                .id("voice-incoming")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(background.opacity(0.85))
                .child(
                    div()
                        .w(px(360.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(border)
                        .bg(card)
                        .p_6()
                        .flex()
                        .flex_col()
                        .items_center()
                        .shadow_2xl()
                        .child(face(&ring.from.name, 72., 20., cx).border_2().border_color(success.opacity(0.5)))
                        .child(div().id("voice-incoming-name").mt_4().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).aria_label(ring.from.name.clone()).child(ring.from.name.clone()).test_support())
                        .child(
                            div()
                                .id("voice-incoming-detail")
                                .mt_1()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_size(px(13.))
                                .text_color(muted)
                                .aria_label(detail.clone())
                                .child(Icon::new(Lucide::Volume2).size_3p5().text_color(success))
                                .child(detail)
                                .test_support(),
                        )
                        .child(
                            div()
                                .mt_5()
                                .w_full()
                                .flex()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .id("voice-incoming-decline")
                                        .h_10()
                                        .flex_1()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .rounded(px(8.))
                                        .border_1()
                                        .border_color(danger.opacity(0.4))
                                        .bg(danger.opacity(0.1))
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(danger)
                                        .cursor_pointer()
                                        .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.voice().decline_ring().await }))
                                        .child(Icon::new(Lucide::PhoneOff).size_4())
                                        .child(tr(cx, "calls:declineCall"))
                                        .test_support(),
                                )
                                .child(
                                    div()
                                        .id("voice-incoming-join")
                                        .h_10()
                                        .flex_1()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(8.))
                                        .bg(success)
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(background)
                                        .cursor_pointer()
                                        .on_click(|_, _, cx| {
                                            // A comunicación, donde está la sala.
                                            let core = Backend::core(cx);
                                            core.comms().enter();
                                            Backend::run(cx, |core| async move { core.voice().accept_ring().await });
                                        })
                                        .child(tr(cx, "calls:join"))
                                        .test_support(),
                                ),
                        ),
                )
                .test_support()
                .into_any_element(),
        )
    }
}
