//! Los servidores de una organización, tal como los cuenta el backend
//! (`domain.ServerResponse`), y lo que se decide sólo con mirarlos.

use serde::{Deserialize, Serialize};

/// Qué clase de máquina es. Cambia qué se le puede hacer: un servidor de
/// kubernetes no lleva el agente de swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "kebab-case")]
pub enum ServerKind {
    #[default]
    DockerSwarm,
    Kubernetes,
    /// Uno que esta versión de la app no conoce: se enseña, pero no se le
    /// supone nada.
    #[serde(other)]
    Unknown,
}

impl ServerKind {
    /// Como lo escribe el backend, que es como lo enseña la tabla.
    pub fn as_str(self) -> &'static str {
        match self {
            ServerKind::DockerSwarm => "docker-swarm",
            ServerKind::Kubernetes => "kubernetes",
            ServerKind::Unknown => "unknown",
        }
    }
}

/// Lo que el backend dice de su estado.
///
/// Con un agente con identidad es su latido; sin ella, lo que la última app
/// que miró pudo alcanzar (ver [`super::Platform::refresh`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum ServerHealth {
    /// Nadie lo ha mirado todavía: lo pone el backend al dar de alta.
    #[default]
    Pending,
    Online,
    Offline,
    Error,
    #[serde(other)]
    Unknown,
}

impl ServerHealth {
    /// La clave del catálogo que lo dice (`nav:panel.serverStatus.*`). Uno
    /// desconocido se lee como pendiente: es lo honesto, «no sé».
    pub fn label_key(self) -> &'static str {
        match self {
            ServerHealth::Online => "nav:panel.serverStatus.online",
            ServerHealth::Offline => "nav:panel.serverStatus.offline",
            ServerHealth::Error => "nav:panel.serverStatus.error",
            ServerHealth::Pending | ServerHealth::Unknown => "nav:panel.serverStatus.pending",
        }
    }

    /// Como lo escribe el backend.
    pub fn as_str(self) -> &'static str {
        match self {
            ServerHealth::Pending => "pending",
            ServerHealth::Online => "online",
            ServerHealth::Offline => "offline",
            ServerHealth::Error => "error",
            ServerHealth::Unknown => "unknown",
        }
    }

    /// Lo que se escribe en `agent-status`, que sólo acepta estos dos:
    /// `pending` es del backend ponerlo, no de un cliente devolverlo.
    pub(crate) fn as_reported(self) -> Option<&'static str> {
        matches!(self, ServerHealth::Online | ServerHealth::Offline).then(|| self.as_str())
    }
}

/// Un servidor, como lo devuelve `GET /api/v1/servers/`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub org_id: String,
    pub name: String,
    pub host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    #[serde(rename = "type")]
    pub kind: ServerKind,
    pub agent_port: u16,
    pub status: ServerHealth,
    /// Si su agente tiene identidad (la v2 en adelante). Con ella hablarle
    /// exige un pase del backend, y `status` es su latido.
    #[serde(default)]
    pub has_agent_token: bool,
    /// La versión que dijo el agente en su último latido; 0 si no la dice.
    #[serde(default)]
    pub agent_version: i64,
    /// Cuándo latió por última vez, tal como llega (RFC 3339).
    #[serde(default)]
    pub agent_seen_at: Option<String>,
}

impl Server {
    /// Cómo se escribe en la tabla: con el puerto de su agente si lo lleva.
    /// Uno de kubernetes no lo lleva, y su puerto no diría nada.
    pub fn address(&self) -> String {
        if self.kind == ServerKind::Kubernetes {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.agent_port)
        }
    }

    /// Si esta app tiene que preguntarle a su agente si contesta.
    ///
    /// Sólo los swarm sin identidad. Uno de kubernetes no lleva este agente
    /// (sondearlo sería inventarse una avería), y uno con identidad ya le
    /// late al backend: ponerle encima «lo que vi yo» pisaría lo que dice la
    /// máquina.
    pub fn needs_probe(&self) -> bool {
        self.kind == ServerKind::DockerSwarm && !self.has_agent_token
    }
}

/// Los de una organización, en el orden en que llegaron. El backend los
/// manda de todas las de la persona: la app enseña sólo la que se mira.
pub fn servers_of(all: &[Server], org_id: Option<&str>) -> Vec<Server> {
    let Some(org_id) = org_id else { return Vec::new() };
    all.iter().filter(|server| server.org_id == org_id).cloned().collect()
}

/// Una fila del panel de Plataforma.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PlatformSection {
    #[default]
    Servers,
    Activity,
    Diagnostics,
    Users,
    DevTools,
}

impl PlatformSection {
    pub fn label_key(self) -> &'static str {
        match self {
            PlatformSection::Servers => "nav:item.servers",
            PlatformSection::Activity => "nav:item.activity",
            PlatformSection::Diagnostics => "nav:item.diagnostics",
            PlatformSection::Users => "nav:item.users",
            PlatformSection::DevTools => "common:misc.devtools",
        }
    }
}

/// Las filas del panel, en el orden de la app de Tauri (`lib/nav.ts`).
/// Usuarios es sólo para quien administra la plataforma entera.
pub fn platform_sections(superadmin: bool) -> Vec<PlatformSection> {
    use PlatformSection::*;
    [Servers, Activity, Diagnostics, Users, DevTools]
        .into_iter()
        .filter(|section| *section != Users || superadmin)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(id: &str, org: &str) -> Server {
        Server { id: id.into(), org_id: org.into(), ..Server::default() }
    }

    /// Rotura que atrapa: enseñar los servidores de todas las organizaciones
    /// juntos, o los de alguna sin haber elegido ninguna.
    #[test]
    fn only_the_servers_of_the_organization_you_look_at() {
        let all = [server("a", "o1"), server("b", "o2"), server("c", "o1")];
        let ids = |org| servers_of(&all, org).into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(ids(Some("o1")), ["a", "c"]);
        assert_eq!(ids(Some("o2")), ["b"]);
        assert!(ids(None).is_empty());
    }

    /// Rotura que atrapa: sondear un kubernetes (se pintaría caído) o un
    /// agente con identidad (se pisaría su latido).
    #[test]
    fn only_a_swarm_without_identity_is_probed() {
        let mut s = server("a", "o1");
        assert!(s.needs_probe());
        s.has_agent_token = true;
        assert!(!s.needs_probe());
        s.has_agent_token = false;
        s.kind = ServerKind::Kubernetes;
        assert!(!s.needs_probe());
    }

    /// Rotura que atrapa: pintar un puerto de agente en un kubernetes, que no
    /// lo tiene.
    #[test]
    fn a_kubernetes_server_has_no_agent_port() {
        let mut s = Server { host: "10.0.0.1".into(), agent_port: 8080, ..Server::default() };
        assert_eq!(s.address(), "10.0.0.1:8080");
        s.kind = ServerKind::Kubernetes;
        assert_eq!(s.address(), "10.0.0.1");
    }

    /// Rotura que atrapa: enseñar «Usuarios» a quien no es superadmin, o
    /// quitárselo a quien sí.
    #[test]
    fn users_is_only_for_a_superadmin() {
        assert!(!platform_sections(false).contains(&PlatformSection::Users));
        assert_eq!(
            platform_sections(true),
            [PlatformSection::Servers, PlatformSection::Activity, PlatformSection::Diagnostics, PlatformSection::Users, PlatformSection::DevTools]
        );
    }

    /// Rotura que atrapa: escribir en `agent-status` algo que el backend no
    /// acepta (`pending` es suyo).
    #[test]
    fn only_online_and_offline_are_reported() {
        assert_eq!(ServerHealth::Online.as_reported(), Some("online"));
        assert_eq!(ServerHealth::Offline.as_reported(), Some("offline"));
        assert_eq!(ServerHealth::Pending.as_reported(), None);
        assert_eq!(ServerHealth::Error.as_reported(), None);
    }
}
