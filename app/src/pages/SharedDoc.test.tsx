import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("@/lib/api", () => ({
  api: { post: vi.fn(), get: vi.fn(), delete: vi.fn() },
  apiUrl: (p: string) => p,
  codigoDe: () => "",
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class {}, convertFileSrc: (p: string) => p }));

import SharedDoc from "@/pages/SharedDoc";

const ATT = "11111111-2222-3333-4444-555555555555";
const TOKEN = "share-1.sig";

const tree = {
  title: "Runbook",
  expiresAt: "2026-12-01T18:00:00Z",
  pages: [
    {
      id: "p-root",
      parentId: null,
      title: "Runbook",
      body: [
        "Root body.",
        `![diagram](/api/v1/docs/d1/attachments/${ATT}/raw)`,
        `[spec.pdf](/api/v1/docs/d1/attachments/${ATT}/raw)`,
        "[task](cac:task/abc) [board](/tasks?space=s1) [web](https://example.com/x) [child](/tasks?space=s1&page=p-child)",
        `[app](${window.location.origin}/app/tasks?space=s1)`,
      ].join("\n\n"),
    },
    { id: "p-child", parentId: "p-root", title: "Deploys", body: "Child body." },
  ],
};

const fetchMock = vi.fn();
const open = vi.fn();

beforeEach(() => {
  fetchMock.mockReset();
  open.mockReset();
  vi.stubGlobal("fetch", fetchMock);
  vi.stubGlobal("open", open);
  window.location.hash = "#" + TOKEN;
  URL.createObjectURL = vi.fn(() => "blob:shared-diagram");
  URL.revokeObjectURL = vi.fn();
  fetchMock.mockImplementation(async (url: string) => {
    if (url === "/api/v1/shared-docs") return new Response(JSON.stringify({ success: true, data: tree }));
    if (url === `/api/v1/shared-docs/attachments/${ATT}`) return new Response(new Blob(["png"]));
    return new Response("", { status: 404 });
  });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the shared page", () => {
  // El token del `#` va en la cabecera, sin sesión, y nunca en la URL.
  it("asks with the token in the X-Doc-Share header and shows the root and its expiry", async () => {
    render(<SharedDoc />);
    expect(await screen.findByText("Root body.")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Runbook" })).toBeTruthy();
    expect(screen.getByText(/Shared until/)).toBeTruthy();
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/v1/shared-docs");
    expect(init.headers).toEqual({ "X-Doc-Share": TOKEN });
    for (const [u, i] of fetchMock.mock.calls) {
      expect(String(u)).not.toContain(TOKEN);
      expect(i.headers.Authorization).toBeUndefined();
    }
  });

  it("navigates to a subpage from the tree", async () => {
    render(<SharedDoc />);
    await screen.findByText("Root body.");
    fireEvent.click(screen.getByRole("button", { name: "Deploys" }));
    expect(await screen.findByText("Child body.")).toBeTruthy();
    expect(screen.queryByText("Root body.")).toBeNull();
  });

  // Un adjunto citado se baja con la cabecera y se pinta como blob; la ruta
  // del documento no llega a pedirse.
  it("loads a cited attachment through the shared route and shows it as a blob", async () => {
    render(<SharedDoc />);
    const img = await screen.findByRole("img", { name: "diagram" });
    expect(img.getAttribute("src")).toBe("blob:shared-diagram");
    const call = fetchMock.mock.calls.find(([u]) => u === `/api/v1/shared-docs/attachments/${ATT}`);
    expect(call?.[1].headers).toEqual({ "X-Doc-Share": TOKEN });
    expect(fetchMock.mock.calls.some(([u]) => String(u).includes("/api/v1/docs/"))).toBe(false);
    fireEvent.click(screen.getByText("spec.pdf"));
    expect(open).toHaveBeenCalledWith("blob:shared-diagram", "_blank", "noopener,noreferrer");
  });

  // Los enlaces de dentro de la app no llevan a ningún sitio: quien lee no
  // tiene sesión. Uno de fuera se abre aparte, y uno a una página del árbol
  // la enseña.
  it("does not follow app-internal links", async () => {
    render(<SharedDoc />);
    await screen.findByText("Root body.");
    const before = window.location.href;
    fireEvent.click(screen.getByText("task"));
    fireEvent.click(screen.getByText("board"));
    fireEvent.click(screen.getByText("app"));
    // Lo que abriría el camino de siempre llega tras un `import()`: se le da
    // tiempo a llegar antes de decir que no pasó.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });
    expect(open).not.toHaveBeenCalled();
    expect(window.location.href).toBe(before);

    fireEvent.click(screen.getByText("web"));
    expect(open).toHaveBeenCalledWith("https://example.com/x", "_blank", "noopener,noreferrer");

    fireEvent.click(screen.getByText("child"));
    expect(await screen.findByText("Child body.")).toBeTruthy();
  });

  it("says the link opens nothing on a 404", async () => {
    fetchMock.mockImplementation(async () => new Response("", { status: 404 }));
    render(<SharedDoc />);
    expect(await screen.findByText("This link doesn't open anything")).toBeTruthy();
  });

  it("does not ask without a token", async () => {
    window.location.hash = "";
    render(<SharedDoc />);
    await waitFor(() => expect(screen.getByText("This link doesn't open anything")).toBeTruthy());
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
