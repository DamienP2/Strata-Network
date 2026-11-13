import { useEffect, useState } from 'react';
import { Keyboard, Users } from 'lucide-react';
import { t } from '../i18n';
import { api, ROLES } from '../store';
import { Card } from '../settings/ui';
import { Keys, shortcutList } from '../ui/Shortcuts';
import { ROLE_ICON, roleHint, roleLabel } from '../ui/roles';
import { PageHead } from './SidePage';

/** La page d'aide de l'app (bouton ? de la barre du haut). */
export function Help() {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    api<{ version: string }>('GET', '/api/health').then(
      (h) => setVersion(h.version),
      () => {},
    );
  }, []);
  return (
    <main className="library">
      <div className="library-inner">
        <PageHead title={t('Help')} sub={version ? t('Strata Network {version}', { version }) : undefined} />
        <Card icon={<Keyboard size={16} />} title={t('Keyboard shortcuts')}>
          <div className="kbd-rows">
            {shortcutList().map((s) => (
              <div key={s.label} className="kbd-row">
                <span>{s.label}</span>
                <Keys keys={s.keys} />
              </div>
            ))}
          </div>
        </Card>
        <Card icon={<Users size={16} />} title={t('Roles')}>
          <div className="kbd-rows">
            {ROLES.map((r) => {
              const Icon = ROLE_ICON[r];
              return (
                <div key={r} className="kbd-row">
                  <span className={`badge role-${r}`}>
                    <Icon size={11} /> {roleLabel(r)}
                  </span>
                  <span className="muted">{roleHint(r)}</span>
                </div>
              );
            })}
          </div>
        </Card>
      </div>
    </main>
  );
}
