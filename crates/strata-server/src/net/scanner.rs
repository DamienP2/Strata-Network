//! Scans (by hand and planned) and the monitoring of watched machines.

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use strata_net::scan::{self, Event, Host, Options, Stop};
use strata_net::{cidr, ports};

use super::model::{Address, LastScan};
use super::{File, Net, Store};
use crate::paths::now_ms;

pub struct Run {
    pub stop: Stop,
    pub done: u64,
    pub total: u64,
    pub up: u64,
}

pub fn options(net: &super::model::Network) -> Result<Options, String> {
    let mut o = Options::default();
    if let Some(p) = net.ports.as_deref().filter(|p| !p.trim().is_empty()) {
        o.ports = ports::parse_list(p)?;
    }
    Ok(o)
}

/// Merges what a scan learnt about a machine. Returns the alerts raised.
fn merge(n: &Net, st: &mut Store, h: Host, now: i64) -> Address {
    let mac = h.mac.clone();
    let a = st.address(h.ip);
    let known_mac = a.mac.clone();
    let first = a.first_seen.is_none();
    a.up = true;
    a.via = Some(format!("{:?}", h.via).to_lowercase());
    a.rtt_ms = h.rtt_ms;
    if h.mac.is_some() {
        a.mac = h.mac;
        a.vendor = h.vendor;
    }
    if h.name.is_some() {
        a.name = h.name;
        a.name_source = h.name_source.map(|s| format!("{s:?}").to_lowercase());
    }
    if h.via != scan::Via::Arp || a.ports.is_empty() {
        a.ports = h.ports;
    }
    a.last_seen = Some(now);
    if first {
        a.first_seen = Some(now);
        a.is_new = true;
    }
    let out = a.clone();
    if first {
        let d = [out.name.clone(), out.vendor.clone(), out.mac.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        n.alert(st, "newHost", h.ip, d);
    } else if let (Some(old), Some(new)) = (known_mac, mac) {
        if old != new {
            n.alert(st, "ipConflict", h.ip, format!("{old} → {new}"));
        }
    }
    out
}

impl Net {
    /// Starts scanning network `id` in the background.
    pub fn start_scan(self: &Arc<Self>, id: &str, by: &str) -> Result<(), (u16, String)> {
        let (net, range) = {
            let st = self.store.lock().unwrap();
            let net = st.network(id).cloned().ok_or((404, "no such network".to_string()))?;
            let range = cidr::parse(&net.range).map_err(|e| (400, e))?;
            (net, range)
        };
        let opts = options(&net).map_err(|e| (400, e))?;
        let stop = Stop::default();
        {
            let mut scans = self.scans.lock().unwrap();
            if scans.contains_key(id) {
                return Err((409, "a scan is already running".into()));
            }
            scans.insert(id.to_string(), Run { stop: stop.clone(), done: 0, total: range.len(), up: 0 });
        }
        tracing::info!("{by} scans {} ({})", net.name, net.range);
        let me = self.clone();
        let (id, by) = (id.to_string(), by.to_string());
        tokio::spawn(async move { me.scan(id, by, range, opts, stop).await });
        Ok(())
    }

    pub fn stop_scan(&self, id: &str) -> bool {
        self.scans.lock().unwrap().get(id).map(|r| r.stop.stop()).is_some()
    }

    async fn scan(self: Arc<Self>, id: String, by: String, range: cidr::Range, opts: Options, stop: Stop) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(scan::run(range, opts, tx, stop));
        let mut found: HashSet<Ipv4Addr> = HashSet::new();
        let mut last_progress = Instant::now() - Duration::from_secs(1);
        let progress = |n: &Net, id: &str, running: bool| {
            let scans = n.scans.lock().unwrap();
            if let Some(r) = scans.get(id) {
                n.send(&serde_json::json!({"type": "scan", "networkId": id, "running": running, "done": r.done, "total": r.total, "up": r.up}));
            }
        };
        while let Some(e) = rx.recv().await {
            match e {
                Event::Host(h) => {
                    found.insert(h.ip);
                    let a = {
                        let mut st = self.store.lock().unwrap();
                        merge(&self, &mut st, h, now_ms())
                    };
                    if let Some(r) = self.scans.lock().unwrap().get_mut(&id) {
                        r.up += 1;
                    }
                    self.send(&serde_json::json!({"type": "address", "networkId": id, "address": a}));
                }
                Event::Progress { done, total } => {
                    if let Some(r) = self.scans.lock().unwrap().get_mut(&id) {
                        r.done = done;
                        r.total = total;
                    }
                    if last_progress.elapsed() >= Duration::from_millis(250) {
                        last_progress = Instant::now();
                        progress(&self, &id, true);
                    }
                }
                Event::Done { secs, stopped, .. } => {
                    let now = now_ms();
                    let arp = self.snmp_arp();
                    let mut st = self.store.lock().unwrap();
                    // Machines only a router knows (SNMP ARP table).
                    if !stopped {
                        let quiet: Vec<(Ipv4Addr, String)> = arp.into_iter().filter(|(ip, _)| range.contains(*ip) && !found.contains(ip)).collect();
                        for (ip, mac) in quiet {
                            let h = Host {
                                ip,
                                via: scan::Via::Arp,
                                rtt_ms: None,
                                vendor: strata_net::mac::parse(&mac).and_then(|m| strata_net::mac::vendor(&m)).map(Into::into),
                                mac: Some(mac.clone()),
                                name: None,
                                name_source: None,
                                ports: vec![],
                            };
                            merge(&self, &mut st, h, now);
                            st.address(ip).via = Some("snmp".into());
                            found.insert(ip);
                        }
                    }
                    let mut gone = Vec::new();
                    if !stopped {
                        for a in st.addresses.values_mut().filter(|a| range.contains(a.ip) && !found.contains(&a.ip)) {
                            if a.up {
                                gone.push((a.ip, a.label.clone()));
                            }
                            a.up = false;
                            a.rtt_ms = None;
                        }
                    }
                    for (ip, label) in gone {
                        self.alert(&mut st, "hostGone", ip, label);
                    }
                    self.check_vlans(&mut st);
                    let up = found.len() as u64;
                    if let Some(n) = st.networks.iter_mut().find(|n| n.id == id) {
                        n.last_scan = Some(LastScan { at: now, secs, up, stopped });
                    }
                    st.prune();
                    let name = st.network(&id).map(|n| n.name.clone()).unwrap_or_default();
                    drop(st);
                    if let Some(r) = self.scans.lock().unwrap().get_mut(&id) {
                        r.up = up;
                    }
                    progress(&self, &id, false);
                    self.scans.lock().unwrap().remove(&id);
                    self.save(&[File::Addresses, File::Networks, File::Alerts]);
                    self.changed("addresses");
                    self.changed("networks");
                    self.changed("alerts");
                    self.log(&by, if stopped { "stopped scan" } else { "scanned" }, &name, &format!("{up} up · {secs:.0} s"));
                }
            }
        }
        // The engine ended without Done (should not happen): never leave a scan "running".
        if self.scans.lock().unwrap().remove(&id).is_some() {
            progress(&self, &id, false);
        }
    }

    /// Scans one address now and returns it.
    pub async fn scan_one(self: &Arc<Self>, ip: Ipv4Addr, by: &str) -> Address {
        let opts = {
            let st = self.store.lock().unwrap();
            st.network_of(ip).and_then(|n| options(n).ok()).unwrap_or_default()
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(scan::run(cidr::Range::single(ip), opts, tx, Stop::default()));
        let mut host = None;
        while let Some(e) = rx.recv().await {
            if let Event::Host(h) = e {
                host = Some(h);
            }
        }
        let a = {
            let mut st = self.store.lock().unwrap();
            match host {
                Some(h) => merge(self, &mut st, h, now_ms()),
                None => {
                    let a = st.address(ip);
                    a.up = false;
                    a.rtt_ms = None;
                    a.clone()
                }
            }
        };
        self.save(&[File::Addresses, File::Alerts]);
        self.changed("addresses");
        self.log(by, "scanned", &ip.to_string(), if a.up { "up" } else { "no answer" });
        a
    }
}

/// Planned scans: every 30 s, starts the networks whose time has come.
pub fn spawn_scheduler(net: Arc<Net>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.tick().await;
        loop {
            tick.tick().await;
            let now = now_ms();
            let due: Vec<String> = {
                let st = net.store.lock().unwrap();
                st.networks
                    .iter()
                    .filter(|n| n.schedule_mins > 0 && n.last_scan.as_ref().is_none_or(|l| now - l.at >= i64::from(n.schedule_mins) * 60_000))
                    .map(|n| n.id.clone())
                    .collect()
            };
            for id in due {
                if !net.scans.lock().unwrap().contains_key(&id) {
                    let _ = net.start_scan(&id, "scheduler");
                }
            }
        }
    });
}

/// Monitoring: every minute, pings the watched machines (TCP on their open ports
/// when ICMP is not allowed or blocked). Two failures in a row: `watchDown`.
pub fn spawn_monitor(net: Arc<Net>) {
    tokio::spawn(async move {
        let pinger = strata_net::ping::Pinger::new().map(Arc::new);
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let watched: Vec<(Ipv4Addr, Vec<u16>)> = {
                let st = net.store.lock().unwrap();
                st.addresses.values().filter(|a| a.watched).map(|a| (a.ip, a.ports.iter().map(|p| p.port).collect())).collect()
            };
            if watched.is_empty() {
                continue;
            }
            let checks = watched.into_iter().map(|(ip, ports)| {
                let pinger = pinger.clone();
                async move {
                    let t = Duration::from_millis(1000);
                    if let Some(p) = &pinger {
                        if let Some(rtt) = p.ping(ip, t, 2).await {
                            return (ip, Some(rtt.as_secs_f32() * 1000.0));
                        }
                    }
                    for port in ports.into_iter().take(4) {
                        let start = Instant::now();
                        if ports::connect(ip, port, t).await != ports::Probe::Silent {
                            return (ip, Some(start.elapsed().as_secs_f32() * 1000.0));
                        }
                    }
                    (ip, None)
                }
            });
            let results = futures_util::future::join_all(checks).await;
            let mut alerted = false;
            {
                let mut st = net.store.lock().unwrap();
                let mut fails = net.watch.lock().unwrap();
                let now = now_ms();
                for (ip, rtt) in results {
                    let key = u32::from(ip);
                    let count = fails.entry(key).or_default();
                    let a = st.address(ip);
                    match rtt {
                        Some(ms) => {
                            let was_down = *count >= 2;
                            *count = 0;
                            a.up = true;
                            a.rtt_ms = Some((ms * 10.0).round() / 10.0);
                            a.last_seen = Some(now);
                            if was_down {
                                let label = a.label.clone();
                                alerted |= net.alert(&mut st, "watchUp", ip, label);
                            }
                        }
                        None => {
                            *count += 1;
                            if *count == 2 {
                                a.up = false;
                                a.rtt_ms = None;
                                let label = a.label.clone();
                                alerted |= net.alert(&mut st, "watchDown", ip, label);
                            }
                        }
                    }
                }
            }
            net.save(&[File::Addresses, File::Alerts]);
            net.changed("addresses");
            if alerted {
                net.changed("alerts");
            }
        }
    });
}
