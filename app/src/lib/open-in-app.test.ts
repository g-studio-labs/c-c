import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { decideMode, isDesktopBrowser, readChoice, resetTried, alreadyTried, markTried, nativeOpens, toAppUrl, writeChoice } from "./open-in-app";

const LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120 Safari/537.36";
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15";
const WIN = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120 Safari/537.36";
const ANDROID = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/120 Mobile Safari/537.36";
const IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148";

describe("el enlace cac:// de una ruta de la web", () => {
  it("es la misma ruta y consulta sin /app", () => {
    expect(toAppUrl("/app/tasks", "?task=X")).toBe("cac://tasks?task=X");
    expect(toAppUrl("/app/chat/general", "")).toBe("cac://chat/general");
    expect(toAppUrl("/app/tasks", "?a=1&b=2")).toBe("cac://tasks?a=1&b=2");
    expect(toAppUrl("/app/", "")).toBe("cac://");
    expect(toAppUrl("/app", "")).toBe("cac://");
  });
  it("sólo quita /app al principio, no uno que esté dentro", () => {
    expect(toAppUrl("/app/docs/app/x", "")).toBe("cac://docs/app/x");
    expect(toAppUrl("/application/x", "")).toBe("cac://application/x");
    expect(toAppUrl("/chat/app/x", "")).toBe("cac://chat/app/x");
  });
  it("con la ruta que ya dio el router (sin base) no quita nada", () => {
    expect(toAppUrl("/tasks", "?task=X", "")).toBe("cac://tasks?task=X");
    expect(toAppUrl("/app/x", "", "")).toBe("cac://app/x");
  });
});

describe("dónde se ofrece", () => {
  it("escritorio sí: Linux, macOS y Windows", () => {
    for (const ua of [LINUX, MAC, WIN]) expect(isDesktopBrowser(ua, 0)).toBe(true);
  });
  it("móvil y tableta no", () => {
    expect(isDesktopBrowser(ANDROID, 5)).toBe(false);
    expect(isDesktopBrowser(IPHONE, 5)).toBe(false);
    expect(isDesktopBrowser(MAC, 5)).toBe(false); // iPadOS
  });
  const base = { tauri: false, desktop: true, choice: null, alreadyTried: false } as const;
  it("sin elección, barra; en móvil o en Tauri, nada", () => {
    expect(decideMode(base)).toBe("bar");
    expect(decideMode({ ...base, desktop: false })).toBe("none");
    expect(decideMode({ ...base, tauri: true })).toBe("none");
    expect(decideMode({ ...base, tauri: true, choice: "app" })).toBe("none");
  });
});

describe("recordar la elección", () => {
  beforeEach(() => {
    localStorage.clear();
    resetTried();
  });
  const base = { tauri: false, desktop: true, choice: null, alreadyTried: false } as const;
  it("«siempre en la app» se guarda y enseña la barra, sin lanzar nada", () => {
    writeChoice("app");
    expect(readChoice()).toBe("app");
    expect(decideMode({ ...base, choice: readChoice() })).toBe("bar");
  });
  it("«seguir en la web» se guarda y no vuelve a salir nada", () => {
    writeChoice("web");
    expect(readChoice()).toBe("web");
    expect(decideMode({ ...base, choice: readChoice() })).toBe("none");
  });
  it("un valor raro no cuenta como elección", () => {
    localStorage.setItem("cac.openInApp", "quizá");
    expect(readChoice()).toBeNull();
  });
  it("sin almacenamiento no revienta", () => {
    const orig = Storage.prototype.getItem;
    Storage.prototype.getItem = () => {
      throw new Error("blocked");
    };
    try {
      expect(readChoice()).toBeNull();
    } finally {
      Storage.prototype.getItem = orig;
    }
  });
});

describe("si la app no está, no hay bucle", () => {
  beforeEach(() => resetTried());
  it("el intento se hace una vez y el siguiente enlace se queda en la web", () => {
    const base = { tauri: false, desktop: true, choice: "app", alreadyTried: false } as const;
    expect(decideMode({ ...base, alreadyTried: alreadyTried() })).toBe("bar");
    markTried();
    expect(decideMode({ ...base, alreadyTried: alreadyTried() })).toBe("none");
  });
});

describe("las rutas que la nativa abre", () => {
  // La tabla es del core (`app_link` y `launch_landing` la cumplen en
  // `core/tests/app_link.rs`): si la web y la nativa se separan, falla una de
  // las dos pruebas.
  const table = JSON.parse(readFileSync(resolve(process.cwd(), "../core/tests/fixtures/native_routes.json"), "utf8")) as {
    routes: { route: string; screen: string; landing?: string }[];
  };

  it("la web ofrece abrir donde la nativa tiene pantalla, y sólo ahí", () => {
    expect(table.routes.length).toBeGreaterThan(20);
    for (const { route, screen, landing } of table.routes) {
      const url = new URL(route, "http://x");
      const opens = screen !== "Browser" || landing === "my_work";
      expect(nativeOpens(url.pathname, url.search), route).toBe(opens);
    }
  });

  it("una barra final no cambia la respuesta", () => {
    expect(nativeOpens("/organization/", "")).toBe(true);
    expect(nativeOpens("/servers/", "")).toBe(false);
  });
});
