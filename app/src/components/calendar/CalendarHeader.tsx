import { ChevronLeft, ChevronRight, PanelLeft } from "lucide-react";

import type { CalendarTone } from "@/lib/calendar";
import type { CalendarView } from "@/store/calendar.store";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";

/**
 * La cabecera del calendario de escritorio (command-and-control-335): título,
 * «Por programar», Mes / Semana, leyenda y ‹ Hoy ›. Medidas de
 * `docs/design/calendario.md` §«Encabezado de la vista».
 */
export default function CalendarHeader({
  title,
  view,
  onView,
  tones,
  lane,
  onPrev,
  onToday,
  onNext,
}: {
  title: string;
  view: CalendarView;
  onView: (v: CalendarView) => void;
  tones: CalendarTone[];
  /** El botón del carril; sin él no se pinta (un calendario sin «Por programar»). */
  lane?: { open: boolean; count: number; onToggle: () => void };
  onPrev: () => void;
  onToday: () => void;
  onNext: () => void;
}) {
  const { t } = useT();
  const navBox =
    "motion-press grid size-7 place-items-center rounded-[6px] border border-border hover:bg-accent";
  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex flex-wrap items-center gap-[14px]">
        <span className="min-w-[190px] text-[17px] font-semibold">{title}</span>
        {lane && (
          <button
            type="button"
            onClick={lane.onToggle}
            aria-pressed={lane.open}
            title={lane.open ? t("common:calendar.hideLane") : t("common:calendar.showLane")}
            className={cn(
              "motion-press flex h-7 items-center gap-[7px] rounded-[6px] border pr-[10px] pl-2 text-[13px] text-foreground hover:bg-accent",
              lane.open ? "border-border-strong bg-accent" : "border-border bg-transparent",
            )}
          >
            <PanelLeft className="size-4" />
            {t("common:calendar.lane")}
            <span
              className={cn(
                "grid h-[18px] min-w-[18px] place-items-center rounded-full px-[5px] text-[11px] font-bold",
                lane.open ? "bg-border text-foreground" : "bg-primary text-primary-foreground",
              )}
            >
              {lane.count}
            </span>
          </button>
        )}
        <div role="tablist" className="flex gap-[2px] rounded-[8px] border border-border bg-muted p-[2px] text-[13px]">
          {(["month", "week"] as const).map((v) => (
            <button
              key={v}
              type="button"
              role="tab"
              aria-selected={view === v}
              onClick={() => onView(v)}
              className={cn(
                "motion-press rounded-[6px] px-3 py-[3px]",
                view === v ? "bg-accent font-semibold text-foreground" : "font-normal text-muted-foreground",
              )}
            >
              {v === "month" ? t("common:calendar.month") : t("common:calendar.week")}
            </button>
          ))}
        </div>
      </div>
      <div className="flex items-center gap-[14px]">
        {tones.length > 1 && (
          <div className="flex gap-[10px] text-[12px] text-muted-foreground">
            {tones.map((tone) => (
              <span key={tone.key} className="flex items-center gap-[5px]">
                <span className={cn("size-2 rounded-full", tone.dot)} />
                {tone.label}
              </span>
            ))}
          </div>
        )}
        <div className="flex gap-1">
          <button type="button" aria-label={t("common:calendar.previous")} onClick={onPrev} className={navBox}>
            <ChevronLeft className="size-4" />
          </button>
          <button
            type="button"
            onClick={onToday}
            className="motion-press h-7 rounded-[6px] border border-border px-[10px] text-[13px] font-semibold hover:bg-accent"
          >
            {t("common:calendar.today")}
          </button>
          <button type="button" aria-label={t("common:calendar.next")} onClick={onNext} className={navBox}>
            <ChevronRight className="size-4" />
          </button>
        </div>
      </div>
    </div>
  );
}
