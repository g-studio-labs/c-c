//! Las utilidades de la pantalla de tokens: leer y verificar un JWT, generar
//! ids, hashes y HMAC, hashes de contraseña y codificar o descodificar.
//!
//! Salen de la app de Tauri (`crypto_tools.rs`) para que la nativa use lo
//! mismo: decir «firma válida» de un token que no lo es no puede depender de
//! en qué app se pegó. Todo corre en la máquina; nada viaja.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Serialize;

// ─── JWT ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JwtDecoded {
    pub header: String,
    pub payload: String,
    pub signature_valid: Option<bool>,
    pub error: Option<String>,
}

pub fn jwt_decode(token: &str, secret: Option<&str>, algorithm: Option<&str>) -> JwtDecoded {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return JwtDecoded {
            header: String::new(),
            payload: String::new(),
            signature_valid: None,
            error: Some("Invalid JWT: expected 3 parts".into()),
        };
    }

    let decode_part = |part: &str| -> Result<String, String> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(part)
            .map_err(|e| format!("Base64 decode: {e}"))?;
        let raw = String::from_utf8_lossy(&bytes).to_string();
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(v) => Ok(serde_json::to_string_pretty(&v).unwrap_or(raw)),
            Err(_) => Ok(raw),
        }
    };

    let header = decode_part(parts[0]).unwrap_or_default();
    let payload = decode_part(parts[1]).unwrap_or_default();

    let signature_valid = secret.and_then(|s| {
        if s.is_empty() {
            return None;
        }
        let alg = algorithm.unwrap_or("HS256");
        let validation_alg = match alg {
            "HS256" => jsonwebtoken::Algorithm::HS256,
            "HS384" => jsonwebtoken::Algorithm::HS384,
            "HS512" => jsonwebtoken::Algorithm::HS512,
            _ => return Some(false),
        };
        let key = jsonwebtoken::DecodingKey::from_secret(s.as_bytes());
        let mut validation = jsonwebtoken::Validation::new(validation_alg);
        validation.validate_exp = false;
        validation.validate_aud = false;
        validation.required_spec_claims.clear();
        match jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation) {
            Ok(_) => Some(true),
            Err(_) => Some(false),
        }
    });

    JwtDecoded {
        header,
        payload,
        signature_valid,
        error: None,
    }
}

/// Una de las tres fechas de un JWT, dicha (`iat`, `nbf`, `exp`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimeClaim {
    /// `iat`, `nbf` o `exp`.
    pub key: String,
    /// Segundos desde 1970, como dice la especificación (no milisegundos).
    pub at: i64,
    /// `exp` ya pasó.
    pub expired: bool,
    /// `nbf` todavía no llegó.
    pub not_yet: bool,
}

/// Las fechas de un JWT, para decir en palabras si caducó: `exp: 1755640800`
/// es el campo a por el que se viene, y el que una persona no sabe leer.
/// Un payload que no es JSON no tiene nada que explicar.
pub fn time_claims(payload: &str, now: i64) -> Vec<TimeClaim> {
    let Ok(serde_json::Value::Object(claims)) = serde_json::from_str::<serde_json::Value>(payload) else { return Vec::new() };
    ["iat", "nbf", "exp"]
        .into_iter()
        .filter_map(|key| {
            let at = claims.get(key)?.as_f64()? as i64;
            Some(TimeClaim { key: key.to_string(), at, expired: key == "exp" && at < now, not_yet: key == "nbf" && at > now })
        })
        .collect()
}

// ─── UUID / CUID2 ────────────────────────────────────────────────────────────

/// `uuid-v4`, `uuid-v7` o `cuid2`.
pub fn generate_id(kind: &str) -> Result<String, String> {
    match kind {
        "uuid-v4" => Ok(generate_uuid_v4()),
        "uuid-v7" => Ok(generate_uuid_v7()),
        "cuid2" => Ok(generate_cuid2()),
        _ => Err(format!("Unknown id kind: {kind}")),
    }
}

pub fn generate_uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn generate_uuid_v7() -> String {
    uuid::Uuid::now_v7().to_string()
}

pub fn generate_cuid2() -> String {
    cuid2::create_id()
}

// ─── Hash ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HashResult {
    pub hash: String,
    pub algorithm: String,
}

pub fn hash_text(input: &str, algorithm: &str) -> Result<HashResult, String> {
    use sha2::Digest;

    let hash = match algorithm {
        "md5" => {
            let mut h = md5::Md5::new();
            h.update(input.as_bytes());
            hex::encode(h.finalize())
        }
        "sha256" => {
            let mut h = sha2::Sha256::new();
            h.update(input.as_bytes());
            hex::encode(h.finalize())
        }
        "sha384" => {
            let mut h = sha2::Sha384::new();
            h.update(input.as_bytes());
            hex::encode(h.finalize())
        }
        "sha512" => {
            let mut h = sha2::Sha512::new();
            h.update(input.as_bytes());
            hex::encode(h.finalize())
        }
        _ => return Err(format!("Unsupported algorithm: {algorithm}")),
    };

    Ok(HashResult {
        hash,
        algorithm: algorithm.to_string(),
    })
}

pub fn hmac_sign(input: &str, key: &str, algorithm: &str) -> Result<HashResult, String> {
    use hmac::{Hmac, Mac};

    let hash = match algorithm {
        "hmac-sha256" => {
            let mut mac =
                Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).map_err(|e| e.to_string())?;
            mac.update(input.as_bytes());
            hex::encode(mac.finalize().into_bytes())
        }
        "hmac-sha512" => {
            let mut mac =
                Hmac::<sha2::Sha512>::new_from_slice(key.as_bytes()).map_err(|e| e.to_string())?;
            mac.update(input.as_bytes());
            hex::encode(mac.finalize().into_bytes())
        }
        _ => return Err(format!("Unsupported HMAC algorithm: {algorithm}")),
    };

    Ok(HashResult {
        hash,
        algorithm: algorithm.to_string(),
    })
}

// ─── Bcrypt / Argon2 ─────────────────────────────────────────────────────────

pub fn bcrypt_hash(input: &str, cost: u32) -> Result<String, String> {
    bcrypt::hash(input, cost).map_err(|e| e.to_string())
}

pub fn bcrypt_verify(input: &str, hash: &str) -> Result<bool, String> {
    bcrypt::verify(input, hash).map_err(|e| e.to_string())
}

pub fn argon2_hash(input: &str) -> Result<String, String> {
    use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    Argon2::default()
        .hash_password(input.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn argon2_verify(input: &str, hash: &str) -> Result<bool, String> {
    use argon2::{password_hash::PasswordHash, Argon2, PasswordVerifier};
    let parsed = PasswordHash::new(hash).map_err(|e| e.to_string())?;
    Ok(Argon2::default()
        .verify_password(input.as_bytes(), &parsed)
        .is_ok())
}

// ─── Base64 encode / decode ──────────────────────────────────────────────────

/// `codec` es `base64`, `url` o `hex`; `direction`, `encode` o `decode`.
pub fn encode_decode(input: &str, codec: &str, direction: &str) -> Result<String, String> {
    match (codec, direction) {
        ("base64", "encode") => Ok(base64_encode(input)),
        ("base64", "decode") => base64_decode(input),
        ("url", "encode") => Ok(url_encode(input)),
        ("url", "decode") => url_decode(input),
        ("hex", "encode") => Ok(hex_encode(input)),
        ("hex", "decode") => hex_decode(input),
        _ => Err(format!("Unknown codec/direction: {codec}/{direction}")),
    }
}

pub fn base64_encode(input: &str) -> String {
    B64.encode(input.as_bytes())
}

pub fn base64_decode(input: &str) -> Result<String, String> {
    let bytes = B64
        .decode(input)
        .map_err(|e| format!("Base64 decode: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("UTF-8 decode: {e}"))
}

// ─── URL encode / decode ─────────────────────────────────────────────────────

pub fn url_encode(input: &str) -> String {
    urlencoding_encode(input)
}

pub fn url_decode(input: &str) -> Result<String, String> {
    urlencoding_decode(input)
}

fn urlencoding_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(
                    char::from_digit((byte >> 4) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((byte & 0xf) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
            }
        }
    }
    out
}

fn urlencoding_decode(input: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().ok_or("Incomplete percent encoding")?;
            let lo = chars.next().ok_or("Incomplete percent encoding")?;
            let val = u8::from_str_radix(&format!("{}{}", hi as char, lo as char), 16)
                .map_err(|e| format!("Invalid percent encoding: {e}"))?;
            bytes.push(val);
        } else if b == b'+' {
            bytes.push(b' ');
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).map_err(|e| format!("UTF-8 decode: {e}"))
}

// ─── Hex encode / decode ─────────────────────────────────────────────────────

pub fn hex_encode(input: &str) -> String {
    hex::encode(input.as_bytes())
}

pub fn hex_decode(input: &str) -> Result<String, String> {
    let bytes = hex::decode(input).map_err(|e| format!("Hex decode: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("UTF-8 decode: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El de jwt.io, firmado con `your-256-bit-secret`.
    const TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

    /// Rotura que atrapa: dar por buena la firma de un token con otro
    /// secreto, con la firma retocada, o con otro algoritmo que el suyo; o no
    /// dar por buena la que sí lo es.
    #[test]
    fn a_jwt_with_a_bad_signature_is_invalid() {
        assert_eq!(jwt_decode(TOKEN, Some("your-256-bit-secret"), Some("HS256")).signature_valid, Some(true));
        assert_eq!(jwt_decode(TOKEN, Some("otro-secreto"), Some("HS256")).signature_valid, Some(false));
        let tampered = format!("{}x", &TOKEN[..TOKEN.len() - 1]);
        assert_eq!(jwt_decode(&tampered, Some("your-256-bit-secret"), Some("HS256")).signature_valid, Some(false));
        assert_eq!(jwt_decode(TOKEN, Some("your-256-bit-secret"), Some("HS512")).signature_valid, Some(false));
        let read = jwt_decode(TOKEN, None, None);
        assert_eq!(read.signature_valid, None, "sin secreto no se puede decir nada de la firma");
        assert!(read.payload.contains("John Doe"));
        assert!(jwt_decode("no.es", None, None).error.is_some());
    }

    /// Rotura que atrapa: un hash o un HMAC que no es el del algoritmo que
    /// dice (vectores de FIPS 180 y de la RFC 4231).
    #[test]
    fn the_hash_matches_a_known_vector() {
        assert_eq!(hash_text("abc", "md5").unwrap().hash, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hash_text("abc", "sha256").unwrap().hash, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hash_text("abc", "sha384").unwrap().hash, "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7");
        assert_eq!(
            hash_text("abc", "sha512").unwrap().hash,
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(hmac_sign("what do ya want for nothing?", "Jefe", "hmac-sha256").unwrap().hash, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert!(hash_text("abc", "sha1").is_err());
    }

    /// Rotura que atrapa: un hash de contraseña que no se verifica con la
    /// suya, o que se verifica con otra.
    #[test]
    fn a_password_hash_verifies_only_its_password() {
        let bcrypt = bcrypt_hash("hunter22", 4).unwrap();
        assert!(bcrypt_verify("hunter22", &bcrypt).unwrap());
        assert!(!bcrypt_verify("hunter23", &bcrypt).unwrap());
        let argon = argon2_hash("hunter22").unwrap();
        assert!(argon2_verify("hunter22", &argon).unwrap());
        assert!(!argon2_verify("hunter23", &argon).unwrap());
    }

    /// Rotura que atrapa: leer `exp` en milisegundos (todo token caducó en
    /// 1970) o decir caducado lo que vence mañana.
    #[test]
    fn the_time_claims_say_if_it_expired() {
        let claims = time_claims(r#"{"iat":1000,"nbf":3000,"exp":1500}"#, 2000);
        assert_eq!(claims.iter().map(|claim| (claim.key.as_str(), claim.at, claim.expired, claim.not_yet)).collect::<Vec<_>>(), [("iat", 1000, false, false), ("nbf", 3000, false, true), ("exp", 1500, true, false)]);
        assert!(!time_claims(r#"{"exp":2500}"#, 2000)[0].expired);
        assert!(time_claims("no es json", 0).is_empty());
    }

    /// Rotura que atrapa: escapar de más o de menos en una URL, o no volver
    /// de lo codificado a lo escrito.
    #[test]
    fn encoding_goes_and_comes_back() {
        assert_eq!(encode_decode("a b/ñ", "url", "encode").unwrap(), "a%20b%2F%C3%B1");
        assert_eq!(encode_decode("a+b%20c", "url", "decode").unwrap(), "a b c");
        for codec in ["base64", "url", "hex"] {
            let there = encode_decode("hola, ñandú", codec, "encode").unwrap();
            assert_eq!(encode_decode(&there, codec, "decode").unwrap(), "hola, ñandú", "{codec}");
        }
        assert!(encode_decode("zz", "hex", "decode").is_err());
    }
}
