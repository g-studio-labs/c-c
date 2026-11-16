//! El detalle de una tarea tal como se lee: cada cosa ya dicha.
//!
//! Vino de `app/src/components/TaskDetailDrawer.tsx` y `lib/thread.ts`. La
//! pantalla recibe frases, nombres y listas ya partidas, y no vuelve a decidir
//! quién es un agente, qué subtarea está hecha ni en qué día cae nada.

use chrono::{NaiveDate, Timelike, Utc};

use super::files::{is_image, size_label};
use super::{shown_name, StatusOption, TaskComment, TaskDetail, Tasks};
use crate::dates::{due_day, today_in};
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

/// Un fichero de la tarea, dicho.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SheetFile {
    pub id: String,
    pub name: String,
    /// «12 KB»; falta si el servidor no dijo cuánto pesa.
    pub size: Option<String>,
    /// Si se puede enseñar como imagen.
    pub image: bool,
    /// Si la descripción lo cita: quitarlo lo quita también del texto, y
    /// entre dos capturas iguales es lo que dice cuál es la que se ve.
    pub cited: bool,
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
    /// El día en que vence, `YYYY-MM-DD`: lo que se le pone a un selector de
    /// fecha. Sale del instante guardado con `due_day`, no de sus diez
    /// primeras letras leídas en la hora de la máquina.
    pub due_day: Option<String>,
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
    /// Los ficheros de la tarea. Los de un comentario van con él.
    pub attachments: Vec<SheetFile>,
    /// Si un cliente lee esta tarea: entonces lo que se comente lo lee él,
    /// salvo que se diga que es interno, y la pantalla tiene que decirlo.
    pub client_reads: bool,
}

/// Si un cliente lee la tarea: es de su proyecto y no se le ha escondido.
pub fn client_reads(detail: &TaskDetail) -> bool {
    !detail.task.project_id.is_empty() && detail.task.visibility != "internal"
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
    let client_reads = client_reads(detail);
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
        due_day: due_day(task.due_at.as_deref()).map(|day| day.to_string()),
        assignees: detail.assignees.iter().map(shown_name).collect(),
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
            .map(|file| SheetFile {
                id: file.id.clone(),
                name: file.file_name.clone(),
                size: file.bytes.map(size_label),
                image: is_image(file),
                cited: task.description.contains(&file.id),
            })
            .collect(),
        client_reads,
    }
}

impl super::TaskState {
    /// Por qué no hay detalle que leer, como clave del catálogo; `None` si
    /// lo hay. Una recarga que falla deja lo que había: sigue siendo lo
    /// último que se supo, y no se cambia por un aviso.
    pub fn waiting_reason(&self) -> Option<&'static str> {
        if self.detail.is_some() {
            None
        } else if self.error.is_some() {
            Some("common:last.requestsFailing")
        } else {
            Some("work:tree.loading")
        }
    }
}

impl Tasks {
    /// Comenta en la tarea abierta. `internal` es «que el cliente no lo lea»,
    /// y sólo viaja donde un cliente podría leerlo: en una tarea del equipo
    /// todo es interno ya, y lo decide el servidor.
    ///
    /// Un comentario en blanco no se manda.
    pub async fn say(&self, body: &str, internal: bool) -> Result<(), crate::ApiError> {
        let body = body.trim();
        if body.is_empty() {
            return Err(super::refused("empty-comment", "A comment needs some text"));
        }
        let to_the_client = self.state().detail.as_ref().is_some_and(client_reads);
        self.add_comment(body, (to_the_client && internal).then_some("internal")).await
    }

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
        assert_eq!(sheet.due_day.as_deref(), Some("2026-09-30"));
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
    /// Roturas que atrapa: la captura de un comentario listada otra vez como
    /// adjunto de la tarea; un fichero sin su peso, o con un peso inventado
    /// cuando el servidor no lo dijo; no saber cuál de dos capturas iguales
    /// es la que cita la descripción.
    #[test]
    fn only_the_tasks_own_files_are_listed() {
        let mut detail = contract();
        let of_comment = detail["attachments"][0].clone();
        let mut plan = of_comment.clone();
        plan["id"] = json!("a-plan");
        plan["commentId"] = Value::Null;
        plan["fileName"] = json!("plan.pdf");
        plan["contentType"] = json!("application/pdf");
        plan["bytes"] = json!(12 * 1024);
        let mut shot = plan.clone();
        shot["id"] = json!("a-shot");
        shot["fileName"] = json!("captura.PNG");
        shot["contentType"] = Value::Null;
        shot["bytes"] = Value::Null;
        detail["task"]["description"] = json!("Mira: ![](/api/v1/tasks/t1/attachments/a-shot/raw)");
        detail["attachments"] = json!([of_comment, plan, shot]);

        let files = sheet_of(detail, json!([])).attachments;
        assert_eq!(
            files,
            [
                SheetFile { id: "a-plan".into(), name: "plan.pdf".into(), size: Some("12 KB".into()), image: false, cited: false },
                SheetFile { id: "a-shot".into(), name: "captura.PNG".into(), size: None, image: true, cited: true },
            ]
        );
    }

    /// Roturas que atrapa: un panel en blanco mientras llega o cuando falla;
    /// tapar con un aviso una tarea que ya se estaba leyendo.
    #[test]
    fn a_sheet_that_is_not_there_says_why() {
        use crate::task::TaskState;
        let arriving = TaskState { task_id: Some("t1".into()), loading: true, ..TaskState::default() };
        assert_eq!(arriving.waiting_reason(), Some("work:tree.loading"));
        let failed = TaskState { task_id: Some("t1".into()), error: Some("x".into()), ..TaskState::default() };
        assert_eq!(failed.waiting_reason(), Some("common:last.requestsFailing"));
        let read: TaskDetail = serde_json::from_value(contract()).unwrap();
        let stale = TaskState { detail: Some(read), error: Some("x".into()), ..TaskState::default() };
        assert_eq!(stale.waiting_reason(), None);
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
