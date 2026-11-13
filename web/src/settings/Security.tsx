import { useCallback, useEffect, useState } from 'react';
import { Check as CheckIcon, Copy, KeyRound, Plug, Plus, RefreshCw, ShieldCheck, Upload, X } from 'lucide-react';
import { t, tc } from '../i18n';
import { api, ROLES, useApp, type LocalAccounts, type Role } from '../store';
import { PageHead } from '../pages/SidePage';
import { Check, NumberField, Segmented } from '../ui/fields';
import { Dropdown } from '../ui/Dropdown';
import { errorText } from '../ui/serverError';
import { fmtDate } from '../ui/format';
import { roleLabel } from '../ui/roles';
import { Card, Field, FieldGroup, SaveBar } from './ui';
import { useServerConfig, type HttpMode, type HttpsConfig, type HttpsMode, type ServerConfig, type SsoConfig, type SsoProvider } from './serverConfig';

/** Système › Accès › Sécurité. */
export function Security() {
  const { cfg, dirty, saving, set, save, discard } = useServerConfig();
  if (!cfg) return <div className="loading">{t('Loading…')}</div>;
  const setSso = (patch: Partial<SsoConfig>) => set((c) => ({ ...c, security: { ...c.security, sso: { ...c.security.sso, ...patch } } }));
  const setHttps = (patch: Partial<HttpsConfig>) => set((c) => ({ ...c, security: { ...c.security, https: { ...c.security.https, ...patch } } }));
  return (
    <>
      <PageHead title={t('Security')} />
      <SsoCard cfg={cfg} setSso={setSso} />
      <HttpsCard cfg={cfg} setHttps={setHttps} />
      <SaveBar show={dirty} saving={saving} onSave={() => void save()} onDiscard={discard} />
    </>
  );
}

/** L'issuer à saisir, selon le fournisseur. */
const ISSUER_HINT: Record<SsoProvider, string> = {
  entra: 'https://login.microsoftonline.com/<tenant>/v2.0',
  okta: 'https://<domain>.okta.com/oauth2/default',
  keycloak: 'https://<host>/realms/<realm>',
  google: 'https://accounts.google.com',
  other: 'https://…',
};

interface SsoTest {
  issuer: string;
  authorizationEndpoint: string;
  tokenEndpoint: string;
  keys: number;
}

function SsoCard({ cfg, setSso }: { cfg: ServerConfig; setSso: (p: Partial<SsoConfig>) => void }) {
  const toast = useApp((s) => s.toast);
  const sso = cfg.security.sso;
  const [test, setTest] = useState<{ ok: SsoTest } | { error: string } | null>(null);
  const [testing, setTesting] = useState(false);
  const callback = `${location.origin}/api/auth/sso/callback`;

  const providers: { value: SsoProvider; label: string }[] = [
    { value: 'entra', label: 'Microsoft Entra' },
    { value: 'okta', label: 'Okta' },
    { value: 'keycloak', label: 'Keycloak' },
    { value: 'google', label: 'Google' },
    { value: 'other', label: t('Other') },
  ];
  const roleOptions = ROLES.map((r) => ({ value: r, label: roleLabel(r) }));
  const setGroup = (i: number, patch: Partial<{ group: string; role: Role }>) => setSso({ groups: sso.groups.map((g, n) => (n === i ? { ...g, ...patch } : g)) });

  const runTest = async () => {
    setTesting(true);
    setTest(null);
    try {
      setTest({ ok: await api<SsoTest>('POST', '/api/security/sso/test', { issuer: sso.issuer.trim() }) });
    } catch (e) {
      setTest({ error: errorText(e) });
    }
    setTesting(false);
  };

  return (
    <Card
      icon={<KeyRound size={16} />}
      title={t('Sign-in with the company directory (SSO)')}
      actions={
        <Check checked={sso.enabled} onChange={(enabled) => setSso({ enabled })}>
          {sso.enabled ? t('Enabled') : t('Disabled')}
        </Check>
      }
    >
      {sso.enabled && (
        <>
          <FieldGroup label={t('Provider')}>
            <div>
              <Segmented value={sso.provider} options={providers} onChange={(provider) => setSso({ provider })} />
            </div>
          </FieldGroup>
          <div className="set-grid">
            <Field label={t('Tenant / Issuer')}>
              <input value={sso.issuer} placeholder={ISSUER_HINT[sso.provider]} onChange={(e) => setSso({ issuer: e.target.value })} />
            </Field>
            <Field label={t('Client ID')}>
              <input value={sso.clientId} onChange={(e) => setSso({ clientId: e.target.value })} />
            </Field>
            <Field label={t('Secret')}>
              <input type="password" value={sso.clientSecret} autoComplete="new-password" onChange={(e) => setSso({ clientSecret: e.target.value })} />
            </Field>
            <FieldGroup label={t('Redirect URL')}>
              <div className="row">
                <code className="grow-code">{callback}</code>
                <button
                  type="button"
                  className="btn small"
                  onClick={() =>
                    navigator.clipboard.writeText(callback).then(
                      () => toast(t('Copied')),
                      () => toast(t('Copy failed'), 'error'),
                    )
                  }
                >
                  <Copy size={13} /> {t('Copy')}
                </button>
              </div>
            </FieldGroup>
            <Field label={t('Groups claim')}>
              <input value={sso.groupsClaim} onChange={(e) => setSso({ groupsClaim: e.target.value })} />
            </Field>
          </div>
          <FieldGroup label={t('Roles by group')}>
            <div className="sso-groups">
              {sso.groups.map((g, i) => (
                <div key={i} className="row">
                  <input value={g.group} placeholder={t('Group')} onChange={(e) => setGroup(i, { group: e.target.value })} />
                  <div className="sso-role">
                    <Dropdown value={g.role} options={roleOptions} onChange={(role) => setGroup(i, { role })} />
                  </div>
                  <button type="button" className="icon-btn ghost" title={t('Remove')} onClick={() => setSso({ groups: sso.groups.filter((_, n) => n !== i) })}>
                    <X size={14} />
                  </button>
                </div>
              ))}
              <div>
                <button type="button" className="btn small" onClick={() => setSso({ groups: [...sso.groups, { group: '', role: 'viewer' }] })}>
                  <Plus size={13} /> {t('group')}
                </button>
              </div>
            </div>
          </FieldGroup>
          <div className="set-grid two">
            <FieldGroup label={t('No known group')}>
              <div>
                <Segmented<'none' | Role>
                  value={sso.defaultRole ?? 'none'}
                  options={[{ value: 'none', label: t('Refused') }, ...roleOptions]}
                  onChange={(v) => setSso({ defaultRole: v === 'none' ? null : v })}
                />
              </div>
            </FieldGroup>
            <FieldGroup label={t('Local accounts')}>
              <div>
                <Segmented<LocalAccounts>
                  value={sso.localAccounts}
                  options={[
                    { value: 'none', label: tc('accounts', 'Disabled') },
                    { value: 'adminOnly', label: t('Admins only (fallback)') },
                    { value: 'all', label: t('All') },
                  ]}
                  onChange={(localAccounts) => setSso({ localAccounts })}
                />
              </div>
            </FieldGroup>
          </div>
          <div className="row sso-test">
            <button type="button" className="btn" disabled={!sso.issuer.trim() || testing} onClick={() => void runTest()}>
              {testing ? <RefreshCw size={14} className="spin" /> : <Plug size={14} />} {t('Test the connection')}
            </button>
            {test && 'ok' in test && (
              <span className="sso-ok">
                <CheckIcon size={14} /> {test.ok.issuer} · {t('{n} keys', { n: test.ok.keys })}
              </span>
            )}
            {test && 'error' in test && <span className="set-error">{test.error}</span>}
          </div>
        </>
      )}
    </Card>
  );
}

const PEM_CERT = '-----BEGIN CERTIFICATE-----';
const PEM_KEY = '-----BEGIN PRIVATE KEY-----';

interface CertInfo {
  subject?: string;
  issuer?: string;
  notBefore?: number | string;
  notAfter?: number | string;
  selfSigned?: boolean;
  names?: string[];
}
/** `GET /api/security/tls` : `{certificate}` (ou `cert`), ou directement le certificat. */
type TlsState = (CertInfo & { certificate?: CertInfo | null; cert?: CertInfo | null }) | null;

function HttpsCard({ cfg, setHttps }: { cfg: ServerConfig; setHttps: (p: Partial<HttpsConfig>) => void }) {
  const toast = useApp((s) => s.toast);
  const https = cfg.security.https;
  const [tls, setTls] = useState<TlsState | null>(null);
  const [cert, setCert] = useState('');
  const [key, setKey] = useState('');
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setTls(await api<TlsState>('GET', '/api/security/tls'));
    } catch {
      setTls(null);
    }
  }, []);
  useEffect(() => void load(), [load]);

  const send = async (path: string, body?: unknown) => {
    setBusy(true);
    try {
      await api('POST', path, body);
      toast(t('Certificate saved'));
      setCert('');
      setKey('');
      void load();
    } catch (e) {
      toast(errorText(e), 'error', (e as Error).message);
    }
    setBusy(false);
  };

  const c = tls ? (tls.certificate ?? tls.cert ?? (tls.subject ? tls : null)) : null;
  return (
    <Card icon={<ShieldCheck size={16} />} title={t('HTTPS')}>
      <div className="set-grid">
        <FieldGroup label={t('Mode')}>
          <div>
            <Segmented<HttpsMode>
              value={https.mode}
              options={[
                { value: 'off', label: tc('https', 'Off') },
                { value: 'selfSigned', label: t('Self-signed') },
                { value: 'custom', label: t('Own certificate') },
              ]}
              onChange={(mode) => setHttps({ mode })}
            />
          </div>
        </FieldGroup>
        {https.mode !== 'off' && (
          <FieldGroup label={t('HTTPS port')}>
            <NumberField value={https.port} min={1} max={65535} onChange={(port) => setHttps({ port })} />
          </FieldGroup>
        )}
        {https.mode !== 'off' && (
          <FieldGroup label={t('HTTP')}>
            <div>
              <Segmented<HttpMode>
                value={https.http}
                options={[
                  { value: 'redirect', label: t('Redirect') },
                  { value: 'httpsOnly', label: t('HTTPS only') },
                  { value: 'open', label: t('Open') },
                ]}
                onChange={(http) => setHttps({ http })}
              />
            </div>
          </FieldGroup>
        )}
      </div>
      {https.mode !== 'off' && c && (
        <dl className="sys-list">
          <dt>{t('Subject')}</dt>
          <dd>{c.subject || '—'}</dd>
          <dt>{t('Issuer')}</dt>
          <dd>{c.selfSigned ? t('Self-signed') : c.issuer || '—'}</dd>
          {c.names && c.names.length > 0 && (
            <>
              <dt>{t('Names')}</dt>
              <dd>{c.names.join(', ')}</dd>
            </>
          )}
          <dt>{t('Valid until')}</dt>
          <dd>{fmtDate(c.notAfter)}</dd>
        </dl>
      )}
      {https.mode === 'selfSigned' && (
        <div>
          <button type="button" className="btn small" disabled={busy} onClick={() => void send('/api/security/tls/self-signed')}>
            <RefreshCw size={13} /> {t('Regenerate')}
          </button>
        </div>
      )}
      {https.mode === 'custom' && (
        <>
          <div className="set-grid two">
            <Field label={t('Certificate (PEM)')}>
              <textarea className="pem" rows={5} value={cert} placeholder={PEM_CERT} onChange={(e) => setCert(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
            </Field>
            <Field label={t('Private key (PEM)')}>
              <textarea className="pem" rows={5} value={key} placeholder={PEM_KEY} onChange={(e) => setKey(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
            </Field>
          </div>
          <div>
            <button type="button" className="btn small" disabled={busy || !cert.trim() || !key.trim()} onClick={() => void send('/api/security/tls/certificate', { cert, key })}>
              <Upload size={13} /> {t('Import the certificate')}
            </button>
          </div>
        </>
      )}
    </Card>
  );
}
