import { useEffect, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";

import PanelFilter from "@/components/sidebar/PanelFilter";
import PanelRow from "@/components/sidebar/PanelRow";
import SpacesNavigator from "@/components/tree/SpacesNavigator";
import { useT } from "@/lib/i18n";
import { navItem } from "@/lib/nav";
import { useMyWorkStore } from "@/store/mywork.store";
import { inOrg } from "@/lib/requests";
import { useAgentRequestsStore } from "@/store/agent-requests.store";
import { useOrgsStore } from "@/store/orgs.store";

/**
 * Trabajo: lo tuyo, el resumen, las notas y el árbol de espacios.
 *
 * Las tres filas fijas van antes del árbol porque son lo que se abre más y
 * no dependen de en qué espacio estés. Notas está aquí y no en el riel a
 * propósito: es trabajo, y un quinto icono por una pantalla que va a cambiar
 * de sitio en la siguiente tanda sería un icono que luego hay que quitar.
 */
const ROWS = ["/my-work", "/requests", "/overview", "/notes"].map(navItem);

export default function WorkPanel() {
  const { t } = useT();
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const setScope = useMyWorkStore((s) => s.setScope);
  const [query, setQuery] = useState("");
  // Cuántos pedidos te esperan, en «Por atender». Se lee al montar y luego lo
  // mueven los eventos `request:*`.
  const currentOrgId = useOrgsStore((s) => s.currentOrgId);
  const waiting = useAgentRequestsStore((s) => inOrg(s.mineOpen, currentOrgId).length);
  const refreshCount = useAgentRequestsStore((s) => s.refreshCount);
  useEffect(() => {
    void refreshCount();
  }, [refreshCount]);

  return (
    <div className="flex flex-col">
      <div className="px-1 pb-1">
        {ROWS.map((item) => (
          <PanelRow
            key={item.path}
            icon={<item.icon />}
            label={t(item.labelKey)}
            badge={item.path === "/requests" ? waiting : undefined}
            active={pathname === item.path || pathname.startsWith(item.path + "/")}
            onClick={() => {
              // Entrar por aquí es pedir «mi trabajo», todo él. El filtro lo
              // pone pulsar una lista en el árbol, y hasta ahora la única forma
              // de quitárselo era la píldora: venir por esta fila y seguir
              // viendo una lista se lee como que no tienes nada más.
              if (item.path === "/my-work") setScope(null);
              navigate(item.path);
            }}
          />
        ))}
      </div>
      <PanelFilter value={query} onChange={setQuery} />
      <SpacesNavigator filter={query} />
    </div>
  );
}
