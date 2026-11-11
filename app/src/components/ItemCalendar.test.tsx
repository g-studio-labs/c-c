import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import ItemCalendar, { type CalendarEntry, type CalendarItem } from "@/components/ItemCalendar";
import { HOVER_DELAY } from "@/components/calendar/MonthGrid";
import { dueCalendarItems } from "@/components/tasks/due-calendar";
import { useCalendarStore } from "@/store/calendar.store";

/**
 * El mes de escritorio (command-and-control-335).
 *
 * Lo que había antes: los títulos dentro del día, cortados a quince
 * caracteres, y «+N más» que abría el día **debajo de todo el mes**, fuera de
 * la pantalla. Ahora cada día dice cuánto hay y de qué tono, y el detalle sale
 * en un popover anclado al día: un vistazo con el ratón encima, fijado con un
 * clic.
 *
 * Roturas que atrapan: abrir sin esperar al vistazo, o no abrir; que salir con
 * el ratón cierre uno fijado; que Esc o un clic fuera no lo cierren; que con
 * uno fijado otro día abra el suyo; títulos dentro de la celda otra vez; el
 * punto de vencidas sin vencidas; el pie de «nueva tarea» donde no se pidió; y
 * el día equivocado al oeste de Greenwich.
 */

// Los tonos de prueba se llaman como su clase: así las comprobaciones de color
// leen la clase que pintan.
const TONES = ["bg-error", "bg-info", "bg-primary", "bg-muted"].map((c) => ({ key: c, label: c, dot: c, edge: c }));

// Fijado a un día concreto: «hoy» se pinta distinto, y una prueba que dependa
// de la fecha real fallaría sola dentro de un mes.
const TODAY = new Date(2026, 7, 26);

// El dato que produce la app: un día, sin hora ni zona.
const item = (id: string, dayOfMonth: number, extra: Partial<CalendarEntry> = {}): CalendarItem => ({
  id,
  title: `tarea ${id}`,
  day: `2026-08-${String(dayOfMonth).padStart(2, "0")}`,
  tone: "bg-primary",
  ...extra,
});

const cell = (iso: string) => document.querySelector<HTMLElement>(`[data-day="${iso}"]`)!;
const popover = () => screen.queryByRole("dialog");

const renderCalendar = (items: CalendarItem[], props: Partial<Parameters<typeof ItemCalendar>[0]> = {}) => {
  const onOpen = vi.fn();
  render(
    <ItemCalendar tones={TONES} calendarId="test" items={items} onOpen={onOpen} countKey="common:count.tasks" {...props} />,
  );
  return onOpen;
};

const hover = (iso: string) => {
  fireEvent.mouseEnter(cell(iso));
  act(() => {
    vi.advanceTimersByTime(HOVER_DELAY);
  });
};

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(TODAY);
  useCalendarStore.setState({ views: {}, lanes: {} });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("cada día, de un vistazo", () => {
  it("dice cuántas hay, no sus títulos", () => {
    renderCalendar([item("a", 12, { title: "pensar en ideas de contenido" }), item("b", 12), item("c", 12)]);
    const day = cell("2026-08-12");
    expect(day.textContent).toContain("3");
    expect(day.textContent).toContain("tasks");
    expect(within(day).queryByText("pensar en ideas de contenido")).toBeNull();
  });

  it("la barra va por tonos, en el orden de los tonos", () => {
    renderCalendar([item("a", 12, { tone: "bg-primary" }), item("b", 12, { tone: "bg-error" }), item("c", 12, { tone: "bg-primary" })]);
    const segs = [...cell("2026-08-12").querySelectorAll("[data-tone]")].map((s) => s.getAttribute("data-tone"));
    expect(segs).toEqual(["bg-error", "bg-primary"]);
  });

  it("el punto de vencidas sólo sale si alguna venció", () => {
    renderCalendar([item("a", 12, { meta: { overdue: true } }), item("b", 13)]);
    expect(cell("2026-08-12").querySelector("[data-overdue]")).not.toBeNull();
    expect(cell("2026-08-13").querySelector("[data-overdue]")).toBeNull();
  });

  it("hoy lleva el círculo relleno y los demás días no", () => {
    renderCalendar([]);
    expect(cell("2026-08-26").querySelector(".bg-primary")).not.toBeNull();
    expect(cell("2026-08-27").querySelector(".bg-primary")).toBeNull();
  });

  it("el mes ocupa sus semanas, no seis siempre", () => {
    // Agosto de 2026 empieza en sábado: seis. Febrero de 2027, cuatro.
    renderCalendar([]);
    expect(document.querySelectorAll("[data-day]")).toHaveLength(42);
    cleanup();
    vi.setSystemTime(new Date(2027, 1, 10));
    renderCalendar([]);
    expect(document.querySelectorAll("[data-day]")).toHaveLength(28);
  });
});

describe("el popover del día", () => {
  it("con el ratón encima se abre al vistazo, no antes", () => {
    renderCalendar([item("a", 12, { title: "abrir esto" })]);
    fireEvent.mouseEnter(cell("2026-08-12"));
    act(() => {
      vi.advanceTimersByTime(HOVER_DELAY - 10);
    });
    expect(popover()).toBeNull();
    act(() => {
      vi.advanceTimersByTime(10);
    });
    expect(within(popover()!).getByText("abrir esto")).toBeTruthy();
    expect(within(popover()!).getByText("Click to pin")).toBeTruthy();
    fireEvent.mouseLeave(cell("2026-08-12"));
    expect(popover()).toBeNull();
  });

  it("un clic lo fija: salir con el ratón no lo cierra, y otro clic lo suelta", () => {
    renderCalendar([item("a", 12)]);
    fireEvent.click(cell("2026-08-12"));
    fireEvent.mouseLeave(cell("2026-08-12"));
    expect(popover()).not.toBeNull();
    expect(within(popover()!).getByRole("button", { name: "Close" })).toBeTruthy();
    fireEvent.click(cell("2026-08-12"));
    expect(popover()).toBeNull();
  });

  it("Esc y un clic fuera de los días lo cierran", () => {
    renderCalendar([item("a", 12)]);
    fireEvent.click(cell("2026-08-12"));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(popover()).toBeNull();
    fireEvent.click(cell("2026-08-12"));
    fireEvent.mouseDown(document.body);
    expect(popover()).toBeNull();
  });

  it("con uno fijado, pasar por otro día no abre otro", () => {
    renderCalendar([item("a", 12), item("b", 14, { title: "el otro" })]);
    fireEvent.click(cell("2026-08-12"));
    hover("2026-08-14");
    expect(screen.queryByText("el otro")).toBeNull();
  });

  it("se abre con el teclado", () => {
    renderCalendar([item("a", 12)]);
    fireEvent.keyDown(cell("2026-08-12"), { key: "Enter" });
    expect(popover()).not.toBeNull();
  });

  it("el de hoy lo dice, y el de otro día no", () => {
    renderCalendar([]);
    hover("2026-08-26");
    expect(within(popover()!).getByText(/· today$/)).toBeTruthy();
    fireEvent.mouseLeave(cell("2026-08-26"));
    hover("2026-08-27");
    expect(within(popover()!).queryByText(/· today$/)).toBeNull();
  });

  it("una fila abre su tarea", () => {
    const onOpen = renderCalendar([item("a", 12, { title: "abrir esto" })]);
    hover("2026-08-12");
    fireEvent.click(within(popover()!).getByText("abrir esto"));
    expect(onOpen).toHaveBeenCalledWith("a");
  });

  it("el pie de nueva tarea sólo sale si se pide, y lleva el día", () => {
    renderCalendar([item("a", 12)]);
    hover("2026-08-12");
    expect(within(popover()!).queryByText("New task for this day")).toBeNull();
    cleanup();
    const onCreateForDay = vi.fn();
    renderCalendar([], { onCreateForDay });
    hover("2026-08-20");
    fireEvent.click(within(popover()!).getByText("New task for this day"));
    expect(onCreateForDay).toHaveBeenCalledWith("2026-08-20");
  });
});

describe("la cabecera", () => {
  it("titula el mes sin «De» y pasa a la semana, recordándolo por calendario", () => {
    renderCalendar([]);
    expect(screen.getByText("August 2026")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "Week" }));
    expect(screen.getByText("24 – 30 August 2026")).toBeTruthy();
    expect(useCalendarStore.getState().views.test).toBe("week");
    expect(useCalendarStore.getState().views.mywork).toBeUndefined();
  });

  it("‹ › mueven el mes, y en la semana siete días", () => {
    renderCalendar([]);
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByText("September 2026")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Today" }));
    fireEvent.click(screen.getByRole("tab", { name: "Week" }));
    fireEvent.click(screen.getByRole("button", { name: "Previous" }));
    expect(screen.getByText("17 – 23 August 2026")).toBeTruthy();
  });
});

/**
 * El día que alguien eligió, se mire desde donde se mire.
 *
 * jose, en Querétaro (UTC−6), soltó una tarea en el 30 de septiembre y se pintó
 * en el 29. Lo guardado estaba bien —`2026-09-30T00:00:00.000Z`—; lo que fallaba
 * era leerlo con captadores locales. Estas pruebas tienen que pasar en
 * cualquier zona (`bun run test:timezones`).
 */
describe("en qué día cae", () => {
  beforeEach(() => vi.setSystemTime(new Date(2026, 8, 15)));

  it("un vencimiento a medianoche UTC cae en su día, no en el anterior", () => {
    renderCalendar(
      dueCalendarItems([{ id: "a", seq: 1, title: "entregar", priority: "none", dueAt: "2026-09-30T00:00:00.000Z" }]),
    );
    expect(cell("2026-09-30").textContent).toContain("1");
    expect(cell("2026-09-29").textContent).toBe("29");
  });

  // Lo contrario también es verdad, y es por lo que no vale «pasarlo todo a
  // UTC»: una reunión es un instante, y cae en el día de quien la mira.
  it("un instante cae en el día local de quien mira", () => {
    const at = "2026-09-30T02:00:00.000Z"; // el 29 a las 20:00 en Querétaro
    renderCalendar([{ id: "r", title: "reunión", at, tone: "bg-primary" }]);
    const local = new Date(at);
    const iso = `2026-09-${String(local.getDate()).padStart(2, "0")}`;
    expect(cell(iso).textContent).toContain("1");
  });

  it("el popover dice el día elegido", () => {
    renderCalendar(
      dueCalendarItems([{ id: "a", seq: 1, title: "entregar", priority: "none", dueAt: "2026-09-30T00:00:00.000Z" }]),
    );
    hover("2026-09-30");
    expect(within(popover()!).getByText("September 30")).toBeTruthy();
  });
});
