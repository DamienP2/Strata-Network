import { create } from 'zustand';

export type Theme = 'dark' | 'light' | 'system';
export const THEMES: Theme[] = ['dark', 'light', 'system'];
const KEY = 'strata.theme';
const isTheme = (v: unknown): v is Theme => v === 'dark' || v === 'light' || v === 'system';

function saved(): Theme {
  try {
    const v = localStorage.getItem(KEY);
    if (isTheme(v)) return v;
  } catch {
    /* navigation privée */
  }
  return 'dark';
}

const media = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: light)') : null;

function apply(theme: Theme) {
  const light = theme === 'light' || (theme === 'system' && !!media?.matches);
  if (light) document.documentElement.dataset.theme = 'light';
  else delete document.documentElement.dataset.theme;
}

/** Le thème choisi ; le menu utilisateur se redessine quand il change. */
export const useTheme = create<{ theme: Theme }>(() => ({ theme: saved() }));

/** Enregistre et applique le thème (les valeurs inconnues sont ignorées). */
export function setTheme(theme: string | null | undefined) {
  if (!isTheme(theme)) return;
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    /* */
  }
  useTheme.setState({ theme });
  apply(theme);
}

apply(useTheme.getState().theme);
// « Système » suit l'ordinateur en direct.
media?.addEventListener('change', () => apply(useTheme.getState().theme));
