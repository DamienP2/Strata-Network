/** Adresses IPv4 : texte ↔ nombre, plages, sous-réseaux. */

export function ipToInt(s: string | undefined | null): number | null {
  if (!s) return null;
  const p = s.trim().split('.');
  if (p.length !== 4) return null;
  let n = 0;
  for (const x of p) {
    if (!/^\d{1,3}$/.test(x)) return null;
    const v = Number(x);
    if (v > 255) return null;
    n = n * 256 + v;
  }
  return n;
}

export const intToIp = (n: number) => `${(n >>> 24) & 255}.${(n >>> 16) & 255}.${(n >>> 8) & 255}.${n & 255}`;

export interface Range {
  /** Premières et dernières adresses listées (sans réseau ni diffusion pour un préfixe ≤ 30). */
  first: number;
  last: number;
  /** Le bloc entier (réseau et diffusion compris) pour un préfixe. */
  lo: number;
  hi: number;
}

const MAX = 65536;

/** « 10.10.0.0/24 », « 10.10.0.1-10.10.0.80 » ou une seule adresse. */
export function parseRange(range: string): Range | null {
  const r = range.trim();
  const slash = r.indexOf('/');
  if (slash >= 0) {
    const base = ipToInt(r.slice(0, slash));
    const prefix = Number(r.slice(slash + 1));
    if (base === null || !Number.isInteger(prefix) || prefix < 16 || prefix > 32) return null;
    const size = 2 ** (32 - prefix);
    const lo = Math.floor(base / size) * size;
    const hi = lo + size - 1;
    return prefix <= 30 ? { first: lo + 1, last: hi - 1, lo, hi } : { first: lo, last: hi, lo, hi };
  }
  const dash = r.indexOf('-');
  if (dash >= 0) {
    const a = ipToInt(r.slice(0, dash));
    const b = ipToInt(r.slice(dash + 1));
    if (a === null || b === null || b < a || b - a >= MAX) return null;
    return { first: a, last: b, lo: a, hi: b };
  }
  const one = ipToInt(r);
  return one === null ? null : { first: one, last: one, lo: one, hi: one };
}

export const rangeSize = (r: Range) => r.last - r.first + 1;

/** Vrai si l'adresse est dans le sous-réseau « a.b.c.d/p ». */
export function inSubnet(ip: string | number, cidr: string | undefined): boolean {
  if (!cidr) return false;
  const n = typeof ip === 'number' ? ip : ipToInt(ip);
  const slash = cidr.indexOf('/');
  const base = ipToInt(slash >= 0 ? cidr.slice(0, slash) : cidr);
  const prefix = slash >= 0 ? Number(cidr.slice(slash + 1)) : 32;
  if (n === null || base === null || !(prefix >= 0 && prefix <= 32)) return false;
  const size = 2 ** (32 - prefix);
  return Math.floor(n / size) === Math.floor(base / size);
}

export const validIp = (s: string) => ipToInt(s) !== null;

export function validCidr(s: string): boolean {
  const slash = s.indexOf('/');
  if (slash < 0) return false;
  const p = Number(s.slice(slash + 1));
  return validIp(s.slice(0, slash)) && Number.isInteger(p) && p >= 0 && p <= 32 && s.slice(slash + 1).trim() !== '';
}

export const byIp = (a: string, b: string) => (ipToInt(a) ?? 0) - (ipToInt(b) ?? 0);
