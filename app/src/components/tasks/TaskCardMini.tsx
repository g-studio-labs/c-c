import PriorityChip from "@/components/tasks/PriorityChip";
import { diaYMes } from "@/lib/fechas";
import i18next from "@/lib/i18n";
import { dueDay } from "@/lib/month";
import { orgZone, todayIn, zoneHint } from "@/lib/due";
import type { OpenTask } from "@/types/task";
import { cn } from "@/lib/utils";
import Initials from "@/components/ui/initials";

/**
 * One card, as it appears on a board that crosses lists.
 *
 * It carries the path because a board of "my work" mixes six lists, and a
 * title on its own does not say whose problem it is. And it carries the due
 * date in words: `2026-08-16` needs arithmetic to read, while "today" and
 * "yesterday" are the two answers that change what you do next.
 */

/**
 * Today, yesterday, or a short date. Overdue is the caller's to colour —
 * `dueClass` does it the same way everywhere.
 *
 * `hoy` vence hoy: todavía no está vencida —el día no ha acabado—, así que va
 * en ámbar y con peso, no en rojo (jose, 13-nov-2026; hasta entonces hoy ya
 * contaba como vencida aquí y no en el calendario). `pronto` es mañana, en
 * ámbar sin peso.
 *
 * «Hoy» es el de la zona de la organización, no el de quien mira: ver
 * `lib/due.ts`.
 */
export interface DueState {
  texto: string;
  vencida: boolean;
  hoy: boolean;
  pronto: boolean;
}

export function cuando(iso?: string | null, zone: string | undefined = orgZone(), now: Date = new Date()): DueState {
  const none = { texto: "", vencida: false, hoy: false, pronto: false };
  const d = dueDay(iso);
  if (!d) return none;
  const dias = Math.round((d.getTime() - todayIn(zone, now).getTime()) / 86_400_000);
  if (dias === 0) return { ...none, texto: i18next.t("work:due.today"), hoy: true };
  if (dias === -1) return { ...none, texto: i18next.t("work:due.yesterday"), vencida: true };
  if (dias < -1) return { ...none, texto: diaYMes(d), vencida: true };
  if (dias === 1) return { ...none, texto: i18next.t("work:due.tomorrow"), pronto: true };
  return { ...none, texto: diaYMes(d) };
}

/** El color de un vencimiento: rojo si ya pasó, ámbar si es hoy o mañana. */
export function dueClass(v: Pick<DueState, "vencida" | "pronto"> & { hoy?: boolean }): string {
  if (v.vencida) return "font-medium text-error";
  if (v.hoy) return "font-medium text-warning";
  return v.pronto ? "text-warning" : "text-muted-foreground";
}

/**
 * Lo que sale al pasar el cursor por un vencimiento, para quien no está en la
 * hora de la organización: en una tarjeta no cabe escrito.
 */
export function dueTitle(): string | undefined {
  return zoneHint() || undefined;
}

/**
 * De qué tarea es una subtarea, para la ruta de una tarjeta: «› #289 Título».
 * Una subtarea sale en «Mi trabajo» cuando es de alguien aunque su madre no lo
 * sea, y sin esto parecía una tarea suelta (command-and-control-297).
 */
export function parentCrumb(task: Pick<OpenTask, "parentSeq" | "parentTitle">): string {
  return task.parentSeq ? ` › #${task.parentSeq} ${task.parentTitle ?? ""}`.trimEnd() : "";
}

export default function TaskCardMini({ task, onOpen }: { task: OpenTask; onOpen: () => void }) {
  const vence = cuando(task.dueAt);
  return (
    <button
      onClick={onOpen}
      className="motion-press lift w-full rounded-lg border bg-raised px-2.5 py-2 text-left shadow-sm hover:border-primary/40"
    >
      <div className="flex items-baseline gap-1.5 text-[11px] text-muted-foreground">
        <span className="font-mono shrink-0">#{task.seq}</span>
        <span className="truncate">
          {task.spaceName} / {task.listName}
          {parentCrumb(task)}
        </span>
      </div>
      <p className="mt-0.5 line-clamp-2 text-sm">{task.title}</p>
      <div className="mt-1.5 flex items-center gap-2 text-[11px]">
        <PriorityChip priority={task.priority} />
        {task.subtasksTotal > 0 && (
          <span className="shrink-0 font-mono text-muted-foreground">
            {task.subtasksDone}/{task.subtasksTotal}
          </span>
        )}
        <span className={cn("shrink-0", dueClass(vence))} title={dueTitle()}>
          {vence.texto || "—"}
        </span>
        {task.assignee && (
          <Initials
            colorKey={task.assignee}
            title={task.assignee}
            text={task.assignee.slice(0, 2)}
            className="ml-auto size-5 text-[9px]"
          />
        )}
      </div>
    </button>
  );
}
