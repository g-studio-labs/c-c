//! La pestaña Provisioning de un servidor (`ServerProvision.tsx`): aplicar
//! un playbook de Ansible desde esta máquina. La carpeta del proyecto
//! primero (el manifiesto, la plantilla y los playbooks se leen de ella);
//! luego el playbook, y con un manifiesto v2 se ensaya y se pide (el
//! protocolo, también en persona); con uno v1, se aplica a mano. A la
//! derecha la salida, la bandeja de peticiones y lo aplicado.
//!
//! El orden de las llamadas y lo que se apunta en cac son del core; aquí, las
//! preguntas antes de tocar nada, los campos y la pintura.

use cac_core::{provision_phrase, time_ago, ManualInputs, PlatformState, PlaybookInfo, ProvisionError, ProvisionRun, ProvisionState, RunEnd, Server};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::log_view::LogBox;
use crate::theme::Leading as _;

use super::provision_engine::engine;
use super::services::pill;

const FORM_WIDTH: f32 = 420.;

pub struct ProvisionView {
    project: Entity<InputState>,
    /// La contraseña de sudo (v1). Se vacía en cuanto se lanza: no se queda
    /// en pantalla más de lo necesario.
    become_password: Entity<InputState>,
    extra: Entity<TextareaState>,
    /// Un campo por variable del playbook elegido (v1), y de cuál son.
    vars: Vec<(String, Entity<InputState>)>,
    vars_of: Option<String>,
    /// La carpeta que ya se puso en el campo y se leyó.
    project_shown: Option<String>,
    /// Para qué playbook, versión de la config y carpeta se calculó lo que
    /// falta: si cambia cualquiera, se vuelve a calcular.
    gaps_for: Option<(String, i64, String)>,
    /// Cómo acabó lo último que se pidió: la frase y si falló.
    line: Option<(String, bool)>,
    /// La ejecución del historial cuya config aplicada se mira.
    open_run: Option<String>,
    logs: LogBox,
    _subscriptions: Vec<Subscription>,
}

impl ProvisionView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project = cx.new(|cx| InputState::new(window, cx).placeholder("~/projects/ansible/contabo"));
        let become_password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let extra = cx.new(|cx| TextareaState::new(window, cx).placeholder("nombre=valor").auto_grow(2, 6));
        let mut subscriptions = vec![cx.subscribe_in(&project, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.project_typed(window, cx);
            }
        })];
        for notify in [cx.subscribe(&become_password, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })] {
            subscriptions.push(notify);
        }
        subscriptions.push(cx.subscribe(&extra, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }));
        ProvisionView {
            project,
            become_password,
            extra,
            vars: Vec::new(),
            vars_of: None,
            project_shown: None,
            gaps_for: None,
            line: None,
            open_run: None,
            logs: LogBox::new(),
            _subscriptions: subscriptions,
        }
    }

    fn project_typed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.project.read(cx).value().trim().to_string();
        let current = Backend::snapshot(cx).read(cx).platform.provision.as_ref().map(|p| p.project_dir.clone());
        if current.as_deref() != Some(typed.as_str()) {
            self.use_project(typed, cx);
        }
    }

    fn use_project(&mut self, dir: String, cx: &mut Context<Self>) {
        self.project_shown = Some(dir.clone());
        self.gaps_for = None;
        let engine = engine(cx);
        Backend::run(cx, move |core| async move {
            core.platform().set_project_dir(&dir);
            core.platform().load_project(engine).await;
        });
    }

    /// Sin carpeta recordada, la de casa: sin ella, el diálogo abre donde
    /// esté el proceso.
    fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            let Some(dir) = paths.into_iter().next() else { return };
            let dir = dir.to_string_lossy().into_owned();
            this.update_in(cx, |this, window, cx| {
                this.project.update(cx, |input, cx| input.set_value(dir.clone(), window, cx));
                this.use_project(dir, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Los campos de las variables del playbook elegido, uno por variable.
    fn sync_vars(&mut self, playbook: Option<&PlaybookInfo>, window: &mut Window, cx: &mut Context<Self>) {
        let id = playbook.map(|p| p.id.clone());
        if self.vars_of == id {
            return;
        }
        self.vars_of = id;
        self.vars = playbook
            .map(|playbook| {
                playbook
                    .vars
                    .iter()
                    .filter(|var| var.from.is_none())
                    .map(|var| {
                        let secret = var.secret;
                        let input = cx.new(|cx| InputState::new(window, cx).masked(secret));
                        self._subscriptions.push(cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                            if matches!(event, InputEvent::Change) {
                                cx.notify();
                            }
                        }));
                        (var.name.clone(), input)
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    fn value_of(&self, name: &str, cx: &App) -> String {
        self.vars.iter().find(|(known, _)| known == name).map(|(_, input)| input.read(cx).value().to_string()).unwrap_or_default()
    }

    fn ask(&mut self, title: String, body: String, confirm: String, window: &mut Window, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        let cancel = tr(cx, "common:servers.cancel");
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |this, window, cx| then(this, window, cx)).ok();
            }
        })
        .detach();
    }

    /// Lo que se pide al core, con su fallo dicho debajo.
    fn launch(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(cac_core::Core) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ProvisionError>> + Send>> + Send + 'static,
        done_key: &'static str,
    ) {
        self.line = None;
        cx.notify();
        let answer = Backend::ask(cx, work);
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.line = Some(match result {
                    Ok(()) => (tr(cx, done_key), false),
                    Err(error) => (provision_phrase(locale(cx), &error, "common:misc.errSave"), true),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Manifiesto v2: ensayar es lo que se hace en persona. Por cada
    /// playbook se abre una petición y se corre su ensayo, que no cambia
    /// nada; lo aprueba un admin en la bandeja.
    fn rehearse(&mut self, server: &Server, provision: &ProvisionState, ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let manifest = provision.project.as_ref().and_then(|p| p.manifest.as_ref());
        let names: Vec<String> = ids.iter().map(|id| manifest.and_then(|m| m.playbook(id)).map(|p| p.name.clone()).unwrap_or_else(|| id.clone())).collect();
        let seq = provision.effective.as_ref().map(|e| e.config_seq).unwrap_or_default();
        let language = locale(cx);
        let title = cac_core::text_with(language, "common:provision.rehearseTitle", &[("playbook", &names.join(" → ")), ("server", &server.name)]);
        let body = cac_core::text_with(language, "common:provision.rehearseBody", &[("seq", &seq.to_string()), ("files", &ids.join(" → "))]);
        let engine = engine(cx);
        self.ask(title, body, tr(cx, "common:provision.rehearse"), window, cx, move |this, _, cx| {
            this.launch(cx, move |core| Box::pin(async move { core.platform().rehearse(engine, &ids).await }), "common:provision.rehearsed");
        });
    }

    /// Manifiesto v1: se aplica a mano, preguntando qué y contra qué.
    fn run_manual(&mut self, server: &Server, provision: &ProvisionState, playbook: &PlaybookInfo, window: &mut Window, cx: &mut Context<Self>) {
        let (extra, _) = cac_core::parse_extra_vars(&self.extra.read(cx).value());
        let names: Vec<String> = playbook.vars.iter().map(|v| v.name.clone()).chain(extra.into_iter().map(|(name, _)| name)).collect();
        let language = locale(cx);
        let target = if provision.limit.is_empty() { tr(cx, "common:provision.allHosts") } else { provision.limit.clone() };
        let title = cac_core::text_with(language, "common:provision.confirmTitle", &[("playbook", &playbook.name), ("server", &server.name)]);
        let vars = if names.is_empty() { "—".to_string() } else { names.join(", ") };
        let body = cac_core::text_with(language, "common:provision.confirmBody", &[("file", &playbook.file), ("target", &target), ("vars", &vars)]);
        let engine = engine(cx);
        self.ask(title, body, tr(cx, "common:provision.run"), window, cx, move |this, window, cx| {
            let inputs = ManualInputs {
                values: this.vars.iter().map(|(name, input)| (name.clone(), input.read(cx).value().to_string())).collect(),
                extra: this.extra.read(cx).value().to_string(),
                become_password: this.become_password.read(cx).value().to_string(),
            };
            this.become_password.update(cx, |input, cx| input.set_value("", window, cx));
            this.launch(cx, move |core| Box::pin(async move { core.platform().run_manual(engine, inputs).await }), "common:provision.done");
        });
    }

    fn project_card(&self, provision: &ProvisionState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let project = provision.project.as_ref().filter(|p| p.dir == provision.project_dir);
        let tools = project.and_then(|p| p.tools.as_ref());
        let problem = |id: &'static str, text: String| div().id(id).leading_xs().text_color(danger).child(text).test_support();
        card(
            "provision-project-card",
            tr(cx, "common:provision.project"),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(div().id("provision-project").flex_1().child(Input::new(&self.project).small()).test_support())
                        .child(
                            Button::new("provision-pick")
                                .outline()
                                .small()
                                .icon(Icon::new(Lucide::FolderOpen))
                                .tooltip(tr(cx, "common:provision.pick"))
                                .on_click(cx.listener(|this, _, window, cx| this.pick_folder(window, cx))),
                        ),
                )
                .children(tools.filter(|t| !t.platform_supported).map(|t| problem("provision-unsupported", t.reason.clone())))
                .children(tools.filter(|t| t.platform_supported && t.ansible_playbook.is_none()).map(|_| problem("provision-no-ansible", tr(cx, "common:provision.noAnsible"))))
                .children(project.and_then(|p| p.error.clone()).map(|error| problem("provision-manifest-error", error)))
                .children(
                    project
                        .and_then(|p| p.manifest.as_ref())
                        .filter(|m| m.discovered)
                        .map(|_| div().id("provision-no-manifest").leading_xs().text_color(muted).child(tr(cx, "common:provision.noManifest")).test_support()),
                )
                .into_any_element(),
            cx,
        )
    }

    fn playbook_card(&mut self, server: &Server, state: &PlatformState, provision: &ProvisionState, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let project = provision.project.as_ref().filter(|p| p.dir == provision.project_dir)?;
        let manifest = project.manifest.as_ref().filter(|m| !m.playbooks.is_empty())?;
        let playbook = provision.playbook()?.clone();
        self.sync_vars(if manifest.is_v2() { None } else { Some(&playbook) }, window, cx);
        let theme = cx.theme();
        let (muted, danger, border, accent, mono) = (theme.muted_foreground, theme.danger, theme.border, theme.accent, theme.mono_font_family.clone());
        let output = state.provision_output.as_ref();
        let busy = output.is_some_and(|out| out.running);
        let has_ansible = project.tools.as_ref().is_some_and(|t| t.ansible_playbook.is_some());
        let v2 = manifest.is_v2();

        let choices = manifest.playbooks.iter().map(|choice| {
            let id = choice.id.clone();
            div()
                .id(SharedString::from(format!("provision-playbook-{}", choice.id)))
                .aria_label(choice.name.clone())
                .px_2()
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .cursor_pointer()
                .when(choice.id == playbook.id, |row| row.bg(accent))
                .hover(|style| style.bg(accent.opacity(0.4)))
                .child(choice.name.clone())
                .on_click(cx.listener(move |this, _, _, cx| {
                    Backend::core(cx).platform().choose_playbook(&id);
                    this.gaps_for = None;
                    cx.notify();
                }))
                .test_support()
        });

        let mut body = div().flex().flex_col().gap_2().child(div().flex().flex_wrap().gap_1().leading_sm().children(choices));
        if !playbook.description.is_empty() {
            body = body.child(div().leading_xs().text_color(muted).child(playbook.description.clone()));
        }
        let can_run;
        if v2 {
            let effective = provision.effective.as_ref();
            let gaps = project.gaps.as_ref().filter(|_| project.gaps_of == playbook.id);
            let config_line = match effective.filter(|e| e.config_seq > 0) {
                Some(e) => {
                    let host = format!("{}@{}:{}", e.connection.ssh_user, e.connection.host, e.connection.ssh_port);
                    let text = cac_core::text_with(locale(cx), "common:provision.withConfig", &[("seq", &e.config_seq.to_string()), ("host", &host)]);
                    div().id("provision-with-config").aria_label(text.clone()).child(text)
                }
                None => {
                    let text = tr(cx, "common:provision.noConfig");
                    div().id("provision-no-config").aria_label(text.clone()).child(text)
                }
            };
            body = body.child(config_line.leading_xs().text_color(muted).test_support());
            if let Some(gaps) = gaps {
                if !gaps.vars.is_empty() {
                    body = body.child(div().id("provision-missing-vars").leading_xs().text_color(danger).child(cac_core::text_with(locale(cx), "common:provision.missingVars", &[("names", &gaps.vars.join(", "))])).test_support());
                }
                if !gaps.secrets.is_empty() {
                    body = body.child(div().id("provision-missing-secrets").leading_xs().text_color(danger).child(cac_core::text_with(locale(cx), "common:provision.missingSecrets", &[("names", &gaps.secrets.join(", "))])).test_support());
                }
            }
            can_run = has_ansible && !busy && effective.is_some_and(|e| e.config_seq > 0) && gaps.is_some_and(|g| g.is_empty());
        } else {
            body = body.child(self.manual_fields(&playbook, project, provision, cx));
            let (_, bad) = cac_core::parse_extra_vars(&self.extra.read(cx).value());
            let missing = playbook.vars.iter().any(|v| v.required && v.from.is_none() && self.value_of(&v.name, cx).trim().is_empty());
            let needs_password = playbook.become_sudo && playbook.become_password_ref.is_none();
            let password = !self.become_password.read(cx).value().is_empty();
            can_run = has_ansible && !busy && !missing && bad.is_empty() && (!needs_password || password);
        }

        let permitted = provision.permissions.run;
        let label = if busy { tr(cx, "common:provision.running") } else { tr(cx, if v2 { "common:provision.rehearse" } else { "common:provision.run" }) };
        let (run_server, run_provision, run_playbook) = (server.clone(), provision.clone(), playbook.clone());
        let template = provision.effective.as_ref().filter(|e| v2 && e.playbooks.len() > 1).map(|e| {
            let ids = e.playbooks.clone();
            let ready = has_ansible && !busy && e.missing.is_empty();
            let (server, provision) = (server.clone(), provision.clone());
            Button::new("provision-rehearse-template")
                .outline()
                .small()
                .icon(Icon::new(Lucide::ListOrdered))
                .label(tr(cx, "common:provision.rehearseTemplate"))
                .tooltip(ids.join(" → "))
                .disabled(!ready)
                .on_click(cx.listener(move |this, _, window, cx| this.rehearse(&server, &provision, ids.clone(), window, cx)))
        });
        let running_here = output.is_some_and(|out| out.running && out.server_id == server.id);
        let buttons = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .when(permitted, |row| {
                row.child(
                    Button::new("provision-run")
                        .primary()
                        .small()
                        .icon(Icon::new(Lucide::Play))
                        .label(label)
                        .disabled(!can_run)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if v2 {
                                this.rehearse(&run_server, &run_provision, vec![run_playbook.id.clone()], window, cx);
                            } else {
                                this.run_manual(&run_server, &run_provision, &run_playbook, window, cx);
                            }
                        })),
                )
                .children(template)
            })
            .when(running_here, |row| {
                row.child(Button::new("provision-cancel").danger().small().icon(Icon::new(Lucide::Square)).label(tr(cx, "common:provision.cancel")).on_click(|_, _, cx| {
                    let engine = engine(cx);
                    Backend::run(cx, move |core| async move { core.platform().cancel_run(engine).await });
                }))
            });
        let _ = mono;
        body = body.child(buttons).child(div().leading_xs().text_color(muted).child(tr(cx, "common:provision.redactionNote")));
        Some(card("provision-playbook-card", tr(cx, "common:provision.playbook"), body.into_any_element(), cx))
    }

    /// v1: contra qué, las variables, la contraseña de sudo y las extra.
    fn manual_fields(&self, playbook: &PlaybookInfo, project: &cac_core::ProjectInfo, provision: &ProvisionState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger, border, accent, mono) = (theme.muted_foreground, theme.danger, theme.border, theme.accent, theme.mono_font_family.clone());
        let mut groups: Vec<String> = project.hosts.iter().flat_map(|h| h.groups.clone()).collect();
        groups.sort();
        groups.dedup();
        let all = format!("{}{}", tr(cx, "common:provision.allHosts"), playbook.hosts.as_ref().map(|h| format!(" ({h})")).unwrap_or_default());
        let mut targets = vec![(String::new(), all)];
        targets.extend(groups.into_iter().map(|g| (g.clone(), cac_core::text_with(locale(cx), "common:provision.group", &[("name", &g)]))));
        targets.extend(project.hosts.iter().map(|h| (h.name.clone(), format!("{}{}", h.name, h.ansible_host.as_ref().map(|a| format!(" → {a}")).unwrap_or_default()))));
        let target_chips = targets.into_iter().map(|(value, label)| {
            let chosen = value == provision.limit;
            let id = if value.is_empty() { "provision-target-all".to_string() } else { format!("provision-target-{value}") };
            div()
                .id(SharedString::from(id))
                .aria_label(label.clone())
                .px_2()
                .py_0p5()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .cursor_pointer()
                .when(chosen, |chip| chip.bg(accent))
                .child(label)
                .on_click(move |_, _, cx| Backend::core(cx).platform().set_limit(&value))
                .test_support()
        });
        let fields = playbook.vars.iter().map(|var| {
            let label = format!("{}{}", var.name, if var.required && var.from.is_none() { " *" } else { "" });
            let control = match (&var.from, self.vars.iter().find(|(name, _)| name == &var.name)) {
                (Some(reference), _) => div().px_2().py_1().rounded(px(6.)).border_1().border_color(border).font_family(mono.clone()).text_xs().text_color(muted).truncate().child(cac_core::text_with(locale(cx), "common:provision.fromOp", &[("ref", reference)])).into_any_element(),
                (None, Some((_, input))) => Input::new(input).small().into_any_element(),
                (None, None) => div().into_any_element(),
            };
            div()
                .id(SharedString::from(format!("provision-var-{}", var.name)))
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_family(mono.clone()).text_xs().child(label))
                .child(control)
                .when(!var.description.is_empty(), |field| field.child(div().leading_xs().text_color(muted).child(var.description.clone())))
                .test_support()
        });
        let needs_password = playbook.become_sudo && playbook.become_password_ref.is_none();
        let (_, bad) = cac_core::parse_extra_vars(&self.extra.read(cx).value());
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().leading_xs().child(tr(cx, "common:provision.target")))
            .child(div().flex().flex_wrap().gap_1().leading_xs().children(target_chips))
            .children(fields)
            .when(needs_password, |column| {
                column.child(
                    div()
                        .id("provision-become")
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().leading_xs().child(tr(cx, "common:provision.becomePassword")))
                        .child(Input::new(&self.become_password).small())
                        .test_support(),
                )
            })
            .when_some(playbook.become_password_ref.clone().filter(|_| playbook.become_sudo), |column, reference| {
                column.child(div().leading_xs().text_color(muted).child(cac_core::text_with(locale(cx), "common:provision.becomeFromOp", &[("ref", &reference)])))
            })
            .child(
                div()
                    .id("provision-extra")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().leading_xs().child(tr(cx, "common:provision.extraVars")))
                    .child(Textarea::new(&self.extra))
                    .test_support(),
            )
            .when(!bad.is_empty(), |column| {
                column.child(div().id("provision-bad-vars").leading_xs().text_color(danger).child(cac_core::text_with(locale(cx), "common:provision.badVars", &[("lines", &bad.join(", "))])).test_support())
            })
            .into_any_element()
    }

    fn output(&self, server: &Server, state: &PlatformState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (danger, mono) = (theme.danger, theme.mono_font_family.clone());
        let output = state.provision_output.as_ref().filter(|out| out.server_id == server.id);
        let lines = output.map(|out| out.lines.clone()).unwrap_or_default();
        let ended = output.and_then(|out| out.ended).map(|end| {
            let (key, color) = match end {
                RunEnd::Succeeded => ("common:provision.status.succeeded", theme.success),
                RunEnd::Failed => ("common:provision.status.failed", danger),
                RunEnd::Cancelled => ("common:provision.status.cancelled", theme.muted_foreground),
            };
            let text = tr(cx, key);
            div().id("provision-output-status").aria_label(text.clone()).child(pill(text, color.opacity(0.15), color)).test_support()
        });
        let error = output.and_then(|out| out.error.clone()).map(|error| div().id("provision-output-error").leading_xs().text_color(danger).child(error).test_support());
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().flex().items_center().gap_2().child(div().font_weight(FontWeight::MEDIUM).leading_sm().child(tr(cx, "common:provision.output"))).children(ended))
            .children(error)
            .child(self.logs.render("provision-output", &lines, tr(cx, "common:provision.noOutput"), mono))
            .into_any_element()
    }

    fn history(&self, provision: &ProvisionState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, mono) = (theme.muted_foreground, theme.border, theme.mono_font_family.clone());
        let title = div().font_weight(FontWeight::MEDIUM).leading_sm().child(tr(cx, "common:provision.history"));
        if provision.runs.is_empty() {
            return div().flex().flex_col().gap_1().child(title).child(div().id("provision-no-history").leading_sm().text_color(muted).child(tr(cx, "common:provision.noHistory")).test_support()).into_any_element();
        }
        let rows = provision.runs.iter().map(|run| self.history_row(run, cx));
        let _ = mono;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(title)
            .child(div().id("provision-history").flex().flex_col().rounded(px(8.)).border_1().border_color(border).leading_sm().children(rows).test_support())
            .into_any_element()
    }

    fn history_row(&self, run: &ProvisionRun, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, mono) = (theme.muted_foreground, theme.border, theme.mono_font_family.clone());
        let (bg, fg) = match run.status.as_str() {
            "succeeded" => (theme.success.opacity(0.15), theme.success),
            "failed" => (theme.danger.opacity(0.15), theme.danger),
            "running" => (theme.secondary, theme.secondary_foreground),
            _ => (theme.border, theme.muted_foreground),
        };
        let status = tr(cx, &run.status_key());
        let open = self.open_run.as_deref() == Some(run.id.as_str());
        let who = [run.started_by_name.clone(), time_ago(&run.started_at, &chrono::Local::now(), locale(cx))].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
        let config = (run.config_seq > 0).then(|| {
            let id = run.id.clone();
            Button::new(SharedString::from(format!("provision-run-config-{}", run.id)))
                .link()
                .xsmall()
                .label(cac_core::text_with(locale(cx), "common:provision.configVersion", &[("seq", &run.config_seq.to_string())]))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_run = if this.open_run.as_deref() == Some(id.as_str()) { None } else { Some(id.clone()) };
                    cx.notify();
                }))
        });
        let applied = run.applied_text().filter(|_| open).map(|text| {
            div()
                .id(SharedString::from(format!("provision-applied-{}", run.id)))
                .aria_label(text.clone())
                .p_2()
                .rounded(px(6.))
                .bg(theme.muted)
                .font_family(mono.clone())
                .text_xs()
                .children(text.lines().map(|line| div().whitespace_nowrap().child(line.to_string())).collect::<Vec<_>>())
                .test_support()
        });
        div()
            .id(SharedString::from(format!("provision-run-{}", run.id)))
            .aria_label(status.clone())
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(pill(status, bg, fg))
                    .child(div().font_family(mono.clone()).text_xs().child(run.playbook.clone()))
                    .when(!run.target.is_empty(), |row| row.child(div().text_xs().text_color(muted).child(format!("→ {}", run.target))))
                    .children(config)
                    .when(run.source == "cli", |row| row.child(pill(tr(cx, "common:provision.fromTerminal"), theme.border, muted)))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(muted).child(who)),
            )
            .when(!run.summary.is_empty(), |column| column.child(div().font_family(mono.clone()).text_xs().text_color(muted).children(run.summary.lines().map(|line| div().whitespace_nowrap().child(line.to_string())).collect::<Vec<_>>())))
            .children(applied)
            .test_support()
            .into_any_element()
    }
}

/// Una tarjeta con título, como las de la app de Tauri.
pub(crate) fn card(id: &'static str, title: String, body: AnyElement, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded(px(10.))
        .border_1()
        .border_color(theme.border)
        .child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(title))
        .child(body)
        .test_support()
        .into_any_element()
}

impl Render for ProvisionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let Some(server) = state.open_server.as_ref().and_then(|id| state.servers.iter().find(|server| &server.id == id)).cloned() else {
            return div().into_any_element();
        };
        let Some(provision) = state.provision.clone().filter(|p| p.server_id == server.id) else {
            return div().id("provision-loading").leading_sm().text_color(muted).child(tr(cx, "common:servers.loading")).test_support().into_any_element();
        };
        // La carpeta recordada de esta máquina: al entrar se pone en el campo
        // y se lee.
        if self.project_shown.is_none() {
            self.project_shown = Some(provision.project_dir.clone());
            let dir = provision.project_dir.clone();
            self.project.update(cx, |input, cx| input.set_value(dir.clone(), window, cx));
            if !dir.is_empty() && provision.project.is_none() {
                let engine = engine(cx);
                Backend::run(cx, move |core| async move { core.platform().load_project(engine).await });
            }
        }
        let wanted = provision.playbook().map(|p| (p.id.clone(), provision.effective.as_ref().map(|e| e.config_seq).unwrap_or_default(), provision.project_dir.clone()));
        let v2 = provision.project.as_ref().and_then(|p| p.manifest.as_ref()).is_some_and(|m| m.is_v2());
        if v2 && provision.effective.is_some() && wanted.is_some() && self.gaps_for != wanted {
            self.gaps_for = wanted;
            let engine = engine(cx);
            Backend::run(cx, move |core| async move { core.platform().refresh_gaps(engine).await });
        }
        let line = self.line.clone().map(|(phrase, failed)| div().id("provision-line").aria_label(phrase.clone()).leading_xs().text_color(if failed { danger } else { muted }).child(phrase).test_support());
        let error = provision.error.clone().map(|error| div().id("provision-error").leading_xs().text_color(danger).child(error).test_support());
        let project = self.project_card(&provision, cx);
        let playbook = self.playbook_card(&server, &state, &provision, window, cx);
        let left = div().w(px(FORM_WIDTH)).flex_none().flex().flex_col().gap_3().child(project).children(playbook).children(line);
        let right = div().flex_1().min_w_0().flex().flex_col().gap_4().child(self.output(&server, &state, cx)).child(self.history(&provision, cx));
        div().id("server-provision").flex().flex_col().gap_3().children(error).child(div().flex().items_start().gap_4().child(left).child(right)).test_support().into_any_element()
    }
}
