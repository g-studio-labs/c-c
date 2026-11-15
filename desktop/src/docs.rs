//! La documentación de un nodo, para leerla: sus cuatro pestañas y, en la de
//! decisiones, el registro vigente.
//!
//! Qué documento hay abierto, qué pestaña, qué se lee y por qué está vacío lo
//! dice el core. Aquí sólo se pinta. Escribirla sigue en la app de siempre.

use cac_core::{DocDecision, DOC_TABS};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};

/// La medida de lectura. Es para la prosa: una tabla ancha se desliza dentro.
const READING_WIDTH: f32 = 760.;

pub struct DocsView;

impl DocsView {
    fn render_decision(&self, decision: DocDecision, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, card, radius) = (theme.muted_foreground, theme.border, theme.group_box, theme.radius);
        let when = decision.decided_on(locale(cx));
        let by = [decision.author_name.clone(), Some(when), decision.tag.clone()]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let body = (!decision.body.trim().is_empty())
            .then(|| TextView::markdown(SharedString::from(format!("decision-body-{}", decision.id)), decision.body.clone()));

        div()
            .id(SharedString::from(format!("doc-decision-{}", decision.id)))
            .test_support()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .bg(card)
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(decision.title))
            .child(div().text_xs().text_color(muted).child(by))
            .children(body)
    }
}

impl Render for DocsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted, border, primary) =
            (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.primary);
        let docs = Backend::snapshot(cx).read(cx).docs.clone();
        let name = docs.target.as_ref().map(|target| target.name.clone()).unwrap_or_default();
        let node = docs.target.as_ref().map(|target| target.id.clone()).unwrap_or_default();
        let empty = docs.empty_reason().map(|key| tr(cx, key));
        let body = docs.body().to_string();
        let decisions = docs.standing_decisions();

        div()
            .id("docs")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .gap_3()
                    .px_4()
                    .pt_3()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().id("doc-title").test_support().text_lg().font_weight(FontWeight::SEMIBOLD).child(name))
                            .child(div().text_xs().text_color(muted).child(tr(cx, "work:board.view.docs")))
                            .child(div().flex_1())
                            // De vuelta a las tareas del mismo nodo.
                            .child(
                                Button::new("view-list")
                                    .ghost()
                                    .small()
                                    .label(tr(cx, "work:board.view.list"))
                                    .on_click(|_, _, cx| Backend::core(cx).back_to_tasks()),
                            ),
                    )
                    .child(div().flex().items_center().gap_4().children(DOC_TABS.map(|key| {
                        let active = docs.tab == key;
                        div()
                            .id(SharedString::from(format!("doc-tab-{key}")))
                            .test_support()
                            .pb_2()
                            .border_b_2()
                            .border_color(if active { primary } else { primary.opacity(0.) })
                            .text_sm()
                            .text_color(if active { foreground } else { muted })
                            .when(active, |this| this.font_weight(FontWeight::MEDIUM))
                            .cursor_pointer()
                            .on_click(move |_, _, cx| Backend::core(cx).docs().show_tab(key))
                            .child(tr(cx, &format!("work:docs.{key}")))
                    }))),
            )
            .child(
                div().id("doc-scroll").flex_1().overflow_y_scroll().p_4().child(
                    div()
                        .max_w(px(READING_WIDTH))
                        .flex()
                        .flex_col()
                        .gap_4()
                        .children(empty.map(|phrase| div().id("doc-empty").test_support().text_sm().text_color(muted).child(phrase)))
                        .when(!body.is_empty(), |this| {
                            // Un id por nodo y pestaña: el del texto anterior no
                            // debe prestarle su estado a éste.
                            let id = SharedString::from(format!("doc-text-{node}-{}", docs.tab));
                            this.child(div().id("doc-body").test_support().child(TextView::markdown(id, body.clone())))
                        })
                        .children(decisions.into_iter().map(|decision| self.render_decision(decision, cx))),
                ),
            )
    }
}
