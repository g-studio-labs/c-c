#!/usr/bin/env bash
# Corre un comando en la caja de compilación de gstd (`rust-box`) sobre una
# copia del árbol de trabajo, con lo no commiteado incluido, en vez de
# compilar en esta máquina. Ver command-and-control-464.
#
#   scripts/remote.sh -- cargo test                 # desde desktop/, core/…
#   scripts/remote.sh --out capturas -- ./pasada.sh # trae lo que deje en $CAC_OUT
#   scripts/remote.sh --sync-only
#   scripts/remote.sh --kill                        # corta lo que corra de este worktree
#   scripts/remote.sh --off                         # apaga la caja ya
#
# El comando corre en la misma subcarpeta desde la que se llama. La copia de
# allí es de usar y tirar: lo que el comando cambie en ella (las roturas de
# `/mutar`) no vuelve, y la siguiente sincronización lo pisa.
#
# Una corrida nueva corta antes lo que quede de otra del mismo worktree: dos
# sobre la misma copia se pisan las roturas, y una que se dejó colgada seguía
# horas comiendo CPU (23-nov-2026: una prueba colgada, 4 h viva).
#
# Todo entra por el SSH de siempre (`ssh gstd`) y sólo toca el namespace
# `build`.

set -euo pipefail

HOST="${CAC_BUILD_HOST:-gstd}"
NS=build
BOX=rust-box
POD="$BOX-0"

# Una sola conexión para todas las llamadas: cada `ssh` nuevo es una
# aprobación del agente de llaves y un segundo de espera.
SSH=(ssh -o ControlMaster=auto -o "ControlPath=${XDG_RUNTIME_DIR:-/tmp}/cac-remote-%C" -o ControlPersist=120 -o BatchMode=yes)

# `ssh` junta sus argumentos en una línea para el shell de allá: sin citar
# cada uno, un argumento con espacios llega partido.
remote() {
  local line
  printf -v line '%q ' "$@"
  "${SSH[@]}" "$HOST" "$line"
}
kube() { remote kubectl -n "$NS" "$@"; }

# rsync llama a su «shell remoto» con el host por delante. El host aquí es el
# pod, así que se descarta y lo demás corre dentro de la caja.
if [[ "${1:-}" == --rsh ]]; then
  shift 2
  remote kubectl -n "$NS" exec -i "$POD" -- "$@"
  exit
fi

usage() { sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

out="" sync_only=false off=false kill_only=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) out="$2"; shift 2 ;;
    --sync-only) sync_only=true; shift ;;
    --off) off=true; shift ;;
    --kill) kill_only=true; shift ;;
    -h | --help) usage ;;
    --) shift; break ;;
    *) echo "remote.sh: no entiendo «$1»" >&2; usage 2 >&2 ;;
  esac
done

if $off; then
  kube scale statefulset "$BOX" --replicas=0
  exit
fi
if ! $sync_only && ! $kill_only && [[ $# -eq 0 ]]; then
  echo "remote.sh: falta el comando, después de «--»" >&2
  exit 2
fi

root="$(git rev-parse --show-toplevel)"
here="$(git rev-parse --show-prefix)"
# Un worktree, una copia: dos worktrees de la misma rama no se pisan.
name="$(basename "$root")"
src="/work/src/$name"
remote_out="/work/out/$name"

touch_box() { kube annotate statefulset "$BOX" "cac.build/last-used=$(date +%s)" --overwrite >/dev/null; }

# Lo de este worktree es todo proceso que trabaja dentro de su copia: así se
# alcanza también lo que se salió de su grupo o quedó de una corrida cuyo
# `remote.sh` murió sin avisar (un SIGKILL aquí no deja hacer nada). Dice
# cuántos cortó.
sweep() {
  kube exec "$POD" -- sh -c "n=0
    for d in /proc/[0-9]*; do
      case \"\$(readlink \$d/cwd 2>/dev/null)\" in
        $src | $src/*) kill -KILL \${d#/proc/} 2>/dev/null && n=\$((n + 1)) ;;
      esac
    done
    echo \$n"
}
say_swept() {
  [[ "${1:-0}" -gt 0 ]] && echo "remote.sh: corté $1 procesos que seguían corriendo de $name" >&2
  return 0
}

if $kill_only; then
  [[ "$(kube get statefulset "$BOX" -o 'jsonpath={.status.readyReplicas}')" == 1 ]] || exit 0
  say_swept "$(sweep)"
  exit 0
fi

# Apuntar el uso ANTES de encender: el apagado corre cada 15 minutos y una
# caja recién encendida sin apuntar es, para él, una que no usa nadie.
touch_box
if [[ "$(kube get statefulset "$BOX" -o 'jsonpath={.status.readyReplicas}')" != 1 ]]; then
  echo "remote.sh: encendiendo la caja…" >&2
  kube scale statefulset "$BOX" --replicas=1 >/dev/null
  kube rollout status statefulset "$BOX" --timeout=300s >&2
fi

# Lo que git ignora (`target/`, `node_modules/`) no viaja, y `--delete` quita
# de allá lo que aquí ya no está: un `tests/viejo.rs` borrado aquí seguiría
# compilando allí.
ignored="$(mktemp)"
trap 'rm -f "$ignored"' EXIT
git -C "$root" ls-files --others --ignored --exclude-standard --directory | sed 's|^|/|' >"$ignored"
say_swept "$(sweep)"
kube exec "$POD" -- mkdir -p "$src" "$remote_out"
# Por contenido y sin copiar fechas: con `-a` un fichero que se rompió allí y
# se devuelve desde aquí llegaba con su fecha de aquí, más vieja que la del
# último build, y cargo daba por bueno el binario con la rotura dentro (24-nov,
# un rojo falso en core y otro en el MCP). Así sólo lo que cambió de contenido
# se reescribe, con la hora de ahora, y cargo lo recompila.
rsync -a --no-times --checksum --delete --exclude=/.git --exclude-from="$ignored" -e "$0 --rsh" "$root/" "pod:$src/"

$sync_only && exit 0

# Mientras el comando corre, se apunta el uso: una pasada de `/mutar` dura
# más de las dos horas del apagado. Si esta máquina se cae, deja de apuntarse
# y lo que quedara colgado se va con la caja.
(while sleep 300; do touch_box || true; done) >/dev/null 2>&1 &
beat=$!

# El comando va con su propio grupo de procesos y su pid apuntado: `kubectl
# exec` sin terminal no pasa las señales, y un Ctrl-C aquí dejaría `cargo`
# compilando allí.
run="run-$$-$(date +%s)"
# Sin `--`: el `sh` de la caja es dash, y su `kill` lo toma por un pid
# («Illegal number»), falla callado y deja el comando vivo allá.
stop() {
  kube exec "$POD" -- sh -c "kill -TERM -\$(cat /work/tmp/$run.pid) 2>/dev/null; rm -f /work/tmp/$run.pid" || true
  sweep >/dev/null || true
}
finish() {
  # El `sleep` del latido primero: huérfano, seguiría vivo cinco minutos.
  pkill -P "$beat" 2>/dev/null || true
  kill "$beat" 2>/dev/null || true
  rm -f "$ignored"
}
trap 'stop; finish; exit 130' INT TERM
trap finish EXIT

printf -v command '%q ' "$@"
status=0
kube exec -i "$POD" -- setsid --wait bash -c "
  echo \$\$ > /work/tmp/$run.pid
  trap 'rm -f /work/tmp/$run.pid' EXIT
  rm -rf $(printf %q "$remote_out") && mkdir -p $(printf %q "$remote_out")
  export CAC_OUT=$(printf %q "$remote_out")
  cd $(printf %q "$src/$here")
  $command
" || status=$?

if [[ -n "$out" ]]; then
  mkdir -p "$out"
  kube exec "$POD" -- tar -C "$remote_out" -cf - . | tar -C "$out" -xf -
fi
touch_box || true
exit "$status"
