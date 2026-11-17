//! El detalle de una tarea tal como se lee: cada cosa ya dicha.
//!
//! Vino de `app/src/components/TaskDetailDrawer.tsx` y `lib/thread.ts`. La
//! pantalla recibe frases, nombres y listas ya partidas, y no vuelve a decidir
//! quién es un agente, qué subtarea está hecha ni en qué día cae nada.

use chrono::{NaiveDate, Utc};

use super::files::{is_image, size_label};
use super::{shown_name, StatusOption, TaskComment, TaskDetail, Tasks};
use crate::dates::{date_and_time, due_day, today_in};
use crate::mywork::{due_label, BoardStatus, DueLabel};
use crate::session::Session;
use crate::texts::{text, text_with, Locale};

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
    /// Si quien mira puede editarlo o borrarlo.
    pub mine: bool,
    /// Si se editó después de escribirse.
    pub edited: bool,
    /// Si lo anotó un agente: lleva su icono delante del nombre.
    pub agent: bool,
}

/// Si esa persona puede editar o borrar ese comentario: quien lo escribió, o
/// un superadmin. Una línea del servidor no es de nadie.
///
/// Es la regla del servidor (`commentScope`), repetida aquí para no ofrecer
/// lo que va a rechazar.
pub fn can_change_comment(comment: &TaskComment, viewer: Option<&Session>) -> bool {
    comment.kind != "system"
        && viewer.is_some_and(|viewer| viewer.superadmin || (!viewer.id.is_empty() && viewer.id == comment.author_user_id))
}

/// Si el comentario cambió después de escribirse. Con un segundo de margen:
/// el servidor pone las dos fechas al crearlo y no siempre coinciden.
fn was_edited(comment: &TaskComment) -> bool {
    let at = |instant: &str| chrono::DateTime::parse_from_rfc3339(instant).ok();
    match (at(&comment.created_at), at(&comment.updated_at)) {
        (Some(created), Some(updated)) => (updated - created).num_milliseconds() > 1000,
        _ => false,
    }
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

/// Una tarea a la que ésta espera, como se lee en el riel: su referencia (o
/// su título si no tiene) y, si ya está hecha, dicho. Una dependencia hecha
/// sigue listada, tachada: dice que ya no para nada.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SheetBlocker {
    pub id: String,
    /// «proteus-12», o «proteus-12 · hecha».
    pub label: String,
    pub title: String,
    pub done: bool,
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
    /// «Espacio / Lista», como encabeza la página en la app de siempre.
    pub place: String,
    /// La lista en la que vive, sola: lo que dice su campo del riel.
    pub list: String,
    /// «#7 La tarea madre», si es una subtarea.
    pub parent: Option<String>,
    /// El id de la madre, para abrirla desde su nombre.
    pub parent_id: Option<String>,
    pub status_name: String,
    pub status_color: String,
    /// Cuál de los cuatro estados es su columna: de ahí sale el color del
    /// chip (los `--status-*` del tema), no del hex que manda el servidor,
    /// que no cambia con el tema. `None` si es uno que esta versión no conoce.
    pub status: Option<BoardStatus>,
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
    /// «Ana Pérez, 14/11/26, 14:17:40»: de quién y de cuándo es el «dónde
    /// quedó». Falta si no se sabe alguna de las dos cosas.
    pub handoff_signature: Option<String>,
    /// De qué depende; sólo se enseña.
    pub blockers: Vec<SheetBlocker>,
    /// «Actualizada el 14/11/26, 14:17:40», al pie.
    pub updated: String,
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
    /// Si tiene ramas, PRs o commits enlazados. Sin nada, la app de siempre
    /// no pinta el panel de desarrollo sino una línea: «Enlazar una PR»
    /// (`TaskGitPanel.tsx`).
    pub git_linked: bool,
}

/// Si eso es un comentario que mandar: uno en blanco no lo es, y el botón de
/// comentar está apagado mientras (`disabled={sending || !comment.trim()}`
/// en `TaskDetailDrawer.tsx`).
pub fn says_something(body: &str) -> bool {
    !body.trim().is_empty()
}

/// Si un cliente lee la tarea: es de su proyecto y no se le ha escondido.
pub fn client_reads(detail: &TaskDetail) -> bool {
    !detail.task.project_id.is_empty() && detail.task.visibility != "internal"
}

/// Cuándo se dijo algo, en la hora de quien mira y con el formato de los
/// sellos de tiempo de la app de siempre (`fechaYHora`): la firma del «dónde
/// quedó», el pie y cada comentario.
fn said_at(locale: Locale, instant: &str) -> String {
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(instant) else { return String::new() };
    date_and_time(locale, at.with_timezone(&chrono::Local).naive_local())
}

/// El detalle, dicho. `statuses` son las columnas de su lista (de ellas sale
/// qué subtarea está hecha) y `today` el día de la organización.
pub fn task_sheet(
    detail: &TaskDetail,
    statuses: &[StatusOption],
    locale: Locale,
    today: NaiveDate,
    viewer: Option<&Session>,
) -> TaskSheet {
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
        mine: can_change_comment(comment, viewer),
        edited: was_edited(comment),
        agent: is_agent_note(comment),
    };
    let (notes, talk): (Vec<&TaskComment>, Vec<&TaskComment>) = detail.comments.iter().partition(|c| is_agent_note(c));
    let handoff = task.handoff.clone().filter(|handoff| !handoff.trim().is_empty());
    let done = |status_id: &str| statuses.iter().any(|option| option.status.id == status_id && option.status.kind == "done");

    TaskSheet {
        id: task.id.clone(),
        seq: task.seq,
        reference: detail.reference.clone(),
        title: task.title.clone(),
        place: format!("{} / {}", detail.space_name, detail.list_name),
        list: detail.list_name.clone(),
        parent: detail.parent.as_ref().map(|parent| format!("#{} {}", parent.seq, parent.title)),
        parent_id: detail.parent.as_ref().map(|parent| parent.id.clone()),
        status_name: detail.status.name.clone(),
        status_color: detail.status.color.clone(),
        status: BoardStatus::of(&detail.status.status),
        priority: task.priority.clone(),
        due: due_label(locale, task.due_at.as_deref(), today),
        due_day: due_day(task.due_at.as_deref()).map(|day| day.to_string()),
        assignees: detail.assignees.iter().map(shown_name).collect(),
        tags: detail.tags.iter().map(|tag| SheetTag { name: tag.name.clone(), color: tag.color.clone() }).collect(),
        description: task.description.trim().to_string(),
        handoff: handoff.clone(),
        handoff_by: detail.handoff_by.clone(),
        handoff_signature: handoff.as_ref().and(detail.handoff_by.as_deref()).zip(task.handoff_at.as_deref()).map(|(who, when)| {
            text_with(locale, "work:task.handoffBy", &[("who", who), ("when", &said_at(locale, when))])
        }),
        blockers: detail
            .blocked_by
            .iter()
            .map(|blocker| {
                let name = blocker.reference.clone().filter(|reference| !reference.is_empty()).unwrap_or_else(|| blocker.title.clone());
                SheetBlocker {
                    id: blocker.id.clone(),
                    label: if blocker.open { name } else { format!("{name} · {}", text(locale, "work:task.blockerDone")) },
                    title: blocker.title.clone(),
                    done: !blocker.open,
                }
            })
            .collect(),
        updated: text_with(locale, "work:task.updated", &[("when", &said_at(locale, &task.updated_at))]),
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
        git_linked: detail.git.as_ref().is_some_and(|git| !(git.branches.is_empty() && git.prs.is_empty() && git.commits.is_empty())),
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
        if !says_something(body) {
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
        Some(task_sheet(detail, &state.statuses, locale, today, self.core.session().as_ref()))
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
        task_sheet(&detail, &statuses, Locale::Es, NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(), None)
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
        assert_eq!(sheet.place, "Command and control / tasks");
        assert_eq!(sheet.list, "tasks");
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

    /// El color del estado sale de cuál de los cuatro es su columna, la
    /// madre se puede abrir, y el pie y el «dónde quedó» dicen de cuándo son.
    ///
    /// Roturas que atrapa: pintar el chip con el hex del servidor (o gris)
    /// por no saber qué estado es; una madre que se nombra y no se puede
    /// abrir; un «dónde quedó» sin firma, o firmado sin que haya ninguno; un
    /// pie que no dice cuándo se tocó por última vez.
    #[test]
    fn the_sheet_says_which_state_it_is_and_when_it_was_touched() {
        let mut detail = contract();
        detail["status"]["status"] = json!("in_progress");
        detail["parent"]["id"] = json!("madre");
        detail["task"]["updatedAt"] = json!("2026-11-14T20:17:40Z");
        // Otro día que el de la última vez: el pie no dice cuándo se creó.
        detail["task"]["createdAt"] = json!("2026-11-10T08:00:00Z");
        let sheet = sheet_of(detail.clone(), json!([]));
        let when = said_at(Locale::Es, "2026-11-14T20:17:40Z");

        assert_eq!(sheet.status, Some(BoardStatus::InProgress));
        assert_eq!(sheet.parent_id.as_deref(), Some("madre"));
        assert_eq!(sheet.updated, format!("Actualizada el {when}"));
        assert_eq!(sheet.handoff_signature, Some(format!("Ana Pérez, {when}")));

        detail["status"]["status"] = json!("resolved");
        detail["task"]["handoff"] = json!("  ");
        let sheet = sheet_of(detail.clone(), json!([]));
        assert_eq!(sheet.status, Some(BoardStatus::Done));
        assert_eq!(sheet.handoff_signature, None);

        detail["status"]["status"] = json!("inventado");
        detail["parent"] = Value::Null;
        let sheet = sheet_of(detail, json!([]));
        assert_eq!((sheet.status, sheet.parent_id), (None, None));
    }

    /// De qué depende sale por su referencia, o por su título si no tiene, y
    /// lo que ya está hecho lo dice.
    ///
    /// Roturas que atrapa: perder las dependencias; dar por pendiente una
    /// que ya se hizo (o al revés); una dependencia sin referencia que sale
    /// en blanco.
    #[test]
    fn what_the_task_waits_for_is_listed() {
        let mut detail = contract();
        detail["blockedBy"] = json!([
            { "id": "b1", "ref": "proteus-12", "title": "La API", "open": true },
            { "id": "b2", "title": "El diseño", "open": false },
        ]);
        let sheet = sheet_of(detail, json!([]));

        let said: Vec<(&str, &str, bool)> = sheet.blockers.iter().map(|b| (b.id.as_str(), b.label.as_str(), b.done)).collect();
        assert_eq!(said, [("b1", "proteus-12", false), ("b2", "El diseño · hecha", true)]);
        assert_eq!(sheet.blockers[0].title, "La API");
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
        // La marca de agente (su icono) va con la nota y con nada más.
        assert!(sheet.notes[0].agent, "la nota de un agente no lleva su marca");
        assert!(sheet.talk.iter().all(|comment| !comment.agent), "algo de la conversación va marcado como de agente");
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

    /// Sin ramas, PRs ni commits no hay nada enlazado; con cualquiera, sí. Y
    /// un comentario en blanco no es un comentario.
    ///
    /// Roturas que atrapa: ofrecer «Enlazar una PR» como si no hubiera nada
    /// en una tarea que ya tiene su PR (o callarlo en la que no tiene);
    /// mirar sólo las PRs y no las ramas o los commits; un botón de comentar
    /// encendido con la caja llena de espacios.
    #[test]
    fn the_sheet_says_if_code_is_linked_and_if_there_is_something_to_say() {
        let mut detail = contract();
        assert!(sheet_of(detail.clone(), json!([])).git_linked);
        for only in ["branches", "prs", "commits"] {
            let mut one = detail.clone();
            for kind in ["branches", "prs", "commits"] {
                if kind != only {
                    one["git"][kind] = json!([]);
                }
            }
            assert!(sheet_of(one, json!([])).git_linked, "sólo con {only}");
        }
        for kind in ["branches", "prs", "commits"] {
            detail["git"][kind] = json!([]);
        }
        assert!(!sheet_of(detail.clone(), json!([])).git_linked);
        detail["git"] = Value::Null;
        assert!(!sheet_of(detail, json!([])).git_linked);

        assert!(says_something(" hola\n"));
        assert!(!says_something("") && !says_something(" \n\t "));
    }

    /// Rotura que atrapa: la hora de un comentario en Greenwich, o sin hora.
    #[test]
    fn a_comment_says_when_in_local_time() {
        let local = chrono::DateTime::parse_from_rfc3339("2026-11-14T12:05:09Z").unwrap().with_timezone(&chrono::Local);
        let expected = format!("{}, {}:05:09", crate::dates::short_date(Locale::Es, local.date_naive()), chrono::Timelike::hour(&local));
        assert_eq!(said_at(Locale::Es, "2026-11-14T12:05:09Z"), expected);
        assert_eq!(said_at(Locale::Es, "nunca"), "");
    }
    fn viewer(id: &str, superadmin: bool) -> Session {
        serde_json::from_value(json!({ "id": id, "username": id, "superadmin": superadmin })).unwrap()
    }

    /// Un comentario lo cambia quien lo escribió o un superadmin, y se dice
    /// si se editó.
    ///
    /// Roturas que atrapa: ofrecer editar el comentario de otro (el servidor
    /// contesta 403 `not-the-author`: sería un botón que no hace nada); que un superadmin no
    /// pueda; ofrecer editar una línea del servidor; dar por editado todo
    /// comentario porque sus dos fechas no son idénticas; sin sesión, o con
    /// un autor en blanco, dar a cualquiera por autor.
    #[test]
    fn a_comment_is_changed_by_whoever_wrote_it() {
        let mut detail = contract();
        detail["comments"] = json!([
            comment("mio", |c| { c["authorUserId"] = json!("u1"); c["updatedAt"] = json!("2026-11-14T12:00:00.400Z") }),
            comment("ajeno", |c| { c["authorUserId"] = json!("u2"); c["updatedAt"] = json!("2026-11-14T12:05:00Z") }),
            comment("del-servidor", |c| { c["authorUserId"] = json!("u1"); c["kind"] = json!("system") }),
            comment("sin-autor", |c| c["authorUserId"] = json!("")),
        ]);
        let detail: TaskDetail = serde_json::from_value(detail).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let seen_by = |viewer: Option<&Session>| -> Vec<(String, bool, bool)> {
            task_sheet(&detail, &[], Locale::Es, today, viewer).talk.into_iter().map(|c| (c.id, c.mine, c.edited)).collect()
        };
        let said = |mine: [bool; 4]| -> Vec<(String, bool, bool)> {
            ["mio", "ajeno", "del-servidor", "sin-autor"]
                .iter()
                .zip(mine)
                .map(|(id, mine)| (id.to_string(), mine, *id == "ajeno"))
                .collect()
        };

        assert_eq!(seen_by(Some(&viewer("u1", false))), said([true, false, false, false]));
        assert_eq!(seen_by(Some(&viewer("u9", true))), said([true, true, false, true]));
        assert_eq!(seen_by(Some(&viewer("", false))), said([false, false, false, false]));
        assert_eq!(seen_by(None), said([false, false, false, false]));
    }

}
