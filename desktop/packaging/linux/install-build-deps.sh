#!/usr/bin/env bash
# Lo que GPUI enlaza en Linux, para Ubuntu/Debian.
#
# La lista es la del CI de GPUI Kit (`script/install-linux.sh`), menos WebKit:
# aquí no hay webview. Vive en un script porque la usan dos workflows
# (`desktop.yml` para probar y `desktop-release.yml` para empaquetar): con dos
# copias, la que se queda vieja es la que construye el instalador.
#
# `libglib2.0-dev` es del motor de voz (`voice/`): libwebrtc usa GLib para
# capturar la pantalla, y sin GTK nadie más la trae (ver `voice/build.rs`).

set -euo pipefail

sudo apt-get update
sudo apt-get install -y gcc g++ clang libfontconfig-dev libwayland-dev \
  libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev \
  vulkan-validationlayers libvulkan1 libglib2.0-dev lsb-release wget gnupg

# El clang del sistema no basta para el motor de voz. libwebrtc trae su propio
# libc++, que sigue a LLVM trunk, y `webrtc-sys` se niega a compilar con un
# clang por debajo del que ese libc++ pide (21 con libwebrtc 0.4.2; lo lee del
# artefacto). Ubuntu 24.04 trae el 18. Se instala de apt.llvm.org y se le dice a
# `cc` que lo use: con GCC compilaría y fallaría al ejecutar, porque ignora las
# anotaciones `trivial_abi` de ese libc++ (ver `webrtc-sys`, build.rs).
CLANG_MAJOR=21
wget -qO /tmp/llvm.sh https://apt.llvm.org/llvm.sh
sudo bash /tmp/llvm.sh "$CLANG_MAJOR"
if [ -n "${GITHUB_ENV:-}" ]; then
  echo "CC=clang-$CLANG_MAJOR" >> "$GITHUB_ENV"
  echo "CXX=clang++-$CLANG_MAJOR" >> "$GITHUB_ENV"
fi
