//! Un servidor a mano: contesta entero lo que se le da por ruta y, en una
//! sola ruta, lo que la prueba le vaya mandando, cuando lo mande; cierra
//! esa respuesta al soltar el emisor. Con wiremock la respuesta sale entera
//! de golpe, y lo que se mira aquí es qué pasa con lo que va llegando.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot};

pub struct ByHand {
    pub base: String,
    /// Los trozos de la respuesta de la ruta a mano, cabecera incluida.
    pub send: mpsc::UnboundedSender<Vec<u8>>,
    /// La cabecera con que se pidió la ruta a mano, cuando ya se contestó.
    pub asked: oneshot::Receiver<String>,
    /// Las rutas que se pidieron, en orden.
    pub routes: Arc<Mutex<Vec<String>>>,
}

/// `canned`: (ruta, JSON) que se contestan enteros con 200. Lo demás, 404.
pub async fn serve(streamed: &str, canned: Vec<(String, String)>) -> ByHand {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (send, parts) = mpsc::unbounded_channel::<Vec<u8>>();
    let (said, asked) = oneshot::channel();
    let streamed = streamed.to_string();
    let parts = Arc::new(Mutex::new(Some((parts, said))));
    let routes = Arc::new(Mutex::new(Vec::new()));
    let seen = routes.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let (streamed, canned, parts, seen) = (streamed.clone(), canned.clone(), parts.clone(), seen.clone());
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if socket.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8(head).unwrap();
                let route = head.split(' ').nth(1).unwrap_or_default().split('?').next().unwrap_or_default().to_string();
                seen.lock().unwrap().push(route.clone());
                if route == streamed {
                    let Some((mut parts, said)) = parts.lock().unwrap().take() else { return };
                    while let Some(part) = parts.recv().await {
                        socket.write_all(&part).await.unwrap();
                        socket.flush().await.unwrap();
                    }
                    let _ = said.send(head);
                    return;
                }
                let reply = match canned.iter().find(|(path, _)| *path == route) {
                    Some((_, body)) => format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()),
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                };
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    ByHand { base, send, asked, routes }
}

/// El `.part` que se escribe al lado de `target` mientras llega.
pub fn part_of(target: &std::path::Path) -> std::path::PathBuf {
    let mut partial = target.as_os_str().to_owned();
    partial.push(".part");
    partial.into()
}
