import { useEffect, useState } from 'react';
import { Bug } from 'lucide-react';
import { t } from '../i18n';
import { api, useApp } from '../store';
import { navigate } from '../router';
import { fmtDateTime } from './format';

/** Relit la santé chaque minute (admins) ; le bandeau après un plantage. */
export function CrashBanner() {
  const health = useApp((s) => s.health);
  const loadHealth = useApp((s) => s.loadHealth);
  const [hidden, setHidden] = useState<string | number | null>(null);

  useEffect(() => {
    void loadHealth();
    const id = setInterval(() => void loadHealth(), 60_000);
    return () => clearInterval(id);
  }, [loadHealth]);

  if (!health?.unseenCrash || !health.lastCrash || hidden === health.lastCrash) return null;
  return (
    <div className="crash-banner" role="alert">
      <Bug size={16} />
      <span>{t('Strata stopped unexpectedly on {date}, then restarted.', { date: fmtDateTime(health.lastCrash) })}</span>
      <button
        className="btn small"
        onClick={async () => {
          navigate('/system/health');
          try {
            await api('POST', '/api/system/health/seen');
          } catch {
            /* */
          }
          void loadHealth();
        }}
      >
        {t('View')}
      </button>
      <button className="btn small ghost" onClick={() => setHidden(health.lastCrash)}>
        {t('OK')}
      </button>
    </div>
  );
}
