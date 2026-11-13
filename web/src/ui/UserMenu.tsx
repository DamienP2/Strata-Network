import { ChevronDown, Keyboard, Languages, LogOut, SunMoon } from 'lucide-react';
import { LANGS, setLang, t, tc, useLang, type Lang } from '../i18n';
import { setTheme, THEMES, useTheme, type Theme } from '../theme';
import { api, useApp, type PublicUser } from '../store';
import { useMenu } from './NavMenu';
import { openShortcuts } from './Shortcuts';
import { errorText } from './serverError';
import { roleLabel } from './roles';

export const initial = (u: Pick<PublicUser, 'displayName' | 'username'>) => (u.displayName || u.username).trim().charAt(0).toUpperCase() || '?';

const themeLabel = (th: Theme) => (th === 'dark' ? tc('theme', 'Dark') : th === 'light' ? tc('theme', 'Light') : tc('theme', 'System'));

/** Le menu de l'utilisateur connecté : langue, thème, raccourcis, déconnexion. */
export function UserMenu({ named = false }: { named?: boolean }) {
  const user = useApp((s) => s.auth?.user);
  const logout = useApp((s) => s.logout);
  const toast = useApp((s) => s.toast);
  const lang = useLang((s) => s.lang);
  const theme = useTheme((s) => s.theme);
  const m = useMenu();
  if (!user) return null;
  const name = user.displayName || user.username;

  const keep = (u: PublicUser) => {
    const auth = useApp.getState().auth;
    if (auth?.user) useApp.setState({ auth: { ...auth, user: { ...auth.user, lang: u.lang, theme: u.theme } } });
  };
  const pickLang = (l: Lang) => {
    setLang(l);
    api<PublicUser>('PUT', '/api/me/lang', { lang: l }).then(keep, (e) => toast(errorText(e), 'error'));
  };
  const pickTheme = (th: Theme) => {
    setTheme(th);
    api<PublicUser>('PUT', '/api/me/theme', { theme: th }).then(keep, (e) => toast(errorText(e), 'error'));
  };

  return (
    <div className="ux1-user" ref={m.root} onKeyDown={m.onMenuKey}>
      <button
        ref={m.button}
        className={`ux1-user-btn ${named ? 'named' : ''} ${m.open ? 'on' : ''}`}
        aria-haspopup="menu"
        aria-expanded={m.open}
        title={named ? undefined : name}
        onClick={() => m.setOpen(!m.open)}
        onKeyDown={m.onButtonKey}
      >
        <span className="avatar">{initial(user)}</span>
        {named && <span className="user-name">{name}</span>}
        {named && <ChevronDown size={13} />}
      </button>
      {m.open && (
        <div className="menu ux1-umenu" role="menu">
          <div className="ux1-who">
            <span className="avatar">{initial(user)}</span>
            <div>
              <b>{name}</b>
              <small>{roleLabel(user.role)}</small>
            </div>
          </div>
          <div className="menu-sep" />
          <div className="ux1-mrow">
            <Languages size={14} /> {t('Language')}
            <div className="lang-switch" role="group" aria-label={t('Language')}>
              {LANGS.map((l) => (
                <button key={l.id} className={lang === l.id ? 'on' : ''} aria-pressed={lang === l.id} title={l.name} onClick={() => pickLang(l.id)}>
                  {l.label}
                </button>
              ))}
            </div>
          </div>
          <div className="ux1-mrow">
            <SunMoon size={14} /> {t('Theme')}
            <div className="lang-switch theme-switch" role="group" aria-label={t('Theme')}>
              {THEMES.map((th) => (
                <button key={th} className={theme === th ? 'on' : ''} aria-pressed={theme === th} onClick={() => pickTheme(th)}>
                  {themeLabel(th)}
                </button>
              ))}
            </div>
          </div>
          <button
            role="menuitem"
            className="ux1-mi"
            onClick={() => {
              m.setOpen(false);
              openShortcuts();
            }}
          >
            <Keyboard size={14} /> {t('Keyboard shortcuts')}
            <span className="ux1-cap">?</span>
          </button>
          <div className="menu-sep" />
          <button
            role="menuitem"
            className="ux1-mi danger"
            onClick={() => {
              m.setOpen(false);
              void logout();
            }}
          >
            <LogOut size={14} /> {t('Sign out')}
          </button>
        </div>
      )}
    </div>
  );
}
