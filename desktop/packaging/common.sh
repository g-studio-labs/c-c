#!/usr/bin/env bash
# Lo que comparten los empaquetadores: quién es la app y qué versión es.
#
# La identidad sale de `identity.env` y la versión de `Cargo.toml`: ninguna de
# las dos se escribe en otro sitio. Se usa con `source` desde cada
# `package.sh`; con `--print` enseña lo que leyó, que es lo que comprueba la
# prueba `identity::tests::the_packagers_read_what_the_binary_carries`.

set -euo pipefail

PACKAGING_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(dirname "$PACKAGING_DIR")"

# shellcheck source=identity.env
source "$PACKAGING_DIR/identity.env"

# La versión del paquete: la primera `version = "…"` de la sección `[package]`.
# Acotada a esa sección a propósito; una dependencia escrita en varias líneas
# también tiene su `version = `.
APP_VERSION="$(awk '
  /^\[/ { in_package = ($0 == "[package]") }
  in_package && /^version[[:space:]]*=/ { gsub(/^[^"]*"|"[^"]*$/, ""); print; exit }
' "$CRATE_DIR/Cargo.toml")"

for name in APP_NAME APP_ID APP_BINARY APP_VERSION; do
  if [ -z "${!name:-}" ]; then
    echo "packaging: falta $name" >&2
    exit 1
  fi
done

# Dónde está el binario de release y dónde se dejan los instaladores.
RELEASE_DIR="$CRATE_DIR/target/release"
STAGE_DIR="$CRATE_DIR/target/packaging/stage"
OUT_DIR="$CRATE_DIR/target/packaging/out"

# El binario tiene que estar ya compilado: empaquetar no compila, para que lo
# que se empaqueta sea exactamente lo que se acaba de construir y probar.
require_binary() {
  if [ ! -f "$1" ]; then
    echo "packaging: no está $1; primero \`cargo build --release --locked\`" >&2
    exit 1
  fi
}

if [ "${1:-}" = "--print" ]; then
  printf 'APP_NAME=%s\nAPP_ID=%s\nAPP_BINARY=%s\nAPP_VERSION=%s\n' "$APP_NAME" "$APP_ID" "$APP_BINARY" "$APP_VERSION"
fi
