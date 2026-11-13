import { useCallback, useEffect, useState } from 'react';
import type { LucideIcon } from 'lucide-react';
import {
  AudioLines,
  Box,
  Cctv,
  Clapperboard,
  Cpu,
  EthernetPort,
  HardDrive,
  Laptop,
  Monitor,
  Printer,
  Router,
  Server,
  Shield,
  Smartphone,
  Tv,
  Wifi,
} from 'lucide-react';
import { locale, t } from '../i18n';
import { api, useApp, useChanged } from '../store';
import { errorText } from '../ui/serverError';
import { fmtAgo, toMs } from '../ui/format';
import { inSubnet, intToIp, ipToInt } from './ip';
import type { Address, AlertKind, Kind, Network, Stream, Switch, SwitchPort, Vlan } from './types';

/** Toast d'erreur d'une exception (texte traduit, brut en infobulle). */
export function fail(e: unknown) {
  useApp.getState().toast(errorText(e), 'error', e instanceof Error ? e.message : String(e));
}

/** Une liste du serveur, rechargée quand le serveur signale un changement de `what`. */
export function useList<T>(what: string, path: string | null): [T | null, () => Promise<void>, (v: T | null) => void] {
  const [data, setData] = useState<T | null>(null);
  const load = useCallback(async () => {
    if (!path) return;
    try {
      setData(await api<T>('GET', path));
    } catch (e) {
      fail(e);
    }
  }, [path]);
  useEffect(() => {
    setData(null);
    void load();
  }, [load]);
  useChanged(what, load);
  return [data, load, setData];
}

/** Les adresses connues de tous les réseaux (pour les switchs, la topologie, le multicast). */
export function useAllAddresses(networks: Network[] | null): Map<string, Address> {
  const [all, setAll] = useState<Map<string, Address>>(new Map());
  const ids = networks?.map((n) => n.id).join(',') ?? '';
  const load = useCallback(async () => {
    if (!ids) return setAll(new Map());
    try {
      const lists = await Promise.all(ids.split(',').map((id) => api<Address[]>('GET', `/api/networks/${id}/addresses`)));
      const m = new Map<string, Address>();
      for (const l of lists) for (const a of l) m.set(a.ip, a);
      setAll(m);
    } catch (e) {
      fail(e);
    }
  }, [ids]);
  useEffect(() => void load(), [load]);
  useChanged('addresses', load);
  return all;
}

/* ---------- Types de machine ---------- */

export const KINDS: Kind[] = ['computer', 'server', 'laptop', 'phone', 'printer', 'camera', 'monitor', 'audio', 'video', 'switch', 'router', 'firewall', 'wifi', 'storage', 'iot', 'other'];

export const KIND_ICON: Record<Kind, LucideIcon> = {
  computer: Monitor,
  server: Server,
  laptop: Laptop,
  phone: Smartphone,
  printer: Printer,
  camera: Cctv,
  monitor: Tv,
  audio: AudioLines,
  video: Clapperboard,
  switch: EthernetPort,
  router: Router,
  firewall: Shield,
  wifi: Wifi,
  storage: HardDrive,
  iot: Cpu,
  other: Box,
};

export function kindLabel(k: Kind): string {
  switch (k) {
    case 'computer':
      return t('Computer');
    case 'server':
      return t('Server');
    case 'laptop':
      return t('Laptop');
    case 'phone':
      return t('Phone');
    case 'printer':
      return t('Printer');
    case 'camera':
      return t('Camera');
    case 'monitor':
      return t('Display');
    case 'audio':
      return t('Audio');
    case 'video':
      return t('Video');
    case 'switch':
      return t('Switch');
    case 'router':
      return t('Router');
    case 'firewall':
      return t('Firewall');
    case 'wifi':
      return t('Wi-Fi');
    case 'storage':
      return t('Storage');
    case 'iot':
      return t('Connected device');
    case 'other':
      return t('Other');
  }
}

/* ---------- États d'une adresse ---------- */

export type IpState = 'used' | 'offline' | 'reserved' | 'free';

export function ipState(a: Address | undefined): IpState {
  if (!a) return 'free';
  if (a.up) return 'used';
  if (a.lastSeen) return 'offline';
  if (a.reserved || a.label) return 'reserved';
  return 'free';
}

export const displayName = (a: Address | undefined, ip: string) => a?.label || a?.name || ip;

export const hasWeb = (a: Address | undefined) => !!a?.ports.some((p) => p.web);

/* ---------- Formats ---------- */

const num = (v: number, digits = 1) => v.toLocaleString(locale(), { maximumFractionDigits: digits });

export const fmtMs = (ms: number | undefined) => (ms === undefined || ms === null ? '—' : t('{n} ms', { n: num(ms, ms < 10 ? 1 : 0) }));

/** « à l’instant », « il y a 2 min », « jamais ». */
export function fmtSeen(v: number | undefined): string {
  const ms = toMs(v);
  if (ms === null) return t('never');
  if (Date.now() + useApp.getState().serverOffset - ms < 60_000) return t('just now');
  return fmtAgo(ms);
}

export function fmtBps(bps: number): string {
  if (bps >= 1e9) return t('{n} Gb/s', { n: num(bps / 1e9, 2) });
  if (bps >= 1e6) return t('{n} Mb/s', { n: num(bps / 1e6, 1) });
  if (bps >= 1e3) return t('{n} kb/s', { n: num(bps / 1e3, 0) });
  return t('{n} b/s', { n: num(bps, 0) });
}

export const fmtInt = (n: number) => Math.round(n).toLocaleString(locale());

/** Télécharge un texte comme fichier. */
export function downloadText(name: string, text: string, type: string) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** Copie un texte (presse-papiers, ou sélection pour une page en http). */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand('copy');
    } catch {
      ok = false;
    }
    ta.remove();
    return ok;
  }
}

/* ---------- VLAN et switchs ---------- */

export const vlanById = (vlans: Vlan[] | null, id: number | undefined) => (id === undefined ? undefined : vlans?.find((v) => v.id === id));

/** Le VLAN dont le sous-réseau contient l'adresse. */
export const vlanOfIp = (vlans: Vlan[] | null, ip: string | undefined) => (ip ? vlans?.find((v) => v.subnet && inSubnet(ip, v.subnet)) : undefined);

/** Le port d'un switch où la machine est branchée (sa fiche, ou le port qui la déclare). */
export function portOf(switches: Switch[] | null, a: Address | undefined, ip: string): { sw: Switch; port: SwitchPort } | null {
  if (!switches) return null;
  if (a?.switch) {
    const sw = switches.find((s) => s.id === a.switch!.id);
    const port = sw?.ports.find((p) => p.n === a.switch!.port);
    if (sw && port) return { sw, port };
  }
  for (const sw of switches) {
    const port = sw.ports.find((p) => p.host === ip);
    if (port) return { sw, port };
  }
  return null;
}

/** Le VLAN d'accès du port, quand l'adresse n'est pas dans son sous-réseau. */
export function vlanMismatch(vlans: Vlan[] | null, port: SwitchPort | undefined, ip: string): Vlan | null {
  if (!port || port.mode !== 'access' || port.vlan === undefined) return null;
  const v = vlanById(vlans, port.vlan);
  return v?.subnet && !inSubnet(ip, v.subnet) ? v : null;
}

export const portName = (sw: Switch | undefined, n: number) => sw?.ports.find((p) => p.n === n)?.name || String(n);

/* ---------- Multicast ---------- */

/** « ST 2110-20 », « AES67 », « MPEG-TS »… */
export function streamType(s: Stream): string {
  const e = (s.encoding ?? '').toLowerCase();
  if (s.kind === 'ptp') return t('PTP');
  if (s.kind === 'sap') return t('SAP');
  if (e === 'raw') return 'ST 2110-20';
  if (e === 'jxsv') return 'ST 2110-22';
  if (e === 'smpte291') return 'ST 2110-40';
  if (e === 'l24' || e === 'l16') return s.group.startsWith('239.69.') ? 'AES67' : 'ST 2110-30';
  if (e === 'mp2t' || s.kind === 'mpegTs') return 'MPEG-TS';
  switch (s.kind) {
    case 'video':
      return t('Video');
    case 'audio':
      return t('Audio');
    case 'data':
      return t('Data');
    case 'rtp':
      return t('RTP');
    default:
      return t('Other');
  }
}

/** Classe de couleur du type. */
export function streamTone(s: Stream): string {
  if (s.kind === 'video') return 'video';
  if (s.kind === 'audio') return 'audio';
  if (s.kind === 'ptp') return 'ptp';
  if (s.kind === 'mpegTs' || (s.encoding ?? '').toLowerCase() === 'mp2t') return 'ts';
  return '';
}

/** Vu il y a moins de 6 minutes (les tables des routeurs sont relues toutes les 5 minutes). */
export const streamLive = (s: Stream) => Date.now() + useApp.getState().serverOffset - (toMs(s.lastSeen) ?? 0) < 6 * 60_000;

export const streamKey = (s: Pick<Stream, 'group' | 'port' | 'source'>) => `${s.group}:${s.port}:${s.source ?? ''}`;

/* ---------- Alertes ---------- */

export const ALERT_KINDS: AlertKind[] = ['newHost', 'hostGone', 'ipConflict', 'vlanMismatch', 'watchDown', 'watchUp'];

export function alertLabel(k: AlertKind): string {
  switch (k) {
    case 'newHost':
      return t('New machine');
    case 'hostGone':
      return t('Machine gone');
    case 'ipConflict':
      return t('IP conflict');
    case 'vlanMismatch':
      return t('VLAN mismatch');
    case 'watchDown':
      return t('Watched, unreachable');
    case 'watchUp':
      return t('Watched, back');
  }
}

export const ALERT_TONE: Record<AlertKind, string> = { newHost: 'video', hostGone: '', ipConflict: 'bad', vlanMismatch: 'ptp', watchDown: 'bad', watchUp: 'audio' };

/** Les faits du serveur (anglais) : seuls les mots de liaison sont traduits. */
export const factText = (s: string) => s.replace(/ and /g, ` ${t('and')} `);

/** Le nombre d'alertes pas encore vues (pastille de l'onglet). */
export function useUnseenAlerts(enabled: boolean): number {
  const [n, setN] = useState(0);
  const load = useCallback(async () => {
    if (!enabled) return;
    try {
      const list = await api<{ seen: boolean }[]>('GET', '/api/alerts');
      setN(list.filter((a) => !a.seen).length);
    } catch {
      /* la pastille garde sa valeur */
    }
  }, [enabled]);
  useEffect(() => void load(), [load]);
  useChanged('alerts', load);
  return n;
}

export { intToIp, ipToInt };
