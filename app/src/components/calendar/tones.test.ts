import { describe, expect, it } from "vitest";
import { priorityTone, priorityTones } from "@/components/calendar/tones";

/**
 * Los tonos de prioridad del calendario (command-and-control-336). Roturas que
 * atrapan: sacar la urgente de «prioridad alta» del resumen de la semana, un
 * orden de la barra que no vaya de más a menos, y perder `medium`.
 */
const t = (k: string) => k;

describe("los tonos de prioridad", () => {
  it("van de más a menos, y «alta» es la urgente y la alta", () => {
    const tones = priorityTones(t);
    expect(tones.map((x) => x.key)).toEqual(["urgent", "high", "normal", "low", "none"]);
    expect(tones.filter((x) => x.high).map((x) => x.key)).toEqual(["urgent", "high"]);
  });

  it("`medium` es la normal, y lo desconocido no tiene prioridad", () => {
    expect(priorityTone("medium")).toBe("normal");
    expect(priorityTone(undefined)).toBe("none");
  });
});
