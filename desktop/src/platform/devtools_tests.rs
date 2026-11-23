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
/// guardar, y el botón de la fila la abre.
///
/// Rotura que atrapa: un botón que abre otra carpeta (la de inicio, la
/// temporal) o que no abre nada.
#[gpui_kit::test]
fn the_folder_button_opens_where_the_original_was(cx: &mut TestAppContext) {
    use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput};
    let (_server, window) = platform_as("member", cx);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<std::path::PathBuf>::new()));
    let opened = seen.clone();
    cx.update(|cx| cx.set_global(crate::files::Opener(std::sync::Arc::new(move |path| opened.lock().unwrap().push(path.to_path_buf())))));
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
    let _ = std::fs::remove_dir_all(&dir);
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}
