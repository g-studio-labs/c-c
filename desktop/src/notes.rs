//! Notas: las páginas personales en un árbol, que se leen y se escriben
//! (`app/src/pages/Notes.tsx`, command-and-control-511).
//!
//! Qué filas tiene el árbol, qué se manda al guardar, qué se pregunta antes
//! de perder texto y qué dice cada fallo lo decide el core
//! (`cac_core::notes`); aquí se pinta y se le pasa cada gesto. A la izquierda
//! el navegador (`w-64`), a la derecha la página.
//!
//! Escribir es el editor de markdown de la app (`editor.rs`) con la página
//! ya pintada al lado, como en la documentación: no es el tiptap de la app de
//! siempre y se guarda con su botón (decidido con jose, 18-nov-2026).

use cac_core::task::Question;
use cac_core::{note_link_target, note_rows, NoteEdit, NoteRow, NotesState};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::text::TextView;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::editor::MarkdownEditor;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// El navegador (`w-64`).
const NAVIGATOR_WIDTH: f32 = 256.;
/// Lo que entra cada nivel del árbol (`8 + depth * 14`).
const STEP: f32 = 14.;
/// La medida de la página (`max-w-3xl`).
const PAGE_WIDTH: f32 = 768.;

pub struct NotesView {
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    search: Entity<InputState>,
    /// De qué sesión de escritura (`NoteEdit::session`) es el texto que hay
    /// en las cajas: se pone una vez al abrirse el editor, no en cada tecla.
    loaded_session: Option<u64>,
    question_focus: FocusHandle,
    was_asking: bool,
    /// El foco de la papelera, con el que Escape la cierra.
    trash_focus: FocusHandle,
    was_trash: bool,
    was_searching: bool,
    _subscriptions: Vec<Subscription>,
}

/// Un botón pequeño de cabecera: su icono y, al pasar el ratón, qué es.
fn tool(id: impl Into<ElementId>, icon: impl Into<Icon>, label: String, tone: Option<Hsla>, cx: &App, click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    let theme = cx.theme();
    let (muted, hover, foreground) = (tone.unwrap_or(theme.muted_foreground), crate::theme::ghost_hover(cx), theme.foreground);
    div()
        .id(id)
        .size(px(24.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .text_color(muted)
        .cursor_pointer()
        .hover(move |style| style.bg(hover).text_color(foreground))
        .child(Icon::new(icon).size_3p5())
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .on_click(click)
        .test_support()
}

/// Un botón pequeño con borde: los de Editar y Cancelar.
fn text_button(id: impl Into<ElementId>, label: String, active: bool, tone: Option<Hsla>, cx: &App, click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    let theme = cx.theme();
    let (border, accent, foreground) = (theme.border, theme.accent, theme.foreground);
    div()
        .id(id)
        .h_7()
        .px_2p5()
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(8.))
        .border_1()
        .border_color(border)
        .leading_xs()
        .font_weight(FontWeight::MEDIUM)
        .when(active, |this| this.bg(accent))
        .when_some(tone, |this, tone| this.text_color(tone))
        .cursor_pointer()
        .hover(move |style| style.bg(accent).text_color(foreground))
        .child(label)
        .on_click(click)
        .test_support()
}

/// Guarda lo escrito; un título en blanco queda como «Sin título».
fn save(cx: &App) {
    let untitled = cac_core::text(locale(cx), "work:notes.untitled");
    Backend::run(cx, move |core| async move { core.notes().save(&untitled).await });
}

fn section_title(text: String, muted: Hsla) -> Div {
    div().leading_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(text.to_uppercase())
}

/// Una caja encima de toda la ventana, con su velo: lo de debajo no recibe
/// ni teclas ni ratón mientras está.
fn modal(id: &'static str, card: impl IntoElement, window: &Window) -> impl IntoElement {
    let size = window.viewport_size();
    deferred(
        anchored().position(Point::default()).child(
            div().id(id).w(size.width).h(size.height).occlude().flex().items_center().justify_center().p_4().bg(gpui_kit::black().opacity(0.35)).child(card),
        ),
    )
    .with_priority(2)
}

/// Un clic en un enlace del texto: otra nota se abre aquí; una dirección web,
/// en el navegador.
fn follow_link(url: &str, cx: &mut App) {
    if let Some(id) = note_link_target(url) {
        Backend::run(cx, move |core| async move { core.notes().open(&id).await });
    } else if url.starts_with("http://") || url.starts_with("https://") {
        cx.open_url(url);
    }
}

fn markdown(id: impl Into<ElementId>, text: &str, cx: &App) -> TextView {
    TextView::markdown(id, cac_core::prose_for_painting(text)).style(crate::docs::reading_style(cx)).on_link_click(|url, _, _, cx| follow_link(url, cx))
}

impl NotesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let untitled = tr(cx, "work:notes.untitled");
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(untitled));
        let body = cx.new(|cx| TextareaState::new(window, cx));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr(cx, "work:notes.searchNotes")));
        let subscriptions = vec![
            cx.subscribe_in(&title, window, |this, title, event: &InputEvent, window, cx| match event {
                InputEvent::Change => Backend::core(cx).notes().draft_title(&title.read(cx).value()),
                // Enter pasa al cuerpo, como en la app de siempre.
                InputEvent::PressEnter { .. } => this.body.update(cx, |input, cx| input.focus(window, cx)),
                _ => {}
            }),
            cx.subscribe(&body, |_, body, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    Backend::core(cx).notes().draft_body(&body.read(cx).value());
                }
            }),
            cx.subscribe(&search, |_, search, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = search.read(cx).value().to_string();
                    Backend::run(cx, move |core| async move { core.notes().search(&query).await });
                }
            }),
        ];
        Self {
            title,
            body,
            search,
            loaded_session: None,
            question_focus: cx.focus_handle(),
            was_asking: false,
            trash_focus: cx.focus_handle(),
            was_trash: false,
            was_searching: false,
            _subscriptions: subscriptions,
        }
    }

    /// Pone en las cajas el texto de esta sesión de escritura, una vez.
    fn load_boxes(&mut self, edit: &NoteEdit, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded_session == Some(edit.session) {
            return;
        }
        self.loaded_session = Some(edit.session);
        self.title.update(cx, |input, cx| input.set_value(edit.title.clone(), window, cx));
        self.body.update(cx, |input, cx| input.set_value(edit.body.clone(), window, cx));
        // Una nota recién creada o que se renombra empieza por su título.
        if edit.focus_title {
            self.title.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.body.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn render_row(row: NoteRow, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, hover, primary) = (theme.muted_foreground, crate::theme::ghost_hover(cx).opacity(0.5), theme.primary);
        let here = locale(cx);
        let id = row.id.clone();
        let group = SharedString::from(format!("note-group-{}", row.id));
        let title = NotesState::shown_title(&row.title, here);
        let (open_id, toggle_id, menu_id) = (row.id.clone(), row.id.clone(), row.id.clone());
        let favorite = row.favorite;
        let menu_label = tr(cx, "work:notes.pageMenu");
        let actions: Vec<(&'static str, String, Lucide)> = vec![
            ("new", tr(cx, "work:notes.newSubpage"), Lucide::Plus),
            ("rename", tr(cx, "work:notes.rename"), Lucide::Pencil),
            ("favorite", tr(cx, if favorite { "work:notes.removeFavorite" } else { "work:notes.addFavorite" }), Lucide::Star),
            ("up", tr(cx, "work:notes.moveUp"), Lucide::ArrowUp),
            ("down", tr(cx, "work:notes.moveDown"), Lucide::ArrowDown),
            ("delete", tr(cx, "work:notes.delete"), Lucide::Trash),
        ];
        let danger = theme.danger;
        div()
            .id(SharedString::from(format!("note-{id}")))
            .test_support()
            .group(group.clone())
            .h(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .pl(px(8. + STEP * row.depth as f32))
            .pr_2()
            .leading_sm()
            .when(row.active, |this| this.text_color(primary))
            .hover(move |style| style.bg(hover))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                let id = open_id.clone();
                Backend::run(cx, move |core| async move { core.notes().open(&id).await });
            })
            .child(
                div()
                    .id(SharedString::from(format!("note-toggle-{id}")))
                    .test_support()
                    .size(px(14.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .when(row.expandable, |this| {
                        this.child(Icon::new(if row.open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(14.)).text_color(muted)).on_click(move |_, _, cx| {
                            // El chevrón abre y cierra; no es un clic en la fila.
                            cx.stop_propagation();
                            Backend::core(cx).notes().toggle_fold(&toggle_id);
                        })
                    }),
            )
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title))
            .when(favorite, |this| this.child(Icon::new(Lucide::Star).size_3().flex_none().text_color(theme.warning)))
            .child(
                div().flex_none().opacity(0.).group_hover(group, |style| style.opacity(1.)).child(
                    Button::new(SharedString::from(format!("note-more-{id}")))
                        .ghost()
                        .xsmall()
                        .size(px(14.))
                        .p_0()
                        .child(Icon::new(IconName::Ellipsis).size_3p5().text_color(muted))
                        .tooltip(menu_label)
                        .dropdown_menu(move |mut menu, _, _| {
                            for (slug, label, icon) in actions.clone() {
                                let (note, item_id) = (menu_id.clone(), SharedString::from(format!("note-act-{slug}-{menu_id}")));
                                let deletes = slug == "delete";
                                menu = menu.item(
                                    PopupMenuItem::element(move |_, _| div().id(item_id.clone()).when(deletes, |this| this.text_color(danger)).child(label.clone()).test_support())
                                        .icon(Icon::new(icon))
                                        .on_click(move |_, _, cx| {
                                            let note = note.clone();
                                            let untitled = cac_core::text(crate::lang::locale(cx), "work:notes.untitled");
                                            match slug {
                                                "new" => Backend::run(cx, move |core| async move { core.notes().create(Some(&note), &untitled).await }),
                                                "rename" => Backend::run(cx, move |core| async move { core.notes().begin_rename(&note).await }),
                                                "favorite" => Backend::run(cx, move |core| async move { core.notes().toggle_favorite(&note).await }),
                                                "up" => Backend::run(cx, move |core| async move { core.notes().move_by_one(&note, true).await }),
                                                "down" => Backend::run(cx, move |core| async move { core.notes().move_by_one(&note, false).await }),
                                                _ => Backend::core(cx).notes().ask_delete(&note),
                                            }
                                        }),
                                );
                            }
                            menu
                        }),
                ),
            )
    }

    fn render_navigator(&self, state: &NotesState, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, danger, warning) = (theme.border, theme.muted_foreground, crate::theme::ghost_hover(cx).opacity(0.5), theme.danger, theme.warning);
        let here = locale(cx);
        let untitled = tr(cx, "work:notes.untitled");
        let rows = note_rows(state);
        let favorites: Vec<_> = state.favorites().into_iter().map(|item| (item.id.clone(), NotesState::shown_title(&item.title, here), state.open_id.as_deref() == Some(&item.id))).collect();
        let problem = state.problem.as_ref().map(|problem| problem.phrase(here));
        let empty = state.tree_loaded && rows.is_empty();
        let loading = !state.tree_loaded && rows.is_empty();
        let primary = theme.primary;
        div()
            .id("notes-navigator")
            .test_support()
            .w(px(NAVIGATOR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(border)
            .bg(theme.muted.opacity(0.1))
            .child(
                div()
                    .h_12()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .border_b_1()
                    .border_color(border)
                    .child(div().id("notes-title").test_support().leading_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, "work:notes.title")))
                    .child(div().ml_auto().child(tool("note-search", IconName::Search, tr(cx, "work:notes.search"), None, cx, |_, _, cx| Backend::core(cx).notes().open_search())))
                    .child(tool("note-trash", Lucide::Trash, tr(cx, "work:notes.trash"), None, cx, |_, _, cx| Backend::run(cx, |core| async move { core.notes().open_trash().await })))
                    .child(tool("note-new", Lucide::Plus, tr(cx, "work:notes.newPage"), None, cx, move |_, _, cx| {
                        let untitled = untitled.clone();
                        Backend::run(cx, move |core| async move { core.notes().create(None, &untitled).await })
                    })),
            )
            .children(problem.map(|said| {
                div()
                    .id("notes-problem")
                    .test_support()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .leading_xs()
                    .text_color(danger)
                    .cursor_pointer()
                    .on_click(|_, _, cx| Backend::core(cx).notes().dismiss_problem())
                    .aria_label(said.clone())
                    .child(Icon::new(Lucide::CircleAlert).size_3().flex_none())
                    .child(said)
            }))
            .child(
                div()
                    .id("notes-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .py_1()
                    .when(!favorites.is_empty(), |this| {
                        this.child(
                            div()
                                .id("notes-favorites")
                                .test_support()
                                .mb_1()
                                .pb_1()
                                .border_b_1()
                                .border_color(border)
                                .child(div().px_3().py_1().child(section_title(tr(cx, "work:notes.favorites"), muted)))
                                .children(favorites.into_iter().map(|(id, title, active)| {
                                    let open = id.clone();
                                    div()
                                        .id(SharedString::from(format!("note-favorite-{id}")))
                                        .test_support()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .px_2()
                                        .pl_3()
                                        .py_1()
                                        .leading_sm()
                                        .when(active, |this| this.text_color(primary))
                                        .hover(move |style| style.bg(hover))
                                        .cursor_pointer()
                                        .on_click(move |_, _, cx| {
                                            let open = open.clone();
                                            Backend::run(cx, move |core| async move { core.notes().open(&open).await })
                                        })
                                        .child(Icon::new(Lucide::Star).size_3().flex_none().text_color(warning))
                                        .child(div().min_w_0().truncate().child(title))
                                })),
                        )
                    })
                    .when(loading, |this| this.child(div().px_3().py_2().leading_xs().text_color(muted).child(tr(cx, "common:servers.loading"))))
                    .when(empty, |this| this.child(div().id("notes-empty").test_support().px_3().py_3().leading_xs().text_color(muted).child(tr(cx, "work:notes.noPages"))))
                    .children(rows.into_iter().map(|row| Self::render_row(row, cx))),
            )
    }

    /// El aviso de que lo escrito quedó en otra página porque otro
    /// dispositivo llegó antes.
    fn render_conflict(state: &NotesState, cx: &App) -> Option<impl IntoElement> {
        let conflict = state.conflict.clone()?;
        let theme = cx.theme();
        let (warning, foreground, muted) = (theme.warning, theme.foreground, theme.muted_foreground);
        let name = NotesState::shown_title(&conflict.title, locale(cx));
        let said = cac_core::text_with(locale(cx), "work:notes.conflictBody", &[("name", &name)]);
        let open = conflict.id.clone();
        Some(
            div()
                .id("note-conflict")
                .test_support()
                .flex_none()
                .mx_6()
                .mt_3()
                .px_3()
                .py_2()
                .rounded(px(8.))
                .border_1()
                .border_color(warning.opacity(0.4))
                .bg(warning.opacity(0.07))
                .leading_xs()
                .child(div().font_weight(FontWeight::MEDIUM).text_color(foreground).child(tr(cx, "work:notes.conflictTitle")))
                .child(div().mt_0p5().text_color(muted).child(said))
                .child(
                    div()
                        .mt_2()
                        .flex()
                        .gap_2()
                        .child(text_button("note-conflict-open", tr(cx, "work:notes.open"), false, None, cx, move |_, _, cx| {
                            let open = open.clone();
                            Backend::core(cx).notes().dismiss_conflict();
                            Backend::run(cx, move |core| async move { core.notes().open(&open).await })
                        }))
                        .child(text_button("note-conflict-dismiss", tr(cx, "work:notes.close"), false, None, cx, |_, _, cx| Backend::core(cx).notes().dismiss_conflict())),
                ),
        )
    }

    fn render_pane(&mut self, state: &NotesState, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, primary, danger, warning) = (theme.border, theme.muted_foreground, theme.primary, theme.danger, theme.warning);
        let here = locale(cx);
        let frame = || div().id("notes-pane").test_support().flex_1().min_w_0().h_full().flex().flex_col();
        let Some(open_id) = state.open_id.clone() else {
            return frame()
                .items_center()
                .justify_center()
                .child(Icon::new(Lucide::FileText).size(px(32.)).text_color(muted.opacity(0.5)))
                .child(div().id("notes-pick").test_support().mt_2().leading_sm().text_color(muted).child(tr(cx, "work:notes.pickAPage")))
                .into_any_element();
        };
        let Some(detail) = state.detail.clone().filter(|detail| detail.note.id == open_id) else {
            let said = if state.detail_failed { "work:notes.couldNotLoadPage" } else { "common:servers.loading" };
            return frame().items_center().justify_center().child(div().id("notes-loading").test_support().leading_sm().text_color(muted).child(tr(cx, said))).into_any_element();
        };
        let note = detail.note.clone();
        let editing = state.edit.clone().filter(|edit| edit.id == note.id);
        if let Some(edit) = &editing {
            self.load_boxes(edit, window, cx);
        }

        let close = tool("note-close", Lucide::X, tr(cx, "work:notes.close"), None, cx, |_, _, cx| Backend::core(cx).notes().close());
        let header = div().h_12().flex_none().flex().items_center().gap_2().px_4().border_b_1().border_color(border);
        let header = match &editing {
            Some(edit) => {
                let status = edit.status(here).map(|said| div().id("note-status").test_support().flex_none().leading_xs().text_color(if edit.saving { warning } else { muted }).aria_label(said.clone()).child(said));
                let failed = edit.error_phrase(here).map(|said| {
                    div().id("note-error").test_support().min_w_0().flex().items_center().gap_1().leading_xs().text_color(danger).aria_label(said.clone()).child(Icon::new(Lucide::CircleAlert).size_3().flex_none()).child(said)
                });
                header
                    .child(div().flex_1().min_w_0().child(Input::new(&self.title).id("note-title")))
                    .children(failed)
                    .children(status)
                    .child(Button::new("note-save").primary().small().h_7().px_2p5().label(tr(cx, "work:docs.save")).disabled(!edit.can_save()).loading(edit.saving).on_click(|_, _, cx| {
                        save(cx)
                    }))
                    .child(text_button("note-cancel", tr(cx, "work:docs.cancel"), false, None, cx, |_, _, cx| Backend::core(cx).notes().cancel_edit()))
                    .child(close)
            }
            None => header
                .child({
                    let shown = NotesState::shown_title(&note.title, here);
                    div().id("note-heading").test_support().flex_1().min_w_0().truncate().leading_sm().font_weight(FontWeight::MEDIUM).aria_label(shown.clone()).child(shown)
                })
                .child(text_button("note-edit", tr(cx, "work:docs.edit"), false, None, cx, |_, _, cx| {
                    Backend::core(cx).notes().start_edit();
                }))
                .child(close),
        };

        let content: AnyElement = match &editing {
            Some(edit) => {
                // Lado a lado si caben los dos; si no, uno u otro. El navegador
                // se lleva su ancho.
                let side = cac_core::doc_editor_side_by_side(f32::from(window.viewport_size().width) - NAVIGATOR_WIDTH);
                let (shows_box, shows_preview) = (side || !edit.previewing, side || edit.previewing);
                let previewing = edit.previewing;
                let switch = (!side).then(|| {
                    div()
                        .id("note-edit-switch")
                        .test_support()
                        .flex()
                        .flex_none()
                        .gap_1()
                        .child(text_button("note-edit-show-box", tr(cx, "work:docs.write"), !previewing, None, cx, |_, _, cx| Backend::core(cx).notes().show_preview(false)))
                        .child(text_button("note-edit-show-preview", tr(cx, "work:docs.preview"), previewing, None, cx, |_, _, cx| Backend::core(cx).notes().show_preview(true)))
                });
                let previewed = edit.body.trim().to_string();
                let preview = div()
                    .id("note-edit-preview")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .p_4()
                    .map(|this| {
                        if previewed.is_empty() {
                            this.child(div().leading_sm().text_color(muted).child(tr(cx, "work:docs.emptyTab")))
                        } else {
                            this.child(div().id("note-edit-preview-text").test_support().size_full().child(markdown(SharedString::from(format!("note-edit-preview-{}", note.id)), &previewed, cx).scrollable(true)))
                        }
                    });
                div()
                    .id("note-editor")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .children(switch)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .gap_4()
                            .when(shows_box, |this| {
                                this.child(div().id("note-edit-box").flex_1().min_w_0().min_h_0().flex().flex_col().child(MarkdownEditor::new("note-edit", &self.body).fill().placeholder(tr(cx, "work:notes.writePlaceholder"))))
                            })
                            .when(shows_preview, |this| this.child(preview)),
                    )
                    // Ctrl+S (Cmd en un Mac) guarda, se esté en la caja que se esté.
                    .on_key_down(|event: &KeyDownEvent, _, cx| {
                        let held = event.keystroke.modifiers;
                        if held.secondary(cx) && !held.shift && !held.alt && event.keystroke.key == "s" {
                            save(cx);
                            cx.stop_propagation();
                        }
                    })
                    .into_any_element()
            }
            None => {
                let children: Vec<_> = state.unlinked_children().into_iter().map(|item| (item.id.clone(), NotesState::shown_title(&item.title, here))).collect();
                let backlinks = detail.backlinks.clone();
                let body = note.body.trim().to_string();
                let link = |id: String, title: String, prefix: &'static str, icon: Option<Lucide>| {
                    let open = id.clone();
                    div()
                        .id(SharedString::from(format!("{prefix}-{id}")))
                        .test_support()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .max_w_full()
                        .leading_sm()
                        .text_color(primary)
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(move |_, _, cx| {
                            let open = open.clone();
                            Backend::run(cx, move |core| async move { core.notes().open(&open).await })
                        })
                        .children(icon.map(|icon| Icon::new(icon).size_3p5().flex_none().text_color(muted)))
                        .child(div().min_w_0().truncate().child(title))
                };
                div()
                    .id("note-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(px(PAGE_WIDTH))
                            .flex()
                            .flex_col()
                            .child(if body.is_empty() {
                                div().id("note-empty").test_support().leading_sm().text_color(muted).child(tr(cx, "work:notes.writePlaceholder")).into_any_element()
                            } else {
                                div().id("note-body").test_support().text_size(px(15.)).line_height(relative(1.75)).child(markdown(SharedString::from(format!("note-text-{}", note.id)), &body, cx)).into_any_element()
                            })
                            .when(!children.is_empty(), |this| {
                                this.child(
                                    div()
                                        .id("note-subpages")
                                        .test_support()
                                        .mt_8()
                                        .pt_4()
                                        .border_t_1()
                                        .border_color(border)
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(section_title(tr(cx, "work:notes.subpages"), muted))
                                        .children(children.into_iter().map(|(id, title)| link(id, title, "note-subpage", Some(Lucide::FileText)))),
                                )
                            })
                            .when(!backlinks.is_empty(), |this| {
                                this.child(
                                    div()
                                        .id("note-backlinks")
                                        .test_support()
                                        .mt_8()
                                        .pt_4()
                                        .border_t_1()
                                        .border_color(border)
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(section_title(tr(cx, "work:notes.linkedFrom"), muted))
                                        .children(backlinks.into_iter().map(|found| {
                                            let title = NotesState::shown_title(&found.title, here);
                                            link(found.id, title, "note-backlink", None)
                                        })),
                                )
                            }),
                    )
                    .into_any_element()
            }
        };
        frame().child(header).children(Self::render_conflict(state, cx)).child(content).into_any_element()
    }

    fn render_search(&mut self, state: &NotesState, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.was_searching {
            self.search.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        }
        let theme = cx.theme();
        let (muted, hover, foreground, background, danger) = (theme.muted_foreground, crate::theme::ghost_hover(cx), theme.foreground, theme.background, theme.danger);
        let here = locale(cx);
        let query = state.search.query.trim().to_string();
        let found = state.search.results.clone();
        let card = div()
            .id("note-search-card")
            .test_support()
            .w_full()
            .max_w(px(512.))
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(px(14.))
            .border_1()
            .border_color(foreground.opacity(0.1))
            .bg(background)
            .shadow_2xl()
            .on_key_down(|event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    Backend::core(cx).notes().close_search();
                    cx.stop_propagation();
                }
            })
            .child(div().flex().items_center().gap_2().leading_sm().font_weight(FontWeight::MEDIUM).child(Icon::new(IconName::Search).size_4()).child(tr(cx, "work:notes.search")))
            .child(Input::new(&self.search).id("note-search-input"))
            .child(
                div()
                    .id("note-search-results")
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .when(state.search.searching, |this| this.child(div().px_1().py_2().leading_xs().text_color(muted).child(tr(cx, "common:servers.loading"))))
                    .when_some(state.search.failed.as_ref().map(|failed| failed.phrase(here)), |this, said| {
                        this.child(div().id("note-search-failed").test_support().px_1().py_2().leading_xs().text_color(danger).aria_label(said.clone()).child(said))
                    })
                    .when(!state.search.searching && state.search.failed.is_none() && !query.is_empty() && found.is_empty(), |this| {
                        this.child(div().id("note-search-none").test_support().px_1().py_2().leading_xs().text_color(muted).child(tr(cx, "work:notes.noMatches")))
                    })
                    .when(!state.search.searching && !query.is_empty(), |this| {
                        this.children(found.into_iter().map(|found| {
                            let open = found.id.clone();
                            div()
                                .id(SharedString::from(format!("note-found-{}", found.id)))
                                .test_support()
                                .flex()
                                .flex_col()
                                .px_2()
                                .py_1p5()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(move |style| style.bg(hover))
                                .on_click(move |_, _, cx| {
                                    let open = open.clone();
                                    Backend::core(cx).notes().close_search();
                                    Backend::run(cx, move |core| async move { core.notes().open(&open).await })
                                })
                                .child(div().truncate().leading_sm().font_weight(FontWeight::MEDIUM).child(NotesState::shown_title(&found.title, here)))
                                .when(!found.excerpt.is_empty(), |this| this.child(div().truncate().leading_xs().text_color(muted).child(found.excerpt)))
                        }))
                    }),
            );
        modal("note-search-veil", card, window)
    }

    fn render_trash(&mut self, state: &NotesState, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.was_trash {
            self.trash_focus.focus(window, cx);
        }
        let theme = cx.theme();
        let (muted, foreground, background, danger) = (theme.muted_foreground, theme.foreground, theme.background, theme.danger);
        let here = locale(cx);
        let items = state.trash.items.clone();
        let has_items = !items.is_empty();
        let hover = crate::theme::ghost_hover(cx);
        let card = div()
            .id("note-trash-card")
            .test_support()
            .track_focus(&self.trash_focus)
            .w_full()
            .max_w(px(512.))
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(px(14.))
            .border_1()
            .border_color(foreground.opacity(0.1))
            .bg(background)
            .shadow_2xl()
            .on_key_down(|event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    Backend::core(cx).notes().close_trash();
                }
                cx.stop_propagation();
            })
            .child(div().flex().items_center().gap_2().leading_sm().font_weight(FontWeight::MEDIUM).child(Icon::new(Lucide::Trash).size_4()).child(tr(cx, "work:notes.trash")).child(
                div().ml_auto().child(Button::new("note-trash-close").ghost().xsmall().size_7().p_0().child(Icon::new(Lucide::X).size_4()).on_click(|_, _, cx| Backend::core(cx).notes().close_trash())),
            ))
            .child(
                div()
                    .id("note-trash-items")
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .when(state.trash.loading && items.is_empty(), |this| this.child(div().px_1().py_2().leading_xs().text_color(muted).child(tr(cx, "common:servers.loading"))))
                    .when(!state.trash.loading && items.is_empty(), |this| {
                        this.child(div().id("note-trash-empty").test_support().px_1().py_2().leading_xs().text_color(muted).child(tr(cx, "work:notes.trashEmpty")))
                    })
                    .children(items.into_iter().map(|item| {
                        let (restore, purge) = (item.id.clone(), item.id.clone());
                        let when = item.deleted_on(here);
                        let subpages = (item.subpages > 0).then(|| cac_core::text_count(here, "common:count.subpages", item.subpages as i64));
                        let meta = [Some(when).filter(|when| !when.is_empty()), subpages].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                        div()
                            .id(SharedString::from(format!("note-trashed-{}", item.id)))
                            .test_support()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1p5()
                            .rounded(px(6.))
                            .hover(move |style| style.bg(hover))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .child(div().truncate().leading_sm().child(NotesState::shown_title(&item.title, here)))
                                    .child(div().truncate().leading_xs().text_color(muted).child(meta)),
                            )
                            .child(
                                text_button(SharedString::from(format!("note-restore-{}", item.id)), tr(cx, "work:notes.restore"), false, None, cx, move |_, _, cx| {
                                    let restore = restore.clone();
                                    Backend::run(cx, move |core| async move { core.notes().restore(&restore).await })
                                }),
                            )
                            .child(
                                tool(SharedString::from(format!("note-purge-{}", item.id)), Lucide::Trash, tr(cx, "work:notes.deleteForever"), Some(danger.opacity(0.7)), cx, move |_, _, cx| Backend::core(cx).notes().ask_purge(&purge)),
                            )
                    })),
            )
            .when(has_items, |this| {
                this.child(div().flex().child(text_button("note-trash-empty-all", tr(cx, "work:notes.emptyTrash"), false, Some(danger), cx, |_, _, cx| Backend::core(cx).notes().ask_empty_trash())))
            });
        modal("note-trash-veil", card, window)
    }

    fn render_question(&mut self, question: Question, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.was_asking {
            self.question_focus.focus(window, cx);
        }
        crate::ask::question_dialog("note-ask", question, &self.question_focus, |yes, _, cx| Backend::run(cx, move |core| async move { core.notes().answer(yes).await }), window, cx)
    }
}

impl Render for NotesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).notes.clone();
        let (background, foreground) = (cx.theme().background, cx.theme().foreground);
        let question = state.question(locale(cx));
        let navigator = self.render_navigator(&state, cx);
        let pane = self.render_pane(&state, window, cx);
        let search = state.search.open.then(|| self.render_search(&state, window, cx));
        let trash = state.trash.open.then(|| self.render_trash(&state, window, cx));
        let asking = question.map(|question| self.render_question(question, window, cx));
        self.was_asking = asking.is_some();
        self.was_searching = search.is_some();
        self.was_trash = trash.is_some();
        div()
            .id("notes")
            .test_support()
            .size_full()
            .relative()
            .flex()
            .bg(background)
            .text_color(foreground)
            .child(navigator)
            .child(pane)
            .children(search)
            .children(trash)
            .children(asking)
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::NotesView;
    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::path_regex;
    use wiremock::{Mock, Request, ResponseTemplate};

    #[derive(Clone)]
    struct Stored {
        id: &'static str,
        parent: Option<&'static str>,
        title: String,
        body: String,
        version: u32,
        trashed: bool,
    }

    fn stored(id: &'static str, parent: Option<&'static str>, title: &str, body: &str) -> Stored {
        Stored { id, parent, title: title.into(), body: body.into(), version: 1, trashed: false }
    }

    fn note(stored: &Stored) -> Value {
        json!({ "id": stored.id, "parentId": stored.parent, "title": stored.title, "body": stored.body, "bodyHash": format!("h{}", stored.version) })
    }

    /// Alfa (con Beta dentro) y Gamma, con su texto.
    fn server() -> (FakeServer, Arc<Mutex<Vec<Stored>>>) {
        let world = Arc::new(Mutex::new(vec![stored("a", None, "Alfa", "Hola **mundo**"), stored("b", Some("a"), "Beta", "Dentro."), stored("c", None, "Gamma", "")]));
        let seen = world.clone();
        let server = FakeServer::start();
        server.mount(Mock::given(path_regex("^/api/v1/notes(/|$)")).respond_with(move |request: &Request| {
            let mut notes = seen.lock().unwrap();
            let route = request.url.path().trim_start_matches("/api/v1/notes").to_string();
            let parts: Vec<&str> = route.split('/').filter(|part| !part.is_empty()).collect();
            let sent: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            match (request.method.as_str(), parts.as_slice()) {
                ("GET", []) => ok(Value::Array(
                    notes.iter().filter(|n| !n.trashed).enumerate().map(|(at, n)| json!({ "id": n.id, "parentId": n.parent, "position": at, "title": n.title, "hasBody": !n.body.is_empty(), "favorite": false })).collect(),
                )),
                ("GET", ["search"]) => {
                    let wanted = request.url.query_pairs().find(|(key, _)| key == "q").map(|(_, value)| value.to_lowercase()).unwrap_or_default();
                    if wanted == "boom" {
                        return ResponseTemplate::new(500).set_body_json(json!({ "success": false, "error": "internal", "message": "boom" }));
                    }
                    ok(Value::Array(notes.iter().filter(|n| n.title.to_lowercase().contains(&wanted)).map(|n| json!({ "id": n.id, "title": n.title, "excerpt": n.body })).collect()))
                }
                ("GET", ["trash"]) => ok(Value::Array(notes.iter().filter(|n| n.trashed).map(|n| json!({ "id": n.id, "title": n.title, "deletedAt": "2026-11-25T10:00:00Z", "subpages": 0 })).collect())),
                ("GET", [id]) => match notes.iter().find(|n| n.id == *id) {
                    Some(found) => ok(json!({ "note": note(found), "attachments": null, "backlinks": null })),
                    None => ResponseTemplate::new(404).set_body_json(json!({ "success": false, "error": "not-found" })),
                },
                ("POST", []) => {
                    notes.push(stored("n1", sent["parentId"].as_str().map(|_| "a"), sent["title"].as_str().unwrap_or_default(), ""));
                    ok(note(notes.last().unwrap()))
                }
                ("PATCH", [id]) => {
                    let found = notes.iter_mut().find(|n| n.id == *id).unwrap();
                    if let Some(title) = sent.get("title").and_then(Value::as_str) {
                        found.title = title.into();
                    }
                    if let Some(body) = sent.get("body").and_then(Value::as_str) {
                        found.body = body.into();
                        found.version += 1;
                    }
                    ok(json!({ "note": note(found) }))
                }
                ("DELETE", [id]) => {
                    for n in notes.iter_mut().filter(|n| n.id == *id || n.parent == Some(id)) {
                        n.trashed = true;
                    }
                    ok(json!({ "deleted": 2 }))
                }
                ("POST", ["trash", id, "restore"]) => {
                    notes.iter_mut().filter(|n| n.id == *id).for_each(|n| n.trashed = false);
                    ok(json!({ "restored": 1 }))
                }
                _ => ResponseTemplate::new(404).set_body_json(json!({ "success": false, "error": "not-found" })),
            }
        }));
        (server, world)
    }

    fn open(server: &FakeServer, width: f32, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<NotesView>) {
        server.install(cx);
        sign_in(cx);
        let (window, view) = cx.update(|cx| {
            let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(width), px(760.)) })), ..WindowOptions::default() };
            let (window, view) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    // Quien pone esta vista (el espacio de trabajo) es quien la
                    // repinta cuando el core cambia; aquí va sola.
                    cx.observe(&Backend::snapshot(cx), |_, _, cx| cx.notify()).detach();
                    NotesView::new(window, cx)
                })
            })
            .expect("open test window");
            (gpui_kit::AnyWindowHandle::from(window), view)
        });
        cx.update(|cx| Backend::run(cx, |core| async move { core.notes().enter().await }));
        wait_until(cx, "llega el árbol", |cx| on_screen(window, "note-a", cx));
        (window, view)
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

    fn label_of(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).label().unwrap_or_default().to_string()
        })
        .unwrap()
    }

    fn type_in(window: AnyWindowHandle, id: &str, text: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
            window.input(text, cx);
        })
        .unwrap();
    }

    fn state(cx: &mut TestAppContext) -> cac_core::NotesState {
        cx.update(|cx| Backend::snapshot(cx).read(cx).notes.clone())
    }

    fn writes(server: &FakeServer) -> Vec<(String, String, Value)> {
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() != "GET")
            .map(|request| (request.method.to_string(), request.url.path().to_string(), serde_json::from_slice(&request.body).unwrap_or(Value::Null)))
            .collect()
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Abre el «···» de una nota y pulsa una de sus entradas.
    fn choose(window: AnyWindowHandle, note: &str, action: &str, cx: &mut TestAppContext) {
        let item = format!("note-act-{action}-{note}");
        click(window, &format!("note-more-{note}"), cx);
        wait_until(cx, "el menú de la fila se abre", |cx| on_screen(window, &item, cx));
        click(window, &item, cx);
    }

    /// El árbol sale con sus hijas dentro, y plegar las esconde; una nota se
    /// abre al pulsarla y se lee ya pintada.
    ///
    /// Roturas que atrapa: un navegador sin las hijas; un chevrón que abre la
    /// nota en vez de plegar; abrir sin pintar el texto (se vería el markdown
    /// crudo); una nota vacía sin su aviso.
    #[gpui_kit::test]
    fn the_tree_is_painted_and_a_note_is_read(cx: &mut TestAppContext) {
        let (server, _) = server();
        let (window, _view) = open(&server, 1200., cx);
        assert!(on_screen(window, "note-b", cx), "falta la hija");
        assert!(on_screen(window, "notes-pick", cx), "sin nota abierta se pide elegir una");

        click(window, "note-toggle-a", cx);
        wait_until(cx, "pliega", |cx| !on_screen(window, "note-b", cx));
        click(window, "note-toggle-a", cx);
        wait_until(cx, "despliega", |cx| on_screen(window, "note-b", cx));

        click(window, "note-a", cx);
        wait_until(cx, "se lee la nota", |cx| on_screen(window, "note-body", cx));
        // Las dos pulsaciones del chevrón van antes que esta apertura: si
        // hubieran pedido la nota, sus lecturas ya habrían llegado.
        assert_eq!(server.hits("/api/v1/notes/a"), 1, "el chevrón abrió la nota");
        assert_eq!(label_of(window, "note-heading", cx), "Alfa");
        assert!(on_screen(window, "note-subpages", cx) && on_screen(window, "note-subpage-b", cx), "las hijas que el texto no enlaza van al pie");

        click(window, "note-c", cx);
        wait_until(cx, "se lee la vacía", |cx| on_screen(window, "note-empty", cx));
        stop(cx);
    }

    /// Escribir guarda con su botón y sólo lo cambiado; con el texto
    /// tecleado en la caja llegando al borrador.
    ///
    /// Roturas que atrapa: un editor que no se abre; un teclazo que no llega
    /// al core; guardar sin la versión de la que salió; dejar el editor
    /// abierto tras guardar.
    #[gpui_kit::test]
    fn writing_saves_with_its_button_and_the_version_it_came_from(cx: &mut TestAppContext) {
        let (server, _) = server();
        let (window, _view) = open(&server, 1400., cx);
        click(window, "note-a", cx);
        wait_until(cx, "se lee", |cx| on_screen(window, "note-edit", cx));
        click(window, "note-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "note-editor", cx));
        assert!(!on_screen(window, "note-status", cx), "dice «sin guardar» sin haber tocado nada");

        type_in(window, "note-edit-input", "X", cx);
        wait_until(cx, "lo tecleado llega al borrador", |cx| state(cx).edit.is_some_and(|edit| edit.body.contains('X')));
        assert_eq!(label_of(window, "note-status", cx), "Cambios sin guardar");
        let typed = state(cx).edit.unwrap().body;

        click(window, "note-save", cx);
        wait_until(cx, "guardado, se vuelve a leer", |cx| state(cx).edit.is_none() && on_screen(window, "note-body", cx));
        assert_eq!(writes(&server), [("PATCH".to_string(), "/api/v1/notes/a".to_string(), json!({ "body": typed, "baseHash": "h1" }))]);
        stop(cx);
    }

    /// «Nueva» crea una nota y la deja abierta para titularla, y el título
    /// escrito se guarda sin mandar el texto; la entrada del menú de una nota
    /// la crea dentro de ella.
    ///
    /// Roturas que atrapa: crear sin abrirla; abrirla sin editor; un título
    /// tecleado que no llega al core; crearla fuera de la nota desde la que se
    /// pidió.
    #[gpui_kit::test]
    fn a_new_note_opens_ready_to_be_titled(cx: &mut TestAppContext) {
        let (server, _) = server();
        let (window, _view) = open(&server, 1200., cx);
        click(window, "note-new", cx);
        wait_until(cx, "se abre para escribir", |cx| on_screen(window, "note-editor", cx));
        assert_eq!(writes(&server)[0], ("POST".to_string(), "/api/v1/notes/".to_string(), json!({ "title": "Sin título" })));
        assert!(state(cx).edit.is_some_and(|edit| edit.focus_title));

        // El título que se escribe llega al borrador y se guarda solo, sin el texto.
        type_in(window, "note-title", "Plan", cx);
        wait_until(cx, "el título llega al borrador", |cx| state(cx).edit.is_some_and(|edit| edit.title.contains("Plan")));
        click(window, "note-save", cx);
        wait_until(cx, "guardado", |cx| state(cx).edit.is_none());
        let titled = state(cx).detail.unwrap().note.title;
        assert!(titled.contains("Plan"), "{titled}");
        assert_eq!(writes(&server)[1], ("PATCH".to_string(), "/api/v1/notes/n1".to_string(), json!({ "title": titled })));

        choose(window, "c", "new", cx);
        wait_until(cx, "la tercera", |_| writes(&server).len() == 3);
        assert_eq!(writes(&server)[2].2["parentId"], "c");
        stop(cx);
    }

    /// Con texto sin guardar, abrir otra nota pregunta en el diálogo: «Seguir
    /// editando» y Escape dejan todo como estaba; descartar abre la otra.
    ///
    /// Roturas que atrapa: perder lo escrito al pulsar otra nota; una
    /// pregunta que no se pinta; un Escape que descarta.
    #[gpui_kit::test]
    fn leaving_unsaved_text_asks_on_screen(cx: &mut TestAppContext) {
        let (server, _) = server();
        let (window, _view) = open(&server, 1400., cx);
        click(window, "note-a", cx);
        wait_until(cx, "se lee", |cx| on_screen(window, "note-edit", cx));
        click(window, "note-edit", cx);
        wait_until(cx, "sale el editor", |cx| on_screen(window, "note-edit-input", cx));
        type_in(window, "note-edit-input", "X", cx);
        wait_until(cx, "llega al borrador", |cx| state(cx).edit.is_some_and(|edit| edit.is_dirty()));

        click(window, "note-c", cx);
        wait_until(cx, "se pregunta", |cx| on_screen(window, "note-ask", cx));
        cx.update_window(window, |_, window, cx| window.press("escape", cx)).unwrap();
        wait_until(cx, "Escape la quita", |cx| !on_screen(window, "note-ask", cx));
        assert_eq!(state(cx).open_id.as_deref(), Some("a"));
        assert!(state(cx).edit.is_some_and(|edit| edit.is_dirty()), "Escape tiró lo escrito");

        click(window, "note-c", cx);
        wait_until(cx, "se pregunta otra vez", |cx| on_screen(window, "note-ask", cx));
        click(window, "note-ask-yes", cx);
        wait_until(cx, "se abre la otra", |cx| state(cx).open_id.as_deref() == Some("c") && state(cx).edit.is_none());
        stop(cx);
    }

    /// Borrar pregunta diciendo qué se va, y sólo el sí lo manda a la
    /// papelera; desde ella se recupera.
    ///
    /// Roturas que atrapa: borrar al pulsar la entrada; un diálogo sin el
    /// nombre de la nota; una papelera que no enseña lo borrado o cuyo
    /// «Recuperar» no vuelve a poner la nota en el árbol.
    #[gpui_kit::test]
    fn deleting_asks_first_and_the_trash_gives_it_back(cx: &mut TestAppContext) {
        let (server, _) = server();
        let (window, _view) = open(&server, 1200., cx);
        choose(window, "a", "delete", cx);
        wait_until(cx, "se pregunta", |cx| on_screen(window, "note-ask", cx));
        assert!(label_of(window, "note-ask-title", cx).contains("Alfa"));
        assert!(writes(&server).is_empty());
        click(window, "note-ask-no", cx);
        wait_until(cx, "se quita", |cx| !on_screen(window, "note-ask", cx));
        assert!(writes(&server).is_empty());

        choose(window, "a", "delete", cx);
        wait_until(cx, "se pregunta otra vez", |cx| on_screen(window, "note-ask", cx));
        click(window, "note-ask-yes", cx);
        wait_until(cx, "desaparece del árbol", |cx| !on_screen(window, "note-a", cx));

        click(window, "note-trash", cx);
        wait_until(cx, "la papelera la enseña", |cx| on_screen(window, "note-trashed-a", cx));
        click(window, "note-restore-a", cx);
        wait_until(cx, "vuelve al árbol", |cx| state(cx).tree.iter().any(|item| item.id == "a"));
        stop(cx);
    }

    /// Buscar abre su diálogo, enseña lo que encuentra y abre la nota pulsada.
    ///
    /// Roturas que atrapa: una búsqueda que no llega al core; resultados que
    /// no se abren; un diálogo que se queda tras elegir; un fallo del servidor
    /// leído como «nada coincide».
    #[gpui_kit::test]
    fn search_finds_a_note_and_opens_it(cx: &mut TestAppContext) {
        let (server, _) = server();
        let (window, _view) = open(&server, 1200., cx);
        click(window, "note-search", cx);
        wait_until(cx, "sale el diálogo", |cx| on_screen(window, "note-search-input", cx));
        type_in(window, "note-search-input", "gam", cx);
        wait_until(cx, "sale el resultado", |cx| on_screen(window, "note-found-c", cx));
        click(window, "note-found-c", cx);
        wait_until(cx, "se abre", |cx| state(cx).open_id.as_deref() == Some("c") && !on_screen(window, "note-search-card", cx));

        // Una búsqueda que falla lo dice; no se lee como «nada coincide».
        click(window, "note-search", cx);
        wait_until(cx, "sale el diálogo", |cx| on_screen(window, "note-search-input", cx));
        type_in(window, "note-search-input", "boom", cx);
        wait_until(cx, "dice que falló", |cx| on_screen(window, "note-search-failed", cx));
        assert!(!on_screen(window, "note-search-none", cx));
        stop(cx);
    }
}
