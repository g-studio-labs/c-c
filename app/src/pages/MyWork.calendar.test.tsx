import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";

/**
 * Lo que «Mi trabajo» le pide al servidor desde el calendario
 * (command-and-control-337).
 *
 * El calendario se sustituye por un doble que guarda lo que recibe: lo que se
 * prueba aquí es la costura, qué llega al servidor al quitar o poner una
 * fecha. Rotura que atrapa: quitar la fecha con `dueAt: null`, que el servidor
 * lee como «no tocar», y la fecha volvía a salir al recargar (#95).
 */
const { patch, calendar } = vi.hoisted(() => ({
  patch: vi.fn(),
  calendar: { props: null as null | Record<string, unknown> },
}));

vi.mock("@/lib/api", () => ({
  api: { get: vi.fn().mockResolvedValue({ success: true, data: [] }), post: vi.fn(), delete: vi.fn(), patch },
}));
vi.mock("@/components/tasks/NewTaskRow", () => ({ default: () => null }));
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
  useMyWorkStore.setState({ tasks: [task] as never, scope: null, loading: false, error: null, vista: "calendar" });
});
afterEach(() => {
  cleanup();
  patch.mockReset();
  calendar.props = null;
});

const mount = async () => {
  render(
    <MemoryRouter>
      <MyWork />
    </MemoryRouter>,
  );
  await waitFor(() => expect(calendar.props).not.toBeNull());
  return calendar.props as {
    onUnschedule: (id: string) => void;
    onSchedule: (id: string, iso: string) => void;
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
    useMyWorkStore.setState({ tasks: [task, { ...task, id: "t2", dueAt: null }] as never });
    const props = await mount();
    expect(props.undated).toHaveLength(1);
  });
});
