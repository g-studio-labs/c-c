//! «Por atender»: lo que los agentes esperan de quien mira, con sus botones
//! (`AgentRequests.tsx` y `RequestCard.tsx`, command-and-control-504).
//!
//! Qué se manda por cada botón, cómo quedó uno cerrado y qué falta para
//! poder mandarlo lo decide el core (`cac_core::build_answer`,
//! `request_outcome`); aquí se pinta y se pulsa. Lo propio de esta pantalla
//! es lo que se está escribiendo en cada tarjeta y qué modo tiene abierto
//! (editar, rechazar, «no funciona»).

use std::collections::{HashMap, HashSet};

use cac_core::{AgentRequest, AnswerDraft, InboxTarget, RequestKind, RequestScope, RequestStatus};
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
    shown_highlight: Option<String>,
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

        let mut column = div()
            .id(SharedString::from(format!("request-{code}")))
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
            .child(div().whitespace_normal().child(request.summary.clone()))
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

    fn block(id: String, text: String, mono: bool, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        div()
            .id(SharedString::from(id))
            .w_full()
            .min_w_0()
            .p_3()
            .rounded(px(6.))
            .bg(theme.muted)
            .when(mono, |this| this.font_family(theme.mono_font_family.clone()).overflow_x_scroll())
            .children(text.lines().map(|line| div().when(mono, |this| this.whitespace_nowrap()).child(line.to_string())).collect::<Vec<_>>())
    }

    /// Lo que el agente dejó, según el tipo.
    fn render_body(&mut self, request: &AgentRequest, cx: &mut Context<Self>) -> AnyElement {
        let code = request.code.clone();
        let body = &request.body;
        let primary = cx.theme().primary;
        let column = div().flex().flex_col().gap_2();
        match request.kind {
            RequestKind::ApproveMessage => {
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
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.draft"), cx)).child(Self::block(format!("request-draft-{code}"), body.draft.clone(), false, cx).test_support()))
                    .into_any_element()
            }
            RequestKind::Test => {
                let steps = body.steps.iter().enumerate().map(|(n, step)| div().flex().gap_1p5().child(div().flex_none().child(format!("{}.", n + 1))).child(div().child(step.clone())));
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
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.steps"), cx)).children(steps.collect::<Vec<_>>()))
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.expected"), cx)).child(body.expected.clone()))
                    .into_any_element()
            }
            RequestKind::Decide => column.when(!body.question.is_empty(), |this| this.child(div().font_weight(FontWeight::MEDIUM).child(body.question.clone()))).into_any_element(),
            RequestKind::Run => {
                let command = body.command.clone();
                column
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.where"), cx)).child(body.where_.clone()))
                    .child(
                        div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.command"), cx)).child(
                            div()
                                .flex()
                                .items_start()
                                .gap_2()
                                .child(Self::block(format!("request-command-{code}"), body.command.clone(), true, cx).flex_1().test_support())
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
                    .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.expected"), cx)).child(body.expected.clone()))
                    .when(!body.undo.is_empty(), |this| this.child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.undo"), cx)).child(body.undo.clone())))
                    .into_any_element()
            }
            RequestKind::Obtain => column
                .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.what"), cx)).child(body.what.clone()))
                .child(div().flex().flex_col().gap_1().child(Self::field_label(tr(cx, "work:requests.from"), cx)).child(body.from.clone()))
                .into_any_element(),
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
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded(px(6.))
            .bg(theme.muted)
            .child(div().id(SharedString::from(format!("request-outcome-{code}"))).font_weight(FontWeight::MEDIUM).child(cac_core::request_outcome(request, language)).test_support())
            .when(!answer.note.is_empty() && request.kind != RequestKind::Test, |this| this.child(div().child(answer.note.clone())))
            .when(!answer.output.is_empty(), |this| this.child(Self::block(format!("request-outcome-output-{code}"), answer.output.clone(), true, cx)))
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

impl Render for RequestsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Backend::snapshot(cx).read(cx).requests.clone();
        let theme = cx.theme().clone();
        let (background, foreground, muted, border, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.danger);
        // Un resaltado nuevo: se lleva la vista a él, y si ya está resuelto,
        // «Resueltos» se abre.
        if state.highlight != self.shown_highlight {
            self.shown_highlight = state.highlight.clone();
            self.scroll_to = state.highlight.clone();
        }
        if let Some(code) = self.scroll_to.clone() {
            if state.closed.iter().any(|request| request.code == code) {
                self.resolved_open = true;
            }
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
        // Las tarjetas son hijas directas de lo que se desliza (para
        // `scroll_to_item`): en qué puesto va la resaltada, contando lo de
        // encima.
        let resolved_open = self.resolved_open;
        let before_open = 1 + usize::from(state.failed) + usize::from(state.loaded && state.open.is_empty() || !state.loaded);
        let at = highlight.as_deref().and_then(|code| {
            state.open.iter().position(|request| request.code == code).map(|n| before_open + n).or_else(|| {
                state.closed.iter().position(|request| request.code == code).filter(|_| resolved_open).map(|n| before_open + state.open.len() + 1 + n)
            })
        });
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
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w_full()
                    .max_w(rems(48.))
                    .mx_auto()
                    .p_6()
                    .child(header)
                    .when(state.failed, |this| this.child(div().id("requests-error").leading_sm().text_color(danger).child(tr(cx, "work:requests.errLoad")).test_support()))
                    .child(status_line)
                    .children(open)
                    .children(toggle)
                    .children(closed)
                    .children(said.map(|said| {
                        div()
                            .id("requests-said")
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
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::Mock;

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
        server.install(cx);
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1200.), px(1400.)) })),
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
}
