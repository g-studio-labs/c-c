//! Una sola app abierta por persona.
//!
//! La primera que arranca escucha en un socket local; la que arranca
//! después (`cac-desktop`, o `cac-desktop <enlace>` cuando el sistema abre un
//! enlace de cac) le pasa su enlace —o sólo «al frente»— y se cierra. Así no
//! hay dos ventanas con la misma sesión peleándose por ella.
//!
//! Lo que guarda la puerta, porque quien hable por el socket mueve la app de
//! alguien:
//!
//! - **Sólo el mismo usuario.** El socket vive en una carpeta propia `0700`
//!   (en `XDG_RUNTIME_DIR`, que ya es sólo de uno, o en la carpeta de datos)
//!   y es `0600`. Una carpeta que no es de uno o que otros pueden abrir no se
//!   usa. Y aunque llegue alguien, la primera instancia pregunta al sistema
//!   quién está al otro lado (`SO_PEERCRED`, `getpeereid`) y cuelga a
//!   cualquier otro usuario sin leerle nada.
//! - **Un socket viejo no bloquea.** Si la app murió sin borrarlo, conectar
//!   falla; entonces se borra —sólo si es un socket y es de uno— y se escucha
//!   de nuevo.
//! - **Dos que arrancan a la vez no se quedan las dos.** Mirar si hay
//!   alguien, borrar el socket viejo y escuchar se hace con un cerrojo
//!   (`flock`) de la carpeta: sin él, la segunda borraba el socket recién
//!   creado por la primera y quedaban dos ventanas (un doble clic en el
//!   lanzador bastaba).
//! - **Lo que entra se lee poco y con desconfianza**: una línea, como mucho
//!   [`MAX_LINE`] bytes, y sólo `focus` u `open <enlace>`. El enlace pasa por
//!   el traductor del core como cualquier otro: lo que no es de cac va al
//!   navegador, y lo que no es un enlace no va a ningún sitio.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SOCKET: &str = "instance.sock";

/// El cerrojo del reclamo (ver [`claim`]).
const LOCK: &str = "instance.lock";

/// Lo más que se lee de quien llama. Un enlace de cac cabe de sobra.
const MAX_LINE: usize = 8 * 1024;

/// Cuánto se espera a la otra punta. No mide nada: está para que una
/// instancia colgada no deje colgada a la que arranca.
const PATIENCE: Duration = Duration::from_secs(5);

/// Lo que la segunda instancia le pide a la primera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    /// Sólo traer la ventana al frente.
    Focus,
    /// Traerla y abrir ese enlace.
    Open(String),
}

impl Message {
    /// Lo que se le pide a la abierta: abrir el enlace con el que se lanzó
    /// ésta (`links::launch_link`), o sólo «al frente».
    pub(crate) fn for_launch(link: Option<String>) -> Message {
        link.map_or(Message::Focus, Message::Open)
    }

    /// Una línea. Un enlace con un salto de línea dentro no es un enlace:
    /// se queda en «al frente».
    fn encode(&self) -> String {
        match self {
            Message::Open(link) if !link.contains(['\n', '\r']) && link.len() < MAX_LINE - 8 => format!("open {link}\n"),
            _ => "focus\n".to_string(),
        }
    }

    fn decode(line: &str) -> Option<Message> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        if line == "focus" {
            return Some(Message::Focus);
        }
        let link = line.strip_prefix("open ")?;
        (!link.trim().is_empty() && !link.contains(['\n', '\r'])).then(|| Message::Open(link.to_string()))
    }
}

/// Dónde vive el socket: en `XDG_RUNTIME_DIR` si el sistema lo da (es de
/// uno solo y se vacía al cerrar la sesión), y si no, en la carpeta de datos
/// de la app.
pub(crate) fn socket_dir(var: impl Fn(&str) -> Option<String>, data_dir: &Path) -> PathBuf {
    match var("XDG_RUNTIME_DIR").filter(|dir| !dir.trim().is_empty()) {
        Some(runtime) => PathBuf::from(runtime).join(crate::system::APP_DIR),
        None => data_dir.join("run"),
    }
}

/// Quién es esta persona para el sistema.
pub(crate) fn my_uid() -> u32 {
    // SAFETY: `getuid` no falla ni toca memoria.
    unsafe { libc::getuid() }
}

/// Quién está al otro lado de un socket.
fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let fd = stream.as_raw_fd();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut credentials = libc::ucred { pid: 0, uid: 0, gid: 0 };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `credentials` y `size` viven durante la llamada y `size` es
        // el tamaño de lo que se le da.
        let failed = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut credentials as *mut libc::ucred).cast(), &mut size) } != 0;
        if failed {
            return Err(io::Error::last_os_error());
        }
        Ok(credentials.uid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: los dos punteros son a variables vivas.
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(uid)
    }
}

fn refused(what: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, what)
}

/// La carpeta del socket: se crea `0700` si no está, y si está tiene que
/// ser de uno y no dejar entrar a nadie más.
fn prepare_dir(dir: &Path, me: u32) -> io::Result<()> {
    if std::fs::symlink_metadata(dir).is_err() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != me || meta.mode() & 0o077 != 0 {
        return Err(refused(format!("{} no es una carpeta sólo de este usuario", dir.display())));
    }
    Ok(())
}

/// Lo que pasó al llamar a la que ya estaba.
enum Call {
    /// Lo recibió.
    Delivered,
    /// No hay nadie escuchando (no hay socket, o es de una app que murió).
    Nobody,
}

fn call(path: &Path, me: u32, message: &Message) -> io::Result<Call> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Call::Nobody),
        Err(error) => return Err(error),
    };
    if !meta.file_type().is_socket() || meta.uid() != me {
        return Err(refused(format!("{} no es un socket de este usuario", path.display())));
    }
    let mut stream = match UnixStream::connect(path) {
        Ok(stream) => stream,
        Err(error) if matches!(error.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound) => return Ok(Call::Nobody),
        Err(error) => return Err(error),
    };
    if peer_uid(&stream)? != me {
        return Err(refused("escucha otro usuario".into()));
    }
    stream.set_read_timeout(Some(PATIENCE))?;
    stream.set_write_timeout(Some(PATIENCE))?;
    stream.write_all(message.encode().as_bytes())?;
    let mut answer = String::new();
    BufReader::new(stream.take(16)).read_line(&mut answer)?;
    if answer == "ok\n" {
        Ok(Call::Delivered)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, "la app abierta no contestó"))
    }
}

/// Quién se queda con la ventana.
pub(crate) enum Claim {
    /// Ésta es la primera: escucha aquí.
    First(UnixListener),
    /// Ya había una, y tiene el mensaje: ésta se cierra.
    Delivered,
}

/// Un cerrojo exclusivo sobre el fichero de la carpeta; se suelta al
/// soltar el `File`.
fn lock(dir: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(dir.join(LOCK))?;
    loop {
        // SAFETY: el descriptor es de `file`, que vive durante la llamada.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(file);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// Le pasa el mensaje a la instancia abierta, o se queda con el socket si
/// no hay ninguna. `me` es quien arranca ([`my_uid`]).
///
/// Todo bajo el cerrojo: quien lo tiene después de otra que ya escucha la
/// encuentra (su socket ya está en disco, y conectar entra en la cola
/// aunque aún no atienda), y nunca borra un socket vivo.
pub(crate) fn claim(dir: &Path, me: u32, message: &Message) -> io::Result<Claim> {
    prepare_dir(dir, me)?;
    let _held = lock(dir)?;
    let path = dir.join(SOCKET);
    if let Call::Delivered = call(&path, me, message)? {
        return Ok(Claim::Delivered);
    }
    // Nadie escucha: lo que quede es de una app que murió. `call` ya
    // comprobó que es un socket de uno.
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(Claim::First(listener))
}

/// Lee lo que pide quien llama, si es este mismo usuario.
fn answer(mut stream: UnixStream, me: u32) -> Option<Message> {
    if peer_uid(&stream).ok()? != me {
        return None;
    }
    stream.set_read_timeout(Some(PATIENCE)).ok()?;
    stream.set_write_timeout(Some(PATIENCE)).ok()?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_LINE as u64)).read_line(&mut line).ok()?;
    // Sin el salto de línea, se cortó en el límite: no es un mensaje entero.
    if !line.ends_with('\n') {
        return None;
    }
    let message = Message::decode(&line)?;
    stream.write_all(b"ok\n").ok()?;
    Some(message)
}

/// Atiende el socket en un hilo aparte, un mensaje cada vez, y le da cada
/// uno a `arrived`. Quien no es `me` se cuelga sin leerle nada.
pub(crate) fn serve(listener: UnixListener, me: u32, arrived: impl Fn(Message) + Send + 'static) {
    std::thread::Builder::new()
        .name("cac-instance".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if let Some(message) = answer(stream, me) {
                    arrived(message);
                }
            }
        })
        .expect("no se pudo arrancar el hilo de la instancia única");
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    /// Una carpeta nueva para cada prueba, corta: la ruta de un socket no
    /// puede pasar de ~100 bytes.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-si-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("run")
    }

    fn first(dir: &Path, me: u32) -> UnixListener {
        match claim(dir, me, &Message::Focus).unwrap() {
            Claim::First(listener) => listener,
            Claim::Delivered => panic!("no había ninguna abierta"),
        }
    }

    /// La segunda le entrega su enlace a la primera y se va; y lo que queda
    /// en disco no deja entrar a nadie más.
    ///
    /// Roturas que atrapa: dos ventanas (la segunda no encuentra a la
    /// primera, o arranca igual); perder el enlace por el camino; un socket
    /// o una carpeta que otros pueden abrir.
    #[test]
    fn the_second_launch_hands_its_link_to_the_first_and_leaves() {
        let dir = scratch("hand");
        let me = my_uid();
        let listener = first(&dir, me);
        let (sender, received) = mpsc::channel();
        serve(listener, me, move |message| sender.send(message).unwrap());

        let link = "cac://tasks?task=command-and-control-12";
        assert!(matches!(claim(&dir, me, &Message::Open(link.into())).unwrap(), Claim::Delivered));
        assert_eq!(received.recv_timeout(PATIENCE).unwrap(), Message::Open(link.into()));
        assert!(matches!(claim(&dir, me, &Message::Focus).unwrap(), Claim::Delivered));
        assert_eq!(received.recv_timeout(PATIENCE).unwrap(), Message::Focus);

        assert_eq!(std::fs::metadata(dir.join(SOCKET)).unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// Una app que murió deja su socket; la siguiente arranca igual.
    ///
    /// Rotura que atrapa: un socket viejo que impide abrir la app (o que
    /// la hace esperar a nadie).
    #[test]
    fn a_socket_left_by_a_dead_app_does_not_block_the_start() {
        let dir = scratch("stale");
        let me = my_uid();
        drop(first(&dir, me));
        assert!(dir.join(SOCKET).exists(), "el socket se queda en disco");
        drop(first(&dir, me));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// Otro usuario no puede usar el socket: la abierta lo cuelga sin
    /// leerle nada, y una carpeta o un socket de otro no se usan.
    ///
    /// Se finge «otro» cambiando a quién se espera, que es lo único que una
    /// prueba sin root puede cambiar. Roturas que atrapa: atender a quien
    /// no es el dueño; fiarse de una carpeta de otro, o de una que todos
    /// pueden abrir; borrar o usar el socket de otro.
    #[test]
    fn nobody_else_can_use_the_socket() {
        let me = my_uid();
        let someone_else = me.wrapping_add(1);

        let dir = scratch("peer");
        let (sender, received) = mpsc::channel();
        serve(first(&dir, me), someone_else, move |message| sender.send(message).unwrap());
        assert!(claim(&dir, me, &Message::Open("cac://tasks?task=1".into())).is_err(), "colgado, no le contesta");
        assert!(received.recv_timeout(Duration::from_millis(300)).is_err(), "y no le lee nada");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());

        // Vacía: sin socket dentro que delate al dueño, sólo la carpeta lo dice.
        let dir = scratch("owner");
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).unwrap();
        assert!(claim(&dir, someone_else, &Message::Focus).is_err(), "la carpeta es de otro");
        assert!(!dir.join(SOCKET).exists(), "y no se escucha en ella");
        drop(first(&dir, me));
        assert!(claim(&dir, someone_else, &Message::Focus).is_err(), "ni con el socket de otro dentro");
        assert!(dir.join(SOCKET).exists(), "y su socket no se toca");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(claim(&dir, me, &Message::Focus).is_err(), "una carpeta que otros pueden abrir no se usa");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// Dos que arrancan a la vez: una se queda escuchando y la otra le
    /// entrega su enlace. Cientos de rondas, la mitad con el socket viejo
    /// de la ronda anterior en disco y la otra mitad sin nada.
    ///
    /// No se cronometra nada: la ganadora atiende hasta que las dos han
    /// dicho qué les tocó, y el veredicto es la cuenta. Rotura que atrapa:
    /// reclamar sin cerrojo, con lo que la segunda borraba el socket recién
    /// creado por la primera y escuchaban las dos (en la caja, hacia la
    /// ronda veinte).
    #[test]
    fn two_launches_at_once_leave_exactly_one_listening() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Barrier};

        let dir = scratch("race");
        let me = my_uid();
        prepare_dir(&dir, me).unwrap();
        for round in 0..400 {
            if round % 2 == 0 {
                let _ = std::fs::remove_file(dir.join(SOCKET));
            }
            let start = Arc::new(Barrier::new(2));
            let stop = Arc::new(AtomicBool::new(false));
            let (told, outcomes) = mpsc::channel();
            let launches: Vec<_> = (0..2)
                .map(|n| {
                    let (dir, start, stop, told) = (dir.clone(), start.clone(), stop.clone(), told.clone());
                    std::thread::spawn(move || {
                        start.wait();
                        match claim(&dir, me, &Message::Open(format!("cac://tasks?task={n}"))) {
                            Ok(Claim::First(listener)) => {
                                told.send("first").unwrap();
                                listener.set_nonblocking(true).unwrap();
                                while !stop.load(Ordering::SeqCst) {
                                    match listener.accept() {
                                        Ok((stream, _)) => {
                                            stream.set_nonblocking(false).unwrap();
                                            answer(stream, me);
                                        }
                                        Err(_) => std::thread::yield_now(),
                                    }
                                }
                            }
                            Ok(Claim::Delivered) => told.send("delivered").unwrap(),
                            Err(error) => told.send(if error.kind() == io::ErrorKind::AddrInUse { "addr-in-use" } else { "error" }).unwrap(),
                        }
                    })
                })
                .collect();
            let mut seen = [outcomes.recv().unwrap(), outcomes.recv().unwrap()];
            stop.store(true, Ordering::SeqCst);
            for launch in launches {
                launch.join().unwrap();
            }
            seen.sort();
            assert_eq!(seen, ["delivered", "first"], "ronda {round}");
        }
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// Un enlace demasiado largo no se manda, y una línea enorme por el
    /// socket no se lee entera ni se toma por un mensaje.
    ///
    /// Roturas que atrapa: mandar cualquier longitud (`encode` sin su
    /// condición); leer sin límite (`take` quitado); aceptar como enlace la
    /// línea cortada en el límite.
    #[test]
    fn a_line_is_read_only_up_to_its_limit() {
        let near = Message::Open(format!("cac://{}", "a".repeat(MAX_LINE - 15)));
        assert!(near.encode().starts_with("open "), "lo que cabe viaja");
        assert!(near.encode().len() <= MAX_LINE);
        let long = Message::Open(format!("cac://{}", "a".repeat(MAX_LINE)));
        assert_eq!(long.encode(), "focus\n", "lo que no cabe se queda en «al frente»");

        let dir = scratch("long");
        let me = my_uid();
        let (sender, received) = mpsc::channel();
        serve(first(&dir, me), me, move |message| sender.send(message).unwrap());
        let mut raw = UnixStream::connect(dir.join(SOCKET)).unwrap();
        let _ = raw.write_all(format!("open cac://{}\n", "a".repeat(4 * MAX_LINE)).as_bytes());
        let mut answered = String::new();
        let _ = raw.read_to_string(&mut answered);
        assert_eq!(answered, "", "no se le contesta");
        // La abierta atiende de una en una: lo siguiente que llega dice si
        // la línea enorme pasó.
        assert!(matches!(claim(&dir, me, &Message::Focus).unwrap(), Claim::Delivered));
        assert_eq!(received.recv_timeout(PATIENCE).unwrap(), Message::Focus);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// Lo que viaja: una línea, y sólo lo que se sabe pedir.
    ///
    /// Roturas que atrapa: que un salto de línea dentro de un enlace cuele
    /// una segunda orden; aceptar cualquier cosa como enlace; perder el
    /// enlace con el que se lanzó.
    #[test]
    fn only_a_focus_or_a_link_travels() {
        assert_eq!(Message::for_launch(Some("cac://dm?c=c1".into())), Message::Open("cac://dm?c=c1".into()));
        assert_eq!(Message::for_launch(None), Message::Focus);
        let sneaky = Message::Open("cac://x\nopen https://ejemplo.com".into());
        assert_eq!(sneaky.encode(), "focus\n");
        let link = Message::Open("cac://tasks?task=1".into());
        assert_eq!(Message::decode(&link.encode()), Some(link));
        for junk in ["", "open ", "abrir cac://x", "open\tcac://x", "focus please"] {
            assert_eq!(Message::decode(junk), None, "{junk:?}");
        }
    }
}
