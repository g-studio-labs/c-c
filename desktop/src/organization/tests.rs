// Las pruebas de la pantalla de la organización y de las invitaciones
// recibidas. Se incluyen en las de `workspace.rs` (con `include!`) para usar
// su servidor de mentira y sus ayudas.

/// Una organización `o1` en la que Ana tiene `role`, con el espacio `a`,
/// dos miembros (Ana y Bob) y una invitación enviada a Caro, ya caducada.
/// Y otra, `o2` («Cliente»), de la que Ana es miembro.
fn org_server(role: &str) -> FakeServer {
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
    server.mount(get("/api/v1/report-projects/").respond_with(ok(json!([]))));
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
/// documento en el de Documentación) lleva a lo pulsado.
///
/// Rotura que atrapa: la pantalla de la organización que se queda encima
/// de lo que se abrió desde el panel.
#[gpui_kit::test]
fn a_click_in_another_modules_panel_leaves_the_organization(cx: &mut TestAppContext) {
    let (_server, window) = open_org("member", cx);
    click(window, "rail-docs", cx);
    wait_until(cx, "sale el panel de Documentación", |cx| on_screen(window, "docs-node-a", cx));
    click(window, "rail-organization", cx);
    wait_until(cx, "vuelve la organización", |cx| on_screen(window, "org-page", cx));
    assert!(on_screen(window, "docs-node-a", cx), "el panel deja de ser el del módulo");
    click(window, "docs-node-a", cx);
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
/// Rotura que atrapa: cerrar la pantalla porque el core vuelve a elegir
/// conversación, ámbito o documento al cambiar de organización.
#[gpui_kit::test]
fn switching_organization_keeps_its_screen(cx: &mut TestAppContext) {
    let (_server, window) = open_org("admin", cx);
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
