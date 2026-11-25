import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";

/**
 * `org:role`: te cambiaron el papel en una organización. Se renueva el token y
 * se releen las organizaciones (que es lo que `adoptarMembresia` ordena), y se
 * dice cuál es el papel nuevo, no «te han añadido» (que es `org:membership`).
 *
 * Roturas que atrapa: no escuchar el evento en el navegador; no adoptar el
 * cambio; contestar con el aviso de haber sido añadido; un aviso sin la
 * organización o sin el papel.
 */
const { urlTicket, adoptarMembresia, toast } = vi.hoisted(() => ({
  urlTicket: vi.fn(),
  adoptarMembresia: vi.fn(),
  toast: { info: vi.fn(), error: vi.fn(), success: vi.fn(), message: vi.fn(), warning: vi.fn() },
}));
vi.mock("@/lib/url-ticket", () => ({ urlTicket }));
vi.mock("@/lib/membresia", () => ({ adoptarMembresia }));
vi.mock("sonner", () => ({ toast }));

const oyentes = new Map<string, (e: { data: string }) => void>();
class FakeES {
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: (() => void) | null = null;
  addEventListener(kind: string, fn: (e: { data: string }) => void) {
    oyentes.set(kind, fn);
  }
  close() {}
}
vi.stubGlobal("EventSource", FakeES);

const { useReportEvents } = await import("./use-report-events");
const { useAuthStore } = await import("@/store/auth.store");

afterEach(() => {
  oyentes.clear();
  vi.clearAllMocks();
});

describe("el evento org:role", () => {
  it("renueva la pertenencia y dice el papel nuevo", async () => {
    useAuthStore.setState({ accessToken: "t" } as never);
    urlTicket.mockResolvedValue("pase");
    const { unmount } = renderHook(() => useReportEvents());
    await waitFor(() => expect(oyentes.has("org:role")).toBe(true));

    oyentes.get("org:role")!({ data: JSON.stringify({ orgId: "o1", orgName: "Portento", role: "viewer" }) });

    expect(adoptarMembresia).toHaveBeenCalledTimes(1);
    expect(toast.info).toHaveBeenCalledTimes(1);
    const text = String(toast.info.mock.calls[0][0]);
    expect(text).toContain("Portento");
    expect(text).toContain("viewer");
    expect(text).not.toMatch(/added|añadido/i);
    unmount();
  });
});
