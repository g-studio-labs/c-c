import { useEffect } from "react";
import { useNavigate } from "react-router-dom";

import { ackOpened } from "@/lib/requests";
import { useAgentRequestsStore } from "@/store/agent-requests.store";

/**
 * Un agente pidió abrir un pedido (`request:open`): se navega a «Por atender»
 * con él resaltado y se le dice al servidor que se enseñó, que es lo que el MCP
 * espera para contestar `delivered: true`. Sin esto, el agente diría que no hay
 * app abierta habiéndola.
 */
export function usePendingRequestOpen(): void {
  const pending = useAgentRequestsStore((s) => s.pendingOpen);
  const navigate = useNavigate();
  useEffect(() => {
    if (!pending) return;
    useAgentRequestsStore.getState().clearPendingOpen();
    navigate(`/requests?open=${encodeURIComponent(pending.code)}`);
    void ackOpened(pending.code, pending.nonce).catch(() => {});
  }, [pending, navigate]);
}
