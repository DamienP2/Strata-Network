import { useEffect, useRef, useState, type ReactNode } from 'react';
import { create } from 'zustand';
import { AlertTriangle, Info, X } from 'lucide-react';
import { t } from '../i18n';

interface DialogBase {
  title: string;
  message?: ReactNode;
}
interface ConfirmOpts extends DialogBase {
  confirm?: string;
  cancel?: string;
  danger?: boolean;
}
interface PromptOpts extends DialogBase {
  label?: string;
  value?: string;
  placeholder?: string;
  confirm?: string;
  password?: boolean;
}
type Request = { id: number } & (
  | { kind: 'confirm'; opts: ConfirmOpts; done: (v: boolean) => void }
  | { kind: 'prompt'; opts: PromptOpts; done: (v: string | null) => void }
  | { kind: 'alert'; opts: DialogBase; done: () => void }
);
type Pending = Request extends infer R ? (R extends Request ? Omit<R, 'id'> : never) : never;
let seq = 0;

/** La file d'attente : un dialogue à la fois. */
const useDialogs = create<{ queue: Request[] }>(() => ({ queue: [] }));
const push = (r: Pending) => useDialogs.setState((s) => ({ queue: [...s.queue, { ...r, id: ++seq } as Request] }));
const pop = () => useDialogs.setState((s) => ({ queue: s.queue.slice(1) }));

export function confirmDialog(opts: ConfirmOpts): Promise<boolean> {
  return new Promise((done) => push({ kind: 'confirm', opts, done }));
}
/** Le texte saisi, ou null si annulé ou vide. */
export function promptDialog(opts: PromptOpts): Promise<string | null> {
  return new Promise((done) => push({ kind: 'prompt', opts, done }));
}
export function alertDialog(opts: DialogBase): Promise<void> {
  return new Promise((done) => push({ kind: 'alert', opts, done }));
}

export function DialogHost() {
  const req = useDialogs((s) => s.queue[0]);
  if (!req) return null;
  return <Dialog key={req.id} req={req} />;
}

function Dialog({ req }: { req: Request }) {
  const [value, setValue] = useState(req.kind === 'prompt' ? (req.opts.value ?? '') : '');
  const input = useRef<HTMLInputElement>(null);
  const ok = useRef<HTMLButtonElement>(null);
  const danger = req.kind === 'confirm' && !!req.opts.danger;

  useEffect(() => {
    if (input.current) {
      input.current.focus();
      input.current.select();
    } else ok.current?.focus();
  }, []);

  const close = (yes: boolean) => {
    pop();
    if (req.kind === 'confirm') req.done(yes);
    else if (req.kind === 'prompt') req.done(yes && value.trim() ? value : null);
    else req.done();
  };

  const { title, message } = req.opts;
  const confirm = req.kind === 'alert' ? t('OK') : req.opts.confirm || t('Confirm');
  return (
    <div className="modal-back dialog-back" onPointerDown={() => close(false)}>
      <form
        className={`modal dialog ${danger ? 'danger' : ''}`}
        role="dialog"
        aria-labelledby="dialog-title"
        onPointerDown={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          close(true);
        }}
      >
        <header>
          <span className="dialog-icon">{danger ? <AlertTriangle size={16} /> : <Info size={16} />}</span>
          <h2 id="dialog-title">{title}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={() => close(false)}>
            <X size={16} />
          </button>
        </header>
        {message && <div className="dialog-message">{message}</div>}
        {req.kind === 'prompt' && (
          <label className="set-field">
            {req.opts.label && <span className="set-label">{req.opts.label}</span>}
            <input
              ref={input}
              type={req.opts.password ? 'password' : 'text'}
              value={value}
              placeholder={req.opts.placeholder}
              autoComplete={req.opts.password ? 'new-password' : 'off'}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => e.stopPropagation()}
            />
          </label>
        )}
        <footer>
          {req.kind !== 'alert' && (
            <button type="button" className="btn" onClick={() => close(false)}>
              {(req.kind === 'confirm' && req.opts.cancel) || t('Cancel')}
            </button>
          )}
          <button ref={ok} type="submit" className={`btn ${danger ? 'danger-solid' : 'primary'}`}>
            {confirm}
          </button>
        </footer>
      </form>
    </div>
  );
}
