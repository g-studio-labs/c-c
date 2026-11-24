//! «Compartir con enlace» de una página de documentación: crear un enlace
//! público que caduca, copiarlo y revocarlo (command-and-control-496).
//!
//! Calca «Llamar con invitados» (`GuestCall` en `channel_tabs.rs`). Qué se
//! pide y cuál es el enlace lo dice el core (`cac_core::doc_share`); aquí se
//! pinta y se le pasa cada gesto.

use cac_core::{DocShare, NodeKind};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable as _, Icon, Selectable as _, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{error_text, locale, tr};
use crate::theme::Leading as _;

/// El ancho del diálogo: el mismo que el de invitados.
const DIALOG_WIDTH: f32 = 512.;

/// El diálogo se cerró: quien lo tiene lo quita.
pub struct PageShareClosed;

/// Los enlaces públicos de una página.
///
/// Crear es un botón aparte y no un clic en la duración: lo que sale de
/// aquí abre la página a quien lo tenga, sin cuenta.
pub struct PageShare {
    kind: NodeKind,
    owner_id: String,
    page_id: String,
    ttl_hours: u32,
    creating: bool,
    /// `None` mientras llegan los vivos.
    shares: Option<Vec<DocShare>>,
    problem: Option<String>,
    /// «Enlace copiado…», dicho en el diálogo.
    copied: bool,
    focus: FocusHandle,
}

impl EventEmitter<PageShareClosed> for PageShare {}

impl Focusable for PageShare {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PageShare {
    pub fn new(kind: NodeKind, owner_id: String, page_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        // Con el foco, Escape lo cierra.
        focus.focus(window, cx);
        let (owner, page) = (owner_id.clone(), page_id.clone());
        let live = Backend::ask(cx, move |core| async move { core.doc_shares().list_shares(kind, &owner, &page).await });
        cx.spawn(async move |this, cx| {
            let Ok(live) = live.await else { return };
            this.update(cx, |this, cx| {
                match live {
                    Ok(shares) => this.shares = Some(shares),
                    Err(error) => {
                        this.shares = Some(Vec::new());
                        this.problem = Some(error_text(cx, &error, "work:docShare.errList"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        let ttl_hours = cac_core::DOC_SHARE_TTLS[0].0;
        Self { kind, owner_id, page_id, ttl_hours, creating: false, shares: None, problem: None, copied: false, focus }
    }

    /// De qué página son los enlaces que enseña.
    pub fn page_id(&self) -> &str {
        &self.page_id
    }

    fn copy(&mut self, share: &DocShare, cx: &mut Context<Self>) {
        let link = Backend::core(cx).doc_shares().link(share);
        cx.write_to_clipboard(ClipboardItem::new_string(link));
        self.copied = true;
        cx.notify();
    }

    /// Crea el enlace y lo deja en el portapapeles. Si no se crea, se dice
    /// por qué.
    fn create(&mut self, cx: &mut Context<Self>) {
        if self.creating {
            return;
        }
        self.creating = true;
        self.problem = None;
        self.copied = false;
        cx.notify();
        let (kind, owner, page, ttl) = (self.kind, self.owner_id.clone(), self.page_id.clone(), self.ttl_hours);
        let created = Backend::ask(cx, move |core| async move { core.doc_shares().create_share(kind, &owner, &page, ttl).await });
        cx.spawn(async move |this, cx| {
            let Ok(created) = created.await else { return };
            this.update(cx, |this, cx| {
                this.creating = false;
                match created {
                    Ok(share) => {
                        this.copy(&share, cx);
                        this.shares.get_or_insert_with(Vec::new).insert(0, share);
                    }
                    Err(error) => this.problem = Some(error_text(cx, &error, "common:misc.errSave")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Revoca un enlace, preguntando antes: quien ya lo tenga deja de poder
    /// abrir la página, y no tiene vuelta.
    fn revoke(&mut self, share_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let (title, body, confirm, cancel) =
            (tr(cx, "work:docShare.revokeTitle"), tr(cx, "work:docShare.revokeBody"), tr(cx, "work:docShare.revokeAction"), tr(cx, "common:action.cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let Ok(revoked) = this.update(cx, |_, cx| {
                let id = share_id.clone();
                Backend::ask(cx, move |core| async move { core.doc_shares().revoke_share(&id).await })
            }) else {
                return;
            };
            let revoked = revoked.await;
            this.update(cx, |this, cx| {
                match revoked {
                    Ok(Ok(())) => {
                        if let Some(shares) = this.shares.as_mut() {
                            shares.retain(|share| share.id != share_id);
                        }
                    }
                    Ok(Err(error)) => this.problem = Some(error_text(cx, &error, "common:misc.errSave")),
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_share(&self, share: &DocShare, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let here = locale(cx);
        let when = cac_core::local_moment(&share.expires_at, here, &chrono::Local);
        let who = if share.created_by_name.trim().is_empty() { tr(cx, "work:docs.someone") } else { share.created_by_name.clone() };
        let line = cac_core::text_with(here, "work:docShare.line", &[("when", &when), ("who", &who)]);
        let (copy, revoke) = (share.clone(), share.id.clone());
        let id = &share.id;
        div()
            .id(SharedString::from(format!("doc-share-link-{id}")))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .px_3()
            .py_2()
            .aria_label(line.clone())
            .child(div().min_w_0().flex_1().truncate().leading_sm().text_color(muted).child(line))
            .child(
                Button::new(SharedString::from(format!("doc-share-copy-{id}")))
                    .outline()
                    .small()
                    .icon(Icon::new(Lucide::Link2))
                    .tooltip(tr(cx, "work:docShare.copy"))
                    .on_click(cx.listener(move |this, _, _, cx| this.copy(&copy, cx))),
            )
            .child(
                Button::new(SharedString::from(format!("doc-share-revoke-{id}")))
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::Trash))
                    .label(tr(cx, "work:docShare.revoke"))
                    .on_click(cx.listener(move |this, _, window, cx| this.revoke(revoke.clone(), window, cx))),
            )
            .test_support()
            .into_any_element()
    }
}

impl Render for PageShare {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.danger);
        let ttls: Vec<AnyElement> = cac_core::DOC_SHARE_TTLS
            .iter()
            .map(|(hours, key)| {
                let hours = *hours;
                Button::new(SharedString::from(format!("doc-share-ttl-{hours}")))
                    .outline()
                    .small()
                    .selected(self.ttl_hours == hours)
                    .label(tr(cx, key))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ttl_hours = hours;
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let list: Vec<AnyElement> = self.shares.clone().unwrap_or_default().iter().map(|share| self.render_share(share, cx)).collect();
        let none_live = self.shares.as_ref().is_some_and(|shares| shares.is_empty());
        let dialog = div()
            .id("doc-share-link")
            .test_support()
            .relative()
            .w(px(DIALOG_WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .rounded(px(12.))
            .border_1()
            .border_color(foreground.opacity(0.1))
            .bg(background)
            .text_color(foreground)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(div().text_size(px(16.)).line_height(px(16.)).font_weight(FontWeight::MEDIUM).child(tr(cx, "work:docShare.title")))
                    // Que abre también lo que cuelgue después no es obvio, y
                    // es lo que más importa saber antes de mandarlo.
                    .child(div().id("doc-share-scope").leading_sm().text_color(muted).child(tr(cx, "work:docShare.body")).test_support()),
            )
            .child(
                div().absolute().top_2().right_2().child(
                    Button::new("doc-share-dismiss")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::X))
                        .tooltip(tr(cx, "common:action.close"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(PageShareClosed))),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(div().leading_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, "work:docShare.validFor")))
                            .child(div().flex().gap_1().children(ttls)),
                    )
                    .child(
                        Button::new("doc-share-create")
                            .primary()
                            .icon(Icon::new(Lucide::Link2))
                            .label(tr(cx, if self.creating { "work:docShare.creating" } else { "work:docShare.create" }))
                            .loading(self.creating)
                            .disabled(self.creating)
                            .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                    ),
            )
            .children(self.problem.clone().map(|problem| div().id("doc-share-problem").leading_xs().text_color(danger).aria_label(problem.clone()).child(problem).test_support()))
            .when(self.copied, |this| {
                let said = tr(cx, "work:docShare.copied");
                this.child(div().id("doc-share-copied").leading_xs().text_color(muted).aria_label(said.clone()).child(said).test_support())
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().leading_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted).child(tr(cx, "work:docShare.live").to_uppercase()))
                    .when(self.shares.is_none(), |this| this.child(Spinner::new().color(muted)))
                    .when(none_live, |this| this.child(div().id("doc-share-none").leading_sm().text_color(muted).child(tr(cx, "work:docShare.none")).test_support()))
                    .children(list),
            );
        div()
            .id("doc-share-backdrop")
            .size_full()
            .track_focus(&self.focus)
            .occlude()
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(PageShareClosed);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(PageShareClosed)))
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(black().opacity(0.35))
            .child(dialog)
    }
}
