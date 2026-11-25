import { create } from "zustand";

import {
  answerRequest,
  listRequests,
  type AgentRequest,
  type RequestAnswer,
  type RequestScope,
} from "@/lib/requests";

/**
 * «Por atender»: los pedidos abiertos y los resueltos de quien mira (o del
 * equipo), y cuántos le esperan a él para el contador de la navegación.
 *
 * El contador va aparte de la lista a propósito: con el filtro «Equipo» la
 * lista enseña los de todos, y el número de la barra tiene que seguir diciendo
 * cuántos son **tuyos**.
 */
interface AgentRequestsState {
  scope: RequestScope;
  open: AgentRequest[];
  closed: AgentRequest[];
  mineCount: number;
  loaded: boolean;
  loading: boolean;
  error: string | null;
  /** Lo que un agente pidió abrir (`request:open`), hasta que la pantalla lo enseñe. */
  pendingOpen: { code: string; nonce: string } | null;
  setScope: (scope: RequestScope) => void;
  load: () => Promise<void>;
  refreshCount: () => Promise<void>;
  /** Algo cambió en el servidor (un evento `request:*`). */
  onChange: () => void;
  answer: (code: string, answer: RequestAnswer) => Promise<AgentRequest>;
  askToOpen: (code: string, nonce: string) => void;
  clearPendingOpen: () => void;
}

export const useAgentRequestsStore = create<AgentRequestsState>()((set, get) => ({
  scope: "mine",
  open: [],
  closed: [],
  mineCount: 0,
  loaded: false,
  loading: false,
  error: null,
  pendingOpen: null,

  setScope: (scope) => {
    set({ scope });
    void get().load();
  },

  load: async () => {
    const scope = get().scope;
    set({ loading: true, error: null });
    try {
      const [open, closed] = await Promise.all([listRequests(scope, "open"), listRequests(scope, "closed")]);
      // Si mientras tanto cambiaron de filtro, esta respuesta ya no es la suya.
      if (get().scope !== scope) return;
      set({ open, closed, loaded: true, loading: false });
      if (scope === "mine") set({ mineCount: open.length });
    } catch (e) {
      set({ loading: false, error: e instanceof Error ? e.message : String(e) });
    }
  },

  refreshCount: async () => {
    try {
      const mine = await listRequests("mine", "open");
      set({ mineCount: mine.length });
    } catch {
      // El contador es un extra: sin red se queda el último.
    }
  },

  onChange: () => {
    void get().refreshCount();
    if (get().loaded) void get().load();
  },

  answer: async (code, answer) => {
    const done = await answerRequest(code, answer);
    // Pasa a «Resueltos» en el acto; la recarga del evento confirma.
    set((s) => ({
      open: s.open.filter((r) => r.code !== code),
      closed: [done, ...s.closed.filter((r) => r.code !== code)],
      mineCount: s.open.some((r) => r.code === code && r.canAnswer) ? Math.max(0, s.mineCount - 1) : s.mineCount,
    }));
    return done;
  },

  askToOpen: (code, nonce) => set({ pendingOpen: { code, nonce } }),
  clearPendingOpen: () => set({ pendingOpen: null }),
}));
