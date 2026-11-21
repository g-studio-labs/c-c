# La caja donde se compila y se prueba la app nativa (`desktop/`, `core/`,
# `voice/`) en gstd, para no hacerlo en la máquina de quien desarrolla.
# Ver «Diseño · Compilar la app nativa en gstd» y command-and-control-464.
#
# Sin secretos: no hace falta ninguno para compilar ni para probar.
FROM ubuntu:24.04

ARG CLANG_MAJOR=21
ENV DEBIAN_FRONTEND=noninteractive

# Lo que GPUI y el motor de voz enlazan en Linux. Es la lista de
# `desktop/packaging/linux/install-build-deps.sh`, que vive con la app en su
# rama; si aquélla cambia, ésta también. El resto es para las pasadas a ojo
# (Xvfb con lavapipe, xdotool, maim) y para `/mutar` (python3).
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl wget gnupg lsb-release software-properties-common \
      git rsync tar xz-utils zstd python3 pkg-config make \
      gcc g++ clang libfontconfig-dev libwayland-dev libxkbcommon-x11-dev \
      libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev \
      vulkan-validationlayers libvulkan1 libglib2.0-dev \
      xvfb xdotool maim mesa-vulkan-drivers fonts-dejavu-core fonts-noto-color-emoji \
      mold \
    && rm -rf /var/lib/apt/lists/*

# libwebrtc trae su propio libc++ y `webrtc-sys` se niega a compilar con un
# clang por debajo del que ese libc++ pide. Ubuntu 24.04 trae el 18.
RUN wget -qO /tmp/llvm.sh https://apt.llvm.org/llvm.sh \
    && bash /tmp/llvm.sh "$CLANG_MAJOR" \
    && rm -rf /tmp/llvm.sh /var/lib/apt/lists/*
ENV CC=clang-${CLANG_MAJOR} CXX=clang++-${CLANG_MAJOR}

# La cadena de Rust va en la imagen; `CARGO_HOME` y `target/` van en el
# volumen (los pone el StatefulSet), que es lo que tiene que durar.
ENV RUSTUP_HOME=/opt/rustup PATH=/opt/cargo-bin/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | CARGO_HOME=/opt/cargo-bin sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path \
    && chmod -R a+rX /opt/rustup /opt/cargo-bin

# Ubuntu 24.04 ya trae el usuario `ubuntu` con uid 1000.
USER 1000:1000
WORKDIR /work/src
CMD ["sleep", "infinity"]
