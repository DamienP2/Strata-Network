import { Fragment, useEffect } from 'react';
import { create } from 'zustand';
import { Keyboard, X } from 'lucide-react';
import { t } from '../i18n';

const useShortcuts = create<{ open: boolean }>(() => ({ open: false }));
export const openShortcuts = () => useShortcuts.setState({ open: true });
const close = () => useShortcuts.setState({ open: false });

/** Les raccourcis de l'app : libellé (anglais, traduit à l'affichage) et touches. */
export function shortcutList(): { label: string; keys: string[] }[] {
  return [
    { label: t('Keyboard shortcuts'), keys: ['?'] },
    { label: t('Close the dialog or menu'), keys: [t('Esc')] },
    { label: t('Confirm the dialog'), keys: [t('Enter')] },
  ];
}

export function Keys({ keys }: { keys: string[] }) {
  return (
    <span className="kbd-keys">
      {keys.map((k, i) => (
        <Fragment key={i}>
          {i > 0 && <span className="kbd-plus">+</span>}
          <kbd>{k}</kbd>
        </Fragment>
      ))}
    </span>
  );
}

/** Le calque des raccourcis (touche ? ou menu utilisateur). */
export function ShortcutsOverlay() {
  const open = useShortcuts((s) => s.open);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== '?' || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName))) return;
      if (!useShortcuts.getState().open && document.querySelector('.modal-back')) return;
      e.preventDefault();
      useShortcuts.setState((s) => ({ open: !s.open }));
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);

  if (!open) return null;
  return (
    <div className="modal-back" onPointerDown={close}>
      <div className="modal kbd-modal" role="dialog" aria-labelledby="kbd-title" onPointerDown={(e) => e.stopPropagation()}>
        <header>
          <span className="dialog-icon">
            <Keyboard size={16} />
          </span>
          <h2 id="kbd-title">{t('Keyboard shortcuts')}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={close}>
            <X size={16} />
          </button>
        </header>
        <div className="kbd-rows">
          {shortcutList().map((s) => (
            <div key={s.label} className="kbd-row">
              <span>{s.label}</span>
              <Keys keys={s.keys} />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
