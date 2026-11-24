import { fireEvent, waitFor } from "@testing-library/react";

/**
 * Elige una opción en un desplegable de la app (`Picker`, `Select`), como lo
 * hace una persona: abrirlo, pasar el ratón por la opción y hacer clic. Sin el
 * paso del ratón no elige: base-ui sólo acepta el clic sobre una opción
 * resaltada, y una prueba no «pasa por encima» si no se lo dice. Sustituye al
 * `fireEvent.change` de un `<select>` nativo.
 */
export async function pick(trigger: HTMLElement, option: string | RegExp): Promise<void> {
  fireEvent.click(trigger);
  const op = await waitFor(() => {
    const found = options().find((o) => matches(o.textContent ?? "", option));
    if (!found) throw new Error(`no option ${String(option)} among ${options().length}`);
    return found;
  });
  fireEvent.mouseMove(op);
  fireEvent.click(op);
}

/**
 * Las opciones abiertas, por su rol pero sin `getAllByRole`: ése calcula para
 * cada una si se ve y cómo se llama, y con las ~400 zonas horarias eran medio
 * segundo por búsqueda; con la máquina cargada, la prueba pasaba de sus 5 s.
 * El nombre de una opción es su texto.
 */
export function options(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('[role="option"]')];
}

function matches(text: string, option: string | RegExp): boolean {
  return typeof option === "string" ? text.trim() === option : option.test(text.trim());
}
