//! Cómo se entera la pantalla de que algo cambió.
//!
//! El core no sabe qué pinta: avisa por este trait, y cada plataforma lo
//! convierte en lo suyo (un evento de Tauri, un `StateFlow`, un `@Published`).

use crate::comms::CommsState;
use crate::counters::CountersState;
use crate::docs::DocsState;
use crate::mywork::MyWorkState;
use crate::orgs::OrgsState;
use crate::tree::TreeState;
use crate::session::Session;
use crate::stream::{ConnectionState, ServerEvent};
use crate::task::TaskState;

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

    /// La conexión con el servidor cambió: para el aviso de «sin conexión».
    fn connection_changed(&self, _state: ConnectionState) {}

    /// Llegó un evento del servidor. Los de tarea ya pusieron al día «Mi
    /// trabajo» y la tarea abierta; llegan aquí igualmente, con todos los
    /// demás, para lo que es de cada plataforma (un aviso, un sonido).
    fn server_event(&self, _event: ServerEvent) {}
}

/// Para quien no necesita enterarse (el MCP, una prueba).
pub struct NoListener;

impl Listener for NoListener {
    fn session_changed(&self, _session: Option<Session>) {}
}
