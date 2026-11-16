import { Fragment, useCallback, useLayoutEffect, useRef, useState } from 'react';
import { Activity, ChevronDown, EthernetPort, FileText, GitCompareArrows, Pencil, RefreshCw, Users } from 'lucide-react';
import { t, tn } from '../../i18n';
import { api, ApiError, can, useApp } from '../../store';
import { alertDialog, promptDialog } from '../../ui/dialogs';
import { fail, fmtBps, fmtInt, fmtSeen } from '../../net/model';
import { attachText, flowRate, formatText, imageText, shortPort, toneLabel, toneOf, type Flow, type Flows, type Receiver, type Tone } from '../../net/mcast';
import { toneIcon } from './icons';
import { originLabel } from '../../net/nmos';

/* ---------- Petits éléments ---------- */

export function TypeChip({ f }: { f: Flow }) {
  const k = toneOf(f);
  return <span className={`tchip ${k}`}>{toneLabel(k)}</span>;
}

export function ToneIcon({ k, size }: { k: Tone; size: number }) {
  const Icon = toneIcon(k);
  return <Icon size={size} />;
}

/** R / B : les deux jambes ST 2022-7, barrée celle qui manque. */
export function PairBadge({ f }: { f: Flow }) {
  if (!f.pairState || !f.leg) return <span className="muted2">—</span>;
  const missing = f.pairState === 'missing';
  // La jambe absente : l'autre si celle-ci vit, sinon celle-ci.
  const other = f.leg === 'a' ? 'b' : 'a';
  const gone = missing ? (f.live ? other : f.leg) : null;
  return (
    <span className="pair" title={f.pair ? `${f.pair.group}:${f.pair.port}${f.pair.name ? ` · ${f.pair.name}` : ''}` : undefined}>
      <i className={gone === 'a' ? 'x' : 'r'}>{t('pair|R')}</i>
      <i className={gone === 'b' ? 'x' : 'b'}>{t('pair|B')}</i>
    </span>
  );
}

/** « mixer-a », sinon « SW-B Eth1/9 ». */
export const receiverName = (r: Receiver) => r.host || r.label || `${r.switchName} ${shortPort(r.port)}`;
const receiverChip = (r: Receiver) => (r.router || r.peer ? `${r.switchName} ${shortPort(r.agg ?? r.port)}` : r.host || `${r.switchName} ${shortPort(r.port)}`);

/** Les puces des récepteurs, autant que la cellule en tient, puis « +N ». */
export function ReceiverChips({ f }: { f: Flow }) {
  const ref = useRef<HTMLDivElement>(null);
  const igmp = [...f.receivers]
    .sort((a, b) => Number(a.router || !!a.peer) - Number(b.router || !!b.peer))
    .map((r) => ({ key: `${r.switchId}|${r.port}`, text: receiverChip(r), title: `${r.switchName} ${r.port}${r.label ? ` · ${r.label}` : ''}`, router: r.router }));
  // Les abonnés NMOS que les tables IGMP ne montrent pas.
  const subs = (f.subscribers ?? [])
    .filter((s) => !s.igmp)
    .map((s) => ({ key: `nmos|${s.id}`, text: s.node || s.label, title: [s.label, attachText(s.at)].filter(Boolean).join(' · '), router: false }));
  const list = [...igmp, ...subs];
  const [fit, setFit] = useState(list.length);
  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const ws = [...el.querySelectorAll<HTMLElement>(':scope > .pchip:not(.more)')].map((c) => c.offsetWidth);
    const avail = el.clientWidth;
    let k = ws.length;
    let used = ws.reduce((s, w, i) => s + w + (i ? 4 : 0), 0);
    while (k > 0 && used + (k < ws.length ? 40 : 0) > avail) {
      k--;
      used -= ws[k] + (k ? 4 : 0);
    }
    setFit(k);
  }, []);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure, f.receivers, f.subscribers]);
  const rest = list.slice(fit);
  return (
    <div className="ports-list" ref={ref}>
      {list.map((r, i) => (
        <span key={r.key} className={`pchip ${r.router ? 'router' : ''} ${i >= fit ? 'hide' : ''}`} title={r.title}>
          {r.text}
        </span>
      ))}
      {rest.length > 0 && (
        <span className="pchip more" title={rest.map((r) => r.text).join('\n')}>
          +{rest.length}
        </span>
      )}
    </div>
  );
}

/* ---------- Actions ---------- */

export function useFlowActions() {
  const toast = useApp((s) => s.toast);
  const [checking, setChecking] = useState<string | null>(null);
  const check = async (f: Flow) => {
    setChecking(f.key);
    const where = `${f.group}:${f.port}`;
    try {
      const r = await api<{ received: boolean; cutShort: boolean; bps: number; pps: number }>('POST', '/api/multicast/check', { group: f.group, port: f.port });
      if (!r.received) toast(t('{stream}: nothing received', { stream: where }), 'error');
      else if (r.cutShort) toast(t('{stream}: received, stopped above {rate}', { stream: where, rate: fmtBps(r.bps) }));
      else toast(t('{stream}: received · {rate} · {pps} packets/s', { stream: where, rate: fmtBps(r.bps), pps: fmtInt(r.pps) }));
    } catch (e) {
      if (e instanceof ApiError && e.status === 409 && /already running/.test(e.message)) toast(t('A check is already running'), 'error');
      else fail(e);
    }
    setChecking(null);
  };
  const sdp = async (f: Flow) => {
    try {
      const res = await fetch(`/api/multicast/sdp/${f.group}/${f.port}`, { credentials: 'same-origin' });
      if (!res.ok) throw new ApiError(res.status, (await res.text()) || res.statusText);
      const text = await res.text();
      await alertDialog({ title: t('SDP of {stream}', { stream: `${f.group}:${f.port}` }), message: <pre className="sdp-box">{text.replace(/\r/g, '')}</pre> });
    } catch (e) {
      fail(e);
    }
  };
  const rename = async (f: Flow) => {
    const label = await promptDialog({ title: t('Name the group {group}', { group: f.group }), label: t('Name'), value: f.groupLabel, confirm: t('Save') });
    if (label === null) return;
    try {
      await api('PUT', `/api/multicast/labels/${f.group}`, { label });
    } catch (e) {
      fail(e);
    }
  };
  return { check, sdp, rename, checking };
}

export function FlowButtons({ f, checkOn, onPair }: { f: Flow; checkOn: boolean; onPair?: (key: string) => void }) {
  const user = useApp((s) => s.auth?.user);
  const { check, sdp, rename, checking } = useFlowActions();
  return (
    <>
      {checkOn && can.edit(user) && f.port > 0 && (
        <button className="btn small" disabled={checking !== null} onClick={() => void check(f)}>
          {checking === f.key ? <RefreshCw size={13} className="spin" /> : <Activity size={13} />} {t('Measure 2 s')}
        </button>
      )}
      {f.announced && (
        <button className="btn small" onClick={() => void sdp(f)}>
          <FileText size={13} /> {t('SDP')}
        </button>
      )}
      {can.edit(user) && !f.announced && (
        <button className="btn small" onClick={() => void rename(f)}>
          <Pencil size={13} /> {t('Name')}
        </button>
      )}
      {onPair && f.pair && (
        <button className="btn small" onClick={() => onPair(f.pair!.key)}>
          <GitCompareArrows size={13} /> {f.leg === 'a' ? t('Show the Blue stream') : t('Show the Red stream')}
        </button>
      )}
    </>
  );
}

/** Format, débit, horloge, autre jambe, annonce, chemin : les faits d'un flux. */
export function FlowFacts({ f, data }: { f: Flow; data: Flows }) {
  const m = f.format;
  const gmName = m?.ptpGm ? (data.ptp.clocks.find((c) => c.gm === m.ptpGm)?.name ?? m.ptpGm) : undefined;
  const clock = m?.ptpGm || m?.ptpDomain !== undefined ? [t('PTP'), gmName, m?.ptpDomain].filter((x) => x !== undefined && x !== '').join(' · ') : m?.tsRefclk;
  const rate = [f.estBps ? t('{rate} (SDP)', { rate: fmtBps(f.estBps) }) : '', f.bps > 0 ? t('{rate} measured', { rate: fmtBps(f.bps) }) : ''].filter(Boolean).join(' · ');
  const hops = [f.emitter?.at ? shortPort(f.emitter.at.port) : '', ...f.path.filter((n) => n.egress).map((n) => shortPort(n.egress!))].filter(Boolean).join(' → ');
  const enc = [f.encoding, m?.sampling, m?.depth && toneOf(f) === 'v' ? t('{n} bits', { n: m.depth }) : ''].filter(Boolean).join(' · ');
  const rows: [string, string][] = [
    [t('Announcement'), [f.nmos ? t('NMOS · {name}', { name: f.nmos.node ?? f.nmos.sender }) : '', f.lastAnnounced ? t('SAP · {ago}', { ago: fmtSeen(f.lastAnnounced) }) : ''].filter(Boolean).join(' · ') || t('none')],
    [t('Encoding'), enc || formatText(f)],
    [toneOf(f) === 'v' ? t('Image') : t('Format'), imageText(m) || formatText(f)],
    [t('Clock'), clock ?? ''],
    [t('Rate'), rate],
    [f.leg === 'b' ? t('Red') : t('Blue'), f.pair ? [`${f.pair.group}:${f.pair.port}`, attachText(f.pair.at)].filter(Boolean).join(' · ') : ''],
    [t('Path'), hops],
  ];
  return (
    <div className="kv">
      {rows
        .filter(([, v]) => v)
        .map(([k, v]) => (
          <Fragment key={k}>
            <span>{k}</span>
            <b title={v}>{v}</b>
          </Fragment>
        ))}
    </div>
  );
}

/** Émetteur → switchs → récepteurs, en ligne. */
export function PathChain({ f }: { f: Flow }) {
  const edge = [
    ...f.receivers.filter((r) => !r.router && !r.peer).map((r) => ({ key: `${r.switchId}|${r.port}`, name: receiverName(r), port: shortPort(r.port), title: `${r.switchName} ${r.port}` })),
    // Les abonnés NMOS absents des tables IGMP.
    ...(f.subscribers ?? []).filter((s) => !s.igmp).map((s) => ({ key: `nmos|${s.id}`, name: s.node || s.label, port: s.at ? shortPort(s.at.port) : '', title: [s.label, attachText(s.at)].filter(Boolean).join(' · ') })),
  ];
  const Icon = toneIcon(toneOf(f));
  return (
    <div className="path">
      {f.emitter && (
        <div className="node src-n">
          <b>
            <Icon size={13} /> {f.emitter.name ?? f.emitter.ip}
          </b>
          <span className="ip">{t('{ip} · emitter', { ip: f.emitter.ip })}</span>
        </div>
      )}
      {f.path.map((n, i) => (
        <Fragment key={n.switchId}>
          {(i > 0 || f.emitter) && <div className="wire">{i === 0 ? shortPort(n.ingress ?? '') : shortPort(n.egress ?? '')}</div>}
          <div className="node">
            <b>
              <EthernetPort size={13} /> {n.switchName}
            </b>
            <span className="ip">{tn(n.igmpPorts, 'IGMP: {n} port', 'IGMP: {n} ports')}</span>
          </div>
        </Fragment>
      ))}
      {edge.length > 0 && (
        <>
          {(f.path.length > 0 || f.emitter) && <div className="wire">{[...new Set(edge.map((r) => r.port).filter(Boolean))].slice(0, 3).join(' · ')}</div>}
          <div className="node rcv">
            <b>
              <Users size={13} /> {tn(edge.length, '{n} receiver', '{n} receivers')}
            </b>
            <div className="pl">
              {edge.slice(0, 12).map((r) => (
                <span key={r.key} className="pchip" title={r.title}>
                  {r.name}
                </span>
              ))}
              {edge.length > 12 && <span className="pchip">+{edge.length - 12}</span>}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

/* ---------- Mosaïque : une carte par émetteur ---------- */

const bar = (bps: number) => (bps > 0 ? Math.max(2, Math.min(100, (Math.log10(bps / 1e6 + 1) / Math.log10(3001)) * 100)) : 0);

export function Mosaic({ flows, onPick }: { flows: Flow[]; onPick: (key: string) => void }) {
  const groups = new Map<string, Flow[]>();
  for (const f of flows) {
    const k = f.emitter?.ip ?? f.source ?? '';
    groups.set(k, [...(groups.get(k) ?? []), f]);
  }
  return (
    <div className="mosaic">
      {[...groups].map(([ip, list]) => {
        const e = list[0].emitter;
        const allOff = list.every((f) => f.state === 'off');
        const warn = list.some((f) => f.state === 'warn' || f.state === 'unk');
        const cls = list.some((f) => f.conflict) ? 'warnb' : list.every((f) => !f.announced) ? 'unk' : allOff ? 'off' : '';
        return (
          <div key={ip || 'none'} className={`mc ${cls}`}>
            <div className="mc-h">
              <span className="ic">
                <ToneIcon k={toneOf(list[0])} size={14} />
              </span>
              <span>
                <b>{e?.name ?? (ip || t('Unknown source'))}</b>
                <small>{[ip, attachText(e?.at)].filter(Boolean).join(' · ')}</small>
              </span>
              <span className={`glow ${allOff ? 'o' : warn ? 'w' : ''}`} />
            </div>
            {list.map((f) => {
              const k = toneOf(f);
              const r = flowRate(f);
              return (
                <div key={f.key} className="mf" role="button" tabIndex={0} onClick={() => onPick(f.key)} onKeyDown={(ev) => ev.key === 'Enter' && onPick(f.key)}>
                  <span className={`t ${k}`}>
                    <ToneIcon k={k} size={12} />
                  </span>
                  <span className="nm">
                    <div>{f.name ?? <span className="muted2">{t('Unannounced')}</span>}</div>
                    <small>{f.sources && f.sources.length > 1 ? t('{group} · {n} sources', { group: f.group, n: f.sources.length }) : f.group}</small>
                  </span>
                  <span className="bar">
                    {f.live && r > 0 ? fmtBps(r) : '—'}
                    <span className="tr">
                      <i className={`c-${k}`} style={{ width: `${f.live ? bar(r) : 0}%` }} />
                    </span>
                  </span>
                  <span className={`rcv ${f.receiverCount ? '' : 'z'}`} title={t('Receivers')}>
                    {f.receiverCount}
                  </span>
                  <PairBadge f={f} />
                </div>
              );
            })}
          </div>
        );
      })}
    </div>
  );
}

/* ---------- Liste ---------- */

const DOT: Record<Flow['state'], string> = { ok: 'ok', warn: 'warn', unk: 'warn', off: 'free' };

export function FlowList({ flows, data, open, onOpen }: { flows: Flow[]; data: Flows; open: string | null; onOpen: (key: string | null) => void }) {
  return (
    <table className="pt">
      <colgroup>
        <col style={{ width: 26 }} />
        <col style={{ width: 150 }} />
        <col style={{ width: '17%' }} />
        <col style={{ width: 84 }} />
        <col style={{ width: 160 }} />
        <col style={{ width: '17%' }} />
        <col style={{ width: '15%' }} />
        <col style={{ width: 96 }} />
        <col style={{ width: 70 }} />
        <col style={{ width: 104 }} />
        <col style={{ width: 42 }} />
      </colgroup>
      <thead>
        <tr>
          <th />
          <th>{t('Group')}</th>
          <th>{t('Name')}</th>
          <th>{t('Type')}</th>
          <th>{t('Format')}</th>
          <th>{t('Emitter')}</th>
          <th>{t('Receivers')}</th>
          <th className="num">{t('Rate')}</th>
          <th>{t('2022-7')}</th>
          <th>{t('Origin')}</th>
          <th />
        </tr>
      </thead>
      <tbody>
        {flows.map((f) => {
          const on = open === f.key;
          const r = flowRate(f);
          return (
            <Fragment key={f.key}>
              <tr className={`${on ? 'open' : ''} ${f.live ? '' : 'flow-off'}`} onClick={() => onOpen(on ? null : f.key)}>
                <td className="st-cell">
                  <span className={`nav-dot ${DOT[f.state]}`} style={{ display: 'inline-block' }} />
                </td>
                <td className="ip">{f.port ? `${f.group}:${f.port}` : f.group}</td>
                <td title={f.sapName && f.name !== f.sapName ? f.sapName : undefined}>
                  {f.name ?? <span className="muted2">—</span>}
                  {!f.announced && <span className="unk">{t('Unannounced')}</span>}
                </td>
                <td>
                  <TypeChip f={f} />
                </td>
                <td className="muted">{formatText(f)}</td>
                <td>
                  {f.sources && f.sources.length > 1 ? (
                    <span className="src">{t('{n} sources', { n: f.sources.length })}</span>
                  ) : f.emitter ? (
                    <span className="src" title={f.emitter.ip}>
                      {f.emitter.name ?? f.emitter.ip}
                      <small>{attachText(f.emitter.at) || (f.nmos && !f.nmos.inventory ? t('not in inventory') : '')}</small>
                    </span>
                  ) : (
                    <span className="muted2">—</span>
                  )}
                </td>
                <td>
                  <div className="rx">
                    <b className={f.receiverCount ? '' : 'warn-t'}>{f.receiverCount}</b>
                    <ReceiverChips f={f} />
                  </div>
                </td>
                <td className="num ip" title={f.estBps ? t('{rate} (SDP)', { rate: fmtBps(f.estBps) }) : undefined}>
                  {f.live && r > 0 ? fmtBps(r) : '—'}
                </td>
                <td>
                  <PairBadge f={f} />
                </td>
                <td>
                  {f.origins.map((o) => (
                    <span key={o} className={`srcb ${o}`}>
                      {originLabel(o)}
                    </span>
                  ))}
                </td>
                <td className="exp-cell">
                  <button className="exp" aria-expanded={on} title={on ? t('Close') : t('Details')} onClick={(e) => (e.stopPropagation(), onOpen(on ? null : f.key))}>
                    <ChevronDown size={12} />
                  </button>
                </td>
              </tr>
              {on && (
                <tr className="detail">
                  <td colSpan={11}>
                    <div className="det">
                      <PathChain f={f} />
                      <div className="det-actions">
                        <FlowButtons f={f} checkOn={data.check.enabled} onPair={onOpen} />
                      </div>
                      <FlowFacts f={f} data={data} />
                    </div>
                  </td>
                </tr>
              )}
            </Fragment>
          );
        })}
      </tbody>
    </table>
  );
}

