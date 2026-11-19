//! El micrófono: de cpal a tramas de 10 ms, procesadas y publicadas.
//!
//! Portado de `start_capture` del spike y `start_capture_thread` de
//! `app/src-tauri/src/voice.rs`. Todo lo que allí eran globales (`MUTED`,
//! `NOISE_SUPPRESSION`, `PEAK_PERMILLE`, `PUBLISHING`…) vive en `MicState`,
//! uno por llamada.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::native::apm::AudioProcessingModule;
use livekit::webrtc::prelude::AudioFrame;
use serde::Serialize;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::devices::{self, DeviceKind};
use crate::dsp::{drain_frames, interleaved_to_mono, peak_permille, resample, Phase, VoiceActivity, CHANNELS, SAMPLES_PER_FRAME, SAMPLE_RATE};
use crate::error::VoiceError;
use crate::events::{EngineEvent, EventSink};
use crate::mixer::Mixer;

/// Lo que se tarda como mucho en abrir un dispositivo de audio. Abrirlo puede
/// colgarse (Windows reenumerando); sin plazo, `join` no volvía nunca.
pub(crate) const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

/// Con qué formato quedó abierto el micrófono: la primera pregunta de
/// cualquier «no se me oye».
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputFormat {
    pub device: String,
    pub rate_hz: u32,
    pub channels: u16,
    pub sample_format: String,
    /// Si el dispositivo ya va a 48 kHz. Si no, se remuestrea (bien), pero es
    /// lo primero que mirar si la voz suena rara.
    pub matches_rate: bool,
}

/// El estado del micrófono de una llamada.
pub(crate) struct MicState {
    /// Lo que se publica son ceros (silenciado o ensordecido). La fase del
    /// remuestreo sigue corriendo.
    pub(crate) silent: AtomicBool,
    pub(crate) noise_suppression: AtomicBool,
    /// El pico de la última trama enviada, en tanto por mil.
    pub(crate) peak: AtomicU32,
    /// Cuántos bucles de publicación hay vivos. Un contador y no un booleano:
    /// al cambiar de micrófono conviven el viejo y el nuevo un instante, y el
    /// viejo al morir apagaba el «publicando» del nuevo.
    pub(crate) publishers: AtomicU32,
    pub(crate) frames_published: AtomicU64,
    pub(crate) last_error: Mutex<Option<String>>,
    pub(crate) input_format: Mutex<Option<InputFormat>>,
}

impl MicState {
    pub(crate) fn new(noise_suppression: bool) -> Self {
        MicState {
            silent: AtomicBool::new(false),
            noise_suppression: AtomicBool::new(noise_suppression),
            peak: AtomicU32::new(0),
            publishers: AtomicU32::new(0),
            frames_published: AtomicU64::new(0),
            last_error: Mutex::new(None),
            input_format: Mutex::new(None),
        }
    }
}

/// Lo que necesita una captura. Se clona para cada una: cambiar de micrófono
/// levanta otra sobre la misma fuente publicada.
#[derive(Clone)]
pub(crate) struct CaptureCtx {
    pub(crate) source: NativeAudioSource,
    pub(crate) mic: Arc<MicState>,
    /// De donde sale la referencia del eco: lo que suena en **esta** llamada.
    pub(crate) mixer: Arc<Mixer>,
    pub(crate) sink: EventSink,
    pub(crate) handle: Handle,
}

/// Un hilo que mantiene vivo un flujo de cpal (que no es `Send`) hasta que se
/// suelta esto. La forma honesta de guardar un flujo en algo compartido.
pub(crate) struct ThreadGuard(Option<std::sync::mpsc::Sender<()>>);

impl ThreadGuard {
    pub(crate) fn new(stop: std::sync::mpsc::Sender<()>) -> Self {
        ThreadGuard(Some(stop))
    }
}

impl Drop for ThreadGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            let _ = stop.send(());
        }
    }
}

/// Abre el micrófono (el elegido, o el del sistema) y empieza a publicar.
/// **Bloquea** hasta que abre o hasta `OPEN_TIMEOUT`: desde código async, en
/// `spawn_blocking`.
///
/// Sin probar: desenchufar el micrófono a media llamada (el flujo falla, se
/// apunta en `last_error` y la voz se para; el de Tauri tampoco lo recupera).
pub(crate) fn start(ctx: CaptureCtx, chosen: Option<String>) -> Result<ThreadGuard, VoiceError> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), VoiceError>>();
    std::thread::Builder::new()
        .name("cac-voice-mic".into())
        .spawn(move || {
            let host = cpal::default_host();
            let Some(device) = devices::find(&host, DeviceKind::Input, chosen.as_deref()) else {
                let _ = ready_tx.send(Err(VoiceError::NoMic));
                return;
            };
            let config = match device.default_input_config() {
                Ok(config) => config,
                Err(e) => {
                    let _ = ready_tx.send(Err(VoiceError::MicOpen(format!("el micrófono no dice su formato: {e}"))));
                    return;
                }
            };
            let input_rate = config.sample_rate();
            let input_channels = config.channels() as usize;
            let format = InputFormat {
                device: device.to_string(),
                rate_hz: input_rate,
                channels: config.channels(),
                sample_format: format!("{:?}", config.sample_format()),
                matches_rate: input_rate == SAMPLE_RATE,
            };
            log::info!("micro: «{}» a {} Hz, {} canal(es), {}", format.device, format.rate_hz, format.channels, format.sample_format);
            *ctx.mic.input_format.lock().unwrap() = Some(format);

            let (frames_tx, frames_rx) = mpsc::unbounded_channel::<Vec<i16>>();
            // Muestras ya a 48 kHz: una trama de `SAMPLES_PER_FRAME` son
            // siempre 10 ms, venga el dispositivo al ritmo que venga.
            let mut accumulator: Vec<i16> = Vec::with_capacity(SAMPLES_PER_FRAME * 2);
            let mut phase = Phase::default();
            let step = input_rate as f32 / SAMPLE_RATE as f32;
            let (mic, mic_for_errors) = (ctx.mic.clone(), ctx.mic.clone());
            let stream = device.build_input_stream(
                config.config(),
                move |samples: &[f32], _: &cpal::InputCallbackInfo| {
                    let silent = mic.silent.load(Ordering::Relaxed);
                    resample(&interleaved_to_mono(samples, input_channels), step, &mut phase, silent, &mut accumulator);
                    for frame in drain_frames(&mut accumulator) {
                        let _ = frames_tx.send(frame);
                    }
                },
                move |e| {
                    log::warn!("micro: el flujo de captura falló: {e}");
                    *mic_for_errors.last_error.lock().unwrap() = Some(e.to_string());
                },
                None,
            );
            let stream = match stream {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = ready_tx.send(Err(VoiceError::MicOpen(format!("no se pudo abrir «{device}»: {e}"))));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(VoiceError::MicOpen(format!("no se pudo arrancar «{device}»: {e}"))));
                return;
            }
            // El procesado y la publicación, en el runtime: `capture_frame` es
            // asíncrono. Acaba sola cuando este flujo se suelta (se cierra
            // `frames_tx`).
            ctx.handle.spawn(publish(ctx.clone(), frames_rx));
            let _ = ready_tx.send(Ok(()));
            let _ = stop_rx.recv();
            drop(stream);
        })
        .map_err(|e| VoiceError::MicOpen(format!("no se pudo lanzar el hilo del micrófono: {e}")))?;
    match ready_rx.recv_timeout(OPEN_TIMEOUT) {
        Ok(Ok(())) => Ok(ThreadGuard::new(stop_tx)),
        Ok(Err(e)) => Err(e),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            log::warn!("micro: quince segundos sin abrir el dispositivo, se abandona");
            Err(VoiceError::MicTimeout)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(VoiceError::MicThreadDied),
    }
}

/// Procesa cada trama, la mide y la publica.
async fn publish(ctx: CaptureCtx, mut frames: mpsc::UnboundedReceiver<Vec<i16>>) {
    let mut processor = Processor::default();
    let mut activity = VoiceActivity::default();
    let mut warned = false;
    ctx.mic.publishers.fetch_add(1, Ordering::Relaxed);
    while let Some(mut frame) = frames.recv().await {
        // Primero el procesado y todo lo demás detrás: el medidor y «estás
        // hablando» miran lo que de verdad se manda. Medido antes, el
        // ventilador encendía el aro de hablar.
        let on = ctx.mic.noise_suppression.load(Ordering::Relaxed);
        if let Err(e) = processor.process_frame(on, &mut frame, ctx.mixer.take_reference()) {
            if !warned {
                log::warn!("micro: trama sin procesar: {e}");
                warned = true;
            }
        }
        ctx.mic.peak.store(peak_permille(&frame), Ordering::Relaxed);
        if let Some(speaking) = activity.update(&frame, Instant::now()) {
            (ctx.sink)(EngineEvent::SelfSpeaking { speaking });
        }
        let rtc_frame = AudioFrame { data: frame.into(), sample_rate: SAMPLE_RATE, num_channels: CHANNELS, samples_per_channel: SAMPLES_PER_FRAME as u32 };
        // Un rechazo del SDK no puede pasar en silencio: era el fallo
        // «micrófono abierto, nadie te oye» de la v1.6.5x.
        if let Err(e) = ctx.source.capture_frame(&rtc_frame).await {
            log::warn!("micro: el SDK rechazó una trama y se deja de publicar: {e}");
            *ctx.mic.last_error.lock().unwrap() = Some(e.to_string());
            break;
        }
        ctx.mic.frames_published.fetch_add(1, Ordering::Relaxed);
    }
    ctx.mic.publishers.fetch_sub(1, Ordering::Relaxed);
}

/// El procesado de la voz que WebRTC no hace por nosotros.
///
/// Con una fuente nativa (`capture_frame`) las opciones de
/// `AudioSourceOptions` **no se aplican**: las tramas van directas a los
/// sumideros. El procesado de verdad es éste: cancelación de eco (con lo que
/// suena como referencia), paso alto y supresión de ruido, y **sin control de
/// ganancia**, que con el cancelador encendido volvía a subir el ruido que el
/// supresor acababa de bajar (medido en voice.rs, `processing_tests`).
///
/// Apagado (música, un instrumento) no toca nada, eco incluido.
#[derive(Default)]
pub(crate) struct Processor {
    apm: Option<AudioProcessingModule>,
}

impl Processor {
    /// Procesa una trama en su sitio, después de darle al cancelador lo que
    /// sonó desde la anterior. Apagado suelta el módulo: al volver empieza
    /// limpio.
    ///
    /// `process_stream` **hace panic** si la longitud no es múltiplo de 10 ms:
    /// se comprueba antes, y una trama que no cuadra se devuelve sin tocar.
    pub(crate) fn process_frame(&mut self, on: bool, frame: &mut [i16], reference: Vec<Vec<i16>>) -> Result<(), String> {
        if !on {
            self.apm = None;
            return Ok(());
        }
        if frame.is_empty() || !frame.len().is_multiple_of(SAMPLES_PER_FRAME) {
            return Err(format!("trama de {} muestras, no múltiplo de 10 ms", frame.len()));
        }
        // (eco, ganancia, paso alto, ruido).
        let apm = self.apm.get_or_insert_with(|| AudioProcessingModule::new(true, false, true, true));
        for mut played in reference {
            apm.process_reverse_stream(&mut played, SAMPLE_RATE as i32, CHANNELS as i32).map_err(|e| e.to_string())?;
        }
        apm.process_stream(frame, SAMPLE_RATE as i32, CHANNELS as i32).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una trama que no es de 10 ms no llega al módulo (que haría panic y se
    /// llevaría la captura): vuelve como error y sin tocar.
    #[test]
    fn a_frame_that_is_not_10ms_is_refused_not_a_panic() {
        let mut processor = Processor::default();
        let mut odd = vec![1_000i16; SAMPLES_PER_FRAME - 1];
        assert!(processor.process_frame(true, &mut odd, Vec::new()).is_err());
        assert!(odd.iter().all(|&s| s == 1_000));
        let mut good = vec![0i16; SAMPLES_PER_FRAME];
        assert!(processor.process_frame(true, &mut good, vec![vec![0; SAMPLES_PER_FRAME]]).is_ok());
    }

    /// Apagado no toca nada: para mandar música tal cual.
    #[test]
    fn off_leaves_the_sound_untouched() {
        let mut processor = Processor::default();
        let original: Vec<i16> = (0..SAMPLES_PER_FRAME).map(|i| ((i as f32 * 0.3).sin() * 9_000.0) as i16).collect();
        let mut frame = original.clone();
        processor.process_frame(false, &mut frame, vec![original.clone()]).unwrap();
        assert_eq!(frame, original);
    }
}
