import { afterEach, describe, expect, it, vi } from "vitest";
import { taskEntry } from "@/components/tasks/due-calendar";
import { cuando } from "@/components/tasks/TaskCardMini";
import { orgZone, todayIn, zoneHint } from "@/lib/due";
import { myZone } from "@/lib/timezones";
import { useOrgsStore } from "@/store/orgs.store";

/**
 * «Hoy», para un vencimiento, es el de la zona de la organización
 * (jose, 13-nov-2026): la misma tarea vence para todos en el mismo instante,
 * y a quien está en otra hora se le dice de quién es el día.
 *
 * Roturas que atrapa: medir «hoy» con el reloj de quien mira cuando la
 * organización tiene zona; no caer al reloj de quien mira cuando no la tiene o
 * no existe; avisar de la zona a quien ya está en esa hora, o no avisar a quien
 * no; comparar el nombre de la zona en vez de su hora.
 */

const ymd = (d: Date) => [d.getFullYear(), d.getMonth() + 1, d.getDate()];
const setOrg = (timezone?: string) =>
  useOrgsStore.setState({ currentOrgId: "o-1", orgs: [{ id: "o-1", name: "Uno", timezone }] as never });
afterEach(() => useOrgsStore.setState({ currentOrgId: null, orgs: [] }));

describe("el día de hoy de una organización", () => {
  // 13-nov a las 05:30 UTC: en México sigue siendo el 12; en Auckland ya es 13
  // por la tarde; en Kiritimati (UTC+14) también 13.
  const now = new Date("2026-11-13T05:30:00Z");

  it("es el de su zona, no el de quien mira", () => {
    expect(ymd(todayIn("America/Mexico_City", now))).toEqual([2026, 11, 12]);
    expect(ymd(todayIn("Pacific/Auckland", now))).toEqual([2026, 11, 13]);
    expect(ymd(todayIn("UTC", now))).toEqual([2026, 11, 13]);
  });

  it("sin zona, o con una que no existe, es el reloj de quien mira", () => {
    const mine = [now.getFullYear(), now.getMonth() + 1, now.getDate()];
    expect(ymd(todayIn(undefined, now))).toEqual(mine);
    expect(ymd(todayIn("Marte/Olympus", now))).toEqual(mine);
    setOrg("Marte/Olympus");
    expect(orgZone()).toBeUndefined();
    setOrg();
    expect(orgZone()).toBeUndefined();
  });

  it("lo que vence el 12 vence hoy en México y ya venció en Auckland, a la vez", () => {
    const due = "2026-11-12T00:00:00.000Z";
    expect(cuando(due, "America/Mexico_City", now)).toMatchObject({ hoy: true, vencida: false });
    expect(cuando(due, "Pacific/Auckland", now)).toMatchObject({ hoy: false, vencida: true });
    // Y la zona de la organización es la que se usa si no se dice otra.
    setOrg("Pacific/Auckland");
    expect(orgZone()).toBe("Pacific/Auckland");
    expect(cuando(due, undefined, now).vencida).toBe(true);
  });
});

describe("de quién es el día, para quien está en otra hora", () => {
  const now = new Date("2026-11-13T05:30:00Z");
  // Una zona que nunca coincide con la de la máquina de pruebas (UTC, México,
  // Auckland): UTC+5:45.
  const far = "Asia/Kathmandu";

  it("se le dice a quien no está en la hora de la organización", () => {
    expect(zoneHint(far, now)).toMatch(/Kathmandu/);
    setOrg(far);
    expect(zoneHint(undefined, now)).toMatch(/Kathmandu/);
  });

  it("a quien está en esa hora no se le dice nada, se llame como se llame su zona", () => {
    expect(zoneHint(myZone(), now)).toBe("");
    expect(zoneHint(undefined, now)).toBe("");
    // Mismo desfase, otro nombre: Mérida y Ciudad de México son la misma hora.
    const twin: Record<string, string> = { "America/Mexico_City": "America/Merida", UTC: "Africa/Abidjan", "Pacific/Auckland": "Antarctica/McMurdo" };
    if (twin[myZone()]) expect(zoneHint(twin[myZone()], now)).toBe("");
  });
});

describe("el calendario mide vencida con el día de la organización", () => {
  afterEach(() => vi.useRealTimers());
  const task = { id: "t", seq: 1, title: "x", priority: "normal", dueAt: "2026-11-12T00:00:00.000Z", status: "pending" } as never;

  it("el 12 no está vencido mientras en la organización siga siendo 12", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-11-13T05:30:00Z"));
    setOrg("America/Mexico_City");
    expect(taskEntry(task).meta?.overdue).toBe(false);
    setOrg("Pacific/Auckland");
    expect(taskEntry(task).meta?.overdue).toBe(true);
  });
});
