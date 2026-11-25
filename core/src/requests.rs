//! «Por atender»: lo que los agentes esperan de una persona, con su botón
//! (command-and-control-500/504).
//!
//! Vino de `app/src/lib/requests.ts`, `store/agent-requests.store.ts` y del
//! manejo de `request:*` en `use-report-events.ts`. El contrato es el del
//! backend (`domain/request.go`): aquí se describe, se pide y se decide qué
//! se manda al contestar.
//!
//! - **Los pedidos no son de una organización**: el servidor contesta los de
//!   todas las de quien pregunta, como en la app de siempre. Un pedido trae
//!   la suya, y abrir su tarea cambia a ella si hace falta.
//! - **El número es tuyo aunque mires «Equipo»**: se cuenta aparte de la
//!   lista, sobre los abiertos de todos, con `canAnswer`.
//! - **«Espera a Ana» sale de los mismos abiertos de todos**: la lista de
//!   «Mi trabajo» (`GET /api/v1/tasks/`) no trae la marca, y el tablero del
//!   backend sí; aquí se deduce de lo que ya se pide para el número, y la
//!   mueven los mismos eventos.
//! - **`request:open` es un agente pidiendo que se mire ya**: la pantalla se
//!   pone delante con el pedido resaltado y se le dice al servidor que se
//!   enseñó (`/opened` con el `nonce`), que es lo que el MCP espera para decir
//!   `delivered`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::doc_edit::DocLeave;
use crate::error::ApiError;
use crate::stream::ServerEvent;
use crate::texts::{text, text_with, Locale};
use crate::Core;

/// Qué se le pide a la persona.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    ApproveMessage,
    Test,
    Decide,
    Run,
    Obtain,
    /// Uno que esta versión no conoce: se enseña sin botones.
    #[default]
    #[serde(other)]
    Unknown,
}

impl RequestKind {
    pub fn label(self, locale: Locale) -> String {
        let key = match self {
            RequestKind::ApproveMessage => "work:requests.kind.approve_message",
            RequestKind::Test => "work:requests.kind.test",
            RequestKind::Decide => "work:requests.kind.decide",
            RequestKind::Run => "work:requests.kind.run",
            RequestKind::Obtain => "work:requests.kind.obtain",
            RequestKind::Unknown => return String::new(),
        };
        text(locale, key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum RequestStatus {
    #[default]
    Open,
    Answered,
    Withdrawn,
}

/// Una opción de «decidir», con su consecuencia.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct RequestOption {
    pub label: String,
    pub consequence: String,
    pub recommended: bool,
}

/// Lo que el agente dejó, según el tipo (`domain.RequestBody`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct RequestBody {
    pub draft: String,
    pub destination: String,
    pub destination_url: String,
    pub url: String,
    pub steps: Vec<String>,
    pub expected: String,
    pub question: String,
    pub options: Vec<RequestOption>,
    pub command: String,
    /// `where` en el cable: `camelCase` se come el guion bajo.
    pub where_: String,
    pub undo: String,
    pub what: String,
    pub from: String,
}

/// Lo que se contesta (`domain.RequestAnswer`). Lo que no va, no se manda:
/// el servidor valida por tipo con lo que llega.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct RequestAnswer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved: Option<bool>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub final_text: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub edited: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub works: Option<bool>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub screenshot_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub option: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub output: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct RequestPerson {
    pub id: String,
    pub name: String,
}

/// La tarea de la que cuelga.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct RequestTask {
    pub id: String,
    pub r#ref: String,
    pub title: String,
    pub priority: String,
    pub status: String,
}

/// Un pedido tal como lo leen las apps (`domain.RequestView`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct AgentRequest {
    pub id: String,
    /// `req-…`: lo que se copia para seguirlo desde la terminal.
    pub code: String,
    pub org_id: String,
    pub task: RequestTask,
    pub kind: RequestKind,
    pub summary: String,
    pub body: RequestBody,
    pub version: i64,
    pub status: RequestStatus,
    pub requested_of: RequestPerson,
    pub requested_by: RequestPerson,
    pub via: String,
    pub created_at: String,
    pub answer: Option<RequestAnswer>,
    pub answered_by: Option<RequestPerson>,
    pub answered_at: Option<String>,
    pub answered_via: String,
    pub published_url: String,
    /// Quien mira es a quien se le pidió y sigue abierto. En «Equipo» se ven
    /// los de los demás sin poder contestarlos por ellos.
    pub can_answer: bool,
}

impl AgentRequest {
    /// «command-and-control-12 · Título», o sólo el título.
    pub fn task_line(&self) -> String {
        [self.task.r#ref.as_str(), self.task.title.as_str()].iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join(" · ")
    }
}

/// De quién se miran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum RequestScope {
    #[default]
    Mine,
    Team,
}

impl RequestScope {
    fn query(self) -> &'static str {
        match self {
            RequestScope::Mine => "mine",
            RequestScope::Team => "team",
        }
    }
}

/// «Por atender», como lo pinta una pantalla.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RequestsState {
    pub scope: RequestScope,
    pub open: Vec<AgentRequest>,
    /// Contestados y retirados: «Resueltos», plegados.
    pub closed: Vec<AgentRequest>,
    /// `false` mientras la lista de este filtro no ha llegado.
    pub loaded: bool,
    /// La última carga falló: lo que hubiera se queda.
    pub failed: bool,
    /// Cuántos abiertos te esperan **a ti**, mires el filtro que mires.
    pub mine_count: i64,
    /// A quién espera cada tarea con pedidos abiertos, por id de tarea.
    pub waiting_on: HashMap<String, Vec<String>>,
    /// Si es la pantalla que se tiene delante.
    pub on_screen: bool,
    /// El pedido que hay que resaltar: al que llevó un agente o un aviso.
    pub highlight: Option<String>,
}

/// Lo que se pulsó, antes de volverlo respuesta.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AnswerDraft {
    /// Aprobar tal cual, o con el texto editado.
    Approve { edited: Option<String>, original: String },
    Reject { reason: String },
    Works { note: String },
    Fails { note: String },
    Choose { option: String, note: String },
    Done { note: String, output: String },
}

/// Lo que se manda al contestar, o `None` si falta lo que el tipo exige:
/// rechazar pide motivo, «no funciona» pide nota, y editar pide texto.
///
/// Un texto «editado» igual al borrador no es una edición: se aprueba tal
/// cual, y el pedido no dice «aprobado con cambios» sin haberlos.
pub fn build_answer(draft: &AnswerDraft) -> Option<RequestAnswer> {
    let trim = |text: &str| text.trim().to_string();
    let base = RequestAnswer::default();
    Some(match draft {
        AnswerDraft::Approve { edited: None, .. } => RequestAnswer { approved: Some(true), ..base },
        AnswerDraft::Approve { edited: Some(edited), original } => {
            let edited = trim(edited);
            if edited.is_empty() {
                return None;
            }
            if edited == original.trim() {
                RequestAnswer { approved: Some(true), ..base }
            } else {
                RequestAnswer { approved: Some(true), final_text: edited, ..base }
            }
        }
        AnswerDraft::Reject { reason } => {
            let reason = trim(reason);
            if reason.is_empty() {
                return None;
            }
            RequestAnswer { approved: Some(false), reason, ..base }
        }
        AnswerDraft::Works { note } => RequestAnswer { works: Some(true), note: trim(note), ..base },
        AnswerDraft::Fails { note } => {
            let note = trim(note);
            if note.is_empty() {
                return None;
            }
            RequestAnswer { works: Some(false), note, ..base }
        }
        AnswerDraft::Choose { option, note } => RequestAnswer { option: option.clone(), note: trim(note), ..base },
        // La salida se pega como salió: sólo sobra el final en blanco.
        AnswerDraft::Done { note, output } => RequestAnswer { output: output.trim_end().to_string(), note: trim(note), ..base },
    })
}

/// Cómo quedó un pedido cerrado, en una línea (`Outcome` de `RequestCard.tsx`).
pub fn request_outcome(request: &AgentRequest, locale: Locale) -> String {
    let answer = request.answer.clone().unwrap_or_default();
    if request.status == RequestStatus::Withdrawn {
        return text(locale, "work:requests.outcome.withdrawn");
    }
    match request.kind {
        RequestKind::ApproveMessage if answer.approved == Some(true) => {
            text(locale, if answer.edited { "work:requests.outcome.approvedEdited" } else { "work:requests.outcome.approved" })
        }
        RequestKind::ApproveMessage => text_with(locale, "work:requests.outcome.rejected", &[("reason", &answer.reason)]),
        RequestKind::Test if answer.works == Some(true) => text(locale, "work:requests.outcome.works"),
        RequestKind::Test => text_with(locale, "work:requests.outcome.fails", &[("note", &answer.note)]),
        RequestKind::Decide => text_with(locale, "work:requests.outcome.chose", &[("option", &answer.option)]),
        _ => text(locale, "work:requests.outcome.done"),
    }
}

/// «Espera a Ana, Luis»: la marca de una tarea con pedidos abiertos.
pub fn waiting_label(names: &[String], locale: Locale) -> Option<String> {
    (!names.is_empty()).then(|| text_with(locale, "work:board.waitingOn", &[("names", &names.join(", "))]))
}

/// A quién espera cada tarea, sin repetir a nadie, en el orden en que se
/// pidió (`OpenRequestsOfTasks` del backend).
fn waiting_of(open: &[AgentRequest]) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for request in open.iter().filter(|request| request.status == RequestStatus::Open) {
        let names = out.entry(request.task.id.clone()).or_default();
        if !names.contains(&request.requested_of.name) {
            names.push(request.requested_of.name.clone());
        }
    }
    out
}

/// Lo de `request:open`.
#[derive(Deserialize)]
struct OpenAsk {
    #[serde(default)]
    code: String,
    #[serde(default)]
    nonce: String,
}

/// Lo que hay detrás de la pantalla.
#[derive(Default)]
pub(crate) struct RequestsData {
    state: RequestsState,
    /// Los abiertos de todos: de aquí salen el número y las marcas.
    team_open: Vec<AgentRequest>,
    /// Cuál de las cargas de la lista es la vigente: una que llega tarde (de
    /// otro filtro, o adelantada por un evento) no pisa a la de después.
    list_turn: u64,
    marks_turn: u64,
}

pub struct Requests {
    core: Core,
}

impl Core {
    pub fn requests(&self) -> Requests {
        Requests { core: self.clone() }
    }
}

impl Requests {
    fn data(&self) -> std::sync::MutexGuard<'_, RequestsData> {
        self.core.inner.requests.lock().expect("requests lock")
    }

    pub fn state(&self) -> RequestsState {
        self.data().state.clone()
    }

    fn change(&self, edit: impl FnOnce(&mut RequestsData)) {
        let state = {
            let mut data = self.data();
            edit(&mut data);
            data.state.clone()
        };
        self.core.inner.listener.requests_changed(state);
    }

    /// Pone «Por atender» delante, con ese pedido resaltado si se da. Es de
    /// «Trabajo»: deja comunicación, Plataforma y la documentación. Con texto
    /// sin guardar en un documento no se va (se pregunta, `doc_edit`), y
    /// devuelve `false`.
    pub fn show(&self, highlight: Option<String>) -> bool {
        if self.core.docs().held_back(DocLeave::AllWork) {
            return false;
        }
        self.core.docs().close_now();
        self.core.docs().leave_module();
        self.core.comms().leave();
        self.core.platform().leave();
        self.core.overview().leave();
        let highlight = highlight.map(|code| code.to_lowercase());
        self.change(|data| {
            data.state.on_screen = true;
            data.state.highlight = highlight;
        });
        true
    }

    /// Volver a las tareas. Lo cargado se queda para la vuelta.
    pub fn leave(&self) {
        if self.data().state.on_screen {
            self.change(|data| {
                data.state.on_screen = false;
                data.state.highlight = None;
            });
        }
    }

    /// Cambia de filtro y trae el suyo. Lo de antes no se enseña mientras:
    /// los de otro filtro no son lo que se pidió.
    pub async fn set_scope(&self, scope: RequestScope) {
        if self.data().state.scope == scope {
            return;
        }
        self.change(|data| {
            data.state.scope = scope;
            data.state.open.clear();
            data.state.closed.clear();
            data.state.loaded = false;
        });
        self.load().await;
    }

    /// Trae los abiertos y los resueltos del filtro de ahora.
    pub async fn load(&self) {
        let (scope, turn) = {
            let mut data = self.data();
            data.list_turn += 1;
            (data.state.scope, data.list_turn)
        };
        let path = |status: &str| format!("/api/v1/requests?scope={}&status={status}", scope.query());
        let (open_path, closed_path) = (path("open"), path("closed"));
        let (open, closed) = tokio::join!(self.core.get_list::<AgentRequest>(&open_path), self.core.get_list::<AgentRequest>(&closed_path));
        self.change(|data| {
            if data.list_turn != turn || data.state.scope != scope {
                return;
            }
            match (open, closed) {
                (Ok(open), Ok(closed)) => {
                    data.state.open = open;
                    data.state.closed = closed;
                    data.state.loaded = true;
                    data.state.failed = false;
                }
                _ => data.state.failed = true,
            }
        });
    }

    /// El número y las marcas «espera a…», de los abiertos de todos. Un fallo
    /// deja los de antes: son un extra, no algo por lo que interrumpir.
    pub async fn refresh_marks(&self) {
        let turn = {
            let mut data = self.data();
            data.marks_turn += 1;
            data.marks_turn
        };
        let Ok(open) = self.core.get_list::<AgentRequest>("/api/v1/requests?scope=team&status=open").await else { return };
        self.change(|data| {
            if data.marks_turn != turn {
                return;
            }
            data.team_open = open;
            Self::recount(data);
        });
    }

    fn recount(data: &mut RequestsData) {
        data.state.mine_count = data.team_open.iter().filter(|request| request.can_answer).count() as i64;
        data.state.waiting_on = waiting_of(&data.team_open);
    }

    /// Contesta. Sale bien → pasa a «Resueltos» en el acto, y baja el número
    /// y la marca de su tarea; el evento que manda el servidor lo confirma.
    /// Lo que el tipo exige y falta (`build_answer`) ni se manda.
    pub async fn answer(&self, code: &str, draft: AnswerDraft) -> Result<AgentRequest, ApiError> {
        let Some(answer) = build_answer(&draft) else {
            return Err(ApiError::Server { status: 400, code: "request-invalid".into(), message: "missing what this kind needs".into() });
        };
        let body = serde_json::to_value(&answer).unwrap_or_default();
        let done: AgentRequest = self.core.call("POST", &format!("/api/v1/requests/{code}/answer"), Some(body)).await?;
        self.change(|data| {
            data.state.open.retain(|request| request.code != done.code);
            data.state.closed.retain(|request| request.code != done.code);
            data.state.closed.insert(0, done.clone());
            data.team_open.retain(|request| request.code != done.code);
            Self::recount(data);
        });
        Ok(done)
    }

    /// Abre la tarea del pedido, en su organización.
    pub async fn open_task(&self, request: &AgentRequest) -> bool {
        self.core.inbox().open(&format!("/tasks?task={}", request.task.id), &request.org_id).await
    }

    /// Que se trae de nuevo lo que haya cambiado en el servidor: la lista,
    /// sólo si alguna vez se pidió.
    async fn reload(&self) {
        let loaded = self.data().state.loaded;
        if loaded {
            self.reload_with_list().await;
        } else {
            self.refresh_marks().await;
        }
    }

    /// Un agente pidió que se mire ya: delante, resaltado, y el «lo
    /// enseñé» que espera el MCP. Sin enseñarlo (un documento a medio
    /// escribir) no se dice que se enseñó.
    async fn open_asked(&self, ask: OpenAsk) {
        if ask.code.is_empty() || ask.nonce.is_empty() {
            return;
        }
        let code = ask.code.to_lowercase();
        if !self.show(Some(code.clone())) {
            return;
        }
        self.core.inner.listener.show_request(code.clone());
        let path = format!("/api/v1/requests/{code}/opened");
        let _ = self.core.call_ok("POST", &path, Some(serde_json::json!({ "nonce": ask.nonce }))).await;
        self.reload_with_list().await;
    }

    async fn reload_with_list(&self) {
        tokio::join!(self.refresh_marks(), self.load());
    }

    pub(crate) fn server_event(&self, event: &ServerEvent) {
        match event.event.as_str() {
            "request:created" | "request:updated" | "request:answered" | "request:withdrawn" => {
                let requests = self.core.requests();
                tokio::spawn(async move { requests.reload().await });
            }
            "request:open" => {
                let Ok(ask) = serde_json::from_str::<OpenAsk>(&event.data) else { return };
                let requests = self.core.requests();
                tokio::spawn(async move { requests.open_asked(ask).await });
            }
            _ => {}
        }
    }

    /// Al entrar, y al volver de una caída: lo que pasó sin stream no llegó.
    pub(crate) async fn refresh(&self) {
        self.reload().await;
    }

    /// Al salir: los pedidos de una persona no son de la siguiente.
    pub(crate) fn clear(&self) {
        self.change(|data| *data = RequestsData::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(code: &str, task: &str, of: &str) -> AgentRequest {
        AgentRequest {
            code: code.into(),
            task: RequestTask { id: task.into(), ..Default::default() },
            requested_of: RequestPerson { id: of.to_lowercase(), name: of.into() },
            ..Default::default()
        }
    }

    /// Lo que se manda por cada botón. Roturas que atrapa: rechazar sin
    /// motivo; «no funciona» sin nota; aprobar «editado» con el mismo texto
    /// (diría «con cambios» sin haberlos); un editado vacío que aprueba
    /// nada; recortar el principio de la salida pegada (su sangría es parte
    /// de ella); mandar campos de otro tipo.
    #[test]
    fn each_button_sends_what_its_kind_needs() {
        let draft = "Hola equipo".to_string();
        assert_eq!(build_answer(&AnswerDraft::Approve { edited: None, original: draft.clone() }), Some(RequestAnswer { approved: Some(true), ..Default::default() }));
        assert_eq!(build_answer(&AnswerDraft::Approve { edited: Some("  Hola equipo \n".into()), original: draft.clone() }), Some(RequestAnswer { approved: Some(true), ..Default::default() }), "sin cambios no es editar");
        assert_eq!(build_answer(&AnswerDraft::Approve { edited: Some("Hola, equipo".into()), original: draft.clone() }).unwrap().final_text, "Hola, equipo");
        assert_eq!(build_answer(&AnswerDraft::Approve { edited: Some("  ".into()), original: draft }), None);
        assert_eq!(build_answer(&AnswerDraft::Reject { reason: " ".into() }), None);
        assert_eq!(build_answer(&AnswerDraft::Reject { reason: " tono ".into() }), Some(RequestAnswer { approved: Some(false), reason: "tono".into(), ..Default::default() }));
        assert_eq!(build_answer(&AnswerDraft::Fails { note: "".into() }), None);
        assert_eq!(build_answer(&AnswerDraft::Fails { note: "sale 18:40".into() }).unwrap().works, Some(false));
        assert_eq!(build_answer(&AnswerDraft::Works { note: "".into() }), Some(RequestAnswer { works: Some(true), ..Default::default() }));
        assert_eq!(build_answer(&AnswerDraft::Choose { option: "B".into(), note: "".into() }), Some(RequestAnswer { option: "B".into(), ..Default::default() }));
        let done = build_answer(&AnswerDraft::Done { note: "".into(), output: "  ok\n  hecho\n\n".into() }).unwrap();
        assert_eq!(done.output, "  ok\n  hecho");
        assert_eq!(serde_json::to_value(&done).unwrap(), serde_json::json!({ "output": "  ok\n  hecho" }), "sólo lo que va");
    }

    /// La marca de cada tarea. Roturas que atrapa: repetir a la misma persona
    /// con dos pedidos; contar los cerrados; juntar las de dos tareas.
    #[test]
    fn each_task_says_who_it_waits_on_once() {
        let mut closed = request("req-3", "t1", "Luis");
        closed.status = RequestStatus::Answered;
        let open = [request("req-1", "t1", "Ana"), request("req-2", "t1", "Ana"), closed, request("req-4", "t2", "Luis"), request("req-5", "t1", "Bea")];
        let waiting = waiting_of(&open);
        assert_eq!(waiting.get("t1"), Some(&vec!["Ana".to_string(), "Bea".to_string()]));
        assert_eq!(waiting.get("t2"), Some(&vec!["Luis".to_string()]));
        assert_eq!(waiting_label(&waiting["t1"], Locale::Es).as_deref(), Some("Espera a Ana, Bea"));
        assert_eq!(waiting_label(&[], Locale::Es), None);
    }

    /// Cómo quedó. Roturas que atrapa: decir «aprobado» de uno retirado;
    /// «con cambios» sin `edited`; un rechazo sin su motivo.
    #[test]
    fn a_closed_request_says_how_it_ended() {
        let mut approved = request("req-1", "t1", "Ana");
        approved.kind = RequestKind::ApproveMessage;
        approved.status = RequestStatus::Answered;
        approved.answer = Some(RequestAnswer { approved: Some(true), ..Default::default() });
        assert_eq!(request_outcome(&approved, Locale::Es), "Aprobado");
        approved.answer.as_mut().unwrap().edited = true;
        assert_eq!(request_outcome(&approved, Locale::Es), "Aprobado con cambios");
        approved.answer = Some(RequestAnswer { approved: Some(false), reason: "tono".into(), ..Default::default() });
        assert_eq!(request_outcome(&approved, Locale::Es), "Rechazado: tono");
        approved.status = RequestStatus::Withdrawn;
        assert_eq!(request_outcome(&approved, Locale::Es), "Retirado por el agente");
    }

    /// Lo que manda el servidor se lee entero. Roturas que atrapa: perder
    /// `where` (palabra reservada) o `ref`; un tipo nuevo que rompe la lista
    /// entera en vez de llegar como desconocido.
    #[test]
    fn the_wire_reads_where_ref_and_unknown_kinds() {
        let raw = serde_json::json!({
            "id": "1", "code": "req-ab12", "orgId": "o1", "kind": "run", "summary": "s", "status": "open", "canAnswer": true,
            "task": { "id": "t1", "ref": "cac-12", "title": "T", "priority": "high", "status": "in_progress", "link": "/tasks?task=t1" },
            "body": { "command": "ls", "where": "gstd" }, "requestedOf": { "id": "u1", "name": "Ana" }, "requestedBy": { "id": "u2", "name": "Bot" },
            "createdAt": "2026-11-24T10:00:00Z", "updatedAt": "2026-11-24T10:00:00Z", "version": 1, "link": "/requests?open=req-ab12"
        });
        let request: AgentRequest = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!((request.kind, request.body.where_.as_str(), request.task.r#ref.as_str()), (RequestKind::Run, "gstd", "cac-12"));
        assert_eq!(request.task_line(), "cac-12 · T");
        let mut other = raw;
        other["kind"] = serde_json::json!("sign");
        assert_eq!(serde_json::from_value::<AgentRequest>(other).unwrap().kind, RequestKind::Unknown);
    }
}
