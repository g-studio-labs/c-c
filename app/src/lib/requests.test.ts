import { describe, expect, it } from "vitest";

import { buildAnswer } from "@/lib/requests";

/**
 * Lo que se manda al contestar, según el botón.
 *
 * Roturas que atrapa: mandar «no funciona» sin nota, o un rechazo sin motivo
 * (el servidor lo rechazaría y la persona no sabría por qué); aprobar sin
 * editar mandando un texto final vacío; perder lo pegado de una ejecución.
 */
describe("la respuesta de cada botón", () => {
  it("«no funciona» y rechazar no salen sin decir por qué", () => {
    expect(buildAnswer({ kind: "fails", note: "  " })).toBeNull();
    expect(buildAnswer({ kind: "reject", reason: "" })).toBeNull();
    expect(buildAnswer({ kind: "fails", note: " sale 07:00 " })).toEqual({ works: false, note: "sale 07:00" });
    expect(buildAnswer({ kind: "reject", reason: "tono" })).toEqual({ approved: false, reason: "tono" });
  });

  it("aprobar sin editar no manda texto; editado, manda el final", () => {
    expect(buildAnswer({ kind: "approve" })).toEqual({ approved: true });
    expect(buildAnswer({ kind: "approve", edited: " Otro texto " })).toEqual({ approved: true, finalText: "Otro texto" });
  });

  it("funciona, la opción y hecho llevan su nota y lo pegado", () => {
    expect(buildAnswer({ kind: "works", note: "" })).toEqual({ works: true });
    expect(buildAnswer({ kind: "works", note: "ok", screenshotUrl: "/x.png" })).toEqual({ works: true, note: "ok", screenshotUrl: "/x.png" });
    expect(buildAnswer({ kind: "choose", option: "B", note: "" })).toEqual({ option: "B" });
    expect(buildAnswer({ kind: "done", note: "", output: "  ok\n  linea 2\n\n" })).toEqual({ output: "  ok\n  linea 2" });
  });
});
