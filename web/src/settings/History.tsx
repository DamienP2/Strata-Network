import { useMemo, useState } from 'react';
import { History as HistoryIcon, Search } from 'lucide-react';
import { t, tn } from '../i18n';
import { fmtDateTime } from '../ui/format';
import { PageHead } from '../pages/SidePage';
import { factText, useList } from '../net/model';
import type { HistoryEntry } from '../net/types';

/** Les actions écrites par le serveur (anglais) → texte de l'interface. */
function actionText(a: string): string {
  const known: Record<string, () => string> = {
    'created network': () => t('created network'),
    'changed network': () => t('changed network'),
    'deleted network': () => t('deleted network'),
    scanned: () => t('scanned'),
    'stopped scan': () => t('stopped scan'),
    'scanned address': () => t('scanned address'),
    'changed address': () => t('changed address'),
    'imported csv': () => t('imported csv'),
    'created vlan': () => t('created vlan'),
    'changed vlan': () => t('changed vlan'),
    'deleted vlan': () => t('deleted vlan'),
    'created switch': () => t('created switch'),
    'changed switch': () => t('changed switch'),
    'deleted switch': () => t('deleted switch'),
    'changed port': () => t('changed port'),
    'changed ports': () => t('changed ports'),
    'created snmp agent': () => t('created snmp agent'),
    'changed snmp agent': () => t('changed snmp agent'),
    'deleted snmp agent': () => t('deleted snmp agent'),
    'polled snmp agent': () => t('polled snmp agent'),
    'changed multicast settings': () => t('changed multicast settings'),
    'checked stream': () => t('checked stream'),
    'labelled stream': () => t('labelled stream'),
    'browsed services': () => t('browsed services'),
    'marked alerts seen': () => t('marked alerts seen'),
  };
  return known[a.toLowerCase()]?.() ?? a;
}

export function History() {
  const [entries] = useList<HistoryEntry[]>('history', '/api/history?limit=2000');
  const [q, setQ] = useState('');
  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    const list = entries ?? [];
    return needle ? list.filter((e) => `${e.user} ${e.action} ${actionText(e.action)} ${e.target} ${e.detail}`.toLowerCase().includes(needle)) : list;
  }, [entries, q]);

  if (!entries) return <div className="loading">{t('Loading…')}</div>;
  return (
    <>
      <PageHead title={t('History')} sub={tn(entries.length, '{n} entry', '{n} entries')} />
      <div className="media-toolbar">
        <label className="search">
          <Search size={14} />
          <input value={q} placeholder={t('Search the history')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
        </label>
      </div>
      {shown.length === 0 ? (
        <div className="empty big">
          <HistoryIcon size={22} />
          <p>{q ? t('No match') : t('Nothing in the history')}</p>
        </div>
      ) : (
        <div className="card-table">
          <table className="data-table plain">
            <thead>
              <tr>
                <th>{t('Date')}</th>
                <th>{t('User')}</th>
                <th>{t('Action')}</th>
                <th>{t('Target')}</th>
                <th>{t('Detail')}</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((e, i) => (
                <tr key={`${e.at}-${i}`}>
                  <td>{fmtDateTime(e.at)}</td>
                  <td>{e.user === 'scheduler' ? t('Scheduler') : e.user}</td>
                  <td>{actionText(e.action)}</td>
                  <td className="ip">{e.target}</td>
                  <td title={e.detail}>{factText(e.detail)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}
