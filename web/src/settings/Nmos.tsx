import { useEffect, useState } from 'react';
import { Check as CheckIcon, CircleX, Lock, Radar, RefreshCw, Server } from 'lucide-react';
import { t, tn } from '../i18n';
import { api, serverNow } from '../store';
import { Card, Field, FieldGroup } from './ui';
import { Check, Segmented } from '../ui/fields';
import { errorText } from '../ui/serverError';
import { fmtAgo } from '../ui/format';
import { PageHead } from '../pages/SidePage';
import { fail, useList } from '../net/model';
import { shortPort } from '../net/mcast';
import type { NmosFound, NmosNode, NmosSettings, NmosStatus } from '../net/nmos';

type TestState = { ok: true; nodes: number; senders: number; receivers: number } | { ok: false; text: string } | 'run' | null;

/** L'adresse avec une autre version d'API (…/x-nmos/query/v1.2 → v1.3). */
const withVersion = (url: string, v: string) => url.replace(/(\/x-nmos\/query\/)v1\.\d+\/?$/, `$1${v}`);

/** « lu il y a 4 s », recalculé chaque seconde. */
function ReadAgo({ at }: { at: number }) {
  const [, setTick] = useState(0);
  useEffect(() => {
    const i = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(i);
  }, []);
  const s = Math.max(0, Math.round((serverNow() - at) / 1000));
  return <>{s < 60 ? t('read {n} s ago', { n: s }) : t('read {n} min ago', { n: Math.floor(s / 60) })}</>;
}

export function Nmos() {
  const [saved, , setSaved] = useList<NmosSettings>('nmos', '/api/nmos');
  const [status] = useList<NmosStatus>('nmos', '/api/nmos/status');
  const [nodes] = useList<NmosNode[]>('nmos', '/api/nmos/nodes');
  const [found, setFound] = useState<NmosFound[]>([]);
  const [draft, setDraft] = useState<NmosSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [test, setTest] = useState<TestState>(null);
  const dirty = !!draft && !!saved && JSON.stringify(draft) !== JSON.stringify(saved);

  useEffect(() => {
    if (saved && (!draft || !dirty)) setDraft(saved);
  }, [saved]);
  useEffect(() => {
    api<{ registries: NmosFound[] }>('GET', '/api/nmos/discovered')
      .then((r) => setFound(r.registries))
      .catch(() => setFound([]));
  }, []);

  if (!draft) return <div className="loading">{t('Loading…')}</div>;
  const set = (patch: Partial<NmosSettings>) => {
    setDraft({ ...draft, ...patch });
    setTest(null);
  };

  const save = async () => {
    setSaving(true);
    try {
      const r = await api<NmosSettings>('PUT', '/api/nmos', draft);
      setSaved(r);
      setDraft(r);
    } catch (e) {
      fail(e);
    }
    setSaving(false);
  };

  const run = async () => {
    setTest('run');
    try {
      const r = await api<{ nodes: number; senders: number; receivers: number }>('POST', '/api/nmos/test', { url: draft.url, version: draft.version });
      setTest({ ok: true, ...r });
    } catch (e) {
      setTest({ ok: false, text: errorText(e) });
    }
  };

  const use = (f: NmosFound) => {
    const v = /\/(v1\.\d+)$/.exec(f.url)?.[1];
    set({ url: f.url, version: v === 'v1.2' ? 'v1.2' : 'v1.3', enabled: true });
  };

  const st = status && status.enabled && !dirty ? status : null;
  const shown = st && st.at > 0 ? st : null;
  const list = nodes ?? [];

  return (
    <>
      <PageHead title={t('NMOS')}>
        <span className="ro-badge">
          <Lock size={12} /> {t('Read only')}
        </span>
        <button className="btn primary" disabled={!dirty || saving || (draft.enabled && !draft.url.trim())} onClick={() => void save()}>
          <CheckIcon size={14} /> {t('Save')}
        </button>
      </PageHead>

      <Card icon={<Server size={16} />} title={t('Registry')} actions={<Check checked={draft.enabled} onChange={(enabled) => set({ enabled })}>{t('Enabled')}</Check>}>
        {found.length > 0 && (
          <div className="nmos-found">
            <Radar size={14} />
            <span className="grow">
              {t('Found by mDNS:')}{' '}
              {found.map((f, i) => (
                <span key={f.url}>
                  {i > 0 && ' · '}
                  <b>{f.name}</b> · <span className="ip">{`${f.ip ?? f.host}:${f.port}`}</span>
                  {f.priority !== undefined && ` · ${t('priority {n}', { n: f.priority })}`}
                </span>
              ))}
            </span>
            <button className="btn small" disabled={draft.url === found[0].url && draft.enabled} onClick={() => use(found[0])}>
              {t('Use')}
            </button>
          </div>
        )}
        <div className="set-grid" style={{ gridTemplateColumns: 'minmax(0,2fr) 140px 160px' }}>
          <Field label={t('Query API address')}>
            <input className="mono" value={draft.url} spellCheck={false} autoComplete="off" placeholder="http://" onChange={(e) => set({ url: e.target.value })} />
          </Field>
          <FieldGroup label={t('Version')}>
            <Segmented
              value={draft.version}
              onChange={(version) => set({ version, url: withVersion(draft.url, version) })}
              options={[
                { value: 'v1.2', label: 'v1.2' },
                { value: 'v1.3', label: 'v1.3' },
              ]}
            />
          </FieldGroup>
          <FieldGroup label={t('Refresh')}>
            <Segmented
              value={draft.refresh}
              onChange={(refresh) => set({ refresh })}
              options={[
                { value: 'live', label: t('Live') },
                { value: '60s', label: t('{n} s', { n: 60 }) },
              ]}
            />
          </FieldGroup>
        </div>
        <div className="testline">
          <button className="btn small" disabled={test === 'run' || !draft.url.trim()} onClick={() => void run()}>
            <RefreshCw size={13} className={test === 'run' ? 'spin' : undefined} /> {t('Test')}
          </button>
          {test && test !== 'run' && test.ok && (
            <span className="ok-pill">
              <CheckIcon size={13} /> {[t('Connected'), tn(test.nodes, '{n} node', '{n} nodes'), tn(test.senders, '{n} sender', '{n} senders'), tn(test.receivers, '{n} receiver', '{n} receivers')].join(' · ')}
            </span>
          )}
          {test && test !== 'run' && !test.ok && (
            <span className="snmp-result bad" title={test.text}>
              <CircleX size={14} />
              <span>{test.text}</span>
            </span>
          )}
          {!test && st?.error && (
            <span className="snmp-result bad" title={st.error}>
              <CircleX size={14} />
              <span>{errorText(st.error)}</span>
            </span>
          )}
          {!test && st && !st.error && st.at > 0 && (
            <span className="ok-pill">
              <CheckIcon size={13} /> {t('Connected')} · {st.refresh === 'live' ? t('live') : t('every 60 s')} · <ReadAgo at={st.at} />
            </span>
          )}
        </div>
      </Card>

      {shown && (
        <>
          <div className="kpis reg">
            <div className="kpi">
              <span className="set-label">
                <Server size={13} /> {t('Registry')}
              </span>
              <span className="v name" title={shown.url}>
                {shown.registry ?? '—'}
              </span>
              <span className="muted">{[`IS-04 ${shown.version}`, shown.backup ? t('backup: {name}', { name: shown.backup }) : ''].filter(Boolean).join(' · ')}</span>
            </div>
            <div className="kpi">
              <span className="set-label">{t('Nodes')}</span>
              <span className="v">{shown.nodes}</span>
            </div>
            <div className="kpi">
              <span className="set-label">{t('Devices')}</span>
              <span className="v">{shown.devices}</span>
            </div>
            <div className="kpi">
              <span className="set-label">{t('Senders')}</span>
              <span className="v">
                {shown.senders} <small>· {t('{n} video', { n: shown.video })}</small>
              </span>
            </div>
            <div className="kpi">
              <span className="set-label">{t('Receivers')}</span>
              <span className="v">
                {shown.receivers} <small>· {tn(shown.subscribed, '{n} subscriber', '{n} subscribers')}</small>
              </span>
            </div>
            <div className="kpi">
              <span className="set-label">{t('Unknown to the scan')}</span>
              <span className={`v ${shown.unknown ? 'warn-t' : ''}`}>{shown.unknown}</span>
            </div>
          </div>
          {list.length === 0 ? (
            <div className="empty">{t('No node')}</div>
          ) : (
            <table className="pt">
              <colgroup>
                <col style={{ width: '22%' }} />
                <col style={{ width: '16%' }} />
                <col style={{ width: 140 }} />
                <col style={{ width: 90 }} />
                <col style={{ width: 90 }} />
                <col />
                <col style={{ width: 120 }} />
              </colgroup>
              <thead>
                <tr>
                  <th>{t('Node')}</th>
                  <th>{t('Manufacturer · model')}</th>
                  <th>{t('IP')}</th>
                  <th className="num">{t('Senders')}</th>
                  <th className="num">{t('Receivers')}</th>
                  <th>{t('Connection')}</th>
                  <th>{t('Seen')}</th>
                </tr>
              </thead>
              <tbody>
                {list.map((n) => {
                  const at = n.at[0];
                  return (
                    <tr key={n.id} className={n.present ? '' : 'off'}>
                      <td title={[n.hostname, n.name && n.name !== n.label ? n.name : ''].filter(Boolean).join(' · ') || undefined}>{n.label}</td>
                      <td>{n.maker ?? <span className="muted2">—</span>}</td>
                      <td className="ip" title={n.ips.join('\n') || undefined}>
                        {n.ip ?? '—'}
                      </td>
                      <td className="num ip">{n.senders}</td>
                      <td className="num ip">{n.receivers}</td>
                      <td title={n.at.map((a) => `${a.switchName} ${a.port}`).join('\n') || undefined}>
                        {!n.inventory ? (
                          <span className="warn-t">{t('Not in inventory')}</span>
                        ) : at ? (
                          <>
                            {at.switchName} · <span className="ip">{shortPort(at.port)}</span>
                          </>
                        ) : (
                          <span className="muted2">—</span>
                        )}
                      </td>
                      <td>
                        <span className={`alive ${n.present ? '' : 'w'}`}>
                          <i />
                          {fmtAgo(n.seen)}
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </>
      )}
    </>
  );
}
