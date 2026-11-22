// Las pruebas de la pestaña de backups. Se incluyen dentro de
// `workspace::tests` (con `include!`) para usar su arnés sin copiarlo, en un
// fichero aparte para no pisarse con las otras fases de Plataforma, que
// añaden las suyas al final de `workspace.rs`.

/// Un swarm con agente cuyos servicios son `shop_web` y `shop_db`
/// (postgres): una base `t1` que se respalda en un contenedor suelto, con
/// un backup bueno de Ana. `shop_db` sale como base detectada.
fn backups_as(role: &str, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
    let (server, window) = swarm_servers_with(role, json!([]), &["s1"], true, json!([]), cx);
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    server.mount(get("/api/v1/servers/s1/backups/status").respond_with(ok(json!({ "enabled": true, "agentReady": true, "agentVersion": 14, "minAgent": 9, "hasRecipients": true, "everyHours": [6, 24] }))));
    server.mount(get("/api/v1/servers/s1/backups/targets").respond_with(ok(json!([backup_target("t1", "boaty-postgres", "boaty")]))));
    server.mount(get("/api/v1/servers/s1/backups/recipients").respond_with(ok(json!([{ "id": "r1", "orgId": "o1", "label": "jose", "publicKey": format!("age1{}", "q".repeat(58)) }]))));
    server.mount(get("/api/v1/servers/s1/backups/targets/t1/history").respond_with(ok(json!([backup_run("b1", "succeeded")]))));
    click(window, "server-tab-backups", cx);
    wait_until(cx, "sale la base", |cx| on_screen(window, "backup-target-t1", cx));
    (server, window)
}

fn backup_target(id: &str, service: &str, database: &str) -> serde_json::Value {
    json!({ "id": id, "orgId": "o1", "serverId": "s1", "serviceName": service, "engine": "postgres", "database": database, "credentialEnv": "POSTGRES_USER",
            "username": "", "everyHours": 24, "anchorTime": "03:30", "timezone": "America/Mexico_City", "keepDays": 7, "keepWeekly": 4, "keepMonthly": 6,
            "restoreTestEveryDays": 7, "enabled": true, "lastSuccessAt": "2026-11-21T03:35:00Z" })
}

fn backup_run(id: &str, status: &str) -> serde_json::Value {
    json!({ "id": id, "createdAt": "2026-11-21T03:30:00Z", "orgId": "o1", "serverId": "s1", "targetId": "t1", "kind": "scheduled", "status": status, "retentionClass": "regular", "sizeBytes": 2048, "error": "" })
}

/// Quien administra configura una base detectada con lo que se deduce de
/// su imagen; probar la restauración y borrar una base preguntan, y
/// cancelar no manda nada.
///
/// Roturas que atrapa: mandar la base con otro servicio, motor o variable
/// que los de la imagen detectada; probar la restauración o borrar sin
/// preguntar; borrar al cancelar; dejar la tarjeta borrada en pantalla.
#[gpui_kit::test]
fn an_admin_configures_a_detected_database_and_deleting_asks(cx: &mut TestAppContext) {
    let (server, window) = backups_as("admin", cx);
    server.mount(
        Mock::given(method("POST"))
            .and(path("/api/v1/servers/s1/backups/targets"))
            .and(wiremock::matchers::body_json(json!({ "serviceName": "shop_db", "engine": "postgres", "database": "shop", "credentialEnv": "POSTGRES_USER", "username": "",
                "everyHours": 24, "anchorTime": "03:30", "timezone": "America/Mexico_City", "keepDays": 7, "keepWeekly": 4, "keepMonthly": 6, "restoreTestEveryDays": 7, "enabled": true })))
            .respond_with(ok(backup_target("t2", "shop_db", "shop"))),
    );
    server.mount(Mock::given(method("DELETE")).and(path("/api/v1/servers/s1/backups/targets/t1")).respond_with(wiremock::ResponseTemplate::new(204)));

    click(window, "backup-configure-shop_db", cx);
    wait_until(cx, "se abre el formulario", |cx| on_screen(window, "backup-form", cx));
    type_text(window, "shop", cx);
    click(window, "backup-form-save", cx);
    wait_until(cx, "sale la nueva", |cx| on_screen(window, "backup-target-t2", cx));
    assert!(!on_screen(window, "backup-form", cx), "el formulario se quedó abierto");
    assert_eq!(server.hits("/api/v1/servers/s1/backups/targets"), 2, "una lectura y un alta");

    click(window, "backup-test-t1", cx);
    let (title, _) = cx.pending_prompt().expect("probó la restauración sin preguntar");
    assert_eq!(title, "¿Probar la restauración ahora?");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert_eq!(server.hits("/api/v1/servers/s1/backups/targets/t1/test"), 0, "probó al cancelar");

    click(window, "backup-delete-t1", cx);
    let (title, _) = cx.pending_prompt().expect("borró sin preguntar");
    assert_eq!(title, "¿Dejar de respaldar boaty?");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert_eq!(server.hits("/api/v1/servers/s1/backups/targets/t1"), 0, "borró al cancelar");
    click(window, "backup-delete-t1", cx);
    cx.simulate_prompt_answer("Borrar");
    wait_until(cx, "se va la tarjeta", |cx| !on_screen(window, "backup-target-t1", cx));
    assert_eq!(server.hits("/api/v1/servers/s1/backups/targets/t1"), 1);
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// Un lector ve el historial y su log, que siguen al stream, pero no puede
/// lanzar ni tocar nada.
///
/// Roturas que atrapa: ofrecerle a un lector correr, probar, editar, borrar
/// o añadir; que un `backup:status` no cambie la fila en pantalla; que el
/// log no siga lo que llega por el stream.
#[gpui_kit::test]
fn a_reader_watches_history_and_log_follow_the_stream(cx: &mut TestAppContext) {
    let (server, window) = backups_as("viewer", cx);
    wait_until(cx, "sale el historial", |cx| on_screen(window, "backup-row-b1", cx));
    assert_eq!(label(window, "backup-row-b1", cx), "Hecho");
    for id in ["backup-run-t1", "backup-test-t1", "backup-edit-t1", "backup-delete-t1", "backup-add", "backup-configure-shop_db"] {
        assert!(!on_screen(window, id, cx), "un lector ve {id}");
    }

    let started = cac_core::ServerEvent { event: "backup:status".into(), data: backup_run("b2", "running").to_string() };
    cx.update(|cx| Backend::run(cx, move |core| async move { core.stream().deliver(started) }));
    wait_until(cx, "sale el nuevo", |cx| on_screen(window, "backup-row-b2", cx));
    assert_eq!(label(window, "backup-row-b2", cx), "En curso");

    server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/s1/backups/runs/b2")).respond_with(ok(json!({ "id": "b2", "log": "dump\n" }))));
    click(window, "backup-log-b2", cx);
    wait_until(cx, "sale el log", |cx| on_screen(window, "backup-log-line-0", cx));
    let more = cac_core::ServerEvent { event: "backup:log".into(), data: json!({ "backupId": "b2", "targetId": "t1", "lines": ["encrypt"] }).to_string() };
    cx.update(|cx| Backend::run(cx, move |core| async move { core.stream().deliver(more) }));
    wait_until(cx, "sigue el log", |cx| on_screen(window, "backup-log-line-1", cx));
    assert_eq!(label(window, "backup-log-line-1", cx), "encrypt");
    let finished = cac_core::ServerEvent { event: "backup:status".into(), data: backup_run("b2", "failed").to_string() };
    cx.update(|cx| Backend::run(cx, move |core| async move { core.stream().deliver(finished) }));
    wait_until(cx, "el log dice cómo acabó", |cx| label(window, "backup-log-status", cx) == "Falló");
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}
