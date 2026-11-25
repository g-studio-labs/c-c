#!/usr/bin/env bash
# Corre sólo las pruebas de lo que cambió. La app nativa (`core/`, `desktop/`,
# `mcp/`, `ops/`, `voice/`) en la caja de gstd, por módulo; la app de Tauri, el
# backend y los hooks, aquí, por suite.
#
#   scripts/test-changed.sh               # lo no commiteado
#   scripts/test-changed.sh --since main  # y lo commiteado desde main
#   scripts/test-changed.sh --plan        # sólo dice qué correría
#   scripts/test-changed.sh --all         # las suites enteras
#
# La red entera la tiende el CI (`desktop.yml`, `mcp.yml`, `ops.yml`), que
# corre todo en cada push en unos seis minutos. Aquí se trata de no esperar
# diez minutos de caja por tocar una pantalla (jose, 24-nov-2026: «si solo
# hicimos cambios en una sección no tiene caso correr toda la suite»).
#
# Qué se corre por cada fichero cambiado:
#   - `src/a/b.rs`            → las pruebas del módulo `a::b`;
#   - un `*_tests.rs` que otro mete con `include!` → sus funciones de prueba,
#     por nombre;
#   - `tests/x.rs`            → ese fichero de pruebas;
#   - `lib.rs`, `main.rs`, `testing.rs`, `Cargo.*`, `build.rs`, `tests/fixtures`
#     → todo el crate: lo usan todas sus pruebas;
#   - un módulo del core también corre `core/tests/<módulo>.rs` y el módulo
#     del mismo nombre del escritorio, si existen;
#   - los catálogos de textos → las pruebas de `texts` del core;
#   - el dominio de Go (`backend/internal/core/domain/`) → los ficheros de
#     prueba del core, que leen los fixtures del contrato que escribe.
# Y aquí, por suite: `app/` (tipos y vitest), su lado Rust si cambió él o lo
# que enlaza (`core`, `mcp`, `ops`), el backend entero y los hooks.
# Y cada crate que depende de uno cambiado se compila con sus pruebas
# (`cargo check --tests`): un cambio del core que rompe la firma que usa el
# escritorio sale aquí, no en el CI.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(git rev-parse --show-toplevel)"
since=""
mode=run
while [ $# -gt 0 ]; do
  case "$1" in
    --since) since="$2"; shift 2 ;;
    --plan) mode=plan; shift ;;
    --all) mode=all; shift ;;
    -h|--help) sed -n '2,29p' "$0"; exit 0 ;;
    *) echo "test-changed.sh: no entiendo «$1»" >&2; exit 2 ;;
  esac
done

# Una referencia que no existe no puede dar «nada que probar» con código 0:
# los `git diff` de abajo callan su error.
if [ -n "$since" ] && ! git -C "$root" rev-parse --verify --quiet "$since^{commit}" >/dev/null; then
  echo "test-changed.sh: no existe la referencia «$since»" >&2
  exit 2
fi

if [ "$mode" = all ]; then
  # El código de salida de cargo manda (PIPESTATUS): el `grep` que recorta la
  # salida sale 1 cuando no hay nada que enseñar, y antes un `|| true` se
  # tragaba también el rojo de la suite (command-and-control-529).
  plan='fail=0; for c in voice core mcp ops desktop; do [ -d "$c" ] || continue; echo "== $c"; (cd "$c" && cargo test --quiet 2>&1 | grep -E "^test .* FAILED|panicked|test result|^error"; s=("${PIPESTATUS[@]}"); test ${s[0]} -eq 0) || { echo "test-changed.sh: FALLÓ $c" >&2; fail=1; }; done; exit "$fail"'
else
  changed="$(
    cd "$root"
    { git diff --name-only ${since:+"$since"} HEAD 2>/dev/null; git diff --name-only; git diff --name-only --cached; git ls-files --others --exclude-standard; } | sort -u
  )"
  plan="$(cd "$root" && SINCE="$since" python3 -I "$here/test_changed_plan.py" <<<"$changed")"
  local_steps="$(cd "$root" && python3 -I "$here/test_changed_plan.py" --local <<<"$changed")"
fi

[ -n "${local_steps:-}" ] && echo "$local_steps" | sed 's/^/  aquí: /' >&2
[ -n "$plan" ] && echo "$plan" | sed 's/^/  caja: /' >&2
if [ -z "$plan" ] && [ -z "${local_steps:-}" ]; then
  echo "test-changed.sh: nada que probar"
  exit 0
fi
[ "$mode" = plan ] && exit 0

cd "$root"
# Cualquier suite que falle da código distinto de 0 y se nombra al final.
failed=()
while IFS= read -r step; do
  [ -z "$step" ] && continue
  echo "== $step"
  (cd "$root" && bash -c "$step") || failed+=("$step")
done <<<"${local_steps:-}"
if [ -n "$plan" ]; then
  "$here/remote.sh" -- bash -c "$plan" || failed+=("caja (remote.sh)")
fi
if [ "${#failed[@]}" -gt 0 ]; then
  echo "test-changed.sh: FALLÓ:" >&2
  printf '  - %s\n' "${failed[@]}" >&2
  exit 1
fi
