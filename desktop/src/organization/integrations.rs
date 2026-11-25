//! Integraciones (`OrgIntegrations.tsx`): a qué está conectada la
//! organización, o sea, sus proyectos de reportes.
//!
//! Son integraciones de servidor a servidor: otro sistema guarda una llave de
//! ingesta y empuja trabajo, y todo lo que manda cae en una lista de aquí.
//! Qué se manda al guardar (sólo lo que cambió) y qué lista se dice lo decide
//! el core (`cac_core::org_integrations`).

use cac_core::{inbox_lists, list_path, lists_by_space, text_with, time_ago, OrgMember, ProjectForm, ReportProject};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::pickers::{choice_chip, Choice};
use super::{card, OrgView};
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// La ficha en edición.
pub(super) struct Editing {
    id: String,
    name: Entity<InputState>,
    per_hour: Entity<InputState>,
    per_reporter: Entity<InputState>,
    webhook: Entity<InputState>,
    secret: Entity<InputState>,
    assignee: String,
    list_id: String,
}

#[derive(Default)]
pub(super) struct IntegrationsTab {
    /// El nombre de la nueva, mientras se escribe.
    creating: Option<Entity<InputState>>,
    _on_creating: Option<Subscription>,
    /// La llave recién entregada. El servidor la da al crear y al rotar y no
    /// vuelve a darla —guarda un hash y nada más—, así que es el único
    /// momento en que existe legible: se queda a la vista hasta que se diga
    /// que ya se guardó, y no en un aviso que se va mientras se busca el
    /// gestor de contraseñas.
    key: Option<String>,
    editing: Option<Editing>,
}

impl OrgView {
    /// Vuelve a pedir los proyectos, sin vaciar lo que se ve.
    pub(super) fn refresh_projects(&mut self, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let id = org.id.clone();
        let answer = Backend::ask(cx, move |core| async move { (id.clone(), core.orgs().report_projects(&id).await) });
        cx.spawn(async move |this, cx| {
            let Ok((id, projects)) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.loaded_for.as_deref() == Some(id.as_str()) {
                    if let Ok(projects) = projects {
                        this.projects = projects;
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn start_creating(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr(cx, "org:integrationNamePlaceholder")));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| match event {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => this.create_project(cx),
            _ => {}
        });
        self.integrations._on_creating = Some(subscription);
        self.integrations.creating = Some(input);
        self.notice = None;
        cx.notify();
    }

    fn create_project(&mut self, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let Some(input) = self.integrations.creating.clone() else { return };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let id = org.id.clone();
        self.act_with(
            move |core| async move { core.orgs().create_report_project(&id, &name).await },
            |this, key, cx| {
                this.integrations.creating = None;
                this.integrations.key = Some(key);
                this.refresh_projects(cx);
            },
            "org:errCreate",
            cx,
        );
    }

    fn start_editing(&mut self, project: &ReportProject, window: &mut Window, cx: &mut Context<Self>) {
        let form = ProjectForm::of(project);
        let field = |value: String, placeholder: String, masked: bool, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(placeholder).masked(masked);
                state.set_value(value, window, cx);
                state
            })
        };
        let name = field(form.name, tr(cx, "org:namePlaceholder"), false, window, cx);
        let per_hour = field(form.per_hour, String::new(), false, window, cx);
        let per_reporter = field(form.per_reporter, String::new(), false, window, cx);
        let webhook = field(form.webhook_url, tr(cx, "org:webhookPlaceholder"), false, window, cx);
        let secret = field(String::new(), tr(cx, if project.webhook_configured { "org:leaveBlankToKeep" } else { "org:optional" }), true, window, cx);
        name.update(cx, |input, cx| input.focus(window, cx));
        self.integrations.editing = Some(Editing { id: project.id.clone(), name, per_hour, per_reporter, webhook, secret, assignee: form.assignee, list_id: form.list_id });
        self.notice = None;
        cx.notify();
    }

    fn save_editing(&mut self, cx: &mut Context<Self>) {
        let Some(editing) = self.integrations.editing.as_ref() else { return };
        let Some(project) = self.projects.iter().find(|project| project.id == editing.id).cloned() else { return };
        let read = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let form = ProjectForm {
            name: read(&editing.name),
            per_hour: read(&editing.per_hour),
            per_reporter: read(&editing.per_reporter),
            webhook_url: read(&editing.webhook),
            webhook_secret: read(&editing.secret),
            assignee: editing.assignee.clone(),
            list_id: editing.list_id.clone(),
        };
        let patch = form.patch(&project);
        // Nadie que sólo venía a mirar espera que abrir y cerrar el
        // formulario escriba nada.
        if patch.is_empty() {
            self.integrations.editing = None;
            cx.notify();
            return;
        }
        let id = project.id;
        self.act(
            move |core| async move { core.orgs().update_report_project(&id, &patch).await },
            |this, cx| {
                this.integrations.editing = None;
                this.refresh_projects(cx);
            },
            "org:errSave",
            cx,
        );
    }

    fn toggle_active(&mut self, project: &ReportProject, cx: &mut Context<Self>) {
        let (id, active) = (project.id.clone(), !project.is_active);
        self.act(move |core| async move { core.orgs().set_report_project_active(&id, active).await }, |this, cx| this.refresh_projects(cx), "org:errSave", cx);
    }

    fn ask_rotate(&mut self, project: ReportProject, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_with(locale(cx), "org:rotateNamed", &[("name", &project.name)]);
        let (body, confirm, cancel) = (tr(cx, "org:rotateBody"), tr(cx, "org:rotate"), tr(cx, "org:cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                let id = project.id.clone();
                this.act_with(move |core| async move { core.orgs().rotate_report_key(&id).await }, |this, key, _| this.integrations.key = Some(key), "org:errSave", cx);
            })
            .ok();
        })
        .detach();
    }

    fn ask_delete_project(&mut self, project: ReportProject, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_with(locale(cx), "org:deleteTitle", &[("name", &project.name)]);
        let (body, confirm, cancel) = (tr(cx, "org:deleteIntegrationBody"), tr(cx, "org:delete"), tr(cx, "org:cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                let id = project.id.clone();
                this.act(move |core| async move { core.orgs().delete_report_project(&id).await }, |this, cx| this.refresh_projects(cx), "org:errSave", cx);
            })
            .ok();
        })
        .detach();
    }

    fn render_key(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let key = self.integrations.key.clone()?;
        let theme = cx.theme();
        let (warning, muted, background, mono) = (theme.warning, theme.muted_foreground, theme.background, theme.mono_font_family.clone());
        let copied = key.clone();
        Some(
            div()
                .id("org-key-once")
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .rounded(px(6.))
                .border_1()
                .border_color(warning.opacity(0.5))
                .bg(warning.opacity(0.1))
                .leading_xs()
                .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "org:keyShownOnce")))
                .child(div().text_color(muted).child(tr(cx, "org:keyHashOnly")))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .mt_1()
                        .child(div().id("org-key").aria_label(key.clone()).flex_1().min_w_0().truncate().px_2().py_1().rounded(px(4.)).bg(background).font_family(mono).child(key).test_support())
                        .child(Button::new("org-key-copy").ghost().small().icon(Icon::new(Lucide::Copy)).label(tr(cx, "common:admin.copy")).on_click(cx.listener(move |_, _, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone())))))
                        .child(Button::new("org-key-saved").ghost().small().label(tr(cx, "org:iSavedIt")).on_click(cx.listener(|this, _, _, cx| {
                            this.integrations.key = None;
                            cx.notify();
                        }))),
                )
                .test_support()
                .into_any_element(),
        )
    }

    fn render_editing(&self, editing: &Editing, project: &ReportProject, lists: &[cac_core::ListPath], members: &[OrgMember], cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let label = |key: &str, input: AnyElement| div().flex().flex_col().gap_1().child(div().leading_xs().text_color(muted).child(tr(cx, key))).child(input);

        let mut choices = Vec::new();
        for (space, group) in lists_by_space(lists.to_vec()) {
            choices.push(Choice::Header(space));
            choices.extend(group.into_iter().map(|list| Choice::option(list.id, list.rest)));
        }
        let list_label = lists.iter().find(|list| list.id == editing.list_id).map_or_else(|| tr(cx, "org:pickList"), |list| list.path());
        let view = cx.entity().downgrade();
        // Sin opción de «ninguna»: quitarla no desconfigura la integración,
        // hace que todo lo que le manden se pierda sin decir nada.
        let list_picker = choice_chip("org-project-list", list_label, choices, editing.list_id.clone(), false, move |value, _, cx| {
            view.update(cx, |this, cx| {
                if let Some(editing) = this.integrations.editing.as_mut() {
                    editing.list_id = value;
                }
                cx.notify();
            })
            .ok();
        });

        let mut people = vec![Choice::option("", tr(cx, "org:nobody"))];
        people.extend(members.iter().map(|member| Choice::option(member.user_id.clone(), format!("@{}", member.username))));
        let assignee_label = members.iter().find(|member| member.user_id == editing.assignee).map_or_else(|| tr(cx, "org:nobody"), |member| format!("@{}", member.username));
        let view = cx.entity().downgrade();
        let assignee_picker = choice_chip("org-project-assignee", assignee_label, people, editing.assignee.clone(), false, move |value, _, cx| {
            view.update(cx, |this, cx| {
                if let Some(editing) = this.integrations.editing.as_mut() {
                    editing.assignee = value;
                }
                cx.notify();
            })
            .ok();
        });

        let secret_label = if project.webhook_configured { "org:replaceSecret" } else { "org:signingSecret" };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_3p5()
            .py_3()
            .child(div().max_w(px(384.)).child(Input::new(&editing.name).id("org-project-name")))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(label("org:perHour", div().w(px(112.)).child(Input::new(&editing.per_hour).id("org-project-per-hour")).into_any_element()))
                    .child(label("org:perReporter", div().w(px(112.)).child(Input::new(&editing.per_reporter).id("org-project-per-reporter")).into_any_element()))
                    .child(label("org:webhook", div().w(px(288.)).child(Input::new(&editing.webhook).id("org-project-webhook")).into_any_element()))
                    .child(label(secret_label, div().w(px(240.)).child(Input::new(&editing.secret).id("org-project-secret").mask_toggle()).into_any_element()))
                    .child(label("org:reportsArriveIn", list_picker.into_any_element()))
                    .child(label("org:defaultAssignee", assignee_picker.into_any_element())),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .pt_1()
                    .child(Button::new("org-project-save").primary().small().label(tr(cx, "common:action.save")).disabled(self.busy).on_click(cx.listener(|this, _, _, cx| this.save_editing(cx))))
                    .child(Button::new("org-project-cancel").ghost().small().label(tr(cx, "org:cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.integrations.editing = None;
                        cx.notify();
                    }))),
            )
            .into_any_element()
    }

    fn render_project(&self, project: &ReportProject, manage: bool, lists: &[cac_core::ListPath], spaces: Option<&[cac_core::SpaceTree]>, org_id: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, warning, danger, card_bg, chip, mono) = (theme.border, theme.muted_foreground, theme.warning, theme.danger, theme.group_box, theme.muted, theme.mono_font_family.clone());
        let lang = locale(cx);
        let id = project.id.clone();
        let members = self.members.clone().unwrap_or_default();
        let editing = self.integrations.editing.as_ref().filter(|editing| editing.id == project.id);
        let assignee = members.iter().find(|member| Some(&member.user_id) == project.default_assignee_user_id.as_ref());
        let badge = |text: String, outline: bool| div().px_1p5().rounded(px(4.)).text_size(px(10.)).when(outline, |this| this.border_1().border_color(border)).when(!outline, |this| this.bg(chip)).child(text);
        let heading = |key: &str| div().text_size(px(10.5)).text_color(muted).child(tr(cx, key).to_uppercase());
        let cell = |key: &str, value: AnyElement| div().flex().flex_col().gap_0p5().min_w(px(240.)).flex_1().child(heading(key)).child(value);

        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3p5()
            .py_3()
            .border_b_1()
            .border_color(border)
            .child(div().truncate().font_weight(FontWeight::MEDIUM).when(!project.is_active, |this| this.text_color(muted)).child(project.name.clone()))
            .child(div().truncate().font_family(mono.clone()).text_size(px(11.)).text_color(muted).child(project.slug.clone()))
            .child(badge(tr(cx, "org:serverToServer"), false))
            .child(badge(tr(cx, if project.is_active { "org:active" } else { "org:paused" }), true))
            .child(div().flex_1())
            .child(div().text_color(muted).text_size(px(12.)).child(cac_core::text_count(lang, "common:count.reportsThisMonth", project.reports_this_month)));

        let rotate = manage.then(|| {
            let target = project.clone();
            Button::new(SharedString::from(format!("org-project-rotate-{id}"))).ghost().small().icon(Icon::new(Lucide::KeyRound)).label(tr(cx, "org:rotate")).on_click(cx.listener(move |this, _, window, cx| this.ask_rotate(target.clone(), window, cx)))
        });
        // Lo primero que se pregunta de una integración y lo único que no se
        // podía ver: dónde acaba lo que manda.
        let arrives = match project.list_id.as_deref().filter(|list| !list.is_empty()) {
            // El árbol todavía no llegó: decir «de otra organización» sería falso.
            Some(_) if spaces.is_none() => div().id(SharedString::from(format!("org-project-list-{id}"))).text_color(muted).child(tr(cx, "common:servers.loading")),
            Some(list) => match list_path(spaces.unwrap_or_default(), org_id, list) {
                Some(path) => div().id(SharedString::from(format!("org-project-list-{id}"))).font_weight(FontWeight::MEDIUM).aria_label(path.clone()).child(path),
                // Hay bandeja, pero no es de esta organización o ya no
                // existe. Decirlo es mejor que pintar el uuid.
                None => div().id(SharedString::from(format!("org-project-list-{id}"))).text_color(warning).aria_label(tr(cx, "channel:outsideOrg")).child(tr(cx, "channel:outsideOrg")),
            },
            None => div().id(SharedString::from(format!("org-project-list-{id}"))).text_color(danger).aria_label(tr(cx, "org:nowhereLost")).child(tr(cx, "org:nowhereLost")),
        }
        .test_support();
        let facts = div()
            .flex()
            .flex_wrap()
            .gap_x_6()
            .gap_y_3()
            .px_3p5()
            .py_3()
            .leading_xs()
            .child(cell("org:ingestKey", div().flex().items_center().gap_2().child(div().text_color(muted).child(tr(cx, "org:shownOnceShort"))).children(rotate).into_any_element()))
            .child(cell("org:origin", div().child(tr(cx, "org:originNotChecked")).into_any_element()))
            .child(cell("org:limits", div().child(text_with(lang, "org:limitsLine", &[("perHour", &project.rate_limit_per_hour.to_string()), ("perReporter", &project.rate_limit_per_reporter_per_hour.to_string())])).into_any_element()))
            .child(cell(
                "org:webhook",
                if project.webhook_url.is_empty() {
                    div().text_color(muted).child(tr(cx, "org:none"))
                } else {
                    div().child(project.webhook_url.clone()).child(div().text_color(muted).child(format!(" · {}", tr(cx, if project.webhook_configured { "org:signed" } else { "org:unsigned" }))))
                }
                .into_any_element(),
            ))
            .child(cell("org:reportsArriveInShort", arrives.into_any_element()))
            .child(cell("org:defaultAssignee", match assignee {
                Some(member) => div().child(format!("@{}", member.username)),
                None => div().text_color(muted).child(tr(cx, "org:nobody")),
            }.into_any_element()))
            .child(cell("org:created", div().child(time_ago(&project.created_at, &chrono::Utc::now(), lang)).into_any_element()));

        let footer = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3p5()
            .py_2()
            .border_t_1()
            .border_color(border)
            .child(div().flex_1().min_w_0().truncate().font_family(mono).text_size(px(11.)).text_color(muted).child("POST /api/v1/reports/ingest · X-Ingest-Key"))
            .when(manage && editing.is_none(), |this| {
                let (editable, pausing, deleting) = (project.clone(), project.clone(), project.clone());
                this.child(Button::new(SharedString::from(format!("org-project-edit-{id}"))).ghost().small().icon(Icon::new(Lucide::Pencil)).label(tr(cx, "org:edit")).on_click(cx.listener(move |this, _, window, cx| this.start_editing(&editable, window, cx))))
                    .child(
                        Button::new(SharedString::from(format!("org-project-pause-{id}")))
                            .ghost()
                            .small()
                            .icon(Icon::new(Lucide::Power))
                            .label(tr(cx, if project.is_active { "org:pause" } else { "org:resume" }))
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_active(&pausing, cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("org-project-delete-{id}")))
                            .ghost()
                            .small()
                            .text_color(danger)
                            .icon(Icon::new(Lucide::Trash))
                            .label(tr(cx, "org:delete"))
                            .on_click(cx.listener(move |this, _, window, cx| this.ask_delete_project(deleting.clone(), window, cx))),
                    )
            });

        div()
            .id(SharedString::from(format!("org-project-{id}")))
            .rounded(px(12.))
            .border_1()
            .border_color(border)
            .bg(card_bg)
            .when(!project.is_active, |this| this.opacity(0.7))
            .child(header)
            .child(match editing {
                Some(editing) => self.render_editing(editing, project, lists, &members, cx),
                None => facts.into_any_element(),
            })
            .child(footer)
            .test_support()
            .into_any_element()
    }

    pub(super) fn render_integrations(&mut self, org: &cac_core::Organization, manage: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let spaces = Self::current_spaces(cx);
        let lists = inbox_lists(spaces.as_deref().unwrap_or_default(), &org.id);
        let key = self.render_key(cx);
        let busy = self.busy;

        let creating = self.integrations.creating.clone().map(|input| {
            card("org-project-new", cx)
                .gap_2()
                .child(div().max_w(px(384.)).child(Input::new(&input).id("org-project-new-name")))
                .child(div().leading_xs().text_color(muted).child(tr(cx, "org:noOriginChecked")))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(Button::new("org-project-create").primary().small().label(tr(cx, "org:create")).disabled(busy || input.read(cx).value().trim().is_empty()).on_click(cx.listener(|this, _, _, cx| this.create_project(cx))))
                        .child(Button::new("org-project-create-cancel").ghost().small().label(tr(cx, "org:cancel")).on_click(cx.listener(|this, _, _, cx| {
                            this.integrations.creating = None;
                            cx.notify();
                        }))),
                )
                .test_support()
        });

        let body: AnyElement = if self.projects.is_empty() {
            div().id("org-projects-none").px_3().py_8().rounded(px(12.)).border_1().border_dashed().border_color(border).text_center().text_color(muted).child(tr(cx, "org:nothingWired")).test_support().into_any_element()
        } else {
            let projects = self.projects.clone();
            div().id("org-projects").flex().flex_col().gap_3().children(projects.iter().map(|project| self.render_project(project, manage, &lists, spaces.as_deref(), &org.id, cx))).test_support().into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(key)
            // Lo que hay que saber antes de crear una, no después.
            .child(div().max_w(px(660.)).leading_xs().text_color(muted).child(tr(cx, "org:lead")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "org:reportProjects")))
                    .child(div().flex_1())
                    .when(manage && self.integrations.creating.is_none(), |this| {
                        this.child(Button::new("org-project-new-button").outline().small().icon(Icon::new(Lucide::Plus)).label(tr(cx, "org:new")).on_click(cx.listener(|this, _, window, cx| this.start_creating(window, cx))))
                    }),
            )
            .children(creating)
            .child(body)
            .into_any_element()
    }
}
