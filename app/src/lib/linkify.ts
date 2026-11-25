/**
 * Los enlaces de un texto plano, para pintarlos como enlaces sin pasar por
 * markdown. Lo usa el chat de la llamada, que es texto plano a propósito
 * (mientras alguien habla se pasa una frase o un enlace) y por eso pintaba las
 * URLs como texto: había que seleccionarlas y copiarlas a mano.
 *
 * Sólo `http://` y `https://`. La puntuación con la que se suele cerrar una frase
 * («mira https://x.com/a.») no es del enlace, y un `)` final sólo lo es si
 * abre otro dentro (`https://es.wikipedia.org/wiki/Foo_(bar)`).
 */
export type TextPart = { kind: "text"; text: string } | { kind: "link"; href: string };

const URL = /https?:\/\/[^\s<>"'`]+/g;
const TRAILING = /[.,;:!?'"»]+$/;

export function trimUrl(raw: string): string {
  let url = raw.replace(TRAILING, "");
  while (url.endsWith(")") && (url.match(/\(/g)?.length ?? 0) < (url.match(/\)/g)?.length ?? 0)) {
    url = url.slice(0, -1).replace(TRAILING, "");
  }
  return url;
}

export function linkify(text: string): TextPart[] {
  const out: TextPart[] = [];
  let last = 0;
  for (const m of text.matchAll(URL)) {
    const href = trimUrl(m[0]);
    const start = m.index ?? 0;
    if (start > last) out.push({ kind: "text", text: text.slice(last, start) });
    out.push({ kind: "link", href });
    last = start + href.length;
  }
  if (last < text.length) out.push({ kind: "text", text: text.slice(last) });
  return out;
}

/**
 * Lo mismo para los textos de un pedido, donde un agente escribe también rutas
 * de ficheros y enlaces a pantallas de la app. Espejo de `link_segments`
 * (`core/src/request_links.rs`): lo que uno reconoce, lo reconoce el otro.
 */
export type RequestSegment =
  | { kind: "text"; text: string }
  | { kind: "url"; url: string }
  | { kind: "path"; shown: string; path: string }
  | { kind: "route"; shown: string; route: string };

const APP_HOSTS = ["cac.guz-studio.dev"];
// `/app/build/out.log` es un fichero: sólo es de la app lo que nombra una pantalla suya.
const APP_SCREENS = ["tasks", "requests", "chat", "dm", "docs", "notes", "inbox", "reports"];
const LINK_END = /[\s<>"'`\]]/;
const LINK_START_AFTER = /[\s`"'(\[<=«*]/;

const isScreen = (route: string) => APP_SCREENS.includes(route.replace(/^\//, "").split(/[/?#]/)[0]);

function trimLink(raw: string): string {
  let out = raw.replace(/[.,;:!?»\]}*]+$/, "");
  while (out.endsWith(")") && (out.match(/\(/g)?.length ?? 0) < (out.match(/\)/g)?.length ?? 0)) {
    out = out.slice(0, -1).replace(/[.,;:!?»\]}*]+$/, "");
  }
  return out;
}

// Dos separadores seguidos son una ruta de red (UNC) en Windows: mirar si
// existe ya contacta al servidor. Se rechaza lo que se decodifica a eso.
const isNetwork = (path: string) => /^[/\\]{2}/.test(path);

function routeOfUrl(url: string): string | null {
  const m = /^https?:\/\/([^/]+)(\/.*)$/.exec(url);
  if (!m || !APP_HOSTS.includes(m[1]) || !m[2].startsWith("/app/")) return null;
  const route = m[2].slice("/app".length);
  return isScreen(route) ? route : null;
}

function classify(token: string): RequestSegment | null {
  if (/^https?:\/\/[^/\s]/.test(token)) {
    const route = routeOfUrl(token);
    return route ? { kind: "route", shown: token, route } : { kind: "url", url: token };
  }
  if (token.startsWith("file://")) {
    const rest = token.slice("file://".length).replace(/^localhost(?=\/)/, "");
    if (!rest.startsWith("/")) return null;
    let path = rest;
    try {
      path = decodeURIComponent(rest);
    } catch {
      /* un escape roto se deja como está */
    }
    if (!path.startsWith("/") || isNetwork(path)) return null;
    return { kind: "path", shown: token, path };
  }
  if (token.startsWith("/app/") && isScreen(token.slice("/app".length))) {
    return { kind: "route", shown: token, route: token.slice("/app".length) };
  }
  const rooted = token.startsWith("/") && !isNetwork(token) && token.length > 1;
  if (rooted || (token.startsWith("~/") && token.length > 2) || /^[A-Za-z]:[\\/]/.test(token)) {
    return { kind: "path", shown: token, path: token };
  }
  return null;
}

export function requestSegments(text: string): RequestSegment[] {
  const out: RequestSegment[] = [];
  let plain = "";
  let i = 0;
  while (i < text.length) {
    const prev = i === 0 ? null : text[i - 1];
    if (prev === null || LINK_START_AFTER.test(prev)) {
      const rest = text.slice(i);
      const end = rest.search(LINK_END);
      let raw = end === -1 ? rest : rest.slice(0, end);
      // `[/tmp/a](/tmp/b)`: el `(` abre el destino del enlace markdown. En una
      // dirección web puede ser de ella (`Foo_(bar)`).
      if (!rest.startsWith("http")) raw = raw.split("(")[0];
      const token = trimLink(raw);
      const seg = token ? classify(token) : null;
      if (seg) {
        if (plain) out.push({ kind: "text", text: plain });
        plain = "";
        out.push(seg);
        i += token.length;
        continue;
      }
    }
    plain += text[i];
    i += 1;
  }
  if (plain) out.push({ kind: "text", text: plain });
  return out;
}
