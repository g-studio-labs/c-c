import { afterEach, beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

/**
 * «Por atender»: lo tuyo por defecto, lo del equipo con el filtro, lo resuelto
 * plegado abajo, y `?open=` resalta el pedido que pidió abrir un agente.
 *
 * Roturas que atrapa: pedir los de todos por defecto; que «Equipo» no cambie
 * lo que se pide; mezclar resueltos con abiertos o enseñarlos desplegados;
 * no resaltar el pedido del enlace (o abrir «Resueltos» sin motivo).
 *
 * Y lo que cambia mientras se lee no mueve la pantalla (25-nov): contestar el
 * pedido del enlace desplegaba «Resueltos» y bajaba hasta él, y cada recarga
 * volvía a desplazar; un pedido que entra por encima empujaba lo que se leía.
 */

import type { AgentRequest } from "@/lib/requests";

const get = vi.fn();
vi.mock("@/lib/api", () => ({
  api: { get: (p: string) => get(p), post: vi.fn(), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
  apiUrl: (p: string) => `http://localhost${p}`,
}));

const { MemoryRouter } = await import("react-router-dom");
const { default: AgentRequests } = await import("@/pages/AgentRequests");
const { useAgentRequestsStore } = await import("@/store/agent-requests.store");
const { useOrgsStore } = await import("@/store/orgs.store");
// El estado con el que arranca la app: el filtro por defecto es parte de lo que se prueba.
const initial = useAgentRequestsStore.getState();

const req = (code: string, status: "open" | "answered", who = "me"): AgentRequest => ({
  id: code, code, orgId: "o1", kind: "obtain", summary: `Pedido ${code}`, version: 1, status,
  task: { id: "t1", ref: "cac-10", title: "Panel", priority: "high", status: "in_progress", link: "/tasks?task=t1" },
  body: { what: "x", from: "y" }, requestedOf: { id: who, name: who }, requestedBy: { id: "me", name: "me" },
  createdAt: new Date().toISOString(), updatedAt: new Date().toISOString(), canAnswer: status === "open" && who === "me",
  link: `/requests?open=${code}`, ...(status === "answered" && { answer: {}, answeredBy: { id: "me", name: "me" } }),
}) as AgentRequest;

beforeEach(() => {
  get.mockReset();
  get.mockImplementation(async (p: string) => {
    const team = p.includes("scope=team");
    if (p.includes("status=open")) return { success: true, data: team ? [req("req-aaaaaa", "open"), req("req-bbbbbb", "open", "bea")] : [req("req-aaaaaa", "open")] };
    return { success: true, data: [req("req-cccccc", "answered")] };
  });
  useAgentRequestsStore.setState(initial, true);
  useOrgsStore.setState({
    currentOrgId: "o1",
    orgs: [{ id: "o1", name: "DWIT" }, { id: "o2", name: "G Studio" }] as never,
  });
});
afterEach(cleanup);

const mount = (url = "/requests") =>
  render(<MemoryRouter initialEntries={[url]}><AgentRequests /></MemoryRouter>);

describe("Por atender", () => {
  it("por defecto, lo tuyo; resueltos plegados", async () => {
    mount();
    await screen.findByText("Pedido req-aaaaaa");
    expect(get.mock.calls.map(([p]) => p)).toEqual([
      "/api/v1/requests?scope=mine&status=open",
      "/api/v1/requests?scope=mine&status=closed",
    ]);
    expect(screen.queryByText("Pedido req-cccccc")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Resolved \(1\)/ }));
    expect(screen.getByText("Pedido req-cccccc")).toBeTruthy();
  });

  it("«Equipo» pide los de todos y los de otro se ven sin botones", async () => {
    mount();
    await screen.findByText("Pedido req-aaaaaa");
    fireEvent.click(screen.getByRole("tab", { name: "Team" }));
    await screen.findByText("Pedido req-bbbbbb");
    expect(get.mock.calls.map(([p]) => p)).toContain("/api/v1/requests?scope=team&status=open");
    expect(screen.getAllByRole("button", { name: "Got it" })).toHaveLength(1);
  });

  it("?open= resalta el pedido, también si ya está resuelto", async () => {
    mount("/requests?open=req-cccccc");
    await screen.findByText("Pedido req-cccccc");
    expect(document.getElementById("request-req-cccccc")?.dataset.highlighted).toBe("true");
    expect(document.getElementById("request-req-aaaaaa")?.dataset.highlighted).toBeUndefined();
  });

  it("contestar el pedido del enlace no despliega «Resueltos» ni vuelve a desplazar", async () => {
    const scrolled = vi.fn();
    const before = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = scrolled;
    onTestFinished(() => {
      Element.prototype.scrollIntoView = before;
    });
    mount("/requests?open=req-aaaaaa");
    await screen.findByText("Pedido req-aaaaaa");
    expect(scrolled).toHaveBeenCalledTimes(1);
    const answered = { ...req("req-aaaaaa", "answered") };
    act(() => {
      const s = useAgentRequestsStore.getState();
      useAgentRequestsStore.setState({ open: [], closed: [answered, ...s.closed] });
    });
    act(() => useAgentRequestsStore.setState({ open: [req("req-dddddd", "open")] }));
    expect(screen.queryByText("Pedido req-cccccc")).toBeNull();
    expect(scrolled).toHaveBeenCalledTimes(1);
  });

  it("un pedido que entra por encima no mueve lo que se está leyendo", async () => {
    const { container } = mount();
    await screen.findByText("Pedido req-aaaaaa");
    const scroller = container.firstElementChild as HTMLDivElement;
    // Sin maquetación en jsdom: cada tarjeta mide 100 y se coloca por su puesto.
    const box = (top: number) => ({ top, bottom: top + 100, left: 0, right: 0, width: 0, height: 100, x: 0, y: top, toJSON: () => ({}) });
    const spy = vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
      if (this === scroller) return box(0) as DOMRect;
      const cards = [...scroller.querySelectorAll("[data-request-card]")];
      return box(cards.indexOf(this) * 100 - scroller.scrollTop) as DOMRect;
    });
    act(() => useAgentRequestsStore.setState({ open: [req("req-aaaaaa", "open"), req("req-bbbbbb", "open")] }));
    scroller.scrollTop = 100;
    fireEvent.scroll(scroller);
    act(() => useAgentRequestsStore.setState({ open: [req("req-eeeeee", "open"), req("req-ffffff", "open"), req("req-aaaaaa", "open"), req("req-bbbbbb", "open")] }));
    expect(scroller.scrollTop).toBe(300);
    spy.mockRestore();
  });

  // Rotura que atrapa: pintar o contar los pedidos de otra organización, o no
  // avisar de ellos (command-and-control-527).
  describe("varias organizaciones", () => {
    const other = (code: string) => ({ ...req(code, "open"), orgId: "o2" });
    const withOtherOrg = () =>
      get.mockImplementation(async (p: string) =>
        p.includes("status=open")
          ? { success: true, data: [req("req-aaaaaa", "open"), other("req-gggggg"), other("req-hhhhhh")] }
          : { success: true, data: [req("req-cccccc", "answered"), { ...req("req-iiiiii", "answered"), orgId: "o2" }] },
      );

    it("sólo enseña los de la org que se mira, y avisa de los de las otras", async () => {
      withOtherOrg();
      mount();
      await screen.findByText("Pedido req-aaaaaa");
      expect(screen.queryByText("Pedido req-gggggg")).toBeNull();
      expect(screen.getByTestId("requests-elsewhere").textContent).toBe("2 more requests in G Studio");
      expect(screen.getByRole("button", { name: /Resolved \(1\)/ })).toBeTruthy();
    });

    // Rotura que atrapa: avisar «2 more requests in o-ghost» con el id de una org sin nombre.
    it("una org sin nombre conocido no avisa (nunca se dice un id)", async () => {
      get.mockImplementation(async (p: string) =>
        p.includes("status=open")
          ? { success: true, data: [req("req-aaaaaa", "open"), { ...req("req-gggggg", "open"), orgId: "o-ghost" }] }
          : { success: true, data: [] },
      );
      mount();
      await screen.findByText("Pedido req-aaaaaa");
      expect(screen.queryByTestId("requests-elsewhere")).toBeNull();
      expect(document.body.textContent).not.toContain("o-ghost");
    });

    it("pulsar el aviso cambia de organización y enseña los de allí", async () => {
      withOtherOrg();
      mount();
      await screen.findByText("Pedido req-aaaaaa");
      fireEvent.click(screen.getByTestId("requests-elsewhere"));
      await screen.findByText("Pedido req-gggggg");
      expect(useOrgsStore.getState().currentOrgId).toBe("o2");
      expect(screen.queryByText("Pedido req-aaaaaa")).toBeNull();
      expect(screen.getByTestId("requests-elsewhere").textContent).toBe("1 more request in DWIT");
    });
  });

  // Rotura que atrapa: que `refreshCount` no guarde el número (el del riel al
  // arrancar y tras cada evento `request:*` sale de ahí).
  it("refreshCount guarda lo tuyo abierto, de todas las organizaciones", async () => {
    get.mockImplementation(async () => ({ success: true, data: [req("req-aaaaaa", "open"), { ...req("req-gggggg", "open"), orgId: "o2" }] }));
    await useAgentRequestsStore.getState().refreshCount();
    expect(useAgentRequestsStore.getState().mineOpen.map((r) => r.code)).toEqual(["req-aaaaaa", "req-gggggg"]);
    expect(get).toHaveBeenCalledWith(expect.stringContaining("scope=mine"));
  });

  // Rotura que atrapa: contestar un pedido y que el número de la barra siga
  // contándolo hasta que llegue la recarga.
  it("contestar un pedido lo quita del número en el acto", async () => {
    const { api } = await import("@/lib/api");
    vi.mocked(api.post).mockResolvedValueOnce({ success: true, data: req("req-aaaaaa", "answered") });
    useAgentRequestsStore.setState({ open: [req("req-aaaaaa", "open")], mineOpen: [req("req-aaaaaa", "open"), req("req-zzzzzz", "open")] });
    await useAgentRequestsStore.getState().answer("req-aaaaaa", { approved: true });
    expect(useAgentRequestsStore.getState().mineOpen.map((r) => r.code)).toEqual(["req-zzzzzz"]);
  });
});
