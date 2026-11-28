//! «Por atender»: lo que los agentes esperan de una persona, con su botón
//! (command-and-control-500/504).
//!
//! Vino de `app/src/lib/requests.ts`, `store/agent-requests.store.ts` y del
//! manejo de `request:*` en `use-report-events.ts`. El contrato es el del
//! backend (`domain/request.go`): aquí se describe, se pide y se decide qué
//! se manda al contestar.
//!
//! - **El servidor contesta los pedidos de todas las organizaciones** de quien
//!   pregunta, y aquí se reparten (command-and-control-527): la lista, el
//!   número y «Resueltos» son de la organización que se mira, y de los
//!   abiertos tuyos en las otras se avisa (`elsewhere`) para que no se pierda
//!   ninguno. Lo de fuera se guarda entero y se reparte al cambiar de
//!   organización, sin volver a pedir.
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
    /// Cómo lo llama el servidor; uno desconocido no se puede pedir.
    pub fn wire(self) -> Option<&'static str> {
        Some(match self {
            RequestKind::ApproveMessage => "approve_message",
            RequestKind::Test => "test",
            RequestKind::Decide => "decide",
            RequestKind::Run => "run",
            RequestKind::Obtain => "obtain",
            RequestKind::Unknown => return None,
        })
    }

    pub const ALL: [RequestKind; 5] = [RequestKind::ApproveMessage, RequestKind::Test, RequestKind::Decide, RequestKind::Run, RequestKind::Obtain];

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

/// Los pedidos abiertos que te esperan en una organización que no se mira.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RequestsElsewhere {
    pub org_id: String,
    pub org_name: String,
    pub count: i64,
}

/// «N pedidos más en G Studio».
pub fn elsewhere_label(elsewhere: &RequestsElsewhere, locale: Locale) -> String {
    let phrase = crate::texts::text_count(locale, "work:requests.elsewhere", elsewhere.count);
    phrase.replace("{{org}}", &elsewhere.org_name)
}

/// Qué estado se mira. `All` es la bandeja de siempre: los abiertos y, debajo,
/// «Resueltos» plegado. `Resolved` son esos mismos (contestados y retirados)
/// sin pliegue; `Withdrawn`, sólo los retirados.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum RequestStatusFilter {
    #[default]
    All,
    Open,
    Resolved,
    Withdrawn,
}

impl RequestStatusFilter {
    /// Si hay lista de abiertos, y con qué estado se piden los de abajo.
    fn shows_open(self) -> bool {
        matches!(self, RequestStatusFilter::All | RequestStatusFilter::Open)
    }

    fn closed_query(self) -> Option<&'static str> {
        match self {
            RequestStatusFilter::All => Some("closed"),
            RequestStatusFilter::Open => None,
            RequestStatusFilter::Resolved => Some("closed"),
            RequestStatusFilter::Withdrawn => Some("withdrawn"),
        }
    }

    fn admits(self, status: RequestStatus) -> bool {
        match self {
            RequestStatusFilter::All => status != RequestStatus::Open,
            RequestStatusFilter::Open => false,
            RequestStatusFilter::Resolved => status != RequestStatus::Open,
            RequestStatusFilter::Withdrawn => status == RequestStatus::Withdrawn,
        }
    }
}

/// Los filtros de la bandeja, además de «Míos / Equipo». Van al servidor en
/// la misma consulta que el cursor: la página siguiente es de lo filtrado.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RequestFilter {
    /// Código, resumen, título o ref de la tarea.
    pub text: String,
    /// Vacío: todos los tipos.
    pub kinds: Vec<RequestKind>,
    pub status: RequestStatusFilter,
}

impl RequestFilter {
    fn query(&self) -> String {
        let mut out = String::new();
        let text = self.text.trim();
        if !text.is_empty() {
            out.push_str("&q=");
            out.push_str(&crate::chat::query_escaped(text));
        }
        let kinds: Vec<&str> = self.kinds.iter().filter_map(|kind| kind.wire()).collect();
        if !kinds.is_empty() {
            out.push_str("&kind=");
            out.push_str(&kinds.join(","));
        }
        out
    }

    /// Si un pedido que llega por un evento entra en lo filtrado. El texto se
    /// mira en lo que se tiene (código, resumen, título); la ref, no.
    fn admits(&self, request: &AgentRequest) -> bool {
        if !self.kinds.is_empty() && !self.kinds.contains(&request.kind) {
            return false;
        }
        let text = self.text.trim().to_lowercase();
        text.is_empty() || [&request.code, &request.summary, &request.task.title, &request.task.r#ref].iter().any(|field| field.to_lowercase().contains(&text))
    }

    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || !self.kinds.is_empty() || self.status != RequestStatusFilter::All
    }
}

/// Cómo va una lista por páginas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RequestPaging {
    /// Llegó la primera página.
    pub loaded: bool,
    /// Hay una página en camino (la primera o la siguiente).
    pub loading: bool,
    /// La última petición falló: se dice, y se reintenta a mano.
    pub failed: bool,
    /// El servidor dijo que hay más.
    pub more: bool,
}

/// «Por atender», como lo pinta una pantalla.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RequestsState {
    pub scope: RequestScope,
    pub filter: RequestFilter,
    pub open: Vec<AgentRequest>,
    /// Contestados y retirados: «Resueltos», plegados. Sólo se piden al
    /// desplegarlos (o al filtrar por uno de esos estados), y por páginas.
    pub closed: Vec<AgentRequest>,
    /// `false` mientras la lista de este filtro no ha llegado.
    pub loaded: bool,
    /// La última carga falló: lo que hubiera se queda.
    pub failed: bool,
    /// Las páginas de los abiertos y de los de abajo.
    pub open_paging: RequestPaging,
    pub closed_paging: RequestPaging,
    /// «Resueltos» desplegado. Con un filtro de estado cerrado no hay pliegue:
    /// la lista se enseña siempre.
    pub resolved_open: bool,
    /// Cuántos abiertos te esperan **a ti**, mires el filtro que mires.
    pub mine_count: i64,
    /// Lo tuyo abierto en las otras organizaciones, para el aviso de arriba.
    pub elsewhere: Vec<RequestsElsewhere>,
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

/// Una página como la contesta el servidor cuando se le pide con cursor. Un
/// servidor de antes contesta la lista entera, sin cursor: es la última.
#[derive(Deserialize)]
#[serde(untagged)]
enum PageWire {
    Page {
        #[serde(default)]
        requests: Vec<AgentRequest>,
        #[serde(default)]
        next: String,
    },
    List(Vec<AgentRequest>),
}

impl PageWire {
    fn split(self) -> (Vec<AgentRequest>, String) {
        match self {
            PageWire::Page { requests, next } => (requests, next),
            PageWire::List(requests) => (requests, String::new()),
        }
    }
}

/// De cuántos en cuántos. Los abiertos son pocos y mandan; los resueltos se
/// leen hacia atrás, a ratos.
const OPEN_PAGE: usize = 50;
const CLOSED_PAGE: usize = 20;
/// El tope del servidor por página (`repository.MaxRequestPage`).
const MAX_PAGE: usize = 100;

/// Lo que hay detrás de la pantalla.
#[derive(Default)]
pub(crate) struct RequestsData {
    /// Lo que se pinta: sale de lo de abajo y de la organización que se mira
    /// (`project`), nunca se escribe a mano.
    state: RequestsState,
    /// Los abiertos de este filtro, de todas las organizaciones.
    all_open: Vec<AgentRequest>,
    /// Los de abajo, de la organización que se mira (`closed_org`): se piden
    /// por páginas, y una página de todas no diría cuántos son de ésta.
    all_closed: Vec<AgentRequest>,
    closed_org: Option<String>,
    /// Los pedidos sueltos a los que llevó un enlace y que no estaban en lo
    /// cargado (un resuelto de hace meses): se enseñan igual.
    linked: Vec<AgentRequest>,
    /// Dónde sigue cada lista; vacío: no hay más.
    open_next: String,
    closed_next: String,
    /// Los abiertos de todos: de aquí salen el número y las marcas.
    team_open: Vec<AgentRequest>,
    /// Cuál de las cargas de cada lista es la vigente: una que llega tarde (de
    /// otro filtro, o adelantada por un evento) no pisa a la de después.
    list_turn: u64,
    closed_turn: u64,
    marks_turn: u64,
    /// El resaltado que ya se buscó: se busca (y abre «Resueltos») una vez.
    revealed: Option<String>,
}

impl RequestsData {
    /// Si la lista de abajo se tiene que pedir: plegada, no.
    fn wants_closed(&self) -> bool {
        match self.state.filter.status {
            RequestStatusFilter::All => self.state.resolved_open,
            RequestStatusFilter::Open => false,
            RequestStatusFilter::Resolved | RequestStatusFilter::Withdrawn => true,
        }
    }

    fn reset_closed(&mut self) {
        self.closed_turn += 1;
        self.all_closed.clear();
        self.closed_next.clear();
        self.closed_org = None;
        self.state.closed_paging = RequestPaging::default();
    }
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
        self.change_with(edit)
    }

    fn change_with<R>(&self, edit: impl FnOnce(&mut RequestsData) -> R) -> R {
        // La organización se lee antes de coger el candado: así no hay dos
        // candados anidados en ningún orden.
        let orgs = self.core.orgs().state();
        let (state, out) = {
            let mut data = self.data();
            let out = edit(&mut data);
            Self::project(&mut data, &orgs);
            (data.state.clone(), out)
        };
        self.core.inner.listener.requests_changed(state);
        out
    }

    /// Se cambió de organización (o llegaron las organizaciones): los abiertos
    /// y el número se reparten de otra manera, sin volver a pedirlos; los de
    /// abajo son de una organización, y si se estaban mirando se piden los de
    /// la nueva.
    pub(crate) fn org_changed(&self) {
        let here = self.core.orgs().state().current_org_id;
        let reload = self.change_with(|data| {
            let moved = data.closed_org.is_some() && data.closed_org != here;
            if moved {
                data.reset_closed();
            }
            moved && data.wants_closed()
        });
        if reload {
            let requests = self.core.requests();
            tokio::spawn(async move { requests.closed_page(true).await });
        }
    }

    /// Reparte lo guardado según la organización que se mira.
    fn project(data: &mut RequestsData, orgs: &crate::orgs::OrgsState) {
        let here = orgs.current_org_id.as_deref();
        let mine_here = |request: &&AgentRequest| Some(request.org_id.as_str()) == here;
        let (linked_open, linked_closed): (Vec<&AgentRequest>, Vec<&AgentRequest>) = data.linked.iter().filter(mine_here).partition(|request| request.status == RequestStatus::Open);
        let mut open: Vec<AgentRequest> = data.all_open.iter().filter(mine_here).cloned().collect();
        for request in linked_open {
            if !open.iter().any(|listed| listed.code == request.code) {
                open.push(request.clone());
            }
        }
        let mut closed: Vec<AgentRequest> = linked_closed.into_iter().filter(|request| !data.all_closed.iter().any(|listed| listed.code == request.code)).cloned().collect();
        closed.extend(data.all_closed.iter().filter(mine_here).cloned());
        data.state.open = open;
        data.state.closed = closed;
        data.state.mine_count = data.team_open.iter().filter(mine_here).filter(|request| request.can_answer).count() as i64;
        let mut elsewhere: Vec<RequestsElsewhere> = Vec::new();
        for request in data.team_open.iter().filter(|request| request.can_answer && Some(request.org_id.as_str()) != here) {
            match elsewhere.iter_mut().find(|entry| entry.org_id == request.org_id) {
                Some(entry) => entry.count += 1,
                None => {
                    // Sin su nombre no se avisa: un uuid no dice nada.
                    let Some(org_name) = orgs.orgs.iter().find(|org| org.id == request.org_id).map(|org| org.name.clone()) else { continue };
                    elsewhere.push(RequestsElsewhere { org_id: request.org_id.clone(), org_name, count: 1 });
                }
            }
        }
        data.state.elsewhere = elsewhere;
        data.state.waiting_on = waiting_of(&data.team_open);
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
        self.core.notes().leave();
        let highlight = highlight.map(|code| code.to_lowercase());
        self.change(|data| {
            data.state.on_screen = true;
            if data.state.highlight != highlight {
                data.revealed = None;
            }
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
                data.revealed = None;
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
            Self::forget_lists(data);
        });
        self.load().await;
    }

    /// Cambia los filtros: se vuelve a la primera página de lo filtrado.
    pub async fn set_filter(&self, filter: RequestFilter) {
        if self.data().state.filter == filter {
            return;
        }
        self.change(|data| {
            data.state.filter = filter;
            Self::forget_lists(data);
        });
        self.load().await;
    }

    fn forget_lists(data: &mut RequestsData) {
        data.list_turn += 1;
        data.all_open.clear();
        data.open_next.clear();
        data.linked.clear();
        data.state.open_paging = RequestPaging::default();
        data.state.loaded = false;
        data.reset_closed();
    }

    /// Despliega o pliega «Resueltos». Desplegarlo la primera vez pide su
    /// primera página; plegado no se pide nada.
    pub async fn set_resolved_open(&self, open: bool) {
        let first = self.change_with(|data| {
            data.state.resolved_open = open;
            open && data.wants_closed() && !data.state.closed_paging.loaded && !data.state.closed_paging.loading
        });
        if first {
            self.closed_page(true).await;
        }
    }

    fn list_path(scope: RequestScope, filter: &RequestFilter, status: &str, limit: usize, cursor: &str, org: Option<&str>) -> String {
        let mut path = format!("/api/v1/requests?scope={}&status={status}{}&limit={limit}&cursor={}", scope.query(), filter.query(), crate::chat::query_escaped(cursor));
        if let Some(org) = org {
            path.push_str("&orgId=");
            path.push_str(&crate::chat::query_escaped(org));
        }
        path
    }

    async fn fetch(&self, path: &str) -> Result<(Vec<AgentRequest>, String), ApiError> {
        self.core.get::<PageWire>(path).await.map(PageWire::split)
    }

    /// Trae la primera página de los abiertos del filtro de ahora y, si
    /// «Resueltos» está desplegado, la primera de los de abajo. Es la carga
    /// entera: al entrar, al cambiar de filtro, al volver de una caída.
    pub async fn load(&self) {
        let (scope, filter, turn) = self.change_with(|data| {
            data.list_turn += 1;
            data.state.open_paging.loading = data.state.filter.status.shows_open();
            (data.state.scope, data.state.filter.clone(), data.list_turn)
        });
        let open = match filter.status.shows_open() {
            true => Some(self.fetch(&Self::list_path(scope, &filter, "open", OPEN_PAGE, "", None)).await),
            false => None,
        };
        let applied = self.change_with(|data| {
            if data.list_turn != turn || data.state.scope != scope || data.state.filter != filter {
                return false;
            }
            match open {
                Some(Err(_)) => {
                    data.state.failed = true;
                    data.state.open_paging.loading = false;
                    return true;
                }
                Some(Ok((requests, next))) => {
                    data.state.open_paging = RequestPaging { loaded: true, loading: false, failed: false, more: !next.is_empty() };
                    data.all_open = requests;
                    data.open_next = next;
                }
                None => {
                    data.state.open_paging = RequestPaging { loaded: true, ..Default::default() };
                    data.all_open.clear();
                    data.open_next.clear();
                }
            }
            data.state.loaded = true;
            data.state.failed = false;
            data.reset_closed();
            true
        });
        if !applied {
            return;
        }
        if self.data().wants_closed() {
            self.closed_page(true).await;
        }
        self.reveal().await;
    }

    /// La página siguiente de lo que se está leyendo: de los abiertos si les
    /// quedan, y si no, de los de abajo. Una sola petición por lista a la vez:
    /// el scroll pide muchas veces seguidas, y la segunda no sale.
    pub async fn load_more(&self) {
        let (open_more, closed_first) = {
            let data = self.data();
            let paging = data.state.open_paging;
            (data.state.filter.status.shows_open() && paging.loaded && paging.more, !data.state.closed_paging.loaded)
        };
        if open_more {
            self.open_page().await;
        } else if self.data().wants_closed() {
            self.closed_page(closed_first).await;
        }
    }

    async fn open_page(&self) {
        let Some((scope, filter, cursor, turn)) = self.change_with(|data| {
            let paging = &mut data.state.open_paging;
            if paging.loading || !paging.more || data.open_next.is_empty() {
                return None;
            }
            paging.loading = true;
            paging.failed = false;
            Some((data.state.scope, data.state.filter.clone(), data.open_next.clone(), data.list_turn))
        }) else {
            return;
        };
        let page = self.fetch(&Self::list_path(scope, &filter, "open", OPEN_PAGE, &cursor, None)).await;
        self.change(|data| {
            if data.list_turn != turn || data.open_next != cursor {
                return;
            }
            let paging = &mut data.state.open_paging;
            paging.loading = false;
            match page {
                Ok((requests, next)) => {
                    paging.more = !next.is_empty();
                    for request in requests {
                        if !data.all_open.iter().any(|listed| listed.code == request.code) {
                            data.all_open.push(request);
                        }
                    }
                    data.open_next = next;
                }
                Err(_) => paging.failed = true,
            }
        });
    }

    /// Una página de los de abajo: la primera (`first`) o la siguiente.
    async fn closed_page(&self, first: bool) {
        let here = self.core.orgs().state().current_org_id;
        let Some((scope, filter, status, cursor, turn)) = self.change_with(|data| {
            let paging = data.state.closed_paging;
            let status = data.state.filter.status.closed_query()?;
            if paging.loading || (!first && (!paging.more || data.closed_next.is_empty())) {
                return None;
            }
            if first {
                data.reset_closed();
            }
            data.closed_org = here.clone();
            data.state.closed_paging.loading = true;
            data.state.closed_paging.failed = false;
            Some((data.state.scope, data.state.filter.clone(), status, data.closed_next.clone(), data.closed_turn))
        }) else {
            return;
        };
        // Sin organización no hay de cuál pedirlos.
        let page = match here.as_deref() {
            Some(org) => self.fetch(&Self::list_path(scope, &filter, status, CLOSED_PAGE, &cursor, Some(org))).await,
            None => Ok((Vec::new(), String::new())),
        };
        self.change(|data| {
            if data.closed_turn != turn || data.closed_next != cursor {
                return;
            }
            let paging = &mut data.state.closed_paging;
            paging.loading = false;
            match page {
                Ok((requests, next)) => {
                    paging.loaded = true;
                    paging.more = !next.is_empty();
                    for request in requests {
                        match data.all_closed.iter_mut().find(|listed| listed.code == request.code) {
                            Some(listed) => *listed = request,
                            None => data.all_closed.push(request),
                        }
                    }
                    data.closed_next = next;
                }
                Err(_) => paging.failed = true,
            }
        });
    }

    /// El pedido al que llevó un enlace, si no está en lo cargado: se pide
    /// suelto por su código, y si está resuelto se despliega «Resueltos». Una
    /// vez por enlace: que luego se conteste no vuelve a abrir nada.
    async fn reveal(&self) {
        let Some(code) = self.change_with(|data| {
            let code = data.state.highlight.clone()?;
            if data.revealed.as_ref() == Some(&code) {
                return None;
            }
            data.revealed = Some(code.clone());
            if data.state.open.iter().any(|request| request.code == code) {
                return None;
            }
            if data.state.closed.iter().any(|request| request.code == code) {
                data.state.resolved_open = true;
                return None;
            }
            Some(code)
        }) else {
            return;
        };
        let Ok(request) = self.core.get::<AgentRequest>(&format!("/api/v1/requests/{}", crate::chat::query_escaped(&code))).await else { return };
        let first = self.change_with(|data| {
            if data.state.highlight.as_deref() != Some(code.as_str()) {
                return false;
            }
            let closed = request.status != RequestStatus::Open;
            data.linked.retain(|listed| listed.code != request.code);
            data.linked.push(request);
            if closed {
                data.state.resolved_open = true;
            }
            closed && data.wants_closed() && !data.state.closed_paging.loaded && !data.state.closed_paging.loading
        });
        if first {
            self.closed_page(true).await;
        }
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
        });
    }

    /// Si un pedido cerrado entra en la lista de abajo tal como se mira.
    fn closed_admits(&self, data: &RequestsData, request: &AgentRequest) -> bool {
        let mine = self.core.session().map(|session| session.id).unwrap_or_default();
        data.state.filter.status.admits(request.status)
            && data.state.filter.admits(request)
            && (data.state.scope == RequestScope::Team || request.requested_of.id == mine)
            && (data.closed_org.is_none() || data.closed_org.as_deref() == Some(request.org_id.as_str()))
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
        self.settle(done.clone());
        Ok(done)
    }

    /// Un pedido que se cerró: sale de los abiertos y entra arriba en los de
    /// abajo, si es de lo que se mira. Entra como suelto (arriba, mientras
    /// ninguna página lo traiga): así la primera página, si llega después, no
    /// se lo lleva.
    fn settle(&self, done: AgentRequest) {
        let admitted = {
            let data = self.data();
            self.closed_admits(&data, &done)
        };
        self.change(|data| {
            data.all_open.retain(|request| request.code != done.code);
            data.all_closed.retain(|request| request.code != done.code);
            data.linked.retain(|request| request.code != done.code || request.status != RequestStatus::Open);
            data.team_open.retain(|request| request.code != done.code);
            if admitted {
                data.linked.retain(|request| request.code != done.code);
                data.linked.push(done);
            }
        });
    }

    /// Abre la tarea del pedido, en su organización.
    pub async fn open_task(&self, request: &AgentRequest) -> bool {
        self.core.inbox().open(&format!("/tasks?task={}", request.task.id), &request.org_id).await
    }

    /// Un pedido nuevo o rehecho: se vuelven a pedir los abiertos, tantos
    /// como se tenían (no se pierde lo que el scroll ya trajo), sin tocar los
    /// de abajo. Si la lista nunca se pidió, sólo el número.
    async fn open_changed(&self) {
        let (loaded, scope, filter, turn, count) = self.change_with(|data| {
            data.list_turn += 1;
            (data.state.loaded && data.state.filter.status.shows_open(), data.state.scope, data.state.filter.clone(), data.list_turn, data.all_open.len())
        });
        if !loaded {
            self.refresh_marks().await;
            return;
        }
        let limit = count.clamp(OPEN_PAGE, MAX_PAGE);
        let path = Self::list_path(scope, &filter, "open", limit, "", None);
        let (open, _) = tokio::join!(self.fetch(&path), self.refresh_marks());
        self.change(|data| {
            if data.list_turn != turn || data.state.scope != scope || data.state.filter != filter {
                return;
            }
            if let Ok((requests, next)) = open {
                data.state.open_paging.more = !next.is_empty();
                data.all_open = requests;
                data.open_next = next;
            }
        });
    }

    /// Un pedido contestado o retirado en otro sitio: sale de los abiertos y,
    /// si los de abajo están cargados, se pide suelto para ponerlo arriba.
    async fn closed_elsewhere(&self, code: String) {
        let closed_loaded = self.change_with(|data| {
            data.all_open.retain(|request| request.code != code);
            data.state.closed_paging.loaded
        });
        let fetched = async {
            if !closed_loaded {
                return None;
            }
            self.core.get::<AgentRequest>(&format!("/api/v1/requests/{}", crate::chat::query_escaped(&code))).await.ok()
        };
        let (done, _) = tokio::join!(fetched, self.refresh_marks());
        if let Some(done) = done.filter(|done| done.status != RequestStatus::Open) {
            self.settle(done);
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
        // La lista sólo trae la organización que se mira: se cambia a la del
        // pedido antes de enseñarlo, o su tarjeta no estaría y se diría
        // «enseñé» de algo que nadie tiene delante.
        let Some(org_id) = self.org_of(&code).await else { return };
        if Some(org_id.as_str()) != self.core.orgs().state().current_org_id.as_deref() && !self.core.switch_org(&org_id).await {
            return;
        }
        if !self.show(Some(code.clone())) {
            return;
        }
        self.core.inner.listener.show_request(code.clone());
        let path = format!("/api/v1/requests/{code}/opened");
        let _ = self.core.call_ok("POST", &path, Some(serde_json::json!({ "nonce": ask.nonce }))).await;
        self.reload_with_list().await;
    }

    /// De qué organización es un pedido, mirando lo guardado y, si no está
    /// (uno recién creado), lo que dice el servidor.
    async fn org_of(&self, code: &str) -> Option<String> {
        let find = |data: &RequestsData| {
            data.team_open.iter().chain(&data.all_open).chain(&data.all_closed).chain(&data.linked).find(|request| request.code == code).map(|request| request.org_id.clone())
        };
        if let Some(org_id) = find(&self.data()) {
            return Some(org_id);
        }
        self.refresh_marks().await;
        find(&self.data())
    }

    async fn reload_with_list(&self) {
        tokio::join!(self.refresh_marks(), self.load());
    }

    pub(crate) fn server_event(&self, event: &ServerEvent) {
        match event.event.as_str() {
            "request:created" | "request:updated" => {
                let requests = self.core.requests();
                tokio::spawn(async move { requests.open_changed().await });
            }
            "request:answered" | "request:withdrawn" => {
                let code = serde_json::from_str::<OpenAsk>(&event.data).map(|ask| ask.code.to_lowercase()).unwrap_or_default();
                let requests = self.core.requests();
                tokio::spawn(async move { requests.closed_elsewhere(code).await });
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
        if self.data().state.loaded {
            self.reload_with_list().await;
        } else {
            self.refresh_marks().await;
        }
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
