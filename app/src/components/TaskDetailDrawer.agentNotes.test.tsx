import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

/**
 * Lo que anotan los agentes en una tarea va aparte de la conversación, con de
 * quién es el agente (command-and-control-359).
 *
 * En Portento el cliente ve el tablero y las notas de trabajo de los agentes le
 * salían a él; y aun sin cliente, enterraban lo que se dicen las personas.
 *
 * Roturas que atrapa: mezclar las notas con la conversación; esconder en las
 * notas lo que un agente publicó para el cliente, una línea de sistema o lo
 * que escribe una persona; no decir de quién es el agente; enseñar la barra
 * de pestañas cuando no hay notas; contar mal cada pestaña.
 */

const { estado } = vi.hoisted(() => ({ estado: { current: {} as Record<string, unknown> } }));

const tarjeta = (extra: Record<string, unknown>, detalle: Record<string, unknown> = {}) => ({
  task: {
    id: "t-1", seq: 203, title: "Claves de espacio", description: "La descripción de siempre",
    priority: "normal", status: "in_progress", listId: "li-1", orgId: "o-1",
    createdAt: new Date().toISOString(), updatedAt: new Date().toISOString(),
    ...extra,
  },
  attachments: [], comments: [], subtasks: [], tags: [], assignees: [],
  listName: "tasks", spaceName: "Command and control", ref: "command-and-control-203",
  status: { id: "li-1/in_progress", name: "In progress", kind: "active", color: "#888", listId: "li-1" },
  ...detalle,
});

vi.mock("@/lib/media", () => ({
  isPdfAttachment: () => false,
  linkClickAction: () => "edit",
  mediaSrc: (u?: string) => u,
  openAttachment: vi.fn(),
  attachmentPath: (u?: string) => u ?? null,
}));
vi.mock("@/store/tasks.store", () => ({
  useTasksStore: (sel: (s: Record<string, unknown>) => unknown) => sel(estado.current),
}));
vi.mock("@/store/auth.store", () => {
  const estadoAuth = { session: { id: "u-1" }, accessToken: "t" };
  return {
    useAuthStore: Object.assign((sel: (s: typeof estadoAuth) => unknown) => sel(estadoAuth), {
      getState: () => estadoAuth,
      subscribe: () => () => {},
    }),
  };
});
vi.mock("@/components/markdown/MarkdownEditor", () => ({ default: () => null }));
// El markdown se pinta como texto plano: lo que se comprueba es qué texto llega.
vi.mock("@/components/markdown/Markdown", () => ({ default: ({ children }: { children: string }) => <p>{children}</p> }));
vi.mock("@/components/UserPicker", () => ({ default: () => null }));
vi.mock("@/components/ConfirmDialog", () => ({ useConfirm: () => async () => true }));
vi.mock("@/components/PromptDialog", () => ({ usePrompt: () => async () => "" }));

const { default: TaskDetailDrawer } = await import("@/components/TaskDetailDrawer");
const { useOrgsStore } = await import("@/store/orgs.store");
beforeEach(() => useOrgsStore.setState({ currentOrgId: "o-1" }));


const comment = (id: string, body: string, extra: Record<string, unknown> = {}) => ({
  id, body, authorUserId: "u-2", authorName: "jose", kind: "user", visibility: "internal",
  attachments: [], createdAt: "2026-11-13T10:00:00Z", updatedAt: "2026-11-13T10:00:00Z", ...extra,
});
const montar = (comments: unknown[]) => {
  estado.current = {
    openTaskId: "t-1", detail: tarjeta({ projectId: "p-1", visibility: "public" }, { comments }),
    loadingDetail: false, detailError: null,
    closeTask: () => {}, updateTask: vi.fn(), deleteTask: vi.fn(), addComment: vi.fn(),
    editComment: vi.fn(), deleteComment: vi.fn(), uploadAttachment: vi.fn(),
    deleteAttachment: vi.fn(), createTag: vi.fn(), tags: [], statusesOf: async () => [],
    createSubtask: vi.fn(), openTask: vi.fn(), refreshOpenTask: vi.fn(), tree: [],
  };
  return render(<TaskDetailDrawer />);
};

beforeEach(cleanup);
afterEach(cleanup);

describe("las notas de agentes en el hilo de una tarea", () => {
  const hilo = [
    comment("c1", "Ya lo reviso hoy", { visibility: "public" }),
    comment("c2", "medí la consulta: 40 ms", { via: "mcp" }),
    comment("c3", "callejón sin salida: el índice no ayuda", { via: "mcp" }),
    comment("c4", "Listo, ya puede probarlo", { via: "mcp", visibility: "public" }),
    comment("c5", "status: pending → in_progress", { via: "mcp", kind: "system" }),
    comment("c6", "nota mía, a mano"),
  ];

  it("van en su pestaña, y la conversación se queda con lo demás", () => {
    montar(hilo);
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((x) => x.textContent)).toEqual([expect.stringMatching(/\(4\)/), expect.stringMatching(/\(2\)/)]);
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");
    // En la conversación: las personas, lo que el agente publicó para el
    // cliente y la línea de sistema. Las notas, no.
    for (const texto of ["Ya lo reviso hoy", "Listo, ya puede probarlo", "status: pending → in_progress", "nota mía, a mano"]) {
      expect(screen.getByText(texto)).toBeTruthy();
    }
    expect(screen.queryByText("medí la consulta: 40 ms")).toBeNull();

    fireEvent.click(tabs[1]);
    expect(screen.getByText("medí la consulta: 40 ms")).toBeTruthy();
    expect(screen.getByText("callejón sin salida: el índice no ayuda")).toBeTruthy();
    expect(screen.queryByText("Ya lo reviso hoy")).toBeNull();
    expect(screen.queryByText("nota mía, a mano")).toBeNull();
    // Cada nota dice de quién es el agente.
    expect(screen.getAllByText(/jose's agent|agente de jose/i)).toHaveLength(2);
  });

  it("sin notas de agentes no hay pestañas: el hilo es el de siempre", () => {
    montar([hilo[0], hilo[5]]);
    expect(screen.queryAllByRole("tab")).toHaveLength(0);
    expect(screen.getByText("Ya lo reviso hoy")).toBeTruthy();
    expect(screen.getByText("nota mía, a mano")).toBeTruthy();
    expect(screen.queryByText(/jose's agent|agente de jose/i)).toBeNull();
  });
});

// jose, 13-nov-2026: «hoy» es el de la organización, y a quien está en otra
// hora se le dice de quién es el día junto a la fecha de la tarea.
describe("la zona del vencimiento en el detalle", () => {
  it("sale entre paréntesis sólo para quien está en otra hora", () => {
    useOrgsStore.setState({ currentOrgId: "o-1", orgs: [{ id: "o-1", name: "Uno", timezone: "Asia/Kathmandu" }] as never });
    montar([]);
    expect(screen.getByTestId("due-zone").textContent).toMatch(/^\(.*Kathmandu.*\)$/);
    cleanup();
    useOrgsStore.setState({ currentOrgId: "o-1", orgs: [{ id: "o-1", name: "Uno" }] as never });
    montar([]);
    expect(screen.queryByTestId("due-zone")).toBeNull();
  });
});
