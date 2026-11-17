//! Quién es esta app para el sistema: su nombre, su identificador y la
//! ventana que abre con ellos.
//!
//! Los valores se escriben una sola vez, en `packaging/identity.env`; aquí
//! llegan por `build.rs`. Los instaladores leen el mismo fichero, así que la
//! ventana, el lanzador y el paquete no pueden llamarse distinto sin que la
//! prueba de abajo se queje.

use std::sync::Arc;

use gpui_kit::*;

/// El nombre que ve la persona: el título de la ventana, el lanzador.
pub const APP_NAME: &str = env!("CAC_APP_NAME");
/// El identificador con el que el sistema la reconoce. En Linux es además la
/// clase de la ventana (X11) o su `app_id` (Wayland): con él empareja el
/// escritorio la ventana con su lanzador y su icono.
pub const APP_ID: &str = env!("CAC_APP_ID");
/// Cómo se llama el binario instalado.
#[cfg(test)]
pub const APP_BINARY: &str = env!("CAC_APP_BINARY");

/// El icono de la ventana, el mismo que instala el paquete.
const WINDOW_ICON: &[u8] = include_bytes!("../packaging/icons/128x128.png");

/// El icono, descodificado. GPUI sólo lo usa en X11: en Wayland lo pone el
/// escritorio a partir del `app_id`, en macOS el `.app` y en Windows el
/// `.exe`.
fn window_icon() -> Option<Arc<image::RgbaImage>> {
    image::load_from_memory_with_format(WINDOW_ICON, image::ImageFormat::Png)
        .ok()
        .map(|icon| Arc::new(icon.into_rgba8()))
}

/// La ventana principal: su tamaño, y a nombre de quién se abre.
pub fn main_window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1100.), px(760.)), cx))),
        titlebar: Some(TitlebarOptions { title: Some(APP_NAME.into()), ..TitlebarOptions::default() }),
        app_id: Some(APP_ID.to_string()),
        icon: window_icon(),
        ..WindowOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{main_window_options, APP_BINARY, APP_ID, APP_NAME};
    use gpui_kit::TestAppContext;

    const MANIFEST: &str = include_str!("../Cargo.toml");
    const DESKTOP_ENTRY_PATH: &str = "packaging/linux/dev.guz-studio.cac.native.desktop";
    const DESKTOP_ENTRY: &str = include_str!("../packaging/linux/dev.guz-studio.cac.native.desktop");
    const INFO_PLIST: &str = include_str!("../packaging/macos/Info.plist");
    const INSTALLER: &str = include_str!("../packaging/windows/installer.nsi");

    /// El valor de una clave de un `.desktop`.
    fn entry(key: &str) -> &'static str {
        DESKTOP_ENTRY
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or_else(|| panic!("el .desktop no dice {key}"))
    }

    /// El texto de la `<string>` que sigue a una `<key>` del `Info.plist`.
    fn plist(key: &str) -> &'static str {
        let after = INFO_PLIST
            .split_once(&format!("<key>{key}</key>"))
            .unwrap_or_else(|| panic!("el Info.plist no dice {key}"))
            .1;
        let value = after.split_once("<string>").expect("la clave lleva un valor").1;
        value.split_once("</string>").expect("el valor se cierra").0
    }

    /// Esta app no es la de Tauri: ni se llama igual ni se identifica igual.
    ///
    /// Rotura que atrapa: darle el identificador o el nombre de la app de
    /// Tauri. Las dos conviven en la misma máquina, y con el mismo
    /// identificador el instalador de una se lleva el lanzador y el icono de
    /// la otra (y en macOS y Windows, la app entera).
    #[test]
    fn it_is_not_the_tauri_app() {
        let tauri: serde_json::Value =
            serde_json::from_str(include_str!("../../app/src-tauri/tauri.conf.json")).unwrap();
        let tauri_id = tauri["identifier"].as_str().expect("la app de Tauri tiene identificador");
        let tauri_name = tauri["productName"].as_str().expect("la app de Tauri tiene nombre");
        assert_ne!(APP_ID, tauri_id);
        assert!(!APP_ID.eq_ignore_ascii_case(tauri_id));
        assert!(!APP_NAME.eq_ignore_ascii_case(tauri_name));
        assert!(!APP_BINARY.eq_ignore_ascii_case(tauri_name), "en Linux el binario de Tauri se llama como su producto");
    }

    /// La ventana se abre a nombre de la app.
    ///
    /// Roturas que atrapa: abrirla sin `app_id` (el escritorio no la empareja
    /// con su lanzador: sale sin icono y sin agrupar), sin título, o sin el
    /// icono (en X11 no hay otro sitio de donde salga).
    #[gpui_kit::test]
    fn the_window_opens_in_the_apps_name(cx: &mut TestAppContext) {
        let options = cx.update(|cx| main_window_options(cx));
        assert_eq!(options.app_id.as_deref(), Some(APP_ID));
        let title = options.titlebar.and_then(|titlebar| titlebar.title);
        assert_eq!(title.as_ref().map(|title| title.as_ref()), Some(APP_NAME));
        let icon = options.icon.expect("el icono de la ventana se lee");
        assert_eq!((icon.width(), icon.height()), (128, 128));
    }

    /// El lanzador de Linux es el de esta ventana y el de este binario.
    ///
    /// Roturas que atrapa: un `StartupWMClass` que no es el `app_id` de la
    /// ventana (el escritorio abre un segundo icono genérico al lanzarla);
    /// un fichero que no se llama como el identificador (Wayland lo busca por
    /// nombre); un `Exec` o un `Icon` que apuntan a algo que el paquete no
    /// instala.
    #[test]
    fn the_linux_launcher_is_this_apps() {
        assert_eq!(DESKTOP_ENTRY_PATH, format!("packaging/linux/{APP_ID}.desktop"));
        assert_eq!(entry("Name"), APP_NAME);
        assert_eq!(entry("Exec"), APP_BINARY);
        assert_eq!(entry("Icon"), APP_ID);
        assert_eq!(entry("StartupWMClass"), APP_ID);
    }

    /// El `.deb` instala lo que el lanzador nombra.
    ///
    /// Roturas que atrapa: que el paquete instale el binario, el lanzador o
    /// los iconos con otro nombre que el de la identidad (el menú enseñaría
    /// una entrada sin icono o que no arranca), y que el binario del paquete
    /// no sea el que compila este crate.
    #[test]
    fn the_deb_installs_what_the_launcher_names() {
        let deb = MANIFEST.split_once("[package.metadata.deb]").expect("Cargo.toml describe el .deb").1;
        let deb = deb.split("\n[").next().unwrap();
        assert_eq!(APP_BINARY, env!("CARGO_PKG_NAME"));
        assert!(deb.contains(&format!("name = \"{APP_BINARY}\"")));
        assert!(deb.contains(&format!("[\"target/release/{APP_BINARY}\", \"usr/bin/\", \"755\"]")));
        assert!(deb.contains(&format!("[\"{DESKTOP_ENTRY_PATH}\", \"usr/share/applications/\", \"644\"]")));
        let icons: Vec<&str> = deb.lines().filter(|line| line.contains("usr/share/icons/")).collect();
        assert_eq!(icons.len(), 5);
        for icon in icons {
            assert!(icon.contains(&format!("/apps/{APP_ID}.png\"")), "{icon}");
        }
    }

    /// Los instaladores de macOS y Windows no llevan la identidad escrita:
    /// la reciben al empaquetar.
    ///
    /// Roturas que atrapa: fijar en el `Info.plist` o en el instalador de
    /// Windows un nombre, un identificador o una versión a mano, que se
    /// quedarían viejos al renombrar la app o al subir la versión de
    /// `Cargo.toml` sin que nada lo dijera.
    #[test]
    fn the_other_installers_take_the_identity_when_packaging() {
        assert_eq!(plist("CFBundleIdentifier"), "__APP_ID__");
        assert_eq!(plist("CFBundleName"), "__APP_NAME__");
        assert_eq!(plist("CFBundleDisplayName"), "__APP_NAME__");
        assert_eq!(plist("CFBundleExecutable"), "__APP_BINARY__");
        assert_eq!(plist("CFBundleShortVersionString"), "__APP_VERSION__");
        assert_eq!(plist("CFBundleVersion"), "__APP_VERSION__");
        for written in [APP_NAME, APP_ID, APP_BINARY, env!("CARGO_PKG_VERSION")] {
            assert!(!INFO_PLIST.contains(written), "el Info.plist lleva escrito «{written}»");
            assert!(!INSTALLER.contains(written), "el instalador de Windows lleva escrito «{written}»");
        }
        for define in ["APP_NAME", "APP_ID", "APP_BINARY", "APP_VERSION"] {
            assert!(INSTALLER.contains(&format!("${{{define}}}")), "el instalador no usa {define}");
            assert!(!INSTALLER.contains(&format!("!define {define} ")), "el instalador fija {define}");
        }
    }

    /// Lo que leen los empaquetadores es lo que lleva el binario.
    ///
    /// Roturas que atrapa: que `common.sh` lleve la versión escrita en vez de
    /// sacarla de `Cargo.toml`, o que lea otra identidad que la del binario.
    /// El instalador se llamaría de una versión y la app de otra.
    ///
    /// No atrapa hoy que deje de acotar la búsqueda a `[package]`: con el
    /// `Cargo.toml` de ahora la primera `version =` ya es la del paquete. Lo
    /// atrapará el día que una dependencia en varias líneas quede por delante.
    #[cfg(unix)]
    #[test]
    fn the_packagers_read_what_the_binary_carries() {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/common.sh");
        let output = std::process::Command::new("bash").arg(script).arg("--print").output().expect("hay bash");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let printed = String::from_utf8(output.stdout).unwrap();
        let expected = format!(
            "APP_NAME={APP_NAME}\nAPP_ID={APP_ID}\nAPP_BINARY={APP_BINARY}\nAPP_VERSION={}\n",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(printed, expected);
    }
}
