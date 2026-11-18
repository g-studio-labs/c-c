//! Cómo se enseña el markdown que se lee, donde el pintor no llega solo.
//!
//! En la app de siempre un título de tercer nivel es una etiqueta de
//! sección: pequeño, en mayúsculas y apagado (`.md-body h3` en
//! `app/src/index.css`: `uppercase tracking-wide text-muted-foreground`). El
//! pintor de markdown de la app nativa no sabe poner mayúsculas ni cambiar
//! el color de un nivel, pero sí pinta apagado un título de sexto. Así que
//! antes de pintar, un `###` se le da como un `######` con su texto ya en
//! mayúsculas: sale como en la app de siempre. El texto que se guarda, se
//! edita y se copia no cambia: esto es sólo para pintarlo.
//!
//! Lo que se pierde: un `######` de verdad se ve igual que un `###` (sin las
//! mayúsculas). En la app de siempre los niveles cuarto a sexto no tienen
//! estilo propio y casi nadie los escribe.

/// El texto de un título, en mayúsculas, sin tocar a dónde apuntan sus
/// enlaces: `[ver](/tasks?doc=list:l1)` sigue llevando a `list:l1`.
fn shouted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        let (label, target) = rest.split_at(at + 2);
        out.push_str(&label.to_uppercase());
        // Hasta su paréntesis de cierre, tal cual. Sin él no era un enlace.
        match target.find(')') {
            Some(end) => {
                out.push_str(&target[..=end]);
                rest = &target[end + 1..];
            }
            None => {
                out.push_str(&target.to_uppercase());
                rest = "";
            }
        }
    }
    out.push_str(&rest.to_uppercase());
    out
}

/// El markdown tal como se le da al pintor: cada título de tercer nivel,
/// como una etiqueta de sección (ver arriba). Lo de dentro de un bloque de
/// código no se toca.
pub fn prose_for_painting(markdown: &str) -> String {
    let mut fence = "";
    let mut out = String::with_capacity(markdown.len() + 16);
    for (at, line) in markdown.split('\n').enumerate() {
        if at > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start_matches([' ', '\t']);
        let marker = ["```", "~~~"].into_iter().find(|marker| trimmed.starts_with(marker)).unwrap_or("");
        if fence.is_empty() && !marker.is_empty() {
            fence = marker;
        } else if !fence.is_empty() && marker == fence {
            fence = "";
        }
        match line.strip_prefix("### ") {
            Some(title) if fence.is_empty() && marker.is_empty() => {
                out.push_str("###### ");
                out.push_str(&shouted(title));
            }
            _ => out.push_str(line),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un título de tercer nivel se le da al pintor como etiqueta de
    /// sección: de sexto nivel y en mayúsculas. Lo demás, tal cual.
    ///
    /// Roturas que atrapa: un `###` pintado como un título más (en blanco y
    /// en minúsculas, que es lo que había); tocar un `##` o un `####`;
    /// poner en mayúsculas un comentario de un bloque de código, o un
    /// `###sin espacio`; comerse o añadir saltos de línea.
    #[test]
    fn a_third_level_heading_is_painted_as_a_section_label() {
        let text = "# Doc\n\n### Lo que no entra\n\ntexto ### no\n## Otra\n#### Detalle\n###pegado\n\n```sh\n### un comentario\n```\n\n### Final\n";
        let painted = "# Doc\n\n###### LO QUE NO ENTRA\n\ntexto ### no\n## Otra\n#### Detalle\n###pegado\n\n```sh\n### un comentario\n```\n\n###### FINAL\n";
        assert_eq!(prose_for_painting(text), painted);
        assert_eq!(prose_for_painting(""), "");
        assert_eq!(prose_for_painting("sin títulos"), "sin títulos");
    }

    /// Las mayúsculas son para lo que se lee, no para a dónde lleva un
    /// enlace del título; y las letras con acento también suben.
    ///
    /// Rotura que atrapa: un enlace dentro de un título de sección que deja
    /// de abrir su página porque su dirección quedó en mayúsculas.
    #[test]
    fn a_link_in_a_section_label_still_goes_where_it_pointed() {
        assert_eq!(
            prose_for_painting("### Ver [la página](/tasks?doc=list:l1&page=p2) y [más](https://example.com/a) aquí"),
            "###### VER [LA PÁGINA](/tasks?doc=list:l1&page=p2) Y [MÁS](https://example.com/a) AQUÍ"
        );
        assert_eq!(prose_for_painting("### Configuración ]( rota"), "###### CONFIGURACIÓN ]( ROTA");
    }
}
