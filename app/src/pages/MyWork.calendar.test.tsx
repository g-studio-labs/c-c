import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, waitFor } from "@testing-library/react";

/**
 * Lo que «Mi trabajo» le pide al servidor desde el calendario
 * (command-and-control-337).
 *
 * El calendario se sustituye por un doble que guarda lo que recibe: lo que se
 * prueba aquí es la costura, qué llega al servidor al quitar o poner una
 * fecha. Rotura que atrapa: quitar la fecha con `dueAt: null`, que el servidor
 * lee como «no tocar», y la fecha volvía a salir al recargar (#95).
 */
const { patch, calendar, row, served } = vi.hoisted(() => ({
  patch: vi.fn(),
  // Lo que contesta «mi trabajo» al cargar: la página pide la lista al montar.
  served: { tasks: [] as unknown[] },
  calendar: { props: null as null | Record<string, unknown> },
  row: { props: null as null | Record<string, unknown> },
}));

vi.mock("@/lib/api", () => ({
  api: {
    get: vi.fn((url: string) =>
      Promise.resolve({ success: true, data: url.startsWith("/api/v1/tasks/?") ? served.tasks : [] }),
    ),
    post: vi.fn(),
    delete: vi.fn(),
    patch,
  },
}));
vi.mock("@/components/tasks/NewTaskRow", () => ({
  default: (props: Record<string, unknown>) => {
    row.props = props;
    return null;
  },
}));
vi.mock("@/components/ItemCalendar", () => ({
  default: (props: Record<string, unknown>) => {
    calendar.props = props;
    return null;
  },
}));

const { MemoryRouter } = await import("react-router-dom");
const { default: MyWork } = await import("@/pages/MyWork");
const { useTasksStore } = await import("@/store/tasks.store");
const { useMyWorkStore } = await import("@/store/mywork.store");
const { useOrgsStore } = await import("@/store/orgs.store");

const task = {
  id: "t1", seq: 7, title: "Algo", priority: "none", status: "open", statusName: "Open", statusKind: "open",
  listId: "l", listName: "L", spaceId: "s", spaceName: "S", updatedAt: "2026-11-01T00:00:00Z",
  subtasksDone: 0, subtasksTotal: 0, dueAt: "2026-11-12T00:00:00.000Z",
};

beforeEach(() => {
  patch.mockResolvedValue({ success: true, data: null });
  useTasksStore.setState({ tree: [] } as never);
  useOrgsStore.setState({ currentOrgId: "org-1" } as never);
  served.tasks = [task];
  useMyWorkStore.setState({ tasks: [task] as never, scope: null, loading: false, error: null, vista: "calendar" });
});
afterEach(() => {
  cleanup();
  patch.mockReset();
  calendar.props = null;
  row.props = null;
});

const mount = async () => {
  render(
    <MemoryRouter>
      <MyWork />
    </MemoryRouter>,
  );
  await waitFor(() => expect(calendar.props).not.toBeNull());
  await waitFor(() => expect(useMyWorkStore.getState().loading).toBe(false));
  return calendar.props as {
    onUnschedule: (id: string) => void;
    onSchedule: (id: string, iso: string) => void;
    onCreateForDay: (iso: string) => void;
    undated: unknown[];
  };
};

describe("el calendario de «Mi trabajo»", () => {
  it("quitar la fecha manda clearDueAt, no dueAt: null", async () => {
    const props = await mount();
    props.onUnschedule("t1");
    await waitFor(() => expect(patch).toHaveBeenCalled());
    const body = patch.mock.calls[0][1] as Record<string, unknown>;
    expect(body).toEqual({ clearDueAt: true });
  });

  it("poner fecha manda ese día a medianoche UTC", async () => {
    const props = await mount();
    props.onSchedule("t1", "2026-11-20");
    await waitFor(() => expect(patch).toHaveBeenCalled());
    expect((patch.mock.calls[0][1] as { dueAt: string }).dueAt).toBe("2026-11-20T00:00:00.000Z");
  });

  it("le da el carril: lo que no tiene fecha va en `undated`", async () => {
    // En el store y en lo que contesta el servidor: la pantalla no vuelve a
    // pedir la lista si ya la pidió con la misma pregunta.
    served.tasks = [task, { ...task, id: "t2", dueAt: null }];
    useMyWorkStore.setState({ tasks: served.tasks as never });
    await mount();
    await waitFor(() => expect((calendar.props as { undated: unknown[] }).undated).toHaveLength(1));
  });

  /**
   * Optimista (command-and-control-338): la tarjeta se mueve al soltarla, y
   * vuelve si el servidor no lo acepta. Roturas que atrapa: esperar al
   * servidor para moverla, y dejarla en el día equivocado cuando falla.
   */
  it("mover una fecha se ve al instante, y vuelve si el servidor falla", async () => {
    let fail: (e: Error) => void = () => {};
    patch.mockReturnValueOnce(new Promise((_, reject) => (fail = reject)));
    const props = await mount();
    props.onSchedule("t1", "2026-11-20");
    const due = () => useMyWorkStore.getState().tasks.find((x) => x.id === "t1")?.dueAt;
    expect(due()).toBe("2026-11-20T00:00:00.000Z");
    fail(new Error("409"));
    await waitFor(() => expect(due()).toBe("2026-11-12T00:00:00.000Z"));
  });

  it("y quitarla también: sale del día al instante", async () => {
    patch.mockReturnValueOnce(new Promise(() => {}));
    const props = await mount();
    props.onUnschedule("t1");
    expect(useMyWorkStore.getState().tasks.find((x) => x.id === "t1")?.dueAt).toBeNull();
  });

  it("«+ Nueva tarea para este día» abre la fila con ese día", async () => {
    const props = await mount();
    expect(row.props).toBeNull();
    act(() => props.onCreateForDay("2026-11-20"));
    await waitFor(() => expect(row.props?.initialDueAt).toBe("2026-11-20"));
  });
});
