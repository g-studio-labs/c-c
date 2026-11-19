# Prueba de viabilidad: llamadas en la app nativa (GPUI + LiveKit)

Tarea: `command-and-control-419` (pasos 420–425). Plan aprobado por jose el
19-nov-2026: servidor de **producción**, alcance **todo** (voz, ver vídeo,
cámara, pantalla), **LiveKit 0.12**.

**Estado: hecha (19-nov-2026).** Pasos 0–4 medidos, los 2–4 contra producción
con jose al otro lado en la app de Tauri; este informe es el paso 5.

## Veredicto

**Las llamadas son viables en la app nativa.** Voz de ida y vuelta, ver la
cámara y la pantalla de otro, y publicar cámara y pantalla funcionan con
LiveKit 0.12 dentro de GPUI, en Linux X11, contra el servidor de producción.

La incógnita, pintar vídeo en vivo, se resuelve por el camino que usa Zed fuera
de macOS: cada fotograma se convierte con libyuv a BGRA en la tarea de su pista,
se sube al atlas de la gráfica con `img(ImageSource::Render(..))` y el anterior
se tira con `window.drop_image`. En la pantalla real, **9 vídeos 720p a 30 fps
se pintan enteros con 0,74 núcleos**, y la memoria queda plana durante 10
minutos. El techo conocido: un vídeo **1080p a 60 fps se queda en 42 fps**.

Lo que queda sin cerrar está en «Riesgos»: la calidad de la voz con dos
máquinas, compartir pantalla en Wayland, macOS y Windows.

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

## Paso 1 bis · En la pantalla real

X11, monitores a 180 y 144 Hz, NVIDIA RTX 3060.

| caso | fps de cada vídeo | perdidos/s | CPU (núcleos) |
|---|---|---|---|
| ventana vacía pintando sin parar | 177 repintados/s | — | 0,15 |
| 4 × 720p30 | 30,0 | 0 | 0,53 |
| 9 × 720p30 | 29,8 | 0,2 | 0,74 |
| 1 × 1080p60 | **41,8** | 18 | 0,29 |

El techo de 1080p60 es subir ~8 MB por fotograma al atlas creando una textura
cada vez. Cámaras a 720p y pantallas a 1080p hasta ~30 fps entran de sobra. El
camino sin copias (dma-buf, que 0.12 ya expone con `get_dmabuf_fd`) lo subiría,
pero pide parchear GPUI.

## Pasos 2–4 · La llamada (`call.rs`)

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

Para medir el audio: `CALLS_NO_APM=1` publica sin procesado y
`CALLS_DUMP_DIR=<dir>` guarda lo que capta el micrófono y lo que se manda
(`capta.s16`, `manda.s16`: PCM de 16 bits, 48 kHz, mono).

### Lo medido el 19-nov con jose (reunión «Prueba con claude», producción)

| qué | resultado |
|---|---|
| Entrar como invitado | pide entrar, espera, jose admite, entra. Cuatro veces seguidas sin fallo; sale limpio con Ctrl+C |
| Voz | se oye en los dos sentidos; 100 tramas/s publicadas, 0 huecos de reproducción, **0,07 núcleos** |
| Ver la cámara de jose | 10–15 fps (lo que manda Tauri), todas pintadas, convertir 0,4–1,2 ms |
| Ver su pantalla | 20–21 fps, todas pintadas, 0 perdidas, convertir 2–4 ms |
| Ver las dos a la vez | **0,35–0,41 núcleos**, ~370 MB residentes |
| Publicar cámara (1280×720 MJPEG) y pantalla (HDMI-0, 1080×1920) | jose las ve en Tauri; cámara ~10 fps (la webcam con esa luz); **1,0–1,3 núcleos** publicando las dos |

**La voz en una sola máquina no sirve para juzgar la calidad.** Con la prueba y
Tauri en el mismo equipo, compartiendo micrófono y auriculares, la voz de jose
le vuelve:
- **con el cancelador de eco** suena robótica: recortó más de 20 dB el 18 % de
  las tramas con voz, porque lo que reproduce (la voz de jose que llega de
  Tauri) es casi lo que capta el micrófono (su voz en vivo) y lo toma por eco;
- **sin él** suena limpia pero con eco, su voz de vuelta.

Ninguna de las dos cosas pasa entre dos máquinas; eso queda por comprobar.

## Riesgos

- **Calidad de voz con dos personas en dos máquinas: sin comprobar.** Es la
  primera prueba de la fase de voz.
- **Ruidos sueltos.** El supresor de WebRTC quita ruido constante (ventilador),
  no respiraciones ni golpes. Si molesta: RNNoise (`nnnoiseless`, Rust puro) o
  DeepFilterNet entre el micrófono y el envío.
- **Compartir pantalla en Wayland: sin probar** (esta máquina es X11). Va por el
  portal de xdg-desktop y quizá pida la feature `glib-main-loop`.
- **macOS y Windows: sin probar.** En macOS hará falta el permiso del micrófono y
  la cámara; en Windows, `+crt-static` como en Tauri.
- **Publicar cuesta ~1 núcleo** con cámara y pantalla: decodificar MJPEG,
  convertir color con un bucle propio en coma flotante (portado de Tauri) y
  comprimir dos vídeos. La conversión se puede pasar a libyuv.
- **1080p60 se queda en 42 fps** por el camino actual de GPUI.
- **Compilar**: +8 min en frío, libwebrtc se descarga de GitHub al compilar, el
  CI necesita `libglib2.0-dev`, y el binario crece ~40 MB.

## Qué se reaprovecha

Casi todo el motor de Tauri (`app/src-tauri/src/voice.rs`): procesador de audio,
mezcla de salida, captura y remuestreo del micrófono, recepción de pistas,
cámara y pantalla. De Tauri sólo dependía del canal de eventos y del runtime.
Los cambios de LiveKit 0.8.3 a 0.12 fueron pocos (ver «Paso 0» y los
comentarios de `call.rs`). Lo de `video_frames.rs` (JPEG por `cacvideo://`) ya no
hace falta: en nativo se pinta directo. El backend no cambia: pases, timbre,
presencia y salas ya existen.

## Coste estimado por fase

| fase | qué | días |
|---|---|---|
| 1 · Voz | el motor como módulo sin interfaz (del spike y `voice.rs`, con sus pruebas), entrar a la voz de un canal, silenciar y ensordecer, quién está y quién habla, elegir dispositivos, timbre; **probada con dos máquinas** | 6–8 |
| 2 · Ver vídeo | recuadros, rejilla y foco, la vista previa propia en espejo, dejar de pintar lo que no se ve | 3 |
| 3 · Cámara | publicarla, elegir cámara, volver a publicar si cambia la resolución (en Tauri los demás veían la imagen congelada), conversión con libyuv | 2–3 |
| 4 · Pantalla | selector en X11, el portal en Wayland, permisos en macOS y Windows | 3–5 |
| 5 · Empaquetar | CI con `libglib2.0-dev` y red, comprobar macOS y Windows, tamaño | 2 |

Total: **16–21 días** para tener llamadas como en Tauri.

## Recomendación

Seguir, por fases y empezando por la voz: es la que más se usa, la que más se
reaprovecha, y la única con una duda de calidad que sólo se cierra con dos
máquinas. El vídeo ya no es una incógnita. El motor debería vivir en un módulo
propio sin interfaz, como el core, para que la app nativa sólo pinte.
