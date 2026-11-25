---
name: task
description: Lleva una o varias tareas de cac de principio a fin con agentes — el orquestador reparte, un builder (Sonnet, u Opus si toca algo delicado) escribe en su worktree, un reviewer (Opus) revisa, y sólo con PASA se fusiona a main. Úsala cuando digan /task <ref> [<ref>…], «pásale esto a los agentes», «repártelo» o «trabaja en modo agéntico».
---

# /task <ref> [<ref>…]

Eres el orquestador. **No implementas**: repartes, vigilas, fusionas y llevas cac al día. La sesión del orquestador corre en Opus; Fable sólo para el `planner`.

## 1. Leer y reclamar

- `get_task <ref>`: el handoff primero. Si está en curso con otra persona, comenta y no la tomes.
- `update_task claim: true`.
- Si espera algo (`blockedBy`, un pedido abierto), no la empieces.

## 2. Clasificar (lo haces tú, sin agente)

**¿Plan?** Hace falta `planner` (Fable) sólo si:
- cruza más de un crate o servicio **con diseño nuevo**, o
- un error ahí es caro: seguridad, datos de producción, el actualizador, la release.

Si no, va directo al builder con tu brief.

**¿Quién escribe?**
- `builder` en Sonnet, por defecto.
- `builder` con `model: "opus"` si toca auth o sesión (`ViaToken`, tokens, refresh), las dos máquinas de estados, una migración, o concurrencia del core (turnos, carreras). También tras un segundo FALLA.

**¿Falta el criterio de aceptación?** No lo inventes: `create_request` de tipo decidir con 2–4 opciones y para esa tarea.

## 3. Worktree

Uno por tarea, desde `main` al día:

```sh
git -C ~/projects/personal/guz-studio/cac fetch origin main
git -C ~/projects/personal/guz-studio/cac worktree add \
  ~/projects/personal/guz-studio/cac/.claude/worktrees/<ref> -b <ref> origin/main
```

Tareas que tocan los mismos ficheros no van en paralelo. Tareas chicas de la misma zona van juntas en un builder. La caja de gstd pone en fila las corridas nativas: en cada tanda, mezcla trabajo nativo con backend, MCP o Tauri, que corren aquí.

## 4. Builder

`Agent` con `subagent_type: "builder"` (y `model: "opus"` si toca). El brief lleva todo, porque empieza sin contexto:
- la ref y qué se hace, en dos o tres frases;
- el criterio de aceptación;
- el plan, si lo hubo;
- la ruta del worktree y la rama;
- lo que ya sabes que muerde ahí (la trampa del CLAUDE.md, el incidente).

Lánzalo en segundo plano y sigue con otra tarea.

## 5. Reviewer

Con el builder en `listo`: `Agent` con `subagent_type: "reviewer"`, con la ref, el plan si lo hubo y el worktree con su rama. **No le pases el resumen del builder ni tu opinión.**

- `FALLA` → un builder nuevo con los hallazgos, una vez. Segundo `FALLA` → builder en Opus con todo el historial; si vuelve a fallar, `create_request` decidir con los hallazgos y para.
- `PASA` → paso 6.

## 6. Fusionar

Desde el worktree de la tarea:

```sh
git fetch origin main && git rebase origin/main
scripts/test-changed.sh --since origin/main   # tras el rebase, otra vez
git push origin HEAD:main
```

Si el push falla porque `main` se movió, rebase y repite; si el rebase choca, es del builder. Después borra el worktree y la rama (`git worktree remove`, `git branch -d`).

## 7. Cerrar

- `update_task handoff`: qué quedó, en qué commits, cómo se comprueba que salió, qué falta. Firma qué agentes y modelos lo hicieron.
- **done sólo si está publicado.** Backend: con el deploy de `backend.yml` en verde. Tauri, nativa y MCP: cuando salga su release. Mientras, en curso con «sale con …».
- Lo que mordió y morderá al siguiente: `record_incident` con la prueba que lo guarda.

## Preguntas a jose

Todo lo que necesite de una persona va por `create_request` (sale en «Por atender» y sobrevive a la sesión), nunca sólo en el chat. Y en el chat, una línea que diga que está ahí.

Nada sale a un canal, a Basecamp o a un cliente sin su sí sobre el texto exacto. Cortar una release se le pregunta siempre.

## Al terminar una tanda

Dile a jose, por tarea: qué modelo escribió, cuántas vueltas con el reviewer, qué está en main y qué espera de él.
