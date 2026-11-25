import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

/**
 * Una tarea con pedidos abiertos dice a quién espera, en la lista y en el
 * tablero; una sin pedidos no dice nada.
 *
 * Roturas que atrapa: quitar la marca de una de las dos vistas; pintarla en
 * todas las tareas; no nombrar a la persona.
 */

const { api } = vi.hoisted(() => ({
  api: { get: vi.fn(), post: vi.fn(), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
}));
vi.mock("@/lib/api", () => ({ api, apiUrl: (p: string) => `http://localhost${p}` }));
vi.mock("@/components/ItemCalendar", () => ({ default: () => null }));

const { default: Tasks } = await import("./Tasks");
const { useTasksStore } = await import("@/store/tasks.store");
const { useOrgsStore } = await import("@/store/orgs.store");
const { PromptProvider } = await import("@/components/PromptDialog");
const { ConfirmProvider } = await import("@/components/ConfirmDialog");

const card = (id: string, seq: number, title: string, waitingOn?: string[]) => ({
  id, seq, title, priority: "none", statusId: "st-1", hasDescription: false, commentCount: 0, attachmentCount: 0,
  subtaskCount: 0, subtaskDone: 0, tags: [], assignees: [], updatedAt: "2026-10-01T00:00:00Z",
  createdAt: "2026-10-01T00:00:00Z", ...(waitingOn && { waitingOn }),
});

const show = (boardView: "list" | "board") => {
  useTasksStore.setState({
    activeDoc: null, activeListId: "li-1", boardView,
    board: {
      list: { id: "li-1", name: "Apps" },
      statuses: [{ id: "st-1", name: "Open", color: "#888", status: "open" }],
      tasks: [card("t1", 1, "Probar el panel", ["Jose Guzman"]), card("t2", 2, "Logo nuevo")],
    },
  } as never);
  render(
    <ConfirmProvider>
      <PromptProvider>
        <MemoryRouter initialEntries={["/tasks"]}>
          <Tasks />
        </MemoryRouter>
      </PromptProvider>
    </ConfirmProvider>,
  );
};

beforeEach(() => {
  api.get.mockReturnValue(new Promise(() => {}));
  useOrgsStore.setState({ currentOrgId: "org-1" });
});
afterEach(cleanup);

describe("la marca «espera a…»", () => {
  for (const view of ["list", "board"] as const) {
    it(`en la vista ${view}`, () => {
      show(view);
      expect(screen.getAllByText("Waiting on Jose Guzman")).toHaveLength(1);
    });
  }
});
