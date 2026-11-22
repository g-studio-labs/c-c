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
use gpui_kit::{img, AnyElement, AssetSource, IntoElement, Pixels, Result, SharedString, Styled};

// Los de Lucide que la app nombra y los componentes no traen. Al usar uno
// nuevo, se añade aquí y a `USED`. Guardián: `tests::every_icon_can_be_loaded`.
gpui_kit::assets::icon_assets!(Extra, [WifiOff, RefreshCw, LogOut, X, Paperclip, Image, Download, Trash, Pencil, Folder, ListChecks, FileText, ChevronsUpDown, Building2, Moon, Languages, PanelLeftClose, PanelLeftOpen, UserRound, KeyRound, Briefcase, MessagesSquare, Server, NotebookPen, Lock, List, SquareKanban, CalendarDays, PanelLeft, ChevronLeft, GripVertical, Plus, Eye, EyeOff, CircleAlert, Flag, Tag, Users, Calendar, FolderInput, Link2, Bot, Gavel, Send, Copy, Share2, ListTree, SquareCheck, Bold, Italic, Code, Heading2, ListOrdered, Quote, Table, GitPullRequest, Activity, FolderPlus, House, Puzzle, ClipboardList, RotateCcwClock, Clock, CircleCheck, BellRing, Pin, Megaphone, Hash, UserPlus, Check, CheckCheck, MessageSquare, CornerUpLeft, ExternalLink, ChevronRight, HeartPlus, Bell, BellOff, AtSign, Zap, CalendarClock, Workflow, Rocket, Archive, Info, Volume2, VolumeX, Mic, MicOff, Headphones, HeadphoneOff, PhoneOff, Minimize2, SlidersHorizontal, Phone, AudioLines, RotateCcw, Maximize2, Video, MonitorUp, VideoOff, Monitor, AppWindow, Wrench, Terminal, SquareTerminal, ArrowUpDown, ShieldCheck, ShieldAlert, Boxes, Database, LayoutGrid, Undo2, ArrowLeft, FolderOpen, Play, Square]);

/// Los mismos, para la prueba que comprueba que se cargan.
#[cfg(test)]
const USED: [Lucide; 113] = [
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
    Lucide::NotebookPen,
    Lucide::Lock,
    Lucide::List,
    Lucide::SquareKanban,
    Lucide::CalendarDays,
    Lucide::PanelLeft,
    Lucide::ChevronLeft,
    Lucide::GripVertical,
    Lucide::Plus,
    Lucide::Eye,
    Lucide::EyeOff,
    Lucide::CircleAlert,
    Lucide::Flag,
    Lucide::Tag,
    Lucide::Users,
    Lucide::Calendar,
    Lucide::FolderInput,
    Lucide::Link2,
    Lucide::Bot,
    Lucide::Gavel,
    Lucide::Send,
    Lucide::Copy,
    Lucide::Share2,
    Lucide::ListTree,
    Lucide::SquareCheck,
    Lucide::Bold,
    Lucide::Italic,
    Lucide::Code,
    Lucide::Heading2,
    Lucide::ListOrdered,
    Lucide::Quote,
    Lucide::Table,
    Lucide::GitPullRequest,
    Lucide::Activity,
    Lucide::FolderPlus,
    Lucide::House,
    Lucide::Puzzle,
    Lucide::ClipboardList,
    Lucide::RotateCcwClock,
    Lucide::Clock,
    Lucide::CircleCheck,
    Lucide::BellRing,
    Lucide::Pin,
    Lucide::Megaphone,
    Lucide::Hash,
    Lucide::UserPlus,
    Lucide::Check,
    Lucide::CheckCheck,
    Lucide::MessageSquare,
    Lucide::Bell,
    Lucide::BellOff,
    Lucide::AtSign,
    Lucide::Zap,
    Lucide::CalendarClock,
    Lucide::Workflow,
    Lucide::Rocket,
    Lucide::Archive,
    Lucide::Info,
    Lucide::Volume2,
    Lucide::VolumeX,
    Lucide::Mic,
    Lucide::MicOff,
    Lucide::Headphones,
    Lucide::HeadphoneOff,
    Lucide::PhoneOff,
    Lucide::Minimize2,
    Lucide::SlidersHorizontal,
    Lucide::Phone,
    Lucide::AudioLines,
    Lucide::RotateCcw,
    Lucide::Maximize2,
    Lucide::Video,
    Lucide::MonitorUp,
    Lucide::VideoOff,
    Lucide::Monitor,
    Lucide::AppWindow,
    Lucide::Wrench,
    Lucide::Terminal,
    Lucide::SquareTerminal,
    Lucide::ArrowUpDown,
    Lucide::ShieldCheck,
    Lucide::ShieldAlert,
    Lucide::Boxes,
    Lucide::Database,
    Lucide::LayoutGrid,
    Lucide::Undo2,
    Lucide::ArrowLeft,
    Lucide::FolderOpen,
    Lucide::Play,
    Lucide::Square,
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, png)) = EMOJI.iter().find(|(name, _)| path.strip_prefix("emoji/").and_then(|file| file.strip_suffix(".png")) == Some(*name)) {
            return Ok(Some(Cow::Borrowed(*png)));
        }
        if let Some(found) = Extra.load(path)? {
            return Ok(Some(found));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all = Extra.list(path)?;
        all.extend(gpui_kit::assets::Assets.list(path)?);
        if "emoji/".starts_with(path) {
            all.extend(EMOJI.iter().map(|(name, _)| SharedString::from(format!("emoji/{name}.png"))));
        }
        Ok(all)
    }
}

// Los emojis de las reacciones, como imágenes. Con la fuente no salen:
// GPUI no le hace caso a la preferencia de emoji de fontconfig, y el ❤️ y el
// 😂 que existen en blanco y negro en DejaVu se pintan grises (incidente
// ca5c0caa). Slack, Discord y Telegram hacen lo mismo: su propio juego en
// imágenes, igual en todas las máquinas.
//
// Son los de Noto Color Emoji (OFL, la licencia va al lado), sacados de la
// fuente a 48 px de alto —sobra para pintarlos a 16–20 px en una pantalla
// doble— y con paleta: unos 11 KB los ocho. Uno por cada `REACTION_EMOJIS`.
// Guardián: `tests::every_reaction_has_its_picture`.
const EMOJI: [(&str, &[u8]); 8] = [
    ("1f44d", include_bytes!("../assets/emoji/1f44d.png")),
    ("2764", include_bytes!("../assets/emoji/2764.png")),
    ("1f602", include_bytes!("../assets/emoji/1f602.png")),
    ("1f389", include_bytes!("../assets/emoji/1f389.png")),
    ("1f440", include_bytes!("../assets/emoji/1f440.png")),
    ("2705", include_bytes!("../assets/emoji/2705.png")),
    ("1f64f", include_bytes!("../assets/emoji/1f64f.png")),
    ("1f525", include_bytes!("../assets/emoji/1f525.png")),
];

/// Dónde está la imagen de un emoji, si la app la trae: los puntos de
/// código en hexadecimal, sin el selector de variación (U+FE0F), que sólo
/// pide «en color» y es lo que le falta al ❤️ de DejaVu.
pub fn emoji_picture(emoji: &str) -> Option<SharedString> {
    let name = emoji.chars().filter(|c| *c != '\u{fe0f}').map(|c| format!("{:x}", c as u32)).collect::<Vec<_>>().join("-");
    EMOJI.iter().any(|(known, _)| *known == name).then(|| SharedString::from(format!("emoji/{name}.png")))
}

/// Un emoji para pintar en un renglón: su imagen si la app la trae, si no
/// el carácter tal cual.
pub fn emoji(emoji: &str, size: Pixels) -> AnyElement {
    match emoji_picture(emoji) {
        Some(path) => img(path).size(size).flex_none().into_any_element(),
        None => SharedString::from(emoji.to_string()).into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::{emoji_picture, Assets, EMOJI, USED};
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
        // También los de las carpetas de un módulo (`platform/`): un icono
        // que sólo se nombra ahí dentro se escapaba.
        let mut pending = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        let mut files = Vec::new();
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    files.push(path);
                }
            }
        }
        let mut seen = 0;
        for path in files {
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

    /// Cada reacción del juego tiene su imagen, y es un PNG de verdad.
    ///
    /// Una que falte no falla: se pinta el carácter, y vuelve a salir gris.
    /// Roturas que atrapa: una reacción nueva en el juego sin su imagen;
    /// dejar el U+FE0F en el nombre (el ❤️ se quedaría sin la suya); una
    /// ruta que la fuente de la app no sirve.
    #[test]
    fn every_reaction_has_its_picture() {
        for reaction in cac_core::REACTION_EMOJIS {
            let path = emoji_picture(reaction).unwrap_or_else(|| panic!("{reaction} no tiene imagen"));
            let png = Assets.load(&path).unwrap().unwrap_or_else(|| panic!("{path} no se sirve"));
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{path} no es un PNG");
        }
        assert_eq!(EMOJI.len(), cac_core::REACTION_EMOJIS.len(), "sobra alguna imagen");
        assert_eq!(emoji_picture("🦀"), None, "uno que no está en el juego se pinta como texto");
    }

}
