//! Crear una plantilla de servidor, o guardar una versión nueva de una
//! (`ServerTemplateDialog`). Una plantilla es una secuencia de etapas del
//! manifiesto, los valores que comparten y los **nombres** de los secretos
//! que cada servidor tiene que traer. Nunca un valor: cac rechaza un secreto
//! como variable. Se rellena sola al elegir etapas; sólo se toca lo que se
//! quiere distinto.

use cac_core::{from_input, provision_phrase, stage_playbooks, to_input, var_fields, AnsibleManifest, ServerTemplate, TemplateDraft};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::Value;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// El diálogo se cerró: quien lo tiene lo quita. Con la plantilla guardada,
/// su id.
pub enum TemplateClosed {
    Cancelled,
    Saved(String),
}

pub struct TemplateDialog {
    /// La que se edita; sin ella, se crea una.
    template: Option<ServerTemplate>,
    manifest: Option<AnsibleManifest>,
    draft: TemplateDraft,
    name: Entity<InputState>,
    description: Entity<InputState>,
    note: Entity<InputState>,
    /// Un campo por valor común, por nombre.
    values: Vec<(String, Entity<InputState>)>,
    busy: bool,
    problem: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TemplateClosed> for TemplateDialog {}

impl TemplateDialog {
    pub fn new(template: Option<ServerTemplate>, manifest: Option<AnsibleManifest>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = template.as_ref().map(TemplateDraft::of).unwrap_or_default();
        let input = |value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let (value, placeholder) = (value.to_string(), placeholder.to_string());
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
        };
        let name = input(&draft.name, "Swarm + Traefik", window, cx);
        let description = input(&draft.description, "", window, cx);
        let note = input("", "", window, cx);
        let subscriptions = [&name, &description, &note]
            .into_iter()
            .map(|field| {
                cx.subscribe(field, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .collect();
        TemplateDialog { manifest: manifest.filter(|m| m.is_v2()), template, draft, name, description, note, values: Vec::new(), busy: false, problem: None, _subscriptions: subscriptions }
    }

    /// Los campos de los valores comunes de las etapas elegidas: cada uno
    /// con lo que ya tenía o su default.
    fn sync_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(manifest) = &self.manifest else { return };
        let fields = var_fields(manifest, &cac_core::ordered_playbooks(manifest, &self.draft.playbooks));
        let names: Vec<String> = fields.shared.iter().map(|f| f.name.clone()).collect();
        if names == self.values.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>() {
            return;
        }
        let mut kept = std::mem::take(&mut self.values);
        for field in fields.shared {
            if let Some(at) = kept.iter().position(|(name, _)| *name == field.name) {
                self.values.push(kept.remove(at));
                continue;
            }
            let value = self.draft.values.get(&field.name).cloned().or_else(|| field.default.as_deref().and_then(|d| serde_json::from_str::<Value>(d).ok()));
            let text = value.as_ref().map(to_input).unwrap_or_default();
            let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
            let (name, like) = (field.name.clone(), field.default.as_deref().and_then(|d| serde_json::from_str::<Value>(d).ok()));
            self._subscriptions.push(cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    match from_input(&input.read(cx).value(), like.as_ref()) {
                        Some(value) => this.draft.values.insert(name.clone(), value),
                        None => this.draft.values.remove(&name),
                    };
                    cx.notify();
                }
            }));
            self.values.push((field.name.clone(), input));
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(manifest) = self.manifest.clone() else { return };
        let mut draft = self.draft.clone();
        draft.name = self.name.read(cx).value().to_string();
        draft.description = self.description.read(cx).value().to_string();
        draft.note = self.note.read(cx).value().to_string();
        let editing = self.template.as_ref().map(|t| t.id.clone());
        self.busy = true;
        self.problem = None;
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move {
            match editing {
                Some(id) => core.platform().save_template(&id, &draft, &manifest).await.map(|_| id),
                None => core.platform().create_template(&draft, &manifest).await,
            }
        });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(id) => cx.emit(TemplateClosed::Saved(id)),
                    Err(error) => this.problem = Some(provision_phrase(locale(cx), &error, "common:misc.errSave")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for TemplateDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_values(window, cx);
        let theme = cx.theme();
        let (background, foreground, muted, danger, border, mono) = (theme.background, theme.foreground, theme.muted_foreground, theme.danger, theme.border, theme.mono_font_family.clone());
        let creating = self.template.is_none();
        let title = match &self.template {
            Some(template) => cac_core::text_with(locale(cx), "common:serverConfig.editTemplate", &[("name", &template.name)]),
            None => tr(cx, "common:serverConfig.newTemplate"),
        };
        let section = |key: &str, lead: &str, cx: &App| div().flex().flex_col().gap_0p5().child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, key))).child(div().text_xs().text_color(muted).child(tr(cx, lead)));
        let mut body = div().flex().flex_col().gap_4().child(div().text_xs().text_color(muted).child(tr(cx, "common:serverConfig.templateLead")));
        if creating {
            body = body.child(
                div()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().flex().flex_col().gap_1().child(tr(cx, "common:serverConfig.templateName")).child(div().id("template-name").child(Input::new(&self.name)).test_support()))
                    .child(div().flex_1().flex().flex_col().gap_1().child(tr(cx, "common:serverConfig.templateDescription")).child(div().id("template-description").child(Input::new(&self.description)).test_support())),
            );
        }
        let manifest = self.manifest.clone();
        let stages = manifest.as_ref().map(stage_playbooks).unwrap_or_default();
        let name_of = |id: &str| manifest.as_ref().and_then(|m| m.playbook(id)).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string());
        let stage_rows = stages.iter().map(|stage| {
            let id = stage.id.clone();
            let checked = self.draft.playbooks.contains(&stage.id);
            let mut facts = Vec::new();
            if let Some(verify) = &stage.verify {
                facts.push(cac_core::text_with(locale(cx), "common:serverConfig.verifiedBy", &[("name", &name_of(verify))]));
            }
            if let Some(rollback) = &stage.rollback {
                facts.push(if rollback == "manual" { tr(cx, "common:serverConfig.rollbackManual") } else { cac_core::text_with(locale(cx), "common:serverConfig.undoneBy", &[("name", &name_of(rollback))]) });
            }
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .child(Checkbox::new(SharedString::from(format!("template-stage-{}", stage.id))).checked(checked).label(stage.name.clone()).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    this.draft.playbooks.retain(|known| known != &id);
                    if *checked {
                        this.draft.playbooks.push(id.clone());
                    }
                    cx.notify();
                })))
                .when(!stage.description.is_empty(), |row| row.child(div().text_xs().text_color(muted).child(stage.description.clone())))
                .when(!facts.is_empty(), |row| row.child(div().text_xs().text_color(muted).child(facts.join(" · "))))
        });
        let ordered = manifest.as_ref().map(|m| cac_core::ordered_playbooks(m, &self.draft.playbooks)).unwrap_or_default();
        body = body
            .child(section("common:serverConfig.stepStages", "common:serverConfig.stagesLead", cx))
            .when(manifest.is_none(), |column| column.child(div().id("template-no-manifest").text_xs().text_color(danger).child(tr(cx, "common:serverConfig.pickFolderForPlaybooks")).test_support()))
            .child(div().flex().flex_col().gap_1().children(stage_rows))
            .when(!ordered.is_empty(), |column| column.child(div().id("template-order").text_xs().text_color(muted).child(format!("{} {}", tr(cx, "common:serverConfig.runsInOrder"), ordered.join(" → "))).test_support()));
        if let Some(manifest) = &manifest {
            let fields = var_fields(manifest, &ordered);
            if !self.values.is_empty() {
                let rows = self.values.iter().map(|(name, input)| {
                    let description = fields.shared.iter().find(|f| &f.name == name).map(|f| f.description.clone()).unwrap_or_default();
                    div()
                        .id(SharedString::from(format!("template-value-{name}")))
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(div().font_family(mono.clone()).text_xs().child(name.clone()))
                        .child(Input::new(input))
                        .when(!description.is_empty(), |column| column.child(div().text_xs().text_color(muted).child(description)))
                        .test_support()
                });
                body = body.child(section("common:serverConfig.stepShared", "common:serverConfig.sharedLead", cx)).child(div().flex().flex_col().gap_2().children(rows));
            }
            if !fields.per_server.is_empty() {
                body = body.child(section("common:serverConfig.stepPerServer", "common:serverConfig.perServerLead", cx)).child(
                    div().flex().flex_col().text_xs().children(fields.per_server.iter().map(|f| {
                        div().child(format!("{}{}", f.name, if f.description.is_empty() { String::new() } else { format!(" — {}", f.description) }))
                    })),
                );
            }
            if !fields.secrets.is_empty() {
                let own = self.draft.own(&fields);
                let rows = fields.secrets.iter().map(|secret| {
                    let mine = own.contains(&secret.name);
                    let choice = |id: String, label: String, chosen: bool, make_own: bool, name: String, cx: &mut Context<Self>| {
                        Button::new(SharedString::from(id)).xsmall().when(chosen, |b| b.primary()).when(!chosen, |b| b.outline()).label(label).on_click(cx.listener(move |this, _, _, cx| {
                            let manifest = this.manifest.clone().unwrap_or_default();
                            let fields = var_fields(&manifest, &cac_core::ordered_playbooks(&manifest, &this.draft.playbooks));
                            let mut own = this.draft.own(&fields);
                            own.retain(|known| known != &name);
                            if make_own {
                                own.push(name.clone());
                            }
                            this.draft.per_server = Some(own);
                            cx.notify();
                        }))
                    };
                    let lead = if !secret.description.is_empty() { secret.description.clone() } else if secret.sudo { tr(cx, "common:serverConfig.sudoSecret") } else { String::new() };
                    div()
                        .id(SharedString::from(format!("template-secret-{}", secret.name)))
                        .flex()
                        .items_center()
                        .gap_2()
                        .p_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(border)
                        .child(div().flex_1().min_w_0().flex().flex_col().child(div().font_family(mono.clone()).text_xs().child(secret.name.clone())).child(div().text_xs().text_color(muted).child(lead)))
                        .child(choice(format!("template-secret-own-{}", secret.name), tr(cx, "common:serverConfig.secretOwn"), mine, true, secret.name.clone(), cx))
                        .child(choice(format!("template-secret-shared-{}", secret.name), tr(cx, "common:serverConfig.secretShared"), !mine, false, secret.name.clone(), cx))
                        .test_support()
                });
                let rows: Vec<_> = rows.collect();
                body = body.child(section("common:serverConfig.stepSecrets", "common:serverConfig.secretsLead", cx)).child(div().flex().flex_col().gap_1().children(rows));
            }
        }
        let name_ok = !creating || !self.name.read(cx).value().trim().is_empty();
        let can_save = !self.busy && manifest.is_some() && !self.draft.playbooks.is_empty() && name_ok;
        body = body
            .child(div().flex().flex_col().gap_1().child(tr(cx, "common:serverConfig.note")).child(div().id("template-note").child(Input::new(&self.note)).test_support()))
            .children(self.problem.clone().map(|problem| div().id("template-problem").text_xs().text_color(danger).child(problem).test_support()));
        let card = div()
            .id("template-dialog")
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(TemplateClosed::Cancelled);
                    cx.stop_propagation();
                }
            }))
            .w_full()
            .max_w(px(640.))
            .max_h(px(680.))
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
            .child(div().id("template-title").aria_label(title.clone()).text_size(rems(1.)).font_weight(FontWeight::SEMIBOLD).child(title).test_support())
            .child(div().id("template-body").flex_1().min_h_0().overflow_y_scroll().child(body))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("template-cancel").outline().small().label(tr(cx, "common:serverConfig.cancel")).on_click(cx.listener(|_, _, _, cx| cx.emit(TemplateClosed::Cancelled))))
                    .child(
                        Button::new("template-save")
                            .primary()
                            .small()
                            .label(tr(cx, if creating { "common:serverConfig.createTemplate" } else { "common:serverConfig.saveVersion" }))
                            .disabled(!can_save)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
            .test_support();
        super::veil("template-dialog-veil", card, window)
    }
}
