//! La pantalla de entrada: los textos, las reglas de sus formularios, el
//! enlace de «olvidé mi contraseña» y el cambio de contraseña.
//!
//! Cada prueba dice qué rotura atrapa, y se ha comprobado rompiendo el código
//! a propósito.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{
    check_new_password, check_sign_in, error_phrase, first_spoken, folds_panel, gate, is_dark, shortcut_text, KeysHeld, pick_locale, text, text_count, text_with, ApiError, Config, Core,
    FormCheck, Gate, Locale, PasswordChanged, ThemePreference, WorkLayout, NoListener, Session, Storage,
};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

#[derive(Default)]
struct MemoryStorage(Mutex<HashMap<String, String>>, std::sync::atomic::AtomicBool);

impl MemoryStorage {
    /// Desde ahora no guarda nada: un disco lleno, un llavero cerrado.
    fn refuse(&self) {
        self.1.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Storage for MemoryStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        if self.1.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("disk full".into());
        }
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn remove(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}

fn problem(key: &str) -> FormCheck {
    FormCheck::Problem { key: key.into() }
}

/// Una frase sale en el idioma pedido, con sus huecos rellenos.
///
/// Roturas que atrapa: contestar siempre en inglés; no encontrar el espacio
/// `common` cuando la clave no lo nombra; dejar el `{{hueco}}` sin rellenar.
#[test]
fn a_phrase_comes_in_the_language_asked() {
    assert_eq!(text(Locale::Es, "common:misc.signIn"), "Entrar");
    assert_eq!(text(Locale::En, "common:misc.signIn"), "Sign in");
    assert_eq!(text(Locale::Es, "misc.signIn"), "Entrar");
    assert_eq!(text(Locale::Es, "nav:panel.spaces"), "Espacios");
    assert_eq!(text_with(Locale::Es, "common:account.welcomeTitle", &[("name", "Ana")]), "Hola, Ana");
}

/// Una cantidad se dice en singular o en plural.
///
/// Roturas que atrapa: «1 visibles»; «0 visible» (el cero va en plural);
/// dejar el número sin poner.
#[test]
fn a_count_is_said_in_singular_or_plural() {
    assert_eq!(text_count(Locale::Es, "work:myWork.visible", 1), "1 visible");
    assert_eq!(text_count(Locale::Es, "work:myWork.visible", 0), "0 visibles");
    assert_eq!(text_count(Locale::Es, "work:myWork.visible", 12), "12 visibles");
}

/// El idioma es el que eligió la persona, y si no el de su máquina.
///
/// Roturas que atrapa: que mande la máquina sobre la preferencia; tomar una
/// preferencia vacía por un idioma; no entender `es_MX.UTF-8`, que es como lo
/// dice Linux; pintar en un idioma que cac no tiene.
#[test]
fn the_language_is_the_persons_choice_and_then_the_machines() {
    assert_eq!(pick_locale(Some("en"), Some("es_MX.UTF-8")), Locale::En);
    assert_eq!(pick_locale(Some(""), Some("es_MX.UTF-8")), Locale::Es);
    assert_eq!(pick_locale(None, Some("ES-mx")), Locale::Es);
    assert_eq!(pick_locale(None, Some("C")), Locale::En);
    assert_eq!(pick_locale(Some("fr"), None), Locale::En);
    assert_eq!(Locale::from_tag("essex"), None);
    assert_eq!(Locale::Es.tag(), "es");
}

/// Un fallo se dice con la frase del catálogo que lleva su etiqueta.
///
/// Roturas que atrapa: enseñar la frase en inglés del servidor teniendo la
/// del catálogo; enseñar una etiqueta cruda cuando el catálogo no la conoce;
/// decir «falló» cuando lo que pasa es que no hay red.
#[test]
fn a_failure_is_said_with_its_catalog_phrase() {
    let server = |code: &str| ApiError::Server { status: 400, code: code.into(), message: "From the server".into() };

    assert_eq!(
        error_phrase(Locale::Es, &server("same-password"), Some("common:admin.changeFailed")),
        text(Locale::Es, "errors:same-password")
    );
    assert_ne!(text(Locale::Es, "errors:same-password"), "errors:same-password");
    // Una etiqueta que esta versión no conoce: lo que quien llama estaba
    // haciendo, o la frase del servidor.
    assert_eq!(
        error_phrase(Locale::Es, &server("widget-exploded"), Some("common:misc.authFailed")),
        "Falló la autenticación"
    );
    assert_eq!(error_phrase(Locale::Es, &server("widget-exploded"), None), "From the server");
    assert_eq!(error_phrase(Locale::Es, &server(""), None), "From the server");
    assert_eq!(
        error_phrase(Locale::Es, &ApiError::Network("timed out".into()), Some("common:misc.authFailed")),
        "Sin conexión"
    );
}

/// El formulario de entrar no manda lo que el servidor va a rechazar.
///
/// Roturas que atrapa: mandar un login sin usuario; reprochar la longitud de
/// una contraseña que todavía no se ha escrito; dejar pasar una de siete.
#[test]
fn the_sign_in_form_knows_when_it_can_be_sent() {
    assert_eq!(check_sign_in("", "secreto-largo"), FormCheck::Incomplete);
    assert_eq!(check_sign_in("   ", "secreto-largo"), FormCheck::Incomplete);
    assert_eq!(check_sign_in("ana", ""), FormCheck::Incomplete);
    assert_eq!(check_sign_in("ana", "1234567"), problem("common:misc.passwordMin8"));
    assert_eq!(check_sign_in("ana", "12345678"), FormCheck::Ready);
    // Ocho letras son ocho letras, aunque pesen más de un byte.
    assert_eq!(check_sign_in("ana", "ñañañaña"), FormCheck::Ready);
}

/// El de cambiar la contraseña reprocha sólo lo que ya está escrito.
///
/// Roturas que atrapa: aceptar la misma contraseña; aceptar una repetición
/// distinta; decir «no coinciden» antes de que se escriba la repetición;
/// dejar mandar una nueva de menos de ocho.
#[test]
fn the_new_password_form_knows_when_it_can_be_sent() {
    assert_eq!(check_new_password("", "", ""), FormCheck::Incomplete);
    assert_eq!(check_new_password("vieja-clave", "nueva-clave", ""), FormCheck::Incomplete);
    assert_eq!(check_new_password("", "nueva-clave", "nueva-clave"), FormCheck::Incomplete);
    assert_eq!(check_new_password("vieja-clave", "corta", "corta"), FormCheck::Incomplete);
    assert_eq!(
        check_new_password("vieja-clave", "nueva-clave", "nueva-clavo"),
        problem("common:admin.passwordsDontMatch")
    );
    assert_eq!(
        check_new_password("vieja-clave", "vieja-clave", "vieja-clave"),
        problem("common:admin.mustDiffer")
    );
    assert_eq!(check_new_password("vieja-clave", "nueva-clave", "nueva-clave"), FormCheck::Ready);
    // Las claves que nombra existen en el catálogo.
    for key in ["common:misc.passwordMin8", "common:admin.passwordsDontMatch", "common:admin.mustDiffer"] {
        assert_ne!(text(Locale::En, key), key);
    }
}

fn ana(must_change_password: bool) -> Session {
    Session {
        id: "u1".into(),
        username: "ana".into(),
        name: None,
        email: None,
        superadmin: false,
        must_change_password,
        scopes: Vec::new(),
        locale: None,
    }
}

/// Quien tiene una contraseña que puso otro no pasa de cambiarla.
///
/// Roturas que atrapa: dejar entrar a la app con la contraseña que mandó un
/// administrador; pedirle que la cambie a quien ya puso la suya; enseñar la
/// app sin nadie dentro.
#[test]
fn a_password_set_by_someone_else_stops_at_changing_it() {
    assert_eq!(gate(None), Gate::SignIn);
    assert_eq!(gate(Some(&ana(true))), Gate::NewPassword);
    assert_eq!(gate(Some(&ana(false))), Gate::Open);
}

async fn core_at(server: &MockServer) -> (Core, Arc<MemoryStorage>) {
    let storage = Arc::new(MemoryStorage::default());
    let core = Core::new(Config::new(server.uri()), storage.clone(), Arc::new(NoListener)).unwrap();
    (core, storage)
}

/// Pedir el enlace va sin sesión, con el usuario limpio y en el idioma de
/// quien lo pide.
///
/// Roturas que atrapa: mandar el token de quien estuviera dentro a una ruta
/// pública; mandar el usuario con los espacios que se colaron al pegar; que
/// el correo salga en el idioma del servidor y no en el de la persona; dar
/// por bueno un «demasiados intentos».
#[tokio::test]
async fn asking_for_the_reset_link_goes_without_a_session_and_in_the_persons_language() {
    let server = MockServer::start().await;
    let (core, _) = core_at(&server).await;
    core.sign_in(ana(false), "access".into(), "refresh".into()).unwrap();

    Mock::given(method("POST"))
        .and(path("/api/v1/public/account/forgot"))
        .and(body_json(serde_json::json!({ "login": "ana@cac.dev" })))
        .and(header("Accept-Language", "es"))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({ "success": true })))
        .expect(1)
        .mount(&server)
        .await;

    core.forgot_password("  ana@cac.dev ", Locale::Es).await.unwrap();
    let sent: Vec<Request> = server.received_requests().await.unwrap();
    assert!(sent[0].headers.get("Authorization").is_none(), "una ruta pública no lleva el token");

    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/public/account/forgot"))
        .respond_with(ResponseTemplate::new(429).set_body_json(
            serde_json::json!({ "success": false, "message": "Too many attempts", "error": "rate-limited" }),
        ))
        .mount(&server)
        .await;
    let refused = core.forgot_password("ana", Locale::En).await.unwrap_err();
    assert!(matches!(refused, ApiError::Server { status: 429, ref code, .. } if code == "rate-limited"));
}

/// Cambiar la contraseña deja dentro a quien la cambia, con el par nuevo.
///
/// Cambiarla revoca el refresh que había. Roturas que atrapa: seguir con el
/// par viejo (la siguiente renovación sería un 401 que saca a quien la acaba
/// de cambiar); renovar con el refresh revocado; no volver a preguntar quién
/// es, que es lo que quita el «tiene que cambiarla».
#[tokio::test]
async fn changing_the_password_keeps_you_in_with_the_new_pair() {
    let server = MockServer::start().await;
    let (core, storage) = core_at(&server).await;
    core.sign_in(ana(true), "old-access".into(), "old-refresh".into()).unwrap();

    Mock::given(method("POST"))
        .and(path("/api/v1/auth/change-password"))
        .and(header("Authorization", "Bearer old-access"))
        .and(body_json(serde_json::json!({ "currentPassword": "vieja-clave", "newPassword": "nueva-clave" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": { "accessToken": "new-access", "refreshToken": "new-refresh" }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/me"))
        .and(header("Authorization", "Bearer new-access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": { "id": "u1", "username": "ana", "mustChangePassword": false }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(401))
        .expect(0)
        .mount(&server)
        .await;

    assert_eq!(core.change_password("vieja-clave", "nueva-clave").await.unwrap(), PasswordChanged::Done);

    assert!(!core.session().unwrap().must_change_password);
    assert_eq!(core.access_token().as_deref(), Some("new-access"));
    let stored = storage.get("session").unwrap();
    assert!(stored.contains("new-refresh") && !stored.contains("old-refresh"), "el par nuevo no se guardó");
}

/// Un servidor que rechaza el cambio lo dice, y la sesión se queda como estaba.
///
/// Rotura que atrapa: dar el cambio por hecho con un 400 (la persona creería
/// que ya tiene la contraseña nueva).
#[tokio::test]
async fn a_refused_password_change_says_why_and_changes_nothing() {
    let server = MockServer::start().await;
    let (core, _) = core_at(&server).await;
    core.sign_in(ana(true), "old-access".into(), "old-refresh".into()).unwrap();

    Mock::given(method("POST"))
        .and(path("/api/v1/auth/change-password"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "success": false, "message": "Could not change password", "error": "invalid-credentials"
        })))
        .mount(&server)
        .await;

    let refused = core.change_password("no-es-esta", "nueva-clave").await.unwrap_err();

    assert!(matches!(refused, ApiError::Server { status: 400, ref code, .. } if code == "invalid-credentials"));
    assert_eq!(core.access_token().as_deref(), Some("old-access"));
    assert!(core.session().unwrap().must_change_password);
}

/// Con un servidor que no devuelve par al cambiar, se renueva como siempre.
///
/// Rotura que atrapa: quedarse con el token de antes, que se acuñó sin las
/// organizaciones de quien acaba de poner su contraseña.
#[tokio::test]
async fn a_server_that_returns_no_pair_gets_a_regular_refresh() {
    let server = MockServer::start().await;
    let (core, _) = core_at(&server).await;
    core.sign_in(ana(true), "old-access".into(), "old-refresh".into()).unwrap();

    Mock::given(method("POST"))
        .and(path("/api/v1/auth/change-password"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .and(header("Authorization", "Bearer old-refresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": { "accessToken": "renewed-access", "refreshToken": "renewed-refresh" }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/me"))
        .and(header("Authorization", "Bearer renewed-access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": { "id": "u1", "username": "ana" }
        })))
        .mount(&server)
        .await;

    core.change_password("vieja-clave", "nueva-clave").await.unwrap();

    assert_eq!(core.access_token().as_deref(), Some("renewed-access"));
}

/// Guardar el perfil manda el nombre y el correo limpios, nunca el usuario,
/// y deja la sesión con lo que el servidor dice ahora.
///
/// Roturas que atrapa: mandar el usuario (cambiarlo rompería las menciones
/// ya escritas); mandar los espacios que se colaron al escribir; no releer la
/// sesión (el menú de la cuenta seguiría con el nombre de antes hasta
/// reiniciar); dar por guardado lo que el servidor rechazó.
#[tokio::test]
async fn saving_the_profile_sends_name_and_email_and_rereads_the_session() {
    let server = MockServer::start().await;
    let (core, storage) = core_at(&server).await;
    core.sign_in(ana(false), "access".into(), "refresh".into()).unwrap();

    Mock::given(method("PATCH"))
        .and(path("/api/v1/auth/me"))
        .and(header("Authorization", "Bearer access"))
        .and(body_json(serde_json::json!({ "name": "Ana Ruiz", "email": "ana@cac.dev" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true, "message": "Profile updated" })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": { "id": "u1", "username": "ana", "name": "Ana Ruiz", "email": "ana@cac.dev" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let session = core.update_profile("  Ana Ruiz ", " ana@cac.dev ").await.unwrap();

    assert_eq!(session.name.as_deref(), Some("Ana Ruiz"));
    assert_eq!(core.session().unwrap().name.as_deref(), Some("Ana Ruiz"));
    assert!(storage.get("session").unwrap().contains("Ana Ruiz"), "la sesión guardada sigue con el nombre de antes");

    server.reset().await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/auth/me"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            serde_json::json!({ "success": false, "message": "Invalid request", "error": "email" }),
        ))
        .mount(&server)
        .await;
    let refused = core.update_profile("Otra", "no-es-correo").await.unwrap_err();
    assert!(matches!(refused, ApiError::Server { status: 400, .. }));
    assert_eq!(core.session().unwrap().name.as_deref(), Some("Ana Ruiz"));
}

/// Lo que falla después de guardar el perfil no lo da por no guardado.
///
/// Roturas que atrapa: decir «no se pudo guardar» cuando lo que falló fue
/// releer; dejar en el menú el nombre de antes porque no se pudo releer.
#[tokio::test]
async fn a_profile_that_saved_but_could_not_be_reread_counts_as_saved() {
    let server = MockServer::start().await;
    let (core, _) = core_at(&server).await;
    core.sign_in(ana(false), "access".into(), "refresh".into()).unwrap();
    Mock::given(method("PATCH"))
        .and(path("/api/v1/auth/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/me"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({ "success": false, "error": "internal" })))
        .mount(&server)
        .await;

    let session = core.update_profile(" Ana Ruiz ", "ana@cac.dev").await.unwrap();

    assert_eq!(session.name.as_deref(), Some("Ana Ruiz"));
    assert_eq!(core.session().unwrap().name.as_deref(), Some("Ana Ruiz"));
    assert_eq!(core.session().unwrap().email.as_deref(), Some("ana@cac.dev"));
}

/// Una contraseña que se cambió no se da por no cambiada por lo que falle
/// después: si no se pudo guardar la sesión nueva se dice eso, y si no se
/// pudo releer quién es, no se deja a nadie en «tienes que cambiarla».
///
/// Roturas que atrapan: decir «falló el cambio» con la contraseña ya
/// cambiada (la persona seguiría usando la de antes); callar que la sesión
/// no quedó guardada; dejar en la pantalla de la contraseña forzada a quien
/// ya la cambió.
#[tokio::test]
async fn a_changed_password_stays_changed_whatever_fails_afterwards() {
    let server = MockServer::start().await;
    let (core, storage) = core_at(&server).await;
    core.sign_in(ana(true), "old-access".into(), "old-refresh".into()).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/change-password"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": { "accessToken": "new-access", "refreshToken": "new-refresh" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/me"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({ "success": false, "error": "internal" })))
        .mount(&server)
        .await;

    assert_eq!(core.change_password("vieja-clave", "nueva-clave").await.unwrap(), PasswordChanged::Done);
    assert!(!core.session().unwrap().must_change_password, "se queda en «tienes que cambiarla»");

    core.sign_in(ana(false), "mid-access".into(), "mid-refresh".into()).unwrap();
    storage.refuse();
    assert_eq!(core.change_password("nueva-clave", "otra-clave").await.unwrap(), PasswordChanged::NotStored);
    assert_eq!(core.access_token().as_deref(), Some("new-access"), "sin guardar, el par nuevo tampoco está en la mano");
}

// ── Preferencias ────────────────────────────────────────────────────────

/// «Mi trabajo» se mira en lista hasta que alguien elige otra vista, y lo
/// elegido se recuerda aparte del tema.
///
/// Roturas que atrapa: olvidar la vista al cerrar; arrancar en otra que no es
/// la lista; guardar la vista donde el tema (elegir una pisaría el otro); que
/// algo guardado que ya no se entiende deje la app sin vista.
#[tokio::test]
async fn my_work_opens_as_a_list_until_another_layout_is_chosen() {
    let server = MockServer::start().await;
    let (core, storage) = core_at(&server).await;
    assert_eq!(core.work_layout(), WorkLayout::List);

    core.set_theme(ThemePreference::Light).unwrap();
    core.set_work_layout(WorkLayout::Calendar).unwrap();
    let reopened = Core::new(Config::new(server.uri()), storage.clone(), Arc::new(NoListener)).unwrap();
    assert_eq!(reopened.work_layout(), WorkLayout::Calendar);
    assert_eq!(reopened.theme(), ThemePreference::Light);

    storage.set("work-layout", "\"timeline\"").unwrap();
    assert_eq!(reopened.work_layout(), WorkLayout::List);
}

/// El panel va fijo hasta que alguien lo pliega, y lo elegido se recuerda en
/// esta máquina, aparte de lo demás.
///
/// Roturas que atrapa: una app que abre con el panel escondido sin que nadie
/// lo pidiera; olvidar al cerrar que se plegó; guardarlo donde el tema o la
/// vista (plegar pisaría el otro); que algo guardado que no se entiende deje
/// la app sin panel.
#[tokio::test]
async fn the_panel_stays_pinned_until_folded_and_then_remembered() {
    let server = MockServer::start().await;
    let (core, storage) = core_at(&server).await;
    assert!(core.panel_pinned());

    core.set_theme(ThemePreference::Light).unwrap();
    core.set_work_layout(WorkLayout::Board).unwrap();
    core.set_panel_pinned(false).unwrap();
    let reopened = Core::new(Config::new(server.uri()), storage.clone(), Arc::new(NoListener)).unwrap();
    assert!(!reopened.panel_pinned());
    assert_eq!(reopened.theme(), ThemePreference::Light);
    assert_eq!(reopened.work_layout(), WorkLayout::Board);

    reopened.set_panel_pinned(true).unwrap();
    assert!(core.panel_pinned());

    storage.set("panel-pinned", "\"a medias\"").unwrap();
    assert!(core.panel_pinned());
}

/// El panel se pliega con ⌘B o Ctrl+B y con nada más, y el texto que lo
/// anuncia nombra la tecla del sistema en el que se está.
///
/// Roturas que atrapa: plegar con una B suelta al escribir; plegar con
/// Ctrl+Mayús+B o Ctrl+Alt+B, que son otros atajos; no plegar con Bloq Mayús
/// puesto; anunciar «⌘B» en un teclado sin esa tecla, o «Ctrl+B» en un Mac.
#[test]
fn the_panel_folds_with_the_systems_shortcut_and_says_so() {
    let secondary = KeysHeld { secondary: true, ..KeysHeld::default() };
    assert!(folds_panel("b", secondary));
    assert!(folds_panel("B", secondary));
    assert!(!folds_panel("b", KeysHeld::default()));
    assert!(!folds_panel("n", secondary));
    assert!(!folds_panel("b", KeysHeld { shift: true, ..secondary }));
    assert!(!folds_panel("b", KeysHeld { alt: true, ..secondary }));

    let hint = text(Locale::Es, "nav:rail.collapsePanel");
    assert_eq!(shortcut_text(&hint, true), "Plegar el panel (⌘B)");
    assert_eq!(shortcut_text(&hint, false), "Plegar el panel (Ctrl+B)");
    assert_eq!(shortcut_text(&text(Locale::En, "nav:rail.collapsePanel"), false), "Collapse the panel (Ctrl+B)");
}

/// De la lista de idiomas del sistema se toma el primero que cac habla.
///
/// Roturas que atrapa: quedarse con el primero a secas (quien prefiere
/// francés y después castellano vería cac en inglés); inventar un idioma
/// cuando el sistema no dice ninguno; callar lo que dijo el sistema cuando
/// cac no habla ninguno de los suyos.
#[test]
fn of_the_systems_languages_the_first_one_cac_speaks_is_taken() {
    let tags = |list: &[&str]| list.iter().map(|tag| tag.to_string()).collect::<Vec<_>>();
    assert_eq!(first_spoken(&tags(&["fr-FR", "es-MX", "en-US"])).as_deref(), Some("es-MX"));
    assert_eq!(first_spoken(&tags(&["en-GB", "es-ES"])).as_deref(), Some("en-GB"));
    assert_eq!(first_spoken(&tags(&["fr-FR", "de-DE"])).as_deref(), Some("fr-FR"));
    assert_eq!(first_spoken(&[]), None);
    // Y con eso el core elige: castellano en el primer caso, inglés sin nada.
    assert_eq!(pick_locale(None, first_spoken(&tags(&["fr-FR", "es-MX"])).as_deref()), Locale::Es);
    assert_eq!(pick_locale(None, first_spoken(&tags(&["fr-FR"])).as_deref()), Locale::En);
}

/// El tema es oscuro hasta que alguien elige otro, y lo elegido se recuerda.
///
/// Roturas que atrapa: abrir en claro una herramienta que se mira de noche;
/// olvidar lo elegido al cerrar; que «sistema» con un sistema que no contesta
/// salga en claro; que «claro» obedezca al sistema.
#[tokio::test]
async fn the_theme_is_dark_until_chosen_and_then_remembered() {
    let server = MockServer::start().await;
    let (core, storage) = core_at(&server).await;
    assert_eq!(core.theme(), ThemePreference::Dark);

    core.set_theme(ThemePreference::Light).unwrap();
    assert_eq!(core.theme(), ThemePreference::Light);
    let reopened = Core::new(Config::new(server.uri()), storage.clone(), Arc::new(NoListener)).unwrap();
    assert_eq!(reopened.theme(), ThemePreference::Light);

    assert!(is_dark(ThemePreference::Dark, Some(false)));
    assert!(!is_dark(ThemePreference::Light, Some(true)));
    assert!(is_dark(ThemePreference::System, Some(true)));
    assert!(!is_dark(ThemePreference::System, Some(false)));
    assert!(is_dark(ThemePreference::System, None));
}

/// Elegir idioma cambia ya en esta pantalla y se guarda en la cuenta.
///
/// Roturas que atrapa: esperar al servidor para cambiar el idioma en
/// pantalla; no decírselo al servidor (en otro dispositivo seguiría el de
/// antes); mandar «el del sistema» como una ausencia en vez de como vacío;
/// perder lo elegido porque el servidor no pudo guardarlo.
#[tokio::test]
async fn choosing_a_language_changes_it_here_and_saves_it_on_the_account() {
    let server = MockServer::start().await;
    let (core, storage) = core_at(&server).await;
    core.sign_in(ana(false), "access".into(), "refresh".into()).unwrap();
    assert_eq!(core.locale_preference(), None);

    Mock::given(method("PATCH"))
        .and(path("/api/v1/auth/locale"))
        .and(header("Authorization", "Bearer access"))
        .and(body_json(serde_json::json!({ "locale": "es" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "success": true })))
        .expect(1)
        .mount(&server)
        .await;
    core.set_locale(Some(Locale::Es)).await.unwrap();
    assert_eq!(core.locale_preference(), Some(Locale::Es));
    assert!(storage.get("session").unwrap().contains("\"locale\":\"es\""), "no quedó guardado con la sesión");

    // «El del sistema» viaja como vacío; y si el servidor no puede guardarlo,
    // se dice, pero lo elegido aquí se queda.
    server.reset().await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/auth/locale"))
        .and(body_json(serde_json::json!({ "locale": "" })))
        .respond_with(ResponseTemplate::new(500).set_body_json(
            serde_json::json!({ "success": false, "message": "Could not save the language", "error": "locale-not-saved" }),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let failed = core.set_locale(None).await.unwrap_err();
    assert!(matches!(failed, ApiError::Server { status: 500, ref code, .. } if code == "locale-not-saved"));
    assert_eq!(core.locale_preference(), None);
}
