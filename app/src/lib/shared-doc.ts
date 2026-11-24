import { apiUrl } from "@/lib/api";

/**
 * Una página compartida con un enlace (command-and-control-496), vista desde
 * fuera: `/app/shared#<token>`.
 *
 * El token va **siempre en la cabecera** `X-Doc-Share`, nunca en una URL: lo de
 * detrás del `#` no sale del navegador, y una cabecera no acaba en los logs de
 * nginx como acabaría una query. Por eso los adjuntos no pueden ir en un
 * `<img src>`: se piden con `fetch` y se pintan como blobs.
 */

export const DOC_SHARE_HEADER = "X-Doc-Share";

export interface SharedPage {
  id: string;
  parentId: string | null;
  title: string;
  body: string;
}

export interface SharedDoc {
  title: string;
  expiresAt: string;
  pages: SharedPage[];
}

/** El servidor contesta 404 a todo lo que no abre, sin decir por qué. */
export class SharedDocGone extends Error {
  constructor() {
    super("share-invalid");
  }
}

export async function fetchSharedDoc(token: string): Promise<SharedDoc> {
  const res = await fetch(apiUrl("/api/v1/shared-docs"), { headers: { [DOC_SHARE_HEADER]: token } });
  if (res.status === 404) throw new SharedDocGone();
  if (!res.ok) throw new Error(`shared-doc-${res.status}`);
  const body = (await res.json()) as { data?: SharedDoc };
  if (!body.data) throw new Error("shared-doc-empty");
  return body.data;
}

export async function fetchSharedAttachment(token: string, id: string): Promise<Blob> {
  const res = await fetch(apiUrl(`/api/v1/shared-docs/attachments/${encodeURIComponent(id)}`), {
    headers: { [DOC_SHARE_HEADER]: token },
  });
  if (!res.ok) throw new Error(`shared-attachment-${res.status}`);
  return res.blob();
}

/**
 * Cómo cita un cuerpo un adjunto de su documento: la ruta relativa que guarda
 * el editor (`domain.DocAttachmentRef`), o la misma con el dominio delante y
 * con un `?token=` colgando si se pegó desde la versión web.
 */
const DOC_ATTACHMENT = /(?:https?:\/\/[^\s()"'<>]*?)?\/api\/v1\/docs\/[^/\s()"'<>]+\/attachments\/([0-9a-fA-F-]{36})\/raw(?:\?[^\s()"'<>]*)?/g;

export function citedAttachmentIds(body: string): string[] {
  return [...new Set([...body.matchAll(DOC_ATTACHMENT)].map((m) => m[1].toLowerCase()))];
}

const SHARED_ROUTE = "/api/v1/shared-docs/attachments/";
const LINK_PREFIX = "#shared-attachment=";

/**
 * Cambia cada cita a un adjunto del documento por la ruta compartida
 * (`/api/v1/shared-docs/attachments/<id>`), que es la única que abre el enlace.
 *
 * Un **enlace** a un adjunto (no una imagen) va además a un fragmento: con una
 * ruta de `/api/` el pintor de markdown lo trataría como adjunto de la app y un
 * PDF se pediría sin la cabecera desde su tarjeta. Como fragmento es un enlace
 * corriente, y el clic lo abre el visor con su blob.
 *
 * No se pone el blob directamente: el pintor sólo deja pasar los esquemas
 * conocidos y vaciaría un `blob:`.
 */
export function rewriteAttachmentUrls(body: string): string {
  const inMarkdown = new RegExp(String.raw`(!?)\[([^\]]*)\]\(\s*(?:` + DOC_ATTACHMENT.source + String.raw`)\s*\)`, "g");
  return body
    .replace(inMarkdown, (_all, bang: string, label: string, id: string) =>
      bang ? `![${label}](${SHARED_ROUTE}${id.toLowerCase()})` : `[${label}](${LINK_PREFIX}${id.toLowerCase()})`,
    )
    .replace(DOC_ATTACHMENT, (_all, id: string) => SHARED_ROUTE + id.toLowerCase());
}

/** El adjunto al que apunta una URL ya reescrita, si apunta a uno. */
export function sharedAttachmentId(href: string): string | null {
  for (const prefix of [SHARED_ROUTE, LINK_PREFIX]) {
    const i = href.indexOf(prefix);
    if (i >= 0) return href.slice(i + prefix.length).split(/[?#&/]/)[0] || null;
  }
  return null;
}

/** Las hijas de cada página, en el orden en que vinieron (el del rango). */
export function childrenOf(pages: SharedPage[]): Map<string | null, SharedPage[]> {
  const out = new Map<string | null, SharedPage[]>();
  for (const p of pages) {
    const k = p.parentId ?? null;
    out.set(k, [...(out.get(k) ?? []), p]);
  }
  return out;
}

/** El id de página de un enlace de la app a una página de docs, si lo es. */
export function pageIdOf(href: string): string | null {
  const m = /[?&]page=([^&#]+)/.exec(href);
  return m ? decodeURIComponent(m[1]) : null;
}
