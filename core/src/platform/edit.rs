//! Dar de alta, editar y borrar servidores: quién puede qué, el formulario y
//! por qué no se hizo un cambio.

use serde::{Deserialize, Serialize};

use super::servers::{Server, ServerKind};
use crate::error::ApiError;
use crate::texts::{error_phrase, text, text_with};
use crate::Locale;

/// Lo que quien mira puede hacer con los servidores de la organización.
///
/// Espejo de `CreateServer`, `UpdateServer` y `DeleteServer` en
/// `backend/internal/adapters/handler/server.go`. El servidor lo vuelve a
/// decir con un 403; esto es para no ofrecer lo que va a rechazar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ServerPermissions {
    pub create: bool,
    /// Un servidor kubernetes lee el clúster de la plataforma con la cuenta
    /// del backend, y cualquiera puede crearse una organización y ser su
    /// admin: crearlo, o convertir uno en él, no depende del papel.
    pub kubernetes: bool,
    /// El nombre.
    pub edit: bool,
    /// A dónde se conecta (host, usuario, puertos). Con eso en otra máquina,
    /// la app de un admin mandaría allí su SSH y sus pases de agente.
    pub edit_connection: bool,
    pub delete: bool,
    /// Instalar o actualizar su agente: acuñar la identidad nueva revoca la
    /// anterior (`MintToken`, sólo admin).
    pub install_agent: bool,
    /// Reiniciar un servicio por su agente: el pase de un lector es de
    /// lectura (agente v5).
    pub restart_service: bool,
    /// Ver el secreto guardado de una integración de un clúster: miembro o
    /// más, como en Tauri.
    pub reveal_integrations: bool,
}

pub fn server_permissions(role: &str, superadmin: bool) -> ServerPermissions {
    let writer = superadmin || matches!(role, "admin" | "member");
    let admin = superadmin || role == "admin";
    ServerPermissions { create: writer, kubernetes: superadmin, edit: writer, edit_connection: admin, delete: admin, install_agent: admin, restart_service: writer, reveal_integrations: writer }
}

/// Un campo del formulario de un servidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ServerField {
    Name,
    Host,
    SshUser,
    SshPort,
    AgentPort,
}

impl ServerField {
    pub fn label_key(self) -> &'static str {
        match self {
            ServerField::Name => "common:servers.thName",
            ServerField::Host => "common:servers.host",
            ServerField::SshUser => "common:servers.sshUser",
            ServerField::SshPort => "common:servers.sshPort",
            ServerField::AgentPort => "common:servers.agentPort",
        }
    }
}

/// El formulario de alta y de edición, tal como se escribe: los puertos son
/// texto hasta que se mandan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ServerForm {
    pub name: String,
    pub host: String,
    pub ssh_user: String,
    pub ssh_port: String,
    pub agent_port: String,
    pub kind: ServerKind,
}

impl Default for ServerForm {
    /// Los de `AddServerDialog.tsx`.
    fn default() -> Self {
        ServerForm {
            name: String::new(),
            host: String::new(),
            ssh_user: String::new(),
            ssh_port: "22".into(),
            agent_port: "9090".into(),
            kind: ServerKind::DockerSwarm,
        }
    }
}

impl From<&Server> for ServerForm {
    fn from(server: &Server) -> Self {
        ServerForm {
            name: server.name.clone(),
            host: server.host.clone(),
            ssh_user: server.ssh_user.clone(),
            ssh_port: server.ssh_port.to_string(),
            agent_port: server.agent_port.to_string(),
            kind: server.kind,
        }
    }
}

/// El formulario ya comprobado, como lo pide `CreateServerRequest` /
/// `UpdateServerRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerFields {
    pub name: String,
    pub host: String,
    pub ssh_user: String,
    pub ssh_port: u16,
    pub agent_port: u16,
    pub kind: ServerKind,
}

impl ServerFields {
    pub fn body(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "host": self.host,
            "sshUser": self.ssh_user,
            "sshPort": self.ssh_port,
            "agentPort": self.agent_port,
            "type": self.kind.as_str(),
        })
    }

    pub fn same_connection(&self, server: &Server) -> bool {
        self.host == server.host && self.ssh_user == server.ssh_user && self.ssh_port == server.ssh_port && self.agent_port == server.agent_port
    }

    pub fn unchanged(&self, server: &Server) -> bool {
        self.same_connection(server) && self.name == server.name && self.kind == server.kind
    }
}

/// El tope de `name` en `CreateServerRequest` (`max=100`).
pub const MAX_SERVER_NAME: usize = 100;

impl ServerForm {
    /// Sin los espacios de los lados, y sólo si cada campo vale. Un tipo
    /// desconocido no se manda: el backend sólo acepta los dos.
    pub(crate) fn validate(&self) -> Result<ServerFields, ServerEditError> {
        let required = |value: &str, field| {
            let value = value.trim();
            if value.is_empty() {
                Err(ServerEditError::Required(field))
            } else {
                Ok(value.to_string())
            }
        };
        let port = |value: &str, field| match value.trim().parse::<u16>() {
            Ok(port) if port > 0 => Ok(port),
            _ => Err(ServerEditError::BadPort(field)),
        };
        let name = required(&self.name, ServerField::Name)?;
        if name.chars().count() > MAX_SERVER_NAME {
            return Err(ServerEditError::NameTooLong);
        }
        if self.kind == ServerKind::Unknown {
            return Err(ServerEditError::UnknownKind);
        }
        Ok(ServerFields {
            name,
            host: required(&self.host, ServerField::Host)?,
            ssh_user: required(&self.ssh_user, ServerField::SshUser)?,
            ssh_port: port(&self.ssh_port, ServerField::SshPort)?,
            agent_port: port(&self.agent_port, ServerField::AgentPort)?,
            kind: self.kind,
        })
    }
}

/// Por qué no se hizo un cambio en un servidor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServerEditError {
    #[error("required")]
    Required(ServerField),
    #[error("bad-port")]
    BadPort(ServerField),
    #[error("name-too-long")]
    NameTooLong,
    #[error("unknown-kind")]
    UnknownKind,
    /// Su papel en la organización no llega (los textos del backend).
    #[error("not-a-writer-in-org")]
    ReadOnly,
    #[error("kubernetes-superadmin-only")]
    KubernetesSuperadminOnly,
    #[error("connection-admin-only")]
    ConnectionAdminOnly,
    #[error("admin-required")]
    AdminOnly,
    #[error("no-organization")]
    NoOrganization,
    /// Ya no está en la lista de la organización que se mira.
    #[error("server-gone")]
    ServerGone,
    /// Ya se está instalando un agente.
    #[error("agent-busy")]
    AgentBusy,
    /// Lo que contestó el SSH al instalar el agente.
    #[error("{0}")]
    Ssh(String),
    #[error(transparent)]
    Api(#[from] ApiError),
}

/// Un fallo al cambiar un servidor, dicho para quien lo lee. Los de permiso
/// se dicen con la frase del código que contestaría el backend.
pub fn server_edit_phrase(locale: Locale, error: &ServerEditError, fallback_key: &str) -> String {
    let field = |field: ServerField, key: &str| format!("{}: {}", text(locale, field.label_key()), text(locale, key));
    match error {
        ServerEditError::Required(f) => field(*f, "common:servers.required"),
        ServerEditError::BadPort(f) => field(*f, "common:servers.badPort"),
        ServerEditError::NameTooLong => text_with(locale, "common:servers.nameTooLong", &[("max", &MAX_SERVER_NAME.to_string())]),
        ServerEditError::UnknownKind => text(locale, "common:servers.unknownKind"),
        ServerEditError::ServerGone => text(locale, "common:servers.serverGone"),
        ServerEditError::NoOrganization => text(locale, "work:tree.pickOrg"),
        ServerEditError::AgentBusy => text(locale, "common:servers.agentUpdate.busy"),
        ServerEditError::Ssh(message) => message.clone(),
        ServerEditError::ReadOnly | ServerEditError::KubernetesSuperadminOnly | ServerEditError::ConnectionAdminOnly | ServerEditError::AdminOnly => {
            text(locale, &format!("errors:{error}"))
        }
        ServerEditError::Api(error) => error_phrase(locale, error, Some(fallback_key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que atrapa: separarse de los handlers del backend, ofreciendo
    /// a un miembro borrar o a un admin crear kubernetes.
    #[test]
    fn permissions_mirror_the_backend() {
        let p = |role, superadmin| server_permissions(role, superadmin);
        assert_eq!(p("viewer", false), ServerPermissions::default());
        assert_eq!(p("member", false), ServerPermissions { create: true, edit: true, restart_service: true, reveal_integrations: true, ..Default::default() });
        assert_eq!(p("admin", false), ServerPermissions { create: true, edit: true, edit_connection: true, delete: true, kubernetes: false, install_agent: true, restart_service: true, reveal_integrations: true });
        assert_eq!(p("", true), ServerPermissions { create: true, edit: true, edit_connection: true, delete: true, kubernetes: true, install_agent: true, restart_service: true, reveal_integrations: true });
    }

    fn form() -> ServerForm {
        ServerForm { name: " prod-01 ".into(), host: " 10.0.0.1".into(), ssh_user: "root ".into(), ..ServerForm::default() }
    }

    /// Rotura que atrapa: mandar los espacios que se escribieron, o un
    /// puerto como texto.
    #[test]
    fn a_valid_form_is_sent_trimmed_with_numeric_ports() {
        assert_eq!(
            form().validate().unwrap().body(),
            serde_json::json!({ "name": "prod-01", "host": "10.0.0.1", "sshUser": "root", "sshPort": 22, "agentPort": 9090, "type": "docker-swarm" })
        );
    }

    /// Rotura que atrapa: dejar pasar un campo en blanco, un puerto fuera
    /// de rango o un tipo que el backend no acepta.
    #[test]
    fn each_bad_field_is_named() {
        let with = |edit: fn(&mut ServerForm)| {
            let mut form = form();
            edit(&mut form);
            form.validate().unwrap_err()
        };
        assert_eq!(with(|f| f.name = "  ".into()), ServerEditError::Required(ServerField::Name));
        assert_eq!(with(|f| f.host = String::new()), ServerEditError::Required(ServerField::Host));
        assert_eq!(with(|f| f.ssh_user = " ".into()), ServerEditError::Required(ServerField::SshUser));
        assert_eq!(with(|f| f.ssh_port = "0".into()), ServerEditError::BadPort(ServerField::SshPort));
        assert_eq!(with(|f| f.agent_port = "65536".into()), ServerEditError::BadPort(ServerField::AgentPort));
        assert_eq!(with(|f| f.agent_port = "abc".into()), ServerEditError::BadPort(ServerField::AgentPort));
        assert_eq!(with(|f| f.name = "x".repeat(MAX_SERVER_NAME + 1)), ServerEditError::NameTooLong);
        assert_eq!(with(|f| f.kind = ServerKind::Unknown), ServerEditError::UnknownKind);
        let mut longest = form();
        longest.name = "x".repeat(MAX_SERVER_NAME);
        assert!(longest.validate().is_ok());
    }

    /// Rotura que atrapa: una clave que no está en el catálogo (se pintaría
    /// la clave cruda) o un fallo que no dice qué campo.
    #[test]
    fn every_failure_reads_as_a_phrase() {
        let errors = [
            ServerEditError::Required(ServerField::Host),
            ServerEditError::BadPort(ServerField::AgentPort),
            ServerEditError::NameTooLong,
            ServerEditError::UnknownKind,
            ServerEditError::ServerGone,
            ServerEditError::NoOrganization,
            ServerEditError::ReadOnly,
            ServerEditError::KubernetesSuperadminOnly,
            ServerEditError::ConnectionAdminOnly,
            ServerEditError::AdminOnly,
        ];
        for error in &errors {
            let phrase = server_edit_phrase(Locale::Es, error, "common:misc.errSave");
            assert!(!phrase.contains("common:") && !phrase.contains("errors:") && !phrase.contains("{{"), "{error:?} → {phrase}");
        }
        assert_eq!(server_edit_phrase(Locale::Es, &ServerEditError::BadPort(ServerField::AgentPort), ""), "Puerto del agente: Un puerto va de 1 a 65535");
        assert_eq!(server_edit_phrase(Locale::Es, &ServerEditError::NameTooLong, ""), "Como mucho 100 caracteres");
    }
}
