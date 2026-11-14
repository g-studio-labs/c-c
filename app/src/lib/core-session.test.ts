import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * La sesión del escritorio, vista desde la pantalla.
 *
 * En el escritorio la sesión es del core en Rust. Aquí se guarda lo que eso
 * obliga a la pantalla: que no deje tokens en `localStorage`, que no arranque
 * con los que dejó una versión anterior, que se los entregue al core una vez,
 * y que las llamadas pasen por él.
 */

const invoke = vi.fn();
type Handler = (event: { payload: unknown }) => void;
const listeners = new Map<string, Handler>();

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: Handler) => {
    listeners.set(name, handler);
    return Promise.resolve(() => {});
  },
}));

const ana = { id: "u1", username: "ana" };

/** Lo que guardaba una versión anterior: la sesión con su par de tokens. */
const legacy = JSON.stringify({
  state: { session: ana, accessToken: "old-access", refreshToken: "old-refresh", guest: false },
  version: 0,
});

/** Un core de mentira: contesta a cada comando lo que se le diga. */
function core(answers: Record<string, unknown>) {
  invoke.mockImplementation((command: string) =>
    command in answers ? Promise.resolve(answers[command]) : Promise.resolve(null),
  );
}

/** Lo que se le pidió al core sobre la sesión, en orden. */
function sessionCommands() {
  return invoke.mock.calls.map(([command]) => command as string).filter((c) => c.startsWith("core_"));
}

function desktop() {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}

beforeEach(() => {
  vi.resetModules();
  invoke.mockReset();
  listeners.clear();
  localStorage.clear();
});

afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe("los tokens del escritorio", () => {
  it("entrar no deja ningún token en localStorage", async () => {
    desktop();
    core({});
    const { useAuthStore } = await import("@/store/auth.store");

    useAuthStore.getState().setAuth(ana, "access", "refresh");

    const everything = JSON.stringify({ ...localStorage });
    expect(everything).not.toContain("access");
    expect(everything).not.toContain("refresh");
    // El par entero se le da al core, que es quien lo guarda.
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("core_sign_in", {
        session: ana,
        accessToken: "access",
        refreshToken: "refresh",
      }),
    );
    // Y la pantalla no se queda con el de refresh ni en memoria.
    expect(useAuthStore.getState().refreshToken).toBeNull();
    expect(useAuthStore.getState().accessToken).toBe("access");
  });

  it("en la web sigue guardándolos, que ahí no hay core", async () => {
    const { useAuthStore } = await import("@/store/auth.store");

    useAuthStore.getState().setAuth(ana, "access", "refresh");

    expect(localStorage.getItem("cac-auth")).toContain("refresh");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("no arranca con el par que dejó una versión anterior", async () => {
    desktop();
    localStorage.setItem("cac-auth", legacy);

    const { useAuthStore } = await import("@/store/auth.store");

    // Puede ser un refresh ya rotado: quien manda es el core.
    expect(useAuthStore.getState().accessToken).toBeNull();
    expect(useAuthStore.getState().refreshToken).toBeNull();
  });
});

describe("el arranque de la sesión", () => {
  it("entrega al core la sesión de una versión anterior y la borra de aquí", async () => {
    desktop();
    localStorage.setItem("cac-auth", legacy);
    localStorage.setItem("access_token", "old-access");
    core({ core_adopt_session: true, core_session: { session: ana, accessToken: "old-access" } });
    const { startCoreSession } = await import("./core-session");
    const { useAuthStore } = await import("@/store/auth.store");

    await startCoreSession();

    expect(invoke).toHaveBeenCalledWith("core_adopt_session", {
      session: ana,
      accessToken: "old-access",
      refreshToken: "old-refresh",
    });
    expect(JSON.stringify({ ...localStorage })).not.toContain("old-");
    expect(useAuthStore.getState().session).toEqual(ana);
  });

  it("arranca el core antes de ofrecerle nada", async () => {
    desktop();
    localStorage.setItem("cac-auth", legacy);
    core({ core_session: { session: null, accessToken: null } });
    const { startCoreSession } = await import("./core-session");

    await startCoreSession();

    expect(sessionCommands()).toEqual(["core_init", "core_adopt_session", "core_session"]);
  });

  it("sin nada de antes no ofrece nada", async () => {
    desktop();
    core({ core_session: { session: null, accessToken: null } });
    const { startCoreSession } = await import("./core-session");

    await startCoreSession();

    expect(sessionCommands()).toEqual(["core_init", "core_session"]);
  });

  it("pone en el store lo que diga el core, no lo que hubiera guardado", async () => {
    desktop();
    localStorage.setItem("cac-auth", legacy);
    // El core ya tenía una sesión más nueva: no adopta, y contesta la suya.
    core({ core_adopt_session: false, core_session: { session: ana, accessToken: "newer-access" } });
    const { startCoreSession } = await import("./core-session");
    const { useAuthStore } = await import("@/store/auth.store");

    await startCoreSession();

    expect(useAuthStore.getState().accessToken).toBe("newer-access");
  });

  it("un fallo al ofrecerla no deja la app sin arrancar", async () => {
    desktop();
    localStorage.setItem("cac-auth", legacy);
    invoke.mockImplementation((command: string) =>
      command === "core_adopt_session"
        ? Promise.reject("storage: disk full")
        : Promise.resolve({ session: null, accessToken: null }),
    );
    const { startCoreSession } = await import("./core-session");

    await expect(startCoreSession()).resolves.toBeUndefined();
  });

  it("después de la sesión engancha «Mi trabajo» al core", async () => {
    desktop();
    core({ core_session: { session: ana, accessToken: "access" } });
    const { startCoreSession } = await import("./core-session");

    await startCoreSession();

    const commands = invoke.mock.calls.map(([command]) => command);
    expect(commands.indexOf("mywork_state")).toBeGreaterThan(commands.indexOf("core_session"));
    expect(listeners.has("core://mywork")).toBe(true);
  });

  it("lo que el core avise después llega al store", async () => {
    desktop();
    core({ core_session: { session: ana, accessToken: "access" } });
    const { startCoreSession } = await import("./core-session");
    const { useAuthStore } = await import("@/store/auth.store");
    await startCoreSession();

    // Renovó: el token de acceso es otro.
    listeners.get("core://session")!({ payload: { session: ana, accessToken: "renewed" } });
    expect(useAuthStore.getState().accessToken).toBe("renewed");

    // El servidor dio la sesión por terminada.
    listeners.get("core://session")!({ payload: { session: null, accessToken: null } });
    expect(useAuthStore.getState().session).toBeNull();
    expect(useAuthStore.getState().accessToken).toBeNull();
  });
});

describe("las llamadas del escritorio", () => {
  it("van por el core, que pone el token: la pantalla no manda cabeceras", async () => {
    desktop();
    core({ core_request: { status: 200, body: '{"success":true,"data":[1]}' } });
    const { api } = await import("./api");

    const res = await api.get<{ data: number[] }>("/api/v1/tasks/");

    expect(res.data).toEqual([1]);
    expect(invoke).toHaveBeenCalledWith("core_request", {
      method: "GET",
      path: "/api/v1/tasks/",
      body: null,
      auth: true,
    });
  });

  it("un error del servidor conserva su código y su frase", async () => {
    desktop();
    core({
      core_request: {
        status: 409,
        body: '{"success":false,"error":"inbox-other-org","message":"That list belongs to another organization"}',
      },
    });
    const { api, codigoDe } = await import("./api");

    const error = await api.get("/api/v1/tasks/").catch((e) => e);

    expect(codigoDe(error)).toBe("inbox-other-org");
  });

  it("si el core no pudo renovar, la sesión caducó: no es un fallo de red", async () => {
    desktop();
    invoke.mockRejectedValue("session-expired");
    const { api } = await import("./api");
    const { useConnectionStore } = await import("@/store/connection.store");
    const markFail = vi.spyOn(useConnectionStore.getState(), "markFail");

    await expect(api.get("/api/v1/tasks/")).rejects.toThrow("session-expired");
    expect(markFail).not.toHaveBeenCalled();
  });

  it("si no hubo respuesta, se marca la conexión", async () => {
    desktop();
    invoke.mockRejectedValue("network: could not connect");
    const { api } = await import("./api");
    const { useConnectionStore } = await import("@/store/connection.store");
    const markFail = vi.spyOn(useConnectionStore.getState(), "markFail");

    await expect(api.get("/api/v1/tasks/")).rejects.toThrow("network-error");
    expect(markFail).toHaveBeenCalled();
  });

  it("renovar a mano se le pide al core", async () => {
    desktop();
    core({ core_refresh: "renewed" });
    const { refreshAccessToken } = await import("./api");

    expect(await refreshAccessToken()).toBe("renewed");
    expect(invoke).toHaveBeenCalledWith("core_refresh");
  });
});
