import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

/**
 * «Reportes abiertos» del resumen: sólo los que siguen abiertos.
 *
 * Rotura que atrapa: filtrar por el nombre viejo («resolved») después de
 * normalizar, que es «done»; así la tarjeta contaba los reportes hechos.
 */

const { api } = vi.hoisted(() => ({
  api: { get: vi.fn(), post: vi.fn(), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
}));
vi.mock("@/lib/api", () => ({ api, apiUrl: (p: string) => `http://localhost${p}` }));
vi.mock("@/hooks/use-servers", () => ({ useServers: () => ({ servers: [] }) }));

const { default: Overview } = await import("./Overview");
const { useOrgsStore } = await import("@/store/orgs.store");

const report = (id: string, status: string) => ({ id, seq: 1, title: `Reporte ${status}`, status, createdAt: new Date().toISOString() });

beforeEach(() => {
  api.get.mockImplementation(async (url: string) => {
    if (url.startsWith("/api/v1/reports/"))
      return {
        success: true,
        data: {
          items: [report("r1", "open"), report("r2", "pending"), report("r3", "in_progress"), report("r4", "resolved"), report("r5", "done"), report("r6", "closed")],
        },
      };
    return { success: true, data: [] };
  });
  useOrgsStore.setState({ orgs: [{ id: "org-1", name: "Uno" }], currentOrgId: "org-1" } as never);
});
afterEach(cleanup);

describe("reportes abiertos, en el resumen", () => {
  it("no cuenta los hechos ni los cerrados", async () => {
    render(
      <MemoryRouter>
        <Overview />
      </MemoryRouter>,
    );
    await waitFor(() => expect(screen.getByText("Reporte open")).toBeTruthy());
    expect(screen.getByText("Reporte pending")).toBeTruthy();
    expect(screen.getByText("Reporte in_progress")).toBeTruthy();
    for (const gone of ["resolved", "done", "closed"]) expect(screen.queryByText(`Reporte ${gone}`)).toBeNull();
  });
});
