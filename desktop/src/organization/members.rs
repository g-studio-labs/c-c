//! Miembros: quién está, con qué papel, y cuándo se le vio por última vez.

use cac_core::{initials, member_face, members_matching, text_with, time_ago, OrgMember, Organization};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::{capitalized, role_chip, OrgView};
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Las columnas: usuario, correo, papel, actividad y acciones.
const COLUMNS: [(&str, f32); 5] = [("org:thUser", 3.), ("org:thEmail", 3.), ("org:thRole", 1.6), ("org:thActivity", 1.4), ("org:thActions", 1.2)];

fn cells(cells: Vec<AnyElement>) -> Div {
    div().flex().items_center().px_3p5().gap_3().children(cells.into_iter().zip(COLUMNS).map(|(cell, (_, share))| div().flex_grow(share).flex_basis(px(0.)).min_w_0().child(cell)))
}

impl OrgView {
    fn join_self(&mut self, org: &Organization, cx: &mut Context<Self>) {
        let Some(me) = Backend::snapshot(cx).read(cx).session.as_ref().map(|session| session.id.clone()) else { return };
        let (id, name) = (org.id.clone(), org.name.clone());
        self.act(
            move |core| async move { core.orgs().add_member(&id, &me, "admin").await },
            move |this, cx| {
                let text = text_with(locale(cx), "common:crash.joined", &[("org", &name)]);
                this.say(text, false, cx);
                this.refresh(cx);
            },
            "common:crash.joinFailed",
            cx,
        );
    }

    fn change_role(&mut self, member: OrgMember, role: String, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        if member.role == role {
            return;
        }
        let (id, user) = (org.id, member.user_id.clone());
        let picked = role.clone();
        self.act(
            move |core| async move { core.orgs().set_member_role(&id, &user, &picked).await },
            move |this, cx| {
                if let Some(slot) = this.members.as_mut().and_then(|members| members.iter_mut().find(|candidate| candidate.user_id == member.user_id)) {
                    slot.role = role;
                }
                cx.notify();
            },
            "org:errRole",
            cx,
        );
    }

    /// Quitar a alguien pregunta nombrándolo: pierde el acceso a todo.
    fn ask_remove(&mut self, member: OrgMember, window: &mut Window, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let title = text_with(locale(cx), "org:removeTitle", &[("who", &member.username)]);
        let body = text_with(locale(cx), "org:removeBody", &[("org", &org.name)]);
        let (confirm, cancel) = (tr(cx, "org:remove"), tr(cx, "org:cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                let (id, user) = (org.id.clone(), member.user_id.clone());
                let username = member.username.clone();
                this.act(
                    move |core| async move { core.orgs().remove_member(&id, &user).await },
                    move |this, cx| {
                        if let Some(members) = this.members.as_mut() {
                            members.retain(|candidate| candidate.user_id != member.user_id);
                        }
                        let text = text_with(locale(cx), "common:last.memberRemoved", &[("name", &username)]);
                        this.say(text, false, cx);
                    },
                    "org:errRemove",
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// Otro enlace para quien no terminó el registro. Si el correo no sale,
    /// el enlace se ofrece para pasarlo a mano, igual que al dar de alta.
    fn resend_link(&mut self, member: OrgMember, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let (id, user) = (org.id, member.user_id.clone());
        let answer = Backend::ask(cx, move |core| async move { core.orgs().resend_account_link(&id, &user).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(out) if out.email_sent => this.say(tr(cx, "common:account.resent"), false, cx),
                    Ok(out) => {
                        if let Some(link) = out.link {
                            let name = member_face(&member).0;
                            this.account_link = Some((text_with(locale(cx), "common:account.linkLead", &[("name", &name)]), link));
                        }
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

    pub(super) fn render_members(&mut self, org: &Organization, manage: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, danger, warning, card_bg, chip) = (theme.border, theme.muted_foreground, theme.danger, theme.warning, theme.group_box, theme.muted);
        let me = Backend::snapshot(cx).read(cx).session.as_ref().map(|session| session.id.clone()).unwrap_or_default();
        let members = self.members.clone();
        let i_am_in = members.as_ref().is_none_or(|members| members.iter().any(|member| member.user_id == me));
        let query = self.search.read(cx).value().to_string();
        let visible = members.as_deref().map(|members| members_matching(members, &query, &self.role_filter));
        let now = chrono::Local::now();
        let lang = locale(cx);

        // Quien administra sin pertenecer (un superadmin las ve todas) se
        // puede meter: es lo que le hace mencionable y le deja escribir.
        let join = (manage && !i_am_in).then(|| {
            let org = org.clone();
            div()
                .id("org-join-self")
                .flex()
                .items_center()
                .gap_2()
                .p_2()
                .rounded(px(8.))
                .border_1()
                .border_dashed()
                .border_color(border)
                .leading_xs()
                .child(div().flex_1().text_color(muted).child(tr(cx, "common:crash.notAMember")))
                .child(Button::new("org-join-self-button").outline().small().label(tr(cx, "common:crash.addMe")).on_click(cx.listener(move |this, _, _, cx| this.join_self(&org, cx))))
                .test_support()
        });

        let filter_label = if self.role_filter.is_empty() { tr(cx, "org:roleAny") } else { capitalized(&self.role_filter) };
        let any = tr(cx, "org:roleAny");
        let view = cx.entity().downgrade();
        let role_filter = self.role_filter.clone();
        let filter = Button::new("org-role-filter").outline().small().label(filter_label).icon(Icon::new(Lucide::ChevronDown)).dropdown_menu(move |mut menu, _, _| {
            for (value, label) in std::iter::once((String::new(), any.clone())).chain(cac_core::ORG_ROLES.iter().map(|role| (role.to_string(), role.to_string()))) {
                let view = view.clone();
                let item = SharedString::from(format!("org-role-filter-{}", if value.is_empty() { "any" } else { value.as_str() }));
                let checked = value == role_filter;
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| div().id(item.clone()).child(label.clone()).test_support()).checked(checked).on_click(move |_, _, cx| {
                        let value = value.clone();
                        view.update(cx, |this, cx| {
                            this.role_filter = value;
                            cx.notify();
                        })
                        .ok();
                    }),
                );
            }
            menu
        });

        let heading = cells(COLUMNS.iter().map(|(key, _)| div().leading_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, key)).into_any_element()).collect()).h(px(36.));
        let rows: Vec<AnyElement> = match visible {
            None => vec![div().id("org-members-loading").py_6().text_center().text_color(muted).border_t_1().border_color(border).child(tr(cx, "common:servers.loading")).test_support().into_any_element()],
            Some(visible) => visible
                .into_iter()
                .map(|member| {
                    let (name, handle) = member_face(&member);
                    let face_initials = if member.setup_pending { name.chars().take(2).flat_map(char::to_uppercase).collect() } else { initials(member.name.as_deref(), &member.username) };
                    let who = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().size(px(24.)).flex_none().flex().items_center().justify_center().rounded_full().bg(chip).text_size(px(10.)).font_weight(FontWeight::MEDIUM).child(face_initials))
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(name))
                                .children(handle.map(|handle| div().truncate().text_size(px(11.)).text_color(muted).child(handle)))
                                .when(member.setup_pending, |this| {
                                    this.child(
                                        div()
                                            .id(SharedString::from(format!("org-member-pending-{}", member.user_id)))
                                            .flex()
                                            .child(div().px_1p5().rounded(px(4.)).bg(warning.opacity(0.15)).text_color(warning).text_size(px(10.)).font_weight(FontWeight::MEDIUM).child(tr(cx, "common:account.pending")))
                                            .test_support(),
                                    )
                                }),
                        );
                    let email = div().truncate().leading_xs().text_color(muted).child(member.email.clone().filter(|email| !email.is_empty()).unwrap_or_else(|| "—".into()));
                    let role: AnyElement = if manage {
                        let (view, changing) = (cx.entity().downgrade(), member.clone());
                        role_chip(
                            SharedString::from(format!("org-member-role-{}", member.user_id)),
                            &member.role,
                            false,
                            move |role, _, cx| {
                                let changing = changing.clone();
                                view.update(cx, |this, cx| this.change_role(changing, role, cx)).ok();
                            },
                            cx,
                        )
                        .into_any_element()
                    } else {
                        div().text_color(muted).child(capitalized(&member.role)).into_any_element()
                    };
                    // «nunca» y no un hueco: es un hecho de la cuenta, no algo
                    // que no sepamos.
                    let seen = member.last_seen_at.as_deref().map(|at| time_ago(at, &now, lang)).unwrap_or_else(|| tr(cx, "common:time.never"));
                    let (resending, removing) = (member.clone(), member.clone());
                    let actions = div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1()
                        .when(member.user_id == me, |this| this.child(div().px_1().rounded(px(4.)).bg(chip).text_size(px(10.)).text_color(muted).child(tr(cx, "org:you"))))
                        .when(manage && member.setup_pending, |this| {
                            this.child(
                                Button::new(SharedString::from(format!("org-member-resend-{}", member.user_id)))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(Lucide::RotateCw))
                                    .tooltip(tr(cx, "common:account.resend"))
                                    .on_click(cx.listener(move |this, _, _, cx| this.resend_link(resending.clone(), cx))),
                            )
                        })
                        .when(manage, |this| {
                            this.child(
                                Button::new(SharedString::from(format!("org-member-remove-{}", member.user_id)))
                                    .ghost()
                                    .xsmall()
                                    .text_color(danger)
                                    .icon(Icon::new(Lucide::Trash))
                                    .tooltip(tr(cx, "org:remove"))
                                    .on_click(cx.listener(move |this, _, window, cx| this.ask_remove(removing.clone(), window, cx))),
                            )
                        });
                    cells(vec![who.into_any_element(), email.into_any_element(), role, div().leading_xs().text_color(muted).child(seen).into_any_element(), actions.into_any_element()])
                        .id(SharedString::from(format!("org-member-{}", member.user_id)))
                        .py_2p5()
                        .border_t_1()
                        .border_color(border)
                        .test_support()
                        .into_any_element()
                })
                .collect(),
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(join)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(div().w(px(320.)).max_w_full().child(Input::new(&self.search).id("org-member-search").cleanable(true).prefix(Icon::new(IconName::Search).size_3p5().text_color(muted))))
                    .child(filter)
                    .child(div().flex_1())
                    .child(div().text_size(px(11.)).text_color(muted).child(tr(cx, "org:rolesExplain"))),
            )
            .child(div().id("org-members").flex().flex_col().rounded(px(12.)).border_1().border_color(border).bg(card_bg).overflow_hidden().child(heading).children(rows).test_support())
            .child(div().px_3().py_2().rounded(px(6.)).border_1().border_color(border).bg(chip.opacity(0.2)).leading_xs().text_color(muted).child(tr(cx, "org:guestsNotListed")))
            .into_any_element()
    }
}
