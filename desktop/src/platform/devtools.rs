//! DevTools (`app/src/pages/DevTools.tsx`): las herramientas que corren en
//! esta máquina, juntas bajo un techo y con su riel. Ninguna toca los datos
//! del producto. El cliente de peticiones todavía no está en esta app: su
//! alcance lo decide jose antes de construirlo.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::image_tool::ImageToolView;
use super::tokens::TokensView;
use crate::lang::tr;
use crate::theme::Leading as _;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Image,
    Tokens,
}

pub struct DevToolsView {
    tool: Tool,
    rail_open: bool,
    image: Option<Entity<ImageToolView>>,
    tokens: Option<Entity<TokensView>>,
}

impl DevToolsView {
    pub fn new() -> Self {
        DevToolsView { tool: Tool::Image, rail_open: true, image: None, tokens: None }
    }

    fn entry(&self, id: &'static str, icon: Lucide, label_key: &str, tool: Option<Tool>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, foreground, accent) = (theme.muted_foreground, theme.foreground, theme.accent);
        let label = tr(cx, label_key);
        let active = tool.is_some_and(|tool| tool == self.tool);
        let open = self.rail_open;
        let row = div()
            .id(id)
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded(px(4.))
            .when(!open, |row| row.justify_center())
            .when(active, |row| row.bg(accent).font_weight(FontWeight::MEDIUM).text_color(foreground))
            .when(!active, |row| row.text_color(muted).hover(move |style| style.bg(accent.opacity(0.5)).text_color(foreground)))
            .child(Icon::new(icon).size_4().flex_none())
            .when(open, |row| row.child(div().truncate().child(label.clone())));
        let row = match tool {
            Some(tool) => row.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                this.tool = tool;
                cx.notify();
            })),
            None => {
                let tip = format!("{label} · {}", tr(cx, "common:misc.notInThisApp"));
                row.opacity(0.6).tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            }
        };
        row.test_support().into_any_element()
    }
}

impl Render for DevToolsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted, border, panel, accent) = (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.muted, theme.accent);
        let open = self.rail_open;
        let body = match self.tool {
            Tool::Image => self.image.get_or_insert_with(|| cx.new(|cx| ImageToolView::new(window, cx))).clone().into_any_element(),
            Tool::Tokens => self.tokens.get_or_insert_with(|| cx.new(|cx| TokensView::new(window, cx))).clone().into_any_element(),
        };
        let toggle_tip = tr(cx, if open { "common:devtools.collapse" } else { "common:devtools.expand" });
        let rail = div()
            .id("devtools-rail")
            .w(px(if open { 200. } else { 46. }))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(border)
            .bg(panel.opacity(0.1))
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px_2()
                    .border_b_1()
                    .border_color(border)
                    .when(open, |head| head.child(div().px_1().text_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child(tr(cx, "common:misc.devtools"))))
                    .child(
                        div()
                            .id("devtools-rail-toggle")
                            .ml_auto()
                            .p_1()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_color(muted)
                            .hover(move |style| style.bg(accent).text_color(foreground))
                            .aria_label(toggle_tip.clone())
                            .tooltip(move |window, cx| Tooltip::new(toggle_tip.clone()).build(window, cx))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.rail_open = !this.rail_open;
                                cx.notify();
                            }))
                            .child(Icon::new(if open { Lucide::PanelLeftClose } else { Lucide::PanelLeftOpen }).size_4())
                            .test_support(),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p_1()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .leading_sm()
                    .child(self.entry("devtools-image", Lucide::ImageDown, "common:devtools.tools.image", Some(Tool::Image), cx))
                    .child(self.entry("devtools-requests", Lucide::Send, "common:devtools.tools.requests", None, cx))
                    .child(self.entry("devtools-tokens", Lucide::KeyRound, "common:devtools.tools.tokens", Some(Tool::Tokens), cx)),
            )
            .test_support();
        div().size_full().flex().bg(background).text_color(foreground).child(rail).child(div().flex_1().min_w_0().h_full().child(body))
    }
}
