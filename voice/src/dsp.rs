//! Lo que se le hace al sonido, sin hardware ni red.
//!
//! Todo lo de aquí es puro a propósito: es la parte del motor que se puede
//! probar sin tarjeta de sonido (el CI no tiene), y lo que hace mal no lanza
//! nada: se oye. Voz con clics, metálica, acelerada, o un chasquido al sumar
//! dos voces fuertes. Portado de `app/src-tauri/src/voice.rs` y del spike
//! `spikes/calls-native/src/call.rs`, donde cada número tiene su historia.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// 48 kHz mono: lo que WebRTC usa internamente. Publicar en su tasa evita un
/// remuestreo que sólo añadiría latencia.
pub(crate) const SAMPLE_RATE: u32 = 48_000;
pub(crate) const CHANNELS: u32 = 1;
/// 10 ms por trama, que es el tamaño que espera libwebrtc.
pub(crate) const SAMPLES_PER_FRAME: usize = SAMPLE_RATE as usize / 100;

/// A partir de qué energía (RMS sobre 1 en escala completa) hay voz. Un 2 %
/// deja fuera el ventilador y entra con voz normal a medio metro.
const VOICE_THRESHOLD: f32 = 0.02;

/// Cuánto sigue encendido «estás hablando» tras la última trama con voz: más
/// que la pausa de una coma, menos que la de un turno. Sin esto el indicador
/// parpadea entre sílabas.
pub(crate) const VOICE_HANGOVER: Duration = Duration::from_millis(300);

/// 60 ms de colchón antes de que una pista empiece a sonar. Sin él la cola
/// rondaba el cero y cada vez que se vaciaba a mitad de un bloque de la
/// tarjeta sonaba un clic (v1.6.101, jose, 15-oct-2026).
pub(crate) const JITTER_CUSHION: usize = SAMPLES_PER_FRAME * 6;

/// 200 ms como mucho por pista; pasado eso se tira lo más viejo hasta el
/// colchón, en vez de vaciar la cola (que era otro hueco).
pub(crate) const QUEUE_MAX: usize = SAMPLES_PER_FRAME * 20;

/// 200 ms de referencia del eco como mucho: si la captura no la consume
/// (micro cambiando de dispositivo), lo viejo ya no cancela nada y sólo
/// desalinearía lo que llega después.
pub(crate) const REFERENCE_MAX: usize = SAMPLES_PER_FRAME * 20;

// ─── La captura: de lo que entrega cpal a tramas de 10 ms ─────────────────────

/// Por dónde va el remuestreo entre un bloque de cpal y el siguiente.
///
/// `pos` es la fracción pendiente: sin ella la fase se reinicia en cada
/// callback y sale un clic cada pocos milisegundos. `last_value` es el último
/// cuadro del bloque anterior: sin él la muestra de la frontera se interpola
/// contra sí misma, un codo más pequeño en el mismo sitio.
#[derive(Default, Debug, Clone, PartialEq)]
pub(crate) struct Phase {
    /// Relativa al primer cuadro del bloque actual; puede ser negativa
    /// (`[-1, 0)`): «entre el último del anterior y el primero de éste».
    pub(crate) pos: f32,
    pub(crate) last_value: f32,
}

/// Lleva un bloque de cuadros mono al ritmo de la fuente (`step` = ritmo del
/// dispositivo / 48 kHz), por interpolación lineal, y lo deja en `output`
/// como `i16`.
///
/// Sin esto, un micrófono a 44,1 kHz (lo normal en Windows con auriculares
/// USB) hacía que el SDK rechazara la primera trama y se dejara de publicar en
/// silencio. Lineal y no un remuestreador de verdad: es voz camino de Opus, y
/// en el callback de audio tardar es un corte.
///
/// `silent` da ceros **sin parar la fase**: al quitar el silencio la voz no
/// sale desfasada con un chasquido.
pub(crate) fn resample(mono_frames: &[f32], step: f32, phase: &mut Phase, silent: bool, output: &mut Vec<i16>) {
    let n = mono_frames.len();
    if n == 0 {
        return;
    }
    let mut pos = phase.pos;
    // Hasta `n - 1`: la última posición necesita el cuadro que no ha llegado,
    // y se aplaza al bloque siguiente.
    while pos < (n - 1) as f32 {
        let i = pos.floor();
        let frac = pos - i;
        let a = if i < 0.0 { phase.last_value } else { mono_frames[i as usize] };
        let b = mono_frames[(i + 1.0) as usize];
        let mean = a + (b - a) * frac;
        output.push(if silent { 0 } else { (mean.clamp(-1.0, 1.0) * i16::MAX as f32) as i16 });
        pos += step;
    }
    phase.pos = pos - n as f32;
    phase.last_value = mono_frames[n - 1];
}

/// Cuadros intercalados de `channels` canales, a mono promediando: publicar
/// sólo el izquierdo pierde la mitad de la señal en un micrófono estéreo.
pub(crate) fn interleaved_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    samples.chunks(channels).map(|t| t.iter().sum::<f32>() / t.len() as f32).collect()
}

/// Saca del acumulador las tramas de 10 ms enteras; lo que no llega a trama se
/// queda para la siguiente vez. libwebrtc sólo acepta tramas de 10 ms justos.
pub(crate) fn drain_frames(accumulator: &mut Vec<i16>) -> Vec<Vec<i16>> {
    let mut frames = Vec::with_capacity(accumulator.len() / SAMPLES_PER_FRAME);
    while accumulator.len() >= SAMPLES_PER_FRAME {
        let rest = accumulator.split_off(SAMPLES_PER_FRAME);
        frames.push(std::mem::replace(accumulator, rest));
    }
    frames
}

/// El pico de una trama, en tanto por mil de la escala completa. Se mide sobre
/// lo que **de verdad** se envía (después del procesado y del silencio): medir
/// antes diría que hay señal mientras se publican ceros.
pub(crate) fn peak_permille(frame: &[i16]) -> u32 {
    let peak = frame.iter().map(|s| s.unsigned_abs() as u32).max().unwrap_or(0);
    peak * 1000 / i16::MAX as u32
}

/// ¿Hay voz en estos 10 ms? RMS y no el pico: el pico lo alcanza el roce de
/// un dedo en el micrófono. Un teclazo fuerte sí cuela (se midió y se acepta).
pub(crate) fn has_voice(frame: &[i16]) -> bool {
    if frame.is_empty() {
        return false;
    }
    let total: f64 = frame.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (total / frame.len() as f64).sqrt() / i16::MAX as f64;
    rms as f32 > VOICE_THRESHOLD
}

/// «Estás hablando», medido del propio micrófono y con cola.
///
/// Distinto del motor de Tauri en un detalle: allí la última voz empezaba en
/// `Instant::now()` al arrancar, y la primera trama decía «hablando» durante
/// 300 ms sin que nadie hablara. Aquí empieza sin voz.
#[derive(Default, Debug)]
pub(crate) struct VoiceActivity {
    last_voice: Option<Instant>,
    speaking: bool,
}

impl VoiceActivity {
    /// El estado nuevo, sólo si cambió: los eventos van por cambio.
    pub(crate) fn update(&mut self, frame: &[i16], now: Instant) -> Option<bool> {
        if has_voice(frame) {
            self.last_voice = Some(now);
        }
        let speaking = self.last_voice.is_some_and(|t| now.saturating_duration_since(t) < VOICE_HANGOVER);
        (speaking != self.speaking).then(|| {
            self.speaking = speaking;
            speaking
        })
    }
}

// ─── La salida: lo que dicen los demás ────────────────────────────────────────

/// Suma una muestra de cada persona, en `i32` y recortada: dos voces fuertes a
/// la vez pasan de `i16::MAX`, y dar la vuelta suena a chasquido brutal;
/// recortar, sólo a algo áspero.
pub(crate) fn mix_samples(samples: impl Iterator<Item = i16>) -> i16 {
    samples.map(i32::from).sum::<i32>().clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Una trama del SDK, a mono. Se pide mono, pero si llegara estéreo tratarla
/// como mono la reproduciría a media velocidad y llenaría la cola el doble.
pub(crate) fn to_mono(samples: &[i16], channels: usize) -> Vec<i16> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples.chunks(channels).map(|c| (c.iter().map(|&v| v as i32).sum::<i32>() / c.len() as i32) as i16).collect()
}

/// Una pista remota en la mezcla: su cola a 48 kHz mono, y si está sonando o
/// esperando a tener colchón.
#[derive(Default, Debug)]
pub(crate) struct MixTrack {
    pub(crate) queue: VecDeque<i16>,
    playing: bool,
    /// Veces que se quedó sin muestras sonando: la medida de los clics.
    pub(crate) gaps: u64,
}

impl MixTrack {
    /// Lo que llega del SDK.
    pub(crate) fn push_samples(&mut self, samples: &[i16]) {
        self.queue.extend(samples.iter().copied());
        if self.queue.len() > QUEUE_MAX {
            let excess = self.queue.len() - JITTER_CUSHION;
            self.queue.drain(..excess);
        }
    }

    /// La siguiente muestra para la mezcla. Sin colchón, silencio y a esperar;
    /// si se vacía sonando, cuenta un hueco y vuelve a esperar al colchón, en
    /// vez de soltar muestras sueltas entre huecos.
    pub(crate) fn next_sample(&mut self) -> i16 {
        if !self.playing {
            if self.queue.len() < JITTER_CUSHION {
                return 0;
            }
            self.playing = true;
        }
        match self.queue.pop_front() {
            Some(sample) => sample,
            None => {
                self.playing = false;
                self.gaps += 1;
                0
            }
        }
    }
}

/// Deja lo que acaba de sonar para el cancelador de eco, sin pasar del tope.
pub(crate) fn note_reference(queue: &mut VecDeque<i16>, played: &[i16]) {
    queue.extend(played.iter().copied());
    if queue.len() > REFERENCE_MAX {
        let excess = queue.len() - REFERENCE_MAX;
        queue.drain(..excess);
    }
}

/// Lo que sonó, en tramas de 10 ms; lo que no llega a trama se queda.
pub(crate) fn frames_of(queue: &mut VecDeque<i16>) -> Vec<Vec<i16>> {
    let mut out = Vec::with_capacity(queue.len() / SAMPLES_PER_FRAME);
    while queue.len() >= SAMPLES_PER_FRAME {
        out.push(queue.drain(..SAMPLES_PER_FRAME).collect());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bloques del tamaño que entrega una tarjeta de verdad, y **no** de 10 ms
    /// redondos: 441 cuadros a 44,1 kHz dan 480 muestras exactas y el arrastre
    /// de fase vale cero siempre, así que con ese tamaño la prueba pasaba con
    /// y sin arrastre (la lección de `voice.rs`). 512 es lo típico en WASAPI.
    const PER_BLOCK: usize = 512;

    /// Una senoide de 440 Hz a media escala, continua entre bloques.
    fn blocks(rate: u32, how_many: usize) -> Vec<Vec<f32>> {
        (0..how_many)
            .map(|b| {
                (0..PER_BLOCK)
                    .map(|i| {
                        let n = (b * PER_BLOCK + i) as f32;
                        (n * 2.0 * std::f32::consts::PI * 440.0 / rate as f32).sin() * 0.5
                    })
                    .collect()
            })
            .collect()
    }

    fn resample_all(rate: u32, how_many: usize) -> Vec<i16> {
        let step = rate as f32 / SAMPLE_RATE as f32;
        let (mut output, mut phase) = (Vec::new(), Phase::default());
        for block in blocks(rate, how_many) {
            resample(&block, step, &mut phase, false, &mut output);
        }
        output
    }

    /// El salto más grande entre dos muestras seguidas.
    fn jump(v: &[i16]) -> i32 {
        v.windows(2).map(|p| (p[1] as i32 - p[0] as i32).abs()).max().unwrap()
    }

    /// La pendiente máxima de la propia onda a 48 kHz: `2π·440/48000 · 0,5 ·
    /// 32767`. Un remuestreo transparente se queda en ese número.
    const WAVE_SLOPE: i32 = 943;

    /// El caso que rompía: 44,1 kHz tiene que salir como 48 kHz. Más corto o
    /// más largo es voz acelerada o lenta.
    #[test]
    fn from_44k_comes_out_at_48k() {
        let n = resample_all(44_100, 100).len();
        let expected = (PER_BLOCK * 100) as f32 * 48_000.0 / 44_100.0;
        assert!((n as f32 - expected).abs() <= 2.0, "de 44,1 kHz salieron {n} muestras y se esperaban unas {expected:.0}");
    }

    /// Y la forma: el salto más grande es el de la onda, no un codo en cada
    /// frontera de bloque (que es lo que da no arrastrar la fase).
    #[test]
    fn the_output_is_the_wave_without_elbows() {
        let output = resample_all(44_100, 20);
        assert!(jump(&output) <= WAVE_SLOPE + 10, "el salto máximo fue {} y la onda sola da {WAVE_SLOPE}", jump(&output));
        // Y es la onda de verdad, no silencio ni una constante: llega a su pico.
        let peak = output.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((16_000..=16_400).contains(&peak), "el pico fue {peak}; media escala es 16383");
    }

    /// Silenciado son ceros, y la fase sigue corriendo igual.
    #[test]
    fn silent_gives_zeros_and_keeps_the_phase() {
        let step = 44_100f32 / SAMPLE_RATE as f32;
        let (mut quiet, mut normal) = (Vec::new(), Vec::new());
        let (mut pq, mut pn) = (Phase::default(), Phase::default());
        for block in blocks(44_100, 5) {
            resample(&block, step, &mut pq, true, &mut quiet);
            resample(&block, step, &mut pn, false, &mut normal);
        }
        assert!(quiet.iter().all(|&s| s == 0));
        assert_eq!(quiet.len(), normal.len());
        assert_eq!(pq, pn);
    }

    /// Lo que se publica son tramas de 10 ms exactos, y lo que sobra espera.
    #[test]
    fn only_whole_10ms_frames_leave_the_accumulator() {
        let mut acc: Vec<i16> = (0..(SAMPLES_PER_FRAME * 2 + 7) as i16).collect();
        let frames = drain_frames(&mut acc);
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|f| f.len() == SAMPLES_PER_FRAME));
        assert_eq!(frames[1][0], SAMPLES_PER_FRAME as i16, "en orden");
        assert_eq!(acc.len(), 7, "el resto se queda para la siguiente");
        // Justo una trama también sale: no se queda esperando a la siguiente.
        let mut exact = vec![1i16; SAMPLES_PER_FRAME];
        assert_eq!(drain_frames(&mut exact).len(), 1);
        assert!(exact.is_empty());
    }

    /// Dos voces a tope no dan la vuelta: se recortan.
    #[test]
    fn the_mix_clamps_instead_of_wrapping() {
        assert_eq!(mix_samples([30_000, 30_000].into_iter()), i16::MAX);
        assert_eq!(mix_samples([-30_000, -30_000].into_iter()), i16::MIN);
        assert_eq!(mix_samples([1_000, -250, 7].into_iter()), 757);
        assert_eq!(mix_samples(std::iter::empty()), 0);
    }

    /// Una pista no suena hasta tener 60 ms, y entonces suena en orden.
    #[test]
    fn the_cushion_holds_until_it_is_full() {
        let mut track = MixTrack::default();
        let almost: Vec<i16> = (1..JITTER_CUSHION as i16).collect();
        track.push_samples(&almost);
        assert_eq!(track.next_sample(), 0, "con {} muestras todavía no suena", almost.len());
        assert_eq!(track.queue.len(), almost.len(), "esperar no consume nada");
        track.push_samples(&[JITTER_CUSHION as i16]);
        assert_eq!(track.next_sample(), 1, "con el colchón lleno empieza por la primera");
        assert_eq!(track.next_sample(), 2);
    }

    /// Vaciada sonando, cuenta un hueco y vuelve a esperar el colchón entero.
    #[test]
    fn an_empty_queue_is_a_gap_and_waits_again() {
        let mut track = MixTrack::default();
        track.push_samples(&vec![5; JITTER_CUSHION]);
        for _ in 0..JITTER_CUSHION {
            assert_eq!(track.next_sample(), 5);
        }
        assert_eq!(track.next_sample(), 0);
        assert_eq!(track.gaps, 1);
        track.push_samples(&[9; 10]);
        assert_eq!(track.next_sample(), 0, "diez muestras no son colchón");
    }

    /// Pasado el tope se tira lo viejo hasta el colchón, no la cola entera.
    #[test]
    fn an_overfull_queue_drops_the_oldest_down_to_the_cushion() {
        let mut track = MixTrack::default();
        let lots: Vec<i16> = (0..(QUEUE_MAX + 1) as i32).map(|v| v as i16).collect();
        track.push_samples(&lots);
        assert_eq!(track.queue.len(), JITTER_CUSHION);
        assert_eq!(*track.queue.back().unwrap(), *lots.last().unwrap(), "se queda lo más nuevo");
    }

    #[test]
    fn peak_permille_is_thousandths_of_full_scale() {
        assert_eq!(peak_permille(&[]), 0);
        assert_eq!(peak_permille(&[0, 0]), 0);
        assert_eq!(peak_permille(&[i16::MAX]), 1000);
        // Negativo cuenta igual, e `i16::MIN` no desborda.
        assert_eq!(peak_permille(&[i16::MIN]), 1000);
        assert_eq!(peak_permille(&[100, -16_384, 3]), 500);
    }

    /// Hablando se enciende en la primera trama con voz, aguanta la cola y se
    /// apaga después; sólo avisa al cambiar. Y no empieza encendido.
    #[test]
    fn self_speaking_has_a_hangover_and_starts_off() {
        let t0 = Instant::now();
        let voice = vec![8_000i16; SAMPLES_PER_FRAME];
        let hush = vec![0i16; SAMPLES_PER_FRAME];
        let mut activity = VoiceActivity::default();
        assert_eq!(activity.update(&hush, t0), None, "sin voz no hay nada que avisar");
        assert_eq!(activity.update(&voice, t0), Some(true));
        assert_eq!(activity.update(&voice, t0 + Duration::from_millis(10)), None);
        assert_eq!(activity.update(&hush, t0 + Duration::from_millis(200)), None, "la pausa de una coma no apaga");
        assert_eq!(activity.update(&hush, t0 + Duration::from_millis(400)), Some(false));
    }

    #[test]
    fn stereo_frames_become_mono() {
        assert_eq!(to_mono(&[10, 20, -4, 4], 2), vec![15, 0]);
        assert_eq!(interleaved_to_mono(&[0.5, 0.25, 1.0, 0.0], 2), vec![0.375, 0.5]);
    }
}
