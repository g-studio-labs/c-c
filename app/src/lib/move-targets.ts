import type { FolderTree, SpaceTree } from "@/types/task";

/**
 * Las listas a las que se puede mover una tarea, agrupadas por espacio para el
 * desplegable del detalle (command-and-control-328).
 *
 * Lo que se ofrece es lo que el servidor acepta, para que el menú no proponga
 * algo que luego conteste con un 409:
 *
 * - **misma organización**: cruzarla enseñaría la tarjeta a quien no ve de
 *   dónde viene (`ErrChannelOtherOrg`);
 * - **una tarjeta de cliente sólo entre listas de su canal**: gastó un folio
 *   de ese cliente y sale en su tablero por su proyecto, no por su lista
 *   (`ErrMoveLeavesChannel`). Una interna puede ir a cualquiera.
 *
 * `crossesSpace` marca las de otro espacio: allí una interna toma el número
 * siguiente, así que su ref cambia, y quien elige tiene que saberlo antes.
 */
export interface MoveTarget {
  id: string;
  /** «Carpeta · Lista», sin el espacio: el espacio es la cabecera del grupo. */
  label: string;
  crossesSpace: boolean;
}

export interface MoveTargetGroup {
  space: string;
  targets: MoveTarget[];
}

interface Movable {
  orgId: string;
  listId: string;
  /** El canal del cliente, si la tarjeta gastó uno de sus folios. */
  projectId?: string;
}

interface Flat {
  id: string;
  label: string;
  projectId?: string;
}

// Las carpetas anidan (FolderTree.folders): un recorrido de un solo nivel se
// dejaba fuera las listas de una subcarpeta.
function walk(folder: FolderTree, prefix: string, out: Flat[]) {
  const here = prefix ? `${prefix} · ${folder.name}` : folder.name;
  for (const l of folder.lists ?? []) out.push({ id: l.id, label: `${here} · ${l.name}`, projectId: l.projectId });
  for (const f of folder.folders ?? []) walk(f, here, out);
}

function listsOf(space: SpaceTree): Flat[] {
  const out: Flat[] = (space.lists ?? []).map((l) => ({ id: l.id, label: l.name, projectId: l.projectId }));
  for (const f of space.folders ?? []) walk(f, "", out);
  return out;
}

// `tree` puede faltar: el detalle se abre también sin el navegador cargado
// (desde una notificación, o en una prueba que no lo monta), y entonces no hay
// a dónde ofrecer, no un error.
export function moveTargets(tree: SpaceTree[] | undefined, task: Movable): MoveTargetGroup[] {
  if (!tree) return [];
  const home = tree.find((sp) => listsOf(sp).some((l) => l.id === task.listId));
  const groups: MoveTargetGroup[] = [];
  for (const sp of tree) {
    if (sp.orgId !== task.orgId) continue;
    const targets = listsOf(sp)
      .filter((l) => !task.projectId || l.projectId === task.projectId)
      .map((l) => ({
        id: l.id,
        label: l.label,
        // Una de cliente conserva su folio aunque cambie de espacio: el número
        // es del proyecto, no del espacio.
        crossesSpace: !task.projectId && !!home && sp.id !== home.id,
      }));
    if (targets.length > 0) groups.push({ space: sp.name, targets });
  }
  return groups;
}
