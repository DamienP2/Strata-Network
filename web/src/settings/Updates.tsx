import { useCallback, useEffect, useRef, useState } from 'react';
import { CloudDownload, History, RefreshCw, RotateCcw, Save, Trash2, Upload } from 'lucide-react';
import { t } from '../i18n';
import { api, useApp } from '../store';
import { PageHead } from '../pages/SidePage';
import { Check, TextInput } from '../ui/fields';
import { confirmDialog } from '../ui/dialogs';
import { errorText } from '../ui/serverError';
import { fmtAgo, fmtBytes, fmtDate } from '../ui/format';
import { rich } from '../ui/rich';
import { Card, Field, FieldGroup } from './ui';

interface Release {
  version: string;
  name?: string;
  publishedAt?: string | number | null;
  notes?: string;
  size?: number | null;
  asset?: { size?: number | null } | null;
}
type Phase = 'idle' | 'checking' | 'downloading' | 'installing' | 'restarting' | 'error';
interface UpdateStatus {
  phase: Phase;
  progress: number;
  message: string;
  latest: Release | null;
  releases: Release[];
  lastCheck: string | number | null;
}
interface UpdateInfo {
  current: string;
  platform: string;
  package: string;
  repo: string;
  hasToken: boolean;
  autoCheck: boolean;
  previous: string | null;
  history: { from: string; to: string; at: string | number; outcome: string }[];
  status: UpdateStatus;
  supervised: boolean;
}

/** "1.2.10" > "1.2.9". */
export function cmpVersion(a: string, b: string): number {
  const pa = a.replace(/^v/, '').split(/[.+-]/).map((x) => Number(x) || 0);
  const pb = b.replace(/^v/, '').split(/[.+-]/).map((x) => Number(x) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d;
  }
  return 0;
}

const ACTIVE: Phase[] = ['checking', 'downloading', 'installing'];
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
let waiting = false;

/** Attend le retour du serveur sous une autre version, puis recharge la page. */
async function waitBack(from: string, onWait: (v: boolean) => void) {
  if (waiting) return;
  waiting = true;
  onWait(true);
  const { toast } = useApp.getState();
  const until = Date.now() + 120_000;
  while (Date.now() < until) {
    await sleep(1000);
    try {
      const h = await api<{ version: string }>('GET', '/api/health');
      if (h.version && h.version !== from) {
        toast(t('Strata {version} is running', { version: h.version }));
        setTimeout(() => location.reload(), 600);
        return;
      }
    } catch {
      /* toujours en redémarrage */
    }
  }
  waiting = false;
  onWait(false);
  toast(t('The server did not come back within 2 minutes: check its logs.'), 'error');
}

/** Système › Serveur › Mises à jour. */
export function Updates() {
  const toast = useApp((s) => s.toast);
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [restarting, setRestarting] = useState(waiting);
  const [token, setToken] = useState('');
  const [allowOlder, setAllowOlder] = useState(false);
  const file = useRef<HTMLInputElement>(null);

  const fail = useCallback((e: unknown) => toast(errorText(e), 'error', (e as Error).message), [toast]);
  const last = useRef<{ phase: Phase; current: string } | null>(null);
  const load = useCallback(async () => {
    try {
      const i = await api<UpdateInfo>('GET', '/api/system/update');
      setInfo(i);
      last.current = { phase: i.status?.phase ?? 'idle', current: i.current };
      if (i.status?.phase === 'restarting') void waitBack(i.current, setRestarting);
    } catch (e) {
      const l = last.current;
      // Le serveur a disparu pendant l'installation : il redémarre.
      if (l && (l.phase === 'installing' || l.phase === 'restarting')) void waitBack(l.current, setRestarting);
      else if (!waiting) fail(e);
    }
  }, [fail]);
  useEffect(() => void load(), [load]);

  // Pendant une phase active : relecture toutes les 700 ms.
  const phase = info?.status?.phase ?? 'idle';
  useEffect(() => {
    if (!ACTIVE.includes(phase)) return;
    const id = setInterval(() => void load(), 700);
    return () => clearInterval(id);
  }, [phase, load]);

  if (!info) return <div className="loading">{t('Loading…')}</div>;
  const st = info.status ?? { phase: 'idle', progress: 0, message: '', latest: null, releases: [], lastCheck: null };
  const latest = st.latest;
  const newer = !!latest && cmpVersion(latest.version, info.current) > 0;

  const check = async () => {
    try {
      const p = api('POST', '/api/system/update/check');
      setTimeout(() => void load(), 100);
      await p;
    } catch (e) {
      fail(e);
    }
    void load();
  };

  const install = async (version: string, older = false) => {
    const ok = await confirmDialog(
      older
        ? { title: t('Go back to version {version}?', { version }), message: t('The server restarts: it is unavailable for a few seconds.'), confirm: t('Go back'), danger: true }
        : { title: t('Install Strata {version}?', { version }), message: t('The server restarts: it is unavailable for a few seconds.'), confirm: t('Install') },
    );
    if (!ok) return;
    try {
      await api('POST', `/api/system/update/install?version=${encodeURIComponent(version)}`);
      void load();
    } catch (e) {
      fail(e);
    }
  };

  const rollback = async () => {
    if (!info.previous) return;
    const ok = await confirmDialog({ title: t('Go back to version {version}?', { version: info.previous }), message: t('The server restarts: it is unavailable for a few seconds.'), confirm: t('Go back'), danger: true });
    if (!ok) return;
    try {
      await api('POST', '/api/system/update/rollback');
      void waitBack(info.current, setRestarting);
    } catch (e) {
      fail(e);
    }
  };

  const upload = async (f: File) => {
    const ok = await confirmDialog({ title: t('Install {name}?', { name: f.name }), message: t('The server restarts: it is unavailable for a few seconds.'), confirm: t('Install') });
    if (!ok) return;
    try {
      await api<{ version: string }>('POST', `/api/system/update/upload?allowOlder=${allowOlder}`, f);
      void waitBack(info.current, setRestarting);
    } catch (e) {
      fail(e);
    }
  };

  const saveSettings = async (patch: { repo?: string; token?: string | null; autoCheck?: boolean }) => {
    try {
      await api('PUT', '/api/system/update/settings', { repo: info.repo, token: null, autoCheck: info.autoCheck, ...patch });
      if (patch.token !== undefined && patch.token !== null) {
        setToken('');
        toast(patch.token ? t('Token saved') : t('Token removed'));
      }
    } catch (e) {
      fail(e);
    }
    void load();
  };

  const busy = restarting || phase === 'restarting' || ACTIVE.includes(phase);
  const busyText = restarting || phase === 'restarting' ? t('Restarting…') : st.message || (phase === 'checking' ? t('Checking…') : phase === 'downloading' ? t('Downloading…') : t('Installing…'));
  const outcome = (o: string) => (o === 'installed' ? t('installed') : o === 'rolled back' ? t('rolled back') : o === 'restored automatically' ? t('restored automatically') : t(o));

  return (
    <>
      <PageHead title={t('Updates')} />
      <Card
        icon={<CloudDownload size={16} />}
        title={t('Updates')}
        actions={
          <button className="btn small" disabled={busy} onClick={() => void check()}>
            <RefreshCw size={13} /> {t('Check now')}
          </button>
        }
      >
        <div className="update-head">
          <div>
            <span className="set-label">{t('Installed')}</span>
            <span className="update-version">{info.current}</span>
            <span className="muted">{info.platform}</span>
          </div>
          <div className="update-latest">
            {busy ? (
              <span className="update-busy">
                <RefreshCw size={14} className="spin" /> {busyText}
                {phase === 'downloading' && <progress value={st.progress} max={1} />}
              </span>
            ) : phase === 'error' ? (
              <span className="update-error">{errorText(st.message)}</span>
            ) : newer && latest ? (
              <>
                <span>{rich(t('Version {v} is available ({date}).', { date: fmtDate(latest.publishedAt) }), { v: <strong>{latest.version}</strong> })}</span>
                <button className="btn primary" onClick={() => void install(latest.version)}>
                  <CloudDownload size={14} /> {t('Install {version}', { version: latest.version })}
                </button>
              </>
            ) : st.lastCheck ? (
              <span className="muted">{t('Up to date: latest version {v}, checked {ago}.', { v: latest?.version ?? info.current, ago: fmtAgo(st.lastCheck) })}</span>
            ) : (
              <span className="muted">{t('Not checked yet.')}</span>
            )}
          </div>
        </div>
        {newer && latest?.notes && (
          <details className="update-notes">
            <summary>{t('What’s new in {version}', { version: latest.version })}</summary>
            <pre>{latest.notes}</pre>
          </details>
        )}
        {st.releases.length > 0 && (
          <div className="update-list">
            <span className="set-label">{t('Available versions')}</span>
            <div className="update-rows">
              {st.releases.map((r) => {
                const here = r.version === info.current;
                const older = cmpVersion(r.version, info.current) < 0;
                return (
                  <div key={r.version} className={`update-row ${here ? 'here' : ''}`}>
                    <strong>{r.version}</strong>
                    <span className="muted">{fmtDate(r.publishedAt)}</span>
                    {(r.asset?.size ?? r.size) ? <span className="muted">{fmtBytes(r.asset?.size ?? r.size)}</span> : null}
                    <div className="spacer" />
                    {here ? (
                      <span className="badge">{t('Installed')}</span>
                    ) : (
                      <button className={`btn small ${older ? 'ghost' : ''}`} disabled={busy} onClick={() => void install(r.version, older)}>
                        {older ? <RotateCcw size={13} /> : <CloudDownload size={13} />} {older ? t('Go back') : t('Install')}
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        )}
        {info.history.length > 0 && (
          <div>
            {[...info.history]
              .reverse()
              .slice(0, 5)
              .map((h, i) => (
                <p key={i} className="hint">
                  <History size={12} />{' '}
                  {h.outcome === 'restored automatically'
                    ? t('Version {from} did not start: {to} was restored automatically', { from: h.from, to: h.to })
                    : t('{from} → {to} ({outcome}), {ago}', { from: h.from, to: h.to, outcome: outcome(h.outcome), ago: fmtAgo(h.at) })}
                </p>
              ))}
          </div>
        )}
        <div className="set-grid">
          <Field label={t('GitHub repository')}>
            <TextInput value={info.repo} commitOnBlur onChange={(repo) => void saveSettings({ repo: repo.trim() })} />
          </Field>
          <FieldGroup
            label={
              <>
                {t('Access token')} {info.hasToken && <span className="badge">{t('saved')}</span>}
              </>
            }
          >
            <input type="password" value={token} autoComplete="off" placeholder={info.hasToken ? '••••••••' : 'github_pat_…'} onChange={(e) => setToken(e.target.value)} />
            <div className="row">
              <button className="btn small" disabled={!token.trim()} onClick={() => void saveSettings({ token: token.trim() })}>
                <Save size={13} /> {t('Save the token')}
              </button>
              {info.hasToken && (
                <button className="btn small ghost" onClick={() => void saveSettings({ token: '' })}>
                  <Trash2 size={13} /> {t('Remove the token')}
                </button>
              )}
            </div>
          </FieldGroup>
          <FieldGroup label={t('Automatic check')}>
            <Check checked={info.autoCheck} onChange={(autoCheck) => void saveSettings({ autoCheck })}>
              {t('Look for a new version every 6 hours')}
            </Check>
          </FieldGroup>
        </div>
        <div className="update-offline">
          <div>
            <strong>{t('Offline update')}</strong>
            <code>{info.package}</code>
          </div>
          <Check checked={allowOlder} onChange={setAllowOlder}>
            {t('Allow older versions')}
          </Check>
          <button className="btn" disabled={busy} onClick={() => file.current?.click()}>
            <Upload size={14} /> {t('Import a package')}
          </button>
          <input
            ref={file}
            type="file"
            accept=".gz,.tgz,.zip,.pkg,.msi"
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              e.target.value = '';
              if (f) void upload(f);
            }}
          />
          {info.previous && (
            <button className="btn danger" disabled={busy} onClick={() => void rollback()}>
              <RotateCcw size={14} /> {t('Go back to {version}', { version: info.previous })}
            </button>
          )}
        </div>
      </Card>
    </>
  );
}
