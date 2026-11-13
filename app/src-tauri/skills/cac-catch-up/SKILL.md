---
name: cac-catch-up
description: Pone un espacio de cac al día con lo que pasó fuera desde la última vez (los tickets que se movieron y se comentaron en Jira, las decisiones y lo por definir que el cliente apunta en Notion, las reglas nuevas) y propone qué entra en cac. No escribe nada hasta que el usuario aprueba, y nunca escribe en el tracker. Úsala cuando digan «ponte al día», «qué pasó en Jira», «sincroniza con Jira», «trae lo de Notion», o una vez al día por espacio.
---

# Poner cac al día

cac es la fuente, y lo de fuera es copia. Pero la copia se mueve sola: el
equipo de un cliente comenta y decide en Jira y en Notion, y no lo trae a
cac. Medido en un proyecto el 12-nov-2026: en siete días, 183 tickets
movidos y 156 comentarios en 57 tickets; sólo 7 tenían una tarea de cac que
los nombrara, y de 35 decisiones escritas allí, ninguna estaba en cac.

Esta skill es el pase que lo trae, **a mano**: lee desde donde se quedó,
propone todo junto, y sólo escribe lo que el usuario apruebe. Decidido con
jose el 12-nov-2026 (decisión «cac es la fuente; Jira y Notion son copias»,
espacio Command and control).

## 1 · De dónde leer

`get_space_sources` del espacio (el de `get_context`, o el que nombre el
usuario). Cada fuente dice qué es y hasta cuándo se leyó ya (`lastSyncAt`):

| `role` | Qué es | Con qué se lee |
|---|---|---|
| `tracker` | el tablero donde se mueven los tickets (`ref`: la clave del proyecto de Jira) | el MCP de Atlassian |
| `decisions` | donde el cliente apunta lo resuelto (`ref`: la URL) | el MCP de Notion |
| `open-questions` | lo que está por definir | el MCP de Notion |
| `rules` | el registro de reglas: una página **de cac** (`ref`: su id; `pattern`: cómo se escribe una regla) | `get_doc_page` |
| `rules-copy` | la copia del registro que lee el cliente (`ref`: la URL) | el MCP de Notion |

**Si el espacio no tiene fuentes**, no las inventes: pregúntale al usuario
cuáles son (la clave del proyecto de Jira, las páginas), enséñale la lista
como quedaría y guárdala con `set_space_sources` después del sí.

**Si falta la herramienta de una fuente** (la sesión no tiene el MCP de
Atlassian o de Notion), dilo, sáltate esa fuente y **no muevas su cursor**.

Apunta la hora a la que empiezas: es la que irá al cursor.

## 2 · Qué leer, y sólo eso

Desde `lastSyncAt` de cada fuente. Sin cursor (la primera vez), los últimos
siete días, y díselo al usuario. Nunca el proyecto entero.

- **`tracker`**: los tickets actualizados desde el cursor
  (`project = <ref> AND updated >= "<cursor>" ORDER BY updated`). De cada
  uno: estado, a quién está asignado, y sus comentarios posteriores al
  cursor. Su tarea de cac, con `get_task` y la clave (`PROTEUS-165`): si no
  hay ninguna, o contesta varias, se apunta así.
- **`decisions`** y **`open-questions`**: la página, y qué entradas no están
  todavía en cac (`search` con dos o tres palabras de cada una).
- **`rules`**: el registro, para saber qué reglas existen. Una regla citada
  en un comentario nuevo que `get_related` no encuentra es una regla que
  falta.

## 3 · Qué proponer: una tabla, antes de escribir nada

Todo junto, agrupado así, cada fila con **de dónde sale** (el ticket o la
página, quién, cuándo, el enlace) y **lo que se escribiría** tal cual:

1. **Decisiones.** Un comentario con el formato «Decisión / Fuente /
   Supuesto / Se revierte si», o una entrada nueva de la página de
   decisiones. Título, cuerpo con el porqué, quién la escribió allí y su
   enlace.
2. **Tareas que se movieron fuera.** El ticket cambió de estado y su tarea de
   cac dice otra cosa: el estado nuevo, y una línea de por qué.
3. **Lo que hay que saber en la tarea.** Un dato, un bloqueo, un cambio de
   alcance dicho en un comentario: el comentario que se añadiría a la tarea
   de cac, con el enlace.
4. **Reglas.** Una regla nueva, o una que cambió: la sección que se añadiría
   al registro, con su formato (ver abajo).
5. **Por definir.** Lo que se abrió y lo que se cerró.
6. **Tickets sin tarea en cac.** Sólo la lista, con título, estado y de quién
   son. **No se propone crear una tarea por cada uno**: la mayoría son del
   equipo del cliente y no son trabajo de este lado. El usuario dice cuáles
   quiere.

Lo que ya está en cac no se propone. Una puesta al día vacía («nada nuevo
desde el martes») es una respuesta válida y corta.

Si son más de treinta filas, enséñalas por grupos y pide el sí grupo a
grupo: una tabla que no se lee se aprueba sin leer.

## 4 · Escribir, después del sí

Sólo lo aprobado, fila por fila:

- **Decisiones:** `record_decision` con `sourceUrl` (el enlace al comentario
  o a la entrada), `sourceAuthor` (quien la escribió allí), `decidedBy` con
  el nombre del usuario (sólo acepta gente de la organización) y
  `originTaskId` si el ticket tiene tarea; si no, en el nodo del espacio. Si
  contesta `recorded: false`, ya estaba: la misma fuente no entra dos veces.
- **Estados:** `update_task` con `status` y `comment` («En Jira pasó a … el
  …», con el enlace). Si la tarea la lleva otra persona, sólo el comentario.
- **Comentarios:** `add_task_comment`, con el enlace.
- **Reglas:** `append_doc_page` en el registro, una sección por regla:

  ```markdown
  ## R-012 · La regla en una línea

  - **Tipo:** dictada | inferida
  - **Estado:** vigente
  - **Fuente:** quién y dónde, con enlace
  - **Vigente desde:** 2026-11-12
  - **Reemplaza a:** R-005
  - **Se revierte si:** …
  ```

  El número que ya trae la regla se respeta. Una sin número toma el
  siguiente libre, y un número no se reutiliza nunca. Una regla que cambia
  no se reescribe: entra la nueva con «Reemplaza a», y a la vieja se le pone
  **Estado:** reemplazada por R-nnn (`write_doc_page_section`).
- **Tickets sin tarea** que el usuario eligió: `create_task`, con la línea
  `**Jira:** PROTEUS-165` en la descripción, que es lo que la hace la tarea
  de ese ticket.

**En el tracker no se escribe nada.** Ni un comentario, ni un cambio de
estado: aquí se lee. Si algo de cac tiene que llegar allí, se le dice al
usuario.

## 5 · La copia del registro en Notion

Sólo si el espacio tiene una fuente `rules-copy`, y sólo si el registro
cambió (en esta pasada o desde su `lastSyncAt`).

El registro vive en cac y la página de Notion es su copia para quien no usa
cac: se **regenera entera** desde cac, no se edita a mano. Antes de
escribir:

1. Lee la página de Notion. Si alguien la cambió a mano desde la última
   copia (dice algo que el registro de cac no dice), **no la pises**:
   enséñale la diferencia al usuario. Lo que valga entra primero en cac, como
   regla.
2. Enséñale al usuario qué cambia en la copia (reglas nuevas, las que
   cambian de estado).
3. Con el sí, reescribe la página con el contenido del registro y una
   primera línea: «Copia del registro de reglas, generada desde cac el
   <fecha>. No se edita aquí.»

## 6 · Mover el cursor, y sólo si terminó

De cada fuente que se leyó **entera** y cuyas filas quedaron escritas o
descartadas por el usuario: `set_space_sources` con
`synced: { id, at: <la hora a la que empezaste> }`. La hora del principio y
no la del final: lo que llegó mientras leías se lee la próxima vez.

Una fuente que no se terminó (faltó la herramienta, el usuario paró a la
mitad, un error) **se queda con su cursor**: la próxima pasada relee ese
tramo, y como las decisiones no entran dos veces, releer no duplica.

Y dile al usuario, en dos líneas, qué entró y hasta cuándo quedó leída cada
fuente.

## Lo que no se hace

- No escribir nada sin haberlo enseñado antes.
- No escribir en Jira ni mover tickets.
- No crear una tarea por cada ticket del tablero.
- No leer más atrás del cursor «por si acaso».
- No mover el cursor de una fuente que no se leyó entera.

<!-- cac-catch-up v1 -->
