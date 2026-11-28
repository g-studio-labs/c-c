// Las pruebas del módulo Documentación: se incluyen en las de `workspace.rs`
// para usar su servidor de mentira y sus ayudas.

/// Una organización con el espacio `a` (la lista `l-a` y la carpeta `f-a`
/// con la lista `in-a`). Documentados: la lista `l-a`, con una página, y el
/// espacio, con otra que nadie ha abierto. `in-a` no tiene documento.
fn server_with_doc_index() -> FakeServer {
    server_with_doc_bodies("# Qué es\n\nUn párrafo.", "## Capas\n\nTres.")
}

/// Lo mismo, con `body` como portada de cada documento y `page` como la
/// página «Arquitectura» de la lista `l-a`.
fn server_with_doc_bodies(body: &str, page: &str) -> FakeServer {
    let server = FakeServer::start();
    let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
    server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org("o1", "G Studio")]))));
    server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))));
    server.mount(get("/api/v1/docs/").respond_with(ok(json!({
        "list:l-a": { "written": true, "maintainerId": "u1", "maintainerName": "Ana Ruiz", "reviewedAt": "2026-10-01T12:00:00Z", "pages": 1 },
        "space:a": { "written": true, "pages": 1 },
    }))));
    server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
    let doc = json!({ "doc": null, "orgId": "o1", "tabs": [{ "key": "overview", "body": body }], "decisions": [] });
    server.mount(Mock::given(method("GET")).and(wiremock::matchers::path_regex("^/api/v1/docs/(space|folder|list)/[^/]+$")).respond_with(ok(doc)));
    server.mount(get("/api/v1/docs/list/l-a/pages").respond_with(ok(json!([page_item("p1", None, "Arquitectura")]))));
    server.mount(get("/api/v1/docs/list/l-a/pages/p1").respond_with(ok(page_view("p1", "Arquitectura", page, json!([]), json!([]), json!([])))));
    server.mount(get("/api/v1/docs/space/a/pages").respond_with(ok(json!([page_item("q1", None, "Guía")]))));
    server.mount(get("/api/v1/docs/space/a/pages/q1").respond_with(ok(page_view("q1", "Guía", "Cómo se trabaja.", json!([]), json!([]), json!([])))));
    server
}

/// Un ```mermaid de un documento se ve como diagrama: mientras se dibuja, su
/// código atenuado; dibujado, la imagen. Uno roto enseña el código con el
/// motivo, como la web.
///
/// Roturas que atrapa: pintar el documento con `TextView::markdown` sin los
/// diagramas (sale el bloque de código); que el diagrama no se pida o no
/// vuelva a pintarse al llegar; un error sin su motivo.
#[gpui_kit::test]
fn a_mermaid_block_in_a_doc_is_drawn_and_a_broken_one_says_why(cx: &mut TestAppContext) {
    let server = server_with_doc_bodies(
        "# Mapa\n\n```mermaid\nflowchart LR\n  A --> B\n```\n",
        "## Roto\n\n```mermaid\nflowchart LR\n  A --> \n  [[[\n```\n",
    );
    server.install(cx);
    let hold = cx.update(|cx| crate::diagrams::Diagrams::install_held(None, cx));
    let window = open(cx);
    wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
    click(window, "rail-docs", cx);
    wait_until(cx, "sale la tabla", |cx| on_screen(window, "doc-index-row-l-a", cx));
    click(window, "doc-index-row-l-a", cx);
    wait_until(cx, "mientras se dibuja, el código", |cx| on_screen(window, "mermaid-loading", cx));
    assert!(!on_screen(window, "mermaid-diagram", cx), "dibujado antes de dibujarse");

    hold.add_permits(2);
    wait_until(cx, "sale el diagrama", |cx| on_screen(window, "mermaid-diagram", cx));
    assert!(!on_screen(window, "mermaid-loading", cx), "el código atenuado sigue ahí");

    wait_until(cx, "el árbol enseña sus páginas", |cx| on_screen(window, "docs-page-p1", cx));
    click(window, "docs-page-p1", cx);
    wait_until(cx, "sale el error", |cx| on_screen(window, "mermaid-error", cx));
    assert!(!on_screen(window, "mermaid-diagram", cx), "un diagrama roto se dibuja");
    let said = label(window, "mermaid-error", cx);
    let prefix = cx.update(|cx| crate::lang::tr(cx, "common:last.mermaidError"));
    assert!(said.starts_with(&prefix) && said.len() > prefix.len() + 10, "sin motivo: «{said}»");
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

fn in_docs_module(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| Backend::snapshot(cx).read(cx).docs.in_module)
}

/// Documentación en el riel lleva a su panel y a la tabla de todo lo
/// documentado; una fila abre su documento con su camino desplegado en el
/// árbol, y de ahí se va a una página. «Todos los docs» vuelve a la tabla y
/// «Trabajo» vuelve al árbol de siempre, sin el documento.
///
/// Roturas que atrapa: el botón del riel que sigue sin hacer nada; la
/// tabla que no sale sin documento abierto (o sale encima de uno); una
/// fila de la tabla que no abre nada; el árbol que no despliega el camino
/// hasta lo que se lee, o sin su portada y sus páginas; «Todos los docs»
/// que no vuelve a la tabla; «Trabajo» que se queda en Documentación o
/// enseña el documento de allí en vez de «Mi trabajo».
#[gpui_kit::test]
fn the_docs_rail_shows_the_index_and_a_row_opens_its_doc(cx: &mut TestAppContext) {
    let server = server_with_doc_index();
    server.install(cx);
    let window = open(cx);
    wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
    assert!(!on_screen(window, "doc-index", cx));

    click(window, "rail-docs", cx);
    wait_until(cx, "sale la tabla", |cx| on_screen(window, "doc-index-row-l-a", cx));
    assert!(on_screen(window, "docs-all", cx) && on_screen(window, "docs-node-a", cx), "falta el panel de Documentación");
    assert!(!on_screen(window, "tree-l-a", cx), "el panel sigue siendo el árbol de Trabajo");
    assert!(on_screen(window, "doc-index-undocumented-in-a", cx), "una lista sin documento no sale al pie");
    assert!(!on_screen(window, "doc-index-undocumented-l-a", cx), "una lista documentada sale como pendiente");
    assert!(!on_screen(window, "docs-node-l-a", cx), "sin nada abierto, el árbol va desplegado");

    click(window, "doc-index-row-l-a", cx);
    wait_until(cx, "abre su documento", |cx| doc_open(cx).as_deref() == Some("l-a") && on_screen(window, "doc-body", cx));
    assert!(!on_screen(window, "doc-index", cx), "la tabla sigue encima del documento");
    wait_until(cx, "el árbol enseña sus páginas", |cx| on_screen(window, "docs-page-p1", cx));
    assert!(on_screen(window, "docs-home-l-a", cx), "falta la portada bajo el documento abierto");

    click(window, "docs-page-p1", cx);
    wait_until(cx, "se lee la página", |cx| page_open(cx).as_deref() == Some("p1") && on_screen(window, "doc-page-title", cx));

    click(window, "docs-all", cx);
    wait_until(cx, "vuelve la tabla", |cx| on_screen(window, "doc-index", cx));
    assert!(doc_open(cx).is_none(), "la tabla con un documento abierto debajo");

    click(window, "doc-index-row-l-a", cx);
    wait_until(cx, "abre otra vez", |cx| on_screen(window, "doc-body", cx));
    click(window, "rail-work", cx);
    wait_until(cx, "«Trabajo» devuelve el árbol", |cx| on_screen(window, "tree-l-a", cx));
    assert!(!in_docs_module(cx) && doc_open(cx).is_none(), "«Trabajo» enseña lo que se leía en Documentación");
    assert!(on_screen(window, "view-title", cx));
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}

/// Desplegar en el árbol un documento que no es el abierto enseña sus
/// páginas sin abrirlo, y pulsar una abre ese documento en ella. El filtro
/// busca también en ellas, y dice cuando no hay nada.
///
/// Roturas que atrapa: un nodo con páginas que no se despliega o que se
/// despliega vacío (no se piden); desplegar que abre el documento; una
/// página de otro documento que abre su portada o nada; el filtro que no
/// mira las páginas; un árbol en blanco sin decir por qué.
#[gpui_kit::test]
fn unfolding_another_doc_shows_its_pages_without_opening_it(cx: &mut TestAppContext) {
    let server = server_with_doc_index();
    server.install(cx);
    let window = open(cx);
    wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
    click(window, "rail-docs", cx);
    wait_until(cx, "sale el árbol de Documentación", |cx| on_screen(window, "docs-node-a", cx));

    click(window, "docs-node-a-toggle", cx);
    wait_until(cx, "salen sus páginas", |cx| on_screen(window, "docs-page-q1", cx));
    assert!(on_screen(window, "docs-node-l-a", cx), "desplegar no enseña lo que cuelga del espacio");
    assert!(doc_open(cx).is_none(), "desplegar abrió el documento");

    click(window, "docs-page-q1", cx);
    wait_until(cx, "abre el espacio en esa página", |cx| doc_open(cx).as_deref() == Some("a") && page_open(cx).as_deref() == Some("q1"));
    wait_until(cx, "se lee", |cx| on_screen(window, "doc-page-title", cx));

    let typed = |text: &str, cx: &mut TestAppContext| {
        let text = text.to_string();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("docs-filter", cx);
            window.input(&text, cx);
        })
        .unwrap();
    };
    typed("guí", cx);
    assert!(on_screen(window, "docs-page-q1", cx), "el filtro no mira las páginas");
    assert!(!on_screen(window, "docs-node-l-a", cx), "el filtro no quitó lo que no casa");
    typed("zzz", cx);
    assert!(on_screen(window, "docs-no-matches", cx), "un árbol en blanco sin decir por qué");
    cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
}
