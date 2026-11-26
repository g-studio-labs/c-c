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

**Una sesión de fondo, no un subagente** (jose, 25-nov): jose la ve en su lista, puede entrar a corregirla (`claude attach <id>`), sobrevive si el orquestador se cae y corre los hooks de cac. Desde el worktree de la tarea:

```sh
cd <worktree> && (unset CLAUDE_CONFIG_DIR; command claude --bg \
  --model sonnet --permission-mode bypassPermissions \
  --name "<n>-builder" "$(cat <brief.md>)")
```

`command` salta la función de zsh que pregunta la cuenta; cac va con la de `~/.claude`. `--model opus` si toca. El brief (en un fichero, fuera del repo) le dice que ya está aislado en su worktree, que empiece con la skill `cac-work` (`sigue <ref>`) —así reclama, lee el handoff y deja el suyo, y jose ve en el tablero cuánto falta—, que lea `.claude/agents/builder.md` y lo siga, y lleva todo lo demás, porque empieza sin contexto:
- la ref y qué se hace, en dos o tres frases;
- el criterio de aceptación;
- el plan, si lo hubo;
- lo que ya sabes que muerde ahí (la trampa del CLAUDE.md, el incidente);
- su Postgres desechable, si toca Go, en memoria y sin `fsync` (`docker run -d --rm --name cac-test-pg-<n> --tmpfs /var/lib/postgresql/data -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=cac -p 127.0.0.1:<puerto>:5432 postgres:16 -c fsync=off -c synchronous_commit=off -c full_page_writes=off`), nunca la de otro proyecto;
- que sólo corra sus propias pruebas, nunca una suite entera (paso 6).

Una sesión de fondo no avisa al terminar, y `idle` no quiere decir que haya acabado: también se queda `idle` mientras espera sus propias pruebas. Para enterarte:
- `SendMessage` a su nombre con `notify_when_idle: true`: un aviso cuando se pare, que no gasta nada; luego confirma que acabó de verdad.
- La señal de verdad: el brief le pide, como último paso, `touch <job tmp>/done-<nombre>`; un fichero que aparece es que terminó.
- Su informe se lee en la tarea (`get_task`); los logs (`claude logs`) vienen llenos de códigos de terminal.
- **En cuanto leas su informe, `claude stop <id>`**: las sesiones paradas en `idle` se quedan con 300–500 MB cada una, y con varias la máquina de jose (15 GB) se queda sin memoria y el sistema mata procesos (25-nov).

## 5. Reviewer

Con el builder en `listo`: otra sesión, igual, con `--model opus`, `--name "<n>-reviewer"` y un brief que le diga que lea `.claude/agents/reviewer.md`, con la ref, el plan si lo hubo y el worktree con su rama. **No le pases el resumen del builder ni tu opinión.**

- `FALLA` → un builder nuevo con los hallazgos, una vez. Segundo `FALLA` → builder en Opus con todo el historial; si vuelve a fallar, `create_request` decidir con los hallazgos y para.
- `PASA` → paso 6.

## 6. Juntar y compilar una sola vez

**Builders y reviewers sólo corren sus propias pruebas, nunca una suite entera** (jose, 25-nov: varias suites a la vez saturan la CPU). La puerta entera la corre el orquestador **una vez**, sobre todas las ramas con PASA juntas:

```sh
cd ~/projects/personal/guz-studio/cac
git fetch origin main
git worktree add .claude/worktrees/integrate -b integrate origin/main
cd .claude/worktrees/integrate
git cherry-pick <commits de cada rama con PASA, en orden>   # o merge de cada rama
scripts/test-changed.sh --since origin/main                 # una sola corrida
git push origin HEAD:main
```

- Junta lo que haya con PASA en ese momento; no esperes a una tarea lenta si las demás ya están.
- Si la puerta sale en rojo, `test-changed.sh` dice qué suite: averigua de qué rama viene (`git log` de los ficheros), quita esa rama del lote y devuélvesela a su builder con el fallo; el resto sale.
- Si un cherry-pick choca, esa rama vuelve a su builder para rebasarla; el resto sale.
- Go necesita su base desechable; para la puerta, una rápida: `docker run -d --rm --name cac-gate-pg --tmpfs /var/lib/postgresql/data -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=cac -p 127.0.0.1:55499:5432 postgres:16 -c fsync=off -c synchronous_commit=off -c full_page_writes=off` (en disco, `service` tarda ~10 min aquí; en CI, 72 s).
- Un worktree nuevo no tiene `app/node_modules`: enlázalo antes (`ln -s ~/projects/personal/guz-studio/cac/app/node_modules app/node_modules`) o la app sale en rojo por entorno (`tsc` no existe), no por código. Bórralo antes de subir.
- Si la puerta sale en rojo por algo de una línea (un log en castellano que atrapa `TestElServidorNoEscribeEnCastellano`), arréglalo en la rama de su tarea con un commit propio y repite la puerta con lo que haya aprobado entretanto: una corrida más, no una sesión más.
- Después borra los worktrees y ramas de lo que salió, y para la base de la puerta.

## 7. Cerrar

- `update_task handoff`: qué quedó, en qué commits, cómo se comprueba que salió, qué falta. Firma qué agentes y modelos lo hicieron.
- **done sólo si está publicado.** Backend: con el deploy de `backend.yml` en verde. Tauri, nativa y MCP: cuando salga su release. Mientras, en curso con «sale con …».
- Lo que mordió y morderá al siguiente: `record_incident` con la prueba que lo guarda.

## Preguntas a jose

Todo lo que necesite de una persona va por `create_request` (sale en «Por atender» y sobrevive a la sesión), nunca sólo en el chat. Y en el chat, una línea que diga que está ahí.

Nada sale a un canal, a Basecamp o a un cliente sin su sí sobre el texto exacto. Cortar una release se le pregunta siempre.

## Al terminar una tanda

Dile a jose, por tarea: qué modelo escribió, cuántas vueltas con el reviewer, qué está en main y qué espera de él.
