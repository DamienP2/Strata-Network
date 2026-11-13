//! Read-only SNMP polling (every 5 minutes, or on demand): routers' ARP tables
//! and multicast routes, switches' interface names, MAC tables and LLDP.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use strata_net::snmp::{self as s, Agent, Session};

use super::model::{Lldp, Poll, PortRef, SnmpAgent};
use super::{File, Net};
use crate::paths::now_ms;

#[derive(Default)]
pub struct Cache {
    /// ARP tables of the agents that read them: agent id → ip → MAC.
    arp: HashMap<String, HashMap<Ipv4Addr, String>>,
    /// Multicast route counters at the last poll, for rates.
    routes: HashMap<(String, Ipv4Addr, Ipv4Addr), (u64, u64, Instant)>,
}

const TIMEOUT: Duration = Duration::from_secs(2);

impl Net {
    /// Every MAC known by a router's ARP table.
    pub fn snmp_arp(&self) -> HashMap<Ipv4Addr, String> {
        let c = self.snmp_cache.lock().unwrap();
        c.arp.values().flat_map(|t| t.iter().map(|(ip, m)| (*ip, m.clone()))).collect()
    }

    /// Reads an agent now; records the result in `lastPoll`.
    pub async fn poll(self: &Arc<Self>, id: &str) -> Option<SnmpAgent> {
        let agent = self.store.lock().unwrap().snmp.iter().find(|a| a.id == id).cloned()?;
        let result = self.poll_agent(&agent).await;
        let poll = match result {
            Ok(msg) => Poll { at: now_ms(), ok: true, message: msg },
            Err(e) => {
                tracing::warn!("snmp {} ({}): {e}", agent.name, agent.ip);
                Poll { at: now_ms(), ok: false, message: e }
            }
        };
        let out = {
            let mut st = self.store.lock().unwrap();
            let a = st.snmp.iter_mut().find(|a| a.id == id)?;
            a.last_poll = Some(poll);
            a.clone()
        };
        self.save(&[File::Snmp, File::Switches, File::Addresses]);
        self.changed("snmp");
        Some(out)
    }

    async fn poll_agent(self: &Arc<Self>, a: &SnmpAgent) -> Result<String, String> {
        let session = Session::open(&Agent { ip: a.ip, port: a.port, community: a.community.clone() }, TIMEOUT).await.map_err(|e| e.to_string())?;
        let id = s::identity(&session).await.map_err(|e| e.to_string())?;
        let mut said = vec![id.name.clone()];
        if a.read_arp {
            let t = s::arp(&session).await.map_err(|e| e.to_string())?;
            said.push(format!("{} arp", t.len()));
            let t = t.into_iter().map(|(ip, m)| (ip, strata_net::mac::format(&m))).collect();
            self.snmp_cache.lock().unwrap().arp.insert(a.id.clone(), t);
        } else {
            self.snmp_cache.lock().unwrap().arp.remove(&a.id);
        }
        if a.read_multicast {
            let routes = s::mroutes(&session).await.map_err(|e| e.to_string())?;
            said.push(format!("{} routes", routes.len()));
            self.report_routes(&a.id, routes);
        }
        if a.read_switch {
            let names = s::interfaces(&session).await.map_err(|e| e.to_string())?;
            let macs = s::mac_table(&session).await.unwrap_or_default();
            let lldp = s::lldp(&session).await.unwrap_or_default();
            said.push(format!("{} mac", macs.len()));
            self.apply_switch(&a.id, &names, &macs, &lldp);
        }
        Ok(said.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "))
    }

    fn report_routes(&self, agent: &str, routes: Vec<s::Route>) {
        let now = Instant::now();
        let mut out = Vec::new();
        {
            let mut c = self.snmp_cache.lock().unwrap();
            for r in routes {
                let key = (agent.to_string(), r.group, r.source);
                let rate = c.routes.get(&key).and_then(|(o, p, t)| {
                    let secs = now.duration_since(*t).as_secs_f64();
                    (secs > 1.0 && r.octets >= *o).then(|| ((r.octets - o) as f64 * 8.0 / secs, r.packets.saturating_sub(*p) as f64 / secs))
                });
                c.routes.insert(key, (r.octets, r.packets, now));
                out.push((r.group, r.source, rate));
            }
        }
        if let Some(e) = self.mcast.lock().unwrap().as_ref() {
            for (g, src, rate) in out {
                e.report_routed(g, src, rate.map(|r| r.0), rate.map(|r| r.1));
            }
        }
        self.changed("multicast");
    }

    /// Puts what a switch says (MACs, LLDP) on the ports of the switches read through this agent.
    fn apply_switch(&self, agent: &str, names: &HashMap<u32, String>, macs: &[s::Learnt], lldp: &[s::Neighbour]) {
        let mut st = self.store.lock().unwrap();
        let by_mac: HashMap<String, Ipv4Addr> = st.addresses.values().filter_map(|a| Some((a.mac.clone()?, a.ip))).collect();
        let mut plug: Vec<(Ipv4Addr, PortRef)> = Vec::new();
        for sw in st.switches.iter_mut().filter(|s| s.snmp.as_deref() == Some(agent)) {
            let sw_id = sw.id.clone();
            for p in sw.ports.iter_mut() {
                let ifs: Vec<u32> = names.iter().filter(|(_, n)| same_port(n, &p.name)).map(|(i, _)| *i).collect();
                let mut learnt: Vec<String> = macs.iter().filter(|m| ifs.contains(&m.if_index)).map(|m| m.mac.clone()).collect();
                learnt.sort();
                learnt.dedup();
                p.lldp = lldp.iter().find(|n| same_port(&n.local_port, &p.name)).map(|n| Lldp { name: n.name.clone(), port: n.port.clone() });
                if learnt.len() == 1 && p.mode == "access" {
                    if let Some(ip) = by_mac.get(&learnt[0]) {
                        if p.host.is_none() {
                            p.host = Some(*ip);
                        }
                        plug.push((*ip, PortRef { id: sw_id.clone(), port: p.n }));
                    }
                }
                p.learnt = learnt;
            }
        }
        for (ip, at) in plug {
            let a = st.address(ip);
            if a.switch.is_none() {
                a.switch = Some(at);
            }
        }
        drop(st);
        self.changed("switches");
        self.changed("addresses");
    }
}

/// `GigabitEthernet1/0/12` and `Gi1/0/12` are the same port: same first letter, same numbers.
pub fn same_port(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    let digits = |s: &str| s.find(|c: char| c.is_ascii_digit()).map(|i| s[i..].to_string());
    let first = |s: &str| s.chars().next().map(|c| c.to_ascii_lowercase());
    match (digits(a), digits(b)) {
        (Some(x), Some(y)) => x == y && first(a) == first(b),
        _ => false,
    }
}

/// Polls every agent every 5 minutes.
pub fn spawn_poller(net: Arc<Net>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            let ids: Vec<String> = net.store.lock().unwrap().snmp.iter().map(|a| a.id.clone()).collect();
            for id in ids {
                net.poll(&id).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::same_port;

    #[test]
    fn port_names() {
        assert!(same_port("GigabitEthernet1/0/12", "Gi1/0/12"));
        assert!(same_port("Te1/1/1", "TenGigabitEthernet1/1/1"));
        assert!(!same_port("Gi1/0/12", "Gi1/0/1"));
        assert!(!same_port("Gi1/0/1", "Te1/0/1"));
        assert!(same_port("ge-0/0/3", "GE-0/0/3"));
    }
}
