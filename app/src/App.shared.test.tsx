import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";

/**
 * La página compartida se abre **sin sesión**, como `/join`: quien llega con
 * el enlace no tiene cuenta, y dentro de `ProtectedRoute` acabaría en el login.
 */
vi.stubEnv("VITE_TARGET", "web");
vi.mock("@/lib/platform", () => ({ isTauri: false, isWeb: true, isWebBuild: true, openExternal: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class {} }));
vi.mock("@/pages/SharedDoc", () => ({ default: () => <p>the shared page</p> }));

const { default: App } = await import("@/App");
const { useAuthStore } = await import("@/store/auth.store");

afterEach(cleanup);

describe("the /shared route", () => {
  it("renders without a session", async () => {
    useAuthStore.setState({ accessToken: null, refreshToken: null, user: null } as never);
    window.history.pushState({}, "", "/shared#share-1.sig");
    render(<App />);
    expect(await screen.findByText("the shared page")).toBeTruthy();
    expect(window.location.pathname).toBe("/shared");
  });
});
