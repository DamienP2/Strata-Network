import { useState, type CSSProperties, type ReactNode } from 'react';
import { ExternalLink } from 'lucide-react';
import { t } from '../i18n';
import { TextInput } from '../ui/fields';
import type { Address, Vlan } from './types';

/** Pastille de VLAN : carré de couleur, numéro, nom éventuel. */
export function VlanTag({ vlan, id, name }: { vlan?: Vlan; id?: number; name?: boolean }) {
  const n = vlan?.id ?? id;
  if (n === undefined) return null;
  return (
    <span className="vlan-tag" style={vlanStyle(vlan)}>
      <span className="vlan-sw" />
      <b>{n}</b>
      {name && vlan?.name ? ` ${vlan.name}` : null}
    </span>
  );
}

export const vlanStyle = (v: Vlan | undefined): CSSProperties | undefined => (v ? ({ '--vlan': v.color } as CSSProperties) : undefined);

/** Les ports ouverts d'une machine : lien pour les ports web. */
/** The open ports, web pages first; past `max`, a "+N" chip lists the others on hover. */
export function PortChips({ a, max }: { a: Address; max?: number }) {
  const sorted = [...a.ports].sort((x, y) => Number(!!y.web) - Number(!!x.web) || x.port - y.port);
  const shown = max === undefined ? sorted : sorted.slice(0, max);
  const rest = sorted.slice(shown.length);
  return (
    <div className="ports">
      {shown.map((p) => (
        <PortChip key={p.port} ip={a.ip} p={p} svc />
      ))}
      {rest.length > 0 && (
        <span className="port-more" title={rest.map((p) => `${p.port} ${p.web?.scheme ?? p.service ?? ''}`.trim()).join('\n')}>
          +{rest.length}
        </span>
      )}
    </div>
  );
}

export function PortChip({ ip, p, svc, children }: { ip: string; p: Address['ports'][number]; svc?: boolean; children?: ReactNode }) {
  const name = p.web?.scheme ?? p.service;
  const body = (
    <>
      {p.port}
      {svc && name && <span className="svc">{name}</span>}
      {children}
    </>
  );
  if (!p.web) return <span className="port-chip">{body}</span>;
  return (
    <a className="port-chip" href={`${p.web.scheme}://${ip}:${p.port}/`} target="_blank" rel="noreferrer" title={p.web.title || t('Open the page')} onClick={(e) => e.stopPropagation()}>
      {body} <ExternalLink size={11} />
    </a>
  );
}

/**
 * A table cell that reads like text and becomes a field on click (Enter or leaving saves, Escape cancels).
 * Read-only users see the plain text.
 */
export function InlineText({ value, placeholder, editable, onSave }: { value: string; placeholder?: string; editable: boolean; onSave: (v: string) => void }) {
  const [editing, setEditing] = useState(false);
  if (!editable) return <span className="cell-text">{value}</span>;
  if (editing)
    return (
      <div className="ed editing">
        <TextInput value={value} autoFocus commitOnBlur onFocus={(e) => e.currentTarget.select()} onChange={(v) => v.trim() !== value && onSave(v.trim())} onBlur={() => setEditing(false)} />
      </div>
    );
  return (
    <div
      className="ed"
      role="button"
      tabIndex={0}
      onClick={() => setEditing(true)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          setEditing(true);
        }
      }}
    >
      <span className={value ? '' : 'ph'}>{value || placeholder}</span>
    </div>
  );
}
