import { Fragment, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";

import { copyText } from "@/lib/clipboard";
import { useT } from "@/lib/i18n";
import { requestSegments } from "@/lib/linkify";
import { isTauri, openExternal, openLocalPath } from "@/lib/platform";

const LINK = "text-primary underline-offset-2 hover:underline cursor-pointer";

/**
 * Un texto del pedido con sus enlaces vivos: direcciones al navegador, rutas de
 * la app dentro, y rutas de ficheros al sistema (sólo en escritorio: en la web
 * la ruta es de otra máquina, y se deja como código que se copia). Lo que se
 * copia con el botón de un bloque de comando es el texto crudo, no esto.
 */
export default function RequestText({ text }: { text: string }) {
  const { t } = useT();
  const navigate = useNavigate();
  const parts = useMemo(() => requestSegments(text), [text]);

  return (
    <>
      {parts.map((part, i) => {
        switch (part.kind) {
          case "text":
            return <Fragment key={i}>{part.text}</Fragment>;
          case "url":
            return (
              <a key={i} href={part.url} className={LINK} onClick={(e) => { e.preventDefault(); void openExternal(part.url).catch(() => {}); }}>
                {part.url}
              </a>
            );
          case "route":
            return (
              <a key={i} href={part.route} className={LINK} onClick={(e) => { e.preventDefault(); navigate(part.route); }}>
                {part.shown}
              </a>
            );
          case "path":
            return isTauri ? (
              <a key={i} href={part.path} className={LINK} onClick={(e) => { e.preventDefault(); void openLocalPath(part.path); }}>
                {part.shown}
              </a>
            ) : (
              <code
                key={i}
                className="cursor-copy rounded bg-muted px-1"
                title={part.path}
                onClick={() => void copyText(part.path).then(() => toast.success(t("work:requests.pathCopied")))}
              >
                {part.shown}
              </code>
            );
        }
      })}
    </>
  );
}
