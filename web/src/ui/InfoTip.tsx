import { Info } from 'lucide-react';

/** Le « i » après un libellé, quand une précision est indispensable. */
export function InfoTip({ text }: { text: string }) {
  return (
    <span className="ux3-info" title={text} aria-label={text} role="img" tabIndex={0}>
      <Info size={13} />
    </span>
  );
}
