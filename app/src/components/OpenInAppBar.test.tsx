import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

/**
 * La barra «Abrir en la app» de la web. Mutantes: volver al iframe oculto; lanzar el cac:// sin gesto en «siempre»; mostrarla en Tauri o en
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
const { resetTried, readChoice, writeChoice, windowNav } = await import("@/lib/open-in-app");

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
  document.querySelectorAll("iframe").forEach((f) => f.remove());
  setUA(DESKTOP);
  vi.spyOn(windowNav, "to").mockImplementation((url) => {
    clicks.push(url);
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
  });

  it("recordar «abrir en la app» se guarda", () => {
    mount();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(readChoice()).toBe("app");
  });

  it("el router ya quitó /app: una ruta que empieza por /app no pierde nada", () => {
    mount("/app/notes");
    expect(screen.queryByRole("region")).toBeNull();
    cleanup();
    mount("/notes/n1");
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(clicks).toEqual(["cac://notes/n1"]);
  });

  it("sólo sale donde la nativa abre algo: en un servidor o en /tasks a secas, no", () => {
    for (const path of ["/servers/x/backups", "/tasks", "/call/abc"]) {
      mount(path);
      expect(screen.queryByRole("region"), path).toBeNull();
      cleanup();
    }
    mount("/tasks?doc=space:s1&tab=runbook");
    expect(screen.getByRole("region")).toBeTruthy();
  });

  it("en Mi trabajo sí: un cac://my-work lleva allí", () => {
    mount("/my-work");
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(clicks).toEqual(["cac://my-work"]);
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

  it("navega la ventana principal y no crea ningún iframe ni ancla", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(clicks).toEqual(["cac://tasks?task=X"]);
    expect(document.querySelector("iframe")).toBeNull();
    expect(document.querySelector("a[href^='cac:']")).toBeNull();
  });

  it("tras pulsar, la barra sigue y ofrece instalar la app", () => {
    mount();
    expect(screen.queryByRole("link")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(screen.getByRole("region")).toBeTruthy();
    const link = screen.getByRole("link", { name: /install the app|instala la app/ });
    expect(link.getAttribute("href")).toBe("https://github.com/g-studio-labs/c-c/releases?q=desktop-v");
  });

  it("tras pulsar, dice cómo no preguntar cada vez en el diálogo del navegador", () => {
    mount();
    expect(screen.queryByText(/Always allow/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Open in the app|Abrir en la app/ }));
    expect(screen.getByText(/Always allow … cac links/)).toBeTruthy();
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

  it("«siempre en la app» no lanza nada sin gesto: enseña la barra una vez", () => {
    localStorage.setItem("cac.openInApp", "app");
    mount();
    expect(clicks).toEqual([]);
    expect(screen.getByRole("region")).toBeTruthy();
    cleanup();
    mount("/chat");
    expect(clicks).toEqual([]);
    expect(screen.queryByRole("region")).toBeNull();
  });
});
