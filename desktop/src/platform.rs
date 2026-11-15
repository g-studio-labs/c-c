//! Lo que la app le pregunta a la máquina: dónde guarda lo suyo, con qué
//! servidor habla y en qué idioma está el sistema.
//!
//! Todo recibe el entorno por parámetro: así se prueba sin tocar el de verdad.

use std::path::PathBuf;

/// El servidor de cac.
pub const DEFAULT_SERVER: &str = "https://cac.guz-studio.dev";
/// Para apuntar la app a otro servidor (uno local, al desarrollar).
pub const SERVER_VAR: &str = "CAC_API_URL";

/// El nombre de la carpeta de esta app.
///
/// **No es la de la app de Tauri**, y tiene que seguir sin serlo: las dos
/// conviven en la misma máquina y el servidor rota el refresh al usarlo. Con
/// el fichero de sesión compartido, la segunda en renovar presentaría un
/// refresh gastado y el servidor cerraría la sesión en todas partes.
pub const APP_DIR: &str = "cac-desktop";

/// El sistema, por lo que cambia dónde se guardan los datos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

fn set(var: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    var(name).filter(|value| !value.trim().is_empty())
}

/// Dónde guarda la app su sesión y sus preferencias, según la costumbre de
/// cada sistema. `None` si la máquina no dice dónde vive su usuario.
pub fn data_dir(os: Os, var: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let base = match os {
        Os::Linux => match set(&var, "XDG_DATA_HOME") {
            Some(data) => PathBuf::from(data),
            None => PathBuf::from(set(&var, "HOME")?).join(".local").join("share"),
        },
        Os::MacOs => PathBuf::from(set(&var, "HOME")?).join("Library").join("Application Support"),
        Os::Windows => PathBuf::from(set(&var, "APPDATA")?),
    };
    Some(base.join(APP_DIR))
}

/// Con qué servidor habla.
pub fn server_url(var: impl Fn(&str) -> Option<String>) -> String {
    set(&var, SERVER_VAR).unwrap_or_else(|| DEFAULT_SERVER.to_string())
}

/// El idioma del sistema, tal como lo dice el entorno, en el orden en que
/// POSIX lo lee. Qué idioma de cac es eso lo decide el core.
pub fn system_language(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|name| set(&var, name))
}

#[cfg(test)]
mod tests {
    use super::{data_dir, server_url, system_language, Os, DEFAULT_SERVER};
    use std::path::PathBuf;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| value.to_string())
    }

    /// La sesión de esta app va en su propia carpeta, en cada sistema.
    ///
    /// Roturas que atrapa: guardar en la carpeta de la app de Tauri (las dos
    /// se cerrarían la sesión la una a la otra al renovar); escribir en una
    /// ruta relativa cuando la máquina no dice dónde vive el usuario.
    #[test]
    fn the_session_lives_in_this_apps_own_folder() {
        let home = [("HOME", "/home/ana")];
        assert_eq!(data_dir(Os::Linux, env(&home)), Some(PathBuf::from("/home/ana/.local/share/cac-desktop")));
        assert_eq!(
            data_dir(Os::Linux, env(&[("HOME", "/home/ana"), ("XDG_DATA_HOME", "/datos")])),
            Some(PathBuf::from("/datos/cac-desktop"))
        );
        assert_eq!(
            data_dir(Os::MacOs, env(&home)),
            Some(PathBuf::from("/home/ana/Library/Application Support/cac-desktop"))
        );
        assert_eq!(
            data_dir(Os::Windows, env(&[("APPDATA", "C:/Users/ana/AppData/Roaming")])),
            Some(PathBuf::from("C:/Users/ana/AppData/Roaming").join("cac-desktop"))
        );
        assert_eq!(data_dir(Os::Linux, env(&[("HOME", " ")])), None);
        assert_eq!(data_dir(Os::Windows, env(&home)), None);

        // El identificador de la app de Tauri: su carpeta no es ésta.
        let tauri: serde_json::Value =
            serde_json::from_str(include_str!("../../app/src-tauri/tauri.conf.json")).unwrap();
        let tauri_dir = tauri["identifier"].as_str().expect("la app de Tauri tiene identificador");
        assert_ne!(tauri_dir, super::APP_DIR);
    }

    /// Habla con el servidor de cac, salvo que se le diga otro.
    ///
    /// Roturas que atrapa: no poder apuntarla a un servidor local; tomar una
    /// variable vacía por un servidor.
    #[test]
    fn it_talks_to_cac_unless_told_otherwise() {
        assert_eq!(server_url(env(&[])), DEFAULT_SERVER);
        assert_eq!(server_url(env(&[("CAC_API_URL", "")])), DEFAULT_SERVER);
        assert_eq!(server_url(env(&[("CAC_API_URL", "http://localhost:8080")])), "http://localhost:8080");
    }

    /// El idioma del sistema se lee en el orden de POSIX.
    ///
    /// Rotura que atrapa: mirar sólo `LANG`, que `LC_ALL` y `LC_MESSAGES`
    /// pisan.
    #[test]
    fn the_system_language_is_read_in_posix_order() {
        let all = [("LANG", "en_US.UTF-8"), ("LC_MESSAGES", "es_MX.UTF-8"), ("LC_ALL", "fr_FR")];
        assert_eq!(system_language(env(&all)).as_deref(), Some("fr_FR"));
        assert_eq!(system_language(env(&all[..2])).as_deref(), Some("es_MX.UTF-8"));
        assert_eq!(system_language(env(&all[..1])).as_deref(), Some("en_US.UTF-8"));
        assert_eq!(system_language(env(&[("LC_ALL", "")])), None);
    }
}
