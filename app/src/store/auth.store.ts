import { isTauri } from "@/lib/platform";
import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Session } from "@/types/auth";

interface AuthState {
  session: Session | null;
  accessToken: string | null;
  refreshToken: string | null;
  // guest: the user chose "continue as guest" from the login screen. Grants
  // access to the on-device tools (Image Tool, Crypto Tools) without signing in.
  guest: boolean;
  setAuth: (session: Session, accessToken: string, refreshToken: string) => void;
  setSession: (session: Session) => void;
  clearAuth: () => void;
  continueAsGuest: () => void;
  isAuthenticated: () => boolean;
  isGuest: () => boolean;
}


/**
 * En el escritorio la sesión es del core en Rust (`cac-core`), no de aquí.
 *
 * Este store era el dueño: guardaba el par de tokens en `localStorage` y se
 * lo copiaba a Rust. Ahora es al revés. El core guarda el par, lo renueva y
 * lo persiste; aquí queda una **copia en memoria** de quién está dentro y del
 * token de acceso —que dura minutos, y que la pantalla usa para las subidas y
 * el stream de eventos—. El de refresh no llega nunca a la pantalla, y en
 * `localStorage` no queda ningún token.
 *
 * La copia la pone `startCoreSession` al arrancar y la mantiene el evento
 * `core://session`. Lo de abajo sólo le **dice** al core lo que pasó.
 *
 * En la versión web no hay core: sigue todo como estaba.
 */
function tellCore(command: string, args?: Record<string, unknown>) {
  void import("@tauri-apps/api/core")
    .then(({ invoke }) => invoke(command, args))
    .catch(() => {});
}

/** La forma en que zustand guarda este store en `localStorage`. */
const STORAGE_KEY = "cac-auth";

export interface LegacySession {
  session: Session;
  accessToken: string;
  refreshToken: string;
}

/**
 * La sesión que una versión anterior dejó en `localStorage`, si la dejó.
 *
 * Se lee **aquí, al cargar el módulo**, antes de crear el store: en el
 * escritorio el store ya no guarda tokens, así que el primer cambio de estado
 * los borra del almacenamiento. Leerla después sería llegar tarde, y quien
 * actualizara la app aparecería en la pantalla de login.
 */
function readLegacySession(): LegacySession | null {
  if (!isTauri) return null;
  try {
    const state = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null")?.state;
    if (state?.session && state.accessToken && state.refreshToken) {
      return {
        session: state.session,
        accessToken: state.accessToken,
        refreshToken: state.refreshToken,
      };
    }
  } catch {
    // Un almacenamiento ilegible es como uno vacío.
  }
  return null;
}

export const legacySession: LegacySession | null = readLegacySession();

export const useAuthStore = create<AuthState>()(
  persist(
    (set, get) => ({
      session: null,
      accessToken: null,
      refreshToken: null,
      guest: false,

      setAuth: (session, accessToken, refreshToken) => {
        if (isTauri) {
          // El par entero va al core; aquí no se queda el refresh.
          tellCore("core_sign_in", { session, accessToken, refreshToken });
          set({ session, accessToken, refreshToken: null, guest: false });
          return;
        }
        localStorage.setItem("access_token", accessToken);
        set({ session, accessToken, refreshToken, guest: false });
      },

      setSession: (session) => {
        if (isTauri) tellCore("core_update_session", { session });
        set({ session });
      },

      clearAuth: () => {
        // En el escritorio salir es cosa del core: revoca en el servidor y
        // borra lo guardado.
        if (isTauri) tellCore("core_logout");
        localStorage.removeItem("access_token");
        set({ session: null, accessToken: null, refreshToken: null });
      },

      continueAsGuest: () => set({ guest: true }),

      isAuthenticated: () => !!get().accessToken,

      // A guest is someone in guest mode who is NOT actually signed in.
      isGuest: () => get().guest && !get().accessToken,
    }),
    {
      name: STORAGE_KEY,
      // En el escritorio sólo se guarda si se eligió entrar como invitado: la
      // sesión y sus tokens son del core. Guardián: `auth.store.test.ts`.
      partialize: (state) =>
        isTauri
          ? { guest: state.guest }
          : {
              session: state.session,
              accessToken: state.accessToken,
              refreshToken: state.refreshToken,
              guest: state.guest,
            },
      // Y tampoco se **lee** de ahí: lo que una versión anterior dejó guardado
      // puede ser un par ya rotado, y arrancar con él es justo lo que cerraba
      // la sesión al actualizar. Lo recoge `startCoreSession`, una vez.
      merge: (persisted, current) => {
        const saved = (persisted ?? {}) as Partial<AuthState>;
        return isTauri ? { ...current, guest: saved.guest ?? false } : { ...current, ...saved };
      },
    }
  )
);
