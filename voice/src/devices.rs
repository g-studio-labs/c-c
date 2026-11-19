//! Micrófonos y altavoces: listarlos, y encontrar el elegido.
//!
//! Portado de `voice_list_devices` (`app/src-tauri/src/voice.rs`), que sólo
//! listaba micrófonos; aquí también salidas, que elegir altavoz es nuevo.
//! Se identifican por el `DeviceId` de cpal, que la biblioteca documenta como
//! estable entre desconexiones y reinicios.

use std::collections::HashSet;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    Input,
    Output,
}

/// Un dispositivo, como lo enseña la pantalla.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Device {
    /// El `DeviceId` de cpal como texto; es lo que se pasa a `set_device`.
    pub id: String,
    pub name: String,
    /// El que se usa: el elegido si sigue enchufado, y si no el del sistema.
    pub current: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Devices {
    pub inputs: Vec<Device>,
    pub outputs: Vec<Device>,
}

/// Lo que hay ahora mismo. Se pregunta al abrir el desplegable y no al entrar:
/// enchufar unos auriculares a media llamada es justo cuando alguien lo abre.
pub(crate) fn list(chosen_input: Option<&str>, chosen_output: Option<&str>) -> Devices {
    let host = cpal::default_host();
    Devices { inputs: list_kind(&host, DeviceKind::Input, chosen_input), outputs: list_kind(&host, DeviceKind::Output, chosen_output) }
}

fn list_kind(host: &cpal::Host, kind: DeviceKind, chosen: Option<&str>) -> Vec<Device> {
    let (all, default) = match kind {
        DeviceKind::Input => (host.input_devices().map(|i| i.collect::<Vec<_>>()), host.default_input_device()),
        DeviceKind::Output => (host.output_devices().map(|i| i.collect::<Vec<_>>()), host.default_output_device()),
    };
    let all = match all {
        Ok(all) => all,
        Err(e) => {
            log::warn!("dispositivos: no se pudieron listar los de {kind:?}: {e}");
            return Vec::new();
        }
    };
    // En cpal 0.18 el nombre legible es el `Display` del dispositivo.
    let entries: Vec<(String, String)> = all.iter().filter_map(|d| Some((id_of(d)?, d.to_string()))).collect();
    let default_id = default.as_ref().and_then(id_of);
    let current = pick_current(entries.iter().map(|(id, _)| id.as_str()), chosen, default_id.as_deref());
    let marked = entries.into_iter().map(|(id, name)| Device { current: Some(id.as_str()) == current.as_deref(), id, name }).collect();
    clean(marked)
}

/// El dispositivo que se abrirá: el elegido si sigue existiendo; si se
/// desenchufó, el del sistema. Caer al del sistema es mejor que no sonar.
pub(crate) fn find(host: &cpal::Host, kind: DeviceKind, chosen: Option<&str>) -> Option<cpal::Device> {
    let found = chosen.and_then(|id| {
        let mut all: Box<dyn Iterator<Item = cpal::Device>> = match kind {
            DeviceKind::Input => Box::new(host.input_devices().ok()?),
            DeviceKind::Output => Box::new(host.output_devices().ok()?),
        };
        all.find(|d| id_of(d).as_deref() == Some(id))
    });
    found.or_else(|| match kind {
        DeviceKind::Input => host.default_input_device(),
        DeviceKind::Output => host.default_output_device(),
    })
}

fn id_of(d: &cpal::Device) -> Option<String> {
    d.id().ok().map(|i| i.to_string())
}

/// Cuál se marca como puesto: el elegido si está en la lista, y si no el del
/// sistema, que es el que `find` abrirá. Marcar el elegido desenchufado no
/// marcaba nada (el de Tauri lo hacía así).
fn pick_current<'a>(ids: impl Iterator<Item = &'a str>, chosen: Option<&str>, default: Option<&str>) -> Option<String> {
    let ids: Vec<&str> = ids.collect();
    chosen.filter(|c| ids.contains(c)).or(default).map(str::to_string)
}

/// Quita lo que no es un dispositivo para una persona, y los repetidos.
///
/// En Linux cpal enumera la configuración de ALSA entera: plugins de
/// conversión, mezcla a 4/6/8 canales, puentes a JACK y OSS. Lista de
/// permitidos por el nombre PCM, y **nunca hasta dejarla vacía**: si no
/// sobrevive nada se devuelve entera. Una lista fea es mejor que ninguna.
fn clean(everyone: Vec<Device>) -> Vec<Device> {
    let usable: Vec<Device> = everyone.iter().filter(|d| is_usable(&d.id)).cloned().collect();
    dedup(if usable.is_empty() { everyone } else { usable })
}

/// El identificador viene como `<host>:<pcm>`; sólo el `pcm` dice qué es.
fn is_usable(id: &str) -> bool {
    let Some((host, pcm)) = id.split_once(':') else { return true };
    if host != "alsa" {
        return true;
    }
    pcm == "default" || ["hw:", "plughw:", "sysdefault:", "front:"].iter().any(|p| pcm.starts_with(p))
}

/// Un nombre, una entrada: el mismo códec sale varias veces con el mismo
/// nombre. Se queda el puesto si es uno de ellos (si no, quitarlo dejaba la
/// lista sin ninguno marcado), y si no el primero.
fn dedup(list: Vec<Device>) -> Vec<Device> {
    let current_names: HashSet<String> = list.iter().filter(|d| d.current).map(|d| d.name.clone()).collect();
    let mut seen = HashSet::new();
    list.into_iter().filter(|d| (d.current || !current_names.contains(&d.name)) && seen.insert(d.name.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, current: bool) -> Device {
        Device { id: id.into(), name: name.into(), current }
    }

    #[test]
    fn alsa_plugins_are_left_out_but_never_everything() {
        let list = vec![device("alsa:default", "Default", false), device("alsa:jack", "JACK", false), device("alsa:hw:CARD=PCH,DEV=0", "HDA Intel", true)];
        let names: Vec<String> = clean(list).into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Default", "HDA Intel"]);
        let only_plugins = vec![device("alsa:jack", "JACK", false), device("alsa:oss", "OSS", false)];
        assert_eq!(clean(only_plugins).len(), 2, "si no sobrevive nada, la lista entera");
    }

    /// El puesto sobrevive a quitar repetidos aunque no sea el primero.
    #[test]
    fn the_current_one_survives_dedup() {
        let list = vec![device("alsa:hw:CARD=0,DEV=0", "sof-hda-dsp", false), device("alsa:hw:CARD=0,DEV=1", "sof-hda-dsp", true), device("alsa:hw:CARD=1,DEV=0", "USB", false)];
        let kept = dedup(list);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|d| d.current && d.id == "alsa:hw:CARD=0,DEV=1"));
    }

    /// Elegido y enchufado, ése; desenchufado, el del sistema.
    #[test]
    fn current_is_the_chosen_while_it_exists() {
        let ids = ["a", "b"];
        assert_eq!(pick_current(ids.iter().copied(), Some("b"), Some("a")).as_deref(), Some("b"));
        assert_eq!(pick_current(ids.iter().copied(), Some("gone"), Some("a")).as_deref(), Some("a"));
        assert_eq!(pick_current(ids.iter().copied(), None, None), None);
    }
}
