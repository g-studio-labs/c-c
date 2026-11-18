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
//! **No crea ni enseña tokens.** La orden lleva un marcador donde va el de la
//! persona, y dice dónde se saca.
//!
//! El aspecto es el de `ConnectMcpDialog.tsx`: el título y la entradilla, los
//! bloques con borde, y la orden en un campo que se copia.

use std::time::Duration;

use cac_mcp::update::{Installed, Machine, Progress, Status, Step, TOKEN_PLACEHOLDER};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::*;

use crate::lang::{locale, tr};
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

/// Se pidió cerrar: quien la tiene la quita.
pub struct ConnectClosed;

pub struct ConnectView {
    /// Lo que se sabe de la máquina. `None` mientras se mira por primera vez.
    pub(crate) status: Option<Status>,
    pub(crate) work: Work,
    copied: bool,
    focus: FocusHandle,
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
        Self { status: None, work: Work::Idle, copied: false, focus: cx.focus_handle() }
    }

    /// Al abrirla: se mira ahora, preguntando por la última versión, y un
    /// resultado de antes (hecho o fallido) deja de enseñarse.
    pub fn opened(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.work, Work::Running(_)) {
            self.work = Work::Idle;
        }
        self.copied = false;
        self.look(0, cx);
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

impl Render for ConnectView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            Some(cac_mcp::update::register_command(&setup.server_url, &target))
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
        let command_field = command.map(|command| {
            let to_copy = command.clone();
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div().flex().items_center().gap_2().child(div().flex_1().leading_xs().child(tr(cx, "nav:mcp.command"))).child(
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
                .child(div().w_full().leading_xs().text_color(muted).child(cac_core::text_with(
                    here,
                    "nav:mcp.tokenHelp",
                    &[("placeholder", TOKEN_PLACEHOLDER)],
                )))
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
