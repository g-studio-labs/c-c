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
  it("son las que lee `app_link` en el core, con sus parámetros", () => {
    const rust = readFileSync(resolve(process.cwd(), "../core/src/app_link.rs"), "utf8");
    for (const arm of ['"/tasks" =>', '"/chat" =>', '"/dm" =>', '"/requests" =>', '"/feedback" =>', '"/notes" =>', '"/organization" =>', 'strip_prefix("/notes/")']) {
      expect(rust, arm).toContain(arm);
    }
    expect(nativeOpens("/tasks", "?task=X")).toBe(true);
    expect(nativeOpens("/tasks", "?doc=list:l1&page=p")).toBe(true);
    expect(nativeOpens("/chat", "?space=s1")).toBe(true);
    expect(nativeOpens("/dm", "?u=u1")).toBe(true);
    expect(nativeOpens("/notes/n1", "")).toBe(true);
    for (const p of ["/requests", "/feedback", "/organization/", "/notes"]) expect(nativeOpens(p, ""), p).toBe(true);
  });

  it("lo demás no: el core lo mandaría a «al frente»", () => {
    expect(nativeOpens("/tasks", "")).toBe(false);
    expect(nativeOpens("/tasks", "?task=")).toBe(false);
    expect(nativeOpens("/chat", "")).toBe(false);
    expect(nativeOpens("/dm", "")).toBe(false);
    for (const p of ["/servers", "/my-work", "/servers/x/backups", "/call/abc", "/overview", "/profile"]) expect(nativeOpens(p, ""), p).toBe(false);
  });
});
