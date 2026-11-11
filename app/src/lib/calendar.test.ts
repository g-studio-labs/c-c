import { afterEach, describe, expect, it } from "vitest";
import i18next from "i18next";

import { heatLevel, monthWeeks, toneSegments, weekDays, weekLabel, weekStart, type CalendarTone } from "@/lib/calendar";
import { mesYAno } from "@/lib/fechas";

/**
 * La aritmética del calendario (command-and-control-334). Roturas que atrapa:
 * segmentos fuera del orden de los tonos o con vacíos; los cortes del mapa de
 * calor; un mes con filas de más o de menos; una semana que no empieza en
 * lunes; el título de una semana que cruza de mes o de año; y «De» en
 * mayúscula en el título del mes.
 */
const TONES: CalendarTone[] = [
  { key: "high", label: "Alta", dot: "bg-warning", edge: "bg-warning" },
  { key: "normal", label: "Normal", dot: "bg-info", edge: "bg-info" },
  { key: "none", label: "Sin prioridad", dot: "bg-muted-soft", edge: "bg-border-strong" },
];

afterEach(() => {
  i18next.changeLanguage("en");
});

describe("el calendario, por dentro", () => {
  it("cuenta por tono en el orden de los tonos y sin los vacíos", () => {
    const segs = toneSegments([{ tone: "none" }, { tone: "high" }, { tone: "none" }], TONES);
    expect(segs.map((s) => [s.tone.key, s.count])).toEqual([["high", 1], ["none", 2]]);
  });

  it("el mapa de calor corta en 1, 2–3 y 4", () => {
    expect([0, 1, 2, 3, 4, 9].map(heatLevel)).toEqual([0, 1, 2, 2, 3, 3]);
  });

  it("un mes ocupa las semanas que necesita, empezando en lunes", () => {
    // Noviembre de 2026 empieza en domingo: seis semanas.
    const nov = monthWeeks(new Date(2026, 10, 1));
    expect(nov).toHaveLength(42);
    expect(nov[0].getDay()).toBe(1);
    expect([nov[0].getMonth(), nov[0].getDate()]).toEqual([9, 26]);
    // Febrero de 2027 empieza en lunes y tiene 28 días: cuatro.
    expect(monthWeeks(new Date(2027, 1, 1))).toHaveLength(28);
    // Octubre de 2026 empieza en jueves: cinco.
    expect(monthWeeks(new Date(2026, 9, 1))).toHaveLength(35);
  });

  it("la semana empieza en lunes, también desde un domingo", () => {
    const s = weekStart(new Date(2026, 10, 15)); // domingo
    expect([s.getMonth(), s.getDate(), s.getDay()]).toEqual([10, 9, 1]);
    expect(weekDays(s).map((d) => d.getDate())).toEqual([9, 10, 11, 12, 13, 14, 15]);
  });

  it("titula la semana como el diseño, cruce o no de mes o de año", () => {
    i18next.changeLanguage("es");
    expect(weekLabel(new Date(2026, 10, 9))).toBe("9 – 15 noviembre 2026");
    expect(weekLabel(new Date(2026, 10, 30))).toBe("30 nov – 6 diciembre 2026");
    expect(weekLabel(new Date(2026, 11, 28))).toBe("28 dic 2026 – 3 enero 2027");
    expect(weekLabel(new Date(2026, 10, 9), true)).toBe("9 – 15 nov");
    expect(weekLabel(new Date(2026, 10, 30), true)).toBe("30 nov – 6 dic");
  });

  it("el título del mes sólo lleva mayúscula al principio", () => {
    i18next.changeLanguage("es");
    expect(mesYAno(new Date(2026, 10, 1))).toBe("Noviembre de 2026");
    i18next.changeLanguage("en");
    expect(mesYAno(new Date(2026, 10, 1))).toBe("November 2026");
  });
});
