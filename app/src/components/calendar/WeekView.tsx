import { useDroppable } from "@dnd-kit/core";

import type { CalendarItem } from "@/components/ItemCalendar";
import CalendarCard from "@/components/calendar/CalendarCard";
import { ToneBar } from "@/components/calendar/parts";
import { toneSegments, type CalendarTone } from "@/lib/calendar";
import { paraFormatear } from "@/lib/fechas";
import { toISODate } from "@/lib/month";
import { useT, type MessageKey } from "@/lib/i18n";
import { cn } from "@/lib/utils";

/**
 * La semana de escritorio (command-and-control-336): un resumen arriba y una
 * columna por día con las tarjetas enteras. Medidas de
 * `docs/design/calendario.md` §«Vista Semana».
 *
 * El resumen sólo enseña lo que los datos pueden decir: «alta», «vencidas» y
 * «hechas» salen de los tonos marcados `high` y de `meta`, y una reunión no
 * trae ninguno de los dos. Lo que no aplica no se pinta, en vez de pintarse a
 * cero, que se leería como «no hay».
 */
export function WeekSummary({
  week,
  all,
  tones,
  countKey,
  mixLabel,
}: {
  /** Lo de esta semana. */
  week: CalendarItem[];
  /** Todo lo que hay: «vencidas sin cerrar» cuenta las de cualquier semana. */
  all: CalendarItem[];
  tones: CalendarTone[];
  countKey: MessageKey;
  mixLabel: string;
}) {
  const { t } = useT();
  const highKeys = tones.filter((x) => x.high).map((x) => x.key);
  const hasMeta = all.some((x) => x.meta);
  const blocks: { label: string; value: string | number; tone?: string }[] = [
    { label: t("common:calendar.thisWeek"), value: t(countKey, { count: week.length }) },
  ];
  if (highKeys.length) {
    blocks.push({ label: t("common:calendar.highPriority"), value: week.filter((x) => highKeys.includes(x.tone)).length, tone: "text-warning" });
  }
  if (hasMeta) {
    blocks.push({ label: t("common:calendar.overdueOpen"), value: all.filter((x) => x.meta?.overdue).length, tone: "text-error" });
    blocks.push({ label: t("common:calendar.done"), value: week.filter((x) => x.meta?.done).length, tone: "text-success" });
  }
  const segments = toneSegments(week, tones);
  return (
    <div
      className="grid gap-px overflow-hidden rounded-[10px] border border-border bg-border"
      style={{ gridTemplateColumns: `repeat(${blocks.length},minmax(0,1fr)) minmax(0,2fr)` }}
    >
      {blocks.map((b) => (
        <div key={b.label} className="flex flex-col gap-[2px] bg-muted px-[14px] py-[10px]">
          <span className="text-[11px] text-muted-foreground">{b.label}</span>
          <span className={cn("text-[20px] font-semibold", b.tone)}>{b.value}</span>
        </div>
      ))}
      <div className="flex flex-col justify-center gap-2 bg-muted px-[14px] py-[10px]">
        <span className="text-[11px] text-muted-foreground">{mixLabel}</span>
        <ToneBar segments={segments} height={6} track />
        <div className="flex flex-wrap gap-[10px]">
          {segments.map((s) => (
            <span key={s.tone.key} className="flex items-center gap-1 text-[11px] text-muted-foreground">
              <span className={cn("size-[6px] rounded-full", s.tone.dot)} />
              {s.count} {s.tone.label.toLowerCase()}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}

export default function WeekView({
  days,
  byDay,
  tones,
  today,
  dragging,
  canDrop,
  onOpen,
}: {
  days: Date[];
  byDay: Map<string, CalendarItem[]>;
  tones: CalendarTone[];
  today: string;
  dragging: boolean;
  canDrop: boolean;
  onOpen: (id: string) => void;
}) {
  return (
    // Fin de semana angosto: lun–vie a 1fr, sáb y dom a .62fr.
    <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(5,minmax(0,1fr)) repeat(2,minmax(0,.62fr))" }}>
      {days.map((d) => {
        const iso = toISODate(d);
        return (
          <DayColumn
            key={iso}
            date={d}
            iso={iso}
            items={byDay.get(iso) ?? []}
            tones={tones}
            isToday={iso === today}
            isPast={iso < today}
            acceptsDrop={canDrop && dragging}
            canDrag={canDrop}
            onOpen={onOpen}
          />
        );
      })}
    </div>
  );
}

function DayColumn({
  date,
  iso,
  items,
  tones,
  isToday,
  isPast,
  acceptsDrop,
  canDrag,
  onOpen,
}: {
  date: Date;
  iso: string;
  items: CalendarItem[];
  tones: CalendarTone[];
  isToday: boolean;
  isPast: boolean;
  acceptsDrop: boolean;
  canDrag: boolean;
  onOpen: (id: string) => void;
}) {
  const { t } = useT();
  const { setNodeRef, isOver } = useDroppable({ id: iso, disabled: !acceptsDrop });
  const over = acceptsDrop && isOver;
  const weekday = new Intl.DateTimeFormat(paraFormatear(), { weekday: "short" }).format(date).replace(/\.$/, "");
  // «N abiertas» sólo si los datos saben qué está hecho; si no, el conteo.
  const hasMeta = items.some((x) => x.meta);
  const open = items.filter((x) => !x.meta?.done).length;
  return (
    <div
      ref={setNodeRef}
      data-column={iso}
      className={cn(
        "flex min-h-[480px] min-w-0 flex-col gap-[6px] rounded-[10px] border p-2",
        over
          ? "border-primary bg-primary/8"
          : isToday
            ? "border-primary/45 bg-today-column"
            : "border-border bg-muted",
      )}
    >
      <div className="flex flex-col gap-[6px] border-b border-border px-[2px] pt-[2px] pb-2">
        <div className="flex items-center gap-[6px]">
          <span className="text-[12px] tracking-[.05em] text-muted-foreground uppercase">{weekday}</span>
          <span
            className={cn(
              "grid h-6 min-w-6 place-items-center rounded-full px-1 text-[14px] font-semibold",
              isToday ? "bg-primary text-primary-foreground" : isPast ? "text-muted-foreground" : "text-foreground",
            )}
          >
            {date.getDate()}
          </span>
          {items.length > 0 && (
            <span className="ml-auto text-[11px] whitespace-nowrap text-muted-foreground">
              {hasMeta ? t("common:calendar.openCount", { count: open }) : items.length}
            </span>
          )}
        </div>
        <ToneBar segments={toneSegments(items, tones)} height={4} track />
      </div>
      {items.map((it) => (
        <CalendarCard
          key={it.id}
          item={it}
          tones={tones}
          draggable={canDrag}
          dimmed={isPast}
          onOpen={() => onOpen(it.id)}
        />
      ))}
      {items.length === 0 && (
        <div className="grid min-h-[80px] place-items-center rounded-[7px] border border-dashed border-border text-[12px] text-muted-soft">
          {t("common:calendar.free")}
        </div>
      )}
    </div>
  );
}
