//! Lo que tiene que ser verdad para repartir la app de escritorio nativa, leído
//! de los ficheros que la reparten: el workflow de la release, los
//! empaquetadores y `desktop/VERSION`, contra lo que pide el actualizador
//! (`cac_mcp::desktop_update`).
//!
//! Corre aquí, en `mcp/`, porque la app es un binario sin biblioteca que
//! importar y porque `mcp.yml` ya lo corre en un runner limpio.
//!
//! El primero es el que más importa. El actualizador de la app de Tauri lee
//! `https://github.com/g-studio-labs/c-c/releases/latest/download/latest.json`.
//! Las releases de la app nativa salen del **mismo repo**; si una se marcase
//! como «latest», esa dirección apuntaría a una release sin `latest.json` y la
//! app de siempre dejaría de actualizarse para todo el mundo, sin un solo error
//! a la vista.

use cac_mcp::desktop_update as desktop;
use cac_mcp::update;

fn read(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

const WORKFLOW: &str = "../.github/workflows/desktop-release.yml";

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

fn position(commands: &[String], needle: &str) -> usize {
    commands.iter().position(|line| line.contains(needle)).unwrap_or_else(|| panic!("falta `{needle}`"))
}

fn exact(commands: &[String], line: &str) -> usize {
    commands.iter().position(|candidate| candidate == line).unwrap_or_else(|| panic!("falta la línea `{line}`"))
}

/// Una release de la app nativa nunca es la «latest» del repo.
///
/// Roturas que atrapa: quitar `--latest=false` de la orden que publica (GitHub
/// la marcaría sola, por fecha); publicar por otro camino que no lo lleva (una
/// action, la API); editar la release después para marcarla; que el
/// actualizador pregunte por `releases/latest`, que es de la app de Tauri.
#[test]
fn a_desktop_release_is_never_marked_as_the_latest() {
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
    for line in read("src/desktop_update.rs").lines().map(str::trim).filter(|line| !line.starts_with("//")) {
        assert!(!line.contains("releases/latest"), "el actualizador pregunta por la release «latest», que es de la app de Tauri: {line}");
    }
}

/// Sólo sale a mano y con `publish`; una versión publicada no se re-corta; y
/// sólo el trabajo que publica puede escribir.
///
/// Roturas que atrapa: un disparador por tag o por push (empujar una etiqueta
/// publicaría sin que nadie lo decidiera); que `publish` valga `true` por
/// defecto; publicar sin mirar antes si la etiqueta existe, o mirarlo después
/// de compilar; que el trabajo que publica corra sin `publish`; dar permiso de
/// escritura a todo el workflow; publicar con algún sistema sin construir.
#[test]
fn publishing_is_opt_in_by_hand_and_refuses_a_version_already_out() {
    let text = read(WORKFLOW);
    let commands = commands(&text);
    let on = exact(&commands, "on:");
    assert_eq!(commands[on + 1], "workflow_dispatch:", "{:?}", &commands[on..on + 3]);
    for trigger in ["push:", "pull_request", "schedule:", "tags:", "release:", "workflow_run:"] {
        assert!(!commands.iter().any(|line| line.starts_with(trigger)), "el workflow se dispara con `{trigger}`");
    }
    let publish_input = exact(&commands, "publish:");
    assert_eq!(commands[publish_input + 3], "default: false", "publicar no puede ser lo normal: {:?}", &commands[publish_input..publish_input + 4]);

    let tag_check = position(&commands, "git ls-remote --exit-code --tags origin \"refs/tags/$tag\"");
    let release_check = position(&commands, "gh release view \"$tag\" --repo \"$GITHUB_REPOSITORY\" >/dev/null 2>&1; then");
    let version_job = exact(&commands, "version:");
    let build_job = exact(&commands, "build:");
    let publish_job = commands.iter().rposition(|line| line == "publish:").expect("falta el trabajo publish");
    let create = position(&commands, "gh release create");
    assert!(publish_job < create);
    assert!(version_job < tag_check && tag_check < build_job, "no mira si la etiqueta existe antes de compilar");
    assert!(version_job < release_check && release_check < build_job, "no mira si la release existe antes de compilar");
    assert!(commands[build_job + 1..build_job + 3].iter().any(|line| line == "needs: [test, guards, plan, version]"), "compilar no espera a las pruebas, a los guardianes y a la comprobación de la versión");
    assert!(position(&commands, "uses: ./.github/workflows/desktop.yml") < build_job, "las pruebas no van por delante");
    assert!(position(&commands, "uses: ./.github/workflows/mcp.yml") < build_job, "los guardianes de la release no van por delante");
    // La etiqueta se vuelve a mirar en el trabajo que publica, justo antes.
    let rechecks: Vec<usize> = commands.iter().enumerate().filter(|(_, line)| line.contains("git ls-remote --exit-code --tags origin")).map(|(at, _)| at).collect();
    assert_eq!(rechecks.len(), 2, "la etiqueta se mira en `version` y otra vez al publicar: {rechecks:?}");
    assert!(publish_job < rechecks[1] && rechecks[1] < create, "el trabajo que publica no vuelve a mirar la etiqueta antes de crear la release");
    assert!(commands.iter().any(|line| line == "tag=\"desktop-v$version\""), "la etiqueta no es desktop-vX.Y.Z");
    assert!(commands.iter().any(|line| line == "tag=\"desktop-v$VERSION\""), "se publica con otra etiqueta");
    assert!(commands.iter().any(|line| line.contains("refs/heads/main") && line.contains("GITHUB_REF")), "se puede publicar fuera de main");
    for system in ["LINUX", "WINDOWS", "MACOS"] {
        assert!(commands.iter().any(|line| line.contains(&format!("\"${system}\" != \"true\""))), "publicar sin exigir {system}");
    }

    // Sólo con `publish`: el trabajo, y el paso que comprueba la versión.
    assert_eq!(commands[publish_job + 1], "if: inputs.publish", "el trabajo que publica corre sin `publish`");
    assert!(commands[version_job..build_job].iter().any(|line| line == "if: inputs.publish"), "la comprobación de la versión estorba sin `publish`");

    // Leer para todos; escribir, sólo el trabajo que publica.
    let writes = commands.iter().filter(|line| line.starts_with("contents: write")).count();
    let reads = commands.iter().filter(|line| line.starts_with("contents: read")).count();
    assert_eq!((reads, writes), (1, 1), "los permisos del workflow cambiaron");
    assert!(publish_job < position(&commands, "contents: write") && position(&commands, "contents: write") < create);
}

/// `desktop/VERSION` es lo que leen las apps instaladas para saber cuál es la
/// última, y tiene que ser la versión de lo que se compila.
///
/// Rotura que atrapa: subir una y no la otra. Con `VERSION` por delante, la app
/// avisaría de una versión que al instalarla dice ser la anterior, y volvería a
/// avisar para siempre; con `Cargo.toml` por delante, nadie se enteraría de que
/// hay una nueva (el workflow también se niega, pero esto lo dice antes).
#[test]
fn the_version_file_and_the_manifest_agree() {
    let file = read("../desktop/VERSION");
    let manifest = read("../desktop/Cargo.toml");
    let in_package = format!("\n{manifest}");
    let in_package = in_package.split("\n[").find(|section| section.starts_with("package]")).expect("Cargo.toml tiene [package]");
    let version = in_package.lines().find_map(|line| line.strip_prefix("version = ")).expect("[package] tiene version").trim_matches('"');
    assert_eq!(file, format!("{version}\n"), "desktop/VERSION y desktop/Cargo.toml dicen versiones distintas");
    assert!(update::parse_version(&file).is_some());
    assert!(desktop::VERSION_URL.ends_with("/main/desktop/VERSION"));
    assert_eq!(desktop::tag("1.2.3"), "desktop-v1.2.3");
}

/// Los sitios que nombran los ficheros de una release dicen lo mismo: lo que
/// piden el actualizador y el workflow, y lo que fabrican los tres
/// empaquetadores.
///
/// Roturas que atrapa: renombrar un fichero en un sitio y no en otro (la app
/// pediría un 404, o el workflow exigiría uno que nadie fabrica); un
/// empaquetador que deja de sacar el binario suelto de Linux, que es lo único
/// que se puede sustituir sin permisos; cambiar el nombre del binario en
/// `identity.env` sin que el actualizador se entere.
#[test]
fn the_workflow_the_packagers_and_the_updater_name_the_same_files() {
    let workflow = read(WORKFLOW);
    let identity = read("../desktop/packaging/identity.env");
    assert!(identity.lines().any(|line| line == &format!("APP_BINARY=\"{}\"", desktop::APP_BINARY)), "identity.env nombra otro binario");

    // El workflow exige exactamente lo que pide la app en cada sistema.
    let asked = [("linux", "x86_64"), ("macos", "aarch64"), ("windows", "x86_64")].map(|(os, arch)| desktop::asset_name("$VERSION", os, arch).unwrap().0);
    let required: Vec<String> = workflow
        .split('"')
        .filter(|piece| piece.starts_with("cac-desktop-$VERSION-"))
        .map(str::to_string)
        .collect();
    assert_eq!(required, asked, "el workflow exige otros ficheros que los que pide la app");
    assert!(workflow.contains("sha256sum > SHA256SUMS") && workflow.contains(update::CHECKSUMS) && workflow.contains("! -name SHA256SUMS"), "SHA256SUMS no suma lo que se sube");
    assert!(workflow.lines().any(|line| line.trim().ends_with("./*")), "la release no sube todo lo construido");

    // Y los empaquetadores fabrican esos nombres, con las variables suyas.
    let (linux, macos, windows) = (read("../desktop/packaging/linux/package.sh"), read("../desktop/packaging/macos/package.sh"), read("../desktop/packaging/windows/package.ps1"));
    let linux_name = desktop::asset_name("$APP_VERSION", "linux", "x86_64").unwrap().0.replace("cac-desktop", "$APP_BINARY").replace("x86_64", "$ARCH");
    assert!(linux.contains(&format!("NAME=\"{linux_name}\"")), "package.sh de Linux nombra otro fichero que {linux_name}");
    assert!(linux.contains("install -Dm755 \"$ROOT/bin/$APP_BINARY\" \"$OUT_DIR/$NAME\""), "package.sh de Linux no deja el binario suelto");
    assert!(linux.contains("strip --strip-unneeded \"$ROOT/bin/$APP_BINARY\"") && linux.find("strip --strip-unneeded").unwrap() < linux.find("\"$OUT_DIR/$NAME\"").unwrap(), "el binario suelto sale sin quitar los símbolos");
    let macos_name = desktop::asset_name("$APP_VERSION", "macos", "aarch64").unwrap().0.replace("cac-desktop", "$APP_BINARY").replace("arm64", "$ARCH");
    assert!(macos.contains(&format!("DMG=\"$OUT_DIR/{macos_name}\"")), "package.sh de macOS nombra otro fichero que {macos_name}");
    let windows_name = desktop::asset_name("$version", "windows", "x86_64").unwrap().0.replace("cac-desktop", "$binary");
    let (base, setup) = windows_name.split_at(windows_name.find("-setup.exe").unwrap());
    assert!(windows.contains(&format!("$base = \"{}\"", base.replace("x86_64", "$arch"))), "package.ps1 nombra otra base que {base}");
    assert!(windows.contains(&format!("\"$base{setup}\"")), "package.ps1 nombra otro instalador que {setup}");
    assert!(windows.contains("\"x86_64\""), "package.ps1 no llama x86_64 a la arquitectura de Windows");
    assert!(read("../desktop/packaging/macos/package.sh").contains("ARCH=\"$(uname -m)\""));
}

/// La app de macOS se firma con la identidad fija de la casa y declara el
/// micrófono y la cámara.
///
/// macOS ata el permiso de captura a la identidad de la firma: con la ad hoc
/// cambiaba en cada build y el aviso volvía en cada llamada y con cada
/// actualización. Y con el runtime reforzado (`--options runtime`) y sin
/// entitlements, deniega el micrófono sin preguntar.
///
/// Roturas que atrapa: quitar del `Info.plist` el texto del micrófono o de la
/// cámara (macOS cierra la app al capturar); firmar ad hoc o con otra identidad
/// que `MACOS_SIGNING_IDENTITY`; añadir `--options runtime`; que la release
/// salga sin exigir la identidad (firmaría ad hoc en silencio); que el workflow
/// no importe el certificado ni pase la identidad al empaquetador.
#[test]
fn the_macos_app_is_signed_with_the_fixed_identity_and_declares_mic_and_camera() {
    let plist = read("../desktop/packaging/macos/Info.plist");
    for key in ["NSMicrophoneUsageDescription", "NSCameraUsageDescription"] {
        let at = plist.find(&format!("<key>{key}</key>")).unwrap_or_else(|| panic!("el Info.plist no declara {key}"));
        let value = plist[at..].lines().nth(1).unwrap().trim();
        assert!(value.starts_with("<string>") && value.len() > "<string></string>".len(), "{key} sin texto: {value}");
    }

    let package = commands(&read("../desktop/packaging/macos/package.sh"));
    for line in &package {
        assert!(!line.contains("--options") && !line.contains("runtime"), "runtime reforzado: macOS denegaría el micrófono sin preguntar: {line}");
    }
    let signing: Vec<&String> = package.iter().filter(|line| line.starts_with("codesign --force")).collect();
    assert_eq!(signing.len(), 1, "{signing:#?}");
    assert_eq!(signing[0], "codesign --force --deep --sign \"$MACOS_SIGNING_IDENTITY\" \"$APP\"");
    let refuses = position(&package, "REQUIRE_SIGNING_IDENTITY:-}\" = \"1\"");
    assert!(refuses < package.iter().position(|line| line == signing[0]).unwrap() && package[refuses..].iter().any(|line| line == "exit 1"), "la release no se niega a firmar ad hoc");

    let workflow = commands(&read(WORKFLOW));
    assert!(workflow.iter().any(|line| line.starts_with("MACOS_SIGNING_IDENTITY: ${{ secrets.MACOS_SIGNING_IDENTITY }}")));
    assert!(workflow.iter().any(|line| line == "REQUIRE_SIGNING_IDENTITY: \"1\""), "el empaquetador de la release no exige la identidad");
    assert!(workflow.iter().any(|line| line.contains("security import")) && workflow.iter().any(|line| line.contains("MACOS_CERTIFICATE: ${{ secrets.MACOS_CERTIFICATE }}")), "no importa el certificado");
    let import = position(&workflow, "security import");
    assert!(import < position(&workflow, "bash packaging/macos/package.sh"), "firma antes de importar el certificado");
}
