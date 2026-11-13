import type { LucideIcon } from 'lucide-react';
import { CloudDownload, Gauge, HeartPulse, History, Lock, Network, RotateCcw, SlidersHorizontal, Users } from 'lucide-react';
import { t } from '../i18n';
import { can, type PublicUser } from '../store';

export interface SystemSection {
  id: string;
  icon: LucideIcon;
  label: () => string;
  /** Réservé aux admins. */
  admin: boolean;
}
export interface SystemGroup {
  id: string;
  title: () => string;
  sections: SystemSection[];
}

/** Les groupes et sections de la page Système (`/system/<section>`). */
export const SYSTEM_GROUPS: SystemGroup[] = [
  {
    id: 'supervision',
    title: () => t('Monitoring'),
    sections: [
      { id: 'performance', icon: Gauge, label: () => t('Performance'), admin: true },
      { id: 'health', icon: HeartPulse, label: () => t('Health'), admin: true },
      { id: 'history', icon: History, label: () => t('History'), admin: true },
    ],
  },
  {
    id: 'access',
    title: () => t('Access'),
    sections: [
      { id: 'users', icon: Users, label: () => t('Users'), admin: true },
      { id: 'security', icon: Lock, label: () => t('Security'), admin: true },
    ],
  },
  {
    id: 'network',
    title: () => t('Network'),
    sections: [{ id: 'snmp', icon: Network, label: () => t('SNMP'), admin: true }],
  },
  {
    id: 'server',
    title: () => t('Server'),
    sections: [
      { id: 'general', icon: SlidersHorizontal, label: () => t('General'), admin: true },
      { id: 'updates', icon: CloudDownload, label: () => t('Updates'), admin: true },
      { id: 'maintenance', icon: RotateCcw, label: () => t('Maintenance'), admin: true },
    ],
  },
];

/** Les groupes, avec seulement les sections que cet utilisateur peut voir. */
export function visibleGroups(user: PublicUser | null | undefined): SystemGroup[] {
  return SYSTEM_GROUPS.map((g) => ({ ...g, sections: g.sections.filter((s) => !s.admin || can.admin(user)) })).filter((g) => g.sections.length > 0);
}

export const visibleSections = (user: PublicUser | null | undefined) => visibleGroups(user).flatMap((g) => g.sections);
