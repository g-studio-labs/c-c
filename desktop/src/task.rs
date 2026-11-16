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
    ApiError, Choice, DueState, MentionMark, MentionMenu, MentionPerson, SheetComment, SheetFile, SheetSubtask, StatusOption, TaskAttachment, TaskSheet,
    PRIORITIES,
};
use chrono::NaiveDate;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter, Escape, IndentInline, Input, InputState, MoveDown, MoveUp, Position, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::files;
use crate::lang::{error_text, locale, tr};

/// El ancho del riel de propiedades: el de la app de siempre (`lg:w-72`).
const RAIL_WIDTH: f32 = 288.;
/// Por debajo de este ancho de ventana el riel va debajo de la lectura (el
/// `lg` de la app de siempre).
pub(crate) const STACK_BELOW: f32 = 1024.;

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
    /// Hay ficheros subiendo: treinta megas tardan, y un botón quieto parece
    /// no haber hecho nada.
    attaching: bool,
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
    /// Hay ficheros subiendo para citarlos en el comentario.
    citing: bool,
    /// Las imágenes de la tarea abierta, por el id de su adjunto. Se bajan
    /// una vez, al verla, y se tiran al abrir otra.
    thumbs: HashMap<String, Thumb>,
    _task_closed: Subscription,
}

/// Sube esos ficheros a la tarea abierta, uno tras otro, y al primero que no
/// se pueda se para. Dice cuáles quedaron subidos y, si lo hubo, cuál no y
/// por qué (sin porqué si no se pudo ni leer del disco).
async fn upload_all(core: cac_core::Core, paths: Vec<std::path::PathBuf>) -> (Vec<TaskAttachment>, Option<(String, Option<ApiError>)>) {
    let (mut attached, mut failed) = (Vec::new(), None);
    for path in paths {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        // El tamaño se mira antes de leer: un fichero enorme no se carga en
        // memoria para que digan que no. (Sin esto el core lo rechaza igual,
        // ya leído: no hay prueba que lo distinga sin medir memoria.)
        let read = std::fs::metadata(&path)
            .map_err(|_| None)
            .and_then(|file| cac_core::attachment_fits(file.len()).map_err(Some))
            .and_then(|()| std::fs::read(&path).map_err(|_| None));
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

/// Con qué tono se pinta una prioridad.
fn priority_tone(priority: &str, cx: &App) -> Hsla {
    let theme = cx.theme();
    match priority {
        "urgent" => theme.danger,
        "high" => theme.warning,
        "normal" => theme.info,
        _ => theme.muted_foreground,
    }
}

impl TaskView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "work:task.writeComment");
        let due = cx.new(|cx| DatePickerState::new(window, cx).date_format("%d/%m/%Y"));
        // El selector avisa cuando alguien elige un día o lo quita; ponerle
        // desde aquí el de la tarea no avisa.
        let due_changed = cx.subscribe_in(&due, window, |this, _, event: &DatePickerEvent, window, cx| {
            let DatePickerEvent::Change(value) = event;
            this.set_due(value.format("%Y-%m-%d").map(|day| day.to_string()), window, cx);
        });
        Self {
            thread: Thread::Talk,
            thread_of: None,
            problem: None,
            comment: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 8).placeholder(placeholder)),
            internal: false,
            editing: None,
            title_input: cx.new(|cx| InputState::new(window, cx)),
            body_input: cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 24)),
            due,
            due_shown: None,
            _due_changed: due_changed,
            attaching: false,
            editing_comment: None,
            comment_edit: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 12)),
            comment_promise: "work:task.writeComment",
            page_focus: cx.focus_handle(),
            focused_for: None,
            mention_mark: None,
            mention_scroll: ScrollHandle::new(),
            citing: false,
            thumbs: HashMap::new(),
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

    /// Pregunta qué ficheros y los sube a la tarea abierta, uno tras otro. Al
    /// primero que no se pueda se para y se dice cuál.
    pub(crate) fn attach_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = Self::open_task_id(cx) else { return };
        self.problem = None;
        cx.notify();
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            let Ok(answer) = this.update(cx, |this, cx| {
                this.attaching = true;
                cx.notify();
                Backend::ask(cx, move |core| upload_all(core, paths))
            }) else {
                return;
            };
            let result = answer.await;
            this.update(cx, |this, cx| {
                this.attaching = false;
                if let Ok((_, Some((name, error)))) = result {
                    this.say_not_attached(task_id, &name, error, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Pregunta qué ficheros, los sube a la tarea y los cita en el
    /// comentario que se escribe, donde está el cursor. Como en la app de
    /// siempre: un fichero que se adjunta al comentar es de la tarea, y el
    /// comentario lo nombra. Cómo se cita y dónde lo dice el core.
    pub(crate) fn attach_to_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = Self::open_task_id(cx) else { return };
        self.problem = None;
        cx.notify();
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            let Ok(answer) = this.update(cx, |this, cx| {
                this.citing = true;
                cx.notify();
                Backend::ask(cx, move |core| upload_all(core, paths))
            }) else {
                return;
            };
            let result = answer.await;
            this.update_in(cx, |this, window, cx| {
                this.citing = false;
                cx.notify();
                let Ok((files, failed)) = result else { return };
                // Lo escrito es para la tarea en la que se adjuntó: si ya
                // se mira otra, los ficheros quedan en la suya y no se
                // citan en un comentario de ésta.
                if Self::open_task_id(cx).as_deref() == Some(task_id.as_str()) {
                    for file in &files {
                        // Con lo que la caja dice ahora, no con lo que
                        // decía al elegir: subir tarda, y se sigue escribiendo.
                        let (text, cursor) = this.comment_cursor(cx);
                        let Some(cited) = cac_core::with_citation(&text, cursor, file) else { continue };
                        let position = position_at(&cited.text, cited.cursor as usize);
                        this.comment.update(cx, |comment, cx| {
                            comment.set_value(cited.text, window, cx);
                            comment.set_cursor_position(position, window, cx);
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
        crate::theme::prose_with_links(id, text, move |url, window, cx| {
            let _ = view.update(cx, |this, cx| this.follow_link(url, window, cx));
        })
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
    /// lado guardar una copia y quitarlo.
    fn render_file(&self, file: &SheetFile, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, primary) = (theme.muted_foreground, theme.primary);
        let icon = Lucide::Paperclip;
        let (open, save, remove) = (file.clone(), file.clone(), file.clone());
        div()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .child(Icon::new(icon).xsmall().text_color(muted))
            .child(
                div()
                    .id(SharedString::from(format!("task-file-{}", file.id)))
                    .test_support()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(primary)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.open_file(open.id.clone(), open.name.clone(), window, cx)))
                    .child(file.name.clone()),
            )
            .children(file.size.clone().map(|size| div().flex_none().text_xs().text_color(muted).child(size)))
            .children(file.cited.then(|| div().flex_none().text_xs().text_color(muted).child(tr(cx, "work:task.citedHere"))))
            .child(div().flex_1())
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
    /// deja cambiarlo antes de mandar.
    fn render_composer(&self, client_reads: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let internal = client_reads && self.internal;
        let audience = client_reads.then(|| {
            Button::new("comment-audience")
                .ghost()
                .small()
                .label(tr(cx, if internal { "work:task.internalNote" } else { "work:task.theClientReads" }))
                .tooltip(tr(cx, if internal { "work:task.onlyTeamWillSee" } else { "work:task.clientReadsSwitch" }))
                .on_click(cx.listener(move |this, _, _, cx| this.set_internal(!internal, cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("comment-input")
                    .test_support()
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
                    .child(Textarea::new(&self.comment)),
            )
            .children(self.render_mentions(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("comment-attach")
                            .ghost()
                            .small()
                            .icon(Icon::new(Lucide::Paperclip))
                            .tooltip(tr(cx, "common:editor.attachFile"))
                            .loading(self.citing)
                            .on_click(cx.listener(|this, _, window, cx| this.attach_to_comment(window, cx))),
                    )
                    .child(div().flex_1())
                    .children(audience)
                    .child(
                    Button::new("comment-send")
                        .primary()
                        .small()
                        .label(tr(cx, if internal { "work:task.commentInternally" } else { "work:task.comment" }))
                        .on_click(cx.listener(|this, _, window, cx| this.send_comment(window, cx))),
                ),
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

    fn section(&self, title: String, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        div().text_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(title.to_uppercase())
    }

    fn render_chips(&self, sheet: &TaskSheet, statuses: Vec<StatusOption>, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let current = sheet.status_name.clone();
        let theme = cx.theme();
        let (muted, danger, warning, accent) = (theme.muted_foreground, theme.danger, theme.warning, theme.accent);
        let chip = |tone: Hsla, label: String| div().flex_none().px_2().rounded_md().bg(tone.opacity(0.15)).text_xs().text_color(tone).child(label);
        let status_tone = gpui_kit::rgb(u32::from_str_radix(sheet.status_color.trim_start_matches('#'), 16).unwrap_or(0x888888));
        let due = sheet.due.clone().map(|due| {
            let tone = match due.state {
                DueState::Overdue | DueState::Yesterday => danger,
                DueState::Today | DueState::Tomorrow => warning,
                DueState::Later => muted,
            };
            chip(tone, due.text)
        });

        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .pl_2()
                    .rounded_md()
                    .bg(accent)
                    .child(div().size(px(8.)).rounded_full().bg(status_tone))
                    // Las columnas de su lista; la que su máquina no deja
                    // sale apagada en vez de ser un botón que contesta 409.
                    .child(
                        Button::new("task-status")
                            .ghost()
                            .xsmall()
                            .label(sheet.status_name.clone())
                            .dropdown_caret(!statuses.is_empty())
                            .dropdown_menu(move |mut menu, _, _| {
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
                            }),
                    ),
            )
            .children(due)
    }

    /// Un desplegable de cosas que se ponen y se quitan: lo que la tarea
    /// tiene, marcado. Sin nada puesto, el botón dice qué se puede hacer.
    fn render_choices(
        &self,
        id: &'static str,
        empty_key: &str,
        choices: Vec<Choice>,
        toggle: fn(&mut Self, String, &mut Window, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let chosen: Vec<String> = choices.iter().filter(|choice| choice.chosen).map(|choice| choice.name.clone()).collect();
        let label = if chosen.is_empty() { tr(cx, empty_key) } else { chosen.join(", ") };
        Button::new(id).ghost().xsmall().label(label).dropdown_caret(!choices.is_empty()).dropdown_menu(move |mut menu, _, _| {
            for choice in choices.clone() {
                let view = view.clone();
                menu = menu.item(PopupMenuItem::new(choice.name).checked(choice.chosen).on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| toggle(this, choice.id.clone(), window, cx));
                }));
            }
            menu
        })
    }

    /// Lo que se le cambia a la tarea de un toque, cada cosa en su fila.
    fn render_fields(&self, sheet: &TaskSheet, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let view = cx.entity().downgrade();
        let tasks = Backend::core(cx).tasks();
        let current = sheet.priority.clone();
        // Apiladas, no en dos columnas: en un riel de 288 px una etiqueta al
        // lado deja al control sin sitio.
        let row = |name: String, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(muted).child(name))
                .child(div().min_w_0().flex().child(control))
        };
        let priority = Button::new("task-priority")
            .ghost()
            .xsmall()
            .label(tr(cx, &format!("work:taskPriority.{current}")))
            .text_color(priority_tone(&current, cx))
            .dropdown_caret(true)
            .dropdown_menu({
                let names: Vec<(String, String)> =
                    PRIORITIES.iter().map(|priority| (priority.to_string(), tr(cx, &format!("work:taskPriority.{priority}")))).collect();
                move |mut menu, _, _| {
                    for (priority, name) in names.clone() {
                        let view = view.clone();
                        menu = menu.item(PopupMenuItem::new(name).checked(priority == current).on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| this.set_priority(priority.clone(), window, cx));
                        }));
                    }
                    menu
                }
            });
        let tags = self.render_choices("task-tags", "work:task.addTags", tasks.tag_choices(), Self::toggle_tag, cx);
        let assignees = self.render_choices("task-assignees", "work:task.assign", tasks.assignee_choices(), Self::toggle_assignee, cx);
        let due = div().id("task-due").test_support().w(px(170.)).child(
            DatePicker::new(&self.due).small().cleanable(true).placeholder(tr(cx, "common:last.clearDate")),
        );

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(row(tr(cx, "work:board.priority"), priority.into_any_element()))
            .child(row(tr(cx, "work:task.tags"), tags.into_any_element()))
            .child(row(tr(cx, "work:task.assignees"), assignees.into_any_element()))
            .child(row(tr(cx, "work:task.due"), due.into_any_element()))
    }

    fn render_subtask(&self, subtask: &SheetSubtask, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, success) = (theme.muted_foreground, theme.border, theme.success);
        div()
            .id(SharedString::from(format!("subtask-{}", subtask.id)))
            .test_support()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .child(
                div()
                    .id(SharedString::from(format!("subtask-tick-{}", subtask.id)))
                    .test_support()
                    .size(px(14.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(if subtask.done { success } else { border })
                    .cursor_pointer()
                    .on_click({
                        let (id, done) = (subtask.id.clone(), subtask.done);
                        cx.listener(move |this, _, window, cx| this.tick_subtask(id.clone(), !done, window, cx))
                    })
                    .children(subtask.done.then(|| {
                        div()
                            .id(SharedString::from(format!("subtask-done-{}", subtask.id)))
                            .test_support()
                            .child(Icon::new(IconName::Check).xsmall().text_color(success))
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .when(subtask.done, |this| this.text_color(muted).line_through())
                    .child(subtask.title.clone()),
            )
            .child(div().flex_none().text_xs().text_color(muted).child(format!("#{}", subtask.seq)))
    }

    fn render_comment(&self, comment: &SheetComment, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, accent) = (theme.muted_foreground, theme.border, theme.accent);
        let id = SharedString::from(format!("comment-{}", comment.id));
        // Una línea del servidor es un apunte, no alguien hablando.
        if comment.system {
            return div()
                .id(id)
                .test_support()
                .text_xs()
                .text_color(muted)
                .child(format!("{} · {}", comment.body, comment.when))
                .into_any_element();
        }
        let editing = self.editing_comment.as_deref() == Some(comment.id.as_str());
        let audience = comment.audience.as_ref().map(|key| {
            div().flex_none().px_1().rounded_sm().bg(accent).text_xs().text_color(muted).child(tr(cx, key))
        });
        div()
            .id(id)
            .test_support()
            .flex()
            .flex_col()
            .gap_1()
            .pb_3()
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(comment.author.clone()))
                    .child(div().text_xs().text_color(muted).child(comment.when.clone()))
                    .children(comment.edited.then(|| div().text_xs().text_color(muted).child(tr(cx, "work:task.edited"))))
                    .children(audience)
                    .child(div().flex_1())
                    // Sólo en lo que quien mira puede cambiar: el servidor
                    // rechazaría lo demás.
                    .children((comment.mine && !editing).then(|| {
                        let (edit_id, body, delete_id) = (comment.id.clone(), comment.body.clone(), comment.id.clone());
                        div()
                            .flex()
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
                    .child(div().id("comment-edit-input").test_support().child(Textarea::new(&self.comment_edit)))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_1())
                            .child(
                                Button::new("comment-edit-cancel")
                                    .ghost()
                                    .small()
                                    .label(tr(cx, "work:task.cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_editing_comment(cx))),
                            )
                            .child(
                                Button::new("comment-edit-save")
                                    .primary()
                                    .small()
                                    .label(tr(cx, "work:task.save"))
                                    .on_click(cx.listener(|this, _, window, cx| this.save_comment_edit(window, cx))),
                            ),
                    )
                    .into_any_element()
            } else {
                self.prose(format!("comment-body-{}", comment.id), comment.body.clone(), cx).into_any_element()
            })
            .into_any_element()
    }

    /// El riel: el estado y lo que se le cambia a la tarea de un toque.
    fn render_properties(&self, sheet: &TaskSheet, statuses: Vec<StatusOption>, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("task-properties")
            .test_support()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.render_chips(sheet, statuses, cx))
            .child(self.render_fields(sheet, cx))
    }

    fn render_sheet(&mut self, sheet: TaskSheet, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let danger = theme.danger;
        let problem = self.problem.as_ref().filter(|(task_id, _)| *task_id == sheet.id).map(|(_, phrase)| phrase.clone());
        let (muted, border, primary, foreground, card, radius) =
            (theme.muted_foreground, theme.border, theme.primary, theme.foreground, theme.group_box, theme.radius);
        // La pestaña elegida era de otra tarea: en ésta se empieza por la conversación.
        let thread = if self.thread_of.as_deref() == Some(sheet.id.as_str()) { self.thread } else { Thread::Talk };
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

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(if editing {
                        div().id("task-title-input").test_support().child(Input::new(&self.title_input)).into_any_element()
                    } else {
                        div().id("task-title").test_support().text_size(px(crate::theme::TITLE_SIZE)).font_weight(FontWeight::SEMIBOLD).child(sheet.title.clone()).into_any_element()
                    })
                    .child(div().text_xs().text_color(muted).child(sheet.place.clone()))
                    .children(sheet.parent.clone().map(|parent| div().text_xs().text_color(muted).child(format!("› {parent}")))),
            )
            .children(problem.map(|phrase| div().id("task-problem").test_support().text_sm().text_color(danger).child(phrase)))
            .children(sheet.handoff.clone().map(|handoff| {
                let by = sheet.handoff_by.clone().map(|by| format!(" · {by}")).unwrap_or_default();
                div()
                    .id("task-handoff")
                    .test_support()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_3()
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .bg(card)
                    .child(self.section(format!("{}{by}", tr(cx, "work:task.handoff")), cx))
                    .child(self.prose(format!("handoff-{}", sheet.id), handoff, cx))
            }))
            .child(
                div().flex().flex_col().gap_1().child(self.section(tr(cx, "work:task.description"), cx)).child(
                    if editing {
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().id("task-description-input").test_support().child(Textarea::new(&self.body_input)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("task-cancel")
                                            .ghost()
                                            .small()
                                            .label(tr(cx, "work:task.cancel"))
                                            .on_click(cx.listener(|this, _, _, cx| this.cancel_editing(cx))),
                                    )
                                    .child(
                                        Button::new("task-save")
                                            .primary()
                                            .small()
                                            .label(tr(cx, "work:task.save"))
                                            .on_click(cx.listener(|this, _, window, cx| this.save_edit(window, cx))),
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
                    },
                ),
            )
            .when(!sheet.subtasks.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(self.section(format!("{} · {done}/{}", tr(cx, "work:task.subtasks"), sheet.subtasks.len()), cx))
                        .children(sheet.subtasks.iter().map(|subtask| self.render_subtask(subtask, cx))),
                )
            })
            // Siempre a la vista, también sin ficheros: es donde se adjunta.
            .child(
                div()
                    .id("task-attachments")
                    .test_support()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div().flex().items_center().gap_2().child(self.section(tr(cx, "work:task.attachments"), cx)).child(div().flex_1()).child(
                            Button::new("task-attach")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(Lucide::Paperclip))
                                .label(tr(cx, "common:editor.attachFile"))
                                .loading(self.attaching)
                                .on_click(cx.listener(|this, _, window, cx| this.attach_files(window, cx))),
                        ),
                    )
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
                    .children(sheet.attachments.iter().filter(|file| !file.image).map(|file| self.render_file(file, cx))),
            )
            .child(div().flex().items_center().gap_4().border_b_1().border_color(border).children(tabs.map(
                |(tab, id, key, count)| {
                    let active = thread == tab;
                    let task_id = sheet.id.clone();
                    div()
                        .id(id)
                        .test_support()
                        .pb_2()
                        .border_b_2()
                        .border_color(if active { primary } else { primary.opacity(0.) })
                        .text_sm()
                        .text_color(if active { foreground } else { muted })
                        .when(active, |this| this.font_weight(FontWeight::MEDIUM))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.show_thread(tab, task_id.clone(), cx)))
                        .child(format!("{} ({count})", tr(cx, key)))
                },
            )))
            .when(thread == Thread::Notes, |this| {
                this.child(div().text_xs().text_color(muted).child(tr(cx, "work:task.thread.agentNotesHint")))
            })
            .child(div().flex().flex_col().gap_3().children(comments.iter().map(|comment| self.render_comment(comment, cx))))
            // Se comenta en la conversación; las notas de agentes son suyas.
            .when(thread == Thread::Talk, |this| this.child(self.render_composer(sheet.client_reads, cx)))
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
            if promise != self.comment_promise {
                self.comment_promise = promise;
                let text = tr(cx, promise);
                self.comment.update(cx, |comment, cx| comment.set_placeholder(text, window, cx));
            }
            // Lo recordado de la lista es de un `@`: cuando deja de
            // escribirse, se olvida.
            if self.mention_mark.is_some() {
                let (text, cursor) = self.comment_cursor(cx);
                self.mention_mark = cac_core::mention_mark_kept(&text, cursor, self.mention_mark.take());
            }
        }
        let theme = cx.theme();
        let (background, foreground, muted, border) = (theme.background, theme.foreground, theme.muted_foreground, theme.border);
        let waiting = state.waiting_reason().map(|key| tr(cx, key));
        // Editar se ofrece con la tarea a la vista y mientras no se edita ya.
        let can_edit = sheet.as_ref().is_some_and(|sheet| self.editing.as_deref() != Some(sheet.id.as_str()));
        let edit = can_edit.then(|| {
            Button::new("task-edit")
                .ghost()
                .small()
                .label(tr(cx, "work:task.edit"))
                .on_click(cx.listener(|this, _, window, cx| this.start_editing(window, cx)))
        });
        let name = sheet
            .as_ref()
            .map(|sheet| match &sheet.reference {
                Some(reference) => format!("#{} · {reference}", sheet.seq),
                None => format!("#{}", sheet.seq),
            })
            .unwrap_or_default();

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
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .child(div().p_4().child(reading))
                .children(properties.map(|properties| div().p_4().border_t_1().border_color(border).child(properties)))
                .into_any_element()
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
                    .child(div().flex_1().text_xs().text_color(muted).child(name))
                    .children(edit)
                    .child(
                        Button::new("task-close")
                            .ghost()
                            .small()
                            .icon(Icon::new(Lucide::X))
                            .tooltip(tr(cx, "work:task.close"))
                            .on_click(|_, _, cx| Backend::core(cx).tasks().close()),
                    ),
            )
            .child(body)
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

    /// «Adjuntar» pregunta qué ficheros y los sube a la tarea; uno que pasa
    /// del tope se dice y no se manda.
    ///
    /// Roturas que atrapa: un botón que no pregunta; subir sólo el primero;
    /// mandar otro contenido u otro nombre; cargar y mandar un fichero que
    /// el servidor va a rechazar por grande; callar que uno no se subió.
    #[gpui_kit::test]
    fn attaching_uploads_what_was_chosen(cx: &mut TestAppContext) {
        let server = server_with_a_file();
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/tasks/t1/attachments"))
                .respond_with(wiremock::ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": { "id": "a2" } }))),
        );
        let window = open(&server, cx);
        open_first_task(window, cx);
        let dir = scratch("attach");
        let (notes, shot, huge) = (dir.join("notas.txt"), dir.join("captura.png"), dir.join("enorme.bin"));
        std::fs::write(&notes, b"hola").unwrap();
        std::fs::write(&shot, b"\x89PNG\x00\xff").unwrap();
        // Un byte por encima del tope, sin ocupar disco.
        std::fs::File::create(&huge).unwrap().set_len(cac_core::MAX_ATTACHMENT_BYTES as u64 + 1).unwrap();

        click(window, "task-attach", cx);
        assert!(cx.did_prompt_for_paths(), "no preguntó qué adjuntar");
        let chosen = vec![notes.clone(), shot.clone()];
        cx.simulate_path_prompt_response(move |_| Some(chosen));
        wait_until(cx, "suben los dos", |_| asked(&server, "POST", "/api/v1/tasks/t1/attachments").len() == 2);

        let sent = asked(&server, "POST", "/api/v1/tasks/t1/attachments");
        assert_eq!(sent[0].body, cac_core::multipart("notas.txt", b"hola").1);
        assert_eq!(sent[1].body, cac_core::multipart("captura.png", b"\x89PNG\x00\xff").1);
        assert!(!on_screen(window, "task-problem", cx));

        click(window, "task-attach", cx);
        cx.simulate_path_prompt_response(move |_| Some(vec![huge]));
        wait_until(cx, "dice que no se pudo", |cx| on_screen(window, "task-problem", cx));
        assert_eq!(asked(&server, "POST", "/api/v1/tasks/t1/attachments").len(), 2, "mandó el que pasa del tope");
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

}
