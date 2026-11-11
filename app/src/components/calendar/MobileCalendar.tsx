import { useMemo, useRef, useState } from "react";
import { Calendar, CalendarMinus, ChevronLeft, ChevronRight, Plus, X } from "lucide-react";
import { toast } from "sonner";

import type { CalendarEntry, CalendarItem } from "@/components/ItemCalendar";
import DatePicker from "@/components/DatePicker";
import { CalendarAvatar, ToneBar } from "@/components/calendar/parts";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { monthWeeks, toneSegments, weekDays, weekLabel, weekStart, type CalendarTone } from "@/lib/calendar";
import { mesYAno, paraFormatear } from "@/lib/fechas";
import { fromISODate, toISODate } from "@/lib/month";
import { useT, type MessageKey } from "@/lib/i18n";
import type { CalendarView } from "@/store/calendar.store";
import { cn } from "@/lib/utils";

/**
 * El calendario del teléfono (command-and-control-339). Reemplaza al mes
 * compacto y conserva su decisión principal: tocar un día abre una hoja
 * inferior y el mes no se mueve (jose, 18-oct-2026). En táctil no hay hover ni
 * arrastre: lo que no tiene fecha se programa con atajos (Hoy, Mañana, Lunes,
 * elegir fecha) y un «Deshacer». Medidas de `docs/design/calendario.md`
 * §«Móvil».
 */
const UPCOMING = 5;

/** El ancho de la barra de carga: crece con las tareas, con tope en cinco. */
export const loadWidth = (n: number) => (n ? Math.min(n, 5) * 5 + 4 : 0);

type Sheet = "day" | "lane" | null;

export default function MobileCalendar({
  view,
  onView,
  byDay,
  items,
  pending,
  hasLane,
  tones,
  today,
  onOpen,
  onSchedule,
  onUnschedule,
  onCreateForDay,
  countKey,
}: {
  view: CalendarView;
  onView: (v: CalendarView) => void;
  byDay: Map<string, CalendarItem[]>;
  items: CalendarItem[];
  pending: CalendarEntry[];
  hasLane: boolean;
  tones: CalendarTone[];
  today: string;
  onOpen: (id: string) => void;
  onSchedule?: (id: string, iso: string) => void;
  onUnschedule?: (id: string) => void;
  onCreateForDay?: (iso: string) => void;
  countKey: MessageKey;
}) {
  const { t } = useT();
  const locale = paraFormatear();
  const [cursor, setCursor] = useState(() => {
    const n = fromISODate(today)!;
    return new Date(n.getFullYear(), n.getMonth(), 1);
  });
  const [picked, setPicked] = useState(today);
  const [sheet, setSheet] = useState<Sheet>(null);
  const touchX = useRef<number | null>(null);

  const pickedDate = fromISODate(picked)!;
  const week = weekStart(pickedDate);
  const days = weekDays(week);
  const shift = (iso: string, n: number) => {
    const d = fromISODate(iso)!;
    return toISODate(new Date(d.getFullYear(), d.getMonth(), d.getDate() + n));
  };
  const overdue = items.filter((x) => x.meta?.overdue).length;
  const highKeys = tones.filter((x) => x.high).map((x) => x.key);
  const hasMeta = items.some((x) => x.meta);

  const upcoming = useMemo(
    () =>
      items
        .map((r) => ({ r, iso: [...byDay].find(([, v]) => v.includes(r))?.[0] }))
        .filter((x): x is { r: CalendarItem; iso: string } => !!x.iso && x.iso >= today && !x.r.meta?.done)
        .sort((a, b) => (a.iso < b.iso ? -1 : a.iso > b.iso ? 1 : 0))
        .slice(0, UPCOMING),
    [items, byDay, today],
  );

  const goToday = () => {
    const n = fromISODate(today)!;
    setCursor(new Date(n.getFullYear(), n.getMonth(), 1));
    setPicked(today);
  };
  const step = (dir: 1 | -1) => {
    if (view === "week") setPicked((p) => shift(p, 7 * dir));
    else setCursor((c) => new Date(c.getFullYear(), c.getMonth() + dir, 1));
  };

  const open = (id: string) => {
    setSheet(null);
    onOpen(id);
  };

  /** Programar desde la hoja: la tarjeta sale de la lista y queda un «Deshacer». */
  const schedule = (it: CalendarEntry, iso: string, when: string) => {
    onSchedule?.(it.id, iso);
    toast(t("common:calendar.movedTo", { label: it.label ?? it.title, when }), {
      duration: 4000,
      action: onUnschedule ? { label: t("common:calendar.undo"), onClick: () => onUnschedule(it.id) } : undefined,
    });
  };
  const nextMonday = () => {
    const d = fromISODate(today)!;
    const add = ((8 - d.getDay()) % 7) || 7;
    return toISODate(new Date(d.getFullYear(), d.getMonth(), d.getDate() + add));
  };

  const box = "motion-press grid size-10 place-items-center rounded-[8px] border border-border";
  const dayItems = byDay.get(picked) ?? [];
  const weekdayLong = (d: Date) => new Intl.DateTimeFormat(locale, { weekday: "long" }).format(d);
  const dayMonth = (d: Date) => new Intl.DateTimeFormat(locale, { day: "numeric", month: "long" }).format(d);
  const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
  const weekItems = days.flatMap((d) => byDay.get(toISODate(d)) ?? []);

  return (
    <div className="flex flex-col gap-[14px] leading-[normal]">
      <div className="flex items-center gap-2 border-b border-border pb-[10px]">
        <div role="tablist" className="flex gap-[2px] rounded-[9px] border border-border bg-muted p-[2px] text-[13px]">
          {(["month", "week"] as const).map((v) => (
            <button
              key={v}
              type="button"
              role="tab"
              aria-selected={view === v}
              onClick={() => {
                if (v === "month") setCursor(new Date(pickedDate.getFullYear(), pickedDate.getMonth(), 1));
                else setSheet(null);
                onView(v);
              }}
              className={cn(
                "motion-press grid h-9 place-items-center rounded-[7px] px-[14px]",
                view === v ? "bg-accent font-semibold text-foreground" : "font-normal text-muted-foreground",
              )}
            >
              {v === "month" ? t("common:calendar.month") : t("common:calendar.week")}
            </button>
          ))}
        </div>
        {hasLane && (
          <button
            type="button"
            aria-label={t("common:calendar.lane")}
            onClick={() => setSheet("lane")}
            className="motion-press relative grid h-10 w-11 place-items-center rounded-[8px] border border-border text-foreground"
          >
            <CalendarMinus className="size-[18px]" />
            {pending.length > 0 && (
              <span className="absolute -top-[6px] -right-[6px] grid h-[18px] min-w-[18px] place-items-center rounded-full bg-primary px-[5px] text-[11px] font-bold text-primary-foreground">
                {pending.length}
              </span>
            )}
          </button>
        )}
        <div className="ml-auto flex gap-1">
          <button type="button" aria-label={t("common:calendar.previous")} onClick={() => step(-1)} className={box}>
            <ChevronLeft className="size-4" />
          </button>
          <button
            type="button"
            onClick={goToday}
            className="motion-press h-10 rounded-[8px] border border-border px-3 text-[13px] font-semibold"
          >
            {t("common:calendar.today")}
          </button>
          <button type="button" aria-label={t("common:calendar.next")} onClick={() => step(1)} className={box}>
            <ChevronRight className="size-4" />
          </button>
        </div>
      </div>

      <div className="flex items-baseline justify-between">
        <span className="text-[16px] font-semibold">{view === "week" ? weekLabel(week, true) : mesYAno(cursor)}</span>
        {overdue > 0 && (
          <span className="flex items-center gap-[5px] text-[12px] text-error">
            <span className="size-[6px] rounded-full bg-error" />
            {t("common:calendar.overdueCount", { count: overdue })}
          </span>
        )}
      </div>

      {view === "month" ? (
        <>
          <div className="grid grid-cols-7 gap-y-[2px] text-center">
            {days.map((d) => (
              <span key={d.getDay()} className="pb-1 text-[11px] font-medium text-muted-foreground">
                {new Intl.DateTimeFormat(locale, { weekday: "narrow" }).format(d)}
              </span>
            ))}
            {monthWeeks(cursor).map((d) => {
              const iso = toISODate(d);
              const its = byDay.get(iso) ?? [];
              return (
                <button
                  key={iso}
                  type="button"
                  data-day={iso}
                  aria-label={`${dayMonth(d)} · ${t(countKey, { count: its.length })}`}
                  onClick={() => {
                    setPicked(iso);
                    setSheet("day");
                  }}
                  className={cn(
                    "motion-press flex h-[50px] flex-col items-center justify-center gap-[5px] rounded-[10px]",
                    d.getMonth() !== cursor.getMonth() && "opacity-35",
                    sheet === "day" && picked === iso && "bg-accent",
                  )}
                >
                  <span
                    className={cn(
                      "relative grid size-7 place-items-center rounded-full text-[14px]",
                      iso === today ? "bg-primary font-semibold text-primary-foreground" : "font-normal text-foreground",
                    )}
                  >
                    {d.getDate()}
                    {its.some((x) => x.meta?.overdue) && (
                      <span data-overdue className="absolute top-0 -right-px size-[6px] rounded-full bg-error" />
                    )}
                  </span>
                  <LoadBar items={its} tones={tones} />
                </button>
              );
            })}
          </div>

          {hasLane && pending.length > 0 && (
            <button
              type="button"
              onClick={() => setSheet("lane")}
              className="flex w-full items-center gap-3 rounded-[12px] border border-dashed border-primary/45 bg-primary/4 p-3 text-left"
            >
              <span className="grid h-[30px] min-w-[30px] place-items-center rounded-full bg-primary text-[14px] font-bold text-primary-foreground">
                {pending.length}
              </span>
              <span className="flex flex-1 flex-col gap-px">
                <span className="text-[14px] font-semibold text-primary">{t("common:calendar.lane")}</span>
                <span className="text-[12px] text-muted-foreground">{t("common:calendar.laneBanner")}</span>
              </span>
              <ChevronRight className="size-4 text-muted-foreground" />
            </button>
          )}

          {upcoming.length > 0 && (
            <section className="flex flex-col gap-[6px]">
              <span className="text-[12px] font-medium text-muted-foreground">{t("common:calendar.upcoming")}</span>
              {upcoming.map(({ r, iso }) => {
                const tone = tones.find((x) => x.key === r.tone);
                const isToday = iso === today;
                const label = isToday
                  ? t("common:calendar.today")
                  : iso === shift(today, 1)
                    ? t("common:calendar.tomorrow")
                    : new Intl.DateTimeFormat(locale, { day: "numeric", month: "short" }).format(fromISODate(iso)!).replace(/\.$/, "");
                return (
                  <button
                    key={r.id}
                    type="button"
                    onClick={() => onOpen(r.id)}
                    className="flex w-full items-stretch gap-[10px] rounded-[10px] border border-border bg-raised p-[10px] text-left"
                  >
                    <span className={cn("w-[3px] flex-none rounded-[2px]", tone?.edge ?? "bg-border-strong")} />
                    <span
                      className={cn(
                        "w-[52px] flex-none pt-px text-[12px]",
                        isToday ? "font-semibold text-primary" : "font-normal text-muted-foreground",
                      )}
                    >
                      {label}
                    </span>
                    <span className="flex min-w-0 flex-1 flex-col gap-[3px]">
                      <span className="line-clamp-2 text-[14px] leading-[19px]">{r.title}</span>
                      <Meta item={r} size={11} />
                    </span>
                  </button>
                );
              })}
            </section>
          )}
        </>
      ) : (
        <>
          <div className="grid grid-cols-7 gap-1">
            {days.map((d) => {
              const iso = toISODate(d);
              const its = byDay.get(iso) ?? [];
              return (
                <button
                  key={iso}
                  type="button"
                  data-strip={iso}
                  aria-pressed={iso === picked}
                  onClick={() => setPicked(iso)}
                  className={cn(
                    "motion-press flex h-[66px] flex-col items-center justify-center gap-1 rounded-[12px] border",
                    iso === picked ? "border-border-strong bg-raised" : "border-transparent",
                  )}
                >
                  <span className="text-[11px] font-medium text-muted-foreground">
                    {new Intl.DateTimeFormat(locale, { weekday: "narrow" }).format(d)}
                  </span>
                  <span
                    className={cn(
                      "grid size-[26px] place-items-center rounded-full text-[14px] font-semibold",
                      iso === today ? "bg-primary text-primary-foreground" : "text-foreground",
                    )}
                  >
                    {d.getDate()}
                  </span>
                  <LoadBar items={its} tones={tones} />
                </button>
              );
            })}
          </div>
          <div className="flex flex-wrap gap-[6px]">
            <span className="rounded-full border border-border bg-card px-[10px] py-1 text-[12px]">
              {t("common:calendar.weekCount", { count: weekItems.length })}
            </span>
            {highKeys.length > 0 && (
              <span className="rounded-full bg-warning/12 px-[10px] py-1 text-[12px] text-warning">
                {t("common:calendar.highCount", { count: weekItems.filter((x) => highKeys.includes(x.tone)).length })}
              </span>
            )}
            {hasMeta && (
              <span className="rounded-full bg-success/12 px-[10px] py-1 text-[12px] text-success">
                {t("common:calendar.doneCount", { count: weekItems.filter((x) => x.meta?.done).length })}
              </span>
            )}
          </div>
          <div
            data-swipe
            className="flex flex-col gap-2"
            onTouchStart={(e) => (touchX.current = e.touches[0].clientX)}
            onTouchEnd={(e) => {
              if (touchX.current === null) return;
              const dx = e.changedTouches[0].clientX - touchX.current;
              touchX.current = null;
              if (Math.abs(dx) > 50) setPicked((p) => shift(p, dx < 0 ? 1 : -1));
            }}
          >
            <div className="flex items-baseline justify-between">
              <span className="text-[15px] font-semibold">
                {cap(weekdayLong(pickedDate))} {pickedDate.getDate()}
                {picked === today && ` · ${t("common:calendar.todayLower")}`}
              </span>
              <span className="text-[12px] text-muted-foreground">{t(countKey, { count: dayItems.length })}</span>
            </div>
            {dayItems.map((it) => (
              <BigCard key={it.id} item={it} tones={tones} onOpen={() => onOpen(it.id)} />
            ))}
            {dayItems.length === 0 && (
              <div className="rounded-[10px] border border-dashed border-border p-[22px] text-center text-[13px] text-muted-foreground">
                {t("common:calendar.dayFree")}
              </div>
            )}
            <span className="text-center text-[11px] text-muted-soft">{t("common:calendar.swipeHint")}</span>
          </div>
        </>
      )}

      <Sheet open={sheet !== null} onOpenChange={(o) => !o && setSheet(null)}>
        <SheetContent
          side="bottom"
          showCloseButton={false}
          overlayClassName="bg-black/55"
          className="gap-3 rounded-t-[18px] border-t border-border bg-muted px-4 pt-2 pb-[calc(30px+env(safe-area-inset-bottom))] shadow-[0_-10px_30px_rgba(0,0,0,.4)] data-[side=bottom]:max-h-[76dvh]"
        >
          <span aria-hidden className="h-1 w-10 self-center rounded-full bg-muted-foreground/35" />
          <div className="flex items-start justify-between">
            <div className="flex flex-col gap-px">
              <SheetDescription className="text-[11px] tracking-[.06em] text-muted-foreground uppercase">
                {sheet === "lane"
                  ? t("common:calendar.withoutDate", { count: pending.length })
                  : `${weekdayLong(pickedDate)}${picked === today ? ` · ${t("common:calendar.todayLower")}` : ""} · ${t(countKey, { count: dayItems.length })}`}
              </SheetDescription>
              <SheetTitle className="text-[20px] font-semibold">
                {sheet === "lane" ? t("common:calendar.lane") : dayMonth(pickedDate)}
              </SheetTitle>
            </div>
            <button
              type="button"
              aria-label={t("common:calendar.close")}
              onClick={() => setSheet(null)}
              className="motion-press grid size-10 place-items-center rounded-[10px] bg-track text-muted-foreground"
            >
              <X className="size-4" />
            </button>
          </div>

          {sheet === "day" && (
            <>
              {dayItems.length > 0 && (
                <div className="flex flex-col gap-[6px]">
                  <ToneBar segments={toneSegments(dayItems, tones)} height={5} />
                  <div className="flex flex-wrap gap-[10px]">
                    {toneSegments(dayItems, tones).map((s) => (
                      <span key={s.tone.key} className="flex items-center gap-[5px] text-[12px] text-muted-foreground">
                        <span className={cn("size-[6px] rounded-full", s.tone.dot)} />
                        {s.count} {s.tone.label.toLowerCase()}
                      </span>
                    ))}
                  </div>
                </div>
              )}
              <div className="flex min-h-0 flex-col gap-[6px] overflow-y-auto">
                {dayItems.map((it) => {
                  const tone = tones.find((x) => x.key === it.tone);
                  return (
                    <button
                      key={it.id}
                      type="button"
                      onClick={() => open(it.id)}
                      className={cn(
                        "flex w-full items-start gap-[10px] rounded-[10px] border bg-raised p-3 text-left",
                        it.meta?.overdue ? "border-error/55" : "border-border",
                      )}
                    >
                      <span className={cn("w-[3px] flex-none self-stretch rounded-[2px]", tone?.edge ?? "bg-border-strong")} />
                      <span className="flex min-w-0 flex-1 flex-col gap-1">
                        <span
                          className={cn(
                            "text-[14px] leading-[19px] text-pretty",
                            it.meta?.done ? "text-muted-foreground line-through" : "text-foreground",
                          )}
                        >
                          {it.title}
                        </span>
                        <Meta item={it} size={12} overdue />
                      </span>
                      {it.meta?.assignee && (
                        <CalendarAvatar initials={it.meta.assignee.initials} colorKey={it.meta.assignee.key} size={24} />
                      )}
                    </button>
                  );
                })}
                {dayItems.length === 0 && (
                  <div className="p-[18px] text-center text-[14px] text-muted-foreground">{t("common:calendar.nothingDue")}</div>
                )}
              </div>
              {onCreateForDay && (
                <button
                  type="button"
                  onClick={() => {
                    setSheet(null);
                    onCreateForDay(picked);
                  }}
                  className="motion-press flex h-[46px] items-center justify-center gap-2 rounded-[10px] border border-border text-[14px] font-medium text-primary"
                >
                  <Plus className="size-4" />
                  {t("common:calendar.newForDay")}
                </button>
              )}
            </>
          )}

          {sheet === "lane" && (
            <>
              <span className="-mt-[6px] text-[13px] text-muted-foreground">{t("common:calendar.laneSheetHelp")}</span>
              <div className="flex min-h-0 flex-col gap-2 overflow-y-auto">
                {pending.map((it) => {
                  const tone = tones.find((x) => x.key === it.tone);
                  const shortcut = "motion-press grid h-[38px] flex-1 place-items-center rounded-[8px] border border-border text-[13px] font-medium";
                  return (
                    <div
                      key={it.id}
                      data-pending={it.id}
                      className="relative flex flex-col gap-[10px] rounded-[10px] border border-border bg-raised py-3 pr-3 pl-[14px]"
                    >
                      <span aria-hidden className={cn("absolute -top-px -bottom-px -left-px w-[3px] rounded-l-[10px]", tone?.edge ?? "bg-border-strong")} />
                      <button type="button" onClick={() => open(it.id)} className="flex flex-col gap-[10px] text-left">
                        <span className="flex items-center gap-[6px]">
                          {it.label && <span className="font-mono text-[12px] text-muted-foreground">{it.label}</span>}
                          {it.meta?.assignee && (
                            <span className="ml-auto">
                              <CalendarAvatar initials={it.meta.assignee.initials} colorKey={it.meta.assignee.key} size={22} />
                            </span>
                          )}
                        </span>
                        <span className="text-[14px] leading-[19px]">{it.title}</span>
                      </button>
                      <div className="flex gap-[6px]">
                        <button
                          type="button"
                          onClick={() => schedule(it, today, t("common:calendar.todayLower"))}
                          className="motion-press grid h-[38px] flex-1 place-items-center rounded-[8px] bg-primary text-[13px] font-semibold text-primary-foreground"
                        >
                          {t("common:calendar.today")}
                        </button>
                        <button type="button" onClick={() => schedule(it, shift(today, 1), t("common:calendar.tomorrowLower"))} className={shortcut}>
                          {t("common:calendar.tomorrow")}
                        </button>
                        <button
                          type="button"
                          onClick={() => {
                            const m = nextMonday();
                            schedule(it, m, `${weekdayLong(fromISODate(m)!)} ${fromISODate(m)!.getDate()}`);
                          }}
                          className={shortcut}
                        >
                          {t("common:calendar.monday")}
                        </button>
                        <DatePicker
                          value=""
                          onChange={(iso) => {
                            if (iso) schedule(it, iso, new Intl.DateTimeFormat(locale, { day: "numeric", month: "short" }).format(fromISODate(iso)!));
                          }}
                          trigger={
                            <button
                              type="button"
                              aria-label={t("common:calendar.pickDate")}
                              title={t("common:calendar.pickDate")}
                              className="motion-press grid h-[38px] w-11 place-items-center rounded-[8px] border border-border text-muted-foreground"
                            >
                              <Calendar className="size-4" />
                            </button>
                          }
                        />
                      </div>
                    </div>
                  );
                })}
                {pending.length === 0 && (
                  <div className="p-[22px] text-center text-[14px] text-success">{t("common:calendar.allDated")}</div>
                )}
              </div>
            </>
          )}
        </SheetContent>
      </Sheet>
    </div>
  );
}

/** La barra de carga de un día del teléfono: más larga con más tareas, tope en cinco. */
function LoadBar({ items, tones }: { items: CalendarItem[]; tones: CalendarTone[] }) {
  return (
    <span data-load={items.length} className="flex h-[3px] gap-[1.5px] overflow-hidden rounded-full" style={{ width: loadWidth(items.length) }}>
      {toneSegments(items, tones).map((s) => (
        <span key={s.tone.key} className={s.tone.dot} style={{ flex: s.count }} />
      ))}
    </span>
  );
}

/** Folio, estado y, si se pide, «· Vencida». */
function Meta({ item, size, overdue = false }: { item: CalendarEntry; size: 11 | 12; overdue?: boolean }) {
  const { t } = useT();
  const m = item.meta;
  return (
    <span className={cn("flex items-center gap-[6px] text-muted-foreground", size === 11 ? "text-[11px]" : "text-[12px]")}>
      {item.label && <span className="font-mono">{item.label}</span>}
      {m?.status && (
        <>
          <span className={cn("size-[5px] rounded-full", m.status.dot)} />
          {m.status.name}
        </>
      )}
      {overdue && m?.overdue && <span className="font-semibold text-error">· {t("common:calendar.overdue")}</span>}
    </span>
  );
}

/** La tarjeta del día elegido en la semana del teléfono: la de la semana de escritorio, en grande. */
function BigCard({ item, tones, onOpen }: { item: CalendarItem; tones: CalendarTone[]; onOpen: () => void }) {
  const { t } = useT();
  const tone = tones.find((x) => x.key === item.tone);
  const m = item.meta;
  return (
    <button
      type="button"
      onClick={onOpen}
      className={cn(
        "relative flex w-full flex-col gap-[6px] rounded-[10px] border bg-raised py-3 pr-3 pl-[14px] text-left",
        m?.overdue ? "border-error/55" : "border-border",
      )}
    >
      <span aria-hidden className={cn("absolute -top-px -bottom-px -left-px w-[3px] rounded-l-[10px]", tone?.edge ?? "bg-border-strong")} />
      <span className="flex w-full items-center gap-[6px]">
        {item.label && <span className="font-mono text-[12px] text-muted-foreground">{item.label}</span>}
        {m?.overdue && (
          <span className="rounded-[4px] bg-error/12 px-[6px] text-[11px] font-semibold text-error">{t("common:calendar.overdue")}</span>
        )}
        {m?.assignee && (
          <span className="ml-auto">
            <CalendarAvatar initials={m.assignee.initials} colorKey={m.assignee.key} size={22} />
          </span>
        )}
      </span>
      <span
        className={cn("text-[14px] leading-[19px] text-pretty", m?.done ? "text-muted-foreground line-through" : "text-foreground")}
      >
        {item.title}
      </span>
      {m?.status && (
        <span className="flex items-center gap-[5px] text-[12px] text-muted-foreground">
          <span className={cn("size-[6px] rounded-full", m.status.dot)} />
          {m.status.name}
        </span>
      )}
    </button>
  );
}
