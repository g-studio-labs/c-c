import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * «Mi trabajo» en el escritorio: el store es un espejo del core en Rust.
 *
 * Las reglas —qué pregunta cada lente, cuándo se recarga— se prueban en el
 * core (`core/tests/mywork.rs`). Aquí se guarda el espejo: que cada acción le
 * llegue al core, en orden, que lo que el core cuenta acabe en pantalla, y que
 * el store no vuelva a decidir por su cuenta.
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

const task = (id: string, dueAt: string | null = null) => ({
  id, seq: 1, title: id, priority: "none", dueAt, status: "open", statusName: "Open", statusKind: "open",
  listId: "l1", listName: "L", spaceId: "s1", spaceName: "S", updatedAt: "", subtasksDone: 0, subtasksTotal: 0,
});

const empty = {
  lens: "assigned", scope: null, orgId: null, includeClosed: false,
  tasks: [], loading: false, error: null, loadedKey: null,
};

/** Un core de mentira: `mywork_state` contesta `state`; lo demás, nada. */
function core(state: Record<string, unknown> = empty) {
  invoke.mockImplementation((command: string) => Promise.resolve(command === "mywork_state" ? state : null));
}

const commands = () => invoke.mock.calls.map(([command]) => command as string);

async function desktopStore() {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  return import("@/store/mywork.store");
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

describe("«Mi trabajo» en el escritorio", () => {
  it("cargar se le pide al core: el store no llama a la API", async () => {
    core({ ...empty, tasks: [task("a")], loadedKey: "org1|Assigned|false" });
    const { useMyWorkStore } = await desktopStore();

    await useMyWorkStore.getState().load("org1");

    expect(invoke).toHaveBeenCalledWith("mywork_load", { orgId: "org1" });
    expect(commands()).not.toContain("core_request");
    // Quien espera la carga ya ve lo cargado.
    expect(useMyWorkStore.getState().tasks.map((t) => t.id)).toEqual(["a"]);
  });

  it("volver a la pantalla le pregunta al core si hace falta cargar", async () => {
    core();
    const { useMyWorkStore } = await desktopStore();

    await useMyWorkStore.getState().ensureLoaded("org1");

    expect(invoke).toHaveBeenCalledWith("mywork_ensure_loaded", { orgId: "org1" });
    expect(commands()).not.toContain("mywork_load");
  });

  it("cambiar la lente y cargar llegan al core en ese orden", async () => {
    // El core tarda en cada orden: si no fueran en cola, la carga adelantaría.
    invoke.mockImplementation(
      (command: string) =>
        new Promise((resolve) =>
          setTimeout(() => resolve(command === "mywork_state" ? empty : null), command === "mywork_set_lens" ? 30 : 0),
        ),
    );
    const { useMyWorkStore } = await desktopStore();
    const finished: string[] = [];
    const original = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string) =>
      (original(command) as Promise<unknown>).then((value) => {
        finished.push(command);
        return value;
      }),
    );

    useMyWorkStore.getState().setLens("watching");
    await useMyWorkStore.getState().load("org1");

    expect(finished.slice(0, 2)).toEqual(["mywork_set_lens", "mywork_load"]);
    // Y la lente se ve cambiada sin esperar al core.
    expect(invoke).toHaveBeenCalledWith("mywork_set_lens", { lens: "watching" });
  });

  it("el ámbito va sellado con la organización desde la que se eligió", async () => {
    core();
    const { useMyWorkStore } = await desktopStore();
    const { useOrgsStore } = await import("@/store/orgs.store");
    useOrgsStore.setState({ currentOrgId: "org1" });

    useMyWorkStore.getState().setScope({ kind: "list", id: "l1", name: "Lista" });

    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("mywork_set_scope", {
        scope: { kind: "list", id: "l1", name: "Lista", orgId: "org1" },
      }),
    );
  });

  it("un evento de tarea se le pasa al core, que decide si recarga", async () => {
    core();
    const { useMyWorkStore } = await desktopStore();

    useMyWorkStore.getState().refrescarSiEsNuestra("t9", true);

    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("mywork_task_event", { taskId: "t9", new: true }));
    // El store no recarga por su cuenta.
    await new Promise((r) => setTimeout(r, 500));
    expect(commands()).not.toContain("mywork_load");
  });

  it("mover un vencimiento se ve ya, devuelve el anterior y se le dice al core", async () => {
    core();
    const { useMyWorkStore } = await desktopStore();
    useMyWorkStore.setState({ tasks: [task("a", "2026-09-30T00:00:00Z")] as never });

    const previous = useMyWorkStore.getState().setDueLocal("a", "2026-10-02T00:00:00Z");

    expect(previous).toBe("2026-09-30T00:00:00Z");
    expect(useMyWorkStore.getState().tasks[0].dueAt).toBe("2026-10-02T00:00:00Z");
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("mywork_set_due_local", { taskId: "a", dueAt: "2026-10-02T00:00:00Z" }),
    );
  });

  it("quitar una tarea se ve ya y se le dice al core", async () => {
    core();
    const { useMyWorkStore } = await desktopStore();
    useMyWorkStore.setState({ tasks: [task("a"), task("b")] as never });

    useMyWorkStore.getState().olvidar("a");

    expect(useMyWorkStore.getState().tasks.map((t) => t.id)).toEqual(["b"]);
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("mywork_forget", { taskId: "a" }));
  });

  it("seguir se le pide al core, y su rechazo llega a quien lo pidió", async () => {
    invoke.mockImplementation((command: string) =>
      command === "mywork_set_watching" ? Promise.reject("Task not found") : Promise.resolve(null),
    );
    const { useMyWorkStore } = await desktopStore();

    await expect(useMyWorkStore.getState().setWatching("x", true)).rejects.toBe("Task not found");
    expect(invoke).toHaveBeenCalledWith("mywork_set_watching", { taskId: "x", on: true });
    // Y un fallo no atasca lo siguiente.
    core();
    await useMyWorkStore.getState().load(null);
    expect(commands()).toContain("mywork_load");
  });

  it("lo que el core cuenta llega a la pantalla", async () => {
    core();
    const { useMyWorkStore, connectMyWorkToCore } = await desktopStore();
    await connectMyWorkToCore();

    listeners.get("core://mywork")!({ payload: { ...empty, lens: "created", tasks: [task("z")], loading: true } });

    const state = useMyWorkStore.getState();
    expect([state.lens, state.loading, state.tasks.map((t) => t.id)]).toEqual(["created", true, ["z"]]);
  });

  it("la lente que guardaba una versión anterior pasa al core, una vez", async () => {
    localStorage.setItem(
      "cac-mywork",
      JSON.stringify({ state: { lens: "watching", includeClosed: true, vista: "board" }, version: 0 }),
    );
    core({ ...empty, lens: "watching", includeClosed: true });
    const { useMyWorkStore, connectMyWorkToCore } = await desktopStore();

    await connectMyWorkToCore();

    expect(invoke).toHaveBeenCalledWith("mywork_set_lens", { lens: "watching" });
    expect(invoke).toHaveBeenCalledWith("mywork_set_include_closed", { on: true });
    // Aquí sólo queda guardada la vista: lo demás es del core.
    expect(Object.keys(JSON.parse(localStorage.getItem("cac-mywork")!).state)).toEqual(["vista"]);
    expect(useMyWorkStore.getState().vista).toBe("board");
  });

  it("sin nada de antes no le cambia la lente al core", async () => {
    core({ ...empty, lens: "created" });
    const { useMyWorkStore, connectMyWorkToCore } = await desktopStore();

    await connectMyWorkToCore();

    expect(commands()).toEqual(["mywork_state"]);
    expect(useMyWorkStore.getState().lens).toBe("created");
  });
});
