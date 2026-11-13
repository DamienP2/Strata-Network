import { useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import type { LucideIcon } from 'lucide-react';
import { ChevronDown, ChevronRight, Download, Ellipsis, EthernetPort, Layers, ListTree, Network as NetIcon, Scan as Fit } from 'lucide-react';
import { t, tn } from '../i18n';
import { Segmented } from '../ui/fields';
import { Empty } from '../ui/Empty';
import { byIp, parseRange, ipToInt } from '../net/ip';
import { KIND_ICON, displayName, downloadText, portName, useAllAddresses, useList, vlanOfIp } from '../net/model';
import type { Address, Network, Switch, Vlan } from '../net/types';

interface TNode {
  id: string;
  name: string;
  ip?: string;
  sub?: string;
  icon: LucideIcon;
  up?: boolean;
  vlan?: Vlan;
  children: TNode[];
  /** Le port du parent par lequel il est relié. */
  port?: string;
  /** Lien entre deux switchs. */
  trunk?: boolean;
  more?: boolean;
}

/* ---------- L'arbre, construit depuis les données ---------- */

function buildTree(switches: Switch[], addrs: Map<string, Address>, networks: Network[]): TNode[] {
  const nodes = new Map<string, TNode>();
  const swByIp = new Map<string, Switch>();
  for (const sw of switches) if (sw.ip) swByIp.set(sw.ip, sw);
  const idOfIp = (ip: string) => (swByIp.has(ip) ? `sw:${swByIp.get(ip)!.id}` : `ip:${ip}`);

  for (const sw of switches) {
    const a = sw.ip ? addrs.get(sw.ip) : undefined;
    nodes.set(`sw:${sw.id}`, { id: `sw:${sw.id}`, name: sw.name, ip: sw.ip, icon: EthernetPort, up: a?.up, children: [] });
  }
  const hostNode = (ip: string) => {
    const id = idOfIp(ip);
    if (!nodes.has(id)) {
      const a = addrs.get(ip);
      nodes.set(id, { id, name: displayName(a, ip), ip, icon: KIND_ICON[a?.kind ?? 'other'], up: a?.up, children: [] });
    }
    return id;
  };
  for (const a of addrs.values()) if (a.up || a.lastSeen) hostNode(a.ip);

  const parent = new Map<string, { id: string; port?: string; trunk?: boolean }>();
  const isAncestor = (anc: string, of: string) => {
    for (let cur: string | undefined = of, guard = 0; cur && guard < 10000; cur = parent.get(cur)?.id, guard++) if (cur === anc) return true;
    return false;
  };
  const link = (child: string, p: { id: string; port?: string; trunk?: boolean }) => {
    if (child === p.id || parent.has(child) || !nodes.has(p.id) || isAncestor(child, p.id)) return;
    parent.set(child, p);
  };

  // Machines branchées sur un port de switch.
  for (const sw of switches)
    for (const p of sw.ports) if (p.host && !swByIp.has(p.host)) link(hostNode(p.host), { id: `sw:${sw.id}`, port: p.name || String(p.n) });
  for (const a of addrs.values())
    if (a.switch && !swByIp.has(a.ip) && nodes.has(`sw:${a.switch.id}`)) {
      const sw = switches.find((s) => s.id === a.switch!.id);
      link(hostNode(a.ip), { id: `sw:${a.switch.id}`, port: portName(sw, a.switch.port) });
    }

  // Switchs reliés entre eux : chaque groupe part de celui qui a un parent, sinon du plus relié.
  const adj = new Map<string, { to: string; port: string }[]>();
  for (const sw of switches)
    for (const p of sw.ports)
      if (p.peer && nodes.has(`sw:${p.peer.id}`)) {
        const a = `sw:${sw.id}`;
        const b = `sw:${p.peer.id}`;
        const other = switches.find((s) => s.id === p.peer!.id);
        adj.set(a, [...(adj.get(a) ?? []), { to: b, port: p.name || String(p.n) }]);
        adj.set(b, [...(adj.get(b) ?? []), { to: a, port: portName(other, p.peer.port) }]);
      }
  const seen = new Set<string>();
  const ordered = [...switches].sort((x, y) => {
    const px = x.ip && addrs.get(x.ip)?.parent ? 1 : 0;
    const py = y.ip && addrs.get(y.ip)?.parent ? 1 : 0;
    return py - px || (adj.get(`sw:${y.id}`)?.length ?? 0) - (adj.get(`sw:${x.id}`)?.length ?? 0) || x.name.localeCompare(y.name);
  });
  for (const sw of ordered) {
    const root = `sw:${sw.id}`;
    if (seen.has(root)) continue;
    seen.add(root);
    const queue = [root];
    while (queue.length) {
      const cur = queue.shift()!;
      for (const e of adj.get(cur) ?? []) {
        if (seen.has(e.to)) continue;
        seen.add(e.to);
        link(e.to, { id: cur, port: e.port, trunk: true });
        queue.push(e.to);
      }
    }
  }

  // Parents posés à la main.
  for (const a of addrs.values()) if (a.parent && nodes.has(idOfIp(a.ip))) link(idOfIp(a.ip), { id: hostNode(a.parent) });

  // Le reste sous la passerelle de son réseau, quand elle est connue.
  const nets = networks.map((n) => ({ n, r: parseRange(n.range) }));
  const gatewayOf = (ip: string | undefined) => {
    const v = ipToInt(ip);
    if (v === null) return null;
    const hit = nets.find(({ n, r }) => r && n.gateway && v >= r.lo && v <= r.hi && addrs.has(n.gateway));
    return hit ? hit.n.gateway! : null;
  };
  for (const node of [...nodes.values()]) {
    if (parent.has(node.id)) continue;
    const gw = gatewayOf(node.ip);
    if (gw && gw !== node.ip) link(node.id, { id: hostNode(gw) });
  }

  // Ce qui reste seul se range sous son réseau.
  const hasKids = new Set([...parent.values()].map((p) => p.id));
  for (const node of [...nodes.values()]) {
    if (parent.has(node.id) || hasKids.has(node.id) || !node.id.startsWith('ip:')) continue;
    const v = ipToInt(node.ip);
    const hit = v === null ? undefined : nets.find(({ r }) => r && v >= r.lo && v <= r.hi);
    if (!hit) continue;
    const id = `net:${hit.n.id}`;
    if (!nodes.has(id)) nodes.set(id, { id, name: hit.n.name, sub: hit.n.range, icon: NetIcon, children: [] });
    link(node.id, { id });
  }

  for (const [child, p] of parent) {
    const c = nodes.get(child)!;
    c.port = p.port;
    c.trunk = p.trunk;
    nodes.get(p.id)!.children.push(c);
  }
  const sort = (list: TNode[]) =>
    list.sort((a, b) => Number(b.children.length > 0) - Number(a.children.length > 0) || (a.ip && b.ip ? byIp(a.ip, b.ip) : a.name.localeCompare(b.name)));
  for (const n of nodes.values()) sort(n.children);
  return sort([...nodes.values()].filter((n) => !parent.has(n.id)));
}

/** Par VLAN : un groupe par VLAN, ses machines dessous. */
function buildVlans(switches: Switch[], addrs: Map<string, Address>, vlans: Vlan[]): TNode[] {
  const groups = new Map<number, TNode[]>();
  const none: TNode[] = [];
  const machines: TNode[] = [];
  const swIps = new Set(switches.map((s) => s.ip).filter(Boolean));
  for (const sw of switches) machines.push({ id: `sw:${sw.id}`, name: sw.name, ip: sw.ip, icon: EthernetPort, up: sw.ip ? addrs.get(sw.ip)?.up : undefined, children: [] });
  for (const a of addrs.values())
    if ((a.up || a.lastSeen) && !swIps.has(a.ip)) machines.push({ id: `ip:${a.ip}`, name: displayName(a, a.ip), ip: a.ip, icon: KIND_ICON[a.kind ?? 'other'], up: a.up, children: [] });
  const portVlan = new Map<string, number>();
  for (const sw of switches) for (const p of sw.ports) if (p.host && p.mode === 'access' && p.vlan !== undefined) portVlan.set(p.host, p.vlan);
  for (const m of machines) {
    const v = vlanOfIp(vlans, m.ip)?.id ?? (m.ip ? portVlan.get(m.ip) : undefined);
    if (v === undefined) none.push(m);
    else groups.set(v, [...(groups.get(v) ?? []), m]);
  }
  const byAddr = (a: TNode, b: TNode) => (a.ip && b.ip ? byIp(a.ip, b.ip) : a.name.localeCompare(b.name));
  const out: TNode[] = [...vlans]
    .sort((a, b) => a.id - b.id)
    .filter((v) => groups.has(v.id))
    .map((v) => ({ id: `vlan:${v.id}`, name: `${v.id} · ${v.name}`, sub: v.subnet, icon: Layers, vlan: v, children: groups.get(v.id)!.sort(byAddr) }));
  if (none.length) out.push({ id: 'vlan:none', name: t('Without VLAN'), icon: Layers, children: none.sort(byAddr) });
  return out;
}

/* ---------- Mise en page du schéma ---------- */

const NW = 180;
const NH = 44;
const LW = 160;
const LH = 38;
const COLW = 210;
const LEAF_TOP = 22;
const LEAF_STEP = 46;

interface Placed {
  node: TNode;
  x: number;
  y: number;
  leaf: boolean;
}
interface Edge {
  d: string;
  trunk?: boolean;
  vlan?: Vlan;
}
interface Label {
  x: number;
  y: number;
  text: string;
}

function layout(roots: TNode[], maxLeaves: number, expanded: Set<string>, restSub: (rest: TNode[]) => string) {
  const placed: Placed[] = [];
  const edges: Edge[] = [];
  const labels: Label[] = [];
  const split = (n: TNode) => {
    const inner = n.children.filter((c) => c.children.length > 0);
    let leaves = n.children.filter((c) => c.children.length === 0);
    if (leaves.length > maxLeaves + 1 && !expanded.has(n.id)) {
      const rest = leaves.slice(maxLeaves);
      leaves = [
        ...leaves.slice(0, maxLeaves),
        { id: `more:${n.id}`, name: tn(rest.length, '{n} other', '{n} others'), sub: restSub(rest), icon: Ellipsis, children: [], more: true },
      ];
    }
    return { inner, leaves };
  };
  const size = new Map<string, { w: number; h: number }>();
  const measure = (n: TNode): { w: number; h: number } => {
    const { inner, leaves } = split(n);
    const leafH = leaves.length ? LEAF_TOP + leaves.length * LEAF_STEP - (LEAF_STEP - LH) : 0;
    let kw = 0;
    let kh = 0;
    for (const k of inner) {
      const s = measure(k);
      kw += s.w;
      kh = Math.max(kh, s.h);
    }
    const gap = inner.length > 1 ? 56 : 36;
    const r = { w: Math.max(COLW, kw), h: NH + leafH + (inner.length ? gap + kh : 0) };
    size.set(n.id, r);
    return r;
  };
  const place = (n: TNode, x0: number, y: number) => {
    const s = size.get(n.id)!;
    const x = x0 + (s.w - NW) / 2;
    placed.push({ node: n, x, y, leaf: false });
    const { inner, leaves } = split(n);
    const tone = n.vlan;
    leaves.forEach((l, i) => {
      const ly = y + NH + LEAF_TOP + i * LEAF_STEP;
      placed.push({ node: l, x: x + 20, y: ly, leaf: true });
    });
    if (leaves.length) {
      const last = y + NH + LEAF_TOP + (leaves.length - 1) * LEAF_STEP + LH / 2;
      edges.push({ d: `M${x + 10} ${y + NH} V${last + 2}` + leaves.map((_, i) => ` M${x + 10} ${y + NH + LEAF_TOP + i * LEAF_STEP + LH / 2 + 1} H${x + 20}`).join(''), vlan: tone });
    }
    const leafH = leaves.length ? LEAF_TOP + leaves.length * LEAF_STEP - (LEAF_STEP - LH) : 0;
    const gap = inner.length > 1 ? 56 : 36;
    const ky = y + NH + leafH + gap;
    let cx = x0 + (s.w - inner.reduce((w, k) => w + size.get(k.id)!.w, 0)) / 2;
    const pc = x + NW / 2;
    for (const k of inner) {
      const ks = size.get(k.id)!;
      const kc = cx + ks.w / 2;
      edges.push({ d: inner.length > 1 || kc !== pc ? `M${pc} ${y + NH + leafH} V${ky - 28} H${kc} V${ky}` : `M${pc} ${y + NH + leafH} V${ky}`, trunk: k.trunk });
      if (k.port) labels.push({ x: kc + 6, y: ky - 6, text: k.port });
      place(k, cx, ky);
      cx += ks.w;
    }
  };
  let x = 0;
  let h = 0;
  for (const r of roots) {
    const s = measure(r);
    place(r, x, 16);
    x += s.w;
    h = Math.max(h, s.h);
  }
  return { placed, edges, labels, width: Math.max(x, COLW), height: h + 56 };
}

/* ---------- La page ---------- */

export function Topology() {
  const [switches] = useList<Switch[]>('switches', '/api/switches');
  const [networks] = useList<Network[]>('networks', '/api/networks');
  const [vlans] = useList<Vlan[]>('vlans', '/api/vlans');
  const addrs = useAllAddresses(networks);
  const [mode, setMode] = useState<'graph' | 'vlan'>('graph');
  const [sel, setSel] = useState<string | null>(null);
  const [fit, setFit] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const canvas = useRef<HTMLDivElement>(null);
  const board = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ w: 1200, h: 800 });

  useLayoutEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setBox({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    return () => ro.disconnect();
  }, [switches]);

  const tree = useMemo(() => (switches && networks ? buildTree(switches, addrs, networks) : []), [switches, addrs, networks]);
  const groups = useMemo(() => (switches && vlans ? buildVlans(switches, addrs, vlans) : []), [switches, addrs, vlans]);
  const roots = mode === 'graph' ? tree : groups;
  const plan = useMemo(() => {
    // Sous « 14 autres » : leurs VLAN.
    const restSub = (rest: TNode[]) => {
      if (mode === 'vlan') return '';
      const ids = [...new Set(rest.map((r) => vlanOfIp(vlans, r.ip)?.id).filter((x): x is number => x !== undefined))].sort((a, b) => a - b);
      return ids.length ? t('VLAN {list}', { list: ids.join(', ') }) : '';
    };
    return layout(roots, mode === 'graph' ? 4 : 12, expanded, restSub);
  }, [roots, mode, expanded, vlans]);

  if (!switches || !networks) return <div className="loading">{t('Loading…')}</div>;

  const scale = fit ? Math.min(1, (box.w - 36) / plan.width, (box.h - 76) / plan.height) : 1;
  const left = Math.max(18, (box.w - plan.width * scale) / 2);
  const pick = (n: TNode) => {
    if (n.more) {
      const owner = n.id.slice(5);
      setExpanded((s) => new Set(s).add(owner));
    } else setSel(n.id);
  };

  const exportSvg = () => {
    const el = board.current;
    if (!el) return;
    const css = getComputedStyle(el);
    const v = (name: string) => css.getPropertyValue(name).trim();
    const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
    const parts: string[] = [];
    parts.push(`<rect x="-18" y="0" width="${plan.width + 36}" height="${plan.height}" fill="${v('--bg')}"/>`);
    for (const e of plan.edges) parts.push(`<path d="${e.d}" fill="none" stroke="${e.vlan ? e.vlan.color : e.trunk ? v('--accent') : v('--line-2')}" stroke-width="${e.trunk ? 2 : 1.5}"${e.trunk || e.vlan ? ' opacity="0.7"' : ''}/>`);
    for (const l of plan.labels) parts.push(`<text x="${l.x}" y="${l.y}" fill="${v('--text-3')}" font-family="monospace" font-size="11">${esc(l.text)}</text>`);
    for (const p of plan.placed) {
      const w = p.leaf ? LW : NW;
      const h = p.leaf ? LH : NH;
      const icon = el.querySelector(`[data-id="${CSS.escape(p.node.id)}"] svg`)?.outerHTML ?? '';
      parts.push(
        `<g transform="translate(${p.x} ${p.y})"><rect x="0.5" y="0.5" width="${w - 1}" height="${h - 1}" rx="8" fill="${v('--panel')}" stroke="${v('--line-2')}"/>` +
          `<g transform="translate(10 ${(h - 16) / 2})" color="${p.node.vlan?.color ?? v('--accent-2')}">${icon}</g>` +
          `<text x="35" y="${h / 2 - 2}" fill="${v('--text')}" font-family="Inter, sans-serif" font-size="${p.leaf ? 12 : 12.5}" font-weight="${p.leaf ? 500 : 600}">${esc(p.node.name)}</text>` +
          `<text x="35" y="${h / 2 + 12}" fill="${v('--text-3')}" font-family="monospace" font-size="11">${esc(p.node.ip && p.node.ip !== p.node.name ? p.node.ip : (p.node.sub ?? ''))}</text>` +
          (p.node.up && !p.leaf ? `<circle cx="${w - 10.5}" cy="10.5" r="3.5" fill="${v('--pvw')}"/>` : '') +
          `</g>`,
      );
    }
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${plan.width + 36}" height="${plan.height}" viewBox="-18 0 ${plan.width + 36} ${plan.height}">${parts.join('')}</svg>`;
    downloadText(mode === 'graph' ? 'topology.svg' : 'topology-vlan.svg', svg, 'image/svg+xml');
  };

  return (
    <div className="topo-page">
      <aside className="panel left">
        <div className="lib-nav-section" style={{ padding: '18px 18px 6px' }}>
          <span className="grow">{t('Tree')}</span>
        </div>
        <div className="tree">
          {roots.map((n) => (
            <TreeRow key={n.id} node={n} depth={0} sel={sel} onPick={setSel} />
          ))}
        </div>
      </aside>
      <div className="topo-canvas" ref={canvas}>
        <div className="topo-tools">
          <Segmented
            value={mode}
            onChange={setMode}
            options={[
              { value: 'graph', label: t('Diagram') },
              { value: 'vlan', label: t('By VLAN') },
            ]}
          />
          <div className="spacer" />
          <button className={`icon-btn ${fit ? 'on accent' : ''}`} title={t('Fit to the window')} aria-pressed={fit} onClick={() => setFit((f) => !f)}>
            <Fit size={14} />
          </button>
          <button className="btn" disabled={roots.length === 0} onClick={exportSvg}>
            <Download size={14} /> {t('Export')}
          </button>
        </div>
        {roots.length === 0 ? (
          <Empty icon={ListTree} text={t('Nothing to show')} />
        ) : (
          <div style={{ width: plan.width * scale + left, height: plan.height * scale }}>
            <div className="topo-board" ref={board} style={{ width: plan.width, height: plan.height, marginLeft: left, transform: scale !== 1 ? `scale(${scale})` : undefined }}>
              <svg className="links" width={plan.width} height={plan.height}>
                {plan.edges.map((e, i) => (
                  <path key={i} d={e.d} className={e.vlan ? 'vlan' : e.trunk ? 'trunk' : undefined} style={e.vlan ? ({ '--vlan': e.vlan.color } as CSSProperties) : undefined} />
                ))}
                {plan.labels.map((l, i) => (
                  <text key={i} x={l.x} y={l.y}>
                    {l.text}
                  </text>
                ))}
              </svg>
              {plan.placed.map((p) => (
                <div
                  key={p.node.id}
                  data-id={p.node.id}
                  className={`topo-node ${p.leaf ? 'leaf' : ''} ${p.node.vlan ? 'vlan' : ''} ${sel === p.node.id ? 'on' : ''}`}
                  style={{ left: p.x, top: p.y, ...(p.node.vlan ? ({ '--vlan': p.node.vlan.color } as CSSProperties) : {}) }}
                  title={p.node.more ? undefined : [p.node.name, p.node.ip, p.node.port].filter(Boolean).join(' · ')}
                  onClick={() => pick(p.node)}
                >
                  <p.node.icon size={p.leaf ? 15 : 16} />
                  <div>
                    <b>{p.node.name}</b>
                    <small>{p.node.ip && p.node.ip !== p.node.name ? p.node.ip : (p.node.sub ?? '')}</small>
                  </div>
                  {!p.leaf && p.node.up !== undefined && <span className={`nav-dot ${p.node.up ? 'ok' : ''}`} />}
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function TreeRow({ node, depth, sel, onPick }: { node: TNode; depth: number; sel: string | null; onPick: (id: string) => void }) {
  const [open, setOpen] = useState(depth < 3);
  const kids = node.children.length > 0;
  return (
    <>
      <div className={`tree-row ${sel === node.id ? 'on' : ''}`} onClick={() => onPick(node.id)}>
        <span
          className="tw"
          onClick={(e) => {
            if (!kids) return;
            e.stopPropagation();
            setOpen((o) => !o);
          }}
        >
          {kids && (open ? <ChevronDown size={12} /> : <ChevronRight size={12} />)}
        </span>
        {node.vlan ? <span className="vlan-sw" style={{ '--vlan': node.vlan.color } as CSSProperties} /> : <node.icon size={15} />}
        <span className="name">{node.name}</span>
        {node.ip && <span className="ip">{node.ip}</span>}
      </div>
      {kids && open && (
        <div className="tree-kids">
          {node.children.map((c) => (
            <TreeRow key={c.id} node={c} depth={depth + 1} sel={sel} onPick={onPick} />
          ))}
        </div>
      )}
    </>
  );
}
