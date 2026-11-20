import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * `scripts/test-changed.sh`: qué suites corre según lo que se tocó. Vive en
 * la suite de la app para que corra en el CI, como `next-version.test.ts`.
 *
 * Lo que cuesta un fallo aquí es una prueba roja que no se corre: por eso
 * cada caso dice qué dependencia guarda.
 *
 * Mutantes: el core sin el escritorio (que lo usa) o sin el lado Rust de la
 * app; ops sin la app de Tauri, que lo envuelve, o el mcp sin ops, que lo usa; un catálogo que no recompila el core, que lo incrusta; acotar el
 * escritorio sin `--quick`, o acotarlo cuando se tocó también el core;
 * correr algo por tocar un `.md`.
 */
const script = join(process.cwd(), "..", "scripts", "test-changed.sh");

function plan(...files: string[]): string[] {
  return execFileSync("bash", [script, "--plan", "--files", ...files], { stdio: ["ignore", "pipe", "ignore"] })
    .toString()
    .trim()
    .split("\n")
    .filter(Boolean);
}

function quick(...files: string[]): string[] {
  return execFileSync("bash", [script, "--quick", "--plan", "--files", ...files], { stdio: ["ignore", "pipe", "ignore"] })
    .toString()
    .trim()
    .split("\n")
    .filter(Boolean);
}

describe("test-changed.sh", () => {
  it("el core arrastra a quien lo usa", () => {
    expect(plan("core/src/notice.rs")).toEqual([
      "cd core && cargo test --locked && cargo check --locked --features uniffi",
      "cd app/src-tauri && cargo check --locked",
      "cd desktop && cargo test --locked",
    ]);
  });

  it("un catálogo recompila el core, que lo incrusta", () => {
    expect(plan("app/src/locales/es/notifications.json")).toEqual([
      "cd core && cargo test --locked && cargo check --locked --features uniffi",
      "cd app && npx tsc --noEmit && bun run test",
      "cd desktop && cargo test --locked",
    ]);
  });

  it("el escritorio solo, entero salvo con --quick", () => {
    expect(plan("desktop/src/chat.rs")).toEqual(["cd desktop && cargo test --locked"]);
    expect(quick("desktop/src/chat.rs", "desktop/src/composer.rs")).toEqual(["cd desktop && cargo test --locked -- chat:: composer::"]);
  });

  it("--quick no acota si se tocó algo de lo que depende, ni main.rs ni un recurso", () => {
    expect(quick("desktop/src/chat.rs", "core/src/chat.rs")).toContain("cd desktop && cargo test --locked");
    expect(quick("desktop/src/main.rs")).toEqual(["cd desktop && cargo test --locked"]);
    expect(quick("desktop/src/chat.rs", "desktop/assets/emoji/2764.png")).toEqual(["cd desktop && cargo test --locked"]);
  });

  it("el mcp arrastra a ops, al escritorio y a la app, que lo enlazan", () => {
    expect(plan("mcp/src/main.rs")).toEqual([
      "cd mcp && cargo test --locked",
      "cd ops && cargo test --locked",
      "cd app/src-tauri && cargo check --locked",
      "cd desktop && cargo test --locked",
    ]);
  });

  it("ops arrastra a la app de Tauri, que lo envuelve", () => {
    expect(plan("ops/src/ssh.rs")).toEqual(["cd ops && cargo test --locked", "cd app/src-tauri && cargo check --locked"]);
    expect(quick("ops/src/ssh.rs", "desktop/src/chat.rs")).toContain("cd desktop && cargo test --locked");
  });

  it("el motor de voz arrastra al escritorio, que lo enlaza", () => {
    expect(plan("voice/src/engine.rs")).toEqual(["cd voice && cargo test --locked", "cd desktop && cargo test --locked"]);
    expect(quick("voice/src/engine.rs", "desktop/src/voice.rs")).toContain("cd desktop && cargo test --locked");
  });

  it("el dominio de Go escribe los fixtures del core", () => {
    expect(plan("backend/internal/core/domain/core_contract_test.go")).toEqual([
      "cd backend && go test ./internal/core/domain/",
      "cd core && cargo test --locked && cargo check --locked --features uniffi",
    ]);
    expect(plan("backend/internal/core/service/chat.go")).toEqual(["cd backend && go build ./... && go vet ./... && go test ./..."]);
  });

  it("la app y los hooks, solos", () => {
    expect(plan("app/src/components/Foo.tsx")).toEqual(["cd app && npx tsc --noEmit && bun run test"]);
    expect(plan(".claude/hooks/cac_hooks.py")).toEqual(["python3 .claude/hooks/hooks_test.py"]);
  });

  it("lo que no es código no corre nada", () => {
    expect(plan("STATUS.md", "docs/grabacion.md")).toEqual([]);
  });
});
