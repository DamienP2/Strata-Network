import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Activity as ActivityIcon, EthernetPort, Monitor, Timer, Users } from 'lucide-react';
import { t, tc, tn } from '../../i18n';
import { attachText, shortPort, type Flows, type PtpPlace, type PtpTreeNode } from '../../net/mcast';
import { Activity, CorrSpark, Intervals, PortStateBadge, clockTypeLabel, fmtCorr, fmtNs, fmtRate, timeSourceLabel } from '../../net/ptp';
import { fmtDateTime, fmtTime } from '../../ui/format';

/** Au-delà, les appareils sont regroupés par switch. */
const MAX_DEVICES = 18;

type Ptp = Flows['ptp'];

interface TreeNode {
  id: string;
  x: number;
  y: number;
  label: string;
  sub: string;
  cls: string;
  icon: 'gm' | 'sw' | 'dev' | 'group';
  title?: string;
}

const curve = (a: [number, number], b: [number, number]) => {
  const dx = (b[0] - a[0]) / 2;
  return `M${a[0].toFixed(1)},${a[1].toFixed(1)} C${(a[0] + dx).toFixed(1)},${a[1].toFixed(1)} ${(b[0] - dx).toFixed(1)},${b[1].toFixed(1)} ${b[0].toFixed(1)},${b[1].toFixed(1)}`;
};

const placeText = (p: PtpPlace | undefined) => (p ? (p.port ? `${p.switchName} · ${shortPort(p.port)}` : p.switchName) : '');

/** Les lignes du tableau : le grandmaster, les ports maîtres lus sur les switchs, ce serveur, ce qui est entendu. */
interface Row {
  key: string;
  device: string;
  title?: string;
  place: string;
  state?: React.ReactNode;
  domain?: number;
  intervals: React.ReactNode;
  activity: React.ReactNode;
  sources: ('SNMP' | 'PTP')[];
  off?: boolean;
}

function rows(p: Ptp): Row[] {
  const out: Row[] = [];
  const g = p.grandmaster;
  if (g)
    out.push({
      key: 'gm',
      device: g.name ?? g.identity,
      title: g.identity,
      place: placeText(g.at),
      state: g.portState ? <PortStateBadge state={g.portState} /> : undefined,
      domain: g.domain,
      intervals: <span className="ivl">{t('GM')}</span>,
      activity: <span className="muted2">—</span>,
      sources: g.portState ? ['SNMP', 'PTP'] : ['PTP'],
      off: !g.live,
    });
  for (const r of p.ports)
    out.push({
      key: `p:${r.switchId}:${r.number}`,
      device: r.strata ? t('Strata (this server)') : (r.device ?? '—'),
      title: r.deviceIp,
      place: `${r.switchName} · ${shortPort(r.port)}`,
      state: <PortStateBadge state={r.state} />,
      domain: r.domain,
      intervals: <Intervals ann={r.logAnnounce} sync={r.logSync} dreq={r.logDelayReq} />,
      activity: r.strata ? <span className="muted2">{t('listening only')}</span> : <Activity activity={r.activity} rate={r.rxPs} since={r.muteSince} words="rate" />,
      sources: r.strata ? ['SNMP', 'PTP'] : ['SNMP'],
    });
  const tk = p.talker;
  if (tk && !p.ports.some((r) => r.strata))
    out.push({
      key: 'strata',
      device: t('Strata (this server)'),
      title: tk.clock,
      place: placeText(tk.at),
      state: tk.state ? <PortStateBadge state={tk.state} /> : undefined,
      domain: tk.domain,
      intervals: <Intervals ann={tk.logAnnounce} sync={tk.logSync} dreq={tk.logDelayReq} />,
      activity: <span className="muted2">{t('listening only')}</span>,
      sources: ['PTP'],
      off: !tk.live,
    });
  // Les autres horloges entendues (secours, autre domaine).
  for (const c of p.clocks.filter((c) => !(tk && c.ip === tk.ip && c.clock === tk.clock && c.domain === tk.domain)))
    out.push({
      key: `c:${c.ip}:${c.domain}:${c.clock}`,
      device: c.name ?? c.ip,
      title: c.clock,
      place: attachText(c.at),
      state: <PortStateBadge state="master" />,
      domain: c.domain,
      intervals: <Intervals ann={c.logAnnounce} sync={c.logSync} dreq={c.logDelayReq} />,
      activity: <span className="muted2">—</span>,
      sources: ['PTP'],
      off: !c.live,
    });
  // Les appareils dont les Delay_Req arrivent ici.
  for (const d of p.devices) {
    const secs = (d.lastSeen - d.firstSeen) / 1000;
    out.push({
      key: `d:${d.clock}:${d.domain}`,
      device: d.name ?? d.ip,
      title: d.clock,
      place: attachText(d.at),
      domain: d.domain,
      intervals: <span className="ivl">—</span>,
      activity: d.live ? (
        <span className="alive">
          <i />
          {secs > 0 && d.count > 1 ? t('Delay_Req {n}/s', { n: fmtRate((d.count - 1) / secs) }) : t('follower active')}
        </span>
      ) : (
        <span className="alive w">
          <i />
          {t('silent since {time}', { time: fmtTime(d.lastSeen) })}
        </span>
      ),
      sources: ['PTP'],
      off: !d.live,
    });
  }
  return out;
}

/** Le nombre de lignes de la vue PTP (l'onglet l'affiche). */
export const ptpCount = (p: Ptp) => rows(p).length;

function activityWord(a: PtpTreeNode['activity']): string {
  return a === 'active' ? t('active') : a === 'mute' ? t('silent') : a === 'none' ? t('no request') : '';
}

/** Place les nœuds : une colonne par profondeur (les appareils dans la dernière), chaque parent au milieu de ses enfants. */
function layout(tree: PtpTreeNode[], grouped: boolean): { nodes: TreeNode[]; links: [string, string, string][]; rows: number } {
  const kids = new Map<string, PtpTreeNode[]>();
  for (const n of tree) if (n.parent) kids.set(n.parent, [...(kids.get(n.parent) ?? []), n]);
  for (const list of kids.values()) list.sort((a, b) => Number(a.kind === 'device') - Number(b.kind === 'device') || a.name.localeCompare(b.name));
  // Regroupés : un nœud par switch au lieu de ses appareils.
  const shown: PtpTreeNode[] = [];
  const children = new Map<string, PtpTreeNode[]>();
  for (const n of tree) {
    const list = kids.get(n.id) ?? [];
    const devs = list.filter((x) => x.kind === 'device');
    if (grouped && devs.length) {
      const bad = devs.filter((d) => d.link === 'bad').length;
      const grp: PtpTreeNode = { id: `g:${n.id}`, kind: 'device', name: tn(devs.length, '{n} device', '{n} devices'), parent: n.id, link: bad ? 'bad' : devs.every((d) => d.link === 'idle') ? 'idle' : 'ok' };
      children.set(n.id, [...list.filter((x) => x.kind !== 'device'), grp]);
      shown.push(grp);
    } else children.set(n.id, list);
  }
  const all = [...tree.filter((n) => !(grouped && n.kind === 'device')), ...shown];
  const depth = new Map<string, number>();
  const byId = new Map(all.map((n) => [n.id, n]));
  const depthOf = (n: PtpTreeNode, guard = 0): number => {
    if (depth.has(n.id)) return depth.get(n.id)!;
    const p = n.parent ? byId.get(n.parent) : undefined;
    const d = p && guard < 64 ? depthOf(p, guard + 1) + 1 : 0;
    depth.set(n.id, d);
    return d;
  };
  for (const n of all) depthOf(n);
  const maxInner = Math.max(0, ...all.filter((n) => n.kind !== 'device').map((n) => depth.get(n.id)!));
  const hasDevices = all.some((n) => n.kind === 'device');
  const cols = maxInner + (hasDevices ? 1 : 0);
  const col = (n: PtpTreeNode) => (n.kind === 'device' ? cols : depth.get(n.id)!);
  const x = (c: number) => (cols === 0 ? 50 : 11 + (c * 72) / cols);
  // Les feuilles prennent une place chacune, de haut en bas ; un parent est au milieu des siens.
  const slot = new Map<string, number>();
  let leaves = 0;
  const place = (n: PtpTreeNode, guard = 0): number => {
    const list = guard < 64 ? (children.get(n.id) ?? []) : [];
    const ys = list.map((c) => place(c, guard + 1));
    const y = ys.length ? (Math.min(...ys) + Math.max(...ys)) / 2 : leaves++;
    slot.set(n.id, y);
    return y;
  };
  for (const r of all.filter((n) => !n.parent || !byId.has(n.parent))) place(r);
  const total = Math.max(leaves, 1);
  const y = (s: number) => 8 + ((s + 0.5) * 84) / total;
  const nodes: TreeNode[] = [];
  const links: [string, string, string][] = [];
  for (const n of all) {
    const s = slot.get(n.id);
    if (s === undefined) continue;
    let sub = '';
    let cls = '';
    let icon: TreeNode['icon'] = 'dev';
    if (n.kind === 'gm') {
      sub = t('Grandmaster · class {c} · {src}', { c: n.clockClass ?? '—', src: timeSourceLabel(n.timeSource) });
      cls = 'gmnode';
      icon = 'gm';
    } else if (n.kind === 'switch' || n.kind === 'clock') {
      sub = n.port ? [t('slave {port}', { port: shortPort(n.port) }), n.offsetNs !== undefined ? `±${fmtNs(Math.abs(n.offsetNs))}` : ''].filter(Boolean).join(' · ') : clockTypeLabel(n.clockType ?? 'boundary');
      cls = 'sw';
      icon = 'sw';
    } else if (n.id.startsWith('g:')) {
      icon = 'group';
      cls = n.link === 'bad' ? 'warnb' : '';
    } else {
      sub = [n.port ? shortPort(n.port) : '', activityWord(n.activity)].filter(Boolean).join(' · ');
      cls = n.link === 'bad' ? 'warnb' : '';
    }
    nodes.push({ id: n.id, x: x(col(n)), y: y(s), label: n.name, sub, cls, icon });
    if (n.parent && byId.has(n.parent)) links.push([n.parent, n.id, n.link === 'ok' ? '' : n.link === 'bad' ? 'bad' : 'bk']);
  }
  return { nodes, links, rows: total };
}

export function Ptp({ data }: { data: Flows }) {
  const p = data.ptp;
  const box = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<[number, number]>([0, 0]);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize([el.clientWidth, el.clientHeight]));
    ro.observe(el);
    setSize([el.clientWidth, el.clientHeight]);
    return () => ro.disconnect();
  }, [p.tree.length]);

  const { nodes, links, height } = useMemo(() => {
    const grouped = p.tree.filter((n) => n.kind === 'device').length > MAX_DEVICES;
    const l = layout(p.tree, grouped);
    return { ...l, height: Math.max(300, l.rows * 54 + 40) };
  }, [p.tree]);
  const list = useMemo(() => rows(p), [p]);

  const at = (id: string): [number, number] | null => {
    const n = nodes.find((x) => x.id === id);
    return n ? [(n.x / 100) * size[0], (n.y / 100) * size[1]] : null;
  };

  if (!list.length && !nodes.length) return <div className="empty">{t('No PTP message heard')}</div>;
  const g = p.grandmaster;
  const tk = p.talker;
  const last = p.changes[0];
  const via = g && g.path.length ? `${g.path[0].switchName} ${shortPort(g.path[0].port)}${g.path.length > 1 ? ` (${t('via {list}', { list: g.path.slice(1).map((x) => x.switchName).join(' · ') })})` : ''}` : placeText(g?.at);
  const cards = [g, tk, p.switchSync.length ? 1 : null].filter(Boolean).length;
  return (
    <>
      {cards > 0 && (
        <div className="gmcards" style={{ gridTemplateColumns: cards === 3 ? undefined : `repeat(${cards}, minmax(0, 1fr))` }}>
          {g && (
            <div className="hcard" title={last ? t('Last change: {date}', { date: fmtDateTime(last.at) }) : undefined}>
              <h3>
                <Timer size={14} /> {t('Grandmaster')}
              </h3>
              <div className="gm">
                <b>{g.name ?? g.identity}</b>
                <span className={`badge-gm ${g.live ? '' : 'bk'}`}>{g.live ? tc('gm', 'Active') : tc('gm', 'Silent')}</span>
              </div>
              <span className="bigid muted2">{g.identity}</span>
              <div className="hrow">
                <span className="grow">{t('Domain · priorities')}</span>
                <span className="mono">{`${g.domain} · ${g.priority1 ?? '—'} / ${g.priority2 ?? '—'}`}</span>
              </div>
              <div className="hrow">
                <span className="grow">{t('Class · accuracy · source')}</span>
                <span className="mono">{`${g.clockClass ?? '—'} · ${g.accuracyNs !== undefined ? fmtNs(g.accuracyNs) : '—'} · ${timeSourceLabel(g.timeSource)}`}</span>
              </div>
              {via && (
                <div className="hrow">
                  <span className="grow">{t('Connection')}</span>
                  <span className="mono">{via}</span>
                </div>
              )}
            </div>
          )}
          {tk && (
            <div className="hcard">
              <h3>
                <EthernetPort size={14} /> {t('Clock that talks to Strata')}
              </h3>
              <b>{tk.at ? placeText(tk.at) : (tk.name ?? tk.ip)}</b>
              <span className="bigid muted2">{tk.clock}</span>
              <div className="hrow">
                <span className="grow">{t('Role')}</span>
                <span className="mono">{tk.role === 'gm' ? t('Grandmaster') : t('Boundary clock')}</span>
              </div>
              <div className="hrow">
                <span className="grow">{t('Steps from the GM')}</span>
                <span className="mono">{tk.stepsRemoved}</span>
              </div>
              <div className="hrow">
                <span className="grow">{t('Address')}</span>
                <span className="mono">{tk.ip}</span>
              </div>
            </div>
          )}
          {p.switchSync.length > 0 && (
            <div className="hcard">
              <h3>
                <ActivityIcon size={14} /> {t('Switch synchronisation')}
              </h3>
              {p.switchSync.map((s) => (
                <div key={s.switchId} className="hrow">
                  <span className="grow">{s.slave ? `${s.switchName} · ${t('slave {port}', { port: shortPort(s.slave.port) })}` : s.switchName}</span>
                  <span className="corr mono">
                    <CorrSpark values={s.history} />
                    {fmtCorr(s.history.length ? s.history : s.offsetNs !== undefined ? [s.offsetNs] : [])}
                  </span>
                </div>
              ))}
              {p.switchSync.some((s) => s.meanPathDelayNs !== undefined) && (
                <div className="hrow">
                  <span className="grow">{t('Mean delay to the master')}</span>
                  <span className="mono">{p.switchSync.filter((s) => s.meanPathDelayNs !== undefined).map((s) => fmtNs(s.meanPathDelayNs!)).join(' · ')}</span>
                </div>
              )}
            </div>
          )}
        </div>
      )}
      {nodes.length > 0 && (
        <div className="ptree" ref={box} style={{ height }}>
          <svg>
            {links.map(([a, b, cls], i) => {
              const pa = at(a);
              const pb = at(b);
              return pa && pb ? <path key={i} className={cls} d={curve(pa, pb)} /> : null;
            })}
          </svg>
          {nodes.map((n) => (
            <div key={n.id} className={`mn ${n.cls}`} style={{ left: `${n.x}%`, top: `${n.y}%` }} title={n.title}>
              <span className="ic">{n.icon === 'gm' ? <Timer size={14} /> : n.icon === 'sw' ? <EthernetPort size={14} /> : n.icon === 'group' ? <Users size={14} /> : <Monitor size={14} />}</span>
              <span>
                <b>{n.label}</b>
                {n.sub && <small>{n.sub}</small>}
              </span>
            </div>
          ))}
        </div>
      )}
      <table className="pt">
        <colgroup>
          <col style={{ width: '18%' }} />
          <col style={{ width: '16%' }} />
          <col style={{ width: 110 }} />
          <col style={{ width: 80 }} />
          <col style={{ width: 230 }} />
          <col />
          <col style={{ width: 150 }} />
        </colgroup>
        <thead>
          <tr>
            <th>{t('Device')}</th>
            <th>{t('Switch · port')}</th>
            <th>{t('Port state')}</th>
            <th className="num">{t('Domain')}</th>
            <th>{t('Announce · Sync · Delay_Req')}</th>
            <th>{t('Follower activity')}</th>
            <th>{t('Source')}</th>
          </tr>
        </thead>
        <tbody>
          {list.map((r) => (
            <tr key={r.key} className={r.off ? 'off' : ''}>
              <td title={r.title}>{r.device}</td>
              <td>{r.place || <span className="muted2">—</span>}</td>
              <td>{r.state ?? <span className="muted2">—</span>}</td>
              <td className={`num ip ${g && r.domain !== undefined && r.domain !== g.domain ? 'warn-t' : ''}`}>{r.domain ?? '—'}</td>
              <td>{r.intervals}</td>
              <td>{r.activity}</td>
              <td>
                {r.sources.map((s) => (
                  <span key={s} className="srcb">
                    {s}
                  </span>
                ))}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
