import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

/**
 * Cada tipo de pedido se contesta con sus botones, y sólo quien puede.
 *
 * Roturas que atrapa: un botón que manda otra cosa (aprobar con el texto
 * vacío, una opción que no es la pulsada); «No funciona» que sale sin nota;
 * rechazar sin motivo; botones de contestar en el pedido de otra persona
 * (en «Equipo»); el ID que no se copia o se copia otro; la recomendada que no
 * se distingue.
 */

const post = vi.fn(async (_path: string, body: unknown) => ({ success: true, data: { ...base("approve_message"), status: "answered", answer: body } }));
vi.mock("@/lib/api", () => ({
  api: { get: vi.fn(), post: (p: string, b: unknown) => post(p, b), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
  apiUrl: (p: string) => `http://localhost${p}`,
}));
const copied: string[] = [];
vi.mock("@/lib/clipboard", () => ({ copyText: async (t: string) => void copied.push(t) }));

const { MemoryRouter } = await import("react-router-dom");
const { default: RequestCard } = await import("@/components/requests/RequestCard");
import type { AgentRequest, RequestKind } from "@/lib/requests";

function base(kind: RequestKind): AgentRequest {
  return {
    id: "u1", code: "req-7k2m9x", orgId: "o1", kind, summary: "Mira esto", version: 1, status: "open",
    task: { id: "t1", ref: "cac-10", title: "Panel", priority: "high", status: "in_progress", link: "/tasks?task=t1" },
    body: {}, requestedOf: { id: "me", name: "Jose" }, requestedBy: { id: "me", name: "Jose" }, via: "mcp",
    createdAt: new Date().toISOString(), updatedAt: new Date().toISOString(), canAnswer: true, link: "/requests?open=req-7k2m9x",
  };
}

const mount = (r: AgentRequest) => render(<MemoryRouter><RequestCard request={r} /></MemoryRouter>);
const sent = () => post.mock.calls.map(([p, b]) => [p, b]);

beforeEach(() => {
  post.mockClear();
  copied.length = 0;
});
afterEach(cleanup);

describe("contestar un pedido", () => {
  it("aprobar manda el borrador tal cual; editar, el texto final", async () => {
    mount({ ...base("approve_message"), body: { draft: "Hola equipo", destination: "Basecamp" } });
    expect(screen.getByText("Hola equipo")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(sent()).toEqual([["/api/v1/requests/req-7k2m9x/answer", { approved: true }]]));

    cleanup();
    post.mockClear();
    mount({ ...base("approve_message"), body: { draft: "Hola equipo", destination: "Basecamp" } });
    fireEvent.click(screen.getByRole("button", { name: "Edit and approve" }));
    fireEvent.change(screen.getByLabelText("Final text"), { target: { value: "Hola a todos" } });
    fireEvent.click(screen.getByRole("button", { name: "Approve this text" }));
    await waitFor(() => expect(sent()).toEqual([["/api/v1/requests/req-7k2m9x/answer", { approved: true, finalText: "Hola a todos" }]]));
  });

  it("rechazar pide motivo antes de mandar", async () => {
    mount({ ...base("approve_message"), body: { draft: "x", destination: "y" } });
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    const send = screen.getByRole("button", { name: "Reject with this reason" });
    expect((send as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Why it's rejected"), { target: { value: "tono" } });
    fireEvent.click(send);
    await waitFor(() => expect(sent()).toEqual([["/api/v1/requests/req-7k2m9x/answer", { approved: false, reason: "tono" }]]));
  });

  it("«No funciona» no sale sin decir qué se vio", async () => {
    mount({ ...base("test"), body: { steps: ["abrir el panel"], expected: "07:30" } });
    fireEvent.click(screen.getByRole("button", { name: "Doesn't work" }));
    expect(post).not.toHaveBeenCalled();
    expect(screen.getByLabelText("What you saw (required)")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("What you saw (required)"), { target: { value: "sale 07:00" } });
    fireEvent.click(screen.getByRole("button", { name: "Doesn't work" }));
    await waitFor(() => expect(sent()).toEqual([["/api/v1/requests/req-7k2m9x/answer", { works: false, note: "sale 07:00" }]]));
  });

  it("decidir: un botón por opción, la recomendada marcada, y manda la pulsada", async () => {
    mount({ ...base("decide"), body: { options: [
      { label: "Darles SendMessage", consequence: "hablan entre ellos", recommended: true },
      { label: "Dejarlo", consequence: "nada cambia" },
    ] } });
    expect(screen.getByText("Recommended")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Dejarlo/ }));
    await waitFor(() => expect(sent()).toEqual([["/api/v1/requests/req-7k2m9x/answer", { option: "Dejarlo" }]]));
  });

  it("ejecutar: copia el comando y manda lo pegado", async () => {
    mount({ ...base("run"), body: { command: "git tag v1.7.0", where: "cac", expected: "la etiqueta" } });
    fireEvent.click(screen.getByRole("button", { name: "Copy the command" }));
    await waitFor(() => expect(copied).toEqual(["git tag v1.7.0"]));
    fireEvent.change(screen.getByLabelText("Output (paste what came out)"), { target: { value: "ok" } });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(sent()).toEqual([["/api/v1/requests/req-7k2m9x/answer", { output: "ok" }]]));
  });

  it("el ID se copia con un clic", async () => {
    mount({ ...base("obtain"), body: { what: "la llave", from: "Bea" } });
    fireEvent.click(screen.getByRole("button", { name: "Copy the ID req-7k2m9x" }));
    await waitFor(() => expect(copied).toEqual(["req-7k2m9x"]));
  });

  it("el pedido de otra persona se lee sin botones", () => {
    mount({ ...base("obtain"), body: { what: "la llave", from: "Bea" }, canAnswer: false, requestedOf: { id: "bea", name: "Bea" } });
    expect(screen.queryByRole("button", { name: "Got it" })).toBeNull();
    expect(screen.getByText("Asked of Bea: only they can answer it.")).toBeTruthy();
  });
});
