//! «Por atender»: lo que los agentes esperan de quien mira, con sus botones
//! (`AgentRequests.tsx` y `RequestCard.tsx`, command-and-control-504).
//!
//! Qué se manda por cada botón, cómo quedó uno cerrado y qué falta para
//! poder mandarlo lo decide el core (`cac_core::build_answer`,
//! `request_outcome`); aquí se pinta y se pulsa. Lo propio de esta pantalla
//! es lo que se está escribiendo en cada tarjeta y qué modo tiene abierto
//! (editar, rechazar, «no funciona»).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use cac_core::{AgentRequest, AnswerDraft, InboxTarget, LinkSegment, RequestKind, RequestScope, RequestStatus};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{error_text, locale, tr};
use crate::theme::Leading as _;

/// Lo abierto en una tarjeta, además de sus botones.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Idle,
    /// «Editar y aprobar»: el borrador en un campo.
    Edit,
    /// «Rechazar»: el motivo, obligatorio.
    Reject,
    /// «No funciona» se pulsó sin nota: ahora la nota es obligatoria.
    Fails,
}

/// Lo escrito en una tarjeta. Vive mientras el pedido siga abierto: la lista
/// se vuelve a pedir con cada evento, y lo escrito no se pierde por eso.
struct Card {
    mode: Mode,
    text: Entity<TextareaState>,
    note: Entity<TextareaState>,
    reason: Entity<TextareaState>,
    output: Entity<TextareaState>,
}

pub struct RequestsView {
    cards: HashMap<String, Card>,
    busy: HashSet<String>,
    /// Por qué no se pudo contestar, por pedido.
    problems: HashMap<String, String>,
    /// El último ID copiado: su botón dice que ya está.
    copied: Option<String>,
    /// Lo que se dice tras pulsar algo que esta app todavía no tiene.
    said: Option<String>,
    resolved_open: bool,
    scroll: ScrollHandle,
    /// El pedido resaltado al que todavía no se ha llevado la vista.
    scroll_to: Option<String>,
    /// El resaltado ya atendido: se atiende una sola vez, la primera vez que su
    /// pedido aparece en la lista.
    shown_highlight: Option<String>,
    /// Las tarjetas en el orden en que quedaron pintadas, para saber si la
    /// lista cambió.
    order: Vec<String>,
    /// Dónde quedó pintada cada tarjeta en el último cuadro, que cada una
    /// anota al pintarse (el `ScrollHandle` de esta versión no da las de sus
    /// hijos).
    tops: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    /// La primera tarjeta que se veía y a cuántos píxeles del borde de arriba,
    /// anotada cuando la lista cambió. La corrige esa misma tarjeta al pintarse
    /// en su sitio nuevo.
    anchor: Rc<RefCell<Option<(String, Pixels)>>>,
    _subscriptions: Vec<Subscription>,
}

impl RequestsView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            cards: HashMap::new(),
            busy: HashSet::new(),
            problems: HashMap::new(),
            copied: None,
            said: None,
            resolved_open: false,
            scroll: ScrollHandle::new(),
            scroll_to: None,
            shown_highlight: None,
            order: Vec::new(),
            tops: Rc::default(),
            anchor: Rc::default(),
            _subscriptions: Vec::new(),
        }
    }

    fn card(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.cards.contains_key(code) {
            return;
        }
        let field = |placeholder: String, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| TextareaState::new(window, cx).placeholder(placeholder).auto_grow(2, 10));
        let text = field(tr(cx, "work:requests.finalText"), window, cx);
        let note = field(tr(cx, "work:requests.noteOptional"), window, cx);
        let reason = field(tr(cx, "work:requests.rejectReason"), window, cx);
        let output = field(tr(cx, "work:requests.output"), window, cx);
        for input in [&text, &note, &reason, &output] {
            // Los botones que piden texto se encienden al escribirlo.
            self._subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        self.cards.insert(code.to_string(), Card { mode: Mode::Idle, text, note, reason, output });
    }

    fn value(field: &Entity<TextareaState>, cx: &App) -> String {
        field.read(cx).value().to_string()
    }

    fn set_mode(&mut self, code: &str, mode: Mode, cx: &mut Context<Self>) {
        if let Some(card) = self.cards.get_mut(code) {
            card.mode = mode;
        }
        self.problems.remove(code);
        cx.notify();
    }

    /// «Editar y aprobar»: el campo empieza con el borrador.
    fn start_edit(&mut self, request: &AgentRequest, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.cards.get(&request.code) else { return };
        let draft = request.body.draft.clone();
        card.text.update(cx, |field, cx| {
            field.set_value(draft, window, cx);
            field.focus(window, cx);
        });
        self.set_mode(&request.code, Mode::Edit, cx);
    }

    fn send(&mut self, code: String, draft: AnswerDraft, cx: &mut Context<Self>) {
        // Lo que no se puede mandar ni se intenta: el botón ya está apagado,
        // esto es por si llega un clic igual.
        if cac_core::build_answer(&draft).is_none() || !self.busy.insert(code.clone()) {
            return;
        }
        self.problems.remove(&code);
        cx.notify();
        let asked = code.clone();
        let answer = Backend::ask(cx, move |core| async move { core.requests().answer(&asked, draft).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                this.busy.remove(&code);
                match result {
                    Ok(_) => {
                        this.cards.remove(&code);
                    }
                    Err(error) => {
                        let said = error_text(cx, &error, "work:requests.errAnswer");
                        this.problems.insert(code, said);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Un enlace del pedido: una dirección de fuera, al navegador; una ruta
    /// de la app, a su sitio aquí si lo tiene.
    fn open_link(&mut self, url: String, org_id: String, cx: &mut Context<Self>) {
        if !url.starts_with('/') || url.starts_with("//") {
            cx.open_url(&url);
            return;
        }
        if cac_core::link_target(&url) == InboxTarget::NotHere {
            self.said = Some(tr(cx, "common:misc.notInThisApp"));
            cx.notify();
            return;
        }
        Backend::run(cx, move |core| async move {
            core.inbox().open(&url, &org_id).await;
        });
    }

    /// Lo que llega no mueve lo que se lee: si la lista cambia (un pedido
    /// nuevo entra por encima, se contesta uno, se abre «Resueltos»), la primera
    /// tarjeta que se veía se queda a la misma altura. Se anota antes de que se
    /// pinte la lista nueva y esa tarjeta, al pintarse, corrige lo que se
    /// desliza (`AgentRequests.tsx` lo hace con el anclaje del navegador).
    fn keep_reading(&mut self, shown: Vec<String>) {
        let changed = self.order != shown;
        let area = self.scroll.bounds();
        let mut anchor = self.anchor.borrow_mut();
        // Si la tarjeta anotada se fue (se contestó), no hay a quién corregir.
        if anchor.as_ref().is_some_and(|(code, _)| !shown.contains(code)) {
            *anchor = None;
        }
        if self.scroll_to.is_some() {
            *anchor = None;
        } else if changed && anchor.is_none() && area.size.height > px(0.) {
            let tops = self.tops.borrow();
            *anchor = self.order.iter().find_map(|code| {
                let card = tops.get(code)?;
                (card.bottom() > area.top()).then(|| (code.clone(), card.top() - area.top()))
            });
        }
        drop(anchor);
        self.order = shown;
    }

    /// Un enlace dentro de un texto del pedido: la web, al navegador; una
    /// pantalla de la app, dentro; una ruta, con el sistema, y si no existe se
    /// dice.
    fn follow(&mut self, target: LinkSegment, org_id: String, cx: &mut Context<Self>) {
        match target {
            LinkSegment::Url(url) => cx.open_url(&url),
            LinkSegment::AppRoute { route, .. } => self.open_link(route, org_id, cx),
            LinkSegment::Path { path, .. } => {
                // Mirar si existe puede tardar (una ruta de red): fuera del hilo de la ventana.
                let shown = path.clone();
                cx.spawn(async move |this, cx| {
                    let result = cx.background_executor().spawn(async move { reveal(&path) }).await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(()) => return,
                            Err(cac_mcp::host::RevealError::Failed(_)) => this.said = Some(tr(cx, "common:last.openLinkFailed")),
                            Err(_) => this.said = Some(cac_core::text_with(locale(cx), "work:requests.pathMissing", &[("path", &shown)])),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            LinkSegment::Text(_) => {}
        }
    }

    /// Un texto del pedido con sus enlaces pintados y pulsables. Sin enlaces,
    /// texto corriente.
    fn rich(&self, id: String, text: &str, org_id: &str, cx: &mut Context<Self>) -> AnyElement {
        let segments = cac_core::link_segments(text);
        if segments.iter().all(|segment| matches!(segment, LinkSegment::Text(_))) {
            return div().id(SharedString::from(id)).child(text.to_string()).test_support().into_any_element();
        }
        let primary = cx.theme().primary;
        let (mut shown, mut highlights, mut ranges, mut targets) = (String::new(), Vec::new(), Vec::new(), Vec::new());
        for segment in segments {
            let start = shown.len();
            match &segment {
                LinkSegment::Text(text) => shown.push_str(text),
                LinkSegment::Url(url) => shown.push_str(url),
                LinkSegment::Path { shown: written, .. } | LinkSegment::AppRoute { shown: written, .. } => shown.push_str(written),
            }
            if !matches!(segment, LinkSegment::Text(_)) {
                let range = start..shown.len();
                let style = HighlightStyle { color: Some(primary), underline: Some(UnderlineStyle { thickness: px(1.), color: Some(primary), wavy: false }), ..Default::default() };
                highlights.push((range.clone(), style));
                ranges.push(range);
                targets.push(segment);
            }
        }
        let (view, org_id) = (cx.entity().downgrade(), org_id.to_string());
        let path_ranges: Vec<std::ops::Range<usize>> = ranges.iter().zip(&targets).filter(|(_, target)| matches!(target, LinkSegment::Path { .. })).map(|(range, _)| range.clone()).collect();
        let hint = tr(cx, "work:requests.revealPath");
        let text = InteractiveText::new(SharedString::from(format!("{id}-text")), StyledText::new(shown).with_highlights(highlights))
            .tooltip(move |at, window, cx| path_ranges.iter().any(|range| range.contains(&at)).then(|| gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx)))
            .on_click(ranges, move |at, _, cx| {
            let target = targets[at].clone();
            let org_id = org_id.clone();
            view.update(cx, |this, cx| this.follow(target, org_id, cx)).ok();
        });
        div().id(SharedString::from(id)).child(text).test_support().into_any_element()
    }

    fn copy(&mut self, text: String, code: Option<String>, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = code;
        cx.notify();
    }

    fn render_card(&mut self, request: &AgentRequest, highlighted: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let open = request.status == RequestStatus::Open;
        if open && request.can_answer {
            self.card(&request.code, window, cx);
        }
        let language = locale(cx);
        let theme = cx.theme().clone();
        let (border, muted, primary, mono) = (theme.border, theme.muted_foreground, theme.primary, theme.mono_font_family.clone());
        let code = request.code.clone();
        let copied = self.copied.as_deref() == Some(code.as_str());
        let task_request = request.clone();
        let age = cac_core::time_ago(&request.created_at, &chrono::Utc::now(), language);
        let mut asked = cac_core::text_with(language, "work:requests.askedBy", &[("name", &request.requested_by.name)]);
        if request.via == "mcp" {
            asked = format!("{asked} {}", tr(cx, "work:requests.viaAgent"));
        }
        let asked = format!("{asked} · {}", cac_core::text_with(language, "work:requests.waitsOn", &[("name", &request.requested_of.name)]));
        let copy_label = cac_core::text_with(language, "work:requests.copyId", &[("code", &code)]);
        let copy_code = code.clone();

        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id(SharedString::from(format!("request-kind-{code}")))
                    .px_2()
                    .py_0p5()
                    .rounded(px(6.))
                    .bg(theme.secondary)
                    .text_color(theme.secondary_foreground)
                    .leading_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(request.kind.label(language))
                    .test_support(),
            )
            .child(
                div()
                    .id(SharedString::from(format!("request-task-{code}")))
                    .min_w_0()
                    .truncate()
                    .leading_xs()
                    .text_color(muted)
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .tooltip({
                        let hint = tr(cx, "work:requests.openTask");
                        move |window, cx| gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx)
                    })
                    .on_click(move |_, _, cx| {
                        let request = task_request.clone();
                        Backend::run(cx, move |core| async move {
                            core.requests().open_task(&request).await;
                        });
                    })
                    .child(request.task_line())
                    .test_support(),
            )
            .child(div().leading_xs().text_color(muted).child(format!("· {age}")))
            .child(
                Button::new(SharedString::from(format!("request-copy-{code}")))
                    .outline()
                    .xsmall()
                    .ml_auto()
                    .icon(Icon::new(if copied { Lucide::Check } else { Lucide::Copy }))
                    .child(div().font_family(mono.clone()).child(code.clone()))
                    .tooltip(copy_label)
                    .on_click(cx.listener(move |this, _, _, cx| this.copy(copy_code.clone(), Some(copy_code.clone()), cx))),
            );

        let summary = self.rich(format!("request-summary-{code}"), &request.summary, &request.org_id, cx);
        let (tops, anchor, scroll, noted) = (self.tops.clone(), self.anchor.clone(), self.scroll.clone(), code.clone());
        let mut column = div()
            .id(SharedString::from(format!("request-{code}")))
            .relative()
            .child(
                canvas(
                    move |bounds, window, _| {
                        tops.borrow_mut().insert(noted.clone(), bounds);
                        // Si esta es la tarjeta que se leía, y la lista nueva
                        // la dejó en otro sitio, se sube o baja lo que se desliza
                        // lo que se movió y se pinta otra vez.
                        let was = anchor.borrow().as_ref().filter(|(code, _)| *code == noted).map(|(_, was)| *was);
                        if let Some(was) = was {
                            anchor.borrow_mut().take();
                            let moved = (bounds.top() - scroll.bounds().top()) - was;
                            if moved.abs() > px(0.5) {
                                let offset = scroll.offset();
                                scroll.set_offset(point(offset.x, offset.y - moved));
                                window.refresh();
                            }
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(px(10.))
            .border_1()
            .border_color(if highlighted { primary } else { border })
            .when(highlighted, |this| {
                this.shadow(vec![BoxShadow { color: primary, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false }])
            })
            .bg(crate::theme::css_color("raised", cx))
            .leading_sm()
            .child(header)
            .child(div().whitespace_normal().child(summary))
            .child(div().leading_xs().text_color(muted).child(asked))
            .child(self.render_body(request, cx));
        column = if open {
            if request.can_answer {
                column.child(self.render_controls(request, cx))
            } else {
                column.child(
                    div()
                        .id(SharedString::from(format!("request-not-yours-{code}")))
                        .leading_xs()
                        .text_color(muted)
                        .child(cac_core::text_with(language, "work:requests.notYours", &[("name", &request.requested_of.name)]))
                        .test_support(),
                )
            }
        } else {
            column.child(self.render_outcome(request, cx))
        };
        if let Some(problem) = self.problems.get(&code) {
            column = column.child(div().id(SharedString::from(format!("request-problem-{code}"))).leading_xs().text_color(theme.danger).child(problem.clone()).test_support());
        }
        column.test_support().into_any_element()
    }

    fn field_label(label: String, cx: &App) -> Div {
        div().leading_xs().font_weight(FontWeight::MEDIUM).text_color(cx.theme().muted_foreground).child(label)
    }

    /// Un bloque de texto en caja (el borrador, el comando, lo que salió). Con
    /// enlaces pintados si `org_id` viene: copiar sigue copiando el texto crudo,
    /// que no pasa por aquí.
    fn block(&self, id: String, text: String, mono: bool, org_id: Option<&str>, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let lines: Vec<AnyElement> = text
            .lines()
            .enumerate()
            .map(|(n, line)| {
                let content = match org_id {
                    Some(org) => self.rich(format!("{id}-line-{n}"), line, org, cx),
                    None => div().child(line.to_string()).into_any_element(),
                };
                div().when(mono, |this| this.whitespace_nowrap()).child(content).into_any_element()
            })
            .collect();
        div()
            .id(SharedString::from(id))
            .w_full()
            .min_w_0()
            .p_3()
            .rounded(px(6.))
            .bg(theme.muted)
            .when(mono, |this| this.font_family(theme.mono_font_family.clone()).overflow_x_scroll())
            .children(lines)
    }

    /// Lo que el agente dejó, según el tipo.
    fn render_body(&mut self, request: &AgentRequest, cx: &mut Context<Self>) -> AnyElement {
        let code = request.code.clone();
        let body = &request.body;
        let primary = cx.theme().primary;
        let org = request.org_id.clone();
        let column = div().flex().flex_col().gap_2();
        match request.kind {
            RequestKind::ApproveMessage => {
                let draft_block = self.block(format!("request-draft-{code}"), body.draft.clone(), false, Some(&org), cx);
                let destination: AnyElement = if body.destination_url.is_empty() {
                    div().child(body.destination.clone()).into_any_element()
                } else {
                    let (url, org) = (body.destination_url.clone(), request.org_id.clone());
                    div()
                        .id(SharedString::from(format!("request-destination-{code}")))
                        .text_color(primary)
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(cx.listener(move |this, _, _, cx| this.open_link(url.clone(), org.clone(), cx)))
                        .child(body.destination.clone())
                        .into_any_element()
                };
                column
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.destination"), cx)).child(destination))
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.draft"), cx)).child(draft_block.test_support()))
                    .into_any_element()
            }
            RequestKind::Test => {
                let steps: Vec<AnyElement> = body
                    .steps
                    .iter()
                    .enumerate()
                    .map(|(n, step)| div().flex().gap_1p5().child(div().flex_none().child(format!("{}.", n + 1))).child(div().min_w_0().child(self.rich(format!("request-step-{code}-{n}"), step, &org, cx))).into_any_element())
                    .collect();
                let expected = self.rich(format!("request-expected-{code}"), &body.expected, &org, cx);
                let (url, org) = (body.url.clone(), request.org_id.clone());
                column
                    .when(!body.url.is_empty(), |this| {
                        this.child(
                            div().child(
                                Button::new(SharedString::from(format!("request-open-screen-{code}")))
                                    .outline()
                                    .small()
                                    .icon(Icon::new(Lucide::ExternalLink))
                                    .label(tr(cx, "work:requests.openScreen"))
                                    .on_click(cx.listener(move |this, _, _, cx| this.open_link(url.clone(), org.clone(), cx))),
                            ),
                        )
                    })
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.steps"), cx)).children(steps))
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.expected"), cx)).child(expected))
                    .into_any_element()
            }
            RequestKind::Decide => {
                let question = self.rich(format!("request-question-{code}"), &body.question, &org, cx);
                column.when(!body.question.is_empty(), |this| this.child(div().font_weight(FontWeight::MEDIUM).child(question))).into_any_element()
            }
            RequestKind::Run => {
                let command = body.command.clone();
                let (where_, expected, undo) = (
                    self.rich(format!("request-where-{code}"), &body.where_, &org, cx),
                    self.rich(format!("request-expected-{code}"), &body.expected, &org, cx),
                    self.rich(format!("request-undo-{code}"), &body.undo, &org, cx),
                );
                let command_block = self.block(format!("request-command-{code}"), body.command.clone(), true, Some(&org), cx);
                column
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.where"), cx)).child(where_))
                    .child(
                        div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.command"), cx)).child(
                            div()
                                .flex()
                                .items_start()
                                .gap_2()
                                .child(command_block.flex_1().test_support())
                                .child(
                                    Button::new(SharedString::from(format!("request-copy-command-{code}")))
                                        .outline()
                                        .small()
                                        .icon(Icon::new(Lucide::Copy))
                                        .tooltip(tr(cx, "work:requests.copyCommand"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.copy(command.clone(), None, cx);
                                            this.said = Some(tr(cx, "work:requests.commandCopied"));
                                        })),
                                ),
                        ),
                    )
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.expected"), cx)).child(expected))
                    .when(!body.undo.is_empty(), |this| this.child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.undo"), cx)).child(undo)))
                    .into_any_element()
            }
            RequestKind::Obtain => {
                let (what, from) = (self.rich(format!("request-what-{code}"), &body.what, &org, cx), self.rich(format!("request-from-{code}"), &body.from, &org, cx));
                column
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.what"), cx)).child(what))
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.from"), cx)).child(from))
                    .into_any_element()
            }
            RequestKind::Unknown => column.into_any_element(),
        }
    }

    fn textarea(id: String, field: &Entity<TextareaState>, mono: bool, cx: &App) -> Stateful<Div> {
        div()
            .id(SharedString::from(id))
            .when(mono, |this| this.font_family(cx.theme().mono_font_family.clone()))
            .child(Textarea::new(field))
    }

    /// Los botones del tipo.
    fn render_controls(&mut self, request: &AgentRequest, cx: &mut Context<Self>) -> AnyElement {
        let code = request.code.clone();
        let Some(card) = self.cards.get(&code) else { return div().into_any_element() };
        let (mode, text, note, reason, output) = (card.mode, card.text.clone(), card.note.clone(), card.reason.clone(), card.output.clone());
        let busy = self.busy.contains(&code);
        let note_typed = !Self::value(&note, cx).trim().is_empty();
        let row = || div().flex().flex_wrap().gap_2();
        let id = |name: &str| SharedString::from(format!("request-{name}-{code}"));
        // Cuando la nota es obligatoria se dice encima: el campo es el mismo,
        // y lo escrito antes de saberlo no se pierde.
        let note_box = |required: bool, cx: &mut Context<Self>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .when(required, |this| this.child(div().id(SharedString::from(format!("request-note-required-{code}"))).child(Self::field_label(tr(cx, "work:requests.noteRequired"), cx)).test_support()))
                .child(Self::textarea(format!("request-note-{code}"), &note, false, cx).test_support())
        };
        match request.kind {
            RequestKind::ApproveMessage if mode == Mode::Edit => {
                let typed = Self::value(&text, cx);
                let original = request.body.draft.clone();
                let send_code = code.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Self::textarea(format!("request-text-{code}"), &text, false, cx).test_support())
                    .child(
                        row()
                            .child(Button::new(id("approve-text")).primary().small().label(tr(cx, "work:requests.approveThis")).disabled(busy || typed.trim().is_empty()).on_click(cx.listener(move |this, _, _, cx| {
                                let edited = Self::value(&text, cx);
                                this.send(send_code.clone(), AnswerDraft::Approve { edited: Some(edited), original: original.clone() }, cx)
                            })))
                            .child(Button::new(id("cancel")).ghost().small().label(tr(cx, "work:requests.cancel")).on_click(cx.listener({
                                let code = code.clone();
                                move |this, _, _, cx| this.set_mode(&code, Mode::Idle, cx)
                            }))),
                    )
                    .into_any_element()
            }
            RequestKind::ApproveMessage if mode == Mode::Reject => {
                let send_code = code.clone();
                let reason_typed = !Self::value(&reason, cx).trim().is_empty();
                let typed_reason = reason.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Self::textarea(format!("request-reason-{code}"), &reason, false, cx).test_support())
                    .child(
                        row()
                            .child(Button::new(id("send-reject")).danger().small().label(tr(cx, "work:requests.sendReject")).disabled(busy || !reason_typed).on_click(cx.listener(move |this, _, _, cx| {
                                let reason = Self::value(&typed_reason, cx);
                                this.send(send_code.clone(), AnswerDraft::Reject { reason }, cx)
                            })))
                            .child(Button::new(id("cancel")).ghost().small().label(tr(cx, "work:requests.cancel")).on_click(cx.listener({
                                let code = code.clone();
                                move |this, _, _, cx| this.set_mode(&code, Mode::Idle, cx)
                            }))),
                    )
                    .into_any_element()
            }
            RequestKind::ApproveMessage => {
                let (approve_code, original, reject_code, edit_request) = (code.clone(), request.body.draft.clone(), code.clone(), request.clone());
                row()
                    .child(Button::new(id("approve")).primary().small().label(tr(cx, "work:requests.approve")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
                        this.send(approve_code.clone(), AnswerDraft::Approve { edited: None, original: original.clone() }, cx)
                    })))
                    .child(Button::new(id("edit")).outline().small().label(tr(cx, "work:requests.editApprove")).disabled(busy).on_click(cx.listener(move |this, _, window, cx| this.start_edit(&edit_request, window, cx))))
                    .child(Button::new(id("reject")).danger().small().label(tr(cx, "work:requests.reject")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| this.set_mode(&reject_code, Mode::Reject, cx))))
                    .into_any_element()
            }
            RequestKind::Test => {
                let failing = mode == Mode::Fails;
                let (works_code, fails_code, works_note, fails_note) = (code.clone(), code.clone(), note.clone(), note.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(note_box(failing, cx))
                    .child(
                        row()
                            .child(Button::new(id("works")).primary().small().label(tr(cx, "work:requests.works")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
                                let note = Self::value(&works_note, cx);
                                this.send(works_code.clone(), AnswerDraft::Works { note }, cx)
                            })))
                            .child(Button::new(id("fails")).danger().small().label(tr(cx, "work:requests.fails")).disabled(busy || (failing && !note_typed)).on_click(cx.listener(move |this, _, _, cx| {
                                // «No funciona» pide decir qué se vio: el
                                // primer clic sin nota la pide; con ella, manda.
                                let note = Self::value(&fails_note, cx);
                                if note.trim().is_empty() {
                                    this.set_mode(&fails_code, Mode::Fails, cx);
                                    return;
                                }
                                this.send(fails_code.clone(), AnswerDraft::Fails { note }, cx)
                            }))),
                    )
                    .into_any_element()
            }
            RequestKind::Decide => {
                let options = request.body.options.iter().enumerate().map(|(n, option)| {
                    let (choose_code, label, chosen_note) = (code.clone(), option.label.clone(), note.clone());
                    let button = Button::new(SharedString::from(format!("request-option-{code}-{n}")))
                        .small()
                        .disabled(busy)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_start()
                                .py_1()
                                .child(
                                    div().flex().items_center().gap_2().child(option.label.clone()).when(option.recommended, |this| {
                                        this.child(div().px_1p5().rounded(px(4.)).border_1().leading_xs().child(tr(cx, "work:requests.recommended")))
                                    }),
                                )
                                .child(div().leading_xs().whitespace_normal().opacity(0.8).child(option.consequence.clone())),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let note = Self::value(&chosen_note, cx);
                            this.send(choose_code.clone(), AnswerDraft::Choose { option: label.clone(), note }, cx)
                        }));
                    if option.recommended { button.primary() } else { button.outline() }
                });
                div().flex().flex_col().gap_2().children(options.collect::<Vec<_>>()).child(note_box(false, cx)).into_any_element()
            }
            RequestKind::Run => {
                let (done_code, pasted) = (code.clone(), output.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Self::textarea(format!("request-output-{code}"), &output, true, cx).test_support())
                    .child(row().child(Button::new(id("done")).primary().small().label(tr(cx, "work:requests.done")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
                        let output = Self::value(&pasted, cx);
                        this.send(done_code.clone(), AnswerDraft::Done { note: String::new(), output }, cx)
                    }))))
                    .into_any_element()
            }
            RequestKind::Obtain => {
                let (done_code, typed) = (code.clone(), note.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(note_box(false, cx))
                    .child(row().child(Button::new(id("ready")).primary().small().label(tr(cx, "work:requests.ready")).disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
                        let note = Self::value(&typed, cx);
                        this.send(done_code.clone(), AnswerDraft::Done { note, output: String::new() }, cx)
                    }))))
                    .into_any_element()
            }
            RequestKind::Unknown => div().into_any_element(),
        }
    }

    /// Cómo quedó un pedido cerrado.
    fn render_outcome(&mut self, request: &AgentRequest, cx: &mut Context<Self>) -> AnyElement {
        let language = locale(cx);
        let theme = cx.theme().clone();
        let code = request.code.clone();
        let answer = request.answer.clone().unwrap_or_default();
        let by = request.answered_by.as_ref().map(|person| {
            let when = cac_core::time_ago(request.answered_at.as_deref().unwrap_or_default(), &chrono::Utc::now(), language);
            let mut line = cac_core::text_with(language, "work:requests.outcome.by", &[("name", &person.name), ("when", &when)]);
            if request.answered_via == "session" {
                line = format!("{line} · {}", tr(cx, "work:requests.outcome.viaSession"));
            }
            line
        });
        let (published, org) = (request.published_url.clone(), request.org_id.clone());
        let output_block = (!answer.output.is_empty()).then(|| self.block(format!("request-outcome-output-{code}"), answer.output.clone(), true, None, cx));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded(px(6.))
            .bg(theme.muted)
            .child(div().id(SharedString::from(format!("request-outcome-{code}"))).font_weight(FontWeight::MEDIUM).child(cac_core::request_outcome(request, language)).test_support())
            .when(!answer.note.is_empty() && request.kind != RequestKind::Test, |this| this.child(div().child(answer.note.clone())))
            .children(output_block)
            .when(answer.edited && !answer.final_text.is_empty(), |this| this.child(div().leading_xs().child(answer.final_text.clone())))
            .children(by.map(|line| div().leading_xs().text_color(theme.muted_foreground).child(line)))
            .when(!published.is_empty(), |this| {
                this.child(
                    div()
                        .id(SharedString::from(format!("request-published-{code}")))
                        .leading_xs()
                        .text_color(theme.primary)
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(cx.listener(move |this, _, _, cx| this.open_link(published.clone(), org.clone(), cx)))
                        .child(tr(cx, "work:requests.outcome.published")),
                )
            })
            .into_any_element()
    }

    fn render_scope(&self, scope: RequestScope, cx: &mut Context<Self>) -> impl IntoElement {
        let chip = |id: &'static str, this: RequestScope, key: &str, cx: &mut Context<Self>| {
            let button = Button::new(id).small().label(tr(cx, key)).on_click(cx.listener(move |view, _, _, cx| {
                view.cards.clear();
                view.problems.clear();
                cx.notify();
                Backend::run(cx, move |core| async move { core.requests().set_scope(this).await });
            }));
            if scope == this { button.secondary() } else { button.ghost() }
        };
        div()
            .flex()
            .gap_0p5()
            .p_0p5()
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().border)
            .child(chip("requests-scope-mine", RequestScope::Mine, "work:requests.scope.mine", cx))
            .child(chip("requests-scope-team", RequestScope::Team, "work:requests.scope.team", cx))
    }
}

/// Mostrar una ruta en el gestor de ficheros, seleccionada: nunca se abre
/// (ver `cac_mcp::host::reveal_path`). En las pruebas sólo se anota.
fn reveal(path: &str) -> Result<(), cac_mcp::host::RevealError> {
    #[cfg(test)]
    {
        if !cac_mcp::host::expand_home(path).exists() {
            return Err(cac_mcp::host::RevealError::NotFound);
        }
        tests::OPENED.lock().unwrap().push(path.to_string());
        Ok(())
    }
    #[cfg(not(test))]
    cac_mcp::host::reveal(path)
}

impl Render for RequestsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).requests.clone();
        let theme = cx.theme().clone();
        let (background, foreground, muted, border, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.danger);
        // El enlace se atiende una vez, cuando su pedido aparece: se lleva la
        // vista a él, y «Resueltos» se abre sólo si ya estaba resuelto. Que
        // luego se conteste (y pase a resueltos) o lleguen otros no lo vuelve
        // a abrir ni a mover.
        if let Some(code) = state.highlight.as_ref().filter(|code| self.shown_highlight.as_ref() != Some(code)) {
            let closed = state.closed.iter().any(|request| &request.code == code);
            if closed || state.open.iter().any(|request| &request.code == code) {
                self.shown_highlight = Some(code.clone());
                self.scroll_to = Some(code.clone());
                self.resolved_open |= closed;
            }
        } else if state.highlight.is_none() {
            self.shown_highlight = None;
        }
        // Lo escrito en un pedido que ya no está abierto no tiene a dónde ir.
        self.cards.retain(|code, _| state.open.iter().any(|request| &request.code == code));

        let highlight = state.highlight.clone();
        let open: Vec<AnyElement> = state.open.iter().map(|request| self.render_card(request, highlight.as_deref() == Some(request.code.as_str()), window, cx)).collect();
        let closed: Vec<AnyElement> = if self.resolved_open {
            state.closed.iter().map(|request| self.render_card(request, highlight.as_deref() == Some(request.code.as_str()), window, cx)).collect()
        } else {
            Vec::new()
        };
        // `scroll_to_item` cuenta hijos directos de lo que se desliza: en qué
        // puesto va la resaltada, contando lo de encima (título, error, estado)
        // y el pliegue «Resueltos».
        let resolved_open = self.resolved_open;
        let before_open = 2 + usize::from(state.failed) + state.elsewhere.len();
        let shown: Vec<&str> = state.open.iter().chain(state.closed.iter().filter(|_| resolved_open)).map(|request| request.code.as_str()).collect();
        let at = highlight.as_deref().and_then(|code| {
            let n = shown.iter().position(|shown| *shown == code)?;
            Some(before_open + n + usize::from(n >= state.open.len()))
        });
        self.keep_reading(shown.iter().map(|code| code.to_string()).collect());
        if let (Some(_), Some(at)) = (self.scroll_to.as_ref(), at) {
            // Un `ScrollHandle` que aún no se pintó no sabe su alto y tira la
            // petición sin moverse: se pide cuando la lista ya se pintó.
            if self.scroll.bounds().size.height > px(0.) {
                self.scroll.scroll_to_item(at);
                self.scroll_to = None;
            } else {
                window.refresh();
            }
        }

        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(div().id("requests-title").leading_lg().font_weight(FontWeight::SEMIBOLD).child(tr(cx, "work:requests.title")).test_support())
                    .child(div().leading_sm().text_color(muted).child(tr(cx, "work:requests.lead"))),
            )
            .child(self.render_scope(state.scope, cx));
        let empty_key = if state.scope == RequestScope::Mine { "work:requests.empty" } else { "work:requests.emptyTeam" };
        let status_line: AnyElement = if !state.loaded {
            div().id("requests-loading").leading_sm().text_color(muted).child(tr(cx, "work:requests.loading")).test_support().into_any_element()
        } else if state.open.is_empty() {
            div()
                .id("requests-empty")
                .p_6()
                .rounded(px(10.))
                .border_1()
                .border_dashed()
                .border_color(border)
                .text_center()
                .leading_sm()
                .text_color(muted)
                .child(tr(cx, empty_key))
                .test_support()
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let resolved_count = state.closed.len();
        let toggle = (resolved_count > 0).then(|| {
            div()
                .id("requests-resolved")
                .flex()
                .items_center()
                .gap_2()
                .leading_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(muted)
                .cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.resolved_open = !this.resolved_open;
                    cx.notify();
                }))
                .child(Icon::new(if self.resolved_open { Lucide::ChevronDown } else { Lucide::ChevronRight }).size_4())
                .child(cac_core::text_with(locale(cx), "work:requests.resolved", &[("count", &resolved_count.to_string())]))
                .test_support()
                .into_any_element()
        });
        let language = locale(cx);
        let elsewhere: Vec<AnyElement> = state
            .elsewhere
            .iter()
            .map(|other| {
                let org_id = other.org_id.clone();
                let text = cac_core::elsewhere_label(other, language);
                div()
                    .id(SharedString::from(format!("requests-elsewhere-{}", other.org_id)))
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .leading_sm()
                    .cursor_pointer()
                    .hover(move |style| style.bg(muted.opacity(0.1)))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let org_id = org_id.clone();
                        Backend::run(cx, move |core| async move {
                            core.switch_org(&org_id).await;
                        });
                    }))
                    .aria_label(text.clone())
                    .child(text)
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let said = self.said.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .line_height(relative(1.5))
            .child(
                div()
                    .id("requests")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .test_support()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w_full()
                    .max_w(rems(48.))
                    .mx_auto()
                    .p_6()
                    .child(header)
                    .children(elsewhere)
                    .when(state.failed, |this| this.child(div().id("requests-error").leading_sm().text_color(danger).child(tr(cx, "work:requests.errLoad")).test_support()))
                    .child(status_line)
                    .children(open)
                    .children(toggle)
                    .children(closed)
                    .children(said.map(|said| {
                        div()
                            .id("requests-said")
                            .aria_label(said.clone())
                            .leading_xs()
                            .text_color(muted)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.said = None;
                                cx.notify();
                            }))
                            .child(said)
                            .test_support()
                    })),
            )
    }
}

/// «Espera a Ana»: la tarea tiene pedidos abiertos a una persona
/// (`WaitingOnChip` de `Tasks.tsx`). A quién, lo sabe el core.
pub(crate) fn waiting_chip(id: String, task_id: &str, cx: &App) -> Option<AnyElement> {
    let names = Backend::snapshot(cx).read(cx).requests.waiting_on.get(task_id).cloned()?;
    let text = cac_core::waiting_label(&names, locale(cx))?;
    let warning = cx.theme().warning;
    Some(
        div()
            .id(SharedString::from(id))
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .px_1p5()
            .py_0p5()
            .rounded(px(4.))
            .bg(warning.opacity(0.15))
            .text_color(warning)
            .leading_xs()
            .aria_label(text.clone())
            .child(Icon::new(Lucide::Hand).size_3())
            .child(text)
            .test_support()
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use crate::backend::Backend;
    use crate::testing::{contract_task, ok, server_with_tasks, sign_in, wait_until, FakeServer, Gate};
    use crate::workspace::Workspace;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{point, px, size, AnyWindowHandle, AppContext, Bounds, Pixels, Point, ScrollDelta, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, ResponseTemplate};

    /// Las rutas que la pantalla habría mostrado en el gestor de ficheros.
    pub(super) static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn request(code: &str, kind: &str, task: &str, of: (&str, &str), mine: bool, body: Value) -> Value {
        json!({
            "id": format!("id-{code}"), "code": code, "orgId": "o1", "kind": kind, "summary": format!("resumen {code}"), "body": body,
            "version": 1, "status": "open", "requestedOf": { "id": of.0, "name": of.1 }, "requestedBy": { "id": "bot", "name": "Agente" }, "via": "mcp",
            "task": { "id": task, "ref": "cac-1", "title": "Subir el backend", "priority": "high", "status": "in_progress", "link": format!("/tasks?task={task}") },
            "createdAt": "2026-11-24T10:00:00Z", "updatedAt": "2026-11-24T10:00:00Z", "canAnswer": mine, "link": format!("/requests?open={code}"),
        })
    }

    fn mine() -> Vec<Value> {
        let ana = ("u1", "Ana Ruiz");
        vec![
            request("req-a1", "approve_message", "t1", ana, true, json!({ "draft": "Hola equipo", "destination": "Basecamp" })),
            request("req-a2", "test", "t1", ana, true, json!({ "url": "https://staging.example.com", "steps": ["Entra"], "expected": "07:30" })),
        ]
    }

    /// Ana tiene dos pedidos en `t1`; Luis, uno en `t2`.
    fn server() -> FakeServer {
        let server = server_with_tasks(vec![
            contract_task("t1", "a", |task| task["title"] = json!("Subir el backend")),
            contract_task("t2", "a", |task| task["title"] = json!("Revisar el diseño")),
        ]);
        let list = |scope: &str, status: &str| {
            Mock::given(method("GET")).and(path("/api/v1/requests")).and(query_param("scope", scope.to_string())).and(query_param("status", status.to_string()))
        };
        let mut team = mine();
        team.push(request("req-l1", "decide", "t2", ("u2", "Luis"), false, json!({ "question": "¿Cuál?", "options": [{ "label": "A", "consequence": "rápido" }] })));
        server.mount(list("team", "open").respond_with(ok(Value::Array(team))));
        server.mount(list("mine", "open").respond_with(ok(Value::Array(mine()))));
        server.mount(list("mine", "closed").respond_with(ok(json!([]))));
        server.mount(list("team", "closed").respond_with(ok(json!([]))));
        server
    }

    fn open(server: &FakeServer, cx: &mut TestAppContext) -> AnyWindowHandle {
        open_sized(server, 1400., cx)
    }

    fn open_sized(server: &FakeServer, height: f32, cx: &mut TestAppContext) -> AnyWindowHandle {
        server.install(cx);
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1200.), px(height)) })),
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

    fn label(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).and_then(|found| found.label().map(str::to_string)).unwrap_or_default()
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

    fn type_in(window: AnyWindowHandle, field: &str, text: &str, cx: &mut TestAppContext) {
        let (field, text) = (gpui_kit::SharedString::from(field.to_string()), text.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(field, cx);
            window.input(&text, cx);
        })
        .unwrap();
    }

    fn answers(server: &FakeServer, code: &str) -> Vec<Value> {
        let route = format!("/api/v1/requests/{code}/answer");
        server.requests().iter().filter(|request| request.method.as_str() == "POST" && request.url.path() == route).map(|request| serde_json::from_slice(&request.body).unwrap()).collect()
    }

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    fn show_requests(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "el número de «Por atender»", |cx| on_screen(window, "panel-requests-count", cx));
        click(window, "panel-requests", cx);
        wait_until(cx, "los pedidos de Ana", |cx| on_screen(window, "request-req-a1", cx));
    }

    /// El panel cuenta los tuyos, las tareas dicen a quién esperan, y en
    /// «Equipo» los de otro se leen sin botones mientras el número sigue
    /// siendo tuyo.
    ///
    /// Roturas que atrapa: un número que cuenta los del equipo; la marca
    /// «espera a…» que no llega a la fila; ofrecer contestar el de otro; que
    /// «Mi trabajo» siga delante tras pulsar la fila; que el tablero de
    /// pulsar el árbol no devuelva las tareas.
    #[gpui_kit::test]
    fn the_panel_counts_yours_and_the_team_reads_without_buttons(cx: &mut TestAppContext) {
        let server = server();
        let window = open(&server, cx);
        wait_until(cx, "el número de «Por atender»", |cx| on_screen(window, "panel-requests-count", cx));
        assert_eq!(label(window, "panel-requests-count", cx), "2");
        wait_until(cx, "la marca en la fila", |cx| on_screen(window, "work-waiting-t2", cx));
        assert_eq!(label(window, "work-waiting-t1", cx), "Espera a Ana Ruiz");
        assert_eq!(label(window, "work-waiting-t2", cx), "Espera a Luis");

        show_requests(window, cx);
        assert!(!on_screen(window, "work-row-t1", cx), "«Mi trabajo» sigue delante");
        assert!(on_screen(window, "request-approve-req-a1", cx));
        assert!(!on_screen(window, "request-req-l1", cx), "el de Luis no es de Ana");

        click(window, "requests-scope-team", cx);
        wait_until(cx, "los del equipo", |cx| on_screen(window, "request-req-l1", cx));
        assert!(on_screen(window, "request-not-yours-req-l1", cx));
        assert!(!on_screen(window, "request-option-req-l1-0", cx), "el de Luis no se contesta por él");
        assert_eq!(label(window, "panel-requests-count", cx), "2", "el número sigue siendo de Ana");

        click(window, "tree-everything", cx);
        wait_until(cx, "vuelven las tareas", |cx| on_screen(window, "work-row-t1", cx));
        stop(cx);
    }

    /// Los pedidos de otra organización no salen ni cuentan, se avisa de ellos,
    /// y el aviso lleva a esa organización (command-and-control-527).
    ///
    /// Roturas que atrapa: pintar o contar los de la otra organización; no
    /// avisar de ellos; un aviso que no cambia de organización; que tras
    /// cambiar sigan saliendo los de la de antes.
    #[gpui_kit::test]
    fn the_other_organizations_are_announced_not_listed(cx: &mut TestAppContext) {
        let server = server();
        let org = |id: &str, name: &str| json!({ "id": id, "name": name, "slug": id, "role": "member", "memberCount": 2, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
        server.mount(Mock::given(method("GET")).and(path("/api/v1/organizations/")).respond_with(ok(json!([org("o1", "G Studio"), org("o2", "Otra org")]))).with_priority(1));
        let mut elsewhere = request("req-g1", "decide", "t9", ("u1", "Ana Ruiz"), true, json!({ "question": "¿Cuál?", "options": [{ "label": "A", "consequence": "rápido" }] }));
        elsewhere["orgId"] = json!("o2");
        let with_it = |mut list: Vec<Value>| {
            list.push(elsewhere.clone());
            Value::Array(list)
        };
        let list = |scope: &str, status: &str| {
            Mock::given(method("GET")).and(path("/api/v1/requests")).and(query_param("scope", scope.to_string())).and(query_param("status", status.to_string()))
        };
        server.mount(list("team", "open").respond_with(ok(with_it(mine()))).with_priority(1));
        server.mount(list("mine", "open").respond_with(ok(with_it(mine()))).with_priority(1));
        let window = open(&server, cx);
        wait_until(cx, "el número de «Por atender»", |cx| on_screen(window, "panel-requests-count", cx));
        assert_eq!(label(window, "panel-requests-count", cx), "2", "el de la otra organización no cuenta");

        show_requests(window, cx);
        assert!(!on_screen(window, "request-req-g1", cx), "el de la otra organización no se pinta");
        assert_eq!(label(window, "requests-elsewhere-o2", cx), "1 pedido más en Otra org");

        click(window, "requests-elsewhere-o2", cx);
        wait_until(cx, "los de la otra organización", |cx| on_screen(window, "request-req-g1", cx));
        assert!(!on_screen(window, "request-req-a1", cx), "los de la de antes ya no");
        assert_eq!(label(window, "requests-elsewhere-o1", cx), "2 pedidos más en G Studio");
        stop(cx);
    }

    /// Rechazar pide motivo, y mientras el servidor contesta no se puede
    /// mandar otra vez; al contestar pasa a «Resueltos». El ID se copia.
    ///
    /// Roturas que atrapa: rechazar sin motivo; un doble clic que contesta
    /// dos veces; dejarlo en abiertos tras contestarlo; un ID que no copia.
    #[gpui_kit::test]
    fn rejecting_asks_for_a_reason_and_sends_once(cx: &mut TestAppContext) {
        let server = server();
        let gate = Gate::default();
        let mut done = mine()[0].clone();
        done["status"] = json!("answered");
        done["answer"] = json!({ "approved": false, "reason": "el tono" });
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a1/answer")).respond_with(gate.hold(ok(done))));
        let window = open(&server, cx);
        show_requests(window, cx);

        click(window, "request-copy-req-a1", cx);
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("req-a1"));

        click(window, "request-reject-req-a1", cx);
        click(window, "request-send-reject-req-a1", cx);
        assert!(answers(&server, "req-a1").is_empty(), "sin motivo no sale");
        type_in(window, "request-reason-req-a1", "el tono", cx);
        click(window, "request-send-reject-req-a1", cx);
        wait_until(cx, "la respuesta, retenida", |_| gate.held() == 1);
        click(window, "request-send-reject-req-a1", cx);
        assert_eq!(answers(&server, "req-a1"), [json!({ "approved": false, "reason": "el tono" })], "una sola vez");

        gate.open();
        wait_until(cx, "pasa a «Resueltos»", |cx| on_screen(window, "requests-resolved", cx) && !on_screen(window, "request-req-a1", cx));
        click(window, "requests-resolved", cx);
        assert!(on_screen(window, "request-outcome-req-a1", cx));
        stop(cx);
    }

    /// «No funciona» sin nota la pide en vez de mandar; con ella, manda.
    ///
    /// Rotura que atrapa: un «no funciona» que sale sin decir qué se vio.
    #[gpui_kit::test]
    fn doesnt_work_needs_a_note(cx: &mut TestAppContext) {
        let server = server();
        let mut done = mine()[1].clone();
        done["status"] = json!("answered");
        done["answer"] = json!({ "works": false, "note": "sale 18:40" });
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a2/answer")).respond_with(ok(done)));
        let window = open(&server, cx);
        show_requests(window, cx);

        click(window, "request-fails-req-a2", cx);
        assert!(answers(&server, "req-a2").is_empty());
        assert!(on_screen(window, "request-note-required-req-a2", cx), "dice que la nota hace falta");
        type_in(window, "request-note-req-a2", "sale 18:40", cx);
        click(window, "request-fails-req-a2", cx);
        wait_until(cx, "sale la respuesta", |_| !answers(&server, "req-a2").is_empty());
        assert_eq!(answers(&server, "req-a2"), [json!({ "works": false, "note": "sale 18:40" })]);
        stop(cx);
    }

    /// Un agente pide abrir un pedido: «Por atender» delante, aunque se
    /// estuviera en comunicación, y el servidor sabe que se enseñó.
    ///
    /// Roturas que atrapa: que la pantalla no cambie; que el «lo enseñé» no
    /// salga (el MCP diría que no hay app abierta).
    #[gpui_kit::test]
    fn an_agent_asking_to_open_shows_it(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a2/opened")).respond_with(ok(json!(null))));
        let window = open(&server, cx);
        wait_until(cx, "las tareas", |cx| on_screen(window, "work-row-t1", cx));
        click(window, "rail-comms", cx);
        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.stream().deliver(cac_core::ServerEvent { event: "request:open".into(), data: json!({ "code": "req-a2", "nonce": "n1" }).to_string() });
            })
        });
        wait_until(cx, "el pedido delante", |cx| on_screen(window, "request-req-a2", cx));
        wait_until(cx, "el «lo enseñé»", |_| server.hits("/api/v1/requests/req-a2/opened") == 1);
        assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).requests.highlight.clone()).as_deref(), Some("req-a2"));
        stop(cx);
    }

    type Shared = Arc<Mutex<Vec<Value>>>;

    /// Un servidor cuyas listas de «mine» y «team» se cambian en plena prueba.
    fn live_server(open: &Shared, closed: &Shared) -> FakeServer {
        let server = server_with_tasks(vec![contract_task("t1", "a", |task| task["title"] = json!("Subir el backend"))]);
        for scope in ["mine", "team"] {
            for (status, list) in [("open", open), ("closed", closed)] {
                let list = list.clone();
                server.mount(
                    Mock::given(method("GET"))
                        .and(path("/api/v1/requests"))
                        .and(query_param("scope", scope.to_string()))
                        .and(query_param("status", status.to_string()))
                        .respond_with(move |_: &wiremock::Request| ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": list.lock().unwrap().clone() }))),
                );
            }
        }
        server
    }

    fn approve(code: &str, summary: &str) -> Value {
        let mut value = request(code, "approve_message", "t1", ("u1", "Ana Ruiz"), true, json!({ "draft": "Hola", "destination": "Basecamp" }));
        value["summary"] = json!(summary);
        value
    }

    fn bounds(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> Option<Bounds<Pixels>> {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).map(|found| found.bounds())
        })
        .unwrap()
    }

    fn click_start(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click_at(id, point(px(3.), px(3.)), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Una dirección, una ruta que existe, una que no y una pantalla de la app
    /// escritas en el texto de un pedido se pulsan, y cada una hace lo suyo.
    ///
    /// Roturas que atrapa: la dirección que no llega al navegador; la ruta
    /// que no llega al gestor de ficheros, o que se muestra sin existir; una ruta que no
    /// existe y no dice nada; una pantalla de la app que sale al navegador en
    /// vez de abrirse dentro.
    #[gpui_kit::test]
    fn links_in_a_request_open_what_they_point_at(cx: &mut TestAppContext) {
        let here = std::env::temp_dir().to_string_lossy().to_string();
        let listed: Shared = Arc::new(Mutex::new(vec![
            approve("req-u1", "https://supabase.example/sql/new y listo"),
            approve("req-p1", &format!("{here} existe")),
            approve("req-p2", "/no/existe/cac-504.sql no está"),
            approve("req-r1", "/app/tasks?task=t1 es la tarea"),
        ]));
        let server = live_server(&listed, &Arc::new(Mutex::new(Vec::new())));
        let window = open_sized(&server, 2400., cx);
        show_list(window, "req-u1", cx);
        wait_until(cx, "el último pedido", |cx| on_screen(window, "request-summary-req-r1", cx));
        OPENED.lock().unwrap().clear();

        click_start(window, "request-summary-req-u1", cx);
        assert_eq!(cx.opened_url().as_deref(), Some("https://supabase.example/sql/new"));
        assert!(OPENED.lock().unwrap().is_empty(), "una dirección no es una ruta");

        click_start(window, "request-summary-req-p1", cx);
        wait_until(cx, "la ruta mostrada", |_| !OPENED.lock().unwrap().is_empty());
        assert_eq!(OPENED.lock().unwrap().clone(), [here.clone()]);

        click_start(window, "request-summary-req-p2", cx);
        assert_eq!(OPENED.lock().unwrap().len(), 1, "una ruta que no existe no se abre");
        wait_until(cx, "el aviso de que no existe", |cx| label(window, "requests-said", cx).contains("/no/existe/cac-504.sql"));

        click_start(window, "request-summary-req-r1", cx);
        assert_eq!(cx.opened_url().as_deref(), Some("https://supabase.example/sql/new"), "la pantalla de la app no se le da al navegador");
        wait_until(cx, "la tarea se abre dentro", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).task.task_id.clone()).as_deref() == Some("t1"));
        stop(cx);
    }

    /// Lo que entra por encima no mueve lo que se lee: con la lista deslizada,
    /// un pedido nuevo (que ordena primero) deja la primera tarjeta visible
    /// donde estaba.
    ///
    /// Rotura que atrapa: no corregir el desplazamiento tras pintar la lista
    /// nueva (todo baja lo que mide el pedido que entró).
    #[gpui_kit::test]
    fn a_request_arriving_above_does_not_move_what_is_read(cx: &mut TestAppContext) {
        let codes = ["req-b1", "req-b2", "req-b3", "req-b4", "req-b5", "req-b6"];
        let listed: Shared = Arc::new(Mutex::new(codes.iter().map(|code| approve(code, "uno")).collect()));
        let server = live_server(&listed, &Arc::new(Mutex::new(Vec::new())));
        let window = open_sized(&server, 560., cx);
        show_list(window, "req-b1", cx);
        scroll_down(window, 420., cx);
        let (code, was) = first_visible(window, &codes, cx);
        assert_ne!(code, "req-b1", "la lista tenía que estar deslizada");

        listed.lock().unwrap().insert(0, approve("req-b0", "nuevo"));
        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.stream().deliver(cac_core::ServerEvent { event: "request:created".into(), data: json!({ "code": "req-b0" }).to_string() });
            })
        });
        wait_until(cx, "el nuevo en la lista", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).requests.open.len()) == 7);
        wait_until(cx, "la tarjeta de antes, donde estaba", |cx| bounds(window, &format!("request-{code}"), cx).is_some_and(|now| (f32::from(now.top()) - was).abs() < 1.));
        stop(cx);
    }

    /// Contestar el pedido resaltado no abre «Resueltos»: sólo se abre si el
    /// enlace llegó cuando ya estaba resuelto.
    ///
    /// Rotura que atrapa: abrir «Resueltos» cada vez que el pedido resaltado
    /// cae en resueltos, que es lo que desplegaba la lista bajo quien estaba
    /// contestando.
    #[gpui_kit::test]
    fn answering_the_highlighted_request_does_not_open_resolved(cx: &mut TestAppContext) {
        let listed: Shared = Arc::new(Mutex::new(vec![approve("req-a1", "uno"), approve("req-a2", "dos")]));
        let closed: Shared = Arc::new(Mutex::new(Vec::new()));
        let server = live_server(&listed, &closed);
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a1/opened")).respond_with(ok(json!(null))));
        let mut done = approve("req-a1", "uno");
        done["status"] = json!("answered");
        done["answer"] = json!({ "approved": true });
        server.mount(Mock::given(method("POST")).and(path("/api/v1/requests/req-a1/answer")).respond_with(ok(done.clone())));
        let window = open(&server, cx);
        wait_until(cx, "las tareas", |cx| on_screen(window, "work-row-t1", cx));
        cx.update(|cx| {
            Backend::run(cx, |core| async move {
                core.stream().deliver(cac_core::ServerEvent { event: "request:open".into(), data: json!({ "code": "req-a1", "nonce": "n1" }).to_string() });
            })
        });
        wait_until(cx, "el pedido resaltado", |cx| on_screen(window, "request-req-a1", cx));

        *listed.lock().unwrap() = vec![approve("req-a2", "dos")];
        *closed.lock().unwrap() = vec![done];
        click(window, "request-approve-req-a1", cx);
        wait_until(cx, "pasa a «Resueltos»", |cx| on_screen(window, "requests-resolved", cx) && !on_screen(window, "request-req-a1", cx));
        for _ in 0..5 {
            cx.run_until_parked();
            assert!(!on_screen(window, "request-outcome-req-a1", cx), "«Resueltos» no se abre solo");
        }
        stop(cx);
    }

    fn show_list(window: AnyWindowHandle, first: &str, cx: &mut TestAppContext) {
        wait_until(cx, "el número de «Por atender»", |cx| on_screen(window, "panel-requests-count", cx));
        click(window, "panel-requests", cx);
        let wanted = format!("request-{first}");
        wait_until(cx, "los pedidos", |cx| on_screen(window, &wanted, cx));
    }

    fn scroll_down(window: AnyWindowHandle, by: f32, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.scroll("requests", ScrollDelta::Pixels(point(px(0.), px(-by))), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// La primera tarjeta que aún se ve, y a qué altura de la ventana.
    fn first_visible(window: AnyWindowHandle, codes: &[&str], cx: &mut TestAppContext) -> (String, f32) {
        let area = bounds(window, "requests", cx).expect("la lista");
        for code in codes {
            if let Some(card) = bounds(window, &format!("request-{code}"), cx) {
                if card.bottom() > area.top() {
                    return (code.to_string(), f32::from(card.top()));
                }
            }
        }
        panic!("no se ve ninguna tarjeta");
    }
}
