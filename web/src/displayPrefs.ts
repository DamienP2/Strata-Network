import { useCallback } from 'react';
import { create } from 'zustand';
import { api } from './store';

type Prefs = Record<string, unknown>;

/** Préférences d'affichage de l'utilisateur connecté, gardées sur le serveur (`/api/me/display`). */
export const useDisplayPrefs = create<{ user: string | null; prefs: Prefs }>(() => ({ user: null, prefs: {} }));

let pending: Prefs = {};
let timer: ReturnType<typeof setTimeout> | undefined;

/** Charge les préférences avant de dessiner les pages. */
export async function loadDisplayPrefs(userId: string) {
  let prefs: Prefs = {};
  try {
    prefs = (await api<Prefs>('GET', '/api/me/display')) ?? {};
  } catch {
    /* rendu avec les valeurs par défaut */
  }
  pending = {};
  useDisplayPrefs.setState({ user: userId, prefs });
}

export function resetDisplayPrefs() {
  clearTimeout(timer);
  pending = {};
  useDisplayPrefs.setState({ user: null, prefs: {} });
}

function flush(keepalive = false) {
  clearTimeout(timer);
  timer = undefined;
  if (!Object.keys(pending).length) return;
  const body = JSON.stringify(pending);
  pending = {};
  if (keepalive) {
    void fetch('/api/me/display', { method: 'PATCH', credentials: 'same-origin', keepalive: true, headers: { 'Content-Type': 'application/json' }, body }).catch(() => {});
  } else {
    void api('PATCH', '/api/me/display', JSON.parse(body)).catch(() => {});
  }
}

export function setDisplayPref(key: string, value: unknown) {
  useDisplayPrefs.setState((s) => {
    const prefs = { ...s.prefs };
    if (value === null || value === undefined) delete prefs[key];
    else prefs[key] = value;
    return { prefs };
  });
  pending[key] = value ?? null;
  clearTimeout(timer);
  timer = setTimeout(() => flush(), 600);
}

/** Comme useState, mais gardé pour l'utilisateur sur tous ses navigateurs. */
export function useDisplayPref<T>(key: string, fallback: T): [T, (v: T | ((old: T) => T)) => void] {
  const stored = useDisplayPrefs((s) => s.prefs[key]);
  const value = (stored === undefined ? fallback : stored) as T;
  const set = useCallback(
    (v: T | ((old: T) => T)) => {
      const old = useDisplayPrefs.getState().prefs[key];
      const prev = (old === undefined ? fallback : old) as T;
      setDisplayPref(key, typeof v === 'function' ? (v as (o: T) => T)(prev) : v);
    },
    [key],
  );
  return [value, set];
}

if (typeof window !== 'undefined') window.addEventListener('pagehide', () => flush(true));
