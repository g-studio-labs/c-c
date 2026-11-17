#!/usr/bin/env bash
# Lo que GPUI enlaza en Linux, para Ubuntu/Debian.
#
# La lista es la del CI de GPUI Kit (`script/install-linux.sh`), menos WebKit:
# aquí no hay webview. Vive en un script porque la usan dos workflows
# (`desktop.yml` para probar y `desktop-release.yml` para empaquetar): con dos
# copias, la que se queda vieja es la que construye el instalador.

set -euo pipefail

sudo apt-get update
sudo apt-get install -y gcc g++ clang libfontconfig-dev libwayland-dev \
  libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev \
  vulkan-validationlayers libvulkan1
