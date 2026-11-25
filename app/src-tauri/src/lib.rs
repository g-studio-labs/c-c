mod agent_install;
mod host;
mod ansible;
mod secrets;
mod op_env;
mod paste_guard;
mod api_client;
mod core_bridge;
mod download;
mod http_client;
mod notify;
mod media;
mod notes_export;
mod pty;
pub mod icu_tz;
mod relaunch;
mod sse;
mod video_frames;
mod voice;
mod webview_crash;

/// Entry point for `cac --mcp`. El servidor MCP vive en su propio crate
/// (`mcp/`, el mismo que compila el binario `cac-mcp`); aquí sólo se le dice
/// quién lo arranca. La versión que anuncia es la de la app, como siempre: la
/// release la escribe en este `Cargo.toml` (`app-release.yml`).
pub fn serve_mcp() {
    cac_mcp::serve(cac_mcp::Host::desktop_app(env!("CARGO_PKG_VERSION")));
}

/// Entry point for `cac provision …`: aplica un playbook de `infra/ansible`
/// con la config de cac de un servidor, desde una terminal (ver
/// `mcp/src/provision_cli.rs`). Devuelve el código de salida.
pub fn run_provision_cli(args: Vec<String>) -> i32 {
    cac_mcp::provision_cli::main(args)
}

// Lo que la app comparte con el MCP vive en su crate (`mcp/`), que no depende
// de Tauri. Aquí entra con los nombres que ya tenía.
use cac_mcp::files::{free_name, safe_file_name};
use cac_mcp::ssh_agent::resolve_agent_socket;
use cac_mcp::{dropped, image};

/// Absolute path of the running executable, so the "Connect Claude Code" modal
/// can show a command that's correct on this machine.
#[tauri::command]
fn executable_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

use std::sync::Mutex;

use cac_ops::github::{self, GitHubSecret, GitHubVariable, TokenCache};

/// Para la app: si esta copia quedó vieja tras una actualización. Una
/// instancia así funciona para casi todo, pero 1Password ya no le habla
/// (`InvalidClientInfo`): lo que use `op` falla hasta reabrirla.
#[tauri::command]
fn binary_replaced() -> bool {
    cac_mcp::op::running_binary_replaced()
}

/// Ver `cac_ops::op::list_vaults`.
#[tauri::command]
async fn op_list_vaults() -> Vec<cac_ops::op::OpVault> {
    cac_ops::op::list_vaults()
}

/// Ver `cac_ops::op::item_create`: los valores van en un fichero 0600, nunca
/// en el argv.
#[tauri::command]
async fn op_item_create(
    title: String,
    account: String,
    vault: String,
    fields: Vec<(String, String)>,
) -> Result<Vec<(String, String)>, String> {
    cac_ops::op::item_create(&title, &account, &vault, &fields)
}

// ─── SSH y llavero ────────────────────────────────────────────────────────────
//
// Lo que se hace en la máquina local vive en `ops/` (`cac_ops`), sin Tauri,
// para que lo use también la app nativa. Aquí quedan los comandos, como una
// capa fina, y los nombres que el resto del crate ya usaba.

pub(crate) use cac_ops::ssh::{
    ephemeral_suffix, stage_public_key, EphemeralIdentity,
};
use cac_ops::keychain;
pub(crate) use cac_ops::keychain::ssh_key as stored_ssh_key;

/// Agents the machine actually has (ver `cac_ops::ssh::list_ssh_agents`).
#[tauri::command]
fn list_ssh_agents() -> Vec<cac_ops::ssh::SshAgent> {
    cac_ops::ssh::list_ssh_agents()
}

/// Las llaves que tiene el agente (ver `cac_ops::ssh::list_agent_ssh_keys`).
#[tauri::command]
fn list_agent_ssh_keys(socket: Option<String>) -> Result<Vec<cac_ops::ssh::SshKeyItem>, String> {
    cac_ops::ssh::list_agent_ssh_keys(socket)
}

#[tauri::command]
fn set_server_ssh_key(server_id: String, public_key: String) -> Result<(), String> {
    keychain::set_ssh_key(&server_id, &public_key)
}

#[tauri::command]
fn get_server_ssh_key(server_id: String) -> Result<Option<String>, String> {
    keychain::ssh_key(&server_id)
}

// ─── El token de GitHub y los secrets de Actions (`cac_ops::github`) ─────────

#[tauri::command]
fn set_github_token(server_id: String, token: String, cache: tauri::State<TokenCache>) -> Result<(), String> {
    github::set_token(&cache, &server_id, &token)
}

#[tauri::command]
fn delete_github_token(server_id: String, cache: tauri::State<TokenCache>) -> Result<(), String> {
    github::delete_token(&cache, &server_id)
}

#[tauri::command]
fn load_github_token_from_1password(
    server_id: String,
    op_reference: String,
    cache: tauri::State<TokenCache>,
) -> Result<(), String> {
    github::load_token_from_1password(&cache, &server_id, &op_reference)
}

#[tauri::command]
fn refresh_github_token_from_1password(server_id: String, cache: tauri::State<TokenCache>) -> Result<(), String> {
    github::refresh_token_from_1password(&cache, &server_id)
}

#[tauri::command]
fn get_op_reference(server_id: String) -> Option<String> {
    keychain::op_reference(&server_id).ok()
}

#[tauri::command]
fn clear_op_reference(server_id: String) -> Result<(), String> {
    keychain::delete_op_reference(&server_id)
}

#[tauri::command]
fn github_token_configured(server_id: String, cache: tauri::State<TokenCache>) -> bool {
    github::token_configured(&cache, &server_id)
}

#[tauri::command]
async fn list_github_secrets(
    server_id: String,
    owner: String,
    repo: String,
    cache: tauri::State<'_, TokenCache>,
) -> Result<Vec<GitHubSecret>, String> {
    github::list_secrets(&cache, &server_id, &owner, &repo).await
}

#[tauri::command]
async fn list_github_variables(
    server_id: String,
    owner: String,
    repo: String,
    cache: tauri::State<'_, TokenCache>,
) -> Result<Vec<GitHubVariable>, String> {
    github::list_variables(&cache, &server_id, &owner, &repo).await
}

#[tauri::command]
async fn set_github_secret(
    server_id: String,
    owner: String,
    repo: String,
    name: String,
    value: String,
    cache: tauri::State<'_, TokenCache>,
) -> Result<(), String> {
    github::set_secret(&cache, &server_id, &owner, &repo, &name, &value).await
}

#[tauri::command]
async fn set_github_variable(
    server_id: String,
    owner: String,
    repo: String,
    name: String,
    value: String,
    exists: bool,
    cache: tauri::State<'_, TokenCache>,
) -> Result<(), String> {
    github::set_variable(&cache, &server_id, &owner, &repo, &name, &value, exists).await
}

#[tauri::command]
async fn delete_github_secret(
    server_id: String,
    owner: String,
    repo: String,
    name: String,
    cache: tauri::State<'_, TokenCache>,
) -> Result<(), String> {
    github::delete_secret(&cache, &server_id, &owner, &repo, &name).await
}

#[tauri::command]
async fn delete_github_variable(
    server_id: String,
    owner: String,
    repo: String,
    name: String,
    cache: tauri::State<'_, TokenCache>,
) -> Result<(), String> {
    github::delete_variable(&cache, &server_id, &owner, &repo, &name).await
}

// ─── Image compression ───────────────────────────────────────────────────────

#[tauri::command]
async fn compress_image(
    data: Vec<u8>,
    quality: Option<u8>,
    max_width: Option<u32>,
    format: String,
) -> Result<image::CompressResult, String> {
    let fmt: image::OutputFormat = serde_json::from_value(serde_json::Value::String(format))
        .map_err(|e| format!("Invalid format: {e}"))?;

    let opts = image::CompressOptions {
        quality,
        max_width,
        format: fmt,
    };

    tokio::task::spawn_blocking(move || image::compress(&data, &opts))
        .await
        .map_err(|e| e.to_string())?
}

// ─── Crypto tools (`cac_ops::crypto`) ────────────────────────────────────────

use cac_ops::crypto as crypto_tools;

#[tauri::command]
fn jwt_decode(
    token: String,
    secret: Option<String>,
    algorithm: Option<String>,
) -> crypto_tools::JwtDecoded {
    crypto_tools::jwt_decode(&token, secret.as_deref(), algorithm.as_deref())
}

#[tauri::command]
fn generate_id(kind: String) -> Result<String, String> {
    crypto_tools::generate_id(&kind)
}

#[tauri::command]
fn hash_text(input: String, algorithm: String) -> Result<crypto_tools::HashResult, String> {
    crypto_tools::hash_text(&input, &algorithm)
}

#[tauri::command]
fn hmac_sign(
    input: String,
    key: String,
    algorithm: String,
) -> Result<crypto_tools::HashResult, String> {
    crypto_tools::hmac_sign(&input, &key, &algorithm)
}

#[tauri::command]
fn bcrypt_hash(input: String, cost: Option<u32>) -> Result<String, String> {
    crypto_tools::bcrypt_hash(&input, cost.unwrap_or(12))
}

#[tauri::command]
fn bcrypt_verify(input: String, hash: String) -> Result<bool, String> {
    crypto_tools::bcrypt_verify(&input, &hash)
}

#[tauri::command]
fn argon2_hash(input: String) -> Result<String, String> {
    crypto_tools::argon2_hash(&input)
}

#[tauri::command]
fn argon2_verify(input: String, hash: String) -> Result<bool, String> {
    crypto_tools::argon2_verify(&input, &hash)
}

#[tauri::command]
fn encode_decode(input: String, codec: String, direction: String) -> Result<String, String> {
    crypto_tools::encode_decode(&input, &codec, &direction)
}

// ─── HTTP ────────────────────────────────────────────────────────────────────

/// The app's own backend calls. Routed through Rust so they don't ride the
/// webview's connection pool — see api_client for why that matters.
/// Los bytes de un fichero soltado o pegado desde un gestor de ficheros, que
/// en Linux llega sólo como `file:///…`. Ver `dropped.rs` para lo que sirve y
/// lo que no.
#[tauri::command]
fn read_dropped_file(uri: String) -> Result<tauri::ipc::Response, String> {
    dropped::read(&uri).map(tauri::ipc::Response::new)
}

#[tauri::command]
async fn api_request(req: api_client::ApiRequest) -> Result<api_client::ApiResponse, String> {
    api_client::execute(req).await
}

/// Downloads an attachment to a temp file and hands it to the OS.
///
/// A `cacmedia://` URL is only meaningful inside this webview, so links to
/// non-image files can't just be opened in the browser — and doing it this way
/// keeps the token out of any URL, which is the point of the exercise.
#[tauri::command]
async fn open_attachment(
    state: tauri::State<'_, core_bridge::CoreSlot>,
    path: String,
    file_name: String,
) -> Result<(), String> {

    let Some((token, base)) = core_bridge::credentials(&state) else {
        return Err("Not signed in".into());
    };
    let path = media::backend_path(&format!("cacmedia://localhost{path}"))
        .ok_or_else(|| "Not an attachment path".to_string())?;

    let res = reqwest::Client::new()
        .get(format!("{}{}", base.trim_end_matches('/'), path))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("Could not download: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Server returned {}", res.status().as_u16()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;

    let safe: String = file_name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .collect();
    let mut target = std::env::temp_dir();
    target.push(format!(
        "cac-{}-{}",
        ephemeral_suffix(),
        if safe.is_empty() { "file".into() } else { safe }
    ));
    std::fs::write(&target, &bytes).map_err(|e| format!("Could not write the file: {e}"))?;

    // Con el entorno limpio de la imagen: el del plugin lanza `xdg-open` con
    // el de la app y, con un bash del sistema más nuevo, muere (ver host.rs).
    crate::host::open_path(&target).map_err(|e| format!("Could not open it: {e}"))
}

/// Guarda un adjunto (hoy, una grabación) en la carpeta de Descargas y
/// devuelve dónde quedó.
///
/// Existe porque en el escritorio una grabación **no se puede escuchar dentro
/// de la app** hasta la AppImage con GStreamer propio (10-oct-2026: el
/// `<audio>` tumbaba la página), y aunque se pudiera, bajarse el fichero es
/// lo que se pide para pasarlo, medirlo o escucharlo fuera. Por trozos y no
/// entero en memoria: una reunión son cientos de MB.
#[tauri::command]
async fn save_attachment(
    app: tauri::AppHandle,
    state: tauri::State<'_, core_bridge::CoreSlot>,
    path: String,
    file_name: String,
) -> Result<String, String> {
    use std::io::Write;
    use tauri::Manager;

    let Some((token, base)) = core_bridge::credentials(&state) else {
        return Err("Not signed in".into());
    };
    let path = media::backend_path(&format!("cacmedia://localhost{path}"))
        .ok_or_else(|| "Not an attachment path".to_string())?;
    let dir = app
        .path()
        .download_dir()
        .map_err(|e| format!("No downloads folder: {e}"))?;
    let target = free_name(&dir, &safe_file_name(&file_name));

    let mut res = reqwest::Client::new()
        .get(format!("{}{}", base.trim_end_matches('/'), path))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("Could not download: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Server returned {}", res.status().as_u16()));
    }
    // Se escribe a un `.part` y se renombra al final: un corte a mitad no deja
    // en Descargas algo que parece la grabación y no lo es.
    let parcial = target.with_extension(format!(
        "{}.part",
        target.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    let escribir = async {
        let mut f = std::fs::File::create(&parcial).map_err(|e| e.to_string())?;
        while let Some(trozo) = res.chunk().await.map_err(|e| e.to_string())? {
            f.write_all(&trozo).map_err(|e| e.to_string())?;
        }
        f.flush().map_err(|e| e.to_string())?;
        std::fs::rename(&parcial, &target).map_err(|e| e.to_string())
    };
    if let Err(e) = escribir.await {
        let _ = std::fs::remove_file(&parcial);
        return Err(format!("Could not save the file: {e}"));
    }
    Ok(target.to_string_lossy().to_string())
}

/// Opens the live event stream. Idempotent: connecting again replaces the
/// previous stream, which is what a token refresh needs.
#[tauri::command]
fn sse_connect(app: tauri::AppHandle, url: String, token: String) {
    sse::spawn(app, url, token);
}

#[tauri::command]
fn sse_disconnect() {
    sse::stop();
}

// ─── HTTP client (Request Client tool) ───────────────────────────────────────

#[tauri::command]
async fn send_http_request(
    method: String,
    url: String,
    headers: Vec<http_client::KeyValue>,
    body: Option<String>,
) -> Result<http_client::HttpResponse, String> {
    let m: http_client::HttpMethod = serde_json::from_value(serde_json::Value::String(method))
        .map_err(|e| format!("Invalid method: {e}"))?;

    let req = http_client::HttpRequest {
        method: m,
        url,
        headers,
        body,
    };

    http_client::execute(req).await
}

// ─── File save ───────────────────────────────────────────────────────────────

#[tauri::command]
async fn save_file(
    app: tauri::AppHandle,
    data: Vec<u8>,
    file_name: String,
    filter_name: String,
    filter_ext: String,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;

    let mut dialog = app
        .dialog()
        .file()
        .set_file_name(&file_name)
        .add_filter(&filter_name, &[&filter_ext]);
    if let Some(dir) = dialog_dir(&app) {
        dialog = dialog.set_directory(dir);
    }
    let path = dialog.blocking_save_file();

    let Some(path) = path else {
        return Ok(false);
    };

    let path = path.as_path().ok_or("Invalid path")?;
    std::fs::write(path, &data).map_err(|e| e.to_string())?;
    remember_dialog_dir(path.parent());

    Ok(true)
}

// ─── Dónde abre un diálogo de ficheros ───────────────────────────────────────
//
// Sin carpeta, GTK abre el diálogo en el directorio del proceso, y en la
// AppImage ése es su montaje: el AppRun de linuxdeploy hace `cd $APPDIR/usr`
// antes de lanzar la app, así que «guardar» abría en `/tmp/.mount_cac…/usr`
// (14-oct-2026), una carpeta que desaparece al cerrar la app. Se abre en la
// última donde se guardó o se eligió algo y, la primera vez, en Descargas.

static LAST_DIALOG_DIR: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

fn remember_dialog_dir(dir: Option<&std::path::Path>) {
    if let (Some(d), Ok(mut last)) = (dir, LAST_DIALOG_DIR.lock()) {
        *last = Some(d.to_path_buf());
    }
}

fn dialog_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    let last = LAST_DIALOG_DIR.lock().ok().and_then(|l| l.clone());
    pick_dialog_dir(&[last, app.path().download_dir().ok(), app.path().home_dir().ok()])
}

/// La primera que existe como carpeta y no está dentro del montaje de la
/// AppImage (ni en ningún `/tmp/.mount_*`).
fn pick_dialog_dir(candidates: &[Option<std::path::PathBuf>]) -> Option<std::path::PathBuf> {
    candidates
        .iter()
        .flatten()
        .find(|d| d.is_dir() && !d.to_string_lossy().contains("/.mount_"))
        .cloned()
}

// ─── Notes export ────────────────────────────────────────────────────────────

/// Asks for a folder and writes every note into it as markdown.
///
/// Returns `None` when the user dismissed the picker, so the UI can stay quiet
/// instead of reporting a cancellation as a failure.
#[tauri::command]
async fn export_notes(
    app: tauri::AppHandle,
    state: tauri::State<'_, core_bridge::CoreSlot>,
    subfolder: String,
) -> Result<Option<notes_export::ExportSummary>, String> {
    use tauri_plugin_dialog::DialogExt;

    // Copied out before the await: the guard is not Send, and holding it across
    // one would make this command fail to compile.
    let Some((token, base)) = core_bridge::credentials(&state) else {
        return Err("Not signed in".into());
    };

    let mut picker = app.dialog().file();
    if let Some(dir) = dialog_dir(&app) {
        picker = picker.set_directory(dir);
    }
    let Some(chosen) = picker.blocking_pick_folder() else {
        return Ok(None);
    };
    let chosen = chosen.into_path().map_err(|e| e.to_string())?;
    remember_dialog_dir(Some(&chosen));

    // Everything lands in one new subfolder rather than loose in whatever the
    // user picked: an export writes a whole tree, and scattering it across an
    // existing Documents folder would be hard to undo.
    let dest = chosen.join(sanitize_component(&subfolder, "cac-notes"));
    std::fs::create_dir_all(&dest).map_err(|e| format!("Could not create {dest:?}: {e}"))?;

    notes_export::run(&base, &token, dest).await.map(Some)
}

/// One path component, with anything that could escape it removed.
fn sanitize_component(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

// ─── App entry ────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Antes que nada: si no, los primeros registros del SDK se pierden.
    voice::listen_to_sdk();
    tauri::Builder::default()
        // Las sesiones de terminal se matan al cerrar la ventana. En Unix el
        // `ssh` moriría igual al soltarse el pty, pero apoyarse en eso es
        // apostar a en qué orden se destruyen los descriptores al salir.
        .on_window_event(|_, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                pty::close_all();
                // Un playbook a medias contra un servidor, sin nadie mirando.
                ansible::close_all();
                // Igual que el pty: una sala abierta y un micrófono vivo en un
                // proceso que ya nadie mira es peor que un recurso filtrado.
                voice::close_all();
            }
        })
        .manage(TokenCache::default())
        // La sesión ya no se copia desde la pantalla (`set_session`): es del
        // core, y quien necesita el token se lo pide a él.
        .manage(core_bridge::CoreSlot::default())
        // Attachments are served under our own scheme so an <img> can carry
        // credentials in a header instead of the URL. See media.rs.
        .register_asynchronous_uri_scheme_protocol(video_frames::SCHEME, |_ctx, req, responder| {
            // Fuera del hilo de la interfaz, y esto **no** es una
            // optimización.
            //
            // La primera versión era síncrona, con el argumento de que aquí
            // no hay red de por medio y sólo hay que comprimir algo que ya
            // está en memoria. Pero comprimir cuesta once milisegundos y el
            // manejador síncrono corre en el hilo que atiende al webview:
            // con la pantalla pidiendo tramas seguidas, la ventana deja de
            // responder y acaba muriendo. Pasó en la v1.6.38.
            //
            // Y en el **pool** de hilos bloqueantes, no en un hilo nuevo por
            // petición. Con treinta peticiones por segundo y por mosaico,
            // crear y destruir un hilo cada vez cuesta más que el trabajo
            // que hace; además el búfer que `comprimir` reutiliza es
            // `thread_local`, y con un hilo distinto cada vez no se
            // reutilizaría nunca.
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(video_frames::servir(&req))
            });
        })
        .register_asynchronous_uri_scheme_protocol(media::SCHEME, |ctx, req, responder| {
            use tauri::Manager;
            let (token, base) =
                core_bridge::credentials(&ctx.app_handle().state::<core_bridge::CoreSlot>()).unzip();
            tauri::async_runtime::spawn(async move {
                responder.respond(media::serve(token, base, req).await);
            });
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        // Reads the clipboard from the OS instead of the webview. WebKitGTK
        // hands a pasted bitmap to the page as an <img> with no src — see
        // readClipboardImage() in the frontend for the whole story.
        .plugin(tauri_plugin_clipboard_manager::init())
        // WebKitGTK trae WebRTC apagado de fábrica: sin esto, en Linux
        // `getUserMedia` ni siquiera existe en `navigator.mediaDevices` y los
        // canales de voz mueren antes de pedir el micrófono. Windows (WebView2)
        // y macOS (WKWebView) no necesitan nada equivalente.
        .setup(|app| {
            #[cfg(target_os = "linux")]
            {
                use tauri::Manager;
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.with_webview(|webview| {
                        use webkit2gtk::{PermissionRequestExt, SettingsExt, WebViewExt};
                        let inner = webview.inner();
                        if let Some(settings) = inner.settings() {
                            settings.set_enable_webrtc(true);
                            settings.set_enable_media_stream(true);
                        }
                        // WebKitGTK deniega todo permiso que el embebedor no
                        // conteste — sin esto, `getUserMedia` devuelve
                        // NotAllowedError sin preguntarle a nadie. Se concede
                        // micrófono/cámara sin diálogo propio porque este
                        // webview sólo carga cac (los enlaces externos se abren
                        // en el navegador del sistema), así que quien pide es
                        // siempre nuestra propia app; el permiso del sistema
                        // operativo sigue aplicando por encima. La pantalla no
                        // pasa por aquí: la pide el portal del escritorio con
                        // su propio selector.
                        inner.connect_permission_request(|_, req| {
                            use webkit2gtk::glib::object::Cast;
                            use webkit2gtk::{
                                DeviceInfoPermissionRequest, UserMediaPermissionRequest,
                            };
                            if req.downcast_ref::<UserMediaPermissionRequest>().is_some()
                                || req.downcast_ref::<DeviceInfoPermissionRequest>().is_some()
                            {
                                req.allow();
                                return true;
                            }
                            false
                        });
                        // Si el proceso de la página se muere, la ventana se
                        // queda en negro y la página no puede ni enterarse:
                        // se apunta y se recarga desde aquí
                        // (ver `webview_crash.rs`).
                        // Pegar con el portapapeles sin dueño tumba WebKit
                        // 2.54 (ver `paste_guard.rs`).
                        paste_guard::install(&inner);
                        inner.connect_web_process_terminated(|wv, reason| {
                            let url = wv.uri().map(|u| u.to_string()).unwrap_or_default();
                            if webview_crash::on_terminated(
                                webview_crash::reason_name(reason),
                                &url,
                            ) {
                                wv.reload();
                            }
                        });
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            relaunch::relaunch_app,
            host::open_url,
            host::open_local_path,
            agent_install::install_swarm_manage_agent,
            set_github_token,
            delete_github_token,
            github_token_configured,
            load_github_token_from_1password,
            refresh_github_token_from_1password,
            get_op_reference,
            clear_op_reference,
            list_github_secrets,
            binary_replaced,
            list_github_variables,
            set_github_secret,
            set_github_variable,
            delete_github_secret,
            delete_github_variable,
            compress_image,
            executable_path,
            get_server_ssh_key,
            set_server_ssh_key,
            list_agent_ssh_keys,
            list_ssh_agents,
            jwt_decode,
            generate_id,
            hash_text,
            hmac_sign,
            bcrypt_hash,
            bcrypt_verify,
            argon2_hash,
            argon2_verify,
            encode_decode,
            send_http_request,
            api_request,
            core_bridge::core_init,
            core_bridge::core_session,
            core_bridge::core_request,
            core_bridge::core_sign_in,
            core_bridge::core_adopt_session,
            core_bridge::core_update_session,
            core_bridge::core_refresh,
            core_bridge::core_logout,
            core_bridge::mywork_state,
            core_bridge::mywork_set_lens,
            core_bridge::mywork_set_include_closed,
            core_bridge::mywork_set_scope,
            core_bridge::mywork_load,
            core_bridge::mywork_ensure_loaded,
            core_bridge::mywork_set_watching,
            core_bridge::mywork_forget,
            core_bridge::mywork_set_due_local,
            core_bridge::mywork_task_event,
            open_attachment,
            save_attachment,
            read_dropped_file,
            sse_connect,
            sse_disconnect,
            save_file,
            download::download_to_file,
            notify::desktop_notify,
            export_notes,
            op_item_create,
            op_list_vaults,
            secrets::create_backup_key,
            secrets::rotate_service_secrets,
            secrets::capture_service_env,
            secrets::check_op_refs,
            op_env::env_file_names,
            secrets::op_item_fields,
            ansible::ansible_tools,
            ansible::ansible_manifest,
            ansible::ansible_inventory,
            ansible::ansible_run,
            ansible::ansible_run_with_config,
            ansible::ansible_check_report,
            ansible::ansible_plan_gaps,
            ansible::ansible_cancel,
            pty::pty_open,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_close,
            pty::pty_close_all,
            voice::voice_join,
            voice::voice_attach,
            voice::voice_leave,
            voice::voice_set_mic,
            voice::voice_set_noise_suppression,
            webview_crash::take_webview_crash,
            voice::voice_set_deaf,
            voice::voice_set_camera,
            voice::voice_share_screen,
            voice::voice_screen_sources,
            voice::voice_stop_share,
            voice::voice_diagnostics,
            voice::voice_test_camera,
            voice::voice_report,
            voice::voice_mic_level,
            voice::voice_list_devices,
            voice::voice_set_device,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod dialog_dir_tests {
    use super::pick_dialog_dir;

    /// Nunca en el montaje de la AppImage, y la primera que exista.
    /// Mutantes: no saltar `/.mount_`; no mirar que exista.
    #[test]
    fn a_dialog_never_opens_inside_the_appimage() {
        let tmp = std::env::temp_dir();
        let mount = tmp.join(".mount_cacTEST/usr");
        std::fs::create_dir_all(&mount).unwrap();
        let missing = tmp.join("cac-no-existe-nunca");
        let home = tmp.clone();
        assert_eq!(
            pick_dialog_dir(&[Some(mount.clone()), Some(missing), None, Some(home.clone())]),
            Some(home)
        );
        assert_eq!(pick_dialog_dir(&[Some(mount.clone())]), None);
        let _ = std::fs::remove_dir_all(tmp.join(".mount_cacTEST"));
    }
}
