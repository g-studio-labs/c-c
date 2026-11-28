import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";

/**
 * El canal que se ve está en la dirección (command-and-control-576), también
 * cuando salió del recordado o del primero: es lo que copia quien comparte la
 * URL y lo que «Abrir en la app» lleva. Roturas que atrapa: no escribir
 * `?space=` cuando el canal se resolvió solo; pisar otros parámetros
 * (`&message=`); escribirlo en el teléfono, donde sin `?space=` se ve la
 * lista y escribirlo abriría un canal que nadie pidió.
 */

const mobile = vi.hoisted(() => ({ on: false }));
vi.mock("@/hooks/use-mobile", () => ({ useIsMobile: () => mobile.on }));
vi.mock("@/lib/api", () => ({
  api: { get: vi.fn(), post: vi.fn(), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() },
  apiUrl: (p: string) => `http://localhost${p}`,
}));
vi.mock("@/components/chat/ChannelView", () => ({ default: () => <div>el canal</div> }));
vi.mock("@/components/comms/CommsList", () => ({ default: () => <div>la lista</div> }));

const { MemoryRouter, useLocation, useNavigationType } = await import("react-router-dom");
const { default: Channels } = await import("@/pages/Channels");
const { useTasksStore } = await import("@/store/tasks.store");
const { SidebarProvider } = await import("@/components/ui/sidebar");

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
      <SidebarProvider>
        <Channels />
      </SidebarProvider>
    </MemoryRouter>,
  );

beforeEach(() => {
  mobile.on = false;
  useTasksStore.setState({
    tree: [
      { id: "sp-1", orgId: "o", name: "Uno", color: "#888", folders: [], lists: [] },
      { id: "sp-2", orgId: "o", name: "Dos", color: "#888", folders: [], lists: [] },
    ],
  } as never);
});
afterEach(cleanup);

describe("la dirección dice el canal", () => {
  it("sin canal pedido, el que se abrió queda escrito, reemplazando", async () => {
    montar("/chat");
    await waitFor(() => expect(search).toBe("?space=sp-1"));
    expect(how).toBe("REPLACE");
  });

  it("un canal pedido no se toca", async () => {
    montar("/chat?space=sp-2&message=m1");
    await Promise.resolve();
    expect(search).toBe("?space=sp-2&message=m1");
  });

  it("al escribirlo se respeta lo demás de la dirección", async () => {
    montar("/chat?message=m1");
    await waitFor(() => expect(search).toBe("?message=m1&space=sp-1"));
  });

  it("en el teléfono sin canal pedido es la lista, y la dirección sigue vacía", async () => {
    mobile.on = true;
    montar("/chat");
    await Promise.resolve();
    expect(search).toBe("");
  });
});
