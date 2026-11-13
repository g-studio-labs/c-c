import type { TaskComment } from "@/types/task";

/**
 * El hilo de una tarea, partido en dos: lo que se dicen las personas (con las
 * líneas de sistema y las de GitHub, que cuentan lo que pasó) y lo que anotan
 * los agentes mientras trabajan (command-and-control-359).
 *
 * Un agente apunta mucho —un dato medido, un callejón sin salida, el cierre de
 * cada paso— y todo eso, mezclado, entierra la conversación. No se tira: se
 * pone aparte, con de quién es el agente que lo escribió.
 *
 * Lo que un agente escribió **para el cliente** (lo publicó a propósito) sigue
 * en la conversación: es parte de lo que el cliente lee, y esconderlo en la
 * otra pestaña dejaría el hilo distinto del suyo.
 */
export function isAgentNote(c: TaskComment): boolean {
  return c.via === "mcp" && c.kind !== "system" && c.visibility !== "public";
}

export function splitThread(comments: TaskComment[]): { talk: TaskComment[]; notes: TaskComment[] } {
  const talk: TaskComment[] = [];
  const notes: TaskComment[] = [];
  for (const c of comments) (isAgentNote(c) ? notes : talk).push(c);
  return { talk, notes };
}
