//! Los secrets y las variables de Actions de un repo de GitHub, con el token
//! (PAT) de cada servidor guardado en el llavero.
//!
//! Sale de la app de Tauri para que la app nativa use lo mismo. El token no
//! viaja por la interfaz: las funciones de la API reciben el id del servidor y
//! lo sacan de `TokenCache`, que lo lee del llavero. Un secret va sellado con la
//! llave pública del repo (la caja sellada de libsodium) y no vuelve nunca:
//! GitHub sólo devuelve su nombre y fechas.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::keychain;

const GITHUB_API: &str = "https://api.github.com";

// In-memory cache: survives the process lifetime, avoids repeated keychain reads
// from async contexts where some Linux keyring backends fail.
#[derive(Default)]
pub struct TokenCache(Mutex<HashMap<String, String>>);

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

    // Retrieve token: cache-first, then keychain fallback
    fn token(&self, server_id: &str) -> Result<String, String> {
        self.token_with(server_id, keychain::github_token)
    }

    fn token_with(&self, server_id: &str, keychain: impl FnOnce(&str) -> Result<String, String>) -> Result<String, String> {
        if let Some(t) = self.get(server_id) {
            return Ok(t);
        }
        let t = keychain(server_id)?;
        self.set(server_id, &t);
        Ok(t)
    }
}

// ─── El token en el llavero ───────────────────────────────────────────────────

pub fn set_token(cache: &TokenCache, server_id: &str, token: &str) -> Result<(), String> {
    keychain::set_github_token(server_id, token)?;
    cache.set(server_id, token);
    Ok(())
}

pub fn delete_token(cache: &TokenCache, server_id: &str) -> Result<(), String> {
    cache.remove(server_id);
    let _ = keychain::delete_op_reference(server_id);
    keychain::delete_github_token(server_id)
}

/// Lee el token de 1Password, lo guarda en el llavero y apunta de qué
/// referencia salió, para refrescarlo luego. Una referencia mal escrita no
/// llega a `op`.
pub fn load_token_from_1password(cache: &TokenCache, server_id: &str, op_reference: &str) -> Result<(), String> {
    if !op_reference.starts_with("op://") || !crate::secrets::valid_ref(op_reference) {
        return Err(format!("`{op_reference}` is not op://vault/item/field"));
    }
    let token = cac_mcp::op::read(op_reference)?;
    keychain::set_github_token(server_id, &token)?;
    let _ = keychain::set_op_reference(server_id, op_reference);
    cache.set(server_id, &token);
    Ok(())
}

pub fn refresh_token_from_1password(cache: &TokenCache, server_id: &str) -> Result<(), String> {
    let reference = keychain::op_reference(server_id).map_err(|_| {
        "No 1Password reference saved for this server. Use 'Load from 1Password' first.".to_string()
    })?;
    let token = cac_mcp::op::read(&reference)?;
    keychain::set_github_token(server_id, &token)?;
    cache.set(server_id, &token);
    Ok(())
}

pub fn token_configured(cache: &TokenCache, server_id: &str) -> bool {
    cache.token(server_id).is_ok()
}

// ─── Lo que devuelve GitHub ───────────────────────────────────────────────────

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

pub fn seal_box(recipient_pub_b64: &str, plaintext: &[u8]) -> Result<String, String> {
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

/// Lo que se le manda a GitHub para poner un secret: el valor sellado con la
/// llave pública del repo y el id de esa llave. El valor en claro no va.
fn secret_body(key_id: &str, repo_key_b64: &str, value: &str) -> Result<serde_json::Value, String> {
    let encrypted = seal_box(repo_key_b64, value.as_bytes())?;
    Ok(serde_json::json!({ "encrypted_value": encrypted, "key_id": key_id }))
}

// ─── La API de GitHub ─────────────────────────────────────────────────────────

pub async fn list_secrets(cache: &TokenCache, server_id: &str, owner: &str, repo: &str) -> Result<Vec<GitHubSecret>, String> {
    let client = gh_client(&cache.token(server_id)?)?;
    let page: SecretsPage = gh_get(
        &client,
        &format!("{GITHUB_API}/repos/{owner}/{repo}/actions/secrets"),
    )
    .await?;
    Ok(page.secrets)
}

pub async fn list_variables(cache: &TokenCache, server_id: &str, owner: &str, repo: &str) -> Result<Vec<GitHubVariable>, String> {
    let client = gh_client(&cache.token(server_id)?)?;
    let page: VariablesPage = gh_get(
        &client,
        &format!("{GITHUB_API}/repos/{owner}/{repo}/actions/variables"),
    )
    .await?;
    Ok(page.variables)
}

pub async fn set_secret(cache: &TokenCache, server_id: &str, owner: &str, repo: &str, name: &str, value: &str) -> Result<(), String> {
    let client = gh_client(&cache.token(server_id)?)?;

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

    let body = secret_body(&pk.key_id, &pk.key, value)?;
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

pub async fn set_variable(cache: &TokenCache, server_id: &str, owner: &str, repo: &str, name: &str, value: &str, exists: bool) -> Result<(), String> {
    let client = gh_client(&cache.token(server_id)?)?;
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

pub async fn delete_secret(cache: &TokenCache, server_id: &str, owner: &str, repo: &str, name: &str) -> Result<(), String> {
    let client = gh_client(&cache.token(server_id)?)?;
    gh_send(
        client.delete(format!(
            "{GITHUB_API}/repos/{owner}/{repo}/actions/secrets/{name}"
        )),
        &[204],
    )
    .await
}

pub async fn delete_variable(cache: &TokenCache, server_id: &str, owner: &str, repo: &str, name: &str) -> Result<(), String> {
    let client = gh_client(&cache.token(server_id)?)?;
    gh_send(
        client.delete(format!(
            "{GITHUB_API}/repos/{owner}/{repo}/actions/variables/{name}"
        )),
        &[204],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use crypto_box::{aead::Aead, PublicKey, SalsaBox, SecretKey};

    /// Abre una caja sellada como lo hace GitHub (`crypto_box_seal_open`).
    fn open(sealed_b64: &str, repo: &SecretKey) -> Option<Vec<u8>> {
        use blake2::digest::{Update, VariableOutput};
        let raw = STANDARD.decode(sealed_b64).ok()?;
        if raw.len() < 32 {
            return None;
        }
        let (eph, ciphertext) = raw.split_at(32);
        let eph = PublicKey::from(<[u8; 32]>::try_from(eph).ok()?);
        let mut hasher = blake2::Blake2bVar::new(24).ok()?;
        hasher.update(eph.as_bytes());
        hasher.update(repo.public_key().as_bytes());
        let mut nonce = [0u8; 24];
        hasher.finalize_variable(&mut nonce).ok()?;
        SalsaBox::new(&eph, repo).decrypt(crypto_box::Nonce::from_slice(&nonce), ciphertext).ok()
    }

    fn repo_key() -> (SecretKey, String) {
        let key = SecretKey::generate(&mut rand::rngs::OsRng);
        let public = STANDARD.encode(key.public_key().as_bytes());
        (key, public)
    }

    /// La caja sellada se abre con la llave del repo y con ninguna otra.
    /// Roturas que atrapa: el nonce con las llaves al revés; sellar con una
    /// llave que no es la del repo; no poner delante la pública efímera (GitHub
    /// no sabría abrirla).
    #[test]
    fn the_sealed_box_opens_only_with_the_repo_key() {
        let (repo, public) = repo_key();
        let sealed = seal_box(&public, b"ghp_s3cr3t").unwrap();
        assert_eq!(open(&sealed, &repo).as_deref(), Some(&b"ghp_s3cr3t"[..]));
        let (other, _) = repo_key();
        assert!(open(&sealed, &other).is_none(), "otra llave abre la caja");
        assert!(seal_box(&STANDARD.encode([1u8; 31]), b"x").is_err());
    }

    /// Lo que se manda a GitHub lleva el valor sellado y el id de la llave, y
    /// el valor en claro en ningún sitio. Rotura que atrapa: mandar también el
    /// valor (o mandarlo sin sellar).
    #[test]
    fn a_secret_travels_sealed_and_never_in_the_clear() {
        let (repo, public) = repo_key();
        let body = secret_body("kid-1", &public, "s3cr3t-valor").unwrap();
        assert!(!body.to_string().contains("s3cr3t-valor"), "el valor va en claro: {body}");
        let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["encrypted_value", "key_id"]);
        assert_eq!(body["key_id"], "kid-1");
        assert_eq!(open(body["encrypted_value"].as_str().unwrap(), &repo).as_deref(), Some(&b"s3cr3t-valor"[..]));
        // Lo que vuelve de GitHub de un secret no tiene dónde llevar su valor.
        let listed = serde_json::to_value(GitHubSecret { name: "N".into(), created_at: "a".into(), updated_at: "b".into() }).unwrap();
        assert_eq!(listed.as_object().unwrap().len(), 3);
    }

    /// El token sale del llavero, por servidor, y la segunda vez de la caché;
    /// el de un servidor no vale para otro. Roturas que atrapa: no ir al
    /// llavero cuando no está en la caché; una caché que no mira el servidor.
    #[test]
    fn the_token_comes_from_the_keychain_per_server() {
        let cache = TokenCache::default();
        let t = cache
            .token_with("s1", |id| {
                assert_eq!(id, "s1");
                Ok("ghp_one".into())
            })
            .unwrap();
        assert_eq!(t, "ghp_one");
        assert_eq!(cache.token_with("s1", |_| panic!("volvió al llavero teniéndolo")).unwrap(), "ghp_one");
        assert!(cache.token_with("s2", |_| Err("no entry".into())).is_err(), "s2 usó el token de s1");
    }

    /// Una referencia mal escrita no llega a `op`. Rotura que atrapa: quitar
    /// la comprobación (lanzaría `op read` con lo que sea).
    #[test]
    fn a_malformed_token_reference_is_refused_before_op() {
        let cache = TokenCache::default();
        for bad in ["op://vault/item", "op-env://Env/VAR", "ghp_literal"] {
            let err = load_token_from_1password(&cache, "s1", bad).unwrap_err();
            assert!(err.contains("is not op://"), "{bad}: {err}");
        }
    }
}
