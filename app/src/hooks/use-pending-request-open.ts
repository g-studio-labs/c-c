import { useEffect } from "react";
import { useNavigate } from "react-router-dom";

import { enterOrg } from "@/lib/ir-en-org";
import { ackOpened } from "@/lib/requests";
import { useAgentRequestsStore } from "@/store/agent-requests.store";

/**
 * Un agente pidió abrir un pedido (`request:open`): se pasa a la organización
 * del pedido, se navega a «Por atender» con él resaltado y se le dice al
 * servidor que se enseñó, que es lo que el MCP espera para contestar
 * `delivered: true`. La lista sólo trae la organización que se mira, así que
 * sin cambiar antes la tarjeta no estaría; y de un pedido que no se encuentra
 * no se dice que se enseñó.
 */
export function usePendingRequestOpen(): void {
  const pending = useAgentRequestsStore((s) => s.pendingOpen);
  const navigate = useNavigate();
  useEffect(() => {
    if (!pending) return;
    useAgentRequestsStore.getState().clearPendingOpen();
    void (async () => {
      const code = pending.code.toLowerCase();
      const find = () => {
        const s = useAgentRequestsStore.getState();
        return [...s.mineOpen, ...s.open, ...s.closed].find((r) => r.code === code);
      };
      let request = find();
      if (!request) {
        await useAgentRequestsStore.getState().refreshCount();
        request = find();
      }
      if (request && !enterOrg(request.orgId)) return;
      navigate(`/requests?open=${encodeURIComponent(pending.code)}`);
      if (request) await ackOpened(pending.code, pending.nonce).catch(() => {});
    })();
  }, [pending, navigate]);
}
