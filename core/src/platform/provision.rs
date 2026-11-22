//! Provisioning de un servidor (`ServerProvision.tsx`, `ServerConfigPanel`,
//! `ServerTemplateDialog`, `ProvisioningRequests.tsx` y `ansible-vars.tsx`):
//! la config versionada que vive en cac, las plantillas de la organización,
//! lo que se le aplicó y las peticiones de ejecución del protocolo (decisión
//! 4d28f4ab).
//!
//! Aquí sólo hay lo que se pide al backend y las reglas; correr Ansible es de
//! `provision_run.rs`, con el motor que pone quien usa el core. **Nunca un
//! valor secreto**: los secretos viajan como referencia `op://`, y el backend
//! rechaza una variable que lo parezca.
//!
//! Los mapas de JSON (variables) se guardan como texto JSON con las claves
//! ordenadas: así cruzan UniFFI, y una config sin tocar no parece cambiada
//! porque Postgres devuelva sus claves en otro orden.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use super::{Platform, Server};
use crate::error::ApiError;
use crate::texts::{error_phrase, text, text_with};
use crate::Locale;

// ─── JSON como texto ─────────────────────────────────────────────────────────

/// El mismo valor con las claves de cada objeto ordenadas. Hace falta aunque
/// serde_json ordene solo: con `preserve_order` (que trae el MCP al mismo
/// binario) guarda el orden de llegada.
pub fn sorted_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|key| (key.clone(), sorted_json(&map[key]))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted_json).collect()),
        other => other.clone(),
    }
}

/// JSON con las claves ordenadas y sangrado de dos espacios: el que se
/// enseña y el que se compara.
pub fn stable_json(value: &Value) -> String {
    serde_json::to_string_pretty(&sorted_json(value)).unwrap_or_default()
}

/// Un objeto JSON escrito como texto, o vacío si no lo es.
pub fn json_object(text: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn object_text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(value @ Value::Object(_)) => stable_json(&value),
        _ => "{}".into(),
    })
}

fn object_value<S: Serializer>(text: &str, s: S) -> Result<S::Ok, S::Error> {
    Value::Object(json_object(text)).serialize(s)
}

fn optional_json_text<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => None,
        Some(value) => Some(serde_json::to_string(&sorted_json(&value)).unwrap_or_default()),
    })
}

fn optional_json_value<S: Serializer>(text: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    text.as_deref().and_then(|text| serde_json::from_str::<Value>(text).ok()).serialize(s)
}

/// Go escribe `null` para una lista vacía.
fn null_vec<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

fn refs_of<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<SecretRef>, D::Error> {
    let map = Option::<std::collections::BTreeMap<String, String>>::deserialize(d)?.unwrap_or_default();
    Ok(map.into_iter().map(|(name, reference)| SecretRef { name, reference }).collect())
}

fn refs_map<S: Serializer>(refs: &[SecretRef], s: S) -> Result<S::Ok, S::Error> {
    refs.iter().map(|r| (r.name.clone(), r.reference.clone())).collect::<std::collections::BTreeMap<_, _>>().serialize(s)
}

// ─── Lo que contesta el backend ──────────────────────────────────────────────

/// Un secreto por referencia: su nombre y dónde está en 1Password.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SecretRef {
    pub name: String,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ConfigOrigin {
    pub server_id: String,
    pub seq: i64,
}

/// Una versión de la config de un servidor (`ServerConfigVersionResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ServerConfigVersion {
    pub id: String,
    pub created_at: String,
    pub server_id: String,
    pub seq: i64,
    pub template_id: String,
    pub template_seq: i64,
    #[serde(deserialize_with = "object_text", serialize_with = "object_value")]
    pub vars: String,
    #[serde(deserialize_with = "refs_of", serialize_with = "refs_map")]
    pub op_refs: Vec<SecretRef>,
    pub created_by: String,
    pub created_by_name: String,
    /// `app` o `mcp`.
    pub source: String,
    pub note: String,
    pub based_on: Option<ConfigOrigin>,
}

impl ServerConfigVersion {
    /// Lo que se compara entre versiones: plantilla, variables y referencias.
    pub fn as_text(&self) -> String {
        config_text(&self.template_id, &json_object(&self.vars), &self.op_refs)
    }
}

fn config_text(template_id: &str, vars: &Map<String, Value>, refs: &[SecretRef]) -> String {
    let refs: Map<String, Value> = refs.iter().map(|r| (r.name.clone(), Value::String(r.reference.clone()))).collect();
    stable_json(&serde_json::json!({ "templateId": template_id, "vars": vars, "opRefs": refs }))
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct ServerConfigSummary {
    pub current: Option<ServerConfigVersion>,
    #[serde(deserialize_with = "null_vec")]
    pub versions: Vec<ServerConfigVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ServerConnection {
    pub name: String,
    pub host: String,
    pub ssh_port: i64,
    pub ssh_user: String,
}

/// Lo que se aplicaría ahora (`EffectiveConfig`): la plantilla combinada con
/// lo del servidor, y la conexión.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct EffectiveConfig {
    pub server_id: String,
    pub config_seq: i64,
    pub template_id: String,
    pub template_name: String,
    pub template_seq: i64,
    #[serde(deserialize_with = "null_vec")]
    pub playbooks: Vec<String>,
    #[serde(deserialize_with = "object_text", serialize_with = "object_value")]
    pub vars: String,
    #[serde(deserialize_with = "refs_of", serialize_with = "refs_map")]
    pub op_refs: Vec<SecretRef>,
    #[serde(deserialize_with = "null_vec")]
    pub requires: Vec<String>,
    #[serde(deserialize_with = "null_vec")]
    pub per_server: Vec<String>,
    /// Los que la plantilla pide y el servidor aún no tiene.
    #[serde(deserialize_with = "null_vec")]
    pub missing: Vec<String>,
    pub connection: ServerConnection,
}

impl EffectiveConfig {
    /// Como la escribió el backend: lo que lee el motor de Ansible.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ServerTemplateVersion {
    pub id: String,
    pub created_at: String,
    pub template_id: String,
    pub seq: i64,
    #[serde(deserialize_with = "null_vec")]
    pub playbooks: Vec<String>,
    #[serde(deserialize_with = "object_text", serialize_with = "object_value")]
    pub vars: String,
    #[serde(deserialize_with = "null_vec")]
    pub requires: Vec<String>,
    #[serde(deserialize_with = "null_vec")]
    pub per_server: Vec<String>,
    pub created_by_name: String,
    pub source: String,
    pub note: String,
}

/// Una plantilla de la organización con su última versión.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ServerTemplate {
    pub id: String,
    pub org_id: String,
    pub name: String,
    pub description: String,
    pub current: Option<ServerTemplateVersion>,
}

/// Algo que se le aplicó (`ProvisioningRunResponse`). Nunca un valor: de las
/// variables, los nombres; de lo aplicado, las variables ya validadas y los
/// secretos por referencia.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ProvisionRun {
    pub id: String,
    pub kind: String,
    pub project: String,
    pub playbook: String,
    pub target: String,
    pub var_names: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub exit_code: Option<i64>,
    /// `running`, `succeeded`, `failed`, `cancelled` o `interrupted`.
    pub status: String,
    pub summary: String,
    pub config_seq: i64,
    /// `app` o `cli`.
    pub source: String,
    #[serde(deserialize_with = "optional_json_text", serialize_with = "optional_json_value")]
    pub applied: Option<String>,
    pub request_id: String,
    pub started_by_name: String,
}

impl ProvisionRun {
    /// Lo aplicado, para leerlo.
    pub fn applied_text(&self) -> Option<String> {
        self.applied.as_deref().and_then(|text| serde_json::from_str::<Value>(text).ok()).map(|value| stable_json(&value))
    }

    pub fn status_key(&self) -> String {
        format!("common:provision.status.{}", self.status)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum FlagLevel {
    Red,
    Amber,
    #[default]
    #[serde(other)]
    Info,
}

/// Una cosa que la evaluación del ensayo encontró.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct RiskFlag {
    pub code: String,
    pub level: FlagLevel,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct ProvisioningFailure {
    pub host: String,
    pub task: String,
    pub message: String,
}

/// Lo que dijo el ensayo (`--check --diff`), ya tachado.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ProvisioningCheck {
    #[serde(deserialize_with = "null_vec")]
    pub hosts: Vec<String>,
    pub ok: i64,
    pub changed: i64,
    pub unreachable: i64,
    pub failed: i64,
    #[serde(deserialize_with = "null_vec")]
    pub changed_tasks: Vec<String>,
    #[serde(deserialize_with = "null_vec")]
    pub paths: Vec<String>,
    pub diff: String,
    pub exit_code: i64,
    #[serde(deserialize_with = "null_vec")]
    pub failures: Vec<ProvisioningFailure>,
    pub log_tail: String,
}

/// El riesgo que declara un playbook en `cac.playbooks.yml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Read,
    #[default]
    Change,
    Destructive,
}

impl Risk {
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Read => "read",
            Risk::Change => "change",
            Risk::Destructive => "destructive",
        }
    }

    pub fn parse(text: &str) -> Option<Risk> {
        match text {
            "read" => Some(Risk::Read),
            "change" => Some(Risk::Change),
            "destructive" => Some(Risk::Destructive),
            _ => None,
        }
    }
}

/// Una petición de ejecución (`ProvisioningRequestResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ProvisioningRequest {
    pub id: String,
    pub created_at: String,
    pub server_id: String,
    pub playbook: String,
    pub project: String,
    pub risk: Risk,
    pub verify: String,
    pub rollback: String,
    pub config_seq: i64,
    pub reason: String,
    pub requested_by: String,
    /// `mcp` o `app`.
    pub source: String,
    /// El estado tal como se lee ahora (una autorización pasada de hora
    /// dice `expired`).
    pub status: String,
    pub check: Option<ProvisioningCheck>,
    #[serde(deserialize_with = "null_vec")]
    pub flags: Vec<RiskFlag>,
    pub decided_by: String,
    pub decision_note: String,
    pub auto_approved: bool,
    pub grant_expires_at: Option<String>,
    pub run_id: String,
    pub verify_run_id: String,
    pub requested_by_name: String,
    pub decided_by_name: String,
}

impl ProvisioningRequest {
    pub fn has_red_flag(&self) -> bool {
        self.flags.iter().any(|flag| flag.level == FlagLevel::Red)
    }

    /// Puede seguir moviéndose: se relee sola.
    pub fn is_live(&self) -> bool {
        LIVE_STATUSES.contains(&self.status.as_str())
    }
}

// ─── Las reglas de la bandeja ────────────────────────────────────────────────

/// Los estados en los que algo puede seguir moviéndose.
pub const LIVE_STATUSES: [&str; 5] = ["checking", "awaiting_approval", "approved", "running", "verifying"];

/// Pasado esto, una petición que sigue «ensayando» ya no la ensaya nadie: la
/// app se cerró, se cayó la red o el backend rechazó el ensayo. Un ensayo de
/// etapa tarda unos minutos; quince es holgado.
pub const STALE_CHECK_SECONDS: i64 = 15 * 60;

/// De las cerradas, cuántas se ven sin pedirlo.
pub const RECENT_CLOSED: usize = 3;

/// Lo que dura una autorización (`ProvisioningGrantTTL`), para decirlo.
pub const GRANT_MINUTES: i64 = 15;

/// Se quedó sin ensayo: se enseña así y se puede descartar.
pub fn is_stale(request: &ProvisioningRequest, now: chrono::DateTime<chrono::Utc>) -> bool {
    request.status == "checking"
        && chrono::DateTime::parse_from_rfc3339(&request.created_at).is_ok_and(|at| (now - at.with_timezone(&chrono::Utc)).num_seconds() > STALE_CHECK_SECONDS)
}

/// Lo que se ve de la bandeja: todas las abiertas y las últimas cerradas; el
/// resto, tras «Ver las anteriores». Con todas a la vista, las descartadas de
/// un simulacro empujaban el playbook fuera de la pantalla (jose, 2-nov-2026).
pub fn visible_requests(all: &[ProvisioningRequest], show_all: bool) -> (Vec<ProvisioningRequest>, usize) {
    let closed: Vec<&ProvisioningRequest> = all.iter().filter(|r| !r.is_live()).collect();
    let keep: Vec<&str> = closed.iter().take(if show_all { usize::MAX } else { RECENT_CLOSED }).map(|r| r.id.as_str()).collect();
    let shown = all.iter().filter(|r| r.is_live() || keep.contains(&r.id.as_str())).cloned().collect();
    (shown, closed.len() - keep.len())
}

/// Lo que hace falta para aprobar, según el protocolo: el nombre del
/// servidor escrito a mano si es destructivo, y reconocer las banderas rojas
/// si las hay. El backend lo vuelve a exigir.
pub fn can_approve(request: &ProvisioningRequest, server_name: &str, typed: &str, acknowledged: bool) -> bool {
    if request.risk == Risk::Destructive && typed != server_name {
        return false;
    }
    !(request.has_red_flag() && !acknowledged)
}

/// El comando con el que se corre una autorizada desde una terminal.
pub fn grant_command(server_name: &str, request_id: &str) -> String {
    format!("cac provision run --server '{}' --grant {request_id}", server_name.replace('\'', "'\\''"))
}

// ─── El manifiesto ───────────────────────────────────────────────────────────

/// Una variable de un playbook (`VarDef` del motor).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct PlaybookVar {
    pub name: String,
    pub description: String,
    /// v1: una referencia `op://`. v2: el nombre de un secreto de la config.
    pub from: Option<String>,
    /// El valor por defecto, como JSON.
    #[serde(deserialize_with = "optional_json_text", serialize_with = "optional_json_value")]
    pub default: Option<String>,
    pub secret: bool,
    pub required: bool,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

impl PlaybookVar {
    fn default_value(&self) -> Option<Value> {
        self.default.as_deref().and_then(|text| serde_json::from_str(text).ok()).filter(|value: &Value| !value.is_null())
    }
}

/// Un playbook del manifiesto (`PlaybookDef` del motor).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct PlaybookInfo {
    pub id: String,
    pub file: String,
    pub name: String,
    pub description: String,
    pub hosts: Option<String>,
    #[serde(rename = "become")]
    pub become_sudo: bool,
    pub become_password_ref: Option<String>,
    pub become_password_from: Option<String>,
    pub vars: Vec<PlaybookVar>,
    pub risk: Option<String>,
    pub verify: Option<String>,
    pub rollback: Option<String>,
}

/// `cac.playbooks.yml`, o lo que se dedujo del repo sin él.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct AnsibleManifest {
    pub version: i64,
    pub venv: Option<String>,
    pub inventory: String,
    pub playbooks: Vec<PlaybookInfo>,
    pub discovered: bool,
}

impl AnsibleManifest {
    pub fn playbook(&self, id: &str) -> Option<&PlaybookInfo> {
        self.playbooks.iter().find(|playbook| playbook.id == id)
    }

    /// El de la config en cac, con el protocolo.
    pub fn is_v2(&self) -> bool {
        self.version == 2
    }
}

/// Una variable para el formulario: nombre, para qué es y su default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct VarField {
    pub name: String,
    pub description: String,
    /// Como JSON.
    pub default: Option<String>,
    pub required: bool,
    /// Un campo por elemento.
    pub list: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SecretField {
    pub name: String,
    pub description: String,
    /// La contraseña de sudo: de cada máquina por defecto.
    pub sudo: bool,
}

/// Las variables de unos playbooks, para pintarlas como formulario: las que
/// traen default son de la plantilla (`shared`); las que no, de cada
/// servidor (`per_server`); los secretos, por nombre.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct VarFields {
    pub shared: Vec<VarField>,
    pub per_server: Vec<VarField>,
    pub secrets: Vec<SecretField>,
}

pub fn var_fields(manifest: &AnsibleManifest, playbook_ids: &[String]) -> VarFields {
    let mut fields: Vec<VarField> = Vec::new();
    let mut secrets: Vec<SecretField> = Vec::new();
    for playbook in playbook_ids.iter().filter_map(|id| manifest.playbook(id)) {
        for var in &playbook.vars {
            if let Some(from) = &var.from {
                match secrets.iter_mut().find(|s| &s.name == from) {
                    Some(known) if known.description.is_empty() => known.description = var.description.clone(),
                    Some(_) => {}
                    None => secrets.push(SecretField { name: from.clone(), description: var.description.clone(), sudo: false }),
                }
                continue;
            }
            if var.secret {
                continue;
            }
            let default = var.default_value();
            let list = var.kind.as_deref() == Some("list") || matches!(default, Some(Value::Array(_)));
            let default_text = default.map(|value| value.to_string());
            match fields.iter_mut().find(|f| f.name == var.name) {
                Some(known) => {
                    if known.description.is_empty() {
                        known.description = var.description.clone();
                    }
                    if known.default.is_none() {
                        known.default = default_text;
                    }
                    known.required |= var.required;
                    known.list |= list;
                }
                None => fields.push(VarField { name: var.name.clone(), description: var.description.clone(), default: default_text, required: var.required, list }),
            }
        }
        if let (true, Some(from)) = (playbook.become_sudo, &playbook.become_password_from) {
            match secrets.iter_mut().find(|s| &s.name == from) {
                Some(known) => known.sudo = true,
                None => secrets.push(SecretField { name: from.clone(), description: String::new(), sudo: true }),
            }
        }
    }
    let (shared, mut per_server): (Vec<VarField>, Vec<VarField>) = fields.into_iter().partition(|f| f.default.is_some());
    per_server.sort_by_key(|f| !f.required);
    secrets.sort_by(|a, b| a.name.cmp(&b.name));
    VarFields { shared, per_server, secrets }
}

/// Las etapas que se pueden poner en una plantilla: lo que monta algo
/// (`change`). Una verificación corre sola después de su etapa y un
/// desmontaje se pide aparte. Un manifiesto sin riesgos declarados enseña
/// todo.
pub fn stage_playbooks(manifest: &AnsibleManifest) -> Vec<PlaybookInfo> {
    let declared = manifest.playbooks.iter().any(|p| p.risk.is_some());
    manifest.playbooks.iter().filter(|p| !declared || p.risk.as_deref() == Some("change")).cloned().collect()
}

/// Los elegidos, en el orden del manifiesto: el de las etapas.
pub fn ordered_playbooks(manifest: &AnsibleManifest, picked: &[String]) -> Vec<String> {
    manifest.playbooks.iter().map(|p| p.id.clone()).filter(|id| picked.contains(id)).collect()
}

/// Lo que se guarda de una plantilla: cada variable de las etapas con su
/// valor (el default si no se tocó) y lo que ya tenía que no sale en el
/// formulario; los secretos que piden las etapas; y de ésos, los que son de
/// cada servidor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TemplateDraft {
    pub name: String,
    pub description: String,
    pub playbooks: Vec<String>,
    /// Lo escrito encima de los defaults, y al editar, lo que ya tenía.
    pub values: Map<String, Value>,
    /// `None`: lo que diga el manifiesto (la de sudo, de cada servidor).
    pub per_server: Option<Vec<String>>,
    pub note: String,
}

impl TemplateDraft {
    pub fn of(template: &ServerTemplate) -> TemplateDraft {
        let current = template.current.clone().unwrap_or_default();
        TemplateDraft {
            name: template.name.clone(),
            description: template.description.clone(),
            playbooks: current.playbooks.clone(),
            values: json_object(&current.vars),
            per_server: template.current.as_ref().map(|current| current.per_server.clone()),
            note: String::new(),
        }
    }

    /// Los secretos que son de cada servidor, como quedan.
    pub fn own(&self, fields: &VarFields) -> Vec<String> {
        self.per_server.clone().unwrap_or_else(|| fields.secrets.iter().filter(|s| s.sudo).map(|s| s.name.clone()).collect())
    }

    /// El cuerpo para `POST`/`PUT` de una plantilla.
    pub fn body(&self, manifest: &AnsibleManifest, creating: bool) -> Value {
        let playbooks = ordered_playbooks(manifest, &self.playbooks);
        let fields = var_fields(manifest, &playbooks);
        let shown: Vec<&str> = fields.shared.iter().chain(&fields.per_server).map(|f| f.name.as_str()).collect();
        let mut vars = Map::new();
        for (key, value) in &self.values {
            if !shown.contains(&key.as_str()) && !value.is_null() {
                vars.insert(key.clone(), value.clone());
            }
        }
        for field in &fields.shared {
            let value = self.values.get(&field.name).cloned().or_else(|| field.default.as_deref().and_then(|d| serde_json::from_str(d).ok()));
            if let Some(value) = value.filter(|v| !v.is_null()) {
                vars.insert(field.name.clone(), value);
            }
        }
        let requires: Vec<String> = fields.secrets.iter().map(|s| s.name.clone()).collect();
        let mut own: Vec<String> = self.own(&fields).into_iter().filter(|name| requires.contains(name)).collect();
        own.sort();
        let mut body = serde_json::json!({ "playbooks": playbooks, "vars": sorted_json(&Value::Object(vars)), "requires": requires, "perServer": own, "note": self.note.trim() });
        if creating {
            body["name"] = Value::String(self.name.trim().to_string());
            body["description"] = Value::String(self.description.trim().to_string());
        }
        body
    }

    pub fn can_save(&self, creating: bool) -> bool {
        !self.playbooks.is_empty() && (!creating || !self.name.trim().is_empty())
    }
}

/// Un valor como se escribe en un campo de texto.
pub fn to_input(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Lo escrito, de vuelta a su tipo. Se parece al default si lo hay (con un
/// default de texto sigue siendo texto aunque sean cifras: «0755»); sin
/// default, un número es número, `true`/`false` booleano y `[…]`/`{…}` JSON
/// si se lee. Vacío es `None`: no se guarda.
pub fn from_input(text: &str, like: Option<&Value>) -> Option<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let numeric = trimmed.parse::<f64>().ok().filter(|_| trimmed.bytes().all(|b| b.is_ascii_digit() || b == b'-' || b == b'.'));
    if matches!(like, Some(Value::Number(_))) || (like.is_none() && numeric.is_some()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Number>(trimmed) {
            return Some(Value::Number(value));
        }
    }
    if matches!(like, Some(Value::Bool(_)) | None) {
        match trimmed {
            "true" => return Some(Value::Bool(true)),
            "false" => return Some(Value::Bool(false)),
            _ => {}
        }
    }
    let structured = matches!(like, Some(Value::Array(_) | Value::Object(_))) || (like.is_none() && (trimmed.starts_with('[') || trimmed.starts_with('{')));
    if structured {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            return Some(value);
        }
    }
    Some(Value::String(text.to_string()))
}

/// Las filas de una lista, de vuelta a lista: las vacías no cuentan, y cada
/// fila entera es un elemento (una llave SSH lleva espacios).
pub fn from_items(rows: &[String], like: Option<&Value>) -> Option<Value> {
    let first = match like {
        Some(Value::Array(items)) => items.first().cloned(),
        _ => None,
    }
    .unwrap_or(Value::String(String::new()));
    let items: Vec<Value> = rows.iter().map(|row| row.trim()).filter(|row| !row.is_empty()).filter_map(|row| from_input(row, Some(&first))).collect();
    (!items.is_empty()).then_some(Value::Array(items))
}

/// Un valor como filas del formulario: una por elemento, y al menos una.
pub fn to_items(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) if !items.is_empty() => items.iter().map(to_input).collect(),
        Some(other) => vec![to_input(other)],
        None => vec![String::new()],
    }
}

// ─── La config que se guarda ─────────────────────────────────────────────────

// Un nombre de secreto como lo acepta cac (`domain.SecretNamePattern`): el
// mismo que el de los secrets de un desplegable.
use super::secret_refs::valid_secret_name;

/// Una referencia de 1Password: un ítem (`op://bóveda/ítem/campo`) o la
/// variable de un Environment (`op-env://entorno/NOMBRE`).
pub fn valid_op_ref(reference: &str) -> bool {
    if let Some(rest) = reference.strip_prefix("op://") {
        let mut parts = rest.splitn(3, '/');
        return matches!((parts.next(), parts.next(), parts.next()), (Some(v), Some(i), Some(f)) if !v.is_empty() && !i.is_empty() && !f.is_empty());
    }
    if let Some(rest) = reference.strip_prefix("op-env://") {
        if let Some((env, name)) = rest.split_once('/') {
            let mut chars = name.chars();
            return !env.is_empty()
                && name.len() <= 64
                && matches!(chars.next(), Some('A'..='Z' | 'a'..='z' | '_'))
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        }
    }
    false
}

/// Por qué no se puede guardar una config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigProblem {
    /// Las variables no son un objeto JSON.
    BadJson,
    /// Un nombre de secreto mal escrito o repetido, o un valor pegado en vez
    /// de su referencia.
    BadRefs,
}

impl ConfigProblem {
    pub fn key(self) -> &'static str {
        match self {
            ConfigProblem::BadJson => "common:serverConfig.badJson",
            ConfigProblem::BadRefs => "common:serverConfig.badRefs",
        }
    }
}

/// Lo que se manda al guardar, o por qué no se puede. Un valor pegado en vez
/// de su referencia se para aquí, antes de viajar. Una fila sin referencia es
/// un secreto que aún falta (la plantilla lo pide): no se guarda.
pub fn config_body(template_id: &str, vars_text: &str, rows: &[SecretRef], note: &str) -> Result<Value, ConfigProblem> {
    let vars = match serde_json::from_str::<Value>(if vars_text.trim().is_empty() { "{}" } else { vars_text }) {
        Ok(Value::Object(map)) => map,
        _ => return Err(ConfigProblem::BadJson),
    };
    let mut refs = Map::new();
    for row in rows {
        let (name, reference) = (row.name.trim(), row.reference.trim());
        if reference.is_empty() {
            continue;
        }
        if !valid_secret_name(name) || !valid_op_ref(reference) || refs.contains_key(name) {
            return Err(ConfigProblem::BadRefs);
        }
        refs.insert(name.to_string(), Value::String(reference.to_string()));
    }
    Ok(serde_json::json!({ "templateId": template_id, "vars": sorted_json(&Value::Object(vars)), "opRefs": refs, "note": note.trim() }))
}

/// Si lo que se mandaría es distinto de la versión actual.
pub fn config_changed(body: &Value, current: Option<&ServerConfigVersion>) -> bool {
    let refs: Vec<SecretRef> = body["opRefs"]
        .as_object()
        .map(|map| map.iter().map(|(name, reference)| SecretRef { name: name.clone(), reference: reference.as_str().unwrap_or_default().to_string() }).collect())
        .unwrap_or_default();
    let drafted = config_text(body["templateId"].as_str().unwrap_or_default(), body["vars"].as_object().unwrap_or(&Map::new()), &refs);
    let now = current.map(ServerConfigVersion::as_text).unwrap_or_else(|| config_text("", &Map::new(), &[]));
    drafted != now
}

/// Las filas, más una vacía por cada secreto que pide la plantilla y no está.
pub fn with_required(rows: &[SecretRef], requires: &[String]) -> Vec<SecretRef> {
    let mut out = rows.to_vec();
    for name in requires {
        if !rows.iter().any(|row| &row.name == name) {
            out.push(SecretRef { name: name.clone(), reference: String::new() });
        }
    }
    out
}

/// Una fila de lo que cambió entre dos textos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DiffRow {
    Same { text: String },
    Added { text: String },
    Removed { text: String },
    /// Líneas iguales plegadas.
    Skip { count: u32 },
}

/// Lo que cambió, línea a línea, con las iguales lejos de un cambio plegadas
/// (`lib/line-diff.ts`).
pub fn line_diff(before: &str, after: &str, context: usize) -> Vec<DiffRow> {
    let (a, b): (Vec<&str>, Vec<&str>) = (if before.is_empty() { vec![] } else { before.lines().collect() }, after.lines().collect());
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let mut rows = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            rows.push(DiffRow::Same { text: a[i].to_string() });
            (i, j) = (i + 1, j + 1);
        } else if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            rows.push(DiffRow::Removed { text: a[i].to_string() });
            i += 1;
        } else {
            rows.push(DiffRow::Added { text: b[j].to_string() });
            j += 1;
        }
    }
    let changed: Vec<bool> = rows.iter().map(|row| !matches!(row, DiffRow::Same { .. })).collect();
    let near = |n: usize| (n.saturating_sub(context)..=(n + context).min(rows.len().saturating_sub(1))).any(|k| changed[k]);
    let mut out = Vec::new();
    let mut skipped = 0u32;
    for (n, row) in rows.iter().enumerate() {
        if matches!(row, DiffRow::Same { .. }) && !near(n) {
            skipped += 1;
            continue;
        }
        if skipped > 0 {
            out.push(DiffRow::Skip { count: skipped });
            skipped = 0;
        }
        out.push(row.clone());
    }
    if skipped > 0 {
        out.push(DiffRow::Skip { count: skipped });
    }
    out
}

// ─── Estado y permisos ───────────────────────────────────────────────────────

/// Lo que quien mira puede hacer en Provisioning. Espejo de los handlers:
/// guardar config y plantillas y decidir son de admin
/// (`ServerConfigHandler.Save`, `ProvisioningHandler.Decide`); correr y pedir,
/// de miembro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ProvisionPermissions {
    pub edit_config: bool,
    pub decide: bool,
    pub run: bool,
}

pub fn provision_permissions(role: &str, superadmin: bool) -> ProvisionPermissions {
    let admin = superadmin || role == "admin";
    ProvisionPermissions { edit_config: admin, decide: admin, run: admin || role == "member" }
}

/// Provisioning del servidor abierto, desde que se miró su pestaña.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ProvisionState {
    pub server_id: String,
    pub permissions: ProvisionPermissions,
    /// Quien mira: descarta o vuelve a verificar las suyas.
    pub me: String,
    pub config: Option<ServerConfigSummary>,
    /// Sin ella (un backend de antes, o un fallo), no se corre con config.
    pub effective: Option<EffectiveConfig>,
    pub templates: Vec<ServerTemplate>,
    pub runs: Vec<ProvisionRun>,
    pub requests: Vec<ProvisioningRequest>,
    pub error: Option<String>,
    /// Lo de esta máquina, recordado por servidor: de dónde y contra qué.
    pub project_dir: String,
    pub playbook_id: String,
    pub limit: String,
    /// Lo leído de la carpeta con el motor de Ansible.
    pub project: Option<ProjectInfo>,
}

impl ProvisionState {
    /// Descartar o volver a verificar: quien la pidió o un admin.
    pub fn owns(&self, request: &ProvisioningRequest) -> bool {
        self.permissions.decide || (!self.me.is_empty() && request.requested_by == self.me)
    }

    pub fn request(&self, id: &str) -> Option<&ProvisioningRequest> {
        self.requests.iter().find(|r| r.id == id)
    }

    /// El playbook elegido, o el primero.
    pub fn playbook(&self) -> Option<&PlaybookInfo> {
        let manifest = self.project.as_ref()?.manifest.as_ref()?;
        manifest.playbook(&self.playbook_id).or_else(|| manifest.playbooks.first())
    }
}

/// Si en esta máquina se puede correr Ansible para ese proyecto.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct AnsibleTools {
    pub platform_supported: bool,
    pub reason: String,
    pub ansible_playbook: Option<String>,
}

/// Un host del inventario de un repo v1: nombres y grupos, nunca sus
/// variables.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct InventoryHost {
    pub name: String,
    pub groups: Vec<String>,
    pub ansible_host: Option<String>,
}

/// Lo que le falta a la config para correr un playbook.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct PlanGaps {
    pub vars: Vec<String>,
    pub secrets: Vec<String>,
}

impl PlanGaps {
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty() && self.secrets.is_empty()
    }
}

/// Lo leído de la carpeta del proyecto con el motor de Ansible de esta
/// máquina.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// De qué carpeta es: si se cambia mientras se lee, lo leído sobra.
    pub dir: String,
    pub manifest: Option<AnsibleManifest>,
    pub error: Option<String>,
    pub tools: Option<AnsibleTools>,
    pub hosts: Vec<InventoryHost>,
    /// De qué playbook son `gaps`.
    pub gaps_of: String,
    pub gaps: Option<PlanGaps>,
}

/// Lo que se recuerda por servidor en esta máquina.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ProvisionPrefs {
    pub project_dir: String,
    pub playbook_id: String,
    pub limit: String,
}

const PREFS_KEY: &str = "provision-prefs";

// ─── Errores ─────────────────────────────────────────────────────────────────

/// Por qué no se hizo algo de Provisioning.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProvisionError {
    #[error("admin-required")]
    AdminOnly,
    #[error("not-a-writer-in-org")]
    ReadOnly,
    #[error("server-gone")]
    ServerGone,
    #[error("config: {0:?}")]
    Config(ConfigProblem),
    /// Destructiva: el nombre escrito no es el del servidor.
    #[error("confirm-server-name")]
    ConfirmServerName,
    /// Hay banderas rojas y no se reconocieron.
    #[error("acknowledge-flags")]
    AcknowledgeFlags,
    /// Ni la pidió quien mira ni es admin.
    #[error("provisioning-request-not-yours")]
    NotYours,
    #[error("provisioning-request-not-found")]
    RequestGone,
    /// Ya corre algo desde esta app.
    #[error("busy")]
    Busy,
    /// Sin carpeta o sin manifiesto que leer.
    #[error("no-project")]
    NoProject,
    /// El servidor no tiene config en cac.
    #[error("no-config")]
    NoConfig,
    /// El playbook no declara su riesgo.
    #[error("no-risk: {0}")]
    NoRisk(String),
    /// El riesgo del manifiesto ya no es el aprobado.
    #[error("risk-changed: {0}")]
    RiskChanged(String),
    #[error("cancelled")]
    Cancelled,
    /// Variables extra que no son `nombre=valor`.
    #[error("bad-vars: {0}")]
    BadVars(String),
    /// Variables obligatorias sin valor.
    #[error("missing-vars: {0}")]
    MissingVars(String),
    /// Lo que dijo el motor de Ansible (un secreto que no se lee, el venv…).
    #[error("{0}")]
    Engine(String),
    #[error(transparent)]
    Api(#[from] ApiError),
}

/// Un fallo de Provisioning, dicho para quien lo lee.
pub fn provision_phrase(locale: Locale, error: &ProvisionError, fallback_key: &str) -> String {
    match error {
        ProvisionError::AdminOnly | ProvisionError::ReadOnly => text(locale, &format!("errors:{error}")),
        ProvisionError::ServerGone => text(locale, "common:servers.serverGone"),
        ProvisionError::Config(problem) => text(locale, problem.key()),
        ProvisionError::ConfirmServerName | ProvisionError::AcknowledgeFlags | ProvisionError::NotYours | ProvisionError::RequestGone => {
            text(locale, &format!("errors:{error}"))
        }
        ProvisionError::Busy => text(locale, "common:provision.running"),
        ProvisionError::NoProject => text(locale, "common:serverConfig.pickFolderForPlaybooks"),
        ProvisionError::NoConfig => text(locale, "common:provision.noConfig"),
        ProvisionError::NoRisk(id) => text_with(locale, "common:provision.noRisk", &[("id", id)]),
        ProvisionError::RiskChanged(id) => text_with(locale, "common:provision.riskChanged", &[("playbook", id)]),
        ProvisionError::Cancelled => text(locale, "common:provision.rehearseCancelled"),
        ProvisionError::BadVars(lines) => text_with(locale, "common:provision.badVars", &[("lines", lines)]),
        ProvisionError::MissingVars(names) => text_with(locale, "common:provision.missingVars", &[("names", names)]),
        ProvisionError::Engine(message) => message.clone(),
        ProvisionError::Api(error) => error_phrase(locale, error, Some(fallback_key)),
    }
}

// ─── Lo que se pide ──────────────────────────────────────────────────────────

impl Platform {
    pub(crate) fn provision_permissions(&self) -> ProvisionPermissions {
        let role = self.core.orgs().state().current().map(|org| org.role.clone()).unwrap_or_default();
        let superadmin = self.core.session().is_some_and(|session| session.superadmin);
        provision_permissions(&role, superadmin)
    }

    pub(crate) fn me(&self) -> String {
        self.core.session().map(|session| session.id).unwrap_or_default()
    }

    pub(crate) fn edit_provision(&self, server_id: &str, edit: impl FnOnce(&mut ProvisionState)) {
        self.change(|data| {
            if let Some(provision) = data.provision.as_mut().filter(|p| p.server_id == server_id) {
                edit(provision);
            }
        });
    }

    fn prefs(&self) -> std::collections::HashMap<String, ProvisionPrefs> {
        self.core.inner.storage.get(PREFS_KEY).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
    }

    /// Recuerda de dónde y contra qué, por servidor, en esta máquina.
    pub(crate) fn keep_prefs(&self, state: &ProvisionState) {
        let mut all = self.prefs();
        all.insert(state.server_id.clone(), ProvisionPrefs { project_dir: state.project_dir.clone(), playbook_id: state.playbook_id.clone(), limit: state.limit.clone() });
        if let Ok(raw) = serde_json::to_string(&all) {
            let _ = self.core.inner.storage.set(PREFS_KEY, &raw);
        }
    }

    /// Entra en la pestaña: la config, las plantillas, lo aplicado y las
    /// peticiones, a la vez. Lo que falle se queda como estaba.
    pub async fn load_provision(&self) {
        let Some(server) = self.open() else { return };
        self.close_orphan().await;
        let prefs = self.prefs().remove(&server.id).unwrap_or_default();
        self.change(|data| {
            if data.provision.as_ref().is_none_or(|p| p.server_id != server.id) {
                data.provision = Some(ProvisionState {
                    server_id: server.id.clone(),
                    project_dir: prefs.project_dir,
                    playbook_id: prefs.playbook_id,
                    limit: prefs.limit,
                    ..Default::default()
                });
            }
        });
        let route = |what: &str| format!("/api/v1/servers/{}/{what}", server.id);
        let (core, path) = (self.core.clone(), route("config"));
        let config = tokio::spawn(async move { core.get::<ServerConfigSummary>(&path).await });
        let (core, path) = (self.core.clone(), format!("/api/v1/organizations/{}/server-templates/", server.org_id));
        let templates = tokio::spawn(async move { core.get_list::<ServerTemplate>(&path).await });
        let platform = self.core.platform();
        let effective = tokio::spawn(async move { platform.reload_effective().await });
        let platform = self.core.platform();
        let runs = tokio::spawn(async move { platform.reload_runs().await });
        let platform = self.core.platform();
        let requests = tokio::spawn(async move { platform.reload_requests().await });
        let (config, templates) = (super::flatten(config.await), super::flatten(templates.await));
        let _ = (effective.await, runs.await, requests.await);
        let error = config.as_ref().err().or(templates.as_ref().err()).cloned();
        self.edit_provision(&server.id, |provision| {
            if let Ok(config) = config {
                provision.config = Some(config);
            }
            if let Ok(templates) = templates {
                provision.templates = templates;
            }
            provision.error = error;
        });
    }

    /// La config efectiva de ahora. Un backend de antes de la config en cac
    /// no tiene la ruta: sin efectiva.
    pub async fn reload_effective(&self) {
        let Some(server) = self.open() else { return };
        let answer = self.core.get::<Value>(&format!("/api/v1/servers/{}/config/effective", server.id)).await;
        let effective = answer.ok().filter(|value| value.get("connection").is_some()).and_then(|value| serde_json::from_value::<EffectiveConfig>(value).ok());
        self.edit_provision(&server.id, |provision| provision.effective = effective);
    }

    pub async fn reload_runs(&self) {
        let Some(server) = self.open() else { return };
        if let Ok(runs) = self.core.get_list::<ProvisionRun>(&format!("/api/v1/servers/{}/provisioning-runs", server.id)).await {
            self.edit_provision(&server.id, |provision| provision.runs = runs);
        }
    }

    pub async fn reload_requests(&self) {
        let Some(server) = self.open() else { return };
        if let Ok(requests) = self.core.get_list::<ProvisioningRequest>(&format!("/api/v1/servers/{}/provisioning-requests", server.id)).await {
            self.edit_provision(&server.id, |provision| provision.requests = requests);
        }
    }

    async fn reload_config(&self, server: &Server) {
        if let Ok(config) = self.core.get::<ServerConfigSummary>(&format!("/api/v1/servers/{}/config", server.id)).await {
            self.edit_provision(&server.id, |provision| provision.config = Some(config));
        }
    }

    async fn reload_templates(&self, server: &Server) {
        if let Ok(templates) = self.core.get_list::<ServerTemplate>(&format!("/api/v1/organizations/{}/server-templates/", server.org_id)).await {
            self.edit_provision(&server.id, |provision| provision.templates = templates);
        }
    }

    fn admin_on_open(&self) -> Result<Server, ProvisionError> {
        if !self.provision_permissions().edit_config {
            return Err(ProvisionError::AdminOnly);
        }
        self.open().ok_or(ProvisionError::ServerGone)
    }

    /// Guarda una versión nueva de la config del servidor abierto. Lo que se
    /// manda ya pasó por `config_body`; aquí se vuelve a mirar que no lleve
    /// nada que no sea una referencia.
    pub async fn save_config(&self, template_id: &str, vars_text: &str, rows: &[SecretRef], note: &str) -> Result<(), ProvisionError> {
        let server = self.admin_on_open()?;
        let body = config_body(template_id, vars_text, rows, note).map_err(ProvisionError::Config)?;
        self.put_config(&server, body).await
    }

    /// Guarda como versión nueva lo que decía una vieja.
    pub async fn restore_config(&self, version: &ServerConfigVersion, note: &str) -> Result<(), ProvisionError> {
        let server = self.admin_on_open()?;
        let mut body = config_body(&version.template_id, &version.vars, &version.op_refs, note).map_err(ProvisionError::Config)?;
        body["templateSeq"] = Value::from(version.template_seq);
        self.put_config(&server, body).await
    }

    /// Copia la config de otro servidor de la organización, menos sus
    /// secretos de cada máquina (lo hace el backend).
    pub async fn copy_config_from(&self, source_id: &str, note: &str) -> Result<(), ProvisionError> {
        let server = self.admin_on_open()?;
        self.put_config(&server, serde_json::json!({ "basedOnServerId": source_id, "note": note.trim() })).await
    }

    async fn put_config(&self, server: &Server, body: Value) -> Result<(), ProvisionError> {
        let _: Value = self.core.call("PUT", &format!("/api/v1/servers/{}/config", server.id), Some(body)).await?;
        self.reload_config(server).await;
        self.reload_effective().await;
        Ok(())
    }

    /// Crea una plantilla; devuelve su id.
    pub async fn create_template(&self, draft: &TemplateDraft, manifest: &AnsibleManifest) -> Result<String, ProvisionError> {
        let server = self.admin_on_open()?;
        #[derive(Deserialize)]
        struct Created {
            id: String,
        }
        let created: Created = self.core.call("POST", &format!("/api/v1/organizations/{}/server-templates/", server.org_id), Some(draft.body(manifest, true))).await?;
        self.reload_templates(&server).await;
        Ok(created.id)
    }

    /// Guarda una versión nueva de una plantilla.
    pub async fn save_template(&self, template_id: &str, draft: &TemplateDraft, manifest: &AnsibleManifest) -> Result<(), ProvisionError> {
        let server = self.admin_on_open()?;
        let _: Value = self.core.call("PUT", &format!("/api/v1/organizations/{}/server-templates/{template_id}", server.org_id), Some(draft.body(manifest, false))).await?;
        self.reload_templates(&server).await;
        self.reload_effective().await;
        Ok(())
    }

    fn requests_route(&self, request_id: &str, what: &str) -> Result<(Server, String), ProvisionError> {
        let server = self.open().ok_or(ProvisionError::ServerGone)?;
        let route = format!("/api/v1/servers/{}/provisioning-requests/{request_id}/{what}", server.id);
        Ok((server, route))
    }

    fn request_of(&self, server_id: &str, request_id: &str) -> Result<ProvisioningRequest, ProvisionError> {
        let data = self.data();
        data.provision.as_ref().filter(|p| p.server_id == server_id).and_then(|p| p.request(request_id).cloned()).ok_or(ProvisionError::RequestGone)
    }

    fn request_seen(&self, request: ProvisioningRequest) {
        let server_id = request.server_id.clone();
        self.edit_provision(&server_id, |provision| match provision.requests.iter_mut().find(|known| known.id == request.id) {
            Some(slot) => *slot = request,
            None => provision.requests.insert(0, request),
        });
    }

    /// Aprueba o rechaza una petición. **Quien llama ya preguntó** al
    /// aprobar. Una destructiva necesita el nombre del servidor escrito tal
    /// cual, y una con banderas rojas que se reconozcan: no se manda sin eso.
    pub async fn decide_request(&self, request_id: &str, approve: bool, note: &str, typed: &str, acknowledged: bool) -> Result<(), ProvisionError> {
        if !self.provision_permissions().decide {
            return Err(ProvisionError::AdminOnly);
        }
        let (server, route) = self.requests_route(request_id, "decision")?;
        let request = self.request_of(&server.id, request_id)?;
        if approve {
            if request.risk == Risk::Destructive && typed != server.name {
                return Err(ProvisionError::ConfirmServerName);
            }
            if !can_approve(&request, &server.name, typed, acknowledged) {
                return Err(ProvisionError::AcknowledgeFlags);
            }
        }
        let body = serde_json::json!({ "approve": approve, "note": note.trim(), "confirmServer": typed, "acknowledgeFlags": acknowledged });
        let decided: ProvisioningRequest = self.core.call("POST", &route, Some(body)).await?;
        self.request_seen(decided);
        Ok(())
    }

    /// Quita una petición que se quedó sin ensayo. **Quien llama ya preguntó.**
    pub async fn discard_request(&self, request_id: &str) -> Result<(), ProvisionError> {
        self.owner_action(request_id, "discard").await
    }

    /// Vuelve a correr la verificación de una que la falló (la cierra lo que
    /// diga ésta). **Quien llama ya preguntó.**
    pub async fn reverify_request(&self, request_id: &str) -> Result<(), ProvisionError> {
        self.owner_action(request_id, "reverify").await
    }

    async fn owner_action(&self, request_id: &str, what: &str) -> Result<(), ProvisionError> {
        let (server, route) = self.requests_route(request_id, what)?;
        let request = self.request_of(&server.id, request_id)?;
        let owns = self.provision_permissions().decide || request.requested_by == self.me();
        if !owns {
            return Err(ProvisionError::NotYours);
        }
        let changed: ProvisioningRequest = self.core.call("POST", &route, Some(serde_json::json!({}))).await?;
        self.request_seen(changed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(risk: Risk, flags: &[FlagLevel]) -> ProvisioningRequest {
        ProvisioningRequest { risk, flags: flags.iter().map(|level| RiskFlag { code: "x".into(), level: *level, detail: String::new() }).collect(), ..Default::default() }
    }

    /// Rotura que atrapa: aprobar una destructiva sin el nombre exacto (otra
    /// mayúscula, un espacio) o con una bandera roja sin reconocer; o pedir
    /// el nombre a una que no es destructiva.
    #[test]
    fn approving_follows_the_protocol() {
        let destructive = request(Risk::Destructive, &[]);
        assert!(!can_approve(&destructive, "gstd", "", false));
        assert!(!can_approve(&destructive, "gstd", "GSTD", false));
        assert!(!can_approve(&destructive, "gstd", "gstd ", false));
        assert!(can_approve(&destructive, "gstd", "gstd", false));
        let red = request(Risk::Change, &[FlagLevel::Amber, FlagLevel::Red]);
        assert!(!can_approve(&red, "gstd", "", false));
        assert!(can_approve(&red, "gstd", "", true));
        assert!(can_approve(&request(Risk::Change, &[FlagLevel::Amber]), "gstd", "", false));
    }

    /// Rotura que atrapa: dejar pasar un valor pegado en vez de su referencia,
    /// un nombre en minúsculas o repetido, o guardar una fila vacía (un
    /// secreto que aún falta) como si fuera una referencia.
    #[test]
    fn only_references_travel() {
        let row = |name: &str, reference: &str| SecretRef { name: name.into(), reference: reference.into() };
        let body = config_body("t1", "{\"b\":1,\"a\":2}", &[row("OPS_PASSWORD", "op://Infra/gstd/password"), row("TOKEN", "")], " por qué ").unwrap();
        assert_eq!(body, serde_json::json!({ "templateId": "t1", "vars": { "a": 2, "b": 1 }, "opRefs": { "OPS_PASSWORD": "op://Infra/gstd/password" }, "note": "por qué" }));
        assert_eq!(config_body("", "{}", &[row("OPS_PASSWORD", "hunter2")], ""), Err(ConfigProblem::BadRefs));
        assert_eq!(config_body("", "{}", &[row("ops_password", "op://a/b/c")], ""), Err(ConfigProblem::BadRefs));
        assert_eq!(config_body("", "{}", &[row("A", "op://a/b/c"), row("A", "op://a/b/d")], ""), Err(ConfigProblem::BadRefs));
        assert_eq!(config_body("", "{}", &[row("A", "op://a/b")], ""), Err(ConfigProblem::BadRefs));
        assert_eq!(config_body("", "{}", &[row("A", "op://a/b/")], ""), Err(ConfigProblem::BadRefs));
        assert!(config_body("", "{}", &[row("A", "op-env://prod/DB_URL")], "").is_ok());
        assert_eq!(config_body("", "[1]", &[], ""), Err(ConfigProblem::BadJson));
        assert_eq!(config_body("", "{", &[], ""), Err(ConfigProblem::BadJson));
    }

    /// Rotura que atrapa: dar por cambiada una config sin tocar porque el
    /// backend devolvió sus claves en otro orden, o por sin cambios una que
    /// cambió una referencia.
    #[test]
    fn a_config_is_compared_by_what_it_says() {
        let current: ServerConfigVersion = serde_json::from_value(serde_json::json!({ "seq": 3, "templateId": "t1", "vars": { "z": 1, "a": [1, 2] }, "opRefs": { "B": "op://v/i/b", "A": "op://v/i/a" } })).unwrap();
        let rows = [SecretRef { name: "A".into(), reference: "op://v/i/a".into() }, SecretRef { name: "B".into(), reference: "op://v/i/b".into() }];
        let same = config_body("t1", "{\"a\":[1,2],\"z\":1}", &rows, "nota").unwrap();
        assert!(!config_changed(&same, Some(&current)));
        let other = config_body("t1", "{\"a\":[1,2],\"z\":1}", &rows[..1], "").unwrap();
        assert!(config_changed(&other, Some(&current)));
        assert!(!config_changed(&config_body("", "{}", &[], "").unwrap(), None));
    }

    fn manifest() -> AnsibleManifest {
        serde_json::from_value(serde_json::json!({
            "version": 2,
            "playbooks": [
                { "id": "baseline", "file": "b.yml", "name": "Baseline", "risk": "change", "become": true, "becomePasswordFrom": "OPS_PASSWORD", "verify": "check",
                  "vars": [ { "name": "domain", "required": true }, { "name": "ports", "default": [80, 443] }, { "name": "keys", "type": "list", "required": true } ] },
                { "id": "swarm", "file": "s.yml", "name": "Swarm", "risk": "change",
                  "vars": [ { "name": "domain", "description": "El dominio" }, { "name": "token", "from": "CF_TOKEN", "description": "Cloudflare" }, { "name": "pw", "secret": true } ] },
                { "id": "check", "file": "c.yml", "name": "Check", "risk": "read" },
            ]
        }))
        .unwrap()
    }

    /// Rotura que atrapa: pintar como campo una variable secreta, perder la
    /// descripción que trae otra etapa, tomar una lista sin default por un
    /// campo de una línea, o no marcar la de sudo como de cada máquina.
    #[test]
    fn the_variables_of_the_stages_become_a_form() {
        let fields = var_fields(&manifest(), &["baseline".into(), "swarm".into()]);
        assert_eq!(fields.shared.iter().map(|f| (f.name.as_str(), f.default.as_deref(), f.list)).collect::<Vec<_>>(), [("ports", Some("[80,443]"), true)]);
        assert_eq!(fields.per_server.iter().map(|f| (f.name.as_str(), f.description.as_str(), f.list)).collect::<Vec<_>>(), [("domain", "El dominio", false), ("keys", "", true)]);
        assert_eq!(fields.secrets.iter().map(|s| (s.name.as_str(), s.sudo)).collect::<Vec<_>>(), [("CF_TOKEN", false), ("OPS_PASSWORD", true)]);
        assert_eq!(stage_playbooks(&manifest()).iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["baseline", "swarm"], "una verificación no es una etapa");
    }

    /// Rotura que atrapa: guardar las etapas en el orden en que se marcaron
    /// y no en el del manifiesto, perder los defaults sin tocar, o dejar la
    /// contraseña de sudo como compartida.
    #[test]
    fn a_template_is_drafted_from_its_stages() {
        let draft = TemplateDraft { name: " Swarm ".into(), playbooks: vec!["swarm".into(), "baseline".into()], ..Default::default() };
        let body = draft.body(&manifest(), true);
        assert_eq!(body["playbooks"], serde_json::json!(["baseline", "swarm"]));
        assert_eq!(body["vars"], serde_json::json!({ "ports": [80, 443] }));
        assert_eq!(body["requires"], serde_json::json!(["CF_TOKEN", "OPS_PASSWORD"]));
        assert_eq!(body["perServer"], serde_json::json!(["OPS_PASSWORD"]));
        assert_eq!(body["name"], "Swarm");
        assert!(draft.body(&manifest(), false).get("name").is_none());
        assert!(!TemplateDraft::default().can_save(false));
    }

    /// Rotura que atrapa: convertir en número un «0755» que el default dice
    /// que es texto, guardar un campo vacío, o partir una llave SSH por sus
    /// espacios.
    #[test]
    fn what_is_typed_keeps_the_type_of_its_default() {
        let text = Value::String("0644".into());
        assert_eq!(from_input("0755", Some(&text)), Some(Value::String("0755".into())));
        assert_eq!(from_input("755", Some(&text)), Some(Value::String("755".into())));
        assert_eq!(from_input("22", None), Some(serde_json::json!(22)));
        assert_eq!(from_input("true", None), Some(Value::Bool(true)));
        assert_eq!(from_input("[1,2]", None), Some(serde_json::json!([1, 2])));
        assert_eq!(from_input("  ", None), None);
        let rows = ["ssh-ed25519 AAAA ana@x".to_string(), " ".into()];
        assert_eq!(from_items(&rows, None), Some(serde_json::json!(["ssh-ed25519 AAAA ana@x"])));
        assert_eq!(from_items(&["80".into(), "443".into()], Some(&serde_json::json!([8080]))), Some(serde_json::json!([80, 443])));
        assert_eq!(to_items(None), [""]);
    }

    /// Rotura que atrapa: esconder una abierta tras «Ver las anteriores», o
    /// enseñar todas las cerradas de un simulacro.
    #[test]
    fn the_inbox_shows_the_open_and_the_last_closed() {
        let r = |id: &str, status: &str| ProvisioningRequest { id: id.into(), status: status.into(), ..Default::default() };
        let all = [r("a", "rejected"), r("b", "awaiting_approval"), r("c", "discarded"), r("d", "succeeded"), r("e", "failed"), r("f", "approved")];
        let (shown, hidden) = visible_requests(&all, false);
        assert_eq!(shown.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d", "f"]);
        assert_eq!(hidden, 1);
        assert_eq!(visible_requests(&all, true).0.len(), 6);
    }

    /// Rotura que atrapa: dar por abandonada una que se está ensayando, o no
    /// dejar descartar una que lleva horas «ensayando».
    #[test]
    fn a_check_nobody_runs_goes_stale() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-21T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let at = |created: &str, status: &str| ProvisioningRequest { created_at: created.into(), status: status.into(), ..Default::default() };
        assert!(!is_stale(&at("2026-11-21T11:50:00Z", "checking"), now));
        assert!(is_stale(&at("2026-11-21T11:40:00Z", "checking"), now));
        assert!(!is_stale(&at("2026-11-21T08:00:00Z", "awaiting_approval"), now));
    }

    /// Rotura que atrapa: un nombre de servidor con comilla que rompe el
    /// comando de la terminal.
    #[test]
    fn the_grant_command_quotes_the_server() {
        assert_eq!(grant_command("o'neil", "r1"), "cac provision run --server 'o'\\''neil' --grant r1");
    }

    /// Rotura que atrapa: enseñar entero un fichero que cambió una línea, o
    /// no marcar qué entra y qué sale.
    #[test]
    fn a_diff_shows_the_change_and_folds_the_rest() {
        let before = (1..=10).map(|n| format!("l{n}")).collect::<Vec<_>>().join("\n");
        let after = before.replace("l5", "cinco");
        let rows = line_diff(&before, &after, 1);
        assert_eq!(
            rows,
            [
                DiffRow::Skip { count: 3 },
                DiffRow::Same { text: "l4".into() },
                DiffRow::Removed { text: "l5".into() },
                DiffRow::Added { text: "cinco".into() },
                DiffRow::Same { text: "l6".into() },
                DiffRow::Skip { count: 4 },
            ]
        );
        assert_eq!(line_diff("", "a", 3), [DiffRow::Added { text: "a".into() }]);
    }

    /// Rotura que atrapa: separarse de los handlers (un miembro que guarda
    /// la config o aprueba; un lector que corre).
    #[test]
    fn permissions_mirror_the_handlers() {
        assert_eq!(provision_permissions("viewer", false), ProvisionPermissions::default());
        assert_eq!(provision_permissions("member", false), ProvisionPermissions { run: true, ..Default::default() });
        assert_eq!(provision_permissions("admin", false), ProvisionPermissions { edit_config: true, decide: true, run: true });
        assert_eq!(provision_permissions("", true), ProvisionPermissions { edit_config: true, decide: true, run: true });
    }

    /// Rotura que atrapa: leer como vacía una petición con `null` en sus
    /// listas, o perder la forma de lo aplicado al pasarlo a texto.
    #[test]
    fn the_backend_shapes_are_read() {
        let request: ProvisioningRequest = serde_json::from_value(serde_json::json!({
            "id": "r", "risk": "destructive", "status": "awaiting_approval", "flags": null,
            "check": { "hosts": ["gstd"], "changedTasks": null, "failures": null, "changed": 2 }
        }))
        .unwrap();
        assert_eq!((request.risk, request.flags.len(), request.check.unwrap().changed), (Risk::Destructive, 0, 2));
        let run: ProvisionRun = serde_json::from_value(serde_json::json!({ "id": "x", "applied": { "vars": { "b": 1, "a": 2 }, "opRefs": {} } })).unwrap();
        assert_eq!(run.applied_text().unwrap(), "{\n  \"opRefs\": {},\n  \"vars\": {\n    \"a\": 2,\n    \"b\": 1\n  }\n}");
        let effective: EffectiveConfig = serde_json::from_value(serde_json::json!({ "configSeq": 4, "vars": { "x": 1 }, "opRefs": { "A": "op://v/i/a" }, "connection": { "name": "gstd", "host": "h", "sshPort": 22, "sshUser": "ops" } })).unwrap();
        let back: Value = serde_json::from_str(&effective.to_json()).unwrap();
        assert_eq!((back["vars"]["x"].clone(), back["opRefs"]["A"].clone(), back["connection"]["sshPort"].clone()), (serde_json::json!(1), serde_json::json!("op://v/i/a"), serde_json::json!(22)));
    }
}
