import { describe, expect, it } from "vitest";
import { moveTargets } from "./move-targets";
import type { SpaceTree } from "@/types/task";

/**
 * Qué listas ofrece el detalle para mover una tarea. Roturas que atrapa:
 * ofrecer listas de otra organización, sacar una de cliente de su canal,
 * perder las listas de una subcarpeta, o no avisar de que la ref cambia al
 * cruzar de espacio (y avisarlo a una de cliente, que conserva su folio).
 */
const list = (id: string, name: string, projectId?: string) => ({ id, name, projectId, taskCount: 0, openCount: 0 });
const tree = [
  {
    id: "s-1", orgId: "org-1", name: "Uno", color: "", people: [],
    lists: [list("l-a", "A"), list("l-cli", "Del cliente", "p-1")],
    folders: [{ id: "f-1", name: "Carpeta", lists: [], folders: [{ id: "f-2", name: "Sub", lists: [list("l-sub", "Honda")], folders: [] }] }],
  },
  { id: "s-2", orgId: "org-1", name: "Dos", color: "", people: [], lists: [list("l-b", "B"), list("l-cli2", "Cliente allí", "p-1"), list("l-otro", "Otro cliente", "p-2")], folders: [] },
  { id: "s-x", orgId: "org-2", name: "Ajena", color: "", people: [], lists: [list("l-x", "X")], folders: [] },
] as unknown as SpaceTree[];

const ids = (g: ReturnType<typeof moveTargets>) => g.flatMap((x) => x.targets.map((t) => t.id));

describe("a qué listas se puede mover una tarea", () => {
  it("una interna, a cualquiera de su organización, con las de subcarpetas", () => {
    const g = moveTargets(tree, { orgId: "org-1", listId: "l-a" });
    expect(ids(g)).toEqual(["l-a", "l-cli", "l-sub", "l-b", "l-cli2", "l-otro"]);
    expect(g.find((x) => x.space === "Uno")!.targets.find((t) => t.id === "l-sub")!.label).toBe("Carpeta · Sub · Honda");
  });

  it("avisa de que la ref cambia sólo fuera de su espacio", () => {
    const g = moveTargets(tree, { orgId: "org-1", listId: "l-a" });
    const cross = Object.fromEntries(g.flatMap((x) => x.targets.map((t) => [t.id, t.crossesSpace])));
    expect(cross["l-sub"]).toBe(false);
    expect(cross["l-b"]).toBe(true);
  });

  it("una de cliente, sólo a las listas de su canal, y conserva su folio", () => {
    const g = moveTargets(tree, { orgId: "org-1", listId: "l-cli", projectId: "p-1" });
    expect(ids(g)).toEqual(["l-cli", "l-cli2"]);
    expect(g.flatMap((x) => x.targets).every((t) => !t.crossesSpace)).toBe(true);
  });
});
