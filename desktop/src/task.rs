//! El detalle de una tarea: una página a pantalla completa, con la columna
//! de lectura y, al lado, un riel con sus propiedades.
//!
//! Página y no cajón, como en la app de siempre: una tarea es donde el
//! trabajo se describe, se discute y se decide, y para eso necesita sitio.
//! Las propiedades van en su riel para que la columna de lectura siga siendo
//! de lectura; en una ventana estrecha el riel baja debajo en vez de
//! apretarse al lado, donde ninguna de las dos mitades se leería.
//!
//! Qué se lee (cada frase, qué comentario es de un agente, qué subtarea está
//! hecha, por qué no hay nada todavía) lo dice el core en su `TaskSheet`.
//! Aquí está qué pestaña del hilo se mira, que es lo único propio, y la
//! pintura. Cada cambio se le pide al core, que es quien sabe qué mandar.

use std::collections::HashMap;
use std::sync::Arc;

use cac_core::{
    ApiError, Choice, MentionMark, MentionMenu, MentionPerson, SheetBlocker, SheetComment, SheetFile, SheetSubtask, SheetTag, StatusOption, TaskAttachment, TaskSheet,
    PRIORITIES,
};
use chrono::NaiveDate;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter, Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp, Position, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::editor::{Incoming, MarkdownEditor};
use crate::files;
use crate::lang::{error_text, locale, tr};

/// El ancho del riel de propiedades: el de la app de siempre (`lg:w-72`).
const RAIL_WIDTH: f32 = 288.;
/// Por debajo de este ancho de ventana el riel va debajo de la lectura (el
/// `lg` de la app de siempre).
pub(crate) const STACK_BELOW: f32 = 1024.;

/// Las líneas con que empieza cada caja de escribir, que luego crece con lo
/// escrito: el `minHeight` de cada `MarkdownEditor` de `TaskDetailDrawer.tsx`
/// (5rem la de comentar, 4rem la de corregir un comentario y 10rem la
/// descripción) en líneas de `text-sm`, que miden 1.25rem.
const COMMENT_ROWS: usize = 4;
const COMMENT_EDIT_ROWS: usize = 3;
const DESCRIPTION_ROWS: usize = 8;

/// Una de las cajas en las que se escribe markdown en la página.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Draft {
    /// El comentario nuevo.
    Comment,
    /// El comentario que se corrige.
    CommentEdit,
    Description,
}

/// Una imagen de la tarea, en lo que tarda en poder enseñarse.
enum Thumb {
    Loading,
    Ready(Arc<RenderImage>),
    /// No llegó, o no es algo que se pueda pintar: se queda con su nombre,
    /// que sigue abriéndola.
    Failed,
}

/// La imagen ya decodificada, o `None` si esta app no sabe pintarla (un
/// AVIF) o el fichero está roto. Qué imagen es lo dijo el core.
fn decoded(picture: cac_core::Picture, svg: SvgRenderer) -> Option<Arc<RenderImage>> {
    let format = ImageFormat::from_mime_type(&picture.mime)?;
    Image::from_bytes(format, picture.bytes).to_image_data(svg).ok()
}

/// Las dos pestañas del hilo.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Thread {
    Talk,
    Notes,
}

pub struct TaskView {
    thread: Thread,
    /// De qué tarea es la pestaña elegida: al abrir otra se vuelve a la
    /// conversación, que es donde se le habla a quien la abre.
    thread_of: Option<String>,
    /// Lo último que no se pudo guardar, dicho, y de qué tarea: no hay otro
    /// sitio donde enseñarlo, y callarlo deja un botón que parece no hacer nada.
    problem: Option<(String, String)>,
    /// Lo que se está escribiendo para comentar.
    comment: Entity<TextareaState>,
    /// Que el cliente no lo lea. Sólo cuenta donde un cliente podría leerlo.
    internal: bool,
    /// De qué tarea se están editando el título y la descripción. Al abrir
    /// otra no se sigue editando: lo escrito era de aquélla.
    editing: Option<String>,
    title_input: Entity<InputState>,
    body_input: Entity<TextareaState>,
    /// El selector del vencimiento.
    due: Entity<DatePickerState>,
    /// De qué tarea y qué día es lo que el selector enseña: se le pone el de
    /// la tarea cuando cambia, y no en cada pintada, que le pisaría a quien
    /// está eligiendo.
    due_shown: Option<(String, Option<String>)>,
    _due_changed: Subscription,
    /// Qué comentario se está editando, y lo que se escribe para él.
    editing_comment: Option<String>,
    comment_edit: Entity<TextareaState>,
    /// Lo que la caja de comentar promete ahora mismo (la clave del texto):
    /// cambia con quién vaya a leerlo.
    comment_promise: &'static str,
    /// El foco de la página cuando no se escribe en nada: es lo que deja a
    /// Escape cerrarla.
    page_focus: FocusHandle,
    /// A qué tarea se le dio ya el foco al abrirse: una vez, o cada pintada
    /// se lo quitaría a quien escribe.
    focused_for: Option<String>,
    /// Lo que se recuerda de la lista de a quién nombrar: a quién se bajó
    /// con las flechas, o que se cerró. Qué significa lo dice el core.
    mention_mark: Option<MentionMark>,
    /// Para que el marcado no se salga de la vista al bajar por la lista.
    mention_scroll: ScrollHandle,
    /// En qué tarea se le está poniendo nombre a una etiqueta nueva, y el
    /// nombre. Al abrir otra no se sigue: lo escrito era para aquélla.
    naming_tag: Option<String>,
    tag_name: Entity<InputState>,
    _tag_named: Subscription,
    /// Hay ficheros subiendo para citarlos, y en cuál de las cajas: treinta
    /// megas tardan, y un clip quieto parece no haber hecho nada.
    citing: Option<Draft>,
    /// Las imágenes de la tarea abierta, por el id de su adjunto. Se bajan
    /// una vez, al verla, y se tiran al abrir otra.
    thumbs: HashMap<String, Thumb>,
    /// Qué identificador se acaba de copiar: su pastilla lo dice un momento
    /// con una marca en vez del icono de copiar.
    copied: Option<&'static str>,
    _task_closed: Subscription,
}

/// Sube esos ficheros a la tarea abierta, uno tras otro, y al primero que no
/// se pueda se para. Dice cuáles quedaron subidos y, si lo hubo, cuál no y
/// por qué (sin porqué si no se pudo ni leer del disco).
async fn upload_all(core: cac_core::Core, files: Vec<Incoming>) -> (Vec<TaskAttachment>, Option<(String, Option<ApiError>)>) {
    let (mut attached, mut failed) = (Vec::new(), None);
    for file in files {
        let (name, read) = match file {
            Incoming::Path(path) => {
                let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                // El tamaño se mira antes de leer: un fichero enorme no se carga en
                // memoria para que digan que no. (Sin esto el core lo rechaza igual,
                // ya leído: no hay prueba que lo distinga sin medir memoria.)
                let read = std::fs::metadata(&path)
                    .map_err(|_| None)
                    .and_then(|file| cac_core::attachment_fits(file.len()).map_err(Some))
                    .and_then(|()| std::fs::read(&path).map_err(|_| None));
                (name, read)
            }
            // Una imagen pegada ya está en memoria.
            Incoming::Picture { name, bytes } => (name, Ok(bytes)),
        };
        let uploaded = match read {
            Ok(bytes) => core.tasks().attach(&name, bytes).await.map_err(Some),
            Err(error) => Err(error),
        };
        match uploaded {
            Ok(file) => attached.push(file),
            Err(error) => {
                failed = Some((name, error));
                break;
            }
        }
    }
    (attached, failed)
}

/// La línea y la letra (no el byte) de esa posición del texto, que es como
/// el campo de texto del kit nombra dónde va el cursor.
fn position_at(text: &str, offset: usize) -> Position {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() as u32;
    let character = before.rsplit('\n').next().unwrap_or("").chars().count() as u32;
    Position::new(line, character)
}

/// El rótulo de una sección de la columna de lectura: el `<h3>` de
/// `TaskDetailDrawer.tsx` (`text-xs font-medium uppercase`, apagado).
fn section_title(title: String, cx: &App) -> Div {
    div().flex_none().text_xs().font_weight(FontWeight::MEDIUM).text_color(cx.theme().muted_foreground).child(title.to_uppercase())
}

/// El botón pequeño que acompaña al rótulo de una sección («Editar»,
/// «+ Añadir»): el `Button size="sm" variant="ghost"` con `h-6 px-2
/// text-xs` de la app de siempre.
fn section_action(id: &'static str, label: String, cx: &App) -> Stateful<Div> {
    let hover = crate::theme::ghost_hover(cx);
    div()
        .id(id)
        .h_6()
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(8.))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label)
}

/// El rótulo de una propiedad del riel, con su icono (`mb-1 flex
/// items-center gap-1.5 text-xs text-muted-foreground`, icono `size-3.5`).
fn rail_label(icon: impl Into<Icon>, text: String, cx: &App) -> Div {
    div().flex().items_center().gap_1p5().text_xs().text_color(cx.theme().muted_foreground).child(icon.into().size_3p5().flex_none()).child(text)
}

/// Lo que dice al pasar el ratón una pieza que la app de siempre tiene y
/// ésta todavía no: está en su sitio, y no calla.
fn not_yet(cx: &App) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let phrase = tr(cx, "common:misc.notInThisApp");
    move |window, cx| Tooltip::new(phrase.clone()).build(window, cx)
}

/// El color de una etiqueta: el suyo o, sin él, el violeta de `TagChip.tsx`.
fn tag_tone(tag: &SheetTag) -> Hsla {
    let own = tag.color.strip_prefix('#').filter(|hex| hex.len() == 6).and_then(|hex| u32::from_str_radix(hex, 16).ok());
    Hsla::from(gpui_kit::rgb(own.unwrap_or(0x8B5CF6)))
}

impl TaskView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let due = cx.new(|cx| DatePickerState::new(window, cx));
        // El selector avisa cuando alguien elige un día o lo quita; ponerle
        // desde aquí el de la tarea no avisa.
        let due_changed = cx.subscribe_in(&due, window, |this, _, event: &DatePickerEvent, window, cx| {
            let DatePickerEvent::Change(value) = event;
            this.set_due(value.format("%Y-%m-%d").map(|day| day.to_string()), window, cx);
        });
        let tag_placeholder = tr(cx, "work:task.newTagLabel");
        let tag_name = cx.new(|cx| InputState::new(window, cx).placeholder(tag_placeholder));
        // Enter en el nombre crea la etiqueta, como «Crear».
        let tag_named = cx.subscribe_in(&tag_name, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create_tag(window, cx);
            }
        });
        Self {
            thread: Thread::Talk,
            thread_of: None,
            problem: None,
            comment: cx.new(|cx| TextareaState::new(window, cx).auto_grow(COMMENT_ROWS, 16)),
            internal: false,
            editing: None,
            title_input: cx.new(|cx| InputState::new(window, cx)),
            body_input: cx.new(|cx| TextareaState::new(window, cx).auto_grow(DESCRIPTION_ROWS, 30)),
            due,
            due_shown: None,
            _due_changed: due_changed,
            editing_comment: None,
            comment_edit: cx.new(|cx| TextareaState::new(window, cx).auto_grow(COMMENT_EDIT_ROWS, 16)),
            comment_promise: "work:task.writeComment",
            page_focus: cx.focus_handle(),
            focused_for: None,
            mention_mark: None,
            mention_scroll: ScrollHandle::new(),
            naming_tag: None,
            tag_name,
            _tag_named: tag_named,
            citing: None,
            thumbs: HashMap::new(),
            copied: None,
            // Cerrada la página ya no se pinta, así que no es al pintar
            // donde se tiran sus imágenes: una captura decodificada pesa.
            _task_closed: cx.observe(&Backend::snapshot(cx), |this, snapshot, cx| {
                if snapshot.read(cx).task.task_id.is_none() {
                    this.thumbs.clear();
                }
            }),
        }
    }

    /// Deja dicho en el cajón algo que no se pudo hacer con esa tarea.
    fn say_problem(&mut self, task_id: String, text: String, cx: &mut Context<Self>) {
        self.problem = Some((task_id, text));
        cx.notify();
    }

    fn open_task_id(cx: &App) -> Option<String> {
        Backend::snapshot(cx).read(cx).task.task_id.clone()
    }

    /// Baja un adjunto y lo abre con el programa del sistema que lo entiende.
    /// Su dirección pide sesión, así que no se le puede dar tal cual.
    pub(crate) fn open_file(&mut self, file_id: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = Self::open_task_id(cx) else { return };
        self.problem = None;
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move {
            let bytes = core.tasks().attachment_bytes(&file_id).await.map_err(Some)?;
            files::write_for_opening(&std::env::temp_dir(), &name, &bytes).map_err(|_| None::<ApiError>)
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, _, cx| match result {
                Ok(path) => files::open_with_system(&path, cx),
                Err(error) => {
                    let text = match error {
                        Some(error) => error_text(cx, &error, "work:task.errOpenFile"),
                        None => tr(cx, "work:task.errOpenFile"),
                    };
                    this.say_problem(task_id, text, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Guarda una copia de un adjunto donde se elija.
    pub(crate) fn save_file(&mut self, file_id: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = Self::open_task_id(cx) else { return };
        self.problem = None;
        cx.notify();
        let home = std::env::home_dir().unwrap_or_else(std::env::temp_dir);
        let chosen = cx.prompt_for_new_path(&home, Some(&cac_core::safe_file_name(&name)));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(target))) = chosen.await else { return };
            let Ok(answer) = this.update(cx, |_, cx| {
                Backend::ask(cx, move |core| async move {
                    let bytes = core.tasks().attachment_bytes(&file_id).await.map_err(Some)?;
                    files::write_copy(&target, &bytes).map_err(|_| None::<ApiError>)
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
                this.say_problem(task_id, text, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Dice que un fichero no se pudo adjuntar, y por qué si se sabe.
    fn say_not_attached(&mut self, task_id: String, name: &str, error: Option<ApiError>, cx: &mut Context<Self>) {
        let mut text = cac_core::text_with(locale(cx), "common:editor.errAttach", &[("name", name)]);
        if let Some(error) = error {
            text = format!("{text}. {}", error_text(cx, &error, "common:editor.errAttachBody"));
        }
        self.say_problem(task_id, text, cx);
    }

    /// El campo de esa caja.
    fn draft(&self, draft: Draft) -> &Entity<TextareaState> {
        match draft {
            Draft::Comment => &self.comment,
            Draft::CommentEdit => &self.comment_edit,
            Draft::Description => &self.body_input,
        }
    }

    /// Qué es lo que esa caja escribe ahora mismo: el comentario que se
    /// corrige, o la tarea. Si cambia mientras sube un fichero, lo escrito
    /// ya es otra cosa y no se cita en ello.
    fn draft_target(&self, draft: Draft, cx: &App) -> Option<String> {
        let task = Self::open_task_id(cx)?;
        match draft {
            Draft::Comment => Some(task),
            Draft::CommentEdit => self.editing_comment.clone(),
            Draft::Description => self.editing.clone().filter(|editing| *editing == task),
        }
    }

    /// Pregunta qué ficheros y los adjunta citándolos en esa caja: es el
    /// clip de su barra.
    pub(crate) fn attach_to(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            this.update_in(cx, |this, window, cx| this.cite_files(draft, paths.into_iter().map(Incoming::Path).collect(), window, cx)).ok();
        })
        .detach();
    }

    /// Sube esos ficheros a la tarea (los que se eligieron con el clip, se
    /// soltaron en la caja o se pegaron) y los cita en lo que esa caja
    /// escribe, donde está el cursor. Como en la app de siempre: un fichero
    /// que se adjunta al escribir es de la tarea, y lo escrito lo nombra.
    /// Cómo se cita y dónde lo dice el core.
    pub(crate) fn cite_files(&mut self, draft: Draft, files: Vec<Incoming>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = Self::open_task_id(cx) else { return };
        let target = self.draft_target(draft, cx);
        self.problem = None;
        self.citing = Some(draft);
        cx.notify();
        let answer = Backend::ask(cx, move |core| upload_all(core, files));
        cx.spawn_in(window, async move |this, cx| {
            let result = answer.await;
            this.update_in(cx, |this, window, cx| {
                this.citing = None;
                cx.notify();
                let Ok((files, failed)) = result else { return };
                // Lo escrito es para lo que se escribía al adjuntar: si ya
                // se mira otra tarea (o se corrige otro comentario), los
                // ficheros quedan en su tarea y no se citan en esto.
                if target.is_some() && this.draft_target(draft, cx) == target {
                    let field = this.draft(draft).clone();
                    for file in &files {
                        // Con lo que la caja dice ahora, no con lo que
                        // decía al elegir: subir tarda, y se sigue escribiendo.
                        let (text, cursor) = {
                            let field = field.read(cx);
                            (field.value().to_string(), field.cursor())
                        };
                        let Some(cited) = cac_core::with_citation(&text, cursor, file) else { continue };
                        let position = position_at(&cited.text, cited.cursor as usize);
                        field.update(cx, |field, cx| {
                            field.set_value(cited.text, window, cx);
                            field.set_cursor_position(position, window, cx);
                        });
                    }
                }
                if let Some((name, error)) = failed {
                    this.say_not_attached(task_id, &name, error, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// La caja de escribir de la página: el editor con su barra, con el clip
    /// que adjunta citando en ella y lo que se le suelte o se le pegue.
    fn render_draft(&self, prefix: &'static str, draft: Draft, cx: &mut Context<Self>) -> MarkdownEditor {
        let (choose, take) = (cx.entity().downgrade(), cx.entity().downgrade());
        MarkdownEditor::new(prefix, self.draft(draft))
            .attaching(self.citing == Some(draft))
            .attach(
                move |window, cx| {
                    let _ = choose.update(cx, |this, cx| this.attach_to(draft, window, cx));
                },
                move |files, window, cx| {
                    let _ = take.update(cx, |this, cx| this.cite_files(draft, files, window, cx));
                },
            )
    }

    /// Sigue un enlace de lo que se lee en la tarea. El de un fichero suyo
    /// se baja y se abre como al pulsarlo en sus adjuntos: su dirección pide
    /// sesión, y el navegador no la tiene. Una mención no va a ningún sitio.
    pub(crate) fn follow_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(file) = Backend::core(cx).tasks().linked_attachment(url) {
            self.open_file(file.id, file.file_name, window, cx);
        } else if crate::theme::leads_somewhere(url) {
            cx.open_url(url);
        }
    }

    /// Markdown de la tarea para leer, con sus enlaces seguidos por
    /// [`Self::follow_link`].
    fn prose(&self, id: String, text: String, cx: &mut Context<Self>) -> Div {
        let view = cx.entity().downgrade();
        crate::theme::prose_with_links(
            id,
            text,
            move |url, window, cx| {
                let _ = view.update(cx, |this, cx| this.follow_link(url, window, cx));
            },
            cx,
        )
    }

    /// Pasa a editar un comentario, con lo que dice.
    pub(crate) fn start_editing_comment(&mut self, comment_id: String, body: String, window: &mut Window, cx: &mut Context<Self>) {
        self.comment_edit.update(cx, |input, cx| input.set_value(body, window, cx));
        self.editing_comment = Some(comment_id);
        cx.notify();
    }

    pub(crate) fn cancel_editing_comment(&mut self, cx: &mut Context<Self>) {
        self.editing_comment = None;
        cx.notify();
    }

    /// Guarda lo escrito en el comentario que se edita. Si no se guardó, se
    /// sigue editando con lo escrito.
    pub(crate) fn save_comment_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(comment_id) = self.editing_comment.clone() else { return };
        let body = self.comment_edit.read(cx).value().to_string();
        self.save(
            "work:task.errSaveComment",
            move |core| async move { core.tasks().edit_comment(&comment_id, &body).await },
            |this, _, _| this.editing_comment = None,
            window,
            cx,
        );
    }

    /// Borra un comentario, preguntando antes: si el cliente podía verlo,
    /// también deja de verlo.
    pub(crate) fn delete_comment(&mut self, comment_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let (title, detail) = (tr(cx, "work:task.deleteCommentTitle"), tr(cx, "work:task.deleteCommentBody"));
        let (delete, cancel) = (tr(cx, "work:task.delete"), tr(cx, "work:task.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&detail), &[delete.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update_in(cx, |this, window, cx| {
                this.change(move |core| async move { core.tasks().delete_comment(&comment_id).await }, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Quita un adjunto, preguntando antes: si la descripción lo enseña, se
    /// dice que también se va del texto.
    pub(crate) fn remove_file(&mut self, file: SheetFile, window: &mut Window, cx: &mut Context<Self>) {
        let title = cac_core::text_with(locale(cx), "work:task.removeAttachmentTitle", &[("name", &file.name)]);
        let detail = tr(cx, if file.cited { "work:task.removeAttachmentInUse" } else { "work:task.removeAttachmentPlain" });
        let (remove, cancel) = (tr(cx, "work:task.remove"), tr(cx, "work:task.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&detail), &[remove.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update_in(cx, |this, window, cx| {
                this.change(move |core| async move { core.tasks().remove_attachment(&file.id).await }, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Pide las imágenes de la tarea que aún no se han pedido, y tira las
    /// que ya no son suyas. Cuáles son imágenes lo dice el core, y por él
    /// vienen: con la sesión, y sólo de este servidor.
    fn show_thumbs(&mut self, sheet: &TaskSheet, window: &mut Window, cx: &mut Context<Self>) {
        self.thumbs.retain(|id, _| sheet.attachments.iter().any(|file| file.image && file.id == *id));
        for file in sheet.attachments.iter().filter(|file| file.image) {
            if self.thumbs.contains_key(&file.id) {
                continue;
            }
            // Apuntada ya como pedida: la siguiente pintada no la pide otra
            // vez, ni tampoco si falla.
            self.thumbs.insert(file.id.clone(), Thumb::Loading);
            let id = file.id.clone();
            let answer = Backend::ask(cx, {
                let id = id.clone();
                move |core| async move { core.tasks().picture(&id).await }
            });
            let svg = cx.svg_renderer();
            cx.spawn_in(window, async move |this, cx| {
                let image = match answer.await {
                    // Decodificar una captura grande tarda: fuera del hilo
                    // que pinta.
                    Ok(Ok(picture)) => cx.background_spawn(async move { decoded(picture, svg) }).await,
                    _ => None,
                };
                this.update(cx, |this, cx| {
                    if let Some(thumb) = this.thumbs.get_mut(&id) {
                        *thumb = image.map_or(Thumb::Failed, Thumb::Ready);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// Una imagen de la tarea, a la vista: en un reporte de cliente la
    /// captura **es** el reporte. Pulsarla la abre; encima, al pasar el
    /// ratón, guardar una copia y quitarla. Como en la app de siempre
    /// (`max-h-40 max-w-64 object-contain`, la papelera arriba a la derecha
    /// y «citada aquí» abajo a la izquierda).
    fn render_picture(&self, file: &SheetFile, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, background) = (theme.muted_foreground, theme.border, theme.background);
        let (open, save, remove) = (file.clone(), file.clone(), file.clone());
        let group = SharedString::from(format!("task-image-{}", file.id));
        let body = match self.thumbs.get(&file.id) {
            Some(Thumb::Ready(image)) => {
                let natural = image.size(0);
                let (width, height) = cac_core::thumbnail_size(natural.width.0.max(0) as u32, natural.height.0.max(0) as u32);
                div()
                    .id(SharedString::from(format!("task-thumb-{}", file.id)))
                    .test_support()
                    .child(img(image.clone()).w(px(width as f32)).h(px(height as f32)).object_fit(ObjectFit::Contain))
                    .into_any_element()
            }
            // Mientras llega, o si no se pudo pintar: su hueco con su
            // nombre, que la abre igual.
            other => {
                let state = if matches!(other, Some(Thumb::Failed)) { "missing" } else { "wait" };
                div()
                    .id(SharedString::from(format!("task-thumb-{state}-{}", file.id)))
                    .test_support()
                    .w(px(160.))
                    .h(px(96.))
                    .p_2()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(Icon::new(Lucide::Image).text_color(muted))
                    .child(div().w_full().overflow_hidden().text_ellipsis().whitespace_nowrap().text_xs().text_color(muted).child(file.name.clone()))
                    .into_any_element()
            }
        };
        div()
            .group(group.clone())
            .relative()
            .flex_none()
            .child(
                div()
                    .id(SharedString::from(format!("task-file-{}", file.id)))
                    .test_support()
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.open_file(open.id.clone(), open.name.clone(), window, cx)))
                    .child(body),
            )
            .children(file.cited.then(|| {
                div()
                    .id(SharedString::from(format!("task-file-cited-{}", file.id)))
                    .test_support()
                    .absolute()
                    .bottom_1()
                    .left_1()
                    .px_1()
                    .rounded_sm()
                    .bg(background.opacity(0.85))
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(tr(cx, "work:task.citedHere"))
            }))
            .child(
                div()
                    .absolute()
                    .top_1()
                    .right_1()
                    .flex()
                    .rounded_sm()
                    .bg(background.opacity(0.8))
                    .opacity(0.)
                    .group_hover(group, |style| style.opacity(1.))
                    .child(
                        Button::new(SharedString::from(format!("task-file-save-{}", file.id)))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::Download))
                            .tooltip(tr(cx, "work:task.saveFile"))
                            .on_click(cx.listener(move |this, _, window, cx| this.save_file(save.id.clone(), save.name.clone(), window, cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("task-file-remove-{}", file.id)))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::Trash))
                            .tooltip(tr(cx, "work:task.removeAttachment"))
                            .on_click(cx.listener(move |this, _, window, cx| this.remove_file(remove.clone(), window, cx))),
                    ),
            )
    }

    /// Un fichero de la tarea que no es una imagen: su nombre lo abre, y al
    /// pasar el ratón salen guardar una copia y quitarlo. La fila es la de
    /// «lo demás» en `TaskDetailDrawer.tsx` (`group flex items-center gap-2
    /// text-xs`, el clip `size-3`, el nombre subrayado en el color de acción).
    fn render_file(&self, file: &SheetFile, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, primary, chip) = (theme.muted_foreground, theme.primary, theme.muted);
        let (open, save, remove) = (file.clone(), file.clone(), file.clone());
        let group = SharedString::from(format!("task-file-row-{}", file.id));
        div()
            .group(group.clone())
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .child(Icon::new(Lucide::Paperclip).size_3().flex_none().text_color(muted))
            .child(
                div()
                    .id(SharedString::from(format!("task-file-{}", file.id)))
                    .test_support()
                    .min_w_0()
                    .truncate()
                    .text_color(primary)
                    .underline()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.open_file(open.id.clone(), open.name.clone(), window, cx)))
                    .child(file.name.clone()),
            )
            .children(file.size.clone().map(|size| div().flex_none().text_color(muted).child(size)))
            .children(file.cited.then(|| div().flex_none().px_1().rounded(px(4.)).bg(chip).text_color(muted).child(tr(cx, "work:task.citedHere"))))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .opacity(0.)
                    .group_hover(group, |style| style.opacity(1.))
                    .child(
                        Button::new(SharedString::from(format!("task-file-save-{}", file.id)))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::Download))
                            .tooltip(tr(cx, "work:task.saveFile"))
                            .on_click(cx.listener(move |this, _, window, cx| this.save_file(save.id.clone(), save.name.clone(), window, cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("task-file-remove-{}", file.id)))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::Trash))
                            .tooltip(tr(cx, "work:task.removeAttachment"))
                            .on_click(cx.listener(move |this, _, window, cx| this.remove_file(remove.clone(), window, cx))),
                    ),
            )
    }

    /// Pasa a editar el título y la descripción, con lo que la tarea dice.
    pub(crate) fn start_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = Backend::core(cx).tasks().sheet(locale(cx)) else { return };
        self.title_input.update(cx, |input, cx| input.set_value(sheet.title, window, cx));
        self.body_input.update(cx, |input, cx| input.set_value(sheet.description, window, cx));
        self.editing = Some(sheet.id);
        cx.notify();
    }

    /// Deja de editar sin guardar: lo escrito se tira.
    pub(crate) fn cancel_editing(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        cx.notify();
    }

    /// Guarda el título y la descripción escritos y, si se guardaron, deja de
    /// editar. Si no, se sigue editando con lo escrito.
    pub(crate) fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.title_input.read(cx).value().to_string();
        let body = self.body_input.read(cx).value().to_string();
        self.save(
            "common:misc.errSave",
            move |core| async move {
                core.tasks().set_title(&title).await?;
                core.tasks().set_description(&body).await
            },
            |this, _, _| this.editing = None,
            window,
            cx,
        );
    }

    /// Le pone al selector el día de la tarea, cuando es otra tarea u otro
    /// día que el que enseña.
    fn show_due(&mut self, sheet: &TaskSheet, window: &mut Window, cx: &mut Context<Self>) {
        let now = (sheet.id.clone(), sheet.due_day.clone());
        if self.due_shown.as_ref() == Some(&now) {
            return;
        }
        let day = sheet.due_day.as_deref().and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok());
        self.due.update(cx, |due, cx| due.set_date(Date::Single(day), window, cx));
        self.due_shown = Some(now);
    }

    /// Un cambio de un toque en la tarea abierta.
    fn change<F, Fut>(&mut self, work: F, window: &mut Window, cx: &mut Context<Self>)
    where
        F: FnOnce(cac_core::Core) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), cac_core::ApiError>> + Send + 'static,
    {
        self.save("common:misc.errSave", work, |_, _, _| {}, window, cx);
    }

    pub(crate) fn set_priority(&mut self, priority: String, window: &mut Window, cx: &mut Context<Self>) {
        self.change(move |core| async move { core.tasks().set_priority(&priority).await }, window, cx);
    }

    /// Pone el vencimiento en ese día (`YYYY-MM-DD`), o lo quita.
    pub(crate) fn set_due(&mut self, day: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.change(move |core| async move { core.tasks().set_due(day.as_deref()).await }, window, cx);
    }

    pub(crate) fn toggle_tag(&mut self, tag_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.change(move |core| async move { core.tasks().toggle_tag(&tag_id).await }, window, cx);
    }

    /// Pasa a pedir el nombre de una etiqueta nueva para la tarea abierta.
    pub(crate) fn start_naming_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = Self::open_task_id(cx) else { return };
        self.naming_tag = Some(task_id);
        self.tag_name.update(cx, |name, cx| {
            name.set_value("", window, cx);
            name.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn cancel_naming_tag(&mut self, cx: &mut Context<Self>) {
        self.naming_tag = None;
        cx.notify();
    }

    /// Le pone a la tarea la etiqueta con el nombre escrito: la crea si no
    /// la hay (eso lo sabe el core). Si no se pudo, se dice y el nombre se
    /// queda escrito.
    pub(crate) fn create_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.tag_name.read(cx).value().to_string();
        // Como al comentar: con nada escrito no hay nada que pedir ni que avisar.
        if name.trim().is_empty() {
            return;
        }
        self.save(
            "common:misc.errSave",
            move |core| async move { core.tasks().add_tag_named(&name).await },
            |this, _, _| this.naming_tag = None,
            window,
            cx,
        );
    }

    pub(crate) fn toggle_assignee(&mut self, user_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.change(move |core| async move { core.tasks().toggle_assignee(&user_id).await }, window, cx);
    }

    /// Le pide al core algo que se guarda en la tarea abierta y, si no se
    /// pudo, lo deja dicho en el cajón. `done` corre si salió bien.
    fn save<F, Fut>(
        &mut self,
        fallback_key: &'static str,
        work: F,
        done: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(cac_core::Core) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), cac_core::ApiError>> + Send + 'static,
    {
        let Some(task_id) = Backend::snapshot(cx).read(cx).task.task_id.clone() else { return };
        self.problem = None;
        cx.notify();
        let answer = Backend::ask(cx, work);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => done(this, window, cx),
                    Err(error) => {
                        this.problem = Some((task_id, error_text(cx, &error, fallback_key)));
                        // El selector de fecha se quedó con lo que se eligió
                        // y no se guardó: que vuelva a enseñar lo que hay.
                        this.due_shown = None;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Marca o desmarca una línea del checklist.
    pub(crate) fn tick_subtask(&mut self, subtask_id: String, done: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.save(
            "work:task.errMoveList",
            move |core| async move { core.tasks().set_subtask_done(&subtask_id, done).await },
            |_, _, _| {},
            window,
            cx,
        );
    }

    /// Manda lo escrito como comentario y, si se guardó, vacía la caja. Si
    /// no, lo escrito se queda: perderlo por un fallo de red es lo peor.
    pub(crate) fn send_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.comment.read(cx).value().to_string();
        if body.trim().is_empty() {
            return;
        }
        let internal = self.internal;
        self.save(
            "work:task.errComment",
            move |core| async move { core.tasks().say(&body, internal).await },
            |this, window, cx| this.comment.update(cx, |comment, cx| comment.set_value("", window, cx)),
            window,
            cx,
        );
    }

    /// Lo escrito en la caja de comentar y dónde está el cursor.
    fn comment_cursor(&self, cx: &App) -> (String, usize) {
        let comment = self.comment.read(cx);
        (comment.value().to_string(), comment.cursor())
    }

    /// Cambia el `@algo` que se escribía por la mención de esa persona y deja
    /// el cursor detrás, en la caja, para seguir escribiendo.
    pub(crate) fn pick_mention(&mut self, person: &MentionPerson, window: &mut Window, cx: &mut Context<Self>) {
        let (text, cursor) = self.comment_cursor(cx);
        let Some(edit) = cac_core::with_mention(&text, cursor, person) else { return };
        let position = position_at(&edit.text, edit.cursor as usize);
        self.comment.update(cx, |comment, cx| {
            comment.set_value(edit.text, window, cx);
            comment.set_cursor_position(position, window, cx);
        });
        cx.notify();
    }

    /// La lista de a quién nombrar con lo que se está escribiendo, si hay
    /// alguna abierta. Quién sale, quién va marcado y cuándo se cierra lo
    /// decide el core.
    fn mention_menu(&self, cx: &App) -> Option<MentionMenu> {
        let (text, cursor) = self.comment_cursor(cx);
        Backend::core(cx).tasks().mention_menu(&text, cursor, self.internal, self.mention_mark.as_ref())
    }

    /// Mueve el marcado de la lista. `false` si no hay lista: la tecla es
    /// entonces de la caja.
    pub(crate) fn move_mention(&mut self, by: i32, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.mention_menu(cx) else { return false };
        let mark = cac_core::mention_moved(&menu, by);
        self.mention_scroll.scroll_to_item(mark.highlighted as usize);
        self.mention_mark = Some(mark);
        cx.notify();
        true
    }

    /// Nombra a quien va marcado. `false` si no hay lista.
    pub(crate) fn pick_highlighted_mention(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.mention_menu(cx) else { return false };
        let Some(person) = cac_core::mention_highlighted(&menu).cloned() else { return false };
        self.pick_mention(&person, window, cx);
        true
    }

    /// Cierra la lista sin nombrar a nadie. `false` si no hay lista.
    pub(crate) fn close_mentions(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.mention_menu(cx) else { return false };
        self.mention_mark = Some(cac_core::mention_closed(&menu));
        cx.notify();
        true
    }

    /// A quién se puede nombrar con lo que se está escribiendo; elegir a
    /// alguien escribe su mención.
    fn render_mentions(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let menu = self.mention_menu(cx)?;
        let highlighted = menu.highlighted as usize;
        let offered = menu.people;
        let theme = cx.theme();
        let (border, popover, hover, muted, radius) = (theme.border, theme.popover, theme.accent, theme.muted_foreground, theme.radius);
        Some(
            div()
                .id("mentions")
                .test_support()
                .flex()
                .flex_col()
                .p_1()
                .max_h(px(220.))
                .overflow_y_scroll()
                .track_scroll(&self.mention_scroll)
                .rounded(radius)
                .border_1()
                .border_color(border)
                .bg(popover)
                .children(offered.into_iter().enumerate().map(|(at, person)| {
                    let named = person.name != person.username;
                    let picked = person.clone();
                    div()
                        .id(SharedString::from(format!("mention-{}", person.id)))
                        .test_support()
                        // El marcado, con el fondo del que se señala con el
                        // ratón (`bg-accent` en la app de siempre): es a
                        // quien Enter va a nombrar.
                        .when(at == highlighted, |this| this.bg(hover))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_sm()
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| this.pick_mention(&picked, window, cx)))
                        .when(named, |this| this.child(person.name.clone()))
                        .child(div().text_color(muted).child(format!("@{}", person.username)))
                })),
        )
    }

    pub(crate) fn set_internal(&mut self, internal: bool, cx: &mut Context<Self>) {
        self.internal = internal;
        cx.notify();
    }

    /// La caja de comentar. Donde un cliente lee, dice quién va a leerlo y
    /// deja cambiarlo antes de mandar. La fila de debajo es la de
    /// `TaskDetailDrawer.tsx`: comentar (con su avión), registrar una
    /// decisión y, donde un cliente lee, el interruptor de para quién es.
    fn render_composer(&self, client_reads: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let internal = client_reads && self.internal;
        let theme = cx.theme();
        let (muted, foreground, chip) = (theme.muted_foreground, theme.foreground, theme.muted);
        let audience = client_reads.then(|| {
            let hint = tr(cx, if internal { "work:task.onlyTeamWillSee" } else { "work:task.clientReadsSwitch" });
            div()
                .id("comment-audience")
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .text_xs()
                .cursor_pointer()
                .when(internal, |this| this.bg(chip).text_color(foreground))
                .when(!internal, |this| this.text_color(muted).hover(move |style| style.text_color(foreground)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_internal(!internal, cx)))
                .child(Icon::new(if internal { Lucide::EyeOff } else { Lucide::Eye }).size_3())
                .child(tr(cx, if internal { "work:task.internalNote" } else { "work:task.theClientReads" }))
                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                .test_support()
        });
        // Con la caja en blanco no hay nada que mandar: el botón sale apagado
        // (`disabled={sending || !comment.trim()}`), y qué es «en blanco» lo
        // dice el core.
        let empty = !cac_core::says_something(self.comment.read(cx).value().as_ref());
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    // Con la lista abierta, las flechas, Enter, Tab y Escape
                    // son suyas. Se cogen antes de que lleguen a la caja, y
                    // sólo entonces: sin lista siguen siendo de quien escribe.
                    .when(self.mention_menu(cx).is_some(), |this| {
                        this.capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                            if this.move_mention(-1, cx) {
                                cx.stop_propagation();
                            }
                        }))
                        .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                            if this.move_mention(1, cx) {
                                cx.stop_propagation();
                            }
                        }))
                        // Mayúsculas+Enter sigue siendo un salto de línea.
                        .capture_action(cx.listener(|this, enter: &Enter, window, cx| {
                            if !enter.shift && !enter.secondary && this.pick_highlighted_mention(window, cx) {
                                cx.stop_propagation();
                            }
                        }))
                        .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                            if this.pick_highlighted_mention(window, cx) {
                                cx.stop_propagation();
                            }
                        }))
                        .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                            if this.close_mentions(cx) {
                                cx.stop_propagation();
                            }
                        }))
                    })
                    .child(self.render_draft("comment", Draft::Comment, cx).placeholder(tr(cx, self.comment_promise))),
            )
            .children(self.render_mentions(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("comment-send")
                            .primary()
                            .xsmall()
                                    .h_7()
                                    .px_2p5()
                            .icon(Icon::new(Lucide::Send))
                            .label(tr(cx, if internal { "work:task.commentInternally" } else { "work:task.comment" }))
                            .disabled(empty)
                            .on_click(cx.listener(|this, _, window, cx| this.send_comment(window, cx))),
                    )
                    // Publicar y registrar en un gesto: todavía no está aquí.
                    .child(
                        div()
                            .id("comment-decision")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .text_xs()
                            .text_color(muted)
                            // `disabled={sending || !comment.trim()}`, con
                            // su `disabled:opacity-50`: apagado como «Comentar».
                            .when(empty, |this| this.opacity(0.5))
                            .when(!empty, |this| this.hover(move |style| style.text_color(foreground)))
                            .child(Icon::new(Lucide::Gavel).size_3())
                            .child(tr(cx, "work:decisions.record"))
                            .tooltip(not_yet(cx))
                            .test_support(),
                    )
                    .children(audience),
            )
    }

    fn show_thread(&mut self, thread: Thread, task_id: String, cx: &mut Context<Self>) {
        self.thread = thread;
        self.thread_of = Some(task_id);
        cx.notify();
    }

    /// Mueve la tarea abierta a otra columna. Si se puede lo decide el core
    /// (y el servidor); aquí sólo se enseña lo que conteste.
    pub(crate) fn move_to(&mut self, status_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.save(
            "work:task.errMoveList",
            move |core| async move { core.tasks().set_status(&status_id).await },
            |_, _, _| {},
            window,
            cx,
        );
    }

    /// Un desplegable de cosas que se ponen y se quitan: lo que la tarea
    /// tiene, marcado. Lo que ya tiene se pinta al lado, cada cosa con su
    /// forma; el botón sólo dice qué se puede hacer (`face`).
    fn render_choices(
        &self,
        id: &'static str,
        face: AnyElement,
        choices: Vec<Choice>,
        toggle: fn(&mut Self, String, &mut Window, &mut Context<Self>),
        // Lo último del desplegable, si lo hay: hacer uno nuevo (el
        // «+ Etiqueta nueva» de la app de siempre).
        make: Option<(String, fn(&mut Self, &mut Window, &mut Context<Self>))>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().downgrade();
        Button::new(id).ghost().p_0().h_auto().child(face).dropdown_menu(move |mut menu, _, _| {
            for choice in choices.clone() {
                let view = view.clone();
                menu = menu.item(PopupMenuItem::new(choice.name).checked(choice.chosen).on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| toggle(this, choice.id.clone(), window, cx));
                }));
            }
            if let Some((label, make)) = make.clone() {
                let view = view.clone();
                if !choices.is_empty() {
                    menu = menu.separator();
                }
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| make(this, window, cx));
                }));
            }
            menu
        })
    }

    /// El nombre de una etiqueta nueva, debajo de las etiquetas: en la app
    /// de siempre es un diálogo con esta misma pregunta.
    fn render_tag_naming(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("task-new-tag")
            .test_support()
            .flex()
            .items_center()
            .gap_1()
            .child(div().id("task-new-tag-name").test_support().flex_1().min_w_0().child(Input::new(&self.tag_name).small()))
            .child(
                Button::new("task-new-tag-create")
                    .primary()
                    .xsmall()
                    .label(tr(cx, "work:task.create"))
                    .on_click(cx.listener(|this, _, window, cx| this.create_tag(window, cx))),
            )
            .child(
                Button::new("task-new-tag-cancel")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Lucide::X))
                    .tooltip(tr(cx, "work:task.cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_naming_tag(cx))),
            )
    }

    /// Copia un identificador y lo dice un momento en su pastilla.
    pub(crate) fn copy_id(&mut self, chip: &'static str, value: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        self.copied = Some(chip);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(1200)).await;
            this.update(cx, |this, cx| {
                if this.copied == Some(chip) {
                    this.copied = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Un identificador que se copia de un clic, enseñado corto (la pastilla
    /// de `theme::id_chip`). Lo que se copia es el identificador entero, que
    /// es lo que sale al pasar el ratón.
    fn render_copy_chip(&self, id: &'static str, value: String, shown: String, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = value.clone();
        crate::theme::id_chip(id, shown, self.copied == Some(id), cx)
            .on_click(cx.listener(move |this, _, _, cx| this.copy_id(id, value.clone(), cx)))
            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
            .test_support()
    }

    /// Abre otra tarea desde ésta: su madre, o una de la que depende.
    fn open_task(id: String, cx: &App) {
        Backend::run(cx, move |core| async move { core.tasks().open(&id).await });
    }

    /// Una tarea a la que ésta espera: su referencia en una pastilla con
    /// borde (`rounded border px-1.5 py-0.5 text-xs`), tachada y apagada si
    /// ya está hecha. Pulsarla la abre.
    fn render_blocker(&self, blocker: &SheetBlocker, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let (id, title) = (blocker.id.clone(), blocker.title.clone());
        div()
            .id(SharedString::from(format!("task-blocker-{}", blocker.id)))
            .flex_none()
            .px_1p5()
            .py_0p5()
            .rounded(px(4.))
            .border_1()
            .border_color(border)
            .text_xs()
            .cursor_pointer()
            .when(blocker.done, |this| this.line_through().opacity(0.6))
            .on_click(move |_, _, cx| Self::open_task(id.clone(), cx))
            .child(blocker.label.clone())
            .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
            .test_support()
    }

    /// El riel de propiedades, con las medidas del `<aside>` de
    /// `TaskDetailDrawer.tsx`: cada propiedad es su rótulo con icono y,
    /// debajo, su control; apiladas, no en dos columnas (en 288 px una
    /// etiqueta al lado deja al control sin sitio).
    fn render_properties(&self, sheet: &TaskSheet, statuses: Vec<StatusOption>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground, border, secondary, on_secondary, unknown, input) =
            (theme.muted_foreground, theme.foreground, theme.border, theme.secondary, theme.secondary_foreground, theme.muted, theme.input);
        let field = crate::theme::field_background(cx);
        let view = cx.entity().downgrade();
        let tasks = Backend::core(cx).tasks();
        // El rótulo lleva `mb-1` y todo va a `space-y-3`: 4 px del rótulo a
        // su control y 12 del control al rótulo siguiente.
        let property = |label: Div, control: AnyElement| div().flex().flex_col().gap_1().child(label).child(div().min_w_0().flex().child(control));

        // El estado: un chip del color de su estado (los `--status-*` del
        // tema, no el hex del servidor), que abre las columnas de su lista.
        let status_chip = {
            let tone = sheet.status.map(|status| crate::board::status_tone(status, cx));
            div()
                .px_2()
                .py_0p5()
                .rounded(px(4.))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .map(|this| match tone {
                    Some(tone) => this.bg(tone.opacity(0.15)).text_color(tone),
                    None => this.bg(unknown).text_color(muted),
                })
                .child(sheet.status_name.clone())
        };
        let current = sheet.status_name.clone();
        let status = Button::new("task-status").ghost().p_0().h_auto().child(status_chip).dropdown_menu({
            let view = view.clone();
            move |mut menu, _, _| {
                // Las columnas de su lista; la que su máquina no deja sale
                // apagada en vez de ser un botón que contesta 409.
                for option in statuses.clone() {
                    let (view, id) = (view.clone(), option.status.id.clone());
                    menu = menu.item(
                        PopupMenuItem::new(option.status.name.clone())
                            .checked(option.status.name == current)
                            .disabled(!option.allowed)
                            .on_click(move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| this.move_to(id.clone(), window, cx));
                            }),
                    );
                }
                menu
            }
        });

        // Mover a otra lista: en la app de siempre es un desplegable con las
        // listas agrupadas por espacio. Aquí todavía sólo dice en cuál vive.
        let list = div()
            .id("task-list")
            .w_full()
            .h_7()
            .flex()
            .items_center()
            .justify_between()
            .gap_1p5()
            .pl_2p5()
            .pr_2()
            .rounded(px(8.))
            .border_1()
            .border_color(input)
            .bg(field)
            .text_sm()
            .child(div().min_w_0().truncate().child(sheet.list.clone()))
            .child(Icon::new(IconName::ChevronDown).size_4().flex_none().text_color(muted))
            .tooltip(not_yet(cx))
            .test_support();

        let current_priority = sheet.priority.clone();
        let priority = Button::new("task-priority")
            .ghost()
            .p_0()
            .h_auto()
            .rounded_full()
            .child(crate::mywork::priority_chip_shown(&sheet.priority, cx))
            .dropdown_menu({
                let view = view.clone();
                let names: Vec<(String, String)> =
                    PRIORITIES.iter().map(|priority| (priority.to_string(), tr(cx, &format!("work:taskPriority.{priority}")))).collect();
                move |mut menu, _, _| {
                    for (priority, name) in names.clone() {
                        let view = view.clone();
                        menu = menu.item(PopupMenuItem::new(name).checked(priority == current_priority).on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| this.set_priority(priority.clone(), window, cx));
                        }));
                    }
                    menu
                }
            });

        // Las etiquetas que tiene, cada una en su chip (`TagChip.tsx`: su
        // color al 15% y su punto), y al final el «+» que abre las demás.
        let tag_chips = sheet.tags.iter().map(|tag| {
            let tone = tag_tone(tag);
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .px_1p5()
                .py_0p5()
                .rounded(px(4.))
                .bg(tone.opacity(0.15))
                .text_xs()
                .text_color(foreground)
                .child(div().size_2().flex_none().rounded_full().bg(tone))
                .child(tag.name.clone())
        });
        let add_tag = div()
            .px_1p5()
            .py_0p5()
            .rounded(px(4.))
            .border_1()
            .border_dashed()
            .border_color(border)
            .text_xs()
            .text_color(muted)
            .child(tr(cx, "work:task.addTagShort"))
            .into_any_element();
        let new_tag: fn(&mut Self, &mut Window, &mut Context<Self>) = Self::start_naming_tag;
        let tag_menu = self.render_choices("task-tags", add_tag, tasks.tag_choices(), Self::toggle_tag, Some((tr(cx, "work:task.newTag"), new_tag)), cx);
        // El nombre se pide en la tarea donde se pidió, no en la que se abra después.
        let naming = (self.naming_tag.as_deref() == Some(sheet.id.as_str())).then(|| self.render_tag_naming(cx).into_any_element());
        let tags = div()
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .child(div().flex().flex_wrap().items_center().gap_1().children(tag_chips).child(tag_menu))
            .children(naming);

        // Quién la lleva: cada persona en su pastilla con su × (el `Badge`
        // secundario de la app de siempre), y al lado a quién más dársela.
        let people = tasks.assignee_choices();
        let badges = people.iter().filter(|person| person.chosen).map(|person| {
            let id = person.id.clone();
            let remove = cac_core::text_with(locale(cx), "work:task.removeAssignee", &[("name", &person.name)]);
            div()
                .h_5()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .px_2()
                .rounded_full()
                .bg(secondary)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(on_secondary)
                .child(person.name.clone())
                .child(
                    div()
                        .id(SharedString::from(format!("task-unassign-{}", person.id)))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| this.toggle_assignee(id.clone(), window, cx)))
                        .child(Icon::new(Lucide::X).size_3())
                        .tooltip(move |window, cx| Tooltip::new(remove.clone()).build(window, cx))
                        .test_support(),
                )
        });
        let badges: Vec<_> = badges.collect();
        // El campo de buscar a quién (`UserPicker`, `w-44`, con el `Input`
        // de `ui/input.tsx`: `h-8 rounded-lg border-input px-2.5 text-sm`):
        // aquí abre la lista del equipo.
        let assign = div()
            .w(rems(11.))
            .h_8()
            .flex()
            .items_center()
            .px_2p5()
            // `rounded-lg`: 10 px con el `--radius` de `index.css`.
            .rounded(px(10.))
            .border_1()
            .border_color(input)
            .bg(field)
            .text_sm()
            .text_color(muted)
            .child(tr(cx, "work:task.assign"))
            .into_any_element();
        let assignees = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .children(badges)
            .child(self.render_choices("task-assignees", assign, people, Self::toggle_assignee, None, cx));

        let blockers = (!sheet.blockers.is_empty()).then(|| {
            property(
                rail_label(Lucide::Link2, tr(cx, "work:task.blockedBy"), cx),
                div()
                    .id("task-blockers")
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .children(sheet.blockers.iter().map(|blocker| self.render_blocker(blocker, cx)))
                    .into_any_element(),
            )
        });

        // A quien está en otra hora que la organización se le dice de quién
        // es el día: aquí sí cabe escrito.
        let zone = Backend::core(cx).my_work().zone_hint(locale(cx));
        // El botón de `DatePicker.tsx` (`flex items-center gap-1.5 rounded
        // border px-2 py-1 text-xs`, el calendario `size-3.5` delante y el día
        // con números, `30/10/26`; sin día, apagado). Lo que se abre al
        // pulsarlo es el calendario del kit, que va encima y sin pintar: su
        // campo es un `input` con el icono detrás, y aquí no se le ve más
        // que el desplegable. El día lo dice el selector, que ya enseña el
        // elegido mientras se guarda; cómo se escribe, el core.
        let picked = match self.due.read(cx).date() {
            Date::Single(day) => day,
            _ => None,
        };
        let due_label = picked.map(|day| cac_core::short_date(locale(cx), day)).unwrap_or_else(|| tr(cx, "common:last.pickADate"));
        let unseen = gpui_kit::transparent_black();
        let due_button = div()
            .id("task-due")
            .relative()
            .flex_none()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(border)
                    .text_xs()
                    .when(picked.is_none(), |this| this.text_color(muted))
                    .child(Icon::new(Lucide::CalendarDays).size_3p5().flex_none())
                    .child(
                        div()
                            .id("task-due-day")
                            .aria_label(due_label.clone())
                            .child(due_label)
                            .test_support(),
                    ),
            )
            // Más ancho que el botón y recortado a él: así el icono que el
            // campo del kit lleva a la derecha cae fuera de lo que se ve.
            .child(
                div().absolute().top_0().left_0().size_full().overflow_hidden().child(
                    div().w(px(480.)).h_full().child(DatePicker::new(&self.due).small().appearance(false).placeholder(" ").h_full().text_color(unseen)),
                ),
            )
            .test_support();
        // Quitar la fecha: en la app de siempre es un botón dentro del
        // calendario («Sin fecha»); el del kit no lo tiene, así que va al lado.
        let clear_due = picked.is_some().then(|| {
            let hint = tr(cx, "common:last.clearDate");
            div()
                .id("task-due-clear")
                .flex_none()
                .p_1()
                .rounded(px(4.))
                .text_color(muted)
                .cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .on_click(cx.listener(|this, _, window, cx| this.set_due(None, window, cx)))
                .child(Icon::new(Lucide::X).size_3())
                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                .test_support()
        });
        let due = div()
            .flex()
            .flex_col()
            .child(div().flex().items_center().gap_1().child(due_button).children(clear_due))
            .children(zone.map(|zone| div().id("task-due-zone").mt_1().text_size(px(11.)).text_color(muted).child(format!("({zone})"))));

        div()
            .id("task-properties")
            .test_support()
            .flex()
            .flex_col()
            .gap_3()
            .text_sm()
            // El chip del estado, el de la prioridad y la fila de etiquetas
            // van en la app de siempre dentro de un renglón de texto, que
            // mide más que ellos (24 px los dos chips, 2 px de más la fila):
            // de ahí el relleno, que deja cada rótulo donde cae allí.
            .child(property(rail_label(IconName::Check, tr(cx, "work:task.status"), cx), div().pb(px(2.)).child(status).into_any_element()))
            // Una subtarea no se mueve de lista: vive en la de su madre.
            .when(sheet.parent.is_none(), |this| this.child(property(rail_label(Lucide::FolderInput, tr(cx, "work:task.list"), cx), list.into_any_element())))
            .child(property(rail_label(Lucide::Flag, tr(cx, "work:board.priority"), cx), div().pt(px(3.)).pb(px(1.)).child(priority).into_any_element()))
            .child(property(rail_label(Lucide::Tag, tr(cx, "work:task.tags"), cx), tags.pb(px(2.)).into_any_element()))
            .child(property(rail_label(Lucide::Users, tr(cx, "work:task.assignees"), cx), assignees.into_any_element()))
            .children(blockers)
            .child(property(rail_label(Lucide::Calendar, tr(cx, "work:task.due"), cx), due.into_any_element()))
    }

    /// Una subtarea, con las medidas de su fila en `TaskDetailDrawer.tsx`
    /// (`px-2 py-1.5 text-sm`, la casilla `size-4 rounded border`, en verde
    /// al 20% cuando está hecha, y su folio en mono al final).
    fn render_subtask(&self, subtask: &SheetSubtask, first: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, success, mono) = (theme.muted_foreground, theme.border, theme.success, theme.mono_font_family.clone());
        let hint = tr(cx, if subtask.done { "work:task.reopen" } else { "work:task.markComplete" });
        div()
            .id(SharedString::from(format!("subtask-{}", subtask.id)))
            .test_support()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1p5()
            .text_sm()
            .when(!first, |this| this.border_t_1().border_color(border))
            .child(
                div()
                    .id(SharedString::from(format!("subtask-tick-{}", subtask.id)))
                    .size_4()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if subtask.done { success } else { border })
                    .when(subtask.done, |this| this.bg(success.opacity(0.2)))
                    .cursor_pointer()
                    .on_click({
                        let (id, done) = (subtask.id.clone(), subtask.done);
                        cx.listener(move |this, _, window, cx| this.tick_subtask(id.clone(), !done, window, cx))
                    })
                    .children(subtask.done.then(|| {
                        div()
                            .id(SharedString::from(format!("subtask-done-{}", subtask.id)))
                            .test_support()
                            .child(Icon::new(IconName::Check).size_3().text_color(success))
                    }))
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                    .test_support(),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .when(subtask.done, |this| this.text_color(muted).line_through())
                    .child(subtask.title.clone()),
            )
            .child(div().flex_none().font_family(mono).text_xs().text_color(muted).child(format!("#{}", subtask.seq)))
    }

    /// Un comentario: la tarjeta de `CommentItem` (`rounded-md border
    /// p-2.5`; lo interno en una tarea que lee un cliente, con el borde a
    /// trazos y el fondo apagado). Arriba quién, cuándo y para quién; lo de
    /// editar y borrar sale al pasar el ratón.
    fn render_comment(&self, comment: &SheetComment, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground, border, primary, soft) = (theme.muted_foreground, theme.foreground, theme.border, theme.primary, theme.muted);
        let id = SharedString::from(format!("comment-{}", comment.id));
        let group = SharedString::from(format!("comment-card-{}", comment.id));
        let editing = self.editing_comment.as_deref() == Some(comment.id.as_str());
        let internal = comment.audience.as_deref() == Some("work:task.internal");
        let audience = comment.audience.as_ref().map(|key| {
            let hint = tr(cx, if internal { "work:task.onlyTeam" } else { "work:task.clientCanRead" });
            div()
                .id(SharedString::from(format!("comment-audience-{}", comment.id)))
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .text_color(if internal { muted } else { primary })
                .child(Icon::new(if internal { Lucide::EyeOff } else { Lucide::Eye }).size_3())
                .child(tr(cx, key))
                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
        });
        div()
            .id(id)
            .test_support()
            .group(group.clone())
            .flex()
            .flex_col()
            .p_2p5()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .when(internal, |this| this.border_dashed().bg(soft.opacity(0.4)))
            .child(
                div()
                    .mb_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(muted)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(foreground)
                            // Lo anotó un agente: lleva su icono, y el
                            // nombre ya dice de quién es.
                            .when(comment.agent, |this| this.child(Icon::new(Lucide::Bot).size_3()))
                            .child(comment.author.clone()),
                    )
                    .child(div().flex_none().child(comment.when.clone()))
                    .children(comment.edited.then(|| div().flex_none().italic().child(tr(cx, "work:task.edited"))))
                    .children(audience)
                    // Sólo en lo que quien mira puede cambiar: el servidor
                    // rechazaría lo demás.
                    .children((comment.mine && !editing).then(|| {
                        let (edit_id, body, delete_id) = (comment.id.clone(), comment.body.clone(), comment.id.clone());
                        div()
                            .ml_auto()
                            .flex()
                            .flex_none()
                            .opacity(0.)
                            .group_hover(group, |style| style.opacity(1.))
                            .child(
                                Button::new(SharedString::from(format!("comment-edit-{}", comment.id)))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(Lucide::Pencil))
                                    .tooltip(tr(cx, "work:task.editComment"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.start_editing_comment(edit_id.clone(), body.clone(), window, cx)
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("comment-delete-{}", comment.id)))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(Lucide::Trash))
                                    .tooltip(tr(cx, "work:task.deleteComment"))
                                    .on_click(cx.listener(move |this, _, window, cx| this.delete_comment(delete_id.clone(), window, cx))),
                            )
                    })),
            )
            .child(if editing {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.render_draft("comment-edit", Draft::CommentEdit, cx))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("comment-edit-save")
                                    .primary()
                                    .xsmall()
                                    .h_7()
                                    .px_2p5()
                                    .label(tr(cx, "work:task.save"))
                                    // `disabled={saving || !draft.trim()}`.
                                    .disabled(!cac_core::says_something(self.comment_edit.read(cx).value().as_ref()))
                                    .on_click(cx.listener(|this, _, window, cx| this.save_comment_edit(window, cx))),
                            )
                            .child(
                                Button::new("comment-edit-cancel")
                                    .outline()
                                    .xsmall()
                                    .h_7()
                                    .px_2p5()
                                    .label(tr(cx, "work:task.cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_editing_comment(cx))),
                            ),
                    )
                    .into_any_element()
            } else {
                self.prose(format!("comment-body-{}", comment.id), comment.body.clone(), cx).into_any_element()
            })
    }

    /// La columna de lectura, en el orden y con las medidas de
    /// `TaskDetailDrawer.tsx` (`space-y-5`; cada sección, su rótulo y lo
    /// suyo a `space-y-2`): título, dónde quedó, descripción, adjuntos,
    /// subtareas y el hilo con su caja.
    fn render_sheet(&mut self, sheet: TaskSheet, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let danger = theme.danger;
        let problem = self.problem.as_ref().filter(|(task_id, _)| *task_id == sheet.id).map(|(_, phrase)| phrase.clone());
        let (muted, border, primary, foreground) = (theme.muted_foreground, theme.border, theme.primary, theme.foreground);
        // La pestaña elegida era de otra tarea: en ésta se empieza por la
        // conversación. Y sin notas de agentes no hay pestañas que elegir.
        let chosen = if self.thread_of.as_deref() == Some(sheet.id.as_str()) { self.thread } else { Thread::Talk };
        let thread = if sheet.notes.is_empty() { Thread::Talk } else { chosen };
        let editing = self.editing.as_deref() == Some(sheet.id.as_str());
        let done = sheet.subtasks.iter().filter(|subtask| subtask.done).count();
        let comments = match thread {
            Thread::Talk => &sheet.talk,
            Thread::Notes => &sheet.notes,
        };
        let tabs = [
            (Thread::Talk, "thread-talk", "work:task.thread.talk", sheet.talk.len()),
            (Thread::Notes, "thread-notes", "work:task.thread.agentNotes", sheet.notes.len()),
        ];

        let title = if editing {
            div().id("task-title-input").test_support().child(Input::new(&self.title_input)).into_any_element()
        } else {
            // `text-lg font-semibold`: el título de una tarea no es el de
            // una pantalla.
            div()
                .id("task-title")
                .test_support()
                // El `textarea` del título: `text-lg` con su interlínea de
                // 28 px, 2 px de relleno arriba y abajo, y el hueco que una
                // caja en línea deja bajo su renglón.
                .pt(px(2.))
                .pb(px(5.))
                .text_lg()
                .line_height(px(28.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(sheet.title.clone())
                .into_any_element()
        };

        // Dónde quedó: siempre a la vista, también sin nada escrito, que es
        // cuando hay que decir que falta.
        let handoff = div()
            .id("task-handoff")
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(section_title(tr(cx, "work:task.handoff"), cx))
                    // Escribirlo o corregirlo desde aquí todavía no está.
                    .child(
                        section_action("task-handoff-edit", tr(cx, if sheet.handoff.is_some() { "work:task.edit" } else { "work:task.writeHandoff" }), cx)
                            .tooltip(not_yet(cx))
                            .test_support(),
                    )
                    .children(sheet.handoff_signature.clone().map(|signature| div().min_w_0().truncate().text_xs().text_color(muted).child(signature))),
            )
            .child(match sheet.handoff.clone() {
                Some(handoff) => self.prose(format!("handoff-{}", sheet.id), handoff, cx).into_any_element(),
                None => div().id("task-no-handoff").test_support().text_sm().text_color(muted).child(tr(cx, "work:task.noHandoff")).into_any_element(),
            });

        let description = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div().flex().items_center().gap_2().child(section_title(tr(cx, "work:task.description"), cx)).when(!editing, |this| {
                    this.child(
                        section_action("task-edit", tr(cx, "work:task.edit"), cx)
                            .on_click(cx.listener(|this, _, window, cx| this.start_editing(window, cx)))
                            .test_support(),
                    )
                }),
            )
            .child(if editing {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.render_draft("task-description", Draft::Description, cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("task-save")
                                    .primary()
                                    .xsmall()
                                    .h_7()
                                    .px_2p5()
                                    .label(tr(cx, "work:task.save"))
                                    .on_click(cx.listener(|this, _, window, cx| this.save_edit(window, cx))),
                            )
                            .child(
                                Button::new("task-cancel")
                                    .outline()
                                    .xsmall()
                                    .h_7()
                                    .px_2p5()
                                    .label(tr(cx, "work:task.cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_editing(cx))),
                            ),
                    )
                    .into_any_element()
            } else if sheet.description.is_empty() {
                div().id("task-no-description").test_support().text_sm().text_color(muted).child(tr(cx, "work:task.noDescription")).into_any_element()
            } else {
                div()
                    .id("task-description")
                    .test_support()
                    .child(self.prose(format!("description-{}", sheet.id), sheet.description.clone(), cx))
                    .into_any_element()
            });

        // Sólo cuando hay alguno, como en la app de siempre (`detail.
        // attachments.length > 0`): se adjunta desde el clip de cada editor.
        let attachments = (!sheet.attachments.is_empty()).then(|| {
            div()
                .id("task-attachments")
                .test_support()
                .flex()
                .flex_col()
                .gap_2()
                .child(section_title(tr(cx, "work:task.attachments"), cx))
                // Las imágenes se ven, no se listan; lo demás, debajo.
                .when(sheet.attachments.iter().any(|file| file.image), |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .children(sheet.attachments.iter().filter(|file| file.image).map(|file| self.render_picture(file, cx))),
                    )
                })
                .when(sheet.attachments.iter().any(|file| !file.image), |this| {
                    this.child(div().flex().flex_col().gap_1().children(sheet.attachments.iter().filter(|file| !file.image).map(|file| self.render_file(file, cx))))
                })
        });

        // Enlazar una PR pegando su dirección: la línea discreta que
        // `TaskGitPanel.tsx` pinta bajo las subtareas cuando la tarea no
        // tiene código enlazado (`flex items-center gap-1.5 text-xs
        // text-muted-foreground`, el icono `size-3.5`). Todavía no está aquí.
        let link_pr = (!sheet.git_linked).then(|| {
            div().flex().child(
                div()
                    .id("task-link-pr")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .text_color(muted)
                    .hover(move |style| style.text_color(foreground))
                    .child(Icon::new(Lucide::GitPullRequest).size_3p5())
                    .child(tr(cx, "work:task.git.linkPR"))
                    .tooltip(not_yet(cx))
                    .test_support(),
            )
        });

        // Las subtareas: el rótulo con cuántas van hechas, cómo añadir otra
        // y, sin ninguna, para qué sirven.
        let count = (!sheet.subtasks.is_empty()).then(|| format!("({done}/{})", sheet.subtasks.len()));
        let subtasks = div()
            .id("task-subtasks")
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(section_title(tr(cx, "work:task.subtasks"), cx))
                            .children(count.map(|count| div().text_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(count))),
                    )
                    // Añadir una desde aquí todavía no está.
                    .child(section_action("task-add-subtask", tr(cx, "work:task.addSubtask"), cx).tooltip(not_yet(cx)).test_support()),
            )
            .child(if sheet.subtasks.is_empty() {
                div().id("task-no-subtasks").test_support().text_xs().text_color(muted).child(tr(cx, "work:task.noSubtasks")).into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .children(sheet.subtasks.iter().enumerate().map(|(index, subtask)| self.render_subtask(subtask, index == 0, cx)))
                    .into_any_element()
            })
            .test_support();

        // El hilo: sin notas de agentes es un rótulo con cuántos van; con
        // ellas, dos pestañas (`gap-4 border-b`, cada una `border-b-2 pb-1.5
        // text-xs font-medium uppercase`, la de notas con su robot).
        let thread_head = if sheet.notes.is_empty() {
            div().id("thread-title").test_support().child(section_title(format!("{} ({})", tr(cx, "work:task.thread.talk"), sheet.talk.len()), cx)).into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap_4()
                .border_b_1()
                .border_color(border)
                .children(tabs.map(|(tab, id, key, count)| {
                    let active = thread == tab;
                    let task_id = sheet.id.clone();
                    div()
                        .id(id)
                        .test_support()
                        .mb(px(-1.))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .pb_1p5()
                        .border_b_2()
                        .border_color(if active { primary } else { primary.opacity(0.) })
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if active { foreground } else { muted })
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.show_thread(tab, task_id.clone(), cx)))
                        .when(tab == Thread::Notes, |this| this.child(Icon::new(Lucide::Bot).size_3p5()))
                        .child(format!("{} ({count})", tr(cx, key).to_uppercase()))
                }))
                .into_any_element()
        };
        let thread_section = div()
            .id("comments")
            .flex()
            .flex_col()
            .gap_3()
            .child(thread_head)
            .when(thread == Thread::Notes, |this| {
                this.child(div().text_xs().text_color(muted).child(tr(cx, "work:task.thread.agentNotesHint")))
            })
            .children(comments.iter().map(|comment| self.render_comment(comment, cx)))
            // Se comenta en la conversación; las notas de agentes son suyas.
            .when(thread == Thread::Talk, |this| this.child(self.render_composer(sheet.client_reads, cx)));

        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(title)
            .children(problem.map(|phrase| div().id("task-problem").test_support().text_sm().text_color(danger).child(phrase)))
            // «Dónde quedó» y la descripción van seguidas, sin el hueco de
            // las demás secciones: en `TaskDetailDrawer.tsx` las dos viven
            // dentro de la misma zona para soltar ficheros (`FileDropZone`),
            // que es un solo hijo del `space-y-5`.
            .child(div().flex().flex_col().child(handoff).child(description))
            .children(attachments)
            .child(subtasks)
            .children(link_pr)
            .child(thread_section)
    }
}

impl Render for TaskView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).task.clone();
        // Al abrirse una tarea el foco pasa a su página: lo tenía la lista de
        // debajo, y sin él Escape no llega aquí.
        if self.focused_for != state.task_id {
            self.focused_for = state.task_id.clone();
            if state.task_id.is_some() {
                self.page_focus.focus(window, cx);
            }
        }
        let sheet = Backend::core(cx).tasks().sheet(locale(cx));
        if let Some(sheet) = &sheet {
            self.show_due(sheet, window, cx);
            self.show_thumbs(sheet, window, cx);
            // La caja promete el `@` sólo donde nombra a alguien: en lo que
            // lee un cliente dice quién lo lee.
            let promise = cac_core::comment_placeholder(sheet.client_reads, self.internal);
            self.comment_promise = promise;
            // Lo recordado de la lista es de un `@`: cuando deja de
            // escribirse, se olvida.
            if self.mention_mark.is_some() {
                let (text, cursor) = self.comment_cursor(cx);
                self.mention_mark = cac_core::mention_mark_kept(&text, cursor, self.mention_mark.take());
            }
        }
        let theme = cx.theme();
        let (background, foreground, muted, border, primary, danger) =
            (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.primary, theme.danger);
        let hover = crate::theme::ghost_hover(cx);
        let waiting = state.waiting_reason().map(|key| tr(cx, key));

        // La cabecera de `TaskDetailDrawer.tsx` (`h-12 gap-2 border-b
        // px-4`): dónde vive (y de quién cuelga), su referencia para copiar,
        // si un cliente la lee, su id para dárselo a un agente y la ×.
        let head = sheet.as_ref().map(|sheet| {
            let place = div()
                .min_w_0()
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_xs()
                .text_color(muted)
                .child(div().flex_none().child(sheet.place.clone()))
                .children(sheet.parent.clone().zip(sheet.parent_id.clone()).map(|(parent, id)| {
                    let hint = tr(cx, "work:task.openParent");
                    div().flex().min_w_0().child(div().flex_none().child("\u{a0}/\u{a0}")).child(
                        div()
                            .id("task-parent")
                            .min_w_0()
                            .truncate()
                            .underline()
                            .cursor_pointer()
                            .hover(move |style| style.text_color(foreground))
                            .on_click(move |_, _, cx| Self::open_task(id.clone(), cx))
                            .child(parent)
                            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                            .test_support(),
                    )
                }))
                // Sin referencia (un servidor de antes), el número a secas.
                .when(sheet.reference.is_none(), |this| this.child(div().flex_none().child(format!("\u{a0}· #{}", sheet.seq))));
            let reference = sheet.reference.clone().map(|reference| self.render_copy_chip("task-ref", reference.clone(), reference, cx).into_any_element());
            // Que un cliente la lee se dice aquí arriba: quien va a escribir
            // una nota franca tiene que saberlo antes de escribirla.
            let visible = sheet.client_reads.then(|| {
                div()
                    .id("task-visible-to-client")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(4.))
                    .bg(primary.opacity(0.1))
                    .text_xs()
                    .text_color(primary)
                    .child(Icon::new(Lucide::Eye).size_3())
                    .child(tr(cx, "work:task.visibleToClient"))
                    .test_support()
            });
            let short: String = sheet.id.chars().take(8).collect();
            let task_id = self.render_copy_chip("task-id", sheet.id.clone(), short, cx).into_any_element();
            (place, reference, visible, task_id)
        });
        // El pie (`border-t px-4 py-2`): borrar la tarea, retirarla de la
        // vista del cliente y cuándo se tocó por última vez. Borrar y
        // retirar todavía no están en esta app.
        let foot = sheet.as_ref().map(|sheet| {
            let action = |id: &'static str, icon: Lucide, label: String, tone: Hsla| {
                div()
                    .id(id)
                    .h_7()
                    .px_2p5()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .rounded(px(8.))
                    // `Button size="sm"`: `text-[0.8rem]`, y el icono con
                    // su `mr-1` además del `gap-1`.
                    .text_size(rems(0.8))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(tone)
                    .hover(move |style| style.bg(hover))
                    .child(Icon::new(icon).size_3().mr_1())
                    .child(label)
                    .tooltip(not_yet(cx))
                    .test_support()
            };
            div()
                .id("task-foot")
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(border)
                .child(action("task-delete", Lucide::Trash, tr(cx, "work:task.deleteTask"), danger))
                .when(sheet.client_reads, |this| this.child(action("task-withdraw", Lucide::EyeOff, tr(cx, "work:task.withdraw"), foreground)))
                .child(div().id("task-updated").ml_auto().text_xs().text_color(muted).child(sheet.updated.clone()).test_support())
        });

        // Ancha, el riel al lado; estrecha, debajo de la lectura.
        let wide = window.viewport_size().width >= px(STACK_BELOW);
        let statuses = state.statuses.clone();
        let properties = sheet.as_ref().map(|sheet| self.render_properties(sheet, statuses, cx).into_any_element());
        let reading = div()
            .flex()
            .flex_col()
            .gap_4()
            .children(waiting.map(|phrase| div().id("task-waiting").test_support().text_sm().text_color(muted).child(phrase)))
            .children(sheet.map(|sheet| self.render_sheet(sheet, cx)));
        let body = if wide {
            div()
                .id("task-wide")
                .test_support()
                .flex_1()
                .min_h_0()
                .flex()
                .overflow_hidden()
                .child(div().id("task-scroll").flex_1().min_w_0().h_full().overflow_y_scroll().px_8().py_4().child(reading))
                .children(properties.map(|properties| {
                    div()
                        .id("task-rail")
                        .w(px(RAIL_WIDTH))
                        .h_full()
                        .flex_none()
                        .overflow_y_scroll()
                        .p_4()
                        .border_l_1()
                        .border_color(border)
                        .child(properties)
                }))
                .into_any_element()
        } else {
            div()
                .id("task-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .child(div().p_4().child(reading))
                .children(properties.map(|properties| div().p_4().border_t_1().border_color(border).child(properties)))
                .into_any_element()
        };

        let (place, reference, visible, task_id) = match head {
            Some((place, reference, visible, task_id)) => (Some(place), reference, visible, Some(task_id)),
            None => (None, None, None, None),
        };
        div()
            .id("task-page")
            .test_support()
            .track_focus(&self.page_focus)
            // Escape la cierra, salvo escribiendo: ahí cancela lo que se
            // escribe, y llevarse la pantalla entera lo perdería. Quien
            // escribe tiene el foco en su campo, no en la página.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && this.page_focus.is_focused(window) {
                    Backend::core(cx).tasks().close();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .h(px(48.))
                    .px_4()
                    .border_b_1()
                    .border_color(border)
                    .children(place)
                    .children(reference)
                    .children(visible)
                    .children(task_id)
                    .child(div().flex_1())
                    .child(
                        // `Button size="icon-xs"` (`size-6`) con la × a
                        // `size-4`: como hijo y no como icono del botón,
                        // que le pondría el tamaño de su talla.
                        Button::new("task-close")
                            .ghost()
                            .xsmall()
                            .size_6()
                            .p_0()
                            .child(Icon::new(Lucide::X).size_4())
                            .tooltip(tr(cx, "work:task.close"))
                            .on_click(|_, _, cx| Backend::core(cx).tasks().close()),
                    ),
            )
            .child(body)
            .children(foot)
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::TaskView;
    use crate::backend::Backend;
    use crate::testing::{contract_task, ok, server_with_tasks, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use gpui_kit::component::calendar::Date;
    use gpui_kit::component::date_picker::DatePickerEvent;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::Mock;

    /// El detalle del contrato del core, con otro id y un hilo de dos: lo
    /// que dijo una persona y lo que anotó un agente.
    fn detail(id: &str) -> serde_json::Value {
        let mut detail: serde_json::Value =
            serde_json::from_str(include_str!("../../core/tests/fixtures/task_detail.json")).unwrap();
        let note = detail["comments"][0].clone();
        let mut said = note.clone();
        said["id"] = json!("said");
        said["via"] = serde_json::Value::Null;
        let mut noted = note;
        noted["id"] = json!("noted");
        detail["comments"] = json!([said, noted]);
        detail["task"]["id"] = json!(id);
        detail
    }

    /// Dos tareas en la lista; la primera con su detalle y las columnas de
    /// su lista (la `s1`, donde está su subtarea, es de «hecho»). El detalle
    /// de la segunda no existe: el servidor contesta 404.
    fn server() -> FakeServer {
        server_with_column("done")
    }

    fn server_with_column(kind: &str) -> FakeServer {
        let server = server_with_tasks(vec![contract_task("t1", "a", |_| {}), contract_task("t2", "a", |_| {})]);
        let first = detail("t1");
        let mut column = first["status"].clone();
        column["kind"] = json!(kind);
        column["listId"] = first["task"]["listId"].clone();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(first)));
        server.mount(
            Mock::given(method("GET")).and(path_regex("^/api/v1/task-lists/[^/]+/statuses$")).respond_with(ok(json!([column]))),
        );
        server
    }

    fn open(server: &FakeServer, cx: &mut TestAppContext) -> AnyWindowHandle {
        open_with_view(server, cx).0
    }

    fn open_with_view(server: &FakeServer, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TaskView>) {
        open_sized(server, 1700., cx)
    }

    fn open_sized(server: &FakeServer, width: f32, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TaskView>) {
        server.install(cx);
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(width), px(900.)),
                })),
                ..WindowOptions::default()
            };
            let (window, workspace) =
                gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window");
            (window, workspace.read(cx).task.clone())
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

    /// Abre la primera tarea y espera a que estén su detalle y sus columnas.
    fn open_first_task(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llegan el detalle y sus columnas", |cx| {
            cx.update(|cx| !Backend::snapshot(cx).read(cx).task.statuses.is_empty()) && on_screen(window, "task-title", cx)
        });
    }

    /// Abre la segunda tarea con la primera aún abierta, como hace un
    /// enlace o un aviso: su fila queda debajo de la página y no se pulsa.
    fn open_other_task(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.tasks().open("t2").await }));
    }

    fn move_to(window: AnyWindowHandle, view: &Entity<TaskView>, status: &str, cx: &mut TestAppContext) {
        let status = status.to_string();
        cx.update_window(window, |_, window, cx| view.update(cx, |view, cx| view.move_to(status, window, cx))).unwrap();
    }

    fn moves(server: &FakeServer) -> Vec<String> {
        server
            .requests()
            .iter()
            .filter(|request| request.url.path() == "/api/v1/tasks/t1/move")
            .map(|request| String::from_utf8_lossy(&request.body).to_string())
            .collect()
    }

    fn click(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Pulsar una tarea abre su detalle sobre la lista, con lo suyo, y la ×
    /// lo cierra.
    ///
    /// Roturas que atrapa: una fila que no abre nada; un detalle a un lado
    /// que deja pulsar la lista de debajo (es una página: lo tapa todo); una
    /// subtarea hecha pintada sin hacer; no tener cómo cerrarlo.
    #[gpui_kit::test]
    fn clicking_a_task_opens_its_detail_over_the_list(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        assert!(!on_screen(window, "task-page", cx), "el panel sale sin ninguna tarea abierta");

        click(window, "work-row-t1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "task-title", cx));

        assert!(!on_screen(window, "task-waiting", cx));
        assert!(on_screen(window, "task-description", cx) && on_screen(window, "task-handoff", cx));
        assert!(on_screen(window, "task-status", cx) && on_screen(window, "subtask-t2", cx));
        wait_until(cx, "llegan las columnas y la subtarea sale hecha", |cx| on_screen(window, "subtask-done-t2", cx));
        // La página tapa la lista: pulsar donde está otra fila no abre otra.
        click(window, "work-row-t2", cx);
        settle(cx);
        assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.clone()).as_deref(), Some("t1"), "se pulsó la lista a través de la página");

        click(window, "task-close", cx);
        wait_until(cx, "se cierra", |cx| !on_screen(window, "task-page", cx));
        stop(cx);
    }

    /// Escape cierra la página, pero no a quien está escribiendo.
    ///
    /// Roturas que atrapa: una página a pantalla completa cuya única salida
    /// es un botón pequeño; que Escape se lleve la pantalla con un comentario
    /// a medio escribir.
    #[gpui_kit::test]
    fn escape_closes_the_page_unless_someone_is_typing(cx: &mut TestAppContext) {
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        let press_key = |key: &'static str, cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.press(key, cx);
            })
            .unwrap();
        };
        let press = |cx: &mut TestAppContext| press_key("escape", cx);
        let open_task = |cx: &mut TestAppContext| cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.clone());

        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "comment-input", cx));
        // Recién abierta, sin pulsar nada dentro: otra tecla no la cierra, y
        // Escape sí.
        press_key("a", cx);
        settle(cx);
        assert_eq!(open_task(cx).as_deref(), Some("t1"), "cualquier tecla cierra la página");
        press(cx);
        wait_until(cx, "se cierra sin haber pulsado nada dentro", |cx| !on_screen(window, "task-page", cx));

        click(window, "work-row-t1", cx);
        wait_until(cx, "vuelve a abrirse", |cx| on_screen(window, "comment-input", cx));
        write(window, "a medias", cx);
        press(cx);
        settle(cx);
        assert_eq!(open_task(cx).as_deref(), Some("t1"), "Escape cerró la página a quien escribía");
        assert_eq!(typed(&view, cx), "a medias");

        // Fuera del campo: el foco vuelve a la página.
        click(window, "task-title", cx);
        press(cx);
        wait_until(cx, "se cierra", |cx| !on_screen(window, "task-page", cx));
        stop(cx);
    }

    /// Elegir una columna le pide al servidor mover la tarea a ésa.
    ///
    /// Roturas que atrapa: un desplegable que no manda nada; mandar otra
    /// columna; decir que falló cuando salió bien.
    #[gpui_kit::test]
    fn choosing_a_column_asks_the_server_to_move_the_task(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t1/move")).respond_with(ok(json!(null))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(moves(&server).is_empty());

        move_to(window, &view, "s1", cx);
        wait_until(cx, "llega la petición de mover", |_| !moves(&server).is_empty());

        assert!(moves(&server)[0].contains("\"statusId\":\"s1\""), "mandó {:?}", moves(&server));
        cx.run_until_parked();
        assert!(!on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// Un cambio de estado que el servidor rechaza se dice en el cajón, con
    /// su frase.
    ///
    /// Rotura que atrapa: un desplegable que parece no hacer nada cuando el
    /// cambio no se guardó.
    #[gpui_kit::test]
    fn a_move_that_fails_is_said_in_the_drawer(cx: &mut TestAppContext) {
        let server = server();
        // La primera vez lo rechaza; la segunda lo acepta.
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/tasks/t1/move"))
                .respond_with(
                    wiremock::ResponseTemplate::new(409)
                        .set_body_json(json!({ "success": false, "message": "no", "error": "bad-transition" })),
                )
                .up_to_n_times(1),
        );
        server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t1/move")).respond_with(ok(json!(null))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(!on_screen(window, "task-problem", cx));

        move_to(window, &view, "s1", cx);
        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));

        // Al volver a intentarlo y salir bien, el aviso de antes se quita.
        move_to(window, &view, "s1", cx);
        wait_until(cx, "el segundo intento sale bien", |cx| moves(&server).len() == 2 && !on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// Lo que no se pudo guardar en una tarea no se dice sobre otra.
    ///
    /// Rotura que atrapa: abrir una tarea y leer el fallo de la anterior.
    #[gpui_kit::test]
    fn a_failure_is_not_said_over_another_task(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        // Sin ruta de mover montada: el servidor contesta 404.
        move_to(window, &view, "s1", cx);
        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));

        open_other_task(cx);
        wait_until(cx, "llega la otra", |cx| {
            cx.update(|cx| Backend::snapshot(cx).read(cx).task.detail.as_ref().is_some_and(|detail| detail.task.id == "t2"))
        });

        assert!(!on_screen(window, "task-problem", cx));
        stop(cx);
    }

    fn bodies(server: &FakeServer, route: &str) -> Vec<String> {
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() == "POST" && request.url.path() == route)
            .map(|request| String::from_utf8_lossy(&request.body).to_string())
            .collect()
    }

    fn typed(view: &Entity<TaskView>, cx: &mut TestAppContext) -> String {
        cx.update(|cx| view.read(cx).comment.read(cx).value().to_string())
    }

    fn write(window: AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("comment-input", cx);
            window.input(text, cx);
        })
        .unwrap();
    }

    /// El servidor de siempre, con lo que la organización de la tarea ofrece
    /// (otra etiqueta y otra persona) y un `PATCH` que contesta la tarea.
    fn server_with_choices() -> FakeServer {
        let server = server();
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/task-tags/"))
                .respond_with(ok(json!([{ "id": "tag2", "orgId": "o1", "name": "backend", "color": "#0f0" }]))),
        );
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/users/search"))
                .respond_with(ok(json!([{ "id": "u2", "username": "luis" }]))),
        );
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(detail("t1"))));
        server
    }

    fn patches(server: &FakeServer) -> Vec<serde_json::Value> {
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() == "PATCH" && request.url.path() == "/api/v1/tasks/t1")
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect()
    }

    /// Hace algo con la vista del cajón, en su ventana.
    fn with_view(
        window: AnyWindowHandle,
        view: &Entity<TaskView>,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut TaskView, &mut gpui_kit::Window, &mut gpui_kit::Context<TaskView>),
    ) {
        cx.update_window(window, |_, window, cx| view.update(cx, |view, cx| act(view, window, cx))).unwrap();
    }

    fn day(year: i32, month: u32, day: u32) -> Date {
        Date::Single(chrono::NaiveDate::from_ymd_opt(year, month, day))
    }

    fn picked(view: &Entity<TaskView>, cx: &mut TestAppContext) -> Date {
        cx.update(|cx| view.read(cx).due.read(cx).date())
    }

    /// Elige un día en el selector, como quien lo pulsa en el calendario.
    fn pick(window: AnyWindowHandle, view: &Entity<TaskView>, date: Date, cx: &mut TestAppContext) {
        let due = cx.update(|cx| view.read(cx).due.clone());
        cx.update_window(window, |_, window, cx| {
            due.update(cx, |due, cx| {
                due.set_date(date, window, cx);
                cx.emit(DatePickerEvent::Change(due.date_time()));
            })
        })
        .unwrap();
    }

    /// Cada selector del cajón le pide al servidor su cambio, y sólo el suyo.
    ///
    /// Roturas que atrapa: un selector que no manda nada; la prioridad que
    /// manda otra; poner una etiqueta como responsable o al revés; quitar la
    /// fecha cuando se quería poner.
    #[gpui_kit::test]
    fn each_field_asks_for_its_own_change(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        wait_until(cx, "llega lo que se puede elegir", |cx| {
            cx.update(|cx| !Backend::snapshot(cx).read(cx).task.team.is_empty())
        });
        for id in ["task-priority", "task-tags", "task-assignees", "task-due"] {
            assert!(on_screen(window, id, cx), "falta {id}");
        }
        assert!(patches(&server).is_empty(), "abrir la tarea ya guardó algo: {:?}", patches(&server));

        let steps: [(Box<dyn FnOnce(&mut TaskView, &mut gpui_kit::Window, &mut gpui_kit::Context<TaskView>)>, serde_json::Value); 5] = [
            (Box::new(|view, window, cx| view.set_priority("urgent".into(), window, cx)), json!({ "priority": "urgent" })),
            (Box::new(|view, window, cx| view.toggle_tag("tag2".into(), window, cx)), json!({ "tagIds": ["tag1", "tag2"] })),
            (Box::new(|view, window, cx| view.toggle_assignee("u2".into(), window, cx)), json!({ "assigneeIds": ["u1", "u2"] })),
            (Box::new(|view, window, cx| view.set_due(Some("2026-10-02".into()), window, cx)), json!({ "dueAt": "2026-10-02T00:00:00Z" })),
            (Box::new(|view, window, cx| view.set_due(None, window, cx)), json!({ "clearDueAt": true })),
        ];
        for (at, (act, sent)) in steps.into_iter().enumerate() {
            with_view(window, &view, cx, act);
            wait_until(cx, "llega el cambio", |_| patches(&server).len() == at + 1);
            assert_eq!(patches(&server)[at], sent);
        }
        cx.run_until_parked();
        assert!(!on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// El selector de fecha enseña el día de la tarea, y elegir otro lo guarda.
    ///
    /// Roturas que atrapa: un selector vacío sobre una tarea con fecha;
    /// elegir un día y que no se guarde; pisarle lo elegido a quien elige
    /// mientras se guarda; enseñar en otra tarea el día que se eligió en
    /// ésta; guardar por el mero hecho de enseñarlo (un bucle de peticiones).
    #[gpui_kit::test]
    fn the_date_picker_shows_the_due_day_and_saves_another(cx: &mut TestAppContext) {
        let server = server_with_choices();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert_eq!(picked(&view, cx), day(2026, 9, 30));

        pick(window, &view, day(2026, 10, 2), cx);
        wait_until(cx, "se guarda el día elegido", |_| !patches(&server).is_empty());
        assert_eq!(patches(&server), [json!({ "dueAt": "2026-10-02T00:00:00Z" })]);
        // El servidor de mentira contesta la tarea como estaba, así que nada
        // nuevo hay que enseñar: lo elegido sigue en el selector.
        cx.run_until_parked();
        on_screen(window, "task-title", cx);
        assert_eq!(picked(&view, cx), day(2026, 10, 2), "se pisó lo que se acababa de elegir");

        // La otra tarea vence el 30: el selector no se queda con lo de ésta.
        open_other_task(cx);
        wait_until(cx, "llega la otra, con su día", |cx| {
            on_screen(window, "task-title", cx) && picked(&view, cx) == day(2026, 9, 30)
        });
        cx.run_until_parked();
        assert_eq!(patches(&server).len(), 1, "enseñar una fecha guardó: {:?}", patches(&server));
        stop(cx);
    }

    /// Si el día de la tarea cambia, el selector lo sigue.
    ///
    /// Rotura que atrapa: un selector que se queda con el día de cuando se
    /// abrió la tarea, aunque alguien se lo haya quitado.
    #[gpui_kit::test]
    fn the_date_picker_follows_the_task(cx: &mut TestAppContext) {
        let server = server();
        let mut without = detail("t1");
        without["task"]["dueAt"] = serde_json::Value::Null;
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/tasks/t1")).respond_with(ok(without)));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert_eq!(picked(&view, cx), day(2026, 9, 30));

        with_view(window, &view, cx, |view, window, cx| view.set_priority("urgent".into(), window, cx));

        wait_until(cx, "llega la tarea ya sin fecha", |cx| {
            on_screen(window, "task-title", cx) && picked(&view, cx) == Date::Single(None)
        });
        stop(cx);
    }

    /// Un cambio que no se pudo guardar se dice, y el selector de fecha
    /// vuelve al día que la tarea tiene de verdad.
    ///
    /// Roturas que atrapa: un selector que parece no hacer nada cuando el
    /// cambio falló; un selector que se queda enseñando el día que no se
    /// guardó.
    #[gpui_kit::test]
    fn a_change_that_fails_is_said_and_the_date_goes_back(cx: &mut TestAppContext) {
        // Sin `PATCH` montado: el servidor contesta 404.
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(!on_screen(window, "task-problem", cx));

        pick(window, &view, day(2026, 10, 2), cx);

        wait_until(cx, "dice que no se pudo y vuelve al día guardado", |cx| {
            on_screen(window, "task-problem", cx) && picked(&view, cx) == day(2026, 9, 30)
        });
        stop(cx);
    }

    fn written(view: &Entity<TaskView>, cx: &mut TestAppContext) -> (String, String) {
        cx.update(|cx| {
            let view = view.read(cx);
            (view.title_input.read(cx).value().to_string(), view.body_input.read(cx).value().to_string())
        })
    }

    fn rewrite(window: AnyWindowHandle, view: &Entity<TaskView>, title: &str, body: &str, cx: &mut TestAppContext) {
        let (title, body) = (title.to_string(), body.to_string());
        with_view(window, view, cx, |view, window, cx| {
            view.title_input.update(cx, |input, cx| input.set_value(title, window, cx));
            view.body_input.update(cx, |input, cx| input.set_value(body, window, cx));
        });
    }

    /// «Editar» pone el título y la descripción en sus cajas con lo que la
    /// tarea dice, y «Guardar» manda lo escrito y vuelve a la lectura.
    ///
    /// Roturas que atrapa: unas cajas vacías sobre una tarea con texto (y
    /// guardar la borra); un «Guardar» que no manda, o que manda sólo el
    /// título; quedarse editando después de guardar.
    #[gpui_kit::test]
    fn editing_starts_from_what_the_task_says_and_saves_what_was_written(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(!on_screen(window, "task-title-input", cx) && !on_screen(window, "task-save", cx));

        click(window, "task-edit", cx);
        assert!(on_screen(window, "task-title-input", cx) && on_screen(window, "task-description-input", cx));
        assert!(!on_screen(window, "task-title", cx) && !on_screen(window, "task-edit", cx));
        assert_eq!(written(&view, cx), ("Subir el backend".to_string(), "Con **markdown**.".to_string()));

        rewrite(window, &view, "Otro título", "Otra descripción", cx);
        click(window, "task-save", cx);
        wait_until(cx, "se guardan los dos y vuelve a la lectura", |cx| {
            patches(&server).len() == 2 && on_screen(window, "task-title", cx)
        });

        assert_eq!(patches(&server), [json!({ "title": "Otro título" }), json!({ "description": "Otra descripción" })]);
        assert!(!on_screen(window, "task-title-input", cx) && !on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// «Cancelar» tira lo escrito: no se manda, y al volver a editar las
    /// cajas dicen otra vez lo que dice la tarea.
    ///
    /// Roturas que atrapa: un «Cancelar» que guarda; volver a editar y
    /// encontrarse lo que se había descartado.
    #[gpui_kit::test]
    fn cancelling_throws_away_what_was_written(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        click(window, "task-edit", cx);
        rewrite(window, &view, "No", "Tampoco", cx);

        click(window, "task-cancel", cx);
        cx.run_until_parked();
        assert!(on_screen(window, "task-title", cx) && !on_screen(window, "task-title-input", cx));
        assert!(patches(&server).is_empty(), "cancelar guardó {:?}", patches(&server));

        click(window, "task-edit", cx);
        assert_eq!(written(&view, cx).0, "Subir el backend");
        stop(cx);
    }

    /// Si no se pudo guardar, se dice y se sigue editando con lo escrito.
    ///
    /// Rotura que atrapa: perder una descripción entera por un fallo de red.
    #[gpui_kit::test]
    fn an_edit_that_fails_keeps_what_was_written(cx: &mut TestAppContext) {
        // Sin `PATCH` montado: el servidor contesta 404.
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        click(window, "task-edit", cx);
        rewrite(window, &view, "Otro título", "Mucho texto", cx);

        click(window, "task-save", cx);

        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        assert!(on_screen(window, "task-title-input", cx));
        assert_eq!(written(&view, cx), ("Otro título".to_string(), "Mucho texto".to_string()));
        stop(cx);
    }

    /// Al abrir otra tarea no se sigue editando: lo escrito era de la anterior.
    ///
    /// Rotura que atrapa: guardar en una tarea el título que se escribía
    /// para otra.
    #[gpui_kit::test]
    fn another_task_is_not_opened_in_edit_mode(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        let (window, _view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        click(window, "task-edit", cx);
        assert!(on_screen(window, "task-title-input", cx));

        open_other_task(cx);
        wait_until(cx, "llega la otra", |cx| {
            cx.update(|cx| Backend::snapshot(cx).read(cx).task.detail.as_ref().is_some_and(|detail| detail.task.id == "t2"))
        });

        assert!(on_screen(window, "task-title", cx) && !on_screen(window, "task-title-input", cx));
        assert!(on_screen(window, "task-edit", cx));
        stop(cx);
    }

    /// Pulsar la casilla de una subtarea sin hacer la manda a la columna de
    /// «hecho» de su lista.
    ///
    /// Roturas que atrapa: una casilla que no hace nada; mover la tarea madre
    /// en vez de la subtarea; desmarcar lo que se quería marcar.
    #[gpui_kit::test]
    fn ticking_a_subtask_moves_it_to_done(cx: &mut TestAppContext) {
        let mut done = detail("t1")["status"].clone();
        done["id"] = json!("s-done");
        done["kind"] = json!("done");
        done["status"] = json!("resolved");
        // Dos columnas: la abierta, donde está la subtarea, y la de «hecho».
        let mut open_column = detail("t1")["status"].clone();
        open_column["kind"] = json!("active");
        let server_columns = json!([open_column, done]);
        let server = {
            let server = server_with_tasks(vec![contract_task("t1", "a", |_| {})]);
            server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(detail("t1"))));
            server.mount(
                Mock::given(method("GET")).and(path_regex("^/api/v1/task-lists/[^/]+/statuses$")).respond_with(ok(server_columns)),
            );
            server.mount(Mock::given(method("POST")).and(path("/api/v1/tasks/t2/move")).respond_with(ok(json!(null))));
            server
        };
        let window = open(&server, cx);
        open_first_task(window, cx);
        assert!(!on_screen(window, "subtask-done-t2", cx));

        click(window, "subtask-tick-t2", cx);
        wait_until(cx, "pide mover la subtarea", |_| !bodies(&server, "/api/v1/tasks/t2/move").is_empty());

        assert!(bodies(&server, "/api/v1/tasks/t2/move")[0].contains("\"statusId\":\"s-done\""));
        assert!(bodies(&server, "/api/v1/tasks/t1/move").is_empty(), "movió la tarea madre");
        stop(cx);
    }

    /// Si la casilla no se puede guardar, se dice.
    ///
    /// Rotura que atrapa: una casilla que parece no hacer nada (aquí la
    /// lista no tiene columna de «pendiente» a la que devolverla).
    #[gpui_kit::test]
    fn a_tick_that_cannot_be_saved_is_said(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        open_first_task(window, cx);
        assert!(on_screen(window, "subtask-done-t2", cx));

        click(window, "subtask-tick-t2", cx);

        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// Escribir `@` y unas letras ofrece al equipo, y elegir a alguien pone
    /// su mención; donde lee un cliente no se ofrece a nadie y la caja no lo
    /// promete.
    ///
    /// Roturas que atrapa: una caja que promete el `@` y no hace nada;
    /// ofrecer nombrar a un compañero en lo que lee un cliente; elegir y que
    /// quede un `@luis` suelto, que no avisa a nadie; ofrecer a quien no casa
    /// con lo escrito; seguir ofreciendo tras elegir.
    #[gpui_kit::test]
    fn an_at_offers_the_team_and_choosing_writes_the_mention(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        wait_until(cx, "llega el equipo", |cx| cx.update(|cx| !Backend::snapshot(cx).read(cx).task.team.is_empty()));
        let promise = |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).comment_promise);

        // La tarea la lee un cliente y el comentario va para él.
        write(window, "mira @lu", cx);
        assert!(!on_screen(window, "mentions", cx), "ofrece nombrar a alguien en lo que lee un cliente");
        assert_eq!(promise(cx), "work:task.writeCommentClient");

        click(window, "comment-audience", cx);
        assert!(on_screen(window, "mention-u2", cx), "en la nota interna no ofrece al equipo");
        assert_eq!(promise(cx), "work:task.writeComment");

        click(window, "mention-u2", cx);
        assert_eq!(typed(&view, cx), "mira [@luis](cac:user/u2) ");
        assert!(!on_screen(window, "mentions", cx), "sigue ofreciendo después de elegir");

        // Se sigue escribiendo donde quedó el cursor, y quien no casa no sale.
        cx.update_window(window, |_, window, cx| window.input("@zz", cx)).unwrap();
        assert_eq!(typed(&view, cx), "mira [@luis](cac:user/u2) @zz");
        assert!(!on_screen(window, "mentions", cx), "ofrece a quien no casa");
        stop(cx);
    }

    /// Nombrar a alguien a mitad de un texto de varias líneas deja el cursor
    /// justo detrás de la mención, y lo de después intacto.
    ///
    /// Roturas que atrapa: mandar el cursor al final del texto, o a otra
    /// línea; contar bytes donde el campo cuenta letras (un acento antes y el
    /// cursor cae una letra más allá).
    #[gpui_kit::test]
    fn naming_someone_mid_text_leaves_the_cursor_after_the_mention(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        wait_until(cx, "llega el equipo", |cx| cx.update(|cx| !Backend::snapshot(cx).read(cx).task.team.is_empty()));
        click(window, "comment-audience", cx);

        with_view(window, &view, cx, |view, window, cx| {
            view.comment.update(cx, |comment, cx| {
                comment.set_value("uno\ncafé @lu fin", window, cx);
                comment.set_cursor_position(gpui_kit::component::input::Position::new(1, 8), window, cx);
            });
        });
        assert!(on_screen(window, "mention-u2", cx), "no ofrece con el cursor a mitad del texto");
        click(window, "mention-u2", cx);
        cx.update_window(window, |_, window, cx| window.input("X", cx)).unwrap();

        assert_eq!(typed(&view, cx), "uno\ncafé [@luis](cac:user/u2) X fin");
        stop(cx);
    }

    /// El servidor de siempre con un equipo de tantos (`p0`, `p1`…), y la
    /// tarea abierta en una nota interna, que es donde se nombra.
    fn open_with_a_team(people: usize, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle, Entity<TaskView>) {
        let server = server();
        let team: Vec<serde_json::Value> = (0..people).map(|n| json!({ "id": format!("p{n}"), "username": format!("persona{n}") })).collect();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/users/search")).respond_with(ok(json!(team))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        wait_until(cx, "llega el equipo", |cx| cx.update(|cx| !Backend::snapshot(cx).read(cx).task.team.is_empty()));
        click(window, "comment-audience", cx);
        (server, window, view)
    }

    fn press(window: AnyWindowHandle, key: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
    }

    fn type_more(window: AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| window.input(text, cx)).unwrap();
    }

    /// Con la lista abierta, las flechas eligen a quién y Enter o Tab lo
    /// nombran; sin lista, esas teclas siguen siendo de la caja.
    ///
    /// Roturas que atrapa: unas flechas que mueven el cursor del texto y no
    /// el marcado; un Enter que parte la línea (o un Tab que sangra) en vez
    /// de nombrar; nombrar siempre al primero; que un `@` nuevo herede el
    /// marcado del anterior; quedarse con Enter cuando no hay lista, y no
    /// poder partir una línea; que Mayúsculas+Enter nombre en vez de partirla.
    #[gpui_kit::test]
    fn the_keyboard_picks_who_to_name(cx: &mut TestAppContext) {
        let (_server, window, view) = open_with_a_team(3, cx);

        write(window, "@", cx);
        assert!(on_screen(window, "mentions", cx));
        press(window, "down", cx);
        press(window, "enter", cx);
        assert_eq!(typed(&view, cx), "[@persona1](cac:user/p1) ");
        assert!(!on_screen(window, "mentions", cx), "sigue ofreciendo después de elegir");

        // Otro `@`: empieza por el primero, y hacia arriba da la vuelta.
        type_more(window, "@", cx);
        press(window, "up", cx);
        press(window, "tab", cx);
        assert_eq!(typed(&view, cx), "[@persona1](cac:user/p1) [@persona2](cac:user/p2) ");

        // Sin lista, Enter es de quien escribe.
        press(window, "enter", cx);
        type_more(window, "fin", cx);
        assert_eq!(typed(&view, cx), "[@persona1](cac:user/p1) [@persona2](cac:user/p2) \nfin");

        // Y con la lista abierta, Mayúsculas+Enter sigue partiendo la línea.
        type_more(window, " @", cx);
        assert!(on_screen(window, "mentions", cx));
        press(window, "shift-enter", cx);
        assert!(typed(&view, cx).ends_with("fin @\n"), "Mayúsculas+Enter nombró a alguien: {:?}", typed(&view, cx));
        stop(cx);
    }

    /// Escape cierra la lista y deja la página y lo escrito como estaban; la
    /// lista no vuelve con la letra siguiente, y sí con otro `@`.
    ///
    /// Roturas que atrapa: un Escape que no cierra la lista; que se lleve la
    /// página entera con el comentario a medias; una lista que reaparece al
    /// seguir escribiendo; no volver a ofrecer a nadie en ese comentario; que
    /// un `@` borrado y vuelto a escribir siga cerrado.
    #[gpui_kit::test]
    fn escape_closes_the_list_and_not_the_page(cx: &mut TestAppContext) {
        let (_server, window, view) = open_with_a_team(3, cx);
        let open_task = |cx: &mut TestAppContext| cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.clone());

        write(window, "@per", cx);
        assert!(on_screen(window, "mentions", cx));
        press(window, "escape", cx);
        settle(cx);
        assert!(!on_screen(window, "mentions", cx), "Escape no cerró la lista");
        assert_eq!(open_task(cx).as_deref(), Some("t1"), "Escape cerró la página con la lista abierta");
        assert_eq!(typed(&view, cx), "@per");

        type_more(window, "s", cx);
        assert!(!on_screen(window, "mentions", cx), "la lista vuelve con la letra siguiente");
        type_more(window, " @", cx);
        assert!(on_screen(window, "mentions", cx), "otro @ ya no ofrece a nadie");
        // Con las flechas de por medio también se cierra, y lo marcado no se nombra.
        press(window, "down", cx);
        press(window, "escape", cx);
        assert!(!on_screen(window, "mentions", cx));
        assert_eq!(typed(&view, cx), "@pers @");

        // Borrado ese `@` y vuelto a escribir en el mismo sitio, es otro.
        press(window, "backspace", cx);
        assert_eq!(typed(&view, cx), "@pers ");
        on_screen(window, "mentions", cx);
        type_more(window, "@", cx);
        assert!(on_screen(window, "mentions", cx), "un @ vuelto a escribir sigue cerrado");
        stop(cx);
    }

    /// Quien va marcado se distingue, y bajando por una lista larga sigue a
    /// la vista.
    ///
    /// Roturas que atrapa: bajar con las flechas a alguien que queda fuera
    /// de la lista, y nombrar a ciegas; un marcado que no se mueve de la
    /// primera fila.
    #[gpui_kit::test]
    fn the_highlighted_person_stays_in_view(cx: &mut TestAppContext) {
        let (_server, window, view) = open_with_a_team(20, cx);
        let inside = |cx: &mut TestAppContext, row: &str| {
            let row = gpui_kit::SharedString::from(row.to_string());
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let (list, row) = (window.try_find("mentions").unwrap().bounds(), window.try_find(row).unwrap().bounds());
                row.top() >= list.top() && row.bottom() <= list.bottom()
            })
            .unwrap()
        };
        let highlighted = |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).mention_menu(cx).map(|menu| menu.highlighted));

        write(window, "@", cx);
        assert!(inside(cx, "mention-p0") && !inside(cx, "mention-p19"), "la lista no es más larga que su caja: la prueba no prueba nada");
        for _ in 0..19 {
            press(window, "down", cx);
        }
        assert_eq!(highlighted(cx), Some(19));
        assert!(inside(cx, "mention-p19"), "el marcado quedó fuera de la vista");

        press(window, "down", cx);
        assert_eq!(highlighted(cx), Some(0));
        assert!(inside(cx, "mention-p0"), "al dar la vuelta, el primero quedó fuera de la vista");
        stop(cx);
    }

    /// Lo escrito se manda como comentario y la caja se vacía; donde un
    /// cliente lee, se puede dejar en interno antes de mandar.
    ///
    /// Roturas que atrapa: un botón que no manda; mandar otra cosa que lo
    /// escrito; una caja que se queda con lo ya mandado (y se manda dos
    /// veces); un interruptor de «interno» que no llega al servidor.
    #[gpui_kit::test]
    fn what_is_written_is_sent_and_can_be_kept_internal(cx: &mut TestAppContext) {
        let server = server();
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/tasks/t1/comments"))
                .respond_with(wiremock::ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": detail("t1") }))),
        );
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        let sent = |server: &FakeServer| bodies(server, "/api/v1/tasks/t1/comments");

        write(window, "para todos", cx);
        click(window, "comment-send", cx);
        wait_until(cx, "llega y la caja se vacía", |cx| sent(&server).len() == 1 && typed(&view, cx).is_empty());
        assert_eq!(sent(&server)[0], r#"{"body":"para todos"}"#);

        click(window, "comment-audience", cx);
        write(window, "entre nosotros", cx);
        click(window, "comment-send", cx);
        wait_until(cx, "llega el segundo", |_| sent(&server).len() == 2);
        assert!(sent(&server)[1].contains(r#""visibility":"internal""#), "mandó {:?}", sent(&server)[1]);

        // Con la caja vacía no se manda nada, y tampoco es un fallo que avisar.
        wait_until(cx, "la caja se vacía", |cx| typed(&view, cx).is_empty());
        click(window, "comment-send", cx);
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(150));
        cx.run_until_parked();
        assert_eq!(sent(&server).len(), 2);
        assert!(!on_screen(window, "task-problem", cx), "avisa de un fallo por pulsar con la caja vacía");
        stop(cx);
    }

    /// Un comentario que no se pudo mandar se dice, y lo escrito se queda.
    ///
    /// Rotura que atrapa: perder lo escrito por un fallo de red.
    #[gpui_kit::test]
    fn a_comment_that_fails_keeps_what_was_written(cx: &mut TestAppContext) {
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);

        write(window, "no se pierde", cx);
        click(window, "comment-send", cx);

        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        assert_eq!(typed(&view, cx), "no se pierde");
        stop(cx);
    }

    /// Una subtarea que no está hecha no lleva la marca.
    ///
    /// Rotura que atrapa: un checklist entero marcado, hecho o no.
    #[gpui_kit::test]
    fn a_subtask_that_is_not_done_is_not_ticked(cx: &mut TestAppContext) {
        let server = server_with_column("active");
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llegan el detalle y sus columnas", |cx| {
            cx.update(|cx| !Backend::snapshot(cx).read(cx).task.statuses.is_empty()) && on_screen(window, "subtask-t2", cx)
        });

        assert!(!on_screen(window, "subtask-done-t2", cx));
        stop(cx);
    }

    /// Las notas de agentes están en su pestaña, aparte de la conversación.
    ///
    /// Roturas que atrapa: las dos cosas mezcladas en un hilo; una pestaña
    /// que cambia de color y enseña lo mismo.
    #[gpui_kit::test]
    fn agent_notes_are_behind_their_own_tab(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "comment-said", cx));
        assert!(!on_screen(window, "comment-noted", cx), "la nota del agente sale en la conversación");

        click(window, "thread-notes", cx);

        assert!(on_screen(window, "comment-noted", cx));
        assert!(!on_screen(window, "comment-said", cx), "la conversación sigue bajo la pestaña de notas");
        stop(cx);
    }

    /// Al abrir otra tarea se empieza por su conversación, aunque en la
    /// anterior se estuvieran leyendo las notas de agentes.
    ///
    /// Rotura que atrapa: abrir una tarea y encontrarse las notas de un
    /// agente en vez de lo que le han dicho a uno.
    #[gpui_kit::test]
    fn another_task_starts_on_its_conversation(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "comment-said", cx));
        click(window, "thread-notes", cx);
        assert!(on_screen(window, "comment-noted", cx));

        open_other_task(cx);
        wait_until(cx, "llega la otra", |cx| {
            cx.update(|cx| Backend::snapshot(cx).read(cx).task.detail.as_ref().is_some_and(|detail| detail.task.id == "t2"))
        });

        assert!(on_screen(window, "comment-said", cx) && !on_screen(window, "comment-noted", cx));
        stop(cx);
    }

    /// Una tarea que no llega lo dice, y no deja a la vista la que se leía.
    ///
    /// Roturas que atrapa: un panel en blanco para siempre; el título de la
    /// tarea anterior sobre un detalle que no es el suyo.
    #[gpui_kit::test]
    fn a_task_that_does_not_arrive_says_so(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "work-row-t1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "task-title", cx));

        open_other_task(cx);
        wait_until(cx, "falla y lo dice", |cx| {
            cx.update(|cx| Backend::snapshot(cx).read(cx).task.error.is_some()) && on_screen(window, "task-waiting", cx)
        });

        assert!(!on_screen(window, "task-title", cx), "sigue a la vista la tarea anterior");
        stop(cx);
    }
    const RAW: &str = "/api/v1/tasks/t1/attachments/a1/raw";

    /// La primera tarea con un fichero suyo, `plan.pdf`.
    fn server_with_a_file() -> FakeServer {
        let server = server_with_tasks(vec![contract_task("t1", "a", |_| {})]);
        let mut first = detail("t1");
        first["attachments"] = json!([{
            "id": "a1", "itemId": "t1", "commentId": null, "url": RAW, "fileName": "plan.pdf",
            "contentType": "application/pdf", "bytes": 12288,
        }]);
        let mut column = first["status"].clone();
        column["listId"] = first["task"]["listId"].clone();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(first)));
        server.mount(
            Mock::given(method("GET")).and(path_regex("^/api/v1/task-lists/[^/]+/statuses$")).respond_with(ok(json!([column]))),
        );
        server
    }

    fn serving_the_file(server: &FakeServer) {
        server.mount(
            Mock::given(method("GET")).and(path(RAW)).respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.7".to_vec())),
        );
    }

    /// Apunta lo que la app le pide abrir al sistema, en vez de abrirlo.
    fn record_opened(cx: &mut TestAppContext) -> std::sync::Arc<std::sync::Mutex<Vec<std::path::PathBuf>>> {
        let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = opened.clone();
        cx.update(|cx| {
            cx.set_global(crate::files::Opener(std::sync::Arc::new(move |path| seen.lock().unwrap().push(path.to_path_buf()))))
        });
        opened
    }

    /// Una carpeta vacía para esta prueba.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cac-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Da tiempo a lo que **no** debería pasar: una petición sale desde otro
    /// hilo, y mirar nada más cancelar daría por bueno lo que aún no salió.
    fn settle(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        for _ in 0..20 {
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn asked(server: &FakeServer, verb: &str, route: &str) -> Vec<wiremock::Request> {
        server.requests().into_iter().filter(|request| request.method.as_str() == verb && request.url.path() == route).collect()
    }

    /// Los ficheros de la tarea salen en su sección, y pulsar el nombre de
    /// uno lo baja y se lo da al sistema para abrirlo.
    ///
    /// Roturas que atrapa: una sección de adjuntos vacía con ficheros; un
    /// nombre que no abre nada; darle al sistema un fichero con otro
    /// contenido o con otro nombre.
    #[gpui_kit::test]
    fn the_name_of_a_file_opens_it(cx: &mut TestAppContext) {
        let server = server_with_a_file();
        serving_the_file(&server);
        let window = open(&server, cx);
        let opened = record_opened(cx);
        open_first_task(window, cx);
        assert!(on_screen(window, "task-file-a1", cx));

        click(window, "task-file-a1", cx);

        wait_until(cx, "se le da al sistema", |_| !opened.lock().unwrap().is_empty());
        let path = opened.lock().unwrap()[0].clone();
        assert_eq!(path.file_name().unwrap(), "plan.pdf");
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.7");
        assert!(!on_screen(window, "task-problem", cx));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        stop(cx);
    }

    /// Un fichero que no se pudo bajar se dice, y no se abre nada.
    ///
    /// Rotura que atrapa: un nombre que parece no hacer nada cuando el
    /// fichero no llega.
    #[gpui_kit::test]
    fn a_file_that_does_not_arrive_says_so(cx: &mut TestAppContext) {
        // Sin la ruta del fichero montada: el servidor contesta 404.
        let server = server_with_a_file();
        let window = open(&server, cx);
        let opened = record_opened(cx);
        open_first_task(window, cx);

        click(window, "task-file-a1", cx);

        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        assert!(opened.lock().unwrap().is_empty());
        stop(cx);
    }

    /// «Guardar una copia» pregunta dónde y la escribe ahí; si se cancela,
    /// no se baja nada.
    ///
    /// Roturas que atrapa: un botón que no pregunta; una copia con otro
    /// contenido; bajar el fichero aunque se cancele.
    #[gpui_kit::test]
    fn saving_a_copy_writes_it_where_it_was_chosen(cx: &mut TestAppContext) {
        let server = server_with_a_file();
        serving_the_file(&server);
        let window = open(&server, cx);
        open_first_task(window, cx);
        let dir = scratch("save");
        let target = dir.join("copia.pdf");

        click(window, "task-file-save-a1", cx);
        assert!(cx.did_prompt_for_new_path(), "no preguntó dónde guardar");
        cx.simulate_new_path_selection(|_| None);
        settle(cx);
        assert!(asked(&server, "GET", RAW).is_empty(), "bajó el fichero aunque se canceló");

        click(window, "task-file-save-a1", cx);
        let chosen = target.clone();
        cx.simulate_new_path_selection(move |_| Some(chosen));
        wait_until(cx, "la copia queda escrita", |_| target.exists());

        assert_eq!(std::fs::read(&target).unwrap(), b"%PDF-1.7");
        let _ = std::fs::remove_dir_all(&dir);
        stop(cx);
    }

    /// Quitar un adjunto pregunta antes, diciendo cuál; «Cancelar» no quita
    /// nada y «Quitar» lo pide.
    ///
    /// Roturas que atrapa: un clic que borra sin preguntar; un «Cancelar»
    /// que borra igual; un «Quitar» que no hace nada.
    #[gpui_kit::test]
    fn removing_a_file_asks_first(cx: &mut TestAppContext) {
        let server = server_with_a_file();
        server.mount(
            Mock::given(method("DELETE"))
                .and(path("/api/v1/tasks/t1/attachments/a1"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "Attachment deleted" }))),
        );
        let window = open(&server, cx);
        open_first_task(window, cx);
        let removals = |server: &FakeServer| asked(server, "DELETE", "/api/v1/tasks/t1/attachments/a1").len();

        click(window, "task-file-remove-a1", cx);
        let (question, _) = cx.pending_prompt().expect("no preguntó antes de quitar");
        assert!(question.contains("plan.pdf"), "no dice qué fichero: {question}");
        cx.simulate_prompt_answer("Cancelar");
        settle(cx);
        assert_eq!(removals(&server), 0, "quitó el fichero aunque se canceló");

        click(window, "task-file-remove-a1", cx);
        assert_eq!(removals(&server), 0, "quitó el fichero sin esperar la respuesta");
        cx.simulate_prompt_answer("Quitar");
        wait_until(cx, "se pide quitarlo", |_| removals(&server) == 1);
        stop(cx);
    }

    fn tag_name(view: &Entity<TaskView>, cx: &mut TestAppContext) -> String {
        cx.update(|cx| view.read(cx).tag_name.read(cx).value().to_string())
    }

    /// Pulsa lo que se llama así: una entrada de un desplegable no tiene
    /// un id propio con el que buscarla.
    fn click_label(window: AnyWindowHandle, label: &str, cx: &mut TestAppContext) {
        use gpui_kit::test::{TestEventExt, TestQueryExt};
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let at = window.find_by_label(label).bounds().center();
            window.pointer_move(at, None, cx);
            window.pointer_down(at, gpui_kit::MouseButton::Left, cx);
            window.pointer_up(at, gpui_kit::MouseButton::Left, cx);
        })
        .unwrap();
    }

    fn start_naming(window: AnyWindowHandle, view: &Entity<TaskView>, cx: &mut TestAppContext) {
        with_view(window, view, cx, |view, window, cx| view.start_naming_tag(window, cx));
    }

    /// `server_with_choices`, con un servidor que crea la etiqueta `tag9`.
    fn server_that_makes_tags() -> FakeServer {
        let server = server_with_choices();
        server.mount(Mock::given(method("POST")).and(path("/api/v1/task-tags/")).respond_with(
            wiremock::ResponseTemplate::new(201)
                .set_body_json(json!({ "success": true, "data": { "id": "tag9", "orgId": "o1", "name": "Diseño", "color": "#06B6D4" } })),
        ));
        server
    }

    fn made(server: &FakeServer) -> Vec<serde_json::Value> {
        asked(server, "POST", "/api/v1/task-tags/").iter().map(|request| serde_json::from_slice(&request.body).unwrap()).collect()
    }

    /// «+ Etiqueta nueva» pide un nombre; Enter o «Crear» la crean en la
    /// organización de la tarea y se la ponen, y con el nombre de una que ya
    /// hay se pone ésa. Cancelar no manda nada.
    ///
    /// Roturas que atrapa: no tener cómo crear una etiqueta desde la tarea;
    /// un nombre que hay que ir a pulsar antes de escribirlo; un Enter (o un
    /// «Crear») que no hace nada; crearla y no ponérsela; quedarse pidiendo
    /// el nombre después de crearla; un «Cancelar» que crea; reencontrarse
    /// lo cancelado al volver a empezar; avisar de un fallo por un nombre en
    /// blanco.
    #[gpui_kit::test]
    fn a_new_tag_is_named_made_and_put_on_the_task(cx: &mut TestAppContext) {
        let server = server_that_makes_tags();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        wait_until(cx, "llega lo que se puede elegir", |cx| cx.update(|cx| !Backend::snapshot(cx).read(cx).task.tags.is_empty()));
        assert!(!on_screen(window, "task-new-tag", cx), "pide un nombre sin que nadie lo pidiera");

        // El desplegable de etiquetas acaba en «+ Etiqueta nueva».
        click(window, "task-tags", cx);
        click_label(window, "+ Etiqueta nueva", cx);
        settle(cx);
        assert!(on_screen(window, "task-new-tag-name", cx));
        // Ya con el foco: se escribe sin pulsar antes.
        type_more(window, "Diseño", cx);
        assert_eq!(tag_name(&view, cx), "Diseño");
        press(window, "enter", cx);
        wait_until(cx, "se crea, se le pone y deja de pedir el nombre", |cx| {
            patches(&server).len() == 1 && !on_screen(window, "task-new-tag", cx)
        });
        assert_eq!(made(&server), [json!({ "orgId": "o1", "name": "Diseño", "color": "#06B6D4" })]);
        assert_eq!(patches(&server), [json!({ "tagIds": ["tag1", "tag9"] })]);
        assert!(!on_screen(window, "task-problem", cx));

        // Con el nombre de una que ya hay, se pone ésa.
        start_naming(window, &view, cx);
        assert_eq!(tag_name(&view, cx), "", "arranca con el nombre de la anterior");
        type_more(window, "Backend", cx);
        click(window, "task-new-tag-create", cx);
        wait_until(cx, "se le pone la que había", |_| patches(&server).len() == 2);
        assert_eq!(patches(&server)[1], json!({ "tagIds": ["tag1", "tag2"] }));
        assert_eq!(made(&server).len(), 1, "creó otra «backend»");

        start_naming(window, &view, cx);
        type_more(window, "No", cx);
        click(window, "task-new-tag-cancel", cx);
        settle(cx);
        assert!(!on_screen(window, "task-new-tag", cx), "Cancelar no dejó de pedir el nombre");

        // En blanco no se pide nada, ni es un fallo que avisar.
        start_naming(window, &view, cx);
        assert_eq!(tag_name(&view, cx), "", "arranca con lo que se canceló");
        type_more(window, "  ", cx);
        press(window, "enter", cx);
        settle(cx);
        assert_eq!((made(&server).len(), patches(&server).len()), (1, 2), "Cancelar, o un nombre en blanco, mandó algo");
        assert!(on_screen(window, "task-new-tag", cx) && !on_screen(window, "task-problem", cx));
        stop(cx);
    }

    /// Una etiqueta que no se pudo crear se dice, y el nombre se queda
    /// escrito; al abrir otra tarea no se sigue pidiendo.
    ///
    /// Roturas que atrapa: un «Crear» que parece no hacer nada cuando falla;
    /// perder el nombre escrito; ponerle a otra tarea la etiqueta que se
    /// nombraba para ésta.
    #[gpui_kit::test]
    fn a_tag_that_is_not_made_keeps_its_name(cx: &mut TestAppContext) {
        // Sin la ruta de crear etiquetas montada: el servidor contesta 404.
        let server = server_with_choices();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);

        start_naming(window, &view, cx);
        type_more(window, "Diseño", cx);
        click(window, "task-new-tag-create", cx);
        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        assert!(on_screen(window, "task-new-tag", cx));
        assert_eq!(tag_name(&view, cx), "Diseño");
        assert!(patches(&server).is_empty());

        open_other_task(cx);
        wait_until(cx, "llega la otra", |cx| {
            cx.update(|cx| Backend::snapshot(cx).read(cx).task.detail.as_ref().is_some_and(|detail| detail.task.id == "t2"))
        });
        assert!(on_screen(window, "task-tags", cx) && !on_screen(window, "task-new-tag", cx));
        stop(cx);
    }

    /// Contesta a una subida, una vez, con el adjunto que quedó.
    fn storing(server: &FakeServer, id: &str, name: &str) {
        let stored = json!({ "id": id, "itemId": "t1", "url": format!("/api/v1/tasks/t1/attachments/{id}/raw"), "fileName": name });
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/tasks/t1/attachments"))
                .respond_with(wiremock::ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": stored })))
                .up_to_n_times(1),
        );
    }

    /// Adjuntar al comentar sube los ficheros a la tarea y los cita donde
    /// está el cursor, cada uno como lo que es; se sigue escribiendo detrás.
    ///
    /// Roturas que atrapa: un botón que no pregunta; subir y no citar (el
    /// comentario sale sin su fichero); citar sólo el primero; la cita al
    /// final aunque se escribía en medio, o el cursor perdido tras ella;
    /// subir aunque se cancele; callar el que no se pudo subir, tirar lo
    /// escrito por él, o seguir subiendo los de detrás.
    #[gpui_kit::test]
    fn a_file_attached_while_commenting_is_cited_in_the_comment(cx: &mut TestAppContext) {
        let server = server_with_a_file();
        storing(&server, "a2", "notas.txt");
        storing(&server, "a3", "captura.png");
        // Por si se sube lo que no se debe: que no sea un 404 lo que lo tape.
        storing(&server, "a4", "notas.txt");
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        let dir = scratch("cite");
        let (notes, shot, huge) = (dir.join("notas.txt"), dir.join("captura.png"), dir.join("enorme.bin"));
        std::fs::write(&notes, b"hola").unwrap();
        std::fs::write(&shot, b"\x89PNG\x00\xff").unwrap();
        std::fs::File::create(&huge).unwrap().set_len(cac_core::MAX_ATTACHMENT_BYTES as u64 + 1).unwrap();
        let uploads = |server: &FakeServer| asked(server, "POST", "/api/v1/tasks/t1/attachments");
        // «mira esto», con el cursor detrás de «mira».
        with_view(window, &view, cx, |view, window, cx| {
            view.comment.update(cx, |comment, cx| {
                comment.set_value("mira esto", window, cx);
                comment.set_cursor_position(gpui_kit::component::input::Position::new(0, 4), window, cx);
            });
        });

        click(window, "comment-attach", cx);
        assert!(cx.did_prompt_for_paths(), "no preguntó qué adjuntar");
        cx.simulate_path_prompt_response(|_| None);
        settle(cx);
        assert!(uploads(&server).is_empty(), "subió algo aunque se canceló");
        assert_eq!(typed(&view, cx), "mira esto");

        click(window, "comment-attach", cx);
        let chosen = vec![notes.clone(), shot.clone()];
        cx.simulate_path_prompt_response(move |_| Some(chosen));
        let cited = "mira [notas.txt](/api/v1/tasks/t1/attachments/a2/raw) \n![captura.png](/api/v1/tasks/t1/attachments/a3/raw)\n";
        wait_until(cx, "suben los dos y quedan citados", |cx| typed(&view, cx) == format!("{cited} esto"));
        assert_eq!(uploads(&server)[0].body, cac_core::multipart("notas.txt", b"hola").1);
        assert_eq!(uploads(&server)[1].body, cac_core::multipart("captura.png", b"\x89PNG\x00\xff").1);
        assert!(!on_screen(window, "task-problem", cx));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("X", cx);
        })
        .unwrap();
        assert_eq!(typed(&view, cx), format!("{cited}X esto"));

        // Uno que no cabe, y otro detrás: se para en el primero.
        click(window, "comment-attach", cx);
        cx.simulate_path_prompt_response(move |_| Some(vec![huge, notes]));
        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        settle(cx);
        assert_eq!(uploads(&server).len(), 2, "mandó el que pasa del tope, o siguió con el de detrás");
        assert_eq!(typed(&view, cx), format!("{cited}X esto"));
        let _ = std::fs::remove_dir_all(&dir);
        stop(cx);
    }

    /// Un enlace a un fichero de la tarea lo baja y lo abre como desde sus
    /// adjuntos; uno de fuera va al navegador, y una mención a ningún sitio.
    ///
    /// Roturas que atrapa: darle al navegador una dirección que pide sesión
    /// (el fichero citado en un comentario no abre nada); dejar de abrir los
    /// enlaces de verdad; pedirle al sistema que abra `cac:user/…`.
    #[gpui_kit::test]
    fn a_link_to_a_file_of_the_task_opens_the_file(cx: &mut TestAppContext) {
        let server = server_with_a_file();
        serving_the_file(&server);
        let (window, view) = open_with_view(&server, cx);
        let opened = record_opened(cx);
        open_first_task(window, cx);
        let follow = |url: &'static str, cx: &mut TestAppContext| with_view(window, &view, cx, move |view, window, cx| view.follow_link(url, window, cx));

        follow("cac:user/11111111-1111-4111-8111-111111111111", cx);
        settle(cx);
        assert_eq!(cx.opened_url(), None, "le pidió al sistema abrir una mención");
        assert!(opened.lock().unwrap().is_empty());

        follow(RAW, cx);
        wait_until(cx, "se le da al sistema", |_| !opened.lock().unwrap().is_empty());
        let path = opened.lock().unwrap()[0].clone();
        assert_eq!(path.file_name().unwrap(), "plan.pdf");
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.7");
        assert_eq!(cx.opened_url(), None, "le dio al navegador una dirección que pide sesión");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        follow("https://example.com/docs", cx);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
        assert_eq!(opened.lock().unwrap().len(), 1);
        stop(cx);
    }

    const SHOT: &str = "/api/v1/tasks/t1/attachments/a2/raw";

    /// La tarea con `plan.pdf` y una captura, `captura.png`, que llegó sin
    /// tipo (como todo lo que entra por la integración) y que la descripción
    /// cita. El servidor contesta por la captura lo que se le diga.
    fn server_with_a_picture(served: &[u8]) -> FakeServer {
        let server = server_with_tasks(vec![contract_task("t1", "a", |_| {}), contract_task("t2", "a", |_| {})]);
        let mut first = detail("t1");
        first["attachments"] = json!([
            { "id": "a1", "itemId": "t1", "commentId": null, "url": RAW, "fileName": "plan.pdf", "contentType": "application/pdf", "bytes": 12288 },
            { "id": "a2", "itemId": "t1", "commentId": null, "url": SHOT, "fileName": "captura.png" },
        ]);
        first["task"]["description"] = json!(format!("Mira: ![]({SHOT})"));
        let mut column = first["status"].clone();
        column["listId"] = first["task"]["listId"].clone();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(first)));
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t2")).respond_with(ok(detail("t2"))));
        server.mount(
            Mock::given(method("GET")).and(path_regex("^/api/v1/task-lists/[^/]+/statuses$")).respond_with(ok(json!([column]))),
        );
        server.mount(Mock::given(method("GET")).and(path(SHOT)).respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(served.to_vec())));
        server
    }

    /// Una imagen de verdad, de 600 × 200.
    const PICTURE: &[u8] = include_bytes!("../../core/tests/fixtures/gray_600x200.png");

    fn size_on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> (f32, f32) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let bounds = window.try_find(id).expect("no está a la vista").bounds();
            (f32::from(bounds.size.width), f32::from(bounds.size.height))
        })
        .unwrap()
    }

    /// Una imagen de la tarea se ve en su sección, a su tamaño de miniatura,
    /// y se baja una sola vez y con la sesión; lo que no es imagen se sigue
    /// listando. Pulsarla la abre, y al abrir otra tarea se tira.
    ///
    /// Roturas que atrapa: una captura que sale como una línea con un clip;
    /// una miniatura deformada o a tamaño natural; pedir la imagen sin
    /// sesión (el servidor contesta 404), o volver a pedirla en cada
    /// pintada; pintar un PDF como imagen o perderlo de la lista; una
    /// miniatura que no abre nada; perder de vista cuál cita la descripción;
    /// guardarse en memoria las imágenes de todas las tareas abiertas.
    #[gpui_kit::test]
    fn a_picture_of_the_task_is_seen_and_not_listed(cx: &mut TestAppContext) {
        let server = server_with_a_picture(PICTURE);
        serving_the_file(&server);
        let (window, view) = open_with_view(&server, cx);
        let opened = record_opened(cx);
        open_first_task(window, cx);

        wait_until(cx, "la captura se ve", |cx| on_screen(window, "task-thumb-a2", cx));
        // 600 × 200 en un hueco de 256 × 160: 256 × 85, con el margen de un
        // píxel que se lleva el ajuste a la rejilla de la pantalla.
        let (width, height) = size_on_screen(window, "task-thumb-a2", cx);
        assert!((width - 256.).abs() < 1. && (height - 85.).abs() < 1., "la miniatura mide {width} × {height}");
        assert!(on_screen(window, "task-file-cited-a2", cx), "no dice que la descripción la cita");
        assert!(on_screen(window, "task-file-a1", cx), "el PDF ya no está en la lista");
        assert!(!on_screen(window, "task-thumb-a1", cx) && !on_screen(window, "task-thumb-wait-a1", cx), "pinta el PDF como imagen");
        settle(cx);
        let asked_for = asked(&server, "GET", SHOT);
        assert_eq!(asked_for.len(), 1, "pidió la imagen {} veces", asked_for.len());
        assert_eq!(asked_for[0].headers.get("Authorization").and_then(|value| value.to_str().ok()), Some("Bearer access"));
        assert!(asked(&server, "GET", RAW).is_empty(), "bajó el PDF sin que nadie lo abriera");

        click(window, "task-file-a2", cx);
        wait_until(cx, "se le da al sistema", |_| !opened.lock().unwrap().is_empty());
        let path = opened.lock().unwrap()[0].clone();
        assert_eq!(path.file_name().unwrap(), "captura.png");
        assert_eq!(std::fs::read(&path).unwrap(), PICTURE);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        open_other_task(cx);
        wait_until(cx, "llega la otra, sin imágenes guardadas", |cx| {
            on_screen(window, "task-title", cx) && cx.update(|cx| view.read(cx).thumbs.is_empty())
        });
        assert!(!on_screen(window, "task-thumb-a2", cx));
        stop(cx);
    }

    /// Un GIF de un píxel.
    const GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x21, 0xf9, 0x04,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
    ];

    /// Una imagen que no es un PNG también se ve, se llame como se llame; y
    /// al cerrar la página sus imágenes se tiran.
    ///
    /// Roturas que atrapa: decodificar todo como PNG, o según la extensión
    /// del nombre (aquí `captura.png` es un GIF); quedarse con las capturas
    /// en memoria con la página ya cerrada.
    #[gpui_kit::test]
    fn a_picture_is_painted_as_what_it_is(cx: &mut TestAppContext) {
        let server = server_with_a_picture(GIF);
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);

        wait_until(cx, "el GIF se ve", |cx| on_screen(window, "task-thumb-a2", cx));

        click(window, "task-close", cx);
        wait_until(cx, "se cierra y se tiran sus imágenes", |cx| {
            !on_screen(window, "task-page", cx) && cx.update(|cx| view.read(cx).thumbs.is_empty())
        });
        stop(cx);
    }

    /// Una imagen que no se puede pintar se queda con su nombre, que la
    /// sigue abriendo y dejando quitar; no es un fallo que avisar, ni se
    /// vuelve a pedir sin parar.
    ///
    /// Roturas que atrapa: un hueco en blanco sin nombre ni forma de abrirlo;
    /// darle a quien decodifica lo que no es una imagen; un aviso rojo por
    /// una miniatura; pedir en bucle una imagen que no va a llegar.
    #[gpui_kit::test]
    fn a_picture_that_cannot_be_shown_keeps_its_name(cx: &mut TestAppContext) {
        for served in [b"<!doctype html>".as_slice(), &PICTURE[..40]] {
            let server = server_with_a_picture(served);
            let window = open(&server, cx);
            open_first_task(window, cx);

            wait_until(cx, "se da por no pintable", |cx| on_screen(window, "task-thumb-missing-a2", cx));
            assert!(!on_screen(window, "task-thumb-a2", cx));
            assert!(on_screen(window, "task-file-a2", cx) && on_screen(window, "task-file-remove-a2", cx));
            assert!(!on_screen(window, "task-problem", cx), "avisa de un fallo por una miniatura");
            settle(cx);
            assert_eq!(asked(&server, "GET", SHOT).len(), 1, "la pide en bucle");

            click(window, "task-file-remove-a2", cx);
            let (question, _) = cx.pending_prompt().expect("no preguntó antes de quitar");
            assert!(question.contains("captura.png"), "no dice qué fichero: {question}");
            cx.simulate_prompt_answer("Cancelar");
            stop(cx);
            cx.update_window(window, |_, window, _| window.remove_window()).unwrap();
        }
    }

    const SAID: &str = "/api/v1/tasks/t1/comments/said";

    /// La primera tarea con dos comentarios: `said`, de quien está dentro
    /// (Ana, `u1`), y `other`, de otra persona.
    fn server_with_two_voices() -> FakeServer {
        let server = server_with_tasks(vec![contract_task("t1", "a", |_| {})]);
        let mut first = detail("t1");
        let mut other = first["comments"][0].clone();
        other["id"] = json!("other");
        other["authorUserId"] = json!("u2");
        first["comments"] = json!([first["comments"][0].clone(), other]);
        let mut column = first["status"].clone();
        column["listId"] = first["task"]["listId"].clone();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(first)));
        server.mount(
            Mock::given(method("GET")).and(path_regex("^/api/v1/task-lists/[^/]+/statuses$")).respond_with(ok(json!([column]))),
        );
        server
    }

    fn answering(server: &FakeServer, verb: &str) {
        server.mount(
            Mock::given(method(verb))
                .and(path(SAID))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "ok" }))),
        );
    }

    fn edit_typed(view: &Entity<TaskView>, cx: &mut TestAppContext) -> String {
        cx.update(|cx| view.read(cx).comment_edit.read(cx).value().to_string())
    }

    fn retype(window: AnyWindowHandle, view: &Entity<TaskView>, text: &str, cx: &mut TestAppContext) {
        let text = text.to_string();
        with_view(window, view, cx, |view, window, cx| view.comment_edit.update(cx, |input, cx| input.set_value(text, window, cx)));
    }

    /// Editar y borrar se ofrecen en lo que uno escribió, y editar parte de
    /// lo que el comentario dice y guarda lo escrito.
    ///
    /// Roturas que atrapa: ofrecer cambiar el comentario de otro; una caja
    /// de edición vacía; guardar otro texto o en otro comentario; seguir
    /// editando después de guardar; un «Cancelar» que guarda.
    #[gpui_kit::test]
    fn my_comment_is_edited_from_what_it_says(cx: &mut TestAppContext) {
        let server = server_with_two_voices();
        answering(&server, "PATCH");
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(on_screen(window, "comment-edit-said", cx) && on_screen(window, "comment-delete-said", cx));
        assert!(!on_screen(window, "comment-edit-other", cx) && !on_screen(window, "comment-delete-other", cx));
        let says = cx.update(|cx| Backend::core(cx).tasks().sheet(cac_core::Locale::Es).unwrap().talk[0].body.clone());

        click(window, "comment-edit-said", cx);
        assert!(on_screen(window, "comment-edit-input", cx));
        assert!(!on_screen(window, "comment-edit-said", cx), "ofrece editar lo que ya se edita");
        assert_eq!(edit_typed(&view, cx), says);
        retype(window, &view, "otra cosa", cx);
        click(window, "comment-edit-cancel", cx);
        assert!(!on_screen(window, "comment-edit-input", cx), "Cancelar no dejó de editar");

        click(window, "comment-edit-said", cx);
        assert_eq!(edit_typed(&view, cx), says, "arranca con lo que se canceló");
        retype(window, &view, "mejor dicho", cx);
        click(window, "comment-edit-save", cx);

        wait_until(cx, "se guarda y deja de editar", |cx| !on_screen(window, "comment-edit-input", cx));
        let sent: Vec<serde_json::Value> =
            asked(&server, "PATCH", SAID).iter().map(|request| serde_json::from_slice(&request.body).unwrap()).collect();
        assert_eq!(sent, [json!({ "body": "mejor dicho" })]);
        stop(cx);
    }

    /// Si la edición no se guarda, se dice y se sigue editando con lo
    /// escrito.
    ///
    /// Rotura que atrapa: perder lo reescrito por un fallo de red.
    #[gpui_kit::test]
    fn a_comment_edit_that_fails_keeps_what_was_written(cx: &mut TestAppContext) {
        // Sin `PATCH` montado: el servidor contesta 404.
        let server = server_with_two_voices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);

        click(window, "comment-edit-said", cx);
        retype(window, &view, "mejor dicho", cx);
        click(window, "comment-edit-save", cx);

        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        assert!(on_screen(window, "comment-edit-input", cx));
        assert_eq!(edit_typed(&view, cx), "mejor dicho");
        stop(cx);
    }

    /// Borrar un comentario pregunta antes; «Cancelar» no borra y «Borrar»
    /// lo pide.
    ///
    /// Roturas que atrapa: un clic que borra sin preguntar; un «Cancelar»
    /// que borra igual; un «Borrar» que no hace nada.
    #[gpui_kit::test]
    fn deleting_a_comment_asks_first(cx: &mut TestAppContext) {
        let server = server_with_two_voices();
        answering(&server, "DELETE");
        let window = open(&server, cx);
        open_first_task(window, cx);
        let deletions = |server: &FakeServer| asked(server, "DELETE", SAID).len();

        click(window, "comment-delete-said", cx);
        assert!(cx.has_pending_prompt(), "no preguntó antes de borrar");
        cx.simulate_prompt_answer("Cancelar");
        settle(cx);
        assert_eq!(deletions(&server), 0, "borró aunque se canceló");

        click(window, "comment-delete-said", cx);
        cx.simulate_prompt_answer("Borrar");
        wait_until(cx, "se pide borrarlo", |_| deletions(&server) == 1);
        stop(cx);
    }

    /// En una ventana ancha las propiedades van en su riel, al lado de la
    /// lectura; en una estrecha, debajo. En las dos están.
    ///
    /// Roturas que atrapa: un riel apretado al lado en una ventana estrecha,
    /// donde ninguna mitad se lee; perder las propiedades al estrechar.
    #[gpui_kit::test]
    fn the_properties_sit_beside_the_reading_only_when_there_is_room(cx: &mut TestAppContext) {
        for (width, beside) in [(super::STACK_BELOW, true), (super::STACK_BELOW - 1., false)] {
            let server = server();
            let (window, _) = open_sized(&server, width, cx);
            open_first_task(window, cx);

            assert_eq!(on_screen(window, "task-wide", cx), beside, "a {width} px");
            assert!(on_screen(window, "task-properties", cx) && on_screen(window, "task-priority", cx), "a {width} px");
            stop(cx);
            cx.update_window(window, |_, window, _| window.remove_window()).unwrap();
        }
    }

    fn copied(cx: &mut TestAppContext) -> Option<String> {
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    /// La referencia y el id de la tarea se copian de un clic, cada uno el suyo.
    ///
    /// Roturas que atrapa: una pastilla que parece copiar y no copia nada;
    /// copiar el id donde se pulsó la referencia (lo que se pega en un
    /// commit), o al revés.
    #[gpui_kit::test]
    fn the_reference_and_the_id_are_copied_from_their_chips(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        open_first_task(window, cx);

        click(window, "task-ref", cx);
        assert_eq!(copied(cx).as_deref(), Some("acme-7"));
        click(window, "task-id", cx);
        assert_eq!(copied(cx).as_deref(), Some("t1"));
        stop(cx);
    }

    fn open_task_id(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.clone())
    }

    /// La madre y las tareas a las que espera se abren desde su nombre.
    ///
    /// Roturas que atrapa: un nombre subrayado que no lleva a ningún sitio;
    /// abrir la madre al pulsar una dependencia; no enseñar de qué depende.
    #[gpui_kit::test]
    fn the_parent_and_what_it_waits_for_open_from_their_names(cx: &mut TestAppContext) {
        let server = server();
        let mut first = detail("t1");
        first["blockedBy"] = json!([{ "id": "t9", "ref": "acme-3", "title": "La API", "open": true }]);
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(first)).with_priority(1));
        let window = open(&server, cx);
        open_first_task(window, cx);
        assert!(on_screen(window, "task-parent", cx) && on_screen(window, "task-blocker-t9", cx));

        click(window, "task-blocker-t9", cx);
        wait_until(cx, "abre aquélla a la que espera", |cx| open_task_id(cx).as_deref() == Some("t9"));

        cx.update(|cx| Backend::run(cx, |core| async move { core.tasks().open("t1").await }));
        wait_until(cx, "vuelve la primera", |cx| on_screen(window, "task-parent", cx));
        click(window, "task-parent", cx);
        wait_until(cx, "abre la madre", |cx| open_task_id(cx).as_deref() == Some("t2"));
        stop(cx);
    }

    /// La × de la pastilla de una persona le quita la tarea.
    ///
    /// Roturas que atrapa: una × que no pide nada; quitar a otra persona, o
    /// añadirla otra vez en vez de quitarla.
    #[gpui_kit::test]
    fn the_cross_on_a_person_takes_the_task_from_them(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let window = open(&server, cx);
        open_first_task(window, cx);
        wait_until(cx, "sale quien la lleva", |cx| on_screen(window, "task-unassign-u1", cx));

        click(window, "task-unassign-u1", cx);

        wait_until(cx, "pide quitársela", |_| !patches(&server).is_empty());
        assert_eq!(patches(&server), [json!({ "assigneeIds": [] })]);
        stop(cx);
    }

    /// Lo que la app de siempre tiene en esta página y aquí todavía no se
    /// hace está pintado en su sitio, y el pie dice cuándo se tocó.
    ///
    /// Roturas que atrapa: dejar fuera una pieza por no tener aún qué hacer
    /// con ella (el ojo la echa en falta al comparar las dos apps); decir
    /// que un cliente la lee en una tarea interna, o callarlo en una que lee.
    #[gpui_kit::test]
    fn what_is_not_here_yet_is_still_painted_in_its_place(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        open_first_task(window, cx);

        for id in ["task-handoff-edit", "task-add-subtask", "comment-decision", "task-delete", "task-withdraw", "task-updated", "task-visible-to-client"] {
            assert!(on_screen(window, id, cx), "falta {id}");
        }
        stop(cx);
        cx.update_window(window, |_, window, _| window.remove_window()).unwrap();

        // Una tarea del equipo, sin madre: nadie de fuera la lee, no hay de
        // dónde retirarla, y sí tiene su lista en el riel.
        let other = self::server();
        let mut internal = detail("t1");
        internal["task"]["projectId"] = json!("");
        internal["parent"] = serde_json::Value::Null;
        other.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(internal)).with_priority(1));
        let window = open(&other, cx);
        open_first_task(window, cx);
        assert!(on_screen(window, "task-delete", cx) && on_screen(window, "task-list", cx));
        assert!(!on_screen(window, "task-withdraw", cx) && !on_screen(window, "task-visible-to-client", cx));
        assert!(!on_screen(window, "task-parent", cx));
        stop(cx);
    }

    /// Deja la caja de comentar con ese texto y esa selección (en bytes).
    fn compose(window: AnyWindowHandle, view: &Entity<TaskView>, text: &str, chosen: std::ops::Range<usize>, cx: &mut TestAppContext) {
        let text = text.to_string();
        with_view(window, view, cx, |view, window, cx| {
            view.comment.update(cx, |comment, cx| {
                comment.set_value(text, window, cx);
                comment.set_selected_range(chosen, cx);
            });
        });
    }

    fn chosen(view: &Entity<TaskView>, cx: &mut TestAppContext) -> std::ops::Range<usize> {
        cx.update(|cx| view.read(cx).comment.read(cx).selected_range())
    }

    /// Cada botón de la barra le hace lo suyo a lo escrito, deja la
    /// selección donde el core dice y devuelve el foco a la caja.
    ///
    /// Roturas que atrapa: un botón pintado que no hace nada; uno que hace
    /// lo de otro; no pasarle al core lo seleccionado (envuelve en el cursor
    /// y no la palabra); no poner de vuelta la selección, o quedarse el foco
    /// en el botón, con lo que lo siguiente que se teclea no va a donde toca.
    #[gpui_kit::test]
    fn each_button_of_the_bar_does_its_thing_to_what_is_written(cx: &mut TestAppContext) {
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        let table = "|   |   |   |\n| --- | --- | --- |\n|   |   |   |\n|   |   |   |";
        let buttons = [
            ("comment-bold", "****"),
            ("comment-italic", "**"),
            ("comment-code", "``"),
            ("comment-heading", "## "),
            ("comment-bullets", "- "),
            ("comment-numbers", "1. "),
            ("comment-checklist", "- [ ] "),
            ("comment-quote", "> "),
            ("comment-table", table),
            ("comment-link", "[]()"),
        ];
        for (id, written) in buttons {
            compose(window, &view, "", 0..0, cx);
            click(window, id, cx);
            assert_eq!(typed(&view, cx), written, "{id}");
        }

        compose(window, &view, "di hola ya", 3..7, cx);
        click(window, "comment-bold", cx);
        assert_eq!(typed(&view, cx), "di **hola** ya");
        assert_eq!(chosen(&view, cx), 5..9);
        type_more(window, "X", cx);
        assert_eq!(typed(&view, cx), "di **X** ya", "lo tecleado tras el botón no fue a lo seleccionado");
        stop(cx);
    }

    /// Los atajos de tiptap hacen lo que sus botones: Ctrl con B, I, E y K.
    ///
    /// Roturas que atrapa: un atajo que no hace nada, o que hace lo de otro;
    /// que la letra además se escriba; coger la letra sin Ctrl, y no poder
    /// escribir una «b».
    #[gpui_kit::test]
    fn the_shortcuts_do_what_their_buttons_do(cx: &mut TestAppContext) {
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        write(window, "x", cx);
        for (key, written) in [("secondary-b", "**hola**"), ("secondary-i", "*hola*"), ("secondary-e", "`hola`"), ("secondary-k", "[hola]()")] {
            compose(window, &view, "hola", 0..4, cx);
            press(window, key, cx);
            assert_eq!(typed(&view, cx), written, "{key}");
        }
        compose(window, &view, "", 0..0, cx);
        type_more(window, "bike", cx);
        assert_eq!(typed(&view, cx), "bike");
        stop(cx);
    }

    /// Enter dentro de una lista sigue con otro elemento, y en uno vacío sale
    /// de ella; fuera de una lista, y con Mayúsculas, es un salto de línea.
    ///
    /// Roturas que atrapa: una lista que hay que seguir a mano; el elemento
    /// nuevo **y además** el salto de línea del campo; quedarse con Enter
    /// donde no hay lista, y no poder partir una línea.
    #[gpui_kit::test]
    fn enter_carries_a_list_on_in_the_editor(cx: &mut TestAppContext) {
        let server = server();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        write(window, "- uno", cx);
        press(window, "enter", cx);
        assert_eq!(typed(&view, cx), "- uno\n- ");
        type_more(window, "dos", cx);
        press(window, "shift-enter", cx);
        assert_eq!(typed(&view, cx), "- uno\n- dos\n");
        press(window, "enter", cx);
        assert_eq!(typed(&view, cx), "- uno\n- dos\n\n");

        compose(window, &view, "- uno\n- ", 8..8, cx);
        press(window, "enter", cx);
        assert_eq!(typed(&view, cx), "- uno\n");
        stop(cx);
    }

    fn is_off(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).disabled() == Some(true)
        })
        .unwrap()
    }

    /// «Comentar» está apagado mientras no hay nada escrito, y «Guardar» al
    /// corregir un comentario, lo mismo.
    ///
    /// Roturas que atrapa: un botón encendido que no hace nada al pulsarlo
    /// (la caja está vacía, o sólo tiene espacios); uno que se queda apagado
    /// con algo escrito.
    #[gpui_kit::test]
    fn the_send_button_is_off_while_nothing_is_written(cx: &mut TestAppContext) {
        let server = server_with_two_voices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert!(is_off(window, "comment-send", cx));
        write(window, "  ", cx);
        assert!(is_off(window, "comment-send", cx), "encendido con sólo espacios");
        type_more(window, "hola", cx);
        assert!(!is_off(window, "comment-send", cx), "apagado con algo escrito");

        click(window, "comment-edit-said", cx);
        assert!(!is_off(window, "comment-edit-save", cx));
        retype(window, &view, " \n", cx);
        assert!(is_off(window, "comment-edit-save", cx));
        stop(cx);
    }

    /// La descripción y el comentario que se corrige se escriben en el mismo
    /// editor que el comentario nuevo: con su barra, y con su clip.
    ///
    /// Roturas que atrapa: una caja de texto pelada donde la app de siempre
    /// tiene el editor; una barra cuyos botones escriben en otra caja (la
    /// del comentario nuevo).
    #[gpui_kit::test]
    fn the_description_and_an_edited_comment_use_the_same_editor(cx: &mut TestAppContext) {
        let server = server_with_two_voices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);

        click(window, "task-edit", cx);
        assert!(on_screen(window, "task-description-input", cx) && on_screen(window, "task-description-attach", cx));
        rewrite(window, &view, "título", "cuerpo", cx);
        click(window, "task-description-quote", cx);
        assert_eq!(written(&view, cx).1, "> cuerpo");

        click(window, "comment-edit-said", cx);
        assert!(on_screen(window, "comment-edit-input", cx) && on_screen(window, "comment-edit-attach", cx));
        retype(window, &view, "dicho", cx);
        click(window, "comment-edit-heading", cx);
        assert_eq!(edit_typed(&view, cx), "## dicho");
        assert_eq!(typed(&view, cx), "", "la barra de otra caja escribió en la de comentar");
        stop(cx);
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\xff";

    /// Lo que se suelta o se pega en una caja se adjunta a la tarea y se
    /// cita en **esa** caja; el texto que se pega sigue siendo texto.
    ///
    /// Roturas que atrapa: una barra que promete «pega o suelta ficheros» y
    /// no hace nada con ellos; subir y no citar; citar en la caja de
    /// comentar lo que se soltó en la descripción; una imagen pegada sin
    /// nombre de imagen (se subiría como un fichero cualquiera y no se
    /// vería); tragarse un texto pegado creyéndolo un fichero.
    #[gpui_kit::test]
    fn what_is_dropped_or_pasted_in_a_box_is_attached_and_cited_there(cx: &mut TestAppContext) {
        use gpui_kit::{ClipboardItem, ExternalPaths, FileDropEvent, Image, ImageFormat, PlatformInput};
        let server = server();
        storing(&server, "a2", "pasted.png");
        storing(&server, "a3", "notas.txt");
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        let uploads = |server: &FakeServer| asked(server, "POST", "/api/v1/tasks/t1/attachments");

        // Un texto pegado es texto.
        write(window, "a", cx);
        cx.write_to_clipboard(ClipboardItem::new_string("b".into()));
        cx.update_window(window, |_, window, cx| window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx)).unwrap();
        settle(cx);
        assert_eq!(typed(&view, cx), "ab");
        assert!(uploads(&server).is_empty());

        // Una imagen pegada en la caja de comentar.
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, PNG.to_vec())));
        cx.update_window(window, |_, window, cx| window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx)).unwrap();
        wait_until(cx, "sube la imagen pegada y queda citada", |cx| {
            typed(&view, cx) == "ab\n![pasted.png](/api/v1/tasks/t1/attachments/a2/raw)\n"
        });
        assert_eq!(uploads(&server)[0].body, cac_core::multipart("pasted.png", PNG).1);

        // Un fichero soltado en la descripción, que se está editando.
        let dir = scratch("drop");
        let notes = dir.join("notas.txt");
        std::fs::write(&notes, b"hola").unwrap();
        click(window, "task-edit", cx);
        rewrite(window, &view, "título", "", cx);
        let before = typed(&view, cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let position = window.find("task-description-input").bounds().center();
            let paths = ExternalPaths(vec![notes.clone()].into());
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position, paths }), cx);
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Submit { position }), cx);
        })
        .unwrap();
        wait_until(cx, "sube lo soltado y queda citado en la descripción", |cx| {
            written(&view, cx).1 == "[notas.txt](/api/v1/tasks/t1/attachments/a3/raw) "
        });
        assert_eq!(uploads(&server)[1].body, cac_core::multipart("notas.txt", b"hola").1);
        assert_eq!(typed(&view, cx), before, "lo soltado en la descripción se citó en el comentario");
        assert!(!on_screen(window, "task-problem", cx));
        let _ = std::fs::remove_dir_all(&dir);
        stop(cx);
    }

    fn label_of(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).label().unwrap_or_default().to_string()
        })
        .unwrap()
    }

    /// Cuántas piezas hay a la vista: sube cuando se abre un desplegable.
    fn pieces(window: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
        use gpui_kit::test::TestQueryExt;
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.elements().len()
        })
        .unwrap()
    }

    /// El vencimiento es un botón con el día escrito con números, como en la
    /// app de siempre; pulsarlo abre el calendario y su × quita la fecha.
    ///
    /// Roturas que atrapa: el día con el año entero o en otro orden; un botón
    /// que enseña el día de la tarea y no el que se acaba de elegir; un
    /// botón pintado que no abre nada (el calendario del kit tapado o
    /// recortado); una × que no quita la fecha, o que sale sin fecha que quitar.
    #[gpui_kit::test]
    fn the_due_day_is_a_button_that_says_it_with_numbers(cx: &mut TestAppContext) {
        let server = server_with_choices();
        let (window, view) = open_with_view(&server, cx);
        open_first_task(window, cx);
        assert_eq!(label_of(window, "task-due-day", cx), "30/9/26");

        let closed = pieces(window, cx);
        click(window, "task-due", cx);
        cx.run_until_parked();
        assert!(pieces(window, cx) > closed, "pulsar el botón no abrió el calendario");
        press(window, "escape", cx);
        cx.run_until_parked();
        assert_eq!(pieces(window, cx), closed, "Escape no cerró el calendario");

        pick(window, &view, day(2026, 10, 2), cx);
        assert_eq!(label_of(window, "task-due-day", cx), "2/10/26", "no dice el día recién elegido");
        wait_until(cx, "se guarda el día elegido", |_| patches(&server).len() == 1);

        click(window, "task-due-clear", cx);
        wait_until(cx, "se pide quitar la fecha", |_| patches(&server).len() == 2);
        assert_eq!(patches(&server)[1], json!({ "clearDueAt": true }));
        stop(cx);
        cx.update_window(window, |_, window, _| window.remove_window()).unwrap();

        let other = self::server();
        let mut without = detail("t1");
        without["task"]["dueAt"] = serde_json::Value::Null;
        other.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(without)).with_priority(1));
        let window = open(&other, cx);
        open_first_task(window, cx);
        assert!(on_screen(window, "task-due", cx) && !on_screen(window, "task-due-clear", cx));
        stop(cx);
    }

    /// La sección de adjuntos sólo está cuando hay alguno, y «Enlazar una
    /// PR» sólo donde no hay código enlazado.
    ///
    /// Roturas que atrapa: una sección «Adjuntos» vacía en cada tarea (se
    /// adjunta desde el clip del editor); no enseñarla cuando sí hay; la
    /// línea de enlazar una PR en una tarea que ya tiene la suya, o que
    /// falte en la que no tiene nada.
    #[gpui_kit::test]
    fn attachments_and_the_pr_line_are_there_only_when_they_apply(cx: &mut TestAppContext) {
        // La del contrato: sin ficheros suyos, con código enlazado.
        let server = server();
        let window = open(&server, cx);
        open_first_task(window, cx);
        assert!(!on_screen(window, "task-attachments", cx));
        assert!(!on_screen(window, "task-link-pr", cx));
        stop(cx);
        cx.update_window(window, |_, window, _| window.remove_window()).unwrap();

        // Con un fichero y sin código.
        let other = self::server();
        let mut bare = detail("t1");
        bare["git"] = serde_json::Value::Null;
        bare["attachments"] = json!([{
            "id": "a1", "itemId": "t1", "commentId": null, "url": RAW, "fileName": "plan.pdf",
            "contentType": "application/pdf", "bytes": 12288,
        }]);
        other.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/t1")).respond_with(ok(bare)).with_priority(1));
        let window = open(&other, cx);
        open_first_task(window, cx);
        assert!(on_screen(window, "task-attachments", cx) && on_screen(window, "task-file-a1", cx));
        assert!(on_screen(window, "task-link-pr", cx));
        stop(cx);
    }
}
