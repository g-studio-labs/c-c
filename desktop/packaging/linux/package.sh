#!/usr/bin/env bash
# Los paquetes de Linux: un `.deb` y un `.tar.gz` portable.
#
# El `.deb` lo hace cargo-deb (su configuración está en `Cargo.toml`,
# `[package.metadata.deb]`). El `.tar.gz` lleva el mismo árbol —`bin/` y
# `share/`— para quien no usa Debian: se descomprime sobre `~/.local` o
# `/usr/local` y queda con su lanzador y sus iconos.
#
# AppImage no hay, a propósito. Meter en una las bibliotecas de GPUI (Vulkan,
# Wayland) es justo lo que no se debe empaquetar, porque tienen que ser las de
# la gráfica de la máquina; y dentro de una AppImage todo programa del sistema
# que la app lance hereda sus bibliotecas (la trampa de `CLAUDE.md`, «Lo que la
# app lanza del sistema hereda la AppImage»): GPUI abre los enlaces con
# `xdg-open`, que es un script de shell.

source "$(dirname "${BASH_SOURCE[0]}")/../common.sh"

BINARY="$RELEASE_DIR/$APP_BINARY"
require_binary "$BINARY"
mkdir -p "$OUT_DIR"

# --- .deb -------------------------------------------------------------------
# `--no-build`: empaqueta el binario que ya hay, no compila otro.
(cd "$CRATE_DIR" && cargo deb --no-build --locked --output "$OUT_DIR/")

# --- .tar.gz ----------------------------------------------------------------
ARCH="$(uname -m)"
NAME="$APP_BINARY-$APP_VERSION-linux-$ARCH"
ROOT="$STAGE_DIR/$NAME"
rm -rf "$ROOT"
install -Dm755 "$BINARY" "$ROOT/bin/$APP_BINARY"
# Sin símbolos de depuración, como lo deja cargo-deb en el `.deb`: son veinte
# megas que no le sirven a quien instala.
strip --strip-unneeded "$ROOT/bin/$APP_BINARY"
install -Dm644 "$PACKAGING_DIR/linux/$APP_ID.desktop" "$ROOT/share/applications/$APP_ID.desktop"
for size in 32 64 128 256 512; do
  install -Dm644 "$PACKAGING_DIR/icons/${size}x${size}.png" \
    "$ROOT/share/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
done
# Quien no usa el .deb registra cac:// con esto (el postinst del .deb ya lo hace).
install -Dm755 "$PACKAGING_DIR/linux/register-url-scheme.sh" \
  "$ROOT/share/doc/$APP_BINARY/register-url-scheme.sh"
# La licencia de las tipografías que lleva el binario (OFL).
for license in "$CRATE_DIR"/assets/fonts/LICENSE-*.txt; do
  install -Dm644 "$license" "$ROOT/share/doc/$APP_BINARY/licenses/$(basename "$license")"
done
# El binario suelto, sin empaquetar: es lo que baja el actualizador de la app
# (`mcp/src/desktop_update.rs`) para ponerlo donde está el que corre. Va con el
# nombre que ese módulo espera; `mcp/tests/desktop_release.rs` lo comprueba.
install -Dm755 "$ROOT/bin/$APP_BINARY" "$OUT_DIR/$NAME"
# Sin dueño ni fecha de esta máquina: el mismo binario da el mismo archivo.
tar --owner=0 --group=0 --numeric-owner -C "$STAGE_DIR" -czf "$OUT_DIR/$NAME.tar.gz" "$NAME"

ls -l "$OUT_DIR"
