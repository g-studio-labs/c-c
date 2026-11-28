import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, waitFor } from "@testing-library/react";

/**
 * El directo abierto está en la dirección (`?c=`, command-and-control-576).
 * Roturas que atrapa: no escribirla al abrir uno; no cambiarla al abrir otro;
 * dejarla al cerrarlo; que escribir lo abierto pise un directo pedido por
 * enlace que aún se está abriendo; que lo escrito se lea de vuelta como un
 * pedido y vuelva a abrir el directo.
 */

vi.mock("@/lib/api", () => ({
  api: { get: vi.fn(), post: vi.fn(), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
  apiUrl: (p: string) => `http://localhost${p}`,
}));
vi.mock("@/components/comms/CommsList", () => ({ default: () => null }));
vi.mock("@/components/DMThread", () => ({ default: () => <p>el hilo</p> }));

const { MemoryRouter, useLocation, useNavigationType } = await import("react-router-dom");
const { default: DirectMessages } = await import("./DirectMessages");
const { useDMStore } = await import("@/store/dm.store");
const { useOrgsStore } = await import("@/store/orgs.store");

let search = "";
let how = "";
function Spy() {
  search = useLocation().search;
  how = useNavigationType();
  return null;
}

const montar = (url: string) =>
  render(
    <MemoryRouter initialEntries={[url]}>
      <Spy />
      <DirectMessages />
    </MemoryRouter>,
  );

const open = vi.fn();
beforeEach(() => {
  open.mockReset();
  open.mockResolvedValue(undefined);
  useOrgsStore.setState({ currentOrgId: "org-b" } as never);
  useDMStore.setState({
    open,
    conversationId: null,
    conversationOrgId: null,
    conversations: [
      { conversationId: "c-1", orgId: "org-b" },
      { conversationId: "c-7", orgId: "org-b" },
    ],
  } as never);
});
afterEach(cleanup);

describe("la dirección dice el directo", () => {
  it("al abrir uno se escribe, al abrir otro cambia y al cerrarlo se quita", async () => {
    montar("/dm");
    await act(async () => {
      useDMStore.setState({ conversationId: "c-1", conversationOrgId: "org-b" } as never);
    });
    await waitFor(() => expect(search).toBe("?c=c-1"));
    expect(how).toBe("REPLACE");
    expect(open, "lo escrito no se lee de vuelta como un pedido").not.toHaveBeenCalled();

    await act(async () => {
      useDMStore.setState({ conversationId: "c-7" } as never);
    });
    await waitFor(() => expect(search).toBe("?c=c-7"));

    await act(async () => {
      useDMStore.setState({ conversationId: null, conversationOrgId: null } as never);
    });
    await waitFor(() => expect(search).toBe(""));
  });

  it("un directo pedido por enlace no lo pisa el que ya estaba abierto", async () => {
    open.mockReturnValue(new Promise(() => {}));
    useDMStore.setState({ conversationId: "c-1", conversationOrgId: "org-b" } as never);
    montar("/dm?c=c-7");
    await waitFor(() => expect(open).toHaveBeenCalledWith("c-7", "org-b"));
    expect(search).toBe("?c=c-7");
  });
});
