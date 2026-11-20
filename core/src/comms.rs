//! La comunicación de una organización: sus canales y sus conversaciones
//! privadas, en una lista, y cuál se tiene delante.
//!
//! Vino de `app/src/components/sidebar/panels/CommsPanel.tsx`,
//! `components/comms/ChannelsSection.tsx`, `DirectsSection.tsx` y
//! `NewConversation.tsx`, de `lib/comms.ts`, y de lo que `chat.store.ts`,
//! `dm.store.ts`, `people.store.ts` y `places.store.ts` guardaban cada uno
//! por su lado.
//!
//! **Todo lo que llega del servidor llega de todas las organizaciones** de la
//! persona (de la plataforma entera para un superadmin): lo sin leer de cada
//! canal, su última línea y las conversaciones privadas. Aquí se guarda tal
//! como llega y se acota al pintar, contra la organización que se mira **en
//! ese momento**: así una respuesta que llega tarde, cuando ya se cambió de
//! organización, no puede pintar nada de la anterior.
//!
//! - Un canal es un espacio: los de la lista son los del árbol de la
//!   organización, con la sala general delante. Un espacio que se borra deja
//!   de tener canal en cuanto llega el árbol sin él.
//! - Lo sin leer de los canales y las conversaciones privadas las pide y las
//!   guarda `counters`, que las cuenta para el riel; aquí se leen de allí en
//!   vez de pedirlas otra vez, y así el número del riel y la suma de las
//!   filas no pueden decir cosas distintas.
//! - La última línea de cada canal se pide aquí (`/chat/latest`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::comms_text::{list_time, preview_line, seen_recently, was_read, LastLine};
use crate::comms_wire::{ChatLatest, ChatMessageEvent, DmReadEvent, DmSummary, Person};
use crate::counters::{count_badge, RAIL_CAP};
use crate::error::ApiError;
use crate::mywork::filter::fold;
use crate::stream::ServerEvent;
use crate::texts::Locale;
use crate::tree::SpaceTree;
use crate::Core;

/// Bajo qué clave se guarda la última conversación de cada organización.
const PLACES_KEY: &str = "comms-places";
/// Bajo qué clave se guarda qué secciones están plegadas en cada organización.
const FOLDED_KEY: &str = "comms-folded";

/// Un respiro antes de volver a pedir las últimas líneas: diez mensajes
/// seguidos son diez eventos, y la lista a la que se llega es la misma.
const COALESCE: Duration = Duration::from_millis(400);

/// Cuánta gente se pide para «Nueva conversación»: la de `people.store.ts`.
const PEOPLE_LIMIT: usize = 50;

/// Una conversación: el canal de un espacio o un directo con alguien.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Conversation {
    #[serde(rename_all = "camelCase")]
    Channel { space_id: String },
    #[serde(rename_all = "camelCase")]
    Direct { conversation_id: String },
}

impl Conversation {
    /// El id de lo que es: el del espacio o el de la conversación.
    pub fn id(&self) -> &str {
        match self {
            Conversation::Channel { space_id } => space_id,
            Conversation::Direct { conversation_id } => conversation_id,
        }
    }
}

/// Las dos secciones de la lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum CommsSection {
    Channels,
    Directs,
}

/// Un canal de la organización que se mira.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub space_id: String,
    pub name: String,
    /// La sala general: de todos, sin tareas, y la primera.
    pub general: bool,
    pub unread: i64,
    /// Su última línea, si tiene alguna y se sabe.
    pub last: Option<ChatLatest>,
}

/// La comunicación de la organización que se mira, ya acotada a ella.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct CommsState {
    /// De qué organización es todo lo de abajo.
    pub org_id: Option<String>,
    /// Se está en comunicación: el panel es su lista y la vista, la
    /// conversación.
    pub on_screen: bool,
    /// La sala general delante y luego los espacios, en el orden del árbol.
    pub channels: Vec<Channel>,
    /// Las conversaciones privadas, en el orden del servidor: la del último
    /// mensaje, la primera.
    pub directs: Vec<DmSummary>,
    /// La que se tiene delante: la última que se eligió en esta
    /// organización si sigue existiendo, si no la sala general, y si no el
    /// primer canal.
    pub selected: Option<Conversation>,
    /// Si se sigue el canal que se tiene delante: `None` si lo de delante
    /// es un directo, o mientras no se sabe qué se sigue.
    pub selected_followed: Option<bool>,
    pub channels_folded: bool,
    pub directs_folded: bool,
}

/// La que toca tener delante (`resolveSpace` en `lib/comms.ts`): la
/// recordada si sigue ahí, y si no el primer canal, que es la sala general
/// cuando existe.
///
/// Una sola función porque la contestan dos sitios: la vista, que pinta esa
/// conversación, y la lista, que marca su fila. Algo recordado que ya no
/// está —un canal borrado, una conversación de otra organización— cae a lo
/// siguiente en vez de dejar la pantalla vacía.
pub fn resolve_conversation(channels: &[Channel], directs: &[DmSummary], remembered: Option<&Conversation>) -> Option<Conversation> {
    let still_there = |conversation: &&Conversation| match conversation {
        Conversation::Channel { space_id } => channels.iter().any(|channel| &channel.space_id == space_id),
        Conversation::Direct { conversation_id } => directs.iter().any(|direct| &direct.conversation_id == conversation_id),
    };
    remembered
        .filter(still_there)
        .cloned()
        // El primero es la sala general si la hay: así llegan ordenados.
        .or_else(|| channels.first().map(|channel| Conversation::Channel { space_id: channel.space_id.clone() }))
}

/// Qué secciones plegó alguien en una organización.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
struct Folded {
    #[serde(default)]
    channels: bool,
    #[serde(default)]
    directs: bool,
}

/// El espacio de un canal, con lo que la lista necesita de él.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChannelSpace {
    id: String,
    name: String,
    general: bool,
}

/// Lo que hay detrás de la lista.
#[derive(Default)]
pub(crate) struct CommsData {
    on_screen: bool,
    /// Los espacios de cada organización cuyo árbol ha llegado. Por
    /// organización, y no «los últimos que llegaron»: el árbol de la que se
    /// dejó, si llega tarde, se apunta en lo suyo y no le quita a la nueva
    /// sus canales.
    spaces: HashMap<String, Vec<ChannelSpace>>,
    /// La última línea de cada canal, de todas las organizaciones.
    latest: Option<Vec<ChatLatest>>,
    /// La última conversación que se eligió en cada organización.
    places: HashMap<String, Conversation>,
    folded: HashMap<String, Folded>,
    /// La gente de cada organización para la que se ha pedido. Por
    /// organización, por lo mismo que los espacios: la respuesta tardía de
    /// la que se dejó no le quita a la nueva la suya.
    people: HashMap<String, Vec<Person>>,
    /// Los canales que se siguen, de todas las organizaciones de uno
    /// (`/chat/following`): los suyos menos los silenciados. `None` mientras
    /// no se sabe; lo usan los avisos.
    following: Option<HashSet<String>>,
    /// Lo último que se avisó, para no avisar de lo mismo.
    published: CommsState,
}

/// La comunicación de este core.
pub struct Comms {
    core: Core,
}

impl Core {
    pub fn comms(&self) -> Comms {
        Comms { core: self.clone() }
    }
}

impl Comms {
    fn data(&self) -> std::sync::MutexGuard<'_, CommsData> {
        self.core.inner.comms.lock().expect("comms lock")
    }

    /// La lista de la organización que se mira **ahora**.
    ///
    /// Se arma cada vez con la organización del momento, y no se guarda con
    /// la de cuando llegó cada cosa: es lo que impide que una respuesta
    /// tardía pinte lo de otra.
    pub fn state(&self) -> CommsState {
        let org_id = self.core.orgs().state().current_org_id.filter(|id| !id.is_empty());
        let (unread, dms) = self.core.counters().conversations();
        let data = self.data();
        let Some(org) = org_id.as_deref() else {
            return CommsState { on_screen: data.on_screen, ..CommsState::default() };
        };
        let spaces = data.spaces.get(org).map(Vec::as_slice).unwrap_or_default();
        let channel = |space: &ChannelSpace| Channel {
            space_id: space.id.clone(),
            name: space.name.clone(),
            general: space.general,
            unread: unread.iter().flatten().filter(|row| row.space_id == space.id).map(|row| row.count).sum(),
            last: data.latest.iter().flatten().find(|line| line.space_id == space.id).cloned(),
        };
        // La sala general, la primera: es de todos, y es donde cae quien no
        // eligió nada (`orderedChannels`).
        let channels: Vec<Channel> =
            spaces.iter().filter(|space| space.general).chain(spaces.iter().filter(|space| !space.general)).map(channel).collect();
        let directs: Vec<DmSummary> = dms.into_iter().flatten().filter(|direct| direct.org_id == org).collect();
        let selected = resolve_conversation(&channels, &directs, data.places.get(org));
        let folded = data.folded.get(org).copied().unwrap_or_default();
        let selected_followed = match &selected {
            Some(Conversation::Channel { space_id }) => data.following.as_ref().map(|following| following.contains(space_id)),
            _ => None,
        };
        CommsState {
            org_id,
            on_screen: data.on_screen,
            channels,
            directs,
            selected,
            selected_followed,
            channels_folded: folded.channels,
            directs_folded: folded.directs,
        }
    }

    /// Vuelve a armar la lista y avisa si cambió. Lo llama todo lo que pueda
    /// moverla: lo de aquí, y también `counters` (lo sin leer, los directos),
    /// el árbol (los canales) y el cambio de organización.
    pub(crate) fn publish(&self) {
        let state = self.state();
        {
            let mut data = self.data();
            if data.published == state {
                return;
            }
            data.published = state.clone();
        }
        self.core.inner.listener.comms_changed(state);
        // La conversación que se pinta depende de cuál marca la lista y de
        // hasta dónde leyó el otro: se mueve con ella.
        self.core.chat().publish();
    }

    /// Lo que se eligió y lo que se plegó la última vez, para arrancar con ello.
    pub(crate) fn restore(&self) {
        let storage = &self.core.inner.storage;
        let places = storage.get(PLACES_KEY).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default();
        let folded = storage.get(FOLDED_KEY).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default();
        let mut data = self.data();
        data.places = places;
        data.folded = folded;
    }

    /// Al salir: las líneas y la gente de una persona no son de la
    /// siguiente. Lo recordado por organización se queda: es de la máquina,
    /// y sólo vale para una organización a la que se pertenezca.
    pub(crate) fn clear(&self) {
        {
            let mut data = self.data();
            data.on_screen = false;
            data.spaces.clear();
            data.latest = None;
            data.following = None;
            data.people.clear();
        }
        self.publish();
    }

    /// Entrar a comunicación: el panel pasa a ser su lista. Sólo un módulo
    /// está en pantalla a la vez: deja Plataforma si se estaba en ella.
    pub fn enter(&self) {
        self.core.platform().leave();
        self.data().on_screen = true;
        self.publish();
    }

    /// Volver a lo demás. Lo elegido se queda para la vuelta.
    pub fn leave(&self) {
        self.data().on_screen = false;
        self.publish();
    }

    /// Trae la última línea de cada canal y qué canales se siguen: al entrar
    /// y al volver de una caída. Lo que falle se queda como estuviera.
    pub async fn load(&self) {
        let following = {
            let core = self.core.clone();
            tokio::spawn(async move { core.get_list::<String>("/api/v1/chat/following").await })
        };
        self.load_latest().await;
        if let Ok(Ok(following)) = following.await {
            self.data().following = Some(following.into_iter().collect());
            self.publish();
        }
    }

    /// Trae la última línea de cada canal. Si falla se queda lo que hubiera:
    /// la lista sale sin adelantos, que es como era antes de tenerlos. Con
    /// ellas, los avisos saben qué mensaje era nuevo (`Notices`).
    async fn load_latest(&self) {
        let asked_at = Instant::now();
        let Ok(latest) = self.core.get_list::<ChatLatest>("/api/v1/chat/latest").await else { return };
        self.data().latest = Some(latest.clone());
        self.publish();
        self.core.notices().channels_loaded(&latest, asked_at);
    }

    /// Sigue o silencia un canal (`setFollowing` en `chat.store.ts`).
    ///
    /// Silenciar es pedir que lo corriente deje de avisar: las menciones y
    /// las respuestas llegan igual (`notice.rs`), y el servidor deja de
    /// contarlo como sin leer, así que se vuelve a contar. El botón cambia en
    /// el acto; si el servidor dice que no, vuelve a como estaba y se dice
    /// por qué. Después se vuelve a pedir la lista, que es la que manda.
    pub async fn set_following(&self, space_id: &str, on: bool) -> Result<(), ApiError> {
        let before = {
            let mut data = self.data();
            let before = data.following.clone();
            let following = data.following.get_or_insert_with(HashSet::new);
            if on {
                following.insert(space_id.to_string());
            } else {
                following.remove(space_id);
            }
            before
        };
        self.publish();
        let path = format!("/api/v1/task-spaces/{space_id}/chat/follow");
        let answer = if on { self.core.call_ok("POST", &path, Some(serde_json::json!({}))).await } else { self.core.call_ok("DELETE", &path, None).await };
        if answer.is_err() {
            self.data().following = before;
            self.publish();
        }
        if let Ok(following) = self.core.get_list::<String>("/api/v1/chat/following").await {
            self.data().following = Some(following.into_iter().collect());
            self.publish();
        }
        self.core.counters().reload_channels().await;
        answer
    }

    /// Los canales que se siguen, si se sabe.
    pub(crate) fn following(&self) -> Option<HashSet<String>> {
        self.data().following.clone()
    }

    /// De qué organización es un canal, aunque no sea la que se mira: si su
    /// árbol no ha llegado, se pregunta por los espacios de las demás
    /// organizaciones de uno hasta dar con él. Lo usa abrir un aviso, que
    /// puede ser de cualquiera. `None` si no es de ninguna (un superadmin oye
    /// los canales de toda la plataforma).
    pub(crate) async fn channel_org(&self, space_id: &str) -> Option<String> {
        if let (Some(org_id), _) = self.channel_place(space_id) {
            return Some(org_id);
        }
        let known: Vec<String> = self.data().spaces.keys().cloned().collect();
        let orgs = self.core.orgs().state().orgs;
        for org in orgs.iter().filter(|org| !known.contains(&org.id)) {
            let path = format!("/api/v1/task-spaces/?orgId={}", org.id);
            let Ok(spaces) = self.core.get::<Vec<SpaceTree>>(&path).await else { continue };
            let found = spaces.iter().any(|space| space.id == space_id);
            self.spaces_loaded(&org.id, &spaces);
            if found {
                return Some(org.id.clone());
            }
        }
        None
    }

    /// De qué organización es un canal y cómo se llama, si ha llegado su
    /// árbol.
    pub(crate) fn channel_place(&self, space_id: &str) -> (Option<String>, Option<String>) {
        let data = self.data();
        data.spaces
            .iter()
            .find_map(|(org_id, spaces)| spaces.iter().find(|space| space.id == space_id).map(|space| (Some(org_id.clone()), Some(space.name.clone()))))
            .unwrap_or_default()
    }

    /// Vuelve a pedir las últimas líneas tras un respiro, y sólo la última
    /// vez que se pidió (como `Counters::reload_soon`).
    fn reload_soon(&self) {
        let turn = self.core.inner.comms_reloads.fetch_add(1, Ordering::SeqCst) + 1;
        let comms = self.core.comms();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            if comms.core.inner.comms_reloads.load(Ordering::SeqCst) != turn {
                return;
            }
            comms.load_latest().await;
        });
    }

    /// El árbol de una organización llegó: sus espacios son sus canales,
    /// también la sala general, que el árbol no pinta.
    pub(crate) fn spaces_loaded(&self, org_id: &str, spaces: &[SpaceTree]) {
        let channels = spaces.iter().map(|space| ChannelSpace { id: space.id.clone(), name: space.name.clone(), general: space.is_general() }).collect();
        self.data().spaces.insert(org_id.to_string(), channels);
        self.publish();
    }

    /// Un evento del servidor: se mueve la fila que ese evento toca.
    ///
    /// - `chat:message` trae el adelanto: la fila de ese canal lo enseña ya,
    ///   sin pedir nada, y luego se vuelven a pedir las últimas líneas, que
    ///   es quien sabe la hora de verdad. Una línea que ya estaba y cambió
    ///   (`edited`) no se toma por la última —puede no serlo—: sólo se pide.
    /// - `dm:read` es el otro leyendo: lo propio pasa a leído, con lo que
    ///   trae el evento.
    /// - `dm:message` no trae texto (lo privado no viaja en un aviso): la
    ///   lista de directos la vuelve a pedir `counters`, y al llegar se
    ///   repinta aquí.
    ///
    /// Cuánto hay sin leer no se suma aquí nunca: lo sabe el servidor.
    pub(crate) fn server_event(&self, event: &ServerEvent) {
        match event.event.as_str() {
            "chat:message" => {
                let Ok(said) = serde_json::from_str::<ChatMessageEvent>(&event.data) else { return };
                if said.space_id.is_empty() {
                    return;
                }
                if !said.edited {
                    let line = ChatLatest {
                        space_id: said.space_id.clone(),
                        author_user_id: said.actor_id,
                        author_name: said.author_name,
                        kind: said.kind,
                        body: said.preview,
                        created_at: Utc::now().to_rfc3339(),
                    };
                    {
                        let mut data = self.data();
                        // Sin la lista todavía no hay dónde ponerla: llegará
                        // entera con la carga.
                        if let Some(latest) = data.latest.as_mut() {
                            latest.retain(|other| other.space_id != line.space_id);
                            latest.push(line);
                        }
                    }
                    self.publish();
                }
                self.reload_soon();
            }
            "dm:read" => {
                let Ok(read) = serde_json::from_str::<DmReadEvent>(&event.data) else { return };
                self.core.counters().direct_read(&read.conversation_id, &read.at);
            }
            _ => {}
        }
    }

    /// Elige una conversación de la organización que se mira, y la recuerda
    /// para la vuelta. Una que no es de esta organización no se elige.
    pub fn select(&self, conversation: Conversation) -> bool {
        let state = self.state();
        let Some(org_id) = state.org_id else { return false };
        let here = match &conversation {
            Conversation::Channel { space_id } => state.channels.iter().any(|channel| &channel.space_id == space_id),
            Conversation::Direct { conversation_id } => state.directs.iter().any(|direct| &direct.conversation_id == conversation_id),
        };
        if !here {
            return false;
        }
        self.remember(&org_id, conversation);
        true
    }

    /// Apunta la conversación **bajo su organización**, no bajo la que esté
    /// en pantalla (`places.store.ts`).
    fn remember(&self, org_id: &str, conversation: Conversation) {
        let raw = {
            let mut data = self.data();
            data.places.insert(org_id.to_string(), conversation);
            serde_json::to_string(&data.places).ok()
        };
        if let Some(raw) = raw {
            let _ = self.core.inner.storage.set(PLACES_KEY, &raw);
        }
        self.publish();
    }

    /// Pliega o despliega una sección en la organización que se mira, y lo
    /// recuerda.
    pub fn toggle_section(&self, section: CommsSection) {
        let Some(org_id) = self.core.orgs().state().current_org_id else { return };
        let raw = {
            let mut data = self.data();
            let folded = data.folded.entry(org_id).or_default();
            match section {
                CommsSection::Channels => folded.channels = !folded.channels,
                CommsSection::Directs => folded.directs = !folded.directs,
            }
            serde_json::to_string(&data.folded).ok()
        };
        if let Some(raw) = raw {
            let _ = self.core.inner.storage.set(FOLDED_KEY, &raw);
        }
        self.publish();
    }

    /// La lista tal como se pinta, para quien está dentro.
    pub fn panel<Viewer: TimeZone>(&self, query: &str, locale: Locale, now: &DateTime<Viewer>) -> CommsPanel {
        let session = self.core.session();
        let admin = session.as_ref().is_some_and(|session| session.superadmin)
            || self.core.orgs().state().current().is_some_and(|org| org.role == "admin");
        let viewer = CommsViewer { user_id: session.as_ref().map(|session| session.id.as_str()), admin };
        comms_panel(&self.state(), viewer, query, locale, now)
    }

    // ── Nueva conversación ──────────────────────────────────────────────

    /// Trae la gente de la organización que se mira, para ofrecerla.
    ///
    /// Una vez por organización y no por tecla: un equipo cabe en memoria, y
    /// la búsqueda de mientras se escribe es sobre lo traído. El servidor
    /// rechaza una organización a la que no se pertenece.
    pub async fn load_people(&self) {
        let Some(org_id) = self.core.orgs().state().current_org_id else { return };
        let path = format!("/api/v1/users/search?q=&orgId={org_id}&limit={PEOPLE_LIMIT}");
        let Ok(people) = self.core.get_list::<Person>(&path).await else { return };
        self.data().people.insert(org_id, people);
    }

    /// Si ya se trajo la gente de la organización que se mira.
    pub(crate) fn has_people(&self) -> bool {
        let Some(org_id) = self.core.orgs().state().current_org_id else { return true };
        self.data().people.contains_key(&org_id)
    }

    /// Quién de la organización que se mira anda por aquí, sin contar a
    /// quien mira (`QuienAnda` en `ChannelView.tsx`): sus nombres, de la
    /// gente ya traída.
    pub fn around(&self, now: DateTime<Utc>) -> Vec<String> {
        self.people("").into_iter().filter(|person| seen_recently(person.last_seen_at.as_deref(), now)).map(|person| person.shown_name().to_string()).collect()
    }

    /// A quién se le puede escribir en la organización que se mira, de la
    /// gente ya traída. La de otra organización —una respuesta que llegó
    /// tarde, o la de antes de cambiar— no se ofrece.
    pub fn people(&self, query: &str) -> Vec<Person> {
        let Some(org_id) = self.core.orgs().state().current_org_id else { return Vec::new() };
        let me = self.core.session().map(|session| session.id);
        let data = self.data();
        let people = data.people.get(&org_id).map(Vec::as_slice).unwrap_or_default();
        people_matching(people, me.as_deref(), query)
    }

    /// Abre la conversación con alguien de la organización que se mira, y la
    /// deja delante.
    ///
    /// Abrir es idempotente en el servidor: nombrar a alguien dos veces
    /// nunca hace un segundo hilo. La conversación se apunta bajo la
    /// organización **en la que se pidió**: si mientras contestaba se cambió
    /// a otra, en esa otra no se toca nada.
    pub async fn open_with(&self, user_id: &str) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        struct Opened {
            id: String,
        }
        let Some(org_id) = self.core.orgs().state().current_org_id else {
            return Err(ApiError::Server { status: 400, code: "no-organization".into(), message: "No organization is open".into() });
        };
        let body = serde_json::json!({ "orgId": org_id, "userId": user_id });
        let opened: Opened = self.core.call("POST", "/api/v1/dm/open", Some(body)).await?;
        // La lista antes de elegirla: una conversación recién nacida no
        // está en ella, y sin fila no hay qué marcar.
        self.core.counters().reload_directs().await;
        self.remember(&org_id, Conversation::Direct { conversation_id: opened.id.clone() });
        Ok(opened.id)
    }
}

/// A quién se le ofrece escribir (`NewConversation.tsx`): toda la gente menos
/// uno mismo —el servidor rechaza una conversación contigo—, también con
/// quien ya hay hilo. Lo escrito casa con el nombre o con el usuario, sin
/// acentos ni mayúsculas.
pub fn people_matching(people: &[Person], me: Option<&str>, query: &str) -> Vec<Person> {
    let term = fold(query.trim());
    people
        .iter()
        .filter(|person| Some(person.id.as_str()) != me)
        .filter(|person| term.is_empty() || fold(&person.username).contains(&term) || fold(person.shown_name()).contains(&term))
        .cloned()
        .collect()
}

// ── La lista, tal como se pinta ─────────────────────────────────────────

/// Quién mira la lista.
#[derive(Debug, Clone, Copy, Default)]
pub struct CommsViewer<'a> {
    pub user_id: Option<&'a str>,
    /// Administra la organización (o es superadmin): puede abrir su sala
    /// general.
    pub admin: bool,
}

/// Una fila de la lista: un canal o un directo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CommsRow {
    pub conversation: Conversation,
    pub name: String,
    /// La sala general: lleva megáfono en vez de almohadilla.
    pub general: bool,
    /// La línea de debajo del nombre. `None` sin mensajes.
    pub preview: Option<String>,
    /// La hora de esa línea, arriba a la derecha.
    pub time: Option<String>,
    /// Lo sin leer, ya escrito («3», «99+»). `None` sin nada.
    pub badge: Option<String>,
    /// Es la que se tiene delante.
    pub active: bool,
    /// En un directo, si lo último es de quien mira: sus palomitas, leído
    /// (`true`) o sólo enviado. `None` si no es suyo, o en un canal.
    pub ticks: Option<bool>,
    /// En un directo, si el otro anda por aquí. `None` en un canal.
    pub present: Option<bool>,
    /// La sala general con más canales debajo: va separada de ellos.
    pub set_apart: bool,
}

/// Una sección de la lista.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CommsRows {
    /// Se pinta. Filtrando, una sección sin nada que case no sale.
    pub shown: bool,
    pub open: bool,
    /// Lo sin leer de toda la sección, plegada o no.
    pub badge: Option<String>,
    /// Abierta, sus filas; plegada, las que se siguen viendo: la que se
    /// tiene delante y las que tienen algo sin leer.
    pub rows: Vec<CommsRow>,
}

/// La lista entera.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CommsPanel {
    pub channels: CommsRows,
    pub directs: CommsRows,
    /// La organización no tiene ningún espacio: se dice dónde nace un canal.
    pub no_channels: bool,
    /// Se ofrece abrir la sala general: no existe, y quien mira administra.
    pub offers_general: bool,
    /// Se ofrece «Nueva conversación»: siempre, salvo filtrando.
    pub offers_new_conversation: bool,
    /// Se filtra y nada casa, ni un canal ni una persona.
    pub nothing_matches: bool,
}

/// Arma la lista (`CommsPanel.tsx`, `ChannelsSection.tsx` y
/// `DirectsSection.tsx`).
///
/// El filtro casa canales por nombre y personas por usuario, sin acentos ni
/// mayúsculas, y abre una sección plegada: esconder lo que se busca detrás
/// de un chevrón sería no encontrarlo.
pub fn comms_panel<Viewer: TimeZone>(state: &CommsState, viewer: CommsViewer, query: &str, locale: Locale, now: &DateTime<Viewer>) -> CommsPanel {
    let term = fold(query.trim());
    let filtering = !term.is_empty();
    let matches = |name: &str| !filtering || fold(name).contains(&term);
    let badge = |count: i64| count_badge(Some(count), RAIL_CAP);
    let active = |conversation: &Conversation| state.on_screen && state.selected.as_ref() == Some(conversation);

    let others = state.channels.iter().filter(|channel| !channel.general && matches(&channel.name)).count();
    let channel_rows: Vec<CommsRow> = state
        .channels
        .iter()
        .filter(|channel| matches(&channel.name))
        .map(|channel| {
            let conversation = Conversation::Channel { space_id: channel.space_id.clone() };
            let last = channel.last.as_ref();
            CommsRow {
                active: active(&conversation),
                conversation,
                name: channel.name.clone(),
                general: channel.general,
                preview: last.map(|last| {
                    // Que una línea del sistema no es de nadie aunque la
                    // provocara quien mira lo decide `preview_line`, en un
                    // solo sitio.
                    let mine = viewer.user_id.is_some_and(|me| me == last.author_user_id);
                    preview_line(LastLine { body: &last.body, system: last.is_system(), author_name: &last.author_name, mine }, true, locale)
                }),
                time: last.map(|last| list_time(&last.created_at, now, locale)).filter(|time| !time.is_empty()),
                badge: badge(channel.unread),
                ticks: None,
                present: None,
                set_apart: channel.general && others > 0,
            }
        })
        .collect();

    let direct_rows: Vec<CommsRow> = state
        .directs
        .iter()
        .filter(|direct| matches(&direct.username))
        .map(|direct| {
            let conversation = Conversation::Direct { conversation_id: direct.conversation_id.clone() };
            let said = direct.last_body.as_deref().filter(|body| !body.is_empty());
            let mine = viewer.user_id.is_some_and(|me| direct.last_author_id.as_deref() == Some(me));
            CommsRow {
                active: active(&conversation),
                conversation,
                name: direct.username.clone(),
                general: false,
                // Sin «Tú:» ni nombre: en un directo ya se sabe quién es, y
                // lo propio lo dicen las palomitas.
                preview: said.map(|body| preview_line(LastLine { body, system: false, author_name: "", mine: false }, false, locale)),
                time: direct.last_message_at.as_deref().map(|at| list_time(at, now, locale)).filter(|time| !time.is_empty()),
                badge: badge(direct.unread),
                ticks: (said.is_some() && mine).then(|| was_read(direct.last_message_at.as_deref(), direct.other_read_at.as_deref())),
                present: Some(seen_recently(direct.last_seen_at.as_deref(), now.with_timezone(&Utc))),
                set_apart: false,
            }
        })
        .collect();

    let section = |rows: Vec<CommsRow>, folded: bool, unread: i64| {
        let open = filtering || !folded;
        // Plegada sigue a la vista lo que importa ahora: plegar es «no me
        // enseñes lo demás», no «escóndeme el canal que tengo delante».
        let rows = if open { rows } else { rows.into_iter().filter(|row| row.active || row.badge.is_some()).collect() };
        CommsRows { shown: !(filtering && rows.is_empty()), open, badge: badge(unread), rows }
    };
    let nothing_matches = filtering && channel_rows.is_empty() && direct_rows.is_empty();
    CommsPanel {
        no_channels: state.channels.is_empty() && !filtering,
        offers_general: viewer.admin && !filtering && !state.channels.iter().any(|channel| channel.general),
        offers_new_conversation: !filtering,
        nothing_matches,
        channels: section(channel_rows, state.channels_folded, state.channels.iter().map(|channel| channel.unread).sum()),
        directs: section(direct_rows, state.directs_folded, state.directs.iter().map(|direct| direct.unread).sum()),
    }
}

/// La cabecera de la conversación que se tiene delante: cómo se llama, y
/// qué es.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConversationTitle {
    /// «#producto» para un canal; el nombre de la persona en un directo.
    pub title: String,
    pub direct: bool,
    /// En un directo, si el otro anda por aquí.
    pub present: bool,
}

/// Cómo se titula la conversación elegida. `None` si no hay ninguna: una
/// organización sin espacios.
pub fn conversation_title(state: &CommsState, now: DateTime<Utc>) -> Option<ConversationTitle> {
    match state.selected.as_ref()? {
        Conversation::Channel { space_id } => {
            let channel = state.channels.iter().find(|channel| &channel.space_id == space_id)?;
            Some(ConversationTitle { title: format!("#{}", channel.name), direct: false, present: false })
        }
        Conversation::Direct { conversation_id } => {
            let direct = state.directs.iter().find(|direct| &direct.conversation_id == conversation_id)?;
            Some(ConversationTitle { title: direct.username.clone(), direct: true, present: seen_recently(direct.last_seen_at.as_deref(), now) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn channel(id: &str, name: &str, general: bool, unread: i64) -> Channel {
        Channel { space_id: id.into(), name: name.into(), general, unread, last: None }
    }

    fn direct(id: &str, username: &str, unread: i64) -> DmSummary {
        DmSummary { conversation_id: id.into(), org_id: "o1".into(), user_id: format!("u-{username}"), username: username.into(), unread, ..DmSummary::default() }
    }

    fn line(space: &str, author: &str, name: &str, kind: &str, body: &str, at: &str) -> ChatLatest {
        ChatLatest { space_id: space.into(), author_user_id: author.into(), author_name: name.into(), kind: kind.into(), body: body.into(), created_at: at.into() }
    }

    fn now() -> DateTime<FixedOffset> {
        FixedOffset::west_opt(6 * 3600).unwrap().with_ymd_and_hms(2026, 10, 15, 10, 0, 0).unwrap()
    }

    const ANA: CommsViewer = CommsViewer { user_id: Some("u1"), admin: false };

    fn state() -> CommsState {
        CommsState {
            org_id: Some("o1".into()),
            on_screen: true,
            channels: vec![channel("g", "General", true, 0), channel("a", "Producto", false, 3), channel("b", "Diseño", false, 0)],
            directs: vec![direct("c1", "luis", 2), direct("c2", "bea", 0)],
            selected: Some(Conversation::Channel { space_id: "b".into() }),
            ..CommsState::default()
        }
    }

    fn names(rows: &CommsRows) -> Vec<&str> {
        rows.rows.iter().map(|row| row.name.as_str()).collect()
    }

    /// Qué conversación toca tener delante.
    ///
    /// Roturas que atrapa: quedarse con un canal recordado que ya se borró
    /// (la vista en blanco, o la cabecera de algo que no existe); con un
    /// directo que no es de esta organización; no caer a la sala general; no
    /// caer a nada cuando no hay general.
    #[test]
    fn what_was_remembered_is_kept_only_while_it_is_still_there() {
        let state = state();
        let pick = |remembered: Option<Conversation>| resolve_conversation(&state.channels, &state.directs, remembered.as_ref());
        let channel = |id: &str| Conversation::Channel { space_id: id.into() };
        let direct = |id: &str| Conversation::Direct { conversation_id: id.into() };

        assert_eq!(pick(Some(channel("a"))), Some(channel("a")));
        assert_eq!(pick(Some(direct("c2"))), Some(direct("c2")));
        assert_eq!(pick(Some(channel("borrado"))), Some(channel("g")));
        assert_eq!(pick(Some(direct("de-otra-org"))), Some(channel("g")));
        // Un canal y un directo no se confunden aunque compartieran id.
        assert_eq!(pick(Some(direct("a"))), Some(channel("g")));
        assert_eq!(pick(None), Some(channel("g")));
        assert_eq!(resolve_conversation(&state.channels[1..], &state.directs, None), Some(channel("a")));
        assert_eq!(resolve_conversation(&[], &state.directs, None), None);
    }

    /// La fila de un canal: quién dijo lo último, cuándo y cuánto hay sin leer.
    ///
    /// Roturas que atrapa: «Tú:» delante de una línea del sistema que uno
    /// provocó; no decir «Tú:» en lo propio; no firmar lo de otro en un
    /// canal; el markdown crudo; un «0» de contador; «120» donde no cabe; la
    /// hora de la última línea en UTC; una hora inventada para un canal sin
    /// mensajes; la sala general sin separar de los demás.
    #[test]
    fn a_channel_row_says_who_spoke_last_and_what_is_unread() {
        let mut state = state();
        state.channels[0].last = Some(line("g", "u1", "Ana Ruiz", "system", "Grabación lista — [verla](cac:recording/r1)", "2026-10-15T15:30:00Z"));
        state.channels[1].last = Some(line("a", "u1", "Ana Ruiz", "user", "**voy** yo", "2026-10-15T05:59:00Z"));
        state.channels[1].unread = 120;
        state.channels[2].last = Some(line("b", "u2", "Luis Mora", "user", "¿sale [hoy](https://x.y)?", "2026-10-14T12:00:00Z"));
        let panel = comms_panel(&state, ANA, "", Locale::Es, &now());
        let rows = &panel.channels.rows;

        assert_eq!(names(&panel.channels), ["General", "Producto", "Diseño"]);
        assert_eq!(rows[0].preview.as_deref(), Some("Grabación lista — verla"));
        assert_eq!(rows[0].time.as_deref(), Some("09:30"));
        assert!(rows[0].general && rows[0].set_apart && rows[0].badge.is_none());
        assert_eq!(rows[1].preview.as_deref(), Some("Tú: voy yo"));
        // Las 05:59 UTC son las 23:59 de ayer para quien mira.
        assert_eq!(rows[1].time.as_deref(), Some("Ayer"));
        assert_eq!(rows[1].badge.as_deref(), Some("99+"));
        assert_eq!(rows[2].preview.as_deref(), Some("Luis Mora: ¿sale hoy?"));
        assert!(!rows[2].set_apart && rows[2].ticks.is_none() && rows[2].present.is_none());
        assert_eq!(panel.channels.badge.as_deref(), Some("99+"));

        state.channels[2].last = None;
        let panel = comms_panel(&state, ANA, "", Locale::Es, &now());
        assert_eq!((panel.channels.rows[2].preview.clone(), panel.channels.rows[2].time.clone()), (None, None));
        // A otra persona, lo de Ana va firmado por Ana.
        let other = comms_panel(&state, CommsViewer { user_id: Some("u9"), admin: false }, "", Locale::Es, &now());
        assert_eq!(other.channels.rows[1].preview.as_deref(), Some("Ana Ruiz: voy yo"));
    }

    /// La fila de un directo: sin firma, con palomitas en lo propio y si el
    /// otro anda por aquí.
    ///
    /// Roturas que atrapa: «Tú:» o el nombre en un directo; palomitas en lo
    /// que escribió el otro; leído cuando el otro leyó antes de que se
    /// escribiera; «activo» para quien se fue hace una hora; las entidades
    /// del editor crudas («&lt;3»); una fila sin mensajes con adelanto vacío.
    #[test]
    fn a_direct_row_has_ticks_on_what_is_mine_and_no_signature() {
        let mut state = state();
        state.directs[0] = DmSummary {
            last_body: Some("&lt;3 **vale**".into()),
            last_author_id: Some("u1".into()),
            last_message_at: Some("2026-10-15T15:00:00Z".into()),
            other_read_at: Some("2026-10-15T15:00:05Z".into()),
            last_seen_at: Some("2026-10-15T15:55:00Z".into()),
            ..direct("c1", "luis", 2)
        };
        state.directs[1] = DmSummary {
            last_body: Some("¿vienes?".into()),
            last_author_id: Some("u-bea".into()),
            last_message_at: Some("2026-10-15T15:00:00Z".into()),
            other_read_at: Some("2026-10-15T15:00:05Z".into()),
            last_seen_at: Some("2026-10-15T15:00:00Z".into()),
            ..direct("c2", "bea", 0)
        };
        state.directs.push(direct("c3", "caro", 0));
        let panel = comms_panel(&state, ANA, "", Locale::Es, &now());
        let rows = &panel.directs.rows;

        assert_eq!(names(&panel.directs), ["luis", "bea", "caro"]);
        assert_eq!((rows[0].preview.as_deref(), rows[0].ticks, rows[0].present), (Some("<3 vale"), Some(true), Some(true)));
        assert_eq!((rows[0].time.as_deref(), rows[0].badge.as_deref()), (Some("09:00"), Some("2")));
        assert_eq!((rows[1].preview.as_deref(), rows[1].ticks, rows[1].present), (Some("¿vienes?"), None, Some(false)));
        assert_eq!((rows[2].preview.clone(), rows[2].ticks, rows[2].time.clone()), (None, None, None));
        assert_eq!(panel.directs.badge.as_deref(), Some("2"));

        // El otro leyó antes de que se escribiera: enviado, no leído.
        state.directs[0].other_read_at = Some("2026-10-15T14:00:00Z".into());
        assert_eq!(comms_panel(&state, ANA, "", Locale::Es, &now()).directs.rows[0].ticks, Some(false));
    }

    /// La fila que se tiene delante se marca, y sólo estando en comunicación.
    ///
    /// Roturas que atrapa: ninguna fila marcada; marcada la fila de un canal
    /// cuando lo elegido es un directo (o al revés); una fila marcada en la
    /// lista mientras se mira «Mi trabajo».
    #[test]
    fn only_the_conversation_in_front_is_marked() {
        let mut state = state();
        let marked = |state: &CommsState| {
            let panel = comms_panel(state, ANA, "", Locale::Es, &now());
            panel.channels.rows.iter().chain(&panel.directs.rows).filter(|row| row.active).map(|row| row.name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(marked(&state), ["Diseño"]);
        state.selected = Some(Conversation::Direct { conversation_id: "c2".into() });
        assert_eq!(marked(&state), ["bea"]);
        state.on_screen = false;
        assert!(marked(&state).is_empty());
    }

    /// Plegar una sección deja a la vista lo que importa, y filtrar la abre.
    ///
    /// Roturas que atrapa: una sección plegada que esconde el canal que se
    /// tiene delante o los que tienen algo sin leer; una plegada que lo
    /// enseña todo; perder la cuenta de la cabecera al plegar; un filtro que
    /// no abre la sección plegada; una sección sin nada que case que sigue
    /// pintando su cabecera; no decir que nada casa; ofrecer «Nueva
    /// conversación» mientras se filtra; un filtro que distingue acentos.
    #[test]
    fn a_folded_section_keeps_what_matters_and_a_filter_opens_it() {
        let mut state = state();
        state.channels_folded = true;
        state.directs_folded = true;
        let panel = comms_panel(&state, ANA, "", Locale::Es, &now());
        assert!(!panel.channels.open && !panel.directs.open && panel.channels.shown && panel.directs.shown);
        assert_eq!(names(&panel.channels), ["Producto", "Diseño"]);
        assert_eq!(names(&panel.directs), ["luis"]);
        assert_eq!(panel.channels.badge.as_deref(), Some("3"));
        assert!(panel.offers_new_conversation && !panel.nothing_matches);

        let panel = comms_panel(&state, ANA, " diseno ", Locale::Es, &now());
        assert!(panel.channels.open && panel.channels.shown);
        assert_eq!(names(&panel.channels), ["Diseño"]);
        assert!(!panel.directs.shown && !panel.offers_new_conversation && !panel.nothing_matches);

        let panel = comms_panel(&state, ANA, "BEA", Locale::Es, &now());
        assert_eq!(names(&panel.directs), ["bea"]);
        assert!(!panel.channels.shown && panel.directs.open);

        let panel = comms_panel(&state, ANA, "zzz", Locale::Es, &now());
        assert!(panel.nothing_matches && !panel.channels.shown && !panel.directs.shown);

        state.channels_folded = false;
        assert_eq!(names(&comms_panel(&state, ANA, "", Locale::Es, &now()).channels), ["General", "Producto", "Diseño"]);
    }

    /// Lo que se ofrece cuando falta algo.
    ///
    /// Roturas que atrapa: ofrecer la sala general a quien no administra (el
    /// servidor se lo va a negar), o cuando ya existe (un botón que miente);
    /// no decir dónde nace un canal en una organización sin espacios; la sala
    /// general separada con una raya de nada cuando es el único canal.
    #[test]
    fn what_is_missing_is_offered_only_to_who_can_have_it() {
        let admin = CommsViewer { user_id: Some("u1"), admin: true };
        let mut state = state();
        assert!(!comms_panel(&state, admin, "", Locale::Es, &now()).offers_general);
        state.channels.remove(0);
        assert!(comms_panel(&state, admin, "", Locale::Es, &now()).offers_general);
        assert!(!comms_panel(&state, ANA, "", Locale::Es, &now()).offers_general);
        assert!(!comms_panel(&state, admin, "pro", Locale::Es, &now()).offers_general);
        assert!(!comms_panel(&state, admin, "", Locale::Es, &now()).no_channels);

        state.channels = vec![channel("g", "General", true, 0)];
        let panel = comms_panel(&state, ANA, "", Locale::Es, &now());
        assert!(!panel.channels.rows[0].set_apart && !panel.no_channels);
        state.channels.clear();
        assert!(comms_panel(&state, ANA, "", Locale::Es, &now()).no_channels);
    }

    /// A quién se le ofrece escribir.
    ///
    /// Roturas que atrapa: ofrecerse a uno mismo (el servidor lo rechaza);
    /// buscar sólo por usuario, o sólo por nombre; una búsqueda que distingue
    /// acentos o mayúsculas; dejar fuera a alguien con quien ya hay hilo.
    #[test]
    fn everyone_but_yourself_is_offered_by_name_or_username() {
        let person = |id: &str, username: &str, name: Option<&str>| Person { id: id.into(), username: username.into(), name: name.map(str::to_string), last_seen_at: None };
        let people = [person("u1", "ana", Some("Ana Ruiz")), person("u2", "lmora", Some("Luis Mora")), person("u3", "bea", None), person("u4", "angel", Some("Ángel Pérez"))];
        let found = |query: &str| people_matching(&people, Some("u1"), query).into_iter().map(|person| person.id).collect::<Vec<_>>();

        assert_eq!(found(""), ["u2", "u3", "u4"]);
        assert_eq!(found("  LUIS "), ["u2"]);
        assert_eq!(found("lmo"), ["u2"]);
        assert_eq!(found("angel p"), ["u4"]);
        assert_eq!(found("ana"), Vec::<String>::new());
        assert_eq!(people_matching(&people, None, "ana").len(), 1);
    }

    /// Cómo se titula lo que se tiene delante.
    ///
    /// Roturas que atrapa: un canal sin su almohadilla; el título de un canal
    /// para un directo; un título para algo que ya no está en la lista.
    #[test]
    fn the_conversation_in_front_has_a_title() {
        let mut state = state();
        let at = Utc.with_ymd_and_hms(2026, 10, 15, 16, 0, 0).unwrap();
        assert_eq!(conversation_title(&state, at).map(|title| (title.title, title.direct)), Some(("#Diseño".into(), false)));
        state.directs[1].last_seen_at = Some("2026-10-15T15:55:00Z".into());
        state.selected = Some(Conversation::Direct { conversation_id: "c2".into() });
        assert_eq!(conversation_title(&state, at).map(|title| (title.title, title.direct, title.present)), Some(("bea".into(), true, true)));
        state.selected = Some(Conversation::Channel { space_id: "borrado".into() });
        assert_eq!(conversation_title(&state, at), None);
        state.selected = None;
        assert_eq!(conversation_title(&state, at), None);
    }
}
