import { t } from '../i18n';
import type { Attach } from './mcast';

/* Types de /api/nmos (docs/api-network.md § NMOS). */

export type NmosSource = 'registry' | 'nodes';
export type FoundBy = 'mdns' | 'scan' | 'manual' | 'registry';

export interface NmosSettings {
  enabled: boolean;
  source: NmosSource;
  url: string;
  version: 'v1.2' | 'v1.3';
  refresh: 'live' | '60s';
  nodeUrls: string[];
  probeScan: boolean;
  ranges: string[];
  nodePorts: number[];
}

export interface NmosStatus {
  enabled: boolean;
  source: NmosSource;
  url: string;
  version: string;
  refresh: 'live' | '60s';
  registry?: string;
  backup?: string;
  at: number;
  tried: number;
  error?: string;
  nodes: number;
  devices: number;
  senders: number;
  video: number;
  receivers: number;
  subscribed: number;
  unknown: number;
  withoutSdp: number;
  polled: number;
  answering: number;
  unreachable: number;
  mdns: number;
  scan: number;
  manual: number;
  apis: string[];
  ifaces: string[];
}

export interface NmosNode {
  id: string;
  label: string;
  hostname?: string;
  maker?: string;
  ip?: string;
  ips: string[];
  media: string[];
  name?: string;
  senders: number;
  receivers: number;
  at: Attach[];
  inventory: boolean;
  seen: number;
  present: boolean;
  found: FoundBy;
  foundBy: FoundBy[];
  url?: string;
  api?: string;
  lastOk?: number;
  error?: string;
}

/** Un test du mode nœuds : une API Node entendue par mDNS ou saisie. */
export interface NmosChecked {
  url: string;
  found: FoundBy[];
  ok: boolean;
  label?: string;
  api?: string;
  senders: number;
  receivers: number;
  error?: string;
}

export function foundLabel(f: FoundBy): string {
  return f === 'mdns' ? t('mDNS') : f === 'scan' ? t('Scan') : f === 'manual' ? t('Manual') : t('Registry');
}

export interface NmosFound {
  name: string;
  host: string;
  ip?: string;
  port: number;
  url: string;
  priority?: number;
  proto: string;
  versions: string[];
}

export type Origin = 'nmos' | 'sap' | 'igmp';
export const ORIGINS: Origin[] = ['nmos', 'sap', 'igmp'];

export function originLabel(o: Origin): string {
  return o === 'nmos' ? t('NMOS') : o === 'sap' ? t('SAP') : t('IGMP');
}
