//! El aviso de «hay una versión nueva» y lo que hace el botón.
//!
//! Qué versión hay, qué se baja en cada sistema y cómo se comprueba lo decide
//! `cac_mcp::desktop_update` (la misma lógica que actualiza `cac-mcp`); aquí
//! sólo se pinta y se espera un clic. **Nada se instala solo**: se mira al
//! arrancar y una vez al día, en otro hilo, y lo único que pasa es que sale el
//! aviso. Bajar y sustituir es lo que hace «Actualizar».
//!
//! Después de sustituir el binario se ofrece reiniciar; en Windows el
//! instalador necesita que la app esté cerrada, así que se baja y se espera a
//! «Instalar y cerrar» (cerrar una app con una llamada en curso no se hace sin
//! preguntar); en macOS se abre la imagen de disco y arrastrar a Aplicaciones
//! lo hace la persona. Una instalación que no es de la persona (un `.deb`) sólo
//! enseña la página de la descarga.

use std::path::PathBuf;
use std::time::Duration;

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
    /// El instalador de Windows está bajado y comprobado; falta que la persona
    /// diga que cierre la app.
    InstallReady { outcome: Outcome, offer: Offer },
    DiskOpened { version: String },
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

/// Lo bajado hace menos de esto no se borra al arrancar: puede ser de otra
/// instancia que está instalando ahora.
const LEFTOVERS_AFTER: Duration = Duration::from_secs(3600);

impl UpdateNotice {
    pub fn new(cx: &mut Context<Self>) -> Self {
        if cx.has_global::<UpdateSetup>() {
            cx.spawn(async move |this, cx| loop {
                let Ok(setup) = this.update(cx, |_, cx| cx.global::<UpdateSetup>().0.clone()) else { break };
                // Fuera del hilo que pinta: lanza la red y toca el disco.
                let found = cx
                    .background_spawn(async move {
                        setup.clean_leftovers(LEFTOVERS_AFTER);
                        setup.check(CHECK_EVERY)
                    })
                    .await;
                if let Some(offer) = found {
                    if this.update(cx, |this, cx| this.found(offer, cx)).is_err() {
                        break;
                    }
                }
                cx.background_executor().timer(CHECK_EVERY).await;
            })
            .detach();
        }
        Self { state: State::Quiet, dismissed: None }
    }

    /// Una versión nueva publicada. No pisa lo que la persona tiene entre
    /// manos, ni vuelve con una versión a la que ya dijo «ahora no».
    pub(crate) fn found(&mut self, offer: Offer, cx: &mut Context<Self>) {
        let idle = matches!(self.state, State::Quiet | State::Offered(_));
        if idle && self.dismissed.as_deref() != Some(offer.version.as_str()) {
            self.state = State::Offered(offer);
            cx.notify();
        }
    }

    /// «Ahora no», en el estado que sea: esa versión no vuelve a avisar, ni a
    /// bajarse otra vez mañana si ya estaba puesta.
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        match &self.state {
            State::Offered(offer) | State::Failed { offer, .. } | State::InstallReady { offer, .. } => self.dismissed = Some(offer.version.clone()),
            State::Ready { version, .. } | State::RestartFailed { version, .. } | State::DiskOpened { version } => self.dismissed = Some(version.clone()),
            State::Quiet | State::Running(_) => {}
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
        cx.background_spawn(async move {
            let steps = sender.clone();
            let end = updater.install(&task, &mut |step| {
                let _ = steps.send(Report::Step(step));
            });
            // La imagen de disco se abre sola: no cierra nada. El instalador de
            // Windows no, que cierra la app: espera al clic.
            let end = end.and_then(|outcome| match &outcome {
                Outcome::Downloaded { delivery: Delivery::OpenDisk, .. } => outcome.launch().map(|()| outcome),
                _ => Ok(outcome),
            });
            let _ = sender.send(Report::End(end));
        })
        .detach();
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
                        Report::End(Ok(outcome @ Outcome::Downloaded { delivery: Delivery::RunInstaller, .. })) => {
                            this.state = State::InstallReady { outcome, offer: offer.clone() };
                        }
                        Report::End(Ok(Outcome::Downloaded { .. })) => this.state = State::DiskOpened { version: offer.version.clone() },
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

    /// El instalador de Windows no puede sustituir un `.exe` en marcha: se
    /// lanza y la app se cierra. Sólo con el clic de la persona.
    fn install_and_close(&mut self, outcome: Outcome, offer: Offer, cx: &mut Context<Self>) {
        match outcome.launch() {
            Ok(()) => cx.quit(),
            Err(reason) => {
                self.state = State::Failed { offer, reason };
                cx.notify();
            }
        }
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
            State::InstallReady { outcome, offer } => {
                let text = cac_core::text_with(here, "nav:updater.installReady", &[("version", &offer.version)]);
                let install = Button::new("update-install")
                    .primary()
                    .small()
                    .label(tr(cx, "nav:updater.installAndClose"))
                    .on_click(cx.listener(move |this, _, _, cx| this.install_and_close(outcome.clone(), offer.clone(), cx)))
                    .into_any_element();
                ("update-ready", text, None, vec![later("update-dismiss", "nav:updater.later", cx).into_any_element(), install])
            }
            State::DiskOpened { .. } => {
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
    use cac_mcp::desktop_update::{asset_name, Updater};
    use cac_mcp::update::fake::FakeReleases;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};

    use super::{State, UpdateNotice, UpdateSetup};
    use crate::testing::FakeServer;

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

    /// Abre una ventana con el aviso y deja correr el hilo que mira la versión.
    /// Sin esperas: la mirada y las descargas corren en el ejecutor de fondo
    /// de la prueba, y `run_until_parked` vuelve cuando terminaron.
    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<UpdateNotice>) {
        let opened = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(900.), px(600.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |_, cx| cx.new(UpdateNotice::new)).expect("open test window")
        });
        cx.run_until_parked();
        opened
    }

    struct Machine {
        releases: FakeReleases,
        scratch: std::path::PathBuf,
        exe: std::path::PathBuf,
        updater: Updater,
        _server: FakeServer,
    }

    fn machine(name: &str, os: &'static str, arch: &'static str, cx: &mut TestAppContext) -> Machine {
        let releases = FakeReleases::start();
        let scratch = std::env::temp_dir().join(format!("cac-desktop-notice-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let exe = scratch.join("bin").join("cac-desktop");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, OLD).unwrap();
        let updater = Updater {
            source: releases.desktop_source(),
            state_dir: Some(scratch.join("state")),
            current: "0.1.0".into(),
            exe: Some(exe.clone()),
            os,
            arch,
            download_dir: scratch.join("downloads"),
        };
        let server = FakeServer::start();
        server.install(cx);
        // El core de la app despierta tareas desde su propio hilo; es lo único
        // que lo permite. Lo nuestro (mirar la versión, bajar) corre entero
        // dentro de la prueba y no espera a ningún reloj.
        cx.executor().allow_parking();
        cx.update(|cx| cx.set_global(UpdateSetup(updater.clone())));
        Machine { releases, scratch, exe, updater, _server: server }
    }

    impl Machine {
        /// Lo que se supo hace un momento vale un día: se olvida para que la
        /// próxima ventana pregunte de nuevo.
        fn forget_what_was_known(&self) {
            let _ = std::fs::remove_dir_all(self.scratch.join("state"));
        }
    }

    fn finish(cx: &mut TestAppContext, machine: Machine) {
        cx.update(|cx| crate::backend::Backend::run(cx, |core| async move { core.stream().stop() }));
        let _ = std::fs::remove_dir_all(machine.scratch);
    }

    /// El aviso sale solo, pero bajar e instalar espera a un clic; y no sale
    /// una versión que todavía no está publicada.
    ///
    /// Roturas que atrapa: instalar al ver la versión nueva sin que nadie
    /// pulse; ofrecer lo que `VERSION` anuncia antes de que la release exista
    /// (el clic acabaría en un 404); un «Actualizar» que no sustituye el
    /// binario; que tras sustituirlo no ofrezca reiniciar; que «Ahora no» no
    /// calle esa versión, ni siquiera estando ya puesta (se bajaría otra vez
    /// mañana); una descarga rechazada que toque lo instalado o no se diga.
    #[gpui_kit::test]
    #[cfg(target_os = "linux")]
    fn the_notice_offers_only_what_is_published_and_only_a_click_installs(cx: &mut TestAppContext) {
        let m = machine("linux", "linux", "x86_64", cx);
        let (name, _) = asset_name("0.2.0", "linux", "x86_64").unwrap();
        let downloaded = |m: &Machine| m.releases.asked().iter().any(|path| path.ends_with(&name));

        // Al día: se mira, y no hay nada que avisar.
        m.releases.announce_desktop("0.1.0");
        let (window, _) = open(cx);
        assert!(m.releases.asked().iter().any(|path| path == "/desktop/VERSION"), "no miró la versión al arrancar");
        assert!(!on_screen(window, "update-notice", cx), "avisa de la versión que ya corre");

        // `VERSION` ya dice 0.2.0 pero el workflow aún no publicó: silencio.
        m.releases.announce_desktop("0.2.0");
        m.forget_what_was_known();
        let (window, _) = open(cx);
        assert!(!on_screen(window, "update-notice", cx), "ofrece una versión que no existe todavía");

        // Publicada: sale el aviso, y no se baja nada hasta el clic.
        m.releases.publish_desktop("0.2.0", &[(&name, b"the new binary")]);
        m.forget_what_was_known();
        let (window, view) = open(cx);
        assert!(on_screen(window, "update-offer", cx), "publicada y sin aviso");
        assert!(matches!(cx.update(|cx| view.read(cx).state.clone()), State::Offered(offer) if offer.version == "0.2.0"));
        assert!(!downloaded(&m), "bajó algo sin que nadie pulsara");
        assert_eq!(std::fs::read(&m.exe).unwrap(), OLD, "cambió el binario sin que nadie pulsara");

        // «Ahora no» lo calla, y esa versión no vuelve a esa vista.
        click(window, "update-dismiss", cx);
        assert!(!on_screen(window, "update-notice", cx));
        let offer = m.updater.check(std::time::Duration::ZERO).unwrap();
        cx.update(|cx| view.update(cx, |view, cx| view.found(offer.clone(), cx)));
        assert!(!on_screen(window, "update-notice", cx), "volvió a avisar de la versión descartada");

        // Una descarga que no es la publicada: se rechaza, se dice, y lo
        // instalado sigue igual.
        m.releases.put(&format!("/download/desktop-v0.2.0/{name}"), "tampered");
        m.forget_what_was_known();
        let (window, view) = open(cx);
        click(window, "update-run", cx);
        cx.run_until_parked();
        assert!(on_screen(window, "update-problem", cx), "el fallo no se dice");
        assert!(matches!(cx.update(|cx| view.read(cx).state.clone()), State::Failed { reason, .. } if reason.contains("checksum mismatch")));
        assert_eq!(std::fs::read(&m.exe).unwrap(), OLD, "una descarga rechazada tocó lo instalado");

        // Publicada bien, y reintentando: se sustituye y se ofrece reiniciar.
        m.releases.publish_desktop("0.2.0", &[(&name, b"the new binary")]);
        click(window, "update-run", cx);
        cx.run_until_parked();
        assert!(on_screen(window, "update-ready", cx), "no quedó lista");
        assert_eq!(std::fs::read(&m.exe).unwrap(), b"the new binary");
        assert!(on_screen(window, "update-restart", cx), "no ofrece reiniciar");

        // «Ahora no» ante el reinicio: esa versión no se vuelve a ofrecer ni a
        // bajar, aunque el que corre siga siendo el viejo.
        click(window, "update-dismiss", cx);
        cx.update(|cx| view.update(cx, |view, cx| view.found(offer.clone(), cx)));
        assert!(!on_screen(window, "update-notice", cx), "tras «ahora no» al reinicio, vuelve a ofrecer lo ya puesto");
        finish(cx, m);
    }

    /// En Windows se baja el instalador y se espera: la app no se cierra hasta
    /// que la persona pulsa «Instalar y cerrar» (podría estar en una llamada).
    ///
    /// Roturas que atrapa: lanzar el instalador, y con él cerrar la app, nada
    /// más bajarlo; no dejar el botón; sustituir el `.exe` que corre.
    #[gpui_kit::test]
    #[cfg(target_os = "linux")]
    fn the_windows_installer_waits_for_install_and_close(cx: &mut TestAppContext) {
        let m = machine("windows", "windows", "x86_64", cx);
        let (name, _) = asset_name("0.2.0", "windows", "x86_64").unwrap();
        m.releases.announce_desktop("0.2.0");
        m.releases.publish_desktop("0.2.0", &[(&name, b"the installer")]);
        let (window, view) = open(cx);
        assert!(on_screen(window, "update-offer", cx));

        click(window, "update-run", cx);
        cx.run_until_parked();
        assert!(matches!(cx.update(|cx| view.read(cx).state.clone()), State::InstallReady { .. }), "no espera al clic");
        assert!(on_screen(window, "update-install", cx), "no hay «Instalar y cerrar»");
        assert_eq!(std::fs::read(m.scratch.join("downloads").join(&name)).unwrap(), b"the installer");
        assert_eq!(std::fs::read(&m.exe).unwrap(), OLD, "se tocó el .exe que corre");

        // «Ahora no» deja la app como estaba.
        click(window, "update-dismiss", cx);
        assert!(!on_screen(window, "update-notice", cx));
        finish(cx, m);
    }
}
