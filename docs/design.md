# Reproduire le design et les fonctions de base de l'application de référence

Reproduis exactement le design et les fonctions de base suivants dans cette application. Ils viennent d'une application de référence (serveur Rust + interface web React) dont l'utilisateur veut retrouver partout le même aspect et le même comportement : même pile technique, mêmes jetons CSS, mêmes composants, même connexion, mêmes mises à jour, même page Système, mêmes langues et thèmes. Tout ce qui suit est tiré du code réel de l'application de référence (valeurs exactes, extraits de code et de CSS) : ne réinvente rien, ne « modernise » rien, n'ajoute aucune fonction qui n'est pas demandée. Les fonctions métier de cette application-ci restent les siennes ; seul le cadre (coquille, design, connexion, système) est repris à l'identique.

Conventions de ce document (à remplacer partout) :

| Repère | Signification | Dans la référence |
| --- | --- | --- |
| `<Marque> <Module>` | nom affiché | `Strata Graphics` (marque « Strata », module « Graphics ») |
| `<app>` | identifiant court (clés localStorage, agent HTTP) | `strata` |
| `<bin>` | exécutable du serveur | `strata-server` |
| `<PFX>_` | préfixe des variables d'environnement | `CG_` |
| `<app>_session` | cookie de session | `cg_session` |
| `<owner>/<repo>` | dépôt GitHub des releases | — |

---

## 1. Règles d'interface (non négociables)

1. **Interface en français.** Le code écrit les textes en anglais dans `t('…')` ; chaque texte a sa traduction française dans `src/i18n/fr/*.ts` (vérifié par le script, § 5.7). Typographie française : espace insécable (U+00A0) avant `? ! : ;` et à l'intérieur de `« »`, apostrophe typographique `’`.
2. **Simple.** Une page = un titre, éventuellement un sous-titre factuel (un compte, une version, un état : « 3 utilisateurs »), et le contenu. Pas de paragraphe d'introduction.
3. **Aucun texte explicatif dans l'interface.** Pas de phrase qui explique ce que fait une page ou un réglage. Si une précision est indispensable : un `title` (infobulle dessinée, § 4.21) ou l'icône « i » (`InfoTip`).
4. **Ne jamais proposer de fonction superflue.** Ne pas ajouter de boutons, d'options, de pages, de statistiques ou de raccourcis « en plus ». Faire ce qui est demandé, rien d'autre.
5. **Contrôles dessinés uniquement.** Jamais de `<select>` natif (→ `Dropdown`), jamais `confirm()/prompt()/alert()` (→ `confirmDialog/promptDialog/alertDialog`), jamais de cases à cocher natives visibles (→ interrupteur `.check`), jamais de boutons radio (→ `Segmented`), jamais l'infobulle du navigateur (→ infobulles dessinées).
6. **Graphisme cohérent partout.** Toujours les jetons CSS et les classes de ce document ; pas de couleur, de rayon, de taille ou d'ombre en dur hors de ces valeurs ; mêmes tailles d'icônes, mêmes hauteurs de contrôles.
7. **Maquette avant toute nouvelle interface.** Avant de coder un nouvel écran ou un nouveau panneau : faire une maquette HTML statique qui charge le vrai CSS de l'app (`styles.css`, `styles-light.css`) et reprend le vrai DOM (mêmes classes que les composants existants), la montrer, la faire valider, puis seulement coder.
8. **Libellés courts.** Un à trois mots, verbe à l'infinitif pour les actions (« Installer », « Vérifier maintenant »), nom pour les sections (« Mises à jour »). Pas de point final sur les libellés.

---

## 2. Pile technique et architecture

### 2.1 Dépôt

```
Cargo.toml                 workspace ([workspace.package] version = "x.y.z", edition 2021)
crates/<app>-server/       le serveur (axum) : un seul exécutable qui embarque l'app web
  build.rs                 copies gzip de web/dist, icône Windows
  src/main.rs api.rs auth.rs sso.rs guard.rs web.rs config.rs paths.rs prefs.rs
      update.rs service.rs health.rs logs.rs perf.rs state.rs …
crates/<app>-tray/         lanceur de bureau (barre des menus macOS / zone de notification Windows)
web/                       React + TypeScript + Vite
  index.html  vite.config.ts  tsconfig.json  package.json
  scripts/i18n-check.mjs
  src/main.tsx App.tsx router.ts store.ts theme.ts displayPrefs.ts
  src/styles.css styles-light.css fonts/inter-{400,500,600,700,800}.woff2
  src/i18n/index.ts  src/i18n/fr/{index,common,…}.ts
  src/ui/  Brand Dropdown dialogs fields tooltips escape Empty InfoTip NavMenu StatusPill UserMenu Splitter ContextMenu serverError
  src/pages/ Login SidePage SystemPage Users
  src/settings/ Settings Updates Security ui serverConfig
```

Profil release : `[profile.release] lto = "thin"`, `codegen-units = 1`, `strip = true`. Dépendances du workspace : `serde = { version = "1", features = ["derive", "rc"] }`, `serde_json = { version = "1", features = ["preserve_order"] }`, `thiserror = "2"`, `anyhow = "1"`, `tracing = "0.1"`, `uuid = { version = "1", features = ["v4", "serde"] }`. Chaque crate : `version.workspace = true`, `edition.workspace = true`.

### 2.2 Serveur Rust

Dépendances (versions de la référence) : `axum = { version = "0.8", features = ["ws"] }`, `tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "fs", "signal", "net"] }`, `tower-http = { version = "0.6", features = ["fs", "cors", "trace"] }`, `rust-embed = { version = "8", features = ["mime-guess", "include-exclude"] }`, `clap = { version = "4", features = ["derive", "env"] }`, `argon2 = "0.5"`, `rpassword = "7"`, `directories = "6"`, `arc-swap = "1"`, `futures-util = "0.3"`, `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`, `ureq = { version = "3", default-features = false, features = ["rustls", "platform-verifier"] }`, `sha2 = "0.10"`, `ring = "0.17"`, `base64 = "0.22"`, `flate2 = "1"`, `tar = "0.4"`, `zip = { version = "9", default-features = false, features = ["deflate-flate2"] }`, `jiff = "0.2"`, `gethostname = "1"`, `if-addrs = "0.15"`, `sysinfo = { version = "0.37", default-features = false, features = ["system", "disk"] }`; HTTPS optionnel : `axum-server = { version = "0.8", features = ["tls-rustls-no-provider"] }`, `rustls 0.23`, `rcgen 0.14`, `x509-parser 0.18`. Unix : `libc`. Windows : `windows-service = "0.8"`. Build : `flate2` ; Windows : `winresource` (icône et `FileDescription` de l'exe).

**Ligne de commande (clap)** — `#[command(name = "<bin>", version, args_conflicts_with_subcommands = true)]` :

```
<bin> [run] [--bind ADDR (env <PFX>_BIND)] [--data-dir DIR (env <PFX>_DATA_DIR)] [--web-dir DIR (env <PFX>_WEB_DIR)]
            [--default-bind ADDR (caché, installateurs)] [--service (caché, Windows SCM)]
<bin> service install [--bind 0.0.0.0:8080] [--data-dir DIR] | uninstall | start | stop | status
<bin> user add <nom> [--role admin|…] [--data-dir]   (demande le mot de passe deux fois, rpassword)
<bin> user list | passwd <nom> | delete <nom>
```

`--version` imprime `<bin> x.y.z` (l'outil de mise à jour s'en sert, § 5.6). Les commandes `user` agissent sur le dossier du service installé s'il existe, sinon sur celui de l'utilisateur ; lancées avec sudo, elles redonnent `users.json` au propriétaire du dossier de données.

**Dossier de données** (`paths.rs`) : interactif `directories::ProjectDirs::from("com", "<Org>", "<Marque> <Module>").data_dir()` ; service `/var/lib/<app>-<module>` (Linux), `/Library/Application Support/<Marque> <Module>` (macOS), `%ProgramData%\<Marque> <Module>` (Windows). Contenu :

```
config.json            réglages du serveur (§ 2.3)
users.json             comptes, hachages Argon2 (0600)
prefs/<userId>.json    préférences par utilisateur (§ 5.9)
update/settings.json   {repo, token, autoCheck} (0600) · update/pending.json · update/history.json
health/journal-<jour>.jsonl (30 jours) · health/crashes/ (90 jours)
logs/<bin>.log         quand il tourne en service
runtime.json           {"bind", "version", "pid"} écrit à chaque démarrage, pour le lanceur
```

Toute écriture de JSON : `serde_json::to_vec_pretty`, écrite dans `<fichier>.json.tmp` puis `rename` (atomique) ; fichiers sensibles en `0600` sous Unix. `users.json` est relu quand sa date de modification change (la CLI peut agir pendant que le serveur tourne).

**`main.rs` → `serve()`** : la boucle de « redémarrage à chaud » reconstruit l'état et le listener sans quitter le processus quand les réglages changent.

```rust
pub const DEFAULT_BIND: &str = "0.0.0.0:8080";

pub async fn serve(args: RunArgs, shutdown: impl Future<Output = ()> + Send + 'static) -> anyhow::Result<()> {
    let data_dir = args.data_dir.clone().unwrap_or_else(paths::user_data_dir);
    health::start(&data_dir);                       // journal sur disque, rapports de plantage
    update::startup_guard(&data_dir);               // retour automatique si la nouvelle version échoue
    update::confirm_after(data_dir.clone(), Duration::from_secs(60));
    update::spawn_auto_check(data_dir.clone());
    let auth = Arc::new(auth::Auth::load(&data_dir)); // hors de la boucle : les sessions survivent
    if auth.needs_setup() { tracing::warn!("no user yet: open http://localhost:<port>/ on this machine …"); }
    loop {
        let config = config::ServerConfig::load(&data_dir);
        let wanted = args.bind.or(config.bind).or(args.default_bind).unwrap_or(DEFAULT_BIND.parse()?);
        // bind_retry : 20 essais × 250 ms si AddrInUse (l'ancien processus libère le port après une mise à jour) ;
        // échec sur une autre adresse → repli sur 0.0.0.0:8080 pour garder l'interface joignable.
        let listener = bind_retry(wanted).await?;
        std::fs::write(data_dir.join("runtime.json"), json!({"bind": bind, "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id()}).to_string())?;
        let shared: Shared = Arc::new(AppState { data_dir, auth: auth.clone(), config: ArcSwap::from_pointee(config),
            events: Events::new(256), bind, started_at, restart: Notify::new(), closing: watch::channel(false).0, /* + état de l'app */ });
        let app = api::router()
            .fallback(web::embedded)                       // ou ServeDir(--web-dir) avec index.html en repli
            .layer(axum::middleware::from_fn(web::compress))
            .layer(axum::middleware::from_fn_with_state(shared.clone(), guard::csrf))
            .layer(axum::middleware::from_fn(guard::every_answer))
            .layer(CorsLayer::permissive())
            .with_state(shared.clone());
        let (cycle, stop) = (shared.clone(), until_true(stop_rx.clone())); // stop_rx : watch mis à true par Ctrl-C / SIGTERM
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(async move { tokio::select! { _ = stop => {}, _ = cycle.restart.notified() => {} } let _ = cycle.closing.send(true); })
            .await?;
        /* arrêter les tâches de fond du cycle */
        if *stop_rx.borrow() { break; }
    }
    health::stopped(); // arrêt propre : pas compté comme plantage
    Ok(())
}
```

Journalisation : `tracing_subscriber::registry()` + `EnvFilter` (défaut `"info,tower_http=warn"`) + sortie console (ANSI si terminal) ou fichier `logs/<bin>.log` en service + la couche `logs::Ring` (§ 5.11). Arrêt : Ctrl-C et SIGTERM.

### 2.3 Réglages (`config.json`)

`#[derive(Serialize, Deserialize)] #[serde(rename_all = "camelCase")] struct ServerConfig` dont chaque champ a `#[serde(default)]` (ou `default = "fn"`) : `name` (défaut le nom de la marque), `bind: Option<SocketAddr>`, `trustedOrigins: Vec<String>` (omise si vide), `timezone`, `security: { https, sso }` + les réglages propres à l'app. `load()` : lit, valeurs par défaut si absent ou invalide (`tracing::error!`), migre, `normalize()`, réenregistre si changé. `validate()` renvoie `Err(String)` (« the server name cannot be empty »). Il est en mémoire dans `ArcSwap<ServerConfig>` (`s.config.load()`).

`PUT /api/system/config` (admin) : `normalize` + `validate` (400), garde le secret SSO masqué tel quel, teste l'adresse d'écoute si elle change, enregistre, puis `s.restart.notify_one()` (redémarrage à chaud) et `s.changed("config")` ; répond `202 Accepted`.
`GET /api/system` → `{version, os, arch, hostname, cpus, dataDir, bind, uptimeSecs, timezone, config (admins seulement, client_secret remplacé par "••••••••"), interfaces:[{name, ip}]}`.

### 2.4 Conventions d'API

- Tout sous `/api/...`, JSON en `camelCase` (`#[serde(rename_all = "camelCase")]`), méthodes REST (`GET` liste/lecture, `POST` création/action, `PUT` remplacement, `PATCH` fusion, `DELETE`).
- Erreurs : `type ApiResult<T> = Result<T, (StatusCode, String)>` ; le corps d'erreur est **du texte brut, court, en anglais, en minuscules** (« wrong user name or password », « no such user », « you do not have the rights for this »). Aides : `bad(e)` 400, `internal(e)` 500, `not_found("user")` 404 « no such user », `require(cond)` 403. Extracteur `CurrentUser` → 401 « login required » ; `MaybeUser(Option<User>)`. 409 conflit, 429 trop d'essais, 502 service distant (GitHub), `202 Accepted` pour une action lancée en fond, `204 No Content` pour suppression / déconnexion / réglage sans réponse.
- Travail bloquant (Argon2, fichiers, HTTP sortant) dans `tokio::task::spawn_blocking`.
- Jeton d'une requête (`auth::token_from`) : `Authorization: Bearer <jeton>`, sinon le cookie `<app>_session`.
- Identifiants : `uuid::Uuid::new_v4().simple().to_string()[..12]`.
- Route publique de supervision : `GET /api/health` → `{name, version, bind, needsSetup}` (sans connexion ; le lanceur et la page Mises à jour s'en servent).

Côté web, un seul client (`store.ts`) :

```ts
export class ApiError extends Error { constructor(public status: number, message: string) { super(message); } }
export async function api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {};
  const raw = typeof Blob !== 'undefined' && body instanceof Blob; // un fichier part tel quel
  if (body !== undefined) headers['Content-Type'] = raw ? (body as Blob).type || 'application/octet-stream' : 'application/json';
  const res = await fetch(path, { method, credentials: 'same-origin', headers, body: body === undefined ? undefined : raw ? (body as Blob) : JSON.stringify(body) });
  if (res.status === 401 && !path.startsWith('/api/auth/')) { const a = useApp.getState().auth; if (a?.user) useApp.setState({ auth: { ...a, user: null } }); } // session expirée → page de connexion
  if (!res.ok) throw new ApiError(res.status, (await res.text()) || res.statusText);
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}
```

Les erreurs s'affichent en toast (`toast((e as Error).message, 'error')`). Les erreurs connues sont traduites : par statut (page de connexion) ou par `friendlyError(raw)` (`ui/serverError.ts` : motif anglais → `t('…')`, texte brut en infobulle). Les « Failed to fetch / NetworkError… / Load failed » deviennent « Le serveur ne répond pas ».

### 2.5 Session, cookie, CSRF, en-têtes

- Cookie : `"<app>_session=<jeton>; Path=/; HttpOnly; SameSite=Lax; Max-Age=1209600"` + `; Secure` si la requête est arrivée en HTTPS (ou `X-Forwarded-Proto: https`). Déconnexion : même cookie vide avec `Max-Age=0`.
- Jeton : deux UUID v4 `simple()` concaténés (64 hex). Sessions **en mémoire** (`HashMap<jeton, {user_id, expires}>`), durée 14 jours **glissante** (prolongée à chaque requête). Elles survivent au redémarrage à chaud, pas au redémarrage du processus.
- CSRF (`guard::csrf`, middleware) : refuse en `403 "request sent by another site refused"` toute requête qui modifie (méthode ≠ GET/HEAD/OPTIONS, ou mise à niveau WebSocket) portée **par le seul cookie** (pas d'`Authorization`) quand elle vient d'un autre site : `Sec-Fetch-Site: cross-site`, ou `Origin` absent de `Host` / `X-Forwarded-Host` (port compris ; `Origin: null` = autre site), sauf si l'origine est dans `trustedOrigins`. Pas de jeton CSRF à gérer côté web.
- `guard::this_machine(peer, headers)` : adresse de bouclage, pas d'un autre site, `Host`/`Origin` locaux (`localhost`, `127.0.0.1`, `::1`). Sert à la création du premier compte et au lanceur.
- `guard::every_answer` : `X-Content-Type-Options: nosniff`, `Referrer-Policy: same-origin` sur chaque réponse.

### 2.6 Temps réel : WebSocket `/ws`

- `GET /ws` exige une session (401 sinon). Le serveur garde un `broadcast` d'évènements (`Events::new(256)`, chaque évènement sérialisé une seule fois) et les filtre par utilisateur.
- Message d'accueil : `{"type":"hello","user":PublicUser,"serverName":"…","serverTime":<ms>,"timezone":"Europe/Paris", …listes de l'app}`.
- Listes modifiées par un autre client : `{"type":"changed","what":"config"}` (`s.changed("…")` côté serveur ; le client refait le `GET` concerné). Côté web : `changeEvents` (EventTarget) et `useChanged(what, fn)` ; les autres types passent par `serverEvents`.
- `{"type":"sessionEnded"}` : le même utilisateur s'est connecté ailleurs (§ 5.2). `{"type":"error","message":"…"}` → toast d'erreur.
- Le serveur ferme le socket au redémarrage à chaud, quand l'utilisateur change (rôle, désactivé, mot de passe, supprimé) et quand le client est trop lent (`Lagged`) : le client se reconnecte et reçoit tout à neuf.
- Client : `connect()` dans le store ; reconnexion 500 ms puis ×2 jusqu'à 5 s ; pas de reconnexion si déconnecté ; `connected` dans le store alimente la pastille (§ 4.4) ; après une reconnexion, le `hello` fait recharger ce qui est ouvert.

### 2.7 L'app web servie par le binaire

- `web.rs` : `#[derive(RustEmbed)] #[folder = "../../web/dist"] struct WebApp;` ; `embedded(uri, headers)` sert le fichier demandé, sinon `index.html` (routes du client) ; `Cache-Control: public, max-age=31536000, immutable` pour `static/…` (fichiers empreintés par Vite), `no-cache` pour le reste ; si l'app n'est pas construite : 503 avec une petite page « Web app not built — run `cd web && npm install && npm run build` ».
- `build.rs` : crée `web/dist` s'il manque (le serveur compile sans l'UI), `cargo:rerun-if-changed=../../web/dist`, puis fait une copie **gzip niveau max** de chaque fichier texte (`js mjs css html svg json txt map ico xml`) dans `OUT_DIR/web-gz/`, sauf si < 512 octets ou gain < 10 %, et écrit la liste `&[("chemin", include_bytes!(…)), …]` incluse par `web.rs` : servie avec `Content-Encoding: gzip` + `Vary: accept-encoding` aux clients qui acceptent gzip (q=0 respecté).
- Middleware `compress` : gzip à la volée des réponses 200 JSON / texte / SVG / XML entre 1 Ko et 64 Mo (niveau 6, niveau 1 au-delà de 256 Ko, `spawn_blocking` au-delà).
- `--web-dir web/dist` : sert le dossier (`ServeDir` + `ServeFile(index.html)`) au lieu de la copie embarquée.

### 2.8 L'app web

`package.json` : `"type": "module"`, scripts `"dev": "vite"`, `"build": "tsc --noEmit && vite build"`, `"typecheck": "tsc --noEmit"`. Dépendances : `react ^19.3.1`, `react-dom ^19.3.1`, `zustand ^5.0.15`, `lucide-react ^1.57.0` ; dev : `typescript ~5.9.3`, `vite ^8.3.4`, `@vitejs/plugin-react ^6.1.2`, `@types/react ^19.3.0`, `@types/react-dom ^19.3.0`, `@types/node ^22`. Aucune autre bibliothèque d'interface (pas de routeur, pas de kit de composants, pas de CSS-in-JS) : tout est dans `styles.css`.

`tsconfig.json` : `target ES2022`, `lib ["ES2023","DOM","DOM.Iterable"]`, `module ESNext`, `moduleResolution bundler`, `jsx react-jsx`, `strict`, `noUnusedLocals`, `noUnusedParameters`, `noFallthroughCasesInSwitch`, `skipLibCheck`, `isolatedModules`, `resolveJsonModule`, `types ["vite/client","node"]`.

`vite.config.ts` :

```ts
const server = process.env.<PFX>_SERVER ?? 'http://localhost:8080';
export default defineConfig({
  plugins: [react()],
  build: {
    assetsDir: 'static', // /assets est réservé aux fichiers servis par le serveur
    rolldownOptions: { output: { codeSplitting: { groups: [
      { name: 'react', test: /node_modules[\\/](react|react-dom|scheduler|zustand|use-sync-external-store)[\\/]/ },
      { name: 'icons', test: /node_modules[\\/]lucide-react[\\/]/ },
    ] } } },
  },
  server: { port: 5173, proxy: { '/api': server, '/ws': { target: server.replace(/^http/, 'ws'), ws: true } } },
});
```

`index.html` : `<html lang="fr">`, favicons (`/favicon.ico`, `/favicon.svg`, `/apple-touch-icon.png`), `<title><Marque> <Module></title>`, et **avant les styles** un script qui pose le thème pour la première image :

```html
<script>
  try {
    var t = localStorage.getItem('<app>.theme');
    if (t === 'light' || (t === 'system' && matchMedia('(prefers-color-scheme: light)').matches)) document.documentElement.dataset.theme = 'light';
  } catch (e) {}
</script>
```

`main.tsx` : importe `./styles.css`, `./styles-light.css`, `./theme` ; appelle `installNumberWheel()`, `installTooltips()`, `installEscape()` ; rend `<StrictMode><App /></StrictMode>`.

**Store** : un store zustand (`create<AppStore>((set, get) => ({…}))`) contient `auth: AuthState | null`, `elsewhere`, `connected`, `serverOffset`, `timezone`, `toasts`, les listes de l'app, et les actions `init`, `login`, `setup`, `logout`, `connect`, `toast`. Les composants lisent avec des sélecteurs (`useApp((s) => s.x)`, `useShallow` pour plusieurs champs). `can = { admin: (u) => u?.role === 'admin', … }` reflète les droits du serveur.

**Routeur** (`router.ts`, pas de bibliothèque) : un type `Route` en union discriminée, `parseRoute(location.pathname)`, `navigate(path, replace = false)` (`history.pushState/replaceState` puis `dispatchEvent(new PopStateEvent('popstate'))`), `useRoute()` dans `App` (état + écoute de `popstate`), et `setLeaveGuard(leaves, ask)` pour demander avant de quitter une page non enregistrée (Back compris, `beforeunload` pour fermer l'onglet). Les sections système : `/system/<section>`.

**`App.tsx`** :

```tsx
export function App() {
  const route = useRoute();
  const { auth, init, connect } = useApp(useShallow((s) => ({ auth: s.auth, init: s.init, connect: s.connect })));
  const user = auth?.user;
  const lang = useLang((s) => s.lang); // tout est redessiné dans la nouvelle langue
  useEffect(() => void init(), [init]);
  useEffect(() => { if (user) connect(); }, [user, connect]);
  const prefsOf = useDisplayPrefs((s) => s.user);
  useEffect(() => { if (user) void loadDisplayPrefs(user.id); }, [user]);
  if (!auth) return <div className="loading">{t('Connecting…')}</div>;
  if (!user) return <Login key={lang} />;
  if (prefsOf !== user.id) return <div className="loading">{t('Connecting…')}</div>;
  return (
    <div className="app" key={lang}>
      <Shell route={route} />
      {can.admin(user) && <CrashBanner />}
      <DialogHost />
      <Toasts />
    </div>
  );
}
```

Les pages lourdes se chargent à l'ouverture (`lazy(() => import('./pages/X').then((m) => ({ default: m.X })))` dans `<Suspense fallback={<div className="loading">{t('Loading…')}</div>}>`). `document.title = '<Marque> <Module>'`.

### 2.9 Service système et lanceur de bureau

- `<bin> service install` : **systemd** (`Restart=always`, `RestartSec=2`, `After=network-online.target`, utilisateur dédié, `NoNewPrivileges=yes`, `ProtectSystem=full`, `ReadWritePaths=<data>`, `AmbientCapabilities=CAP_NET_BIND_SERVICE`, `LimitNOFILE=65536`, `ExecStart="<exe>" run --default-bind <bind> --data-dir "<data>"`), **launchd** (`/Library/LaunchDaemons/com.<org>.server.plist`, `KeepAlive` → `SuccessfulExit` = false), **service Windows** (actions de reprise sur échec). Le serveur sait qu'il est supervisé par `INVOCATION_ID` (systemd), `XPC_SERVICE_NAME` (launchd), `--service` (Windows) ou `<PFX>_SUPERVISED=1`.
- **Lanceur** (`crates/<app>-tray`, binaire `<app>-tray`, macOS et Windows seulement ; sous Linux il affiche un message et quitte) : `tray-icon = "0.26"`, `tao = "0.37"`, `ureq = "3"`, `serde_json` ; `#![cfg_attr(windows, windows_subsystem = "windows")]`. Instance unique : `TcpListener::bind("127.0.0.1:47819")`. Port lu dans `<data>/runtime.json`, sinon `config.json`, sinon 8080. Interroge toutes les 2 s `GET http://localhost:<port>/api/system/tray` (réservé à la machine, sans mot de passe ; repli `/api/health`) → `{name, version, bind, uptimeSecs, urls, interfaces, control, update:{available, phase, progress, message}}`. Menu (mots en français ou en anglais selon la langue du système) :
  `● <Marque> <Module> — en marche` / `○ … — arrêté` / `… — vérification…` · `Version {v} · port {p} · en marche depuis {t}` · `Copier l’adresse  {url}` · — · `Ouvrir la page web` · — · `Carte réseau ▸` (Interface web : Toutes les cartes / chaque carte, cochée) · `Port de connexion : {p}…` (boîte de saisie système) · — · `Démarrer le serveur` / `Arrêter le serveur` · `Redémarrer le serveur` · `Rechercher une mise à jour` / `Installer la version {v}…` / `Téléchargement de la version {v}… {n} %` · — · `Journal du serveur…` · `Désinstaller <Marque> <Module>…` · `Quitter`.
  Routes qu'il appelle (autorisées à la machine elle-même ou à un admin) : `PUT /api/system/network {control?, port?}` (carte et port de l'interface web, refus si le port est pris), `POST /api/system/restart-server` (redémarre le processus), `POST /api/system/stop` (Unix : SIGTERM à soi-même), `POST /api/system/update/check`, `POST /api/system/update/install`.

### 2.10 Build, CI, versions, paquets

- Développement : `cd web && npm install && npm run build && cd .. && cargo run --release -p <app>-server -- --data-dir data` → http://localhost:8080 ; ou `npm run dev` (port 5173, relaie vers 8080).
- CI : `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cd web && npm ci && npm run build` ; ajouter `node scripts/i18n-check.mjs` à l'étape web.
- Version unique : `[workspace.package] version`, recopiée dans `web/package.json`. En release, un script prend la version du tag `vX.Y.Z` (complète `v1.2` → `1.2.0`) et l'écrit dans `Cargo.toml`, `Cargo.lock` et `web/package.json`.
- Release GitHub (tag `v*`) : un job construit `web/dist` (artefact), les jobs plateforme construisent `cargo build --release --locked -p <app>-server` (+ `-p <app>-tray` sur macOS/Windows), produisent les installateurs **et le paquet de mise à jour** `<Marque>-<version>-<plateforme>.tar.gz` (`linux-x86_64`, `linux-arm64`, `macos-arm64`, `macos-x86_64`) ou `.zip` (`windows-x86_64`) qui contient le dossier `<Marque>-<version>-<plateforme>/<bin>` : c'est ce fichier que les serveurs installés téléchargent (§ 5.6).

---

## 3. Système de design (CSS prêt à coller)

### 3.1 Polices et jetons

Police : **Inter**, embarquée (pas de CDN, fonctionne hors ligne) en WOFF2 400, 500, 600, 700, 800 dans `web/src/fonts/`, découpée aux écritures latines (`pyftsubset Inter-Regular.otf --flavor=woff2 --layout-features='kern,liga,calt,tnum,ccmp,locl,mark,mkmk,case' --unicodes=<plage ci-dessous>`, ~75 ko par graisse). Icônes : **lucide-react** uniquement.

```css
@font-face { font-family: 'Inter'; font-weight: 400; font-display: swap; src: url('./fonts/inter-400.woff2') format('woff2');
  unicode-range: U+0000-024F, U+0259, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0300-036F, U+0391-03C9, U+1E00-1EFF, U+2000-20C0, U+2100-214F, U+2190-23FF, U+25A0-27BF, U+2C60-2C7F, U+A720-A7FF, U+FEFF, U+FFFD; }
/* idem pour 500, 600, 700 et 800 (inter-500.woff2 … inter-800.woff2), même unicode-range */

:root {
  --bg: #0b0c12; --panel: #12141d; --panel-2: #181b26; --panel-3: #1f2331;
  --line: #262a3a; --line-2: #323750;
  --text: #e6e8f2; --text-2: #a3a8bf; --text-3: #8a90ad;
  --accent: #8fa3ff; --accent-2: #bfcaff; --accent-soft: rgba(143, 163, 255, 0.16);
  --on-accent: #0b1236;                                  /* encre sur l'accent (pervenche claire : encre sombre) */
  --live: #ff3b5c; --live-soft: rgba(255, 59, 92, 0.16); /* rouge : erreur, danger, perdu (points, bordures) */
  --live-btn: #e11d48;                                   /* boutons rouges pleins à texte blanc */
  --pvw: #22c55e;                                        /* vert : ok, connecté */
  --warn: #f5a524;                                       /* orange : attention */
  --key: #f5c542; --mask: #2dd4bf;                       /* jaune, turquoise */
  --radius: 8px;
  --fs-11: 11px; --fs-12: 12px; --fs-13: 13px; --fs-15: 15px; --fs-20: 20px; /* rien sous 11 px */
  --h-control: 32px;        /* champs, listes, boutons des pages */
  --h-control-dense: 28px;  /* .btn.small */
  --hit-min: 28px;          /* boutons icône, interrupteurs */
  --mono: ui-monospace, 'SF Mono', 'Cascadia Mono', Consolas, 'DejaVu Sans Mono', monospace;
  color-scheme: dark;
}

/* styles-light.css — thème clair : seulement des surcharges ; le sombre est le défaut */
:root[data-theme='light'] {
  --bg: #eef0f5; --panel: #ffffff; --panel-2: #f5f6fa; --panel-3: #e9ecf3;
  --line: #dde1ea; --line-2: #cfd4e0;
  --text: #161925; --text-2: #4f5670; --text-3: #6b7290;
  --accent: #4f63e6; --accent-2: #3a4bc4; --accent-soft: rgba(79, 99, 230, 0.12);
  --mask: #0d9488; --on-accent: #ffffff;
  --live: #e11d48; --live-soft: rgba(225, 29, 72, 0.12); --live-btn: #e11d48;
  --pvw: #15803d; --warn: #b45309; --key: #a16207;      /* vert et orange foncés pour le texte sur blanc */
  --pvw-fill: #22c55e; --warn-fill: #f5a524;             /* aplats à encre sombre : teintes vives */
  /* texte de couleur sur fond clair (le sombre utilise des teintes pâles, ex. #ff8aa0 pour le rouge) */
  --t-red: #c8183f; --t-green: #15803d; --t-blue: #0369a1; --t-yellow: #a16207; --t-pink: #be185d; --t-violet: #6d28d9; --t-orange: #c2410c;
  color-scheme: light;
}
/* Les images (aperçus, vignettes, consoles de journal) restent sombres dans les deux thèmes : remettre les jetons sombres sur leur conteneur,
   par ex. :root[data-theme='light'] :is(.log-box, .media-preview) { --bg: #0b0c12; --panel: #12141d; … color-scheme: dark; } */
```

### 3.2 Base

```css
* { box-sizing: border-box; }
input, textarea, [contenteditable]:not([contenteditable='false']), pre, code, .log-box { user-select: text; -webkit-user-select: text; }
html, body, #root { height: 100%; margin: 0; }
body { user-select: none; -webkit-user-select: none; /* une application, pas un document */
  background: var(--bg); color: var(--text); font: 13px/1.4 Inter, system-ui, sans-serif; -webkit-font-smoothing: antialiased; overflow: hidden; }
button, input, select, textarea { font: inherit; color: inherit; }
input, select, textarea { background: var(--panel-3); border: 1px solid var(--line); border-radius: 6px; padding: 5px 8px; min-width: 0; outline: none; transition: border-color 0.12s; }
textarea { width: 100%; resize: vertical; }
input[type='range'] { accent-color: var(--accent); }
hr { border: none; border-top: 1px solid var(--line); margin: 6px 0; width: 100%; }
kbd { font-family: var(--mono); font-size: 10px; background: var(--panel-3); border: 1px solid var(--line-2); border-bottom-width: 2px; border-radius: 4px; padding: 0 4px; }
code { font-family: var(--mono); font-size: 11px; color: var(--text-3); }
::-webkit-scrollbar { width: 10px; height: 10px; }
::-webkit-scrollbar-thumb { background: var(--line-2); border-radius: 10px; border: 2px solid transparent; background-clip: padding-box; }
::-webkit-scrollbar-corner { background: transparent; }
/* Pas de cadre de focus (les champs gardent leur bordure) ; au clavier seulement, sur ce qui n'est pas un champ : */
:focus, :focus-visible { outline: none; }
:is(button, a, [role='button'], [role='tab'], [tabindex]:not([tabindex='-1'])):focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
/* Hauteur des champs dans les pages et les modales */
:is(.projects-page, .settings, .modal-back) :is(input:not([type='checkbox']):not([type='radio']):not([type='color']):not([type='range']), select) { min-height: var(--h-control); }
:is(.projects-page, .settings, .modal-back) .search input { min-height: 0; }

.spacer { flex: 1; }
.muted { color: var(--text-3); font-size: 12px; }
.hint { color: var(--text-3); font-size: 11.5px; margin: 2px 0; }
.hint svg { vertical-align: -2px; }
.small { font-size: 11.5px; }
.empty, .loading { color: var(--text-3); padding: 24px 16px; text-align: center; }
.dim { opacity: 0.45; }
.row { display: flex; gap: 6px; align-items: center; }
.row > input, .row > select { flex: 1; }
.spin { animation: spin 1s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@keyframes blink { 50% { opacity: 0.3; } }
```

### 3.3 Mesures à respecter

- Texte : corps 13 px / 1,4 ; titre de page `h1` 22 px graisse 800 ; titre de modale 16 px ; titre de carte 14 px / 700 ; titres de section (liste de gauche, groupes de liste) 11 px / 700 / MAJUSCULES / `letter-spacing: 0.08em` / `--text-3` ; libellés de champ 11 px / 600 / MAJUSCULES / `0.04em` / `--text-2` ; secondaire `.muted` 12 px ; aide 11,5 px ; nombres en `font-variant-numeric: tabular-nums`, valeurs techniques en `var(--mono)`.
- Icônes lucide : 13 dans les petits boutons, 14 dans les boutons et onglets, 15 dans la liste de gauche et le bouton de connexion, 16 dans les en-têtes de carte et les modales, 17 pour l'aide, 22 dans les états vides et cartes « + ».
- Rayons : 4 (segments, `kbd`), 6 (boutons, champs, boutons icône, items de menu), 7 (infobulle), 8 (items de liste de gauche, recherche, toasts, encadrés), 9 (conteneur d'onglets, liste déroulante), 10 (menus, badges), 12 (cartes de réglages, barre d'enregistrement), 14 (cartes de grille, modales), 16 (carte de connexion), 999 px (puces).
- Ombres : menus `0 16px 40px rgba(0,0,0,.5)` ; liste déroulante `0 14px 40px rgba(0,0,0,.5)` ; modale `0 30px 80px rgba(0,0,0,.5)` ; carte de connexion `0 30px 80px rgba(0,0,0,.45)` ; toast `0 8px 30px rgba(0,0,0,.4)` ; infobulle `0 8px 24px rgba(0,0,0,.45)` ; carte survolée `0 14px 34px rgba(0,0,0,.35)` + `translateY(-2px)` ; barre d'enregistrement `0 12px 40px rgba(0,0,0,.5)`. Thème clair : tout ce qui flotte `0 12px 32px rgba(22,25,37,.16)`, cartes survolées `0 6px 18px rgba(22,25,37,.1)`.
- Transitions : 0,12 s (survol, bordure), 0,15 s (cartes, interrupteurs), apparition des listes 0,12 s ; `@media (prefers-reduced-motion: reduce)` les coupe.
- Fonds de page : contenu `radial-gradient(ellipse at top, #141725 0%, #0b0c12 60%)` ; barre du haut `linear-gradient(180deg, #141623, #10121a)` ; connexion `radial-gradient(ellipse at 30% 0%, #1d1840 0%, #0b0c12 60%)`.

---

## 4. Coquille de l'app et composants

Les noms de classes ci-dessous sont ceux de la référence : garde-les tels quels (même CSS, mêmes maquettes d'une app à l'autre). Les textes des exemples passent tous par `t()`.

### 4.1 Barre du haut

```tsx
<nav className="topbar">
  <Brand />
  <div className="tabs home-tabs">
    <button className={on ? 'on' : ''} aria-current={on ? 'page' : undefined} onClick={() => navigate('/…')}>
      <FolderOpen size={14} /> <span className="tab-label">{t('…')}</span>
    </button>
    {/* groupe de pages : NavGroup (§ 4.3) */}
    <button className={route.page === 'system' ? 'on' : ''} onClick={() => navigate('/system')}>
      <Server size={14} /> <span className="tab-label">{t('System')}</span>
      {admin && needsLook(health) && <span className="nav-dot warn" />}
    </button>
  </div>
  <div className="spacer" />
  <StatusPill />
  <HelpButton on={route.page === 'help'} />
  <UserMenu named />
</nav>
<div className="content">{/* page */}</div>
```

```css
.app { display: flex; flex-direction: column; height: 100%; }
.topbar { height: 46px; flex: none; display: flex; align-items: center; gap: 18px; padding: 0 14px; border-bottom: 1px solid var(--line); background: linear-gradient(180deg, #141623, #10121a); }
.topbar .tabs button { white-space: nowrap; }
.topbar-sep { flex: none; width: 1px; height: 20px; background: var(--line-2); }
.content { flex: 1; min-height: 0; }
@media (max-width: 1500px) { .topbar { gap: 14px; } .topbar .tabs button { padding: 5px 10px; } }
@media (max-width: 1360px) { .topbar { gap: 10px; } .topbar .tabs button { padding: 5px 8px; } }
:root[data-theme='light'] .topbar { background: var(--panel); }
```

### 4.2 Marque

`<Brand />` (code complet en annexe A) : à gauche le « F » (SVG 23,7 × 24, blanc) puis le nom de marque en 800 et le nom de module en 400 couleur accent ; `stacked` (page de connexion) : centré, 40 px, avec la signature « F + AudioVisuel » dessous. Remplacer `MODULE` par le nom du module de cette app.

```css
.brand { display: flex; flex-direction: column; align-items: flex-start; gap: 3px; flex: none; }
.brand-lockup { display: flex; align-items: baseline; gap: 6px; }
.brand-lockup:has(.brand-f-lead) { align-items: flex-end; }
.brand .brand-f-lead { color: #fff; margin-right: 2px; margin-bottom: -1px; }
.brand-name, .brand-module { font-weight: 800; font-size: 16px; letter-spacing: -0.035em; line-height: 1; color: #fff; }
.brand-module { font-weight: 400; letter-spacing: -0.02em; color: var(--accent); }
.brand-sign { display: flex; align-items: flex-end; gap: 3px; color: var(--text-2); }
.brand:not(.stacked) .brand-sign { opacity: 0.6; }
.brand svg { display: block; flex: none; overflow: visible; }
.brand.stacked { align-items: center; gap: 12px; }
.brand.stacked .brand-lockup { gap: 10px; }
.brand.stacked .brand-name, .brand.stacked .brand-module { font-size: 40px; }
.brand.stacked .brand-sign { gap: 6px; }
:root[data-theme='light'] :is(.brand .brand-f-lead, .brand-name) { color: var(--text); }
```

### 4.3 Navigation principale (groupe segmenté) et menus

Les pages principales sont des boutons dans `.tabs` ; un ensemble de pages s'ouvre par `NavGroup` (`ui/NavMenu.tsx`) : bouton `icône + libellé + ChevronDown(13, .ux1-chev)`, `on` quand une de ses pages est ouverte, menu `.menu.ux1-navmenu` d'items `.ux1-mi` (`role="menuitem"`, `aria-current="page"`). Clavier : ↓/Entrée ouvre, ↑ ↓ Début Fin déplacent, Échap ferme, Tab ferme ; un clic ailleurs (`pointerdown`) ferme.

```css
.tabs { display: flex; gap: 4px; background: var(--panel); border: 1px solid var(--line); padding: 3px; border-radius: 9px; }
.tabs button { display: flex; align-items: center; gap: 6px; border: none; background: none; padding: 5px 12px; border-radius: 6px; cursor: pointer; color: var(--text-2); font-weight: 500; }
.tabs button.on { background: var(--panel-3); color: var(--text); box-shadow: 0 1px 0 rgba(255, 255, 255, 0.04) inset; }
:root[data-theme='light'] .tabs button.on { box-shadow: 0 1px 2px rgba(22, 25, 37, 0.08); }
.menu { position: absolute; z-index: 50; top: calc(100% + 6px); left: 0; background: var(--panel-2); border: 1px solid var(--line-2); border-radius: 10px; box-shadow: 0 16px 40px rgba(0, 0, 0, 0.5); padding: 6px; }
.menu.right { left: auto; right: 0; }
.menu-sep { height: 1px; background: var(--line); margin: 4px 2px; }
.ux1-navgroup { position: relative; display: flex; }
.ux1-navgroup > button.open { background: var(--panel-2); color: var(--text); }
.ux1-navgroup .ux1-chev { margin-left: -2px; opacity: 0.7; }
.ux1-navmenu { top: calc(100% + 9px); min-width: 210px; z-index: 300; }
/* le menu est dans .tabs : le second sélecteur l'emporte sur `.tabs button` */
.ux1-mi, .tabs .ux1-navmenu .ux1-mi { display: flex; align-items: center; gap: 10px; width: 100%; height: 32px; padding: 0 8px; border: none; border-radius: 6px; background: none; color: var(--text); font-size: 13px; font-weight: 500; text-align: left; white-space: nowrap; cursor: pointer; }
.ux1-mi:hover, .ux1-mi:focus-visible, .tabs .ux1-navmenu .ux1-mi:hover, .tabs .ux1-navmenu .ux1-mi:focus-visible, .tabs .ux1-navmenu .ux1-mi.on { background: var(--accent-soft); outline: none; }
.ux1-mi svg { flex: none; color: var(--text-2); }
.ux1-mi .ux1-cap { margin-left: auto; }
.ux1-mi.danger, .ux1-mi.danger svg { color: #ff8aa0; }
:root[data-theme='light'] :is(.ux1-mi.danger, .ux1-mi.danger svg) { color: var(--t-red); }
.ux1-navgroup .nav-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--live); box-shadow: 0 0 0 2px var(--bg); margin: -8px 0 0 -4px; }
```

Menu contextuel (`ui/ContextMenu.tsx`, « … » ou clic droit) : même `.menu` en `position: fixed` dans un portail, placé sous le pointeur ou au-dessus s'il ne tient pas, toujours dans la fenêtre (marge 8 px) ; items `<button>` icône + libellé, `.danger`, séparateurs `.menu-sep` ; Échap, clic ailleurs ou redimensionnement ferment.

### 4.4 Pastille de connexion (`ui/StatusPill.tsx`)

Une pastille : « ● Connecté » (vert) ; « Connexion… » (gris, point qui clignote) ; `TriangleAlert` « CPU 92 % » (orange) quand la machine dépasse 85 % (retour sous 80 %) ; « Serveur perdu · reconnexion… » (rouge, icône `WifiOff`) quand le WebSocket est fermé depuis 800 ms (4 s au premier chargement). Pendant la perte, `GET /api/auth/state` toutes les 3 s : si la session n'existe plus (serveur redémarré), retour à la page de connexion. Infobulle : nom du serveur, état, `CPU x % · n cœurs`, `Mémoire a sur b`, nom de la machine. Pour un admin c'est un bouton qui ouvre `/system/performance`. Données machine : `GET /api/system/metrics` toutes les 3 s (`{machine: {cpu, cores, cpuName, memUsed, memTotal, hostname}}`, via `sysinfo`).

```css
.ux1-pill { display: inline-flex; align-items: center; gap: 6px; flex: none; height: 28px; padding: 0 10px; border-radius: 14px; border: 1px solid transparent; background: none; color: var(--pvw); font-size: 12px; font-weight: 600; white-space: nowrap; }
.ux1-pill .ux1-dot { width: 8px; height: 8px; border-radius: 50%; background: currentColor; box-shadow: 0 0 8px currentColor; }
.ux1-pill.wait { color: var(--text-3); }
.ux1-pill.wait .ux1-dot { box-shadow: none; animation: blink 1s infinite; }
.ux1-pill.hot { color: var(--warn); background: rgba(245, 165, 36, 0.14); border-color: rgba(245, 165, 36, 0.5); font-weight: 700; }
.ux1-pill.lost { color: #fff; background: rgba(225, 29, 72, 0.22); border-color: var(--live); font-weight: 700; }
.ux1-pill.lost svg { color: var(--live); }
:root[data-theme='light'] .ux1-pill.lost { color: var(--t-red); background: rgba(225, 29, 72, 0.1); }
@media (max-width: 1500px) { .ux1-lost-txt { display: flex; flex-direction: column; line-height: 1.1; font-size: 11px; } .ux1-lost-sep { display: none; } }
```

### 4.5 Aide

Bouton rond `CircleHelp` 17 px à droite de la pastille, `title={t('Help')}`, qui ouvre la page d'aide de l'app (`on` quand elle est ouverte).

```css
.learn-help { width: 30px; height: 30px; display: grid; place-items: center; border: 0; border-radius: 8px; background: none; color: var(--text-2); cursor: pointer; flex: none; }
.learn-help:hover, .learn-help.on { background: var(--panel-3); color: var(--text); }
```

### 4.6 Menu utilisateur (`ui/UserMenu.tsx`)

Bouton : avatar (initiale du nom affiché) + nom + `ChevronDown 13` (`named` sur les pages principales). Menu `.menu.ux1-umenu` : en-tête `.ux1-who` (avatar, nom en gras, rôle) · séparateur · ligne « Langue » avec `EN | FR` · ligne « Thème » avec `Sombre | Clair | Système` · item « Raccourcis clavier » avec la touche `?` · séparateur · item danger « Se déconnecter » (`LogOut`). Clavier comme NavGroup. Changer la langue ou le thème : appliqué tout de suite, puis `PUT /api/me/lang` / `PUT /api/me/theme` (toast d'erreur si refus).

```tsx
<div className="ux1-mrow"><Languages size={14} /> {t('Language')}
  <div className="lang-switch" role="group" aria-label={t('Language')}>
    {LANGS.map((l) => <button key={l.id} className={lang === l.id ? 'on' : ''} aria-pressed={lang === l.id} title={l.name} onClick={() => pick(l.id)}>{l.label}</button>)}
  </div>
</div>
<div className="ux1-mrow"><SunMoon size={14} /> {t('Theme')}
  <div className="lang-switch theme-switch" role="group" aria-label={t('Theme')}>…{tc('theme', 'Dark')} {tc('theme', 'Light')} {tc('theme', 'System')}</div>
</div>
```

```css
.avatar { width: 24px; height: 24px; border-radius: 50%; display: grid; place-items: center; font-size: 11px; font-weight: 700; background: linear-gradient(135deg, var(--accent), var(--accent-2)); color: var(--on-accent); }
.user-name { font-size: 12px; color: var(--text-2); max-width: 140px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ux1-user { position: relative; flex: none; }
.ux1-user-btn { display: inline-flex; align-items: center; gap: 7px; height: 32px; padding: 0 2px; border: none; border-radius: 16px; background: none; color: var(--text-2); cursor: pointer; }
.ux1-user-btn .avatar { width: 28px; height: 28px; font-size: 12px; }
.ux1-user-btn.named { padding: 0 9px 0 2px; }
.ux1-user-btn.named:hover, .ux1-user-btn.named.on { background: var(--panel-2); }
.ux1-user-btn.on .avatar { box-shadow: 0 0 0 2px #12141f, 0 0 0 4px var(--accent); }
:root[data-theme='light'] .ux1-user-btn.on .avatar { box-shadow: 0 0 0 2px var(--panel), 0 0 0 4px var(--accent); }
.ux1-umenu { left: auto; right: 0; top: calc(100% + 8px); z-index: 300; min-width: 250px; }
.ux1-who { display: flex; align-items: center; gap: 10px; padding: 6px 8px 8px; }
.ux1-who b { display: block; font-size: 13px; color: var(--text); }
.ux1-who small { font-size: 12px; color: var(--text-3); }
.ux1-mrow { display: flex; align-items: center; gap: 10px; height: 36px; padding: 0 8px; font-size: 13px; color: var(--text); }
.ux1-mrow svg { color: var(--text-2); }
.ux1-mrow .lang-switch { margin-left: auto; }
.lang-switch { display: flex; padding: 2px; gap: 1px; border-radius: 7px; background: var(--panel-2); border: 1px solid var(--line); }
.lang-switch button { border: none; background: none; color: var(--text-3); font-size: var(--fs-11); font-weight: 700; letter-spacing: 0.04em; padding: 3px 7px; border-radius: 5px; cursor: pointer; }
.lang-switch button:hover { color: var(--text); }
.lang-switch button.on { background: var(--accent-soft); color: var(--accent-2); }
.ux1-cap { font: 600 11px/15px var(--mono); letter-spacing: 0; text-transform: none; padding: 1px 5px; border-radius: 4px; border: 1px solid rgba(255, 255, 255, 0.28); border-bottom-width: 2px; background: rgba(0, 0, 0, 0.22); color: inherit; opacity: 0.9; }
:root[data-theme='light'] .ux1-cap { border-color: color-mix(in srgb, currentColor 30%, transparent); background: color-mix(in srgb, currentColor 8%, transparent); }
```

### 4.7 Mise en page des pages : liste à gauche, page à droite (`pages/SidePage.tsx`)

```tsx
<div className="projects-page">
  <aside className="lib-nav">
    <div className="lib-nav-section"><span className="grow">{t('…')}</span>{/* actions */}</div>
    <div className={`lib-nav-item ${on ? 'on' : ''}`} onClick={…}>
      <Icon size={15} /><span className="grow">{label}</span>
      <span className="lib-nav-actions">{/* icon-btn 22 px, visibles au survol */}</span>
      {status && <span className={`nav-dot ${status}`} />}
      {count !== undefined && <span className="count">{count}</span>}
    </div>
  </aside>
  <main className="library">
    <div className="library-inner wide">
      <div className="settings-head"><div><h1>{title}</h1>{sub && <p className="muted">{sub}</p>}</div><div className="spacer" />{/* action principale : <button className="btn primary"><Plus size={14} /> …</button> */}</div>
      {/* sous-pages : SubTabs */}
      <div className="sys-tabrow"><div className="tabs sys-tabs" role="tablist">{items.map((i) => <button role="tab" aria-selected={…} className={…}>{i.icon}<span>{i.label}</span>{i.count !== undefined && <span className="sys-tab-n">{i.count}</span>}</button>)}</div>{/* actions de la sous-page */}</div>
      {/* contenu */}
    </div>
  </main>
</div>
```

```css
.projects-page { height: 100%; display: grid; grid-template-columns: 240px minmax(0, 1fr); }
.projects-page > .library { min-width: 0; }
.lib-nav { border-right: 1px solid var(--line); background: var(--panel); padding: 18px 10px; overflow: auto; display: flex; flex-direction: column; gap: 2px; }
.lib-nav-section { display: flex; align-items: center; font-size: var(--fs-11); font-weight: 700; letter-spacing: 0.08em; text-transform: uppercase; color: var(--text-3); padding: 14px 10px 6px; min-height: 38px; }
.lib-nav-section:first-child { padding-top: 0; min-height: 0; }
.sys-group:not(:first-child) > .lib-nav-section:first-child { padding-top: 14px; min-height: 38px; }
.lib-nav-item { display: flex; align-items: center; gap: 9px; padding: 7px 10px; border-radius: 8px; color: var(--text-2); cursor: pointer; border: 1px dashed transparent; transition: background 0.12s, color 0.12s; }
.lib-nav-item:hover { background: var(--panel-2); color: var(--text); }
.lib-nav-item.on { background: var(--accent-soft); color: var(--text); }
.lib-nav-item.on svg { color: var(--accent-2); }
.lib-nav-item.drop { border-color: var(--accent); background: var(--accent-soft); }
.lib-nav-item .grow { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.lib-nav-actions { display: none; gap: 0; }
.lib-nav-actions .icon-btn { width: 22px; height: 22px; }
.lib-nav-item:hover .lib-nav-actions { display: flex; }
.count { font-size: 11px; color: var(--text-3); font-variant-numeric: tabular-nums; }
.library, .settings { height: 100%; overflow: auto; background: radial-gradient(ellipse at top, #141725 0%, #0b0c12 60%); }
:root[data-theme='light'] :is(.library, .settings) { background: var(--bg); }
.library-inner { max-width: 1180px; margin: 0 auto; padding: 28px 24px 80px; display: flex; flex-direction: column; gap: 20px; }
.library-inner.wide { max-width: 1400px; }
.settings-head { display: flex; align-items: center; gap: 12px; }
.settings-head h1 { margin: 0; font-size: 22px; font-weight: 800; }
.settings-head p { margin: 4px 0 0; }
.sys-tabrow { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; margin: -6px 0 18px; }
.sys-tabs { width: fit-content; max-width: 100%; flex-wrap: wrap; }
.sys-tab-n { font-size: var(--fs-11); color: var(--text-3); font-variant-numeric: tabular-nums; }
```

### 4.8 Cartes de réglages, champs, grilles (`settings/ui.tsx`)

```tsx
export function Card({ icon, title, children, actions }) {
  return (<section className="set-card"><header>{icon /* size 16 */}<h2>{title}</h2><div className="spacer" />{actions}</header>{children ? <div className="set-body">{children}</div> : null}</section>);
}
export function Field({ label, children }) {
  return (<label className="set-field"><span className="set-label">{label}</span>{children}</label>);
}
// <div className="set-grid">…Field…</div>  (3 colonnes ; .set-grid.two : 2 colonnes)
```

```css
.set-card { background: var(--panel); border: 1px solid var(--line); border-radius: 12px; }
.set-card > header { display: flex; align-items: center; gap: 10px; padding: 12px 16px; border-bottom: 1px solid var(--line); color: var(--accent-2); }
.set-card h2 { margin: 0; font-size: 14px; font-weight: 700; color: var(--text); }
.set-body { padding: 16px; display: flex; flex-direction: column; gap: 14px; }
.set-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; }
.set-grid.two { grid-template-columns: 1fr 1fr; }
@media (max-width: 760px) { .set-grid { grid-template-columns: 1fr; } }
@media (max-width: 640px) { .set-grid.two { grid-template-columns: 1fr; } }
.set-field { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
.set-label { font-size: 11px; font-weight: 600; letter-spacing: 0.04em; text-transform: uppercase; color: var(--text-2); }
.set-field input, .set-field select { padding: 7px 10px; font-size: 13px; }
.set-field input[readonly] { color: var(--text-2); }
.set-field input[aria-invalid='true'] { border-color: var(--live); }
.set-hint { font-size: 11.5px; color: var(--text-3); }
.set-error { font-size: var(--fs-12); color: var(--live); }
.sys-list { display: grid; grid-template-columns: 140px 1fr; gap: 8px 16px; margin: 0; }
.sys-list dt { color: var(--text-3); }
.sys-list dd { margin: 0; overflow-wrap: anywhere; }
.sys-list a { color: var(--accent-2); }
```

### 4.9 Grilles de cartes

Grille `repeat(auto-fill, minmax(290px, 1fr))`, gap 18 (16 pour les utilisateurs). Carte : fond `--panel`, bordure `--line`, rayon 14, survol bordure `--line-2` + `translateY(-2px)` + ombre. Dernière case : la carte « + » en pointillés.

```css
.project-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(290px, 1fr)); gap: 18px; }
.project-card { position: relative; background: var(--panel); border: 1px solid var(--line); border-radius: 14px; display: flex; flex-direction: column; overflow: visible; transition: border-color 0.15s, transform 0.15s, box-shadow 0.15s; }
.project-card:hover { border-color: var(--line-2); transform: translateY(-2px); box-shadow: 0 14px 34px rgba(0, 0, 0, 0.35); }
:root[data-theme='light'] :is(.project-card:hover, .user-card:hover) { box-shadow: 0 6px 18px rgba(22, 25, 37, 0.1); }
.project-body { padding: 12px 14px 4px; display: flex; flex-direction: column; gap: 6px; }
.project-desc { margin: 0; font-size: 12.5px; color: var(--text-2); display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; min-height: 2.8em; }
.project-meta { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 11.5px; color: var(--text-3); }
.project-meta > span { display: inline-flex; align-items: center; gap: 5px; }
.project-new { border: 1.5px dashed var(--line-2); border-radius: 14px; background: none; color: var(--text-3); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; min-height: 260px; cursor: pointer; font: inherit; transition: border-color 0.15s, color 0.15s, background 0.15s; }
.project-new:hover { border-color: var(--accent); color: var(--text); background: var(--accent-soft); }
```

### 4.10 États vides et chargement

- Page sans élément : `<div className="empty big"><Icon size={22} /><p>{q ? t('No match') : t('No …')}</p></div>` — un mot, pas d'explication.
- Zone vide dans un panneau : `<Empty icon={Icon} text={t('…')}>{/* bouton éventuel */}</Empty>`.
- Chargement : `<div className="loading">{t('Loading…')}</div>` (« Chargement… », « Connexion… ») ; vignette en chargement : `.ux3-skel` (reflet animé).

```css
.empty.big { grid-column: 1 / -1; display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 60px 16px; }
.opx-void { flex: 1; grid-column: 1 / -1; min-height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; padding: 16px; color: var(--text-3); text-align: center; }
.opx-void-ic { width: 40px; height: 40px; border-radius: 10px; background: var(--op-2, #1c1f2c); display: grid; place-items: center; color: var(--text-2); } /* --op-2 : #1c1f2c sombre, #f3f4f8 clair */
.opx-void-t { color: var(--text-2); }
@keyframes ux3-shimmer { from { background-position: 120% 0, 0 0; } to { background-position: -20% 0, 0 0; } }
.ux3-skel { position: absolute; inset: 0; z-index: 0; border-radius: inherit; pointer-events: none;
  background: linear-gradient(100deg, transparent 20%, rgba(143, 163, 255, 0.09) 40%, rgba(143, 163, 255, 0.16) 50%, rgba(143, 163, 255, 0.09) 60%, transparent 80%), linear-gradient(var(--panel-2), var(--panel-2));
  background-size: 200% 100%, 100% 100%; animation: ux3-shimmer 1.4s linear infinite; }
.ux3-skel::before { content: ''; position: absolute; left: 50%; top: 50%; width: 46%; height: 18%; transform: translate(-50%, -50%); border-radius: 8px; background: var(--panel-3); opacity: 0.8; }
```

### 4.11 Panneaux latéraux, sections repliables, lignes libellé / contrôle, poignées

- Panneau : `.panel.left` / `.panel.right` (fond `--panel`, bordure côté contenu). Section fixe : `<section className="insp-section"><header><span>{titre}</span>{actions}</header><div className="insp-body">…</div></section>`.
- Section repliable : `<section className={`pp-group l1 ${open ? 'open' : ''}`}><header><button className="pp-twirl" aria-expanded={open}><ChevronDown size={14} /><span>{titre}</span></button>{actions}</header>{open && <div className="pp-body">…</div>}</section>` ; niveau 2 : `l2`, chevron 12. L'état ouvert est gardé pour l'utilisateur : `useDisplayPref('group.<id>', defaultOpen)` (§ 5.9).
- Ligne libellé à gauche, contrôle à droite : `<div className="prop-row"><span className="prop-label">{t('…')}</span>{contrôle}</div>` dans un panneau ; dans un menu : `<div className="ux2-dline"><span>{t('…')}</span>{contrôle}</div>`.
- Poignée de redimensionnement (`ui/Splitter.tsx`) : 7 px, trait d'accent de 1 px au survol, double-clic = taille par défaut, tailles gardées par `useLayout(key, defaults, user)`.

```css
.panel { background: var(--panel); display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
.panel.left { border-right: 1px solid var(--line); }
.panel.right { border-left: 1px solid var(--line); }
.insp-section { border-bottom: 1px solid var(--line); }
.insp-section header { display: flex; align-items: center; justify-content: space-between; font-size: var(--fs-11); font-weight: 700; letter-spacing: 0.08em; text-transform: uppercase; color: var(--text-3); padding: 10px 12px 6px; }
.insp-body { padding: 2px 12px 12px; display: flex; flex-direction: column; gap: 6px; }
.pp-group > header { display: flex; align-items: center; }
.pp-twirl { display: flex; align-items: center; gap: 6px; flex: 1; border: none; background: none; color: var(--text); font: inherit; cursor: pointer; text-align: left; }
.pp-twirl svg { color: var(--text-3); transition: transform 0.12s; transform: rotate(-90deg); }
.pp-group.open > header .pp-twirl svg { transform: none; }
.pp-group > header .pp-twirl:hover { color: var(--text); }
.pp-group.l1 { border-bottom: 1px solid var(--line); }
.pp-group.l1 > header .pp-twirl { padding: 12px 14px 10px; font-size: 13.5px; font-weight: 600; letter-spacing: 0.01em; color: var(--text); }
.pp-group.l2 > header .pp-twirl { padding: 7px 12px 7px 22px; font-size: 12.5px; font-weight: 500; color: var(--text-2); }
.pp-group.l2 > .pp-body { padding: 0 10px 8px 14px; display: flex; flex-direction: column; gap: 3px; }
.prop-row { display: flex; align-items: center; gap: 4px; min-height: 28px; }
.prop-label { color: var(--text-2); font-size: 12px; min-width: 56px; }
.ux2-dline { display: flex; align-items: center; gap: 8px; padding: 4px 8px; min-height: 34px; }
.ux2-dline > span:first-child { flex: 1; min-width: 0; white-space: nowrap; }
.ux2-dline .segmented button { flex: 1; padding: 3px 6px; }
.splitter { z-index: 20; flex: none; user-select: none; touch-action: none; }
.splitter.x { position: absolute; top: 0; bottom: 0; width: 7px; cursor: col-resize; }
.splitter.y { height: 7px; cursor: row-resize; }
.splitter.edge-right { right: 0; } .splitter.edge-left { left: 0; }
.splitter::after { content: ''; position: absolute; opacity: 0; background: var(--accent); box-shadow: 0 0 5px 1px rgba(143, 163, 255, 0.55); pointer-events: none; transition: opacity 0.12s; }
.splitter.x::after { top: 0; bottom: 0; left: 3px; width: 1px; }
.splitter.x.edge-right::after { left: auto; right: 0; }
.splitter.x.edge-left::after { left: 0; }
.splitter.y::after { left: 0; right: 0; top: 3px; height: 1px; }
.splitter:hover::after, body.resizing-x .splitter.x:active::after, body.resizing-y .splitter.y:active::after { opacity: 1; }
body.resizing-x, body.resizing-x * { cursor: col-resize !important; user-select: none; }
body.resizing-y, body.resizing-y * { cursor: row-resize !important; user-select: none; }
```

### 4.12 Boutons

`btn` (neutre), `btn primary` (une seule action principale par vue), `btn ghost`, `btn small`, `btn danger` (texte rouge), `btn danger-solid` (confirmation destructive), `btn block` (pleine largeur), `btn on` (activé) ; `icon-btn` (28 × 28, `title` obligatoire), `icon-btn ghost`. Contenu : `<Icon size={14} /> {t('Libellé')}` (13 en `small`).

```css
.btn { display: inline-flex; align-items: center; gap: 6px; min-height: var(--h-control); background: var(--panel-3); border: 1px solid var(--line); border-radius: 6px; padding: 6px 12px; cursor: pointer; transition: background 0.12s, border-color 0.12s; white-space: nowrap; }
.btn:hover:not(:disabled) { background: var(--line); }
.btn:disabled { opacity: 0.4; cursor: default; }
.btn.small { min-height: var(--h-control-dense); padding: 4px 9px; font-size: 12px; }
.btn.on { background: var(--accent-soft); border-color: var(--accent); color: var(--accent-2); }
.btn.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
.btn.primary:hover:not(:disabled) { background: var(--accent-2); }
.btn.ghost { background: none; border-color: transparent; color: var(--text-2); }
.btn.ghost:hover:not(:disabled) { background: var(--panel-2); color: var(--text); }
.btn.danger { color: var(--live); }
.btn.danger-solid { background: var(--live-btn); border-color: var(--live); color: white; }
.btn.danger-solid:hover { filter: brightness(1.1); }
.btn.warn { background: var(--warn); border-color: var(--warn); color: #1a1205; font-weight: 700; }
:root[data-theme='light'] .btn.warn { background: var(--warn-fill); border-color: var(--warn-fill); }
.btn.block { justify-content: center; padding: 9px 12px; }
a.btn { text-decoration: none; }
.icon-btn { display: inline-grid; place-items: center; width: var(--hit-min); height: var(--hit-min); border-radius: 6px; border: 1px solid var(--line); background: var(--panel-3); cursor: pointer; color: var(--text-2); transition: all 0.12s; flex: none; }
.icon-btn:hover:not(:disabled) { color: var(--text); background: var(--line); }
.icon-btn:disabled { opacity: 0.3; cursor: default; }
.icon-btn.ghost { background: transparent; border-color: transparent; }
.icon-btn.ghost:hover:not(:disabled) { background: var(--panel-3); }
.icon-btn.on { color: var(--text); }
.icon-btn.accent { color: var(--accent-2); }
```

### 4.13 Contrôle segmenté (`Segmented`, `ui/fields.tsx`)

Remplace les boutons radio et les petites listes (2 à 5 choix courts).

```tsx
export function Segmented<T extends string>({ value, options, onChange, className }: { value: T; options: { value: T; label: React.ReactNode; title?: string; disabled?: boolean }[]; onChange: (v: T) => void; className?: string }) {
  return (<div className={className ? `segmented ${className}` : 'segmented'}>{options.map((o) => (
    <button key={o.value} type="button" title={o.title} disabled={o.disabled} className={o.value === value ? 'on' : ''} onClick={() => onChange(o.value)}>{o.label}</button>))}</div>);
}
```

```css
.segmented { display: inline-flex; background: var(--panel-3); border: 1px solid var(--line); border-radius: 6px; padding: 2px; }
.segmented button { border: none; background: none; padding: 3px 8px; border-radius: 4px; cursor: pointer; color: var(--text-2); display: grid; place-items: center; font-size: 11.5px; }
.segmented button.on { background: var(--line-2); color: var(--text); }
.segmented button:disabled { opacity: 0.4; cursor: default; }
```

### 4.14 Liste déroulante dessinée (`Dropdown`, `ui/Dropdown.tsx`)

API : `<Dropdown value options onChange title? placeholder="—" className? />`, `DropOption = { value, label, group?, note?, style?, sub?, icon?, detail?, disabled? }`. Comportement :
- Bouton `.dd-btn` : icône de l'option courante, libellé (ellipse), `detail` en mono, `ChevronDown 12` qui tourne de 180° ouvert. `onPointerDown` stoppé (pas de glisser parasite).
- Liste dans un **portail** sur `document.body`, `position: fixed`, sous le bouton (`top = bas + 4`), ou **au-dessus** si moins de 240 px dessous et plus de place dessus ; largeur min = max(160, largeur du bouton) (260 en mode riche) ; décalée à gauche si elle dépasse la fenêtre (marge 8).
- Mode « riche » si une option a `icon` ou `sub` : deux lignes (`dd-main` + `dd-sub`), coche à droite ; sinon coche à gauche. `group` : titre de groupe quand il change. `disabled` : affiché, non sélectionnable (sa `sub` dit pourquoi, en orange).
- Clavier : ↓ / Entrée / Espace ouvre ; ↑ ↓ sautent les options désactivées ; Entrée choisit ; Échap ferme et rend le focus. Ferme sur `pointerdown` ailleurs (capture), molette hors de la liste, redimensionnement. L'option survolée défile en vue. Liste vide : « Rien à choisir ».

```css
.dd-btn { display: inline-flex; align-items: center; gap: 6px; width: 100%; min-width: 0; height: 26px; padding: 0 8px 0 9px; border: 1px solid var(--line-2); border-radius: 6px; background: var(--panel-2); color: var(--text); font: inherit; font-size: 12px; text-align: left; cursor: pointer; }
.dd-btn:hover { border-color: var(--text-3); background: var(--panel-3); }
.dd-btn > svg { flex: none; color: var(--text-3); transition: transform 0.15s; }
.dd-btn.open > svg { transform: rotate(180deg); }
.dd-label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dd-detail { margin-left: 8px; font: 11px var(--mono); color: var(--text-3); }
.dd-icon { flex: none; display: grid; place-items: center; width: 12px; }
@keyframes dd-in { from { opacity: 0; transform: translateY(-4px); } }
@keyframes dd-in-up { from { opacity: 0; transform: translateY(4px); } }
.dd-list { position: fixed; z-index: 2000; max-width: 320px; max-height: 300px; overflow-y: auto; padding: 4px; border: 1px solid var(--line-2); border-radius: 9px; background: var(--panel); box-shadow: 0 14px 40px rgba(0, 0, 0, 0.5); font-size: 12.5px; animation: dd-in 0.12s ease-out; }
.dd-list.up { animation-name: dd-in-up; }
.dd-group { padding: 8px 8px 4px; font-size: var(--fs-11); font-weight: 700; letter-spacing: 0.08em; text-transform: uppercase; color: var(--text-3); }
.dd-opt { display: flex; align-items: center; gap: 6px; height: 28px; padding: 0 10px 0 6px; border-radius: 6px; color: var(--text-2); cursor: pointer; white-space: nowrap; }
.dd-opt.hover { background: var(--panel-3); color: var(--text); }
.dd-opt.on { color: var(--text); font-weight: 600; }
.dd-opt.off { cursor: default; }
.dd-check { flex: none; width: 14px; display: grid; place-items: center; color: var(--accent-2); }
.dd-text { flex: 1; overflow: hidden; text-overflow: ellipsis; }
.dd-note { font: 10.5px var(--mono); color: var(--text-3); }
.dd-empty { padding: 8px 10px; color: var(--text-3); }
.dd-list.rich { max-width: 400px; padding: 5px; }
.dd-list.rich .dd-group { margin-top: 4px; padding: 9px 9px 4px; border-top: 1px solid var(--line); }
.dd-list.rich > div:first-child .dd-group { margin-top: 0; border-top: none; }
.dd-list.rich .dd-opt { gap: 9px; height: auto; min-height: 30px; padding: 6px 9px; white-space: normal; }
.dd-list.rich .dd-text { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
.dd-main { font-size: 13px; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.dd-sub { font-size: 11.5px; color: var(--text-3); line-height: 1.35; }
.dd-list.rich .dd-note { padding: 2px 6px; border-radius: 4px; background: var(--panel-3); color: var(--text-2); }
.dd-opt.off .dd-main { color: var(--text-3); font-weight: 500; }
.dd-opt.off .dd-sub { color: rgba(245, 165, 36, 0.8); }
.dd-opt.off .dd-icon { opacity: 0.35; }
@media (prefers-reduced-motion: reduce) { .dd-list { animation: none; } }
```

### 4.15 Champs texte et nombre (`ui/fields.tsx`)

- Texte : `<input>` simple dans un `Field` ; `TextInput` (valeur locale, `commitOnBlur`, Entrée = valider en quittant le champ, `onKeyDown` stoppe la propagation pour ne pas déclencher les raccourcis, `autoGrow` pour une zone qui suit son texte).
- Nombre : `NumberField({ label?, value, onChange, step = 1, display = 1, precision = 0, suffix?, min?, max?, title? })` → `<label className="num-field"><span className="scrub">{label}</span><input /><span className="suffix">{suffix}</span></label>`. Champ texte (pas `type=number`) aligné à droite en mono ; `display` multiplie l'affichage (100 → pourcentage d'une valeur 0..1) ; focus = tout sélectionner ; validation au blur / Entrée, accepte une expression simple (`100+20`, `1920/2`), bornée par `min`/`max`, valeur inchangée = rien envoyé ; ↑ ↓ = ±pas (Maj ×10) ; **glisser le libellé** horizontalement règle la valeur (`setPointerCapture`, Maj ×10, Alt ×0,1) ; **molette / pavé tactile** sans cliquer : le champ s'arme si le pointeur s'y est posé depuis 120 ms et que la page n'a pas défilé depuis 300 ms (sinon la page défile) ; un cran de molette = un pas, le pavé tactile cumule 16 px par pas, Maj ×10, Alt ×0,1. Infobulle : « Molette ou pavé tactile pour régler · Shift ×10 · Alt ×0,1 ». `installNumberWheel()` applique la même molette à tout `<input type="number">`.

```css
.num-field { display: flex; align-items: center; gap: 6px; min-width: 0; }
.num-field .scrub { cursor: ew-resize; color: var(--text-2); font-size: 12px; min-width: 60px; user-select: none; }
.num-field .scrub:hover { color: var(--accent-2); }
.num-field input { width: 100%; min-width: 46px; font-family: var(--mono); font-size: 12px; text-align: right; }
.num-field .suffix { color: var(--text-3); font-size: 11px; width: 14px; }
```

### 4.16 Interrupteurs

Toute case à cocher est un interrupteur : `<label className="check"><input type="checkbox" checked={v} onChange={…} />{t('Libellé')}</label>` (variante `.switch` identique).

```css
.check { display: flex; gap: 8px; align-items: center; color: var(--text-2); }
.check input[type='checkbox'] { appearance: none; width: 34px; height: 18px; border-radius: 9px; background: var(--panel-3); position: relative; cursor: pointer; margin: 0; flex: none; }
.check input[type='checkbox']::after { content: ''; position: absolute; left: 2px; top: 2px; width: 14px; height: 14px; border-radius: 50%; background: var(--text-2); transition: left 0.15s; }
.check input[type='checkbox']:checked { background: var(--accent); }
.check input[type='checkbox']:checked::after { left: 18px; background: #fff; }
.check input[type='checkbox']:disabled { opacity: 0.5; cursor: default; }
```

### 4.17 Recherche, barre d'outils, puces de filtre

```tsx
<div className="media-toolbar">
  <label className="search"><Search size={14} /><input value={q} placeholder={t('Search …')} onChange={…} /></label>
  <div className="asr-chips">
    <button className={`asr-chip ${on ? 'on' : ''}`}>{t('All …')} <span className="asr-chip-n">{n}</span></button>
  </div>
</div>
```

```css
.media-toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.search { display: flex; align-items: center; gap: 8px; padding: 0 10px; height: 32px; width: 280px; max-width: 100%; border-radius: 8px; border: 1px solid var(--line); background: var(--panel); color: var(--text-3); }
.search input { flex: 1; min-width: 0; border: none; background: none; padding: 0; outline: none; }
.asr-chips { display: flex; gap: 6px; flex-wrap: wrap; }
.asr-chip { border: 1px solid var(--line-2); background: none; border-radius: 999px; padding: 3px 9px; font-size: 11px; color: var(--text-2); cursor: pointer; white-space: nowrap; }
.asr-chip:hover { color: var(--text); }
.asr-chip.on { border-color: var(--accent); background: var(--accent-soft); color: var(--text); }
.asr-chip-n { margin-left: 4px; opacity: 0.7; font-variant-numeric: tabular-nums; }
```

### 4.18 Tableaux

Tableau de page dans une carte (`.user-table`) ; tableau de données dense (`.data-table`, en-tête collant) ; liste de lignes simples (`.update-rows` / `.update-row`, § 5.6).

```css
.user-table { width: 100%; border-collapse: collapse; background: var(--panel); border: 1px solid var(--line); border-radius: 12px; overflow: hidden; }
.user-table th { text-align: left; font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-3); font-weight: 600; padding: 10px 14px; border-bottom: 1px solid var(--line); }
.user-table td { padding: 10px 14px; border-bottom: 1px solid var(--line); font-size: 12.5px; }
.user-table tr:last-child td { border-bottom: none; }
.data-table { border-collapse: collapse; width: 100%; font-size: 12px; }
.data-table th, .data-table td { padding: 5px 9px; border-bottom: 1px solid var(--line); text-align: left; white-space: nowrap; max-width: 320px; overflow: hidden; text-overflow: ellipsis; user-select: text; }
.data-table th { position: sticky; top: 0; background: var(--panel-2); color: var(--text-2); font-weight: 600; }
```

### 4.19 Toasts

`toast(message, kind = 'info' | 'error')` dans le store : empilés en bas à droite, disparaissent après 4 s. Texte court, sans point final pour un libellé, avec pour une phrase.

```tsx
<div className="toasts">{toasts.map((x) => <div key={x.id} className={`toast ${x.kind}`}>{x.message}</div>)}</div>
```

```css
.toasts { position: fixed; bottom: 18px; right: 18px; display: flex; flex-direction: column; gap: 8px; z-index: 1600; }
.toast { background: var(--panel-3); border: 1px solid var(--line-2); border-left: 3px solid var(--accent); padding: 10px 14px; border-radius: 8px; max-width: 380px; box-shadow: 0 8px 30px rgba(0, 0, 0, 0.4); }
.toast.error { border-left-color: var(--live); }
```

### 4.20 Dialogues et modales (`ui/dialogs.tsx`, `ui/escape.ts`)

API : `if (!(await confirmDialog({ title, message?, confirm?, cancel?, danger? }))) return;` · `const name = await promptDialog({ title, message?, label?, value?, placeholder?, confirm?, password? })` (null si annulé ou vide) · `await alertDialog({ title, message? })`. File d'attente zustand, un seul `<DialogHost />` monté dans `App`, un dialogue à la fois. Focus : le champ (texte sélectionné) ou le bouton de validation. Échap ou clic sur le fond = annuler ; Entrée = valider (`<form onSubmit>`). `installEscape()` : Échap n'importe où « clique » le dernier `.modal-back` (sauf si l'évènement est déjà traité).

```tsx
<div className="modal-back dialog-back" onPointerDown={() => close(false)}>
  <form className={`modal dialog ${danger ? 'danger' : ''}`} role="dialog" aria-labelledby="dialog-title" onPointerDown={(e) => e.stopPropagation()} onSubmit={(e) => { e.preventDefault(); close(true); }}>
    <header>
      <span className="dialog-icon">{danger ? <AlertTriangle size={16} /> : <Info size={16} />}</span>
      <h2 id="dialog-title">{title}</h2>
      <button type="button" className="icon-btn ghost" title={t('Close')} onClick={() => close(false)}><X size={16} /></button>
    </header>
    {message && <div className="dialog-message">{message}</div>}
    <footer>
      <button type="button" className="btn" onClick={() => close(false)}>{cancel || t('Cancel')}</button>
      <button type="submit" className={`btn ${danger ? 'danger-solid' : 'primary'}`}>{confirm || t('Confirm')}</button>
    </footer>
  </form>
</div>
```

Modale d'édition : même structure, `<form className="modal wide">` (720 px), en-tête sans icône, corps en `.set-grid two` de `Field`, pied « Annuler » + bouton `primary` avec icône.

```css
.modal-back { position: fixed; inset: 0; z-index: 100; background: rgba(5, 6, 10, 0.65); backdrop-filter: blur(3px); display: grid; place-items: center; padding: 16px; }
:root[data-theme='light'] .modal-back { background: rgba(22, 25, 37, 0.35); }
.modal { width: min(460px, 100%); max-height: calc(100vh - 32px); overflow: auto; background: var(--panel); border: 1px solid var(--line-2); border-radius: 14px; padding: 20px; display: flex; flex-direction: column; gap: 14px; box-shadow: 0 30px 80px rgba(0, 0, 0, 0.5); }
.modal.wide { width: min(720px, 100%); }
.modal header, .modal footer { display: flex; align-items: center; gap: 8px; }
.modal header h2 { font-size: 16px; margin: 0; flex: 1; }
.modal footer { justify-content: flex-end; padding-top: 4px; }
.dialog-back { z-index: 1500; }
.modal.dialog { width: min(440px, 100%); }
.dialog-icon { width: 28px; height: 28px; border-radius: 8px; display: grid; place-items: center; flex: none; color: var(--accent-2); background: var(--accent-soft); }
.modal.dialog.danger .dialog-icon { color: #ff8aa0; background: var(--live-soft); }
:root[data-theme='light'] .modal.dialog.danger .dialog-icon { color: var(--t-red); }
.dialog-message { color: var(--text-2); line-height: 1.55; white-space: pre-line; }
```

### 4.21 Infobulles dessinées (`ui/tooltips.ts`) et « i »

`installTooltips()` crée une fois `<div id="tip" class="tip" role="tooltip">`. Au survol (pas au toucher) de l'élément le plus proche ayant un `title`, le `title` passe dans `data-tip` (le navigateur n'affiche plus le sien ; `aria-label` posé si l'élément n'a pas de texte), puis la carte apparaît après **380 ms**, à +14 / +18 px du pointeur, le suit, se retourne au bord de la fenêtre (marge 8), disparaît quand le pointeur quitte l'élément ou sur `pointerdown`, molette, touche, perte du focus. Donc : on écrit simplement `title={t('…')}`. `InfoTip` : `<span className="ux3-info" title={text} aria-label={text} role="img" tabIndex={0}><Info size={13} /></span>` après un libellé, avec parcimonie.

```css
.tip { position: fixed; left: 0; top: 0; z-index: 5000; max-width: 300px; padding: 6px 9px; border-radius: 7px; border: 1px solid var(--line-2); background: color-mix(in srgb, var(--panel-3) 94%, transparent); backdrop-filter: blur(6px); color: var(--text); font-size: 11.5px; line-height: 1.4; box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45); pointer-events: none; opacity: 0; transition: opacity 0.12s; white-space: pre-line; }
.tip.on { opacity: 1; }
.ux3-info { display: inline-grid; place-items: center; width: 16px; height: 16px; margin-left: 4px; color: var(--text-3); cursor: help; vertical-align: -3px; flex: none; border-radius: 50%; }
.ux3-info:hover { color: var(--text); }
```

### 4.22 Badges, points d'état, barres de charge

```css
.badge { font-size: var(--fs-11); padding: 1px 6px; border-radius: 10px; background: var(--panel-3); color: var(--text-3); }
.badge.warn { background: rgba(245, 165, 36, 0.15); color: var(--warn); }
.nav-dot { width: 7px; height: 7px; border-radius: 50%; flex: none; background: var(--text-3); }
.nav-dot.ok { background: var(--pvw); box-shadow: 0 0 0 3px rgba(34, 197, 94, 0.15); }
.nav-dot.warn { background: var(--warn); box-shadow: 0 0 0 3px rgba(245, 165, 36, 0.15); }
/* tons : ok < 60 %, warn < 85 %, bad au-delà ; idle = gris */
.tone-ok { --tone: var(--pvw); } .tone-warn { --tone: var(--warn); } .tone-bad { --tone: var(--live); } .tone-idle { --tone: var(--text-3); }
.perf-bar { position: relative; display: block; height: 6px; min-width: 40px; border-radius: 4px; background: rgba(255, 255, 255, 0.07); box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.04); overflow: hidden; }
.perf-bar-track { position: absolute; inset: 0; border-radius: inherit; overflow: hidden; }
.perf-bar-fill { position: absolute; inset: 0; border-radius: inherit; background: var(--tone, var(--pvw)); transition: transform 0.6s ease, background-color 0.3s; } /* style={{ transform: `translateX(${pct - 100}%)` }} */
.perf-dot { flex: none; width: 7px; height: 7px; border-radius: 50%; background: var(--tone); box-shadow: 0 0 0 3px color-mix(in srgb, var(--tone) 18%, transparent); }
```

### 4.23 Barre d'enregistrement

Les pages de réglages modifient une copie locale ; tant qu'elle diffère (comparaison JSON), une barre monte du bas : « Des modifications ne sont pas enregistrées. » · « Abandonner » · `primary` « Enregistrer et appliquer » (`Save` 14 ; « Application… » pendant l'envoi). Puis toast « Paramètres enregistrés », rechargement des réglages (réessai chaque seconde, 15 fois, pendant le redémarrage) ; si le port d'écoute change, la page part vers le nouveau port.

```css
.save-bar { position: fixed; left: 50%; bottom: 18px; transform: translate(-50%, 150%); width: min(720px, calc(100% - 32px)); display: flex; align-items: center; gap: 10px; padding: 10px 12px 10px 18px; background: var(--panel-3); border: 1px solid var(--line-2); border-radius: 12px; box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5); transition: transform 0.2s; z-index: 50; }
.save-bar.show { transform: translate(-50%, 0); }
:root[data-theme='light'] :is(.toast, .menu, .modal, .login-card, .dd-list, .tip, .save-bar) { box-shadow: 0 12px 32px rgba(22, 25, 37, 0.16); }
```

---

## 5. Fonctions de base à reproduire à l'identique

### 5.1 Connexion

**Comptes** (`auth.rs`) dans `users.json` (tableau) :

```rust
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)] #[serde(rename_all = "camelCase")]
pub enum Role { Admin, /* les rôles propres à cette app */ Viewer /* lecture seule */ }
#[derive(Serialize, Deserialize, Clone)] #[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String, pub username: String,
    #[serde(default)] pub display_name: String, pub role: Role,
    #[serde(default)] pub disabled: bool,
    #[serde(default)] pub lang: String,   // "en" | "fr" ; vide = celle du navigateur
    #[serde(default)] pub theme: String,  // "dark" | "light" | "system" ; vide = sombre
    #[serde(default, skip_serializing_if = "String::is_empty")] pub sso: String,  // "issuer|subject"
    #[serde(default, skip_serializing_if = "std::ops::Not::not")] pub role_fixed: bool,
    password_hash: String,               // jamais envoyé
}
// PublicUser = {id, username, displayName, role, disabled, lang, theme, sso: bool, roleFixed}
```

- Mot de passe : Argon2 par défaut (`Argon2::default().hash_password`, sel = 16 octets d'un UUID v4), vérifié aussi pour un nom inconnu (hachage factice) pour ne rien révéler par le temps de réponse ; vérification dans `spawn_blocking`, 4 à la fois au plus (`Semaphore`). Règles : mot de passe ≥ 8 caractères ; nom = lettres, chiffres, `. - _ @`, 64 au plus, unique sans tenir compte de la casse ; nom affiché vide = le nom d'utilisateur ; toujours au moins un admin actif.
- Freinage des essais (`guard::Logins`) : par (adresse IP + nom) 5 essais libres, par adresse 20 ; ensuite attente `2^(essais en trop) s` (plafond 5 min), oubli après 15 min sans essai ; un succès remet le compteur (IP + nom) à zéro.
- `GET /api/auth/state` → `{needsSetup, user: PublicUser | null, serverName, sso: "Microsoft" | null, localAccounts: "none" | "adminOnly" | "all"}`.
- `POST /api/auth/login {username, password}` → `200 {user, token}` + `Set-Cookie`. Erreurs : `401 "wrong user name or password"`, `429 "too many tries: wait N s"`, `403 "sign in with Microsoft"` (SSO actif et comptes locaux refusés pour ce rôle, sauf depuis la machine elle-même). Un compte désactivé ne se connecte pas.
- `POST /api/auth/logout` → retire la session, `Set-Cookie` vide `Max-Age=0`, `204`. Client : ferme le WebSocket et vide le store (retour à la page de connexion).
- Droits : chaque route vérifie (`require(me.is_admin())`…) ; le client cache ce qu'un rôle ne peut pas faire (`can.admin(user)`) et redirige une page interdite vers l'accueil.

**Page de connexion** (`pages/Login.tsx`) : centrée sur le fond dégradé, sélecteur `EN | FR` fixé en haut à droite (`.lang-switch.login-lang` ; la langue choisie ici devient celle de l'utilisateur à la connexion), carte de 400 px : `<Brand stacked />` et, si le serveur a un autre nom que la marque, ce nom en `.muted` · (SSO) bouton `a.btn.primary.block.sso-btn` (`KeyRound` 15) « Se connecter avec {name} » vers `/api/auth/sso/start?next=<chemin courant>` puis séparateur « compte local » · champs `set-field` « Nom d’utilisateur » (autofocus, `autocomplete="username"`) et « Mot de passe » · bouton `btn block primary` (`btn block` simple si SSO) « Se connecter » (`LogIn` 15), désactivé si un champ est vide ou pendant l'envoi · encadré d'erreur `.login-error`. Messages : 401 « Nom d’utilisateur ou mot de passe incorrect. », 429 « Trop d’essais : patientez un instant, puis réessayez. », 403 avec SSO « Se connecter avec {name} », sinon le texte du serveur ; retour SSO `?sso_error=noGroup|disabled|expired|failed&sso_detail=…` → « Aucun de vos groupes de l’annuaire ne donne accès à <Marque>. » / « Ce compte est désactivé. » / « La connexion a pris trop de temps : réessayez. » / « L’annuaire a refusé la connexion : {détail} » (puis l'URL est nettoyée).

```css
.login-page { min-height: 100vh; display: grid; place-items: center; padding: 24px 16px; background: radial-gradient(ellipse at 30% 0%, #1d1840 0%, #0b0c12 60%); }
:root[data-theme='light'] .login-page { background: radial-gradient(ellipse at 30% 0%, #dfe3fb 0%, var(--bg) 60%); }
.login-lang { position: fixed; top: 16px; right: 16px; }
.login-card { width: min(400px, 100%); background: var(--panel); border: 1px solid var(--line); border-radius: 16px; padding: 34px 30px 28px; display: flex; flex-direction: column; gap: 14px; box-shadow: 0 30px 80px rgba(0, 0, 0, 0.45); }
.login-head { display: flex; flex-direction: column; align-items: center; gap: 8px; padding-bottom: 22px; margin-bottom: 6px; border-bottom: 1px solid var(--line); }
.login-head .muted { margin: 0; font-size: 12.5px; }
.login-intro { display: flex; gap: 10px; align-items: flex-start; background: var(--accent-soft); color: var(--accent-2); border-radius: 8px; padding: 10px 12px; font-size: 12.5px; }
.login-intro svg { flex: none; margin-top: 1px; }
.login-error { color: var(--live); background: var(--live-soft); border-radius: 8px; padding: 8px 12px; font-size: 12.5px; }
.sso-btn { justify-content: center; text-decoration: none; }
.login-sep { display: flex; align-items: center; gap: 10px; color: var(--text-3); font-size: 11px; margin: 2px 0; }
.login-sep::before, .login-sep::after { content: ''; flex: 1; border-top: 1px solid var(--line); }
```

### 5.2 Une session par utilisateur

Ouvrir une session (mot de passe ou SSO) **ferme toutes les autres sessions du même utilisateur** : elles sont retirées de la table et leurs jetons sont publiés sur un `broadcast` `ended`. La session WebSocket qui porte ce jeton envoie `{"type":"sessionEnded"}` puis se ferme. Le client met `elsewhere: true` et `auth.user = null` : la page de connexion s'affiche avec l'erreur « Connecté sur un autre poste. ». Les autres utilisateurs ne sont pas touchés. Désactiver un compte ou changer son mot de passe ferme aussi toutes ses sessions ; tout changement d'un utilisateur (rôle, désactivé, supprimé) publie son id sur `changed`, ce qui ferme ses WebSocket (la page se reconnecte avec ses nouveaux droits ou revient à la connexion).

### 5.3 Premier démarrage

Tant que `users.json` est vide (`needsSetup`), la page de connexion devient la création du compte administrateur : encadré `.login-intro` (`ShieldCheck`) « Bienvenue ! Créez le compte administrateur. » ; « Nom d’utilisateur » prérempli `admin`, « Nom affiché » (placeholder « Facultatif »), « Mot de passe », « Répéter le mot de passe » avec l'aide « Au moins 8 caractères. » ; bouton « Créer le compte administrateur » ; erreur locale « Les mots de passe ne correspondent pas. » ; en bas `.set-hint` : « Par sécurité, cette page ne marche que sur le serveur lui-même (http://localhost). À distance, lancez ceci sur le serveur : `<bin> user add admin` ». Serveur : `POST /api/auth/setup {username, password, displayName}` → crée un admin et le connecte (`{user, token}` + cookie) ; `409 "already set up"` s'il existe déjà un compte ; `403` si la requête ne vient pas de la machine elle-même (`guard::this_machine`). Au démarrage sans compte, le serveur journalise comment créer l'admin.

### 5.4 Utilisateurs (Système › Accès › Utilisateurs)

API (admin) : `GET /api/users` → `PublicUser[]` ; `POST /api/users` `{username, displayName, role, disabled, password, roleFixed}` → `PublicUser` (si SSO actif et mot de passe vide : mot de passe aléatoire, connexion par l'annuaire seulement) ; `PUT /api/users/{id}` (mot de passe vide = inchangé) ; `DELETE /api/users/{id}` → 204 (`400 "you cannot delete your own account"` ; refus si c'était le dernier admin actif).

Page (`pages/Users.tsx`) : `PageHead` « Utilisateurs », sous-titre « {n} utilisateurs » (`tn`), bouton `primary` « + Nouvel utilisateur ». Barre : recherche « Rechercher des utilisateurs » + puces « Tous les utilisateurs n », une par rôle au pluriel (« Admins n »…), « Désactivé n » s'il y en a. Grille `.user-grid` de cartes `.user-card role-<role>` (clic = modifier) : avatar carré 40 px (initiale, couleur du rôle), nom affiché, `@nom · vous`, badge `SSO`, badge de rôle avec icône (11) ; ligne `Ban` 12 « Compte désactivé » (orange) ; pied : « Modifier », « Mot de passe » (pas pour un compte SSO), espace, corbeille (pas pour soi) → `confirmDialog({ title: 'Supprimer l’utilisateur « {name} » ?', confirm: 'Supprimer', danger: true })`. Dernière case : carte pointillée « + Nouvel utilisateur ». Aucun résultat : `.empty.big` « Aucun résultat ».
Modale `.modal.wide` « Nouvel utilisateur » / « Modifier {name} » : `.set-grid.two` « Nom d’utilisateur », « Nom affiché », « Mot de passe » (placeholder « Au moins 8 caractères ») ou « Nouveau mot de passe » (« Laisser vide pour le garder ») ou, pour un compte SSO, « Connexion : SSO », « État » avec l'interrupteur « Compte désactivé » ; « Rôle » en `.role-pick` (un bouton par rôle, `title` = ce que fait le rôle) ; si SSO : interrupteur « Rôle fixé à la main » ; pied « Annuler » / « Créer l’utilisateur » (`Plus`) ou « Enregistrer » (`KeyRound`). Libellés de rôle : `tc('role', role)` → « Administrateur », …, « Lecteur ».

```css
.user-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(290px, 1fr)); gap: 16px; }
.user-card { background: var(--panel); border: 1px solid var(--line); border-radius: 14px; padding: 16px; display: flex; flex-direction: column; gap: 12px; cursor: pointer; position: relative; overflow: hidden; transition: border-color 0.15s, transform 0.15s, box-shadow 0.15s; }
.user-card::before { content: ''; position: absolute; inset: 0 0 auto; height: 64px; background: radial-gradient(ellipse at 20% 0%, var(--role-glow, rgba(143, 163, 255, 0.18)), transparent 70%); pointer-events: none; }
.user-card.role-admin { --role: #ff8aa0; --role-glow: rgba(255, 59, 92, 0.16); }      /* clair : --role: var(--t-red) */
/* rôle 2 : --role: var(--accent-2); --role-glow: rgba(143, 163, 255, 0.2) · rôle 3 : --role: var(--pvw); --role-glow: rgba(34, 197, 94, 0.14) */
.user-card.role-viewer { --role: var(--text-2); --role-glow: rgba(163, 168, 191, 0.12); }
.user-card:hover { border-color: var(--line-2); transform: translateY(-2px); box-shadow: 0 14px 34px rgba(0, 0, 0, 0.35); }
.user-card.dim { opacity: 0.6; }
.user-card-head { display: flex; align-items: center; gap: 12px; position: relative; }
.user-avatar { width: 40px; height: 40px; border-radius: 12px; flex: none; display: grid; place-items: center; font-size: 17px; font-weight: 800; color: var(--role, var(--text)); background: var(--panel-3); border: 1px solid var(--line-2); }
.user-card-name { flex: 1; min-width: 0; display: flex; flex-direction: column; }
.user-card-name strong { font-size: 14.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.user-card .badge { display: inline-flex; align-items: center; gap: 4px; }
.badge.role-admin { background: var(--live-soft); color: #ff8aa0; }                     /* clair : color: var(--t-red) */
/* rôle 2 : background: var(--accent-soft); color: var(--accent-2) · rôle 3 : background: rgba(34, 197, 94, 0.15); color: var(--pvw) */
.badge.role-viewer { background: var(--panel-3); color: var(--text-2); }
.badge.sso { margin-left: 6px; font-size: 10px; padding: 1px 6px; }
.user-disabled { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--warn); }
.user-card-actions { display: flex; align-items: center; gap: 6px; margin-top: auto; padding-top: 10px; border-top: 1px solid var(--line); }
.user-new { min-height: 190px; }
.role-pick { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 8px; }
.role-pick button { text-align: left; display: flex; flex-direction: column; gap: 3px; background: var(--panel-2); border: 1px solid var(--line); border-radius: 9px; padding: 9px 11px; color: var(--text); cursor: pointer; }
.role-pick button.on { border-color: var(--accent); background: var(--accent-soft); }
```

### 5.5 SSO OpenID Connect (Système › Accès › Sécurité)

Réglages dans `config.json` → `security.sso` : `{enabled: false, provider: "entra"|"okta"|"keycloak"|"google"|"other", issuer: "", clientId: "", clientSecret: "", groupsClaim: "groups", groups: [{group, role}], defaultRole: null|role, localAccounts: "adminOnly"}`. Actif si `enabled` et `issuer` et `clientId` remplis. Nom du bouton : Microsoft, Okta, Keycloak, Google, sinon « SSO ».

- `GET /api/auth/sso/start?next=/chemin` : lit `<issuer>/.well-known/openid-configuration` (mis en cache 1 h), crée `state`, `nonce`, `code_verifier` aléatoires (gardés 10 min en mémoire avec `redirect_uri` et `next` — seulement un chemin local commençant par `/`, sans `//` ni `\`), redirige vers `authorization_endpoint?response_type=code&client_id=…&redirect_uri=<scheme>://<host>/api/auth/sso/callback&scope=openid profile email&state=…&nonce=…&code_challenge=<S256>&code_challenge_method=S256` (`host` = `X-Forwarded-Host` ou `Host`).
- `GET /api/auth/sso/callback` : échange le code (`grant_type=authorization_code`, `code_verifier` ; secret en `Authorization: Basic`, sinon `client_id` dans le corps), vérifie l'`id_token` (signature RS256/PS256/ES256 avec les clés JWKS, rechargées une fois si la clé est inconnue ; `iss`, `aud`, `exp` + 120 s de marge, `nonce`), en tire l'identité (`clé = "issuer|sub"`, login = `email` sauf si `email_verified` vaut false, sinon `preferred_username`, sinon `upn` ; adresse garantie = email vérifié ou `upn`), le rôle = **le plus élevé** des groupes listés (comparaison sans casse, `/` initial ignoré), sinon `defaultRole`, sinon refus. Compte : retrouvé par sa clé SSO, ou compte local créé d'avance dont le nom contient `@` et égale l'adresse garantie, sinon créé (nom tiré du login, `-2`, `-3` si pris ; sans mot de passe). Le rôle suit les groupes à chaque connexion sauf `roleFixed` ; jamais de perte du dernier admin par l'annuaire. Succès : cookie + redirection vers `next`. Échec : redirection `/?sso_error=noGroup|disabled|expired|failed&sso_detail=<texte>`.
- `POST /api/security/sso/test {issuer}` (admin) → `{issuer, authorizationEndpoint, tokenEndpoint, keys}`.
- Le secret est renvoyé masqué `"••••••••"` et conservé s'il revient masqué.
- `localAccounts` : `none` (personne, sauf depuis la machine elle-même pour réparer), `adminOnly` (secours), `all`.

Carte « Connexion par l’annuaire de l’entreprise (SSO) » (`KeyRound`), interrupteur Activé/Désactivé dans les actions de la carte ; si activé, `.set-grid` : « Fournisseur » (`Segmented` Microsoft Entra / Okta / Keycloak / Google / Autres), « Tenant / Issuer » (placeholder selon le fournisseur, ex. `https://login.microsoftonline.com/<tenant>/v2.0`), « ID client », « Secret » (password), « URL de retour » (code + bouton « Copier »), « Claim des groupes », « Rôles par groupe » (lignes : champ groupe + `Dropdown` rôle + `icon-btn ghost X` ; bouton `btn small` « + groupe »), « Sans groupe connu » (`Segmented` Refusé / rôles), « Comptes locaux » (`Segmented` Désactivé / Admins seulement (secours) / Tout), « Tester la connexion » + résultat. Le tout enregistré par la barre d'enregistrement (§ 4.23).

### 5.6 Mises à jour (Système › Serveur › Mises à jour)

**Serveur (`update.rs`)** — jamais d'installation automatique : la vérification informe seulement.

- Réglages `<data>/update/settings.json` (0600) : `{repo: "<owner>/<repo>" (constante DEFAULT_REPO), token: "" (lecture seule, dépôt privé, jamais renvoyé), autoCheck: true}`.
- Plateforme du binaire : `linux-x86_64`, `linux-arm64`, `macos-arm64`, `macos-x86_64`, `windows-x86_64` ; paquets acceptés par ordre de préférence : `-<plateforme>.tar.gz` (Windows `.zip`), puis les installateurs.
- Vérifier : `GET https://api.github.com/repos/<repo>/releases?per_page=50` avec `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`, `Authorization: Bearer <token>` si présent, `User-Agent: <app>/<version>` ; client `ureq` avec le magasin de certificats du système (`RootCerts::PlatformVerifier`, passe les proxys d'entreprise), délai global 600 s, **pas de redirection automatique**. Garde les releases non brouillon, non préversion, dont le paquet de cette plateforme est entièrement téléversé (`state == "uploaded"`) ; version = tag sans `v` ; tri numérique décroissant (`"1.2.10" > "1.2.9"`). Erreurs lisibles : 401 « GitHub refused the token (expired or revoked?) », 403 « GitHub refused the request (rate limit, or the token cannot read <repo>) », 404 sans jeton « <repo> not found: the repository is private, add a read-only token ».
- État en mémoire : `{phase: idle|checking|downloading|installing|restarting|error, progress: 0..1, message, latest, releases, lastCheck}`.
- Vérification automatique : 30 s après le démarrage puis toutes les 6 h, si `autoCheck` et rien en cours.
- Installer (en fond) : télécharge `…/releases/assets/<id>` avec `Accept: application/octet-stream` (la redirection vers le stockage est suivie **sans** le jeton), progression, limite 512 Mo, vérifie le SHA-256 publié par GitHub (`digest: "sha256:<hex>"`) ; extrait `<bin>` du `.tar.gz` / `.zip` (ou du `.pkg` via `pkgutil --expand-full`, du `.msi` via `msiexec /a … /qn TARGETDIR=…`) ou accepte un binaire nu (ELF, Mach-O, PE) ; l'écrit à côté (`<bin>.new`, 0755) et lance `<bin>.new --version` : doit répondre `<bin> x.y.z` ; refuse une version qui n'est pas plus récente (sauf version choisie dans la liste ou « autoriser plus ancien ») ; garde l'actuel en `<bin>.previous` et met le nouveau à sa place (`rename` atomique ; sous Windows on renomme d'abord l'exe en cours) ; écrit `pending.json {from, to, starts: 0}` et `history.json {from, to, at, outcome: "installed"}` ; phase `restarting` ; redémarre 800 ms plus tard : supervisé → `exit(75)` (le gestionnaire de services relance) ; au premier plan → relance le même exécutable avec les mêmes arguments puis `exit(0)`. Droits insuffisants (Linux) : message qui dit de relancer l'installateur.
- Garde au démarrage : si `pending.json` concerne cette version, compte les démarrages ; au-delà de 3 démarrages ratés, remet `<bin>.previous` (`outcome: "restored automatically"`) et redémarre ; après 60 s de fonctionnement, `pending.json` est supprimé (version validée).
- Routes (admin ; `check` et `install` aussi pour le lanceur sur la machine) :
  `GET /api/system/update` → `{current, platform, package: "<Marque>-<version>-<plateforme>.tar.gz", repo, hasToken, autoCheck, previous: "x.y.z"|null, history, status, supervised}` ·
  `PUT /api/system/update/settings {repo (owner/name ou URL github.com), token?: null = garder, "" = retirer, autoCheck}` → 204 (`400 "the repository is written owner/name"`) ·
  `POST /api/system/update/check` → dernière release ou `null` (502 + message en cas d'échec) ·
  `POST /api/system/update/install[?version=x.y.z]` → 202 ·
  `POST /api/system/update/upload?allowOlder=false` (corps = le fichier, 512 Mo max) → `{version}` puis redémarrage ·
  `POST /api/system/update/rollback` → `{version}` puis redémarrage.

**Page (`settings/Updates.tsx`)** : une carte « Mises à jour » (`CloudDownload`), action `btn small` « Vérifier maintenant » (`RefreshCw`).
- En-tête `.update-head` : « INSTALLÉE » / version en mono 22 px / plateforme ; à droite : en cours → icône qui tourne + message (+ `<progress>` pendant le téléchargement) ; erreur → `.update-error` ; plus récente → « Version **x.y.z** est disponible (date). » + `primary` « Installer x.y.z » ; sinon « À jour : dernière version x.y.z, vérifié il y a 5 min. » ; jamais vérifié → « Pas encore vérifié. ».
- `<details className="update-notes">` « Nouveautés de x.y.z » (notes de la release dans un `<pre>`).
- « VERSIONS DISPONIBLES » : `.update-rows` (version, date, « 12 Mo », bouton « Installer » ou « Revenir en arrière », badge « Installée » sur la version courante).
- Historique en `.hint` (`History` 12) : « 1.2.0 → 1.3.0 (installée), il y a 2 h » ou « La version {from} n’a pas démarré : {to} a été remise automatiquement ». L'issue passe par `t(outcome)` : mettre `installed` → « installée », `rolled back` → « retour en arrière », `restored automatically` → « remise automatiquement » dans le dictionnaire (le script ne voit pas ces clés, elles viennent du serveur).
- `.set-grid` : « Dépôt GitHub » (enregistré en quittant le champ), « Jeton d’accès » (password, placeholder `••••••••` si enregistré sinon `github_pat_…`, badge « enregistré », boutons « Enregistrer le jeton » / « Retirer le jeton »), « Vérification automatique » (interrupteur « Chercher une nouvelle version toutes les 6 heures »).
- `.update-offline` : « Mise à jour hors ligne » + nom du paquet en `code`, interrupteur « Autoriser les versions plus anciennes », bouton « Importer un paquet » (`Upload`, `<input type="file" accept=".gz,.tgz,.zip,.pkg,.msi" hidden>`), bouton « Revenir à x.y.z » si une version précédente existe.
- Toute installation passe par `confirmDialog` (« Installer <Marque> x.y.z ? » / « Revenir à la version x.y.z ? » en `danger`), message : « Le serveur redémarre : il est indisponible quelques secondes. ».
- Pendant une phase active : relecture toutes les 700 ms. Après installation : attente du retour (`GET /api/health` chaque seconde jusqu'à ce que `version` change) → toast « <Marque> {version} est en marche » puis `location.reload()` 600 ms après ; au-delà de 2 min : toast d'erreur « Le serveur n’est pas revenu en 2 minutes : consultez ses journaux. ».

```css
.update-head { display: flex; align-items: center; gap: 24px; flex-wrap: wrap; }
.update-head > div:first-child { display: flex; flex-direction: column; gap: 2px; }
.update-version { font-size: 22px; font-family: var(--mono); }
.update-latest { flex: 1; display: flex; align-items: center; justify-content: flex-end; gap: 12px; flex-wrap: wrap; min-width: 260px; }
.update-busy { display: inline-flex; align-items: center; gap: 8px; color: var(--accent-2); }
.update-busy progress { width: 140px; accent-color: var(--accent); }
.update-error { color: var(--live); background: var(--live-soft); border-radius: 8px; padding: 6px 10px; font-size: 12.5px; }
.update-notes summary { cursor: pointer; color: var(--text-2); font-size: 12.5px; }
.update-notes pre { white-space: pre-wrap; font-family: inherit; font-size: 12.5px; color: var(--text-2); background: var(--panel-2); border-radius: 8px; padding: 10px 12px; max-height: 240px; overflow: auto; }
.update-list { display: flex; flex-direction: column; gap: 6px; }
.update-rows { display: flex; flex-direction: column; max-height: 260px; overflow-y: auto; border: 1px solid var(--line); border-radius: 8px; }
.update-row { display: flex; align-items: center; gap: 12px; padding: 6px 10px; min-height: 38px; }
.update-row + .update-row { border-top: 1px solid var(--line); }
.update-row strong { min-width: 64px; font-variant-numeric: tabular-nums; }
.update-row.here { background: color-mix(in srgb, var(--accent) 8%, transparent); }
.update-offline { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; border-top: 1px solid var(--line); padding-top: 14px; }
.update-offline > div { flex: 1; display: flex; flex-direction: column; gap: 2px; min-width: 240px; }
```

### 5.7 Langue FR / EN par utilisateur

`src/i18n/index.ts` (à reprendre tel quel, seule la clé change) :

```ts
import { create } from 'zustand';
import { fr } from './fr';
export type Lang = 'en' | 'fr';
export const LANGS: { id: Lang; label: string; name: string }[] = [ { id: 'en', label: 'EN', name: 'English' }, { id: 'fr', label: 'FR', name: 'Français' } ];
const KEY = '<app>.lang';
const isLang = (l: unknown): l is Lang => l === 'en' || l === 'fr';
function first(): Lang {
  try { const saved = localStorage.getItem(KEY); if (isLang(saved)) return saved; } catch { /* navigation privée */ }
  return navigator.language?.toLowerCase().startsWith('fr') ? 'fr' : 'en';
}
let current: Lang = first();
document.documentElement.lang = current;
/** La langue affichée ; les composants qui tiennent des textes se redessinent quand elle change. */
export const useLang = create<{ lang: Lang }>(() => ({ lang: current }));
export function setLang(l: string | null | undefined) {
  if (!isLang(l) || l === current) return;
  current = l; document.documentElement.lang = l;
  try { localStorage.setItem(KEY, l); } catch { /* */ }
  useLang.setState({ lang: l });
}
export const lang = () => current;
let picked = false;
/** Langue choisie à la main sur la page de connexion : elle l'emporte sur celle enregistrée pour l'utilisateur. */
export function pickLang(l: Lang) { picked = true; setLang(l); }
export const pickedByHand = () => picked;
/** Pour les dates et les nombres. */
export const locale = () => (current === 'fr' ? 'fr-FR' : undefined);
const DICT: Record<Lang, Record<string, string>> = { en: {}, fr };
/** Le texte dans la langue de l'interface, `{nom}` remplacé par vars. */
export function t(s: string, vars?: Record<string, string | number>): string {
  const out = DICT[current][s] ?? s;
  return vars ? out.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m)) : out;
}
/** Un mot court dont le français dépend du contexte : tc('theme', 'Light') cherche d'abord 'theme|Light' (« Clair »). */
export function tc(context: string, s: string, vars?: Record<string, string | number>): string {
  const key = `${context}|${s}`;
  return key in DICT[current] ? t(key, vars) : t(s, vars);
}
/** Singulier ou pluriel selon n (en français 0 et 1 sont au singulier), {n} remplacé. */
export function tn(n: number, one: string, other: string, vars?: Record<string, string | number>): string {
  const single = current === 'fr' ? Math.abs(n) < 2 : n === 1;
  return t(single ? one : other, { n, ...vars });
}
```

- Appeler `t()` **au rendu**, jamais au chargement d'un module (les listes de libellés gardent l'anglais et sont traduites là où elles s'affichent : `t(r.label)`).
- Dictionnaires : `src/i18n/fr/<partie>.ts` → `export const common: Record<string, string> = { Cancel: 'Annuler', 'Delete “{name}”?': 'Supprimer « {name} » ?', '{n} user': '{n} utilisateur', '{n} users': '{n} utilisateurs', 'theme|Dark': 'Sombre', 'theme|Light': 'Clair', 'theme|System': 'Système', 'role|admin': 'Administrateur', … }` ; `src/i18n/fr/index.ts` les fusionne : `export const fr = { ...common, ...users, ...system, … }`. Un fichier par partie de l'interface. Mêmes `{variables}` dans les deux langues.
- Au démarrage `init()` applique `setLang(auth.user?.lang)` et `setTheme(auth.user.theme || 'dark')` avant de dessiner. À la connexion : si la langue a été choisie à la main sur la page de connexion et diffère, elle est enregistrée pour l'utilisateur (`PUT /api/me/lang`), sinon on prend la sienne. `<div className="app" key={lang}>` redessine tout au changement.
- Serveur : `LANGS = ["en", "fr"]`, `PUT /api/me/lang {lang}` → `PublicUser` (400 « unknown language »).
- Script `web/scripts/i18n-check.mjs` (annexe B) : `node scripts/i18n-check.mjs` liste chaque texte de `t()` / `tn()` / `tc()` sans français (et les `{variables}` différentes), code de sortie 1 s'il en manque ; `node scripts/i18n-check.mjs --raw src/pages` liste l'anglais resté hors de `t()` (texte JSX et attributs `title`, `placeholder`, `label`, `aria-label`, `alt`…). À lancer avant chaque livraison : 0 texte sans français.

### 5.8 Thème clair / sombre par utilisateur (`theme.ts`)

Sombre par défaut, Clair, ou Système (suit l'ordinateur en direct via `matchMedia('(prefers-color-scheme: light)')`). Clair = `document.documentElement.dataset.theme = 'light'` (supprimé sinon) ; les jetons clairs vivent sous `:root[data-theme='light']` dans `styles-light.css`, qui ne contient que des surcharges. Gardé pour l'utilisateur sur le serveur (`PUT /api/me/theme {theme}`, `THEMES = ["dark","light","system"]`) et dans le navigateur (`localStorage['<app>.theme']`, lu par le script de `index.html` pour la première image). `useTheme` (zustand `{theme}`) redessine le menu ; `setTheme(t)` ignore les valeurs inconnues, enregistre, applique.

### 5.9 Préférences par utilisateur

`<data>/prefs/<userId>.json` (objet ; écriture atomique, un écrivain à la fois). `GET /api/me/display` → objet ; `PATCH /api/me/display {clé: valeur}` fusionne (`null` supprime ; clés ≤ 200 caractères ; 256 Ko au total). Client `displayPrefs.ts` : chargé avant de dessiner les pages (`loadDisplayPrefs(user.id)`), `useDisplayPref(key, fallback)` comme `useState`, envois regroupés 600 ms plus tard, et envoi `keepalive` à `pagehide`. Sert pour la taille des panneaux, les sections repliées, les choix d'affichage : même rendu sur tous les navigateurs de l'utilisateur.

### 5.10 Page Système

`/system/<section>` ; liste à gauche (`.projects-page.sys-page` + `aside.lib-nav`), section ouverte à droite (`main.library`). Groupes (titres `lib-nav-section`, items `lib-nav-item` icône 15) — garder seulement ceux qui ont un sens pour cette app :

| Groupe | Sections (icône lucide) | Réservé admin |
| --- | --- | --- |
| Supervision | Performances (`Gauge`), Santé (`HeartPulse`) | oui |
| Accès | Utilisateurs (`Users`), Sécurité (`Lock`) | oui |
| Serveur | Général (`SlidersHorizontal`), Mises à jour (`CloudDownload`), Maintenance (`RotateCcw`) | oui |

Un non-admin ne voit que les sections non réservées (le bouton « Système » de la barre du haut n'apparaît que s'il en reste une) ; sans section dans l'URL, la première visible s'ouvre. Point orange sur « Santé » (et sur « Système » dans la barre du haut) quand un plantage n'a pas été vu ou qu'il y a eu des erreurs dans l'heure. Une section à plusieurs pages les montre en `SubTabs` sous son titre. Chaque section : `PageHead` (titre seul ; sous-titre seulement factuel) puis des `Card`.
- **Général** : carte « Général » (`Server`) avec « Nom du serveur » (+ les réglages généraux de l'app) → barre d'enregistrement.
- **Maintenance** : `PageHead` « Maintenance », sous-titre `<Marque> {version} · {machine} · {os}/{arch} · en marche depuis {durée}`, bouton « Redémarrer » (`RotateCcw`, `POST /api/system/restart`, toast) ; carte « Système » (`Cpu`) en `dl.sys-list` : Version, Machine, Dossier des données (`code`), À l’écoute sur (`code`), Threads CPU, En marche depuis (`2d 3h`, `3h 12m`, `5m 10s`).
- **Sécurité** : SSO (§ 5.5) et, si besoin, HTTPS (`security.https = {mode: "off"|"selfSigned"|"custom", port: 8443, http: "redirect"|"httpsOnly"|"open"}`, `GET /api/security/tls`, `POST /api/security/tls/certificate {cert, key}` PEM, `POST /api/security/tls/self-signed`).
- **Mises à jour** : § 5.6.
- `useServerConfig()` : `GET /api/system` au chargement puis toutes les 5 s si l'onglet est visible ; copie modifiable `cfg`, `dirty`, `save()` → `PUT /api/system/config`.

### 5.11 Santé et journaux

- Journal en mémoire (`logs.rs`) : une couche `tracing` garde les 5000 dernières lignes `{seq, at (ms), level, target, message, fields: "k=v k=v"}`. `GET /api/system/logs?after=<seq>&level=<WARN…>&limit=<≤5000, défaut 1000>` → `{entries, last, file}` (lecture incrémentale) ; `GET /api/system/logs/download` → `text/plain` en pièce jointe `<app>.log`. Affichage en console sombre `.log-box` (mono), filtres niveau / recherche / pause / téléchargement.
- Santé (`health.rs`) : journal sur disque `<data>/health/journal-<jour>.jsonl` (30 jours) et rapports de plantage `<data>/health/crashes/` (90 jours : paniques, et arrêts que le serveur n'a pas faits lui-même ; `health::stopped()` marque un arrêt propre). `GET /api/system/health` → `{version, startedAt, uptimeSecs, crashes, lastCrash, unseenCrash, errors, warnings, lastHourErrors, disk: {total, free}, incidents: [{kind, level: CRASH|ERROR|WARN, cat, title, detail, first, last, count, crash?}]}` (même message regroupé avec son compte) ; `GET /api/system/health/journal` ; `POST /api/system/health/seen` ; `GET /api/system/health/export.zip` (tout, pour le support). Client : relu chaque minute pour les admins.
- Bandeau après un plantage (admins, hors pages plein écran) : en bas à gauche, « <Marque> s’est arrêté de façon inattendue le {date} puis a redémarré. » + `btn small` « Voir » (→ `/system/health`, marque vu) + `btn small ghost` « OK ».

```css
.crash-banner { position: fixed; left: 18px; bottom: 18px; z-index: 60; display: flex; align-items: center; gap: 10px; max-width: calc(100vw - 32px); padding: 8px 10px 8px 14px; border-radius: 10px; border: 1px solid rgba(232, 121, 249, 0.45); background: var(--panel); box-shadow: 0 10px 30px rgba(0, 0, 0, 0.35); color: var(--text); font-size: var(--fs-13); }
.crash-banner svg { color: #e879f9; flex: none; }
```

### 5.12 Aide et raccourcis

- Bouton d'aide `?` dans la barre du haut (§ 4.5) → page d'aide de l'app.
- Touche `?` et item « Raccourcis clavier » du menu utilisateur : calque listant les raccourcis de l'utilisateur (lignes libellé à gauche, touches `kbd` à droite, `+` entre les touches) ; Échap ferme. Raccourcis gardés par utilisateur (`GET/PUT /api/me/shortcuts {bindings: [{action, keys}] | null}`) si l'app en a.
- Échap ferme toujours le dialogue du dessus ; un champ texte arrête la propagation de ses touches pour ne pas déclencher les raccourcis.

---

## 6. Ordre de travail et vérifications

1. Poser la pile (workspace, serveur axum avec `web.rs`/`build.rs`/rust-embed, Vite/React/zustand), puis `styles.css` + `styles-light.css` + Inter, puis la coquille (`App`, barre du haut, `Brand`, `StatusPill`, `UserMenu`, `DialogHost`, toasts, infobulles, Échap).
2. Connexion complète (§ 5.1 à 5.3), puis Utilisateurs, puis la page Système avec Général, Maintenance, Mises à jour, Sécurité/SSO, Santé et journaux.
3. Vérifier : `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `npm run build`, `node scripts/i18n-check.mjs` (0 manquant). Ouvrir l'app en sombre puis en clair, en français puis en anglais : chaque écran doit être visuellement identique à la référence (mêmes hauteurs 46/32/28 px, mêmes rayons, mêmes couleurs). Tester : deux navigateurs avec le même compte (le premier revient à la connexion avec « Connecté sur un autre poste. »), premier démarrage depuis `localhost` et refus à distance, 6 mauvais mots de passe (attente), mise à jour depuis une release GitHub et depuis un paquet importé, retour à la version précédente.
4. Pour tout nouvel écran métier : maquette sur le vrai DOM/CSS, validation, puis code — avec ces composants uniquement.

---

## Annexe A — `web/src/ui/Brand.tsx` (marque et signature, à copier tel quel ; changer `MODULE`, et `Strata` si la marque diffère)

```tsx
/*
 * The Strata name and the Le Figaro AudioVisuel signature (the F with its
 * quill, then "Audio" bold and "Visuel" light, as on the Le Figaro
 * Audiovisuel logo, the two words joined).
 */

/** The F with its quill, from the LE FIGARO wordmark. */
const F = 'M145.93,22.93v-10.33h-35.97c2.12,2.08,7.43,7.77,7.92,13.05-.05-.42-.28-1.57-1.25-3.27-.98-1.72-2.72-4-5.8-6.66-.03-.03-.06-.05-.09-.08-1.11-.96-2.39-1.98-3.87-3.03C98.07,5.95,95.14,2.38,94.38.01c0,0-2.69,9.66,4.5,17.36,0,0-1.74-.36-3.41-1.02,0,0,2.98,7.41,11.12,11.33,0,0-2.11.07-3.92-.14,0,0,1.93,2.92,7.66,5.88v8.46c0,.57-.46,1.04-1.04,1.04h-3.97v7.83h24.8v-7.85h-3.76c-.57,0-1.04-.46-1.04-1.04v-3.41c1.19.21,4.51,1.95,5.96,3.12,3.92,3.17,8.11,8.05,8.11,8.05l1.83,1.13s-5.07-6.05-8.33-9.3c-2.27-2.27-6.1-6.1-6.1-6.1l-.2-.22h9.57v-7.19h-10.82v-7.51l9.78-.04c.58,0,1.04.46,1.04,1.04v1.5h9.8-.03Z';
/** "Audio" and "Visuel", letter shapes of the Le Figaro Audiovisuel logo. */
const AUDIO = ['M45.78,89.77l-17.93,54.78-6.7,19.9H0l34.55-96.81h22.86l34.29,96.81h-21.94l-23.97-74.68h0Z', 'M148.37,154.34c-.18.22-.61.88-1.31,1.97s-1.53,2.06-2.5,2.89c-2.93,2.63-5.77,4.42-8.51,5.39-2.74.96-5.94,1.44-9.62,1.44-10.6,0-17.73-3.81-21.41-11.43-2.06-4.2-3.09-10.4-3.09-18.59v-43.15h19.18v43.15c0,4.07.48,7.14,1.44,9.2,1.71,3.64,5.06,5.45,10.05,5.45,6.39,0,10.77-2.58,13.14-7.75,1.23-2.8,1.84-6.5,1.84-11.1v-38.95h18.98v71.59h-18.19v-10.11Z', 'M249.78,67.77v96.68h-18.19v-9.92c-2.67,4.25-5.71,7.33-9.13,9.26s-7.66,2.89-12.74,2.89c-8.36,0-15.4-3.38-21.12-10.15-5.71-6.77-8.57-15.44-8.57-26.04,0-12.22,2.81-21.83,8.44-28.83,5.63-7.01,13.15-10.51,22.56-10.51,4.33,0,8.19.95,11.56,2.86,3.37,1.9,6.11,4.54,8.21,7.91v-34.15h18.98ZM199.34,129.11c0,6.61,1.31,11.89,3.94,15.83,2.58,3.98,6.52,5.98,11.82,5.98s9.33-1.97,12.09-5.91,4.14-9.04,4.14-15.3c0-8.76-2.21-15.02-6.63-18.78-2.71-2.28-5.87-3.42-9.46-3.42-5.47,0-9.49,2.07-12.05,6.21s-3.84,9.27-3.84,15.4h0Z', 'M286.04,67.05v17.27h-18.98v-17.27h18.98ZM286.04,92.86v71.59h-18.98v-71.59h18.98Z', 'M363.87,101.92c6.04,7.58,9.06,16.53,9.06,26.86s-3.02,19.5-9.06,26.96c-6.04,7.47-15.22,11.2-27.52,11.2s-21.48-3.73-27.52-11.2c-6.04-7.46-9.06-16.45-9.06-26.96s3.02-19.29,9.06-26.86c6.04-7.58,15.22-11.36,27.52-11.36s21.48,3.79,27.52,11.36ZM336.28,106.39c-5.47,0-9.69,1.94-12.64,5.81s-4.43,9.4-4.43,16.58,1.48,12.72,4.43,16.62c2.96,3.9,7.17,5.85,12.64,5.85s9.68-1.95,12.61-5.85c2.93-3.9,4.4-9.44,4.4-16.62s-1.47-12.71-4.4-16.58c-2.93-3.88-7.14-5.81-12.61-5.81Z'];
const VISUEL = ['M43.99,270.11h.27l33.69-92.19h6l-36.41,97.38h-6.96L4.3,177.92h6.27l33.41,92.19h.01Z', 'M99.09,191.96v-14.05h5.18v14.05h-5.18ZM99.09,275.29v-70.1h5.18v70.1h-5.18Z', 'M133.86,251.97c.18,3.27.91,6.16,2.18,8.66,1.27,2.5,2.93,4.64,4.98,6.41s4.43,3.11,7.16,4.02c2.73.91,5.68,1.36,8.87,1.36,2.27,0,4.71-.2,7.3-.61,2.59-.41,5-1.16,7.23-2.25s4.07-2.61,5.52-4.57c1.45-1.95,2.18-4.43,2.18-7.43,0-3.91-1.18-6.86-3.55-8.86-2.37-2-5.34-3.55-8.93-4.64s-7.46-1.98-11.59-2.66c-4.14-.68-8-1.7-11.59-3.07-3.59-1.36-6.57-3.29-8.93-5.8-2.37-2.5-3.55-6.16-3.55-10.98,0-3.45.77-6.34,2.32-8.66,1.54-2.32,3.52-4.16,5.93-5.52,2.41-1.36,5.11-2.34,8.12-2.93,3-.59,6-.89,9-.89,3.64,0,7.05.41,10.23,1.23,3.18.82,5.93,2.14,8.25,3.96s4.14,4.14,5.46,6.96,1.98,6.23,1.98,10.23h-5.18c0-3.18-.52-5.91-1.57-8.18-1.05-2.27-2.5-4.11-4.36-5.52s-4.07-2.45-6.61-3.14c-2.55-.68-5.27-1.02-8.18-1.02-2.37,0-4.75.23-7.16.68-2.41.46-4.57,1.23-6.48,2.32s-3.48,2.5-4.71,4.23c-1.23,1.73-1.84,3.82-1.84,6.27,0,2.82.68,5.09,2.05,6.82,1.36,1.73,3.16,3.11,5.39,4.16s4.73,1.89,7.5,2.52c2.77.64,5.57,1.28,8.39,1.91,3.45.64,6.7,1.39,9.75,2.25,3.05.87,5.68,2.05,7.91,3.55,2.23,1.5,3.98,3.43,5.25,5.8,1.27,2.36,1.91,5.37,1.91,9,0,3.82-.86,6.96-2.59,9.41-1.73,2.46-3.91,4.43-6.55,5.93s-5.57,2.55-8.8,3.14c-3.23.59-6.39.89-9.48.89-4,0-7.71-.52-11.12-1.57-3.41-1.05-6.39-2.64-8.93-4.77-2.55-2.13-4.55-4.75-6-7.84-1.46-3.09-2.23-6.68-2.32-10.77h5.18l-.02-.03Z', 'M263.15,275.29h-5.18v-15h-.27c-.91,2.64-2.21,4.98-3.89,7.02-1.68,2.05-3.64,3.8-5.87,5.25-2.23,1.46-4.66,2.55-7.3,3.27-2.64.73-5.32,1.09-8.05,1.09-4.09,0-7.57-.5-10.43-1.5s-5.21-2.32-7.02-3.96c-1.82-1.64-3.25-3.48-4.29-5.52-1.05-2.05-1.8-4.11-2.25-6.2-.46-2.09-.73-4.11-.82-6.07-.09-1.95-.14-3.66-.14-5.11v-43.37h5.18v44.32c0,1.82.14,4.02.41,6.62.27,2.59,1.07,5.09,2.39,7.5,1.32,2.41,3.29,4.48,5.93,6.2,2.64,1.73,6.36,2.59,11.18,2.59,3.91,0,7.43-.73,10.57-2.18,3.14-1.45,5.8-3.48,7.98-6.07s3.84-5.68,4.98-9.27c1.14-3.59,1.71-7.52,1.71-11.8v-37.91h5.18v70.1h0Z', 'M288.49,240.92v.82c0,4.09.55,8,1.64,11.73,1.09,3.73,2.71,7,4.85,9.82s4.83,5.05,8.07,6.68c3.23,1.64,6.99,2.45,11.28,2.45,7.02,0,12.58-1.84,16.68-5.52s6.93-8.8,8.48-15.34h5.18c-1.55,8.18-4.89,14.46-10.02,18.82-5.14,4.36-11.93,6.55-20.39,6.55-5.18,0-9.71-.93-13.57-2.8-3.87-1.86-7.09-4.41-9.68-7.64s-4.52-7.05-5.8-11.46c-1.28-4.41-1.91-9.16-1.91-14.25s.66-9.61,1.98-14.12c1.32-4.5,3.29-8.45,5.93-11.86,2.63-3.41,5.93-6.14,9.89-8.18,3.96-2.05,8.61-3.07,13.98-3.07,4.91,0,9.25.91,13.02,2.73,3.77,1.82,6.93,4.3,9.48,7.43,2.55,3.14,4.48,6.77,5.8,10.91,1.32,4.14,1.98,8.57,1.98,13.3,0,.46-.02.98-.07,1.57-.05.59-.07,1.07-.07,1.43h-56.73ZM340.04,236.42c-.18-3.82-.82-7.43-1.91-10.84s-2.71-6.41-4.85-9-4.74-4.66-7.79-6.21c-3.06-1.55-6.54-2.32-10.46-2.32-4.19,0-7.89.8-11.08,2.39-3.19,1.59-5.88,3.68-8.07,6.27-2.19,2.59-3.88,5.62-5.06,9.07-1.18,3.46-1.92,7-2.19,10.64h51.42,0Z', 'M371.92,177.92v97.38h-5.18v-97.38h5.18Z'];

/**
 * The F for AudioVisuel `height` high: its bar and foot level with the
 * letters' top and baseline, its quill rising above (taller than `height`).
 */
export function FigaroF({ height }: { height: number }) {
  // F units per AudioVisuel unit: the F (bar 12.6 → foot 50.75) over the letters (67.1 → 164.4).
  const s = 38.15 / 97.3;
  // From the quill's top to the AudioVisuel box's bottom (5.1 below its baseline).
  const vh = 50.95 + 5.1 * s;
  const h = (height * vh) / s / 104;
  return (
    <svg className="brand-f" viewBox={`93.8 -0.2 52.3 ${vh}`} width={(h * 52.3) / vh} height={h} aria-hidden="true">
      <path fill="currentColor" d={F} />
    </svg>
  );
}

/** "AudioVisuel", the two words joined on one baseline. */
export function AudioVisuel({ height }: { height: number }) {
  return (
    <svg className="brand-av" viewBox="0 65.5 756 104" width={(height * 756) / 104} height={height} aria-hidden="true">
      <g fill="currentColor">
        {AUDIO.map((d, i) => (
          <path key={i} d={d} />
        ))}
      </g>
      <g fill="currentColor" transform="translate(383 -110.84)">
        {VISUEL.map((d, i) => (
          <path key={i} d={d} />
        ))}
      </g>
    </svg>
  );
}

/** The module of Strata this app is. */
export const MODULE = 'Graphics';

/**
 * « Strata Graphics » with the F AudioVisuel signature, small, under it (top
 * bar). `stacked`: centred and larger (login page).
 */
export function Brand({ stacked = false }: { stacked?: boolean }) {
  // The signature stays well below the name.
  const k = stacked ? 1.15 : 0.62;
  return (
    <div className={`brand ${stacked ? 'stacked' : ''}`} aria-label={`Strata ${MODULE}, Le Figaro AudioVisuel`}>
      <span className="brand-lockup">
        {/* The F with its quill before the name (the top bar): as tall as the name, its quill above. */}
        {!stacked && (
          <svg className="brand-f-lead" viewBox="93.8 -0.2 52.3 52.95" width={23.7} height={24} aria-hidden="true">
            <path fill="currentColor" d={F} />
          </svg>
        )}
        <span className="brand-name">Strata</span>
        <span className="brand-module">{MODULE}</span>
      </span>
      {/* The signature on the login page only: too small in the top bar. */}
      {stacked && (
        <span className="brand-sign">
          <FigaroF height={11 * k} />
          <AudioVisuel height={11 * k} />
        </span>
      )}
    </div>
  );
}
```

## Annexe B — `web/scripts/i18n-check.mjs`

```js
// Checks the French texts of the interface.
import fs from 'node:fs';
import path from 'node:path';
import ts from 'typescript';
const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..', 'src');
const args = process.argv.slice(2);
function files(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...files(p));
    else if (/\.tsx?$/.test(e.name)) out.push(p);
  }
  return out;
}
const parse = (f) => ts.createSourceFile(f, fs.readFileSync(f, 'utf8'), ts.ScriptTarget.Latest, true, f.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
const literal = (n) => (n && (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) ? n.text : null);
const vars = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(',');
const fr = new Map();
for (const f of files(path.join(root, 'i18n', 'fr'))) {
  const visit = (n) => {
    if (ts.isPropertyAssignment(n)) {
      const k = literal(n.name) ?? (ts.isIdentifier(n.name) ? n.name.text : null);
      const v = literal(n.initializer);
      if (k !== null && v !== null) {
        if (fr.has(k) && fr.get(k).value !== v) console.log(`conflict: ${JSON.stringify(k)} in ${path.relative(root, fr.get(k).file)} and ${path.relative(root, f)}`);
        fr.set(k, { value: v, file: f });
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(parse(f));
}
if (args[0] === '--raw') {
  const target = path.resolve(args[1] ?? root);
  const list = fs.statSync(target).isDirectory() ? files(target) : [target];
  const ATTRS = new Set(['title', 'placeholder', 'label', 'aria-label', 'hint', 'confirm', 'message', 'alt']);
  let n = 0;
  for (const f of list) {
    if (f.includes(`${path.sep}i18n${path.sep}`)) continue;
    const sf = parse(f);
    const report = (node, text) => {
      if (!/[A-Za-z]{2}/.test(text)) return;
      const { line } = sf.getLineAndCharacterOfPosition(node.getStart());
      console.log(`${path.relative(root, f)}:${line + 1}: ${text.trim().replace(/\s+/g, ' ').slice(0, 100)}`);
      n++;
    };
    const visit = (node) => {
      if (ts.isJsxText(node) && node.text.trim()) report(node, node.text);
      if (ts.isJsxAttribute(node) && ATTRS.has(node.name.getText()) && node.initializer && ts.isStringLiteral(node.initializer)) report(node, node.initializer.text);
      ts.forEachChild(node, visit);
    };
    visit(sf);
  }
  console.log(`${n} text(s) outside t()`);
  process.exit(0);
}
const used = new Map();
for (const f of files(root)) {
  if (f.includes(`${path.sep}i18n${path.sep}`)) continue;
  const sf = parse(f);
  const visit = (node) => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
      const name = node.expression.text;
      const ctx = name === 'tc' ? literal(node.arguments[0]) : null;
      const texts = name === 't' ? [literal(node.arguments[0])] : name === 'tn' ? [literal(node.arguments[1]), literal(node.arguments[2])] : name === 'tc' || name === 'tl' ? [literal(node.arguments[1])] : [];
      for (const s of texts) {
        if (s === null) continue;
        const { line } = sf.getLineAndCharacterOfPosition(node.getStart());
        const key = ctx !== null && fr.has(`${ctx}|${s}`) ? `${ctx}|${s}` : s;
        if (!used.has(key)) used.set(key, `${path.relative(root, f)}:${line + 1}`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
}
let missing = 0;
for (const [s, at] of used) {
  const e = fr.get(s);
  if (!e) {
    console.log(`missing: ${at}: ${JSON.stringify(s)}`);
    missing++;
  } else if (vars(s) !== vars(e.value)) console.log(`placeholders differ: ${at}: ${JSON.stringify(s)} → ${JSON.stringify(e.value)}`);
}
console.log(`${used.size} texts, ${missing} without French, ${fr.size} French entries`);
process.exit(missing ? 1 : 0);
```
