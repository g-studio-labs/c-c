//! Reglas de aspecto que valen para toda la app y que jose ya tuvo que
//! pedir más de una vez.

use std::path::Path;

/// Las llamadas `Input::new(…)` con `.small()` o `.xsmall()` en su cadena
/// (no sólo pegado: un `.id(…)` en medio lo escondía): un campo de texto en
/// tamaño chico queda sin aire y apretado (23-nov-2026, pedido varias veces).
fn shrunk_inputs(source: &str) -> usize {
    let mut found = 0;
    let mut rest = source;
    while let Some(at) = rest.find("Input::new(") {
        let (_, mut tail) = balanced(&rest[at + "Input::new(".len()..]);
        loop {
            tail = tail.trim_start();
            if tail.starts_with("//") {
                tail = tail.split_once('\n').map_or("", |(_, next)| next);
                continue;
            }
            let Some(call) = tail.strip_prefix('.') else { break };
            let name_len = call.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(call.len());
            let (name, after) = call.split_at(name_len);
            let Some(args) = after.strip_prefix('(') else { break };
            let (inside, next) = balanced(args);
            if (name == "small" || name == "xsmall") && inside.trim().is_empty() {
                found += 1;
            }
            tail = next;
        }
        rest = tail;
    }
    found
}

/// Parte `text`, que empieza justo después de un `(`, en lo de dentro y lo
/// que sigue al `)` que lo cierra.
fn balanced(text: &str) -> (&str, &str) {
    let mut depth = 1;
    for (index, char) in text.char_indices() {
        match char {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return (&text[..index], &text[index + 1..]);
        }
    }
    (text, "")
}

fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") && !path.ends_with("ui_rules.rs") {
            out.push((path.display().to_string(), std::fs::read_to_string(&path).unwrap()));
        }
    }
}

#[test]
fn no_text_input_is_shrunk() {
    assert_eq!(shrunk_inputs("Input::new(&self.a).small()"), 1);
    assert_eq!(shrunk_inputs("Input::new(&self.a)\n    .xsmall()"), 1);
    assert_eq!(shrunk_inputs("Input::new(&self.a)\n    .id(\"x\")\n    // por qué\n    .small()\n    .h_7()"), 1);
    assert_eq!(shrunk_inputs("Input::new(&state(x)).w_full().prefix(Button::new(\"b\").small())"), 0);
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let shrunk: Vec<&str> = files.iter().filter(|(_, source)| shrunk_inputs(source) > 0).map(|(path, _)| path.as_str()).collect();
    assert!(shrunk.is_empty(), "un campo de texto en tamaño chico (sin aire): {shrunk:?}");
}
