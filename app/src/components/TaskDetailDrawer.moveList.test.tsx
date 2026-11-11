import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

/**
 * El campo «Lista» del detalle (command-and-control-328). Roturas que atrapa:
 * no mandar el `listId` elegido, cambiar de espacio sin preguntar (la ref
 * cambia), y enseñar el campo en una subtarea, que sólo se mueve con su padre.
 */
const get = vi.fn();
const patch = vi.fn();
vi.mock("@/lib/api", () => ({
  api: { get, post: vi.fn(), patch, delete: vi.fn(), postForm: vi.fn() },
  apiUrl: (p: string) => `http://localhost${p}`,
}));
vi.mock("@/components/markdown/MarkdownEditor", () => ({ default: () => null }));
vi.mock("@/components/markdown/Markdown", () => ({ default: () => null }));

const { default: TaskDetailDrawer } = await import("@/components/TaskDetailDrawer");
const { useOrgsStore } = await import("@/store/orgs.store");
const { useTasksStore } = await import("@/store/tasks.store");
const { PromptProvider } = await import("@/components/PromptDialog");
const { ConfirmProvider } = await import("@/components/ConfirmDialog");
const { pick } = await import("@/test-pick");

const list = (id: string, name: string) => ({ id, name, taskCount: 0, openCount: 0 });
const tree = [
  { id: "s-1", orgId: "org-1", name: "Uno", color: "", people: [], lists: [list("l-a", "A"), list("l-hermana", "Hermana")], folders: [] },
  { id: "s-2", orgId: "org-1", name: "Dos", color: "", people: [], lists: [list("l-b", "B")], folders: [] },
];

const detalle = (parentId?: string) => ({
  task: {
    id: "t-1", seq: 7, title: "Una tarea", listId: "l-a", orgId: "org-1", parentId,
    priority: "normal", statusId: "s-open", visibility: "internal", description: "",
  },
  status: { id: "s-open", name: "Open", color: "#888", kind: "open" },
  ref: "uno-7",
  spaceName: "Uno", listName: "A", tags: [], assignees: [], comments: [],
  attachments: [], subtasks: [], backlinks: [],
});

function montar(parentId?: string) {
  useTasksStore.setState({ openTaskId: "t-1", detail: detalle(parentId), loadingDetail: false, tree } as never);
  render(
    <ConfirmProvider>
      <PromptProvider>
        <TaskDetailDrawer />
      </PromptProvider>
    </ConfirmProvider>,
  );
}

beforeEach(() => {
  useOrgsStore.setState({ currentOrgId: "org-1" });
  get.mockResolvedValue({ success: true, data: [] });
  patch.mockResolvedValue({ success: true, data: detalle() });
});
afterEach(() => {
  get.mockReset();
  patch.mockReset();
  cleanup();
});

const patchedLists = () => patch.mock.calls.map((c) => (c[1] as { listId?: string }).listId).filter(Boolean);

describe("mover la tarea a otra lista", () => {
  it("dentro del espacio, manda el listId sin preguntar", async () => {
    montar();
    await pick(screen.getByRole("combobox", { name: "Move to another list" }), "Hermana");
    await waitFor(() => expect(patchedLists()).toEqual(["l-hermana"]));
  });

  it("a otro espacio, pregunta antes porque la ref cambia", async () => {
    montar();
    await pick(screen.getByRole("combobox", { name: "Move to another list" }), "B · ref changes");
    expect(await screen.findByText(/uno-7 stops being its ref/)).toBeTruthy();
    expect(patchedLists()).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: "Move" }));
    await waitFor(() => expect(patchedLists()).toEqual(["l-b"]));
  });

  it("una subtarea no lo ofrece: se mueve con su padre", () => {
    montar("t-padre");
    expect(screen.queryByRole("combobox", { name: "Move to another list" })).toBeNull();
  });
});
