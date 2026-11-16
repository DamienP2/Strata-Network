import { useEffect, useMemo, useState, type CSSProperties, type ReactNode } from 'react';
import {
  Activity,
  EthernetPort,
  EyeOff,
  LayoutGrid,
  List,
  Network as NetworkIcon,
  Radio,
  RefreshCw,
  Route,
  Search,
  Split,
  Timer,
  TriangleAlert,
  Users,
  Waves,
  type LucideIcon,
} from 'lucide-react';
import { t, tn } from '../i18n';
import { api, can, useApp } from '../store';
import { Check as Toggle, Segmented } from '../ui/fields';
import { Dropdown, type DropOption } from '../ui/Dropdown';
import { SubTabs } from './SidePage';
import { byIp } from '../net/ip';
import { fail, fmtBps, fmtSeen, useList, vlanById } from '../net/model';
import { TONES, toneLabel, toneOf, attachText, type Flow, type Flows, type Tone } from '../net/mcast';
import { FlowList, Mosaic } from './multicast/FlowViews';
import { FlowMap } from './multicast/FlowMap';
import { Plan } from './multicast/Plan';
import { Ptp } from './multicast/Ptp';
import { toneIcon } from './multicast/icons';
import type { MulticastState, Service, Vlan } from '../net/types';

type Tab = 'flows' | 'plan' | 'ptp' | 'services';
type Check = 'unannounced' | 'noReceivers' | 'pairMissing' | 'conflict' | 'flooded';
type Filter = 'all' | Tone | `vlan:${number}` | Check;
type Leg = 'a' | 'b' | 'both';
type View = 'map' | 'mosaic' | 'list';


/** Le débit estimé total, gardé entre deux visites de la page. */
const rateHistory: { at: number; v: number }[] = [];

function matches(f: Flow, filter: Filter): boolean {
  if (filter === 'all') return true;
  if (filter.startsWith('vlan:')) return f.vlan === Number(filter.slice(5));
  if ((TONES as string[]).includes(filter)) return toneOf(f) === filter;
  if (filter === 'conflict') return f.conflict;
  return f.issues.includes(filter as Check);
}

export function Multicast() {
  const user = useApp((s) => s.auth?.user);
  const [state] = useList<MulticastState>('multicast', '/api/multicast');
  const [data] = useList<Flows>('multicast', '/api/multicast/flows');
  const [vlans] = useList<Vlan[]>('vlans', '/api/vlans');
  const [services, reloadServices] = useList<{ at: number; services: Service[] }>('services', '/api/services');
  const [tab, setTab] = useState<Tab>('flows');
  const [filter, setFilter] = useState<Filter>('all');
  const [leg, setLeg] = useState<Leg>('a');
  const [view, setView] = useState<View>('map');
  const [q, setQ] = useState('');
  const [selected, setSelected] = useState<string | null>(null);

  useEffect(() => {
    if (!data) return;
    const last = rateHistory[rateHistory.length - 1];
    if (!last || last.at !== data.at) rateHistory.push({ at: data.at, v: data.summary.estBps });
    if (rateHistory.length > 60) rateHistory.splice(0, rateHistory.length - 60);
  }, [data]);

  const flows = useMemo(() => data?.flows ?? [], [data]);
  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return flows
      .filter((f) => matches(f, filter))
      .filter((f) => leg === 'both' || !f.leg || f.leg === leg)
      .filter((f) => {
        if (!needle) return true;
        const rx = f.receivers.map((r) => `${r.host ?? ''} ${r.switchName} ${r.port}`).join(' ');
        return `${f.group}:${f.port} ${f.name ?? ''} ${f.source ?? ''} ${f.emitter?.name ?? ''} ${attachText(f.emitter?.at)} ${rx}`.toLowerCase().includes(needle);
      })
      .sort((a, b) => Number(b.live) - Number(a.live) || byIp(a.group, b.group) || a.port - b.port);
  }, [flows, filter, leg, q]);

  const setIface = async (iface: string) => {
    if (!state) return;
    try {
      await api('PUT', '/api/multicast/settings', { iface, checkMaxMbps: state.checkMaxMbps });
    } catch (e) {
      fail(e);
    }
  };

  const setOption = async (o: { checkEnabled?: boolean; floodWatch?: boolean }) => {
    try {
      await api('PUT', '/api/multicast/options', o);
    } catch (e) {
      fail(e);
    }
  };

  if (!state || !data) return <div className="loading">{t('Loading…')}</div>;
  const nic = state.ifaces.find((i) => i.name === state.iface || i.ip === state.iface);
  const ifaceOptions: DropOption[] = state.ifaces.map((i) => ({ value: i.name, label: i.name, detail: i.ip }));
  const count = (fn: (f: Flow) => boolean) => flows.filter(fn).length;
  const vlanIds = [...new Set(flows.map((f) => f.vlan).filter((v): v is number => v !== undefined))].sort((a, b) => a - b);
  const checks: [Check, LucideIcon, string, 'warn' | 'bad' | undefined][] = [
    ['unannounced', EyeOff, t('Unannounced'), 'warn'],
    ['noReceivers', Users, t('Without receiver'), undefined],
    ['pairMissing', Split, t('Incomplete 2022-7 pair'), 'warn'],
    ['conflict', TriangleAlert, t('Conflicts'), 'bad'],
  ];
  const pick = (f: Filter) => {
    setFilter(f);
    setTab('flows');
  };
  const nav = (id: Filter, icon: ReactNode, label: string, n: number, dot?: 'warn' | 'bad') => (
    <div
      key={id}
      className={`lib-nav-item ${filter === id ? 'on' : ''}`}
      role="link"
      tabIndex={0}
      aria-current={filter === id ? 'page' : undefined}
      onClick={() => pick(id)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          pick(id);
        }
      }}
    >
      {icon}
      <span className="grow">{label}</span>
      {dot && n > 0 && <span className={`nav-dot ${dot}`} />}
      <span className="count">{n}</span>
    </div>
  );

  return (
    <div className="projects-page mc-page">
      <aside className="lib-nav">
        <div className="sys-group">
          <div className="lib-nav-section">
            <span className="grow">{t('Streams')}</span>
          </div>
          {nav('all', <Radio size={15} />, t('All the streams'), flows.length)}
          {TONES.map((k) => {
            const Icon = toneIcon(k);
            return nav(k, <Icon size={15} />, toneLabel(k), count((f) => toneOf(f) === k));
          })}
        </div>
        {vlanIds.length > 0 && (
          <div className="sys-group">
            <div className="lib-nav-section">
              <span className="grow">{t('Networks')}</span>
            </div>
            {vlanIds.map((id) => {
              const v = vlanById(vlans, id);
              return nav(
                `vlan:${id}`,
                <span className="vlan-sw" style={v ? ({ '--vlan': v.color } as CSSProperties) : undefined} />,
                v?.name ? `${id} · ${v.name}` : String(id),
                count((f) => f.vlan === id),
              );
            })}
          </div>
        )}
        <div className="sys-group">
          <div className="lib-nav-section">
            <span className="grow">{t('To check')}</span>
          </div>
          {checks.map(([id, Icon, label, dot]) => nav(id, <Icon size={15} />, label, count((f) => matches(f, id)), dot))}
        </div>
      </aside>
      <main className="library">
        <div className="library-inner full">
          <div className="settings-head">
            <div>
              <h1>{t('Multicast')}</h1>
            </div>
            <div className="spacer" />
            {can.admin(user) && (
              <>
                <Toggle checked={data.check.enabled} onChange={(v) => void setOption({ checkEnabled: v })} title={t('Joins the stream 2 s at most, refused above {n} Mb/s', { n: data.check.maxMbps })}>
                  {t('On-demand measure')}
                </Toggle>
                <Toggle checked={data.flood.enabled} onChange={(v) => void setOption({ floodWatch: v })} title={data.flood.error}>
                  {t('Flood detection')}
                </Toggle>
              </>
            )}
            {can.admin(user) && (
              <div style={{ width: 220 }}>
                <Dropdown value={nic?.name ?? null} options={ifaceOptions} title={t('Interface')} onChange={(v) => void setIface(v)} className="iface-dd" />
              </div>
            )}
          </div>
          <Kpis data={data} />
          <Problems data={data} vlans={vlans} onShow={(f) => pick(f)} />
          <SubTabs
            current={tab}
            onPick={(v) => setTab(v as Tab)}
            items={[
              { id: 'flows', icon: <Radio size={14} />, label: t('Streams'), count: flows.length },
              { id: 'plan', icon: <LayoutGrid size={14} />, label: t('Group plan') },
              { id: 'ptp', icon: <Timer size={14} />, label: t('PTP'), count: data.ptp.clocks.length + data.ptp.devices.length },
              { id: 'services', icon: <NetworkIcon size={14} />, label: t('Services'), count: services?.services.length },
            ]}
          >
            {tab === 'services' && can.edit(user) && <BrowseButton onDone={() => void reloadServices()} />}
          </SubTabs>
          {tab === 'flows' && (
            <>
              <div className="media-toolbar">
                <label className="search" style={{ width: 260 }}>
                  <Search size={14} />
                  <input value={q} placeholder={t('Group, name, emitter, port')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
                </label>
                <Segmented<Leg>
                  value={leg}
                  onChange={setLeg}
                  options={[
                    { value: 'a', label: t('Red') },
                    { value: 'b', label: t('Blue') },
                    { value: 'both', label: t('Both') },
                  ]}
                />
                <div className="spacer" />
                <Segmented<View>
                  value={view}
                  onChange={setView}
                  className="views"
                  options={[
                    { value: 'map', label: <><Route size={13} /> {t('Map')}</> },
                    { value: 'mosaic', label: <><LayoutGrid size={13} /> {t('Mosaic')}</> },
                    { value: 'list', label: <><List size={13} /> {t('List')}</> },
                  ]}
                />
              </div>
              {shown.length === 0 ? (
                <div className="empty big">
                  <Radio size={22} />
                  <p>{q || filter !== 'all' ? t('No match') : t('No stream')}</p>
                </div>
              ) : view === 'map' ? (
                <FlowMap data={data} flows={shown} selected={selected} onSelect={setSelected} />
              ) : view === 'mosaic' ? (
                <Mosaic flows={shown} onPick={(k) => { setSelected(k); setView('list'); }} />
              ) : (
                <FlowList flows={shown} data={data} open={selected} onOpen={setSelected} />
              )}
            </>
          )}
          {tab === 'plan' && <Plan data={data} vlans={vlans} />}
          {tab === 'ptp' && <Ptp data={data} />}
          {tab === 'services' && <Services data={services} />}
        </div>
      </main>
    </div>
  );
}

/* ---------- Chiffres ---------- */

function Stack({ parts }: { parts: [Tone, number][] }) {
  return (
    <div className="stack">
      {parts.filter(([, n]) => n > 0).map(([k, n]) => (
        <i key={k} className={`c-${k}`} style={{ flex: n }} title={`${toneLabel(k)} · ${n}`} />
      ))}
    </div>
  );
}

function Kpis({ data }: { data: Flows }) {
  const live = data.flows.filter((f) => f.live);
  const byTone = (list: Flow[], w: (f: Flow) => number) => TONES.map((k) => [k, list.filter((f) => toneOf(f) === k).reduce((s, f) => s + w(f), 0)] as [Tone, number]);
  const rxSwitches = new Set(data.flows.flatMap((f) => f.receivers.filter((r) => !r.router && !r.peer).map((r) => r.switchId))).size;
  const { pairs, pairsIncomplete } = data.summary;
  const circ = 2 * Math.PI * 15;
  const ok = pairs ? (pairs - pairsIncomplete) / pairs : 0;
  const gm = data.ptp.gm;
  const read = data.switches.filter((s) => s.read);
  const lastRead = Math.max(0, ...read.map((s) => s.at));
  const pts = rateHistory.map((p) => p.v);
  const max = Math.max(1, ...pts);
  const spark = pts.length > 1 ? pts.map((v, i) => `${((i / (pts.length - 1)) * 200).toFixed(1)},${(22 - (v / max) * 18).toFixed(1)}`).join(' ') : '';
  return (
    <div className="kpis">
      <div className="kpi">
        <span className="set-label">
          <Radio size={13} /> {t('Live streams')}
        </span>
        <span className="v">
          {live.length} <small>{t('of {n}', { n: data.flows.length })}</small>
        </span>
        <Stack parts={byTone(live, () => 1)} />
      </div>
      <div className="kpi">
        <span className="set-label">
          <Activity size={13} /> {t('Estimated rate')}
        </span>
        <span className="v">{fmtBps(data.summary.estBps)}</span>
        <svg className="sp" viewBox="0 0 200 24" preserveAspectRatio="none">
          {spark && <polyline fill="none" stroke="currentColor" strokeWidth="1.6" points={spark} />}
        </svg>
      </div>
      <div className="kpi">
        <span className="set-label">
          <Users size={13} /> {t('Receivers')}
        </span>
        <span className="v">
          {data.summary.receivers} <small>{`${tn(data.summary.receivers, 'port', 'ports')} · ${tn(rxSwitches, '{n} switch', '{n} switches')}`}</small>
        </span>
        <Stack parts={byTone(data.flows, (f) => f.receiverCount)} />
      </div>
      <div className="kpi">
        <span className="set-label">
          <Split size={13} /> {t('2022-7 pairs')}
        </span>
        <div className="ring">
          <svg width="38" height="38" viewBox="0 0 38 38">
            <circle className="bg" cx="19" cy="19" r="15" fill="none" strokeWidth="5" />
            {pairs > 0 && <circle className="fg" cx="19" cy="19" r="15" fill="none" strokeWidth="5" strokeDasharray={`${(ok * circ).toFixed(1)} ${circ.toFixed(1)}`} transform="rotate(-90 19 19)" strokeLinecap="round" />}
          </svg>
          <span className="v">
            {pairs - pairsIncomplete}
            <small>/ {pairs}</small>
          </span>
        </div>
      </div>
      <div className="kpi">
        <span className="set-label">
          <Timer size={13} /> {t('PTP')}
        </span>
        <span className="v name" title={gm?.gm}>
          {gm ? gm.name || gm.ip : '—'}
        </span>
        {gm && (
          <span className="muted">
            <span className={gm.live ? 'ok-t' : 'warn-t'}>●</span> {t('domain {d}', { d: gm.domain })} · {tn(data.ptp.devices.length, '{n} follower', '{n} followers')}
          </span>
        )}
      </div>
      <div className="kpi">
        <span className="set-label">
          <EthernetPort size={13} /> {t('Switches read')}
        </span>
        <span className={`v ${data.summary.switchesOnline && read.length === data.summary.switchesOnline ? 'ok-t' : read.length ? 'warn-t' : ''}`}>
          {read.length} <small>/ {data.summary.switchesOnline}</small>
        </span>
        {lastRead > 0 && <span className="muted">{t('IGMP read {ago}', { ago: fmtSeen(lastRead) })}</span>}
      </div>
    </div>
  );
}

/** Ce qui demande de l'attention : groupes émis deux fois, inondation. */
function Problems({ data, vlans, onShow }: { data: Flows; vlans: Vlan[] | null; onShow: (f: Filter) => void }) {
  const conflicts = new Map<string, string[]>();
  for (const f of data.flows) if (f.conflict && f.sources) conflicts.set(f.group, f.sources);
  const flood = data.flood.groups.length > 0 ? data.flood : null;
  if (!conflicts.size && !flood) return null;
  const parts: ReactNode[] = [];
  for (const [g, src] of [...conflicts].slice(0, 2)) {
    parts.push(
      <span key={g}>
        <b className="ip">{g}</b> {t('sent by {n} sources ({list})', { n: src.length, list: src.join(` ${t('and')} `) })}
      </span>,
    );
  }
  if (conflicts.size > 2) parts.push(<span key="more">{tn(conflicts.size - 2, '{n} other conflict', '{n} other conflicts')}</span>);
  if (flood) {
    const v = vlanById(vlans, flood.vlan);
    const where = flood.vlan !== undefined ? t('VLAN {n} on {iface}', { n: v ? `${v.id} ${v.name}` : flood.vlan, iface: flood.iface }) : flood.iface;
    parts.push(
      <span key="flood">
        {tn(flood.groups.length, '{where}: no IGMP snooping, {n} stream received without joining', '{where}: no IGMP snooping, {n} streams received without joining', { where })}
      </span>,
    );
  }
  return (
    <div className="diffs">
      <TriangleAlert size={16} />
      <span className="grow">
        {parts.map((p, i) => (
          <span key={i}>
            {i > 0 && ' · '}
            {p}
          </span>
        ))}
      </span>
      <button className="btn small" onClick={() => onShow(conflicts.size ? 'conflict' : 'flooded')}>
        {t('Show')}
      </button>
    </div>
  );
}

/* ---------- Services (mDNS) ---------- */

function BrowseButton({ onDone }: { onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const browse = async () => {
    setBusy(true);
    try {
      await api('POST', '/api/services/browse');
      setTimeout(() => {
        setBusy(false);
        onDone();
      }, 3500);
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  };
  return (
    <button className="btn" disabled={busy} onClick={() => void browse()}>
      <RefreshCw size={14} className={busy ? 'spin' : undefined} /> {t('Search')}
    </button>
  );
}

function Services({ data }: { data: { at: number; services: Service[] } | null }) {
  const [q, setQ] = useState('');
  const list = useMemo(() => data?.services ?? [], [data]);
  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return list
      .filter((s) => !needle || `${s.kind} ${s.name} ${s.host ?? ''} ${s.ip ?? ''}`.toLowerCase().includes(needle))
      .sort((a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name));
  }, [list, q]);
  if (!data) return <div className="loading">{t('Loading…')}</div>;
  return (
    <>
      <div className="media-toolbar">
        <label className="search">
          <Search size={14} />
          <input value={q} placeholder={t('Search a service')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
        </label>
      </div>
      {shown.length === 0 ? (
        <div className="empty big">
          <Waves size={22} />
          <p>{q ? t('No match') : t('No service')}</p>
        </div>
      ) : (
        <div className="card-table">
          <table className="data-table plain">
            <thead>
              <tr>
                <th>{t('Type')}</th>
                <th>{t('Name')}</th>
                <th>{t('Machine')}</th>
                <th>{t('IP')}</th>
                <th className="num">{t('Port')}</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((s) => (
                <tr key={`${s.kind}|${s.name}|${s.host}|${s.port}`} title={s.txt?.length ? s.txt.join('\n') : undefined}>
                  <td>
                    <span className="kind">{s.kind}</span>
                  </td>
                  <td>{s.name}</td>
                  <td>{s.host}</td>
                  <td className="ip">{s.ip ?? '—'}</td>
                  <td className="num ip">{s.port}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}
