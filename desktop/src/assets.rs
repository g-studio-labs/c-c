//! Los iconos de la app.
//!
//! GPUI no trae ninguno puesto: sin una fuente registrada al arrancar, un
//! icono no da error, simplemente no se pinta (así estuvo la app hasta el
//! 15-nov-2026, sin un solo chevrón). La fuente son los de los componentes de
//! GPUI Kit más los que esta app usa del catálogo de Lucide, y sólo ésos: el
//! catálogo entero son más de mil SVG dentro del binario.

use std::borrow::Cow;

#[cfg(test)]
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::{AssetSource, Result, SharedString};

// Los de Lucide que la app nombra y los componentes no traen. Al usar uno
// nuevo, se añade aquí y a `USED`. Guardián: `tests::every_icon_can_be_loaded`.
gpui_kit::assets::icon_assets!(Extra, [WifiOff, RefreshCw, LogOut, X, Paperclip, Image, Download, Trash, Pencil, Folder, ListChecks, FileText, ChevronsUpDown, Building2, Moon, Languages, PanelLeftClose, PanelLeftOpen, UserRound, KeyRound, Briefcase, MessagesSquare, Server]);

/// Los mismos, para la prueba que comprueba que se cargan.
#[cfg(test)]
const USED: [Lucide; 23] = [
    Lucide::WifiOff,
    Lucide::RefreshCw,
    Lucide::LogOut,
    Lucide::X,
    Lucide::Paperclip,
    Lucide::Image,
    Lucide::Download,
    Lucide::Trash,
    Lucide::Pencil,
    Lucide::Folder,
    Lucide::ListChecks,
    Lucide::FileText,
    Lucide::ChevronsUpDown,
    Lucide::Building2,
    Lucide::Moon,
    Lucide::Languages,
    Lucide::PanelLeftClose,
    Lucide::PanelLeftOpen,
    Lucide::UserRound,
    Lucide::KeyRound,
    Lucide::Briefcase,
    Lucide::MessagesSquare,
    Lucide::Server,
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(found) = Extra.load(path)? {
            return Ok(Some(found));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all = Extra.list(path)?;
        all.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::{Assets, USED};
    use gpui_kit::assets::IconNamed;
    use gpui_kit::component::IconName;
    use gpui_kit::AssetSource;

    /// Todo icono que la app nombra está en su fuente.
    ///
    /// Un icono que falta no falla: no se pinta. Roturas que atrapa: nombrar
    /// un icono de Lucide sin incrustarlo; quedarse sin los de los
    /// componentes (el chevrón de un desplegable, el ojo de la contraseña).
    #[test]
    fn every_icon_can_be_loaded() {
        let mut paths: Vec<String> = USED.into_iter().map(|icon| icon.path().to_string()).collect();
        for icon in [IconName::Inbox, IconName::ChevronDown, IconName::ChevronRight, IconName::Check, IconName::Eye] {
            paths.push(icon.path().to_string());
        }
        for path in paths {
            let svg = Assets.load(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
            assert!(svg.is_some_and(|bytes| !bytes.is_empty()), "{path} no está en la fuente de iconos");
        }
    }
    /// `RefreshCw` → `refresh-cw`: como se llama el fichero de un icono.
    fn kebab(name: &str) -> String {
        let mut out = String::new();
        for c in name.chars() {
            if (c.is_uppercase() || c.is_ascii_digit()) && !out.is_empty() {
                out.push('-');
            }
            out.extend(c.to_lowercase());
        }
        out
    }

    /// Todo icono de Lucide nombrado en el código de la app está incrustado.
    ///
    /// La lista de arriba se mantiene a mano, y olvidarla no falla: el icono
    /// no se pinta (así salieron sin clip ni papelera los adjuntos, el
    /// 15-nov-2026). Rotura que atrapa: usar un icono nuevo sin añadirlo.
    #[test]
    fn every_icon_named_in_the_code_is_embedded() {
        // La regla del nombre, comprobada con los que ya se sabe que están.
        for icon in USED {
            let name = format!("{icon:?}");
            assert_eq!(icon.path().to_string(), format!("icons/{}.svg", kebab(&name)), "{name}");
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut seen = 0;
        for file in std::fs::read_dir(src).unwrap() {
            let path = file.unwrap().path();
            let code = std::fs::read_to_string(&path).unwrap();
            for piece in code.split("Lucide::").skip(1) {
                let name: String = piece.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                if name.is_empty() {
                    continue;
                }
                seen += 1;
                let svg = Assets.load(&format!("icons/{}.svg", kebab(&name))).ok().flatten();
                assert!(svg.is_some_and(|bytes| !bytes.is_empty()), "{name} ({path:?}) no está incrustado");
            }
        }
        assert!(seen > 8, "no encontró los iconos del código");
    }

}
