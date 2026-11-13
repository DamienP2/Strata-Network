import type { CSSProperties, ReactNode } from 'react';
import { ExternalLink } from 'lucide-react';
import { t } from '../i18n';
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
export function PortChips({ a }: { a: Address }) {
  return (
    <div className="ports">
      {a.ports.map((p) => (
        <PortChip key={p.port} ip={a.ip} p={p} svc />
      ))}
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
