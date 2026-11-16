import { useCallback, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
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

const byWebThenPort = (a: Address) => [...a.ports].sort((x, y) => Number(!!y.web) - Number(!!x.web) || x.port - y.port);
const moreTitle = (rest: Address['ports']) => rest.map((p) => `${p.port} ${p.web?.scheme ?? p.service ?? ''}`.trim()).join('\n');

/** Les ports ouverts d'une machine : lien pour les ports web. */
/** The open ports, web pages first; past `max`, a "+N" chip lists the others on hover. */
export function PortChips({ a, max }: { a: Address; max?: number }) {
  const sorted = byWebThenPort(a);
  const shown = max === undefined ? sorted : sorted.slice(0, max);
  const rest = sorted.slice(shown.length);
  return (
    <div className="ports">
      {shown.map((p) => (
        <PortChip key={p.port} ip={a.ip} p={p} svc />
      ))}
      {rest.length > 0 && (
        <span className="port-more" title={moreTitle(rest)}>
          +{rest.length}
        </span>
      )}
    </div>
  );
}

/**
 * The open ports, web pages first, as many as the cell can hold; then a "+N" chip lists the others on hover.
 * Every chip stays in the DOM (the hidden ones out of the flow) so that it can be measured.
 */
export function FitChips({ a }: { a: Address }) {
  const sorted = useMemo(() => byWebThenPort(a), [a]);
  const ref = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState(sorted.length);

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const ws = [...el.querySelectorAll<HTMLElement>(':scope > .port-chip')].map((c) => c.offsetWidth);
    const avail = el.clientWidth;
    const gap = parseFloat(getComputedStyle(el).columnGap) || 0;
    const more = el.querySelector<HTMLElement>(':scope > .port-more');
    // Largeur du « +N » : mesurée quand il est là (par caractère), sinon estimée.
    const char = more && more.textContent ? (more.offsetWidth - 14) / more.textContent.length : 6.7;
    const moreW = (n: number) => Math.ceil(14 + char * (String(n).length + 1));
    let k = ws.length;
    let used = ws.reduce((s, w, i) => s + w + (i ? gap : 0), 0);
    while (k > 0 && used + (k < ws.length ? gap + moreW(ws.length - k) : 0) > avail) {
      k--;
      used -= ws[k] + (k ? gap : 0);
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
  }, [measure, sorted]);

  const rest = sorted.slice(fit);
  return (
    <div className="ports fit" ref={ref}>
      {sorted.map((p, i) => (
        <PortChip key={p.port} ip={a.ip} p={p} svc hidden={i >= fit} />
      ))}
      {rest.length > 0 && (
        <span className="port-more" title={moreTitle(rest)}>
          +{rest.length}
        </span>
      )}
    </div>
  );
}

export function PortChip({ ip, p, svc, hidden, children }: { ip: string; p: Address['ports'][number]; svc?: boolean; hidden?: boolean; children?: ReactNode }) {
  const name = p.web?.scheme ?? p.service;
  const body = (
    <>
      {p.port}
      {svc && name && <span className="svc">{name}</span>}
      {children}
    </>
  );
  const cls = hidden ? 'port-chip fit-hid' : 'port-chip';
  if (!p.web) return <span className={cls}>{body}</span>;
  return (
    <a className={cls} tabIndex={hidden ? -1 : undefined} href={`${p.web.scheme}://${ip}:${p.port}/`} target="_blank" rel="noreferrer" title={p.web.title || t('Open the page')} onClick={(e) => e.stopPropagation()}>
      {body} <ExternalLink size={11} />
    </a>
  );
}

/**
 * A table cell that reads like text and becomes a field on click (Enter or leaving saves, Escape cancels).
 * Read-only users see the plain text.
 */
export function InlineText({ value, placeholder, editable, extra, onSave }: { value: string; placeholder?: string; editable: boolean; extra?: ReactNode; onSave: (v: string) => void }) {
  const [editing, setEditing] = useState(false);
  if (!editable)
    return extra ? (
      <div className="ed ro">
        <span>{value}</span>
        {extra}
      </div>
    ) : (
      <span className="cell-text">{value}</span>
    );
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
      {extra}
    </div>
  );
}
