//! El aviso de «hay una versión nueva» y lo que hace el botón.
//!
//! Qué versión hay, qué se baja en cada sistema y cómo se comprueba lo decide
//! `cac_mcp::desktop_update` (la misma lógica que actualiza `cac-mcp`); aquí
//! sólo se pinta y se espera un clic. **Nada se instala solo**: se mira al
//! arrancar y una vez al día, en otro hilo, y lo único que pasa es que sale el
//! aviso. Bajar y sustituir es lo que hace «Actualizar».
//!
//! Después de sustituir el binario se ofrece reiniciar; en Windows el
//! instalador necesita que la app esté cerrada, así que se cierra tras
//! lanzarlo; en macOS se abre la imagen de disco y arrastrar a Aplicaciones lo
//! hace la persona. Una instalación que no es de la persona (un `.deb`) sólo
//! enseña la página de la descarga.

use std::path::PathBuf;

use cac_mcp::desktop_update::{self, Delivery, Offer, Outcome, Updater, CHECK_EVERY};
use cac_mcp::update::Progress;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::*;

use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Esta máquina para actualizar la app. Lo pone `main`; sin ello (las pruebas
/// de otras pantallas, o `CAC_DESKTOP_NO_UPDATE_CHECK`) el aviso ni mira ni
/// baja nada.
pub struct UpdateSetup(pub Updater);

impl Global for UpdateSetup {}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Quiet,
    Offered(Offer),
    Running(Option<Progress>),
    /// El binario nuevo está en su sitio; falta reiniciar.
    Ready { exe: PathBuf, version: String },
    DiskOpened,
    Failed { offer: Offer, reason: String },
    RestartFailed { exe: PathBuf, version: String },
}

pub struct UpdateNotice {
    pub(crate) state: State,
    /// La versión que se dijo «ahora no»: no vuelve a avisar de ésa.
    dismissed: Option<String>,
}

enum Report {
    Step(Progress),
    End(Result<Outcome, String>),
}

impl UpdateNotice {
    pub fn new(cx: &mut Context<Self>) -> Self {
        if cx.has_global::<UpdateSetup>() {
            cx.spawn(async move |this, cx| loop {
                let Ok(setup) = this.update(cx, |_, cx| cx.global::<UpdateSetup>().0.clone()) else { break };
                let (answer, receiver) = tokio::sync::oneshot::channel();
                std::thread::spawn(move || {
                    let _ = answer.send(setup.check(CHECK_EVERY));
                });
                if let Ok(Some(offer)) = receiver.await {
                    let shown = this.update(cx, |this, cx| {
                        let quiet = matches!(this.state, State::Quiet | State::Offered(_));
                        if quiet && this.dismissed.as_deref() != Some(offer.version.as_str()) {
                            this.state = State::Offered(offer);
                            cx.notify();
                        }
                    });
                    if shown.is_err() {
                        break;
                    }
                }
                cx.background_executor().timer(CHECK_EVERY).await;
            })
            .detach();
        }
        Self { state: State::Quiet, dismissed: None }
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if let State::Offered(offer) | State::Failed { offer, .. } = &self.state {
            self.dismissed = Some(offer.version.clone());
        }
        self.state = State::Quiet;
        cx.notify();
    }

    /// Lo único que baja e instala algo, y sólo lo llama un clic.
    fn run(&mut self, offer: Offer, cx: &mut Context<Self>) {
        let Some(setup) = cx.try_global::<UpdateSetup>() else { return };
        if matches!(self.state, State::Running(_)) {
            return;
        }
        let updater = setup.0.clone();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let task = offer.clone();
        std::thread::spawn(move || {
            let steps = sender.clone();
            let end = updater.install(&task, &mut |step| {
                let _ = steps.send(Report::Step(step));
            });
            // Lanzar el instalador o abrir la imagen, fuera del hilo que pinta.
            let end = end.and_then(|outcome| outcome.launch().map(|()| outcome));
            let _ = sender.send(Report::End(end));
        });
        self.state = State::Running(None);
        cx.notify();
        cx.spawn(async move |this, cx| {
            while let Some(report) = receiver.recv().await {
                let updated = this.update(cx, |this, cx| {
                    match report {
                        Report::Step(step) => this.state = State::Running(Some(step)),
                        Report::End(Ok(Outcome::Replaced { exe })) => {
                            this.state = State::Ready { exe, version: offer.version.clone() };
                        }
                        Report::End(Ok(Outcome::Downloaded { delivery: Delivery::RunInstaller, .. })) => {
                            // El instalador no puede sustituir un `.exe` en marcha.
                            cx.quit();
                        }
                        Report::End(Ok(Outcome::Downloaded { .. })) => this.state = State::DiskOpened,
                        Report::End(Err(reason)) => this.state = State::Failed { offer: offer.clone(), reason },
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

    fn restart(&mut self, exe: PathBuf, version: String, cx: &mut Context<Self>) {
        match desktop_update::relaunch(&exe) {
            Ok(()) => cx.quit(),
            Err(_) => {
                self.state = State::RestartFailed { exe, version };
                cx.notify();
            }
        }
    }
}

impl Render for UpdateNotice {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (popover, border, foreground, muted, danger) = (theme.popover, theme.border, theme.foreground, theme.muted_foreground, theme.danger);
        let here = locale(cx);
        let this = cx.entity().downgrade();
        let later = |id: &'static str, key: &str, cx: &mut Context<Self>| {
            Button::new(id).ghost().small().label(tr(cx, key)).on_click(cx.listener(|this, _, _, cx| this.dismiss(cx)))
        };

        let (id, text, danger_text, actions): (&'static str, String, Option<String>, Vec<AnyElement>) = match self.state.clone() {
            State::Quiet => return div().into_any_element(),
            State::Offered(offer) if offer.delivery == Delivery::Manual => {
                let page = desktop_update::release_page(&offer.version);
                let text = format!(
                    "{} {}",
                    cac_core::text_with(here, "nav:updater.available", &[("version", &offer.version)]),
                    tr(cx, "nav:updater.manual")
                );
                let view = Button::new("update-view")
                    .primary()
                    .small()
                    .label(tr(cx, "nav:updater.viewRelease"))
                    .on_click(move |_, _, cx| cx.open_url(&page))
                    .into_any_element();
                ("update-offer", text, None, vec![later("update-dismiss", "nav:updater.later", cx).into_any_element(), view])
            }
            State::Offered(offer) => {
                let text = cac_core::text_with(here, "nav:updater.available", &[("version", &offer.version)]);
                let run = Button::new("update-run")
                    .primary()
                    .small()
                    .label(tr(cx, "nav:updater.update"))
                    .on_click(cx.listener(move |this, _, _, cx| this.run(offer.clone(), cx)))
                    .into_any_element();
                ("update-offer", text, None, vec![later("update-dismiss", "nav:updater.later", cx).into_any_element(), run])
            }
            State::Running(step) => {
                let label = match step {
                    Some(Progress::Downloading { received, total: Some(total) }) if total > 0 => {
                        cac_core::text_with(here, "nav:mcp.downloadingPercent", &[("percent", &(received * 100 / total).min(100).to_string())])
                    }
                    None | Some(Progress::Downloading { .. }) => tr(cx, "nav:mcp.downloading"),
                    Some(Progress::Verifying) => tr(cx, "nav:mcp.verifying"),
                    Some(Progress::Installing) => tr(cx, "nav:mcp.installing"),
                };
                ("update-progress", label, None, vec![Spinner::new().color(muted).into_any_element()])
            }
            State::Ready { exe, version } => {
                let text = cac_core::text_with(here, "nav:updater.ready", &[("version", &version)]);
                let restart = Button::new("update-restart")
                    .primary()
                    .small()
                    .label(tr(cx, "nav:updater.restart"))
                    .on_click(cx.listener(move |this, _, _, cx| this.restart(exe.clone(), version.clone(), cx)))
                    .into_any_element();
                ("update-ready", text, None, vec![later("update-dismiss", "nav:updater.later", cx).into_any_element(), restart])
            }
            State::RestartFailed { exe, version } => {
                let retry = Button::new("update-restart")
                    .primary()
                    .small()
                    .label(tr(cx, "nav:updater.retry"))
                    .on_click(cx.listener(move |this, _, _, cx| this.restart(exe.clone(), version.clone(), cx)))
                    .into_any_element();
                ("update-problem", tr(cx, "nav:updater.restartFailed"), None, vec![retry])
            }
            State::DiskOpened => {
                ("update-opened", tr(cx, "nav:updater.diskOpened"), None, vec![later("update-dismiss", "nav:updater.close", cx).into_any_element()])
            }
            State::Failed { offer, reason } => {
                let retry = Button::new("update-run")
                    .primary()
                    .small()
                    .label(tr(cx, "nav:updater.retry"))
                    .on_click(cx.listener(move |this, _, _, cx| this.run(offer.clone(), cx)))
                    .into_any_element();
                ("update-problem", tr(cx, "nav:updater.failed"), Some(reason), vec![later("update-dismiss", "nav:updater.later", cx).into_any_element(), retry])
            }
        };
        let _ = this;

        div()
            .id("update-notice")
            .test_support()
            .absolute()
            .bottom(px(16.))
            .right(px(16.))
            .child(
                div()
                    .id(id)
                    .test_support()
                    .max_w(px(420.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .bg(popover)
                    .text_color(foreground)
                    .shadow_lg()
                    .child(div().w_full().leading_xs().child(text))
                    .children(danger_text.map(|reason| div().w_full().text_size(px(11.)).text_color(danger).child(reason)))
                    .child(div().flex().items_center().justify_end().gap_2().children(actions)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cac_mcp::desktop_update::{asset_name, Updater};
    use cac_mcp::update::fake::FakeReleases;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};

    use super::{State, UpdateNotice, UpdateSetup};
    use crate::testing::{wait_until, FakeServer};

    const OLD: &[u8] = b"the running binary";

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn click(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<UpdateNotice>) {
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(900.), px(600.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |_, cx| cx.new(UpdateNotice::new)).expect("open test window")
        })
    }

    /// El aviso sale solo, pero bajar e instalar espera a un clic.
    ///
    /// Roturas que atrapa: instalar al ver la versión nueva sin que nadie
    /// pulse (el binario cambiaría o se bajaría algo con sólo abrir la app);
    /// un «Actualizar» que no sustituye el binario; que tras sustituirlo no
    /// ofrezca reiniciar; que «Ahora no» no calle el aviso de esa versión; que
    /// una descarga rechazada toque lo instalado o no se diga.
    #[gpui_kit::test]
    #[cfg(target_os = "linux")]
    fn the_notice_offers_and_only_a_click_installs(cx: &mut TestAppContext) {
        let releases = FakeReleases::start();
        let scratch = std::env::temp_dir().join(format!("cac-desktop-notice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let exe = scratch.join("bin").join("cac-desktop");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, OLD).unwrap();
        let updater = Updater {
            source: releases.desktop_source(),
            state_dir: Some(scratch.join("state")),
            current: "0.1.0".into(),
            exe: Some(exe.clone()),
            os: "linux",
            arch: "x86_64",
            download_dir: scratch.join("downloads"),
        };
        let (name, _) = asset_name("0.2.0", "linux", "x86_64").unwrap();

        let server = FakeServer::start();
        server.install(cx);
        // Al día: nada que avisar.
        releases.announce_desktop("0.1.0");
        cx.update(|cx| cx.set_global(UpdateSetup(updater.clone())));
        let (window, view) = open(cx);
        wait_until(cx, "mira la versión", |_| releases.asked().iter().any(|path| path == "/desktop/VERSION"));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(100));
        cx.run_until_parked();
        assert!(!on_screen(window, "update-notice", cx), "avisa de la versión que ya corre");

        // Sale una nueva: el aviso, y no se baja nada hasta el clic.
        releases.announce_desktop("0.2.0");
        releases.publish_desktop("0.2.0", &[(&name, b"the new binary")]);
        // Lo que se supo hace un momento vale un día: se olvida para que
        // esta vuelta pregunte de nuevo.
        let _ = std::fs::remove_dir_all(scratch.join("state"));
        let (window, view2) = open(cx);
        wait_until(cx, "sale el aviso", |cx| on_screen(window, "update-offer", cx));
        assert!(matches!(cx.update(|cx| view2.read(cx).state.clone()), State::Offered(offer) if offer.version == "0.2.0"));
        assert!(releases.asked().iter().all(|path| !path.contains("/download/")), "bajó algo sin que nadie pulsara: {:?}", releases.asked());
        assert_eq!(std::fs::read(&exe).unwrap(), OLD, "cambió el binario sin que nadie pulsara");
        drop(view);

        // «Ahora no» lo calla.
        click(window, "update-dismiss", cx);
        assert!(!on_screen(window, "update-notice", cx));

        // Una descarga que no es la publicada: se rechaza, se dice, y lo
        // instalado sigue igual.
        releases.put(&format!("/download/desktop-v0.2.0/{name}"), "tampered");
        let (window, view3) = open(cx);
        wait_until(cx, "sale el aviso", |cx| on_screen(window, "update-run", cx));
        click(window, "update-run", cx);
        wait_until(cx, "el fallo se dice", |cx| on_screen(window, "update-problem", cx));
        assert!(matches!(cx.update(|cx| view3.read(cx).state.clone()), State::Failed { reason, .. } if reason.contains("checksum mismatch")));
        assert_eq!(std::fs::read(&exe).unwrap(), OLD, "una descarga rechazada tocó lo instalado");

        // Publicada bien, y reintentando: se sustituye y se ofrece reiniciar.
        releases.publish_desktop("0.2.0", &[(&name, b"the new binary")]);
        click(window, "update-run", cx);
        wait_until(cx, "queda lista", |cx| on_screen(window, "update-ready", cx));
        assert_eq!(std::fs::read(&exe).unwrap(), b"the new binary");
        assert!(on_screen(window, "update-restart", cx), "no ofrece reiniciar");

        cx.update(|cx| crate::backend::Backend::run(cx, |core| async move { core.stream().stop() }));
        let _ = std::fs::remove_dir_all(scratch);
    }
}
