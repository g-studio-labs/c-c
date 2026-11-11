import { describe, expect, it } from "vitest";
import { dueCalendarItems, taskEntry } from "@/components/tasks/due-calendar";

/**
 * Lo que una tarea le cuenta al calendario además del título
 * (command-and-control-334). Roturas que atrapa: contar como vencida la que
 * vence hoy, o una terminada; no tachar una cerrada; perder la prioridad
 * `medium`, que es como se guarda la normal.
 */
const iso = (d: Date) => new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate())).toISOString();
const today = new Date();
const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);

const base = { seq: 1, title: "t", priority: "none" as const };

describe("una tarea en el calendario", () => {
  it("vencida es la de ayer sin cerrar; la de hoy todavía no", () => {
    const [ayer, hoy] = dueCalendarItems([
      { ...base, id: "a", dueAt: iso(yesterday), status: "open" },
      { ...base, id: "b", dueAt: iso(today), status: "open" },
    ]);
    expect(ayer.meta?.overdue).toBe(true);
    expect(hoy.meta?.overdue).toBe(false);
  });

  it("una terminada o cerrada no vence: se tacha", () => {
    const items = dueCalendarItems([
      { ...base, id: "d", dueAt: iso(yesterday), status: "done" },
      { ...base, id: "c", dueAt: iso(yesterday), status: "closed" },
    ]);
    for (const i of items) {
      expect(i.meta?.overdue).toBe(false);
      expect(i.meta?.done).toBe(true);
    }
  });

  it("el tono es la prioridad, con `medium` como normal", () => {
    expect(taskEntry({ ...base, id: "m", priority: "medium" as never }).tone).toBe("normal");
    expect(taskEntry({ ...base, id: "u", priority: "urgent" }).tone).toBe("urgent");
  });

  it("lleva estado y las iniciales de quien la lleva", () => {
    const e = taskEntry({ ...base, id: "s", status: "in_progress", statusName: "En curso", assignee: "Ana López" });
    expect(e.meta?.status?.name).toBe("En curso");
    expect(e.meta?.assignee?.initials).toBe("AL");
  });
});
