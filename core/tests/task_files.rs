//! Los ficheros de una tarea, contra un servidor de mentira. Cada prueba
//! dice la rotura que atrapa, comprobada rompiendo el código (`/mutar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cac_core::{
    file_citation, image_type, thumbnail_size, with_citation, Cited, Picture,
    attachment_path, is_image, multipart, safe_file_name, size_label, ApiError, Config, Core, NoListener, Storage,
    TaskAttachment, MAX_ATTACHMENT_BYTES,
};
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct MemoryStorage(Mutex<HashMap<String, String>>);

impl Storage for MemoryStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn remove(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}

/// Sólo se pide lo que vive en la API de este servidor.
///
/// Roturas que atrapa: mandar la sesión a la dirección de un bucket o a
/// cualquier otro sitio que alguien dejara escrito en un adjunto; salirse de
/// la API con `..`; colar una consulta o un salto de línea en la petición.
#[test]
fn only_this_servers_files_are_asked_for() {
    let ours = "/api/v1/tasks/t1/attachments/a1/raw";
    assert_eq!(attachment_path(ours), Some(ours));

    for other in [
        "https://bucket.s3.us-east-1.amazonaws.com/tasks/t1/x.png",
        "//evil.example/api/v1/x",
        "/api/v1//evil.example/x",
        "/api/v2/tasks/t1",
        "/health",
        "/proxy/api/v1/tasks/t1/attachments/a1/raw",
        "api/v1/tasks/t1/attachments/a1/raw",
        "/api/v1/tasks/../../admin",
        "/api/v1/tasks/t1/raw?token=x",
        "/api/v1/tasks/t1/raw#x",
        "/api/v1/tasks/t1/raw\r\nHost: evil",
        "/api/v1/tasks/t1 /raw",
        "/api/v1/tasks\\t1",
        "/api/v1/@evil.example",
        "",
    ] {
        assert_eq!(attachment_path(other), None, "se pediría {other:?}");
    }
}

/// El nombre con que se escribe un fichero no se sale de su carpeta.
///
/// Roturas que atrapa: `../../.ssh/authorized_keys` escrito donde dice;
/// un fichero oculto; un nombre vacío; perder la extensión o los acentos.
#[test]
fn a_file_name_stays_in_its_folder() {
    assert_eq!(safe_file_name("Informe año 2026.pdf"), "Informe año 2026.pdf");
    assert_eq!(safe_file_name("../../.ssh/authorized_keys"), "sshauthorized_keys");
    assert_eq!(safe_file_name("..\\..\\boot.ini"), "boot.ini");
    assert_eq!(safe_file_name(".bashrc"), "bashrc");
    assert_eq!(safe_file_name("  . .oculto"), "oculto");
    assert_eq!(safe_file_name("a\0b\n.txt"), "ab.txt");
    assert_eq!(safe_file_name("///"), "file");
    assert_eq!(safe_file_name(""), "file");
}

fn file(name: &str, kind: Option<&str>) -> TaskAttachment {
    TaskAttachment { file_name: name.into(), content_type: kind.map(str::to_string), ..TaskAttachment::default() }
}

/// Una imagen se reconoce por su tipo y, si no lo trae, por su nombre.
///
/// Roturas que atrapa: una captura sin tipo listada como un fichero
/// cualquiera; un PDF llamado `foto.png.pdf` pintado como imagen.
#[test]
fn an_image_is_told_by_its_type_or_its_name() {
    assert!(is_image(&file("x", Some("image/png"))));
    assert!(is_image(&file("Captura.JPEG", None)));
    assert!(is_image(&file("a.webp", Some("application/octet-stream"))));
    assert!(!is_image(&file("foto.png.pdf", Some("application/pdf"))));
    assert!(!is_image(&file("png", None)));
    assert!(!is_image(&file("notas.txt", None)));
}

/// Roturas que atrapa: «0 KB» para un fichero pequeño; megas contados como
/// kilos.
#[test]
fn a_size_is_said_to_be_read() {
    assert_eq!(size_label(10), "1 KB");
    assert_eq!(size_label(12 * 1024), "12 KB");
    assert_eq!(size_label(1024 * 1024 - 1), "1024 KB");
    assert_eq!(size_label(1024 * 1024), "1.0 MB");
    assert_eq!(size_label(1_500_000), "1.4 MB");
}

/// El cuerpo de una subida lleva el fichero intacto en el campo `file`, y un
/// nombre raro no rompe la cabecera.
///
/// Roturas que atrapa: un nombre con comillas que corta la cabecera o con un
/// salto de línea que escribe otra; una frontera que aparece dentro del
/// fichero y lo parte; bytes cambiados.
#[test]
fn an_upload_carries_the_file_whole() {
    let bytes = b"\x89PNG\r\n----cac-file-00000000\r\nresto\x00\xff".to_vec();
    let (content_type, body) = multipart("mi \"foto\"\r\nX-Evil: 1.png", &bytes);

    let boundary = content_type.strip_prefix("multipart/form-data; boundary=").expect("dice su frontera");
    assert_ne!(boundary, "----cac-file-00000000", "la frontera aparece dentro del fichero");
    let head = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"mi %22foto%22%0D%0AX-Evil: 1.png\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    );
    let tail = format!("\r\n--{boundary}--\r\n");
    assert!(body.starts_with(head.as_bytes()), "{}", String::from_utf8_lossy(&body));
    assert!(body.ends_with(tail.as_bytes()));
    assert_eq!(&body[head.len()..body.len() - tail.len()], &bytes[..]);
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn attachment(id: &str, url: &str, comment: Value) -> Value {
    json!({ "id": id, "itemId": "t1", "commentId": comment, "url": url, "fileName": format!("{id}.png") })
}

fn detail(attachments: Value) -> Value {
    json!({
        "task": {
            "id": "t1", "createdAt": "2026-11-14T20:00:00Z", "updatedAt": "2026-11-14T20:00:00Z",
            "listId": "l1", "seq": 12, "title": "Tarea", "status": "pending", "priority": "medium",
        },
        "listName": "tasks", "spaceName": "Espacio",
        "status": { "id": "s-pending", "listId": "l1", "name": "pending", "kind": "open", "status": "pending" },
        "comments": [{
            "id": "c1", "body": "mira", "createdAt": "2026-11-14T20:00:00Z",
            "attachments": [attachment("a-comment", "/api/v1/tasks/t1/attachments/a-comment/raw", json!("c1"))],
        }],
        "attachments": attachments,
    })
}

/// Una tarea abierta con un fichero suyo, uno de un comentario y uno de
/// antes del proxy, que guarda la dirección del bucket.
async fn rig() -> (MockServer, Core) {
    let server = MockServer::start().await;
    let core = Core::new(Config::new(server.uri()), Arc::new(MemoryStorage::default()), Arc::new(NoListener)).unwrap();
    let ana = serde_json::from_value(json!({ "id": "u1", "username": "ana" })).unwrap();
    core.sign_in(ana, "access".into(), "refresh".into()).unwrap();
    let files = json!([
        attachment("a1", "/api/v1/tasks/t1/attachments/a1/raw", Value::Null),
        attachment("a-old", "https://bucket.s3.us-east-1.amazonaws.com/tasks/t1/old.png", Value::Null),
    ]);
    Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(detail(files))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/task-lists/l1/statuses")).respond_with(ok(json!([]))).mount(&server).await;
    core.tasks().open("t1").await;
    (server, core)
}

fn code(error: ApiError) -> String {
    match error {
        ApiError::Server { code, .. } => code,
        other => other.to_string(),
    }
}

async fn asked(server: &MockServer, verb: &str, route: &str) -> Vec<wiremock::Request> {
    let all = server.received_requests().await.unwrap();
    all.into_iter().filter(|request| request.method.as_str() == verb && request.url.path() == route).collect()
}

const RAW: &str = "/api/v1/tasks/t1/attachments/a1/raw";

/// Los bytes de un adjunto llegan intactos, pedidos con la sesión en la
/// cabecera; también los de un comentario.
///
/// Roturas que atrapa: leer el fichero como texto (una imagen deja de serlo);
/// pedirlo sin sesión; no encontrar los adjuntos de los comentarios.
#[tokio::test]
async fn a_file_comes_whole_and_asked_with_the_session() {
    let (server, core) = rig().await;
    let png = vec![0x89, b'P', b'N', b'G', 0x00, 0xff, 0xfe, 0x80];
    for route in [RAW, "/api/v1/tasks/t1/attachments/a-comment/raw"] {
        Mock::given(method("GET"))
            .and(path(route))
            .and(header("Authorization", "Bearer access"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png.clone()))
            .mount(&server)
            .await;
    }

    assert_eq!(core.tasks().attachment_bytes("a1").await.unwrap(), png);
    assert_eq!(core.tasks().attachment_bytes("a-comment").await.unwrap(), png);
}

/// Lo que no es de la tarea abierta, o no vive en este servidor, no se pide.
///
/// Roturas que atrapa: bajar con la sesión un fichero cualquiera dando su
/// id; mandar la sesión a la dirección del bucket que guarda una fila vieja.
#[tokio::test]
async fn what_is_not_this_tasks_or_this_servers_is_not_asked_for() {
    let (server, core) = rig().await;
    let before = server.received_requests().await.unwrap().len();

    assert_eq!(code(core.tasks().attachment_bytes("a-otra-tarea").await.unwrap_err()), "attachment-not-found");
    assert_eq!(code(core.tasks().attachment_bytes("a-old").await.unwrap_err()), "not-an-attachment");
    assert_eq!(code(core.tasks().remove_attachment("a-otra-tarea").await.unwrap_err()), "attachment-not-found");

    assert_eq!(server.received_requests().await.unwrap().len(), before, "pidió algo");
    core.tasks().close();
    assert_eq!(code(core.tasks().attachment_bytes("a1").await.unwrap_err()), "no-task-open");
}

/// Con el token caducado el servidor contesta 404 en esta ruta: se renueva
/// una vez y se vuelve a pedir. Un 404 que sigue siéndolo es que no está.
///
/// Roturas que atrapa: «no encontrado» al abrir un adjunto tras un rato con
/// la app abierta; renovar en bucle por un fichero que de verdad no está;
/// dar la sesión por caducada porque la renovación no llegó.
#[tokio::test]
async fn a_file_refused_for_a_stale_token_is_asked_again_with_a_fresh_one() {
    let (server, core) = rig().await;
    let missing = ResponseTemplate::new(404).set_body_json(json!({ "success": false, "message": "Not found", "error": "not-found" }));
    Mock::given(method("GET"))
        .and(path(RAW))
        .and(header("Authorization", "Bearer fresh"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path(RAW)).respond_with(missing.clone()).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .and(header("Authorization", "Bearer refresh"))
        .respond_with(ok(json!({ "accessToken": "fresh", "refreshToken": "refresh-2" })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    assert_eq!(core.tasks().attachment_bytes("a1").await.unwrap(), b"ok");
    assert_eq!(asked(&server, "POST", "/api/v1/auth/refresh").await.len(), 1);

    // Uno que no está ni con el token nuevo: se dice, tras **una** renovación
    // (que aquí ya no contesta: la sesión se queda como estaba).
    Mock::given(method("GET")).and(path("/api/v1/tasks/t1/attachments/a-comment/raw")).respond_with(missing).mount(&server).await;
    let error = core.tasks().attachment_bytes("a-comment").await.unwrap_err();
    assert!(matches!(error, ApiError::Server { status: 404, .. }), "{error:?}");
    assert_eq!(asked(&server, "POST", "/api/v1/auth/refresh").await.len(), 2);
    assert_eq!(asked(&server, "GET", "/api/v1/tasks/t1/attachments/a-comment/raw").await.len(), 1);
    assert!(core.session().is_some(), "cerró la sesión por un fichero que no está");
}

/// Subir manda el fichero a la tarea abierta y la vuelve a pedir; uno que
/// pasa del tope no sale de la máquina.
///
/// Roturas que atrapa: subir a otra tarea; que el fichero subido no aparezca
/// hasta cerrar y abrir el cajón; mandar treinta megas para que el servidor
/// diga que no; un rechazo del servidor que no se cuenta.
#[tokio::test]
async fn attaching_sends_the_file_and_brings_the_task_again() {
    let (server, core) = rig().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/t1/attachments"))
        .and(header("Authorization", "Bearer access"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "success": true,
            "data": { "id": "a2", "itemId": "t1", "url": "/api/v1/tasks/t1/attachments/a2/raw", "fileName": "plan.pdf" },
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let huge = vec![0u8; MAX_ATTACHMENT_BYTES + 1];
    assert_eq!(code(core.tasks().attach("enorme.bin", huge).await.unwrap_err()), "too-large");
    assert!(asked(&server, "POST", "/api/v1/tasks/t1/attachments").await.is_empty(), "mandó un fichero que pasa del tope");

    let attached = core.tasks().attach("plan.pdf", b"%PDF-1.7".to_vec()).await.unwrap();
    // Lo que quedó guardado, tal como lo dice el servidor: con eso se cita.
    assert_eq!((attached.id.as_str(), attached.url.as_str()), ("a2", "/api/v1/tasks/t1/attachments/a2/raw"));
    assert_eq!(file_citation(&attached).as_deref(), Some("[plan.pdf](/api/v1/tasks/t1/attachments/a2/raw)"));

    let sent = asked(&server, "POST", "/api/v1/tasks/t1/attachments").await;
    assert_eq!(sent.len(), 1);
    let content_type = sent[0].headers.get("content-type").unwrap().to_str().unwrap().to_string();
    assert_eq!(sent[0].body, multipart("plan.pdf", b"%PDF-1.7").1);
    assert_eq!(content_type, multipart("plan.pdf", b"%PDF-1.7").0);
    assert_eq!(asked(&server, "GET", "/api/v1/tasks/t1").await.len(), 2, "no volvió a pedir la tarea");

    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/t1/attachments"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "success": false, "message": "Upload rejected", "error": "type not allowed" })))
        .mount(&server)
        .await;
    assert_eq!(code(core.tasks().attach("virus.exe", b"MZ".to_vec()).await.unwrap_err()), "type not allowed");
}

/// Quitar un adjunto lo pide por su id en la tarea abierta y la vuelve a
/// pedir.
///
/// Roturas que atrapa: quitar otro fichero; que siga en la lista tras
/// quitarlo; dar por quitado uno que el servidor no quitó.
#[tokio::test]
async fn removing_asks_for_that_file_and_brings_the_task_again() {
    let (server, core) = rig().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/t1/attachments/a1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "Attachment deleted" })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    core.tasks().remove_attachment("a1").await.unwrap();
    assert_eq!(asked(&server, "DELETE", "/api/v1/tasks/t1/attachments/a1").await.len(), 1);
    assert_eq!(asked(&server, "GET", "/api/v1/tasks/t1").await.len(), 2, "no volvió a pedir la tarea");

    // Ya no contesta: wiremock da 404.
    assert!(core.tasks().remove_attachment("a1").await.is_err());
}

/// Qué imagen es se mira en el fichero, no en su nombre ni en su tipo.
///
/// Roturas que atrapa: darle a quien decodifica un HTML que se llama
/// `foto.png`; no reconocer una captura que llegó sin tipo; tomar por WebP
/// cualquier RIFF (un WAV); reventar con un fichero de dos bytes.
#[test]
fn what_image_it_is_is_read_from_the_file() {
    assert_eq!(image_type(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Some("image/png"));
    assert_eq!(image_type(b"\xff\xd8\xff\xe0\0\x10JFIF"), Some("image/jpeg"));
    assert_eq!(image_type(b"GIF89a\x01\0"), Some("image/gif"));
    assert_eq!(image_type(b"GIF87a\x01\0"), Some("image/gif"));
    assert_eq!(image_type(b"RIFF\x24\0\0\0WEBPVP8 "), Some("image/webp"));
    assert_eq!(image_type(b"\0\0\0\x1cftypavif\0\0"), Some("image/avif"));
    assert_eq!(image_type(b"BM\x36\0\0\0"), Some("image/bmp"));

    assert_eq!(image_type(b"<!doctype html><script>"), None);
    assert_eq!(image_type(b"RIFF\x24\0\0\0WAVEfmt "), None);
    assert_eq!(image_type(b"\x89PNG"), None, "media firma no es un PNG");
    assert_eq!(image_type(b"GIF90a"), None);
    assert_eq!(image_type(b"RI"), None);
    assert_eq!(image_type(b""), None);
}

/// Una miniatura cabe entera en su hueco sin deformarse, y una imagen
/// pequeña no se amplía.
///
/// Roturas que atrapa: una captura ancha aplastada o recortada; una
/// alargada que se sale por abajo; un icono ampliado hasta ser una mancha;
/// una miniatura de cero píxeles, que no se ve ni se pulsa.
#[test]
fn a_thumbnail_fits_its_slot_without_being_deformed() {
    assert_eq!(thumbnail_size(1920, 1080), (256, 144));
    assert_eq!(thumbnail_size(1080, 1920), (90, 160));
    assert_eq!(thumbnail_size(512, 320), (256, 160));
    assert_eq!(thumbnail_size(256, 160), (256, 160));
    assert_eq!(thumbnail_size(16, 16), (16, 16));
    assert_eq!(thumbnail_size(300, 100), (256, 85));
    // Una tira finísima sigue teniendo dónde pulsarla.
    assert_eq!(thumbnail_size(10_000, 2), (256, 1));
    assert_eq!(thumbnail_size(2, 10_000), (1, 160));
    assert_eq!(thumbnail_size(0, 100), (0, 0));
}

/// Una imagen de la tarea se baja como cualquier fichero suyo, y dice qué
/// imagen es; lo que no lo es, o no vive en este servidor, no se enseña.
///
/// Roturas que atrapa: una miniatura pedida sin sesión (el servidor contesta
/// 404); dar por imagen lo que llegó y no lo es; mandar la sesión a la
/// dirección de un bucket por querer pintar una fila vieja.
#[tokio::test]
async fn a_picture_comes_through_the_same_door_as_any_file() {
    let (server, core) = rig().await;
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    Mock::given(method("GET"))
        .and(path(RAW))
        .and(header("Authorization", "Bearer access"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(png.clone()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/t1/attachments/a-comment/raw"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"<html>".to_vec()))
        .mount(&server)
        .await;

    assert_eq!(core.tasks().picture("a1").await.unwrap(), Picture { mime: "image/png".into(), bytes: png });
    assert_eq!(code(core.tasks().picture("a-comment").await.unwrap_err()), "not-an-image");

    let before = server.received_requests().await.unwrap().len();
    assert_eq!(code(core.tasks().picture("a-old").await.unwrap_err()), "not-an-attachment");
    assert_eq!(code(core.tasks().picture("a-otra-tarea").await.unwrap_err()), "attachment-not-found");
    assert_eq!(server.received_requests().await.unwrap().len(), before, "pidió algo");
}

fn stored(name: &str, url: &str) -> TaskAttachment {
    serde_json::from_value(json!({ "id": "a9", "itemId": "t1", "url": url, "fileName": name })).unwrap()
}

/// Un fichero se cita como lo que es, por su ruta en este servidor.
///
/// Roturas que atrapa: una captura citada como un enlace que hay que abrir,
/// o un PDF como una imagen rota; un nombre con corchetes que corta el
/// enlace; guardar en el texto la dirección entera del servidor (no se lee
/// desde otro) o la de un bucket.
#[test]
fn a_file_is_cited_as_what_it_is() {
    let raw = "/api/v1/tasks/t1/attachments/a9/raw";
    assert_eq!(file_citation(&stored("captura.png", raw)).unwrap(), format!("![captura.png]({raw})"));
    assert_eq!(file_citation(&stored("plan.pdf", raw)).unwrap(), format!("[plan.pdf]({raw})"));
    assert_eq!(file_citation(&stored("acta [v2]\\final\n.pdf", raw)).unwrap(), format!("[acta \\[v2\\]\\\\final .pdf]({raw})"));

    assert_eq!(file_citation(&stored("x.png", "https://bucket.s3.us-east-1.amazonaws.com/tasks/t1/x.png")), None);
    assert_eq!(file_citation(&stored("x.png", "")), None);
}

/// La cita va donde está el cursor, y el cursor queda detrás.
///
/// Roturas que atrapa: mandar la cita al final aunque se escribía en medio;
/// pegarla a la palabra de antes; una imagen metida a mitad de una frase;
/// comerse lo que había después; dejar el cursor delante de la cita (el
/// segundo fichero cae antes que el primero); reventar con un cursor que ya
/// no es de este texto.
#[test]
fn a_citation_goes_where_the_cursor_is() {
    let raw = "/api/v1/tasks/t1/attachments/a9/raw";
    let (pdf, shot) = (stored("plan.pdf", raw), stored("captura.png", raw));
    let link = format!("[plan.pdf]({raw})");
    let image = format!("![captura.png]({raw})");
    let cited = |text: &str, cursor: usize, file: &TaskAttachment| {
        let Cited { text, cursor } = with_citation(text, cursor, file).unwrap();
        (text.clone(), text[..cursor as usize].to_string())
    };

    assert_eq!(cited("", 0, &pdf), (format!("{link} "), format!("{link} ")));
    assert_eq!(cited("mira esto", 4, &pdf), (format!("mira {link}  esto"), format!("mira {link} ")));
    assert_eq!(cited("mira", 4, &pdf), (format!("mira {link} "), format!("mira {link} ")));
    assert_eq!(cited("año", 4, &pdf).0, format!("año {link} "));

    assert_eq!(cited("", 0, &shot), (format!("{image}\n"), format!("{image}\n")));
    assert_eq!(cited("mira esto", 4, &shot), (format!("mira\n{image}\n esto"), format!("mira\n{image}\n")));
    assert_eq!(cited("uno\n", 4, &shot).0, format!("uno\n{image}\n"));

    // El cursor cambió mientras subía: al final, sin romper una letra.
    assert_eq!(cited("año", 2, &pdf).0, format!("año {link} "));
    assert_eq!(cited("mira", 99, &pdf).0, format!("mira {link} "));
    assert_eq!(with_citation("x", 1, &stored("x.png", "https://bucket.example/x.png")), None);
}

/// Un enlace a un fichero de la tarea abierta se reconoce, escrito por su
/// ruta o con la dirección entera de este servidor; cualquier otro, no.
///
/// Roturas que atrapa: darle al navegador una dirección que pide sesión (no
/// abre nada); tomar por fichero de la tarea el de otra, o un enlace de
/// fuera que acaba igual; no reconocer lo que escribió la app de siempre.
#[tokio::test]
async fn a_link_to_a_file_of_this_task_is_known() {
    let (server, core) = rig().await;
    let linked = |href: &str| core.tasks().linked_attachment(href).map(|file| file.id);

    assert_eq!(linked(RAW).as_deref(), Some("a1"));
    assert_eq!(linked(&format!("{}{RAW}", server.uri())).as_deref(), Some("a1"));
    assert_eq!(linked("/api/v1/tasks/t1/attachments/a-comment/raw").as_deref(), Some("a-comment"));

    assert_eq!(linked("/api/v1/tasks/t9/attachments/a1/raw"), None);
    assert_eq!(linked(&format!("https://evil.example{RAW}")), None);
    assert_eq!(linked(&format!("{}.evil.example{RAW}", server.uri())), None);
    assert_eq!(linked("https://bucket.s3.us-east-1.amazonaws.com/tasks/t1/old.png"), None, "la fila vieja no vive aquí");
    assert_eq!(linked("https://cac.guz-studio.dev/docs"), None);
    core.tasks().close();
    assert_eq!(linked(RAW), None);
}
