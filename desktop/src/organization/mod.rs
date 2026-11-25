//! La pantalla de la organización (`OrganizationSettings.tsx`): su cabecera,
//! sus pestañas y lo que cada una enseña. Qué se lista, quién administra y qué
//! se dice de cada cosa lo decide el core (`cac_core::org_admin`); aquí se
//! pinta y se le pasa cada gesto.
//!
//! Y las invitaciones que le llegan a quien mira (`Invitations.tsx`), en
//! `received.rs`: otra pantalla, que se abre desde el pie del panel.

mod general;
mod invites;
mod members;
pub mod received;
mod spaces;

pub use spaces::ManageSpace;

use cac_core::{can_manage_org, org_summary, org_tabs, Invitation, OrgMember, OrgTab, Organization, PersonHit, ReportProject};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// El aviso de la última acción: lo que en la app de siempre es un toast.
#[derive(Clone)]
pub(crate) struct Notice {
    pub text: String,
    pub problem: bool,
}

pub struct OrgView {
    tab: OrgTab,
    /// De qué organización es lo cargado. Otra en pantalla lo vuelve a pedir.
    loaded_for: Option<String>,
    members: Option<Vec<OrgMember>>,
    invites: Vec<Invitation>,
    projects: Vec<ReportProject>,
    notice: Option<Notice>,
    /// Pidiendo algo que cambia la organización: otro clic no lo repite.
    busy: bool,
    search: Entity<InputState>,
    role_filter: String,
    // Invitar.
    people_query: Entity<InputState>,
    people_hits: Vec<PersonHit>,
    /// Para no pintar los resultados de una búsqueda vieja que llega tarde.
    people_turn: u64,
    picked: Option<PersonHit>,
    invite_role: Option<String>,
    // Dar de alta por correo.
    account_name: Entity<InputState>,
    account_email: Entity<InputState>,
    account_role: Option<String>,
    /// El enlace para pasarlo a mano, cuando el correo no salió.
    account_link: Option<(String, String)>,
    // General.
    name_input: Entity<InputState>,
    domain_input: Entity<InputState>,
    zone_input: Entity<InputState>,
    /// La frase escrita para borrar la organización, con el diálogo abierto.
    deleting: Option<Entity<InputState>>,
    // Espacios: el nombre del nuevo, mientras se escribe.
    new_space: Option<Entity<InputState>>,
    /// Las del campo de borrar y del espacio nuevo: una cada vez, la que se
    /// abre sustituye a la anterior.
    _on_deleting: Option<Subscription>,
    _on_new_space: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl OrgView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: String, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let search = input(tr(cx, "org:searchMember"), window, cx);
        let people_query = input(tr(cx, "org:searchByUsername"), window, cx);
        let account_name = input(tr(cx, "common:account.name"), window, cx);
        let account_email = input(tr(cx, "common:account.email"), window, cx);
        let name_input = input(String::new(), window, cx);
        let domain_input = input("example.com".into(), window, cx);
        let zone_input = input(tr(cx, "org:timeZonePlaceholder"), window, cx);
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&people_query, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search_people(cx);
                }
            }),
            cx.subscribe_in(&account_name, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&account_email, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.create_account(window, cx),
                _ => {}
            }),
            // Nombre, dominio y zona se guardan al salir del campo o con Enter.
            cx.subscribe_in(&name_input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.save_identity(general::Field::Name, window, cx);
                }
            }),
            cx.subscribe_in(&domain_input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.save_identity(general::Field::Domain, window, cx);
                }
            }),
            cx.subscribe_in(&zone_input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.save_identity(general::Field::Zone, window, cx);
                }
            }),
        ];
        OrgView {
            tab: OrgTab::Members,
            loaded_for: None,
            members: None,
            invites: Vec::new(),
            projects: Vec::new(),
            notice: None,
            busy: false,
            search,
            role_filter: String::new(),
            people_query,
            people_hits: Vec::new(),
            people_turn: 0,
            picked: None,
            invite_role: None,
            account_name,
            account_email,
            account_role: None,
            account_link: None,
            name_input,
            domain_input,
            zone_input,
            deleting: None,
            new_space: None,
            _on_deleting: None,
            _on_new_space: None,
            _subscriptions: subscriptions,
        }
    }

    /// Cuando se vuelve a la pantalla: lo de antes puede haber cambiado.
    pub fn reopened(&mut self, cx: &mut Context<Self>) {
        self.loaded_for = None;
        self.notice = None;
        cx.notify();
    }

    /// Pone una pestaña delante.
    pub fn show_tab(&mut self, tab: OrgTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.notice = None;
        cx.notify();
    }

    pub(crate) fn current(cx: &App) -> Option<(Organization, bool)> {
        let snapshot = Backend::snapshot(cx).read(cx);
        let superadmin = snapshot.session.as_ref().is_some_and(|session| session.superadmin);
        let org = snapshot.orgs.current()?.clone();
        let manage = can_manage_org(&org, superadmin);
        Some((org, manage))
    }

    /// Los espacios, si el árbol ya es el de la organización que se mira. Al
    /// cambiar de una a otra, el de la anterior sigue un rato en el snapshot.
    pub(crate) fn current_spaces(cx: &App) -> Option<Vec<cac_core::SpaceTree>> {
        let snapshot = Backend::snapshot(cx).read(cx);
        let tree = &snapshot.tree;
        let ours = tree.org_id.is_some() && tree.org_id == snapshot.orgs.current_org_id;
        (ours && !(tree.loading && tree.spaces.is_empty())).then(|| tree.spaces.clone())
    }

    pub(crate) fn say(&mut self, text: String, problem: bool, cx: &mut Context<Self>) {
        self.notice = Some(Notice { text, problem });
        cx.notify();
    }

    /// Trae lo de la organización que se mira: sus miembros, sus
    /// invitaciones (sólo a quien administra) y sus proyectos de reportes.
    fn load(&mut self, org: &Organization, manage: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.loaded_for = Some(org.id.clone());
        self.members = None;
        self.invites.clear();
        self.picked = None;
        self.people_hits.clear();
        self.invite_role = None;
        self.account_role = None;
        self.account_link = None;
        self.deleting = None;
        self.new_space = None;
        let (name, domain, zone) = (org.name.clone(), org.domain.clone().unwrap_or_default(), org.timezone.clone());
        self.name_input.update(cx, |input, cx| input.set_value(name, window, cx));
        self.domain_input.update(cx, |input, cx| input.set_value(domain, window, cx));
        self.zone_input.update(cx, |input, cx| input.set_value(zone, window, cx));
        if !manage && self.tab == OrgTab::Invites {
            self.tab = OrgTab::Members;
        }
        let id = org.id.clone();
        let answer = Backend::ask(cx, move |core| async move {
            let orgs = core.orgs();
            let members = orgs.members(&id).await;
            let invites = if manage { orgs.sent_invitations(&id).await.unwrap_or_default() } else { Vec::new() };
            let projects = orgs.report_projects(&id).await.unwrap_or_default();
            (id, members, invites, projects)
        });
        cx.spawn(async move |this, cx| {
            let Ok((id, members, invites, projects)) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.loaded_for.as_deref() != Some(id.as_str()) {
                    return;
                }
                match members {
                    Ok(members) => this.members = Some(members),
                    Err(error) => {
                        this.members = Some(Vec::new());
                        this.notice = Some(Notice { text: crate::lang::error_text(cx, &error, "org:errLoad"), problem: true });
                    }
                }
                this.invites = invites;
                this.projects = projects;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Vuelve a pedir miembros e invitaciones, sin vaciar lo que se ve.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some((org, manage)) = Self::current(cx) else { return };
        let id = org.id.clone();
        let answer = Backend::ask(cx, move |core| async move {
            let orgs = core.orgs();
            let members = orgs.members(&id).await.ok();
            let invites = if manage { orgs.sent_invitations(&id).await.ok() } else { Some(Vec::new()) };
            (id, members, invites)
        });
        cx.spawn(async move |this, cx| {
            let Ok((id, members, invites)) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.loaded_for.as_deref() != Some(id.as_str()) {
                    return;
                }
                if let Some(members) = members {
                    this.members = Some(members);
                }
                if let Some(invites) = invites {
                    this.invites = invites;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Lo que pide algo al servidor y sólo tiene que decir cómo fue.
    pub(crate) fn act<F, Fut>(&mut self, work: F, done: impl FnOnce(&mut Self, &mut Context<Self>) + 'static, fallback_key: &'static str, cx: &mut Context<Self>)
    where
        F: FnOnce(cac_core::Core) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), cac_core::ApiError>> + Send + 'static,
    {
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = None;
        cx.notify();
        let answer = Backend::ask(cx, work);
        let asked_for = self.loaded_for.clone();
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                // Lo que contesta tras cambiar de organización es de la otra.
                if this.loaded_for != asked_for {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(()) => done(this, cx),
                    Err(error) => this.notice = Some(Notice { text: crate::lang::error_text(cx, &error, fallback_key), problem: true }),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_header(&self, org: &Organization, manage: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (primary, muted) = (theme.primary, theme.muted_foreground);
        let superadmin = Backend::snapshot(cx).read(cx).session.as_ref().is_some_and(|session| session.superadmin);
        let spaces = Self::current_spaces(cx).map_or(0, |spaces| spaces.len());
        let summary = org_summary(org, superadmin, spaces, locale(cx));
        let initial: String = org.name.chars().take(1).flat_map(char::to_uppercase).collect();
        div()
            .flex()
            .items_start()
            .gap_3()
            .child(
                div()
                    .size(px(36.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .bg(primary.opacity(0.15))
                    .text_color(primary)
                    .leading_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(initial),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().id("org-page-title").aria_label(org.name.clone()).truncate().text_size(rems(1.25)).line_height(rems(1.75)).font_weight(FontWeight::SEMIBOLD).child(org.name.clone()).test_support())
                    .child(div().id("org-page-summary").leading_xs().text_color(muted).aria_label(summary.clone()).child(summary).test_support()),
            )
            .when(manage, |this| {
                this.child(
                    Button::new("org-invite")
                        .primary()
                        .small()
                        .label(tr(cx, "org:invite"))
                        .on_click(cx.listener(|this, _, _, cx| this.show_tab(OrgTab::Invites, cx))),
                )
            })
    }

    fn render_tabs(&self, manage: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, primary, muted, foreground) = (theme.border, theme.primary, theme.muted_foreground, theme.foreground);
        let spaces = Self::current_spaces(cx).map(|spaces| spaces.len());
        // Cuentan las pestañas que son listas; al lado de un formulario, un
        // «0» es ruido.
        let count = |tab: OrgTab| match tab {
            OrgTab::Members => self.members.as_ref().map(|members| members.len()),
            OrgTab::Invites => Some(self.invites.len()),
            OrgTab::Spaces => spaces,
            _ => None,
        };
        div().flex().flex_none().gap_4().border_b_1().border_color(border).children(org_tabs(manage).into_iter().map(|tab| {
            let active = tab == self.tab;
            let label = tr(cx, tab.label_key());
            div()
                .id(SharedString::from(format!("org-tab-{}", tab.as_str())))
                .flex()
                .items_center()
                .gap_1p5()
                .pb_2()
                .border_b_2()
                .border_color(if active { primary } else { transparent_black() })
                .leading_sm()
                .when(active, |this| this.font_weight(FontWeight::MEDIUM).text_color(foreground))
                .when(!active, |this| this.text_color(muted).hover(move |style| style.text_color(foreground)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.show_tab(tab, cx)))
                .aria_label(label.clone())
                .child(label)
                .children(count(tab).map(|count| div().text_xs().text_color(muted).child(count.to_string())))
                .test_support()
        }))
    }

    fn render_notice(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let notice = self.notice.clone()?;
        let theme = cx.theme();
        let (danger, muted, border) = (theme.danger, theme.muted_foreground, theme.border);
        Some(
            div()
                .id("org-notice")
                .px_3()
                .py_2()
                .rounded(px(8.))
                .border_1()
                .border_color(if notice.problem { danger.opacity(0.4) } else { border })
                .leading_xs()
                .text_color(if notice.problem { danger } else { muted })
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.notice = None;
                    cx.notify();
                }))
                .aria_label(notice.text.clone())
                .child(notice.text)
                .test_support(),
        )
    }

    fn render_pending(cx: &App) -> AnyElement {
        div().id("org-tab-pending").py_8().text_center().leading_sm().text_color(cx.theme().muted_foreground).child(tr(cx, "common:misc.notInThisApp")).test_support().into_any_element()
    }
}

impl Render for OrgView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted) = (theme.background, theme.foreground, theme.muted_foreground);
        let Some((org, manage)) = Self::current(cx) else {
            return div().id("org-page").size_full().flex().items_center().justify_center().bg(background).text_color(muted).leading_sm().child(tr(cx, "org:none")).test_support().into_any_element();
        };
        if self.loaded_for.as_deref() != Some(org.id.as_str()) {
            self.load(&org, manage, window, cx);
        }
        let body = match self.tab {
            OrgTab::Members => self.render_members(&org, manage, cx),
            OrgTab::Invites => self.render_invites(&org, manage, cx),
            OrgTab::Spaces => self.render_spaces(manage, cx),
            OrgTab::General => self.render_general(&org, manage, cx),
            OrgTab::Integrations | OrgTab::GitHub | OrgTab::Meetings => Self::render_pending(cx),
        };
        let dialog = self.render_delete_dialog(&org, window, cx);
        let account_link = self.render_account_link(window, cx);
        div()
            .relative()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(
                div().id("org-page").size_full().overflow_y_scroll().test_support().child(
                    div()
                        .w_full()
                        .px_8()
                        .py_6()
                        .flex()
                        .flex_col()
                        .gap_6()
                        .leading_sm()
                        .child(self.render_header(&org, manage, cx))
                        .child(self.render_tabs(manage, cx))
                        .children(self.render_notice(cx))
                        .child(body),
                ),
            )
            .children(dialog)
            .children(account_link)
            .into_any_element()
    }
}

/// Un menú para elegir un papel, como chip: el que se ve y los tres.
pub(crate) fn role_chip(id: SharedString, current: &str, disabled: bool, on_pick: impl Fn(String, &mut Window, &mut App) + 'static, _cx: &App) -> impl IntoElement {
    use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
    use gpui_kit::component::{Disableable, Icon};
    let on_pick = std::rc::Rc::new(on_pick);
    let (current, menu_id) = (current.to_string(), id.clone());
    Button::new(id)
        .outline()
        .small()
        .label(crate::organization::capitalized(&current))
        .icon(Icon::new(gpui_kit::assets::IconName::ChevronDown))
        .disabled(disabled)
        .dropdown_menu(move |mut menu, _, _| {
            for role in cac_core::ORG_ROLES {
                let on_pick = on_pick.clone();
                let item = SharedString::from(format!("{menu_id}-{role}"));
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| div().id(item.clone()).child(capitalized(role)).test_support())
                        .checked(role == current)
                        .on_click(move |_, window, cx| on_pick(role.to_string(), window, cx)),
                );
            }
            menu
        })
}

/// `capitalize` de CSS para una palabra.
pub(crate) fn capitalized(word: &str) -> String {
    let mut letters = word.chars();
    letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
}

/// Una tarjeta: `rounded-xl border bg-card p-4`.
pub(crate) fn card(id: &'static str, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    div().id(id).flex().flex_col().gap_3().p_4().rounded(px(12.)).border_1().border_color(theme.border).bg(theme.group_box)
}
