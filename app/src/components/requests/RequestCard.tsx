import { Check, Copy, ExternalLink } from "lucide-react";
import { useRef, useState, type ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import RequestText from "@/components/requests/RequestText";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { desde } from "@/lib/desde";
import { useT } from "@/lib/i18n";
import { openExternal } from "@/lib/platform";
import { buildAnswer, KIND_LABEL, type AgentRequest, type AnswerDraft } from "@/lib/requests";
import { cn } from "@/lib/utils";
import { useAgentRequestsStore } from "@/store/agent-requests.store";
import type { APIResponse } from "@/types/auth";

/**
 * Un pedido: qué es, de qué tarea, cuánto lleva, su ID para seguirlo en la
 * terminal, y los botones de su tipo. Sólo los tiene quien puede contestar
 * (`canAnswer`): en «Equipo» se lee el de otro sin poder contestarlo por él.
 */
export default function RequestCard({ request, highlighted }: { request: AgentRequest; highlighted?: boolean }) {
  const { t } = useT();
  const navigate = useNavigate();
  const r = request;

  return (
    <article
      id={`request-${r.code}`}
      data-request-card=""
      data-highlighted={highlighted || undefined}
      className={cn(
        "flex flex-col gap-3 rounded-lg border bg-card p-4 text-sm",
        highlighted && "ring-2 ring-primary",
      )}
    >
      <header className="flex flex-wrap items-center gap-2">
        <Badge variant="secondary">{t(KIND_LABEL[r.kind])}</Badge>
        <button
          type="button"
          className="truncate text-xs text-muted-foreground hover:text-foreground hover:underline"
          onClick={() => navigate(`/tasks?task=${r.task.id}`)}
          title={t("work:requests.openTask")}
        >
          {[r.task.ref, r.task.title].filter(Boolean).join(" · ")}
        </button>
        <span className="text-xs text-muted-foreground">· {desde(r.createdAt)}</span>
        <CopyId code={r.code} />
      </header>
      <p className="whitespace-pre-wrap"><RequestText text={r.summary} /></p>
      <p className="text-xs text-muted-foreground">
        {t("work:requests.askedBy", { name: r.requestedBy.name })}
        {r.via === "mcp" && ` ${t("work:requests.viaAgent")}`}
        {" · "}
        {t("work:requests.waitsOn", { name: r.requestedOf.name })}
      </p>
      <KindBody request={r} />
      {r.status === "open" ? (
        r.canAnswer ? (
          <AnswerControls request={r} />
        ) : (
          <p className="text-xs text-muted-foreground">{t("work:requests.notYours", { name: r.requestedOf.name })}</p>
        )
      ) : (
        <Outcome request={r} />
      )}
    </article>
  );
}

function CopyId({ code }: { code: string }) {
  const { t } = useT();
  const [copied, setCopied] = useState(false);
  return (
    <Button
      variant="outline"
      size="xs"
      className="ml-auto font-mono"
      aria-label={t("work:requests.copyId", { code })}
      title={t("work:requests.copyId", { code })}
      onClick={() => {
        void copyText(code).then(() => {
          setCopied(true);
          toast.success(t("work:requests.copied", { code }));
          setTimeout(() => setCopied(false), 1500);
        });
      }}
    >
      {copied ? <Check /> : <Copy />}
      {code}
    </Button>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      {children}
    </div>
  );
}

/** Lo que el agente dejó, según el tipo. */
function KindBody({ request: r }: { request: AgentRequest }) {
  const { t } = useT();
  const navigate = useNavigate();
  const b = r.body;
  // Una ruta de la app se abre dentro; una dirección de fuera, en el navegador.
  const openLink = (url: string) => {
    if (url.startsWith("/") && !url.startsWith("//")) navigate(url);
    else void openExternal(url);
  };
  switch (r.kind) {
    case "approve_message":
      return (
        <div className="flex flex-col gap-2">
          <Field label={t("work:requests.destination")}>
            {b.destinationUrl ? (
              <button type="button" className="text-left text-primary hover:underline" onClick={() => void openExternal(b.destinationUrl!)}>
                {b.destination}
              </button>
            ) : (
              <span>{b.destination}</span>
            )}
          </Field>
          <Field label={t("work:requests.draft")}>
            <pre className="whitespace-pre-wrap rounded-md bg-muted p-3 font-sans"><RequestText text={b.draft ?? ""} /></pre>
          </Field>
        </div>
      );
    case "test":
      return (
        <div className="flex flex-col gap-2">
          {b.url && (
            <Button variant="outline" size="sm" className="self-start" onClick={() => openLink(b.url!)}>
              <ExternalLink />
              {t("work:requests.openScreen")}
            </Button>
          )}
          <Field label={t("work:requests.steps")}>
            <ol className="list-decimal pl-5">
              {(b.steps ?? []).map((s, i) => (
                <li key={i}><RequestText text={s} /></li>
              ))}
            </ol>
          </Field>
          <Field label={t("work:requests.expected")}>
            <span><RequestText text={b.expected ?? ""} /></span>
          </Field>
        </div>
      );
    case "decide":
      return b.question ? <p className="font-medium"><RequestText text={b.question} /></p> : null;
    case "run":
      return (
        <div className="flex flex-col gap-2">
          <Field label={t("work:requests.where")}>
            <span><RequestText text={b.where ?? ""} /></span>
          </Field>
          <Field label={t("work:requests.command")}>
            <div className="flex items-start gap-2">
              <pre className="min-w-0 flex-1 overflow-x-auto rounded-md bg-muted p-3"><RequestText text={b.command ?? ""} /></pre>
              <Button
                variant="outline"
                size="icon-sm"
                aria-label={t("work:requests.copyCommand")}
                title={t("work:requests.copyCommand")}
                onClick={() => void copyText(b.command ?? "").then(() => toast.success(t("work:requests.commandCopied")))}
              >
                <Copy />
              </Button>
            </div>
          </Field>
          <Field label={t("work:requests.expected")}>
            <span><RequestText text={b.expected ?? ""} /></span>
          </Field>
          {b.undo && (
            <Field label={t("work:requests.undo")}>
              <span><RequestText text={b.undo ?? ""} /></span>
            </Field>
          )}
        </div>
      );
    case "obtain":
      return (
        <div className="flex flex-col gap-2">
          <Field label={t("work:requests.what")}>
            <span><RequestText text={b.what ?? ""} /></span>
          </Field>
          <Field label={t("work:requests.from")}>
            <span><RequestText text={b.from ?? ""} /></span>
          </Field>
        </div>
      );
  }
}

/** Los botones del tipo, y el envío. */
function AnswerControls({ request: r }: { request: AgentRequest }) {
  const { t } = useT();
  const answer = useAgentRequestsStore((s) => s.answer);
  const [busy, setBusy] = useState(false);
  const [mode, setMode] = useState<"idle" | "edit" | "reject" | "fails">("idle");
  const [text, setText] = useState("");
  const [note, setNote] = useState("");
  const [output, setOutput] = useState("");
  const [shot, setShot] = useState<{ url: string; name: string } | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  const send = async (draft: AnswerDraft) => {
    const body = buildAnswer(draft);
    if (!body) return;
    setBusy(true);
    try {
      await answer(r.code, body);
    } catch {
      toast.error(t("work:requests.errAnswer"));
    } finally {
      setBusy(false);
    }
  };

  const upload = async (file: File) => {
    const form = new FormData();
    form.append("file", file);
    try {
      const res = await api.postForm<APIResponse<{ url: string; fileName: string }>>(`/api/v1/tasks/${r.task.id}/attachments`, form);
      if (res.data) setShot({ url: res.data.url, name: res.data.fileName });
    } catch {
      toast.error(t("work:requests.errScreenshot"));
    }
  };

  const noteBox = (label: string, required = false) => (
    <Textarea
      aria-label={label}
      placeholder={label}
      value={note}
      required={required}
      onChange={(e) => setNote(e.target.value)}
      rows={2}
    />
  );

  switch (r.kind) {
    case "approve_message":
      if (mode === "edit")
        return (
          <div className="flex flex-col gap-2">
            <Textarea aria-label={t("work:requests.finalText")} value={text} onChange={(e) => setText(e.target.value)} rows={6} />
            <div className="flex gap-2">
              <Button disabled={busy || !text.trim()} onClick={() => void send({ kind: "approve", edited: text })}>
                {t("work:requests.approveThis")}
              </Button>
              <Button variant="ghost" onClick={() => setMode("idle")}>{t("work:requests.cancel")}</Button>
            </div>
          </div>
        );
      if (mode === "reject")
        return (
          <div className="flex flex-col gap-2">
            {noteBox(t("work:requests.rejectReason"), true)}
            <div className="flex gap-2">
              <Button variant="destructive" disabled={busy || !note.trim()} onClick={() => void send({ kind: "reject", reason: note })}>
                {t("work:requests.sendReject")}
              </Button>
              <Button variant="ghost" onClick={() => setMode("idle")}>{t("work:requests.cancel")}</Button>
            </div>
          </div>
        );
      return (
        <div className="flex flex-wrap gap-2">
          <Button disabled={busy} onClick={() => void send({ kind: "approve" })}>{t("work:requests.approve")}</Button>
          <Button variant="outline" disabled={busy} onClick={() => { setText(r.body.draft ?? ""); setMode("edit"); }}>
            {t("work:requests.editApprove")}
          </Button>
          <Button variant="destructive" disabled={busy} onClick={() => setMode("reject")}>{t("work:requests.reject")}</Button>
        </div>
      );

    case "test":
      return (
        <div className="flex flex-col gap-2">
          {noteBox(mode === "fails" ? t("work:requests.noteRequired") : t("work:requests.noteOptional"), mode === "fails")}
          <div className="flex flex-wrap items-center gap-2">
            <input
              ref={fileRef}
              type="file"
              accept="image/*"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) void upload(f);
              }}
            />
            <Button variant="ghost" size="sm" onClick={() => fileRef.current?.click()}>
              {shot ? t("work:requests.screenshotAttached", { name: shot.name }) : t("work:requests.screenshot")}
            </Button>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button disabled={busy} onClick={() => void send({ kind: "works", note, screenshotUrl: shot?.url })}>
              {t("work:requests.works")}
            </Button>
            <Button
              variant="destructive"
              disabled={busy || (mode === "fails" && !note.trim())}
              onClick={() => {
                // «No funciona» pide decir qué se vio: el primer clic abre la
                // nota obligatoria, el segundo manda.
                if (!note.trim()) {
                  setMode("fails");
                  return;
                }
                void send({ kind: "fails", note, screenshotUrl: shot?.url });
              }}
            >
              {t("work:requests.fails")}
            </Button>
          </div>
        </div>
      );

    case "decide":
      return (
        <div className="flex flex-col gap-2">
          <div className="flex flex-col gap-2">
            {(r.body.options ?? []).map((o) => (
              <Button
                key={o.label}
                variant={o.recommended ? "default" : "outline"}
                className="h-auto flex-col items-start gap-0.5 whitespace-normal py-2 text-left"
                disabled={busy}
                onClick={() => void send({ kind: "choose", option: o.label, note })}
              >
                <span className="flex items-center gap-2">
                  {o.label}
                  {o.recommended && <Badge variant="outline">{t("work:requests.recommended")}</Badge>}
                </span>
                <span className="text-xs font-normal opacity-80">{o.consequence}</span>
              </Button>
            ))}
          </div>
          {noteBox(t("work:requests.noteOptional"))}
        </div>
      );

    case "run":
      return (
        <div className="flex flex-col gap-2">
          <Textarea
            aria-label={t("work:requests.output")}
            placeholder={t("work:requests.output")}
            className="font-mono"
            value={output}
            onChange={(e) => setOutput(e.target.value)}
            rows={3}
          />
          <Button className="self-start" disabled={busy} onClick={() => void send({ kind: "done", note: "", output })}>
            {t("work:requests.done")}
          </Button>
        </div>
      );

    case "obtain":
      return (
        <div className="flex flex-col gap-2">
          {noteBox(t("work:requests.noteOptional"))}
          <Button className="self-start" disabled={busy} onClick={() => void send({ kind: "done", note })}>
            {t("work:requests.ready")}
          </Button>
        </div>
      );
  }
}

/** Cómo quedó un pedido cerrado. */
function Outcome({ request: r }: { request: AgentRequest }) {
  const { t } = useT();
  const a = r.answer;
  let line: string;
  if (r.status === "withdrawn") line = t("work:requests.outcome.withdrawn");
  else if (r.kind === "approve_message")
    line = a?.approved
      ? t(a.edited ? "work:requests.outcome.approvedEdited" : "work:requests.outcome.approved")
      : t("work:requests.outcome.rejected", { reason: a?.reason ?? "" });
  else if (r.kind === "test")
    line = a?.works ? t("work:requests.outcome.works") : t("work:requests.outcome.fails", { note: a?.note ?? "" });
  else if (r.kind === "decide") line = t("work:requests.outcome.chose", { option: a?.option ?? "" });
  else line = t("work:requests.outcome.done");

  return (
    <div className="flex flex-col gap-1 rounded-md bg-muted p-3">
      <span className="font-medium">{line}</span>
      {a?.note && r.kind !== "test" && <span className="whitespace-pre-wrap">{a.note}</span>}
      {a?.output && <pre className="overflow-x-auto whitespace-pre-wrap text-xs">{a.output}</pre>}
      {a?.finalText && a.edited && <pre className="whitespace-pre-wrap font-sans text-xs">{a.finalText}</pre>}
      {r.answeredBy && (
        <span className="text-xs text-muted-foreground">
          {t("work:requests.outcome.by", { name: r.answeredBy.name, when: desde(r.answeredAt) })}
          {r.answeredVia === "session" && ` · ${t("work:requests.outcome.viaSession")}`}
        </span>
      )}
      {r.publishedUrl && (
        <button type="button" className="self-start text-xs text-primary hover:underline" onClick={() => void openExternal(r.publishedUrl!)}>
          {t("work:requests.outcome.published")}
        </button>
      )}
    </div>
  );
}
