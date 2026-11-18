//! Lo que tiene que ser verdad para repartir `cac-mcp`, leído de los ficheros
//! que lo reparten: el workflow de la release y los dos scripts de instalar.
//!
//! El más importante es el primero. El actualizador de la app de Tauri lee
//! `https://github.com/g-studio-labs/c-c/releases/latest/download/latest.json`.
//! Las releases del MCP salen del **mismo repo**; si una se marcase como
//! «latest», esa dirección apuntaría a una release sin `latest.json` y la app
//! dejaría de actualizarse para todo el mundo, sin un solo error a la vista.

use cac_mcp::update;

fn read(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

const WORKFLOW: &str = "../.github/workflows/mcp-release.yml";

/// Las líneas que hacen algo: sin comentarios, y con las órdenes partidas con
/// `\` juntas en una.
fn commands(text: &str) -> Vec<String> {
    let mut joined = Vec::new();
    let mut pending = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        match line.strip_suffix('\\') {
            Some(start) => pending.push_str(start),
            None => joined.push(std::mem::take(&mut pending) + line),
        }
    }
    joined
}

/// Una release del MCP nunca es la «latest» del repo.
///
/// Roturas que atrapa: quitar `--latest=false` de la orden que publica (GitHub
/// la marcaría sola, por fecha); publicar por otro camino que no lo lleva (una
/// action, la API); editar la release después para marcarla.
#[test]
fn an_mcp_release_is_never_marked_as_the_latest() {
    let commands = commands(&read(WORKFLOW));
    let creating: Vec<&String> = commands.iter().filter(|line| line.contains("release create")).collect();
    assert_eq!(creating.len(), 1, "el workflow publica con una sola orden, y ésta no está: {creating:#?}");
    assert!(creating[0].contains("gh release create") && creating[0].contains("--latest=false"), "la release saldría como «latest»: {}", creating[0]);
    for line in &commands {
        for other_way in ["action-gh-release", "tauri-action", "create-release", "make_latest", "release edit", "gh api", "releases/latest", "--latest=true", "--latest "] {
            assert!(!line.contains(other_way), "otro camino para publicar o marcar la última (`{other_way}`): {line}");
        }
        assert!(!line.ends_with("--latest"), "{line}");
    }
    // Y quien instala tampoco pregunta por «latest»: la última versión del MCP
    // se lee de `mcp/VERSION`.
    for file in ["src/update.rs", "../scripts/install-mcp.sh", "../scripts/install-mcp.ps1"] {
        for line in commands_of_any(&read(file)) {
            assert!(!line.contains("releases/latest"), "{file} pregunta por la release «latest», que es de la app: {line}");
        }
    }
}

/// Las líneas que no son comentario, en Rust, sh o PowerShell.
fn commands_of_any(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|line| !line.starts_with('#') && !line.starts_with("//")).map(str::to_string).collect()
}

/// La release sólo sale a mano, y una versión publicada no se re-corta.
///
/// Roturas que atrapa: un disparador por tag o por push (empujar una etiqueta
/// publicaría sin que nadie lo decidiera); publicar sin mirar antes si la
/// etiqueta ya existe; dar permiso de escritura a todo el workflow.
#[test]
fn the_release_only_runs_by_hand_and_refuses_a_version_already_out() {
    let text = read(WORKFLOW);
    let commands = commands(&text);
    let on = commands.iter().position(|line| line == "on:").expect("el workflow dice cuándo corre");
    assert_eq!(commands[on + 1], "workflow_dispatch:", "{:?}", &commands[on..on + 3]);
    assert_eq!(commands[on + 2], "permissions:", "hay otro disparador");
    for trigger in ["push:", "pull_request", "schedule:", "tags:", "release:", "workflow_run:"] {
        assert!(!commands.iter().any(|line| line.starts_with(trigger)), "el workflow se dispara con `{trigger}`");
    }

    let at = |needle: &str| commands.iter().position(|line| line.contains(needle)).unwrap_or_else(|| panic!("falta `{needle}`"));
    let publish = at("gh release create");
    assert!(at("git ls-remote --exit-code --tags origin \"refs/tags/$tag\"") < publish, "no mira si la etiqueta existe antes de publicar");
    assert!(at("gh release view \"$tag\"") < publish, "no mira si la release existe antes de publicar");
    assert!(at("uses: ./.github/workflows/mcp.yml") < publish, "las pruebas no van por delante");
    assert!(commands[publish].contains("\"$tag\"") && commands.iter().any(|line| line == "tag=\"mcp-v$VERSION\""), "la etiqueta no es mcp-vX.Y.Z");

    // Leer para todos; escribir, sólo el trabajo que publica.
    let writes = commands.iter().filter(|line| line.starts_with("contents: write")).count();
    let reads = commands.iter().filter(|line| line.starts_with("contents: read")).count();
    assert_eq!((reads, writes), (1, 1), "los permisos del workflow cambiaron");
    let job = at("publish:");
    assert!(job < at("contents: write") && at("contents: write") < publish);
}

/// `mcp/VERSION` es lo que leen los instalados para saber cuál es la última,
/// y tiene que ser la versión de lo que se compila.
///
/// Rotura que atrapa: subir una y no la otra. Con `VERSION` por delante, todo
/// `cac-mcp` avisaría de una versión que al instalarla dice ser la anterior, y
/// volvería a avisar para siempre; con `Cargo.toml` por delante, nadie se
/// enteraría de que hay una nueva.
#[test]
fn the_version_file_and_the_manifest_agree() {
    let file = read("VERSION");
    assert_eq!(file, format!("{}\n", env!("CARGO_PKG_VERSION")), "mcp/VERSION y mcp/Cargo.toml dicen versiones distintas");
    assert!(update::parse_version(&file).is_some());
    assert!(update::VERSION_URL.ends_with("/main/mcp/VERSION"));
}

/// Los cuatro sitios que nombran los ficheros de una release dicen lo mismo:
/// el workflow que los sube, y los tres que los bajan (`update.rs` y los dos
/// scripts). Y bajan de las mismas direcciones.
///
/// Roturas que atrapa: renombrar un fichero en un sitio y no en otro (el
/// instalador pediría un 404, o `cac-mcp update` uno que el workflow no sube);
/// un script que lee la versión o baja de otro sitio que el binario; una
/// plataforma que el workflow construye y nadie sabe pedir.
#[test]
fn the_workflow_the_scripts_and_the_binary_name_the_same_files() {
    let (workflow, sh, ps) = (read(WORKFLOW), read("../scripts/install-mcp.sh"), read("../scripts/install-mcp.ps1"));
    let by_platform = [("linux", "x86_64"), ("macos", "aarch64"), ("macos", "x86_64"), ("windows", "x86_64")];
    let named: Vec<&str> = by_platform.iter().map(|(os, arch)| update::asset_name(os, arch).unwrap()).collect();
    assert_eq!(named, update::ASSETS);
    assert_eq!(update::asset_name("linux", "aarch64"), None);

    // El workflow construye exactamente esos, y los suma y publica todos.
    let built: Vec<&str> = workflow.lines().filter_map(|line| line.trim().strip_prefix("asset: ")).collect();
    assert_eq!(built, update::ASSETS, "el workflow construye otros ficheros");
    let all = update::ASSETS.join(" ");
    let commands = commands(&workflow);
    assert!(commands.iter().any(|line| line == &format!("sha256sum {all} > {}", update::CHECKSUMS)), "SHA256SUMS no suma los cuatro");
    assert!(commands.iter().any(|line| line.contains("gh release create") && line.ends_with(&format!("{all} {}", update::CHECKSUMS))), "la release no sube los cuatro y sus sumas");

    // Cada script pide los de su sistema.
    for asset in update::ASSETS {
        let script = if asset.ends_with(".exe") { &ps } else { &sh };
        assert!(script.contains(&format!("\"{asset}\"")) || script.contains(&format!("'{asset}'")), "ningún script instala {asset}");
    }
    for (name, script) in [("install-mcp.sh", &sh), ("install-mcp.ps1", &ps)] {
        assert!(script.contains(update::VERSION_URL), "{name} lee la versión de otro sitio");
        assert!(script.contains(update::DOWNLOAD_BASE), "{name} baja de otro sitio");
        assert!(script.contains("/mcp-v$version"), "{name} no baja de la etiqueta mcp-vX.Y.Z");
        assert!(script.contains(&format!("/{}", update::CHECKSUMS)), "{name} no baja las sumas");
        assert!(script.contains(update::TOKEN_PLACEHOLDER), "{name} no marca dónde va el token");
        assert!(script.contains(&format!("https://raw.githubusercontent.com/g-studio-labs/c-c/main/scripts/{name}")), "{name} no dice cómo se lanza");
        for variable in [update::ENV_VERSION_URL, update::ENV_DOWNLOAD_BASE] {
            assert!(script.contains(variable), "{name} no deja cambiar {variable}");
        }
    }
    assert!(update::INSTALL_SH.contains("/main/scripts/install-mcp.sh") && update::INSTALL_PS.contains("/main/scripts/install-mcp.ps1"));
    assert_eq!(update::tag("1.33.0"), "mcp-v1.33.0");
}
