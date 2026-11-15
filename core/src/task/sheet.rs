//! El detalle de una tarea tal como se lee: cada cosa ya dicha.
//!
//! Vino de `app/src/components/TaskDetailDrawer.tsx` y `lib/thread.ts`. La
//! pantalla recibe frases, nombres y listas ya partidas, y no vuelve a decidir
//! quién es un agente, qué subtarea está hecha ni en qué día cae nada.

use chrono::{NaiveDate, Timelike, Utc};

use super::{StatusOption, TaskComment, TaskDetail, Tasks};
use crate::dates::today_in;
use crate::mywork::{day_and_month, due_label, DueLabel};
use crate::texts::{text_with, Locale};

/// Una nota de agente: lo que un agente anotó con el token de alguien. Va
/// aparte de la conversación (command-and-control-359). Una que se publicó
/// para el cliente ya es conversación, y una línea de sistema nunca lo fue.
pub fn is_agent_note(comment: &TaskComment) -> bool {
    comment.via.as_deref() == Some("mcp") && comment.kind != "system" && comment.visibility != "public"
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SheetComment {
    pub id: String,
    /// Quién lo dice: su nombre, o «Agente de Ana» en una nota de agente.
    pub author: String,
    /// «14 nov, 14:05», en la hora de quien lee.
    pub when: String,
    /// Markdown.
    pub body: String,
    /// En una tarea que un cliente lee, quién lee este comentario, como clave
    /// del catálogo. `None` en una tarea interna: ahí todo es del equipo y
    /// decirlo en cada línea es ruido.
    pub audience: Option<String>,
    /// Una línea que escribió el servidor («status: x → y»).
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SheetSubtask {
    pub id: String,
    pub seq: i64,
    pub title: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SheetTag {
    pub name: String,
    pub color: String,
}

/// Lo que pinta el detalle de una tarea.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TaskSheet {
    pub id: String,
    pub seq: i64,
    /// Cómo se nombra en un commit (`proteus-12`, o el folio de un cliente).
    pub reference: Option<String>,
    pub title: String,
    /// «Espacio › Lista».
    pub place: String,
    /// «#7 La tarea madre», si es una subtarea.
    pub parent: Option<String>,
    pub status_name: String,
    pub status_color: String,
    /// Con su nombre de la API (`normal`, no `medium`). `none` es sin prioridad.
    pub priority: String,
    pub due: Option<DueLabel>,
    /// Por el nombre con que se les llama, no por su usuario.
    pub assignees: Vec<String>,
    pub tags: Vec<SheetTag>,
    /// Markdown; vacío si no tiene.
    pub description: String,
    /// Dónde la dejó la última sesión, y quién lo escribió.
    pub handoff: Option<String>,
    pub handoff_by: Option<String>,
    pub subtasks: Vec<SheetSubtask>,
    /// La conversación, en el orden en que se escribió.
    pub talk: Vec<SheetComment>,
    /// Las notas de agentes, aparte.
    pub notes: Vec<SheetComment>,
    /// Los ficheros de la tarea, por su nombre. Los de un comentario van con él.
    pub attachments: Vec<String>,
}

fn said_at(locale: Locale, instant: &str) -> String {
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(instant) else { return String::new() };
    let at = at.with_timezone(&chrono::Local);
    format!("{}, {:02}:{:02}", day_and_month(locale, at.date_naive()), at.hour(), at.minute())
}

/// El detalle, dicho. `statuses` son las columnas de su lista (de ellas sale
/// qué subtarea está hecha) y `today` el día de la organización.
pub fn task_sheet(detail: &TaskDetail, statuses: &[StatusOption], locale: Locale, today: NaiveDate) -> TaskSheet {
    let task = &detail.task;
    // El cliente lee una tarea de su proyecto que no se le ha escondido.
    let client_reads = !task.project_id.is_empty() && task.visibility != "internal";
    let comment = |comment: &TaskComment| SheetComment {
        id: comment.id.clone(),
        author: if is_agent_note(comment) {
            text_with(locale, "work:task.agentOf", &[("name", &comment.author_name)])
        } else {
            comment.author_name.clone()
        },
        when: said_at(locale, &comment.created_at),
        body: comment.body.trim().to_string(),
        audience: (client_reads && comment.kind != "system").then(|| {
            if comment.visibility == "public" { "work:task.clientSees" } else { "work:task.internal" }.to_string()
        }),
        system: comment.kind == "system",
    };
    let (notes, talk): (Vec<&TaskComment>, Vec<&TaskComment>) = detail.comments.iter().partition(|c| is_agent_note(c));
    let done = |status_id: &str| statuses.iter().any(|option| option.status.id == status_id && option.status.kind == "done");

    TaskSheet {
        id: task.id.clone(),
        seq: task.seq,
        reference: detail.reference.clone(),
        title: task.title.clone(),
        place: format!("{} › {}", detail.space_name, detail.list_name),
        parent: detail.parent.as_ref().map(|parent| format!("#{} {}", parent.seq, parent.title)),
        status_name: detail.status.name.clone(),
        status_color: detail.status.color.clone(),
        priority: task.priority.clone(),
        due: due_label(locale, task.due_at.as_deref(), today),
        assignees: detail.assignees.iter().map(|person| person.name.clone().unwrap_or_else(|| person.username.clone())).collect(),
        tags: detail.tags.iter().map(|tag| SheetTag { name: tag.name.clone(), color: tag.color.clone() }).collect(),
        description: task.description.trim().to_string(),
        handoff: task.handoff.clone().filter(|handoff| !handoff.trim().is_empty()),
        handoff_by: detail.handoff_by.clone(),
        subtasks: detail
            .subtasks
            .iter()
            .map(|card| SheetSubtask { id: card.id.clone(), seq: card.seq, title: card.title.clone(), done: done(&card.status_id) })
            .collect(),
        talk: talk.into_iter().map(comment).collect(),
        notes: notes.into_iter().map(comment).collect(),
        attachments: detail
            .attachments
            .iter()
            .filter(|file| file.comment_id.is_none())
            .map(|file| file.file_name.clone())
            .collect(),
    }
}

impl Tasks {
    /// El detalle de la tarea abierta, dicho; `None` mientras no ha llegado.
    ///
    /// «Hoy» es el de la organización que se mira, no el de la máquina.
    pub fn sheet(&self, locale: Locale) -> Option<TaskSheet> {
        let state = self.state();
        let detail = state.detail.as_ref()?;
        let today = today_in(self.core.orgs().state().current_zone(), Utc::now(), &chrono::Local);
        Some(task_sheet(detail, &state.statuses, locale, today))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::DueState;
    use serde_json::{json, Value};

    /// El detalle del contrato: una tarea de cliente, pública, con una nota
    /// de agente, una subtarea en la columna `s1` y un fichero de comentario.
    fn contract() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/task_detail.json")).unwrap()
    }

    fn sheet_of(detail: Value, statuses: Value) -> TaskSheet {
        let detail: TaskDetail = serde_json::from_value(detail).unwrap();
        let statuses: Vec<StatusOption> = serde_json::from_value(statuses).unwrap();
        task_sheet(&detail, &statuses, Locale::Es, NaiveDate::from_ymd_opt(2026, 9, 30).unwrap())
    }

    fn column(id: &str, kind: &str) -> Value {
        json!({ "status": { "id": id, "listId": "l1", "name": id, "kind": kind, "status": "x" }, "allowed": true })
    }

    fn comment(id: &str, edit: impl FnOnce(&mut Value)) -> Value {
        let mut comment = json!({
            "id": id, "authorName": "Ana Pérez", "visibility": "internal", "kind": "user",
            "body": "  hola  ", "createdAt": "2026-11-14T12:00:00Z",
        });
        edit(&mut comment);
        comment
    }

    /// Lo de arriba del detalle sale dicho: dónde está, de quién cuelga,
    /// quién la lleva por su nombre, y su vencimiento respecto a hoy.
    ///
    /// Roturas que atrapa: enseñar el usuario en vez del nombre; perder la
    /// tarea madre; un vencimiento sin decir que es hoy.
    #[test]
    fn the_head_of_the_sheet_is_already_said() {
        let sheet = sheet_of(contract(), json!([]));

        assert_eq!((sheet.seq, sheet.title.as_str()), (12, "Subir el backend"));
        assert_eq!(sheet.reference.as_deref(), Some("acme-7"));
        assert_eq!(sheet.place, "Command and control › tasks");
        assert_eq!(sheet.status_name, "In progress");
        assert_eq!(sheet.assignees, ["Ana Pérez"]);
        assert_eq!(sheet.tags, [SheetTag { name: "urgente".into(), color: "#f00".into() }]);
        assert_eq!(sheet.due.as_ref().map(|due| due.state), Some(DueState::Today));
        assert_eq!(sheet.description, "Con **markdown**.");
        assert_eq!((sheet.handoff.as_deref(), sheet.handoff_by.as_deref()), (Some("Dónde quedó."), Some("Ana Pérez")));
        let parent = sheet.parent.unwrap();
        assert!(parent.starts_with('#') && parent.len() > 3, "la madre sale como {parent:?}");

        // Sin nombre puesto, se le llama por su usuario; y un handoff en
        // blanco no es un handoff.
        let mut bare = contract();
        bare["assignees"] = json!([{ "id": "u2", "username": "luis" }]);
        bare["task"]["handoff"] = json!("   ");
        bare["parent"] = Value::Null;
        let sheet = sheet_of(bare, json!([]));
        assert_eq!(sheet.assignees, ["luis"]);
        assert_eq!((sheet.handoff, sheet.parent), (None, None));
    }

    /// Lo que anotó un agente va aparte de la conversación, y dice de quién
    /// es el agente; lo que se publicó para el cliente y las líneas de
    /// sistema se quedan en la conversación.
    ///
    /// Roturas que atrapa: las notas de agentes mezcladas en el hilo; una
    /// respuesta al cliente escondida entre las notas; firmar una nota de
    /// agente como si la hubiera escrito la persona.
    #[test]
    fn agent_notes_go_apart_from_the_conversation() {
        let mut detail = contract();
        detail["comments"] = json!([
            comment("person", |_| {}),
            comment("note", |c| c["via"] = json!("mcp")),
            comment("answer", |c| {
                c["via"] = json!("mcp");
                c["visibility"] = json!("public");
            }),
            comment("system", |c| {
                c["via"] = json!("mcp");
                c["kind"] = json!("system");
            }),
        ]);
        let sheet = sheet_of(detail, json!([]));

        let ids = |rows: &[SheetComment]| rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&sheet.talk), ["person", "answer", "system"]);
        assert_eq!(ids(&sheet.notes), ["note"]);
        assert_eq!(sheet.notes[0].author, "Agente de Ana Pérez");
        assert_eq!(sheet.talk[0].author, "Ana Pérez");
        assert_eq!(sheet.talk[0].body, "hola");
        assert!(sheet.talk[2].system && !sheet.talk[0].system);
    }

    /// En una tarea que el cliente lee, cada comentario dice quién lo lee; en
    /// una interna no se dice nada.
    ///
    /// Roturas que atrapa: marcar como interno lo que el cliente está
    /// leyendo (o al revés); llenar de «interno» una tarea que nunca sale del
    /// equipo; etiquetar una línea de sistema.
    #[test]
    fn a_comment_says_who_reads_it_only_where_a_client_could() {
        let mut detail = contract();
        detail["comments"] = json!([
            comment("team", |_| {}),
            comment("client", |c| c["visibility"] = json!("public")),
            comment("system", |c| c["kind"] = json!("system")),
        ]);
        let audiences = |detail: Value| {
            sheet_of(detail, json!([])).talk.into_iter().map(|row| row.audience).collect::<Vec<_>>()
        };
        assert_eq!(
            audiences(detail.clone()),
            [Some("work:task.internal".to_string()), Some("work:task.clientSees".to_string()), None]
        );

        // Escondida al cliente, o sin proyecto: es del equipo entera.
        let mut hidden = detail.clone();
        hidden["task"]["visibility"] = json!("internal");
        assert_eq!(audiences(hidden), [None, None, None]);
        let mut internal = detail;
        internal["task"]["projectId"] = json!("");
        assert_eq!(audiences(internal), [None, None, None]);
    }

    /// Una subtarea está hecha si su columna es de las de «hecho».
    ///
    /// Roturas que atrapa: darlas todas por hechas (o ninguna); mirar el
    /// nombre de la columna en vez de su clase.
    #[test]
    fn a_subtask_is_done_when_its_column_says_so() {
        let detail = contract();
        let open = sheet_of(detail.clone(), json!([column("s1", "active"), column("s9", "done")]));
        assert_eq!((open.subtasks[0].seq, open.subtasks[0].title.as_str()), (13, "Una línea del checklist"));
        assert!(!open.subtasks[0].done);

        let done = sheet_of(detail.clone(), json!([column("s1", "done")]));
        assert!(done.subtasks[0].done);
        // Sin columnas todavía, no se da por hecha.
        assert!(!sheet_of(detail, json!([])).subtasks[0].done);
    }

    /// Los ficheros de la tarea se listan; los de un comentario van con su
    /// comentario, no repetidos arriba.
    ///
    /// Rotura que atrapa: la captura de un comentario listada otra vez como
    /// adjunto de la tarea.
    #[test]
    fn only_the_tasks_own_files_are_listed() {
        let mut detail = contract();
        let of_comment = detail["attachments"][0].clone();
        let mut of_task = of_comment.clone();
        of_task["commentId"] = Value::Null;
        of_task["fileName"] = json!("plan.pdf");
        detail["attachments"] = json!([of_comment, of_task]);

        assert_eq!(sheet_of(detail, json!([])).attachments, ["plan.pdf"]);
    }

    /// Rotura que atrapa: la hora de un comentario en Greenwich, o sin hora.
    #[test]
    fn a_comment_says_when_in_local_time() {
        let local = chrono::DateTime::parse_from_rfc3339("2026-11-14T12:05:00Z").unwrap().with_timezone(&chrono::Local);
        let expected = format!("{}, {:02}:05", day_and_month(Locale::Es, local.date_naive()), local.hour());
        assert_eq!(said_at(Locale::Es, "2026-11-14T12:05:00Z"), expected);
        assert_eq!(said_at(Locale::Es, "nunca"), "");
    }
}
