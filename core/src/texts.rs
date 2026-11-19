//! Los textos que lee una persona, en su idioma.
//!
//! Los catálogos son los de la app de siempre (`app/src/locales/`), **los
//! mismos ficheros**: se incrustan al compilar, no se copian. Una frase se
//! escribe una vez y la leen todas las apps; con una copia por app, cada una
//! acabaría diciendo lo mismo de otra manera.
//!
//! Las reglas son las de `app/src/lib/i18n.ts` y `server-errors.ts`: el inglés
//! es el idioma base y el de reserva, y un error del servidor se dice con la
//! frase del catálogo que lleva su etiqueta.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::error::ApiError;

/// Un idioma en el que cac está escrito.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Locale {
    En,
    Es,
}

impl Locale {
    /// De una etiqueta tal como la da una persona o un sistema: `es`,
    /// `es-MX`, `es_MX.UTF-8`. `None` si no es un idioma de cac (`C`, `fr`).
    pub fn from_tag(tag: &str) -> Option<Locale> {
        let language: String = tag
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect::<String>()
            .to_ascii_lowercase();
        match language.as_str() {
            "en" => Some(Locale::En),
            "es" => Some(Locale::Es),
            _ => None,
        }
    }

    /// Como lo guarda el servidor y como se manda en `Accept-Language`.
    pub fn tag(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Es => "es",
        }
    }
}

/// En qué idioma se pinta: el que eligió la persona, si no el de su máquina,
/// y si ninguno es de cac, inglés.
///
/// Una preferencia vacía es «pregúntale a la máquina», no un idioma.
pub fn pick_locale(preferred: Option<&str>, system: Option<&str>) -> Locale {
    preferred
        .and_then(Locale::from_tag)
        .or_else(|| system.and_then(Locale::from_tag))
        .unwrap_or(Locale::En)
}

/// Los espacios de nombres del catálogo. Uno por fichero de
/// `app/src/locales/<idioma>/`.
/// Guardián: `tests::every_catalog_file_is_embedded`.
macro_rules! catalogs {
    ($($namespace:literal),* $(,)?) => {
        // Sólo lo lee la prueba que compara esta lista con la carpeta.
        #[cfg(test)]
        const NAMESPACES: &[&str] = &[$($namespace),*];

        fn sources() -> Vec<(Locale, &'static str, &'static str)> {
            vec![
                $(
                    (Locale::En, $namespace, include_str!(concat!("../../app/src/locales/en/", $namespace, ".json"))),
                    (Locale::Es, $namespace, include_str!(concat!("../../app/src/locales/es/", $namespace, ".json"))),
                )*
            ]
        }
    };
}

catalogs!(
    "activity",
    "calls",
    "channel",
    "chat",
    "common",
    "diagnostics",
    "errors",
    "nav",
    "notifications",
    "org",
    "recordings",
    "work",
);

/// El espacio de una clave que no dice el suyo.
const DEFAULT_NAMESPACE: &str = "common";

/// Unos catálogos ya leídos: por idioma y espacio de nombres.
struct Catalogs(HashMap<(Locale, String), Value>);

impl Catalogs {
    fn parse<'a>(sources: impl IntoIterator<Item = (Locale, &'a str, &'a str)>) -> Self {
        Self(
            sources
                .into_iter()
                .map(|(locale, namespace, raw)| {
                    // Un catálogo que no se deja leer es un fallo de
                    // compilación tardío: lo atrapa cualquier prueba que pida
                    // un texto.
                    let parsed = serde_json::from_str(raw)
                        .unwrap_or_else(|error| panic!("catálogo {namespace} ({}): {error}", locale.tag()));
                    ((locale, namespace.to_string()), parsed)
                })
                .collect(),
        )
    }

    /// La frase de una clave en un idioma, si está.
    fn lookup(&self, locale: Locale, key: &str) -> Option<&str> {
        let (namespace, path) = key.split_once(':').unwrap_or((DEFAULT_NAMESPACE, key));
        let mut node = self.0.get(&(locale, namespace.to_string()))?;
        for part in path.split('.') {
            node = node.get(part)?;
        }
        node.as_str()
    }

    /// Si falta en el idioma pedido sale en inglés, y si no existe sale la
    /// propia clave: una etiqueta rara es mejor que una pantalla caída.
    fn text(&self, locale: Locale, key: &str) -> String {
        self.lookup(locale, key).or_else(|| self.lookup(Locale::En, key)).unwrap_or(key).to_string()
    }
}

/// Los de la app, incrustados.
fn embedded() -> &'static Catalogs {
    static CATALOGS: OnceLock<Catalogs> = OnceLock::new();
    CATALOGS.get_or_init(|| Catalogs::parse(sources()))
}

/// El texto de una clave (`common:misc.signIn`; sin espacio es `common`), en
/// ese idioma o, si ahí falta, en inglés.
/// Si el catálogo tiene esa clave (en inglés, que las tiene todas).
pub(crate) fn has_text(key: &str) -> bool {
    embedded().lookup(Locale::En, key).is_some()
}

pub fn text(locale: Locale, key: &str) -> String {
    embedded().text(locale, key)
}

/// Lo mismo, con sus huecos rellenos: `{{name}}` por el valor de `name`.
pub fn text_with(locale: Locale, key: &str, values: &[(&str, &str)]) -> String {
    let mut phrase = text(locale, key);
    for (name, value) in values {
        phrase = phrase.replace(&format!("{{{{{name}}}}}"), value);
    }
    phrase
}

/// Una frase que cambia con la cantidad: la clave lleva `_one` para una y
/// `_other` para las demás (cero incluido), que es lo que piden el inglés y
/// el castellano. El hueco `{{count}}` se rellena con el número.
pub fn text_count(locale: Locale, key: &str, count: i64) -> String {
    let form = if count == 1 { "one" } else { "other" };
    text_with(locale, &format!("{key}_{form}"), &[("count", &count.to_string())])
}

/// La misma frase sin su número, para pintarlos aparte: el «5» grande de un
/// día del calendario y «tareas» pequeño al lado. Sale del mensaje entero
/// para no duplicar en el catálogo el plural de cada cosa que se cuenta
/// (`unitOf` en `components/calendar/parts.tsx`).
pub fn text_count_unit(locale: Locale, key: &str, count: i64) -> String {
    let phrase = text_count(locale, key, count);
    let number = count.to_string();
    phrase.trim_start().strip_prefix(number.as_str()).map(|unit| unit.trim_start().to_string()).unwrap_or(phrase)
}

/// La clave de «sin conexión».
const OFFLINE_KEY: &str = "nav:panel.serverStatus.offline";

/// Un fallo, dicho para quien lo lee.
///
/// La etiqueta de un error del servidor es la clave del catálogo `errors`. Una
/// que el catálogo no conoce se queda con `fallback_key` si quien llama sabe
/// qué estaba haciendo (entrar, cambiar la contraseña), y si no con la frase
/// del servidor: uno más nuevo puede inventar un código que esta versión no
/// conoce, y su frase en inglés es mejor que la etiqueta cruda.
pub fn error_phrase(locale: Locale, error: &ApiError, fallback_key: Option<&str>) -> String {
    match error {
        ApiError::Network(_) => text(locale, OFFLINE_KEY),
        ApiError::Server { code, message, .. } => {
            let known = format!("errors:{code}");
            if !code.is_empty() && embedded().lookup(Locale::En, &known).is_some() {
                text(locale, &known)
            } else if let Some(key) = fallback_key {
                text(locale, key)
            } else {
                message.clone()
            }
        }
        other => fallback_key.map(|key| text(locale, key)).unwrap_or_else(|| other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{embedded, Catalogs, Locale, NAMESPACES};

    /// Todo fichero del catálogo de la app está incrustado, en los dos idiomas.
    ///
    /// Rotura que atrapa: añadir un espacio de nombres en la app y no aquí —
    /// sus claves saldrían crudas en pantalla, sin que nada más lo avisara.
    #[test]
    fn every_catalog_file_is_embedded() {
        for language in ["en", "es"] {
            let dir = format!("{}/../app/src/locales/{language}", env!("CARGO_MANIFEST_DIR"));
            let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
                .unwrap_or_else(|error| panic!("{dir}: {error}"))
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .filter_map(|name| name.strip_suffix(".json").map(str::to_string))
                .collect();
            on_disk.sort();
            assert_eq!(on_disk, NAMESPACES, "los ficheros de {language}");
        }
        assert_eq!(embedded().0.len(), NAMESPACES.len() * 2);
    }

    /// Una frase que falta en un idioma sale en inglés, y una clave que no
    /// existe sale ella misma.
    ///
    /// Con unos catálogos de mentira: los de verdad están completos en los dos
    /// idiomas, así que con ellos el camino de reserva no se pisa nunca.
    /// Roturas que atrapa: un hueco (o un pánico) donde falta una traducción.
    #[test]
    fn a_missing_phrase_falls_back_to_english_and_then_to_its_key() {
        let catalogs = Catalogs::parse([
            (Locale::En, "common", r#"{"a": {"both": "Both", "onlyEnglish": "English only"}}"#),
            (Locale::Es, "common", r#"{"a": {"both": "Los dos"}}"#),
        ]);
        assert_eq!(catalogs.text(Locale::Es, "common:a.both"), "Los dos");
        assert_eq!(catalogs.text(Locale::Es, "a.onlyEnglish"), "English only");
        assert_eq!(catalogs.text(Locale::Es, "common:a.nowhere"), "common:a.nowhere");
        assert_eq!(catalogs.text(Locale::Es, "other:a.both"), "other:a.both");
        // Un nodo que no es una frase tampoco es un texto.
        assert_eq!(catalogs.text(Locale::Es, "common:a"), "common:a");
    }
}
