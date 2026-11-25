//! El muro de feedback y comunicados (`pages/Feedback.tsx` y
//! `components/feedback/`, command-and-control-511).
//!
//! Qué se manda, quién puede publicar, qué dice cada línea («Solo Uno · hace
//! 1 hora · editada») y la cuenta optimista de las reacciones lo decide el
//! core (`cac_core::wall`); aquí se pinta con las medidas de la página de
//! siempre y se le pasa cada gesto. Lo propio de esta pantalla es lo que se
//! está escribiendo (publicar, editar, comentar) y qué hay abierto encima: el
//! diálogo de publicar, el de una publicación y la pregunta de borrar.

use cac_core::{NewPost, PostAudience, PostKind, ReactionSummary, WallComment, WallPost, REACTION_EMOJIS};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter, Input, InputEvent, InputState, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Selectable as _, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::editor::MarkdownEditor;
use crate::lang::{error_text, locale, tr};
use crate::theme::Leading as _;

/// El ancho del muro: el `max-w-2xl` de `Feedback.tsx`.
const PAGE_WIDTH: f32 = 672.;
/// Hasta dónde crece el texto de una tarjeta antes de cortarse: `max-h-56`.
const CARD_BODY_HEIGHT: f32 = 224.;

/// Lo que se está escribiendo en «Publicar».
struct Compose {
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    announce: bool,
    audience: PostAudience,
    busy: bool,
    problem: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// Lo que se está cambiando de una publicación abierta.
struct Edit {
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    audience: PostAudience,
    _subscriptions: Vec<Subscription>,
}

/// Qué se pregunta antes de borrar.
#[derive(Clone, PartialEq, Eq)]
enum Confirm {
    Post(String),
    Comment { post: String, comment: String },
}

/// De quién es el selector de reacciones abierto.
#[derive(Clone, PartialEq, Eq)]
enum Pick {
    Card(String),
    Post,
    Comment(String),
}

pub struct WallView {
    composing: Option<Compose>,
    editing: Option<Edit>,
    comment: Entity<TextareaState>,
    /// De qué publicación es lo escrito en `comment`.
    comment_for: Option<String>,
    /// El comentario salió: la caja se vacía al pintar, que es cuando hay
    /// ventana.
    clear_comment: bool,
    confirm: Option<Confirm>,
    picking: Option<Pick>,
    /// Por qué no salió lo último.
    problem: Option<String>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

/// Que lo escrito repinte: los botones se encienden al haber texto.
fn repaint_on_change<T: EventEmitter<InputEvent> + 'static>(field: &Entity<T>, cx: &mut Context<WallView>) -> Subscription {
    cx.subscribe(field, |_, _, event: &InputEvent, cx| {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    })
}

impl WallView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let comment = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 10));
        let on_change = repaint_on_change(&comment, cx);
        Self {
            composing: None,
            editing: None,
            comment,
            comment_for: None,
            clear_comment: false,
            confirm: None,
            picking: None,
            problem: None,
            saving: false,
            _subscriptions: vec![on_change],
        }
    }

    fn say(&mut self, error: &cac_core::ApiError, cx: &mut Context<Self>) {
        self.problem = Some(error_text(cx, error, "common:feedback.failed"));
        cx.notify();
    }

    /// Le pide algo al servidor y, si falla, dice por qué.
    fn run<F, Fut>(&mut self, work: F, cx: &mut Context<Self>)
    where
        F: FnOnce(cac_core::Core) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), cac_core::ApiError>> + Send + 'static,
    {
        self.problem = None;
        let answer = Backend::ask(cx, work);
        cx.spawn(async move |this, cx| {
            let Ok(Err(error)) = answer.await else { return };
            this.update(cx, |this, cx| this.say(&error, cx)).ok();
        })
        .detach();
    }

    fn open_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let permissions = Backend::core(cx).wall().permissions();
        if !permissions.can_publish() {
            return;
        }
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(tr(cx, "common:feedback.titlePlaceholder")));
        title.update(cx, |input, cx| input.focus(window, cx));
        let body = cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 18));
        let subscriptions = vec![repaint_on_change(&title, cx), repaint_on_change(&body, cx)];
        self.composing = Some(Compose {
            title,
            body,
            announce: !permissions.can_feedback && permissions.can_announce,
            audience: PostAudience::Platform,
            busy: false,
            problem: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn new_post(compose: &Compose, cx: &App) -> NewPost {
        NewPost { announce: compose.announce, audience: compose.audience, title: compose.title.read(cx).value().to_string(), body: compose.body.read(cx).value().to_string() }
    }

    /// Si lo escrito se puede publicar: lo dice el core.
    fn ready(compose: &Compose, cx: &App) -> bool {
        !compose.busy && cac_core::post_payload(&Self::new_post(compose, cx), &Backend::core(cx).wall().permissions()).is_some()
    }

    fn submit_composer(&mut self, cx: &mut Context<Self>) {
        let Some(compose) = self.composing.as_ref().filter(|compose| Self::ready(compose, cx)) else { return };
        let new = Self::new_post(compose, cx);
        if let Some(compose) = self.composing.as_mut() {
            compose.busy = true;
            compose.problem = None;
        }
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move { core.wall().create(new).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.composing = None,
                    Err(error) => {
                        let text = error_text(cx, &error, "common:feedback.failed");
                        if let Some(compose) = this.composing.as_mut() {
                            compose.busy = false;
                            compose.problem = Some(text);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_editing(&mut self, post: &WallPost, window: &mut Window, cx: &mut Context<Self>) {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(tr(cx, "common:feedback.titlePlaceholder")));
        title.update(cx, |input, cx| input.set_value(post.title.clone(), window, cx));
        let body = cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 18));
        body.update(cx, |input, cx| input.set_value(post.body.clone(), window, cx));
        let subscriptions = vec![repaint_on_change(&body, cx)];
        self.editing = Some(Edit { title, body, audience: post.audience, _subscriptions: subscriptions });
        cx.notify();
    }

    fn save_edit(&mut self, post_id: String, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.as_ref().filter(|_| !self.saving) else { return };
        let (title, body, audience) = (edit.title.read(cx).value().to_string(), edit.body.read(cx).value().to_string(), edit.audience);
        if body.trim().is_empty() {
            return;
        }
        self.saving = true;
        self.problem = None;
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move { core.wall().update(&post_id, &title, &body, audience).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(()) => this.editing = None,
                    Err(error) => this.say(&error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn send_comment(&mut self, post_id: String, cx: &mut Context<Self>) {
        let body = self.comment.read(cx).value().to_string();
        if body.trim().is_empty() || self.saving {
            return;
        }
        self.saving = true;
        self.problem = None;
        cx.notify();
        let answer = Backend::ask(cx, move |core| async move { core.wall().comment(&post_id, &body).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(()) => this.clear_comment = true,
                    Err(error) => this.say(&error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Lo que se contestó a la pregunta de borrar.
    fn confirmed(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.confirm.take() else { return };
        match confirm {
            Confirm::Post(id) => self.run(move |core| async move { core.wall().remove(&id).await }, cx),
            Confirm::Comment { post, comment } => self.run(move |core| async move { core.wall().remove_comment(&post, &comment).await }, cx),
        }
        cx.notify();
    }
}

impl WallView {
    fn avatar(post: &WallPost, size: Pixels, cx: &App) -> Div {
        let theme = cx.theme();
        div()
            .size(size)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme.muted)
            .text_size(px(11.))
            .font_weight(FontWeight::BOLD)
            .map(|this| {
                if post.kind == PostKind::Announcement {
                    this.child(Icon::new(Lucide::Megaphone).size_3p5())
                } else {
                    this.child(cac_core::username_initials(&post.author_name))
                }
            })
    }

    /// Una etiqueta (`Badge`): llena, apagada o con borde.
    fn badge(id: Option<String>, text: String, tone: BadgeTone, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let base = div().flex_none().rounded_full().px(px(8.)).py(px(1.)).text_size(px(11.)).line_height(px(16.)).font_weight(FontWeight::MEDIUM);
        let base = match tone {
            BadgeTone::Solid => base.bg(theme.primary).text_color(theme.primary_foreground),
            BadgeTone::Soft => base.bg(theme.secondary).text_color(theme.secondary_foreground),
            BadgeTone::Outline => base.border_1().border_color(theme.border).text_color(theme.foreground),
            BadgeTone::Warning => base.border_1().border_color(theme.warning.opacity(0.5)).text_color(theme.warning),
        };
        match id {
            Some(id) => base.id(SharedString::from(id)).child(text).test_support().into_any_element(),
            None => base.child(text).into_any_element(),
        }
    }

    fn audience_badge(post: &WallPost, cx: &App) -> AnyElement {
        let label = cac_core::audience_label(post, locale(cx));
        let tone = if post.kind == PostKind::Announcement { BadgeTone::Soft } else { BadgeTone::Outline };
        Self::badge(None, label, tone, cx)
    }

    /// Markdown de una publicación para leer.
    fn prose(id: String, text: &str, cx: &App) -> Div {
        crate::theme::prose_with_links(
            id,
            text.to_string(),
            |url, _, cx| {
                if crate::theme::leads_somewhere(url) {
                    cx.open_url(url);
                }
            },
            cx,
        )
    }

    /// Las reacciones de algo (`Reactions.tsx`): una pastilla por emoji con
    /// cuántos y, al pasar el ratón, quiénes; la propia, marcada. Y el botón
    /// de elegir otra.
    fn reactions(
        prefix: String,
        reactions: &[ReactionSummary],
        pick: Pick,
        on_react: impl Fn(&str, bool, &mut App) + Clone + 'static,
        picking: &Option<Pick>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (foreground, muted_ink, muted, border, primary) = (theme.foreground, theme.muted_foreground, theme.muted, theme.border, theme.primary);
        let language = locale(cx);
        let pills = reactions.iter().enumerate().map(|(index, reaction)| {
            let (emoji, on) = (reaction.emoji.clone(), !reaction.mine);
            let label = cac_core::text_with(language, "chat:reactionCount", &[("emoji", &reaction.emoji), ("count", &reaction.count.to_string())]);
            let names = reaction.names.join(", ");
            let on_react = on_react.clone();
            div()
                .id(SharedString::from(format!("{prefix}-reaction-{index}")))
                .h_6()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .rounded_full()
                .border_1()
                .px_2()
                .leading_xs()
                .cursor_pointer()
                .map(|this| {
                    if reaction.mine {
                        this.border_color(primary.opacity(0.5)).bg(primary.opacity(0.1)).text_color(foreground)
                    } else {
                        this.border_color(border).bg(muted.opacity(0.4)).text_color(muted_ink)
                    }
                })
                .when(!names.is_empty(), |this| this.tooltip(move |window, cx| Tooltip::new(names.clone()).build(window, cx)))
                .on_click(move |_, _, cx| on_react(&emoji, on, cx))
                .aria_label(label)
                .aria_selected(reaction.mine)
                .child(crate::assets::emoji(&reaction.emoji, px(16.)))
                .child(reaction.count.to_string())
                .test_support()
        });
        let open = picking.as_ref() == Some(&pick);
        let toggle_to = pick.clone();
        let add = div()
            .id(SharedString::from(format!("{prefix}-react")))
            .h_6()
            .w_7()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(border)
            .text_color(muted_ink)
            .cursor_pointer()
            .hover(move |style| style.text_color(foreground))
            .aria_label(tr(cx, "chat:react"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.picking = if this.picking.as_ref() == Some(&toggle_to) { None } else { Some(toggle_to.clone()) };
                cx.notify();
            }))
            .child(Icon::new(Lucide::Plus).size_3p5())
            .test_support();
        let picker = open.then(|| {
            let popover = cx.theme().popover;
            div()
                .id(SharedString::from(format!("{prefix}-picker")))
                .flex()
                .gap_0p5()
                .p_1()
                .rounded_full()
                .border_1()
                .border_color(border)
                .bg(popover)
                .shadow_md()
                .children(REACTION_EMOJIS.iter().enumerate().map(|(index, emoji)| {
                    let mine = reactions.iter().any(|reaction| reaction.emoji == *emoji && reaction.mine);
                    let (emoji, on_react) = (emoji.to_string(), on_react.clone());
                    Button::new(SharedString::from(format!("{prefix}-pick-{index}")))
                        .ghost()
                        .small()
                        .child(crate::assets::emoji(&emoji, px(20.)))
                        .selected(mine)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.picking = None;
                            on_react(&emoji, !mine, cx);
                            cx.notify();
                        }))
                }))
                .test_support()
        });
        div().flex().flex_col().gap_1().child(div().flex().flex_wrap().items_center().gap_1().children(pills).child(add)).children(picker).into_any_element()
    }

    /// Una publicación en el muro (`PostCard.tsx`).
    fn card(&mut self, post: &WallPost, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, primary, warning) = (theme.border, theme.muted_foreground, theme.primary, theme.warning);
        let language = locale(cx);
        let announcement = post.kind == PostKind::Announcement;
        let id = post.id.clone();
        let meta = cac_core::post_meta(post, &chrono::Utc::now(), language);
        let open = {
            let id = id.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let id = id.clone();
                Backend::run(cx, move |core| async move {
                    let _ = core.wall().open_post(&id).await;
                });
            }
        };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(Self::avatar(post, px(28.), cx))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(div().truncate().leading_sm().font_weight(FontWeight::MEDIUM).child(post.author_name.clone()))
                    .child(div().truncate().leading_xs().text_color(muted).child(meta)),
            )
            .when(post.pinned, |this| {
                let label = tr(cx, "common:feedback.pinned");
                this.child(div().id(SharedString::from(format!("wall-post-{id}-pinned"))).text_color(primary).aria_label(label).child(Icon::new(Lucide::Pin).size_3p5()).test_support())
            })
            .when(post.draft, |this| this.child(Self::badge(Some(format!("wall-post-{id}-draft")), tr(cx, "common:feedback.draft"), BadgeTone::Warning, cx)))
            .when(post.unseen, |this| this.child(Self::badge(Some(format!("wall-post-{id}-new")), tr(cx, "common:feedback.new"), BadgeTone::Solid, cx)))
            .child(if announcement { Self::badge(None, tr(cx, "common:feedback.announcement"), BadgeTone::Soft, cx) } else { Self::audience_badge(post, cx) });
        let _ = warning;
        let reactions = {
            let id = id.clone();
            Self::reactions(
                format!("wall-post-{id}"),
                &post.reactions,
                Pick::Card(id.clone()),
                move |emoji: &str, on, cx: &mut App| {
                    let (id, emoji) = (id.clone(), emoji.to_string());
                    Backend::run(cx, move |core| async move {
                        let _ = core.wall().react(&id, &emoji, on).await;
                    });
                },
                &self.picking,
                cx,
            )
        };
        div()
            .id(SharedString::from(format!("wall-post-{id}")))
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded(px(12.))
            .border_1()
            .border_color(if announcement { primary.opacity(0.4) } else { border })
            .bg(if announcement { primary.opacity(0.05) } else { cx.theme().group_box })
            .aria_label(if post.title.is_empty() { post.author_name.clone() } else { post.title.clone() })
            .child(
                div()
                    .id(SharedString::from(format!("wall-post-{id}-open")))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .cursor_pointer()
                    .on_click(open.clone())
                    .child(header)
                    .when(!post.title.is_empty(), |this| this.child(div().leading_sm().font_weight(FontWeight::SEMIBOLD).child(post.title.clone())))
                    // Recortado por alto; el entero, en el diálogo.
                    .child(div().max_h(px(CARD_BODY_HEIGHT)).overflow_hidden().child(Self::prose(format!("wall-post-{id}-body"), &post.body, cx)))
                    .test_support(),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(div().min_w_0().flex_1().child(reactions))
                    .child(
                        div()
                            .id(SharedString::from(format!("wall-post-{id}-comments")))
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .leading_xs()
                            .text_color(muted)
                            .cursor_pointer()
                            .hover(move |style| style.text_color(primary))
                            .on_click(open)
                            .aria_label(cac_core::comments_label(post.comments, language))
                            .child(Icon::new(Lucide::MessageSquare).size_3p5())
                            .child(cac_core::comments_label(post.comments, language))
                            .test_support(),
                    ),
            )
            .test_support()
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum BadgeTone {
    Solid,
    Soft,
    Outline,
    Warning,
}

/// Un diálogo de `Dialog`: la cortina que tapa lo de debajo, y la caja
/// centrada. Un clic fuera lo cierra; uno dentro, no.
fn dialog(id: &'static str, width: f32, on_close: impl Fn(&mut Window, &mut App) + Clone + 'static, content: Stateful<Div>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (background, foreground) = (theme.background, theme.foreground);
    let (outside, escape) = (on_close.clone(), on_close);
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .bg(black().opacity(0.35))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                escape(window, cx);
                cx.stop_propagation();
            }
        })
        .on_mouse_down(MouseButton::Left, move |_, window, cx| outside(window, cx))
        .child(
            content
                .test_support()
                .relative()
                .w(px(width))
                .max_w_full()
                .max_h(relative(0.9))
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .rounded(px(12.))
                .border_1()
                .border_color(foreground.opacity(0.1))
                .bg(background)
                .text_color(foreground)
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
        )
        .test_support()
        .into_any_element()
}

impl WallView {
    /// Una opción de «Quién lo ve» (`role="radio"`).
    fn audience_option(id: String, label: String, hint: Option<String>, chosen: bool, on_pick: impl Fn(&mut App) + 'static, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (primary, border, muted, hover) = (theme.primary, theme.border, theme.muted_foreground, theme.muted);
        div()
            .id(SharedString::from(id))
            .flex_1()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .px_3()
            .py_2()
            .cursor_pointer()
            .map(|this| if chosen { this.border_color(primary).bg(primary.opacity(0.1)) } else { this.border_color(border).hover(move |style| style.bg(hover)) })
            .aria_label(label.clone())
            .aria_selected(chosen)
            .on_click(move |_, _, cx| on_pick(cx))
            .child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(label))
            .children(hint.map(|hint| div().leading_xs().text_color(muted).child(hint)))
            .test_support()
            .into_any_element()
    }

    /// El diálogo de «Publicar» (`PostComposer.tsx`).
    fn render_composer(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let compose = self.composing.as_ref()?;
        let permissions = Backend::core(cx).wall().permissions();
        let (theme_border, muted, danger, primary) = {
            let theme = cx.theme();
            (theme.border, theme.muted_foreground, theme.danger, theme.primary)
        };
        let as_announcement = compose.announce || !permissions.can_feedback;
        let ready = Self::ready(compose, cx);
        let (busy, announce, audience) = (compose.busy, compose.announce, compose.audience);
        let org_name = permissions.org_name.clone();
        let option = |value: PostAudience, label: String, hint: String, cx: &mut Context<Self>| {
            let id = format!("post-audience-{}", value.as_str());
            Self::audience_option(id, label, Some(hint), audience == value, {
                let view = cx.entity().downgrade();
                move |cx| {
                    let _ = view.update(cx, |this, cx| {
                        if let Some(compose) = this.composing.as_mut() {
                            compose.audience = value;
                        }
                        cx.notify();
                    });
                }
            }, cx)
        };
        let audiences = (!as_announcement).then(|| {
            let (platform_label, platform_hint) = match org_name.as_deref() {
                Some(org) => (
                    cac_core::text_with(locale(cx), "common:feedback.audienceOrg", &[("org", org)]),
                    cac_core::text_with(locale(cx), "common:feedback.audienceOrgHint", &[("org", org)]),
                ),
                None => (tr(cx, "common:feedback.audiencePlatform"), tr(cx, "common:feedback.audiencePlatformHint")),
            };
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:feedback.audience")))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(option(PostAudience::Platform, platform_label, platform_hint, cx))
                        .child(option(PostAudience::Everyone, tr(cx, "common:feedback.audienceEveryone"), tr(cx, "common:feedback.audienceEveryoneHint"), cx)),
                )
        });
        let toggle = (permissions.can_announce && permissions.can_feedback).then(|| {
            div()
                .id("post-announce")
                .flex()
                .items_start()
                .gap_2()
                .cursor_pointer()
                .aria_label(tr(cx, "common:feedback.asAnnouncement"))
                .aria_selected(announce)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(compose) = this.composing.as_mut() {
                        compose.announce = !compose.announce;
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .mt(px(3.))
                        .size_4()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(if announce { primary } else { theme_border })
                        .when(announce, |this| this.bg(primary).text_color(cx.theme().primary_foreground).child(Icon::new(Lucide::Check).size_3()))
                )
                .child(
                    div()
                        .child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, "common:feedback.asAnnouncement")))
                        .child(div().leading_xs().text_color(muted).child(tr(cx, "common:feedback.asAnnouncementHint"))),
                )
                .test_support()
        });
        let content = div()
            .id("post-composer")
            .child(div().text_size(px(16.)).line_height(px(16.)).font_weight(FontWeight::MEDIUM).child(tr(cx, "common:feedback.publish")))
            .child(
                div().absolute().top_2().right_2().child(
                    Button::new("post-composer-close")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::X))
                        .tooltip(tr(cx, "common:action.close"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.composing = None;
                            cx.notify();
                        })),
                ),
            )
            .child(Input::new(&compose.title).id("post-title"))
            .child(div().id("post-body-scroll").min_h_0().overflow_y_scroll().child(MarkdownEditor::new("post-body", &compose.body).placeholder(tr(cx, "common:feedback.bodyPlaceholder"))))
            .children(toggle)
            .children(audiences)
            .children(compose.problem.clone().map(|problem| div().id("post-problem").leading_xs().text_color(danger).aria_label(problem.clone()).child(problem).test_support()))
            .child(
                div().flex().justify_end().child(
                    Button::new("post-submit")
                        .primary()
                        .label(tr(cx, if busy { "common:feedback.publishing" } else { "common:feedback.publish" }))
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, _, cx| this.submit_composer(cx))),
                ),
            );
        let view = cx.entity().downgrade();
        Some(dialog(
            "post-composer-backdrop",
            PAGE_WIDTH,
            move |_, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.composing = None;
                    cx.notify();
                });
            },
            content,
            cx,
        ))
    }

    /// La pregunta de borrar.
    fn render_confirm(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let confirm = self.confirm.clone()?;
        let muted = cx.theme().muted_foreground;
        let (title, body) = match confirm {
            Confirm::Post(_) => (tr(cx, "common:feedback.deleteTitle"), Some(tr(cx, "common:feedback.deleteBody"))),
            Confirm::Comment { .. } => (tr(cx, "common:feedback.deleteCommentTitle"), None),
        };
        let view = cx.entity().downgrade();
        let content = div()
            .id("wall-confirm")
            .child(div().text_size(px(16.)).line_height(px(16.)).font_weight(FontWeight::MEDIUM).child(title))
            .children(body.map(|body| div().leading_sm().text_color(muted).child(body)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("wall-confirm-cancel").ghost().label(tr(cx, "common:feedback.cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.confirm = None;
                        cx.notify();
                    })))
                    .child(Button::new("wall-confirm-delete").danger().label(tr(cx, "common:feedback.delete")).on_click(cx.listener(|this, _, _, cx| this.confirmed(cx)))),
            );
        Some(dialog(
            "wall-confirm-backdrop",
            384.,
            move |_, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.confirm = None;
                    cx.notify();
                });
            },
            content,
            cx,
        ))
    }

    /// Una publicación abierta, entera, con sus reacciones y sus comentarios
    /// (`PostModal.tsx`).
    fn render_post(&mut self, post: &WallPost, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let _ = window;
        let (muted, danger, warning, border) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.danger, theme.warning, theme.border)
        };
        let language = locale(cx);
        let announcement = post.kind == PostKind::Announcement;
        let id = post.id.clone();
        let editing = self.editing.is_some();
        let now = chrono::Utc::now();

        let icon_button = |name: &'static str, icon: Lucide, label: String, selected: bool| {
            Button::new(name).ghost().small().icon(Icon::new(icon)).selected(selected).tooltip(label)
        };
        let actions = div()
            .flex()
            .flex_none()
            .items_center()
            .when(post.can_pin, |this| {
                let id = id.clone();
                let pinned = post.pinned;
                this.child(icon_button("wall-pin", Lucide::Pin, tr(cx, if pinned { "common:feedback.unpin" } else { "common:feedback.pin" }), pinned).on_click(cx.listener(
                    move |this, _, _, cx| {
                        let id = id.clone();
                        this.run(move |core| async move { core.wall().pin(&id, !pinned).await }, cx);
                    },
                )))
            })
            .when(post.can_edit && !editing, |this| {
                let post = post.clone();
                this.child(icon_button("wall-edit", Lucide::Pencil, tr(cx, "common:feedback.edit"), false).on_click(cx.listener(move |this, _, window, cx| this.start_editing(&post, window, cx))))
            })
            .when(post.can_delete, |this| {
                let id = id.clone();
                this.child(icon_button("wall-delete", Lucide::Trash, tr(cx, "common:feedback.delete"), false).on_click(cx.listener(move |this, _, _, cx| {
                    this.confirm = Some(Confirm::Post(id.clone()));
                    cx.notify();
                })))
            })
            .child(
                Button::new("wall-close")
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::X))
                    .tooltip(tr(cx, "common:action.close"))
                    .on_click(|_, _, cx| Backend::core(cx).wall().close_post()),
            );
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(Self::avatar(post, px(32.), cx))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(div().id("wall-post-title").truncate().text_size(px(16.)).line_height(px(20.)).font_weight(FontWeight::MEDIUM).child(if post.title.is_empty() { post.author_name.clone() } else { post.title.clone() }).test_support())
                    .child(div().truncate().leading_sm().text_color(muted).child(cac_core::post_detail_meta(post, &now, language))),
            )
            .child(if announcement { Self::badge(None, tr(cx, "common:feedback.announcement"), BadgeTone::Soft, cx) } else { Self::audience_badge(post, cx) })
            .child(actions);

        // Un borrador que redactó un agente: lo revisa y lo publica su autor,
        // en persona (el backend no deja publicarlo con un token).
        let draft = (post.can_publish && !editing).then(|| {
            let id = id.clone();
            div()
                .id("wall-draft")
                .flex()
                .items_center()
                .gap_3()
                .rounded(px(8.))
                .border_1()
                .border_color(warning.opacity(0.4))
                .bg(warning.opacity(0.1))
                .px_3()
                .py_2()
                .child(div().min_w_0().flex_1().leading_sm().child(tr(cx, "common:feedback.publishDraftHint")))
                .child(Button::new("wall-publish-draft").primary().small().label(tr(cx, "common:feedback.publishDraft")).on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.run(move |core| async move { core.wall().publish(&id).await }, cx);
                })))
                .test_support()
        });

        let body: AnyElement = match self.editing.as_ref() {
            Some(edit) => {
                let (chosen, org) = (edit.audience, post.org_name.clone());
                let option = |value: PostAudience, cx: &mut Context<Self>| {
                    let label = cac_core::audience_text(PostKind::Feedback, value, org.as_deref(), locale(cx));
                    let view = cx.entity().downgrade();
                    Self::audience_option(format!("wall-edit-audience-{}", value.as_str()), label, None, chosen == value, move |cx| {
                        let _ = view.update(cx, |this, cx| {
                            if let Some(edit) = this.editing.as_mut() {
                                edit.audience = value;
                            }
                            cx.notify();
                        });
                    }, cx)
                };
                let (id_save, ready) = (id.clone(), !edit.body.read(cx).value().trim().is_empty() && !self.saving);
                div()
                    .id("wall-edit-form")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Input::new(&edit.title).id("wall-edit-title"))
                    .child(MarkdownEditor::new("wall-edit-body", &edit.body).placeholder(tr(cx, "common:feedback.bodyPlaceholder")))
                    .when(!announcement, |this| this.child(div().flex().gap_2().child(option(PostAudience::Platform, cx)).child(option(PostAudience::Everyone, cx))))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(Button::new("wall-edit-cancel").ghost().label(tr(cx, "common:feedback.cancel")).on_click(cx.listener(|this, _, _, cx| {
                                this.editing = None;
                                cx.notify();
                            })))
                            .child(
                                Button::new("wall-edit-save")
                                    .primary()
                                    .label(tr(cx, if self.saving { "common:feedback.saving" } else { "common:feedback.save" }))
                                    .disabled(!ready)
                                    .on_click(cx.listener(move |this, _, _, cx| this.save_edit(id_save.clone(), cx))),
                            ),
                    )
                    .into_any_element()
            }
            None => div().id("wall-post-body").child(Self::prose(format!("wall-open-{id}-body"), &post.body, cx)).test_support().into_any_element(),
        };

        let reactions = {
            let id = id.clone();
            Self::reactions(
                "wall-open".to_string(),
                &post.reactions,
                Pick::Post,
                move |emoji: &str, on, cx: &mut App| {
                    let (id, emoji) = (id.clone(), emoji.to_string());
                    Backend::run(cx, move |core| async move {
                        let _ = core.wall().react(&id, &emoji, on).await;
                    });
                },
                &self.picking,
                cx,
            )
        };

        let comments: Vec<AnyElement> = post.comment_list.iter().map(|comment| self.render_comment(&id, comment, &now, cx)).collect();
        let none = comments.is_empty();
        let thread = div()
            .id("wall-comments")
            .flex()
            .flex_col()
            .gap_3()
            .border_t_1()
            .border_color(border)
            .pt_3()
            .when(none, |this| this.child(div().id("wall-no-comments").leading_xs().text_color(muted).child(tr(cx, "common:feedback.noComments")).test_support()))
            .children(comments)
            .test_support();

        let sending = self.saving;
        let can_send = !self.comment.read(cx).value().trim().is_empty() && !sending;
        let id_send = id.clone();
        let id_enter = id.clone();
        let composer = div()
            .flex_none()
            .border_t_1()
            .border_color(border)
            .pt_3()
            .flex()
            .flex_col()
            .gap_2()
            // Enter comenta; Mayús+Enter salta de línea.
            .capture_action(cx.listener(move |this, enter: &Enter, _, cx| {
                if !enter.shift && !enter.secondary {
                    cx.stop_propagation();
                    this.send_comment(id_enter.clone(), cx);
                }
            }))
            .child(MarkdownEditor::new("wall-comment", &self.comment).placeholder(tr(cx, "common:feedback.commentPlaceholder")))
            .child(
                div().flex().justify_end().child(
                    Button::new("wall-comment-send")
                        .primary()
                        .small()
                        .icon(Icon::new(Lucide::Send))
                        .disabled(!can_send)
                        .tooltip(tr(cx, "chat:send"))
                        .on_click(cx.listener(move |this, _, _, cx| this.send_comment(id_send.clone(), cx))),
                ),
            );

        let content = div()
            .id("wall-modal")
            .child(header)
            .children(draft)
            .child(div().id("wall-modal-scroll").min_h_0().flex_1().overflow_y_scroll().flex().flex_col().gap_4().pr_1().child(body).child(reactions).child(thread))
            .children(self.problem.clone().map(|problem| div().id("wall-modal-problem").leading_xs().text_color(danger).aria_label(problem.clone()).child(problem).test_support()))
            .child(composer);
        dialog(
            "wall-modal-backdrop",
            PAGE_WIDTH,
            |_, cx| Backend::core(cx).wall().close_post(),
            content,
            cx,
        )
    }

    fn render_comment(&mut self, post_id: &str, comment: &WallComment, now: &chrono::DateTime<chrono::Utc>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted_bg, muted) = (theme.muted, theme.muted_foreground);
        let (post, cid) = (post_id.to_string(), comment.id.clone());
        let reactions = {
            let (post, cid) = (post.clone(), cid.clone());
            Self::reactions(
                format!("wall-comment-{cid}"),
                &comment.reactions,
                Pick::Comment(cid.clone()),
                move |emoji: &str, on, cx: &mut App| {
                    let (post, cid, emoji) = (post.clone(), cid.clone(), emoji.to_string());
                    Backend::run(cx, move |core| async move {
                        let _ = core.wall().react_comment(&post, &cid, &emoji, on).await;
                    });
                },
                &self.picking,
                cx,
            )
        };
        let language = locale(cx);
        div()
            .id(SharedString::from(format!("wall-comment-{cid}")))
            .flex()
            .gap_2()
            .child(
                div()
                    .size_7()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(muted_bg)
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .child(cac_core::username_initials(&comment.author_name)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .leading_xs()
                            .text_color(muted)
                            .child(div().font_weight(FontWeight::SEMIBOLD).text_color(cx.theme().foreground).child(comment.author_name.clone()))
                            .child(cac_core::comment_meta(comment, now, language))
                            .when(comment.can_delete, |this| {
                                this.child(
                                    div()
                                        .id(SharedString::from(format!("wall-comment-{cid}-delete")))
                                        .ml_auto()
                                        .cursor_pointer()
                                        .hover(|style| style.text_color(gpui_kit::red()))
                                        .aria_label(tr(cx, "common:feedback.delete"))
                                        .on_click(cx.listener({
                                            let (post, cid) = (post.clone(), cid.clone());
                                            move |this, _, _, cx| {
                                                this.confirm = Some(Confirm::Comment { post: post.clone(), comment: cid.clone() });
                                                cx.notify();
                                            }
                                        }))
                                        .child(Icon::new(Lucide::Trash).size_3p5())
                                        .test_support(),
                                )
                            }),
                    )
                    .child(Self::prose(format!("wall-comment-{cid}-body"), &comment.body, cx))
                    .child(reactions),
            )
            .test_support()
            .into_any_element()
    }
}

impl Render for WallView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).wall.clone();
        let permissions = Backend::core(cx).wall().permissions();
        let (background, foreground, muted, border, danger) = {
            let theme = cx.theme();
            (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.danger)
        };

        // Lo escrito en la caja de comentar y lo que se estaba editando son de
        // la publicación que estaba abierta.
        let open_id = state.open.as_ref().map(|post| post.id.clone());
        if self.comment_for != open_id {
            self.comment_for = open_id;
            self.editing = None;
            self.confirm = None;
            self.picking = None;
            self.clear_comment = true;
        }
        if self.clear_comment {
            self.clear_comment = false;
            self.comment.update(cx, |field, cx| field.set_value("", window, cx));
        }

        let header = div()
            .flex()
            .items_start()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(div().id("wall-title").text_size(px(20.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).child(tr(cx, "common:feedback.title")).test_support())
                    .child(div().leading_sm().text_color(muted).child(tr(cx, "common:feedback.lead"))),
            )
            .when(permissions.can_publish(), |this| {
                this.child(
                    Button::new("wall-publish")
                        .primary()
                        .icon(Icon::new(Lucide::Plus))
                        .label(tr(cx, "common:feedback.publish"))
                        .on_click(cx.listener(|this, _, window, cx| this.open_composer(window, cx))),
                )
            });

        let status: AnyElement = if !state.loaded {
            div()
                .id("wall-loading")
                .flex()
                .flex_col()
                .gap_3()
                .children((0..3).map(|_| div().h(px(160.)).w_full().rounded(px(12.)).bg(cx.theme().muted.opacity(0.5))))
                .test_support()
                .into_any_element()
        } else if state.posts.is_empty() {
            div()
                .id("wall-empty")
                .rounded(px(12.))
                .border_1()
                .border_dashed()
                .border_color(border)
                .px_3()
                .py(px(40.))
                .text_center()
                .leading_sm()
                .text_color(muted)
                .child(tr(cx, "common:feedback.empty"))
                .test_support()
                .into_any_element()
        } else {
            let cards: Vec<AnyElement> = state.posts.iter().map(|post| self.card(post, cx)).collect();
            div()
                .flex()
                .flex_col()
                .gap_3()
                .children(cards)
                .when(state.next_cursor.is_some(), |this| {
                    this.child(
                        Button::new("wall-load-more")
                            .outline()
                            .w_full()
                            .disabled(state.loading)
                            .label(tr(cx, "common:feedback.loadMore"))
                            .on_click(|_, _, cx| {
                                Backend::run(cx, |core| async move { core.wall().load_more().await });
                            }),
                    )
                })
                .into_any_element()
        };

        let page = div()
            .id("wall-page")
            .w_full()
            .max_w(px(PAGE_WIDTH))
            .mx_auto()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .child(header)
            .when(!permissions.can_publish() && permissions.org_id.is_some(), |this| {
                this.child(div().id("wall-read-only").leading_xs().text_color(muted).child(tr(cx, "common:feedback.readOnlyHint")).test_support())
            })
            .when(state.failed && !state.loaded, |this| this.child(div().id("wall-error").leading_sm().text_color(danger).child(tr(cx, "common:feedback.failed")).test_support()))
            .children(self.problem.clone().filter(|_| state.open.is_none() && self.composing.is_none()).map(|problem| {
                div().id("wall-problem").leading_sm().text_color(danger).aria_label(problem.clone()).child(problem).test_support()
            }))
            .child(status);

        let modal = state.open.as_ref().map(|post| self.render_post(post, window, cx));
        let composer = self.render_composer(cx);
        let confirm = self.render_confirm(cx);
        div()
            .id("wall")
            .test_support()
            .relative()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .line_height(relative(1.5))
            .child(div().id("wall-scroll").size_full().overflow_y_scroll().child(page))
            .children(modal)
            .children(composer)
            .children(confirm)
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path};
    use wiremock::Mock;

    fn get(route: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET")).and(path(route.to_string()))
    }

    fn post(route: &str) -> wiremock::MockBuilder {
        Mock::given(method("POST")).and(path(route.to_string()))
    }

    fn card(id: &str, over: Value) -> Value {
        let mut card = json!({
            "id": id, "kind": "feedback", "audience": "platform", "orgId": "o1", "orgName": "G Studio", "authorId": "u9", "authorName": "Bea Soto",
            "title": format!("t-{id}"), "body": "cuerpo", "createdAt": "2026-11-25T10:00:00Z", "comments": 0, "reactions": [],
            "unseen": false, "canDelete": false, "canEdit": false, "canPin": false, "pinned": false, "draft": false, "canPublish": false,
        });
        for (key, value) in over.as_object().cloned().unwrap_or_default() {
            card[key] = value;
        }
        card
    }

    fn detail(id: &str, over: Value, comments: Value) -> Value {
        let mut detail = card(id, over);
        detail["commentList"] = comments;
        detail
    }

    /// `o1` con ese papel, su sala general, y el muro con esas tarjetas.
    fn server(role: &str, posts: Value) -> FakeServer {
        let server = FakeServer::start();
        let org = json!({ "id": "o1", "name": "G Studio", "slug": "o1", "role": role, "memberCount": 3, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org]))));
        let general = json!({ "id": "g", "orgId": "o1", "name": "General", "color": "#888", "kind": "general", "folders": [], "lists": [], "people": [] });
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([general]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/unread").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/posts").respond_with(ok(json!({ "posts": posts }))));
        server.mount(post("/api/v1/posts/seen").respond_with(ok(json!(null))));
        server
    }

    fn open(cx: &mut TestAppContext) -> AnyWindowHandle {
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(900.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window").0
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

    fn type_in(window: AnyWindowHandle, field: &str, text: &str, cx: &mut TestAppContext) {
        let (field, text) = (gpui_kit::SharedString::from(field.to_string()), text.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(field, cx);
            window.input(&text, cx);
        })
        .unwrap();
    }

    fn sent(server: &FakeServer, verb: &str, route: &str) -> Vec<Value> {
        server.requests().iter().filter(|r| r.method.as_str() == verb && r.url.path() == route).map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null)).collect()
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Entra a comunicación y pulsa el muro.
    fn open_wall(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "rail-comms", cx) && cx.update(|cx| Backend::snapshot(cx).read(cx).wall.loaded));
        click(window, "rail-comms", cx);
        wait_until(cx, "la lista de comunicación", |cx| on_screen(window, "comms-wall", cx));
        click(window, "comms-wall", cx);
        wait_until(cx, "el muro está delante", |cx| on_screen(window, "wall", cx));
    }

    /// La fila del muro lo pone delante de la conversación, con sus
    /// publicaciones y su punto de «nuevo», que se apaga al abrirlo; y elegir
    /// una conversación lo quita.
    ///
    /// Roturas que atrapa: una fila que no abre nada; el muro y el canal
    /// marcados a la vez; el punto que no se enciende con lo no visto o que
    /// sigue tras abrir; no avisar al servidor de que se vio; la conversación
    /// que no vuelve al elegirla.
    #[gpui_kit::test]
    fn the_wall_row_opens_the_wall_and_a_channel_takes_it_away(cx: &mut TestAppContext) {
        let server = server("member", json!([card("a", json!({ "unseen": true })), card("b", json!({ "kind": "announcement", "pinned": true }))]));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "llega el muro", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).wall.loaded));
        click(window, "rail-comms", cx);
        wait_until(cx, "la lista de comunicación", |cx| on_screen(window, "comms-wall-new", cx));
        assert!(on_screen(window, "conversation", cx) && !on_screen(window, "wall", cx));

        click(window, "comms-wall", cx);
        wait_until(cx, "el muro está delante", |cx| on_screen(window, "wall-post-a", cx));
        assert!(!on_screen(window, "conversation", cx) && !on_screen(window, "comms-wall-new", cx));
        assert!(on_screen(window, "wall-post-a-new", cx) && on_screen(window, "wall-post-b-pinned", cx), "las tarjetas siguen diciendo qué es nuevo");
        assert_eq!(label(window, "wall-post-b", cx), "t-b");
        assert!(!on_screen(window, "wall-read-only", cx));
        wait_until(cx, "se avisó de que se vio", |_| !sent(&server, "POST", "/api/v1/posts/seen").is_empty());

        click(window, "comms-channel-g", cx);
        wait_until(cx, "vuelve la conversación", |cx| on_screen(window, "conversation", cx) && !on_screen(window, "wall", cx));
        stop(cx);
    }

    /// Publicar manda lo escrito con la organización y la audiencia elegidas,
    /// no deja publicar en blanco, y un viewer sólo lee.
    ///
    /// Roturas que atrapa: el botón de enviar encendido sin texto; mandar la
    /// audiencia que no se eligió; el diálogo abierto tras publicar; que un
    /// viewer vea «Publicar».
    #[gpui_kit::test]
    fn posting_sends_what_was_chosen_and_a_viewer_only_reads(cx: &mut TestAppContext) {
        let server = server("member", json!([card("a", json!({}))]));
        server.mount(post("/api/v1/posts").respond_with(ok(detail("n", json!({}), json!([])))));
        server.install(cx);
        let window = open(cx);
        open_wall(window, cx);

        click(window, "wall-publish", cx);
        wait_until(cx, "el diálogo", |cx| on_screen(window, "post-composer", cx));
        click(window, "post-submit", cx);
        assert!(sent(&server, "POST", "/api/v1/posts").is_empty(), "en blanco no sale");
        click(window, "post-audience-everyone", cx);
        type_in(window, "post-body-input", "que el muro tenga encuestas", cx);
        click(window, "post-submit", cx);
        wait_until(cx, "se manda", |_| !sent(&server, "POST", "/api/v1/posts").is_empty());
        assert_eq!(
            sent(&server, "POST", "/api/v1/posts"),
            [json!({ "kind": "feedback", "orgId": "o1", "audience": "everyone", "title": "", "body": "que el muro tenga encuestas" })]
        );
        wait_until(cx, "el diálogo se cierra", |cx| !on_screen(window, "post-composer", cx));
        stop(cx);
    }

    /// Un aviso del muro, con el perfil o la organización encima, los quita.
    ///
    /// Roturas que atrapa: abrir la publicación por debajo de la pantalla de
    /// al lado, sin que se vea nada.
    #[gpui_kit::test]
    fn a_wall_notice_closes_the_screen_beside(cx: &mut TestAppContext) {
        let server = server("member", json!([card("p1", json!({}))]));
        server.mount(get("/api/v1/posts/p1").respond_with(ok(detail("p1", json!({}), json!([])))));
        server.install(cx);
        let window = open(cx);
        open_wall(window, cx);
        click(window, "rail-organization", cx);
        wait_until(cx, "la organización delante", |cx| on_screen(window, "org-page", cx));

        cx.update(|cx| Backend::run(cx, |core| async move { core.inbox().open("/feedback?post=p1", "o1").await; }));
        wait_until(cx, "la publicación, a la vista", |cx| on_screen(window, "wall-modal", cx));
        assert!(!on_screen(window, "org-page", cx), "la organización sigue encima");
        stop(cx);
    }

    #[gpui_kit::test]
    fn a_viewer_sees_the_wall_but_cannot_post(cx: &mut TestAppContext) {
        let server = server("viewer", json!([card("a", json!({}))]));
        server.install(cx);
        let window = open(cx);
        open_wall(window, cx);
        wait_until(cx, "llegan las tarjetas", |cx| on_screen(window, "wall-post-a", cx));
        assert!(!on_screen(window, "wall-publish", cx));
        assert!(on_screen(window, "wall-read-only", cx));
        stop(cx);
    }

    /// Una publicación abierta: comentar manda el texto y lo pone en la
    /// lista, reaccionar manda el emoji, y borrar pregunta antes.
    ///
    /// Roturas que atrapa: comentar sin mandar o sin vaciar la caja; reaccionar
    /// con otro emoji o al revés (`on`); borrar sin preguntar, o que el
    /// «cancelar» borre; dejar el diálogo abierto tras borrar.
    #[gpui_kit::test]
    fn an_open_post_takes_comments_reactions_and_asks_before_deleting(cx: &mut TestAppContext) {
        let server = server("member", json!([card("a", json!({ "canDelete": true }))]));
        let comment = json!({ "id": "c1", "postId": "a", "authorId": "u1", "authorName": "Ana Ruiz", "body": "+1", "createdAt": "2026-11-25T11:00:00Z", "canDelete": true, "reactions": [] });
        server.mount(get("/api/v1/posts/a").respond_with(ok(detail("a", json!({ "canDelete": true }), json!([])))));
        server.mount(post("/api/v1/posts/a/comments").respond_with(ok(detail("a", json!({ "canDelete": true, "comments": 1 }), json!([comment])))));
        server.mount(post("/api/v1/posts/a/reactions").respond_with(ok(json!(null))));
        server.mount(Mock::given(method("DELETE")).and(path("/api/v1/posts/a")).respond_with(ok(json!(null))));
        server.install(cx);
        let window = open(cx);
        open_wall(window, cx);

        click(window, "wall-post-a-open", cx);
        wait_until(cx, "el diálogo de la publicación", |cx| on_screen(window, "wall-modal", cx));
        assert!(on_screen(window, "wall-no-comments", cx));
        type_in(window, "wall-comment-input", "+1", cx);
        click(window, "wall-comment-send", cx);
        wait_until(cx, "el comentario aparece", |cx| on_screen(window, "wall-comment-c1", cx));
        assert_eq!(sent(&server, "POST", "/api/v1/posts/a/comments"), [json!({ "body": "+1" })]);
        assert_eq!(label(window, "wall-post-a-comments", cx), "1 comentario");

        click(window, "wall-open-react", cx);
        click(window, "wall-open-pick-1", cx);
        wait_until(cx, "se manda la reacción", |_| !sent(&server, "POST", "/api/v1/posts/a/reactions").is_empty());
        assert_eq!(sent(&server, "POST", "/api/v1/posts/a/reactions"), [json!({ "emoji": "❤️", "on": true })]);
        wait_until(cx, "se ve la pastilla", |cx| on_screen(window, "wall-open-reaction-0", cx));

        click(window, "wall-delete", cx);
        wait_until(cx, "pregunta", |cx| on_screen(window, "wall-confirm", cx));
        click(window, "wall-confirm-cancel", cx);
        assert!(!on_screen(window, "wall-confirm", cx) && sent(&server, "DELETE", "/api/v1/posts/a").is_empty());
        click(window, "wall-delete", cx);
        click(window, "wall-confirm-delete", cx);
        wait_until(cx, "se borra", |cx| !on_screen(window, "wall-modal", cx) && !on_screen(window, "wall-post-a", cx));
        assert_eq!(server.requests().iter().filter(|r| r.method.as_str() == "DELETE" && r.url.path() == "/api/v1/posts/a").count(), 1);
        stop(cx);
    }

    /// Lo que redactó un agente: se marca, y el borrador se publica desde el
    /// diálogo sólo si se puede.
    ///
    /// Roturas que atrapa: ofrecer «Publicar» sin `canPublish`; publicar por
    /// otra ruta.
    #[gpui_kit::test]
    fn an_agents_draft_is_marked_and_published_from_the_dialog(cx: &mut TestAppContext) {
        let draft = card("d", json!({ "kind": "announcement", "draft": true, "via": "mcp", "canPublish": true }));
        let server = server("member", json!([draft.clone()]));
        server.mount(get("/api/v1/posts/d").respond_with(ok(detail("d", json!({ "kind": "announcement", "draft": true, "via": "mcp", "canPublish": true }), json!([])))));
        server.mount(post("/api/v1/posts/d/publish").respond_with(ok(detail("d", json!({ "kind": "announcement" }), json!([])))));
        server.install(cx);
        let window = open(cx);
        open_wall(window, cx);
        assert!(on_screen(window, "wall-post-d-draft", cx));
        click(window, "wall-post-d-open", cx);
        wait_until(cx, "el borrador", |cx| on_screen(window, "wall-draft", cx));
        click(window, "wall-publish-draft", cx);
        wait_until(cx, "se publica", |_| !sent(&server, "POST", "/api/v1/posts/d/publish").is_empty());
        stop(cx);
    }
}
