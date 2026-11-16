import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import { EthernetPort, Monitor, Users } from 'lucide-react';
import { t, tn } from '../../i18n';
import { attachText, flowName, flowRate, shortPort, TONES, toneLabel, toneOf, type Flow, type Flows, type Tone } from '../../net/mcast';
import { FlowButtons, FlowFacts, receiverName, ToneIcon, TypeChip } from './FlowViews';

/** Au-delà, les flux les moins lourds ne sont pas tracés (le sélectionné l'est toujours). */
const MAX_FLOWS = 60;
const MAX_PILLS = 40;
/** Au-delà, les récepteurs sont regroupés par switch. */
const MAX_RECEIVERS = 16;
/** Au-delà, les émetteurs sont regroupés par switch. */
const MAX_EMITTERS = 16;

interface MapNode {
  id: string;
  kind: 'e' | 's' | 'r';
  x: number; // %
  y: number; // %
  label: string;
  sub: string;
  tone?: Tone;
  group?: boolean;
}

interface Edge {
  a: string;
  b: string;
  tone: Tone;
  w: number;
  dash: boolean;
}

const width = (bps: number) => Math.max(1.5, Math.log10(bps / 1e6 + 1) * 2.2);
const curve = (a: [number, number], b: [number, number]) => {
  const dx = (b[0] - a[0]) / 2;
  return `M${a[0].toFixed(1)},${a[1].toFixed(1)} C${(a[0] + dx).toFixed(1)},${a[1].toFixed(1)} ${(b[0] - dx).toFixed(1)},${b[1].toFixed(1)} ${b[0].toFixed(1)},${b[1].toFixed(1)}`;
};

/** Spreads n items over a column, top margin for the column titles. */
const spread = (i: number, n: number) => 12 + ((i + 0.5) * 84) / Math.max(n, 1);

/** The edges of one flow: emitter → its switch → … → receiving switches → receivers. */
function edgesOf(f: Flow, rxNode: (switchId: string, port: string) => string, emNode: (f: Flow) => string | null): Edge[] {
  const tone = toneOf(f);
  const w = width(flowRate(f));
  const dash = !f.announced;
  const out: Edge[] = [];
  const e = emNode(f);
  if (e && f.emitter?.at) out.push({ a: e, b: `s:${f.emitter.at.switchId}`, tone, w, dash });
  for (const n of f.path) if (n.parent) out.push({ a: `s:${n.parent}`, b: `s:${n.switchId}`, tone, w, dash });
  for (const r of f.receivers) if (!r.router && !r.peer) out.push({ a: `s:${r.switchId}`, b: rxNode(r.switchId, r.port), tone, w, dash });
  return out;
}

export function FlowMap({ data, flows, selected, onSelect }: { data: Flows; flows: Flow[]; selected: string | null; onSelect: (key: string | null) => void }) {
  const box = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<[number, number]>([0, 0]);
  const sel = flows.find((f) => f.key === selected) ?? flows.find((f) => f.live) ?? flows[0];

  // The flows drawn: the live ones first, the heaviest first, the selected one always.
  const drawn = useMemo(() => {
    const sorted = [...flows].sort((a, b) => Number(b.live) - Number(a.live) || flowRate(b) - flowRate(a));
    const list = sorted.slice(0, MAX_FLOWS);
    if (sel && !list.includes(sel)) list.push(sel);
    return list;
  }, [flows, sel]);

  const { nodes, edges, labels, height, emNode } = useMemo(() => {
    const swById = new Map(data.switches.map((s) => [s.id, s]));
    // Receivers: one node each, or one per switch when there are too many.
    const rxKeys = new Set<string>();
    for (const f of drawn) for (const r of f.receivers) if (!r.router && !r.peer) rxKeys.add(`${r.switchId}|${r.port}`);
    const grouped = rxKeys.size > MAX_RECEIVERS;
    const rxNode = (sw: string, port: string) => (grouped ? `g:${sw}` : `r:${sw}|${port}`);
    // Switches involved, and who sends or receives behind them.
    const used = new Set<string>();
    const sends = new Set<string>();
    const gets = new Set<string>();
    for (const f of drawn) {
      if (f.emitter?.at) {
        used.add(f.emitter.at.switchId);
        sends.add(f.emitter.at.switchId);
      }
      for (const n of f.path) used.add(n.switchId);
      for (const r of f.receivers) {
        used.add(r.switchId);
        if (!r.router && !r.peer) gets.add(r.switchId);
      }
    }
    const ids = [...used].filter((id) => swById.has(id));
    const links = data.links.filter((l) => used.has(l.a) && used.has(l.b));
    const degree = (id: string) => links.filter((l) => l.a === id || l.b === id).length;
    // Breadth first from the most linked switch of each part of the graph.
    const depth = new Map<string, number>();
    for (const root of [...ids].sort((a, b) => degree(b) - degree(a) || (swById.get(a)!.name < swById.get(b)!.name ? -1 : 1))) {
      if (depth.has(root)) continue;
      depth.set(root, 0);
      const queue = [root];
      while (queue.length) {
        const x = queue.shift()!;
        for (const l of links) {
          const y = l.a === x ? l.b : l.b === x ? l.a : null;
          if (y && !depth.has(y)) {
            depth.set(y, depth.get(x)! + 1);
            queue.push(y);
          }
        }
      }
    }
    // Columns: the core in the middle, switches with emitters on its left, those with only receivers on its right.
    const col = (id: string) => {
      const linked = degree(id) > 0;
      if (linked && depth.get(id) === 0 && ids.length > 1) return 52;
      return sends.has(id) || !gets.has(id) ? 32 : 70;
    };
    const cols = new Map<number, string[]>();
    for (const id of ids) cols.set(col(id), [...(cols.get(col(id)) ?? []), id]);
    const pos = new Map<string, [number, number]>();
    for (const [x, list] of cols) {
      list.sort((a, b) => (swById.get(a)!.name < swById.get(b)!.name ? -1 : 1));
      list.forEach((id, i) => pos.set(`s:${id}`, [x, spread(i, list.length)]));
    }
    const out: MapNode[] = ids.map((id) => {
      const s = swById.get(id)!;
      const [x, y] = pos.get(`s:${id}`)!;
      return { id: `s:${id}`, kind: 's', x, y, label: s.name, sub: s.ip ?? '' };
    });
    // Emitters, in the order of their switches.
    const emitters = new Map<string, { label: string; sub: string; tone: Tone; sw?: string }>();
    for (const f of drawn) {
      const ip = f.emitter?.ip ?? f.source;
      if (!ip) continue;
      const known = emitters.get(ip);
      if (known) {
        if (toneOf(f) === 'v') known.tone = 'v';
        continue;
      }
      emitters.set(ip, { label: f.emitter?.name ?? ip, sub: f.emitter?.name ? ip : attachText(f.emitter?.at), tone: toneOf(f), sw: f.emitter?.at?.switchId });
    }
    const swY = (sw?: string) => (sw ? (pos.get(`s:${sw}`)?.[1] ?? 50) : 101);
    const emGrouped = emitters.size > MAX_EMITTERS;
    const emNode = (f: Flow) => {
      const ip = f.emitter?.ip ?? f.source;
      if (!ip) return null;
      return emGrouped ? `eg:${f.emitter?.at?.switchId ?? ''}` : `e:${ip}`;
    };
    if (emGrouped) {
      const per = new Map<string, { n: number; name: string }>();
      for (const e of emitters.values()) {
        const k = e.sw ?? '';
        const g = per.get(k) ?? { n: 0, name: e.sw ? (swById.get(e.sw)?.name ?? '') : t('Unknown port') };
        g.n++;
        per.set(k, g);
      }
      const glist = [...per].sort((a, b) => swY(a[0] || undefined) - swY(b[0] || undefined));
      glist.forEach(([k, g], i) => out.push({ id: `eg:${k}`, kind: 'e', x: 9, y: spread(i, glist.length), label: tn(g.n, '{n} emitter', '{n} emitters'), sub: g.name, group: true }));
    }
    const elist = emGrouped ? [] : [...emitters].sort((a, b) => swY(a[1].sw) - swY(b[1].sw));
    elist.forEach(([ip, e], i) => out.push({ id: `e:${ip}`, kind: 'e', x: 9, y: spread(i, elist.length), label: e.label, sub: e.sub, tone: e.tone }));
    // Receivers.
    const rx = new Map<string, { label: string; sub: string; sw: string; n: number }>();
    for (const f of drawn) {
      for (const r of f.receivers) {
        if (r.router || r.peer) continue;
        const id = rxNode(r.switchId, r.port);
        if (!rx.has(id)) rx.set(id, grouped ? { label: '', sub: r.switchName, sw: r.switchId, n: 0 } : { label: receiverName(r), sub: `${r.switchName} ${shortPort(r.port)}`, sw: r.switchId, n: 1 });
      }
    }
    if (grouped) {
      // Distinct ports per switch.
      const per = new Map<string, Set<string>>();
      for (const k of rxKeys) {
        const [sw, port] = k.split('|');
        per.set(sw, (per.get(sw) ?? new Set()).add(port));
      }
      for (const r of rx.values()) r.n = per.get(r.sw)?.size ?? r.n;
    }
    const rlist = [...rx].sort((a, b) => swY(a[1].sw) - swY(b[1].sw) || (a[1].label < b[1].label ? -1 : 1));
    rlist.forEach(([id, r], i) => out.push({ id, kind: 'r', x: 86, y: spread(i, rlist.length), label: grouped ? tn(r.n, '{n} receiver', '{n} receivers') : r.label, sub: r.sub, group: grouped }));
    // Edges: the others merged by pair of nodes and type.
    const merged = new Map<string, Edge>();
    for (const f of drawn) {
      if (f === sel) continue;
      for (const e of edgesOf(f, rxNode, emNode)) {
        const k = `${e.a}>${e.b}>${e.tone}>${e.dash}`;
        const cur = merged.get(k);
        if (!cur || cur.w < e.w) merged.set(k, e);
      }
    }
    const rows = Math.max(emGrouped ? 0 : elist.length, rlist.length, ...[...cols.values()].map((l) => l.length), 4);
    const lbl = links.map((l) => {
      const [a, b] = [pos.get(`s:${l.a}`)!, pos.get(`s:${l.b}`)!];
      const left = a[0] <= b[0] ? l.aPort : l.bPort;
      return { a: `s:${l.a}`, b: `s:${l.b}`, text: shortPort(left.split(' · ')[0]) + (left.includes(' · ') ? ' …' : '') };
    });
    return { nodes: out, edges: [...merged.values()], labels: lbl, height: Math.max(380, rows * 50 + 70), emNode };
  }, [data, drawn, sel]);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize([el.clientWidth, el.clientHeight]));
    ro.observe(el);
    setSize([el.clientWidth, el.clientHeight]);
    return () => ro.disconnect();
  }, []);

  const at = (id: string): [number, number] | null => {
    const n = nodes.find((x) => x.id === id);
    return n ? [(n.x / 100) * size[0], (n.y / 100) * size[1]] : null;
  };
  const selEdges = sel ? edgesOf(sel, (sw, port) => (nodes.some((n) => n.id === `g:${sw}`) ? `g:${sw}` : `r:${sw}|${port}`), emNode) : [];
  const onPath = new Set(selEdges.flatMap((e) => [e.a, e.b]));
  const path = (e: Edge, cls: string, key: string, extra = 0) => {
    const a = at(e.a);
    const b = at(e.b);
    if (!a || !b) return null;
    return <path key={key} d={curve(a, b)} className={`t-${e.tone} ${e.dash ? 'unk' : ''} ${cls}`} strokeWidth={e.w + extra} />;
  };
  const pills = flows.slice(0, MAX_PILLS);
  return (
    <>
      <div className="flowpills">
        {pills.map((f) => (
          <button key={f.key} className={`fp ${f === sel ? 'on' : ''} ${f.state === 'warn' || f.state === 'unk' ? 'warn' : ''}`} title={`${f.group}:${f.port}`} onClick={() => onSelect(f.key)}>
            <i className={`c-${toneOf(f)}`} />
            <span>{flowName(f)}</span>
          </button>
        ))}
        {flows.length > MAX_PILLS && <span className="fp more">+{flows.length - MAX_PILLS}</span>}
      </div>
      <div className="map" ref={box} style={{ height }}>
        <svg className="links">
          {labels.map((l) => {
            const a = at(l.a);
            const b = at(l.b);
            if (!a || !b) return null;
            return <path key={`sw-${l.a}-${l.b}`} d={curve(a, b)} className="t-sw" />;
          })}
          {edges.map((e, i) => path(e, sel ? 'dim' : '', `e${i}`))}
          {selEdges.map((e, i) => path(e, 'sel', `s${i}`, 1.5))}
          {labels.map((l) => {
            const a = at(l.a);
            const b = at(l.b);
            if (!a || !b) return null;
            return (
              <text key={`t-${l.a}-${l.b}`} x={(a[0] + b[0]) / 2} y={(a[1] + b[1]) / 2 - 6} textAnchor="middle">
                {l.text}
              </text>
            );
          })}
        </svg>
        <span className="col-h" style={{ left: '9%' }}>
          {t('Emitters')}
        </span>
        <span className="col-h" style={{ left: '50%' }}>
          {t('Switches')}
        </span>
        <span className="col-h" style={{ left: '86%' }}>
          {t('Receivers')}
        </span>
        {nodes.map((n) => {
          const hot = sel && onPath.has(n.id);
          const cls = n.kind === 's' ? 'sw' : '';
          const mark = hot ? (n.kind === 'r' ? 'rx-on' : sel.state === 'warn' && n.kind === 'e' ? 'warnb' : 'hot') : '';
          return (
            <div key={n.id} className={`mn ${cls} ${mark}`} style={{ left: `${n.x}%`, top: `${n.y}%` }} title={n.sub ? `${n.label} · ${n.sub}` : n.label}>
              <span className="ic">{n.kind === 's' ? <EthernetPort size={14} /> : n.kind === 'e' ? (n.group ? <Users size={14} /> : <ToneIcon k={n.tone ?? 'o'} size={14} />) : n.group ? <Users size={14} /> : <Monitor size={14} />}</span>
              <span>
                <b>{n.label}</b>
                {n.sub && <small>{n.sub}</small>}
              </span>
            </div>
          );
        })}
        <div className="legend-map">
          {TONES.map((k) => (
            <span key={k}>
              <i className={`c-${k}`} />
              {toneLabel(k)}
            </span>
          ))}
          <span className="muted2">{t('width = rate')}</span>
        </div>
      </div>
      {sel && (
        <div className="selcard">
          <div>
            <h3>
              <TypeChip f={sel} /> <span>{flowName(sel)}</span>
            </h3>
            <span className="ip">
              {sel.port ? `${sel.group}:${sel.port}` : sel.group}
              {' · '}
              {sel.emitter?.name ?? sel.source ?? '—'} → {tn(sel.receiverCount, '{n} receiver', '{n} receivers')}
            </span>
          </div>
          <FlowFacts f={sel} data={data} />
          <div className="act">
            <FlowButtons f={sel} checkOn={data.check.enabled} onPair={onSelect} />
          </div>
        </div>
      )}
    </>
  );
}
