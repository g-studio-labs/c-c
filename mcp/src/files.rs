//! Nombres y rutas de ficheros: lo poco que comparten la app (guardar un
//! adjunto, soltar un fichero) y el MCP (bajar un adjunto, subir uno por ruta).

/// Un nombre que no se sale de la carpeta ni trae caracteres raros.
pub fn safe_file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .collect();
    let safe = safe.trim_start_matches('.').trim().to_string();
    if safe.is_empty() { "file".into() } else { safe }
}

/// `nombre.ext`, o `nombre (2).ext` si ya existe: bajar dos veces no pisa nada.
pub fn free_name(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("siempre hay un número libre")
}

/// Deshace el `%XX` de una dirección. Lo que no es un escape válido se deja
/// como está.
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_never_leaves_its_folder() {
        assert_eq!(safe_file_name("../../.ssh/id_rsa"), "sshid_rsa");
        assert!(!safe_file_name("../x").contains('/'));
        assert!(!safe_file_name("..").starts_with('.'));
        assert_eq!(safe_file_name("grabacion 2026-10-10.m4a"), "grabacion 2026-10-10.m4a");
        assert_eq!(safe_file_name("///"), "file");
    }

    #[test]
    fn saving_twice_overwrites_nothing() {
        let dir = std::env::temp_dir().join(format!("cac-free-name-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_name(&dir, "g.m4a"), dir.join("g.m4a"));
        std::fs::write(dir.join("g.m4a"), b"1").unwrap();
        assert_eq!(free_name(&dir, "g.m4a"), dir.join("g (2).m4a"));
        std::fs::write(dir.join("g (2).m4a"), b"2").unwrap();
        assert_eq!(free_name(&dir, "g.m4a"), dir.join("g (3).m4a"));
        assert_eq!(free_name(&dir, "sin-ext"), dir.join("sin-ext"));
        std::fs::write(dir.join("sin-ext"), b"3").unwrap();
        assert_eq!(free_name(&dir, "sin-ext"), dir.join("sin-ext (2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

