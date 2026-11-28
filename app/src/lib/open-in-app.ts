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

export type OpenInAppMode = "none" | "bar";

/**
 * Qué hacer al abrir un enlace de la web. Nunca se lanza el `cac://` sin un
 * gesto: el navegador no lo permite sin él (command-and-control-574), así que
 * «siempre en la app» enseña la barra **una vez** por pestaña (`alreadyTried`)
 * en vez de intentarlo en silencio, y la persona lo abre de un clic.
 */
export function decideMode(input: {
  tauri: boolean;
  desktop: boolean;
  choice: OpenInAppChoice | null;
  alreadyTried: boolean;
}): OpenInAppMode {
  if (input.tauri || !input.desktop || input.choice === "web") return "none";
  if (input.choice === "app" && input.alreadyTried) return "none";
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

/** Aparte para que las pruebas vean adónde navega la ventana (jsdom no deja espiar `location`). */
export const windowNav = {
  to(url: string): void {
    window.location.href = url;
  },
};

/**
 * Lanza el `cac://` navegando la **ventana principal**, y tiene que llamarse
 * desde un gesto (un clic). Firefox y Chrome no lanzan un protocolo externo
 * desde un `<iframe>` oculto —que es lo que había—, así que el botón no hacía
 * nada. Con un manejador registrado el navegador pregunta y abre la app, y la
 * página se queda donde está; sin él no hace nada, y por eso la barra enseña
 * después el enlace para instalarla.
 */
export function launchApp(url: string): void {
  markTried();
  windowNav.to(url);
}

export const NATIVE_RELEASES_URL = "https://github.com/g-studio-labs/c-c/releases?q=desktop-v";
