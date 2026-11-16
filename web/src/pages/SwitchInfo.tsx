import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Cpu, Fan, MemoryStick, Power, RotateCcw, Search, Thermometer } from 'lucide-react';
import { locale, t, tc, tn } from '../i18n';
import { api, serverEvents, serverNow, useChanged } from '../store';
import { Check, NumberField, Segmented } from '../ui/fields';
import { fmtBytes, fmtDate, fmtDateTime, fmtDuration, fmtHm, fmtWeekday, sameDay, tone } from '../ui/format';
import { ALERT_TONE, alertDetail, alertLabel, fail, fmtBps, portName, signed, unitState, vlanById } from '../net/model';
import { vlanStyle } from '../net/parts';
import type { Alert, AlertRules, InfoPort, Live, Optic, PortHistory, Rule, Switch, SwitchInfo, SwitchPort, Vlan } from '../net/types';

/* ---------- Lectures lentes d'un switch en ligne ---------- */

/** Ce que le serveur lit du switch toutes les minutes : chargé à l'ouverture, puis poussé à chaque lecture. */
export function useInfo(id: string, online: boolean): SwitchInfo | null {
  const [info, setInfo] = useState<SwitchInfo | null>(null);
  const load = useCallback(async () => {
    if (!online) return;
    try {
      setInfo(await api<SwitchInfo>('GET', `/api/switches/${id}/info`));
    } catch {
      /* pas encore en ligne côté serveur : la lecture arrive par le WebSocket */
    }
  }, [id, online]);
  useEffect(() => {
    setInfo(null);
    if (!online) return;
    void load();
    const on = (e: Event) => {
      const m = (e as CustomEvent<{ id: string; info: SwitchInfo }>).detail;
      if (m.id === id) setInfo(m.info);
    };
    serverEvents.addEventListener('switchInfo', on);
    return () => serverEvents.removeEventListener('switchInfo', on);
  }, [id, online, load]);
  // Les switchs reliés changent les voisins à relier.
  useChanged('switches', load);
  return online ? info : null;
}

/** Rien n'a encore été lu (ou plus rien ne répond). */
export const hasHealth = (i: SwitchInfo | null) => !!i && (i.health.cpuPct !== undefined || i.health.memTotalBytes !== undefined || i.health.temps.length + i.health.fans.length + i.health.psus.length > 0);

/** « port-channel1 » → « Po1 ». */
export const shortAgg = (s: string) => s.replace(/^port-?channel\s*/i, 'Po');

/** Le nom d'une interface : celui de l'inventaire pour un port de la façade. */
export const ifName = (sw: Switch, p: InfoPort) => (p.n !== undefined ? portName(sw, p.n) : shortAgg(p.port));

export const weakOptic = (o: Optic, rules: AlertRules) => o.rxDbm !== undefined && o.rxDbm < rules.opticRx.value;

export const poePct = (i: SwitchInfo | null) => {
  const p = i?.poe;
  return p?.budgetW && p.usedW !== undefined ? (p.usedW / p.budgetW) * 100 : null;
};

const num = (v: number, d = 0) => v.toLocaleString(locale(), { maximumFractionDigits: d });
const pct = (v: number) => t('{n} %', { n: num(v) });

function Bar({ value }: { value: number }) {
  return (
    <span className={`perf-bar ${tone(value)}`}>
      <span className="perf-bar-track">
        <span className="perf-bar-fill" style={{ transform: `translateX(${Math.min(100, Math.max(0, value)) - 100}%)` }} />
      </span>
    </span>
  );
}

/** Identité et santé en un coup d'œil. */
export function HealthStrip({ sw, live, info }: { sw: Switch; live: Live; info: SwitchInfo | null }) {
  const sys = live.system;
  const h = info?.health;
  const items: { label: string; value: React.ReactNode; mono?: boolean; title?: string }[] = [
    { label: t('Name in the switch'), value: sys?.name, title: sys?.name },
    { label: t('Firmware'), value: sys?.firmware, mono: true, title: sys?.firmware },
    { label: t('Serial number'), value: sys?.serial, mono: true },
    { label: t('Running for'), value: sys ? fmtDuration(sys.uptimeSecs) : undefined },
  ];
  if (h?.cpuPct !== undefined)
    items.push({
      label: t('CPU'),
      value: (
        <>
          <span>{pct(h.cpuPct)}</span>
          <Bar value={h.cpuPct} />
        </>
      ),
    });
  if (h?.memTotalBytes && h.memUsedBytes !== undefined) {
    const m = (h.memUsedBytes / h.memTotalBytes) * 100;
    items.push({
      label: t('Memory'),
      value: (
        <>
          <span>{pct(m)}</span>
          <Bar value={m} />
        </>
      ),
    });
  }
  if (h && h.temps.length > 0) {
    const hot = Math.max(...h.temps.map((x) => x.c));
    items.push({ label: t('Temperature'), value: <span className={hot > sw.rules.health.value ? 'warn-t' : ''}>{t('{n} °C', { n: num(hot) })}</span> });
  }
  if (h && h.psus.length + h.fans.length > 0) {
    const part = (list: { ok: boolean }[]) => {
      const ok = list.filter((x) => x.ok).length;
      return <span className={ok === list.length ? 'ok-t' : 'warn-t'}>{`${ok}/${list.length}`}</span>;
    };
    items.push({
      label: t('Power supplies · fans'),
      value: (
        <>
          {h.psus.length > 0 && part(h.psus)}
          {h.psus.length > 0 && h.fans.length > 0 && ' · '}
          {h.fans.length > 0 && part(h.fans)}
        </>
      ),
    });
  }
  const pp = poePct(info);
  if (pp !== null && info?.poe) {
    const over = pp > sw.rules.poe.value;
    items.push({
      label: t('PoE budget'),
      value: (
        <>
          <span className={over ? 'warn-t' : ''}>{t('{a} / {b} W', { a: num(info.poe.usedW ?? 0), b: num(info.poe.budgetW ?? 0) })}</span>
          <span className={`perf-bar ${over ? 'tone-warn' : tone(pp)}`}>
            <span className="perf-bar-track">
              <span className="perf-bar-fill" style={{ transform: `translateX(${Math.min(100, pp) - 100}%)` }} />
            </span>
          </span>
        </>
      ),
    });
  }
  return (
    <div className="sys-strip health" style={{ '--cols': items.length } as React.CSSProperties}>
      {items.map((i) => {
        const node = typeof i.value === 'string' || i.value === undefined;
        return (
          <div key={i.label}>
            <span className="set-label">{i.label}</span>
            <b className={node ? (i.mono && i.value ? 'mono' : '') : 'hv'} title={i.title || undefined}>
              {i.value || '—'}
            </b>
          </div>
        );
      })}
    </div>
  );
}

/* ---------- Historique des débits d'un port ---------- */

type Range = '1h' | '24h' | '7d';
let lastRange: Range = '24h';

/** Un débit d'axe : « 25 Gb/s », « 12,5 Gb/s », « 400 Mb/s ». */
function fmtAxis(bps: number): string {
  if (bps >= 1e9) return t('{n} Gb/s', { n: num(bps / 1e9, 1) });
  if (bps >= 1e6) return t('{n} Mb/s', { n: num(bps / 1e6, 1) });
  if (bps >= 1e3) return t('{n} kb/s', { n: num(bps / 1e3, 0) });
  return t('{n} b/s', { n: num(bps, 0) });
}

/** Paquets par seconde : « 0,04 », « 2,5 », « 1 250 ». */
const fmtPps = (v: number) => num(v, v < 1 ? 2 : v < 10 ? 1 : 0);

/** La vitesse du lien en repère : « 25G », « 100M ». */
const speedTag = (mbps: number) => (mbps >= 1000 ? `${num(mbps / 1000, 1)}G` : `${mbps}M`);

/** Le haut d'une échelle : 1, 2, 2,5 ou 5 × 10ⁿ au-dessus de v. */
function niceCeil(v: number): number {
  if (v <= 0) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  const m = [1, 2, 2.5, 5, 10].find((x) => x * p >= v) ?? 10;
  return m * p;
}

/** Un pourcentage d'axe, plus fin sous 10 %. */
const fmtPct = (v: number) => t('{n} %', { n: num(v, v === 0 || v >= 10 ? 0 : v < 1 ? 2 : 1) });

/** La courbe des débits d'un port (entrée en aplat, sortie en trait), la vitesse du lien en repère, et les rejets en dessous. */
export function PortChart({ sw, p, speedMbps }: { sw: Switch; p: SwitchPort; speedMbps?: number }) {
  const [range, setRangeState] = useState<Range>(lastRange);
  const setRange = (r: Range) => {
    lastRange = r;
    setRangeState(r);
  };
  const [h, setH] = useState<PortHistory | null>(null);
  const load = useCallback(async () => {
    try {
      setH(await api<PortHistory>('GET', `/api/switches/${sw.id}/ports/${p.n}/history?range=${range}`));
    } catch (e) {
      fail(e);
    }
  }, [sw.id, p.n, range]);
  useEffect(() => {
    void load();
    const every = range === '1h' ? Math.max(5, sw.onlineSecs) * 1000 : 60_000;
    const i = setInterval(() => void load(), every);
    return () => clearInterval(i);
  }, [load, range, sw.onlineSecs]);

  const W = 1000;
  const TOP = 10;
  const BOTTOM = 110;
  const DH = 36;
  const speed = (speedMbps ?? 0) * 1e6;
  const shapes = useMemo(() => {
    if (!h || h.points.length === 0) return null;
    const span = Math.max(1, h.to - h.from);
    const peak = Math.max(0, ...h.points.map((x) => Math.max(x.in, x.out)));
    // Échelle : la vitesse du lien (ou le pic s'il la dépasse) ; un trafic sous 5 % de la vitesse : à sa propre échelle.
    const full = speed > 0 && peak >= speed * 0.05;
    const max = full ? Math.max(speed, peak) : niceCeil(peak);
    const x = (at: number) => Math.max(0, Math.min(W, ((at - h.from) / span) * W));
    const y = (v: number) => BOTTOM - (v / max) * (BOTTOM - TOP);
    const levels = full ? [0, speed / 2, speed] : [0, max / 2, max];
    const grid = levels.map((v) => ({ y: y(v), left: v === 0 ? '0' : fmtAxis(v), right: speed > 0 ? fmtPct((v / speed) * 100) : '' }));
    if (full && max > speed) grid.push({ y: TOP, left: fmtAxis(max), right: '' });
    // Sans lecture pendant plus de 3,5 pas : la courbe s'interrompt (un débit couvre jusqu'à 3 lectures quand le switch rafraîchit ses compteurs moins souvent).
    const segs: PortHistory['points'][] = [];
    for (const pt of h.points) {
      const last = segs[segs.length - 1];
      if (last && pt.at - last[last.length - 1].at <= h.stepSecs * 3500) last.push(pt);
      else segs.push([pt]);
    }
    const stepW = ((h.stepSecs * 1000) / span) * W;
    const lines = segs.map((s) => {
      const xs = s.map((pt) => x(pt.at));
      if (s.length === 1) {
        s = [s[0], s[0]];
        xs.push(Math.min(W, xs[0] + Math.max(2, stepW)));
      }
      const inLine = s.map((pt, i) => `L${xs[i].toFixed(1)},${y(pt.in).toFixed(1)}`).join(' ');
      return {
        area: `M${xs[0].toFixed(1)},${BOTTOM} ${inLine} L${xs[xs.length - 1].toFixed(1)},${BOTTOM} Z`,
        out: s.map((pt, i) => `${xs[i].toFixed(1)},${y(pt.out).toFixed(1)}`).join(' '),
      };
    });
    // Rejets : sortie (↑) au-dessus du milieu, entrée (↓) en dessous ; les pics des tranches en clair.
    const dMax = Math.max(0, ...h.points.map((pt) => Math.max(pt.inDiscards, pt.outDiscards, pt.inDiscardsPeak ?? 0, pt.outDiscardsPeak ?? 0)));
    const half = DH / 2 - 1;
    const dh = (v: number) => (dMax > 0 ? Math.max(v > 0 ? 1 : 0, (v / dMax) * half) : 0);
    const bw = Math.max(1, stepW * 0.8);
    const bars: { x: number; w: number; y: number; h: number; cls: string }[] = [];
    for (const pt of h.points) {
      const bx = Math.max(0, x(pt.at) - bw / 2);
      const add = (v: number | undefined, up: boolean, cls: string) => {
        const hh = dh(v ?? 0);
        if (hh > 0) bars.push({ x: bx, w: bw, y: up ? DH / 2 - hh : DH / 2, h: hh, cls });
      };
      add(pt.outDiscardsPeak, true, 'dout peak');
      add(pt.inDiscardsPeak, false, 'din peak');
      add(pt.outDiscards, true, 'dout');
      add(pt.inDiscards, false, 'din');
    }
    return { lines, grid, speedY: speed > 0 && speed <= max ? y(speed) : null, bars, dMax };
  }, [h, speed]);

  const ticks = useMemo(() => {
    if (!h) return [];
    const now = h.to;
    return [0, 1, 2, 3, 4].map((k) => {
      const at = h.from + ((h.to - h.from) * k) / 4;
      if (k === 4) return t('now');
      if (range === '7d') return fmtWeekday(at);
      return sameDay(at, now) ? fmtHm(at) : `${t('yesterday')} ${fmtHm(at)}`;
    });
  }, [h, range]);

  const dropping = !!h && (h.inDiscards.peak > 0 || h.outDiscards.peak > 0);
  return (
    <div className="chart-box">
      <div className="chart-head">
        <b>{[p.name, p.label].filter(Boolean).join(' · ')}</b>
        {h && h.points.length > 0 && (
          <>
            <span>↓ {t('average {a} · peak {b}', { a: fmtBps(h.in.avg), b: fmtBps(h.in.peak) })}</span>
            <span>↑ {t('average {a} · peak {b}', { a: fmtBps(h.out.avg), b: fmtBps(h.out.peak) })}</span>
            <span className={dropping ? 'bad-t' : ''} title={t('Peak ↓ {a}/s · ↑ {b}/s', { a: fmtPps(h.inDiscards.peak), b: fmtPps(h.outDiscards.peak) })}>
              {t('discards ↓ {a}/s · ↑ {b}/s', { a: fmtPps(h.inDiscards.avg), b: fmtPps(h.outDiscards.avg) })}
            </span>
          </>
        )}
        <div className="spacer" />
        <Segmented
          value={range}
          options={[
            { value: '1h' as Range, label: t('1 h') },
            { value: '24h' as Range, label: t('24 h') },
            { value: '7d' as Range, label: t('7 d') },
          ]}
          onChange={setRange}
        />
      </div>
      <div className="chart-plot">
        <svg className="chart-svg" viewBox={`0 0 ${W} 120`} preserveAspectRatio="none">
          {(shapes?.grid ?? [{ y: TOP }, { y: 60 }, { y: BOTTOM }]).map((g, i) => (
            <line key={i} className="grid" x1="0" y1={g.y} x2={W} y2={g.y} />
          ))}
          {shapes?.lines.map((s, i) => (
            <g key={i}>
              <path className="in" d={s.area} />
              <polyline className="out" points={s.out} />
            </g>
          ))}
          {shapes?.speedY != null && <line className="speed" x1="0" y1={shapes.speedY} x2={W} y2={shapes.speedY} />}
        </svg>
        {shapes && (
          <>
            <div className="chart-axis left">
              {shapes.grid.map((g, i) => (
                <span key={i} style={{ top: g.y }}>
                  {g.left}
                </span>
              ))}
            </div>
            <div className="chart-axis right">
              {shapes.grid.map((g, i) => (
                <span key={i} style={{ top: g.y }}>
                  {g.right}
                </span>
              ))}
            </div>
            {shapes.speedY != null && speedMbps ? (
              <span className="chart-speed" style={{ top: shapes.speedY }}>
                {speedTag(speedMbps)}
              </span>
            ) : null}
          </>
        )}
      </div>
      <div className="chart-plot">
        <svg className="chart-disc" viewBox={`0 0 ${W} ${DH}`} preserveAspectRatio="none">
          <line className="grid" x1="0" y1={DH / 2} x2={W} y2={DH / 2} />
          {shapes?.bars.map((b, i) => <rect key={i} className={b.cls} x={b.x} y={b.y} width={b.w} height={b.h} />)}
        </svg>
        {shapes && (
          <div className="chart-axis left">
            {shapes.dMax > 0 ? (
              <>
                <span style={{ top: 6 }}>↑ {t('{n}/s', { n: fmtPps(shapes.dMax) })}</span>
                <span style={{ top: DH - 6 }}>↓ {t('{n}/s', { n: fmtPps(shapes.dMax) })}</span>
              </>
            ) : (
              <span style={{ top: DH / 2 }}>{t('{n}/s', { n: 0 })}</span>
            )}
          </div>
        )}
      </div>
      <div className="chart-head chart-ticks">
        {ticks.map((x, i) => (
          <span key={i}>{x}</span>
        ))}
      </div>
    </div>
  );
}

/* ---------- Optiques ---------- */

/** Une puissance optique sur l'échelle −15 … +5 dBm. */
function Dbm({ v, warn }: { v: number | undefined; warn?: boolean }) {
  if (v === undefined) return <span className="muted2">—</span>;
  const at = Math.max(2, Math.min(97, ((v + 15) / 20) * 100));
  return (
    <div className="dbm">
      <span className="scale">
        <i style={{ left: `${at}%` }} />
      </span>
      <span className={warn ? 'warn-t' : ''}>{t('{n} dBm', { n: signed(v) })}</span>
    </div>
  );
}

export function OpticsPane({ sw, info }: { sw: Switch; info: SwitchInfo }) {
  const list = [...info.optics].sort((a, b) => (a.n ?? 1e9) - (b.n ?? 1e9) || a.port.localeCompare(b.port));
  return (
    <table className="pt">
      <colgroup>
        <col style={{ width: 110 }} />
        <col style={{ width: '22%' }} />
        <col style={{ width: 200 }} />
        <col style={{ width: 200 }} />
        <col style={{ width: 90 }} />
        <col style={{ width: 90 }} />
        <col />
      </colgroup>
      <thead>
        <tr>
          <th>{t('Port')}</th>
          <th>{t('Optic')}</th>
          <th>{t('Receive (Rx)')}</th>
          <th>{t('Transmit (Tx)')}</th>
          <th className="num">{t('Temp.')}</th>
          <th className="num">{t('Current')}</th>
          <th>{t('State')}</th>
        </tr>
      </thead>
      <tbody>
        {list.map((o) => {
          const weak = weakOptic(o, sw.rules);
          const what = [o.kind || o.part, o.vendor, o.serial].filter(Boolean).join(' · ');
          return (
            <tr key={o.port}>
              <td>
                {o.n !== undefined && <b className="ip">{o.n}</b>} <span className="ip muted2">{ifName(sw, o)}</span>
              </td>
              <td title={[what, o.part].filter(Boolean).join('\n')}>{what || '—'}</td>
              <td title={o.rxLanesDbm.length > 1 ? o.rxLanesDbm.map((x) => t('{n} dBm', { n: signed(x) })).join(' · ') : undefined}>
                <Dbm v={o.rxDbm} warn={weak} />
              </td>
              <td>
                <Dbm v={o.txDbm} />
              </td>
              <td className="num ip">{o.tempC !== undefined ? t('{n} °C', { n: num(o.tempC) }) : '—'}</td>
              <td className="num ip">{o.biasMa !== undefined ? t('{n} mA', { n: num(o.biasMa, 1) }) : '—'}</td>
              <td>{o.rxDbm === undefined ? <span className="muted2">—</span> : weak ? <span className="st off">{t('Weak reception')}</span> : <span className="st used">{t('OK')}</span>}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

/* ---------- Multicast (IGMP snooping) ---------- */

export function McastPane({ sw, info, vlans }: { sw: Switch; info: SwitchInfo; vlans: Vlan[] | null }) {
  const [q, setQ] = useState('');
  const [vlan, setVlan] = useState<number | null>(null);
  const used = useMemo(() => [...new Set(info.mcast.map((g) => g.vlan))].sort((a, b) => a - b), [info.mcast]);
  const routers = useMemo(() => new Map(info.routerPorts.map((r) => [r.vlan, r.ports])), [info.routerPorts]);
  const needle = q.trim().toLowerCase();
  const rows = info.mcast.filter((g) => {
    if (vlan !== null && g.vlan !== vlan) return false;
    if (!needle) return true;
    return `${g.group ?? ''} ${g.mac} ${g.name ?? ''} ${g.ports.map((p) => ifName(sw, p)).join(' ')}`.toLowerCase().includes(needle);
  });
  const chip = (p: InfoPort, router: boolean) => (
    <span key={p.port} className={`pchip ${router ? 'router' : ''}`}>
      {ifName(sw, p)}
    </span>
  );
  return (
    <>
      <div className="media-toolbar">
        <label className="search">
          <Search size={14} />
          <input value={q} placeholder={t('Group, name, port')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
        </label>
        <div className="asr-chips">
          <button className={`asr-chip ${vlan === null ? 'on' : ''}`} onClick={() => setVlan(null)}>
            {tc('groups', 'All')} <span className="asr-chip-n">{info.mcast.length}</span>
          </button>
          {used.map((v) => (
            <button key={v} className={`asr-chip ${vlan === v ? 'on' : ''}`} onClick={() => setVlan(v)}>
              {t('VLAN {id}', { id: v })} <span className="asr-chip-n">{info.mcast.filter((g) => g.vlan === v).length}</span>
            </button>
          ))}
        </div>
      </div>
      {rows.length === 0 ? (
        <div className="empty">{t('No match')}</div>
      ) : (
        <table className="pt">
          <colgroup>
            <col style={{ width: 150 }} />
            <col style={{ width: '22%' }} />
            <col style={{ width: 180 }} />
            <col />
            <col style={{ width: 170 }} />
          </colgroup>
          <thead>
            <tr>
              <th>{t('Group')}</th>
              <th>{t('Name (SAP announce)')}</th>
              <th>{t('VLAN')}</th>
              <th>{t('Ports receiving it')}</th>
              <th>{t('To the router')}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((g) => {
              const rp = routers.get(g.vlan) ?? [];
              const isRouter = (p: InfoPort) => rp.some((r) => r.port === p.port);
              const receiving = g.ports.filter((p) => !isRouter(p));
              const toRouter = [...g.ports.filter(isRouter), ...rp.filter((r) => !g.ports.some((p) => p.port === r.port))];
              const v = vlanById(vlans, g.vlan);
              return (
                <tr key={`${g.vlan}-${g.mac}`}>
                  <td className="ip" title={g.mac}>
                    {g.group ?? g.mac}
                  </td>
                  <td>{g.name ?? '—'}</td>
                  <td>
                    <span className="vlan-tag" style={vlanStyle(v)}>
                      <span className="vlan-sw" />
                      <b>{g.vlan}</b>
                      {v?.name ? ` ${v.name}` : null}
                    </span>
                  </td>
                  <td>
                    <div className="ports-list" title={receiving.map((p) => ifName(sw, p)).join(' ')}>
                      {receiving.map((p) => chip(p, false))}
                    </div>
                  </td>
                  <td>
                    <div className="ports-list">{toRouter.length ? toRouter.map((p) => chip(p, true)) : '—'}</div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </>
  );
}

/* ---------- Santé ---------- */

function Spark({ points }: { points: [number, number][] }) {
  if (points.length < 2) return null;
  const W = 300;
  const H = 34;
  const max = Math.max(100, ...points.map((p) => p[1]));
  const t0 = points[0][0];
  const span = Math.max(1, points[points.length - 1][0] - t0);
  const pts = points.map(([at, v]) => `${(((at - t0) / span) * W).toFixed(1)},${(H - 2 - (v / max) * (H - 4)).toFixed(1)}`).join(' ');
  return (
    <svg className="spark" width="100%" height={H} viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
      <polyline fill="none" stroke="currentColor" strokeWidth="1.5" points={pts} />
    </svg>
  );
}

const unitRow = (u: { name: string; ok: boolean; detail: string; watts?: number }) => (
  <div key={u.name} className="hrow">
    <span className="grow">{u.name}</span>
    <span className={`st ${u.ok ? 'used' : 'off'}`}>
      {u.ok ? [t('OK'), u.watts !== undefined ? t('{n} W', { n: num(u.watts) }) : ''].filter(Boolean).join(' · ') : `${t('Fault')} · ${unitState(u.detail)}`}
    </span>
  </div>
);

export function HealthPane({ live, info }: { live: Live | null; info: SwitchInfo }) {
  const h = info.health;
  const cpu = h.cpu.map((x) => x[1]);
  const avg = cpu.length ? cpu.reduce((a, b) => a + b, 0) / cpu.length : null;
  const boot = live?.system ? serverNow() - live.system.uptimeSecs * 1000 : null;
  return (
    <div className="health-grid">
      {h.cpuPct !== undefined && (
        <div className="hcard">
          <h3>
            <Cpu size={14} /> {t('CPU')}
          </h3>
          <span className="big">{pct(h.cpuPct)}</span>
          <Spark points={h.cpu} />
          {avg !== null && cpu.length > 1 && <span className="muted">{t('24 h average: {a} · peak {b}', { a: pct(avg), b: pct(Math.max(...cpu)) })}</span>}
        </div>
      )}
      {h.memTotalBytes !== undefined && h.memUsedBytes !== undefined && (
        <div className="hcard">
          <h3>
            <MemoryStick size={14} /> {t('Memory')}
          </h3>
          <span className="big">{pct((h.memUsedBytes / h.memTotalBytes) * 100)}</span>
          <span className="muted">{t('{a} of {b}', { a: fmtBytes(h.memUsedBytes), b: fmtBytes(h.memTotalBytes) })}</span>
        </div>
      )}
      {h.psus.length > 0 && (
        <div className="hcard">
          <h3>
            <Power size={14} /> {t('Power supplies')}
          </h3>
          {h.psus.map(unitRow)}
        </div>
      )}
      {h.temps.length > 0 && (
        <div className="hcard">
          <h3>
            <Thermometer size={14} /> {t('Temperatures')}
          </h3>
          {h.temps.map((x) => (
            <div key={x.name} className="hrow">
              <span className="grow">{x.name}</span>
              <span className="mono">{t('{n} °C', { n: num(x.c) })}</span>
            </div>
          ))}
        </div>
      )}
      {h.fans.length > 0 && (
        <div className="hcard">
          <h3>
            <Fan size={14} /> {t('Fans')}
          </h3>
          {h.fans.map(unitRow)}
        </div>
      )}
      {boot !== null && (
        <div className="hcard">
          <h3>
            <RotateCcw size={14} /> {t('Last restart')}
          </h3>
          <span className="big date">{`${fmtDate(boot)}, ${fmtHm(boot)}`}</span>
        </div>
      )}
    </div>
  );
}

/* ---------- Alertes : règles et historique ---------- */

type RuleKey = keyof AlertRules;

export function AlertsPane({
  sw,
  alerts,
  poe,
  editable,
  onRules,
}: {
  sw: Switch;
  alerts: Alert[];
  poe: boolean;
  editable: boolean;
  onRules: (r: AlertRules) => Promise<void>;
}) {
  const [rules, setRules] = useState(sw.rules);
  const pending = useRef<number | null>(null);
  useEffect(() => {
    if (pending.current === null) setRules(sw.rules);
  }, [sw.rules]);
  useEffect(() => () => void (pending.current !== null && clearTimeout(pending.current)), []);
  const set = (k: RuleKey, r: Partial<Rule>) => {
    const next = { ...rules, [k]: { ...rules[k], ...r } };
    setRules(next);
    if (pending.current !== null) clearTimeout(pending.current);
    // Les nombres changent par à-coups (molette) : un seul envoi.
    pending.current = window.setTimeout(() => {
      pending.current = null;
      void onRules(next);
    }, 400);
  };
  const watched = sw.ports.filter((p) => p.watched).length;
  const row = (k: RuleKey, title: string, mid: React.ReactNode) => (
    <div key={k} className="rule">
      <div>
        <b>{title}</b>
      </div>
      {mid}
      <Check checked={rules[k].on} disabled={!editable} onChange={(on) => set(k, { on })} />
    </div>
  );
  const numIn = (k: RuleKey, before: string, after: string, opts: { min: number; max: number; precision?: number }) => (
    <div className="num-in">
      {before && <span>{before}</span>}
      {editable ? (
        <NumberField value={rules[k].value} min={opts.min} max={opts.max} precision={opts.precision ?? 0} onChange={(value) => set(k, { value })} />
      ) : (
        <b className="mono">{signed(rules[k].value, opts.precision ?? 0)}</b>
      )}
      {after && <span>{after}</span>}
    </div>
  );
  const sorted = [...alerts].sort((a, b) => b.at - a.at);
  return (
    <>
      <div className="rules">
        {row('linkDown', t('Link down on a watched port'), <div className="muted">{tn(watched, '★ in the list of ports · {n} port', '★ in the list of ports · {n} ports')}</div>)}
        {row('flap', t('Unstable port'), numIn('flap', t('more than'), t('drops in 10 min'), { min: 1, max: 1000 }))}
        {row('errors', t('Errors on a port'), numIn('errors', t('more than'), t('per minute'), { min: 1, max: 1e9 }))}
        {row('discards', t('Discards on a port'), numIn('discards', t('more than'), t('packets/s for 1 min'), { min: 0.1, max: 1e9, precision: 1 }))}
        {row('opticRx', t('Weak optical reception'), numIn('opticRx', t('below'), t('dBm'), { min: -40, max: 10 }))}
        {row('saturation', t('Saturated port'), numIn('saturation', t('above'), t('% for 1 min'), { min: 1, max: 100 }))}
        {row('health', t('Power supply, fan or temperature'), numIn('health', t('above'), t('°C'), { min: 20, max: 150 }))}
        {poe && row('poe', t('PoE budget'), numIn('poe', t('above'), t('%'), { min: 1, max: 100 }))}
        {row('drift', t('New setup difference'), <div className="muted">{t('VLAN, mode or disabled port')}</div>)}
        {row('neighbours', t('New or gone neighbour'), <div className="muted">{t('CDP / LLDP')}</div>)}
      </div>
      {sorted.length > 0 && (
        <table className="pt">
          <colgroup>
            <col style={{ width: 150 }} />
            <col style={{ width: 200 }} />
            <col style={{ width: 110 }} />
            <col />
          </colgroup>
          <thead>
            <tr>
              <th>{t('Date')}</th>
              <th>{t('Alert')}</th>
              <th>{t('Port')}</th>
              <th>{t('Detail')}</th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((a) => (
              <tr key={a.id}>
                <td>{fmtDateTime(a.at)}</td>
                <td>
                  <span className={`kind ${ALERT_TONE[a.kind]}`}>{alertLabel(a.kind)}</span>
                </td>
                <td className={a.port !== undefined ? 'ip' : ''}>{a.port !== undefined ? portName(sw, a.port) : '—'}</td>
                <td title={a.detail}>{alertDetail(a)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}
