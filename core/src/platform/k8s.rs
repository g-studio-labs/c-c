//! La consola de un servidor kubernetes (`K8sHub.tsx`, `K8sStatusBadge.tsx`
//! e `IntegrationsSection.tsx`): sus rutas, la salud del clúster y las
//! herramientas que cuelgan de él. La lee el backend con su cuenta del
//! clúster (`domain/k8s.go`, `domain/integration.go`); aquí sólo se mira.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct K8sGateway {
    pub name: String,
    pub namespace: String,
    pub address: String,
    pub programmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct K8sRoute {
    pub name: String,
    pub namespace: String,
    pub gateway: String,
    pub hostnames: Vec<String>,
    /// Uno por nombre, en su orden.
    pub links: Vec<String>,
    /// Sin certificado que case, no viene.
    pub cert_ready: Option<bool>,
    pub cert_expiry: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct K8sRoutes {
    pub gateways: Vec<K8sGateway>,
    pub routes: Vec<K8sRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct K8sNode {
    pub name: String,
    pub kubelet_version: String,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct K8sWorkload {
    pub name: String,
    pub namespace: String,
    pub ready: i64,
    pub desired: i64,
    pub healthy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct K8sCert {
    pub name: String,
    pub namespace: String,
    pub dns_names: Vec<String>,
    pub not_after: Option<String>,
    pub days_left: Option<i64>,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct K8sDatastore {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub instances: i64,
    pub ready: i64,
    pub phase: String,
    pub healthy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct K8sHealth {
    pub nodes: Vec<K8sNode>,
    pub workloads: Vec<K8sWorkload>,
    pub certs: Vec<K8sCert>,
    pub datastores: Vec<K8sDatastore>,
}

/// Una herramienta del clúster (Grafana, pgAdmin…), sin su secreto: sólo si
/// lo tiene.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct Integration {
    pub id: String,
    pub server_id: String,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub auth_method: String,
    pub has_secret: bool,
    pub hidden: bool,
}

impl Integration {
    /// Con `header` se entra por el proxy de cac, que firma por ti
    /// (`auth.proxy` de Grafana): hay que pedirle el enlace.
    pub fn through_cac(&self) -> bool {
        self.auth_method == "header"
    }
}

/// La consola de un servidor kubernetes, tal como llegó. Lo que falle se
/// queda en `None` y lo dice `error`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct K8sHub {
    pub server_id: String,
    pub routes: Option<K8sRoutes>,
    pub health: Option<K8sHealth>,
    pub integrations: Vec<Integration>,
    pub error: Option<String>,
}

/// La etiqueta de un clúster en la lista. La columna `status` del servidor
/// sigue el ciclo del agente de swarm, que un clúster no lleva: diría
/// «pendiente» para siempre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ClusterStatus {
    Healthy { nodes: u32 },
    /// Corre, con cosas por debajo de lo pedido. `detail`, una por línea.
    Issues { count: u32, detail: String },
    /// Un nodo no está listo: el clúster está caído.
    Down { detail: String },
    /// No se pudo leer.
    Unknown { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClusterBadge {
    pub server_id: String,
    pub status: ClusterStatus,
}

/// Un nodo que no está listo es el clúster caído; un deployment por debajo
/// de sus réplicas no: en un clúster de un nodo, lo que pide dos se queda
/// corto para siempre, y la etiqueta estaría siempre en rojo y no diría nada.
/// Eso se cuenta aparte.
pub fn cluster_status(health: &K8sHealth) -> ClusterStatus {
    let down: Vec<&str> = health.nodes.iter().filter(|node| !node.ready).map(|node| node.name.as_str()).collect();
    if !down.is_empty() {
        return ClusterStatus::Down { detail: down.join(", ") };
    }
    let issues: Vec<String> = health
        .workloads
        .iter()
        .filter(|workload| !workload.healthy)
        .map(|workload| format!("{}/{} {}/{}", workload.namespace, workload.name, workload.ready, workload.desired))
        .chain(health.datastores.iter().filter(|store| !store.healthy).map(|store| format!("{}/{} ({})", store.namespace, store.name, store.phase)))
        .collect();
    if issues.is_empty() {
        ClusterStatus::Healthy { nodes: health.nodes.len() as u32 }
    } else {
        ClusterStatus::Issues { count: issues.len() as u32, detail: issues.join("\n") }
    }
}

/// Cada nombre de una ruta con el enlace que abre: el que da el backend, o
/// `https://` y el nombre si faltara.
pub fn route_links(route: &K8sRoute) -> Vec<(String, String)> {
    route.hostnames.iter().enumerate().map(|(n, host)| (host.clone(), route.links.get(n).cloned().unwrap_or_else(|| format!("https://{host}")))).collect()
}

/// Lo que un certificado tiene de margen antes de avisar: tres semanas, lo
/// que cert-manager tarda en reintentar con calma.
pub const CERT_WARN_DAYS: i64 = 21;

/// Si un certificado avisa: no está listo, o le quedan menos de
/// `CERT_WARN_DAYS`.
pub fn cert_warns(ready: bool, days_left: Option<i64>) -> bool {
    !ready || days_left.is_some_and(|days| days < CERT_WARN_DAYS)
}

/// Los días que le quedan a un certificado de una ruta, por su caducidad.
pub fn days_until(expiry: &str, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let at = chrono::DateTime::parse_from_rfc3339(expiry).ok()?;
    Some((at.with_timezone(&chrono::Utc) - now).num_days())
}

/// Los workloads con los que fallan primero: son lo que se viene a ver.
pub fn workloads_failing_first(workloads: &[K8sWorkload]) -> Vec<K8sWorkload> {
    let mut sorted = workloads.to_vec();
    sorted.sort_by_key(|workload| workload.healthy);
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, ready: bool) -> K8sNode {
        K8sNode { name: name.into(), kubelet_version: "v1.35.10".into(), ready }
    }

    fn workload(name: &str, ready: i64, desired: i64) -> K8sWorkload {
        K8sWorkload { name: name.into(), namespace: "cac".into(), ready, desired, healthy: ready >= desired }
    }

    /// Rotura que atrapa: pintar en rojo un clúster de un nodo porque algo
    /// pide dos réplicas, o dar por sano uno con un nodo caído o una base
    /// que no arranca.
    #[test]
    fn a_short_workload_is_an_issue_and_a_down_node_is_down() {
        let mut health = K8sHealth { nodes: vec![node("vmi1", true)], workloads: vec![workload("api", 1, 1)], ..Default::default() };
        assert_eq!(cluster_status(&health), ClusterStatus::Healthy { nodes: 1 });

        health.workloads.push(workload("web", 1, 2));
        health.datastores.push(K8sDatastore { name: "postgres".into(), namespace: "data".into(), phase: "Setting up primary".into(), healthy: false, ..Default::default() });
        assert_eq!(cluster_status(&health), ClusterStatus::Issues { count: 2, detail: "cac/web 1/2\ndata/postgres (Setting up primary)".into() });

        health.nodes.push(node("vmi2", false));
        assert_eq!(cluster_status(&health), ClusterStatus::Down { detail: "vmi2".into() });
    }

    /// Rotura que atrapa: abrir el enlace de otro nombre de la misma ruta, o
    /// no abrir nada cuando el backend no lo trae.
    #[test]
    fn each_hostname_opens_its_own_link() {
        let route = K8sRoute { hostnames: vec!["a.dev".into(), "b.dev".into()], links: vec!["https://a.dev/".into()], ..Default::default() };
        assert_eq!(route_links(&route), [("a.dev".into(), "https://a.dev/".into()), ("b.dev".into(), "https://b.dev".into())]);
    }

    /// Rotura que atrapa: no avisar de un certificado que caduca en días, o
    /// avisar de todos.
    #[test]
    fn a_cert_warns_three_weeks_before_or_when_not_ready() {
        assert!(!cert_warns(true, Some(60)));
        assert!(!cert_warns(true, None));
        assert!(cert_warns(true, Some(20)));
        assert!(!cert_warns(true, Some(21)));
        assert!(cert_warns(false, Some(60)));
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-21T00:00:00Z").unwrap().with_timezone(&chrono::Utc);
        assert_eq!(days_until("2026-12-01T12:00:00Z", now), Some(10));
        assert_eq!(days_until("roto", now), None);
    }

    /// Rotura que atrapa: dejar al fondo lo que falla, que es lo que se viene
    /// a ver.
    #[test]
    fn failing_workloads_come_first() {
        let sorted = workloads_failing_first(&[workload("a", 1, 1), workload("b", 0, 1), workload("c", 2, 2), workload("d", 1, 3)]);
        assert_eq!(sorted.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["b", "d", "a", "c"]);
    }
}
