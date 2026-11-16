import { useState, type FormEvent } from 'react';
import { Pencil, Plus, Save, Trash2, X } from 'lucide-react';
import { t, tc, tn } from '../../i18n';
import { api, can, useApp } from '../../store';
import { confirmDialog, promptDialog } from '../../ui/dialogs';
import { Dropdown, type DropOption } from '../../ui/Dropdown';
import { Field } from '../../settings/ui';
import { fail, vlanById } from '../../net/model';
import { VlanTag } from '../../net/parts';
import type { Flows, GroupUse, McastRange, RangeView } from '../../net/mcast';
import type { Vlan } from '../../net/types';

const ipNum = (ip: string) => ip.split('.').reduce((s, o) => s * 256 + Number(o), 0);
const numIp = (n: number) => [n >>> 24, (n >>> 16) & 255, (n >>> 8) & 255, n & 255].join('.');
const RANK: Record<GroupUse['state'], number> = { conflict: 3, used: 2, reserved: 1 };
const CELL: Record<GroupUse['state'], string> = { conflict: 'c', used: 'u', reserved: 'r' };

/** 256 cells (32 × 8); a cell holds several groups in ranges above /24 (the worst state shows). */
function Cells({ r, onRelease }: { r: RangeView; onRelease?: (g: GroupUse) => void }) {
  const first = ipNum(r.first);
  const n = Math.min(256, r.size);
  const per = r.size / n;
  const cells: (GroupUse | null)[] = Array.from({ length: n }, () => null);
  for (const g of r.groups) {
    const i = Math.min(n - 1, Math.floor((ipNum(g.group) - first) / per));
    if (!cells[i] || RANK[g.state] > RANK[cells[i]!.state]) cells[i] = g;
  }
  return (
    <div className="grid16">
      {cells.map((g, i) => {
        const ip = first + i * per;
        const na = per === 1 && (ip % 256 === 0 || ip % 256 === 255);
        const title = per === 1 ? [numIp(ip), g?.name].filter(Boolean).join(' · ') : `${numIp(ip)} – ${numIp(ip + per - 1)}`;
        const release = g && g.state === 'reserved' && per === 1 && onRelease ? () => onRelease(g) : undefined;
        return <i key={i} className={`${g ? CELL[g.state] : na ? 'na' : ''} ${release ? 'click' : ''}`} title={title} onClick={release} />;
      })}
    </div>
  );
}

export function Plan({ data, vlans }: { data: Flows; vlans: Vlan[] | null }) {
  const user = useApp((s) => s.auth?.user);
  const editor = can.edit(user);
  const [editing, setEditing] = useState<McastRange | 'new' | null>(null);
  const [preset, setPreset] = useState('');
  const { ranges, outside } = data.plan;
  const total = (k: 'used' | 'reserved' | 'conflicts') => ranges.reduce((s, r) => s + r[k], 0) + outside[k];

  const reserve = async (group: string) => {
    const name = await promptDialog({ title: t('Reserve {group}', { group }), label: t('Name'), confirm: t('Reserve') });
    if (name === null) return;
    try {
      await api('POST', '/api/multicast/reservations', { group, name });
    } catch (e) {
      fail(e);
    }
  };
  const release = async (g: GroupUse) => {
    if (!(await confirmDialog({ title: t('Release {group}?', { group: g.group }), message: g.name, confirm: t('Release') }))) return;
    try {
      await api('DELETE', `/api/multicast/reservations/${g.group}`);
    } catch (e) {
      fail(e);
    }
  };
  const remove = async (r: RangeView) => {
    if (!(await confirmDialog({ title: t('Delete the range {cidr}?', { cidr: r.cidr }), message: r.usage, confirm: t('Delete'), danger: true }))) return;
    try {
      await api('DELETE', `/api/multicast/ranges/${r.id}`);
    } catch (e) {
      fail(e);
    }
  };

  return (
    <>
      <div className="occ-head" style={{ padding: '0 2px' }}>
        <span>
          <b>{total('used')}</b> {tc('plan', 'used')}
        </span>
        <span>
          <b>{total('reserved')}</b> {tc('plan', 'reserved')}
        </span>
        <span>
          <b>{total('conflicts')}</b> {t('in conflict')}
        </span>
        <span className="spacer" />
        <div className="sw-legend">
          <span>
            <span className="vlan-sw" style={{ background: 'var(--pvw)' }} />
            {t('Sent')}
          </span>
          <span>
            <span className="vlan-sw" style={{ background: 'var(--accent)' }} />
            {t('Reserved')}
          </span>
          <span>
            <span className="vlan-sw" style={{ background: 'var(--warn)' }} />
            {t('Conflict')}
          </span>
          <span>
            <span className="vlan-sw" style={{ background: 'var(--panel-3)' }} />
            {t('Free')}
          </span>
        </div>
        {editor && (
          <button className="btn small" onClick={() => (setPreset(''), setEditing('new'))}>
            <Plus size={13} /> {t('New range')}
          </button>
        )}
      </div>
      <div className="blocks">
        {ranges.map((r) => (
          <div key={r.id} className="blk">
            <div className="blk-h">
              <b>{r.cidr}</b>
              <span>{r.usage}</span>
              {r.vlan !== undefined && <VlanTag vlan={vlanById(vlans, r.vlan)} id={r.vlan} />}
              <span className="n">
                {r.used}/{r.size}
              </span>
              {editor && (
                <>
                  <button className="icon-btn ghost" title={t('Edit')} onClick={() => setEditing(r)}>
                    <Pencil size={13} />
                  </button>
                  <button className="icon-btn ghost" title={t('Delete')} onClick={() => void remove(r)}>
                    <Trash2 size={13} />
                  </button>
                </>
              )}
            </div>
            <Cells r={r} onRelease={editor ? (g) => void release(g) : undefined} />
            <div className="blk-f">
              {r.nextFree ? (
                <span>
                  {t('Next:')} <span className="ip">{r.nextFree}</span>
                </span>
              ) : (
                <span className="muted2">{t('Full')}</span>
              )}
              <span className="spacer" />
              {editor && r.nextFree && (
                <button className="btn small" onClick={() => void reserve(r.nextFree!)}>
                  <Plus size={12} /> {t('Reserve')}
                </button>
              )}
            </div>
          </div>
        ))}
        {outside.groups.length > 0 && (
          <div className="blk out">
            <div className="blk-h">
              <b>{t('Outside the plan')}</b>
              <span className="n">{tn(outside.groups.length, '{n} group', '{n} groups')}</span>
            </div>
            <div className="out-groups">
              {outside.groups.map((g) => (
                <span key={g.group} className={`pchip ${g.state === 'conflict' ? 'router' : ''}`} title={g.name}>
                  {g.group}
                </span>
              ))}
            </div>
            <div className="blk-f">
              <span className="spacer" />
              {editor && (
                <button
                  className="btn small"
                  onClick={() => {
                    const o = outside.groups[0].group.split('.');
                    setPreset(`${o[0]}.${o[1]}.${o[2]}.0/24`);
                    setEditing('new');
                  }}
                >
                  <Plus size={12} /> {t('Create a range')}
                </button>
              )}
            </div>
          </div>
        )}
      </div>
      {ranges.length === 0 && outside.groups.length === 0 && <div className="empty">{t('No range')}</div>}
      {editing && <RangeModal range={editing === 'new' ? null : editing} preset={preset} vlans={vlans} onClose={() => setEditing(null)} />}
    </>
  );
}

function RangeModal({ range, preset, vlans, onClose }: { range: McastRange | null; preset: string; vlans: Vlan[] | null; onClose: () => void }) {
  const [cidr, setCidr] = useState(range?.cidr ?? preset);
  const [usage, setUsage] = useState(range?.usage ?? '');
  const [vlan, setVlan] = useState<string>(range?.vlan !== undefined ? String(range.vlan) : '');
  const [busy, setBusy] = useState(false);
  const bad = !/^(\d{1,3})(\.\d{1,3}){3}\/\d{1,2}$/.test(cidr.trim());
  const options: DropOption[] = [{ value: '', label: t('None') }, ...(vlans ?? []).map((v) => ({ value: String(v.id), label: `${v.id} · ${v.name}`, detail: v.subnet }))];
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    try {
      const body = { cidr: cidr.trim(), usage: usage.trim(), vlan: vlan ? Number(vlan) : undefined };
      if (range) await api('PUT', `/api/multicast/ranges/${range.id}`, body);
      else await api('POST', '/api/multicast/ranges', body);
      onClose();
    } catch (err) {
      fail(err);
    }
    setBusy(false);
  };
  return (
    <div className="modal-back" onPointerDown={onClose}>
      <form className="modal" role="dialog" aria-labelledby="range-title" onPointerDown={(e) => e.stopPropagation()} onSubmit={(e) => void submit(e)}>
        <header>
          <h2 id="range-title">{range ? t('Edit the range {cidr}', { cidr: range.cidr }) : t('New range')}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <Field label={t('Range')}>
          <input className="mono" value={cidr} placeholder="239.10.1.0/24" aria-invalid={!!cidr && bad} autoFocus autoComplete="off" onChange={(e) => setCidr(e.target.value)} />
        </Field>
        <Field label={t('Usage')}>
          <input value={usage} autoComplete="off" onChange={(e) => setUsage(e.target.value)} />
        </Field>
        <Field label={t('VLAN')}>
          <Dropdown value={vlan} options={options} onChange={setVlan} />
        </Field>
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            {t('Cancel')}
          </button>
          <button type="submit" className="btn primary" disabled={bad || busy}>
            {range ? <Save size={14} /> : <Plus size={14} />} {range ? t('Save') : t('Create the range')}
          </button>
        </footer>
      </form>
    </div>
  );
}
