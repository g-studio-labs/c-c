//! Las invitaciones que esperan a quien mira (`Invitations.tsx`): de qué
//! organización, quién invitó y con qué papel; aceptar o rechazar.

use cac_core::{text_with, Invitation};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::*;

use super::{capitalized, Notice};
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

pub struct ReceivedView {
    pending: Option<Vec<Invitation>>,
    /// La que se está aceptando o rechazando.
    busy: Option<String>,
    notice: Option<Notice>,
}

impl ReceivedView {
    pub fn new() -> Self {
        ReceivedView { pending: None, busy: None, notice: None }
    }

    /// Las vuelve a pedir: al abrir la pantalla.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        let answer = Backend::ask(cx, |core| async move { core.orgs().my_invitations().await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                match result {
                    Ok(pending) => this.pending = Some(pending),
                    Err(error) => {
                        this.pending.get_or_insert_with(Vec::new);
                        this.notice = Some(Notice { text: crate::lang::error_text(cx, &error, "org:errLoad"), problem: true });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn answer(&mut self, invitation: Invitation, accept: bool, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(invitation.id.clone());
        self.notice = None;
        cx.notify();
        let id = invitation.id.clone();
        let answer = Backend::ask(cx, move |core| async move {
            if accept {
                core.orgs().accept_invitation(&id).await
            } else {
                core.orgs().decline_invitation(&id).await.map(|()| true)
            }
        });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = None;
                let lang = locale(cx);
                match result {
                    Ok(renewed) => {
                        if let Some(pending) = this.pending.as_mut() {
                            pending.retain(|candidate| candidate.id != invitation.id);
                        }
                        // Sin sesión renovada la invitación **sí** quedó
                        // aceptada: decir «no se pudo» llevaría a reintentar
                        // una ya gastada. Lo que falta es volver a entrar.
                        let text = match (accept, renewed) {
                            (true, true) => Some(text_with(lang, "common:last.joinedOrg", &[("name", &invitation.org_name)])),
                            (true, false) => Some(text_with(lang, "org:joinedRenew", &[("name", &invitation.org_name)])),
                            (false, _) => None,
                        };
                        this.notice = text.map(|text| Notice { text, problem: false });
                    }
                    Err(error) => {
                        let fallback = if accept { "common:misc.errAccept" } else { "common:misc.errDecline" };
                        this.notice = Some(Notice { text: crate::lang::error_text(cx, &error, fallback), problem: true });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for ReceivedView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted, border, danger, chip) = (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.danger, theme.muted);
        let lang = locale(cx);
        let rows: Vec<AnyElement> = match &self.pending {
            None => vec![div().id("invitations-loading").text_color(muted).child(tr(cx, "common:servers.loading")).test_support().into_any_element()],
            Some(pending) if pending.is_empty() => vec![div().id("invitations-none").text_color(muted).child(tr(cx, "common:misc.noPendingInvites")).test_support().into_any_element()],
            Some(pending) => pending
                .iter()
                .map(|invitation| {
                    let busy = self.busy.as_deref() == Some(invitation.id.as_str());
                    let (accepting, declining) = (invitation.clone(), invitation.clone());
                    div()
                        .id(SharedString::from(format!("invitation-{}", invitation.id)))
                        .flex()
                        .items_center()
                        .gap_3()
                        .p_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(border)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(invitation.org_name.clone()))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .leading_xs()
                                        .text_color(muted)
                                        .child(text_with(lang, "org:invitedYou", &[("who", &invitation.invited_by)]))
                                        .child(div().px_1p5().rounded(px(4.)).bg(chip).text_color(foreground).child(capitalized(&invitation.role))),
                                ),
                        )
                        .child(
                            Button::new(SharedString::from(format!("invitation-accept-{}", invitation.id)))
                                .primary()
                                .small()
                                .icon(Icon::new(Lucide::Check))
                                .label(tr(cx, "org:accept"))
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| this.answer(accepting.clone(), true, cx))),
                        )
                        .child(
                            Button::new(SharedString::from(format!("invitation-decline-{}", invitation.id)))
                                .outline()
                                .small()
                                .icon(Icon::new(Lucide::X))
                                .tooltip(tr(cx, "org:decline"))
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| this.answer(declining.clone(), false, cx))),
                        )
                        .test_support()
                        .into_any_element()
                })
                .collect(),
        };
        let notice = self.notice.clone().map(|notice| {
            div().id("invitations-notice").leading_xs().text_color(if notice.problem { danger } else { muted }).aria_label(notice.text.clone()).child(notice.text).test_support()
        });
        div().size_full().bg(background).text_color(foreground).child(
            div().id("invitations-page").size_full().overflow_y_scroll().test_support().child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(672.))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .leading_sm()
                    .child(div().flex().items_center().gap_3().child(Icon::new(Lucide::Mail).size_6().text_color(muted)).child(div().text_size(rems(1.25)).font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:misc.invitations"))))
                    .children(notice)
                    .child(div().flex().flex_col().gap_2().children(rows)),
            ),
        )
    }
}
