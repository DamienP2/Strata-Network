import { locale, t, tc } from '../i18n';
import { fmtTime } from '../ui/format';
import type { PtpActivity, PtpPortState } from './types';

/* Ce que les vues PTP partagent : unités, états des ports, intervalles, activité. */

/** L'intervalle d'annonce du profil SMPTE ST 2059-2 (log2 s). */
export const SMPTE_LOG_ANNOUNCE = -2;

const num = (v: number, d = 0) => v.toLocaleString(locale(), { maximumFractionDigits: d });

/** Une durée PTP : « 100 ns », « 6,9 µs », « 1,2 ms » (vrai signe moins). */
export function fmtNs(ns: number): string {
  const a = Math.abs(ns);
  const sign = ns < 0 ? '−' : '';
  if (a >= 1e6) return t('{n} ms', { n: sign + num(a / 1e6, 1) });
  if (a >= 1e3) return t('{n} µs', { n: sign + num(a / 1e3, 1) });
  return t('{n} ns', { n: sign + num(a, 0) });
}

/** La correction : le plus grand écart au maître, « ±100 ns ». */
export function fmtCorr(values: number[]): string {
  return values.length ? `±${fmtNs(Math.max(...values.map(Math.abs)))}` : '—';
}

/** Un intervalle log2 : « −3 », « 0 ». */
export const logText = (v: number | undefined) => (v === undefined ? '—' : v < 0 ? `−${-v}` : String(v));

/** Des messages par seconde : « 8 », « 0,5 ». */
export const fmtRate = (v: number) => num(v, v < 1 ? 1 : 0);

export function stateLabel(s: PtpPortState): string {
  switch (s) {
    case 'master':
      return t('Master');
    case 'slave':
      return t('Follower');
    case 'passive':
      return t('Passive');
    case 'disabled':
      return tc('ptp', 'Disabled');
    case 'faulty':
      return t('Faulty');
    case 'listening':
      return t('Listening');
    case 'initializing':
      return t('Starting');
    case 'preMaster':
      return t('Pre-master');
    case 'uncalibrated':
      return t('Uncalibrated');
    default:
      return t('Unknown');
  }
}

/** L'état d'un port PTP en pastille. */
export function PortStateBadge({ state }: { state: PtpPortState }) {
  const cls = state === 'master' ? 'm' : state === 'slave' ? 's' : state === 'disabled' || state === 'faulty' || state === 'unknown' ? 'd' : 'p';
  return <span className={`ptpst ${cls}`}>{stateLabel(state)}</span>;
}

export function clockTypeLabel(c: string | undefined): string {
  switch (c) {
    case 'boundary':
    case 'boundaryNode':
      return t('Boundary clock');
    case 'ordinary':
      return t('Ordinary clock');
    case 'transparent':
      return t('Transparent clock');
    default:
      return t('Clock');
  }
}

/** La source de temps du grandmaster. */
export function timeSourceLabel(s: string | undefined): string {
  switch (s) {
    case 'gps':
      return 'GPS';
    case 'ptp':
      return 'PTP';
    case 'ntp':
      return 'NTP';
    case 'atomic':
      return t('Atomic clock');
    case 'radio':
      return t('Radio');
    case 'handSet':
      return t('Set by hand');
    case 'internalOscillator':
      return t('Internal oscillator');
    case 'other':
      return tc('ptp', 'Other');
    default:
      return '—';
  }
}

/** « Annonce · Sync · Delay_Req » : l'annonce en orange quand elle s'écarte du profil SMPTE ST 2059-2. */
export function Intervals({ ann, sync, dreq }: { ann?: number; sync?: number; dreq?: number }) {
  if (ann === undefined && sync === undefined && dreq === undefined) return <span className="ivl">—</span>;
  const off = ann !== undefined && ann !== SMPTE_LOG_ANNOUNCE;
  return (
    <>
      <span className={`ivl ${off ? 'w' : ''}`} title={off ? t('The SMPTE ST 2059-2 profile announces at −2') : undefined}>
        {logText(ann)}
      </span>{' '}
      <span className="ivl">{`· ${logText(sync)} · ${logText(dreq)}`}</span>
    </>
  );
}

/** L'activité de l'appareil derrière un port maître : ses Delay_Req. */
export function Activity({ activity, rate, since, words }: { activity?: PtpActivity; rate?: number; since?: number; words: 'rate' | 'device' }) {
  if (!activity) return <span className="muted2">—</span>;
  if (activity === 'active')
    return (
      <span className="alive">
        <i />
        {words === 'rate' && rate !== undefined ? t('Delay_Req {n}/s', { n: fmtRate(rate) }) : t('follower active')}
      </span>
    );
  if (activity === 'mute')
    return (
      <span className="alive w">
        <i />
        {since !== undefined ? t('silent since {time}', { time: fmtTime(since) }) : t('silent')}
      </span>
    );
  return (
    <span className="alive off">
      <i />
      {t('no request')}
    </span>
  );
}

/** La courbe des corrections (écarts au maître), centrée sur zéro. */
export function CorrSpark({ values, width = 60 }: { values: number[]; width?: number }) {
  if (values.length < 2) return null;
  const H = 14;
  const max = Math.max(1, ...values.map(Math.abs));
  const pts = values.map((v, i) => `${((i / (values.length - 1)) * width).toFixed(1)},${(H / 2 - (v / max) * (H / 2 - 1)).toFixed(1)}`).join(' ');
  return (
    <svg width={width} height={H} viewBox={`0 0 ${width} ${H}`}>
      <polyline fill="none" stroke="currentColor" strokeWidth="1.4" points={pts} />
    </svg>
  );
}
