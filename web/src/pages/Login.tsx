import { useEffect, useState, type FormEvent } from 'react';
import { KeyRound, LogIn, ShieldCheck } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { LANGS, pickLang, t, useLang } from '../i18n';
import { ApiError, useApp } from '../store';
import { Brand } from '../ui/Brand';
import { friendlyError } from '../ui/serverError';

/** Le message d'un retour SSO raté (`?sso_error=…&sso_detail=…`), lu une fois. */
function ssoError(): (() => string) | null {
  const q = new URLSearchParams(location.search);
  const code = q.get('sso_error');
  if (!code) return null;
  const detail = q.get('sso_detail') ?? '';
  q.delete('sso_error');
  q.delete('sso_detail');
  const rest = q.toString();
  history.replaceState(null, '', location.pathname + (rest ? `?${rest}` : '') + location.hash);
  switch (code) {
    case 'noGroup':
      return () => t('None of your directory groups gives access to Strata.');
    case 'disabled':
      return () => t('This account is disabled.');
    case 'expired':
      return () => t('Signing in took too long: try again.');
    default:
      return () => t('The directory refused the sign-in: {detail}', { detail });
  }
}
/** La commande qui crée l'admin à distance. */
const SETUP_CMD = 'strata-server user add admin';

let firstSsoError: (() => string) | null | undefined;

export function Login() {
  const { auth, elsewhere, login, setup } = useApp(useShallow((s) => ({ auth: s.auth, elsewhere: s.elsewhere, login: s.login, setup: s.setup })));
  const lang = useLang((s) => s.lang);
  const needsSetup = !!auth?.needsSetup;
  const sso = auth?.sso ?? null;
  const [username, setUsername] = useState(needsSetup ? 'admin' : '');
  const [displayName, setDisplayName] = useState('');
  const [password, setPassword] = useState('');
  const [password2, setPassword2] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setErrorText] = useState<string | null>(() => {
    if (firstSsoError === undefined) firstSsoError = ssoError();
    return firstSsoError ? firstSsoError() : elsewhere ? t('Signed in on another computer.') : null;
  });
  const setError = (e: string | null) => {
    firstSsoError = null;
    setErrorText(e);
  };

  useEffect(() => {
    if (needsSetup) setUsername((u) => u || 'admin');
  }, [needsSetup]);

  const ssoName = sso === 'SSO' || !sso ? 'SSO' : sso;
  const fail = (e: unknown) => {
    const status = e instanceof ApiError ? e.status : 0;
    const raw = e instanceof Error ? e.message : String(e);
    if (status === 401) setError(t('Wrong user name or password.'));
    else if (status === 429) setError(t('Too many tries: wait a moment, then try again.'));
    else if (status === 403 && sso) setError(t('Sign in with {name}', { name: ssoName }));
    else setError(friendlyError(raw));
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    setError(null);
    if (needsSetup && password !== password2) {
      setError(t('The passwords do not match.'));
      return;
    }
    setBusy(true);
    try {
      if (needsSetup) await setup(username.trim(), password, displayName.trim());
      else await login(username.trim(), password);
    } catch (err) {
      fail(err);
      setBusy(false);
    }
  };

  const empty = !username.trim() || !password || (needsSetup && !password2);
  const next = encodeURIComponent(location.pathname + location.search);
  return (
    <div className="login-page">
      <div className="lang-switch login-lang" role="group" aria-label={t('Language')}>
        {LANGS.map((l) => (
          <button key={l.id} type="button" className={lang === l.id ? 'on' : ''} aria-pressed={lang === l.id} title={l.name} onClick={() => pickLang(l.id)}>
            {l.label}
          </button>
        ))}
      </div>
      <form className="login-card" onSubmit={submit}>
        <div className="login-head">
          <Brand stacked />
          {auth?.serverName && auth.serverName !== 'Strata' && auth.serverName !== 'Strata Network' && <p className="muted">{auth.serverName}</p>}
        </div>
        {needsSetup ? (
          <>
            <div className="login-intro">
              <ShieldCheck size={15} />
              <span>{t('Welcome! Create the administrator account.')}</span>
            </div>
            <label className="set-field">
              <span className="set-label">{t('User name')}</span>
              <input value={username} autoFocus autoComplete="username" onChange={(e) => setUsername(e.target.value)} />
            </label>
            <label className="set-field">
              <span className="set-label">{t('Display name')}</span>
              <input value={displayName} placeholder={t('Optional')} autoComplete="name" onChange={(e) => setDisplayName(e.target.value)} />
            </label>
            <label className="set-field">
              <span className="set-label">{t('Password')}</span>
              <input type="password" value={password} autoComplete="new-password" onChange={(e) => setPassword(e.target.value)} />
            </label>
            <label className="set-field">
              <span className="set-label">{t('Repeat the password')}</span>
              <input type="password" value={password2} autoComplete="new-password" onChange={(e) => setPassword2(e.target.value)} />
              <span className="set-hint">{t('At least 8 characters.')}</span>
            </label>
            {error && <div className="login-error">{error}</div>}
            <button type="submit" className="btn block primary" disabled={empty || busy}>
              <ShieldCheck size={15} /> {t('Create the administrator account')}
            </button>
            <p className="set-hint">
              {t('For security, this page only works on the server itself (http://localhost). Remotely, run this on the server:')} <code>{SETUP_CMD}</code>
            </p>
          </>
        ) : (
          <>
            {sso && (
              <>
                <a className="btn primary block sso-btn" href={`/api/auth/sso/start?next=${next}`}>
                  <KeyRound size={15} /> {t('Sign in with {name}', { name: ssoName })}
                </a>
                <div className="login-sep">{t('local account')}</div>
              </>
            )}
            <label className="set-field">
              <span className="set-label">{t('User name')}</span>
              <input value={username} autoFocus={!sso} autoComplete="username" onChange={(e) => setUsername(e.target.value)} />
            </label>
            <label className="set-field">
              <span className="set-label">{t('Password')}</span>
              <input type="password" value={password} autoComplete="current-password" onChange={(e) => setPassword(e.target.value)} />
            </label>
            {error && <div className="login-error">{error}</div>}
            <button type="submit" className={`btn block ${sso ? '' : 'primary'}`} disabled={empty || busy}>
              <LogIn size={15} /> {t('Sign in')}
            </button>
          </>
        )}
      </form>
    </div>
  );
}
