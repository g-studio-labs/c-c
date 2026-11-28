import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { MemoryRouter, useLocation, useNavigationType } from "react-router-dom";

/**
 * La URL dice lo que se ve (command-and-control-576). Roturas que atrapa: no
 * escribir la URL al abrir un documento o cambiar de pestaña; perder `tab` o
 * `page`; que el `cac://` de «Abrir en la app» no sea el de lo visto; que leer
 * de vuelta lo escrito reabra el documento en bucle.
 */

const { api } = vi.hoisted(() => ({
  api: { get: vi.fn(), post: vi.fn(), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
}));
vi.mock("@/lib/api", () => ({ api, apiUrl: (p: string) => `http://localhost${p}` }));
vi.mock("@/components/ItemCalendar", () => ({ default: () => null }));

const { default: Tasks } = await import("./Tasks");
const { useTasksStore } = await import("@/store/tasks.store");
const { useDocPages } = await import("@/store/doc-pages.store");
const { useOrgsStore } = await import("@/store/orgs.store");
const { toAppUrl } = await import("@/lib/open-in-app");
const { PromptProvider } = await import("@/components/PromptDialog");
const { ConfirmProvider } = await import("@/components/ConfirmDialog");

let where = { pathname: "", search: "" };
let how = "";
function Spy() {
  const l = useLocation();
  where = { pathname: l.pathname, search: l.search };
  how = useNavigationType();
  return null;
}

beforeEach(() => {
  api.get.mockImplementation(async (url: string) => {
    if (url.startsWith("/api/v1/docs/space/sp-a/pages")) return { success: true, data: [] };
    if (url.startsWith("/api/v1/docs/space/sp-a"))
      return { success: true, data: { doc: null, tabs: [], decisions: [], attachments: [], orgId: "org-a" } };
    return { success: true, data: [] };
  });
  useOrgsStore.setState({ orgs: [{ id: "org-a", name: "A" }], currentOrgId: "org-a" } as never);
  useTasksStore.setState({ openTaskId: null, detail: null, activeDoc: null });
  useDocPages.setState({ shownTab: "overview", activePageId: null });
});
afterEach(cleanup);

describe("la URL refleja lo que se ve", () => {
  it("un documento con su pestaña y su página, y el cac:// de lo visto", async () => {
    render(
      <ConfirmProvider>
        <PromptProvider>
          <MemoryRouter initialEntries={["/tasks"]}>
            <Spy />
            <Tasks />
          </MemoryRouter>
        </PromptProvider>
      </ConfirmProvider>,
    );
    await act(async () => {
      useTasksStore.setState({ activeDoc: { kind: "space", id: "sp-a", name: "", orgId: "org-a" } });
    });
    await waitFor(() => expect(new URLSearchParams(where.search).get("doc")).toBe("space:sp-a"));

    await act(async () => {
      useDocPages.setState({ shownTab: "runbook", activePageId: "p-1" });
    });
    await waitFor(() => expect(where.search).toContain("page=p-1"));
    const q = new URLSearchParams(where.search);
    expect([q.get("doc"), q.get("tab"), q.get("page")]).toEqual(["space:sp-a", "runbook", "p-1"]);
    expect(decodeURIComponent(toAppUrl(where.pathname, where.search, ""))).toBe("cac://tasks?doc=space:sp-a&tab=runbook&page=p-1");
    expect(how, "cada pestaña reemplaza, no llena el historial").toBe("REPLACE");
    // Lo escrito no se lee de vuelta como un enlace nuevo: la página sigue.
    expect(useDocPages.getState().activePageId).toBe("p-1");
  });
});
