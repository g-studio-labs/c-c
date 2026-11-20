//! La voz en pantalla: quién anda por cada canal, la puerta de la cabecera,
//! la sala y la llamada minimizada.
//!
//! Qué se pinta lo decide el core (`cac_core::voice`): quién cuenta, qué
//! dice la cabecera, si un fallo es de este canal. Aquí se copia cómo se ve
//! la app de Tauri (`ChannelsSection.tsx`, `VoiceBar.tsx`, `VoiceStage.tsx`,
//! `VoiceTile.tsx`, `VoiceControls.tsx`, `VoiceMini.tsx`) y se le pasa cada
//! gesto.
//!
//! El audio no pasa por aquí: lo lleva el motor (`voice_engine.rs`). El
//! vídeo, tampoco: cada vídeo es un `voice_video::VideoTile` que toma sus
//! fotogramas del motor; aquí sólo se decide dónde va.

use cac_core::{person_initials, voice_error_phrase, voice_layout, voice_room_name, voice_status, Publishing, StageItem, VideoSource, VoiceLayout, VoicePeer, VoicePhase, VoiceState, VoiceTile};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;
use crate::voice_video::{VideoTile, VideoWall};

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

/// El vídeo de alguien, si está encendido y hay quien lo pinte.
fn video_of(wall: Option<&Entity<VideoWall>>, identity: &str, source: VideoSource, cx: &App) -> Option<Entity<VideoTile>> {
    wall.and_then(|wall| wall.read(cx).tile(identity, source))
}

/// Un recuadro de la sala. El borde verde es el **único** indicador de que
/// alguien habla: con cuatro recuadros lo que se busca es cuál se ha
/// encendido, y el contorno es lo más rápido de leer.
///
/// Con la cámara encendida, el vídeo tapa la cara; hasta el primer fotograma
/// se ve la cara (no hay «cargando»: en Tauri tampoco). Pulsarlo lo lleva al
/// escenario. `compact` es el de la tira del escenario (`h-30`).
fn tile(tile: &VoiceTile, video: Option<Entity<VideoTile>>, compact: bool, cx: &App) -> AnyElement {
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
        .when(compact, |this| this.flex_none().h(px(120.)))
        .child(if compact { face(tile.initials.clone(), 44., 14., cx) } else { face(tile.initials.clone(), 84., 24., cx) })
        .when_some(video.filter(|_| tile.camera), |this, video| {
            let identity = tile.identity.clone();
            let label = tr(cx, "calls:toStage");
            this.child(div().absolute().inset_0().child(video))
                .cursor_pointer()
                .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                .on_click(move |_, _, cx| Backend::core(cx).voice().toggle_focus(&identity, VideoSource::Camera))
        })
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

/// De qué color se enciende un mando: rojo lo que apaga algo tuyo
/// (silencio, sordera), cian lo que saca algo tuyo a la sala (cámara,
/// pantalla). El `tone` de `Round` en `VoiceControls.tsx`.
#[derive(Clone, Copy)]
enum Tone {
    Danger,
    Primary,
}

/// Un mando redondo de la barra (`Round` en `VoiceControls.tsx`): rojo
/// cuando está «activo» (silenciado, sordo).
pub(crate) fn round_button(id: &'static str, icon: Lucide, label: String, active: bool, cx: &App) -> Stateful<Div> {
    round(id, icon, label, active, cx)
}

fn round(id: &'static str, icon: Lucide, label: String, active: bool, cx: &App) -> Stateful<Div> {
    round_with(id, Icon::new(icon).size_5().into_any_element(), label, active, Tone::Danger, cx)
}

fn round_with(id: &'static str, content: AnyElement, label: String, active: bool, tone: Tone, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let (border, card, foreground, accent) = (theme.border, theme.group_box, theme.foreground, theme.accent);
    let lit = match tone {
        Tone::Danger => theme.danger,
        Tone::Primary => theme.primary,
    };
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
                this.border_color(lit.opacity(0.5)).bg(lit.opacity(0.15)).text_color(lit)
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
        .child(content)
}

/// Lo que sacas a la sala (cámara, pantalla): encendido en cian cuando ya
/// sale, y girando mientras el motor lo arranca, que abrir una cámara o
/// esperar al diálogo de la pantalla tarda. Pulsarlo mientras gira lo
/// cancela, así que ya dice «apagar».
fn publish_button(id: &'static str, publishing: Publishing, icon: Lucide, turn_on: &str, turn_off: &str, cx: &App) -> Stateful<Div> {
    let content = if publishing == Publishing::Starting {
        div().id(SharedString::from(format!("{id}-starting"))).child(Spinner::new().with_size(px(20.))).test_support().into_any_element()
    } else {
        Icon::new(icon).size_5().into_any_element()
    };
    let label = tr(cx, if publishing.is_off() { turn_on } else { turn_off });
    round_with(id, content, label, publishing == Publishing::On, Tone::Primary, cx)
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
        .child(
            publish_button("voice-camera", state.camera, if state.camera.is_off() { Lucide::VideoOff } else { Lucide::Video }, "common:voice.cameraOn", "common:voice.cameraOff", cx)
                .on_click(|_, _, cx| Backend::core(cx).voice().toggle_camera())
                .test_support(),
        )
        .child(
            publish_button("voice-share", state.sharing, Lucide::MonitorUp, "common:voice.share", "common:voice.stopSharing", cx)
                .on_click(|_, _, cx| Backend::core(cx).voice().toggle_share())
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
    /// Qué pantalla o ventana compartir (`voice_screen_picker.rs`).
    pub screen_picker: Option<AnyElement>,
    /// Los vídeos de la sala (`voice_video.rs`).
    pub video: Option<Entity<VideoWall>>,
}

/// La sala: sustituye a la conversación mientras está abierta.
pub fn stage(extras: StageExtras, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (sidebar, border, success, muted, danger) = (theme.sidebar, theme.border, theme.success, theme.muted_foreground, theme.danger);
    let state = voice(cx).clone();
    let lang = locale(cx);
    let room = voice_room_name(&state, lang);
    let status = voice_status(&state, lang);
    let wall = extras.video.as_ref();
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
        match voice_layout(&state, lang) {
            VoiceLayout::Grid { tiles } => {
                // Solo, ocupas la sala; con más gente, de dos en dos.
                let columns = if tiles.len() > 1 { 2 } else { 1 };
                div()
                    .id("voice-grid")
                    .size_full()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    // Como la rejilla de Tauri: el último de una fila impar se
                    // queda en su columna, no se estira a toda la fila.
                    .children(tiles.chunks(columns).map(|row| {
                        let gap = columns - row.len();
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .gap(px(14.))
                            .children(row.iter().map(|one| tile(one, video_of(wall, &one.identity, VideoSource::Camera, cx), false, cx)))
                            .children((0..gap).map(|_| div().flex_1()))
                    }))
                    .test_support()
                    .into_any_element()
            }
            VoiceLayout::Stage { on_stage, strip, faces_out } => {
                let big = stage_main(&on_stage, &strip, faces_out, state.focus.is_some(), wall, cx);
                div()
                    .id("voice-stage-layout")
                    .size_full()
                    .flex()
                    .gap(px(14.))
                    .child(big)
                    .when(faces_out, |this| {
                        this.child(
                            div()
                                .id("voice-strip")
                                .w(px(200.))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap(px(10.))
                                .overflow_y_scroll()
                                // Quien está en grande con su cámara sale aquí con
                                // su cara: una vista no se pinta en dos sitios.
                                .children(strip.iter().map(|one| {
                                    let in_big = on_stage.source == VideoSource::Camera && on_stage.identity == one.identity;
                                    let video = if in_big { None } else { video_of(wall, &one.identity, VideoSource::Camera, cx) };
                                    tile(one, video, true, cx)
                                }))
                                .test_support(),
                        )
                    })
                    .test_support()
                    .into_any_element()
            }
        }
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
        .children(extras.screen_picker)
        .test_support()
        .into_any_element()
}

/// Lo grande del escenario: la pantalla compartida o lo que se pulsó.
///
/// Encima: arriba a la izquierda, pantalla completa; arriba a la derecha, las
/// caras (la píldora de iniciales de Tauri: quien habla en verde, los demás
/// apagados; pulsarla las saca a la tira); abajo, de quién es.
fn stage_main(on_stage: &StageItem, strip: &[VoiceTile], faces_out: bool, by_hand: bool, wall: Option<&Entity<VideoWall>>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (primary, background, success, danger) = (theme.primary, theme.background, theme.success, theme.danger);
    let lang = locale(cx);
    let caption = match (on_stage.source, on_stage.me) {
        (VideoSource::Screen, true) => tr(cx, "common:last.youAreSharing"),
        (VideoSource::Screen, false) => cac_core::text_with(lang, "calls:isSharing", &[("name", &on_stage.name)]),
        (VideoSource::Camera, _) => on_stage.name.clone(),
    };
    let picture: AnyElement = match video_of(wall, &on_stage.identity, on_stage.source, cx) {
        Some(video) => video.into_any_element(),
        None => face(person_initials(&on_stage.name), 84., 24., cx).into_any_element(),
    };
    let faces = (!faces_out).then(|| {
        div()
            .id("voice-faces")
            .absolute()
            .top_3()
            .right_3()
            .flex()
            .gap_1()
            .rounded_full()
            .bg(background.opacity(0.7))
            .p_1()
            .cursor_pointer()
            // Que no le llegue al escenario: con una cámara en grande, ese
            // clic la devolvería a la rejilla.
            .on_click(|_, _, cx| {
                cx.stop_propagation();
                Backend::core(cx).voice().show_faces();
            })
            .aria_label(tr(cx, "common:last.showParticipants"))
            .tooltip({
                let label = tr(cx, "common:last.showParticipants");
                move |window, cx| Tooltip::new(label.clone()).build(window, cx)
            })
            .children(strip.iter().map(|one| {
                div()
                    .rounded_full()
                    .map(|this| if one.speaking { this.border_2().border_color(success) } else { this.opacity(0.4) })
                    .child(face(one.initials.clone(), 28., 11., cx))
            }))
            .test_support()
    });
    let (identity, source) = (on_stage.identity.clone(), on_stage.source);
    div()
        .id("voice-stage-main")
        .relative()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .rounded(px(12.))
        .border_2()
        .border_color(primary)
        .bg(gpui_kit::black())
        .aria_label(caption.clone())
        .child(div().absolute().inset_0().flex().items_center().justify_center().child(picture))
        // Lo que se llevó a mano vuelve a la rejilla con otro clic; una
        // pantalla compartida está ahí sola y no se puede devolver.
        .when(by_hand, |this| {
            let label = tr(cx, "calls:backToGrid");
            this.cursor_pointer()
                .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                .on_click(move |_, _, cx| Backend::core(cx).voice().toggle_focus(&identity, source))
        })
        .child(
            header_button("voice-fullscreen", Lucide::Maximize2, String::new(), cx)
                .absolute()
                .top_3()
                .left_3()
                .bg(background.opacity(0.7))
                .map(|this| {
                    let label = tr(cx, "calls:fullscreen");
                    this.aria_label(label.clone()).tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                })
                .on_click(|_, window, cx| {
                    cx.stop_propagation();
                    window.toggle_fullscreen();
                })
                .test_support(),
        )
        .children(faces)
        // Lo que compartes tú se para desde donde lo estás mirando, no sólo
        // desde la barra.
        .when(on_stage.source == VideoSource::Screen && on_stage.me, |this| {
            this.child(
                div()
                    .id("voice-stop-sharing")
                    .absolute()
                    .bottom_3()
                    .right_3()
                    .rounded_full()
                    .border_1()
                    .border_color(danger.opacity(0.5))
                    .bg(background.opacity(0.8))
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(danger)
                    .cursor_pointer()
                    .on_click(|_, _, cx| {
                        cx.stop_propagation();
                        Backend::core(cx).voice().toggle_share();
                    })
                    .aria_label(tr(cx, "common:last.stopSharing"))
                    .child(tr(cx, "common:last.stopSharing"))
                    .test_support(),
            )
        })
        .child(
            div()
                .id("voice-stage-caption")
                .absolute()
                .bottom_3()
                .left_3()
                .max_w(relative(0.7))
                .rounded_full()
                .bg(background.opacity(0.7))
                .px(px(10.))
                .py_1()
                .text_size(px(13.))
                .aria_label(caption.clone())
                .child(div().truncate().child(caption))
                .test_support(),
        )
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
    use crate::voice_video::{Feeds, Frame, VideoFeeds, Wake};
    use cac_core::{ScreenSource, ScreenSources, VideoSource, VoiceEngine, VoiceEvent, VoiceFuture};
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
        /// Lo que se le dijo de mirar el vídeo, aparte de `calls`.
        pub video: Mutex<Vec<bool>>,
        /// Las pantallas y ventanas que dice tener.
        pub sources: Mutex<ScreenSources>,
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
        fn watch_video(&self, on: bool) {
            self.video.lock().unwrap().push(on);
        }
        fn set_camera(&self, on: bool) {
            self.calls.lock().unwrap().push(format!("camera {on}"));
        }
        fn screen_sources(&self) -> ScreenSources {
            self.sources.lock().unwrap().clone()
        }
        fn share_screen(&self, source: Option<ScreenSource>) {
            self.calls.lock().unwrap().push(format!("share {}", source.map_or("system".to_string(), |source| source.id)));
        }
        fn stop_share(&self) {
            self.calls.lock().unwrap().push("stop share".into());
        }
    }

    /// Fotogramas de mentira: los que se le dejan, de uno en uno, y apunta
    /// los tamaños que le dicen.
    #[derive(Default)]
    struct FakeFeeds {
        frames: Mutex<std::collections::HashMap<(String, VideoSource), Frame>>,
        taken: Mutex<usize>,
        targets: Mutex<Vec<(String, u32, u32)>>,
        wake: Mutex<Option<Wake>>,
    }

    impl FakeFeeds {
        fn push(&self, identity: &str, source: VideoSource, fill: u8) {
            self.push_sized(identity, source, (4, 2), fill);
        }

        fn push_sized(&self, identity: &str, source: VideoSource, (width, height): (u32, u32), fill: u8) {
            let frame = Frame { width, height, bgra: vec![fill; (width * height * 4) as usize] };
            self.frames.lock().unwrap().insert((identity.into(), source), frame);
            if let Some(wake) = self.wake.lock().unwrap().clone() {
                wake();
            }
        }
    }

    impl Feeds for FakeFeeds {
        fn take(&self, identity: &str, source: VideoSource) -> Option<Frame> {
            let frame = self.frames.lock().unwrap().remove(&(identity.to_string(), source));
            if frame.is_some() {
                *self.taken.lock().unwrap() += 1;
            }
            frame
        }
        fn set_target(&self, identity: &str, _source: VideoSource, width: u32, height: u32) {
            self.targets.lock().unwrap().push((identity.into(), width, height));
        }
        fn on_frame(&self, wake: Wake) {
            *self.wake.lock().unwrap() = Some(wake);
        }
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
        let (window, engine, _) = open_workspace(server, cx);
        (window, engine)
    }

    fn open_workspace(server: &FakeServer, cx: &mut TestAppContext) -> (AnyWindowHandle, Arc<FakeEngine>, gpui_kit::Entity<Workspace>) {
        open_polling_every(server, Duration::from_millis(100), cx)
    }

    fn open_polling_every(server: &FakeServer, every: Duration, cx: &mut TestAppContext) -> (AnyWindowHandle, Arc<FakeEngine>, gpui_kit::Entity<Workspace>) {
        server.install_with(cx, move |config| config.voice_presence_every = every);
        let engine = Arc::new(FakeEngine::default());
        cx.update(|cx| Backend::core(cx).set_voice_engine(engine.clone()));
        sign_in(cx);
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1100.), px(760.)) })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window")
        });
        (window.0, engine, window.1)
    }

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn bounds(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> gpui_kit::Bounds<gpui_kit::Pixels> {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).bounds()
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

    /// Entra a «Producto» con Luis y Eva dentro y vídeo de mentira.
    fn in_a_call_with_video(cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle, Arc<FakeEngine>, Arc<FakeFeeds>, gpui_kit::Entity<Workspace>) {
        let server = server();
        let feeds = Arc::new(FakeFeeds::default());
        // Sin sondeo que repinte cada poco: un fotograma tiene que pintarse
        // porque llegó, no porque otra cosa repintó la ventana.
        let (window, engine, workspace) = open_polling_every(&server, Duration::from_secs(3600), cx);
        let installed = feeds.clone();
        cx.update(|cx| cx.set_global(VideoFeeds(installed)));
        to_product(window, cx);
        click(window, "voice-join", cx);
        wait_until(cx, "dentro de la sala", |cx| on_screen(window, "voice-tile-u1", cx));
        for (identity, name) in [("u2", "Luis Mora"), ("u3", "Eva")] {
            tell(cx, VoiceEvent::Joined { identity: identity.into(), name: name.into() });
        }
        (server, window, engine, feeds, workspace)
    }

    fn tell(cx: &mut TestAppContext, event: VoiceEvent) {
        cx.update(|cx| Backend::core(cx).voice().engine_event(event));
    }

    fn video(cx: &mut TestAppContext, identity: &str, source: VideoSource, enabled: bool) {
        tell(cx, VoiceEvent::Video { identity: identity.into(), source, enabled });
    }

    /// Lo que sabe el recuadro de vídeo de alguien: (si tiene imagen, cuántas
    /// pintó, cuántas tiró del atlas).
    fn picture(workspace: &gpui_kit::Entity<Workspace>, identity: &str, source: VideoSource, cx: &mut TestAppContext) -> Option<(bool, usize, usize)> {
        cx.update(|cx| {
            let wall = workspace.read(cx).video.clone();
            let tile = wall.read(cx).tile(identity, source)?;
            let tile = tile.read(cx);
            Some((tile.has_picture(), tile.painted, tile.dropped))
        })
    }

    /// El fondo desenfocado de un recuadro: (si lo tiene, cuántos tiró del
    /// atlas).
    fn backdrop(workspace: &gpui_kit::Entity<Workspace>, identity: &str, source: VideoSource, cx: &mut TestAppContext) -> Option<(bool, usize)> {
        cx.update(|cx| {
            let wall = workspace.read(cx).video.clone();
            let tile = wall.read(cx).tile(identity, source)?;
            let tile = tile.read(cx);
            Some((tile.has_backdrop(), tile.blurred_dropped))
        })
    }

    /// Una cámara que se enciende tapa la cara con su vídeo, entero y sobre
    /// su fondo desenfocado; cada fotograma nuevo tira el anterior (y su
    /// fondo) del atlas, el motor se entera de a qué tamaño se ve, y apagarla
    /// devuelve la cara.
    ///
    /// Roturas que atrapa: no pintar el vídeo de una cámara encendida; una
    /// cámara sin fondo; no tirar la imagen o el fondo anteriores (la memoria
    /// crece a gigas en segundos); no
    /// decirle el tamaño al motor (convierte el vídeo entero); dejar el
    /// vídeo, o su último fotograma, cuando la cámara se apaga; no repintar
    /// al llegar un fotograma.
    #[gpui_kit::test]
    fn a_camera_covers_the_face_and_turning_it_off_brings_it_back(cx: &mut TestAppContext) {
        let (_server, window, _, feeds, workspace) = in_a_call_with_video(cx);
        assert!(!on_screen(window, "voice-video-u2-camera", cx), "sin cámara, la cara");

        video(cx, "u2", VideoSource::Camera, true);
        wait_until(cx, "el vídeo de Luis en su recuadro", |cx| on_screen(window, "voice-video-u2-camera", cx));
        feeds.push("u2", VideoSource::Camera, 1);
        wait_until(cx, "su primer fotograma, pintado", |cx| picture(&workspace, "u2", VideoSource::Camera, cx).is_some_and(|(shown, _, _)| shown));
        // Que llegue un fotograma avisa a su recuadro: sin eso se queda en el
        // primero hasta que otra cosa repinte la ventana. Se cuenta el aviso y
        // no el pintado, porque la ventana se repinta sola de vez en cuando.
        let told = Arc::new(Mutex::new(0usize));
        let counter = told.clone();
        let _watching = cx.update(|cx| {
            let tile = workspace.read(cx).video.read(cx).tile("u2", VideoSource::Camera).unwrap();
            cx.observe(&tile, move |_, _| *counter.lock().unwrap() += 1)
        });
        feeds.push("u2", VideoSource::Camera, 2);
        wait_until(cx, "el aviso al recuadro", |_| *told.lock().unwrap() > 0);
        wait_until(cx, "el segundo, pintado", |cx| picture(&workspace, "u2", VideoSource::Camera, cx).is_some_and(|(_, painted, _)| painted == 2));
        assert_eq!(picture(&workspace, "u2", VideoSource::Camera, cx).unwrap().2, 1, "el primero salió del atlas");
        assert_eq!(backdrop(&workspace, "u2", VideoSource::Camera, cx), Some((true, 1)), "con su fondo, y el anterior también salió");
        assert!(
            feeds.targets.lock().unwrap().iter().any(|(who, width, height)| who == "u2" && *width > 0 && *height > 0),
            "el motor sabe a qué tamaño se ve"
        );

        video(cx, "u2", VideoSource::Camera, false);
        wait_until(cx, "la cara de vuelta", |cx| !on_screen(window, "voice-video-u2-camera", cx));
        assert!(picture(&workspace, "u2", VideoSource::Camera, cx).is_none(), "su recuadro de vídeo se soltó");
        stop(cx);
    }

    /// La imagen de una cámara ocupa justo su recuadro, también la de un
    /// móvil en vertical en uno ancho; dentro, `video_fit` la deja entera.
    ///
    /// Roturas que atrapa: la imagen con `size_full` sin más (`img` le pone
    /// la proporción del vídeo y crece de alto hasta tres veces el recuadro,
    /// que la recorta: así una cámara vertical a pantalla completa era una
    /// nariz, 19-nov); clavada a los bordes sin tamaño (se queda con el ancho
    /// de su proporción, pegada a la izquierda).
    #[gpui_kit::test]
    fn a_tall_camera_gets_exactly_its_tile(cx: &mut TestAppContext) {
        let (_server, window, _, feeds, workspace) = in_a_call_with_video(cx);
        video(cx, "u2", VideoSource::Camera, true);
        wait_until(cx, "el vídeo de Luis en su recuadro", |cx| on_screen(window, "voice-video-u2-camera", cx));
        feeds.push_sized("u2", VideoSource::Camera, (2, 8), 1);
        wait_until(cx, "su fotograma, pintado", |cx| picture(&workspace, "u2", VideoSource::Camera, cx).is_some_and(|(shown, _, _)| shown));
        let tile = bounds(window, "voice-video-u2-camera", cx);
        let image = bounds(window, "voice-video-u2-camera-picture", cx);
        assert!(tile.size.width > tile.size.height, "el recuadro es ancho: {tile:?}");
        assert_eq!(image, tile, "la imagen ocupa su recuadro, ni más alta ni más estrecha");
        stop(cx);
    }

    /// Una pantalla compartida va al escenario con quién la comparte; las
    /// caras pasan a la tira; pulsar una cámara la lleva al escenario y
    /// pulsar el escenario la devuelve; minimizar deja de mirar el vídeo.
    ///
    /// Roturas que atrapa: una pantalla que se queda en la rejilla; una
    /// pantalla con fondo de colores; el rótulo sin nombre; la píldora que no abre la tira; el clic que no
    /// lleva una cámara al escenario, o el que no la devuelve; minimizar sin
    /// decirle al motor que nadie mira.
    #[gpui_kit::test]
    fn a_shared_screen_takes_the_stage_and_a_click_takes_a_camera_there(cx: &mut TestAppContext) {
        let (_server, window, _, feeds, workspace) = in_a_call_with_video(cx);
        video(cx, "u2", VideoSource::Screen, true);
        video(cx, "u3", VideoSource::Camera, true);
        wait_until(cx, "la pantalla de Luis, en grande", |cx| on_screen(window, "voice-stage-main", cx));
        assert_eq!(label(window, "voice-stage-caption", cx), "Luis Mora está compartiendo");
        assert!(on_screen(window, "voice-video-u2-screen", cx));
        feeds.push("u2", VideoSource::Screen, 1);
        wait_until(cx, "la pantalla, pintada", |cx| picture(&workspace, "u2", VideoSource::Screen, cx).is_some_and(|(shown, _, _)| shown));
        assert_eq!(backdrop(&workspace, "u2", VideoSource::Screen, cx), Some((false, 0)), "una pantalla va sobre negro");
        assert!(!on_screen(window, "voice-strip", cx), "las caras, en su píldora");

        click(window, "voice-faces", cx);
        wait_until(cx, "la tira", |cx| on_screen(window, "voice-strip", cx));
        click(window, "voice-tile-u3", cx);
        wait_until(cx, "Eva en grande", |cx| label(window, "voice-stage-caption", cx) == "Eva");
        assert!(on_screen(window, "voice-video-u3-camera", cx));

        click(window, "voice-stage-main", cx);
        wait_until(cx, "de vuelta a la pantalla", |cx| label(window, "voice-stage-caption", cx) == "Luis Mora está compartiendo");
        stop(cx);
    }

    /// Con una cámara llevada a mano al escenario, abrir la tira de caras la
    /// deja donde está.
    /// Rotura que atrapa: que el clic en la píldora le llegue también al
    /// escenario y lo devuelva a la rejilla (pasó en Xvfb el 19-nov).
    #[gpui_kit::test]
    fn opening_the_faces_keeps_a_camera_on_the_stage(cx: &mut TestAppContext) {
        let (_server, window, engine, _, workspace) = in_a_call_with_video(cx);
        video(cx, "u3", VideoSource::Camera, true);
        wait_until(cx, "la cámara de Eva", |cx| on_screen(window, "voice-video-u3-camera", cx));
        click(window, "voice-tile-u3", cx);
        wait_until(cx, "Eva en grande", |cx| label(window, "voice-stage-caption", cx) == "Eva");
        click(window, "voice-faces", cx);
        wait_until(cx, "la tira", |cx| on_screen(window, "voice-strip", cx));
        assert_eq!(label(window, "voice-stage-caption", cx), "Eva", "sigue en grande");

        // Y minimizar deja de mirar el vídeo.
        click(window, "voice-minimize", cx);
        wait_until(cx, "minimizada", |cx| on_screen(window, "voice-mini", cx));
        assert_eq!(*engine.video.lock().unwrap(), [true, false]);
        assert!(picture(&workspace, "u2", VideoSource::Screen, cx).is_none(), "sin sala a la vista, ningún vídeo pintándose");
        stop(cx);
    }

    fn source(id: &str, title: &str) -> ScreenSource {
        ScreenSource { id: id.into(), title: title.into() }
    }

    fn last_call(engine: &FakeEngine) -> String {
        engine.calls.lock().unwrap().last().cloned().unwrap_or_default()
    }

    /// Pulsar la cámara se la pide al motor y gira hasta que contesta; con
    /// su sí, tu recuadro enseña tu vídeo y el botón se enciende; volver a
    /// pulsarla la apaga ya, sin esperar.
    ///
    /// Roturas que atrapa: un botón que no llega al motor; darla por
    /// encendida antes de que conteste (sin giro); tu recuadro sin tu vídeo;
    /// apagar esperando al motor.
    #[gpui_kit::test]
    fn the_camera_waits_for_the_engine_and_shows_in_your_tile(cx: &mut TestAppContext) {
        let (_server, window, engine, _, _) = in_a_call_with_video(cx);
        assert_eq!(label(window, "voice-camera", cx), "Encender tu cámara");

        click(window, "voice-camera", cx);
        wait_until(cx, "gira mientras arranca", |cx| on_screen(window, "voice-camera-starting", cx));
        assert_eq!(last_call(&engine), "camera true");
        assert_eq!(label(window, "voice-camera", cx), "Apagar tu cámara", "pulsarla mientras gira la cancela");
        assert!(!on_screen(window, "voice-video-u1-camera", cx), "sin el sí del motor no hay vídeo");

        video(cx, "u1", VideoSource::Camera, true);
        wait_until(cx, "tu vídeo en tu recuadro", |cx| on_screen(window, "voice-video-u1-camera", cx));
        assert!(!on_screen(window, "voice-camera-starting", cx), "ya no gira");
        assert_eq!(label(window, "voice-camera", cx), "Apagar tu cámara");

        click(window, "voice-camera", cx);
        wait_until(cx, "apagada en el acto", |cx| !on_screen(window, "voice-video-u1-camera", cx));
        assert_eq!(last_call(&engine), "camera false");
        assert_eq!(label(window, "voice-camera", cx), "Encender tu cámara");
        stop(cx);
    }

    /// Si la cámara no sale, el botón vuelve a apagado y la sala dice por
    /// qué.
    /// Rotura que atrapa: un botón que se queda girando, o que vuelve sin
    /// decir nada.
    #[gpui_kit::test]
    fn a_camera_that_fails_puts_the_button_back_and_says_why(cx: &mut TestAppContext) {
        let (_server, window, _, _, _) = in_a_call_with_video(cx);
        click(window, "voice-camera", cx);
        wait_until(cx, "arrancando", |cx| on_screen(window, "voice-camera-starting", cx));
        tell(cx, VoiceEvent::VideoFailed { source: VideoSource::Camera, code: "voice-camera-busy".into() });
        wait_until(cx, "la frase", |cx| label(window, "voice-error", cx) == "Otro programa está usando la cámara.");
        assert!(!on_screen(window, "voice-camera-starting", cx));
        assert_eq!(label(window, "voice-camera", cx), "Encender tu cámara");
        stop(cx);
    }

    /// Con varias pantallas y ventanas, compartir abre el selector, con las
    /// pantallas por su número y las ventanas por su título; Escape lo
    /// cierra sin compartir nada, cerrar la sala también, y pulsar una
    /// ventana manda esa al motor.
    ///
    /// Roturas que atrapa: compartir sin preguntar; un selector sin las
    /// ventanas; una pantalla con el nombre del capturador («screen»);
    /// Escape que no cierra, o que comparte; el selector que vuelve al abrir
    /// otra vez la sala; mandar otra fuente que la pulsada; el selector que
    /// se queda abierto después de elegir.
    #[gpui_kit::test]
    fn several_sources_open_the_picker_and_the_one_picked_is_shared(cx: &mut TestAppContext) {
        let (_server, window, engine, _, _) = in_a_call_with_video(cx);
        *engine.sources.lock().unwrap() = ScreenSources { screens: vec![source("screen:1", "screen"), source("screen:2", "screen")], windows: vec![source("window:7", "Notas — Editor")] };

        click(window, "voice-share", cx);
        wait_until(cx, "el selector", |cx| on_screen(window, "voice-picker", cx));
        assert_eq!(label(window, "voice-picker-title", cx), "¿Qué compartes?");
        assert_eq!(label(window, "voice-pick-screen:2", cx), "Pantalla 2");
        assert_eq!(label(window, "voice-pick-window:7", cx), "Notas — Editor");
        assert!(!last_call(&engine).starts_with("share"), "sin elegir no se comparte");

        cx.simulate_keystrokes(window, "escape");
        wait_until(cx, "Escape lo cierra", |cx| !on_screen(window, "voice-picker", cx));
        assert!(!engine.calls.lock().unwrap().iter().any(|call| call.starts_with("share")), "y no comparte nada");

        // Cerrar la sala con el selector abierto no lo deja pendiente.
        click(window, "voice-share", cx);
        wait_until(cx, "abierto", |cx| on_screen(window, "voice-picker", cx));
        cx.update(|cx| Backend::core(cx).voice().close_stage());
        wait_until(cx, "minimizada", |cx| on_screen(window, "voice-mini", cx));
        cx.update(|cx| Backend::core(cx).voice().open_stage());
        wait_until(cx, "de vuelta", |cx| on_screen(window, "voice-share", cx));
        assert!(!on_screen(window, "voice-picker", cx), "el selector no vuelve solo");

        click(window, "voice-share", cx);
        wait_until(cx, "otra vez", |cx| on_screen(window, "voice-picker", cx));
        click(window, "voice-pick-window:7", cx);
        wait_until(cx, "cerrado al elegir", |cx| !on_screen(window, "voice-picker", cx));
        assert_eq!(last_call(&engine), "share window:7");
        assert!(on_screen(window, "voice-share-starting", cx), "y espera al motor");
        stop(cx);
    }

    /// Lo que compartes tú va al escenario con «Estás compartiendo» y su
    /// botón de parar; pararlo se lo dice al motor y la sala vuelve a la
    /// rejilla. Con una sola pantalla se comparte sin preguntar.
    ///
    /// Roturas que atrapa: preguntar con una sola fuente; tu pantalla sin
    /// botón de parar, o un botón que no para; el escenario que se queda con
    /// tu pantalla después de pararla.
    #[gpui_kit::test]
    fn stopping_your_share_from_the_stage_goes_back_to_the_grid(cx: &mut TestAppContext) {
        let (_server, window, engine, _, _) = in_a_call_with_video(cx);
        *engine.sources.lock().unwrap() = ScreenSources { screens: vec![source("screen:1", "screen")], windows: vec![] };

        click(window, "voice-share", cx);
        wait_until(cx, "pedida sin preguntar", |_| last_call(&engine) == "share screen:1");
        assert!(!on_screen(window, "voice-picker", cx));
        video(cx, "u1", VideoSource::Screen, true);
        wait_until(cx, "tu pantalla, en grande", |cx| label(window, "voice-stage-caption", cx) == "Estás compartiendo");
        assert_eq!(label(window, "voice-share", cx), "Dejar de compartir la pantalla");

        click(window, "voice-stop-sharing", cx);
        wait_until(cx, "la rejilla", |cx| on_screen(window, "voice-grid", cx));
        assert_eq!(last_call(&engine), "stop share");
        assert_eq!(label(window, "voice-share", cx), "Compartir tu pantalla");
        stop(cx);
    }

    /// Elegir una cámara en los ajustes la deja apuntada para la próxima
    /// vez, sin tocar el micrófono ni el altavoz elegidos.
    /// Roturas que atrapa: no guardarla; guardarla como micrófono o altavoz.
    #[gpui_kit::test]
    fn the_camera_chosen_in_settings_is_remembered(cx: &mut TestAppContext) {
        use crate::voice_settings::{DeviceLists, DeviceRow};
        let (_server, window, _, _, workspace) = in_a_call_with_video(cx);
        cx.update(|cx| Backend::core(cx).voice().choose_device(false, Some("mic-1".into())));
        let row = |name: &str, current: bool| DeviceRow { id: name.into(), name: name.into(), current };
        workspace.update(cx, |workspace, cx| {
            workspace.voice_settings.open = true;
            workspace.voice_settings.lists = Some(DeviceLists { cameras: vec![row("Integrated", true), row("Logitech C920", false)], ..DeviceLists::default() });
            cx.notify();
        });
        wait_until(cx, "el grupo Cámara", |cx| on_screen(window, "voice-device-camera-Logitech C920", cx));

        click(window, "voice-device-camera-Logitech C920", cx);
        let choices = cx.update(|cx| Backend::core(cx).voice().device_choices());
        assert_eq!(choices.camera.as_deref(), Some("Logitech C920"));
        assert_eq!((choices.input.as_deref(), choices.output.as_deref()), (Some("mic-1"), None), "lo demás, como estaba");
        stop(cx);
    }
}
