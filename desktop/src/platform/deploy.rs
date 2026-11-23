//! La pestaña de despliegue de un swarm (`ServiceDeploy.tsx`, `DeployPanel` y
//! `CommitPicker`): a la izquierda los servicios; a la derecha, el elegido.
//! Uno que todavía no se despliega desde cac se registra (no toca el
//! servidor); uno registrado ofrece sus commits, el historial con su log y
//! volver atrás.

use cac_core::{commit_pick, server_edit_phrase, short_ref, DeployStatus, Deployable, Deployment, LogLine, NewDeployable, PlatformState, Server, SwarmService, AGENT_VERSION_DEPLOYS, LIST_PREVIEW};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::log_view::LogBox;
use crate::theme::Leading as _;

use super::deploy_settings::DeploySettingsView;
use super::secret_refs::SecretRefsView;
use super::services::{empty, pill};

const LIST_WIDTH: f32 = 240.;

pub struct DeployView {
    /// El registro de un servicio, prellenado al elegirlo.
    name: Entity<InputState>,
    environment: Entity<InputState>,
    image_repo: Entity<InputState>,
    /// «Otro commit…»: un sha escrito a mano.
    sha: Entity<InputState>,
    manual: bool,
    /// El commit elegido en la lista, mientras siga en ella.
    picked: Option<String>,
    all_commits: bool,
    all_history: bool,
    busy: bool,
    /// Cómo acabó lo último que se pidió: la frase y si falló.
    line: Option<(String, bool)>,
    logs: LogBox,
    /// De qué servicio es lo escrito en el registro: se elige aquí o desde
    /// su fila en Servicios, y al cambiar se vuelve a prellenar.
    prefilled_for: Option<String>,
    /// Los ajustes del desplegable que se mira, para quien administra.
    settings: Option<Entity<DeploySettingsView>>,
    /// Sus secrets por referencia: los ve quien mira, los cambia quien
    /// administra y los rota un miembro.
    secrets: Option<Entity<SecretRefsView>>,
}

impl DeployView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: Option<String>, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.unwrap_or_default()));
        let repo_hint = tr(cx, "common:deploy.imageRepoHint");
        let sha_hint = tr(cx, "common:deploy.shaHint");
        DeployView {
            name: input(None, window, cx),
            environment: input(None, window, cx),
            image_repo: input(Some(repo_hint), window, cx),
            sha: input(Some(sha_hint), window, cx),
            manual: false,
            picked: None,
            all_commits: false,
            all_history: false,
            busy: false,
            line: None,
            logs: LogBox::new(),
            prefilled_for: None,
            settings: None,
            secrets: None,
        }
    }

    fn choose(&mut self, service: SwarmService, cx: &mut Context<Self>) {
        Backend::run(cx, move |core| async move { core.platform().choose_service(&service.name).await });
    }

    /// Lo que se deduce del servicio que corre, y lo demás de cero.
    fn prefill(&mut self, service: &SwarmService, window: &mut Window, cx: &mut Context<Self>) {
        let new = NewDeployable::from_service(&service.name, &service.stack, &service.image);
        self.name.update(cx, |input, cx| input.set_value(new.name, window, cx));
        self.environment.update(cx, |input, cx| input.set_value(new.environment, window, cx));
        self.image_repo.update(cx, |input, cx| input.set_value(new.image_repo, window, cx));
        self.sha.update(cx, |input, cx| input.set_value("", window, cx));
        (self.picked, self.manual, self.all_commits, self.all_history, self.line) = (None, false, false, false, None);
        self.prefilled_for = Some(service.name.clone());
        // Otro servicio: los ajustes del anterior (y su llave del CI, que se
        // enseña una sola vez) no se quedan.
        self.settings = None;
        self.secrets = None;
    }

    /// Lo que se pide al backend, con su fallo dicho debajo.
    fn ask<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(cac_core::Core) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, cac_core::ServerEditError>> + Send>> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        fallback_key: &'static str,
    ) {
        self.busy = true;
        self.line = None;
        cx.notify();
        let answer = Backend::ask(cx, move |core| work(core));
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(value) => done(this, value, cx),
                    Err(error) => this.line = Some((server_edit_phrase(locale(cx), &error, fallback_key), true)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn register(&mut self, service: SwarmService, cx: &mut Context<Self>) {
        let new = NewDeployable {
            name: self.name.read(cx).value().trim().to_string(),
            stack: service.stack.clone(),
            service_name: service.name.clone(),
            image_repo: self.image_repo.read(cx).value().trim().to_string(),
            environment: self.environment.read(cx).value().trim().to_string(),
            short_tags: cac_core::uses_short_sha(&service.image),
        };
        let name = service.name;
        self.ask(
            cx,
            move |core| {
                Box::pin(async move {
                    core.platform().create_deployable(&new).await?;
                    core.platform().choose_service(&name).await;
                    Ok(())
                })
            },
            |_, _, _| {},
            "common:deploy.register",
        );
    }

    /// Desplegar lo que ya corre no trae nada: se pregunta (jose, 19-oct-2026:
    /// con la pantalla abierta desde antes de un merge, lo de arriba era lo
    /// que ya corría).
    fn deploy(&mut self, deployable: Deployable, sha: String, window: &mut Window, cx: &mut Context<Self>) {
        if cac_core::runs_commit(&deployable.current_image, &sha) {
            let short: String = sha.chars().take(7).collect();
            let title = tr(cx, "common:deploy.redeployTitle");
            let body = cac_core::text_with(locale(cx), "common:deploy.redeployBody", &[("sha", &short)]);
            let (confirm, cancel) = (tr(cx, "common:deploy.redeployConfirm"), tr(cx, "common:servers.cancel"));
            let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
            cx.spawn(async move |this, cx| {
                if answer.await == Ok(0) {
                    this.update(cx, |this, cx| this.deploy_now(deployable, sha, cx)).ok();
                }
            })
            .detach();
        } else {
            self.deploy_now(deployable, sha, cx);
        }
    }

    fn deploy_now(&mut self, deployable: Deployable, sha: String, cx: &mut Context<Self>) {
        self.ask(
            cx,
            move |core| {
                Box::pin(async move {
                    let started = core.platform().deploy(&deployable.id, &sha).await?;
                    core.platform().open_deploy_log(&started.id).await;
                    Ok(())
                })
            },
            |this, _, cx| this.line = Some((tr(cx, "common:deploy.queued"), false)),
            "common:deploy.deploy",
        );
    }

    fn rollback(&mut self, deployable: Deployable, deployment: Deployment, window: &mut Window, cx: &mut Context<Self>) {
        let title = tr(cx, "common:deploy.rollbackTitle");
        let body = cac_core::text_with(locale(cx), "common:deploy.rollbackBody", &[("image", short_ref(&deployment.previous_image))]);
        let (confirm, cancel) = (tr(cx, "common:deploy.rollback"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                this.ask(
                    cx,
                    move |core| {
                        Box::pin(async move {
                            let started = core.platform().rollback(&deployable.id, &deployment.id).await?;
                            core.platform().open_deploy_log(&started.id).await;
                            Ok(())
                        })
                    },
                    |_, _, _| {},
                    "common:deploy.rollback",
                )
            })
            .ok();
        })
        .detach();
    }

    fn services_list(&self, services: &[SwarmService], state: &PlatformState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, accent) = (theme.border, theme.muted_foreground, theme.accent);
        let deploys = state.deploys.as_ref();
        let chosen = deploys.and_then(|deploys| deploys.selected_service.clone());
        let registered = |name: &str| deploys.and_then(|deploys| deploys.deployables.as_ref()).is_some_and(|list| list.iter().any(|known| known.service_name == name));
        let rows = services.iter().map(|service| {
            let picked = service.clone();
            div()
                .id(SharedString::from(format!("deploy-service-{}", service.id)))
                .aria_label(service.name.clone())
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .border_t_1()
                .border_color(border)
                .cursor_pointer()
                .hover(|style| style.bg(accent.opacity(0.4)))
                .when(chosen.as_deref() == Some(service.name.as_str()), |row| row.bg(accent))
                .child(div().flex_1().min_w_0().truncate().child(service.name.clone()))
                .when(registered(&service.name), |row| row.child(Icon::new(Lucide::Rocket).size_3().text_color(muted)))
                .on_click(cx.listener(move |this, _, _, cx| this.choose(picked.clone(), cx)))
                .test_support()
        });
        div()
            .id("deploy-services")
            .w(px(LIST_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .leading_sm()
            .child(div().px_3().py_2().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, "common:deploy.pagePick")))
            .children(rows)
            .test_support()
            .into_any_element()
    }

    fn register_form(&self, service: &SwarmService, state: &PlatformState, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        if !state.permissions.manage_deploys {
            return div().id("deploy-not-yet").leading_sm().text_color(muted).child(tr(cx, "common:deploy.notYet")).test_support().into_any_element();
        }
        let field = |label: String, input: &Entity<InputState>, id: &'static str| div().id(id).flex().flex_col().gap_1().child(div().leading_xs().child(label)).child(Input::new(input)).test_support();
        let ready = !self.name.read(cx).value().trim().is_empty() && !self.image_repo.read(cx).value().trim().is_empty();
        let target = service.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().font_weight(FontWeight::MEDIUM).leading_sm().child(tr(cx, "common:deploy.notYet")))
            .child(div().leading_sm().text_color(muted).child(tr(cx, "common:deploy.notYetLead")))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().child(field(tr(cx, "common:deploy.name"), &self.name, "deploy-name")))
                    .child(div().flex_1().child(field(tr(cx, "common:deploy.environment"), &self.environment, "deploy-environment"))),
            )
            .child(field(tr(cx, "common:deploy.imageRepo"), &self.image_repo, "deploy-image-repo"))
            .child(
                div().flex().justify_end().child(
                    Button::new("deploy-register")
                        .primary()
                        .small()
                        .label(tr(cx, "common:deploy.register"))
                        .disabled(self.busy || !ready)
                        .on_click(cx.listener(move |this, _, _, cx| this.register(target.clone(), cx))),
                ),
            )
            .into_any_element()
    }

    fn commits(&self, deployable: &Deployable, state: &PlatformState, blocked: bool, deploying: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(deploys) = state.deploys.as_ref() else { return div().into_any_element() };
        let theme = cx.theme();
        let (border, muted, accent, mono) = (theme.border, theme.muted_foreground, theme.accent, theme.mono_font_family.clone());
        let pick = commit_pick(deploys.commits.as_deref().unwrap_or_default(), &deploys.builds, deployable, self.picked.as_deref());
        let can_deploy = state.permissions.deploy;
        let deploy_label = |cx: &App, label: String| if deploying { tr(cx, "common:deploy.deploying") } else { label };
        let typed = self.sha.read(cx).value().trim().to_lowercase();
        let manual_target = deployable.clone();
        let manual = div()
            .id("deploy-manual")
            .flex()
            .items_end()
            .gap_2()
            .child(div().flex_1().flex().flex_col().gap_1().child(div().leading_xs().child(tr(cx, "common:deploy.sha"))).child(Input::new(&self.sha)))
            .when(can_deploy, |row| {
                row.child(
                    Button::new("deploy-manual-go")
                        .primary()
                        .small()
                        .icon(Icon::new(Lucide::Rocket))
                        .label(deploy_label(cx, tr(cx, "common:deploy.deploy")))
                        .disabled(blocked || !cac_core::is_sha(&typed))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let sha = this.sha.read(cx).value().trim().to_lowercase();
                            this.deploy(manual_target.clone(), sha, window, cx)
                        })),
                )
            })
            .test_support();
        if pick.rows.is_empty() {
            let why = if deployable.repo_full_name.is_empty() {
                Some(tr(cx, "common:deploy.commitsNeedRepo"))
            } else {
                deploys.commits_error.as_ref().map(|reason| cac_core::text_with(locale(cx), "common:deploy.commitsFailed", &[("reason", reason)]))
            };
            return div()
                .flex()
                .flex_col()
                .gap_2()
                .children(why.map(|why| div().id("deploy-commits-why").leading_xs().text_color(muted).child(why).test_support()))
                .child(manual)
                .into_any_element();
        }
        let branch = if deployable.branch.is_empty() { tr(cx, "common:deploy.defaultBranch") } else { deployable.branch.clone() };
        let title = cac_core::text_with(locale(cx), "common:deploy.commits", &[("branch", &branch)]);
        let behind = (pick.behind > 0).then(|| cac_core::text_count(locale(cx), "common:deploy.behind", pick.behind as i64));
        let shown = if self.all_commits { pick.rows.len() } else { pick.rows.len().min(LIST_PREVIEW) };
        let rows = pick.rows.iter().take(shown).map(|row| {
            let sha = row.sha.clone();
            let short: String = row.sha.chars().take(7).collect();
            div()
                .id(SharedString::from(format!("deploy-commit-{short}")))
                .aria_label(row.message.clone())
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .border_t_1()
                .border_color(border)
                .cursor_pointer()
                .when(pick.chosen.as_deref() == Some(row.sha.as_str()), |this| this.bg(accent))
                .when(row.older, |this| this.text_color(muted))
                .child(div().font_family(mono.clone()).text_xs().child(short))
                .child(div().flex_1().min_w_0().truncate().child(row.message.clone()))
                .when(row.running, |this| this.child(pill(tr(cx, "common:deploy.running"), border, muted)))
                .when(row.published, |this| this.child(pill(tr(cx, "common:deploy.imagePublished"), theme.secondary, theme.secondary_foreground)))
                .child(div().flex_none().text_xs().text_color(muted).child(row.author.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.picked = Some(sha.clone());
                    cx.notify();
                }))
                .test_support()
        });
        let more = (pick.rows.len() > LIST_PREVIEW).then(|| {
            let label = if self.all_commits { tr(cx, "common:deploy.showLess") } else { cac_core::text_with(locale(cx), "common:deploy.showMore", &[("count", &(pick.rows.len() - LIST_PREVIEW).to_string())]) };
            Button::new("deploy-commits-more").link().xsmall().label(label).on_click(cx.listener(|this, _, _, cx| {
                this.all_commits = !this.all_commits;
                cx.notify();
            }))
        });
        let go = pick.chosen.clone().filter(|_| can_deploy).map(|chosen| {
            let short: String = chosen.chars().take(7).collect();
            let target = deployable.clone();
            let value = pick.value_of(&chosen, deployable);
            div().flex().justify_end().child(
                Button::new("deploy-go")
                    .primary()
                    .small()
                    .icon(Icon::new(Lucide::Rocket))
                    .label(deploy_label(cx, cac_core::text_with(locale(cx), "common:deploy.deployCommit", &[("sha", &short)])))
                    .disabled(blocked)
                    .on_click(cx.listener(move |this, _, window, cx| this.deploy(target.clone(), value.clone(), window, cx))),
            )
        });
        div()
            .id("deploy-commits")
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(div().flex_1().font_weight(FontWeight::MEDIUM).leading_sm().child(title).children(behind.map(|behind| div().id("deploy-behind").aria_label(behind.clone()).text_xs().text_color(muted).child(behind).test_support())))
                    .child(Button::new("deploy-commits-refresh").ghost().xsmall().icon(Icon::new(Lucide::RefreshCw)).tooltip(tr(cx, "common:deploy.refreshCommits")).on_click(|_, _, cx| {
                        Backend::run(cx, |core| async move { core.platform().load_commits().await })
                    }))
                    .child(Button::new("deploy-other-commit").link().xsmall().label(tr(cx, "common:deploy.otherCommit")).on_click(cx.listener(|this, _, _, cx| {
                        this.manual = !this.manual;
                        cx.notify();
                    }))),
            )
            .child(div().flex().flex_col().rounded(px(8.)).border_1().border_color(border).leading_sm().children(rows))
            .children(more)
            .children(go)
            .when(self.manual, |this| this.child(manual))
            .test_support()
            .into_any_element()
    }

    fn history(&self, deployable: &Deployable, state: &PlatformState, blocked: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(deploys) = state.deploys.as_ref() else { return div().into_any_element() };
        let theme = cx.theme();
        let (border, muted, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
        if deploys.history.is_empty() {
            return div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(FontWeight::MEDIUM).leading_sm().child(tr(cx, "common:deploy.history")))
                .child(empty("deploy-history-empty", tr(cx, "common:deploy.empty"), muted))
                .into_any_element();
        }
        let shown = if self.all_history { deploys.history.len() } else { deploys.history.len().min(LIST_PREVIEW) };
        let rows = deploys.history.iter().take(shown).map(|deployment| {
            let (status_bg, status_fg) = status_colors(deployment.status, cx);
            let who = who(deployment, cx);
            let when = if deployment.created_at.is_empty() { String::new() } else { cac_core::time_ago(&deployment.created_at, &chrono::Local::now(), locale(cx)) };
            let detail = [Some(who), Some(when), (!deployment.error.is_empty()).then(|| deployment.error.clone())].into_iter().flatten().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
            let opened = deployment.id.clone();
            let back = (state.permissions.deploy && deployment.status == DeployStatus::Succeeded && !deployment.previous_image.is_empty()).then(|| {
                let (target, deployment) = (deployable.clone(), deployment.clone());
                Button::new(SharedString::from(format!("deploy-rollback-{}", deployment.id)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Lucide::Undo2))
                    .tooltip(tr(cx, "common:deploy.rollback"))
                    .disabled(blocked)
                    .on_click(cx.listener(move |this, _, window, cx| this.rollback(target.clone(), deployment.clone(), window, cx)))
            });
            div()
                .id(SharedString::from(format!("deploy-row-{}", deployment.id)))
                .aria_label(tr(cx, deployment.status.label_key()))
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .border_t_1()
                .border_color(border)
                .child(pill(tr(cx, deployment.status.label_key()), status_bg, status_fg))
                .child(div().font_family(mono.clone()).text_xs().child(short_ref(if deployment.final_image.is_empty() { &deployment.image } else { &deployment.final_image }).to_string()))
                .child(div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(detail))
                .child(
                    Button::new(SharedString::from(format!("deploy-log-{}", deployment.id)))
                        .ghost()
                        .xsmall()
                        .label(tr(cx, "common:deploy.viewLog"))
                        .on_click(move |_, _, cx| {
                            let id = opened.clone();
                            Backend::run(cx, move |core| async move { core.platform().open_deploy_log(&id).await });
                        }),
                )
                .children(back)
                .test_support()
        });
        let more = (deploys.history.len() > LIST_PREVIEW).then(|| {
            let label = if self.all_history { tr(cx, "common:deploy.showLess") } else { cac_core::text_with(locale(cx), "common:deploy.showMore", &[("count", &(deploys.history.len() - LIST_PREVIEW).to_string())]) };
            Button::new("deploy-history-more").link().xsmall().label(label).on_click(cx.listener(|this, _, _, cx| {
                this.all_history = !this.all_history;
                cx.notify();
            }))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().font_weight(FontWeight::MEDIUM).leading_sm().child(tr(cx, "common:deploy.history")))
            .child(div().id("deploy-history").flex().flex_col().rounded(px(8.)).border_1().border_color(border).leading_sm().children(rows).test_support())
            .children(more)
            .into_any_element()
    }

    fn log(&self, state: &PlatformState, cx: &mut Context<Self>) -> Option<AnyElement> {
        let deploys = state.deploys.as_ref()?;
        let id = deploys.log_of.as_ref()?;
        let shown = deploys.history.iter().find(|deployment| &deployment.id == id);
        let theme = cx.theme();
        let (muted, danger, mono) = (theme.muted_foreground, theme.danger, theme.mono_font_family.clone());
        let lines: Vec<LogLine> = deploys.log.lines().map(|line| LogLine { spans: cac_core::ansi_spans(line) }).collect();
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(Button::new("deploy-log-back").ghost().xsmall().icon(Icon::new(Lucide::ArrowLeft)).label(tr(cx, "common:deploy.backToHistory")).on_click(|_, _, cx| Backend::core(cx).platform().close_deploy_log()))
            .children(shown.map(|deployment| {
                let (bg, fg) = status_colors(deployment.status, cx);
                div().id("deploy-log-status").aria_label(tr(cx, deployment.status.label_key())).child(pill(tr(cx, deployment.status.label_key()), bg, fg)).test_support()
            }))
            .children(shown.map(|deployment| div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(who(deployment, cx))));
        let error = shown.filter(|deployment| !deployment.error.is_empty()).map(|deployment| div().leading_xs().text_color(danger).child(deployment.error.clone()));
        Some(
            div()
                .id("deploy-log-panel")
                .flex()
                .flex_col()
                .gap_2()
                .child(header)
                .children(error)
                .child(self.logs.render("deploy-log", &lines, "…".to_string(), mono))
                .test_support()
                .into_any_element(),
        )
    }

    fn detail(&mut self, server: &Server, service: &SwarmService, state: &PlatformState, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, warning, danger, mono) = (theme.muted_foreground, theme.warning, theme.danger, theme.mono_font_family.clone());
        let deploys = state.deploys.as_ref();
        let deployable = deploys.and_then(|deploys| deploys.deployables.as_ref()).and_then(|list| list.iter().find(|known| known.service_name == service.name)).cloned();
        let no_agent = !server.has_agent_token || server.agent_version < AGENT_VERSION_DEPLOYS;
        let deploying = deploys.is_some_and(|deploys| deploys.history.iter().any(|deployment| !deployment.status.finished()));
        let blocked = self.busy || deploying || no_agent;
        let line = self.line.clone().map(|(phrase, failed)| div().id("deploy-result").aria_label(phrase.clone()).leading_xs().text_color(if failed { danger } else { muted }).child(phrase).test_support());
        let warning_line = no_agent.then(|| div().id("deploy-no-agent").p_2().rounded(px(6.)).border_1().border_color(warning.opacity(0.4)).bg(warning.opacity(0.1)).leading_sm().child(tr(cx, "common:deploy.noAgent")).test_support());
        let body = match &deployable {
            None if deploys.and_then(|deploys| deploys.deployables.as_ref()).is_none() => empty("deploy-loading", tr(cx, "common:servers.loading"), muted),
            None => self.register_form(service, state, cx),
            Some(deployable) => {
                let ready = deploys.and_then(|deploys| deploys.selected.as_deref()) == Some(deployable.id.as_str());
                if !ready {
                    empty("deploy-loading", tr(cx, "common:servers.loading"), muted)
                } else {
                    let settings = state.permissions.manage_deploys.then(|| {
                        if self.settings.as_ref().is_none_or(|view| view.read(cx).deployable_id() != deployable.id) {
                            let target = deployable.clone();
                            self.settings = Some(cx.new(|cx| DeploySettingsView::new(&target, window, cx)));
                        }
                        self.settings.clone().map(|view| view.into_any_element())
                    });
                    div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(self.commits(deployable, state, blocked, deploying, cx))
                        .child(match self.log(state, cx) {
                            Some(log) => log,
                            None => self.history(deployable, state, blocked, cx),
                        })
                        .children(settings.flatten())
                        .child({
                            if self.secrets.as_ref().is_none_or(|view| view.read(cx).deployable_id() != deployable.id) {
                                let target = deployable.clone();
                                self.secrets = Some(cx.new(|cx| SecretRefsView::new(&target, window, cx)));
                            }
                            self.secrets.clone().map(|view| view.into_any_element()).unwrap_or_else(|| div().into_any_element())
                        })
                        .into_any_element()
                }
            }
        };
        let now = deployable.as_ref().map(|deployable| {
            let image = if deployable.current_image.is_empty() { &service.image } else { &deployable.current_image };
            div().id("deploy-now").font_family(mono).text_xs().text_color(muted).child(cac_core::text_with(locale(cx), "common:deploy.now", &[("image", image)])).test_support()
        });
        div()
            .id("deploy-detail")
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().font_family(cx.theme().mono_font_family.clone()).leading_sm().child(service.name.clone()))
            .children(now)
            .children(warning_line)
            .children(line)
            .child(body)
            .test_support()
            .into_any_element()
    }
}

pub(crate) fn who(deployment: &Deployment, cx: &App) -> String {
    match deployment.requested_by.as_str() {
        "ci" => tr(cx, "common:deploy.byCI"),
        "github" => tr(cx, "common:deploy.byGitHub"),
        _ => deployment.requested_by_name.clone().unwrap_or_default(),
    }
}

pub(crate) fn status_colors(status: DeployStatus, cx: &App) -> (Hsla, Hsla) {
    let theme = cx.theme();
    match status {
        DeployStatus::Succeeded => (theme.success.opacity(0.15), theme.success),
        DeployStatus::Failed => (theme.danger.opacity(0.15), theme.danger),
        DeployStatus::Running => (theme.secondary, theme.secondary_foreground),
        _ => (theme.border, theme.muted_foreground),
    }
}

impl Render for DeployView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let muted = cx.theme().muted_foreground;
        let Some(server) = state.open_server.as_ref().and_then(|id| state.servers.iter().find(|server| &server.id == id)).cloned() else {
            return div().into_any_element();
        };
        let services = state.services.clone().unwrap_or_default();
        let error = state.deploys.as_ref().and_then(|deploys| deploys.error.clone()).map(|error| div().id("deploy-error").leading_xs().text_color(cx.theme().danger).child(error).test_support());
        let chosen = state.deploys.as_ref().and_then(|deploys| deploys.selected_service.clone()).and_then(|name| services.iter().find(|service| service.name == name).cloned());
        if let Some(service) = chosen.as_ref().filter(|service| self.prefilled_for.as_ref() != Some(&service.name)) {
            self.prefill(service, window, cx);
        }
        let detail = match &chosen {
            Some(service) => self.detail(&server, service, &state, window, cx),
            None => div().flex_1().child(empty("deploy-lead", tr(cx, "common:deploy.pageLead"), muted)).into_any_element(),
        };
        div()
            .id("server-deploy")
            .flex()
            .flex_col()
            .gap_3()
            .children(error)
            .child(div().flex().items_start().gap_4().child(self.services_list(&services, &state, cx)).child(detail))
            .test_support()
            .into_any_element()
    }
}
