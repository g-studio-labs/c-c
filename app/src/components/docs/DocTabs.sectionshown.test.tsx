import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";

/**
 * La sección a la que lleva un enlace queda dicha como la que se ve, y dónde
 * (documento y pestaña): es lo que la URL refleja (`Tasks.view-url.test.tsx`).
 * Roturas que atrapa: saltar a la sección y no dejar rastro; dejarlo sin la
 * pestaña o sin el documento; no dar por atendida la petición.
 */

const saveDoc = vi.fn(async () => "hash-nuevo");

vi.mock("@/lib/api", () => ({
  api: { get: vi.fn(), post: vi.fn(), put: vi.fn(), patch: vi.fn(), delete: vi.fn() },
  apiUrl: (p: string) => p,
}));
vi.mock("@/components/markdown/MarkdownEditor", () => ({
  default: ({ value, onChange }: { value: string; onChange: (v: string) => void }) => (
    <textarea aria-label="editor" value={value} onChange={(e) => onChange(e.target.value)} />
  ),
}));
vi.mock("@/components/markdown/Markdown", () => ({
  default: ({ children }: { children: string }) => <div data-section-id="s9">{children}</div>,
}));
vi.mock("@/components/docs/DocHeader", () => ({ default: () => null }));
vi.mock("@/components/docs/DocHistory", () => ({ default: () => null }));
vi.mock("@/components/docs/ShareDoc", () => ({ default: () => null }));
vi.mock("@/components/docs/DocToc", () => ({ default: () => null }));
vi.mock("@/components/CopyId", () => ({ default: () => null }));
vi.mock("@/components/tasks/ViewSwitch", () => ({ default: () => null }));

const estado = {
  activeDoc: { kind: "list", id: "l1", name: "Portento", orgId: "o1" },
  doc: {
    doc: { id: "d1", orgId: "o1", stale: false },
    tabs: [
      { id: "t1", docId: "d1", key: "overview", body: "el resumen", bodyHash: "hash-resumen" },
      { id: "t2", docId: "d1", key: "runbook", body: "el runbook", bodyHash: "hash-runbook" },
    ],
    attachments: [],
  },
  loadingDoc: false,
  saveDoc,
  uploadDocAttachment: vi.fn(),
  closeDoc: vi.fn(),
  board: null,
  docVersions: vi.fn(async () => []),
  restoreDoc: vi.fn(),
};

vi.mock("@/store/tasks.store", () => ({
  useTasksStore: Object.assign(
    (sel: (s: Record<string, unknown>) => unknown) => sel(estado),
    { getState: () => estado },
  ),
}));

const DocTabs = (await import("@/components/docs/DocTabs")).default;
const { useDocPages } = await import("@/store/doc-pages.store");

afterEach(() => {
  cleanup();
  useDocPages.setState({ requestedSection: null, shownSection: null });
});

describe("saltar a una sección", () => {
  it("la deja como la sección que se ve, en su documento y pestaña", () => {
    Element.prototype.scrollIntoView = vi.fn();
    useDocPages.setState({ requestedSection: "s9", shownSection: null });
    render(<DocTabs onView={() => {}} />);
    expect(useDocPages.getState().shownSection).toEqual({ id: "s9", ref: "list:l1", tab: "overview", pageId: null });
    expect(useDocPages.getState().requestedSection).toBeNull();
  });
});
