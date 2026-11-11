import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";

/**
 * La semana de escritorio (command-and-control-336).
 *
 * El arrastre se prueba capturando lo que el calendario le pasa a dnd-kit, como
 * en `KanbanBoard.rechazo.test.tsx`: arrastrar de verdad probaría a dnd-kit.
 * Lo nuestro es **qué día** recibe cada columna como destino y qué se hace al
 * soltar.
 *
 * Roturas que atrapan: una semana que empieza en domingo o no cruza de mes; el
 * resumen contando sólo las vencidas de esta semana, o la urgente fuera de
 * «alta»; «abiertas» contando las hechas; una reunión en el día UTC; soltar en
 * una columna sin llamar a `onSchedule` con su día; columnas que aceptan
 * soltar donde no se puede cambiar la fecha; y pasar a Mes sin abrir el mes de
 * la semana que se miraba.
 */
const { dnd, droppables } = vi.hoisted(() => ({
  dnd: { onDragStart: null as null | ((e: unknown) => void), onDragEnd: null as null | ((e: unknown) => void) },
  droppables: new Map<string, boolean>(),
}));

vi.mock("@dnd-kit/core", async () => {
  const real = await vi.importActual<typeof import("@dnd-kit/core")>("@dnd-kit/core");
  return {
    ...real,
    DndContext: (props: { onDragStart: (e: unknown) => void; onDragEnd: (e: unknown) => void; children: React.ReactNode }) => {
      dnd.onDragStart = props.onDragStart;
      dnd.onDragEnd = props.onDragEnd;
      return <>{props.children}</>;
    },
    DragOverlay: () => null,
    useDroppable: ({ id, disabled }: { id: string; disabled?: boolean }) => {
      droppables.set(String(id), !disabled);
      return { setNodeRef: () => {}, isOver: false };
    },
    useDraggable: () => ({ attributes: {}, listeners: {}, setNodeRef: () => {}, isDragging: false }),
  };
});

const { default: ItemCalendar } = await import("@/components/ItemCalendar");
const { useCalendarStore } = await import("@/store/calendar.store");
type Item = import("@/components/ItemCalendar").CalendarItem;

const TONES = [
  { key: "urgent", label: "Urgent", dot: "bg-error", edge: "bg-error", high: true },
  { key: "high", label: "High", dot: "bg-warning", edge: "bg-warning", high: true },
  { key: "none", label: "None", dot: "bg-muted-soft", edge: "bg-border-strong" },
];

const columns = () => [...document.querySelectorAll<HTMLElement>("[data-column]")].map((c) => c.dataset.column);
const column = (iso: string) => document.querySelector<HTMLElement>(`[data-column="${iso}"]`)!;

const renderWeek = (items: Item[], onSchedule?: (id: string, iso: string) => void) =>
  render(
    <ItemCalendar
      tones={TONES}
      calendarId="test"
      items={items}
      onOpen={() => {}}
      onSchedule={onSchedule}
      countKey="common:count.tasks"
    />,
  );

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 10, 11)); // miércoles
  useCalendarStore.setState({ views: { test: "week" }, lanes: {} });
  droppables.clear();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("la semana", () => {
  it("empieza en lunes y cruza de mes", () => {
    vi.setSystemTime(new Date(2026, 11, 2));
    renderWeek([]);
    expect(columns()).toEqual(["2026-11-30", "2026-12-01", "2026-12-02", "2026-12-03", "2026-12-04", "2026-12-05", "2026-12-06"]);
    expect(screen.getByText("30 Nov – 6 December 2026")).toBeTruthy();
  });

  it("el resumen: alta incluye la urgente, vencidas cuenta las de cualquier semana, hechas las de ésta", () => {
    renderWeek([
      { id: "a", title: "a", day: "2026-11-10", tone: "urgent" },
      { id: "b", title: "b", day: "2026-11-12", tone: "high", meta: { done: true } },
      { id: "c", title: "c", day: "2026-11-13", tone: "none" },
      { id: "d", title: "d", day: "2026-10-01", tone: "none", meta: { overdue: true } },
      { id: "e", title: "e", day: "2026-11-09", tone: "none", meta: { overdue: true } },
    ]);
    const block = (label: string) => screen.getByText(label).parentElement!;
    expect(block("This week").textContent).toContain("4 tasks");
    expect(block("High priority").textContent).toContain("2");
    expect(block("Overdue and open").textContent).toContain("2");
    expect(block("Done").textContent).toContain("1");
  });

  it("sin metadatos no pinta alta, vencidas ni hechas a cero", () => {
    render(
      <ItemCalendar
        tones={[{ key: "active", label: "Active", dot: "bg-primary", edge: "bg-primary" }]}
        calendarId="test"
        items={[{ id: "m", title: "reunión", at: new Date(2026, 10, 12, 10).toISOString(), tone: "active" }]}
        onOpen={() => {}}
        countKey="common:count.meetings"
      />,
    );
    expect(screen.queryByText("High priority")).toBeNull();
    expect(screen.queryByText("Overdue and open")).toBeNull();
    expect(screen.queryByText("Done")).toBeNull();
  });

  it("cada columna cuenta sus abiertas, y una vacía está libre", () => {
    renderWeek([
      { id: "a", title: "a", day: "2026-11-12", tone: "none", meta: { done: true } },
      { id: "b", title: "b", day: "2026-11-12", tone: "none", meta: {} },
    ]);
    expect(column("2026-11-12").textContent).toContain("1 open");
    expect(within(column("2026-11-14")).getByText("Free")).toBeTruthy();
  });

  it("un instante cae en la columna de su día local", () => {
    const at = "2026-11-12T02:00:00.000Z";
    renderWeek([{ id: "r", title: "reunión", at, tone: "none" }]);
    const local = new Date(at);
    const iso = `2026-11-${String(local.getDate()).padStart(2, "0")}`;
    expect(within(column(iso)).getByText("reunión")).toBeTruthy();
  });

  it("hoy se distingue, y lo de días pasados va atenuado", () => {
    renderWeek([{ id: "p", title: "pasada", day: "2026-11-09", tone: "none" }]);
    expect(column("2026-11-11").className).toContain("bg-today-column");
    expect(column("2026-11-12").className).not.toContain("bg-today-column");
    expect(document.querySelector('[data-card="p"]')!.className).toContain("opacity-75");
  });
});

describe("arrastrar entre días", () => {
  it("cada columna recibe su día, y soltar le pone esa fecha", () => {
    const onSchedule = vi.fn();
    renderWeek([{ id: "a", title: "a", day: "2026-11-10", tone: "none" }], onSchedule);
    act(() => dnd.onDragStart!({ active: { id: "a" } }));
    for (const iso of columns()) expect(droppables.get(iso!)).toBe(true);
    act(() => dnd.onDragEnd!({ active: { id: "a" }, over: { id: "2026-11-13" } }));
    expect(onSchedule).toHaveBeenCalledWith("a", "2026-11-13");
  });

  it("sin onSchedule ninguna columna acepta", () => {
    renderWeek([{ id: "a", title: "a", day: "2026-11-10", tone: "none" }]);
    act(() => dnd.onDragStart!({ active: { id: "a" } }));
    for (const iso of columns()) expect(droppables.get(iso!)).toBe(false);
  });
});

describe("pasar de una vista a otra", () => {
  it("la semana se mueve de siete en siete y al Mes abre el de la semana visible", () => {
    renderWeek([]);
    for (let i = 0; i < 3; i++) fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(columns()[0]).toBe("2026-11-30");
    fireEvent.click(screen.getByRole("tab", { name: "Month" }));
    expect(screen.getByText("November 2026")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "Week" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    fireEvent.click(screen.getByRole("tab", { name: "Month" }));
    expect(screen.getByText("December 2026")).toBeTruthy();
  });
});
