//! La pestaña Secrets de un swarm (`pages/StackSecrets.tsx`): los secrets y
//! variables del repo de GitHub de sus servicios, con el PAT de la app, que
//! vive en el llavero de esta máquina (`cac_ops::github`). Un valor de secret
//! se escribe y se va sellado: nunca vuelve a la pantalla ni al estado.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use cac_core::{infer_owner_repo, PlatformState};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

use super::deploy_settings::{GITHUB_PAT_KEY, GITHUB_TOKENS};
use super::services::empty;

type Pending<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

/// Un secret o una variable del repo: nombre, valor (sólo las variables),
/// creado y cambiado.
#[derive(Clone, Debug, PartialEq)]
pub struct RepoEntry {
    pub name: String,
    pub value: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// GitHub con el PAT del llavero. Aparte para que las pruebas no lean el
/// llavero ni hablen con GitHub. Lo que bloquea (llavero, `op`) se llama en
/// un hilo aparte.
pub trait GitHubRepo: Send + Sync {
    fn configured(&self) -> bool;
    fn op_reference(&self) -> Option<String>;
    fn set_token(&self, token: &str) -> Result<(), String>;
    fn delete_token(&self) -> Result<(), String>;
    fn load_from_1password(&self, reference: &str) -> Result<(), String>;
    fn refresh_from_1password(&self) -> Result<(), String>;
    fn list(&self, owner: String, repo: String, variables: bool) -> Pending<Vec<RepoEntry>>;
    fn put(&self, owner: String, repo: String, name: String, value: String, variable: Option<bool>) -> Pending<()>;
    fn delete(&self, owner: String, repo: String, name: String, variable: bool) -> Pending<()>;
}

struct SystemGitHub;

impl GitHubRepo for SystemGitHub {
    fn configured(&self) -> bool {
        cac_ops::github::token_configured(&GITHUB_TOKENS, GITHUB_PAT_KEY)
    }
    fn op_reference(&self) -> Option<String> {
        cac_ops::keychain::op_reference(GITHUB_PAT_KEY).ok()
    }
    fn set_token(&self, token: &str) -> Result<(), String> {
        cac_ops::github::set_token(&GITHUB_TOKENS, GITHUB_PAT_KEY, token)
    }
    fn delete_token(&self) -> Result<(), String> {
        cac_ops::github::delete_token(&GITHUB_TOKENS, GITHUB_PAT_KEY)
    }
    fn load_from_1password(&self, reference: &str) -> Result<(), String> {
        cac_ops::github::load_token_from_1password(&GITHUB_TOKENS, GITHUB_PAT_KEY, reference)
    }
    fn refresh_from_1password(&self) -> Result<(), String> {
        cac_ops::github::refresh_token_from_1password(&GITHUB_TOKENS, GITHUB_PAT_KEY)
    }
    fn list(&self, owner: String, repo: String, variables: bool) -> Pending<Vec<RepoEntry>> {
        Box::pin(async move {
            if variables {
                let list = cac_ops::github::list_variables(&GITHUB_TOKENS, GITHUB_PAT_KEY, &owner, &repo).await?;
                Ok(list.into_iter().map(|v| RepoEntry { name: v.name, value: Some(v.value), created_at: v.created_at, updated_at: v.updated_at }).collect())
            } else {
                let list = cac_ops::github::list_secrets(&GITHUB_TOKENS, GITHUB_PAT_KEY, &owner, &repo).await?;
                Ok(list.into_iter().map(|s| RepoEntry { name: s.name, value: None, created_at: s.created_at, updated_at: s.updated_at }).collect())
            }
        })
    }
    fn put(&self, owner: String, repo: String, name: String, value: String, variable: Option<bool>) -> Pending<()> {
        Box::pin(async move {
            match variable {
                Some(exists) => cac_ops::github::set_variable(&GITHUB_TOKENS, GITHUB_PAT_KEY, &owner, &repo, &name, &value, exists).await,
                None => cac_ops::github::set_secret(&GITHUB_TOKENS, GITHUB_PAT_KEY, &owner, &repo, &name, &value).await,
            }
        })
    }
    fn delete(&self, owner: String, repo: String, name: String, variable: bool) -> Pending<()> {
        Box::pin(async move {
            if variable {
                cac_ops::github::delete_variable(&GITHUB_TOKENS, GITHUB_PAT_KEY, &owner, &repo, &name).await
            } else {
                cac_ops::github::delete_secret(&GITHUB_TOKENS, GITHUB_PAT_KEY, &owner, &repo, &name).await
            }
        })
    }
}

/// Los de esta app: los del sistema, salvo que alguien ponga otros.
pub struct LocalGitHub(pub Arc<dyn GitHubRepo>);

impl Global for LocalGitHub {}

fn github(cx: &App) -> Arc<dyn GitHubRepo> {
    cx.try_global::<LocalGitHub>().map(|github| github.0.clone()).unwrap_or_else(|| Arc::new(SystemGitHub))
}

pub struct StackSecretsView {
    owner: Entity<InputState>,
    repo: Entity<InputState>,
    token: Entity<InputState>,
    op_reference: Entity<InputState>,
    name: Entity<InputState>,
    value: Entity<InputState>,
    configured: Option<bool>,
    variables: bool,
    entries: Option<Vec<RepoEntry>>,
    /// El formulario de uno nuevo o de cambiar uno: `Some(None)` es nuevo.
    editing: Option<Option<String>>,
    busy: bool,
    line: Option<(String, bool)>,
    prefilled: bool,
    /// Qué se pidió la última vez (dueño, repo, variables): con PAT y repo,
    /// lo que no se ha pedido se pide al pintar.
    asked_for: Option<(String, String, bool)>,
    _changed: Vec<Subscription>,
}

impl StackSecretsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let plain = |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx));
        let owner = plain(window, cx);
        let repo = plain(window, cx);
        let name = plain(window, cx);
        let op_reference = cx.new(|cx| InputState::new(window, cx).placeholder("op://Private/GitHub PAT/credential"));
        let token = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("ghp_…"));
        let value = cx.new(|cx| InputState::new(window, cx).masked(true));
        let changed = [&owner, &repo, &name, &op_reference, &token, &value].into_iter().map(|input| cx.subscribe(input, |_, _, event: &InputEvent, cx| if matches!(event, InputEvent::Change) { cx.notify() })).collect();
        let mut view = StackSecretsView { owner, repo, token, op_reference, name, value, configured: None, variables: false, entries: None, editing: None, busy: false, line: None, prefilled: false, asked_for: None, _changed: changed };
        view.refresh_token(window, cx);
        view
    }

    fn text(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    /// Algo que bloquea (llavero, `op`) en otro hilo, con su resultado aquí.
    fn blocking<T: Send + 'static>(&mut self, window: &mut Window, cx: &mut Context<Self>, work: impl FnOnce(Arc<dyn GitHubRepo>) -> T + Send + 'static, done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static) {
        self.busy = true;
        self.line = None;
        cx.notify();
        let github = github(cx);
        let answer = Backend::ask(cx, move |_| async move { tokio::task::spawn_blocking(move || work(github)).await.ok() });
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(Some(result)) = answer.await else { return };
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    done(this, result, window, cx);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn refresh_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.blocking(window, cx, |github| (github.configured(), github.op_reference()), |this, (configured, reference), window, cx| {
            this.configured = Some(configured);
            if let Some(reference) = reference {
                this.op_reference.update(cx, |input, cx| input.set_value(reference, window, cx));
            }
        });
    }

    fn token_done(&mut self, result: Result<(), String>, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                self.token.update(cx, |input, cx| input.set_value("", window, cx));
                self.refresh_token(window, cx);
            }
            Err(error) => self.line = Some((error, true)),
        }
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (owner, repo, variables) = (Self::text(&self.owner, cx), Self::text(&self.repo, cx), self.variables);
        if owner.is_empty() || repo.is_empty() {
            return;
        }
        self.asked_for = Some((owner.clone(), repo.clone(), variables));
        self.busy = true;
        self.line = None;
        cx.notify();
        let pending = github(cx).list(owner, repo, variables);
        let answer = Backend::ask(cx, move |_| async move { pending.await });
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = handle.update(cx, |_, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    match result {
                        Ok(entries) => this.entries = Some(entries),
                        Err(error) => this.line = Some((error, true)),
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn save_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (owner, repo) = (Self::text(&self.owner, cx), Self::text(&self.repo, cx));
        let name = match self.editing.clone() {
            Some(Some(name)) => name,
            _ => Self::text(&self.name, cx),
        };
        let value = self.value.read(cx).value().to_string();
        let exists = self.entries.iter().flatten().any(|entry| entry.name == name);
        let variable = self.variables.then_some(exists);
        self.busy = true;
        cx.notify();
        let pending = github(cx).put(owner, repo, name, value, variable);
        let answer = Backend::ask(cx, move |_| async move { pending.await });
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    match result {
                        Ok(()) => {
                            // El valor no se queda en ninguna parte.
                            this.value.update(cx, |input, cx| input.set_value("", window, cx));
                            this.name.update(cx, |input, cx| input.set_value("", window, cx));
                            this.editing = None;
                            this.load(window, cx);
                        }
                        Err(error) => this.line = Some((error, true)),
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn delete_entry(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let thing = tr(cx, if self.variables { "common:servers.variable" } else { "common:servers.secret" });
        let title = format!("{} · {name}", cac_core::text_with(locale(cx), "common:servers.deleteThing", &[("what", &thing)]));
        let (confirm, cancel) = (tr(cx, "common:servers.delete"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, None, &[confirm.as_str(), cancel.as_str()], cx);
        let (owner, repo, variable) = (Self::text(&self.owner, cx), Self::text(&self.repo, cx), self.variables);
        let github = github(cx);
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let pending = github.delete(owner, repo, name, variable);
            let answer = cx.update(|cx| Backend::ask(cx, move |_| async move { pending.await }));
            let Ok(result) = answer.await else { return };
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(()) => this.load(window, cx),
                        Err(error) => this.line = Some((error, true)),
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn pat_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, success, warning) = (theme.border, theme.muted_foreground, theme.success, theme.warning);
        // Mientras se mira en el llavero no se sabe: decir «sin configurar»
        // entonces era decir algo que quizá no es, y con los botones aún
        // apagados (un «Guardar» pulsado ahí no hacía nada).
        let status = match self.configured {
            Some(true) => Some(div().id("github-pat-status").aria_label(tr(cx, "common:servers.configured")).text_xs().text_color(success).child(tr(cx, "common:servers.configured")).test_support()),
            Some(false) => Some(div().id("github-pat-status").aria_label(tr(cx, "common:servers.notConfigured")).text_xs().text_color(warning).child(tr(cx, "common:servers.notConfigured")).test_support()),
            None => None,
        };
        let has_reference = !Self::text(&self.op_reference, cx).is_empty();
        let configured = self.configured == Some(true);
        div()
            .id("github-pat")
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .child(div().flex().items_center().gap_2().child(Icon::new(Lucide::KeyRound).size_4()).child(div().flex_1().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:servers.githubPat"))).children(status))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(div().id("github-op-reference").flex_1().child(Input::new(&self.op_reference)).test_support())
                    .child(Button::new("github-op-load").outline().xsmall().label(tr(cx, if configured { "common:servers.update" } else { "common:servers.load" })).disabled(self.busy || !has_reference).on_click(cx.listener(|this, _, window, cx| {
                        let reference = Self::text(&this.op_reference, cx);
                        this.blocking(window, cx, move |github| github.load_from_1password(&reference), |this, result, window, cx| this.token_done(result, window, cx));
                    })))
                    .when(configured && has_reference, |row| {
                        row.child(Button::new("github-op-refresh").ghost().xsmall().icon(Icon::new(Lucide::RefreshCw)).tooltip(tr(cx, "common:servers.refresh")).disabled(self.busy).on_click(cx.listener(|this, _, window, cx| {
                            this.blocking(window, cx, |github| github.refresh_from_1password(), |this, result, window, cx| this.token_done(result, window, cx));
                        })))
                    }),
            )
            .child(div().text_xs().text_color(muted).child(tr(cx, "common:servers.orPasteManually")))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(div().id("github-token").flex_1().child(Input::new(&self.token)).test_support())
                    .child(Button::new("github-token-save").outline().xsmall().label(tr(cx, if configured { "common:servers.update" } else { "common:servers.save" })).disabled(self.busy || self.token.read(cx).value().trim().is_empty()).on_click(cx.listener(|this, _, window, cx| {
                        let token = this.token.read(cx).value().trim().to_string();
                        this.blocking(window, cx, move |github| github.set_token(&token), |this, result, window, cx| this.token_done(result, window, cx));
                    })))
                    .when(configured, |row| {
                        row.child(Button::new("github-token-remove").ghost().xsmall().label(tr(cx, "common:servers.remove")).disabled(self.busy).on_click(cx.listener(|this, _, window, cx| {
                            this.blocking(window, cx, |github| github.delete_token(), |this, result, window, cx| {
                                this.entries = None;
                                this.token_done(result, window, cx)
                            });
                        })))
                    }),
            )
    }

    fn entries_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, accent, mono) = (theme.border, theme.muted_foreground, theme.accent, theme.mono_font_family.clone());
        let tab = |id: &'static str, key: &str, variables: bool, cx: &mut Context<Self>| {
            let on = self.variables == variables;
            div()
                .id(id)
                .aria_label(tr(cx, key))
                .px_2()
                .py_0p5()
                .rounded(px(4.))
                .text_xs()
                .cursor_pointer()
                .when(on, |this| this.bg(accent).font_weight(FontWeight::MEDIUM))
                .when(!on, |this| this.text_color(muted))
                .child(tr(cx, key))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.variables != variables {
                        (this.variables, this.entries, this.editing) = (variables, None, None);
                        this.load(window, cx);
                    }
                }))
                .test_support()
        };
        let new_key = if self.variables { "common:servers.newVariable" } else { "common:servers.newSecret" };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex().p_0p5().rounded(px(6.)).border_1().border_color(border).child(tab("github-tab-secrets", "common:servers.secrets", false, cx)).child(tab("github-tab-variables", "common:servers.variables", true, cx)))
            .child(div().flex_1())
            .child(Button::new("github-new").outline().xsmall().icon(Icon::new(Lucide::Plus)).label(tr(cx, new_key)).disabled(self.busy).on_click(cx.listener(|this, _, _, cx| {
                this.editing = Some(None);
                cx.notify();
            })));
        let form = self.editing.clone().map(|editing| {
            let ready = editing.is_some() || !Self::text(&self.name, cx).is_empty();
            let placeholder = tr(cx, if self.variables { "common:servers.variableValue" } else { "common:servers.secretValue" });
            div()
                .id("github-form")
                .flex()
                .items_end()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .bg(cx.theme().muted)
                .child(match &editing {
                    Some(name) => div().w(px(200.)).font_family(mono.clone()).text_xs().child(name.clone()).into_any_element(),
                    None => div().id("github-form-name").w(px(200.)).child(Input::new(&self.name)).test_support().into_any_element(),
                })
                .child(div().id("github-form-value").flex_1().child(Input::new(&self.value)).aria_label(placeholder).test_support())
                .child(Button::new("github-form-save").primary().xsmall().label(tr(cx, "common:servers.save")).disabled(self.busy || !ready).on_click(cx.listener(|this, _, window, cx| this.save_entry(window, cx))))
                .child(Button::new("github-form-cancel").ghost().xsmall().label(tr(cx, "common:servers.cancel")).on_click(cx.listener(|this, _, window, cx| {
                    this.editing = None;
                    this.value.update(cx, |input, cx| input.set_value("", window, cx));
                    cx.notify();
                })))
                .test_support()
        });
        let body = match &self.entries {
            None => empty("github-loading", tr(cx, "common:servers.loading"), muted),
            Some(entries) if entries.is_empty() => empty("github-empty", tr(cx, if self.variables { "common:servers.noVariables" } else { "common:servers.noSecrets" }), muted),
            Some(entries) => div()
                .id("github-entries")
                .flex()
                .flex_col()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .children(entries.iter().map(|entry| {
                    let (edited, deleted) = (entry.name.clone(), entry.name.clone());
                    let updated = entry.updated_at.get(..10).unwrap_or(&entry.updated_at).to_string();
                    div()
                        .id(SharedString::from(format!("github-entry-{}", entry.name)))
                        .aria_label(entry.name.clone())
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(border)
                        .text_xs()
                        .child(div().w(px(200.)).font_family(mono.clone()).truncate().child(entry.name.clone()))
                        .child(div().flex_1().min_w_0().truncate().font_family(mono.clone()).text_color(muted).child(entry.value.clone().unwrap_or_default()))
                        .child(div().text_color(muted).child(updated))
                        .child(Button::new(SharedString::from(format!("github-edit-{}", entry.name))).ghost().xsmall().label(tr(cx, if self.variables { "common:servers.edit" } else { "common:servers.update" })).on_click(cx.listener(move |this, _, window, cx| {
                            this.editing = Some(Some(edited.clone()));
                            this.value.update(cx, |input, cx| input.set_value("", window, cx));
                            cx.notify();
                        })))
                        .child(Button::new(SharedString::from(format!("github-delete-{}", entry.name))).ghost().xsmall().icon(Icon::new(Lucide::Trash)).tooltip(tr(cx, "common:servers.delete")).on_click(cx.listener(move |this, _, window, cx| this.delete_entry(deleted.clone(), window, cx))))
                        .test_support()
                }))
                .test_support()
                .into_any_element(),
        };
        div().id("github-repo-entries").flex().flex_col().gap_2().child(header).children(form).child(body)
    }
}

impl Render for StackSecretsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state: PlatformState = Backend::snapshot(cx).read(cx).platform.clone();
        // El repo, deducido de la imagen del primer servicio que lo diga.
        if !self.prefilled {
            if let Some((owner, repo)) = state.services.iter().flatten().find_map(|service| infer_owner_repo(&service.image)) {
                self.owner.update(cx, |input, cx| input.set_value(owner, window, cx));
                self.repo.update(cx, |input, cx| input.set_value(repo, window, cx));
                self.prefilled = true;
            }
        }
        let key = (Self::text(&self.owner, cx), Self::text(&self.repo, cx), self.variables);
        if self.configured == Some(true) && !self.busy && self.entries.is_none() && !key.0.is_empty() && !key.1.is_empty() && self.asked_for.as_ref() != Some(&key) {
            self.load(window, cx);
        }
        let theme = cx.theme();
        let (border, danger) = (theme.border, theme.danger);
        let line = self.line.clone().map(|(phrase, failed)| div().id("github-result").aria_label(phrase.clone()).text_xs().text_color(if failed { danger } else { theme.muted_foreground }).child(phrase).test_support());
        let ready = !Self::text(&self.owner, cx).is_empty() && !Self::text(&self.repo, cx).is_empty();
        let configured = self.configured == Some(true);
        div()
            .id("server-secrets")
            .flex()
            .flex_col()
            .gap_3()
            .child(div().font_weight(FontWeight::MEDIUM).leading_sm().child(tr(cx, "common:servers.githubSecrets")))
            .child(div().text_xs().text_color(theme.muted_foreground).child(tr(cx, "common:servers.secretsMoved")))
            .child(self.pat_card(cx))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .child(div().flex_1().flex().flex_col().gap_1().child(div().text_xs().child(tr(cx, "common:servers.owner"))).child(div().id("github-owner").child(Input::new(&self.owner)).test_support()))
                    .child(div().flex_1().flex().flex_col().gap_1().child(div().text_xs().child(tr(cx, "common:servers.repository"))).child(div().id("github-repo").child(Input::new(&self.repo)).test_support()))
                    .child(Button::new("github-load").outline().xsmall().icon(Icon::new(Lucide::RefreshCw)).label(tr(cx, "common:servers.load")).disabled(self.busy || !ready || !configured).on_click(cx.listener(|this, _, window, cx| this.load(window, cx)))),
            )
            .children(line)
            .when(configured, |this| this.child(self.entries_card(cx)))
    }
}
