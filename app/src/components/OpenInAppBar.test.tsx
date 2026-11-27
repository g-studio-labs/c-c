import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

/**
 * La barra «Abrir en la app» de la web. Mutantes: mostrarla en Tauri o en
 * móvil; no guardar la elección; repetir el intento de «siempre en la app» en
 * cada ruta; dejar el `/app` en el enlace.
 */
const platform = vi.hoisted(() => ({ isTauri: false }));
vi.mock("@/lib/platform", () => ({
  get isTauri() {
    return platform.isTauri;
  },
  isWebBuild: true,
}));

const { MemoryRouter, useNavigate } = await import("react-router-dom");
const { default: OpenInAppBar } = await import("@/components/OpenInAppBar");
const { resetTried, readChoice, writeChoice } = await import("@/lib/open-in-app");

const DESKTOP = "Mozilla/5.0 (X11; Linux x86_64) Chrome/120";
const PHONE = "Mozilla/5.0 (Linux; Android 14; Pixel 8) Chrome/120 Mobile";
const clicks: string[] = [];

function setUA(ua: string) {
  Object.defineProperty(navigator, "userAgent", { value: ua, configurable: true });
}

function Hop() {
  const nav = useNavigate();
  return <button onClick={() => nav("/chat")}>hop</button>;
}

function mount(path = "/tasks?task=X") {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <OpenInAppBar />
      <Hop />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  localStorage.clear();
  resetTried();
  platform.isTauri = false;
  clicks.length = 0;
  setUA(DESKTOP);
  const append = document.body.appendChild.bind(document.body);
  vi.spyOn(document.body, "appendChild").mockImplementation(((node: Node) => {
    if (node instanceof HTMLIFrameElement) clicks.push(node.src);
    return append(node);
  }) as typeof document.body.appendChild);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("la barra «Abrir en la app»", () => {
  it("sale en escritorio y abre el cac:// de la ruta actual", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(clicks).toEqual(["cac://tasks?task=X"]);
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("recordar «abrir en la app» se guarda", () => {
    mount();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(readChoice()).toBe("app");
  });

  it("el router ya quitó /app: una ruta que empieza por /app no pierde nada", () => {
    mount("/app/notes");
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(clicks).toEqual(["cac://app/notes"]);
  });

  it("no sale en móvil ni dentro de Tauri", () => {
    setUA(PHONE);
    mount();
    expect(screen.queryByRole("region")).toBeNull();
    cleanup();
    setUA(DESKTOP);
    platform.isTauri = true;
    mount();
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("recordar «seguir en la web» la quita para siempre", () => {
    mount();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /stay on the web|seguir en la web/ }));
    expect(readChoice()).toBe("web");
    cleanup();
    mount();
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("abrir sin recordar no guarda nada", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(readChoice()).toBeNull();
    expect(localStorage.getItem("cac.openInApp")).toBeNull();
  });

  it("con el almacenamiento bloqueado, recordar no impide abrir la app", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    mount();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(clicks).toEqual(["cac://tasks?task=X"]);
  });

  it("el intento no navega la página: ni location ni historial", () => {
    const before = [location.href, history.length];
    // jsdom avisa por la consola de «navegación no implementada» si la página
    // se mueve: es lo único que delata un `location.href = "cac://…"`.
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect([location.href, history.length]).toEqual(before);
    expect(errors).not.toHaveBeenCalled();
    const frame = document.querySelector("iframe");
    expect(frame?.style.display).toBe("none");
    expect(document.querySelector("a[href^='cac:']")).toBeNull();
  });

  it("deshacer la elección: la barra vuelve a salir", () => {
    localStorage.setItem("cac.openInApp", "web");
    mount();
    expect(screen.queryByRole("region")).toBeNull();
    cleanup();
    writeChoice(null);
    mount();
    expect(screen.getByRole("region")).toBeTruthy();
  });

  it("sin recordar, «seguir en la web» no guarda nada", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /stay on the web|seguir en la web/ }));
    expect(readChoice()).toBeNull();
  });

  it("«siempre en la app» intenta una vez y no repite al cambiar de ruta", () => {
    localStorage.setItem("cac.openInApp", "app");
    mount();
    expect(clicks).toEqual(["cac://tasks?task=X"]);
    expect(screen.queryByRole("region")).toBeNull();
    fireEvent.click(screen.getByText("hop"));
    cleanup();
    mount("/chat");
    expect(clicks).toHaveLength(1);
  });
});
