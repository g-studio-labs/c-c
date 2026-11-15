//! El aspecto de cac sobre el tema de GPUI Kit, claro y oscuro.
//!
//! Los colores no se copian: se leen del `index.css` de la app de siempre, de
//! sus bloques `:root` (claro) y `.dark`. Cambia un color allí y cambia aquí.

use std::collections::HashMap;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{px, rgb, App, Hsla};

const CSS: &str = include_str!("../../app/src/index.css");

/// Los colores que esta app usa del CSS. Los dos temas tienen que traerlos
/// todos. Guardián: `tests::both_themes_bring_every_colour`, que es quien la lee.
#[cfg(test)]
const TOKENS: [&str; 26] = [
    "background",
    "foreground",
    "card",
    "popover",
    "popover-foreground",
    "primary",
    "primary-foreground",
    "secondary",
    "secondary-foreground",
    "muted",
    "muted-foreground",
    "accent",
    "accent-foreground",
    "destructive",
    "border",
    "input",
    "ring",
    "sidebar",
    "sidebar-foreground",
    "sidebar-primary",
    "sidebar-primary-foreground",
    "sidebar-accent",
    "sidebar-accent-foreground",
    "sidebar-border",
    "success",
    "warning",
];

/// Los colores de un bloque del CSS (`:root` o `.dark`): `--nombre: #RRGGBB;`.
///
/// Sólo los que son un hexadecimal de seis cifras; lo demás del bloque (un
/// radio, un color calculado) no es de aquí.
fn palette(css: &str, selector: &str) -> HashMap<String, u32> {
    let opening = format!("\n{selector} {{");
    let Some(start) = css.find(&opening) else { return HashMap::new() };
    let block = &css[start + opening.len()..];
    let block = &block[..block.find("\n}").unwrap_or(block.len())];
    block
        .lines()
        .filter_map(|line| {
            let (name, value) = line.trim().strip_prefix("--")?.split_once(':')?;
            let hex = value.trim().trim_end_matches(';').strip_prefix('#')?;
            (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten().map(|rgb| (name.to_string(), rgb))
        })
        .collect()
}

fn selector(mode: ThemeMode) -> &'static str {
    if mode.is_dark() {
        ".dark"
    } else {
        ":root"
    }
}

/// Viste el tema de GPUI Kit de cac.
pub fn apply(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
    let palette = palette(CSS, selector(mode));
    // Un color que faltara se queda con el de GPUI Kit: una pantalla con un
    // tono raro es mejor que una que no abre. Que no falte lo dice la prueba.
    let color = |name: &str, fallback: Hsla| palette.get(name).map(|value| Hsla::from(rgb(*value))).unwrap_or(fallback);

    // Por `Theme::update` y no tocando el global a mano: GPUI Kit guarda cada
    // color dos veces (el color y lo que pinta de verdad el componente), y
    // sólo este camino pone al día las dos. Tocando sólo el color, el botón
    // principal siguió saliendo blanco (medido, 15-nov-2026).
    Theme::update(cx, |theme| {
        let colors = &mut theme.colors;
        colors.background = color("background", colors.background);
        colors.foreground = color("foreground", colors.foreground);
        // La tarjeta de cac: GPUI Kit la llama «group box».
        colors.group_box = color("card", colors.group_box);
        colors.popover = color("popover", colors.popover);
        colors.popover_foreground = color("popover-foreground", colors.popover_foreground);

        let primary = color("primary", colors.primary);
        let primary_foreground = color("primary-foreground", colors.primary_foreground);
        colors.primary = primary;
        colors.primary_hover = primary.opacity(0.9);
        colors.primary_active = primary.opacity(0.8);
        colors.primary_foreground = primary_foreground;
        // El botón principal lleva sus propios colores en GPUI Kit; sin esto sale
        // en blanco y negro, no en el cian de cac.
        colors.button_primary = primary;
        colors.button_primary_hover = primary.opacity(0.9);
        colors.button_primary_active = primary.opacity(0.8);
        colors.button_primary_foreground = primary_foreground;
        colors.link = primary;
        colors.link_hover = primary.opacity(0.9);
        colors.link_active = primary.opacity(0.8);

        colors.secondary = color("secondary", colors.secondary);
        colors.secondary_foreground = color("secondary-foreground", colors.secondary_foreground);
        colors.muted = color("muted", colors.muted);
        colors.muted_foreground = color("muted-foreground", colors.muted_foreground);
        colors.accent = color("accent", colors.accent);
        colors.accent_foreground = color("accent-foreground", colors.accent_foreground);
        colors.list_hover = colors.accent;
        colors.danger = color("destructive", colors.danger);
        colors.success = color("success", colors.success);
        colors.warning = color("warning", colors.warning);
        colors.border = color("border", colors.border);
        colors.input = color("input", colors.input);
        colors.ring = color("ring", colors.ring);

        colors.sidebar = color("sidebar", colors.sidebar);
        colors.sidebar_foreground = color("sidebar-foreground", colors.sidebar_foreground);
        colors.sidebar_primary = color("sidebar-primary", colors.sidebar_primary);
        colors.sidebar_primary_foreground = color("sidebar-primary-foreground", colors.sidebar_primary_foreground);
        colors.sidebar_accent = color("sidebar-accent", colors.sidebar_accent);
        colors.sidebar_accent_foreground = color("sidebar-accent-foreground", colors.sidebar_accent_foreground);
        colors.sidebar_border = color("sidebar-border", colors.sidebar_border);

        // La base de cac son 14px, no los 16 de GPUI Kit.
        theme.font_size = px(14.);
        theme.radius = px(8.);
    });
}

#[cfg(test)]
mod tests {
    use super::{apply, palette, selector, CSS, TOKENS};
    use gpui_kit::component::{ActiveTheme, ThemeMode};
    use gpui_kit::{rgb, Hsla, TestAppContext};

    /// Los dos temas traen todos los colores que la app usa, y son los del CSS.
    ///
    /// Roturas que atrapa: leer el bloque que no es (el claro pintado con los
    /// colores del oscuro); un color que el CSS deja de traer y aquí se queda,
    /// callado, con el de GPUI Kit.
    #[test]
    fn both_themes_bring_every_colour() {
        let (light, dark) = (palette(CSS, selector(ThemeMode::Light)), palette(CSS, selector(ThemeMode::Dark)));
        for token in TOKENS {
            assert!(light.contains_key(token), "al tema claro le falta --{token}");
            assert!(dark.contains_key(token), "al tema oscuro le falta --{token}");
        }
        assert_eq!(dark["background"], 0x101013);
        assert_eq!(dark["primary"], 0x20D9E8);
        assert_eq!(light["background"], 0xffffff);
        assert_eq!(light["primary"], 0x00717D);
    }

    /// Lo que no es un color de seis cifras no entra, y un bloque que no
    /// existe no trae nada.
    ///
    /// Roturas que atrapa: tomar `--radius: 0.625rem` por un color; seguir
    /// leyendo más allá del cierre del bloque (los colores de otro tema).
    #[test]
    fn only_the_colours_of_the_block_asked_are_read() {
        let css = "\n:root {\n    --radius: 0.625rem;\n    --ink: #252525;\n    --short: #fff;\n}\n\n.dark {\n    --ink: #ECECEE;\n}\n";
        let light = palette(css, ":root");
        assert_eq!(light.len(), 1);
        assert_eq!(light["ink"], 0x252525);
        assert_eq!(palette(css, ".dark")["ink"], 0xECECEE);
        assert!(palette(css, ".sepia").is_empty());
    }

    /// El tema de la app queda con los colores de cac, en claro y en oscuro.
    ///
    /// Roturas que atrapa: un botón principal que no es del color de cac (así
    /// salió la primera vez: blanco); cambiar de tema y quedarse con los
    /// colores del otro.
    #[gpui_kit::test]
    fn the_theme_wears_cacs_colours(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply(ThemeMode::Dark, cx);
            let theme = cx.theme();
            assert_eq!(theme.background, Hsla::from(rgb(0x101013)));
            assert_eq!(theme.button_primary, Hsla::from(rgb(0x20D9E8)));
            // Lo que el botón pinta de verdad, que GPUI Kit guarda aparte.
            assert_eq!(theme.tokens.button_primary.color, Hsla::from(rgb(0x20D9E8)));
            assert_eq!(theme.tokens.sidebar.color, Hsla::from(rgb(0x0A0A0D)));
            assert_eq!(theme.button_primary_foreground, Hsla::from(rgb(0x0B0B0E)));
            assert_eq!(theme.group_box, Hsla::from(rgb(0x17171B)));
            assert_eq!(theme.danger, Hsla::from(rgb(0xFB7185)));
            assert_eq!(theme.sidebar, Hsla::from(rgb(0x0A0A0D)));

            apply(ThemeMode::Light, cx);
            let theme = cx.theme();
            assert_eq!(theme.background, Hsla::from(rgb(0xffffff)));
            assert_eq!(theme.button_primary, Hsla::from(rgb(0x00717D)));
            assert_eq!(theme.muted_foreground, Hsla::from(rgb(0x676767)));
        });
    }
}
