//! Las imágenes que una pantalla baja para enseñarlas en pequeño: las de una
//! tarea y las de los mensajes de una conversación.
//!
//! Vivía dentro de `task.rs`, que era quien las enseñaba; la conversación
//! las pide igual, así que está aquí para las dos. Qué es una imagen, de
//! dónde se baja y con qué sesión lo decide el core: aquí sólo se lleva la
//! cuenta de cuáles se pidieron ya y se decodifican fuera del hilo que pinta.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use cac_core::{ApiError, Core, Picture};
use gpui_kit::{AppContext, Context, Image, ImageFormat, RenderImage, SvgRenderer, Window};

use crate::backend::Backend;

/// Una imagen, en lo que tarda en poder enseñarse.
pub(crate) enum Thumb {
    Loading,
    Ready(Arc<RenderImage>),
    /// No llegó, o no es algo que se pueda pintar: se queda con su nombre,
    /// que sigue abriéndola.
    Failed,
}

/// La imagen ya decodificada, o `None` si esta app no sabe pintarla (un
/// AVIF) o el fichero está roto. Qué imagen es lo dijo el core.
fn decoded(picture: Picture, svg: SvgRenderer) -> Option<Arc<RenderImage>> {
    let format = ImageFormat::from_mime_type(&picture.mime)?;
    Image::from_bytes(format, picture.bytes).to_image_data(svg).ok()
}

/// Las imágenes de una pantalla, por su clave (el id de un adjunto, o su
/// ruta). Cada una se baja una vez.
#[derive(Default)]
pub(crate) struct Thumbs {
    held: HashMap<String, Thumb>,
}

impl Thumbs {
    pub(crate) fn get(&self, key: &str) -> Option<&Thumb> {
        self.held.get(key)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.held.clear();
    }

    /// Tira las que ya no son de lo que se mira.
    pub(crate) fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.held.retain(|key, _| keep(key));
    }

    /// Pide esa imagen si aún no se ha pedido. `fetch` la trae por el core;
    /// `at` dice dónde guarda la vista estas imágenes, para poner ahí la
    /// respuesta; y `landed` se llama cuando ya está (o ya se sabe que no):
    /// quien la enseña puede necesitar volver a medir su hueco.
    pub(crate) fn request<V, F, Fut>(
        &mut self,
        key: String,
        fetch: F,
        at: fn(&mut V) -> &mut Thumbs,
        landed: fn(&mut V, &str, &mut Context<V>),
        window: &mut Window,
        cx: &mut Context<V>,
    ) where
        V: 'static,
        F: FnOnce(Core) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Picture, ApiError>> + Send + 'static,
    {
        if self.held.contains_key(&key) {
            return;
        }
        // Apuntada ya como pedida: la siguiente pintada no la pide otra
        // vez, ni tampoco si falla.
        self.held.insert(key.clone(), Thumb::Loading);
        let answer = Backend::ask(cx, fetch);
        let svg = cx.svg_renderer();
        cx.spawn_in(window, async move |this, cx| {
            let image = match answer.await {
                // Decodificar una captura grande tarda: fuera del hilo
                // que pinta.
                Ok(Ok(picture)) => cx.background_spawn(async move { decoded(picture, svg) }).await,
                _ => None,
            };
            this.update(cx, |this, cx| {
                // Si ya no está es que se dejó de mirar: no se pone.
                if let Some(thumb) = at(this).held.get_mut(&key) {
                    *thumb = image.map_or(Thumb::Failed, Thumb::Ready);
                    landed(this, &key, cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}
