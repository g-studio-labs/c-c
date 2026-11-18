//! El editor de markdown: la caja con su barra de botones encima.
//!
//! En la app de siempre es tiptap (`app/src/components/markdown/
//! MarkdownEditor.tsx`), que es de la web y aquí no existe. Decidido con jose
//! (17-nov-2026): aquí se escribe el markdown tal cual, con la misma barra, y
//! cada botón le hace al texto lo que el de tiptap le hace al documento. Qué
//! le hace lo dice el core (`cac_core::compose`); aquí sólo se le pasa lo
//! escrito y lo seleccionado y se pone de vuelta lo que conteste.
//!
//! Por qué el campo de varias líneas del kit (`TextareaState`) y no su
//! editor de código (`EditorState`): el de código colorea, pero el markdown
//! pide compilar dos gramáticas de tree-sitter que la app no lleva; no crece
//! con lo escrito (va a filas fijas, y la caja de comentar tiene que empezar
//! baja y crecer); no enseña un texto de ejemplo mientras está vacío, y trae
//! modales de programar (números de línea, sangrado con tabulador, cierre de
//! paréntesis, buscar) que en un comentario estorban. El campo de varias
//! líneas ya hacía lo que la página necesita —las menciones cogen sus teclas
//! antes que él, crece solo— y da la selección en bytes, que es como el core
//! la pide. Lo que se pierde es el color al escribir.

use std::path::PathBuf;
use std::rc::Rc;

use cac_core::{ComposeAction, ComposeEdit};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{Enter, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::lang::tr;
use crate::theme::Leading as _;

/// Algo que llegó a la caja para adjuntarse: un fichero del disco (soltado,
/// o copiado en el gestor de ficheros y pegado) o una imagen pegada, que no
/// tiene fichero.
pub enum Incoming {
    Path(PathBuf),
    Picture { name: String, bytes: Vec<u8> },
}

/// El nombre y los bytes de lo que llegó, para subirlo. Un fichero del
/// disco se lee aquí; el error es `None` si no se pudo leer, o el rechazo
/// del core si no cabe.
pub fn read_incoming(file: Incoming) -> (String, Result<Vec<u8>, Option<cac_core::ApiError>>) {
    match file {
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
    }
}

type Attach = Rc<dyn Fn(&mut Window, &mut App)>;
type TakeFiles = Rc<dyn Fn(Vec<Incoming>, &mut Window, &mut App)>;

/// Los botones de la barra, en el orden de `Toolbar` en `MarkdownEditor.tsx`.
/// `true` es «va detrás de un separador».
const BUTTONS: [(&str, Lucide, &str, ComposeAction, bool); 10] = [
    ("bold", Lucide::Bold, "common:editor.bold", ComposeAction::Bold, false),
    ("italic", Lucide::Italic, "common:editor.italic", ComposeAction::Italic, false),
    ("code", Lucide::Code, "common:editor.code", ComposeAction::Code, false),
    ("heading", Lucide::Heading2, "common:editor.heading", ComposeAction::Heading, true),
    ("bullets", Lucide::List, "common:editor.bulletList", ComposeAction::BulletList, false),
    ("numbers", Lucide::ListOrdered, "common:editor.numberedList", ComposeAction::OrderedList, false),
    ("checklist", Lucide::ListChecks, "common:editor.checklist", ComposeAction::TaskList, false),
    ("quote", Lucide::Quote, "common:editor.quote", ComposeAction::Quote, false),
    ("table", Lucide::Table, "common:editor.table", ComposeAction::Table, false),
    ("link", Lucide::Link2, "common:editor.link", ComposeAction::Link, false),
];

/// Lo que hace cada atajo de teclado, que son los de tiptap: Ctrl (Cmd en un
/// Mac) con B, I, E y K.
fn shortcut(key: &str) -> Option<ComposeAction> {
    match key {
        "b" => Some(ComposeAction::Bold),
        "i" => Some(ComposeAction::Italic),
        "e" => Some(ComposeAction::Code),
        "k" => Some(ComposeAction::Link),
        _ => None,
    }
}

/// Lo escrito y lo seleccionado, en bytes: como lo pide el core.
fn written(state: &Entity<TextareaState>, cx: &App) -> (String, usize, usize) {
    let state = state.read(cx);
    let chosen = state.selected_range();
    (state.value().to_string(), chosen.start, chosen.end)
}

/// Pone en el campo lo que el core contestó, con su selección, y le deja el
/// foco para seguir escribiendo. Con `replace_all` y no `set_value`: así
/// Ctrl+Z deshace el botón.
fn put(state: &Entity<TextareaState>, edit: ComposeEdit, window: &mut Window, cx: &mut App) {
    state.update(cx, |state, cx| {
        if edit.text != state.value().as_ref() {
            state.replace_all(edit.text, window, cx);
        }
        state.set_selected_range(edit.start as usize..edit.end as usize, cx);
        state.focus(window, cx);
        cx.notify();
    });
}

/// Lo que hace un botón de la barra (o su atajo) con lo escrito.
pub fn press(state: &Entity<TextareaState>, action: ComposeAction, window: &mut Window, cx: &mut App) {
    let (text, start, end) = written(state, cx);
    put(state, cac_core::compose(&text, start, end, action), window, cx);
}

/// Enter dentro de una lista: sigue con otro elemento, o sale de ella. Dice
/// si lo cogió; si no, Enter es del campo.
fn carry_list_on(state: &Entity<TextareaState>, window: &mut Window, cx: &mut App) -> bool {
    let (text, start, end) = written(state, cx);
    let Some(edit) = cac_core::continue_list(&text, start, end) else { return false };
    put(state, edit, window, cx);
    true
}

/// Lo que hay que adjuntar de lo que se pegó: los ficheros copiados y las
/// imágenes. Vacío si era texto, que entonces se pega como texto.
pub(crate) fn pasted(item: &ClipboardItem) -> Vec<Incoming> {
    let mut out = Vec::new();
    for entry in item.entries() {
        match entry {
            ClipboardEntry::ExternalPaths(paths) => out.extend(paths.paths().iter().cloned().map(Incoming::Path)),
            ClipboardEntry::Image(image) => {
                // `pasted.png`, como la app de siempre llama a lo que se
                // pega sin nombre.
                let kind = image.format.mime_type().rsplit('/').next().unwrap_or("png");
                let kind = kind.split('+').next().unwrap_or(kind);
                out.push(Incoming::Picture { name: format!("pasted.{kind}"), bytes: image.bytes.clone() });
            }
            _ => {}
        }
    }
    out
}

/// La caja de escribir markdown, con su barra.
#[derive(IntoElement)]
pub struct MarkdownEditor {
    prefix: &'static str,
    state: Entity<TextareaState>,
    attach: Option<Attach>,
    take_files: Option<TakeFiles>,
    attaching: bool,
    placeholder: Option<SharedString>,
    fill: bool,
}

impl MarkdownEditor {
    /// `prefix` nombra sus piezas: el campo es `<prefix>-input` y cada botón
    /// `<prefix>-bold`, `<prefix>-attach`…
    pub fn new(prefix: &'static str, state: &Entity<TextareaState>) -> Self {
        Self { prefix, state: state.clone(), attach: None, take_files: None, attaching: false, placeholder: None, fill: false }
    }

    /// Con esto la barra lleva el clip: lo que hace al pulsarlo, y qué se
    /// hace con lo que se suelte o se pegue en la caja.
    pub fn attach(
        mut self,
        choose: impl Fn(&mut Window, &mut App) + 'static,
        take: impl Fn(Vec<Incoming>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.attach = Some(Rc::new(choose));
        self.take_files = Some(Rc::new(take));
        self
    }

    /// Lo que la caja dice mientras está en blanco. Sin esto dice lo de
    /// siempre («Escribe en markdown…»), como `MarkdownEditor.tsx`.
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// La caja ocupa todo el alto que le dé quien la pone, y lo escrito se
    /// desliza dentro: para escribir un documento, no un comentario. El
    /// campo tiene que ser de filas fijas (sin `auto_grow`).
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }

    /// Hay ficheros subiendo para esta caja: el clip se apaga mientras.
    pub fn attaching(mut self, attaching: bool) -> Self {
        self.attaching = attaching;
        self
    }
}

impl RenderOnce for MarkdownEditor {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, foreground, accent, primary, background) =
            (theme.border, theme.muted_foreground, theme.foreground, theme.accent, theme.primary, theme.background);
        let prefix = self.prefix;
        // El botón de `Toolbar` (`rounded p-1 text-muted-foreground
        // hover:bg-accent hover:text-foreground`, el icono `size-3.5`).
        let button = |name: &str, icon: Lucide, label: String| {
            div()
                .id(SharedString::from(format!("{prefix}-{name}")))
                .flex_none()
                .p_1()
                .rounded(px(4.))
                .text_color(muted)
                .cursor_pointer()
                .hover(move |style| style.bg(accent).text_color(foreground))
                .child(Icon::new(icon).size_3p5())
                .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        };
        let separator = || div().flex_none().mx_1().h_4().w(px(1.)).bg(border);

        let mut bar = div().flex().flex_wrap().items_center().gap_0p5().px_2().py_1().border_b_1().border_color(border);
        for (name, icon, label, action, apart) in BUTTONS {
            if apart {
                bar = bar.child(separator());
            }
            let state = self.state.clone();
            bar = bar.child(button(name, icon, tr(cx, label)).on_click(move |_, window, cx| press(&state, action, window, cx)).test_support());
        }
        if let Some(choose) = self.attach.clone() {
            let attaching = self.attaching;
            bar = bar.child(separator()).child(
                button("attach", Lucide::Paperclip, tr(cx, "common:editor.attachFile"))
                    .when(attaching, |this| this.opacity(0.5))
                    .on_click(move |_, window, cx| {
                        if !attaching {
                            choose(window, cx);
                        }
                    })
                    .test_support(),
            );
        }
        // A la derecha, lo que la caja entiende. Sólo promete pegar o soltar
        // ficheros donde se pueden adjuntar.
        let hint = tr(cx, if self.attach.is_some() { "common:editor.hintFiles" } else { "common:editor.hint" });
        bar = bar.child(div().ml_auto().pl_2().leading_xs().text_color(muted).child(hint));

        let fill = self.fill;
        let field = Textarea::new(&self.state).appearance(false).bordered(false).when(fill, |this| this.h(relative(1.))).when_some(self.take_files.clone(), |this, take| {
            this.on_paste(move |item, window, cx| {
                let files = pasted(item);
                if files.is_empty() {
                    return false;
                }
                take(files, window, cx);
                true
            })
        });

        // El texto de ejemplo no es el del campo del kit, que lo pone arriba,
        // donde va el cursor. En `MarkdownEditor.tsx` es un párrafo aparte
        // debajo del editor (`-mt-[1.6rem] text-sm text-muted-foreground/60`)
        // que cae en la última línea de la caja, a 8 px del borde de abajo, y
        // con él la caja en blanco mide 5,6 px menos que con algo escrito.
        // Así se ve allí, y así se pinta aquí.
        let blank = self.state.read(cx).value().is_empty();
        let example = blank.then(|| self.placeholder.clone().unwrap_or_else(|| tr(cx, "common:editor.writeInMarkdown").into()));

        let (for_keys, for_enter) = (self.state.clone(), self.state.clone());
        // La caja de `MarkdownEditor.tsx`: `rounded-md border bg-background`,
        // la barra con su raya debajo y el texto en `px-3 py-2`. El campo
        // del kit ya trae su propio margen por dentro; aquí sólo se le pone
        // lo que le falta para llegar a esos 12 px de lado.
        div()
            .id(SharedString::from(format!("{prefix}-editor")))
            .flex()
            .flex_col()
            .when(fill, |this| this.flex_1().min_h_0())
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .bg(background)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let held = event.keystroke.modifiers;
                if !held.secondary(cx) || held.shift || held.alt {
                    return;
                }
                if let Some(action) = shortcut(&event.keystroke.key) {
                    press(&for_keys, action, window, cx);
                    cx.stop_propagation();
                }
            })
            // Antes de que llegue al campo: si el core dice que ese Enter
            // sigue una lista, el campo no pone su salto de línea. Con
            // Mayúsculas es un salto sin más.
            .capture_action(move |enter: &Enter, window, cx| {
                if !enter.shift && !enter.secondary && carry_list_on(&for_enter, window, cx) {
                    cx.stop_propagation();
                }
            })
            .when_some(self.take_files, |this, take| {
                // Con un fichero encima, el aro de `ring-2 ring-primary`.
                this.drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(primary)).on_drop(move |paths: &ExternalPaths, window, cx| {
                    take(paths.paths().iter().cloned().map(Incoming::Path).collect(), window, cx);
                })
            })
            .child(bar)
            .child(
                div()
                    .id(SharedString::from(format!("{prefix}-input")))
                    .relative()
                    .px(px(2.))
                    .when(fill, |this| this.flex_1().min_h_0())
                    .when(blank && !fill, |this| this.mb(px(-6.)))
                    .child(field)
                    .children(example.map(|example| {
                        div()
                            .absolute()
                            .left_3()
                            .right_3()
                            // En la caja que crece, el ejemplo cae en su
                            // única línea; en la que llena el alto, arriba,
                            // donde está el cursor.
                            .map(|this| if fill { this.top(px(8.)) } else { this.bottom(px(14.)) })
                            .truncate()
                            .leading_sm()
                            .line_height(px(20.))
                            .text_color(muted.opacity(0.6))
                            .child(example)
                    }))
                    .test_support(),
            )
    }
}
