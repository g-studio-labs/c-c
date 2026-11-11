import { useEffect, useRef, useState } from "react";
import { useDroppable } from "@dnd-kit/core";
import { Plus, X } from "lucide-react";

import type { CalendarItem } from "@/components/ItemCalendar";
import { CalendarAvatar, ToneBar, unitOf } from "@/components/calendar/parts";
import { heatLevel, toneSegments, type CalendarTone } from "@/lib/calendar";
import { paraFormatear } from "@/lib/fechas";
import { toISODate, weekdayInitials } from "@/lib/month";
import { useT, type MessageKey } from "@/lib/i18n";
import { cn } from "@/lib/utils";

/**
 * El mes de escritorio (command-and-control-335): cada día dice cuánto hay y de
 * qué tono, y el detalle sale en un popover **anclado al día**. Antes los
 * títulos se cortaban a quince caracteres y «+N más» abría el día debajo de
 * todo el mes, fuera de la pantalla.
 *
 * El popover vive **dentro** de la celda, absoluto y desplazado −8 px para
 * tapar su borde, como en el diseño; por eso la rejilla no lleva
 * `overflow:hidden` y las esquinas se redondean celda a celda.
 *
 * Abrir: el ratón encima 220 ms es un vistazo, que se cierra al salir; un clic
 * lo fija (aparece la ×) y otro clic en el mismo día lo suelta. Se cierra con
 * la ×, con Esc, con un clic fuera de cualquier día y al empezar a arrastrar.
 * Con uno fijado, pasar por otro día no abre otro.
 */
export const HOVER_DELAY = 220;

const HEAT = ["bg-background", "bg-heat-1", "bg-heat-2", "bg-heat-3"] as const;

export default function MonthGrid({
  cells,
  month,
  byDay,
  tones,
  today,
  dragging,
  canDrop,
  onOpen,
  onCreateForDay,
  countKey,
}: {
  cells: Date[];
  /** El mes que se mira (0–11): lo de fuera va al 40%. */
  month: number;
  /** Lo de cada día, por `YYYY-MM-DD` local. */
  byDay: Map<string, CalendarItem[]>;
  tones: CalendarTone[];
  today: string;
  dragging: boolean;
  canDrop: boolean;
  onOpen: (id: string) => void;
  onCreateForDay?: (iso: string) => void;
  countKey: MessageKey;
}) {
  const { t } = useT();
  const [hovered, setHovered] = useState<string | null>(null);
  const [pinned, setPinned] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const rows = cells.length / 7;

  // Esc cierra; un clic fuera de cualquier día suelta el fijado.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setPinned(null);
        setHovered(null);
      }
    };
    const down = (e: MouseEvent) => {
      const el = e.target as Element | null;
      if (!el?.closest?.("[data-cell]")) setPinned(null);
    };
    window.addEventListener("keydown", key);
    window.addEventListener("mousedown", down);
    return () => {
      window.removeEventListener("keydown", key);
      window.removeEventListener("mousedown", down);
      clearTimeout(timer.current);
    };
  }, []);

  // Empezar a arrastrar cierra lo abierto y deja de escuchar el hover.
  useEffect(() => {
    if (!dragging) return;
    clearTimeout(timer.current);
    setHovered(null);
    setPinned(null);
  }, [dragging]);

  const weekdays = weekdayInitials();
  return (
    <div className="grid grid-cols-7 gap-px rounded-[10px] border border-border bg-border">
      {weekdays.map((w, i) => (
        <div
          key={w}
          className={cn(
            "bg-muted px-2 py-1 text-center text-[13px] font-medium text-muted-foreground",
            i === 0 && "rounded-tl-[9px]",
            i === 6 && "rounded-tr-[9px]",
          )}
        >
          {w}
        </div>
      ))}
      {cells.map((d, i) => {
        const iso = toISODate(d);
        const col = i % 7;
        const row = Math.floor(i / 7);
        const items = byDay.get(iso) ?? [];
        const open = !dragging && (pinned ? pinned === iso : hovered === iso);
        return (
          <DayCell
            key={iso}
            iso={iso}
            acceptsDrop={canDrop && dragging}
            open={open}
            className={cn(
              row === rows - 1 && col === 0 && "rounded-bl-[9px]",
              row === rows - 1 && col === 6 && "rounded-br-[9px]",
            )}
            heat={HEAT[heatLevel(items.length)]}
            onEnter={() => {
              if (dragging) return;
              clearTimeout(timer.current);
              timer.current = setTimeout(() => setHovered(iso), HOVER_DELAY);
            }}
            onLeave={() => {
              clearTimeout(timer.current);
              setHovered(null);
            }}
            onToggle={() => setPinned((p) => (p === iso ? null : iso))}
            label={`${new Intl.DateTimeFormat(paraFormatear(), { day: "numeric", month: "long" }).format(d)} · ${t(countKey, { count: items.length })}`}
          >
            <div
              className={cn("flex min-h-[84px] flex-col justify-between", d.getMonth() !== month && "opacity-40")}
            >
              <div className="flex items-center justify-between">
                <span
                  className={cn(
                    "grid size-[22px] place-items-center rounded-full text-[13px]",
                    iso === today
                      ? "bg-primary font-semibold text-primary-foreground"
                      : "font-normal text-muted-foreground",
                  )}
                >
                  {d.getDate()}
                </span>
                {items.some((x) => x.meta?.overdue) && (
                  <span
                    data-overdue
                    title={t("common:calendar.overdueOpen")}
                    className="size-[7px] rounded-full bg-error"
                  />
                )}
              </div>
              {items.length > 0 && (
                <div className="flex flex-col gap-[5px]">
                  <span className="text-[20px] leading-[22px] font-semibold tabular-nums">
                    {items.length}
                    <span className="ml-1 text-[11px] font-normal text-muted-foreground">
                      {unitOf(t(countKey, { count: items.length }), items.length)}
                    </span>
                  </span>
                  <ToneBar segments={toneSegments(items, tones)} height={5} />
                </div>
              )}
            </div>
            {open && (
              <DayPopover
                date={d}
                isToday={iso === today}
                items={items}
                tones={tones}
                pinned={pinned === iso}
                countKey={countKey}
                // Lun–jue se abre hacia la derecha; vie–dom, hacia la izquierda.
                // Las dos últimas filas, hacia arriba.
                position={cn(col >= 4 ? "right-[-8px]" : "left-[-8px]", row >= rows - 2 ? "bottom-[-8px]" : "top-[-8px]")}
                onClose={() => {
                  setPinned(null);
                  setHovered(null);
                }}
                onOpen={onOpen}
                onCreate={onCreateForDay ? () => onCreateForDay(iso) : undefined}
              />
            )}
          </DayCell>
        );
      })}
    </div>
  );
}

/** Un día: recibe lo que se suelta encima, y se abre con ratón o teclado. */
function DayCell({
  iso,
  acceptsDrop,
  open,
  heat,
  className,
  onEnter,
  onLeave,
  onToggle,
  label,
  children,
}: {
  iso: string;
  acceptsDrop: boolean;
  open: boolean;
  heat: string;
  className?: string;
  onEnter: () => void;
  onLeave: () => void;
  onToggle: () => void;
  label: string;
  children: React.ReactNode;
}) {
  const { setNodeRef, isOver } = useDroppable({ id: iso, disabled: !acceptsDrop });
  const over = acceptsDrop && isOver;
  return (
    <div
      ref={setNodeRef}
      data-cell
      data-day={iso}
      role="button"
      tabIndex={0}
      aria-expanded={open}
      aria-label={label}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      onClick={onToggle}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onToggle();
        }
      }}
      className={cn(
        "relative min-h-[96px] cursor-pointer px-2 py-[6px] outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
        over ? "bg-primary/10 ring-2 ring-primary ring-inset" : cn(heat, "hover:bg-heat-hover"),
        open && "z-20",
        open && !over && "ring-1 ring-primary/70 ring-inset",
        className,
      )}
    >
      {children}
    </div>
  );
}

function DayPopover({
  date,
  isToday,
  items,
  tones,
  pinned,
  countKey,
  position,
  onClose,
  onOpen,
  onCreate,
}: {
  date: Date;
  isToday: boolean;
  items: CalendarItem[];
  tones: CalendarTone[];
  pinned: boolean;
  countKey: MessageKey;
  position: string;
  onClose: () => void;
  onOpen: (id: string) => void;
  onCreate?: () => void;
}) {
  const { t } = useT();
  const locale = paraFormatear();
  const weekday = new Intl.DateTimeFormat(locale, { weekday: "long" }).format(date);
  const title = new Intl.DateTimeFormat(locale, { day: "numeric", month: "long" }).format(date);
  const segments = toneSegments(items, tones);
  return (
    // Los clics de dentro no llegan a la celda: si no, abrir una tarea soltaba
    // el popover y elegir una fila lo cerraba antes de abrirla.
    <div
      role="dialog"
      aria-label={title}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => e.stopPropagation()}
      className={cn(
        "absolute flex w-[340px] max-w-[calc(100vw-80px)] cursor-default flex-col gap-2 rounded-[12px] border border-border-strong bg-popover p-3 text-popover-foreground shadow-[0_18px_40px_rgba(0,0,0,.55)]",
        position,
      )}
    >
      <div className="flex items-start justify-between">
        <div className="flex flex-col gap-px">
          <span className="text-[11px] tracking-[.06em] text-muted-foreground uppercase">
            {isToday ? `${weekday} · ${t("common:calendar.todayLower")}` : weekday}
          </span>
          <span className="text-[18px] font-semibold">{title}</span>
        </div>
        <div className="flex items-center gap-[6px]">
          <span className="text-[12px] text-muted-foreground">{t(countKey, { count: items.length })}</span>
          {pinned && (
            <button
              type="button"
              aria-label={t("common:calendar.close")}
              onClick={onClose}
              className="motion-press grid size-[26px] place-items-center rounded-[6px] text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              <X className="size-[15px]" />
            </button>
          )}
        </div>
      </div>

      {segments.length > 0 && (
        <div className="flex flex-wrap gap-[6px]">
          {segments.map((s) => (
            <span
              key={s.tone.key}
              className="flex items-center gap-[5px] rounded-full border border-border bg-card px-2 py-px text-[11px] text-foreground"
            >
              <span className={cn("size-[6px] rounded-full", s.tone.dot)} />
              {s.count} {s.tone.label.toLowerCase()}
            </span>
          ))}
        </div>
      )}

      {items.length > 0 ? (
        <div className="-mx-1 flex max-h-[320px] flex-col gap-[2px] overflow-auto">
          {items.map((it) => (
            <PopoverRow key={it.id} item={it} tones={tones} onOpen={() => onOpen(it.id)} />
          ))}
        </div>
      ) : (
        <span className="px-[2px] py-1 text-[13px] text-muted-foreground">{t("common:calendar.nothingDue")}</span>
      )}

      {(onCreate || !pinned) && (
        <div className="flex items-center justify-between border-t border-border pt-2 text-[12px]">
          {onCreate ? (
            <button type="button" onClick={onCreate} className="motion-press flex items-center gap-[6px] text-primary">
              <Plus className="size-[14px]" />
              {t("common:calendar.newForDay")}
            </button>
          ) : (
            <span />
          )}
          {!pinned && <span className="text-muted-soft">{t("common:calendar.clickToPin")}</span>}
        </div>
      )}
    </div>
  );
}

function PopoverRow({ item, tones, onOpen }: { item: CalendarItem; tones: CalendarTone[]; onOpen: () => void }) {
  const { t } = useT();
  const tone = tones.find((x) => x.key === item.tone);
  const m = item.meta;
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex w-full items-start gap-2 rounded-[8px] p-[6px] text-left hover:bg-accent"
    >
      <span className={cn("w-[3px] flex-none self-stretch rounded-[2px]", tone?.edge ?? "bg-border-strong")} />
      <span className="flex min-w-0 flex-1 flex-col gap-[2px]">
        <span
          className={cn(
            "text-[13px] leading-[18px] text-pretty",
            m?.done ? "text-muted-foreground line-through" : "text-foreground",
          )}
        >
          {item.title}
        </span>
        <span className="flex items-center gap-[6px] text-[11px] text-muted-foreground">
          {item.label && <span className="font-mono">{item.label}</span>}
          {m?.status && (
            <>
              <span className={cn("size-[5px] rounded-full", m.status.dot)} />
              {m.status.name}
            </>
          )}
          {m?.overdue && (
            <span className="text-[10px] font-semibold text-error">· {t("common:calendar.overdue")}</span>
          )}
        </span>
      </span>
      {m?.assignee && <CalendarAvatar initials={m.assignee.initials} colorKey={m.assignee.key} size={20} />}
    </button>
  );
}
