//! La bandeja de peticiones de ejecución de un servidor
//! (`ProvisioningRequests.tsx`, decisión 4d28f4ab). Un agente, o quien ensayó
//! aquí, pide; una persona ve el ensayo y sus banderas, y aprueba o rechaza.
//! **Sólo en persona**: el backend no deja decidir con un token. Lo
//! autorizado se corre aquí («Correr ahora») o desde una terminal.

use std::collections::HashMap;
use std::time::Duration;

use cac_core::{can_approve, grant_command, is_stale, provision_phrase, time_ago, visible_requests, FlagLevel, PlatformState, ProvisionError, ProvisionState, ProvisioningRequest, Risk, Server, GRANT_MINUTES};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};

use super::provision::card;
use super::provision_engine::engine;
use super::services::pill;

/// Cada cuánto se relee mientras haya algo que se mueve.
const POLL: Duration = Duration::from_secs(15);

/// Lo escrito para decidir una petición.
struct Decision {
    note: Entity<InputState>,
    typed: Entity<InputState>,
    acknowledged: bool,
}

pub struct RequestsPanel {
    server_id: String,
    show_all: bool,
    decisions: HashMap<String, Decision>,
    /// Las que enseñan su diff o su salida.
    open_diff: Vec<String>,
    open_log: Vec<String>,
    busy: bool,
    line: Option<(String, bool)>,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}

impl RequestsPanel {
    pub fn new(server_id: String, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            let Ok(live) = this.update(cx, |this, cx| {
                let state = Backend::snapshot(cx).read(cx).platform.clone();
                let now = chrono::Utc::now();
                state.provision.as_ref().filter(|p| p.server_id == this.server_id).is_some_and(|p| p.requests.iter().any(|r| r.is_live() && !is_stale(r, now)))
            }) else {
                return;
            };
            if live {
                let _ = cx.update(|cx| Backend::run(cx, |core| async move { core.platform().reload_requests().await }));
            }
        });
        RequestsPanel { server_id, show_all: false, decisions: HashMap::new(), open_diff: Vec::new(), open_log: Vec::new(), busy: false, line: None, _subscriptions: Vec::new(), _poll: poll }
    }

    fn decision(&mut self, id: &str, server_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.decisions.contains_key(id) {
            return;
        }
        let note_hint = tr(cx, "common:provisionRequests.note");
        let typed_hint = server_name.to_string();
        let note = cx.new(|cx| InputState::new(window, cx).placeholder(note_hint));
        let typed = cx.new(|cx| InputState::new(window, cx).placeholder(typed_hint));
        for input in [&note, &typed] {
            self._subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        self.decisions.insert(id.to_string(), Decision { note, typed, acknowledged: false });
    }

    fn work(&mut self, cx: &mut Context<Self>, work: impl FnOnce(cac_core::Core) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ProvisionError>> + Send>> + Send + 'static, done_key: Option<&'static str>) {
        self.busy = true;
        self.line = None;
        cx.notify();
        let answer = Backend::ask(cx, work);
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                this.line = match result {
                    Ok(()) => done_key.map(|key| (tr(cx, key), false)),
                    Err(error) => Some((provision_phrase(locale(cx), &error, "common:misc.errSave"), true)),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn ask(&mut self, level: PromptLevel, title: String, body: String, confirm: String, window: &mut Window, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        let cancel = tr(cx, "common:servers.cancel");
        let answer = window.prompt(level, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |this, cx| then(this, cx)).ok();
            }
        })
        .detach();
    }

    fn approve(&mut self, request: &ProvisioningRequest, server: &Server, window: &mut Window, cx: &mut Context<Self>) {
        let Some(decision) = self.decisions.get(&request.id) else { return };
        let (note, typed, acknowledged) = (decision.note.read(cx).value().to_string(), decision.typed.read(cx).value().to_string(), decision.acknowledged);
        let language = locale(cx);
        let title = cac_core::text_with(language, "common:provisionRequests.approveTitle", &[("playbook", &request.playbook), ("server", &server.name)]);
        let body = cac_core::text_with(language, "common:provisionRequests.approveBody", &[("seq", &request.config_seq.to_string()), ("minutes", &GRANT_MINUTES.to_string())]);
        let level = if request.risk == Risk::Destructive { PromptLevel::Critical } else { PromptLevel::Warning };
        let id = request.id.clone();
        self.ask(level, title, body, tr(cx, "common:provisionRequests.approve"), window, cx, move |this, cx| {
            this.work(cx, move |core| Box::pin(async move { core.platform().decide_request(&id, true, &note, &typed, acknowledged).await }), Some("common:provisionRequests.approved"));
        });
    }

    fn reject(&mut self, request: &ProvisioningRequest, cx: &mut Context<Self>) {
        let note = self.decisions.get(&request.id).map(|d| d.note.read(cx).value().to_string()).unwrap_or_default();
        let id = request.id.clone();
        self.work(cx, move |core| Box::pin(async move { core.platform().decide_request(&id, false, &note, "", false).await }), Some("common:provisionRequests.rejected"));
    }

    fn discard(&mut self, request: &ProvisioningRequest, window: &mut Window, cx: &mut Context<Self>) {
        let title = cac_core::text_with(locale(cx), "common:provisionRequests.discardTitle", &[("playbook", &request.playbook)]);
        let id = request.id.clone();
        self.ask(PromptLevel::Warning, title, tr(cx, "common:provisionRequests.discardBody"), tr(cx, "common:provisionRequests.discard"), window, cx, move |this, cx| {
            this.work(cx, move |core| Box::pin(async move { core.platform().discard_request(&id).await }), Some("common:provisionRequests.discarded"));
        });
    }

    fn reverify(&mut self, request: &ProvisioningRequest, window: &mut Window, cx: &mut Context<Self>) {
        let title = cac_core::text_with(locale(cx), "common:provisionRequests.reverifyTitle", &[("verify", &request.verify)]);
        let id = request.id.clone();
        self.ask(PromptLevel::Warning, title, tr(cx, "common:provisionRequests.reverifyBody"), tr(cx, "common:provisionRequests.reverify"), window, cx, move |this, cx| {
            this.work(cx, move |core| Box::pin(async move { core.platform().reverify_request(&id).await }), None);
        });
    }

    /// Correr una autorizada, o la verificación que se quedó sin correr.
    fn run(&mut self, request: &ProvisioningRequest, provision: &ProvisionState, server: &Server, window: &mut Window, cx: &mut Context<Self>) {
        let manifest = provision.project.as_ref().and_then(|p| p.manifest.as_ref());
        let name_of = |id: &str| manifest.and_then(|m| m.playbook(id)).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string());
        let language = locale(cx);
        let verifying = request.status == "verifying";
        let shown = if verifying { name_of(&request.verify) } else { name_of(&request.playbook) };
        let title = cac_core::text_with(language, "common:provision.confirmTitle", &[("playbook", &shown), ("server", &server.name)]);
        let body = if verifying {
            cac_core::text_with(language, "common:provision.confirmVerifyOnly", &[("playbook", &name_of(&request.playbook))])
        } else {
            let verify = if request.verify.is_empty() { "—".to_string() } else { request.verify.clone() };
            cac_core::text_with(language, "common:provision.confirmGrant", &[("seq", &request.config_seq.to_string()), ("verify", &verify)])
        };
        let level = if request.risk == Risk::Destructive { PromptLevel::Critical } else { PromptLevel::Warning };
        let (id, engine) = (request.id.clone(), engine(cx));
        self.ask(level, title, body, tr(cx, "common:provision.run"), window, cx, move |this, cx| {
            this.work(cx, move |core| Box::pin(async move { core.platform().run_grant(engine, &id).await }), None);
        });
    }

    fn row(&mut self, request: &ProvisioningRequest, provision: &ProvisionState, server: &Server, can_run: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let awaiting = request.status == "awaiting_approval";
        if awaiting && provision.permissions.decide {
            self.decision(&request.id, &server.name, window, cx);
        }
        let theme = cx.theme().clone();
        let (muted, danger, warning, border, mono) = (theme.muted_foreground, theme.danger, theme.warning, theme.border, theme.mono_font_family.clone());
        let stale = is_stale(request, chrono::Utc::now());
        let id = request.id.clone();
        let (risk_bg, risk_fg) = match request.risk {
            Risk::Read => (theme.secondary, theme.secondary_foreground),
            Risk::Change => (theme.primary.opacity(0.15), theme.primary),
            Risk::Destructive => (danger.opacity(0.15), danger),
        };
        let status = if stale { tr(cx, "common:provisionRequests.staleBadge") } else { tr(cx, &format!("common:provisionRequests.status.{}", request.status)) };
        let asked = cac_core::text_with(locale(cx), "common:provisionRequests.askedBy", &[("name", if request.requested_by_name.is_empty() { "?" } else { &request.requested_by_name }), ("when", &time_ago(&request.created_at, &chrono::Local::now(), locale(cx))), ("seq", &request.config_seq.to_string())]);
        let asked = if request.source == "mcp" { format!("{asked} · {}", tr(cx, "common:provisionRequests.viaAgent")) } else { asked };
        let mut column = div()
            .id(SharedString::from(format!("request-{id}")))
            .aria_label(status.clone())
            .flex()
            .flex_col()
            .gap_1p5()
            .p_2()
            .rounded(px(6.))
            .border_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .child(div().font_family(mono.clone()).text_xs().child(request.playbook.clone()))
                    .child(pill(tr(cx, &format!("common:provisionRequests.risk.{}", request.risk.as_str())), risk_bg, risk_fg))
                    .child(div().id(SharedString::from(format!("request-status-{id}"))).aria_label(status.clone()).child(pill(status, theme.border, muted)).test_support())
                    .when(request.auto_approved, |row| row.child(pill(tr(cx, "common:provisionRequests.preAuthorized"), theme.secondary, theme.secondary_foreground))),
            )
            .child(div().text_xs().text_color(muted).child(asked))
            .when(!request.reason.is_empty(), |c| c.child(div().text_xs().child(request.reason.clone())));

        if stale {
            let target = request.clone();
            column = column.child(
                div().flex().flex_wrap().items_center().gap_2().text_xs().text_color(muted).child(tr(cx, "common:provisionRequests.stale")).when(provision.owns(request), |row| {
                    row.child(Button::new(SharedString::from(format!("request-discard-{id}"))).outline().xsmall().label(tr(cx, "common:provisionRequests.discard")).disabled(self.busy).on_click(cx.listener(move |this, _, window, cx| this.discard(&target, window, cx))))
                }),
            );
        }
        if !request.flags.is_empty() {
            let flags = request.flags.iter().enumerate().map(|(n, flag)| {
                let color = match flag.level {
                    FlagLevel::Red => danger,
                    FlagLevel::Amber => warning,
                    FlagLevel::Info => muted,
                };
                let key = format!("common:provisionRequests.flag.{}", flag.code);
                let text = tr(cx, &key);
                let text = if text == key { flag.code.clone() } else { text };
                div()
                    .id(SharedString::from(format!("request-flag-{id}-{n}")))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .child(div().size_2().flex_none().rounded_full().bg(color))
                    .child(text)
                    .when(!flag.detail.is_empty(), |row| row.child(div().font_family(mono.clone()).text_color(muted).child(flag.detail.clone())))
                    .test_support()
            });
            column = column.child(div().flex().flex_col().gap_0p5().children(flags.collect::<Vec<_>>()));
        }
        if let Some(check) = &request.check {
            let summary = cac_core::text_with(locale(cx), "common:provisionRequests.checkSummary", &[("hosts", &if check.hosts.is_empty() { "—".to_string() } else { check.hosts.join(", ") }), ("changed", &check.changed.to_string()), ("ok", &check.ok.to_string())]);
            let failures = check.failures.iter().map(|failure| {
                div().p_1p5().rounded(px(4.)).border_1().border_color(danger.opacity(0.4)).bg(danger.opacity(0.05)).child(div().font_family(mono.clone()).text_xs().child(if failure.task.is_empty() { "?".to_string() } else { failure.task.clone() })).child(div().text_xs().text_color(danger).child(failure.message.clone()))
            });
            let tasks = check.changed_tasks.iter().map(|task| div().text_xs().child(format!("• {task}")));
            let diff_open = self.open_diff.contains(&id);
            let log_open = self.open_log.contains(&id);
            let toggle = |name: &'static str, open: bool, show: &str, hide: &str, cx: &mut Context<Self>| {
                let key = id.clone();
                Button::new(SharedString::from(format!("request-{name}-{id}"))).ghost().xsmall().label(tr(cx, if open { hide } else { show })).on_click(cx.listener(move |this, _, _, cx| {
                    let list = if name == "diff" { &mut this.open_diff } else { &mut this.open_log };
                    if let Some(at) = list.iter().position(|known| known == &key) {
                        list.remove(at);
                    } else {
                        list.push(key.clone());
                    }
                    cx.notify();
                }))
            };
            let block = |id: String, text: &str| div().id(SharedString::from(id)).p_2().rounded(px(4.)).bg(theme.muted).font_family(mono.clone()).text_xs().children(text.lines().map(|line| div().whitespace_nowrap().child(line.to_string())).collect::<Vec<_>>()).test_support();
            column = column
                .child(div().id(SharedString::from(format!("request-check-{id}"))).text_xs().text_color(muted).child(summary).test_support())
                .child(div().flex().flex_col().gap_1().children(failures.collect::<Vec<_>>()))
                .child(div().flex().flex_col().children(tasks.collect::<Vec<_>>()))
                .when(!check.diff.is_empty(), |c| c.child(toggle("diff", diff_open, "common:provisionRequests.showDiff", "common:provisionRequests.hideDiff", cx)).when(diff_open, |c| c.child(block(format!("request-diff-text-{id}"), &check.diff))))
                .when(!check.log_tail.is_empty(), |c| c.child(toggle("log", log_open, "common:provisionRequests.showLog", "common:provisionRequests.hideLog", cx)).when(log_open, |c| c.child(block(format!("request-log-text-{id}"), &check.log_tail))));
        }
        if awaiting && provision.permissions.decide {
            column = column.child(self.decide_controls(request, server, cx));
        }
        if awaiting && !provision.permissions.decide {
            column = column.child(div().id(SharedString::from(format!("request-admin-decides-{id}"))).text_xs().text_color(muted).child(tr(cx, "common:provisionRequests.adminDecides")).test_support());
        }
        if request.status == "approved" {
            let command = grant_command(&server.name, &request.id);
            let until = request.grant_expires_at.as_deref().and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok()).map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string());
            let (copied, target, run_provision, run_server) = (command.clone(), request.clone(), provision.clone(), server.clone());
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pt_2()
                    .border_t_1()
                    .border_color(border)
                    .children(until.map(|time| div().text_xs().text_color(muted).child(cac_core::text_with(locale(cx), "common:provisionRequests.grantUntil", &[("time", &time)]))))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(div().id(SharedString::from(format!("request-command-{id}"))).aria_label(command.clone()).flex_1().min_w_0().truncate().px_1p5().py_1().rounded(px(4.)).bg(theme.muted).font_family(mono.clone()).text_xs().child(command).test_support())
                            .when(can_run, |row| {
                                row.child(Button::new(SharedString::from(format!("request-run-{id}"))).primary().xsmall().label(tr(cx, "common:provisionRequests.runNow")).on_click(cx.listener(move |this, _, window, cx| this.run(&target, &run_provision, &run_server, window, cx))))
                            })
                            .child(Button::new(SharedString::from(format!("request-copy-{id}"))).ghost().xsmall().icon(Icon::new(Lucide::Copy)).tooltip(tr(cx, "common:provisionRequests.copy")).on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                                this.line = Some((tr(cx, "common:provisionRequests.copied"), false));
                                cx.notify();
                            }))),
                    ),
            );
        }
        // Aplicada, pero su verificación no llegó a arrancar: se corre aquí,
        // sin otra aprobación.
        if request.status == "verifying" && !request.verify.is_empty() && request.verify_run_id.is_empty() && can_run {
            let (target, run_provision, run_server) = (request.clone(), provision.clone(), server.clone());
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .child(div().text_color(muted).child(cac_core::text_with(locale(cx), "common:provisionRequests.verifyPending", &[("verify", &request.verify)])))
                    .child(Button::new(SharedString::from(format!("request-verify-{id}"))).primary().xsmall().label(tr(cx, "common:provisionRequests.runVerify")).on_click(cx.listener(move |this, _, window, cx| this.run(&target, &run_provision, &run_server, window, cx)))),
            );
        }
        if matches!(request.status.as_str(), "verify_failed" | "failed" | "violated") && !request.rollback.is_empty() {
            column = column.child(div().id(SharedString::from(format!("request-rollback-{id}"))).text_xs().text_color(danger).child(cac_core::text_with(locale(cx), "common:provisionRequests.rollbackHint", &[("rollback", &request.rollback)])).test_support());
        }
        if request.status == "verify_failed" && !request.verify.is_empty() && provision.owns(request) {
            let target = request.clone();
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .child(div().text_color(muted).child(tr(cx, "common:provisionRequests.reverifyHint")))
                    .child(Button::new(SharedString::from(format!("request-reverify-{id}"))).outline().xsmall().label(tr(cx, "common:provisionRequests.reverify")).disabled(self.busy).on_click(cx.listener(move |this, _, window, cx| this.reverify(&target, window, cx)))),
            );
        }
        if !request.decision_note.is_empty() {
            column = column.child(div().text_xs().text_color(muted).child(cac_core::text_with(locale(cx), "common:provisionRequests.decisionNote", &[("name", &request.decided_by_name), ("note", &request.decision_note)])));
        }
        column.test_support().into_any_element()
    }

    /// Aprobar: el nombre del servidor escrito tal cual si es destructivo, y
    /// las banderas rojas reconocidas si las hay. Rechazar no pide nada.
    fn decide_controls(&self, request: &ProvisioningRequest, server: &Server, cx: &mut Context<Self>) -> AnyElement {
        let Some(decision) = self.decisions.get(&request.id) else { return div().into_any_element() };
        let id = request.id.clone();
        let typed = decision.typed.read(cx).value().to_string();
        let ready = !self.busy && can_approve(request, &server.name, &typed, decision.acknowledged);
        let (approving, rejecting, server_copy) = (request.clone(), request.clone(), server.clone());
        let acknowledging = id.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .when(request.has_red_flag(), |column| {
                column.child(Checkbox::new(SharedString::from(format!("request-ack-{id}"))).small().checked(decision.acknowledged).label(tr(cx, "common:provisionRequests.acknowledge")).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    if let Some(decision) = this.decisions.get_mut(&acknowledging) {
                        decision.acknowledged = *checked;
                    }
                    cx.notify();
                })))
            })
            .when(request.risk == Risk::Destructive, |column| {
                column
                    .child(div().text_xs().child(cac_core::text_with(locale(cx), "common:provisionRequests.typeServer", &[("name", &server.name)])))
                    .child(div().id(SharedString::from(format!("request-typed-{id}"))).child(Input::new(&decision.typed).small()).test_support())
            })
            .child(div().id(SharedString::from(format!("request-note-{id}"))).child(Input::new(&decision.note).small()).test_support())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(Button::new(SharedString::from(format!("request-approve-{id}"))).primary().xsmall().label(tr(cx, "common:provisionRequests.approve")).disabled(!ready).on_click(cx.listener(move |this, _, window, cx| this.approve(&approving, &server_copy, window, cx))))
                    .child(Button::new(SharedString::from(format!("request-reject-{id}"))).outline().xsmall().label(tr(cx, "common:provisionRequests.reject")).disabled(self.busy).on_click(cx.listener(move |this, _, _, cx| this.reject(&rejecting, cx)))),
            )
            .into_any_element()
    }
}

impl Render for RequestsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state: PlatformState = Backend::snapshot(cx).read(cx).platform.clone();
        let Some(provision) = state.provision.clone().filter(|p| p.server_id == self.server_id) else { return div().into_any_element() };
        let Some(server) = state.servers.iter().find(|s| s.id == self.server_id).cloned() else { return div().into_any_element() };
        if provision.requests.is_empty() {
            return div().into_any_element();
        }
        let project = provision.project.as_ref().filter(|p| p.dir == provision.project_dir);
        let v2 = project.and_then(|p| p.manifest.as_ref()).is_some_and(|m| m.is_v2());
        let has_ansible = project.and_then(|p| p.tools.as_ref()).is_some_and(|t| t.ansible_playbook.is_some());
        let busy = state.provision_output.as_ref().is_some_and(|out| out.running);
        let can_run = provision.permissions.run && v2 && has_ansible && !busy;
        let (shown, hidden) = visible_requests(&provision.requests, self.show_all);
        let rows: Vec<AnyElement> = shown.iter().map(|request| self.row(request, &provision, &server, can_run, window, cx)).collect();
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let more = (hidden > 0 || self.show_all).then(|| {
            let label = if self.show_all { tr(cx, "common:provisionRequests.showFewer") } else { cac_core::text_count(locale(cx), "common:provisionRequests.showOlder", hidden as i64) };
            Button::new("requests-more").ghost().xsmall().label(label).on_click(cx.listener(|this, _, _, cx| {
                this.show_all = !this.show_all;
                cx.notify();
            }))
        });
        let line = self.line.clone().map(|(phrase, failed)| div().id("requests-line").aria_label(phrase.clone()).text_xs().text_color(if failed { danger } else { muted }).child(phrase).test_support());
        let body = div().id("provision-requests").flex().flex_col().gap_2().children(line).children(rows).children(more).test_support();
        card("provision-requests-card", tr(cx, "common:provisionRequests.title"), body.into_any_element(), cx)
    }
}
