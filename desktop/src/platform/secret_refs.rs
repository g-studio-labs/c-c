//! Los secrets de un servicio que despliega cac, por referencia a 1Password
//! (`ServiceSecretRefs.tsx`), en su página de despliegue. Aquí sólo se dice de
//! dónde sale cada uno; rotarlos los lee en esta máquina y los lleva al
//! servidor por ssh (`cac_ops::secrets`). A cac y a la pantalla sólo llegan
//! los nombres.

use std::sync::{Arc, Mutex};

use cac_core::{append_rows, label_to_name, merge_build_args, ref_problems, server_edit_phrase, Deployable, RefProblem, SecretRefRow, SecretUsage, Server, CiNotify, SECRETS_ENTRYPOINT_SNIPPET};
use cac_ops::secrets::{SecretRef, SshTarget};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Lo que se rotó: por secret, su nombre, el secret de Docker que le tocó y
/// si cambió; y si el servicio se reinició.
pub struct Rotated {
    pub items: Vec<(String, String, bool)>,
    pub updated: bool,
}

type Progress = Box<dyn Fn(&'static str, usize, usize) + Send>;

/// Lo que toca 1Password y el servidor desde esta máquina. Aparte para que
/// las pruebas no lancen `op` ni `ssh` de verdad. Todo bloquea: se llama en
/// un hilo aparte.
pub trait OnePassword: Send + Sync {
    fn check(&self, op_ref: &str) -> Result<(), String>;
    fn item_fields(&self, item: &str) -> Result<Vec<(String, String)>, String>;
    fn rotate(&self, target: &SshTarget, service: &str, refs: &[SecretRef], command: Option<&str>, progress: Progress) -> Result<Rotated, String>;
    fn capture(&self, target: &SshTarget, service: &str, vault: &str, title: &str) -> Result<(String, Vec<String>), String>;
}

struct SystemOnePassword;

impl OnePassword for SystemOnePassword {
    fn check(&self, op_ref: &str) -> Result<(), String> {
        let check = cac_ops::secrets::check_op_refs(vec![op_ref.to_string()], &Default::default()).into_iter().next();
        match check {
            Some(check) if check.ok => Ok(()),
            Some(check) => Err(check.error),
            None => Err(String::new()),
        }
    }
    fn item_fields(&self, item: &str) -> Result<Vec<(String, String)>, String> {
        Ok(cac_ops::secrets::op_item_fields(item)?.into_iter().map(|field| (field.label, field.op_ref)).collect())
    }
    fn rotate(&self, target: &SshTarget, service: &str, refs: &[SecretRef], command: Option<&str>, progress: Progress) -> Result<Rotated, String> {
        let result = cac_ops::secrets::rotate_service_secrets(target, service, refs, &Default::default(), command, move |step| progress(step.step, step.done, step.total))?;
        Ok(Rotated { items: result.items.into_iter().map(|item| (item.name, item.secret, item.changed)).collect(), updated: result.updated })
    }
    fn capture(&self, target: &SshTarget, service: &str, vault: &str, title: &str) -> Result<(String, Vec<String>), String> {
        let result = cac_ops::secrets::capture_service_env(target, service, vault, title)?;
        Ok((result.item, result.names))
    }
}

/// Los de esta app: los del sistema, salvo que alguien ponga otros.
pub struct LocalOnePassword(pub Arc<dyn OnePassword>);

impl Global for LocalOnePassword {}

fn one_password(cx: &App) -> Arc<dyn OnePassword> {
    cx.try_global::<LocalOnePassword>().map(|op| op.0.clone()).unwrap_or_else(|| Arc::new(SystemOnePassword))
}

struct Row {
    name: Entity<InputState>,
    op_ref: Entity<InputState>,
    usage: SecretUsage,
}

#[derive(Clone, Copy, PartialEq)]
enum Busy {
    Save,
    Check,
    Import,
    Args,
    Rotate,
    Capture,
}

pub struct SecretRefsView {
    deployable_id: String,
    rows: Vec<Row>,
    /// Lo guardado cuando se llenaron las filas: si llega otro, se vuelven a
    /// llenar (salvo que haya cambios sin guardar).
    loaded: Option<Vec<SecretRefRow>>,
    item: Entity<InputState>,
    vault: Entity<InputState>,
    title: Entity<InputState>,
    rotate_command: Entity<InputState>,
    /// Lo que dijo «Probar» de cada referencia: `None` es que se lee.
    checks: Vec<(String, Option<String>)>,
    checked: Option<(usize, usize)>,
    progress: Arc<Mutex<Option<(&'static str, usize, usize)>>>,
    rotated: Option<Rotated>,
    busy: Option<Busy>,
    line: Option<(String, bool)>,
    bridge_open: bool,
    _changed: Vec<Subscription>,
}

impl SecretRefsView {
    pub fn new(deployable: &Deployable, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &str, value: &str, window: &mut Window, cx: &mut Context<Self>| {
            let (placeholder, value) = (placeholder.to_string(), value.to_string());
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
        };
        let item = input("op://Prod/api", "", window, cx);
        let vault = input("Prod", "", window, cx);
        let title = input("", "", window, cx);
        let rotate_command = input("sudo -n /usr/local/bin/rotate-secrets prod", &deployable.rotate_command, window, cx);
        let changed = [&item, &rotate_command].into_iter().map(|input| cx.subscribe(input, |_, _, event: &InputEvent, cx| if matches!(event, InputEvent::Change) { cx.notify() })).collect();
        Backend::run(cx, {
            let id = deployable.id.clone();
            move |core| async move { core.platform().load_secret_refs(&id).await }
        });
        SecretRefsView {
            deployable_id: deployable.id.clone(),
            rows: Vec::new(),
            loaded: None,
            item,
            vault,
            title,
            rotate_command,
            checks: Vec::new(),
            checked: None,
            progress: Arc::new(Mutex::new(None)),
            rotated: None,
            busy: None,
            line: None,
            bridge_open: false,
            _changed: changed,
        }
    }

    pub fn deployable_id(&self) -> &str {
        &self.deployable_id
    }

    fn current(&self, cx: &App) -> Vec<SecretRefRow> {
        self.rows.iter().map(|row| SecretRefRow { name: row.name.read(cx).value().trim().to_string(), op_ref: row.op_ref.read(cx).value().trim().to_string(), usage: row.usage }).collect()
    }

    fn fill(&mut self, rows: &[SecretRefRow], window: &mut Window, cx: &mut Context<Self>) {
        self.rows = rows
            .iter()
            .map(|row| {
                let (name, op_ref) = (row.name.clone(), row.op_ref.clone());
                let name = cx.new(|cx| InputState::new(window, cx).default_value(name));
                let op_ref = cx.new(|cx| InputState::new(window, cx).placeholder("op://bóveda/ítem/campo").default_value(op_ref));
                self._changed.push(cx.subscribe(&name, |_, _, event: &InputEvent, cx| if matches!(event, InputEvent::Change) { cx.notify() }));
                self._changed.push(cx.subscribe(&op_ref, |_, _, event: &InputEvent, cx| if matches!(event, InputEvent::Change) { cx.notify() }));
                Row { name, op_ref, usage: row.usage }
            })
            .collect();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let (id, rows) = (self.deployable_id.clone(), self.current(cx));
        self.busy = Some(Busy::Save);
        self.line = None;
        cx.notify();
        let done = Backend::ask(cx, move |core| async move { core.platform().save_secret_refs(&id, &rows).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = done.await else { return };
            this.update(cx, |this, cx| {
                this.busy = None;
                this.loaded = None;
                if let Err(error) = result {
                    this.line = Some((server_edit_phrase(locale(cx), &error, "common:secrets.saveFailed"), true));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// De una en una, para que cada fila diga lo suyo en cuanto lo sabe y el
    /// botón cuente por dónde va (jose, 23-oct-2026).
    fn check(&mut self, cx: &mut Context<Self>) {
        let mut refs: Vec<String> = self.current(cx).into_iter().map(|row| row.op_ref).filter(|op_ref| !op_ref.is_empty()).collect();
        refs.dedup();
        self.checks.clear();
        self.checked = Some((0, refs.len()));
        self.busy = Some(Busy::Check);
        cx.notify();
        let op = one_password(cx);
        cx.spawn(async move |this, cx| {
            for (n, op_ref) in refs.iter().enumerate() {
                let (op, target) = (op.clone(), op_ref.clone());
                let verdict = cx.update(|cx| Backend::ask(cx, move |_| async move { tokio::task::spawn_blocking(move || op.check(&target)).await.unwrap_or_else(|error| Err(error.to_string())) }));
                let Ok(verdict) = verdict.await else { return };
                let reference = op_ref.clone();
                if this
                    .update(cx, |this, cx| {
                        this.checks.push((reference, verdict.err()));
                        this.checked = Some((n + 1, refs.len()));
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            this.update(cx, |this, cx| {
                this.busy = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn import_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let item = self.item.read(cx).value().trim().to_string();
        let op = one_password(cx);
        self.busy = Some(Busy::Import);
        cx.notify();
        let answer = Backend::ask(cx, move |_| async move { tokio::task::spawn_blocking(move || op.item_fields(&item)).await.unwrap_or_else(|error| Err(error.to_string())) });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.busy = None;
                    match result {
                        Ok(fields) => {
                            let more = fields.into_iter().map(|(label, op_ref)| SecretRefRow { name: label_to_name(&label), op_ref, usage: SecretUsage::Runtime });
                            let next = append_rows(&this.current(cx), more);
                            this.fill(&next, window, cx);
                            this.item.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        Err(error) => this.line = Some((error, true)),
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn read_build_args(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.deployable_id.clone();
        self.busy = Some(Busy::Args);
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move { core.platform().build_args(&id).await });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.busy = None;
                    match result {
                        Ok(args) => {
                            let (next, added, promoted) = merge_build_args(&this.current(cx), &args);
                            this.fill(&next, window, cx);
                            let phrase = cac_core::text_with(locale(cx), "common:secrets.argsMerged", &[("count", &args.len().to_string()), ("added", &added.to_string()), ("promoted", &promoted.to_string())]);
                            this.line = Some((phrase, false));
                        }
                        Err(error) => this.line = Some((server_edit_phrase(locale(cx), &error, "common:secrets.argsFailed"), true)),
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn target(server: &Server) -> SshTarget {
        SshTarget { server_id: server.id.clone(), host: server.host.clone(), ssh_port: server.ssh_port as u16, ssh_user: server.ssh_user.clone() }
    }

    /// Se lee de 1Password en esta máquina y se lleva al servidor, que
    /// reinicia el servicio: se pregunta nombrando lo que se lleva.
    fn rotate(&mut self, deployable: &Deployable, server: &Server, saved: Vec<SecretRefRow>, window: &mut Window, cx: &mut Context<Self>) {
        let names = saved.iter().map(|row| row.name.clone()).collect::<Vec<_>>();
        let title = cac_core::text_with(locale(cx), "common:secrets.rotateTitle", &[("service", &deployable.service_name)]);
        let body = cac_core::text_with(locale(cx), "common:secrets.rotateBody", &[("names", &names.join(", ")), ("server", &server.name)]);
        let (confirm, cancel) = (tr(cx, "common:secrets.rotate"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        let (target, service, command) = (Self::target(server), deployable.service_name.clone(), Some(deployable.rotate_command.trim().to_string()).filter(|command| !command.is_empty()));
        let (op, progress, id) = (one_password(cx), self.progress.clone(), self.deployable_id.clone());
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let refs: Vec<SecretRef> = saved.iter().map(|row| SecretRef { name: row.name.clone(), op_ref: row.op_ref.clone(), usage: Some(match row.usage { SecretUsage::Runtime => "runtime", SecretUsage::Build => "build", SecretUsage::Both => "both" }.to_string()) }).collect();
            *progress.lock().unwrap() = Some(("read", 0, refs.len()));
            let _ = this.update(cx, |this, cx| {
                (this.busy, this.rotated, this.line) = (Some(Busy::Rotate), None, None);
                cx.notify();
                // Mientras tanto, el progreso que va contando.
                cx.spawn(async move |this, cx| loop {
                    cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
                    let rotating = this.update(cx, |this, cx| {
                        cx.notify();
                        this.busy == Some(Busy::Rotate)
                    });
                    if !matches!(rotating, Ok(true)) {
                        return;
                    }
                })
                .detach();
            });
            let shared = progress.clone();
            let done = cx.update(|cx| {
                Backend::ask(cx, move |core| async move {
                    let names: Vec<String> = refs.iter().map(|row| row.name.clone()).collect();
                    let result = tokio::task::spawn_blocking(move || op.rotate(&target, &service, &refs, command.as_deref(), Box::new(move |step, done, total| *shared.lock().unwrap() = Some((step, done, total)))))
                        .await
                        .unwrap_or_else(|error| Err(error.to_string()));
                    // Lo que se apunta en cac son nombres y versiones; si no se
                    // pudo apuntar, la rotación ya está hecha y no se dice.
                    match &result {
                        Ok(rotated) => {
                            let versions: Vec<(String, String)> = rotated.items.iter().map(|(name, secret, _)| (name.clone(), secret.clone())).collect();
                            let _ = core.platform().record_rotation(&id, &names, &versions, None).await;
                        }
                        Err(error) => {
                            let _ = core.platform().record_rotation(&id, &names, &[], Some(error)).await;
                        }
                    }
                    result
                })
            });
            let Ok(result) = done.await else { return };
            *progress.lock().unwrap() = None;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(rotated) => {
                        this.line = Some((tr(cx, if rotated.updated { "common:secrets.rotated" } else { "common:secrets.unchanged" }), false));
                        this.rotated = Some(rotated);
                    }
                    Err(error) => this.line = Some((format!("{}: {error}", tr(cx, "common:secrets.rotateFailed")), true)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// La primera vez: lo que el CI dejó como variables de entorno del
    /// servicio, a un ítem nuevo de 1Password. Sólo vuelven los nombres.
    fn capture(&mut self, deployable: &Deployable, server: &Server, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.current(cx);
        let vault = Some(self.vault.read(cx).value().trim().to_string()).filter(|vault| !vault.is_empty()).or_else(|| rows.iter().find_map(|row| row.op_ref.strip_prefix("op://").and_then(|rest| rest.split('/').next()).map(str::to_string))).unwrap_or_default();
        let guessed = [deployable.repo_full_name.split('/').nth(1).unwrap_or(&deployable.service_name).to_string(), deployable.environment.clone()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ");
        let title = Some(self.title.read(cx).value().trim().to_string()).filter(|title| !title.is_empty()).unwrap_or(guessed);
        let heading = cac_core::text_with(locale(cx), "common:secrets.captureTitle", &[("service", &deployable.service_name)]);
        let body = cac_core::text_with(locale(cx), "common:secrets.captureBody", &[("service", &deployable.service_name), ("server", &server.name), ("item", &format!("op://{vault}/{title}"))]);
        let (confirm, cancel) = (tr(cx, "common:secrets.capture"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &heading, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        let (target, service, op) = (Self::target(server), deployable.service_name.clone(), one_password(cx));
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.busy = Some(Busy::Capture);
                cx.notify();
            });
            let done = cx.update(|cx| Backend::ask(cx, move |_| async move { tokio::task::spawn_blocking(move || op.capture(&target, &service, &vault, &title)).await.unwrap_or_else(|error| Err(error.to_string())) }));
            let Ok(result) = done.await else { return };
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.busy = None;
                    match result {
                        Ok((item, names)) => {
                            let count = names.len();
                            let more = names.into_iter().map(|name| SecretRefRow { op_ref: format!("{item}/{name}"), name, usage: SecretUsage::Runtime });
                            let next = append_rows(&this.current(cx), more);
                            this.fill(&next, window, cx);
                            this.line = Some((cac_core::text_with(locale(cx), "common:secrets.captured", &[("count", &count.to_string()), ("item", &item)]), false));
                        }
                        Err(error) => this.line = Some((format!("{}: {error}", tr(cx, "common:secrets.captureFailed")), true)),
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn problem_phrase(problem: &RefProblem, cx: &App) -> String {
        let named = |name: &str| if name.is_empty() { tr(cx, "common:secrets.unnamed") } else { name.to_string() };
        match problem {
            RefProblem::Name(name) => cac_core::text_with(locale(cx), "common:secrets.problemName", &[("name", &named(name))]),
            RefProblem::Repeated(name) => cac_core::text_with(locale(cx), "common:secrets.problemRepeated", &[("name", &named(name))]),
            RefProblem::Ref(name) => cac_core::text_with(locale(cx), "common:secrets.problemRef", &[("name", &named(name))]),
        }
    }

    fn render_for(&mut self, deployable: &Deployable, server: &Server, state: &cac_core::PlatformState, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let secrets = state.secrets.as_ref().filter(|secrets| secrets.deployable_id == deployable.id);
        let saved = secrets.and_then(|secrets| secrets.refs.clone());
        let current = self.current(cx);
        let dirty = self.loaded.as_ref().is_some_and(|loaded| *loaded != current);
        if let Some(saved) = saved.as_ref().filter(|saved| self.loaded.as_ref() != Some(saved) && !dirty) {
            let saved = saved.clone();
            self.fill(&saved, window, cx);
            self.loaded = Some(saved);
        }
        let current = self.current(cx);
        let dirty = self.loaded.as_ref().is_some_and(|loaded| *loaded != current);
        let (can_edit, can_rotate) = (state.permissions.manage_deploys, state.permissions.deploy);
        let theme = cx.theme();
        let (border, muted, danger, success, warning, mono) = (theme.border, theme.muted_foreground, theme.danger, theme.success, theme.warning, theme.mono_font_family.clone());
        let problems = ref_problems(&current);
        let check_of = |op_ref: &str| self.checks.iter().find(|(known, _)| known == op_ref).map(|(_, verdict)| verdict.clone());
        let rows: Vec<AnyElement> = self
            .rows
            .iter()
            .enumerate()
            .map(|(n, row)| {
                let op_ref = row.op_ref.read(cx).value().trim().to_string();
                let mark = check_of(&op_ref).map(|verdict| match verdict {
                    None => Icon::new(Lucide::Check).size_4().text_color(success).into_any_element(),
                    Some(_) => Icon::new(Lucide::X).size_4().text_color(danger).into_any_element(),
                });
                let usage = tr(cx, row.usage.label_key());
                div()
                    .id(SharedString::from(format!("secret-row-{n}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().id(SharedString::from(format!("secret-name-{n}"))).w(px(180.)).flex_none().child(Input::new(&row.name).disabled(!can_edit)).test_support())
                    .child(div().id(SharedString::from(format!("secret-ref-{n}"))).flex_1().min_w_0().child(Input::new(&row.op_ref).disabled(!can_edit)).test_support())
                    .child(
                        Button::new(SharedString::from(format!("secret-usage-{n}")))
                            .outline()
                            .xsmall()
                            .label(usage)
                            .disabled(!can_edit)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(row) = this.rows.get_mut(n) {
                                    row.usage = row.usage.next();
                                }
                                cx.notify();
                            })),
                    )
                    .children(mark)
                    .when(can_edit, |this| {
                        this.child(Button::new(SharedString::from(format!("secret-remove-{n}"))).ghost().xsmall().icon(Icon::new(Lucide::Trash)).tooltip(tr(cx, "common:secrets.remove")).on_click(cx.listener(move |this, _, _, cx| {
                            if n < this.rows.len() {
                                this.rows.remove(n);
                            }
                            cx.notify();
                        })))
                    })
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let failures: Vec<(String, Vec<String>)> = {
            let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
            for row in &current {
                if let Some(Some(reason)) = check_of(&row.op_ref) {
                    match grouped.iter_mut().find(|(known, _)| *known == reason) {
                        Some((_, names)) => names.push(row.name.clone()),
                        None => grouped.push((reason, vec![row.name.clone()])),
                    }
                }
            }
            grouped
        };
        let checking = self.checked.filter(|_| self.busy == Some(Busy::Check)).map(|(done, total)| cac_core::text_with(locale(cx), "common:secrets.checking", &[("done", &done.to_string()), ("total", &total.to_string())]));
        let saved_rows = saved.clone().unwrap_or_default();
        let rotate_here = deployable.on_ci_notify == CiNotify::Deploy || !deployable.rotate_command.trim().is_empty();
        let progress = *self.progress.lock().unwrap();
        let progress_line = progress.filter(|_| self.busy == Some(Busy::Rotate)).map(|(step, done, total)| {
            if step == "upload" {
                cac_core::text_with(locale(cx), "common:secrets.progressUpload", &[("service", &deployable.service_name)])
            } else {
                cac_core::text_with(locale(cx), "common:secrets.progressRead", &[("done", &done.to_string()), ("total", &total.to_string())])
            }
        });
        let line = self.line.clone().map(|(phrase, failed)| div().id("secrets-result").aria_label(phrase.clone()).leading_xs().text_xs().text_color(if failed { danger } else { muted }).child(phrase).test_support());
        let (target_deployable, target_server) = (deployable.clone(), server.clone());
        let (capture_deployable, capture_server) = (deployable.clone(), server.clone());
        let rotated = self.rotated.as_ref().map(|rotated| {
            div().id("secrets-rotated").flex().flex_col().gap_0p5().text_xs().children(rotated.items.iter().map(|(name, secret, changed)| {
                div().flex().gap_2().child(div().font_family(mono.clone()).child(name.clone())).child(div().text_color(muted).child(secret.clone())).child(div().text_color(if *changed { success } else { muted }).child(tr(cx, if *changed { "common:secrets.changed" } else { "common:secrets.same" })))
            })).test_support()
        });
        let history = secrets.map(|secrets| secrets.rotations.clone()).unwrap_or_default();
        let bridge = self.bridge_open.then(|| {
            let snippet = SECRETS_ENTRYPOINT_SNIPPET.to_string();
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(muted).child(tr(cx, "common:secrets.bridgeLead")))
                .child(div().p_2().rounded(px(6.)).bg(cx.theme().muted).font_family(mono.clone()).text_xs().child(snippet.clone()))
                .child(Button::new("secrets-bridge-copy").ghost().xsmall().icon(Icon::new(Lucide::Copy)).label(tr(cx, "common:secrets.bridgeCopy")).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(snippet.clone()))))
        });
        let command_dirty = self.rotate_command.read(cx).value().trim() != deployable.rotate_command;
        let command_target = deployable.clone();
        div()
            .id("secret-refs")
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .child(div().flex().items_center().gap_2().child(Icon::new(Lucide::KeyRound).size_4()).child(div().font_weight(FontWeight::MEDIUM).leading_sm().child(tr(cx, "common:secrets.title"))))
            .child(div().text_xs().leading_xs().text_color(muted).child(tr(cx, "common:secrets.lead")))
            .child(Button::new("secrets-bridge").link().xsmall().label(tr(cx, "common:secrets.bridgeTitle")).on_click(cx.listener(|this, _, _, cx| {
                this.bridge_open = !this.bridge_open;
                cx.notify();
            })))
            .children(bridge)
            .when(self.rows.is_empty() && saved.is_some(), |this| this.child(div().id("secrets-empty").text_xs().text_color(muted).child(tr(cx, "common:secrets.empty")).test_support()))
            .child(div().id("secret-rows").flex().flex_col().gap_1p5().children(rows).test_support())
            .when(!problems.is_empty(), |this| this.child(div().id("secrets-problems").flex().flex_col().text_xs().text_color(warning).children(problems.iter().map(|problem| div().child(Self::problem_phrase(problem, cx)))).test_support()))
            .when(!failures.is_empty(), |this| {
                this.child(div().id("secrets-unreadable").flex().flex_col().gap_0p5().text_xs().text_color(danger).children(failures.iter().map(|(reason, names)| {
                    div().child(format!("{} {} — {}", cac_core::text_count(locale(cx), "common:secrets.unreadable", names.len() as i64), names.join(", "), reason))
                })).test_support())
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .when(can_edit, |this| {
                        this.child(Button::new("secrets-add").outline().xsmall().icon(Icon::new(Lucide::Plus)).label(tr(cx, "common:secrets.add")).on_click(cx.listener(|this, _, window, cx| {
                            let mut next = this.current(cx);
                            next.push(SecretRefRow::default());
                            this.fill(&next, window, cx);
                            cx.notify();
                        })))
                        .child(Button::new("secrets-read-args").outline().xsmall().label(tr(cx, "common:secrets.readArgs")).disabled(self.busy.is_some()).on_click(cx.listener(|this, _, window, cx| this.read_build_args(window, cx))))
                    })
                    .child(Button::new("secrets-check").outline().xsmall().label(checking.unwrap_or_else(|| tr(cx, "common:secrets.check"))).disabled(self.busy.is_some() || current.is_empty()).on_click(cx.listener(|this, _, _, cx| this.check(cx))))
                    .when(can_edit, |this| {
                        this.child(Button::new("secrets-save").primary().xsmall().label(tr(cx, "common:secrets.save")).disabled(self.busy.is_some() || !dirty || !problems.is_empty()).on_click(cx.listener(|this, _, _, cx| this.save(cx))))
                    }),
            )
            .when(can_edit, |this| {
                this.child(
                    div()
                        .flex()
                        .items_end()
                        .gap_2()
                        .child(div().flex_1().flex().flex_col().gap_1().child(div().text_xs().child(tr(cx, "common:secrets.importItem"))).child(div().id("secrets-item").child(Input::new(&self.item)).test_support()))
                        .child(Button::new("secrets-import").outline().xsmall().label(tr(cx, "common:secrets.import")).disabled(self.busy.is_some() || self.item.read(cx).value().trim().is_empty()).on_click(cx.listener(|this, _, window, cx| this.import_item(window, cx)))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_xs().text_color(muted).child(tr(cx, "common:secrets.captureSummary")))
                        .child(
                            div()
                                .flex()
                                .items_end()
                                .gap_2()
                                .child(div().flex_1().flex().flex_col().gap_1().child(div().text_xs().child(tr(cx, "common:secrets.captureVault"))).child(Input::new(&self.vault)))
                                .child(div().flex_1().flex().flex_col().gap_1().child(div().text_xs().child(tr(cx, "common:secrets.captureItem"))).child(Input::new(&self.title)))
                                .child(Button::new("secrets-capture").outline().xsmall().label(tr(cx, "common:secrets.capture")).disabled(self.busy.is_some()).on_click(cx.listener(move |this, _, window, cx| this.capture(&capture_deployable, &capture_server, window, cx)))),
                        ),
                )
            })
            .when(can_rotate, |this| {
                let blocked = self.busy.is_some() || dirty || saved_rows.is_empty() || !rotate_here;
                let why = if !rotate_here { Some(tr(cx, "common:secrets.needsDeploy")) } else if dirty { Some(tr(cx, "common:secrets.saveFirst")) } else { None };
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(Button::new("secrets-rotate").primary().xsmall().icon(Icon::new(Lucide::RefreshCw)).label(tr(cx, "common:secrets.rotate")).disabled(blocked).on_click(cx.listener(move |this, _, window, cx| {
                            this.rotate(&target_deployable, &target_server, saved_rows.clone(), window, cx)
                        })))
                        .children(why.map(|why| div().id("secrets-rotate-why").text_xs().text_color(muted).child(why).test_support()))
                        .children(progress_line.map(|line| div().id("secrets-progress").text_xs().text_color(muted).child(line).test_support())),
                )
            })
            .children(line)
            .children(rotated)
            .when(can_edit, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_xs().child(tr(cx, "common:secrets.rotateCommand")))
                        .child(div().text_xs().text_color(muted).child(tr(cx, "common:secrets.rotateCommandLead")))
                        .child(
                            div().flex().gap_2().child(div().id("secrets-rotate-command").flex_1().child(Input::new(&self.rotate_command)).test_support()).child(
                                Button::new("secrets-rotate-command-save").outline().xsmall().label(tr(cx, "common:secrets.rotateCommandSave")).disabled(self.busy.is_some() || !command_dirty).on_click(cx.listener(move |this, _, _, cx| {
                                    let mut settings = command_target.settings();
                                    settings.rotate_command = this.rotate_command.read(cx).value().trim().to_string();
                                    let id = command_target.id.clone();
                                    Backend::run(cx, move |core| async move {
                                        let _ = core.platform().update_deployable(&id, &settings).await;
                                    });
                                })),
                            ),
                        ),
                )
            })
            .when(!history.is_empty(), |this| {
                this.child(div().text_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:secrets.history"))).child(div().id("secrets-history").flex().flex_col().rounded(px(6.)).border_1().border_color(border).children(history.iter().take(10).map(|rotation| {
                    let ok = rotation.status == "succeeded";
                    let detail = [rotation.started_by_name.clone().unwrap_or_default(), rotation.names.clone(), rotation.error.clone()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
                    div().flex().gap_2().px_2().py_1().border_t_1().border_color(border).text_xs()
                        .child(div().text_color(if ok { success } else { danger }).child(tr(cx, if ok { "common:secrets.status.succeeded" } else { "common:secrets.status.failed" })))
                        .child(div().flex_1().min_w_0().truncate().text_color(muted).child(detail))
                })).test_support())
            })
            .test_support()
            .into_any_element()
    }
}

impl Render for SecretRefsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let server = state.open_server.as_ref().and_then(|id| state.servers.iter().find(|server| &server.id == id)).cloned();
        let deployable = state.deploys.as_ref().and_then(|deploys| deploys.deployables.as_ref()).and_then(|list| list.iter().find(|known| known.id == self.deployable_id)).cloned();
        match (server, deployable) {
            (Some(server), Some(deployable)) => self.render_for(&deployable, &server, &state, window, cx),
            _ => div().into_any_element(),
        }
    }
}
