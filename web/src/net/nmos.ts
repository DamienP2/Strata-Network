import { t } from '../i18n';
import type { Attach } from './mcast';

/* Types de /api/nmos (docs/api-network.md § NMOS). */

export interface NmosSettings {
  enabled: boolean;
  url: string;
  version: 'v1.2' | 'v1.3';
  refresh: 'live' | '60s';
}

export interface NmosStatus {
  enabled: boolean;
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
}

export interface NmosNode {
  id: string;
  label: string;
  hostname?: string;
  maker?: string;
  ip?: string;
  ips: string[];
  name?: string;
  senders: number;
  receivers: number;
  at: Attach[];
  inventory: boolean;
  seen: number;
  present: boolean;
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
