import type { ReactNode } from 'react';
import { Save } from 'lucide-react';
import { t } from '../i18n';

export function Card({ icon, title, children, actions }: { icon?: ReactNode; title: ReactNode; children?: ReactNode; actions?: ReactNode }) {
  return (
    <section className="set-card">
      <header>
        {icon /* size 16 */}
        <h2>{title}</h2>
        <div className="spacer" />
        {actions}
      </header>
      {children ? <div className="set-body">{children}</div> : null}
    </section>
  );
}

export function Field({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <label className="set-field">
      <span className="set-label">{label}</span>
      {children}
    </label>
  );
}

/** Un champ qui n'est pas un `<label>` (plusieurs contrôles dedans). */
export function FieldGroup({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <div className="set-field">
      <span className="set-label">{label}</span>
      {children}
    </div>
  );
}

/** La barre qui monte du bas tant que la copie locale diffère. */
export function SaveBar({ show, saving, onSave, onDiscard }: { show: boolean; saving: boolean; onSave: () => void; onDiscard: () => void }) {
  return (
    <div className={`save-bar ${show ? 'show' : ''}`} aria-hidden={!show}>
      <span>{t('Some changes are not saved.')}</span>
      <div className="spacer" />
      <button className="btn ghost" disabled={saving} onClick={onDiscard} tabIndex={show ? 0 : -1}>
        {t('Discard')}
      </button>
      <button className="btn primary" disabled={saving} onClick={onSave} tabIndex={show ? 0 : -1}>
        <Save size={14} /> {saving ? t('Applying…') : t('Save and apply')}
      </button>
    </div>
  );
}
