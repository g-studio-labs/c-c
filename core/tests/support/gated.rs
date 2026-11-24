//! Un servidor de mentira con respuestas que la prueba suelta cuando quiere.
//!
//! wiremock contesta desde un solo hilo y su `respond` es síncrono: una
//! respuesta retenida ahí dentro deja al servidor sin contestar nada más, y
//! un `set_delay` es un plazo de reloj de pared, que en una máquina cargada
//! llega antes o después de lo que la prueba quería mirar. Aquí la retención
//! va por delante de wiremock, en un proxy: wiremock calcula la respuesta al
//! llegar la petición (lo que el servidor «sabía» entonces) y el proxy no la
//! entrega hasta que la prueba abre su [`Gate`]. Lo demás sigue contestando.
//!
//! Lo usan las pruebas del core (`#[path]`) y las del escritorio
//! (`desktop/src/testing.rs`).
#![allow(dead_code)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

/// La cabecera con la que una respuesta de wiremock pide que se retenga.
const GATE_HEADER: &str = "x-test-gate";

/// Sólo para que una prueba rota no se quede colgada: no es un plazo que
/// mida nada. Una prueba sana abre la puerta mucho antes.
const HANG_GUARD: Duration = Duration::from_secs(120);

struct GateState {
    open: tokio::sync::watch::Sender<bool>,
    held: AtomicUsize,
    passed: AtomicUsize,
}

fn gates() -> &'static Mutex<HashMap<u64, Arc<GateState>>> {
    static GATES: OnceLock<Mutex<HashMap<u64, Arc<GateState>>>> = OnceLock::new();
    GATES.get_or_init(Default::default)
}

/// Retiene las respuestas que se le dan hasta que la prueba la abre. Una
/// vez abierta, deja pasar todo lo que retenía y lo que llegue después.
#[derive(Clone)]
pub struct Gate {
    id: u64,
    state: Arc<GateState>,
}

impl Default for Gate {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        let state = Arc::new(GateState { open: tokio::sync::watch::channel(false).0, held: AtomicUsize::new(0), passed: AtomicUsize::new(0) });
        gates().lock().unwrap().insert(id, state.clone());
        Self { id, state }
    }
}

impl Gate {
    /// Esa respuesta, retenida por esta puerta.
    pub fn hold(&self, template: wiremock::ResponseTemplate) -> wiremock::ResponseTemplate {
        template.insert_header(GATE_HEADER, self.id.to_string().as_str())
    }

    pub fn open(&self) {
        self.state.open.send_replace(true);
    }

    /// Si ya se abrió: desde entonces, lo retenido ya está dicho.
    pub fn is_open(&self) -> bool {
        *self.state.open.borrow()
    }

    /// Cuántas respuestas están ahora mismo esperando a esta puerta: ya
    /// calculadas, sin entregar.
    pub fn held(&self) -> usize {
        self.state.held.load(Ordering::SeqCst)
    }

    /// Cuántas respuestas ha soltado ya.
    pub fn passed(&self) -> usize {
        self.state.passed.load(Ordering::SeqCst)
    }

    async fn wait(&self) {
        self.state.held.fetch_add(1, Ordering::SeqCst);
        let mut open = self.state.open.subscribe();
        let _ = tokio::time::timeout(HANG_GUARD, open.wait_for(|open| *open)).await;
        self.state.held.fetch_sub(1, Ordering::SeqCst);
    }
}

/// El proxy delante de un wiremock. Corre en su propio hilo y su propio
/// runtime, como wiremock: no depende de cómo corre la prueba.
pub struct GatedProxy {
    address: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl GatedProxy {
    pub fn start(upstream: String) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown, mut stop) = tokio::sync::oneshot::channel::<()>();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
                loop {
                    let stream = tokio::select! {
                        accepted = listener.accept() => match accepted {
                            Ok((stream, _)) => stream,
                            Err(_) => break,
                        },
                        _ = &mut stop => break,
                    };
                    let (client, upstream) = (client.clone(), upstream.clone());
                    tokio::spawn(async move {
                        let service = service_fn(move |request| forward(client.clone(), upstream.clone(), request));
                        let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), service).await;
                    });
                }
            });
        });
        Self { address, _shutdown: shutdown }
    }

    pub fn uri(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }
}

/// Espera a algo que viene de otro hilo. El tope sólo es para que una prueba
/// rota no se quede colgada.
pub async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..300 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no llegó a pasar: {what}");
}

/// Un proxy delante de ese servidor que vive lo que el proceso: para las
/// pruebas que sólo necesitan la dirección por donde hablar.
pub fn front(upstream: &str) -> String {
    let proxy = GatedProxy::start(upstream.to_string());
    let uri = proxy.uri();
    std::mem::forget(proxy);
    uri
}

async fn forward(client: reqwest::Client, upstream: String, request: hyper::Request<Incoming>) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let (parts, body) = request.into_parts();
    let body = body.collect().await.map(|body| body.to_bytes()).unwrap_or_default();
    let target = format!("{upstream}{}", parts.uri.path_and_query().map(|path| path.as_str()).unwrap_or("/"));
    let mut outgoing = client.request(parts.method.clone(), target).body(body);
    for (name, value) in &parts.headers {
        if name != hyper::header::HOST && name != hyper::header::CONTENT_LENGTH && name != hyper::header::TRANSFER_ENCODING {
            outgoing = outgoing.header(name, value);
        }
    }
    let answer = match outgoing.send().await {
        Ok(answer) => answer,
        Err(error) => {
            let mut response = hyper::Response::new(Full::new(Bytes::from(error.to_string())));
            *response.status_mut() = hyper::StatusCode::BAD_GATEWAY;
            return Ok(response);
        }
    };
    let gate = answer
        .headers()
        .get(GATE_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|id| id.parse::<u64>().ok())
        .and_then(|id| gates().lock().unwrap().get(&id).cloned())
        .map(|state| Gate { id: 0, state });
    let status = answer.status();
    let mut headers = answer.headers().clone();
    let bytes = answer.bytes().await.unwrap_or_default();
    if let Some(gate) = &gate {
        gate.wait().await;
    }
    headers.remove(GATE_HEADER);
    headers.remove(hyper::header::TRANSFER_ENCODING);
    headers.remove(hyper::header::CONNECTION);
    let mut response = hyper::Response::new(Full::new(bytes));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    if let Some(gate) = gate {
        gate.state.passed.fetch_add(1, Ordering::SeqCst);
    }
    Ok(response)
}
