import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import { EthernetPort, Monitor, Timer, Users } from 'lucide-react';
import { t, tn } from '../../i18n';
import { fmtSeen } from '../../net/model';
import { fmtDateTime } from '../../ui/format';
import { attachText, type Flows, type PtpClock } from '../../net/mcast';

/** Au-delà, les appareils sont regroupés par switch. */
const MAX_DEVICES = 18;

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
const spread = (i: number, n: number) => 10 + ((i + 0.5) * 82) / Math.max(n, 1);

function roleText(c: PtpClock): string {
  return c.role === 'gm' ? t('Grandmaster') : c.role === 'boundary' ? t('Boundary clock') : t('Backup');
}

export function Ptp({ data }: { data: Flows }) {
  const { gm, clocks, devices, changes } = data.ptp;
  const box = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<[number, number]>([0, 0]);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize([el.clientWidth, el.clientHeight]));
    ro.observe(el);
    setSize([el.clientWidth, el.clientHeight]);
    return () => ro.disconnect();
  }, []);

  const { nodes, links, height } = useMemo(() => {
    const out: TreeNode[] = [];
    const lk: [string, string, string][] = [];
    // Clocks on the left, the grandmaster first.
    const left = [...clocks].sort((a, b) => Number(b.ip === gm?.ip) - Number(a.ip === gm?.ip));
    const last = changes[0];
    left.forEach((c, i) => {
      const main = gm && c.ip === gm.ip && c.clock === gm.clock;
      const sub = main ? t('Grandmaster · class {c} · {src}', { c: c.clockClass, src: c.timeSource.toUpperCase() }) : `${roleText(c)} · ${t('priority {p1} / {p2}', { p1: c.priority1, p2: c.priority2 })}`;
      out.push({
        id: `c:${c.ip}:${c.domain}`,
        x: 13,
        y: spread(i, left.length),
        label: c.name ?? c.ip,
        sub: c.wrongDomain ? `${sub} · ${t('domain {d}', { d: c.domain })}` : sub,
        cls: main ? 'gmnode' : c.wrongDomain ? 'warnb' : '',
        icon: 'gm',
        title: main && last ? t('Last change: {date}', { date: fmtDateTime(last.at) }) : c.gm,
      });
    });
    // Switches where clocks and devices are plugged.
    const sw = new Map<string, string>();
    for (const x of [...clocks, ...devices]) if (x.at) sw.set(x.at.switchId, x.at.switchName);
    const swIds = [...sw.keys()].sort((a, b) => (sw.get(a)! < sw.get(b)! ? -1 : 1));
    swIds.forEach((id, i) => out.push({ id: `s:${id}`, x: 45, y: spread(i, swIds.length), label: sw.get(id)!, sub: '', cls: 'sw', icon: 'sw' }));
    // Switch to switch, along the links, from the grandmaster's switch.
    const root = gm?.at?.switchId;
    if (root) {
      const parent = new Map<string, string>();
      const seen = new Set([root]);
      const queue = [root];
      while (queue.length) {
        const x = queue.shift()!;
        for (const l of data.links) {
          const y = l.a === x ? l.b : l.b === x ? l.a : null;
          if (y && !seen.has(y)) {
            seen.add(y);
            parent.set(y, x);
            queue.push(y);
          }
        }
      }
      const drawn = new Set<string>();
      for (const id of swIds) {
        let cur = id;
        while (parent.has(cur) && !drawn.has(cur)) {
          drawn.add(cur);
          const p = parent.get(cur)!;
          if (sw.has(p) && sw.has(cur)) lk.push([`s:${p}`, `s:${cur}`, '']);
          else if (sw.has(cur) && !sw.has(p)) {
            // Through a switch with no clock: straight to the nearest shown ancestor.
            let up = p;
            while (!sw.has(up) && parent.has(up)) up = parent.get(up)!;
            if (sw.has(up)) lk.push([`s:${up}`, `s:${cur}`, '']);
          }
          cur = p;
        }
      }
    }
    for (const c of clocks) {
      const main = gm && c.ip === gm.ip && c.clock === gm.clock;
      if (c.at) lk.push([`c:${c.ip}:${c.domain}`, `s:${c.at.switchId}`, main ? '' : c.wrongDomain ? 'bad' : 'bk']);
    }
    // Devices on the right (grouped by switch when there are many).
    const grouped = devices.length > MAX_DEVICES;
    const gmNode = gm ? `c:${gm.ip}:${gm.domain}` : null;
    if (grouped) {
      const per = new Map<string, { n: number; bad: number; name: string }>();
      for (const d of devices) {
        const k = d.at?.switchId ?? '';
        const e = per.get(k) ?? { n: 0, bad: 0, name: d.at?.switchName ?? t('Unknown port') };
        e.n++;
        e.bad += Number(d.wrongDomain);
        per.set(k, e);
      }
      [...per].forEach(([k, e], i) => {
        out.push({ id: `g:${k}`, x: 80, y: spread(i, per.size), label: tn(e.n, '{n} device', '{n} devices'), sub: e.name, cls: e.bad ? 'warnb' : '', icon: 'group' });
        const from = k ? `s:${k}` : gmNode;
        if (from) lk.push([from, `g:${k}`, e.bad ? 'bad' : k ? '' : 'bk']);
      });
    } else {
      const list = [...devices].sort((a, b) => (a.at?.switchName ?? '~').localeCompare(b.at?.switchName ?? '~') || a.ip.localeCompare(b.ip));
      list.forEach((d, i) => {
        out.push({ id: `d:${d.clock}:${d.domain}`, x: 80, y: spread(i, list.length), label: d.name ?? d.ip, sub: t('domain {d}', { d: d.domain }), cls: d.wrongDomain ? 'warnb' : '', icon: 'dev' });
        const from = d.at ? `s:${d.at.switchId}` : gmNode;
        if (from) lk.push([from, `d:${d.clock}:${d.domain}`, d.wrongDomain ? 'bad' : d.at ? '' : 'bk']);
      });
    }
    const rows = Math.max(left.length, swIds.length, grouped ? 0 : devices.length, 3);
    return { nodes: out, links: lk, height: Math.max(300, rows * 48 + 50) };
  }, [clocks, devices, gm, changes, data.links]);

  const at = (id: string): [number, number] | null => {
    const n = nodes.find((x) => x.id === id);
    return n ? [(n.x / 100) * size[0], (n.y / 100) * size[1]] : null;
  };

  if (!clocks.length && !devices.length) return <div className="empty">{t('No PTP message heard')}</div>;
  const domainCell = (d: number, bad: boolean) => <td className={`num ip ${bad ? 'warn-t' : ''}`}>{d}</td>;
  return (
    <>
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
      <table className="pt">
        <colgroup>
          <col style={{ width: '22%' }} />
          <col style={{ width: 140 }} />
          <col style={{ width: 130 }} />
          <col style={{ width: 90 }} />
          <col />
          <col style={{ width: 150 }} />
        </colgroup>
        <thead>
          <tr>
            <th>{t('Device')}</th>
            <th>{t('IP')}</th>
            <th>{t('Role')}</th>
            <th className="num">{t('Domain')}</th>
            <th>{t('Connection')}</th>
            <th>{t('Last message')}</th>
          </tr>
        </thead>
        <tbody>
          {clocks.map((c) => (
            <tr key={`c${c.ip}:${c.domain}:${c.clock}`} className={c.live ? '' : 'off'} title={c.clock}>
              <td>{c.name ?? c.clock}</td>
              <td className="ip">{c.ip}</td>
              <td>{c.role === 'gm' && !c.wrongDomain ? <span className="st used">{t('Master')}</span> : <span className="muted2">{roleText(c)}</span>}</td>
              {domainCell(c.domain, c.wrongDomain)}
              <td>{c.at ? <span className="ip">{attachText(c.at)}</span> : <span className="muted2">—</span>}</td>
              <td>{fmtSeen(c.lastSeen)}</td>
            </tr>
          ))}
          {devices.map((d) => (
            <tr key={`d${d.clock}:${d.domain}`} className={d.live ? '' : 'off'} title={d.clock}>
              <td>{d.name ?? d.clock}</td>
              <td className="ip">{d.ip}</td>
              <td>{t('Follower')}</td>
              {domainCell(d.domain, d.wrongDomain)}
              <td>{d.at ? <span className="ip">{attachText(d.at)}</span> : <span className="muted2">—</span>}</td>
              <td>{fmtSeen(d.lastSeen)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
