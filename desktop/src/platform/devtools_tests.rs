// Las pruebas de las DevTools. Se incluyen dentro de `workspace::tests` (con
// `include!`) para usar su arnés sin copiarlo.

/// El de jwt.io, firmado con `your-256-bit-secret`.
const JWT_IO: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

/// Un token verificado con otro secreto se dice inválido, y con el suyo,
/// válido; su payload se lee.
///
/// Roturas que atrapa: que la pantalla no mande el secreto (no diría nada
/// de la firma) o mande otro algoritmo; pintar «válida» con cualquier
/// respuesta.
#[gpui_kit::test]
fn a_token_with_another_secret_says_its_signature_is_invalid(cx: &mut TestAppContext) {
    let (_server, window) = platform_as("member", cx);
    click(window, "platform-devtools", cx);
    wait_until(cx, "se abren las DevTools", |cx| on_screen(window, "devtools-rail", cx));
    click(window, "devtools-tokens", cx);
    wait_until(cx, "sale el JWT", |cx| on_screen(window, "tokens-jwt-token", cx));
    click(window, "tokens-jwt-token", cx);
    type_text(window, JWT_IO, cx);
    click(window, "tokens-jwt-secret", cx);
    type_text(window, "otro", cx);
    click(window, "tokens-jwt-decode", cx);
    wait_until(cx, "dice algo de la firma", |cx| on_screen(window, "tokens-jwt-signature", cx));
    assert_eq!(label(window, "tokens-jwt-signature", cx), "Firma inválida");
    assert!(label(window, "tokens-jwt-payload", cx).contains("John Doe"));

    click(window, "tokens-jwt-secret", cx);
    cx.simulate_keystrokes(window, &["backspace"; 4].join(" "));
    type_text(window, "your-256-bit-secret", cx);
    click(window, "tokens-jwt-decode", cx);
    wait_until(cx, "la firma pasa", |cx| label(window, "tokens-jwt-signature", cx) == "Firma válida");
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// La carpeta de una imagen comprimida es la del original: ahí se propone
/// guardar, y el botón de la fila la muestra (y, guardada, muestra el fichero).
///
/// Roturas que atrapa: un botón que abre otra carpeta (la de inicio, la
/// temporal) o que no hace nada; que abra el fichero guardado en vez de
/// revelarlo (`open_with_system` por `reveal_in_folder`).
#[gpui_kit::test]
fn the_folder_button_opens_where_the_original_was(cx: &mut TestAppContext) {
    use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput};
    let (_server, window) = platform_as("member", cx);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<std::path::PathBuf>::new()));
    let opened = seen.clone();
    cx.update(|cx| cx.set_global(crate::files::Revealer(std::sync::Arc::new(move |path| opened.lock().unwrap().push(path.to_path_buf())))));
    let dir = std::env::temp_dir().join(format!("cac-image-folder-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let photo = dir.join("foto.png");
    image::RgbaImage::new(4, 4).save(&photo).unwrap();

    click(window, "platform-devtools", cx);
    wait_until(cx, "se abren las DevTools", |cx| on_screen(window, "devtools-rail", cx));
    click(window, "devtools-image", cx);
    wait_until(cx, "sale el compresor", |cx| on_screen(window, "image-drop", cx));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let position = window.find("image-drop").bounds().center();
        window.dispatch_event(PlatformInput::MouseMove(gpui_kit::MouseMoveEvent { position, pressed_button: None, modifiers: Default::default() }), cx);
        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position, paths: ExternalPaths(vec![photo.clone()].into()) }), cx);
        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Submit { position }), cx);
    })
    .unwrap();
    wait_until(cx, "sale la fila", |cx| on_screen(window, "image-folder-1", cx));
    click(window, "image-folder-1", cx);
    assert_eq!(*seen.lock().unwrap(), [dir.clone()]);

    // Guardada en otra carpeta, el botón muestra ese fichero, no la carpeta
    // de antes.
    let out = dir.join("salida");
    std::fs::create_dir_all(&out).unwrap();
    let saved = out.join("foto-guardada.png");
    click(window, "image-download-1", cx);
    let chosen = saved.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    wait_until(cx, "queda guardada", |_| saved.exists());
    wait_until(cx, "la fila lo sabe", |cx| on_screen(window, "image-folder-1", cx));
    cx.run_until_parked();
    click(window, "image-folder-1", cx);
    assert_eq!(seen.lock().unwrap().last(), Some(&saved));
    let _ = std::fs::remove_dir_all(&dir);
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

static LAUNCHED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Sin `Opener` (como en producción), abrir lanza por la función de `host`
/// con la ruta tal cual, y si falla se dice en la pantalla; si abre, no.
///
/// Roturas que atrapa: lanzar con otra ruta (el padre, la carpeta
/// temporal); tragarse el fallo; avisar aunque se abriera.
#[gpui_kit::test]
fn opening_without_an_opener_launches_the_path_and_says_a_failure(cx: &mut TestAppContext) {
    struct Probe(Vec<String>);
    let (_server, _window) = platform_as("member", cx);
    LAUNCHED.lock().unwrap().clear();
    let probe = cx.new(|_| Probe(Vec::new()));
    let folder = std::path::PathBuf::from("/tmp/cac-575/carpeta");
    for (run, lines_so_far) in [(fail as fn(&str) -> _, 1), (succeed as fn(&str) -> _, 1)] {
        probe.update(cx, |_, cx| crate::files::launch(&folder, cx, |probe: &mut Probe, text, _| probe.0.push(text), run));
        cx.run_until_parked();
        assert_eq!(probe.read_with(cx, |probe, _| probe.0.len()), lines_so_far, "el fallo se dice una vez y el que abrió no dice nada");
    }
    assert_eq!(*LAUNCHED.lock().unwrap(), ["/tmp/cac-575/carpeta", "/tmp/cac-575/carpeta"]);
    assert!(probe.read_with(cx, |probe, _| probe.0[0].contains("No se pudo abrir")));
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));

    fn fail(path: &str) -> Result<cac_mcp::host::Opened, cac_mcp::host::RevealError> {
        LAUNCHED.lock().unwrap().push(path.to_string());
        Err(cac_mcp::host::RevealError::Failed("sin gestor".into()))
    }
    fn succeed(path: &str) -> Result<cac_mcp::host::Opened, cac_mcp::host::RevealError> {
        LAUNCHED.lock().unwrap().push(path.to_string());
        Ok(cac_mcp::host::Opened::WithItsProgram)
    }
}

/// Sin ningún gancho de prueba (como en producción), `open_with_system` y
/// `reveal_in_folder` llegan a `cac_mcp::host`, que resuelve la ruta antes de
/// lanzar nada: una ruta que no existe vuelve con su propia línea, con la ruta.
///
/// Roturas que atrapa: abrir con un `xdg-open` sin reglas (`host::open_dir`,
/// que ejecutaría un `.sh` adjunto) o con algo que no mira la ruta: no diría
/// que falta; y que `reveal_in_folder` no pase por `host`.
#[gpui_kit::test]
fn opening_by_default_goes_through_host(cx: &mut TestAppContext) {
    struct Probe(Vec<String>);
    let (_server, _window) = platform_as("member", cx);
    let probe = cx.new(|_| Probe(Vec::new()));
    let missing = std::path::PathBuf::from("/tmp/cac-575/no-existe/adjunto.sh");
    probe.update(cx, |_, cx| crate::files::open_with_system(&missing, cx, |probe: &mut Probe, text, _| probe.0.push(text)));
    probe.update(cx, |_, cx| crate::files::reveal_in_folder(&missing, cx, |probe: &mut Probe, text, _| probe.0.push(text)));
    cx.run_until_parked();
    let lines = probe.read_with(cx, |probe, _| probe.0.clone());
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in &lines {
        assert!(line.contains("/tmp/cac-575/no-existe/adjunto.sh"), "{line}");
    }
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}
