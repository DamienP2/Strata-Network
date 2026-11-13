import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown } from 'lucide-react';
import { t } from '../i18n';

export interface DropOption<T extends string = string> {
  value: T;
  label: string;
  group?: string;
  note?: string;
  style?: CSSProperties;
  sub?: string;
  icon?: ReactNode;
  detail?: string;
  disabled?: boolean;
}

const MARGIN = 8;

/** La liste déroulante dessinée (jamais de `<select>` natif). */
export function Dropdown<T extends string>({
  value,
  options,
  onChange,
  title,
  placeholder = '—',
  className,
  disabled,
}: {
  value: T | null;
  options: DropOption<T>[];
  onChange: (v: T) => void;
  title?: string;
  placeholder?: string;
  className?: string;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(-1);
  const [pos, setPos] = useState<{ left: number; top?: number; bottom?: number; minWidth: number; up: boolean } | null>(null);
  const btn = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const rich = options.some((o) => o.icon !== undefined || o.sub !== undefined);
  const current = options.find((o) => o.value === value);

  const close = (focus = false) => {
    setOpen(false);
    if (focus) btn.current?.focus();
  };
  const show = () => {
    if (disabled) return;
    setHover(Math.max(0, options.findIndex((o) => o.value === value && !o.disabled)));
    setOpen(true);
  };
  const pick = (o: DropOption<T>) => {
    if (o.disabled) return;
    close(true);
    if (o.value !== value) onChange(o.value);
  };
  const move = (from: number, dir: 1 | -1) => {
    for (let i = from + dir; i >= 0 && i < options.length; i += dir) if (!options[i].disabled) return i;
    return from;
  };

  // Placement : sous le bouton, ou au-dessus s'il manque de place ; toujours dans la fenêtre.
  useLayoutEffect(() => {
    if (!open || !btn.current) return;
    const r = btn.current.getBoundingClientRect();
    const minWidth = Math.max(rich ? 260 : 160, r.width);
    const below = innerHeight - r.bottom;
    const up = below < 240 && r.top > below;
    let left = r.left;
    const w = Math.max(minWidth, list.current?.offsetWidth ?? minWidth);
    if (left + w > innerWidth - MARGIN) left = Math.max(MARGIN, innerWidth - MARGIN - w);
    setPos(up ? { left, bottom: innerHeight - r.top + 4, minWidth, up } : { left, top: r.bottom + 4, minWidth, up });
  }, [open, rich]);

  // Fermer : clic ailleurs, molette hors de la liste, redimensionnement.
  useEffect(() => {
    if (!open) return;
    const down = (e: PointerEvent) => {
      const n = e.target as Node;
      if (list.current?.contains(n) || btn.current?.contains(n)) return;
      close();
    };
    const wheel = (e: WheelEvent) => {
      if (!list.current?.contains(e.target as Node)) close();
    };
    const resize = () => close();
    document.addEventListener('pointerdown', down, true);
    document.addEventListener('wheel', wheel, { capture: true, passive: true });
    window.addEventListener('resize', resize);
    return () => {
      document.removeEventListener('pointerdown', down, true);
      document.removeEventListener('wheel', wheel, true);
      window.removeEventListener('resize', resize);
    };
  }, [open]);

  // L'option survolée défile en vue.
  useEffect(() => {
    if (open) list.current?.querySelector('.dd-opt.hover')?.scrollIntoView({ block: 'nearest' });
  }, [open, hover]);

  let lastGroup: string | undefined;
  return (
    <>
      <button
        ref={btn}
        type="button"
        className={`dd-btn ${open ? 'open' : ''} ${className ?? ''}`}
        title={title}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        onPointerDown={(e) => e.stopPropagation()}
        onClick={() => (open ? close() : show())}
        onKeyDown={(e) => {
          if (!open) {
            if (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              show();
            }
            return;
          }
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            setHover((h) => move(h, e.key === 'ArrowDown' ? 1 : -1));
          } else if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            const o = options[hover];
            if (o) pick(o);
          } else if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            close(true);
          } else if (e.key === 'Tab') close();
        }}
      >
        {current?.icon !== undefined && <span className="dd-icon">{current.icon}</span>}
        <span className="dd-label" style={current?.style}>
          {current ? current.label : placeholder}
        </span>
        {current?.detail && <span className="dd-detail">{current.detail}</span>}
        <ChevronDown size={12} />
      </button>
      {open &&
        createPortal(
          <div
            ref={list}
            role="listbox"
            className={`dd-list ${rich ? 'rich' : ''} ${pos?.up ? 'up' : ''}`}
            style={pos ? { left: pos.left, top: pos.top, bottom: pos.bottom, minWidth: pos.minWidth } : { visibility: 'hidden' }}
            onPointerDown={(e) => e.stopPropagation()}
          >
            {options.length === 0 && <div className="dd-empty">{t('Nothing to choose')}</div>}
            {options.map((o, i) => {
              const head = o.group !== undefined && o.group !== lastGroup ? o.group : null;
              lastGroup = o.group;
              const cls = `dd-opt ${i === hover ? 'hover' : ''} ${o.value === value ? 'on' : ''} ${o.disabled ? 'off' : ''}`;
              const check = <span className="dd-check">{o.value === value && <Check size={12} />}</span>;
              return (
                <div key={o.value}>
                  {head !== null && <div className="dd-group">{head}</div>}
                  <div role="option" aria-selected={o.value === value} aria-disabled={o.disabled} className={cls} onPointerEnter={() => !o.disabled && setHover(i)} onClick={() => pick(o)}>
                    {!rich && check}
                    {o.icon !== undefined && <span className="dd-icon">{o.icon}</span>}
                    {rich ? (
                      <span className="dd-text">
                        <span className="dd-main" style={o.style}>
                          {o.label}
                        </span>
                        {o.sub && <span className="dd-sub">{o.sub}</span>}
                      </span>
                    ) : (
                      <span className="dd-text" style={o.style}>
                        {o.label}
                      </span>
                    )}
                    {o.note && <span className="dd-note">{o.note}</span>}
                    {rich && check}
                  </div>
                </div>
              );
            })}
          </div>,
          document.body,
        )}
    </>
  );
}
