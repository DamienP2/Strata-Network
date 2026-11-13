import { lazy, Suspense, useEffect, type ReactNode } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { LayoutGrid, Server, type LucideIcon } from 'lucide-react';
import { t, useLang } from './i18n';
import { can, needsLook, useApp, type PublicUser } from './store';
import { navigate, useRoute, type Route } from './router';
import { loadDisplayPrefs, useDisplayPrefs } from './displayPrefs';
import { Login } from './pages/Login';
import { visibleSections } from './pages/systemSections';
import { Brand } from './ui/Brand';
import { StatusPill } from './ui/StatusPill';
import { HelpButton } from './ui/HelpButton';
import { UserMenu } from './ui/UserMenu';
import { DialogHost } from './ui/dialogs';
import { Toasts } from './ui/Toasts';
import { CrashBanner } from './ui/CrashBanner';
import { ShortcutsOverlay } from './ui/Shortcuts';
import { ContextMenuHost } from './ui/ContextMenu';
import { Empty } from './ui/Empty';

const SystemPage = lazy(() => import('./pages/SystemPage').then((m) => ({ default: m.SystemPage })));
const Help = lazy(() => import('./pages/Help').then((m) => ({ default: m.Help })));

/**
 * Les pages principales, dans l'ordre des onglets de la barre du haut.
 * Les pages réseau (Scan, VLAN, Switchs, Multicast, Topologie) s'ajoutent ici :
 * une variante dans `Route` (router.ts), une entrée dans cette liste.
 */
interface MainPage {
  page: Route['page'];
  path: string;
  icon: LucideIcon;
  label: () => string;
  visible: (u: PublicUser) => boolean;
  render: (route: Route) => ReactNode;
}
const MAIN_PAGES: MainPage[] = [
  {
    page: 'system',
    path: '/system',
    icon: Server,
    label: () => t('System'),
    visible: (u) => visibleSections(u).length > 0,
    render: (r) => (r.page === 'system' ? <SystemPage section={r.section} sub={r.sub} /> : null),
  },
];

function Shell({ route }: { route: Route }) {
  const { user, health } = useApp(useShallow((s) => ({ user: s.auth?.user, health: s.health })));
  const pages = user ? MAIN_PAGES.filter((p) => p.visible(user)) : [];
  const page = pages.find((p) => p.page === route.page);
  const home = pages[0]?.path;

  // L'accueil, ou une page interdite : la première page principale.
  useEffect(() => {
    if (route.page !== 'help' && !page && home) navigate(home, true);
  }, [route.page, page, home]);

  useEffect(() => {
    document.title = 'Strata Network';
  }, []);

  return (
    <>
      <nav className="topbar">
        <Brand />
        {pages.length > 0 && (
          <div className="tabs home-tabs">
            {pages.map((p) => {
              const on = route.page === p.page;
              return (
                <button key={p.page} className={on ? 'on' : ''} aria-current={on ? 'page' : undefined} onClick={() => navigate(p.path)}>
                  <p.icon size={14} /> <span className="tab-label">{p.label()}</span>
                  {p.page === 'system' && can.admin(user) && needsLook(health) && <span className="nav-dot warn" />}
                </button>
              );
            })}
          </div>
        )}
        <div className="spacer" />
        <StatusPill />
        <HelpButton on={route.page === 'help'} />
        <UserMenu named />
      </nav>
      <div className="content">
        <Suspense fallback={<div className="loading">{t('Loading…')}</div>}>
          {route.page === 'help' ? <Help /> : page ? page.render(route) : !home ? <Empty icon={LayoutGrid} text={t('No page')} /> : null}
        </Suspense>
      </div>
    </>
  );
}

export function App() {
  const route = useRoute();
  const { auth, init, connect } = useApp(useShallow((s) => ({ auth: s.auth, init: s.init, connect: s.connect })));
  const user = auth?.user;
  const lang = useLang((s) => s.lang); // tout est redessiné dans la nouvelle langue
  useEffect(() => void init(), [init]);
  useEffect(() => {
    if (user) connect();
  }, [user, connect]);
  const prefsOf = useDisplayPrefs((s) => s.user);
  const userId = user?.id;
  useEffect(() => {
    if (userId) void loadDisplayPrefs(userId);
  }, [userId]);
  if (!auth) return <div className="loading">{t('Connecting…')}</div>;
  if (!user)
    return (
      <>
        <Login key={lang} />
        <DialogHost />
        <Toasts />
      </>
    );
  if (prefsOf !== user.id) return <div className="loading">{t('Connecting…')}</div>;
  return (
    <div className="app" key={lang}>
      <Shell route={route} />
      {can.admin(user) && <CrashBanner />}
      <DialogHost />
      <ShortcutsOverlay />
      <ContextMenuHost />
      <Toasts />
    </div>
  );
}
