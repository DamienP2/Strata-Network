import { useEffect, useMemo, useState, type FormEvent, type MouseEvent } from 'react';
import { EthernetPort, LayoutGrid, List, Pencil, Plus, Save, Trash2, X } from 'lucide-react';
import { t, tn } from '../i18n';
import { api, can, useApp } from '../store';
import { navigate } from '../router';
import { Field } from '../settings/ui';
import { Segmented, TextInput } from '../ui/fields';
import { Dropdown, type DropOption } from '../ui/Dropdown';
import { confirmDialog } from '../ui/dialogs';
import { Empty } from '../ui/Empty';
import { SubTabs } from './SidePage';
import { byIp, validIp } from '../net/ip';
import { KIND_ICON, displayName, fail, useAllAddresses, useList, vlanById, vlanMismatch } from '../net/model';
import { VlanTag, vlanStyle } from '../net/parts';
import type { Address, Network, PortMode, SnmpAgent, Switch, SwitchPort, Vlan } from '../net/types';

export function Switches({ id }: { id?: string }) {
  const user = useApp((s) => s.auth?.user);
  const editable = can.edit(user);
  const [switches, , setSwitches] = useList<Switch[]>('switches', '/api/switches');
  const [vlans] = useList<Vlan[]>('vlans', '/api/vlans');
  const [networks] = useList<Network[]>('networks', '/api/networks');
  const [agents] = useList<SnmpAgent[]>('snmp', '/api/snmp');
  const addrs = useAllAddresses(networks);
  const [editing, setEditing] = useState<Switch | 'new' | null>(null);
  const sw = switches?.find((s) => s.id === id);
  const first = switches?.[0]?.id;

  useEffect(() => {
    if (switches && !sw && first) navigate(`/switches/${first}`, true);
  }, [switches, sw, first]);

  // Sections de la liste : le groupe de chaque switch (sans groupe d'abord).
  const groups = useMemo(() => {
    const m = new Map<string, Switch[]>();
    for (const s of [...(switches ?? [])].sort((a, b) => a.name.localeCompare(b.name))) {
      const g = s.group.trim();
      if (!m.has(g)) m.set(g, []);
      m.get(g)!.push(s);
    }
    return [...m.entries()].sort((a, b) => (a[0] === '' ? -1 : b[0] === '' ? 1 : a[0].localeCompare(b[0])));
  }, [switches]);

  const replace = (s: Switch) => setSwitches((switches ?? []).map((x) => (x.id === s.id ? s : x)));

  if (!switches) return <div className="loading">{t('Loading…')}</div>;
  return (
    <>
      <div className="net-page two sw-page">
        <aside className="lib-nav">
          {groups.length === 0 && (
            <div className="lib-nav-section">
              <span className="grow">{t('Switches')}</span>
              {editable && <NewButton onClick={() => setEditing('new')} />}
            </div>
          )}
          {groups.map(([g, list], i) => (
            <div key={g} className="sys-group">
              <div className="lib-nav-section">
                <span className="grow">{g || t('Switches')}</span>
                {i === 0 && editable && <NewButton onClick={() => setEditing('new')} />}
              </div>
              {list.map((s) => (
                <div
                  key={s.id}
                  className={`lib-nav-item ${s.id === sw?.id ? 'on' : ''}`}
                  role="link"
                  tabIndex={0}
                  aria-current={s.id === sw?.id ? 'page' : undefined}
                  onClick={() => navigate(`/switches/${s.id}`)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      navigate(`/switches/${s.id}`);
                    }
                  }}
                >
                  <EthernetPort size={15} />
                  <span className="grow">{s.name}</span>
                  <span className="count">{s.ports.length}</span>
                </div>
              ))}
            </div>
          ))}
        </aside>
        {sw ? (
          <SwitchView key={sw.id} sw={sw} switches={switches} vlans={vlans} addrs={addrs} editable={editable} onEdit={() => setEditing(sw)} onChanged={replace} />
        ) : (
          <main className="library">
            <Empty icon={EthernetPort} text={t('No switch')}>
              {editable && (
                <button className="btn primary" onClick={() => setEditing('new')}>
                  <Plus size={14} /> {t('New switch')}
                </button>
              )}
            </Empty>
          </main>
        )}
      </div>
      {editing && (
        <SwitchModal
          sw={editing === 'new' ? null : editing}
          agents={agents}
          groups={groups.map(([g]) => g).filter(Boolean)}
          onClose={() => setEditing(null)}
          onSaved={(s) => {
            setEditing(null);
            if (s) navigate(`/switches/${s.id}`);
          }}
        />
      )}
    </>
  );
}

function NewButton({ onClick }: { onClick: () => void }) {
  return (
    <button className="icon-btn ghost" style={{ width: 22, height: 22 }} title={t('New switch')} onClick={onClick}>
      <Plus size={13} />
    </button>
  );
}

/* ---------- Un switch : façade ou liste, port à droite ---------- */

function SwitchView({
  sw,
  switches,
  vlans,
  addrs,
  editable,
  onEdit,
  onChanged,
}: {
  sw: Switch;
  switches: Switch[];
  vlans: Vlan[] | null;
  addrs: Map<string, Address>;
  editable: boolean;
  onEdit: () => void;
  onChanged: (s: Switch) => void;
}) {
  const [tab, setTab] = useState<'face' | 'list'>('face');
  const [sel, setSel] = useState<number[]>([]);
  const copper = sw.ports.filter((p) => p.n <= sw.copper);
  const sfp = sw.ports.filter((p) => p.n > sw.copper);
  const blocks: SwitchPort[][] = [];
  for (let i = 0; i < copper.length; i += 12) blocks.push(copper.slice(i, i + 12));

  const pick = (n: number, e: MouseEvent) => {
    if (e.ctrlKey || e.metaKey || e.shiftKey) setSel((s) => (s.includes(n) ? s.filter((x) => x !== n) : [...s, n]));
    else setSel([n]);
  };

  const patch = async (ports: number[], set: Partial<Record<keyof SwitchPort, unknown>>) => {
    try {
      onChanged(await api<Switch>('PATCH', `/api/switches/${sw.id}/ports`, { ports, set }));
    } catch (e) {
      fail(e);
    }
  };

  // Légende : ports par VLAN, trunks, libres.
  const legend = useMemo(() => {
    const per = new Map<number, number>();
    let trunk = 0;
    let free = 0;
    for (const p of sw.ports) {
      if (p.mode === 'trunk') trunk++;
      else if (p.mode === 'off' || p.vlan === undefined) free++;
      else per.set(p.vlan, (per.get(p.vlan) ?? 0) + 1);
    }
    return { per: [...per.entries()].sort((a, b) => a[0] - b[0]), trunk, free };
  }, [sw.ports]);

  const linked = (p: SwitchPort) => !!p.peer || (!!p.host && !!addrs.get(p.host)?.up);
  const portTitle = (p: SwitchPort) => {
    const v = vlanById(vlans, p.vlan);
    const what = p.mode === 'off' ? t('disabled') : p.mode === 'trunk' ? t('trunk') : p.vlan !== undefined ? t('VLAN {id}', { id: p.vlan }) + (v ? ` ${v.name}` : '') : t('free');
    return [p.name || String(p.n), what, p.label].filter(Boolean).join(' · ');
  };
  const face = (p: SwitchPort, isSfp: boolean) => (
    <button
      key={p.n}
      type="button"
      className={`sw-port ${isSfp ? 'sfp' : ''} ${p.mode === 'trunk' ? 'trunk' : ''} ${p.mode === 'off' ? 'off' : ''} ${sel.includes(p.n) ? 'sel' : ''}`}
      style={p.mode !== 'off' ? vlanStyle(vlanById(vlans, p.vlan)) : undefined}
      title={portTitle(p)}
      onClick={(e) => pick(p.n, e)}
    >
      {p.n}
      {linked(p) && <span className="link" />}
    </button>
  );

  const sub = [sw.model, sw.ip, sw.sfp ? t('{a} + {b} ports', { a: sw.copper, b: sw.sfp }) : tn(sw.copper, '{n} port', '{n} ports')].filter(Boolean);
  return (
    <>
      <main className="library">
        <div className="library-inner wide">
          <div className="settings-head">
            <div>
              <h1>{sw.name}</h1>
              <p className="muted">
                {sub.map((s, i) => (
                  <span key={i}>
                    {i > 0 && ' · '}
                    {s === sw.ip ? <span className="mono">{s}</span> : s}
                  </span>
                ))}
              </p>
            </div>
            <div className="spacer" />
            {editable && (
              <button className="btn" onClick={onEdit}>
                <Pencil size={14} /> {t('Edit')}
              </button>
            )}
          </div>
          <SubTabs
            current={tab}
            onPick={(v) => setTab(v as 'face' | 'list')}
            items={[
              { id: 'face', icon: <LayoutGrid size={14} />, label: t('Faceplate') },
              { id: 'list', icon: <List size={14} />, label: t('List'), count: sw.ports.length },
            ]}
          />
          {tab === 'face' ? (
            <>
              <div className="sw-face">
                {blocks.map((b, i) => (
                  <div key={i} className="sw-block">
                    {b.map((p) => face(p, false))}
                  </div>
                ))}
                {sfp.length > 0 && <div className="sw-sfp">{sfp.map((p) => face(p, true))}</div>}
              </div>
              <div className="sw-legend">
                {legend.per.map(([id, n]) => {
                  const v = vlanById(vlans, id);
                  return (
                    <span key={id}>
                      <span className="vlan-sw" style={vlanStyle(v)} />
                      {`${id}${v ? ` ${v.name}` : ''} · ${n}`}
                    </span>
                  );
                })}
                {legend.trunk > 0 && (
                  <span>
                    <span className="trunk-key" />
                    {t('Trunk · {n}', { n: legend.trunk })}
                  </span>
                )}
                {legend.free > 0 && (
                  <span>
                    <span className="vlan-sw" />
                    {t('Free · {n}', { n: legend.free })}
                  </span>
                )}
              </div>
            </>
          ) : (
            <div className="card-table">
              <table className="data-table">
                <thead>
                  <tr>
                    <th className="num">{t('Port')}</th>
                    <th>{t('Name')}</th>
                    <th>{t('Label')}</th>
                    <th>{t('Mode')}</th>
                    <th>{t('VLAN')}</th>
                    <th>{t('Tagged')}</th>
                    <th>{t('Connected to')}</th>
                  </tr>
                </thead>
                <tbody>
                  {sw.ports.map((p) => {
                    const peer = p.peer ? switches.find((s) => s.id === p.peer!.id) : undefined;
                    const host = p.host ? addrs.get(p.host) : undefined;
                    return (
                      <tr key={p.n} className={`${sel.includes(p.n) ? 'sel' : ''} ${p.mode === 'off' ? 'free' : ''}`} onClick={(e) => pick(p.n, e)}>
                        <td className="num ip">{p.n}</td>
                        <td className="ip">{p.name}</td>
                        <td>{p.label}</td>
                        <td>{modeLabel(p.mode)}</td>
                        <td>{p.vlan !== undefined && p.mode !== 'off' ? <VlanTag vlan={vlanById(vlans, p.vlan)} id={p.vlan} /> : ''}</td>
                        <td className="ip">{p.mode === 'trunk' ? p.tagged.join(', ') : ''}</td>
                        <td>{peer ? `${peer.name} · ${peer.ports.find((x) => x.n === p.peer!.port)?.name ?? p.peer!.port}` : p.host ? `${displayName(host, p.host)}${host?.label || host?.name ? ` · ${p.host}` : ''}` : ''}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </main>
      {sel.length > 0 && <PortPanel key={sel.join(',')} sw={sw} ports={sw.ports.filter((p) => sel.includes(p.n))} switches={switches} vlans={vlans} addrs={addrs} editable={editable} onPatch={patch} onClose={() => setSel([])} />}
    </>
  );
}

function modeLabel(m: PortMode): string {
  return m === 'access' ? t('Access') : m === 'trunk' ? t('Trunk') : t('Disabled');
}

function PortPanel({
  sw,
  ports,
  switches,
  vlans,
  addrs,
  editable,
  onPatch,
  onClose,
}: {
  sw: Switch;
  ports: SwitchPort[];
  switches: Switch[];
  vlans: Vlan[] | null;
  addrs: Map<string, Address>;
  editable: boolean;
  onPatch: (ports: number[], set: Partial<Record<keyof SwitchPort, unknown>>) => Promise<void>;
  onClose: () => void;
}) {
  const p = ports[0];
  const one = ports.length === 1;
  const ns = ports.map((x) => x.n);
  const same = <T,>(f: (x: SwitchPort) => T) => (ports.every((x) => f(x) === f(p)) ? f(p) : undefined);
  const mode = same((x) => x.mode);
  const vlan = same((x) => x.vlan);
  const [peerSw, setPeerSw] = useState(p.peer?.id ?? '');
  const peer = switches.find((s) => s.id === peerSw);
  const host = p.host ? addrs.get(p.host) : undefined;
  const mm = one && p.host ? vlanMismatch(vlans, p, p.host) : null;

  const vlanOptions: DropOption[] = [{ value: '', label: '—' }, ...(vlans ?? []).map((v) => ({ value: String(v.id), label: `${v.id} · ${v.name}`, icon: <span className="vlan-sw" style={vlanStyle(v)} /> }))];
  const hostOptions: DropOption[] = [
    { value: '', label: '—' },
    ...[...addrs.values()]
      .filter((a) => a.up || a.label || a.ip === p.host)
      .sort((a, b) => byIp(a.ip, b.ip))
      .map((a) => {
        const I = KIND_ICON[a.kind ?? 'other'];
        return { value: a.ip, label: displayName(a, a.ip), detail: a.ip, icon: <I size={12} /> };
      }),
  ];
  if (p.host && !addrs.has(p.host)) hostOptions.push({ value: p.host, label: p.host });
  const swOptions: DropOption[] = [{ value: '', label: '—' }, ...switches.filter((s) => s.id !== sw.id).map((s) => ({ value: s.id, label: s.name, detail: s.ip }))];
  const peerPorts: DropOption[] = (peer?.ports ?? []).map((x) => ({ value: String(x.n), label: x.name || String(x.n), note: x.label || undefined }));
  const tagged = (id: number) => ports.every((x) => x.tagged.includes(id));

  return (
    <aside className="panel right">
      <div className="host-head">
        <span className="host-ico">
          <EthernetPort size={18} />
        </span>
        <div>
          <b>{one ? t('Port {n}', { n: p.n }) : tn(ports.length, '{n} port', '{n} ports')}</b>
          <span className="ip">{one ? p.name : ports.map((x) => x.n).join(', ')}</span>
        </div>
        <button className="icon-btn ghost" title={t('Close')} onClick={onClose}>
          <X size={14} />
        </button>
      </div>
      <section className="insp-section">
        <header>
          <span>{t('Port')}</span>
        </header>
        <div className="insp-body">
          {one && (
            <div className="prop-row">
              <span className="prop-label">{t('Label')}</span>
              <TextInput value={p.label} commitOnBlur disabled={!editable} onChange={(v) => v !== p.label && void onPatch(ns, { label: v })} />
            </div>
          )}
          <div className="prop-row">
            <span className="prop-label">{t('Mode')}</span>
            <Segmented
              value={(mode ?? '') as PortMode}
              onChange={(m) => editable && void onPatch(ns, { mode: m })}
              options={[
                { value: 'access', label: t('Access'), disabled: !editable },
                { value: 'trunk', label: t('Trunk'), disabled: !editable },
                { value: 'off', label: t('Disabled'), disabled: !editable },
              ]}
            />
          </div>
          {mode !== 'off' && (
            <div className="prop-row">
              <span className="prop-label">{mode === 'trunk' ? t('Native VLAN') : t('VLAN')}</span>
              <Dropdown value={vlan === undefined ? (ports.some((x) => x.vlan !== undefined) ? null : '') : String(vlan)} options={vlanOptions} disabled={!editable} onChange={(v) => void onPatch(ns, { vlan: v ? Number(v) : null })} />
            </div>
          )}
        </div>
      </section>
      {mode === 'trunk' && (
        <section className="insp-section">
          <header>
            <span>{t('Tagged VLANs')}</span>
          </header>
          <div className="insp-body">
            <div className="chip-pick">
              {(vlans ?? []).map((v) => {
                const on = tagged(v.id);
                return (
                  <button
                    key={v.id}
                    type="button"
                    className={`asr-chip ${on ? 'on' : ''}`}
                    disabled={!editable}
                    aria-pressed={on}
                    title={v.name}
                    onClick={() => {
                      for (const x of ports) {
                        const list = on ? x.tagged.filter((y) => y !== v.id) : [...new Set([...x.tagged, v.id])].sort((a, b) => a - b);
                        void onPatch([x.n], { tagged: list });
                      }
                    }}
                  >
                    <span className="vlan-sw" style={vlanStyle(v)} />
                    {v.id}
                  </button>
                );
              })}
            </div>
          </div>
        </section>
      )}
      {one && (
        <section className="insp-section">
          <header>
            <span>{t('Connected to')}</span>
          </header>
          <div className="insp-body">
            <div className="prop-row">
              <span className="prop-label">{t('Machine')}</span>
              <Dropdown value={p.host ?? ''} options={hostOptions} disabled={!editable} onChange={(v) => void onPatch(ns, { host: v || null })} />
              {mm && (
                <span className="badge warn" title={t('Outside the subnet of VLAN {id} ({subnet})', { id: mm.id, subnet: mm.subnet ?? '' })}>
                  {t('VLAN')}
                </span>
              )}
            </div>
            {host && host.ip !== displayName(host, host.ip) && (
              <div className="prop-row">
                <span className="prop-label">{t('IP')}</span>
                <span className="prop-val ip">{host.ip}</span>
              </div>
            )}
            <div className="prop-row">
              <span className="prop-label">{t('Switch')}</span>
              <Dropdown
                value={peerSw}
                options={swOptions}
                disabled={!editable}
                onChange={(v) => {
                  setPeerSw(v);
                  if (!v) void onPatch(ns, { peer: null });
                }}
              />
            </div>
            {peerSw && (
              <div className="prop-row">
                <span className="prop-label">{t('Its port')}</span>
                <Dropdown value={p.peer?.id === peerSw ? String(p.peer.port) : null} options={peerPorts} disabled={!editable} onChange={(v) => void onPatch(ns, { peer: { id: peerSw, port: Number(v) } })} />
              </div>
            )}
            <div className="prop-row">
              <span className="prop-label">{t('MACs seen')}</span>
              {p.learnt.length === 0 ? (
                <span className="prop-val">—</span>
              ) : (
                <div className="prop-val lldp-list">
                  {p.learnt.map((m) => (
                    <span key={m}>{m}</span>
                  ))}
                </div>
              )}
            </div>
            <div className="prop-row">
              <span className="prop-label">{t('LLDP')}</span>
              <span className="prop-val">{p.lldp ? `${p.lldp.name} · ${p.lldp.port}` : '—'}</span>
            </div>
          </div>
        </section>
      )}
    </aside>
  );
}

/* ---------- Créer / modifier un switch ---------- */

function SwitchModal({ sw, agents, groups, onClose, onSaved }: { sw: Switch | null; agents: SnmpAgent[] | null; groups: string[]; onClose: () => void; onSaved: (s: Switch | null) => void }) {
  const [name, setName] = useState(sw?.name ?? '');
  const [model, setModel] = useState(sw?.model ?? '');
  const [ip, setIp] = useState(sw?.ip ?? '');
  const [group, setGroup] = useState(sw?.group ?? groups[0] ?? '');
  const [copper, setCopper] = useState(String(sw?.copper ?? 24));
  const [sfp, setSfp] = useState(String(sw?.sfp ?? 4));
  const [naming, setNaming] = useState(sw?.naming ?? 'Gi1/0/{n}');
  const [sfpNaming, setSfpNaming] = useState(sw?.sfpNaming ?? 'Te1/1/{n}');
  const [snmp, setSnmp] = useState(sw?.snmp ?? '');
  const [busy, setBusy] = useState(false);
  const c = Number(copper);
  const f = Number(sfp);
  const badIp = ip.trim() !== '' && !validIp(ip.trim());
  const badC = !Number.isInteger(c) || c < 0 || c > 96;
  const badF = !Number.isInteger(f) || f < 0 || f > 32;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy || badIp || badC || badF) return;
    setBusy(true);
    const body = {
      ...(sw ?? { ports: [] }),
      name: name.trim(),
      model: model.trim(),
      ip: ip.trim() || undefined,
      group: group.trim(),
      copper: c,
      sfp: f,
      naming: naming.trim(),
      sfpNaming: sfpNaming.trim(),
      snmp: snmp || undefined,
    };
    try {
      onSaved(sw ? await api<Switch>('PUT', `/api/switches/${sw.id}`, body) : await api<Switch>('POST', '/api/switches', body));
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

  const agentOptions: DropOption[] = [{ value: '', label: '—' }, ...(agents ?? []).map((a) => ({ value: a.id, label: a.name, detail: a.ip }))];
  if (sw?.snmp && !agents?.some((a) => a.id === sw.snmp)) agentOptions.push({ value: sw.snmp, label: sw.snmp });
  return (
    <div className="modal-back" onPointerDown={onClose}>
      <form className="modal wide" role="dialog" aria-labelledby="sw-title" onPointerDown={(e) => e.stopPropagation()} onSubmit={submit}>
        <header>
          <h2 id="sw-title">{sw ? t('Edit {name}', { name: sw.name }) : t('New switch')}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="set-grid two">
          <Field label={t('Name')}>
            <input value={name} autoFocus={!sw} autoComplete="off" onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label={t('Model')}>
            <input value={model} autoComplete="off" onChange={(e) => setModel(e.target.value)} />
          </Field>
          <Field label={t('IP')}>
            <input className="mono" value={ip} placeholder={t('Optional')} aria-invalid={badIp} autoComplete="off" onChange={(e) => setIp(e.target.value)} />
          </Field>
          <Field label={t('Group')}>
            <input value={group} placeholder={t('Optional')} autoComplete="off" onChange={(e) => setGroup(e.target.value)} />
          </Field>
          <Field label={t('Copper ports')}>
            <input className="mono" value={copper} inputMode="numeric" aria-invalid={badC} autoComplete="off" onChange={(e) => setCopper(e.target.value)} />
          </Field>
          <Field label={t('SFP ports')}>
            <input className="mono" value={sfp} inputMode="numeric" aria-invalid={badF} autoComplete="off" onChange={(e) => setSfp(e.target.value)} />
          </Field>
          <Field label={t('Port naming')}>
            <input className="mono" value={naming} autoComplete="off" onChange={(e) => setNaming(e.target.value)} />
          </Field>
          <Field label={t('SFP naming')}>
            <input className="mono" value={sfpNaming} autoComplete="off" onChange={(e) => setSfpNaming(e.target.value)} />
          </Field>
          <div className="set-field">
            <span className="set-label">{t('SNMP agent')}</span>
            <Dropdown value={snmp} options={agentOptions} onChange={setSnmp} />
          </div>
        </div>
        <footer>
          {sw && (
            <button type="button" className="btn danger left" onClick={() => void remove()}>
              <Trash2 size={14} /> {t('Delete')}
            </button>
          )}
          <button type="button" className="btn" onClick={onClose}>
            {t('Cancel')}
          </button>
          <button type="submit" className="btn primary" disabled={!name.trim() || badIp || badC || badF || busy}>
            {sw ? <Save size={14} /> : <Plus size={14} />} {sw ? t('Save') : t('Create the switch')}
          </button>
        </footer>
      </form>
    </div>
  );
}
