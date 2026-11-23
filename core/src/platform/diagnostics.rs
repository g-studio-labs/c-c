//! Diagnóstico: la telemetría pasiva de las apps de los clientes
//! (`app/src/pages/Diagnostics.tsx`, `components/diagnostics/*`,
//! `store/telemetry.store.ts`, `lib/telemetry.ts`). Qué se manda y qué
//! entiende cac de ello: `docs/integrations/telemetry.md`.
//!
//! La gravedad de cada evento **no** se decide aquí: la manda el backend, la
//! misma que ve el MCP. El estado de un dispositivo y cada evento son JSON
//! libre que escribe su app; aquí viajan como texto y se despliegan en filas.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use super::{Platform, PlatformSection};

pub const DEVICE_PAGE: usize = 100;
pub const TIMELINE_PAGE: usize = 100;

/// Un JSON cualquiera, como texto compacto. Nulo o ausente es vacío.
fn json_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(value) => value.to_string(),
    })
}

fn json_texts<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let values = Option::<Vec<serde_json::Value>>::deserialize(deserializer)?;
    Ok(values.unwrap_or_default().into_iter().map(|value| value.to_string()).collect())
}

fn null_is_empty<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(deserializer: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Una regla del proyecto que el último estado incumple (la decide el
/// backend).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct HealthAlert {
    pub path: String,
    /// `warn` o `error`.
    pub severity: String,
    pub message: String,
    /// El valor que tenía, como JSON.
    #[serde(deserialize_with = "json_text")]
    pub actual: String,
}

impl HealthAlert {
    pub fn is_error(&self) -> bool {
        self.severity == "error"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceSummary {
    pub device_id: String,
    pub project_id: String,
    pub project_name: String,
    /// Cómo se llama el dispositivo. Vacío si su app no lo dice.
    pub label: String,
    /// Quién lo usa: un identificador, nunca un correo.
    pub subject: String,
    pub platform: String,
    pub app_version: String,
    pub batches: i64,
    pub req_count: i64,
    pub error_count: i64,
    pub warn_count: i64,
    pub last_seen: String,
    pub last_heartbeat_at: Option<String>,
    pub silent_since: Option<String>,
    #[serde(deserialize_with = "null_is_empty")]
    pub alerts: Vec<HealthAlert>,
    /// Sólo en la última fila de una página que tiene siguiente.
    pub cursor: Option<String>,
}

impl DeviceSummary {
    /// Cómo se le llama: su nombre, o el principio del id.
    pub fn title(&self) -> String {
        let label = self.label.trim();
        if label.is_empty() { short_id(&self.device_id) } else { label.to_string() }
    }

    pub fn reference(&self) -> DeviceRef {
        DeviceRef { project_id: self.project_id.clone(), device_id: self.device_id.clone() }
    }

    /// La peor de sus alertas, si tiene.
    pub fn worst(&self) -> Option<&'static str> {
        if self.alerts.iter().any(HealthAlert::is_error) {
            Some("error")
        } else if self.alerts.is_empty() {
            None
        } else {
            Some("warn")
        }
    }
}

pub fn short_id(id: &str) -> String {
    if id.chars().count() > 12 { format!("{}…", id.chars().take(8).collect::<String>()) } else { id.to_string() }
}

/// Un dispositivo se elige por proyecto **y** id: el mismo teléfono puede
/// mandar a dos proyectos, y elegir sólo por id mezclaba sus timelines.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DeviceRef {
    pub project_id: String,
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct HeartbeatConfig {
    pub interval_seconds: i64,
    pub grace_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct HeartbeatGap {
    pub from: String,
    pub to: String,
    pub seconds: i64,
    /// El que sigue abierto, el de ahora.
    pub open: bool,
}

/// La ficha: la última foto del dispositivo, sus alertas y sus latidos.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceDetail {
    #[serde(flatten)]
    pub summary: DeviceSummary,
    pub first_seen: String,
    pub session_id: String,
    pub unhealthy_since: Option<String>,
    /// El estado, como JSON; vacío si no ha mandado ninguno.
    #[serde(deserialize_with = "json_text")]
    pub device: String,
    pub heartbeat: Option<HeartbeatConfig>,
    /// Cuándo llegó cada latido guardado, el más viejo primero.
    #[serde(deserialize_with = "null_is_empty")]
    pub heartbeats: Vec<String>,
    #[serde(deserialize_with = "null_is_empty")]
    pub gaps: Vec<HeartbeatGap>,
}

/// Un lote, descifrado.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct TimelineBatch {
    pub id: String,
    pub session_id: String,
    pub received_at: String,
    /// Cada evento, como JSON.
    #[serde(deserialize_with = "json_texts")]
    pub breadcrumbs: Vec<String>,
    /// En paralelo a `breadcrumbs`: la gravedad de cada uno.
    #[serde(deserialize_with = "null_is_empty")]
    pub severities: Vec<String>,
    /// El lote existe pero no se pudo abrir.
    pub undecryptable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct TimelineFilters {
    pub types: Vec<String>,
    /// `info`, `warn` o `error`.
    pub min_severity: String,
    pub session_id: String,
    /// Desde hace cuántas horas; 0 es sin límite.
    pub since_hours: u32,
    /// Se filtra en pantalla, no en el servidor.
    pub text: String,
}

impl Default for TimelineFilters {
    fn default() -> Self {
        TimelineFilters { types: Vec::new(), min_severity: "info".into(), session_id: String::new(), since_hours: 0, text: String::new() }
    }
}

impl TimelineFilters {
    /// Si cambiar de `self` a `next` obliga a volver a pedir.
    fn asks_again(&self, next: &TimelineFilters) -> bool {
        TimelineFilters { text: String::new(), ..self.clone() } != TimelineFilters { text: String::new(), ..next.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsState {
    pub devices: Vec<DeviceSummary>,
    pub devices_cursor: Option<String>,
    pub query: String,
    pub loading_devices: bool,
    pub loaded: bool,
    pub error: Option<String>,
    pub selected: Option<DeviceRef>,
    pub detail: Option<DeviceDetail>,
    pub detail_error: Option<String>,
    pub timeline: Vec<TimelineBatch>,
    /// `receivedAt` del último lote, si puede haber más.
    pub timeline_before: Option<String>,
    pub loading_timeline: bool,
    pub timeline_error: Option<String>,
    pub filters: TimelineFilters,
}

/// La consulta del timeline. `now` en milisegundos.
pub fn timeline_path(device: &DeviceRef, filters: &TimelineFilters, before: Option<&str>, now: i64) -> String {
    use crate::chat::query_escaped as q;
    let mut query = vec![format!("deviceId={}", q(&device.device_id)), format!("projectId={}", q(&device.project_id)), format!("limit={TIMELINE_PAGE}")];
    if !filters.session_id.is_empty() {
        query.push(format!("sessionId={}", q(&filters.session_id)));
    }
    if !filters.types.is_empty() {
        query.push(format!("types={}", q(&filters.types.join(","))));
    }
    if filters.min_severity != "info" {
        query.push(format!("minSeverity={}", q(&filters.min_severity)));
    }
    if filters.since_hours > 0 {
        query.push(format!("since={}", now - i64::from(filters.since_hours) * 3_600_000));
    }
    if let Some(before) = before {
        query.push(format!("before={}", q(before)));
    }
    format!("/api/v1/telemetry/timeline?{}", query.join("&"))
}

fn millis(iso: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso).ok().map(|at| at.timestamp_millis())
}

/// Una fila del timeline plano.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineRow {
    pub key: String,
    /// Milisegundos: el `ts` del evento, o la llegada del lote si no lo trae.
    pub at: i64,
    /// El evento entero, como JSON.
    pub crumb: String,
    pub severity: String,
    pub kind: String,
    pub title: String,
    /// El `status` de una petición, si lo trae.
    pub status: Option<i64>,
    pub session_id: String,
    /// La primera fila (en orden de pantalla) de otra sesión.
    pub session_start: bool,
}

const NETWORK: [&str; 4] = ["network", "request", "fetch", "xhr"];

/// La línea de un evento.
fn crumb_title(crumb: &serde_json::Map<String, serde_json::Value>) -> String {
    let field = |name: &str| crumb.get(name).and_then(|value| value.as_str()).filter(|value| !value.is_empty()).map(str::to_string);
    let kind = field("type").unwrap_or_default();
    if NETWORK.contains(&kind.as_str()) {
        return [field("method"), field("url")].into_iter().flatten().collect::<Vec<_>>().join(" ");
    }
    field("message").or_else(|| field("name")).or_else(|| field("eventName")).or_else(|| field("type")).unwrap_or_else(|| "event".into())
}

/// Los lotes, en una sola lista por hora, la más nueva arriba. Por el `ts`
/// del evento y no por el lote: agrupado por lote, un error de las 10:05
/// salía debajo de una petición de las 10:01 del lote siguiente. A igual hora
/// se respeta el orden de llegada.
pub fn flatten_timeline(batches: &[TimelineBatch]) -> Vec<TimelineRow> {
    let mut rows: Vec<(TimelineRow, usize)> = Vec::new();
    for batch in batches {
        let fallback = millis(&batch.received_at).unwrap_or_default();
        for (index, raw) in batch.breadcrumbs.iter().enumerate() {
            let crumb = serde_json::from_str::<serde_json::Value>(raw).ok().and_then(|value| value.as_object().cloned()).unwrap_or_default();
            let at = crumb.get("ts").and_then(|ts| ts.as_f64()).filter(|ts| ts.is_finite()).map(|ts| ts as i64).unwrap_or(fallback);
            let row = TimelineRow {
                key: format!("{}:{index}", batch.id),
                at,
                crumb: raw.clone(),
                severity: batch.severities.get(index).cloned().unwrap_or_else(|| "info".into()),
                kind: crumb.get("type").and_then(|kind| kind.as_str()).unwrap_or("?").to_string(),
                title: crumb_title(&crumb),
                status: crumb.get("status").and_then(|status| status.as_i64()),
                session_id: batch.session_id.clone(),
                session_start: false,
            };
            let seq = rows.len();
            rows.push((row, seq));
        }
    }
    rows.sort_by(|(a, a_seq), (b, b_seq)| b.at.cmp(&a.at).then(b_seq.cmp(a_seq)));
    let mut previous: Option<String> = None;
    rows.into_iter()
        .map(|(mut row, _)| {
            row.session_start = previous.as_ref().is_some_and(|previous| *previous != row.session_id);
            previous = Some(row.session_id.clone());
            row
        })
        .collect()
}

/// Si una fila contiene el texto, en su línea o en su contenido.
pub fn matches_text(row: &TimelineRow, text: &str) -> bool {
    let wanted = text.trim().to_lowercase();
    wanted.is_empty() || row.crumb.to_lowercase().contains(&wanted)
}

/// Los tipos que han salido, para ofrecerlos como filtro.
pub fn crumb_types(batches: &[TimelineBatch]) -> Vec<String> {
    let mut kinds = BTreeSet::new();
    for raw in batches.iter().flat_map(|batch| batch.breadcrumbs.iter()) {
        if let Some(kind) = serde_json::from_str::<serde_json::Value>(raw).ok().and_then(|crumb| crumb.get("type").and_then(|kind| kind.as_str()).map(str::to_string)) {
            kinds.insert(kind);
        }
    }
    kinds.into_iter().collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum StripKind {
    Beat,
    Gap,
}

/// Un punto o un tramo de la tira de latidos, en fracciones de la ventana.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StripMark {
    pub left: f32,
    pub width: f32,
    pub kind: StripKind,
    pub open: bool,
    pub from: String,
    pub to: String,
}

/// La tira de latidos: un punto por latido y una banda por hueco, en la
/// ventana `[from, to]` (milisegundos). Lo que cae fuera se recorta.
pub fn heartbeat_strip(beats: &[String], gaps: &[HeartbeatGap], from: i64, to: i64) -> Vec<StripMark> {
    let span = (to - from) as f64;
    if span <= 0. {
        return Vec::new();
    }
    let place = |at: i64| (((at - from) as f64 / span).clamp(0., 1.)) as f32;
    let mut marks = Vec::new();
    for gap in gaps {
        let (Some(start), Some(end)) = (millis(&gap.from), millis(&gap.to)) else { continue };
        let (left, right) = (place(start), place(end));
        if right > left {
            marks.push(StripMark { left, width: right - left, kind: StripKind::Gap, open: gap.open, from: gap.from.clone(), to: gap.to.clone() });
        }
    }
    for beat in beats {
        let Some(at) = millis(beat) else { continue };
        if at < from || at > to {
            continue;
        }
        marks.push(StripMark { left: place(at), width: 0., kind: StripKind::Beat, open: false, from: beat.clone(), to: beat.clone() });
    }
    marks
}

/// Cuánto dura un hueco, en palabras cortas («2 h 5 min»).
pub fn gap_duration(seconds: i64) -> String {
    let minutes = (seconds as f64 / 60.).round() as i64;
    if minutes < 60 {
        return format!("{minutes} min");
    }
    let (hours, rest) = (minutes / 60, minutes % 60);
    if hours < 24 {
        return if rest > 0 { format!("{hours} h {rest} min") } else { format!("{hours} h") };
    }
    let days = hours / 24;
    if hours % 24 > 0 { format!("{days} d {} h", hours % 24) } else { format!("{days} d") }
}

/// De qué es una hoja del árbol, para su color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum KvKind {
    Branch,
    Null,
    True,
    False,
    Number,
    Text,
}

/// Una fila del árbol de clave y valor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct KvRow {
    /// La ruta con puntos (`snapshot.battery.level`).
    pub path: String,
    pub depth: u32,
    pub key: String,
    /// El valor de una hoja; en una rama cerrada, `{3}` o `[3]`.
    pub value: String,
    pub kind: KvKind,
    pub open: bool,
    /// `warn` o `error` si una regla incumplida apunta aquí.
    pub tone: Option<String>,
}

/// Un JSON como árbol plegable. Una rama sale abierta hasta `depth_open`, o si
/// guarda una ruta marcada: las de las reglas incumplidas se ven sin
/// buscarlas. `toggled` son las que se han pulsado, que dan la vuelta a eso.
pub fn kv_rows(json: &str, highlight: &[(String, String)], depth_open: u32, toggled: &[String]) -> Vec<KvRow> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut rows = Vec::new();
    match &value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                kv_walk(key, child, key.clone(), 0, highlight, depth_open, toggled, &mut rows);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                kv_walk(&index.to_string(), child, index.to_string(), 0, highlight, depth_open, toggled, &mut rows);
            }
        }
        leaf => rows.push(KvRow { path: String::new(), depth: 0, key: String::new(), value: leaf_text(leaf), kind: leaf_kind(leaf), open: false, tone: None }),
    }
    rows
}

fn leaf_kind(value: &serde_json::Value) -> KvKind {
    match value {
        serde_json::Value::Null => KvKind::Null,
        serde_json::Value::Bool(true) => KvKind::True,
        serde_json::Value::Bool(false) => KvKind::False,
        serde_json::Value::Number(_) => KvKind::Number,
        serde_json::Value::String(_) => KvKind::Text,
        _ => KvKind::Branch,
    }
}

fn leaf_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn kv_walk(key: &str, value: &serde_json::Value, path: String, depth: u32, highlight: &[(String, String)], depth_open: u32, toggled: &[String], rows: &mut Vec<KvRow>) {
    let tone = highlight.iter().filter(|(marked, _)| *marked == path).map(|(_, severity)| severity.clone()).max_by_key(|severity| severity == "error");
    let children: Vec<(String, &serde_json::Value)> = match value {
        serde_json::Value::Object(map) => map.iter().map(|(key, child)| (key.clone(), child)).collect(),
        serde_json::Value::Array(items) => items.iter().enumerate().map(|(index, child)| (index.to_string(), child)).collect(),
        leaf => {
            rows.push(KvRow { path, depth, key: key.to_string(), value: leaf_text(leaf), kind: leaf_kind(leaf), open: false, tone });
            return;
        }
    };
    let holds_mark = highlight.iter().any(|(marked, _)| marked.starts_with(&format!("{path}.")));
    let open = (depth < depth_open || holds_mark) != toggled.contains(&path);
    let size = if value.is_array() { format!("[{}]", children.len()) } else { format!("{{{}}}", children.len()) };
    rows.push(KvRow { path: path.clone(), depth, key: key.to_string(), value: if open { String::new() } else { size }, kind: KvKind::Branch, open, tone });
    if open {
        for (child_key, child) in children {
            kv_walk(&child_key, child, format!("{path}.{child_key}"), depth + 1, highlight, depth_open, toggled, rows);
        }
    }
}

fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

impl Platform {
    /// Abre Diagnóstico y pide los dispositivos, si aún no estaban.
    pub async fn open_diagnostics(&self) {
        if !self.show_section(PlatformSection::Diagnostics) {
            return;
        }
        if !self.data().diagnostics.as_ref().is_some_and(|state| state.loaded) {
            self.load_devices(false).await;
        }
    }

    /// Una página de dispositivos: la primera, o la siguiente con `more`. Lo
    /// que llega cuando ya se busca otra cosa no se pinta.
    pub async fn load_devices(&self, more: bool) {
        let mut query = String::new();
        let mut cursor = None;
        self.change(|data| {
            let state = data.diagnostics.get_or_insert_with(DiagnosticsState::default);
            query = state.query.clone();
            cursor = state.devices_cursor.clone();
            state.loading_devices = true;
            state.error = None;
        });
        if more && cursor.is_none() {
            self.change(|data| data.diagnostics.get_or_insert_with(DiagnosticsState::default).loading_devices = false);
            return;
        }
        use crate::chat::query_escaped as q;
        let mut route = format!("/api/v1/telemetry/devices?limit={DEVICE_PAGE}");
        if !query.trim().is_empty() {
            route.push_str(&format!("&q={}", q(query.trim())));
        }
        if let (true, Some(cursor)) = (more, cursor.as_ref()) {
            route.push_str(&format!("&cursor={}", q(cursor)));
        }
        let answer = self.core.get_list::<DeviceSummary>(&route).await;
        self.change(|data| {
            let Some(state) = data.diagnostics.as_mut().filter(|state| state.query == query) else { return };
            state.loading_devices = false;
            match answer {
                Ok(page) => {
                    state.devices_cursor = page.last().and_then(|device| device.cursor.clone());
                    if more {
                        state.devices.extend(page);
                    } else {
                        state.devices = page;
                    }
                    state.loaded = true;
                }
                Err(error) => state.error = Some(error.to_string()),
            }
        });
    }

    /// Busca en el servidor por nombre, persona o id.
    pub async fn search_devices(&self, query: &str) {
        self.change(|data| {
            let state = data.diagnostics.get_or_insert_with(DiagnosticsState::default);
            state.query = query.to_string();
            state.devices_cursor = None;
        });
        self.load_devices(false).await;
    }

    /// Elige un dispositivo: su ficha y su timeline, a la vez. La sesión del
    /// filtro es del anterior y no se lleva.
    pub async fn select_device(&self, device: DeviceRef) {
        self.change(|data| {
            let state = data.diagnostics.get_or_insert_with(DiagnosticsState::default);
            state.selected = Some(device.clone());
            state.detail = None;
            state.detail_error = None;
            state.timeline.clear();
            state.timeline_before = None;
            state.timeline_error = None;
            state.filters.session_id.clear();
        });
        let core = self.core.clone();
        let route = format!("/api/v1/telemetry/devices/{}/{}", crate::chat::query_escaped(&device.project_id), crate::chat::query_escaped(&device.device_id));
        let detail = tokio::spawn(async move { core.get::<DeviceDetail>(&route).await });
        self.load_timeline(false).await;
        let detail = detail.await.map_err(|error| error.to_string()).and_then(|answer| answer.map_err(|error| error.to_string()));
        self.change(|data| {
            let Some(state) = data.diagnostics.as_mut().filter(|state| state.selected.as_ref() == Some(&device)) else { return };
            match detail {
                Ok(detail) => state.detail = Some(detail),
                Err(error) => state.detail_error = Some(error),
            }
        });
    }

    /// Cambia los filtros del timeline. El texto se filtra en pantalla: sólo
    /// él no vuelve a pedir.
    pub async fn set_timeline_filters(&self, filters: TimelineFilters) {
        let mut again = false;
        self.change(|data| {
            let state = data.diagnostics.get_or_insert_with(DiagnosticsState::default);
            again = state.filters.asks_again(&filters);
            state.filters = filters;
        });
        if again {
            self.load_timeline(false).await;
        }
    }

    pub async fn load_more_timeline(&self) {
        self.load_timeline(true).await;
    }

    async fn load_timeline(&self, more: bool) {
        let Some(state) = self.data().diagnostics.clone() else { return };
        let Some(device) = state.selected.clone() else { return };
        let before = if more { state.timeline_before.clone() } else { None };
        if more && before.is_none() {
            return;
        }
        let filters = state.filters.clone();
        self.change(|data| {
            if let Some(state) = data.diagnostics.as_mut() {
                state.loading_timeline = true;
                state.timeline_error = None;
                if !more {
                    state.timeline.clear();
                    state.timeline_before = None;
                }
            }
        });
        let answer = self.core.get_list::<TimelineBatch>(&timeline_path(&device, &filters, before.as_deref(), now_millis())).await;
        self.change(|data| {
            // Sólo si nadie cambió la selección ni los filtros mientras tanto.
            let Some(state) = data.diagnostics.as_mut().filter(|state| state.selected.as_ref() == Some(&device) && !state.filters.asks_again(&filters)) else { return };
            state.loading_timeline = false;
            match answer {
                Ok(page) => {
                    // El servidor limita por lotes: una página llena puede
                    // tener más detrás.
                    state.timeline_before = if page.len() >= TIMELINE_PAGE { page.last().map(|batch| batch.received_at.clone()) } else { None };
                    state.timeline.extend(page);
                }
                Err(error) => state.timeline_error = Some(error.to_string()),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(iso: &str) -> i64 {
        millis(iso).unwrap()
    }

    /// Rotura que atrapa: que la tira no marque un hueco, lo ponga fuera de
    /// su sitio, no distinga el que sigue abierto, o pinte latidos de fuera
    /// de la ventana.
    #[test]
    fn the_heartbeat_strip_marks_the_gaps() {
        let beats = ["2026-11-20T09:00:00Z", "2026-11-20T10:00:00Z", "2026-11-20T11:00:00Z", "2026-11-20T08:00:00Z"].map(String::from);
        let gaps = [
            HeartbeatGap { from: "2026-11-20T11:00:00Z".into(), to: "2026-11-20T12:00:00Z".into(), seconds: 3600, open: false },
            HeartbeatGap { from: "2026-11-20T12:30:00Z".into(), to: "2026-11-20T13:00:00Z".into(), seconds: 1800, open: true },
        ];
        let marks = heartbeat_strip(&beats, &gaps, at("2026-11-20T09:00:00Z"), at("2026-11-20T13:00:00Z"));
        let gaps: Vec<_> = marks.iter().filter(|mark| mark.kind == StripKind::Gap).collect();
        assert_eq!(gaps.len(), 2);
        assert_eq!((gaps[0].left, gaps[0].width, gaps[0].open), (0.5, 0.25, false));
        assert_eq!((gaps[1].left, gaps[1].width, gaps[1].open), (0.875, 0.125, true));
        let beats: Vec<f32> = marks.iter().filter(|mark| mark.kind == StripKind::Beat).map(|mark| mark.left).collect();
        assert_eq!(beats, [0., 0.25, 0.5], "un latido de antes de la ventana se pintó");
    }

    /// Rotura que atrapa: ordenar por lote y no por la hora de cada evento,
    /// no marcar dónde empieza otra sesión, o perder la gravedad que manda el
    /// backend.
    #[test]
    fn the_timeline_goes_by_the_time_of_each_event() {
        let batches = [
            TimelineBatch { id: "b2".into(), session_id: "s2".into(), received_at: "2026-11-20T10:10:00Z".into(), breadcrumbs: vec![r#"{"type":"network","method":"GET","url":"/a","status":500,"ts":1795168860000}"#.into()], severities: vec!["error".into()], undecryptable: false },
            TimelineBatch { id: "b1".into(), session_id: "s1".into(), received_at: "2026-11-20T10:06:00Z".into(), breadcrumbs: vec![r#"{"type":"error","message":"boom","ts":1795169100000}"#.into(), r#"{"type":"lifecycle","name":"resume"}"#.into()], severities: vec!["error".into(), "info".into()], undecryptable: false },
        ];
        let rows = flatten_timeline(&batches);
        assert_eq!(rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(), ["resume", "boom", "GET /a"]);
        assert_eq!(rows.iter().map(|row| row.severity.as_str()).collect::<Vec<_>>(), ["info", "error", "error"]);
        assert_eq!(rows.iter().map(|row| row.session_start).collect::<Vec<_>>(), [false, false, true]);
        assert_eq!(rows[2].status, Some(500));
    }

    /// Rotura que atrapa: que la consulta no lleve los filtros que se
    /// eligieron (o lleve `minSeverity=info`, que es el de siempre), o que
    /// «desde hace N horas» se cuente en otra unidad.
    #[test]
    fn the_timeline_query_carries_the_filters() {
        let device = DeviceRef { project_id: "p1".into(), device_id: "d 1".into() };
        assert_eq!(timeline_path(&device, &TimelineFilters::default(), None, 0), "/api/v1/telemetry/timeline?deviceId=d%201&projectId=p1&limit=100");
        let filters = TimelineFilters { types: vec!["error".into(), "network".into()], min_severity: "warn".into(), session_id: "s1".into(), since_hours: 6, text: "x".into() };
        assert_eq!(
            timeline_path(&device, &filters, Some("2026-11-20T10:00:00Z"), 100_000_000),
            "/api/v1/telemetry/timeline?deviceId=d%201&projectId=p1&limit=100&sessionId=s1&types=error%2Cnetwork&minSeverity=warn&since=78400000&before=2026-11-20T10%3A00%3A00Z"
        );
    }

    /// Rotura que atrapa: que una regla incumplida quede escondida en una
    /// rama cerrada, que pulsar no la pliegue, o que la hoja no lleve su
    /// marca.
    #[test]
    fn a_broken_rule_is_seen_without_looking_for_it() {
        let json = r#"{"battery":{"level":4,"saver":true},"gps":{"on":false}}"#;
        let marks = [("battery.level".to_string(), "error".to_string())];
        let rows = kv_rows(json, &marks, 0, &[]);
        let paths: Vec<&str> = rows.iter().map(|row| row.path.as_str()).collect();
        assert_eq!(paths, ["battery", "battery.level", "battery.saver", "gps"]);
        assert_eq!(rows[1].tone.as_deref(), Some("error"));
        assert_eq!((rows[3].open, rows[3].value.as_str()), (false, "{1}"));
        let folded = kv_rows(json, &marks, 0, &["battery".to_string()]);
        assert_eq!(folded.len(), 2);
    }

    /// Rotura que atrapa: escribir «1 h 0 min», o pasar de horas a días sin
    /// las horas que sobran.
    #[test]
    fn a_gap_is_said_in_short_words() {
        assert_eq!(gap_duration(300), "5 min");
        assert_eq!(gap_duration(3600), "1 h");
        assert_eq!(gap_duration(7500), "2 h 5 min");
        assert_eq!(gap_duration(90_000), "1 d 1 h");
    }
}
