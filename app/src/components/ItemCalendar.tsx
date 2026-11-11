import {
  DndContext,
  PointerSensor,
  KeyboardSensor,
  pointerWithin,
  useDraggable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { GripVertical } from "lucide-react";

import { diaYMes, fecha, mesYAno } from "@/lib/fechas";
import {
  dayKey,
  toISODate,
  fromISODate,
  weekdayInitials,
  monthGrid,
} from "@/lib/month";
import { monthWeeks, weekLabel, weekStart } from "@/lib/calendar";
import { useCalendarStore } from "@/store/calendar.store";
import CalendarHeader from "@/components/calendar/CalendarHeader";
import MonthGrid from "@/components/calendar/MonthGrid";
import { useT, type MessageKey } from "@/lib/i18n";
import { useMemo, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useIsMobile } from "@/hooks/use-mobile";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { cn } from "@/lib/utils";
import type { CalendarTone } from "@/lib/calendar";

/**
 * A month of work, by the day it arrived.
 *
 * Deliberately not typed to reports any more. It groups by a date and paints a
 * dot per item, which is true of a board card as much as it was of a report —
 * the view was only ever tied to reports because that was the page it lived on.
 * The caller says what colour each dot is, so the board can colour by column
 * without this file knowing what a column is.
 */
export interface CalendarEntry {
  id: string;
  title: string;
  /**
   * La clave de su tono (una de `tones`): de qué color es y en qué segmento de
   * la barra cuenta. El llamante decide qué significan los colores —prioridad,
   * reunión pausada— sin que este fichero lo sepa (command-and-control-333).
   */
  tone: string;
  /** A short prefix — a folio, a number — shown before the title. */
  label?: string;
  /**
   * Lo que se lee en una fila o una tarjeta además del título. Opcional entero:
   * una reunión no tiene estado ni responsable, y lo que no viene no se pinta.
   */
  meta?: CalendarMeta;
}

export interface CalendarMeta {
  /** El estado, con el punto de su color (`lib/status-color.ts`). */
  status?: { name: string; dot: string };
  /** Las iniciales y la clave estable de su color (`lib/person-color.ts`). */
  assignee?: { initials: string; key: string };
  /** Terminada: se pinta tachada y apagada. */
  done?: boolean;
  /** Venció sin cerrarse: el punto rojo del día y el chip «Vencida». */
  overdue?: boolean;
}

/**
 * An entry placed on a day, saying **which kind** of date it carries.
 *
 * Una sola `at` para todo es lo que torció el calendario de «Mi trabajo»: un
 * vencimiento es una **fecha** (el día a medianoche UTC) y una reunión es un
 * **instante**, y leídos igual uno de los dos sale mal. Con captadores locales,
 * el vencimiento del 30 se pinta el 29 al oeste de Greenwich; con captadores
 * UTC, la reunión de las 20:00 del 29 en Querétaro se pinta el 30. Las dos
 * lecturas son correctas para datos distintos, y este componente no puede
 * adivinar cuál le dan — así que el tipo obliga al llamante a decirlo.
 *
 * - `day`: `YYYY-MM-DD`, un día sin hora ni zona. Nunca pasa por `new Date()`.
 * - `at`: RFC3339, un momento concreto; cae en el día **local** de quien mira.
 */
export type CalendarItem = CalendarEntry &
  ({ day: string; at?: never } | { at: string; day?: never });

/** El día local en que se pinta, según qué clase de fecha lleve. */
function localDayOf(item: CalendarItem): Date | null {
  if (item.day !== undefined) return fromISODate(item.day);
  const d = new Date(item.at);
  return Number.isNaN(d.getTime()) ? null : d;
}


/** Cuántas de las próximas se enseñan bajo el mes en el teléfono. */
const UPCOMING = 6;

export default function ItemCalendar({
  items,
  tones,
  calendarId,
  undated = [],
  onSchedule,
  onOpen,
  onCreateForDay,
  countKey = "common:count.items",
}: {
  items: CalendarItem[];
  /**
   * Qué significa cada color, en orden: el de los segmentos de la barra y el de
   * la leyenda. Ver `components/calendar/tones.ts`.
   */
  tones: CalendarTone[];
  /**
   * Qué calendario es («mywork», «board», «meetings»): la vista y el carril
   * abierto se recuerdan por calendario (`store/calendar.store.ts`).
   */
  calendarId: string;
  /**
   * Lo que no tiene fecha, y por tanto no cabe en ningún día.
   *
   * Va debajo del mes en vez de descartarse. Descartarlo en silencio es lo que
   * hacía esta vista: de sesenta y cinco tareas se veía **una**, mientras la
   * cabecera seguía diciendo sesenta y cinco. Un calendario que esconde el 98%
   * de lo que hay no está filtrando, está mintiendo.
   */
  undated?: CalendarEntry[];
  /**
   * Poner fecha arrastrando una tarjeta de la tira a un día.
   *
   * Sin esto la tira sería una lista de reproches: «aquí tienes lo que no puedo
   * enseñarte». Con esto el calendario deja de ser algo que se mira y pasa a ser
   * donde se planifica, que es lo que le faltaba para servir de algo.
   *
   * Opcional: el calendario del tablero coloca por fecha de creación, y ésa no
   * se cambia arrastrando nada.
   */
  onSchedule?: (id: string, iso: string) => void;
  onOpen: (id: string) => void;
  /**
   * «+ Nueva tarea para este día», en el pie del popover del día. Sin ella el
   * pie no lo ofrece: en el tablero y en reuniones no se crea desde aquí.
   */
  onCreateForDay?: (iso: string) => void;
  /**
   * Cómo se cuentan estos elementos: «3 tarjetas», «3 reuniones».
   *
   * Antes esto era un sustantivo suelto al que la vista le pegaba «(s)». Eso
   * sólo funciona en inglés y sólo para los plurales regulares: en castellano
   * «reunión» hace «reuniones», con acento que desaparece. Ahora entra la
   * **clave del mensaje entero**, con el número dentro, y cada idioma decide
   * dónde va y qué forma toma.
   */
  countKey?: MessageKey;
}) {
  const { t } = useT();
  const dotOf = (tone: string) => tones.find((x) => x.key === tone)?.dot ?? "bg-muted-foreground";
  const view = useCalendarStore((s) => s.views[calendarId] ?? "month");
  const setView = useCalendarStore((s) => s.setView);
  const [cursor, setCursor] = useState(() => {
    const n = new Date();
    return new Date(n.getFullYear(), n.getMonth(), 1);
  });
  // La semana que se mira, por su lunes. Aparte del mes: «Hoy» lleva las dos a
  // la actual, y pasar de Semana a Mes abre el mes de la semana visible.
  const [week, setWeek] = useState(() => weekStart(new Date()));
  // El día que se mira en el teléfono: hoy, de entrada.
  const [picked, setPicked] = useState(() => dayKey(new Date()));
  const [dragging, setDragging] = useState<string | null>(null);
  // Los mismos sensores que el tablero: puntero **y teclado**. El arrastre HTML5
  // nativo no tiene lo segundo, y esta es la única forma de poner una fecha sin
  // abrir la tarjeta — dejarla fuera del teclado la haría inalcanzable para
  // quien no usa ratón.
  const sensors = useSensors(
    // Ocho píxeles antes de considerarlo arrastre: sin el umbral, un clic con la
    // mano poco firme mueve la tarjeta en vez de abrirla.
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor),
  );

  // Por día local en las dos claves: `dayKey` para el teléfono (sin cambiar
  // todavía) y `YYYY-MM-DD` para el escritorio, que es además el id de cada día
  // como destino de arrastre.
  const { byDay, byISO } = useMemo(() => {
    const byDay = new Map<string, CalendarItem[]>();
    const byISO = new Map<string, CalendarItem[]>();
    for (const r of items) {
      const d = localDayOf(r);
      if (!d) continue;
      const k = dayKey(d);
      (byDay.get(k) ?? byDay.set(k, []).get(k)!).push(r);
      const i = toISODate(d);
      (byISO.get(i) ?? byISO.set(i, []).get(i)!).push(r);
    }
    return { byDay, byISO };
  }, [items]);

  // La rejilla del teléfono sigue en seis semanas (`monthGrid`); la del
  // escritorio, con las justas (`monthWeeks`, ver su comentario).
  const cells = useMemo(() => monthGrid(cursor), [cursor]);
  const weeks = useMemo(() => monthWeeks(cursor), [cursor]);

  const today = dayKey(new Date());
  const todayISO = toISODate(new Date());
  // En el teléfono, otro calendario: el mes de escritorio no cabe (cada día
  // queda de 60 px y una tarea se lee «#14 p…»).
  const compact = useIsMobile();

  const goToday = () => {
    const n = new Date();
    setCursor(new Date(n.getFullYear(), n.getMonth(), 1));
    setWeek(weekStart(n));
    setPicked(dayKey(n));
  };
  const step = (dir: 1 | -1) => {
    if (view === "week") setWeek((w) => new Date(w.getFullYear(), w.getMonth(), w.getDate() + 7 * dir));
    else setCursor((c) => new Date(c.getFullYear(), c.getMonth() + dir, 1));
  };

  const header = (
    <div className="flex items-center justify-between">
      <span className="font-medium">{mesYAno(cursor)}</span>
      <div className="flex gap-1">
        <Button size="icon-sm" variant="outline" onClick={() => setCursor(new Date(cursor.getFullYear(), cursor.getMonth() - 1, 1))}>
          <ChevronLeft className="h-4 w-4" />
        </Button>
        <Button size="sm" variant="outline" onClick={goToday}>
          {t("common:calendar.today")}
        </Button>
        <Button size="icon-sm" variant="outline" onClick={() => setCursor(new Date(cursor.getFullYear(), cursor.getMonth() + 1, 1))}>
          <ChevronRight className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );

  if (compact) {
    return (
      <CompactMonth
        header={header}
        items={items}
        cells={cells}
        cursor={cursor}
        today={today}
        byDay={byDay}
        picked={picked}
        onPick={setPicked}
        undated={undated}
        onOpen={onOpen}
        countKey={countKey}
        dotOf={dotOf}
      />
    );
  }

  return (
    // Interlineado `normal` y no el 1.5 que hereda la app: las medidas del
    // diseño están tomadas con el del navegador, y con 1.5 cada fila del
    // popover salía tres píxeles más alta.
    <div className="flex flex-col gap-[14px] leading-[normal]">
      <CalendarHeader
        title={view === "week" ? weekLabel(week) : mesYAno(cursor)}
        view={view}
        onView={(v) => {
          // De Semana a Mes, el mes de la semana que se miraba.
          if (v === "month" && view === "week") setCursor(new Date(week.getFullYear(), week.getMonth(), 1));
          setView(calendarId, v);
        }}
        tones={tones}
        onPrev={() => step(-1)}
        onToday={goToday}
        onNext={() => step(1)}
      />

      <DndContext
        sensors={sensors}
        collisionDetection={pointerWithin}
        onDragStart={(e) => setDragging(String(e.active.id))}
        onDragEnd={(e) => {
          setDragging(null);
          // `over` es el día sobre el que se soltó; su id **es** la fecha.
          if (onSchedule && e.over) onSchedule(String(e.active.id), String(e.over.id));
        }}
        onDragCancel={() => setDragging(null)}
      >
        <MonthGrid
          cells={weeks}
          month={cursor.getMonth()}
          byDay={byISO}
          tones={tones}
          today={todayISO}
          dragging={dragging !== null}
          canDrop={!!onSchedule}
          onOpen={onOpen}
          onCreateForDay={onCreateForDay}
          countKey={countKey}
        />

        {/* Lo que no tiene fecha, debajo y a mano. Ver el comentario de `undated`. */}
        {undated.length > 0 && (
          <div className="rounded-lg border p-3">
            <p className="mb-2 text-xs text-muted-foreground">
              {onSchedule
                ? t("common:calendar.undatedDrag", { count: undated.length })
                : t("common:calendar.undated", { count: undated.length })}
            </p>
            <div className="flex flex-wrap gap-1.5">
              {undated.map((r) => (
                <UndatedChip
                  key={r.id}
                  item={r}
                  draggable={!!onSchedule}
                  onOpen={() => onOpen(r.id)}
                  dotOf={dotOf}
                />
              ))}
            </div>
          </div>
        )}
      </DndContext>
    </div>
  );
}

/**
 * Una tarjeta sin fecha, de la tira de abajo.
 *
 * El asa es aparte del título: el título abre y el asa arrastra. Con todo el
 * bloque arrastrable, abrir una tarjeta se convierte en una apuesta sobre si la
 * mano se movió tres píxeles.
 */
function UndatedChip({
  item,
  draggable,
  onOpen,
  dotOf,
}: {
  item: CalendarEntry;
  draggable: boolean;
  onOpen: () => void;
  dotOf: (tone: string) => string;
}) {
  const { attributes, listeners, setNodeRef, transform, isDragging } = useDraggable({
    id: item.id,
    disabled: !draggable,
  });
  return (
    <span
      ref={setNodeRef}
      style={transform ? { transform: `translate3d(${transform.x}px, ${transform.y}px, 0)` } : undefined}
      className={`flex max-w-64 items-center gap-1 rounded-md border px-1.5 py-1 text-[11px] ${
        isDragging ? "z-50 opacity-80 shadow-lg" : ""
      }`}
    >
      {draggable && (
        <button
          {...listeners}
          {...attributes}
          className="shrink-0 cursor-grab text-muted-foreground/60 hover:text-foreground active:cursor-grabbing"
        >
          <GripVertical className="size-3" />
        </button>
      )}
      <span className={`size-1.5 shrink-0 rounded-full ${dotOf(item.tone)}`} />
      <button onClick={onOpen} className="flex min-w-0 items-center gap-1 text-left">
        {item.label && (
          <span className="shrink-0 text-[10px] text-muted-foreground">{item.label}</span>
        )}
        <span className="truncate">{item.title}</span>
      </button>
    </span>
  );
}

/**
 * El calendario del teléfono, como el de iOS: el mes compacto con un punto
 * por cosa en cada día. Tocar un día sube una hoja con lo de ese día, con los
 * títulos enteros: así no hay que bajar a buscarlo y el mes no se mueve (jose,
 * 18-oct-2026). Lo que no tiene fecha, detrás de una fila que abre la suya. Sin
 * arrastrar: en una pantalla táctil arrastrar compite con deslizar, y la fecha
 * se pone desde la tarea.
 */
function CompactMonth({
  header,
  items: all,
  cells,
  cursor,
  today,
  byDay,
  picked,
  onPick,
  undated,
  onOpen,
  countKey,
  dotOf,
}: {
  header: React.ReactNode;
  items: CalendarItem[];
  cells: Date[];
  cursor: Date;
  today: string;
  byDay: Map<string, CalendarItem[]>;
  picked: string;
  onPick: (k: string) => void;
  undated: CalendarEntry[];
  onOpen: (id: string) => void;
  countKey: MessageKey;
  dotOf: (tone: string) => string;
}) {
  const { t } = useT();
  const [sheet, setSheet] = useState<"day" | "undated" | null>(null);
  const items = byDay.get(picked) ?? [];
  const pickedDate = cells.find((d) => dayKey(d) === picked) ?? null;

  // Lo que viene, desde hoy: debajo del mes había un hueco, y es lo primero
  // que se quiere saber al abrir un calendario (jose, 18-oct-2026).
  const upcoming = useMemo(() => {
    const now = new Date();
    const start = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    return all
      .map((r) => ({ r, d: localDayOf(r) }))
      .filter((x): x is { r: CalendarItem; d: Date } => !!x.d && x.d.getTime() >= start)
      .sort((a, b) => a.d.getTime() - b.d.getTime())
      .slice(0, UPCOMING);
  }, [all]);

  // Abrir una tarea cierra la hoja: lo que se pidió ya está en pantalla.
  const open = (id: string) => {
    setSheet(null);
    onOpen(id);
  };

  return (
    <div className="flex flex-col gap-3">
      {header}
      <div className="grid grid-cols-7 text-center">
        {weekdayInitials().map((w) => (
          <div key={w} className="pb-1 text-[11px] font-medium text-muted-foreground">
            {w}
          </div>
        ))}
        {cells.map((d) => {
          const k = dayKey(d);
          const dayItems = byDay.get(k) ?? [];
          const inMonth = d.getMonth() === cursor.getMonth();
          return (
            <button
              key={k}
              type="button"
              onClick={() => {
                onPick(k);
                setSheet("day");
              }}
              data-day={k}
              aria-pressed={k === picked}
              aria-label={`${fecha(d)} · ${t(countKey, { count: dayItems.length })}`}
              className={cn(
                "motion-press flex h-11 flex-col items-center justify-center gap-1 rounded-lg",
                !inMonth && "opacity-35",
                k === picked && "bg-accent",
              )}
            >
              <span
                className={cn(
                  "grid size-6 place-items-center rounded-full text-sm",
                  k === today ? "bg-primary font-semibold text-primary-foreground" : k === picked && "font-semibold",
                )}
              >
                {d.getDate()}
              </span>
              {/* Un punto por cosa, hasta tres: dice si hay algo y de qué color
                  sin tener que leer nada. */}
              <span className="flex h-1 gap-0.5">
                {dayItems.slice(0, 3).map((r) => (
                  <span key={r.id} className={cn("size-1 rounded-full", dotOf(r.tone))} />
                ))}
              </span>
            </button>
          );
        })}
      </div>

      {upcoming.length > 0 && (
        <section className="space-y-1.5">
          <p className="text-xs font-medium text-muted-foreground">{t("common:calendar.upcoming")}</p>
          {upcoming.map(({ r, d }) => (
            <button
              key={r.id}
              type="button"
              onClick={() => onOpen(r.id)}
              className="flex w-full items-start gap-2.5 rounded-lg border bg-raised p-2.5 text-left text-sm"
            >
              <span className="w-12 shrink-0 text-xs text-muted-foreground tabular-nums">
                {dayKey(d) === today ? t("common:calendar.today") : diaYMes(d)}
              </span>
              <span className={cn("mt-1.5 size-2 shrink-0 rounded-full", dotOf(r.tone))} />
              <span className="line-clamp-2 min-w-0 flex-1">
                {r.label && <span className="mr-1 font-mono text-xs text-muted-foreground">{r.label}</span>}
                {r.title}
              </span>
            </button>
          ))}
        </section>
      )}

      {undated.length > 0 && (
        <button
          type="button"
          onClick={() => setSheet("undated")}
          className="flex w-full items-center justify-between rounded-lg border px-3 py-2.5 text-sm text-muted-foreground"
        >
          {t("common:calendar.undated", { count: undated.length })}
          <ChevronRight className="size-4" />
        </button>
      )}

      <Sheet open={sheet !== null} onOpenChange={(o) => !o && setSheet(null)}>
        <SheetContent
          side="bottom"
          className="gap-3 rounded-t-xl p-4 pb-[max(1rem,env(safe-area-inset-bottom))] data-[side=bottom]:max-h-[75dvh]"
        >
          <div className="mx-auto h-1 w-10 shrink-0 rounded-full bg-muted-foreground/30" aria-hidden />
          <SheetHeader className="p-0">
            <SheetTitle className="capitalize">
              {sheet === "undated"
                ? t("common:calendar.undated", { count: undated.length })
                : pickedDate
                  ? fecha(pickedDate)
                  : picked}
            </SheetTitle>
            <SheetDescription>
              {t(countKey, { count: sheet === "undated" ? undated.length : items.length })}
            </SheetDescription>
          </SheetHeader>
          <div className="-mx-1 min-h-0 flex-1 space-y-1.5 overflow-y-auto px-1">
            {(sheet === "undated" ? undated : items).length === 0 ? (
              <p className="py-4 text-center text-sm text-muted-foreground">{t("common:calendar.nothingThatDay")}</p>
            ) : (
              (sheet === "undated" ? undated : items).map((r) => (
                <button
                  key={r.id}
                  type="button"
                  onClick={() => open(r.id)}
                  className="flex w-full items-start gap-2 rounded-lg border bg-raised p-2.5 text-left text-sm"
                >
                  <span className={cn("mt-1.5 size-2 shrink-0 rounded-full", dotOf(r.tone))} />
                  {r.label && <span className="mt-0.5 shrink-0 font-mono text-xs text-muted-foreground">{r.label}</span>}
                  <span className="line-clamp-2">{r.title}</span>
                </button>
              ))
            )}
          </div>
        </SheetContent>
      </Sheet>
    </div>
  );
}
