//! «Mi trabajo» tal como se pinta en lista: agrupado por espacio, y cada fila
//! con lo suyo ya dicho.
//!
//! Vino de `app/src/pages/MyWork.tsx` (los grupos) y de `TaskCardMini.tsx`
//! (`cuando`, `parentCrumb`). Aquí la pantalla recibe la frase y el matiz, y
//! no vuelve a calcular en qué día cae nada.

use chrono::{Datelike, NaiveDate, Utc};

use crate::dates::{due_day, due_state, today_in, DueState};
use crate::mywork::{filter_options, FilterOptions, MyWork, OpenTask, TaskFilter};
use crate::task::canonical_priority;
use crate::texts::{text, Locale};

/// Un vencimiento, dicho: la frase y cómo está respecto a hoy (de ahí sale el
/// color: rojo si ya pasó, ámbar si es hoy o mañana).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DueLabel {
    pub text: String,
    pub state: DueState,
}

/// Una fila de la lista.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WorkRow {
    pub id: String,
    pub seq: i64,
    pub title: String,
    /// De qué tarea es subtarea: «› #289 Título». Una subtarea sale aquí
    /// cuando es de alguien aunque su madre no lo sea, y sin esto parece una
    /// tarea suelta (command-and-control-297).
    pub parent: Option<String>,
    /// Con el nombre de la API (`normal`, no `medium`).
    pub priority: String,
    pub due: Option<DueLabel>,
}

/// Las tareas de un espacio.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WorkGroup {
    pub space_id: String,
    pub name: String,
    pub rows: Vec<WorkRow>,
}

pub(super) const MONTHS_ES: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"];
pub(super) const MONTHS_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Día y mes, como se dice en cada idioma: «1 sept», «Sep 1».
pub fn day_and_month(locale: Locale, day: NaiveDate) -> String {
    let month = day.month0() as usize;
    match locale {
        Locale::Es => format!("{} {}", day.day(), MONTHS_ES[month]),
        Locale::En => format!("{} {}", MONTHS_EN[month], day.day()),
    }
}

/// Un vencimiento dicho para quien lo lee: «hoy», «ayer», «mañana», o su día
/// y su mes. `None` si la tarea no tiene.
pub fn due_label(locale: Locale, due_at: Option<&str>, today: NaiveDate) -> Option<DueLabel> {
    let state = due_state(due_at, today)?;
    let day = due_day(due_at)?;
    let text = match state {
        DueState::Today => text(locale, "work:due.today"),
        DueState::Yesterday => text(locale, "work:due.yesterday"),
        DueState::Tomorrow => text(locale, "work:due.tomorrow"),
        DueState::Overdue | DueState::Later => day_and_month(locale, day),
    };
    Some(DueLabel { text, state })
}

/// De qué tarea es subtarea, como se lee en una fila o en una tarjeta:
/// «› #289 Título».
pub(crate) fn parent_crumb(task: &OpenTask) -> Option<String> {
    task.parent_seq
        .map(|seq| format!("› #{seq} {}", task.parent_title.as_deref().unwrap_or("")).trim_end().to_string())
}

fn row(task: &OpenTask, locale: Locale, today: NaiveDate) -> WorkRow {
    WorkRow {
        id: task.id.clone(),
        seq: task.seq,
        title: task.title.clone(),
        parent: parent_crumb(task),
        priority: canonical_priority(&task.priority).to_string(),
        due: due_label(locale, task.due_at.as_deref(), today),
    }
}

/// Agrupa por espacio, **en el orden en que el árbol enseña los espacios**,
/// para que el panel y la lista se lean igual. Un espacio que el árbol no
/// trae (la sala general, uno recién creado) va al final, en el orden en que
/// aparece.
pub fn work_groups(tasks: &[OpenTask], space_order: &[String], locale: Locale, today: NaiveDate) -> Vec<WorkGroup> {
    let mut groups: Vec<WorkGroup> = Vec::new();
    for task in tasks {
        let row = row(task, locale, today);
        match groups.iter_mut().find(|group| group.space_id == task.space_id) {
            Some(group) => group.rows.push(row),
            None => groups.push(WorkGroup {
                space_id: task.space_id.clone(),
                name: task.space_name.clone(),
                rows: vec![row],
            }),
        }
    }
    let place = |group: &WorkGroup| space_order.iter().position(|id| *id == group.space_id).unwrap_or(usize::MAX);
    // Estable: los que el árbol no trae conservan su orden de llegada.
    groups.sort_by_key(place);
    groups
}

/// Por qué la lista está vacía, como clave del catálogo: no es lo mismo «no
/// ha llegado», «se rompió», «nada coincide con lo que buscas» y «no tienes
/// nada», y una lista en blanco no distingue ninguna.
///
/// `hidden_by_filter` es que hay tareas cargadas en el ámbito y el filtro las
/// quitó todas.
pub fn empty_reason(state: &crate::mywork::MyWorkState, hidden_by_filter: bool) -> &'static str {
    if state.loading {
        "work:tree.loading"
    } else if state.error.is_some() {
        "common:last.requestsFailing"
    } else if hidden_by_filter {
        "work:board.filter.noMatch"
    } else if state.lens == crate::mywork::WorkLens::Watching {
        "work:myWork.nothingFollowed"
    } else {
        "work:myWork.nothingHere"
    }
}

impl MyWork {
    /// Por qué la lista no tiene filas. Si en el ámbito hay tareas y aun así
    /// no se pinta ninguna, es el filtro quien las quitó.
    pub fn empty_reason(&self) -> &'static str {
        empty_reason(&self.state(), !self.visible().is_empty())
    }

    /// Qué ofrecen los filtros: lo que hay en el ámbito, antes de filtrar.
    /// Sacarlo de lo ya filtrado dejaría cada desplegable con una sola opción
    /// en cuanto se elige algo.
    pub fn filter_options(&self, filter: &TaskFilter) -> FilterOptions {
        filter_options(&self.visible(), filter)
    }

    /// Lo que la lista pinta ahora: lo visible que pasa el filtro, agrupado.
    ///
    /// «Hoy» es el de la organización que se mira, no el de la máquina.
    pub fn groups(&self, filter: &TaskFilter, locale: Locale) -> Vec<WorkGroup> {
        let orgs = self.core.orgs().state();
        let today = today_in(orgs.current_zone(), Utc::now(), &chrono::Local);
        let order: Vec<String> = self.core.tree().state().spaces.into_iter().map(|space| space.id).collect();
        work_groups(&self.filtered(filter), &order, locale, today)
    }
}
