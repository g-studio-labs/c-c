//! La pestaña de backups de un swarm (`ServerBackups.tsx`, #166): las bases
//! que se respaldan, con su horario, su historial y el log de cada backup.
//!
//! El backup lo hace el agente dentro del contenedor de la base, lo cifra con
//! age y lo sube al bucket: aquí se configura y se mira. Leer es de
//! cualquiera; configurar y lanzar, de quien administra (lo decide el core).

use cac_core::{detect_databases, server_edit_phrase, Backup, BackupStatus, BackupTarget, BackupsState, LogLine, PlatformState};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::log_view::LogBox;
use crate::theme::Leading as _;

use super::backup_form::{BackupFormClosed, BackupFormDialog};
use super::services::{empty, pill};

pub struct BackupsView {
    form: Option<(Entity<BackupFormDialog>, Subscription)>,
    busy: bool,
    /// Cómo acabó lo último que se pidió: la frase y si falló.
    line: Option<(String, bool)>,
    logs: LogBox,
}

type Work<T> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, cac_core::ServerEditError>> + Send>>;

impl BackupsView {
    pub fn new() -> Self {
        BackupsView { form: None, busy: false, line: None, logs: LogBox::new() }
    }

    /// Lo que se pide al backend, con su fallo dicho arriba.
    fn ask<T: Send + 'static>(&mut self, cx: &mut Context<Self>, work: impl FnOnce(cac_core::Core) -> Work<T> + Send + 'static, done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static, fallback_key: &'static str) {
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

    fn open_form(&mut self, target: Option<&BackupTarget>, detected: Option<&cac_core::DetectedDatabase>, state: &PlatformState, window: &mut Window, cx: &mut Context<Self>) {
        let form = match target {
            Some(target) => target.form(),
            None => Backend::core(cx).platform().new_backup_form(detected),
        };
        let services: Vec<String> = state.services.iter().flatten().map(|service| service.name.clone()).collect();
        let every = state.backups.as_ref().and_then(|backups| backups.status.as_ref()).map(|status| status.every_hours.clone()).unwrap_or_default();
        let target_id = target.map(|target| target.id.clone());
        let dialog = cx.new(|cx| BackupFormDialog::new(target_id, form, services, every, window, cx));
        let closed = cx.subscribe(&dialog, |this, _, _: &BackupFormClosed, cx| {
            this.form = None;
            cx.notify();
        });
        self.form = Some((dialog, closed));
        self.line = None;
        cx.notify();
    }

    /// Pregunta nombrando la base, y dice que lo que hay se queda.
    fn ask_delete(&mut self, target: BackupTarget, window: &mut Window, cx: &mut Context<Self>) {
        let title = cac_core::text_with(locale(cx), "common:backups.deleteTitle", &[("db", &target.database)]);
        let (body, confirm, cancel) = (tr(cx, "common:backups.deleteBody"), tr(cx, "common:backups.delete"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                let id = target.id.clone();
                this.ask(cx, move |core| Box::pin(async move { core.platform().delete_backup_target(&id).await }), |_, _, _| {}, "common:misc.errSave");
            })
            .ok();
        })
        .detach();
    }

    fn run_now(&mut self, target: BackupTarget, cx: &mut Context<Self>) {
        self.ask(
            cx,
            move |core| {
                Box::pin(async move {
                    let started = core.platform().run_backup(&target.id).await?;
                    core.platform().open_backup_log(&started.id).await;
                    Ok(())
                })
            },
            |_, _, _| {},
            "common:backups.runNow",
        );
    }

    /// Restaura una copia entera en la misma instancia y la revisa: saca de
    /// la memoria de la base los datos de la web y la deja más lenta unos
    /// minutos (boaty, 18-oct-2026). La automática va de madrugada; a mano,
    /// que se sepa antes.
    fn ask_test(&mut self, target: BackupTarget, window: &mut Window, cx: &mut Context<Self>) {
        let title = tr(cx, "common:backups.testTitle");
        let body = cac_core::text_with(locale(cx), "common:backups.testBody", &[("db", &target.database)]);
        let (confirm, cancel) = (tr(cx, "common:backups.testNow"), tr(cx, "common:servers.cancel"));
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
                            let started = core.platform().test_backup_restore(&target.id).await?;
                            core.platform().open_backup_log(&started.id).await;
                            Ok(())
                        })
                    },
                    |_, _, _| {},
                    "common:backups.testNow",
                )
            })
            .ok();
        })
        .detach();
    }

    fn notice(id: &'static str, text: String, cx: &App) -> AnyElement {
        let warning = cx.theme().warning;
        div().id(id).p_2().rounded(px(6.)).border_1().border_color(warning.opacity(0.4)).bg(warning.opacity(0.1)).leading_sm().child(text).test_support().into_any_element()
    }

    fn detected(&self, state: &PlatformState, backups: &BackupsState, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ready = state.permissions.manage_backups && backups.status.as_ref().is_some_and(|status| status.enabled && status.has_recipients);
        let found = detect_databases(state.services.as_deref().unwrap_or_default(), backups.targets.as_deref().unwrap_or_default());
        if !ready || found.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let rows = found.into_iter().map(|database| {
            let picked = database.clone();
            div()
                .id(SharedString::from(format!("backup-detected-{}", database.service_name)))
                .flex()
                .items_center()
                .gap_2()
                .child(pill(tr(cx, database.engine.label_key()), border, muted))
                .child(div().flex_1().min_w_0().truncate().font_family(cx.theme().mono_font_family.clone()).text_xs().child(database.service_name.clone()))
                .child(
                    Button::new(SharedString::from(format!("backup-configure-{}", database.service_name)))
                        .ghost()
                        .xsmall()
                        .label(tr(cx, "common:backups.configure"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let state = Backend::snapshot(cx).read(cx).platform.clone();
                            this.open_form(None, Some(&picked), &state, window, cx);
                        })),
                )
                .test_support()
        });
        Some(
            div()
                .id("backup-detected")
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .leading_sm()
                .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:backups.detected")))
                .children(rows)
                .test_support()
                .into_any_element(),
        )
    }

    fn target_card(&self, target: &BackupTarget, state: &PlatformState, backups: &BackupsState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, danger, mono) = (theme.border, theme.muted_foreground, theme.danger, theme.mono_font_family.clone());
        let language = locale(cx);
        let admin = state.permissions.manage_backups;
        let live = backups.busy(&target.id);
        let agent_ready = backups.status.as_ref().is_some_and(|status| status.enabled && status.agent_ready);
        let blocked = self.busy || live || !agent_ready;
        let actions = admin.then(|| {
            let (run, test, edit, delete) = (target.clone(), target.clone(), target.clone(), target.clone());
            div()
                .flex()
                .items_center()
                .gap_0p5()
                .child(
                    Button::new(SharedString::from(format!("backup-run-{}", target.id)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::Play))
                        .label(tr(cx, "common:backups.runNow"))
                        .disabled(blocked)
                        .on_click(cx.listener(move |this, _, _, cx| this.run_now(run.clone(), cx))),
                )
                .child(
                    Button::new(SharedString::from(format!("backup-test-{}", target.id)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::FlaskConical))
                        .label(tr(cx, "common:backups.testNow"))
                        .disabled(blocked || target.last_success_at.is_none())
                        .on_click(cx.listener(move |this, _, window, cx| this.ask_test(test.clone(), window, cx))),
                )
                .child(
                    Button::new(SharedString::from(format!("backup-edit-{}", target.id)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::Pencil))
                        .tooltip(tr(cx, "common:backups.edit"))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let state = Backend::snapshot(cx).read(cx).platform.clone();
                            this.open_form(Some(&edit), None, &state, window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("backup-delete-{}", target.id)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::Trash))
                        .tooltip(tr(cx, "common:backups.delete"))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, window, cx| this.ask_delete(delete.clone(), window, cx))),
                )
        });
        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(pill(tr(cx, target.engine.label_key()), border, muted))
            .child(div().font_family(mono.clone()).child(target.database.clone()))
            .child(div().font_family(mono.clone()).text_xs().text_color(muted).child(target.service_name.clone()))
            .when(!target.enabled, |row| row.child(pill(tr(cx, "common:backups.paused"), theme.secondary, theme.secondary_foreground)))
            .child(div().flex_1())
            .children(actions);
        let now = chrono::Local::now();
        let fact = |label_key: &str, value: String, color: Hsla| {
            div().flex_1().min_w_0().flex().flex_col().child(div().text_xs().text_color(muted).child(tr(cx, label_key))).child(div().text_xs().text_color(color).child(value))
        };
        let schedule = cac_core::text_with(language, "common:backups.scheduleValue", &[("hours", &target.every_hours.to_string()), ("at", &target.anchor_time), ("tz", &target.timezone)]);
        let next = target.next_run_at.as_deref().map(local_date_time).unwrap_or_else(|| "—".into());
        let last_good = target.last_success_at.as_deref().map(|at| cac_core::time_ago(at, &now, language)).unwrap_or_else(|| "—".into());
        let last_test = match (&target.last_restore_test_at, target.last_restore_test_ok) {
            (Some(at), ok) => {
                let when = cac_core::time_ago(at, &now, language);
                cac_core::text_with(language, if ok == Some(true) { "common:backups.testOk" } else { "common:backups.testFailed" }, &[("when", &when)])
            }
            (None, _) => "—".into(),
        };
        let test_color = if target.last_restore_test_ok == Some(false) { danger } else { theme.foreground };
        let facts = div()
            .flex()
            .gap_4()
            .child(fact("common:backups.schedule", schedule, theme.foreground))
            .child(fact("common:backups.next", next, theme.foreground))
            .child(fact("common:backups.lastGood", last_good, theme.foreground))
            .child(div().id(SharedString::from(format!("backup-last-test-{}", target.id))).flex_1().min_w_0().aria_label(last_test.clone()).child(fact("common:backups.lastTest", last_test, test_color)).test_support());
        let runs = backups.runs_of(&target.id);
        let log = backups.log_of.as_ref().and_then(|id| runs.iter().find(|run| &run.id == id)).map(|run| self.log_panel(run, backups, cx));
        let history = (!runs.is_empty() && log.is_none()).then(|| {
            div().id(SharedString::from(format!("backup-history-{}", target.id))).flex().flex_col().rounded(px(8.)).border_1().border_color(border).children(runs.iter().map(|run| history_row(run, cx))).test_support()
        });
        div()
            .id(SharedString::from(format!("backup-target-{}", target.id)))
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .leading_sm()
            .child(header)
            .child(facts)
            .children(history)
            .children(log)
            .test_support()
            .into_any_element()
    }

    fn log_panel(&self, run: &Backup, backups: &BackupsState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger, mono) = (theme.muted_foreground, theme.danger, theme.mono_font_family.clone());
        let lines: Vec<LogLine> = backups.log.lines().map(|line| LogLine { spans: cac_core::ansi_spans(line) }).collect();
        let (bg, fg) = status_colors(run.status, cx);
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(Button::new("backup-log-close").ghost().xsmall().icon(Icon::new(Lucide::ArrowLeft)).label(tr(cx, "common:deploy.backToHistory")).on_click(|_, _, cx| Backend::core(cx).platform().close_backup_log()))
            .child(div().id("backup-log-status").aria_label(tr(cx, run.status.label_key())).child(pill(tr(cx, run.status.label_key()), bg, fg)).test_support())
            .child(div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(tr(cx, run.kind.label_key())));
        let error = (!run.error.is_empty()).then(|| div().leading_xs().text_color(danger).child(run.error.clone()));
        div().id("backup-log-panel").flex().flex_col().gap_2().child(header).children(error).child(self.logs.render("backup-log", &lines, "…".to_string(), mono)).test_support().into_any_element()
    }
}

fn history_row(run: &Backup, cx: &mut Context<BackupsView>) -> AnyElement {
    let theme = cx.theme();
    let (border, muted) = (theme.border, theme.muted_foreground);
    let language = locale(cx);
    let (bg, fg) = status_colors(run.status, cx);
    let mut detail = Vec::new();
    if !run.created_at.is_empty() {
        detail.push(cac_core::time_ago(&run.created_at, &chrono::Local::now(), language));
    }
    if run.size_bytes > 0 {
        detail.push(cac_core::format_bytes(run.size_bytes as u64));
    }
    if !run.error.is_empty() {
        detail.push(run.error.clone());
    }
    let class = (!run.retention_class.is_empty()).then(|| div().text_xs().text_color(muted).child(tr(cx, &format!("common:backups.class.{}", run.retention_class))));
    let opened = run.id.clone();
    div()
        .id(SharedString::from(format!("backup-row-{}", run.id)))
        .aria_label(tr(cx, run.status.label_key()))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1p5()
        .border_t_1()
        .border_color(border)
        .child(pill(tr(cx, run.status.label_key()), bg, fg))
        .child(div().text_xs().child(tr(cx, run.kind.label_key())))
        .children(class)
        .child(div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(detail.join(" · ")))
        .child(Button::new(SharedString::from(format!("backup-log-{}", run.id))).ghost().xsmall().label(tr(cx, "common:backups.log")).on_click(move |_, _, cx| {
            let id = opened.clone();
            Backend::run(cx, move |core| async move { core.platform().open_backup_log(&id).await });
        }))
        .test_support()
        .into_any_element()
}

/// Un instante como fecha y hora de esta máquina.
fn local_date_time(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso).map(|at| at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|_| iso.to_string())
}

fn status_colors(status: BackupStatus, cx: &App) -> (Hsla, Hsla) {
    let theme = cx.theme();
    match status {
        BackupStatus::Succeeded => (theme.success.opacity(0.15), theme.success),
        BackupStatus::Failed => (theme.danger.opacity(0.15), theme.danger),
        BackupStatus::Running => (theme.secondary, theme.secondary_foreground),
        _ => (theme.border, theme.muted_foreground),
    }
}

impl Render for BackupsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let Some(backups) = state.backups.clone().filter(|backups| Some(&backups.server_id) == state.open_server.as_ref()) else {
            return empty("backups-loading", tr(cx, "common:servers.loading"), muted);
        };
        let language = locale(cx);
        let status = backups.status.clone();
        let mut notices = Vec::new();
        if let Some(status) = &status {
            if !status.enabled {
                notices.push(Self::notice("backups-disabled", tr(cx, "common:backups.disabled"), cx));
            } else if !status.agent_ready {
                let text = cac_core::text_with(language, "common:backups.agentOld", &[("version", &status.agent_version.to_string()), ("min", &status.min_agent.to_string())]);
                notices.push(Self::notice("backups-agent-old", text, cx));
            }
        }
        let can_add = state.permissions.manage_backups && status.as_ref().is_some_and(|status| status.enabled && status.has_recipients);
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(Icon::new(Lucide::Archive).size_5().text_color(muted))
            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).leading_lg().child(tr(cx, "common:backups.title")))
            .when(state.permissions.manage_backups, |row| {
                row.child(
                    Button::new("backup-add")
                        .primary()
                        .small()
                        .icon(Icon::new(Lucide::Plus))
                        .label(tr(cx, "common:backups.add"))
                        .disabled(!can_add || self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let state = Backend::snapshot(cx).read(cx).platform.clone();
                            let first = state.backups.as_ref().map(|backups| detect_databases(state.services.as_deref().unwrap_or_default(), backups.targets.as_deref().unwrap_or_default())).and_then(|found| found.into_iter().next());
                            this.open_form(None, first.as_ref(), &state, window, cx);
                        })),
                )
            });
        let error = backups.error.clone().map(|error| div().id("backups-error").leading_xs().text_color(danger).child(error).test_support());
        let line = self.line.clone().map(|(phrase, failed)| div().id("backups-result").aria_label(phrase.clone()).leading_xs().text_color(if failed { danger } else { muted }).child(phrase).test_support());
        let targets = backups.targets.clone().unwrap_or_default();
        let none = (backups.targets.is_some() && targets.is_empty()).then(|| div().id("backups-empty").leading_sm().text_color(muted).child(tr(cx, "common:backups.empty")).test_support());
        let cards: Vec<AnyElement> = targets.iter().map(|target| self.target_card(target, &state, &backups, cx)).collect();
        let detected = self.detected(&state, &backups, cx);
        div()
            .id("server-backups")
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .child(div().leading_sm().text_color(muted).child(tr(cx, "common:backups.lead")))
            .children(notices)
            .children(error)
            .children(line)
            .children(detected)
            .children(none)
            .children(cards)
            .children(self.form.as_ref().map(|(dialog, _)| dialog.clone()))
            .test_support()
            .into_any_element()
    }
}

