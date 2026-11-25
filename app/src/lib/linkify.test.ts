import { describe, expect, it } from "vitest";

import { linkify, requestSegments, type RequestSegment } from "@/lib/linkify";

/**
 * Los enlaces de un texto plano.
 * Mutantes: comerse el punto final dentro del enlace; cortar el `)` de un
 * enlace que lo abre; perder el texto de alrededor.
 */
describe("los enlaces de un texto", () => {
  it("separa el enlace del texto y deja fuera la puntuación de la frase", () => {
    expect(linkify("mira https://cac.guz-studio.dev/docs?x=1. y dime")).toEqual([
      { kind: "text", text: "mira " },
      { kind: "link", href: "https://cac.guz-studio.dev/docs?x=1" },
      { kind: "text", text: ". y dime" },
    ]);
  });

  it("un paréntesis es del enlace sólo si lo abre dentro", () => {
    expect(linkify("(ver https://es.wikipedia.org/wiki/Foo_(bar))")).toEqual([
      { kind: "text", text: "(ver " },
      { kind: "link", href: "https://es.wikipedia.org/wiki/Foo_(bar)" },
      { kind: "text", text: ")" },
    ]);
  });

  it("sin enlaces, el texto tal cual; y varios en la misma línea", () => {
    expect(linkify("hola")).toEqual([{ kind: "text", text: "hola" }]);
    expect(linkify("http://a.com y https://b.com").filter((p) => p.kind === "link")).toHaveLength(2);
  });
});

const text = (t: string): RequestSegment => ({ kind: "text", text: t });
const path = (p: string): RequestSegment => ({ kind: "path", shown: p, path: p });

/**
 * Los enlaces de un pedido: direcciones, rutas, `file://` y rutas de la app.
 * Mismos casos que `core/src/request_links.rs`.
 * Roturas que atrapa: la puntuación o el `)` de cierre dentro del enlace; una
 * ruta que se come lo que sigue al espacio o los backticks; `~/` o `C:\` sin
 * reconocer; `y/o` o `km/h` tomados por rutas; `file://` sin quitar o sin
 * decodificar; un enlace a la propia cac que abre el navegador; `/app/build/…`
 * tomado por una pantalla; perder texto al partir.
 */
describe("los enlaces de un pedido", () => {
  it("una dirección acaba donde acaba la frase", () => {
    expect(requestSegments("mira https://x.com/a.")).toEqual([text("mira "), { kind: "url", url: "https://x.com/a" }, text(".")]);
    expect(requestSegments("(https://x.com/a), luego")).toEqual([text("("), { kind: "url", url: "https://x.com/a" }, text("), luego")]);
    expect(requestSegments("https://es.wikipedia.org/wiki/Foo_(bar)")).toEqual([{ kind: "url", url: "https://es.wikipedia.org/wiki/Foo_(bar)" }]);
  });

  it("una ruta acaba en el espacio, la puntuación y los backticks", () => {
    expect(requestSegments("abre /home/rv/respaldos/ris.sql y sigue")).toEqual([text("abre "), path("/home/rv/respaldos/ris.sql"), text(" y sigue")]);
    expect(requestSegments("está en /tmp/a.log.")).toEqual([text("está en "), path("/tmp/a.log"), text(".")]);
    expect(requestSegments("(ver /tmp/a.log), ok")).toEqual([text("(ver "), path("/tmp/a.log"), text("), ok")]);
    expect(requestSegments("`/tmp/a.log`")).toEqual([text("`"), path("/tmp/a.log"), text("`")]);
    expect(requestSegments("`/tmp/a b.log`")).toEqual([text("`"), path("/tmp/a"), text(" b.log`")]);
  });

  it("~/ y C:\\ son rutas; una ~ suelta no", () => {
    expect(requestSegments("usa ~/notas/a.md")).toEqual([text("usa "), path("~/notas/a.md")]);
    expect(requestSegments("el ~ de casa")).toEqual([text("el ~ de casa")]);
    expect(requestSegments("abre C:\\Users\\ana\\a.txt ya")).toEqual([text("abre "), path("C:\\Users\\ana\\a.txt"), text(" ya")]);
  });

  it("file:// apunta a su ruta, ya sin esquema ni escapes", () => {
    expect(requestSegments("file:///home/rv/a%20b.txt")).toEqual([{ kind: "path", shown: "file:///home/rv/a%20b.txt", path: "/home/rv/a b.txt" }]);
    expect(requestSegments("file://otro/x")).toEqual([text("file://otro/x")]);
  });

  it("un enlace de la cac es una pantalla de la app", () => {
    const route = (shown: string, r: string): RequestSegment => ({ kind: "route", shown, route: r });
    expect(requestSegments("ve a /app/tasks?task=t1.")).toEqual([text("ve a "), route("/app/tasks?task=t1", "/tasks?task=t1"), text(".")]);
    expect(requestSegments("https://cac.guz-studio.dev/app/requests?open=req-1")).toEqual([
      route("https://cac.guz-studio.dev/app/requests?open=req-1", "/requests?open=req-1"),
    ]);
    expect(requestSegments("/app/build/out.log")).toEqual([path("/app/build/out.log")]);
    expect(requestSegments("https://otro.com/app/tasks?task=1")).toEqual([{ kind: "url", url: "https://otro.com/app/tasks?task=1" }]);
  });

  it("una barra dentro de una palabra no es una ruta", () => {
    for (const plain of ["y/o", "usa a/b/c.txt", "100 km/h", "1/2 de", "https://", "/", "// comentario", "tcg:/x"]) {
      expect(requestSegments(plain)).toEqual([text(plain)]);
    }
  });

  it("los trozos suman el original", () => {
    const original = "Ábrelo «aquí» https://x.com/ñ, luego ~/año/ü.txt; y /tmp/é (listo)";
    const joined = requestSegments(original)
      .map((s) => (s.kind === "text" ? s.text : s.kind === "url" ? s.url : s.shown))
      .join("");
    expect(joined).toBe(original);
  });
});
