import { useEffect, useState, type KeyboardEvent } from 'react';
import { Check as CheckIcon, CircleX, Lock, Plus, Radar, RefreshCw, Server, X } from 'lucide-react';
import { t, tn } from '../i18n';
import { api, serverNow } from '../store';
import { Card, Field, FieldGroup } from './ui';
import { Check, Segmented } from '../ui/fields';
import { Dropdown } from '../ui/Dropdown';
import { errorText } from '../ui/serverError';
import { fmtAgo } from '../ui/format';
import { PageHead } from '../pages/SidePage';
import { fail, useList } from '../net/model';
import { shortPort } from '../net/mcast';
import { foundLabel, type NmosChecked, type NmosFound, type NmosNode, type NmosSettings, type NmosStatus } from '../net/nmos';
import type { Network } from '../net/types';

type TestState =
  | { ok: true; nodes: number; senders: number; receivers: number }
  | { ok: false; text: string }
  | { peers: true; mdns: number; ifaces: string[]; nodes: NmosChecked[] }
  | 'run'
  | null;

/** Une adresse de nœud : http(s)://hôte[:port][/x-nmos…], ou hôte[:port]. */
const validNodeUrl = (s: string) => /^((https?):\/\/)?[A-Za-z0-9._-]+(:\d{1,5})?(\/x-nmos[^\s?#]*)?\/?$/i.test(s.trim());
/** Une plage : a.b.c.d/p, a.b.c.d-a.b.c.d (ou -d), ou une adresse. */
const validRange = (s: string) => /^\d{1,3}(\.\d{1,3}){3}(\/\d{1,2}|-\d{1,3}(\.\d{1,3}){3}|-\d{1,3})?$/.test(s.trim());

/** Des pastilles ; Entrée, virgule ou espace en ajoute une (comme les destinataires des notifications). */
function Chips({ value, onChange, valid, placeholder, name }: { value: string[]; onChange: (v: string[]) => void; valid: (s: string) => boolean; placeholder: string; name?: (s: string) => string | undefined }) {
  const [text, setText] = useState('');
  const add = () => {
    const parts = text
      .split(/[\s,;]+/)
      .map((x) => x.trim())
      .filter(Boolean);
    if (parts.length === 0) return;
    onChange([...value, ...parts.filter((x) => !value.includes(x))]);
    setText('');
  };
  const key = (ev: KeyboardEvent<HTMLInputElement>) => {
    ev.stopPropagation();
    if (ev.key === 'Enter' || ev.key === ',' || ev.key === ';' || ev.key === ' ') {
      ev.preventDefault();
      add();
    } else if (ev.key === 'Backspace' && text === '' && value.length > 0) {
      onChange(value.slice(0, -1));
    }
  };
  const bad = text.trim() !== '' && !valid(text);
  return (
    <div className="rcpts">
      {value.map((a) => {
        const n = name?.(a);
        return (
          <span key={a} className="rcpt mono" title={n ? a : undefined} style={valid(a) ? undefined : { borderColor: 'var(--live)' }}>
            {n ?? a}
            <button type="button" title={t('Remove')} onClick={() => onChange(value.filter((x) => x !== a))}>
              <X size={12} />
            </button>
          </span>
        );
      })}
      <input className="mono" value={text} spellCheck={false} autoComplete="off" placeholder={value.length ? '' : placeholder} aria-invalid={bad} onChange={(ev) => setText(ev.target.value)} onKeyDown={key} onBlur={add} />
    </div>
  );
}

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
  const [networks] = useList<Network[]>('networks', '/api/networks');
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
    if (draft.source === 'nodes') {
      try {
        const r = await api<{ mdns: number; ifaces: string[]; nodes: NmosChecked[] }>('POST', '/api/nmos/test', { source: 'nodes', nodeUrls: draft.nodeUrls, ranges: draft.ranges });
        setTest({ peers: true, ...r });
      } catch (e) {
        setTest({ ok: false, text: errorText(e) });
      }
      return;
    }
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
  const shown = st && (st.at > 0 || (st.source === 'nodes' && st.polled > 0)) ? st : null;
  const list = nodes ?? [];
  const peers = draft.source === 'nodes';
  const direct = shown?.source === 'nodes';
  const netName = (r: string) => networks?.find((n) => n.range === r)?.name;
  const addNet = (networks ?? []).filter((n) => !draft.ranges.includes(n.range)).map((n) => ({ value: n.range, label: n.name, sub: n.range }));
  const pill = test && test !== 'run' && 'peers' in test ? test : null;
  const failed = pill ? pill.nodes.filter((n) => !n.ok) : [];
  const read = pill ? pill.nodes.filter((n) => n.ok) : [];

  return (
    <>
      <PageHead title={t('NMOS')}>
        <span className="ro-badge">
          <Lock size={12} /> {t('Read only')}
        </span>
        <button className="btn primary" disabled={!dirty || saving || (draft.enabled && !peers && !draft.url.trim())} onClick={() => void save()}>
          <CheckIcon size={14} /> {t('Save')}
        </button>
      </PageHead>

      <Card icon={peers ? <Radar size={16} /> : <Server size={16} />} title={peers ? t('Nodes') : t('Registry')} actions={<Check checked={draft.enabled} onChange={(enabled) => set({ enabled })}>{t('Enabled')}</Check>}>
        <Segmented
          className="nmos-source"
          value={draft.source}
          onChange={(source) => set({ source })}
          options={[
            { value: 'registry', label: t('Registry') },
            { value: 'nodes', label: t('Nodes') },
          ]}
        />
        {peers ? (
          <>
            <div className="set-grid" style={{ gridTemplateColumns: 'minmax(0,1fr) 160px' }}>
              <FieldGroup label={t('Ranges')}>
                <div className="nmos-ranges">
                  <Chips value={draft.ranges} onChange={(ranges) => set({ ranges })} valid={validRange} placeholder={t('Automatic')} name={netName} />
                  {addNet.length > 0 && (
                    <Dropdown
                      value={null}
                      options={addNet}
                      onChange={(r) => set({ ranges: [...draft.ranges, r] })}
                      title={t('Add a network')}
                      face={
                        <>
                          <Plus size={14} /> {t('Network')}
                        </>
                      }
                    />
                  )}
                </div>
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
            <FieldGroup label={t('Node addresses')}>
              <Chips value={draft.nodeUrls} onChange={(nodeUrls) => set({ nodeUrls })} valid={validNodeUrl} placeholder="http://10.1.1.7:8080" />
            </FieldGroup>
            <Check checked={draft.probeScan} onChange={(probeScan) => set({ probeScan })}>
              {t('Also look among the scanned machines')}
            </Check>
          </>
        ) : (
          <>
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
          </>
        )}
        <div className="testline">
          <button className="btn small" disabled={test === 'run' || (!peers && !draft.url.trim())} onClick={() => void run()}>
            <RefreshCw size={13} className={test === 'run' ? 'spin' : undefined} /> {t('Test')}
          </button>
          {pill && (
            <span className="ok-pill" title={read.map((n) => `${n.label ?? n.url} · ${n.url}`).join('\n') || undefined}>
              <CheckIcon size={13} /> {[t('mDNS: {n}', { n: tn(pill.mdns, '{n} node', '{n} nodes') }), tn(read.length, '{n} node read', '{n} nodes read')].join(' · ')}
            </span>
          )}
          {failed.length > 0 && (
            <span className="snmp-result bad" title={failed.map((n) => `${n.url} · ${errorText(n.error ?? '')}`).join('\n')}>
              <CircleX size={14} />
              <span>
                {failed[0].url} · {errorText(failed[0].error ?? '')}
                {failed.length > 1 && ` · +${failed.length - 1}`}
              </span>
            </span>
          )}
          {test && test !== 'run' && 'ok' in test && test.ok && (
            <span className="ok-pill">
              <CheckIcon size={13} /> {[t('Connected'), tn(test.nodes, '{n} node', '{n} nodes'), tn(test.senders, '{n} sender', '{n} senders'), tn(test.receivers, '{n} receiver', '{n} receivers')].join(' · ')}
            </span>
          )}
          {test && test !== 'run' && 'ok' in test && !test.ok && (
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
              <CheckIcon size={13} /> {st.source === 'nodes' ? tn(st.answering, '{n} node', '{n} nodes') : t('Connected')} · {st.refresh === 'live' ? t('live') : t('every 60 s')} · <ReadAgo at={st.at} />
            </span>
          )}
        </div>
      </Card>

      {shown && (
        <>
          <div className="kpis reg">
            {direct ? (
              <div className="kpi">
                <span className="set-label">
                  <Radar size={13} /> {t('Discovery')}
                </span>
                <span className="v name" title={shown.ifaces.length ? t('mDNS: {n}', { n: shown.ifaces.join(' · ') }) : undefined}>
                  {[`${t('mDNS')} ${shown.mdns}`, `${t('Scan')} ${shown.scan}`, `${t('Manual')} ${shown.manual}`].join(' · ')}
                </span>
                <span className="muted">{shown.apis.length ? `IS-04 ${shown.apis.join(' · ')}` : '—'}</span>
              </div>
            ) : (
              <div className="kpi">
                <span className="set-label">
                  <Server size={13} /> {t('Registry')}
                </span>
                <span className="v name" title={shown.url}>
                  {shown.registry ?? '—'}
                </span>
                <span className="muted">{[`IS-04 ${shown.version}`, shown.backup ? t('backup: {name}', { name: shown.backup }) : ''].filter(Boolean).join(' · ')}</span>
              </div>
            )}
            <div className="kpi">
              <span className="set-label">{t('Nodes')}</span>
              <span className="v">
                {shown.nodes}
                {direct && shown.unreachable > 0 && <small className="warn-t"> · {tn(shown.unreachable, '{n} not answering', '{n} not answering')}</small>}
              </span>
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
                <col style={{ width: '18%' }} />
                <col style={{ width: '13%' }} />
                <col style={{ width: 120 }} />
                <col style={{ width: 120 }} />
                <col style={{ width: 84 }} />
                <col style={{ width: 84 }} />
                <col />
                {direct && <col style={{ width: 150 }} />}
                <col style={{ width: 110 }} />
              </colgroup>
              <thead>
                <tr>
                  <th>{t('Node')}</th>
                  <th>{t('Manufacturer · model')}</th>
                  <th>{t('Management')}</th>
                  <th>{t('Media')}</th>
                  <th className="num">{t('Senders')}</th>
                  <th className="num">{t('Receivers')}</th>
                  <th>{t('Connection')}</th>
                  {direct && <th>{t('Found by')}</th>}
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
                      <td className="ip" title={[n.url, n.api ? `IS-04 ${n.api}` : ''].filter(Boolean).join(' · ') || undefined}>
                        {n.ip ?? '—'}
                      </td>
                      <td className="ip" title={n.media.join('\n') || undefined}>
                        {n.media.length ? n.media[0] : <span className="muted2">—</span>}
                        {n.media.length > 1 && <span className="muted2"> +{n.media.length - 1}</span>}
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
                      {direct && (
                        <td>
                          {n.foundBy.map((f) => (
                            <span key={f} className="badge nmos-by">
                              {foundLabel(f)}
                            </span>
                          ))}
                        </td>
                      )}
                      <td title={n.error ? errorText(n.error) : undefined}>
                        <span className={`alive ${n.present && !n.error ? '' : 'w'}`}>
                          <i />
                          {n.seen > 0 ? fmtAgo(n.seen) : '—'}
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
