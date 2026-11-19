//! Los tonos del timbre: el que oye quien llama y el que oye a quien llaman.
//!
//! Vino de `app/src/components/voice/ringtone.ts`. Sintetizados, sin ficheros:
//! dos senos y una envolvente. La **rampa de 30 ms** al entrar y al salir no es
//! estética: cortar una onda en seco es un chasquido, y un chasquido cada 1,6 s
//! es peor que el propio timbre.
//!
//! Aquí se calcula **un ciclo entero**, con su silencio: quien lo hace sonar lo
//! repite hasta que se le diga que pare.

use std::f32::consts::TAU;

/// Los dos tonos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum RingTone {
    /// «Estoy llamando»: 440 + 480 Hz, 1,2 s sonando y 2,4 s callado, el
    /// patrón del teléfono americano. Por reconocible, no por bonito.
    Outgoing,
    /// «Te llaman»: dos notas que suben cada 1,6 s. Un sonido que sube se oye
    /// como una pregunta.
    Incoming,
}

const RAMP: f32 = 0.03;

/// Un pitido: frecuencias a la vez, empezando en `start` segundos.
struct Beep {
    hz: &'static [f32],
    start: f32,
    duration: f32,
    gain: f32,
}

/// Lo que dura un ciclo, en segundos.
pub fn ring_cycle(tone: RingTone) -> f32 {
    match tone {
        RingTone::Outgoing => 3.6,
        RingTone::Incoming => 1.6,
    }
}

fn beeps(tone: RingTone) -> Vec<Beep> {
    match tone {
        RingTone::Outgoing => vec![Beep { hz: &[440.0, 480.0], start: 0.0, duration: 1.2, gain: 0.05 }],
        RingTone::Incoming => vec![
            Beep { hz: &[880.0], start: 0.0, duration: 0.26, gain: 0.045 },
            Beep { hz: &[1174.0], start: 0.3, duration: 0.32, gain: 0.045 },
        ],
    }
}

/// La envolvente: rampa de subida, meseta y rampa de bajada.
fn envelope(t: f32, duration: f32, gain: f32) -> f32 {
    if t < RAMP {
        gain * t / RAMP
    } else if t > duration - RAMP {
        gain * ((duration - t) / RAMP).max(0.0)
    } else {
        gain
    }
}

/// Un ciclo del tono, en mono, a `rate` muestras por segundo, entre -1 y 1.
pub fn render_ring(tone: RingTone, rate: u32) -> Vec<f32> {
    let mut samples = vec![0.0f32; (ring_cycle(tone) * rate as f32).round() as usize];
    let step = 1.0 / rate as f32;
    for beep in beeps(tone) {
        let first = (beep.start * rate as f32) as usize;
        let count = (beep.duration * rate as f32) as usize;
        for (offset, sample) in samples.iter_mut().skip(first).take(count).enumerate() {
            let t = offset as f32 * step;
            let wave: f32 = beep.hz.iter().map(|hz| (TAU * hz * t).sin()).sum();
            *sample += envelope(t, beep.duration, beep.gain) * wave;
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// La frecuencia que domina un trozo, contando cruces por cero.
    fn crossings_hz(samples: &[f32]) -> f32 {
        let crossings = samples.windows(2).filter(|pair| (pair[0] <= 0.0) != (pair[1] <= 0.0)).count();
        crossings as f32 / 2.0 / (samples.len() as f32 / RATE as f32)
    }

    fn at(samples: &[f32], from: f32, to: f32) -> &[f32] {
        &samples[(from * RATE as f32) as usize..(to * RATE as f32) as usize]
    }

    /// El ciclo dura lo que tiene que durar y calla donde tiene que callar.
    /// Roturas que atrapa: un ciclo de otra longitud (el tono se aceleraría o
    /// se arrastraría al repetirse); un tono de llamada que no calla sus 2,4 s;
    /// la segunda nota de la entrante fuera de su sitio.
    #[test]
    fn each_cycle_has_its_rhythm() {
        let outgoing = render_ring(RingTone::Outgoing, RATE);
        assert_eq!(outgoing.len(), (3.6 * RATE as f32) as usize);
        assert!(at(&outgoing, 0.1, 1.1).iter().any(|s| s.abs() > 0.05), "suena el primer segundo");
        assert!(at(&outgoing, 1.25, 3.6).iter().all(|s| *s == 0.0), "y calla el resto");

        let incoming = render_ring(RingTone::Incoming, RATE);
        assert_eq!(incoming.len(), (1.6 * RATE as f32) as usize);
        assert!(at(&incoming, 0.27, 0.29).iter().all(|s| *s == 0.0), "un respiro entre las dos notas");
        assert!(at(&incoming, 0.35, 0.55).iter().any(|s| s.abs() > 0.02), "la segunda nota");
        assert!(at(&incoming, 0.65, 1.6).iter().all(|s| *s == 0.0));
    }

    /// Las notas son las de Tauri. Roturas que atrapa: cambiar una frecuencia
    /// (la entrante sube 880 → 1174; una que bajara se oiría como un aviso).
    #[test]
    fn the_incoming_ring_goes_up() {
        let incoming = render_ring(RingTone::Incoming, RATE);
        let first = crossings_hz(at(&incoming, 0.05, 0.22));
        let second = crossings_hz(at(&incoming, 0.35, 0.58));
        assert!((first - 880.0).abs() < 15.0, "primera nota {first}");
        assert!((second - 1174.0).abs() < 15.0, "segunda nota {second}");
    }

    /// Sin chasquidos: arranca y acaba en cero, y nunca salta de golpe.
    /// Rotura que atrapa: quitar la rampa.
    #[test]
    fn no_clicks() {
        for tone in [RingTone::Outgoing, RingTone::Incoming] {
            let samples = render_ring(tone, RATE);
            assert!(samples[0].abs() < 1e-3);
            let worst = samples.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max);
            assert!(worst < 0.02, "{tone:?}: salto de {worst}");
            assert!(samples.iter().all(|s| s.abs() <= 1.0));
        }
    }
}
