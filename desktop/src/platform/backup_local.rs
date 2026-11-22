//! Lo que los backups hacen en esta máquina: crear la llave age en
//! 1Password y bajar un backup a un fichero. Va detrás de un trait para que
//! las pruebas no lancen `op` ni bajen nada de verdad; lo de verdad es
//! `cac_ops` (`secrets::create_backup_key`, `op::list_vaults`, `download`),
//! el mismo código que usa la app de Tauri.

use cac_core::server_edit_phrase;
use cac_ops::op::OpVault;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

type Pending<T> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, String>> + Send>>;

/// Cuánto se lleva bajado y cuánto es, si se sabe.
pub type Progress = Box<dyn FnMut(u64, Option<u64>) + Send>;

pub trait BackupLocal: Send + Sync {
    /// Las bóvedas de 1Password donde se puede escribir; vacío si no se
    /// llega a `op`. Bloquea.
    fn vaults(&self) -> Vec<OpVault>;
    /// Crea la llave en 1Password y devuelve `(op://bóveda/ítem, pública)`.
    /// La privada no sale de ahí. Bloquea.
    fn create_key(&self, account: &str, vault: &str, title: &str) -> Result<(String, String), String>;
    /// Baja la URL firmada a `target`, sin dejar nada a medias si falla.
    fn download(&self, url: String, target: std::path::PathBuf, progress: Progress) -> Pending<()>;
}

struct SystemBackupLocal;

impl BackupLocal for SystemBackupLocal {
    fn vaults(&self) -> Vec<OpVault> {
        cac_ops::op::list_vaults()
    }
    fn create_key(&self, account: &str, vault: &str, title: &str) -> Result<(String, String), String> {
        cac_ops::secrets::create_backup_key(account, vault, title).map(|key| (key.item, key.public_key))
    }
    fn download(&self, url: String, target: std::path::PathBuf, mut progress: Progress) -> Pending<()> {
        Box::pin(async move { cac_ops::download::download_to(&url, &target, |done, total| progress(done, total)).await })
    }
}

/// Los de esta app: los del sistema, salvo que alguien ponga otros.
pub struct LocalBackups(pub std::sync::Arc<dyn BackupLocal>);

impl Global for LocalBackups {}

pub(crate) fn backup_local(cx: &App) -> std::sync::Arc<dyn BackupLocal> {
    cx.try_global::<LocalBackups>().map(|local| local.0.clone()).unwrap_or_else(|| std::sync::Arc::new(SystemBackupLocal))
}

/// El diálogo se cerró; con la llave creada, lleva la frase que lo dice.
pub struct CreateKeyClosed(pub Option<String>);

/// Crear la llave de los backups directamente en 1Password
/// (`CreateKeyDialog` de `ServerBackups.tsx`). La genera `cac_ops` en
/// memoria y la guarda con `op`: aquí sólo vuelve la pública, que se
/// registra con un nombre que dice dónde vive la privada.
pub struct CreateKeyDialog {
    /// `None` mientras se pregunta a `op`.
    vaults: Option<Vec<OpVault>>,
    chosen: usize,
    title: Entity<InputState>,
    busy: bool,
    problem: Option<String>,
}

impl EventEmitter<CreateKeyClosed> for CreateKeyDialog {}

impl CreateKeyDialog {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).default_value("cac backups – llave age"));
        let local = backup_local(cx);
        let vaults = Backend::ask(cx, move |_| async move { tokio::task::spawn_blocking(move || local.vaults()).await.unwrap_or_default() });
        cx.spawn(async move |this, cx| {
            let Ok(vaults) = vaults.await else { return };
            this.update(cx, |this, cx| {
                this.vaults = Some(vaults);
                cx.notify();
            })
            .ok();
        })
        .detach();
        CreateKeyDialog { vaults: None, chosen: 0, title, busy: false, problem: None }
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vaults.as_ref().and_then(|vaults| vaults.get(self.chosen)).cloned() else { return };
        let title = self.title.read(cx).value().trim().to_string();
        if self.busy || title.is_empty() || title.contains('/') {
            return;
        }
        self.busy = true;
        self.problem = None;
        cx.notify();
        let (local, language) = (backup_local(cx), locale(cx));
        let answer = Backend::ask(cx, move |core| async move {
            let (account, place, name) = (vault.account.clone(), vault.vault.clone(), title.clone());
            let (item, public_key) = tokio::task::spawn_blocking(move || local.create_key(&account, &place, &name)).await.map_err(|error| error.to_string())??;
            let label = format!("{title} (1Password · {})", vault.vault);
            core.platform().add_backup_recipient(&label, &public_key).await.map_err(|error| server_edit_phrase(language, &error, "common:backups.create"))?;
            Ok::<_, String>(item)
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(item) => cx.emit(CreateKeyClosed(Some(cac_core::text_with(locale(cx), "common:backups.created", &[("item", &item)])))),
                    Err(problem) => this.problem = Some(problem),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for CreateKeyDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.danger);
        let busy = self.busy;
        let title_value = self.title.read(cx).value().trim().to_string();
        let body = match &self.vaults {
            None => div().id("backup-op-loading").text_color(muted).child(tr(cx, "common:servers.loading")).test_support().into_any_element(),
            Some(vaults) if vaults.is_empty() => div().id("backup-op-unavailable").text_color(danger).child(tr(cx, "common:backups.opUnavailable")).test_support().into_any_element(),
            Some(vaults) => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div().flex().flex_col().gap_1p5().child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:backups.vault"))).child(div().flex().flex_wrap().gap_1().children(vaults.iter().enumerate().map(|(n, vault)| {
                        let chosen = n == self.chosen;
                        Button::new(SharedString::from(format!("backup-vault-{n}")))
                            .xsmall()
                            .when(chosen, |button| button.primary())
                            .when(!chosen, |button| button.outline())
                            .label(format!("{} · {}", vault.vault, vault.email))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.chosen = n;
                                cx.notify();
                            }))
                    }))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(div().leading_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:backups.itemTitle")))
                        .child(div().id("backup-op-title").child(Input::new(&self.title).small().disabled(busy)).test_support()),
                )
                .into_any_element(),
        };
        let ready = self.vaults.as_ref().is_some_and(|vaults| !vaults.is_empty()) && !title_value.is_empty() && !title_value.contains('/');
        let card = div()
            .id("backup-op-dialog")
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(CreateKeyClosed(None));
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
            .child(div().text_size(rems(1.)).font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:backups.createTitle")))
            .child(div().leading_xs().text_color(muted).child(tr(cx, "common:backups.createLead")))
            .child(body)
            .children(self.problem.clone().map(|problem| div().id("backup-op-problem").flex().items_center().gap_1p5().leading_xs().text_color(danger).child(Icon::new(Lucide::CircleAlert).size_3()).child(problem).test_support()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("backup-op-cancel").outline().small().label(tr(cx, "common:servers.cancel")).on_click(cx.listener(|_, _, _, cx| cx.emit(CreateKeyClosed(None)))))
                    .child(Button::new("backup-op-create").primary().small().label(tr(cx, "common:backups.create")).disabled(busy || !ready).on_click(cx.listener(|this, _, window, cx| this.create(window, cx)))),
            )
            .test_support();
        super::veil("backup-op-veil", card, window)
    }
}
