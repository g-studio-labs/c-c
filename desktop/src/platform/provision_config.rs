//! La configuración del servidor en cac (`ServerConfigPanel`): con qué
//! plantilla se monta, sus variables y **dónde** está cada secreto en
//! 1Password. Cada guardado es una versión nueva; las viejas se ven, se
//! comparan y se restauran. Editar es de admin o superadmin; los demás la
//! leen.
//!
//! Las variables se escriben como formulario cuando hay plantilla y
//! manifiesto v2, y siempre como JSON. El JSON es lo que se guarda: un campo
//! del formulario lo reescribe, y al reescribirlo a mano los campos que no
//! tienen el foco se ponen al día.

use cac_core::{
    config_body, config_changed, from_input, from_items, json_object, line_diff, provision_phrase, stable_json, time_ago, to_input, to_items, var_fields, with_required, AnsibleManifest, DiffRow, PlatformState, ProvisionState,
    SecretRef, Server, ServerConfigVersion, VarField,
};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::{Map, Value};

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

use super::provision::card;
use super::provision_template::{TemplateClosed, TemplateDialog};

/// Un campo del formulario de variables: una línea, o una fila por elemento
/// si es una lista.
enum FieldInput {
    Line(Entity<InputState>),
    List(Entity<TextareaState>),
}

impl FieldInput {
    fn text(&self, cx: &App) -> String {
        match self {
            FieldInput::Line(input) => input.read(cx).value().to_string(),
            FieldInput::List(input) => input.read(cx).value().to_string(),
        }
    }

    fn focused(&self, window: &Window, cx: &App) -> bool {
        match self {
            FieldInput::Line(input) => input.focus_handle(cx).is_focused(window),
            FieldInput::List(input) => input.focus_handle(cx).is_focused(window),
        }
    }

    fn set(&self, text: String, window: &mut Window, cx: &mut App) {
        if self.text(cx) == text {
            return;
        }
        match self {
            FieldInput::Line(input) => input.update(cx, |input, cx| input.set_value(text, window, cx)),
            FieldInput::List(input) => input.update(cx, |input, cx| input.replace_all(text, window, cx)),
        }
    }
}

struct SecretRow {
    name: Entity<InputState>,
    reference: Entity<InputState>,
}

pub struct ConfigPanel {
    server_id: String,
    template_id: String,
    vars: Entity<TextareaState>,
    note: Entity<InputState>,
    /// Los campos del formulario, por variable, y para qué plantilla son.
    fields: Vec<(VarField, FieldInput)>,
    fields_of: Option<String>,
    secrets: Vec<SecretRow>,
    /// La versión de la que salió lo escrito: si llega otra, se vuelve a
    /// leer.
    loaded_seq: Option<i64>,
    /// La versión cuyo cambio se mira.
    open_version: Option<i64>,
    /// El servidor del que se copiaría.
    copy_from: Option<String>,
    /// Lo que dijo «Probar» de cada referencia: vale, o por qué no.
    checks: Vec<(String, Result<(), String>)>,
    busy: bool,
    line: Option<(String, bool)>,
    dialog: Option<(Entity<TemplateDialog>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl ConfigPanel {
    pub fn new(server_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let vars = cx.new(|cx| TextareaState::new(window, cx).auto_grow(4, 16));
        let note = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![cx.subscribe_in(&vars, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.json_typed(window, cx);
                cx.notify();
            }
        })];
        ConfigPanel {
            server_id,
            template_id: String::new(),
            vars,
            note,
            fields: Vec::new(),
            fields_of: None,
            secrets: Vec::new(),
            loaded_seq: None,
            open_version: None,
            copy_from: None,
            checks: Vec::new(),
            busy: false,
            line: None,
            dialog: None,
            _subscriptions: subscriptions,
        }
    }

    fn input(&mut self, value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let (value, placeholder) = (value.to_string(), placeholder.to_string());
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value));
        self._subscriptions.push(cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }));
        input
    }

    /// Lo que dice la versión actual, a los campos.
    fn load(&mut self, provision: &ProvisionState, window: &mut Window, cx: &mut Context<Self>) {
        let current = provision.config.as_ref().and_then(|config| config.current.clone()).unwrap_or_default();
        self.loaded_seq = Some(current.seq);
        self.template_id = current.template_id.clone();
        let vars = current.vars.clone();
        self.vars.update(cx, |input, cx| input.replace_all(if current.seq == 0 { "{}".to_string() } else { vars }, window, cx));
        self.note.update(cx, |input, cx| input.set_value("", window, cx));
        let requires = provision.templates.iter().find(|t| t.id == current.template_id).and_then(|t| t.current.as_ref()).map(|v| v.requires.clone()).unwrap_or_default();
        let rows = with_required(&current.op_refs, &requires);
        self.secrets = rows.iter().map(|row| SecretRow { name: self.input(&row.name, "OPS_PASSWORD", window, cx), reference: self.input(&row.reference, "op://vault/item/field", window, cx) }).collect();
        self.fields_of = None;
        self.checks.clear();
    }

    fn rows(&self, cx: &App) -> Vec<SecretRef> {
        self.secrets.iter().map(|row| SecretRef { name: row.name.read(cx).value().to_string(), reference: row.reference.read(cx).value().to_string() }).collect()
    }

    fn vars_map(&self, cx: &App) -> Option<Map<String, Value>> {
        match serde_json::from_str::<Value>(&self.vars.read(cx).value()) {
            Ok(Value::Object(map)) => Some(map),
            _ => None,
        }
    }

    /// Los campos de las variables de la plantilla elegida, si hay plantilla
    /// y manifiesto v2.
    fn sync_fields(&mut self, manifest: Option<&AnsibleManifest>, provision: &ProvisionState, window: &mut Window, cx: &mut Context<Self>) {
        let template = provision.templates.iter().find(|t| t.id == self.template_id).and_then(|t| t.current.clone());
        let key = match (manifest.filter(|m| m.is_v2()), &template) {
            (Some(_), Some(version)) => Some(format!("{}:{}", self.template_id, version.seq)),
            _ => None,
        };
        if self.fields_of == key {
            return;
        }
        self.fields_of = key.clone();
        self.fields.clear();
        let (Some(manifest), Some(version), Some(_)) = (manifest, template, key) else { return };
        let fields = var_fields(manifest, &version.playbooks);
        let vars = self.vars_map(cx).unwrap_or_default();
        for field in fields.per_server.into_iter().chain(fields.shared) {
            let value = vars.get(&field.name);
            let input = if field.list {
                let text = to_items(value).join("\n");
                let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 8).default_value(text));
                self._subscriptions.push(cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.field_typed(window, cx);
                    }
                }));
                FieldInput::List(input)
            } else {
                let text = value.map(to_input).unwrap_or_default();
                let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
                self._subscriptions.push(cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.field_typed(window, cx);
                    }
                }));
                FieldInput::Line(input)
            };
            self.fields.push((field, input));
        }
    }

    /// Un campo cambió: al JSON.
    fn field_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut vars) = self.vars_map(cx) else { return };
        for (field, input) in &self.fields {
            if !input.focused(window, cx) {
                continue;
            }
            let like = field.default.as_deref().and_then(|text| serde_json::from_str::<Value>(text).ok());
            let value = if field.list {
                from_items(&input.text(cx).lines().map(str::to_string).collect::<Vec<_>>(), like.as_ref())
            } else {
                from_input(&input.text(cx), like.as_ref())
            };
            match value {
                Some(value) => vars.insert(field.name.clone(), value),
                None => vars.remove(&field.name),
            };
        }
        let text = stable_json(&Value::Object(vars));
        if self.vars.read(cx).value().as_ref() != text {
            self.vars.update(cx, |input, cx| input.replace_all(text, window, cx));
        }
        cx.notify();
    }

    /// El JSON cambió a mano: a los campos que no tienen el foco.
    fn json_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vars) = self.vars_map(cx) else { return };
        for (field, input) in &self.fields {
            if input.focused(window, cx) {
                continue;
            }
            let value = vars.get(&field.name);
            let text = if field.list { to_items(value).join("\n") } else { value.map(to_input).unwrap_or_default() };
            input.set(text, window, cx);
        }
    }

    fn pick_template(&mut self, id: String, provision: &ProvisionState, window: &mut Window, cx: &mut Context<Self>) {
        self.template_id = id;
        let requires = provision.templates.iter().find(|t| t.id == self.template_id).and_then(|t| t.current.as_ref()).map(|v| v.requires.clone()).unwrap_or_default();
        let rows = with_required(&self.rows(cx), &requires);
        let missing: Vec<SecretRef> = rows.into_iter().skip(self.secrets.len()).collect();
        for row in missing {
            let secret = SecretRow { name: self.input(&row.name, "OPS_PASSWORD", window, cx), reference: self.input("", "op://vault/item/field", window, cx) };
            self.secrets.push(secret);
        }
        cx.notify();
    }

    fn work(&mut self, cx: &mut Context<Self>, work: impl FnOnce(cac_core::Core) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), cac_core::ProvisionError>> + Send>> + Send + 'static, done_key: &'static str) {
        self.busy = true;
        self.line = None;
        cx.notify();
        let answer = Backend::ask(cx, work);
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.line = Some((tr(cx, done_key), false));
                        // Lo guardado vuelve como versión nueva: se relee.
                        this.loaded_seq = None;
                    }
                    Err(error) => this.line = Some((provision_phrase(locale(cx), &error, "common:misc.errSave"), true)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let (template, vars, rows, note) = (self.template_id.clone(), self.vars.read(cx).value().to_string(), self.rows(cx), self.note.read(cx).value().to_string());
        self.work(cx, move |core| Box::pin(async move { core.platform().save_config(&template, &vars, &rows, &note).await }), "common:serverConfig.saved");
    }

    fn restore(&mut self, version: ServerConfigVersion, window: &mut Window, cx: &mut Context<Self>) {
        let language = locale(cx);
        let seq = version.seq.to_string();
        let title = cac_core::text_with(language, "common:serverConfig.restoreTitle", &[("seq", &seq)]);
        let body = cac_core::text_with(language, "common:serverConfig.restoreBody", &[("seq", &seq)]);
        let note = cac_core::text_with(language, "common:serverConfig.restoredNote", &[("seq", &seq)]);
        let (confirm, cancel) = (tr(cx, "common:serverConfig.restore"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| this.work(cx, move |core| Box::pin(async move { core.platform().restore_config(&version, &note).await }), "common:serverConfig.saved")).ok();
        })
        .detach();
    }

    fn copy(&mut self, source: Server, window: &mut Window, cx: &mut Context<Self>) {
        let language = locale(cx);
        let title = cac_core::text_with(language, "common:serverConfig.copyTitle", &[("from", &source.name)]);
        let body = cac_core::text_with(language, "common:serverConfig.copyBody", &[("from", &source.name)]);
        let note = cac_core::text_with(language, "common:serverConfig.copiedNote", &[("from", &source.name)]);
        let (confirm, cancel) = (tr(cx, "common:serverConfig.copy"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let id = source.id;
            this.update(cx, |this, cx| this.work(cx, move |core| Box::pin(async move { core.platform().copy_config_from(&id, &note).await }), "common:serverConfig.saved")).ok();
        })
        .detach();
    }

    /// «Probar»: lee cada referencia con 1Password y descarta el valor.
    fn check(&mut self, cx: &mut Context<Self>) {
        let refs: Vec<String> = self.rows(cx).into_iter().map(|row| row.reference.trim().to_string()).filter(|r| cac_core::valid_op_ref(r)).collect();
        let answer = Backend::ask(cx, move |_| async move {
            tokio::task::spawn_blocking(move || cac_ops::secrets::check_op_refs(refs, &Default::default())).await.unwrap_or_default()
        });
        cx.spawn(async move |this, cx| {
            let Ok(checks) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.checks = checks.into_iter().map(|check| (check.op_ref, if check.ok { Ok(()) } else { Err(check.error) })).collect();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_dialog(&mut self, editing: Option<String>, manifest: Option<AnsibleManifest>, provision: &ProvisionState, window: &mut Window, cx: &mut Context<Self>) {
        let template = editing.and_then(|id| provision.templates.iter().find(|t| t.id == id).cloned());
        let dialog = cx.new(|cx| TemplateDialog::new(template, manifest, window, cx));
        let closed = cx.subscribe(&dialog, |this, _, event: &TemplateClosed, cx| {
            if let TemplateClosed::Saved(id) = event {
                this.template_id = id.clone();
                this.fields_of = None;
            }
            this.dialog = None;
            cx.notify();
        });
        self.dialog = Some((dialog, closed));
        cx.notify();
    }

    fn template_row(&self, provision: &ProvisionState, manifest: Option<&AnsibleManifest>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, accent, mono) = (theme.muted_foreground, theme.border, theme.accent, theme.mono_font_family.clone());
        let editable = provision.permissions.edit_config;
        let options = std::iter::once((String::new(), tr(cx, "common:serverConfig.noTemplate"))).chain(provision.templates.iter().map(|t| (t.id.clone(), t.name.clone())));
        let chips = options.map(|(id, name)| {
            let chosen = id == self.template_id;
            let picked = id.clone();
            let element_id = if id.is_empty() { "provision-template-none".to_string() } else { format!("provision-template-{id}") };
            let state = provision.clone();
            div()
                .id(SharedString::from(element_id))
                .aria_label(name.clone())
                .px_2()
                .py_0p5()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .when(chosen, |chip| chip.bg(accent))
                .when(editable, |chip| chip.cursor_pointer().on_click(cx.listener(move |this, _, window, cx| this.pick_template(picked.clone(), &state, window, cx))))
                .child(name)
                .test_support()
        });
        let template = provision.templates.iter().find(|t| t.id == self.template_id);
        let summary = template.and_then(|t| t.current.as_ref()).map(|v| div().font_family(mono).text_xs().text_color(muted).child(format!("{} · v{}", v.playbooks.join(" → "), v.seq)));
        let (new_state, edit_state) = (provision.clone(), provision.clone());
        let (new_manifest, edit_manifest) = (manifest.cloned(), manifest.cloned());
        let editing = template.map(|t| t.id.clone());
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().leading_xs().child(tr(cx, "common:serverConfig.template")))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .leading_xs()
                    .children(chips)
                    .when(editable, |row| {
                        row.child(
                            Button::new("provision-template-new")
                                .outline()
                                .xsmall()
                                .icon(Icon::new(Lucide::Plus))
                                .tooltip(tr(cx, "common:serverConfig.newTemplate"))
                                .on_click(cx.listener(move |this, _, window, cx| this.open_dialog(None, new_manifest.clone(), &new_state, window, cx))),
                        )
                        .when_some(editing, |row, id| {
                            row.child(
                                Button::new("provision-template-edit")
                                    .outline()
                                    .xsmall()
                                    .label(tr(cx, "common:serverConfig.edit"))
                                    .on_click(cx.listener(move |this, _, window, cx| this.open_dialog(Some(id.clone()), edit_manifest.clone(), &edit_state, window, cx))),
                            )
                        })
                    }),
            )
            .children(summary)
            .into_any_element()
    }

    fn fields_section(&self, server: &Server, provision: &ProvisionState, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.fields.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let (muted, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
        let editable = provision.permissions.edit_config;
        let vars = self.vars_map(cx).unwrap_or_default();
        let template_vars = provision.templates.iter().find(|t| t.id == self.template_id).and_then(|t| t.current.as_ref()).map(|v| json_object(&v.vars)).unwrap_or_default();
        let suggestions: [(&str, Value); 2] = [("public_ip", Value::String(server.host.clone())), ("ssh_port", Value::from(server.ssh_port))];
        let field = |field: &VarField, input: &FieldInput, inherited: bool, cx: &mut Context<Self>| {
            let control = match input {
                FieldInput::Line(input) => Input::new(input).small().disabled(!editable).into_any_element(),
                FieldInput::List(input) => Textarea::new(input).disabled(!editable).into_any_element(),
            };
            let heir = inherited.then(|| {
                let value = template_vars.get(&field.name).cloned().or_else(|| field.default.as_deref().and_then(|d| serde_json::from_str(d).ok()));
                value.map(|value| if let Value::Array(items) = &value { items.iter().map(to_input).collect::<Vec<_>>().join(", ") } else { to_input(&value) })
            });
            let hint = suggestions.iter().find(|(name, _)| *name == field.name).filter(|_| editable && !inherited && !vars.contains_key(&field.name)).map(|(_, value)| {
                let (name, value) = (field.name.clone(), value.clone());
                Button::new(SharedString::from(format!("provision-config-suggest-{}", field.name)))
                    .link()
                    .xsmall()
                    .label(cac_core::text_with(locale(cx), "common:serverConfig.useFromServer", &[("value", &to_input(&value))]))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let Some(mut vars) = this.vars_map(cx) else { return };
                        vars.insert(name.clone(), value.clone());
                        let text = stable_json(&Value::Object(vars));
                        this.vars.update(cx, |input, cx| input.replace_all(text, window, cx));
                    }))
            });
            let required = field.required && !vars.contains_key(&field.name) && !inherited;
            div()
                .id(SharedString::from(format!("provision-config-field-{}", field.name)))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(div().font_family(mono.clone()).text_xs().child(format!("{}{}", field.name, if required { " *" } else { "" })))
                .child(control)
                .children(heir.flatten().filter(|text| !text.is_empty()).map(|text| div().text_xs().text_color(muted).child(text)))
                .when(!field.description.is_empty(), |column| column.child(div().text_xs().text_color(muted).child(field.description.clone())))
                .children(hint)
                .test_support()
                .into_any_element()
        };
        let (own, shared): (Vec<_>, Vec<_>) = self.fields.iter().partition(|(f, _)| f.default.is_none());
        let own_rows: Vec<AnyElement> = own.iter().map(|(f, input)| field(f, input, false, cx)).collect();
        let shared_rows: Vec<AnyElement> = shared.iter().map(|(f, input)| field(f, input, true, cx)).collect();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .when(!own_rows.is_empty(), |column| {
                    column
                        .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:serverConfig.thisServer")))
                        .child(div().text_xs().text_color(muted).child(tr(cx, "common:serverConfig.thisServerLead")))
                        .children(own_rows)
                })
                .when(!shared_rows.is_empty(), |column| {
                    column
                        .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(cac_core::text_with(locale(cx), "common:serverConfig.inherited", &[("count", &shared_rows.len().to_string())])))
                        .child(div().text_xs().text_color(muted).child(tr(cx, "common:serverConfig.inheritedLead")))
                        .children(shared_rows)
                })
                .into_any_element(),
        )
    }

    fn secrets_section(&self, provision: &ProvisionState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger, warning, success, mono) = (theme.muted_foreground, theme.danger, theme.warning, theme.success, theme.mono_font_family.clone());
        let editable = provision.permissions.edit_config;
        let required = provision.templates.iter().find(|t| t.id == self.template_id).and_then(|t| t.current.as_ref()).map(|v| v.requires.clone()).unwrap_or_default();
        let rows = self.secrets.iter().enumerate().map(|(n, row)| {
            let name = row.name.read(cx).value().to_string();
            let reference = row.reference.read(cx).value().trim().to_string();
            let fixed = required.contains(&name);
            let check = self.checks.iter().find(|(r, _)| *r == reference).map(|(_, check)| match check {
                Ok(()) => div().text_color(success).child("✓"),
                Err(error) => div().text_color(danger).child(format!("✗ {error}")),
            });
            div()
                .id(SharedString::from(format!("provision-secret-{n}")))
                .flex()
                .items_center()
                .gap_1()
                .child(if fixed {
                    div().w(px(150.)).flex_none().font_family(mono.clone()).text_xs().truncate().child(name).into_any_element()
                } else {
                    div().id(SharedString::from(format!("provision-secret-name-{n}"))).w(px(150.)).flex_none().child(Input::new(&row.name).small().disabled(!editable)).test_support().into_any_element()
                })
                .child(div().id(SharedString::from(format!("provision-secret-ref-{n}"))).flex_1().min_w_0().child(Input::new(&row.reference).small().disabled(!editable)).test_support())
                .children(check.map(|check| check.text_xs()))
                .when(editable, |line| {
                    line.child(Button::new(SharedString::from(format!("provision-secret-remove-{n}"))).ghost().xsmall().icon(Icon::new(Lucide::Trash)).tooltip(tr(cx, "common:serverConfig.remove")).on_click(cx.listener(move |this, _, _, cx| {
                        if n < this.secrets.len() {
                            this.secrets.remove(n);
                        }
                        cx.notify();
                    })))
                })
                .test_support()
        });
        let missing = provision.effective.as_ref().filter(|e| !e.missing.is_empty()).map(|e| div().id("provision-config-missing").text_xs().text_color(warning).child(cac_core::text_with(locale(cx), "common:serverConfig.missing", &[("names", &e.missing.join(", "))])).test_support());
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:serverConfig.secrets")))
            .child(div().text_xs().text_color(muted).child(tr(cx, "common:serverConfig.secretsPanelLead")))
            .children(rows)
            .children(missing)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .when(editable, |row| {
                        row.child(Button::new("provision-secret-add").outline().xsmall().icon(Icon::new(Lucide::Plus)).label(tr(cx, "common:serverConfig.addSecret")).on_click(cx.listener(|this, _, window, cx| {
                            let secret = SecretRow { name: this.input("", "OPS_PASSWORD", window, cx), reference: this.input("", "op://vault/item/field", window, cx) };
                            this.secrets.push(secret);
                            cx.notify();
                        })))
                    })
                    .when(!self.secrets.is_empty(), |row| row.child(Button::new("provision-secret-check").outline().xsmall().label(tr(cx, "common:serverConfig.check")).on_click(cx.listener(|this, _, _, cx| this.check(cx))))),
            )
            .into_any_element()
    }

    fn versions(&self, provision: &ProvisionState, state: &PlatformState, cx: &mut Context<Self>) -> Option<AnyElement> {
        let versions = provision.config.as_ref().map(|c| c.versions.clone()).filter(|v| !v.is_empty())?;
        let theme = cx.theme();
        let (muted, border, success, danger, mono) = (theme.muted_foreground, theme.border, theme.success, theme.danger, theme.mono_font_family.clone());
        let editable = provision.permissions.edit_config;
        let name_of = |id: &str| state.servers.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_else(|| id.chars().take(8).collect());
        let rows = versions.iter().enumerate().map(|(n, version)| {
            let previous = versions.get(n + 1).map(ServerConfigVersion::as_text).unwrap_or_default();
            let open = self.open_version == Some(version.seq);
            let seq = version.seq;
            let note = if !version.note.is_empty() {
                version.note.clone()
            } else {
                version.based_on.as_ref().map(|origin| cac_core::text_with(locale(cx), "common:serverConfig.copiedNote", &[("from", &name_of(&origin.server_id))])).unwrap_or_default()
            };
            let who = [version.created_by_name.clone(), time_ago(&version.created_at, &chrono::Local::now(), locale(cx)), note].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · ");
            let diff = open.then(|| {
                let rows = line_diff(&previous, &version.as_text(), 3).into_iter().map(|row| match row {
                    DiffRow::Same { text } => div().whitespace_nowrap().child(format!("  {text}")),
                    DiffRow::Added { text } => div().whitespace_nowrap().text_color(success).child(format!("+ {text}")),
                    DiffRow::Removed { text } => div().whitespace_nowrap().text_color(danger).line_through().child(format!("- {text}")),
                    DiffRow::Skip { count } => div().text_color(muted).child(format!("⋯ {count}")),
                });
                div().id(SharedString::from(format!("provision-version-diff-{seq}"))).p_2().rounded(px(6.)).bg(theme.muted).font_family(mono.clone()).text_xs().children(rows.collect::<Vec<_>>()).test_support()
            });
            let restoring = version.clone();
            div()
                .id(SharedString::from(format!("provision-version-{seq}")))
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .py_1p5()
                .border_t_1()
                .border_color(border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(Button::new(SharedString::from(format!("provision-version-open-{seq}"))).link().xsmall().label(format!("v{seq}")).on_click(cx.listener(move |this, _, _, cx| {
                            this.open_version = if this.open_version == Some(seq) { None } else { Some(seq) };
                            cx.notify();
                        })))
                        .when(version.source == "mcp", |row| row.child(super::services::pill(tr(cx, "common:serverConfig.viaAgent"), border, muted)))
                        .child(div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(who))
                        .when(editable && n > 0, |row| {
                            row.child(Button::new(SharedString::from(format!("provision-version-restore-{seq}"))).ghost().xsmall().icon(Icon::new(Lucide::RotateCcw)).tooltip(tr(cx, "common:serverConfig.restore")).on_click(cx.listener(move |this, _, window, cx| this.restore(restoring.clone(), window, cx))))
                        }),
                )
                .children(diff)
                .test_support()
        });
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:serverConfig.versions")))
                .child(div().id("provision-versions").flex().flex_col().rounded(px(6.)).border_1().border_color(border).children(rows).test_support())
                .into_any_element(),
        )
    }
}

impl Render for ConfigPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let Some(provision) = state.provision.clone().filter(|p| p.server_id == self.server_id) else { return div().into_any_element() };
        let Some(server) = state.servers.iter().find(|s| s.id == self.server_id).cloned() else { return div().into_any_element() };
        let current_seq = provision.config.as_ref().and_then(|c| c.current.as_ref()).map(|v| v.seq).unwrap_or_default();
        if provision.config.is_some() && self.loaded_seq != Some(current_seq) {
            self.load(&provision, window, cx);
        }
        let manifest = provision.project.as_ref().filter(|p| p.dir == provision.project_dir).and_then(|p| p.manifest.clone());
        self.sync_fields(manifest.as_ref(), &provision, window, cx);
        let theme = cx.theme();
        let (muted, danger, border) = (theme.muted_foreground, theme.danger, theme.border);
        let editable = provision.permissions.edit_config;
        let draft = config_body(&self.template_id, &self.vars.read(cx).value(), &self.rows(cx), "");
        let current = provision.config.as_ref().and_then(|c| c.current.as_ref());
        let dirty = draft.as_ref().is_ok_and(|body| config_changed(body, current));
        let problem = draft.as_ref().err().map(|problem| div().id("provision-config-problem").text_xs().text_color(danger).child(tr(cx, problem.key())).test_support());
        let others: Vec<Server> = state.servers.iter().filter(|s| s.id != server.id && s.org_id == server.org_id).cloned().collect();
        let copy = (editable && !others.is_empty()).then(|| {
            let chips = others.iter().map(|other| {
                let id = other.id.clone();
                div()
                    .id(SharedString::from(format!("provision-copy-from-{}", other.id)))
                    .aria_label(other.name.clone())
                    .px_2()
                    .py_0p5()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(border)
                    .cursor_pointer()
                    .when(self.copy_from.as_deref() == Some(other.id.as_str()), |chip| chip.bg(theme.accent))
                    .child(other.name.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.copy_from = Some(id.clone());
                        cx.notify();
                    }))
                    .test_support()
            });
            let source = self.copy_from.as_ref().and_then(|id| others.iter().find(|s| &s.id == id)).cloned();
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().leading_xs().child(tr(cx, "common:serverConfig.basedOn")))
                .child(div().flex().flex_wrap().items_center().gap_1().leading_xs().children(chips).child(
                    Button::new("provision-copy").outline().xsmall().icon(Icon::new(Lucide::Copy)).label(tr(cx, "common:serverConfig.copy")).disabled(source.is_none() || self.busy).on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(source) = source.clone() {
                            this.copy(source, window, cx);
                        }
                    })),
                ))
        });
        let line = self.line.clone().map(|(phrase, failed)| div().id("provision-config-line").aria_label(phrase.clone()).text_xs().text_color(if failed { danger } else { muted }).child(phrase).test_support());
        let title = match current {
            Some(version) => format!("{} · v{}", tr(cx, "common:serverConfig.title"), version.seq),
            None => tr(cx, "common:serverConfig.title"),
        };
        let fields = self.fields_section(&server, &provision, cx);
        let template = self.template_row(&provision, manifest.as_ref(), cx);
        let secrets = self.secrets_section(&provision, cx);
        let versions = self.versions(&provision, &state, cx);
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(muted).child(tr(cx, "common:serverConfig.lead")))
            .child(template)
            .children(fields)
            .child(
                div()
                    .id("provision-config-json")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().leading_xs().child(tr(cx, "common:serverConfig.vars")))
                    .child(Textarea::new(&self.vars).disabled(!editable))
                    .child(div().text_xs().text_color(muted).child(tr(cx, "common:serverConfig.noSecretsInVars")))
                    .test_support(),
            )
            .child(secrets)
            .children(problem)
            .when(editable, |column| {
                column.child(div().id("provision-config-note").child(Input::new(&self.note).small()).test_support()).child(
                    Button::new("provision-config-save")
                        .primary()
                        .small()
                        .label(tr(cx, "common:serverConfig.saveVersion"))
                        .disabled(self.busy || !dirty)
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                )
            })
            .children(line)
            .children(copy)
            .children(versions)
            .children(self.dialog.as_ref().map(|(dialog, _)| dialog.clone()));
        card("provision-config", title, body.into_any_element(), cx)
    }
}
