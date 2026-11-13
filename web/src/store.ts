import { useEffect, useRef } from 'react';
import { create } from 'zustand';
import { lang, pickedByHand, setLang } from './i18n';
import { setTheme } from './theme';
import { resetDisplayPrefs } from './displayPrefs';

export type Role = 'admin' | 'operator' | 'viewer';
export const ROLES: Role[] = ['admin', 'operator', 'viewer'];

export interface PublicUser {
  id: string;
  username: string;
  displayName: string;
  role: Role;
  disabled: boolean;
  lang: string;
  theme: string;
  sso: boolean;
  roleFixed: boolean;
}

export type LocalAccounts = 'none' | 'adminOnly' | 'all';

export interface AuthState {
  needsSetup: boolean;
  user: PublicUser | null;
  serverName: string;
  /** Nom du bouton SSO (« Microsoft »…) quand la connexion par l'annuaire est active. */
  sso: string | null;
  localAccounts: LocalAccounts;
}

export interface Toast {
  id: number;
  message: string;
  kind: 'info' | 'error';
  /** Texte brut du serveur, en infobulle. */
  detail?: string;
}

export interface Metrics {
  machine: { cpu: number; cores: number; cpuName: string; memUsed: number; memTotal: number; hostname: string };
  /** Le processus du serveur. */
  process?: { cpu: number; mem: number; pid: number };
}

export interface Incident {
  kind: string;
  level: 'CRASH' | 'ERROR' | 'WARN';
  cat: string;
  title: string;
  detail: string;
  first: number | string;
  last: number | string;
  count: number;
  crash?: unknown;
}

export interface Health {
  version: string;
  startedAt: number | string;
  uptimeSecs: number;
  crashes: number;
  lastCrash: number | string | null;
  unseenCrash: boolean;
  errors: number;
  warnings: number;
  lastHourErrors: number;
  disk: { total: number; free: number };
  incidents: Incident[];
}

/** Les droits, comme le serveur les vérifie. */
export const can = {
  admin: (u: PublicUser | null | undefined) => u?.role === 'admin',
  edit: (u: PublicUser | null | undefined) => u?.role === 'admin' || u?.role === 'operator',
};

/** Un plantage pas encore vu ou des erreurs dans l'heure. */
export const needsLook = (h: Health | null | undefined) => !!h && (h.unseenCrash || h.lastHourErrors > 0);

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

export async function api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {};
  const raw = typeof Blob !== 'undefined' && body instanceof Blob; // un fichier part tel quel
  if (body !== undefined) headers['Content-Type'] = raw ? (body as Blob).type || 'application/octet-stream' : 'application/json';
  const res = await fetch(path, { method, credentials: 'same-origin', headers, body: body === undefined ? undefined : raw ? (body as Blob) : JSON.stringify(body) });
  if (res.status === 401 && !path.startsWith('/api/auth/')) {
    const a = useApp.getState().auth;
    if (a?.user) useApp.setState({ auth: { ...a, user: null } }); // session expirée → page de connexion
  }
  if (!res.ok) throw new ApiError(res.status, (await res.text()) || res.statusText);
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

/** `{"type":"changed","what":…}` : le nom de la liste modifiée ; `hello` après une reconnexion. */
export const changeEvents = new EventTarget();
/** Les autres messages du serveur, par type (`detail` = le message). */
export const serverEvents = new EventTarget();

/** Rappelle fn quand le serveur signale un changement de `what`, et après une reconnexion. */
export function useChanged(what: string, fn: () => void) {
  const ref = useRef(fn);
  ref.current = fn;
  useEffect(() => {
    const on = () => ref.current();
    changeEvents.addEventListener(what, on);
    changeEvents.addEventListener('hello', on);
    return () => {
      changeEvents.removeEventListener(what, on);
      changeEvents.removeEventListener('hello', on);
    };
  }, [what]);
}

interface Hello {
  type: 'hello';
  user: PublicUser;
  serverName: string;
  serverTime: number;
  timezone: string;
  [list: string]: unknown;
}

export interface ScanProgress {
  done: number;
  total: number;
  up: number;
}

export interface AppStore {
  auth: AuthState | null;
  /** La session a été fermée par une connexion du même utilisateur ailleurs. */
  elsewhere: boolean;
  connected: boolean;
  /** Heure du serveur moins heure locale (ms). */
  serverOffset: number;
  timezone: string;
  toasts: Toast[];
  metrics: Metrics | null;
  health: Health | null;
  /** Les scans en cours, par réseau (hello, puis messages `scan`). */
  scans: Record<string, ScanProgress>;

  init: () => Promise<void>;
  login: (username: string, password: string) => Promise<void>;
  setup: (username: string, password: string, displayName: string) => Promise<void>;
  logout: () => Promise<void>;
  connect: () => void;
  toast: (message: string, kind?: Toast['kind'], detail?: string) => void;
  loadMetrics: () => Promise<void>;
  loadHealth: () => Promise<void>;
}

/** Ce que le message d'accueil met dans le store. Ajouter ici les listes de l'app. */
function fromHello(m: Hello): Partial<AppStore> {
  return {
    serverOffset: (m.serverTime || Date.now()) - Date.now(),
    timezone: m.timezone || '',
    scans: (m.scans as Record<string, ScanProgress> | undefined) ?? {},
  };
}

let ws: WebSocket | null = null;
let wanted = false;
let retry = 500;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let greeted = false;
let toastId = 0;

function disconnect() {
  wanted = false;
  greeted = false;
  clearTimeout(retryTimer);
  const s = ws;
  ws = null;
  s?.close();
}

/** Applique la langue et le thème de l'utilisateur qui vient de se connecter. */
function adopt(user: PublicUser) {
  if (pickedByHand() && user.lang !== lang()) {
    const l = lang();
    void api('PUT', '/api/me/lang', { lang: l }).catch(() => {});
    user = { ...user, lang: l };
  } else setLang(user.lang);
  setTheme(user.theme || 'dark');
  return user;
}

export const useApp = create<AppStore>((set, get) => ({
  auth: null,
  elsewhere: false,
  connected: false,
  serverOffset: 0,
  timezone: '',
  toasts: [],
  metrics: null,
  health: null,
  scans: {},

  init: async () => {
    try {
      const auth = await api<AuthState>('GET', '/api/auth/state');
      if (auth.user) {
        setLang(auth.user.lang);
        setTheme(auth.user.theme || 'dark');
      }
      set({ auth });
    } catch {
      setTimeout(() => void get().init(), 2000); // le serveur démarre : réessayer
    }
  },

  login: async (username, password) => {
    const { user } = await api<{ user: PublicUser; token: string }>('POST', '/api/auth/login', { username, password });
    const auth = get().auth;
    set({ auth: { ...(auth ?? { serverName: '', sso: null, localAccounts: 'all' }), needsSetup: false, user: adopt(user) }, elsewhere: false });
  },

  setup: async (username, password, displayName) => {
    const { user } = await api<{ user: PublicUser; token: string }>('POST', '/api/auth/setup', { username, password, displayName });
    const auth = get().auth;
    set({ auth: { ...(auth ?? { serverName: '', sso: null, localAccounts: 'all' }), needsSetup: false, user: adopt(user) }, elsewhere: false });
  },

  logout: async () => {
    disconnect();
    try {
      await api('POST', '/api/auth/logout');
    } catch {
      /* déjà déconnecté */
    }
    resetDisplayPrefs();
    const auth = get().auth;
    set({ auth: auth ? { ...auth, user: null } : auth, connected: false, metrics: null, health: null, scans: {} });
  },

  connect: () => {
    if (ws && ws.readyState <= WebSocket.OPEN) return;
    wanted = true;
    clearTimeout(retryTimer);
    const sock = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`);
    ws = sock;
    sock.onmessage = (ev) => {
      let m: { type?: string; [k: string]: unknown };
      try {
        m = JSON.parse(String(ev.data));
      } catch {
        return;
      }
      switch (m.type) {
        case 'hello': {
          const h = m as Hello;
          retry = 500;
          const auth = get().auth;
          set({ ...fromHello(h), connected: true, auth: auth ? { ...auth, user: h.user ?? auth.user, serverName: h.serverName ?? auth.serverName } : auth });
          if (greeted) changeEvents.dispatchEvent(new Event('hello')); // reconnexion : recharger ce qui est ouvert
          greeted = true;
          break;
        }
        case 'changed':
          changeEvents.dispatchEvent(new Event(String(m.what)));
          break;
        case 'sessionEnded': {
          disconnect();
          const auth = get().auth;
          resetDisplayPrefs();
          set({ elsewhere: true, connected: false, auth: auth ? { ...auth, user: null } : auth });
          break;
        }
        case 'error':
          get().toast(String(m.message ?? ''), 'error');
          break;
        case 'scan': {
          const id = String(m.networkId ?? '');
          const scans = { ...get().scans };
          if (m.running) scans[id] = { done: Number(m.done) || 0, total: Number(m.total) || 0, up: Number(m.up) || 0 };
          else delete scans[id];
          set({ scans });
          serverEvents.dispatchEvent(new CustomEvent('scan', { detail: m }));
          break;
        }
        default:
          if (m.type) serverEvents.dispatchEvent(new CustomEvent(m.type, { detail: m }));
      }
    };
    sock.onclose = () => {
      if (ws !== sock) return;
      ws = null;
      set({ connected: false });
      if (!wanted || !get().auth?.user) return;
      retryTimer = setTimeout(() => get().connect(), retry);
      retry = Math.min(retry * 2, 5000);
    };
  },

  toast: (message, kind = 'info', detail) => {
    const id = ++toastId;
    set((s) => ({ toasts: [...s.toasts, { id, message, kind, detail }] }));
    setTimeout(() => set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })), 4000);
  },

  loadMetrics: async () => {
    try {
      set({ metrics: await api<Metrics>('GET', '/api/system/metrics') });
    } catch {
      /* la pastille garde la dernière valeur */
    }
  },

  loadHealth: async () => {
    if (!can.admin(get().auth?.user)) return;
    try {
      set({ health: await api<Health>('GET', '/api/system/health') });
    } catch {
      /* */
    }
  },
}));

/** L'heure du serveur, en ms. */
export const serverNow = () => Date.now() + useApp.getState().serverOffset;
