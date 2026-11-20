mod agent_install;
mod host;
mod ansible;
mod secrets;
mod op_env;
mod paste_guard;
mod api_client;
mod core_bridge;
mod crypto_tools;
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
use cac_mcp::op::read as op_read;
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

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const GITHUB_API: &str = "https://api.github.com";

// In-memory cache: survives the process lifetime, avoids repeated keychain reads
// from async contexts where some Linux keyring backends fail.
struct TokenCache(Mutex<HashMap<String, String>>);

impl TokenCache {
    fn get(&self, server_id: &str) -> Option<String> {
        self.0.lock().ok()?.get(server_id).cloned()
    }

    fn set(&self, server_id: &str, token: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.insert(server_id.to_string(), token.to_string());
        }
    }

    fn remove(&self, server_id: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(server_id);
        }
    }
}

/// Para la app: si esta copia quedó vieja tras una actualización. Una
/// instancia así funciona para casi todo, pero 1Password ya no le habla
/// (`InvalidClientInfo`): lo que use `op` falla hasta reabrirla.
#[tauri::command]
fn binary_replaced() -> bool {
    cac_mcp::op::running_binary_replaced()
}

/// One place a secret can be filed: an account and one of its vaults.
#[derive(serde::Serialize)]
struct OpVault {
    account: String, // the sign-in URL, which is what `--account` takes
    email: String,
    vault: String,
}

/// Lists every vault the signed-in accounts can write to.
///
/// Exists because the vault used to be a text box defaulting to "Private", and
/// that only exists in *some* accounts — a personal one has it, a business one
/// typically has "Employee" and named vaults instead. Typing the wrong name
/// failed at save time with a message from `op` that didn't say which of your
/// accounts it had even tried.
///
/// Returns an empty list rather than an error when 1Password isn't reachable:
/// the caller hides the feature instead of showing a scary message next to
/// secrets the user still needs to copy.
#[tauri::command]
async fn op_list_vaults() -> Vec<OpVault> {

    let accounts = match crate::host::command("op")
        .args(["account", "list", "--format", "json"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return vec![],
    };
    let accounts: serde_json::Value = match serde_json::from_slice(&accounts) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut out = vec![];
    for a in accounts.as_array().unwrap_or(&vec![]) {
        let url = a
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let email = a
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if url.is_empty() {
            continue;
        }
        let vaults = match crate::host::command("op")
            .args(["vault", "list", "--account", &url, "--format", "json"])
            .output()
        {
            Ok(o) if o.status.success() => o.stdout,
            // One unreachable account shouldn't hide the others.
            _ => continue,
        };
        let vaults: serde_json::Value = match serde_json::from_slice(&vaults) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for v in vaults.as_array().unwrap_or(&vec![]) {
            if let Some(name) = v.get("name").and_then(|x| x.as_str()) {
                out.push(OpVault {
                    account: url.clone(),
                    email: email.clone(),
                    vault: name.to_string(),
                });
            }
        }
    }
    out
}

/// Stores secrets that are shown exactly once as a new 1Password item, and
/// returns an `op://` reference per field.
///
/// The ingest key and the webhook secret are unrecoverable after the dialog
/// closes: cac keeps only a hash of one and never returns the other. Copying
/// them into a scratch file is what people actually do, so this offers the
/// alternative in the same breath — and hands back references a deploy config
/// can use directly instead of the literal values.
///
/// Values travel in a **0600 temp file passed with `--template`**, never in
/// argv: 1Password's own docs warn that command arguments land in shell history
/// and are readable by other processes. (Assignment statements and a template
/// on stdin were both tried; stdin silently discards the values.)
#[tauri::command]
async fn op_item_create(
    title: String,
    account: String,
    vault: String,
    fields: Vec<(String, String)>,
) -> Result<Vec<(String, String)>, String> {
    use std::io::Write as _;

    if fields.is_empty() {
        return Err("Nothing to save".into());
    }
    if account.trim().is_empty() || vault.trim().is_empty() {
        return Err("Pick an account and a vault first".into());
    }

    let op_err = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => {
            "1Password CLI (op) not found. Install from https://developer.1password.com/docs/cli/"
                .to_string()
        }
        _ => format!("Failed to execute op: {e}"),
    };
    let friendly = |stderr: &str, status: std::process::ExitStatus| {
        let s = stderr.trim();
        if s.contains("not currently signed in") || s.contains("session expired") {
            "1Password session expired. Run `op signin` (or open the desktop app) and try again."
                .to_string()
        } else if s.is_empty() {
            format!("op exited with {status}")
        } else {
            s.to_string()
        }
    };

    // Start from the real category template so the item looks native in the app.
    // --account on every call: with more than one account signed in, letting op
    // pick means the item can land somewhere the user never chose.
    let tpl_out = crate::host::command("op")
        .args([
            "item",
            "template",
            "get",
            "API Credential",
            "--account",
            &account,
        ])
        .output()
        .map_err(op_err)?;
    if !tpl_out.status.success() {
        return Err(friendly(
            &String::from_utf8_lossy(&tpl_out.stderr),
            tpl_out.status,
        ));
    }
    let mut tpl: serde_json::Value = serde_json::from_slice(&tpl_out.stdout)
        .map_err(|e| format!("Unexpected op template: {e}"))?;

    let arr = tpl
        .get_mut("fields")
        .and_then(|f| f.as_array_mut())
        .ok_or("op template has no fields")?;
    for (name, value) in &fields {
        arr.push(serde_json::json!({
            "id": name, "type": "CONCEALED", "label": name, "value": value
        }));
    }

    // 0600 before anything is written, and removed on every path out.
    let path = std::env::temp_dir().join(format!("cac-op-{}.json", ephemeral_suffix()));
    let write_template = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(tpl.to_string().as_bytes())
    };
    write_template().map_err(|e| format!("Could not stage the template: {e}"))?;

    // The category lives in the template; passing --category too is an error.
    let out = crate::host::command("op")
        .args([
            "item",
            "create",
            "--account",
            &account,
            "--vault",
            &vault,
            "--title",
            &title,
            "--format",
            "json",
        ])
        .arg("--template")
        .arg(&path)
        .output();
    let _ = std::fs::remove_file(&path);
    let out = out.map_err(op_err)?;
    if !out.status.success() {
        return Err(friendly(&String::from_utf8_lossy(&out.stderr), out.status));
    }

    // Reference by item id, not title: a title with spaces or a duplicate name
    // would make the reference ambiguous.
    let parsed: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("Unexpected op output: {e}"))?;
    let id = parsed
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if id.is_empty() {
        return Err("op created the item but returned no id".into());
    }
    Ok(fields
        .iter()
        .map(|(name, _)| (name.clone(), format!("op://{vault}/{id}/{name}")))
        .collect())
}

// ─── SSH y llavero ────────────────────────────────────────────────────────────
//
// Lo que se hace en la máquina local vive en `ops/` (`cac_ops`), sin Tauri,
// para que lo use también la app nativa. Aquí quedan los comandos, como una
// capa fina, y los nombres que el resto del crate ya usaba.

pub(crate) use cac_ops::ssh::{
    ephemeral_suffix, ssh_run_input, stage_public_key, EphemeralIdentity, SshOutput,
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

// ─── Keychain commands ────────────────────────────────────────────────────────

#[tauri::command]
fn set_github_token(
    server_id: String,
    token: String,
    cache: tauri::State<TokenCache>,
) -> Result<(), String> {
    keychain::set_github_token(&server_id, &token)?;
    cache.set(&server_id, &token);
    Ok(())
}

#[tauri::command]
fn delete_github_token(server_id: String, cache: tauri::State<TokenCache>) -> Result<(), String> {
    cache.remove(&server_id);
    let _ = keychain::delete_op_reference(&server_id);
    keychain::delete_github_token(&server_id)
}

#[tauri::command]
fn load_github_token_from_1password(
    server_id: String,
    op_reference: String,
    cache: tauri::State<TokenCache>,
) -> Result<(), String> {
    let token = op_read(&op_reference)?;
    keychain::set_github_token(&server_id, &token)?;
    let _ = keychain::set_op_reference(&server_id, &op_reference);
    cache.set(&server_id, &token);
    Ok(())
}

#[tauri::command]
fn refresh_github_token_from_1password(
    server_id: String,
    cache: tauri::State<TokenCache>,
) -> Result<(), String> {
    let reference = keychain::op_reference(&server_id).map_err(|_| {
        "No 1Password reference saved for this server. Use 'Load from 1Password' first.".to_string()
    })?;
    let token = op_read(&reference)?;
    keychain::set_github_token(&server_id, &token)?;
    cache.set(&server_id, &token);
    Ok(())
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
    if cache.get(&server_id).is_some() {
        return true;
    }
    // Try loading from keychain and warming the cache
    if let Ok(token) = keychain::github_token(&server_id) {
        cache.set(&server_id, &token);
        return true;
    }
    false
}

// Retrieve token: cache-first, then keychain fallback
fn get_token(server_id: &str, cache: &TokenCache) -> Result<String, String> {
    if let Some(t) = cache.get(server_id) {
        return Ok(t);
    }
    let t = keychain::github_token(server_id)?;
    cache.set(server_id, &t);
    Ok(t)
}

// ─── Domain types ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct GitHubSecret {
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize)]
pub struct GitHubVariable {
    pub name: String,
    pub value: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
struct SecretsPage {
    secrets: Vec<GitHubSecret>,
}

#[derive(Deserialize)]
struct VariablesPage {
    variables: Vec<GitHubVariable>,
}

// ─── HTTP helpers ─────────────────────────────────────────────────────────────

fn gh_client(token: &str) -> Result<reqwest::Client, String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION};

    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| e.to_string())?,
    );
    headers.insert(
        HeaderName::from_static("x-github-api-version"),
        HeaderValue::from_static("2022-11-28"),
    );
    headers.insert(
        HeaderName::from_static("user-agent"),
        HeaderValue::from_static("cac_desktop_application"),
    );

    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| e.to_string())
}

async fn gh_get<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API {status}: {body}"));
    }
    resp.json::<T>().await.map_err(|e| e.to_string())
}

async fn gh_send(req: reqwest::RequestBuilder, expected: &[u16]) -> Result<(), String> {
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    if !expected.contains(&status) {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API {status}: {body}"));
    }
    Ok(())
}

// ─── NaCl sealed box (crypto_box_seal, libsodium-compatible) ─────────────────

fn seal_box(recipient_pub_b64: &str, plaintext: &[u8]) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use blake2::{
        digest::{Update, VariableOutput},
        Blake2bVar,
    };
    use crypto_box::{aead::Aead, PublicKey, SalsaBox, SecretKey};
    use rand::rngs::OsRng;

    let pub_bytes = STANDARD
        .decode(recipient_pub_b64)
        .map_err(|e| e.to_string())?;
    let pub_array: [u8; 32] = pub_bytes
        .try_into()
        .map_err(|_| "public key must be 32 bytes".to_string())?;
    let recipient_pub = PublicKey::from(pub_array);

    let eph_priv = SecretKey::generate(&mut OsRng);
    let eph_pub = eph_priv.public_key();

    // nonce = BLAKE2b-192(eph_pub || recipient_pub) — matches libsodium crypto_box_seal
    let mut hasher = Blake2bVar::new(24).map_err(|e| e.to_string())?;
    hasher.update(eph_pub.as_bytes());
    hasher.update(recipient_pub.as_bytes());
    let mut nonce_bytes = [0u8; 24];
    hasher
        .finalize_variable(&mut nonce_bytes)
        .map_err(|e| e.to_string())?;
    let nonce = crypto_box::Nonce::from_slice(&nonce_bytes);

    let nacl_box = SalsaBox::new(&recipient_pub, &eph_priv);
    let ciphertext = nacl_box
        .encrypt(nonce, plaintext)
        .map_err(|e| e.to_string())?;

    let mut out = Vec::with_capacity(32 + ciphertext.len());
    out.extend_from_slice(eph_pub.as_bytes());
    out.extend_from_slice(&ciphertext);
    Ok(STANDARD.encode(out))
}

// ─── GitHub API commands ──────────────────────────────────────────────────────

#[tauri::command]
async fn list_github_secrets(
    server_id: String,
    owner: String,
    repo: String,
    cache: tauri::State<'_, TokenCache>,
) -> Result<Vec<GitHubSecret>, String> {
    let client = gh_client(&get_token(&server_id, &cache)?)?;
    let page: SecretsPage = gh_get(
        &client,
        &format!("{GITHUB_API}/repos/{owner}/{repo}/actions/secrets"),
    )
    .await?;
    Ok(page.secrets)
}

#[tauri::command]
async fn list_github_variables(
    server_id: String,
    owner: String,
    repo: String,
    cache: tauri::State<'_, TokenCache>,
) -> Result<Vec<GitHubVariable>, String> {
    let client = gh_client(&get_token(&server_id, &cache)?)?;
    let page: VariablesPage = gh_get(
        &client,
        &format!("{GITHUB_API}/repos/{owner}/{repo}/actions/variables"),
    )
    .await?;
    Ok(page.variables)
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
    let client = gh_client(&get_token(&server_id, &cache)?)?;

    #[derive(Deserialize)]
    struct PubKey {
        key_id: String,
        key: String,
    }
    let pk: PubKey = gh_get(
        &client,
        &format!("{GITHUB_API}/repos/{owner}/{repo}/actions/secrets/public-key"),
    )
    .await?;

    let encrypted = seal_box(&pk.key, value.as_bytes())?;
    let body = serde_json::json!({ "encrypted_value": encrypted, "key_id": pk.key_id });
    gh_send(
        client
            .put(format!(
                "{GITHUB_API}/repos/{owner}/{repo}/actions/secrets/{name}"
            ))
            .json(&body),
        &[201, 204],
    )
    .await
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
    let client = gh_client(&get_token(&server_id, &cache)?)?;
    let body = serde_json::json!({ "name": name, "value": value });
    let req = if exists {
        client
            .patch(format!(
                "{GITHUB_API}/repos/{owner}/{repo}/actions/variables/{name}"
            ))
            .json(&body)
    } else {
        client
            .post(format!(
                "{GITHUB_API}/repos/{owner}/{repo}/actions/variables"
            ))
            .json(&body)
    };
    gh_send(req, &[200, 201, 204]).await
}

#[tauri::command]
async fn delete_github_secret(
    server_id: String,
    owner: String,
    repo: String,
    name: String,
    cache: tauri::State<'_, TokenCache>,
) -> Result<(), String> {
    let client = gh_client(&get_token(&server_id, &cache)?)?;
    gh_send(
        client.delete(format!(
            "{GITHUB_API}/repos/{owner}/{repo}/actions/secrets/{name}"
        )),
        &[204],
    )
    .await
}

#[tauri::command]
async fn delete_github_variable(
    server_id: String,
    owner: String,
    repo: String,
    name: String,
    cache: tauri::State<'_, TokenCache>,
) -> Result<(), String> {
    let client = gh_client(&get_token(&server_id, &cache)?)?;
    gh_send(
        client.delete(format!(
            "{GITHUB_API}/repos/{owner}/{repo}/actions/variables/{name}"
        )),
        &[204],
    )
    .await
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

// ─── Crypto tools ────────────────────────────────────────────────────────────

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
    match kind.as_str() {
        "uuid-v4" => Ok(crypto_tools::generate_uuid_v4()),
        "uuid-v7" => Ok(crypto_tools::generate_uuid_v7()),
        "cuid2" => Ok(crypto_tools::generate_cuid2()),
        _ => Err(format!("Unknown id kind: {kind}")),
    }
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
    match (codec.as_str(), direction.as_str()) {
        ("base64", "encode") => Ok(crypto_tools::base64_encode(&input)),
        ("base64", "decode") => crypto_tools::base64_decode(&input),
        ("url", "encode") => Ok(crypto_tools::url_encode(&input)),
        ("url", "decode") => crypto_tools::url_decode(&input),
        ("hex", "encode") => Ok(crypto_tools::hex_encode(&input)),
        ("hex", "decode") => crypto_tools::hex_decode(&input),
        _ => Err(format!("Unknown codec/direction: {codec}/{direction}")),
    }
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
        .manage(TokenCache(Mutex::new(HashMap::new())))
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
