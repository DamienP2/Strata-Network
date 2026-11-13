import { useCallback, useEffect } from 'react';
import { create } from 'zustand';
import { t } from '../i18n';
import { api, useApp, type LocalAccounts, type Role } from '../store';
import { setLeaveGuard } from '../router';
import { confirmDialog } from '../ui/dialogs';
import { errorText } from '../ui/serverError';

export type SsoProvider = 'entra' | 'okta' | 'keycloak' | 'google' | 'other';
export interface SsoConfig {
  enabled: boolean;
  provider: SsoProvider;
  issuer: string;
  clientId: string;
  clientSecret: string;
  groupsClaim: string;
  groups: { group: string; role: Role }[];
  defaultRole: Role | null;
  localAccounts: LocalAccounts;
}
export type HttpsMode = 'off' | 'selfSigned' | 'custom';
export type HttpMode = 'redirect' | 'httpsOnly' | 'open';
export interface HttpsConfig {
  mode: HttpsMode;
  port: number;
  http: HttpMode;
}
export interface ServerConfig {
  name: string;
  bind?: string | null;
  trustedOrigins?: string[];
  timezone?: string;
  security: { https: HttpsConfig; sso: SsoConfig };
  /* + les réglages propres à l'app */
  [key: string]: unknown;
}
export interface SystemInfo {
  version: string;
  os: string;
  arch: string;
  hostname: string;
  cpus: number;
  dataDir: string;
  bind: string;
  uptimeSecs: number;
  timezone: string;
  config?: ServerConfig;
  interfaces: { name: string; ip: string }[];
}

const DEFAULT_SSO: SsoConfig = { enabled: false, provider: 'entra', issuer: '', clientId: '', clientSecret: '', groupsClaim: 'groups', groups: [], defaultRole: null, localAccounts: 'adminOnly' };
const DEFAULT_HTTPS: HttpsConfig = { mode: 'off', port: 8443, http: 'redirect' };

/** Complète une configuration reçue (champs absents = valeurs par défaut). */
function complete(c: ServerConfig): ServerConfig {
  const sec = c.security ?? ({} as ServerConfig['security']);
  return { ...c, security: { ...sec, https: { ...DEFAULT_HTTPS, ...sec.https }, sso: { ...DEFAULT_SSO, ...sec.sso, groups: sec.sso?.groups ?? [] } } };
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

interface CfgStore {
  system: SystemInfo | null;
  /** La configuration enregistrée. */
  base: ServerConfig | null;
  /** La copie modifiable. */
  cfg: ServerConfig | null;
  saving: boolean;
}
const useCfg = create<CfgStore>(() => ({ system: null, base: null, cfg: null, saving: false }));

async function load(): Promise<boolean> {
  try {
    const system = await api<SystemInfo>('GET', '/api/system');
    const base = system.config ? complete(system.config) : null;
    useCfg.setState((s) => {
      const dirty = s.cfg && s.base && !same(s.cfg, s.base);
      return { system, base, cfg: dirty ? s.cfg : base };
    });
    return true;
  } catch {
    return false;
  }
}

const portOf = (bind: string | null | undefined) => (bind ? Number(/:(\d+)$/.exec(bind)?.[1]) || null : null);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** `GET /api/system` au chargement puis toutes les 5 s (onglet visible) ; copie modifiable `cfg`. */
export function useServerConfig() {
  const { system, cfg, base, saving } = useCfg();
  const dirty = !!cfg && !!base && !same(cfg, base);

  useEffect(() => {
    void load();
    const id = setInterval(() => {
      if (document.visibilityState === 'visible') void load();
    }, 5000);
    return () => clearInterval(id);
  }, []);

  const discard = useCallback(() => useCfg.setState((s) => ({ cfg: s.base })), []);

  // Demander avant de quitter la page avec des modifications.
  useEffect(() => {
    if (!dirty) return;
    const here = location.pathname;
    setLeaveGuard(
      (path) => path !== here,
      async () => {
        const ok = await confirmDialog({ title: t('Discard the changes?'), confirm: t('Discard'), danger: true });
        if (ok) discard();
        return ok;
      },
    );
    return () => setLeaveGuard(null);
  }, [dirty, discard]);

  const set = useCallback((fn: (c: ServerConfig) => ServerConfig) => useCfg.setState((s) => (s.cfg ? { cfg: fn(s.cfg) } : {})), []);

  const save = useCallback(async () => {
    const { cfg: next, base: before } = useCfg.getState();
    if (!next) return;
    const toast = useApp.getState().toast;
    useCfg.setState({ saving: true });
    try {
      await api('PUT', '/api/system/config', next);
    } catch (e) {
      useCfg.setState({ saving: false });
      toast(errorText(e), 'error', (e as Error).message);
      return;
    }
    toast(t('Settings saved'));
    // Le serveur redémarre à chaud.
    const oldPort = portOf(before?.bind);
    const newPort = portOf(next.bind);
    if (newPort && oldPort !== newPort && String(newPort) !== location.port) {
      await sleep(1500);
      location.href = `${location.protocol}//${location.hostname}:${newPort}${location.pathname}`;
      return;
    }
    useCfg.setState({ base: next });
    for (let i = 0; i < 15; i++) {
      await sleep(1000);
      if (await load()) break;
    }
    useCfg.setState({ saving: false });
  }, []);

  return { system, cfg, dirty, saving, set, save, discard, reload: load };
}
