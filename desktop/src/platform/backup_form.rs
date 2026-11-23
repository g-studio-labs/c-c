//! El diálogo de una base que se respalda (`TargetDialog` de
//! `ServerBackups.tsx`): motor, servicio o contenedor, base, variable de la
//! credencial, horario y retención. Qué hace falta y cómo cambia la variable
//! con el motor o el usuario lo decide el core (`BackupTargetForm`).

use cac_core::{server_edit_phrase, BackupEngine, BackupTargetForm};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// El diálogo se cerró: quien lo tiene lo quita.
pub struct BackupFormClosed;

pub struct BackupFormDialog {
    /// La base que se edita; sin ella, es un alta.
    target_id: Option<String>,
    form: BackupTargetForm,
    /// Los servicios del swarm, para elegir; un contenedor fuera de Swarm se
    /// escribe por su nombre.
    services: Vec<String>,
    every_hours: Vec<i64>,
    by_name: bool,
    container: Entity<InputState>,
    database: Entity<InputState>,
    credential_env: Entity<InputState>,
    username: Entity<InputState>,
    anchor_time: Entity<InputState>,
    timezone: Entity<InputState>,
    keep_days: Entity<InputState>,
    keep_weekly: Entity<InputState>,
    keep_monthly: Entity<InputState>,
    restore_every: Entity<InputState>,
    busy: bool,
    problem: Option<String>,
    _changed: Vec<Subscription>,
}

impl EventEmitter<BackupFormClosed> for BackupFormDialog {}

impl BackupFormDialog {
    pub fn new(target_id: Option<String>, form: BackupTargetForm, services: Vec<String>, every_hours: Vec<i64>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut changed = Vec::new();
        let mut input = |value: String, placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_string();
            let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value));
            changed.push(cx.subscribe_in(&state, window, |this: &mut Self, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.read_inputs(window, cx);
                }
            }));
            state
        };
        let by_name = !form.service_name.is_empty() && !services.contains(&form.service_name);
        let container = input(if by_name { form.service_name.clone() } else { String::new() }, "boaty-postgres", window, cx);
        let database = input(form.database.clone(), "boaty", window, cx);
        let credential_env = input(form.credential_env.clone(), "", window, cx);
        let username = input(form.username.clone(), "postgres", window, cx);
        let anchor_time = input(form.anchor_time.clone(), "03:30", window, cx);
        let timezone = input(form.timezone.clone(), "America/Mexico_City", window, cx);
        let keep_days = input(form.keep_days.to_string(), "", window, cx);
        let keep_weekly = input(form.keep_weekly.to_string(), "", window, cx);
        let keep_monthly = input(form.keep_monthly.to_string(), "", window, cx);
        let restore_every = input(form.restore_test_every_days.to_string(), "", window, cx);
        database.update(cx, |input, cx| input.focus(window, cx));
        let every_hours = if every_hours.is_empty() { vec![24] } else { every_hours };
        BackupFormDialog {
            target_id,
            form,
            services,
            every_hours,
            by_name,
            container,
            database,
            credential_env,
            username,
            anchor_time,
            timezone,
            keep_days,
            keep_weekly,
            keep_monthly,
            restore_every,
            busy: false,
            problem: None,
            _changed: changed,
        }
    }

    /// Lo escrito, al formulario. Un usuario nuevo puede cambiar la variable
    /// de la credencial: se escribe de vuelta en su caja.
    fn read_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let number = |input: &Entity<InputState>, before: i64| value(input).trim().parse::<i64>().unwrap_or(if value(input).trim().is_empty() { 0 } else { before });
        if self.by_name {
            self.form.service_name = value(&self.container).trim().to_string();
        }
        self.form.database = value(&self.database);
        self.form.credential_env = value(&self.credential_env);
        self.form.anchor_time = value(&self.anchor_time).trim().to_string();
        self.form.timezone = value(&self.timezone);
        self.form.keep_days = number(&self.keep_days, self.form.keep_days);
        self.form.keep_weekly = number(&self.keep_weekly, self.form.keep_weekly);
        self.form.keep_monthly = number(&self.keep_monthly, self.form.keep_monthly);
        self.form.restore_test_every_days = number(&self.restore_every, self.form.restore_test_every_days);
        let typed = value(&self.username);
        if typed.trim() != self.form.username {
            let before = self.form.credential_env.clone();
            self.form.set_username(&typed);
            if self.form.credential_env != before {
                let env = self.form.credential_env.clone();
                self.credential_env.update(cx, |input, cx| input.set_value(env, window, cx));
            }
        }
        cx.notify();
    }

    fn set_engine(&mut self, engine: BackupEngine, window: &mut Window, cx: &mut Context<Self>) {
        self.form.set_engine(engine);
        let env = self.form.credential_env.clone();
        self.credential_env.update(cx, |input, cx| input.set_value(env, window, cx));
        self.username.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn choose_service(&mut self, service: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.by_name = service.is_none();
        self.form.service_name = service.unwrap_or_default();
        if self.by_name {
            self.container.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.form.ready() {
            return;
        }
        let (form, target_id) = (self.form.clone(), self.target_id.clone());
        self.busy = true;
        self.problem = None;
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move { core.platform().save_backup_target(target_id.as_deref(), &form).await.map(|_| ()) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => cx.emit(BackupFormClosed),
                    Err(error) => this.problem = Some(server_edit_phrase(locale(cx), &error, "common:misc.errSave")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(BackupFormClosed);
    }

    fn field(id: &'static str, label_key: &str, input: &Entity<InputState>, hint: Option<String>, busy: bool, cx: &App) -> Div {
        let muted = cx.theme().muted_foreground;
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, label_key)))
            .child(div().id(id).child(Input::new(input).disabled(busy)).test_support())
            .children(hint.map(|hint| div().text_xs().text_color(muted).child(hint)))
    }

    fn label(key: &str, cx: &App) -> Div {
        div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, key))
    }

    fn engines(&self, cx: &mut Context<Self>) -> Div {
        let choice = |engine: BackupEngine, id: &'static str, cx: &mut Context<Self>| {
            let chosen = self.form.engine == engine;
            Button::new(id)
                .small()
                .when(chosen, |button| button.primary())
                .when(!chosen, |button| button.outline())
                .label(tr(cx, engine.label_key()))
                .disabled(self.busy)
                .on_click(cx.listener(move |this, _, window, cx| this.set_engine(engine, window, cx)))
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(Self::label("common:backups.engineLabel", cx))
            .child(div().flex().gap_2().child(choice(BackupEngine::Mssql, "backup-engine-mssql", cx)).child(choice(BackupEngine::Postgres, "backup-engine-postgres", cx)))
    }

    fn service(&self, cx: &mut Context<Self>) -> Div {
        let container_label = tr(cx, "common:backups.byContainer");
        let shown = if self.by_name { container_label.clone() } else if self.form.service_name.is_empty() { "—".to_string() } else { self.form.service_name.clone() };
        let view = cx.entity().downgrade();
        let services = self.services.clone();
        let chosen = (!self.by_name).then(|| self.form.service_name.clone());
        let picker = crate::mywork::picker("backup-service", shown, Some(px(260.)), cx, move |mut menu, _, _| {
            for name in services.clone() {
                let view = view.clone();
                let pick = name.clone();
                menu = menu.item(PopupMenuItem::new(name.clone()).checked(chosen.as_deref() == Some(name.as_str())).on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.choose_service(Some(pick.clone()), window, cx));
                }));
            }
            let view = view.clone();
            menu.separator().item(PopupMenuItem::new(container_label.clone()).on_click(move |_, window, cx| {
                let _ = view.update(cx, |this, cx| this.choose_service(None, window, cx));
            }))
        });
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(Self::label("common:backups.service", cx))
            .child(picker)
            .when(self.by_name, |column| column.child(div().id("backup-container").child(Input::new(&self.container).disabled(self.busy)).test_support()))
    }

    fn every(&self, cx: &mut Context<Self>) -> Div {
        let language = locale(cx);
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(Self::label("common:backups.every", cx))
            .child(div().flex().flex_wrap().gap_1().children(self.every_hours.clone().into_iter().map(|hours| {
                let chosen = self.form.every_hours == hours;
                Button::new(SharedString::from(format!("backup-every-{hours}")))
                    .xsmall()
                    .when(chosen, |button| button.primary())
                    .when(!chosen, |button| button.outline())
                    .label(cac_core::text_with(language, "common:backups.everyValue", &[("hours", &hours.to_string())]))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.form.every_hours = hours;
                        cx.notify();
                    }))
            })))
    }

    /// Lo que dejan los tres números, dicho como frase: a qué se puede volver.
    fn retention_summary(&self, cx: &App) -> AnyElement {
        let language = locale(cx);
        let form = &self.form;
        let mut parts = vec![cac_core::text_count(language, "common:backups.summary.days", form.keep_days)];
        if form.keep_weekly > 0 {
            parts.push(cac_core::text_count(language, "common:backups.summary.weeks", form.keep_weekly));
        }
        if form.keep_monthly > 0 {
            parts.push(cac_core::text_count(language, "common:backups.summary.months", form.keep_monthly));
        }
        let total = cac_core::text_with(language, "common:backups.retained", &[("n", &form.retained_count().to_string())]);
        let text = format!("{} {}. {total}", tr(cx, "common:backups.summary.lead"), parts.join(", "));
        div().id("backup-retention").aria_label(text.clone()).px_3().py_2().rounded(px(6.)).bg(cx.theme().muted.opacity(0.6)).text_xs().child(text).test_support().into_any_element()
    }
}

impl Render for BackupFormDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, danger) = (theme.background, theme.foreground, theme.danger);
        let busy = self.busy;
        let title = tr(cx, if self.target_id.is_some() { "common:backups.editTitle" } else { "common:backups.addTitle" });
        let env_hint = tr(cx, if self.form.username.is_empty() { "common:backups.credentialEnvHint" } else { "common:backups.credentialEnvPasswordHint" });
        let postgres = self.form.engine == BackupEngine::Postgres;
        let ready = self.form.ready();
        let enabled = self.form.enabled;
        let card = div()
            .id("backup-form")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close(cx);
                    cx.stop_propagation();
                }
            }))
            .w_full()
            .max_w(px(640.))
            .max_h(window.viewport_size().height * 0.9)
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
            .child(div().id("backup-form-title").aria_label(title.clone()).text_size(rems(1.)).font_weight(FontWeight::SEMIBOLD).child(title).test_support())
            .child(
                // Los campos se deslizan; el título y los botones se quedan a
                // la vista en una ventana baja.
                div()
                    .id("backup-form-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().flex().gap_3().child(self.engines(cx)).child(self.service(cx)))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(Self::field("backup-database", "common:backups.database", &self.database, Some(tr(cx, "common:backups.databaseHint")), busy, cx))
                            .child(Self::field("backup-credential-env", "common:backups.credentialEnv", &self.credential_env, Some(env_hint), busy, cx)),
                    )
                    .when(postgres, |card| card.child(Self::field("backup-username", "common:backups.username", &self.username, Some(tr(cx, "common:backups.usernameHint")), busy, cx)))
                    .child(self.every(cx))
                    .child(div().flex().gap_3().child(Self::field("backup-at", "common:backups.at", &self.anchor_time, None, busy, cx)).child(Self::field("backup-timezone", "common:backups.timezone", &self.timezone, None, busy, cx)))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(Self::field("backup-keep-days", "common:backups.keepDays", &self.keep_days, None, busy, cx))
                            .child(Self::field("backup-keep-weekly", "common:backups.keepWeekly", &self.keep_weekly, None, busy, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(Self::field("backup-keep-monthly", "common:backups.keepMonthly", &self.keep_monthly, None, busy, cx))
                            .child(Self::field("backup-restore-every", "common:backups.restoreEvery", &self.restore_every, None, busy, cx)),
                    )
                    .child(Checkbox::new("backup-enabled").checked(enabled).label(tr(cx, "common:backups.enabled")).on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.form.enabled = *checked;
                        cx.notify();
                    })))
                    .child(self.retention_summary(cx)),
            )
            .children(self.problem.clone().map(|problem| {
                div().id("backup-form-problem").flex().items_center().gap_1p5().leading_xs().text_color(danger).child(Icon::new(Lucide::CircleAlert).size_3()).child(problem).test_support()
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("backup-form-cancel").outline().small().label(tr(cx, "common:servers.cancel")).on_click(cx.listener(|this, _, _, cx| this.close(cx))))
                    .child(
                        Button::new("backup-form-save")
                            .primary()
                            .small()
                            .label(tr(cx, "common:backups.save"))
                            .disabled(busy || !ready)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .test_support();
        super::veil("backup-form-veil", card, window)
    }
}
