//! Basecamp: conectar la cuenta de la organización, elegir qué proyectos van a
//! qué espacio y asignar a mano a las personas que no casaron por correo.
//!
//! Sólo la ve quien administra. El servidor hace todo el trabajo y esta
//! pantalla lo pide y dice cómo fue; qué se ofrece y a quién lo decide el
//! core (`cac_core::org_basecamp`). Lo que se pinta es siempre lo que el
//! servidor dijo la última vez, así que cada cambio termina volviendo a
//! leerlo.

use cac_core::{assignable_members, linkable_spaces, ordered_projects, text_with, unmatched_people, ApiError, BasecampAvailable, BasecampPerson, BasecampStatus};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::pickers::{choice_chip, Choice};
use super::OrgView;
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Lo que se leyó de Basecamp. Cada lista trae su propio error: una que
/// falla no tapa a las demás.
#[derive(Default)]
pub(super) struct BasecampTab {
    status: Option<BasecampStatus>,
    /// Por qué no se pudo leer el estado la primera vez: sin estado no hay
    /// qué pintar, y «no configurado» sería mentira.
    status_error: Option<String>,
    loading: bool,
    projects: Option<Result<Vec<BasecampAvailable>, String>>,
    people: Option<Result<Vec<BasecampPerson>, String>>,
    /// La persona a la que se le está eligiendo miembro.
    assigning: Option<i64>,
    /// Se abrió el navegador a autorizar: al volver a la ventana se vuelve
    /// a leer el estado.
    awaiting_return: bool,
}

impl BasecampTab {
    pub(super) fn awaiting_return(&self) -> bool {
        self.awaiting_return
    }

    pub(super) fn stop_waiting(&mut self) {
        self.awaiting_return = false;
    }
}

impl OrgView {
    /// Lo que Basecamp sabe de esta organización, sin vaciar lo que ya se ve.
    /// Con una cuenta conectada trae también sus proyectos y su gente.
    pub(super) fn load_basecamp(&mut self, cx: &mut Context<Self>) {
        let Some((org, manage)) = Self::current(cx) else { return };
        if !manage {
            return;
        }
        self.basecamp.loading = true;
        self.basecamp.status_error = None;
        let id = org.id.clone();
        let answer = Backend::ask(cx, move |core| async move {
            let orgs = core.orgs();
            let status = orgs.basecamp_status(&id).await;
            let (projects, people) = match &status {
                Ok(status) if status.connected => (Some(orgs.basecamp_projects(&id).await), Some(orgs.basecamp_people(&id).await)),
                _ => (None, None),
            };
            (id, status, projects, people)
        });
        cx.spawn(async move |this, cx| {
            let Ok((id, status, projects, people)) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.loaded_for.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.basecamp.loading = false;
                let failed = |cx: &App, error: &ApiError, key: &'static str| crate::lang::error_text(cx, error, key);
                match status {
                    Ok(status) => {
                        this.basecamp.projects = projects.map(|result| result.map_err(|error| failed(cx, &error, "org:basecamp.projectsFailed")));
                        this.basecamp.people = people.map(|result| result.map_err(|error| failed(cx, &error, "org:basecamp.peopleFailed")));
                        if !status.connected {
                            this.basecamp.assigning = None;
                        }
                        this.basecamp.status = Some(status);
                    }
                    Err(error) => {
                        let text = failed(cx, &error, "org:basecamp.loadFailed");
                        if this.basecamp.status.is_some() {
                            this.say(text, true, cx);
                        } else {
                            this.basecamp.status_error = Some(text);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Abre en el navegador la autorización de Basecamp: la dirección, con su
    /// `state`, la da el servidor.
    fn connect_basecamp(&mut self, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let id = org.id.clone();
        self.act_with(
            move |core| async move { core.orgs().basecamp_connect_url(&id).await },
            |this, url, cx| {
                cx.open_url(&url);
                this.basecamp.awaiting_return = true;
                this.say(tr(cx, "org:basecamp.finishInBrowser"), false, cx);
            },
            "org:basecamp.connectFailed",
            cx,
        );
    }

    /// Desconecta preguntando antes: quita los webhooks y los proyectos
    /// enlazados, y volver a tenerlos cuesta enlazarlos de nuevo.
    fn ask_disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let (title, body) = (tr(cx, "org:basecamp.disconnectTitle"), tr(cx, "org:basecamp.disconnectBody"));
        let (confirm, cancel) = (tr(cx, "org:basecamp.disconnect"), tr(cx, "org:cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                let id = org.id.clone();
                this.act(
                    move |core| async move { core.orgs().basecamp_disconnect(&id).await },
                    |this, cx| {
                        this.basecamp = Default::default();
                        this.say(tr(cx, "org:basecamp.disconnected"), false, cx);
                        this.load_basecamp(cx);
                    },
                    "org:basecamp.disconnectFailed",
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// Enlaza el proyecto al espacio, o con el espacio vacío lo quita.
    /// Dice lo que devolvió el servidor, y el webhook que no se dio de alta
    /// es un problema, no un éxito.
    fn pick_project_space(&mut self, project: BasecampAvailable, space_id: String, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        if space_id == project.space_id {
            return;
        }
        let (id, project_id, name) = (org.id.clone(), project.id, project.name.clone());
        if space_id.is_empty() {
            self.act(
                move |core| async move { core.orgs().basecamp_unlink_project(&id, project_id).await },
                move |this, cx| {
                    let text = text_with(locale(cx), "org:basecamp.unlinked", &[("project", &name)]);
                    this.say(text, false, cx);
                    this.load_basecamp(cx);
                },
                "org:basecamp.unlinkFailed",
                cx,
            );
            return;
        }
        let space_name = Self::current_spaces(cx).and_then(|spaces| linkable_spaces(&spaces, &org.id).into_iter().find(|(candidate, _)| *candidate == space_id).map(|(_, name)| name)).unwrap_or_default();
        self.act_with(
            move |core| async move { core.orgs().basecamp_link_project(&id, project_id, &space_id).await },
            move |this, linked, cx| {
                let key = if linked.has_webhook() { "org:basecamp.linked" } else { "org:basecamp.linkedNoWebhook" };
                let text = text_with(locale(cx), key, &[("project", &name), ("space", &space_name)]);
                this.say(text, !linked.has_webhook(), cx);
                this.load_basecamp(cx);
            },
            "org:basecamp.linkFailed",
            cx,
        );
    }

    fn start_assigning(&mut self, person_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.basecamp.assigning = Some(person_id);
        self.assign_query.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.notice = None;
        cx.notify();
    }

    fn assign_person(&mut self, person: BasecampPerson, member: cac_core::OrgMember, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let (id, person_id, member_id) = (org.id.clone(), person.person_id, member.user_id.clone());
        let (person_name, member_name) = (person.name.clone(), cac_core::member_face(&member).0);
        self.act(
            move |core| async move { core.orgs().basecamp_match_person(&id, person_id, &member_id).await.map(|_| ()) },
            move |this, cx| {
                this.basecamp.assigning = None;
                let text = text_with(locale(cx), "org:basecamp.assigned", &[("person", &person_name), ("member", &member_name)]);
                this.say(text, false, cx);
                this.load_basecamp(cx);
            },
            "org:basecamp.assignFailed",
            cx,
        );
    }

    fn render_basecamp_account(&self, status: &BasecampStatus, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, danger) = (theme.border, theme.muted_foreground, theme.danger);
        let busy = self.busy;
        let lang = locale(cx);
        let line = if status.connected {
            let who = if status.identity_name.is_empty() { "—" } else { status.identity_name.as_str() };
            text_with(lang, "org:basecamp.connectedTo", &[("account", &status.account_name), ("identity", who)])
        } else {
            tr(cx, "org:basecamp.notConnected")
        };
        div()
            .id("org-basecamp-account")
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(6.))
            .border_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().id("org-basecamp-line").flex_1().min_w_0().font_weight(FontWeight::MEDIUM).aria_label(line.clone()).child(line).test_support())
                    .child(Button::new("org-basecamp-reload").ghost().small().icon(Icon::new(Lucide::RefreshCw)).tooltip(tr(cx, "org:basecamp.reload")).on_click(cx.listener(|this, _, _, cx| this.load_basecamp(cx))))
                    .when(status.offers_connect(true), |this| {
                        this.child(
                            Button::new("org-basecamp-connect")
                                .outline()
                                .small()
                                .icon(Icon::new(Lucide::ExternalLink))
                                .label(tr(cx, if status.needs_reconnect() { "org:basecamp.reconnect" } else { "org:basecamp.connect" }))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.connect_basecamp(cx))),
                        )
                    })
                    .when(status.offers_disconnect(true), |this| {
                        this.child(Button::new("org-basecamp-disconnect").outline().small().label(tr(cx, "org:basecamp.disconnect")).disabled(busy).on_click(cx.listener(|this, _, window, cx| this.ask_disconnect(window, cx))))
                    }),
            )
            .when(status.needs_reconnect(), |this| {
                let text = tr(cx, "org:basecamp.reconnectNeeded");
                this.child(div().id("org-basecamp-reconnect").leading_xs().text_color(danger).aria_label(text.clone()).child(text).test_support())
            })
            .when(!status.connected, |this| this.child(div().leading_xs().text_color(muted).child(tr(cx, "org:basecamp.lead"))))
            .into_any_element()
    }

    fn render_basecamp_projects(&self, status: &BasecampStatus, manage_spaces: &[(String, String)], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, danger) = (theme.border, theme.muted_foreground, theme.danger);
        let title = div().font_weight(FontWeight::MEDIUM).child(tr(cx, "org:basecamp.projectsTitle"));
        let lead = div().leading_xs().text_color(muted).child(tr(cx, "org:basecamp.projectsLead"));
        let body: AnyElement = match &self.basecamp.projects {
            None => div().id("org-basecamp-projects-loading").text_color(muted).child(tr(cx, "org:basecamp.loading")).test_support().into_any_element(),
            Some(Err(text)) => div().id("org-basecamp-projects-error").text_color(danger).child(text.clone()).test_support().into_any_element(),
            Some(Ok(projects)) if projects.is_empty() => div().id("org-basecamp-projects-none").text_color(muted).child(tr(cx, "org:basecamp.projectsNone")).test_support().into_any_element(),
            Some(Ok(projects)) => div()
                .id("org-basecamp-projects")
                .flex()
                .flex_col()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .children(ordered_projects(projects).into_iter().map(|project| {
                    let space_label = manage_spaces.iter().find(|(id, _)| *id == project.space_id).map_or_else(|| tr(cx, "org:basecamp.notLinked"), |(_, name)| name.clone());
                    let mut choices = vec![Choice::option("", tr(cx, "org:basecamp.notLinked"))];
                    choices.extend(manage_spaces.iter().map(|(id, name)| Choice::option(id.clone(), name.clone())));
                    let view = cx.entity().downgrade();
                    let target = project.clone();
                    let picker = choice_chip(SharedString::from(format!("org-basecamp-space-{}", project.id)), space_label, choices, project.space_id.clone(), self.busy, move |value, _, cx| {
                        let target = target.clone();
                        view.update(cx, |this, cx| this.pick_project_space(target, value, cx)).ok();
                    });
                    // El webhook lo dice el servidor al leer el estado.
                    let linked = status.projects.iter().find(|linked| linked.project_id == project.id);
                    let hook = linked.map(|linked| if linked.has_webhook() { (tr(cx, "org:basecamp.webhookOk"), muted) } else { (tr(cx, "org:basecamp.webhookMissing"), danger) });
                    div()
                        .id(SharedString::from(format!("org-basecamp-project-{}", project.id)))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(border)
                        .child(div().flex_1().min_w_0().truncate().child(project.name.clone()))
                        .children(hook.map(|(text, color)| div().id(SharedString::from(format!("org-basecamp-hook-{}", project.id))).text_xs().text_color(color).aria_label(text.clone()).child(text).test_support()))
                        .child(picker)
                        .test_support()
                }))
                .test_support()
                .into_any_element(),
        };
        div().id("org-basecamp-projects-section").flex().flex_col().gap_2().child(title).child(lead).child(body).into_any_element()
    }

    fn render_basecamp_people(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, danger) = (theme.border, theme.muted_foreground, theme.danger);
        let title = div().font_weight(FontWeight::MEDIUM).child(tr(cx, "org:basecamp.peopleTitle"));
        let lead = div().leading_xs().text_color(muted).child(tr(cx, "org:basecamp.peopleLead"));
        let body: AnyElement = match &self.basecamp.people {
            None => div().id("org-basecamp-people-loading").text_color(muted).child(tr(cx, "org:basecamp.loading")).test_support().into_any_element(),
            Some(Err(text)) => div().id("org-basecamp-people-error").text_color(danger).child(text.clone()).test_support().into_any_element(),
            Some(Ok(people)) => {
                let waiting = unmatched_people(people);
                if waiting.is_empty() {
                    div().id("org-basecamp-people-none").text_color(muted).child(tr(cx, "org:basecamp.peopleNone")).test_support().into_any_element()
                } else {
                    let rows: Vec<AnyElement> = waiting.into_iter().map(|person| self.render_basecamp_person(person, cx)).collect();
                    div().id("org-basecamp-people").flex().flex_col().rounded(px(6.)).border_1().border_color(border).children(rows).test_support().into_any_element()
                }
            }
        };
        div().id("org-basecamp-people-section").flex().flex_col().gap_2().child(title).child(lead).child(body).into_any_element()
    }

    fn render_basecamp_person(&self, person: BasecampPerson, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, hover_bg) = (theme.border, theme.muted_foreground, theme.muted);
        let id = person.person_id;
        let open = self.basecamp.assigning == Some(id);
        let busy = self.busy;
        let target = person.clone();
        let picker = open.then(|| {
            let query = self.assign_query.read(cx).value().to_string();
            let members = assignable_members(self.members.as_deref().unwrap_or_default(), &query);
            let title = text_with(locale(cx), "org:basecamp.assignTo", &[("person", &person.name)]);
            div()
                .id(SharedString::from(format!("org-basecamp-assign-{id}")))
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .pt_2()
                .child(div().leading_xs().text_color(muted).child(title))
                .child(div().max_w(px(384.)).child(Input::new(&self.assign_query).id(SharedString::from(format!("org-basecamp-assign-query-{id}")))))
                .when(members.is_empty(), |this| this.child(div().id("org-basecamp-no-member").text_color(muted).child(tr(cx, "org:basecamp.noMember")).test_support()))
                .children(members.into_iter().take(8).map(|member| {
                    let (name, handle) = cac_core::member_face(&member);
                    let (target, picked) = (target.clone(), member.clone());
                    div()
                        .id(SharedString::from(format!("org-basecamp-assign-{id}-{}", member.user_id)))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover_bg))
                        .on_click(cx.listener(move |this, _, _, cx| this.assign_person(target.clone(), picked.clone(), cx)))
                        .child(name)
                        .children(handle.map(|handle| div().text_xs().text_color(muted).child(handle)))
                        .test_support()
                }))
        });
        div()
            .id(SharedString::from(format!("org-basecamp-person-{id}")))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(div().flex().flex_col().flex_1().min_w_0().child(div().truncate().child(person.name.clone())).child(div().truncate().text_xs().text_color(muted).child(person.email.clone())))
            .when(person.client, |this| this.child(div().text_xs().text_color(muted).child(tr(cx, "org:basecamp.client"))))
            .child(
                Button::new(SharedString::from(format!("org-basecamp-assign-open-{id}")))
                    .outline()
                    .small()
                    .label(tr(cx, "org:basecamp.assign"))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| this.start_assigning(id, window, cx))),
            )
            .children(picker)
            .test_support()
            .into_any_element()
    }

    pub(super) fn render_basecamp(&mut self, manage: bool, cx: &mut Context<Self>) -> AnyElement {
        // La pestaña no existe para quien no administra: si se llega a ella
        // por otro camino, no se enseña ni se pide nada.
        if !manage {
            return div().into_any_element();
        }
        if self.basecamp.status.is_none() && !self.basecamp.loading && self.basecamp.status_error.is_none() {
            self.load_basecamp(cx);
        }
        let muted = cx.theme().muted_foreground;
        if let (None, Some(text)) = (&self.basecamp.status, self.basecamp.status_error.clone()) {
            let danger = cx.theme().danger;
            return div()
                .id("org-basecamp-load-failed")
                .max_w(px(768.))
                .flex()
                .flex_col()
                .items_start()
                .gap_2()
                .child(div().id("org-basecamp-load-failed-text").text_color(danger).aria_label(text.clone()).child(text).test_support())
                .child(Button::new("org-basecamp-retry").outline().small().icon(Icon::new(Lucide::RefreshCw)).label(tr(cx, "org:basecamp.retry")).on_click(cx.listener(|this, _, _, cx| this.load_basecamp(cx))))
                .test_support()
                .into_any_element();
        }
        let Some(status) = self.basecamp.status.clone() else {
            return div().id("org-basecamp-loading").py_8().text_center().text_color(muted).child(tr(cx, "org:basecamp.loading")).test_support().into_any_element();
        };
        if !status.configured {
            return div().id("org-basecamp-off").max_w(px(768.)).text_color(muted).child(tr(cx, "org:basecamp.notConfigured")).test_support().into_any_element();
        }
        let Some((org, _)) = Self::current(cx) else { return div().into_any_element() };
        let spaces = Self::current_spaces(cx).map(|spaces| linkable_spaces(&spaces, &org.id)).unwrap_or_default();
        div()
            .id("org-basecamp")
            .max_w(px(768.))
            .flex()
            .flex_col()
            .gap_4()
            .child(self.render_basecamp_account(&status, cx))
            .when(status.connected, |this| this.child(self.render_basecamp_projects(&status, &spaces, cx)).child(self.render_basecamp_people(cx)))
            .into_any_element()
    }
}
