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
    /// Se quiso cerrar la ventana con texto sin guardar: se está preguntando.
    closing: bool,
    /// El foco de esa pregunta, y quién lo tenía antes: si contestan que
    /// no, se sigue escribiendo donde se estaba.
    close_focus: FocusHandle,
    focus_before: Option<FocusHandle>,
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
        // Cerrar la ventana con texto sin guardar pregunta antes: lo que se
        // pierde sin vuelta atrás siempre pregunta. Sin nada que perder, se
        // cierra sin más.
        let shell = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| shell.update(cx, |this, cx| this.may_close(window, cx)).unwrap_or(true));
        let now = gate(snapshot.read(cx).session.as_ref());
        Self {
            gate: now,
            screen: Self::screen_for(now, window, cx),
            closing: false,
            close_focus: cx.focus_handle(),
            focus_before: None,
            _subscriptions: [subscription, on_return],
        }
    }

    /// Si hay algo escrito que cerrar la ventana perdería: en la
    /// documentación (lo dice el core, que tiene el borrador) o en la tarea
    /// abierta (lo dice su página, que tiene sus cajas).
    fn has_unsaved(&self, cx: &App) -> bool {
        Backend::core(cx).unsaved_docs() || matches!(&self.screen, Screen::Open(workspace) if workspace.read(cx).task.read(cx).has_unsaved(cx))
    }

    /// Si la ventana puede cerrarse ya. Con texto sin guardar no: se
    /// pregunta, y es la respuesta la que la cierra.
    fn may_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.has_unsaved(cx) {
            return true;
        }
        if !self.closing {
            self.closing = true;
            self.focus_before = window.focused(cx);
            self.close_focus.focus(window, cx);
            cx.notify();
        }
        false
    }

    /// La respuesta a «¿cerrar y perder lo escrito?».
    fn answer_close(&mut self, yes: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.closing = false;
        if yes {
            window.remove_window();
        } else if let Some(before) = self.focus_before.take() {
            before.focus(window, cx);
        }
        cx.notify();
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let asking = self.closing.then(|| {
            let shell = cx.entity().downgrade();
            let question = cac_core::close_window_question(crate::lang::locale(cx));
            crate::ask::question_dialog(
                "close-ask",
                question,
                &self.close_focus,
                move |yes, window, cx| {
                    let _ = shell.update(cx, |this, cx| this.answer_close(yes, window, cx));
                },
                window,
                cx,
            )
        });
        let screen = match &self.screen {
            Screen::SignIn(view) => div().size_full().child(view.clone()),
            Screen::NewPassword(view) => div().size_full().child(view.clone()),
            Screen::Open(view) => div().size_full().child(view.clone()),
        };
        screen.child(crate::drop_wake::drop_wake()).children(asking)
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

    /// La ventana despierta el hover cuando entra un fichero de fuera tras
    /// teclear, para que el `drop` de cualquier pantalla llegue.
    ///
    /// Rotura que atrapa: quitar `drop_wake` de la raíz (el destino de un
    /// fichero soltado tras escribir no recibe nada hasta mover el ratón).
    #[gpui_kit::test]
    fn the_window_wakes_hover_when_a_file_enters_after_typing(cx: &mut TestAppContext) {
        use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput};
        let server = FakeServer::start();
        server.install(cx);
        let (window, _) = open(cx);
        cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.press("a", cx);
                let position = Point::new(px(5.), px(5.));
                let paths = ExternalPaths(vec![std::path::PathBuf::from("/tmp/notes.txt")].into());
                assert!(window.last_input_was_keyboard());
                window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position, paths }), cx);
            })
            .unwrap();
        cx.run_until_parked();
        let keyboard_after = cx.update_window(window, |_, window, _| window.last_input_was_keyboard()).unwrap();
        assert!(!keyboard_after, "el hover sigue apagado");
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

    /// La app abierta con la documentación del espacio `a` a la vista, que
    /// quien entra puede escribir.
    fn open_with_a_document(cx: &mut TestAppContext, edit: impl FnOnce(&mut cac_core::Config)) -> (FakeServer, AnyWindowHandle) {
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
        server.install_with(cx, edit);
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
        (server, window)
    }

    /// Cerrar la ventana con texto sin guardar pregunta antes, en un
    /// diálogo: «Seguir aquí», la equis y Escape la dejan abierta con lo
    /// escrito; Enter no contesta; «Cerrar sin guardar» la cierra. Sin nada
    /// que perder, se cierra sin preguntar.
    ///
    /// Roturas que atrapa: cerrar la ventana y perder medio documento sin
    /// aviso; una pregunta que no se pinta (la ventana parece no querer
    /// cerrarse); un Escape o un Enter que cierran; un «Cerrar sin guardar»
    /// que no cierra; preguntar a quien no escribió nada.
    #[gpui_kit::test]
    fn closing_the_window_with_unsaved_text_asks_first(cx: &mut TestAppContext) {
        let (_server, window) = open_with_a_document(cx, |_| {});
        let try_to_close = |cx: &mut TestAppContext| gpui_kit::VisualTestContext::from_window(window.into(), cx).simulate_close();
        let draft = |cx: &mut TestAppContext| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.edit.as_ref().map(|edit| edit.body.clone()));
        let label = |id: &'static str, cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find(id).label().unwrap_or_default().to_string()
            })
            .unwrap()
        };

        // Se escribe en la caja, como quien la pulsa y teclea.
        cx.update(|cx| assert!(Backend::core(cx).docs().start_edit()));
        wait_until(cx, "sale el editor", |cx| on_screen(window, "doc-edit-input", cx));
        cx.update_window(window, |_, window, cx| {
            window.click("doc-edit-input", cx);
            window.input("X", cx);
        })
        .unwrap();
        wait_until(cx, "lo tecleado llega al borrador", |cx| draft(cx).is_some_and(|body| body.contains('X')));

        assert!(!try_to_close(cx), "la ventana se cerró con texto sin guardar");
        wait_until(cx, "se pregunta", |cx| on_screen(window, "close-ask", cx));
        assert_eq!(label("close-ask-title", cx), "¿Cerrar y perder lo que has escrito?");
        cx.update_window(window, |_, window, cx| window.press("enter", cx)).unwrap();
        cx.run_until_parked();
        assert!(on_screen(window, "close-ask", cx), "Enter contestó la pregunta");
        cx.update_window(window, |_, window, cx| window.press("escape", cx)).unwrap();
        wait_until(cx, "Escape la quita", |cx| !on_screen(window, "close-ask", cx));
        // Y se sigue escribiendo donde se estaba, sin pulsar la caja otra vez.
        cx.update_window(window, |_, window, cx| window.input("Z", cx)).unwrap();
        wait_until(cx, "lo siguiente cae en la caja", |cx| draft(cx).is_some_and(|body| body.contains('Z')));
        let typed = draft(cx);

        for way_out in ["close-ask-no", "close-ask-close"] {
            assert!(!try_to_close(cx));
            wait_until(cx, "se pregunta otra vez", |cx| on_screen(window, "close-ask", cx));
            cx.update_window(window, |_, window, cx| window.click(way_out, cx)).unwrap();
            wait_until(cx, "se quita", |cx| !on_screen(window, "close-ask", cx));
            assert_eq!(draft(cx), typed, "{way_out} tiró lo escrito");
        }

        assert!(!try_to_close(cx));
        wait_until(cx, "se pregunta", |cx| on_screen(window, "close-ask", cx));
        cx.update_window(window, |_, window, cx| window.click("close-ask-yes", cx)).unwrap();
        cx.run_until_parked();
        assert!(cx.update_window(window, |_, _, _| ()).is_err(), "«Cerrar sin guardar» no cerró la ventana");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un comentario a medias en una tarea también hace preguntar.
    ///
    /// Rotura que atrapa: que la ventana sólo mire la documentación y se
    /// cierre con un comentario escrito y sin mandar.
    #[gpui_kit::test]
    fn closing_the_window_with_a_half_written_comment_asks_too(cx: &mut TestAppContext) {
        let (server, window) = open_with_a_document(cx, |_| {});
        let detail: serde_json::Value = serde_json::from_str(include_str!("../../core/tests/fixtures/task_detail.json")).unwrap();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": detail }))));
        cx.update(|cx| Backend::run(cx, |core| async move { core.tasks().open("t1").await }));
        wait_until(cx, "llega la tarea", |cx| on_screen(window, "comment-input", cx));
        cx.update_window(window, |_, window, cx| {
            window.click("comment-input", cx);
            window.input("a medias", cx);
        })
        .unwrap();
        assert!(!gpui_kit::VisualTestContext::from_window(window.into(), cx).simulate_close(), "la ventana se cerró con un comentario a medias");
        wait_until(cx, "se pregunta", |cx| on_screen(window, "close-ask", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Sin nada escrito, la ventana se cierra sin preguntar: también con el
    /// editor abierto y sin tocar.
    ///
    /// Rotura que atrapa: una ventana que pregunta «¿perder lo escrito?» a
    /// quien sólo estaba leyendo.
    #[gpui_kit::test]
    fn closing_the_window_with_nothing_to_lose_just_closes(cx: &mut TestAppContext) {
        let (_server, window) = open_with_a_document(cx, |_| {});
        cx.update(|cx| assert!(Backend::core(cx).docs().start_edit()));
        cx.run_until_parked();
        assert!(gpui_kit::VisualTestContext::from_window(window.into(), cx).simulate_close(), "no dejó cerrar la ventana");
        assert!(!on_screen(window, "close-ask", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Al volver a la ventana se pone al día el documento abierto; al irse
    /// de ella, no.
    ///
    /// Rotura que atrapa: volver de otra ventana —donde un agente acaba de
    /// reescribir el runbook— y seguir leyendo el de antes.
    #[gpui_kit::test]
    fn coming_back_to_the_window_refreshes_the_open_document(cx: &mut TestAppContext) {
        // Sin rato de gracia: lo leído hace un instante ya no vale por reciente.
        let (server, window) = open_with_a_document(cx, |config| config.doc_fresh_for = std::time::Duration::ZERO);

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
