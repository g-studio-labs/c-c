//! Un color estable para lo que tiene que distinguirse sin significar nada:
//! un espacio sin color propio, o una persona.
//!
//! Vino de `app/src/lib/person-color.ts`. Tiene que dar **el mismo** tono que
//! la app de siempre: quien conoce un espacio por «el naranja» lo busca por
//! ese color.

/// Cuántos tonos hay: los ocho `--hue-*` de `app/src/index.css`.
pub const HUES: u32 = 8;

/// Qué tono le toca a ese id, de 0 a `HUES - 1` (el `--hue-N` es el
/// siguiente). FNV-1a sobre las unidades UTF-16 del id, que es lo que
/// recorre `charCodeAt` en la app de siempre.
pub fn hue_index(id: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for unit in id.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash % HUES
}

#[cfg(test)]
mod tests {
    use super::hue_index;

    /// Los mismos tonos que da `hueIndex` en la app de siempre (calculados
    /// con su código, 16-nov-2026).
    ///
    /// Roturas que atrapa: otro reparto (cada espacio cambiaría de color al
    /// pasar de una app a otra); recorrer bytes y no unidades UTF-16, que
    /// sólo se nota con acentos y emojis; un tono fijo para todos.
    #[test]
    fn hues_are_the_ones_the_other_app_gives() {
        for (id, hue) in [
            ("sp-1", 0),
            ("a", 4),
            ("", 5),
            ("6ba77b95-2f0e-4c1c-9d3a-0a1b2c3d4e5f", 5),
            ("ñandú", 5),
            ("espacio 😀", 0),
            ("jose", 6),
        ] {
            assert_eq!(hue_index(id), hue, "{id:?}");
        }
    }
}
