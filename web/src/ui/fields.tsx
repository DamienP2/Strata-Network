import { useEffect, useLayoutEffect, useRef, useState, type InputHTMLAttributes, type ReactNode } from 'react';
import { t } from '../i18n';

/* ---------- Segmented : remplace les boutons radio et les petites listes ---------- */

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  className,
}: {
  value: T;
  options: { value: T; label: ReactNode; title?: string; disabled?: boolean }[];
  onChange: (v: T) => void;
  className?: string;
}) {
  return (
    <div className={className ? `segmented ${className}` : 'segmented'}>
      {options.map((o) => (
        <button key={o.value} type="button" title={o.title} disabled={o.disabled} className={o.value === value ? 'on' : ''} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

/* ---------- Interrupteur (toute case à cocher) ---------- */

export function Check({ checked, onChange, children, disabled, title }: { checked: boolean; onChange: (v: boolean) => void; children?: ReactNode; disabled?: boolean; title?: string }) {
  return (
    <label className="check" title={title}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      {children}
    </label>
  );
}

/* ---------- Texte ---------- */

type TextInputProps = Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'> & {
  value: string;
  onChange: (v: string) => void;
  /** Valeur locale, envoyée en quittant le champ (ou Entrée). */
  commitOnBlur?: boolean;
  /** Zone qui suit son texte (plusieurs lignes). */
  autoGrow?: boolean;
};

export function TextInput({ value, onChange, commitOnBlur, autoGrow, onKeyDown, onBlur, ...rest }: TextInputProps) {
  const [local, setLocal] = useState(value);
  useEffect(() => setLocal(value), [value]);
  const area = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = area.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight + 2}px`;
  }, [local, autoGrow]);
  const shown = commitOnBlur ? local : value;
  const change = (v: string) => (commitOnBlur ? setLocal(v) : onChange(v));
  const commit = () => {
    if (commitOnBlur && local !== value) onChange(local);
  };
  if (autoGrow) {
    return (
      <textarea
        ref={area}
        rows={1}
        value={shown}
        placeholder={rest.placeholder}
        disabled={rest.disabled}
        className={rest.className}
        style={{ resize: 'none', overflow: 'hidden' }}
        onChange={(e) => change(e.target.value)}
        onKeyDown={(e) => e.stopPropagation()}
        onBlur={commit}
      />
    );
  }
  return (
    <input
      {...rest}
      value={shown}
      onChange={(e) => change(e.target.value)}
      onKeyDown={(e) => {
        e.stopPropagation(); // pas de raccourci pendant la saisie
        if (e.key === 'Enter' && commitOnBlur) e.currentTarget.blur();
        if (e.key === 'Escape' && commitOnBlur) {
          setLocal(value);
          e.currentTarget.blur();
        }
        onKeyDown?.(e);
      }}
      onBlur={(e) => {
        commit();
        onBlur?.(e);
      }}
    />
  );
}

/* ---------- Molette et pavé tactile ---------- */

let lastScroll = 0;
let hoverEl: Element | null = null;
let hoverSince = 0;
let acc = 0;
let tracking = false;

function track() {
  if (tracking) return;
  tracking = true;
  document.addEventListener('scroll', () => (lastScroll = performance.now()), { capture: true, passive: true });
  document.addEventListener(
    'pointermove',
    (e) => {
      const target = e.target instanceof Element ? e.target : null;
      const el = target?.closest('.num-field, input[type="number"]') ?? target;
      if (el !== hoverEl) {
        hoverEl = el;
        hoverSince = performance.now();
        acc = 0;
      }
    },
    { capture: true, passive: true },
  );
}

/**
 * Les pas d'un évènement de molette sur `el`, ou null si le champ n'est pas
 * armé (le pointeur n'y est posé que depuis peu, ou la page vient de défiler).
 */
function wheelSteps(el: Element, e: WheelEvent): number | null {
  const now = performance.now();
  if (!hoverEl || !el.contains(hoverEl) || now - hoverSince < 120 || now - lastScroll < 300) return null;
  const d = e.deltaY || e.deltaX;
  if (!d) return 0;
  const mult = e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
  // Un cran de molette = un pas ; le pavé tactile cumule 16 px par pas.
  if (e.deltaMode !== 0 || (Math.abs(d) >= 50 && Number.isInteger(d))) return -Math.sign(d) * mult;
  acc += d;
  const n = Math.trunc(acc / 16);
  acc -= n * 16;
  return -n * mult;
}

/** La même molette pour tout `<input type="number">`. */
export function installNumberWheel() {
  track();
  document.addEventListener(
    'wheel',
    (e) => {
      const el = e.target instanceof Element ? e.target.closest<HTMLInputElement>('input[type="number"]') : null;
      if (!el || el.disabled || el.readOnly) return;
      const steps = wheelSteps(el, e);
      if (steps === null) return;
      e.preventDefault();
      if (!steps) return;
      const step = Number(el.step) || 1;
      const min = el.min === '' ? -Infinity : Number(el.min);
      const max = el.max === '' ? Infinity : Number(el.max);
      const next = Math.min(max, Math.max(min, (Number(el.value) || 0) + steps * step));
      const decimals = (String(step).split('.')[1] ?? '').length + (Math.abs(steps) < 1 ? 1 : 0);
      el.value = next.toFixed(decimals);
      el.dispatchEvent(new Event('input', { bubbles: true }));
    },
    { passive: false },
  );
}

/* ---------- Nombre ---------- */

/** Évalue une expression simple (`100+20`, `1920/2`) ; null si invalide. */
export function evalExpr(src: string): number | null {
  const s = src.replace(/\s+/g, '').replace(/,/g, '.');
  if (!s || !/^[-+*/().\d]+$/.test(s)) return null;
  let i = 0;
  const num = (): number | null => {
    if (s[i] === '(') {
      i++;
      const v = sum();
      if (s[i] !== ')') return null;
      i++;
      return v;
    }
    if (s[i] === '-' || s[i] === '+') {
      const neg = s[i++] === '-';
      const v = num();
      return v === null ? null : neg ? -v : v;
    }
    const m = /^\d*\.?\d+|^\d+\.?/.exec(s.slice(i));
    if (!m) return null;
    i += m[0].length;
    return Number(m[0]);
  };
  const prod = (): number | null => {
    let v = num();
    while (v !== null && (s[i] === '*' || s[i] === '/')) {
      const op = s[i++];
      const r = num();
      if (r === null) return null;
      v = op === '*' ? v * r : v / r;
    }
    return v;
  };
  const sum = (): number | null => {
    let v = prod();
    while (v !== null && (s[i] === '+' || s[i] === '-')) {
      const op = s[i++];
      const r = prod();
      if (r === null) return null;
      v = op === '+' ? v + r : v - r;
    }
    return v;
  };
  const v = sum();
  return v !== null && i === s.length && Number.isFinite(v) ? v : null;
}

export function NumberField({
  label,
  value,
  onChange,
  step = 1,
  display = 1,
  precision = 0,
  suffix,
  min,
  max,
  title,
}: {
  label?: ReactNode;
  value: number;
  onChange: (v: number) => void;
  /** Le pas, en unités affichées. */
  step?: number;
  /** Facteur d'affichage (100 : pourcentage d'une valeur 0..1). */
  display?: number;
  precision?: number;
  suffix?: ReactNode;
  min?: number;
  max?: number;
  title?: string;
}) {
  const shown = (v: number) => (Number.isFinite(v) ? (v * display).toFixed(precision) : '');
  const [text, setText] = useState(shown(value));
  const [editing, setEditing] = useState(false);
  const field = useRef<HTMLLabelElement>(null);
  const latest = useRef({ value, onChange });
  latest.current = { value, onChange };
  useEffect(() => {
    if (!editing) setText(shown(value));
  }, [value, editing, display, precision]);

  const clamp = (v: number) => Math.min(max ?? Infinity, Math.max(min ?? -Infinity, v));
  const round = (v: number) => Number((v * display).toFixed(precision)) / display;
  const send = (v: number) => {
    const next = clamp(round(v));
    if (next !== latest.current.value) latest.current.onChange(next);
    return next;
  };
  const bump = (steps: number) => send(latest.current.value + (steps * step) / display);
  const cancelled = useRef(false);
  const commit = () => {
    if (cancelled.current) {
      cancelled.current = false;
      setEditing(false);
      setText(shown(latest.current.value));
      return;
    }
    const v = evalExpr(text);
    setEditing(false);
    if (v === null) setText(shown(value));
    else setText(shown(send(v / display)));
  };

  useEffect(() => {
    track();
    const el = field.current;
    if (!el) return;
    const wheel = (e: WheelEvent) => {
      const steps = wheelSteps(el, e);
      if (steps === null) return;
      e.preventDefault();
      if (steps) bump(steps);
    };
    el.addEventListener('wheel', wheel, { passive: false });
    return () => el.removeEventListener('wheel', wheel);
  });

  const drag = useRef<{ x: number; start: number } | null>(null);
  const tip = title ? `${title}\n${t('Wheel or trackpad to adjust · Shift ×10 · Alt ×0.1')}` : t('Wheel or trackpad to adjust · Shift ×10 · Alt ×0.1');
  return (
    <label className="num-field" ref={field} title={tip}>
      {label !== undefined && (
        <span
          className="scrub"
          onPointerDown={(e) => {
            e.preventDefault();
            e.currentTarget.setPointerCapture(e.pointerId);
            drag.current = { x: e.clientX, start: latest.current.value };
          }}
          onPointerMove={(e) => {
            const d = drag.current;
            if (!d) return;
            const mult = e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
            const steps = Math.round((e.clientX - d.x) / 2) * mult;
            send(d.start + (steps * step) / display);
          }}
          onPointerUp={() => (drag.current = null)}
          onPointerCancel={() => (drag.current = null)}
        >
          {label}
        </span>
      )}
      <input
        type="text"
        inputMode="decimal"
        value={text}
        onFocus={(e) => {
          setEditing(true);
          e.currentTarget.select();
        }}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === 'Enter') e.currentTarget.blur();
          else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
            e.preventDefault();
            const next = bump((e.key === 'ArrowUp' ? 1 : -1) * (e.shiftKey ? 10 : 1));
            setText(shown(next));
          } else if (e.key === 'Escape') {
            cancelled.current = true;
            e.currentTarget.blur();
          }
        }}
      />
      {suffix !== undefined && <span className="suffix">{suffix}</span>}
    </label>
  );
}
