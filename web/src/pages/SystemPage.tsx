import { lazy, Suspense, useEffect } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { t } from '../i18n';
import { needsLook, useApp } from '../store';
import { navigate } from '../router';
import { SidePage } from './SidePage';
import { visibleGroups } from './systemSections';

const Performance = lazy(() => import('../settings/Performance').then((m) => ({ default: m.Performance })));
const Health = lazy(() => import('../settings/Health').then((m) => ({ default: m.Health })));
const Users = lazy(() => import('./Users').then((m) => ({ default: m.Users })));
const Security = lazy(() => import('../settings/Security').then((m) => ({ default: m.Security })));
const General = lazy(() => import('../settings/Settings').then((m) => ({ default: m.General })));
const Updates = lazy(() => import('../settings/Updates').then((m) => ({ default: m.Updates })));
const Maintenance = lazy(() => import('../settings/Settings').then((m) => ({ default: m.Maintenance })));

export function SystemPage({ section, sub }: { section?: string; sub?: string }) {
  const { user, health } = useApp(useShallow((s) => ({ user: s.auth?.user, health: s.health })));
  const groups = visibleGroups(user);
  const all = groups.flatMap((g) => g.sections);
  const current = all.find((s) => s.id === section);
  const first = all[0]?.id;

  // Sans section (ou section interdite) : la première visible.
  useEffect(() => {
    if (!current) navigate(first ? `/system/${first}` : '/', true);
  }, [current, first]);

  if (!current) return null;
  const look = needsLook(health);
  return (
    <SidePage
      className="sys-page"
      current={current.id}
      onPick={(id) => navigate(`/system/${id}`)}
      groups={groups.map((g) => ({
        title: g.title(),
        items: g.sections.map((s) => ({ id: s.id, icon: s.icon, label: s.label(), status: s.id === 'health' && look ? 'warn' : undefined })),
      }))}
    >
      <Suspense fallback={<div className="loading">{t('Loading…')}</div>}>
        {current.id === 'performance' && <Performance />}
        {current.id === 'health' && <Health sub={sub} />}
        {current.id === 'users' && <Users />}
        {current.id === 'security' && <Security />}
        {current.id === 'general' && <General />}
        {current.id === 'updates' && <Updates />}
        {current.id === 'maintenance' && <Maintenance />}
      </Suspense>
    </SidePage>
  );
}
