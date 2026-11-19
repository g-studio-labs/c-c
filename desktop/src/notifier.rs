//! El aviso del sistema: el globo del escritorio cuando llega un mensaje, y
//! lo que pasa al pulsarlo.
//!
//! Qué se avisa lo decide el core (`cac_core::notice`): aquí sólo se enseña,
//! en el idioma de la app, y se lleva a la conversación al pulsar. El aviso
//! lo pinta el sistema por medio de GPUI (en Linux, por D-Bus al servidor de
//! avisos del escritorio; en macOS y Windows, su centro de avisos), y el clic
//! vuelve al hilo principal con la etiqueta del aviso.
//!
//! En la app de Tauri pulsar un aviso no hacía nada.

use std::collections::HashMap;

use cac_core::{Announcement, NoticeTarget};
use gpui_kit::{App, Global, SharedString, SystemNotification, SystemNotificationResponse};

use crate::backend::Backend;
use crate::lang::locale;

/// A dónde lleva cada aviso enseñado, por su etiqueta. Una etiqueta es una
/// conversación (`chat:<espacio>`, `dm:<conversación>`), así que esto crece
/// como mucho hasta las conversaciones que se tienen.
#[derive(Default)]
struct Notifier {
    targets: HashMap<SharedString, NoticeTarget>,
}

impl Global for Notifier {}

/// Lo prepara al arrancar: sin esto, pulsar un aviso no llega a nadie.
pub fn install(cx: &mut App) {
    cx.set_global(Notifier::default());
    cx.on_system_notification_response(respond);
}

/// Enseña un aviso que el core decidió dar.
pub(crate) fn show(announcement: Announcement, cx: &mut App) {
    let language = locale(cx);
    let tag = SharedString::from(announcement.tag.clone());
    let notification = SystemNotification {
        tag: tag.clone(),
        title: announcement.title(language).into(),
        body: announcement.body(language).into(),
        actions: Vec::new(),
    };
    let kind = announcement.kind;
    cx.global_mut::<Notifier>().targets.insert(tag, announcement.target);
    cx.show_system_notification(notification);
    // Después de enseñarlo: un tono sin aviso no dice de qué es.
    crate::sound::after_notice(kind, cx);
}

/// Se pulsó un aviso: la ventana al frente, y su conversación delante.
fn respond(response: SystemNotificationResponse, cx: &mut App) {
    let Some(target) = cx.try_global::<Notifier>().and_then(|notifier| notifier.targets.get(&response.tag).cloned()) else { return };
    cx.activate(true);
    if let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) {
        let _ = window.update(cx, |_, window, _| window.activate_window());
    }
    Backend::run(cx, move |core| async move {
        core.notices().open(target).await;
    });
}
