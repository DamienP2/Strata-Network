import { useMemo, useState } from 'react';
import { Radio, RefreshCw, Search, ShieldCheck, Waves } from 'lucide-react';
import { t, tn } from '../i18n';
import { api, ApiError, can, useApp } from '../store';
import { TextInput } from '../ui/fields';
import { Dropdown, type DropOption } from '../ui/Dropdown';
import { SubTabs } from './SidePage';
import { byIp } from '../net/ip';
import { fail, fmtBps, fmtInt, fmtSeen, streamKey, streamLive, streamTone, streamType, useAllAddresses, useList, vlanOfIp } from '../net/model';
import { VlanTag } from '../net/parts';
import type { MulticastState, Network, Service, Stream, Vlan } from '../net/types';

type Chip = 'all' | 'video' | 'audio' | 'ts' | 'ptp' | 'stopped';

export function Multicast() {
  const user = useApp((s) => s.auth?.user);
  const [state] = useList<MulticastState>('multicast', '/api/multicast');
  const [tab, setTab] = useState<'streams' | 'services'>('streams');
  const [services, reloadServices] = useList<{ at: number; services: Service[] }>('services', '/api/services');

  const setIface = async (iface: string) => {
    if (!state) return;
    try {
      await api('PUT', '/api/multicast/settings', { iface, checkMaxMbps: state.checkMaxMbps });
    } catch (e) {
      fail(e);
    }
  };

  if (!state) return <div className="loading">{t('Loading…')}</div>;
  // Le serveur garde le nom ou l'adresse de l'interface.
  const nic = state.ifaces.find((i) => i.name === state.iface || i.ip === state.iface);
  const ifaceOptions: DropOption[] = state.ifaces.map((i) => ({ value: i.name, label: i.name, detail: i.ip }));
  return (
    <main className="library">
      <div className="library-inner wide">
        <div className="settings-head">
          <div>
            <h1>{t('Multicast')}</h1>
            <p className="muted">
              {tn(state.streams.length, '{n} stream', '{n} streams')}
              {nic && (
                <>
                  {' · '}
                  <span className="mono">{nic.name}</span> (<span className="mono">{nic.ip}</span>)
                </>
              )}
            </p>
          </div>
          <div className="spacer" />
          {can.admin(user) && (
            <div style={{ width: 200 }}>
              <Dropdown value={nic?.name ?? null} options={ifaceOptions} title={t('Interface')} onChange={(v) => void setIface(v)} className="iface-dd" />
            </div>
          )}
        </div>
        <SubTabs
          current={tab}
          onPick={(v) => setTab(v as 'streams' | 'services')}
          items={[
            { id: 'streams', icon: <Radio size={14} />, label: t('Streams'), count: state.streams.length },
            { id: 'services', icon: <Waves size={14} />, label: t('Services'), count: services?.services.length },
          ]}
        >
          {tab === 'services' && can.edit(user) && <BrowseButton onDone={() => void reloadServices()} />}
        </SubTabs>
        {tab === 'streams' ? <Streams streams={state.streams} editable={can.edit(user)} /> : <Services data={services} />}
      </div>
    </main>
  );
}

function Streams({ streams, editable }: { streams: Stream[]; editable: boolean }) {
  const toast = useApp((s) => s.toast);
  const [vlans] = useList<Vlan[]>('vlans', '/api/vlans');
  const [networks] = useList<Network[]>('networks', '/api/networks');
  const addrs = useAllAddresses(networks);
  const [q, setQ] = useState('');
  const [chip, setChip] = useState<Chip>('all');
  const [sel, setSel] = useState<string | null>(null);
  const [checking, setChecking] = useState<string | null>(null);

  const tone = (s: Stream): Chip | '' => (s.kind === 'video' ? 'video' : s.kind === 'audio' ? 'audio' : s.kind === 'ptp' ? 'ptp' : streamTone(s) === 'ts' ? 'ts' : '');
  const counts = useMemo(() => {
    const c: Record<Chip, number> = { all: streams.length, video: 0, audio: 0, ts: 0, ptp: 0, stopped: 0 };
    for (const s of streams) {
      const k = tone(s);
      if (k) c[k]++;
      if (!streamLive(s)) c.stopped++;
    }
    return c;
  }, [streams]);

  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return streams
      .filter((s) => (chip === 'all' ? true : chip === 'stopped' ? !streamLive(s) : tone(s) === chip))
      .filter((s) => !needle || `${s.group}:${s.port} ${s.name ?? ''} ${s.label} ${s.source ?? ''} ${s.source ? (addrs.get(s.source)?.name ?? '') : ''}`.toLowerCase().includes(needle))
      .sort((a, b) => Number(streamLive(b)) - Number(streamLive(a)) || byIp(a.group, b.group) || a.port - b.port);
  }, [streams, q, chip, addrs]);

  const check = async (s: Stream) => {
    setChecking(streamKey(s));
    try {
      const r = await api<{ received: boolean; cutShort: boolean; bps: number; pps: number }>('POST', '/api/multicast/check', { group: s.group, port: s.port });
      const where = `${s.group}:${s.port}`;
      if (!r.received) toast(t('{stream}: nothing received', { stream: where }), 'error');
      else if (r.cutShort) toast(t('{stream}: received, stopped above {rate}', { stream: where, rate: fmtBps(r.bps) }));
      else toast(t('{stream}: received · {rate} · {pps} packets/s', { stream: where, rate: fmtBps(r.bps), pps: fmtInt(r.pps) }));
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) toast(t('A check is already running'), 'error');
      else fail(e);
    }
    setChecking(null);
  };

  const setLabel = async (s: Stream, label: string) => {
    try {
      await api('PATCH', '/api/multicast/label', { group: s.group, port: s.port, source: s.source, label });
    } catch (e) {
      fail(e);
    }
  };

  const chips: [Chip, string][] = [
    ['all', t('All streams')],
    ['video', t('Video')],
    ['audio', t('Audio')],
    ['ts', t('MPEG-TS')],
    ['ptp', t('PTP')],
    ['stopped', t('Stopped')],
  ];
  return (
    <>
      <div className="media-toolbar">
        <label className="search">
          <Search size={14} />
          <input value={q} placeholder={t('Search a stream')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
        </label>
        <div className="asr-chips">
          {chips.map(([id, label]) => (
            <button key={id} className={`asr-chip ${chip === id ? 'on' : ''}`} onClick={() => setChip(id)}>
              {label} <span className="asr-chip-n">{counts[id]}</span>
            </button>
          ))}
        </div>
      </div>
      {shown.length === 0 ? (
        <div className="empty big">
          <Radio size={22} />
          <p>{q || chip !== 'all' ? t('No match') : t('No stream')}</p>
        </div>
      ) : (
        <div className="card-table">
          <table className="data-table">
            <thead>
              <tr>
                <th />
                <th>{t('Group')}</th>
                <th>{t('Name')}</th>
                <th>{t('Source')}</th>
                <th>{t('Type')}</th>
                <th>{t('VLAN')}</th>
                <th className="num">{t('Rate')}</th>
                <th className="num">{t('Packets/s')}</th>
                <th>{t('Seen')}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {shown.map((s) => {
                const key = streamKey(s);
                const live = streamLive(s);
                const src = s.source ? addrs.get(s.source) : undefined;
                const v = vlanOfIp(vlans, s.source);
                const on = sel === key;
                return (
                  <tr key={key} className={`${live ? '' : 'free'} ${on ? 'sel' : ''}`} onClick={() => setSel(key)}>
                    <td>
                      <span className={`nav-dot ${live ? 'ok' : ''}`} />
                    </td>
                    <td className="ip">{s.port ? `${s.group}:${s.port}` : s.group}</td>
                    {on && editable ? (
                      <td onClick={(e) => e.stopPropagation()}>
                        <TextInput className="cell-edit" value={s.label} placeholder={s.name} commitOnBlur autoFocus onChange={(x) => x !== s.label && void setLabel(s, x)} />
                      </td>
                    ) : (
                      <td title={s.label && s.name ? s.name : undefined}>{s.label || s.name || ''}</td>
                    )}
                    <td>
                      {s.source && <span className="ip">{s.source}</span>} {(src?.label || src?.name) && <span className="muted">{src.label || src.name}</span>}
                    </td>
                    <td>
                      <span className={`kind ${streamTone(s)}`} title={ptpTitle(s)}>
                        {streamType(s)}
                      </span>
                    </td>
                    <td>{v && <VlanTag vlan={v} />}</td>
                    <td className="num">
                      {s.bps > 0 ? (
                        <div className="rate">
                          <Spark data={s.history} />
                          <span className="ip">{fmtBps(s.bps)}</span>
                        </div>
                      ) : (
                        <span className="ip">—</span>
                      )}
                    </td>
                    <td className="num ip">{s.pps > 0 ? fmtInt(s.pps) : '—'}</td>
                    <td>{live ? t('live') : fmtSeen(s.lastSeen)}</td>
                    <td className="act" onClick={(e) => e.stopPropagation()}>
                      {editable && s.port > 0 && (
                        <button className="btn small" disabled={checking !== null} onClick={() => void check(s)}>
                          {checking === key ? <RefreshCw size={13} className="spin" /> : <ShieldCheck size={13} />} {t('Check')}
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function ptpTitle(s: Stream): string | undefined {
  if (s.kind !== 'ptp') return s.encoding;
  return [s.ptpDomain !== undefined ? t('Domain {n}', { n: s.ptpDomain }) : '', s.ptpGrandmaster ? t('Grandmaster {id}', { id: s.ptpGrandmaster }) : ''].filter(Boolean).join(' · ') || undefined;
}

/** Courbe du débit (60 dernières valeurs). */
function Spark({ data }: { data: number[] }) {
  const W = 64;
  const H = 16;
  if (data.length < 2) return <svg className="spark" width={W} height={H} />;
  const max = Math.max(...data);
  const min = Math.min(...data);
  const span = max - min || 1;
  const y = (v: number) => (max === min ? H / 2 : H - 3 - ((v - min) / span) * (H - 6));
  const pts = data.map((v, i) => `${((i / (data.length - 1)) * W).toFixed(1)},${y(v).toFixed(1)}`).join(' ');
  return (
    <svg className="spark" width={W} height={H}>
      <polyline fill="none" stroke="currentColor" strokeWidth="1.5" points={pts} />
    </svg>
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
