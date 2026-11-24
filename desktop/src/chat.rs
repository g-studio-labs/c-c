//! La conversación que se tiene delante: sus mensajes, de un canal o de un
//! directo, para leer.
//!
//! Qué mensajes hay, de quién es cada uno, cuál va pegado al anterior, qué
//! día abre, qué imágenes y ficheros lleva y a dónde va cada enlace lo
//! decide el core (`cac_core::chat`). Aquí se pinta con las medidas de la
//! app de siempre (`ChannelView.tsx`, `DMThread.tsx`, `ReplyQuote.tsx`,
//! `Reactions.tsx`, `ReadTicks.tsx`) y se lleva lo único propio de una
//! pantalla: por dónde va la lectura.
//!
//! Un hilo se mira desde abajo (`use-anclaje-de-scroll.ts`):
//!
//! 1. Si se está abajo, se sigue abajo: llega un mensaje y se ve.
//! 2. Si se lee hacia arriba, no se mueve nada, y sale «Mensajes nuevos».
//! 3. Al anteponer una página vieja, se sigue mirando lo mismo.
//! 4. Una imagen que crece al cargar no empuja lo que se lee.
//!
//! Las cuatro las hace la lista de GPUI alineada al fondo y siguiendo la
//! cola; lo que hay que hacer aquí es decirle **qué cambió y dónde**
//! (`thread_splice`) en vez de rehacerla, y volver a medir la fila que
//! crece.
//!
//! Escribir, editar, retirar y reaccionar todavía no están: la caja de
//! escribir está pintada en su sitio y lo dice, y las reacciones se ven
//! pero no se pulsan.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use cac_core::{BodyFile, Conversation, MessageBody, ThreadLine, ThreadLink, ThreadMessage};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::composer::Composer;
use crate::message_actions::Editing;
use crate::picture_viewer::PictureViewer;
use crate::files;
use crate::lang::{error_text, locale, tr};
use crate::theme::Leading as _;
use crate::thumbs::{Thumb, Thumbs};

/// Cuánto se maqueta de más por encima y por debajo de lo que se ve: lo que
/// está a punto de asomar ya está medido.
const OVERDRAW: f32 = 480.;
/// A qué distancia del principio se pide la página anterior: el
/// `UMBRAL_DE_CARGA` de `use-anclaje-de-scroll.ts`.
const LOAD_OLDER_WITHIN: f32 = 40.;
/// Cuánto se queda dicho un aviso («ese mensaje está más arriba»).
const NOTICE_FOR: Duration = Duration::from_secs(4);
/// Cuánto se queda resaltado el mensaje al que lleva una cita: el
/// `reply-flash` de `lib/quote.ts`.
const FLASH_FOR: Duration = Duration::from_millis(1600);

/// Lo que se pinta de un mensaje, ya partido, y de qué texto salió.
struct Body {
    /// El cuerpo del que salió: si el mensaje se edita, se vuelve a partir.
    /// Se compara el texto y no `updatedAt`: también cambia con una
    /// reacción, y no hace falta volver a partir por eso.
    source: String,
    parts: MessageBody,
    /// El texto, ya preparado para el pintor de markdown.
    painted: SharedString,
}

/// La conversación, para leer.
pub struct ChatView {
    list: ListState,
    /// De qué conversación es lo que la lista tiene.
    of: Option<Conversation>,
    /// Las filas que la lista tiene, en su orden: con ellas se le dice qué
    /// cambió en vez de rehacerla.
    shown: Vec<String>,
    lines: Vec<ThreadLine>,
    pub(crate) messages: Vec<ThreadMessage>,
    /// Cada mensaje ya partido en texto, imágenes y ficheros, por su id: un
    /// refresco no vuelve a partir los que no cambiaron.
    bodies: HashMap<String, Rc<Body>>,
    /// Qué mensajes enseñan cada imagen, por su ruta: al llegar, se vuelven
    /// a medir sus filas sin recorrer la conversación.
    pictured_in: HashMap<String, Vec<String>>,
    /// Qué versión de la conversación está puesta en la lista.
    revision: u64,
    /// Cuántas llegadas se han visto: una más, leyendo arriba, es «nuevos».
    arrivals: u64,
    /// Llegó algo mientras se leía hacia arriba.
    new_below: bool,
    /// Lo último que se le dijo al core de si se ve el final: lo que llega
    /// leyendo arriba no se marca leído hasta bajar.
    at_tail: bool,
    /// Lo que se le dijo al core que hay delante la última vez.
    front: Option<(bool, Option<Conversation>)>,
    /// Las imágenes de los mensajes, por su ruta.
    pub(crate) thumbs: Thumbs,
    /// El visor de imágenes, si hay una abierta.
    pub(crate) viewer: Option<PictureViewer>,
    /// El mensaje al que se le está eligiendo una reacción.
    pub(crate) picking: Option<String>,
    /// El mensaje que se está corrigiendo.
    pub(crate) editing: Option<Editing>,
    /// Algo que decir un momento, y cuál de los avisos es (para que el que
    /// lo quita no quite otro posterior).
    notice: Option<(u64, String)>,
    notices: u64,
    /// El mensaje al que acaba de llevar una cita.
    flash: Option<String>,
    /// La caja de escribir, con un borrador por conversación.
    pub(crate) composer: Composer,
    _on_snapshot: Subscription,
}

impl ChatView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(0, ListAlignment::Bottom, px(OVERDRAW));
        list.set_follow_mode(FollowMode::Tail);
        let view = cx.entity().downgrade();
        // Al deslizar: si se volvió abajo ya no hay «nuevos» que anunciar, y
        // si se llegó arriba se pide la página anterior. Después del gesto,
        // no dentro: la lista aún no sabe por dónde quedó.
        list.set_scroll_handler(move |_, _, cx| {
            let view = view.clone();
            cx.defer(move |cx| {
                let _ = view.update(cx, |this, cx| this.scrolled(cx));
            });
        });
        let snapshot = Backend::snapshot(cx);
        let on_snapshot = cx.observe(&snapshot, |this, _, cx| this.sync(cx));
        let mut this = Self {
            list,
            of: None,
            shown: Vec::new(),
            lines: Vec::new(),
            messages: Vec::new(),
            bodies: HashMap::new(),
            pictured_in: HashMap::new(),
            revision: 0,
            arrivals: 0,
            new_below: false,
            at_tail: true,
            front: None,
            thumbs: Thumbs::default(),
            viewer: None,
            picking: None,
            editing: None,
            notice: None,
            notices: 0,
            flash: None,
            composer: Composer::new(window, cx),
            _on_snapshot: on_snapshot,
        };
        this.sync(cx);
        this
    }

    /// Pone la lista al día con lo que dice el core.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let (front, revision) = {
            let snapshot = Backend::snapshot(cx).read(cx);
            ((snapshot.comms.on_screen, snapshot.comms.selected.clone()), snapshot.thread_revision)
        };
        // Lo que hay delante cambió: el core decide qué traer.
        if self.front.as_ref() != Some(&front) {
            self.front = Some(front);
            Backend::run(cx, |core| async move { core.chat().follow().await });
        }
        if revision == self.revision {
            return;
        }
        self.revision = revision;
        let (thread, me) = {
            let snapshot = Backend::snapshot(cx).read(cx);
            (snapshot.thread.clone(), snapshot.session.as_ref().map(|session| session.id.clone()))
        };
        let lines = cac_core::thread_lines(&thread, me.as_deref(), locale(cx), &chrono::Local::now());
        let ids: Vec<String> = lines.iter().map(|line| line.id.clone()).collect();
        if self.of != thread.conversation {
            // Otra conversación: lista nueva, abajo del todo, y nada de la
            // anterior.
            self.of = thread.conversation.clone();
            self.list.reset(ids.len());
            self.list.set_follow_mode(FollowMode::Tail);
            self.new_below = false;
            // El core abre cada conversación por el final.
            self.at_tail = true;
            self.bodies.clear();
            self.pictured_in.clear();
            self.viewer = None;
            self.picking = None;
            self.editing = None;
            self.thumbs.clear();
            self.notice = None;
            self.flash = None;
        } else {
            let splice = cac_core::thread_splice(&self.shown, &ids);
            // Las filas que siguen ahí y dicen otra cosa (se editó, le
            // pusieron una reacción, dejó de ir pegada a la de arriba) se
            // vuelven a medir, sin mover por dónde va la lectura.
            let (head, tail) = match splice {
                Some(splice) => (splice.start as usize, self.shown.len() - (splice.start + splice.removed) as usize),
                None => (self.shown.len(), 0),
            };
            let changed = |old: usize, new: usize| self.lines[old] != lines[new] || self.messages[old].body != thread.messages[new].body;
            let mut again: Vec<usize> = (0..head).filter(|index| changed(*index, *index)).collect();
            again.extend((0..tail).map(|back| (self.shown.len() - 1 - back, ids.len() - 1 - back)).filter(|(old, new)| changed(*old, *new)).map(|(_, new)| new));
            if let Some(splice) = splice {
                let start = splice.start as usize;
                self.list.splice(start..start + splice.removed as usize, splice.inserted as usize);
            }
            for index in again {
                self.list.remeasure_items(index..index + 1);
            }
            // Llegó algo y no se está abajo: se avisa, y no se mueve nada.
            if thread.arrivals > self.arrivals && !self.list.is_following_tail() {
                self.new_below = true;
            }
        }
        self.arrivals = thread.arrivals;
        self.shown = ids;
        self.lines = lines;
        self.messages = thread.messages;
        // Lo partido de mensajes que ya no están no se guarda, ni sus
        // imágenes.
        let kept: std::collections::HashSet<&str> = self.shown.iter().map(String::as_str).collect();
        self.bodies.retain(|id, _| kept.contains(id.as_str()));
        self.pictured_in.retain(|_, ids| {
            ids.retain(|id| kept.contains(id.as_str()));
            !ids.is_empty()
        });
        let pictured = &self.pictured_in;
        self.thumbs.retain(|path| pictured.contains_key(path));
        cx.notify();
    }

    /// Se deslizó la lista.
    fn scrolled(&mut self, cx: &mut Context<Self>) {
        if self.list.is_following_tail() && self.new_below {
            self.new_below = false;
            cx.notify();
        }
        self.tell_tail(cx);
        let top = self.list.logical_scroll_top();
        if top.item_ix == 0 && top.offset_in_item <= px(LOAD_OLDER_WITHIN) && !self.shown.is_empty() {
            self.load_older(cx);
        }
    }

    /// Pide la página anterior. Si se puede y si ya se está pidiendo lo
    /// sabe el core.
    fn load_older(&mut self, cx: &mut Context<Self>) {
        Backend::run(cx, |core| async move { core.chat().load_older().await });
    }

    /// Baja al final: lo que hace «Mensajes nuevos».
    pub(crate) fn to_the_end(&mut self, cx: &mut Context<Self>) {
        self.list.set_follow_mode(FollowMode::Tail);
        self.new_below = false;
        self.tell_tail(cx);
        cx.notify();
    }

    /// Le dice al core si se ve el final, cuando cambia.
    fn tell_tail(&mut self, cx: &mut Context<Self>) {
        let at_tail = self.list.is_following_tail();
        if at_tail != self.at_tail {
            self.at_tail = at_tail;
            Backend::run(cx, move |core| async move { core.chat().at_tail(at_tail).await });
        }
    }

    /// Dice algo un momento.
    pub(crate) fn say(&mut self, text: String, cx: &mut Context<Self>) {
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

    /// Lleva la vista al mensaje citado y lo resalta un momento
    /// (`jumpToMessage`). Si no está cargado —el hilo va por páginas— lo
    /// dice en vez de no hacer nada.
    pub(crate) fn jump_to(&mut self, message_id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.shown.iter().position(|id| id == message_id) else {
            self.say(tr(cx, "chat:quoteNotLoaded"), cx);
            return;
        };
        self.list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
        self.flash = Some(message_id.to_string());
        cx.notify();
        let flashed = message_id.to_string();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLASH_FOR).await;
            this.update(cx, |this, cx| {
                if this.flash.as_deref() == Some(flashed.as_str()) {
                    this.flash = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sigue un enlace de un mensaje. A dónde va lo dice el core, que abre
    /// lo de dentro; aquí queda lo que es de esta máquina: el navegador y
    /// los ficheros.
    pub(crate) fn follow_link(&mut self, href: &str, window: &mut Window, cx: &mut Context<Self>) {
        let href = href.to_string();
        let answer = Backend::ask(cx, move |core| async move { core.chat().follow_link(&href).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(link) = answer.await else { return };
            this.update_in(cx, |this, window, cx| match link {
                ThreadLink::Web { url } => cx.open_url(&url),
                ThreadLink::File { path } => {
                    let name = this.file_name(&path);
                    this.open_file(path, name, window, cx);
                }
                ThreadLink::NotHere => this.say(tr(cx, "common:misc.notInThisApp"), cx),
                ThreadLink::Unknown => this.say(tr(cx, "common:last.openLinkFailed"), cx),
                ThreadLink::Task { .. } | ThreadLink::Doc | ThreadLink::Inert => {}
            })
            .ok();
        })
        .detach();
    }

    /// Cómo se llama el fichero de esa ruta: como lo nombra el mensaje que
    /// lo cita.
    fn file_name(&self, path: &str) -> String {
        self.bodies
            .values()
            .flat_map(|body| body.parts.pictures.iter().chain(&body.parts.files))
            .find(|file| file.path == path && !file.name.trim().is_empty())
            .map(|file| file.name.clone())
            .unwrap_or_else(|| path.rsplit('/').find(|part| !part.is_empty() && *part != "raw").unwrap_or("file").to_string())
    }

    /// Baja un fichero de un mensaje y lo abre con el programa del sistema
    /// que lo entiende. Su dirección pide sesión: lo baja el core.
    pub(crate) fn open_file(&mut self, path: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let answer = Backend::ask(cx, move |core| async move {
            let bytes = core.chat().file_bytes(&path).await.map_err(Some)?;
            files::write_for_opening(&std::env::temp_dir(), &name, &bytes).map_err(|_| None::<cac_core::ApiError>)
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, _, cx| match result {
                Ok(path) => files::open_with_system(&path, cx),
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

    /// Guarda una copia de un fichero de un mensaje donde se elija.
    pub(crate) fn save_file(&mut self, path: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let home = std::env::home_dir().unwrap_or_else(std::env::temp_dir);
        let chosen = cx.prompt_for_new_path(&home, Some(&cac_core::safe_file_name(&name)));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(target))) = chosen.await else { return };
            let Ok(answer) = this.update(cx, |_, cx| {
                Backend::ask(cx, move |core| async move {
                    let bytes = core.chat().file_bytes(&path).await.map_err(Some)?;
                    files::write_copy(&target, &bytes).map_err(|_| None::<cac_core::ApiError>)
                })
            }) else {
                return;
            };
            let Ok(Err(error)) = answer.await else { return };
            this.update(cx, |this, cx| {
                let text = match error {
                    Some(error) => error_text(cx, &error, "common:misc.errSave"),
                    None => tr(cx, "common:misc.errSave"),
                };
                this.say(text, cx);
            })
            .ok();
        })
        .detach();
    }

    /// El mensaje de esa fila, partido. Se parte una vez y se guarda hasta
    /// que su texto cambie.
    fn body_of(&mut self, index: usize, cx: &App) -> Rc<Body> {
        let message = &self.messages[index];
        if let Some(body) = self.bodies.get(&message.id).filter(|body| body.source == message.body) {
            return body.clone();
        }
        let parts = cac_core::message_body(&message.body, Backend::core(cx).base_url());
        let painted = SharedString::from(cac_core::prose_for_painting(&parts.text));
        for picture in &parts.pictures {
            let ids = self.pictured_in.entry(picture.path.clone()).or_default();
            if !ids.contains(&message.id) {
                ids.push(message.id.clone());
            }
        }
        let body = Rc::new(Body { source: message.body.clone(), parts, painted });
        self.bodies.insert(message.id.clone(), body.clone());
        body
    }

    /// Una imagen llegó (o ya se sabe que no llega): la fila que la enseña
    /// cambia de alto, y se vuelve a medir sin mover lo que se lee.
    fn picture_landed(&mut self, path: &str, _: &mut Context<Self>) {
        let Some(ids) = self.pictured_in.get(path) else { return };
        for id in ids {
            if let Some(index) = self.shown.iter().position(|shown| shown == id) {
                self.list.remeasure_items(index..index + 1);
            }
        }
    }

    /// La cita encima de una respuesta (`ReplyQuote.tsx`): de quién era y
    /// qué decía, en dos líneas. Pulsarla lleva al mensaje citado.
    fn render_quote(&self, line: &ThreadLine, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let quote = line.quote.clone()?;
        let theme = cx.theme();
        let (muted_ink, muted, primary) = (theme.muted_foreground, theme.muted, theme.primary);
        let target = quote.id.clone();
        let said = if quote.gone { tr(cx, "chat:quoteGone") } else { quote.text.clone() };
        Some(
            div()
                .id(SharedString::from(format!("thread-quote-{}", line.id)))
                .mb_1()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .rounded(px(4.))
                .border_l_2()
                .border_color(primary.opacity(0.6))
                .bg(muted.opacity(0.6))
                .px_2()
                .py_1()
                .leading_xs()
                .hover(move |style| style.bg(muted))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.jump_to(&target, cx)))
                .aria_label(said.clone())
                .map(|this| {
                    if quote.gone {
                        this.child(div().italic().text_color(muted_ink).child(said))
                    } else {
                        this.child(div().font_weight(FontWeight::MEDIUM).text_color(primary).child(quote.author.clone()))
                            // `line-clamp-2 break-words`.
                            .child(div().min_w_0().line_clamp(2).text_color(muted_ink).child(said))
                    }
                })
                .test_support(),
        )
    }

    /// Una imagen de un mensaje, en miniatura (`max-h-40 max-w-64`, como en
    /// una tarea). Pulsarla la abre; encima, al pasar el ratón, guardarla.
    fn render_picture(&self, picture: &BodyFile, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, background) = (theme.muted_foreground, theme.border, theme.background);
        let (open, save) = (picture.clone(), picture.clone());
        let name = if picture.name.trim().is_empty() { tr(cx, "chat:picture") } else { picture.name.clone() };
        let save_name = name.clone();
        let group = SharedString::from(format!("thread-picture-{}", picture.path));
        let body = match self.thumbs.get(&picture.path) {
            Some(Thumb::Ready(image)) => {
                let natural = image.size(0);
                let (width, height) = cac_core::thumbnail_size(natural.width.0.max(0) as u32, natural.height.0.max(0) as u32);
                div()
                    .id(SharedString::from(format!("thread-thumb-{}", picture.path)))
                    .child(img(image.clone()).w(px(width as f32)).h(px(height as f32)).object_fit(ObjectFit::Contain))
                    .test_support()
                    .into_any_element()
            }
            // Mientras llega, o si no se pudo pintar: su hueco con su
            // nombre, que la abre igual.
            other => {
                let state = if matches!(other, Some(Thumb::Failed)) { "missing" } else { "wait" };
                div()
                    .id(SharedString::from(format!("thread-thumb-{state}-{}", picture.path)))
                    .w(px(160.))
                    .h(px(96.))
                    .p_2()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(Icon::new(Lucide::Image).text_color(muted))
                    .child(div().w_full().overflow_hidden().text_ellipsis().whitespace_nowrap().leading_xs().text_color(muted).child(name.clone()))
                    .test_support()
                    .into_any_element()
            }
        };
        div()
            .group(group.clone())
            .relative()
            .flex_none()
            .child(
                div()
                    .id(SharedString::from(format!("thread-picture-{}", picture.path)))
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .overflow_hidden()
                    .cursor_pointer()
                    // Se abre en el visor, que pasa por las demás del chat.
                    .on_click(cx.listener(move |this, _, window, cx| this.open_viewer(open.path.clone(), window, cx)))
                    .aria_label(name)
                    .child(body)
                    .test_support(),
            )
            .child(
                div().absolute().top_1().right_1().flex().rounded_sm().bg(background.opacity(0.8)).opacity(0.).group_hover(group, |style| style.opacity(1.)).child(
                    Button::new(SharedString::from(format!("thread-picture-save-{}", picture.path)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::Download))
                        .tooltip(tr(cx, "work:task.saveFile"))
                        .on_click(cx.listener(move |this, _, window, cx| this.save_file(save.path.clone(), save_name.clone(), window, cx))),
                ),
            )
    }

    /// Un fichero de un mensaje que no es una imagen: el clip y su nombre,
    /// que lo abre (la fila de «lo demás» de una tarea).
    fn render_file(&self, file: &BodyFile, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, primary) = (theme.muted_foreground, theme.primary);
        let (open, save) = (file.clone(), file.clone());
        let group = SharedString::from(format!("thread-file-row-{}", file.path));
        div()
            .group(group.clone())
            .flex()
            .items_center()
            .gap_2()
            .leading_xs()
            .child(Icon::new(Lucide::Paperclip).size_3().flex_none().text_color(muted))
            .child(
                div()
                    .id(SharedString::from(format!("thread-file-{}", file.path)))
                    .min_w_0()
                    .truncate()
                    .text_color(primary)
                    .underline()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.open_file(open.path.clone(), open.name.clone(), window, cx)))
                    .aria_label(file.name.clone())
                    .child(file.name.clone())
                    .test_support(),
            )
            .child(
                div().flex().flex_none().opacity(0.).group_hover(group, |style| style.opacity(1.)).child(
                    Button::new(SharedString::from(format!("thread-file-save-{}", file.path)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::Download))
                        .tooltip(tr(cx, "work:task.saveFile"))
                        .on_click(cx.listener(move |this, _, window, cx| this.save_file(save.path.clone(), save.name.clone(), window, cx))),
                ),
            )
    }

    /// Las reacciones de un mensaje (`Reactions.tsx`): una pastilla por
    /// emoji con cuántos y, al pasar el ratón, quiénes. La propia va
    /// marcada. El emoji va como imagen (`assets::emoji`): con la fuente,
    /// el ❤️ y el 😂 salen grises.
    fn render_reactions(&self, line: &ThreadLine, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if line.reactions.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let (foreground, muted_ink, muted, border, primary) = (theme.foreground, theme.muted_foreground, theme.muted, theme.border, theme.primary);
        // Pulsar una pastilla pone o quita la reacción de quien mira.
        let can_write = Backend::snapshot(cx).read(cx).thread.can_write && !line.system;
        let pills = line.reactions.iter().enumerate().map(|(index, reaction)| {
            let (id, emoji, on) = (line.id.clone(), reaction.emoji.clone(), !reaction.mine);
            let label = cac_core::text_with(locale(cx), "chat:reactionCount", &[("emoji", &reaction.emoji), ("count", &reaction.count.to_string())]);
            let names = reaction.names.join(", ");
            div()
                .id(SharedString::from(format!("thread-reaction-{}-{index}", line.id)))
                .h_6()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .rounded_full()
                .border_1()
                .px_2()
                .leading_xs()
                .map(|this| {
                    if reaction.mine {
                        this.border_color(primary.opacity(0.5)).bg(primary.opacity(0.1)).text_color(foreground)
                    } else {
                        this.border_color(border).bg(muted.opacity(0.4)).text_color(muted_ink)
                    }
                })
                .when(!names.is_empty(), |this| this.tooltip(move |window, cx| Tooltip::new(names.clone()).build(window, cx)))
                .when(can_write, |this| this.cursor_pointer().on_click(cx.listener(move |this, _, window, cx| this.react(id.clone(), emoji.clone(), on, window, cx))))
                .aria_label(label)
                .aria_selected(reaction.mine)
                .child(crate::assets::emoji(&reaction.emoji, px(16.)))
                .child(reaction.count.to_string())
                .test_support()
        });
        Some(div().mt_1().flex().flex_wrap().items_center().gap_1().when(line.mine, |this| this.justify_end()).children(pills))
    }

    /// Una fila de la conversación (`Message` en `ChannelView.tsx`, `DMLine`
    /// en `DMThread.tsx`): la raya del día si lo abre, la cabecera si abre
    /// turno, y el mensaje. Lo propio, a la derecha.
    fn render_line(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(line) = self.lines.get(index).cloned() else { return div().into_any_element() };
        #[cfg(test)]
        tests::ROWS_BUILT.with(|built| built.borrow_mut().insert(line.id.clone()));
        let body = self.body_of(index, cx);
        // Sus imágenes se piden al asomar la fila: por el core, con la sesión.
        for picture in &body.parts.pictures {
            let path = picture.path.clone();
            self.thumbs.request(
                picture.path.clone(),
                move |core| async move { core.chat().picture(&path).await },
                |this: &mut Self| &mut this.thumbs,
                Self::picture_landed,
                window,
                cx,
            );
        }
        let theme = cx.theme();
        let (foreground, muted_ink, muted, border, primary) = (theme.foreground, theme.muted_foreground, theme.muted, theme.border, theme.primary);
        let id = line.id.clone();

        // La raya del día: su nombre entre dos líneas finas.
        let day = line.day.clone().map(|day| {
            div()
                .id(SharedString::from(format!("thread-day-{id}")))
                .mb_3()
                .flex()
                .items_center()
                .gap_3()
                .leading_xs()
                .text_color(muted_ink)
                .aria_label(day.clone())
                .child(div().h(px(1.)).flex_1().bg(border))
                .child(day)
                .child(div().h(px(1.)).flex_1().bg(border))
                .test_support()
        });

        // `mb-0.5 flex items-center gap-2 text-xs text-muted-foreground`, y
        // al revés en lo propio.
        let header = (!line.grouped).then(|| {
            let who = if line.system {
                // Un chip en vez de un nombre: es de cac, y con la letra de
                // una persona pasaría por una.
                let hint = tr(cx, "chat:systemTitle");
                div()
                    .id(SharedString::from(format!("thread-system-{id}")))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(muted_ink.opacity(0.3))
                    .px_1()
                    .font_weight(FontWeight::MEDIUM)
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                    .aria_label(line.author.clone())
                    .child(line.author.to_uppercase())
                    .test_support()
            } else {
                div()
                    .id(SharedString::from(format!("thread-author-{id}")))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if line.mine { primary } else { foreground })
                    .aria_label(line.author.clone())
                    .child(line.author.clone())
                    .test_support()
            };
            div()
                .mb_0p5()
                .flex()
                .items_center()
                .gap_2()
                .leading_xs()
                .text_color(muted_ink)
                .when(line.mine, |this| this.flex_row_reverse())
                .child(who)
                .when(line.via_agent, |this| {
                    let said = tr(cx, "common:feedback.viaAgent");
                    this.child(
                        div()
                            .id(SharedString::from(format!("thread-via-{id}")))
                            .flex()
                            .items_center()
                            .gap_1()
                            .aria_label(said.clone())
                            .child(Icon::new(Lucide::Bot).size_3())
                            .child(said)
                            .test_support(),
                    )
                })
                .child(line.time.clone())
                .when(line.edited, |this| {
                    let said = tr(cx, "work:task.edited");
                    this.child(div().id(SharedString::from(format!("thread-edited-{id}"))).italic().aria_label(said.clone()).child(said).test_support())
                })
        });

        let follow = cx.entity().downgrade();
        let text = (!body.parts.text.is_empty()).then(|| {
            crate::theme::painted_prose_with_links(
                SharedString::from(format!("thread-body-{id}")),
                body.painted.clone(),
                move |url, window, cx| {
                    let _ = follow.update(cx, |this, cx| this.follow_link(url, window, cx));
                },
                cx,
            )
        });
        let pictures: Vec<AnyElement> = body.parts.pictures.iter().map(|picture| self.render_picture(picture, cx).into_any_element()).collect();
        let files: Vec<AnyElement> = body.parts.files.iter().map(|file| self.render_file(file, cx).into_any_element()).collect();
        let ticks = line.ticks.map(|read| {
            // Las palomitas de lo propio, abajo a la derecha (`ReadTicks`).
            let label = tr(cx, if read { "chat:readByOther" } else { "chat:sentNotRead" });
            div().mt_0p5().flex().justify_end().child(
                div()
                    .id(SharedString::from(format!("thread-ticks-{id}")))
                    .aria_label(label)
                    .child(Icon::new(if read { Lucide::CheckCheck } else { Lucide::Check }).size_3p5().text_color(if read { primary } else { muted_ink }))
                    .test_support(),
            )
        });
        let flashed = self.flash.as_deref() == Some(id.as_str());
        // Las acciones, en la esquina del mensaje al pasar por él (el hueco
        // de `pr-12` es para esto).
        let can_write = Backend::snapshot(cx).read(cx).thread.can_write;
        let group = SharedString::from(format!("thread-row-{id}"));
        let actions = self.render_actions(&line, group.clone(), can_write, cx);
        let editor = self.render_editor(&line, cx);
        let editing = editor.is_some();
        // `relative rounded-md py-1 pl-2 pr-12` (`pr-7` en una línea de
        // cac); lo propio, `max-w-[75%] rounded-tr-sm border
        // border-primary/25 bg-primary/10`; lo demás, su espejo con borde neutro.
        let bubble = div()
            .id(SharedString::from(format!("thread-bubble-{id}")))
            .relative()
            .min_w_0()
            .rounded(px(8.))
            .py_1()
            .pl_2()
            .map(|this| if line.system { this.pr_7() } else { this.pr_12() })
            .when(line.grouped, |this| this.mt_0p5())
            .map(|this| {
                if line.mine {
                    this.max_w(relative(0.75)).rounded_tr(px(6.)).border_1().border_color(primary.opacity(0.25)).bg(primary.opacity(if flashed { 0.25 } else { 0.1 }))
                } else {
                    // Lo de los demás, también en su burbuja (como en
                    // cualquier app de mensajes), con el borde neutro.
                    this.max_w(relative(0.75))
                        .rounded_tl(px(6.))
                        .border_1()
                        .border_color(border)
                        .bg(if flashed { primary.opacity(0.15) } else { muted.opacity(0.35) })
                        .when(!flashed, |this| this.hover(move |style| style.bg(muted.opacity(0.55))))
                }
            })
            .aria_label(body.parts.text.clone())
            .aria_selected(line.mine)
            .children(self.render_quote(&line, cx))
            // Corrigiéndolo, la caja va en el sitio de lo que dice.
            .children(editor)
            .when(!editing, |this| {
                this.children(text)
                    .when(!pictures.is_empty(), |this| this.child(div().mt_1().flex().flex_wrap().gap_2().children(pictures)))
                    .when(!files.is_empty(), |this| this.child(div().mt_1().flex().flex_col().gap_1().children(files)))
            })
            .children(ticks)
            .children(actions)
            .test_support();

        // `space-y-3`: cada fila deja doce píxeles con la de arriba, y la
        // primera, con el borde (el `p-3` de la caja).
        // Las acciones salen al pasar por la fila entera, no sólo por la
        // burbuja: la barra flota sobre su borde de arriba, y subir el ratón
        // hasta ella no la tiene que apagar.
        div()
            .id(SharedString::from(format!("thread-line-{id}")))
            .group(group)
            .w_full()
            .px_3()
            .pt_3()
            .flex()
            .flex_col()
            .aria_label(line.author.clone())
            .children(day)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when(line.mine, |this| this.items_end())
                    .children(header)
                    .child(bubble)
                    .children(self.render_reactions(&line, cx))
                    .children(self.render_picker(&line, cx)),
            )
            .test_support()
            .into_any_element()
    }

    /// Mientras llega la primera página (`MessagesSkeleton`): cinco líneas
    /// de anchos distintos.
    fn render_loading(cx: &App) -> AnyElement {
        let bar = cx.theme().muted;
        let widths = [0.66, 0.5, 0.75, 0.4, 0.6];
        div()
            .id("thread-loading")
            .size_full()
            .p_3()
            .pt_5()
            .flex()
            .flex_col()
            .gap_4()
            .aria_label(tr(cx, "chat:loading"))
            .children(widths.map(|width| {
                div().flex().items_start().gap_2().child(div().size_7().flex_none().rounded_full().bg(bar)).child(
                    div().flex_1().flex().flex_col().gap_1p5().child(div().h_3().w_24().rounded(px(4.)).bg(bar)).child(div().h_3().w(relative(width)).rounded(px(4.)).bg(bar)),
                )
            }))
            .test_support()
            .into_any_element()
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (conversation, loading, failed, loading_older, older_failed) = {
            let snapshot = Backend::snapshot(cx).read(cx);
            let thread = &snapshot.thread;
            (thread.conversation.clone(), thread.loading, thread.failed, thread.loading_older, thread.older_failed)
        };
        let theme = cx.theme();
        let (muted_ink, primary, on_primary, border, background, danger) =
            (theme.muted_foreground, theme.primary, theme.primary_foreground, theme.border, theme.background, theme.danger);
        let direct = matches!(conversation, Some(Conversation::Direct { .. }));

        let body: AnyElement = if self.shown.is_empty() && loading {
            Self::render_loading(cx)
        } else if failed {
            div()
                .id("thread-failed")
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .leading_sm()
                .text_color(muted_ink)
                .aria_label(tr(cx, "chat:couldNotLoad"))
                .child(tr(cx, "chat:couldNotLoad"))
                .child(Button::new("thread-retry").outline().small().label(tr(cx, "chat:retry")).on_click(|_, _, cx| Backend::run(cx, |core| async move { core.chat().retry().await })))
                .test_support()
                .into_any_element()
        } else if self.shown.is_empty() {
            // `pt-6 text-center text-xs text-muted-foreground`.
            let said = tr(cx, if direct { "common:misc.dmEmpty" } else { "chat:emptyTeamOnly" });
            div().id("thread-empty").size_full().p_3().pt_9().text_center().leading_xs().text_color(muted_ink).aria_label(said.clone()).child(said).test_support().into_any_element()
        } else {
            list(self.list.clone(), cx.processor(|this, index: usize, window, cx| this.render_line(index, window, cx))).size_full().pb_3().into_any_element()
        };

        div()
            .id("thread")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(background)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(div().id("thread-messages").flex_1().min_h_0().child(body).test_support())
                    // La página de antes, llegando: arriba, flotando, para
                    // no empujar lo que se lee.
                    .when(loading_older, |this| {
                        this.child(div().id("thread-older-loading").absolute().top_2().left_0().right_0().flex().justify_center().child(Spinner::new().color(muted_ink)).test_support())
                    })
                    .when(older_failed, |this| {
                        let said = tr(cx, "chat:couldNotLoadOlder");
                        this.child(
                            div().absolute().top_2().left_0().right_0().flex().justify_center().child(
                                div()
                                    .id("thread-older-failed")
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(border)
                                    .bg(background)
                                    .px_3()
                                    .py_1()
                                    .leading_xs()
                                    .text_color(danger)
                                    .shadow_md()
                                    .aria_label(said.clone())
                                    .child(said)
                                    .child(Button::new("thread-older-retry").ghost().xsmall().label(tr(cx, "chat:retry")).on_click(cx.listener(|this, _, _, cx| this.load_older(cx))))
                                    .test_support(),
                            ),
                        )
                    })
                    .children(self.notice.clone().map(|(_, said)| {
                        div().absolute().top_2().left_0().right_0().flex().justify_center().child(
                            div()
                                .id("thread-notice")
                                .rounded(px(8.))
                                .border_1()
                                .border_color(border)
                                .bg(background)
                                .px_3()
                                .py_1()
                                .leading_xs()
                                .shadow_md()
                                .aria_label(said.clone())
                                .child(said)
                                .test_support(),
                        )
                    }))
                    // «Mensajes nuevos», sólo cuando llega algo mientras se
                    // lee hacia arriba. Flota para no empujar lo que se lee
                    // (`absolute bottom-3 left-1/2 rounded-full border
                    // bg-primary px-3 py-1 text-xs font-medium shadow-lg`).
                    .when(self.new_below, |this| {
                        let said = tr(cx, "chat:newMessages");
                        this.child(
                            div().absolute().bottom_3().left_0().right_0().flex().justify_center().child(
                                div()
                                    .id("thread-new-messages")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(border)
                                    .bg(primary)
                                    .px_3()
                                    .py_1()
                                    .leading_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(on_primary)
                                    .shadow_lg()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.to_the_end(cx)))
                                    .aria_label(said.clone())
                                    .child(said)
                                    .child(Icon::new(IconName::ChevronDown).size_3())
                                    .test_support(),
                            ),
                        )
                    }),
            )
            // Se está hablando en este canal y tú no estás: encima de lo que
            // se escribe, como `VozEnCurso` en `ChannelView.tsx`.
            .children(match &conversation {
                Some(Conversation::Channel { space_id }) => crate::voice::in_call_banner(space_id, cx),
                _ => None,
            })
            .child(self.render_composer(window, cx))
            .children(self.render_viewer(window, cx))
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::ChatView;
    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use cac_core::Conversation;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{point, px, size, AnyWindowHandle, AppContext, Bounds, Entity, Pixels, Point, ScrollDelta, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, Request, ResponseTemplate};

    type History = Arc<Mutex<Vec<Value>>>;

    const CHAT_A: &str = "/api/v1/task-spaces/a/chat";
    const READ_A: &str = "/api/v1/task-spaces/a/chat/read";
    const CHAT_B: &str = "/api/v1/task-spaces/b/chat";
    const DM_C1: &str = "/api/v1/dm/c1/messages";
    const SHOT: &str = "/api/v1/task-spaces/a/chat/attachments/a1/raw";
    const NOTES: &str = "/api/v1/task-spaces/a/chat/attachments/a2/raw";

    fn get(route: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET")).and(path(route.to_string()))
    }

    fn space(id: &str, name: &str, kind: Option<&str>) -> Value {
        json!({ "id": id, "orgId": "o1", "name": name, "color": "#888", "kind": kind, "folders": [],
            "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 1, "taskCount": 1 }], "people": [] })
    }

    /// De 2020: la raya del día dice su fecha con su año, se corra cuando se
    /// corra; y a mediodía, para que sea el mismo día en cualquier zona.
    fn when(day: u32, second: u32) -> String {
        format!("2020-01-{day:02}T12:{:02}:{:02}Z", second / 60, second % 60)
    }

    fn line(id: &str, author: &str, name: &str, body: &str, day: u32, second: u32) -> Value {
        let at = when(day, second);
        json!({ "id": id, "spaceId": "a", "authorUserId": author, "authorName": name, "kind": "user", "body": body, "createdAt": at, "updatedAt": at })
    }

    fn private_line(id: &str, author: &str, name: &str, body: &str, second: u32) -> Value {
        let at = when(5, second);
        json!({ "id": id, "conversationId": "c1", "authorUserId": author, "authorName": name, "body": body, "createdAt": at, "updatedAt": at })
    }

    /// Mensajes largos y cortos, para que las filas midan distinto.
    fn numbered(range: std::ops::Range<u32>) -> Vec<Value> {
        range.map(|index| line(&format!("m{index:03}"), if index % 3 == 0 { "u1" } else { "u2" }, "Luis Mora", &format!("mensaje {index}{}", "\n\ny sigue".repeat((index % 4) as usize)), 5, index)).collect()
    }

    fn history(messages: Vec<Value>) -> History {
        Arc::new(Mutex::new(messages))
    }

    /// Sirve esa conversación como el servidor de verdad: los cincuenta
    /// últimos, o los cincuenta anteriores a `before`.
    fn mount_history(server: &FakeServer, route: &str, history: &History, delay: Duration) {
        mount_history_held(server, route, history, delay, None);
    }

    /// Retiene una respuesta hasta que la prueba la suelta. Con un plazo de
    /// llegada, en un CI lento la respuesta se adelantaba a lo que la prueba
    /// quería mirar antes, o la prueba no llegaba a mirarlo a tiempo. El servidor tiene un solo hilo: mientras retiene,
    /// no contesta nada más.
    #[derive(Clone, Default)]
    struct Gate(Arc<(Mutex<bool>, Condvar)>);

    impl Gate {
        fn open(&self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }

        fn wait(&self) {
            let (open, opened) = &*self.0;
            // Una prueba que se rompe antes de soltarla no se queda colgada.
            let _ = opened.wait_timeout_while(open.lock().unwrap(), Duration::from_secs(10), |open| !*open).unwrap();
        }
    }

    /// Como `mount_history`, con la página anterior retenida por `older`.
    fn mount_history_held(server: &FakeServer, route: &str, history: &History, delay: Duration, older: Option<Gate>) {
        let history = history.clone();
        server.mount(get(route).respond_with(move |request: &Request| {
            let before = request.url.query_pairs().find(|(name, _)| name == "before").map(|(_, value)| value.to_string());
            if let (Some(gate), Some(_)) = (&older, &before) {
                gate.wait();
            }
            let all = history.lock().unwrap().clone();
            let visible: Vec<Value> = all.into_iter().filter(|message| before.as_deref().is_none_or(|before| message["createdAt"].as_str().unwrap() < before)).collect();
            ok(json!(visible[visible.len().saturating_sub(50)..].to_vec())).set_delay(delay)
        }));
    }

    /// Una organización con la sala general, «Producto» (`a`) y «Diseño»
    /// (`b`), y un directo con Luis (`c1`).
    fn server() -> FakeServer {
        server_as("member")
    }

    /// Lo mismo, con ese papel en la organización.
    fn server_as(role: &str) -> FakeServer {
        let server = FakeServer::start();
        let org = json!({ "id": "o1", "name": "G Studio", "slug": "o1", "role": role, "memberCount": 3, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto", None), space("b", "Diseño", None), space("g", "General", Some("general"))]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/unread").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([
            { "conversationId": "c1", "orgId": "o1", "userId": "u2", "username": "luis", "unread": 0, "lastMessageAt": "2020-01-05T12:00:00Z",
              "lastBody": "¿vienes?", "lastAuthorId": "u2", "otherReadAt": "2020-01-05T12:00:01Z" },
        ]))));
        server.mount(get("/api/v1/users/search").respond_with(ok(json!([
            { "id": "u1", "username": "ana", "name": "Ana Ruiz" },
            { "id": "u2", "username": "luis", "name": "Luis Mora", "lastSeenAt": chrono::Utc::now().to_rfc3339() },
            { "id": "u3", "username": "bea", "name": "Bea Soto" },
        ]))));
        server.mount(Mock::given(method("POST")).respond_with(ok(json!(null))));
        server
    }

    fn open(server: &FakeServer, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ChatView>) {
        server.install(cx);
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(760.)) })),
                ..WindowOptions::default()
            };
            let (window, workspace) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window");
            (window, workspace.read(cx).chat.clone())
        })
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

    /// Dónde está pintado algo. `None` si no está a la vista.
    fn bounds(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> Option<Bounds<Pixels>> {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).map(|found| found.bounds())
        })
        .unwrap()
    }

    fn top(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> f32 {
        f32::from(bounds(window, id, cx).unwrap_or_else(|| panic!("{id} no está a la vista")).top())
    }

    /// Entra a comunicación con esa conversación delante.
    fn enter(window: AnyWindowHandle, conversation: Conversation, cx: &mut TestAppContext) {
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "rail-comms", cx);
        wait_until(cx, "sale la lista", |cx| on_screen(window, "comms-channel-a", cx) && on_screen(window, "comms-direct-c1", cx));
        cx.update(|cx| assert!(Backend::core(cx).comms().select(conversation)));
    }

    fn channel(id: &str) -> Conversation {
        Conversation::Channel { space_id: id.into() }
    }

    /// Relee la conversación, como cuando llega un mensaje.
    fn refresh(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.chat().refresh().await }));
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    thread_local! {
        /// Los mensajes cuya fila se ha construido desde que se vació: lo que
        /// la lista maqueta, se vea o no.
        pub(super) static ROWS_BUILT: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
    }

    /// Un mensaje de más arriba que se corrige y crece no mueve lo que se
    /// lee, ni hace saltar el gesto que llega hasta él.
    ///
    /// Rotura que atrapa: no volver a medir una fila que cambió fuera de la
    /// vista; la lista se queda con su alto de antes y, al subir hasta ella,
    /// lo que se lee da un salto.
    #[gpui_kit::test]
    fn an_edit_above_what_is_read_does_not_make_the_next_gesture_jump(cx: &mut TestAppContext) {
        let server = server();
        let talk = history(numbered(0..40));
        mount_history(&server, CHAT_A, &talk, Duration::ZERO);
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se abre por el final", |cx| on_screen(window, "thread-line-m039", cx));
        scroll_up(window, 40., cx);
        // La fila justo encima de la primera que se ve: medida, y fuera.
        let (row, at) = reading(window, cx);
        let index: usize = row[1..].parse().unwrap();
        let above = format!("m{:03}", index - 2);
        assert!(!on_screen(window, &format!("thread-line-{above}"), cx), "{above} se ve");

        {
            let mut talk = talk.lock().unwrap();
            let edited = talk.iter_mut().find(|message| message["id"] == above.as_str()).unwrap();
            edited["body"] = json!("corregido, y ahora mucho más largo\n\ncon un párrafo\n\ny otro\n\ny otro más\n\ny el último");
            edited["updatedAt"] = json!(when(5, 59));
        }
        refresh(cx);
        wait_until(cx, "llega la corrección", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).thread.messages.iter().any(|message| message.body.starts_with("corregido"))));
        cx.run_until_parked();
        assert_eq!(top(window, &format!("thread-line-{row}"), cx), at, "corregir un mensaje de encima movió lo que se lee");
        climb_to(window, &above, "subir hasta el mensaje corregido dio un salto", cx);
        stop(cx);
    }

    /// De una conversación de dos mil mensajes sólo se construye lo que cabe
    /// en la ventana y un poco más, esté donde esté la lectura; y pintar un
    /// cuadro no cuesta más por tener dos mil que por tener cincuenta.
    ///
    /// Rotura que atrapa: maquetar la conversación entera al abrirla o al
    /// deslizar (una lista que no es virtual, o una que se rehace en cada
    /// cambio); volver a partir y preparar cada mensaje en cada pintado.
    #[gpui_kit::test]
    fn two_thousand_messages_lay_out_only_what_fits(cx: &mut TestAppContext) {
        ROWS_BUILT.with(|built| built.borrow_mut().clear());
        let server = server();
        let many: Vec<Value> = (0..2000u32)
            .map(|index| line(&format!("x{index:04}"), if index % 3 == 0 { "u1" } else { "u2" }, "Luis Mora", &format!("mensaje **{index}** con [un enlace](https://example.com/{index}){}", "\n\ny un párrafo más".repeat((index % 3) as usize)), 1 + index / 3600, index % 3600))
            .collect();
        // Toda de una vez: lo que se mide es pintarla, no traerla.
        server.mount(get(CHAT_A).respond_with(ok(json!(many))));
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se abre por el final", |cx| on_screen(window, "thread-line-x1999", cx));
        assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).thread.messages.len()), 2000);
        let built = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
            ROWS_BUILT.with(|built| built.borrow().len())
        };
        let at_the_end = built(cx);
        assert!((1..80).contains(&at_the_end), "al abrir se construyen {at_the_end} de 2000 mensajes");

        // Subiendo un buen trecho, se construye lo que va asomando, no todo.
        let started = std::time::Instant::now();
        let mut frames = 0;
        for _ in 0..30 {
            scroll_up(window, 500., cx);
            cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
            frames += 1;
        }
        let per_frame = started.elapsed() / frames;
        let after = built(cx);
        assert!(after < 400, "tras subir 15.000 px se han construido {after} de 2000 mensajes");
        // El número de verdad se mide a mano; esto sólo dice si se disparó.
        eprintln!("dos mil mensajes: {after} filas construidas, {per_frame:?} por cuadro al deslizar");
        stop(cx);
    }

    /// Desliza la conversación tantos píxeles hacia arriba (hacia lo viejo).
    fn scroll_up(window: AnyWindowHandle, by: f32, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.scroll("thread-messages", ScrollDelta::Pixels(point(px(0.), px(by))), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Sube de 40 en 40 hasta que se vea esa fila, y comprueba que cada paso
    /// mueve lo que se lee justo 40: una fila de encima con su alto viejo
    /// guardado haría que el paso que entra en ella diera un salto.
    fn climb_to(window: AnyWindowHandle, id: &str, jumped: &str, cx: &mut TestAppContext) {
        let wanted = format!("thread-line-{id}");
        for _ in 0..60 {
            if on_screen(window, &wanted, cx) {
                return;
            }
            let (row, at) = reading(window, cx);
            scroll_up(window, 40., cx);
            assert_eq!(top(window, &format!("thread-line-{row}"), cx), at + 40., "{jumped}");
        }
        panic!("no se llegó a {id}");
    }

    /// La primera fila que se ve entera, y dónde está: lo que se está leyendo.
    fn reading(window: AnyWindowHandle, cx: &mut TestAppContext) -> (String, f32) {
        let area = bounds(window, "thread-messages", cx).expect("la conversación");
        let ids: Vec<String> = cx.update(|cx| Backend::snapshot(cx).read(cx).thread.messages.iter().map(|message| message.id.clone()).collect());
        for id in ids {
            if let Some(row) = bounds(window, &format!("thread-line-{id}"), cx) {
                if row.top() >= area.top() && row.bottom() <= area.bottom() {
                    return (id, f32::from(row.top()));
                }
            }
        }
        panic!("no hay ninguna fila entera a la vista");
    }

    /// Cuánto falta entre el final de esa fila y el de la zona de mensajes.
    fn gap_below(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> f32 {
        let area = bounds(window, "thread-messages", cx).expect("la conversación");
        let row = bounds(window, &format!("thread-line-{id}"), cx).unwrap_or_else(|| panic!("{id} no está a la vista"));
        f32::from(area.bottom() - row.bottom())
    }

    /// La conversación de un canal, pintada: quién dice qué, lo propio a la
    /// derecha, los turnos pegados, el día, lo de cac, las citas y las
    /// reacciones; y lo que todavía no se hace, en su sitio y apagado.
    ///
    /// Roturas que atrapa: el aviso de «todavía no se leen» en vez de los
    /// mensajes; lo propio a la izquierda, o lo de otro a la derecha; una
    /// línea de cac pintada como propia de quien la provocó; un turno
    /// pegado con cabecera, o uno de agente sin ella; un día sin su raya;
    /// una respuesta sin su cita; reacciones que no se ven; mandar algo con
    /// la caja vacía; la caja prometiendo `#`, que aquí todavía no cita
    /// nada; la cabecera sin quién anda por aquí; un mensaje corregido que
    /// sigue diciendo lo de antes.
    #[gpui_kit::test]
    fn a_channel_reads_as_people_talk(cx: &mut TestAppContext) {
        let server = server();
        let mut mine = line("m2", "u1", "Ana Ruiz", "yo me encargo", 5, 2);
        mine["updatedAt"] = json!(when(5, 9));
        let mut agent = line("m5", "u2", "Luis Mora", "lo dejé apuntado en la tarea", 5, 5);
        agent["via"] = json!("mcp");
        let mut system = line("m6", "u1", "Ana Ruiz", "Grabación lista", 5, 6);
        system["kind"] = json!("system");
        let mut reply = line("m7", "u3", "Bea Soto", "sí, **hoy**", 5, 7);
        reply["replyTo"] = json!({ "id": "m1", "authorName": "Luis Mora", "body": "¿sale **hoy**?", "gone": false });
        let mut first = line("m1", "u2", "Luis Mora", "¿sale **hoy**?", 4, 1);
        first["reactions"] = json!([{ "emoji": "👍", "count": 2, "mine": true, "names": ["Ana Ruiz", "Bea Soto"] }]);
        let talk = history(vec![first, mine, line("m3", "u2", "Luis Mora", "vale", 5, 3), line("m4", "u2", "Luis Mora", "gracias", 5, 4), agent, system, reply]);
        mount_history(&server, CHAT_A, &talk, Duration::ZERO);
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pintan los mensajes", |cx| on_screen(window, "thread-line-m7", cx));

        assert_eq!(label(window, "conversation-title", cx), "#Producto");
        assert!(!on_screen(window, "conversation-not-yet", cx));
        assert_eq!(label(window, "thread-bubble-m1", cx), "¿sale **hoy**?");
        assert_eq!(label(window, "thread-author-m1", cx), "Luis Mora");
        assert_eq!(label(window, "thread-author-m2", cx), "Tú");
        assert!(on_screen(window, "thread-edited-m2", cx) && !on_screen(window, "thread-edited-m1", cx));
        // Lo propio, a la derecha; lo de los demás, desde la izquierda.
        let area = bounds(window, "thread-messages", cx).unwrap();
        let (theirs, own, cac) = (bounds(window, "thread-bubble-m1", cx).unwrap(), bounds(window, "thread-bubble-m2", cx).unwrap(), bounds(window, "thread-bubble-m6", cx).unwrap());
        assert!(own.right() > area.right() - px(20.) && own.left() > area.left() + px(100.), "lo propio no está a la derecha: {own:?}");
        assert!(theirs.left() < area.left() + px(20.), "lo de otro no empieza a la izquierda: {theirs:?}");
        // Lo que puso cac por algo que hizo Ana no es de Ana.
        assert!(cac.left() < area.left() + px(20.), "una línea de cac se pintó como propia: {cac:?}");
        assert_eq!(label(window, "thread-system-m6", cx), "cac");
        assert!(!on_screen(window, "thread-author-m6", cx));
        // Los turnos seguidos de Luis van pegados; el de su agente, no.
        assert!(on_screen(window, "thread-author-m3", cx) && !on_screen(window, "thread-author-m4", cx), "un turno pegado lleva cabecera");
        assert!(on_screen(window, "thread-author-m5", cx) && on_screen(window, "thread-via-m5", cx), "lo del agente se pegó a lo de su dueño");
        assert!(!on_screen(window, "thread-via-m4", cx));
        // Dos días, dos rayas.
        assert_eq!(label(window, "thread-day-m1", cx), "4 de enero de 2020");
        assert_eq!(label(window, "thread-day-m2", cx), "5 de enero de 2020");
        assert!(!on_screen(window, "thread-day-m3", cx));
        assert_eq!(label(window, "thread-quote-m7", cx), "¿sale hoy?");
        assert_eq!(label(window, "thread-reaction-m1-0", cx), "👍 2");
        // En orden, de arriba abajo.
        let order: Vec<f32> = ["m1", "m2", "m3", "m4", "m5", "m6", "m7"].iter().map(|id| top(window, &format!("thread-line-{id}"), cx)).collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "los mensajes no están en su orden: {order:?}");

        // La caja vacía no manda nada, y abrir una pestaña sólo lee.
        assert_eq!(label(window, "thread-composer", cx), "Escríbele al equipo — con @ nombras a alguien");
        let before = server.requests().len();
        click(window, "thread-send", cx);
        click(window, "conversation-tab-media", cx);
        cx.run_until_parked();
        assert!(server.requests().iter().skip(before).all(|request| request.method.as_str() == "GET"), "algo de lo que no está escribió en el servidor");
        for tab in ["conversation-tab-thread", "conversation-tab-media", "conversation-tab-links"] {
            assert!(on_screen(window, tab, cx), "falta {tab}");
        }
        click(window, "conversation-tab-thread", cx);
        wait_until(cx, "la cabecera dice quién anda por aquí", |cx| label(window, "conversation-around", cx) == "Luis Mora");

        // Un mensaje que se corrige y una reacción que llega se ven sin salir.
        {
            let mut talk = talk.lock().unwrap();
            talk[2]["body"] = json!("vale, **mañana**");
            talk[2]["updatedAt"] = json!(when(5, 30));
            talk[3]["reactions"] = json!([{ "emoji": "🎉", "count": 1, "mine": false, "names": ["Bea Soto"] }]);
        }
        refresh(cx);
        wait_until(cx, "se ve la corrección", |cx| label(window, "thread-bubble-m3", cx) == "vale, **mañana**");
        assert!(on_screen(window, "thread-edited-m3", cx));
        assert_eq!(label(window, "thread-reaction-m4-0", cx), "🎉 1");
        stop(cx);
    }

    /// Un directo se lee igual, con las palomitas de lo propio y sin lo que
    /// es de un canal.
    ///
    /// Roturas que atrapa: un directo sin palomitas, o con ellas en lo que
    /// escribió el otro; leído lo que el otro no ha leído; las pestañas de
    /// un canal en una conversación privada; la promesa de la caja de un
    /// canal («nombras a alguien») en un directo; un directo vacío en
    /// blanco.
    #[gpui_kit::test]
    fn a_direct_has_ticks_and_nothing_of_a_channel(cx: &mut TestAppContext) {
        let server = server();
        // El otro leyó hasta el segundo 1.
        let talk = history(vec![private_line("d1", "u1", "Ana Ruiz", "¿quedamos?", 1), private_line("d2", "u2", "Luis Mora", "¿vienes?", 2), private_line("d3", "u1", "Ana Ruiz", "voy", 3)]);
        mount_history(&server, DM_C1, &talk, Duration::ZERO);
        let (window, _) = open(&server, cx);
        enter(window, Conversation::Direct { conversation_id: "c1".into() }, cx);
        wait_until(cx, "se pintan los mensajes", |cx| on_screen(window, "thread-line-d3", cx));

        assert_eq!(label(window, "conversation-title", cx), "luis");
        assert!(on_screen(window, "conversation-private", cx) && !on_screen(window, "conversation-tabs", cx) && !on_screen(window, "conversation-around", cx));
        assert_eq!(label(window, "thread-ticks-d1", cx), "Leído");
        assert_eq!(label(window, "thread-ticks-d3", cx), "Enviado");
        assert!(!on_screen(window, "thread-ticks-d2", cx), "palomitas en lo que escribió el otro");
        assert_eq!(label(window, "thread-composer", cx), "Escríbele a luis");

        *talk.lock().unwrap() = Vec::new();
        refresh(cx);
        wait_until(cx, "un directo vacío lo dice", |cx| on_screen(window, "thread-empty", cx));
        assert_eq!(label(window, "thread-empty", cx), "Todavía no hay nada. Esto sólo lo leéis vosotros dos.");
        stop(cx);
    }

    /// Al elegir otra conversación no queda en pantalla nada de la
    /// anterior, ni mientras llega la nueva; y una que no se puede leer lo
    /// dice y se puede reintentar.
    ///
    /// Roturas que atrapa: los mensajes del canal anterior bajo la cabecera
    /// del nuevo; una pantalla en blanco, o cargando para siempre, cuando
    /// el servidor falla; un «Reintentar» que no reintenta; una
    /// conversación que se abre por donde se estaba leyendo la anterior.
    #[gpui_kit::test]
    fn another_conversation_replaces_what_was_on_screen(cx: &mut TestAppContext) {
        let server = server();
        let mut in_a = numbered(0..40);
        in_a.push(line("a1", "u2", "Luis Mora", "esto es de Producto", 5, 50));
        mount_history(&server, CHAT_A, &history(in_a), Duration::ZERO);
        server.mount(get(CHAT_B).respond_with(ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": "boom", "message": "x" })).set_delay(Duration::from_millis(300))).up_to_n_times(1));
        let mut in_b: Vec<Value> = (0..40).map(|index| line(&format!("b{index:03}"), "u3", "Bea Soto", "relleno de Diseño\n\ncon dos párrafos", 5, index)).collect();
        in_b.push(line("b1", "u3", "Bea Soto", "esto es de Diseño", 5, 50));
        mount_history(&server, CHAT_B, &history(in_b), Duration::ZERO);
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta Producto", |cx| on_screen(window, "thread-line-a1", cx));
        // Se deja Producto a medio leer, lejos del final.
        scroll_up(window, 900., cx);
        assert!(!on_screen(window, "thread-line-a1", cx));
        scroll_up(window, -150., cx);

        click(window, "comms-channel-b", cx);
        // En el mismo cuadro en que cambia la cabecera, lo de Producto ya no está.
        wait_until(cx, "la cabecera es la de Diseño", |cx| label(window, "conversation-title", cx) == "#Diseño");
        assert!(!on_screen(window, "thread-line-a1", cx), "lo de Producto sigue bajo la cabecera de Diseño");
        wait_until(cx, "se dice que no se pudo leer", |cx| on_screen(window, "thread-failed", cx));
        assert!(!on_screen(window, "thread-line-a1", cx) && !on_screen(window, "thread-loading", cx));

        click(window, "thread-retry", cx);
        wait_until(cx, "al reintentar llega", |cx| on_screen(window, "thread-line-b1", cx));
        assert!(!on_screen(window, "thread-failed", cx) && !on_screen(window, "thread-line-a1", cx));
        // Y se abre por su final, no por donde se leía la otra.
        assert!(gap_below(window, "b1", cx).abs() <= 13., "la conversación nueva no se abrió por el final: {}", gap_below(window, "b1", cx));
        stop(cx);
    }

    /// Un hilo se mira desde abajo: se abre por el final, lo que llega
    /// estando abajo se ve, y leyendo hacia arriba no se mueve nada y sale
    /// «Mensajes nuevos», que lleva al final.
    ///
    /// Roturas que atrapa: abrir por el principio; un mensaje que llega y
    /// queda debajo del borde; ser arrancado del sitio por el que se lee
    /// cada vez que alguien escribe; no enterarse de que llegó algo; un
    /// aviso de «nuevos» por un mensaje que se retiró, o estando ya abajo;
    /// un aviso que no lleva al final o que no se quita; marcar leído lo que
    /// llegó mientras se leía arriba, o no marcarlo al bajar.
    #[gpui_kit::test]
    fn a_thread_is_read_from_the_bottom_and_reading_above_is_not_disturbed(cx: &mut TestAppContext) {
        let server = server();
        let talk = history(numbered(0..40));
        mount_history(&server, CHAT_A, &talk, Duration::ZERO);
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se abre por el final", |cx| on_screen(window, "thread-line-m039", cx));
        assert!(gap_below(window, "m039", cx).abs() <= 13., "el último no está al fondo: {}", gap_below(window, "m039", cx));
        assert!(!on_screen(window, "thread-line-m000", cx), "se abrió por el principio");

        // Estando abajo, lo que llega se ve.
        talk.lock().unwrap().push(line("m040", "u2", "Luis Mora", "uno nuevo", 5, 40));
        refresh(cx);
        wait_until(cx, "el nuevo se ve", |cx| on_screen(window, "thread-line-m040", cx));
        assert!(gap_below(window, "m040", cx).abs() <= 13., "el nuevo quedó fuera: {}", gap_below(window, "m040", cx));
        assert!(!on_screen(window, "thread-new-messages", cx), "aviso de nuevos estando abajo");
        // Y se marcó leído: al abrir y al llegar.
        wait_until(cx, "lo que llega estando abajo se marca leído", |_| server.hits(READ_A) == 2);

        // Leyendo hacia arriba, lo que llega no mueve nada.
        scroll_up(window, 700., cx);
        let (row, at) = reading(window, cx);
        assert!(!on_screen(window, "thread-line-m040", cx), "no se subió");
        // Uno que se retira no es una novedad.
        talk.lock().unwrap().retain(|message| message["id"] != "m039");
        refresh(cx);
        wait_until(cx, "el retirado se va", |cx| cx.update(|cx| !Backend::snapshot(cx).read(cx).thread.messages.iter().any(|message| message.id == "m039")));
        assert!(!on_screen(window, "thread-new-messages", cx), "un mensaje retirado se anunció como nuevo");
        assert_eq!(top(window, &format!("thread-line-{row}"), cx), at, "retirar un mensaje de abajo movió lo que se lee");

        talk.lock().unwrap().push(line("m041", "u2", "Luis Mora", "otro más\n\nlargo\n\nde varias líneas", 5, 41));
        refresh(cx);
        wait_until(cx, "sale el aviso de nuevos", |cx| on_screen(window, "thread-new-messages", cx));
        assert_eq!(top(window, &format!("thread-line-{row}"), cx), at, "un mensaje nuevo arrancó de donde se leía");
        assert!(!on_screen(window, "thread-line-m041", cx));
        assert_eq!(label(window, "thread-new-messages", cx), "Mensajes nuevos");
        cx.run_until_parked();
        assert_eq!(server.hits(READ_A), 2, "se marcó leído lo que llegó leyendo arriba");

        click(window, "thread-new-messages", cx);
        wait_until(cx, "el aviso lleva al final", |cx| on_screen(window, "thread-line-m041", cx));
        assert!(gap_below(window, "m041", cx).abs() <= 13.);
        assert!(!on_screen(window, "thread-new-messages", cx), "el aviso no se quitó");
        wait_until(cx, "al bajar se marca leído", |_| server.hits(READ_A) == 3);
        stop(cx);
    }

    /// Al llegar arriba se trae la página anterior, y se sigue mirando lo
    /// mismo: ni un píxel de salto.
    ///
    /// Roturas que atrapa: no pedir la página anterior al subir; pedirla
    /// sin parar; que al ponerse delante empuje lo que se lee, o devuelva
    /// al final; rehacer la lista y volver abajo.
    #[gpui_kit::test]
    fn the_older_page_lands_above_without_moving_what_is_read(cx: &mut TestAppContext) {
        let server = server();
        let talk = history(numbered(0..70));
        let older = Gate::default();
        mount_history_held(&server, CHAT_A, &talk, Duration::ZERO, Some(older.clone()));
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se abre por el final", |cx| on_screen(window, "thread-line-m069", cx));
        let loaded = |cx: &mut TestAppContext| cx.update(|cx| Backend::snapshot(cx).read(cx).thread.messages.len());
        assert_eq!(loaded(cx), 50);

        // Subir un poco no pide nada: todavía no se llegó arriba.
        scroll_up(window, 300., cx);
        std::thread::sleep(Duration::from_millis(250));
        cx.run_until_parked();
        assert_eq!((server.hits(CHAT_A), loaded(cx)), (1, 50), "se pidió la página anterior sin llegar arriba");
        // Hasta arriba del todo.
        for _ in 0..40 {
            scroll_up(window, 400., cx);
            if on_screen(window, "thread-line-m020", cx) {
                break;
            }
        }
        scroll_up(window, 4000., cx);
        assert!(on_screen(window, "thread-line-m020", cx), "no se llegó arriba");
        let (row, at) = reading(window, cx);
        wait_until(cx, "se ve que llega la página anterior", |cx| on_screen(window, "thread-older-loading", cx));
        assert_eq!(top(window, &format!("thread-line-{row}"), cx), at, "pedir la página movió lo que se lee");
        older.open();
        wait_until(cx, "llega la página anterior", |cx| loaded(cx) == 70);
        assert_eq!(top(window, &format!("thread-line-{row}"), cx), at, "la página anterior empujó lo que se lee");
        assert!(!on_screen(window, "thread-line-m069", cx), "se volvió al final");
        assert!(!on_screen(window, "thread-older-loading", cx));
        // Una vez: la primera página y la anterior.
        assert_eq!(server.hits(CHAT_A), 2);
        // Y lo de antes está encima, a un gesto.
        scroll_up(window, 300., cx);
        assert!(on_screen(window, "thread-line-m019", cx), "la página anterior no quedó encima");
        stop(cx);
    }

    /// Las imágenes de un mensaje se bajan con la sesión y se pintan debajo
    /// de su texto; una que crece al llegar no empuja lo que se lee, y
    /// pulsarla la abre. Un fichero se abre igual.
    ///
    /// Roturas que atrapa: una imagen pedida sin la sesión (el servidor
    /// contesta que no existe); una imagen que no se ve; el hueco que crece
    /// al llegar la imagen y mueve lo que se lee; la fila de la imagen sin
    /// volver a medir, que hace saltar el siguiente gesto; una miniatura o
    /// un fichero que no abren nada, o que abren otro; abrirlo con su
    /// dirección, que el navegador no puede leer.
    #[gpui_kit::test]
    fn a_picture_comes_with_the_session_and_does_not_push_what_is_read(cx: &mut TestAppContext) {
        let server = server();
        let png = include_bytes!("../../core/tests/fixtures/gray_600x200.png").to_vec();
        // Sólo con la sesión, y tarde: llega cuando ya se lee más arriba.
        server.mount(get(SHOT).and(header("Authorization", "Bearer access")).respond_with(ResponseTemplate::new(200).set_body_raw(png.clone(), "image/png").set_delay(Duration::from_millis(700))));
        server.mount(get(NOTES).and(header("Authorization", "Bearer access")).respond_with(ResponseTemplate::new(200).set_body_raw(b"el acta".to_vec(), "text/plain")));
        let mut messages = numbered(0..30);
        messages.push(line("pic", "u2", "Luis Mora", &format!("así queda:\n\n![captura.png]({SHOT})\n\n[acta.txt]({NOTES})"), 5, 30));
        // Lo bastante detrás para que, al abrir por el final, la imagen
        // quede por encima de lo que se ve (pero dentro de lo que la lista
        // ya maqueta, que es cuando se pide).
        messages.extend(numbered(31..40));
        mount_history(&server, CHAT_A, &history(messages), Duration::ZERO);
        let opened = Arc::new(Mutex::new(Vec::new()));
        let seen = opened.clone();
        cx.update(|cx| cx.set_global(crate::files::Opener(Arc::new(move |path| seen.lock().unwrap().push(path.to_path_buf())))));
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se abre por el final", |cx| on_screen(window, "thread-line-m039", cx));
        assert!(!on_screen(window, "thread-line-pic", cx), "la imagen se ve al abrir: no queda encima de lo que se lee");
        // Se sube un poco, para no seguir el final.
        scroll_up(window, 40., cx);
        assert!(!on_screen(window, "thread-line-pic", cx));

        // La imagen llega con su fila fuera de la vista, encima.
        let (row, at) = reading(window, cx);
        wait_until(cx, "se pide la imagen", |_| server.hits(SHOT) == 1);
        wait_until(cx, "la imagen llega", |cx| view.read_with(cx, |view, _| matches!(view.thumbs.get(SHOT), Some(crate::thumbs::Thumb::Ready(_)))));
        cx.run_until_parked();
        assert_eq!(top(window, &format!("thread-line-{row}"), cx), at, "la imagen, al llegar, empujó lo que se lee");
        // Y su fila, aunque esté fuera de la vista, ya mide lo que mide:
        // subir hasta meterse en ella avanza justo lo que se desliza. Con
        // su alto de antes guardado, la lista contaría mal dónde cae y lo
        // que se lee daría un salto.
        climb_to(window, "pic", "subir tras llegar la imagen dio un salto", cx);

        // Un poco más, hasta verla entera: está pintada, a su tamaño de
        // miniatura.
        scroll_up(window, 200., cx);
        wait_until(cx, "la miniatura se ve", |cx| on_screen(window, &format!("thread-thumb-{SHOT}"), cx));
        let thumb = bounds(window, &format!("thread-thumb-{SHOT}"), cx).unwrap();
        // 600×200 en un hueco de 256×160: entera, sin deformar.
        assert_eq!(f32::from(thumb.size.width), 256.);
        assert!((f32::from(thumb.size.height) - 85.).abs() < 1., "la miniatura mide {:?}", thumb.size);
        // El texto no lleva la imagen: el pintor de markdown no la pide.
        assert_eq!(label(window, "thread-bubble-pic", cx), "así queda:");
        assert!(server.requests().iter().filter(|request| request.url.path() == SHOT).all(|request| request.headers.get("Authorization").is_some()), "se pidió la imagen sin la sesión");

        // Pulsarla la abre en el visor, y desde él, con el programa del
        // sistema.
        click(window, &format!("thread-picture-{SHOT}"), cx);
        wait_until(cx, "se abre el visor", |cx| on_screen(window, "picture-viewer-image", cx));
        assert!(opened.lock().unwrap().is_empty(), "la miniatura se abrió fuera de la app");
        click(window, "picture-viewer-open", cx);
        wait_until(cx, "la imagen se abre", |_| opened.lock().unwrap().len() == 1);
        click(window, "picture-viewer-close", cx);
        assert!(!on_screen(window, "picture-viewer", cx));
        click(window, &format!("thread-file-{NOTES}"), cx);
        wait_until(cx, "el fichero se abre", |_| opened.lock().unwrap().len() == 2);
        let opened = opened.lock().unwrap().clone();
        assert_eq!(opened[0].file_name().unwrap(), "captura.png");
        assert_eq!(std::fs::read(&opened[0]).unwrap(), png);
        assert_eq!(opened[1].file_name().unwrap(), "acta.txt");
        assert_eq!(std::fs::read(&opened[1]).unwrap(), b"el acta");
        for path in &opened {
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }
        stop(cx);
    }

    /// Pulsar una cita lleva al mensaje citado si está cargado, y si no lo
    /// dice; y un enlace a una tarea la abre en su página.
    ///
    /// Roturas que atrapa: una cita que no hace nada; una que no dice que
    /// lo citado está más arriba; una tarea citada en el canal que no abre
    /// nada; un enlace que no se sabe abrir y no lo dice.
    #[gpui_kit::test]
    fn a_quote_jumps_to_its_message_and_a_link_opens_its_task(cx: &mut TestAppContext) {
        let server = server();
        let mut messages = numbered(0..40);
        let mut near = line("near", "u3", "Bea Soto", "a esto", 5, 40);
        near["replyTo"] = json!({ "id": "m002", "authorName": "Luis Mora", "body": "mensaje 2", "gone": false });
        let mut far = line("far", "u3", "Bea Soto", "y a esto", 5, 41);
        far["replyTo"] = json!({ "id": "no-cargado", "authorName": "Luis Mora", "body": "de hace un mes", "gone": false });
        messages.extend([near, far]);
        mount_history(&server, CHAT_A, &history(messages), Duration::ZERO);
        let detail: Value = serde_json::from_str(include_str!("../../core/tests/fixtures/task_detail.json")).unwrap();
        server.mount(get("/api/v1/tasks/by-ref/cac-12").respond_with(ok(detail.clone())));
        server.mount(get("/api/v1/tasks/t1").respond_with(ok(detail)));
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se abre por el final", |cx| on_screen(window, "thread-line-far", cx));
        assert!(!on_screen(window, "thread-line-m002", cx));

        click(window, "thread-quote-far", cx);
        wait_until(cx, "dice que está más arriba", |cx| on_screen(window, "thread-notice", cx));
        assert_eq!(label(window, "thread-notice", cx), "Ese mensaje está más arriba: sube para cargarlo");

        click(window, "thread-quote-near", cx);
        wait_until(cx, "la cita lleva a su mensaje", |cx| on_screen(window, "thread-line-m002", cx));
        assert!(!on_screen(window, "thread-line-far", cx));

        let follow = |href: &'static str, cx: &mut TestAppContext| cx.update_window(window, |_, window, cx| view.update(cx, |view, cx| view.follow_link(href, window, cx))).unwrap();
        follow("cac:call/0f3c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8", cx);
        wait_until(cx, "dice que todavía no está aquí", |cx| label(window, "thread-notice", cx) == "Todavía no está en esta app");
        follow("/tasks?task=no-existe-1", cx);
        wait_until(cx, "dice que no se pudo abrir", |cx| label(window, "thread-notice", cx) == "No se pudo abrir el enlace");
        follow("/tasks?task=cac-12", cx);
        wait_until(cx, "la tarea se abre en su página", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.as_deref() == Some("t1")));
        stop(cx);
    }

    // ── Escribir ────────────────────────────────────────────────────────

    /// Un servidor que, al recibir un mensaje en esa ruta, lo guarda en la
    /// conversación como el de verdad, y contesta con el mensaje en bruto.
    /// Va por delante del POST de cualquier cosa del servidor de mentira.
    fn mount_writes(server: &FakeServer, route: &str, talk: &History) {
        let talk = talk.clone();
        let direct = route.starts_with("/api/v1/dm/");
        server.mount(
            Mock::given(method("POST"))
                .and(path(route.to_string()))
                .respond_with(move |request: &Request| {
                    let sent: Value = serde_json::from_slice(&request.body).unwrap();
                    let mut talk = talk.lock().unwrap();
                    let next = 100 + talk.len() as u32;
                    let body = sent["body"].as_str().unwrap();
                    let written = if direct { private_line(&format!("w{next}"), "u1", "Ana Ruiz", body, next) } else { line(&format!("w{next}"), "u1", "Ana Ruiz", body, 5, next) };
                    talk.push(written.clone());
                    ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": { "id": written["id"], "body": body } }))
                })
                .with_priority(1),
        );
    }

    /// Lo que se mandó a esa ruta, en orden.
    fn sent(server: &FakeServer, route: &str) -> Vec<Value> {
        server.requests().iter().filter(|request| request.method.as_str() == "POST" && request.url.path() == route).map(|request| serde_json::from_slice(&request.body).unwrap()).collect()
    }

    /// Escribe en la caja, como con el teclado.
    fn write(window: AnyWindowHandle, view: &Entity<ChatView>, text: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            let field = view.read(cx).composer.field.clone();
            field.update(cx, |field, cx| field.focus(window, cx));
            window.input(text, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn press(window: AnyWindowHandle, key: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn written(view: &Entity<ChatView>, cx: &mut TestAppContext) -> String {
        cx.update(|cx| view.read(cx).composer.field.read(cx).value().to_string())
    }

    /// Escribir y Enter manda lo escrito, vacía la caja y baja al final,
    /// también si se leía más arriba; Mayús+Enter es otra línea.
    ///
    /// Roturas que atrapa: un Enter que salta de línea en vez de mandar;
    /// Mayús+Enter que manda a medias; la caja con lo mandado todavía
    /// dentro; lo mandado fuera de la vista porque se leía arriba (y un
    /// «Mensajes nuevos» por lo propio).
    #[gpui_kit::test]
    fn writing_and_enter_sends_and_shows_it_at_the_end(cx: &mut TestAppContext) {
        let server = server();
        let talk = history(numbered(0..40));
        mount_history(&server, CHAT_A, &talk, Duration::ZERO);
        mount_writes(&server, CHAT_A, &talk);
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se abre por el final", |cx| on_screen(window, "thread-line-m039", cx));

        write(window, &view, "uno", cx);
        press(window, "shift-enter", cx);
        write(window, &view, "dos", cx);
        assert_eq!(written(&view, cx), "uno\ndos");
        assert!(sent(&server, CHAT_A).is_empty(), "Mayús+Enter mandó");

        // Se lee más arriba y se manda: se baja a verlo.
        scroll_up(window, 700., cx);
        assert!(!on_screen(window, "thread-line-m039", cx));
        press(window, "enter", cx);
        wait_until(cx, "llega lo mandado", |cx| on_screen(window, "thread-line-w140", cx));
        assert_eq!(sent(&server, CHAT_A), [json!({ "body": "uno\ndos" })]);
        assert_eq!(written(&view, cx), "", "la caja se quedó con lo mandado");
        assert!(gap_below(window, "w140", cx).abs() <= 13., "lo mandado quedó fuera: {}", gap_below(window, "w140", cx));
        assert!(!on_screen(window, "thread-new-messages", cx), "se avisó de «nuevos» por lo propio");
        stop(cx);
    }

    /// `@` ofrece a la gente del canal; Enter pone la ficha con el nombre,
    /// que se borra entera; lo que se manda lleva el enlace que el servidor
    /// lee para avisar. En un directo no se ofrece a nadie.
    ///
    /// Roturas que atrapa: el enlace en crudo en la caja; una mención que
    /// se borra a trozos; mandar «@Luis Mora» en texto (el servidor no
    /// avisaría); que Enter mande el mensaje en vez de elegir; menciones en
    /// un directo.
    #[gpui_kit::test]
    fn naming_someone_puts_a_token_and_sends_its_link(cx: &mut TestAppContext) {
        let server = server();
        let talk = history(vec![line("m1", "u2", "Luis Mora", "hola", 5, 1)]);
        mount_history(&server, CHAT_A, &talk, Duration::ZERO);
        mount_writes(&server, CHAT_A, &talk);
        mount_history(&server, DM_C1, &history(Vec::new()), Duration::ZERO);
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta", |cx| on_screen(window, "thread-line-m1", cx));

        write(window, &view, "@lu", cx);
        wait_until(cx, "se ofrece a Luis", |cx| on_screen(window, "mention-u2", cx));
        assert!(!on_screen(window, "mention-u1", cx), "se ofreció a quien escribe");
        press(window, "enter", cx);
        assert_eq!(label(window, "composer-token-user-u2", cx), "@Luis Mora");
        assert_eq!(written(&view, cx), "[@luis](cac:user/u2) ");
        assert!(sent(&server, CHAT_A).is_empty(), "Enter mandó en vez de elegir");
        // El espacio, y luego la ficha entera.
        press(window, "backspace", cx);
        press(window, "backspace", cx);
        assert_eq!(written(&view, cx), "", "la mención se borró a trozos");

        write(window, &view, "hola @lu", cx);
        wait_until(cx, "se ofrece a Luis", |cx| on_screen(window, "mention-u2", cx));
        press(window, "enter", cx);
        write(window, &view, "¿vienes?", cx);
        press(window, "enter", cx);
        wait_until(cx, "se manda", |_| !sent(&server, CHAT_A).is_empty());
        assert_eq!(sent(&server, CHAT_A), [json!({ "body": "hola [@luis](cac:user/u2) ¿vienes?" })]);

        // En un directo, `@` es una arroba.
        cx.update(|cx| assert!(Backend::core(cx).comms().select(Conversation::Direct { conversation_id: "c1".into() })));
        wait_until(cx, "se abre el directo", |cx| on_screen(window, "thread-empty", cx));
        write(window, &view, "@lu", cx);
        assert!(!on_screen(window, "mentions", cx), "se ofrecieron menciones en un directo");
        stop(cx);
    }

    /// Responder pone encima de la caja a qué se responde; Escape lo quita;
    /// lo que se manda lleva a qué responde, y después la barra se va.
    ///
    /// Roturas que atrapa: una respuesta que sale sin su cita; una barra
    /// que no se puede quitar; la barra que se queda tras mandar y se pega
    /// al mensaje siguiente.
    #[gpui_kit::test]
    fn replying_sends_what_it_answers_and_escape_drops_it(cx: &mut TestAppContext) {
        let server = server();
        let talk = history(vec![line("m1", "u2", "Luis Mora", "¿sale **hoy**?", 5, 1)]);
        mount_history(&server, CHAT_A, &talk, Duration::ZERO);
        mount_writes(&server, CHAT_A, &talk);
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta", |cx| on_screen(window, "thread-line-m1", cx));

        click(window, "thread-reply-m1", cx);
        assert_eq!(label(window, "thread-replying", cx), "Respondiendo a Luis Mora: ¿sale hoy?");
        press(window, "escape", cx);
        assert!(!on_screen(window, "thread-replying", cx), "Escape no quitó la respuesta");

        click(window, "thread-reply-m1", cx);
        write(window, &view, "sí", cx);
        press(window, "enter", cx);
        wait_until(cx, "se manda", |_| !sent(&server, CHAT_A).is_empty());
        assert_eq!(sent(&server, CHAT_A), [json!({ "body": "sí", "replyToId": "m1" })]);
        wait_until(cx, "la barra se va", |cx| !on_screen(window, "thread-replying", cx));
        stop(cx);
    }

    /// Si no se puede mandar, lo escrito se queda y se dice por qué.
    ///
    /// Roturas que atrapa: perder lo escrito cuando falla; fallar en
    /// silencio.
    #[gpui_kit::test]
    fn a_failed_send_keeps_the_text_and_says_why(cx: &mut TestAppContext) {
        let server = server();
        mount_history(&server, CHAT_A, &history(vec![line("m1", "u2", "Luis Mora", "hola", 5, 1)]), Duration::ZERO);
        server.mount(
            Mock::given(method("POST"))
                .and(path(CHAT_A))
                .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "success": false, "error": "reply-elsewhere", "message": "x" })))
                .with_priority(1),
        );
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta", |cx| on_screen(window, "thread-line-m1", cx));

        write(window, &view, "a eso", cx);
        press(window, "enter", cx);
        wait_until(cx, "se dice que falló", |cx| on_screen(window, "thread-send-failed", cx));
        assert_eq!(label(window, "thread-send-failed", cx), "Ese mensaje ya no está en esta conversación: puede que lo hayan retirado.");
        assert_eq!(written(&view, cx), "a eso", "se perdió lo escrito");
        stop(cx);
    }

    /// Lo escrito en una conversación se queda en ella, con sus menciones,
    /// al ir a otra y volver; y no aparece en la otra.
    ///
    /// Roturas que atrapa: perder el borrador al cambiar de canal (lo que
    /// pasa en la app de siempre); un borrador que se cuela en otra
    /// conversación (lo que pasa allí con los directos); volver sin la
    /// ficha de la mención, con su enlace en crudo.
    #[gpui_kit::test]
    fn a_draft_stays_with_its_conversation(cx: &mut TestAppContext) {
        let server = server();
        mount_history(&server, CHAT_A, &history(vec![line("a1", "u2", "Luis Mora", "de A", 5, 1)]), Duration::ZERO);
        mount_history(&server, CHAT_B, &history(vec![line("b1", "u3", "Bea Soto", "de B", 5, 1)]), Duration::ZERO);
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta A", |cx| on_screen(window, "thread-line-a1", cx));
        write(window, &view, "para @lu", cx);
        wait_until(cx, "se ofrece a Luis", |cx| on_screen(window, "mention-u2", cx));
        press(window, "enter", cx);
        write(window, &view, "a medias", cx);

        click(window, "comms-channel-b", cx);
        wait_until(cx, "se pinta B", |cx| on_screen(window, "thread-line-b1", cx));
        assert_eq!(written(&view, cx), "", "el borrador de A se coló en B");
        write(window, &view, "otro de B", cx);

        click(window, "comms-channel-a", cx);
        wait_until(cx, "vuelve A", |cx| on_screen(window, "thread-line-a1", cx));
        assert_eq!(written(&view, cx), "para [@luis](cac:user/u2) a medias");
        assert_eq!(label(window, "composer-token-user-u2", cx), "@Luis Mora", "la mención volvió sin su ficha");
        click(window, "comms-channel-b", cx);
        wait_until(cx, "vuelve B", |cx| on_screen(window, "thread-line-b1", cx));
        assert_eq!(written(&view, cx), "otro de B");
        stop(cx);
    }

    /// Quien sólo mira la organización ve la caja de un canal cerrada y por
    /// qué, sin botón de mandar ni de responder.
    ///
    /// Roturas que atrapa: una caja abierta que acaba en un 403 al mandar;
    /// ofrecer responder a quien no puede escribir.
    #[gpui_kit::test]
    fn a_viewer_sees_the_box_closed_and_why(cx: &mut TestAppContext) {
        let server = server_as("viewer");
        mount_history(&server, CHAT_A, &history(vec![line("m1", "u2", "Luis Mora", "hola", 5, 1)]), Duration::ZERO);
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta", |cx| on_screen(window, "thread-line-m1", cx));
        assert_eq!(label(window, "thread-composer", cx), "Aquí puedes mirar, pero no cambiar nada.");
        assert!(!on_screen(window, "thread-send", cx) && !on_screen(window, "thread-reply-m1", cx));
        stop(cx);
    }

    // ── Adjuntar ────────────────────────────────────────────────────────

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\xff";
    const UPLOAD_A: &str = "/api/v1/task-spaces/a/chat/attachments";

    /// Un servidor que guarda lo que se sube a ese canal y contesta como el
    /// de verdad: con su ruta relativa y su nombre. Tarda eso.
    fn mount_uploads(server: &FakeServer, delay: Duration) {
        server.mount(
            Mock::given(method("POST"))
                .and(path(UPLOAD_A))
                .respond_with(move |request: &Request| {
                    let body = String::from_utf8_lossy(&request.body);
                    let (id, name, kind) = if body.contains("notas.txt") { ("f2", "notas.txt", "text/plain") } else { ("f1", "pasted.png", "image/png") };
                    let file = json!({ "id": id, "spaceId": "a", "url": format!("{UPLOAD_A}/{id}/raw"), "fileName": name, "contentType": kind });
                    ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": file })).set_delay(delay)
                })
                .with_priority(1),
        );
    }

    fn paste_picture(window: AnyWindowHandle, cx: &mut TestAppContext) {
        use gpui_kit::{ClipboardItem, Image, ImageFormat};
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, PNG.to_vec())));
        cx.update_window(window, |_, window, cx| window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx)).unwrap();
        cx.run_until_parked();
    }

    /// Una imagen pegada y un fichero soltado se suben al canal con la
    /// sesión y quedan como fichas con su nombre; mientras suben no se
    /// manda; y lo que se manda los cita como los lee quien lo recibe.
    ///
    /// Roturas que atrapa: pegar una imagen y que no se suba; un fichero
    /// soltado en la caja que no hace nada; mandar con un fichero a medio
    /// subir (su enlace caería en el mensaje siguiente, como en la app de
    /// siempre); la cita en crudo en la caja; una captura citada como
    /// enlace, que no se vería.
    #[gpui_kit::test]
    fn what_is_pasted_or_dropped_is_uploaded_and_sent_as_its_citation(cx: &mut TestAppContext) {
        use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput};
        let server = server();
        let talk = history(vec![line("m1", "u2", "Luis Mora", "hola", 5, 1)]);
        mount_history(&server, CHAT_A, &talk, Duration::ZERO);
        mount_writes(&server, CHAT_A, &talk);
        mount_uploads(&server, Duration::from_millis(400));
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta", |cx| on_screen(window, "thread-line-m1", cx));

        write(window, &view, "mira ", cx);
        paste_picture(window, cx);
        assert_eq!(label(window, "thread-uploading-1", cx), "Subiendo pasted.png…");
        // A medio subir no se manda, ni con el botón ni con Enter.
        click(window, "thread-send", cx);
        press(window, "enter", cx);
        assert!(sent(&server, CHAT_A).is_empty(), "se mandó con un fichero a medio subir");
        wait_until(cx, "la imagen queda como ficha", |cx| on_screen(window, "composer-token-file-f1", cx));
        assert_eq!(label(window, "composer-token-file-f1", cx), "pasted.png");
        assert!(!on_screen(window, "thread-uploading-1", cx));
        let uploaded = server.requests().into_iter().find(|request| request.url.path() == UPLOAD_A).unwrap();
        assert_eq!(uploaded.body, cac_core::multipart("pasted.png", PNG).1);
        assert!(uploaded.headers.get("Authorization").is_some(), "se subió sin la sesión");

        // Un fichero soltado en la caja.
        let dir = std::env::temp_dir().join(format!("cac-chat-drop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let notes = dir.join("notas.txt");
        std::fs::write(&notes, b"hola").unwrap();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let position = window.find("thread-composer").bounds().center();
            let paths = ExternalPaths(vec![notes.clone()].into());
            // El ratón pasa por la ventana camino del gestor de ficheros:
            // tras escribir, GPUI no ve nada «bajo el ratón» hasta que se
            // mueve, y un `FileDrop` no cuenta como movimiento (trampa
            // abierta en cac: soltar sin haber movido el ratón no llega).
            window.dispatch_event(PlatformInput::MouseMove(gpui_kit::MouseMoveEvent { position, pressed_button: None, modifiers: Default::default() }), cx);
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position, paths }), cx);
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Submit { position }), cx);
        })
        .unwrap();
        wait_until(cx, "el fichero queda como ficha", |cx| on_screen(window, "composer-token-file-f2", cx));
        let _ = std::fs::remove_dir_all(&dir);

        press(window, "enter", cx);
        wait_until(cx, "se manda", |_| !sent(&server, CHAT_A).is_empty());
        assert_eq!(sent(&server, CHAT_A), [json!({ "body": format!("mira ![pasted.png]({UPLOAD_A}/f1/raw) [notas.txt]({UPLOAD_A}/f2/raw)") })]);
        stop(cx);
    }

    /// Un fichero que termina de subir cuando ya se mira otra conversación
    /// queda en la suya, no en la que se tiene delante.
    ///
    /// Rotura que atrapa: la captura de un canal citada en el mensaje que
    /// se escribe en otro.
    #[gpui_kit::test]
    fn a_file_that_lands_after_leaving_stays_in_its_conversation(cx: &mut TestAppContext) {
        let server = server();
        mount_history(&server, CHAT_A, &history(vec![line("a1", "u2", "Luis Mora", "de A", 5, 1)]), Duration::ZERO);
        mount_history(&server, CHAT_B, &history(vec![line("b1", "u3", "Bea Soto", "de B", 5, 1)]), Duration::ZERO);
        mount_uploads(&server, Duration::from_millis(500));
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta A", |cx| on_screen(window, "thread-line-a1", cx));
        write(window, &view, "en A", cx);
        paste_picture(window, cx);
        click(window, "comms-channel-b", cx);
        wait_until(cx, "se pinta B", |cx| on_screen(window, "thread-line-b1", cx));
        wait_until(cx, "termina de subir", |cx| cx.update(|cx| view.read(cx).composer.field.read(cx).value().is_empty()) && server.hits(UPLOAD_A) == 1);
        std::thread::sleep(Duration::from_millis(700));
        cx.run_until_parked();
        assert_eq!(written(&view, cx), "", "la captura de A se citó en B");

        click(window, "comms-channel-a", cx);
        wait_until(cx, "vuelve A con su captura", |cx| on_screen(window, "composer-token-file-f1", cx));
        assert_eq!(written(&view, cx), format!("en A ![pasted.png]({UPLOAD_A}/f1/raw) "));
        stop(cx);
    }

    /// Un fichero que no se puede subir lo dice con su nombre y por qué, y
    /// no deja nada en la caja.
    ///
    /// Roturas que atrapa: fallar en silencio; decirlo dos veces; una ficha
    /// de un fichero que no está en el servidor.
    #[gpui_kit::test]
    fn a_file_that_cannot_be_uploaded_says_why(cx: &mut TestAppContext) {
        let server = server();
        mount_history(&server, CHAT_A, &history(vec![line("m1", "u2", "Luis Mora", "hola", 5, 1)]), Duration::ZERO);
        server.mount(
            Mock::given(method("POST"))
                .and(path(UPLOAD_A))
                .respond_with(ResponseTemplate::new(413).set_body_json(json!({ "success": false, "error": "too-large", "message": "File exceeds 30 MB" })))
                .with_priority(1),
        );
        let (window, view) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta", |cx| on_screen(window, "thread-line-m1", cx));
        write(window, &view, "x", cx);
        paste_picture(window, cx);
        wait_until(cx, "se dice que no se pudo", |cx| on_screen(window, "thread-send-failed", cx));
        assert_eq!(label(window, "thread-send-failed", cx), "No se pudo adjuntar pasted.png: Ese fichero es demasiado grande.");
        assert_eq!(written(&view, cx), "x");
        stop(cx);
    }

    // ── El visor de imágenes ────────────────────────────────────────────

    /// Una imagen del chat se abre en un visor sobre la ventana, y con las
    /// flechas se pasa por todas las de la conversación, en el orden en que
    /// se leen, sin dar la vuelta; Escape lo cierra.
    ///
    /// Roturas que atrapa: abrir la imagen con el programa del sistema en
    /// vez de en la app; un visor que sólo enseña la que se pulsó; flechas
    /// al revés; dar la vuelta de la última a la primera; un visor que no se
    /// cierra con Escape; la imagen pedida sin la sesión.
    #[gpui_kit::test]
    fn a_picture_opens_in_a_viewer_and_the_arrows_go_through_the_chat(cx: &mut TestAppContext) {
        let server = server();
        let png = include_bytes!("../../core/tests/fixtures/gray_600x200.png").to_vec();
        let raw = |id: &str| format!("/api/v1/task-spaces/a/chat/attachments/{id}/raw");
        for id in ["p1", "p2", "p3"] {
            server.mount(get(&raw(id)).and(header("Authorization", "Bearer access")).respond_with(ResponseTemplate::new(200).set_body_raw(png.clone(), "image/png")));
        }
        let messages = vec![
            line("m1", "u2", "Luis Mora", &format!("![uno.png]({})", raw("p1")), 5, 1),
            line("m2", "u3", "Bea Soto", "sin imagen", 5, 2),
            line("m3", "u2", "Luis Mora", &format!("dos:\n\n![dos.png]({})\n\n![tres.png]({})", raw("p2"), raw("p3")), 5, 3),
        ];
        mount_history(&server, CHAT_A, &history(messages), Duration::ZERO);
        let opened = Arc::new(Mutex::new(Vec::new()));
        let seen = opened.clone();
        cx.update(|cx| cx.set_global(crate::files::Opener(Arc::new(move |path| seen.lock().unwrap().push(path.to_path_buf())))));
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pintan las miniaturas", |cx| on_screen(window, &format!("thread-thumb-{}", raw("p2")), cx));

        click(window, &format!("thread-picture-{}", raw("p2")), cx);
        wait_until(cx, "se abre el visor", |cx| on_screen(window, "picture-viewer-image", cx));
        assert!(opened.lock().unwrap().is_empty(), "se abrió fuera de la app");
        assert_eq!(label(window, "picture-viewer-count", cx), "2 de 3");
        assert_eq!(label(window, "picture-viewer-image", cx), "dos.png");

        press(window, "right", cx);
        assert_eq!(label(window, "picture-viewer-count", cx), "3 de 3");
        assert_eq!(label(window, "picture-viewer-image", cx), "tres.png");
        assert!(!on_screen(window, "picture-viewer-next", cx), "hay flecha más allá de la última");
        press(window, "right", cx);
        assert_eq!(label(window, "picture-viewer-count", cx), "3 de 3", "se dio la vuelta");

        press(window, "left", cx);
        press(window, "left", cx);
        assert_eq!(label(window, "picture-viewer-count", cx), "1 de 3");
        assert_eq!(label(window, "picture-viewer-image", cx), "uno.png");
        press(window, "left", cx);
        assert_eq!(label(window, "picture-viewer-count", cx), "1 de 3", "se dio la vuelta");
        // Con los botones, igual.
        click(window, "picture-viewer-next", cx);
        assert_eq!(label(window, "picture-viewer-count", cx), "2 de 3");

        press(window, "escape", cx);
        assert!(!on_screen(window, "picture-viewer", cx), "Escape no cerró el visor");
        assert!(server.requests().iter().filter(|request| request.url.path().ends_with("/raw")).all(|request| request.headers.get("Authorization").is_some()), "una imagen se pidió sin la sesión");
        stop(cx);
    }

    // ── Corregir, retirar y reaccionar ──────────────────────────────────

    /// Lo propio se corrige en su sitio: Enter guarda, y se ve corregido y
    /// marcado; Escape deja como estaba. Lo de otro no ofrece corregir ni
    /// retirar.
    ///
    /// Roturas que atrapa: corregir lo de otro; una caja de corregir que no
    /// guarda; Escape que guarda; no ver lo corregido sin salir.
    #[gpui_kit::test]
    fn what_is_mine_is_corrected_in_place(cx: &mut TestAppContext) {
        let server = server();
        let talk = history(vec![line("m1", "u2", "Luis Mora", "de Luis", 5, 1), line("m2", "u1", "Ana Ruiz", "hola", 5, 2)]);
        mount_history(&server, CHAT_A, &talk, Duration::ZERO);
        let edits = talk.clone();
        server.mount(
            Mock::given(method("PATCH"))
                .and(path("/api/v1/task-spaces/a/chat/m2"))
                .respond_with(move |request: &Request| {
                    let sent: Value = serde_json::from_slice(&request.body).unwrap();
                    let mut talk = edits.lock().unwrap();
                    talk[1]["body"] = sent["body"].clone();
                    talk[1]["updatedAt"] = json!(when(5, 30));
                    ok(json!(null))
                })
                .with_priority(1),
        );
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta", |cx| on_screen(window, "thread-line-m2", cx));
        assert!(!on_screen(window, "thread-edit-m1", cx) && !on_screen(window, "thread-withdraw-m1", cx), "se ofrece corregir lo de otro");

        click(window, "thread-edit-m2", cx);
        assert!(on_screen(window, "thread-editing-m2", cx));
        cx.update_window(window, |_, window, cx| window.input(" y esto no", cx)).unwrap();
        press(window, "escape", cx);
        assert!(!on_screen(window, "thread-editing-m2", cx), "Escape no cerró la caja de corregir");
        // Lo que se manda sale por otro hilo: se le da tiempo a llegar.
        std::thread::sleep(Duration::from_millis(400));
        cx.run_until_parked();
        assert!(server.requests().iter().all(|request| request.method.as_str() != "PATCH"), "Escape guardó");
        assert_eq!(label(window, "thread-bubble-m2", cx), "hola");

        click(window, "thread-edit-m2", cx);
        cx.update_window(window, |_, window, cx| window.input(", corregido", cx)).unwrap();
        press(window, "enter", cx);
        wait_until(cx, "se ve corregido", |cx| label(window, "thread-bubble-m2", cx) == "hola, corregido");
        assert!(on_screen(window, "thread-edited-m2", cx));
        assert!(!on_screen(window, "thread-editing-m2", cx));
        stop(cx);
    }

    /// Retirar pregunta siempre, y sólo si se dice que sí se retira.
    ///
    /// Roturas que atrapa: retirar sin preguntar; retirar aunque se diga
    /// que no; seguir pintando lo retirado.
    #[gpui_kit::test]
    fn withdrawing_asks_first(cx: &mut TestAppContext) {
        let server = server();
        let talk = history(vec![line("m1", "u1", "Ana Ruiz", "hola", 5, 1), line("m2", "u1", "Ana Ruiz", "esto se va", 5, 2)]);
        mount_history(&server, CHAT_A, &talk, Duration::ZERO);
        let gone = talk.clone();
        server.mount(
            Mock::given(method("DELETE"))
                .and(path("/api/v1/task-spaces/a/chat/m2"))
                .respond_with(move |_: &Request| {
                    gone.lock().unwrap().retain(|message| message["id"] != "m2");
                    ok(json!(null))
                })
                .with_priority(1),
        );
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta", |cx| on_screen(window, "thread-line-m2", cx));

        click(window, "thread-withdraw-m2", cx);
        // Lo que se manda sale por otro hilo: se le da tiempo a llegar.
        std::thread::sleep(Duration::from_millis(400));
        cx.run_until_parked();
        assert!(server.requests().iter().all(|request| request.method.as_str() != "DELETE"), "se retiró sin esperar a que contestaran");
        assert!(cx.has_pending_prompt(), "no preguntó antes de retirar");
        cx.simulate_prompt_answer("Cancelar");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(400));
        cx.run_until_parked();
        assert!(server.requests().iter().all(|request| request.method.as_str() != "DELETE"), "se retiró diciendo que no");

        click(window, "thread-withdraw-m2", cx);
        cx.simulate_prompt_answer("Retirar");
        wait_until(cx, "el mensaje se va", |cx| !on_screen(window, "thread-line-m2", cx));
        stop(cx);
    }

    /// Reaccionar desde el selector y pulsar una pastilla ponen o quitan la
    /// reacción de quien mira, y se ve en el acto.
    ///
    /// Roturas que atrapa: un selector que no manda nada; una pastilla que
    /// no se puede pulsar; el `on` al revés (pulsar la propia la pondría
    /// otra vez); esperar al servidor para verla.
    #[gpui_kit::test]
    fn reacting_from_the_picker_or_a_pill(cx: &mut TestAppContext) {
        let server = server();
        let mut first = line("m1", "u2", "Luis Mora", "hola", 5, 1);
        first["reactions"] = json!([{ "emoji": "👍", "count": 1, "mine": true, "names": ["Ana Ruiz"] }]);
        mount_history(&server, CHAT_A, &history(vec![first]), Duration::ZERO);
        let answer = Gate::default();
        let held = answer.clone();
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/task-spaces/a/chat/m1/reactions"))
                .respond_with(move |_: &Request| {
                    held.wait();
                    ok(json!(null))
                })
                .with_priority(1),
        );
        let (window, _) = open(&server, cx);
        enter(window, channel("a"), cx);
        wait_until(cx, "se pinta", |cx| on_screen(window, "thread-line-m1", cx));
        let reactions = |server: &FakeServer| sent(server, "/api/v1/task-spaces/a/chat/m1/reactions");

        click(window, "thread-react-m1", cx);
        assert!(on_screen(window, "thread-picker-m1", cx));
        // La tercera del juego: 😂.
        click(window, "thread-pick-m1-2", cx);
        // Con el servidor sin contestar ya se ve.
        wait_until(cx, "se ve sin esperar al servidor", |cx| label(window, "thread-reaction-m1-1", cx) == "😂 1");
        assert!(!on_screen(window, "thread-picker-m1", cx));
        answer.open();
        wait_until(cx, "se manda", |_| !reactions(&server).is_empty());

        // La propia, pulsada, se quita.
        click(window, "thread-reaction-m1-0", cx);
        wait_until(cx, "se manda quitarla", |_| reactions(&server).len() == 2);
        assert_eq!(reactions(&server), [json!({ "emoji": "😂", "on": true }), json!({ "emoji": "👍", "on": false })]);
        stop(cx);
    }
}
