//! Un enlace de cac, pulsado o llegado de fuera, abre la app en su sitio.
//!
//! A qué pantalla lleva lo dice el core (`cac_core::app_link` y
//! `Core::open_link`); aquí queda lo que es de esta máquina: el navegador
//! para lo que la app no tiene, la pantalla de la organización (que es de la
//! interfaz, no del core) y decir que no se pudo abrir.
//!
//! Lo que llega de fuera —un `cac-desktop <enlace>`, o la segunda instancia
//! que le pasa el suyo a ésta (`single_instance.rs`)— trae la ventana al
//! frente y espera a que haya sesión y organización para abrirse.

use std::cell::RefCell;
use std::rc::Rc;

use cac_core::{AppLink, LinkOpened};
use gpui_kit::{App, Global, SharedString, SystemNotification};

use crate::backend::Backend;
use crate::lang::tr;

/// Quién abre la pantalla de la organización: el espacio de trabajo, que es
/// quien la pinta, se apunta al nacer.
struct OrganizationOpener(Rc<dyn Fn(&mut App)>);

impl Global for OrganizationOpener {}

/// La etiqueta del aviso que dice que un enlace no se pudo abrir. Una sola:
/// el siguiente sustituye al anterior.
const FAILED_TAG: &str = "link:failed";

pub(crate) fn on_open_organization(cx: &mut App, open: impl Fn(&mut App) + 'static) {
    cx.set_global(OrganizationOpener(Rc::new(open)));
}

pub(crate) fn open_organization(cx: &mut App) {
    if let Some(open) = cx.try_global::<OrganizationOpener>().map(|opener| opener.0.clone()) {
        open(cx);
    }
}

/// Lo que dijo el core al abrir un enlace, hecho en esta máquina.
///
/// Un fallo se dice con un aviso del sistema: el enlace puede venir de fuera
/// de la ventana (otra app, un aviso), y entonces no hay vista donde
/// escribirlo.
fn settle(opened: LinkOpened, cx: &mut App) {
    match opened {
        LinkOpened::Opened => {}
        LinkOpened::Organization => open_organization(cx),
        LinkOpened::Browser { url } => cx.open_url(&url),
        LinkOpened::Failed => cx.show_system_notification(SystemNotification {
            tag: SharedString::from(FAILED_TAG),
            title: SharedString::from(crate::identity::APP_NAME),
            body: tr(cx, "common:last.openLinkFailed").into(),
            actions: Vec::new(),
        }),
    }
}

/// Abre un enlace de cac donde lleva, sin preguntar qué es.
fn follow(href: String, cx: &mut App) {
    let answer = Backend::ask(cx, move |core| async move { core.open_link(&href).await });
    cx.spawn(async move |cx| {
        let Ok(opened) = answer.await else { return };
        cx.update(|cx| settle(opened, cx));
    })
    .detach();
}

/// Un enlace pulsado dentro de la app (una tarea, una nota, un pedido): lo
/// de cac se abre aquí, lo de fuera en el navegador, y una mención o algo
/// que no es un enlace no va a ningún sitio.
pub(crate) fn open(href: &str, cx: &mut App) {
    match Backend::core(cx).app_link(href) {
        AppLink::Browser { url } => cx.open_url(&url),
        AppLink::Unopenable => {}
        _ => follow(href.to_string(), cx),
    }
}

/// El enlace que trae un argumento de la línea de órdenes o el sistema
/// (`cac-desktop <enlace>`, el «abrir URL» de macOS). Sólo un `cac://`: eso lo
/// escribe cualquiera (en Windows `"%1"` llega entero en un argumento, traiga
/// lo que traiga), y otra cosa no se toma por un enlace que abrir.
pub(crate) fn launch_link(arg: &str) -> Option<String> {
    let arg = arg.trim();
    let scheme = arg.get(..6).is_some_and(|head| head.eq_ignore_ascii_case("cac://"));
    (scheme && arg.len() > 6).then(|| arg.to_string())
}

/// Un enlace que llega de fuera de la ventana. Con la sesión y la
/// organización ya cargadas se abre en el acto; si no, en cuanto lo estén
/// (al arrancar con un enlace, o con la pantalla de entrar delante).
pub(crate) fn open_from_outside(href: String, cx: &mut App) {
    crate::notifier::bring_to_front(cx);
    let snapshot = Backend::snapshot(cx);
    let ready = |cx: &App| {
        let snapshot = snapshot.read(cx);
        snapshot.session.is_some() && snapshot.orgs.current_org_id.is_some()
    };
    if ready(cx) {
        follow(href, cx);
        return;
    }
    let pending = Rc::new(RefCell::new(Some(href)));
    cx.observe(&snapshot, move |snapshot, cx| {
        let snapshot = snapshot.read(cx);
        if snapshot.session.is_none() || snapshot.orgs.current_org_id.is_none() {
            return;
        }
        if let Some(href) = pending.borrow_mut().take() {
            follow(href, cx);
        }
    })
    .detach();
}

/// Lo que manda la app que se lanzó con ésta ya abierta: su enlace, o
/// sólo traer la ventana al frente (relanzarla sin nada).
#[cfg(unix)]
pub(crate) fn from_another_launch(message: crate::single_instance::Message, cx: &mut App) {
    match message {
        crate::single_instance::Message::Open(link) => open_from_outside(link, cx),
        crate::single_instance::Message::Focus => crate::notifier::bring_to_front(cx),
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::Mock;

    use super::*;
    use crate::testing::*;
    use crate::workspace::Workspace;

    fn get(route: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET")).and(path(route.to_string()))
    }

    /// Dos organizaciones: la que se mira (`o1`, con el espacio `a`) y otra
    /// (`o2`, con `z`). La tarea `cac-12` (`t1`) es de la otra.
    fn server() -> FakeServer {
        let server = FakeServer::start();
        let org = |id: &str| json!({ "id": id, "name": format!("Org {id}"), "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z" });
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org("o1"), org("o2")]))));
        let space = |id: &str, org: &str| json!({ "id": id, "orgId": org, "name": format!("Espacio {id}"), "color": "#888", "kind": null, "folders": [], "lists": [], "people": [] });
        server.mount(get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(json!([space("a", "o1")]))));
        server.mount(get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "o2")]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([]))));
        let mut detail: serde_json::Value = serde_json::from_str(include_str!("../../core/tests/fixtures/task_detail.json")).unwrap();
        detail["task"]["id"] = json!("t1");
        detail["task"]["orgId"] = json!("o2");
        server.mount(get("/api/v1/tasks/by-ref/cac-12").respond_with(ok(detail.clone())));
        server.mount(get("/api/v1/tasks/t1").respond_with(ok(detail)));
        server.mount(get("/api/v1/tasks/by-ref/no-existe-1").respond_with(wiremock::ResponseTemplate::new(404)));
        server
    }

    fn open_window(cx: &mut TestAppContext) -> AnyWindowHandle {
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(760.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window").0
        })
    }

    fn current_org(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).orgs.current_org_id.clone())
    }

    fn task_open(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.clone())
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Sólo un `cac://` llega de fuera como enlace.
    ///
    /// Roturas que atrapa: tomar por enlace cualquier argumento (una ruta,
    /// una dirección web, una opción) que el sistema o alguien pase; no
    /// aceptar el esquema en mayúsculas; un `cac://` vacío.
    #[test]
    fn only_a_cac_link_comes_in_from_outside() {
        assert_eq!(launch_link(" cac://tasks?task=1 ").as_deref(), Some("cac://tasks?task=1"));
        assert_eq!(launch_link("CAC://dm?c=c1").as_deref(), Some("CAC://dm?c=c1"));
        for other in ["https://cac.guz-studio.dev/app/tasks?task=1", "/tasks?task=1", "--help", "cac:user/u1", "cac://", "", "ca"] {
            assert_eq!(launch_link(other), None, "{other:?}");
        }
    }

    /// Un enlace de cac pulsado dentro de la app —con su dirección de la
    /// web— abre la tarea aquí, en su organización, y no el navegador; uno
    /// de fuera sí va al navegador, y una ruta de cac que la app no tiene,
    /// a la web.
    ///
    /// Roturas que atrapa: un enlace de cac que sale al navegador; no
    /// cambiar a la organización de la tarea (se abriría encima de otra);
    /// darle al navegador una ruta sin el servidor ni `/app`; una mención
    /// que intenta abrirse.
    #[gpui_kit::test]
    fn a_cac_link_clicked_in_the_app_opens_here_and_not_in_the_browser(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        sign_in(cx);
        let _window = open_window(cx);
        wait_until(cx, "la organización se carga", |cx| current_org(cx).as_deref() == Some("o1"));
        let base = cx.update(|cx| Backend::core(cx).base_url().to_string());

        cx.update(|cx| open(&format!("{base}/app/tasks?task=cac-12"), cx));
        wait_until(cx, "la tarea se abre aquí, en su organización", |cx| task_open(cx).as_deref() == Some("t1") && current_org(cx).as_deref() == Some("o2"));
        assert_eq!(cx.opened_url(), None, "un enlace de cac no se le da al navegador");

        cx.update(|cx| open("cac:user/11111111-1111-4111-8111-111111111111", cx));
        cx.run_until_parked();
        assert_eq!(cx.opened_url(), None, "una mención no va a ningún sitio");

        cx.update(|cx| open("cac://servers/x", cx));
        wait_until(cx, "lo que la app no tiene va a la web", |cx| cx.opened_url().is_some());
        assert_eq!(cx.opened_url(), Some(format!("{base}/app/servers/x")));

        cx.update(|cx| open("https://example.com/a", cx));
        wait_until(cx, "lo de fuera, al navegador", |cx| cx.opened_url().as_deref() == Some("https://example.com/a"));
        stop(cx);
    }

    /// Relanzar la app sin enlace la trae al frente, y nada más; con
    /// enlace, además lo abre.
    ///
    /// Rotura que atrapa: que el «al frente» de otra instancia no haga nada
    /// (la segunda se cierra y la abierta sigue escondida).
    #[cfg(unix)]
    #[gpui_kit::test]
    fn launching_again_brings_the_open_app_to_the_front(cx: &mut TestAppContext) {
        use crate::single_instance::Message;
        let server = server();
        server.install(cx);
        sign_in(cx);
        let _window = open_window(cx);
        wait_until(cx, "la organización se carga", |cx| current_org(cx).as_deref() == Some("o1"));
        let before = cx.update(|cx| crate::notifier::times_fronted(cx));

        cx.update(|cx| from_another_launch(Message::Focus, cx));
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| crate::notifier::times_fronted(cx)), before + 1, "al frente");
        assert_eq!(task_open(cx), None, "sin abrir nada");

        cx.update(|cx| from_another_launch(Message::Open("cac://tasks?task=cac-12".into()), cx));
        wait_until(cx, "con enlace, lo abre", |cx| task_open(cx).as_deref() == Some("t1"));
        assert_eq!(cx.update(|cx| crate::notifier::times_fronted(cx)), before + 2);
        stop(cx);
    }

    /// Un enlace que llega de fuera antes de que haya sesión espera a que
    /// la haya, y entonces se abre; uno que no se encuentra lo dice con un
    /// aviso, que de fuera no hay vista donde escribirlo; y uno a la
    /// organización la abre con el enganche de quien la pinta.
    ///
    /// Roturas que atrapa: perder el enlace con el que se arrancó la app
    /// porque la sesión aún no estaba; abrirlo dos veces; no traer la
    /// ventana al frente; un fallo mudo; una pantalla de la organización que
    /// no se abre.
    #[gpui_kit::test]
    fn a_link_from_outside_waits_for_the_session_and_says_when_it_fails(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        cx.update(|cx| open_from_outside("cac://tasks?task=cac-12".into(), cx));
        cx.run_until_parked();
        assert_eq!(task_open(cx), None, "sin sesión no se abre nada");
        assert_eq!(cx.update(|cx| crate::notifier::times_fronted(cx)), 1, "pero la ventana, al frente");

        sign_in(cx);
        let _window = open_window(cx);
        wait_until(cx, "con la sesión, se abre", |cx| task_open(cx).as_deref() == Some("t1"));
        assert_eq!(server.hits("/api/v1/tasks/by-ref/cac-12"), 1, "una sola vez");

        cx.update(|cx| open_from_outside("/tasks?task=no-existe-1".into(), cx));
        wait_until(cx, "dice que no se pudo abrir", |cx| cx.shown_system_notifications().iter().any(|shown| shown.tag.as_ref() == FAILED_TAG));
        let shown = cx.shown_system_notifications();
        assert_eq!(shown.last().unwrap().body.as_ref(), "No se pudo abrir el enlace");

        let asked = Rc::new(std::cell::Cell::new(0));
        let counted = asked.clone();
        cx.update(|cx| on_open_organization(cx, move |_| counted.set(counted.get() + 1)));
        cx.update(|cx| open_from_outside("cac://organization".into(), cx));
        wait_until(cx, "abre la organización", |_| asked.get() == 1);
        stop(cx);
    }
}
