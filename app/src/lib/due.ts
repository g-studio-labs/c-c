import i18next from "@/lib/i18n";
import { isZone, myZone, zoneCity, zoneOffset } from "@/lib/timezones";
import { useOrgsStore } from "@/store/orgs.store";

/**
 * Cuándo es «hoy» para un vencimiento: en la zona de la organización, no en la
 * de quien mira (decidido con jose el 13-nov-2026).
 *
 * Un vencimiento es un día sin hora, y el día 15 es el 15 para todos. Lo que sí
 * depende de una zona es **cuándo se acaba**: medido con el reloj de cada uno,
 * la misma tarea estaba vencida para quien va una hora por delante y vigente
 * para los demás. Con la zona de la organización, vence para todos en el mismo
 * instante, y a quien está en otra zona se le dice cuál es.
 *
 * Sin zona puesta en la organización, sigue siendo el reloj de quien mira.
 */

/** La zona de la organización que se está viendo, si tiene una y existe. */
export function orgZone(): string | undefined {
  // Un vencimiento se pinta en media app: si la tienda aún no tiene
  // organizaciones, no hay zona, y no se cae nada.
  const s = useOrgsStore.getState?.();
  const zone = s?.orgs?.find((o) => o.id === s.currentOrgId)?.timezone;
  return zone && isZone(zone) ? zone : undefined;
}

/**
 * El día de hoy en esa zona, como fecha **local** a medianoche: lo mismo que
 * devuelve `dueDay`, para restarlos sin que la zona de la máquina entre.
 */
export function todayIn(zone: string | undefined, now: Date = new Date()): Date {
  if (zone) {
    try {
      const parts = new Intl.DateTimeFormat("en-CA", { timeZone: zone, year: "numeric", month: "2-digit", day: "2-digit" })
        .formatToParts(now);
      const n = (type: string) => Number(parts.find((p) => p.type === type)?.value);
      const [y, m, d] = [n("year"), n("month"), n("day")];
      if (y && m && d) return new Date(y, m - 1, d);
    } catch {
      // Una zona que la plataforma no conoce: el reloj de quien mira.
    }
  }
  return new Date(now.getFullYear(), now.getMonth(), now.getDate());
}

/**
 * «hora de Mexico City», sólo para quien no está en la hora de la
 * organización: a los demás no hay nada que aclararles. Se compara el desfase
 * de ahora y no el nombre: Mérida y Ciudad de México son la misma hora.
 */
export function zoneHint(zone: string | undefined = orgZone(), now: Date = new Date()): string {
  if (!zone || zoneOffset(zone, now) === zoneOffset(myZone(), now)) return "";
  return i18next.t("work:due.zone", { city: zoneCity(zone) });
}
