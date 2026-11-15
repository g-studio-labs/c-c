//! El stream de eventos del servidor, y si hay conexión.
//!
//! Vino de `app/src-tauri/src/sse.rs`, que lo abría con el token que le pasaba
//! la pantalla y le devolvía cada evento tal cual. Aquí el token es el de la
//! sesión del core (en una cabecera, nunca en la URL, para que no acabe en un
//! log de accesos), un 401 se arregla renovando sin que nadie de fuera tenga
//! que enterarse, y los eventos de tarea ponen al día «Mi trabajo» y la tarea
//! abierta antes de salir hacia la plataforma.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::Core;

/// En qué está la conexión con el servidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum Connection {
    /// Nadie pidió el stream, o se paró.
    #[default]
    Idle,
    Connecting,
    Open,
    /// Se cayó y se está reintentando. Es lo que enseña el aviso de «sin
    /// conexión».
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ConnectionState {
    pub status: Connection,
    /// Por qué se cayó, para un log: no es texto para enseñar.
    pub detail: Option<String>,
}

/// Un evento del servidor, tal como llegó.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ServerEvent {
    /// `task:move`, `report:new`…; `message` si vino sin nombre.
    pub event: String,
    /// Casi siempre JSON, sin leer.
    pub data: String,
}

/// Un trozo del stream → un evento. Las líneas de comentario (`:`) no cuentan,
/// y varias líneas `data:` se unen con saltos, como dice la norma.
pub fn parse_frame(frame: &str) -> Option<ServerEvent> {
    let mut event = String::new();
    let mut data: Vec<&str> = Vec::new();

    for line in frame.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => event = value.to_string(),
            "data" => data.push(value),
            _ => {}
        }
    }

    if event.is_empty() && data.is_empty() {
        return None;
    }
    Some(ServerEvent {
        event: if event.is_empty() { "message".into() } else { event },
        data: data.join("\n"),
    })
}

/// Dónde acaba el primer evento completo (tras `\n\n` o `\r\n\r\n`).
fn frame_end(bytes: &[u8]) -> Option<usize> {
    let find = |needle: &[u8]| bytes.windows(needle.len()).position(|w| w == needle).map(|i| i + needle.len());
    match (find(b"\n\n"), find(b"\r\n\r\n")) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Lo que va llegando del stream, hasta que hay eventos enteros.
///
/// Guarda **bytes**, no texto: la red parte por donde quiere, y un trozo puede
/// acabar en mitad de una «ó». Pasar cada trozo a texto por separado (como
/// hacía `sse.rs`) cambia esa letra por «�» en el título de una tarea.
#[derive(Default)]
pub struct FrameBuffer {
    bytes: Vec<u8>,
}

impl FrameBuffer {
    /// Añade un trozo y devuelve los eventos que con él quedaron completos.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<ServerEvent> {
        self.bytes.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = frame_end(&self.bytes) {
            let frame: Vec<u8> = self.bytes.drain(..end).collect();
            if let Some(event) = parse_frame(&String::from_utf8_lossy(&frame)) {
                events.push(event);
            }
        }
        events
    }
}

/// Cuánto se espera antes del intento número `attempt`: el doble cada vez,
/// hasta treinta veces la base (30 s con la base de un segundo).
pub fn backoff(base: Duration, attempt: u32) -> Duration {
    (base * (1 << attempt.min(5))).min(base * 30)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskEvent {
    task_id: Option<String>,
}

/// El stream de este core.
pub struct Stream {
    core: Core,
}

impl Core {
    pub fn stream(&self) -> Stream {
        Stream { core: self.clone() }
    }
}

impl Stream {
    pub fn state(&self) -> ConnectionState {
        self.core.inner.connection.lock().expect("connection lock").clone()
    }

    fn set(&self, status: Connection, detail: Option<String>) {
        let state = ConnectionState { status, detail };
        *self.core.inner.connection.lock().expect("connection lock") = state.clone();
        self.core.inner.listener.connection_changed(state);
    }

    /// Si `turn` sigue siendo el stream vigente. Cada `start` y cada `stop`
    /// suben el contador, y el que se quedó atrás se retira solo: así no hay
    /// que cortar a nadie a media espera.
    fn current(&self, turn: u64) -> bool {
        self.core.inner.stream_turn.load(Ordering::SeqCst) == turn
    }

    /// Abre el stream y lo mantiene abierto hasta `stop` o hasta que no haya
    /// sesión. Llamarlo otra vez sustituye al anterior.
    pub fn start(&self) {
        let turn = self.core.inner.stream_turn.fetch_add(1, Ordering::SeqCst) + 1;
        let stream = self.core.stream();
        tokio::spawn(async move { stream.run(turn).await });
    }

    pub fn stop(&self) {
        self.core.inner.stream_turn.fetch_add(1, Ordering::SeqCst);
        self.set(Connection::Idle, None);
    }

    async fn run(&self, turn: u64) {
        let url = format!("{}/api/v1/events", self.core.base_url());
        let base = self.core.inner.config.stream_backoff;
        let mut attempt: u32 = 0;
        let mut was_down = false;

        while self.current(turn) {
            let Some(token) = self.core.access_token() else {
                self.set(Connection::Idle, None);
                return;
            };
            self.set(Connection::Connecting, None);

            let answer = self
                .core
                .inner
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Accept", "text/event-stream")
                .send()
                .await;
            if !self.current(turn) {
                return;
            }

            match answer {
                Ok(response) if response.status().is_success() => {
                    attempt = 0;
                    self.set(Connection::Open, None);
                    if was_down {
                        self.catch_up();
                    }
                    let Some(why) = self.read(response, turn).await else {
                        return; // otro stream tomó el relevo
                    };
                    self.set(Connection::Down, Some(why));
                }
                Ok(response) => {
                    let status = response.status().as_u16();
                    self.set(Connection::Down, Some(format!("server returned {status}")));
                    if status == 401 {
                        // Si la renovación da la sesión por terminada, el core
                        // la olvida y con ello para este stream: la vuelta
                        // siguiente ya no es la vigente.
                        let renewed = self.core.refresh(Some(token)).await.is_some();
                        // Con token nuevo se vuelve ya, una vez; si el
                        // servidor lo rechaza otra vez, con espera.
                        if renewed && attempt == 0 {
                            attempt = 1;
                            was_down = true;
                            continue;
                        }
                    }
                }
                Err(error) => self.set(Connection::Down, Some(error.to_string())),
            }

            was_down = true;
            tokio::time::sleep(backoff(base, attempt)).await;
            attempt = attempt.saturating_add(1);
        }
    }

    /// Lee hasta que el stream acaba, se queda callado demasiado, o deja de
    /// ser el vigente (`None`).
    async fn read(&self, mut response: reqwest::Response, turn: u64) -> Option<String> {
        let silence = self.core.inner.config.stream_silence;
        let mut buffer = FrameBuffer::default();
        loop {
            let next = tokio::time::timeout(silence, response.chunk()).await;
            if !self.current(turn) {
                return None;
            }
            let chunk = match next {
                // El servidor manda un ping cada 25 s. Tanto silencio es una
                // conexión muerta aunque el socket parezca abierto: soltarla
                // aquí es lo que fuerza un socket nuevo en el siguiente intento.
                Err(_) => return Some(format!("no data for {}s", silence.as_secs())),
                Ok(Ok(None)) => return Some("stream ended".into()),
                Ok(Err(error)) => return Some(error.to_string()),
                Ok(Ok(Some(chunk))) => chunk,
            };
            for event in buffer.push(&chunk) {
                self.dispatch(event);
            }
        }
    }

    /// Lo que pasó mientras no había stream no llegó: al volver se pide de
    /// nuevo lo que hay en pantalla.
    fn catch_up(&self) {
        let core = self.core.clone();
        tokio::spawn(async move {
            let work = core.my_work();
            let state = work.state();
            if state.loaded_key.is_some() {
                work.load(state.org_id).await;
            }
            core.tasks().refresh().await;
        });
    }

    fn dispatch(&self, event: ServerEvent) {
        if event.event == "ping" {
            return;
        }
        if let Some(kind) = event.event.strip_prefix("task:") {
            let task_id = serde_json::from_str::<TaskEvent>(&event.data).ok().and_then(|e| e.task_id);
            if let Some(task_id) = task_id {
                if kind == "delete" {
                    self.core.my_work().forget(&task_id);
                    self.core.tasks().task_deleted(&task_id);
                } else {
                    self.core.my_work().task_event(&task_id, kind == "new");
                    let core = self.core.clone();
                    tokio::spawn(async move { core.tasks().task_event(&task_id).await });
                }
            }
        }
        self.core.inner.listener.server_event(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_event_is_read() {
        let event = parse_frame("event: task:move\ndata: {\"listId\":\"l1\"}\n").unwrap();
        assert_eq!((event.event.as_str(), event.data.as_str()), ("task:move", "{\"listId\":\"l1\"}"));
    }

    #[test]
    fn an_unnamed_event_is_a_message_and_comments_do_not_count() {
        let event = parse_frame(": keep-alive\ndata: hello\n").unwrap();
        assert_eq!((event.event.as_str(), event.data.as_str()), ("message", "hello"));
        assert!(parse_frame(": just a comment\n").is_none());
    }

    #[test]
    fn several_data_lines_are_joined() {
        assert_eq!(parse_frame("event: x\ndata: a\ndata: b\n").unwrap().data, "a\nb");
    }

    /// Los eventos llegan partidos; sólo sale el que está entero.
    ///
    /// Rotura que atrapa: entregar medio JSON a quien escucha.
    #[test]
    fn only_whole_events_come_out() {
        let mut buffer = FrameBuffer::default();
        assert!(buffer.push(b"event: ping\ndata: {\"ts\":1}").is_empty());
        let events = buffer.push(b"\n\nevent: next\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"ts\":1}");
        assert_eq!(buffer.push(b"data: 2\r\n\r\n")[0].event, "next");
    }

    /// Una letra con acento partida entre dos trozos de red llega entera.
    ///
    /// Rotura que atrapa: pasar cada trozo a texto por su cuenta, que deja
    /// «revisi��n» en el título de una tarea.
    #[test]
    fn a_letter_split_across_chunks_arrives_whole() {
        let frame = "event: task:update\ndata: revisión\n\n".as_bytes();
        let cut = frame.iter().position(|byte| *byte == 0xC3).unwrap() + 1; // en mitad de la «ó»
        let mut buffer = FrameBuffer::default();
        assert!(buffer.push(&frame[..cut]).is_empty());
        assert_eq!(buffer.push(&frame[cut..])[0].data, "revisión");
    }

    /// La espera crece y tiene techo.
    ///
    /// Rotura que atrapa: reintentar sin pausa contra un servidor caído, o
    /// esperar cada vez más sin límite.
    #[test]
    fn the_wait_grows_and_has_a_ceiling() {
        let base = Duration::from_secs(1);
        let waits: Vec<u64> = (0..8).map(|attempt| backoff(base, attempt).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30, 30]);
    }
}
