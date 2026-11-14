import { legacySession, useAuthStore } from "@/store/auth.store";
import type { Session } from "@/types/auth";

const BASE_URL = import.meta.env.VITE_API_URL ?? "https://cac.guz-studio.dev";

/** Lo que el core cuenta de la sesión. El token de refresh no viene nunca. */
interface SessionView {
  session: Session | null;
  accessToken: string | null;
}

function mirror(view: SessionView) {
  useAuthStore.setState({
    session: view.session,
    accessToken: view.accessToken,
    refreshToken: null,
  });
}

/**
 * Arranca el core y pone su sesión en el store, **antes de pintar**.
 *
 * Sólo en el escritorio. Tres cosas, en este orden:
 *
 * 1. `core_init`: el core lee la sesión que tenga guardada.
 * 2. Si una versión anterior dejó una en `localStorage`, se le ofrece. El core
 *    la toma sólo si no tiene ninguna —la suya es más nueva por definición—.
 *    Sin este paso, actualizar la app mandaría a todo el mundo al login.
 * 3. Se copia al store lo que el core diga, y se escucha `core://session`
 *    para lo que cambie después (entrar, salir, renovar).
 *
 * Si el core no arranca, la app se queda sin sesión y enseña el login: es
 * mejor que pintar con una sesión que nadie va a poder renovar.
 */
export async function startCoreSession(): Promise<void> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");

  await listen<SessionView>("core://session", (event) => mirror(event.payload));
  await invoke<SessionView>("core_init", { baseUrl: BASE_URL });

  if (legacySession) {
    // Adopte o no, la copia vieja ya no hace falta: el `mirror` de abajo
    // reescribe lo guardado sin tokens. Un fallo aquí no puede dejar la app
    // sin pintar; lo peor que pasa es que esta persona vuelve a entrar.
    await invoke<boolean>("core_adopt_session", { ...legacySession }).catch(() => false);
    localStorage.removeItem("access_token");
  }

  mirror(await invoke<SessionView>("core_session"));
}
