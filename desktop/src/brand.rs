//! La marca: los tres racks cruzados por la línea roja, y la marca con el
//! nombre al lado.
//!
//! Es el mismo dibujo que `app/src/components/brand/Brand.tsx` y que el
//! icono de la app (`app/src-tauri/icons/icon.svg`), con sus mismas medidas.
//! Va pintado con rectángulos y no cargado como SVG: GPUI pinta un SVG de un
//! solo color, y la línea roja es lo único de la marca que no cambia con el
//! tema.

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

/// La caja del dibujo original (`viewBox="202 220 620 560"`).
const ORIGIN: (f32, f32) = (202., 220.);
const BOX: (f32, f32) = (620., 560.);

/// El rojo de la línea de telemetría, muestreado del arte original.
const ACCENT: u32 = 0xDC2921;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Part {
    Rack,
    /// Una rejilla: un hueco por el que se ve la superficie de detrás.
    Vent,
    Accent,
}

/// Cada rectángulo de la marca: qué es, dónde va (en las medidas del dibujo
/// original) y cuánto se redondea. En el orden en que se pinta.
fn shapes() -> Vec<(Part, [f32; 4], f32)> {
    let mut all = vec![
        (Part::Rack, [202., 420., 170., 360.], 24.),
        (Part::Rack, [427., 320., 170., 460.], 24.),
        (Part::Rack, [652., 220., 170., 560.], 24.),
    ];
    for (x, rows) in [(234., [464., 520., 652., 708.]), (459., [364., 420., 652., 708.]), (684., [264., 320., 652., 708.])] {
        all.extend(rows.map(|y| (Part::Vent, [x, y, 106., 28.], 14.)));
    }
    all.push((Part::Accent, [230., 575., 564., 30.], 0.));
    all.extend([247., 472., 697.].map(|x| (Part::Accent, [x, 550., 80., 80.], 0.)));
    all
}

/// La marca sola, de ese alto. `surface` es el color de lo que tiene detrás:
/// con él se pintan las rejillas, que son huecos.
pub fn brand_mark(height: f32, surface: Hsla, cx: &App) -> Div {
    let scale = height / BOX.1;
    let ink = cx.theme().foreground;
    // Por debajo de este alto una rejilla mide menos de un píxel y sólo
    // ensucia: los racks van lisos, como en el icono a 32 px.
    let with_vents = 28. * scale >= 1.5;
    div().relative().flex_none().w(px(BOX.0 * scale)).h(px(height)).children(
        shapes().into_iter().filter(|(part, ..)| with_vents || *part != Part::Vent).map(|(part, [x, y, w, h], radius)| {
            let color = match part {
                Part::Rack => ink,
                Part::Vent => surface,
                Part::Accent => rgb(ACCENT).into(),
            };
            div()
                .absolute()
                .left(px((x - ORIGIN.0) * scale))
                .top(px((y - ORIGIN.1) * scale))
                .w(px(w * scale))
                .h(px(h * scale))
                .rounded(px(radius * scale))
                .bg(color)
        }),
    )
}

/// La marca con su nombre: «COMMAND» y debajo, más pequeño, «AND CONTROL».
pub fn brand(surface: Hsla, cx: &App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    div()
        .id("brand")
        .test_support()
        .flex()
        .items_center()
        .gap_3()
        .child(brand_mark(38., surface, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xl().line_height(relative(1.)).font_weight(FontWeight::EXTRA_BOLD).child("COMMAND"))
                .child(
                    div().flex().gap_2().text_xs().line_height(relative(1.)).font_weight(FontWeight::MEDIUM).text_color(muted).child(
                        div().opacity(0.55).child("A N D"),
                    ).child("C O N T R O L"),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::{shapes, Part, BOX, ORIGIN};

    /// La marca es la del icono de la app: tres racks que suben, cuatro
    /// rejillas en cada uno y la línea roja con sus tres nodos, todo dentro
    /// de su caja.
    ///
    /// Roturas que atrapa: un rectángulo fuera de la caja (se pintaría sobre
    /// lo de al lado); racks que no suben (se pierde la silueta); una rejilla
    /// fuera de su rack, que sería una raya suelta; la línea roja pintada
    /// debajo de los racks.
    #[test]
    fn the_mark_is_the_one_of_the_app_icon() {
        let all = shapes();
        let of = |part: Part| all.iter().filter(|(kind, ..)| *kind == part).map(|(_, at, _)| *at).collect::<Vec<_>>();
        let (racks, vents, accent) = (of(Part::Rack), of(Part::Vent), of(Part::Accent));
        assert_eq!((racks.len(), vents.len(), accent.len()), (3, 12, 4));

        for (_, [x, y, w, h], _) in &all {
            assert!(*x >= ORIGIN.0 && *y >= ORIGIN.1 && x + w <= ORIGIN.0 + BOX.0 && y + h <= ORIGIN.1 + BOX.1, "{x},{y} se sale");
        }
        assert!(racks[0][3] < racks[1][3] && racks[1][3] < racks[2][3], "los racks no suben");
        for [x, y, w, h] in &vents {
            let inside = racks.iter().filter(|[rx, ry, rw, rh]| x >= rx && y >= ry && x + w <= rx + rw && y + h <= ry + rh).count();
            assert_eq!(inside, 1, "la rejilla en {x},{y} no está en un rack");
        }
        let last_rack = all.iter().rposition(|(part, ..)| *part != Part::Accent).unwrap();
        let first_accent = all.iter().position(|(part, ..)| *part == Part::Accent).unwrap();
        assert!(first_accent > last_rack, "la línea roja queda debajo");
    }
}
