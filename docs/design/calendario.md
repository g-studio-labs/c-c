<!-- Handoff de diseño de jose (11-nov-2026), tal cual llegó en «Calendar UI redesign mockups.zip». Capturas del prototipo adjuntas a command-and-control-333. Lo que el repo resolvió distinto (base-ui en vez de Radix, cinco tonos de prioridad) está en «Diseño · Calendario», en cac. -->

# Handoff: Calendario de «Mi trabajo» (Mes + Semana + Por programar)

## Resumen
Rediseño de la vista calendario de **Mi trabajo** (`app/src/pages/MyWork.tsx` → `app/src/components/ItemCalendar.tsx`). Problemas que resuelve:
1. «+N más» abría el detalle del día **debajo de todo el mes**, fuera de la pantalla.
2. Los títulos truncados a ~15 caracteres no se podían leer.
3. Las tareas sin fecha quedaban en una tira al final y nadie las programaba.

Solución:
- **Mes**: cada celda muestra un resumen visual (contador, barra de prioridad, mapa de calor) y un **popover anclado al día** con el detalle completo.
- **Semana**: una vista nueva con tarjetas completas y un resumen de la semana.
- **Por programar**: un carril lateral colapsable desde donde se arrastran las tareas sin fecha a un día.

## Sobre los archivos de diseño
Los archivos de este paquete son **referencias de diseño hechas en HTML**: prototipos que muestran el aspecto y el comportamiento esperados, no código de producción para copiar. La tarea es **recrearlos en la app existente** (React 19 + Tailwind + shadcn/ui + dnd-kit + lucide-react), con sus patrones, tokens (`index.css`) y componentes (`Button`, `Popover`, `HoverCard`, `Sheet`, etc.).

Para verlo, abre `Calendario Mi Trabajo - Final.dc.html` en un navegador (necesita `support.js` y `fonts/` al lado). Es interactivo: hover y clic en los días, selector Mes/Semana, flechas, «Hoy», botón «Por programar» y arrastrar tarjetas.

## Fidelidad
**Alta (hi-fi), tema oscuro.** Colores, tipografía, espaciados e interacciones son finales. Todos los colores salen de los tokens de `.dark` en `app/src/index.css`; usa las variables (`--primary`, `--warning`…) y no los hex, así el tema claro funciona solo. Hay tres valores intermedios que no existen como token (ver «Tokens»): conviene agregarlos.

---

## Vistas

### Encabezado de la vista (común)
Va dentro del contenido de Mi trabajo, debajo de la fila de filtros. `padding: 16px`, `gap: 14px` entre los bloques.

Fila (`flex; justify-between; flex-wrap; gap 12px`):
- **Izquierda** (`gap 14px`):
  - Título: Mes → «Noviembre de 2026» (mes con mayúscula inicial y «de» en minúscula; hoy sale «Noviembre De 2026», hay que corregirlo). Semana → «9 – 15 noviembre 2026» (si la semana cruza de mes: «30 nov – 6 diciembre 2026»). 17px/600, `min-width: 190px` para que no salte.
  - **Botón «Por programar»**: alto 28px, `padding 0 10px 0 8px`, radio 6px, borde 1px `--border`, 13px. Icono lucide `PanelLeft` de 16px, el texto «Por programar» y un badge con el conteo (18px de alto, `min-width 18px`, radio full, 11px/700).
    - Carril **oculto**: badge con `bg --primary` y texto `--primary-foreground` (llama la atención); botón con fondo transparente.
    - Carril **visible**: badge `bg --border`, texto `--foreground`; botón con `bg --accent` y borde `#3A3A44` (estado activo).
  - **Segmented Mes / Semana**: contenedor `bg --muted`, borde `--border`, radio 8px, `padding 2px`, `gap 2px`, 13px. Opción: `padding 3px 12px`, radio 6px. La activa lleva `bg --accent`, `--foreground` y 600; la inactiva `--muted-foreground` y 400.
- **Derecha** (`gap 14px`):
  - Leyenda: tres puntos de 8px con su texto (12px, `--muted-foreground`): «Alta» `--warning`, «Normal» `--info`, «Sin prioridad» `#5F5F6B`.
  - Navegación: `‹` (28×28) · «Hoy» (alto 28, `padding 0 10px`, 13px/600) · `›`. Son los mismos `Button variant="outline"` que hoy. Hover `bg --accent`.

### Vista Mes
**Layout**: fila `flex; gap 12px; align-items flex-start`. A la izquierda el carril (ver «Por programar»), a la derecha `flex:1; min-width:0` con la rejilla.

**Rejilla**: `grid-template-columns: repeat(7, minmax(0,1fr))`, `gap 1px` sobre `bg --border` (las líneas), borde 1px `--border`, radio 10px. **Sin `overflow:hidden`**, porque el popover se sale de la celda. Los radios se aplican a las celdas de las esquinas: encabezado lun `9px 0 0 0`, dom `0 9px 0 0`, y las dos celdas de la última fila. Filas según el mes (`ceil((offset + díasDelMes)/7)`), semana de lunes a domingo.

- **Encabezado de días**: «lun mar mié jue vie sáb dom», `bg --muted`, `padding 4px 8px`, centrado, 13px/500, `--muted-foreground`.
- **Celda del día** (`min-height 96px`, `padding 6px 8px`, `cursor pointer`, `position relative`). Contenido en columna `justify-between`, `min-height 84px`:
  - Arriba: número en un círculo de 22px, 13px. Normal: `--muted-foreground`/400. **Hoy**: `bg --primary`, texto `--primary-foreground`/600. A la derecha, un **punto de 7px `--error`** si el día tiene tareas vencidas sin cerrar (`title="Vencidas sin cerrar"`).
  - Abajo, solo si hay tareas (`gap 5px`):
    - Contador: número 20px/600, `line-height 22px`, `tabular-nums`, seguido de «tarea»/«tareas» en 11px/400 `--muted-foreground` con `margin-left 4px`.
    - Barra de prioridad: alto 5px, radio full, `gap 2px`, un segmento por prioridad con `flex: <conteo>` en el orden alta → normal → sin prioridad. Las prioridades sin tareas no se pintan.
  - **Fondo según la carga** (mapa de calor): 0 → `--background` #101013 · 1 → `--sunken` #131317 · 2–3 → `#17171C` · 4+ → `--raised` #1C1C22. Hover: `#1A1A20`.
  - Días fuera del mes: el contenido al 40% de opacidad (como hoy).
  - Popover abierto: anillo `inset 0 0 0 1px` con `--primary` al 70%, y la celda con `z-index 20`.
  - Destino de arrastre activo: `bg` con `--primary` al 10% y anillo `inset 0 0 0 2px --primary`.
- **Se eliminan**: los títulos en la celda, `MAX_PER_DAY`, «+N más» y el panel del día seleccionado debajo del mes.

### Popover del día (vista Mes)
- **Apertura**: `mouseenter` en la celda + **220 ms** → modo vistazo; `mouseleave` lo cierra. **Clic** en la celda → lo **fija**: queda abierto aunque salga el mouse y aparece la ×. Otro clic en la misma celda lo suelta.
- **Cierre**: × · `Esc` · clic fuera de cualquier celda (`mousedown` en el documento) · empezar un arrastre.
- **Recomendación**: Radix `Popover` (controlado: `open = pinned ?? hovered`) con `HoverCard`, o un solo `Popover` con los temporizadores a mano. Mientras está fijado, el hover sobre otros días **no** abre otro.
- **Posición**: anclado a la celda con un desfase de −8px, de modo que tapa el borde de la celda. Columnas lun–jue → `left:-8px`; vie–dom → `right:-8px`. Dos últimas filas → `bottom:-8px` (abre hacia arriba); el resto → `top:-8px`. Con Radix basta `side`/`align` y `collisionPadding`.
- **Caja**: ancho 340px (`max-width: calc(100vw - 80px)`), `bg --popover` #1D1D23, borde 1px `#3A3A44`, radio 12px, `box-shadow: 0 18px 40px rgba(0,0,0,.55)`, `padding 12px`, `gap 8px`.
- **Contenido**, en orden:
  1. Cabecera (`justify-between`):
     - Izquierda: día de la semana en 11px, mayúsculas, `letter-spacing .06em`, `--muted-foreground` («miércoles · hoy» si es hoy); debajo «11 de noviembre» en 18px/600.
     - Derecha: «5 tareas» (12px, muted) y, si está fijado, la × (26×26, radio 6, lucide `X` de 15px, hover `bg --accent`).
  2. Chips de prioridad (`flex-wrap; gap 6px`): punto de 6px + «2 alta» / «1 normal» / «2 sin prioridad». 11px, `bg #17171B`, borde `--border`, radio full, `padding 1px 8px`.
  3. Lista (`max-height 320px; overflow auto`, `margin 0 -4px`). Cada fila: `padding 6px`, radio 8, hover `bg --accent`, clic → `onOpen(id)`.
     - Barra de 3px a todo el alto con el color de prioridad (sin prioridad: `#3A3A44`).
     - Título completo en 13px, `line-height 18px`, `text-wrap: pretty`. Las hechas en `--muted-foreground` y tachadas.
     - Meta (11px, muted, `gap 6px`): `#folio` en `--font-mono` · punto de 5px con el color del estado · nombre del estado · «· Vencida» en 10px/600 `--error` si aplica.
     - A la derecha, avatar de 20px (iniciales 9px/700 sobre el color de `lib/person-color.ts`).
  4. Vacío: «Nada vence este día.» (13px, muted).
  5. Pie: separador `--border` y `padding-top 8px`. A la izquierda, «+ Nueva tarea para este día» (12px, `--primary`, lucide `Plus` de 14px), que abre la creación de tarea con `dueAt` prellenado. A la derecha, solo en modo vistazo, «Clic para fijar» (12px, `#5F5F6B`).

### Vista Semana
**Resumen de la semana** (arriba, solo en Semana): `grid-template-columns: repeat(4, minmax(0,1fr)) minmax(0,2fr)`, `gap 1px` sobre `--border`, borde `--border`, radio 10px, `overflow hidden`. Cada bloque: `bg --muted`, `padding 10px 14px`, etiqueta en 11px muted y valor en 20px/600:
- «Esta semana» → «N tareas»
- «Prioridad alta» → número en `--warning`
- «Vencidas sin cerrar» → número en `--error` (cuenta todo lo vencido, no solo esta semana)
- «Hechas» → número en `--success`
- «Mezcla de prioridad» → barra de 6px (fondo `#1B1B21`) y debajo la leyenda con los conteos.

**Columnas**: `grid-template-columns: repeat(5, minmax(0,1fr)) repeat(2, minmax(0,.62fr))` (fin de semana angosto; opción para 7 iguales), `gap 8px`.
- **Columna**: `min-height 480px`, `bg --muted`, borde 1px `--border`, radio 10, `padding 8px`, `gap 6px`.
  - Hoy: `bg #14161A` y borde `--primary` al 45%.
  - Destino de arrastre: `bg` con `--primary` al 8% y borde `--primary`.
  - Encabezado (`padding 2px 2px 8px`, borde inferior `--border`):
    - Día de la semana en 12px, mayúsculas, `letter-spacing .05em`, muted.
    - Número en una píldora de 24px, 14px/600. Hoy: `bg --primary`. Días pasados: muted.
    - A la derecha, «N abiertas» (11px, muted).
    - Debajo, la barra de prioridad de 4px (fondo `#1B1B21`).
  - Días pasados: tarjetas al 75% de opacidad.
  - Vacío: caja punteada «Libre» (`min-height 80px`, 12px, `#5F5F6B`).
- **Tarjeta**: `bg --raised` #1D1D23, borde 1px `--border` y borde izquierdo de 3px con el color de prioridad, radio 7, `padding 8px`, `gap 6px`, `cursor grab`, hover `bg --accent`.
  - Fila 1: `#folio` (mono, 11px, muted) · chip «Vencida» (10px/600 `--error`, fondo `--error` al 12%, radio 4) · avatar de 18px a la derecha.
  - Título completo: 12.5px, `line-height 17px`, `text-wrap: pretty`. Las hechas en muted y tachadas.
  - Estado: punto de 6px + nombre (11px, muted).
  - Vencida (con `dueAt` anterior a hoy y no hecha): borde con `--error` al 55%.
  - Mientras se arrastra, la de origen queda al 40% de opacidad.
- Clic en la tarjeta → `onOpen(id)`. Arrastrar → cambiar de día.

### Por programar (las dos vistas)
- **Contenedor**: `flex:none`, `width` animado **220px ↔ 0** (con `margin-right` de 0 a −12px para absorber el gap), `transition: width .22s ease, margin-right .22s ease`, `overflow hidden`, `position: sticky; top: 12px`.
- **Carril**: ancho fijo de 220px, borde 1px **punteado** con `--primary` al 40%, `bg` con `--primary` al 3%, radio 10, `padding 8px`, `gap 6px`, `max-height: calc(100vh - 24px)` con scroll.
  - Encabezado: «Por programar» (12px/600, `--primary`) · conteo en una píldora (11px, `bg #1B1B21`) · botón `ChevronLeft` de 22px a la derecha para ocultar.
  - Tarjetas: mismas que las de Semana, pero con un asa (`GripVertical` de 12px) antes del `#folio` y, abajo, dos botones de 11px: «→ Hoy» (hover `bg --primary` y texto oscuro) y «Mañana» (hover `bg --accent`).
  - Vacío: «Todo tiene fecha.» (12px, `--success`).
  - Pie: «Arrastra a un día para ponerle fecha. Suelta aquí para quitársela.» (11px, `#5F5F6B`).
  - Destino de arrastre activo: borde `--primary` y `bg` con `--primary` al 8%.
- **Estado por vista**: **Mes → oculto** por defecto (el mes necesita el ancho); **Semana → visible**. Guardar la preferencia (localStorage o el store `mywork`).

---

## Interacciones y comportamiento
- **Arrastre** (usar el `DndContext` que ya existe: `PointerSensor` con `distance: 8` + `KeyboardSensor`, `pointerWithin`):
  - Por programar → celda del mes o columna de la semana: `onSchedule(id, iso)` (ya existe, llama a `updateTask(id, { dueAt })`).
  - Tarjeta de la semana → otra columna: lo mismo.
  - Cualquier tarjeta → el carril: **nuevo**, `updateTask(id, { dueAt: null })`.
  - Al empezar un arrastre se cierran los popovers y se ignora el hover hasta soltar.
  - El prototipo usa drag nativo de HTML5 solo para la demo; en la app va dnd-kit.
- **«→ Hoy» / «Mañana»**: ponen `dueAt` en hoy o mañana (día local, mismo manejo que `day` en `CalendarItem`, sin pasar por `new Date()`).
- **Navegación**: en Mes, `‹ ›` cambia de mes; en Semana, mueve ±7 días. «Hoy» lleva el cursor al mes y a la semana actuales en las dos vistas. Pasar de Semana a Mes abre el mes de la semana visible.
- **Teclado**: `Esc` cierra el popover. Las celdas deberían ser focusables y abrir el popover con `Enter`/`Space` (en el prototipo falta).
- **Optimista**: aplicar el cambio de fecha al instante y revertir con `toast.error` si falla (hoy ya hay `.catch(toast.error)`).
- **Responsive**: por debajo del breakpoint de `useIsMobile` se usa la versión móvil (ver la sección «Móvil» abajo), que reemplaza a `CompactMonth`. Entre ~900 y 1100px la leyenda y la navegación bajan a una segunda línea (flex-wrap) y es aceptable; con el carril abierto en Mes, cada celda queda de unos 100px.

## Móvil (web en teléfono, `useIsMobile`)
Prototipo: `Calendario Mi Trabajo - Móvil.dc.html`: 4 teléfonos de 390×844 con el mismo prototipo arrancando en estados distintos. Reemplaza a `CompactMonth` en `ItemCalendar.tsx` y conserva su decisión principal: **tocar un día abre una hoja inferior** y el mes no se mueve (jose, 18-oct-2026). En táctil **no hay hover ni arrastre**.

**Chrome existente** (no cambia): header de 48px con búsqueda y campana, `MobileTabBar` abajo (Mi trabajo / Comunicación / Más, h-14, la activa en `--primary` con la raya arriba).

**Barra de herramientas** (debajo de las pestañas de Mi trabajo; `padding 10px 14px`, `gap 8px`, borde inferior `--border`). Todos los toques son de ≥40px:
- Segmented **Mes / Semana**: cada opción de 36px de alto, `padding 0 14px`, radio 7. Contenedor `bg --muted`, borde `--border`, radio 9, `padding 2px`.
- Botón **Por programar**: 44×40, radio 8, borde `--border`, lucide `CalendarMinus` de 18px. Badge de conteo arriba a la derecha (`top/right -6px`, 18px, `bg --primary`, 11px/700). Abre la hoja «Por programar».
- A la derecha: `‹` (40×40) · «Hoy» (40 de alto, `padding 0 12px`, 13px/600) · `›` (40×40). En Mes navegan por mes; en Semana, ±7 días.

**Área de contenido**: scroll vertical, `padding 12px 14px 16px`, `gap 14px`.
- Arriba, el título del periodo: «Noviembre de 2026» o «9 – 15 nov», 16px/600.
- A la derecha del título, si hay vencidas: punto de 6px `--error` + «N vencidas» (12px `--error`).

### Mes móvil (1a)
- Rejilla de 7 columnas, `row-gap 2px`.
  - Iniciales L M X J V S D: 11px/500, muted, `padding-bottom 4px`.
  - **Día**: 50px de alto, radio 10.
    - Número en un círculo de 28px, 14px. Hoy: `bg --primary`, texto `--primary-foreground`/600.
    - Punto de 6px `--error` arriba a la derecha del número si hay vencidas.
    - Debajo, **barra de carga** de 3px: ancho `min(n,5)*5+4` px (crece con las tareas, tope en 5), segmentada por prioridad (`flex: n`, `gap 1.5px`).
  - Fuera del mes: opacidad .35. El día con la hoja abierta lleva `bg --accent`.
- **Banner Por programar** (solo si hay sin fecha):
  - Caja: `padding 12px`, borde 1px punteado `--primary` al 45%, `bg` `--primary` al 4%, radio 12.
  - Contenido: círculo de 30px con el conteo (`bg --primary`, 14px/700), «Por programar» (14px/600 `--primary`), «Tareas sin fecha — dales un día» (12px muted) y un chevron.
  - Tocar → hoja Por programar.
- **Próximas** (ya existía, restilizada):
  - Etiqueta «Próximas» en 12px/500 muted. Hasta 5 tareas no hechas desde hoy.
  - Tarjeta: `bg --raised`, borde `--border`, radio 10, `padding 10px`, `gap 10px`.
  - Contenido: barra de prioridad de 3px · fecha en una columna de 52px («Hoy» en `--primary`/600, «Mañana», «13 nov») · título de 14px/19px con `line-clamp 2` · meta de 11px (`#folio` mono, punto de estado y nombre del estado).

### Hoja del día (1b), equivalente al popover del escritorio
- **Overlay**: `rgba(0,0,0,.55)`; tocarlo cierra.
- **Panel**: `bg --muted`, borde superior `--border`, radio `18px 18px 0 0`, `max-height 76%`, `padding 8px 16px 30px` (más `env(safe-area-inset-bottom)`), `gap 12px`, sombra `0 -10px 30px rgba(0,0,0,.4)`. Reusar `Sheet side="bottom"` (ya se usa).
- **Asa**: 40×4, radio full, `--muted-foreground` al 35%.
- **Cabecera**:
  - Kicker en 11px, mayúsculas, `letter-spacing .06em`, muted: «viernes · 5 tareas» («· hoy» si aplica).
  - Título «13 de noviembre» en 20px/600.
  - × de 40×40, radio 10, `bg #1B1B21`.
- **Barra de prioridad** de 5px y la leyenda con los conteos (12px muted).
- **Tarjetas**:
  - Caja: `padding 12px`, `bg --raised`, radio 10, `gap 10px`.
  - Barra de prioridad de 3px · título completo de 14px/19px · meta de 12px (`#folio`, estado y «· Vencida» en `--error`/600) · avatar de 24px.
  - Borde: `--error` al 55% si está vencida.
  - Hechas: tachadas y en muted.
  - Tocar → `onOpen(id)` y se cierra la hoja (ya es así).
- **Vacío**: «Nada vence este día.».
- **Pie**: botón «+ Nueva tarea para este día» de 46px, radio 10, borde `--border`, 14px/500 `--primary`.

### Semana móvil (1c)
- **Tira de 7 días** (`gap 4px`), cada uno de 66px, radio 12.
  - Contenido: inicial (11px/500 muted), número en un círculo de 26px (14px/600; hoy con `bg --primary`) y la barra de carga de 3px.
  - El día elegido lleva `bg --raised` y borde `#3A3A44`.
- **Chips de resumen** (12px, radio full, `padding 4px 10px`):
  - «N esta semana» (`bg #17171B`, borde `--border`)
  - «N alta» (`--warning` sobre `--warning` al 12%)
  - «N hechas» (`--success` sobre `--success` al 12%)
- **Día elegido**:
  - Título «Miércoles 11 · hoy» (15px/600) con «N tareas» a la derecha.
  - Tarjetas como las de la Semana del escritorio, en grande: `padding 12px`, radio 10, título de 14px/19px, `#folio` de 12px, avatar de 22px, chip «Vencida» de 11px.
  - Vacío: caja punteada «Día libre».
- **Gesto**: deslizar a la izquierda o la derecha (>50px) cambia al día siguiente o anterior. Lo indica el texto «Desliza ← → para cambiar de día» (11px, `#5F5F6B`). Las flechas de arriba mueven por semana.

### Hoja Por programar (1d)
- Misma hoja: kicker «Sin fecha · N», título «Por programar» y la ayuda «Elige cuándo las haces. Sin fecha no aparecen en el calendario.» (13px muted).
- **Tarjeta**:
  - Caja: `padding 12px`, `bg --raised`, borde izquierdo de 3px con la prioridad, radio 10, `gap 10px`.
  - Arriba, `#folio` y el avatar de 22px. Luego el título de 14px.
  - Fila de atajos (`gap 6px`, cada uno de 38px de alto, radio 8, 13px):
    - **Hoy**: `bg --primary`, texto oscuro, 600.
    - **Mañana**: borde `--border`.
    - **Lunes** (el próximo lunes): borde `--border`.
    - **Elegir fecha**: 44px de ancho, icono `Calendar`; abre el `DatePicker` que ya existe.
- Al programar, la tarjeta sale de la lista y aparece un **toast con «Deshacer»** (sonner: `toast("#203 → hoy", { action: { label: "Deshacer", onClick } })`, 4 s). Deshacer devuelve el `dueAt` anterior.
- Vacío: «Todo tiene fecha.» (`--success`).

## Estado
- `view: 'month' | 'week'`
- `cursor` (primer día del mes) y `weekStart` (lunes)
- `hovered: dayKey | null` con un temporizador de 220 ms
- `pinned: dayKey | null`
- `dragging: id | null` y `over: dayKey | 'lane' | null` (dnd-kit ya los da)
- `laneOpen: { month: boolean, week: boolean }`, persistido
- Solo móvil: `sheet: 'day' | 'lane' | null`, `picked: dayKey` y el último cambio para Deshacer (`{ id, prevDueAt }`)
- **Datos que necesita cada item**, además de lo que ya lleva `CalendarEntry`:
  - `priority`: ya existe, `priorityDot()` en `components/tasks/PriorityChip.tsx`.
  - `status`: abierta / en curso / hecha, colores de `lib/status-color.ts`.
  - `assignee` (iniciales y color de `lib/person-color.ts`).
  - Vencida = `dueAt` < hoy y estado no terminado (ver `FINISHED` en MyWork).
  - Conviene extender `CalendarEntry` con `priority`, `status` y `assignee`, o pasar un `renderMeta`.

## Tokens (`.dark` en `app/src/index.css`)
| Uso | Token | Hex (oscuro) |
|---|---|---|
| Fondo | `--background` | #101013 |
| Hundido, encabezados | `--muted` / `--sunken` | #131317 |
| Tarjeta, popover | `--raised` / `--popover` | #1D1D23 |
| Hover | `--accent` | #212128 |
| Borde | `--border` | #2B2B33 |
| Texto | `--foreground` | #ECECEE |
| Texto secundario | `--muted-foreground` | #9A9AA4 |
| Acento, hoy, foco | `--primary` | #20D9E8 |
| Prioridad alta | `--warning` | #FBBF24 |
| Prioridad normal | `--info` | #38BDF8 |
| Vencida, urgente | `--error` | #FB7185 |
| Hechas | `--success` | #34D399 |
| Estado abierta / en curso / hecha | `--status-open` / `--status-progress` / `--status-done` | #60A5FA / #FBBF24 / #34D399 |

**Valores nuevos sugeridos** (no existen como token):
- `--muted-soft` #5F5F6B: sin prioridad, textos terciarios.
- `--border-strong` #3A3A44: borde del popover, hover de tarjeta.
- Mapa de calor 2–3 tareas: #17171C.
- Fondo de la columna de hoy: #14161A.
- Pista de barra: #1B1B21.

Para el tema claro, hay que derivar equivalentes de estos.

- **Tipografía**: Inter Variable (UI) e Iosevka Term Slab (`--font-mono`, folios), las dos ya vendorizadas. Tamaños usados: 11, 12, 12.5, 13, 14, 17, 18, 20px. Pesos: 400, 500, 600, 700.
- **Radios**: 4 (chip vencida) · 5–6 (botones) · 7 (tarjeta) · 8 (segmented, filas del popover) · 10 (rejilla, columnas, carril) · 12 (popover) · full (píldoras, avatares).
- **Sombra**: solo el popover, `0 18px 40px rgba(0,0,0,.55)`.
- **Movimiento**: hover del popover a los 220 ms; carril `width .22s ease`.

## Assets
- Iconos: **lucide-react** (ya es dependencia): `ChevronLeft`, `ChevronRight`, `X`, `Plus`, `PanelLeft`, `GripVertical`. Los del rail y la barra superior del prototipo solo dan contexto: no forman parte de este cambio.
- Fuentes: `app/src/assets/fonts/` (se copiaron a `fonts/` solo para el prototipo).
- Los datos del prototipo (títulos, estados, responsables, tareas sin fecha) son de ejemplo.

## Archivos
- `Calendario Mi Trabajo - Final.dc.html`: el prototipo final, interactivo. La lógica está en el `<script data-dc-script>` del final del archivo (`renderVals()` calcula la rejilla, los segmentos, el popover y el arrastre).
- `Calendario Mi Trabajo - Móvil.dc.html`: la versión para teléfono (4 estados interactivos).
- `Calendario Mi Trabajo.dc.html`: la exploración (rondas 1 y 2), con la vista actual recreada (1a) y las alternativas descartadas. Sirve de contexto para decisiones.
- `support.js`: el runtime para abrir los `.dc.html`.
- `fonts/`: Inter e Iosevka.

**Archivos del repo a tocar**: `app/src/components/ItemCalendar.tsx` (mes, semana, popover, carril), `app/src/pages/MyWork.tsx` (props: `onUnschedule`, metadatos de estado y responsable), `app/src/locales/{es,en}/common.json` y `work.json` (textos nuevos: «Por programar», «Semana», «Nada vence este día.», «Clic para fijar», «N abiertas», «Libre», «Vencida», «Todo tiene fecha.», «Arrastra a un día…», «Nueva tarea para este día»), y los tests `ItemCalendar.*.test.tsx`: actualizar «+N más» y la tira de sin fecha.
