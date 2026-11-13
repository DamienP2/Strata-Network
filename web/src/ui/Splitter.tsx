import { useRef } from 'react';
import { useDisplayPref } from '../displayPrefs';

/** Tailles de panneaux gardées pour l'utilisateur (`layout.<key>`). */
export function useLayout<T extends Record<string, number>>(key: string, defaults: T): [T, (patch: Partial<T>) => void, () => void] {
  const [sizes, setSizes] = useDisplayPref<Partial<T>>(`layout.${key}`, {});
  const merged = { ...defaults, ...sizes } as T;
  return [merged, (patch) => setSizes((old) => ({ ...old, ...patch })), () => setSizes({})];
}

/**
 * Poignée de redimensionnement de 7 px. `axis` x : largeur (col-resize), y :
 * hauteur. `dir` : +1 si tirer vers la droite / le bas agrandit, -1 sinon.
 * Double-clic : taille par défaut.
 */
export function Splitter({
  axis,
  value,
  onChange,
  onReset,
  min = 120,
  max = 900,
  dir = 1,
  edge,
}: {
  axis: 'x' | 'y';
  value: number;
  onChange: (v: number) => void;
  onReset?: () => void;
  min?: number;
  max?: number;
  dir?: 1 | -1;
  edge?: 'left' | 'right';
}) {
  const start = useRef<{ p: number; v: number } | null>(null);
  return (
    <div
      className={`splitter ${axis} ${edge ? `edge-${edge}` : ''}`}
      role="separator"
      aria-orientation={axis === 'x' ? 'vertical' : 'horizontal'}
      onPointerDown={(e) => {
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { p: axis === 'x' ? e.clientX : e.clientY, v: value };
        document.body.classList.add(`resizing-${axis}`);
      }}
      onPointerMove={(e) => {
        const s = start.current;
        if (!s) return;
        const d = ((axis === 'x' ? e.clientX : e.clientY) - s.p) * dir;
        onChange(Math.round(Math.min(max, Math.max(min, s.v + d))));
      }}
      onPointerUp={() => {
        start.current = null;
        document.body.classList.remove(`resizing-${axis}`);
      }}
      onPointerCancel={() => {
        start.current = null;
        document.body.classList.remove(`resizing-${axis}`);
      }}
      onDoubleClick={onReset}
    />
  );
}
