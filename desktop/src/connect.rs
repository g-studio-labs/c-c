//! «Conectar Claude Code»: instalar `cac-mcp` y mantenerlo al día.
//!
//! El agente habla con cac a través de `cac-mcp`, un binario aparte. Esta
//! ventana dice si está en la máquina, qué versión, si hay una más nueva, y
//! lo instala o lo actualiza con un botón; debajo va la orden que lo registra
//! en Claude Code. Hace lo mismo que `scripts/install-mcp.sh` y que `cac-mcp
//! update`, con la misma lógica: qué hay instalado, qué se ofrece, de dónde se
//! baja y cómo se comprueba lo decide `cac_mcp::update`. Aquí se pinta lo que
//! diga, y nada más.
//!
//! Debajo, el token personal: se crea aquí, con «Seleccionar todos» en los
//! permisos, y «Registrar en Claude Code» lo crea y registra `cac-mcp` con
//! `claude mcp add` (`cac_mcp::claude_code`) **sin enseñarlo**: no se pinta,
//! no va al portapapeles y no sale en ningún log. Sólo el paso a mano («Crear
//! el token y enseñarlo») lo enseña, una vez, con su «Copiar»; al cerrar la
//! ventana se olvida. Antes de quitar un registro `cac` que ya hubiera, se
//! pregunta.
//!
//! El aspecto es el de `ConnectMcpDialog.tsx`: el título y la entradilla, los
//! bloques con borde, y la orden en un campo que se copia.

use std::time::Duration;

use std::path::PathBuf;
use std::sync::Arc;

use cac_core::{all_scopes_selected, toggle_all_scopes, toggle_scope, TokenValue, TOKEN_SCOPES};
use cac_mcp::claude_code::{CardState, Cli, RegisterError, Registration};
use cac_mcp::update::{Installed, Machine, Progress, Status, Step, TOKEN_PLACEHOLDER};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{error_text, locale, tr};
use crate::theme::Leading as _;

/// El ancho de la caja: el `max-w-2xl` del diálogo de la app de siempre.
const WIDTH: f32 = 672.;
/// Cada cuánto se vuelve a mirar con la app abierta, y lo viejo que puede ser
/// lo sabido para no preguntar otra vez.
const LOOK_EVERY: Duration = Duration::from_secs(3 * 60 * 60);

/// La máquina en la que corre la app, para instalar `cac-mcp`, y el servidor
/// de cac con el que habla (va en la orden de registro). Lo pone `main`; sin
/// ello (las pruebas de otras pantallas) esta vista no mira ni baja nada.
pub struct McpSetup {
    pub machine: Machine,
    pub server_url: String,
    /// El `claude` con el que se registra; en las pruebas, uno de mentira.
    pub claude: Arc<dyn Cli>,
}

impl Global for McpSetup {}

/// Lo que se le pidió y cómo va.
#[derive(Clone, Debug, PartialEq)]
pub enum Work {
    Idle,
    /// Instalando; `None` hasta que llega el primer paso.
    Running(Option<Progress>),
    Done(Installed),
    Failed(String),
}

/// Lo del token y el registro en Claude Code.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenWork {
    Idle,
    /// Creando el token (y registrando, si `registering`).
    Busy { registering: bool },
    Registered,
    /// La clave del catálogo de qué falló, y el motivo (sin token).
    Failed { key: &'static str, reason: String },
}

/// Se pidió cerrar: quien la tiene la quita.
pub struct ConnectClosed;

pub struct ConnectView {
    /// Lo que se sabe de la máquina. `None` mientras se mira por primera vez.
    pub(crate) status: Option<Status>,
    pub(crate) work: Work,
    copied: bool,
    focus: FocusHandle,
    /// Cómo está `cac` en Claude Code. `None` hasta que se mira.
    pub(crate) registration: Option<Registration>,
    pub(crate) scopes: Vec<String>,
    name: Option<Entity<InputState>>,
    pub(crate) token_work: TokenWork,
    /// Hay un registro `cac` (que lanza esto) y se espera el sí para quitarlo.
    pub(crate) confirm_replace: Option<String>,
    /// El token creado para registrar que aún no se registró (se pidió el sí,
    /// o falló): reintentar no crea otro. Nunca se pinta.
    pending_token: Option<TokenValue>,
    /// El del paso a mano: se enseña mientras la ventana sigue abierta.
    pub(crate) shown_token: Option<TokenValue>,
    token_copied: bool,
}

impl EventEmitter<ConnectClosed> for ConnectView {}

impl Focusable for ConnectView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Lo que llega del hilo que instala.
enum Report {
    Step(Progress),
    End(Result<Installed, String>),
}

impl ConnectView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Con la app abierta se mira al entrar y cada pocas horas: es lo que
        // enciende el punto del menú sin que nadie abra esta ventana.
        if cx.has_global::<McpSetup>() {
            cx.spawn(async move |this, cx| loop {
                if this.update(cx, |this, cx| this.look(LOOK_EVERY.as_secs(), cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(LOOK_EVERY).await;
            })
            .detach();
        }
        Self {
            status: None,
            work: Work::Idle,
            copied: false,
            focus: cx.focus_handle(),
            registration: None,
            scopes: Vec::new(),
            name: None,
            token_work: TokenWork::Idle,
            confirm_replace: None,
            pending_token: None,
            shown_token: None,
            token_copied: false,
        }
    }

    /// Al abrirla: se mira ahora, preguntando por la última versión, y un
    /// resultado de antes (hecho o fallido) deja de enseñarse.
    pub fn opened(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.work, Work::Running(_)) {
            self.work = Work::Idle;
        }
        self.copied = false;
        // El token enseñado se enseña una vez: al volver ya no está.
        self.shown_token = None;
        self.token_copied = false;
        self.confirm_replace = None;
        if !matches!(self.token_work, TokenWork::Busy { .. }) {
            self.token_work = TokenWork::Idle;
        }
        self.look(0, cx);
        self.look_registration(cx);
    }

    /// Lo que se ve de la orden de registro: con el token sólo si es el del
    /// paso a mano; si no, con el marcador.
    pub(crate) fn visible_command(&self, server_url: &str, target: &std::path::Path) -> String {
        cac_mcp::update::register_command_with(server_url, self.shown_token.as_ref().map(TokenValue::expose), target)
    }

    /// El `cac-mcp` instalado, el único que se puede registrar.
    fn installed_binary(&self) -> Option<PathBuf> {
        self.status.as_ref()?.installed.as_ref().map(|installed| installed.path.clone())
    }

    /// Pregunta a `claude` cómo está registrado, en un hilo aparte: lanza un
    /// programa, y `claude mcp get` prueba a conectar.
    fn look_registration(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = cx.try_global::<McpSetup>() else { return };
        let claude = setup.claude.clone();
        let (answer, receiver) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = answer.send(cac_mcp::claude_code::look(&*claude));
        });
        cx.spawn(async move |this, cx| {
            let Ok(registration) = receiver.await else { return };
            this.update(cx, |this, cx| {
                this.registration = Some(registration);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn token_name(&self, cx: &App) -> String {
        self.name.as_ref().map(|name| name.read(cx).value().to_string()).unwrap_or_default()
    }

    /// «Registrar en Claude Code». Con un registro `cac` ya puesto, primero
    /// se pregunta (`replace` es el sí), antes incluso de crear el token.
    pub(crate) fn register(&mut self, replace: bool, cx: &mut Context<Self>) {
        if matches!(self.token_work, TokenWork::Busy { .. }) {
            return;
        }
        let (Some(setup), Some(target)) = (cx.try_global::<McpSetup>(), self.installed_binary()) else { return };
        let (claude, url) = (setup.claude.clone(), setup.server_url.clone());
        if !replace {
            if let Some(Registration::Present { command, .. }) = &self.registration {
                self.confirm_replace = Some(command.clone());
                cx.notify();
                return;
            }
        }
        self.confirm_replace = None;
        self.token_work = TokenWork::Busy { registering: true };
        cx.notify();
        let pending = self.pending_token.clone();
        let (name, scopes) = (self.token_name(cx), self.scopes.clone());
        let created = Backend::ask(cx, move |core| async move {
            match pending {
                Some(token) => Ok(token),
                None => core.create_personal_token(&name, &scopes).await.map(|created| created.value),
            }
        });
        cx.spawn(async move |this, cx| {
            let Ok(created) = created.await else { return };
            let token = match created {
                Ok(token) => token,
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.token_work = TokenWork::Failed { key: "nav:mcp.tokenFailed", reason: error_text(cx, &error, "nav:mcp.tokenFailed") };
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let (answer, receiver) = tokio::sync::oneshot::channel();
            let for_claude = token.clone();
            std::thread::spawn(move || {
                let _ = answer.send(cac_mcp::claude_code::register(&*claude, &url, for_claude.expose(), &target, replace));
            });
            let Ok(result) = receiver.await else { return };
            this.update(cx, |this, cx| {
                this.token_work = TokenWork::Idle;
                match result {
                    Ok(()) => {
                        this.pending_token = None;
                        this.token_work = TokenWork::Registered;
                        this.look_registration(cx);
                    }
                    // Lo pusieron mientras tanto: se pregunta, con el token
                    // ya creado guardado para no crear otro.
                    Err(RegisterError::NeedsConfirmation { command }) => {
                        this.pending_token = Some(token);
                        this.confirm_replace = Some(command);
                    }
                    // Sin `claude`, el paso a mano con el token ya creado.
                    Err(RegisterError::NoClaude) => {
                        this.pending_token = None;
                        this.shown_token = Some(token);
                        this.registration = Some(Registration::NoClaude);
                    }
                    Err(RegisterError::Failed(reason)) => {
                        // `reason` viene sin token (`claude_code::RegisterError`).
                        log::warn!("no se pudo registrar cac-mcp en Claude Code: {reason}");
                        this.pending_token = Some(token);
                        this.token_work = TokenWork::Failed { key: "nav:mcp.registerFailed", reason };
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// El paso a mano: crea el token y lo enseña una vez.
    fn create_and_show(&mut self, cx: &mut Context<Self>) {
        if matches!(self.token_work, TokenWork::Busy { .. }) {
            return;
        }
        self.token_work = TokenWork::Busy { registering: false };
        self.confirm_replace = None;
        cx.notify();
        let pending = self.pending_token.take();
        let (name, scopes) = (self.token_name(cx), self.scopes.clone());
        let created = Backend::ask(cx, move |core| async move {
            match pending {
                Some(token) => Ok(token),
                None => core.create_personal_token(&name, &scopes).await.map(|created| created.value),
            }
        });
        cx.spawn(async move |this, cx| {
            let Ok(created) = created.await else { return };
            this.update(cx, |this, cx| {
                match created {
                    Ok(token) => {
                        this.shown_token = Some(token);
                        this.token_copied = false;
                        this.token_work = TokenWork::Idle;
                    }
                    Err(error) => {
                        this.token_work = TokenWork::Failed { key: "nav:mcp.tokenFailed", reason: error_text(cx, &error, "nav:mcp.tokenFailed") }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// «Seleccionar todos» / «Quitar todos».
    pub(crate) fn toggle_all(&mut self, cx: &mut Context<Self>) {
        self.scopes = toggle_all_scopes(&self.scopes);
        cx.notify();
    }

    fn copy_token(&mut self, cx: &mut Context<Self>) {
        // Sólo al pulsar «Copiar» de la persona: es la única vía al
        // portapapeles que tiene el token.
        let Some(token) = &self.shown_token else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(token.expose().to_string()));
        self.token_copied = true;
        cx.notify();
    }

    /// Si hay una versión más nueva que la instalada. Lo decide el MCP.
    pub fn update_available(&self) -> bool {
        self.status.as_ref().is_some_and(|status| status.step() == Step::Update)
    }

    /// Mira la máquina en un hilo aparte: lanza el binario y puede salir a la
    /// red, y ninguna de las dos cosas se hace en el hilo que pinta.
    fn look(&mut self, max_age: u64, cx: &mut Context<Self>) {
        let Some(setup) = cx.try_global::<McpSetup>() else { return };
        let machine = setup.machine.clone();
        let (answer, receiver) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = answer.send(machine.look(max_age));
        });
        cx.spawn(async move |this, cx| {
            let Ok(status) = receiver.await else { return };
            this.update(cx, |this, cx| {
                this.status = Some(status);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Instala o actualiza: lo que toque según lo que se sabe.
    fn run(&mut self, cx: &mut Context<Self>) {
        let (Some(setup), Some(status)) = (cx.try_global::<McpSetup>(), self.status.clone()) else { return };
        if matches!(self.work, Work::Running(_)) || status.step() == Step::Current {
            return;
        }
        let machine = setup.machine.clone();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let known = status.clone();
        std::thread::spawn(move || {
            let steps = sender.clone();
            let end = machine.install(&known, &mut |step| {
                let _ = steps.send(Report::Step(step));
            });
            let _ = sender.send(Report::End(end));
        });
        self.work = Work::Running(None);
        cx.notify();
        cx.spawn(async move |this, cx| {
            while let Some(report) = receiver.recv().await {
                let updated = this.update(cx, |this, cx| {
                    match report {
                        Report::Step(step) => this.work = Work::Running(Some(step)),
                        Report::End(Ok(installed)) => {
                            // Lo instalado es lo que dijo el binario nuevo; la
                            // última conocida sigue siendo la que era.
                            let latest = this.status.take().and_then(|status| status.latest);
                            this.status = Some(Status { installed: Some(installed.clone()), latest });
                            this.work = Work::Done(installed);
                        }
                        Report::End(Err(reason)) => this.work = Work::Failed(reason),
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn copy_command(&mut self, command: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(command));
        self.copied = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1500)).await;
            this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Qué se dice de lo instalado y de lo publicado: una línea cada cosa.
    fn render_status(&self, status: &Status, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, success, warning, mono) = (theme.muted_foreground, theme.success, theme.warning, theme.mono_font_family.clone());
        let here = locale(cx);
        let installed = match &status.installed {
            None => div().id("connect-missing").test_support().child(tr(cx, "nav:mcp.missing")).into_any_element(),
            Some(installed) => div()
                .id("connect-installed")
                .test_support()
                .flex()
                .flex_col()
                .child(cac_core::text_with(here, "nav:mcp.installed", &[("version", &installed.version)]))
                .child(div().w_full().text_size(px(11.)).font_family(mono).text_color(muted).child(installed.path.display().to_string()))
                .into_any_element(),
        };
        let published = match (status.step(), &status.latest) {
            (Step::Update, Some(latest)) => Some(
                div()
                    .id("connect-newer")
                    .test_support()
                    .text_color(warning)
                    .child(cac_core::text_with(here, "nav:mcp.newer", &[("version", latest)]))
                    .into_any_element(),
            ),
            (Step::Current, Some(_)) => Some(
                div()
                    .id("connect-current")
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_color(muted)
                    .child(Icon::new(Lucide::CircleCheck).size_3p5().flex_none().text_color(success))
                    .child(tr(cx, "nav:mcp.current"))
                    .into_any_element(),
            ),
            // Instalado y sin saber la última: se dice, que callar parecería
            // «está al día».
            (Step::Current, None) => {
                Some(div().id("connect-unknown").test_support().text_color(muted).child(tr(cx, "nav:mcp.unknownLatest")).into_any_element())
            }
            _ => None,
        };
        div().flex_1().min_w_0().flex().flex_col().gap_0p5().leading_xs().child(installed).children(published)
    }

    /// El botón, o por dónde va lo que se pidió.
    fn render_action(&self, status: &Status, cx: &mut Context<Self>) -> Option<AnyElement> {
        let here = locale(cx);
        let muted = cx.theme().muted_foreground;
        if let Work::Running(step) = &self.work {
            let label = match step {
                Some(Progress::Downloading { received, total: Some(total) }) if *total > 0 => {
                    cac_core::text_with(here, "nav:mcp.downloadingPercent", &[("percent", &(received * 100 / total).min(100).to_string())])
                }
                None | Some(Progress::Downloading { .. }) => tr(cx, "nav:mcp.downloading"),
                Some(Progress::Verifying) => tr(cx, "nav:mcp.verifying"),
                Some(Progress::Installing) => tr(cx, "nav:mcp.installing"),
            };
            return Some(
                div()
                    .id("connect-progress")
                    .test_support()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .leading_xs()
                    .text_color(muted)
                    .child(Spinner::new().color(muted))
                    .child(label)
                    .into_any_element(),
            );
        }
        let label = match (status.step(), &status.latest) {
            (Step::Install, _) => tr(cx, "nav:mcp.install"),
            (Step::Update, Some(latest)) => cac_core::text_with(here, "nav:mcp.update", &[("version", latest)]),
            _ => return None,
        };
        Some(
            Button::new("connect-run")
                .primary()
                .small()
                .icon(Icon::new(Lucide::Download))
                .label(label)
                .on_click(cx.listener(|this, _, _, cx| this.run(cx)))
                .into_any_element(),
        )
    }
}

/// Las claves de detalle de los permisos llevan el marcado de `<Trans>`
/// (`<1>tú</1>`); aquí se pinta el texto llano.
fn plain(text: String) -> String {
    text.replace("<1>", "").replace("</1>", "")
}

impl ConnectView {
    /// El token personal y el registro en Claude Code.
    fn render_token(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, success, danger, warning, field, mono) =
            (theme.muted_foreground, theme.success, theme.danger, theme.warning, theme.muted.opacity(0.5), theme.mono_font_family.clone());
        let here = locale(cx);
        let installed = self.installed_binary();
        let busy = matches!(self.token_work, TokenWork::Busy { .. });

        let state = self.registration.as_ref().map(|registration| {
            let (id, color, text): (&str, Hsla, String) = match registration.card(installed.as_deref()) {
                CardState::ThisBinary => ("connect-claude-this", success, tr(cx, "nav:mcp.stateThis")),
                CardState::Other { command } => {
                    ("connect-claude-other", warning, cac_core::text_with(here, "nav:mcp.stateOther", &[("command", &command)]))
                }
                CardState::Missing => ("connect-claude-missing", muted, tr(cx, "nav:mcp.stateMissing")),
                CardState::NoClaude => ("connect-claude-absent", muted, tr(cx, "nav:mcp.noClaude")),
                CardState::Unknown => ("connect-claude-unknown", muted, tr(cx, "nav:mcp.stateUnknown")),
            };
            div().id(id).test_support().w_full().leading_xs().text_color(color).child(text)
        });

        let all = all_scopes_selected(&self.scopes);
        let scopes = TOKEN_SCOPES.iter().map(|scope| {
            let id = scope.id;
            // Dos columnas, como `ScopeChecklist` en la otra app.
            div()
                .w(relative(0.5))
                .pr_3()
                .pb_2()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    Checkbox::new(SharedString::from(format!("connect-scope-{id}")))
                        .checked(self.scopes.iter().any(|picked| picked == id))
                        .label(tr(cx, scope.label_key))
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.scopes = toggle_scope(&this.scopes, id, *checked);
                            cx.notify();
                        })),
                )
                .child(div().w_full().pl_6().text_size(px(11.)).leading_xs().text_color(muted).child(plain(tr(cx, scope.detail_key))))
        })
        .collect::<Vec<_>>();

        let confirm = self.confirm_replace.clone().map(|command| {
            div()
                .id("connect-replace-ask")
                .test_support()
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .rounded(px(6.))
                .border_1()
                .border_color(warning)
                .p_2()
                .child(div().w_full().leading_xs().child(cac_core::text_with(here, "nav:mcp.replaceAsk", &[("command", &command)])))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("connect-replace")
                                .primary()
                                .small()
                                .label(tr(cx, "nav:mcp.replace"))
                                .on_click(cx.listener(|this, _, _, cx| this.register(true, cx))),
                        )
                        .child(Button::new("connect-keep").outline().small().label(tr(cx, "nav:mcp.keep")).on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_replace = None;
                            cx.notify();
                        }))),
                )
        });

        let no_claude = matches!(self.registration, Some(Registration::NoClaude));
        let hint = if installed.is_none() {
            tr(cx, "nav:mcp.needsInstall")
        } else if no_claude {
            tr(cx, "nav:mcp.noClaude")
        } else {
            tr(cx, "nav:mcp.registerHint")
        };
        let actions = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("connect-register")
                    .primary()
                    .small()
                    .icon(Icon::new(Lucide::Terminal))
                    .label(tr(cx, "nav:mcp.register"))
                    .disabled(busy || installed.is_none() || no_claude || self.confirm_replace.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.register(false, cx))),
            )
            .child(
                Button::new("connect-show-token")
                    .outline()
                    .small()
                    .icon(Icon::new(Lucide::KeyRound))
                    .label(tr(cx, "nav:mcp.showToken"))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.create_and_show(cx))),
            );

        let outcome = match &self.token_work {
            TokenWork::Busy { registering } => Some(
                div()
                    .id("connect-token-busy")
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_2()
                    .leading_xs()
                    .text_color(muted)
                    .child(Spinner::new().color(muted))
                    .child(tr(cx, if *registering { "nav:mcp.registering" } else { "nav:mcp.creating" }))
                    .into_any_element(),
            ),
            TokenWork::Registered => Some(
                div().id("connect-registered").test_support().w_full().leading_xs().text_color(success).child(tr(cx, "nav:mcp.registered")).into_any_element(),
            ),
            TokenWork::Failed { key, reason } => Some(
                div()
                    .id("connect-token-problem")
                    .test_support()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .leading_xs()
                    .text_color(danger)
                    .child(div().w_full().child(tr(cx, key)))
                    .child(div().w_full().text_size(px(11.)).font_family(mono.clone()).child(reason.clone()))
                    .into_any_element(),
            ),
            TokenWork::Idle => None,
        };

        let token_copied = self.token_copied;
        let shown = self.shown_token.as_ref().map(|token| {
            div()
                .id("connect-token-shown")
                .test_support()
                .w_full()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div().flex().items_center().gap_2().child(div().flex_1().leading_xs().text_color(warning).child(tr(cx, "nav:mcp.tokenOnce"))).child(
                        Button::new("connect-copy-token")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(if token_copied { Lucide::CircleCheck } else { Lucide::Copy }))
                            .tooltip(tr(cx, if token_copied { "nav:mcp.copied" } else { "nav:mcp.copyToken" }))
                            .on_click(cx.listener(|this, _, _, cx| this.copy_token(cx))),
                    ),
                )
                .child(
                    div()
                        .w_full()
                        .overflow_hidden()
                        .rounded(px(4.))
                        .bg(field)
                        .px_2()
                        .py_1p5()
                        .leading_xs()
                        .font_family(mono.clone())
                        .child(token.expose().to_string()),
                )
        });

        let name = self.name.clone();
        div()
            .id("connect-token")
            .test_support()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, "nav:mcp.tokenTitle")))
            .children(state)
            .children(name.map(|name| {
                div().w_full().flex().flex_col().gap_1().child(div().leading_xs().child(tr(cx, "nav:mcp.tokenName"))).child(Input::new(&name).id("connect-token-name"))
            }))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div().flex().items_center().gap_2().child(div().flex_1().leading_xs().child(tr(cx, "nav:mcp.scopes"))).child(
                            Button::new("connect-select-all")
                                .ghost()
                                .xsmall()
                                .label(tr(cx, if all { "nav:mcp.clearAll" } else { "nav:mcp.selectAll" }))
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_all(cx))),
                        ),
                    )
                    .child(div().w_full().flex().flex_wrap().children(scopes))
                    .children(self.scopes.is_empty().then(|| div().w_full().leading_xs().text_color(muted).child(tr(cx, "nav:mcp.readOnly")))),
            )
            .children(confirm)
            .child(actions)
            .child(div().w_full().leading_xs().text_color(muted).child(hint))
            .children(outcome)
            .children(shown)
            .into_any_element()
    }
}

impl Render for ConnectView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.name.is_none() {
            self.name = Some(cx.new(|cx| InputState::new(window, cx).placeholder(cac_core::DEFAULT_TOKEN_NAME)));
        }
        let token_panel = self.render_token(cx);
        let theme = cx.theme();
        let (background, popover, border, foreground, muted, danger, success, field, mono) = (
            theme.background,
            theme.popover,
            theme.border,
            theme.foreground,
            theme.muted_foreground,
            theme.danger,
            theme.success,
            theme.muted.opacity(0.5),
            theme.mono_font_family.clone(),
        );
        let here = locale(cx);
        let setup = cx.try_global::<McpSetup>();
        // La orden apunta a donde está el binario, o a donde va a quedar.
        let command = setup.and_then(|setup| {
            let target = setup.machine.target(self.status.as_ref().unwrap_or(&Status::default()))?;
            Some(self.visible_command(&setup.server_url, &target))
        });

        let machine = match &self.status {
            None => div()
                .id("connect-looking")
                .test_support()
                .flex()
                .items_center()
                .gap_2()
                .leading_xs()
                .text_color(muted)
                .child(Spinner::new().color(muted))
                .child(tr(cx, "nav:mcp.looking"))
                .into_any_element(),
            Some(status) => div()
                .flex()
                .items_center()
                .gap_3()
                .child(self.render_status(status, cx))
                .children(self.render_action(status, cx))
                .into_any_element(),
        };
        let outcome = match &self.work {
            Work::Done(installed) => Some(
                div()
                    .id("connect-done")
                    .test_support()
                    .w_full()
                    .leading_xs()
                    .text_color(success)
                    .child(cac_core::text_with(here, "nav:mcp.done", &[("version", &installed.version)]))
                    .into_any_element(),
            ),
            // Dos líneas: qué pasó, dicho en el idioma de quien lee, y el
            // motivo tal como lo da el instalador (una suma que no coincide,
            // una versión sin publicar, sin red).
            Work::Failed(reason) => Some(
                div()
                    .id("connect-problem")
                    .test_support()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .leading_xs()
                    .text_color(danger)
                    .child(div().w_full().child(tr(cx, "nav:mcp.failed")))
                    .child(div().w_full().text_size(px(11.)).font_family(mono.clone()).child(reason.clone()))
                    .into_any_element(),
            ),
            _ => None,
        };
        let copy_label = tr(cx, if self.copied { "nav:mcp.copied" } else { "nav:mcp.copy" });
        let copied = self.copied;
        let filled = self.shown_token.is_some();
        let command_label = if filled { "nav:mcp.commandFilled" } else { "nav:mcp.command" };
        let command_field = command.map(|command| {
            let to_copy = command.clone();
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div().flex().items_center().gap_2().child(div().flex_1().leading_xs().child(tr(cx, command_label))).child(
                        Button::new("connect-copy")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(if copied { Lucide::CircleCheck } else { Lucide::Copy }))
                            .tooltip(copy_label)
                            .on_click(cx.listener(move |this, _, _, cx| this.copy_command(to_copy.clone(), cx))),
                    ),
                )
                .child(
                    div()
                        .id("connect-command")
                        .test_support()
                        .w_full()
                        .overflow_hidden()
                        .rounded(px(4.))
                        .bg(field)
                        .px_2()
                        .py_1p5()
                        .leading_xs()
                        .font_family(mono.clone())
                        .child(command),
                )
                .children((!filled).then(|| {
                    div().w_full().leading_xs().text_color(muted).child(cac_core::text_with(here, "nav:mcp.tokenHelp", &[("placeholder", TOKEN_PLACEHOLDER)]))
                }))
        });
        let panel = move || div().w_full().flex().flex_col().gap_2().rounded(px(8.)).border_1().border_color(border).p_3();

        let dialog = div()
            .id("connect-dialog")
            .test_support()
            .w(px(WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .rounded(px(10.))
            .border_1()
            .border_color(border)
            .max_h_full()
            .overflow_y_scroll()
            .bg(popover)
            .text_color(foreground)
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(div().w_full().leading_lg().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:admin.connectClaude")))
                    .child(div().w_full().leading_sm().text_color(muted).child(tr(cx, "nav:mcp.lead"))),
            )
            .child(
                panel()
                    .child(div().leading_xs().font_family(mono.clone()).font_weight(FontWeight::MEDIUM).child("cac-mcp"))
                    .child(machine)
                    .children(outcome),
            )
            .child(panel().child(token_panel))
            .children(command_field.map(|field| panel().child(field)))
            .child(
                div().flex().justify_end().child(
                    Button::new("connect-close")
                        .outline()
                        .small()
                        .label(tr(cx, "common:action.close"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectClosed))),
                ),
            );
        div()
            .size_full()
            .track_focus(&self.focus)
            // Tapa lo de debajo: mientras está abierta no se pulsa nada más.
            .occlude()
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(ConnectClosed);
                    cx.stop_propagation();
                }
            }))
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(background.opacity(0.7))
            .child(dialog)
    }
}
