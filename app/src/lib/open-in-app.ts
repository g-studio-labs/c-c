import { isTauri } from "@/lib/platform";

export type OpenInAppChoice = "app" | "web";

const CHOICE_KEY = "cac.openInApp";
const TRIED_KEY = "cac.openInApp.tried";

/** Lo mismo que mide `use-mobile`: no es una pantalla estrecha, es un teléfono. */
export function isDesktopBrowser(ua: string, maxTouchPoints: number): boolean {
  if (/Android|iPhone|iPad|iPod|Mobile/i.test(ua)) return false;
  // iPadOS se presenta como un Mac, pero un Mac no tiene pantalla táctil.
  if (/Macintosh/.test(ua) && maxTouchPoints > 1) return false;
  return /Windows|Macintosh|Linux|X11|CrOS/.test(ua);
}

/**
 * El enlace `cac://` de una ruta de la web: la misma ruta y consulta, sin la
 * base (`/app`). Es el contrato con la app nativa (command-and-control-561):
 * `/app/tasks?task=X` → `cac://tasks?task=X`. El `#` no viaja.
 */
export function toAppUrl(pathname: string, search: string, base = "/app"): string {
  const root = base.replace(/\/$/, "");
  let path = pathname;
  if (root && (path === root || path.startsWith(`${root}/`))) path = path.slice(root.length);
  return `cac://${path.replace(/^\/+/, "")}${search}`;
}

export function readChoice(): OpenInAppChoice | null {
  try {
    const v = localStorage.getItem(CHOICE_KEY);
    return v === "app" || v === "web" ? v : null;
  } catch {
    return null;
  }
}

export function writeChoice(choice: OpenInAppChoice | null): void {
  try {
    if (choice) localStorage.setItem(CHOICE_KEY, choice);
    else localStorage.removeItem(CHOICE_KEY);
  } catch {
    /* sin almacenamiento la elección dura lo que dure la página */
  }
}

export type OpenInAppMode = "none" | "bar" | "auto";

/**
 * Qué hacer al abrir un enlace de la web. «Siempre en la app» intenta el
 * `cac://` **una vez** por pestaña (`alreadyTried`): si la app no está el
 * navegador no hace nada, y repetirlo en cada ruta sería un bucle de intentos.
 */
export function decideMode(input: {
  tauri: boolean;
  desktop: boolean;
  choice: OpenInAppChoice | null;
  alreadyTried: boolean;
}): OpenInAppMode {
  if (input.tauri || !input.desktop || input.choice === "web") return "none";
  if (input.choice === "app") return input.alreadyTried ? "none" : "auto";
  return "bar";
}

let triedInMemory = false;

export function alreadyTried(): boolean {
  if (triedInMemory) return true;
  try {
    return sessionStorage.getItem(TRIED_KEY) === "1";
  } catch {
    return false;
  }
}

export function markTried(): void {
  triedInMemory = true;
  try {
    sessionStorage.setItem(TRIED_KEY, "1");
  } catch {
    /* basta con la marca en memoria */
  }
}

/** Para las pruebas: olvida el intento de esta pestaña. */
export function resetTried(): void {
  triedInMemory = false;
  try {
    sessionStorage.removeItem(TRIED_KEY);
  } catch {
    /* nada que olvidar */
  }
}

export function currentMode(): OpenInAppMode {
  return decideMode({
    tauri: isTauri,
    desktop: typeof navigator !== "undefined" && isDesktopBrowser(navigator.userAgent, navigator.maxTouchPoints ?? 0),
    choice: readChoice(),
    alreadyTried: alreadyTried(),
  });
}

/**
 * Lanza el `cac://` con un enlace invisible y no con `location.href`: sin
 * manejador registrado el navegador no hace nada y la página sigue donde está,
 * en vez de irse a una pantalla en blanco.
 */
export function launchApp(url: string): void {
  markTried();
  const a = document.createElement("a");
  a.href = url;
  a.rel = "noopener";
  a.style.display = "none";
  document.body.appendChild(a);
  a.click();
  a.remove();
}
