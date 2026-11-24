import { describe, expect, it } from "vitest";

import { citedAttachmentIds, rewriteAttachmentUrls, sharedAttachmentId } from "@/lib/shared-doc";

const A = "11111111-2222-3333-4444-555555555555";
const B = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

describe("doc attachment citations", () => {
  // Las tres formas en que un cuerpo cita un adjunto: la relativa que guarda
  // el editor, con dominio, y con el pase de la web colgando.
  const body = [
    `![shot](/api/v1/docs/d1/attachments/${A}/raw)`,
    `[log](https://cac.example/api/v1/docs/d1/attachments/${B}/raw?token=abc)`,
    `again ![same](/api/v1/docs/d1/attachments/${A}/raw)`,
    `not a doc ![t](/api/v1/tasks/t1/attachments/${B.replace("a", "f")}/raw)`,
  ].join("\n\n");

  it("finds each cited attachment once", () => {
    expect(citedAttachmentIds(body).sort()).toEqual([A, B].sort());
  });

  // Una imagen va a la ruta compartida y un enlace a un fragmento: nada queda
  // apuntando a la ruta del documento ni con el pase de la web.
  it("sends every citation through the shared route", () => {
    const out = rewriteAttachmentUrls(body);
    expect(out).toContain(`![shot](/api/v1/shared-docs/attachments/${A})`);
    expect(out).toContain(`![same](/api/v1/shared-docs/attachments/${A})`);
    expect(out).toContain(`[log](#shared-attachment=${B})`);
    expect(out).not.toContain("/api/v1/docs/");
    expect(out).not.toContain("token=abc");
    expect(sharedAttachmentId(`/api/v1/shared-docs/attachments/${A}`)).toBe(A);
    expect(sharedAttachmentId(`#shared-attachment=${B}`)).toBe(B);
    expect(sharedAttachmentId("/tasks?page=x")).toBeNull();
  });
});
