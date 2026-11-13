//! The multicast engine of the chosen interface (passive), and mDNS services.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use strata_net::mcast::Engine;

use super::{interfaces, Net};
use crate::paths::now_ms;

/// The address of the interface named in the settings, else the first one.
pub fn iface_ip(name: &str) -> Option<Ipv4Addr> {
    let list = interfaces();
    list.iter().find(|(n, ip)| n == name || ip.to_string() == name).or_else(|| list.first()).map(|(_, ip)| *ip)
}

/// (Re)starts listening on the interface of the settings.
pub fn start(net: &Arc<Net>) {
    let name = net.store.lock().unwrap().multicast.iface.clone();
    let mut slot = net.mcast.lock().unwrap();
    if let Some(old) = slot.take() {
        old.close();
    }
    match iface_ip(&name) {
        Some(ip) => {
            tracing::info!("multicast: listening to SAP and PTP on {ip}");
            *slot = Some(Engine::start(ip));
        }
        None => tracing::warn!("multicast: no network interface"),
    }
    drop(slot);
    static WATCHING: std::sync::Once = std::sync::Once::new();
    let n = net.clone();
    // Tells the clients when the streams change (at most every 2 s).
    WATCHING.call_once(|| {
        tokio::spawn(async move {
            let mut last = (0usize, 0u64);
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let now = n.mcast.lock().unwrap().as_ref().map(|e| {
                    let s = e.snapshot();
                    (s.len(), s.iter().map(|x| x.last_seen).max().unwrap_or(0))
                });
                if let Some(now) = now {
                    if now != last {
                        last = now;
                        n.changed("multicast");
                    }
                }
            }
        });
    });
}

impl Net {
    /// Asks the network for its services (mDNS) and keeps the answers.
    pub async fn browse(self: &Arc<Self>) {
        let ip = {
            let name = self.store.lock().unwrap().multicast.iface.clone();
            iface_ip(&name)
        };
        let Some(ip) = ip else { return };
        match strata_net::mdns::browse(ip, Duration::from_secs(3)).await {
            Ok(list) => {
                *self.services.lock().unwrap() = (now_ms(), list);
                self.changed("services");
            }
            Err(e) => tracing::warn!("mdns: {e}"),
        }
    }
}

/// Looks for services every 10 minutes.
pub fn spawn_services(net: Arc<Net>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            net.browse().await;
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });
}
