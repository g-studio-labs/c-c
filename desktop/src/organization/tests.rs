// Las pruebas de la pantalla de la organización y de las invitaciones
// recibidas. Se incluyen en las de `workspace.rs` (con `include!`) para usar
// su servidor de mentira y sus ayudas.

/// Una organización `o1` en la que Ana tiene `role`, con el espacio `a`,
/// dos miembros (Ana y Bob) y una invitación enviada a Caro, ya caducada.
/// Y otra, `o2` («Cliente»), de la que Ana es miembro.
fn org_server(role: &str) -> FakeServer {
    org_server_with(role, json!([]))
}

/// Lo mismo, con estos proyectos de reportes (las integraciones).
fn org_server_with(role: &str, projects: serde_json::Value) -> FakeServer {
    let server = FakeServer::start();
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    let mut organization = org("o1", "G Studio");
    organization["role"] = json!(role);
    organization["defaultInviteRole"] = json!("member");
    server.mount(get("/api/v1/organizations/").respond_with(ok(json!([organization, org("o2", "Cliente")]))));
    server.mount(get("/api/v1/organizations/o2/members").respond_with(ok(json!([{ "userId": "u1", "username": "ana", "role": "member" }]))));
    server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))));
    server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
    server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
    server.mount(get("/api/v1/report-projects/").respond_with(ok(projects)));
    server.mount(get("/api/v1/organizations/o1/members").respond_with(ok(json!([
        { "userId": "u1", "username": "ana", "name": "Ana Ruiz", "email": "ana@x.com", "role": role, "lastSeenAt": null },
        { "userId": "u2", "username": "bob", "email": "bob@acme.io", "role": "viewer" },
    ]))));
    server.mount(get("/api/v1/organizations/o1/invitations").respond_with(ok(json!([{
        "id": "inv-1", "orgId": "o1", "orgName": "G Studio", "role": "viewer", "status": "pending", "invitedBy": "ana",
        "invitedUser": "caro", "createdAt": "2026-01-01T10:00:00Z", "expiresAt": "2026-01-15T10:00:00Z",
    }]))));
    server
}

fn open_org(role: &str, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
    open_org_sized(role, 1100., cx)
}

fn open_org_sized(role: &str, width: f32, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
    let server = org_server(role);
    server.install(cx);
    let window = open_sized(width, cx);
    wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
    click(window, "rail-organization", cx);
    wait_until(cx, "llegan los miembros", |cx| on_screen(window, "org-member-u2", cx));
    (server, window)
}

/// Pulsa el campo y escribe en él.
fn type_into(window: AnyWindowHandle, field: &str, text: &str, cx: &mut TestAppContext) {
    let (field, text) = (gpui_kit::SharedString::from(field.to_string()), text.to_string());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click(field, cx);
        window.input(&text, cx);
    })
    .unwrap();
}

fn stop_stream(cx: &mut TestAppContext) {
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// «Organización» en el riel abre su pantalla con quién está; quien sólo es
/// miembro no ve las invitaciones ni lo que no puede tocar, y un clic en el
/// árbol vuelve a las tareas, también en la lista que ya estaba elegida.
///
/// Roturas que atrapa: el botón del riel que sigue sin hacer nada; ofrecer
/// la pestaña de invitaciones, cambiar papeles o quitar gente a quien no
/// administra; la pantalla que se queda encima al pulsar en el árbol la
/// lista en la que ya se estaba.
#[gpui_kit::test]
fn the_organization_rail_opens_its_people_and_the_tree_leaves_it(cx: &mut TestAppContext) {
    let server = org_server("member");
    server.install(cx);
    let window = open(cx);
    wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
    click(window, "tree-l-a", cx);
    wait_until(cx, "acotado a la lista", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).my_work.scope.is_some()));
    click(window, "rail-organization", cx);
    wait_until(cx, "llegan los miembros", |cx| on_screen(window, "org-member-u2", cx));
    assert!(on_screen(window, "org-page", cx) && on_screen(window, "org-tab-members", cx));
    assert!(!on_screen(window, "org-tab-invites", cx), "un miembro ve la pestaña de invitaciones");
    assert!(!on_screen(window, "org-member-remove-u2", cx) && !on_screen(window, "org-member-role-u2", cx), "un miembro puede quitar o cambiar papeles");
    assert!(!on_screen(window, "org-invite", cx));

    click(window, "tree-l-a", cx);
    wait_until(cx, "vuelve a las tareas", |cx| !on_screen(window, "org-page", cx));
    assert!(on_screen(window, "view-title", cx));
    stop_stream(cx);
}

/// Desde la organización, un clic en el panel de otro módulo (aquí, un
/// canal en el de comunicación, que no abre tarea ni documento) lleva a lo
/// pulsado.
///
/// Rotura que atrapa: la pantalla de la organización que se queda encima
/// de lo que se eligió en el panel.
#[gpui_kit::test]
fn a_click_in_another_modules_panel_leaves_the_organization(cx: &mut TestAppContext) {
    let (_server, window) = open_org("member", cx);
    click(window, "rail-comms", cx);
    wait_until(cx, "sale el panel de comunicación", |cx| on_screen(window, "comms-channel-a", cx));
    click(window, "rail-organization", cx);
    wait_until(cx, "vuelve la organización", |cx| on_screen(window, "org-page", cx));
    assert!(on_screen(window, "comms-channel-a", cx), "el panel deja de ser el del módulo");
    click(window, "comms-channel-a", cx);
    wait_until(cx, "abre lo pulsado", |cx| !on_screen(window, "org-page", cx));
    stop_stream(cx);
}

/// Quitar a alguien pregunta nombrándolo; cancelar no lo quita y aceptar
/// lo saca de la tabla.
///
/// Roturas que atrapa: quitar sin preguntar; una pregunta que no dice a
/// quién; quitar al cancelar; dejarlo en la tabla.
#[gpui_kit::test]
fn removing_a_member_asks_naming_them(cx: &mut TestAppContext) {
    let (server, window) = open_org("admin", cx);
    server.mount(Mock::given(method("DELETE")).and(path("/api/v1/organizations/o1/members/u2")).respond_with(ok(json!(null))));
    let deletes = |server: &FakeServer| server.requests().iter().filter(|request| request.method.as_str() == "DELETE").count();

    click(window, "org-member-remove-u2", cx);
    let (title, body) = cx.pending_prompt().expect("quitó sin preguntar");
    assert_eq!(title, "¿Quitar a @bob?");
    assert!(body.contains("G Studio"), "{body}");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert_eq!(deletes(&server), 0, "quitó al cancelar");

    click(window, "org-member-remove-u2", cx);
    cx.simulate_prompt_answer("Quitar");
    wait_until(cx, "sale de la tabla", |cx| !on_screen(window, "org-member-u2", cx));
    assert_eq!(deletes(&server), 1);
    stop_stream(cx);
}

/// Invitaciones: la caducada se ve como tal y se puede revivir; elegir a
/// alguien e invitarlo manda su papel y vuelve a leer la lista.
///
/// Roturas que atrapa: esconder la caducada o llamarla vigente; un
/// «Reenviar» que no llega al servidor; invitar con otro papel que el
/// elegido o sin haber elegido a nadie; no releer tras invitar.
#[gpui_kit::test]
fn an_admin_sees_the_expired_invitation_and_invites_someone(cx: &mut TestAppContext) {
    let (server, window) = open_org("admin", cx);
    server.mount(Mock::given(method("POST")).and(path("/api/v1/organizations/o1/invitations/inv-1/resend")).respond_with(ok(json!(null))));
    server.mount(Mock::given(method("GET")).and(path("/api/v1/users/search")).respond_with(ok(json!([{ "id": "u-dan", "username": "dan" }]))));
    server.mount(Mock::given(method("POST")).and(path("/api/v1/organizations/o1/invitations")).respond_with(ok(json!(null))));

    click(window, "org-tab-invites", cx);
    wait_until(cx, "sale la invitación", |cx| on_screen(window, "org-invite-inv-1", cx));
    assert_eq!(label(window, "org-invite-deadline-inv-1", cx), "caducada");

    let lists_before = server.hits("/api/v1/organizations/o1/invitations");
    click(window, "org-invite-resend-inv-1", cx);
    wait_until(cx, "la reenvía y relee", |_| server.hits("/api/v1/organizations/o1/invitations/inv-1/resend") == 1 && server.hits("/api/v1/organizations/o1/invitations") > lists_before);

    assert!(!on_screen(window, "org-person-u-dan", cx));
    type_into(window, "org-people-search", "da", cx);
    wait_until(cx, "sale Dan", |cx| on_screen(window, "org-person-u-dan", cx));
    click(window, "org-person-u-dan", cx);
    click(window, "org-invite-role", cx);
    wait_until(cx, "se abre el menú del papel", |cx| on_screen(window, "org-invite-role-admin", cx));
    click(window, "org-invite-role-admin", cx);
    click(window, "org-send-invitation", cx);
    wait_until(cx, "la manda", |_| {
        server.requests().iter().any(|request| request.method.as_str() == "POST" && request.url.path() == "/api/v1/organizations/o1/invitations")
    });
    let sent = server.requests().into_iter().find(|request| request.method.as_str() == "POST" && request.url.path() == "/api/v1/organizations/o1/invitations").unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&sent.body).unwrap(), json!({ "userId": "u-dan", "role": "admin" }));
    wait_until(cx, "lo dice", |cx| on_screen(window, "org-notice", cx));
    assert_eq!(label(window, "org-notice", cx), "Invitado @dan a G Studio");
    stop_stream(cx);
}

/// Las fichas de los espacios cuentan sus tareas y dicen quién los ve, y
/// «Gestionar» lleva a las tareas de ese espacio.
///
/// Roturas que atrapa: no sumar las listas de las carpetas; un «Gestionar»
/// que no hace nada o que deja la organización encima.
#[gpui_kit::test]
fn a_space_card_counts_and_manage_goes_to_its_tasks(cx: &mut TestAppContext) {
    let (_server, window) = open_org("member", cx);
    click(window, "org-tab-spaces", cx);
    wait_until(cx, "sale la ficha", |cx| on_screen(window, "org-space-a", cx));
    assert_eq!(label(window, "org-space-tasks-a", cx), "7 · 5 abiertas");
    assert_eq!(label(window, "org-space-access-a", cx), "toda la organización");

    click(window, "org-space-manage-a", cx);
    wait_until(cx, "va a sus tareas", |cx| !on_screen(window, "org-page", cx));
    assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).my_work.scope.clone().map(|scope| scope.id)), Some("a".to_string()));
    stop_stream(cx);
}

/// Borrar la organización: sólo un superadmin, y escribiendo la frase
/// entera.
///
/// Roturas que atrapa: dejar borrar a quien no es superadmin; un botón que
/// se enciende con el nombre a secas; borrar sin la frase exacta, también
/// con Enter.
#[gpui_kit::test]
fn deleting_the_organization_is_typed_out(cx: &mut TestAppContext) {
    // Ancho para las tres tarjetas en una fila: la de peligro, a la vista.
    let (server, window) = open_org_sized("admin", 1500., cx);
    click(window, "org-tab-general", cx);
    wait_until(cx, "sale General", |cx| on_screen(window, "org-danger", cx));
    click(window, "org-delete", cx);
    assert!(!on_screen(window, "org-delete-dialog", cx), "un admin que no es superadmin abre el borrado");

    cx.update(|cx| {
        let boss = serde_json::from_value(json!({ "id": "u1", "username": "ana", "superadmin": true })).unwrap();
        Backend::core(cx).update_session(boss).unwrap();
    });
    wait_until(cx, "la pantalla sabe que es superadmin", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).session.as_ref().is_some_and(|session| session.superadmin)));
    server.mount(Mock::given(method("DELETE")).and(path("/api/v1/organizations/o1")).respond_with(ok(json!(null))));
    click(window, "org-delete", cx);
    wait_until(cx, "se abre el diálogo", |cx| on_screen(window, "org-delete-dialog", cx));
    type_into(window, "org-delete-phrase", "G Studio", cx);
    click(window, "org-delete-confirm", cx);
    // Y tampoco con Enter, que no pasa por el botón apagado.
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    let deletes = |server: &FakeServer| server.requests().iter().filter(|request| request.method.as_str() == "DELETE").count();
    assert_eq!(deletes(&server), 0, "borró con el nombre a secas");

    click(window, "org-delete-cancel", cx);
    click(window, "org-delete", cx);
    wait_until(cx, "se abre otra vez, vacío", |cx| on_screen(window, "org-delete-dialog", cx));
    type_into(window, "org-delete-phrase", "delete/G Studio", cx);
    click(window, "org-delete-confirm", cx);
    wait_until(cx, "la borra", |_| deletes(&server) == 1);
    stop_stream(cx);
}

/// Las invitaciones que te esperan: una fila al pie del panel con cuántas,
/// su pantalla, y aceptar una la quita y baja el número del riel.
///
/// Roturas que atrapa: la fila que no sale (o sale sin ninguna); aceptar
/// sin llamar al servidor; la invitación aceptada que se queda en la lista;
/// el riel que sigue contándola.
#[gpui_kit::test]
fn an_invitation_for_you_is_accepted_from_the_panel(cx: &mut TestAppContext) {
    let server = org_server("member");
    let invitation = json!({ "id": "i9", "orgId": "o9", "orgName": "Dwit", "role": "viewer", "status": "pending", "invitedBy": "bea", "createdAt": "2026-11-20T10:00:00Z" });
    server.mount(Mock::given(method("GET")).and(path("/api/v1/invitations/")).respond_with(ok(json!([invitation]))).up_to_n_times(2));
    server.mount(Mock::given(method("GET")).and(path("/api/v1/invitations/")).respond_with(ok(json!([]))));
    server.mount(Mock::given(method("POST")).and(path("/api/v1/invitations/i9/accept")).respond_with(ok(json!(null))));
    server.install(cx);
    let window = open(cx);
    wait_until(cx, "sale la fila de invitaciones", |cx| on_screen(window, "panel-invitations", cx));
    assert_eq!(label(window, "rail-organization", cx), "Organización (1)");

    click(window, "panel-invitations", cx);
    wait_until(cx, "sale la invitación", |cx| on_screen(window, "invitation-i9", cx));
    click(window, "invitation-accept-i9", cx);
    wait_until(cx, "la acepta", |_| server.hits("/api/v1/invitations/i9/accept") == 1);
    wait_until(cx, "sale de la lista", |cx| !on_screen(window, "invitation-i9", cx));
    wait_until(cx, "el riel deja de contarla", |cx| !on_screen(window, "rail-organization-count", cx));
    assert!(!on_screen(window, "panel-invitations", cx), "la fila sigue sin invitaciones");
    stop_stream(cx);
}

/// Cambiar de organización con su pantalla delante la deja delante, con
/// lo de la otra.
///
/// Rotura que atrapa: cerrar la pantalla porque el core suelta o vuelve a
/// elegir conversación, ámbito o documento al cambiar de organización.
#[gpui_kit::test]
fn switching_organization_keeps_its_screen(cx: &mut TestAppContext) {
    let (server, window) = open_org("admin", cx);
    // Con un documento abierto: cambiar de organización lo cierra, y eso no
    // es irse de la pantalla.
    server.mount(Mock::given(method("GET")).and(path("/api/v1/docs/space/a")).respond_with(ok(json!({ "doc": null, "orgId": "o1", "tabs": [], "decisions": [] }))));
    cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open("a").await }));
    wait_until(cx, "se abre el documento", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.target.is_some()));
    click(window, "rail-organization", cx);
    wait_until(cx, "vuelve la organización", |cx| on_screen(window, "org-page", cx));
    cx.update(|cx| Backend::run(cx, |core| async move { core.switch_org("o2").await; }));
    wait_until(cx, "pasa a la otra", |cx| on_screen(window, "org-page-title", cx) && label(window, "org-page-title", cx) == "Cliente");
    cx.run_until_parked();
    assert!(on_screen(window, "org-page", cx), "cambiar de organización cerró la pantalla");
    stop_stream(cx);
}

/// Quitarte a ti mismo te lleva a otra organización.
///
/// Rotura que atrapa: seguir en la que dejaste, con un papel que ya no
/// tienes.
#[gpui_kit::test]
fn removing_yourself_moves_you_to_another_organization(cx: &mut TestAppContext) {
    let (server, window) = open_org("admin", cx);
    server.mount(Mock::given(method("DELETE")).and(path("/api/v1/organizations/o1/members/u1")).respond_with(ok(json!(null))));
    click(window, "org-member-remove-u1", cx);
    cx.simulate_prompt_answer("Quitar");
    wait_until(cx, "pasa a la otra", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).orgs.current_org_id.as_deref() == Some("o2")));
    stop_stream(cx);
}

/// Bajarte el papel a ti mismo vuelve a leer tu papel: dejan de salir las
/// invitaciones.
///
/// Rotura que atrapa: seguir viendo lo de admin tras dejar de serlo.
#[gpui_kit::test]
fn demoting_yourself_rereads_your_role(cx: &mut TestAppContext) {
    let server = FakeServer::start();
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    let mut admin = org("o1", "G Studio");
    admin["role"] = json!("admin");
    let mut viewer = admin.clone();
    viewer["role"] = json!("viewer");
    server.mount(get("/api/v1/organizations/").respond_with(ok(json!([admin]))).up_to_n_times(1));
    server.mount(get("/api/v1/organizations/").respond_with(ok(json!([viewer]))));
    for mock in [
        get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))),
        get("/api/v1/docs/").respond_with(ok(json!({}))),
        get("/api/v1/tasks/").respond_with(ok(json!([]))),
        get("/api/v1/report-projects/").respond_with(ok(json!([]))),
        get("/api/v1/organizations/o1/invitations").respond_with(ok(json!([]))),
        get("/api/v1/organizations/o1/members").respond_with(ok(json!([{ "userId": "u1", "username": "ana", "role": "admin" }, { "userId": "u2", "username": "bob", "role": "viewer" }]))),
        Mock::given(method("PATCH")).and(path("/api/v1/organizations/o1/members/u1")).respond_with(ok(json!(null))),
    ] {
        server.mount(mock);
    }
    server.install(cx);
    let window = open(cx);
    wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
    click(window, "rail-organization", cx);
    wait_until(cx, "llegan los miembros", |cx| on_screen(window, "org-member-u2", cx));
    assert!(on_screen(window, "org-tab-invites", cx));
    click(window, "org-member-role-u1", cx);
    wait_until(cx, "se abre el menú del papel", |cx| on_screen(window, "org-member-role-u1-viewer", cx));
    click(window, "org-member-role-u1-viewer", cx);
    wait_until(cx, "deja de ser admin", |cx| !on_screen(window, "org-tab-invites", cx));
    stop_stream(cx);
}

/// El perfil y la organización no se pisan: abrir uno cierra el otro, y el
/// riel enciende la organización, o nada con el perfil, nunca dos cosas.
///
/// Roturas que atrapa: el perfil pintado debajo de la organización (o al
/// revés); la organización encendida en el riel con el perfil delante; un
/// módulo encendido a la vez que la organización.
#[gpui_kit::test]
fn the_profile_and_the_organization_exclude_each_other(cx: &mut TestAppContext) {
    let server = org_server("admin");
    server.install(cx);
    let (window, workspace) = open_with_view(1100., cx);
    wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
    let lit = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let workspace = workspace.read(cx);
            let modules = (1..=4).filter(|slot| workspace.module_active(*slot, cx)).count();
            (modules, workspace.organization_lit())
        })
    };
    let shown = |cx: &mut TestAppContext| (on_screen(window, "org-page", cx), on_screen(window, "profile", cx));

    click(window, "rail-organization", cx);
    wait_until(cx, "sale la organización", |cx| on_screen(window, "org-page", cx));
    assert_eq!(lit(cx), (0, true));

    click(window, "account-menu", cx);
    wait_until(cx, "el menú se abre", |cx| on_screen(window, "account-profile", cx));
    click(window, "account-profile", cx);
    cx.run_until_parked();
    assert_eq!(shown(cx), (false, true), "el perfil no tapa la organización");
    assert_eq!(lit(cx), (0, false), "el riel sigue encendiendo algo con el perfil delante");

    click(window, "rail-organization", cx);
    cx.run_until_parked();
    assert_eq!(shown(cx), (true, false), "la organización no cierra el perfil");
    assert_eq!(lit(cx), (0, true));

    click(window, "rail-work", cx);
    cx.run_until_parked();
    assert_eq!(shown(cx), (false, false));
    assert_eq!(lit(cx), (1, false));
    stop_stream(cx);
}

fn open_org_with(server: FakeServer, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
    server.install(cx);
    let window = open_sized(1100., cx);
    wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
    click(window, "rail-organization", cx);
    wait_until(cx, "llegan los miembros", |cx| on_screen(window, "org-member-u2", cx));
    (server, window)
}

fn request_body(server: &FakeServer, verb: &str, route: &str) -> serde_json::Value {
    let sent = server.requests().into_iter().find(|request| request.method.as_str() == verb && request.url.path() == route).unwrap_or_else(|| panic!("no hubo {verb} {route}"));
    serde_json::from_slice(&sent.body).unwrap_or(json!(null))
}

fn count_of(server: &FakeServer, verb: &str, route: &str) -> usize {
    server.requests().iter().filter(|request| request.method.as_str() == verb && request.url.path() == route).count()
}

fn a_meeting() -> serde_json::Value {
    json!({
        "id": "m1", "orgId": "o1", "title": "Daily", "wallTime": "09:30", "timezone": "America/Mexico_City", "freq": "weekly",
        "interval": 1, "weekdays": "1,2,3,4,5", "nextFireAt": "2026-10-05T15:30:00Z", "paused": false, "excludedUserIds": ["u2"],
    })
}

fn meetings_server(role: &str) -> FakeServer {
    let server = org_server(role);
    server.mount(Mock::given(method("GET")).and(path("/api/v1/organizations/o1/meetings/")).respond_with(ok(json!([a_meeting()]))));
    server.mount(Mock::given(method("PATCH")).and(path("/api/v1/meetings/m1/")).respond_with(ok(json!(null))));
    server.mount(Mock::given(method("DELETE")).and(path("/api/v1/meetings/m1/")).respond_with(ok(json!(null))));
    server.mount(Mock::given(method("PUT")).and(path("/api/v1/meetings/m1/recipients")).respond_with(ok(json!(null))));
    server.mount(Mock::given(method("POST")).and(path("/api/v1/organizations/o1/meetings/")).respond_with(ok(json!(null))));
    server
}

/// La ficha de una reunión dice su regla, a qué hora suena con su ciudad y
/// a cuántos les llega; quitar a alguien manda la lista de excluidos
/// entera; pausar manda sólo la bandera; borrar pregunta nombrándola.
///
/// Roturas que atrapa: una regla sin sus días; la hora sin la ciudad; contar
/// como convocado a quien se quitó; pausar mandando la reunión entera;
/// borrar sin preguntar o al cancelar; olvidar releer tras cada cambio.
#[gpui_kit::test]
fn a_meeting_card_says_when_and_who_and_its_buttons_change_it(cx: &mut TestAppContext) {
    let (server, window) = open_org_with(meetings_server("admin"), cx);
    click(window, "org-tab-meetings", cx);
    wait_until(cx, "sale la reunión", |cx| on_screen(window, "org-meeting-m1", cx));
    assert_eq!(label(window, "org-meeting-rule-m1", cx), "Semanal · lun, mar, mié, jue, vie");
    assert!(label(window, "org-meeting-when-m1", cx).starts_with("09:30 en Mexico City"), "{}", label(window, "org-meeting-when-m1", cx));
    assert_eq!(label(window, "org-meeting-reaches-m1", cx), "1 de 2");

    click(window, "org-meeting-people-m1", cx);
    wait_until(cx, "salen los miembros", |cx| on_screen(window, "org-meeting-person-m1-u1", cx));
    click(window, "org-meeting-person-m1-u2", cx);
    wait_until(cx, "manda quién queda excluido", |_| count_of(&server, "PUT", "/api/v1/meetings/m1/recipients") == 1);
    assert_eq!(request_body(&server, "PUT", "/api/v1/meetings/m1/recipients"), json!({ "excludedUserIds": [] }));

    let reads = server.hits("/api/v1/organizations/o1/meetings/");
    click(window, "org-meeting-pause-m1", cx);
    wait_until(cx, "pausa y relee", |_| count_of(&server, "PATCH", "/api/v1/meetings/m1/") == 1 && server.hits("/api/v1/organizations/o1/meetings/") > reads);
    assert_eq!(request_body(&server, "PATCH", "/api/v1/meetings/m1/"), json!({ "paused": true }));

    click(window, "org-meeting-delete-m1", cx);
    let (title, _) = cx.pending_prompt().expect("borró sin preguntar");
    assert_eq!(title, "¿Borrar Daily?");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert_eq!(count_of(&server, "DELETE", "/api/v1/meetings/m1/"), 0, "borró al cancelar");
    click(window, "org-meeting-delete-m1", cx);
    cx.simulate_prompt_answer("Borrar");
    wait_until(cx, "la borra", |_| count_of(&server, "DELETE", "/api/v1/meetings/m1/") == 1);
    stop_stream(cx);
}

/// Crear una reunión manda la regla entera —entre semana, la hora y la zona
/// escritas— y no se puede guardar sin nombre.
///
/// Roturas que atrapa: guardar sin nombre; los días de entre semana que no
/// viajan; la hora que no llega como se escribió; no cerrar el formulario ni
/// releer al guardar.
#[gpui_kit::test]
fn creating_a_meeting_sends_the_whole_rule(cx: &mut TestAppContext) {
    let (server, window) = open_org_with(meetings_server("admin"), cx);
    click(window, "org-tab-meetings", cx);
    wait_until(cx, "sale la reunión", |cx| on_screen(window, "org-meeting-m1", cx));
    click(window, "org-meeting-new", cx);
    wait_until(cx, "sale el formulario", |cx| on_screen(window, "org-meeting-form", cx));
    click(window, "org-meeting-save", cx);
    cx.run_until_parked();
    assert_eq!(count_of(&server, "POST", "/api/v1/organizations/o1/meetings/"), 0, "guardó sin nombre");

    type_into(window, "org-meeting-title", "Retro", cx);
    wait_until(cx, "la vista previa dice la hora", |cx| on_screen(window, "org-meeting-preview", cx));
    click(window, "org-meeting-save", cx);
    wait_until(cx, "la manda", |_| count_of(&server, "POST", "/api/v1/organizations/o1/meetings/") == 1);
    let body = request_body(&server, "POST", "/api/v1/organizations/o1/meetings/");
    assert_eq!((body["title"].as_str(), body["wallTime"].as_str(), body["freq"].as_str(), body["weekdays"].as_str()), (Some("Retro"), Some("09:00"), Some("weekly"), Some("1,2,3,4,5")), "{body}");
    assert!(cac_core::is_zone(body["timezone"].as_str().unwrap_or_default()), "{body}");
    wait_until(cx, "se cierra el formulario", |cx| !on_screen(window, "org-meeting-form", cx));
    stop_stream(cx);
}

/// Quien sólo es miembro ve las reuniones pero ni las crea ni las cambia.
///
/// Rotura que atrapa: ofrecer a un miembro crear, editar, pausar o quitar.
#[gpui_kit::test]
fn a_member_sees_meetings_but_cannot_change_them(cx: &mut TestAppContext) {
    let (_server, window) = open_org_with(meetings_server("member"), cx);
    click(window, "org-tab-meetings", cx);
    wait_until(cx, "sale la reunión", |cx| on_screen(window, "org-meeting-m1", cx));
    let shown: Vec<&str> = ["org-meeting-new", "org-meeting-edit-m1", "org-meeting-pause-m1", "org-meeting-delete-m1", "org-meeting-people-m1"].into_iter().filter(|id| on_screen(window, id, cx)).collect();
    assert!(shown.is_empty(), "un miembro ve: {shown:?}");
    stop_stream(cx);
}

fn github_status(configured: bool, unbound: serde_json::Value) -> serde_json::Value {
    json!({
        "configured": configured,
        "installations": [{ "id": "i1", "installationId": 5, "accountLogin": "dwit", "orgId": "o1" }],
        "repos": [{ "id": "r1", "installationId": 5, "repoId": 100, "fullName": "dwit/api", "orgId": "o1", "spaceId": "", "bareRefs": false, "showDocs": false }],
        "unbound": unbound,
    })
}

fn github_server(role: &str, status: serde_json::Value) -> FakeServer {
    let server = org_server(role);
    server.mount(Mock::given(method("GET")).and(path("/api/v1/organizations/o1/github/")).respond_with(ok(status)));
    server.mount(Mock::given(method("POST")).and(path("/api/v1/organizations/o1/github/link")).respond_with(ok(json!({ "url": "https://github.com/apps/cac/installations/new?state=s" }))));
    server.mount(Mock::given(method("PATCH")).and(path("/api/v1/organizations/o1/github/repos/r1")).respond_with(ok(json!({
        "id": "r1", "installationId": 5, "repoId": 100, "fullName": "dwit/api", "orgId": "o1", "spaceId": "a", "bareRefs": false, "showDocs": false,
    }))));
    server.mount(Mock::given(method("POST")).and(path("/api/v1/organizations/o1/github/installations/9/adopt")).respond_with(ok(json!(null))));
    server
}

/// Sin la App en el servidor se dice, y no hay botón que no lleve a nada.
///
/// Rotura que atrapa: ofrecer conectar un GitHub que el servidor no tiene.
#[gpui_kit::test]
fn without_the_github_app_the_tab_says_so(cx: &mut TestAppContext) {
    let (_server, window) = open_org_with(github_server("admin", github_status(false, json!([]))), cx);
    click(window, "org-tab-github", cx);
    wait_until(cx, "lo dice", |cx| on_screen(window, "org-github-off", cx));
    assert!(!on_screen(window, "org-github-connect", cx));
    stop_stream(cx);
}

/// Conectar abre en el navegador la dirección del servidor; un repo se
/// enlaza a un espacio de esta organización y sólo entonces ofrece sus docs.
///
/// Roturas que atrapa: conectar sin pedir la dirección o sin abrirla; el
/// cuerpo del cambio sin el resto del repo (el servidor reemplaza); los docs
/// de un repo sin enlazar.
#[gpui_kit::test]
fn connecting_opens_the_browser_and_a_repo_links_to_a_space(cx: &mut TestAppContext) {
    let (server, window) = open_org_with(github_server("admin", github_status(true, json!([]))), cx);
    click(window, "org-tab-github", cx);
    wait_until(cx, "sale el repo", |cx| on_screen(window, "org-github-repo-r1", cx));
    assert!(!on_screen(window, "org-github-docs-r1", cx), "docs en un repo sin enlazar");

    click(window, "org-github-connect", cx);
    wait_until(cx, "se abre fuera", |cx| cx.opened_url().is_some());
    assert_eq!(cx.opened_url().as_deref(), Some("https://github.com/apps/cac/installations/new?state=s"));
    assert_eq!(request_body(&server, "POST", "/api/v1/organizations/o1/github/link"), json!({}));

    click(window, "org-github-space-r1", cx);
    wait_until(cx, "se abre el menú", |cx| on_screen(window, "org-github-space-r1-a", cx));
    click(window, "org-github-space-r1-a", cx);
    wait_until(cx, "lo guarda", |_| count_of(&server, "PATCH", "/api/v1/organizations/o1/github/repos/r1") == 1);
    assert_eq!(request_body(&server, "PATCH", "/api/v1/organizations/o1/github/repos/r1"), json!({ "spaceId": "a", "bareRefs": false, "showDocs": false }));
    wait_until(cx, "ya hay docs que ofrecer", |cx| on_screen(window, "org-github-docs-r1", cx));
    stop_stream(cx);
}

/// Una instalación sin organización se ata preguntando antes y con sus
/// repos a la vista; quien no administra ni la ve ni conecta.
///
/// Roturas que atrapa: atar sin preguntar o al cancelar; una pregunta sin
/// los repos; enseñar la sección o el botón a quien no administra.
#[gpui_kit::test]
fn an_unbound_installation_is_linked_only_after_asking(cx: &mut TestAppContext) {
    let unbound = json!([{ "installationId": 9, "accountLogin": "Crossfire", "repos": ["Crossfire/app"] }]);
    let (server, window) = open_org_with(github_server("admin", github_status(true, unbound.clone())), cx);
    click(window, "org-tab-github", cx);
    wait_until(cx, "sale la sección", |cx| on_screen(window, "org-github-adopt-9", cx));
    click(window, "org-github-adopt-9", cx);
    let (title, body) = cx.pending_prompt().expect("ató sin preguntar");
    assert!(title.contains("Crossfire") && body.contains("Crossfire/app"), "{title} / {body}");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert_eq!(count_of(&server, "POST", "/api/v1/organizations/o1/github/installations/9/adopt"), 0, "ató al cancelar");
    click(window, "org-github-adopt-9", cx);
    cx.simulate_prompt_answer("Atar a esta organización");
    wait_until(cx, "la ata", |_| count_of(&server, "POST", "/api/v1/organizations/o1/github/installations/9/adopt") == 1);
    stop_stream(cx);

    let (_server, window) = open_org_with(github_server("member", github_status(true, unbound)), cx);
    click(window, "org-tab-github", cx);
    wait_until(cx, "sale el repo", |cx| on_screen(window, "org-github-repo-r1", cx));
    assert!(!on_screen(window, "org-github-unbound", cx) && !on_screen(window, "org-github-connect", cx));
    stop_stream(cx);
}

fn a_project() -> serde_json::Value {
    json!({
        "id": "p1", "orgId": "o1", "name": "boaty", "slug": "boaty", "rateLimitPerHour": 500, "rateLimitPerReporterPerHour": 7, "isActive": true,
        "webhookUrl": "", "webhookConfigured": false, "createdAt": "2026-08-01T00:00:00Z", "reportsThisMonth": 12, "listId": "in-a",
    })
}

fn integrations_server(projects: serde_json::Value) -> FakeServer {
    integrations_server_as("admin", projects, None)
}

/// Con la respuesta de crear retenida, si se da una `Gate`.
fn integrations_server_as(role: &str, projects: serde_json::Value, create_held: Option<&Gate>) -> FakeServer {
    let server = org_server_with(role, projects);
    server.mount(Mock::given(method("PATCH")).and(path("/api/v1/report-projects/p1")).respond_with(ok(json!(null))));
    server.mount(Mock::given(method("DELETE")).and(path("/api/v1/report-projects/p1")).respond_with(ok(json!(null))));
    server.mount(Mock::given(method("POST")).and(path("/api/v1/report-projects/p1/rotate-key")).respond_with(ok(json!({ "ingestKey": "k-rotated" }))));
    let created = ok(json!({ "project": a_project(), "ingestKey": "k-new" }));
    server.mount(Mock::given(method("POST")).and(path("/api/v1/report-projects/")).respond_with(match create_held {
        Some(gate) => gate.hold(created),
        None => created,
    }));
    server
}

/// La ficha dice dónde caen los reportes con el nombre de la lista (no el
/// uuid), y avisa cuando no caen en ninguna parte o la lista es de otro sitio.
///
/// Roturas que atrapa: pintar el uuid; callar que lo que manda se pierde;
/// llamar «ninguna» a una lista que existe pero no es de aquí.
#[gpui_kit::test]
fn an_integration_says_where_its_reports_land(cx: &mut TestAppContext) {
    let mut lost = a_project();
    lost["id"] = json!("p2");
    lost["listId"] = json!(null);
    let mut elsewhere = a_project();
    elsewhere["id"] = json!("p3");
    elsewhere["listId"] = json!("de-otra-org");
    let (_server, window) = open_org_with(integrations_server(json!([a_project(), lost, elsewhere])), cx);
    click(window, "org-tab-integrations", cx);
    wait_until(cx, "salen las fichas", |cx| on_screen(window, "org-project-p1", cx));
    wait_until(cx, "llega el árbol", |cx| label(window, "org-project-list-p1", cx) == "Producto · Carpeta · En carpeta");
    assert_eq!(label(window, "org-project-list-p2", cx), "en ningún sitio: lo que mande se está perdiendo");
    assert_eq!(label(window, "org-project-list-p3", cx), "una lista de otra organización");
    stop_stream(cx);
}

/// Crear entrega la llave una sola vez y se queda a la vista hasta decir que
/// ya se guardó; rotar pregunta nombrándola y entrega otra; abrir y cerrar
/// el formulario no escribe nada, y pausar manda sólo la bandera.
///
/// Roturas que atrapa: la llave que no se enseña o se va sola; rotar sin
/// preguntar o al cancelar; un PATCH sin cambios; pausar con la ficha entera.
#[gpui_kit::test]
fn an_integration_shows_its_key_once_and_changes_only_what_was_asked(cx: &mut TestAppContext) {
    let (server, window) = open_org_with(integrations_server(json!([a_project()])), cx);
    click(window, "org-tab-integrations", cx);
    wait_until(cx, "salen las fichas", |cx| on_screen(window, "org-project-p1", cx));

    click(window, "org-project-new-button", cx);
    wait_until(cx, "sale el campo", |cx| on_screen(window, "org-project-new-name", cx));
    click(window, "org-project-create", cx);
    cx.run_until_parked();
    assert_eq!(count_of(&server, "POST", "/api/v1/report-projects/"), 0, "creó sin nombre");
    type_into(window, "org-project-new-name", "Latinet", cx);
    click(window, "org-project-create", cx);
    wait_until(cx, "la crea", |_| count_of(&server, "POST", "/api/v1/report-projects/") == 1);
    assert_eq!(request_body(&server, "POST", "/api/v1/report-projects/"), json!({ "orgId": "o1", "name": "Latinet" }));
    wait_until(cx, "enseña la llave", |cx| on_screen(window, "org-key", cx));
    assert_eq!(label(window, "org-key", cx), "k-new");
    click(window, "org-key-saved", cx);
    assert!(!on_screen(window, "org-key", cx));

    click(window, "org-project-rotate-p1", cx);
    let (title, _) = cx.pending_prompt().expect("rotó sin preguntar");
    assert_eq!(title, "¿Rotar la llave de boaty?");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert_eq!(count_of(&server, "POST", "/api/v1/report-projects/p1/rotate-key"), 0, "rotó al cancelar");
    click(window, "org-project-rotate-p1", cx);
    cx.simulate_prompt_answer("Rotar");
    wait_until(cx, "enseña la llave nueva", |cx| on_screen(window, "org-key", cx) && label(window, "org-key", cx) == "k-rotated");
    click(window, "org-key-saved", cx);

    click(window, "org-project-edit-p1", cx);
    wait_until(cx, "sale el formulario", |cx| on_screen(window, "org-project-save", cx));
    click(window, "org-project-save", cx);
    cx.run_until_parked();
    assert_eq!(count_of(&server, "PATCH", "/api/v1/report-projects/p1"), 0, "escribió sin cambios");
    assert!(!on_screen(window, "org-project-save", cx));

    click(window, "org-project-pause-p1", cx);
    wait_until(cx, "pausa", |_| count_of(&server, "PATCH", "/api/v1/report-projects/p1") == 1);
    assert_eq!(request_body(&server, "PATCH", "/api/v1/report-projects/p1"), json!({ "isActive": false }));
    stop_stream(cx);
}

/// Quien sólo es miembro ve las integraciones pero no las crea, edita,
/// pausa, borra ni rota.
///
/// Rotura que atrapa: ofrecerle a un miembro cualquiera de esos controles
/// (el servidor se lo negaría, y la pantalla no debe ofrecerlo).
#[gpui_kit::test]
fn a_member_sees_integrations_but_cannot_change_them(cx: &mut TestAppContext) {
    let (_server, window) = open_org_with(integrations_server_as("member", json!([a_project()]), None), cx);
    click(window, "org-tab-integrations", cx);
    wait_until(cx, "sale la ficha", |cx| on_screen(window, "org-project-p1", cx));
    let shown: Vec<&str> = ["org-project-new-button", "org-project-edit-p1", "org-project-pause-p1", "org-project-delete-p1", "org-project-rotate-p1"].into_iter().filter(|id| on_screen(window, id, cx)).collect();
    assert!(shown.is_empty(), "un miembro ve: {shown:?}");
    stop_stream(cx);
}

/// Un formulario abierto no sobrevive a perder el permiso: si te bajan de
/// admin con el de una reunión y el de una integración abiertos, se cierran.
///
/// Roturas que atrapa: el formulario de reunión o el de integración que se
/// quedan abiertos (y guardan, o lo intentan) tras el aviso `org:role`.
#[gpui_kit::test]
fn an_open_form_closes_when_you_stop_being_admin(cx: &mut TestAppContext) {
    let server = FakeServer::start();
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    let mut admin = org("o1", "G Studio");
    admin["role"] = json!("admin");
    let mut viewer = admin.clone();
    viewer["role"] = json!("viewer");
    server.mount(get("/api/v1/organizations/").respond_with(ok(json!([admin]))).up_to_n_times(1));
    server.mount(get("/api/v1/organizations/").respond_with(ok(json!([viewer]))));
    for mock in [
        get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))),
        get("/api/v1/docs/").respond_with(ok(json!({}))),
        get("/api/v1/tasks/").respond_with(ok(json!([]))),
        get("/api/v1/report-projects/").respond_with(ok(json!([a_project()]))),
        get("/api/v1/organizations/o1/invitations").respond_with(ok(json!([]))),
        get("/api/v1/organizations/o1/members").respond_with(ok(json!([{ "userId": "u1", "username": "ana", "role": "admin" }, { "userId": "u2", "username": "bob", "role": "viewer" }]))),
        get("/api/v1/organizations/o1/meetings/").respond_with(ok(json!([a_meeting()]))),
    ] {
        server.mount(mock);
    }
    let (_server, window) = open_org_with(server, cx);
    click(window, "org-tab-meetings", cx);
    wait_until(cx, "sale la reunión", |cx| on_screen(window, "org-meeting-m1", cx));
    click(window, "org-meeting-edit-m1", cx);
    wait_until(cx, "el formulario de la reunión", |cx| on_screen(window, "org-meeting-form", cx));
    click(window, "org-tab-integrations", cx);
    wait_until(cx, "sale la ficha", |cx| on_screen(window, "org-project-p1", cx));
    click(window, "org-project-edit-p1", cx);
    wait_until(cx, "el formulario de la integración", |cx| on_screen(window, "org-project-save", cx));

    cx.update(|cx| {
        Backend::run(cx, |core| async move {
            core.stream().deliver(cac_core::ServerEvent { event: "org:role".into(), data: json!({ "orgId": "o1", "orgName": "G Studio", "role": "viewer" }).to_string() })
        })
    });
    wait_until(cx, "se cierra el de la integración", |cx| !on_screen(window, "org-project-save", cx));
    click(window, "org-tab-meetings", cx);
    wait_until(cx, "vuelven las reuniones", |cx| on_screen(window, "org-meeting-m1", cx));
    assert!(!on_screen(window, "org-meeting-form", cx), "el formulario de la reunión sigue abierto");
    stop_stream(cx);
}

/// La llave se entrega aunque se cambie de organización mientras se crea, dice
/// de qué proyecto y organización es, y no se va al salir de la pantalla y
/// volver: sólo cuando se dice que ya se guardó.
///
/// Roturas que atrapa: tirar en silencio la llave de lo creado en otra
/// organización; la llave sin su proyecto y su organización; borrarla al
/// reabrir la pantalla sin haberla reconocido.
#[gpui_kit::test]
fn the_key_survives_switching_organization_and_reopening(cx: &mut TestAppContext) {
    let gate = Gate::default();
    let (_server, window) = open_org_with(integrations_server_as("admin", json!([a_project()]), Some(&gate)), cx);
    click(window, "org-tab-integrations", cx);
    wait_until(cx, "salen las fichas", |cx| on_screen(window, "org-project-p1", cx));
    click(window, "org-project-new-button", cx);
    wait_until(cx, "sale el campo", |cx| on_screen(window, "org-project-new-name", cx));
    type_into(window, "org-project-new-name", "Latinet", cx);
    click(window, "org-project-create", cx);
    wait_until(cx, "la petición queda retenida", |_| gate.held() == 1);

    cx.update(|cx| Backend::run(cx, |core| async move { core.switch_org("o2").await; }));
    wait_until(cx, "pasa a la otra", |cx| on_screen(window, "org-page-title", cx) && label(window, "org-page-title", cx) == "Cliente");
    gate.open();
    wait_until(cx, "la llave sale igual", |cx| on_screen(window, "org-key", cx) && label(window, "org-key", cx) == "k-new");
    assert_eq!(label(window, "org-key-owner", cx), "Latinet · G Studio");

    click(window, "rail-comms", cx);
    wait_until(cx, "sale el panel de comunicación", |cx| on_screen(window, "comms-channel-a", cx));
    click(window, "rail-organization", cx);
    wait_until(cx, "vuelve la organización", |cx| on_screen(window, "org-page", cx));
    assert!(on_screen(window, "org-key", cx), "reabrir la pantalla tiró la llave sin reconocer");
    click(window, "org-key-saved", cx);
    assert!(!on_screen(window, "org-key", cx));
    stop_stream(cx);
}

/// Dos clics seguidos sobre la lista de a quién le llega una reunión no se
/// pisan: el segundo se calcula sobre el primero y sólo sale cuando éste
/// vuelve.
///
/// Roturas que atrapa: calcular el segundo sobre la lista vieja (que deshace
/// el primero); mandar los dos a la vez.
#[gpui_kit::test]
fn two_quick_clicks_on_who_a_meeting_reaches_do_not_undo_each_other(cx: &mut TestAppContext) {
    let gate = Gate::default();
    let server = org_server("admin");
    server.mount(Mock::given(method("GET")).and(path("/api/v1/organizations/o1/meetings/")).respond_with(ok(json!([a_meeting()]))));
    server.mount(Mock::given(method("PUT")).and(path("/api/v1/meetings/m1/recipients")).respond_with(gate.hold(ok(json!(null)))));
    let (server, window) = open_org_with(server, cx);
    click(window, "org-tab-meetings", cx);
    wait_until(cx, "sale la reunión", |cx| on_screen(window, "org-meeting-m1", cx));
    click(window, "org-meeting-people-m1", cx);
    wait_until(cx, "salen los miembros", |cx| on_screen(window, "org-meeting-person-m1-u1", cx));

    // Parte de «u2 excluido».
    click(window, "org-meeting-person-m1-u1", cx);
    wait_until(cx, "viaja el primero", |_| gate.held() == 1);
    click(window, "org-meeting-person-m1-u2", cx);
    cx.run_until_parked();
    assert_eq!(count_of(&server, "PUT", "/api/v1/meetings/m1/recipients"), 1, "mandó el segundo sin esperar al primero");

    gate.open();
    wait_until(cx, "sale el segundo", |_| count_of(&server, "PUT", "/api/v1/meetings/m1/recipients") == 2);
    let bodies: Vec<serde_json::Value> = server.requests().iter().filter(|request| request.method.as_str() == "PUT" && request.url.path() == "/api/v1/meetings/m1/recipients").map(|request| serde_json::from_slice(&request.body).unwrap()).collect();
    assert_eq!(bodies, [json!({ "excludedUserIds": ["u2", "u1"] }), json!({ "excludedUserIds": ["u1"] })]);
    stop_stream(cx);
}

/// Dos casillas del mismo repo pulsadas seguidas se guardan juntas: el
/// servidor reemplaza el repo entero, y el segundo cambio sobre el repo viejo
/// habría deshecho el primero.
///
/// Roturas que atrapa: mandar los dos PATCH a la vez; el segundo calculado
/// sobre el repo de antes del primero.
#[gpui_kit::test]
fn two_quick_checkboxes_on_a_repo_are_saved_one_after_the_other(cx: &mut TestAppContext) {
    let gate = Gate::default();
    let linked = |bare: bool, docs: bool| json!({ "id": "r1", "installationId": 5, "repoId": 100, "fullName": "dwit/api", "orgId": "o1", "spaceId": "a", "bareRefs": bare, "showDocs": docs });
    let mut status = github_status(true, json!([]));
    status["repos"] = json!([linked(false, false)]);
    let server = org_server("admin");
    server.mount(Mock::given(method("GET")).and(path("/api/v1/organizations/o1/github/")).respond_with(ok(status)));
    server.mount(Mock::given(method("PATCH")).and(path("/api/v1/organizations/o1/github/repos/r1")).respond_with(gate.hold(ok(linked(true, false)))));
    let (server, window) = open_org_with(server, cx);
    click(window, "org-tab-github", cx);
    wait_until(cx, "sale el repo", |cx| on_screen(window, "org-github-docs-r1", cx));

    click(window, "org-github-bare-r1", cx);
    wait_until(cx, "viaja el primero", |_| gate.held() == 1);
    click(window, "org-github-docs-r1", cx);
    cx.run_until_parked();
    assert_eq!(count_of(&server, "PATCH", "/api/v1/organizations/o1/github/repos/r1"), 1, "mandó el segundo sin esperar al primero");

    gate.open();
    wait_until(cx, "sale el segundo", |_| count_of(&server, "PATCH", "/api/v1/organizations/o1/github/repos/r1") == 2);
    let bodies: Vec<serde_json::Value> = server.requests().iter().filter(|request| request.method.as_str() == "PATCH").map(|request| serde_json::from_slice(&request.body).unwrap()).collect();
    assert_eq!(bodies, [json!({ "spaceId": "a", "bareRefs": true, "showDocs": false }), json!({ "spaceId": "a", "bareRefs": true, "showDocs": true })]);
    stop_stream(cx);
}

const BASECAMP: &str = "/api/v1/organizations/o1/basecamp";

fn basecamp_status(configured: bool, connected: bool, projects: serde_json::Value) -> serde_json::Value {
    if !connected {
        return json!({ "configured": configured, "connected": false, "projects": [], "unmatched": 0 });
    }
    json!({ "configured": configured, "connected": true, "accountId": 99, "accountName": "DWIT", "identityName": "Ana Ruiz", "projects": projects, "unmatched": 1 })
}

/// Una organización con Basecamp: su estado, los proyectos de la cuenta y su
/// gente. Los PUT y los DELETE se montan en cada prueba, con lo que contestan.
fn basecamp_server(role: &str, status: serde_json::Value, projects: serde_json::Value, people: serde_json::Value) -> FakeServer {
    let server = org_server(role);
    server.mount(Mock::given(method("GET")).and(path(BASECAMP)).respond_with(ok(status)));
    server.mount(Mock::given(method("GET")).and(path(format!("{BASECAMP}/projects"))).respond_with(ok(projects)));
    server.mount(Mock::given(method("GET")).and(path(format!("{BASECAMP}/people"))).respond_with(ok(people)));
    server
}

/// Sale de la ventana y vuelve a ella, como quien va al navegador a
/// autorizar y regresa.
fn come_back_to_the_window(window: AnyWindowHandle, cx: &mut TestAppContext) {
    gpui_kit::VisualTestContext::from_window(window.into(), cx).deactivate_window();
    cx.run_until_parked();
    cx.update_window(window, |_, window, _| window.activate_window()).unwrap();
}

/// Sin las credenciales de 37signals en el servidor se dice, y no queda
/// ningún botón que no lleve a nada.
///
/// Rotura que atrapa: ofrecer conectar (o desconectar) con Basecamp sin
/// configurar en el servidor.
#[gpui_kit::test]
fn without_basecamp_credentials_the_tab_says_so_and_offers_nothing(cx: &mut TestAppContext) {
    let (_server, window) = open_org_with(basecamp_server("admin", basecamp_status(false, false, json!([])), json!([]), json!([])), cx);
    click(window, "org-tab-basecamp", cx);
    wait_until(cx, "lo dice", |cx| on_screen(window, "org-basecamp-off", cx));
    assert!(!on_screen(window, "org-basecamp-connect", cx) && !on_screen(window, "org-basecamp-disconnect", cx));
    stop_stream(cx);
}

/// Sólo quien administra ve la pestaña de Basecamp.
///
/// Rotura que atrapa: enseñarle la pestaña (con la gente y los proyectos de
/// la cuenta) a un miembro; que el admin no la vea.
#[gpui_kit::test]
fn only_an_admin_sees_the_basecamp_tab(cx: &mut TestAppContext) {
    let (server, window) = open_org_with(basecamp_server("member", basecamp_status(true, false, json!([])), json!([]), json!([])), cx);
    assert!(on_screen(window, "org-tab-github", cx));
    assert!(!on_screen(window, "org-tab-basecamp", cx), "un miembro ve la pestaña de Basecamp");
    assert_eq!(count_of(&server, "GET", BASECAMP), 0, "pidió Basecamp para quien no administra");
    stop_stream(cx);

    let (_server, window) = open_org_with(basecamp_server("admin", basecamp_status(true, false, json!([])), json!([]), json!([])), cx);
    assert!(on_screen(window, "org-tab-basecamp", cx), "un admin no ve la pestaña de Basecamp");
    stop_stream(cx);
}

/// Conectar abre en el navegador la dirección del servidor y, al volver a la
/// ventana (y sólo entonces), la pantalla vuelve a leer y enseña la cuenta
/// conectada con sus proyectos.
///
/// Roturas que atrapa: conectar sin pedir la dirección o sin abrirla; la
/// pantalla que sigue diciendo «sin conectar» con la cuenta ya conectada
/// (no volver a leer al volver); volver a leer en cada vuelta a la ventana,
/// con o sin haber ido a autorizar.
#[gpui_kit::test]
fn connecting_opens_the_browser_and_coming_back_shows_the_real_state(cx: &mut TestAppContext) {
    let server = org_server("admin");
    server.mount(Mock::given(method("GET")).and(path(BASECAMP)).respond_with(ok(basecamp_status(true, false, json!([])))).up_to_n_times(1));
    server.mount(Mock::given(method("GET")).and(path(BASECAMP)).respond_with(ok(basecamp_status(true, true, json!([])))));
    server.mount(Mock::given(method("GET")).and(path(format!("{BASECAMP}/projects"))).respond_with(ok(json!([{ "id": 7, "name": "Sitio", "appUrl": "https://3.basecamp.com/99/projects/7" }]))));
    server.mount(Mock::given(method("GET")).and(path(format!("{BASECAMP}/people"))).respond_with(ok(json!([]))));
    server.mount(Mock::given(method("POST")).and(path(format!("{BASECAMP}/connect"))).respond_with(ok(json!({ "url": "https://launchpad.37signals.com/authorization/new?state=s" }))));
    let (server, window) = open_org_with(server, cx);
    click(window, "org-tab-basecamp", cx);
    wait_until(cx, "sale sin conectar", |cx| on_screen(window, "org-basecamp-connect", cx));
    assert!(!on_screen(window, "org-basecamp-disconnect", cx));

    // Ir y volver sin haber pedido autorizar no vuelve a leer.
    come_back_to_the_window(window, cx);
    cx.run_until_parked();
    assert_eq!(count_of(&server, "GET", BASECAMP), 1, "volvió a leer sin haber ido a autorizar");

    click(window, "org-basecamp-connect", cx);
    wait_until(cx, "se abre fuera", |cx| cx.opened_url().is_some());
    assert_eq!(cx.opened_url().as_deref(), Some("https://launchpad.37signals.com/authorization/new?state=s"));
    assert_eq!(request_body(&server, "POST", &format!("{BASECAMP}/connect")), json!({}));
    assert_eq!(count_of(&server, "GET", BASECAMP), 1, "leyó antes de volver");
    assert!(on_screen(window, "org-basecamp-connect", cx));

    come_back_to_the_window(window, cx);
    wait_until(cx, "enseña la cuenta conectada", |cx| on_screen(window, "org-basecamp-disconnect", cx));
    assert!(label(window, "org-basecamp-line", cx).contains("DWIT"), "{}", label(window, "org-basecamp-line", cx));
    assert!(!on_screen(window, "org-basecamp-connect", cx), "sigue ofreciendo conectar con la cuenta conectada");
    wait_until(cx, "salen sus proyectos", |cx| on_screen(window, "org-basecamp-project-7", cx));
    stop_stream(cx);
}

/// Un servidor que rechaza conectar se dice con su motivo, y no se abre nada.
///
/// Roturas que atrapa: abrir el navegador con el error; callar el motivo
/// (decir «falló» a secas o nada).
#[gpui_kit::test]
fn a_refused_connection_says_why_and_opens_nothing(cx: &mut TestAppContext) {
    let server = basecamp_server("admin", basecamp_status(true, false, json!([])), json!([]), json!([]));
    server.mount(Mock::given(method("POST")).and(path(format!("{BASECAMP}/connect"))).respond_with(wiremock::ResponseTemplate::new(503).set_body_json(json!({ "success": false, "error": "basecamp-off", "message": "No" }))));
    let (_server, window) = open_org_with(server, cx);
    click(window, "org-tab-basecamp", cx);
    wait_until(cx, "sale conectar", |cx| on_screen(window, "org-basecamp-connect", cx));
    click(window, "org-basecamp-connect", cx);
    wait_until(cx, "lo dice", |cx| on_screen(window, "org-notice", cx));
    assert_eq!(label(window, "org-notice", cx), "Este servidor de cac aún no tiene Basecamp configurado.");
    assert!(cx.opened_url().is_none(), "abrió el navegador con el servidor diciendo que no");
    stop_stream(cx);
}

/// Enlazar un proyecto manda sólo el espacio elegido, al proyecto de esa
/// fila, y dice lo que contestó el servidor (un webhook que no se dio de alta
/// es un problema); dejarlo «sin enlazar» lo quita y no vuelve a enlazar.
///
/// Roturas que atrapa: el cuerpo sin el espacio o con otro campo; el
/// proyecto equivocado en la ruta; un webhook sin dar de alta dicho como
/// éxito; quitar con PUT o no quitar; enlazar al elegir lo mismo que ya está.
#[gpui_kit::test]
fn linking_a_project_sends_its_space_and_unlinking_removes_it(cx: &mut TestAppContext) {
    let projects = json!([
        { "id": 7, "name": "Sitio", "appUrl": "https://x/7" },
        { "id": 8, "name": "App", "appUrl": "https://x/8", "spaceId": "a" },
    ]);
    let status = basecamp_status(true, true, json!([{ "id": "l8", "spaceId": "a", "projectId": 8, "name": "App", "webhookId": 55 }]));
    let server = basecamp_server("admin", status, projects, json!([]));
    server.mount(Mock::given(method("PUT")).and(path(format!("{BASECAMP}/projects/7"))).respond_with(ok(json!({ "id": "l7", "spaceId": "a", "projectId": 7, "name": "Sitio", "webhookId": 0 }))));
    server.mount(Mock::given(method("DELETE")).and(path(format!("{BASECAMP}/projects/8"))).respond_with(wiremock::ResponseTemplate::new(204)));
    let (server, window) = open_org_with(server, cx);
    click(window, "org-tab-basecamp", cx);
    wait_until(cx, "salen los proyectos", |cx| on_screen(window, "org-basecamp-space-7", cx));
    assert!(on_screen(window, "org-basecamp-hook-8", cx) && !on_screen(window, "org-basecamp-hook-7", cx), "el webhook se dice sólo del enlazado");

    click(window, "org-basecamp-space-7", cx);
    wait_until(cx, "se abre el menú", |cx| on_screen(window, "org-basecamp-space-7-a", cx));
    click(window, "org-basecamp-space-7-a", cx);
    wait_until(cx, "lo enlaza", |_| count_of(&server, "PUT", &format!("{BASECAMP}/projects/7")) == 1);
    assert_eq!(request_body(&server, "PUT", &format!("{BASECAMP}/projects/7")), json!({ "spaceId": "a" }));
    wait_until(cx, "lo dice como problema", |cx| on_screen(window, "org-notice", cx));
    let said = label(window, "org-notice", cx);
    assert!(said.contains("Sitio") && said.contains("Producto") && said.contains("webhook no se dio de alta"), "{said}");

    // Elegir el mismo espacio que ya tiene no manda nada.
    click(window, "org-basecamp-space-8", cx);
    wait_until(cx, "se abre el menú", |cx| on_screen(window, "org-basecamp-space-8-a", cx));
    click(window, "org-basecamp-space-8-a", cx);
    cx.run_until_parked();
    assert_eq!(count_of(&server, "PUT", &format!("{BASECAMP}/projects/8")), 0, "enlazó el que ya estaba enlazado");

    click(window, "org-basecamp-space-8", cx);
    wait_until(cx, "se abre el menú", |cx| on_screen(window, "org-basecamp-space-8-", cx));
    click(window, "org-basecamp-space-8-", cx);
    wait_until(cx, "lo quita", |_| count_of(&server, "DELETE", &format!("{BASECAMP}/projects/8")) == 1);
    assert_eq!(count_of(&server, "PUT", &format!("{BASECAMP}/projects/8")), 0, "quitar volvió a enlazar");
    stop_stream(cx);
}

/// Una persona que no casó se asigna a un miembro buscado por su nombre; las
/// ya asignadas no salen.
///
/// Roturas que atrapa: listar a quien ya tiene usuario; el cuerpo sin el
/// usuario elegido o a otra persona; el buscador que no filtra (o que busca
/// por el usuario de acceso en vez del nombre que se enseña); asignar sin
/// elegir miembro.
#[gpui_kit::test]
fn an_unmatched_person_is_assigned_to_a_member_found_by_name(cx: &mut TestAppContext) {
    let people = json!([
        { "id": "a", "personId": 31, "name": "Bob B", "email": "bobb@acme.io", "client": false, "userId": "" },
        { "id": "b", "personId": 32, "name": "Casada", "email": "ana@x.com", "client": false, "userId": "u1", "matchedBy": "email" },
    ]);
    let server = basecamp_server("admin", basecamp_status(true, true, json!([])), json!([]), people);
    server.mount(Mock::given(method("PUT")).and(path(format!("{BASECAMP}/people/31"))).respond_with(ok(json!({ "personId": 31, "name": "Bob B", "userId": "u1", "matchedBy": "manual" }))));
    let (server, window) = open_org_with(server, cx);
    click(window, "org-tab-basecamp", cx);
    wait_until(cx, "sale la persona", |cx| on_screen(window, "org-basecamp-person-31", cx));
    assert!(!on_screen(window, "org-basecamp-person-32", cx), "listó a una persona ya asignada");

    click(window, "org-basecamp-assign-open-31", cx);
    wait_until(cx, "salen los miembros", |cx| on_screen(window, "org-basecamp-assign-31-u1", cx) && on_screen(window, "org-basecamp-assign-31-u2", cx));
    type_into(window, "org-basecamp-assign-query-31", "ruiz", cx);
    wait_until(cx, "filtra por el nombre", |cx| !on_screen(window, "org-basecamp-assign-31-u2", cx));
    assert!(on_screen(window, "org-basecamp-assign-31-u1", cx));
    assert_eq!(count_of(&server, "PUT", &format!("{BASECAMP}/people/31")), 0, "asignó sin elegir");

    click(window, "org-basecamp-assign-31-u1", cx);
    wait_until(cx, "la asigna", |_| count_of(&server, "PUT", &format!("{BASECAMP}/people/31")) == 1);
    assert_eq!(request_body(&server, "PUT", &format!("{BASECAMP}/people/31")), json!({ "userId": "u1" }));
    wait_until(cx, "lo dice", |cx| on_screen(window, "org-notice", cx));
    assert!(label(window, "org-notice", cx).contains("Ana Ruiz"), "{}", label(window, "org-notice", cx));
    stop_stream(cx);
}

/// Cuando el servidor no puede leer los proyectos o la gente, cada lista lo
/// dice con su motivo y no tapa a la otra; y sin nadie por asignar se dice.
///
/// Roturas que atrapa: una lista que falla y deja a la otra vacía o a medio
/// pintar; un error mudo (la lista vacía en vez del motivo); la gente sin
/// asignar que no dice que no queda nadie.
#[gpui_kit::test]
fn a_list_that_fails_says_why_and_does_not_hide_the_other(cx: &mut TestAppContext) {
    let server = org_server("admin");
    server.mount(Mock::given(method("GET")).and(path(BASECAMP)).respond_with(ok(basecamp_status(true, true, json!([])))));
    server.mount(Mock::given(method("GET")).and(path(format!("{BASECAMP}/projects"))).respond_with(wiremock::ResponseTemplate::new(502).set_body_json(json!({ "success": false, "error": "basecamp-unreachable", "message": "No" }))));
    server.mount(Mock::given(method("GET")).and(path(format!("{BASECAMP}/people"))).respond_with(ok(json!([{ "id": "b", "personId": 32, "name": "Casada", "email": "a@x.com", "userId": "u1" }]))));
    let (_server, window) = open_org_with(server, cx);
    click(window, "org-tab-basecamp", cx);
    wait_until(cx, "sale el error de los proyectos", |cx| on_screen(window, "org-basecamp-projects-error", cx));
    wait_until(cx, "dice que nadie falta", |cx| on_screen(window, "org-basecamp-people-none", cx));
    assert!(!on_screen(window, "org-basecamp-people-error", cx));
    stop_stream(cx);
}

/// Desconectar pregunta antes: cancelar no toca nada y aceptar quita la
/// conexión.
///
/// Roturas que atrapa: desconectar sin preguntar o al cancelar; no mandar el
/// DELETE al aceptar.
#[gpui_kit::test]
fn disconnecting_asks_first(cx: &mut TestAppContext) {
    let server = basecamp_server("admin", basecamp_status(true, true, json!([])), json!([]), json!([]));
    server.mount(Mock::given(method("DELETE")).and(path(BASECAMP)).respond_with(wiremock::ResponseTemplate::new(204)));
    let (server, window) = open_org_with(server, cx);
    click(window, "org-tab-basecamp", cx);
    wait_until(cx, "sale desconectar", |cx| on_screen(window, "org-basecamp-disconnect", cx));
    click(window, "org-basecamp-disconnect", cx);
    let (title, _) = cx.pending_prompt().expect("desconectó sin preguntar");
    assert!(title.contains("Basecamp"), "{title}");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert_eq!(count_of(&server, "DELETE", BASECAMP), 0, "desconectó al cancelar");
    click(window, "org-basecamp-disconnect", cx);
    cx.simulate_prompt_answer("Desconectar");
    wait_until(cx, "desconecta", |_| count_of(&server, "DELETE", BASECAMP) == 1);
    stop_stream(cx);
}

/// Lo mismo al revés: la gente que no se pudo leer lo dice con su motivo y
/// los proyectos se siguen viendo.
///
/// Roturas que atrapa: la lista de gente que falla y se pinta como «todos
/// casados» (un error mudo); que el fallo de una tape los proyectos.
#[gpui_kit::test]
fn people_that_fail_to_load_say_why_and_the_projects_stay(cx: &mut TestAppContext) {
    let server = org_server("admin");
    server.mount(Mock::given(method("GET")).and(path(BASECAMP)).respond_with(ok(basecamp_status(true, true, json!([])))));
    server.mount(Mock::given(method("GET")).and(path(format!("{BASECAMP}/projects"))).respond_with(ok(json!([{ "id": 7, "name": "Sitio", "appUrl": "https://x/7" }]))));
    server.mount(Mock::given(method("GET")).and(path(format!("{BASECAMP}/people"))).respond_with(wiremock::ResponseTemplate::new(502).set_body_json(json!({ "success": false, "error": "basecamp-unreachable", "message": "No" }))));
    let (_server, window) = open_org_with(server, cx);
    click(window, "org-tab-basecamp", cx);
    wait_until(cx, "sale el error de la gente", |cx| on_screen(window, "org-basecamp-people-error", cx));
    assert!(on_screen(window, "org-basecamp-project-7", cx) && !on_screen(window, "org-basecamp-people-none", cx));
    assert!(!on_screen(window, "org-basecamp-projects-error", cx));
    stop_stream(cx);
}
