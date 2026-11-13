import { lazy, Suspense, useEffect, type ReactNode } from 'react';
import { PageGuard } from './ui/PageGuard';
import { useShallow } from 'zustand/react/shallow';
import { Bell, EthernetPort, Layers, LayoutGrid, ListTree, Radar, Radio, Server, type LucideIcon } from 'lucide-react';
import { t, tn, useLang } from './i18n';
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
import { useUnseenAlerts } from './net/model';

const Scan = lazy(() => import('./pages/Scan').then((m) => ({ default: m.Scan })));
const Vlans = lazy(() => import('./pages/Vlans').then((m) => ({ default: m.Vlans })));
const Switches = lazy(() => import('./pages/Switches').then((m) => ({ default: m.Switches })));
const Multicast = lazy(() => import('./pages/Multicast').then((m) => ({ default: m.Multicast })));
const Topology = lazy(() => import('./pages/Topology').then((m) => ({ default: m.Topology })));
const Alerts = lazy(() => import('./pages/Alerts').then((m) => ({ default: m.Alerts })));
const SystemPage = lazy(() => import('./pages/SystemPage').then((m) => ({ default: m.SystemPage })));
const Help = lazy(() => import('./pages/Help').then((m) => ({ default: m.Help })));

/**
 * Les pages principales, dans l'ordre des onglets de la barre du haut :
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
const all = () => true;
const MAIN_PAGES: MainPage[] = [
  { page: 'scan', path: '/scan', icon: Radar, label: () => t('Scan'), visible: all, render: (r) => (r.page === 'scan' ? <Scan networkId={r.networkId} ip={r.ip} /> : null) },
  { page: 'vlans', path: '/vlans', icon: Layers, label: () => t('VLAN'), visible: all, render: () => <Vlans /> },
  { page: 'switches', path: '/switches', icon: EthernetPort, label: () => t('Switches'), visible: all, render: (r) => (r.page === 'switches' ? <Switches id={r.id} /> : null) },
  { page: 'multicast', path: '/multicast', icon: Radio, label: () => t('Multicast'), visible: all, render: () => <Multicast /> },
  { page: 'topology', path: '/topology', icon: ListTree, label: () => t('Topology'), visible: all, render: () => <Topology /> },
  { page: 'alerts', path: '/alerts', icon: Bell, label: () => t('Alerts'), visible: all, render: () => <Alerts /> },
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
  const unseen = useUnseenAlerts(!!user);

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
                  {p.page === 'alerts' && unseen > 0 && <span className="nav-dot warn" title={tn(unseen, '{n} unseen alert', '{n} unseen alerts')} />}
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
        <PageGuard page={location.pathname}>
          <Suspense fallback={<div className="loading">{t('Loading…')}</div>}>
            {route.page === 'help' ? <Help /> : page ? page.render(route) : !home ? <Empty icon={LayoutGrid} text={t('No page')} /> : null}
          </Suspense>
        </PageGuard>
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
