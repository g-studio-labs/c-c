//! El tono de un aviso: cuál, cuándo, y sus muestras.
//!
//! Vino de `app/src/lib/notify-sound.ts`. Lo pone la app y no el sistema: en
//! Linux que un aviso suene lo decide el servidor de avisos, y dunst no suena
//! aunque se lo pidan. Sintetizado, sin ficheros que empaquetar.
//!
//! Los dos tonos los eligió jose oyéndolos (20-oct-2026): «gota» para los
//! mensajes, que llegan seguidos y no pueden cansar, y «dos notas» (Mi → La)
//! para lo que te nombra a ti. Aquí sólo se calculan las muestras; quien las
//! hace sonar es cada plataforma.

use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use crate::error::ApiError;
use crate::notice::NoticeKind;
use crate::Core;

/// Bajo qué clave se guarda si los avisos suenan en esta máquina.
const SOUND_KEY: &str = "notify-sound";

/// Entre dos tonos, como mínimo. Diez avisos en un segundo suenan una vez.
pub const TONE_GAP: Duration = Duration::from_millis(1500);

/// Los dos tonos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Tone {
    /// Una nota que cae (Do → La): los mensajes.
    Drop,
    /// Mi y luego La: lo que te nombra a ti.
    TwoNotes,
}

/// El tono de cada clase de aviso.
pub fn tone_for(kind: NoticeKind) -> Tone {
    match kind {
        NoticeKind::Mention | NoticeKind::Reply => Tone::TwoNotes,
        NoticeKind::ChannelMessage | NoticeKind::Direct => Tone::Drop,
    }
}

/// Cuándo sonó el último, para no sonar de seguido.
#[derive(Debug, Default)]
pub struct Chime {
    last: Option<Instant>,
}

impl Chime {
    /// Si toca sonar ahora: encendido y sin otro tono en los últimos
    /// `TONE_GAP`. Si toca, apunta que sonó.
    pub fn should_play(&mut self, now: Instant, enabled: bool) -> bool {
        if !enabled || self.last.is_some_and(|last| now.duration_since(last) < TONE_GAP) {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// Una nota con ataque corto y caída exponencial hasta casi cero (el corte
/// seco es lo que chasquea), y si se pide, deslizándose a otra frecuencia.
struct Note {
    hz: f32,
    start: f32,
    duration: f32,
    gain: f32,
    glide: Option<f32>,
}

const ATTACK: f32 = 0.008;
const FLOOR: f32 = 0.0001;
/// Lo que se deja sonar tras la última nota, como el `stop(+0.05)` de Tauri.
const TAIL: f32 = 0.05;

fn notes(tone: Tone) -> Vec<Note> {
    match tone {
        Tone::Drop => vec![Note { hz: 1046.0, start: 0.0, duration: 0.25, gain: 0.15, glide: Some(880.0) }],
        Tone::TwoNotes => vec![
            Note { hz: 659.0, start: 0.0, duration: 0.22, gain: 0.13, glide: None },
            Note { hz: 880.0, start: 0.12, duration: 0.35, gain: 0.13, glide: None },
        ],
    }
}

/// Una rampa exponencial de `from` a `to` en `length` segundos, a los `t`.
fn ramp(from: f32, to: f32, t: f32, length: f32) -> f32 {
    from * (to / from).powf((t / length).clamp(0.0, 1.0))
}

/// Las muestras del tono, en mono, a `rate` por segundo, entre -1 y 1.
pub fn render(tone: Tone, rate: u32) -> Vec<f32> {
    let notes = notes(tone);
    let seconds = notes.iter().map(|note| note.start + note.duration).fold(0.0, f32::max) + TAIL;
    let mut samples = vec![0.0f32; (seconds * rate as f32).ceil() as usize];
    let step = 1.0 / rate as f32;
    for note in notes {
        let mut phase = 0.0f32;
        let first = (note.start * rate as f32) as usize;
        for (index, sample) in samples.iter_mut().enumerate().skip(first) {
            let t = index as f32 * step - note.start;
            if t > note.duration {
                break;
            }
            let hz = note.glide.map_or(note.hz, |to| ramp(note.hz, to, t, note.duration));
            let gain = if t < ATTACK { ramp(FLOOR, note.gain, t, ATTACK) } else { ramp(note.gain, FLOOR, t - ATTACK, note.duration - ATTACK) };
            *sample += gain * phase.sin();
            phase = (phase + TAU * hz * step) % TAU;
        }
    }
    samples
}

impl Core {
    /// Si los avisos suenan en esta máquina. Por omisión, sí.
    pub fn notify_sound(&self) -> bool {
        self.inner.storage.get(SOUND_KEY).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(true)
    }

    /// Enciende o apaga el sonido de los avisos en esta máquina.
    pub fn set_notify_sound(&self, on: bool) -> Result<(), ApiError> {
        let raw = serde_json::to_string(&on).map_err(|error| ApiError::Storage(error.to_string()))?;
        self.inner.storage.set(SOUND_KEY, &raw).map_err(ApiError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotura que atrapa: que lo que te nombra suene como un mensaje
    /// cualquiera, o al revés.
    #[test]
    fn what_tells_you_about_you_has_its_own_tone() {
        assert_eq!(tone_for(NoticeKind::Mention), Tone::TwoNotes);
        assert_eq!(tone_for(NoticeKind::Reply), Tone::TwoNotes);
        assert_eq!(tone_for(NoticeKind::ChannelMessage), Tone::Drop);
        assert_eq!(tone_for(NoticeKind::Direct), Tone::Drop);
    }

    /// Roturas que atrapa: sonar apagado; sonar diez veces en una ráfaga; no
    /// volver a sonar pasado el respiro; contar como sonado lo que no sonó
    /// por estar apagado.
    #[test]
    fn the_chime_waits_between_tones_and_stays_quiet_when_off() {
        let start = Instant::now();
        let mut chime = Chime::default();
        assert!(!chime.should_play(start, false), "apagado no suena");
        assert!(chime.should_play(start, true), "y apagado no cuenta como que sonó");
        assert!(!chime.should_play(start + Duration::from_millis(400), true));
        assert!(!chime.should_play(start + TONE_GAP - Duration::from_millis(1), true));
        assert!(chime.should_play(start + TONE_GAP, true));
    }

    /// Las muestras. Roturas que atrapa: un tono que dura lo que no es
    /// (la segunda nota de «dos notas» cortada, o silencio); muestras fuera
    /// de [-1, 1], que saturan; un final que no cae (chasquido); silencio.
    #[test]
    fn a_tone_lasts_what_it_should_and_fades_out() {
        let rate = 48_000;
        let drop = render(Tone::Drop, rate);
        let two = render(Tone::TwoNotes, rate);
        assert_eq!(drop.len(), (0.30f32 * rate as f32).ceil() as usize);
        assert_eq!(two.len(), (0.52f32 * rate as f32).ceil() as usize);
        for samples in [&drop, &two] {
            let peak = samples.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
            assert!(peak > 0.1 && peak <= 0.3, "pico {peak}");
            let end = &samples[samples.len() - rate as usize / 20..];
            assert!(end.iter().all(|sample| sample.abs() < 0.002), "el final cae a casi nada");
        }
        // La nota se apaga sola antes de acabar: sin caída, se corta en seco
        // a 0,15 y chasquea.
        let fading = &drop[(0.24 * rate as f32) as usize..(0.25 * rate as f32) as usize];
        assert!(fading.iter().all(|sample| sample.abs() < 0.01), "la gota cae antes de cortarse");
        // La segunda nota de «dos notas» sigue sonando cuando la primera ya
        // calló (a los 0,22 s). La caída es exponencial, como en Tauri: a
        // esas alturas suena flojo, pero suena.
        let late = &two[(0.23 * rate as f32) as usize..(0.30 * rate as f32) as usize];
        assert!(late.iter().any(|sample| sample.abs() > 0.003), "la segunda nota suena");
    }
}
