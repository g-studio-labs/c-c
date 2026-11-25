import { api } from "@/lib/api";
import type { MessageKey } from "@/lib/i18n";
import type { APIResponse } from "@/types/auth";

/**
 * Los pedidos que los agentes dejan para una persona («Por atender»,
 * command-and-control-500/504). El contrato es el del backend
 * (`domain/request.go`): aquí sólo se describe y se llama.
 */

export type RequestKind = "approve_message" | "test" | "decide" | "run" | "obtain";
export type RequestStatus = "open" | "answered" | "withdrawn";

export interface RequestOption {
  label: string;
  consequence: string;
  recommended?: boolean;
}

export interface RequestBody {
  draft?: string;
  destination?: string;
  destinationUrl?: string;
  url?: string;
  steps?: string[];
  expected?: string;
  question?: string;
  options?: RequestOption[];
  command?: string;
  where?: string;
  undo?: string;
  what?: string;
  from?: string;
}

export interface RequestAnswer {
  approved?: boolean;
  finalText?: string;
  edited?: boolean;
  reason?: string;
  works?: boolean;
  screenshotUrl?: string;
  option?: string;
  output?: string;
  note?: string;
}

export interface RequestPerson {
  id: string;
  name: string;
}

export interface AgentRequest {
  id: string;
  code: string;
  orgId: string;
  task: { id: string; ref?: string; title: string; priority: string; status: string; link: string };
  kind: RequestKind;
  summary: string;
  body: RequestBody;
  version: number;
  status: RequestStatus;
  requestedOf: RequestPerson;
  requestedBy: RequestPerson;
  via?: string;
  createdAt: string;
  updatedAt: string;
  answer?: RequestAnswer;
  answeredBy?: RequestPerson;
  answeredAt?: string;
  answeredVia?: "app" | "session";
  publishedUrl?: string;
  withdrawnAt?: string;
  canAnswer: boolean;
  link: string;
}

export type RequestScope = "mine" | "team";

export const KIND_LABEL: Record<RequestKind, MessageKey> = {
  approve_message: "work:requests.kind.approve_message",
  test: "work:requests.kind.test",
  decide: "work:requests.kind.decide",
  run: "work:requests.kind.run",
  obtain: "work:requests.kind.obtain",
};

export async function listRequests(scope: RequestScope, status: "open" | "closed"): Promise<AgentRequest[]> {
  const res = await api.get<APIResponse<AgentRequest[]>>(`/api/v1/requests?scope=${scope}&status=${status}`, true);
  return res.data ?? [];
}

/**
 * «Por atender» enseña los de la organización que se mira (command-and-control-527).
 * El servidor, sin `orgId`, devuelve los de todas las organizaciones de la
 * persona, y se reparte aquí: así un pedido de otra no se pierde, se avisa.
 */
export function inOrg(list: AgentRequest[], orgId: string | null): AgentRequest[] {
  return list.filter((r) => r.orgId === orgId);
}

/** Cuántos pedidos hay en cada otra organización, en el orden en que aparecen. */
export function inOtherOrgs(list: AgentRequest[], orgId: string | null): { orgId: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const r of list) if (r.orgId !== orgId) counts.set(r.orgId, (counts.get(r.orgId) ?? 0) + 1);
  return [...counts].map(([id, count]) => ({ orgId: id, count }));
}

export async function answerRequest(code: string, answer: RequestAnswer): Promise<AgentRequest> {
  const res = await api.post<APIResponse<AgentRequest>>(
    `/api/v1/requests/${encodeURIComponent(code)}/answer`,
    answer,
    true,
  );
  if (!res.data) throw new Error(res.error ?? "answer failed");
  return res.data;
}

/** La app dice «lo enseñé»: es lo que el MCP espera para decir `delivered`. */
export async function ackOpened(code: string, nonce: string): Promise<void> {
  await api.post(`/api/v1/requests/${encodeURIComponent(code)}/opened`, { nonce }, true);
}

/**
 * Lo que se manda al contestar, según el tipo y lo que se pulsó. Pura para
 * poder probar las reglas sin pintar nada: «no funciona» pide nota; rechazar,
 * motivo; editar, el texto final.
 */
export type AnswerDraft =
  | { kind: "approve"; edited?: string }
  | { kind: "reject"; reason: string }
  | { kind: "works"; note: string; screenshotUrl?: string }
  | { kind: "fails"; note: string; screenshotUrl?: string }
  | { kind: "choose"; option: string; note: string }
  | { kind: "done"; note: string; output?: string };

export function buildAnswer(d: AnswerDraft): RequestAnswer | null {
  const t = (s?: string) => (s ?? "").trim();
  switch (d.kind) {
    case "approve":
      return t(d.edited) ? { approved: true, finalText: t(d.edited) } : { approved: true };
    case "reject":
      return t(d.reason) ? { approved: false, reason: t(d.reason) } : null;
    case "works":
      return { works: true, ...(t(d.note) && { note: t(d.note) }), ...(d.screenshotUrl && { screenshotUrl: d.screenshotUrl }) };
    case "fails":
      return t(d.note) ? { works: false, note: t(d.note), ...(d.screenshotUrl && { screenshotUrl: d.screenshotUrl }) } : null;
    case "choose":
      return { option: d.option, ...(t(d.note) && { note: t(d.note) }) };
    case "done":
      return { ...(t(d.output) && { output: d.output!.replace(/\s+$/, "") }), ...(t(d.note) && { note: t(d.note) }) };
  }
}
