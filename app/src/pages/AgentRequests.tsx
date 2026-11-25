import { useEffect, useLayoutEffect, useRef, useState } from "react";
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

  // El enlace se atiende una sola vez, la primera vez que su pedido aparece:
  // si ya estaba resuelto, se abre «Resueltos», y se trae a la vista. Que
  // cambie la lista después (contestarlo, o que lleguen otros) no vuelve a
  // desplegar ni a mover la pantalla: así se perdía lo que se estaba leyendo.
  const handled = useRef<string | null>(null);
  const scrollPending = useRef(false);

  // Lo que llega o cambia no mueve lo que se lee: la primera tarjeta visible
  // se queda donde estaba aunque entren pedidos por encima.
  const scroller = useRef<HTMLDivElement>(null);
  const anchor = useRef<{ id: string; top: number } | null>(null);
  const captureAnchor = () => {
    const el = scroller.current;
    if (!el) return;
    const top = el.getBoundingClientRect().top;
    for (const card of el.querySelectorAll<HTMLElement>("[data-request-card]")) {
      const box = card.getBoundingClientRect();
      if (box.bottom > top) {
        anchor.current = { id: card.id, top: box.top - top };
        return;
      }
    }
    anchor.current = null;
  };
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    if (highlight && handled.current !== highlight) {
      const isClosed = closed.some((r) => r.code === highlight);
      if (isClosed || open.some((r) => r.code === highlight)) {
        handled.current = highlight;
        scrollPending.current = true;
        if (isClosed && !showResolved) {
          setShowResolved(true);
          return;
        }
      }
    }
    if (scrollPending.current) {
      scrollPending.current = false;
      document.getElementById(`request-${highlight}`)?.scrollIntoView?.({ block: "center" });
    } else if (anchor.current) {
      const card = document.getElementById(anchor.current.id);
      if (card && el.contains(card)) {
        el.scrollTop += card.getBoundingClientRect().top - el.getBoundingClientRect().top - anchor.current.top;
      }
    }
    captureAnchor();
  }, [highlight, open, closed, showResolved]);

  return (
    <div ref={scroller} onScroll={captureAnchor} className="mx-auto flex w-full max-w-3xl flex-col gap-4 overflow-y-auto p-4 md:p-6">
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
