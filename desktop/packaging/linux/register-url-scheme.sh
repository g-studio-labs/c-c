#!/bin/sh
# Registra cac:// para la persona que lo corre, para el .tar.gz (el .deb lo hace
# su postinst). Se corre una vez después de descomprimir sobre ~/.local:
#
#   sh ~/.local/share/doc/cac-desktop/register-url-scheme.sh
#
# El lanzador llama a `cac-desktop` por nombre, así que ~/.local/bin tiene que
# estar en el PATH del escritorio. Con otro prefijo: PREFIX=/usr/local sh …
set -eu

PREFIX="${PREFIX:-$HOME/.local}"
DESKTOP_ID="dev.guz-studio.cac.native.desktop"
APPS="$PREFIX/share/applications"

[ -f "$APPS/$DESKTOP_ID" ] || { echo "no encuentro $APPS/$DESKTOP_ID" >&2; exit 1; }
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS"
xdg-mime default "$DESKTOP_ID" x-scheme-handler/cac
echo "cac:// abre ahora CAC Nativa"
