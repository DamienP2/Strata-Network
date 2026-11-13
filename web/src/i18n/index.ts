import { create } from 'zustand';
import { fr } from './fr';

export type Lang = 'en' | 'fr';
export const LANGS: { id: Lang; label: string; name: string }[] = [
  { id: 'en', label: 'EN', name: 'English' },
  { id: 'fr', label: 'FR', name: 'Français' },
];
const KEY = 'strata.lang';
const isLang = (l: unknown): l is Lang => l === 'en' || l === 'fr';
function first(): Lang {
  try {
    const saved = localStorage.getItem(KEY);
    if (isLang(saved)) return saved;
  } catch {
    /* navigation privée */
  }
  return navigator.language?.toLowerCase().startsWith('fr') ? 'fr' : 'en';
}
let current: Lang = first();
document.documentElement.lang = current;
/** La langue affichée ; les composants qui tiennent des textes se redessinent quand elle change. */
export const useLang = create<{ lang: Lang }>(() => ({ lang: current }));
export function setLang(l: string | null | undefined) {
  if (!isLang(l) || l === current) return;
  current = l;
  document.documentElement.lang = l;
  try {
    localStorage.setItem(KEY, l);
  } catch {
    /* */
  }
  useLang.setState({ lang: l });
}
export const lang = () => current;
let picked = false;
/** Langue choisie à la main sur la page de connexion : elle l'emporte sur celle enregistrée pour l'utilisateur. */
export function pickLang(l: Lang) {
  picked = true;
  setLang(l);
}
export const pickedByHand = () => picked;
/** Pour les dates et les nombres. */
export const locale = () => (current === 'fr' ? 'fr-FR' : undefined);
const DICT: Record<Lang, Record<string, string>> = { en: {}, fr };
/** Le texte dans la langue de l'interface, `{nom}` remplacé par vars. */
export function t(s: string, vars?: Record<string, string | number>): string {
  const out = DICT[current][s] ?? s;
  return vars ? out.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m)) : out;
}
/** Un mot court dont le français dépend du contexte : tc('theme', 'Light') cherche d'abord 'theme|Light' (« Clair »). */
export function tc(context: string, s: string, vars?: Record<string, string | number>): string {
  const key = `${context}|${s}`;
  return key in DICT[current] ? t(key, vars) : t(s, vars);
}
/** Singulier ou pluriel selon n (en français 0 et 1 sont au singulier), {n} remplacé. */
export function tn(n: number, one: string, other: string, vars?: Record<string, string | number>): string {
  const single = current === 'fr' ? Math.abs(n) < 2 : n === 1;
  return t(single ? one : other, { n, ...vars });
}
