import type { PortGroup, PortKind } from './types';

/** Le plus de ports sur un switch. */
export const MAX_PORTS = 256;

/** Le premier numéro de chaque groupe : `start`, sinon à la suite du groupe d'avant s'il a le même nommage, sinon 1. */
export function groupStarts(groups: PortGroup[]): number[] {
  const out: number[] = [];
  groups.forEach((g, i) => {
    const prev = groups[i - 1];
    out.push(g.start ?? (prev && prev.naming.trim() === g.naming.trim() ? out[i - 1] + prev.count : 1));
  });
  return out;
}

/** Le nom d'un port : `{n}` remplacé par son numéro (ajouté à la fin s'il manque). */
export function portNameOf(naming: string, num: number): string {
  const n = naming.trim();
  return n.includes('{n}') ? n.split('{n}').join(String(num)) : `${n}${num}`;
}

/** Les ports de la façade, dans l'ordre : nom et type de cage. */
export function facePorts(groups: PortGroup[]): { name: string; kind: PortKind }[] {
  const starts = groupStarts(groups);
  return groups.flatMap((g, i) => Array.from({ length: g.count }, (_, k) => ({ name: portNameOf(g.naming, starts[i] + k), kind: g.kind })));
}

/** « Eth1/1 → 1/48 » : le second sans les lettres communes du début. */
export function rangeText(first: string, last: string): string {
  if (first === last) return first;
  let i = 0;
  while (i < first.length && i < last.length && first[i] === last[i] && !/\d/.test(first[i])) i++;
  return `${first} → ${last.slice(i)}`;
}

/** Les groupes de 12 ports d'un groupe (2 rangées, impairs en haut). */
export function blocksOf<T>(list: T[], size = 12): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < list.length; i += size) out.push(list.slice(i, i + size));
  return out;
}

export const kindShort = (k: PortKind) => (k === 'rj45' ? 'RJ45' : k === 'sfp' ? 'SFP' : 'QSFP');

/** « 48 SFP · 12 QSFP » : le nombre de ports par type, dans l'ordre de la façade. */
export function summary(groups: PortGroup[]): string {
  const per = new Map<PortKind, number>();
  for (const g of groups) if (g.count > 0) per.set(g.kind, (per.get(g.kind) ?? 0) + g.count);
  return [...per.entries()].map(([k, n]) => `${n} ${kindShort(k)}`).join(' · ');
}
