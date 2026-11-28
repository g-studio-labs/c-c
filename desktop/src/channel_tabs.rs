//! Las pestañas de un canal (`ChannelView.tsx`): la conversación,
//! Multimedia, Grabaciones —si el servidor graba— y Enlaces, con la caja de
//! «Buscar en #canal» a su derecha; y el diálogo de «Llamar con invitados».
//!
//! Qué se pide, cómo se pegan las páginas y qué enseña una pestaña sin
//! filas lo decide el core (`cac_core::channel_tabs`). Aquí se pinta con las
//! medidas de `MediaTab.tsx`, `LinksTab.tsx`, `ThreadSearch.tsx`,
//! `RecordingsPanel.tsx` y `GuestInviteDialog.tsx`, y se le pasa cada gesto.

use std::future::Future;

use cac_core::{ApiError, CallInvite, ChannelTab, ChatLinkItem, ChatMediaItem, ChatMessage, Conversation, Core, PagedTab, Recording, RecordingPlace, TabItem, TabPage, TabShows};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable as _, Icon, IconName, Selectable as _, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::chat::{ChatView, OpenRecording};
use crate::files;
use crate::lang::{error_text, locale, tr};
use crate::theme::Leading as _;
use crate::thumbs::{Thumb, Thumbs};

/// Cuánto se queda a la vista un aviso de esta pantalla.
const NOTICE_FOR: std::time::Duration = std::time::Duration::from_secs(6);
/// El alto de una miniatura de Multimedia: el `h-28` de `MediaTab.tsx`.
const TILE_HEIGHT: f32 = 112.;
/// El ancho del diálogo de invitados: el `sm:max-w-lg`.
const GUEST_WIDTH: f32 = 512.;

/// Lo grabado en el canal que se mira.
#[derive(Default)]
struct RecordingList {
    /// `None` mientras llega la primera lectura.
    rows: Option<Vec<Recording>>,
    failed: bool,
    turn: u64,
    /// Las que se están bajando o borrando: su botón gira y no se pulsa dos
    /// veces (un segundo clic sería una segunda descarga de cientos de MB).
    busy: Vec<String>,
    /// La que se pidió ver desde un enlace del chat: va marcada y a la vista.
    pointed: Option<String>,
    /// La fila que hay que traer a la vista en cuanto se pueda.
    scroll_to: Option<usize>,
    scroll: ScrollHandle,
}

/// Las pestañas del canal que se tiene delante, y debajo lo que muestre la
/// elegida.
pub struct ChannelTabsView {
    thread: Entity<ChatView>,
    /// El canal al que pertenece lo que hay aquí. Al cambiar, todo vuelve a
    /// empezar en la conversación: las fotos de un canal bajo el nombre de
    /// otro es el fallo que el hilo ya tiene resuelto.
    space_id: Option<String>,
    tab: ChannelTab,
    recordings_enabled: bool,
    /// Ya contestó si el canal graba. Hasta entonces `recordings_enabled` es
    /// un «aún no sé», no un «no».
    policy_known: bool,
    /// Un enlace a una grabación pulsado antes de saberlo: se sigue al llegar
    /// la respuesta en vez de decir que no está.
    waiting_link: Option<String>,
    /// Un enlace a una grabación de otro canal: se elige ese canal y, al
    /// llegar a él, se sigue (canal, grabación).
    arriving: Option<(String, String)>,
    search: Entity<InputState>,
    /// Lo buscado que ya se pidió. Una caja para las cuatro pestañas, que se
    /// vacía al cambiar: «factura» quiere decir otra cosa en Multimedia.
    query: String,
    media: PagedTab<ChatMediaItem>,
    links: PagedTab<ChatLinkItem>,
    hits: PagedTab<ChatMessage>,
    recordings: RecordingList,
    thumbs: Thumbs,
    notice: Option<(u64, String)>,
    notices: u64,
    /// «Llamar con invitados», abierto. Lo pinta quien tiene la ventana
    /// entera, encima de todo.
    pub(crate) guest: Option<Entity<GuestCall>>,
    _on_guest: Option<Subscription>,
    _on_search: Subscription,
    _on_recording_link: Subscription,
}

impl ChannelTabsView {
    pub fn new(thread: Entity<ChatView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx));
        let on_search = cx.subscribe_in(&search, window, |this, search, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let query = search.read(cx).value().to_string();
                this.searched(query, cx);
            }
        });
        let on_recording_link = cx.subscribe_in(&thread, window, |this, _, event: &OpenRecording, window, cx| this.show_recording(event.id.clone(), window, cx));
        Self {
            thread,
            space_id: None,
            tab: ChannelTab::Thread,
            recordings_enabled: false,
            policy_known: false,
            waiting_link: None,
            arriving: None,
            search,
            query: String::new(),
            media: PagedTab::default(),
            links: PagedTab::default(),
            hits: PagedTab::default(),
            recordings: RecordingList::default(),
            thumbs: Thumbs::default(),
            notice: None,
            notices: 0,
            guest: None,
            _on_guest: None,
            _on_search: on_search,
            _on_recording_link: on_recording_link,
        }
    }

    fn say(&mut self, text: String, cx: &mut Context<Self>) {
        self.notices += 1;
        let serial = self.notices;
        self.notice = Some((serial, text));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_FOR).await;
            this.update(cx, |this, cx| {
                if this.notice.as_ref().is_some_and(|(shown, _)| *shown == serial) {
                    this.notice = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Se mira otro canal: vuelta a la conversación, sin búsqueda, y se
    /// pregunta si este graba.
    fn follow_channel(&mut self, space_id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.space_id = space_id.clone();
        self.tab = ChannelTab::Thread;
        self.recordings_enabled = false;
        self.policy_known = false;
        // Cambiar de canal lo olvida todo menos el enlace que llevó hasta él.
        self.waiting_link = self.arriving.take().filter(|(space, _)| space_id.as_deref() == Some(space.as_str())).map(|(_, id)| id);
        self.query.clear();
        self.media = PagedTab::default();
        self.links = PagedTab::default();
        self.hits = PagedTab::default();
        self.recordings = RecordingList::default();
        self.thumbs.clear();
        self.notice = None;
        self.guest = None;
        self._on_guest = None;
        let name = space_id.as_deref().and_then(|space| {
            Backend::snapshot(cx).read(cx).comms.channels.iter().find(|channel| channel.space_id == space).map(|channel| channel.name.clone())
        });
        let placeholder = cac_core::text_with(locale(cx), "chat:searchHere", &[("name", name.as_deref().unwrap_or_default())]);
        self.search.update(cx, |search, cx| {
            search.set_value("", window, cx);
            search.set_placeholder(placeholder, window, cx);
        });
        let Some(space) = space_id else { return };
        let asked = space.clone();
        let enabled = Backend::ask(cx, move |core| async move { core.channel_tabs().recordings_enabled(&asked).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(enabled) = enabled.await else { return };
            this.update_in(cx, |this, window, cx| {
                if this.space_id.as_deref() == Some(space.as_str()) {
                    this.recordings_enabled = enabled;
                    this.policy_known = true;
                    if let Some(id) = this.waiting_link.take() {
                        this.point_at(id, window, cx);
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn pick(&mut self, tab: ChannelTab, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        self.recordings.pointed = None;
        self.recordings.scroll_to = None;
        self.query.clear();
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        self.reload(cx);
        cx.notify();
    }

    /// Un enlace `cac:recording/…` del chat: al canal donde se grabó, que
    /// dice el servidor —el enlace pudo copiarse de otro canal, o pegarse en
    /// un directo—, y ahí a su pestaña.
    fn show_recording(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let clicked_in = Backend::snapshot(cx).read(cx).comms.selected.clone();
        let asked = id.clone();
        let answer = Backend::ask(cx, move |core| async move { core.channel_tabs().recording_place(&asked).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                let snapshot = Backend::snapshot(cx).read(cx).comms.selected.clone();
                // Quien se fue a otra conversación mientras se preguntaba ya
                // no está donde pulsó: llevarle de vuelta le quitaría lo que
                // tiene delante.
                if snapshot != clicked_in {
                    return;
                }
                match answer {
                    Ok(RecordingPlace::Channel { space_id }) if snapshot == Some(Conversation::Channel { space_id: space_id.clone() }) => this.point_at(id, window, cx),
                    Ok(RecordingPlace::Channel { space_id }) => {
                        this.arriving = Some((space_id.clone(), id));
                        if !Backend::core(cx).comms().select(Conversation::Channel { space_id }) {
                            this.arriving = None;
                            this.say(tr(cx, "recordings:channelHidden"), cx);
                        }
                    }
                    Ok(RecordingPlace::Hidden) => this.say(tr(cx, "recordings:channelHidden"), cx),
                    Err(ApiError::Server { status: 404, .. }) => this.say(tr(cx, "recordings:notFound"), cx),
                    Err(error) => {
                        let text = error_text(cx, &error, "recordings:notFound");
                        this.say(text, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// A Grabaciones del canal que se mira, con esa marcada en cuanto llegue
    /// la lista (`onOpenRecordings` en `ChannelView.tsx`, que sólo cambia de
    /// pestaña).
    fn point_at(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.policy_known {
            self.waiting_link = Some(id);
            return;
        }
        if !self.recordings_enabled {
            self.say(tr(cx, "recordings:notInChannel"), cx);
            return;
        }
        self.pick(ChannelTab::Recordings, window, cx);
        self.recordings.pointed = Some(id);
    }

    fn searched(&mut self, query: String, cx: &mut Context<Self>) {
        if query == self.query {
            return;
        }
        self.query = query;
        self.reload(cx);
        cx.notify();
    }

    /// Vuelve a leer desde el principio lo que enseña la pestaña elegida.
    fn reload(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            ChannelTab::Thread if self.query.trim().is_empty() => self.hits = PagedTab::default(),
            ChannelTab::Thread => self.load_hits(false, cx),
            ChannelTab::Media => self.load_media(false, cx),
            ChannelTab::Links => self.load_links(false, cx),
            ChannelTab::Recordings => self.load_recordings(cx),
        }
    }

    /// Lee una página de una pestaña paginada: la primera (`more` falso) o
    /// la siguiente. Lo que llegue tarde de otra búsqueda lo tira el turno.
    fn load<T, F, Fut>(&mut self, at: fn(&mut Self) -> &mut PagedTab<T>, more: bool, fetch: F, cx: &mut Context<Self>)
    where
        T: TabItem + Send + 'static,
        F: FnOnce(Core, String, String, Option<String>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<TabPage<T>, ApiError>> + Send + 'static,
    {
        let Some(space) = self.space_id.clone() else { return };
        let query = self.query.clone();
        let (turn, before) = if more {
            match at(self).next() {
                Some((turn, before)) => (turn, Some(before)),
                None => return,
            }
        } else {
            (at(self).restart(), None)
        };
        let answer = Backend::ask(cx, move |core| fetch(core, space, query, before));
        cx.spawn(async move |this, cx| {
            let Ok(answer) = answer.await else { return };
            this.update(cx, |this, cx| {
                if at(this).landed(turn, answer) {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn load_media(&mut self, more: bool, cx: &mut Context<Self>) {
        self.load(|this: &mut Self| &mut this.media, more, |core, space, query, before| async move { core.channel_tabs().media(&space, &query, before.as_deref()).await }, cx);
    }

    fn load_links(&mut self, more: bool, cx: &mut Context<Self>) {
        self.load(|this: &mut Self| &mut this.links, more, |core, space, query, before| async move { core.channel_tabs().links(&space, &query, before.as_deref()).await }, cx);
    }

    fn load_hits(&mut self, more: bool, cx: &mut Context<Self>) {
        self.load(|this: &mut Self| &mut this.hits, more, |core, space, query, before| async move { core.channel_tabs().search(&space, &query, before.as_deref()).await }, cx);
    }

    fn load_recordings(&mut self, cx: &mut Context<Self>) {
        let Some(space) = self.space_id.clone() else { return };
        self.recordings.turn += 1;
        let turn = self.recordings.turn;
        self.recordings.failed = false;
        let answer = Backend::ask(cx, move |core| async move { core.channel_tabs().recordings(&space).await });
        cx.spawn(async move |this, cx| {
            let Ok(answer) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.recordings.turn != turn {
                    return;
                }
                match answer {
                    Ok(rows) => {
                        if let Some(pointed) = this.recordings.pointed.clone() {
                            match rows.iter().position(|row| row.id == pointed) {
                                Some(at) => this.recordings.scroll_to = Some(at),
                                // Borrada, o de otro canal: decirlo, en vez
                                // de una lista que no dice por qué se abrió.
                                None => {
                                    this.recordings.pointed = None;
                                    this.say(tr(cx, "recordings:notInChannel"), cx);
                                }
                            }
                        }
                        this.recordings.rows = Some(rows);
                    }
                    Err(_) => this.recordings.failed = true,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ── Gestos ──────────────────────────────────────────────────────────

    /// Baja un fichero de Multimedia y lo abre con el programa del sistema.
    fn open_media(&mut self, url: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let answer = Backend::ask(cx, move |core| async move {
            files::fetch_for_opening(&std::env::temp_dir(), &name, |target| async move { core.chat().save_file(&url, &target).await }).await
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| match result {
                Ok(path) => files::open_with_system(&path, cx, |this, text, cx| this.say(text, cx)),
                Err(Some(error)) => {
                    let text = error_text(cx, &error, "work:task.errOpenFile");
                    this.say(text, cx);
                }
                Err(None) => this.say(tr(cx, "work:task.errOpenFile"), cx),
            })
            .ok();
        })
        .detach();
    }

    fn set_busy(&mut self, id: &str, busy: bool, cx: &mut Context<Self>) {
        self.recordings.busy.retain(|held| held != id);
        if busy {
            self.recordings.busy.push(id.to_string());
        }
        cx.notify();
    }

    /// Abre la grabación fuera de la app, en el navegador, con un pase corto.
    fn watch(&mut self, recording_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let answer = Backend::ask(cx, move |core| async move { core.channel_tabs().recording_watch_url(&recording_id).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = answer.await else { return };
            this.update(cx, |this, cx| match answer {
                Ok(url) => cx.open_url(&url),
                Err(error) => {
                    let text = error_text(cx, &error, "recordings:cannotPlay");
                    this.say(text, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Guarda una copia de la grabación donde se elija.
    fn download(&mut self, recording: Recording, window: &mut Window, cx: &mut Context<Self>) {
        if self.recordings.busy.contains(&recording.id) {
            return;
        }
        let home = std::env::home_dir().unwrap_or_else(std::env::temp_dir);
        let name = cac_core::recording_file_name(&recording, &chrono::Local);
        let chosen = cx.prompt_for_new_path(&home, Some(&name));
        let id = recording.id.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(target))) = chosen.await else { return };
            let shown = target.display().to_string();
            let Ok(answer) = this.update(cx, |this, cx| {
                this.set_busy(&id, true, cx);
                this.say(tr(cx, "recordings:downloadStarted"), cx);
                let id = id.clone();
                Backend::ask(cx, move |core| async move { core.channel_tabs().save_recording(&id, &target).await })
            }) else {
                return;
            };
            let saved = answer.await;
            this.update(cx, |this, cx| {
                this.set_busy(&id, false, cx);
                let text = match saved {
                    Ok(Ok(())) => cac_core::text_with(locale(cx), "recordings:downloaded", &[("path", &shown)]),
                    _ => tr(cx, "recordings:downloadFailed"),
                };
                this.say(text, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Borra una grabación, preguntando: se lleva el vídeo y todas las
    /// pistas, y de eso no se vuelve. Si el servidor no la borra, la fila se
    /// queda: quitarla diría que ya no está cuando sigue ahí.
    fn delete(&mut self, recording_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.recordings.busy.contains(&recording_id) {
            return;
        }
        let (title, confirm, cancel) = (tr(cx, "recordings:deleteConfirm"), tr(cx, "recordings:delete"), tr(cx, "common:action.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, None, &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let Ok(deleted) = this.update(cx, |this, cx| {
                this.set_busy(&recording_id, true, cx);
                let id = recording_id.clone();
                Backend::ask(cx, move |core| async move { core.channel_tabs().delete_recording(&id).await })
            }) else {
                return;
            };
            let deleted = deleted.await;
            this.update(cx, |this, cx| {
                this.set_busy(&recording_id, false, cx);
                match deleted {
                    Ok(Ok(())) => {
                        if let Some(rows) = this.recordings.rows.as_mut() {
                            rows.retain(|row| row.id != recording_id);
                        }
                    }
                    Ok(Err(error)) => {
                        let text = error_text(cx, &error, "common:misc.errSave");
                        this.say(text, cx);
                    }
                    Err(_) => {}
                }
            })
            .ok();
        })
        .detach();
    }

    /// Abre «Llamar con invitados» para el canal que se mira.
    pub(crate) fn open_guest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space_id) = self.space_id.clone() else { return };
        let Some(org_id) = Backend::snapshot(cx).read(cx).comms.org_id.clone() else { return };
        let guest = cx.new(|cx| GuestCall::new(org_id, space_id, window, cx));
        self._on_guest = Some(cx.subscribe(&guest, |this, _, _: &GuestCallClosed, cx| {
            this.guest = None;
            this._on_guest = None;
            cx.notify();
        }));
        self.guest = Some(guest);
        cx.notify();
    }

    // ── Pintar ──────────────────────────────────────────────────────────

    fn render_tab(&self, tab: ChannelTab, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, primary, foreground) = (theme.muted_foreground, theme.primary, theme.foreground);
        let active = self.tab == tab;
        let icon = match tab {
            ChannelTab::Thread => Lucide::MessageSquare,
            ChannelTab::Media => Lucide::Image,
            ChannelTab::Recordings => Lucide::Film,
            ChannelTab::Links => Lucide::Link2,
        };
        let label = tr(cx, tab.label_key());
        div()
            .id(SharedString::from(format!("conversation-tab-{}", tab.id())))
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .rounded(px(8.))
            .px_2()
            .py_1()
            .leading_xs()
            .cursor_pointer()
            .map(|this| if active { this.font_weight(FontWeight::MEDIUM).text_color(primary).bg(primary.opacity(0.1)) } else { this.text_color(muted).hover(move |style| style.text_color(foreground)) })
            .on_click(cx.listener(move |this, _, window, cx| this.pick(tab, window, cx)))
            .aria_label(label.clone())
            .aria_selected(active)
            .child(Icon::new(icon).size_3().flex_none())
            .child(label)
            .test_support()
            .into_any_element()
    }

    /// Lo que enseña una pestaña sin filas (`TabState.tsx`).
    fn render_state(shows: TabShows, empty: String, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let (id, child) = match shows {
            TabShows::Failed => ("channel-tab-failed", div().text_color(danger).child(tr(cx, "chat:errTab")).into_any_element()),
            TabShows::Loading => ("channel-tab-loading", Spinner::new().color(muted).into_any_element()),
            _ => ("channel-tab-empty", div().text_color(muted).child(empty.clone()).into_any_element()),
        };
        let label = match shows {
            TabShows::Failed => tr(cx, "chat:errTab"),
            TabShows::Loading => String::new(),
            _ => empty,
        };
        div().id(id).w_full().flex().justify_center().p_6().leading_sm().aria_label(label).child(child).test_support().into_any_element()
    }

    /// «Cargar lo anterior»: una ventana más de canal hacia atrás.
    fn render_more(&self, loading: bool, more: fn(&mut Self, &mut Context<Self>), cx: &mut Context<Self>) -> AnyElement {
        Button::new("channel-tab-more")
            .outline()
            .small()
            .w_full()
            .loading(loading)
            .disabled(loading)
            .label(tr(cx, "chat:loadMore"))
            .on_click(cx.listener(move |this, _, _, cx| more(this, cx)))
            .into_any_element()
    }

    fn render_media(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let shows = self.media.shows();
        if shows != TabShows::Rows {
            let empty = tr(cx, if self.query.trim().is_empty() { "chat:mediaEmpty" } else { "chat:noHits" });
            return Self::render_state(shows, empty, cx);
        }
        // Las miniaturas de las imágenes se piden al pintarlas, una vez cada una.
        for item in self.media.rows().iter().filter(|item| item.is_image()).cloned().collect::<Vec<_>>() {
            let url = item.url.clone();
            self.thumbs.request(item.url.clone(), move |core| async move { core.chat().picture(&url).await }, |this: &mut Self| &mut this.thumbs, |_, _, _| {}, window, cx);
        }
        let theme = cx.theme();
        let (border, muted, tile_bg) = (theme.border, theme.muted_foreground, theme.muted.opacity(0.3));
        let zone = chrono::Local;
        let tiles: Vec<AnyElement> = self
            .media
            .rows()
            .iter()
            .map(|item| {
                let meta = cac_core::row_meta(item.author_name.as_deref(), &item.posted_at, None, locale(cx), &zone);
                let picture = match self.thumbs.get(&item.url) {
                    Some(Thumb::Ready(image)) => img(image.clone()).size_full().object_fit(ObjectFit::Cover).into_any_element(),
                    _ => Icon::new(Lucide::Paperclip).size_5().text_color(muted).into_any_element(),
                };
                let (url, name) = (item.url.clone(), item.file_name.clone());
                div()
                    .id(SharedString::from(format!("media-{}", item.id)))
                    .w(relative(0.5))
                    .p_1()
                    .child(
                        div()
                            .overflow_hidden()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(border)
                            .cursor_pointer()
                            .child(div().h(px(TILE_HEIGHT)).w_full().flex().items_center().justify_center().bg(tile_bg).overflow_hidden().child(picture))
                            .child(
                                div()
                                    .p_1p5()
                                    .flex()
                                    .flex_col()
                                    .child(div().truncate().leading_xs().font_weight(FontWeight::MEDIUM).child(item.file_name.clone()))
                                    .child(div().truncate().text_size(px(11.)).text_color(muted).child(meta)),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.open_media(url.clone(), name.clone(), window, cx)))
                    .aria_label(item.file_name.clone())
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let more = self.media.has_more().then(|| self.render_more(self.media.loading(), |this, cx| this.load_media(true, cx), cx));
        div().p_2().flex().flex_col().gap_2().child(div().flex().flex_wrap().children(tiles)).children(more.map(|more| div().px_1().child(more))).into_any_element()
    }

    fn render_links(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shows = self.links.shows();
        if shows != TabShows::Rows {
            let empty = tr(cx, if self.query.trim().is_empty() { "chat:linksEmpty" } else { "chat:noHits" });
            return Self::render_state(shows, empty, cx);
        }
        let theme = cx.theme();
        let (border, muted, primary) = (theme.border, theme.muted_foreground, theme.primary);
        let zone = chrono::Local;
        let rows: Vec<AnyElement> = self
            .links
            .rows()
            .iter()
            .enumerate()
            .map(|(index, link)| {
                let label = link.label.clone().filter(|label| !label.trim().is_empty());
                let shown = label.clone().unwrap_or_else(|| link.url.clone());
                let meta = cac_core::row_meta(link.author_name.as_deref(), &link.posted_at, label.is_some().then_some(link.url.as_str()), locale(cx), &zone);
                let (url, hint) = (link.url.clone(), link.url.clone());
                div()
                    .id(SharedString::from(format!("link-{index}")))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .p_2()
                    .cursor_pointer()
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                    // Al navegador de fuera: un enlace externo es de fuera.
                    .on_click(move |_, _, cx| {
                        if url.starts_with("https://") || url.starts_with("http://") {
                            cx.open_url(&url);
                        }
                    })
                    .aria_label(shown.clone())
                    .child(div().truncate().leading_sm().text_color(primary).underline().child(shown))
                    .child(div().truncate().text_size(px(11.)).text_color(muted).child(meta))
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let more = self.links.has_more().then(|| self.render_more(self.links.loading(), |this, cx| this.load_links(true, cx), cx));
        div().p_3().flex().flex_col().gap_1().children(rows).children(more).into_any_element()
    }

    fn render_hits(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shows = self.hits.shows();
        if shows != TabShows::Rows {
            return Self::render_state(shows, tr(cx, "chat:noHits"), cx);
        }
        let theme = cx.theme();
        let (border, muted, foreground) = (theme.border, theme.muted_foreground, theme.foreground);
        let base = Backend::core(cx).base_url().to_string();
        let zone = chrono::Local;
        let rows: Vec<AnyElement> = self
            .hits
            .rows()
            .iter()
            .map(|message| {
                let author = if message.is_system() {
                    tr(cx, "chat:systemAuthor")
                } else if message.author_name.trim().is_empty() {
                    "unknown".to_string()
                } else {
                    message.author_name.clone()
                };
                let when = cac_core::local_moment(&message.created_at, locale(cx), &zone);
                let body = cac_core::prose_for_painting(&cac_core::message_body(&message.body, &base).text);
                div()
                    .id(SharedString::from(format!("hit-{}", message.id)))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .aria_label(body.clone())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .leading_xs()
                            .text_color(muted)
                            .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).text_color(foreground).child(author))
                            .child(div().flex_none().child(when)),
                    )
                    .child(div().leading_sm().child(body))
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let more = self.hits.has_more().then(|| self.render_more(self.hits.loading(), |this, cx| this.load_hits(true, cx), cx));
        div().p_3().flex().flex_col().gap_2().children(rows).children(more).into_any_element()
    }

    fn render_recordings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, danger, primary) = (theme.border, theme.muted_foreground, theme.danger, theme.primary);
        let state = |shows, empty, cx: &App| Self::tab_body().child(Self::render_state(shows, empty, cx)).test_support().into_any_element();
        if self.recordings.failed {
            return state(TabShows::Failed, String::new(), cx);
        }
        let Some(rows) = &self.recordings.rows else {
            return state(TabShows::Loading, String::new(), cx);
        };
        if rows.is_empty() {
            return state(TabShows::Empty, tr(cx, "recordings:panelEmpty"), cx);
        }
        let rows: Vec<AnyElement> = rows
            .iter()
            .map(|recording| {
                let busy = self.recordings.busy.contains(&recording.id);
                let status = tr(cx, recording.status_key());
                let duration = cac_core::recording_duration(recording.duration_ms);
                let by = recording.started_by_name.as_deref().filter(|name| !name.trim().is_empty()).map(|name| cac_core::text_with(locale(cx), "recordings:by", &[("name", name)]));
                let line = [Some(status.clone()), (!duration.is_empty()).then_some(duration), by].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                let id = recording.id.clone();
                let actions = recording.playable().then(|| {
                    let (watch, save) = (recording.id.clone(), recording.clone());
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Button::new(SharedString::from(format!("recording-watch-{id}")))
                                .ghost()
                                .small()
                                .icon(Icon::new(Lucide::Play))
                                .tooltip(tr(cx, "recordings:openOutside"))
                                .on_click(cx.listener(move |this, _, window, cx| this.watch(watch.clone(), window, cx))),
                        )
                        .child(
                            Button::new(SharedString::from(format!("recording-download-{id}")))
                                .ghost()
                                .small()
                                .icon(Icon::new(Lucide::Download))
                                .tooltip(tr(cx, "recordings:download"))
                                .loading(busy)
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, window, cx| this.download(save.clone(), window, cx))),
                        )
                });
                let delete = id.clone();
                let pointed = self.recordings.pointed.as_deref() == Some(id.as_str());
                div()
                    .id(SharedString::from(format!("recording-{id}")))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if pointed { primary } else { border })
                    .when(pointed, |this| this.bg(primary.opacity(0.06)))
                    .aria_selected(pointed)
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .aria_label(line.clone())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .leading_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if recording.failed() { danger } else { muted })
                                    .child(line),
                            )
                            .children(actions)
                            .child(
                                Button::new(SharedString::from(format!("recording-delete-{id}")))
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(Lucide::Trash))
                                    .tooltip(tr(cx, "recordings:delete"))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| this.delete(delete.clone(), window, cx))),
                            ),
                    )
                    // Por qué está a medias, donde se ve.
                    .when(recording.status == "partial", |this| this.child(div().leading_xs().text_color(muted).child(tr(cx, "recordings:statusPartialWhy"))))
                    .test_support()
                    .into_any_element()
            })
            .collect();
        // Las filas, hijas directas de lo que se desliza: `scroll_to_item`
        // cuenta por hijos.
        Self::tab_body().track_scroll(&self.recordings.scroll).p_3().flex().flex_col().gap_2().children(rows).test_support().into_any_element()
    }

    /// Lo que se desliza debajo de las pestañas.
    fn tab_body() -> Stateful<Div> {
        div().id("channel-tab-body").w_full().flex_1().min_h_0().overflow_y_scroll()
    }
}

impl Render for ChannelTabsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let space_id = match &Backend::snapshot(cx).read(cx).comms.selected {
            Some(Conversation::Channel { space_id }) => Some(space_id.clone()),
            _ => None,
        };
        if space_id != self.space_id {
            self.follow_channel(space_id, window, cx);
        }
        // Si deja de grabar estando en su pestaña, se vuelve a la conversación.
        if self.tab == ChannelTab::Recordings && !self.recordings_enabled {
            self.tab = ChannelTab::Thread;
        }
        let theme = cx.theme();
        let (border, muted, foreground, background) = (theme.border, theme.muted_foreground, theme.foreground, theme.background);
        if let Some(at) = self.recordings.scroll_to.filter(|_| self.tab == ChannelTab::Recordings) {
            // Un `ScrollHandle` que aún no se pintó no sabe su alto y tira la
            // petición sin moverse: se pide cuando la lista ya se pintó una vez.
            if self.recordings.scroll.bounds().size.height > px(0.) {
                self.recordings.scroll.scroll_to_item(at);
                self.recordings.scroll_to = None;
            } else {
                window.refresh();
            }
        }
        let tabs: Vec<AnyElement> = cac_core::channel_tabs(self.recordings_enabled).into_iter().map(|tab| self.render_tab(tab, cx)).collect();
        let searching = !self.query.trim().is_empty();
        let search = self.tab.searchable().then(|| {
            div()
                .id("channel-search")
                .ml_auto()
                .min_w_0()
                .flex_1()
                .pl_2()
                // Sin caja, como en Tauri: es un modificador de la pestaña activa en la
                // misma fila, no un campo de formulario.
                .text_xs()
                .child(Input::new(&self.search).id("channel-search-input").appearance(false).cleanable(true).prefix(Icon::new(IconName::Search).size_3().text_color(muted)))
                .test_support()
        });
        let body: AnyElement = match self.tab {
            ChannelTab::Thread if !searching => self.thread.clone().into_any_element(),
            ChannelTab::Recordings => self.render_recordings(cx),
            ChannelTab::Thread => Self::tab_body().child(self.render_hits(cx)).test_support().into_any_element(),
            ChannelTab::Media => Self::tab_body().child(self.render_media(window, cx)).test_support().into_any_element(),
            ChannelTab::Links => Self::tab_body().child(self.render_links(cx)).test_support().into_any_element(),
        };
        let notice = self.notice.clone().map(|(_, text)| {
            div()
                .id("channel-tabs-notice")
                .absolute()
                .bottom_3()
                .left_3()
                .right_3()
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .bg(background)
                .text_color(foreground)
                .shadow_lg()
                .px_3()
                .py_2()
                .leading_xs()
                .aria_label(text.clone())
                .child(text)
                .test_support()
        });
        div()
            .id("channel-tabs-view")
            // El hilo es `flex_1`: necesita un padre columna que lo estire,
            // o se queda en alto cero y no pinta ningún mensaje.
            .w_full()
            .flex_1()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("conversation-tabs")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .border_b_1()
                    .border_color(border)
                    .px_2()
                    .py_1()
                    .children(tabs)
                    .children(search)
                    .test_support(),
            )
            .child(div().flex_1().min_h_0().flex().flex_col().overflow_hidden().child(body))
            .children(notice)
    }
}

// ── Llamar con invitados ────────────────────────────────────────────────

/// «Llamar con invitados» se cerró: quien lo tiene lo quita.
pub struct GuestCallClosed;

/// Llamar con gente de fuera desde un canal (`GuestInviteDialog.tsx`).
///
/// Crear la llamada es un formulario con su botón y no un clic suelto: lo
/// que sale de aquí es un enlace que abre una sala a quien lo tenga. Los
/// invitados entran por la web; desde aquí se crea, se copia el enlace, se
/// entra como miembro o se cierra.
pub struct GuestCall {
    org_id: String,
    space_id: String,
    title: Entity<InputState>,
    ttl_hours: u32,
    creating: bool,
    /// `None` mientras llegan las abiertas.
    invites: Option<Vec<CallInvite>>,
    problem: Option<String>,
    /// «Enlace copiado…», dicho en el diálogo.
    copied: bool,
    focus: FocusHandle,
    _on_enter: Subscription,
}

impl EventEmitter<GuestCallClosed> for GuestCall {}

impl Focusable for GuestCall {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl GuestCall {
    fn new(org_id: String, space_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "calls:titlePlaceholder");
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        title.update(cx, |title, cx| title.focus(window, cx));
        let on_enter = cx.subscribe_in(&title, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create(window, cx);
            }
        });
        let (org, space) = (org_id.clone(), space_id.clone());
        let open = Backend::ask(cx, move |core| async move { core.channel_tabs().call_invites(&org, &space).await });
        cx.spawn(async move |this, cx| {
            let Ok(open) = open.await else { return };
            this.update(cx, |this, cx| {
                match open {
                    Ok(invites) => this.invites = Some(invites),
                    Err(error) => {
                        this.invites = Some(Vec::new());
                        this.problem = Some(error_text(cx, &error, "chat:errTab"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self { org_id, space_id, title, ttl_hours: 24, creating: false, invites: None, problem: None, copied: false, focus: cx.focus_handle(), _on_enter: on_enter }
    }

    fn copy(&mut self, invite: &CallInvite, cx: &mut Context<Self>) {
        let link = Backend::core(cx).channel_tabs().guest_link(invite);
        cx.write_to_clipboard(ClipboardItem::new_string(link));
        self.copied = true;
        cx.notify();
    }

    /// Crea la llamada y deja su enlace en el portapapeles. Si no se crea, el
    /// título se queda escrito y se dice por qué.
    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let written = self.title.read(cx).value().to_string();
        if self.creating || cac_core::call_title(&written).is_none() {
            return;
        }
        self.creating = true;
        self.problem = None;
        self.copied = false;
        cx.notify();
        let (org, space, ttl) = (self.org_id.clone(), self.space_id.clone(), self.ttl_hours);
        let created = Backend::ask(cx, move |core| async move { core.channel_tabs().create_call_invite(&org, &space, &written, ttl).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(created) = created.await else { return };
            this.update_in(cx, |this, window, cx| {
                this.creating = false;
                match created {
                    Ok(invite) => {
                        this.copy(&invite, cx);
                        this.invites.get_or_insert_with(Vec::new).insert(0, invite);
                        this.title.update(cx, |title, cx| title.set_value("", window, cx));
                    }
                    Err(error) => this.problem = Some(error_text(cx, &error, "common:misc.errSave")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Entra a la llamada como miembro, en esta app.
    fn join(&mut self, invite_id: String, cx: &mut Context<Self>) {
        Backend::run(cx, move |core| async move { core.voice().join_meeting(&invite_id).await });
        cx.emit(GuestCallClosed);
    }

    /// Cierra una llamada, preguntando: el enlace deja de valer y se saca a
    /// los invitados que estén dentro.
    fn close_call(&mut self, invite_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let (title, body, confirm, cancel) = (tr(cx, "calls:closeTitle"), tr(cx, "calls:closeBody"), tr(cx, "calls:closeAction"), tr(cx, "common:action.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let Ok(closed) = this.update(cx, |_, cx| {
                let id = invite_id.clone();
                Backend::ask(cx, move |core| async move { core.channel_tabs().close_call_invite(&id).await })
            }) else {
                return;
            };
            let closed = closed.await;
            this.update(cx, |this, cx| {
                match closed {
                    Ok(Ok(())) => {
                        if let Some(invites) = this.invites.as_mut() {
                            invites.retain(|invite| invite.id != invite_id);
                        }
                    }
                    Ok(Err(error)) => this.problem = Some(error_text(cx, &error, "common:misc.errSave")),
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_invite(&self, invite: &CallInvite, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let until = cac_core::text_with(locale(cx), "calls:until", &[("when", &cac_core::local_moment(&invite.expires_at, locale(cx), &chrono::Local))]);
        let inside = (!invite.occupants.is_empty()).then(|| cac_core::text_count(locale(cx), "calls:inside", invite.occupants.len() as i64));
        let line = [Some(until), inside].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let (copy, join, close) = (invite.clone(), invite.id.clone(), invite.id.clone());
        let id = &invite.id;
        div()
            .id(SharedString::from(format!("guest-call-{id}")))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .px_3()
            .py_2()
            .aria_label(invite.title.clone())
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(div().truncate().leading_sm().font_weight(FontWeight::MEDIUM).child(invite.title.clone()))
                    .child(div().truncate().leading_xs().text_color(muted).child(line)),
            )
            .child(
                Button::new(SharedString::from(format!("guest-call-copy-{id}")))
                    .outline()
                    .small()
                    .icon(Icon::new(Lucide::Link2))
                    .tooltip(tr(cx, "calls:copyLink"))
                    .on_click(cx.listener(move |this, _, _, cx| this.copy(&copy, cx))),
            )
            .child(
                Button::new(SharedString::from(format!("guest-call-join-{id}")))
                    .primary()
                    .small()
                    .icon(Icon::new(Lucide::PhoneCall))
                    .label(tr(cx, "calls:join"))
                    .on_click(cx.listener(move |this, _, _, cx| this.join(join.clone(), cx))),
            )
            .child(
                Button::new(SharedString::from(format!("guest-call-close-{id}")))
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::X))
                    .tooltip(tr(cx, "calls:close"))
                    .on_click(cx.listener(move |this, _, window, cx| this.close_call(close.clone(), window, cx))),
            )
            .test_support()
            .into_any_element()
    }
}

impl Render for GuestCall {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.danger);
        let can_create = !self.creating && cac_core::call_title(self.title.read(cx).value().as_ref()).is_some();
        let ttls: Vec<AnyElement> = cac_core::GUEST_TTLS
            .iter()
            .map(|(hours, key)| {
                let hours = *hours;
                Button::new(SharedString::from(format!("guest-call-ttl-{hours}")))
                    .outline()
                    .small()
                    .selected(self.ttl_hours == hours)
                    .label(tr(cx, key))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ttl_hours = hours;
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let list: Vec<AnyElement> = self.invites.clone().unwrap_or_default().iter().map(|invite| self.render_invite(invite, cx)).collect();
        let none_open = self.invites.as_ref().is_some_and(|invites| invites.is_empty());
        let dialog = div()
            .id("guest-call")
            .test_support()
            .relative()
            .w(px(GUEST_WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .rounded(px(12.))
            .border_1()
            .border_color(foreground.opacity(0.1))
            .bg(background)
            .text_color(foreground)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(div().text_size(px(16.)).line_height(px(16.)).font_weight(FontWeight::MEDIUM).child(tr(cx, "calls:dialogTitle")))
                    .child(div().leading_sm().text_color(muted).child(tr(cx, "calls:dialogBody"))),
            )
            .child(
                div().absolute().top_2().right_2().child(
                    Button::new("guest-call-dismiss")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::X))
                        .tooltip(tr(cx, "common:action.close"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(GuestCallClosed))),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, "calls:titleLabel")))
                    .child(Input::new(&self.title).id("guest-call-title")),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, "calls:validFor")))
                            .child(div().flex().gap_1().children(ttls)),
                    )
                    .child(
                        Button::new("guest-call-create")
                            .primary()
                            .icon(Icon::new(Lucide::Link2))
                            .label(tr(cx, if self.creating { "calls:creating" } else { "calls:create" }))
                            .loading(self.creating)
                            .disabled(!can_create)
                            .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
                    ),
            )
            .children(self.problem.clone().map(|problem| div().id("guest-call-problem").leading_xs().text_color(danger).aria_label(problem.clone()).child(problem).test_support()))
            .when(self.copied, |this| {
                let said = tr(cx, "calls:linkCopied");
                this.child(div().id("guest-call-copied").leading_xs().text_color(muted).aria_label(said.clone()).child(said).test_support())
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().leading_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted).child(tr(cx, "calls:openCalls").to_uppercase()))
                    .when(self.invites.is_none(), |this| this.child(Spinner::new().color(muted)))
                    .when(none_open, |this| this.child(div().id("guest-call-none").leading_sm().text_color(muted).child(tr(cx, "calls:noOpenCalls")).test_support()))
                    .children(list),
            );
        div()
            .id("guest-call-backdrop")
            .size_full()
            .track_focus(&self.focus)
            .occlude()
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(GuestCallClosed);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(GuestCallClosed)))
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(black().opacity(0.35))
            .child(dialog)
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, ResponseTemplate};

    fn get(route: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET")).and(path(route.to_string()))
    }

    fn space(id: &str, name: &str, kind: Option<&str>) -> Value {
        json!({ "id": id, "orgId": "o1", "name": name, "color": "#888", "kind": kind, "folders": [],
            "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 0, "taskCount": 0 }], "people": [] })
    }

    /// Una organización con la sala general (`g`, la que se abre sin elegir)
    /// y «Producto» (`a`).
    fn server() -> FakeServer {
        let server = FakeServer::start();
        let org = json!({ "id": "o1", "name": "G Studio", "slug": "o1", "role": "member", "memberCount": 3, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("g", "General", Some("general")), space("a", "Producto", None)]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/unread").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([]))));
        server
    }

    fn open(cx: &mut TestAppContext) -> AnyWindowHandle {
        sign_in(cx);
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(760.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window").0
        });
        wait_until(cx, "llega la organización", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).comms.selected.is_some()));
        click(window, "rail-comms", cx);
        wait_until(cx, "sale el canal", |cx| on_screen(window, "conversation-tab-thread", cx));
        window
    }

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn click(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    fn label(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).and_then(|found| found.label().map(str::to_string)).unwrap_or_default()
        })
        .unwrap()
    }

    fn type_in(window: AnyWindowHandle, field: &str, text: &str, cx: &mut TestAppContext) {
        let (field, text) = (gpui_kit::SharedString::from(field.to_string()), text.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(field, cx);
            window.input(&text, cx);
        })
        .unwrap();
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    fn media(id: &str, name: &str, kind: &str) -> Value {
        json!({ "id": id, "url": format!("/api/v1/files/{id}"), "fileName": name, "contentType": kind, "messageId": "m1", "postedAt": "2026-01-05T18:00:00Z", "authorName": "Luis" })
    }

    /// Multimedia enseña lo subido al canal y sigue hacia atrás con el
    /// cursor del servidor; Enlaces enseña las palabras del enlace y lo abre
    /// fuera; sin grabación en el servidor no hay pestaña de Grabaciones; y
    /// cambiar de canal vuelve a la conversación.
    ///
    /// Roturas que atrapa: las pestañas que no abren nada; «Cargar lo
    /// anterior» que no manda el cursor o repite la página; un enlace que no
    /// se abre, o que enseña la URL pelada teniendo rótulo; Grabaciones en un
    /// servidor que no graba; las fotos de un canal bajo el nombre de otro.
    #[gpui_kit::test]
    fn media_and_links_list_what_the_channel_has(cx: &mut TestAppContext) {
        let server = server();
        server.mount(
            get("/api/v1/task-spaces/g/chat/media")
                .and(query_param("before", "2026-01-01T00:00:00Z"))
                .respond_with(ok(json!({ "items": [media("f2", "contrato.pdf", "application/pdf")] }))),
        );
        server.mount(
            get("/api/v1/task-spaces/g/chat/media").respond_with(ok(json!({ "items": [media("f1", "plano.pdf", "application/pdf")], "before": "2026-01-01T00:00:00Z" }))),
        );
        server.mount(
            get("/api/v1/task-spaces/g/chat/links")
                .respond_with(ok(json!({ "items": [{ "url": "https://example.com/runbook", "label": "el runbook", "messageId": "m2", "postedAt": "2026-01-05T18:00:00Z", "authorName": "Ana" }] }))),
        );
        server.mount(get("/api/v1/task-spaces/a/chat/media").respond_with(ok(json!({ "items": [] }))));
        server.mount(get("/api/v1/task-spaces/g/recordings/policy").respond_with(ok(json!({ "enabled": false, "active": null }))));
        server.install(cx);
        let window = open(cx);
        assert!(on_screen(window, "thread", cx) && !on_screen(window, "conversation-tab-recordings", cx));

        click(window, "conversation-tab-media", cx);
        wait_until(cx, "llega Multimedia", |cx| on_screen(window, "media-f1", cx));
        assert!(!on_screen(window, "thread", cx), "la conversación sigue debajo de Multimedia");
        assert_eq!(label(window, "media-f1", cx), "plano.pdf");
        click(window, "channel-tab-more", cx);
        wait_until(cx, "llega lo anterior", |cx| on_screen(window, "media-f2", cx));
        assert!(on_screen(window, "media-f1", cx) && !on_screen(window, "channel-tab-more", cx));

        click(window, "conversation-tab-links", cx);
        wait_until(cx, "llegan los enlaces", |cx| on_screen(window, "link-0", cx));
        assert_eq!(label(window, "link-0", cx), "el runbook");
        click(window, "link-0", cx);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/runbook"));

        click(window, "conversation-tab-media", cx);
        wait_until(cx, "Multimedia otra vez", |cx| on_screen(window, "media-f1", cx));
        click(window, "comms-channel-a", cx);
        wait_until(cx, "otro canal, en su conversación", |cx| label(window, "conversation-title", cx) == "#Producto" && on_screen(window, "thread", cx));
        click(window, "conversation-tab-media", cx);
        wait_until(cx, "el vacío de este canal", |cx| on_screen(window, "channel-tab-empty", cx));
        assert!(!on_screen(window, "media-f1", cx), "las fotos de un canal bajo el nombre de otro");
        stop(cx);
    }

    /// Buscar en el canal pregunta al servidor y enseña los aciertos con lo
    /// más reciente arriba en vez de la conversación; en Multimedia la
    /// búsqueda viaja con la pestaña; y cambiar de pestaña la vacía.
    ///
    /// Roturas que atrapa: buscar sin mandar lo buscado; filtrar sólo lo
    /// cargado; los aciertos en el orden del hilo; la conversación debajo
    /// de los aciertos; arrastrar la búsqueda a otra pestaña.
    #[gpui_kit::test]
    fn searching_the_channel_asks_the_server(cx: &mut TestAppContext) {
        let server = server();
        let hit = |id: &str, body: &str, at: &str| {
            json!({ "id": id, "spaceId": "g", "authorUserId": "u2", "authorName": "Luis", "kind": "user", "body": body, "createdAt": at, "updatedAt": at })
        };
        server.mount(
            get("/api/v1/task-spaces/g/chat")
                .and(query_param("q", "factura"))
                .respond_with(ok(json!([hit("m1", "la factura vieja", "2026-01-04T18:00:00Z"), hit("m2", "la factura nueva", "2026-01-05T18:00:00Z")]))),
        );
        server.mount(get("/api/v1/task-spaces/g/chat/media").and(query_param("q", "plano")).respond_with(ok(json!({ "items": [media("f1", "plano.png", "application/pdf")] }))));
        server.mount(get("/api/v1/task-spaces/g/chat/media").respond_with(ok(json!({ "items": [] }))));
        server.install(cx);
        let window = open(cx);

        type_in(window, "channel-search-input", "factura", cx);
        wait_until(cx, "llegan los aciertos", |cx| on_screen(window, "hit-m1", cx) && on_screen(window, "hit-m2", cx));
        assert!(!on_screen(window, "thread", cx), "la conversación sigue debajo de los aciertos");
        assert!(super_top(window, "hit-m2", cx) < super_top(window, "hit-m1", cx), "lo más reciente no va arriba");
        assert_eq!(label(window, "hit-m2", cx), "la factura nueva");

        click(window, "conversation-tab-media", cx);
        wait_until(cx, "Multimedia sin búsqueda", |cx| on_screen(window, "channel-tab-empty", cx));
        assert_eq!(label(window, "channel-tab-empty", cx), tr_es("chat:mediaEmpty"), "la búsqueda pasó a otra pestaña");
        type_in(window, "channel-search-input", "plano", cx);
        wait_until(cx, "lo buscado en Multimedia", |cx| on_screen(window, "media-f1", cx));
        stop(cx);
    }

    fn super_top(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> f32 {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            f32::from(window.find(id).bounds().origin.y)
        })
        .unwrap()
    }

    fn tr_es(key: &str) -> String {
        cac_core::text(cac_core::Locale::Es, key)
    }

    fn recording(id: &str, status: &str) -> Value {
        json!({ "id": id, "spaceId": "g", "startedBy": "u1", "startedByName": "Ana", "status": status, "startedAt": "2026-01-05T18:00:00Z", "durationMs": 65000, "finalContentType": "video/mp4", "hasScreen": true })
    }

    /// Con grabación en el servidor sale la pestaña; lista lo grabado, lo
    /// abre fuera con un pase corto y borra preguntando, sin quitar la fila
    /// si el servidor no la borró.
    ///
    /// Roturas que atrapa: Grabaciones escondida en un servidor que graba;
    /// ofrecer ver una que aún se procesa; abrir la grabación con el token
    /// de acceso o sin pase; borrar sin preguntar; quitar la fila cuando el
    /// borrado falló.
    #[gpui_kit::test]
    fn recordings_are_listed_watched_and_deleted(cx: &mut TestAppContext) {
        let server = server();
        server.mount(get("/api/v1/task-spaces/g/recordings/policy").respond_with(ok(json!({ "enabled": true, "active": null }))));
        server.mount(get("/api/v1/task-spaces/g/recordings").respond_with(ok(json!([recording("r1", "ready"), recording("r2", "finalizing")]))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/auth/url-ticket")).respond_with(ok(json!({ "ticket": "pase", "expiresAt": "2026-01-05T18:30:00Z" }))));
        server.mount(Mock::given(method("DELETE")).and(path("/api/v1/recordings/r1")).respond_with(ResponseTemplate::new(500)).up_to_n_times(1));
        server.mount(Mock::given(method("DELETE")).and(path("/api/v1/recordings/r1")).respond_with(ok(json!(null))));
        server.install(cx);
        let window = open(cx);

        wait_until(cx, "sale Grabaciones", |cx| on_screen(window, "conversation-tab-recordings", cx));
        click(window, "conversation-tab-recordings", cx);
        wait_until(cx, "llega lo grabado", |cx| on_screen(window, "recording-r1", cx) && on_screen(window, "recording-r2", cx));
        assert_eq!(label(window, "recording-r1", cx), "Lista · 1:05 · de Ana");
        assert!(!on_screen(window, "recording-watch-r2", cx), "se ofrece ver una que aún se procesa");
        assert!(!on_screen(window, "channel-search", cx), "se ofrece buscar en Grabaciones");

        click(window, "recording-watch-r1", cx);
        wait_until(cx, "se abre fuera", |cx| cx.opened_url().is_some());
        let url = cx.opened_url().unwrap();
        assert!(url.ends_with("/api/v1/recordings/r1/media?token=pase"), "{url}");

        click(window, "recording-delete-r1", cx);
        assert!(cx.has_pending_prompt(), "borró sin preguntar");
        cx.simulate_prompt_answer("Borrar");
        wait_until(cx, "se dice que no se pudo", |cx| on_screen(window, "channel-tabs-notice", cx));
        assert!(on_screen(window, "recording-r1", cx), "quitó la fila de una grabación que sigue ahí");
        click(window, "recording-delete-r1", cx);
        cx.simulate_prompt_answer("Borrar");
        wait_until(cx, "se va la fila", |cx| !on_screen(window, "recording-r1", cx));
        assert_eq!(server.requests().iter().filter(|request| request.method.as_str() == "DELETE").count(), 2);
        stop(cx);
    }

    /// Un enlace a una grabación pulsado en el canal lleva a Grabaciones con
    /// ésa marcada y a la vista, aunque esté al fondo de la lista; y una que
    /// no está en el canal lo dice en vez de abrir la lista sin más.
    ///
    /// Roturas que atrapa: un enlace que no cambia de pestaña; marcar otra o
    /// ninguna; dejarla fuera de la vista al fondo de la lista; abrir la
    /// lista callada cuando la grabación no está en ella.
    #[gpui_kit::test]
    fn a_recording_link_opens_its_tab_pointing_at_it(cx: &mut TestAppContext) {
        let server = server();
        let id = |at: usize| format!("{at:08x}-0000-4000-8000-000000000000");
        let rows: Vec<Value> = (0..30).map(|at| recording(&id(at), "ready")).collect();
        server.mount(get("/api/v1/task-spaces/g/recordings/policy").respond_with(ok(json!({ "enabled": true, "active": null }))));
        server.mount(get("/api/v1/task-spaces/g/recordings").respond_with(ok(Value::Array(rows))));
        // El servidor la pone en este canal, pero ya no está en su lista.
        for at in [29, 99] {
            server.mount(get(&format!("/api/v1/recordings/{}", id(at))).respond_with(ok(recording(&id(at), "ready"))));
        }
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "sale Grabaciones", |cx| on_screen(window, "conversation-tab-recordings", cx));

        let follow = |href: String, cx: &mut TestAppContext| {
            cx.update_window(window, |root, window, cx| {
                let root = root.downcast::<gpui_kit::base::Root>().unwrap();
                let workspace = root.read(cx).view().clone().downcast::<Workspace>().unwrap();
                let chat = workspace.read(cx).chat.clone();
                chat.update(cx, |chat, cx| chat.follow_link(&href, window, cx));
            })
            .unwrap()
        };
        let last = format!("recording-{}", id(29));
        follow(format!("cac:recording/{}", id(29)), cx);
        wait_until(cx, "Grabaciones, con ésa marcada", |cx| element(window, &last, cx).is_some_and(|row| row.selected() == Some(true)));
        assert!(element(window, &last, cx).unwrap().visible(), "la marcada se quedó fuera de la vista");
        assert_eq!(element(window, &format!("recording-{}", id(0)), cx).unwrap().selected(), Some(false));

        follow(format!("cac:recording/{}", id(99)), cx);
        wait_until(cx, "dice que no está", |cx| label(window, "channel-tabs-notice", cx) == tr_es("recordings:notInChannel"));
        assert_eq!(element(window, &last, cx).unwrap().selected(), Some(false));
        stop(cx);
    }

    /// Un enlace pulsado antes de saber si el canal graba espera a saberlo,
    /// en vez de decir que la grabación no está.
    ///
    /// Roturas que atrapa: contestar «no está» mientras la respuesta no ha
    /// llegado; olvidar el enlace y no abrir Grabaciones al llegar.
    #[gpui_kit::test]
    fn a_recording_link_clicked_early_waits_for_the_channel_to_answer(cx: &mut TestAppContext) {
        let server = server();
        let id = "0f3c1a2b-4d5e-4f70-8192-a3b4c5d6e7f8";
        let gate = crate::testing::Gate::default();
        server.mount(get("/api/v1/task-spaces/g/recordings/policy").respond_with(ok(json!({ "enabled": false, "active": null }))));
        server.mount(get("/api/v1/task-spaces/a/recordings/policy").respond_with(gate.hold(ok(json!({ "enabled": true, "active": null })))));
        server.mount(get("/api/v1/task-spaces/a/recordings").respond_with(ok(json!([recorded_in(id, "a")]))));
        server.mount(get(&format!("/api/v1/recordings/{id}")).respond_with(ok(recorded_in(id, "a"))));
        server.install(cx);
        let window = open(cx);
        click(window, "comms-channel-a", cx);
        wait_until(cx, "se pregunta si el canal graba", |_| gate.held() == 1);

        cx.update_window(window, |root, window, cx| {
            let root = root.downcast::<gpui_kit::base::Root>().unwrap();
            let workspace = root.read(cx).view().clone().downcast::<Workspace>().unwrap();
            let chat = workspace.read(cx).chat.clone();
            chat.update(cx, |chat, cx| chat.follow_link(&format!("cac:recording/{id}"), window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!on_screen(window, "channel-tabs-notice", cx), "dijo que no está sin saberlo");

        gate.open();
        let row = format!("recording-{id}");
        wait_until(cx, "Grabaciones, con ésa marcada", |cx| element(window, &row, cx).is_some_and(|row| row.selected() == Some(true)));
        assert!(!on_screen(window, "channel-tabs-notice", cx));
        stop(cx);
    }

    fn recorded_in(id: &str, space: &str) -> Value {
        let mut row = recording(id, "ready");
        row["spaceId"] = json!(space);
        row
    }

    fn follow(window: AnyWindowHandle, href: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |root, window, cx| {
            let root = root.downcast::<gpui_kit::base::Root>().unwrap();
            let workspace = root.read(cx).view().clone().downcast::<Workspace>().unwrap();
            let chat = workspace.read(cx).chat.clone();
            chat.update(cx, |chat, cx| chat.follow_link(href, window, cx));
        })
        .unwrap()
    }

    fn selected(cx: &mut TestAppContext) -> Option<cac_core::Conversation> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).comms.selected.clone())
    }

    /// Un enlace a una grabación de otro canal lleva a ese canal, a su
    /// pestaña y con ella marcada; uno a un canal que no está en la lista
    /// lo dice y no se mueve; una que ya no está, también; y quien cambia de
    /// conversación mientras se pregunta dónde está no es llevado a ningún
    /// sitio.
    ///
    /// Roturas que atrapa: buscarla sólo en el canal que se mira (decía «no
    /// está»); cambiar de canal y no abrir Grabaciones, o abrirla sin
    /// marcar la grabación; perder el enlace al cambiar de canal; callar
    /// ante un canal que no sale o una grabación borrada; seguir una
    /// respuesta tardía cuando ya se está en otra conversación.
    #[gpui_kit::test]
    fn a_recording_link_from_another_channel_goes_there(cx: &mut TestAppContext) {
        let server = server();
        let (in_a, hidden, late) = ("0f3c1a2b-4d5e-4f70-8192-a3b4c5d6e7f8", "1f3c1a2b-4d5e-4f70-8192-a3b4c5d6e7f8", "2f3c1a2b-4d5e-4f70-8192-a3b4c5d6e7f8");
        let gate = crate::testing::Gate::default();
        for space in ["g", "a"] {
            server.mount(get(&format!("/api/v1/task-spaces/{space}/recordings/policy")).respond_with(ok(json!({ "enabled": true, "active": null }))));
        }
        server.mount(get("/api/v1/task-spaces/g/recordings").respond_with(ok(json!([recording("r-g", "ready")]))));
        server.mount(get("/api/v1/task-spaces/a/recordings").respond_with(ok(json!([recorded_in(in_a, "a"), recorded_in(late, "a")]))));
        server.mount(get(&format!("/api/v1/recordings/{in_a}")).respond_with(ok(recorded_in(in_a, "a"))));
        server.mount(get(&format!("/api/v1/recordings/{hidden}")).respond_with(ok(recorded_in(hidden, "otro"))));
        server.mount(get(&format!("/api/v1/recordings/{late}")).respond_with(gate.hold(ok(recorded_in(late, "a")))));
        server.install(cx);
        let window = open(cx);
        let channel = |space: &str| Some(cac_core::Conversation::Channel { space_id: space.into() });
        assert_eq!(selected(cx), channel("g"));

        follow(window, &format!("cac:recording/{in_a}"), cx);
        let row = format!("recording-{in_a}");
        wait_until(cx, "en Producto, con ésa marcada", |cx| element(window, &row, cx).is_some_and(|row| row.selected() == Some(true)));
        assert_eq!(selected(cx), channel("a"));
        assert!(!on_screen(window, "channel-tabs-notice", cx), "dijo algo al llegar");

        click(window, "comms-channel-g", cx);
        wait_until(cx, "de vuelta en General", |cx| on_screen(window, "conversation-tab-thread", cx) && !on_screen(window, &row, cx));
        follow(window, &format!("cac:recording/{hidden}"), cx);
        wait_until(cx, "dice que su canal no sale", |cx| label(window, "channel-tabs-notice", cx) == tr_es("recordings:channelHidden"));
        assert_eq!(selected(cx), channel("g"));

        follow(window, &format!("cac:recording/{late}"), cx);
        wait_until(cx, "se pregunta dónde está", |_| gate.held() == 1);
        click(window, "comms-channel-a", cx);
        wait_until(cx, "en Producto, sabiendo que graba", |cx| selected(cx) == channel("a") && on_screen(window, "conversation-tab-recordings", cx));
        gate.open();
        // Lo que se pide después, y contesta 404, llega cuando la tardía ya
        // se atendió: es la marca de que ya pasó lo que fuera a pasar.
        wait_until(cx, "llega la respuesta", |_| gate.held() == 0);
        follow(window, "cac:recording/3f3c1a2b-4d5e-4f70-8192-a3b4c5d6e7f8", cx);
        wait_until(cx, "dice que ya no está", |cx| label(window, "channel-tabs-notice", cx) == tr_es("recordings:notFound"));
        assert_eq!(element(window, "conversation-tab-thread", cx).unwrap().selected(), Some(true), "una respuesta tardía abrió Grabaciones en la conversación de después");
        stop(cx);
    }

    fn element(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> Option<gpui_kit::test::ElementSnapshot> {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id)
        })
        .unwrap()
    }

    /// «Llamar con invitados» crea la llamada con el título y la duración
    /// elegidos, deja el enlace web en el portapapeles, la lista entre las
    /// abiertas y la cierra preguntando.
    ///
    /// Roturas que atrapa: crear sin canal o con otra duración; el enlace a
    /// la app en vez de a la web, o con el token fuera del `#`; no copiar
    /// nada; la llamada nueva fuera de la lista; cerrar sin preguntar; una
    /// llamada cerrada que sigue en la lista.
    #[gpui_kit::test]
    fn a_guest_call_is_created_and_its_web_link_copied(cx: &mut TestAppContext) {
        let server = server();
        let invite = json!({ "id": "i1", "orgId": "o1", "spaceId": "g", "title": "Revisión", "expiresAt": "2026-01-12T18:00:00Z", "link": "tok", "occupants": [] });
        server.mount(get("/api/v1/call-invites").respond_with(ok(json!([]))));
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/call-invites"))
                .and(body_json(json!({ "orgId": "o1", "spaceId": "g", "title": "Revisión", "ttlHours": 168 })))
                .respond_with(ok(invite)),
        );
        server.mount(Mock::given(method("DELETE")).and(path("/api/v1/call-invites/i1")).respond_with(ok(json!(null))));
        server.install(cx);
        let window = open(cx);

        click(window, "conversation-guest-call", cx);
        wait_until(cx, "sale el diálogo, sin llamadas abiertas", |cx| on_screen(window, "guest-call-none", cx));
        type_in(window, "guest-call-title", "Revisión", cx);
        click(window, "guest-call-ttl-168", cx);
        click(window, "guest-call-create", cx);
        wait_until(cx, "la llamada nueva sale en la lista", |cx| on_screen(window, "guest-call-i1", cx));
        let copied = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        assert!(copied.ends_with("/app/join#tok"), "{copied}");
        assert!(on_screen(window, "guest-call-copied", cx));

        click(window, "guest-call-close-i1", cx);
        assert!(cx.has_pending_prompt(), "cerró sin preguntar");
        cx.simulate_prompt_answer("Cerrar llamada");
        wait_until(cx, "se va de la lista", |cx| !on_screen(window, "guest-call-i1", cx));
        assert_eq!(server.hits("/api/v1/call-invites/i1"), 1);
        stop(cx);
    }
}
