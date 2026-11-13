import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { ChevronDown, type LucideIcon } from 'lucide-react';

/**
 * Un bouton qui ouvre un menu : ↓ / Entrée ouvre, ↑ ↓ Début Fin déplacent,
 * Échap et Tab ferment, un clic ailleurs ferme.
 */
export function useMenu() {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);

  const items = () => Array.from(root.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)') ?? []);
  const focusItem = (i: number) => {
    const list = items();
    if (!list.length) return;
    list[(i + list.length) % list.length].focus();
  };

  useEffect(() => {
    if (!open) return;
    const down = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', down, true);
    return () => document.removeEventListener('pointerdown', down, true);
  }, [open]);

  const onButtonKey = (e: ReactKeyboardEvent) => {
    if (!open && (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ')) {
      e.preventDefault();
      setOpen(true);
      requestAnimationFrame(() => focusItem(0));
    }
  };
  const onMenuKey = (e: ReactKeyboardEvent) => {
    const list = items();
    const i = list.indexOf(document.activeElement as HTMLElement);
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        focusItem(i + 1);
        break;
      case 'ArrowUp':
        e.preventDefault();
        focusItem(i < 0 ? -1 : i - 1);
        break;
      case 'Home':
        e.preventDefault();
        focusItem(0);
        break;
      case 'End':
        e.preventDefault();
        focusItem(-1);
        break;
      case 'Escape':
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
        button.current?.focus();
        break;
      case 'Tab':
        setOpen(false);
        break;
    }
  };
  return { open, setOpen, root, button, onButtonKey, onMenuKey };
}

export interface NavItem {
  id: string;
  icon: LucideIcon;
  label: string;
  on: boolean;
  onClick: () => void;
}

/** Un ensemble de pages dans la barre du haut. */
export function NavGroup({ icon: Icon, label, items, dot }: { icon: LucideIcon; label: string; items: NavItem[]; dot?: ReactNode }) {
  const m = useMenu();
  const on = items.some((i) => i.on);
  return (
    <div className="ux1-navgroup" ref={m.root} onKeyDown={m.onMenuKey}>
      <button
        ref={m.button}
        className={`${on ? 'on' : ''} ${m.open ? 'open' : ''}`}
        aria-haspopup="menu"
        aria-expanded={m.open}
        onClick={() => m.setOpen(!m.open)}
        onKeyDown={m.onButtonKey}
      >
        <Icon size={14} /> <span className="tab-label">{label}</span>
        <ChevronDown size={13} className="ux1-chev" />
        {dot}
      </button>
      {m.open && (
        <div className="menu ux1-navmenu" role="menu">
          {items.map((i) => (
            <button
              key={i.id}
              role="menuitem"
              className={`ux1-mi ${i.on ? 'on' : ''}`}
              aria-current={i.on ? 'page' : undefined}
              onClick={() => {
                m.setOpen(false);
                i.onClick();
              }}
            >
              <i.icon size={14} />
              {i.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
