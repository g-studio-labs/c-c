import type { CalendarTone } from "@/lib/calendar";
import { hueVar } from "@/lib/person-color";
import { cn } from "@/lib/utils";

/**
 * Las piezas pequeñas que repiten el popover, la semana y el carril
 * (command-and-control-333): el avatar, el punto de estado y la barra de
 * tonos. Las medidas son las del diseño (`docs/design/calendario.md`).
 */

// El tamaño de las iniciales por tamaño de avatar, como en el diseño: 8,5 px
// en la tarjeta de la semana (18), 9 px en la fila del popover (20).
const INITIALS_SIZE = { 18: "text-[8.5px]", 20: "text-[9px]", 22: "text-[10px]", 24: "text-[10px]" } as const;

/** Las iniciales sobre el color estable de la persona (`lib/person-color.ts`). */
export function CalendarAvatar({ initials, colorKey, size }: { initials: string; colorKey: string; size: 18 | 20 | 22 | 24 }) {
  return (
    <span
      aria-hidden
      style={{ backgroundColor: hueVar(colorKey), width: size, height: size }}
      className={cn(
        "grid flex-none place-items-center rounded-full font-bold text-primary-foreground",
        INITIALS_SIZE[size],
      )}
    >
      {initials}
    </span>
  );
}

/**
 * La barra segmentada por tono: un segmento por tono con elementos, de ancho
 * proporcional, en el orden de los tonos. `track` le pone el fondo de pista
 * que llevan la semana y el resumen; la celda del mes va sin él.
 */
export function ToneBar({
  segments,
  height,
  track = false,
  className,
}: {
  segments: { tone: CalendarTone; count: number }[];
  height: 3 | 4 | 5 | 6;
  track?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn("flex gap-[2px] overflow-hidden rounded-full", track && "bg-track", className)}
      style={{ height }}
    >
      {segments.map((s) => (
        <span key={s.tone.key} data-tone={s.tone.key} className={s.tone.dot} style={{ flex: s.count }} />
      ))}
    </div>
  );
}

/**
 * El número sin la palabra, para la celda del mes: «5» grande y «tareas»
 * pequeño. Sale del mismo mensaje que el contador entero («5 tareas») para no
 * duplicar en el catálogo el plural de cada cosa que se cuenta.
 */
export function unitOf(counted: string, n: number): string {
  return counted.replace(new RegExp(`^\\s*${n}\\s*`), "");
}
