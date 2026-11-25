//! Lo tuyo, desde el menú de la cuenta: el perfil (`Profile.tsx`) y cambiar
//! la contraseña (`ChangePasswordDialog` en `ChangePassword.tsx`).
//!
//! Qué se manda, cuándo se puede mandar y qué pasa con la sesión después lo
//! decide el core (`account.rs`). Aquí se recoge lo escrito y se pinta.

use std::rc::Rc;

use cac_core::{check_new_password, FormCheck, PasswordChanged};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::*;

use crate::backend::{Backend, Snapshot};
use crate::lang::{error_text, tr};
use crate::theme::Leading as _;

/// El ancho del perfil: `max-w-lg`.
const PROFILE_WIDTH: f32 = 512.;
/// El ancho del diálogo: el `sm:max-w-lg` de `DialogContent`.
const DIALOG_WIDTH: f32 = 512.;

/// La pantalla que se tiene delante, para saber si algo puso otra sin
/// pasar por la app: un aviso que abre una tarea, un agente que trae «Por
/// atender». Lo que se pide a mano (el riel, el árbol) cierra el perfil al
/// pedirlo, aunque lleve adonde ya se estaba.
///
/// Sólo qué pantalla, no qué hay en ella: que alguien renombre el nodo que
/// se miraba no es irse a otro sitio.
#[derive(Clone, Debug, PartialEq)]
pub struct Whereabouts {
    comms: bool,
    platform: bool,
    docs_module: bool,
    doc: bool,
    requests: bool,
    task: bool,
}

impl Whereabouts {
    pub fn of(snapshot: &Snapshot) -> Self {
        Self {
            comms: snapshot.comms.on_screen,
            platform: snapshot.platform.on_screen,
            docs_module: snapshot.docs.in_module,
            doc: snapshot.docs.target.is_some(),
            requests: snapshot.requests.on_screen,
            task: snapshot.task.task_id.is_some(),
        }
    }
}

/// Lo que se dice junto a la cuenta después de hacer algo desde su menú.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountNotice {
    /// La clave del catálogo.
    pub key: &'static str,
    /// Si se va sola. Lo que pide hacer algo se queda hasta que se quita.
    pub fades: bool,
}

impl AccountNotice {
    pub fn password(outcome: PasswordChanged) -> Self {
        match outcome {
            PasswordChanged::Done => Self { key: "common:admin.passwordChanged", fades: true },
            PasswordChanged::NotStored => Self { key: "common:admin.passwordChangedNotStored", fades: false },
        }
    }
}

/// Un campo con su rótulo (`Label`, a `space-y-1.5`) y, debajo, su ayuda.
fn field(label: String, input: impl IntoElement, help: Option<String>, muted: Hsla) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(label))
        .child(input)
        .children(help.map(|help| div().leading_xs().text_color(muted).child(help)))
}

/// Tus propios datos: cómo te llamas y tu correo. El usuario se enseña y no
/// se edita.
pub struct ProfileView {
    name: Entity<InputState>,
    email: Entity<InputState>,
    busy: bool,
    saved: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProfileView {
    /// Con lo que ya tienes puesto: la sesión está cargada, y pedirla otra
    /// vez sólo enseñaría los campos vacíos un momento.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session = Backend::snapshot(cx).read(cx).session.clone();
        let (name, email) = session.map(|session| (session.name.unwrap_or_default(), session.email.unwrap_or_default())).unwrap_or_default();
        let placeholder = tr(cx, "nav:profile.namePlaceholder");
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(name));
        let email = cx.new(|cx| InputState::new(window, cx).default_value(email));
        let on_event = |this: &mut Self, _: &Entity<InputState>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>| match event {
            InputEvent::PressEnter { .. } => this.save(window, cx),
            InputEvent::Change => {
                // Lo de «guardado» es de lo que se guardó, no de lo que se
                // está escribiendo ahora.
                this.saved = false;
                cx.notify();
            }
            _ => {}
        };
        let subscriptions = vec![cx.subscribe_in(&name, window, on_event), cx.subscribe_in(&email, window, on_event)];
        name.update(cx, |state, cx| state.focus(window, cx));
        Self { name, email, busy: false, saved: false, error: None, _subscriptions: subscriptions }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let name = self.name.read(cx).value().to_string();
        let email = self.email.read(cx).value().to_string();
        self.busy = true;
        self.saved = false;
        self.error = None;
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move { core.update_profile(&name, &email).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(session) => {
                        // Con lo que quedó guardado, como la otra app: sin los
                        // espacios que se colaron.
                        let (name, email) = (session.name.unwrap_or_default(), session.email.unwrap_or_default());
                        this.name.update(cx, |field, cx| field.set_value(name, window, cx));
                        this.email.update(cx, |field, cx| field.set_value(email, window, cx));
                        this.saved = true;
                    }
                    Err(error) => this.error = Some(error_text(cx, &error, "nav:profile.errSave")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for ProfileView {
    /// La de `Profile.tsx`: `p-6` y una columna `max-w-lg` centrada a
    /// `space-y-5`; el icono junto al título (`text-xl font-semibold`), la
    /// entradilla, cada campo con su ayuda, y el usuario en una caja con
    /// borde y letra de máquina.
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, field_bg, danger, success, foreground, background, mono) = (
            theme.muted_foreground,
            theme.border,
            theme.muted.opacity(0.4),
            theme.danger,
            theme.success,
            theme.foreground,
            theme.background,
            theme.mono_font_family.clone(),
        );
        let username = Backend::snapshot(cx).read(cx).session.as_ref().map(|session| session.username.clone()).unwrap_or_default();
        div()
            .id("profile")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .bg(background)
            .text_color(foreground)
            .p_6()
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(PROFILE_WIDTH))
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(Lucide::UserRound).size_5().flex_none().text_color(muted))
                            .child(div().text_xl().line_height(rems(1.75)).font_weight(FontWeight::SEMIBOLD).child(tr(cx, "nav:profile.title"))),
                    )
                    .child(div().leading_sm().text_color(muted).child(tr(cx, "nav:profile.lead")))
                    .child(field(tr(cx, "nav:profile.name"), Input::new(&self.name).id("profile-name"), Some(tr(cx, "nav:profile.nameHelp")), muted))
                    .child(field(tr(cx, "nav:profile.email"), Input::new(&self.email).id("profile-email"), Some(tr(cx, "nav:profile.emailHelp")), muted))
                    .child(field(
                        tr(cx, "nav:profile.username"),
                        div()
                            .id("profile-username")
                            .test_support()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(border)
                            .bg(field_bg)
                            .px_3()
                            .py_2()
                            .leading_sm()
                            .font_family(mono)
                            .child(username),
                        Some(tr(cx, "nav:profile.usernameHelp")),
                        muted,
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new("profile-save")
                                    .primary()
                                    .loading(self.busy)
                                    .disabled(self.busy)
                                    .label(tr(cx, if self.busy { "nav:profile.saving" } else { "nav:profile.save" }))
                                    .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                            )
                            // Esta app no tiene avisos flotantes: lo que en la
                            // otra es un toast se dice junto al botón.
                            .children(self.saved.then(|| {
                                div().id("profile-saved").test_support().leading_sm().text_color(success).child(tr(cx, "nav:profile.saved"))
                            })),
                    )
                    .children(self.error.clone().map(|error| {
                        // Sin frase propia del fallo, el detalle sería el
                        // mismo título otra vez.
                        let title = tr(cx, "nav:profile.errSave");
                        let detail = (error != title).then_some(error);
                        div()
                            .id("profile-error")
                            .test_support()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .leading_sm()
                            .text_color(danger)
                            .child(title)
                            .children(detail.map(|detail| div().leading_xs().child(detail)))
                    })),
            )
    }
}

/// Se pidió cerrar el diálogo, o la contraseña ya se cambió: quien lo tiene
/// lo quita.
pub struct PasswordClosed;

/// Cambiar la contraseña porque quieres, desde el menú de la cuenta. La
/// forzada, la que puso otro, es `auth::NewPassword`: la misma regla y la
/// misma llamada, sin salida.
pub struct PasswordDialog {
    current: Entity<InputState>,
    next: Entity<InputState>,
    confirm: Entity<InputState>,
    busy: bool,
    error: Option<String>,
    focus: FocusHandle,
    /// A quién decirle que se cambió. No a este diálogo: pudo cerrarse
    /// mientras la petición estaba en camino.
    changed: Rc<dyn Fn(PasswordChanged, &mut App)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PasswordClosed> for PasswordDialog {}

impl Focusable for PasswordDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PasswordDialog {
    pub fn new(changed: impl Fn(PasswordChanged, &mut App) + 'static, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let masked = |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).masked(true));
        let (current, confirm) = (masked(window, cx), masked(window, cx));
        let hint = tr(cx, "common:admin.min8chars");
        let next = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(hint));
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
        Self { current, next, confirm, busy: false, error: None, focus: cx.focus_handle(), changed: Rc::new(changed), _subscriptions: subscriptions }
    }

    fn check(&self, cx: &App) -> FormCheck {
        check_new_password(&self.current.read(cx).value(), &self.next.read(cx).value(), &self.confirm.read(cx).value())
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
        let answer = Backend::ask(cx, move |core| async move { core.change_password(&current, &next).await });
        let changed = self.changed.clone();
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            if let Ok(outcome) = &result {
                let outcome = *outcome;
                cx.update(|cx| changed(outcome, cx));
            }
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(_) => cx.emit(PasswordClosed),
                    Err(error) => this.error = Some(error_text(cx, &error, "common:admin.changeFailed")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for PasswordDialog {
    /// El `Dialog` de la app de siempre: título y descripción, la X en su
    /// esquina, y el mismo formulario que la contraseña forzada, con el botón
    /// a todo lo ancho.
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, popover, border, foreground, muted, danger) =
            (theme.background, theme.popover, theme.border, theme.foreground, theme.muted_foreground, theme.danger);
        let check = self.check(cx);
        let complaint = match &check {
            FormCheck::Problem { key } => Some(tr(cx, key)),
            _ => None,
        };
        let refused = if complaint.is_none() { self.error.clone() } else { None };
        let masked = |label: String, state: &Entity<InputState>, id: &'static str| field(label, Input::new(state).id(id).mask_toggle(), None, muted);
        let dialog = div()
            .id("password-dialog")
            .test_support()
            .relative()
            .w(px(DIALOG_WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .rounded(px(10.))
            .border_1()
            .border_color(border)
            .bg(popover)
            .text_color(foreground)
            .shadow_lg()
            .child(
                div()
                    .absolute()
                    .top(px(12.))
                    .right(px(12.))
                    .child(
                        Button::new("password-close")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::X))
                            .tooltip(tr(cx, "common:action.close"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(PasswordClosed))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(div().leading_lg().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:admin.changePassword")))
                    .child(div().leading_sm().text_color(muted).child(tr(cx, "common:admin.enterCurrentAndNew"))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(masked(tr(cx, "common:admin.currentPassword"), &self.current, "password-current"))
                    .child(masked(tr(cx, "common:admin.newPassword"), &self.next, "password-next"))
                    .child(
                        masked(tr(cx, "common:admin.confirmNewPassword"), &self.confirm, "password-confirm")
                            .children(complaint.map(|message| div().id("password-error").test_support().leading_xs().text_color(danger).child(message))),
                    )
                    .children(refused.map(|message| div().id("password-refused").test_support().leading_sm().text_color(danger).child(message)))
                    .child(
                        Button::new("password-submit")
                            .primary()
                            .w_full()
                            .loading(self.busy)
                            .disabled(self.busy || check != FormCheck::Ready)
                            .label(tr(cx, if self.busy { "common:admin.saving" } else { "common:admin.changePassword" }))
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            );
        div()
            .size_full()
            .track_focus(&self.focus)
            .occlude()
            // Las teclas de la app (⌘B, ⌘1–⌘9) no siguen hasta la pantalla de
            // detrás. No todas: cortar una tecla aquí también corta que se
            // escriba, y AltGr (Ctrl+Alt en Linux) es como se escribe la @.
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                let key = &event.keystroke.key;
                let held = event.keystroke.modifiers;
                let held = cac_core::KeysHeld { secondary: held.secondary(cx), shift: held.shift, alt: held.alt };
                if key == "escape" {
                    cx.emit(PasswordClosed);
                    cx.stop_propagation();
                } else if cac_core::folds_panel(key, held) || cac_core::rail_slot(key, held).is_some() {
                    cx.stop_propagation();
                }
            }))
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(background.opacity(0.7))
            .child(dialog)
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use crate::backend::Backend;
    use crate::testing::{ok, server_with_tasks, sign_in, wait_until, Gate};
    use crate::workspace::{Workspace, RAIL_WORK};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, SharedString, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(760.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window")
        })
    }

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn click(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    fn press(window: AnyWindowHandle, key: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| window.press(key, cx)).unwrap();
        cx.run_until_parked();
    }

    fn type_in(window: AnyWindowHandle, id: &str, text: &str, cx: &mut TestAppContext) {
        let id = SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
            window.input(text, cx);
        })
        .unwrap();
    }

    /// Del menú de la cuenta, con el árbol ya cargado.
    fn from_the_menu(window: AnyWindowHandle, row: &str, cx: &mut TestAppContext) {
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "account-menu", cx);
        wait_until(cx, "el menú se abre", |cx| on_screen(window, row, cx));
        click(window, row, cx);
        cx.run_until_parked();
        assert!(!on_screen(window, "account-log-out", cx), "el menú se quedó abierto");
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El perfil llega con lo que ya tienes puesto, guarda sin mandar el
    /// usuario, se rellena con lo guardado, y el nombre nuevo es el de la
    /// sesión sin reabrir la app.
    ///
    /// Roturas que atrapa: una entrada del menú que no abre nada; abrir el
    /// perfil con los campos vacíos; mandar el usuario; guardar y seguir
    /// viendo el nombre de antes en el menú; dejar en el campo lo escrito con
    /// sus espacios en vez de lo guardado; no decir que se guardó, o seguir
    /// diciéndolo cuando ya se ha vuelto a escribir.
    #[gpui_kit::test]
    fn the_profile_saves_your_name_and_the_menu_shows_it_at_once(cx: &mut TestAppContext) {
        let server = server_with_tasks(vec![]);
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/auth/me")).respond_with(ok(json!(null))));
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/auth/me"))
                .respond_with(ok(json!({ "id": "u1", "username": "ana", "name": "Ana García", "email": "ana@cac.dev" }))),
        );
        server.install(cx);
        let (window, workspace) = open(cx);

        from_the_menu(window, "account-profile", cx);
        assert!(on_screen(window, "profile", cx), "el perfil no se abre");
        let profile = cx.update(|cx| workspace.read(cx).profile.as_ref().map(|(view, _)| view.clone())).unwrap();
        assert_eq!(cx.update(|cx| profile.read(cx).name.read(cx).value().to_string()), "Ana Ruiz");

        cx.update_window(window, |_, window, cx| {
            profile.update(cx, |profile, cx| {
                profile.name.update(cx, |name, cx| name.set_value("Ana García", window, cx));
                profile.email.update(cx, |email, cx| email.set_value("  ana@cac.dev ", window, cx));
            })
        })
        .unwrap();
        click(window, "profile-save", cx);
        wait_until(cx, "se guarda", |cx| on_screen(window, "profile-saved", cx));

        let sent = server.requests().into_iter().find(|request| request.method.as_str() == "PATCH").expect("no se mandó nada");
        let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(body, json!({ "name": "Ana García", "email": "ana@cac.dev" }));
        let name = cx.update(|cx| Backend::snapshot(cx).read(cx).session.as_ref().and_then(|session| session.name.clone()));
        assert_eq!(name.as_deref(), Some("Ana García"), "el menú sigue con el nombre de antes");
        assert_eq!(cx.update(|cx| profile.read(cx).email.read(cx).value().to_string()), "ana@cac.dev", "el campo no se rellenó con lo guardado");

        type_in(window, "profile-name", "!", cx);
        assert!(!on_screen(window, "profile-saved", cx), "dice «guardado» de algo que se está escribiendo");
        stop(cx);
    }

    /// El perfil se va en cuanto se pide otra cosa, aunque lleve adonde ya
    /// se estaba, y devuelve el foco a la app; con él delante el riel no
    /// marca ningún módulo.
    ///
    /// Roturas que atrapa: un «Trabajo» que pliega el panel en vez de salir
    /// del perfil; pulsar en el árbol el nodo que ya estaba elegido y seguir
    /// en el perfil; un perfil que tapa «Por atender» cuando un aviso lo pone
    /// delante; un riel que marca «Trabajo» con el perfil a la vista; dejar
    /// el foco en un campo que ya no se pinta (las teclas no llegarían a
    /// nada).
    #[gpui_kit::test]
    fn the_profile_goes_away_when_you_look_elsewhere(cx: &mut TestAppContext) {
        let server = server_with_tasks(vec![]);
        server.install(cx);
        let (window, workspace) = open(cx);
        let in_profile = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).profile.is_some());
        let work_lit = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).module_active(RAIL_WORK, cx));
        let anchored = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| workspace.read(cx).focus_anchor.is_focused(window)).unwrap()
        };

        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        assert!(work_lit(cx));
        from_the_menu(window, "account-profile", cx);
        assert!(in_profile(cx));
        assert!(!work_lit(cx), "el riel marca «Trabajo» con el perfil delante");
        assert!(!anchored(cx), "el perfil no se lleva el foco a su campo");
        click(window, "rail-work", cx);
        cx.run_until_parked();
        assert!(!in_profile(cx) && !on_screen(window, "profile", cx), "«Trabajo» no saca del perfil");
        assert!(cx.update(|cx| workspace.read(cx).panel_pinned), "«Trabajo» desde el perfil plegó el panel");
        assert!(work_lit(cx));
        assert!(anchored(cx), "el foco se quedó en un campo que ya no se pinta");

        // El nodo que ya estaba elegido: no cambia nada de lo que se mira, y
        // aun así es irse del perfil.
        click(window, "tree-a", cx);
        wait_until(cx, "se acota al espacio", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).my_work.scope.is_some()));
        from_the_menu(window, "account-profile", cx);
        click(window, "tree-a", cx);
        cx.run_until_parked();
        assert!(!in_profile(cx), "pulsar en el árbol no saca del perfil");

        from_the_menu(window, "account-profile", cx);
        assert!(on_screen(window, "profile", cx));
        cx.update(|cx| Backend::core(cx).requests().show(None));
        wait_until(cx, "«Por atender» lo cierra", |cx| !on_screen(window, "profile", cx));
        assert!(!in_profile(cx));
        stop(cx);
    }

    /// «Cambiar contraseña» del menú: no manda lo que no vale, dice lo que
    /// rechaza el servidor, se queda con las teclas mientras está abierto, y
    /// al cambiarla se cierra con la sesión nueva y lo dice.
    ///
    /// Roturas que atrapa: una entrada que no abre nada; mandar una
    /// repetición que no coincide; un rechazo del servidor (la actual no es
    /// ésa) que no llega a la pantalla o que cierra el diálogo como si se
    /// hubiera cambiado; quedarse con el par revocado; cambiarla sin decirlo;
    /// que ⌘B o ⌘2 obren en la pantalla de detrás; un Escape que no cierra.
    #[gpui_kit::test]
    fn changing_the_password_from_the_menu_keeps_you_in(cx: &mut TestAppContext) {
        let server = server_with_tasks(vec![]);
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/change-password"))
                .respond_with(
                    ResponseTemplate::new(400).set_body_json(json!({ "success": false, "message": "From the server", "error": "invalid-credentials" })),
                )
                .up_to_n_times(1)
                .with_priority(1),
        );
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/change-password"))
                .respond_with(ok(json!({ "accessToken": "new-access", "refreshToken": "new-refresh" }))),
        );
        server.mount(Mock::given(method("GET")).and(path("/api/v1/auth/me")).respond_with(ok(json!({ "id": "u1", "username": "ana", "name": "Ana Ruiz" }))));
        server.install(cx);
        let (window, workspace) = open(cx);
        let dialog_open = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).password.is_some());

        from_the_menu(window, "account-password", cx);
        assert!(on_screen(window, "password-dialog", cx), "el diálogo no se abre");
        let dialog = cx.update(|cx| workspace.read(cx).password.as_ref().map(|(dialog, _)| dialog.clone())).unwrap();

        // Las teclas de la app no pasan del diálogo.
        press(window, "secondary-b", cx);
        press(window, "secondary-2", cx);
        assert!(cx.update(|cx| workspace.read(cx).panel_pinned), "⌘B plegó el panel de detrás");
        assert!(cx.update(|cx| workspace.read(cx).module_active(RAIL_WORK, cx)), "⌘2 cambió de módulo detrás");
        assert!(dialog_open(cx));

        type_in(window, "password-current", "la-de-antes", cx);
        type_in(window, "password-next", "la-mia-propia", cx);
        type_in(window, "password-confirm", "la-mia-propio", cx);
        assert!(on_screen(window, "password-error", cx), "no reprocha la repetición");
        press(window, "enter", cx);
        click(window, "password-submit", cx);
        // Mandar pone el diálogo a esperar en el acto: si no lo está, no se
        // mandó nada, sin depender de cuándo conteste el servidor.
        assert!(!cx.update(|cx| dialog.read(cx).busy), "se mandó una repetición que no coincide");

        // Ahora coincide, y el servidor dice que la actual no es ésa.
        cx.update_window(window, |_, window, cx| {
            window.press("backspace", cx);
            window.input("a", cx);
        })
        .unwrap();
        click(window, "password-submit", cx);
        wait_until(cx, "el rechazo llega", |cx| on_screen(window, "password-refused", cx));
        assert!(dialog_open(cx), "un rechazo cerró el diálogo");
        assert!(!on_screen(window, "account-notice", cx), "un rechazo dice que se cambió");

        click(window, "password-submit", cx);
        wait_until(cx, "se cierra al cambiarla", |cx| !dialog_open(cx));
        assert_eq!(cx.update(|cx| Backend::core(cx).access_token()).as_deref(), Some("new-access"));
        assert!(!on_screen(window, "password-dialog", cx));
        assert!(on_screen(window, "account-notice", cx), "se cambió y no lo dice");
        click(window, "account-notice-close", cx);
        assert!(!on_screen(window, "account-notice", cx));

        // Otra vez: vacío, y Escape lo cierra.
        from_the_menu(window, "account-password", cx);
        let dialog = cx.update(|cx| workspace.read(cx).password.as_ref().map(|(dialog, _)| dialog.clone())).unwrap();
        assert!(cx.update(|cx| dialog.read(cx).current.read(cx).value().is_empty()), "sigue lo escrito la vez anterior");
        press(window, "escape", cx);
        assert!(!dialog_open(cx), "Escape no lo cierra");
        stop(cx);
    }

    /// Cerrar el diálogo con el cambio en camino no calla que se cambió.
    ///
    /// Rotura que atrapa: decirlo desde el diálogo, que ya no está cuando
    /// llega la respuesta (la contraseña cambia y nadie se entera).
    #[gpui_kit::test]
    fn a_password_changed_after_closing_the_dialog_is_still_said(cx: &mut TestAppContext) {
        let server = server_with_tasks(vec![]);
        let answer = Gate::default();
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/change-password"))
                .respond_with(answer.hold(ok(json!({ "accessToken": "new-access", "refreshToken": "new-refresh" })))),
        );
        server.mount(Mock::given(method("GET")).and(path("/api/v1/auth/me")).respond_with(ok(json!({ "id": "u1", "username": "ana" }))));
        server.install(cx);
        let (window, workspace) = open(cx);

        from_the_menu(window, "account-password", cx);
        type_in(window, "password-current", "la-de-antes", cx);
        type_in(window, "password-next", "la-mia-propia", cx);
        type_in(window, "password-confirm", "la-mia-propia", cx);
        click(window, "password-submit", cx);
        wait_until(cx, "el cambio está en camino", |_| answer.held() == 1);
        press(window, "escape", cx);
        assert!(cx.update(|cx| workspace.read(cx).password.is_none()), "Escape no lo cierra");

        answer.open();
        wait_until(cx, "se dice que se cambió", |cx| on_screen(window, "account-notice", cx));
        let said = cx.update(|cx| workspace.read(cx).account_notice.as_ref().map(|notice| notice.key));
        assert_eq!(said, Some("common:admin.passwordChanged"));
        stop(cx);
    }
}
