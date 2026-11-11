import {
  DndContext,
  DragOverlay,
  PointerSensor,
  KeyboardSensor,
  pointerWithin,
  useSensor,
  useSensors,
} from "@dnd-kit/core";

import { mesYAno } from "@/lib/fechas";
import { toISODate, fromISODate } from "@/lib/month";
import { monthWeeks, weekDays, weekLabel, weekStart } from "@/lib/calendar";
import { useCalendarStore } from "@/store/calendar.store";
import CalendarHeader from "@/components/calendar/CalendarHeader";
import MonthGrid from "@/components/calendar/MonthGrid";
import WeekView, { WeekSummary } from "@/components/calendar/WeekView";
import { CardBody } from "@/components/calendar/CalendarCard";
import ScheduleLane, { LANE_ID } from "@/components/calendar/ScheduleLane";
import MobileCalendar from "@/components/calendar/MobileCalendar";
import { useT, type MessageKey } from "@/lib/i18n";
import { useMemo, useState } from "react";
import { useIsMobile } from "@/hooks/use-mobile";
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


export default function ItemCalendar({
  items,
  tones,
  calendarId,
  undated,
  onSchedule,
  onUnschedule,
  onOpen,
  onCreateForDay,
  countKey = "common:count.items",
  mixLabel,
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
  /**
   * Quitarle la fecha a una soltándola en «Por programar». Sin esto el carril
   * no acepta lo que tiene fecha.
   */
  onUnschedule?: (id: string) => void;
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
  /**
   * El título de la mezcla de tonos en el resumen de la semana. Por omisión,
   * «Mezcla de prioridad»; las reuniones lo cambian.
   */
  mixLabel?: string;
}) {
  const { t } = useT();
  const view = useCalendarStore((s) => s.views[calendarId] ?? "month");
  const setView = useCalendarStore((s) => s.setView);
  const laneOpen = useCalendarStore((s) => s.laneOpen(calendarId, view));
  const setLaneOpen = useCalendarStore((s) => s.setLaneOpen);
  // El carril sólo existe donde se puede poner fecha: Mi trabajo. El tablero
  // coloca por creación y las reuniones se reprograman en su ficha.
  const hasLane = !!onSchedule && undated !== undefined;
  const pending = undated ?? [];
  const [cursor, setCursor] = useState(() => {
    const n = new Date();
    return new Date(n.getFullYear(), n.getMonth(), 1);
  });
  // La semana que se mira, por su lunes. Aparte del mes: «Hoy» lleva las dos a
  // la actual, y pasar de Semana a Mes abre el mes de la semana visible.
  const [week, setWeek] = useState(() => weekStart(new Date()));
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

  // Por día local, `YYYY-MM-DD`: es además el id de cada día como destino de
  // arrastre.
  const byISO = useMemo(() => {
    const m = new Map<string, CalendarItem[]>();
    for (const r of items) {
      const d = localDayOf(r);
      if (!d) continue;
      const i = toISODate(d);
      (m.get(i) ?? m.set(i, []).get(i)!).push(r);
    }
    return m;
  }, [items]);

  // Las semanas justas del mes (`monthWeeks`, ver su comentario).
  const weeks = useMemo(() => monthWeeks(cursor), [cursor]);
  const days = useMemo(() => weekDays(week), [week]);
  const weekItems = useMemo(() => days.flatMap((d) => byISO.get(toISODate(d)) ?? []), [days, byISO]);
  // Lo que viaja con el puntero: una copia de la tarjeta que se arrastra.
  const dragged = dragging ? [...items, ...pending].find((x) => x.id === dragging) : undefined;

  const todayISO = toISODate(new Date());
  // En el teléfono, otro calendario: el mes de escritorio no cabe (cada día
  // queda de 60 px y una tarea se lee «#14 p…»).
  const compact = useIsMobile();

  const goToday = () => {
    const n = new Date();
    setCursor(new Date(n.getFullYear(), n.getMonth(), 1));
    setWeek(weekStart(n));
  };
  const step = (dir: 1 | -1) => {
    if (view === "week") setWeek((w) => new Date(w.getFullYear(), w.getMonth(), w.getDate() + 7 * dir));
    else setCursor((c) => new Date(c.getFullYear(), c.getMonth() + dir, 1));
  };

  if (compact) {
    return (
      <MobileCalendar
        view={view}
        onView={(v) => setView(calendarId, v)}
        byDay={byISO}
        items={items}
        pending={pending}
        hasLane={hasLane}
        tones={tones}
        today={todayISO}
        onOpen={onOpen}
        onSchedule={onSchedule}
        onUnschedule={onUnschedule}
        onCreateForDay={onCreateForDay}
        countKey={countKey}
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
        lane={
          hasLane
            ? { open: laneOpen, count: pending.length, onToggle: () => setLaneOpen(calendarId, view, !laneOpen) }
            : undefined
        }
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
          if (!e.over) return;
          const id = String(e.active.id);
          // En el carril se le quita la fecha, si la tenía; en un día, su id
          // **es** la fecha.
          if (e.over.id === LANE_ID) {
            if (onUnschedule && !pending.some((x) => x.id === id)) onUnschedule(id);
            return;
          }
          onSchedule?.(id, String(e.over.id));
        }}
        onDragCancel={() => setDragging(null)}
      >
        {view === "week" && (
          <WeekSummary
            week={weekItems}
            all={items}
            tones={tones}
            countKey={countKey}
            mixLabel={mixLabel ?? t("common:calendar.priorityMix")}
          />
        )}
        <div className="flex min-w-0 items-start gap-3">
          {hasLane && (
            <ScheduleLane
              open={laneOpen}
              items={pending}
              tones={tones}
              dragging={dragging !== null}
              canUnschedule={!!onUnschedule}
              onClose={() => setLaneOpen(calendarId, view, false)}
              onOpen={onOpen}
              onSchedule={onSchedule!}
            />
          )}
          <div className="min-w-0 flex-1">
            {view === "week" ? (
              <WeekView
                days={days}
                byDay={byISO}
                tones={tones}
                today={todayISO}
                dragging={dragging !== null}
                canDrop={!!onSchedule}
                onOpen={onOpen}
              />
            ) : (
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
            )}
          </div>
        </div>
        <DragOverlay dropAnimation={null}>
          {dragged ? (
            <div className="w-[200px] cursor-grabbing shadow-lg">
              <CardBody item={dragged} tones={tones} />
            </div>
          ) : null}
        </DragOverlay>
      </DndContext>
    </div>
  );
}
