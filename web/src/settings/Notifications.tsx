import { useEffect, useMemo, useState, type FormEvent, type KeyboardEvent } from 'react';
import { Check as CheckIcon, CircleX, EthernetPort, Filter, Mail, Pencil, Plus, Radar, Radio, Save, Send, Trash2, Webhook as WebhookIcon, X } from 'lucide-react';
import { t, tc, tn } from '../i18n';
import { api, useApp } from '../store';
import { Card, Field, FieldGroup } from './ui';
import { Check, Segmented } from '../ui/fields';
import { confirmDialog } from '../ui/dialogs';
import { errorText } from '../ui/serverError';
import { fmtHm, fmtTime } from '../ui/format';
import { PageHead } from '../pages/SidePage';
import { ALERT_KINDS, ALERT_TONE, alertDetail, alertLabel, fail, useList } from '../net/model';
import type { Alert, AlertKind, Switch } from '../net/types';

type Security = 'none' | 'starttls' | 'tls';
type HookKind = 'teams' | 'slack' | 'generic';
export interface Webhook {
  id: string;
  name: string;
  kind: HookKind;
  url: string;
  enabled: boolean;
}
export interface NotificationSettings {
  email: { enabled: boolean; host: string; port: number; security: Security; user: string; password: string; from: string; to: string[] };
  webhooks: Webhook[];
  filter: { kinds: 'all' | string[]; groupSecs: number };
  appUrl: string;
}

const MACHINE_KINDS: string[] = ['newHost', 'hostGone', 'ipConflict', 'vlanMismatch', 'watchDown', 'watchUp'];
const SWITCH_KINDS: string[] = ['portDown', 'portFlap', 'portErrors', 'portSaturated', 'configDrift', 'opticLow', 'psuFault', 'fanFault', 'tempHigh', 'poeBudget', 'neighbourNew', 'neighbourGone'];

const hookKindLabel = (k: HookKind) => (k === 'teams' ? 'Teams' : k === 'slack' ? 'Slack' : t('Generic'));
const validEmail = (s: string) => /^[^\s@<>]+@[^\s@<>]+$/.test(s.trim()) || /^[^<>]*<[^\s@<>]+@[^\s@<>]+>$/.test(s.trim());
type TestState = { ok: true; at: number } | { ok: false; text: string } | 'run' | null;

export function Notifications() {
  const [saved, , setSaved] = useList<NotificationSettings>('notifications', '/api/notifications');
  const [draft, setDraft] = useState<NotificationSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [editing, setEditing] = useState<{ index: number | null } | null>(null);
  const [emailTest, setEmailTest] = useState<TestState>(null);
  const [hookTest, setHookTest] = useState<number | null>(null);
  const dirty = !!draft && !!saved && JSON.stringify(draft) !== JSON.stringify(saved);

  // Une copie locale ; rechargée quand le serveur change et qu'elle n'est pas modifiée.
  useEffect(() => {
    if (saved && (!draft || !dirty)) setDraft(saved);
  }, [saved]);

  if (!draft) return <div className="loading">{t('Loading…')}</div>;
  const set = (patch: Partial<NotificationSettings>) => setDraft({ ...draft, ...patch });
  const setEmail = (patch: Partial<NotificationSettings['email']>) => set({ email: { ...draft.email, ...patch } });

  const save = async (): Promise<NotificationSettings | null> => {
    setSaving(true);
    try {
      const r = await api<NotificationSettings>('PUT', '/api/notifications', draft);
      setSaved(r);
      setDraft(r);
      setSaving(false);
      return r;
    } catch (e) {
      fail(e);
      setSaving(false);
      return null;
    }
  };

  /** Un test part avec les réglages enregistrés : les changements sont enregistrés avant. */
  const testEmail = async () => {
    setEmailTest('run');
    const s = dirty ? await save() : saved;
    if (!s) return setEmailTest(null);
    try {
      await api('POST', '/api/notifications/test', { channel: 'email' });
      setEmailTest({ ok: true, at: Date.now() });
    } catch (e) {
      setEmailTest({ ok: false, text: errorText(e) });
    }
  };
  const testHook = async (i: number) => {
    setHookTest(i);
    const s = dirty ? await save() : saved;
    const id = s?.webhooks[i]?.id;
    if (id) {
      try {
        await api('POST', '/api/notifications/test', { channel: id });
        useApp.getState().toast(t('Test message sent'));
      } catch (e) {
        fail(e);
      }
    }
    setHookTest(null);
  };
  const removeHook = async (i: number) => {
    const w = draft.webhooks[i];
    if (!(await confirmDialog({ title: t('Delete the webhook “{name}”?', { name: w.name || hookKindLabel(w.kind) }), confirm: t('Delete'), danger: true }))) return;
    set({ webhooks: draft.webhooks.filter((_, j) => j !== i) });
  };
  const setHook = (i: number, patch: Partial<Webhook>) => set({ webhooks: draft.webhooks.map((w, j) => (j === i ? { ...w, ...patch } : w)) });

  const e = draft.email;
  const port = Number(e.port);
  const kinds = draft.filter.kinds;
  const chosen = (k: string) => kinds === 'all' || kinds.includes(k);
  const toggleKind = (k: string, on: boolean) => {
    const cur = kinds === 'all' ? [] : kinds;
    set({ filter: { ...draft.filter, kinds: on ? [...cur.filter((x) => x !== k), k] : cur.filter((x) => x !== k) } });
  };
  const others = ALERT_KINDS.filter((k) => !MACHINE_KINDS.includes(k) && !SWITCH_KINDS.includes(k));
  const groups: { id: string; icon: typeof Radar; title: string; kinds: AlertKind[] }[] = [
    { id: 'machines', icon: Radar, title: t('Machines'), kinds: ALERT_KINDS.filter((k) => MACHINE_KINDS.includes(k)) },
    { id: 'switches', icon: EthernetPort, title: t('Switches'), kinds: ALERT_KINDS.filter((k) => SWITCH_KINDS.includes(k)) },
    { id: 'multicast', icon: Radio, title: t('Multicast'), kinds: others },
  ];

  return (
    <>
      <PageHead title={t('Notifications')}>
        <button className="btn primary" disabled={!dirty || saving || (e.enabled && (!e.host.trim() || e.to.length === 0))} onClick={() => void save()}>
          <CheckIcon size={14} /> {t('Save')}
        </button>
      </PageHead>

      <Card icon={<Mail size={16} />} title={t('Email')} actions={<Check checked={e.enabled} onChange={(enabled) => setEmail({ enabled })}>{t('Enabled')}</Check>}>
        <div className="set-grid" style={{ gridTemplateColumns: 'minmax(0,2fr) 100px minmax(0,1.4fr)' }}>
          <Field label={t('SMTP server')}>
            <input className="mono" value={e.host} autoComplete="off" onChange={(ev) => setEmail({ host: ev.target.value })} />
          </Field>
          <Field label={t('Port')}>
            <input className="mono" value={String(e.port)} inputMode="numeric" aria-invalid={!Number.isInteger(port) || port < 1 || port > 65535} onChange={(ev) => setEmail({ port: Number(ev.target.value.replace(/\D/g, '')) || 0 })} />
          </Field>
          <FieldGroup label={t('Security')}>
            <Segmented
              value={e.security}
              onChange={(security) => setEmail({ security, port: e.port === 25 || e.port === 587 || e.port === 465 ? (security === 'tls' ? 465 : security === 'starttls' ? 587 : 25) : e.port })}
              options={[
                { value: 'none', label: tc('smtpSecurity', 'None') },
                { value: 'starttls', label: 'STARTTLS' },
                { value: 'tls', label: 'TLS' },
              ]}
            />
          </FieldGroup>
        </div>
        <div className="set-grid">
          <Field label={t('User')}>
            <input value={e.user} autoComplete="off" onChange={(ev) => setEmail({ user: ev.target.value })} />
          </Field>
          <Field label={t('Password')}>
            <input type="password" value={e.password} autoComplete="new-password" onChange={(ev) => setEmail({ password: ev.target.value })} />
          </Field>
          <Field label={t('Sender')}>
            <input value={e.from} autoComplete="off" aria-invalid={e.from.trim() !== '' && !validEmail(e.from)} onChange={(ev) => setEmail({ from: ev.target.value })} />
          </Field>
        </div>
        <FieldGroup label={t('Recipients')}>
          <Recipients value={e.to} onChange={(to) => setEmail({ to })} />
        </FieldGroup>
        <div className="testline">
          <button className="btn small" disabled={emailTest === 'run' || saving || !e.host.trim() || e.to.length === 0 || !e.from.trim()} onClick={() => void testEmail()}>
            <Send size={13} className={emailTest === 'run' ? 'spin' : undefined} /> {t('Send a test')}
          </button>
          {emailTest && emailTest !== 'run' && emailTest.ok && (
            <span className="ok-pill">
              <CheckIcon size={13} /> {t('Sent at {time}', { time: fmtHm(emailTest.at) })}
            </span>
          )}
          {emailTest && emailTest !== 'run' && !emailTest.ok && (
            <span className="snmp-result bad" title={emailTest.text}>
              <CircleX size={14} />
              <span>{emailTest.text}</span>
            </span>
          )}
        </div>
      </Card>

      <section className="set-card">
        <header>
          <WebhookIcon size={16} />
          <h2>{t('Webhooks')}</h2>
          <div className="spacer" />
          <button className="btn small" onClick={() => setEditing({ index: null })}>
            <Plus size={13} /> {t('Add')}
          </button>
        </header>
        {draft.webhooks.length === 0 ? (
          <div className="empty">{t('No webhook')}</div>
        ) : (
          <table className="user-table">
            <thead>
              <tr>
                <th>{t('Name')}</th>
                <th>{t('Type')}</th>
                <th>{t('Address')}</th>
                <th>{t('Enabled')}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {draft.webhooks.map((w, i) => (
                <tr key={w.id || `new-${i}`}>
                  <td>{w.name || '—'}</td>
                  <td>
                    <span className={`wh-kind ${w.kind}`}>{hookKindLabel(w.kind)}</span>
                  </td>
                  <td className="ip muted" title={w.url}>
                    {w.url}
                  </td>
                  <td>
                    <Check checked={w.enabled} onChange={(enabled) => setHook(i, { enabled })} />
                  </td>
                  <td style={{ textAlign: 'right', whiteSpace: 'nowrap' }}>
                    <button className="icon-btn ghost" title={t('Send a test')} disabled={hookTest !== null || saving} onClick={() => void testHook(i)}>
                      <Send size={14} className={hookTest === i ? 'spin' : undefined} />
                    </button>
                    <button className="icon-btn ghost" title={t('Edit')} onClick={() => setEditing({ index: i })}>
                      <Pencil size={14} />
                    </button>
                    <button className="icon-btn ghost" title={t('Delete')} onClick={() => void removeHook(i)}>
                      <Trash2 size={14} />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section className="set-card">
        <header>
          <Filter size={16} />
          <h2>{t('Sending')}</h2>
          <div className="spacer" />
          <Segmented
            value={kinds === 'all' ? 'all' : 'some'}
            onChange={(v) => set({ filter: { ...draft.filter, kinds: v === 'all' ? 'all' : [...ALERT_KINDS] } })}
            options={[
              { value: 'all', label: tc('notify', 'All alerts') },
              { value: 'some', label: t('Chosen alerts') },
            ]}
          />
        </header>
        <div className="set-body send-grid">
          <div className="send-main">
            <div className={`kinds ${kinds === 'all' ? 'off' : ''}`}>
              {groups.map((g) => (
                <div key={g.id} className="kgroup">
                  <h4>
                    <g.icon size={12} /> {g.title}
                  </h4>
                  {g.kinds.length === 0 && <span className="muted">—</span>}
                  {g.kinds.map((k) => (
                    <Check key={k} checked={chosen(k)} disabled={kinds === 'all'} onChange={(on) => toggleKind(k, on)}>
                      {alertLabel(k)}
                    </Check>
                  ))}
                </div>
              ))}
            </div>
            <span className="inline-num">
              {t('Group the alerts during')}
              <input
                value={String(draft.filter.groupSecs)}
                inputMode="numeric"
                onChange={(ev) => set({ filter: { ...draft.filter, groupSecs: Math.min(3600, Number(ev.target.value.replace(/\D/g, '')) || 0) } })}
              />
              {t('s')}
            </span>
          </div>
          <MailPreview settings={draft} />
        </div>
      </section>

      {editing && (
        <WebhookModal
          hook={editing.index === null ? null : draft.webhooks[editing.index]}
          onClose={() => setEditing(null)}
          onDone={(w) => {
            set({ webhooks: editing.index === null ? [...draft.webhooks, w] : draft.webhooks.map((x, j) => (j === editing.index ? w : x)) });
            setEditing(null);
          }}
        />
      )}
    </>
  );
}

/** Les adresses, une pastille chacune ; Entrée, virgule ou espace en ajoute une. */
function Recipients({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const [text, setText] = useState('');
  const add = () => {
    const parts = text
      .split(/[\s,;]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (parts.length === 0) return;
    onChange([...value, ...parts.filter((p) => !value.includes(p))]);
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
  const bad = text.trim() !== '' && !validEmail(text);
  return (
    <div className="rcpts">
      {value.map((a) => (
        <span key={a} className="rcpt" style={validEmail(a) ? undefined : { borderColor: 'var(--live)' }}>
          {a}
          <button type="button" title={t('Remove')} onClick={() => onChange(value.filter((x) => x !== a))}>
            <X size={12} />
          </button>
        </span>
      ))}
      <input value={text} placeholder={t('Add an address')} aria-invalid={bad} onChange={(ev) => setText(ev.target.value)} onKeyDown={key} onBlur={add} />
    </div>
  );
}

/** L'aperçu du prochain message : les dernières alertes que le filtre laisse passer. */
function MailPreview({ settings }: { settings: NotificationSettings }) {
  const serverName = useApp((s) => s.auth?.serverName) || 'Strata Network';
  const [alerts] = useList<Alert[]>('alerts', '/api/alerts');
  const [switches] = useList<Switch[]>('switches', '/api/switches');
  const kinds = settings.filter.kinds;
  const shown = useMemo(() => (alerts ?? []).filter((a) => kinds === 'all' || kinds.includes(a.kind)).slice(0, 3).reverse(), [alerts, kinds]);
  const to = settings.email.to.map((a) => a.replace(/@.*/, '@…')).join(', ');
  const place = (a: Alert) => {
    const sw = a.switchId ? switches?.find((s) => s.id === a.switchId) : undefined;
    if (!sw) return a.ip;
    const p = a.port !== undefined ? sw.ports.find((x) => x.n === a.port) : undefined;
    return p ? `${sw.name} · ${p.name}` : sw.name;
  };
  const subject =
    shown.length === 1 ? `[${serverName}] ${alertLabel(shown[0].kind)} · ${place(shown[0])}` : `[${serverName}] ${tn(shown.length, '{n} alert', '{n} alerts')}`;
  return (
    <div className="mailprev">
      <div className="ph">
        <b>{subject}</b>
        <span>{[to && t('to {who}', { who: to }), shown.length > 0 ? fmtHm(shown[shown.length - 1].at) : ''].filter(Boolean).join(' · ')}</span>
      </div>
      {shown.length === 0 && <div className="empty">{t('No alert')}</div>}
      {shown.map((a) => (
        <div key={a.id} className="pl">
          <time>{fmtTime(a.at)}</time>
          <span>
            <span className={`kind ${ALERT_TONE[a.kind] ?? ''}`}>{alertLabel(a.kind)}</span>
            {[place(a), alertDetail(a)].filter((x) => x && x !== '—').join(' · ')}
          </span>
        </div>
      ))}
    </div>
  );
}

function WebhookModal({ hook, onClose, onDone }: { hook: Webhook | null; onClose: () => void; onDone: (w: Webhook) => void }) {
  const [name, setName] = useState(hook?.name ?? '');
  const [kind, setKind] = useState<HookKind>(hook?.kind ?? 'teams');
  const [url, setUrl] = useState(hook?.url ?? '');
  const badUrl = url.trim() !== '' && !/^https?:\/\/\S+$/.test(url.trim());
  const submit = (ev: FormEvent) => {
    ev.preventDefault();
    if (!url.trim() || badUrl) return;
    onDone({ id: hook?.id ?? '', name: name.trim(), kind, url: url.trim(), enabled: hook?.enabled ?? true });
  };
  return (
    <div className="modal-back" onPointerDown={onClose}>
      <form className="modal" role="dialog" aria-labelledby="hook-title" onPointerDown={(ev) => ev.stopPropagation()} onSubmit={submit}>
        <header>
          <h2 id="hook-title">{hook ? t('Edit {name}', { name: hook.name || hookKindLabel(hook.kind) }) : t('New webhook')}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <Field label={t('Name')}>
          <input value={name} autoFocus={!hook} autoComplete="off" onChange={(ev) => setName(ev.target.value)} />
        </Field>
        <FieldGroup label={t('Type')}>
          <Segmented
            value={kind}
            onChange={setKind}
            options={[
              { value: 'teams', label: 'Teams' },
              { value: 'slack', label: 'Slack' },
              { value: 'generic', label: t('Generic') },
            ]}
          />
        </FieldGroup>
        <Field label={t('Address')}>
          <input className="mono" value={url} placeholder="https://" aria-invalid={badUrl} autoComplete="off" onChange={(ev) => setUrl(ev.target.value)} onFocus={(ev) => url.includes('••') && ev.target.select()} />
        </Field>
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            {t('Cancel')}
          </button>
          <button type="submit" className="btn primary" disabled={!url.trim() || badUrl}>
            {hook ? <Save size={14} /> : <Plus size={14} />} {hook ? t('Apply') : t('Add')}
          </button>
        </footer>
      </form>
    </div>
  );
}
