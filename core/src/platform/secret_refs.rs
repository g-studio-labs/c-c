//! Los secrets de un servicio que despliega cac, por referencia a 1Password
//! (`ServiceSecretRefs.tsx`). Aquí sólo se dice **de dónde** sale cada uno:
//! rotar los lee en esta máquina y los lleva al servidor (`cac_ops::secrets`).
//! A cac, y a este estado, sólo llegan los nombres.

use serde::{Deserialize, Serialize};

use super::{Platform, ServerEditError};

/// Dónde se usa un secret: la app en marcha, el build en el servidor, o los
/// dos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum SecretUsage {
    #[default]
    Runtime,
    Build,
    Both,
}

impl SecretUsage {
    pub fn label_key(self) -> &'static str {
        match self {
            SecretUsage::Runtime => "common:secrets.usageRuntime",
            SecretUsage::Build => "common:secrets.usageBuild",
            SecretUsage::Both => "common:secrets.usageBoth",
        }
    }

    /// La siguiente al pulsar: app, build, los dos.
    pub fn next(self) -> SecretUsage {
        match self {
            SecretUsage::Runtime => SecretUsage::Build,
            SecretUsage::Build => SecretUsage::Both,
            SecretUsage::Both => SecretUsage::Runtime,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct SecretRefRow {
    pub name: String,
    /// `op://bóveda/ítem/campo` u `op-env://Environment/VARIABLE`.
    pub op_ref: String,
    pub usage: SecretUsage,
}

/// Una rotación apuntada (`domain.SecretRotation`), sin sus versiones.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct SecretRotation {
    pub id: String,
    pub created_at: String,
    /// Separados por comas.
    pub names: String,
    /// `succeeded` o `failed`.
    pub status: String,
    pub error: String,
    pub started_by_name: Option<String>,
}

/// Los secrets del desplegable que se mira, como los guardó cac.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct SecretsState {
    pub deployable_id: String,
    pub refs: Option<Vec<SecretRefRow>>,
    pub rotations: Vec<SecretRotation>,
    pub error: Option<String>,
}

/// Lo que le pasa a una fila, para marcarla y decirlo: un aviso general sin
/// decir cuál obligaba a revisar treinta filas a ojo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefProblem {
    Name(String),
    Repeated(String),
    Ref(String),
}

/// Un nombre de secret de Docker que también sea una variable de entorno.
pub fn valid_secret_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_uppercase() || c == '_');
    first_ok && name.len() <= 40 && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Una referencia que se puede leer: un ítem (`op://bóveda/ítem/campo`) o
/// un Environment (`op-env://Environment/VARIABLE`). `cac_ops::secrets` la
/// vuelve a comprobar antes de lanzar nada.
pub fn valid_secret_ref(reference: &str) -> bool {
    if let Some(rest) = reference.strip_prefix("op://") {
        let parts: Vec<&str> = rest.splitn(3, '/').collect();
        return parts.len() == 3 && parts.iter().all(|part| !part.is_empty());
    }
    if let Some((env, var)) = reference.strip_prefix("op-env://").and_then(|rest| rest.split_once('/')) {
        let mut chars = var.chars();
        return !env.is_empty()
            && var.len() <= 64
            && chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    }
    false
}

/// Lo que impide guardar, una vez por fila y en su orden.
pub fn ref_problems(rows: &[SecretRefRow]) -> Vec<RefProblem> {
    let mut problems = Vec::new();
    for row in rows {
        let name = row.name.trim().to_string();
        let problem = if !valid_secret_name(&name) {
            RefProblem::Name(name)
        } else if rows.iter().filter(|other| other.name.trim() == name).count() > 1 {
            RefProblem::Repeated(name)
        } else if !valid_secret_ref(row.op_ref.trim()) {
            RefProblem::Ref(name)
        } else {
            continue;
        };
        if !problems.contains(&problem) {
            problems.push(problem);
        }
    }
    problems
}

/// Una etiqueta de 1Password como nombre de secret: mayúsculas y guiones
/// bajos.
pub fn label_to_name(label: &str) -> String {
    let mut name = String::new();
    let mut underscore = false;
    for c in label.trim().to_uppercase().chars() {
        if c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' {
            name.push(c);
            underscore = false;
        } else if !underscore {
            name.push('_');
            underscore = true;
        }
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name.chars().take(40).collect()
}

/// Cruza los build args de un Dockerfile con los secrets del servicio. Uno
/// que ya está como secret de la app pasa a «los dos»; uno que falta se
/// añade como «el build», sin referencia, para que se la pongan.
pub fn merge_build_args(rows: &[SecretRefRow], args: &[String]) -> (Vec<SecretRefRow>, usize, usize) {
    let mut promoted = 0;
    let mut next: Vec<SecretRefRow> = rows
        .iter()
        .map(|row| {
            if row.usage == SecretUsage::Runtime && args.iter().any(|arg| arg == row.name.trim()) {
                promoted += 1;
                SecretRefRow { usage: SecretUsage::Both, ..row.clone() }
            } else {
                row.clone()
            }
        })
        .collect();
    let mut added = 0;
    for arg in args {
        if !rows.iter().any(|row| row.name.trim() == arg) && !next.iter().any(|row| &row.name == arg) {
            next.push(SecretRefRow { name: arg.clone(), op_ref: String::new(), usage: SecretUsage::Build });
            added += 1;
        }
    }
    (next, added, promoted)
}

/// Unas filas nuevas, sin las que ya están ni las que no valen como nombre.
pub fn append_rows(rows: &[SecretRefRow], more: impl IntoIterator<Item = SecretRefRow>) -> Vec<SecretRefRow> {
    let mut next = rows.to_vec();
    for row in more {
        if valid_secret_name(&row.name) && !next.iter().any(|known| known.name.trim() == row.name) {
            next.push(row);
        }
    }
    next
}

impl Platform {
    fn secrets_route(&self, deployable_id: &str, rest: &str) -> Result<String, ServerEditError> {
        let server = self.open().ok_or(ServerEditError::ServerGone)?;
        Ok(format!("/api/v1/servers/{}/deployables/{deployable_id}/{rest}", server.id))
    }

    /// Pide los secrets de un desplegable y sus rotaciones.
    pub async fn load_secret_refs(&self, deployable_id: &str) {
        let (Ok(refs_route), Ok(rotations_route)) = (self.secrets_route(deployable_id, "secret-refs"), self.secrets_route(deployable_id, "secret-rotations")) else { return };
        let (refs, rotations) = (self.core.get_list::<SecretRefRow>(&refs_route).await, self.core.get_list::<SecretRotation>(&rotations_route).await);
        self.change(|data| {
            let error = refs.as_ref().err().map(|error| error.to_string());
            data.secrets = Some(SecretsState { deployable_id: deployable_id.to_string(), refs: refs.ok(), rotations: rotations.unwrap_or_default(), error });
        });
    }

    /// Guarda la lista entera. Sólo quien administra: dice qué se lleva al
    /// servidor en cada rotación.
    pub async fn save_secret_refs(&self, deployable_id: &str, rows: &[SecretRefRow]) -> Result<(), ServerEditError> {
        if !self.permissions().manage_deploys {
            return Err(ServerEditError::AdminOnly);
        }
        let route = self.secrets_route(deployable_id, "secret-refs")?;
        let refs: Vec<SecretRefRow> = rows.iter().map(|row| SecretRefRow { name: row.name.trim().to_string(), op_ref: row.op_ref.trim().to_string(), usage: row.usage }).collect();
        self.core.call_ok("PUT", &route, Some(serde_json::json!({ "refs": refs }))).await?;
        self.load_secret_refs(deployable_id).await;
        Ok(())
    }

    /// Los `ARG` del Dockerfile del servicio, en su rama.
    pub async fn build_args(&self, deployable_id: &str) -> Result<Vec<String>, ServerEditError> {
        let route = self.secrets_route(deployable_id, "build-args")?;
        Ok(self.core.get_list::<String>(&route).await?)
    }

    /// Apunta en cac una rotación hecha desde esta máquina: los nombres y la
    /// versión de cada secret, nunca su valor.
    pub async fn record_rotation(&self, deployable_id: &str, names: &[String], versions: &[(String, String)], error: Option<&str>) -> Result<(), ServerEditError> {
        if !self.permissions().deploy {
            return Err(ServerEditError::ReadOnly);
        }
        let route = self.secrets_route(deployable_id, "secret-rotations")?;
        let versions: serde_json::Map<String, serde_json::Value> = versions.iter().map(|(name, version)| (name.clone(), serde_json::Value::String(version.clone()))).collect();
        let body = serde_json::json!({
            "names": names,
            "versions": versions,
            "status": if error.is_some() { "failed" } else { "succeeded" },
            "error": error.unwrap_or_default(),
        });
        self.core.call_ok("POST", &route, Some(body)).await?;
        self.load_secret_refs(deployable_id).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, op_ref: &str, usage: SecretUsage) -> SecretRefRow {
        SecretRefRow { name: name.into(), op_ref: op_ref.into(), usage }
    }

    /// Rotura que atrapa: dejar guardar un nombre que no es variable de
    /// entorno, uno repetido (el segundo pisaría al primero en el servidor)
    /// o una referencia a medias; o no decir cuál.
    #[test]
    fn each_bad_row_says_what_is_wrong() {
        let rows = [
            row("DATABASE_URL", "op://Prod/api/url", SecretUsage::Runtime),
            row("1BAD", "op://Prod/api/x", SecretUsage::Runtime),
            row("TOKEN", "op://Prod/api/t", SecretUsage::Runtime),
            row("TOKEN", "op://Prod/api/t2", SecretUsage::Runtime),
            row("KEY", "op://Prod/api", SecretUsage::Runtime),
            row("ENV_VAR", "op-env://RRHH prod/DATABASE_URL", SecretUsage::Runtime),
        ];
        assert_eq!(ref_problems(&rows), [RefProblem::Name("1BAD".into()), RefProblem::Repeated("TOKEN".into()), RefProblem::Ref("KEY".into())]);
        assert!(!valid_secret_ref("op-env://RRHH prod/"));
        assert!(!valid_secret_ref("op:///api/x"));
        assert!(!valid_secret_name(&"A".repeat(41)));
    }

    /// Rotura que atrapa: perder un build arg que ya era secret de la app
    /// (pasa a «los dos»), duplicar uno, o añadir el que falta como secret de
    /// la app.
    #[test]
    fn build_args_join_the_secrets() {
        let rows = [row("DATABASE_URL", "op://p/a/u", SecretUsage::Runtime), row("NPM_TOKEN", "op://p/a/n", SecretUsage::Build)];
        let args = ["DATABASE_URL".to_string(), "NPM_TOKEN".to_string(), "SENTRY_DSN".to_string()];
        let (next, added, promoted) = merge_build_args(&rows, &args);
        assert_eq!((added, promoted), (1, 1));
        assert_eq!(next.iter().map(|r| (r.name.as_str(), r.usage)).collect::<Vec<_>>(), [("DATABASE_URL", SecretUsage::Both), ("NPM_TOKEN", SecretUsage::Build), ("SENTRY_DSN", SecretUsage::Build)]);
        assert_eq!(next[2].op_ref, "");
    }

    /// Rotura que atrapa: importar de 1Password nombres que no valen
    /// («api key» tal cual) o repetir los que ya estaban.
    #[test]
    fn labels_become_names_and_imports_skip_what_is_there() {
        assert_eq!(label_to_name(" api key "), "API_KEY");
        assert_eq!(label_to_name("2fa-secret"), "_2FA_SECRET");
        let rows = [row("API_KEY", "op://p/a/k", SecretUsage::Runtime)];
        let next = append_rows(&rows, [row("API_KEY", "op://p/b/k", SecretUsage::Runtime), row("SMTP_PASS", "op://p/b/s", SecretUsage::Runtime), row("mal", "op://p/b/m", SecretUsage::Runtime)]);
        assert_eq!(next.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["API_KEY", "SMTP_PASS"]);
        assert_eq!(next[0].op_ref, "op://p/a/k");
    }
}
