//! cac de escritorio, nativo, en GPUI.
//!
//! La ventana y sus pantallas. Sin lógica propia: toda regla vive en
//! `cac-core`, y aquí se pinta lo que el core diga.
//!
//! Con `CAC_SPIKE=1` abre la prueba de viabilidad de la Fase 0 en vez de la
//! app (command-and-control-374).

mod assets;
mod auth;
mod backend;
mod lang;
mod mywork;
mod platform;
mod shell;
mod spike;
mod theme;
mod workspace;
#[cfg(test)]
mod testing;

use std::sync::Arc;

use cac_core::{Config, FileStorage};
use gpui_kit::*;

/// Lo que GPUI cuenta de sí mismo, a la salida de errores.
///
/// Sólo con `CAC_LOG=1`. Está para medir: ante una ventana que se pinta mal,
/// lo primero es saber qué tarjeta y qué modo eligió GPUI, no suponerlo.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{}] {}: {}", record.level(), record.target(), record.args());
        }
    }
    fn flush(&self) {}
}

/// La variable con la que GPUI deja fijar la gráfica. El nombre es de Zed.
const GPUI_DEVICE_VAR: &str = "ZED_DEVICE_ID";
/// La de cac, para quien de verdad quiera fijarla en esta app.
const CAC_DEVICE_VAR: &str = "CAC_DEVICE_ID";

/// Qué gráfica se le pide a GPUI: la que diga `CAC_DEVICE_ID`, o ninguna.
///
/// Lo que haya en `ZED_DEVICE_ID` no cuenta: es un ajuste de otro programa.
/// En la máquina de jose estaba puesto para Zed (la Intel), la pantalla la
/// lleva la NVIDIA, y cac heredaba la Intel: al pegar texto salían manchas
/// (15-nov-2026, command-and-control-374). Sin fijar nada, GPUI elige bien.
fn device_to_ask_for(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    var(CAC_DEVICE_VAR).filter(|id| !id.trim().is_empty())
}

fn main() {
    if std::env::var_os("CAC_LOG").is_some() {
        let _ = log::set_logger(&StderrLog).map(|()| log::set_max_level(log::LevelFilter::Info));
    }

    // Antes de arrancar GPUI y sin más hilos: es cuando se puede tocar el
    // entorno sin que nadie lo esté leyendo.
    match device_to_ask_for(|name| std::env::var(name).ok()) {
        Some(id) => std::env::set_var(GPUI_DEVICE_VAR, id),
        None => std::env::remove_var(GPUI_DEVICE_VAR),
    }

    let var = |name: &str| std::env::var(name).ok();
    let spike = std::env::var_os("CAC_SPIKE").is_some();
    let Some(data_dir) = platform::data_dir(platform::Os::current(), var) else {
        eprintln!("cac: esta máquina no dice dónde guardar los datos de su usuario");
        std::process::exit(1);
    };
    let mut config = Config::new(platform::server_url(var));
    config.client = Some("desktop".to_string());
    let system_language = platform::system_language(var);

    gpui_kit::application().with_assets(assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        cx.set_global(lang::SystemLanguage(system_language));
        if let Err(error) = backend::Backend::install(config, Arc::new(FileStorage::new(data_dir)), cx) {
            eprintln!("cac: no se pudo arrancar: {error}");
            std::process::exit(1);
        }
        theme::apply_preference(cx);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1100.), px(760.)),
                cx,
            ))),
            ..WindowOptions::default()
        };
        if spike {
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| spike::Spike::new(window, cx)))
                .expect("no se pudo abrir la ventana");
        } else {
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| shell::Shell::new(window, cx)))
                .expect("no se pudo abrir la ventana");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{device_to_ask_for, CAC_DEVICE_VAR, GPUI_DEVICE_VAR};

    /// La gráfica que fijó otro programa no se hereda.
    ///
    /// Rotura que atrapa: volver a hacer caso de `ZED_DEVICE_ID`, que en la
    /// máquina de jose ponía a cac a pintar con la gráfica que no lleva la
    /// pantalla.
    #[test]
    fn another_programs_gpu_choice_is_not_inherited() {
        let only_zed = |name: &str| (name == GPUI_DEVICE_VAR).then(|| "9a68".to_string());
        assert_eq!(device_to_ask_for(only_zed), None);
    }

    /// Quien quiera fijarla en cac, puede, con la variable de cac.
    ///
    /// Rotura que atrapa: que la app no deje elegir gráfica de ninguna manera.
    #[test]
    fn cac_has_its_own_way_to_pin_the_gpu() {
        let both = |name: &str| match name {
            n if n == CAC_DEVICE_VAR => Some("25a2".to_string()),
            n if n == GPUI_DEVICE_VAR => Some("9a68".to_string()),
            _ => None,
        };
        assert_eq!(device_to_ask_for(both).as_deref(), Some("25a2"));
        let blank = |name: &str| (name == CAC_DEVICE_VAR).then(|| "  ".to_string());
        assert_eq!(device_to_ask_for(blank), None);
    }
}
