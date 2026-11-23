//! Reglas de aspecto que valen para toda la app y que jose ya tuvo que
//! pedir más de una vez.

use std::path::Path;

/// Las llamadas `Input::new(…)` seguidas de `.small()` o `.xsmall()`: un
/// campo de texto en tamaño chico queda sin aire y apretado (23-nov-2026,
/// pedido varias veces).
fn shrunk_inputs(source: &str) -> usize {
    let mut found = 0;
    let mut rest = source;
    while let Some(at) = rest.find("Input::new(") {
        let after = &rest[at + "Input::new(".len()..];
        let mut depth = 1;
        let mut end = after.len();
        for (index, char) in after.char_indices() {
            match char {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = index + 1;
                break;
            }
        }
        let tail = after[end..].trim_start();
        if tail.starts_with(".small()") || tail.starts_with(".xsmall()") {
            found += 1;
        }
        rest = &after[end..];
    }
    found
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
    assert_eq!(shrunk_inputs("Input::new(&state(x)).w_full()"), 0);
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let shrunk: Vec<&str> = files.iter().filter(|(_, source)| shrunk_inputs(source) > 0).map(|(path, _)| path.as_str()).collect();
    assert!(shrunk.is_empty(), "un campo de texto en tamaño chico (sin aire): {shrunk:?}");
}
