import { create } from "zustand";
import { persist } from "zustand/middleware";

/**
 * Cómo quiere ver cada calendario quien lo mira: Mes o Semana, y si el carril
 * «Por programar» está abierto en cada una (command-and-control-333).
 *
 * Por calendario (`calendarId`: «mywork», «board», «meetings») y no uno para
 * todos: quien planifica su semana en Mi trabajo no por eso quiere ver las
 * reuniones por semana.
 *
 * El carril, además, por vista: el diseño lo quiere cerrado en Mes —el mes
 * necesita el ancho— y abierto en Semana. Una sola preferencia obligaría a
 * elegir entre las dos.
 */
export type CalendarView = "month" | "week";

const LANE_DEFAULT: Record<CalendarView, boolean> = { month: false, week: true };

interface CalendarState {
  views: Record<string, CalendarView>;
  lanes: Record<string, Partial<Record<CalendarView, boolean>>>;
  viewOf: (id: string) => CalendarView;
  setView: (id: string, v: CalendarView) => void;
  laneOpen: (id: string, v: CalendarView) => boolean;
  setLaneOpen: (id: string, v: CalendarView, open: boolean) => void;
}

export const useCalendarStore = create<CalendarState>()(
  persist(
    (set, get) => ({
      views: {},
      lanes: {},
      viewOf: (id) => get().views[id] ?? "month",
      setView: (id, v) => set((s) => ({ views: { ...s.views, [id]: v } })),
      laneOpen: (id, v) => get().lanes[id]?.[v] ?? LANE_DEFAULT[v],
      setLaneOpen: (id, v, open) =>
        set((s) => ({ lanes: { ...s.lanes, [id]: { ...s.lanes[id], [v]: open } } })),
    }),
    { name: "cac-calendar", partialize: (s) => ({ views: s.views, lanes: s.lanes }) },
  ),
);
