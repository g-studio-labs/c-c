import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";

import RequestCard from "@/components/requests/RequestCard";
import DisclosureChevron from "@/components/ui/disclosure-chevron";
import { Button } from "@/components/ui/button";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";
import { useAgentRequestsStore } from "@/store/agent-requests.store";

/**
 * «Por atender»: lo que los agentes esperan de una persona, con su botón
 * (command-and-control-504). Por defecto lo pedido a quien mira; «Equipo»
 * enseña también lo de los demás, que se lee sin poder contestarlo por ellos.
 * Lo resuelto baja a «Resueltos», plegado.
 *
 * `?open=req-…` resalta un pedido y lo trae a la vista: es a donde llevan el
 * aviso de la campana y `open_request` del MCP.
 */
export default function AgentRequests() {
  const { t } = useT();
  const { scope, open, closed, loading, loaded, error, setScope, load } = useAgentRequestsStore();
  const [params] = useSearchParams();
  const highlight = params.get("open")?.toLowerCase() ?? null;
  const [showResolved, setShowResolved] = useState(false);

  useEffect(() => {
    void load();
  }, [load]);

  // El pedido resaltado puede estar ya resuelto: entonces «Resueltos» se abre.
  useEffect(() => {
    if (highlight && closed.some((r) => r.code === highlight)) setShowResolved(true);
  }, [highlight, closed]);

  useEffect(() => {
    if (!highlight) return;
    document.getElementById(`request-${highlight}`)?.scrollIntoView?.({ block: "center", behavior: "smooth" });
  }, [highlight, open, closed, showResolved]);

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 overflow-y-auto p-4 md:p-6">
      <header className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <h1 className="text-xl font-semibold">{t("work:requests.title")}</h1>
          <p className="text-sm text-muted-foreground">{t("work:requests.lead")}</p>
        </div>
        <div role="tablist" aria-label={t("work:requests.scopeLabel")} className="flex rounded-lg border p-0.5">
          {(["mine", "team"] as const).map((s) => (
            <Button
              key={s}
              role="tab"
              aria-selected={scope === s}
              variant={scope === s ? "secondary" : "ghost"}
              size="sm"
              onClick={() => setScope(s)}
            >
              {t(s === "mine" ? "work:requests.scope.mine" : "work:requests.scope.team")}
            </Button>
          ))}
        </div>
      </header>

      {error && <p className="text-sm text-destructive">{t("work:requests.errLoad")}</p>}

      <section aria-label={t("work:requests.openLabel")} className="flex flex-col gap-3">
        {loaded && open.length === 0 && (
          <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
            {t(scope === "mine" ? "work:requests.empty" : "work:requests.emptyTeam")}
          </p>
        )}
        {!loaded && loading && <p className="text-sm text-muted-foreground">{t("work:requests.loading")}</p>}
        {open.map((r) => (
          <RequestCard key={r.code} request={r} highlighted={r.code === highlight} />
        ))}
      </section>

      {closed.length > 0 && (
        <section className="flex flex-col gap-3">
          <button
            type="button"
            aria-expanded={showResolved}
            className={cn("flex items-center gap-2 text-sm font-medium text-muted-foreground hover:text-foreground")}
            onClick={() => setShowResolved((v) => !v)}
          >
            <DisclosureChevron open={showResolved} className="size-4" />
            {t("work:requests.resolved", { count: closed.length })}
          </button>
          {showResolved && closed.map((r) => <RequestCard key={r.code} request={r} highlighted={r.code === highlight} />)}
        </section>
      )}
    </div>
  );
}
