//! Los enlaces de un texto de pedido: direcciones web, rutas de ficheros y
//! rutas de la app (`app/src/lib/linkify.ts`, `requestSegments`).
//!
//! Los agentes escriben a mano en campos de texto plano («abre
//! /home/rv/x.sql», «pega https://…/sql/new»). Esto los parte en trozos para
//! que la pantalla pinte cada uno como lo que es. Sólo detecta; qué se hace
//! con cada uno lo decide quien pinta.

/// Un trozo de un texto, tal como se escribió: juntos devuelven el original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSegment {
    Text(String),
    /// Una dirección `http(s)://` de fuera de la app.
    Url(String),
    /// Un fichero o carpeta de esta máquina. `shown` es lo escrito; `path`, a
    /// dónde apunta (un `file://` ya sin esquema ni escapes). La `~` se
    /// expande al abrir.
    Path { shown: String, path: String },
    /// Una pantalla de la app: `/tasks?task=…`, venga escrita como
    /// `/app/tasks?task=…` o como `https://cac.guz-studio.dev/app/…`.
    AppRoute { shown: String, route: String },
}

const APP_HOSTS: [&str; 1] = ["cac.guz-studio.dev"];
// Una ruta `/app/<algo>` sólo es de la app si `<algo>` es una pantalla suya:
// `/app/build/out.log` es un fichero.
const APP_SCREENS: [&str; 8] = ["tasks", "requests", "chat", "dm", "docs", "notes", "inbox", "reports"];

fn is_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`' | '\'')
}

/// Quita lo que cierra una frase pero no es del enlace: puntuación final, y un
/// `)` sólo si no abre otro dentro (`/wiki/Foo_(bar)` se queda entero).
fn trim_link(raw: &str) -> &str {
    let mut end = raw.len();
    loop {
        let piece = &raw[..end];
        let Some(last) = piece.chars().next_back() else { return piece };
        let unbalanced = last == ')' && piece.matches('(').count() < piece.matches(')').count();
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '»' | ']' | '}') || unbalanced {
            end -= last.len_utf8();
        } else {
            return piece;
        }
    }
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let byte = (bytes[at] == b'%').then(|| text.get(at + 1..at + 3)).flatten().and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match byte {
            Some(b) => {
                out.push(b);
                at += 3;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_screen(route: &str) -> bool {
    let first = route.trim_start_matches('/').split(['/', '?', '#']).next().unwrap_or_default();
    APP_SCREENS.contains(&first)
}

/// `Some(ruta)` si una dirección `http(s)` de la propia cac lleva a una pantalla.
fn app_route_of_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    if !APP_HOSTS.contains(&host) {
        return None;
    }
    let route = format!("/{path}").strip_prefix("/app").map(str::to_string)?;
    (route.starts_with('/') && is_screen(&route)).then_some(route)
}

fn is_windows_path(token: &str) -> bool {
    let mut chars = token.chars();
    matches!((chars.next(), chars.next(), chars.next()), (Some(d), Some(':'), Some('\\' | '/')) if d.is_ascii_alphabetic())
}

/// Qué es `token` (ya cortado en blanco y puntuación) si empieza donde un
/// enlace puede empezar.
fn classify(token: &str) -> Option<LinkSegment> {
    if token.starts_with("http://") || token.starts_with("https://") {
        if token.split("://").nth(1).is_none_or(str::is_empty) {
            return None;
        }
        return Some(match app_route_of_url(token) {
            Some(route) => LinkSegment::AppRoute { shown: token.into(), route },
            None => LinkSegment::Url(token.into()),
        });
    }
    if let Some(rest) = token.strip_prefix("file://") {
        // `file://host/x` no es de esta máquina; `file:///x` y `file://localhost/x` sí.
        let path = rest.strip_prefix("localhost").unwrap_or(rest);
        return path.starts_with('/').then(|| LinkSegment::Path { shown: token.into(), path: percent_decoded(path) });
    }
    if let Some(rest) = token.strip_prefix("/app/") {
        if is_screen(rest) {
            return Some(LinkSegment::AppRoute { shown: token.into(), route: token["/app".len()..].into() });
        }
    }
    let rooted = token.starts_with('/') && !token.starts_with("//") && token.len() > 1;
    if rooted || token.starts_with("~/") && token.len() > 2 || is_windows_path(token) {
        return Some(LinkSegment::Path { shown: token.into(), path: token.into() });
    }
    None
}

/// Parte `text` en trozos de texto, direcciones, rutas y pantallas de la app.
pub fn link_segments(text: &str) -> Vec<LinkSegment> {
    let mut out: Vec<LinkSegment> = vec![];
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut Vec<LinkSegment>| {
        if !plain.is_empty() {
            out.push(LinkSegment::Text(std::mem::take(plain)));
        }
    };
    let mut previous: Option<char> = None;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        // Un enlace empieza tras un blanco, comilla, paréntesis o el principio:
        // `y/o` o `a/b/c` dentro de una palabra no son rutas.
        let may_start = previous.is_none_or(|p| p.is_whitespace() || matches!(p, '`' | '"' | '\'' | '(' | '[' | '<' | '=' | '«'));
        if may_start {
            let len = rest.find(is_end).unwrap_or(rest.len());
            let token = trim_link(&rest[..len]);
            if let Some(segment) = classify(token) {
                flush(&mut plain, &mut out);
                previous = token.chars().next_back();
                rest = &rest[token.len()..];
                out.push(segment);
                continue;
            }
        }
        plain.push(c);
        previous = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    flush(&mut plain, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> LinkSegment {
        LinkSegment::Text(s.into())
    }
    fn path(s: &str) -> LinkSegment {
        LinkSegment::Path { shown: s.into(), path: s.into() }
    }

    /// Roturas que atrapa: incluir el punto, la coma o el paréntesis de cierre
    /// en la dirección; perder el `)` que sí es de la dirección; cortar la
    /// dirección en el `?`; devolver un texto que no suma el original.
    #[test]
    fn a_url_ends_where_the_sentence_does() {
        assert_eq!(link_segments("mira https://x.com/a."), vec![text("mira "), LinkSegment::Url("https://x.com/a".into()), text(".")]);
        assert_eq!(link_segments("(https://x.com/a), luego"), vec![text("("), LinkSegment::Url("https://x.com/a".into()), text("), luego")]);
        assert_eq!(link_segments("https://es.wikipedia.org/wiki/Foo_(bar)"), vec![LinkSegment::Url("https://es.wikipedia.org/wiki/Foo_(bar)".into())]);
        assert_eq!(
            link_segments("https://supabase.com/dashboard/project/abc/sql/new?x=1&y=2"),
            vec![LinkSegment::Url("https://supabase.com/dashboard/project/abc/sql/new?x=1&y=2".into())]
        );
    }

    /// Roturas que atrapa: una ruta que se come lo que sigue al espacio; la
    /// puntuación final dentro de la ruta; no ver la ruta tras el «abre»; las
    /// comillas o los backticks dentro de la ruta.
    #[test]
    fn a_path_ends_at_a_space_and_at_punctuation() {
        assert_eq!(
            link_segments("abre /home/rv/projects/respaldos/2026-11-25/ris.sql y sigue"),
            vec![text("abre "), path("/home/rv/projects/respaldos/2026-11-25/ris.sql"), text(" y sigue")]
        );
        assert_eq!(link_segments("está en /tmp/a.log."), vec![text("está en "), path("/tmp/a.log"), text(".")]);
        assert_eq!(link_segments("(ver /tmp/a.log), ok"), vec![text("(ver "), path("/tmp/a.log"), text("), ok")]);
        assert_eq!(link_segments("`/tmp/a b.log`"), vec![text("`"), path("/tmp/a"), text(" b.log`")]);
        assert_eq!(link_segments("`/tmp/a.log`"), vec![text("`"), path("/tmp/a.log"), text("`")]);
        assert_eq!(link_segments("\"/tmp/a.log\""), vec![text("\""), path("/tmp/a.log"), text("\"")]);
    }

    /// Roturas que atrapa: ignorar `~/`, ver ruta en una `~` suelta, no
    /// reconocer `C:\…` ni `C:/…`.
    #[test]
    fn home_and_windows_paths_are_paths() {
        assert_eq!(link_segments("usa ~/notas/a.md"), vec![text("usa "), path("~/notas/a.md")]);
        assert_eq!(link_segments("el ~ de casa"), vec![text("el ~ de casa")]);
        assert_eq!(link_segments(r"abre C:\Users\ana\a.txt ya"), vec![text("abre "), path(r"C:\Users\ana\a.txt"), text(" ya")]);
        assert_eq!(link_segments("C:/Users/ana"), vec![path("C:/Users/ana")]);
    }

    /// Roturas que atrapa: dejar el `file://` en la ruta que se abre; no
    /// decodificar `%20`; abrir como local un `file://otro-equipo/…`.
    #[test]
    fn a_file_link_points_at_its_path() {
        assert_eq!(
            link_segments("file:///home/rv/a%20b.txt"),
            vec![LinkSegment::Path { shown: "file:///home/rv/a%20b.txt".into(), path: "/home/rv/a b.txt".into() }]
        );
        assert_eq!(link_segments("file://otro/x"), vec![text("file://otro/x")]);
    }

    /// Roturas que atrapa: abrir en el navegador un enlace a la propia cac;
    /// tomar por pantalla una carpeta cualquiera de `/app/`; reclamar un
    /// dominio ajeno que termina igual.
    #[test]
    fn a_cac_link_is_a_screen_of_the_app() {
        let route = |shown: &str, route: &str| LinkSegment::AppRoute { shown: shown.into(), route: route.into() };
        assert_eq!(link_segments("ve a /app/tasks?task=t1."), vec![text("ve a "), route("/app/tasks?task=t1", "/tasks?task=t1"), text(".")]);
        assert_eq!(
            link_segments("https://cac.guz-studio.dev/app/requests?open=req-1"),
            vec![route("https://cac.guz-studio.dev/app/requests?open=req-1", "/requests?open=req-1")]
        );
        assert_eq!(link_segments("/app/build/out.log"), vec![path("/app/build/out.log")]);
        assert_eq!(link_segments("https://otro.com/app/tasks?task=1"), vec![LinkSegment::Url("https://otro.com/app/tasks?task=1".into())]);
    }

    /// Roturas que atrapa: ver ruta en `y/o` o `a/b`, en un cociente («km/h»),
    /// o en una fracción; perder texto al partir.
    #[test]
    fn a_slash_inside_a_word_is_not_a_path() {
        for plain in ["y/o", "usa a/b/c.txt", "100 km/h", "1/2 de", "http", "https://", "/", "// comentario", "tcg:/x"] {
            assert_eq!(link_segments(plain), vec![text(plain)], "{plain}");
        }
    }

    /// Roturas que atrapa: partir mal con acentos (cortar dentro de un
    /// carácter) o perder texto: juntos, los trozos son el original.
    #[test]
    fn the_segments_add_up_to_the_original() {
        let original = "Ábrelo «aquí» https://x.com/ñ, luego ~/año/ü.txt; y /tmp/é (listo)";
        let joined: String = link_segments(original)
            .into_iter()
            .map(|s| match s {
                LinkSegment::Text(t) | LinkSegment::Url(t) => t,
                LinkSegment::Path { shown, .. } | LinkSegment::AppRoute { shown, .. } => shown,
            })
            .collect();
        assert_eq!(joined, original);
    }
}
