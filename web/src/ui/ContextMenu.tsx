import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { create } from 'zustand';
import type { LucideIcon } from 'lucide-react';

export type MenuEntry = { icon?: LucideIcon; label: string; onClick: () => void; danger?: boolean; disabled?: boolean } | 'sep';

const useCtx = create<{ at: { x: number; y: number } | null; items: MenuEntry[] }>(() => ({ at: null, items: [] }));

/** Ouvre le menu sous le pointeur (clic droit ou bouton « … »). */
export function openContextMenu(e: { clientX: number; clientY: number; preventDefault?: () => void }, items: MenuEntry[]) {
  e.preventDefault?.();
  useCtx.setState({ at: { x: e.clientX, y: e.clientY }, items });
}
export const closeContextMenu = () => useCtx.setState({ at: null, items: [] });

const MARGIN = 8;

/** Monté une fois dans l'app. */
export function ContextMenuHost() {
  const { at, items } = useCtx();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (!at || !ref.current) return setPos(null);
    const w = ref.current.offsetWidth;
    const h = ref.current.offsetHeight;
    let left = Math.min(at.x, innerWidth - MARGIN - w);
    let top = at.y + h > innerHeight - MARGIN ? at.y - h : at.y; // au-dessus s'il ne tient pas
    left = Math.max(MARGIN, left);
    top = Math.max(MARGIN, Math.min(top, innerHeight - MARGIN - h));
    setPos({ left, top });
  }, [at, items]);

  useEffect(() => {
    if (!at) return;
    const down = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) closeContextMenu();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        closeContextMenu();
      }
    };
    document.addEventListener('pointerdown', down, true);
    document.addEventListener('keydown', key, true);
    window.addEventListener('resize', closeContextMenu);
    window.addEventListener('blur', closeContextMenu);
    return () => {
      document.removeEventListener('pointerdown', down, true);
      document.removeEventListener('keydown', key, true);
      window.removeEventListener('resize', closeContextMenu);
      window.removeEventListener('blur', closeContextMenu);
    };
  }, [at]);

  if (!at) return null;
  return createPortal(
    <div ref={ref} className="menu ctx-menu" role="menu" style={{ top: pos?.top ?? at.y, left: pos?.left ?? at.x, visibility: pos ? 'visible' : 'hidden' }}>
      {items.map((it, i) =>
        it === 'sep' ? (
          <div key={i} className="menu-sep" />
        ) : (
          <button
            key={i}
            role="menuitem"
            className={`ux1-mi ${it.danger ? 'danger' : ''}`}
            disabled={it.disabled}
            onClick={() => {
              closeContextMenu();
              it.onClick();
            }}
          >
            {it.icon && <it.icon size={14} />}
            {it.label}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}
