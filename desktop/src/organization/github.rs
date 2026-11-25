//! GitHub (`OrgGitHub.tsx`): qué cuentas tienen instalada la App de esta
//! organización y a qué espacio va cada repo.
//!
//! Un repo enlazado a un espacio deja una línea en la tarea que nombre cada
//! commit o PR; las líneas son internas. Un repo sin enlazar no hace nada, y
//! así empieza cada uno. Qué repos salen, cuáles se pueden enlazar y cómo
//! va la copia de sus docs lo decide el core (`cac_core::org_github`).

use cac_core::{bare_refs_enabled, docs_line, linkable_spaces, shown_repos, text_with, GitHubRepo, GitHubStatus, GitHubUnbound, RepoChange};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::pickers::{choice_chip, Choice};
use super::OrgView;
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

#[derive(Default)]
pub(super) struct GitHubTab {
    status: Option<GitHubStatus>,
    loading: bool,
    /// El repo cuyos docs se están sincronizando.
    syncing: Option<String>,
}

impl GitHubTab {
    fn replace(&mut self, saved: GitHubRepo) {
        if let Some(slot) = self.status.as_mut().and_then(|status| status.repos.iter_mut().find(|repo| repo.id == saved.id)) {
            *slot = saved;
        }
    }
}

impl OrgView {
    /// Lo que GitHub sabe de esta organización. Sin vaciar lo que ya se ve.
    pub(super) fn load_github(&mut self, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        self.github.loading = true;
        let id = org.id.clone();
        let answer = Backend::ask(cx, move |core| async move { (id.clone(), core.orgs().github_status(&id).await) });
        cx.spawn(async move |this, cx| {
            let Ok((id, status)) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.loaded_for.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.github.loading = false;
                match status {
                    Ok(status) => this.github.status = Some(status),
                    Err(error) => {
                        // Sin esto la pestaña se quedaría leyendo para siempre.
                        this.github.status.get_or_insert_with(GitHubStatus::default);
                        let text = crate::lang::error_text(cx, &error, "org:github.loadFailed");
                        this.say(text, true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Abre en el navegador la instalación de la App: la dirección, con su
    /// `state`, la da el servidor.
    fn connect_github(&mut self, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = None;
        cx.notify();
        let id = org.id.clone();
        let asked_for = self.loaded_for.clone();
        let answer = Backend::ask(cx, move |core| async move { core.orgs().github_link(&id).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                if this.loaded_for != asked_for {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(url) => {
                        cx.open_url(&url);
                        this.say(tr(cx, "org:github.finishInBrowser"), false, cx);
                    }
                    Err(error) => {
                        let text = crate::lang::error_text(cx, &error, "org:github.connectFailed");
                        this.say(text, true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn update_repo(&mut self, repo: GitHubRepo, change: RepoChange, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let id = org.id.clone();
        let asked_for = self.loaded_for.clone();
        let answer = Backend::ask(cx, move |core| async move { core.orgs().github_update_repo(&id, &repo, &change).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.loaded_for != asked_for {
                    return;
                }
                match result {
                    Ok(saved) => this.github.replace(saved),
                    Err(error) => {
                        let text = crate::lang::error_text(cx, &error, "org:github.saveFailed");
                        this.say(text, true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Los docs del repo, a mano: por si se perdió un push.
    fn sync_repo_docs(&mut self, repo: &GitHubRepo, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        if self.github.syncing.is_some() {
            return;
        }
        self.github.syncing = Some(repo.id.clone());
        cx.notify();
        let (id, repo_id) = (org.id.clone(), repo.id.clone());
        let asked_for = self.loaded_for.clone();
        let answer = Backend::ask(cx, move |core| async move { core.orgs().github_sync_docs(&id, &repo_id).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.github.syncing = None;
                if this.loaded_for != asked_for {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(saved) => {
                        this.github.replace(saved);
                        this.say(tr(cx, "work:repoDocs.synced_ok"), false, cx);
                    }
                    Err(_) => this.say(tr(cx, "work:repoDocs.syncFailed"), true, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Ata una instalación sin organización a ésta, preguntando antes y con
    /// su cuenta y sus repos a la vista: una vez atada no se la lleva otra.
    fn ask_adopt(&mut self, installation: GitHubUnbound, window: &mut Window, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let lang = locale(cx);
        let repos = if installation.repos.is_empty() { "—".to_string() } else { installation.repos.join(", ") };
        let title = text_with(lang, "org:github.adoptTitle", &[("account", &installation.label())]);
        let body = text_with(lang, "org:github.adoptBody", &[("repos", &repos)]);
        let (confirm, cancel) = (tr(cx, "org:github.adopt"), tr(cx, "org:cancel"));
        let answer = window.prompt(PromptLevel::Info, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                let (id, installation_id, account) = (org.id.clone(), installation.installation_id, installation.label());
                this.act(
                    move |core| async move { core.orgs().github_adopt(&id, installation_id).await },
                    move |this, cx| {
                        let text = text_with(locale(cx), "org:github.adopted", &[("account", &account)]);
                        this.say(text, false, cx);
                        this.load_github(cx);
                    },
                    "org:github.adoptFailed",
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    fn render_repo(&self, repo: &GitHubRepo, manage: bool, spaces: &[(String, String)], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
        let id = repo.id.clone();
        let lang = locale(cx);
        let space_label = spaces.iter().find(|(space_id, _)| *space_id == repo.space_id).map_or_else(|| tr(cx, "org:github.notLinked"), |(_, name)| name.clone());
        let mut choices = vec![Choice::option("", tr(cx, "org:github.notLinked"))];
        choices.extend(spaces.iter().map(|(space_id, name)| Choice::option(space_id.clone(), name.clone())));
        let view = cx.entity().downgrade();
        let target = repo.clone();
        let space_picker = choice_chip(SharedString::from(format!("org-github-space-{id}")), space_label, choices, repo.space_id.clone(), !manage, move |value, _, cx| {
            let (repo, change) = (target.clone(), RepoChange { space_id: Some(value), ..Default::default() });
            view.update(cx, |this, cx| this.update_repo(repo, change, cx)).ok();
        });

        let bare = {
            let target = repo.clone();
            Checkbox::new(SharedString::from(format!("org-github-bare-{id}")))
                .small()
                .checked(repo.bare_refs)
                .label(tr(cx, "org:github.bareRefs"))
                .disabled(!bare_refs_enabled(repo, manage))
                .tooltip(tr(cx, "org:github.bareRefsHint"))
                .on_click(cx.listener(move |this, checked: &bool, _, cx| this.update_repo(target.clone(), RepoChange { bare_refs: Some(*checked), ..Default::default() }, cx)))
        };

        // Sus .md en Documentación (docs as code): sólo con el repo atado.
        let docs = (!repo.space_id.is_empty()).then(|| {
            let target = repo.clone();
            let syncing = self.github.syncing.as_deref() == Some(repo.id.as_str());
            let line = docs_line(repo, &chrono::Local::now(), lang);
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .text_color(muted)
                .child(
                    Checkbox::new(SharedString::from(format!("org-github-docs-{id}")))
                        .small()
                        .checked(repo.show_docs)
                        .label(tr(cx, "work:repoDocs.showDocs"))
                        .disabled(!manage)
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| this.update_repo(target.clone(), RepoChange { show_docs: Some(*checked), ..Default::default() }, cx))),
                )
                .when(repo.show_docs, |this| {
                    let target = repo.clone();
                    this.child(div().id(SharedString::from(format!("org-github-docs-state-{id}"))).aria_label(line.clone()).child(line).test_support()).when(manage, |this| {
                        this.child(
                            Button::new(SharedString::from(format!("org-github-sync-{id}")))
                                .ghost()
                                .small()
                                .icon(Icon::new(Lucide::RefreshCw))
                                .label(tr(cx, "work:repoDocs.sync"))
                                .disabled(syncing)
                                .on_click(cx.listener(move |this, _, _, cx| this.sync_repo_docs(&target, cx))),
                        )
                    })
                })
        });

        div()
            .id(SharedString::from(format!("org-github-repo-{id}")))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(div().flex_1().min_w_0().truncate().font_family(mono).text_xs().child(repo.full_name.clone()))
            .child(space_picker)
            .child(bare)
            .children(docs)
            .test_support()
            .into_any_element()
    }

    pub(super) fn render_github(&mut self, manage: bool, cx: &mut Context<Self>) -> AnyElement {
        if self.github.status.is_none() && !self.github.loading {
            self.load_github(cx);
        }
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let Some(status) = self.github.status.clone() else {
            return div().id("org-github-loading").py_8().text_center().text_color(muted).child(tr(cx, "common:servers.loading")).test_support().into_any_element();
        };
        if !status.configured {
            return div().id("org-github-off").text_color(muted).child(tr(cx, "org:github.notConfigured")).test_support().into_any_element();
        }
        let Some((org, _)) = Self::current(cx) else { return div().into_any_element() };
        let spaces = Self::current_spaces(cx).map(|spaces| linkable_spaces(&spaces, &org.id)).unwrap_or_default();
        let query = self.repo_filter.read(cx).value().to_string();
        let repos = shown_repos(&status.repos, &query);
        let busy = self.busy;

        let installations = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .children(status.installations.iter().map(|installation| {
                div()
                    .id(SharedString::from(format!("org-github-installation-{}", installation.id)))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_0p5()
                    .rounded(px(6.))
                    .bg(theme.muted)
                    .text_size(px(12.))
                    .child(Icon::new(Lucide::GitBranch).size_3())
                    .child(installation.label())
                    .test_support()
            }))
            .when(status.installations.is_empty(), |this| this.child(div().text_color(muted).child(tr(cx, "org:github.noInstallations"))))
            .child(div().flex_1())
            .child(Button::new("org-github-reload").ghost().small().icon(Icon::new(Lucide::RefreshCw)).tooltip(tr(cx, "org:github.reload")).on_click(cx.listener(|this, _, _, cx| this.load_github(cx))))
            .when(manage, |this| {
                this.child(
                    Button::new("org-github-connect")
                        .outline()
                        .small()
                        .icon(Icon::new(Lucide::ExternalLink))
                        .label(tr(cx, if status.installations.is_empty() { "org:github.connect" } else { "org:github.connectAnother" }))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.connect_github(cx))),
                )
            });

        let unbound = (manage && !status.unbound.is_empty()).then(|| {
            div()
                .id("org-github-unbound")
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(6.))
                .border_1()
                .border_dashed()
                .border_color(border)
                .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "org:github.unboundTitle")))
                .child(div().leading_xs().text_color(muted).child(tr(cx, "org:github.unboundLead")))
                .children(status.unbound.iter().map(|installation| {
                    let target = installation.clone();
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(Icon::new(Lucide::GitBranch).size_3())
                        .child(div().font_weight(FontWeight::MEDIUM).child(installation.label()))
                        .child(div().flex_1().min_w_0().truncate().font_family(cx.theme().mono_font_family.clone()).text_xs().text_color(muted).child(installation.repos.join(", ")))
                        .child(
                            Button::new(SharedString::from(format!("org-github-adopt-{}", installation.installation_id)))
                                .outline()
                                .small()
                                .label(tr(cx, "org:github.adopt"))
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, window, cx| this.ask_adopt(target.clone(), window, cx))),
                        )
                }))
        });

        div()
            .id("org-github")
            .max_w(px(768.))
            .flex()
            .flex_col()
            .gap_4()
            .child(div().flex().flex_col().gap_1().child(div().text_color(muted).child(tr(cx, "org:github.lead"))).child(div().leading_xs().text_color(muted).child(tr(cx, "org:github.syntax"))))
            .child(installations)
            .children(unbound)
            // Una cuenta entera trae decenas de repos: se busca por nombre.
            .when(!status.repos.is_empty(), |this| this.child(div().max_w(px(384.)).child(Input::new(&self.repo_filter).id("org-github-filter"))))
            .when(!status.repos.is_empty() && repos.is_empty(), |this| this.child(div().id("org-github-no-match").text_color(muted).child(tr(cx, "org:github.noMatch")).test_support()))
            .when(!repos.is_empty(), |this| {
                this.child(div().id("org-github-repos").flex().flex_col().rounded(px(6.)).border_1().border_color(border).children(repos.iter().map(|repo| self.render_repo(repo, manage, &spaces, cx))).test_support())
            })
            .into_any_element()
    }
}
