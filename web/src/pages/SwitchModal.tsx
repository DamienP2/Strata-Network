import { useEffect, useRef, useState, type FormEvent } from 'react';
import { CircleCheck, CircleX, GripVertical, LayoutGrid, Pencil, Plus, Radar, Save, Search, Trash2, X } from 'lucide-react';
import { t, tc, tn } from '../i18n';
import { api, can, useApp } from '../store';
import { Field, FieldGroup } from '../settings/ui';
import { Check, Segmented } from '../ui/fields';
import { Dropdown, type DropOption } from '../ui/Dropdown';
import { confirmDialog, promptDialog } from '../ui/dialogs';
import { errorText } from '../ui/serverError';
import { validIp } from '../net/ip';
import { fail, useList } from '../net/model';
import { MAX_PORTS, blocksOf, groupStarts, kindShort, portNameOf, rangeText, summary } from '../net/groups';
import type { Detected, PortGroup, PortKind, SnmpAgent, SnmpV3, Switch, SwitchModel } from '../net/types';

type How = 'snmp' | 'tpl' | 'free';
/** Une ligne de l'éditeur : un groupe de ports, le nombre tel que saisi. */
type Row = { key: number; kind: PortKind; count: string; naming: string; start?: number };
/** Les accès SNMP d'une détection réussie (pour créer l'agent ensuite). */
type Access = { ip: string; port: number; community: string; v3: SnmpV3 | null };

let rowKey = 0;
const toRows = (groups: PortGroup[]): Row[] => groups.map((g) => ({ key: ++rowKey, kind: g.kind, count: String(g.count), naming: g.naming, start: g.start }));
const countOf = (r: Row) => {
  const n = Number(r.count);
  return Number.isInteger(n) && n >= 1 && n <= MAX_PORTS ? n : null;
};

const kindOptions = (): DropOption<PortKind>[] => [
  { value: 'rj45', label: 'RJ45' },
  { value: 'sfp', label: 'SFP / SFP28' },
  { value: 'qsfp', label: 'QSFP / QSFP28' },
];

/** « Eth1/1 → 1/48 » en ne gardant que la fin du second : « 1/48 ». */
const lastPart = (first: string, last: string) => rangeText(first, last).split(' → ')[1] ?? last;

export function SwitchModal({ sw, agents, groups, onClose, onSaved }: { sw: Switch | null; agents: SnmpAgent[] | null; groups: string[]; onClose: () => void; onSaved: (s: Switch | null) => void }) {
  const user = useApp((s) => s.auth?.user);
  const admin = can.admin(user);
  const [how, setHow] = useState<How>(sw ? 'free' : 'snmp');

  // Le switch.
  const [name, setName] = useState(sw?.name ?? '');
  const [model, setModel] = useState(sw?.model ?? '');
  const [ip, setIp] = useState(sw?.ip ?? '');
  const [group, setGroup] = useState(sw?.group ?? groups[0] ?? '');
  const [snmp, setSnmp] = useState(sw?.snmp ?? '');
  const [online, setOnline] = useState(sw?.online ?? false);
  const [rows, setRows] = useState<Row[]>(() =>
    toRows(
      sw?.groups ?? [
        { kind: 'rj45', count: 24, naming: 'Gi1/0/{n}' },
        { kind: 'sfp', count: 4, naming: 'Te1/1/{n}' },
      ],
    ),
  );
  const [asModel, setAsModel] = useState(false);
  const [busy, setBusy] = useState(false);
  const [newGroups, setNewGroups] = useState<string[]>([]);

  // La détection.
  const [dIp, setDIp] = useState(sw?.ip ?? '');
  const [dPort, setDPort] = useState('161');
  const [version, setVersion] = useState<'v2c' | 'v3'>('v2c');
  const [community, setCommunity] = useState('public');
  const [v3, setV3] = useState<SnmpV3>({ user: '', auth: 'sha', authPassword: '', privacy: 'aes128', privPassword: '' });
  const setV = (patch: Partial<SnmpV3>) => setV3((o) => ({ ...o, ...patch }));
  const [pickAgent, setPickAgent] = useState('');
  const [detecting, setDetecting] = useState(false);
  const [found, setFound] = useState<Detected | null>(null);
  const [detectErr, setDetectErr] = useState<string | null>(null);
  const [access, setAccess] = useState<Access | null>(null);

  // Les modèles.
  const [models, loadModels] = useList<{ builtin: SwitchModel[]; mine: SwitchModel[] }>('switchModels', '/api/switch-models');
  const [q, setQ] = useState('');
  const [tpl, setTpl] = useState<string | null>(null);

  const chosen = agents?.find((a) => a.id === pickAgent);
  const dp = Number(dPort);
  const badDIp = dIp.trim() !== '' && !validIp(dIp.trim());
  const badDPort = !Number.isInteger(dp) || dp < 1 || dp > 65535;
  const badIp = ip.trim() !== '' && !validIp(ip.trim());

  // Les groupes valides, leurs premiers numéros et le total.
  const valid = rows.every((r) => countOf(r) !== null);
  const faceGroups: PortGroup[] = rows.map((r) => ({ kind: r.kind, count: countOf(r) ?? 0, naming: r.naming.trim(), ...(r.start !== undefined ? { start: r.start } : {}) }));
  const starts = groupStarts(faceGroups);
  const total = faceGroups.reduce((a, g) => a + g.count, 0);
  const badTotal = total < 1 || total > MAX_PORTS;

  // Sans agent, pas de lecture en direct.
  useEffect(() => {
    if (!snmp) setOnline(false);
  }, [snmp]);

  const detect = async () => {
    setDetecting(true);
    setDetectErr(null);
    const acc: Access | null = chosen ? null : { ip: dIp.trim(), port: dp, community: version === 'v2c' ? community : '', v3: version === 'v3' ? v3 : null };
    try {
      const r = await api<Detected>('POST', '/api/switches/detect', chosen ? { agentId: chosen.id } : acc);
      setFound(r);
      setAccess(acc);
      const s = r.suggestion;
      if (!name.trim() && s.name) setName(s.name);
      if (s.model) setModel(s.model);
      setIp(chosen?.ip ?? acc?.ip ?? ip);
      if (s.groups.length) setRows(toRows(s.groups));
      setTpl(s.matched);
      const agent = chosen ? chosen.id : admin ? 'new' : snmp;
      setSnmp(agent);
      setOnline(!!agent);
    } catch (e) {
      setFound(null);
      setDetectErr(errorText(e));
    }
    setDetecting(false);
  };

  const pickModel = (m: SwitchModel) => {
    setTpl(m.id);
    setModel(m.brand ? `${m.brand} ${m.model}` : m.model);
    setRows(toRows(m.groups));
  };

  const removeModel = async (m: SwitchModel) => {
    if (!(await confirmDialog({ title: t('Delete the model “{name}”?', { name: m.model }), confirm: t('Delete'), danger: true }))) return;
    try {
      await api('DELETE', `/api/switch-models/${m.id}`);
      if (tpl === m.id) setTpl(null);
      await loadModels();
    } catch (e) {
      fail(e);
    }
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy || !name.trim() || badIp || !valid || badTotal) return;
    setBusy(true);
    try {
      // Un nouvel agent d'abord : le switch le lit ensuite.
      let agent = snmp;
      if (agent === 'new' && access) {
        const created = await api<SnmpAgent>('POST', '/api/snmp', { name: name.trim(), ...access, readArp: false, readMulticast: false, readSwitch: true });
        agent = created.id;
        setSnmp(agent);
      }
      const body = { ...(sw ?? { ports: [] }), name: name.trim(), model: model.trim(), ip: ip.trim() || undefined, group: group.trim(), groups: faceGroups, snmp: agent || undefined };
      const saved = sw ? await api<Switch>('PUT', `/api/switches/${sw.id}`, body) : await api<Switch>('POST', '/api/switches', body);
      // Le switch existe : la suite peut échouer sans le recréer.
      try {
        if (agent && online !== saved.online) {
          await api('PATCH', `/api/switches/${saved.id}/online`, { online });
          saved.online = online;
        }
        if (asModel) await api('POST', '/api/switch-models', { brand: '', model: model.trim() || name.trim(), groups: faceGroups });
      } catch (err) {
        fail(err);
      }
      onSaved(saved);
    } catch (err) {
      fail(err);
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!sw || !(await confirmDialog({ title: t('Delete the switch “{name}”?', { name: sw.name }), confirm: t('Delete'), danger: true }))) return;
    try {
      await api('DELETE', `/api/switches/${sw.id}`);
      onSaved(null);
    } catch (err) {
      fail(err);
    }
  };

  // Groupe : ceux qui existent, ou un nouveau.
  const groupOptions: DropOption[] = [{ value: '', label: '—' }, ...[...new Set([...groups, ...newGroups, sw?.group ?? ''])].filter(Boolean).sort((a, b) => a.localeCompare(b)).map((g) => ({ value: g, label: g })), { value: '\u0000new', label: t('New group…') }];
  const pickGroup = async (v: string) => {
    if (v !== '\u0000new') return setGroup(v);
    const g = (await promptDialog({ title: t('New group'), label: t('Name'), confirm: t('Add') }))?.trim();
    if (!g) return;
    setNewGroups((l) => [...l, g]);
    setGroup(g);
  };

  // Agent SNMP : aucun, un existant, ou celui que la détection va créer.
  const agentOptions: DropOption[] = [{ value: '', label: '—' }, ...(agents ?? []).map((a) => ({ value: a.id, label: a.name, detail: a.ip }))];
  if (sw?.snmp && !agents?.some((a) => a.id === sw.snmp)) agentOptions.push({ value: sw.snmp, label: sw.snmp });
  if (access && admin) agentOptions.push({ value: 'new', label: t('New · {ip} ({version})', { ip: access.ip, version: access.v3 ? 'v3' : 'v2c' }) });
  const existingAgents: DropOption[] = [{ value: '', label: '—' }, ...(agents ?? []).map((a) => ({ value: a.id, label: a.name, detail: a.ip }))];

  return (
    <div className="modal-back" onPointerDown={onClose}>
      <form className="modal xwide" role="dialog" aria-labelledby="sw-title" onPointerDown={(e) => e.stopPropagation()} onSubmit={submit}>
        <header>
          <h2 id="sw-title">{sw ? t('Edit {name}', { name: sw.name }) : t('New switch')}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={onClose}>
            <X size={16} />
          </button>
        </header>

        <div className="how">
          <Segmented
            value={how}
            onChange={setHow}
            options={[
              {
                value: 'snmp',
                label: (
                  <>
                    <Radar size={13} /> {t('Detect by SNMP')}
                  </>
                ),
              },
              {
                value: 'tpl',
                label: (
                  <>
                    <LayoutGrid size={13} /> {t('Choose a model')}
                  </>
                ),
              },
              {
                value: 'free',
                label: (
                  <>
                    <Pencil size={13} /> {t('By hand')}
                  </>
                ),
              },
            ]}
          />
        </div>

        {how === 'snmp' && (
          <div className="box">
            <div className="box-row">
              <Field label={t('IP address')}>
                <input className="mono" value={chosen ? chosen.ip : dIp} disabled={!!chosen} aria-invalid={badDIp} autoFocus={!sw} autoComplete="off" onChange={(e) => setDIp(e.target.value)} />
              </Field>
              <label className="set-field narrow">
                <span className="set-label">{t('Port')}</span>
                <input className="mono" value={chosen ? String(chosen.port) : dPort} disabled={!!chosen} inputMode="numeric" aria-invalid={badDPort} autoComplete="off" onChange={(e) => setDPort(e.target.value)} />
              </label>
              <div className="set-field narrow">
                <span className="set-label">{t('Version')}</span>
                <Segmented
                  value={chosen ? (chosen.v3 ? 'v3' : 'v2c') : version}
                  onChange={setVersion}
                  options={[
                    { value: 'v2c', label: 'v2c', disabled: !!chosen },
                    { value: 'v3', label: 'v3', disabled: !!chosen },
                  ]}
                />
              </div>
              <span className="or">{t('or')}</span>
              <FieldGroup label={t('Existing agent')}>
                <Dropdown value={pickAgent} options={existingAgents} onChange={setPickAgent} />
              </FieldGroup>
            </div>
            {!chosen && (
              <div className="box-row">
                {version === 'v2c' ? (
                  <label className="set-field community">
                    <span className="set-label">{t('Community')}</span>
                    <input type="password" value={community} autoComplete="new-password" onChange={(e) => setCommunity(e.target.value)} />
                  </label>
                ) : (
                  <>
                    <Field label={t('SNMP user')}>
                      <input value={v3.user} autoComplete="off" onChange={(e) => setV({ user: e.target.value })} />
                    </Field>
                    <div className="set-field fit">
                      <span className="set-label">{t('Authentication')}</span>
                      <Segmented
                        value={v3.auth}
                        onChange={(auth) => setV(auth === 'none' ? { auth, privacy: 'none' } : { auth })}
                        options={[
                          { value: 'none', label: tc('snmpAuth', 'None') },
                          { value: 'md5', label: 'MD5' },
                          { value: 'sha', label: 'SHA' },
                          { value: 'sha256', label: 'SHA-256' },
                        ]}
                      />
                    </div>
                    <Field label={t('Password')}>
                      <input type="password" value={v3.authPassword} disabled={v3.auth === 'none'} autoComplete="new-password" onChange={(e) => setV({ authPassword: e.target.value })} />
                    </Field>
                    <div className="set-field fit">
                      <span className="set-label">{t('Encryption')}</span>
                      <Segmented
                        value={v3.privacy}
                        onChange={(privacy) => setV({ privacy })}
                        options={[
                          { value: 'none', label: tc('snmpPriv', 'None') },
                          { value: 'des', label: 'DES', disabled: v3.auth === 'none' },
                          { value: 'aes128', label: 'AES-128', disabled: v3.auth === 'none' },
                        ]}
                      />
                    </div>
                    <Field label={t('Password')}>
                      <input type="password" value={v3.privPassword} disabled={v3.privacy === 'none'} autoComplete="new-password" onChange={(e) => setV({ privPassword: e.target.value })} />
                    </Field>
                  </>
                )}
              </div>
            )}
            <div className="box-row center">
              <button type="button" className="btn" disabled={detecting || (!chosen && (!dIp.trim() || badDIp || badDPort))} onClick={() => void detect()}>
                <Radar size={14} className={detecting ? 'spin' : undefined} /> {t('Detect')}
              </button>
              {found && !detecting && <FoundLine found={found} newAgent={snmp === 'new'} />}
              {detectErr && !detecting && (
                <span className="snmp-result bad" title={detectErr}>
                  <CircleX size={14} />
                  <span>{detectErr}</span>
                </span>
              )}
            </div>
          </div>
        )}

        {how === 'tpl' && <ModelPicker models={models} q={q} setQ={setQ} on={tpl} onPick={pickModel} onRemove={(m) => void removeModel(m)} />}

        <div className="set-grid">
          <Field label={t('Name')}>
            <input value={name} autoFocus={!!sw} autoComplete="off" onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label={t('Model')}>
            <input value={model} autoComplete="off" onChange={(e) => setModel(e.target.value)} />
          </Field>
          <Field label={t('IP address')}>
            <input className="mono" value={ip} placeholder={t('Optional')} aria-invalid={badIp} autoComplete="off" onChange={(e) => setIp(e.target.value)} />
          </Field>
          <FieldGroup label={t('Group')}>
            <Dropdown value={group} options={groupOptions} onChange={(v) => void pickGroup(v)} />
          </FieldGroup>
          <FieldGroup label={t('SNMP agent')}>
            <Dropdown value={snmp} options={agentOptions} onChange={setSnmp} />
          </FieldGroup>
          <FieldGroup label={t('Online')}>
            <Check checked={online} disabled={!snmp} title={snmp ? undefined : t('No SNMP agent')} onChange={setOnline}>
              {t('Read live')}
            </Check>
          </FieldGroup>
        </div>

        <GroupsEditor rows={rows} setRows={setRows} starts={starts} total={total} badTotal={badTotal} />

        <div className="set-field">
          <div className="pv-cap">
            <span className="set-label">{t('Preview')}</span>
            <span>{summary(faceGroups)}</span>
          </div>
          <div className="preview">
            {faceGroups.flatMap((g, i) =>
              blocksOf(Array.from({ length: g.count }, (_, k) => k)).map((b, j) => (
                <div key={`${i}-${j}`} className={`pv-block ${g.kind}`}>
                  {b.map((k) => (
                    <i key={k} />
                  ))}
                </div>
              )),
            )}
          </div>
        </div>

        <footer>
          {sw && (
            <button type="button" className="btn danger" onClick={() => void remove()}>
              <Trash2 size={14} /> {t('Delete')}
            </button>
          )}
          <span className="foot-check">
            <Check checked={asModel} onChange={setAsModel}>
              {t('Save as a model')}
            </Check>
          </span>
          <button type="button" className="btn" onClick={onClose}>
            {t('Cancel')}
          </button>
          <button type="submit" className="btn primary" disabled={!name.trim() || badIp || !valid || badTotal || busy}>
            {sw ? <Save size={14} /> : <Plus size={14} />} {sw ? t('Save') : t('Create the switch')}
          </button>
        </footer>
      </form>
    </div>
  );
}

/** Le résultat de la détection : nom, modèle, version, numéro de série, ports. */
function FoundLine({ found, newAgent }: { found: Detected; newAgent: boolean }) {
  const s = found.system;
  const model = found.suggestion.model || s.model;
  const n = found.names;
  const ports =
    n.length > 1
      ? t('{n} ports {first} to {last}', { n: n.length, first: n[0], last: lastPart(n[0], n[n.length - 1]) })
      : n.length === 1
        ? t('1 port {name}', { name: n[0] })
        : t('no Ethernet port');
  const facts = [s.firmware, s.serial && t('No. {serial}', { serial: s.serial }), ports, found.suggestion.matched && t('model recognised')].filter(Boolean).join(' · ');
  return (
    <div className="found">
      <CircleCheck size={18} />
      <div>
        <b>{[s.name, model].filter(Boolean).join(' · ') || s.description}</b>
        <span>{facts}</span>
      </div>
      {newAgent && <span className="badge">{t('the SNMP agent will be created')}</span>}
    </div>
  );
}

/** Les modèles fournis (par marque) et ceux des utilisateurs, à choisir. */
function ModelPicker({
  models,
  q,
  setQ,
  on,
  onPick,
  onRemove,
}: {
  models: { builtin: SwitchModel[]; mine: SwitchModel[] } | null;
  q: string;
  setQ: (v: string) => void;
  on: string | null;
  onPick: (m: SwitchModel) => void;
  onRemove: (m: SwitchModel) => void;
}) {
  const needle = q.trim().toLowerCase();
  const hit = (m: SwitchModel) => !needle || `${m.brand} ${m.model} ${m.groups.map((g) => g.naming).join(' ')}`.toLowerCase().includes(needle);
  const brands = new Map<string, SwitchModel[]>();
  for (const m of models?.builtin ?? []) {
    if (!hit(m)) continue;
    if (!brands.has(m.brand)) brands.set(m.brand, []);
    brands.get(m.brand)!.push(m);
  }
  const mine = (models?.mine ?? []).filter(hit);
  const all = (models?.builtin.length ?? 0) + (models?.mine.length ?? 0);
  return (
    <div className="box">
      <div className="tpl-search">
        <label className="search">
          <Search size={14} />
          <input value={q} placeholder={t('Search for a model')} onChange={(e) => setQ(e.target.value)} />
        </label>
        <div className="spacer" />
        <span className="hint-line">
          {tn(all, '{n} model', '{n} models')}
          {models && models.mine.length > 0 && ` · ${t('{n} yours', { n: models.mine.length })}`}
        </span>
      </div>
      <div className="tpls">
        {!models && <div className="tpl-brand">{t('Loading…')}</div>}
        {models && brands.size === 0 && mine.length === 0 && <div className="tpl-brand">{t('No match')}</div>}
        {[...brands.entries()].map(([brand, list]) => [
          <div key={`b-${brand}`} className="tpl-brand">
            {brand}
          </div>,
          ...list.map((m) => <ModelCard key={m.id} m={m} on={on === m.id} onPick={() => onPick(m)} />),
        ])}
        {mine.length > 0 && (
          <>
            <div className="tpl-brand">{t('Yours')}</div>
            {mine.map((m) => (
              <ModelCard key={m.id} m={m} mine on={on === m.id} onPick={() => onPick(m)} onRemove={() => onRemove(m)} />
            ))}
          </>
        )}
      </div>
    </div>
  );
}

function ModelCard({ m, mine, on, onPick, onRemove }: { m: SwitchModel; mine?: boolean; on: boolean; onPick: () => void; onRemove?: () => void }) {
  const namings = [...new Set(m.groups.map((g) => g.naming))].join(' · ');
  return (
    <div
      className={`tpl ${on ? 'on' : ''} ${mine ? 'mine' : ''}`}
      role="button"
      tabIndex={0}
      onClick={onPick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onPick();
        }
      }}
    >
      <b>{m.model}</b>
      <small>
        {m.groups.map((g) => `${g.count} ${kindShort(g.kind)}`).join(' + ')} · {namings}
      </small>
      <span className="mini">
        {m.groups.flatMap((g, i) =>
          Array.from({ length: Math.min(g.count, 48) }, (_, k) => <i key={`${i}-${k}`} className={g.kind === 'rj45' ? undefined : g.kind === 'sfp' ? 's' : 'q'} />),
        )}
      </span>
      {onRemove && (
        <button
          type="button"
          className="icon-btn ghost tpl-del"
          title={t('Delete the model')}
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          onKeyDown={(e) => e.stopPropagation()}
        >
          <Trash2 size={13} />
        </button>
      )}
    </div>
  );
}

/** Les groupes de ports : type, nombre, nommage, plage calculée ; glisser pour les ranger. */
function GroupsEditor({ rows, setRows, starts, total, badTotal }: { rows: Row[]; setRows: (f: (r: Row[]) => Row[]) => void; starts: number[]; total: number; badTotal: boolean }) {
  const list = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const set = (key: number, patch: Partial<Row>) => setRows((rs) => rs.map((r) => (r.key === key ? { ...r, ...patch } : r)));

  // Glisser par la poignée : la ligne prend la place de celle sous le pointeur.
  useEffect(() => {
    if (dragging === null) return;
    const move = (e: PointerEvent) => {
      const els = [...(list.current?.querySelectorAll<HTMLElement>('.grp') ?? [])];
      const to = els.findIndex((el) => {
        const r = el.getBoundingClientRect();
        return e.clientY < r.top + r.height / 2;
      });
      setRows((rs) => {
        const from = rs.findIndex((r) => r.key === dragging);
        let at = to === -1 ? rs.length - 1 : to > from ? to - 1 : to;
        at = Math.max(0, Math.min(rs.length - 1, at));
        if (from === -1 || at === from) return rs;
        const out = [...rs];
        const [r] = out.splice(from, 1);
        out.splice(at, 0, r);
        return out;
      });
    };
    const up = () => setDragging(null);
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', up);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', up);
    };
  }, [dragging, setRows]);

  const add = () =>
    setRows((rs) => {
      const last = rs[rs.length - 1];
      return [...rs, { key: ++rowKey, kind: last?.kind === 'rj45' || !last ? 'sfp' : last.kind, count: '4', naming: last?.naming ?? 'Te1/1/{n}' }];
    });

  return (
    <div className="set-field">
      <span className={`set-label ${badTotal ? 'bad' : ''}`} title={badTotal ? t('From 1 to {n} ports', { n: MAX_PORTS }) : undefined}>
        {t('Faceplate ports · {n}', { n: total })}
      </span>
      <div className="groups" ref={list}>
        <div className="grp-head">
          <span />
          <span>{t('Type')}</span>
          <span>{t('Count')}</span>
          <span>{t('Naming')}</span>
          <span>{t('Ports')}</span>
          <span />
        </div>
        {rows.map((r, i) => {
          const n = countOf(r);
          const range = n ? rangeText(portNameOf(r.naming, starts[i]), portNameOf(r.naming, starts[i] + n - 1)) : '—';
          return (
            <div key={r.key} className={`grp ${dragging === r.key ? 'dragging' : ''}`}>
              <span
                className="handle"
                title={t('Move')}
                onPointerDown={(e) => {
                  e.preventDefault();
                  setDragging(r.key);
                }}
              >
                <GripVertical size={14} />
              </span>
              <Dropdown value={r.kind} options={kindOptions()} onChange={(kind) => set(r.key, { kind })} />
              <input className="num" value={r.count} inputMode="numeric" aria-invalid={n === null} aria-label={t('Count')} autoComplete="off" onChange={(e) => set(r.key, { count: e.target.value })} />
              <input className="mono" value={r.naming} aria-label={t('Naming')} autoComplete="off" onChange={(e) => set(r.key, { naming: e.target.value })} />
              <span className="range" title={range}>
                {range}
              </span>
              <button type="button" className="icon-btn ghost" title={t('Remove the group')} disabled={rows.length === 1} onClick={() => setRows((rs) => rs.filter((x) => x.key !== r.key))}>
                <X size={13} />
              </button>
            </div>
          );
        })}
        <div>
          <button type="button" className="btn small" onClick={add}>
            <Plus size={13} /> {t('Port group')}
          </button>
        </div>
      </div>
    </div>
  );
}
