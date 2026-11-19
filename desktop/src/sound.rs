//! El tono de los avisos, hecho sonar.
//!
//! Cuál suena y cuándo lo decide el core (`cac_core::notice_sound`); aquí se
//! oye. Por `cpal`, la misma biblioteca de audio que la voz de la app de
//! Tauri, sin decodificadores: las muestras ya vienen hechas.
//!
//! Sonar no puede costarle nada a la pantalla: el sonido va en un hilo suyo,
//! y lo que falle (una máquina sin salida de audio, un servidor de sonido
//! caído) se apunta en el registro y se olvida. Sin sonido, el aviso sale
//! igual.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cac_core::{Chime, NoticeKind, RingTone, Tone};
use gpui_kit::{App, Global};

use crate::backend::Backend;

/// Quien hace sonar un tono. Fuera de las pruebas, la salida de audio.
pub trait TonePlayer: Send + Sync {
    fn play(&self, tone: Tone);
    /// El timbre, en bucle hasta que se le diga otro o ninguno.
    fn ring(&self, tone: Option<RingTone>);
}

/// El reproductor de la app y cuándo sonó el último.
struct Sound {
    player: Arc<dyn TonePlayer>,
    chime: Chime,
    /// El timbre que está sonando: sólo se le dice al reproductor si cambia.
    ringing: Option<RingTone>,
}

impl Global for Sound {}

/// Lo prepara al arrancar, con quien va a sonar.
pub fn install(player: Arc<dyn TonePlayer>, cx: &mut App) {
    cx.set_global(Sound { player, chime: Chime::default(), ringing: None });
}

/// El timbre que toca ahora (lo decide el core: `ring_tone_now`). Se puede
/// llamar en cada pintado: sólo cambia algo si cambia el tono.
pub fn ring(tone: Option<RingTone>, cx: &mut App) {
    if !cx.has_global::<Sound>() {
        return;
    }
    let sound = cx.global_mut::<Sound>();
    if sound.ringing != tone {
        sound.ringing = tone;
        sound.player.ring(tone);
    }
}

/// Suena el tono de un aviso que se acaba de enseñar, si toca: encendido en
/// esta máquina y sin otro hace nada.
pub fn after_notice(kind: NoticeKind, cx: &mut App) {
    // En una llamada no: un tono se metería en lo que se oye (lo decide el core).
    let enabled = cx.try_global::<Backend>().is_some_and(|_| Backend::core(cx).notice_sound_allowed());
    if !cx.has_global::<Sound>() {
        return;
    }
    let sound = cx.global_mut::<Sound>();
    if sound.chime.should_play(Instant::now(), enabled) {
        sound.player.play(cac_core::tone_for(kind));
    }
}

/// La salida de audio de la máquina, en un hilo suyo que se arranca con el
/// primer tono. El timbre va en otro: dura lo que dure la llamada y no puede
/// hacer esperar a un aviso.
pub struct Speaker {
    sender: Mutex<Option<Sender<Tone>>>,
    ringing: Arc<Mutex<Option<RingTone>>>,
    ring_thread: Mutex<bool>,
}

impl Speaker {
    pub fn new() -> Self {
        Speaker { sender: Mutex::new(None), ringing: Arc::default(), ring_thread: Mutex::new(false) }
    }
}

/// Suena el timbre que diga `ringing` mientras diga el mismo; con otro, se
/// cambia; con ninguno, se suelta la salida y se espera.
fn ring_loop(ringing: Arc<Mutex<Option<RingTone>>>) {
    loop {
        let Some(tone) = *ringing.lock().expect("ring lock") else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        if let Err(error) = ring_now(tone, &ringing) {
            log::warn!("no sonó el timbre: {error}");
            // Sin salida no se reintenta en bucle: se espera a otro tono.
            while *ringing.lock().expect("ring lock") == Some(tone) {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

/// Abre la salida y repite el ciclo del tono hasta que deje de ser el que
/// toca.
fn ring_now(tone: RingTone, ringing: &Arc<Mutex<Option<RingTone>>>) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::SampleFormat;

    let device = cpal::default_host().default_output_device().ok_or("no hay salida de audio")?;
    let supported = device.default_output_config().map_err(|error| error.to_string())?;
    let config = supported.config();
    let cycle = cac_core::render_ring(tone, config.sample_rate);
    // El ciclo, una y otra vez: el flujo no se acaba solo.
    let samples: Vec<f32> = cycle.iter().copied().cycle().take(cycle.len() * 2000).collect();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => stream::<f32>(&device, &config, samples),
        SampleFormat::I16 => stream::<i16>(&device, &config, samples),
        SampleFormat::U16 => stream::<u16>(&device, &config, samples),
        SampleFormat::I32 => stream::<i32>(&device, &config, samples),
        other => return Err(format!("formato de audio sin soporte: {other}")),
    }?;
    stream.play().map_err(|error| error.to_string())?;
    while *ringing.lock().expect("ring lock") == Some(tone) {
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

impl TonePlayer for Speaker {
    fn ring(&self, tone: Option<RingTone>) {
        *self.ringing.lock().expect("ring lock") = tone;
        let mut started = self.ring_thread.lock().expect("ring thread lock");
        if tone.is_some() && !*started {
            let ringing = self.ringing.clone();
            match std::thread::Builder::new().name("cac-ring".into()).spawn(move || ring_loop(ringing)) {
                Ok(_) => *started = true,
                Err(error) => log::warn!("no se pudo arrancar el timbre: {error}"),
            }
        }
    }

    fn play(&self, tone: Tone) {
        let mut sender = self.sender.lock().expect("speaker lock");
        if sender.is_none() {
            let (send, receive) = channel::<Tone>();
            let spawned = std::thread::Builder::new().name("cac-sound".into()).spawn(move || {
                for tone in receive {
                    if let Err(error) = play_now(tone) {
                        log::warn!("no sonó el aviso: {error}");
                    }
                }
            });
            match spawned {
                Ok(_) => *sender = Some(send),
                Err(error) => {
                    log::warn!("no se pudo arrancar el sonido: {error}");
                    return;
                }
            }
        }
        if let Some(sender) = sender.as_ref() {
            let _ = sender.send(tone);
        }
    }
}

/// Abre la salida, suena el tono entero y la suelta. Abrirla por tono, y no
/// tenerla abierta, es lo que deja dormir al servidor de sonido entre
/// avisos, y lo que hace que conectar unos auriculares se note en el
/// siguiente.
fn play_now(tone: Tone) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::SampleFormat;

    let device = cpal::default_host().default_output_device().ok_or("no hay salida de audio")?;
    let supported = device.default_output_config().map_err(|error| error.to_string())?;
    let config = supported.config();
    let samples = cac_core::render(tone, config.sample_rate);
    let length = Duration::from_secs_f32(samples.len() as f32 / config.sample_rate as f32);
    let stream = match supported.sample_format() {
        SampleFormat::F32 => stream::<f32>(&device, &config, samples),
        SampleFormat::I16 => stream::<i16>(&device, &config, samples),
        SampleFormat::U16 => stream::<u16>(&device, &config, samples),
        SampleFormat::I32 => stream::<i32>(&device, &config, samples),
        other => return Err(format!("formato de audio sin soporte: {other}")),
    }?;
    stream.play().map_err(|error| error.to_string())?;
    // Un poco más que el tono: lo último que se escribe todavía tiene que
    // salir por el altavoz.
    std::thread::sleep(length + Duration::from_millis(150));
    Ok(())
}

/// Un flujo que escribe el tono, mono, en todos los canales, y luego
/// silencio.
fn stream<T>(device: &cpal::Device, config: &cpal::StreamConfig, samples: Vec<f32>) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;

    let channels = config.channels as usize;
    let mut next = samples.into_iter();
    device
        .build_output_stream(
            config.clone(),
            move |data: &mut [T], _| {
                for frame in data.chunks_mut(channels) {
                    let value = T::from_sample(next.next().unwrap_or(0.0));
                    frame.fill(value);
                }
            },
            |error| log::warn!("el audio del aviso falló: {error}"),
            None,
        )
        .map_err(|error| error.to_string())
}

#[cfg(test)]
pub mod tests {
    use std::sync::{Arc, Mutex};

    use cac_core::{Announcement, Conversation, NoticeKind, NoticeTarget, Tone};
    use gpui_kit::TestAppContext;

    use super::TonePlayer;
    use crate::backend::Backend;
    use crate::testing::FakeServer;

    /// Un reproductor que apunta lo que le piden, para las pruebas.
    #[derive(Default)]
    pub struct Recording(pub Mutex<Vec<Tone>>, pub Mutex<Vec<Option<cac_core::RingTone>>>);

    impl TonePlayer for Recording {
        fn play(&self, tone: Tone) {
            self.0.lock().unwrap().push(tone);
        }
        fn ring(&self, tone: Option<cac_core::RingTone>) {
            self.1.lock().unwrap().push(tone);
        }
    }

    fn announcement(kind: NoticeKind) -> Announcement {
        Announcement {
            kind,
            tag: "chat:a".into(),
            target: NoticeTarget { conversation: Conversation::Channel { space_id: "a".into() }, org_id: Some("o1".into()) },
            place: "Producto".into(),
            author: "Luis Mora".into(),
            preview: "¿sale hoy?".into(),
        }
    }

    /// Un aviso suena su tono después de enseñarse; apagado no suena; y dos
    /// seguidos suenan una vez.
    ///
    /// Roturas que atrapa: no sonar; sonar con el interruptor apagado; un
    /// tono por cada aviso de una ráfaga; el tono de un mensaje para una
    /// mención; que sonar dependa de que el aviso no se haya enseñado.
    #[gpui_kit::test]
    fn a_notice_rings_once_and_not_when_turned_off(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        server.install(cx);
        let played = Arc::new(Recording::default());
        cx.update(|cx| super::install(played.clone(), cx));

        cx.update(|cx| Backend::core(cx).set_notify_sound(false).unwrap());
        cx.update(|cx| crate::notifier::show(announcement(NoticeKind::ChannelMessage), cx));
        assert_eq!(cx.shown_system_notifications().len(), 1, "apagado, el aviso sale igual");
        assert!(played.0.lock().unwrap().is_empty(), "pero no suena");

        cx.update(|cx| Backend::core(cx).set_notify_sound(true).unwrap());
        cx.update(|cx| crate::notifier::show(announcement(NoticeKind::Mention), cx));
        assert_eq!(*played.0.lock().unwrap(), [Tone::TwoNotes], "encendido suena, con el tono de lo que te nombra");
        cx.update(|cx| crate::notifier::show(announcement(NoticeKind::ChannelMessage), cx));
        assert_eq!(cx.shown_system_notifications().len(), 3);
        assert_eq!(played.0.lock().unwrap().len(), 1, "dos seguidos suenan una vez");
    }
}
