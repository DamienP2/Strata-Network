import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { createPortal } from 'react-dom';
import { Bookmark, ChevronDown, Crosshair, Ellipsis, Eye, FileDown, FileUp, LayoutGrid, List, LoaderCircle, Network as NetworkIcon, Pencil, Plus, Radar, RefreshCw, Save, Search, Square, Trash2, X } from 'lucide-react';
import { t, tc, tn } from '../i18n';
import { api, can, serverEvents, useApp, useChanged } from '../store';
import { navigate } from '../router';
import { Field } from '../settings/ui';
import { Segmented } from '../ui/fields';
import { Dropdown, type DropOption } from '../ui/Dropdown';
import { confirmDialog } from '../ui/dialogs';
import { openContextMenu, type MenuEntry } from '../ui/ContextMenu';
import { Empty } from '../ui/Empty';
import { useDisplayPref } from '../displayPrefs';
import { intToIp, ipToInt, parseRange, validIp, type Range } from '../net/ip';
import { KIND_ICON, KINDS, copyText, fail, fmtMs, fmtSeen, hasWeb, ipState, kindLabel, useList, vlanById, vlanMismatch, type IpState } from '../net/model';
import { FitChips, InlineText, PortChips, vlanStyle } from '../net/parts';
import { Splitter } from '../ui/Splitter';
import type { Address, Kind, Network, Switch, SwitchPort, Vlan } from '../net/types';

type Chip = 'all' | 'used' | 'free' | 'offline' | 'web' | 'new' | 'watched';
const ROW = 32;

/** Les colonnes de la liste qui se redimensionnent (Actions garde sa largeur). */
type ColId = 'state' | 'ip' | 'label' | 'kind' | 'dns' | 'mac' | 'ports' | 'plug';
type ColWidths = Partial<Record<ColId, number>>;
const COLS: { id: ColId; w: number; head: () => string }[] = [
  { id: 'state', w: 46, head: () => t('State') },
  { id: 'ip', w: 176, head: () => t('IP') },
  { id: 'label', w: 180, head: () => t('Label') },
  { id: 'kind', w: 124, head: () => t('Type') },
  { id: 'dns', w: 140, head: () => t('DNS name') },
  { id: 'mac', w: 140, head: () => t('MAC') },
  { id: 'ports', w: 260, head: () => t('Ports') },
  { id: 'plug', w: 170, head: () => t('Connection') },
];
const ACTS_W = 98;
const COL_MIN = 36;
const COL_MAX = 1200;
const FIT_MAX = 480;

/**
 * La largeur qu'il faut à chaque colonne : le plus large de son titre et des cellules dessinées.
 * Mesurée sur une copie du tableau en mise en page automatique, toutes les puces de ports visibles.
 */
function fitColumns(box: HTMLElement): ColWidths {
  const table = box.querySelector<HTMLTableElement>(':scope > table.xt');
  if (!table) return {};
  const clone = table.cloneNode(true) as HTMLTableElement;
  clone.querySelector('colgroup')?.remove();
  clone.querySelectorAll('tr.pad, .splitter, .port-more').forEach((e) => e.remove());
  clone.querySelectorAll('.fit-hid').forEach((e) => e.classList.remove('fit-hid'));
  clone.setAttribute('aria-hidden', 'true');
  Object.assign(clone.style, { position: 'absolute', left: '0', top: '0', visibility: 'hidden', width: 'max-content', tableLayout: 'auto' });
  box.appendChild(clone);
  const ths = clone.querySelectorAll<HTMLElement>('thead th');
  const out: ColWidths = {};
  COLS.forEach((c, i) => {
    const th = ths[i];
    if (th) out[c.id] = Math.min(FIT_MAX, Math.max(COL_MIN, Math.ceil(th.getBoundingClientRect().width) + 2));
  });
  clone.remove();
  return out;
}

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
  return t('Planned scan: {time}', { time: mins >= 60 ? t('{n} h', { n: mins / 60 }) : t('{n} min', { n: mins }) });
}

/* ---------- Un réseau : liste ou fiches, tout se modifie dans le tableau ---------- */

function NetworkView({ net, vlans, switches, editable, ip }: { net: Network; vlans: Vlan[] | null; switches: Switch[] | null; editable: boolean; ip?: string }) {
  const toast = useApp((s) => s.toast);
  const progress = useApp((s) => s.scans[net.id]);
  const [addrs, setAddrs] = useState<Map<number, Address> | null>(null);
  const [q, setQ] = useState('');
  const [chip, setChip] = useState<Chip>('all');
  const [view, setView] = useDisplayPref<'list' | 'tiles'>('scan.view', 'list');
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

  // Le scan en cours : les lignes arrivent une à une (regroupées toutes les 300 ms), puis tout est relu à la fin.
  useEffect(() => {
    let batch: Address[] = [];
    let timer: ReturnType<typeof setTimeout> | undefined;
    const flush = () => {
      timer = undefined;
      const list = batch;
      batch = [];
      setAddrs((old) => {
        const m = new Map(old ?? []);
        for (const a of list) {
          const n = ipToInt(a.ip);
          if (n !== null) m.set(n, a);
        }
        return m;
      });
    };
    const onAddress = (e: Event) => {
      const m = (e as CustomEvent<{ networkId: string; address: Address }>).detail;
      if (m.networkId !== net.id || !m.address) return;
      batch.push(m.address);
      timer ??= setTimeout(flush, 300);
    };
    const onScan = (e: Event) => {
      const m = (e as CustomEvent<{ networkId: string; running: boolean }>).detail;
      if (m.networkId === net.id && !m.running) void load();
    };
    serverEvents.addEventListener('address', onAddress);
    serverEvents.addEventListener('scan', onScan);
    return () => {
      clearTimeout(timer);
      serverEvents.removeEventListener('address', onAddress);
      serverEvents.removeEventListener('scan', onScan);
    };
  }, [net.id, load]);

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

  const patch = useCallback(
    async (ipText: string, body: Record<string, unknown>) => {
      try {
        put(await api<Address>('PATCH', `/api/addresses/${ipText}`, body));
      } catch (e) {
        fail(e);
      }
    },
    [put],
  );

  const select = (n: number | null) => {
    setSel(n);
    if (n === null) return;
    const a = addrs?.get(n);
    if (a?.isNew) void patch(intToIp(n), { isNew: false }); // permis à tous
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

  const pct = progress && progress.total ? (progress.done / progress.total) * 100 : 0;
  return (
    <main className="library">
      <div className={`library-inner full ${view === 'list' ? 'fill' : ''}`}>
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
                <Radar size={14} /> {tc('verb', 'Scan')}
              </button>
            ))}
          <button className="icon-btn" title={t('More actions')} onClick={menu}>
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

        {addrs && range && <Occupancy range={range} addrs={addrs} counts={counts} sel={sel} onSel={select} />}

        <div className="media-toolbar">
          <label className="search">
            <Search size={14} />
            <input value={q} placeholder={t('Search an IP, a name, a MAC')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
          </label>
          <div className="asr-chips">
            {(
              [
                ['all', t('All addresses'), counts.all],
                ['used', tc('ips', 'Used'), counts.used],
                ['free', tc('ips', 'Free'), counts.free],
                ['offline', tc('ips', 'Offline'), counts.offline],
                ['web', t('Web'), counts.web],
                ['new', tc('ips', 'New'), counts.new],
                ['watched', tc('ips', 'Watched'), counts.watched],
              ] as [Chip, string, number][]
            ).map(([id, label, n]) => (
              <button key={id} className={`asr-chip ${chip === id ? 'on' : ''}`} onClick={() => setChip(id)}>
                {label} <span className="asr-chip-n">{n}</span>
              </button>
            ))}
          </div>
          <div className="spacer" />
          <Segmented
            value={view === 'tiles' ? 'tiles' : 'list'}
            onChange={setView}
            options={[
              { value: 'list', label: <List size={14} />, title: t('List') },
              { value: 'tiles', label: <LayoutGrid size={14} />, title: t('Cards') },
            ]}
          />
        </div>

        {!addrs || !range ? (
          <div className="loading">{t('Loading…')}</div>
        ) : view === 'tiles' ? (
          <IpTiles
            rows={rows}
            addrs={addrs}
            gw={gw}
            sel={sel}
            onSel={(n) => {
              select(n);
              setView('list');
            }}
          />
        ) : (
          <IpTable rows={rows} addrs={addrs} sel={sel} onSel={select} vlans={vlans} switches={switches} editable={editable} onPatch={patch} onPut={put} />
        )}
      </div>
    </main>
  );
}

/* ---------- Liste : une ligne par adresse, seules les lignes visibles sont dessinées ---------- */

type Plug = { sw: Switch; port: SwitchPort };

function IpTable({
  rows,
  addrs,
  sel,
  onSel,
  vlans,
  switches,
  editable,
  onPatch,
  onPut,
}: {
  rows: number[];
  addrs: Map<number, Address>;
  sel: number | null;
  onSel: (n: number) => void;
  vlans: Vlan[] | null;
  switches: Switch[] | null;
  editable: boolean;
  onPatch: (ip: string, body: Record<string, unknown>) => Promise<void>;
  onPut: (a: Address) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(900);
  const [width, setWidth] = useState(0);
  const [pop, setPop] = useState<{ n: number; el: HTMLElement } | null>(null);
  const [scanning, setScanning] = useState<Set<number>>(() => new Set());
  const [saved, setSaved] = useDisplayPref<ColWidths | null>('scan.cols', null);
  const clicked = useRef<number | null>(null);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const size = () => {
      setHeight(el.clientHeight);
      setWidth(el.clientWidth);
    };
    const ro = new ResizeObserver(size);
    ro.observe(el);
    size();
    return () => ro.disconnect();
  }, []);

  // Largeurs des colonnes : gardées pour l'utilisateur ; la première fois, ajustées au contenu.
  const widths = Object.fromEntries(COLS.map((c) => [c.id, Math.max(COL_MIN, saved?.[c.id] ?? c.w)])) as Record<ColId, number>;
  // The space left goes to the last column (Actions, right-aligned), so every handle follows the mouse.
  const sum = COLS.reduce((s, c) => s + widths[c.id], editable ? ACTS_W : 0);
  const spare = Math.max(0, width - sum);
  const last = COLS[COLS.length - 1].id;
  const shown = editable ? widths : { ...widths, [last]: widths[last] + spare };
  const actsW = ACTS_W + (editable ? spare : 0);
  const tableWidth = sum + spare;
  const fitted = useRef(false);
  useLayoutEffect(() => {
    if (saved || fitted.current || !box.current) return;
    fitted.current = true;
    setSaved({ ...widths, ...fitColumns(box.current) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [saved]);
  const setCol = (id: ColId, w: number) => setSaved((old) => ({ ...widths, ...old, [id]: w }));

  // Une adresse choisie ailleurs (bandeau, fiche, alerte, prochaine IP libre) : la faire défiler en vue.
  useEffect(() => {
    const el = box.current;
    if (!el || sel === null) return;
    if (clicked.current === sel) {
      clicked.current = null; // une ligne cliquée est déjà en vue
      return;
    }
    const i = rows.indexOf(sel);
    if (i < 0) return;
    const y = i * ROW;
    if (y < el.scrollTop || y + ROW * 2 > el.scrollTop + el.clientHeight) el.scrollTop = Math.max(0, y - el.clientHeight / 3);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sel]);

  // Où chaque machine est branchée (le port qui la déclare).
  const hosts = useMemo(() => {
    const m = new Map<string, Plug>();
    for (const sw of switches ?? []) for (const port of sw.ports) if (port.host && !m.has(port.host)) m.set(port.host, { sw, port });
    return m;
  }, [switches]);
  const plugOf = (a: Address | undefined, ip: string): Plug | null => {
    if (a?.switch) {
      const sw = switches?.find((s) => s.id === a.switch!.id);
      const port = sw?.ports.find((p) => p.n === a.switch!.port);
      if (sw && port) return { sw, port };
    }
    return hosts.get(ip) ?? null;
  };

  const kinds: DropOption<Kind | ''>[] = useMemo(() => [{ value: '', label: '—' }, ...KINDS.map((k) => ({ value: k, label: kindLabel(k), icon: iconOf(k) }))], []);

  const rescan = async (n: number) => {
    setScanning((s) => new Set(s).add(n));
    try {
      onPut(await api<Address>('POST', `/api/addresses/${intToIp(n)}/scan`));
    } catch (e) {
      fail(e);
    }
    setScanning((s) => {
      const x = new Set(s);
      x.delete(n);
      return x;
    });
  };

  const cols = editable ? 9 : 8;
  const from = Math.max(0, Math.floor(top / ROW) - 8);
  const to = Math.min(rows.length, Math.ceil((top + height) / ROW) + 8);
  const popAddr = pop ? addrs.get(pop.n) : undefined;
  return (
    <div className="card-table fill" ref={box} onScroll={(e) => setTop(e.currentTarget.scrollTop)}>
      <table className="xt" style={{ width: tableWidth }}>
        <colgroup>
          {COLS.map((c) => (
            <col key={c.id} style={{ width: shown[c.id] }} />
          ))}
          {editable && <col style={{ width: actsW }} />}
        </colgroup>
        <thead>
          <tr>
            {COLS.map((c, i) => (
              <th key={c.id}>
                {c.head()}
                {(editable || i < COLS.length - 1) && (
                  <Splitter
                    axis="x"
                    edge="right"
                    value={shown[c.id]}
                    min={COL_MIN}
                    max={COL_MAX}
                    onChange={(w) => setCol(c.id, w)}
                    onReset={() => box.current && setCol(c.id, fitColumns(box.current)[c.id] ?? c.w)}
                  />
                )}
              </th>
            ))}
            {editable && <th className="right">{t('Actions')}</th>}
          </tr>
        </thead>
        <tbody>
          {from > 0 && <Pad h={from * ROW} cols={cols} />}
          {rows.slice(from, to).map((n) => {
            const a = addrs.get(n);
            const ip = intToIp(n);
            return (
              <IpRow
                key={n}
                n={n}
                ip={ip}
                a={a}
                on={sel === n}
                plug={plugOf(a, ip)}
                vlans={vlans}
                kinds={kinds}
                editable={editable}
                scanning={scanning.has(n)}
                popOpen={pop?.n === n}
                onSel={(x) => {
                  clicked.current = x;
                  onSel(x);
                }}
                onPatch={onPatch}
                onRescan={rescan}
                onPlug={(el) => setPop((p) => (p?.n === n ? null : { n, el }))}
              />
            );
          })}
          {to < rows.length && <Pad h={(rows.length - to) * ROW} cols={cols} />}
        </tbody>
      </table>
      {rows.length === 0 && <div className="empty">{t('No match')}</div>}
      {pop && (
        <PlugPop
          key={pop.n}
          ip={intToIp(pop.n)}
          plug={plugOf(popAddr, intToIp(pop.n))}
          anchor={pop.el}
          switches={switches ?? []}
          vlans={vlans}
          onPut={onPut}
          onClose={() => setPop(null)}
        />
      )}
    </div>
  );
}

function Pad({ h, cols }: { h: number; cols: number }) {
  return (
    <tr className="pad" aria-hidden>
      <td colSpan={cols} style={{ height: h }} />
    </tr>
  );
}

function IpRow({
  n,
  ip,
  a,
  on,
  plug,
  vlans,
  kinds,
  editable,
  scanning,
  popOpen,
  onSel,
  onPatch,
  onRescan,
  onPlug,
}: {
  n: number;
  ip: string;
  a: Address | undefined;
  on: boolean;
  plug: Plug | null;
  vlans: Vlan[] | null;
  kinds: DropOption<Kind | ''>[];
  editable: boolean;
  scanning: boolean;
  popOpen: boolean;
  onSel: (n: number) => void;
  onPatch: (ip: string, body: Record<string, unknown>) => Promise<void>;
  onRescan: (n: number) => void;
  onPlug: (el: HTMLElement) => void;
}) {
  const s = ipState(a);
  const mm = plug ? vlanMismatch(vlans, plug.port, ip) : null;
  const reserve = (
    <button
      type="button"
      className={`icon-btn ghost ${a?.reserved ? 'on' : ''}`}
      title={a?.reserved ? t('Reserved') : t('Reserve')}
      aria-pressed={!!a?.reserved}
      onClick={() => void onPatch(ip, { reserved: !a?.reserved })}
    >
      <Bookmark size={13} />
    </button>
  );
  const head = (
    <>
      <td title={stateTitle(s, a)}>
        <span className={`st ${STATE_CLASS[s]}`} />
      </td>
      <td>
        <span className="ip">{ip}</span>
        {a?.isNew && <span className="new-badge">{t('New')}</span>}
        {mm && (
          <span className="badge warn" title={t('Outside the subnet of VLAN {id} ({subnet})', { id: mm.id, subnet: mm.subnet ?? '' })}>
            {t('VLAN')}
          </span>
        )}
      </td>
    </>
  );
  if (s === 'free')
    return (
      <tr className={`free ${on ? 'sel' : ''}`} onClick={() => onSel(n)}>
        {head}
        <td colSpan={6} />
        {editable && (
          <td>
            <div className="acts">{reserve}</div>
          </td>
        )}
      </tr>
    );

  const Icon = a?.kind ? KIND_ICON[a.kind] : null;
  const vlan = plug ? vlanById(vlans, plug.port.vlan) : undefined;
  const plugText = plug && (
    <span className="plug">
      <span className="vlan-sw" style={vlanStyle(vlan)} />
      <span className="plug-name">
        {plug.sw.name} · <span className="ip">{plug.port.name || plug.port.n}</span>
      </span>
    </span>
  );
  const plugTitle = plug && plug.port.vlan !== undefined ? [t('VLAN {id}', { id: plug.port.vlan }), vlan?.name].filter(Boolean).join(' · ') : undefined;
  return (
    <tr className={`${STATE_CLASS[s]} ${on ? 'sel' : ''}`} onClick={() => onSel(n)}>
      {head}
      <td>
        <InlineText value={a?.label ?? ''} placeholder={t('Add a label')} editable={editable} onSave={(label) => void onPatch(ip, { label })} />
      </td>
      <td>
        {editable ? (
          <Dropdown className="cell" value={a?.kind ?? ''} options={kinds} onChange={(k) => void onPatch(ip, { kind: k || null })} />
        ) : (
          a?.kind &&
          Icon && (
            <span className="cell-kind">
              <Icon size={14} className="kind-ic" />
              {kindLabel(a.kind)}
            </span>
          )
        )}
      </td>
      <td>{a?.name || <span className="muted2">—</span>}</td>
      <td title={a?.vendor}>{a?.mac && <span className="mac">{a.mac}</span>}</td>
      <td>{a && a.ports.length > 0 && <FitChips a={a} />}</td>
      <td>
        {editable ? (
          <div
            className={`ed dd ${popOpen ? 'open' : ''}`}
            role="button"
            tabIndex={0}
            title={plugTitle}
            aria-expanded={popOpen}
            onClick={(e) => onPlug(e.currentTarget)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onPlug(e.currentTarget);
              }
            }}
          >
            {plugText || <span className="ph">{t('Connect…')}</span>}
            <ChevronDown size={12} />
          </div>
        ) : (
          <span title={plugTitle}>{plugText}</span>
        )}
      </td>
      {editable && (
        <td>
          <div className="acts">
            <button type="button" className="icon-btn ghost" title={t('Rescan')} disabled={scanning} onClick={() => onRescan(n)}>
              <RefreshCw size={13} className={scanning ? 'spin' : undefined} />
            </button>
            <button
              type="button"
              className={`icon-btn ghost ${a?.watched ? 'on' : ''}`}
              title={a?.watched ? t('Watched') : t('Watch')}
              aria-pressed={!!a?.watched}
              onClick={() => void onPatch(ip, { watched: !a?.watched })}
            >
              <Eye size={13} />
            </button>
            {reserve}
          </div>
        </td>
      )}
    </tr>
  );
}

const STATE_CLASS: Record<IpState, string> = { used: 'used', offline: 'off', reserved: 'resv', free: 'free' };

/** The state as a dot; its title tells the rest (latency, last seen). */
function stateTitle(s: IpState, a: Address | undefined): string {
  if (s === 'used') return a?.rttMs !== undefined && a?.rttMs !== null ? t('Used · {n}', { n: fmtMs(a.rttMs) }) : stateLabel(s);
  if (s === 'offline') return t('Offline · seen {when}', { when: fmtSeen(a?.lastSeen) });
  return stateLabel(s);
}

/** The state written out: Used (green), Offline (orange), Reserved (blue ring), Free (grey ring). */
function StatePill({ s }: { s: IpState }) {
  return <span className={`st ${STATE_CLASS[s]}`}>{stateLabel(s)}</span>;
}

function stateLabel(s: IpState): string {
  switch (s) {
    case 'used':
      return tc('state', 'Used');
    case 'offline':
      return tc('state', 'Offline');
    case 'reserved':
      return tc('state', 'Reserved');
    default:
      return tc('state', 'Free');
  }
}

const iconOf = (k: Kind) => {
  const I = KIND_ICON[k];
  return <I size={12} />;
};

/* ---------- Raccordement : un petit menu sous la cellule ---------- */

function PlugPop({
  ip,
  plug,
  anchor,
  switches,
  vlans,
  onPut,
  onClose,
}: {
  ip: string;
  plug: Plug | null;
  anchor: HTMLElement;
  switches: Switch[];
  vlans: Vlan[] | null;
  onPut: (a: Address) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [swId, setSwId] = useState(plug?.sw.id ?? switches[0]?.id ?? '');
  const [port, setPort] = useState<string | null>(plug ? String(plug.port.n) : null);
  const [busy, setBusy] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const close = useRef(onClose);
  close.current = onClose;

  // Sous la cellule, ou au-dessus s'il manque de place ; toujours dans la fenêtre.
  const place = useCallback(() => {
    const r = anchor.getBoundingClientRect();
    const box = anchor.closest('.card-table')?.getBoundingClientRect();
    if (!anchor.isConnected || (box && (r.bottom < box.top + 30 || r.top > box.bottom))) return close.current(); // la ligne est sortie de la vue
    const w = ref.current?.offsetWidth ?? 280;
    const h = ref.current?.offsetHeight ?? 180;
    let top = r.bottom + 4;
    if (top + h > innerHeight - 8) top = Math.max(8, r.top - h - 4);
    setPos({ left: Math.max(8, Math.min(r.left, innerWidth - w - 8)), top });
  }, [anchor]);
  useLayoutEffect(place, [place]);

  // Fermer : Échap, clic ailleurs, redimensionnement ; suivre la cellule quand la liste défile.
  useEffect(() => {
    const inside = (n: EventTarget | null) => n instanceof Node && (!!ref.current?.contains(n) || anchor.contains(n) || (n instanceof Element && !!n.closest('.dd-list')));
    const down = (e: PointerEvent) => !inside(e.target) && close.current();
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      e.preventDefault();
      close.current();
      anchor.focus();
    };
    const scroll = (e: Event) => !inside(e.target) && place();
    const resize = () => close.current();
    document.addEventListener('pointerdown', down, true);
    document.addEventListener('keydown', key);
    document.addEventListener('scroll', scroll, true);
    window.addEventListener('resize', resize);
    return () => {
      document.removeEventListener('pointerdown', down, true);
      document.removeEventListener('keydown', key);
      document.removeEventListener('scroll', scroll, true);
      window.removeEventListener('resize', resize);
    };
  }, [anchor, place]);

  const sw = switches.find((x) => x.id === swId);
  const swOptions: DropOption[] = switches.map((x) => ({ value: x.id, label: x.name, detail: x.ip }));
  const portOptions: DropOption[] = (sw?.ports ?? []).map((p) => ({
    value: String(p.n),
    label: [p.name || String(p.n), p.mode === 'trunk' ? t('Trunk') : p.vlan !== undefined ? t('VLAN {id}', { id: p.vlan }) : ''].filter(Boolean).join(' · '),
    icon: <span className="vlan-sw" style={p.mode === 'off' ? undefined : vlanStyle(vlanById(vlans, p.vlan))} />,
    note: (p.host && p.host !== ip ? p.host : p.label) || undefined,
  }));

  // Les ports qui déclarent cette machine, sauf celui-ci.
  const unplugOthers = async (keep?: { id: string; n: number }) => {
    for (const x of switches)
      for (const p of x.ports)
        if (p.host === ip && !(keep && keep.id === x.id && keep.n === p.n)) await api('PATCH', `/api/switches/${x.id}/ports`, { ports: [p.n], set: { host: null } });
  };

  const ok = async () => {
    if (!swId || !port) return;
    const n = Number(port);
    if (plug && plug.sw.id === swId && plug.port.n === n && plug.port.host === ip) return onClose();
    setBusy(true);
    try {
      await api('PATCH', `/api/switches/${swId}/ports`, { ports: [n], set: { host: ip } });
      await unplugOthers({ id: swId, n });
      onClose();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  };
  const unplug = async () => {
    setBusy(true);
    try {
      await unplugOthers();
      onPut(await api<Address>('PATCH', `/api/addresses/${ip}`, { switch: null }));
      onClose();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  };

  return createPortal(
    <div ref={ref} className="pop" role="dialog" style={pos ?? { visibility: 'hidden' }} onClick={(e) => e.stopPropagation()}>
      <span className="set-label">{t('Switch')}</span>
      <Dropdown
        value={swId || null}
        options={swOptions}
        onChange={(v) => {
          setSwId(v);
          setPort(null);
        }}
      />
      <span className="set-label">{t('Port')}</span>
      <Dropdown value={port} options={portOptions} onChange={setPort} />
      <div className="pop-actions">
        {plug && (
          <button type="button" className="btn small" disabled={busy} onClick={() => void unplug()}>
            {t('Unplug')}
          </button>
        )}
        <button type="button" className="btn small primary" disabled={busy || !swId || !port} onClick={() => void ok()}>
          {t('OK')}
        </button>
      </div>
    </div>,
    document.body,
  );
}

/* ---------- Bandeau d'occupation : toute la plage d'un coup d'œil ---------- */

function Occupancy({
  range,
  addrs,
  counts,
  sel,
  onSel,
}: {
  range: Range;
  addrs: Map<number, Address>;
  counts: { used: number; offline: number; reserved: number; freeOnly: number };
  sel: number | null;
  onSel: (n: number) => void;
}) {
  const blocks: number[] = [];
  for (let b = Math.floor(range.lo / 256); b <= Math.floor(range.hi / 256); b++) blocks.push(b);
  // Up to 4 × /24: one bar per address, a line per /24. Beyond: one cell per /24, shaded by how full it is.
  const detailed = blocks.length <= 4;
  return (
    <div className="occ-card">
      <div className="occ-head">
        <span>
          <b>{counts.used}</b> {tn(counts.used, 'used IP', 'used IPs')}
        </span>
        <span>
          <b>{counts.freeOnly}</b> {tn(counts.freeOnly, 'free IP', 'free IPs')}
        </span>
        <span>
          <b>{counts.offline}</b> {t('offline')}
        </span>
        <span>
          <b>{counts.reserved}</b> {tn(counts.reserved, 'reserved IP', 'reserved IPs')}
        </span>
        <span className="spacer" />
        <StatePill s="used" />
        <StatePill s="offline" />
        <StatePill s="reserved" />
        <StatePill s="free" />
      </div>
      {detailed ? (
        blocks.map((b) => (
          <div key={b} className="occ-line">
            {blocks.length > 1 && <span className="occ-label">{`${intToIp(b * 256)}/24`}</span>}
            <div className="occ">
              {Array.from({ length: 256 }, (_, i) => {
                const n = b * 256 + i;
                if (n < range.first || n > range.last) return <i key={i} className="na" />;
                const a = addrs.get(n);
                const st = ipState(a);
                const name = a?.label || a?.name;
                return (
                  <i
                    key={i}
                    className={`${st === 'free' ? '' : STATE_CLASS[st]} ${n === sel ? 'sel' : ''}`}
                    title={[intToIp(n), stateLabel(st), name].filter(Boolean).join(' · ')}
                    onClick={() => onSel(n)}
                  />
                );
              })}
            </div>
          </div>
        ))
      ) : (
        <div className="occ-blocks">
          {blocks.map((b) => {
            let used = 0;
            let total = 0;
            for (let n = Math.max(b * 256, range.first); n <= Math.min(b * 256 + 255, range.last); n++) {
              total++;
              const st = ipState(addrs.get(n));
              if (st === 'used' || st === 'offline') used++;
            }
            const first = Math.max(b * 256, range.first);
            return (
              <i
                key={b}
                style={{ '--fill': `${total ? Math.round((used / total) * 100) : 0}%` } as React.CSSProperties}
                title={`${intToIp(b * 256)}/24 · ${tn(used, '{n} used IP', '{n} used IPs')} · ${tn(total - used, '{n} free IP', '{n} free IPs')}`}
                onClick={() => onSel(first)}
              />
            );
          })}
        </div>
      )}
      {detailed && (
        <div className={`occ-ticks ${blocks.length > 1 ? 'indent' : ''}`}>
          {Array.from({ length: 16 }, (_, i) => (
            <span key={i}>.{i * 16}</span>
          ))}
        </div>
      )}
    </div>
  );
}

/* ---------- Fiches : une fiche par adresse, les libres en petites cases ---------- */

function IpTiles({ rows, addrs, gw, sel, onSel }: { rows: number[]; addrs: Map<number, Address>; gw: number | null; sel: number | null; onSel: (n: number) => void }) {
  // Grouped by /24, each group drawn when it comes near the screen.
  const groups = useMemo(() => {
    const out: { block: number; list: number[] }[] = [];
    for (const n of rows) {
      const b = Math.floor(n / 256);
      if (out.length === 0 || out[out.length - 1].block !== b) out.push({ block: b, list: [] });
      out[out.length - 1].list.push(n);
    }
    return out;
  }, [rows]);
  if (rows.length === 0) return <div className="empty">{t('No match')}</div>;
  return (
    <div className="tiles-wrap">
      {groups.map((g) => (
        <TileGroup key={g.block} block={g.block} list={g.list} title={groups.length > 1} lazy={groups.length > 2} addrs={addrs} gw={gw} sel={sel} onSel={onSel} />
      ))}
    </div>
  );
}

function TileGroup({
  block,
  list,
  title,
  lazy,
  addrs,
  gw,
  sel,
  onSel,
}: {
  block: number;
  list: number[];
  title: boolean;
  lazy: boolean;
  addrs: Map<number, Address>;
  gw: number | null;
  sel: number | null;
  onSel: (n: number) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [shown, setShown] = useState(!lazy);
  useEffect(() => {
    if (shown || !ref.current) return;
    const io = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && setShown(true), { rootMargin: '800px' });
    io.observe(ref.current);
    return () => io.disconnect();
  }, [shown]);
  // A tile chosen elsewhere (band, next free IP, alert): bring it into view.
  useEffect(() => {
    if (sel === null || Math.floor(sel / 256) !== block) return;
    if (!shown) setShown(true);
    requestAnimationFrame(() => ref.current?.querySelector('.tile.sel')?.scrollIntoView({ block: 'nearest' }));
  }, [sel, block, shown]);
  return (
    <div className="tiles-group" ref={ref}>
      {title && <p className="ip-plan-title">{`${intToIp(block * 256)}/24`}</p>}
      {shown ? (
        <div className="tiles">
          {list.map((n) => (
            <Tile key={n} n={n} a={addrs.get(n)} gw={n === gw} on={n === sel} onSel={onSel} />
          ))}
        </div>
      ) : (
        <div style={{ height: Math.ceil(list.length / 6) * 50 }} />
      )}
    </div>
  );
}

function Tile({ n, a, gw, on, onSel }: { n: number; a: Address | undefined; gw: boolean; on: boolean; onSel: (n: number) => void }) {
  const s = ipState(a);
  const last = `.${n % 256}`;
  if (s === 'free')
    return (
      <button type="button" className={`tile free ${on ? 'sel' : ''}`} title={intToIp(n)} onClick={() => onSel(n)}>
        <span className="ip">{last}</span>
        <StatePill s="free" />
      </button>
    );
  const name = a?.label || a?.name || '—';
  const sub = [a?.label ? a?.name : '', a?.vendor].filter(Boolean).join(' · ') || (s === 'offline' ? fmtSeen(a?.lastSeen) : '');
  return (
    <button type="button" className={`tile ${STATE_CLASS[s]} ${on ? 'sel' : ''}`} title={intToIp(n)} onClick={() => onSel(n)}>
      <span className="tile-top">
        <span className="ip">{last}</span>
        {gw && <span className="badge">{t('gateway')}</span>}
        {a?.isNew && <span className="new-badge">{t('New')}</span>}
        <StatePill s={s} />
      </span>
      <span className="t-name">{name}</span>
      {sub && <span className="t-sub">{sub}</span>}
      {a && a.ports.length > 0 && <PortChips a={a} max={2} />}
    </button>
  );
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
