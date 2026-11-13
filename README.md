# Strata Network

Gestion d’un réseau informatique : scan des plages IP (machines, noms, MAC, ports, pages web), VLAN, façades de
switchs étiquetées à la main, flux multicast, topologie en arbre, alertes et historique. Un seul exécutable Rust
(`strata-network`) qui embarque l’interface web.

## Installer

Les paquets sont publiés dans les [releases](https://github.com/DamienP2/Strata-Network/releases).

**Linux** (x86_64 ou arm64) :

```bash
tar xzf Strata-<version>-linux-x86_64.tar.gz
```

```bash
sudo ./Strata-<version>-linux-x86_64/install.sh
```

Le service systemd `strata-network` écoute sur le port 8080. Ouvrez `http://localhost:8080/` sur le serveur
pour créer le compte administrateur, ou lancez `sudo strata-network user add admin`.

**Windows** : lancez `Strata-<version>-windows-x86_64-setup.exe` (service Windows, lanceur dans la zone de
notification, règle du pare-feu).

Pour que le scan trouve toutes les machines, même celles qui ne répondent ni au ping ni sur leurs ports, le
serveur doit avoir une adresse dans chaque VLAN scanné (ARP), ou lire la table ARP du routeur en SNMP
(Système › SNMP).

## Développer

```bash
cd web && npm install && npm run build
```

```bash
cargo run --release -p strata-server -- --data-dir data
```

Puis http://localhost:8080. Pour l’interface en direct : `cd web && npm run dev` (port 5173, relaie vers 8080).

Vérifications avant livraison :

```bash
cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

```bash
cd web && npm run build && node scripts/i18n-check.mjs
```

## Publier une version

```bash
git tag v0.2.0 && git push origin v0.2.0
```

Le workflow `Release` construit l’interface, les binaires Linux (musl, x86_64 et arm64) et Windows, les paquets
`Strata-<version>-<plateforme>.tar.gz` / `.zip` (ceux que les serveurs installés téléchargent pour se mettre à
jour) et l’installateur Windows, puis crée la release GitHub.

## Organisation

| Dossier | Contenu |
| --- | --- |
| `crates/strata-net` | moteur réseau : plages, ping/TCP/ARP, noms (DNS, mDNS, NetBIOS), constructeurs MAC, détection web, SNMP v2c en lecture seule, multicast passif, services mDNS |
| `crates/strata-server` | serveur axum : connexion, utilisateurs, système, mises à jour, et `src/net/` (inventaire, scans, alertes, API réseau) |
| `crates/strata-tray` | lanceur de bureau (Windows, macOS) |
| `web` | interface React + TypeScript + Vite |
| `docs/design.md` | règles de design reprises de l’application de référence |
| `docs/api-network.md` | contrat de l’API réseau |
| `packaging`, `scripts`, `.github/workflows` | installateurs, version, CI et release |
