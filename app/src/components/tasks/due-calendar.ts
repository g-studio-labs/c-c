import type { CalendarEntry, CalendarItem, CalendarMeta } from "@/components/ItemCalendar";
import { priorityTone } from "@/components/calendar/tones";
import { toISODate, dueDay } from "@/lib/month";
import { iniciales } from "@/lib/desde";
import { statusTone } from "@/lib/status-color";
import { normalizeStatus } from "@/types/report";
import type { OpenTask } from "@/types/task";

type Dated = Pick<OpenTask, "id" | "seq" | "title" | "priority" | "dueAt"> &
  Partial<Pick<OpenTask, "status" | "statusName" | "assignee">>;

/** Terminada: lo que el calendario tacha y no cuenta como vencido. */
const FINISHED = ["done", "closed"];

/**
 * Lo que una tarea enseña en el calendario además del título: su estado, quién
 * la lleva, si está hecha y si venció sin cerrarse (command-and-control-333).
 *
 * Vencida se mide contra el **día** de hoy, no contra el instante: una tarea
 * que vence hoy no está vencida hasta mañana.
 */
function metaOf(t: Dated, today: Date): CalendarMeta {
  const done = !!t.status && FINISHED.includes(normalizeStatus(t.status));
  const due = dueDay(t.dueAt);
  return {
    status: t.status ? { name: t.statusName ?? t.status, dot: statusTone(t.status).dot } : undefined,
    assignee: t.assignee ? { initials: iniciales(t.assignee), key: t.assignee } : undefined,
    done,
    overdue: !done && !!due && due.getTime() < today.getTime(),
  };
}

const startOfToday = () => {
  const n = new Date();
  return new Date(n.getFullYear(), n.getMonth(), n.getDate());
};

/** Una tarea como elemento del calendario, sin día: lo que va al carril. */
export function taskEntry(t: Dated, today = startOfToday()): CalendarEntry {
  return { id: t.id, title: t.title, tone: priorityTone(t.priority), label: `#${t.seq}`, meta: metaOf(t, today) };
}

/**
 * Las tareas de «Mi trabajo», colocadas en el día en que vencen.
 *
 * Fuera de la página para poder montarlas en el calendario de verdad desde una
 * prueba: el fallo del 29-por-30 vivía justo en la costura entre las dos, y
 * cada prueba de la página sustituye el calendario por nada.
 *
 * Un vencimiento es un **día**, no un instante: va como `day`, ya resuelto por
 * `dueDay`. Pasarlo como `at` compila —es un `string`— y lo pinta el
 * día anterior en toda zona al oeste de Greenwich.
 */
export function dueCalendarItems(tasks: Dated[]): CalendarItem[] {
  const today = startOfToday();
  return tasks.flatMap((t) => {
    const due = dueDay(t.dueAt);
    if (!due) return [];
    return [{ ...taskEntry(t, today), day: toISODate(due) }];
  });
}
