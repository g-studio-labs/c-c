//! La salida: un flujo de cpal que pide la mezcla de la llamada.
//!
//! Portado de `ensure_output` (spike y voice.rs). Una sola para todas las
//! pistas, que es lo que permite saber qué suena y dárselo al cancelador. Lo
//! que hace con el sonido está en `Mixer::render`, que se prueba sin tarjeta;
//! aquí sólo se abre el dispositivo.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, StreamTrait};

use crate::capture::{ThreadGuard, OPEN_TIMEOUT};
use crate::devices::{self, DeviceKind};
use crate::error::VoiceError;
use crate::mixer::{Mixer, OutputCursor};

/// Abre la salida (la elegida, o la del sistema) sobre `mixer`. **Bloquea**
/// hasta que abre o hasta `OPEN_TIMEOUT`.
pub(crate) fn open(chosen: Option<String>, mixer: Arc<Mixer>) -> Result<ThreadGuard, VoiceError> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), VoiceError>>();
    std::thread::Builder::new()
        .name("cac-voice-out".into())
        .spawn(move || {
            let host = cpal::default_host();
            let Some(device) = devices::find(&host, DeviceKind::Output, chosen.as_deref()) else {
                let _ = ready_tx.send(Err(VoiceError::NoOutput));
                return;
            };
            let config = match device.default_output_config() {
                Ok(config) => config,
                Err(e) => {
                    let _ = ready_tx.send(Err(VoiceError::OutputOpen(format!("el altavoz no dice su formato: {e}"))));
                    return;
                }
            };
            let channels = config.channels() as usize;
            let rate = config.sample_rate();
            log::info!("altavoz: «{device}» a {rate} Hz, {channels} canal(es)");
            let mut cursor = OutputCursor::new(rate);
            let stream = device.build_output_stream(
                config.config(),
                move |output: &mut [f32], _: &cpal::OutputCallbackInfo| mixer.render(output, channels, &mut cursor),
                |e| log::warn!("altavoz: error de reproducción: {e}"),
                None,
            );
            let stream = match stream {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = ready_tx.send(Err(VoiceError::OutputOpen(format!("no se pudo abrir «{device}»: {e}"))));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(VoiceError::OutputOpen(format!("no se pudo arrancar «{device}»: {e}"))));
                return;
            }
            let _ = ready_tx.send(Ok(()));
            let _ = stop_rx.recv();
            drop(stream);
        })
        .map_err(|e| VoiceError::OutputOpen(format!("no se pudo lanzar el hilo del altavoz: {e}")))?;
    match ready_rx.recv_timeout(OPEN_TIMEOUT) {
        Ok(Ok(())) => Ok(ThreadGuard::new(stop_tx)),
        Ok(Err(e)) => Err(e),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(VoiceError::OutputTimeout),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(VoiceError::OutputOpen("el hilo del altavoz murió sin decir nada".into())),
    }
}

/// Lo que suena en una llamada: la mezcla y el flujo que la reproduce.
///
/// La salida se abre con la primera pista que llega, no al entrar: solo en la
/// sala no hay nada que oír.
pub(crate) struct Playback {
    pub(crate) mixer: Arc<Mixer>,
    output: Mutex<Option<ThreadGuard>>,
    /// Hay una apertura en curso: una segunda pista que llega mientras tanto
    /// no abre otra.
    opening: AtomicBool,
    /// Se colgó: lo que termine de abrirse después se suelta.
    closed: AtomicBool,
}

impl Playback {
    pub(crate) fn new(mixer: Arc<Mixer>) -> Self {
        Playback { mixer, output: Mutex::new(None), opening: AtomicBool::new(false), closed: AtomicBool::new(false) }
    }

    /// Abre la salida si no lo está. Bloquea: en `spawn_blocking`.
    pub(crate) fn ensure_output(&self, chosen: Option<String>) -> Result<(), VoiceError> {
        if self.closed.load(Ordering::SeqCst) || self.output.lock().unwrap().is_some() || self.opening.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let opened = open(chosen, self.mixer.clone());
        self.opening.store(false, Ordering::SeqCst);
        let guard = opened?;
        if !self.closed.load(Ordering::SeqCst) {
            *self.output.lock().unwrap() = Some(guard);
        }
        Ok(())
    }

    /// Cambia de altavoz en caliente: abre el nuevo y **después** suelta el
    /// viejo, para que un fallo al abrir no deje la llamada muda. Si todavía
    /// no sonaba nada no abre: la primera pista usará el elegido. Mientras el
    /// nuevo arranca, los dos tiran de la mezcla un instante (un tropiezo de
    /// milisegundos, el mismo precio que paga el micrófono al cambiarlo).
    pub(crate) fn reopen(&self, chosen: Option<String>) -> Result<(), VoiceError> {
        if self.closed.load(Ordering::SeqCst) || self.output.lock().unwrap().is_none() {
            return Ok(());
        }
        let fresh = open(chosen, self.mixer.clone())?;
        if !self.closed.load(Ordering::SeqCst) {
            // El viejo se suelta aquí, al sustituirlo.
            *self.output.lock().unwrap() = Some(fresh);
        }
        Ok(())
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.output.lock().unwrap().take();
        self.mixer.clear();
    }
}
