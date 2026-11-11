import { useDroppable } from "@dnd-kit/core";
import { ChevronLeft, GripVertical } from "lucide-react";

import type { CalendarEntry } from "@/components/ItemCalendar";
import CalendarCard from "@/components/calendar/CalendarCard";
import type { CalendarTone } from "@/lib/calendar";
import { useT } from "@/lib/i18n";
import { toISODate } from "@/lib/month";
import { cn } from "@/lib/utils";

/** El id del carril como destino de arrastre: soltar aquí quita la fecha. */
export const LANE_ID = "lane";

/**
 * «Por programar» (command-and-control-337): lo que no tiene fecha, a mano y
 * en un lado, para dársela arrastrando o con un atajo. Antes era una tira al
 * final del mes que nadie miraba, y lo que no tiene fecha no sale en ningún
 * día. Medidas de `docs/design/calendario.md` §«Por programar».
 *
 * El ancho se anima de 220 a 0 (y el margen de 0 a −12, para comerse el hueco
 * de la fila): cerrado sigue montado, para que abrir y cerrar se deslice.
 */
export default function ScheduleLane({
  open,
  items,
  tones,
  dragging,
  canUnschedule,
  onClose,
  onOpen,
  onSchedule,
}: {
  open: boolean;
  items: CalendarEntry[];
  tones: CalendarTone[];
  dragging: boolean;
  /** Soltar aquí una con fecha se la quita; sin esto el carril no la acepta. */
  canUnschedule: boolean;
  onClose: () => void;
  onOpen: (id: string) => void;
  onSchedule: (id: string, iso: string) => void;
}) {
  const { t } = useT();
  const { setNodeRef, isOver } = useDroppable({ id: LANE_ID, disabled: !(open && dragging && canUnschedule) });
  const hot = open && dragging && canUnschedule && isOver;
  // Hoy y mañana como días **locales**, sin pasar por `new Date(iso)`.
  const day = (offset: number) => {
    const n = new Date();
    return toISODate(new Date(n.getFullYear(), n.getMonth(), n.getDate() + offset));
  };
  return (
    <div
      aria-hidden={!open}
      className={cn(
        "sticky top-3 flex-none self-start overflow-hidden transition-[width,margin-right] duration-[220ms] ease-[ease]",
        open ? "mr-0 w-[220px]" : "-mr-3 w-0",
      )}
    >
      <div
        ref={setNodeRef}
        data-lane
        className={cn(
          "flex max-h-[calc(100vh-24px)] w-[220px] flex-col gap-[6px] overflow-auto rounded-[10px] border border-dashed p-2",
          hot ? "border-primary bg-primary/8" : "border-primary/40 bg-primary/3",
        )}
      >
        <div className="flex items-center gap-[6px] border-b border-border px-[2px] pt-[2px] pb-[6px]">
          <span className="text-[12px] font-semibold text-primary">{t("common:calendar.lane")}</span>
          <span className="rounded-full bg-track px-[6px] text-[11px] text-muted-foreground">{items.length}</span>
          <button
            type="button"
            title={t("common:calendar.hide")}
            aria-label={t("common:calendar.hideLane")}
            onClick={onClose}
            tabIndex={open ? 0 : -1}
            className="motion-press ml-auto grid size-[22px] place-items-center rounded-[5px] text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <ChevronLeft className="size-[14px]" />
          </button>
        </div>
        {items.map((it) => (
          <CalendarCard
            key={it.id}
            item={it}
            tones={tones}
            variant="lane"
            draggable={open}
            onOpen={() => onOpen(it.id)}
            grip={<GripVertical aria-hidden className="size-3 text-muted-foreground" />}
            footer={
              // Los clics de los atajos no abren la tarjeta.
              <div className="flex gap-1" onClick={(e) => e.stopPropagation()} onPointerDown={(e) => e.stopPropagation()}>
                <button
                  type="button"
                  onClick={() => onSchedule(it.id, day(0))}
                  className="motion-press rounded-[5px] border border-border px-2 py-[2px] text-[11px] hover:border-primary hover:bg-primary hover:text-primary-foreground"
                >
                  → {t("common:calendar.today")}
                </button>
                <button
                  type="button"
                  onClick={() => onSchedule(it.id, day(1))}
                  className="motion-press rounded-[5px] border border-border px-2 py-[2px] text-[11px] hover:bg-accent"
                >
                  {t("common:calendar.tomorrow")}
                </button>
              </div>
            }
          />
        ))}
        {items.length === 0 && (
          <div className="px-[2px] py-[10px] text-[12px] text-success">{t("common:calendar.allDated")}</div>
        )}
        <span className="px-[2px] pt-1 pb-[2px] text-[11px] leading-[15px] text-muted-soft">
          {t("common:calendar.laneHelp")}
        </span>
      </div>
    </div>
  );
}
