import { useApp } from '../store';

export function Toasts() {
  const toasts = useApp((s) => s.toasts);
  return (
    <div className="toasts">
      {toasts.map((x) => (
        <div key={x.id} className={`toast ${x.kind}`} title={x.detail && x.detail !== x.message ? x.detail : undefined}>
          {x.message}
        </div>
      ))}
    </div>
  );
}
