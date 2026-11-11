import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";

/**
 * Lo que no tiene fecha no desaparece.
 *
 * Esta vista colocaba por vencimiento y **descartaba en silencio** todo lo que no
 * tuviera uno. Como casi nadie pone vencimientos, de sesenta y cinco tareas se
 * veía una, mientras la cabecera seguía diciendo sesenta y cinco. Un calendario
 * que esconde el 98% de lo que hay no está filtrando: está mintiendo.
 */

vi.mock("@dnd-kit/core", async () => {
  const real = await vi.importActual<Record<string, unknown>>("@dnd-kit/core");
  return {
    ...real,
    // El arrastre en sí no se puede ejercitar en jsdom —no hay punteros ni
    // geometría— así que lo que se comprueba aquí es lo demás: que la tira
    // exista, que liste lo que falta y que el día sea un destino.
    useDraggable: () => ({ attributes: {}, listeners: {}, setNodeRef: () => {}, transform: null, isDragging: false }),
    useDroppable: () => ({ setNodeRef: () => {}, isOver: false }),
  };
});

const ItemCalendar = (await import("@/components/ItemCalendar")).default;

// Los tonos de prueba se llaman como su clase: así las comprobaciones de color
// siguen leyendo la clase que pintan (command-and-control-334).
const TONES = ["bg-error", "bg-info", "bg-primary", "bg-muted"].map((c) => ({ key: c, label: c, dot: c, edge: c }));

const dated = {
  id: "a", title: "tiene fecha", day: "2026-09-07",
  tone: "bg-primary", label: "#7",
};
const undated = (n: number) =>
  Array.from({ length: n }, (_, i) => ({
    id: `s${i}`, title: `sin fecha ${i}`, tone: "bg-muted", label: `#${100 + i}`,
  }));

afterEach(cleanup);

/**
 * Desde command-and-control-337 lo sin fecha vive en el carril «Por
 * programar», no en una tira debajo del mes. Lo que esta prueba guarda sigue
 * siendo lo mismo: que no se trague, y que se diga cuánto hay aunque el carril
 * esté cerrado.
 */
describe("el calendario", () => {
  it("lleva al carril lo que no tiene fecha, en vez de tragárselo", () => {
    render(
      <ItemCalendar tones={TONES} calendarId="test" items={[dated]} undated={undated(3)} onOpen={() => {}} onSchedule={() => {}} />,
    );
    const lane = document.querySelector<HTMLElement>("[data-lane]")!;
    expect(within(lane).getByText("sin fecha 0")).toBeTruthy();
    expect(within(lane).getByText("sin fecha 2")).toBeTruthy();
  });

  it("y dice cuántas son, con el carril cerrado", () => {
    render(
      <ItemCalendar tones={TONES} calendarId="test" items={[dated]} undated={undated(64)} onOpen={() => {}} onSchedule={() => {}} />,
    );
    expect(screen.getByRole("button", { name: /To schedule/ }).textContent).toContain("64");
  });

  // El calendario del tablero coloca por fecha de creación, y ésa no se cambia
  // arrastrando: sin `onSchedule` no debe insinuar que se pueda.
  it("sin poder programar, no hay carril", () => {
    render(<ItemCalendar tones={TONES} calendarId="test" items={[dated]} undated={undated(2)} onOpen={() => {}} />);
    expect(document.querySelector("[data-lane]")).toBeNull();
    expect(screen.queryByText(/onto a day|a un día/i)).toBeNull();
  });

  it("sin nada suelto, el carril dice que todo tiene fecha", () => {
    render(<ItemCalendar tones={TONES} calendarId="test" items={[dated]} undated={[]} onOpen={() => {}} onSchedule={() => {}} />);
    expect(within(document.querySelector<HTMLElement>("[data-lane]")!).getByText("Everything has a date.")).toBeTruthy();
  });
});
