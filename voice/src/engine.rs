//! El motor y la llamada: lo que ve quien lo usa.
//!
//! `Engine` guarda lo que sobrevive a una llamada (los dispositivos elegidos,
//! la supresión de ruido) y abre llamadas; `Call` es una llamada en curso. No
//! hay ningún global: dos `Engine` son dos motores que no se ven.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::StreamExt;
use livekit::options::TrackPublishOptions;
use livekit::track::{LocalAudioTrack, LocalTrack, RemoteTrack, TrackKind, TrackSource};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::prelude::{RtcAudioTrack, RtcVideoTrack, VideoRotation};
use livekit::webrtc::stats::RtcStats;
use livekit::webrtc::video_stream::native::NativeVideoStream;
use livekit::{Room, RoomEvent, RoomOptions};
use serde::Serialize;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::capture::{self, CaptureCtx, InputFormat, MicState, ThreadGuard};
use crate::devices::{self, DeviceKind, Devices};
use crate::dsp::{to_mono, CHANNELS, SAMPLE_RATE};
use crate::error::{redact, JoinError, VoiceError};
use crate::events::{arrival_events, EngineEvent, EventSink, Peer};
use crate::mixer::TrackQueue;
use crate::output::Playback;
use crate::recording::recording_event;
use crate::video::{self, Feed, TrackChange, VideoFeeds, VideoSource};

/// Cada cuánto se mide la latencia: un número para mirar de reojo.
const LATENCY_EVERY: Duration = Duration::from_secs(5);
/// Lo que se espera a que la sala cierre limpia al colgar.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Lo que es del motor y no de una llamada.
pub(crate) struct Shared {
    /// Los elegidos a mano. Sobreviven a la llamada: quien corrigió el
    /// micrófono una vez no tiene que hacerlo en cada sala.
    chosen_input: Mutex<Option<String>>,
    chosen_output: Mutex<Option<String>>,
    /// La de la última llamada, para la siguiente.
    noise_suppression: AtomicBool,
    /// La llamada en curso, débil: el motor no la mantiene viva, y cambiar de
    /// dispositivo la encuentra para aplicarlo en caliente.
    current: Mutex<Weak<CallInner>>,
}

impl Default for Shared {
    fn default() -> Self {
        Shared { chosen_input: Mutex::new(None), chosen_output: Mutex::new(None), noise_suppression: AtomicBool::new(true), current: Mutex::new(Weak::new()) }
    }
}

impl Shared {
    fn slot(&self, kind: DeviceKind) -> &Mutex<Option<String>> {
        match kind {
            DeviceKind::Input => &self.chosen_input,
            DeviceKind::Output => &self.chosen_output,
        }
    }

    pub(crate) fn chosen(&self, kind: DeviceKind) -> Option<String> {
        self.slot(kind).lock().unwrap().clone()
    }
}

/// El motor de voz. Barato de crear y de clonar (los clones son el mismo motor).
#[derive(Clone, Default)]
pub struct Engine {
    shared: Arc<Shared>,
}

/// Cómo va el micrófono en la llamada, para el medidor y para «¿por qué no se
/// me oye?»: si entrega muestras, si traen señal, y si llegan a publicarse
/// son tres fallos distintos con el mismo síntoma.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicLevel {
    /// El pico de la última trama **enviada** (procesada y, si toca, en
    /// silencio), en tanto por mil de la escala completa.
    pub peak_permille: u32,
    pub publishing: bool,
    pub frames_published: u64,
    pub input: Option<InputFormat>,
    /// Lo último que dijo el micrófono o el SDK al romperse.
    pub error: Option<String>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Los micrófonos y altavoces que hay ahora, con el que se usa marcado.
    /// Pregunta al sistema cada vez (tarda unos milisegundos): mejor fuera del
    /// hilo de la interfaz.
    pub fn devices(&self) -> Devices {
        devices::list(self.shared.chosen(DeviceKind::Input).as_deref(), self.shared.chosen(DeviceKind::Output).as_deref())
    }

    /// Elige micrófono o altavoz (`None`: el del sistema). Queda para las
    /// siguientes llamadas, y si hay una en curso se aplica en caliente: el
    /// nuevo se abre antes de soltar el viejo, así que si falla la llamada
    /// sigue con el que tenía (y se devuelve el error; la preferencia queda).
    pub async fn set_device(&self, kind: DeviceKind, id: Option<String>) -> Result<(), VoiceError> {
        *self.shared.slot(kind).lock().unwrap() = id.clone();
        let Some(call) = self.current_call() else { return Ok(()) };
        match kind {
            DeviceKind::Input => call.restart_capture(id).await,
            DeviceKind::Output => {
                let playback = call.inner.playback.clone();
                tokio::task::spawn_blocking(move || playback.reopen(id)).await.map_err(|e| VoiceError::OutputOpen(e.to_string()))?
            }
        }
    }

    /// La supresión de ruido para la próxima llamada (y la que haya en curso,
    /// si hay): quien la apagó para tocar música no tiene que entrar con ella
    /// puesta y apagarla dentro.
    pub fn set_noise_suppression(&self, on: bool) {
        self.shared.noise_suppression.store(on, Ordering::Relaxed);
        if let Some(call) = self.current_call() {
            call.set_noise_suppression(on);
        }
    }

    /// La llamada en curso, si hay.
    pub fn current_call(&self) -> Option<Call> {
        self.shared.current.lock().unwrap().upgrade().map(|inner| Call { inner })
    }

    /// Entrar a una sala, publicar el micrófono y empezar a oír.
    ///
    /// Hay que esperarlo **dentro de un runtime de tokio** (LiveKit lo
    /// necesita); las tareas de la llamada se quedan en ese runtime. Una
    /// llamada a la vez: si había otra, se cuelga antes.
    ///
    /// Todo lo que falla después de conectar cierra la sala antes de volver:
    /// si no, quedabas publicado en el SFU como participante fantasma
    /// (voice.rs, el `armed`).
    pub async fn join(&self, url: &str, token: &str, sink: EventSink) -> Result<Call, JoinError> {
        let handle = Handle::try_current().map_err(|_| VoiceError::NoRuntime)?;
        if let Some(previous) = self.current_call() {
            previous.leave().await;
        }
        let (room, events) = Room::connect(url, token, RoomOptions::default()).await.map_err(|e| VoiceError::Connect(redact(e.to_string(), token)))?;
        let room = Arc::new(room);
        let identity = room.local_participant().identity().to_string();
        log::info!("sala: dentro como {identity}");

        // Estas opciones **no se aplican** a una fuente nativa: el procesado
        // de verdad es `capture::Processor`. Se dejan a `true` porque son lo
        // que se quiere y no cuestan nada.
        let source = NativeAudioSource::new(AudioSourceOptions { echo_cancellation: true, noise_suppression: true, auto_gain_control: true }, SAMPLE_RATE, CHANNELS, 1_000);
        let track = LocalAudioTrack::create_audio_track("micro", RtcAudioSource::Native(source.clone()));
        let mixer = Arc::new(crate::mixer::Mixer::default());
        let mic = Arc::new(MicState::new(self.shared.noise_suppression.load(Ordering::Relaxed)));
        let ctx = CaptureCtx { source, mic: mic.clone(), mixer: mixer.clone(), sink: sink.clone(), handle: handle.clone() };

        let armed = async {
            room.local_participant()
                .publish_track(LocalTrack::Audio(track.clone()), TrackPublishOptions { source: TrackSource::Microphone, ..Default::default() })
                .await
                .map_err(|e| VoiceError::Publish(redact(e.to_string(), token)))?;
            start_capture(ctx.clone(), self.shared.chosen(DeviceKind::Input)).await
        }
        .await;
        let capture = match armed {
            Ok(capture) => capture,
            Err(e) => {
                log::warn!("sala: entrada a medias, se cierra la sala: {e}");
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, room.close()).await;
                return Err(e);
            }
        };

        let playback = Arc::new(Playback::new(mixer));
        let video = VideoFeeds::default();
        let inner = Arc::new(CallInner {
            engine: self.shared.clone(),
            handle: handle.clone(),
            room: room.clone(),
            identity: identity.clone(),
            track,
            ctx,
            capture: Mutex::new(Some(capture)),
            playback: playback.clone(),
            mic,
            mic_wanted: AtomicBool::new(true),
            deaf: AtomicBool::new(false),
            left: AtomicBool::new(false),
            tasks: Mutex::new(Vec::new()),
            video: video.clone(),
        });
        *self.shared.current.lock().unwrap() = Arc::downgrade(&inner);

        sink(EngineEvent::Connected { identity });
        let listening = handle.spawn(listen(events, Arc::downgrade(&room), playback, video, self.shared.clone(), sink.clone(), handle.clone()));
        let latency = handle.spawn(measure_latency(Arc::downgrade(&room), sink));
        inner.tasks.lock().unwrap().extend([listening, latency]);
        Ok(Call { inner })
    }
}

/// Abre el micrófono fuera del runtime: abrir un dispositivo bloquea.
async fn start_capture(ctx: CaptureCtx, chosen: Option<String>) -> Result<ThreadGuard, VoiceError> {
    tokio::task::spawn_blocking(move || capture::start(ctx, chosen)).await.map_err(|_| VoiceError::MicThreadDied)?
}

/// ¿Se publica voz? Silenciar lo cierra, y ensordecer **también**: quien no
/// oye no debería estar hablando sin saber qué le contestan. Al dejar de estar
/// sordo vuelve a como lo tenías, no se abre solo.
fn mic_open(wanted: bool, deaf: bool) -> bool {
    wanted && !deaf
}

/// Una llamada en curso. Los clones son la misma llamada.
#[derive(Clone)]
pub struct Call {
    inner: Arc<CallInner>,
}

struct CallInner {
    engine: Arc<Shared>,
    /// El runtime donde se entró: el SDK necesita uno para silenciar
    /// (`mute()` avisa al servidor con un `tokio::spawn`), y la interfaz llama
    /// desde su hilo. En Tauri silenciar desde fuera de un runtime **cerraba
    /// la app**.
    handle: Handle,
    room: Arc<Room>,
    identity: String,
    track: LocalAudioTrack,
    ctx: CaptureCtx,
    capture: Mutex<Option<ThreadGuard>>,
    playback: Arc<Playback>,
    mic: Arc<MicState>,
    mic_wanted: AtomicBool,
    deaf: AtomicBool,
    left: AtomicBool,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    /// El vídeo de los demás (`video.rs`).
    video: VideoFeeds,
}

impl Call {
    /// Quién eres en la sala (la identidad del token).
    pub fn identity(&self) -> &str {
        &self.inner.identity
    }

    /// Abrir o cerrar el micrófono sin salirse. Se silencia **la pista**
    /// (viaja: a los demás les llega `Muted`) y además las muestras (lo que
    /// salga entre pulsar y que el servidor se entere son ceros). La captura
    /// sigue viva: volver a hablar es inmediato.
    pub fn set_mic(&self, enabled: bool) {
        self.inner.mic_wanted.store(enabled, Ordering::SeqCst);
        self.inner.apply_mic();
    }

    /// Dejar de oír: la salida da silencio (las pistas siguen llegando, y
    /// volver es instantáneo) y el micrófono se cierra. Ver `mic_open`.
    pub fn set_deaf(&self, deaf: bool) {
        self.inner.deaf.store(deaf, Ordering::SeqCst);
        self.inner.playback.mixer.set_deaf(deaf);
        self.inner.apply_mic();
    }

    /// Encender o apagar el procesado (eco, ruido, paso alto) en caliente; se
    /// apaga para música. Queda para la siguiente llamada.
    pub fn set_noise_suppression(&self, on: bool) {
        self.inner.mic.noise_suppression.store(on, Ordering::Relaxed);
        self.inner.engine.noise_suppression.store(on, Ordering::Relaxed);
    }

    pub fn is_mic_on(&self) -> bool {
        self.inner.mic_wanted.load(Ordering::SeqCst)
    }

    pub fn is_deaf(&self) -> bool {
        self.inner.deaf.load(Ordering::SeqCst)
    }

    /// El micrófono ahora mismo. No abre nada: lo mide la propia captura,
    /// sobre lo que de verdad se envía.
    pub fn mic_level(&self) -> MicLevel {
        let mic = &self.inner.mic;
        MicLevel {
            peak_permille: mic.peak.load(Ordering::Relaxed),
            publishing: mic.publishers.load(Ordering::Relaxed) > 0,
            frames_published: mic.frames_published.load(Ordering::Relaxed),
            input: mic.input_format.lock().unwrap().clone(),
            error: mic.last_error.lock().unwrap().clone(),
        }
    }

    /// El vídeo de los demás: sus últimos fotogramas, a qué tamaño se ven y si
    /// alguien mira. Los encendidos y apagados llegan como `EngineEvent::Video`.
    pub fn video(&self) -> VideoFeeds {
        self.inner.video.clone()
    }

    /// Colgar. Idempotente: colgar dos veces no es un error (pasa de verdad,
    /// la ventana se cierra mientras alguien pulsa el botón). No emite
    /// `Disconnected`.
    pub async fn leave(&self) {
        if self.inner.left.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.stop_local();
        match tokio::time::timeout(CLOSE_TIMEOUT, self.inner.room.close()).await {
            Ok(Ok(())) => log::info!("sala: fuera"),
            Ok(Err(e)) => log::warn!("sala: no cerró limpia: {e}"),
            Err(_) => log::warn!("sala: no contestó en cinco segundos; se sale igual"),
        }
    }

    /// Cambiar de micrófono en caliente: la fuente publicada es la misma, así
    /// que para la sala no pasa nada. La nueva captura se levanta **antes** de
    /// soltar la vieja: si fallara, la llamada no se queda muda.
    async fn restart_capture(&self, chosen: Option<String>) -> Result<(), VoiceError> {
        let fresh = start_capture(self.inner.ctx.clone(), chosen).await?;
        if !self.inner.left.load(Ordering::SeqCst) {
            *self.inner.capture.lock().unwrap() = Some(fresh);
        }
        Ok(())
    }
}

impl CallInner {
    fn apply_mic(&self) {
        let open = mic_open(self.mic_wanted.load(Ordering::SeqCst), self.deaf.load(Ordering::SeqCst));
        self.mic.silent.store(!open, Ordering::Relaxed);
        let _runtime = self.handle.enter();
        if open {
            self.track.unmute();
        } else {
            self.track.mute();
        }
    }

    /// Suelta todo lo de esta máquina: el micrófono, la salida y las tareas.
    fn stop_local(&self) {
        // Primero las tareas: la escucha deja de reproducir y de contar cosas
        // de una sala que ya se cuelga.
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
        self.capture.lock().unwrap().take();
        self.playback.close();
        // La última cara de esta sala no se enseña en la siguiente.
        self.video.clear();
        let mut current = self.engine.current.lock().unwrap();
        if std::ptr::eq(current.as_ptr(), self) {
            *current = Weak::new();
        }
    }
}

impl Drop for CallInner {
    /// Soltar la última `Call` sin colgar no deja la sala abierta: se cierra
    /// en el runtime, sin esperar.
    fn drop(&mut self) {
        if self.left.swap(true, Ordering::SeqCst) {
            return;
        }
        self.stop_local();
        let room = self.room.clone();
        self.handle.spawn(async move {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, room.close()).await;
        });
    }
}

/// Lo que suena de cada pista remota, por sid. Al soltarse (la escucha acabó o
/// se abortó) para las tareas y saca las pistas de la mezcla: abortar una
/// tarea no aborta las que lanzó.
struct Receiving {
    playback: Arc<Playback>,
    tracks: HashMap<String, (TrackQueue, JoinHandle<()>)>,
    video: VideoFeeds,
    /// Las pistas de vídeo que se están recibiendo: de quién, qué, y su tarea.
    videos: HashMap<String, (String, VideoSource, JoinHandle<()>)>,
}

impl Receiving {
    fn remove(&mut self, sid: &str) {
        if let Some((queue, task)) = self.tracks.remove(sid) {
            task.abort();
            self.playback.mixer.remove_track(&queue);
        }
        if let Some((identity, source, task)) = self.videos.remove(sid) {
            task.abort();
            self.video.close(&identity, source);
        }
    }
}

impl Drop for Receiving {
    fn drop(&mut self) {
        let sids: Vec<String> = self.tracks.keys().chain(self.videos.keys()).cloned().collect();
        for sid in sids {
            self.remove(&sid);
        }
    }
}

/// Los eventos de la sala, traducidos a `EngineEvent`, y lo que dicen los
/// demás, a la mezcla y a sus casillas de vídeo. Portado de
/// `listen_to_events` (voice.rs).
#[allow(clippy::too_many_arguments)]
async fn listen(mut events: mpsc::UnboundedReceiver<RoomEvent>, room: Weak<Room>, playback: Arc<Playback>, video: VideoFeeds, engine: Arc<Shared>, sink: EventSink, handle: Handle) {
    let mut receiving = Receiving { playback: playback.clone(), tracks: HashMap::new(), video: video.clone(), videos: HashMap::new() };
    // Un vídeo que se enciende o se apaga, contado y, si se apaga, sin su
    // último fotograma.
    let tell = |identity: String, change: Option<(VideoSource, bool)>| {
        if let Some((source, enabled)) = change {
            if !enabled {
                video.blank(&identity, source);
            }
            sink(EngineEvent::Video { identity, source, enabled });
        }
    };
    while let Some(event) = events.recv().await {
        match event {
            // Los que ya estaban: ver `arrival_events`.
            RoomEvent::Connected { participants_with_tracks } => {
                let peers: Vec<Peer> = participants_with_tracks
                    .iter()
                    .map(|(p, publications)| Peer {
                        identity: p.identity().to_string(),
                        name: p.name(),
                        mic_muted: publications.iter().find(|p| p.kind() == TrackKind::Audio).map(|p| p.is_muted()),
                    })
                    .collect();
                let metadata = room.upgrade().map(|r| r.metadata()).unwrap_or_default();
                for event in arrival_events(&peers, &metadata) {
                    sink(event);
                }
            }
            RoomEvent::RoomMetadataChanged { metadata, .. } => sink(recording_event(&metadata)),
            RoomEvent::ParticipantConnected(p) => sink(EngineEvent::Joined { identity: p.identity().to_string(), name: p.name() }),
            RoomEvent::ParticipantDisconnected(p) => {
                video.forget_person(p.identity().as_str());
                sink(EngineEvent::Left { identity: p.identity().to_string() })
            }
            RoomEvent::ActiveSpeakersChanged { speakers } => sink(EngineEvent::Speaking { identities: speakers.iter().map(|s| s.identity().to_string()).collect() }),
            RoomEvent::TrackMuted { participant, publication } | RoomEvent::TrackUnmuted { participant, publication } => {
                if publication.kind() == TrackKind::Audio {
                    sink(EngineEvent::Muted { identity: participant.identity().to_string(), muted: publication.is_muted() });
                }
                // Un vídeo silenciado está apagado: es como apaga la cámara el
                // navegador (ver `video::video_change`).
                let change = if publication.is_muted() { TrackChange::Muted } else { TrackChange::Unmuted };
                tell(participant.identity().to_string(), video::video_change(publication.kind(), publication.source(), change));
            }
            // Publica un micrófono estando ya dentro (se reconectó, cambió de
            // dispositivo): su estado de partida otra vez.
            RoomEvent::TrackPublished { publication, participant } => {
                if publication.kind() == TrackKind::Audio {
                    sink(EngineEvent::Muted { identity: participant.identity().to_string(), muted: publication.is_muted() });
                }
            }
            RoomEvent::TrackSubscribed { track: RemoteTrack::Audio(audio), publication, .. } => {
                let sid = publication.sid().to_string();
                let played = play_track(&handle, &playback, engine.chosen(DeviceKind::Output), audio.rtc_track());
                receiving.remove(&sid);
                receiving.tracks.insert(sid, played);
            }
            RoomEvent::TrackSubscribed { track: RemoteTrack::Video(track), publication, participant } => {
                let sid = publication.sid().to_string();
                let identity = participant.identity().to_string();
                let change = video::video_change(TrackKind::Video, publication.source(), TrackChange::Subscribed { muted: publication.is_muted() });
                if let Some((source, _)) = change {
                    receiving.remove(&sid);
                    let feed = video.open(&identity, source);
                    video.track_publication(&identity, source, publication.clone());
                    let task = receive_video(&handle, track.rtc_track(), feed, video.clone());
                    receiving.videos.insert(sid, (identity.clone(), source, task));
                }
                tell(identity, change);
            }
            RoomEvent::TrackUnsubscribed { publication, participant, .. } => {
                receiving.remove(&publication.sid().to_string());
                tell(participant.identity().to_string(), video::video_change(publication.kind(), publication.source(), TrackChange::Unsubscribed));
            }
            // Dejó de compartir: el SDK no siempre manda antes `TrackUnsubscribed`.
            RoomEvent::TrackUnpublished { publication, participant } => {
                receiving.remove(&publication.sid().to_string());
                tell(participant.identity().to_string(), video::video_change(publication.kind(), publication.source(), TrackChange::Unpublished));
            }
            RoomEvent::Disconnected { reason } => sink(EngineEvent::Disconnected { reason: format!("{reason:?}") }),
            _ => {}
        }
    }
}

/// Lo que dice otro, a la mezcla. Abre la salida si es la primera pista (en
/// un hilo aparte: abrir bloquea).
fn play_track(handle: &Handle, playback: &Arc<Playback>, chosen_output: Option<String>, track: RtcAudioTrack) -> (TrackQueue, JoinHandle<()>) {
    let opener = playback.clone();
    handle.spawn_blocking(move || {
        if let Err(e) = opener.ensure_output(chosen_output) {
            log::warn!("altavoz: {e}");
        }
    });
    let queue = playback.mixer.add_track();
    let (mine, mixer) = (queue.clone(), playback.mixer.clone());
    let task = handle.spawn(async move {
        // A 48 kHz y mono, como mezcla la salida: todas las pistas suman
        // muestra a muestra y la referencia del eco sale al ritmo de la
        // captura sin otro remuestreo.
        let mut incoming = NativeAudioStream::new(track, SAMPLE_RATE as i32, 1);
        while let Some(frame) = incoming.next().await {
            let mono = to_mono(&frame.data, frame.num_channels.max(1) as usize);
            mine.lock().unwrap().push_samples(&mono);
        }
        mixer.remove_track(&mine);
    });
    (queue, task)
}

/// Lo que llega de una pista de vídeo, a su casilla: cada fotograma se
/// convierte en esta tarea al tamaño en que se ve, y se queda sólo el último.
///
/// Cola de **uno** (la de `NativeVideoStream::new`): si convertir se
/// retrasa, el SDK guarda el fotograma más nuevo y tira los de en medio, que
/// es lo que se quiere en una llamada. El spike usaba `queue_size_frames:
/// Some(0)`, que es **sin tope**, para contar todo lo que llegaba; aquí
/// crecería sin fin con la máquina cargada.
fn receive_video(handle: &Handle, track: RtcVideoTrack, feed: Arc<Feed>, video: VideoFeeds) -> JoinHandle<()> {
    handle.spawn(async move {
        let mut incoming = NativeVideoStream::new(track);
        while let Some(frame) = incoming.next().await {
            let rotation: VideoRotation = frame.rotation;
            video.offer(&feed, |(box_w, box_h)| video::convert(&*frame.buffer, rotation, box_w, box_h));
        }
    })
}

/// La latencia de verdad: el ida y vuelta que mide WebRTC sobre el camino que
/// usa. La sala, débil: al colgar este bucle se entera y se acaba.
async fn measure_latency(room: Weak<Room>, sink: EventSink) {
    loop {
        tokio::time::sleep(LATENCY_EVERY).await;
        let Some(room) = room.upgrade() else { break };
        let Ok(stats) = room.get_stats().await else { break };
        drop(room);
        // La subida primero: es la mitad que uno controla. Sin par nominado
        // todavía no se dice nada: un cero se leería como conexión perfecta.
        if let Some(rtt) = nominated_rtt(&stats.publisher_stats).or_else(|| nominated_rtt(&stats.subscriber_stats)) {
            sink(EngineEvent::Latency { ms: (rtt * 1000.0).round() as u32 });
        }
    }
}

/// El ida y vuelta del par de candidatos **nominado**, que es por donde van
/// los paquetes. El primero de la lista o el mínimo dan un número bonito que
/// no corresponde a nada.
fn nominated_rtt(stats: &[RtcStats]) -> Option<f64> {
    stats.iter().find_map(|s| match s {
        RtcStats::CandidatePair(cp) if cp.candidate_pair.nominated && cp.candidate_pair.current_round_trip_time > 0.0 => Some(cp.candidate_pair.current_round_trip_time),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensordecer cierra el micrófono, y dejar de estarlo lo deja como lo
    /// tenías: abierto si lo estaba, cerrado si lo habías cerrado tú.
    #[test]
    fn deaf_closes_the_mic_and_undeafening_restores_it() {
        assert!(mic_open(true, false));
        assert!(!mic_open(true, true), "sordo no publica voz");
        assert!(!mic_open(false, true));
        assert!(!mic_open(false, false), "quitarse la sordera no abre un micro que cerraste tú");
    }

    /// Dos motores no comparten lo elegido.
    #[test]
    fn two_engines_do_not_share_choices() {
        let (a, b) = (Engine::new(), Engine::new());
        futures::executor::block_on(a.set_device(DeviceKind::Input, Some("alsa:hw:CARD=1".into()))).unwrap();
        futures::executor::block_on(a.set_device(DeviceKind::Output, Some("alsa:hw:CARD=2".into()))).unwrap();
        assert_eq!(a.shared.chosen(DeviceKind::Input).as_deref(), Some("alsa:hw:CARD=1"));
        assert_eq!(a.shared.chosen(DeviceKind::Output).as_deref(), Some("alsa:hw:CARD=2"));
        assert_eq!(b.shared.chosen(DeviceKind::Input), None);
        assert_eq!(b.shared.chosen(DeviceKind::Output), None);
        assert!(a.current_call().is_none() && b.current_call().is_none());
    }

    /// Sin runtime, `join` lo dice en vez de reventar.
    #[test]
    fn join_outside_a_runtime_is_an_error_not_a_panic() {
        let sink: EventSink = Arc::new(|_| {});
        let result = futures::executor::block_on(Engine::new().join("wss://nowhere.invalid", "t", sink));
        assert_eq!(result.err().map(|e| e.label()), Some("voice-no-runtime"));
    }
}
