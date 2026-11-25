//! Espacios, como fichas y no como árbol (`OrgSpaces.tsx`): el árbol ya deja
//! recorrerlos; lo que no dice de un vistazo es cuán grande es cada uno, en
//! cuál puede ver un cliente y quién lo sostiene.

use cac_core::{space_access_text, space_cards, text_with, tree_edit_phrase};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::OrgView;
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// Lo que una ficha no puede estrechar: con menos, el nombre y el acceso se
/// cortan a una palabra.
const CARD_MIN: f32 = 290.;

/// Pedido desde una ficha: ir a las tareas de su espacio.
pub struct ManageSpace(pub String);

impl EventEmitter<ManageSpace> for OrgView {}

impl OrgView {
    fn start_new_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr(cx, "common:misc.name")));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create_space(window, cx);
            }
        });
        self._subscriptions.push(subscription);
        self.new_space = Some(input);
        cx.notify();
    }

    fn create_space(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.new_space.clone() else { return };
        let name = input.read(cx).value().to_string();
        if self.busy || name.trim().is_empty() {
            return;
        }
        self.busy = true;
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move { core.tree().create_space(&name).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => this.new_space = None,
                    Err(error) => {
                        let text = tree_edit_phrase(locale(cx), &error, "common:misc.errCreateSpace");
                        this.say(text, true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_spaces(&mut self, manage: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, primary, chip, card_bg) = (theme.border, theme.muted_foreground, theme.primary, theme.muted, theme.group_box);
        let spaces = Backend::snapshot(cx).read(cx).tree.spaces.clone();
        let cards = space_cards(&spaces, &self.projects);
        let lang = locale(cx);
        let busy = self.busy;

        let naming = self.new_space.clone().map(|input| {
            div()
                .id("org-new-space")
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(px(320.)).max_w_full().child(Input::new(&input).id("org-new-space-name")))
                .child(Button::new("org-new-space-create").primary().small().label(tr(cx, "org:create")).disabled(busy).on_click(cx.listener(|this, _, window, cx| this.create_space(window, cx))))
                .child(Button::new("org-new-space-cancel").ghost().small().label(tr(cx, "org:cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.new_space = None;
                    cx.notify();
                })))
                .test_support()
        });

        let grid: AnyElement = if cards.is_empty() {
            div().id("org-spaces-none").px_3().py_8().rounded(px(12.)).border_1().border_dashed().border_color(border).text_center().text_color(muted).child(tr(cx, "common:misc.noSpaces")).test_support().into_any_element()
        } else {
            div()
                .id("org-spaces")
                .flex()
                .flex_wrap()
                .gap_3()
                .children(cards.into_iter().map(|space| {
                    let hue = cac_core::hue_index(&space.id);
                    let dot = crate::theme::space_color(&space.color, hue, cx);
                    let access = space_access_text(&space.access, lang);
                    let tasks = text_with(lang, "org:spaceTasks", &[("all", &space.tasks.to_string()), ("open", &space.open.to_string())]);
                    let row = |label: String, value: AnyElement| div().flex().gap_2p5().child(div().w(px(78.)).flex_none().text_color(muted).child(label)).child(div().flex_1().min_w_0().truncate().child(value));
                    let id = space.id.clone();
                    let nobody = space.people.is_empty();
                    div()
                        .id(SharedString::from(format!("org-space-{}", space.id)))
                        .flex_1()
                        .min_w(px(CARD_MIN))
                        .flex()
                        .flex_col()
                        .gap_2p5()
                        .px_3p5()
                        .py_3()
                        .rounded(px(12.))
                        .border_1()
                        .border_color(border)
                        .bg(card_bg)
                        .child(div().flex().items_center().gap_2().child(div().size(px(10.)).flex_none().rounded_full().bg(dot)).child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(space.name.clone())))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .leading_xs()
                                .child(row(tr(cx, "common:misc.channel"), div().child(format!("#{}", space.name)).into_any_element()))
                                .child(row(tr(cx, "common:misc.tasks"), div().id(SharedString::from(format!("org-space-tasks-{}", space.id))).text_color(muted).aria_label(tasks.clone()).child(tasks).test_support().into_any_element()))
                                .child(row(tr(cx, "common:misc.access"), div().id(SharedString::from(format!("org-space-access-{}", space.id))).text_color(muted).aria_label(access.clone()).child(access).test_support().into_any_element())),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .children(space.people.iter().map(|face| {
                                    let handle = format!("@{}", face.username);
                                    div()
                                        .id(SharedString::from(format!("org-space-face-{}-{}", space.id, face.username)))
                                        .size(px(24.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(chip)
                                        .text_size(px(10.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(handle.clone()).build(window, cx))
                                        .child(face.initials.clone())
                                        .test_support()
                                }))
                                .when(space.more > 0, |this| this.child(div().text_size(px(11.)).text_color(muted).child(format!("+{}", space.more))))
                                .when(nobody, |this| this.child(div().text_size(px(11.)).text_color(muted).child(tr(cx, "org:nobodyOnIt"))))
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .id(SharedString::from(format!("org-space-manage-{}", space.id)))
                                        .text_size(px(11.5))
                                        .text_color(primary)
                                        .cursor_pointer()
                                        .hover(|style| style.underline())
                                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(ManageSpace(id.clone()))))
                                        .child(tr(cx, "common:misc.manage"))
                                        .test_support(),
                                ),
                        )
                        .test_support()
                }))
                .test_support()
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().text_color(muted).child(tr(cx, "common:misc.spacesLead")))
                    .child(div().flex_1())
                    .when(manage && self.new_space.is_none(), |this| {
                        this.child(Button::new("org-new-space").outline().small().icon(Icon::new(Lucide::Plus)).label(tr(cx, "common:misc.newSpace")).on_click(cx.listener(|this, _, window, cx| this.start_new_space(window, cx))))
                    }),
            )
            .children(naming)
            .child(grid)
            .into_any_element()
    }
}
