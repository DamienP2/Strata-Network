import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

/** Zone vide dans un panneau : une icône, un mot, un bouton éventuel. */
export function Empty({ icon: Icon, text, children }: { icon: LucideIcon; text: string; children?: ReactNode }) {
  return (
    <div className="opx-void">
      <div className="opx-void-ic">
        <Icon size={22} />
      </div>
      <div className="opx-void-t">{text}</div>
      {children}
    </div>
  );
}
