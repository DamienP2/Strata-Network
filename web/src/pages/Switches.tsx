import { useCallback, useEffect, useMemo, useState, type FormEvent } from 'react';
import { Check, EthernetPort, Pencil, Plus, Save, Search, Trash2, TriangleAlert, X } from 'lucide-react';
import { locale, t, tc, tn } from '../i18n';
import { api, can, serverEvents, serverNow, useApp, useChanged } from '../store';
import { navigate } from '../router';
import { Field } from '../settings/ui';
import { Segmented } from '../ui/fields';
import { Dropdown, type DropOption } from '../ui/Dropdown';
import { confirmDialog } from '../ui/dialogs';
import { Empty } from '../ui/Empty';
import { PageHead } from './SidePage';
import { byIp, validIp } from '../net/ip';
import { KIND_ICON, displayName, fail, fmtSeen, useAllAddresses, useList, vlanById } from '../net/model';
import { InlineText, vlanStyle } from '../net/parts';
import { fmtDuration } from '../ui/format';
import type { Address, Live, LiveDiff, LivePort, Network, PortMode, SnmpAgent, Switch, SwitchPort, Vlan } from '../net/types';

type PortSet = Partial<Record<keyof SwitchPort, unknown>>;
/** Filtre : tous, un VLAN (`v:10`), les trunks, les libres ; en ligne : lien actif, sans lien, écarts, erreurs. */
type Filter = 'all' | 'trunk' | 'free' | `v:${number}` | 'up' | 'down' | 'diff' | 'errors';
const LIVE_FILTERS: Filter[] = ['up', 'down', 'diff', 'errors'];

export function Switches({ id }: { id?: string }) {
  const user = useApp((s) => s.auth?.user);
  const editable = can.edit(user);
  const [switches, , setSwitches] = useList<Switch[]>('switches', '/api/switches');
  const [vlans] = useList<Vlan[]>('vlans', '/api/vlans');
  const [networks] = useList<Network[]>('networks', '/api/networks');
  const [agents] = useList<SnmpAgent[]>('snmp', '/api/snmp');
  const addrs = useAllAddresses(networks);
  const [editing, setEditing] = useState<Switch | 'new' | null>(null);

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
  const sw = switches?.find((s) => s.id === id);
  const first = groups[0]?.[1][0]?.id;

  // Sans switch dans l'adresse (ou switch supprimé) : le premier de la liste.
  useEffect(() => {
    if (switches && !sw && first) navigate(`/switches/${first}`, true);
  }, [switches, sw, first]);

  const patch = async (s: Switch, ports: number[], set: PortSet) => {
    try {
      const saved = await api<Switch>('PATCH', `/api/switches/${s.id}/ports`, { ports, set });
      setSwitches((list) => (list ?? []).map((x) => (x.id === saved.id ? saved : x)));
    } catch (e) {
      fail(e);
    }
  };

  // Raccordé à : les machines de l'inventaire et les ports des autres switchs.
  const hostOptions = useMemo<DropOption[]>(
    () =>
      [...addrs.values()]
        .filter((a) => a.up || a.label || a.lastSeen)
        .sort((a, b) => byIp(a.ip, b.ip))
        .map((a) => {
          const I = KIND_ICON[a.kind ?? 'other'];
          const name = displayName(a, a.ip);
          return { value: `host:${a.ip}`, label: name, detail: name !== a.ip ? a.ip : undefined, sub: name !== a.ip ? a.ip : undefined, icon: <I size={12} />, group: t('Machines') };
        }),
    [addrs],
  );

  if (!switches) return <div className="loading">{t('Loading…')}</div>;
  const newButton = editable && (
    <button className="icon-btn ghost" style={{ width: 22, height: 22 }} title={t('New switch')} onClick={() => setEditing('new')}>
      <Plus size={13} />
    </button>
  );
  return (
    <>
      <div className="net-page two">
        <aside className="lib-nav">
          {groups.length === 0 && (
            <div className="lib-nav-section">
              <span className="grow">{t('Switches')}</span>
              {newButton}
            </div>
          )}
          {groups.map(([g, list], i) => (
            <div key={g} className="sys-group">
              <div className="lib-nav-section">
                <span className="grow">{g || t('Switches')}</span>
                {i === 0 && newButton}
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
                  {s.online && <span className="nav-dot ok" title={t('Online')} />}
                  <span className="count">{s.ports.length}</span>
                </div>
              ))}
            </div>
          ))}
        </aside>
        {sw ? (
          <SwitchView
            key={sw.id}
            sw={sw}
            switches={switches}
            vlans={vlans}
            addrs={addrs}
            agent={sw.snmp ? agents?.find((a) => a.id === sw.snmp) : undefined}
            hostOptions={hostOptions}
            editable={editable}
            onPatch={(ns, set) => patch(sw, ns, set)}
            onEdit={() => setEditing(sw)}
          />
        ) : (
          <main className="library">
            {switches.length === 0 && (
              <Empty icon={EthernetPort} text={t('No switch')}>
                {editable && (
                  <button className="btn primary" onClick={() => setEditing('new')}>
                    <Plus size={14} /> {t('New switch')}
                  </button>
                )}
              </Empty>
            )}
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
            navigate(s ? `/switches/${s.id}` : '/switches', !s);
          }}
        />
      )}
    </>
  );
}

/* ---------- Un switch : façade, légende, ports ---------- */

function SwitchView({
  sw,
  switches,
  vlans,
  addrs,
  agent,
  hostOptions,
  editable,
  onPatch,
  onEdit,
}: {
  sw: Switch;
  switches: Switch[];
  vlans: Vlan[] | null;
  addrs: Map<string, Address>;
  agent: SnmpAgent | undefined;
  hostOptions: DropOption[];
  editable: boolean;
  onPatch: (ns: number[], set: PortSet) => Promise<void>;
  onEdit: () => void;
}) {
  const [q, setQ] = useState('');
  const [filter, setFilter] = useState<Filter>('all');
  const [ticked, setTicked] = useState<Set<number>>(() => new Set());
  const [goto, setGoto] = useState<{ n: number; seq: number } | null>(null);
  const copper = sw.ports.filter((p) => p.n <= sw.copper);
  const sfp = sw.ports.filter((p) => p.n > sw.copper);
  const blocks: SwitchPort[][] = [];
  for (let i = 0; i < copper.length; i += 12) blocks.push(copper.slice(i, i + 12));

  const tick = (ns: number[], on: boolean) =>
    setTicked((old) => {
      const s = new Set(old);
      for (const n of ns) {
        if (on) s.add(n);
        else s.delete(n);
      }
      return s;
    });

  // Un port choisi sur la façade : sa ligne en vue.
  useEffect(() => {
    if (goto) document.getElementById(`port-${sw.id}-${goto.n}`)?.scrollIntoView({ block: 'nearest' });
  }, [goto, sw.id]);

  const pick = (n: number) => {
    if (editable) tick([n], !ticked.has(n));
    setGoto((g) => ({ n, seq: (g?.seq ?? 0) + 1 }));
  };

  // Les VLAN utilisés (accès, natif ou taggués) : une puce chacun.
  const used = useMemo(() => {
    const ids = new Set<number>();
    for (const p of sw.ports) {
      if (p.mode === 'off') continue;
      if (p.vlan !== undefined) ids.add(p.vlan);
      if (p.mode === 'trunk') for (const v of p.tagged) ids.add(v);
    }
    return [...ids].sort((a, b) => a - b);
  }, [sw.ports]);

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

  const needle = q.trim().toLowerCase();
  const matches = (p: SwitchPort) => {
    if (filter === 'trunk' && p.mode !== 'trunk') return false;
    if (filter === 'free' && (p.host || p.peer)) return false;
    if (filter.startsWith('v:')) {
      const v = Number(filter.slice(2));
      if (p.mode === 'off' || (p.vlan !== v && !(p.mode === 'trunk' && p.tagged.includes(v)))) return false;
    }
    if (needle) {
      const host = p.host ? addrs.get(p.host) : undefined;
      const peer = p.peer ? switches.find((x) => x.id === p.peer!.id) : undefined;
      const hay = `${p.n} ${p.name} ${p.label} ${p.host ?? ''} ${host?.label ?? ''} ${host?.name ?? ''} ${peer?.name ?? ''}`.toLowerCase();
      if (!hay.includes(needle)) return false;
    }
    return true;
  };
  const ports = filter !== 'all' || needle ? sw.ports.filter(matches) : sw.ports;

  const linked = (p: SwitchPort) => !!p.peer || (!!p.host && !!addrs.get(p.host)?.up) || p.learnt.length > 0 || !!p.lldp;
  const portTitle = (p: SwitchPort) => {
    const v = vlanById(vlans, p.vlan);
    const what = p.mode === 'off' ? t('disabled') : p.mode === 'trunk' ? t('trunk') : p.vlan !== undefined ? t('VLAN {id}', { id: p.vlan }) + (v ? ` ${v.name}` : '') : t('free');
    return [p.name || String(p.n), what, p.label].filter(Boolean).join(' · ');
  };
  const face = (p: SwitchPort, isSfp: boolean) => (
    <button
      key={p.n}
      type="button"
      className={`sw-port ${isSfp ? 'sfp' : ''} ${p.mode === 'trunk' ? 'trunk' : ''} ${p.mode === 'off' ? 'off' : ''} ${ticked.has(p.n) ? 'sel' : ''}`}
      style={p.mode !== 'off' ? vlanStyle(vlanById(vlans, p.vlan)) : undefined}
      title={portTitle(p)}
      onClick={() => pick(p.n)}
    >
      {p.n}
      {linked(p) && <span className="link" />}
    </button>
  );

  const sub = [sw.model, sw.ip, sw.group, sw.sfp ? t('{a} + {b} ports', { a: sw.copper, b: sw.sfp }) : tn(sw.copper, '{n} port', '{n} ports')].filter(Boolean);
  const vlanList: DropOption[] = (vlans ?? []).map((v) => ({ value: String(v.id), label: `${v.id} · ${v.name}`, icon: <span className="vlan-sw" style={vlanStyle(v)} /> }));
  const peerOptions: DropOption[] = switches
    .filter((x) => x.id !== sw.id)
    .flatMap((x) => x.ports.map((p) => ({ value: `peer:${x.id}:${p.n}`, label: `${x.name} · ${p.name || p.n}`, sub: p.label || undefined, icon: <EthernetPort size={12} />, group: t('Switches') })));
  const allTicked = ports.length > 0 && ports.every((p) => ticked.has(p.n));
  return (
    <main className="library">
      <div className="library-inner wide sw-page">
        <PageHead
          title={sw.name}
          sub={sub.map((s, i) => (
            <span key={i}>
              {i > 0 && ' · '}
              {s === sw.ip ? <span className="mono">{s}</span> : s}
            </span>
          ))}
        >
          {agent && (
            <span className={`badge ${agent.lastPoll && !agent.lastPoll.ok ? 'warn' : ''}`} title={agent.lastPoll?.message || agent.name}>
              {agent.lastPoll ? t('SNMP · read {when}', { when: fmtSeen(agent.lastPoll.at) }) : t('SNMP')}
            </span>
          )}
          {editable && (
            <button type="button" className="btn" onClick={onEdit}>
              <Pencil size={14} /> {t('Edit')}
            </button>
          )}
        </PageHead>
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
        <div className="media-toolbar">
          <label className="search">
            <Search size={14} />
            <input value={q} placeholder={t('Port, label, machine')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
          </label>
          <div className="asr-chips">
            <button className={`asr-chip ${filter === 'all' ? 'on' : ''}`} onClick={() => setFilter('all')}>
              {t('All ports')}
            </button>
            {used.map((v) => {
              const x = vlanById(vlans, v);
              return (
                <button key={v} className={`asr-chip ${filter === `v:${v}` ? 'on' : ''}`} title={x?.subnet} onClick={() => setFilter(`v:${v}`)}>
                  <span className="vlan-sw" style={vlanStyle(x)} />
                  {x ? `${v} ${x.name}` : v}
                </button>
              );
            })}
            <button className={`asr-chip ${filter === 'trunk' ? 'on' : ''}`} onClick={() => setFilter('trunk')}>
              {t('Trunks')}
            </button>
            <button className={`asr-chip ${filter === 'free' ? 'on' : ''}`} onClick={() => setFilter('free')}>
              {tc('ports', 'Free')}
            </button>
          </div>
        </div>
        {ports.length === 0 ? (
          <div className="empty">{t('No match')}</div>
        ) : (
          <table className="pt">
            <colgroup>
              {editable && <col style={{ width: 36 }} />}
              <col style={{ width: 120 }} />
              <col style={{ width: '16%' }} />
              <col style={{ width: 156 }} />
              <col style={{ width: '26%' }} />
              <col style={{ width: '20%' }} />
              <col />
            </colgroup>
            <thead>
              <tr>
                {editable && (
                  <th>
                    <input type="checkbox" className="tick" checked={allTicked} title={t('Tick all')} onChange={(e) => tick(ports.map((p) => p.n), e.target.checked)} />
                  </th>
                )}
                <th>{t('Port')}</th>
                <th>{t('Label')}</th>
                <th>{t('Mode')}</th>
                <th>{t('VLAN')}</th>
                <th>{t('Connected to')}</th>
                <th>{t('MAC / LLDP')}</th>
              </tr>
            </thead>
            <tbody>
              {ports.map((p) => (
                <PortRow
                  key={p.n}
                  sw={sw}
                  p={p}
                  switches={switches}
                  vlans={vlans}
                  addrs={addrs}
                  vlanList={vlanList}
                  hostOptions={hostOptions}
                  peerOptions={peerOptions}
                  editable={editable}
                  ticked={ticked.has(p.n)}
                  onTick={(on) => tick([p.n], on)}
                  onPatch={(set) => onPatch([p.n], set)}
                />
              ))}
            </tbody>
          </table>
        )}
        {editable && ticked.size > 0 && <BulkBar sw={sw} vlans={vlans} vlanList={vlanList} ticked={ticked} onClear={() => setTicked(new Set())} onPatch={onPatch} />}
      </div>
    </main>
  );
}

function modeLabel(m: PortMode): string {
  return m === 'access' ? t('Access') : m === 'trunk' ? t('Trunk') : tc('mode', 'Off');
}

const modeOptions = () => [
  { value: 'access' as PortMode, label: t('Access') },
  { value: 'trunk' as PortMode, label: t('Trunk') },
  { value: 'off' as PortMode, label: tc('mode', 'Off') },
];

/** Une pastille de VLAN taggué ; la croix l'enlève. */
function TaggedTag({ id, vlans, onRemove }: { id: number; vlans: Vlan[] | null; onRemove?: () => void }) {
  const v = vlanById(vlans, id);
  return (
    <span className="vlan-tag" style={vlanStyle(v)} title={v?.name}>
      <span className="vlan-sw" />
      <b>{id}</b>
      {v?.name ? ` ${v.name}` : null}
      {onRemove && (
        <button type="button" className="tag-x" title={t('Remove')} onClick={onRemove}>
          <X size={10} />
        </button>
      )}
    </span>
  );
}

function PortRow({
  sw,
  p,
  switches,
  vlans,
  addrs,
  vlanList,
  hostOptions,
  peerOptions,
  editable,
  ticked,
  onTick,
  onPatch,
}: {
  sw: Switch;
  p: SwitchPort;
  switches: Switch[];
  vlans: Vlan[] | null;
  addrs: Map<string, Address>;
  vlanList: DropOption[];
  hostOptions: DropOption[];
  peerOptions: DropOption[];
  editable: boolean;
  ticked: boolean;
  onTick: (on: boolean) => void;
  onPatch: (set: PortSet) => Promise<void>;
}) {
  const vlan = vlanById(vlans, p.vlan);
  const trunk = p.mode === 'trunk';
  const peerSw = p.peer ? switches.find((x) => x.id === p.peer!.id) : undefined;
  const host = p.host ? addrs.get(p.host) : undefined;
  const linkValue = p.peer ? `peer:${p.peer.id}:${p.peer.port}` : p.host ? `host:${p.host}` : null;
  const linkText = p.peer
    ? `${peerSw?.name ?? p.peer.id} · ${peerSw?.ports.find((x) => x.n === p.peer!.port)?.name || p.peer.port}`
    : p.host
      ? `${displayName(host, p.host)}${host?.label || host?.name ? ` · ${p.host}` : ''}`
      : '';

  let links: DropOption[] = [];
  if (editable) {
    links = [{ value: '', label: '—' }, ...hostOptions, ...peerOptions];
    if (p.host && !hostOptions.some((o) => o.value === linkValue)) links.splice(1, 0, { value: `host:${p.host}`, label: p.host, group: t('Machines') });
  }
  const link = (v: string) => {
    if (!v) return void onPatch({ host: null, peer: null });
    const [kind, a, b] = v.split(':');
    if (kind === 'host') void onPatch({ host: a, peer: null });
    else void onPatch({ peer: { id: a, port: Number(b) }, host: null });
  };
  const setVlan = (v: string) => void onPatch({ vlan: v ? Number(v) : null });

  // Accès : le VLAN. Trunk : les VLAN taggués, puis le VLAN natif s'il y en a un.
  let vlanCell: React.ReactNode = null;
  if (p.mode === 'access') {
    vlanCell = editable ? (
      <Dropdown className="cell" value={p.vlan === undefined ? '' : String(p.vlan)} options={[{ value: '', label: '—' }, ...vlanList]} onChange={setVlan} />
    ) : (
      p.vlan !== undefined && (
        <span className="cell-kind">
          <span className="vlan-sw" style={vlanStyle(vlan)} />
          {vlan ? `${p.vlan} · ${vlan.name}` : p.vlan}
        </span>
      )
    );
  } else if (trunk) {
    const addable = vlanList.filter((o) => !p.tagged.includes(Number(o.value)));
    const nativeFace =
      p.vlan !== undefined ? (
        <>
          <span className="vlan-sw" style={vlanStyle(vlan)} />
          {tc('vlan', 'native')} <b>{p.vlan}</b>
        </>
      ) : (
        tc('vlan', 'native')
      );
    vlanCell = (
      <div className="tagged">
        {p.tagged.map((id) => (
          <TaggedTag key={id} id={id} vlans={vlans} onRemove={editable ? () => void onPatch({ tagged: p.tagged.filter((x) => x !== id) }) : undefined} />
        ))}
        {editable && addable.length > 0 && (
          <Dropdown className="add" value={null} placeholder="+" title={t('Add a VLAN')} options={addable} onChange={(v) => void onPatch({ tagged: [...p.tagged, Number(v)].sort((a, b) => a - b) })} />
        )}
        {editable ? (
          <Dropdown
            className={`native ${p.vlan === undefined ? 'unset' : ''}`}
            value={p.vlan === undefined ? '' : String(p.vlan)}
            title={vlan ? `${t('Native VLAN')} · ${vlan.name}` : t('Native VLAN')}
            options={[{ value: '', label: t('None') }, ...vlanList]}
            face={nativeFace}
            onChange={setVlan}
          />
        ) : (
          p.vlan !== undefined && (
            <span className="native-tag" title={vlan ? `${t('Native VLAN')} · ${vlan.name}` : t('Native VLAN')}>
              {nativeFace}
            </span>
          )
        )}
      </div>
    );
  }

  const learnt = p.lldp ? t('LLDP: {name} {port}', { name: p.lldp.name, port: p.lldp.port }) : p.learnt.length ? p.learnt[0] + (p.learnt.length > 1 ? ` +${p.learnt.length - 1}` : '') : '';
  return (
    <tr id={`port-${sw.id}-${p.n}`} className={`${ticked ? 'sel' : ''} ${p.mode === 'off' ? 'off' : ''} ${trunk ? 'trunk' : ''}`} style={p.mode !== 'off' ? vlanStyle(vlan) : undefined}>
      {editable && (
        <td>
          <input type="checkbox" className="tick" checked={ticked} aria-label={p.name || String(p.n)} onChange={(e) => onTick(e.target.checked)} />
        </td>
      )}
      <td>
        <b className="ip">{p.n}</b> <span className="ip muted2">{p.name}</span>
      </td>
      <td>
        <InlineText value={p.label} placeholder={t('Label')} editable={editable} onSave={(label) => void onPatch({ label })} />
      </td>
      <td>{editable ? <Segmented value={p.mode} options={modeOptions()} onChange={(mode) => void onPatch({ mode })} /> : modeLabel(p.mode)}</td>
      <td>{vlanCell}</td>
      <td>
        {editable ? (
          (linkValue || p.mode !== 'off') && <Dropdown className="cell" value={linkValue} placeholder={t('Connect…')} options={links} onChange={link} />
        ) : (
          <span className="cell-text">{linkText}</span>
        )}
      </td>
      <td>
        <span className="learnt" title={p.learnt.join('\n') || undefined}>
          {learnt}
        </span>
      </td>
    </tr>
  );
}

/* ---------- Ports cochés : une barre pour tous les changer ---------- */

function BulkBar({
  sw,
  vlans,
  vlanList,
  ticked,
  onClear,
  onPatch,
}: {
  sw: Switch;
  vlans: Vlan[] | null;
  vlanList: DropOption[];
  ticked: Set<number>;
  onClear: () => void;
  onPatch: (ports: number[], set: PortSet) => Promise<void>;
}) {
  const [mode, setMode] = useState<PortMode | null>(null);
  const [vlan, setVlan] = useState<string | null>(null);
  const [tagged, setTagged] = useState<number[]>([]);
  const [busy, setBusy] = useState(false);
  const ports = sw.ports.filter((p) => ticked.has(p.n));
  const names = ports.map((p) => p.name || String(p.n)).join(', ');
  const trunk = mode === 'trunk';
  const ready = mode !== null || vlan !== null;

  const apply = async () => {
    const set: PortSet = {};
    if (mode) set.mode = mode;
    if (trunk) {
      if (tagged.length) set.tagged = tagged;
    } else if (vlan !== null) set.vlan = Number(vlan);
    setBusy(true);
    await onPatch(
      ports.map((p) => p.n),
      set,
    );
    setBusy(false);
    setMode(null);
    setVlan(null);
    setTagged([]);
    onClear();
  };

  const addable = vlanList.filter((o) => !tagged.includes(Number(o.value)));
  return (
    <div className="bulk">
      <b>{tn(ticked.size, '{n} ticked port', '{n} ticked ports')}</b>
      <span className="muted bulk-names" title={names}>
        {names}
      </span>
      <div className="spacer" />
      <span className="muted">{t('Mode')}</span>
      <Segmented value={mode ?? ('' as PortMode)} options={modeOptions()} onChange={setMode} />
      <span className="muted">{trunk ? t('Tagged VLANs') : t('VLAN')}</span>
      {trunk ? (
        <div className="tagged bulk-tagged">
          {tagged.map((id) => (
            <TaggedTag key={id} id={id} vlans={vlans} onRemove={() => setTagged(tagged.filter((x) => x !== id))} />
          ))}
          {addable.length > 0 && <Dropdown className="add" value={null} placeholder="+" title={t('Add a VLAN')} options={addable} onChange={(v) => setTagged([...tagged, Number(v)].sort((a, b) => a - b))} />}
        </div>
      ) : (
        <Dropdown value={vlan} options={vlanList} onChange={setVlan} />
      )}
      <button type="button" className="btn ghost" onClick={onClear}>
        {t('Untick')}
      </button>
      <button type="button" className="btn primary" disabled={busy || !ready} onClick={() => void apply()}>
        <Check size={14} /> {t('Apply')}
      </button>
    </div>
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
