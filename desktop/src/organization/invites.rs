//! Invitaciones: dar de alta por correo, invitar o añadir a alguien, y qué
//! pasó con lo ya invitado (`OrgInvitations.tsx`, `OrgCreateAccount.tsx`).
//!
//! Dos verbos que se confunden: **añadir directo** mete a la persona en el
//! acto, **invitar** le manda una solicitud que acepta desde su propia app.
//! La lista incluye las caducadas: sin ellas a la vista, una vencida no se
//! podía ni retirar ni repetir.

use cac_core::{account_notice, invitation_deadline, text_with, time_ago, username_initials, valid_email, AccountNotice, AddOutcome, Invitation, Organization};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::{capitalized, card, role_chip, OrgView};
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// El texto del catálogo sin sus marcas de énfasis (`<1>…</1>`).
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for letter in text.chars() {
        match letter {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(letter),
            _ => {}
        }
    }
    out
}

impl OrgView {
    /// Busca a quién invitar en toda la plataforma, mientras se escribe.
    pub(super) fn search_people(&mut self, cx: &mut Context<Self>) {
        let query = self.people_query.read(cx).value().to_string();
        // Escribir otra cosa deja de tener elegido a quien se eligió.
        if self.picked.as_ref().is_some_and(|picked| picked.username != query) {
            self.picked = None;
        }
        self.people_turn += 1;
        let turn = self.people_turn;
        if query.trim().is_empty() || self.picked.is_some() {
            self.people_hits.clear();
            cx.notify();
            return;
        }
        let answer = Backend::ask(cx, move |core| async move { core.orgs().search_people(&query).await });
        cx.spawn(async move |this, cx| {
            let Ok(found) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.people_turn == turn {
                    this.people_hits = found.unwrap_or_default();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn pick_person(&mut self, person: cac_core::PersonHit, window: &mut Window, cx: &mut Context<Self>) {
        let username = person.username.clone();
        self.picked = Some(person);
        self.people_hits.clear();
        self.people_query.update(cx, |input, cx| input.set_value(username, window, cx));
        cx.notify();
    }

    fn invite_role(&self, org: &Organization) -> String {
        self.invite_role.clone().or_else(|| org.default_invite_role.clone()).filter(|role| !role.is_empty()).unwrap_or_else(|| "member".into())
    }

    fn clear_picked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picked = None;
        self.people_hits.clear();
        self.people_query.update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn send_invitation(&mut self, direct: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let Some(person) = self.picked.clone() else { return };
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = None;
        cx.notify();
        let role = self.invite_role(&org);
        let (id, user) = (org.id.clone(), person.id.clone());
        let answer = Backend::ask(cx, move |core| async move {
            if direct {
                core.orgs().add_or_invite(&id, &user, &role).await
            } else {
                core.orgs().invite(&id, &user, &role).await.map(|()| AddOutcome::Invited)
            }
        });
        let asked_for = self.loaded_for.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if this.loaded_for != asked_for {
                    cx.notify();
                    return;
                }
                let lang = locale(cx);
                let (who, org_name) = (person.username.as_str(), org.name.as_str());
                match result {
                    Ok(outcome) => {
                        let text = match (direct, outcome) {
                            (_, AddOutcome::Added) => text_with(lang, "common:last.addedTo", &[("who", who), ("org", org_name)]),
                            // Pidió añadir y el servidor pidió invitar: se dice.
                            (true, AddOutcome::Invited) => text_with(lang, "org:inviteInstead", &[("who", who)]),
                            (false, AddOutcome::Invited) => text_with(lang, "common:last.invitedTo", &[("who", who), ("org", org_name)]),
                        };
                        this.say(text, false, cx);
                        this.clear_picked(window, cx);
                        this.refresh(cx);
                    }
                    Err(error) => {
                        let text = crate::lang::error_text(cx, &error, if direct { "org:errAddUser" } else { "org:errInvite" });
                        this.say(text, true, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn resend(&mut self, invitation: Invitation, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let (id, invitation_id) = (org.id, invitation.id.clone());
        let who = invitation.invited_user.clone().unwrap_or_default();
        self.act(
            move |core| async move { core.orgs().resend_invitation(&id, &invitation_id).await },
            move |this, cx| {
                let text = text_with(locale(cx), "common:last.inviteRenewed", &[("who", &who)]);
                this.say(text, false, cx);
                // Sin releer, la fila seguiría diciendo «caducada».
                this.refresh(cx);
            },
            "org:errResend",
            cx,
        );
    }

    fn ask_revoke(&mut self, invitation: Invitation, window: &mut Window, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let who = invitation.invited_user.clone().unwrap_or_default();
        let title = text_with(locale(cx), "org:revokeTitle", &[("who", &who)]);
        let (body, confirm, cancel) = (tr(cx, "org:revokeBody"), tr(cx, "org:revoke"), tr(cx, "org:cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                let (id, invitation_id) = (org.id.clone(), invitation.id.clone());
                this.act(
                    move |core| async move { core.orgs().revoke_invitation(&id, &invitation_id).await },
                    move |this, cx| {
                        this.invites.retain(|candidate| candidate.id != invitation.id);
                        cx.notify();
                    },
                    "org:errRevoke",
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// Dar de alta por correo. Si el correo ya tenía cuenta, se le invita y
    /// se dice; si no salió, se enseña el enlace para pasarlo a mano.
    pub(super) fn create_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let (name, email) = (self.account_name.read(cx).value().to_string(), self.account_email.read(cx).value().to_string());
        if self.busy || name.trim().is_empty() || !valid_email(&email) {
            return;
        }
        self.busy = true;
        self.notice = None;
        cx.notify();
        let role = self.account_role.clone().unwrap_or_else(|| self.invite_role(&org));
        let id = org.id.clone();
        let (sent_name, sent_email) = (name.clone(), email.clone());
        let answer = Backend::ask(cx, move |core| async move { core.orgs().create_account(&id, &sent_name, &sent_email, &role).await });
        let asked_for = self.loaded_for.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if this.loaded_for != asked_for {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(out) => {
                        match account_notice(&out, &name, &email, &org.name, locale(cx)) {
                            AccountNotice::Said { text } => this.say(text, false, cx),
                            AccountNotice::ShowLink { lead, link } => this.account_link = Some((lead, link)),
                        }
                        this.account_name.update(cx, |input, cx| input.set_value("", window, cx));
                        this.account_email.update(cx, |input, cx| input.set_value("", window, cx));
                        this.refresh(cx);
                    }
                    Err(error) => {
                        let text = crate::lang::error_text(cx, &error, "errors:account-failed");
                        this.say(text, true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// El enlace de una cuenta cuyo correo no salió, para copiarlo.
    pub(super) fn render_account_link(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (lead, link) = self.account_link.clone()?;
        let theme = cx.theme();
        let (background, foreground, muted, chip, mono) = (theme.background, theme.foreground, theme.muted_foreground, theme.muted, theme.mono_font_family.clone());
        let copied = link.clone();
        let card = div()
            .id("org-account-link-dialog")
            .w_full()
            .max_w(px(512.))
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded(px(14.))
            .border_1()
            .border_color(foreground.opacity(0.1))
            .bg(background)
            .shadow_2xl()
            .leading_sm()
            .text_color(foreground)
            .child(div().text_size(rems(1.)).font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:account.linkTitle")))
            .child(div().leading_xs().text_color(muted).child(lead))
            .child(div().id("org-account-link").px_3().py_2().rounded(px(6.)).bg(chip).font_family(mono).text_xs().child(link).test_support())
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("org-account-link-close").outline().small().label(tr(cx, "org:done")).on_click(cx.listener(|this, _, _, cx| {
                        this.account_link = None;
                        cx.notify();
                    })))
                    .child(Button::new("org-account-link-copy").primary().small().icon(Icon::new(Lucide::Copy)).label(tr(cx, "common:account.copyLink")).on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                        this.account_link = None;
                        this.say(tr(cx, "common:account.linkCopied"), false, cx);
                    }))),
            )
            .test_support();
        Some(crate::platform::veil("org-account-link-veil", card, window).into_any_element())
    }

    pub(super) fn render_invites(&mut self, org: &Organization, manage: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, danger, chip, card_bg, popover) = (theme.border, theme.muted_foreground, theme.danger, theme.muted, theme.group_box, theme.popover);
        let busy = self.busy;
        let invite_role = self.invite_role(org);
        let account_role = self.account_role.clone().unwrap_or_else(|| invite_role.clone());
        let now = chrono::Local::now();
        let lang = locale(cx);

        let create = manage.then(|| {
            let (name, email) = (self.account_name.read(cx).value().to_string(), self.account_email.read(cx).value().to_string());
            let ready = !name.trim().is_empty() && valid_email(&email) && !busy;
            let view = cx.entity().downgrade();
            card("org-create-account", cx)
                .child(div().flex().flex_col().gap_1().child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:account.createTitle"))).child(div().leading_xs().text_color(muted).child(tr(cx, "common:account.createLead"))))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().min_w(px(160.)).child(Input::new(&self.account_name).id("org-account-name")))
                        .child(div().flex_1().min_w(px(192.)).child(Input::new(&self.account_email).id("org-account-email")))
                        .child(role_chip(
                            "org-account-role".into(),
                            &account_role,
                            false,
                            move |role, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.account_role = Some(role);
                                    cx.notify();
                                })
                                .ok();
                            },
                            cx,
                        ))
                        .child(
                            Button::new("org-account-create")
                                .primary()
                                .icon(Icon::new(Lucide::MailPlus))
                                .label(tr(cx, "common:account.create"))
                                .disabled(!ready)
                                .on_click(cx.listener(|this, _, window, cx| this.create_account(window, cx))),
                        ),
                )
                .test_support()
        });

        let invite = manage.then(|| {
            let picked = self.picked.is_some();
            let view = cx.entity().downgrade();
            let hits = (!self.people_hits.is_empty()).then(|| {
                div()
                    .id("org-people-hits")
                    .flex()
                    .flex_col()
                    .p_1()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(border)
                    .bg(popover)
                    .children(self.people_hits.iter().map(|person| {
                        let chosen = person.clone();
                        div()
                            .id(SharedString::from(format!("org-person-{}", person.id)))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1p5()
                            .rounded(px(4.))
                            .hover(move |style| style.bg(chip))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| this.pick_person(chosen.clone(), window, cx)))
                            .child(div().flex_1().truncate().child(person.username.clone()))
                            .child(div().text_xs().text_color(muted).child(person.id.chars().take(8).collect::<String>()))
                            .test_support()
                    }))
                    .test_support()
            });
            card("org-invite-someone", cx)
                .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "org:inviteSomeone")))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_start()
                        .gap_2()
                        .child(div().flex_1().min_w(px(224.)).flex().flex_col().gap_1().child(Input::new(&self.people_query).id("org-people-search").cleanable(true)).children(hits))
                        .child(role_chip(
                            "org-invite-role".into(),
                            &invite_role,
                            false,
                            move |role, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.invite_role = Some(role);
                                    cx.notify();
                                })
                                .ok();
                            },
                            cx,
                        ))
                        .child(Button::new("org-add-directly").outline().label(tr(cx, "org:addDirectly")).disabled(!picked || busy).on_click(cx.listener(|this, _, window, cx| this.send_invitation(true, window, cx))))
                        .child(
                            Button::new("org-send-invitation")
                                .primary()
                                .icon(Icon::new(Lucide::UserPlus))
                                .label(tr(cx, "org:sendInvitation"))
                                .disabled(!picked || busy)
                                .on_click(cx.listener(|this, _, window, cx| this.send_invitation(false, window, cx))),
                        ),
                )
                .child(div().leading_xs().text_color(muted).child(plain(&tr(cx, "org:inviteExplain"))))
                .test_support()
        });

        let list: AnyElement = if self.invites.is_empty() {
            div().id("org-invites-none").px_3().py_6().rounded(px(12.)).border_1().border_dashed().border_color(border).text_center().text_color(muted).child(tr(cx, "org:noPending")).test_support().into_any_element()
        } else {
            div()
                .id("org-invites")
                .flex()
                .flex_col()
                .rounded(px(12.))
                .border_1()
                .border_color(border)
                .bg(card_bg)
                .overflow_hidden()
                .children(self.invites.iter().enumerate().map(|(index, invitation)| {
                    let who = invitation.invited_user.clone().unwrap_or_default();
                    let (expired, deadline) = invitation_deadline(invitation.expires_at.as_deref(), &now, lang);
                    let when = time_ago(&invitation.created_at, &now, lang);
                    let (resending, revoking) = (invitation.clone(), invitation.clone());
                    div()
                        .id(SharedString::from(format!("org-invite-{}", invitation.id)))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .px_3p5()
                        .py_2p5()
                        .when(index > 0, |this| this.border_t_1().border_color(border))
                        .child(div().size(px(28.)).flex_none().flex().items_center().justify_center().rounded_full().bg(chip).text_size(px(10.)).font_weight(FontWeight::MEDIUM).child(username_initials(&who)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(format!("@{who}")))
                                .child(div().truncate().leading_xs().text_color(muted).child(text_with(lang, "org:invitedBy", &[("who", &invitation.invited_by), ("when", &when)]))),
                        )
                        .child(div().px_2().rounded(px(6.)).bg(chip).text_xs().child(capitalized(&invitation.role)))
                        .child(
                            div()
                                .id(SharedString::from(format!("org-invite-deadline-{}", invitation.id)))
                                .px_2()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(if expired { danger.opacity(0.4) } else { border })
                                .when(expired, |this| this.text_color(danger))
                                .text_xs()
                                .aria_label(deadline.clone())
                                .child(deadline)
                                .test_support(),
                        )
                        .when(manage, |this| {
                            this.child(
                                Button::new(SharedString::from(format!("org-invite-resend-{}", invitation.id)))
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(Lucide::RotateCw))
                                    .label(tr(cx, "org:resend"))
                                    .tooltip(tr(cx, "org:freshDays"))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| this.resend(resending.clone(), cx))),
                            )
                            .child(
                                Button::new(SharedString::from(format!("org-invite-revoke-{}", invitation.id)))
                                    .ghost()
                                    .small()
                                    .text_color(danger)
                                    .icon(Icon::new(Lucide::X))
                                    .label(tr(cx, "org:revoke"))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| this.ask_revoke(revoking.clone(), window, cx))),
                            )
                        })
                        .test_support()
                }))
                .test_support()
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .children(create)
            .children(invite)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().flex().items_center().gap_1p5().font_weight(FontWeight::MEDIUM).child(Icon::new(Lucide::Mail).size_4()).child(text_with(lang, "org:pending", &[("count", &self.invites.len().to_string())])))
                    .child(list),
            )
            .into_any_element()
    }
}
