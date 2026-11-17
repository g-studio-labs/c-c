//! «Mi trabajo»: todo lo abierto en todos los espacios, una pregunta cada vez.
//!
//! Vino de `app/src/store/mywork.store.ts`, regla por regla. Cada una está ahí
//! porque algo falló antes, y su prueba en `tests/mywork.rs` lleva la rotura
//! que atrapa. La pantalla ya no decide nada de esto: manda intenciones
//! (`set_lens`, `set_watching`…) y pinta el estado que le llega por
//! `Listener::my_work_changed`.

pub mod board;
pub mod calendar;
pub mod filter;
pub mod list;
mod types;

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::dates::due_day;
use crate::error::ApiError;
use crate::tree::{find_folder, folder_list_ids, SpaceTree};
use crate::Core;

pub use board::{
    can_drop, work_board, BoardAssignee, BoardCard, BoardColumn, BoardStatus, DropVerdict, BAD_TRANSITION, COLUMN_MISSING,
};
pub use calendar::{
    calendar_chips, calendar_tone, day_detail_opens_up, day_title, lane_open_by_default, month_start, month_title, month_view, tone_is_high, unscheduled,
    week_start, week_title, week_view, weekday_name, CalendarChip, CalendarCursor, CalendarDay, CalendarView, CalendarWeek, MonthView,
    ToneCount, WeekSummary, WeekView, CALENDAR_TONES,
};
pub use filter::{filter_options, matches_filter, FilterOptions, TaskFilter, NOBODY, PRIORITIES};
pub use list::{day_and_month, due_label, empty_reason, work_groups, DueLabel, WorkGroup, WorkRow};
pub use types::{MyWorkState, OpenTask, ScopeKind, WorkLens, WorkScope};

/// Un respiro antes de recargar.
///
/// Mover cinco tarjetas seguidas —lo normal cuando alguien ordena su tablero,
/// y exactamente lo que hace el MCP— son cinco eventos en un segundo. Sin esto
/// serían cinco peticiones para llegar al mismo sitio.
const COALESCE: Duration = Duration::from_millis(400);

const PREFERENCES_KEY: &str = "mywork";

/// Lo que se recuerda entre arranques: la lente y el interruptor.
///
/// El trabajo no: una lista vieja de pendientes es peor que un momento sin
/// ninguna. El ámbito tampoco: es donde hiciste clic hace un rato, y abrir la
/// app acotada a una lista que ya no recuerdas haber elegido es un filtro que
/// parece que faltan datos.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    lens: WorkLens,
    include_closed: bool,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    data: Option<Vec<OpenTask>>,
    error: Option<String>,
    message: Option<String>,
}

/// Qué pregunta responde una carga.
fn key_of(org_id: Option<&str>, lens: WorkLens, include_closed: bool) -> String {
    format!("{}|{:?}|{}", org_id.unwrap_or(""), lens, include_closed)
}

/// Las tareas que caen dentro de un ámbito.
///
/// Una lista y un espacio se leen de la propia tarea. Una carpeta no: la
/// tarea no sabe de qué carpeta cuelga su lista, así que se le pregunta al
/// árbol qué listas hay bajo ella, subcarpetas incluidas. Una carpeta que no
/// está en el árbol no tiene ninguna.
pub fn in_scope(tasks: Vec<OpenTask>, scope: &WorkScope, spaces: &[SpaceTree]) -> Vec<OpenTask> {
    match scope.kind {
        ScopeKind::List => tasks.into_iter().filter(|task| task.list_id == scope.id).collect(),
        ScopeKind::Space => tasks.into_iter().filter(|task| task.space_id == scope.id).collect(),
        ScopeKind::Folder => {
            let lists = folder_list_ids(spaces, &scope.id).unwrap_or_default();
            tasks.into_iter().filter(|task| lists.contains(&task.list_id)).collect()
        }
    }
}

/// «Mi trabajo» de este core. Barato de pedir: es el mismo cada vez.
pub struct MyWork {
    pub(crate) core: Core,
}

impl Core {
    pub fn my_work(&self) -> MyWork {
        MyWork { core: self.clone() }
    }
}

impl MyWork {
    pub fn state(&self) -> MyWorkState {
        self.core.inner.my_work.lock().expect("my work lock").clone()
    }

    /// Cambia el estado y avisa, con el cerrojo ya suelto.
    fn update(&self, change: impl FnOnce(&mut MyWorkState)) {
        let state = {
            let mut state = self.core.inner.my_work.lock().expect("my work lock");
            change(&mut state);
            state.clone()
        };
        self.core.inner.listener.my_work_changed(state);
    }

    pub(crate) fn restore_preferences(&self) {
        let Some(raw) = self.core.inner.storage.get(PREFERENCES_KEY) else { return };
        let Ok(saved) = serde_json::from_str::<Preferences>(&raw) else { return };
        let mut state = self.core.inner.my_work.lock().expect("my work lock");
        state.lens = saved.lens;
        state.include_closed = saved.include_closed;
    }

    fn save_preferences(&self) {
        let state = self.state();
        let saved = Preferences { lens: state.lens, include_closed: state.include_closed };
        if let Ok(raw) = serde_json::to_string(&saved) {
            let _ = self.core.inner.storage.set(PREFERENCES_KEY, &raw);
        }
    }

    pub fn set_lens(&self, lens: WorkLens) {
        self.update(|state| state.lens = lens);
        self.save_preferences();
    }

    pub fn set_include_closed(&self, on: bool) {
        self.update(|state| state.include_closed = on);
        self.save_preferences();
    }

    /// Acota a una lista, a una carpeta o a un espacio, sellado con la
    /// organización desde la que se eligió. `None` quita el filtro.
    pub fn set_scope(&self, scope: Option<WorkScope>) {
        self.update(|state| state.scope = scope);
    }

    /// El árbol se volvió a cargar, y el ámbito de una carpeta vive de él.
    ///
    /// Si la carpeta ya no está (la borraron, o el árbol es de otra
    /// organización), el ámbito se tira, igual que se tira el de otra
    /// organización al cargar: quedarse filtrando por una carpeta que no
    /// existe es una pantalla vacía que parece que no hay trabajo. Si sigue,
    /// se avisa a quien pinta —lo que hay dentro pudo cambiar: una lista
    /// creada o movida a ella trae sus tareas— y se le pone el nombre que
    /// tenga ahora. Una lista o un espacio se leen de la tarea, no del árbol,
    /// y no les toca.
    pub(crate) fn tree_reloaded(&self) {
        let on_a_folder = |state: &MyWorkState| state.scope.as_ref().is_some_and(|scope| scope.kind == ScopeKind::Folder);
        // Sin una carpeta acotada no cambia nada, y no se avisa: el árbol se
        // recarga a menudo y cada aviso repinta la pantalla.
        if !on_a_folder(&self.state()) {
            return;
        }
        let spaces = self.core.tree().state().spaces;
        self.update(|state| {
            if !on_a_folder(state) {
                return;
            }
            let Some(scope) = state.scope.as_mut() else { return };
            match find_folder(&spaces, &scope.id) {
                Some(folder) => scope.name = folder.name.clone(),
                None => state.scope = None,
            }
        });
    }

    /// Lo que hay que pintar: las tareas cargadas, dentro del ámbito puesto.
    pub fn visible(&self) -> Vec<OpenTask> {
        let state = self.state();
        match &state.scope {
            None => state.tasks,
            Some(scope) => in_scope(state.tasks, scope, &self.core.tree().state().spaces),
        }
    }

    /// Lo visible que además pasa el filtro de la barra.
    pub fn filtered(&self, filter: &TaskFilter) -> Vec<OpenTask> {
        let visible = self.visible();
        if !filter.is_filtering() {
            return visible;
        }
        visible.into_iter().filter(|task| matches_filter(task, filter)).collect()
    }

    pub async fn load(&self, org_id: Option<String>) {
        // Un filtro de otra organización se tira.
        //
        // Una lista es de una organización concreta, así que al cambiar se
        // quedaba filtrando por algo que ahí no existe: la pantalla salía
        // vacía y parecía que no tenías trabajo. Se le pregunta **al ámbito**
        // de dónde es, no a la carga anterior de dónde era: la primera carga
        // de la sesión no tiene anterior, y con esa otra pregunta tiraba el
        // filtro que acababa de poner un clic.
        self.update(|state| {
            if state.scope.as_ref().is_some_and(|scope| scope.org_id != org_id) {
                state.scope = None;
            }
            state.org_id = org_id.clone();
            state.loading = true;
            state.error = None;
        });

        let asked = self.state();
        let mut parts: Vec<String> = Vec::new();
        if let Some(org) = &org_id {
            parts.push(format!("orgId={org}"));
        }
        parts.push(asked.lens.query().to_string());
        if asked.include_closed {
            parts.push("status=all".to_string());
        }
        parts.push("limit=200".to_string());
        let path = format!("/api/v1/tasks/?{}", parts.join("&"));

        match self.fetch(&path).await {
            Ok(tasks) => self.update(|state| {
                state.tasks = tasks;
                state.loading = false;
                // Con lo que **se preguntó**, no con lo que haya puesto ahora:
                // si la lente cambió mientras llegaba la respuesta, lo cargado
                // no responde a la nueva y `ensure_loaded` tiene que verlo.
                state.loaded_key =
                    Some(key_of(org_id.as_deref(), asked.lens, asked.include_closed));
            }),
            Err(error) => self.update(|state| {
                state.error = Some(error.to_string());
                state.loading = false;
                state.tasks = Vec::new();
            }),
        }
    }

    async fn fetch(&self, path: &str) -> Result<Vec<OpenTask>, ApiError> {
        let reply = self.core.request("GET", path, None, true).await?;
        let envelope = serde_json::from_str::<Envelope>(&reply.body).ok();
        if reply.status >= 400 {
            let code = envelope.as_ref().and_then(|e| e.error.clone()).unwrap_or_default();
            let message = envelope.and_then(|e| e.message).unwrap_or_else(|| code.clone());
            return Err(ApiError::Server { status: reply.status, code, message });
        }
        let mut tasks = envelope.and_then(|e| e.data).unwrap_or_default();
        for task in &mut tasks {
            // El día, ya resuelto: que ninguna pantalla tenga que leer `dueAt`.
            task.due_day = due_day(task.due_at.as_deref()).map(|day| day.to_string());
        }
        Ok(tasks)
    }

    /// Carga sólo si lo que hay no responde ya a lo que se pregunta.
    ///
    /// Volver a «Mi trabajo» pedía la lista entera cada vez aunque no hubiera
    /// cambiado nada: lo que cambia ya llega por eventos. Sólo se pide de
    /// nuevo si cambió la organización, la lente o el interruptor, o si la
    /// última carga falló.
    pub async fn ensure_loaded(&self, org_id: Option<String>) {
        let state = self.state();
        let foreign_scope = state.scope.as_ref().is_some_and(|scope| scope.org_id != org_id);
        let answered = state.loaded_key.as_deref()
            == Some(key_of(org_id.as_deref(), state.lens, state.include_closed).as_str());
        if foreign_scope || !answered || state.error.is_some() {
            self.load(org_id).await;
        }
    }

    /// Seguir o dejar de seguir, y quitar la fila cuando deja de responder a
    /// la pregunta de la pantalla.
    pub async fn set_watching(&self, task_id: &str, on: bool) -> Result<(), ApiError> {
        let path = format!("/api/v1/tasks/{task_id}/watch");
        let reply = if on {
            self.core.request("POST", &path, Some("{}".to_string()), true).await?
        } else {
            self.core.request("DELETE", &path, None, true).await?
        };
        if reply.status >= 400 {
            let envelope = serde_json::from_str::<Envelope>(&reply.body).ok();
            let code = envelope.as_ref().and_then(|e| e.error.clone()).unwrap_or_default();
            let message = envelope.and_then(|e| e.message).unwrap_or_else(|| code.clone());
            return Err(ApiError::Server { status: reply.status, code, message });
        }
        if !on && self.state().lens == WorkLens::Watching {
            self.forget(task_id);
        }
        Ok(())
    }

    /// Esa tarea ya no existe: fuera de aquí.
    ///
    /// Local y no una recarga: la fila ya no puede estar, y pedir la lista
    /// entera para quitar una que sabemos que se fue es más lento y puede
    /// fallar. Sin esto, borrar desde «Mi trabajo» dejaba la tarjeta pulsable.
    pub fn forget(&self, task_id: &str) {
        self.update(|state| state.tasks.retain(|task| task.id != task_id));
    }

    /// Cambia el vencimiento aquí, antes de que conteste el servidor, y
    /// devuelve el que tenía para poder deshacerlo. Arrastrar a un día y
    /// esperar la vuelta para ver la tarjeta moverse se lee como que el
    /// arrastre no funcionó.
    pub fn set_due_local(&self, task_id: &str, due_at: Option<String>) -> Option<String> {
        let mut previous = None;
        self.update(|state| {
            if let Some(task) = state.tasks.iter_mut().find(|task| task.id == task_id) {
                previous = task.due_at.take();
                task.due_day = due_day(due_at.as_deref()).map(|day| day.to_string());
                task.due_at = due_at;
            }
        });
        previous
    }

    /// Algo pasó con una tarea. Si es una de las que hay, se recarga.
    ///
    /// Sólo si ya la tenemos: una organización con movimiento emite eventos de
    /// tarjetas que a ti no te tocan, y recargar por cada una convertiría esta
    /// pantalla en una encuesta continua. El precio: una tarea que **entra**
    /// en tu trabajo no se refresca sola; eso llega a la campana.
    ///
    /// Salvo una tarea **nueva**: no puede estar todavía, y como la pantalla
    /// ya no recarga al volver a ella, sin esto no aparecería nunca. Y sólo si
    /// ya se cargó algo: si no, no hay lista que poner al día.
    pub fn task_event(&self, task_id: &str, new: bool) {
        let state = self.state();
        let ours = if new {
            state.loaded_key.is_some()
        } else {
            state.tasks.iter().any(|task| task.id == task_id)
        };
        if !ours {
            return;
        }

        // Cada evento pide turno, y tras el respiro sólo carga el último. No
        // se aborta al anterior: si ya pasó su respiro está a media carga, y
        // cortarlo ahí dejaría la pantalla en «cargando» para siempre.
        let turn = self.core.inner.my_work_reloads.fetch_add(1, Ordering::SeqCst) + 1;
        let work = self.core.my_work();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            if work.core.inner.my_work_reloads.load(Ordering::SeqCst) != turn {
                return;
            }
            let org_id = work.state().org_id;
            work.load(org_id).await;
        });
    }
}
