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
const { resetTried, readChoice } = await import("@/lib/open-in-app");

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
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
    clicks.push(this.href);
  });
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
