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
