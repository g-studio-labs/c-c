//! Los ajustes de un desplegable, debajo de su despliegue y sólo para quien
//! administra (`CINoticeSection`, `RepoField`, `WorkflowField`,
//! `BuildOnHostSection` y `MigrateCommandField`): el aviso del CI con su
//! llave, de dónde sale la imagen, la rama y el build en el servidor, y las
//! migraciones. Cada sección se guarda por su lado, como en Tauri.

use cac_core::{ci_notice_step, repo_of_deployable, server_edit_phrase, uses_short_sha, CiNotify, Deployable, DeployableSettings, ImageBuild, Server, AGENT_VERSION_BUILDS, AGENT_VERSION_MIGRATES};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// El nombre del secret que lee el paso del workflow.
const CI_KEY_SECRET: &str = "CAC_DEPLOY_KEY";

/// Dónde guarda la app el PAT de GitHub en el llavero: uno para toda la app
/// (el nombre es histórico, `github-repo.ts`).
pub(crate) const GITHUB_PAT_KEY: &str = "PATK_global_usage";

/// El PAT leído del llavero, para no preguntarle a cada llamada.
pub(crate) static GITHUB_TOKENS: std::sync::LazyLock<cac_ops::github::TokenCache> = std::sync::LazyLock::new(Default::default);

type Pending = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>;

/// Guardar un secret en un repo de GitHub con el PAT de la app. Aparte para
/// que las pruebas no lean el llavero de verdad.
pub trait RepoSecrets: Send + Sync {
    fn configured(&self) -> bool;
    fn put(&self, owner: String, repo: String, name: String, value: String) -> Pending;
}

struct SystemRepoSecrets;

impl RepoSecrets for SystemRepoSecrets {
    fn configured(&self) -> bool {
        cac_ops::github::token_configured(&GITHUB_TOKENS, GITHUB_PAT_KEY)
    }
    fn put(&self, owner: String, repo: String, name: String, value: String) -> Pending {
        Box::pin(async move { cac_ops::github::set_secret(&GITHUB_TOKENS, GITHUB_PAT_KEY, &owner, &repo, &name, &value).await })
    }
}

/// Los de esta app: los del sistema, salvo que alguien ponga otros.
pub struct LocalRepoSecrets(pub std::sync::Arc<dyn RepoSecrets>);

impl Global for LocalRepoSecrets {}

fn repo_secrets(cx: &App) -> std::sync::Arc<dyn RepoSecrets> {
    cx.try_global::<LocalRepoSecrets>().map(|secrets| secrets.0.clone()).unwrap_or_else(|| std::sync::Arc::new(SystemRepoSecrets))
}

pub struct DeploySettingsView {
    deployable_id: String,
    repo: Entity<InputState>,
    workflow: Entity<InputState>,
    branch: Entity<InputState>,
    build_context: Entity<InputState>,
    dockerfile: Entity<InputState>,
    migrate: Entity<InputState>,
    build_on_host: bool,
    /// La llave del CI entera, sólo aquí y sólo si no se pudo guardar sola en
    /// el repo: al cerrar la vista se pierde.
    key: Option<String>,
    /// El repo donde quedó guardada como secret, si se pudo.
    saved_in: Option<String>,
    busy: bool,
    line: Option<(String, bool)>,
    _changed: Vec<Subscription>,
}

impl DeploySettingsView {
    pub fn new(deployable: &Deployable, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        let mut input = |value: &str, placeholder: Option<String>, window: &mut Window, cx: &mut Context<Self>| {
            let value = value.to_string();
            let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.unwrap_or_default()).default_value(value));
            subscriptions.push(cx.subscribe(&state, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
            state
        };
        let repo = input(&deployable.repo_full_name, Some("owner/name".into()), window, cx);
        let workflow = input(&deployable.build_workflow, Some("prod.yml".into()), window, cx);
        let branch = input(&deployable.branch, None, window, cx);
        let build_context = input(&deployable.build_context, Some(".".into()), window, cx);
        let dockerfile = input(&deployable.dockerfile, Some("Dockerfile".into()), window, cx);
        let migrate = input(&deployable.migrate_command, Some("npx prisma migrate deploy".into()), window, cx);
        DeploySettingsView {
            deployable_id: deployable.id.clone(),
            repo,
            workflow,
            branch,
            build_context,
            dockerfile,
            migrate,
            build_on_host: deployable.build_on_host,
            key: None,
            saved_in: None,
            busy: false,
            line: None,
            _changed: subscriptions,
        }
    }

    pub fn deployable_id(&self) -> &str {
        &self.deployable_id
    }

    fn value(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    /// Guarda la fila entera con este cambio encima.
    fn save(&mut self, deployable: &Deployable, change: impl FnOnce(&mut DeployableSettings), cx: &mut Context<Self>) {
        let mut settings = deployable.settings();
        change(&mut settings);
        let id = deployable.id.clone();
        self.busy = true;
        self.line = None;
        cx.notify();
        let done = Backend::ask(cx, move |core| async move { core.platform().update_deployable(&id, &settings).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = done.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                this.line = Some(match result {
                    Ok(()) => (tr(cx, "common:deploy.build.saved"), false),
                    Err(error) => (server_edit_phrase(locale(cx), &error, "common:deploy.build.saveFailed"), true),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Pasar a «Desplegar» cambia quién despliega: se pregunta.
    fn set_mode(&mut self, deployable: &Deployable, server: &Server, mode: CiNotify, window: &mut Window, cx: &mut Context<Self>) {
        if mode == deployable.on_ci_notify {
            return;
        }
        if mode == CiNotify::Record {
            self.save(deployable, |settings| settings.on_ci_notify = mode, cx);
            return;
        }
        let title = tr(cx, "common:deploy.ci.toDeployTitle");
        let body = cac_core::text_with(locale(cx), "common:deploy.ci.toDeployBody", &[("server", &server.name)]);
        let (confirm, cancel) = (tr(cx, "common:deploy.ci.toDeployConfirm"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        let deployable = deployable.clone();
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |this, cx| this.save(&deployable, |settings| settings.on_ci_notify = mode, cx)).ok();
            }
        })
        .detach();
    }

    /// Acuñar otra tumba la de ahora: se pregunta. Con el PAT de la app la
    /// llave va sola a los secrets del repo; sin él, se enseña una vez.
    fn mint(&mut self, deployable: &Deployable, window: &mut Window, cx: &mut Context<Self>) {
        if deployable.ci_key_preview.is_empty() {
            self.mint_now(deployable.clone(), cx);
            return;
        }
        let title = tr(cx, "common:deploy.ci.remintTitle");
        let body = cac_core::text_with(locale(cx), "common:deploy.ci.remintBody", &[("preview", &deployable.ci_key_preview)]);
        let (confirm, cancel) = (tr(cx, "common:deploy.ci.remint"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        let deployable = deployable.clone();
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |this, cx| this.mint_now(deployable, cx)).ok();
            }
        })
        .detach();
    }

    fn mint_now(&mut self, deployable: Deployable, cx: &mut Context<Self>) {
        (self.busy, self.line, self.key, self.saved_in) = (true, None, None, None);
        cx.notify();
        let target = repo_of_deployable(&Deployable { repo_full_name: Self::value(&self.repo, cx), ..deployable.clone() });
        let secrets = repo_secrets(cx);
        let done = Backend::ask(cx, move |core| async move {
            let key = core.platform().mint_ci_key(&deployable.id).await?;
            let pushed = match target.split_once('/') {
                Some((owner, repo)) if !owner.is_empty() && !repo.is_empty() && secrets.configured() => {
                    secrets.put(owner.into(), repo.into(), CI_KEY_SECRET.into(), key.key.clone()).await.map(|()| target.clone()).map_err(|error| (target.clone(), error))
                }
                _ => Err((String::new(), String::new())),
            };
            Ok::<_, cac_core::ServerEditError>((key.key, pushed))
        });
        cx.spawn(async move |this, cx| {
            let Ok(result) = done.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((_, Ok(repo))) => this.saved_in = Some(repo),
                    Ok((key, Err((repo, error)))) => {
                        this.key = Some(key);
                        if !repo.is_empty() {
                            let phrase = cac_core::text_with(locale(cx), "common:deploy.ci.pushFailed", &[("repo", &repo)]);
                            this.line = Some((format!("{phrase}: {error}"), true));
                        }
                    }
                    Err(error) => this.line = Some((server_edit_phrase(locale(cx), &error, "common:deploy.ci.mint"), true)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn field(label: String, input: &Entity<InputState>, id: &'static str) -> impl IntoElement {
        div().flex_1().min_w(px(160.)).flex().flex_col().gap_1().child(div().leading_xs().text_xs().child(label)).child(div().id(id).child(Input::new(input).small()).test_support())
    }

    fn section(id: &'static str, title: String, lead: Option<String>, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        div()
            .id(id)
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .child(div().font_weight(FontWeight::MEDIUM).leading_sm().child(title))
            .children(lead.map(|lead| div().text_xs().leading_xs().text_color(theme.muted_foreground).child(lead)))
    }

    fn copy_block(id: &'static str, text: String, label: String, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let copied = text.clone();
        div()
            .flex()
            .items_start()
            .gap_2()
            .child(div().flex_1().min_w_0().p_2().rounded(px(6.)).bg(theme.muted).font_family(theme.mono_font_family.clone()).text_xs().whitespace_normal().child(text))
            .child(Button::new(id).ghost().xsmall().icon(Icon::new(Lucide::Copy)).tooltip(label).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))))
    }

    fn ci_section(&self, deployable: &Deployable, server: &Server, builds: &[ImageBuild], cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, accent, success, warning, mono, border) = (theme.muted_foreground, theme.accent, theme.success, theme.warning, theme.mono_font_family.clone(), theme.border);
        let short_tags = uses_short_sha(&deployable.current_image) || deployable.short_tags;
        let step = ci_notice_step(&format!("{}/ingest/v1/deploys", Backend::core(cx).base_url().trim_end_matches('/')), short_tags);
        let mode = |mode: CiNotify, id: &'static str, key: &str, cx: &mut Context<Self>| {
            let (target, server) = (deployable.clone(), server.clone());
            let on = deployable.on_ci_notify == mode;
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
                .on_click(cx.listener(move |this, _, window, cx| this.set_mode(&target, &server, mode, window, cx)))
                .test_support()
        };
        // Parte en dos líneas si no cabe: en una ventana estrecha el botón
        // de la llave se salía por la derecha.
        let modes = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(div().text_xs().text_color(muted).child(tr(cx, "common:deploy.ci.mode")))
            .child(
                div()
                    .flex()
                    .p_0p5()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(border)
                    .child(mode(CiNotify::Record, "deploy-ci-record", "common:deploy.ci.modes.record", cx))
                    .child(mode(CiNotify::Deploy, "deploy-ci-deploy", "common:deploy.ci.modes.deploy", cx)),
            )
            .child(div().flex_1())
            .when(deployable.build_on_host, |row| row.child(div().text_xs().text_color(muted).child(tr(cx, "common:deploy.ci.noKeyNeeded"))))
            .when(!deployable.build_on_host, |row| {
                let preview = if deployable.ci_key_preview.is_empty() {
                    tr(cx, "common:deploy.ci.noKey")
                } else {
                    cac_core::text_with(locale(cx), "common:deploy.ci.keyPreview", &[("preview", &deployable.ci_key_preview)])
                };
                let target = deployable.clone();
                row.child(div().id("deploy-ci-preview").aria_label(preview.clone()).font_family(mono.clone()).text_xs().text_color(muted).child(preview).test_support()).child(
                    Button::new("deploy-ci-mint")
                        .outline()
                        .small()
                        .icon(Icon::new(Lucide::KeyRound))
                        .label(if deployable.ci_key_preview.is_empty() { tr(cx, "common:deploy.ci.mint") } else { tr(cx, "common:deploy.ci.remint") })
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, window, cx| this.mint(&target, window, cx))),
                )
            });
        let dirty = Self::value(&self.repo, cx) != deployable.repo_full_name || Self::value(&self.workflow, cx) != deployable.build_workflow;
        let target = deployable.clone();
        let github = div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_2()
            .child(Self::field(tr(cx, "common:deploy.ci.repo"), &self.repo, "deploy-repo"))
            .child(Self::field(tr(cx, "common:deploy.ci.workflow"), &self.workflow, "deploy-workflow"))
            .child(Button::new("deploy-github-save").outline().small().label(tr(cx, "common:deploy.ci.saveGitHub")).disabled(self.busy || !dirty).on_click(cx.listener(move |this, _, _, cx| {
                let (repo, workflow) = (Self::value(&this.repo, cx), Self::value(&this.workflow, cx));
                this.save(&target, |settings| {
                    settings.repo_full_name = repo;
                    settings.build_workflow = workflow;
                    settings.short_tags = short_tags;
                }, cx)
            })));
        let saved = self.saved_in.as_ref().map(|repo| {
            div()
                .id("deploy-ci-saved")
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(success.opacity(0.4))
                .bg(success.opacity(0.1))
                .text_xs()
                .child(cac_core::text_with(locale(cx), "common:deploy.ci.keySaved", &[("repo", repo), ("secret", CI_KEY_SECRET)]))
                .child(tr(cx, "common:deploy.ci.stepLead"))
                .child(Self::copy_block("deploy-ci-copy-step", step.clone(), tr(cx, "common:deploy.ci.copyStep"), cx))
                .test_support()
        });
        let shown_key = self.key.as_ref().map(|key| {
            div()
                .id("deploy-ci-key")
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(warning.opacity(0.4))
                .bg(warning.opacity(0.1))
                .text_xs()
                .child(tr(cx, "common:deploy.ci.keyOnce"))
                .child(Self::copy_block("deploy-ci-copy-key", key.clone(), tr(cx, "common:deploy.ci.copyKey"), cx))
                .child(tr(cx, "common:deploy.ci.stepLead"))
                .child(Self::copy_block("deploy-ci-copy-step", step.clone(), tr(cx, "common:deploy.ci.copyStep"), cx))
                .test_support()
        });
        let builds_list = if builds.is_empty() {
            div().text_xs().text_color(muted).child(tr(cx, "common:deploy.ci.noBuilds")).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .children(builds.iter().take(10).map(|build| {
                    let short: String = build.sha.chars().take(7).collect();
                    let detail = [build.git_ref.trim_start_matches("refs/heads/").to_string(), build.actor.clone()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
                    // Lo escribe quien tiene la llave: sólo se abre si es una página web.
                    let run = build.run_url.starts_with("https://").then(|| {
                        let url = build.run_url.clone();
                        Button::new(SharedString::from(format!("deploy-build-run-{}", build.id))).ghost().xsmall().icon(Icon::new(Lucide::ExternalLink)).tooltip(tr(cx, "common:deploy.ci.openRun")).on_click(move |_, _, cx| cx.open_url(&url))
                    });
                    div()
                        .id(SharedString::from(format!("deploy-build-{}", build.id)))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(border)
                        .text_xs()
                        .child(div().font_family(mono.clone()).child(short))
                        .child(div().flex_1().min_w_0().truncate().text_color(muted).child(detail))
                        .children(run)
                }))
                .into_any_element()
        };
        Self::section("deploy-ci", tr(cx, "common:deploy.ci.title"), Some(tr(cx, "common:deploy.ci.lead")), cx)
            .child(modes)
            .child(github)
            .child(div().text_xs().text_color(muted).child(tr(cx, "common:deploy.ci.githubLead")))
            .children(saved)
            .children(shown_key)
            .child({
                // Todos los runs del CI de este servicio están en Actividad.
                let filter = cac_core::ActivityFilter { deployable_id: deployable.id.clone(), ..Default::default() };
                div().flex().items_center().justify_between().child(div().text_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:deploy.ci.builds"))).child(
                    Button::new("deploy-ci-all-runs").link().xsmall().label(tr(cx, "common:deploy.ci.allRuns")).on_click(move |_, _, cx| {
                        let filter = filter.clone();
                        Backend::run(cx, move |core| async move { core.platform().open_activity(filter).await });
                    }),
                )
            })
            .child(builds_list)
    }

    fn build_section(&self, deployable: &Deployable, server: &Server, cx: &mut Context<Self>) -> impl IntoElement {
        let warning = cx.theme().warning;
        let on = self.build_on_host;
        let too_old = on && server.agent_version < AGENT_VERSION_BUILDS;
        let no_repo = on && deployable.repo_full_name.is_empty();
        let dirty = Self::value(&self.branch, cx) != deployable.branch
            || on != deployable.build_on_host
            || Self::value(&self.build_context, cx) != deployable.build_context
            || Self::value(&self.dockerfile, cx) != deployable.dockerfile;
        let target = deployable.clone();
        Self::section("deploy-build", tr(cx, "common:deploy.build.title"), None, cx)
            .child(Self::field(tr(cx, "common:deploy.build.branch"), &self.branch, "deploy-branch"))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(tr(cx, "common:deploy.build.branchLead")))
            .child(
                Checkbox::new("deploy-build-on-host").checked(on).label(tr(cx, "common:deploy.build.onHost")).on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.build_on_host = *checked;
                    cx.notify();
                })),
            )
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(tr(cx, "common:deploy.build.onHostLead")))
            .when(on, |section| {
                section.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(Self::field(tr(cx, "common:deploy.build.context"), &self.build_context, "deploy-build-context"))
                        .child(Self::field(tr(cx, "common:deploy.build.dockerfile"), &self.dockerfile, "deploy-dockerfile")),
                )
            })
            .when(too_old, |section| section.child(div().id("deploy-build-agent-old").text_xs().text_color(warning).child(cac_core::text_with(locale(cx), "common:deploy.build.agentOld", &[("version", &AGENT_VERSION_BUILDS.to_string())])).test_support()))
            .when(no_repo, |section| section.child(div().id("deploy-build-no-repo").text_xs().text_color(warning).child(tr(cx, "common:deploy.build.noRepo")).test_support()))
            .child(div().flex().justify_end().child(Button::new("deploy-build-save").outline().small().label(tr(cx, "common:deploy.build.save")).disabled(self.busy || !dirty || no_repo).on_click(cx.listener(move |this, _, _, cx| {
                let (branch, on, context, dockerfile) = (Self::value(&this.branch, cx), this.build_on_host, Self::value(&this.build_context, cx), Self::value(&this.dockerfile, cx));
                this.save(&target, |settings| {
                    settings.branch = branch;
                    settings.build_on_host = on;
                    settings.build_context = context;
                    settings.dockerfile = dockerfile;
                }, cx)
            }))))
    }

    fn migrate_section(&self, deployable: &Deployable, server: &Server, cx: &mut Context<Self>) -> impl IntoElement {
        let command = Self::value(&self.migrate, cx);
        let too_old = !command.is_empty() && server.agent_version < AGENT_VERSION_MIGRATES;
        let dirty = command != deployable.migrate_command;
        let target = deployable.clone();
        Self::section("deploy-migrate", tr(cx, "common:deploy.migrate"), Some(tr(cx, "common:deploy.migrateLead")), cx)
            .child(div().id("deploy-migrate-command").child(Input::new(&self.migrate).small()).test_support())
            .when(too_old, |section| section.child(div().id("deploy-migrate-agent-old").text_xs().text_color(cx.theme().warning).child(tr(cx, "common:deploy.migrateAgentOld")).test_support()))
            .child(div().flex().justify_end().child(Button::new("deploy-migrate-save").outline().small().label(tr(cx, "common:deploy.migrateSave")).disabled(self.busy || !dirty).on_click(cx.listener(move |this, _, _, cx| {
                let command = Self::value(&this.migrate, cx);
                this.save(&target, |settings| settings.migrate_command = command, cx)
            }))))
    }

    pub fn render_for(&mut self, deployable: &Deployable, server: &Server, builds: &[ImageBuild], cx: &mut Context<Self>) -> AnyElement {
        let (muted, danger) = (cx.theme().muted_foreground, cx.theme().danger);
        let line = self.line.clone().map(|(phrase, failed)| div().id("deploy-settings-result").aria_label(phrase.clone()).leading_xs().text_color(if failed { danger } else { muted }).child(phrase).test_support());
        div()
            .id("deploy-settings")
            .flex()
            .flex_col()
            .gap_4()
            .pt_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .children(line)
            .child(self.ci_section(deployable, server, builds, cx))
            .child(self.build_section(deployable, server, cx))
            .child(self.migrate_section(deployable, server, cx))
            .test_support()
            .into_any_element()
    }
}

impl Render for DeploySettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let server = state.open_server.as_ref().and_then(|id| state.servers.iter().find(|server| &server.id == id)).cloned();
        let deploys = state.deploys.as_ref();
        let deployable = deploys.and_then(|deploys| deploys.deployables.as_ref()).and_then(|list| list.iter().find(|known| known.id == self.deployable_id)).cloned();
        match (server, deployable) {
            (Some(server), Some(deployable)) => {
                let builds = deploys.map(|deploys| deploys.builds.clone()).unwrap_or_default();
                self.render_for(&deployable, &server, &builds, cx)
            }
            _ => div().into_any_element(),
        }
    }
}
