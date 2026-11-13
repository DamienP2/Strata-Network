import type { LucideIcon } from 'lucide-react';
import { Eye, Radar, ShieldCheck } from 'lucide-react';
import { lang, t, tc } from '../i18n';
import type { Role } from '../store';

const cap = (s: string) => (lang() === 'en' ? s.charAt(0).toUpperCase() + s.slice(1) : s);

/** « Administrateur », « Opérateur », « Lecteur ». */
export function roleLabel(r: Role): string {
  switch (r) {
    case 'admin':
      return cap(tc('role', 'admin'));
    case 'operator':
      return cap(tc('role', 'operator'));
    case 'viewer':
      return cap(tc('role', 'viewer'));
  }
}

/** Le nom du rôle au pluriel (puces de filtre). */
export function rolePlural(r: Role): string {
  switch (r) {
    case 'admin':
      return t('Admins');
    case 'operator':
      return t('Operators');
    case 'viewer':
      return t('Viewers');
  }
}

/** Ce que fait le rôle. */
export function roleHint(r: Role): string {
  switch (r) {
    case 'admin':
      return t('Everything, including users and the system');
    case 'operator':
      return t('Scan the network and edit the inventory');
    case 'viewer':
      return t('Read only');
  }
}

export const ROLE_ICON: Record<Role, LucideIcon> = { admin: ShieldCheck, operator: Radar, viewer: Eye };
