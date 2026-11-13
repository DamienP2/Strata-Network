import { useMemo, useState } from 'react';
import { Bell, CheckCheck } from 'lucide-react';
import { t, tn } from '../i18n';
import { api } from '../store';
import { navigate } from '../router';
import { fmtDateTime } from '../ui/format';
import { ALERT_KINDS, ALERT_TONE, alertLabel, displayName, factText, fail, useAllAddresses, useList } from '../net/model';
import type { Alert, AlertKind, Network } from '../net/types';

export function Alerts() {
  const [alerts, load] = useList<Alert[]>('alerts', '/api/alerts');
  const [networks] = useList<Network[]>('networks', '/api/networks');
  const addrs = useAllAddresses(networks);
  const [kind, setKind] = useState<AlertKind | 'all'>('all');

  const list = useMemo(() => alerts ?? [], [alerts]);
  const shown = useMemo(() => list.filter((a) => kind === 'all' || a.kind === kind), [list, kind]);
  const unseen = list.filter((a) => !a.seen).length;

  const seen = async (ids: string[] | null) => {
    try {
      await api('POST', '/api/alerts/seen', { ids });
      void load();
    } catch (e) {
      fail(e);
    }
  };

  const open = (a: Alert) => {
    if (!a.seen) void seen([a.id]);
    if (a.networkId) navigate(`/scan/${a.networkId}?ip=${encodeURIComponent(a.ip)}`);
  };

  if (!alerts) return <div className="loading">{t('Loading…')}</div>;
  return (
    <main className="library">
      <div className="library-inner wide">
        <div className="settings-head">
          <div>
            <h1>{t('Alerts')}</h1>
            <p className="muted">
              {tn(list.length, '{n} alert', '{n} alerts')}
              {unseen > 0 && ` · ${tn(unseen, '{n} unseen alert', '{n} unseen alerts')}`}
            </p>
          </div>
          <div className="spacer" />
          <button className="btn" disabled={unseen === 0} onClick={() => void seen(null)}>
            <CheckCheck size={14} /> {t('Mark all as seen')}
          </button>
        </div>
        <div className="media-toolbar">
          <div className="asr-chips">
            <button className={`asr-chip ${kind === 'all' ? 'on' : ''}`} onClick={() => setKind('all')}>
              {t('All alerts')} <span className="asr-chip-n">{list.length}</span>
            </button>
            {ALERT_KINDS.filter((k) => list.some((a) => a.kind === k)).map((k) => (
              <button key={k} className={`asr-chip ${kind === k ? 'on' : ''}`} onClick={() => setKind(k)}>
                {alertLabel(k)} <span className="asr-chip-n">{list.filter((a) => a.kind === k).length}</span>
              </button>
            ))}
          </div>
        </div>
        {shown.length === 0 ? (
          <div className="empty big">
            <Bell size={22} />
            <p>{kind !== 'all' ? t('No match') : t('No alert')}</p>
          </div>
        ) : (
          <div className="card-table">
            <table className="data-table">
              <thead>
                <tr>
                  <th />
                  <th>{t('Date')}</th>
                  <th>{t('Type')}</th>
                  <th>{t('IP')}</th>
                  <th>{t('Detail')}</th>
                  <th>{t('Network')}</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((a) => {
                  const host = addrs.get(a.ip);
                  const name = displayName(host, a.ip);
                  return (
                    <tr key={a.id} className={a.seen ? '' : 'unseen'} onClick={() => open(a)}>
                      <td>{!a.seen && <span className="nav-dot warn" title={t('Unseen')} />}</td>
                      <td>{fmtDateTime(a.at)}</td>
                      <td>
                        <span className={`kind ${ALERT_TONE[a.kind]}`}>{alertLabel(a.kind)}</span>
                      </td>
                      <td>
                        <span className="ip">{a.ip}</span> {name !== a.ip && <span className="muted">{name}</span>}
                      </td>
                      <td className="ip" title={a.detail}>
                        {factText(a.detail)}
                      </td>
                      <td>{networks?.find((n) => n.id === a.networkId)?.name ?? '—'}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </main>
  );
}
