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
    ApiError, Config, ConnectionState, Core, Listener, MyWorkState, OrgsState, Session, Storage, TaskState,
    TreeState,
};
use gpui_kit::{App, AppContext, Entity, Global};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

/// Un aviso del core, de camino al hilo principal.
enum CoreEvent {
    Session(Option<Session>),
    MyWork(MyWorkState),
    Orgs(OrgsState),
    Tree(TreeState),
    Task(TaskState),
    Connection(ConnectionState),
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
    fn task_changed(&self, state: TaskState) {
        self.send(CoreEvent::Task(state));
    }
    fn connection_changed(&self, state: ConnectionState) {
        self.send(CoreEvent::Connection(state));
    }
    // `server_event` (lo que no es estado del core: un aviso, un sonido) se
    // reenviará cuando haya una pantalla que lo use.
}

/// El estado del core tal como lo ve la pantalla, entero y ya en el hilo
/// principal.
#[derive(Default)]
pub struct Snapshot {
    pub session: Option<Session>,
    pub my_work: MyWorkState,
    pub orgs: OrgsState,
    pub tree: TreeState,
    pub task: TaskState,
    pub connection: ConnectionState,
}

impl Snapshot {
    fn apply(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::Session(session) => self.session = session,
            CoreEvent::MyWork(state) => self.my_work = state,
            CoreEvent::Orgs(state) => self.orgs = state,
            CoreEvent::Tree(state) => self.tree = state,
            CoreEvent::Task(state) => self.task = state,
            CoreEvent::Connection(state) => self.connection = state,
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
            ..Snapshot::default()
        });

        let target = snapshot.clone();
        cx.spawn(async move |cx| {
            while let Some(event) = receiver.recv().await {
                cx.update(|cx| {
                    target.update(cx, |snapshot, cx| {
                        snapshot.apply(event);
                        cx.notify();
                    });
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
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::Backend;
    use cac_core::{Config, Connection, Storage, WorkLens};
    use gpui_kit::TestAppContext;

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

    /// Un servidor que no existe: el puerto 9 de esta máquina no contesta.
    fn install(cx: &mut TestAppContext) {
        let mut config = Config::new("http://127.0.0.1:9");
        config.stream_backoff = Duration::from_millis(20);
        cx.update(|cx| Backend::install(config, Arc::new(MemoryStorage::default()), cx).unwrap());
    }

    /// Espera a algo que viene de otro hilo (el runtime del core).
    fn wait_until(cx: &mut TestAppContext, what: &str, done: impl Fn(&mut TestAppContext) -> bool) {
        cx.executor().allow_parking();
        for _ in 0..300 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no llegó a pasar: {what}");
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
