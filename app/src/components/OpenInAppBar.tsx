import { useEffect, useState } from "react";
import { useLocation } from "react-router-dom";
import { ExternalLink, X } from "lucide-react";
import { useT } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import { readChoice, NATIVE_RELEASES_URL, currentMode, launchApp, markTried, toAppUrl, writeChoice } from "@/lib/open-in-app";

/**
 * «Abrir en la app» en la versión web, como el de Slack o Linear. Nunca en
 * móvil ni dentro de la app de Tauri (`currentMode` lo decide).
 */
export default function OpenInAppBar() {
  const { t } = useT();
  const { pathname, search } = useLocation();
  const [mode, setMode] = useState(currentMode);
  const [remember, setRemember] = useState(false);
  // El router ya quitó la base (`/app`) de `pathname`: no se quita dos veces.
  const url = toAppUrl(pathname, search, "");

  const [launched, setLaunched] = useState(false);

  // «Siempre en la app» no puede lanzar nada sin gesto: la barra sale una vez.
  useEffect(() => {
    if (readChoice() === "app") markTried();
  }, []);

  if (mode !== "bar") return null;

  return (
    <div role="region" aria-label={t("common:openInApp.title")} className="flex items-center gap-3 border-b bg-muted/50 px-4 py-1.5 text-sm">
      <span className="min-w-0 flex-1 truncate">{t("common:openInApp.title")}</span>
      <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
        {t("common:openInApp.remember")}
      </label>
      <Button
        size="sm"
        onClick={() => {
          if (remember) writeChoice("app");
          launchApp(url);
          setLaunched(true);
        }}
      >
        <ExternalLink className="size-3.5" />
        {t("common:openInApp.open")}
      </Button>
      {launched && (
        <span className="text-xs text-muted-foreground">
          {t("common:openInApp.notOpened")}{" "}
          <a href={NATIVE_RELEASES_URL} target="_blank" rel="noopener noreferrer" className="underline">
            {t("common:openInApp.install")}
          </a>
        </span>
      )}
      <Button
        size="sm"
        variant="ghost"
        onClick={() => {
          if (remember) writeChoice("web");
          setMode("none");
        }}
      >
        {t("common:openInApp.stay")}
      </Button>
      <button type="button" aria-label={t("common:action.close")} className="text-muted-foreground" onClick={() => setMode("none")}>
        <X className="size-4" />
      </button>
    </div>
  );
}
