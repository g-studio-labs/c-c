//! El editor a la vista: un solo `Entity` para todo el documento.
//!
//! Por qué uno y no un campo de GPUI Kit por bloque: con uno por bloque,
//! seleccionar a través de bloques no se puede hacer nunca (cada campo es
//! dueño de su selección), y tres mil campos son tres mil entidades. Aquí
//! hay un cursor (bloque, byte), un solo receptor de teclado e IME, y cada
//! bloque es texto pintado con su `TextLayout` guardado para saber dónde cae
//! cada byte. Es la misma forma que el ejemplo `input.rs` de GPUI, a lo
//! largo de un documento.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::*;
use unicode_segmentation::UnicodeSegmentation;

use crate::doc::{Doc, Kind};
use crate::inline::{self, Look, Shown};

gpui_kit::actions!(
    block_editor,
    [Enter, HardBreak, NewBelow, Backspace, Delete, Left, Right, Up, Down, Home, End, SelectLeft, SelectRight, SelectAll, Copy, Cut, Paste, Save]
);

const CONTEXT: &str = "BlockEditor";

/// El tema oscuro de GPUI Kit con las letras de `desktop/` (Inter e Iosevka,
/// leídas de su carpeta de recursos: sin una negrita cargada, la negrita no
/// se ve), y las teclas del editor. Una vez, al arrancar.
pub fn install(cx: &mut App) {
    let faces: [&'static [u8]; 7] = [
        include_bytes!("../../../desktop/assets/fonts/Inter-Regular.ttf"),
        include_bytes!("../../../desktop/assets/fonts/Inter-Medium.ttf"),
        include_bytes!("../../../desktop/assets/fonts/Inter-SemiBold.ttf"),
        include_bytes!("../../../desktop/assets/fonts/Inter-Bold.ttf"),
        include_bytes!("../../../desktop/assets/fonts/Inter-Italic.ttf"),
        include_bytes!("../../../desktop/assets/fonts/IosevkaTermSlab-Regular.ttf"),
        include_bytes!("../../../desktop/assets/fonts/IosevkaTermSlab-Bold.ttf"),
    ];
    let _ = cx.text_system().add_fonts(faces.into_iter().map(std::borrow::Cow::Borrowed).collect());
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = "Inter".into();
        theme.mono_font_family = "Iosevka Term Slab".into();
    });
    bind_keys(cx);
}

fn bind_keys(cx: &mut App) {
    let context = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("enter", Enter, context),
        KeyBinding::new("shift-enter", HardBreak, context),
        KeyBinding::new("ctrl-enter", NewBelow, context),
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("ctrl-a", SelectAll, context),
        KeyBinding::new("ctrl-c", Copy, context),
        KeyBinding::new("ctrl-x", Cut, context),
        KeyBinding::new("ctrl-v", Paste, context),
        KeyBinding::new("ctrl-s", Save, context),
    ]);
}

/// Lo que se pintó de un bloque en el último fotograma.
struct Painted {
    layout: TextLayout,
    shown: Rc<Shown>,
}

/// El tamaño de letra y el alto de línea de cada tipo de bloque.
fn metrics(kind: Kind) -> (f32, f32) {
    match kind {
        Kind::Heading(1) => (30., 38.),
        Kind::Heading(2) => (24., 32.),
        Kind::Heading(3) => (20., 28.),
        Kind::Heading(_) => (16., 24.),
        Kind::Code => (13., 20.),
        Kind::Opaque => (12.5, 19.),
        _ => (15., 24.),
    }
}

pub struct BlockEditor {
    pub doc: Doc,
    focus: FocusHandle,
    list: ListState,
    /// Con `false` se pintan todos los bloques, sin lista virtual: sólo para
    /// medir cuánto cuesta no tenerla.
    pub virtualized: bool,
    block: usize,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    /// La columna (en píxeles) que se intenta mantener al subir y bajar.
    goal_x: Option<Pixels>,
    painted: HashMap<u64, Painted>,
    frame_started: Option<Instant>,
    /// Cuánto tardó cada fotograma, de `render` al final del pintado.
    pub frames: Vec<Duration>,
    /// El fichero que se abrió: Ctrl+S lo guarda ahí.
    pub path: Option<std::path::PathBuf>,
}

impl BlockEditor {
    pub fn new(source: &str, cx: &mut Context<Self>) -> Self {
        let mut doc = Doc::parse(source);
        doc.ensure_not_empty();
        let list = ListState::new(doc.blocks.len(), ListAlignment::Top, px(600.));
        Self {
            doc,
            focus: cx.focus_handle(),
            list,
            virtualized: true,
            block: 0,
            selected: 0..0,
            reversed: false,
            marked: None,
            goal_x: None,
            painted: HashMap::new(),
            frame_started: None,
            frames: Vec::new(),
            path: None,
        }
    }

    pub fn to_markdown(&self) -> String {
        self.doc.to_markdown()
    }

    /// Dónde está el cursor: bloque y byte dentro de su texto.
    pub fn cursor(&self) -> (usize, usize) {
        (self.block, self.cursor_offset())
    }

    pub fn selection(&self) -> Range<usize> {
        self.selected.clone()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    /// Lo que se pintó de ese bloque en el último fotograma, si se pintó.
    pub fn painted_text(&self, block: usize) -> Option<String> {
        self.painted.get(&self.doc.blocks.get(block)?.id).map(|painted| painted.shown.text.clone())
    }

    /// Cuántos bloques se pintaron en el último fotograma.
    pub fn painted_count(&self) -> usize {
        self.painted.len()
    }

    fn cursor_offset(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    fn text(&self) -> &str {
        &self.doc.blocks[self.block].text
    }

    /// Pone el cursor. Lo usan el ratón, las pruebas y el guion de muestra.
    pub fn place(&mut self, block: usize, offset: usize, cx: &mut Context<Self>) {
        self.block = block.min(self.doc.blocks.len() - 1);
        let offset = floor_boundary(self.text(), offset);
        self.selected = offset..offset;
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        self.reveal();
        cx.notify();
    }

    /// Trae a la vista el bloque del cursor, moviendo lo justo.
    ///
    /// No con `scroll_to_reveal_item`: hace sus cuentas con las alturas de
    /// los bloques de antes, y la lista virtual no ha medido los que nunca
    /// pintó — con tres mil bloques, ir al último no movía nada.
    fn reveal(&mut self) {
        let view = self.list.viewport_bounds();
        // Un bloque recién hecho todavía no tiene sitio: va debajo del anterior.
        let bounds = self.list.bounds_for_item(self.block).or_else(|| {
            let above = self.list.bounds_for_item(self.block.checked_sub(1)?)?;
            Some(Bounds::new(point(above.left(), above.bottom()), gpui_kit::size(above.size.width, px(40.))))
        });
        match bounds {
            Some(bounds) if bounds.top() < view.top() => self.list.scroll_by(bounds.top() - view.top()),
            Some(bounds) if bounds.bottom() > view.bottom() => self.list.scroll_by((bounds.bottom() - view.bottom()).min(bounds.top() - view.top())),
            Some(_) => {}
            None => self.list.scroll_to(ListOffset { item_ix: self.block, offset_in_item: px(0.) }),
        }
    }

    /// Tras una operación que cambió cuántos bloques hay o de qué tipo son.
    fn restructured(&mut self, place: (usize, usize), cx: &mut Context<Self>) {
        // La lista virtual lleva la cuenta de las alturas por índice: si
        // cambió la cuenta, se le dice desde dónde.
        let count = self.doc.blocks.len();
        if count != self.list.item_count() {
            let from = place.0.min(self.block).saturating_sub(1).min(self.list.item_count());
            self.list.splice(from..self.list.item_count(), count - from.min(count));
        }
        self.place(place.0, place.1, cx);
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_selection();
        let place = self.doc.split(self.block, self.cursor_offset());
        self.restructured(place, cx);
    }

    /// Mayúsculas+Enter: un salto de línea que se ve, dentro del bloque. En
    /// markdown eso es un salto duro (barra invertida y salto).
    fn hard_break(&mut self, _: &HardBreak, _: &mut Window, cx: &mut Context<Self>) {
        let text = if self.doc.blocks[self.block].kind.is_raw() { "\n" } else { "\\\n" };
        self.insert(text, cx);
    }

    /// Ctrl+Enter: un párrafo nuevo debajo, esté donde esté el cursor. Es la
    /// salida de un bloque de código.
    fn new_below(&mut self, _: &NewBelow, _: &mut Window, cx: &mut Context<Self>) {
        self.doc.insert(self.block + 1, Kind::Paragraph, String::new());
        self.restructured((self.block + 1, 0), cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            self.delete_selection();
            return cx.notify();
        }
        let at = self.cursor_offset();
        if at == 0 {
            if let Some(place) = self.doc.join_backwards(self.block) {
                self.restructured(place, cx);
            }
            return;
        }
        let from = previous_boundary(self.text(), at);
        self.doc.blocks[self.block].text.replace_range(from..at, "");
        self.place(self.block, from, cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            self.delete_selection();
            return cx.notify();
        }
        let at = self.cursor_offset();
        if at == self.text().len() {
            // Suprimir al final es el retroceso al principio del siguiente,
            // si el siguiente es un párrafo que se pueda subir.
            let next = self.block + 1;
            if self.doc.blocks.get(next).is_some_and(|block| block.kind == Kind::Paragraph) {
                if let Some(place) = self.doc.join_backwards(next) {
                    self.restructured(place, cx);
                }
            }
            return;
        }
        let to = next_boundary(self.text(), at);
        self.doc.blocks[self.block].text.replace_range(at..to, "");
        self.place(self.block, at, cx);
    }

    fn delete_selection(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        let range = self.selected.clone();
        self.doc.blocks[self.block].text.replace_range(range.clone(), "");
        self.selected = range.start..range.start;
        self.reversed = false;
        self.marked = None;
    }

    /// Mete texto donde está el cursor (o sobre lo seleccionado), y si con
    /// eso el párrafo empieza por una marca (`# `, `- `, `1. `, la valla),
    /// lo convierte.
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self.marked.clone().unwrap_or(self.selected.clone());
        self.replace(range, text, cx);
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.doc.blocks[self.block].text.replace_range(range.clone(), text);
        let cursor = range.start + text.len();
        self.selected = cursor..cursor;
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        match self.doc.apply_shortcut(self.block, cursor) {
            Some(place) => self.restructured(place, cx),
            None => {
                self.list.remeasure_items(self.block..self.block + 1);
                cx.notify();
            }
        }
    }

    /// Una letra por el camino del teclado, sin ventana delante: lo usan el
    /// guion de muestra y la medida. Un salto de línea es la tecla Enter.
    pub fn type_char(&mut self, ch: char, cx: &mut Context<Self>) {
        if ch == '\n' {
            self.delete_selection();
            let place = self.doc.split(self.block, self.cursor_offset());
            self.restructured(place, cx);
        } else {
            self.insert(ch.encode_utf8(&mut [0; 4]), cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            return self.place(self.block, self.selected.start, cx);
        }
        let at = self.cursor_offset();
        if at > 0 {
            self.place(self.block, previous_boundary(self.text(), at), cx);
        } else if self.block > 0 {
            let above = self.block - 1;
            self.place(above, self.doc.blocks[above].text.len(), cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            return self.place(self.block, self.selected.end, cx);
        }
        let at = self.cursor_offset();
        if at < self.text().len() {
            self.place(self.block, next_boundary(self.text(), at), cx);
        } else if self.block + 1 < self.doc.blocks.len() {
            self.place(self.block + 1, 0, cx);
        }
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.cursor_offset();
        let start = self.text()[..at].rfind('\n').map_or(0, |ix| ix + 1);
        self.place(self.block, start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.cursor_offset();
        let end = self.text()[at..].find('\n').map_or(self.text().len(), |ix| at + ix);
        self.place(self.block, end, cx);
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify();
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(previous_boundary(self.text(), self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_boundary(self.text(), self.cursor_offset()), cx);
    }

    /// Selecciona el texto del bloque. Seleccionar a través de bloques no
    /// está hecho (ver el README).
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = 0..self.text().len();
        self.reversed = false;
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.text()[self.selected.clone()].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&Copy, window, cx);
        self.delete_selection();
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(&text, cx);
        }
    }

    fn save(&mut self, _: &Save, _: &mut Window, _: &mut Context<Self>) {
        if let Some(path) = &self.path {
            match std::fs::write(path, self.to_markdown()) {
                Ok(()) => eprintln!("guardado: {}", path.display()),
                Err(error) => eprintln!("no se pudo guardar {}: {error}", path.display()),
            }
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, cx);
    }

    /// Sube o baja una línea **de las pintadas** (un párrafo largo son
    /// varias), y al salir del bloque entra en el vecino por la misma
    /// columna de la pantalla.
    fn vertical(&mut self, down: bool, cx: &mut Context<Self>) {
        let at = self.cursor_offset();
        let id = self.doc.blocks[self.block].id;
        let mut goal = self.goal_x;
        if let Some(painted) = self.painted.get(&id) {
            let line = painted.layout.line_height();
            let bounds = painted.layout.bounds();
            let here = painted.layout.position_for_index(painted.shown.to_shown(at)).unwrap_or(bounds.origin);
            let x = *goal.get_or_insert(here.x);
            let y = if down { here.y + line * 1.5 } else { here.y - line * 0.5 };
            if y > bounds.top() && y < bounds.bottom() {
                let offset = painted.shown.to_source(index_in_row(painted, point(x, y)));
                self.place(self.block, offset, cx);
                self.goal_x = goal;
                return;
            }
        }
        let neighbour = if down { Some(self.block + 1).filter(|ix| *ix < self.doc.blocks.len()) } else { self.block.checked_sub(1) };
        let Some(neighbour) = neighbour else {
            // En el primero o el último: al principio o al final del texto.
            let edge = if down { self.text().len() } else { 0 };
            return self.place(self.block, edge, cx);
        };
        let target = &self.doc.blocks[neighbour];
        let offset = match (self.painted.get(&target.id), goal) {
            (Some(painted), Some(x)) => {
                let (line, bounds) = (painted.layout.line_height(), painted.layout.bounds());
                let y = if down { bounds.top() + line * 0.5 } else { bounds.bottom() - line * 0.5 };
                painted.shown.to_source(index_in_row(painted, point(x, y)))
            }
            // El vecino no está pintado (queda fuera de la vista): la misma
            // columna contada en letras.
            _ => {
                let column = self.text()[..at].rsplit('\n').next().unwrap_or("").chars().count();
                let line_start = if down { 0 } else { target.text.rfind('\n').map_or(0, |ix| ix + 1) };
                let line = target.text[line_start..].split('\n').next().unwrap_or("");
                line_start + line.char_indices().nth(column).map_or(line.len(), |(ix, _)| ix)
            }
        };
        self.place(neighbour, offset, cx);
        self.goal_x = goal;
    }

    fn click(&mut self, ix: usize, id: u64, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        if self.doc.blocks.get(ix).map(|block| block.id) != Some(id) {
            return;
        }
        let offset = self.painted.get(&id).map_or(0, |painted| painted.shown.to_source(either(painted.layout.index_for_position(position))));
        self.place(ix, offset, cx);
    }

    fn look(look: &Look, cx: &App) -> HighlightStyle {
        let theme = cx.theme();
        let mut style = HighlightStyle::default();
        if look.bold {
            style.font_weight = Some(FontWeight::BOLD);
        }
        if look.italic {
            style.font_style = Some(FontStyle::Italic);
        }
        if look.strike {
            style.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: None });
        }
        if look.code {
            style.background_color = Some(theme.muted);
        }
        if look.link {
            style.color = Some(theme.link);
            style.underline = Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false });
        }
        if look.marker {
            style.color = Some(theme.muted_foreground.opacity(0.6));
            style.font_weight = Some(FontWeight::NORMAL);
            style.underline = None;
        }
        style
    }

    fn render_block(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(block) = self.doc.blocks.get(ix) else { return div().into_any_element() };
        let (id, kind) = (block.id, block.kind);
        let here = ix == self.block;
        // Con el cursor dentro se enseñan las marcas y lo pintado es el
        // texto byte a byte; sin él, se esconden.
        let shown = Rc::new(if kind.has_inline() { inline::shown(&block.text, here) } else { Shown::plain(&block.text) });
        let highlights: Vec<_> = shown.runs.iter().map(|(range, look)| (range.clone(), Self::look(look, cx))).collect();
        let text = StyledText::new(SharedString::from(shown.text.clone())).with_highlights(highlights);
        let layout = text.layout().clone();

        let theme = cx.theme();
        let (muted, muted_foreground, border, caret, selection_colour) = (theme.muted, theme.muted_foreground, theme.border, theme.caret, theme.selection);
        let mono = theme.mono_font_family.clone();
        let (size, line) = metrics(kind);
        let selection = here.then(|| self.selected.clone());
        let (entity, focus) = (cx.entity(), self.focus.clone());
        let overlay = canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                if let Some(range) = selection {
                    let line = layout.line_height();
                    let at = |ix: usize| layout.position_for_index(ix).unwrap_or(bounds.origin);
                    if range.is_empty() {
                        if focus.is_focused(window) {
                            window.paint_quad(fill(Bounds::new(at(range.start), gpui_kit::size(px(1.5), line)), caret));
                        }
                    } else {
                        // Lo seleccionado, línea a línea de las pintadas.
                        let (from, to) = (at(range.start), at(range.end));
                        if from.y == to.y {
                            window.paint_quad(fill(Bounds::from_corners(from, point(to.x, to.y + line)), selection_colour));
                        } else {
                            window.paint_quad(fill(Bounds::from_corners(from, point(bounds.right(), from.y + line)), selection_colour));
                            window.paint_quad(fill(Bounds::from_corners(point(bounds.left(), from.y + line), point(bounds.right(), to.y)), selection_colour));
                            window.paint_quad(fill(Bounds::from_corners(point(bounds.left(), to.y), point(to.x, to.y + line)), selection_colour));
                        }
                    }
                    // El teclado y el IME, al bloque que tiene el cursor.
                    window.handle_input(&focus, ElementInputHandler::new(bounds, entity.clone()), cx);
                }
                entity.update(cx, |this, _| {
                    this.painted.insert(id, Painted { layout, shown });
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let body = div().relative().flex_1().min_w_0().min_h(px(line)).child(text).child(overlay);
        let marker = |label: String| div().flex_none().w(px(26.)).text_color(muted_foreground).child(label);
        let content = match kind {
            Kind::Paragraph => div().py(px(3.)).child(body),
            Kind::Heading(level) => div()
                .pt(px(if level == 1 { 20. } else { 14. }))
                .pb(px(4.))
                .font_weight(if level == 1 { FontWeight::BOLD } else { FontWeight::SEMIBOLD })
                .child(body),
            Kind::Bullet => div().flex().py(px(1.)).pl(px(4.)).child(marker("•".to_string())).child(body),
            Kind::Numbered => div().flex().py(px(1.)).pl(px(4.)).child(marker(block.lead.trim_end().to_string())).child(body),
            Kind::Code => div().my(px(4.)).px(px(12.)).py(px(8.)).rounded(px(6.)).bg(muted).font_family(mono).child(body),
            // Lo que no se edita por bloques: se ve que es markdown tal cual.
            Kind::Opaque => div().my(px(4.)).pl(px(10.)).border_l_2().border_color(border).text_color(muted_foreground).font_family(mono).child(body),
        };
        div()
            .id(("block", id))
            .test_support()
            .w_full()
            .flex()
            .justify_center()
            .px(px(32.))
            .text_size(px(size))
            .line_height(px(line))
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, window, cx| this.click(ix, id, event.position, window, cx)))
            .child(content.w_full().max_w(px(760.)))
            .into_any_element()
    }
}

/// `index_for_position` contesta `Err` con el byte más cercano cuando el
/// punto cae fuera del texto: aquí vale igual.
fn either(found: Result<usize, usize>) -> usize {
    found.unwrap_or_else(|nearest| nearest)
}

/// El byte de lo pintado que cae en ese punto, **en esa línea**.
///
/// El byte donde un párrafo dobla de línea es a la vez el final de una y el
/// principio de la siguiente, y `TextLayout` lo pinta siempre al final de la
/// de arriba: sin esto, bajar a la segunda línea por su principio dejaba el
/// cursor en la primera y la flecha no salía nunca de ahí. Un editor de
/// verdad guarda a qué lado está el cursor; aquí se pasa una letra adelante.
fn index_in_row(painted: &Painted, position: Point<Pixels>) -> usize {
    let (layout, line) = (&painted.layout, painted.layout.line_height());
    let top = layout.bounds().top();
    let found = either(layout.index_for_position(position)).min(painted.shown.text.len());
    let row_top = top + line * ((position.y - top) / line).floor();
    match layout.position_for_index(found) {
        Some(landed) if landed.y + px(1.) < row_top => next_boundary(&painted.shown.text, found),
        _ => found,
    }
}

fn floor_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// El principio de la letra anterior: una letra con su acento suelto, o un
/// emoji compuesto, cuentan como una.
fn previous_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true).rev().find_map(|(ix, _)| (ix < offset).then_some(ix)).unwrap_or(0)
}

fn next_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true).find_map(|(ix, _)| (ix > offset).then_some(ix)).unwrap_or(text.len())
}

fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (ix, ch) in text.char_indices() {
        if units >= offset {
            return ix;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn byte_to_utf16(text: &str, offset: usize) -> usize {
    text[..floor_boundary(text, offset)].encode_utf16().count()
}

fn range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    byte_to_utf16(text, range.start)..byte_to_utf16(text, range.end)
}

fn range_from_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    utf16_to_byte(text, range.start)..utf16_to_byte(text, range.end)
}

/// Lo que el sistema le pide a un campo de texto: por aquí entra **todo** lo
/// que se escribe, también las teclas muertas (´ + a) y un IME. El «texto»
/// del que habla es el del bloque que tiene el cursor.
impl EntityInputHandler for BlockEditor {
    fn text_for_range(&mut self, range_utf16: Range<usize>, adjusted: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let range = range_from_utf16(self.text(), &range_utf16);
        adjusted.replace(range_to_utf16(self.text(), &range));
        Some(self.text()[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: range_to_utf16(self.text(), &self.selected), reversed: self.reversed })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| range_to_utf16(self.text(), range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.map(|range| range_from_utf16(self.text(), &range)).or(self.marked.clone()).unwrap_or(self.selected.clone());
        self.replace(range, text, cx);
    }

    /// Lo que está a medio componer: el acento que espera su vocal. Se pone
    /// en el texto y se recuerda dónde, para que lo siguiente lo sustituya.
    fn replace_and_mark_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, selected_utf16: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.map(|range| range_from_utf16(self.text(), &range)).or(self.marked.clone()).unwrap_or(self.selected.clone());
        self.doc.blocks[self.block].text.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        self.selected = match selected_utf16 {
            Some(inner) => {
                let inner = range_from_utf16(text, &inner);
                range.start + inner.start..range.start + inner.end
            }
            None => range.start + text.len()..range.start + text.len(),
        };
        self.reversed = false;
        self.list.remeasure_items(self.block..self.block + 1);
        cx.notify();
    }

    fn bounds_for_range(&mut self, range_utf16: Range<usize>, element: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let range = range_from_utf16(self.text(), &range_utf16);
        let painted = self.painted.get(&self.doc.blocks[self.block].id)?;
        let line = painted.layout.line_height();
        let from = painted.layout.position_for_index(range.start).unwrap_or(element.origin);
        let to = painted.layout.position_for_index(range.end).unwrap_or(from);
        Some(Bounds::from_corners(from, point(to.x.max(from.x), from.y + line)))
    }

    fn character_index_for_point(&mut self, position: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let painted = self.painted.get(&self.doc.blocks[self.block].id)?;
        Some(byte_to_utf16(self.text(), either(painted.layout.index_for_position(position))))
    }
}

impl Focusable for BlockEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for BlockEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame_started = Some(Instant::now());
        // Lo pintado es lo de este fotograma: lo que ya no se ve, no cuenta.
        self.painted.clear();
        let entity = cx.entity();
        let body = if self.virtualized {
            let entity = entity.clone();
            list(self.list.clone(), move |ix, window, cx| entity.update(cx, |this, cx| this.render_block(ix, window, cx))).size_full().into_any_element()
        } else {
            let blocks: Vec<_> = (0..self.doc.blocks.len()).map(|ix| self.render_block(ix, window, cx)).collect();
            div().id("every-block").size_full().overflow_y_scroll().children(blocks).into_any_element()
        };
        // Lo último que se pinta: cuánto tardó el fotograma entero.
        let stopwatch = canvas(
            |_, _, _| (),
            move |_, _, _, cx| {
                entity.update(cx, |this, _| {
                    if let Some(started) = this.frame_started.take() {
                        this.frames.push(started.elapsed());
                    }
                })
            },
        )
        .absolute()
        .size_0();
        let theme = cx.theme();
        div()
            .id("block-editor")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .py(px(16.))
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::hard_break))
            .on_action(cx.listener(Self::new_below))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::save))
            .child(body)
            .child(stopwatch)
    }
}
