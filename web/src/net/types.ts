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

export type AlertKind = 'newHost' | 'hostGone' | 'ipConflict' | 'vlanMismatch' | 'watchDown' | 'watchUp';

export interface Alert {
  id: string;
  at: number;
  kind: AlertKind;
  ip: string;
  networkId?: string;
  detail: string;
  seen: boolean;
}

export interface HistoryEntry {
  at: number;
  user: string;
  action: string;
  target: string;
  detail: string;
}
