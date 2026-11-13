import { CircleHelp } from 'lucide-react';
import { t } from '../i18n';
import { navigate } from '../router';

/** Le bouton rond de l'aide, à droite de la pastille. */
export function HelpButton({ on }: { on: boolean }) {
  return (
    <button type="button" className={`learn-help ${on ? 'on' : ''}`} title={t('Help')} aria-current={on ? 'page' : undefined} onClick={() => navigate('/help')}>
      <CircleHelp size={17} />
    </button>
  );
}
