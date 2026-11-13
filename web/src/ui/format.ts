import { locale, t } from '../i18n';
import { useApp } from '../store';

/** Une date du serveur : millisecondes, secondes ou texte ISO. */
export function toMs(v: number | string | null | undefined): number | null {
  if (v === null || v === undefined || v === '') return null;
  if (typeof v === 'number') return v < 1e11 ? v * 1000 : v;
  const n = Date.parse(v);
  return Number.isNaN(n) ? null : n;
}

const zone = () => useApp.getState().timezone || undefined;

function safe(opts: Intl.DateTimeFormatOptions): Intl.DateTimeFormatOptions {
  const tz = zone();
  if (!tz) return opts;
  try {
    new Intl.DateTimeFormat(undefined, { timeZone: tz });
    return { ...opts, timeZone: tz };
  } catch {
    return opts;
  }
}

export function fmtDate(v: number | string | null | undefined): string {
  const ms = toMs(v);
  return ms === null ? '—' : new Date(ms).toLocaleDateString(locale(), safe({ day: 'numeric', month: 'short', year: 'numeric' }));
}

export function fmtDateTime(v: number | string | null | undefined): string {
  const ms = toMs(v);
  return ms === null ? '—' : new Date(ms).toLocaleString(locale(), safe({ day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }));
}

export function fmtTime(v: number | string | null | undefined): string {
  const ms = toMs(v);
  return ms === null ? '—' : new Date(ms).toLocaleTimeString(locale(), safe({ hour: '2-digit', minute: '2-digit', second: '2-digit' }));
}

/** « il y a 5 min ». */
export function fmtAgo(v: number | string | null | undefined): string {
  const ms = toMs(v);
  if (ms === null) return '—';
  const s = Math.round((ms - Date.now() - useApp.getState().serverOffset) / 1000);
  const rtf = new Intl.RelativeTimeFormat(locale(), { numeric: 'always', style: 'short' });
  const a = Math.abs(s);
  if (a < 60) return rtf.format(Math.round(s), 'second');
  if (a < 3600) return rtf.format(Math.round(s / 60), 'minute');
  if (a < 86400) return rtf.format(Math.round(s / 3600), 'hour');
  return rtf.format(Math.round(s / 86400), 'day');
}

/** `2d 3h`, `3h 12m`, `5m 10s`. */
export function fmtDuration(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d) return t('{d}d {h}h', { d, h });
  if (h) return t('{h}h {m}m', { h, m });
  return t('{m}m {s}s', { m, s: s % 60 });
}

export function fmtBytes(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '—';
  const num = (v: number) => v.toLocaleString(locale(), { maximumFractionDigits: v < 10 ? 1 : 0 });
  if (n < 1024) return t('{n} B', { n: num(n) });
  if (n < 1024 ** 2) return t('{n} KB', { n: num(n / 1024) });
  if (n < 1024 ** 3) return t('{n} MB', { n: num(n / 1024 ** 2) });
  return t('{n} GB', { n: num(n / 1024 ** 3) });
}

export const fmtPct = (v: number) => t('{n}%', { n: Math.round(v).toLocaleString(locale()) });

/** Ton d'une charge : ok < 60 %, warn < 85 %, bad au-delà. */
export const tone = (pct: number | null | undefined) => (pct === null || pct === undefined ? 'tone-idle' : pct < 60 ? 'tone-ok' : pct < 85 ? 'tone-warn' : 'tone-bad');
