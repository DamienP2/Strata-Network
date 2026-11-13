import { useEffect, useRef, useState } from 'react';
import { TriangleAlert, WifiOff } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { t } from '../i18n';
import { api, can, useApp, type AuthState } from '../store';
import { navigate } from '../router';
import { fmtBytes, fmtPct } from './format';

/** La pastille de connexion de la barre du haut. */
export function StatusPill() {
  const { connected, metrics, auth, loadMetrics } = useApp(useShallow((s) => ({ connected: s.connected, metrics: s.metrics, auth: s.auth, loadMetrics: s.loadMetrics })));
  const user = auth?.user;
  const [lost, setLost] = useState(false);
  const [hot, setHot] = useState(false);
  const ever = useRef(false);

  // Serveur perdu : socket fermé depuis 800 ms (4 s au premier chargement).
  useEffect(() => {
    if (connected) {
      ever.current = true;
      setLost(false);
      return;
    }
    const id = setTimeout(() => setLost(true), ever.current ? 800 : 4000);
    return () => clearTimeout(id);
  }, [connected]);

  // Pendant la perte : la session existe-t-elle encore ?
  useEffect(() => {
    if (!lost) return;
    const id = setInterval(async () => {
      try {
        const state = await api<AuthState>('GET', '/api/auth/state');
        if (!state.user) useApp.setState({ auth: { ...state, user: null } });
      } catch {
        /* toujours perdu */
      }
    }, 3000);
    return () => clearInterval(id);
  }, [lost]);

  // Données machine toutes les 3 s.
  useEffect(() => {
    void loadMetrics();
    const id = setInterval(() => void loadMetrics(), 3000);
    return () => clearInterval(id);
  }, [loadMetrics]);

  const m = metrics?.machine;
  useEffect(() => {
    if (!m) return;
    setHot((h) => (h ? m.cpu >= 80 : m.cpu > 85));
  }, [m]);

  const state = lost ? 'lost' : !connected ? 'wait' : hot ? 'hot' : 'ok';
  const stateText = state === 'lost' ? t('Server lost · reconnecting…') : state === 'wait' ? t('Connecting…') : state === 'hot' ? t('Machine under heavy load') : t('Connected');
  const tip = [
    auth?.serverName,
    stateText,
    m ? t('CPU {pct} · {n} cores', { pct: fmtPct(m.cpu), n: m.cores }) : null,
    m ? t('Memory {used} of {total}', { used: fmtBytes(m.memUsed), total: fmtBytes(m.memTotal) }) : null,
    m?.hostname,
  ]
    .filter(Boolean)
    .join('\n');

  const body =
    state === 'lost' ? (
      <>
        <WifiOff size={14} />
        <span className="ux1-lost-txt">
          <span>{t('Server lost')}</span>
          <span className="ux1-lost-sep"> · </span>
          <span>{t('reconnecting…')}</span>
        </span>
      </>
    ) : state === 'hot' && m ? (
      <>
        <TriangleAlert size={14} /> {t('CPU {pct}', { pct: fmtPct(m.cpu) })}
      </>
    ) : (
      <>
        <span className="ux1-dot" /> {state === 'wait' ? t('Connecting…') : t('Connected')}
      </>
    );

  const cls = `ux1-pill ${state === 'ok' ? '' : state}`;
  if (can.admin(user))
    return (
      <button type="button" className={cls} title={tip} onClick={() => navigate('/system/performance')}>
        {body}
      </button>
    );
  return (
    <span className={cls} title={tip}>
      {body}
    </span>
  );
}
