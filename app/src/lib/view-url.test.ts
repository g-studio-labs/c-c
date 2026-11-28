import { describe, expect, it } from "vitest";
import { viewSearch } from "./view-url";

/**
 * La consulta de la URL de Tareas (command-and-control-576). Roturas que
 * atrapa: perder la sección, la pestaña o la página; escribir una sección que
 * es de otro documento, de otra pestaña o de otra página; que una tarea abierta
 * no gane al documento.
 */
const doc = { kind: "space", id: "sp-a" } as const;
const section = { id: "s9", ref: "space:sp-a", tab: "runbook", pageId: null };

describe("viewSearch", () => {
  it("un documento con su pestaña y su sección", () => {
    expect(viewSearch({ taskId: null, doc, tab: "runbook", pageId: null, section })).toBe("doc=space%3Asp-a&tab=runbook&section=s9");
  });

  it("una página con su sección", () => {
    const at = { ...section, tab: null, pageId: "p1" };
    expect(viewSearch({ taskId: null, doc, tab: "overview", pageId: "p1", section: at })).toBe("doc=space%3Asp-a&page=p1&section=s9");
  });

  it("una sección de otra pestaña, página o documento no se escribe", () => {
    expect(viewSearch({ taskId: null, doc, tab: "decisions", pageId: null, section })).toBe("doc=space%3Asp-a&tab=decisions");
    expect(viewSearch({ taskId: null, doc, tab: "runbook", pageId: "p2", section })).toBe("doc=space%3Asp-a&tab=runbook&page=p2");
    expect(viewSearch({ taskId: null, doc: { kind: "space", id: "otro" }, tab: "runbook", pageId: null, section })).toBe("doc=space%3Aotro&tab=runbook");
  });

  it("una tarea abierta gana al documento, y sin nada la consulta está vacía", () => {
    expect(viewSearch({ taskId: "t1", doc, tab: "runbook", pageId: null, section })).toBe("task=t1");
    expect(viewSearch({ taskId: null, doc: null, tab: "overview", pageId: null })).toBe("");
  });
});
