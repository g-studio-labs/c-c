//! General (`OrgGeneral.tsx`): qué es la organización, cómo se porta y cómo
//! termina. Tres tarjetas y no una lista, porque son tres clases de decisión:
//! nombrarla es rutina, las reglas cambian lo que ven otros, y la última no
//! se deshace.

use cac_core::{date_with_year, delete_org_confirmed, delete_org_phrase, text_with, valid_team_zone, OrgPatch, Organization};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::{card, role_chip, OrgView};
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Qué campo de la identidad se guarda.
#[derive(Clone, Copy)]
pub(super) enum Field {
    Name,
    Domain,
    Zone,
}

/// Una regla de interruptor.
#[derive(Clone, Copy)]
enum Rule {
    ClientsOwnSpace,
    GuestsDevTools,
    DoneNeedsSubtasks,
}

impl OrgView {
    /// Guarda lo que cambió en un campo. Lo que no cambió no viaja.
    pub(super) fn save_identity(&mut self, field: Field, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((org, manage)) = Self::current(cx) else { return };
        if !manage {
            return;
        }
        let patch = match field {
            Field::Name => {
                let name = self.name_input.read(cx).value().trim().to_string();
                if name.is_empty() || name == org.name {
                    return;
                }
                OrgPatch { name: Some(name), ..Default::default() }
            }
            Field::Domain => {
                let domain = self.domain_input.read(cx).value().trim().to_string();
                if domain == org.domain.clone().unwrap_or_default() {
                    return;
                }
                OrgPatch { domain: Some(domain), ..Default::default() }
            }
            Field::Zone => {
                let zone = self.zone_input.read(cx).value().trim().to_string();
                if zone == org.timezone {
                    return;
                }
                if !valid_team_zone(&zone) {
                    self.say(tr(cx, "org:badZone"), true, cx);
                    return;
                }
                OrgPatch { timezone: Some(zone), ..Default::default() }
            }
        };
        self.save(org.id, patch, cx);
    }

    fn save(&mut self, org_id: String, patch: OrgPatch, cx: &mut Context<Self>) {
        self.act(move |core| async move { core.orgs().save(&org_id, &patch).await }, |this, cx| this.say(tr(cx, "org:saved"), false, cx), "org:errSave", cx);
    }

    fn toggle(&mut self, rule: Rule, org: &Organization, cx: &mut Context<Self>) {
        let patch = match rule {
            Rule::ClientsOwnSpace => OrgPatch { clients_see_only_their_space: Some(!org.clients_see_only_their_space), ..Default::default() },
            Rule::GuestsDevTools => OrgPatch { guests_can_use_dev_tools: Some(!org.guests_can_use_dev_tools), ..Default::default() },
            Rule::DoneNeedsSubtasks => OrgPatch { done_needs_subtasks_done: Some(!org.done_needs_subtasks_done), ..Default::default() },
        };
        self.save(org.id.clone(), patch, cx);
    }

    fn open_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let phrase = delete_org_phrase(&org.name);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(phrase));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |_, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        self._subscriptions.push(subscription);
        self.deleting = Some(input);
        self.notice = None;
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let Some(input) = self.deleting.clone() else { return };
        if !delete_org_confirmed(&org.name, &input.read(cx).value()) {
            return;
        }
        let (id, name) = (org.id.clone(), org.name.clone());
        self.act(
            move |core| async move { core.orgs().delete(&id).await },
            move |this, cx| {
                this.deleting = None;
                let text = text_with(locale(cx), "common:last.orgDeleted", &[("name", &name)]);
                this.say(text, false, cx);
            },
            "common:last.errDeleteOrg",
            cx,
        );
    }

    /// Borrar se escribe entero: un sí/no se contesta por reflejo, y éste se
    /// lleva los espacios, las tareas y los canales.
    pub(super) fn render_delete_dialog(&mut self, org: &Organization, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let input = self.deleting.clone()?;
        let theme = cx.theme();
        let (background, foreground, muted, danger, chip, mono) = (theme.background, theme.foreground, theme.muted_foreground, theme.danger, theme.muted, theme.mono_font_family.clone());
        let phrase = delete_org_phrase(&org.name);
        let ready = delete_org_confirmed(&org.name, &input.read(cx).value()) && !self.busy;
        let problem = self.notice.clone().filter(|notice| notice.problem).map(|notice| notice.text);
        let type_it = text_with(locale(cx), "org:typeToConfirm", &[("phrase", "\u{0}")]);
        let (before, after) = type_it.split_once('\u{0}').unwrap_or((type_it.as_str(), ""));
        let card = div()
            .id("org-delete-dialog")
            .w_full()
            .max_w(px(448.))
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
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| match event.keystroke.key.as_str() {
                "escape" => {
                    this.deleting = None;
                    cx.notify();
                    cx.stop_propagation();
                }
                "enter" => {
                    this.confirm_delete(cx);
                    cx.stop_propagation();
                }
                _ => {}
            }))
            .child(div().text_size(rems(1.)).font_weight(FontWeight::SEMIBOLD).text_color(danger).child(text_with(locale(cx), "org:deleteTitle", &[("name", &org.name)])))
            .child(div().text_color(muted).child(format!("{} {}", tr(cx, "common:last.deleteOrgBody"), tr(cx, "org:cannotBeUndone"))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .leading_xs()
                            .text_color(muted)
                            .child(before.to_string())
                            .child(div().px_1().rounded(px(4.)).bg(chip).font_family(mono).child(phrase))
                            .child(after.to_string()),
                    )
                    .child(Input::new(&input).id("org-delete-phrase")),
            )
            .children(problem.map(|problem| div().id("org-delete-problem").leading_xs().text_color(danger).child(problem).test_support()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("org-delete-cancel").ghost().small().label(tr(cx, "common:last.cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.deleting = None;
                        cx.notify();
                    })))
                    .child(Button::new("org-delete-confirm").danger().small().label(tr(cx, "common:last.deleteIt")).disabled(!ready).on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx)))),
            )
            .test_support();
        Some(crate::platform::veil("org-delete-veil", card, window).into_any_element())
    }

    pub(super) fn render_general(&mut self, org: &Organization, manage: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger, mono) = (theme.muted_foreground, theme.danger, theme.mono_font_family.clone());
        let superadmin = Backend::snapshot(cx).read(cx).session.as_ref().is_some_and(|session| session.superadmin);
        let label = |key: &str| div().leading_xs().text_color(muted).child(tr(cx, key));
        let fact = |key: &str, value: String, mono_font: bool| {
            div().flex().flex_col().gap_0p5().child(label(key)).child(div().when(mono_font, |this| this.font_family(mono.clone()).text_xs().text_color(muted)).child(value))
        };

        let identity = card("org-identity", cx)
            .flex_1()
            .min_w(px(320.))
            .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "org:identity")))
            .child(div().flex().flex_col().gap_1().child(label("org:name")).child(Input::new(&self.name_input).id("org-name").disabled(!manage)))
            // El identificador se enseña y no se edita: las direcciones y las
            // integraciones se construyen sobre él.
            .child(fact("org:identifier", org.slug.clone(), true))
            .child(div().flex().flex_col().gap_1().child(label("org:domain")).child(Input::new(&self.domain_input).id("org-domain").disabled(!manage)))
            .child(fact("org:created", date_with_year(locale(cx), &org.created_at), false))
            .test_support();

        let view = cx.entity().downgrade();
        let org_id = org.id.clone();
        let default_role = org.default_invite_role.clone().filter(|role| !role.is_empty()).unwrap_or_else(|| "member".into());
        let switch = |id: &'static str, key: &str, on: bool, rule: Option<Rule>, cx: &mut Context<Self>| {
            let enabled = manage && rule.is_some();
            div()
                .flex()
                .items_center()
                .gap_3()
                .py_1p5()
                .child(div().flex_1().min_w_0().child(tr(cx, key)))
                .child(
                    Switch::new(id)
                        .checked(on)
                        .disabled(!enabled)
                        .when_some(rule, |switch, rule| switch.on_click(cx.listener(move |this, _: &bool, _, cx| {
                            if let Some((org, _)) = Self::current(cx) {
                                this.toggle(rule, &org, cx);
                            }
                        }))),
                )
        };
        let rules = card("org-rules", cx)
            .flex_1()
            .min_w(px(320.))
            .gap_2()
            .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "org:rules")))
            .child(
                div().flex().flex_wrap().items_center().gap_3().py_1p5().child(div().flex_1().min_w(px(160.)).child(tr(cx, "org:defaultRole"))).child(role_chip(
                    "org-default-role".into(),
                    &default_role,
                    !manage,
                    move |role, _, cx| {
                        let org_id = org_id.clone();
                        view.update(cx, |this, cx| this.save(org_id, OrgPatch { default_invite_role: Some(role), ..Default::default() }, cx)).ok();
                    },
                    cx,
                )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .py_1p5()
                    .child(div().flex_1().min_w(px(160.)).flex().flex_col().child(tr(cx, "org:teamZone")).child(div().leading_xs().text_color(muted).child(tr(cx, "org:teamZoneHint"))))
                    .child(div().w(px(224.)).max_w_full().child(Input::new(&self.zone_input).id("org-zone").disabled(!manage))),
            )
            .child(switch("org-rule-clients", "org:clientsOwnSpace", org.clients_see_only_their_space, Some(Rule::ClientsOwnSpace), cx))
            .child(switch("org-rule-guests", "org:guestsDevTools", org.guests_can_use_dev_tools, Some(Rule::GuestsDevTools), cx))
            .child(switch("org-rule-subtasks", "org:doneNeedsSubtasks", org.done_needs_subtasks_done, Some(Rule::DoneNeedsSubtasks), cx))
            // Apagado y no escondido: está en el plan, y un control que falta
            // se lee como «no se puede» y uno gris como «todavía no».
            .child(switch("org-rule-2fa", "org:require2fa", false, None, cx))
            .test_support();

        let danger_zone = card("org-danger", cx)
            .flex_1()
            .min_w(px(320.))
            .border_color(danger.opacity(0.4))
            .child(div().font_weight(FontWeight::MEDIUM).text_color(danger).child(tr(cx, "org:dangerZone")))
            .child(div().leading_xs().text_color(muted).child(tr(cx, "org:dangerBody")))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(Button::new("org-transfer").outline().small().label(tr(cx, "org:transfer")).disabled(true).tooltip(tr(cx, "org:notAvailableYet")))
                    // Sólo un superadmin, y el servidor se lo niega a los
                    // demás igual. Apagado y no escondido: que se vea que la
                    // puerta existe y a quién pedírsela.
                    .child(
                        Button::new("org-delete")
                            .outline()
                            .small()
                            .text_color(danger)
                            .label(tr(cx, "org:deleteOrg"))
                            .disabled(!superadmin)
                            .when(!superadmin, |button| button.tooltip(tr(cx, "org:onlySuperadminDeletes")))
                            .on_click(cx.listener(|this, _, window, cx| this.open_delete(window, cx))),
                    ),
            )
            .test_support();

        div().flex().flex_wrap().gap_4().child(identity).child(rules).child(danger_zone).into_any_element()
    }
}
