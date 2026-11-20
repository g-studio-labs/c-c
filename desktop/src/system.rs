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
/// POSIX lo lee.
fn posix_language(var: &impl Fn(&str) -> Option<String>) -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|name| set(var, name))
}

/// El idioma del sistema. Qué idioma de cac es eso lo decide el core.
///
/// Hay dos sitios donde mirar, y cuál va primero depende del sistema:
///
/// - En Linux, el entorno: es donde el usuario lo pone (y lo que heredan las
///   apps que lanza su escritorio), en el orden de POSIX. `preferred` sólo
///   se pregunta si el entorno no dice nada.
/// - En macOS y en Windows, lo que el usuario eligió en los ajustes del
///   sistema (`preferred`). Una app abierta desde el Finder o el Explorador
///   no hereda ninguna `LANG`, y hasta el 17-nov-2026 salía siempre en
///   inglés; y la `LANG` de un terminal de macOS sale de la región, no del
///   idioma elegido. El entorno queda de reserva, por si el sistema no
///   contesta.
///
/// `preferred` da los idiomas en el orden del usuario; de ellos se toma el
/// primero que cac habla (`cac_core::first_spoken`).
pub fn system_language(os: Os, var: impl Fn(&str) -> Option<String>, preferred: impl FnOnce() -> Vec<String>) -> Option<String> {
    let asked = || cac_core::first_spoken(&preferred());
    match os {
        Os::Linux => posix_language(&var).or_else(asked),
        Os::MacOs | Os::Windows => asked().or_else(|| posix_language(&var)),
    }
}

/// Los idiomas que el usuario prefiere, preguntados al sistema: los ajustes
/// de idioma en macOS (`CFLocaleCopyPreferredLanguages`) y en Windows
/// (`GetUserPreferredUILanguages`); en Linux, otra lectura del entorno.
pub fn preferred_languages() -> Vec<String> {
    sys_locale::get_locales().collect()
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
        let read = |pairs| system_language(Os::Linux, env(pairs), Vec::new);
        assert_eq!(read(&all).as_deref(), Some("fr_FR"));
        assert_eq!(read(&all[..2]).as_deref(), Some("es_MX.UTF-8"));
        assert_eq!(read(&all[..1]).as_deref(), Some("en_US.UTF-8"));
        assert_eq!(read(&[("LC_ALL", "")]), None);
    }

    /// Dónde se mira primero depende del sistema: en Linux el entorno, y en
    /// macOS y Windows lo elegido en los ajustes; lo otro queda de reserva.
    ///
    /// Roturas que atrapa: una app abierta desde el Finder o el Explorador,
    /// sin `LANG`, que sale en inglés aunque el sistema esté en castellano;
    /// en macOS, obedecer a la `LANG` del terminal antes que a los ajustes;
    /// en Linux, que lo que diga el sistema pise la `LANG` del usuario, o
    /// preguntárselo sin hacer falta; quedarse sin idioma cuando el sistema
    /// no contesta y el entorno sí; tomar el primero de la lista aunque cac
    /// no lo hable y el segundo sí.
    #[test]
    fn where_the_system_language_is_looked_for_first() {
        let spanish = || vec!["es-MX".to_string(), "en-US".to_string()];
        let english_env = [("LANG", "en_US.UTF-8")];
        for os in [Os::MacOs, Os::Windows] {
            assert_eq!(system_language(os, env(&[]), spanish).as_deref(), Some("es-MX"), "{os:?} sin entorno");
            assert_eq!(system_language(os, env(&english_env), spanish).as_deref(), Some("es-MX"), "{os:?} con LANG");
            assert_eq!(system_language(os, env(&english_env), Vec::new).as_deref(), Some("en_US.UTF-8"), "{os:?} sin respuesta del sistema");
            assert_eq!(system_language(os, env(&[]), Vec::new), None);
            let french_first = || vec!["fr-FR".to_string(), "es-ES".to_string()];
            assert_eq!(system_language(os, env(&[]), french_first).as_deref(), Some("es-ES"));
        }
        assert_eq!(system_language(Os::Linux, env(&english_env), || panic!("se preguntó al sistema teniendo LANG")).as_deref(), Some("en_US.UTF-8"));
        assert_eq!(system_language(Os::Linux, env(&[]), spanish).as_deref(), Some("es-MX"));
        assert_eq!(system_language(Os::Linux, env(&[]), Vec::new), None);
    }

    /// Preguntarle al sistema de verdad no revienta y da etiquetas que el
    /// core sabe leer (o ninguna).
    ///
    /// Rotura que atrapa: una biblioteca que en esta plataforma no enlaza o
    /// se cae al preguntar. En Linux lee el entorno; lo de macOS y Windows
    /// sólo se comprueba al compilar y correr allí.
    #[test]
    fn asking_the_real_system_answers_with_tags() {
        for tag in super::preferred_languages() {
            assert!(!tag.trim().is_empty(), "una etiqueta vacía");
        }
    }
}
