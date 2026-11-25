//! Cómo se entera la pantalla de que algo cambió.
//!
//! El core no sabe qué pinta: avisa por este trait, y cada plataforma lo
//! convierte en lo suyo (un evento de Tauri, un `StateFlow`, un `@Published`).

use crate::chat::ThreadState;
use crate::comms::CommsState;
use crate::counters::CountersState;
use crate::docs::DocsState;
use crate::inbox::InboxState;
use crate::mywork::MyWorkState;
use crate::notice::Announcement;
use crate::orgs::OrgsState;
use crate::platform::PlatformState;
use crate::requests::RequestsState;
use crate::tree::TreeState;
use crate::session::Session;
use crate::stream::{ConnectionState, ServerEvent};
use crate::task::TaskState;
use crate::voice::VoiceState;

pub trait Listener: Send + Sync {
    /// La sesión cambió: alguien entró, salió, o el servidor la dio por
    /// terminada. `None` es «no hay nadie dentro».
    fn session_changed(&self, session: Option<Session>);

    /// «Mi trabajo» cambió: llegó la lista, se movió la lente, se quitó una
    /// fila. Va el estado entero, no la diferencia: la pantalla lo pinta tal
    /// cual y no tiene nada que reconstruir.
    fn my_work_changed(&self, _state: MyWorkState) {}

    /// Llegaron las organizaciones, o se cambió de una a otra.
    fn orgs_changed(&self, _state: OrgsState) {}

    /// Llegó el árbol de espacios, o se vació al cambiar de organización.
    fn tree_changed(&self, _state: TreeState) {}

    /// La documentación abierta cambió: se abrió, llegó, se cambió de
    /// pestaña o se cerró.
    fn docs_changed(&self, _state: DocsState) {}

    /// La tarea abierta cambió: se abrió, llegó, se editó o se cerró.
    fn task_changed(&self, _state: TaskState) {}

    /// Un contador de la campana o del riel se movió, o se cambió de
    /// organización y hay que dejar de pintar los de la anterior.
    fn counters_changed(&self, _state: CountersState) {}

    /// La lista de comunicación cambió: llegó un canal o un directo, se
    /// movió una fila, se eligió una conversación, o se cambió de
    /// organización y hay que dejar de pintar la de la anterior.
    fn comms_changed(&self, _state: CommsState) {}

    /// La conversación que se tiene delante cambió: se eligió otra, llegó
    /// su página, llegó un mensaje, o el otro leyó.
    fn thread_changed(&self, _state: ThreadState) {}

    /// La bandeja de la campana cambió: llegó, se leyó algo, o se cambió de
    /// organización.
    fn inbox_changed(&self, _state: InboxState) {}

    /// «Por atender» cambió: llegaron los pedidos, se contestó uno, se movió
    /// el número o las marcas «espera a…», o se entró o se salió de ella.
    fn requests_changed(&self, _state: RequestsState) {}

    /// Un agente pidió que la persona mire ese pedido ya (`request:open`):
    /// «Por atender» ya está delante con él resaltado; la plataforma pone su
    /// ventana al frente.
    fn show_request(&self, _code: String) {}

    /// La voz cambió: la sala en la que estás, quién habla, el timbre, o
    /// quién anda por los canales.
    fn voice_changed(&self, _state: VoiceState) {}

    /// Plataforma cambió: llegaron los servidores, se entró en uno, o se
    /// cambió de organización.
    fn platform_changed(&self, _state: PlatformState) {}

    /// La conexión con el servidor cambió: para el aviso de «sin conexión».
    fn connection_changed(&self, _state: ConnectionState) {}

    /// Llegó un evento del servidor. Los de tarea ya pusieron al día «Mi
    /// trabajo» y la tarea abierta; llegan aquí igualmente, con todos los
    /// demás, para lo que es de cada plataforma (un aviso, un sonido).
    fn server_event(&self, _event: ServerEvent) {}

    /// Hay que avisar de un mensaje: el aviso del sistema y su sonido, que
    /// son de cada plataforma. Ya se decidió todo (`notice.rs`): si es nuevo,
    /// si se sigue, si no se tiene delante; aquí sólo se enseña.
    fn announce(&self, _announcement: Announcement) {}
}

/// Para quien no necesita enterarse (el MCP, una prueba).
pub struct NoListener;

impl Listener for NoListener {
    fn session_changed(&self, _session: Option<Session>) {}
}
