import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Bell, Check, ChevronDown, EthernetPort, HeartPulse, Link2, Pencil, Plus, Radio, Search, Sun, TriangleAlert, X } from 'lucide-react';
import { locale, t, tc, tn } from '../i18n';
import { api, can, serverEvents, serverNow, useApp, useChanged } from '../store';
import { navigate } from '../router';
import { Segmented } from '../ui/fields';
import { Dropdown, type DropOption } from '../ui/Dropdown';
import { confirmDialog } from '../ui/dialogs';
import { Empty } from '../ui/Empty';
import { PageHead, SubTabs, type SubTab } from './SidePage';
import { AlertsPane, HealthPane, HealthStrip, McastPane, OpticsPane, PortChart, hasHealth, ifName, poePct, shortAgg, useInfo, weakOptic } from './SwitchInfo';
import { SwitchModal } from './SwitchModal';
import { byIp } from '../net/ip';
import { KIND_ICON, diffWords, displayName, fail, fmtSeen, portName, signed, useAllAddresses, useList, vlanById } from '../net/model';
import { InlineText, vlanStyle } from '../net/parts';
import { blocksOf, summary } from '../net/groups';
import type { Address, Alert, AlertRules, Live, LiveDiff, LivePort, Neighbour, Network, Optic, PoePort, PortKind, PortMode, SnmpAgent, Switch, SwitchPort, Vlan } from '../net/types';

type PortSet = Partial<Record<keyof SwitchPort, unknown>>;
/** Filtre : tous, un VLAN (`v:10`), les trunks, les libres ; en ligne : lien actif, sans lien, écarts, erreurs, voisins, agrégats, surveillés. */
type Filter = 'all' | 'trunk' | 'free' | `v:${number}` | 'up' | 'down' | 'diff' | 'errors' | 'nb' | 'lag' | 'watched';
const LIVE_FILTERS: Filter[] = ['up', 'down', 'diff', 'errors', 'nb', 'lag', 'watched'];
type Tab = 'ports' | 'optics' | 'mcast' | 'health' | 'alerts';

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

  // Reprendre la config du switch (ports donnés, ou tous les écarts).
  const take = async (s: Switch, ports: number[] | null) => {
    try {
      const saved = await api<Switch>('POST', `/api/switches/${s.id}/take`, { ports });
      setSwitches((list) => (list ?? []).map((x) => (x.id === saved.id ? saved : x)));
    } catch (e) {
      fail(e);
    }
  };

  const setRules = async (s: Switch, rules: AlertRules) => {
    try {
      const saved = await api<Switch>('PUT', `/api/switches/${s.id}/rules`, rules);
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
            onTake={(ns) => take(sw, ns)}
            onEdit={() => setEditing(sw)}
            onRules={(r) => setRules(sw, r)}
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
            // Dans la liste tout de suite : la page ne retombe pas sur un autre switch en attendant le rechargement.
            if (s) setSwitches((list) => (list?.some((x) => x.id === s.id) ? list.map((x) => (x.id === s.id ? s : x)) : [...(list ?? []), s]));
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
  onTake,
  onEdit,
  onRules,
}: {
  sw: Switch;
  switches: Switch[];
  vlans: Vlan[] | null;
  addrs: Map<string, Address>;
  agent: SnmpAgent | undefined;
  hostOptions: DropOption[];
  editable: boolean;
  onPatch: (ns: number[], set: PortSet) => Promise<void>;
  onTake: (ns: number[] | null) => Promise<void>;
  onEdit: () => void;
  onRules: (r: AlertRules) => Promise<void>;
}) {
  const [q, setQ] = useState('');
  const [filter, setFilter] = useState<Filter>('all');
  const [tab, setTab] = useState<Tab>('ports');
  const [ticked, setTicked] = useState<Set<number>>(() => new Set());
  const [opened, setOpened] = useState<number | null>(null);
  const [goto, setGoto] = useState<{ n: number; seq: number } | null>(null);
  const [linking, setLinking] = useState(false);
  const live = useLive(sw.id, sw.online);
  const info = useInfo(sw.id, sw.online);
  const [allAlerts, loadAlerts] = useList<Alert[]>('alerts', '/api/alerts');
  const alerts = useMemo(() => (allAlerts ?? []).filter((a) => a.switchId === sw.id), [allAlerts, sw.id]);
  const unseen = alerts.filter((a) => !a.seen);
  const liveOf = useMemo(() => new Map((live?.ports ?? []).map((p) => [p.n, p])), [live]);
  const diffOf = useMemo(() => new Map((live?.diffs ?? []).map((d) => [d.n, d])), [live]);
  // Ce que les lectures lentes disent de chaque port.
  const facts = useMemo(() => {
    const nb = new Map<number, Neighbour>();
    for (const x of info?.neighbours ?? []) if (x.n !== undefined && !nb.has(x.n)) nb.set(x.n, x);
    const optic = new Map<number, Optic>();
    for (const o of info?.optics ?? []) if (o.n !== undefined) optic.set(o.n, o);
    const poe = new Map<number, PoePort>();
    for (const x of info?.poe?.ports ?? []) if (x.n !== undefined) poe.set(x.n, x);
    return {
      nb,
      optic,
      poe,
      lag: new Map((info?.lags ?? []).map((l) => [l.n, shortAgg(l.agg)])),
      blocked: new Set((info?.stp ?? []).filter((s) => s.state === 'blocking' || s.state === 'broken').map((s) => s.n)),
    };
  }, [info]);
  const isPoe = !!info?.poe && (info.poe.ports.length > 0 || info.poe.budgetW !== undefined);
  const weak = (n: number) => {
    const o = facts.optic.get(n);
    return !!o && weakOptic(o, sw.rules);
  };
  const poeFault = (x: PoePort | undefined) => !!x && (x.status === 'fault' || x.status === 'otherFault');
  // Hors ligne : les filtres et les onglets du switch en ligne n'ont plus de sens.
  useEffect(() => {
    if (!live && LIVE_FILTERS.includes(filter)) setFilter('all');
  }, [live, filter]);
  const tabs: SubTab[] = [{ id: 'ports', icon: <EthernetPort size={14} />, label: t('Ports'), count: sw.ports.length }];
  if (info?.optics.length) tabs.push({ id: 'optics', icon: <Sun size={14} />, label: t('Optics'), count: info.optics.length });
  if (info?.mcast.length) tabs.push({ id: 'mcast', icon: <Radio size={14} />, label: t('Multicast'), count: info.mcast.length });
  if (hasHealth(info)) tabs.push({ id: 'health', icon: <HeartPulse size={14} />, label: t('Health') });
  tabs.push({ id: 'alerts', icon: <Bell size={14} />, label: t('Alerts'), count: unseen.length || undefined });
  const current = tabs.some((x) => x.id === tab) ? tab : 'ports';
  // Les alertes de ce switch sont vues quand l'onglet est ouvert.
  const unseenIds = unseen.map((a) => a.id).join(',');
  useEffect(() => {
    if (current !== 'alerts' || !unseenIds) return;
    api('POST', '/api/alerts/seen', { ids: unseenIds.split(',') })
      .then(() => loadAlerts())
      .catch(fail);
  }, [current, unseenIds, loadAlerts]);
  const counts = live
    ? {
        up: sw.ports.filter((p) => liveOf.get(p.n)?.up).length,
        diff: sw.ports.filter((p) => diffOf.has(p.n)).length,
        errors: sw.ports.filter((p) => errorsOf(liveOf.get(p.n)) > 0).length,
        watched: sw.ports.filter((p) => p.watched).length,
        nb: sw.ports.filter((p) => facts.nb.has(p.n)).length,
        lag: sw.ports.filter((p) => facts.lag.has(p.n)).length,
      }
    : null;
  // La façade : chaque groupe de ports par blocs de 12 (2 rangées).
  const blocks = useMemo(() => {
    let at = 0;
    return sw.groups.flatMap((g) => {
      const list = sw.ports.slice(at, at + g.count);
      at += g.count;
      return blocksOf(list).map((ports) => ({ kind: g.kind, ports }));
    });
  }, [sw.groups, sw.ports]);

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

  // Les agrégats sous la façade : un crochet sous leurs ports.
  const faceRef = useRef<HTMLDivElement>(null);
  const [marks, setMarks] = useState<{ name: string; left: number; width: number }[]>([]);
  const lagKey = [...facts.lag.entries()].map(([n, a]) => `${n}:${a}`).join(',');
  useLayoutEffect(() => {
    const face = faceRef.current;
    if (!face) return;
    const measure = () => {
      const fr = face.getBoundingClientRect();
      const groups = new Map<string, number[]>();
      for (const [n, a] of facts.lag) groups.set(a, [...(groups.get(a) ?? []), n]);
      const next: { name: string; left: number; width: number }[] = [];
      for (const [name, ns] of groups) {
        const rs = ns.map((n) => face.querySelector<HTMLElement>(`[data-n="${n}"]`)?.getBoundingClientRect()).filter((r): r is DOMRect => !!r);
        if (!rs.length) continue;
        const left = Math.min(...rs.map((r) => r.left)) - fr.left + face.scrollLeft;
        const right = Math.max(...rs.map((r) => r.right)) - fr.left + face.scrollLeft;
        next.push({ name, left: left - 1, width: right - left + 2 });
      }
      setMarks((old) => (JSON.stringify(old) === JSON.stringify(next) ? old : next));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(face);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lagKey, blocks]);

  const needle = q.trim().toLowerCase();
  const matches = (p: SwitchPort) => {
    if (filter === 'trunk' && p.mode !== 'trunk') return false;
    if (filter === 'free' && (p.host || p.peer)) return false;
    if (filter === 'up' && !liveOf.get(p.n)?.up) return false;
    if (filter === 'down' && liveOf.get(p.n)?.up) return false;
    if (filter === 'diff' && !diffOf.has(p.n)) return false;
    if (filter === 'errors' && !errorsOf(liveOf.get(p.n))) return false;
    if (filter === 'nb' && !facts.nb.has(p.n)) return false;
    if (filter === 'lag' && !facts.lag.has(p.n)) return false;
    if (filter === 'watched' && !p.watched) return false;
    if (filter.startsWith('v:')) {
      const v = Number(filter.slice(2));
      if (p.mode === 'off' || (p.vlan !== v && !(p.mode === 'trunk' && p.tagged.includes(v)))) return false;
    }
    if (needle) {
      const host = p.host ? addrs.get(p.host) : undefined;
      const peer = p.peer ? switches.find((x) => x.id === p.peer!.id) : undefined;
      const nb = facts.nb.get(p.n);
      const hay = `${p.n} ${p.name} ${p.label} ${p.host ?? ''} ${host?.label ?? ''} ${host?.name ?? ''} ${peer?.name ?? ''} ${nb ? `${nb.name} ${nb.remotePort}` : ''}`.toLowerCase();
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
  const face = (p: SwitchPort, kind: PortKind) => {
    // En ligne : le lien et la charge lus dans le switch.
    const lp = live ? liveOf.get(p.n) : undefined;
    const up = !!lp?.up;
    const load = lp && up && lp.speedMbps ? Math.min(100, ((lp.inBps + lp.outBps) / (lp.speedMbps * 1e6)) * 100) : 0;
    const pp = facts.poe.get(p.n);
    return (
      <button
        key={p.n}
        type="button"
        data-n={p.n}
        className={`sw-port ${kind === 'rj45' ? '' : kind} ${p.mode === 'trunk' ? 'trunk' : ''} ${p.mode === 'off' ? 'off' : ''} ${live && !up ? 'down' : ''} ${diffOf.has(p.n) ? 'diff' : ''} ${ticked.has(p.n) ? 'sel' : ''}`}
        style={p.mode !== 'off' ? vlanStyle(vlanById(vlans, p.vlan)) : undefined}
        title={live ? `${portTitle(p)} · ${linkTitle(lp)}` : portTitle(p)}
        onClick={() => pick(p.n)}
      >
        {p.n}
        {(live || linked(p)) && <span className="link" />}
        {weak(p.n) && <span className="opt" />}
        {(pp?.on || poeFault(pp)) && <span className={`poe ${poeFault(pp) ? 'fault' : ''}`}>⚡</span>}
        {facts.blocked.has(p.n) && <span className="stp">⊘</span>}
        {up && (
          <span className="load">
            <i className={load > 60 ? 'hot' : ''} style={{ width: `${Math.max(6, load)}%` }} />
          </span>
        )}
      </button>
    );
  };

  const takeAll = async () => {
    if (!counts) return;
    const ok = await confirmDialog({
      title: tn(counts.diff, 'Take the setup of {n} port from the switch?', 'Take the setup of {n} ports from the switch?'),
      confirm: t('Take all'),
    });
    if (ok) await onTake(null);
  };

  const linkAll = async () => {
    setLinking(true);
    try {
      await api('POST', `/api/switches/${sw.id}/link-neighbours`);
    } catch (e) {
      fail(e);
    }
    setLinking(false);
  };

  // Les défauts du moment : alimentations, ventilateurs, températures, optiques, PoE.
  const faults: React.ReactNode[] = [];
  if (info) {
    for (const u of [...info.health.psus, ...info.health.fans].filter((x) => !x.ok))
      faults.push(
        <span key={`u-${u.name}`}>
          <b>{u.name}</b> {t('faulty')}
        </span>,
      );
    for (const x of info.health.temps.filter((x) => x.c > sw.rules.health.value))
      faults.push(
        <span key={`t-${x.name}`}>
          <b>{x.name}</b> : {t('{n} °C', { n: Math.round(x.c) })}
        </span>,
      );
    for (const o of info.optics.filter((o) => weakOptic(o, sw.rules)))
      faults.push(
        <span key={`o-${o.port}`}>
          <b>{ifName(sw, o)}</b> : {t('weak optical reception ({n} dBm)', { n: signed(o.rxDbm ?? 0) })}
        </span>,
      );
    const pp = poePct(info);
    if (pp !== null && pp > sw.rules.poe.value)
      faults.push(
        <span key="poe">
          <b>{t('PoE budget at {n} %', { n: Math.round(pp) })}</b>
        </span>,
      );
    for (const x of (info.poe?.ports ?? []).filter(poeFault))
      faults.push(
        <span key={`p-${x.port}`}>
          <b>{ifName(sw, x)}</b> : {t('PoE cut')}
        </span>,
      );
  }

  const sub = [sw.model, sw.ip, sw.group, summary(sw.groups)].filter(Boolean);
  const vlanList: DropOption[] = (vlans ?? []).map((v) => ({ value: String(v.id), label: `${v.id} · ${v.name}`, icon: <span className="vlan-sw" style={vlanStyle(v)} /> }));
  const peerOptions: DropOption[] = switches
    .filter((x) => x.id !== sw.id)
    .flatMap((x) => x.ports.map((p) => ({ value: `peer:${x.id}:${p.n}`, label: `${x.name} · ${p.name || p.n}`, sub: p.label || undefined, icon: <EthernetPort size={12} />, group: t('Switches') })));
  const allTicked = ports.length > 0 && ports.every((p) => ticked.has(p.n));
  const cols = live ? (editable ? 11 : 10) : 0;
  const links = info?.links ?? [];
  const otherName = (id: string) => switches.find((x) => x.id === id)?.name ?? id;
  const otherPort = (id: string, n: number) => portName(switches.find((x) => x.id === id), n);
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
          {agent && !sw.online && (
            <span className={`badge ${agent.lastPoll && !agent.lastPoll.ok ? 'warn' : ''}`} title={agent.lastPoll?.message || agent.name}>
              {agent.lastPoll ? t('SNMP · read {when}', { when: fmtSeen(agent.lastPoll.at) }) : t('SNMP')}
            </span>
          )}
          <OnlineBlock sw={sw} live={live} editable={editable} />
          {editable && (
            <button type="button" className="btn" onClick={onEdit}>
              <Pencil size={14} /> {t('Edit')}
            </button>
          )}
        </PageHead>
        {live && <HealthStrip sw={sw} live={live} info={info} />}
        {links.length > 0 && info && (
          <div className="info-bar">
            <Link2 size={16} />
            <span>
              <b>{tn(info.neighbours.length, '{n} neighbour', '{n} neighbours')}</b>{' '}
              {tn(links.length, 'found by CDP / LLDP, {n} of them a switch not linked in the inventory yet:', 'found by CDP / LLDP, {n} of them switches not linked in the inventory yet:')}{' '}
              {links.map((l) => `${portName(sw, l.n)} ↔ ${otherName(l.switchId)} ${otherPort(l.switchId, l.port)}`).join(', ')}
            </span>
            <div className="spacer" />
            {editable && (
              <button type="button" className="btn small primary" disabled={linking} onClick={() => void linkAll()}>
                {tn(links.length, 'Link the switch', 'Link the {n} switches')}
              </button>
            )}
          </div>
        )}
        {counts && counts.diff > 0 && (
          <div className="diffs">
            <TriangleAlert size={16} />
            <span>
              <b>{tn(counts.diff, '{n} port', '{n} ports')}</b> {tn(counts.diff, 'is not set up in the switch as in the inventory', 'are not set up in the switch as in the inventory')}
            </span>
            <div className="spacer" />
            <button
              type="button"
              className="btn small"
              onClick={() => {
                setTab('ports');
                setFilter('diff');
              }}
            >
              {t('See the differences')}
            </button>
            {editable && (
              <button type="button" className="btn small" onClick={() => void takeAll()}>
                {t('Take all from the switch')}
              </button>
            )}
          </div>
        )}
        {faults.length > 0 && (
          <div className="diffs">
            <TriangleAlert size={16} />
            <span>
              {faults.map((f, i) => (
                <span key={i}>
                  {i > 0 && ' · '}
                  {f}
                </span>
              ))}
            </span>
            <div className="spacer" />
            <button type="button" className="btn small" onClick={() => setTab('alerts')}>
              {t('See the alerts')}
            </button>
          </div>
        )}
        <SubTabs items={tabs} current={current} onPick={(id) => setTab(id as Tab)} />
        {current === 'ports' && (
          <>
            <div ref={faceRef} className={`sw-face ${live ? 'live-face' : ''} ${marks.length ? 'has-po' : ''}`}>
              {blocks.map((b, i) => (
                <div key={i} className={`sw-block ${b.kind}`}>
                  {b.ports.map((p) => face(p, b.kind))}
                </div>
              ))}
              {marks.map((m) => (
                <div key={m.name} className="po-mark" style={{ left: m.left, width: m.width }}>
                  <span>{m.name}</span>
                </div>
              ))}
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
              {counts && (
                <>
                  <span className="gap">
                    <span className="lk up" />
                    {t('Link up · {n}', { n: counts.up })}
                  </span>
                  <span>
                    <span className="lk" />
                    {t('No link · {n}', { n: sw.ports.length - counts.up })}
                  </span>
                  <span>▬ {t('Port load')}</span>
                </>
              )}
              {facts.lag.size > 0 && <span>⌐ {t('Aggregate')}</span>}
              {facts.blocked.size > 0 && <span className="bad-t">⊘ {t('Blocked by STP')}</span>}
              {sw.ports.some((p) => weak(p.n)) && <span className="warn-t">● {t('Optic out of threshold')}</span>}
              {[...facts.poe.values()].some((x) => x.on || poeFault(x)) && <span>⚡ {t('PoE')}</span>}
            </div>
            <div className="media-toolbar">
              <label className="search">
                <Search size={14} />
                <input value={q} placeholder={info ? t('Port, label, machine, neighbour') : t('Port, label, machine')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
              </label>
              <div className="asr-chips">
                <button className={`asr-chip ${filter === 'all' ? 'on' : ''}`} onClick={() => setFilter('all')}>
                  {t('All ports')}
                  {counts && <span className="asr-chip-n">{sw.ports.length}</span>}
                </button>
                {counts && (
                  <>
                    <button className={`asr-chip ${filter === 'up' ? 'on' : ''}`} onClick={() => setFilter('up')}>
                      {t('Link up')} <span className="asr-chip-n">{counts.up}</span>
                    </button>
                    <button className={`asr-chip ${filter === 'down' ? 'on' : ''}`} onClick={() => setFilter('down')}>
                      {t('No link')} <span className="asr-chip-n">{sw.ports.length - counts.up}</span>
                    </button>
                    {info && (
                      <>
                        <button className={`asr-chip ${filter === 'nb' ? 'on' : ''}`} onClick={() => setFilter('nb')}>
                          {t('Neighbours')} <span className="asr-chip-n">{counts.nb}</span>
                        </button>
                        <button className={`asr-chip ${filter === 'lag' ? 'on' : ''}`} onClick={() => setFilter('lag')}>
                          {t('Aggregates')} <span className="asr-chip-n">{counts.lag}</span>
                        </button>
                      </>
                    )}
                    <button className={`asr-chip ${filter === 'watched' ? 'on' : ''}`} onClick={() => setFilter('watched')}>
                      {tc('ports', 'Watched')} <span className="asr-chip-n">{counts.watched}</span>
                    </button>
                    <button className={`asr-chip ${filter === 'diff' ? 'on' : ''}`} onClick={() => setFilter('diff')}>
                      {t('Differences')} <span className="asr-chip-n">{counts.diff}</span>
                    </button>
                    <button className={`asr-chip ${filter === 'errors' ? 'on' : ''}`} onClick={() => setFilter('errors')}>
                      {t('Errors')} <span className="asr-chip-n">{counts.errors}</span>
                    </button>
                  </>
                )}
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
                {live ? (
                  <colgroup>
                    {editable && <col style={{ width: 30 }} />}
                    <col style={{ width: 28 }} />
                    <col style={{ width: 120 }} />
                    <col style={{ width: '12%' }} />
                    <col style={{ width: '22%' }} />
                    <col style={{ width: 76 }} />
                    <col style={{ width: 150 }} />
                    <col style={{ width: 64 }} />
                    <col style={{ width: isPoe ? '12%' : '13%' }} />
                    <col style={{ width: isPoe ? 110 : 120 }} />
                    <col style={{ width: 34 }} />
                  </colgroup>
                ) : (
                  <colgroup>
                    {editable && <col style={{ width: 36 }} />}
                    <col style={{ width: 120 }} />
                    <col style={{ width: '16%' }} />
                    <col style={{ width: 156 }} />
                    <col style={{ width: '26%' }} />
                    <col style={{ width: '20%' }} />
                    <col />
                  </colgroup>
                )}
                <thead>
                  <tr>
                    {editable && (
                      <th>
                        <input type="checkbox" className="tick" checked={allTicked} title={t('Tick all')} onChange={(e) => tick(ports.map((p) => p.n), e.target.checked)} />
                      </th>
                    )}
                    {live && <th title={tc('port', 'Watched')}>★</th>}
                    <th>{t('Port')}</th>
                    <th>{t('Label')}</th>
                    {!live && <th>{t('Mode')}</th>}
                    <th>{t('VLAN')}</th>
                    {live && (
                      <>
                        <th>{t('Link')}</th>
                        <th>{t('Rate ↓ / ↑')}</th>
                        <th className="num">{t('Errors')}</th>
                        <th>{t('Neighbour (CDP / LLDP)')}</th>
                        <th>{isPoe ? t('PoE') : t('Aggregate · STP')}</th>
                        <th />
                      </>
                    )}
                    {!live && (
                      <>
                        <th>{t('Connected to')}</th>
                        <th>{t('MAC / LLDP')}</th>
                      </>
                    )}
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
                      online={!!live}
                      lp={liveOf.get(p.n)}
                      diff={diffOf.get(p.n)}
                      nb={facts.nb.get(p.n)}
                      lag={facts.lag.get(p.n)}
                      blocked={facts.blocked.has(p.n)}
                      poe={isPoe ? (facts.poe.get(p.n) ?? null) : undefined}
                      open={opened === p.n}
                      cols={cols}
                      onOpen={() => setOpened(opened === p.n ? null : p.n)}
                      onTick={(on) => tick([p.n], on)}
                      onPatch={(set) => onPatch([p.n], set)}
                      onTake={() => onTake([p.n])}
                    />
                  ))}
                </tbody>
              </table>
            )}
            {editable && ticked.size > 0 && <BulkBar sw={sw} vlans={vlans} vlanList={vlanList} ticked={ticked} onClear={() => setTicked(new Set())} onPatch={onPatch} />}
          </>
        )}
        {current === 'optics' && info && <OpticsPane sw={sw} info={info} />}
        {current === 'mcast' && info && <McastPane sw={sw} info={info} vlans={vlans} />}
        {current === 'health' && info && <HealthPane live={live} info={info} />}
        {current === 'alerts' && <AlertsPane sw={sw} alerts={alerts} poe={isPoe} editable={editable} onRules={onRules} />}
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
  online,
  lp,
  diff,
  nb,
  lag,
  blocked,
  poe,
  open,
  cols,
  onOpen,
  onTick,
  onPatch,
  onTake,
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
  online: boolean;
  lp: LivePort | undefined;
  diff: LiveDiff | undefined;
  nb: Neighbour | undefined;
  lag: string | undefined;
  blocked: boolean;
  /** Switch PoE : l'état PoE du port (null : pas de PoE sur ce port) ; undefined : pas de colonne PoE. */
  poe: PoePort | null | undefined;
  open: boolean;
  cols: number;
  onOpen: () => void;
  onTick: (on: boolean) => void;
  onPatch: (set: PortSet) => Promise<void>;
  onTake: () => Promise<void>;
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
  const diffTitle = diff ? `${t('Switch: {s}', { s: diffWords(diff.switch) })} · ${t('Inventory: {s}', { s: diffWords(diff.inventory) })}` : undefined;
  const errors = errorsOf(lp);
  const connect = editable ? (
    (linkValue || p.mode !== 'off') && <Dropdown className="cell" value={linkValue} placeholder={online ? '—' : t('Connect…')} title={online ? t('Connect…') : undefined} options={links} onChange={link} />
  ) : (
    <span className="cell-text">{linkText}</span>
  );
  const row = (
    <tr
      id={`port-${sw.id}-${p.n}`}
      className={`${ticked ? 'sel' : ''} ${p.mode === 'off' ? 'off' : ''} ${trunk ? 'trunk' : ''} ${diff ? 'has-diff' : ''} ${open ? 'open' : ''}`}
      style={p.mode !== 'off' ? vlanStyle(vlan) : undefined}
    >
      {editable && (
        <td>
          <input type="checkbox" className="tick" checked={ticked} aria-label={p.name || String(p.n)} onChange={(e) => onTick(e.target.checked)} />
        </td>
      )}
      {online && (
        <td>
          <button type="button" className={`star ${p.watched ? 'on' : ''}`} disabled={!editable} title={t('Watch this port')} aria-pressed={p.watched} onClick={() => void onPatch({ watched: !p.watched })}>
            ★
          </button>
        </td>
      )}
      <td>
        <b className="ip">{p.n}</b> <span className="ip muted2">{p.name}</span>
      </td>
      <td>
        <InlineText
          value={p.label}
          placeholder={t('Label')}
          editable={editable}
          extra={
            online && lp?.alias ? (
              <span className="descr" title={`${t('Description in the switch')} · ${lp.alias}`}>
                « {lp.alias} »
              </span>
            ) : undefined
          }
          onSave={(label) => void onPatch({ label })}
        />
      </td>
      {!online && <td>{editable ? <Segmented value={p.mode} options={modeOptions()} onChange={(mode) => void onPatch({ mode })} /> : modeLabel(p.mode)}</td>}
      {diff ? (
        <td className="vlan-cell">
          <div className="cell-in">
            {vlanCell}
            <span className="diff" title={diffTitle}>
              ≠ {diffLabel(diff)}
            </span>
            {editable && (
              <button type="button" className="take" title={diffTitle} onClick={() => void onTake()}>
                {t('Take back')}
              </button>
            )}
          </div>
        </td>
      ) : (
        <td>{vlanCell}</td>
      )}
      {online && (
        <>
          <td>
            {lp &&
              (lp.up ? (
                <span className="lk up">{fmtSpeed(lp.speedMbps)}</span>
              ) : !lp.adminUp ? (
                <span className="lk down admin-off">{t('disabled')}</span>
              ) : (
                <span className="lk down">{t('no link')}</span>
              ))}
          </td>
          <td>
            {lp?.up && (
              <div className="rate">
                <LiveSpark a={lp.inHistory} b={lp.outHistory} />
                <div className="io">
                  <span>↓ {fmtRate(lp.inBps)}</span>
                  <span>↑ {fmtRate(lp.outBps)}</span>
                </div>
              </div>
            )}
          </td>
          <td className="num">
            {lp && (
              <span className={`err ${errors ? 'bad' : ''}`} title={t('In {a} · out {b}', { a: fmtCount(lp.inErrors), b: fmtCount(lp.outErrors) })}>
                {fmtCount(errors)}
              </span>
            )}
          </td>
          <td>
            {nb ? (
              <span className="nb" title={[nb.platform, nb.ip, nb.chassisId].filter(Boolean).join(' · ') || undefined}>
                <Link2 size={13} />
                <span className="nb-name">
                  {nb.name} · <span className="ip">{nb.remotePort}</span>
                </span>
                <small>{nb.protocol === 'cdp' ? 'CDP' : 'LLDP'}</small>
              </span>
            ) : linkValue || editable ? (
              connect
            ) : (
              <span className="muted2">—</span>
            )}
          </td>
          <td>
            {poe !== undefined ? (
              poe && (poe.status === 'fault' || poe.status === 'otherFault') ? (
                <span className="tagx blk">{poe.watts ? `${t('Cut')} · ${t('{n} W', { n: fmtW(poe.watts) })}` : t('Cut')}</span>
              ) : poe?.on ? (
                <span className="poe-w">
                  {t('{n} W', { n: fmtW(poe.watts ?? 0) })} {poe.class !== undefined && <span className="muted2">{t('cl. {n}', { n: poe.class })}</span>}
                </span>
              ) : (
                <span className="muted2">—</span>
              )
            ) : (
              <>
                {lag && <span className="tagx">{lag}</span>} {blocked && <span className="tagx blk">{t('STP blocked')}</span>}
              </>
            )}
          </td>
          <td className="exp-cell">
            <button type="button" className="exp" title={t('Rate history')} aria-expanded={open} onClick={onOpen}>
              <ChevronDown size={12} />
            </button>
          </td>
        </>
      )}
      {!online && (
        <>
          <td>{connect}</td>
          <td>
            <span className="learnt" title={p.learnt.join('\n') || undefined}>
              {learnt}
            </span>
          </td>
        </>
      )}
    </tr>
  );
  if (!open || !online) return row;
  return (
    <>
      {row}
      <tr className="chart">
        <td colSpan={cols}>
          <PortChart sw={sw} p={p} />
        </td>
      </tr>
    </>
  );
}

/* ---------- Switch en ligne ---------- */

/** Ce que le serveur lit du switch en ligne : chargé à l'ouverture, puis poussé à chaque lecture. */
function useLive(id: string, online: boolean): Live | null {
  const [live, setLive] = useState<Live | null>(null);
  const load = useCallback(async () => {
    if (!online) return;
    try {
      const l = await api<Live>('GET', `/api/switches/${id}/live`);
      setLive((cur) => (cur && cur.at > l.at ? cur : l));
    } catch {
      /* pas encore en ligne côté serveur : la prochaine lecture arrive par le WebSocket */
    }
  }, [id, online]);
  useEffect(() => {
    setLive(null);
    if (!online) return;
    void load();
    const on = (e: Event) => {
      const m = (e as CustomEvent<{ id: string; live: Live }>).detail;
      if (m.id === id) setLive(m.live);
    };
    serverEvents.addEventListener('switchLive', on);
    return () => serverEvents.removeEventListener('switchLive', on);
  }, [id, online, load]);
  // Aucun « changed » ne porte ce nom : rechargé seulement après une reconnexion (hello).
  useChanged('switchLive', load);
  return online ? live : null;
}

const SECS = [5, 10, 30, 60];

/** L'interrupteur « En ligne », l'âge de la dernière lecture et la fréquence. */
function OnlineBlock({ sw, live, editable }: { sw: Switch; live: Live | null; editable: boolean }) {
  const locked = !editable || !sw.snmp;
  const set = async (body: { online?: boolean; onlineSecs?: number }) => {
    try {
      await api('PATCH', `/api/switches/${sw.id}/online`, body);
    } catch (e) {
      fail(e);
    }
  };
  return (
    <div className={`live ${sw.online ? 'on' : ''}`} title={sw.snmp ? undefined : t('No SNMP agent')}>
      <label className="check">
        <input type="checkbox" checked={sw.online} disabled={locked} onChange={(e) => void set({ online: e.target.checked })} /> {t('Online')}
      </label>
      {sw.online &&
        live &&
        (live.error ? (
          <span className="live-state bad" title={live.error}>
            {t('Read failed')}
          </span>
        ) : (
          live.at > 0 && <ReadAgo at={live.at} />
        ))}
      <Dropdown
        value={String(sw.onlineSecs)}
        title={t('Read interval')}
        disabled={locked}
        options={SECS.map((n) => ({ value: String(n), label: t('{n} s', { n }) }))}
        onChange={(v) => void set({ onlineSecs: Number(v) })}
      />
    </div>
  );
}

/** « lu il y a 4 s », recalculé chaque seconde. */
function ReadAgo({ at }: { at: number }) {
  const [, setTick] = useState(0);
  useEffect(() => {
    const i = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(i);
  }, []);
  const s = Math.max(0, Math.round((serverNow() - at) / 1000));
  return <span className="live-state">{s < 60 ? t('read {n} s ago', { n: s }) : t('read {n} min ago', { n: Math.floor(s / 60) })}</span>;
}

const errorsOf = (lp: LivePort | undefined) => (lp ? lp.inErrors + lp.outErrors : 0);
const fmtCount = (n: number) => n.toLocaleString(locale());
const fmtW = (w: number) => w.toLocaleString(locale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 });

/** Vitesse du lien : « 100M », « 1G », « 2,5G », « 10G ». */
function fmtSpeed(mbps: number): string {
  if (!mbps) return '—';
  return mbps >= 1000 ? `${(mbps / 1000).toLocaleString(locale(), { maximumFractionDigits: 1 })}G` : `${mbps}M`;
}

/** Débit d'un port : « 412 Mb/s », « 2,21 Gb/s », « 300 kb/s ». */
function fmtRate(bps: number): string {
  const num = (v: number, d: number) => v.toLocaleString(locale(), { minimumFractionDigits: d, maximumFractionDigits: d });
  if (bps >= 1e9) return t('{n} Gb/s', { n: num(bps / 1e9, 2) });
  if (bps >= 1e6) return t('{n} Mb/s', { n: num(bps / 1e6, 0) });
  return t('{n} kb/s', { n: num(bps / 1e3, 0) });
}

function linkTitle(lp: LivePort | undefined): string {
  if (!lp || (!lp.up && lp.adminUp)) return t('no link');
  if (!lp.up) return t('disabled');
  return `${t('link {speed}', { speed: fmtSpeed(lp.speedMbps) })} · ↓ ${fmtRate(lp.inBps)} ↑ ${fmtRate(lp.outBps)}`;
}

function diffLabel(d: LiveDiff): string {
  return d.field === 'off' ? t('Disabled') : d.field === 'mode' ? t('Mode') : t('VLAN');
}

/** Courbe du débit (entrée + sortie) des dernières lectures. */
function LiveSpark({ a, b }: { a: number[]; b: number[] }) {
  const W = 48;
  const H = 16;
  const data = a.map((v, i) => v + (b[i] ?? 0));
  if (data.length === 1) data.push(data[0]);
  if (data.length < 2) return <svg width={W} height={H} />;
  const max = Math.max(...data);
  const min = Math.min(...data);
  const y = (v: number) => (max === min ? H / 2 : H - 2 - ((v - min) / (max - min)) * (H - 4));
  const pts = data.map((v, i) => `${((i / (data.length - 1)) * W).toFixed(1)},${y(v).toFixed(1)}`).join(' ');
  return (
    <svg width={W} height={H}>
      <polyline fill="none" stroke="currentColor" strokeWidth="1.5" points={pts} />
    </svg>
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
