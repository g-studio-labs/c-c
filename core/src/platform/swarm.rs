//! Los nodos de un swarm y lo que gasta cada contenedor (`ServerNodes.tsx`,
//! `ServerStats.tsx` y `stats.store.ts`). Los da el agente del servidor.

use serde::{Deserialize, Serialize};

/// Un nodo, como lo escribe el agente (`domain.Node` de `swarm-manage`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct SwarmNode {
    pub id: String,
    pub hostname: String,
    pub role: String,
    pub status: String,
    pub availability: String,
    pub engine_version: String,
}

/// Lo que gasta un contenedor, en contadores acumulados desde que arrancó
/// (`domain.ContainerStats`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ContainerStats {
    pub container_id: String,
    pub task_id: String,
    pub node_id: String,
    pub service_id: String,
    pub service_name: String,
    pub stack: String,
    pub state: String,
    /// Como lo cuenta Docker: 100 por cada núcleo lleno.
    pub cpu_percent: f64,
    pub online_cpus: i64,
    pub mem_usage: u64,
    pub mem_limit: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub block_read: u64,
    pub block_write: u64,
    /// Por qué el agente no pudo leer ése; los números vienen a cero.
    pub error: Option<String>,
}

impl ContainerStats {
    /// El identificador corto, como lo enseña `docker ps`.
    pub fn short_id(&self) -> &str {
        self.container_id.get(..12).unwrap_or(&self.container_id)
    }

    /// Lo lleno de la CPU, de 0 a 1, contra lo que da la máquina entera: 200 %
    /// en una de cuatro núcleos es media barra, no una barra llena.
    pub fn cpu_fill(&self) -> f32 {
        let max = (self.online_cpus.max(1) * 100) as f64;
        (self.cpu_percent / max).clamp(0., 1.) as f32
    }

    pub fn mem_fill(&self) -> f32 {
        if self.mem_limit == 0 {
            return 0.;
        }
        (self.mem_usage as f64 / self.mem_limit as f64).clamp(0., 1.) as f32
    }
}

/// Cómo de cargado va algo, para el color de su barra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Load {
    Low,
    Medium,
    High,
}

/// Los umbrales de la app de Tauri: la CPU avisa antes (40 % y 75 %) que la
/// memoria (60 % y 85 %), que llena a medias es lo normal.
pub fn cpu_load(fill: f32) -> Load {
    load(fill, 0.40, 0.75)
}

pub fn mem_load(fill: f32) -> Load {
    load(fill, 0.60, 0.85)
}

fn load(fill: f32, medium: f32, high: f32) -> Load {
    if fill >= high {
        Load::High
    } else if fill >= medium {
        Load::Medium
    } else {
        Load::Low
    }
}

/// Por qué columna se ordena la tabla de contenedores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum StatsColumn {
    Container,
    Service,
    Stack,
    State,
    Cpu,
    Memory,
    NetRx,
    NetTx,
    DiskRead,
    DiskWrite,
}

impl StatsColumn {
    /// Al pulsar una columna nueva: el texto de la A a la Z, los números del
    /// que más gasta al que menos, que es lo que se busca.
    pub fn first_descending(self) -> bool {
        !matches!(self, StatsColumn::Container | StatsColumn::Service | StatsColumn::Stack | StatsColumn::State)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StatsSort {
    pub column: StatsColumn,
    pub descending: bool,
}

impl Default for StatsSort {
    fn default() -> Self {
        StatsSort { column: StatsColumn::Cpu, descending: true }
    }
}

impl StatsSort {
    /// Pulsar la misma columna le da la vuelta; otra empieza en su sentido.
    pub fn toggled(self, column: StatsColumn) -> StatsSort {
        if column == self.column {
            StatsSort { column, descending: !self.descending }
        } else {
            StatsSort { column, descending: column.first_descending() }
        }
    }
}

/// Lo que se enseña de la tabla de contenedores.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StatsView {
    pub rows: Vec<ContainerStats>,
    /// Los parados que se quitaron por no pedir verlos.
    pub hidden_stopped: u32,
}

/// Los contenedores que se ven: sin los parados salvo que se pidan, los que
/// casan con lo escrito (contenedor, servicio o stack) y en el orden pedido.
pub fn stats_view(stats: &[ContainerStats], filter: &str, show_stopped: bool, sort: StatsSort) -> StatsView {
    let needle = filter.trim().to_lowercase();
    let running: Vec<&ContainerStats> = stats.iter().filter(|row| show_stopped || row.state == "running").collect();
    let hidden_stopped = (stats.len() - running.len()) as u32;
    let mut rows: Vec<ContainerStats> = running
        .into_iter()
        .filter(|row| needle.is_empty() || format!("{} {} {}", row.container_id, row.service_name, row.stack).to_lowercase().contains(&needle))
        .cloned()
        .collect();
    rows.sort_by(|a, b| {
        use StatsColumn::*;
        let order = match sort.column {
            Container => a.container_id.cmp(&b.container_id),
            Service => a.service_name.cmp(&b.service_name),
            Stack => a.stack.cmp(&b.stack),
            State => a.state.cmp(&b.state),
            Cpu => a.cpu_percent.total_cmp(&b.cpu_percent),
            Memory => a.mem_usage.cmp(&b.mem_usage),
            NetRx => a.net_rx.cmp(&b.net_rx),
            NetTx => a.net_tx.cmp(&b.net_tx),
            DiskRead => a.block_read.cmp(&b.block_read),
            DiskWrite => a.block_write.cmp(&b.block_write),
        };
        if sort.descending {
            order.reverse()
        } else {
            order
        }
    });
    StatsView { rows, hidden_stopped }
}

/// Bytes para leer, como `formatBytes` de la app de Tauri: de 1024 en 1024,
/// con dos decimales por debajo de 10 y uno por debajo de 100.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".into();
    }
    let (mut value, mut unit) = (bytes as f64, 0);
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    let number = match (unit, value) {
        (0, _) => format!("{}", value.round()),
        (_, v) if v < 10. => format!("{v:.2}"),
        (_, v) if v < 100. => format!("{v:.1}"),
        (_, v) => format!("{}", v.round()),
    };
    format!("{number} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, service: &str, stack: &str, state: &str, cpu: f64, mem: u64) -> ContainerStats {
        ContainerStats {
            container_id: id.into(),
            service_name: service.into(),
            stack: stack.into(),
            state: state.into(),
            cpu_percent: cpu,
            mem_usage: mem,
            ..Default::default()
        }
    }

    fn ids(view: &StatsView) -> Vec<&str> {
        view.rows.iter().map(|row| row.container_id.as_str()).collect()
    }

    /// Rotura que atrapa: enseñar los parados sin pedirlos (o no contarlos),
    /// buscar sólo en una columna, u ordenar al revés de lo pedido.
    #[test]
    fn the_table_hides_the_stopped_filters_and_sorts() {
        let all = [
            row("aaa", "shop_web", "shop", "running", 12.5, 300),
            row("bbb", "shop_db", "shop", "running", 80., 100),
            row("ccc", "old_job", "", "exited", 0., 0),
            row("ddd", "metrics", "monitoring", "running", 3., 900),
        ];
        let by_cpu = stats_view(&all, "", false, StatsSort::default());
        assert_eq!(ids(&by_cpu), ["bbb", "aaa", "ddd"]);
        assert_eq!(by_cpu.hidden_stopped, 1);

        let with_stopped = stats_view(&all, "", true, StatsSort { column: StatsColumn::Memory, descending: false });
        assert_eq!(ids(&with_stopped), ["ccc", "bbb", "aaa", "ddd"]);
        assert_eq!(with_stopped.hidden_stopped, 0);

        assert_eq!(ids(&stats_view(&all, " SHOP ", false, StatsSort::default())), ["bbb", "aaa"]);
        assert_eq!(ids(&stats_view(&all, "monitor", false, StatsSort::default())), ["ddd"]);
        assert_eq!(ids(&stats_view(&all, "dd", false, StatsSort::default())), ["ddd"]);
        assert_eq!(ids(&stats_view(&all, "", false, StatsSort { column: StatsColumn::Service, descending: false })), ["ddd", "bbb", "aaa"]);
    }

    /// Rotura que atrapa: que pulsar la misma columna no le dé la vuelta, o
    /// que una columna de números empiece por el que menos gasta.
    #[test]
    fn clicking_a_column_turns_it_or_starts_it_its_way() {
        let cpu = StatsSort::default();
        assert_eq!(cpu.toggled(StatsColumn::Cpu), StatsSort { column: StatsColumn::Cpu, descending: false });
        assert_eq!(cpu.toggled(StatsColumn::Service), StatsSort { column: StatsColumn::Service, descending: false });
        assert_eq!(cpu.toggled(StatsColumn::Service).toggled(StatsColumn::NetTx), StatsSort { column: StatsColumn::NetTx, descending: true });
    }

    /// Rotura que atrapa: llenar la barra de CPU contra un solo núcleo, o
    /// dividir por cero con un contenedor sin límite de memoria.
    #[test]
    fn the_bars_fill_against_the_whole_machine() {
        let mut busy = row("a", "", "", "running", 200., 50);
        busy.online_cpus = 4;
        busy.mem_limit = 100;
        assert_eq!(busy.cpu_fill(), 0.5);
        assert_eq!(busy.mem_fill(), 0.5);
        assert_eq!(cpu_load(busy.cpu_fill()), Load::Medium);
        assert_eq!(mem_load(busy.mem_fill()), Load::Low);
        busy.mem_limit = 0;
        assert_eq!(busy.mem_fill(), 0.);
        busy.online_cpus = 0;
        assert_eq!(busy.cpu_fill(), 1.);
        assert_eq!((cpu_load(0.75), mem_load(0.85), mem_load(0.6)), (Load::High, Load::High, Load::Medium));
    }

    /// Rotura que atrapa: contar de 1000 en 1000, o perder los decimales que
    /// distinguen 1,5 GB de 2 GB.
    #[test]
    fn bytes_read_like_the_other_app() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.50 kB");
        assert_eq!(format_bytes(50 * 1024 * 1024), "50.0 MB");
        assert_eq!(format_bytes(300 * 1024 * 1024 * 1024), "300 GB");
        assert_eq!(format_bytes(1024u64.pow(5) * 2), "2048 TB");
        assert_eq!(row("0123456789abcdef", "", "", "", 0., 0).short_id(), "0123456789ab");
    }
}
