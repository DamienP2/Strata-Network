import { locale, t } from '../i18n';
import { fmtBps } from './model';
import type { PtpPort, PtpPortState, StreamKind } from './types';

/* Types de GET /api/multicast/flows (docs/api-network.md § Multicast flows). */

export interface McastFormat {
  standard?: string;
  width?: number;
  height?: number;
  frameRate?: number;
  frameRateText?: string;
  interlaced?: boolean;
  sampling?: string;
  depth?: number;
  colorimetry?: string;
  tcs?: string;
  clockRate?: number;
  audioRate?: number;
  channels?: number;
  ptimeMs?: number;
  tsRefclk?: string;
  ptpGm?: string;
  ptpDomain?: number;
  mediaClock?: string;
  bandwidthBps?: number;
}

export interface Attach {
  switchId: string;
  switchName: string;
  n?: number;
  port: string;
  edge: boolean;
  via: 'inventory' | 'macTable' | 'lldp';
}

export interface Receiver {
  switchId: string;
  switchName: string;
  n?: number;
  port: string;
  vlan: number;
  label?: string;
  host?: string;
  hostIp?: string;
  router: boolean;
  peer?: string;
  agg?: string;
}

export interface PathNode {
  switchId: string;
  switchName: string;
  parent?: string;
  egress?: string;
  ingress?: string;
  igmpPorts: number;
  receivers: number;
}

/** Un récepteur NMOS abonné à l'émetteur du flux. */
export interface Subscriber {
  id: string;
  label: string;
  node?: string;
  at?: Attach;
  igmp: boolean;
}

export type Issue = 'unannounced' | 'noReceivers' | 'pairMissing' | 'conflict' | 'flooded' | 'gone';

export interface Flow {
  key: string;
  group: string;
  port: number;
  source?: string;
  kind: StreamKind;
  encoding?: string;
  name?: string;
  label: string;
  groupLabel: string;
  sapName?: string;
  announced: boolean;
  routed: boolean;
  flooded: boolean;
  floodVlan?: number;
  snoopOnly: boolean;
  guessed: boolean;
  ambiguous: boolean;
  live: boolean;
  state: 'ok' | 'warn' | 'unk' | 'off';
  issues: Issue[];
  bps: number;
  pps: number;
  estBps?: number;
  history: number[];
  firstSeen: number;
  lastSeen: number;
  lastAnnounced?: number;
  sapIntervalMs?: number;
  format?: McastFormat;
  vlan?: number;
  emitter?: { ip: string; name?: string; mac?: string; at?: Attach };
  sources?: string[];
  conflict: boolean;
  receivers: Receiver[];
  receiverCount: number;
  path: PathNode[];
  unreachable: string[];
  pair?: { key: string; group: string; port: number; source?: string; name?: string; at?: Attach };
  pairState?: 'ok' | 'missing';
  pairBy?: 'sdp' | 'name';
  leg?: 'a' | 'b';
  origins: ('nmos' | 'sap' | 'igmp')[];
  nmos?: { senderId: string; sender: string; nodeId?: string; node?: string; inventory: boolean; active?: boolean };
  idle?: boolean;
  subscribers?: Subscriber[];
}

export interface PtpClock {
  ip: string;
  domain: number;
  clock: string;
  port: number;
  gm: string;
  priority1: number;
  clockClass: number;
  clockAccuracy: number;
  accuracyNs?: number;
  variance: number;
  priority2: number;
  stepsRemoved: number;
  timeSource: string;
  utcOffset: number;
  firstSeen: number;
  lastSeen: number;
  count: number;
  rate: number;
  logAnnounce?: number;
  logSync?: number;
  logDelayReq?: number;
  name?: string;
  role: 'gm' | 'backup' | 'boundary';
  live: boolean;
  wrongDomain: boolean;
  at?: Attach;
}

/** Un port de switch. */
export interface PtpPlace {
  switchId: string;
  switchName: string;
  n?: number;
  port: string;
}

export interface PtpGrandmaster {
  identity: string;
  name?: string;
  ip?: string;
  domain: number;
  priority1?: number;
  priority2?: number;
  clockClass?: number;
  clockAccuracy?: number;
  accuracyNs?: number;
  timeSource?: string;
  live: boolean;
  via: 'ptp' | 'snmp';
  at?: PtpPlace;
  portState?: PtpPortState;
  /** Du switch de ce serveur jusqu'au grandmaster : le port esclave de chaque switch. */
  path: PtpPlace[];
}

/** L'horloge qui envoie les Announce entendus ici. */
export interface PtpTalker {
  clock: string;
  port: number;
  ip: string;
  name?: string;
  domain: number;
  stepsRemoved: number;
  role: 'gm' | 'boundary';
  live: boolean;
  logAnnounce?: number;
  logSync?: number;
  logDelayReq?: number;
  at?: PtpPlace;
  via?: 'snmp' | 'ip' | 'mac';
  state?: PtpPortState;
}

export interface PtpSwitchSync {
  switchId: string;
  switchName: string;
  clockType: string;
  slave?: { n?: number; port: string };
  offsetNs?: number;
  meanPathDelayNs?: number;
  stepsRemoved?: number;
  history: number[];
}

export interface PtpPortRow extends PtpPort {
  switchId: string;
  switchName: string;
  strata: boolean;
}

export interface PtpTreeNode {
  id: string;
  kind: 'gm' | 'switch' | 'device' | 'clock';
  name: string;
  parent?: string;
  link: 'ok' | 'idle' | 'bad' | 'unknown';
  port?: string;
  offsetNs?: number;
  activity?: 'active' | 'none' | 'mute';
  clockType?: string;
  clockClass?: number;
  timeSource?: string;
  domain?: number;
}

export interface PtpDevice {
  clock: string;
  ip: string;
  domain: number;
  firstSeen: number;
  lastSeen: number;
  count: number;
  name?: string;
  mac?: string;
  live: boolean;
  wrongDomain: boolean;
  at?: Attach;
}

export interface GmChange {
  at: number;
  domain: number;
  from?: string;
  to?: string;
  fromIp?: string;
  toIp?: string;
}

export interface McastRange {
  id: string;
  cidr: string;
  usage: string;
  vlan?: number;
}

export interface GroupUse {
  group: string;
  state: 'used' | 'reserved' | 'conflict';
  name?: string;
}

export interface RangeView extends McastRange {
  first: string;
  last: string;
  size: number;
  used: number;
  reserved: number;
  conflicts: number;
  free: number;
  nextFree?: string;
  groups: GroupUse[];
}

export interface Reservation {
  group: string;
  name: string;
}

export interface Flows {
  at: number;
  flows: Flow[];
  summary: {
    flows: number;
    live: number;
    byKind: Record<string, number>;
    estBps: number;
    bps: number;
    receivers: number;
    pairs: number;
    pairsIncomplete: number;
    unannounced: number;
    noReceivers: number;
    conflicts: number;
    flooded: number;
    switchesOnline: number;
    switchesRead: number;
  };
  switches: { id: string; name: string; ip?: string; online: boolean; read: boolean; at: number }[];
  links: { a: string; b: string; aPort: string; bPort: string }[];
  plan: { ranges: RangeView[]; outside: { used: number; reserved: number; conflicts: number; groups: GroupUse[] }; reservations: Reservation[] };
  ptp: {
    gm?: PtpClock;
    domain?: number;
    clocks: PtpClock[];
    devices: PtpDevice[];
    changes: GmChange[];
    grandmaster?: PtpGrandmaster;
    talker?: PtpTalker;
    switchSync: PtpSwitchSync[];
    ports: PtpPortRow[];
    tree: PtpTreeNode[];
  };
  flood: { iface: string; enabled: boolean; available: boolean; error?: string; at: number; groups: string[]; vlan?: number };
  check: { enabled: boolean; maxMbps: number };
}

/* ---------- Types de flux : couleur et libellé ---------- */

/** v (vidéo), a (audio), d (données), ts (MPEG-TS) ; '' pour le reste. */
export type Tone = 'v' | 'a' | 'd' | 'ts' | 'o';

export function toneOf(f: Pick<Flow, 'kind' | 'encoding' | 'format'>): Tone {
  const e = (f.encoding ?? '').toLowerCase();
  if (f.kind === 'mpegTs' || e === 'mp2t') return 'ts';
  if (f.kind === 'data' || e === 'smpte291') return 'd';
  if (f.kind === 'audio' || ['l16', 'l24', 'l32', 'am824'].includes(e)) return 'a';
  if (f.kind === 'video' || e === 'raw' || e === 'jxsv' || e === 'smpte2022-6') return 'v';
  return 'o';
}

export const TONES: Tone[] = ['v', 'a', 'd', 'ts'];

export function toneLabel(k: Tone): string {
  switch (k) {
    case 'v':
      return t('Video');
    case 'a':
      return t('Audio');
    case 'd':
      return t('Data');
    case 'ts':
      return t('MPEG-TS');
    default:
      return t('Other');
  }
}

/* ---------- Format ---------- */

const num = (v: number, d = 1) => v.toLocaleString(locale(), { maximumFractionDigits: d });

/** « 1080p50 · 4:2:2 10 bits », « L24 · 48 kHz · 8 voies », « ST 2110-40 », « MPEG-TS ». */
export function formatText(f: Flow): string {
  const m = f.format;
  const tone = toneOf(f);
  if (m?.height && m.frameRate) {
    const rate = m.interlaced ? m.frameRate * 2 : m.frameRate;
    const img = `${m.height}${m.interlaced ? 'i' : 'p'}${num(rate, 2)}`;
    const sampling = m.sampling?.replace(/^YCbCr-/, '');
    return [img, [sampling, m.depth ? t('{n} bits', { n: m.depth }) : ''].filter(Boolean).join(' ')].filter(Boolean).join(' · ');
  }
  if (tone === 'a' && (m?.audioRate || m?.channels)) {
    return [f.encoding, m.audioRate ? t('{n} kHz', { n: num(m.audioRate / 1000) }) : '', m.channels ? t('{n} ch', { n: m.channels }) : ''].filter(Boolean).join(' · ');
  }
  if (tone === 'ts' && m?.bandwidthBps) return `${m.standard ?? 'MPEG-TS'} · ${fmtBps(m.bandwidthBps)}`;
  if (m?.standard) return m.standard;
  return f.encoding ?? '';
}

/** « 1920×1080 · 50p ». */
export function imageText(m: McastFormat | undefined): string {
  if (!m?.width || !m.height) return '';
  const rate = m.frameRate ? `${num(m.interlaced ? m.frameRate * 2 : m.frameRate, 2)}${m.interlaced ? 'i' : 'p'}` : '';
  return [`${m.width}×${m.height}`, rate].filter(Boolean).join(' · ');
}

/** Nom affiché : libellé, nom de groupe, nom SAP, sinon le groupe. */
export const flowName = (f: Flow) => f.name || f.group;

/** Débit à montrer : mesuré si vivant, sinon estimé par le SDP. */
export const flowRate = (f: Flow) => (f.bps > 0 ? f.bps : (f.estBps ?? 0));

/** Le port en court : « Ethernet1/9 » → « Eth1/9 », « port-channel1 » → « Po1 ». */
export function shortPort(p: string): string {
  return p
    .replace(/^port-channel/i, 'Po')
    .replace(/^Ethernet/i, 'Eth')
    .replace(/^GigabitEthernet/i, 'Gi')
    .replace(/^TenGigabitEthernet/i, 'Te')
    .replace(/^TwentyFiveGigE/i, 'Twe')
    .replace(/^FortyGigabitEthernet/i, 'Fo')
    .replace(/^HundredGigE/i, 'Hu');
}

/** « SW-Plateau-A Eth1/1 ». */
export const attachText = (a: Attach | undefined) => (a ? `${a.switchName} ${shortPort(a.port)}` : '');
