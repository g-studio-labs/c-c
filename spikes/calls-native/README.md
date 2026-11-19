# Prueba de viabilidad: llamadas en la app nativa (GPUI + LiveKit)

Tarea: `command-and-control-419` (pasos 420–425). Plan aprobado por jose el
19-nov-2026: servidor de **producción**, alcance **todo** (voz, ver vídeo,
cámara, pantalla), **LiveKit 0.12**.

**Estado: pasos 0 y 1 hechos; 2–4 escritos y compilando, sin probar (necesitan
a jose al otro lado); 5, este informe, a medias.**

## Veredicto provisional

Pintar vídeo en vivo en GPUI en Linux **es viable** por el camino que usa Zed
fuera de macOS: cada fotograma se convierte con libyuv a BGRA en la tarea de su
pista, se sube al atlas de la gráfica con `img(ImageSource::Render(..))`, y el
anterior se tira con `window.drop_image`. Con eso la memoria queda plana durante
10 minutos con 4 vídeos, y la CPU es baja (0,6 núcleos con 4 × 720p30). Falta
medir los fotogramas por segundo en una pantalla de verdad: la virtual no
presenta más de 21 por segundo ni con la ventana vacía.

## Cómo se ejecuta

Todo con `taskset -c 0-3 env CARGO_BUILD_JOBS=3` en esta máquina.

```sh
cargo build --release
./target/release/calls-native-spike            # paso 0: la sonda
./target/release/calls-native-spike paint      # paso 1: vídeo de prueba (ver paint.rs)
./target/release/calls-native-spike call       # pasos 2–4 (ver más abajo)
```

Sin tocar el escritorio de nadie, en una pantalla virtual con la gráfica:

```sh
Xvfb :99 -screen 0 1400x900x24 -nolisten tcp &
env -u WAYLAND_DISPLAY DISPLAY=:99 \
  VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/nvidia_icd.json \
  CALLS_TRACKS=4 CALLS_SECONDS=60 ./target/release/calls-native-spike paint
```

Variables de `paint`: `CALLS_TRACKS`, `CALLS_W`, `CALLS_H`, `CALLS_FPS`,
`CALLS_SECONDS`, `CALLS_CSV`, `CALLS_LEAK=1` (no tirar el anterior: la fuga),
`CALLS_SPIN=1` (pintar sin parar: el tope de la pantalla).

## Paso 0 · Compila y enlaza

| | |
|---|---|
| Versiones | `livekit` 0.12.0, `libwebrtc` 0.4.2, `webrtc-sys` 0.3.55, con `gpui-kit` =0.9.1 |
| Debug en frío (libwebrtc descargada) | 228 s; binario 809 MB |
| Release en frío | 8 min 10 s; **54,2 MB**, **39,4 MB sin símbolos** (el `cac-desktop` actual: 79,4 MB con símbolos) |
| Compilador | gcc del sistema bastó; el README del SDK pide clang ≥ 21 (hay 23 instalado) |
| Enlazado | sólo `gio-2.0`, `glib-2.0`, `gobject-2.0` a mano (`build.rs`), porque la app nativa no lleva GTK |

Para el CI de `desktop/` haría falta `libglib2.0-dev` en
`desktop/packaging/linux/install-build-deps.sh` y red al compilar (libwebrtc se
descarga de GitHub; `LK_CUSTOM_WEBRTC` lo evita).

Cambio de API notado: `to_argb`/`to_i420` viven en el rasgo
`video_frame::native::VideoFrameBufferExt` para tipos concretos. Y 0.12 expone
`get_dmabuf_fd()` en Linux, la puerta a un camino sin copias.

## Paso 1 · Pintar vídeo sin servidor

Xvfb + Vulkan de la NVIDIA RTX 3060. Treinta fotogramas I420 prefabricados que
se repiten: sólo cuenta convertir, subir y pintar.

| caso | convertir | CPU (núcleos) | memoria residente | memoria de la gráfica |
|---|---|---|---|---|
| 720p30 × 1 | 0,67 ms | 0,43 | 280 MB, plana | 757 → 847 MiB |
| 720p30 × 4 | 0,71 ms | 0,57 | 444 → 451 | → 1.016 |
| 720p30 × 9 | 0,73 ms | 0,80 | 702 → 723 | → 1.120 |
| 1080p30 × 1 | 1,44 ms | 0,48 | 355, plana | → 968 |
| 1080p30 × 4 | 3,16 ms | 1,03 | 694 → 710 | → 1.112 |
| 1080p60 × 1 | 1,57 ms | 0,54 | 356, plana | → 968 |
| **sin `drop_image`**, 720p30 × 1, 30 s | — | 0,47 | **565 → 2.485 MB** | **llena (4.096)** |
| 720p30 × 4, **10 minutos** | 0,75 ms | 0,58 | 448 → 453 | → 1.016, plana |

- **Colores**: la franja roja sale roja y la azul azul con
  `VideoFormatType::ARGB` (libyuv deja B, G, R, A en memoria, que es lo que
  pinta GPUI). Comprobado en captura.
- **Tope de la pantalla virtual**: 21 presentaciones por segundo también con la
  ventana vacía pidiendo pintar sin parar, con lavapipe, Intel o NVIDIA. No es
  GPUI ni el vídeo. **Falta medir las fps en la pantalla real.**
- Con lavapipe (CPU) la misma prueba gasta 3,1 núcleos: sirve para ver fugas,
  no costes.
- Coste por fotograma en GPUI: cada uno crea y destruye una textura y copia
  ~3,6 MB (720p) dos veces. Aguanta lo medido; el camino sin copias (dma-buf)
  pediría parchear GPUI y queda fuera.

## Pasos 2–4 · La llamada (`call.rs`, sin probar)

Entra a una **reunión de producción como invitado** (`guest:<id>`): así no hace
falta otra cuenta ni la sesión de nadie, y no choca con la identidad de quien
está en la app de Tauri (LiveKit echaría a uno de los dos si fuera la misma).

1. Alguien en la app de Tauri crea una reunión y copia su enlace de invitado.
2. `CALLS_INVITE='<enlace>' ./target/release/calls-native-spike call`
   (opcional `CALLS_CAMERA=1`, `CALLS_SCREEN=1`, `CALLS_SCREEN_SOURCE=<n>`,
   `CALLS_MUTE=1`, `CALLS_CSV=…`, `CALLS_NAME=…`).
3. La prueba pide entrar (`/api/v1/public/calls/join`) y espera en la sala de
   espera preguntando cada 3 s; quien está dentro la admite desde Tauri.
4. Ya dentro: publica el micrófono (eco, ruido y paso alto con el procesador de
   libwebrtc), reproduce lo que llega por una sola salida que alimenta al
   cancelador de eco, pinta cada vídeo que llega por el camino del paso 1 y, si
   se pide, publica la cámara y la pantalla. Una línea por segundo con lo que
   llega, lo que se pinta, el audio, la CPU y la memoria.

Se portó del motor de Tauri (`app/src-tauri/src/voice.rs`): procesador, mezcla
de salida, captura y remuestreo del micrófono, recepción de pistas, cámara con
`nokhwa` y pantalla con el `DesktopCapturer` de libwebrtc. Cambios respecto al
motor: si la cámara cambia de resolución **se vuelve a publicar la pista** (en
Tauri sólo se cambiaba la fuente local y los demás probablemente ven la imagen
congelada); el vídeo que llega se convierte entero (cola sin tope) para contar
de verdad lo que llega.

Sin probar y fuera del alcance de esta máquina: compartir pantalla en Wayland
(portal), enchufar y desenchufar dispositivos a media llamada, macOS y Windows.

## Lo que falta

- Medir fps en la pantalla de verdad (paso 1, con jose delante).
- Pasos 2–4 con jose al otro lado: que se oye y se le oye, que el eco se cancela
  con altavoces, que se ve y se le ve, y las medidas con vídeo real.
- Paso 5: costes por fase y recomendación final.
