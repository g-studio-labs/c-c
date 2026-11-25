import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * Un agente pide abrir un pedido (`open_request`): la app se pone delante,
 * navega a «Por atender» con él resaltado y le dice al servidor «lo abrí» con
 * el mismo nonce, que es lo que el MCP espera para decir `delivered`.
 *
 * Roturas que atrapa: abrir uno de una organización a la que no se puede
 * pasar (se navegaría y se diría «lo abrí»); no pasar el código a minúsculas
 * al buscarlo; no navegar; navegar sin el pedido; no mandar el «lo
 * abrí» o mandarlo con otro nonce (el agente diría que no hay app); abrirlo
 * dos veces; no atender el evento en el repartidor o en el transporte del
 * navegador; que la bandeja no se repinte con los `request:*`.
 */

const post = vi.fn(async () => ({}));
const get = vi.fn(async (..._a: unknown[]) => ({ success: true, data: [] as unknown[] }));
vi.mock("@/lib/api", () => ({
  api: { get: (...a: unknown[]) => get(...a), post: (...a: unknown[]) => post(...(a as [])), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
  apiUrl: (p: string) => `http://localhost${p}`,
}));

const { MemoryRouter, useLocation } = await import("react-router-dom");
const { usePendingRequestOpen } = await import("@/hooks/use-pending-request-open");
const { useAgentRequestsStore } = await import("@/store/agent-requests.store");
const { useOrgsStore } = await import("@/store/orgs.store");

const known = (code: string, orgId: string) => ({ code, orgId }) as never;

let where = "";
function Probe() {
  usePendingRequestOpen();
  const l = useLocation();
  where = l.pathname + l.search;
  return null;
}

beforeEach(() => {
  post.mockClear();
  useAgentRequestsStore.setState({ mineOpen: [], open: [], closed: [], pendingOpen: null });
  useOrgsStore.setState({ currentOrgId: "o1", orgs: [{ id: "o1", name: "DWIT" }, { id: "o2", name: "G Studio" }] as never });
});
afterEach(cleanup);

describe("abrir un pedido desde un agente", () => {
  it("navega a él y dice «lo abrí» con su nonce, una sola vez", async () => {
    useAgentRequestsStore.setState({ mineOpen: [known("req-7k2m9x", "o1")] });
    render(<MemoryRouter initialEntries={["/my-work"]}><Probe /></MemoryRouter>);
    // En mayúsculas, como lo puede escribir quien lo copia: se encuentra igual.
    useAgentRequestsStore.getState().askToOpen("REQ-7K2M9X", "n-1");
    await waitFor(() => expect(where).toBe("/requests?open=REQ-7K2M9X"));
    expect(post.mock.calls).toEqual([["/api/v1/requests/REQ-7K2M9X/opened", { nonce: "n-1" }, true]]);
    expect(useAgentRequestsStore.getState().pendingOpen).toBeNull();
  });

  it("de otra organización: se pasa a ella antes de enseñarlo y de decir «lo abrí»", async () => {
    useAgentRequestsStore.setState({ mineOpen: [known("req-g1", "o2")] });
    render(<MemoryRouter initialEntries={["/my-work"]}><Probe /></MemoryRouter>);
    useAgentRequestsStore.getState().askToOpen("req-g1", "n-3");
    await waitFor(() => expect(where).toBe("/requests?open=req-g1"));
    expect(useOrgsStore.getState().currentOrgId).toBe("o2");
    await waitFor(() => expect(post.mock.calls).toEqual([["/api/v1/requests/req-g1/opened", { nonce: "n-3" }, true]]));
  });

  it("de una organización a la que no se puede pasar: ni se navega ni se dice «lo abrí»", async () => {
    useAgentRequestsStore.setState({ mineOpen: [known("req-x1", "o9")] });
    render(<MemoryRouter initialEntries={["/my-work"]}><Probe /></MemoryRouter>);
    useAgentRequestsStore.getState().askToOpen("req-x1", "n-6");
    await waitFor(() => expect(useAgentRequestsStore.getState().pendingOpen).toBeNull());
    await new Promise((r) => setTimeout(r, 50));
    expect(where).toBe("/my-work");
    expect(post).not.toHaveBeenCalled();
    expect(useOrgsStore.getState().currentOrgId).toBe("o1");
  });

  it("uno recién creado, que aún no está en las listas, se busca antes de decidir la organización", async () => {
    get.mockResolvedValueOnce({ success: true, data: [known("req-new", "o2")] });
    render(<MemoryRouter initialEntries={["/my-work"]}><Probe /></MemoryRouter>);
    useAgentRequestsStore.getState().askToOpen("req-new", "n-5");
    await waitFor(() => expect(post).toHaveBeenCalledTimes(1));
    expect(useOrgsStore.getState().currentOrgId).toBe("o2");
  });

  it("uno que no se encuentra en ninguna lista no se da por enseñado", async () => {
    render(<MemoryRouter initialEntries={["/my-work"]}><Probe /></MemoryRouter>);
    useAgentRequestsStore.getState().askToOpen("req-zzzzzz", "n-4");
    await waitFor(() => expect(where).toBe("/requests?open=req-zzzzzz"));
    await new Promise((r) => setTimeout(r, 50));
    expect(post).not.toHaveBeenCalled();
    expect(useOrgsStore.getState().currentOrgId).toBe("o1");
  });

  const fuente = () => readFileSync(join(process.cwd(), "src/hooks/use-report-events.ts"), "utf-8");

  it("el repartidor lo atiende y trae la ventana al frente", () => {
    const f = fuente();
    const rama = f.slice(f.indexOf('case "request:open":'), f.indexOf('case "meeting:reminder":'));
    expect(rama).toContain("askToOpen(o.code, o.nonce)");
    expect(rama).toContain("bringToFront()");
    for (const ev of ["request:created", "request:updated", "request:answered", "request:withdrawn"]) {
      const at = f.indexOf(`case "${ev}":`);
      expect(at, ev).toBeGreaterThan(0);
      expect(f.slice(at, f.indexOf("break;", at))).toContain("onChange()");
    }
  });

  it("y el transporte del navegador los escucha", () => {
    const lista = fuente().slice(fuente().indexOf("for (const kind of ["));
    const nombres = lista.slice(0, lista.indexOf("]"));
    for (const ev of ["request:created", "request:updated", "request:answered", "request:withdrawn", "request:open"]) {
      expect(nombres, ev).toContain(`"${ev}"`);
    }
  });
});
