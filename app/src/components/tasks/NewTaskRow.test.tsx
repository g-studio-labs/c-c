import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { pick } from "@/test-pick";

/**
 * The composer, and the two decisions worth pinning down.
 *
 * Visibility is only asked about in a list a client can see into — anywhere
 * else the control would be asking about a distinction that does not exist, and
 * offering it would suggest work can be hidden from somebody who was never
 * looking. And when it is asked, leaving it alone must send nothing, so the
 * server's default ("the client sees it") is never contradicted by accident.
 */

const createTaskIn = vi.fn();
vi.mock("@/store/tasks.store", () => ({
  useTasksStore: (sel: (s: unknown) => unknown) =>
    sel({
      createTaskIn,
      tree: [
        {
          id: "sp-1", name: "Interno", folders: [], color: "#888", orgId: "org-1",
          lists: [{ id: "li-libre", name: "Sin cliente", taskCount: 0 }],
        },
        {
          id: "sp-2", name: "Cliente", folders: [], color: "#888", orgId: "org-1",
          lists: [
            { id: "li-canal", name: "Con canal", taskCount: 0, projectId: "proj-1" },
            { id: "li-otra", name: "Otra", taskCount: 0 },
          ],
        },
      ],
    }),
}));
vi.mock("@/store/orgs.store", () => ({
  useOrgsStore: (sel: (s: unknown) => unknown) => sel({ currentOrgId: "org-1" }),
}));
vi.mock("@/store/people.store", () => ({
  usePeopleStore: (sel: (s: unknown) => unknown) =>
    sel({ byOrg: { "org-1": [{ id: "u-ana", username: "ana" }] }, fetchPeople: async () => {} }),
}));

const { default: NewTaskRow } = await import("@/components/tasks/NewTaskRow");

beforeEach(() => createTaskIn.mockResolvedValue(undefined));
afterEach(() => {
  createTaskIn.mockReset();
  cleanup();
});

const escribirTitulo = (t: string) =>
  fireEvent.change(screen.getByLabelText("Task title"), { target: { value: t } });
const enter = () => fireEvent.keyDown(screen.getByLabelText("Task title"), { key: "Enter" });

describe("el composer", () => {
  it("no pregunta por visibilidad en una lista sin cliente", () => {
    render(<NewTaskRow onClose={() => {}} />);
    expect(screen.queryByLabelText("Visibility")).toBeNull();
  });

  it("la pregunta cuando la lista sí tiene canal", async () => {
    render(<NewTaskRow onClose={() => {}} />);
    await pick(screen.getByLabelText("Where it goes"), "Cliente / Con canal");
    expect(await screen.findByLabelText("Visibility")).toBeTruthy();
  });

  it("sin tocar visibilidad no manda ninguna, para no contradecir al servidor", async () => {
    render(<NewTaskRow onClose={() => {}} />);
    await pick(screen.getByLabelText("Where it goes"), "Cliente / Con canal");
    escribirTitulo("Algo");
    enter();
    await waitFor(() => expect(createTaskIn).toHaveBeenCalled());
    expect(createTaskIn.mock.calls[0][0].visibility).toBeUndefined();
  });

  it("crea y se queda abierto, con el destino y la prioridad puestos", async () => {
    const onClose = vi.fn();
    render(<NewTaskRow onClose={onClose} />);
    escribirTitulo("Primera");
    enter();
    await waitFor(() => expect(createTaskIn).toHaveBeenCalledTimes(1));
    // El título se limpia; lo demás no, porque lo siguiente casi siempre va al
    // mismo sitio con la misma urgencia.
    await waitFor(() =>
      expect((screen.getByLabelText("Task title") as HTMLInputElement).value).toBe(""),
    );
    expect(screen.getByLabelText("Where it goes").textContent).toContain("Interno / Sin cliente");
    // Y no se cierra: es la decisión que el componente existe para tomar, y sin
    // esto el test la daba por buena sin mirarla.
    expect(onClose).not.toHaveBeenCalled();

    escribirTitulo("Segunda");
    enter();
    await waitFor(() => expect(createTaskIn).toHaveBeenCalledTimes(2));
  });

  /**
   * El fallo que trajo esto: la fila se queda abierta, así que la pantalla que
   * la contiene sólo volvía a preguntar al cerrarla. Con Enter la tarea existía
   * y no aparecía en ninguna lista hasta cambiar de pestaña y volver.
   */
  it("avisa tras cada Enter, no sólo al cerrar", async () => {
    const onCreated = vi.fn();
    render(<NewTaskRow onClose={() => {}} onCreated={onCreated} />);
    escribirTitulo("Primera");
    enter();
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    escribirTitulo("Segunda");
    enter();
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(2));
  });

  it("si la creación falla no avisa de nada", async () => {
    createTaskIn.mockRejectedValueOnce(new Error("no"));
    const onCreated = vi.fn();
    render(<NewTaskRow onClose={() => {}} onCreated={onCreated} />);
    escribirTitulo("Algo");
    enter();
    // Recargar aquí pintaría la lista igual que estaba y haría creer que la
    // tarea entró; el aviso de error es lo único que debe verse.
    await waitFor(() => expect(createTaskIn).toHaveBeenCalled());
    expect(onCreated).not.toHaveBeenCalled();
  });

  it("un título en blanco no llega al servidor", () => {
    render(<NewTaskRow onClose={() => {}} />);
    escribirTitulo("   ");
    enter();
    expect(createTaskIn).not.toHaveBeenCalled();
  });

  /**
   * Arrancaba siempre en la primera lista del árbol: quien creaba desde su
   * lista la mandaba a otra sin verlo, y en su lista se le preguntaba algo que
   * la pantalla ya sabía (jose, 11-nov-2026).
   */
  it("en una lista crea ahí, sin preguntar a dónde", async () => {
    render(<NewTaskRow onClose={() => {}} scope={{ kind: "list", id: "li-otra" }} />);
    expect(screen.queryByLabelText("Where it goes")).toBeNull();
    expect(screen.getByText("Cliente / Otra")).toBeTruthy();
    escribirTitulo("Aquí");
    enter();
    await waitFor(() => expect(createTaskIn).toHaveBeenCalled());
    expect(createTaskIn.mock.calls[0][0].listId).toBe("li-otra");
  });

  it("en un espacio sólo ofrece sus listas, y arranca en una de ellas", async () => {
    render(<NewTaskRow onClose={() => {}} scope={{ kind: "space", id: "sp-2" }} />);
    expect(screen.getByLabelText("Where it goes").textContent).toContain("Cliente / Con canal");
    escribirTitulo("En el espacio");
    enter();
    await waitFor(() => expect(createTaskIn).toHaveBeenCalled());
    expect(createTaskIn.mock.calls[0][0].listId).toBe("li-canal");
  });

  // Desde un día del calendario (command-and-control-338): la fecha ya puesta,
  // y como día, a medianoche UTC, que es como se guarda un vencimiento.
  it("nace con el día del calendario desde el que se pidió", async () => {
    render(<NewTaskRow onClose={() => {}} initialDueAt="2026-11-20" />);
    escribirTitulo("Ese día");
    enter();
    await waitFor(() => expect(createTaskIn).toHaveBeenCalled());
    expect(createTaskIn.mock.calls[0][0].dueAt).toBe("2026-11-20T00:00:00.000Z");
  });

  it("con la fila abierta, otro día del calendario le cambia la fecha", async () => {
    const { rerender } = render(<NewTaskRow onClose={() => {}} initialDueAt="2026-11-20" />);
    rerender(<NewTaskRow onClose={() => {}} initialDueAt="2026-11-25" />);
    escribirTitulo("Otro día");
    enter();
    await waitFor(() => expect(createTaskIn).toHaveBeenCalled());
    expect(createTaskIn.mock.calls[0][0].dueAt).toBe("2026-11-25T00:00:00.000Z");
  });
});
