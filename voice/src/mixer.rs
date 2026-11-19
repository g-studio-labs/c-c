//! La mezcla de una llamada: una sola salida para todas las pistas, que es
//! también la referencia del cancelador de eco.
//!
//! En el motor de Tauri y en el spike esto eran tres globales (`TRACKS`,
//! `RENDER_REFERENCE`, `DEAFENED`). Aquí es un objeto por llamada: dos motores
//! no comparten pistas ni referencia, y colgar no tiene que acordarse de vaciar
//! nada global. Y `render` no toca hardware, así que lo que hace la salida
//! —mezclar, ensordecer, apuntar la referencia— se prueba sin tarjeta.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::dsp::{frames_of, mix_samples, note_reference, MixTrack, SAMPLE_RATE};

/// Una pista en la mezcla, compartida entre su tarea y la salida.
pub(crate) type TrackQueue = Arc<Mutex<MixTrack>>;

#[derive(Default)]
pub(crate) struct Mixer {
    tracks: Mutex<Vec<TrackQueue>>,
    /// Lo que ha sonado y el cancelador todavía no ha visto, a 48 kHz mono.
    /// Lo llena la salida y lo vacía la captura.
    reference: Mutex<VecDeque<i16>>,
    /// Ensordecido: la salida da silencio, y la referencia lo dice.
    deaf: AtomicBool,
}

/// Por dónde va la salida al remuestrear de 48 kHz (la mezcla) al ritmo del
/// dispositivo: `pos` es la fracción entre la muestra anterior y la siguiente.
/// Vive en el hilo de la salida; uno por flujo abierto.
pub(crate) struct OutputCursor {
    step: f32,
    pos: f32,
    before: i16,
    after: i16,
    played: Vec<i16>,
}

impl OutputCursor {
    pub(crate) fn new(device_rate: u32) -> Self {
        OutputCursor { step: SAMPLE_RATE as f32 / device_rate.max(1) as f32, pos: 1.0, before: 0, after: 0, played: Vec::with_capacity(1024) }
    }
}

impl Mixer {
    pub(crate) fn add_track(&self) -> TrackQueue {
        let queue: TrackQueue = Arc::new(Mutex::new(MixTrack::default()));
        self.tracks.lock().unwrap().push(queue.clone());
        queue
    }

    pub(crate) fn remove_track(&self, queue: &TrackQueue) {
        self.tracks.lock().unwrap().retain(|q| !Arc::ptr_eq(q, queue));
    }

    #[cfg(test)]
    pub(crate) fn track_count(&self) -> usize {
        self.tracks.lock().unwrap().len()
    }

    pub(crate) fn set_deaf(&self, deaf: bool) {
        self.deaf.store(deaf, Ordering::Relaxed);
    }

    /// Llena un bloque del dispositivo (`channels` canales intercalados, en
    /// `f32`) con la mezcla, y deja lo que sonó para el cancelador.
    ///
    /// Silencio cuando una cola se vacía: un hueco es preferible a repetir la
    /// última muestra, que suena a chirrido. Ensordecido **también consume**:
    /// si no, la cola crecería mientras no oyes y al volver sonaría lo de hace
    /// un rato (el tope de la cola lo recortaría, pero a saltos).
    pub(crate) fn render(&self, output: &mut [f32], channels: usize, cursor: &mut OutputCursor) {
        let deaf = self.deaf.load(Ordering::Relaxed);
        let tracks = self.tracks.lock().unwrap();
        let mut queues: Vec<_> = tracks.iter().map(|q| q.lock().unwrap()).collect();
        cursor.played.clear();
        for chunk in output.chunks_mut(channels.max(1)) {
            while cursor.pos >= 1.0 {
                let mixed = mix_samples(queues.iter_mut().map(|q| q.next_sample()));
                let sample = if deaf { 0 } else { mixed };
                cursor.played.push(sample);
                cursor.before = cursor.after;
                cursor.after = sample;
                cursor.pos -= 1.0;
            }
            let value = (cursor.before as f32 + (cursor.after as f32 - cursor.before as f32) * cursor.pos) / i16::MAX as f32;
            cursor.pos += cursor.step;
            for channel in chunk.iter_mut() {
                *channel = value;
            }
        }
        drop(queues);
        drop(tracks);
        note_reference(&mut self.reference.lock().unwrap(), &cursor.played);
    }

    /// Lo que sonó desde la última vez, en tramas de 10 ms, para el cancelador.
    pub(crate) fn take_reference(&self) -> Vec<Vec<i16>> {
        frames_of(&mut self.reference.lock().unwrap())
    }

    /// Al colgar: ni pistas ni referencia para la siguiente.
    pub(crate) fn clear(&self) {
        self.tracks.lock().unwrap().clear();
        self.reference.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::{JITTER_CUSHION, SAMPLES_PER_FRAME};

    /// Una pista con el colchón lleno de un valor constante.
    fn loud_track(mixer: &Mixer, value: i16) -> TrackQueue {
        let track = mixer.add_track();
        track.lock().unwrap().push_samples(&vec![value; JITTER_CUSHION * 2]);
        track
    }

    /// Un bloque estéreo de 10 ms a 48 kHz.
    fn render_block(mixer: &Mixer, cursor: &mut OutputCursor) -> Vec<f32> {
        let mut out = vec![0.5f32; SAMPLES_PER_FRAME * 2];
        mixer.render(&mut out, 2, cursor);
        out
    }

    /// Lo que se oye es la suma de las pistas, en los dos canales.
    #[test]
    fn the_output_plays_the_mix() {
        let mixer = Mixer::default();
        loud_track(&mixer, 8_000);
        loud_track(&mixer, 4_000);
        let mut cursor = OutputCursor::new(48_000);
        render_block(&mixer, &mut cursor);
        let out = render_block(&mixer, &mut cursor);
        let expected = 12_000.0 / i16::MAX as f32;
        assert!(out.iter().all(|&v| (v - expected).abs() < 1e-4), "salió {:?}…", &out[..4]);
        let reference = mixer.take_reference();
        assert_eq!(reference.len(), 2, "20 ms sonados son dos tramas de referencia");
        assert!(reference[1].iter().all(|&s| s == 12_000));
    }

    /// Ensordecido no suena nada, la referencia del eco dice que no sonó, y
    /// las colas se siguen consumiendo.
    #[test]
    fn deafened_the_output_is_silent() {
        let mixer = Mixer::default();
        let track = loud_track(&mixer, 20_000);
        let before = track.lock().unwrap().queue.len();
        mixer.set_deaf(true);
        let mut cursor = OutputCursor::new(48_000);
        for _ in 0..3 {
            let out = render_block(&mixer, &mut cursor);
            assert!(out.iter().all(|&v| v == 0.0), "ensordecido sonó {:?}…", &out[..4]);
        }
        assert!(mixer.take_reference().iter().flatten().all(|&s| s == 0), "la referencia dice lo que sonó: nada");
        assert!(track.lock().unwrap().queue.len() < before, "sordo también consume");

        mixer.set_deaf(false);
        render_block(&mixer, &mut cursor);
        assert!(render_block(&mixer, &mut cursor).iter().any(|&v| v != 0.0), "al volver a oír, suena");
    }

    /// Dos mezclas no comparten nada: lo de una no suena en la otra ni entra
    /// en su referencia. Era el riesgo de los globales del spike.
    #[test]
    fn two_mixers_share_nothing() {
        let (a, b) = (Mixer::default(), Mixer::default());
        loud_track(&a, 10_000);
        a.set_deaf(true);
        let mut cursor = OutputCursor::new(48_000);
        render_block(&a, &mut cursor);
        assert_eq!(b.track_count(), 0);
        let mut cursor_b = OutputCursor::new(48_000);
        render_block(&b, &mut cursor_b);
        assert!(b.take_reference().iter().flatten().all(|&s| s == 0));
        loud_track(&b, 3_000);
        render_block(&b, &mut cursor_b);
        assert!(render_block(&b, &mut cursor_b).iter().any(|&v| v != 0.0), "la sordera de una no ensordece a la otra");
    }

    /// Colgar deja la mezcla vacía.
    #[test]
    fn clear_forgets_tracks_and_reference() {
        let mixer = Mixer::default();
        loud_track(&mixer, 1_000);
        render_block(&mixer, &mut OutputCursor::new(48_000));
        mixer.clear();
        assert_eq!(mixer.track_count(), 0);
        assert!(mixer.take_reference().is_empty());
    }
}
