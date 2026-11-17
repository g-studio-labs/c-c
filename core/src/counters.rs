//! Los contadores de la campana y del riel: avisos sin leer, canales y
//! directos sin leer, servidores que no están en línea e invitaciones
//! pendientes.
//!
//! Vino de cuatro sitios de la app de Tauri (`inbox.store.ts`,
//! `use-comms-unread.ts` con `lib/unread.ts`, `use-servers.ts` e
//! `invitations.store.ts`), cada uno con su carga y su momento. Aquí es un
//! solo estado, porque los cuatro números contestan a lo mismo —«¿hay algo
//! que mirar?»— y los cuatro se equivocan igual: contando lo de otra
//! organización.
//!
//! De los cinco sitios a los que se pregunta, **sólo la campana se acota en
//! el servidor** (`?orgId=`), y sin ese parámetro contesta con lo de todas las
//! organizaciones de la persona: por eso no se pregunta nunca sin él. Canales,
//! directos y servidores llegan siempre de todas (de toda la plataforma para
//! un superadmin) y se acotan aquí, al contar. Las invitaciones son de la
//! persona, no de una organización.
//!
//! Un número que no se ha podido cargar es `None`, que no es cero: la
//! pantalla no pinta nada, ni un «0» ni un aviso de error.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::stream::ServerEvent;
use crate::Core;

/// Hasta qué número se escribe entero en la campana; por encima, `9+`.
pub const BELL_CAP: i64 = 9;
/// Lo mismo en el riel, que tiene sitio para dos cifras: `99+`.
pub const RAIL_CAP: i64 = 99;

/// Un respiro antes de volver a contar: diez mensajes seguidos en un canal
/// son diez eventos, y la cuenta a la que se llega es la misma.
const COALESCE: Duration = Duration::from_millis(400);

/// Lo que se escribe en un contador: nada si no hay nada que contar o no se
/// sabe, el número, o el tope con un «+».
pub fn count_badge(count: Option<i64>, cap: i64) -> Option<String> {
    let count = count.filter(|count| *count > 0)?;
    Some(if count > cap { format!("{cap}+") } else { count.to_string() })
}

/// El nombre de un botón con lo que tiene pendiente: «Comunicación (3)». El
/// número va entero: el tope es del dibujo, que no tiene sitio.
pub fn counted_label(label: &str, count: Option<i64>) -> String {
    match count.filter(|count| *count > 0) {
        Some(count) => format!("{label} ({count})"),
        None => label.to_string(),
    }
}

/// Los eventos que dejan fila en la campana, siempre.
///
/// El servidor guarda el aviso en su tabla, pero la campana no se entera
/// sola: hay que volver a preguntar. Como `DEJAN_FILA` en
/// `app/src/hooks/use-report-events.ts`, que existe porque hubo un tiempo en
/// que sólo tres de estos la refrescaban.
const RING_THE_BELL: &[&str] = &[
    "report:new",
    "telemetry:alert",
    "report:comment",
    "task:comment",
    "task:assigned",
    "task:status",
    "chat:message",
    "chat:mention",
    "dm:message",
    "meeting:reminder",
    "agent:notice",
    "post:new",
    "post:comment",
];

#[derive(Deserialize)]
struct Progress {
    status: Option<String>,
}

/// Si un evento del servidor deja fila en la campana.
///
/// Tres lo hacen según lo que traen: un run del CI sólo al terminar, un
/// despliegue sólo al acabar, y un backup sólo si falla. Mientras corren no
/// hay nada nuevo que contar.
pub fn rings_bell(event: &str, data: &str) -> bool {
    if RING_THE_BELL.contains(&event) {
        return true;
    }
    let status = serde_json::from_str::<Progress>(data).ok().and_then(|progress| progress.status);
    match (event, status.as_deref()) {
        ("ci:run", Some("completed")) => true,
        ("deploy:status", Some("succeeded" | "failed")) => true,
        ("backup:status", Some("failed")) => true,
        _ => false,
    }
}

/// Lo que se pinta: los cuatro números de la organización que se mira.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct CountersState {
    /// De qué organización son.
    pub org_id: Option<String>,
    /// Avisos sin leer: la campana.
    pub notifications: Option<i64>,
    /// Canales y directos sin leer, juntos: el módulo de comunicación.
    pub comms: Option<i64>,
    /// Servidores que no están en línea: el módulo de plataforma.
    pub servers: Option<i64>,
    /// Invitaciones pendientes de quien está dentro: el botón de la
    /// organización. Son de la persona, no de la organización que mira.
    pub invitations: Option<i64>,
}

/// Lo que se lee de `domain.NotificationFeed`: sólo la cuenta. Las filas son
/// de la bandeja, que aquí no se pinta.
#[derive(Deserialize)]
struct Feed {
    unread: i64,
}

/// Lo sin leer de un canal. Espejo de `domain.ChatUnread`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatUnread {
    space_id: String,
    count: i64,
}

/// Lo que se cuenta de `domain.DMSummary`: de qué organización es la
/// conversación y cuánto hay sin leer en ella.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DmUnread {
    org_id: String,
    unread: i64,
}

/// Lo que se cuenta de `domain.ServerResponse`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerStatus {
    org_id: String,
    status: String,
}

/// Lo que hay detrás de los números: las listas tal como llegaron, de todas
/// las organizaciones, para volver a contar cuando cambia algo sin volver a
/// preguntar.
#[derive(Default)]
pub(crate) struct CountersData {
    state: CountersState,
    chat: Option<Vec<ChatUnread>>,
    dms: Option<Vec<DmUnread>>,
    servers: Option<Vec<ServerStatus>>,
    /// Los espacios de una organización, con la organización: un canal es un
    /// espacio, y sólo cuentan los de la que se mira.
    spaces: Option<(String, Vec<String>)>,
}

impl CountersData {
    /// Vuelve a contar lo que sale de las listas.
    fn recount(&mut self) {
        let org_id = self.state.org_id.as_deref();
        let spaces = self.spaces.as_ref().filter(|(of, _)| Some(of.as_str()) == org_id).map(|(_, ids)| ids);
        let channels = self.chat.as_ref().zip(spaces).map(|(rows, spaces)| {
            rows.iter().filter(|row| spaces.contains(&row.space_id)).map(|row| row.count).sum::<i64>()
        });
        let dms = self.dms.as_ref().zip(org_id).map(|(rows, org_id)| {
            rows.iter().filter(|row| row.org_id == org_id).map(|row| row.unread).sum::<i64>()
        });
        // Con una de las dos mitades basta para decir algo: es lo que se sabe.
        // Sin ninguna, nada.
        self.state.comms = match (channels, dms) {
            (None, None) => None,
            (channels, dms) => Some(channels.unwrap_or(0) + dms.unwrap_or(0)),
        };
        // En ámbar y contados, no listados: es un «míralo cuando puedas». Lo
        // que no es `online` —`offline`, `pending`— pide que alguien lo mire.
        self.state.servers = self.servers.as_ref().zip(org_id).map(|(rows, org_id)| {
            rows.iter().filter(|row| row.org_id == org_id && row.status != "online").count() as i64
        });
    }
}

/// Cada cosa que se pregunta por separado. La comunicación son dos.
#[derive(Clone, Copy)]
enum Source {
    Notifications,
    Channels,
    Dms,
    Servers,
    Invitations,
}

pub(crate) const SOURCES: usize = 5;

/// Los contadores de este core.
pub struct Counters {
    core: Core,
}

impl Core {
    pub fn counters(&self) -> Counters {
        Counters { core: self.clone() }
    }
}

impl Counters {
    pub fn state(&self) -> CountersState {
        self.core.inner.counters.lock().expect("counters lock").state.clone()
    }

    /// Cambia lo que hay detrás, vuelve a contar y avisa si algún número se
    /// movió. Con el cerrojo ya suelto.
    fn update(&self, change: impl FnOnce(&mut CountersData)) {
        let changed = {
            let mut data = self.core.inner.counters.lock().expect("counters lock");
            let before = data.state.clone();
            change(&mut data);
            data.recount();
            (data.state != before).then(|| data.state.clone())
        };
        if let Some(state) = changed {
            self.core.inner.listener.counters_changed(state);
        }
    }

    /// Cuenta para esa organización. Sin ninguna —alguien a quien acaban de
    /// invitar y todavía no es de nada— sólo se preguntan las invitaciones,
    /// que es justo lo que esa persona tiene que ver.
    ///
    /// Al cambiar de organización los números de la anterior se quitan antes
    /// de preguntar: si la carga nueva falla, lo que queda es nada, y no la
    /// cuenta de otra organización.
    pub async fn load(&self, org_id: Option<String>) {
        let org_id = org_id.filter(|id| !id.is_empty());
        self.update(|data| {
            if data.state.org_id != org_id {
                data.state.org_id = org_id;
                data.state.notifications = None;
                data.chat = None;
                data.dms = None;
                data.servers = None;
            }
        });
        self.refresh().await;
    }

    /// Vuelve a preguntarlo todo para la organización que se mira: al entrar,
    /// y cuando el stream vuelve de una caída, que es cuando pudo pasar algo
    /// sin que llegara su evento.
    pub async fn refresh(&self) {
        // A la vez, cada una en su tarea: ninguna espera a otra, y una que
        // tarda o falla no retrasa los demás números.
        let loads = [Source::Notifications, Source::Channels, Source::Dms, Source::Servers, Source::Invitations].map(|source| {
            let counters = self.core.counters();
            tokio::spawn(async move { counters.load_one(source).await })
        });
        for load in loads {
            let _ = load.await;
        }
    }

    /// Pregunta una cosa. Si falla, se queda lo que hubiera: lo último que se
    /// supo, o nada si no se llegó a saber.
    async fn load_one(&self, source: Source) {
        let org_id = self.state().org_id;
        if matches!(source, Source::Invitations) {
            let Ok(pending) = self.core.get_list::<serde::de::IgnoredAny>("/api/v1/invitations/").await else { return };
            self.update(|data| data.state.invitations = Some(pending.len() as i64));
            return;
        }
        // Lo demás es de una organización: sin ella no hay nada que contar,
        // y a la campana no se le pregunta, que contestaría con lo de todas.
        let Some(org_id) = org_id else { return };
        match source {
            Source::Notifications => {
                // `limit=1`: sólo hace falta la cuenta, y con cero el servidor
                // manda una página entera de cincuenta filas.
                let path = format!("/api/v1/notifications/?orgId={org_id}&limit=1");
                let Ok(feed) = self.core.get::<Feed>(&path).await else { return };
                // La respuesta de una organización que ya no es la que se
                // mira no se pone.
                self.update(|data| {
                    if data.state.org_id.as_deref() == Some(org_id.as_str()) {
                        data.state.notifications = Some(feed.unread);
                    }
                });
            }
            Source::Channels => {
                let Ok(rows) = self.core.get_list::<ChatUnread>("/api/v1/chat/unread").await else { return };
                self.update(|data| data.chat = Some(rows));
            }
            Source::Dms => {
                let Ok(rows) = self.core.get_list::<DmUnread>("/api/v1/dm/").await else { return };
                self.update(|data| data.dms = Some(rows));
            }
            Source::Servers => {
                let Ok(rows) = self.core.get_list::<ServerStatus>("/api/v1/servers/").await else { return };
                self.update(|data| data.servers = Some(rows));
            }
            Source::Invitations => {}
        }
    }

    /// Vuelve a preguntar una cosa tras un respiro, y sólo la última vez que
    /// se pidió. Como en «Mi trabajo»: no se corta a la anterior, se retira
    /// sola si ya no es la última.
    fn reload_soon(&self, source: Source) {
        let turns = &self.core.inner.counter_reloads[source as usize];
        let turn = turns.fetch_add(1, Ordering::SeqCst) + 1;
        let counters = self.core.counters();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            if counters.core.inner.counter_reloads[source as usize].load(Ordering::SeqCst) != turn {
                return;
            }
            counters.load_one(source).await;
        });
    }

    /// Un evento del servidor: se vuelve a contar lo que ese evento mueve.
    ///
    /// Se pregunta otra vez en vez de sumar uno: quien sabe cuánto hay sin
    /// leer es el servidor (lo propio no cuenta, lo de un canal silenciado
    /// tampoco), y una suma local se va separando de la verdad sin que nada
    /// lo delate. Los servidores no tienen evento: su estado se decide al
    /// leerlo, y se vuelve a leer al entrar, al cambiar de organización y al
    /// recuperar el stream.
    pub(crate) fn server_event(&self, event: &ServerEvent) {
        if rings_bell(&event.event, &event.data) {
            self.reload_soon(Source::Notifications);
        }
        match event.event.as_str() {
            "chat:message" => self.reload_soon(Source::Channels),
            "dm:message" => self.reload_soon(Source::Dms),
            // Entrar a una organización es haber aceptado su invitación,
            // quizá en otro dispositivo.
            "org:membership" => self.reload_soon(Source::Invitations),
            _ => {}
        }
    }

    /// El árbol de una organización llegó: sus espacios son sus canales,
    /// también la sala general, que el árbol no pinta.
    pub(crate) fn spaces_loaded(&self, org_id: &str, space_ids: Vec<String>) {
        self.update(|data| data.spaces = Some((org_id.to_string(), space_ids)));
    }

    /// Al salir: los números de una persona no son los de la siguiente.
    pub(crate) fn clear(&self) {
        self.update(|data| *data = CountersData::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que atrapa: pintar un «0»; pintar algo cuando la cuenta no se
    /// pudo cargar; escribir «10» en la campana o «100» en el riel, que no
    /// caben; poner el «+» ya en el tope («9+» con nueve).
    #[test]
    fn a_counter_says_nothing_at_zero_and_stops_at_its_cap() {
        assert_eq!(count_badge(None, BELL_CAP), None);
        assert_eq!(count_badge(Some(0), BELL_CAP), None);
        assert_eq!(count_badge(Some(-1), BELL_CAP), None);
        assert_eq!(count_badge(Some(1), BELL_CAP).as_deref(), Some("1"));
        assert_eq!(count_badge(Some(9), BELL_CAP).as_deref(), Some("9"));
        assert_eq!(count_badge(Some(10), BELL_CAP).as_deref(), Some("9+"));
        assert_eq!(count_badge(Some(99), RAIL_CAP).as_deref(), Some("99"));
        assert_eq!(count_badge(Some(100), RAIL_CAP).as_deref(), Some("99+"));
    }

    /// Rotura que atrapa: «Comunicación (0)»; un nombre sin su cuenta; la
    /// cuenta recortada al tope del dibujo («(99+)» habiendo ciento veinte).
    #[test]
    fn a_label_carries_what_is_pending() {
        assert_eq!(counted_label("Comunicación", Some(3)), "Comunicación (3)");
        assert_eq!(counted_label("Comunicación", Some(120)), "Comunicación (120)");
        assert_eq!(counted_label("Comunicación", Some(0)), "Comunicación");
        assert_eq!(counted_label("Comunicación", None), "Comunicación");
    }

    /// Rotura que atrapa: un mensaje de canal o un directo que no refrescan
    /// la campana (se veían al reiniciar); releerla en cada paso de un run o
    /// de un despliegue; releerla por un backup que salió bien; darla por
    /// tocada con un evento que no deja fila.
    #[test]
    fn the_bell_rings_for_what_leaves_a_row() {
        for event in ["chat:message", "dm:message", "task:assigned", "report:new", "post:comment"] {
            assert!(rings_bell(event, "{}"), "{event} deja fila");
        }
        for event in ["task:move", "chat:reaction", "dm:read", "ping", "org:membership"] {
            assert!(!rings_bell(event, "{}"), "{event} no deja fila");
        }
        assert!(rings_bell("ci:run", r#"{"status":"completed"}"#));
        assert!(!rings_bell("ci:run", r#"{"status":"in_progress"}"#));
        assert!(rings_bell("deploy:status", r#"{"status":"succeeded"}"#));
        assert!(rings_bell("deploy:status", r#"{"status":"failed"}"#));
        assert!(!rings_bell("deploy:status", r#"{"status":"running"}"#));
        assert!(rings_bell("backup:status", r#"{"status":"failed"}"#));
        assert!(!rings_bell("backup:status", r#"{"status":"succeeded"}"#));
        assert!(!rings_bell("ci:run", "no es json"));
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// El contrato con el servidor, para lo que estos contadores leen: los
    /// ficheros los escribe Go con sus structs de verdad
    /// (`core_contract_test.go`). Está aquí y no en `tests/contract.rs`
    /// porque estos tipos no salen del core: se lee lo que se cuenta, y lo
    /// demás de cada forma es de pantallas que todavía no existen.
    ///
    /// Rotura que atrapa: que en Go se renombre `unread`, `spaceId`, `count`,
    /// `orgId` o `status` y el contador se quede sin número sin que nada
    /// falle.
    #[test]
    fn what_is_counted_is_what_the_server_writes() {
        let feed: Feed = serde_json::from_str(&fixture("notification_feed.json")).unwrap();
        assert_eq!(feed.unread, 3);

        let channel: ChatUnread = serde_json::from_str(&fixture("chat_unread.json")).unwrap();
        assert_eq!((channel.space_id.as_str(), channel.count), ("e8bd6d46-a9ab-494e-ac29-de84d80a55f4", 4));

        let dm: DmUnread = serde_json::from_str(&fixture("dm_summary.json")).unwrap();
        assert_eq!((dm.org_id.as_str(), dm.unread), ("35d32154-922f-47dc-a6d9-051d4e78ae8a", 2));

        let server: ServerStatus = serde_json::from_str(&fixture("server_response.json")).unwrap();
        assert_eq!((server.org_id.as_str(), server.status.as_str()), ("35d32154-922f-47dc-a6d9-051d4e78ae8a", "offline"));
    }
}
