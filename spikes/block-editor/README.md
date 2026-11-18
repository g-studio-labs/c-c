# Editor por bloques sobre GPUI: prueba de viabilidad

Tarea `command-and-control-391`, 18-nov-2026. La pregunta: ¿se puede escribir
la documentación de cac «como en Notion» —editando el contenido ya pintado,
bloque a bloque— en la app nativa (`desktop/`, gpui-kit 0.9.1 sobre gpui-pre
0.3.12), sin dejar de guardar markdown?

## Veredicto: viable, con límites

Se puede, construyéndolo nosotros sobre lo que hay en esta carpeta. No hay nada
publicado que se pueda adoptar tal cual para documentos largos. Lo difícil no
es pintar ni escribir (eso sale en un día y va rápido): es todo lo que rodea a
un editor de verdad —selección a través de bloques, deshacer, tablas, pegar
desde el navegador—, que hay que hacerlo a mano y suma semanas.

Lo que sostiene el veredicto:

- **La fidelidad se resuelve por diseño, no por cuidado.** El modelo
  (`src/doc.rs`) no tiene un «escritor de markdown»: guarda los trozos de
  texto del fichero y guardar es pegarlos. Lo que no se toca vuelve a salir
  byte a byte, y lo que no se sabe editar (tablas, HTML, citas, listas
  anidadas…) se queda como bloque opaco, que además se puede retocar como
  texto. Es justo lo que falló en el componente rechazado el 17-nov.
- **Va sobrado de velocidad** con la lista virtual de GPUI: 3 ms por
  fotograma al escribir en un documento de 3.000 bloques.
- **Los acentos entran por el camino correcto**: el mismo receptor de texto
  (`EntityInputHandler`) que usan los campos de GPUI Kit. Probado con la
  secuencia que manda X11 para una tecla muerta; **no probado con un teclado
  físico** (la prueba no puede pulsar teclas en tu pantalla).

## Qué hay publicado (mirado el 18-nov-2026)

Compilado aquí, contra las versiones exactas de `desktop/`:

| Qué | Versión | Resultado |
|---|---|---|
| `zorite-editor` (MIT) | 0.11.3, 29-oct-2026 | **Compila y funciona** con gpui-kit 0.9.1 / gpui-pre 0.3.12 (`probes/zorite/`). Es un editor de markdown con vista previa en vivo sobre el texto fuente, estilo Obsidian: títulos grandes, marcas escondidas salvo junto al cursor, tablas pintadas, deshacer, selección, IME. No es por bloques (ni arrastrar, ni menú «/»). Pasa las mismas pruebas de fidelidad y de tecla muerta que el prototipo. **No aguanta documentos largos**: no tiene lista virtual (ver medidas). |

Leído (manifiesto en crates.io y código), no compilado:

| Qué | Versión | Por qué no |
|---|---|---|
| `gpui-kit-markdown-editor` | 0.3.1, 18-nov-2026 | Pide `gpui-kit ^0.10`; con el `=0.9.1` de `desktop/` no resuelve. Y sigue reescribiendo: en su código el salto duro sale como `<br>` (`markdown_source.rs`), las tablas se vuelven a escribir con `---` y la regla horizontal siempre como `---` (`serialize.rs`, que lo llama «documented lossiness»). Es un diseño de modelo + escritor: no puede devolver el fichero byte a byte. |
| `ely-editor`, `gpui-monaco` | 0.1.1, 0.2.0 | Fijan `gpui-pre =0.3.10` y `=0.3.8`: chocan con el `=0.3.12` del kit. |
| `bezel-editor` | 0.2.29 | «Notion-style block editor», pero sobre su propio fork de GPUI (`bezel-gpui`). |
| `gpui-flowtext`, `twrite-gpui`, `gpuikit-editor`, Vellum, remindr | varias | Sobre `gpui ^0.2`, otro GPUI. |
| Zed (`editor`, `markdown`) | — | No se publican (`publish = false`, GPL-3.0). Su `markdown` sólo pinta y selecciona. La edición «ya pintada» en Zed sigue sin hacerse (issue #21717 abierto; el PR #62593 se cerró sin fusionar). |
| `writ` | 0.19.3 | Ya no usa GPUI. |

Dos datos del propio kit que importan:

- **gpui-kit 0.10.0 (16-nov) sigue en gpui-pre 0.3.12.** Subir el kit no
  cambia de GPUI. Trae decoraciones de texto por rango en `Textarea`.
- **El editor de bloques dentro del kit se descartó** (PRs #3049 y #3566
  cerrados). Lo que entró es `gpui_kit::base::text::editor_host` (#3580): el
  pintor de markdown del kit ofrecido como pieza para editores de fuera, con
  la geometría de cada hoja. Sigue sin poder cambiar el tamaño de letra por
  tramo (#3762, abierto). Los campos del kit (`Input`, `Textarea`, `Editor`)
  tienen alto de línea uniforme: **un título grande dentro de un campo del
  kit no se puede hacer**, y por eso el prototipo pinta su propio texto.

## El prototipo

```sh
cd spikes/block-editor
taskset -c 0-3 env CARGO_BUILD_JOBS=3 cargo run --release -- ruta/a/un.md   # Ctrl+S guarda
taskset -c 0-3 env CARGO_BUILD_JOBS=3 cargo test                            # 17 pruebas
```

Sin fichero abre `corpus/runbook.md` (y entonces Ctrl+S no guarda nada).

Cómo está hecho: `src/doc.rs` es el modelo (sin GPUI: se prueba sin ventana),
`src/inline.rs` decide cómo se pinta el markdown de línea, y `src/view.rs` es
**un solo** `Entity` para todo el documento: un cursor (bloque, byte), un solo
receptor de teclado, y cada bloque es un `StyledText` cuyo `TextLayout` se
guarda para saber dónde cae cada byte. Los bloques van en el `list()` de GPUI,
que sólo pinta los que se ven. No se usa un campo del kit por bloque: con uno
por bloque la selección a través de bloques no se podría hacer nunca.

### Lo que hace

| | Visto pintado | Probado con teclas |
|---|---|---|
| Párrafos, títulos (h1–h6, con su tamaño), listas de viñetas y numeradas, bloque de código: cada uno editado con su aspecto | sí | sí |
| Enter parte el bloque / sigue la lista / sale de ella en un elemento vacío | sí (guion) | sí |
| Retroceso al principio: une con el de arriba (un título o un elemento pasan antes a párrafo, como en Notion) | — | sí |
| Flechas entre bloques manteniendo la columna de la pantalla, y por las líneas dobladas de un párrafo largo | — | sí |
| `# `, `## `…, `- `, `* `, `1. `, y la valla de código al principio convierten el bloque | sí (guion) | sí |
| Negrita, cursiva, código, tachado, enlaces e imágenes: pintados | sí | sí |
| Tecla muerta (´ + a → á, ¨ + u → ü), ñ, ¿¡, emoji | sí (guion, letras ya compuestas) | sí (la secuencia de X11) |
| Lo desconocido se ve como markdown en un bloque aparte y se edita como texto | sí | sí |
| Clic para poner el cursor; seleccionar **dentro** de un bloque (Mayús+flechas, Ctrl+A); copiar, cortar y pegar texto | sólo el clic | sólo el clic |
| Guardar a markdown (Ctrl+S) | — | sí |

«Guion» es `BLOCKS_DEMO=1`: la app se escribe sola un trozo de documento por
el mismo camino que el teclado, para poder capturarla sin tocar tu ratón.

### El markdown de línea, dicho claro

El texto de un bloque **es** markdown. Con el cursor fuera se pinta el
resultado (las marcas `**`, `` ` ``, `](url)` no se ven); con el cursor dentro
del bloque, las marcas aparecen apagadas y se editan como texto. Es lo que
hace Obsidian, a escala de bloque. No hay «negrita sin asteriscos» con Ctrl+B
(se puede añadir envolviendo la selección, como ya hace `cac_core::compose`).
A cambio, no hay conversión de ida y vuelta que pueda estropear nada.

### Lo que no hace, o hace mal

- **No hay deshacer.**
- **No se selecciona a través de bloques**, ni con el ratón arrastrando.
- Listas anidadas, casillas, citas, tablas, imágenes: opacas (texto).
- El cursor no parpadea ni recuerda a qué lado de un doblez de línea está:
  al principio de una línea doblada se pinta al final de la de arriba.
- El clic cae en la letra de debajo, no en el hueco más cercano.
- Un enlace por referencia (`[texto][ref]`) o una nota al pie no se pintan
  como tales: cada bloque se interpreta solo y la definición está en otro.
- Un párrafo vacío no existe en markdown: al reabrir desaparece.
- Los blancos del final de un bloque pasan a ser separación al reabrir.
- Al insertar en una lista numerada no se renumera el resto (markdown no lo
  necesita, y renumerar sería tocar bloques que no se editaron).
- Inicio/Fin van al principio/fin del párrafo lógico, no de la línea pintada.

## Fidelidad

El corpus (`corpus/`) lleva lo que el prototipo **no** edita: tablas con
alineación y barra escapada, imagen con título, listas anidadas y separadas,
saltos duros (dos espacios y barra) y blandos, HTML en bloque y en línea,
comentario, notas al pie, casillas, enlaces con título y por referencia,
títulos setext y con cierre, código sangrado, vallas de `~~~` y de cuatro
tics, fin de línea de Windows, blancos al principio, valla sin cerrar, final
sin salto de línea, un fichero vacío y otro sólo con blancos.

| Propiedad | Prueba | Resultado |
|---|---|---|
| Abrir y guardar devuelve los mismos bytes | `opening_and_saving_returns_the_same_bytes` | los 6 ficheros |
| …también con markdown roto | `any_piece_of_the_corpus_comes_back_the_same` | ~2.000 trozos del corpus (cortados, barajados) |
| Escribir en un bloque no toca el resto | `typing_in_one_block_changes_only_that_block` | cada bloque de cada fichero |
| Partir, unir y convertir sólo tocan el bloque y sus dos costuras | `structural_edits_leave_the_rest_of_the_file_alone` | cada bloque del runbook |
| Lo desconocido se queda opaco y entero | `what_is_not_understood_stays_opaque_and_whole` | 15 construcciones |
| Lo que se ve tras editar es lo que se lee al reabrir | `what_is_on_screen_after_editing_is_what_reopens` | |
| Pasear el cursor por un documento real no cambia nada; editar una tabla como texto, sí, y sólo eso | `walking_through_a_real_document_changes_nothing` (con ventana) | |

Cada prueba se comprobó rompiendo el código a propósito (`/mutar`, 24
roturas de una en una, restaurando desde la copia en memoria): **las 24 las
atrapa alguna prueba**. Entre ellas, las dos del componente rechazado
(reescribir el salto duro a `<br>` al guardar, juntar los saltos blandos al
leer), normalizar las separaciones, tratar una tabla como párrafo, partir una
lista de casillas, arreglar separaciones de bloques que no se tocaron,
olvidar la tecla muerta a medio componer, contar en bytes lo que el sistema
cuenta en UTF-16, perder la columna al cruzar un bloque, y quitar la lista
virtual.

Una red que hoy ninguna prueba pisa, y se dice: `understand` vuelve a pegar lo
que partió y, si no sale idéntico, deja el bloque opaco. Con lo que hoy se
entiende, lo partido sale siempre de trozos del propio texto y la comparación
no puede fallar; quitarla no rompe ninguna prueba. Está para el día en que
alguien enseñe a `understand` una construcción nueva y se equivoque.

## Medidas

Binario `--release`, ventana de verdad en X11, fijado a 4 núcleos compartidos
con otros dos agentes compilando (carga media 7): son números de máquina
ocupada. El fotograma se mide de `render` al final del pintado, en la CPU; no
incluye lo que tarde la gráfica en presentar. El documento es
`long_document()` de `src/main.rs`: secciones con párrafos de ~50 palabras con
markdown de línea, listas, código y una tabla cada cuatro secciones.

| Documento | Fotograma al escribir (mediana / p95) | Enter (mediana) | Leer | Memoria |
|---|---|---|---|---|
| 300 bloques, 49 kB | 2,9 / 5,1 ms | 5,4 ms | 23 ms | 212 MB |
| **3.000 bloques, 478 kB** | **3,1 / 4,7 ms** | 5,9 ms | 166 ms | 264 MB |
| 20.000 bloques, 3,2 MB | 3,2 / 5,8 ms | 7,3 ms | 2,3 s | 402 MB |
| 3.000 bloques **sin lista virtual** | 347 / 386 ms | 323 ms | — | 325 MB |
| 300 bloques **sin lista virtual** | 37 / 40 ms | 38 ms | — | 221 MB |

- De la tecla al final del pintado, con 3.000 bloques: 6 ms de mediana y
  10 ms de p95 (incluye esperar al siguiente fotograma). Atender la tecla en
  el modelo: 0,02 ms. Guardar a markdown: 0,15 ms.
- **La lista virtual hace falta y GPUI la da**: `list()` / `ListState`, con
  alturas medidas (no hay que saberlas de antemano). Sin ella el coste es
  ~0,12 ms por bloque y fotograma: a 300 bloques ya se nota al escribir.
  Una pega encontrada: `scroll_to_reveal_item` no sirve con bloques que aún
  no se han medido (ir al último de 3.000 no movía nada); se rodea en
  `BlockEditor::reveal`.
- La memoria base de una ventana GPUI vacía con las letras cargadas ronda los
  210 MB; el documento de 3.000 bloques añade unos 50 MB, que incluyen el
  árbol del parser mientras se lee.
- **Leer es lo lento**: el parser (`markdown` 1.0.0, el que ya trae el kit)
  tarda 166 ms en 478 kB y no escala lineal. Para un documento normal no se
  nota; uno enorme pediría leer en segundo plano.
- `zorite-editor` con los mismos documentos: 33 ms de mediana (68 de p95) por
  fotograma a 300 bloques, y **5,2 s por letra a 3.000**. Sirve para textos
  cortos; para un documento largo habría que partirlo en varios editores.

## Lo que falta, y cuánto cuesta

Días de trabajo con sus pruebas, por una persona que ya conoce esta base.
Referencias para el tamaño: el prototipo entero (modelo, vista y 17 pruebas)
son unas 2.200 líneas; `zorite-editor`, que hace casi todo lo de la lista
sobre un solo texto, son 16.600; `gpui-kit-markdown-editor`, 13.300.

| Qué | Lo que da GPUI / el kit | Lo que hay que hacer | Días | Riesgo |
|---|---|---|---|---|
| Selección a través de bloques | La geometría de cada bloque (`TextLayout`). Nada más | Ancla y cabeza como (bloque, byte); pintarla; Mayús+flechas y arrastre con el ratón; borrar y sustituir a través de bloques; arrastrar más allá de lo pintado (la lista virtual no tiene geometría de lo que no pinta) | 4–6 | medio |
| Copiar y pegar texto | Portapapeles de texto | Con la selección de arriba, copiar varios bloques como markdown | 1 | bajo |
| Pegar markdown | — | Partir lo pegado con `Doc::parse` e insertarlo como bloques | 1–2 | bajo |
| Pegar desde el navegador (con formato) | **Nada**: el portapapeles de GPUI no tiene HTML (en X11 está comentado en `x11/clipboard.rs`). Llega sólo el texto plano | Leer `text/html` por fuera de GPUI y convertirlo a markdown | 3–5 | **alto, sin explorar** |
| Deshacer y rehacer | El kit tiene uno genérico (`undo_history.rs`); sin mirar a fondo | Registro de operaciones sobre `Doc`, agrupando lo escrito seguido, y devolver el cursor | 2–3 | bajo |
| Tablas, paso 1: pintada, y al entrar se edita su markdown | El pintor del kit (`TextView`) pinta tablas | Bloque que alterna vista y texto | 2 | bajo |
| Tablas, paso 2: editar por celdas | Nada | Rejilla con cursor por celda, añadir y quitar filas y columnas, y escribirla sin tocar las que no cambian | 6–10 | **alto** |
| Imágenes y adjuntos | `img()` carga y pinta; pegar una imagen ya llega (`ClipboardEntry::Image`); `desktop/src/editor.rs` ya sube lo pegado y soltado | Bloque de imagen; enganchar la subida | 3–4 | medio (los pases de `cacmedia`) |
| Menú «/» | `Popover`, `PopupMenu`; la posición del cursor ya se sabe | Abrirlo al escribir `/`, filtrar, teclas, insertar | 2–3 | bajo |
| Arrastrar para reordenar | `on_drag` / `on_drop` / `drag_over` de GPUI; ningún ayudante de reordenar | Asa al pasar el ratón, raya de destino, desplazar al arrastrar cerca del borde; en el modelo es mover y arreglar costuras | 3–4 | medio |
| Menciones | Lo mismo que el menú «/» | Con el menú hecho: `@`, buscar, insertar | 1–2 | bajo |
| Enlaces a tareas y páginas | — | Pintarlos como ficha y abrirlos al pulsar; elegir destino con el mismo menú | 2–3 | bajo |
| IME | Lo da GPUI (`EntityInputHandler`), ya implementado | Probarlo con un teclado de verdad y con un IME de candidatos | 0,5–1 | **sin probar en físico** |

Y lo que no estaba en la lista pero un editor necesita: listas anidadas con
Tab (cambia el modelo: 4–6 días), casillas (1–2), citas (1–2), moverse y
seleccionar por palabras y doble clic (1–2), a qué lado de un doblez está el
cursor y parpadeo (1), Ctrl+B / Ctrl+I (1), y marcas que se revelan por
construcción y no por bloque (2).

Sumado: **unas 2 semanas para una primera rebanada que se pueda usar, 6–8
para algo que se sienta como un editor, y más de 3 meses para parecerse a
Notion**. Las estimaciones salen de lo que costó este prototipo y del tamaño
de los dos editores de referencia; las de riesgo alto son las menos fiables.

## Camino recomendado

**Construir el nuestro sobre esta base, híbrido**: la prosa (párrafos,
títulos, listas, código) se edita ya pintada; lo demás es un bloque de
markdown que se edita como texto, y se va promocionando construcción a
construcción (primero tabla pintada, luego casillas, citas…). El texto de
siempre se queda como salida de emergencia.

Por qué no adoptar `zorite-editor`: resuelve hoy mucho más que el prototipo,
pero no es por bloques, se cae con documentos largos, es de un solo
mantenedor (304 descargas) y el kit fija la versión exacta de GPUI, así que
cada subida del kit puede romperlo sin que nadie lo arregle. Sí es una opción
barata para textos **cortos** (comentarios, la descripción de una tarea) si
ahí se quiere «ya pintado» antes de tener el nuestro.

Por qué no esperar: nada apunta a que vaya a llegar hecho. El kit descartó su
editor de bloques, Zed cerró el suyo, y el componente que sí avanza
(`gpui-kit-markdown-editor`) reescribe el markdown por diseño.

### Primera rebanada (8–10 días)

1. `doc.rs` e `inline.rs` a `core/` con sus pruebas (no tienen GPUI).
2. En la página de un documento, un conmutador «bloques / markdown» junto a
   la caja de ahora; guarda por el mismo camino.
3. Lo del prototipo, más lo que no puede faltar para usarlo a diario:
   **deshacer**, seleccionar con el ratón dentro de un bloque, y el lado del
   cursor en los dobleces.
4. Antes de darlo por bueno: una pasada con el teclado español de verdad
   (´, ¨, ñ, AltGr) — es lo único de esta prueba que no se pudo comprobar.

Después, por este orden: selección a través de bloques, menú «/» (y con él
menciones y enlaces), tabla pintada, arrastrar.

## Lo que hay en esta carpeta

- `src/doc.rs` — el modelo sin pérdida y las operaciones de edición.
- `src/inline.rs` — qué se pinta de cada byte del markdown de línea.
- `src/view.rs` — el editor en GPUI.
- `src/main.rs` — la ventana y los ganchos `BLOCKS_*` para medir y capturar.
- `tests/fidelity.rs` (8, sin ventana) y `tests/editing.rs` (9, con la
  ventana sin pantalla del kit y teclas de verdad).
- `corpus/` — el markdown de prueba.
- `probes/zorite/` — `zorite-editor` sobre las mismas versiones, con sus tres
  pruebas y su medida. Se compila aparte:
  `cd probes/zorite && taskset -c 0-3 env CARGO_BUILD_JOBS=3 cargo test`.
