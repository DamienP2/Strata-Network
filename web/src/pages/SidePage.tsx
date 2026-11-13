import type { ReactNode } from 'react';
import type { LucideIcon } from 'lucide-react';

export interface SideItem {
  id: string;
  icon: LucideIcon;
  label: string;
  status?: 'ok' | 'warn';
  count?: number;
}
export interface SideGroup {
  title: string;
  items: SideItem[];
}

/** Liste à gauche, page à droite. */
export function SidePage({ groups, current, onPick, children, className }: { groups: SideGroup[]; current: string | undefined; onPick: (id: string) => void; children: ReactNode; className?: string }) {
  return (
    <div className={`projects-page ${className ?? ''}`}>
      <aside className="lib-nav">
        {groups.map((g) => (
          <div key={g.title} className="sys-group">
            <div className="lib-nav-section">
              <span className="grow">{g.title}</span>
            </div>
            {g.items.map((i) => (
              <div
                key={i.id}
                className={`lib-nav-item ${current === i.id ? 'on' : ''}`}
                role="link"
                tabIndex={0}
                aria-current={current === i.id ? 'page' : undefined}
                onClick={() => onPick(i.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    onPick(i.id);
                  }
                }}
              >
                <i.icon size={15} />
                <span className="grow">{i.label}</span>
                {i.status && <span className={`nav-dot ${i.status}`} />}
                {i.count !== undefined && <span className="count">{i.count}</span>}
              </div>
            ))}
          </div>
        ))}
      </aside>
      <main className="library">
        <div className="library-inner wide">{children}</div>
      </main>
    </div>
  );
}

/** Titre de page ; sous-titre seulement factuel ; action principale à droite. */
export function PageHead({ title, sub, children }: { title: ReactNode; sub?: ReactNode; children?: ReactNode }) {
  return (
    <div className="settings-head">
      <div>
        <h1>{title}</h1>
        {sub && <p className="muted">{sub}</p>}
      </div>
      <div className="spacer" />
      {children}
    </div>
  );
}

export interface SubTab {
  id: string;
  icon?: ReactNode;
  label: string;
  count?: number;
}

/** Les pages d'une section, sous son titre. */
export function SubTabs({ items, current, onPick, children }: { items: SubTab[]; current: string; onPick: (id: string) => void; children?: ReactNode }) {
  return (
    <div className="sys-tabrow">
      <div className="tabs sys-tabs" role="tablist">
        {items.map((i) => (
          <button key={i.id} role="tab" aria-selected={current === i.id} className={current === i.id ? 'on' : ''} onClick={() => onPick(i.id)}>
            {i.icon}
            <span>{i.label}</span>
            {i.count !== undefined && <span className="sys-tab-n">{i.count}</span>}
          </button>
        ))}
      </div>
      {children}
    </div>
  );
}
