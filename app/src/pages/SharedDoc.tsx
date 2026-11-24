import { useEffect, useMemo, useState } from "react";
import { FileText } from "lucide-react";

import { Brand } from "@/components/brand/Brand";
import Markdown from "@/components/markdown/Markdown";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { DocSkeleton } from "@/components/ui/skeletons";
import { tokenFromHash } from "@/lib/call-link";
import { fechaYHora } from "@/lib/fechas";
import { useT } from "@/lib/i18n";
import {
  childrenOf,
  citedAttachmentIds,
  fetchSharedAttachment,
  fetchSharedDoc,
  pageIdOf,
  rewriteAttachmentUrls,
  sharedAttachmentId,
  SharedDocGone,
  type SharedDoc as SharedDocData,
  type SharedPage,
} from "@/lib/shared-doc";
import { cn } from "@/lib/utils";

/**
 * Una página de docs compartida con un enlace: `/app/shared#<token>`.
 *
 * Como `JoinCall`, fuera de `ProtectedRoute` y sin sesión: lo que abre es el
 * token, y sólo para leer. La página y sus subpáginas, navegables, con los
 * adjuntos que citan.
 */
export default function SharedDoc() {
  const { t } = useT();
  const [token] = useState(() => tokenFromHash(window.location.hash));
  const [doc, setDoc] = useState<SharedDocData | null>(null);
  const [failure, setFailure] = useState<"gone" | "failed" | null>(null);
  const [current, setCurrent] = useState<string | null>(null);

  useEffect(() => {
    if (!token) {
      setFailure("gone");
      return;
    }
    fetchSharedDoc(token)
      .then((d) => {
        setDoc(d);
        setCurrent(d.pages[0]?.id ?? null);
      })
      .catch((e) => setFailure(e instanceof SharedDocGone ? "gone" : "failed"));
  }, [token]);

  if (failure || !doc) {
    return (
      <div className="flex min-h-dvh flex-col items-center justify-center gap-6 bg-background px-4 py-8">
        <Brand />
        <Card className="w-full max-w-md">
          {failure === "gone" ? (
            <CardHeader>
              <CardTitle>{t("work:sharedDoc.goneTitle")}</CardTitle>
              <CardDescription role="alert">{t("work:sharedDoc.goneBody")}</CardDescription>
            </CardHeader>
          ) : failure === "failed" ? (
            <CardHeader>
              <CardDescription role="alert">{t("work:sharedDoc.failed")}</CardDescription>
            </CardHeader>
          ) : (
            <CardContent className="py-6">
              <DocSkeleton label={t("work:sharedDoc.loading")} />
            </CardContent>
          )}
        </Card>
      </div>
    );
  }

  const page = doc.pages.find((p) => p.id === current) ?? doc.pages[0];
  return (
    <div className="min-h-dvh bg-background">
      <header className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b px-4 py-3">
        <Brand />
        <span className="text-xs text-muted-foreground">
          {t("work:sharedDoc.readOnly")} · {t("work:sharedDoc.expires", { when: fechaYHora(doc.expiresAt) })}
        </span>
      </header>
      <div className="mx-auto flex max-w-6xl flex-col gap-6 px-4 py-6 md:flex-row">
        {doc.pages.length > 1 && (
          <nav aria-label={t("work:sharedDoc.pages")} className="shrink-0 md:w-60">
            <PageTree pages={doc.pages} current={page.id} onPick={setCurrent} />
          </nav>
        )}
        <main className="prose-doc min-w-0 flex-1">
          <h1 className="mb-4 text-2xl font-semibold">{page.title}</h1>
          <PageBody
            key={page.id}
            token={token}
            body={page.body}
            onPage={(id) => {
              if (doc.pages.some((p) => p.id === id)) setCurrent(id);
            }}
          />
        </main>
      </div>
    </div>
  );
}

function PageTree({ pages, current, onPick }: { pages: SharedPage[]; current: string; onPick: (id: string) => void }) {
  const kids = useMemo(() => childrenOf(pages), [pages]);
  const branch = (parent: string | null, depth: number) =>
    (kids.get(parent) ?? []).map((p) => (
      <li key={p.id}>
        <button
          type="button"
          aria-current={p.id === current ? "page" : undefined}
          onClick={() => onPick(p.id)}
          style={{ paddingLeft: `${depth * 12 + 8}px` }}
          className={cn(
            "flex w-full items-center gap-1.5 rounded-md py-1 pr-2 text-left text-sm hover:bg-muted",
            p.id === current ? "bg-muted font-medium" : "text-muted-foreground",
          )}
        >
          <FileText className="size-3.5 shrink-0" />
          <span className="truncate">{p.title}</span>
        </button>
        {kids.has(p.id) && <ul>{branch(p.id, depth + 1)}</ul>}
      </li>
    ));
  return <ul className="flex flex-col gap-0.5">{branch(null, 0)}</ul>;
}

/**
 * El cuerpo de una página, con sus adjuntos ya bajados. Se pinta cuando han
 * terminado todos —bien o mal—: pintarlo antes mostraría cada imagen como
 * borrada hasta que llegara.
 */
function PageBody({ token, body, onPage }: { token: string; body: string; onPage: (id: string) => void }) {
  const { t } = useT();
  const ids = useMemo(() => citedAttachmentIds(body), [body]);
  const [urls, setUrls] = useState<Record<string, string | undefined> | null>(ids.length ? null : {});

  useEffect(() => {
    if (!ids.length) return;
    let alive = true;
    const made: string[] = [];
    Promise.all(
      ids.map((id) =>
        fetchSharedAttachment(token, id)
          .then((blob) => {
            const url = URL.createObjectURL(blob);
            if (alive) made.push(url);
            else URL.revokeObjectURL(url);
            return [id, url] as const;
          })
          .catch(() => [id, undefined] as const),
      ),
    ).then((pairs) => {
      if (alive) setUrls(Object.fromEntries(pairs));
    });
    return () => {
      alive = false;
      made.forEach((u) => URL.revokeObjectURL(u));
    };
  }, [ids, token]);

  const text = useMemo(() => rewriteAttachmentUrls(body), [body]);
  if (!urls) {
    return <DocSkeleton label={t("work:sharedDoc.loading")} />;
  }
  const blobOf = (src: string) => {
    const id = sharedAttachmentId(src);
    return id ? (urls[id] ?? null) : src;
  };
  return (
    <Markdown
      imageSrc={blobOf}
      onInternalLink={(href) => {
        // Nada de aquí navega la pestaña: quien lee no tiene sesión, y un
        // enlace de dentro de la app (`cac:`, `/tasks?…`) le llevaría al login.
        // Sólo se sigue lo que es de fuera, un adjunto ya bajado y una página
        // del propio árbol.
        const pageId = pageIdOf(href);
        const blob = sharedAttachmentId(href) ? blobOf(href) : null;
        if (pageId) onPage(pageId);
        else if (blob) window.open(blob, "_blank", "noopener,noreferrer");
        else if (/^https?:/.test(href) && !href.startsWith(window.location.origin + "/")) {
          window.open(href, "_blank", "noopener,noreferrer");
        }
        return true;
      }}
    >
      {text}
    </Markdown>
  );
}
