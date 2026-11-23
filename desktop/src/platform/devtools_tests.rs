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
