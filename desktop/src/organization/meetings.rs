//! Reuniones (`OrgMeetings.tsx`): las que se repiten y suenan solas.
//!
//! Lo que se programa aquí **suena**: a la hora exacta a cada miembro le sale
//! una tarjeta con timbre, como una llamada. Por eso la pantalla dice, antes
//! de guardar, a qué hora es de verdad para cada quien, y a quién le va a
//! sonar. Qué hora es, cómo se dice la regla y qué se manda lo decide el core
//! (`cac_core::org_meetings`).

use cac_core::{
    default_meeting_zone, is_zone, meeting_preview, meeting_when, next_ring, readable_rule, reached, text_with, toggled_exclusion, weekday_name, zone_label, Freq, Meeting, MeetingForm, MeetingRoom,
    ALL_DAYS, WEEK_ORDER,
};
use chrono::Utc;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::pickers::{choice_chip, Choice};
use super::{card, OrgView};
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// El formulario abierto: crear una reunión, o editar la que ya existe.
pub(super) struct Editor {
    existing: Option<Meeting>,
    form: MeetingForm,
    title: Entity<InputState>,
    time: Entity<InputState>,
    zone: Entity<InputState>,
    every: Entity<InputState>,
    month_day: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(super) struct MeetingsTab {
    list: Option<Vec<Meeting>>,
    rooms: Vec<MeetingRoom>,
    loading: bool,
    editor: Option<Editor>,
    /// La reunión cuyo «a quién le llega» está abierto.
    people_open: Option<String>,
    /// Reuniones con una lista de excluidos viajando, y las que cambiaron
    /// mientras tanto: se vuelve a mandar la última, de una en una.
    sending_people: std::collections::HashSet<String>,
    resend_people: std::collections::HashSet<String>,
}

impl OrgView {
    /// Pide las reuniones y las salas de esta organización. Sin vaciar lo que
    /// ya se ve: es lo que se usa también tras cada cambio.
    pub(super) fn load_meetings(&mut self, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        self.meetings.loading = true;
        let id = org.id.clone();
        let answer = Backend::ask(cx, move |core| async move {
            let orgs = core.orgs();
            let meetings = orgs.meetings(&id).await;
            let rooms = orgs.meeting_rooms(&id).await.unwrap_or_default();
            (id, meetings, rooms)
        });
        cx.spawn(async move |this, cx| {
            let Ok((id, meetings, rooms)) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.loaded_for.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.meetings.loading = false;
                match meetings {
                    Ok(meetings) => this.meetings.list = Some(meetings),
                    Err(error) => {
                        this.meetings.list.get_or_insert_with(Vec::new);
                        let text = crate::lang::error_text(cx, &error, "org:errLoad");
                        this.say(text, true, cx);
                    }
                }
                this.meetings.rooms = rooms;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_editor(&mut self, existing: Option<Meeting>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let zone = default_meeting_zone(Some(org.timezone.as_str()).filter(|zone| !zone.is_empty()), &self.viewer_zone);
        let form = MeetingForm::new(existing.as_ref(), &zone);
        let field = |value: String, placeholder: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(placeholder);
                state.set_value(value, window, cx);
                state
            })
        };
        let title = field(form.title.clone(), tr(cx, "org:meetingNamePlaceholder"), window, cx);
        let time = field(form.time.clone(), "09:30".into(), window, cx);
        let zone_input = field(form.zone.clone(), "America/Mexico_City".into(), window, cx);
        let every = field(form.every.to_string(), String::new(), window, cx);
        let month_day = field(form.month_day.to_string(), String::new(), window, cx);
        title.update(cx, |input, cx| input.focus(window, cx));
        let watch = |input: &Entity<InputState>, apply: fn(&mut MeetingForm, &str), window: &mut Window, cx: &mut Context<Self>| {
            cx.subscribe_in(input, window, move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    if let Some(editor) = this.meetings.editor.as_mut() {
                        apply(&mut editor.form, &value);
                    }
                    cx.notify();
                }
            })
        };
        let subscriptions = vec![
            watch(&title, |form, value| form.title = value.to_string(), window, cx),
            watch(&time, |form, value| form.time = value.to_string(), window, cx),
            watch(&zone_input, |form, value| form.zone = value.trim().to_string(), window, cx),
            watch(&every, |form, value| form.set_every(value), window, cx),
            watch(&month_day, |form, value| form.set_month_day(value), window, cx),
        ];
        self.meetings.editor = Some(Editor { existing, form, title, time, zone: zone_input, every, month_day, _subscriptions: subscriptions });
        self.notice = None;
        cx.notify();
    }

    fn save_meeting(&mut self, cx: &mut Context<Self>) {
        let Some((org, _)) = Self::current(cx) else { return };
        let Some(editor) = self.meetings.editor.as_ref() else { return };
        let Some(draft) = editor.form.draft(editor.existing.as_ref(), chrono::Local::now().date_naive()) else { return };
        let (org_id, existing) = (org.id.clone(), editor.existing.as_ref().map(|meeting| meeting.id.clone()));
        let fallback = if existing.is_some() { "org:errSaveMeeting" } else { "org:errCreateMeeting" };
        self.act(
            move |core| async move {
                match existing {
                    Some(id) => core.orgs().update_meeting(&id, &draft).await,
                    None => core.orgs().create_meeting(&org_id, &draft).await,
                }
            },
            |this, cx| {
                this.meetings.editor = None;
                this.load_meetings(cx);
            },
            fallback,
            cx,
        );
    }

    fn toggle_pause(&mut self, meeting: &Meeting, cx: &mut Context<Self>) {
        let (id, paused) = (meeting.id.clone(), !meeting.paused);
        self.act(move |core| async move { core.orgs().set_meeting_paused(&id, paused).await }, |this, cx| this.load_meetings(cx), "org:errSaveMeeting", cx);
    }

    fn ask_delete(&mut self, meeting: Meeting, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_with(locale(cx), "org:deleteTitle", &[("name", &meeting.title)]);
        let (body, confirm, cancel) = (tr(cx, "org:deleteMeetingBody"), tr(cx, "org:delete"), tr(cx, "org:cancel"));
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&body), &[confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| {
                let id = meeting.id.clone();
                this.act(move |core| async move { core.orgs().delete_meeting(&id).await }, |this, cx| this.load_meetings(cx), "org:errSaveMeeting", cx);
            })
            .ok();
        })
        .detach();
    }

    /// Marca o desmarca a alguien sobre lo último que se ve, incluido lo que
    /// aún no ha llegado al servidor, y manda la lista de una en una: dos
    /// clics seguidos no se pisan.
    fn toggle_person(&mut self, meeting_id: &str, user_id: &str, cx: &mut Context<Self>) {
        let Some(meeting) = self.meetings.list.as_mut().and_then(|list| list.iter_mut().find(|meeting| meeting.id == meeting_id)) else { return };
        meeting.excluded_user_ids = toggled_exclusion(meeting, user_id);
        cx.notify();
        if !self.meetings.sending_people.insert(meeting_id.to_string()) {
            self.meetings.resend_people.insert(meeting_id.to_string());
            return;
        }
        self.send_exclusions(meeting_id.to_string(), cx);
    }

    fn send_exclusions(&mut self, meeting_id: String, cx: &mut Context<Self>) {
        let Some(excluded) = self.meetings.list.as_ref().and_then(|list| list.iter().find(|meeting| meeting.id == meeting_id)).map(|meeting| meeting.excluded_user_ids.clone()) else { return };
        let (id, asked_for) = (meeting_id.clone(), self.loaded_for.clone());
        let answer = Backend::ask(cx, move |core| async move { core.orgs().set_meeting_excluded(&id, &excluded).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update(cx, |this, cx| {
                if this.loaded_for != asked_for {
                    return;
                }
                if result.is_ok() && this.meetings.resend_people.remove(&meeting_id) {
                    this.send_exclusions(meeting_id, cx);
                    return;
                }
                this.meetings.sending_people.remove(&meeting_id);
                this.meetings.resend_people.remove(&meeting_id);
                if let Err(error) = result {
                    let text = crate::lang::error_text(cx, &error, "org:errReach");
                    this.say(text, true, cx);
                }
                // Lo que se ve vuelve a ser lo que dice el servidor.
                this.load_meetings(cx);
            })
            .ok();
        })
        .detach();
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editor = self.meetings.editor.as_ref()?;
        let form = &editor.form;
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let (lang, now) = (locale(cx), Utc::now());
        let label = |key: &str, input: AnyElement| div().flex().flex_col().gap_1().child(div().leading_xs().text_color(muted).child(tr(cx, key))).child(input);

        let (team_zone, viewer_zone) = (Self::current(cx).map(|(org, _)| org.timezone).filter(|zone| !zone.is_empty()), self.viewer_zone.clone());
        let zone_ok = is_zone(&form.zone);
        let mut suggested: Vec<(String, String)> = Vec::new();
        for (zone, key) in team_zone.iter().map(|zone| (zone.clone(), "org:zoneTeam")).chain(std::iter::once((viewer_zone.clone(), "org:zoneYours"))) {
            if is_zone(&zone) {
                match suggested.iter_mut().find(|(known, _)| *known == zone) {
                    Some((_, tags)) => *tags = format!("{tags} · {}", tr(cx, key)),
                    None => suggested.push((zone, tr(cx, key))),
                }
            }
        }
        let zone_chips = suggested.into_iter().map(|(zone, tags)| {
            let id = SharedString::from(format!("org-meeting-zone-{zone}"));
            let target = zone.clone();
            Button::new(id).outline().small().label(format!("{} · {tags}", zone_label(&zone, now))).on_click(cx.listener(move |this, _, window, cx| {
                if let Some(editor) = this.meetings.editor.as_mut() {
                    editor.form.zone = target.clone();
                    editor.zone.update(cx, |input, cx| input.set_value(target.clone(), window, cx));
                }
                cx.notify();
            }))
        });

        let freq_choices = [Freq::Weekly, Freq::Monthly].into_iter().chain((form.freq == Freq::Daily).then_some(Freq::Daily)).map(|freq| {
            Choice::option(
                freq.as_str(),
                tr(
                    cx,
                    match freq {
                        Freq::Weekly => "org:onTheseDays",
                        Freq::Monthly => "org:monthly",
                        Freq::Daily => "org:daily",
                    },
                ),
            )
        });
        let freq_label = tr(
            cx,
            match form.freq {
                Freq::Weekly => "org:onTheseDays",
                Freq::Monthly => "org:monthly",
                Freq::Daily => "org:daily",
            },
        );
        let view = cx.entity().downgrade();
        let freq_picker = choice_chip("org-meeting-freq", freq_label, freq_choices.collect(), form.freq.as_str().into(), false, move |value, _, cx| {
            view.update(cx, |this, cx| {
                if let Some(editor) = this.meetings.editor.as_mut() {
                    editor.form.freq = match value.as_str() {
                        "monthly" => Freq::Monthly,
                        "daily" => Freq::Daily,
                        _ => Freq::Weekly,
                    };
                }
                cx.notify();
            })
            .ok();
        });

        let days = (form.freq == Freq::Weekly).then(|| {
            let chips = WEEK_ORDER.iter().map(|day| {
                let on = form.days.contains(day);
                let (day, name) = (*day, weekday_name(lang, (*day + 6) % 7, true));
                let button = Button::new(SharedString::from(format!("org-meeting-day-{day}"))).small().label(name).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(editor) = this.meetings.editor.as_mut() {
                        editor.form.toggle_day(day);
                    }
                    cx.notify();
                }));
                if on { button.primary() } else { button.outline() }
            });
            let preset = |id: &'static str, key: &str, on: bool, days: &'static [u32], cx: &Context<Self>| {
                let button = Button::new(id).small().label(tr(cx, key)).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(editor) = this.meetings.editor.as_mut() {
                        editor.form.days = days.to_vec();
                    }
                    cx.notify();
                }));
                if on { button.secondary() } else { button.ghost() }
            };
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .children(chips)
                .child(div().mx_1().w(px(1.)).h(px(16.)).bg(cx.theme().border))
                .child(preset("org-meeting-workweek", "org:weekdaysPreset", form.is_workweek(), &cac_core::WORKWEEK, cx))
                .child(preset("org-meeting-everyday", "org:everyDayPreset", form.is_every_day(), &ALL_DAYS, cx))
                .when(form.needs_a_day(), |this| this.child(div().id("org-meeting-pick-a-day").leading_xs().text_color(danger).child(tr(cx, "org:pickADay")).test_support()))
        });

        let month = (form.freq == Freq::Monthly).then(|| {
            div()
                .flex()
                .flex_wrap()
                .items_end()
                .gap_3()
                .child(label("org:dayOfMonth", div().w(px(96.)).child(Input::new(&editor.month_day).id("org-meeting-month-day")).into_any_element()))
                // Lo que pasa en los meses cortos, dicho antes de que sorprenda.
                .when(form.month_day > 28, |this| this.child(div().leading_xs().text_color(muted).pb_2().child(tr(cx, "org:lastDayNote"))))
        });

        // A qué hora suena, allí y para quien mira: antes de guardar, no
        // después.
        let preview = meeting_preview(&form.time, &form.zone, &viewer_zone, now, lang).map(|text| div().id("org-meeting-preview").leading_xs().aria_label(text.clone()).child(text).test_support());

        let mut rooms = vec![Choice::option("", tr(cx, "org:noRoom"))];
        rooms.extend(self.meetings.rooms.iter().map(|room| Choice::option(room.id.clone(), if room.is_general() { format!("{} (general)", room.name) } else { room.name.clone() })));
        let room_label = self.meetings.rooms.iter().find(|room| room.id == form.room).map_or_else(|| tr(cx, "org:noRoom"), |room| room.name.clone());
        let view = cx.entity().downgrade();
        let room_picker = choice_chip("org-meeting-room", room_label, rooms, form.room.clone(), false, move |value, _, cx| {
            view.update(cx, |this, cx| {
                if let Some(editor) = this.meetings.editor.as_mut() {
                    editor.form.room = value;
                }
                cx.notify();
            })
            .ok();
        });

        let saving = self.busy;
        let creating = editor.existing.is_none();
        Some(
            card("org-meeting-form", cx)
                .child(div().max_w(px(384.)).child(Input::new(&editor.title).id("org-meeting-title")))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_end()
                        .gap_3()
                        .child(label("org:time", div().w(px(128.)).child(Input::new(&editor.time).id("org-meeting-time")).into_any_element()))
                        .child(label("org:repeats", freq_picker.into_any_element()))
                        .child(label("org:every", div().w(px(80.)).child(Input::new(&editor.every).id("org-meeting-every")).into_any_element())),
                )
                .child(
                    label(
                        "org:timeZone",
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(div().max_w(px(320.)).child(Input::new(&editor.zone).id("org-meeting-zone")))
                            .child(div().flex().flex_wrap().gap_1().children(zone_chips))
                            .child(if zone_ok {
                                div().id("org-meeting-zone-label").leading_xs().text_color(muted).child(zone_label(&form.zone, now)).into_any_element()
                            } else {
                                div().id("org-meeting-zone-bad").leading_xs().text_color(danger).child(tr(cx, "org:badZone")).into_any_element()
                            })
                            .into_any_element(),
                    ),
                )
                .children(days)
                .children(month)
                .children(preview)
                .child(label("org:roomToJoin", room_picker.into_any_element()))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("org-meeting-save")
                                .primary()
                                .small()
                                .label(tr(cx, if creating { "org:createMeeting" } else { "org:saveMeeting" }))
                                .disabled(saving || !form.can_save())
                                .on_click(cx.listener(|this, _, _, cx| this.save_meeting(cx))),
                        )
                        .child(Button::new("org-meeting-cancel").ghost().small().label(tr(cx, "org:cancel")).on_click(cx.listener(|this, _, _, cx| {
                            this.meetings.editor = None;
                            cx.notify();
                        }))),
                )
                .test_support()
                .into_any_element(),
        )
    }

    fn render_meeting(&self, meeting: &Meeting, manage: bool, team_zone: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, warning, danger, card_bg, chip) = (theme.border, theme.muted_foreground, theme.warning, theme.danger, theme.group_box, theme.muted);
        let lang = locale(cx);
        let id = meeting.id.clone();
        let when = meeting_when(meeting, &self.viewer_zone, team_zone, lang);
        let members = self.members.clone().unwrap_or_default();
        let member_ids: Vec<&str> = members.iter().map(|member| member.user_id.as_str()).collect();
        let reaches = text_with(lang, "org:reachesOf", &[("invited", &reached(meeting, &member_ids).to_string()), ("total", &members.len().to_string())]);
        let rule = readable_rule(meeting, lang);
        let people_open = self.meetings.people_open.as_deref() == Some(meeting.id.as_str());
        let aria = |text: String| text;
        let badge = |text: String, outline: bool| div().px_1p5().rounded(px(4.)).text_size(px(10.)).when(outline, |this| this.border_1().border_color(border)).when(!outline, |this| this.bg(chip)).child(text);
        let heading = |key: &str| div().text_size(px(10.5)).text_color(muted).child(tr(cx, key).to_uppercase());

        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3p5()
            .py_3()
            .border_b_1()
            .border_color(border)
            .child(Icon::new(Lucide::CalendarClock).size_4().text_color(muted))
            .child(div().truncate().font_weight(FontWeight::MEDIUM).when(meeting.paused, |this| this.text_color(muted)).child(meeting.title.clone()))
            .child(div().id(SharedString::from(format!("org-meeting-rule-{id}"))).aria_label(rule.clone()).child(badge(rule, false)).test_support())
            .when(meeting.paused, |this| this.child(badge(tr(cx, "org:paused"), true)))
            .children(meeting.space_name.clone().filter(|name| !name.is_empty()).map(|name| badge(format!("#{name}"), true)));

        let when_cell = div()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(heading("org:ringsAt"))
            .child(
                div()
                    .id(SharedString::from(format!("org-meeting-when-{id}")))
                    .aria_label(format!("{}{}{}", when.there, when.here.as_deref().map(|here| format!(" · {here}")).unwrap_or_default(), when.off_team.as_deref().map(|team| format!(" · {team}")).unwrap_or_default()))
                    .child(div().font_weight(FontWeight::MEDIUM).child(when.there.clone()))
                    .children(when.here.clone().map(|here| div().text_color(muted).child(format!("· {here}"))))
                    .children(when.off_team.clone().map(|team| div().text_size(px(11.)).text_color(warning).child(team)))
                    .test_support(),
            );
        let toggle_people = {
            let id = id.clone();
            Button::new(SharedString::from(format!("org-meeting-people-{id}")))
                .ghost()
                .small()
                .icon(Icon::new(Lucide::Users))
                .label(tr(cx, if people_open { "org:done" } else { "org:change" }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.meetings.people_open = if this.meetings.people_open.as_deref() == Some(id.as_str()) { None } else { Some(id.clone()) };
                    cx.notify();
                }))
        };
        let reach_cell = div()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(heading("org:reaches"))
            .child(div().flex().items_center().gap_2().child(div().id(SharedString::from(format!("org-meeting-reaches-{id}"))).aria_label(aria(reaches.clone())).child(reaches).test_support()).when(manage, |this| this.child(toggle_people)));

        let people = people_open.then(|| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .px_3p5()
                .py_3()
                .border_t_1()
                .border_color(border)
                // Marcados por defecto, y quien entre mañana entra marcado: lo
                // que se guarda es quién se quitó.
                .child(div().leading_xs().text_color(muted).child(tr(cx, "org:everyoneByDefault")))
                .child(div().flex().flex_wrap().gap_2().children(members.iter().map(|member| {
                    let included = !meeting.excluded_user_ids.contains(&member.user_id);
                    let (target, user) = (meeting.id.clone(), member.user_id.clone());
                    let button = Button::new(SharedString::from(format!("org-meeting-person-{}-{}", meeting.id, member.user_id))).small().label(format!("@{}", member.username)).on_click(cx.listener(move |this, _, _, cx| this.toggle_person(&target, &user, cx)));
                    if included { button.primary() } else { button.outline() }
                })))
        });

        let footer = manage.then(|| {
            let (editing, pausing, deleting) = (meeting.clone(), meeting.clone(), meeting.clone());
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .px_3p5()
                .py_2()
                .border_t_1()
                .border_color(border)
                .child(div().flex_1().text_size(px(11.)).text_color(muted).child(if meeting.paused { tr(cx, "org:notRinging") } else { format!("{}{}", tr(cx, "org:next"), next_ring(meeting, &self.viewer_zone, lang)) }))
                .child(Button::new(SharedString::from(format!("org-meeting-edit-{id}"))).ghost().small().icon(Icon::new(Lucide::Pencil)).label(tr(cx, "org:edit")).on_click(cx.listener(move |this, _, window, cx| this.open_editor(Some(editing.clone()), window, cx))))
                .child(
                    Button::new(SharedString::from(format!("org-meeting-pause-{id}")))
                        .ghost()
                        .small()
                        .icon(Icon::new(if meeting.paused { Lucide::Play } else { Lucide::Pause }))
                        .label(tr(cx, if meeting.paused { "org:resume" } else { "org:pause" }))
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_pause(&pausing, cx))),
                )
                .child(
                    Button::new(SharedString::from(format!("org-meeting-delete-{id}")))
                        .ghost()
                        .small()
                        .text_color(danger)
                        .icon(Icon::new(Lucide::Trash))
                        .label(tr(cx, "org:delete"))
                        .on_click(cx.listener(move |this, _, window, cx| this.ask_delete(deleting.clone(), window, cx))),
                )
        });

        div()
            .id(SharedString::from(format!("org-meeting-{id}")))
            .rounded(px(12.))
            .border_1()
            .border_color(border)
            .bg(card_bg)
            .when(meeting.paused, |this| this.opacity(0.7))
            .child(header)
            .child(div().flex().flex_wrap().gap_x_8().gap_y_2().px_3p5().py_3().leading_xs().child(when_cell).child(reach_cell))
            .children(people)
            .children(footer)
            .test_support()
            .into_any_element()
    }

    pub(super) fn render_meetings(&mut self, manage: bool, cx: &mut Context<Self>) -> AnyElement {
        // Un formulario abierto no sobrevive a perder el permiso (un cambio
        // de papel que llega con él delante).
        if !manage && self.meetings.editor.is_some() {
            self.meetings.editor = None;
        }
        if self.meetings.list.is_none() && !self.meetings.loading {
            self.load_meetings(cx);
        }
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let team_zone = Self::current(cx).map(|(org, _)| org.timezone).filter(|zone| !zone.is_empty());
        let editor = self.render_editor(cx);

        let body: AnyElement = match self.meetings.list.clone() {
            None => div().id("org-meetings-loading").py_8().text_center().text_color(muted).child(tr(cx, "common:servers.loading")).test_support().into_any_element(),
            Some(list) if list.is_empty() => div().id("org-meetings-none").px_3().py_8().rounded(px(12.)).border_1().border_dashed().border_color(border).text_center().text_color(muted).child(tr(cx, "org:nothingScheduled")).test_support().into_any_element(),
            Some(list) => div().id("org-meetings").flex().flex_col().gap_3().children(list.iter().map(|meeting| self.render_meeting(meeting, manage, team_zone.as_deref(), cx))).test_support().into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            // Lo que hay que saber antes de crear una, no después.
            .child(div().max_w(px(660.)).leading_xs().text_color(muted).child(tr(cx, "org:meetingsExplain")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().font_weight(FontWeight::MEDIUM).child(tr(cx, "org:recurringMeetings")))
                    .child(div().flex_1())
                    .when(manage && self.meetings.editor.is_none(), |this| {
                        this.child(Button::new("org-meeting-new").outline().small().icon(Icon::new(Lucide::Plus)).label(tr(cx, "org:new")).on_click(cx.listener(|this, _, window, cx| this.open_editor(None, window, cx))))
                    }),
            )
            .children(editor)
            .child(body)
            .into_any_element()
    }
}
