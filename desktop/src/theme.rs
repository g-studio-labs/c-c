//! El aspecto de cac sobre el tema de GPUI Kit, claro y oscuro.
//!
//! Los colores no se copian: se leen del `index.css` de la app de siempre, de
//! sus bloques `:root` (claro) y `.dark`. Cambia un color allí y cambia aquí.

use std::collections::HashMap;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{px, rgb, App, Hsla, WindowAppearance};

use crate::backend::Backend;

const CSS: &str = include_str!("../../app/src/index.css");

/// Los colores que esta app usa del CSS. Los dos temas tienen que traerlos
/// todos. Guardián: `tests::both_themes_bring_every_colour`, que es quien la lee.
#[cfg(test)]
const TOKENS: [&str; 34] = [
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
    "hue-1",
    "hue-2",
    "hue-3",
    "hue-4",
    "hue-5",
    "hue-6",
    "hue-7",
    "hue-8",
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

/// El color de un espacio: el que alguien le puso (`#RRGGBB`) y, si no
/// tiene —lo normal—, el tono que le toca por su id, de los ocho `--hue-*`
/// del tema puesto. Hasta el 16-nov-2026 los que no tenían color propio
/// salían todos grises (jose, probando con sus espacios de verdad: el
/// servidor de mentira les ponía color a todos).
pub fn space_color(color: &str, hue: u32, cx: &App) -> Hsla {
    let own = color.strip_prefix('#').filter(|hex| hex.len() == 6).and_then(|hex| u32::from_str_radix(hex, 16).ok());
    use gpui_kit::component::ActiveTheme as _;
    let value = own.or_else(|| hue_value(cx.theme().mode.is_dark(), hue)).unwrap_or(0x888888);
    Hsla::from(rgb(value))
}

/// Un color del CSS de la app de siempre que el tema del kit no tiene
/// (`--status-open`, `--sunken`, `--raised`…), en el tema puesto. Uno que no
/// exista sale gris, que se ve y no rompe.
pub fn css_color(token: &str, cx: &App) -> Hsla {
    use gpui_kit::component::ActiveTheme as _;
    Hsla::from(rgb(css_value(cx.theme().mode.is_dark(), token).unwrap_or(0x888888)))
}

/// Las paletas se leen del CSS una vez.
fn css_value(dark: bool, token: &str) -> Option<u32> {
    static PALETTES: std::sync::OnceLock<[HashMap<String, u32>; 2]> = std::sync::OnceLock::new();
    let palettes = PALETTES.get_or_init(|| [palette(CSS, selector(ThemeMode::Light)), palette(CSS, selector(ThemeMode::Dark))]);
    palettes[usize::from(dark)].get(token).copied()
}

/// El `--hue-N` de ese tema.
fn hue_value(dark: bool, hue: u32) -> Option<u32> {
    css_value(dark, &format!("hue-{}", hue % cac_core::HUES + 1))
}

/// Pone el tema que eligió quien mira (el core lo recuerda y sabe qué hacer
/// con «el del sistema»). Al arrancar y cada vez que se elige otro.
pub fn apply_preference(cx: &mut App) {
    let system_is_dark = matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
    let preference = Backend::core(cx).theme();
    let mode = if cac_core::is_dark(preference, Some(system_is_dark)) { ThemeMode::Dark } else { ThemeMode::Light };
    apply(mode, cx);
}

/// Viste el tema de GPUI Kit de cac.
pub fn apply(mode: ThemeMode, cx: &mut App) {
    install_fonts(cx);
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

        // GPUI Kit le pone este tamaño al `rem` de la ventana (`Root`), y de
        // él salen todos los textos, también los de sus componentes. Ver
        // `BASE_FONT_SIZE`.
        theme.font_size = px(BASE_FONT_SIZE);
        theme.font_family = FONT_FAMILY.into();
        theme.mono_font_family = MONO_FONT_FAMILY.into();
        theme.radius = px(8.);
    });
}

/// Markdown para leer, al tamaño del cuerpo. Suelto, `TextView` hereda el
/// `rem` entero y sale más grande que el texto de alrededor.
///
/// Adónde llevan sus enlaces lo dice quien lo pone: los de una tarea pueden
/// ser a un fichero suyo, que pide sesión y no se le da al navegador.
pub fn prose_with_links(
    id: impl Into<gpui_kit::SharedString>,
    text: impl Into<gpui_kit::SharedString>,
    follow: impl Fn(&str, &mut gpui_kit::Window, &mut gpui_kit::App) + Send + Sync + 'static,
) -> gpui_kit::Div {
    use gpui_kit::{ParentElement, Styled};
    gpui_kit::div().text_sm().child(
        gpui_kit::component::text::TextView::markdown(id.into(), text.into()).on_link_click(move |url, _, window, cx| follow(url, window, cx)),
    )
}

/// Si pulsar ese enlace lleva a algún sitio. Una mención no: es el nombre de
/// alguien escrito como enlace, y dárselo al sistema para que lo abra acaba
/// en un aviso de «no sé abrir cac:».
pub fn leads_somewhere(url: &str) -> bool {
    cac_core::mentioned_user(url).is_none()
}

/// Un identificador que se copia de un clic, enseñado corto: la pastilla de
/// `CopyId.tsx` (`rounded bg-muted/60 px-1.5 py-0.5 font-mono text-xs`, icono
/// `size-2.5`). Aquí sólo se pinta; quien la pone le da el clic. `copied` es
/// que se acaba de copiar: lo dice con una marca en vez del icono de copiar.
pub fn id_chip(id: impl Into<gpui_kit::ElementId>, shown: impl Into<gpui_kit::SharedString>, copied: bool, cx: &App) -> gpui_kit::Stateful<gpui_kit::Div> {
    use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
    use gpui_kit::{InteractiveElement as _, ParentElement as _, Styled as _};
    let theme = cx.theme();
    let (muted, foreground, chip, mono) = (theme.muted_foreground, theme.foreground, theme.muted, theme.mono_font_family.clone());
    gpui_kit::div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded(px(4.))
        .bg(chip.opacity(0.6))
        .font_family(mono)
        .text_xs()
        .text_color(muted)
        .cursor_pointer()
        .hover(move |style| style.bg(chip).text_color(foreground))
        .child(if copied { Icon::new(IconName::Check).size_2p5() } else { Icon::new(gpui_kit::assets::IconName::Copy).size_2p5() })
        .child(shown.into())
}

/// La letra de la app de siempre (`'Inter Variable'` en `index.css`).
const FONT_FAMILY: &str = "Inter";

/// La letra de ancho fijo de la app de siempre (`--font-mono` en
/// `index.css`): la de los identificadores, las referencias y el código.
const MONO_FONT_FAMILY: &str = "Iosevka Term Slab";

/// Marca de que esta app ya tiene sus tipografías cargadas.
struct FontsInstalled;
impl gpui_kit::Global for FontsInstalled {}

/// Carga Inter en la app, una vez. Va dentro del binario y no se le pide al
/// sistema: con la del sistema, en una máquina sin Inter el texto salía en
/// otra letra y sin negritas (jose, 17-nov-2026: el título de una tarea y lo
/// marcado en negrita en una descripción salían en peso normal). Son los
/// cuatro pesos fijos que usa la app y la cursiva: el fichero variable de la
/// app de siempre no vale aquí, que el motor de texto elige cara por peso.
///
/// Y la de ancho fijo, Iosevka Term Slab, en normal y negrita: sin ella los
/// identificadores y el código salían con la que tuviera cada máquina. Son
/// los TTF de la release oficial (be5invis/Iosevka v34.9.1,
/// `PkgTTF-IosevkaTermSlab`), recortados a latín como los `woff2` de la app
/// de siempre: enteros pesan 11 MB cada uno.
fn install_fonts(cx: &mut App) {
    if cx.has_global::<FontsInstalled>() {
        return;
    }
    cx.set_global(FontsInstalled);
    let faces: [&'static [u8]; 7] = [
        include_bytes!("../assets/fonts/Inter-Regular.ttf"),
        include_bytes!("../assets/fonts/Inter-Medium.ttf"),
        include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
        include_bytes!("../assets/fonts/Inter-Bold.ttf"),
        include_bytes!("../assets/fonts/Inter-Italic.ttf"),
        include_bytes!("../assets/fonts/IosevkaTermSlab-Regular.ttf"),
        include_bytes!("../assets/fonts/IosevkaTermSlab-Bold.ttf"),
    ];
    // Si no se pudieran cargar, el texto sale con la letra del sistema: se
    // lee igual.
    let _ = cx.text_system().add_fonts(faces.into_iter().map(std::borrow::Cow::Borrowed).collect());
}

/// El cuerpo de texto de la app de siempre: su `text-sm`, 14 px. Hasta el
/// 17-nov-2026 fue 13, el de macOS (jose lo eligió el 16); al poner las dos
/// apps una junto a otra todo salía más pequeño en ésta y ninguna página
/// cuadraba, y jose lo subió: «sí, sube el texto a 14».
pub const BODY_SIZE: f32 = 14.;

/// El `rem` con el que el texto normal (`text_sm`, 0.875rem) sale a ese
/// cuerpo: 16, el del navegador. De él salen solos los demás (`text_xs` a
/// 12, `text_lg` a 18), y los botones, campos y menús de GPUI Kit, que se
/// miden en `rem`, quedan a la misma escala que las clases de Tailwind de la
/// app de siempre sin tocarlos uno por uno.
pub const BASE_FONT_SIZE: f32 = BODY_SIZE / 0.875;

#[cfg(test)]
mod tests {
    use super::{apply, palette, selector, CSS, TOKENS};
    use gpui_kit::component::{ActiveTheme, ThemeMode};
    use gpui_kit::{rgb, Hsla, TestAppContext};

    /// Un enlace de verdad se abre; una mención no va a ningún sitio.
    ///
    /// Roturas que atrapa: pulsar el nombre de alguien y que el sistema
    /// avise de que no sabe abrir `cac:`; dejar de abrir los enlaces.
    #[test]
    fn a_mention_is_not_a_place_to_go() {
        assert!(super::leads_somewhere("https://cac.guz-studio.dev/docs"));
        assert!(!super::leads_somewhere("cac:user/11111111-1111-4111-8111-111111111111"));
    }

    /// Un espacio con color propio sale con él; sin él, con el tono de su
    /// id, que cambia con el tema; nunca gris.
    ///
    /// Roturas que atrapa: todos los espacios sin color pintados del mismo
    /// gris; ignorar el color que alguien eligió; el tono del tema claro en
    /// el oscuro (no contrasta).
    #[gpui_kit::test]
    fn a_space_without_a_colour_gets_its_hue(cx: &mut TestAppContext) {
        let (light, dark) = (palette(CSS, selector(ThemeMode::Light)), palette(CSS, selector(ThemeMode::Dark)));
        cx.update(|cx| {
            gpui_kit::component::init(cx);
            apply(ThemeMode::Dark, cx);
            assert_eq!(super::space_color("#20D9E8", 3, cx), Hsla::from(rgb(0x20D9E8)));
            assert_eq!(super::space_color("", 0, cx), Hsla::from(rgb(dark["hue-1"])));
            assert_eq!(super::space_color("", 7, cx), Hsla::from(rgb(dark["hue-8"])));
            assert_eq!(super::space_color("no es un color", 3, cx), Hsla::from(rgb(dark["hue-4"])));
            apply(ThemeMode::Light, cx);
            assert_eq!(super::space_color("", 0, cx), Hsla::from(rgb(light["hue-1"])));
            assert_ne!(light["hue-1"], dark["hue-1"]);
        });
    }

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
    /// La app trae su letra, con sus negritas, y no depende de la que haya
    /// en la máquina.
    ///
    /// Rotura que atrapa: dejar la letra del sistema. Que cada peso tenga
    /// su cara no se puede preguntar aquí (el motor de texto de las pruebas
    /// no carga tipografías): eso se mira en una captura.
    #[gpui_kit::test]
    fn the_app_brings_its_own_typeface_with_its_bold(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::component::ActiveTheme;
        cx.update(|cx| {
            gpui_kit::init(cx);
            super::apply(gpui_kit::component::ThemeMode::Dark, cx);
            assert_eq!(cx.theme().font_family.as_ref(), "Inter");
            assert_eq!(cx.theme().mono_font_family.as_ref(), "Iosevka Term Slab");
        });
    }

    /// El texto normal mide lo que en la app de siempre: 14 de cuerpo, 12 de
    /// secundario y 18 de título.
    #[gpui_kit::test]
    fn normal_text_is_as_big_as_in_the_other_app(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::component::ActiveTheme;
        cx.update(|cx| {
            gpui_kit::init(cx);
            super::apply(gpui_kit::component::ThemeMode::Dark, cx);
            let rem = f32::from(cx.theme().font_size);
            let sizes = [0.875, 0.75, 1.125].map(|rems: f32| (rem * rems).round());
            assert_eq!(sizes, [14., 12., 18.]);
        });
    }

}
