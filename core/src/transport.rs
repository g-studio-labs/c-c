//! El viaje de ida y vuelta al servidor, sin opinión sobre quién llama.
//!
//! Vino de `app/src-tauri/src/api_client.rs`, y con su razón de ser: en un
//! webview, una petición por un socket que el servidor ya cerró no termina
//! nunca y la pantalla parece congelada. Aquí el pool es nuestro: los sockets
//! ociosos se retiran cuando decimos y toda petición tiene un plazo.
//!
//! No sabe de sesiones a propósito. Las cabeceras las pone quien llama, así
//! que sirve igual a la app (un par de tokens que se renueva) que al MCP (un
//! token personal fijo y sus cabeceras `X-Cac-*`).

use std::time::Duration;

use crate::error::ApiError;

/// Por debajo de los 60 s a los que un servidor o un proxy suele cerrar un
/// socket ocioso: lo retiramos nosotros antes de que lo retiren a nuestra
/// espalda.
const POOL_IDLE: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Una petición ya resuelta: URL entera y cabeceras puestas.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub url: String,
    /// El token viaja aquí y no en la URL, para que no acabe en un log de
    /// accesos.
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub timeout: Duration,
}

/// Lo que contestó el servidor. Un estado de error **es** una respuesta, no un
/// fallo de transporte: vuelve aquí para que quien llama lea el porqué.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

/// El cliente con su pool. Uno por proceso: construir uno por llamada tira el
/// keep-alive entero.
pub fn new_client() -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .pool_idle_timeout(POOL_IDLE)
        .connect_timeout(CONNECT_TIMEOUT)
        // Una llamada a la API que redirige es una mala configuración, no un
        // cambio de destino: mejor un error visible que un salto callado.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ApiError::Network(e.to_string()))
}

pub async fn send(client: &reqwest::Client, request: Request) -> Result<Reply, ApiError> {
    let method = reqwest::Method::from_bytes(request.method.to_uppercase().as_bytes())
        .map_err(|_| ApiError::Network(format!("unsupported method {}", request.method)))?;

    let mut builder = client
        .request(method, &request.url)
        .timeout(request.timeout);
    for (name, value) in &request.headers {
        if !name.is_empty() {
            builder = builder.header(name, value);
        }
    }
    if let Some(body) = request.body {
        builder = builder.body(body);
    }

    let response = builder.send().await.map_err(|e| {
        ApiError::Network(if e.is_timeout() {
            "timed out".to_string()
        } else if e.is_connect() {
            "could not connect".to_string()
        } else {
            e.to_string()
        })
    })?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| ApiError::Network(format!("response body: {e}")))?;
    Ok(Reply { status, body })
}
