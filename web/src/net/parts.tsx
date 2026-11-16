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
