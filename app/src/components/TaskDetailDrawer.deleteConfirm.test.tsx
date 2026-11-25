import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

/**
 * Borrar una tarea se lleva sus subtareas (cascada, command-and-control-386):
 * la confirmación lo dice, resaltado y con el número, antes de preguntar. Hasta
 * entonces no decía nada de ellas.
 */

const moveTask = vi.fn(async () => {});
const deleteTask = vi.fn(async () => {});

const COLUMNAS = [
  { id: "l1/pending", name: "Open", kind: "open", status: "pending" },
  { id: "l1/in_progress", name: "In progress", kind: "active", status: "in_progress" },
  { id: "l1/resolved", name: "Done", kind: "done", status: "resolved" },
  { id: "l1/closed", name: "Closed", kind: "done", status: "closed" },
];

const CLIENTE = {
  pending: ["in_progress", "closed"],
  in_progress: ["pending", "resolved", "closed"],
  resolved: ["in_progress", "closed"],
  closed: [],
};
const INTERNA = {
  pending: ["in_progress", "resolved", "closed"],
  in_progress: ["pending", "resolved", "closed"],
  resolved: ["pending", "in_progress", "closed"],
  closed: ["pending", "in_progress", "resolved"],
};

const detalle = { current: {} as Record<string, unknown> };

vi.mock("@/lib/api", () => ({
  api: { get: vi.fn(async () => ({ success: true, data: COLUMNAS })), post: vi.fn(), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
  apiUrl: (p: string) => p,
  codigoDe: () => "",
}));
vi.mock("@/store/reports.store", () => ({
  useReportsStore: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({ transitions: CLIENTE, internalTransitions: INTERNA }),
}));
vi.mock("@/store/tasks.store", () => ({
  useTasksStore: Object.assign(
    (sel: (s: Record<string, unknown>) => unknown) =>
      sel({
        detail: detalle.current, openTaskId: "t1", moveTask,
        closeTask: vi.fn(), deleteTask, updateTask: vi.fn(), addComment: vi.fn(),
        createSubtask: vi.fn(), openTask: vi.fn(), deleteAttachment: vi.fn(),
        uploadTaskAttachment: vi.fn(), tags: [], statusesOf: vi.fn(async () => COLUMNAS),
        editComment: vi.fn(), deleteComment: vi.fn(), refreshOpenTask: vi.fn(),
        setTaskTags: vi.fn(), assign: vi.fn(),
      }),
    { getState: () => ({ statusesOf: vi.fn(async () => COLUMNAS) }) },
  ),
}));

const { default: Drawer } = await import("@/components/TaskDetailDrawer");
const { useOrgsStore } = await import("@/store/orgs.store");
// La tarea es de esta org: el cajón no pinta una tarea de otra.
beforeEach(() => useOrgsStore.setState({ currentOrgId: "o1" }));
const { ConfirmProvider } = await import("@/components/ConfirmDialog");
const { PromptProvider } = await import("@/components/PromptDialog");

/** El panel usa los diálogos imperativos, que viven en un proveedor. */
const montar = () =>
  render(
    <ConfirmProvider>
      <PromptProvider>
        <Drawer />
      </PromptProvider>
    </ConfirmProvider>,
  );

afterEach(() => {
  cleanup();
  moveTask.mockClear();
  deleteTask.mockClear();
});

const conSubtarea = (flow: string) => ({
  task: {
    id: "t1", listId: "l1", seq: 1, title: "padre", description: "",
    statusId: "l1/pending", priority: "normal", orgId: "o1", spaceId: "s1",
    status: "pending", visibility: "internal", projectId: "",
    createdAt: "", updatedAt: "",
  },
  listName: "tasks", spaceName: "cac",
  status: COLUMNAS[0], tags: [], assignees: [], comments: [], attachments: [],
  subtasks: [
    { id: "sub1", seq: 2, title: "una subtarea", statusId: "l1/pending", flow, tags: [], assignees: [] },
  ],
});


const conSubtareas = (n: number) => {
  const d = conSubtarea("internal");
  d.subtasks = Array.from({ length: n }, (_, i) => ({ ...d.subtasks[0], id: `sub${i}`, seq: i + 2 }));
  return d;
};

describe("la confirmación de borrar una tarea", () => {
  /**
   * Rotura que atrapa: una confirmación que calla que las subtareas se van (lo
   * que había), que no dice cuántas, o que sale con «1 subtareas».
   */
  it("avisa, con el número, de que se borran también sus subtareas", async () => {
    detalle.current = conSubtareas(2);
    montar();
    fireEvent.click(await screen.findByText("Delete task"));
    const aviso = await screen.findByTestId("confirm-warning");
    expect(aviso.textContent).toBe("Its 2 linked subtasks are deleted too.");
  });

  it("dice «subtask» en singular con una sola", async () => {
    detalle.current = conSubtareas(1);
    montar();
    fireEvent.click(await screen.findByText("Delete task"));
    expect((await screen.findByTestId("confirm-warning")).textContent).toBe("Its 1 linked subtask is deleted too.");
  });

  /** Rotura que atrapa: un aviso que sale siempre, que acaba por no leerse. */
  it("no avisa de nada si no tiene subtareas, y borra al confirmar", async () => {
    detalle.current = conSubtareas(0);
    montar();
    fireEvent.click(await screen.findByText("Delete task"));
    const dialogo = await screen.findByRole("dialog");
    expect(screen.queryByTestId("confirm-warning")).toBeNull();
    fireEvent.click(within(dialogo).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteTask).toHaveBeenCalledWith("t1"));
  });
});
