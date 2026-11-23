// Las pruebas de Actividad, Diagnóstico y Usuarios. Se incluyen dentro de
// `workspace::tests` (con `include!`) para usar su arnés sin copiarlo, en un
// fichero aparte para no pisarse con las otras fases de Plataforma.

fn ci_run(id: &str, run_id: i64, status: &str, at: &str) -> serde_json::Value {
    json!({ "id": id, "orgId": "o1", "repoFullName": "guz/cac", "runId": run_id, "runAttempt": 1, "runNumber": 7, "workflowName": "CI",
            "path": ".github/workflows/ci.yml", "event": "push", "status": status, "conclusion": if status == "completed" { "success" } else { "" },
            "headSha": "abcdef1234567", "headBranch": "main", "commitTitle": "arregla el login", "actor": "ana", "htmlUrl": "https://github.com/guz/cac/actions/runs/1", "occurredAt": at })
}

/// Actividad se abre desde el panel con lo que hay, y un run que empieza
/// mientras se mira entra sin recargar; cuando termina, cambia en su fila.
///
/// Roturas que atrapa: una fila del panel que no abre nada; no pintar lo
/// que llega por `ci:run` hasta volver a entrar; volver a pedir la página
/// por cada evento.
#[gpui_kit::test]
fn a_new_ci_run_enters_activity_without_reloading(cx: &mut TestAppContext) {
    let (server, window) = platform_as("member", cx);
    let page = json!({ "items": [{ "kind": "run", "at": "2026-11-20T10:00:00Z", "run": ci_run("r1", 100, "completed", "2026-11-20T10:00:00Z") }], "hasMore": false });
    server.mount(Mock::given(method("GET")).and(path("/api/v1/organizations/o1/activity/")).respond_with(ok(page)));
    click(window, "platform-activity", cx);
    wait_until(cx, "sale el run", |cx| on_screen(window, "activity-run-r1", cx));
    assert_eq!(label(window, "activity-run-r1", cx), "Pasó");

    let started = cac_core::ServerEvent { event: "ci:run".into(), data: ci_run("r2", 200, "in_progress", "2026-11-20T11:00:00Z").to_string() };
    cx.update(|cx| Backend::run(cx, move |core| async move { core.stream().deliver(started) }));
    wait_until(cx, "entra el run nuevo", |cx| on_screen(window, "activity-run-r2", cx));
    assert_eq!(label(window, "activity-run-r2", cx), "En curso");
    let finished = cac_core::ServerEvent { event: "ci:run".into(), data: ci_run("r2", 200, "completed", "2026-11-20T11:00:00Z").to_string() };
    cx.update(|cx| Backend::run(cx, move |core| async move { core.stream().deliver(finished) }));
    wait_until(cx, "cambia en su fila", |cx| label(window, "activity-run-r2", cx) == "Pasó");
    assert_eq!(server.queries("/api/v1/organizations/o1/activity/").len(), 1, "volvió a pedir la página");
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// Un superadmin ve a la gente; borrar a alguien pregunta nombrándolo, y
/// cancelar no borra. A sí mismo no se puede borrar.
///
/// Roturas que atrapa: borrar sin preguntar; una pregunta que no dice a
/// quién; borrar al cancelar; dejar a la persona borrada en la tabla;
/// ofrecer borrarse a uno mismo.
#[gpui_kit::test]
fn deleting_a_user_asks_naming_them(cx: &mut TestAppContext) {
    let (server, window) = platform_as("member", cx);
    cx.update(|cx| {
        let boss = serde_json::from_value(json!({ "id": "u1", "username": "ana", "superadmin": true })).unwrap();
        Backend::core(cx).update_session(boss).unwrap();
    });
    let person = |id: &str, username: &str| json!({ "id": id, "username": username, "email": format!("{username}@x.com"), "name": username, "isSuperadmin": false, "createdAt": "2026-01-01T00:00:00Z" });
    server.mount(Mock::given(method("GET")).and(path("/api/v1/users/")).respond_with(ok(json!([person("u1", "ana"), person("u2", "bob")]))));
    server.mount(Mock::given(method("DELETE")).and(path("/api/v1/users/u2")).respond_with(ok(json!(null))));
    wait_until(cx, "sale «Usuarios»", |cx| on_screen(window, "platform-users", cx));
    click(window, "platform-users", cx);
    wait_until(cx, "llega la gente", |cx| on_screen(window, "user-row-u2", cx));
    let deletes = |server: &FakeServer| server.requests().iter().filter(|request| request.method.as_str() == "DELETE").count();

    click(window, "user-delete-u1", cx);
    assert!(cx.pending_prompt().is_none(), "ofrece borrarse a uno mismo");

    click(window, "user-delete-u2", cx);
    let (title, body) = cx.pending_prompt().expect("borró sin preguntar");
    assert_eq!(title, "¿Borrar a @bob?");
    assert!(body.contains("No se puede deshacer"), "{body}");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(deletes(&server), 0, "borró al cancelar");

    click(window, "user-delete-u2", cx);
    cx.simulate_prompt_answer("Borrar");
    wait_until(cx, "sale de la tabla", |cx| !on_screen(window, "user-row-u2", cx));
    assert_eq!(deletes(&server), 1);
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// La ficha de un dispositivo pinta en su tira de latidos los huecos que
/// dice el backend, y el que sigue abierto aparte.
///
/// Roturas que atrapa: una tira sin huecos (sólo puntos); no pintar la
/// ficha al elegir el dispositivo.
#[gpui_kit::test]
fn the_heartbeat_strip_paints_the_gaps(cx: &mut TestAppContext) {
    let (server, window) = platform_as("member", cx);
    let device = json!({ "deviceId": "dev-1", "projectId": "p1", "projectName": "Rastreo", "label": "moto g54", "platform": "android", "appVersion": "2.3.0",
                         "batches": 3, "reqCount": 10, "errorCount": 0, "warnCount": 0, "lastSeen": "2026-11-20T10:00:00Z", "alerts": [] });
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    server.mount(get("/api/v1/telemetry/devices").respond_with(ok(json!([device.clone()]))));
    let now = chrono::Utc::now();
    let ago = |minutes: i64| (now - chrono::Duration::minutes(minutes)).to_rfc3339();
    let mut detail = device;
    detail["firstSeen"] = json!(ago(600));
    detail["device"] = json!({ "battery": { "level": 80 } });
    detail["heartbeat"] = json!({ "intervalSeconds": 300, "graceSeconds": 60 });
    detail["heartbeats"] = json!([ago(240), ago(235), ago(120), ago(115)]);
    detail["gaps"] = json!([{ "from": ago(235), "to": ago(120), "seconds": 6900 }, { "from": ago(115), "to": ago(0), "seconds": 6900, "open": true }]);
    server.mount(get("/api/v1/telemetry/devices/p1/dev-1").respond_with(ok(detail)));
    server.mount(get("/api/v1/telemetry/timeline").respond_with(ok(json!([]))));

    click(window, "platform-diagnostics", cx);
    wait_until(cx, "llega el dispositivo", |cx| on_screen(window, "diag-device-p1-dev-1", cx));
    click(window, "diag-device-p1-dev-1", cx);
    wait_until(cx, "sale la tira", |cx| on_screen(window, "diag-strip", cx));
    assert!(on_screen(window, "diag-strip-gap-0", cx) && on_screen(window, "diag-strip-gap-1", cx), "la tira no marca los huecos");
    assert!(on_screen(window, "diag-strip-beat-2", cx));
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}
