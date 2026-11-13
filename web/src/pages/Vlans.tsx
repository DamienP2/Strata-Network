import { useMemo, useState, type CSSProperties, type FormEvent } from 'react';
import { Layers, Pencil, Plus, Save, Search, Trash2, X } from 'lucide-react';
import { t, tn } from '../i18n';
import { api, can, useApp } from '../store';
import { Field } from '../settings/ui';
import { Check } from '../ui/fields';
import { confirmDialog } from '../ui/dialogs';
import { validCidr, validIp } from '../net/ip';
import { fail, useList } from '../net/model';
import { VlanTag } from '../net/parts';
import type { Network, Switch, Vlan } from '../net/types';

const COLORS = ['#8fa3ff', '#22c55e', '#2dd4bf', '#60a5fa', '#e879f9', '#f5c542', '#f5a524', '#ff3b5c', '#a3a8bf'];

export function Vlans() {
  const editable = can.edit(useApp((s) => s.auth?.user));
  const [vlans] = useList<Vlan[]>('vlans', '/api/vlans');
  const [switches] = useList<Switch[]>('switches', '/api/switches');
  const [networks] = useList<Network[]>('networks', '/api/networks');
  const [q, setQ] = useState('');
  const [editing, setEditing] = useState<Vlan | 'new' | null>(null);

  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (vlans ?? [])
      .filter((v) => !needle || String(v.id).includes(needle) || v.name.toLowerCase().includes(needle) || (v.subnet ?? '').includes(needle))
      .sort((a, b) => a.id - b.id);
  }, [vlans, q]);

  // Ports : accès ou natif, et taggués ; machines : actives au dernier scan des réseaux de ce VLAN.
  const ports = (id: number) => (switches ?? []).reduce((n, sw) => n + sw.ports.filter((p) => p.mode !== 'off' && (p.vlan === id || p.tagged.includes(id))).length, 0);
  const machines = (id: number) => {
    const nets = (networks ?? []).filter((n) => n.vlan === id && n.lastScan);
    return nets.length ? nets.reduce((n, x) => n + (x.lastScan?.up ?? 0), 0) : null;
  };

  const remove = async (v: Vlan) => {
    if (!(await confirmDialog({ title: t('Delete the VLAN {id} “{name}”?', { id: v.id, name: v.name }), confirm: t('Delete'), danger: true }))) return;
    try {
      await api('DELETE', `/api/vlans/${v.id}`);
    } catch (e) {
      fail(e);
    }
  };

  if (!vlans) return <div className="loading">{t('Loading…')}</div>;
  return (
    <main className="library">
      <div className="library-inner">
        <div className="settings-head">
          <div>
            <h1>{t('VLAN')}</h1>
            <p className="muted">{tn(vlans.length, '{n} VLAN', '{n} VLANs')}</p>
          </div>
          <div className="spacer" />
          {editable && (
            <button className="btn primary" onClick={() => setEditing('new')}>
              <Plus size={14} /> {t('New VLAN')}
            </button>
          )}
        </div>
        <div className="media-toolbar">
          <label className="search">
            <Search size={14} />
            <input value={q} placeholder={t('Search VLANs')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
          </label>
        </div>
        {shown.length === 0 ? (
          <div className="empty big">
            <Layers size={22} />
            <p>{q ? t('No match') : t('No VLAN')}</p>
          </div>
        ) : (
          <table className="user-table">
            <thead>
              <tr>
                <th>{t('ID')}</th>
                <th>{t('Name')}</th>
                <th>{t('Subnet')}</th>
                <th>{t('Gateway')}</th>
                <th>{t('Ports')}</th>
                <th>{t('Machines')}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {shown.map((v) => {
                const m = machines(v.id);
                return (
                  <tr key={v.id}>
                    <td>
                      <VlanTag vlan={v} />
                    </td>
                    <td>{v.name}</td>
                    <td className="ip">{v.subnet || '—'}</td>
                    <td className="ip">{v.gateway || '—'}</td>
                    <td>{ports(v.id)}</td>
                    <td>{m === null ? '—' : m}</td>
                    <td style={{ textAlign: 'right' }}>
                      {editable && (
                        <>
                          <button className="icon-btn ghost" title={t('Edit')} onClick={() => setEditing(v)}>
                            <Pencil size={14} />
                          </button>
                          <button className="icon-btn ghost" title={t('Delete')} onClick={() => void remove(v)}>
                            <Trash2 size={14} />
                          </button>
                        </>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
      {editing && <VlanModal vlan={editing === 'new' ? null : editing} used={vlans.map((v) => v.id)} onClose={() => setEditing(null)} />}
    </main>
  );
}

function VlanModal({ vlan, used, onClose }: { vlan: Vlan | null; used: number[]; onClose: () => void }) {
  const [id, setId] = useState(vlan ? String(vlan.id) : '');
  const [name, setName] = useState(vlan?.name ?? '');
  const [subnet, setSubnet] = useState(vlan?.subnet ?? '');
  const [gateway, setGateway] = useState(vlan?.gateway ?? '');
  const [color, setColor] = useState(vlan?.color ?? COLORS[used.length % COLORS.length]);
  const [withNet, setWithNet] = useState(true);
  const [busy, setBusy] = useState(false);
  const n = Number(id);
  const badId = id.trim() !== '' && (!Number.isInteger(n) || n < 1 || n > 4094 || (!vlan && used.includes(n)));
  const badSubnet = subnet.trim() !== '' && !validCidr(subnet.trim());
  const badGw = gateway.trim() !== '' && !validIp(gateway.trim());

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy || badId || badSubnet || badGw) return;
    setBusy(true);
    const body: Vlan = { id: n, name: name.trim(), color, subnet: subnet.trim() || undefined, gateway: gateway.trim() || undefined, notes: vlan?.notes ?? '' };
    try {
      if (vlan) await api('PUT', `/api/vlans/${vlan.id}`, body);
      else {
        await api('POST', '/api/vlans', body);
        if (withNet && body.subnet) await api('POST', '/api/networks', { name: body.name || `VLAN ${n}`, range: body.subnet, gateway: body.gateway, vlan: n, scheduleMins: 0 });
      }
      onClose();
    } catch (err) {
      fail(err);
      setBusy(false);
    }
  };

  return (
    <div className="modal-back" onPointerDown={onClose}>
      <form className="modal wide" role="dialog" aria-labelledby="vlan-title" onPointerDown={(e) => e.stopPropagation()} onSubmit={submit}>
        <header>
          <h2 id="vlan-title">{vlan ? t('Edit the VLAN {id}', { id: vlan.id }) : t('New VLAN')}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="set-grid two">
          <Field label={t('ID')}>
            <input className="mono" value={id} autoFocus={!vlan} readOnly={!!vlan} aria-invalid={badId} inputMode="numeric" autoComplete="off" onChange={(e) => setId(e.target.value)} />
          </Field>
          <Field label={t('Name')}>
            <input value={name} autoComplete="off" onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label={t('Subnet')}>
            <input className="mono" value={subnet} placeholder="10.40.0.0/24" aria-invalid={badSubnet} autoComplete="off" onChange={(e) => setSubnet(e.target.value)} />
          </Field>
          <Field label={t('Gateway')}>
            <input className="mono" value={gateway} placeholder={t('Optional')} aria-invalid={badGw} autoComplete="off" onChange={(e) => setGateway(e.target.value)} />
          </Field>
          <div className="set-field">
            <span className="set-label">{t('Color')}</span>
            <div className="color-pick">
              {COLORS.map((c) => (
                <button key={c} type="button" className={c === color ? 'on' : ''} style={{ '--vlan': c } as CSSProperties} aria-pressed={c === color} aria-label={c} onClick={() => setColor(c)} />
              ))}
            </div>
          </div>
          {!vlan && (
            <div className="set-field">
              <span className="set-label">{t('Network to scan')}</span>
              <Check checked={withNet} onChange={setWithNet} disabled={!subnet.trim() || badSubnet}>
                {t('Create the network')}
              </Check>
            </div>
          )}
        </div>
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            {t('Cancel')}
          </button>
          <button type="submit" className="btn primary" disabled={!id.trim() || !name.trim() || badId || badSubnet || badGw || busy}>
            {vlan ? <Save size={14} /> : <Plus size={14} />} {vlan ? t('Save') : t('Create the VLAN')}
          </button>
        </footer>
      </form>
    </div>
  );
}
