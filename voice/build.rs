//! Enlaza GLib, GIO y GObject en Linux.
//!
//! El capturador de escritorio de libwebrtc habla con xdg-desktop-portal por
//! D-Bus, así que arrastra GIO, y el `.a` precompilado trae los objetos pero no
//! las dependencias del sistema. La app de Tauri no lo nota porque GTK ya las
//! enlaza; la nativa no lleva GTK. Lo mismo que `spikes/voice-native/build.rs`.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        for lib in ["gio-2.0", "glib-2.0", "gobject-2.0"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
}
