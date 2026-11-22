// La pestaña Provisioning en la ventana, contra un backend de mentira y un
// motor de Ansible de mentira: ningún playbook corre de verdad. Lo incluye
// `workspace.rs` dentro de sus pruebas, para usar sus ayudantes. Cada prueba
// dice la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::sync::{Arc, Mutex};

use cac_core::{AnsibleEngine, AnsibleManifest, AnsibleTools, EngineEvent, EngineSink, InventoryHost, ManualRun, PlanGaps};

/// Apunta lo que le piden y contesta como si Ansible hubiera corrido bien.
#[derive(Default)]
struct FakeEngine {
    /// Cada ejecución: el playbook y si era un ensayo.
    runs: Mutex<Vec<(String, bool)>>,
}

fn manifest() -> AnsibleManifest {
    serde_json::from_value(json!({
        "version": 2,
        "playbooks": [
            { "id": "swarm", "file": "swarm.yml", "name": "Swarm", "risk": "change", "verify": "swarm-check", "become": true, "becomePasswordFrom": "OPS_PASSWORD",
              "vars": [ { "name": "domain", "required": true }, { "name": "traefik_version", "default": "3.1" } ] },
            { "id": "swarm-check", "file": "swarm-check.yml", "name": "Comprobar swarm", "risk": "read" },
        ]
    }))
    .unwrap()
}

impl AnsibleEngine for FakeEngine {
    fn manifest(&self, _project_dir: &str) -> Result<AnsibleManifest, String> {
        Ok(manifest())
    }
    fn tools(&self, _project_dir: &str, _venv: Option<&str>) -> AnsibleTools {
        AnsibleTools { platform_supported: true, reason: String::new(), ansible_playbook: Some("/usr/bin/ansible-playbook".into()) }
    }
    fn inventory(&self, _project_dir: &str, _inventory: &str, _venv: Option<&str>) -> Result<Vec<InventoryHost>, String> {
        Ok(vec![])
    }
    fn plan_gaps(&self, _project_dir: &str, _playbook_id: &str, _effective: &str) -> Result<PlanGaps, String> {
        Ok(PlanGaps::default())
    }
    fn run_with_config(&self, _project_dir: &str, playbook_id: &str, _effective: &str, check: bool, sink: EngineSink) -> Result<String, String> {
        self.runs.lock().unwrap().push((playbook_id.to_string(), check));
        let id = playbook_id.to_string();
        std::thread::spawn(move || {
            sink(EngineEvent::Line(format!("TASK [{id}]")));
            sink(EngineEvent::Line("PLAY RECAP *****".into()));
            sink(EngineEvent::Line("pixel-vps : ok=2 changed=0".into()));
            sink(EngineEvent::Exit { code: Some(0), cancelled: false });
        });
        Ok(format!("run-{playbook_id}"))
    }
    fn run_manual(&self, _spec: ManualRun, _sink: EngineSink) -> Result<String, String> {
        Err("v1 en esta prueba no".into())
    }
    fn cancel(&self, _run_id: &str) -> Result<(), String> {
        Ok(())
    }
    fn check_report(&self, _lines: &[String], exit_code: i32) -> serde_json::Value {
        json!({ "hosts": ["pixel-vps"], "ok": 2, "changed": 0, "unreachable": 0, "failed": 0, "changedTasks": [], "paths": [], "diff": "", "exitCode": exit_code, "failures": [], "logTail": "" })
    }
}

fn request_row(id: &str, risk: &str, status: &str, flags: serde_json::Value) -> serde_json::Value {
    json!({
        "id": id, "createdAt": chrono::Utc::now().to_rfc3339(), "serverId": "s1", "playbook": "swarm", "project": "ansible", "risk": risk,
        "verify": "swarm-check", "rollback": "", "configSeq": 4, "reason": "", "requestedBy": "u2", "source": "mcp", "status": status,
        "flags": flags, "autoApproved": false, "requestedByName": "Luis", "grantExpiresAt": "2026-11-21T10:15:00Z",
        "check": { "hosts": ["pixel-vps"], "ok": 3, "changed": 1, "unreachable": 0, "failed": 0, "changedTasks": ["install docker"], "paths": [], "diff": "", "exitCode": 0, "failures": [], "logTail": "" }
    })
}

/// Un swarm `s1` (`pixel-vps`, de mentira) con config v4 en cac, una
/// plantilla y una petición; la pestaña Provisioning abierta con ese papel y
/// la carpeta del proyecto ya escrita.
fn provision_as(role: &str, requests: serde_json::Value, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle, Arc<FakeEngine>) {
    let engine = Arc::new(FakeEngine::default());
    let shared: Arc<dyn AnsibleEngine> = engine.clone();
    cx.update(|cx| cx.set_global(crate::platform::provision_engine::LocalEngine(shared)));
    let (server, window) = swarm_servers_with(role, json!([]), &["s1"], false, json!([]), cx);
    let first = |route: &str, answer: wiremock::ResponseTemplate| Mock::given(method("GET")).and(path(route.to_string())).respond_with(answer).with_priority(2);
    let version = json!({ "id": "v4", "serverId": "s1", "seq": 4, "templateId": "t1", "templateSeq": 1, "vars": { "domain": "x.dev" }, "opRefs": { "OPS_PASSWORD": "op://Infra/pixel/password" }, "source": "app", "createdByName": "Ana", "createdAt": "2026-11-20T10:00:00Z" });
    let older = json!({ "id": "v3", "serverId": "s1", "seq": 3, "templateId": "t1", "templateSeq": 1, "vars": {}, "opRefs": {}, "source": "mcp", "createdByName": "Luis", "createdAt": "2026-11-19T10:00:00Z" });
    server.mount(first("/api/v1/servers/s1/config", ok(json!({ "current": version, "versions": [version, older] }))));
    server.mount(first("/api/v1/servers/s1/config/effective", ok(json!({
        "serverId": "s1", "configSeq": 4, "templateId": "t1", "templateName": "Swarm", "templateSeq": 1, "playbooks": ["swarm"], "vars": { "domain": "x.dev", "traefik_version": "3.1" },
        "opRefs": { "OPS_PASSWORD": "op://Infra/pixel/password" }, "requires": ["OPS_PASSWORD"], "perServer": ["OPS_PASSWORD"], "missing": [],
        "connection": { "name": "pixel-vps", "host": "127.0.0.1", "sshPort": 22, "sshUser": "deploy" }
    }))));
    server.mount(first("/api/v1/organizations/o1/server-templates/", ok(json!([
        { "id": "t1", "orgId": "o1", "name": "Swarm", "current": { "id": "tv1", "templateId": "t1", "seq": 1, "playbooks": ["swarm"], "vars": { "traefik_version": "3.1" }, "requires": ["OPS_PASSWORD"], "perServer": ["OPS_PASSWORD"] } }
    ]))));
    server.mount(first("/api/v1/servers/s1/provisioning-runs", ok(json!([
        { "id": "x1", "playbook": "swarm", "status": "succeeded", "startedAt": "2026-11-20T10:00:00Z", "configSeq": 3, "target": "pixel-vps", "startedByName": "Ana", "applied": { "vars": { "domain": "x.dev" }, "opRefs": {} } }
    ]))));
    server.mount(first("/api/v1/servers/s1/provisioning-requests", ok(requests)));
    click(window, "server-tab-provision", cx);
    wait_until(cx, "abre la pestaña", |cx| on_screen(window, "provision-project", cx));
    click(window, "provision-project", cx);
    type_text(window, "/home/ana/infra/ansible", cx);
    cx.simulate_keystrokes(window, "enter");
    wait_until(cx, "lee el manifiesto", |cx| on_screen(window, "provision-playbook-swarm", cx));
    (server, window, engine)
}

fn writes(server: &FakeServer) -> Vec<(String, String, Option<serde_json::Value>)> {
    server
        .requests()
        .iter()
        .filter(|r| r.method.as_str() != "GET" && !r.url.path().ends_with("/agent-session"))
        .map(|r| (r.method.to_string(), r.url.path().to_string(), serde_json::from_slice(&r.body).ok()))
        .collect()
}

/// Ensayar en persona: pregunta, abre la petición, corre el ensayo con
/// `--check` y manda su informe; la salida se pinta.
///
/// Roturas que atrapa: ensayar sin preguntar o al cancelar; lanzar sin `check` (aplicaría
/// de verdad); no mandar el informe del ensayo (la petición se queda
/// «ensayando»); o una salida que no llega a la pantalla.
#[gpui_kit::test]
fn a_member_rehearses_asking_and_the_check_goes_to_cac(cx: &mut TestAppContext) {
    let (server, window, engine) = provision_as("member", json!([]), cx);
    server.mount(Mock::given(method("POST")).and(path("/api/v1/servers/s1/provisioning-requests")).respond_with(ok(request_row("r9", "change", "checking", json!([])))));
    server.mount(Mock::given(method("PUT")).and(path("/api/v1/servers/s1/provisioning-requests/r9/check")).respond_with(ok(request_row("r9", "change", "awaiting_approval", json!([])))));
    wait_until(cx, "lo que falta se calculó", |cx| on_screen(window, "provision-with-config", cx));
    assert_eq!(label(window, "provision-with-config", cx), "Con la config v4 de cac, contra deploy@127.0.0.1:22.");

    click(window, "provision-run", cx);
    let (title, _) = cx.pending_prompt().expect("ensayó sin preguntar");
    assert_eq!(title, "¿Ensayar «Swarm» en pixel-vps?");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert!(engine.runs.lock().unwrap().is_empty() && writes(&server).is_empty(), "ensayó al cancelar");
    click(window, "provision-run", cx);
    cx.simulate_prompt_answer("Ensayar");
    wait_until(cx, "manda el ensayo", |_| server.hits("/api/v1/servers/s1/provisioning-requests/r9/check") == 1);
    assert_eq!(*engine.runs.lock().unwrap(), [("swarm".to_string(), true)], "el ensayo no fue con --check");
    wait_until(cx, "se pinta la salida", |cx| on_screen(window, "provision-output-line-0", cx));
    assert_eq!(label(window, "provision-output-line-0", cx), "TASK [swarm]");
    wait_until(cx, "dice que se mandó", |cx| on_screen(window, "provision-line", cx));
    assert!(!writes(&server).iter().any(|(_, route, _)| route.ends_with("/provisioning-runs")), "un ensayo abrió una ejecución en cac");
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// Rotura que atrapa: ofrecerle aplicar, aprobar o guardar la config a un
/// lector, o no decirle quién aprueba.
#[gpui_kit::test]
fn a_reader_sees_everything_but_cannot_run_decide_or_save(cx: &mut TestAppContext) {
    let (_server, window, _) = provision_as("viewer", json!([request_row("r1", "change", "awaiting_approval", json!([]))]), cx);
    wait_until(cx, "llega la bandeja", |cx| on_screen(window, "request-r1", cx));
    assert!(on_screen(window, "request-admin-decides-r1", cx));
    assert!(!on_screen(window, "request-approve-r1", cx));
    assert!(!on_screen(window, "provision-run", cx));
    assert!(on_screen(window, "provision-run-x1", cx), "el historial se lee");
    reveal(window, "server-body", "provision-config", cx);
    assert!(!on_screen(window, "provision-config-save", cx));
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// Aprobar una destructiva: el botón no se enciende hasta escribir el
/// nombre exacto y reconocer la bandera roja, y aun así pregunta.
///
/// Roturas que atrapa: aprobar con otro nombre o sin reconocer la bandera;
/// aprobar sin preguntar; mandar otra cosa que lo escrito.
#[gpui_kit::test]
fn an_admin_approves_a_destructive_only_with_the_exact_name(cx: &mut TestAppContext) {
    let (server, window, _) = provision_as("admin", json!([request_row("r1", "destructive", "awaiting_approval", json!([{ "code": "destructive", "level": "red" }]))]), cx);
    let mut approved = request_row("r1", "destructive", "approved", json!([{ "code": "destructive", "level": "red" }]));
    approved["decidedByName"] = json!("Ana");
    server.mount(
        Mock::given(method("POST"))
            .and(path("/api/v1/servers/s1/provisioning-requests/r1/decision"))
            .and(wiremock::matchers::body_json(json!({ "approve": true, "note": "", "confirmServer": "pixel-vps", "acknowledgeFlags": true })))
            .respond_with(ok(approved)),
    );
    wait_until(cx, "llega la bandeja", |cx| on_screen(window, "request-approve-r1", cx));
    click(window, "request-approve-r1", cx);
    assert!(cx.pending_prompt().is_none(), "se pudo aprobar sin nombre ni bandera");

    click(window, "request-typed-r1", cx);
    type_text(window, "PIXEL-VPS", cx);
    click(window, "request-ack-r1", cx);
    cx.run_until_parked();
    click(window, "request-approve-r1", cx);
    assert!(cx.pending_prompt().is_none(), "se pudo aprobar con otro nombre");

    click(window, "request-typed-r1", cx);
    cx.simulate_keystrokes(window, &["backspace"; 9].join(" "));
    type_text(window, "pixel-vps", cx);
    cx.run_until_parked();
    click(window, "request-approve-r1", cx);
    let (title, _) = cx.pending_prompt().expect("aprobó sin preguntar");
    assert_eq!(title, "¿Aprobar swarm en pixel-vps?");
    cx.simulate_prompt_answer("Aprobar");
    wait_until(cx, "aprueba", |_| server.hits("/api/v1/servers/s1/provisioning-requests/r1/decision") == 1);
    wait_until(cx, "queda autorizada", |cx| on_screen(window, "request-command-r1", cx));
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// «Correr ahora» corre lo autorizado y, si sale bien, su verificación,
/// cada una abierta y cerrada en cac con la autorización.
///
/// Roturas que atrapa: correr sin preguntar o al cancelar; correr en ensayo lo autorizado
/// (o de verdad algo que no lo está); saltarse la verificación; abrir la
/// ejecución sin la petición, que es lo que gasta la autorización.
#[gpui_kit::test]
fn run_now_runs_the_grant_and_then_its_verification(cx: &mut TestAppContext) {
    let (server, window, engine) = provision_as("admin", json!([request_row("r1", "change", "approved", json!([]))]), cx);
    server.mount(Mock::given(method("POST")).and(path("/api/v1/servers/s1/provisioning-runs")).respond_with(ok(json!({ "id": "run1" }))));
    server.mount(Mock::given(method("PATCH")).and(path("/api/v1/servers/s1/provisioning-runs/run1")).respond_with(ok(json!(null))));
    server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/s1/config/effective")).and(wiremock::matchers::query_param("seq", "4")).respond_with(ok(json!({
        "serverId": "s1", "configSeq": 4, "playbooks": ["swarm"], "vars": { "domain": "x.dev" }, "opRefs": {}, "requires": [], "perServer": [], "missing": [],
        "connection": { "name": "pixel-vps", "host": "127.0.0.1", "sshPort": 22, "sshUser": "deploy" }
    }))).with_priority(1));
    wait_until(cx, "llega la bandeja", |cx| on_screen(window, "request-run-r1", cx));
    click(window, "request-run-r1", cx);
    let (title, _) = cx.pending_prompt().expect("corrió sin preguntar");
    assert_eq!(title, "¿Aplicar «Swarm» a pixel-vps?");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert!(engine.runs.lock().unwrap().is_empty(), "corrió al cancelar");
    click(window, "request-run-r1", cx);
    cx.simulate_prompt_answer("Aplicar");
    wait_until(cx, "corre las dos", |_| engine.runs.lock().unwrap().len() == 2);
    assert_eq!(*engine.runs.lock().unwrap(), [("swarm".to_string(), false), ("swarm-check".to_string(), false)]);
    wait_until(cx, "cierra las dos", |_| server.requests().iter().filter(|r| r.method.as_str() == "PATCH").count() == 2);
    let opened: Vec<serde_json::Value> = writes(&server).into_iter().filter(|(verb, route, _)| verb == "POST" && route.ends_with("/provisioning-runs")).filter_map(|(_, _, body)| body).collect();
    assert_eq!(opened.iter().map(|body| (body["playbook"].clone(), body["requestId"].clone())).collect::<Vec<_>>(), [(json!("swarm"), json!("r1")), (json!("swarm-check"), json!("r1"))]);
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// La config: un admin añade un secreto por su referencia y lo guarda como
/// versión nueva; un valor pegado en vez de la referencia no deja guardar;
/// restaurar una versión pregunta.
///
/// Roturas que atrapa: guardar un valor en vez de su referencia; mandar algo
/// distinto de lo escrito; restaurar sin preguntar.
#[gpui_kit::test]
fn an_admin_saves_references_and_restoring_asks(cx: &mut TestAppContext) {
    let (server, window, _) = provision_as("admin", json!([]), cx);
    server.mount(Mock::given(method("PUT")).and(path("/api/v1/servers/s1/config")).respond_with(ok(json!({ "id": "v5", "seq": 5 }))));
    reveal(window, "server-body", "provision-secret-add", cx);
    click(window, "provision-secret-add", cx);
    wait_until(cx, "sale la fila", |cx| on_screen(window, "provision-secret-name-1", cx));
    click(window, "provision-secret-name-1", cx);
    type_text(window, "CF_TOKEN", cx);
    click(window, "provision-secret-ref-1", cx);
    type_text(window, "hunter2", cx);
    wait_until(cx, "dice que no es una referencia", |cx| on_screen(window, "provision-config-problem", cx));
    reveal(window, "server-body", "provision-config-save", cx);
    click(window, "provision-config-save", cx);
    cx.run_until_parked();
    assert!(!writes(&server).iter().any(|(verb, _, _)| verb == "PUT"), "con un valor pegado llegó a guardar");

    reveal(window, "server-body", "provision-secret-remove-1", cx);
    click(window, "provision-secret-remove-1", cx);
    reveal(window, "server-body", "provision-secret-add", cx);
    click(window, "provision-secret-add", cx);
    wait_until(cx, "sale otra fila", |cx| on_screen(window, "provision-secret-name-1", cx) && !on_screen(window, "provision-config-problem", cx));
    click(window, "provision-secret-name-1", cx);
    type_text(window, "CF_TOKEN", cx);
    click(window, "provision-secret-ref-1", cx);
    type_text(window, "op://Infra/cloudflare/token", cx);
    cx.run_until_parked();
    reveal(window, "server-body", "provision-config-save", cx);
    click(window, "provision-config-save", cx);
    wait_until(cx, "guarda", |_| server.requests().iter().any(|r| r.method.as_str() == "PUT"));
    let saved = writes(&server).into_iter().find(|(verb, _, _)| verb == "PUT").and_then(|(_, _, body)| body).unwrap();
    assert_eq!(saved["opRefs"], json!({ "CF_TOKEN": "op://Infra/cloudflare/token", "OPS_PASSWORD": "op://Infra/pixel/password" }));
    assert_eq!(saved["templateId"], "t1");

    reveal(window, "server-body", "provision-version-restore-3", cx);
    click(window, "provision-version-restore-3", cx);
    let (title, _) = cx.pending_prompt().expect("restauró sin preguntar");
    assert_eq!(title, "¿Restaurar la versión 3?");
    cx.simulate_prompt_answer("Cancelar");
    cx.run_until_parked();
    assert_eq!(writes(&server).iter().filter(|(verb, _, _)| verb == "PUT").count(), 1, "restauró al cancelar");
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// Una plantilla nueva sale de las etapas elegidas del manifiesto, con sus
/// valores por defecto y la contraseña de sudo como propia de cada servidor.
///
/// Roturas que atrapa: crear la plantilla sin etapas; perder los
/// defaults o el secreto de cada máquina; no cerrar el diálogo al guardar.
#[gpui_kit::test]
fn a_new_template_comes_from_the_chosen_stages(cx: &mut TestAppContext) {
    let (server, window, _) = provision_as("admin", json!([]), cx);
    server.mount(Mock::given(method("POST")).and(path("/api/v1/organizations/o1/server-templates/")).respond_with(ok(json!({ "id": "t2", "orgId": "o1", "name": "Nueva" }))));
    reveal(window, "server-body", "provision-template-new", cx);
    click(window, "provision-template-new", cx);
    wait_until(cx, "se abre", |cx| on_screen(window, "template-dialog", cx));
    click(window, "template-name", cx);
    type_text(window, "Nueva", cx);
    cx.run_until_parked();
    click(window, "template-save", cx);
    cx.run_until_parked();
    assert!(writes(&server).iter().all(|(_, route, _)| !route.contains("server-templates")), "creó una plantilla sin etapas");

    click(window, "template-stage-swarm", cx);
    cx.run_until_parked();
    click(window, "template-save", cx);
    wait_until(cx, "crea", |_| writes(&server).iter().any(|(_, route, _)| route.contains("server-templates")));
    let body = writes(&server).into_iter().find(|(_, route, _)| route.contains("server-templates")).and_then(|(_, _, body)| body).unwrap();
    assert_eq!((body["name"].clone(), body["playbooks"].clone(), body["vars"].clone(), body["requires"].clone(), body["perServer"].clone()), (json!("Nueva"), json!(["swarm"]), json!({ "traefik_version": "3.1" }), json!(["OPS_PASSWORD"]), json!(["OPS_PASSWORD"])));
    wait_until(cx, "se cierra", |cx| !on_screen(window, "template-dialog", cx));
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}
