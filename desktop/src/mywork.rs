//! «Mi trabajo» en lista: la cabecera (ámbito, lentes, sólo lo abierto,
//! buscador) y las tareas agrupadas por espacio.
//!
//! Qué tareas hay, cómo se agrupan, cómo se dice un vencimiento y qué pide
//! cada lente lo decide el core. Aquí está el buscador, que es lo único que
//! esta pantalla tiene propio, y la pintura.

use cac_core::{DueState, TaskFilter, WorkGroup, WorkLayout, WorkLens, WorkRow, NOBODY};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::newtask::{NewTaskClosed, NewTaskRow};

/// Las lentes, en el orden de la app de siempre, con su nombre en el catálogo.
const LENSES: [(WorkLens, &str); 5] = [
    (WorkLens::Assigned, "assigned"),
    (WorkLens::Created, "created"),
    (WorkLens::Watching, "watching"),
    (WorkLens::Clients, "clients"),
    (WorkLens::All, "all"),
];

// Los tamaños de letra con su interlineado de Tailwind viven en el tema,
// para todas las pantallas.
pub(crate) use crate::theme::Leading;

/// El botón de un desplegable, con la forma del `Picker` de la app de
/// siempre en su tamaño `sm` (`SelectTrigger` en `ui/select.tsx`: `h-7
/// rounded-md border border-input py-2 pr-2 pl-2.5 gap-1.5`, la letra
/// `text-xs` que le pone quien lo usa y su chevrón `size-4` apagado). El
/// botón pequeño del kit mide 24 y escribe a 14.
pub(crate) fn picker(
    id: &'static str,
    label: String,
    max_width: Option<Pixels>,
    cx: &App,
    menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    Button::new(id)
        .outline()
        .small()
        .h(px(28.))
        .pl(px(10.))
        .pr_2()
        .when_some(max_width, |this, width| this.max_w(width))
        .child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap_1p5()
                .leading_xs()
                .font_weight(FontWeight::NORMAL)
                .child(div().min_w_0().truncate().child(label))
                .child(Icon::new(gpui_kit::component::IconName::ChevronDown).size_4().flex_none().text_color(muted)),
        )
        .dropdown_menu(menu)
}

/// La prioridad de una tarea como chip: punto y nombre sobre su color al
/// 15%; sin prioridad no se pinta nada. Las medidas y los tonos son los de
/// `app/src/components/tasks/PriorityChip.tsx` (`rounded-full px-1.5 py-px
/// text-xs font-medium`, punto `size-1.5`): sólo texto de color, en una
/// columna llena no se distinguía de los demás metadatos.
pub(crate) fn priority_chip(priority: &str, cx: &App) -> Option<Div> {
    (!matches!(priority, "none" | "")).then(|| priority_chip_shown(priority, cx))
}

/// El mismo chip, también para «ninguna» (`showNone`): en el detalle de una
/// tarea el botón tiene que verse para poder ponerle una. Sin prioridad va
/// sin fondo y con el punto hueco.
pub(crate) fn priority_chip_shown(priority: &str, cx: &App) -> Div {
    let theme = cx.theme();
    let none = matches!(priority, "none" | "");
    let tone = match priority {
        "urgent" => theme.danger,
        "high" => theme.warning,
        // El `--info` del CSS, que el tema del kit no trae con ese valor.
        "normal" => crate::theme::css_color("info", cx),
        _ => theme.muted_foreground,
    };
    let name = if none { "none" } else { priority };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_1p5()
        .py(px(1.))
        .rounded_full()
        .when(!none, |this| this.bg(tone.opacity(0.15)))
        .leading_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(tone)
        .child(div().size_1p5().flex_none().rounded_full().map(|dot| if none { dot.border_1().border_color(tone) } else { dot.bg(tone) }))
        .child(tr(cx, &format!("work:taskPriority.{name}")))
}

/// De qué color va un vencimiento y si pesa: rojo si ya pasó, ámbar si es
/// hoy o mañana; hoy, además, con peso.
pub(crate) fn due_tone(state: DueState, cx: &App) -> (Hsla, bool) {
    let theme = cx.theme();
    match state {
        DueState::Overdue | DueState::Yesterday => (theme.danger, true),
        DueState::Today => (theme.warning, true),
        DueState::Tomorrow => (theme.warning, false),
        DueState::Later => (theme.muted_foreground, false),
    }
}

/// Las vistas, con el nombre de su texto y su icono.
const LAYOUTS: [(WorkLayout, &str, Lucide); 3] = [
    (WorkLayout::List, "list", Lucide::List),
    (WorkLayout::Board, "board", Lucide::SquareKanban),
    (WorkLayout::Calendar, "calendar", Lucide::CalendarDays),
];

pub struct MyWorkView {
    search: Entity<InputState>,
    /// Lo elegido en los desplegables; vacío es «cualquiera».
    assignee: String,
    priority: String,
    /// La fila de «tarea nueva», mientras está abierta.
    pub(crate) new_task: Option<Entity<NewTaskRow>>,
    _new_task_closed: Option<Subscription>,
    /// La tarjeta del tablero que va en el aire, por su id.
    pub(crate) lifted: Option<String>,
    /// Lo que el servidor contestó al rechazar un movimiento del tablero.
    pub(crate) board_error: Option<String>,
    /// Lo que el calendario recuerda mientras está abierto (`calendar.rs`).
    pub(crate) calendar: crate::calendar::CalendarState,
}

impl MyWorkView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = tr(cx, "work:board.filter.search");
        Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)),
            assignee: String::new(),
            priority: String::new(),
            new_task: None,
            _new_task_closed: None,
            lifted: None,
            board_error: None,
            calendar: crate::calendar::CalendarState::new(cx),
        }
    }

    /// Abre la fila de «tarea nueva», o la cierra si ya estaba.
    pub(crate) fn toggle_new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_task.take().is_some() {
            self._new_task_closed = None;
        } else {
            let row = cx.new(|cx| NewTaskRow::new(window, cx));
            self._new_task_closed = Some(cx.subscribe(&row, |this, _, _: &NewTaskClosed, cx| {
                this.new_task = None;
                this._new_task_closed = None;
                cx.notify();
            }));
            self.new_task = Some(row);
        }
        cx.notify();
    }

    fn filter(&self, cx: &App) -> TaskFilter {
        TaskFilter {
            query: self.search.read(cx).value().to_string(),
            assignee: self.assignee.clone(),
            priority: self.priority.clone(),
        }
    }

    pub(crate) fn choose_assignee(&mut self, assignee: String, cx: &mut Context<Self>) {
        self.assignee = assignee;
        cx.notify();
    }

    pub(crate) fn choose_priority(&mut self, priority: String, cx: &mut Context<Self>) {
        self.priority = priority;
        cx.notify();
    }

    /// Quita todo lo que filtra, también lo escrito en el buscador: un
    /// «quitar filtros» que deja la lista a medias no ha quitado los filtros.
    pub(crate) fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.assignee.clear();
        self.priority.clear();
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        cx.notify();
    }

    /// Los desplegables de filtro y, si algo filtra, cómo quitarlo. Qué se
    /// ofrece en cada uno lo decide el core con las tareas que hay.
    fn render_filters(&self, filter: &TaskFilter, cx: &mut Context<Self>) -> impl IntoElement {
        let options = Backend::core(cx).my_work().filter_options(filter);
        let here = locale(cx);
        let phrase = move |key: &str| cac_core::text(here, key);
        let view = cx.entity().downgrade();

        let assignee = (!options.assignees.is_empty()).then(|| {
            let chosen = self.assignee.clone();
            let label = match chosen.as_str() {
                "" => phrase("work:board.filter.anyone"),
                NOBODY => phrase("work:board.filter.nobody"),
                name => name.to_string(),
            };
            let mut choices = vec![(String::new(), phrase("work:board.filter.anyone"))];
            if options.unassigned {
                choices.push((NOBODY.to_string(), phrase("work:board.filter.nobody")));
            }
            choices.extend(options.assignees.iter().map(|name| (name.clone(), name.clone())));
            let view = view.clone();
            picker("filter-assignee", label, None, cx, move |mut menu, _, _| {
                for (value, label) in choices.clone() {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(chosen == value).on_click(move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| this.choose_assignee(value.clone(), cx));
                    }));
                }
                menu
            })
        });

        let priority = (!options.priorities.is_empty()).then(|| {
            let chosen = self.priority.clone();
            let label = match chosen.as_str() {
                "" => phrase("work:board.filter.anyPriority"),
                name => phrase(&format!("work:taskPriority.{name}")),
            };
            let mut choices = vec![(String::new(), phrase("work:board.filter.anyPriority"))];
            choices.extend(options.priorities.iter().map(|name| (name.clone(), phrase(&format!("work:taskPriority.{name}")))));
            let view = view.clone();
            picker("filter-priority", label, None, cx, move |mut menu, _, _| {
                for (value, label) in choices.clone() {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(chosen == value).on_click(move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| this.choose_priority(value.clone(), cx));
                    }));
                }
                menu
            })
        });

        let clear = filter.is_filtering().then(|| {
            // `Button size="xs" variant="ghost"` (`ui/button.tsx`): `h-6
            // gap-1 rounded-md px-2 text-xs font-medium`, la × `size-3`, y al
            // pasar `bg-muted` (en oscuro, a la mitad). El botón más pequeño
            // del kit escribe a 12.
            let (wash, dark) = (cx.theme().muted, cx.theme().mode.is_dark());
            div()
                .id("filter-clear")
                .h_6()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .px_2()
                .rounded(px(8.))
                .leading_xs()
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .hover(move |style| style.bg(if dark { wash.opacity(0.5) } else { wash }))
                .child(Icon::new(Lucide::X).size_3())
                .child(phrase("work:board.filter.clear"))
                .on_click(cx.listener(|this, _, window, cx| this.clear_filters(window, cx)))
                .test_support()
        });

        div().flex().flex_none().items_center().gap_2().children(assignee).children(priority).children(clear)
    }

    fn show_lens(lens: WorkLens, cx: &App) {
        Backend::run(cx, move |core| async move { core.show_lens(lens).await });
    }

    fn show_closed(on: bool, cx: &App) {
        Backend::run(cx, move |core| async move { core.show_closed(on).await });
    }

    fn render_header(&self, shown: usize, filter: &TaskFilter, cx: &mut Context<Self>) -> impl IntoElement {
        let filters = self.render_filters(filter, cx);
        let theme = cx.theme();
        let (border, muted, foreground, primary, on_primary, accent) =
            (theme.border, theme.muted_foreground, theme.foreground, theme.primary, theme.primary_foreground, theme.accent);
        let my_work = Backend::snapshot(cx).read(cx).my_work.clone();
        let here = locale(cx);
        let count = if my_work.loading { "…".to_string() } else { cac_core::text_count(here, "work:myWork.visible", shown as i64) };
        // A qué está acotado: «Lista: nombre», «Carpeta: nombre» o «Espacio:
        // nombre». Cómo se llama cada clase lo dice el core.
        let scope_id = my_work.scope.as_ref().map(|scope| scope.id.clone());
        let surface = theme.muted;
        let scope = my_work.scope.as_ref().map(|scope| format!("{}: {}", tr(cx, scope.kind.label_key()), scope.name));
        let closed = my_work.include_closed;
        let docs = cac_core::docs_reachable_from_scope(my_work.scope.as_ref().map(|scope| scope.kind));
        let docs_hint = tr(cx, "work:board.view.docs");
        let closed_hint = tr(cx, if closed { "work:myWork.hideFinished" } else { "work:myWork.showFinished" });
        let show_everything = tr(cx, "work:myWork.showEverything");

        // Las medidas son las de la cabecera de `MyWork.tsx` y la barra de
        // `TaskFilters.tsx`: el título no cambia con el ámbito (va en su
        // pastilla), las vistas van al final de la fila de lentes, y buscar
        // y filtrar tienen su propia barra debajo.
        let header = div()
            .flex()
            .flex_none()
            .flex_col()
            .px_4()
            .pt_3()
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(div().id("view-title").leading_lg().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "work:myWork.title")).test_support())
                    .child(div().id("view-count").leading_xs().text_color(muted).child(count).test_support())
                    // Los dos botones, centrados en la fila (`self-center`):
                    // el título y la cuenta se alinean por su línea base.
                    .child(
                        div()
                            .id("new-task")
                            .ml_auto()
                            .self_center()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(primary)
                            .leading_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(on_primary)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_new_task(window, cx)))
                            .child(tr(cx, "work:myWork.newTask"))
                            .test_support(),
                    )
                    .child(
                        div()
                            .id("toggle-closed")
                            .self_center()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(border)
                            .leading_xs()
                            .text_color(muted)
                            .cursor_pointer()
                            .hover(move |style| style.bg(accent).text_color(foreground))
                            .on_click(move |_, _, cx| Self::show_closed(!closed, cx))
                            .child(Icon::new(if closed { Lucide::Eye } else { Lucide::EyeOff }).size_3())
                            .child(tr(cx, if closed { "work:myWork.allStates" } else { "work:myWork.openOnly" }))
                            .tooltip(move |window, cx| Tooltip::new(closed_hint.clone()).build(window, cx))
                            .test_support(),
                    ),
            )
            // A qué está acotado, y cómo dejar de estarlo.
            .children(scope.map(|scope| {
                div().mt_1().flex().items_center().gap_1p5().child(
                    div()
                        .id("scope-clear")
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .border_1()
                        .border_color(border)
                        .bg(accent.opacity(0.4))
                        .leading_xs()
                        .cursor_pointer()
                        .hover(move |style| style.bg(accent))
                        .on_click(|_, _, cx| Backend::core(cx).my_work().set_scope(None))
                        .child(div().min_w_0().truncate().child(scope))
                        .child(Icon::new(Lucide::X).size_3().flex_none())
                        .tooltip(move |window, cx| Tooltip::new(show_everything.clone()).build(window, cx))
                        .test_support(),
                )
                // El id del nodo, a un clic (`CopyId.tsx`): se llega aquí
                // pulsando una lista del árbol, y copiarlo a mano de otro
                // sitio es donde se tuerce una sesión de MCP.
                .children(scope_id.clone().map(|id| {
                    let (short, hint, copied) = (id.chars().take(8).collect::<String>(), id.clone(), id);
                    div()
                        .id("scope-copy-id")
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .px_1p5()
                        .py_0p5()
                        .rounded(px(4.))
                        .bg(surface.opacity(0.6))
                        .font_family(cx.theme().mono_font_family.clone())
                        .leading_xs()
                        .text_color(muted)
                        .cursor_pointer()
                        .hover(move |style| style.bg(surface).text_color(foreground))
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone())))
                        .child(Icon::new(Lucide::Copy).size_2p5())
                        .child(short)
                        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                        .test_support()
                }))
            }))
            // La fila de lentes (`<nav className="-mb-px flex items-center
            // gap-4 pt-2 text-sm">`): se monta un píxel sobre la raya de la
            // cabecera, y la raya de la lente puesta (`TabIndicator`:
            // `-bottom-px h-0.5 rounded-full bg-primary`) otro más abajo.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .pt_2()
                    .mb(px(-1.))
                    .child(div().min_w_0().flex().items_center().gap_4().children(LENSES.map(|(lens, name)| {
                        let active = my_work.lens == lens;
                        div()
                            .id(SharedString::from(format!("lens-{name}")))
                            .relative()
                            .flex_none()
                            // `border-b-2 pb-2`: la raya es transparente y
                            // sólo ocupa; la que se ve es el indicador.
                            .pb(px(10.))
                            .leading_sm()
                            .text_color(if active { foreground } else { muted })
                            .when(active, |this| {
                                this.font_weight(FontWeight::MEDIUM)
                                    .child(div().absolute().left_0().right_0().bottom(px(-1.)).h(px(2.)).rounded_full().bg(primary))
                            })
                            .when(!active, |this| this.hover(move |style| style.text_color(foreground)))
                            .cursor_pointer()
                            .on_click(move |_, _, cx| Self::show_lens(lens, cx))
                            .child(tr(cx, &format!("work:myWork.lens.{name}")))
                            .test_support()
                    })))
                    // Las vistas y, detrás de un divisor (`mx-0.5 h-4 w-px
                    // bg-border`), la documentación del nodo: a un clic con
                    // cualquier ámbito, sea lista, carpeta o espacio
                    // (command-and-control-372).
                    .child(
                        div().ml_auto().flex().flex_none().items_center().gap_0p5().pb_1p5().child(Self::render_layouts(cx)).when(docs, |this| {
                            this.child(div().w(px(1.)).h_4().mx_0p5().flex_none().bg(border)).child(
                                div()
                                    .id("open-docs")
                                    .px_1p5()
                                    .py_1()
                                    .rounded(px(4.))
                                    .text_color(muted)
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(accent.opacity(0.5)).text_color(foreground))
                                    .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.open_scope_docs().await }))
                                    .child(Icon::new(Lucide::FileText).size_3p5())
                                    .tooltip(move |window, cx| Tooltip::new(docs_hint.clone()).build(window, cx))
                                    .test_support(),
                            )
                        }),
                    ),
            );
        // La barra de `TaskFilters.tsx` (`h-10 gap-2 px-4`): el buscador
        // (`w-56`, `h-7 rounded-md border-input text-xs`, la lupa `size-3.5`
        // y, en oscuro, `bg-input/30`) y los desplegables.
        let input = cx.theme().input;
        let dark = cx.theme().mode.is_dark();
        let bar = div()
            .id("work-filters")
            .h(px(40.))
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_4()
            .border_b_1()
            .border_color(border)
            .overflow_x_scroll()
            .child(
                div().w(px(224.)).flex_none().child(
                    Input::new(&self.search)
                        .id("work-search")
                        .leading_xs()
                        .when(dark, |this| this.bg(input.opacity(0.3)))
                        .cleanable(true)
                        .prefix(Icon::new(Lucide::Search).size_3p5().text_color(muted)),
                ),
            )
            .child(filters);
        div().flex().flex_none().flex_col().child(header).child(bar)
    }

    /// Las tres maneras de mirar lo mismo, como iconos al final de la fila
    /// de lentes (`VIEWS` en `MyWork.tsx`: `gap-0.5`, cada uno `rounded
    /// px-1.5 py-1` con su icono `size-3.5`). Cuál está puesta lo recuerda
    /// el core.
    fn render_layouts(cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (accent, on_accent, muted, foreground) = (theme.accent, theme.accent_foreground, theme.muted_foreground, theme.foreground);
        let chosen = Backend::core(cx).work_layout();
        div().flex().flex_none().items_center().gap_0p5().children(LAYOUTS.map(|(layout, name, icon)| {
            let active = chosen == layout;
            let label = tr(cx, &format!("work:myWork.view.{name}"));
            div()
                .id(SharedString::from(format!("layout-{name}")))
                .px_1p5()
                .py_1()
                .rounded(px(4.))
                .cursor_pointer()
                // La puesta lleva la pastilla del `TabIndicator` (`rounded-md
                // bg-accent`), más redonda que el `rounded` de las demás.
                .when(active, |this| this.rounded(px(8.)).bg(accent).text_color(on_accent))
                .when(!active, |this| this.text_color(muted).hover(move |style| style.bg(accent.opacity(0.5)).text_color(foreground)))
                .on_click(move |_, _, cx| {
                    // Si no se pudo guardar, la vista no cambia: lo que se
                    // pinta es lo que el core dice que está elegido.
                    let _ = Backend::core(cx).set_work_layout(layout);
                    cx.refresh_windows();
                })
                .child(Icon::new(icon).size_3p5())
                .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                .test_support()
        }))
    }

    /// Una fila de la lista, con las medidas del `<li>` de `MyWork.tsx`:
    /// folio en mono (`w-10`), título con la miga de su madre, el chip de
    /// prioridad, el vencimiento (`w-20`) y, cerrando, su lista (`w-28`).
    /// `first` es que no lleva raya encima: las filas se separan entre sí
    /// (`divide-y`), no del borde de la tarjeta.
    fn render_row(&self, row: WorkRow, first: bool, zone_hint: Option<String>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, hover, border, mono) = (theme.muted_foreground, theme.accent, theme.border, theme.mono_font_family.clone());
        let priority = priority_chip(&row.priority, cx);
        let due = row.due.map(|due| {
            let (tone, strong) = due_tone(due.state, cx);
            div()
                .id(SharedString::from(format!("work-due-{}", row.id)))
                .w_20()
                .flex_none()
                .leading_xs()
                .text_right()
                .text_color(tone)
                .when(strong, |this| this.font_weight(FontWeight::MEDIUM))
                .child(due.text)
                // Para quien no está en la hora de la organización: en qué
                // hora se acaba ese día (`dueTitle` en `TaskCardMini.tsx`).
                .when_some(zone_hint, |this, hint| this.tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx)))
        });

        div()
            .id(SharedString::from(format!("work-row-{}", row.id)))
            .test_support()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .when(!first, |this| this.border_t_1().border_color(border))
            .hover(move |style| style.bg(hover.opacity(0.4)))
            .cursor_pointer()
            .on_click({
                let id = row.id.clone();
                move |_, _, cx| {
                    let id = id.clone();
                    Backend::run(cx, move |core| async move { core.tasks().open(&id).await });
                }
            })
            .child(div().w_10().flex_none().font_family(mono).leading_xs().text_color(muted).child(format!("#{}", row.seq)))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_baseline()
                    .gap_1()
                    .leading_sm()
                    // El título entero primero; lo que se recorta es la miga.
                    .child(div().flex_shrink_0().max_w_full().truncate().child(row.title))
                    .children(row.parent.map(|parent| div().min_w_0().truncate().leading_xs().text_color(muted).child(parent))),
            )
            .children(crate::requests::waiting_chip(format!("work-waiting-{}", row.id), &row.id, cx))
            .children(priority)
            .children(due)
            .child(div().id(SharedString::from(format!("work-list-{}", row.id))).w(rems(7.)).flex_none().truncate().leading_xs().text_right().text_color(muted).child(row.list).test_support())
    }

    /// Un espacio y sus tareas: el rótulo (`mb-1 text-xs font-medium
    /// uppercase`) y la tarjeta (`rounded-xl border bg-card`, que con el
    /// `--radius` de `index.css` son 14 px).
    fn render_group(&self, group: WorkGroup, zone_hint: &Option<String>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, card) = (theme.muted_foreground, theme.border, theme.group_box);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .mb_1()
                    .leading_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(muted)
                    .child(format!("{} · {}", group.name.to_uppercase(), group.rows.len())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(border)
                    .bg(card)
                    .overflow_hidden()
                    .children(group.rows.into_iter().enumerate().map(|(index, row)| self.render_row(row, index == 0, zone_hint.clone(), cx))),
            )
    }

    /// Mientras llega la lista, su forma: dos grupos de tres filas
    /// (`WorkSkeleton` en `MyWork.tsx`). Un «cargando…» no dice nada de lo
    /// que viene, y la pantalla da un salto cuando aparecen las filas.
    fn render_skeleton(cx: &App) -> impl IntoElement {
        let border = cx.theme().border;
        div().id("work-skeleton").flex().flex_col().gap_5().children((0..2).map(|_| {
            div().flex().flex_col().gap_2().child(Skeleton::new().h_3().w(rems(7.)).rounded_md()).children((0..3).map(|_| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(border)
                    .child(Skeleton::new().size_4().flex_none().rounded_full())
                    .child(Skeleton::new().h_3().flex_1().rounded_md())
                    .child(Skeleton::new().h_3().w_16().flex_none().rounded_md())
            }))
        }))
        .test_support()
    }
}

impl Render for MyWorkView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted) = (theme.background, theme.foreground, theme.muted_foreground);
        let filter = self.filter(cx);
        let groups = Backend::core(cx).my_work().groups(&filter, locale(cx));
        let shown: usize = groups.iter().map(|group| group.rows.len()).sum();
        let state = Backend::snapshot(cx).read(cx).my_work.clone();
        // Mientras llega y no hay nada que enseñar, la forma de lo que viene.
        let arriving = state.loading && shown == 0;
        // Por qué está vacía lo dice el core; aquí sólo se enseña.
        let empty = (shown == 0 && !arriving).then(|| tr(cx, Backend::core(cx).my_work().empty_reason()));
        let zone_hint = Backend::core(cx).my_work().zone_hint(locale(cx));
        let danger = theme.danger;

        let layout = Backend::core(cx).work_layout();
        let header = self.render_header(shown, &filter, cx);
        let body = match layout {
            WorkLayout::Board => self.render_board(&filter, cx).into_any_element(),
            WorkLayout::Calendar => self.render_calendar(&filter, cx).into_any_element(),
            // El cuerpo de la lista de `MyWork.tsx`: `p-4`, la fila de «tarea
            // nueva» dentro de lo que se desliza (`mb-3`), el fallo en su
            // línea y los grupos separados `space-y-5`.
            WorkLayout::List => div()
                .id("work-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .p_4()
                .children(self.new_task.clone().map(|row| div().mb_3().child(row)))
                .children(state.error.clone().map(|error| {
                    div()
                        .id("work-error")
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .leading_xs()
                        .text_color(danger)
                        .child(Icon::new(Lucide::CircleAlert).size_3())
                        .child(error)
                        .test_support()
                }))
                .when(arriving, |this| this.child(Self::render_skeleton(cx)))
                .children(empty.map(|phrase| div().id("work-empty").test_support().leading_sm().text_color(muted).child(phrase)))
                .child(div().flex().flex_col().gap_5().children(groups.into_iter().map(|group| self.render_group(group, &zone_hint, cx))))
                .into_any_element(),
        };
        let list = layout == WorkLayout::List;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            // El 1,5 del cuerpo de la app de siempre, para la letra de
            // tamaño suelto (`text-[11px]`); ver `Leading`.
            .line_height(relative(1.5))
            .child(header)
            // La fila de «tarea nueva» va sobre cualquiera de las vistas; en
            // la lista, dentro de lo que se desliza.
            .children(self.new_task.clone().filter(|_| !list).map(|row| div().px_4().pt_4().child(row)))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::MyWorkView;
    use crate::backend::Backend;
    use crate::testing::{contract_task, server_with_tasks, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;

    fn open(server: &FakeServer, cx: &mut TestAppContext) -> AnyWindowHandle {
        open_with_view(server, cx).0
    }

    fn open_with_view(server: &FakeServer, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<MyWorkView>) {
        server.install(cx);
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1100.), px(760.)),
                })),
                ..WindowOptions::default()
            };
            let (window, workspace) =
                gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx)))
                    .expect("open test window");
            (window, workspace.read(cx).my_work.clone())
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

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    fn tasks() -> Vec<serde_json::Value> {
        vec![
            contract_task("t1", "b", |task| task["title"] = json!("Subir el backend")),
            contract_task("t2", "a", |task| task["title"] = json!("Revisión del diseño")),
        ]
    }

    /// Las tareas que trae el servidor salen en la lista.
    ///
    /// Roturas que atrapa: una pantalla que sigue en el marcador; una lista
    /// que dice «aquí no hay nada» con tareas cargadas.
    #[gpui_kit::test]
    fn the_tasks_the_server_brings_are_listed(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);

        wait_until(cx, "las tareas llegan a la lista", |cx| on_screen(window, "work-row-t1", cx));

        assert!(on_screen(window, "work-row-t2", cx));
        assert!(!on_screen(window, "work-empty", cx), "dice que no hay nada con dos tareas en pantalla");
        assert!(on_screen(window, "work-list-t1", cx), "la fila no dice en qué lista vive");
        assert!(!on_screen(window, "work-skeleton", cx));
        stop(cx);
    }

    /// Sin tareas se dice que no hay nada, y no se queda en «cargando».
    ///
    /// Rotura que atrapa: una lista en blanco, que no distingue «no tienes
    /// nada» de «no ha llegado» ni de «se rompió».
    #[gpui_kit::test]
    fn an_empty_list_says_so(cx: &mut TestAppContext) {
        let server = server_with_tasks(vec![]);
        let window = open(&server, cx);

        wait_until(cx, "termina de cargar", |cx| {
            cx.update(|cx| {
                let my_work = &Backend::snapshot(cx).read(cx).my_work;
                !my_work.loading && my_work.loaded_key.is_some()
            })
        });

        assert!(on_screen(window, "work-empty", cx));
        assert!(!on_screen(window, "work-skeleton", cx), "sigue pintando la forma de lo que viene con la respuesta ya aquí");
        stop(cx);
    }

    /// Escribir en el buscador deja sólo lo que coincide, sin acentos de por
    /// medio.
    ///
    /// Roturas que atrapa: un buscador que no filtra; que lo escrito no
    /// llegue al filtro del core.
    #[gpui_kit::test]
    fn typing_in_the_search_narrows_the_list(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));

        cx.update_window(window, |_, window, cx| {
            window.click("work-search", cx);
            window.input("revision", cx);
        })
        .unwrap();

        assert!(on_screen(window, "work-row-t2", cx));
        assert!(!on_screen(window, "work-row-t1", cx), "el buscador no quitó lo que no coincide");
        stop(cx);
    }

    fn mixed_tasks() -> Vec<serde_json::Value> {
        vec![
            contract_task("t1", "b", |task| {
                task["priority"] = json!("urgent");
                task["assignee"] = json!("ana");
            }),
            contract_task("t2", "a", |task| {
                task["priority"] = json!("low");
                task["assignee"] = json!("luis");
            }),
        ]
    }

    /// Los desplegables salen cuando hay algo que separar, y elegir en uno
    /// deja sólo lo que coincide.
    ///
    /// Roturas que atrapa: un desplegable que elige y no filtra; que el de
    /// responsable filtre por prioridad o al revés; desplegables que no salen.
    #[gpui_kit::test]
    fn choosing_in_a_picker_narrows_the_list(cx: &mut TestAppContext) {
        let server = server_with_tasks(mixed_tasks());
        let (window, view) = open_with_view(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        assert!(on_screen(window, "filter-assignee", cx) && on_screen(window, "filter-priority", cx));
        assert!(!on_screen(window, "filter-clear", cx), "ofrece quitar filtros sin ninguno puesto");

        cx.update(|cx| view.update(cx, |view, cx| view.choose_priority("low".into(), cx)));
        assert!(on_screen(window, "work-row-t2", cx));
        assert!(!on_screen(window, "work-row-t1", cx), "la prioridad elegida no filtró");

        cx.update(|cx| view.update(cx, |view, cx| view.choose_priority(String::new(), cx)));
        cx.update(|cx| view.update(cx, |view, cx| view.choose_assignee("ana".into(), cx)));
        assert!(on_screen(window, "work-row-t1", cx));
        assert!(!on_screen(window, "work-row-t2", cx), "el responsable elegido no filtró");
        stop(cx);
    }

    /// Sin nada que separar no hay desplegables: una sola persona con una
    /// sola prioridad.
    ///
    /// Rotura que atrapa: desplegables de una opción ocupando la cabecera.
    #[gpui_kit::test]
    fn pickers_with_nothing_to_separate_are_not_shown(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));

        assert!(!on_screen(window, "filter-assignee", cx));
        assert!(!on_screen(window, "filter-priority", cx));
        stop(cx);
    }

    /// «Quitar filtros» lo quita todo, también lo escrito en el buscador.
    ///
    /// Roturas que atrapa: un botón que deja un filtro puesto; uno que
    /// limpia los desplegables y deja la búsqueda escondiendo tareas.
    #[gpui_kit::test]
    fn clearing_the_filters_brings_everything_back(cx: &mut TestAppContext) {
        let server = server_with_tasks(mixed_tasks());
        let (window, view) = open_with_view(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        cx.update_window(window, |_, window, cx| {
            window.click("work-search", cx);
            window.input("nada que exista", cx);
        })
        .unwrap();
        cx.update(|cx| view.update(cx, |view, cx| view.choose_assignee("ana".into(), cx)));
        cx.update(|cx| view.update(cx, |view, cx| view.choose_priority("urgent".into(), cx)));
        assert!(!on_screen(window, "work-row-t1", cx) && !on_screen(window, "work-row-t2", cx));

        click(window, "filter-clear", cx);

        assert!(on_screen(window, "work-row-t1", cx) && on_screen(window, "work-row-t2", cx), "quedó algún filtro puesto");
        assert!(!on_screen(window, "filter-clear", cx));
        stop(cx);
    }

    /// Pulsar una lente le hace al servidor la pregunta de esa lente, y
    /// «sólo lo abierto» pide también lo terminado.
    ///
    /// Roturas que atrapa: pestañas que cambian de color y enseñan lo mismo;
    /// un interruptor que no pide nada.
    #[gpui_kit::test]
    fn a_lens_and_the_open_only_switch_ask_the_server(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-t1", cx));
        let asked = |server: &FakeServer, what: &str| server.queries("/api/v1/tasks/").iter().any(|query| query.contains(what));
        assert!(asked(&server, "assignee=me") && !asked(&server, "watcher=me") && !asked(&server, "status=all"));

        click(window, "lens-watching", cx);
        wait_until(cx, "pregunta por lo que sigue", |_| asked(&server, "watcher=me"));

        click(window, "toggle-closed", cx);
        wait_until(cx, "pide también lo terminado", |_| asked(&server, "status=all"));
        stop(cx);
    }

    /// Con un ámbito puesto, la cabecera lo dice y su × lo quita.
    ///
    /// Roturas que atrapa: quedarse acotado sin saberlo; no tener cómo
    /// volver a verlo todo desde la propia lista.
    #[gpui_kit::test]
    fn the_scope_shows_in_the_header_and_can_be_cleared(cx: &mut TestAppContext) {
        let server = server_with_tasks(tasks());
        let window = open(&server, cx);
        wait_until(cx, "llega el árbol", |cx| on_screen(window, "tree-a", cx));
        assert!(!on_screen(window, "scope-clear", cx));

        click(window, "tree-a", cx);
        wait_until(cx, "sólo las de ese espacio", |cx| on_screen(window, "scope-clear", cx) && !on_screen(window, "work-row-t1", cx));
        assert!(on_screen(window, "work-row-t2", cx));

        click(window, "scope-clear", cx);
        wait_until(cx, "vuelven todas", |cx| on_screen(window, "work-row-t1", cx));
        assert!(!on_screen(window, "scope-clear", cx));
        stop(cx);
    }
}
