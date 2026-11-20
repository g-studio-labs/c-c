//! Plataforma: los servidores de la organización que se mira y lo que se
//! hace con ellos.
//!
//! Es el cuarto módulo del riel. Aquí van las reglas y el estado; la pintura,
//! en `desktop/src/platform/`. El diseño, en cac: «Diseño · Plataforma en la
//! app nativa». Cada fase añade su fichero junto a éstos.

pub mod agent;
pub mod servers;

use serde::{Deserialize, Serialize};

use crate::Core;
pub use agent::{agent_base, Agent, AgentStream, AGENT_TIMEOUT, PASS_MARGIN, PROBE_TIMEOUT};
pub use servers::{platform_sections, servers_of, PlatformSection, Server, ServerHealth, ServerKind};

/// Lo que se pinta de Plataforma.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct PlatformState {
    /// Se está en Plataforma: el panel son sus filas y la vista, sus páginas.
    pub on_screen: bool,
    /// De qué organización son los servidores de abajo.
    pub org_id: Option<String>,
    /// Los servidores de esa organización, en el orden del backend.
    pub servers: Vec<Server>,
    /// Si ya llegó la lista alguna vez: sin ella, «no hay servidores» sería
    /// mentira.
    pub loaded: bool,
    pub loading: bool,
    /// La frase del último fallo al pedirla. La lista que hubiera se queda.
    pub error: Option<String>,
    /// El servidor abierto, si se entró en uno.
    pub open_server: Option<String>,
}

#[derive(Default)]
pub(crate) struct PlatformData {
    on_screen: bool,
    /// Los de todas las organizaciones de la persona, como llegan.
    all: Option<Vec<Server>>,
    loading: bool,
    error: Option<String>,
    open_server: Option<String>,
}

/// Plataforma en este core.
pub struct Platform {
    core: Core,
}

impl Core {
    pub fn platform(&self) -> Platform {
        Platform { core: self.clone() }
    }
}

impl Platform {
    pub fn state(&self) -> PlatformState {
        let org_id = self.core.orgs().state().current_org_id;
        let data = self.data();
        PlatformState {
            on_screen: data.on_screen,
            servers: data.all.as_deref().map(|all| servers_of(all, org_id.as_deref())).unwrap_or_default(),
            org_id,
            loaded: data.all.is_some(),
            loading: data.loading,
            error: data.error.clone(),
            open_server: data.open_server.clone(),
        }
    }

    /// Entrar a Plataforma. Sólo un módulo está en pantalla a la vez: deja
    /// comunicación si se estaba en ella.
    pub fn enter(&self) {
        self.core.comms().leave();
        self.change(|data| data.on_screen = true);
    }

    /// Volver a lo demás. El servidor abierto se queda para la vuelta.
    pub fn leave(&self) {
        if self.data().on_screen {
            self.change(|data| data.on_screen = false);
        }
    }

    /// Pide los servidores y, a los agentes de los de antes, si contestan.
    ///
    /// Lo que falle deja la lista que hubiera: un fallo de red no es «no
    /// tienes servidores».
    pub async fn refresh(&self) {
        self.change(|data| {
            data.loading = true;
            data.error = None;
        });
        match self.core.get_list::<Server>("/api/v1/servers/").await {
            Ok(servers) => {
                self.core.counters().servers_seen(&servers);
                self.change(|data| {
                    data.all = Some(servers);
                    data.loading = false;
                });
                self.probe_old_agents().await;
            }
            Err(error) => self.change(|data| {
                data.error = Some(error.to_string());
                data.loading = false;
            }),
        }
    }

    /// Entra en un servidor. Uno que no es de la organización que se mira no
    /// se abre.
    pub fn open_server(&self, server_id: &str) -> bool {
        if !self.state().servers.iter().any(|server| server.id == server_id) {
            return false;
        }
        self.change(|data| data.open_server = Some(server_id.to_string()));
        true
    }

    /// Vuelve a la lista de servidores.
    pub fn close_server(&self) {
        self.change(|data| data.open_server = None);
    }

    /// Se cambió de organización: la lista es la misma respuesta, filtrada
    /// por otra; el servidor abierto era de la anterior.
    pub(crate) fn org_changed(&self) {
        self.change(|data| data.open_server = None);
    }

    /// Al salir: los servidores y los pases de una persona no son de la
    /// siguiente.
    pub(crate) fn clear(&self) {
        self.core.agent().clear();
        self.change(|data| *data = PlatformData::default());
    }

    /// Pregunta a los agentes sin identidad si contestan, y lo que cambie lo
    /// escribe en el backend (`agent-status`).
    ///
    /// Lo hace la app y no el servidor porque el agente vive en la VPS del
    /// cliente, y quien alcanza esa red es este escritorio. El estado es,
    /// dicho tal cual, «lo que la última app que miró pudo alcanzar». Sin
    /// esto, uno dado de alta se quedaba «pendiente» para siempre aunque su
    /// agente estuviera en pie (`use-servers.ts`).
    async fn probe_old_agents(&self) {
        let candidates: Vec<Server> = self.data().all.iter().flatten().filter(|server| server.needs_probe()).cloned().collect();
        let probes: Vec<_> = candidates
            .into_iter()
            .map(|server| {
                let core = self.core.clone();
                tokio::spawn(async move {
                    let seen = if core.agent().alive(&server).await { ServerHealth::Online } else { ServerHealth::Offline };
                    (server, seen)
                })
            })
            .collect();
        let mut changed = Vec::new();
        for probe in probes {
            if let Ok((server, seen)) = probe.await {
                if server.status != seen {
                    changed.push((server.id, seen));
                }
            }
        }
        if changed.is_empty() {
            return;
        }
        self.change(|data| {
            for server in data.all.iter_mut().flatten() {
                if let Some((_, seen)) = changed.iter().find(|(id, _)| *id == server.id) {
                    server.status = *seen;
                }
            }
        });
        if let Some(all) = self.data().all.clone() {
            self.core.counters().servers_seen(&all);
        }
        for (id, seen) in changed {
            let Some(status) = seen.as_reported() else { continue };
            // Si no se pudo escribir, la próxima app que mire lo vuelve a
            // intentar: no es algo que enseñar.
            let _ = self.core.call_ok("POST", &format!("/api/v1/servers/{id}/agent-status"), Some(serde_json::json!({ "status": status }))).await;
        }
    }

    fn data(&self) -> std::sync::MutexGuard<'_, PlatformData> {
        self.core.inner.platform.lock().expect("platform lock")
    }

    fn change(&self, edit: impl FnOnce(&mut PlatformData)) {
        edit(&mut self.data());
        self.core.inner.listener.platform_changed(self.state());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que atrapa: leer el contrato a mano y que se separe del
    /// backend. El fixture lo escribe `domain.TestCoreContract`.
    #[test]
    fn a_server_is_read_as_the_backend_writes_it() {
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/server_response.json")).unwrap();
        let server: Server = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            (server.name.as_str(), server.address(), server.ssh_port, server.ssh_user.as_str()),
            ("pixel-vps", "203.0.113.7:8080".to_string(), 22, "deploy")
        );
        assert_eq!((server.kind, server.status), (ServerKind::DockerSwarm, ServerHealth::Offline));
        assert!(server.has_agent_token);
        assert_eq!(server.agent_version, 2);
        assert_eq!(server.agent_seen_at.as_deref(), Some("2026-09-30T00:00:00Z"));
    }

    /// Rotura que atrapa: que un tipo o un estado nuevo del backend tire la
    /// lista entera en vez de enseñarse como desconocido.
    #[test]
    fn an_unknown_kind_or_status_does_not_break_the_list() {
        let server: Server = serde_json::from_value(serde_json::json!({
            "id": "s", "orgId": "o", "name": "n", "host": "h", "sshPort": 22, "sshUser": "u",
            "type": "nomad", "agentPort": 1, "status": "rebooting", "agentVersion": 0
        }))
        .unwrap();
        assert_eq!((server.kind, server.status), (ServerKind::Unknown, ServerHealth::Unknown));
        assert!(!server.needs_probe());
    }
}
