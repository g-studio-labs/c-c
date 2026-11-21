//! Qué llave del agente SSH se le ofrece a un servidor (`SshKeyDialog.tsx`).
//!
//! Un agente con muchas llaves las prueba de una en una, y el servidor corta
//! en `MaxAuthTries` (6) con «Too many authentication failures», casi siempre
//! antes de llegar a la buena. Nombrar la llave fija el intento a una sola.
//! cac sólo guarda la **pública**, en el llavero del sistema; la privada no
//! sale del agente.

use std::sync::Arc;

use cac_ops::ssh::{SshAgent, SshKeyItem};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Lo de la máquina local que hace falta aquí. Las pruebas ponen uno de
/// mentira: el de verdad lanza `ssh-add` y escribe en el llavero.
pub trait SshKeys: Send + Sync {
    fn agents(&self) -> Vec<SshAgent>;
    fn keys(&self, socket: Option<String>) -> Result<Vec<SshKeyItem>, String>;
    fn saved(&self, server_id: &str) -> Result<Option<String>, String>;
    /// En blanco, la quita.
    fn save(&self, server_id: &str, public_key: &str) -> Result<(), String>;
}

struct SystemKeys;

impl SshKeys for SystemKeys {
    fn agents(&self) -> Vec<SshAgent> {
        cac_ops::ssh::list_ssh_agents()
    }
    fn keys(&self, socket: Option<String>) -> Result<Vec<SshKeyItem>, String> {
        cac_ops::ssh::list_agent_ssh_keys(socket)
    }
    fn saved(&self, server_id: &str) -> Result<Option<String>, String> {
        cac_ops::keychain::ssh_key(server_id)
    }
    fn save(&self, server_id: &str, public_key: &str) -> Result<(), String> {
        cac_ops::keychain::set_ssh_key(server_id, public_key)
    }
}

/// Las llaves de esta app: las del sistema, salvo que alguien ponga otras.
pub struct LocalKeys(pub Arc<dyn SshKeys>);

impl Global for LocalKeys {}

pub(crate) fn local(cx: &App) -> Arc<dyn SshKeys> {
    cx.try_global::<LocalKeys>().map(|keys| keys.0.clone()).unwrap_or_else(|| Arc::new(SystemKeys))
}

/// El diálogo se cerró: quien lo tiene lo quita.
pub struct SshKeyClosed;

pub struct SshKeyDialog {
    server_id: String,
    server_name: String,
    agents: Vec<SshAgent>,
    agent: Option<String>,
    keys: Vec<SshKeyItem>,
    /// La guardada, y la elegida. `None` es «que decida el agente».
    current: Option<String>,
    selected: Option<String>,
    loading: bool,
    saving: bool,
    error: Option<String>,
}

impl EventEmitter<SshKeyClosed> for SshKeyDialog {}

impl SshKeyDialog {
    pub fn new(server_id: String, server_name: String, cx: &mut Context<Self>) -> Self {
        let mut dialog = SshKeyDialog {
            server_id,
            server_name,
            agents: Vec::new(),
            agent: None,
            keys: Vec::new(),
            current: None,
            selected: None,
            loading: true,
            saving: false,
            error: None,
        };
        dialog.load(None, cx);
        dialog
    }

    /// Qué agentes hay es otra pregunta que qué llaves tiene uno: un portátil
    /// puede tener el de 1Password y el del sistema a la vez, con llaves
    /// distintas. Sin elegir, el primero que contesta con llaves.
    fn load(&mut self, socket: Option<String>, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        cx.notify();
        let (keys, server_id) = (local(cx), self.server_id.clone());
        let read = cx.background_spawn(async move {
            let agents = keys.agents();
            let pick = socket
                .or_else(|| agents.iter().find(|agent| agent.status == "ok").map(|agent| agent.socket.clone()))
                .or_else(|| agents.first().map(|agent| agent.socket.clone()));
            let found = keys.keys(pick.clone()).and_then(|items| keys.saved(&server_id).map(|saved| (items, saved)));
            (agents, pick, found)
        });
        cx.spawn(async move |this, cx| {
            let (agents, pick, found) = read.await;
            this.update(cx, |this, cx| {
                this.loading = false;
                this.agents = agents;
                this.agent = pick;
                match found {
                    Ok((items, saved)) => {
                        this.keys = items;
                        this.current = saved.clone();
                        this.selected = saved;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.error = None;
        cx.notify();
        let (keys, server_id, chosen) = (local(cx), self.server_id.clone(), self.selected.clone().unwrap_or_default());
        let saved = cx.background_spawn(async move { keys.save(&server_id, &chosen) });
        cx.spawn(async move |this, cx| {
            let result = saved.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(()) => cx.emit(SshKeyClosed),
                    Err(error) => this.error = Some(format!("{}: {error}", tr(cx, "common:misc.errSave"))),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn choice(&self, id: SharedString, value: Option<String>, title: String, detail: String, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, accent, primary, muted) = (theme.border, theme.accent, theme.primary, theme.muted_foreground);
        let chosen = self.selected == value;
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_2()
            .p_2()
            .rounded(px(6.))
            .border_1()
            .border_color(if chosen { primary } else { border })
            .when(chosen, |row| row.bg(accent))
            .cursor_pointer()
            .hover(move |style| style.bg(accent))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = value.clone();
                cx.notify();
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().child(title))
                    .child(div().truncate().leading_xs().text_color(muted).child(detail)),
            )
            .when(chosen, |row| row.child(Icon::new(Lucide::Check).size_4()))
            .test_support()
            .into_any_element()
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        if let Some(error) = &self.error {
            // El mensaje nombra los sockets que se probaron: va con sus saltos.
            return div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().id("ssh-key-error").whitespace_normal().leading_xs().text_color(danger).child(error.clone()).test_support())
                .child(
                    Button::new("ssh-key-retry")
                        .outline()
                        .small()
                        .icon(Icon::new(Lucide::RefreshCw))
                        .label(tr(cx, "common:servers.retry"))
                        .on_click(cx.listener(|this, _, _, cx| this.load(this.agent.clone(), cx))),
                )
                .into_any_element();
        }
        if self.loading {
            return div().id("ssh-key-loading").py_6().text_center().leading_sm().text_color(muted).child(tr(cx, "common:servers.readingAgent")).test_support().into_any_element();
        }
        let lead = self.choice("ssh-key-agent-decides".into(), None, tr(cx, "common:misc.letAgentDecide"), tr(cx, "common:misc.agentDecideHelp"), cx);
        let keys: Vec<_> = self
            .keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let id = SharedString::from(format!("ssh-key-{index}"));
                self.choice(id, Some(key.public_key.clone()), key.title.clone(), format!("{} · {}", key.key_type, key.fingerprint), cx)
            })
            .collect();
        let empty = self.keys.is_empty().then(|| div().id("ssh-key-empty").py_4().text_center().leading_xs().text_color(muted).child(tr(cx, "common:servers.agentHoldsNoKeys")).test_support());
        div().id("ssh-key-list").max_h(px(320.)).overflow_y_scroll().flex().flex_col().gap_1().child(lead).children(keys).children(empty).into_any_element()
    }

    fn agents_row(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.agents.len() < 2 {
            return None;
        }
        let chips: Vec<_> = self
            .agents
            .iter()
            .enumerate()
            .map(|(index, agent)| {
                let socket = agent.socket.clone();
                let detail = if agent.status == "refused" {
                    tr(cx, "common:servers.agentNotRunning")
                } else {
                    cac_core::text_count(locale(cx), "common:servers.agentKeys", agent.key_count as i64)
                };
                let chosen = self.agent.as_deref() == Some(agent.socket.as_str());
                Button::new(SharedString::from(format!("ssh-agent-{index}")))
                    .xsmall()
                    .when(chosen, |button| button.primary())
                    .when(!chosen, |button| button.outline())
                    .label(format!("{} · {detail}", agent.label))
                    .on_click(cx.listener(move |this, _, _, cx| this.load(Some(socket.clone()), cx)))
            })
            .collect();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().leading_xs().text_color(cx.theme().muted_foreground).child(tr(cx, "common:misc.agent")))
                .child(div().flex().flex_wrap().gap_1().children(chips)),
        )
    }
}

impl Render for SshKeyDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted) = (theme.background, theme.foreground, theme.muted_foreground);
        let title = cac_core::text_with(locale(cx), "common:servers.sshKeyTitle", &[("name", &self.server_name)]);
        let unchanged = self.selected == self.current;
        let card = div()
            .id("ssh-key-dialog")
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(SshKeyClosed);
                    cx.stop_propagation();
                }
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
                    .child(
                        div()
                            .id("ssh-key-title")
                            .aria_label(title.clone())
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(rems(1.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(Icon::new(Lucide::KeyRound).size_4())
                            .child(title)
                            .test_support(),
                    )
                    .child(div().leading_xs().text_color(muted).child(tr(cx, "common:misc.sshLead"))),
            )
            .children(self.agents_row(cx))
            .child(self.body(cx))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("ssh-key-cancel").outline().small().label(tr(cx, "common:servers.cancel")).on_click(cx.listener(|_, _, _, cx| cx.emit(SshKeyClosed))))
                    .child(
                        Button::new("ssh-key-save")
                            .primary()
                            .small()
                            .label(tr(cx, if self.saving { "common:servers.saving" } else { "common:servers.save" }))
                            .disabled(self.saving || self.loading || unchanged)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
            .test_support();

        super::veil("ssh-key-veil", card, window)
    }
}
