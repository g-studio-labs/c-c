import { isTauri } from "@/lib/platform";

/**
 * Trae la ventana de la app al frente: la usa `request:open`, que es un agente
 * pidiendo que la persona mire un pedido ahora. En la web no se puede robar el
 * foco de otra pestaña; sólo se intenta con la propia.
 */
export async function bringToFront(): Promise<void> {
  if (!isTauri) {
    window.focus();
    return;
  }
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const w = getCurrentWindow();
    await w.unminimize();
    await w.show();
    await w.setFocus();
  } catch {
    // Un gestor de ventanas que no deja robar el foco no es un fallo: el
    // pedido igual queda abierto en la pantalla.
  }
}
