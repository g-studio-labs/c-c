import type { ReactNode } from "react";
import { useDraggable } from "@dnd-kit/core";

import type { CalendarEntry } from "@/components/ItemCalendar";
import { CalendarAvatar } from "@/components/calendar/parts";
import type { CalendarTone } from "@/lib/calendar";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";

/**
 * La tarjeta de la semana y del carril «Por programar» (command-and-control-336):
 * el título entero, su folio, si venció, quién la lleva y su estado. Medidas
 * de `docs/design/calendario.md` §«Vista Semana» (Tarjeta).
 *
 * Arrastrable entera cuando se puede cambiar su fecha; con el umbral de ocho
 * píxeles del sensor, un clic sigue abriéndola. La barra de prioridad de 3 px
 * es un elemento encima del borde y no un `border-left` de color: el color del
 * tono llega como clase de fondo (`tone.edge`), que es como lo dan los tonos.
 */
export default function CalendarCard({
  item,
  tones,
  draggable,
  onOpen,
  dimmed = false,
  variant = "week",
  grip,
  footer,
}: {
  item: CalendarEntry;
  tones: CalendarTone[];
  draggable: boolean;
  onOpen: () => void;
  /** Un día que ya pasó: la tarjeta al 75%. */
  dimmed?: boolean;
  /**
   * La del carril no enseña el estado (lo que no tiene fecha no puede ir
   * tarde) y al pasar por encima se le marca el borde en vez del fondo.
   */
  variant?: "week" | "lane";
  /** El asa delante del folio, en el carril. */
  grip?: ReactNode;
  /** Lo de abajo, en el carril: «→ Hoy» y «Mañana». */
  footer?: ReactNode;
}) {
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({ id: item.id, disabled: !draggable });
  return (
    <div
      ref={setNodeRef}
      data-card={item.id}
      {...(draggable ? listeners : {})}
      {...(draggable ? attributes : {})}
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.target === e.currentTarget && e.key === "Enter") onOpen();
      }}
      className={cn(
        "outline-none focus-visible:ring-2 focus-visible:ring-ring",
        draggable ? "cursor-grab" : "cursor-pointer",
        // La de origen se queda al 40% mientras su copia viaja con el puntero.
        isDragging ? "opacity-40" : dimmed && "opacity-75",
      )}
    >
      <CardBody item={item} tones={tones} variant={variant} grip={grip} footer={footer} />
    </div>
  );
}

/** El cuerpo, aparte para pintar la copia que viaja con el puntero. */
export function CardBody({
  item,
  tones,
  variant = "week",
  grip,
  footer,
}: {
  item: CalendarEntry;
  tones: CalendarTone[];
  variant?: "week" | "lane";
  grip?: ReactNode;
  footer?: ReactNode;
}) {
  const { t } = useT();
  const tone = tones.find((x) => x.key === item.tone);
  const m = item.meta;
  return (
    <div
      className={cn(
        "relative flex flex-col gap-[6px] rounded-[7px] border bg-raised py-2 pr-2 pl-[10px]",
        variant === "lane" ? "hover:border-border-strong" : "hover:bg-accent",
        m?.overdue ? "border-error/55" : "border-border",
      )}
    >
      <span
        aria-hidden
        className={cn("absolute -top-px -bottom-px -left-px w-[3px] rounded-l-[7px]", tone?.edge ?? "bg-border-strong")}
      />
      <div className="flex items-center gap-[6px]">
        {grip}
        {item.label && <span className="font-mono text-[11px] text-muted-foreground">{item.label}</span>}
        {m?.overdue && (
          <span className="rounded-[4px] bg-error/12 px-[5px] text-[10px] font-semibold text-error">
            {t("common:calendar.overdue")}
          </span>
        )}
        {m?.assignee && (
          <span className="ml-auto">
            <CalendarAvatar initials={m.assignee.initials} colorKey={m.assignee.key} size={18} />
          </span>
        )}
      </div>
      <span
        className={cn(
          "text-[12.5px] leading-[17px] text-pretty",
          // En el carril no se tacha: lo que no tiene fecha está por hacer,
          // y el diseño lo pinta siempre igual.
          variant === "week" && m?.done ? "text-muted-foreground line-through" : "text-foreground",
        )}
      >
        {item.title}
      </span>
      {variant === "week" && m?.status && (
        <span className="flex items-center gap-[5px] text-[11px] text-muted-foreground">
          <span className={cn("size-[6px] rounded-full", m.status.dot)} />
          {m.status.name}
        </span>
      )}
      {footer}
    </div>
  );
}
