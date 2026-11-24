#!/usr/bin/env bash
# Corre en la caja sólo las pruebas de lo que cambió, en vez de las suites
# enteras de la app nativa (`core/`, `desktop/`, `mcp/`, `ops/`, `voice/`).
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
#   - los catálogos de textos → las pruebas de `texts` del core.
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

if [ "$mode" = all ]; then
  plan='for c in voice core mcp ops desktop; do [ -d "$c" ] && (echo "== $c" && cd "$c" && cargo test --quiet 2>&1 | grep -E "^test .* FAILED|panicked|test result|^error") || true; done'
else
  changed="$(
    cd "$root"
    { git diff --name-only ${since:+"$since"} HEAD 2>/dev/null; git diff --name-only; git diff --name-only --cached; git ls-files --others --exclude-standard; } | sort -u
  )"
  plan="$(cd "$root" && SINCE="$since" python3 -I "$here/test_changed_plan.py" <<<"$changed")"
fi

if [ -z "$plan" ]; then
  echo "test-changed.sh: nada de la app nativa cambió"
  exit 0
fi
echo "$plan" | sed 's/^/  plan: /' >&2
[ "$mode" = plan ] && exit 0

cd "$root"
exec "$here/remote.sh" -- bash -c "$plan"
