import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

/**
 * El calendario del teléfono (command-and-control-339). Sin hover ni arrastre:
 * tocar un día sube una hoja y el mes no se mueve (jose, 18-oct-2026); la
 * semana es una tira de siete días que se desliza; lo que no tiene fecha se
 * programa con atajos y un «Deshacer».
 *
 * Roturas que atrapan: la lista del día debajo del mes en vez de en una hoja;
 * una barra de carga sin tope; «Próximas» con lo hecho o lo de ayer; el
 * deslizar que no cambia de día o lo cambia con un temblor; «Lunes» que no sea
 * el próximo lunes; programar sin ofrecer deshacer, o un deshacer que no quita
 * la fecha; y la hoja que sigue abierta al abrir una tarea.
 */
const { toastCalls } = vi.hoisted(() => ({ toastCalls: [] as { text: string; opts: { action?: { onClick: () => void } } }[] }));
vi.mock("sonner", async () => {
  const real = await vi.importActual<typeof import("sonner")>("sonner");
  const fn = Object.assign((text: string, opts: never) => toastCalls.push({ text, opts }), real.toast);
  return { ...real, toast: fn };
});

const { default: ItemCalendar } = await import("@/components/ItemCalendar");
const { loadWidth } = await import("@/components/calendar/MobileCalendar");
const { useCalendarStore } = await import("@/store/calendar.store");
type Item = import("@/components/ItemCalendar").CalendarItem;

const TONES = [
  { key: "high", label: "High", dot: "bg-warning", edge: "bg-warning", high: true },
  { key: "none", label: "None", dot: "bg-muted-soft", edge: "bg-border-strong" },
];

const wide = window.innerWidth;
beforeEach(() => {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 390 });
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 10, 11, 12)); // miércoles 11 de noviembre
  useCalendarStore.setState({ views: {}, lanes: {} });
  toastCalls.length = 0;
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  Object.defineProperty(window, "innerWidth", { configurable: true, value: wide });
});

const items: Item[] = [
  { id: "a", title: "Revisar el backup", day: "2026-11-11", tone: "high", label: "#26", meta: { status: { name: "Open", dot: "bg-status-open" } } },
  { id: "b", title: "Logo nuevo", day: "2026-11-13", tone: "none", label: "#3", meta: {} },
  { id: "h", title: "Ya hecha", day: "2026-11-12", tone: "none", label: "#4", meta: { done: true } },
  { id: "y", title: "De ayer", day: "2026-11-10", tone: "none", label: "#5", meta: { overdue: true } },
];

const renderPhone = (props: Partial<Parameters<typeof ItemCalendar>[0]> = {}) => {
  const fns = { onOpen: vi.fn(), onSchedule: vi.fn(), onUnschedule: vi.fn(), onCreateForDay: vi.fn() };
  render(
    <ItemCalendar
      tones={TONES}
      calendarId="test"
      items={items}
      undated={[{ id: "u", title: "Sin fecha aún", tone: "none", label: "#203" }]}
      countKey="common:count.tasks"
      {...fns}
      {...props}
    />,
  );
  return fns;
};
const day = (iso: string) => document.querySelector<HTMLElement>(`[data-day="${iso}"]`)!;

describe("el mes del teléfono", () => {
  it("tocar un día abre su hoja y el mes sigue en su sitio", () => {
    renderPhone();
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(day("2026-11-13"));
    const sheet = screen.getByRole("dialog");
    expect(within(sheet).getByText("Logo nuevo")).toBeTruthy();
    expect(day("2026-11-13")).not.toBeNull();
  });

  it("abrir una tarea desde la hoja la cierra", () => {
    const fns = renderPhone();
    fireEvent.click(day("2026-11-13"));
    fireEvent.click(within(screen.getByRole("dialog")).getByText("Logo nuevo"));
    expect(fns.onOpen).toHaveBeenCalledWith("b");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("la hoja ofrece crear para ese día", () => {
    const fns = renderPhone();
    fireEvent.click(day("2026-11-20"));
    fireEvent.click(within(screen.getByRole("dialog")).getByText("New task for this day"));
    expect(fns.onCreateForDay).toHaveBeenCalledWith("2026-11-20");
  });

  it("la barra de carga crece con las tareas, con tope en cinco", () => {
    expect([0, 1, 3, 5, 9].map(loadWidth)).toEqual([0, 9, 19, 29, 29]);
  });

  it("el punto de vencidas, y cuántas hay arriba", () => {
    renderPhone();
    expect(day("2026-11-10").querySelector("[data-overdue]")).not.toBeNull();
    expect(day("2026-11-13").querySelector("[data-overdue]")).toBeNull();
    expect(screen.getByText("1 overdue")).toBeTruthy();
  });

  it("Próximas: desde hoy, sin lo hecho, en orden", () => {
    renderPhone();
    const section = screen.getByText("Upcoming").parentElement!;
    const titles = [...section.querySelectorAll("button")].map((b) => b.textContent);
    expect(titles[0]).toContain("Revisar el backup");
    expect(titles[1]).toContain("Logo nuevo");
    expect(titles.join()).not.toContain("Ya hecha");
    expect(titles.join()).not.toContain("De ayer");
  });
});

describe("la semana del teléfono", () => {
  beforeEach(() => useCalendarStore.setState({ views: { test: "week" }, lanes: {} }));

  it("enseña el día elegido, y deslizar cambia de día pero un temblor no", () => {
    renderPhone();
    expect(screen.getByText("Revisar el backup")).toBeTruthy();
    const swipe = document.querySelector<HTMLElement>("[data-swipe]")!;
    fireEvent.touchStart(swipe, { touches: [{ clientX: 200 }] });
    fireEvent.touchEnd(swipe, { changedTouches: [{ clientX: 170 }] });
    expect(document.querySelector('[data-strip="2026-11-11"]')!.getAttribute("aria-pressed")).toBe("true");
    fireEvent.touchStart(swipe, { touches: [{ clientX: 200 }] });
    fireEvent.touchEnd(swipe, { changedTouches: [{ clientX: 100 }] });
    expect(document.querySelector('[data-strip="2026-11-12"]')!.getAttribute("aria-pressed")).toBe("true");
  });

  it("las flechas mueven la semana entera", () => {
    renderPhone();
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(document.querySelector('[data-strip="2026-11-18"]')!.getAttribute("aria-pressed")).toBe("true");
  });
});

describe("la hoja Por programar", () => {
  const openLane = () => fireEvent.click(screen.getByRole("button", { name: "To schedule" }));

  it("«Lunes» es el próximo lunes, y programar deja un Deshacer que quita la fecha", () => {
    const fns = renderPhone();
    openLane();
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Monday" }));
    expect(fns.onSchedule).toHaveBeenCalledWith("u", "2026-11-16");
    expect(toastCalls[0].text).toContain("#203 →");
    toastCalls[0].opts.action!.onClick();
    expect(fns.onUnschedule).toHaveBeenCalledWith("u");
  });

  it("«Hoy» y «Mañana», en días locales", () => {
    const fns = renderPhone();
    openLane();
    const sheet = screen.getByRole("dialog");
    fireEvent.click(within(sheet).getByRole("button", { name: "Today" }));
    expect(fns.onSchedule).toHaveBeenLastCalledWith("u", "2026-11-11");
    fireEvent.click(within(sheet).getByRole("button", { name: "Tomorrow" }));
    expect(fns.onSchedule).toHaveBeenLastCalledWith("u", "2026-11-12");
  });

  it("sin carril no hay botón ni banner", () => {
    renderPhone({ onSchedule: undefined });
    expect(screen.queryByRole("button", { name: "To schedule" })).toBeNull();
  });
});
