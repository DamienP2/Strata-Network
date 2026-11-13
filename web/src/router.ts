import { useEffect, useState } from 'react';

/** Les pages de l'app. Ajouter ici une variante par nouvelle page principale. */
export type Route =
  | { page: 'home' }
  | { page: 'scan'; networkId?: string; ip?: string }
  | { page: 'vlans' }
  | { page: 'switches'; id?: string }
  | { page: 'multicast' }
  | { page: 'topology' }
  | { page: 'alerts' }
  | { page: 'system'; section?: string; sub?: string }
  | { page: 'help' };

export function parseRoute(pathname: string, search = ''): Route {
  const parts = pathname.split('/').filter(Boolean).map(decodeURIComponent);
  switch (parts[0]) {
    case 'scan':
      return { page: 'scan', networkId: parts[1], ip: new URLSearchParams(search).get('ip') ?? undefined };
    case 'vlans':
      return { page: 'vlans' };
    case 'switches':
      return { page: 'switches', id: parts[1] };
    case 'multicast':
      return { page: 'multicast' };
    case 'topology':
      return { page: 'topology' };
    case 'alerts':
      return { page: 'alerts' };
    case 'system':
      return { page: 'system', section: parts[1], sub: parts[2] };
    case 'help':
      return { page: 'help' };
    default:
      return { page: 'home' };
  }
}

interface LeaveGuard {
  /** Vrai si aller vers ce chemin quitte la page protégée. */
  leaves: (path: string) => boolean;
  /** Demande la permission de partir. */
  ask: () => Promise<boolean>;
}
let guard: LeaveGuard | null = null;
let here = typeof location !== 'undefined' ? location.pathname : '/';
let internal = false;

/** Demande avant de quitter une page non enregistrée (null pour retirer la garde). */
export function setLeaveGuard(leaves: LeaveGuard['leaves'] | null, ask?: LeaveGuard['ask']) {
  guard = leaves && ask ? { leaves, ask } : null;
}

function go(path: string, replace: boolean) {
  if (replace) history.replaceState(null, '', path);
  else history.pushState(null, '', path);
  here = path;
  internal = true;
  dispatchEvent(new PopStateEvent('popstate'));
  internal = false;
}

export function navigate(path: string, replace = false) {
  if (path === location.pathname + location.search) return;
  const g = guard;
  if (g && g.leaves(path)) {
    void g.ask().then((ok) => {
      if (!ok) return;
      if (guard === g) guard = null;
      go(path, replace);
    });
    return;
  }
  go(path, replace);
}

if (typeof window !== 'undefined') {
  // Retour arrière vers une autre page : demander d'abord.
  window.addEventListener(
    'popstate',
    (e) => {
      if (internal) return;
      const target = location.pathname + location.search;
      const g = guard;
      if (g && g.leaves(target)) {
        e.stopImmediatePropagation();
        history.pushState(null, '', here);
        void g.ask().then((ok) => {
          if (!ok) return;
          if (guard === g) guard = null;
          go(target, false);
        });
        return;
      }
      here = location.pathname;
    },
    true,
  );
  window.addEventListener('beforeunload', (e) => {
    if (guard) e.preventDefault();
  });
}

export function useRoute(): Route {
  const [route, setRoute] = useState(() => parseRoute(location.pathname, location.search));
  useEffect(() => {
    const on = () => setRoute(parseRoute(location.pathname, location.search));
    addEventListener('popstate', on);
    return () => removeEventListener('popstate', on);
  }, []);
  return route;
}
