//! Lo que hay que saber antes de compilar: quién es la app, y en Windows su
//! icono.
//!
//! La identidad se escribe una vez, en `packaging/identity.env`, y de ahí la
//! leen también los empaquetadores. Aquí se le pasa al binario como variables
//! de compilación (`src/identity.rs` las lee con `env!`).

use std::path::Path;

const IDENTITY_FILE: &str = "packaging/identity.env";
#[cfg(windows)]
const WINDOWS_ICON: &str = "packaging/icons/icon.ico";

/// El valor de `KEY="valor"`. Sin él no se compila: una app sin nombre o sin
/// identificador se instalaría encima de cualquier otra cosa.
fn value(identity: &str, key: &str) -> String {
    identity
        .lines()
        .filter_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
        .map(|value| value.trim().trim_matches('"').to_string())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("{IDENTITY_FILE} no dice {key}"))
}

fn main() {
    // Sin esto, tener un `build.rs` haría recompilar con cualquier cambio del
    // paquete.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={IDENTITY_FILE}");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo dice dónde está el paquete");
    let identity = std::fs::read_to_string(Path::new(&manifest_dir).join(IDENTITY_FILE))
        .unwrap_or_else(|error| panic!("no se pudo leer {IDENTITY_FILE}: {error}"));
    let name = value(&identity, "APP_NAME");
    let id = value(&identity, "APP_ID");
    let binary = value(&identity, "APP_BINARY");
    println!("cargo:rustc-env=CAC_APP_NAME={name}");
    println!("cargo:rustc-env=CAC_APP_ID={id}");
    println!("cargo:rustc-env=CAC_APP_BINARY={binary}");

    // El icono del `.exe`, y lo que enseña «Propiedades». Va con el
    // identificador 1 (el que pone `set_icon`) porque es el que GPUI le pide a
    // Windows para el icono de la ventana: con otro número, la ventana saldría
    // con el icono genérico aunque el fichero tuviera el bueno.
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed={WINDOWS_ICON}");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(WINDOWS_ICON);
        resource.set("ProductName", &name);
        resource.set("FileDescription", &name);
        resource.set("InternalName", &binary);
        resource.compile().expect("no se pudo incrustar el icono en el .exe");
    }
}
