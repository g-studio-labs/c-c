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
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`' | '\'' | ']')
}

/// Dos separadores seguidos son una ruta de red (UNC) en Windows: mirar si
/// existe ya contacta al servidor. Se rechaza lo que se decodifica a eso, que
/// es como se colaba dentro de un `file://`.
fn is_network(path: &str) -> bool {
    let mut chars = path.chars();
    matches!((chars.next(), chars.next()), (Some('/' | '\\'), Some('/' | '\\')))
}

/// Quita lo que cierra una frase pero no es del enlace: puntuación final, y un
/// `)` sólo si no abre otro dentro (`/wiki/Foo_(bar)` se queda entero).
fn trim_link(raw: &str) -> &str {
    let mut end = raw.len();
    loop {
        let piece = &raw[..end];
        let Some(last) = piece.chars().next_back() else { return piece };
        let unbalanced = last == ')' && piece.matches('(').count() < piece.matches(')').count();
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '»' | ']' | '}' | '*') || unbalanced {
            end -= last.len_utf8();
        } else {
            return piece;
        }
    }
}

/// `%C3%B3` → `ó`. Un escape roto, o unos bytes que no son UTF-8, dejan el
/// texto como estaba entero (lo mismo que `decodeURIComponent` en la app de Tauri).
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'%' {
            out.push(bytes[at]);
            at += 1;
            continue;
        }
        let Some(byte) = text.get(at + 1..at + 3).and_then(|pair| u8::from_str_radix(pair, 16).ok()) else { return text.to_string() };
        out.push(byte);
        at += 3;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
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
        // `https:///x` no tiene anfitrión: no es una dirección.
        if token.split("://").nth(1).is_none_or(|rest| rest.is_empty() || rest.starts_with('/')) {
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
        let path = percent_decoded(path);
        return (path.starts_with('/') && !is_network(&path)).then(|| LinkSegment::Path { shown: token.into(), path });
    }
    if let Some(rest) = token.strip_prefix("/app/") {
        if is_screen(rest) {
            return Some(LinkSegment::AppRoute { shown: token.into(), route: token["/app".len()..].into() });
        }
    }
    let rooted = token.starts_with('/') && !is_network(token) && token.len() > 1;
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
        let may_start = previous.is_none_or(|p| p.is_whitespace() || matches!(p, '`' | '"' | '\'' | '(' | '[' | '<' | '=' | '«' | '*'));
        if may_start {
            let mut len = rest.find(is_end).unwrap_or(rest.len());
            // `[/tmp/a](/tmp/b)`: el `(` abre el destino del enlace markdown. En
            // una dirección web puede ser de ella (`Foo_(bar)`).
            if !rest.starts_with("http") {
                len = rest[..len].find('(').unwrap_or(len);
            }
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

    /// Una ruta de red escondida en un `file://` no es un enlace: mirar si
    /// existe ya contacta al servidor SMB (y le da el hash NTLM a quien lo
    /// puso). Mismos casos que `linkify.test.ts`.
    ///
    /// Roturas que atrapa: no rechazar lo que se decodifica a `//`, `/\` o
    /// `\\`; mirar sólo el texto sin decodificar (`%5C%5C`).
    #[test]
    fn a_network_path_hidden_in_a_file_link_is_text() {
        for hidden in ["file:////srv/share/x", "file://localhost//srv/share/x", "file:///%5C%5Csrv%5Cshare", "file:///%2F%2Fsrv/x", "//srv/share/x", r"/\srv/x"] {
            assert!(link_segments(hidden).iter().all(|s| matches!(s, LinkSegment::Text(_))), "{hidden}: {:?}", link_segments(hidden));
        }
    }

    /// Los casos que Rust y TypeScript tienen que resolver igual.
    ///
    /// Roturas que atrapa: `https:///x` como dirección; un escape roto
    /// decodificado a medias (Rust dejaba el roto y decodificaba el resto);
    /// bytes que no son UTF-8 convertidos en `�`.
    #[test]
    fn rust_and_typescript_agree_on_the_odd_cases() {
        assert_eq!(link_segments("https:///x"), vec![text("https:///x")]);
        let decoded = |url: &str| match link_segments(url).as_slice() {
            [LinkSegment::Path { path, .. }] => path.clone(),
            other => panic!("{url}: {other:?}"),
        };
        assert_eq!(decoded("file:///a%zzb"), "/a%zzb");
        assert_eq!(decoded("file:///a%20b%zz"), "/a%20b%zz");
        assert_eq!(decoded("file:///%ff"), "/%ff");
        assert_eq!(decoded("file:///a%20b"), "/a b");
    }

    /// Un enlace markdown y el énfasis no se pegan a la ruta.
    ///
    /// Roturas que atrapa: una ruta `/tmp/a](/tmp/b)` de una pieza; no ver el
    /// destino del enlace; no ver la ruta entre asteriscos.
    #[test]
    fn markdown_around_a_path_is_not_part_of_it() {
        assert_eq!(link_segments("[/tmp/a](/tmp/b)"), vec![text("["), path("/tmp/a"), text("]("), path("/tmp/b"), text(")")]);
        assert_eq!(
            link_segments("[web](https://x.com/a_(b))"),
            vec![text("[web]("), LinkSegment::Url("https://x.com/a_(b)".into()), text(")")]
        );
        let url = |u: &str| LinkSegment::Url(u.into());
        assert_eq!(link_segments("[https://a.com/x](https://b.com/y)"), vec![text("["), url("https://a.com/x"), text("]("), url("https://b.com/y"), text(")")]);
        assert_eq!(link_segments("ve /tmp/a(/tmp/b)"), vec![text("ve "), path("/tmp/a"), text("("), path("/tmp/b"), text(")")]);
        assert_eq!(link_segments("**/tmp/a**"), vec![text("**"), path("/tmp/a"), text("**")]);
        assert_eq!(link_segments("*~/x.md*"), vec![text("*"), path("~/x.md"), text("*")]);
    }
}
