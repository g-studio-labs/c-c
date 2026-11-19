#!/usr/bin/env bash
# Corre las pruebas de lo que se tocó, no las de todo el repo.
#
# Mira qué ficheros cambiaron respecto a la rama de arriba (lo commiteado que
# falta por subir, lo que está sin commitear y lo nuevo sin añadir) y corre
# las suites que pueden romperse por eso. Una suite se corre entera o no se
# corre: dentro de un crate todo se compila junto, y una prueba de
# `workspace.rs` puede caerse por un cambio en `chat.rs`.
#
# Lo que depende de qué (las rutas de los workflows dicen lo mismo para el
# CI; si cambia una, cambia la otra):
#   core/                → core, desktop y el lado Rust de la app (tauri)
#   mcp/                 → mcp, desktop y tauri
#   desktop/             → desktop
#   app/src/locales/     → app, y además core y desktop: el core incrusta
#                          los catálogos al compilar
#   app/src-tauri/       → tauri y mcp
#   app/ (lo demás)      → app
#   backend/internal/core/domain/ → domain y core (los fixtures del contrato)
#   backend/ (lo demás)  → backend
#   .claude/hooks/       → hooks
#   scripts/test-changed.sh → app (su prueba vive allí)
# Lo demás (docs, STATUS.md) no corre nada.
#
# `--quick` acota además el escritorio a los módulos tocados (`chat.rs` →
# `chat::`), si sólo se tocó el escritorio: es la suite lenta. Es una primera
# pasada, no la puerta: antes de subir, sin `--quick`.
#
# Uso:
#   scripts/test-changed.sh [--quick] [--plan] [--base <ref>] [--files <ruta>...]
#     --plan   sólo dice qué correría, sin correrlo.
#     --base   contra qué comparar (por defecto, la rama de arriba).
#     --files  estos ficheros en vez de los de git (para la prueba).
# En esta máquina, con la carga repartida: `taskset -c 0-3 env CARGO_BUILD_JOBS=3 scripts/test-changed.sh`.
# Guardián: `app/src/lib/test-changed.test.ts`.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
quick=0
plan=0
base=""
files=()
from_git=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --quick) quick=1 ;;
    --plan) plan=1 ;;
    --base) base="$2"; shift ;;
    --files) from_git=0; shift; files=("$@"); break ;;
    *) echo "no entiendo «$1»" >&2; exit 2 ;;
  esac
  shift
done

if [[ $from_git == 1 ]]; then
  if [[ -z "$base" ]]; then
    base=$(git merge-base HEAD '@{upstream}' 2>/dev/null || git rev-parse HEAD)
  fi
  mapfile -t files < <(
    {
      git diff --name-only "$base" HEAD
      git diff --name-only HEAD
      git ls-files --others --exclude-standard
    } | sort -u
  )
fi

declare -A suites=()
desktop_modules=()
only_desktop=1
for file in "${files[@]}"; do
  [[ -z "$file" ]] && continue
  case "$file" in
    desktop/*) suites[desktop]=1 ;;
    *) ;;
  esac
  case "$file" in
    desktop/src/*.rs)
      module=$(basename "$file" .rs)
      desktop_modules+=("$module")
      ;;
    desktop/*) desktop_modules+=("*") ;;
    core/*) suites[core]=1; suites[desktop]=1; suites[tauri]=1; only_desktop=0 ;;
    mcp/*) suites[mcp]=1; suites[desktop]=1; suites[tauri]=1; only_desktop=0 ;;
    app/src/locales/*) suites[app]=1; suites[core]=1; suites[desktop]=1; only_desktop=0 ;;
    app/src-tauri/*) suites[tauri]=1; suites[mcp]=1; only_desktop=0 ;;
    app/*) suites[app]=1; only_desktop=0 ;;
    backend/internal/core/domain/*) suites[domain]=1; suites[core]=1; only_desktop=0 ;;
    backend/*) suites[backend]=1; only_desktop=0 ;;
    .claude/hooks/*) suites[hooks]=1; only_desktop=0 ;;
    scripts/test-changed.sh) suites[app]=1; only_desktop=0 ;;
    *) only_desktop=0 ;;
  esac
done

# El escritorio acotado: sólo con --quick, sólo si no se tocó nada de lo que
# depende, y sólo si todo lo tocado son módulos (`main.rs` o un recurso
# pueden mover cualquier pantalla).
desktop_filter=()
if [[ $quick == 1 && $only_desktop == 1 && ${#desktop_modules[@]} -gt 0 ]]; then
  for module in "${desktop_modules[@]}"; do
    if [[ "$module" == "*" || "$module" == "main" ]]; then
      desktop_filter=()
      break
    fi
    desktop_filter+=("$module::")
  done
fi

# Del más rápido al más lento: lo que falla antes, se ve antes.
order=(hooks domain core mcp app tauri backend desktop)
commands=()
for suite in "${order[@]}"; do
  [[ -z "${suites[$suite]:-}" ]] && continue
  case "$suite" in
    hooks) commands+=("python3 .claude/hooks/hooks_test.py") ;;
    domain) commands+=("cd backend && go test ./internal/core/domain/") ;;
    core) commands+=("cd core && cargo test --locked && cargo check --locked --features uniffi") ;;
    mcp) commands+=("cd mcp && cargo test --locked") ;;
    app) commands+=("cd app && npx tsc --noEmit && bun run test") ;;
    tauri) commands+=("cd app/src-tauri && cargo check --locked") ;;
    backend) commands+=("cd backend && go build ./... && go vet ./... && go test ./...") ;;
    desktop)
      if [[ ${#desktop_filter[@]} -gt 0 ]]; then
        commands+=("cd desktop && cargo test --locked -- ${desktop_filter[*]}")
      else
        commands+=("cd desktop && cargo test --locked")
      fi
      ;;
  esac
done

if [[ ${#commands[@]} -eq 0 ]]; then
  echo "nada que probar: lo tocado no es código" >&2
  exit 0
fi

if [[ $plan == 1 ]]; then
  printf '%s\n' "${commands[@]}"
  exit 0
fi

failed=()
for command in "${commands[@]}"; do
  echo "== $command" >&2
  if ! (cd "$root" && bash -c "$command"); then
    failed+=("$command")
  fi
done
if [[ ${#desktop_filter[@]} -gt 0 ]]; then
  echo "(escritorio acotado a ${desktop_filter[*]}: antes de subir, sin --quick)" >&2
fi
if [[ ${#failed[@]} -gt 0 ]]; then
  echo "fallaron:" >&2
  printf '  %s\n' "${failed[@]}" >&2
  exit 1
fi
echo "todo en verde" >&2
