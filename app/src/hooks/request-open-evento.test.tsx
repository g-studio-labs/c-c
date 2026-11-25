import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * Un agente pide abrir un pedido (`open_request`): la app se pone delante,
 * navega a «Por atender» con él resaltado y le dice al servidor «lo abrí» con
 * el mismo nonce, que es lo que el MCP espera para decir `delivered`.
 *
 * Roturas que atrapa: no navegar; navegar sin el pedido; no mandar el «lo
 * abrí» o mandarlo con otro nonce (el agente diría que no hay app); abrirlo
 * dos veces; no atender el evento en el repartidor o en el transporte del
 * navegador; que la bandeja no se repinte con los `request:*`.
 */

const post = vi.fn(async () => ({}));
vi.mock("@/lib/api", () => ({
  api: { get: vi.fn(async () => ({ success: true, data: [] })), post: (...a: unknown[]) => post(...(a as [])), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
  apiUrl: (p: string) => `http://localhost${p}`,
}));

const { MemoryRouter, useLocation } = await import("react-router-dom");
const { usePendingRequestOpen } = await import("@/hooks/use-pending-request-open");
const { useAgentRequestsStore } = await import("@/store/agent-requests.store");

let where = "";
function Probe() {
  usePendingRequestOpen();
  const l = useLocation();
  where = l.pathname + l.search;
  return null;
}

afterEach(cleanup);

describe("abrir un pedido desde un agente", () => {
  it("navega a él y dice «lo abrí» con su nonce, una sola vez", async () => {
    render(<MemoryRouter initialEntries={["/my-work"]}><Probe /></MemoryRouter>);
    useAgentRequestsStore.getState().askToOpen("req-7k2m9x", "n-1");
    await waitFor(() => expect(where).toBe("/requests?open=req-7k2m9x"));
    expect(post.mock.calls).toEqual([["/api/v1/requests/req-7k2m9x/opened", { nonce: "n-1" }, true]]);
    expect(useAgentRequestsStore.getState().pendingOpen).toBeNull();
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
