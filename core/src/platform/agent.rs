//! Hablarle al agente de un servidor (`http://<host>:<agentPort>`).
//!
//! Las reglas son las de `app/src/lib/agent.ts`, que es donde se aprendieron:
//!
//! - **Todo lleva plazo.** Se habla directo con una VPS por la red; una
//!   petición a una máquina que no contesta, o a un socket medio cerrado tras
//!   dormir el portátil, no termina nunca. Con las peticiones apiladas, la
//!   app parecía congelada hasta reiniciarla.
//! - **Un agente con identidad (v2) exige un pase** firmado por el backend
//!   (`POST /api/v1/servers/{id}/agent-session`). Dura minutos: se guarda por
//!   servidor y se pide otro cuando le queda menos de un minuto.
//! - **Un 401 es un pase que ya no vale** (se reacuñó la identidad, o el reloj
//!   de esta máquina va adelantado): se tira, se pide otro y se reintenta una
//!   vez. Una sola: un segundo 401 es una respuesta, no mala suerte.
//! - A un agente de los de antes se le habla sin pase, como siempre.
//!
//! El pase va siempre en la cabecera, también en los streams: `agent.ts` lo
//! pone en la URL (`?access_token=`) porque `EventSource` no sabe mandar
//! cabeceras, y aquí no hace falta. En la URL acabaría en algún log.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::ApiError;
use crate::platform::servers::Server;
use crate::stream::{FrameBuffer, ServerEvent};
use crate::transport::{self, Reply, Request};
use crate::Core;

/// Cuánto se espera a una llamada al agente.
pub const AGENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Cuánto se espera a una prueba de vida: corta, porque se hacen varias a la
/// vez y una máquina caída no debe retrasar a las demás.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
/// Con menos de esto por delante, un pase ya no se usa: la petición que lo
/// lleva podría llegar con él caducado.
pub const PASS_MARGIN: Duration = Duration::from_secs(60);

/// Un pase para un agente.
#[derive(Debug, Clone)]
pub(crate) struct AgentPass {
    token: String,
    expires_at: DateTime<Utc>,
}

/// Lo que contesta `agent-session` (`domain.AgentSessionResponse`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentSessionResponse {
    token: String,
    expires_at: String,
}

impl AgentSessionResponse {
    /// Una caducidad que no se entiende cuenta como ya caducada: el pase vale
    /// para esta petición y la siguiente pide otro. Leerla como «nunca»
    /// guardaría uno muerto hasta el primer 401.
    fn into_pass(self) -> AgentPass {
        let expires_at = DateTime::parse_from_rfc3339(&self.expires_at).map(|at| at.with_timezone(&Utc)).unwrap_or(DateTime::<Utc>::MIN_UTC);
        AgentPass { token: self.token, expires_at }
    }
}

impl AgentPass {
    /// Si todavía vale para una petición que sale ahora.
    fn usable_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at - now > chrono::Duration::from_std(PASS_MARGIN).expect("margin fits")
    }
}

/// Los pases de esta persona, por servidor. Son de quien está dentro: al
/// salir se tiran (`Agent::clear`).
#[derive(Default)]
pub(crate) struct AgentPasses(HashMap<String, AgentPass>);

/// La raíz del agente de un servidor. Una IPv6 va entre corchetes, o el
/// puerto se leería como parte de la dirección.
pub fn agent_base(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("http://[{host}]:{port}")
    } else {
        format!("http://{host}:{port}")
    }
}

/// El cliente de los agentes de este core.
pub struct Agent {
    core: Core,
}

impl Core {
    pub fn agent(&self) -> Agent {
        Agent { core: self.clone() }
    }
}

impl Agent {
    /// Un GET al agente. Un estado de error vuelve como respuesta, como en
    /// `Core::request`: quien llama lee el porqué.
    pub async fn get(&self, server: &Server, path: &str) -> Result<Reply, ApiError> {
        self.send(server, "GET", path, None, AGENT_TIMEOUT).await
    }

    /// Un POST al agente, con cuerpo JSON si lo lleva.
    pub async fn post(&self, server: &Server, path: &str, body: Option<serde_json::Value>) -> Result<Reply, ApiError> {
        self.send(server, "POST", path, body.map(|body| body.to_string()), AGENT_TIMEOUT).await
    }

    /// Un GET cuyo `data` (el agente contesta con el mismo sobre que el
    /// backend) se lee ya con su tipo.
    pub async fn get_data<T: for<'de> Deserialize<'de>>(&self, server: &Server, path: &str) -> Result<T, ApiError> {
        crate::session::unwrap_data(&self.get(server, path).await?)
    }

    /// Una lista del agente. Vacía, llega sin `data`.
    pub async fn get_list<T: for<'de> Deserialize<'de>>(&self, server: &Server, path: &str) -> Result<Vec<T>, ApiError> {
        crate::session::unwrap_list(&self.get(server, path).await?)
    }

    /// ¿Contesta su agente?
    ///
    /// Pregunta por los nodos y no por las estadísticas: las dos prueban lo
    /// mismo —que hay alguien escuchando— y la lista de nodos son cuatro
    /// líneas, mientras que las estadísticas recorren todos los contenedores.
    /// Una prueba de vida no debe costarle nada al servidor que prueba.
    pub async fn alive(&self, server: &Server) -> bool {
        matches!(self.send(server, "GET", "/api/v1/nodes", None, PROBE_TIMEOUT).await, Ok(reply) if reply.status < 400)
    }

    /// Abre un stream de eventos del agente (los logs de un servicio).
    ///
    /// Sin plazo para el stream entero, que dura lo que se mire; sí para
    /// conectar. Un 401 al abrirlo reintenta una vez, como el resto.
    pub async fn stream(&self, server: &Server, path: &str) -> Result<AgentStream, ApiError> {
        let mut response = self.open_stream(server, path).await?;
        if response.status().as_u16() == 401 && server.has_agent_token {
            self.forget_pass(&server.id);
            response = self.open_stream(server, path).await?;
        }
        let status = response.status().as_u16();
        if status >= 400 {
            let body = response.text().await.unwrap_or_default();
            return Err(crate::session::server_error(&Reply { status, body }));
        }
        Ok(AgentStream { response, buffer: FrameBuffer::default(), ready: VecDeque::new() })
    }

    /// Tira los pases: son de quien sale.
    pub(crate) fn clear(&self) {
        self.passes().0.clear();
    }

    async fn open_stream(&self, server: &Server, path: &str) -> Result<reqwest::Response, ApiError> {
        let mut request = self.core.inner.http.get(format!("{}{path}", agent_base(&server.host, server.agent_port)));
        if let Some(pass) = self.pass_for(server).await? {
            request = request.bearer_auth(pass);
        }
        request.header("Accept", "text/event-stream").send().await.map_err(|e| ApiError::Network(e.to_string()))
    }

    async fn send(&self, server: &Server, method: &str, path: &str, body: Option<String>, timeout: Duration) -> Result<Reply, ApiError> {
        let reply = self.send_once(server, method, path, body.clone(), timeout).await?;
        if reply.status != 401 || !server.has_agent_token {
            return Ok(reply);
        }
        self.forget_pass(&server.id);
        self.send_once(server, method, path, body, timeout).await
    }

    async fn send_once(&self, server: &Server, method: &str, path: &str, body: Option<String>, timeout: Duration) -> Result<Reply, ApiError> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if let Some(pass) = self.pass_for(server).await? {
            headers.push(("Authorization".to_string(), format!("Bearer {pass}")));
        }
        let request = Request {
            method: method.to_string(),
            url: format!("{}{path}", agent_base(&server.host, server.agent_port)),
            headers,
            body,
            timeout,
        };
        transport::send(&self.core.inner.http, request).await
    }

    /// El pase para este servidor: el guardado si aún vale, o uno nuevo. Un
    /// agente sin identidad no lo necesita, y no se pide.
    ///
    /// Si el backend no lo da, es un error y no «sin pase»: un agente con
    /// identidad contestaría 401 y se perdería el porqué (no eres de esa
    /// organización, el servidor ya no existe).
    async fn pass_for(&self, server: &Server) -> Result<Option<String>, ApiError> {
        if !server.has_agent_token {
            return Ok(None);
        }
        if let Some(pass) = self.passes().0.get(&server.id).filter(|pass| pass.usable_at(Utc::now())) {
            return Ok(Some(pass.token.clone()));
        }
        let reply: AgentSessionResponse =
            self.core.call("POST", &format!("/api/v1/servers/{}/agent-session", server.id), Some(serde_json::json!({}))).await?;
        let pass = reply.into_pass();
        let token = pass.token.clone();
        self.passes().0.insert(server.id.clone(), pass);
        Ok(Some(token))
    }

    fn forget_pass(&self, server_id: &str) {
        self.passes().0.remove(server_id);
    }

    fn passes(&self) -> std::sync::MutexGuard<'_, AgentPasses> {
        self.core.inner.agent_passes.lock().expect("agent passes lock")
    }
}

/// Un stream de eventos abierto con un agente.
pub struct AgentStream {
    response: reqwest::Response,
    buffer: FrameBuffer,
    ready: VecDeque<ServerEvent>,
}

impl AgentStream {
    /// El siguiente evento, o `None` cuando el agente cierra o se corta.
    pub async fn next(&mut self) -> Option<ServerEvent> {
        loop {
            if let Some(event) = self.ready.pop_front() {
                return Some(event);
            }
            let chunk = self.response.chunk().await.ok()??;
            self.ready.extend(self.buffer.push(&chunk));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass_ending_in(seconds: i64, now: DateTime<Utc>) -> AgentPass {
        AgentPass { token: "t".into(), expires_at: now + chrono::Duration::seconds(seconds) }
    }

    /// Rotura que atrapa: usar un pase al que le queda menos de un minuto
    /// (llegaría caducado), o tirar uno al que le queda de sobra.
    #[test]
    fn a_pass_is_used_until_a_minute_before_it_ends() {
        let now = Utc::now();
        assert!(pass_ending_in(61, now).usable_at(now));
        assert!(!pass_ending_in(60, now).usable_at(now));
        assert!(!pass_ending_in(-5, now).usable_at(now));
    }

    /// Rotura que atrapa: pegar el puerto a una IPv6 sin corchetes, que la
    /// convierte en otra dirección.
    #[test]
    fn the_agent_lives_on_its_own_port() {
        assert_eq!(agent_base("203.0.113.7", 8080), "http://203.0.113.7:8080");
        assert_eq!(agent_base("2001:db8::1", 8080), "http://[2001:db8::1]:8080");
        assert_eq!(agent_base("vps.example.com", 9000), "http://vps.example.com:9000");
    }
}
