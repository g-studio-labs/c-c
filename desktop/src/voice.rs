//! La voz en pantalla: quién anda por cada canal, la puerta de la cabecera,
//! la sala y la llamada minimizada.
//!
//! Qué se pinta lo decide el core (`cac_core::voice`): quién cuenta, qué
//! dice la cabecera, si un fallo es de este canal. Aquí se copia cómo se ve
//! la app de Tauri (`ChannelsSection.tsx`, `VoiceBar.tsx`, `VoiceStage.tsx`,
//! `VoiceTile.tsx`, `VoiceControls.tsx`, `VoiceMini.tsx`) y se le pasa cada
//! gesto.
//!
//! El audio no pasa por aquí: lo lleva el motor (`voice_engine.rs`).

use cac_core::{person_initials, voice_error_phrase, voice_room_name, voice_status, voice_tiles, VoicePeer, VoicePhase, VoiceState, VoiceTile};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

/// La barra de mandos: el `h-21` de `VoiceControls.tsx`.
const CONTROLS_HEIGHT: f32 = 84.;
/// La cabecera de la sala: `h-13`.
const STAGE_HEADER_HEIGHT: f32 = 52.;

fn voice(cx: &App) -> &VoiceState {
    &Backend::snapshot(cx).read(cx).voice
}

/// Quién está en la voz de un canal, sin haber entrado.
pub fn occupants<'a>(space_id: &str, cx: &'a App) -> &'a [VoicePeer] {
    voice(cx).occupancy.get(space_id).map(Vec::as_slice).unwrap_or(&[])
}

fn peer_name(peer: &VoicePeer) -> String {
    if peer.name.is_empty() { peer.identity.clone() } else { peer.name.clone() }
}

/// Una cara redonda con iniciales (`grid size-5 rounded-full bg-accent
/// text-[10px] font-bold text-accent-foreground`).
fn face(initials: String, size: f32, text: f32, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .size(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.accent)
        .text_size(px(text))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.accent_foreground)
        .child(initials)
}

/// Junto al nombre de un canal: el altavoz verde con cuántos hay dentro, y
/// sus nombres al pasar el ratón. Va antes del contador de no leídos: una
/// conversación en curso es más urgente que un mensaje de ayer.
pub fn occupancy_count(space_id: &str, cx: &App) -> Option<AnyElement> {
    let peers = occupants(space_id, cx);
    if peers.is_empty() {
        return None;
    }
    let names: Vec<String> = peers.iter().map(peer_name).collect();
    let label = cac_core::text_with(locale(cx), "nav:comms.inVoice", &[("names", &names.join(", "))]);
    let success = cx.theme().success;
    Some(
        div()
            .id(SharedString::from(format!("comms-channel-{space_id}-voice")))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .text_size(px(10.))
            .text_color(success)
            .tooltip({
                let label = label.clone();
                move |window, cx| Tooltip::new(label.clone()).build(window, cx)
            })
            .aria_label(label)
            .child(Icon::new(Lucide::Volume2).size_3())
            .child(peers.len().to_string())
            .test_support()
            .into_any_element(),
    )
}

/// Debajo de un canal, quién está hablando ahí, con nombre y cara, como
/// cuelgan en Discord: el número dice «hay dos»; esto dice «son Marta y
/// Luis», que es con lo que uno decide si entrar.
pub fn occupancy_list(space_id: &str, cx: &App) -> Option<AnyElement> {
    let peers = occupants(space_id, cx);
    if peers.is_empty() {
        return None;
    }
    let foreground = cx.theme().foreground;
    Some(
        div()
            .id(SharedString::from(format!("comms-channel-{space_id}-voice-people")))
            .flex()
            .flex_col()
            .gap(px(2.))
            .py(px(2.))
            .pl_7()
            .children(peers.iter().map(|peer| {
                let name = peer_name(peer);
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py(px(2.))
                    .child(face(person_initials(&name), 20., 10., cx))
                    .child(div().min_w_0().truncate().leading_xs().text_color(foreground).child(name))
            }))
            .test_support()
            .into_any_element(),
    )
}

fn join_channel(space_id: String, cx: &App) {
    Backend::run(cx, move |core| async move { core.voice().join_channel(&space_id, None, None).await });
}

/// En la cabecera de un canal: entrar a su voz, o volver a la llamada que ya
/// tienes ahí con el silencio a mano (es lo único que se pide con prisa
/// desde fuera de la llamada).
pub fn voice_bar(space_id: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (success, danger, muted, foreground, accent, border, card) =
        (theme.success, theme.danger, theme.muted_foreground, theme.foreground, theme.accent, theme.border, theme.group_box);
    let state = voice(cx).clone();
    if state.in_channel(space_id) {
        let mic = state.mic;
        let mute_label = tr(cx, if mic { "common:last.mute" } else { "common:last.unmute" });
        return div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("voice-back")
                    .h_8()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px(px(14.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(success.opacity(0.4))
                    .bg(success.opacity(0.1))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(success)
                    .cursor_pointer()
                    .on_click(|_, _, cx| Backend::core(cx).voice().open_stage())
                    .aria_label(tr(cx, "common:last.backToCall"))
                    .child(div().size_1p5().rounded_full().bg(success))
                    .child(tr(cx, "common:last.backToCall"))
                    .test_support(),
            )
            .child(
                div()
                    .id("voice-bar-mic")
                    .size_8()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if mic { border } else { danger.opacity(0.4) })
                    .bg(card)
                    .text_color(if mic { foreground } else { danger })
                    .hover(move |style| style.bg(accent))
                    .cursor_pointer()
                    .on_click(|_, _, cx| Backend::core(cx).voice().toggle_mic())
                    .tooltip({
                        let label = mute_label.clone();
                        move |window, cx| Tooltip::new(label.clone()).build(window, cx)
                    })
                    .aria_label(mute_label)
                    .aria_selected(!mic)
                    .child(Icon::new(if mic { Lucide::Mic } else { Lucide::MicOff }).size_3p5())
                    .test_support(),
            )
            .into_any_element();
    }
    // Sólo el fallo de **este** canal: uno de otra llamada pintaría de rojo
    // el botón de todos.
    let failure = state.error.as_ref().filter(|error| error.space_id.as_deref() == Some(space_id)).map(|error| voice_error_phrase(error, locale(cx)));
    let inside = occupants(space_id, cx).to_vec();
    let joining = state.phase == VoicePhase::Joining;
    let label = failure.clone().unwrap_or_else(|| tr(cx, "common:last.joinVoiceChannel"));
    let target = space_id.to_string();
    let lead: AnyElement = if joining {
        Spinner::new().color(muted).with_size(px(14.)).into_any_element()
    } else if !inside.is_empty() {
        div()
            .flex()
            .items_center()
            .children(inside.iter().take(3).enumerate().map(|(index, peer)| {
                face(person_initials(&peer_name(peer)), 20., 10., cx).when(index > 0, |this| this.ml(px(-6.)).border_1().border_color(theme.background))
            }))
            .into_any_element()
    } else {
        Icon::new(Lucide::Volume2).size_3p5().into_any_element()
    };
    div()
        .id("voice-join")
        .h_8()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .pl_2()
        .pr_3()
        .rounded(px(8.))
        .border_1()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .map(|this| {
            if failure.is_some() {
                this.border_color(danger.opacity(0.4)).text_color(danger)
            } else if !inside.is_empty() {
                this.border_color(success.opacity(0.4)).bg(success.opacity(0.1)).text_color(success)
            } else {
                this.border_color(border).text_color(muted).hover(move |style| style.bg(accent).text_color(foreground))
            }
        })
        .when(!joining, |this| this.cursor_pointer().on_click(move |_, _, cx| join_channel(target.clone(), cx)))
        .tooltip({
            let label = label.clone();
            move |window, cx| Tooltip::new(label.clone()).build(window, cx)
        })
        .aria_label(label)
        .child(lead)
        .child(tr(cx, "common:last.joinVoice"))
        .test_support()
        .into_any_element()
}

/// Encima de lo que se escribe, en un canal donde se está hablando y tú no:
/// sin esto te enteras de la conversación en curso media hora tarde. Se va
/// en cuanto entras.
pub fn in_call_banner(space_id: &str, cx: &App) -> Option<AnyElement> {
    let peers = occupants(space_id, cx);
    if peers.is_empty() || voice(cx).in_channel(space_id) {
        return None;
    }
    let theme = cx.theme();
    let (success, muted) = (theme.success, theme.muted_foreground);
    let names: Vec<String> = peers.iter().map(peer_name).collect();
    let people = cac_core::join_names(&names, locale(cx));
    let count = names.len() as i64;
    let phrase = cac_core::text_with(locale(cx), &format!("common:voice.inChannel_{}", if count == 1 { "one" } else { "other" }), &[("people", &people), ("count", &count.to_string())]);
    let target = space_id.to_string();
    Some(
        div()
            .id("voice-in-progress")
            .mx_3()
            .mb_2()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_dashed()
            .border_color(success.opacity(0.3))
            .bg(success.opacity(0.05))
            .px_3()
            .py(px(10.))
            .text_size(px(13.))
            .text_color(muted)
            .child(Icon::new(Lucide::Volume2).size(px(15.)).flex_none().text_color(success))
            .child(div().id("voice-in-progress-text").min_w_0().truncate().aria_label(phrase.clone()).child(phrase).test_support())
            .child(
                div()
                    .id("voice-in-progress-join")
                    .ml_auto()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(success)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| join_channel(target.clone(), cx))
                    .child(tr(cx, "common:voice.join"))
                    .test_support(),
            )
            .test_support()
            .into_any_element(),
    )
}

/// El chip REC: lo que se graba es la llamada entera, no un botón, y quien
/// mire la pantalla un segundo tiene que verlo sin buscarlo.
fn rec_chip(state: &VoiceState, cx: &App) -> Option<AnyElement> {
    let recording = state.recording.as_ref()?;
    let danger = cx.theme().danger;
    let by = state.people.iter().find(|peer| peer.identity == recording.by).map(peer_name);
    let label = match by {
        Some(name) => cac_core::text_with(locale(cx), "recordings:chip", &[("name", &name)]),
        None => tr(cx, "recordings:chipUnknown"),
    };
    Some(
        div()
            .id("voice-rec")
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .rounded_full()
            .border_1()
            .border_color(danger.opacity(0.4))
            .bg(danger.opacity(0.1))
            .px_2()
            .py(px(2.))
            .text_size(px(11.))
            .font_weight(FontWeight::BOLD)
            .text_color(danger)
            .tooltip({
                let label = label.clone();
                move |window, cx| Tooltip::new(label.clone()).build(window, cx)
            })
            .aria_label(label.clone())
            .child(div().size_1p5().rounded_full().bg(danger))
            .child(label)
            .test_support()
            .into_any_element(),
    )
}

/// Un botón de la cabecera de la sala (`h-8 rounded-md border bg-card px-2.5
/// text-[13px]`).
fn header_button(id: &'static str, icon: Lucide, label: String, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let (border, card, accent) = (theme.border, theme.group_box, theme.accent);
    div()
        .id(id)
        .h_8()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .rounded(px(6.))
        .border_1()
        .border_color(border)
        .bg(card)
        .px(px(10.))
        .text_size(px(13.))
        .hover(move |style| style.bg(accent))
        .cursor_pointer()
        .aria_label(label.clone())
        .child(Icon::new(icon).size(px(15.)))
        .child(label)
}

/// Un recuadro de la sala. El borde verde es el **único** indicador de que
/// alguien habla: con cuatro recuadros lo que se busca es cuál se ha
/// encendido, y el contorno es lo más rápido de leer.
fn tile(tile: &VoiceTile, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (success, border, card, danger, background) = (theme.success, theme.border, theme.group_box, theme.danger, theme.background);
    let muted_label = cac_core::text_with(locale(cx), "common:voice.isMuted", &[("name", &tile.name)]);
    div()
        .id(SharedString::from(format!("voice-tile-{}", tile.identity)))
        .relative()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .rounded(px(12.))
        .border_2()
        .border_color(if tile.speaking { success } else { border })
        .bg(card)
        .aria_label(tile.name.clone())
        .aria_selected(tile.speaking)
        .child(face(tile.initials.clone(), 84., 24., cx))
        .child(
            div()
                .absolute()
                .bottom_3()
                .left_3()
                .max_w(relative(0.7))
                .rounded_full()
                .bg(background.opacity(0.7))
                .px(px(10.))
                .py_1()
                .text_size(px(13.))
                .child(div().truncate().child(tile.name.clone())),
        )
        // El icono y no la palabra: en un recuadro pequeño «Silenciado» tapa
        // el nombre, y el micrófono tachado se entiende en cualquier idioma.
        .when(tile.muted, |this| {
            this.child(
                div()
                    .id(SharedString::from(format!("voice-tile-{}-muted", tile.identity)))
                    .absolute()
                    .bottom_3()
                    .right_3()
                    .rounded_full()
                    .bg(background.opacity(0.7))
                    .p_1p5()
                    .text_color(danger)
                    .aria_label(muted_label)
                    .child(Icon::new(Lucide::MicOff).size(px(15.)))
                    .test_support(),
            )
        })
        .test_support()
        .into_any_element()
}

/// Un mando redondo de la barra (`Round` en `VoiceControls.tsx`): rojo
/// cuando está «activo» (silenciado, sordo).
pub(crate) fn round_button(id: &'static str, icon: Lucide, label: String, active: bool, cx: &App) -> Stateful<Div> {
    round(id, icon, label, active, cx)
}

fn round(id: &'static str, icon: Lucide, label: String, active: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let (danger, border, card, foreground, accent) = (theme.danger, theme.border, theme.group_box, theme.foreground, theme.accent);
    div()
        .id(id)
        .size(px(46.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .map(|this| {
            if active {
                this.border_color(danger.opacity(0.5)).bg(danger.opacity(0.15)).text_color(danger)
            } else {
                this.border_color(border).bg(card).text_color(foreground).hover(move |style| style.bg(accent))
            }
        })
        .cursor_pointer()
        .tooltip({
            let label = label.clone();
            move |window, cx| Tooltip::new(label.clone()).build(window, cx)
        })
        .aria_label(label)
        .aria_selected(active)
        .child(Icon::new(icon).size_5())
}

/// La barra de mandos: lo tuyo (micrófono, oír, ajustes) y colgar, aparte.
fn controls(state: &VoiceState, settings: Option<AnyElement>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (border, sidebar, danger, background) = (theme.border, theme.sidebar, theme.danger, theme.background);
    let (mic, deaf) = (state.mic, state.deaf);
    div()
        .id("voice-controls")
        .h(px(CONTROLS_HEIGHT))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .border_t_1()
        .border_color(border)
        .bg(sidebar)
        .child(
            round("voice-mic", if mic { Lucide::Mic } else { Lucide::MicOff }, tr(cx, if mic { "common:voice.mute" } else { "common:voice.unmute" }), !mic, cx)
                .on_click(|_, _, cx| Backend::core(cx).voice().toggle_mic())
                .test_support(),
        )
        .child(
            round(
                "voice-deaf",
                if deaf { Lucide::HeadphoneOff } else { Lucide::Headphones },
                tr(cx, if deaf { "common:voice.undeafen" } else { "common:voice.deafen" }),
                deaf,
                cx,
            )
            .on_click(|_, _, cx| Backend::core(cx).voice().toggle_deaf())
            .test_support(),
        )
        .children(settings)
        .child(div().mx_1p5().h_7().w(px(1.)).bg(border))
        .child(
            div()
                .id("voice-leave")
                .h(px(46.))
                .flex()
                .items_center()
                .gap_2()
                .rounded_full()
                .bg(danger)
                .px_5()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(background)
                .cursor_pointer()
                .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.voice().leave().await }))
                .aria_label(tr(cx, "common:voice.leave"))
                .child(Icon::new(Lucide::PhoneOff).size_5())
                .child(tr(cx, "common:voice.leaveShort"))
                .test_support(),
        )
        .test_support()
        .into_any_element()
}

/// Lo que la sala lleva además de lo suyo: los ajustes de audio y llamar a
/// alguien, que pinta quien los tiene abiertos.
#[derive(Default)]
pub struct StageExtras {
    pub settings_button: Option<AnyElement>,
    pub settings_panel: Option<AnyElement>,
    pub invite_button: Option<AnyElement>,
    pub invite_panel: Option<AnyElement>,
    pub ring_row: Option<AnyElement>,
}

/// La sala: sustituye a la conversación mientras está abierta.
pub fn stage(extras: StageExtras, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (sidebar, border, success, muted, danger) = (theme.sidebar, theme.border, theme.success, theme.muted_foreground, theme.danger);
    let state = voice(cx).clone();
    let lang = locale(cx);
    let room = voice_room_name(&state, lang);
    let status = voice_status(&state, lang);
    let tiles = voice_tiles(&state, lang);
    let body: AnyElement = if state.phase == VoicePhase::Joining {
        let joining = cac_core::text_with(lang, "calls:joiningRoom", &[("room", &room)]);
        div()
            .id("voice-joining")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .leading_sm()
            .text_color(muted)
            .child(Spinner::new().color(muted))
            .child(joining)
            .test_support()
            .into_any_element()
    } else {
        // Solo, ocupas la sala; con más gente, de dos en dos.
        let columns = if tiles.len() > 1 { 2 } else { 1 };
        div()
            .id("voice-grid")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            // Como la rejilla de Tauri: el último de una fila impar se queda
            // en su columna, no se estira a toda la fila.
            .children(tiles.chunks(columns).map(|row| {
                let gap = columns - row.len();
                div().flex_1().min_h_0().flex().gap(px(14.)).children(row.iter().map(|one| tile(one, cx))).children((0..gap).map(|_| div().flex_1()))
            }))
            .test_support()
            .into_any_element()
    };
    let error = state.error.as_ref().map(|error| {
        let phrase = voice_error_phrase(error, lang);
        div()
            .id("voice-error")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap_2()
            .border_t_1()
            .border_color(border)
            .bg(danger.opacity(0.1))
            .px_4()
            .py_2()
            .leading_xs()
            .text_color(danger)
            .aria_label(phrase.clone())
            .child(Icon::new(Lucide::CircleAlert).size_3p5().flex_none())
            .child(phrase)
            .child(
                div()
                    .id("voice-error-dismiss")
                    .ml_1()
                    .flex_none()
                    .rounded(px(4.))
                    .p(px(2.))
                    .cursor_pointer()
                    .on_click(|_, _, cx| Backend::core(cx).voice().clear_error())
                    .aria_label(tr(cx, "common:last.dismiss"))
                    .child(Icon::new(Lucide::X).size_3p5())
                    .test_support(),
            )
            .test_support()
    });
    div()
        .id("voice-stage")
        .relative()
        .size_full()
        .min_w_0()
        .flex()
        .flex_col()
        .bg(sidebar)
        .child(
            div()
                .id("voice-stage-header")
                .h(px(STAGE_HEADER_HEIGHT))
                .flex()
                .flex_none()
                .items_center()
                .gap_3()
                .px_4()
                .border_b_1()
                .border_color(border)
                .child(Icon::new(Lucide::Volume2).size_4().flex_none().text_color(success))
                .child(div().id("voice-room").truncate().text_sm().font_weight(FontWeight::SEMIBOLD).aria_label(room.clone()).child(room).test_support())
                .child(div().id("voice-status").min_w_0().truncate().text_size(px(13.)).text_color(muted).aria_label(status.clone()).child(status).test_support())
                .children(rec_chip(&state, cx))
                .child(div().flex_1())
                .children(extras.invite_button)
                .child(
                    header_button("voice-minimize", Lucide::Minimize2, tr(cx, "calls:minimize"), cx)
                        .tooltip({
                            let label = tr(cx, "common:last.backToChannel");
                            move |window, cx| Tooltip::new(label.clone()).build(window, cx)
                        })
                        .on_click(|_, _, cx| Backend::core(cx).voice().close_stage())
                        .test_support(),
                )
                .test_support(),
        )
        .child(div().relative().flex_1().min_h_0().p_5().child(body))
        .children(error)
        .children(extras.ring_row)
        .child(controls(&state, extras.settings_button, cx))
        .children(extras.invite_panel)
        .children(extras.settings_panel)
        .test_support()
        .into_any_element()
}

/// Vuelve a la sala desde donde se esté: a comunicación, con la sala
/// delante.
fn back_to_call(cx: &App) {
    let core = Backend::core(cx);
    core.comms().enter();
    core.voice().open_stage();
}

/// La llamada minimizada, al pie del panel: lo único que te dice que tienes
/// el micrófono abierto mientras miras otra cosa. Se queda al navegar.
pub fn mini(cx: &App) -> Option<AnyElement> {
    let state = voice(cx).clone();
    if state.phase == VoicePhase::Out {
        return None;
    }
    let theme = cx.theme();
    let (success, danger, muted, foreground, border, card) = (theme.success, theme.danger, theme.muted_foreground, theme.foreground, theme.border, theme.group_box);
    let lang = locale(cx);
    let name = voice_room_name(&state, lang);
    // De qué organización es, cuando no es la de la pantalla: «general» a
    // secas no dice a cuál de tus clientes estás hablando.
    let other_org = {
        let orgs = &Backend::snapshot(cx).read(cx).orgs;
        state.org_id.as_ref().filter(|id| orgs.current_org_id.as_ref() != Some(*id)).and_then(|id| orgs.orgs.iter().find(|org| &org.id == id)).map(|org| org.name.clone())
    };
    let line = tr(cx, if state.phase == VoicePhase::Joining { "common:servers.connecting" } else { "common:servers.voiceConnected" });
    let mic = state.mic;
    Some(
        div()
            .id("voice-mini")
            .m_2()
            .flex()
            .flex_none()
            .flex_col()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(success.opacity(0.35))
            .bg(success.opacity(0.07))
            .p_2()
            .px(px(10.))
            .child(
                div()
                    .id("voice-mini-back")
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .on_click(|_, _, cx| back_to_call(cx))
                    .aria_label(tr(cx, "common:servers.backToCall"))
                    .child(div().size_1p5().flex_none().rounded_full().bg(success))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().id("voice-mini-name").truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(success).aria_label(name.clone()).child(name).test_support())
                            .children(other_org.map(|org| {
                                let said = cac_core::text_with(lang, "common:servers.inOrg", &[("org", &org)]);
                                div().id("voice-mini-org").truncate().leading_xs().text_color(foreground.opacity(0.8)).aria_label(said.clone()).child(said).test_support()
                            }))
                            .child(div().leading_xs().text_color(muted).child(line))
                            .children(rec_chip(&state, cx).map(|chip| div().mt_1().flex().child(chip))),
                    )
                    .test_support(),
            )
            .child(
                div()
                    .flex()
                    .gap_1p5()
                    .child(
                        div()
                            .id("voice-mini-mic")
                            .h_7()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_1p5()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(if mic { border } else { danger.opacity(0.4) })
                            .bg(card)
                            .leading_xs()
                            .text_color(if mic { foreground } else { danger })
                            .cursor_pointer()
                            .on_click(|_, _, cx| Backend::core(cx).voice().toggle_mic())
                            .aria_selected(!mic)
                            .child(Icon::new(if mic { Lucide::Mic } else { Lucide::MicOff }).size_3p5())
                            .child(tr(cx, if mic { "common:servers.mute" } else { "common:servers.unmute" }))
                            .test_support(),
                    )
                    .child(
                        div()
                            .id("voice-mini-leave")
                            .h_7()
                            .w(px(34.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(danger.opacity(0.4))
                            .bg(danger.opacity(0.1))
                            .text_color(danger)
                            .cursor_pointer()
                            .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.voice().leave().await }))
                            .tooltip({
                                let label = tr(cx, "common:servers.disconnect");
                                move |window, cx| Tooltip::new(label.clone()).build(window, cx)
                            })
                            .aria_label(tr(cx, "common:servers.disconnect"))
                            .child(Icon::new(Lucide::PhoneOff).size_3p5())
                            .test_support(),
                    ),
            )
            .test_support()
            .into_any_element(),
    )
}

/// Con el panel plegado no cabe la caja, pero desaparecer no es una opción:
/// plegar el panel es justo lo que hace quien va a olvidarse de que tiene el
/// micrófono abierto. Queda el punto verde en el riel.
pub fn mini_dot(cx: &App) -> Option<AnyElement> {
    let state = voice(cx);
    if state.phase == VoicePhase::Out {
        return None;
    }
    let success = cx.theme().success;
    let name = voice_room_name(state, locale(cx));
    let label = format!("{} · {name}", tr(cx, "common:servers.voiceConnected"));
    Some(
        div()
            .id("voice-mini-dot")
            .my_1()
            .size_8()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .border_1()
            .border_color(success.opacity(0.35))
            .bg(success.opacity(0.07))
            .cursor_pointer()
            .on_click(|_, _, cx| back_to_call(cx))
            .tooltip({
                let label = label.clone();
                move |window, cx| Tooltip::new(label.clone()).build(window, cx)
            })
            .aria_label(tr(cx, "common:servers.backToCall"))
            .child(div().size_2().rounded_full().bg(success))
            .test_support()
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer};
    use crate::workspace::Workspace;
    use cac_core::{VoiceEngine, VoiceFuture};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::Mock;

    /// Un motor que apunta lo que se le pide y contesta en el acto.
    #[derive(Default)]
    pub(crate) struct FakeEngine {
        pub calls: Mutex<Vec<String>>,
        pub fail: Mutex<Option<String>>,
    }

    impl VoiceEngine for FakeEngine {
        fn join(&self, _url: String, token: String, _noise: bool) -> VoiceFuture<Result<String, String>> {
            self.calls.lock().unwrap().push(format!("join {token}"));
            let fail = self.fail.lock().unwrap().clone();
            Box::pin(async move { fail.map_or(Ok("u1".to_string()), Err) })
        }
        fn leave(&self) -> VoiceFuture<()> {
            self.calls.lock().unwrap().push("leave".into());
            Box::pin(async {})
        }
        fn set_mic(&self, on: bool) {
            self.calls.lock().unwrap().push(format!("mic {on}"));
        }
        fn set_deaf(&self, on: bool) {
            self.calls.lock().unwrap().push(format!("deaf {on}"));
        }
        fn set_noise_suppression(&self, _on: bool) {}
    }

    fn get(route: &str) -> wiremock::MockBuilder {
        Mock::given(method("GET")).and(path(route.to_string()))
    }

    fn space(id: &str, name: &str, kind: Option<&str>) -> Value {
        json!({ "id": id, "orgId": "o1", "name": name, "color": "#888", "kind": kind, "folders": [],
            "lists": [{ "id": format!("l-{id}"), "name": "Lista", "openCount": 1, "taskCount": 1 }], "people": [] })
    }

    /// Una organización con la sala general y «Producto» (`a`), donde Luis
    /// está hablando.
    fn server() -> FakeServer {
        let server = FakeServer::start();
        let org = json!({ "id": "o1", "name": "G Studio", "slug": "g", "role": "member", "memberCount": 3, "createdAt": "2026-06-09T12:00:00Z", "timezone": "" });
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org]))));
        server.mount(get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(json!([space("a", "Producto", None), space("g", "General", Some("general"))]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/unread").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/latest").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/chat/voice-presence").respond_with(ok(json!({ "a": [{ "identity": "u2", "name": "Luis Mora" }] }))));
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/task-spaces/a/voice/token"))
                .respond_with(ok(json!({ "url": "wss://sfu", "token": "t-a", "room": "voice:a", "orgId": "o1", "spaceName": "Producto" }))),
        );
        server
    }

    fn open(server: &FakeServer, cx: &mut TestAppContext) -> (AnyWindowHandle, Arc<FakeEngine>) {
        server.install_with(cx, |config| config.voice_presence_every = Duration::from_millis(100));
        let engine = Arc::new(FakeEngine::default());
        cx.update(|cx| Backend::core(cx).set_voice_engine(engine.clone()));
        sign_in(cx);
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(760.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window").0
        });
        (window, engine)
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

    fn stop(cx: &mut TestAppContext) {
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Entra a comunicación y pone delante «Producto».
    fn to_product(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "rail-comms", cx);
        wait_until(cx, "la lista de canales", |cx| on_screen(window, "comms-channel-a", cx));
        click(window, "comms-channel-a", cx);
        wait_until(cx, "la cabecera de Producto", |cx| label(window, "conversation-title", cx).contains("Producto"));
    }

    /// Quién anda por la voz de un canal se ve sin entrar: en su fila, con
    /// su nombre debajo, en la puerta de la cabecera y encima de lo que se
    /// escribe. Y sólo se pregunta mientras se mira comunicación.
    ///
    /// Roturas que atrapa: no preguntar nunca, o seguir preguntando en
    /// «Trabajo»; perder los nombres; el aviso de «están hablando» sin el
    /// verbo en plural correcto.
    #[gpui_kit::test]
    fn a_channel_shows_who_is_talking_in_it(cx: &mut TestAppContext) {
        let server = server();
        let (window, _) = open(&server, cx);
        to_product(window, cx);
        wait_until(cx, "Luis, en la fila", |cx| on_screen(window, "comms-channel-a-voice", cx));
        assert_eq!(label(window, "comms-channel-a-voice", cx), "En la voz: Luis Mora");
        assert!(on_screen(window, "comms-channel-a-voice-people", cx));
        assert!(on_screen(window, "voice-join", cx));
        assert_eq!(label(window, "voice-in-progress-text", cx), "Luis Mora está hablando en este canal.");

        click(window, "rail-work", cx);
        wait_until(cx, "de vuelta en Trabajo", |cx| !on_screen(window, "comms-channel-a", cx));
        std::thread::sleep(Duration::from_millis(150));
        cx.run_until_parked();
        let asked = server.hits("/api/v1/chat/voice-presence");
        std::thread::sleep(Duration::from_millis(350));
        cx.run_until_parked();
        assert_eq!(server.hits("/api/v1/chat/voice-presence"), asked, "fuera de comunicación no se pregunta");
        stop(cx);
    }

    /// Entrar abre la sala en lugar de la conversación y pliega el panel;
    /// silenciarse se ve en tu recuadro; minimizar devuelve el canal y el
    /// panel con la llamada viva al pie, que sigue al ir a «Trabajo» y desde
    /// la que se vuelve; colgar lo quita todo.
    ///
    /// Roturas que atrapa: la sala que no se abre o no sustituye a la
    /// conversación; el panel que no se pliega o no vuelve; un botón de
    /// silencio que no llega al motor; minimizar que cuelga; la mini que
    /// desaparece al navegar; colgar sin avisar al motor.
    #[gpui_kit::test]
    fn joining_minimizing_and_leaving(cx: &mut TestAppContext) {
        let server = server();
        let (window, engine) = open(&server, cx);
        to_product(window, cx);
        assert!(on_screen(window, "comms-panel", cx), "el panel, fijo");

        click(window, "voice-join", cx);
        wait_until(cx, "dentro de la sala", |cx| on_screen(window, "voice-tile-u1", cx));
        assert!(!on_screen(window, "conversation", cx), "la sala ocupa el sitio de la conversación");
        assert!(!on_screen(window, "comms-panel", cx), "el panel se pliega");
        assert_eq!(label(window, "voice-room", cx), "Producto");
        assert_eq!(engine.calls.lock().unwrap().clone(), ["join t-a"]);

        click(window, "voice-mic", cx);
        wait_until(cx, "tu recuadro dice que estás mudo", |cx| on_screen(window, "voice-tile-u1-muted", cx));
        assert_eq!(engine.calls.lock().unwrap().last().unwrap(), "mic false");

        click(window, "voice-minimize", cx);
        wait_until(cx, "de vuelta al canal", |cx| on_screen(window, "conversation", cx));
        assert!(on_screen(window, "comms-panel", cx), "el panel vuelve");
        assert!(on_screen(window, "voice-mini", cx) && on_screen(window, "voice-back", cx));
        assert!(!engine.calls.lock().unwrap().contains(&"leave".to_string()), "minimizar no cuelga");

        click(window, "rail-work", cx);
        wait_until(cx, "en Trabajo", |cx| on_screen(window, "tree-a", cx));
        assert!(on_screen(window, "voice-mini", cx), "la llamada sigue a la vista");
        click(window, "voice-mini-back", cx);
        wait_until(cx, "de vuelta en la sala", |cx| on_screen(window, "voice-stage", cx));

        click(window, "voice-leave", cx);
        wait_until(cx, "fuera", |cx| !on_screen(window, "voice-stage", cx) && !on_screen(window, "voice-mini", cx));
        assert_eq!(engine.calls.lock().unwrap().last().unwrap(), "leave");
        assert!(on_screen(window, "comms-panel", cx), "y el panel, como estaba");
        stop(cx);
    }

    /// Si durante la llamada se toca el panel a mano, cerrar la sala lo deja
    /// como lo dejaste.
    /// Rotura que atrapa: devolver el panel aunque lo hayas plegado tú.
    #[gpui_kit::test]
    fn the_panel_you_touched_stays_as_you_left_it(cx: &mut TestAppContext) {
        let server = server();
        let (window, _) = open(&server, cx);
        to_product(window, cx);
        click(window, "voice-join", cx);
        wait_until(cx, "dentro", |cx| on_screen(window, "voice-stage", cx));
        // Fijarlo y volver a plegarlo, desde el riel.
        click(window, "rail-comms", cx);
        wait_until(cx, "fijado", |cx| on_screen(window, "comms-panel", cx));
        click(window, "rail-comms", cx);
        click(window, "voice-minimize", cx);
        wait_until(cx, "el canal", |cx| on_screen(window, "conversation", cx));
        assert!(!on_screen(window, "comms-panel", cx), "plegado, como lo dejaste");
        assert!(on_screen(window, "voice-mini-dot", cx), "con el punto verde en el riel");
        stop(cx);
    }

    fn ring_player(cx: &mut TestAppContext) -> Arc<crate::sound::tests::Recording> {
        let played = Arc::new(crate::sound::tests::Recording::default());
        cx.update(|cx| crate::sound::install(played.clone(), cx));
        played
    }

    fn requests(server: &FakeServer, verb: &str, route: &str) -> usize {
        server.requests().iter().filter(|request| request.method.as_str() == verb && request.url.path() == route).count()
    }

    /// Llamar a alguien desde la sala: «Invitar» ofrece a quien no está,
    /// llamar suena el tono de llamada con «Llamando a…», y cancelar le calla
    /// el teléfono y el tono.
    ///
    /// Roturas que atrapa: ofrecer a quien ya está dentro; no llamar; no sonar
    /// el tono de llamada, o seguir sonando al cancelar; cancelar sin avisar
    /// al servidor.
    #[gpui_kit::test]
    fn ringing_someone_from_the_room(cx: &mut TestAppContext) {
        let server = server();
        let people = json!([{ "id": "u1", "username": "ana" }, { "id": "u2", "username": "luis", "name": "Luis Mora" }, { "id": "u3", "username": "eva", "name": "Eva Paz" }]);
        server.mount(get("/api/v1/users/search").respond_with(ok(people)));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/task-spaces/a/voice/ring")).respond_with(ok(json!({ "ringId": "r1" }))));
        server.mount(Mock::given(method("DELETE")).and(path("/api/v1/task-spaces/a/voice/ring/u3")).respond_with(ok(json!(null))));
        let (window, _) = open(&server, cx);
        let played = ring_player(cx);
        to_product(window, cx);
        click(window, "voice-join", cx);
        wait_until(cx, "dentro", |cx| on_screen(window, "voice-stage", cx));
        cx.update(|cx| Backend::core(cx).voice().engine_event(cac_core::VoiceEvent::Joined { identity: "u2".into(), name: "Luis Mora".into() }));

        click(window, "voice-invite", cx);
        wait_until(cx, "la gente", |cx| on_screen(window, "voice-invite-u3", cx));
        assert!(!on_screen(window, "voice-invite-u2", cx), "Luis ya está dentro");
        assert!(!on_screen(window, "voice-invite-u1", cx), "ni tú");
        click(window, "voice-invite-u3", cx);
        wait_until(cx, "llamando", |cx| label(window, "voice-ring-title", cx) == "Llamando a Eva Paz");
        assert!(label(window, "voice-ring-detail", cx).starts_with("Llamando · 0:0"));
        assert_eq!(requests(&server, "POST", "/api/v1/task-spaces/a/voice/ring"), 1);
        assert_eq!(played.1.lock().unwrap().last().copied().flatten(), Some(cac_core::RingTone::Outgoing));

        click(window, "voice-ring-cancel", cx);
        wait_until(cx, "sin fila", |cx| !on_screen(window, "voice-ring-row", cx));
        wait_until(cx, "le calla el teléfono", |_| requests(&server, "DELETE", "/api/v1/task-spaces/a/voice/ring/u3") == 1);
        assert_eq!(played.1.lock().unwrap().last().copied(), Some(None), "y el tono para");
        stop(cx);
    }

    fn ring_from_luis(cx: &mut TestAppContext) {
        let expires = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
        let data = json!({ "ringId": format!("r-{expires}"), "spaceId": "a", "orgId": "o1", "spaceName": "Producto", "from": { "id": "u2", "name": "Luis Mora" }, "expiresAt": expires });
        cx.update(|cx| Backend::run(cx, move |core| async move { core.stream().deliver(cac_core::ServerEvent { event: "voice.ring".into(), data: data.to_string() }) }));
    }

    /// Te llaman: la tarjeta sale encima de todo aunque estés en «Trabajo»,
    /// suena el tono de te llaman y avisa al sistema una vez; rechazar la
    /// quita, calla el tono y se lo dice a quien llamaba; aceptar entra.
    ///
    /// Roturas que atrapa: la tarjeta sólo en comunicación; sin tono, o un
    /// tono que no para; un aviso del sistema por pintado; rechazar sin
    /// avisar; aceptar sin entrar.
    #[gpui_kit::test]
    fn being_rung(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("DELETE")).and(path("/api/v1/task-spaces/a/voice/ring/u2")).respond_with(ok(json!(null))));
        let (window, engine) = open(&server, cx);
        let played = ring_player(cx);
        wait_until(cx, "el árbol", |cx| on_screen(window, "tree-a", cx));

        ring_from_luis(cx);
        wait_until(cx, "la tarjeta", |cx| on_screen(window, "voice-incoming", cx));
        assert_eq!(label(window, "voice-incoming-name", cx), "Luis Mora");
        assert_eq!(label(window, "voice-incoming-detail", cx), "Te llama a #Producto");
        assert_eq!(played.1.lock().unwrap().last().copied().flatten(), Some(cac_core::RingTone::Incoming));
        for _ in 0..3 {
            cx.run_until_parked();
            let _ = on_screen(window, "voice-incoming", cx);
        }
        let shown = cx.shown_system_notifications();
        assert_eq!(shown.iter().filter(|notice| notice.title.as_ref() == "Luis Mora te llama").count(), 1, "un aviso, no uno por pintado");

        click(window, "voice-incoming-decline", cx);
        wait_until(cx, "sin tarjeta", |cx| !on_screen(window, "voice-incoming", cx));
        wait_until(cx, "se lo dice a Luis", |_| requests(&server, "DELETE", "/api/v1/task-spaces/a/voice/ring/u2") == 1);
        assert_eq!(played.1.lock().unwrap().last().copied(), Some(None));

        ring_from_luis(cx);
        wait_until(cx, "otra vez", |cx| on_screen(window, "voice-incoming-join", cx));
        click(window, "voice-incoming-join", cx);
        wait_until(cx, "en la sala", |cx| on_screen(window, "voice-stage", cx));
        assert_eq!(engine.calls.lock().unwrap().clone(), ["join t-a"]);
        stop(cx);
    }

    /// Un fallo al entrar se dice en la puerta de ese canal, en castellano.
    /// Rotura que atrapa: un botón que no hace nada y no dice por qué.
    #[gpui_kit::test]
    fn a_failure_to_join_is_said_at_the_door(cx: &mut TestAppContext) {
        let server = server();
        let (window, engine) = open(&server, cx);
        *engine.fail.lock().unwrap() = Some("voice-no-mic".into());
        to_product(window, cx);
        click(window, "voice-join", cx);
        wait_until(cx, "el fallo", |cx| label(window, "voice-join", cx) == "No se encontró ningún micrófono.");
        assert!(on_screen(window, "conversation", cx), "la sala se cierra");
        stop(cx);
    }
}
