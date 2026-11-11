import type { CalendarTone } from "@/lib/calendar";
import { canonicalPriority } from "@/components/tasks/PriorityChip";

type T = (key: "work:taskPriority.urgent" | "work:taskPriority.high" | "work:taskPriority.normal" | "work:taskPriority.low" | "work:taskPriority.none" | "common:calendar.meetingActive" | "common:calendar.meetingPaused" | "common:calendar.meetingMix") => string;

/**
 * Los tonos de una tarea: su prioridad, de más a menos (command-and-control-333).
 *
 * Cinco y no las tres del diseño: el diseño sólo dibujó las que traían sus
 * datos de ejemplo, y su propia tabla de tokens ya le da `--error` a la
 * urgente. Cada una con el color que ya tiene en el resto de la app
 * (`PriorityChip`); «sin prioridad» con los dos grises que pide el diseño, el
 * suave en el punto y el fuerte en la barra lateral.
 *
 * Las clases van enteras: Tailwind sólo genera las que lee literales.
 */
export function priorityTones(t: T): CalendarTone[] {
  return [
    { key: "urgent", label: t("work:taskPriority.urgent"), dot: "bg-error", edge: "bg-error", high: true },
    { key: "high", label: t("work:taskPriority.high"), dot: "bg-warning", edge: "bg-warning", high: true },
    { key: "normal", label: t("work:taskPriority.normal"), dot: "bg-info", edge: "bg-info" },
    { key: "low", label: t("work:taskPriority.low"), dot: "bg-muted-foreground", edge: "bg-muted-foreground" },
    { key: "none", label: t("work:taskPriority.none"), dot: "bg-muted-soft", edge: "bg-border-strong" },
  ];
}

/** El tono de una prioridad tal como llega (`medium` es `normal`; ver CLAUDE.md). */
export function priorityTone(p: string | undefined): string {
  return canonicalPriority(p) ?? "none";
}

/**
 * Los de una reunión: activa o en pausa. La pausada se pinta apagada en vez de
 * esconderse: una reunión pausada por error es invisible justo donde se
 * buscaría.
 */
export function meetingTones(t: T): CalendarTone[] {
  return [
    { key: "active", label: t("common:calendar.meetingActive"), dot: "bg-primary", edge: "bg-primary" },
    { key: "paused", label: t("common:calendar.meetingPaused"), dot: "bg-muted-foreground/40", edge: "bg-muted-foreground/40" },
  ];
}

/** El título del bloque «Mezcla» del resumen de la semana, para reuniones. */
export const meetingMixLabel = (t: T) => t("common:calendar.meetingMix");
