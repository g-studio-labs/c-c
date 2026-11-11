import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";

/**
 * «Por programar» (command-and-control-337).
 *
 * Roturas que atrapan: el carril donde no se puede poner fecha (tablero,
 * reuniones); abierto en Mes o cerrado en Semana de entrada; la preferencia
 * compartida entre vistas o entre calendarios; soltar en el carril sin quitar
 * la fecha, o quitársela a una que no la tenía; el carril aceptando sin
 * `onUnschedule`; «Mañana» que no sea el día siguiente **local** (fin de mes,
 * en las tres zonas de `test:timezones`); y un atajo que además abre la
 * tarjeta.
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
type Props = Parameters<typeof ItemCalendar>[0];

const TONES = [{ key: "none", label: "None", dot: "bg-muted-soft", edge: "bg-border-strong" }];
const lane = () => document.querySelector<HTMLElement>("[data-lane]");
const laneOpen = () => !!lane() && lane()!.parentElement!.getAttribute("aria-hidden") === "false";
const toggle = () => screen.getByRole("button", { name: /To schedule/ });

const renderCal = (props: Partial<Props> = {}) => {
  const fns = { onSchedule: vi.fn(), onUnschedule: vi.fn(), onOpen: vi.fn() };
  render(
    <ItemCalendar
      tones={TONES}
      calendarId="test"
      items={[{ id: "dated", title: "con fecha", day: "2026-11-12", tone: "none" }]}
      undated={[{ id: "u1", title: "sin fecha", tone: "none", label: "#1" }]}
      countKey="common:count.tasks"
      {...fns}
      {...props}
    />,
  );
  return fns;
};

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 10, 30, 12)); // 30 de noviembre: mañana es otro mes
  useCalendarStore.setState({ views: {}, lanes: {} });
  droppables.clear();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("dónde hay carril", () => {
  it("sólo con lo sin fecha y onSchedule", () => {
    renderCal({ onSchedule: undefined });
    expect(lane()).toBeNull();
    expect(screen.queryByRole("button", { name: /To schedule/ })).toBeNull();
  });

  it("cerrado en Mes y abierto en Semana, recordado por vista y por calendario", () => {
    renderCal();
    expect(laneOpen()).toBe(false);
    fireEvent.click(toggle());
    expect(laneOpen()).toBe(true);
    fireEvent.click(screen.getByRole("tab", { name: "Week" }));
    expect(laneOpen()).toBe(true);
    fireEvent.click(toggle());
    expect(laneOpen()).toBe(false);
    fireEvent.click(screen.getByRole("tab", { name: "Month" }));
    expect(laneOpen()).toBe(true);
    expect(useCalendarStore.getState().lanes.mywork).toBeUndefined();
  });

  it("vacío dice que todo tiene fecha", () => {
    useCalendarStore.setState({ views: {}, lanes: { test: { month: true } } });
    renderCal({ undated: [] });
    expect(within(lane()!).getByText("Everything has a date.")).toBeTruthy();
  });
});

describe("soltar en el carril", () => {
  beforeEach(() => useCalendarStore.setState({ views: {}, lanes: { test: { month: true } } }));

  it("le quita la fecha a la que la tenía", () => {
    const fns = renderCal();
    act(() => dnd.onDragStart!({ active: { id: "dated" } }));
    expect(droppables.get("lane")).toBe(true);
    act(() => dnd.onDragEnd!({ active: { id: "dated" }, over: { id: "lane" } }));
    expect(fns.onUnschedule).toHaveBeenCalledWith("dated");
    expect(fns.onSchedule).not.toHaveBeenCalled();
  });

  it("y a una que ya estaba en el carril no le hace nada", () => {
    const fns = renderCal();
    act(() => dnd.onDragEnd!({ active: { id: "u1" }, over: { id: "lane" } }));
    expect(fns.onUnschedule).not.toHaveBeenCalled();
    expect(fns.onSchedule).not.toHaveBeenCalled();
  });

  it("sin onUnschedule el carril no acepta", () => {
    renderCal({ onUnschedule: undefined });
    act(() => dnd.onDragStart!({ active: { id: "dated" } }));
    expect(droppables.get("lane")).toBe(false);
  });

  it("de un día del carril a un día, le pone ese día", () => {
    const fns = renderCal();
    act(() => dnd.onDragEnd!({ active: { id: "u1" }, over: { id: "2026-11-18" } }));
    expect(fns.onSchedule).toHaveBeenCalledWith("u1", "2026-11-18");
  });
});

describe("los atajos", () => {
  beforeEach(() => useCalendarStore.setState({ views: {}, lanes: { test: { month: true } } }));

  // A primera y a última hora: un día calculado en UTC sale bien a mediodía en
  // casi todas las zonas, y mal en uno de los dos extremos según la zona.
  it.each([
    [0, 30],
    [23, 30],
  ])("«→ Hoy» y «Mañana» mandan el día local a las %i:%i, también a fin de mes, sin abrir la tarjeta", (h, m) => {
    vi.setSystemTime(new Date(2026, 10, 30, h, m));
    const fns = renderCal();
    fireEvent.click(within(lane()!).getByRole("button", { name: "→ Today" }));
    expect(fns.onSchedule).toHaveBeenLastCalledWith("u1", "2026-11-30");
    fireEvent.click(within(lane()!).getByRole("button", { name: "Tomorrow" }));
    expect(fns.onSchedule).toHaveBeenLastCalledWith("u1", "2026-12-01");
    expect(fns.onOpen).not.toHaveBeenCalled();
  });

  it("la tarjeta se abre con un clic", () => {
    const fns = renderCal();
    fireEvent.click(within(lane()!).getByText("sin fecha"));
    expect(fns.onOpen).toHaveBeenCalledWith("u1");
  });
});
