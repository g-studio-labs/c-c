import type { DocOwnerKind } from "@/types/task";

/**
 * La consulta de la URL que dice lo que se ve en «Tareas», con la forma que
 * entienden `doc_link` y `app_link` en el core (`?task=`, `?doc=kind:id&tab=&page=`):
 * es lo que `toAppUrl` convierte en el `cac://` de «Abrir en la app»
 * (command-and-control-576). Una tarea abierta gana al documento: el cajón
 * está encima. Sin lo uno ni lo otro, vacía.
 */
export function viewSearch(view: {
  taskId: string | null;
  doc: { kind: DocOwnerKind; id: string } | null;
  tab: string;
  pageId: string | null;
}): string {
  const params = new URLSearchParams();
  if (view.taskId) {
    params.set("task", view.taskId);
  } else if (view.doc) {
    params.set("doc", `${view.doc.kind}:${view.doc.id}`);
    if (view.tab !== "overview") params.set("tab", view.tab);
    if (view.pageId) params.set("page", view.pageId);
  }
  return params.toString();
}
