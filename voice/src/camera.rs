//! Tu cámara: listarla, abrirla y capturarla en su propio hilo
//! (command-and-control-445). Portado de `usable_cameras` y `start_camera`
//! de `spikes/calls-native/src/call.rs`.
//!
//! Se elige **por nombre**: el índice de V4L2 se mueve al enchufar otra.

use std::sync::atomic::{AtomicBool, Ordering};

use livekit::webrtc::native::yuv_helper;
use livekit::webrtc::prelude::I420Buffer;
use livekit::webrtc::video_source::native::NativeVideoSource;
use tokio::runtime::Handle;

use crate::devices::Device;
use crate::error::VoiceError;
use crate::publish::{self, Capture, Outlet, Publishing, Started};

/// Fallos seguidos de la cámara antes de rendirse.
const STREAK_MAX: u32 = 30;

/// En qué orden probar las cámaras: las que se llaman como la elegida
/// primero; si no hay ninguna (se desenchufó), todas, que caer en otra es
/// mejor que no tener cámara. En Linux cada webcam sale dos veces con el
/// mismo nombre (el segundo nodo es de metadatos y no captura): se prueban
/// las dos.
pub(crate) fn candidates(names: &[String], chosen: Option<&str>) -> Vec<usize> {
    let matching: Vec<usize> = (0..names.len()).filter(|&i| Some(names[i].as_str()) == chosen).collect();
    if matching.is_empty() {
        (0..names.len()).collect()
    } else {
        matching
    }
}

/// Las cámaras como las enseña la pantalla: una por nombre, con la que se
/// usaría marcada. El `id` es el nombre, que es lo que se elige.
pub(crate) fn as_devices(names: &[String], chosen: Option<&str>) -> Vec<Device> {
    let current = candidates(names, chosen).first().map(|&i| names[i].clone());
    let mut seen = std::collections::HashSet::new();
    names.iter().filter(|name| seen.insert(name.as_str())).map(|name| Device { id: name.clone(), name: name.clone(), current: Some(name) == current.as_ref() }).collect()
}

/// Las cámaras que hay. Barato: no abre ninguna.
pub(crate) fn list(chosen: Option<&str>) -> Vec<Device> {
    as_devices(&native::names(), chosen)
}

/// Abre la cámara elegida en su hilo y empieza a entregar.
pub(crate) fn start(handle: Handle, chosen: Option<String>, publishing: Publishing) -> Result<(Capture, Started), VoiceError> {
    publish::spawn_capture("cámara", handle, publishing, move |outlet, stop| capture(chosen, outlet, stop))
}

fn capture(chosen: Option<String>, mut outlet: Outlet<NativeVideoSource>, stop: &AtomicBool) {
    let mut camera = match native::open(chosen.as_deref()) {
        Ok(camera) => camera,
        Err(e) => return outlet.fail(e),
    };
    let mut streak = 0;
    while !stop.load(Ordering::SeqCst) {
        match camera.next_frame() {
            Ok((rgba, width, height)) => {
                streak = 0;
                if !outlet.deliver(&rgba_to_i420(&rgba, width, height)) {
                    return;
                }
            }
            Err(e) => {
                streak += 1;
                if streak >= STREAK_MAX {
                    log::warn!("cámara: {STREAK_MAX} fallos seguidos, se apaga: {e}");
                    return outlet.fail(VoiceError::CameraNoFrames);
                }
            }
        }
    }
}

/// RGBA → I420 con libyuv. La prueba de viabilidad lo hacía a mano, píxel a
/// píxel, y se comía casi un núcleo. En libyuv «ABGR» es R, G, B, A en
/// memoria.
fn rgba_to_i420(rgba: &[u8], width: u32, height: u32) -> I420Buffer {
    let mut buffer = I420Buffer::new(width, height);
    let (stride_y, stride_u, stride_v) = buffer.strides();
    let (y, u, v) = buffer.data_mut();
    yuv_helper::abgr_to_i420(rgba, width * 4, y, stride_y, u, stride_u, v, stride_v, width as i32, height as i32);
    buffer
}

/// Lo que habla con V4L2 por `nokhwa`. Aparte, para que las reglas de arriba
/// se prueben sin cámara.
#[cfg(target_os = "linux")]
mod native {
    use nokhwa::pixel_format::RgbAFormat;
    use nokhwa::utils::{ApiBackend, CameraInfo, RequestedFormat, RequestedFormatType, Resolution};
    use nokhwa::Camera;

    use super::candidates;
    use crate::error::VoiceError;

    fn query() -> Vec<CameraInfo> {
        nokhwa::query(ApiBackend::Auto).unwrap_or_else(|e| {
            log::warn!("cámara: no se pudieron listar: {e}");
            Vec::new()
        })
    }

    pub(super) fn names() -> Vec<String> {
        query().iter().map(CameraInfo::human_name).collect()
    }

    pub(super) struct OpenCamera(Camera);

    impl OpenCamera {
        /// El siguiente fotograma, en RGBA, con sus medidas.
        pub(super) fn next_frame(&mut self) -> Result<(Vec<u8>, u32, u32), String> {
            let raw = self.0.frame().map_err(|e| format!("dejó de dar imagen: {e}"))?;
            let image = raw.decode_image::<RgbAFormat>().map_err(|e| format!("no se pudo descodificar: {e}"))?;
            let (width, height) = image.dimensions();
            Ok((image.into_raw(), width, height))
        }
    }

    impl Drop for OpenCamera {
        fn drop(&mut self) {
            let _ = self.0.stop_stream();
        }
    }

    /// La primera candidata que abra, pidiendo en orden 720p, 480p, la de
    /// más fotogramas y la de más resolución: `HighestResolution` exige la
    /// medida exacta, y pedir sólo 720p dejaba sin cámara a quien no la
    /// ofrecía (voice.rs de Tauri).
    pub(super) fn open(chosen: Option<&str>) -> Result<OpenCamera, VoiceError> {
        let cameras = query();
        let names: Vec<String> = cameras.iter().map(CameraInfo::human_name).collect();
        if cameras.is_empty() {
            return Err(VoiceError::NoCamera);
        }
        let formats = [
            RequestedFormatType::HighestResolution(Resolution::new(1280, 720)),
            RequestedFormatType::HighestResolution(Resolution::new(640, 480)),
            RequestedFormatType::AbsoluteHighestFrameRate,
            RequestedFormatType::AbsoluteHighestResolution,
        ];
        let mut reasons = Vec::new();
        for i in candidates(&names, chosen) {
            for format in formats {
                let opened = Camera::new(cameras[i].index().clone(), RequestedFormat::new::<RgbAFormat>(format)).and_then(|mut camera| camera.open_stream().map(|()| camera));
                match opened {
                    Ok(camera) => {
                        log::info!("cámara: «{}» abierta a {}", names[i], camera.camera_format());
                        return Ok(OpenCamera(camera));
                    }
                    Err(e) => reasons.push(e.to_string()),
                }
            }
        }
        log::warn!("cámara: ninguna abrió: {}", reasons.join("; "));
        // EBUSY: la tiene otro programa. Lo único que se distingue y que la
        // persona puede arreglar.
        if reasons.iter().any(|r| r.to_lowercase().contains("busy")) {
            return Err(VoiceError::CameraBusy);
        }
        Err(VoiceError::CameraFailed(reasons.pop().unwrap_or_default()))
    }
}

/// Fuera de Linux no hay cámara todavía (macOS y Windows, fuera de esta fase).
#[cfg(not(target_os = "linux"))]
mod native {
    use crate::error::VoiceError;

    pub(super) fn names() -> Vec<String> {
        Vec::new()
    }

    pub(super) struct OpenCamera;

    impl OpenCamera {
        pub(super) fn next_frame(&mut self) -> Result<(Vec<u8>, u32, u32), String> {
            Err("sin cámara".into())
        }
    }

    pub(super) fn open(_chosen: Option<&str>) -> Result<OpenCamera, VoiceError> {
        Err(VoiceError::NoCamera)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    /// La elegida se encuentra por su nombre aunque cambie el orden de la
    /// lista; desenchufada, se prueban todas; y se marca una sola vez.
    ///
    /// Roturas que atrapa: elegir por posición (al enchufar otra se abre la
    /// que no es); quedarse sin cámara si la elegida no está; probar sólo el
    /// primer nodo con ese nombre (en Linux, el de metadatos, que no
    /// captura); enseñar la misma webcam dos veces.
    #[test]
    fn the_camera_is_found_by_name_whatever_the_order() {
        let before = names(&["Integrated", "Integrated", "Logitech C920", "Logitech C920"]);
        let after = names(&["Logitech C920", "Logitech C920", "Integrated", "Integrated"]);
        assert_eq!(candidates(&before, Some("Logitech C920")), [2, 3]);
        assert_eq!(candidates(&after, Some("Logitech C920")), [0, 1]);
        assert_eq!(candidates(&after, Some("Desenchufada")), [0, 1, 2, 3]);
        assert_eq!(candidates(&after, None), [0, 1, 2, 3]);

        let shown = as_devices(&after, Some("Integrated"));
        let summary: Vec<(&str, bool)> = shown.iter().map(|d| (d.name.as_str(), d.current)).collect();
        assert_eq!(summary, [("Logitech C920", false), ("Integrated", true)]);
        assert!(as_devices(&after, None)[0].current, "sin elegir, la primera");
    }
}
