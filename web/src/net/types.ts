/** Les types de l'API réseau (docs/api-network.md). */

export type Kind =
  | 'computer'
  | 'server'
  | 'laptop'
  | 'phone'
  | 'printer'
  | 'camera'
  | 'monitor'
  | 'audio'
  | 'video'
  | 'switch'
  | 'router'
  | 'firewall'
  | 'wifi'
  | 'storage'
  | 'iot'
  | 'other';

export interface Network {
  id: string;
  name: string;
  range: string;
  gateway?: string;
  vlan?: number;
  ports?: string;
  scheduleMins: number;
  lastScan?: { at: number; secs: number; up: number; stopped: boolean };
}

export interface Port {
  port: number;
  service?: string;
  web?: { scheme: 'http' | 'https'; status: number; title?: string; server?: string };
  banner?: string;
}

export interface Address {
  ip: string;
  label: string;
  kind?: Kind;
  notes: string;
  reserved: boolean;
  watched: boolean;
  up: boolean;
  via?: 'icmp' | 'tcp' | 'arp' | 'snmp';
  rttMs?: number;
  mac?: string;
  vendor?: string;
  name?: string;
  nameSource?: 'dns' | 'mdns' | 'netbios';
  ports: Port[];
  firstSeen?: number;
  lastSeen?: number;
  isNew: boolean;
  parent?: string;
  switch?: { id: string; port: number };
}

export interface Vlan {
  id: number;
  name: string;
  color: string;
  subnet?: string;
  gateway?: string;
  notes: string;
}

export type PortMode = 'access' | 'trunk' | 'off';

export interface SwitchPort {
  n: number;
  name: string;
  label: string;
  mode: PortMode;
  vlan?: number;
  tagged: number[];
  host?: string;
  peer?: { id: string; port: number };
  learnt: string[];
  lldp?: { name: string; port: string };
  /** Alerte quand son lien tombe (switch en ligne). */
  watched: boolean;
}

export type PortKind = 'rj45' | 'sfp' | 'qsfp';

export interface PortGroup {
  kind: PortKind;
  count: number;
  /** Nom d'interface avec `{n}` pour le numéro : « Gi1/0/{n} ». */
  naming: string;
  /** Premier numéro ; absent = à la suite du groupe d'avant s'il a le même nommage, sinon 1. */
  start?: number;
}

/** Une façade de départ : fournie avec le serveur, ou enregistrée par un utilisateur. */
export interface SwitchModel {
  id: string;
  brand: string;
  model: string;
  groups: PortGroup[];
}

/** Ce que la détection SNMP a lu et la façade qu'elle propose. */
export interface Detected {
  system: { name: string; description: string; model: string; serial: string; firmware: string; location: string };
  interfaces: number;
  names: string[];
  suggestion: { name: string; model: string; groups: PortGroup[]; matched: string | null };
}

export interface Switch {
  id: string;
  name: string;
  model: string;
  ip?: string;
  group: string;
  /** La façade : groupes de ports, dans l'ordre. */
  groups: PortGroup[];
  snmp?: string;
  ports: SwitchPort[];
  online: boolean;
  onlineSecs: number;
  rules: AlertRules;
}

/** Une règle d'alerte : active ou non, et sa limite. */
export interface Rule {
  on: boolean;
  value: number;
}

export interface AlertRules {
  linkDown: Rule;
  flap: Rule;
  errors: Rule;
  opticRx: Rule;
  saturation: Rule;
  health: Rule;
  drift: Rule;
  neighbours: Rule;
  poe: Rule;
}

/** Une interface du switch : son port de façade quand elle en est un. */
export interface InfoPort {
  n?: number;
  port: string;
}

export interface Neighbour extends InfoPort {
  name: string;
  remotePort: string;
  ip?: string;
  platform: string;
  chassisId: string;
  protocol: 'lldp' | 'cdp';
}

export interface Optic extends InfoPort {
  kind: string;
  vendor: string;
  part: string;
  serial: string;
  rxDbm?: number;
  txDbm?: number;
  rxLanesDbm: number[];
  tempC?: number;
  biasMa?: number;
  voltage?: number;
  rxLowWarnDbm?: number;
  rxLowAlarmDbm?: number;
}

export interface SnoopGroup {
  vlan: number;
  group?: string;
  mac: string;
  name?: string;
  ports: InfoPort[];
}

export interface HealthUnit {
  name: string;
  ok: boolean;
  detail: string;
  watts?: number;
}

export type StpState = 'disabled' | 'blocking' | 'listening' | 'learning' | 'forwarding' | 'broken';
export type PoeStatus = 'disabled' | 'searching' | 'delivering' | 'fault' | 'test' | 'otherFault' | 'unknown';

export interface PoePort extends InfoPort {
  adminOn: boolean;
  on: boolean;
  status: PoeStatus;
  class?: number;
  watts?: number;
}

/** Les lectures lentes d'un switch en ligne (toutes les minutes). */
export interface SwitchInfo {
  at: number;
  error?: string;
  errors: string[];
  neighbours: Neighbour[];
  optics: Optic[];
  mcast: SnoopGroup[];
  routerPorts: { vlan: number; ports: InfoPort[] }[];
  health: {
    cpuPct?: number;
    cpu5minPct?: number;
    memUsedBytes?: number;
    memTotalBytes?: number;
    temps: { name: string; c: number }[];
    fans: HealthUnit[];
    psus: HealthUnit[];
    /** [ms, %], 24 h. */
    cpu: [number, number][];
  };
  lags: { n: number; port: string; agg: string }[];
  stp: { n: number; state: StpState }[];
  poe?: { budgetW?: number; usedW?: number; ports: PoePort[] };
  /** Voisins qui sont un autre switch de l'inventaire, pas encore reliés. */
  links: { n: number; switchId: string; port: number }[];
}

/** L'historique des débits d'un port. */
export interface PortHistory {
  from: number;
  to: number;
  stepSecs: number;
  points: { at: number; in: number; out: number; inPeak?: number; outPeak?: number }[];
  in: { avg: number; peak: number };
  out: { avg: number; peak: number };
}

export interface LivePort {
  n: number;
  ifIndex: number;
  up: boolean;
  adminUp: boolean;
  speedMbps: number;
  inBps: number;
  outBps: number;
  inErrors: number;
  outErrors: number;
  alias: string;
  inHistory: number[];
  outHistory: number[];
}

export interface LiveDiff {
  n: number;
  field: 'mode' | 'vlan' | 'tagged' | 'off';
  switch: string;
  inventory: string;
}

/** Un switch en ligne : ce que le serveur vient d'y lire. */
export interface Live {
  at: number;
  error?: string;
  system?: { name: string; description: string; uptimeSecs: number; contact: string; location: string; model: string; serial: string; firmware: string };
  ports: LivePort[];
  vlans?: { n: number; pvid?: number; untagged: number[]; tagged: number[] }[];
  diffs: LiveDiff[];
}

export interface SnmpAgent {
  id: string;
  name: string;
  ip: string;
  port: number;
  community: string;
  /** SNMP v3 user; absent = v2c with the community. */
  v3?: SnmpV3 | null;
  readArp: boolean;
  readMulticast: boolean;
  readSwitch: boolean;
  lastPoll?: { at: number; ok: boolean; message: string };
}

export interface SnmpV3 {
  user: string;
  auth: 'none' | 'md5' | 'sha' | 'sha256';
  authPassword: string;
  privacy: 'none' | 'des' | 'aes128';
  privPassword: string;
}

export type StreamKind = 'video' | 'audio' | 'data' | 'mpegTs' | 'rtp' | 'ptp' | 'sap' | 'other';

export interface Stream {
  group: string;
  port: number;
  source?: string;
  kind: StreamKind;
  encoding?: string;
  name?: string;
  label: string;
  rtpType?: number;
  ptpDomain?: number;
  ptpGrandmaster?: string;
  bps: number;
  pps: number;
  packets: number;
  history: number[];
  firstSeen: number;
  lastSeen: number;
  announced: boolean;
  routed: boolean;
}

export interface MulticastState {
  iface: string;
  ifaces: { name: string; ip: string }[];
  streams: Stream[];
  checkMaxMbps: number;
}

export interface Service {
  kind: string;
  name: string;
  host: string;
  ip?: string;
  port: number;
  txt: string[];
}

export type AlertKind =
  | 'newHost'
  | 'hostGone'
  | 'ipConflict'
  | 'vlanMismatch'
  | 'watchDown'
  | 'watchUp'
  | 'portDown'
  | 'portFlap'
  | 'portErrors'
  | 'portSaturated'
  | 'configDrift'
  | 'opticLow'
  | 'psuFault'
  | 'fanFault'
  | 'tempHigh'
  | 'poeBudget'
  | 'neighbourNew'
  | 'neighbourGone'
  | 'streamGone'
  | 'noReceivers'
  | 'groupConflict'
  | 'flooding'
  | 'pairMissing'
  | 'ptpGmChange'
  | 'ptpDomain';

export interface Alert {
  id: string;
  at: number;
  kind: AlertKind;
  ip: string;
  networkId?: string;
  detail: string;
  seen: boolean;
  /** Alertes d'un switch : le switch, le port, la mesure et sa limite. */
  switchId?: string;
  port?: number;
  value?: number;
  limit?: number;
}

export interface HistoryEntry {
  at: number;
  user: string;
  action: string;
  target: string;
  detail: string;
}
