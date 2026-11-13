//! The network features: inventory (networks, addresses, VLANs, switches),
//! scans and planned scans, monitoring, read-only SNMP, multicast, mDNS
//! services, alerts and history. Lives across hot restarts; talks to the
//! clients of the current cycle.

use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, Weak};

use serde::Serialize;
use tokio::sync::mpsc;

use crate::paths::{new_id, now_ms, read_json};
use crate::state::{AppState, Audience};

pub mod api;
pub mod model;
mod multicast;
mod scanner;
mod snmp;

use model::*;

/// The files of `<data>/network/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum File {
    Networks,
    Addresses,
    Vlans,
    Switches,
    Snmp,
    Multicast,
    Alerts,
    History,
}

impl File {
    fn name(self) -> &'static str {
        match self {
            File::Networks => "networks.json",
            File::Addresses => "addresses.json",
            File::Vlans => "vlans.json",
            File::Switches => "switches.json",
            File::Snmp => "snmp.json",
            File::Multicast => "multicast.json",
            File::Alerts => "alerts.json",
            File::History => "history.json",
        }
    }
}

const MAX_ALERTS: usize = 500;
const HISTORY_DAYS: i64 = 366;

#[derive(Default)]
pub struct Store {
    pub networks: Vec<Network>,
    pub addresses: BTreeMap<u32, Address>,
    pub vlans: BTreeMap<u16, Vlan>,
    pub switches: Vec<Switch>,
    pub snmp: Vec<SnmpAgent>,
    pub multicast: McastSettings,
    pub alerts: Vec<Alert>,
    pub history: Vec<HistoryEntry>,
}

impl Store {
    pub fn address(&mut self, ip: Ipv4Addr) -> &mut Address {
        self.addresses.entry(u32::from(ip)).or_insert_with(|| Address::new(ip))
    }

    pub fn network(&self, id: &str) -> Option<&Network> {
        self.networks.iter().find(|n| n.id == id)
    }

    /// Drops addresses that hold nothing.
    pub fn prune(&mut self) {
        self.addresses.retain(|_, a| !a.is_blank());
    }

    /// The network whose range holds `ip`.
    pub fn network_of(&self, ip: Ipv4Addr) -> Option<&Network> {
        self.networks.iter().find(|n| strata_net::cidr::parse(&n.range).is_ok_and(|r| r.contains(ip)))
    }
}

pub struct Net {
    dir: PathBuf,
    pub store: Mutex<Store>,
    app: RwLock<Weak<AppState>>,
    writer: mpsc::UnboundedSender<(PathBuf, Vec<u8>)>,
    pub scans: Mutex<HashMap<String, scanner::Run>>,
    pub mcast: Mutex<Option<strata_net::mcast::Engine>>,
    pub services: Mutex<(i64, Vec<strata_net::mdns::Service>)>,
    pub snmp_cache: Mutex<snmp::Cache>,
    pub watch: Mutex<HashMap<u32, u32>>,
}

fn load<T: serde::de::DeserializeOwned + Default>(dir: &std::path::Path, f: File) -> T {
    match read_json(&dir.join(f.name())) {
        Ok(v) => v.unwrap_or_default(),
        Err(e) => {
            tracing::error!("cannot read {}: {e}; starting empty", f.name());
            T::default()
        }
    }
}

impl Net {
    /// Loads the inventory and starts the background work (planned scans,
    /// monitoring, SNMP polling, multicast listening, mDNS). Call once.
    pub fn start(data_dir: &std::path::Path) -> Arc<Net> {
        let dir = data_dir.join("network");
        let _ = std::fs::create_dir_all(&dir);
        let addresses: Vec<Address> = load(&dir, File::Addresses);
        let vlans: Vec<Vlan> = load(&dir, File::Vlans);
        let store = Store {
            networks: load(&dir, File::Networks),
            addresses: addresses.into_iter().map(|a| (u32::from(a.ip), a)).collect(),
            vlans: vlans.into_iter().map(|v| (v.id, v)).collect(),
            switches: load(&dir, File::Switches),
            snmp: load(&dir, File::Snmp),
            multicast: load(&dir, File::Multicast),
            alerts: load(&dir, File::Alerts),
            history: load(&dir, File::History),
        };
        // One writer thread: files are written in the order they were changed.
        let (writer, mut rx) = mpsc::unbounded_channel::<(PathBuf, Vec<u8>)>();
        std::thread::Builder::new()
            .name("net-writer".into())
            .spawn(move || {
                while let Some((path, bytes)) = rx.blocking_recv() {
                    if let Err(e) = crate::paths::write_atomic(&path, &bytes, path.ends_with(File::Snmp.name())) {
                        tracing::error!("cannot write {}: {e}", path.display());
                    }
                }
            })
            .expect("writer thread");
        #[cfg(unix)]
        strata_net::raise_file_limit();
        let net = Arc::new(Net {
            dir,
            store: Mutex::new(store),
            app: RwLock::new(Weak::new()),
            writer,
            scans: Mutex::default(),
            mcast: Mutex::default(),
            services: Mutex::default(),
            snmp_cache: Mutex::default(),
            watch: Mutex::default(),
        });
        scanner::spawn_scheduler(net.clone());
        scanner::spawn_monitor(net.clone());
        snmp::spawn_poller(net.clone());
        multicast::start(&net);
        multicast::spawn_services(net.clone());
        net
    }

    /// The server cycle whose clients receive the events.
    pub fn attach(&self, app: &Arc<AppState>) {
        *self.app.write().unwrap() = Arc::downgrade(app);
    }

    pub fn send<T: Serialize>(&self, msg: &T) {
        if let Some(app) = self.app.read().unwrap().upgrade() {
            app.events.send(Audience::All, msg);
        }
    }

    pub fn changed(&self, what: &str) {
        if let Some(app) = self.app.read().unwrap().upgrade() {
            app.changed(what);
        }
    }

    /// Writes the files (from the store as it is now).
    pub fn save(&self, files: &[File]) {
        let st = self.store.lock().unwrap();
        for f in files {
            let bytes = match f {
                File::Networks => serde_json::to_vec_pretty(&st.networks),
                File::Addresses => serde_json::to_vec_pretty(&st.addresses.values().collect::<Vec<_>>()),
                File::Vlans => serde_json::to_vec_pretty(&st.vlans.values().collect::<Vec<_>>()),
                File::Switches => serde_json::to_vec_pretty(&st.switches),
                File::Snmp => serde_json::to_vec_pretty(&st.snmp),
                File::Multicast => serde_json::to_vec_pretty(&st.multicast),
                File::Alerts => serde_json::to_vec_pretty(&st.alerts),
                File::History => serde_json::to_vec_pretty(&st.history),
            };
            match bytes {
                Ok(b) => {
                    let _ = self.writer.send((self.dir.join(f.name()), b));
                }
                Err(e) => tracing::error!("cannot serialize {}: {e}", f.name()),
            }
        }
    }

    /// Writes one entry of the history.
    pub fn log(&self, user: &str, action: &str, target: &str, detail: &str) {
        {
            let mut st = self.store.lock().unwrap();
            let now = now_ms();
            st.history.push(HistoryEntry { at: now, user: user.into(), action: action.into(), target: target.into(), detail: detail.into() });
            let cutoff = now - HISTORY_DAYS * 86_400_000;
            if st.history.first().is_some_and(|h| h.at < cutoff) {
                st.history.retain(|h| h.at >= cutoff);
            }
        }
        self.save(&[File::History]);
        self.changed("history");
    }

    /// Raises an alert, unless the same one is already waiting unseen.
    pub fn alert(&self, st: &mut Store, kind: &str, ip: Ipv4Addr, detail: String) -> bool {
        if st.alerts.iter().any(|a| !a.seen && a.kind == kind && a.ip == ip && a.detail == detail) {
            return false;
        }
        let network_id = st.network_of(ip).map(|n| n.id.clone());
        st.alerts.insert(0, Alert { id: new_id(), at: now_ms(), kind: kind.into(), ip, network_id, detail, seen: false });
        st.alerts.truncate(MAX_ALERTS);
        true
    }

    /// `vlanMismatch` alerts: machines plugged on a port whose VLAN subnet does not hold them.
    pub fn check_vlans(&self, st: &mut Store) -> bool {
        let mut raised = false;
        let found: Vec<(Ipv4Addr, String)> = st
            .switches
            .iter()
            .flat_map(|s| s.ports.iter().map(move |p| (s, p)))
            .filter_map(|(s, p)| {
                let ip = p.host?;
                let vlan = st.vlans.get(&p.vlan?)?;
                let (net, prefix) = strata_net::cidr::parse_block(vlan.subnet.as_deref()?).ok()?;
                (!strata_net::cidr::in_block(ip, net, prefix)).then(|| (ip, format!("{} {} · VLAN {} {}", s.name, p.name, vlan.id, vlan.subnet.as_deref().unwrap_or(""))))
            })
            .collect();
        for (ip, detail) in found {
            raised |= self.alert(st, "vlanMismatch", ip, detail);
        }
        raised
    }

    pub fn snapshot_scans(&self) -> serde_json::Value {
        let scans = self.scans.lock().unwrap();
        serde_json::Value::Object(scans.iter().map(|(id, r)| (id.clone(), serde_json::json!({"done": r.done, "total": r.total, "up": r.up}))).collect())
    }
}

/// The local IPv4 interfaces (name, address), loopback left out.
pub fn interfaces() -> Vec<(String, Ipv4Addr)> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_loopback() => Some((i.name, ip)),
            _ => None,
        })
        .collect()
}
