//! La pantalla de entrada: entrar, pedir el enlace de «olvidé mi contraseña»
//! y poner una contraseña propia cuando la que hay la puso otro.
//!
//! Aquí no se decide nada: cuándo se puede mandar un formulario, qué se le
//! pide al servidor y cómo se dice un fallo son del core. La pantalla recoge
//! lo escrito, pregunta, y pinta la respuesta.

use cac_core::{check_new_password, check_sign_in, ApiError, FormCheck};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Icon};
use gpui_kit::*;

use crate::backend::Backend;
use crate::theme::Leading as _;
use crate::lang::{error_text, locale, tr};

/// El ancho de la tarjeta, el de la app de siempre (`max-w-sm`).
const CARD_WIDTH: f32 = 384.;

fn card(cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.background)
        .text_color(theme.foreground)
}

/// La tarjeta de entrar: la `Card` de la app de siempre (`gap-4 rounded-xl
/// bg-card py-4 text-sm` con su aro `ring-1 ring-foreground/10`; `rounded-xl`
/// son 14 px con el `--radius` de `index.css`). El margen de los lados lo
/// pone cada parte (`px-4`), como `CardHeader` y `CardContent`.
fn card_body(cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .w(px(CARD_WIDTH))
        .flex()
        .flex_col()
        .gap_4()
        .py_4()
        .rounded(px(14.))
        .border_1()
        .border_color(theme.foreground.opacity(0.1))
        .bg(theme.group_box)
        .leading_sm()
}

/// Un campo con su rótulo encima (`Label`: `text-sm font-medium`), a
/// `space-y-2`.
fn field(label: String, input: impl IntoElement) -> Div {
    // `leading-none`: el rótulo mide lo que su letra, no un renglón.
    div().flex().flex_col().gap_2().child(div().leading_sm().line_height(px(14.)).font_weight(FontWeight::MEDIUM).child(label)).child(input)
}

/// Un fallo, en su caja (`rounded-md bg-destructive/10 px-3 py-2 text-sm`).
/// Con identificador para que una prueba lo encuentre.
fn failure(id: &'static str, message: String, cx: &App) -> impl IntoElement {
    let danger = cx.theme().danger;
    div()
        .id(id)
        .test_support()
        .px_3()
        .py_2()
        .rounded(px(8.))
        .bg(danger.opacity(0.1))
        .leading_sm()
        .text_color(danger)
        .child(message)
}

/// Por dónde va quien pidió el enlace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forgot {
    Asking,
    /// Ya se pidió. No dice que haya cuenta: el servidor contesta lo mismo
    /// exista o no, y lo que se enseña tampoco lo distingue.
    Sent,
}

/// Entrar, y «olvidé mi contraseña» en la misma tarjeta.
pub struct SignIn {
    username: Entity<InputState>,
    password: Entity<InputState>,
    forgot_login: Entity<InputState>,
    /// `None` es el formulario de entrar.
    forgot: Option<Forgot>,
    busy: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl SignIn {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // El ejemplo del campo es el de `Login.tsx`: no se traduce, es un usuario.
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("admin"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let forgot_login = cx.new(|cx| InputState::new(window, cx));

        let on_enter = |this: &mut Self, _: &Entity<InputState>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                match this.forgot {
                    None => this.submit(window, cx),
                    Some(Forgot::Asking) => this.ask_for_link(cx),
                    Some(Forgot::Sent) => {}
                }
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&username, window, on_enter),
            cx.subscribe_in(&password, window, on_enter),
            cx.subscribe_in(&forgot_login, window, on_enter),
        ];
        username.update(cx, |state, cx| state.focus(window, cx));

        Self { username, password, forgot_login, forgot: None, busy: false, error: None, _subscriptions: subscriptions }
    }

    /// Lo escrito para entrar: usuario y contraseña.
    #[cfg(test)]
    pub fn typed(&self, cx: &App) -> (String, String) {
        (self.username.read(cx).value().to_string(), self.password.read(cx).value().to_string())
    }

    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let username = self.username.read(cx).value().to_string();
        let password = self.password.read(cx).value().to_string();
        match check_sign_in(&username, &password) {
            FormCheck::Incomplete => return,
            FormCheck::Problem { key } => {
                self.error = Some(tr(cx, &key));
                cx.notify();
                return;
            }
            FormCheck::Ready => {}
        }
        self.busy = true;
        self.error = None;
        cx.notify();

        let answer = Backend::ask(cx, move |core| async move { core.login(&username, &password).await.map(|_| ()) });
        self.when_answered(answer, "common:misc.authFailed", |_| {}, cx);
    }

    fn open_forgot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Lo que ya escribió para entrar es, casi siempre, por quien pregunta.
        let typed = self.username.read(cx).value();
        self.forgot_login.update(cx, |state, cx| {
            state.set_value(typed, window, cx);
            state.focus(window, cx);
        });
        self.forgot = Some(Forgot::Asking);
        self.error = None;
        cx.notify();
    }

    fn back_to_sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forgot = None;
        self.error = None;
        self.username.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn ask_for_link(&mut self, cx: &mut Context<Self>) {
        let login = self.forgot_login.read(cx).value().to_string();
        if self.busy || login.trim().is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();

        let language = locale(cx);
        let answer = Backend::ask(cx, move |core| async move { core.forgot_password(&login, language).await });
        self.when_answered(answer, "errors:account-failed", |this| this.forgot = Some(Forgot::Sent), cx);
    }

    /// Espera lo que diga el core y lo deja en pantalla.
    fn when_answered(
        &mut self,
        answer: tokio::sync::oneshot::Receiver<Result<(), ApiError>>,
        fallback_key: &'static str,
        on_ok: impl FnOnce(&mut Self) + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => on_ok(this),
                    Err(error) => this.error = Some(error_text(cx, &error, fallback_key)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// El formulario de entrar, con las medidas de `Login.tsx`
    /// (`space-y-4`), y debajo lo de entrar como invitado, que esta app
    /// todavía no tiene: se pinta en su sitio y lo dice al pasar el ratón.
    fn render_sign_in(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (muted, foreground, border, input, hover) = (theme.muted_foreground, theme.foreground, theme.border, theme.input, theme.muted);
        let not_yet = tr(cx, "common:misc.notInThisApp");
        let rule = move || div().h(px(1.)).flex_1().bg(border);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(field(tr(cx, "common:misc.username"), Input::new(&self.username).id("sign-in-username")))
                    .child(
                        field(
                            tr(cx, "common:misc.password"),
                            Input::new(&self.password).id("sign-in-password").mask_toggle(),
                        )
                        .child(
                            div().flex().child(
                                div()
                                    .id("sign-in-forgot")
                                    .test_support()
                                    .leading_xs()
                                    .text_color(muted)
                                    .cursor_pointer()
                                    .hover(move |style| style.underline().text_color(foreground))
                                    .on_click(cx.listener(|this, _, window, cx| this.open_forgot(window, cx)))
                                    .child(tr(cx, "common:account.forgot")),
                            ),
                        ),
                    )
                    .children(self.error.clone().map(|message| failure("sign-in-error", message, cx)))
                    .child(
                        Button::new("sign-in-submit")
                            .primary()
                            .w_full()
                            .rounded(px(10.))
                            .font_weight(FontWeight::MEDIUM)
                            .loading(self.busy)
                            .disabled(self.busy)
                            .label(tr(cx, if self.busy { "common:misc.signingIn" } else { "common:misc.signIn" }))
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
            .child(
                div()
                    .mt_4()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(rule())
                    .child(div().leading_xs().text_color(muted).child(tr(cx, "common:misc.orElse")))
                    .child(rule()),
            )
            .child(
                div()
                    .id("sign-in-guest")
                    .mt_4()
                    .w_full()
                    .h_8()
                    .flex()
                    .items_center()
                    .justify_center()
                    // `Button variant="outline"`: `rounded-lg` (10 px) y, en
                    // oscuro, `dark:bg-input/30`.
                    .rounded(px(10.))
                    .border_1()
                    .border_color(input)
                    .bg(crate::theme::field_background(cx))
                    .leading_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .hover(move |style| style.bg(hover))
                    .child(tr(cx, "common:misc.continueAsGuest"))
                    .tooltip(move |window, cx| Tooltip::new(not_yet.clone()).build(window, cx))
                    .test_support(),
            )
            .child(div().mt_2().leading_xs().text_center().text_color(muted).child(tr(cx, "common:misc.guestNote")))
    }

    fn render_forgot(&self, step: Forgot, cx: &mut Context<Self>) -> Div {
        let (muted, sent_bg) = (cx.theme().muted_foreground, cx.theme().muted);
        let empty = self.forgot_login.read(cx).value().trim().is_empty();
        let mut form = div().flex().flex_col().gap_4().child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:account.forgotTitle")))
                .child(div().leading_sm().text_color(muted).child(tr(cx, "common:account.forgotLead"))),
        );
        form = match step {
            Forgot::Sent => form.child(
                div()
                    .id("forgot-sent")
                    .test_support()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .bg(sent_bg)
                    .leading_sm()
                    .child(tr(cx, "common:account.forgotSent")),
            ),
            Forgot::Asking => form.child(field(
                tr(cx, "common:account.forgotLogin"),
                Input::new(&self.forgot_login).id("forgot-login"),
            )),
        };
        form = form.children(self.error.clone().map(|message| failure("forgot-error", message, cx)));
        if step == Forgot::Asking {
            form = form.child(
                Button::new("forgot-send")
                    .primary()
                    .w_full()
                    .rounded(px(10.))
                    .font_weight(FontWeight::MEDIUM)
                    .loading(self.busy)
                    .disabled(self.busy || empty)
                    .label(tr(cx, "common:account.forgotSend"))
                    .on_click(cx.listener(|this, _, _, cx| this.ask_for_link(cx))),
            );
        }
        form.child(
            Button::new("forgot-back")
                .ghost()
                .w_full()
                .rounded(px(10.))
                .font_weight(FontWeight::MEDIUM)
                .label(tr(cx, "common:account.backToSignIn"))
                .on_click(cx.listener(|this, _, window, cx| this.back_to_sign_in(window, cx))),
        )
    }
}

impl Render for SignIn {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let form = match self.forgot {
            None => self.render_sign_in(cx),
            Some(step) => self.render_forgot(step, cx),
        };
        card(cx).child(
            card_body(cx)
                // `CardHeader` (`px-4`, lo suyo a `space-y-1`): la marca y
                // para qué se entra.
                .child(
                    div()
                        .px_4()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(crate::brand::brand(cx.theme().group_box, cx))
                        .child(div().leading_sm().text_color(muted).child(tr(cx, "common:misc.signInLead"))),
                )
                .child(div().px_4().child(form)),
        )
    }
}

/// Poner una contraseña propia. Sin salida: quien llega aquí no pasa a la app
/// hasta cambiarla.
pub struct NewPassword {
    current: Entity<InputState>,
    next: Entity<InputState>,
    confirm: Entity<InputState>,
    busy: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl NewPassword {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let masked = |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).masked(true));
        let (current, confirm) = (masked(window, cx), masked(window, cx));
        // La nueva dice en el propio campo cuánto tiene que medir.
        let hint = tr(cx, "common:admin.min8chars");
        let next = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(hint));

        // Lo que se reprocha depende de lo escrito, y no hace falta pedir que
        // se repinte: un campo que cambia ya invalida la ventana que lo pinta.
        let on_event = |this: &mut Self, _: &Entity<InputState>, event: &InputEvent, _: &mut Window, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit(cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&current, window, on_event),
            cx.subscribe_in(&next, window, on_event),
            cx.subscribe_in(&confirm, window, on_event),
        ];
        current.update(cx, |state, cx| state.focus(window, cx));

        Self { current, next, confirm, busy: false, error: None, _subscriptions: subscriptions }
    }

    fn check(&self, cx: &App) -> FormCheck {
        check_new_password(
            &self.current.read(cx).value(),
            &self.next.read(cx).value(),
            &self.confirm.read(cx).value(),
        )
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.check(cx) != FormCheck::Ready {
            return;
        }
        let current = self.current.read(cx).value().to_string();
        let next = self.next.read(cx).value().to_string();
        self.busy = true;
        self.error = None;
        cx.notify();

        let answer =
            Backend::ask(cx, move |core| async move { core.change_password(&current, &next).await.map(|_| ()) });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                if let Err(error) = result {
                    this.error = Some(error_text(cx, &error, "common:admin.changeFailed"));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for NewPassword {
    /// La de `ForcedChangePassword` en `ChangePassword.tsx`: una caja con
    /// borde y sin fondo (`max-w-sm space-y-4 rounded-xl border p-6`), la
    /// llave junto al título, y el formulario a `space-y-3` con cada campo a
    /// `space-y-1.5`. Lo que no vale de lo escrito se dice bajo el último
    /// campo, en pequeño; lo que contestó el servidor, sobre el botón.
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, danger) = (theme.muted_foreground, theme.border, theme.danger);
        let check = self.check(cx);
        let complaint = match &check {
            FormCheck::Problem { key } => Some(tr(cx, key)),
            _ => None,
        };
        let refused = if complaint.is_none() { self.error.clone() } else { None };
        let field = |label: String, input: Input| div().flex().flex_col().gap_1p5().child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(label)).child(input);
        card(cx).child(
            div()
                .w(px(CARD_WIDTH))
                .flex()
                .flex_col()
                .gap_4()
                .p_6()
                .rounded(px(14.))
                .border_1()
                .border_color(border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::new(Lucide::KeyRound).size_5().flex_none().text_color(muted))
                        .child(div().leading_lg().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:admin.setNewPassword"))),
                )
                .child(div().leading_sm().text_color(muted).child(tr(cx, "common:admin.setByAdmin")))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(field(tr(cx, "common:admin.currentPassword"), Input::new(&self.current).id("new-password-current").mask_toggle()))
                        .child(field(tr(cx, "common:admin.newPassword"), Input::new(&self.next).id("new-password-next").mask_toggle()))
                        .child(
                            field(tr(cx, "common:admin.confirmNewPassword"), Input::new(&self.confirm).id("new-password-confirm").mask_toggle())
                                .children(complaint.map(|message| div().id("new-password-error").test_support().leading_xs().text_color(danger).child(message))),
                        )
                        .children(refused.map(|message| div().id("new-password-refused").test_support().leading_sm().text_color(danger).child(message)))
                        .child(
                            Button::new("new-password-submit")
                                .primary()
                                .w_full()
                                .rounded(px(10.))
                                .font_weight(FontWeight::MEDIUM)
                                .loading(self.busy)
                                .disabled(self.busy || check != FormCheck::Ready)
                                .label(tr(cx, if self.busy { "common:admin.saving" } else { "common:admin.changePassword" }))
                                .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                        ),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::{Forgot, NewPassword, SignIn};
    use crate::testing::{wait_until, FakeServer};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        px, size, AnyWindowHandle, AppContext, Bounds, Context, Entity, Point, Render, TestAppContext, Window,
        WindowBounds, WindowOptions,
    };
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, ResponseTemplate};

    fn open<V: Render + 'static>(
        cx: &mut TestAppContext,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
    ) -> (AnyWindowHandle, Entity<V>) {
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(760.)),
                })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| build(window, cx))).expect("open test window")
        })
    }

    fn fill(window: AnyWindowHandle, cx: &mut TestAppContext, fields: &[(&'static str, &str)]) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            for (id, text) in fields {
                window.click(*id, cx);
                window.input(text, cx);
            }
        })
        .unwrap();
    }

    fn click(window: AnyWindowHandle, cx: &mut TestAppContext, id: &'static str) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    fn press(window: AnyWindowHandle, cx: &mut TestAppContext, key: &str) {
        cx.update_window(window, |_, window, cx| window.press(key, cx)).unwrap();
    }

    fn refusal(status: u16, code: &str) -> ResponseTemplate {
        ResponseTemplate::new(status)
            .set_body_json(json!({ "success": false, "message": "From the server", "error": code }))
    }

    /// Una contraseña no se pinta tal como se escribe, en ninguno de los
    /// formularios.
    ///
    /// Rotura que atrapa: un campo de contraseña a la vista de quien mire la
    /// pantalla (una llamada compartida, alguien detrás).
    #[gpui_kit::test]
    fn a_password_is_never_shown_as_typed(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        server.install(cx);
        let (_, sign_in) = open(cx, SignIn::new);
        let (_, new_password) = open(cx, NewPassword::new);

        cx.update(|cx| {
            let sign_in = sign_in.read(cx);
            assert!(sign_in.password.read(cx).presentation().is_masked());
            assert!(!sign_in.username.read(cx).presentation().is_masked(), "el usuario sí se lee");
            let new_password = new_password.read(cx);
            for field in [&new_password.current, &new_password.next, &new_password.confirm] {
                assert!(field.read(cx).presentation().is_masked());
            }
        });
    }

    /// La pantalla de entrada lleva la marca.
    ///
    /// Rotura que atrapa: una pantalla de entrada sin nada que diga de qué
    /// app es.
    #[gpui_kit::test]
    fn the_sign_in_screen_carries_the_brand(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        server.install(cx);
        let (window, _) = open(cx, SignIn::new);

        let shown = cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("brand").is_some()
        });
        assert!(shown.unwrap());
    }

    /// Un login que el servidor rechaza dice por qué, en el idioma de quien
    /// lo lee, y deja volver a intentarlo.
    ///
    /// Roturas que atrapa: tragarse el rechazo (la pantalla se queda igual y
    /// nadie sabe qué pasó); enseñar la etiqueta cruda o la frase en inglés
    /// del servidor; dejar el botón girando para siempre.
    #[gpui_kit::test]
    fn a_refused_sign_in_says_why(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        server.mount(
            Mock::given(method("POST")).and(path("/api/v1/auth/login")).respond_with(refusal(401, "invalid-credentials")),
        );
        server.install(cx);
        let (window, view) = open(cx, SignIn::new);

        fill(window, cx, &[("sign-in-username", "ana"), ("sign-in-password", "no-es-esta")]);
        // Con Intro, como lo manda casi todo el mundo.
        press(window, cx, "enter");

        wait_until(cx, "el rechazo llega a la pantalla", |cx| cx.update(|cx| view.read(cx).error.is_some()));
        let (error, busy) = cx.update(|cx| (view.read(cx).error.clone(), view.read(cx).busy));
        assert_eq!(error.as_deref(), Some("Falló la autenticación"));
        assert!(!busy, "el botón se quedó esperando");
        let shown = cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("sign-in-error").is_some()
        });
        assert!(shown.unwrap(), "el fallo no está en pantalla");
    }

    /// Lo que el servidor va a rechazar no se manda, y se dice.
    ///
    /// Roturas que atrapa: gastar un intento de login (hay un tope por cuenta
    /// y por IP) con una contraseña que no puede ser; mandar un formulario
    /// vacío.
    #[gpui_kit::test]
    fn what_the_server_would_refuse_is_not_sent(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        server.install(cx);
        let (window, view) = open(cx, SignIn::new);

        click(window, cx, "sign-in-submit");
        assert_eq!(cx.update(|cx| view.read(cx).error.clone()), None);

        fill(window, cx, &[("sign-in-username", "ana"), ("sign-in-password", "corta")]);
        click(window, cx, "sign-in-submit");
        cx.run_until_parked();

        assert_eq!(
            cx.update(|cx| view.read(cx).error.clone()).as_deref(),
            Some("La contraseña tiene que tener al menos 8 caracteres")
        );
        assert!(!cx.update(|cx| view.read(cx).busy));
        assert_eq!(server.requests().len(), 0, "se mandó algo al servidor");
    }

    /// «¿Olvidaste tu contraseña?» pide el enlace para quien ya se escribió,
    /// y dice lo mismo exista o no la cuenta.
    ///
    /// Roturas que atrapa: hacer escribir el usuario dos veces; que el botón
    /// no pida nada; pedir el correo en un idioma que no es el de la pantalla;
    /// no decir que se pidió; no poder volver a entrar.
    #[gpui_kit::test]
    fn forgetting_the_password_asks_for_the_link(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/public/account/forgot"))
                .and(body_json(json!({ "login": "ana" })))
                .and(header("Accept-Language", "es"))
                .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "success": true }))),
        );
        server.install(cx);
        let (window, view) = open(cx, SignIn::new);
        fill(window, cx, &[("sign-in-username", "ana")]);

        click(window, cx, "sign-in-forgot");
        assert_eq!(cx.update(|cx| view.read(cx).forgot), Some(Forgot::Asking));
        assert_eq!(cx.update(|cx| view.read(cx).forgot_login.read(cx).value().to_string()), "ana");

        click(window, cx, "forgot-send");
        wait_until(cx, "dice que el enlace se pidió", |cx| {
            cx.update(|cx| view.read(cx).forgot == Some(Forgot::Sent))
        });
        assert_eq!(server.hits("/api/v1/public/account/forgot"), 1);
        let (sent, send_again) = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                (window.try_find("forgot-sent").is_some(), window.try_find("forgot-send").is_some())
            })
            .unwrap();
        assert!(sent, "no dice que se pidió");
        assert!(!send_again, "deja pedirlo otra vez sin más");

        click(window, cx, "forgot-back");
        assert_eq!(cx.update(|cx| view.read(cx).forgot), None);
    }

    /// Un «demasiados intentos» al pedir el enlace se dice, y no se da por
    /// pedido.
    ///
    /// Rotura que atrapa: decir «te llegó un enlace» cuando el servidor no
    /// mandó ninguno.
    #[gpui_kit::test]
    fn a_refused_link_request_is_not_reported_as_sent(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/public/account/forgot"))
                .respond_with(refusal(429, "rate-limited")),
        );
        server.install(cx);
        let (window, view) = open(cx, SignIn::new);
        fill(window, cx, &[("sign-in-username", "ana")]);
        click(window, cx, "sign-in-forgot");
        click(window, cx, "forgot-send");

        wait_until(cx, "el rechazo llega", |cx| cx.update(|cx| view.read(cx).error.is_some()));
        assert_eq!(cx.update(|cx| view.read(cx).forgot), Some(Forgot::Asking));
        assert_eq!(
            cx.update(|cx| view.read(cx).error.clone()),
            Some(cac_core::text(cac_core::Locale::Es, "errors:rate-limited"))
        );
    }

    /// El cambio de contraseña reprocha lo que no vale y no lo manda.
    ///
    /// Roturas que atrapa: mandar una repetición que no coincide; no decir
    /// por qué el botón no hace nada; que un rechazo del servidor (la actual
    /// no es ésa) no llegue a la pantalla.
    #[gpui_kit::test]
    fn a_new_password_that_does_not_hold_is_not_sent(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/change-password"))
                .respond_with(refusal(400, "invalid-credentials")),
        );
        server.install(cx);
        cx.update(|cx| {
            let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana", "mustChangePassword": true }));
            crate::backend::Backend::core(cx).sign_in(ana.unwrap(), "access".into(), "refresh".into()).unwrap();
        });
        let (window, view) = open(cx, NewPassword::new);

        fill(
            window,
            cx,
            &[
                ("new-password-current", "la-de-antes"),
                ("new-password-next", "la-mia-propia"),
                ("new-password-confirm", "la-mia-propio"),
            ],
        );
        let complaint = cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("new-password-error").is_some()
        });
        assert!(complaint.unwrap(), "no reprocha la repetición");
        press(window, cx, "enter");
        click(window, cx, "new-password-submit");
        cx.run_until_parked();
        assert_eq!(server.requests().len(), 0, "se mandó una repetición que no coincide");

        // Corregida la repetición, se manda; y lo que conteste el servidor
        // llega a la pantalla.
        press(window, cx, "backspace");
        cx.update_window(window, |_, window, cx| window.input("a", cx)).unwrap();
        click(window, cx, "new-password-submit");
        wait_until(cx, "el rechazo del servidor llega", |cx| cx.update(|cx| view.read(cx).error.is_some()));
        assert_eq!(server.hits("/api/v1/auth/change-password"), 1);
        assert_eq!(cx.update(|cx| view.read(cx).error.clone()).as_deref(), Some("Falló el cambio"));
        let (refused, complaint) = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                (window.try_find("new-password-refused").is_some(), window.try_find("new-password-error").is_some())
            })
            .unwrap();
        assert!(refused, "el rechazo del servidor no está en pantalla");
        assert!(!complaint, "reprocha lo escrito cuando lo que falló fue el servidor");
    }

    /// Entrar como invitado, que la app de siempre ofrece y ésta todavía no,
    /// está pintado en su sitio; y no sale al pedir el enlace de la contraseña.
    ///
    /// Roturas que atrapa: dejarlo fuera por no tener aún qué hacer con él
    /// (la tarjeta se queda más corta que la de la otra app); arrastrarlo
    /// al paso de «olvidé mi contraseña», donde no pinta nada.
    #[gpui_kit::test]
    fn guest_access_is_painted_only_on_the_sign_in_form(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        server.install(cx);
        let (window, _) = open(cx, SignIn::new);
        let shown = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("sign-in-guest").is_some()
            })
            .unwrap()
        };
        assert!(shown(cx), "falta entrar como invitado");

        click(window, cx, "sign-in-forgot");
        assert!(!shown(cx), "sigue ofreciéndolo al pedir el enlace");
    }
}
