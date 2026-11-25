import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

/**
 * Los enlaces de un pedido se pueden pulsar.
 *
 * Roturas que atrapa: una dirección que no llega al navegador; una ruta que no
 * llega a Rust; una ruta de la app que abre el navegador en vez de navegar; en
 * la web, una ruta pulsable (es de otra máquina); un comando cuyo botón copia
 * el texto con enlaces en vez del crudo.
 */
const platform = vi.hoisted(() => ({ tauri: true, external: [] as string[], local: [] as string[] }));
vi.mock("@/lib/platform", () => ({
  get isTauri() {
    return platform.tauri;
  },
  openExternal: async (u: string) => void platform.external.push(u),
  openLocalPath: async (p: string) => void platform.local.push(p),
}));
const copied: string[] = [];
vi.mock("@/lib/clipboard", () => ({ copyText: async (t: string) => void copied.push(t) }));
vi.mock("@/lib/api", () => ({ api: { get: vi.fn(), post: vi.fn(), patch: vi.fn(), delete: vi.fn(), postForm: vi.fn() }, apiUrl: (p: string) => p }));

const { MemoryRouter, Routes, Route, useLocation } = await import("react-router-dom");
const { default: RequestCard } = await import("@/components/requests/RequestCard");
import type { AgentRequest } from "@/lib/requests";

const Where = () => <span data-testid="where">{useLocation().pathname + useLocation().search}</span>;

function request(body: AgentRequest["body"], kind: AgentRequest["kind"] = "run"): AgentRequest {
  return {
    id: "u1", code: "req-7k2m9x", orgId: "o1", kind, summary: "Mira esto", version: 1, status: "open",
    task: { id: "t1", ref: "cac-10", title: "Panel", priority: "high", status: "in_progress", link: "/tasks?task=t1" },
    body, requestedOf: { id: "me", name: "Jose" }, requestedBy: { id: "me", name: "Jose" }, via: "mcp",
    createdAt: new Date().toISOString(), updatedAt: new Date().toISOString(), canAnswer: true, link: "/requests?open=req-7k2m9x",
  };
}
const mount = (r: AgentRequest) =>
  render(
    <MemoryRouter initialEntries={["/requests"]}>
      <Routes><Route path="*" element={<><RequestCard request={r} /><Where /></>} /></Routes>
    </MemoryRouter>,
  );

beforeEach(() => {
  platform.tauri = true;
  platform.external.length = 0;
  platform.local.length = 0;
  copied.length = 0;
});
afterEach(cleanup);

describe("enlaces en un pedido", () => {
  it("una dirección va al navegador y una ruta al sistema", () => {
    mount(request({ command: "psql -f x", where: "abre https://supabase.com/dashboard/p/sql/new.", expected: "ok", undo: "borra ~/tmp/a.sql" }));
    fireEvent.click(screen.getByText("https://supabase.com/dashboard/p/sql/new"));
    expect(platform.external).toEqual(["https://supabase.com/dashboard/p/sql/new"]);
    fireEvent.click(screen.getByText("~/tmp/a.sql"));
    expect(platform.local).toEqual(["~/tmp/a.sql"]);
  });

  it("una ruta de la app navega dentro, sin salir al navegador", async () => {
    mount(request({ steps: ["ve a /app/tasks?task=t9"], expected: "ok" }, "test"));
    fireEvent.click(screen.getByText("/app/tasks?task=t9"));
    await waitFor(() => expect(screen.getByTestId("where").textContent).toBe("/tasks?task=t9"));
    expect(platform.external).toEqual([]);
    expect(platform.local).toEqual([]);
  });

  it("en la web una ruta no se abre: se copia", async () => {
    platform.tauri = false;
    mount(request({ steps: ["abre /home/rv/a.sql"], expected: "ok" }, "test"));
    fireEvent.click(screen.getByText("/home/rv/a.sql"));
    await waitFor(() => expect(copied).toEqual(["/home/rv/a.sql"]));
    expect(platform.local).toEqual([]);
  });

  it("el botón del comando copia el texto crudo", async () => {
    const command = "open https://x.com/a /tmp/b.sql";
    mount(request({ command, where: "cac", expected: "ok" }));
    fireEvent.click(screen.getByRole("button", { name: "Copy the command" }));
    await waitFor(() => expect(copied).toEqual([command]));
  });
});
