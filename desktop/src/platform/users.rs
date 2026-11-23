//! Usuarios (`app/src/pages/Users.tsx`): las personas de la plataforma, con
//! su alta, edición y baja. Sólo superadmin: la puerta la guarda el core
//! (`cac_core::platform::users`), que ni abre la sección ni pide la lista a
//! quien no lo es.

use cac_core::{new_user_problems, text_with, user_edit_phrase, AdminUser, EditUserForm, NewUserForm, UserField};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

const COLUMNS: [(&str, f32); 5] = [("common:admin.thUsername", 2.), ("common:admin.thName", 2.), ("common:admin.thEmail", 3.), ("common:admin.thRole", 1.5), ("common:admin.thActions", 1.)];

/// El diálogo abierto: un alta o la edición de alguien.
enum Dialog {
    New { username: Entity<InputState>, password: Entity<InputState>, name: Entity<InputState>, email: Entity<InputState>, superadmin: bool, touched: Vec<UserField> },
    Edit { user: AdminUser, name: Entity<InputState>, email: Entity<InputState>, password: Entity<InputState>, superadmin: bool },
}

pub struct UsersView {
    dialog: Option<Dialog>,
    busy: bool,
    /// Por qué no se hizo lo último que se pidió.
    problem: Option<String>,
}

impl UsersView {
    pub fn new() -> Self {
        UsersView { dialog: None, busy: false, problem: None }
    }

    fn open_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let min8 = tr(cx, "common:admin.min8");
        let mut field = |placeholder: &str, masked: bool| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).masked(masked))
        };
        let username = field("jdoe", false);
        let password = field(&min8, true);
        let name = field("Jane Doe", false);
        let email = field("jane@x.com", false);
        username.update(cx, |input, cx| input.focus(window, cx));
        self.dialog = Some(Dialog::New { username, password, name, email, superadmin: false, touched: Vec::new() });
        self.problem = None;
        cx.notify();
    }

    fn open_edit(&mut self, user: AdminUser, window: &mut Window, cx: &mut Context<Self>) {
        let unchanged = tr(cx, "common:admin.unchanged");
        let mut field = |value: &str, placeholder: String, masked: bool| {
            let value = value.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).masked(masked).default_value(value))
        };
        let name = field(&user.name, String::new(), false);
        let email = field(&user.email, String::new(), false);
        let password = field("", unchanged, true);
        let superadmin = user.is_superadmin;
        self.dialog = Some(Dialog::Edit { user, name, email, password, superadmin });
        self.problem = None;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.dialog = None;
        self.busy = false;
        self.problem = None;
        cx.notify();
    }

    fn new_form(&self, cx: &App) -> Option<NewUserForm> {
        let Some(Dialog::New { username, password, name, email, superadmin, .. }) = &self.dialog else { return None };
        let value = |input: &Entity<InputState>| input.read(cx).value().to_string();
        Some(NewUserForm { username: value(username), password: value(password), name: value(name), email: value(email), is_superadmin: *superadmin })
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let answer = match &mut self.dialog {
            Some(Dialog::New { touched, .. }) => {
                // Pulsar «Crear» es haber pasado por todos los campos.
                *touched = vec![UserField::Username, UserField::Password, UserField::Name, UserField::Email];
                let Some(form) = self.new_form(cx) else { return };
                if !new_user_problems(locale(cx), &form).is_empty() {
                    cx.notify();
                    return;
                }
                Backend::ask(cx, move |core| async move { core.platform().create_user(&form).await.map(|_| ()).map_err(|error| (error, "common:admin.errCreateUser")) })
            }
            Some(Dialog::Edit { user, name, email, password, superadmin }) => {
                let value = |input: &Entity<InputState>| input.read(cx).value().to_string();
                let form = EditUserForm { name: value(name), email: value(email), password: value(password), is_superadmin: *superadmin };
                let id = user.id.clone();
                Backend::ask(cx, move |core| async move { core.platform().update_user(&id, &form).await.map_err(|error| (error, "common:admin.errUpdateUser")) })
            }
            None => return,
        };
        self.busy = true;
        self.problem = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => this.dialog = None,
                    Err((error, fallback)) => this.problem = Some(user_edit_phrase(locale(cx), &error, fallback)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Pregunta nombrando a quién: borrar a alguien no se deshace.
    fn ask_delete(&mut self, user: AdminUser, window: &mut Window, cx: &mut Context<Self>) {
        self.problem = None;
        cx.notify();
        let title = text_with(locale(cx), "common:admin.deleteUserTitle", &[("username", &user.username)]);
        let (body, confirm, cancel) = (tr(cx, "common:admin.deleteUserBody"), tr(cx, "common:admin.delete"), tr(cx, "common:admin.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let id = user.id;
            let Ok(deleted) = cx.update(|_, cx| Backend::ask(cx, move |core| async move { core.platform().delete_user(&id).await })) else { return };
            let Ok(Err(error)) = deleted.await else { return };
            this.update(cx, |this, cx| {
                this.problem = Some(user_edit_phrase(locale(cx), &error, "common:admin.errDeleteUser"));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn row(&self, user: &AdminUser, me: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, primary, primary_foreground, secondary, secondary_foreground, danger) =
            (theme.border, theme.muted_foreground, theme.primary, theme.primary_foreground, theme.secondary, theme.secondary_foreground, theme.danger);
        let role = if user.is_superadmin {
            div().flex().items_center().gap_1().px_1p5().rounded(px(4.)).bg(primary).text_color(primary_foreground).text_xs().child(Icon::new(Lucide::ShieldCheck).size_3()).child("Superadmin")
        } else {
            div().px_1p5().rounded(px(4.)).bg(secondary).text_color(secondary_foreground).text_xs().child(tr(cx, "common:admin.user"))
        };
        let myself = user.id == me;
        let (editing, deleting) = (user.clone(), user.clone());
        let actions = div()
            .flex()
            .justify_end()
            .gap_0p5()
            .child(
                Button::new(SharedString::from(format!("user-edit-{}", user.id)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Lucide::Pencil))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_edit(editing.clone(), window, cx))),
            )
            .child(
                Button::new(SharedString::from(format!("user-delete-{}", user.id)))
                    .ghost()
                    .xsmall()
                    .text_color(danger)
                    .icon(Icon::new(Lucide::Trash))
                    .tooltip(tr(cx, if myself { "common:admin.cantDeleteYourself" } else { "common:admin.deleteUser" }))
                    .disabled(myself)
                    .on_click(cx.listener(move |this, _, window, cx| this.ask_delete(deleting.clone(), window, cx))),
            );
        let dash = |text: &str| if text.is_empty() { "—".to_string() } else { text.to_string() };
        cells(vec![
            div().font_weight(FontWeight::MEDIUM).truncate().child(user.username.clone()).into_any_element(),
            div().text_color(muted).truncate().child(dash(&user.name)).into_any_element(),
            div().text_color(muted).truncate().child(dash(&user.email)).into_any_element(),
            div().flex().child(role).into_any_element(),
            actions.into_any_element(),
        ])
        .id(SharedString::from(format!("user-row-{}", user.id)))
        .h(px(44.))
        .border_t_1()
        .border_color(border)
        .test_support()
        .into_any_element()
    }

    fn field(id: &'static str, label_key: &str, input: &Entity<InputState>, problem: Option<String>, busy: bool, cx: &App) -> Div {
        let danger = cx.theme().danger;
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, label_key)))
            .child(div().id(id).child(Input::new(input).small().disabled(busy)).test_support())
            .children(problem.map(|problem| div().text_xs().text_color(danger).child(problem)))
    }

    fn render_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let me = Backend::snapshot(cx).read(cx).session.as_ref().map(|session| session.id.clone()).unwrap_or_default();
        let busy = self.busy;
        let theme = cx.theme();
        let (background, foreground, muted, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.danger);
        let (title, lead, body, save_key, saving_key, blocked) = match self.dialog.as_ref()? {
            Dialog::New { username, password, name, email, superadmin, touched } => {
                let form = self.new_form(cx)?;
                let problems = new_user_problems(locale(cx), &form);
                // Un campo se queja cuando ya tiene algo o ya se pasó por él.
                let shown = |field: UserField, written: &str| problems.iter().find(|problem| problem.field == field).filter(|_| touched.contains(&field) || !written.is_empty()).map(|problem| problem.text.clone());
                let body = div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(Self::field("user-new-username", "common:admin.username", username, shown(UserField::Username, &form.username), busy, cx))
                    .child(Self::field("user-new-password", "common:admin.password", password, shown(UserField::Password, &form.password), busy, cx))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(Self::field("user-new-name", "common:admin.thName", name, shown(UserField::Name, &form.name), busy, cx))
                            .child(Self::field("user-new-email", "common:admin.thEmail", email, shown(UserField::Email, &form.email), busy, cx)),
                    )
                    .child(Checkbox::new("user-new-superadmin").small().checked(*superadmin).label(tr(cx, "common:admin.superadminAll")).on_click(cx.listener(|this, checked: &bool, _, cx| {
                        if let Some(Dialog::New { superadmin, .. }) = &mut this.dialog {
                            *superadmin = *checked;
                        }
                        cx.notify();
                    })));
                let blocked = (!problems.is_empty()).then(|| problems.iter().map(|problem| problem.text.clone()).collect::<Vec<_>>().join(" · "));
                (tr(cx, "common:admin.newUser"), tr(cx, "common:admin.createUserLead"), body, "common:admin.create", "common:admin.creating", blocked)
            }
            Dialog::Edit { user, name, email, password, superadmin } => {
                let myself = user.id == me;
                let written = password.read(cx).value().chars().count();
                let short = (written > 0 && written < 8).then(|| text_with(locale(cx), "common:admin.passwordShort", &[("count", &written.to_string())]));
                let body = div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().flex().gap_3().child(Self::field("user-edit-name", "common:admin.thName", name, None, busy, cx)).child(Self::field("user-edit-email", "common:admin.thEmail", email, None, busy, cx)))
                    .child(Self::field("user-edit-password", "common:admin.newPasswordOptional", password, short.clone(), busy, cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Checkbox::new("user-edit-superadmin").small().checked(*superadmin).disabled(myself).label(tr(cx, "common:admin.superadmin")).on_click(cx.listener(|this, checked: &bool, _, cx| {
                                if let Some(Dialog::Edit { superadmin, .. }) = &mut this.dialog {
                                    *superadmin = *checked;
                                }
                                cx.notify();
                            })))
                            .when(myself, |row| row.child(div().text_xs().text_color(muted).child(tr(cx, "common:admin.cantChangeOwnRole")))),
                    );
                (text_with(locale(cx), "common:admin.editUser", &[("username", &user.username)]), tr(cx, "common:admin.blankKeeps"), body, "common:admin.save", "common:admin.saving", short)
            }
        };
        let card = div()
            .id("user-dialog")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => this.close(cx),
                    "enter" => this.save(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .w_full()
            .max_w(px(512.))
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded(px(14.))
            .border_1()
            .border_color(foreground.opacity(0.1))
            .bg(background)
            .shadow_2xl()
            .leading_sm()
            .text_color(foreground)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().id("user-dialog-title").aria_label(title.clone()).text_size(rems(1.)).font_weight(FontWeight::SEMIBOLD).child(title).test_support())
                    .child(div().leading_xs().text_color(muted).child(lead)),
            )
            .child(body)
            .children(self.problem.clone().map(|problem| {
                div().id("user-dialog-problem").flex().items_center().gap_1p5().leading_xs().text_color(danger).child(Icon::new(Lucide::CircleAlert).size_3()).child(problem).test_support()
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("user-dialog-cancel").outline().small().label(tr(cx, "common:admin.cancel")).on_click(cx.listener(|this, _, _, cx| this.close(cx))))
                    .child(
                        Button::new("user-dialog-save")
                            .primary()
                            .small()
                            .label(tr(cx, if busy { saving_key } else { save_key }))
                            // Apagado nunca en silencio: dice todo lo que falta.
                            .when_some(blocked.clone(), |button, why| button.tooltip(why))
                            .disabled(busy || blocked.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .test_support();
        Some(super::veil("user-dialog-veil", card, window).into_any_element())
    }
}

impl Render for UsersView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.users.clone().unwrap_or_default();
        let me = Backend::snapshot(cx).read(cx).session.as_ref().map(|session| session.id.clone()).unwrap_or_default();
        let theme = cx.theme();
        let (background, foreground, muted, border, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.danger);
        let heading = cells(COLUMNS.iter().map(|(key, _)| div().leading_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, key)).into_any_element()).collect()).h(px(36.));
        let rows: Vec<AnyElement> = if state.users.is_empty() {
            let phrase = tr(cx, if state.loading || !state.loaded { "common:admin.loading" } else { "common:admin.noUsers" });
            vec![div().id("users-empty").py_6().text_center().text_color(muted).border_t_1().border_color(border).child(phrase).test_support().into_any_element()]
        } else {
            state.users.iter().map(|user| self.row(user, &me, cx)).collect()
        };
        let problem = state.error.clone().or_else(|| self.problem.clone().filter(|_| self.dialog.is_none()));
        let dialog = self.render_dialog(window, cx);
        div()
            .relative()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(
                div().id("users-page").size_full().overflow_y_scroll().child(
                    div()
                        .mx_auto()
                        .w_full()
                        .max_w(px(896.))
                        .p_6()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .leading_sm()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(div().id("users-title").flex().items_center().gap_3().text_size(rems(1.25)).font_weight(FontWeight::SEMIBOLD).child(Icon::new(Lucide::Users).size_6().text_color(muted)).child(tr(cx, "common:admin.users")).test_support())
                                .child(Button::new("users-add").primary().small().icon(Icon::new(Lucide::Plus)).label(tr(cx, "common:admin.newUser")).on_click(cx.listener(|this, _, window, cx| this.open_new(window, cx)))),
                        )
                        .child(div().text_color(muted).child(tr(cx, "common:admin.usersLead")))
                        .children(problem.map(|problem| div().id("users-error").text_color(danger).child(problem).test_support()))
                        .child(div().id("users-table").flex().flex_col().rounded(px(8.)).border_1().border_color(border).child(heading).children(rows).test_support()),
                ),
            )
            .children(dialog)
    }
}

fn cells(cells: Vec<AnyElement>) -> Div {
    div().flex().items_center().px_3().gap_3().children(cells.into_iter().zip(COLUMNS).map(|(cell, (_, share))| div().flex_grow(share).flex_basis(px(0.)).min_w_0().child(cell)))
}
