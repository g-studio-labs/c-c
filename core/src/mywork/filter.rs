//! El filtro de una lista de tareas. Vino de `app/src/lib/task-filter.ts`.

use unicode_normalization::UnicodeNormalization;

use super::types::OpenTask;

/// El valor de «sin asignar». No puede chocar con un id: los ids son UUID.
pub const NOBODY: &str = "-";

/// Lo que filtra una lista. Vacío es «cualquiera» en cada campo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TaskFilter {
    /// Texto del título, o el folio (`12` o `#12`).
    pub query: String,
    /// En «Mi trabajo» el responsable llega por nombre, y hace de id.
    pub assignee: String,
    pub priority: String,
}

impl TaskFilter {
    pub fn is_filtering(&self) -> bool {
        [&self.query, &self.assignee, &self.priority].iter().any(|v| !v.trim().is_empty())
    }
}

/// Sin acentos y en minúsculas: «revision» encuentra «Revisión».
pub fn fold(text: &str) -> String {
    text.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

// La prioridad guardada `medium` es la `normal` de la API de tareas. Se
// comparan por el mismo nombre, o una tarea que llegue con el guardado no
// saldría al filtrar por «media».
use crate::task::canonical_priority;

/// El número de un folio escrito como `12` o `#12`.
fn as_seq(query: &str) -> Option<i64> {
    let digits = query.strip_prefix('#').unwrap_or(query);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

pub fn matches_filter(task: &OpenTask, filter: &TaskFilter) -> bool {
    let query = filter.query.trim();
    if !query.is_empty() {
        // Un número es un folio, y se busca entero: «#1» no es «#12». Pero un
        // número también puede estar en el título («migrar a v2»), así que
        // cualquiera de las dos cosas vale.
        let by_seq = as_seq(query) == Some(task.seq);
        if !by_seq && !fold(&task.title).contains(&fold(query)) {
            return false;
        }
    }
    if !filter.assignee.is_empty() {
        let matches = match (filter.assignee.as_str(), task.assignee.as_deref()) {
            (NOBODY, assignee) => assignee.is_none(),
            (wanted, assignee) => assignee == Some(wanted),
        };
        if !matches {
            return false;
        }
    }
    if !filter.priority.is_empty() && canonical_priority(&task.priority) != filter.priority {
        return false;
    }
    true
}

/// Las prioridades, de la peor a la mejor: el orden en que se ofrecen.
pub const PRIORITIES: [&str; 5] = ["urgent", "high", "normal", "low", "none"];

/// Qué se puede elegir en cada desplegable de filtro. Una lista vacía es «no
/// ofrezcas ese desplegable».
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FilterOptions {
    /// Los responsables, por nombre y en orden alfabético.
    pub assignees: Vec<String>,
    /// Si alguna tarea no tiene responsable: entonces se ofrece «sin asignar».
    pub unassigned: bool,
    pub priorities: Vec<String>,
}

/// Las opciones salen de las tareas que hay, no de un catálogo: filtrar por
/// alguien que no tiene ninguna sólo enseña una lista vacía.
///
/// Un desplegable se ofrece si hay algo que separar (dos valores o más), o si
/// ya tiene algo elegido: sin eso, un filtro puesto no tendría cómo quitarse
/// cuando las tareas cambian por debajo.
pub fn filter_options(tasks: &[OpenTask], filter: &TaskFilter) -> FilterOptions {
    let mut people: Vec<String> = Vec::new();
    let mut unassigned = false;
    let mut priorities: Vec<&str> = Vec::new();
    for task in tasks {
        match task.assignee.as_deref() {
            Some(name) if !people.iter().any(|known| known == name) => people.push(name.to_string()),
            Some(_) => {}
            None => unassigned = true,
        }
        let priority = canonical_priority(&task.priority);
        if !priorities.contains(&priority) {
            priorities.push(priority);
        }
    }
    people.sort_by_key(|name| fold(name));
    let offer = |count: usize, chosen: &str| count >= 2 || !chosen.is_empty();
    FilterOptions {
        assignees: if offer(people.len() + usize::from(unassigned), &filter.assignee) { people } else { Vec::new() },
        unassigned,
        priorities: if offer(priorities.len(), &filter.priority) {
            PRIORITIES.iter().filter(|known| priorities.contains(known)).map(|known| known.to_string()).collect()
        } else {
            Vec::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(seq: i64, title: &str, priority: &str, assignee: Option<&str>) -> OpenTask {
        serde_json::from_value(serde_json::json!({
            "id": format!("t{seq}"), "seq": seq, "title": title, "priority": priority,
            "status": "open", "statusName": "Open", "statusKind": "open",
            "listId": "l1", "listName": "L", "spaceId": "s1", "spaceName": "S",
            "updatedAt": "2026-11-14T00:00:00Z", "assignee": assignee,
        }))
        .unwrap()
    }

    fn query(text: &str) -> TaskFilter {
        TaskFilter { query: text.into(), ..TaskFilter::default() }
    }

    /// Rotura que atrapa: comparar con acentos o con mayúsculas.
    #[test]
    fn a_search_ignores_accents_and_case() {
        let review = task(3, "Revisión del Diseño", "none", None);
        assert!(matches_filter(&review, &query("revision")));
        assert!(matches_filter(&review, &query("DISEÑO")));
        assert!(!matches_filter(&review, &query("despliegue")));
    }

    /// Rotura que atrapa: buscar el folio como texto (`#1` encontraría el 12),
    /// o dejar de mirar el título cuando la búsqueda es un número.
    #[test]
    fn a_number_is_a_whole_folio_or_part_of_the_title() {
        let twelve = task(12, "Subir el backend", "none", None);
        assert!(matches_filter(&twelve, &query("#12")));
        assert!(matches_filter(&twelve, &query("12")));
        assert!(!matches_filter(&twelve, &query("#1")));
        assert!(matches_filter(&task(7, "Migrar a v2", "none", None), &query("2")));
    }

    /// Rotura que atrapa: comparar la prioridad guardada con la de la API sin
    /// pasarlas al mismo nombre.
    #[test]
    fn the_stored_medium_is_the_normal_that_is_asked_for() {
        let normal = TaskFilter { priority: "normal".into(), ..TaskFilter::default() };
        assert!(matches_filter(&task(1, "a", "medium", None), &normal));
        assert!(matches_filter(&task(1, "a", "normal", None), &normal));
        assert!(!matches_filter(&task(1, "a", "high", None), &normal));
    }

    /// Rotura que atrapa: tratar «sin asignar» como un nombre más.
    #[test]
    fn nobody_finds_what_has_no_one() {
        let nobody = TaskFilter { assignee: NOBODY.into(), ..TaskFilter::default() };
        assert!(matches_filter(&task(1, "a", "none", None), &nobody));
        assert!(!matches_filter(&task(1, "a", "none", Some("ana")), &nobody));
        let ana = TaskFilter { assignee: "ana".into(), ..TaskFilter::default() };
        assert!(matches_filter(&task(1, "a", "none", Some("ana")), &ana));
        assert!(!matches_filter(&task(1, "a", "none", Some("luis")), &ana));
    }

    /// Roturas que atrapa: ofrecer a alguien dos veces; desordenar los
    /// nombres o las prioridades; olvidar «sin asignar»; ofrecer la prioridad
    /// guardada `medium` aparte de `normal`.
    #[test]
    fn the_options_are_what_the_tasks_have() {
        let tasks = [
            task(1, "a", "low", Some("Luis")),
            task(2, "b", "urgent", Some("ana")),
            task(3, "c", "medium", Some("Luis")),
            task(4, "d", "normal", None),
        ];
        let options = filter_options(&tasks, &TaskFilter::default());
        // Por nombre como lo lee una persona: la mayúscula no va delante.
        assert_eq!(options.assignees, ["ana", "Luis"]);
        assert!(options.unassigned);
        assert_eq!(options.priorities, ["urgent", "normal", "low"]);

        let assigned = filter_options(&tasks[..3], &TaskFilter::default());
        assert!(!assigned.unassigned);

        // La guardada y la de la API son la misma prioridad: no separan nada.
        let one = filter_options(&tasks[2..], &TaskFilter::default());
        assert!(one.priorities.is_empty(), "ofrece {:?}", one.priorities);
    }

    /// Roturas que atrapa: un desplegable con una sola opción, que no separa
    /// nada; y un filtro ya puesto que se queda sin desplegable para quitarlo.
    #[test]
    fn a_picker_is_offered_when_it_separates_or_is_already_set() {
        let same = [task(1, "a", "high", Some("ana")), task(2, "b", "high", Some("ana"))];
        let nothing = filter_options(&same, &TaskFilter::default());
        assert!(nothing.assignees.is_empty() && nothing.priorities.is_empty());

        // Cada desplegable mira lo suyo: elegir persona no ofrece prioridades.
        let person = filter_options(&same, &TaskFilter { assignee: "ana".into(), ..TaskFilter::default() });
        assert_eq!(person.assignees, ["ana"]);
        assert!(person.priorities.is_empty());
        let priority = filter_options(&same, &TaskFilter { priority: "high".into(), ..TaskFilter::default() });
        assert_eq!(priority.priorities, ["high"]);
        assert!(priority.assignees.is_empty());

        // Una persona y tareas sin nadie ya son dos cosas que separar.
        let mixed = [task(1, "a", "high", Some("ana")), task(2, "b", "high", None)];
        assert_eq!(filter_options(&mixed, &TaskFilter::default()).assignees, ["ana"]);
    }

    /// Rotura que atrapa: dar por puesto un filtro de espacios.
    #[test]
    fn an_empty_filter_is_not_filtering() {
        assert!(!TaskFilter::default().is_filtering());
        assert!(!query("   ").is_filtering());
        assert!(query("x").is_filtering());
    }
}
