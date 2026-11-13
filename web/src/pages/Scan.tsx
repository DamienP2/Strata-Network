import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { Crosshair, Ellipsis, FileDown, FileUp, LayoutGrid, List, LoaderCircle, Network as NetworkIcon, Pencil, Plus, Radar, RefreshCw, Save, Search, Square, Trash2, X } from 'lucide-react';
import { t, tn } from '../i18n';
import { api, can, serverEvents, useApp, useChanged } from '../store';
import { navigate } from '../router';
import { Field } from '../settings/ui';
import { Check, Segmented, TextInput } from '../ui/fields';
import { Dropdown, type DropOption } from '../ui/Dropdown';
import { confirmDialog } from '../ui/dialogs';
import { openContextMenu, type MenuEntry } from '../ui/ContextMenu';
import { Empty } from '../ui/Empty';
import { fmtDate } from '../ui/format';
import { intToIp, ipToInt, parseRange, validIp, type Range } from '../net/ip';
import {
  KIND_ICON,
  KINDS,
  copyText,
  displayName,
  fail,
  fmtMs,
  fmtSeen,
  hasWeb,
  ipState,
  kindLabel,
  portOf,
  useList,
  vlanById,
  vlanMismatch,
  type IpState,
} from '../net/model';
import { PortChip, PortChips, VlanTag, vlanStyle } from '../net/parts';
import type { Address, Kind, Network, Switch, Vlan } from '../net/types';

type Chip = 'all' | 'used' | 'free' | 'offline' | 'web' | 'new' | 'watched';
const ROW = 29;

export function Scan({ networkId, ip }: { networkId?: string; ip?: string }) {
  const user = useApp((s) => s.auth?.user);
  const editable = can.edit(user);
  const [networks] = useList<Network[]>('networks', '/api/networks');
  const [vlans] = useList<Vlan[]>('vlans', '/api/vlans');
  const [switches] = useList<Switch[]>('switches', '/api/switches');
  const [editing, setEditing] = useState<Network | 'new' | null>(null);
  const net = networks?.find((n) => n.id === networkId);
  const first = networks?.[0]?.id;

  // Sans réseau dans l'adresse (ou réseau supprimé) : le premier.
  useEffect(() => {
    if (networks && !net && first) navigate(`/scan/${first}`, true);
  }, [networks, net, first]);

  const remove = async (n: Network) => {
    if (!(await confirmDialog({ title: t('Delete the network “{name}”?', { name: n.name }), confirm: t('Delete'), danger: true }))) return;
    try {
      await api('DELETE', `/api/networks/${n.id}`);
    } catch (e) {
      fail(e);
    }
  };

  if (!networks) return <div className="loading">{t('Loading…')}</div>;
  return (
    <ScanLayout
      networks={networks}
      current={net?.id}
      editable={editable}
      onNew={() => setEditing('new')}
      onEdit={setEditing}
      onRemove={(n) => void remove(n)}
      modal={editing && <NetworkModal net={editing === 'new' ? null : editing} vlans={vlans} onClose={() => setEditing(null)} onSaved={(n) => (setEditing(null), navigate(`/scan/${n.id}`))} />}
    >
      {net ? (
        <NetworkView key={net.id} net={net} vlans={vlans} switches={switches} editable={editable} ip={ip} />
      ) : (
        <main className="library">
          <Empty icon={NetworkIcon} text={t('No network')}>
            {editable && (
              <button className="btn primary" onClick={() => setEditing('new')}>
                <Plus size={14} /> {t('New network')}
              </button>
            )}
          </Empty>
        </main>
      )}
    </ScanLayout>
  );
}

function ScanLayout({
  networks,
  current,
  editable,
  onNew,
  onEdit,
  onRemove,
  modal,
  children,
}: {
  networks: Network[];
  current?: string;
  editable: boolean;
  onNew: () => void;
  onEdit: (n: Network) => void;
  onRemove: (n: Network) => void;
  modal: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <>
      <div className="net-page two scan-page">
        <aside className="lib-nav">
          <div className="lib-nav-section">
            <span className="grow">{t('Networks')}</span>
            {editable && (
              <button className="icon-btn ghost" style={{ width: 22, height: 22 }} title={t('New network')} onClick={onNew}>
                <Plus size={13} />
              </button>
            )}
          </div>
          {networks.map((n) => (
            <div
              key={n.id}
              className={`lib-nav-item ${current === n.id ? 'on' : ''}`}
              role="link"
              tabIndex={0}
              aria-current={current === n.id ? 'page' : undefined}
              onClick={() => navigate(`/scan/${n.id}`)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  navigate(`/scan/${n.id}`);
                }
              }}
            >
              <NetworkIcon size={15} />
              <span className="grow">{n.name}</span>
              {editable && (
                <span className="lib-nav-actions" onClick={(e) => e.stopPropagation()}>
                  <button className="icon-btn ghost" title={t('Edit')} onClick={() => onEdit(n)}>
                    <Pencil size={13} />
                  </button>
                  <button className="icon-btn ghost" title={t('Delete')} onClick={() => onRemove(n)}>
                    <Trash2 size={13} />
                  </button>
                </span>
              )}
              {n.scheduleMins > 0 && <span className="nav-dot ok" title={scheduleText(n.scheduleMins)} />}
              {n.lastScan && <span className="count">{n.lastScan.up}</span>}
            </div>
          ))}
        </aside>
        {children}
      </div>
      {modal}
    </>
  );
}

function scheduleText(mins: number): string {
  return t('Planned scan: every {time}', { time: mins >= 60 ? t('{n} h', { n: mins / 60 }) : t('{n} min', { n: mins }) });
}

/* ---------- Un réseau : liste ou plan, fiche à droite ---------- */

function NetworkView({ net, vlans, switches, editable, ip }: { net: Network; vlans: Vlan[] | null; switches: Switch[] | null; editable: boolean; ip?: string }) {
  const toast = useApp((s) => s.toast);
  const progress = useApp((s) => s.scans[net.id]);
  const [addrs, setAddrs] = useState<Map<number, Address> | null>(null);
  const [q, setQ] = useState('');
  const [chip, setChip] = useState<Chip>('all');
  const [view, setView] = useState<'list' | 'plan'>('list');
  const [sel, setSel] = useState<number | null>(() => ipToInt(ip));
  const file = useRef<HTMLInputElement>(null);
  const range = useMemo(() => parseRange(net.range), [net.range]);
  const gw = ipToInt(net.gateway);

  const load = useCallback(async () => {
    try {
      const list = await api<Address[]>('GET', `/api/networks/${net.id}/addresses`);
      const m = new Map<number, Address>();
      for (const a of list) {
        const n = ipToInt(a.ip);
        if (n !== null) m.set(n, a);
      }
      setAddrs(m);
    } catch (e) {
      fail(e);
    }
  }, [net.id]);
  useEffect(() => void load(), [load]);
  useChanged('addresses', load);

  const put = useCallback((a: Address) => {
    const n = ipToInt(a.ip);
    if (n === null) return;
    setAddrs((old) => {
      const m = new Map(old ?? []);
      m.set(n, a);
      return m;
    });
  }, []);

  // Le scan en cours : une ligne à la fois, puis tout relire à la fin.
  useEffect(() => {
    const onAddress = (e: Event) => {
      const m = (e as CustomEvent<{ networkId: string; address: Address }>).detail;
      if (m.networkId === net.id && m.address) put(m.address);
    };
    const onScan = (e: Event) => {
      const m = (e as CustomEvent<{ networkId: string; running: boolean }>).detail;
      if (m.networkId === net.id && !m.running) void load();
    };
    serverEvents.addEventListener('address', onAddress);
    serverEvents.addEventListener('scan', onScan);
    return () => {
      serverEvents.removeEventListener('address', onAddress);
      serverEvents.removeEventListener('scan', onScan);
    };
  }, [net.id, put, load]);

  useEffect(() => {
    const n = ipToInt(ip);
    if (n !== null) setSel(n);
  }, [ip]);

  // Les comptes, pour toute la plage.
  const counts = useMemo(() => {
    const c = { all: 0, used: 0, free: 0, offline: 0, web: 0, new: 0, watched: 0, reserved: 0, freeOnly: 0 };
    if (!range || !addrs) return c;
    c.all = range.last - range.first + 1;
    let known = 0;
    for (const [n, a] of addrs) {
      if (n < range.first || n > range.last) continue;
      known++;
      const s = ipState(a);
      if (s === 'used') c.used++;
      else if (s === 'offline') c.offline++;
      else if (s === 'reserved') c.reserved++;
      else c.freeOnly++;
      if (hasWeb(a)) c.web++;
      if (a.isNew) c.new++;
      if (a.watched) c.watched++;
    }
    c.freeOnly += c.all - known;
    c.free = c.freeOnly + c.reserved;
    return c;
  }, [range, addrs]);

  // Les lignes affichées : toute la plage, filtrée.
  const rows = useMemo(() => {
    const out: number[] = [];
    if (!range || !addrs) return out;
    const needle = q.trim().toLowerCase();
    for (let n = range.first; n <= range.last; n++) {
      const a = addrs.get(n);
      if (chip !== 'all') {
        const s = ipState(a);
        if (chip === 'used' && s !== 'used') continue;
        if (chip === 'free' && s !== 'free' && s !== 'reserved') continue;
        if (chip === 'offline' && s !== 'offline') continue;
        if (chip === 'web' && !hasWeb(a)) continue;
        if (chip === 'new' && !a?.isNew) continue;
        if (chip === 'watched' && !a?.watched) continue;
      }
      if (needle) {
        const hay = `${intToIp(n)} ${a?.name ?? ''} ${a?.label ?? ''} ${a?.mac ?? ''} ${a?.vendor ?? ''}`.toLowerCase();
        if (!hay.includes(needle)) continue;
      }
      out.push(n);
    }
    return out;
  }, [range, addrs, q, chip]);

  const select = (n: number | null) => {
    setSel(n);
    if (n === null) return;
    const a = addrs?.get(n);
    if (a?.isNew && editable) void patch(intToIp(n), { isNew: false });
  };

  const patch = async (ipText: string, body: Record<string, unknown>) => {
    try {
      put(await api<Address>('PATCH', `/api/addresses/${ipText}`, body));
    } catch (e) {
      fail(e);
    }
  };

  const start = async () => {
    try {
      await api('POST', `/api/networks/${net.id}/scan`);
    } catch (e) {
      fail(e);
    }
  };
  const stop = async () => {
    try {
      await api('DELETE', `/api/networks/${net.id}/scan`);
    } catch (e) {
      fail(e);
    }
  };
  const nextFree = async () => {
    try {
      const { ip: next } = await api<{ ip: string | null }>('GET', `/api/networks/${net.id}/next-free`);
      if (!next) return toast(t('No free IP'));
      const copied = await copyText(next);
      toast(copied ? t('Next free IP: {ip} · copied', { ip: next }) : t('Next free IP: {ip}', { ip: next }));
      const n = ipToInt(next);
      if (n !== null) setSel(n);
    } catch (e) {
      fail(e);
    }
  };
  const importCsv = async (f: File) => {
    try {
      const text = await f.text();
      const r = await api<{ imported: number }>('POST', `/api/networks/${net.id}/import`, new Blob([text], { type: 'text/csv' }));
      toast(tn(r.imported, '{n} address imported', '{n} addresses imported'));
      void load();
    } catch (e) {
      fail(e);
    }
  };
  const menu = (e: React.MouseEvent) => {
    const r = e.currentTarget.getBoundingClientRect();
    const items: MenuEntry[] = [
      {
        icon: FileDown,
        label: t('Export as CSV'),
        onClick: () => {
          const a = document.createElement('a');
          a.href = `/api/networks/${net.id}/export.csv`;
          a.download = `${net.name}.csv`;
          document.body.appendChild(a);
          a.click();
          a.remove();
        },
      },
    ];
    if (editable) items.push({ icon: FileUp, label: t('Import a CSV'), onClick: () => file.current?.click() });
    openContextMenu({ clientX: r.right - 200, clientY: r.bottom + 6 }, items);
  };

  const selIp = sel === null ? null : intToIp(sel);
  const pct = progress && progress.total ? (progress.done / progress.total) * 100 : 0;
  return (
    <>
      <main className="library">
        <div className={`library-inner wide ${view === 'list' ? 'fill' : ''}`}>
          <div className="settings-head">
            <div>
              <h1>{net.name}</h1>
              <p className="muted">
                <span className="mono">{net.range}</span>
                {net.gateway && (
                  <>
                    {' · '}
                    {t('gateway')} <span className="mono">{net.gateway}</span>
                  </>
                )}
                {addrs && ` · ${tn(counts.used, '{n} used IP', '{n} used IPs')} · ${tn(counts.free, '{n} free IP', '{n} free IPs')}`}
              </p>
            </div>
            <div className="spacer" />
            {progress && (
              <div className="scan-run">
                <LoaderCircle size={14} className="spin" /> {progress.done} / {progress.total}
                <span className="perf-bar">
                  <span className="perf-bar-track">
                    <span className="perf-bar-fill" style={{ transform: `translateX(${pct - 100}%)` }} />
                  </span>
                </span>
              </div>
            )}
            <button className="btn" onClick={() => void nextFree()}>
              <Crosshair size={14} /> {t('Next free IP')}
            </button>
            {editable &&
              (progress ? (
                <button className="btn" onClick={() => void stop()}>
                  <Square size={14} /> {t('Stop')}
                </button>
              ) : (
                <button className="btn primary" onClick={() => void start()}>
                  <Radar size={14} /> {t('Scan')}
                </button>
              ))}
            <button className="icon-btn" title={t('More')} onClick={menu}>
              <Ellipsis size={14} />
            </button>
            <input
              ref={file}
              className="file-hidden"
              type="file"
              accept=".csv,text/csv"
              onChange={(e) => {
                const f = e.target.files?.[0];
                e.target.value = '';
                if (f) void importCsv(f);
              }}
            />
          </div>

          <div className="media-toolbar">
            <label className="search">
              <Search size={14} />
              <input value={q} placeholder={t('Search an IP, a name, a MAC')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
            </label>
            <div className="asr-chips">
              {(
                [
                  ['all', t('All addresses'), counts.all],
                  ['used', t('Used'), counts.used],
                  ['free', t('Free'), counts.free],
                  ['offline', t('Offline'), counts.offline],
                  ['web', t('Web'), counts.web],
                  ['new', t('New'), counts.new],
                  ['watched', t('Watched'), counts.watched],
                ] as [Chip, string, number][]
              ).map(([id, label, n]) => (
                <button key={id} className={`asr-chip ${chip === id ? 'on' : ''}`} onClick={() => setChip(id)}>
                  {label} <span className="asr-chip-n">{n}</span>
                </button>
              ))}
            </div>
            <div className="spacer" />
            <Segmented
              value={view}
              onChange={setView}
              options={[
                { value: 'list', label: <List size={14} />, title: t('List') },
                { value: 'plan', label: <LayoutGrid size={14} />, title: t('Address plan') },
              ]}
            />
          </div>

          {!addrs || !range ? (
            <div className="loading">{t('Loading…')}</div>
          ) : view === 'list' ? (
            <IpTable rows={rows} addrs={addrs} sel={sel} onSel={select} vlans={vlans} switches={switches} editable={editable} onLabel={(n, label) => void patch(intToIp(n), { label })} />
          ) : (
            <IpPlan range={range} addrs={addrs} gw={gw} sel={sel} onSel={select} counts={counts} />
          )}
        </div>
      </main>
      {selIp && (
        <HostPanel
          key={selIp}
          ip={selIp}
          a={addrs?.get(sel!)}
          vlans={vlans}
          switches={switches}
          addrs={addrs}
          editable={editable}
          onPatch={(body) => void patch(selIp, body)}
          onPut={put}
          onClose={() => setSel(null)}
        />
      )}
    </>
  );
}

/* ---------- Liste : une ligne par adresse, seules les lignes visibles sont dessinées ---------- */

function IpTable({
  rows,
  addrs,
  sel,
  onSel,
  vlans,
  switches,
  editable,
  onLabel,
}: {
  rows: number[];
  addrs: Map<number, Address>;
  sel: number | null;
  onSel: (n: number) => void;
  vlans: Vlan[] | null;
  switches: Switch[] | null;
  editable: boolean;
  onLabel: (n: number, label: string) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(900);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    setHeight(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  // Une adresse choisie ailleurs (alerte, prochaine IP libre) : la faire défiler en vue.
  useEffect(() => {
    const el = box.current;
    if (!el || sel === null) return;
    const i = rows.indexOf(sel);
    if (i < 0) return;
    const y = i * ROW;
    if (y < el.scrollTop || y + ROW * 2 > el.scrollTop + el.clientHeight) el.scrollTop = Math.max(0, y - el.clientHeight / 3);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sel]);

  const from = Math.max(0, Math.floor(top / ROW) - 8);
  const to = Math.min(rows.length, Math.ceil((top + height) / ROW) + 8);
  return (
    <div className="card-table fill" ref={box} onScroll={(e) => setTop(e.currentTarget.scrollTop)}>
      <table className="data-table ip-rows">
        <colgroup>
          <col style={{ width: 30 }} />
          <col style={{ width: 150 }} />
          <col style={{ width: '17%' }} />
          <col style={{ width: '17%' }} />
          <col style={{ width: 150 }} />
          <col style={{ width: '12%' }} />
          <col />
          <col style={{ width: 80 }} />
          <col style={{ width: 104 }} />
        </colgroup>
        <thead>
          <tr>
            <th />
            <th>{t('IP')}</th>
            <th>{t('DNS name')}</th>
            <th>{t('Label')}</th>
            <th>{t('MAC')}</th>
            <th>{t('Vendor')}</th>
            <th>{t('Open ports')}</th>
            <th className="num">{t('Latency')}</th>
            <th>{t('Seen')}</th>
          </tr>
        </thead>
        <tbody>
          {from > 0 && <Pad h={from * ROW} />}
          {rows.slice(from, to).map((n) => (
            <IpRow key={n} n={n} a={addrs.get(n)} on={sel === n} onSel={onSel} vlans={vlans} switches={switches} editable={editable} onLabel={onLabel} />
          ))}
          {to < rows.length && <Pad h={(rows.length - to) * ROW} />}
        </tbody>
      </table>
      {rows.length === 0 && <div className="empty">{t('No match')}</div>}
    </div>
  );
}

function Pad({ h }: { h: number }) {
  return (
    <tr className="pad" aria-hidden>
      <td colSpan={9} style={{ height: h }} />
    </tr>
  );
}

function IpRow({
  n,
  a,
  on,
  onSel,
  vlans,
  switches,
  editable,
  onLabel,
}: {
  n: number;
  a: Address | undefined;
  on: boolean;
  onSel: (n: number) => void;
  vlans: Vlan[] | null;
  switches: Switch[] | null;
  editable: boolean;
  onLabel: (n: number, label: string) => void;
}) {
  const ip = intToIp(n);
  const s = ipState(a);
  const cls = `${s === 'free' || s === 'reserved' ? 'free' : s === 'offline' ? 'off' : ''} ${on ? 'sel' : ''}`;
  const plug = a?.switch || switches?.some((sw) => sw.ports.some((p) => p.host === ip)) ? portOf(switches, a, ip) : null;
  const mm = plug ? vlanMismatch(vlans, plug.port, ip) : null;
  return (
    <tr className={cls} onClick={() => onSel(n)}>
      <td>
        <StateDot s={s} isNew={a?.isNew} />
      </td>
      <td>
        <span className="ip">{ip}</span>
        {mm && (
          <span className="badge warn" title={t('Outside the subnet of VLAN {id} ({subnet})', { id: mm.id, subnet: mm.subnet ?? '' })}>
            {t('VLAN')}
          </span>
        )}
      </td>
      <td>{a?.name || '—'}</td>
      {on && editable ? (
        <td onClick={(e) => e.stopPropagation()}>
          <TextInput className="cell-edit" value={a?.label ?? ''} commitOnBlur onChange={(v) => v !== (a?.label ?? '') && onLabel(n, v)} autoFocus />
        </td>
      ) : (
        <LabelCell a={a} s={s} editable={editable} />
      )}
      <td className="ip">{a?.mac ?? ''}</td>
      <td>{a?.vendor ?? ''}</td>
      <td>{a && <PortChips a={a} />}</td>
      <td className="num ip">{a?.up ? fmtMs(a.rttMs) : a?.lastSeen ? '—' : ''}</td>
      <td>{fmtSeen(a?.lastSeen)}</td>
    </tr>
  );
}

function LabelCell({ a, s, editable }: { a: Address | undefined; s: IpState; editable: boolean }) {
  if (s === 'free') return <td className="lbl-empty">{t('Free')}</td>;
  if (s === 'reserved') return <td className="lbl-empty">{a?.label ? t('Reserved · {label}', { label: a.label }) : t('Reserved')}</td>;
  if (a?.label) return <td>{a.label}</td>;
  return <td className="lbl-empty">{editable ? t('Add a label') : ''}</td>;
}

function StateDot({ s, isNew }: { s: IpState; isNew?: boolean }) {
  if (isNew) return <span className="nav-dot warn" title={t('New machine')} />;
  if (s === 'used') return <span className="nav-dot ok" title={t('Used')} />;
  if (s === 'offline') return <span className="nav-dot" title={t('Offline')} />;
  return <span className="nav-dot free" title={s === 'reserved' ? t('Reserved') : t('Free')} />;
}

/* ---------- Plan d'adressage : 16 colonnes, un bloc par /24 ---------- */

function IpPlan({
  range,
  addrs,
  gw,
  sel,
  onSel,
  counts,
}: {
  range: Range;
  addrs: Map<number, Address>;
  gw: number | null;
  sel: number | null;
  onSel: (n: number) => void;
  counts: { used: number; offline: number; reserved: number; freeOnly: number };
}) {
  const blocks: number[] = [];
  for (let b = Math.floor(range.lo / 256); b <= Math.floor(range.hi / 256); b++) blocks.push(b);
  const many = blocks.length > 1;
  return (
    <div className="set-card ip-plan-card">
      <header>
        <LayoutGrid size={16} />
        <h2>{t('Address plan')}</h2>
        <div className="spacer" />
        <div className="sw-legend">
          <span>
            <span className="ipc used" />
            {t('Used · {n}', { n: counts.used })}
          </span>
          <span>
            <span className="ipc off" />
            {t('Offline · {n}', { n: counts.offline })}
          </span>
          <span>
            <span className="ipc resv" />
            {t('Reserved · {n}', { n: counts.reserved })}
          </span>
          <span>
            <span className="ipc" />
            {t('Free · {n}', { n: counts.freeOnly })}
          </span>
        </div>
      </header>
      <div className="set-body">
        {blocks.map((b) => (
          <PlanBlock key={b} block={b} title={many} lazy={blocks.length > 4} range={range} addrs={addrs} gw={gw} sel={sel} onSel={onSel} />
        ))}
      </div>
    </div>
  );
}

function PlanBlock({
  block,
  title,
  lazy,
  range,
  addrs,
  gw,
  sel,
  onSel,
}: {
  block: number;
  title: boolean;
  lazy: boolean;
  range: Range;
  addrs: Map<number, Address>;
  gw: number | null;
  sel: number | null;
  onSel: (n: number) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [shown, setShown] = useState(!lazy);
  useEffect(() => {
    if (shown || !ref.current) return;
    const io = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && setShown(true), { rootMargin: '600px' });
    io.observe(ref.current);
    return () => io.disconnect();
  }, [shown]);

  const base = block * 256;
  const cells = [];
  if (shown)
    for (let i = 0; i < 256; i++) {
      const n = base + i;
      const ip = intToIp(n);
      const out = n < range.first || n > range.last;
      const a = addrs.get(n);
      const s = ipState(a);
      const cls = out ? 'na' : s === 'used' ? 'used' : s === 'offline' ? 'off' : s === 'reserved' ? 'resv' : '';
      const what = out ? (n === range.lo ? t('network') : n === range.hi ? t('broadcast') : t('outside the range')) : stateWord(s);
      cells.push(
        <button key={i} type="button" className={`ipc ${cls} ${n === gw ? 'gw' : ''} ${n === sel ? 'sel' : ''}`} title={`${ip} · ${what}${a?.label ? ` · ${a.label}` : ''}`} disabled={out} onClick={() => onSel(n)}>
          {i}
        </button>,
      );
    }
  return (
    <div className="ip-plan-block" ref={ref}>
      {title && <p className="ip-plan-title">{`${intToIp(base)}/24`}</p>}
      {shown ? <div className="ip-plan">{cells}</div> : <div className="ip-plan-wait" />}
    </div>
  );
}

function stateWord(s: IpState): string {
  switch (s) {
    case 'used':
      return t('used');
    case 'offline':
      return t('offline');
    case 'reserved':
      return t('reserved');
    case 'free':
      return t('free');
  }
}

/* ---------- Fiche de la machine ---------- */

function HostPanel({
  ip,
  a,
  vlans,
  switches,
  addrs,
  editable,
  onPatch,
  onPut,
  onClose,
}: {
  ip: string;
  a: Address | undefined;
  vlans: Vlan[] | null;
  switches: Switch[] | null;
  addrs: Map<number, Address> | null;
  editable: boolean;
  onPatch: (body: Record<string, unknown>) => void;
  onPut: (a: Address) => void;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const s = ipState(a);
  const Icon = KIND_ICON[a?.kind ?? 'other'];
  const plug = portOf(switches, a, ip);
  const [swId, setSwId] = useState<string>(plug?.sw.id ?? '');
  useEffect(() => setSwId(plug?.sw.id ?? ''), [plug?.sw.id]);
  const sw = switches?.find((x) => x.id === swId);
  const vlan = plug?.port.vlan !== undefined ? vlanById(vlans, plug.port.vlan) : undefined;
  const mm = plug ? vlanMismatch(vlans, plug.port, ip) : null;

  const rescan = async () => {
    setBusy(true);
    try {
      onPut(await api<Address>('POST', `/api/addresses/${ip}/scan`));
    } catch (e) {
      fail(e);
    }
    setBusy(false);
  };

  const kindOptions: DropOption<Kind | ''>[] = [{ value: '', label: '—' }, ...KINDS.map((k) => ({ value: k, label: kindLabel(k), icon: iconOf(k) }))];
  const swOptions: DropOption[] = [{ value: '', label: '—' }, ...(switches ?? []).map((x) => ({ value: x.id, label: x.name, detail: x.ip }))];
  const portOptions: DropOption[] = (sw?.ports ?? []).map((p) => ({ value: String(p.n), label: p.name || String(p.n), note: p.label || undefined }));
  const parents: DropOption[] = [
    { value: '', label: '—' },
    ...[...(addrs?.values() ?? [])]
      .filter((x) => x.ip !== ip && (x.up || x.label) && x.kind && ['switch', 'router', 'firewall', 'wifi', 'server'].includes(x.kind))
      .map((x) => ({ value: x.ip, label: displayName(x, x.ip), detail: x.ip })),
    ...(switches ?? []).filter((x) => x.ip && x.ip !== ip).map((x) => ({ value: x.ip!, label: x.name, detail: x.ip })),
  ].filter((o, i, all) => all.findIndex((y) => y.value === o.value) === i);
  if (a?.parent && !parents.some((o) => o.value === a.parent)) parents.push({ value: a.parent, label: a.parent });

  return (
    <aside className="panel right">
      <div className="host-head">
        <span className="host-ico">
          <Icon size={18} />
        </span>
        <div>
          <b>{a?.label || a?.name || (s === 'free' ? t('Free') : ip)}</b>
          <span className="ip">
            {ip}
            {a?.name ? ` · ${a.name}` : ''}
          </span>
        </div>
        {editable && (
          <button className="icon-btn ghost" title={t('Scan this machine')} disabled={busy} onClick={() => void rescan()}>
            <RefreshCw size={14} className={busy ? 'spin' : undefined} />
          </button>
        )}
        <button className="icon-btn ghost" title={t('Close')} onClick={onClose}>
          <X size={14} />
        </button>
      </div>
      <section className="insp-section">
        <header>
          <span>{t('Machine')}</span>
        </header>
        <div className="insp-body">
          <div className="prop-row">
            <span className="prop-label">{t('Label')}</span>
            <TextInput value={a?.label ?? ''} commitOnBlur disabled={!editable} onChange={(v) => v !== (a?.label ?? '') && onPatch({ label: v })} />
          </div>
          <div className="prop-row">
            <span className="prop-label">{t('Type')}</span>
            <Dropdown value={a?.kind ?? ''} options={kindOptions} disabled={!editable} onChange={(k) => onPatch({ kind: k || null })} />
          </div>
          {a?.mac && (
            <div className="prop-row">
              <span className="prop-label">{t('MAC')}</span>
              <span className="prop-val ip">{a.mac}</span>
            </div>
          )}
          {a?.vendor && (
            <div className="prop-row">
              <span className="prop-label">{t('Vendor')}</span>
              <span className="prop-val">{a.vendor}</span>
            </div>
          )}
          {a?.firstSeen && (
            <div className="prop-row">
              <span className="prop-label">{t('Seen since')}</span>
              <span className="prop-val">{fmtDate(a.firstSeen)}</span>
            </div>
          )}
          {s !== 'used' && (
            <div className="prop-row">
              <span className="prop-label">{t('Reserved')}</span>
              <Check checked={!!a?.reserved} disabled={!editable} onChange={(v) => onPatch({ reserved: v })} />
            </div>
          )}
          <div className="prop-row">
            <span className="prop-label">{t('Watch')}</span>
            <Check checked={!!a?.watched} disabled={!editable} onChange={(v) => onPatch({ watched: v })} />
          </div>
        </div>
      </section>
      {a && a.ports.length > 0 && (
        <section className="insp-section">
          <header>
            <span>{t('Open ports')}</span>
          </header>
          <div className="insp-body">
            {a.ports.map((p) => (
              <div key={p.port} className="svc-row">
                <PortChip ip={ip} p={p} />
                <span className="grow" title={p.banner}>
                  {[p.web?.scheme ?? p.service, p.web?.title ? `« ${p.web.title} »` : '', !p.web && p.banner ? p.banner : ''].filter(Boolean).join(' · ')}
                </span>
              </div>
            ))}
          </div>
        </section>
      )}
      <section className="insp-section">
        <header>
          <span>{t('Connection')}</span>
        </header>
        <div className="insp-body">
          <div className="prop-row">
            <span className="prop-label">{t('Switch')}</span>
            <Dropdown
              value={swId}
              options={swOptions}
              disabled={!editable}
              onChange={(v) => {
                setSwId(v);
                if (!v) onPatch({ switch: null });
              }}
            />
          </div>
          {swId && (
            <div className="prop-row">
              <span className="prop-label">{t('Port')}</span>
              <Dropdown value={plug?.sw.id === swId ? String(plug.port.n) : null} options={portOptions} disabled={!editable} onChange={(v) => onPatch({ switch: { id: swId, port: Number(v) } })} />
            </div>
          )}
          {plug && plug.port.vlan !== undefined && (
            <div className="prop-row">
              <span className="prop-label">{t('VLAN')}</span>
              <VlanTag vlan={vlan} id={plug.port.vlan} name />
              {mm && (
                <span className="badge warn" title={t('Outside the subnet of VLAN {id} ({subnet})', { id: mm.id, subnet: mm.subnet ?? '' })}>
                  {t('VLAN')}
                </span>
              )}
            </div>
          )}
          <div className="prop-row">
            <span className="prop-label">{t('Parent')}</span>
            <Dropdown value={a?.parent ?? ''} options={parents} disabled={!editable} onChange={(v) => onPatch({ parent: v || null })} />
          </div>
        </div>
      </section>
      <section className="insp-section">
        <header>
          <span>{t('Notes')}</span>
        </header>
        <div className="insp-body">
          <NotesField value={a?.notes ?? ''} disabled={!editable} onChange={(v) => onPatch({ notes: v })} />
        </div>
      </section>
    </aside>
  );
}

const iconOf = (k: Kind) => {
  const I = KIND_ICON[k];
  return <I size={12} />;
};

function NotesField({ value, disabled, onChange }: { value: string; disabled: boolean; onChange: (v: string) => void }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return <textarea value={v} disabled={disabled} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => e.stopPropagation()} onBlur={() => v !== value && onChange(v)} />;
}

/* ---------- Créer / modifier un réseau ---------- */

const SCHEDULES = ['0', '15', '60', '360', '1440'] as const;

function NetworkModal({ net, vlans, onClose, onSaved }: { net: Network | null; vlans: Vlan[] | null; onClose: () => void; onSaved: (n: Network) => void }) {
  const [name, setName] = useState(net?.name ?? '');
  const [range, setRange] = useState(net?.range ?? '');
  const [gateway, setGateway] = useState(net?.gateway ?? '');
  const [vlan, setVlan] = useState<string>(net?.vlan !== undefined ? String(net.vlan) : '');
  const [ports, setPorts] = useState(net?.ports ?? '');
  const [schedule, setSchedule] = useState<(typeof SCHEDULES)[number]>(
    (SCHEDULES.find((s) => Number(s) === net?.scheduleMins) ?? '0') as (typeof SCHEDULES)[number],
  );
  const [busy, setBusy] = useState(false);
  const badRange = range.trim() !== '' && !parseRange(range);
  const badGw = gateway.trim() !== '' && !validIp(gateway.trim());

  // Un VLAN choisi propose son sous-réseau et sa passerelle.
  const pickVlan = (v: string) => {
    setVlan(v);
    const x = vlans?.find((y) => String(y.id) === v);
    if (x?.subnet && !range.trim()) setRange(x.subnet);
    if (x?.gateway && !gateway.trim()) setGateway(x.gateway);
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy || badRange || badGw) return;
    setBusy(true);
    const body = {
      ...(net ?? {}),
      name: name.trim(),
      range: range.trim(),
      gateway: gateway.trim() || undefined,
      vlan: vlan ? Number(vlan) : undefined,
      ports: ports.trim() || undefined,
      scheduleMins: Number(schedule),
    };
    try {
      const saved = net ? await api<Network>('PUT', `/api/networks/${net.id}`, body) : await api<Network>('POST', '/api/networks', body);
      onSaved(saved);
    } catch (err) {
      fail(err);
      setBusy(false);
    }
  };

  const vlanOptions: DropOption[] = [{ value: '', label: '—' }, ...(vlans ?? []).map((v) => ({ value: String(v.id), label: `${v.id} · ${v.name}`, icon: <span className="vlan-sw" style={vlanStyle(v)} /> }))];
  return (
    <div className="modal-back" onPointerDown={onClose}>
      <form className="modal wide" role="dialog" aria-labelledby="net-title" onPointerDown={(e) => e.stopPropagation()} onSubmit={submit}>
        <header>
          <h2 id="net-title">{net ? t('Edit {name}', { name: net.name }) : t('New network')}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="set-grid two">
          <Field label={t('Name')}>
            <input value={name} autoFocus={!net} autoComplete="off" onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label={t('Range')}>
            <input className="mono" value={range} placeholder="10.10.0.0/24" aria-invalid={badRange} autoComplete="off" onChange={(e) => setRange(e.target.value)} />
          </Field>
          <Field label={t('Gateway')}>
            <input className="mono" value={gateway} placeholder={t('Optional')} aria-invalid={badGw} autoComplete="off" onChange={(e) => setGateway(e.target.value)} />
          </Field>
          <div className="set-field">
            <span className="set-label">{t('VLAN')}</span>
            <Dropdown value={vlan} options={vlanOptions} onChange={pickVlan} />
          </div>
          <Field label={t('Ports')}>
            <input className="mono" value={ports} placeholder={t('Default list')} autoComplete="off" onChange={(e) => setPorts(e.target.value)} />
          </Field>
          <div className="set-field">
            <span className="set-label">{t('Planned scan')}</span>
            <Segmented
              value={schedule}
              onChange={setSchedule}
              options={[
                { value: '0', label: t('Never') },
                { value: '15', label: t('{n} min', { n: 15 }) },
                { value: '60', label: t('{n} h', { n: 1 }) },
                { value: '360', label: t('{n} h', { n: 6 }) },
                { value: '1440', label: t('{n} h', { n: 24 }) },
              ]}
            />
          </div>
        </div>
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            {t('Cancel')}
          </button>
          <button type="submit" className="btn primary" disabled={!name.trim() || !range.trim() || badRange || badGw || busy}>
            {net ? <Save size={14} /> : <Plus size={14} />} {net ? t('Save') : t('Create the network')}
          </button>
        </footer>
      </form>
    </div>
  );
}
