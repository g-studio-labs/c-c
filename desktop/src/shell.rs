//! La raíz de la ventana: qué pantalla toca según quién esté dentro.
//!
//! Qué pantalla le corresponde a una sesión lo dice el core (`gate`). Aquí
//! sólo se cambia de una a otra cuando esa respuesta cambia.

use cac_core::{gate, Gate};
use gpui_kit::*;

use crate::auth::{NewPassword, SignIn};
use crate::backend::Backend;
use crate::workspace::Workspace;

enum Screen {
    SignIn(Entity<SignIn>),
    NewPassword(Entity<NewPassword>),
    Open(Entity<Workspace>),
}

pub struct Shell {
    gate: Gate,
    screen: Screen,
    _subscriptions: [Subscription; 2],
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let snapshot = Backend::snapshot(cx);
        let subscription = cx.observe_in(&snapshot, window, |this, snapshot, window, cx| {
            let now = gate(snapshot.read(cx).session.as_ref());
            // Una pantalla nueva sólo al cambiar de puerta: reconstruirla en
            // cada aviso del core borraría lo que se está escribiendo.
            if now != this.gate {
                this.gate = now;
                this.screen = Self::screen_for(now, window, cx);
            }
            cx.notify();
        });
        // Al volver a la ventana, el documento abierto se pone al día: lo
        // pudo cambiar cualquiera mientras se estaba en otra, y el servidor
        // no avisa de ello. Cuándo hace falta lo dice el core.
        let on_return = cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                Backend::run(cx, |core| async move { core.docs().came_back().await });
            }
        });
        let now = gate(snapshot.read(cx).session.as_ref());
        Self { gate: now, screen: Self::screen_for(now, window, cx), _subscriptions: [subscription, on_return] }
    }

    /// Siempre una pantalla recién hecha: al salir, la contraseña que se
    /// escribió para entrar no sigue en su campo.
    fn screen_for(gate: Gate, window: &mut Window, cx: &mut Context<Self>) -> Screen {
        match gate {
            Gate::SignIn => Screen::SignIn(cx.new(|cx| SignIn::new(window, cx))),
            Gate::NewPassword => Screen::NewPassword(cx.new(|cx| NewPassword::new(window, cx))),
            Gate::Open => Screen::Open(cx.new(|cx| Workspace::new(window, cx))),
        }
    }

    #[cfg(test)]
    fn screen_kind(&self) -> &'static str {
        match self.screen {
            Screen::SignIn(_) => "sign-in",
            Screen::NewPassword(_) => "new-password",
            Screen::Open(_) => "open",
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match &self.screen {
            Screen::SignIn(view) => div().size_full().child(view.clone()),
            Screen::NewPassword(view) => div().size_full().child(view.clone()),
            Screen::Open(view) => div().size_full().child(view.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: arriba se importa `gpui_kit::*`, que trae una macro
    // `test`, y `#[gpui_kit::test]` se expandiría sobre sí misma sin fin.
    use super::{Screen, Shell};
    use crate::backend::Backend;
    use crate::testing::{wait_until, FakeServer};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, ResponseTemplate};

    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Shell>) {
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(760.)),
                })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(window, cx)))
                .expect("open test window")
        })
    }

    fn session(must_change_password: bool) -> serde_json::Value {
        json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz", "mustChangePassword": must_change_password })
    }

    fn login_answers(server: &FakeServer, must_change_password: bool) {
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/login"))
                .and(body_json(json!({ "username": "ana", "password": "secreto-largo", "client": "desktop" })))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "success": true,
                    "data": {
                        "accessToken": "access", "refreshToken": "refresh",
                        "session": session(must_change_password)
                    }
                }))),
        );
    }

    fn type_and_send(window: AnyWindowHandle, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("sign-in-username", cx);
            window.input("ana", cx);
            window.click("sign-in-password", cx);
            window.input("secreto-largo", cx);
            window.click("sign-in-submit", cx);
        })
        .unwrap();
    }

    fn on_screen(window: AnyWindowHandle, id: &'static str, cx: &mut TestAppContext) -> bool {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    /// Escribir usuario y contraseña y pulsar «Entrar» abre la app.
    ///
    /// Roturas que atrapa: que el botón no mande nada; que lo escrito no sea
    /// lo que se manda; que el login no diga que es el escritorio (el
    /// servidor decide con eso qué rutas abre); que la pantalla no cambie al
    /// entrar.
    #[gpui_kit::test]
    fn signing_in_opens_the_app(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        login_answers(&server, false);
        server.install(cx);
        let (window, shell) = open(cx);
        assert!(on_screen(window, "sign-in-submit", cx));
        assert!(!on_screen(window, "open", cx));

        type_and_send(window, cx);

        wait_until(cx, "la app se abre tras entrar", |cx| on_screen(window, "open", cx));
        assert!(matches!(cx.update(|cx| shell.read(cx).screen_kind()), "open"));
        assert!(!on_screen(window, "sign-in-submit", cx));
    }

    /// Quien entra con una contraseña que puso otro la cambia antes de pasar.
    ///
    /// Roturas que atrapa: abrir la app con la contraseña de un alta; que el
    /// formulario de cambio no mande lo escrito; quedarse en él después de
    /// cambiarla.
    #[gpui_kit::test]
    fn a_password_set_by_someone_else_is_changed_before_going_in(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        login_answers(&server, true);
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/change-password"))
                .and(body_json(json!({ "currentPassword": "secreto-largo", "newPassword": "la-mia-propia" })))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "success": true,
                    "data": { "accessToken": "new-access", "refreshToken": "new-refresh" }
                }))),
        );
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/auth/me"))
                .and(header("Authorization", "Bearer new-access"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": session(false) })),
                ),
        );
        server.install(cx);
        let (window, _) = open(cx);
        type_and_send(window, cx);
        wait_until(cx, "pide la contraseña nueva", |cx| on_screen(window, "new-password-submit", cx));
        assert!(!on_screen(window, "open", cx));

        cx.update_window(window, |_, window, cx| {
            window.click("new-password-current", cx);
            window.input("secreto-largo", cx);
            window.click("new-password-next", cx);
            window.input("la-mia-propia", cx);
            window.click("new-password-confirm", cx);
            window.input("la-mia-propia", cx);
            window.click("new-password-submit", cx);
        })
        .unwrap();

        wait_until(cx, "la app se abre tras cambiarla", |cx| on_screen(window, "open", cx));
        assert_eq!(server.hits("/api/v1/auth/change-password"), 1);
    }

    /// Al volver a la ventana se pone al día el documento abierto; al irse
    /// de ella, no.
    ///
    /// Rotura que atrapa: volver de otra ventana —donde un agente acaba de
    /// reescribir el runbook— y seguir leyendo el de antes.
    #[gpui_kit::test]
    fn coming_back_to_the_window_refreshes_the_open_document(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        login_answers(&server, false);
        let get = |route: &str, data: serde_json::Value| {
            Mock::given(method("GET")).and(path(route.to_string())).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data })))
        };
        let org = json!({ "id": "o1", "name": "G Studio", "slug": "g", "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
        let space = json!({ "id": "a", "orgId": "o1", "name": "Producto", "color": "#888", "folders": [], "people": [], "lists": [] });
        server.mount(get("/api/v1/organizations/", json!([org])));
        server.mount(get("/api/v1/task-spaces/", json!([space])));
        server.mount(get("/api/v1/docs/space/a", json!({ "doc": null, "tabs": [{ "key": "overview", "body": "Lo de siempre." }], "decisions": [] })));
        server.mount(get("/api/v1/docs/space/a/pages", json!([])));
        // Sin rato de gracia: lo leído hace un instante ya no vale por reciente.
        server.install_with(cx, |config| config.doc_fresh_for = std::time::Duration::ZERO);
        let (window, _) = open(cx);
        type_and_send(window, cx);
        wait_until(cx, "la app se abre", |cx| on_screen(window, "open", cx));
        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.orgs().load().await;
                core.tree().load("o1").await;
                core.docs().open("a").await;
            })
        });
        wait_until(cx, "llega el documento", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.body() == "Lo de siempre."));

        let leave = |cx: &mut TestAppContext| {
            gpui_kit::VisualTestContext::from_window(window.into(), cx).deactivate_window();
            cx.run_until_parked();
        };
        // Se sale, se vuelve —y entonces se pide—, y se sale otra vez.
        leave(cx);
        let read = server.hits("/api/v1/docs/space/a");
        cx.update_window(window, |_, window, _| window.activate_window()).unwrap();
        wait_until(cx, "al volver se vuelve a pedir", |_| server.hits("/api/v1/docs/space/a") > read);
        let read = server.hits("/api/v1/docs/space/a");
        leave(cx);
        std::thread::sleep(std::time::Duration::from_millis(80));
        cx.run_until_parked();
        assert_eq!(server.hits("/api/v1/docs/space/a"), read, "irse de la ventana volvió a pedir el documento");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Salir vuelve a la pantalla de entrar, vacía, y lo dice al servidor.
    ///
    /// Roturas que atrapa: que «Salir» no haga nada; salir sólo en esta
    /// máquina y dejar el refresh vivo en el servidor; volver a un formulario
    /// con la contraseña de antes todavía escrita.
    #[gpui_kit::test]
    fn logging_out_returns_to_an_empty_sign_in(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        login_answers(&server, false);
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/logout"))
                .and(header("Authorization", "Bearer refresh"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true }))),
        );
        server.install(cx);
        let (window, shell) = open(cx);
        type_and_send(window, cx);
        wait_until(cx, "la app se abre", |cx| on_screen(window, "open", cx));

        // «Salir» vive en el menú de la cuenta, cuyas opciones no se pueden
        // pulsar desde una prueba: se llama a lo que hace.
        cx.update(|cx| crate::workspace::Workspace::log_out(cx));

        wait_until(cx, "vuelve a la pantalla de entrar", |cx| on_screen(window, "sign-in-submit", cx));
        assert_eq!(server.hits("/api/v1/auth/logout"), 1);
        let typed = cx.update(|cx| match &shell.read(cx).screen {
            Screen::SignIn(view) => view.read(cx).typed(cx),
            _ => panic!("no es la pantalla de entrar"),
        });
        assert_eq!(typed, (String::new(), String::new()));
        assert!(cx.update(|cx| Backend::snapshot(cx).read(cx).session.is_none()));
    }
}
