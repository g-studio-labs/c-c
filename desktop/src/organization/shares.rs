//! Enlaces compartidos: lo que esta organización tiene abierto hacia fuera.
//!
//! Cada enlace público de una página sigue abriéndola, sin cuenta, hasta que
//! caduca; aquí se ven todos juntos para tenerlos controlados (command-and-
//! control-577). Qué enlaces hay, su página y su espacio los resuelve el
//! servidor; aquí se pinta y se le pasa cada gesto.

use cac_core::{share_spaces, shares_in_space, text_with, OrgDocShare};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Icon, Selectable as _, Sizable};
use gpui_kit::*;

use super::OrgView;
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

#[derive(Default)]
pub(super) struct SharesTab {
    /// `None` mientras llegan.
    pub(super) list: Option<Vec<OrgDocShare>>,
    loading: bool,
    /// El espacio por el que se filtra; `None` es «todos».
    space: Option<String>,
}

impl OrgView {
    /// Pide los enlaces vivos de la organización. Sin vaciar lo que se ve:
    /// es lo que se usa también tras revocar.
    pub(super) fn load_shares(&mut self, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        self.shares.loading = true;
        let id = org.id.clone();
        let answer = Backend::ask(cx, move |core| async move { (id.clone(), core.doc_shares().list_org_shares(&id, None).await) });
        cx.spawn(async move |this, cx| {
            let Ok((id, shares)) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.loaded_for.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.shares.loading = false;
                match shares {
                    Ok(shares) => this.shares.list = Some(shares),
                    Err(error) => {
                        this.shares.list.get_or_insert_with(Vec::new);
                        let text = crate::lang::error_text(cx, &error, "org:errLoad");
                        this.say(text, true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn copy_share(&mut self, share: &OrgDocShare, cx: &mut Context<Self>) {
        let link = Backend::core(cx).doc_shares().org_link(share);
        cx.write_to_clipboard(ClipboardItem::new_string(link));
        let said = tr(cx, "org:shares.copied");
        self.say(said, false, cx);
    }

    /// Revoca un enlace, preguntando antes nombrando la página: quien ya lo
    /// tenga deja de poder abrirla, y no tiene vuelta. Sin el sí no se llama
    /// al servidor.
    fn ask_revoke_share(&mut self, share: OrgDocShare, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_with(locale(cx), "org:shares.revokeTitle", &[("page", &share.page_title)]);
        let (body, confirm, cancel) = (tr(cx, "org:shares.revokeBody"), tr(cx, "work:docShare.revokeAction"), tr(cx, "org:cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                let id = share.id.clone();
                this.act(move |core| async move { core.doc_shares().revoke_share(&id).await }, |this, cx| this.load_shares(cx), "org:shares.errRevoke", cx);
            })
            .ok();
        })
        .detach();
    }

    fn render_share(&self, share: &OrgDocShare, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, primary, card_bg) = (theme.border, theme.muted_foreground, theme.primary, theme.group_box);
        let here = locale(cx);
        let when = cac_core::local_moment(&share.expires_at, here, &chrono::Local);
        let who = if share.created_by_name.trim().is_empty() { tr(cx, "work:docs.someone") } else { share.created_by_name.clone() };
        let line = text_with(here, "work:docShare.line", &[("when", &when), ("who", &who)]);
        let space = if share.space_name.is_empty() { tr(cx, "org:shares.noSpace") } else { share.space_name.clone() };
        let place = if share.owner_name.is_empty() { space } else { format!("{space} · {}", share.owner_name) };
        let mode = tr(cx, if share.mode == "respond" { "org:shares.modeRespond" } else { "org:shares.modeRead" });
        let answers = text_with(here, "org:shares.answers", &[("n", &share.response_count.to_string())]);
        let id = &share.id;
        let (route, copy, revoke) = (share.page_route(), share.clone(), share.clone());
        div()
            .id(SharedString::from(format!("org-share-{id}")))
            .flex()
            .items_center()
            .gap_3()
            .px_3p5()
            .py_3()
            .rounded(px(12.))
            .border_1()
            .border_color(border)
            .bg(card_bg)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .id(SharedString::from(format!("org-share-page-{id}")))
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(primary)
                            .cursor_pointer()
                            .hover(|style| style.underline())
                            .tooltip({
                                let open = tr(cx, "org:shares.open");
                                move |window, cx| gpui_kit::component::tooltip::Tooltip::new(open.clone()).build(window, cx)
                            })
                            .on_click(cx.listener(move |_, _, _, cx| crate::links::open(&route, cx)))
                            .aria_label(share.page_title.clone())
                            .child(share.page_title.clone())
                            .test_support(),
                    )
                    .child(div().id(SharedString::from(format!("org-share-place-{id}"))).truncate().leading_xs().text_color(muted).aria_label(place.clone()).child(place).test_support())
                    .child(div().id(SharedString::from(format!("org-share-line-{id}"))).truncate().leading_xs().text_color(muted).aria_label(line.clone()).child(line).test_support())
                    .child({
                        let summary = format!("{mode} · {answers}");
                        div().id(SharedString::from(format!("org-share-mode-{id}"))).leading_xs().text_color(muted).aria_label(summary.clone()).child(summary).test_support()
                    }),
            )
            .child(
                Button::new(SharedString::from(format!("org-share-copy-{id}")))
                    .outline()
                    .small()
                    .icon(Icon::new(Lucide::Link2))
                    .tooltip(tr(cx, "work:docShare.copy"))
                    .on_click(cx.listener(move |this, _, _, cx| this.copy_share(&copy, cx))),
            )
            .child(
                Button::new(SharedString::from(format!("org-share-revoke-{id}")))
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::Trash))
                    .label(tr(cx, "work:docShare.revoke"))
                    .on_click(cx.listener(move |this, _, window, cx| this.ask_revoke_share(revoke.clone(), window, cx))),
            )
            .test_support()
            .into_any_element()
    }

    pub(super) fn render_shares(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.shares.list.is_none() && !self.shares.loading {
            self.load_shares(cx);
        }
        let (muted, border) = (cx.theme().muted_foreground, cx.theme().border);
        let list = self.shares.list.clone();
        let spaces = list.as_deref().map(share_spaces).unwrap_or_default();
        // Un espacio que ya no tiene enlaces (se revocó el último) no deja la
        // lista vacía con un filtro que ya no se ve.
        let chosen = self.shares.space.clone().filter(|space| spaces.iter().any(|(id, _)| id == space));
        let chips = (spaces.len() > 1).then(|| {
            let all = Button::new("org-shares-space-all").outline().small().selected(chosen.is_none()).label(tr(cx, "org:shares.all")).on_click(cx.listener(|this, _, _, cx| {
                this.shares.space = None;
                cx.notify();
            }));
            div().id("org-shares-spaces").flex().flex_wrap().gap_1p5().child(all).children(spaces.iter().map(|(id, name)| {
                let pick = id.clone();
                Button::new(SharedString::from(format!("org-shares-space-{id}")))
                    .outline()
                    .small()
                    .selected(chosen.as_deref() == Some(id.as_str()))
                    .label(name.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.shares.space = Some(pick.clone());
                        cx.notify();
                    }))
            }))
        });
        let body: AnyElement = match list.as_deref() {
            None => div().id("org-shares-loading").py_8().text_center().text_color(muted).child(tr(cx, "common:servers.loading")).test_support().into_any_element(),
            Some([]) => div().id("org-shares-none").px_3().py_8().rounded(px(12.)).border_1().border_dashed().border_color(border).text_center().text_color(muted).child(tr(cx, "org:shares.none")).test_support().into_any_element(),
            Some(all) => {
                let shown = shares_in_space(all, chosen.as_deref());
                div().id("org-shares").flex().flex_col().gap_3().children(shown.into_iter().map(|share| self.render_share(share, cx))).test_support().into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().max_w(px(660.)).leading_xs().text_color(muted).child(tr(cx, "org:shares.lead")))
            .children(chips)
            .child(body)
            .into_any_element()
    }
}
