import { useState, type FormEvent } from 'react';
import { CircleCheck, CircleX, Network, Pencil, Plus, RefreshCw, Save, Trash2, X } from 'lucide-react';
import { t, tn } from '../i18n';
import { api } from '../store';
import { Field, FieldGroup } from './ui';
import { Check } from '../ui/fields';
import { confirmDialog } from '../ui/dialogs';
import { errorText } from '../ui/serverError';
import { fmtAgo, fmtDateTime } from '../ui/format';
import { PageHead } from '../pages/SidePage';
import { validIp } from '../net/ip';
import { fail, useList } from '../net/model';
import type { SnmpAgent } from '../net/types';

export function Snmp() {
  const [agents, , setAgents] = useList<SnmpAgent[]>('snmp', '/api/snmp');
  const [editing, setEditing] = useState<SnmpAgent | 'new' | null>(null);
  const [polling, setPolling] = useState<string | null>(null);

  const poll = async (a: SnmpAgent) => {
    setPolling(a.id);
    try {
      const r = await api<SnmpAgent>('POST', `/api/snmp/${a.id}/poll`);
      setAgents((agents ?? []).map((x) => (x.id === r.id ? r : x)));
    } catch (e) {
      fail(e);
    }
    setPolling(null);
  };

  const remove = async (a: SnmpAgent) => {
    if (!(await confirmDialog({ title: t('Delete the agent “{name}”?', { name: a.name }), confirm: t('Delete'), danger: true }))) return;
    try {
      await api('DELETE', `/api/snmp/${a.id}`);
    } catch (e) {
      fail(e);
    }
  };

  if (!agents) return <div className="loading">{t('Loading…')}</div>;
  return (
    <>
      <PageHead title={t('SNMP')} sub={tn(agents.length, '{n} agent', '{n} agents')}>
        <button className="btn primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> {t('New agent')}
        </button>
      </PageHead>
      {agents.length === 0 ? (
        <div className="empty big">
          <Network size={22} />
          <p>{t('No agent')}</p>
        </div>
      ) : (
        <table className="user-table">
          <thead>
            <tr>
              <th>{t('Name')}</th>
              <th>{t('Address')}</th>
              <th>{t('Reads')}</th>
              <th>{t('Last read')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {agents.map((a) => (
              <tr key={a.id}>
                <td>{a.name}</td>
                <td className="ip">{a.port === 161 ? a.ip : `${a.ip}:${a.port}`}</td>
                <td>
                  <span className="snmp-reads">
                    {a.readArp && <span className="badge">{t('ARP')}</span>}
                    {a.readMulticast && <span className="badge">{t('Multicast')}</span>}
                    {a.readSwitch && <span className="badge">{t('Switch')}</span>}
                  </span>
                </td>
                <td>
                  <PollResult agent={a} />
                </td>
                <td style={{ textAlign: 'right', whiteSpace: 'nowrap' }}>
                  <button className="icon-btn ghost" title={t('Read now')} disabled={polling !== null} onClick={() => void poll(a)}>
                    <RefreshCw size={14} className={polling === a.id ? 'spin' : undefined} />
                  </button>
                  <button className="icon-btn ghost" title={t('Edit')} onClick={() => setEditing(a)}>
                    <Pencil size={14} />
                  </button>
                  <button className="icon-btn ghost" title={t('Delete')} onClick={() => void remove(a)}>
                    <Trash2 size={14} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {editing && <AgentModal agent={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </>
  );
}

function PollResult({ agent }: { agent: SnmpAgent }) {
  const p = agent.lastPoll;
  if (!p) return <span className="muted">{t('never')}</span>;
  return (
    <span className={`snmp-result ${p.ok ? 'ok' : 'bad'}`} title={`${fmtDateTime(p.at)}${p.message ? ` · ${p.message}` : ''}`}>
      {p.ok ? <CircleCheck size={14} /> : <CircleX size={14} />}
      <span>{p.ok ? fmtAgo(p.at) : p.message || fmtAgo(p.at)}</span>
    </span>
  );
}

function AgentModal({ agent, onClose }: { agent: SnmpAgent | null; onClose: () => void }) {
  const [name, setName] = useState(agent?.name ?? '');
  const [ip, setIp] = useState(agent?.ip ?? '');
  const [port, setPort] = useState(String(agent?.port ?? 161));
  const [community, setCommunity] = useState(agent?.community ?? 'public');
  const [readArp, setReadArp] = useState(agent?.readArp ?? false);
  const [readMulticast, setReadMulticast] = useState(agent?.readMulticast ?? false);
  const [readSwitch, setReadSwitch] = useState(agent?.readSwitch ?? false);
  const [busy, setBusy] = useState(false);
  const [test, setTest] = useState<{ ok: boolean; text: string } | 'run' | null>(null);
  const p = Number(port);
  const badIp = ip.trim() !== '' && !validIp(ip.trim());
  const badPort = !Number.isInteger(p) || p < 1 || p > 65535;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy || badIp || badPort) return;
    setBusy(true);
    const body = { ...(agent ?? {}), name: name.trim(), ip: ip.trim(), port: p, community, readArp, readMulticast, readSwitch };
    try {
      if (agent) await api('PUT', `/api/snmp/${agent.id}`, body);
      else await api('POST', '/api/snmp', body);
      onClose();
    } catch (err) {
      fail(err);
      setBusy(false);
    }
  };

  const runTest = async () => {
    setTest('run');
    try {
      const r = await api<{ name: string; description: string }>('POST', '/api/snmp/test', { ip: ip.trim(), port: p, community, id: agent?.id });
      setTest({ ok: true, text: [r.name, r.description].filter(Boolean).join(' · ') || t('Answered') });
    } catch (err) {
      setTest({ ok: false, text: errorText(err) });
    }
  };

  return (
    <div className="modal-back" onPointerDown={onClose}>
      <form className="modal wide" role="dialog" aria-labelledby="snmp-title" onPointerDown={(e) => e.stopPropagation()} onSubmit={submit}>
        <header>
          <h2 id="snmp-title">{agent ? t('Edit {name}', { name: agent.name }) : t('New agent')}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="set-grid two">
          <Field label={t('Name')}>
            <input value={name} autoFocus={!agent} autoComplete="off" onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label={t('IP')}>
            <input className="mono" value={ip} aria-invalid={badIp} autoComplete="off" onChange={(e) => setIp(e.target.value)} />
          </Field>
          <Field label={t('Port')}>
            <input className="mono" value={port} inputMode="numeric" aria-invalid={badPort} autoComplete="off" onChange={(e) => setPort(e.target.value)} />
          </Field>
          <Field label={t('Community')}>
            <input type="password" value={community} autoComplete="new-password" onChange={(e) => setCommunity(e.target.value)} />
          </Field>
        </div>
        <FieldGroup label={t('Reads')}>
          <Check checked={readArp} onChange={setReadArp}>
            {t('Read the ARP table')}
          </Check>
          <Check checked={readMulticast} onChange={setReadMulticast}>
            {t('Read the multicast routes')}
          </Check>
          <Check checked={readSwitch} onChange={setReadSwitch}>
            {t('Read the switch (names, MACs, LLDP)')}
          </Check>
        </FieldGroup>
        {agent?.lastPoll && (
          <FieldGroup label={t('Last read')}>
            <span className={`snmp-result ${agent.lastPoll.ok ? 'ok' : 'bad'}`}>
              {agent.lastPoll.ok ? <CircleCheck size={14} /> : <CircleX size={14} />}
              <span>
                {fmtDateTime(agent.lastPoll.at)}
                {agent.lastPoll.message ? ` · ${agent.lastPoll.message}` : ''}
              </span>
            </span>
          </FieldGroup>
        )}
        <footer>
          <button type="button" className="btn left" disabled={!ip.trim() || badIp || badPort || test === 'run'} onClick={() => void runTest()}>
            <RefreshCw size={14} className={test === 'run' ? 'spin' : undefined} /> {t('Test')}
          </button>
          {test && test !== 'run' && (
            <span className={`snmp-result ${test.ok ? 'ok' : 'bad'}`} style={{ flex: 1 }} title={test.text}>
              {test.ok ? <CircleCheck size={14} /> : <CircleX size={14} />}
              <span>{test.text}</span>
            </span>
          )}
          <button type="button" className="btn" onClick={onClose}>
            {t('Cancel')}
          </button>
          <button type="submit" className="btn primary" disabled={!name.trim() || !ip.trim() || badIp || badPort || busy}>
            {agent ? <Save size={14} /> : <Plus size={14} />} {agent ? t('Save') : t('Create the agent')}
          </button>
        </footer>
      </form>
    </div>
  );
}
