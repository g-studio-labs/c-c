//! El diálogo de alta y de edición de un servidor (`AddServerDialog.tsx` y
//! `EditServerDialog.tsx`). Qué vale y quién puede qué lo decide el core
//! (`cac_core::platform::edit`); aquí está lo escrito.

use cac_core::{server_edit_phrase, Server, ServerForm, ServerKind};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// El diálogo se cerró: quien lo tiene lo quita.
pub struct ServerDialogClosed;

pub struct ServerDialog {
    /// El que se edita; sin él, es un alta.
    editing: Option<Server>,
    name: Entity<InputState>,
    host: Entity<InputState>,
    ssh_user: Entity<InputState>,
    ssh_port: Entity<InputState>,
    agent_port: Entity<InputState>,
    kind: ServerKind,
    busy: bool,
    problem: Option<String>,
}

impl EventEmitter<ServerDialogClosed> for ServerDialog {}

impl ServerDialog {
    pub fn new(editing: Option<Server>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let form = editing.as_ref().map(ServerForm::from).unwrap_or_default();
        let mut field = |value: &str, placeholder: &str| {
            let (value, placeholder) = (value.to_string(), placeholder.to_string());
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
        };
        let name = field(&form.name, "prod-01");
        let host = field(&form.host, "192.168.1.10");
        let ssh_user = field(&form.ssh_user, "root");
        let ssh_port = field(&form.ssh_port, "");
        let agent_port = field(&form.agent_port, "");
        name.update(cx, |name, cx| name.focus(window, cx));
        ServerDialog { editing, name, host, ssh_user, ssh_port, agent_port, kind: form.kind, busy: false, problem: None }
    }

    fn form(&self, cx: &App) -> ServerForm {
        let value = |input: &Entity<InputState>| input.read(cx).value().to_string();
        ServerForm {
            name: value(&self.name),
            host: value(&self.host),
            ssh_user: value(&self.ssh_user),
            ssh_port: value(&self.ssh_port),
            agent_port: value(&self.agent_port),
            kind: self.kind,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let form = self.form(cx);
        let editing = self.editing.as_ref().map(|server| server.id.clone());
        self.busy = true;
        self.problem = None;
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move {
            match editing {
                Some(id) => core.platform().update_server(&id, &form).await,
                None => core.platform().create_server(&form).await,
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => cx.emit(ServerDialogClosed),
                    Err(error) => this.problem = Some(server_edit_phrase(locale(cx), &error, "common:misc.errSave")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(ServerDialogClosed);
    }

    fn field(id: &'static str, label_key: &str, input: &Entity<InputState>, busy: bool, cx: &App) -> Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, label_key)))
            .child(div().id(id).child(Input::new(input).disabled(busy)).test_support())
    }

    /// Las dos clases de servidor. Kubernetes sólo se ofrece a quien puede
    /// crearlo, o si ya lo es.
    fn kinds(&self, cx: &mut Context<Self>) -> Div {
        let kubernetes = Backend::snapshot(cx).read(cx).platform.permissions.kubernetes || self.editing.as_ref().is_some_and(|server| server.kind == ServerKind::Kubernetes);
        let choice = |id: &'static str, kind: ServerKind, label: &'static str, cx: &mut Context<Self>| {
            let chosen = self.kind == kind;
            Button::new(id)
                .small()
                .when(chosen, |button| button.primary())
                .when(!chosen, |button| button.outline())
                .label(label)
                .disabled(self.busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.kind = kind;
                    cx.notify();
                }))
        };
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:servers.type")))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(choice("server-kind-swarm", ServerKind::DockerSwarm, "Docker Swarm", cx))
                    .when(kubernetes, |row| row.child(choice("server-kind-kubernetes", ServerKind::Kubernetes, "Kubernetes", cx))),
            )
    }
}

impl Render for ServerDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.danger);
        let title = match &self.editing {
            Some(server) => cac_core::text_with(locale(cx), "common:servers.editTitle", &[("name", &server.name)]),
            None => tr(cx, "common:servers.addServer"),
        };
        let lead = tr(cx, "common:servers.connectionDetails");
        let busy = self.busy;
        let (save, saving) = if self.editing.is_some() { ("common:servers.save", "common:servers.saving") } else { ("common:servers.addServer", "common:servers.adding") };

        let card = div()
            .id("server-dialog")
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
                    .child(div().id("server-dialog-title").aria_label(title.clone()).text_size(rems(1.)).font_weight(FontWeight::SEMIBOLD).child(title).test_support())
                    .child(div().leading_xs().text_color(muted).child(lead)),
            )
            .child(div().flex().gap_3().child(Self::field("server-name", "common:servers.thName", &self.name, busy, cx)).child(Self::field("server-host", "common:servers.host", &self.host, busy, cx)))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(Self::field("server-ssh-user", "common:servers.sshUser", &self.ssh_user, busy, cx))
                    .child(Self::field("server-ssh-port", "common:servers.sshPort", &self.ssh_port, busy, cx))
                    .child(Self::field("server-agent-port", "common:servers.agentPort", &self.agent_port, busy, cx)),
            )
            .child(self.kinds(cx))
            .children(self.problem.clone().map(|problem| {
                div()
                    .id("server-dialog-problem")
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .leading_xs()
                    .text_color(danger)
                    .child(Icon::new(Lucide::CircleAlert).size_3())
                    .child(problem)
                    .test_support()
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("server-dialog-cancel").outline().small().label(tr(cx, "common:servers.cancel")).on_click(cx.listener(|this, _, _, cx| this.close(cx))))
                    .child(
                        Button::new("server-dialog-save")
                            .primary()
                            .small()
                            .label(tr(cx, if busy { saving } else { save }))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .test_support();

        super::veil("server-dialog-veil", card, window)
    }
}
