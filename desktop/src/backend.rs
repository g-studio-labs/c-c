//! El core dentro de la app: dónde corre y cómo llega a la pantalla.
//!
//! Dos mundos que no se mezclan solos. El core habla con el servidor sobre
//! tokio y avisa desde sus hilos; GPUI pinta en el hilo principal con su
//! propio ejecutor y no deja tocar la pantalla desde otro sitio. Aquí está el
//! puente, y sólo aquí:
//!
//! - **hacia el core**, todo va por [`Backend::run`], que lo ejecuta en un
//!   runtime de tokio de la app;
//! - **hacia la pantalla**, cada aviso del core cruza por un canal al hilo
//!   principal y acaba en un [`Snapshot`], que es lo único que las vistas leen.
//!
//! Las vistas no guardan estado del core ni deciden nada: observan el
//! `Snapshot` y pintan lo que diga.

use std::future::Future;
use std::sync::Arc;

use cac_core::{
    Announcement, ApiError, CommsState, InboxState, Config, ConnectionState, Core, CountersState, DocsState, Listener, MyWorkState, OrgsState, OverviewState, PlatformState, RequestsState, Session, Storage,
    TaskState, ThreadState, TreeState, VoiceState, WallState,
};
use gpui_kit::{App, AppContext, Entity, Global};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

/// Un aviso del core, de camino al hilo principal.
enum CoreEvent {
    Session(Option<Session>),
    MyWork(MyWorkState),
    Orgs(OrgsState),
    Tree(TreeState),
    Docs(DocsState),
    Task(TaskState),
    Counters(CountersState),
    Comms(CommsState),
    Thread(ThreadState),
    Connection(ConnectionState),
    Inbox(InboxState),
    Voice(VoiceState),
    Platform(PlatformState),
    Requests(RequestsState),
    Wall(WallState),
    Overview(OverviewState),
    /// No es estado: un agente pidió que se mire un pedido ya.
    ShowRequest,
    /// No es estado: un aviso del sistema que enseñar (`notifier`).
    Announce(Announcement),
}

/// Lo que el core oye, reenviado tal cual. No hace nada más a propósito:
/// corre en un hilo de tokio, donde no se puede tocar nada de GPUI.
struct Forwarder(UnboundedSender<CoreEvent>);

impl Forwarder {
    fn send(&self, event: CoreEvent) {
        // Sin nadie al otro lado la app se está cerrando: no hay a quién avisar.
        let _ = self.0.send(event);
    }
}

impl Listener for Forwarder {
    fn session_changed(&self, session: Option<Session>) {
        self.send(CoreEvent::Session(session));
    }
    fn my_work_changed(&self, state: MyWorkState) {
        self.send(CoreEvent::MyWork(state));
    }
    fn orgs_changed(&self, state: OrgsState) {
        self.send(CoreEvent::Orgs(state));
    }
    fn tree_changed(&self, state: TreeState) {
        self.send(CoreEvent::Tree(state));
    }
    fn docs_changed(&self, state: DocsState) {
        self.send(CoreEvent::Docs(state));
    }
    fn task_changed(&self, state: TaskState) {
        self.send(CoreEvent::Task(state));
    }
    fn counters_changed(&self, state: CountersState) {
        self.send(CoreEvent::Counters(state));
    }
    fn comms_changed(&self, state: CommsState) {
        self.send(CoreEvent::Comms(state));
    }
    fn platform_changed(&self, state: PlatformState) {
        self.send(CoreEvent::Platform(state));
    }
    fn thread_changed(&self, state: ThreadState) {
        self.send(CoreEvent::Thread(state));
    }
    fn connection_changed(&self, state: ConnectionState) {
        self.send(CoreEvent::Connection(state));
    }
    fn inbox_changed(&self, state: InboxState) {
        self.send(CoreEvent::Inbox(state));
    }
    fn voice_changed(&self, state: VoiceState) {
        self.send(CoreEvent::Voice(state));
    }
    fn announce(&self, announcement: Announcement) {
        self.send(CoreEvent::Announce(announcement));
    }
    fn requests_changed(&self, state: RequestsState) {
        self.send(CoreEvent::Requests(state));
    }
    fn wall_changed(&self, state: WallState) {
        self.send(CoreEvent::Wall(state));
    }
    fn overview_changed(&self, state: OverviewState) {
        self.send(CoreEvent::Overview(state));
    }
    fn show_request(&self, _code: String) {
        self.send(CoreEvent::ShowRequest);
    }
}

/// El estado del core tal como lo ve la pantalla, entero y ya en el hilo
/// principal.
#[derive(Default)]
pub struct Snapshot {
    pub session: Option<Session>,
    pub my_work: MyWorkState,
    pub orgs: OrgsState,
    pub tree: TreeState,
    pub docs: DocsState,
    pub task: TaskState,
    /// Los números de la campana y del riel.
    pub counters: CountersState,
    /// La lista de comunicación de la organización que se mira.
    pub comms: CommsState,
    /// La conversación que se tiene delante, con sus mensajes.
    pub thread: ThreadState,
    /// Cuántas veces ha cambiado: quien la pinta lleva su lista al día
    /// cuando cambia, y no en cada aviso de cualquier otra cosa.
    pub thread_revision: u64,
    pub connection: ConnectionState,
    /// La bandeja de la campana, desde que se abrió el panel.
    pub inbox: InboxState,
    /// La voz: la sala en la que estás, el timbre y quién anda por los
    /// canales.
    pub voice: VoiceState,
    /// Plataforma: los servidores de la organización que se mira, y si se
    /// está en ella.
    pub platform: PlatformState,
    /// «Por atender»: los pedidos de los agentes, su número y las marcas
    /// «espera a…» de las tareas.
    pub requests: RequestsState,
    /// El muro de feedback.
    pub wall: WallState,
    /// Lo que lee el resumen; lo demás lo toma de comunicación y Plataforma.
    pub overview: OverviewState,
}

impl Snapshot {
    fn apply(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::Session(session) => self.session = session,
            CoreEvent::MyWork(state) => self.my_work = state,
            CoreEvent::Orgs(state) => self.orgs = state,
            CoreEvent::Tree(state) => self.tree = state,
            CoreEvent::Docs(state) => self.docs = state,
            CoreEvent::Task(state) => self.task = state,
            CoreEvent::Counters(state) => self.counters = state,
            CoreEvent::Comms(state) => self.comms = state,
            CoreEvent::Thread(state) => {
                self.thread = state;
                self.thread_revision += 1;
            }
            CoreEvent::Connection(state) => self.connection = state,
            CoreEvent::Inbox(state) => self.inbox = state,
            CoreEvent::Voice(state) => self.voice = state,
            CoreEvent::Platform(state) => self.platform = state,
            CoreEvent::Requests(state) => self.requests = state,
            CoreEvent::Wall(state) => self.wall = state,
            CoreEvent::Overview(state) => self.overview = state,
            // No llegan aquí: no son estado.
            CoreEvent::Announce(_) | CoreEvent::ShowRequest => {}
        }
    }
}

/// El core de esta app. Uno por proceso, global de GPUI.
pub struct Backend {
    core: Core,
    runtime: Arc<tokio::runtime::Runtime>,
    snapshot: Entity<Snapshot>,
}

impl Global for Backend {}

impl Backend {
    /// Arranca el core con lo que hubiera guardado y lo deja como global.
    pub fn install(config: Config, storage: Arc<dyn Storage>, cx: &mut App) -> Result<(), ApiError> {
        let (sender, mut receiver) = unbounded_channel();
        let core = Core::new(config, storage, Arc::new(Forwarder(sender)))?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("cac-core")
            .enable_all()
            .build()
            .map_err(|error| ApiError::Network(error.to_string()))?;

        // Lo que el core ya sabe al arrancar (la sesión y las preferencias
        // guardadas) no lo anuncia: se lee una vez aquí.
        let snapshot = cx.new(|_| Snapshot {
            session: core.session(),
            my_work: core.my_work().state(),
            orgs: core.orgs().state(),
            voice: core.voice().state(),
            ..Snapshot::default()
        });

        crate::notifier::install(cx);
        let target = snapshot.clone();
        cx.spawn(async move |cx| {
            while let Some(event) = receiver.recv().await {
                cx.update(|cx| match event {
                    CoreEvent::Announce(announcement) => crate::notifier::show(announcement, cx),
                    CoreEvent::ShowRequest => crate::notifier::bring_to_front(cx),
                    event => target.update(cx, |snapshot, cx| {
                        snapshot.apply(event);
                        cx.notify();
                    }),
                });
            }
        })
        .detach();

        cx.set_global(Backend { core, runtime: Arc::new(runtime), snapshot });
        Ok(())
    }

    /// El estado que las vistas observan.
    pub fn snapshot(cx: &App) -> Entity<Snapshot> {
        cx.global::<Backend>().snapshot.clone()
    }

    /// Para lo que el core contesta sin esperar (una regla, un filtro).
    pub fn core(cx: &App) -> Core {
        cx.global::<Backend>().core.clone()
    }

    /// Para lo que el core arranca por su cuenta desde una llamada que no
    /// espera (empezar a preguntar cada poco quién anda por la voz): se le
    /// pide dentro del runtime, que es donde viven sus tareas.
    pub fn within<T>(cx: &App, work: impl FnOnce(Core) -> T) -> T {
        let backend = cx.global::<Backend>();
        let _inside = backend.runtime.enter();
        work(backend.core.clone())
    }

    /// Le pide algo al core que tarda (la red). Corre en el runtime de la
    /// app; el resultado no vuelve por aquí: llega como estado al `Snapshot`.
    pub fn run<F, Fut>(cx: &App, work: F)
    where
        F: FnOnce(Core) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let backend = cx.global::<Backend>();
        // En el runtime de tokio y no en el ejecutor de GPUI: el core lanza
        // tareas propias (`tokio::spawn`) y pide por la red con reqwest, y las
        // dos cosas sólo existen ahí dentro.
        backend.runtime.spawn(work(backend.core.clone()));
    }

    /// Lo mismo, para quien necesita la respuesta (un formulario que tiene
    /// que decir por qué falló). La respuesta se espera desde la pantalla.
    pub fn ask<F, Fut, T>(cx: &App, work: F) -> tokio::sync::oneshot::Receiver<T>
    where
        F: FnOnce(Core) -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let (answer, receiver) = tokio::sync::oneshot::channel();
        Self::run(cx, |core| async move {
            // Sin nadie esperando, la pantalla que preguntó ya no está.
            let _ = answer.send(work(core).await);
        });
        receiver
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::Backend;
    use crate::testing::{wait_until, MemoryStorage};
    use cac_core::{Config, Connection, WorkLens};
    use gpui_kit::TestAppContext;

    /// Un servidor que no existe: el puerto 9 de esta máquina no contesta.
    fn install(cx: &mut TestAppContext) {
        let mut config = Config::new("http://127.0.0.1:9");
        config.stream_backoff = Duration::from_millis(20);
        cx.update(|cx| Backend::install(config, Arc::new(MemoryStorage::default()), cx).unwrap());
    }

    /// Lo que cambia en el core llega a lo que la pantalla lee, y la pantalla
    /// se entera.
    ///
    /// Roturas que atrapa: que el aviso del core no cruce al hilo principal,
    /// o que cruce sin avisar a quien observa (la vista no se repinta).
    #[gpui_kit::test]
    fn a_change_in_the_core_reaches_the_screen(cx: &mut TestAppContext) {
        install(cx);
        let snapshot = cx.update(|cx| Backend::snapshot(cx));
        let repaints = Arc::new(Mutex::new(0));
        let seen = repaints.clone();
        cx.update(|cx| cx.observe(&snapshot, move |_, _| *seen.lock().unwrap() += 1).detach());
        assert!(cx.update(|cx| snapshot.read(cx).session.is_none()));

        let ana = serde_json::from_value(serde_json::json!({ "id": "u1", "username": "ana" })).unwrap();
        cx.update(|cx| Backend::core(cx).sign_in(ana, "access".into(), "refresh".into()).unwrap());
        cx.run_until_parked();

        let username = cx.update(|cx| snapshot.read(cx).session.as_ref().map(|session| session.username.clone()));
        assert_eq!(username.as_deref(), Some("ana"));
        assert!(*repaints.lock().unwrap() >= 1, "quien observa no se enteró");
    }

    /// Lo que se le pide al core corre en su runtime, y su resultado vuelve
    /// como estado.
    ///
    /// El stream de eventos es el caso que lo exige: lanza una tarea propia
    /// de tokio nada más llamarlo. Roturas que atrapa: llamar al core fuera
    /// del runtime (el proceso se cae con «there is no reactor running»); no
    /// ejecutar lo pedido.
    #[gpui_kit::test]
    fn what_is_asked_of_the_core_runs_and_comes_back_as_state(cx: &mut TestAppContext) {
        install(cx);
        let snapshot = cx.update(|cx| Backend::snapshot(cx));
        let ana = serde_json::from_value(serde_json::json!({ "id": "u1", "username": "ana" })).unwrap();
        cx.update(|cx| Backend::core(cx).sign_in(ana, "access".into(), "refresh".into()).unwrap());

        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.my_work().set_lens(WorkLens::Watching);
                core.stream().start();
            })
        });

        wait_until(cx, "el stream se da por caído contra un servidor que no existe", |cx| {
            cx.update(|cx| snapshot.read(cx).connection.status == Connection::Down)
        });
        assert_eq!(cx.update(|cx| snapshot.read(cx).my_work.lens), WorkLens::Watching);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }
}
