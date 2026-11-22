//! La lista de servidores (la tabla de `app/src/pages/Dashboard.tsx`): darlos
//! de alta, editarlos y borrarlos, según el papel de quien mira.

use cac_core::{server_edit_phrase, AgentInstallPhase, PlatformState, Server};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::agent_install::{install_for, installer, outcome_phrase};
use super::server_dialog::{ServerDialog, ServerDialogClosed};
use super::services::ServicesView;
use super::k8s_hub::K8sConsole;
use super::stats::StatsPanel;
use super::ssh_key::{SshKeyClosed, SshKeyDialog};
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Las columnas, con su parte del ancho: el nombre y la dirección son lo
/// que se lee; tipo, estado y acciones caben en poco.
const COLUMNS: [(&str, f32); 5] =
    [("common:admin.thName", 3.), ("common:admin.thHost", 3.), ("common:admin.thType", 2.), ("common:admin.thStatus", 2.), ("common:admin.thActions", 1.5)];

pub struct ServersView {
    dialog: Option<(Entity<ServerDialog>, Subscription)>,
    ssh_key: Option<(Entity<SshKeyDialog>, Subscription)>,
    /// Por qué no se borró el último.
    problem: Option<String>,
    /// Cómo acabó la última instalación del agente: de qué servidor, la
    /// frase y si es un aviso.
    agent_line: Option<(String, String, bool)>,
    /// La pestaña de servicios, con su filtro: se crea al abrirla la primera
    /// vez, que es cuando hay ventana.
    services: Option<Entity<ServicesView>>,
    stats: Option<Entity<StatsPanel>>,
    k8s: Option<Entity<K8sConsole>>,
    /// De qué servidor son las tres de arriba. Otro servidor empieza con
    /// las suyas: el filtro, la línea de reinicio o un secreto revelado de
    /// uno salían en el siguiente.
    views_of: Option<String>,
}

impl ServersView {
    pub fn new() -> Self {
        ServersView { dialog: None, ssh_key: None, problem: None, agent_line: None, services: None, stats: None, k8s: None, views_of: None }
    }

    fn open_dialog(&mut self, editing: Option<Server>, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| ServerDialog::new(editing, window, cx));
        let closed = cx.subscribe(&dialog, |this, _, _: &ServerDialogClosed, cx| {
            this.dialog = None;
            cx.notify();
        });
        self.dialog = Some((dialog, closed));
        self.problem = None;
        cx.notify();
    }

    fn open_ssh_key(&mut self, server: &Server, cx: &mut Context<Self>) {
        let (id, name) = (server.id.clone(), server.name.clone());
        let dialog = cx.new(|cx| SshKeyDialog::new(id, name, cx));
        let closed = cx.subscribe(&dialog, |this, _, _: &SshKeyClosed, cx| {
            this.ssh_key = None;
            cx.notify();
        });
        self.ssh_key = Some((dialog, closed));
        cx.notify();
    }

    /// Pregunta nombrando el servidor, y dice que la máquina se queda.
    fn ask_delete(&mut self, server: Server, window: &mut Window, cx: &mut Context<Self>) {
        self.problem = None;
        cx.notify();
        let title = cac_core::text_with(locale(cx), "common:servers.deleteTitle", &[("name", &server.name)]);
        let (body, confirm, cancel) = (tr(cx, "common:admin.deleteServerBody"), tr(cx, "common:admin.delete"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let id = server.id;
            let Ok(deleted) = cx.update(|_, cx| Backend::ask(cx, move |core| async move { core.platform().delete_server(&id).await })) else { return };
            let Ok(Err(error)) = deleted.await else { return };
            this.update(cx, |this, cx| {
                this.problem = Some(server_edit_phrase(locale(cx), &error, "common:admin.errDelete"));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Pregunta antes: la identidad nueva tumba la del agente que hay.
    fn ask_install(&mut self, server: Server, window: &mut Window, cx: &mut Context<Self>) {
        self.agent_line = None;
        cx.notify();
        let language = locale(cx);
        let title = cac_core::text_with(language, "common:servers.agentUpdate.confirmTitle", &[("name", &server.name)]);
        let (body, confirm, cancel) = (tr(cx, "common:servers.agentUpdate.confirmBody"), tr(cx, "common:servers.agentUpdate.confirm"), tr(cx, "common:servers.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        let (installer, keys) = (installer(cx), super::ssh_key::local(cx));
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let (id, name) = (server.id.clone(), server.name.clone());
            let Ok(done) = cx.update(|_, cx| {
                Backend::ask(cx, move |core| async move {
                    // La llave se lee antes de acuñar: reacuñar tumba la
                    // identidad del agente, y con el llavero bloqueado el SSH
                    // ofrecía todas las llaves y acababa en «Too many
                    // authentication failures», con el agente ya sin hablar.
                    let saved = {
                        let id = id.clone();
                        tokio::task::spawn_blocking(move || keys.saved(&id)).await.map_err(|error| error.to_string()).and_then(|saved| saved)
                    };
                    let identity_key = match saved {
                        Ok(key) => key.filter(|key| !key.trim().is_empty()),
                        Err(error) => return Err(error),
                    };
                    let backend_url = core.base_url().to_string();
                    let wait = installer.wait();
                    Ok(core.platform().install_agent(&id, wait, move |server, identity| installer.install(&install_for(server, identity, identity_key, &backend_url))).await)
                })
            }) else {
                return;
            };
            let Ok(outcome) = done.await else { return };
            this.update(cx, |this, cx| {
                let (phrase, warning) = match &outcome {
                    Ok(outcome) => outcome_phrase(locale(cx), &name, outcome),
                    Err(error) => (cac_core::text_with(locale(cx), "common:servers.agentUpdate.keychainFailed", &[("name", &name), ("error", error)]), true),
                };
                this.agent_line = Some((server.id, phrase, warning));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// El botón de la tarjeta del agente y cómo acabó la última vez.
    fn agent_actions(&self, server: &Server, state: &PlatformState, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !state.permissions.install_agent {
            return None;
        }
        let job = state.agent_job.as_ref();
        let label = match job.filter(|job| job.server_id == server.id).map(|job| job.phase) {
            Some(AgentInstallPhase::Waiting) => tr(cx, "common:servers.agentUpdate.waiting"),
            Some(AgentInstallPhase::Installing) => tr(cx, "common:admin.updating"),
            None if server.has_agent_token => tr(cx, "common:admin.updateAgent"),
            None => tr(cx, "common:admin.deployAgent"),
        };
        let theme = cx.theme();
        let (muted, warning_color) = (theme.muted_foreground, theme.warning);
        let line = self.agent_line.as_ref().filter(|(id, _, _)| *id == server.id).map(|(_, phrase, warning)| {
            div().id("server-agent-result").aria_label(phrase.clone()).text_color(if *warning { warning_color } else { muted }).child(phrase.clone()).test_support()
        });
        let target = server.clone();
        Some(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_1()
                .children(line)
                .child(
                    Button::new("server-agent-install")
                        .outline()
                        .small()
                        .icon(Icon::new(Lucide::RefreshCw))
                        .label(label)
                        .disabled(job.is_some())
                        .on_click(cx.listener(move |this, _, window, cx| this.ask_install(target.clone(), window, cx))),
                )
                .into_any_element(),
        )
    }

    fn content(&self, state: &PlatformState, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        if !state.loaded {
            let phrase = if state.loading { tr(cx, "common:admin.loading") } else { String::new() };
            return div().id("platform-loading").leading_sm().text_color(muted).child(phrase).test_support().into_any_element();
        }
        if state.servers.is_empty() {
            return div().id("platform-empty").py_8().text_center().leading_sm().text_color(muted).child(tr(cx, "common:admin.noServers")).test_support().into_any_element();
        }
        let border = cx.theme().border;
        let heading = cells(COLUMNS.iter().map(|(key, _)| div().leading_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, key)).into_any_element()).collect()).h(px(36.));
        let rows: Vec<AnyElement> = state.servers.iter().map(|server| self.server_row(server, state, cx)).collect();
        div().id("platform-table").flex().flex_col().rounded(px(8.)).border_1().border_color(border).child(heading).children(rows).test_support().into_any_element()
    }

    fn server_row(&self, server: &Server, state: &PlatformState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, accent, mono) = (theme.border, theme.muted_foreground, theme.accent, theme.mono_font_family.clone());
        let open = state.open_server.as_deref() == Some(&server.id);
        // Un clúster no lleva el agente de swarm cuyo ciclo sigue `status`.
        let status = if server.kind == cac_core::ServerKind::Kubernetes {
            div().child(super::k8s_hub::cluster_badge(&server.id, state, cx))
        } else {
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(div().size_2().flex_none().rounded_full().bg(super::health_color(server.status, cx)))
                .child(tr(cx, server.status.label_key()))
        };
        let (editing, deleting, keyed) = (server.clone(), server.clone(), server.clone());
        // La llave vive en el llavero de esta máquina, no en cac: cualquiera
        // puede elegir la suya.
        let actions = div()
            .flex()
            .justify_end()
            .gap_0p5()
            .child(
                Button::new(SharedString::from(format!("platform-ssh-{}", server.id)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Lucide::KeyRound))
                    .tooltip(tr(cx, "common:admin.sshKey"))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_ssh_key(&keyed, cx))),
            )
            .when(state.permissions.edit, |row| {
                row.child(
                    Button::new(SharedString::from(format!("platform-edit-{}", server.id)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::Pencil))
                        .tooltip(tr(cx, "common:admin.editServer"))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_dialog(Some(editing.clone()), window, cx))),
                )
            })
            .when(state.permissions.delete, |row| {
                row.child(
                    Button::new(SharedString::from(format!("platform-delete-{}", server.id)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::Trash))
                        .tooltip(tr(cx, "common:admin.deleteServer"))
                        .on_click(cx.listener(move |this, _, window, cx| this.ask_delete(deleting.clone(), window, cx))),
                )
            });
        cells(vec![
            {
                let id = server.id.clone();
                div()
                    .id(SharedString::from(format!("platform-open-{}", server.id)))
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .on_click(move |_, _, cx| {
                        let id = id.clone();
                        Backend::run(cx, move |core| async move { core.platform().open_server_and_load(&id).await });
                    })
                    .child(server.name.clone())
                    .test_support()
                    .into_any_element()
            },
            div().font_family(mono).text_color(muted).truncate().child(server.address()).into_any_element(),
            div().text_color(muted).child(server.kind.as_str()).into_any_element(),
            status.into_any_element(),
            actions.into_any_element(),
        ])
        .id(SharedString::from(format!("platform-row-{}", server.id)))
        .h(px(44.))
        .border_t_1()
        .border_color(border)
        .leading_sm()
        .when(open, |this| this.bg(accent))
        .test_support()
        .into_any_element()
    }
}

impl Render for ServersView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).platform.clone();
        let open = state.open_server.as_ref().and_then(|id| state.servers.iter().find(|server| &server.id == id));
        if let Some(server) = open {
            if self.views_of.as_ref() != Some(&server.id) {
                self.views_of = Some(server.id.clone());
                (self.services, self.stats, self.k8s) = (None, None, None);
            }
            let actions = self.agent_actions(server, &state, cx);
            let services = (state.tab == cac_core::ServerTab::Services)
                .then(|| self.services.get_or_insert_with(|| cx.new(|cx| ServicesView::new(window, cx))).clone().into_any_element());
            let stats = (state.tab == cac_core::ServerTab::Stats).then(|| self.stats.get_or_insert_with(|| cx.new(|cx| StatsPanel::new(window, cx))).clone().into_any_element());
            let k8s = (server.kind == cac_core::ServerKind::Kubernetes && state.tab == cac_core::ServerTab::Overview).then(|| self.k8s.get_or_insert_with(|| cx.new(|_| K8sConsole::new())).clone().into_any_element());
            return div().size_full().child(super::server_page::page(server, &state, actions, services, stats, k8s, cx)).into_any_element();
        }
        let theme = cx.theme();
        let (background, foreground, border, danger) = (theme.background, theme.foreground, theme.border, theme.danger);

        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .min_h(px(52.))
            .border_b_1()
            .border_color(border)
            .child(div().id("platform-title").flex_1().leading_lg().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:admin.servers")).test_support())
            .when(state.permissions.create, |header| {
                header.child(
                    Button::new("platform-add")
                        .primary()
                        .small()
                        .icon(Icon::new(Lucide::Plus))
                        .label(tr(cx, "common:servers.addServer"))
                        .on_click(cx.listener(|this, _, window, cx| this.open_dialog(None, window, cx))),
                )
            });

        let problem = state.error.clone().or_else(|| self.problem.clone());
        let body = div()
            .id("platform-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .children(problem.map(|problem| {
                div()
                    .id("platform-error")
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .leading_xs()
                    .text_color(danger)
                    .child(Icon::new(Lucide::CircleAlert).size_3())
                    .child(problem)
                    .test_support()
            }))
            .child(self.content(&state, cx));

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .line_height(relative(1.5))
            .child(header)
            .child(body)
            .children(self.dialog.as_ref().map(|(dialog, _)| dialog.clone()))
            .children(self.ssh_key.as_ref().map(|(dialog, _)| dialog.clone()))
            .into_any_element()
    }
}

fn cells(cells: Vec<AnyElement>) -> Div {
    div().flex().items_center().px_3().gap_3().children(
        cells.into_iter().zip(COLUMNS).map(|(cell, (_, share))| div().flex_grow(share).flex_basis(px(0.)).min_w_0().child(cell)),
    )
}
