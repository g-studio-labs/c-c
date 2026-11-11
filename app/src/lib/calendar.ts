import { paraFormatear } from "@/lib/fechas";

/**
 * La aritmética y los resúmenes del calendario (command-and-control-333).
 *
 * Aparte del componente para probarla sola: los bordes —un mes que empieza en
 * domingo, una semana que cruza de mes o de año— son donde se equivoca, y una
 * prueba de componente no los recorre.
 *
 * Todo aquí trabaja con fechas **locales** a medianoche. Un vencimiento llega
 * ya resuelto por `dueDay` (ver `lib/month.ts`); pasarle un instante crudo
 * correría el día al oeste de Greenwich.
 */

/**
 * Un tono: lo que significa un color en este calendario.
 *
 * El calendario no sabe qué es una prioridad ni una reunión pausada: el
 * llamante le da la lista ordenada, y ese orden es el de los segmentos de la
 * barra y el de la leyenda. Las clases van escritas enteras en el llamante
 * porque Tailwind sólo genera las que encuentra literales.
 */
export interface CalendarTone {
  key: string;
  label: string;
  /** Fondo del punto, del segmento de la barra y del chip. */
  dot: string;
  /** Fondo de la barra lateral de 3 px de una fila o una tarjeta. */
  edge: string;
  /** Cuenta como «prioridad alta» en el resumen de la semana. */
  high?: boolean;
}

export interface ToneSegment {
  tone: CalendarTone;
  count: number;
}

/** Cuántos hay de cada tono, en el orden de los tonos y sin los vacíos. */
export function toneSegments(items: { tone: string }[], tones: CalendarTone[]): ToneSegment[] {
  return tones
    .map((tone) => ({ tone, count: items.filter((i) => i.tone === tone.key).length }))
    .filter((s) => s.count > 0);
}

/**
 * El mapa de calor de una celda del mes: 0 vacío, 1 una, 2 de dos a tres, 3
 * cuatro o más. Los cortes son los del diseño.
 */
export function heatLevel(n: number): 0 | 1 | 2 | 3 {
  if (n <= 0) return 0;
  if (n === 1) return 1;
  if (n <= 3) return 2;
  return 3;
}

/** El lunes de la semana de `d`, a medianoche local. */
export function weekStart(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() - ((d.getDay() + 6) % 7));
}

/** Los siete días de la semana que empieza en `start`. */
export function weekDays(start: Date): Date[] {
  return Array.from({ length: 7 }, (_, i) => new Date(start.getFullYear(), start.getMonth(), start.getDate() + i));
}

/**
 * Las semanas que ocupa un mes, empezando en lunes: cinco o seis, las que
 * hagan falta.
 *
 * `monthGrid` pinta siempre seis para que el botón de navegar no se mueva bajo
 * el cursor al cambiar de mes; en este calendario la navegación va **encima**
 * de la rejilla, así que esa razón no aplica y el diseño pide las justas.
 * `monthGrid` se queda como está para el selector de fecha.
 */
export function monthWeeks(cursor: Date): Date[] {
  const y = cursor.getFullYear();
  const m = cursor.getMonth();
  const offset = (new Date(y, m, 1).getDay() + 6) % 7;
  const days = new Date(y, m + 1, 0).getDate();
  const total = Math.ceil((offset + days) / 7) * 7;
  return Array.from({ length: total }, (_, i) => new Date(y, m, 1 - offset + i));
}

const fmt = (o: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat(paraFormatear(), o);

/**
 * El título de una semana: «9 – 15 noviembre 2026», y si cruza de mes «30 nov
 * – 6 diciembre 2026». Con `short`, el del teléfono: «9 – 15 nov».
 *
 * A mano y no con `formatRange`: en castellano éste dice «9–15 de noviembre de
 * 2026», y el diseño es el de arriba.
 */
export function weekLabel(start: Date, short = false): string {
  const end = new Date(start.getFullYear(), start.getMonth(), start.getDate() + 6);
  const long = fmt({ month: "long" });
  const abbr = fmt({ month: "short" });
  const month = (d: Date, s: boolean) => (s ? abbr : long).format(d).replace(/\.$/, "");
  const sameMonth = start.getMonth() === end.getMonth();
  const sameYear = start.getFullYear() === end.getFullYear();
  if (short) {
    return sameMonth
      ? `${start.getDate()} – ${end.getDate()} ${month(end, true)}`
      : `${start.getDate()} ${month(start, true)} – ${end.getDate()} ${month(end, true)}`;
  }
  if (sameMonth) return `${start.getDate()} – ${end.getDate()} ${month(end, false)} ${end.getFullYear()}`;
  const left = sameYear
    ? `${start.getDate()} ${month(start, true)}`
    : `${start.getDate()} ${month(start, true)} ${start.getFullYear()}`;
  return `${left} – ${end.getDate()} ${month(end, false)} ${end.getFullYear()}`;
}
