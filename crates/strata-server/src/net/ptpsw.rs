//! PTP as the switches tell it (CISCO-PTP-MIB, read-only SNMP) and as this
//! machine hears it (passive Announce): each online switch's clock and PTP
//! ports with the activity of the device behind each master port (its
//! Delay_Req), the switch tab's view, and for the multicast PTP view the
//! grandmaster told apart from the clock that talks to this machine (a
//! boundary clock: a switch's master port), the switches' synchronisation and
//! the clock tree.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::Ipv4Addr;

use serde::Serialize;
use strata_net::ptp;
use strata_net::ptpmib::{ClockType, PortState, PtpClockMib};
use strata_net::switchinfo as si;

use super::info::{Ifs, PortV};
use super::model::Switch;

/// Time between two PTP reads of a switch that has PTP.
pub const EVERY_SECS: u64 = 15;
/// Offsets from the master kept for the correction sparkline (15 min at one every 15 s).
pub const CORRECTIONS_KEPT: usize = 60;

type ClockKey = (u8, u32, u32);
type PortKey = (u8, u32, u32, u32);

fn clock_key(c: &PtpClockMib) -> ClockKey {
    let t = match c.clock_type {
        ClockType::Ordinary => 1,
        ClockType::Boundary => 2,
        ClockType::Transparent => 3,
        ClockType::BoundaryNode => 4,
        ClockType::Other(x) => x,
    };
    (c.domain, t, c.instance)
}

// ── What is remembered between reads ──

/// The packet counters of one PTP port between two reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    rx: Option<u64>,
    tx: Option<u64>,
    at: i64,
    /// Packets a second between the last two reads (`None`: not known yet, or a counter went back).
    pub rx_ps: Option<f64>,
    pub tx_ps: Option<f64>,
    /// When the received counter was last seen increasing (unix ms, the read that saw it).
    pub last_rx: Option<i64>,
    /// It increased at least once since it is watched.
    pub had_rx: bool,
}

impl Track {
    /// A new reading of the counters at `now` (unix ms).
    pub fn update(&mut self, rx: Option<u64>, tx: Option<u64>, now: i64) {
        let secs = (now - self.at) as f64 / 1000.0;
        let rate = |prev: Option<u64>, cur: Option<u64>| match (prev, cur) {
            (Some(p), Some(c)) if c >= p && secs > 0.0 => Some((c - p) as f64 / secs),
            _ => None,
        };
        self.rx_ps = rate(self.rx, rx);
        self.tx_ps = rate(self.tx, tx);
        if let (Some(p), Some(c)) = (self.rx, rx) {
            if c > p {
                self.last_rx = Some(now);
                self.had_rx = true;
            }
        }
        self.rx = rx;
        self.tx = tx;
        self.at = now;
    }

    /// The device behind a master port: asking (Delay_Req received), silent since it
    /// asked (`mute`), never asked (`none`); `None` before two reads.
    pub fn activity(&self) -> Option<&'static str> {
        match self.rx_ps? {
            r if r > 0.0 => Some("active"),
            _ if self.had_rx => Some("mute"),
            _ => Some("none"),
        }
    }
}

/// The PTP reads of one online switch (in memory only).
#[derive(Default)]
pub struct PtpMem {
    pub clocks: Vec<PtpClockMib>,
    /// Last successful read (unix ms).
    pub at: i64,
    /// `None` before the first read; `Some(false)`: the switch has no PTP clock (or no CISCO-PTP-MIB).
    pub present: Option<bool>,
    pub error: Option<String>,
    tracks: HashMap<PortKey, Track>,
    /// Offsets from the master of each clock: (unix ms, ns), oldest first.
    history: HashMap<ClockKey, VecDeque<(i64, f64)>>,
}

impl PtpMem {
    /// A new read at `now` (unix ms).
    pub fn update(&mut self, clocks: Vec<PtpClockMib>, now: i64) {
        let mut keys = HashSet::new();
        let mut clock_keys = HashSet::new();
        for c in &clocks {
            let ck = clock_key(c);
            clock_keys.insert(ck);
            if let Some(o) = c.offset_ns {
                let h = self.history.entry(ck).or_default();
                h.push_back((now, o));
                while h.len() > CORRECTIONS_KEPT {
                    h.pop_front();
                }
            }
            for p in &c.ports {
                let k = (ck.0, ck.1, ck.2, p.number);
                keys.insert(k);
                self.tracks.entry(k).or_default().update(p.rx, p.tx, now);
            }
        }
        self.tracks.retain(|k, _| keys.contains(k));
        self.history.retain(|k, _| clock_keys.contains(k));
        self.present = Some(!clocks.is_empty());
        self.clocks = clocks;
        self.at = now;
        self.error = None;
    }

    fn track(&self, c: &PtpClockMib, number: u32) -> Option<&Track> {
        let k = clock_key(c);
        self.tracks.get(&(k.0, k.1, k.2, number))
    }

    fn history(&self, c: &PtpClockMib) -> Vec<(i64, f64)> {
        self.history.get(&clock_key(c)).map(|h| h.iter().copied().collect()).unwrap_or_default()
    }

    /// The main clock: the one with a slave port, else the one with the most ports.
    fn main(&self) -> Option<&PtpClockMib> {
        self.clocks.iter().find(|c| c.slave().is_some()).or_else(|| self.clocks.iter().max_by_key(|c| c.ports.len()))
    }
}

// ── The switch's view ──

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GmV {
    pub identity: String,
    /// Inventory name of the address whose MAC the identity holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<Ipv4Addr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority1: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority2: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_class: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_accuracy: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy_ns: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtpPortV {
    #[serde(flatten)]
    pub at: PortV,
    /// PTP port number.
    pub number: u32,
    pub domain: u8,
    /// initializing, faulty, disabled, listening, preMaster, master, passive, uncalibrated, slave, unknown.
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_announce: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_sync: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_delay_req: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub announce_timeout: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_mech: Option<&'static str>,
    /// PTP packets a second (all kinds) received and sent, between the last two reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_ps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_ps: Option<f64>,
    /// Master ports: active (Delay_Req received), none (never), mute (stopped).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<&'static str>,
    /// Mute: when Delay_Req were last seen (unix ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_since: Option<i64>,
    /// What is plugged: another switch ("SW-Core · Eth1/1"), the inventory host, the LLDP neighbour, the port label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_ip: Option<Ipv4Addr>,
    /// The port leads to this inventory switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_switch: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtpSwitchV {
    /// Last read (unix ms).
    pub at: i64,
    pub domain: u8,
    /// ordinary, boundary, transparent, boundaryNode, other.
    pub clock_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub two_step: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps_removed: Option<u16>,
    /// Current DS: offset from the master (the correction) and mean path delay, ns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_ns: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_path_delay_ns: Option<f64>,
    /// The master this clock follows: its clock identity and port number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gm: Option<GmV>,
    /// freerun, holdover, acquiring, frequencyLocked, phaseAligned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub servo: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slave: Option<PortV>,
    /// The offsets from the master of the last reads: [unix ms, ns], oldest first.
    pub corrections: Vec<(i64, f64)>,
    /// Slave first, then masters, others, disabled ones.
    pub ports: Vec<PtpPortV>,
}

/// An address of the inventory, as the PTP views name things.
#[derive(Clone, Debug, Default)]
pub struct AddrInfo {
    pub name: Option<String>,
    pub mac: Option<String>,
}

/// An NMOS node of the registry: its label, addresses and MACs.
#[derive(Clone, Debug, Default)]
pub struct NmosNode {
    pub label: String,
    pub ips: Vec<Ipv4Addr>,
    pub macs: Vec<String>,
}

/// What names the ports' devices.
pub struct Names<'a> {
    pub switches: &'a [Switch],
    pub addresses: &'a HashMap<Ipv4Addr, AddrInfo>,
    /// The NMOS registry's nodes (empty when it is not read).
    pub nmos: &'a [NmosNode],
}

impl Names<'_> {
    fn addr(&self, ip: Ipv4Addr) -> String {
        self.addresses
            .get(&ip)
            .and_then(|a| a.name.clone())
            .or_else(|| self.nmos.iter().find(|n| n.ips.contains(&ip) && !n.label.is_empty()).map(|n| n.label.clone()))
            .unwrap_or_else(|| ip.to_string())
    }

    /// The address whose MAC this is.
    pub fn by_mac(&self, mac: &str) -> Option<(Ipv4Addr, Option<String>)> {
        let mut v: Vec<(&Ipv4Addr, &AddrInfo)> = self.addresses.iter().filter(|(_, a)| a.mac.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(mac))).collect();
        v.sort_by_key(|(ip, _)| u32::from(**ip));
        v.first().map(|(ip, a)| (**ip, a.name.clone()))
    }

    /// A grandmaster (or any clock) named by the inventory address of the MAC inside its identity.
    pub fn clock(&self, identity: &str) -> (Option<String>, Option<Ipv4Addr>) {
        match ptp::identity_mac(identity).and_then(|m| self.by_mac(&m)) {
            Some((ip, name)) => (Some(name.unwrap_or_else(|| ip.to_string())), Some(ip)),
            None => (None, None),
        }
    }

    /// The inventory switch a name designates (its name, without domain or serial).
    fn switch_named(&self, name: &str) -> Option<&Switch> {
        let n = si::clean_device_id(name);
        (!n.is_empty()).then_some(())?;
        self.switches.iter().find(|s| si::clean_device_id(&s.name).eq_ignore_ascii_case(&n))
    }

    /// What is plugged on port `n` of `sw`: (name, IP, inventory switch).
    pub fn device(&self, sw: &Switch, n: u16) -> (Option<String>, Option<Ipv4Addr>, Option<String>) {
        let Some(p) = sw.ports.iter().find(|p| p.n == n) else { return (None, None, None) };
        if let Some(r) = &p.peer {
            if let Some(o) = self.switches.iter().find(|s| s.id == r.id) {
                let port = o.ports.iter().find(|x| x.n == r.port).map(|x| x.name.clone()).unwrap_or_else(|| r.port.to_string());
                return (Some(format!("{} · {port}", o.name)), o.ip, Some(o.id.clone()));
            }
        }
        if let Some(l) = p.lldp.as_ref().filter(|l| !l.name.is_empty()) {
            if let Some(o) = self.switch_named(&l.name).filter(|o| o.id != sw.id) {
                let port = o.ports.iter().find(|x| si::same_port(&x.name, &l.port)).map(|x| x.name.clone()).unwrap_or_else(|| l.port.clone());
                return (Some(format!("{} · {port}", o.name)), o.ip, Some(o.id.clone()));
            }
        }
        if let Some(ip) = p.host {
            return (Some(self.addr(ip)), Some(ip), None);
        }
        if let Some(l) = p.lldp.as_ref().filter(|l| !l.name.is_empty()) {
            return (Some(l.name.clone()), None, None);
        }
        // An edge port with few MACs: the inventory address of one of them, else an NMOS node.
        if (1..=3).contains(&p.learnt.len()) {
            if let Some((ip, name)) = p.learnt.iter().find_map(|m| self.by_mac(m)) {
                return (Some(name.unwrap_or_else(|| ip.to_string())), Some(ip), None);
            }
            if let Some(n) = self.nmos.iter().find(|n| !n.label.is_empty() && n.macs.iter().any(|m| p.learnt.iter().any(|l| l.eq_ignore_ascii_case(m)))) {
                return (Some(n.label.clone()), n.ips.first().copied(), None);
            }
        }
        (Some(p.label.clone()).filter(|l| !l.is_empty()), None, None)
    }
}

fn state_rank(s: Option<PortState>) -> u8 {
    match s {
        Some(PortState::Slave) => 0,
        Some(PortState::Master) => 1,
        Some(PortState::Disabled | PortState::Faulty) | None => 3,
        _ => 2,
    }
}

/// The ifIndex of a PTP port: given by the switch, else by its name.
fn port_if(ifs: &Ifs, p: &strata_net::ptpmib::PtpPortMib) -> Option<u32> {
    if p.if_index > 0 {
        return Some(p.if_index);
    }
    let name = p.name.trim();
    if name.is_empty() {
        return None;
    }
    let mut v: Vec<(&u32, &String)> = ifs.names.iter().collect();
    v.sort_by_key(|x| *x.0);
    v.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).or_else(|| v.iter().find(|(_, n)| si::same_port(n, name))).map(|(i, _)| **i)
}

/// The PTP view of a switch; `None` when it has no PTP data.
pub fn switch_view(mem: &PtpMem, sw: &Switch, ifs: &Ifs, names: &Names) -> Option<PtpSwitchV> {
    let main = mem.main()?;
    let place = |p: &strata_net::ptpmib::PtpPortMib| match port_if(ifs, p) {
        Some(i) => ifs.port(i),
        None => PortV { n: None, port: if p.name.is_empty() { format!("PTP {}", p.number) } else { p.name.clone() } },
    };
    let mut rows: Vec<(u8, u32, PtpPortV)> = Vec::new();
    for c in &mem.clocks {
        for p in &c.ports {
            let at = place(p);
            let t = mem.track(c, p.number);
            let master = p.state == Some(PortState::Master);
            let activity = t.and_then(Track::activity).filter(|_| master);
            let (device, device_ip, peer_switch) = at.n.map(|n| names.device(sw, n)).unwrap_or_default();
            let row = PtpPortV {
                number: p.number,
                domain: c.domain,
                state: p.state.map_or("unknown", PortState::name),
                log_announce: p.log_announce,
                log_sync: p.log_sync,
                log_delay_req: p.log_min_delay_req,
                announce_timeout: p.announce_timeout,
                delay_mech: p.delay_mech.map(|d| d.name()),
                rx_ps: t.and_then(|t| t.rx_ps),
                tx_ps: t.and_then(|t| t.tx_ps),
                activity,
                mute_since: t.and_then(|t| t.last_rx).filter(|_| activity == Some("mute")),
                device,
                device_ip,
                peer_switch,
                at,
            };
            rows.push((state_rank(p.state), row.at.n.map_or(u32::MAX, u32::from), row));
        }
    }
    rows.sort_by_key(|r| (r.0, r.1, r.2.number));
    let gm = main.gm.clone().map(|identity| {
        let (name, ip) = names.clock(&identity);
        GmV {
            name,
            ip,
            priority1: main.gm_priority1,
            priority2: main.gm_priority2,
            clock_class: main.gm_class,
            clock_accuracy: main.gm_accuracy,
            accuracy_ns: main.gm_accuracy.and_then(ptp::accuracy_ns),
            identity,
        }
    });
    Some(PtpSwitchV {
        at: mem.at,
        domain: main.domain,
        clock_type: main.clock_type.name(),
        identity: main.identity.clone(),
        two_step: main.two_step,
        steps_removed: main.steps_removed,
        offset_ns: main.offset_ns,
        mean_path_delay_ns: main.mean_path_delay_ns,
        parent: main.parent.clone(),
        parent_port: main.parent_port,
        gm,
        servo: main.servo,
        slave: main.slave().map(place),
        corrections: mem.history(main),
        ports: rows.into_iter().map(|r| r.2).collect(),
    })
}

/// Master ports whose device went silent more than `after_secs` ago while the
/// link is up (`up` of the faceplate port): (port, device name).
pub fn mutes(v: &PtpSwitchV, up: impl Fn(u16) -> bool, after_secs: f64, now: i64) -> Vec<(u16, String)> {
    v.ports
        .iter()
        .filter(|p| p.activity == Some("mute") && p.peer_switch.is_none())
        .filter_map(|p| Some((p.at.n?, p)))
        .filter(|(n, p)| up(*n) && p.mute_since.is_some_and(|t| (now - t) as f64 / 1000.0 > after_secs))
        .map(|(n, p)| (n, p.device.clone().unwrap_or_default()))
        .collect()
}

// ── The multicast PTP view ──

/// An online switch's PTP view, for the multicast view.
#[derive(Clone, Debug, Default)]
pub struct SwitchPtpIn {
    pub id: String,
    pub name: String,
    pub ip: Option<Ipv4Addr>,
    pub view: PtpSwitchV,
}

/// A switch port.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub switch_id: String,
    pub switch_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u16>,
    pub port: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrandmasterV {
    pub identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<Ipv4Addr>,
    pub domain: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority1: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority2: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_class: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_accuracy: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy_ns: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_source: Option<String>,
    /// Heard in an Announce in the last 10 s (or read from a switch).
    pub live: bool,
    /// Where its dataset comes from: ptp (an Announce), snmp (a switch's parent dataset).
    pub via: &'static str,
    /// The switch port it is plugged on (a switch's slave port, else its MAC in the MAC tables).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Place>,
    /// That port's PTP state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_state: Option<&'static str>,
    /// From the switch that talks to this machine up to the grandmaster: each switch's slave port.
    pub path: Vec<Place>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkerV {
    /// Clock identity and PTP port number of the Announce sender.
    pub clock: String,
    pub port: u16,
    pub ip: Ipv4Addr,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub domain: u8,
    pub steps_removed: u16,
    /// gm (the grandmaster itself), boundary (relays it).
    pub role: &'static str,
    pub live: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_announce: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_sync: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_delay_req: Option<i8>,
    /// The switch port it is (or this machine's port when the sender is a switch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Place>,
    /// How the switch was found: snmp (its clock identity), ip (the switch's address), mac (its MAC table).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<&'static str>,
    /// The port's PTP state read from the switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<&'static str>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchSyncV {
    pub switch_id: String,
    pub switch_name: String,
    pub clock_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slave: Option<PortV>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_ns: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_path_delay_ns: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps_removed: Option<u16>,
    /// The offsets of the last reads, oldest first (ns).
    pub history: Vec<f64>,
}

/// A master (or edge slave) port of a switch and the device behind it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortRowV {
    pub switch_id: String,
    pub switch_name: String,
    #[serde(flatten)]
    pub port: PtpPortV,
    /// This machine's port (the clock that talks to it).
    pub strata: bool,
}

/// A node of the clock tree; links go to `parent`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub id: String,
    /// gm, switch, device, clock (a clock heard, not a switch read).
    pub kind: &'static str,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The link to the parent: ok, idle (no Delay_Req), bad (silent, other domain), unknown (guessed).
    pub link: &'static str,
    /// Switch: its slave port; device: the master port it is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_ns: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_class: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<u8>,
}

impl TreeNode {
    fn new(id: String, kind: &'static str, name: String, parent: Option<String>, link: &'static str) -> TreeNode {
        TreeNode { id, kind, name, parent, link, port: None, offset_ns: None, activity: None, clock_type: None, clock_class: None, time_source: None, domain: None }
    }
}

/// What the multicast PTP view adds to the clocks heard.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtpSync {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grandmaster: Option<GrandmasterV>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub talker: Option<TalkerV>,
    pub switch_sync: Vec<SwitchSyncV>,
    pub ports: Vec<PortRowV>,
    pub tree: Vec<TreeNode>,
}

/// What [`sync_view`] reads.
#[derive(Clone)]
pub struct SyncIn<'a> {
    /// The best clock heard (the one that talks to this machine) and whether it is live.
    pub talker: Option<(&'a ptp::Announcer, bool)>,
    pub switches: &'a [SwitchPtpIn],
    /// Every inventory switch: id, name, IP, its agent's IP.
    pub inventory: &'a [(String, String, Option<Ipv4Addr>, Option<Ipv4Addr>)],
    /// Inventory name of the talker's address.
    pub talker_name: Option<String>,
    /// Where this machine is plugged (its address in the MAC tables).
    pub strata_at: Option<Place>,
    /// Where the grandmaster's MAC is learnt.
    pub gm_at: Option<Place>,
    /// Inventory name and address of the grandmaster (by the MAC in its identity).
    pub gm_name: Option<String>,
    pub gm_ip: Option<Ipv4Addr>,
}

fn place_of(sw: &SwitchPtpIn, at: &PortV) -> Place {
    Place { switch_id: sw.id.clone(), switch_name: sw.name.clone(), n: at.n, port: at.port.clone() }
}

/// The parent of each switch in the clock tree: another switch (its slave port's
/// peer, else the switch whose clock identity its parent dataset names), else the
/// grandmaster ("gm"). With whether the link is known (not guessed).
pub fn parents(switches: &[SwitchPtpIn], gm: Option<&str>) -> HashMap<String, (String, bool)> {
    let by_identity: HashMap<&str, &str> = switches.iter().filter_map(|s| Some((s.view.identity.as_deref()?, s.id.as_str()))).collect();
    let mut out: HashMap<String, (String, bool)> = HashMap::new();
    for s in switches {
        let slave = s.view.slave.as_ref().and_then(|sl| s.view.ports.iter().find(|p| p.at == *sl));
        let by_peer = slave.and_then(|p| p.peer_switch.clone()).filter(|id| *id != s.id);
        let by_parent = s.view.parent.as_deref().and_then(|p| by_identity.get(p)).map(|id| id.to_string()).filter(|id| *id != s.id);
        let to_gm = s.view.parent.is_some() && s.view.parent.as_deref() == gm || s.view.steps_removed == Some(1);
        let p = match (by_peer, by_parent) {
            (Some(id), _) | (None, Some(id)) => (id, true),
            (None, None) => ("gm".to_string(), to_gm),
        };
        out.insert(s.id.clone(), p);
    }
    // Loops (wrong data): broken at the member nearest the grandmaster (fewest steps), linked to it.
    let steps: HashMap<&str, u16> = switches.iter().map(|s| (s.id.as_str(), s.view.steps_removed.unwrap_or(u16::MAX))).collect();
    let mut ids: Vec<String> = out.keys().cloned().collect();
    ids.sort();
    for id in ids {
        let mut chain = vec![id.clone()];
        let mut cur = id.clone();
        while let Some((p, _)) = out.get(&cur).cloned() {
            if p == "gm" || !out.contains_key(&p) {
                break;
            }
            if let Some(i) = chain.iter().position(|x| *x == p) {
                let lp = &chain[i..];
                if let Some(cut) = lp.iter().min_by_key(|x| (steps.get(x.as_str()).copied().unwrap_or(u16::MAX), (*x).clone())).cloned() {
                    out.insert(cut, ("gm".to_string(), false));
                }
                break;
            }
            chain.push(p.clone());
            cur = p;
        }
    }
    out
}

/// The grandmaster, the clock that talks to this machine, the switches' synchronisation, their ports, the tree.
pub fn sync_view(inp: &SyncIn) -> PtpSync {
    let mut out = PtpSync::default();
    let talker = inp.talker.map(|t| t.0);
    let sw_gm = inp.switches.iter().find_map(|s| Some((s, s.view.gm.as_ref()?)));
    let gm_identity = talker.map(|t| t.gm.clone()).or_else(|| sw_gm.map(|(_, g)| g.identity.clone()));
    let parents = parents(inp.switches, gm_identity.as_deref());
    let by_id: HashMap<&str, &SwitchPtpIn> = inp.switches.iter().map(|s| (s.id.as_str(), s)).collect();

    // The talker: the sender of the best Announce, identified as a switch port when it is one.
    if let Some((a, live)) = inp.talker {
        let mut t = TalkerV {
            clock: a.clock.clone(),
            port: a.port,
            ip: a.ip,
            name: inp.talker_name.clone(),
            domain: a.domain,
            steps_removed: a.steps_removed,
            role: if a.clock == a.gm && a.steps_removed == 0 { "gm" } else { "boundary" },
            live,
            log_announce: a.log_announce,
            log_sync: a.log_sync,
            log_delay_req: a.log_delay_req,
            at: None,
            via: None,
            state: None,
        };
        let by_snmp = inp.switches.iter().find(|s| s.view.identity.as_deref() == Some(a.clock.as_str()));
        let by_ip = || inp.inventory.iter().find(|(_, _, ip, agent)| *ip == Some(a.ip) || *agent == Some(a.ip)).map(|x| x.0.clone());
        let (sw_id, via) = match by_snmp {
            Some(s) => (Some(s.id.clone()), Some("snmp")),
            None => match by_ip() {
                Some(id) => (Some(id), Some("ip")),
                // A sender with an identity that is not the grandmaster's, on the switch this machine is on.
                None if t.role == "boundary" => (inp.strata_at.as_ref().map(|p| p.switch_id.clone()), inp.strata_at.as_ref().map(|_| "mac")),
                None => (None, None),
            },
        };
        if let Some(id) = sw_id {
            let sw = by_id.get(id.as_str());
            // Its port: the PTP port of the Announce's number, else where this machine is plugged.
            let snmp_port = sw.filter(|_| via == Some("snmp")).and_then(|s| s.view.ports.iter().find(|p| p.number == u32::from(a.port)).map(|p| (place_of(s, &p.at), p.state)));
            let at = snmp_port
                .clone()
                .map(|x| x.0)
                .or_else(|| inp.strata_at.clone().filter(|p| p.switch_id == id))
                .or_else(|| inp.inventory.iter().find(|x| x.0 == id).map(|x| Place { switch_id: id.clone(), switch_name: x.1.clone(), n: None, port: String::new() }));
            let state = snmp_port.map(|x| x.1).or_else(|| at.as_ref().and_then(|p| sw.and_then(|s| s.view.ports.iter().find(|x| x.at.n.is_some() && x.at.n == p.n)).map(|x| x.state)));
            t.state = state.filter(|s| *s != "unknown");
            t.at = at;
            t.via = via;
        } else if t.role == "gm" {
            t.at.clone_from(&inp.gm_at);
        }
        out.talker = Some(t);
    }

    // The grandmaster: the dataset the talker announces, else a switch's parent dataset.
    if let Some(identity) = gm_identity.clone() {
        let mut g = GrandmasterV { identity: identity.clone(), name: inp.gm_name.clone(), ip: inp.gm_ip, ..GrandmasterV::default() };
        match talker {
            Some(a) => {
                g.domain = a.domain;
                g.priority1 = Some(a.priority1);
                g.priority2 = Some(a.priority2);
                g.clock_class = Some(a.clock_class);
                g.clock_accuracy = Some(a.clock_accuracy);
                g.accuracy_ns = a.accuracy_ns;
                g.time_source = Some(a.time_source.clone());
                g.live = inp.talker.is_some_and(|t| t.1);
                g.via = "ptp";
            }
            None => {
                if let Some((s, sg)) = sw_gm {
                    g.domain = s.view.domain;
                    g.priority1 = sg.priority1;
                    g.priority2 = sg.priority2;
                    g.clock_class = sg.clock_class;
                    g.clock_accuracy = sg.clock_accuracy;
                    g.accuracy_ns = sg.accuracy_ns;
                    g.live = true;
                    g.via = "snmp";
                }
            }
        }
        // Plugged on the slave port of a switch that follows it directly (not another switch).
        let top = inp.switches.iter().find(|s| parents.get(&s.id).is_some_and(|p| p.0 == "gm") && (s.view.parent.as_deref() == Some(identity.as_str()) || s.view.steps_removed == Some(1)));
        let top_port = top.and_then(|s| {
            let sl = s.view.slave.as_ref()?;
            let p = s.view.ports.iter().find(|p| p.at == *sl)?;
            p.peer_switch.is_none().then(|| (place_of(s, sl), p.state))
        });
        match top_port {
            Some((at, state)) => {
                g.at = Some(at);
                g.port_state = Some(state);
            }
            None => g.at.clone_from(&inp.gm_at),
        }
        // The path from this machine's switch up to the grandmaster.
        let start = out.talker.as_ref().and_then(|t| t.at.as_ref()).map(|p| p.switch_id.clone()).filter(|id| by_id.contains_key(id.as_str()));
        let mut cur = start;
        let mut seen = HashSet::new();
        while let Some(id) = cur.take() {
            if !seen.insert(id.clone()) {
                break;
            }
            let Some(s) = by_id.get(id.as_str()) else { break };
            if let Some(sl) = &s.view.slave {
                g.path.push(place_of(s, sl));
            }
            cur = parents.get(&id).map(|p| p.0.clone()).filter(|p| p != "gm");
        }
        out.grandmaster = Some(g);
    }

    // The switches' synchronisation.
    for s in inp.switches.iter().filter(|s| s.view.slave.is_some() || s.view.offset_ns.is_some()) {
        out.switch_sync.push(SwitchSyncV {
            switch_id: s.id.clone(),
            switch_name: s.name.clone(),
            clock_type: s.view.clock_type,
            slave: s.view.slave.clone(),
            offset_ns: s.view.offset_ns,
            mean_path_delay_ns: s.view.mean_path_delay_ns,
            steps_removed: s.view.steps_removed,
            history: s.view.corrections.iter().map(|c| c.1).collect(),
        });
    }
    out.switch_sync.sort_by(|a, b| a.switch_name.cmp(&b.switch_name));

    // The master ports facing devices (not switch links).
    let strata = out.talker.as_ref().and_then(|t| t.at.clone()).filter(|p| p.n.is_some());
    for s in inp.switches {
        for p in s.view.ports.iter().filter(|p| p.state == "master" && p.peer_switch.is_none()) {
            let is_strata = strata.as_ref().is_some_and(|x| x.switch_id == s.id && x.n == p.at.n);
            out.ports.push(PortRowV { switch_id: s.id.clone(), switch_name: s.name.clone(), port: p.clone(), strata: is_strata });
        }
    }
    out.ports.sort_by(|a, b| (&a.switch_name, a.port.at.n, a.port.number).cmp(&(&b.switch_name, b.port.at.n, b.port.number)));

    out.tree = tree(&out, inp, &parents);
    out
}

/// The clock tree: grandmaster, then the switches by their parents, then the devices on their master ports.
fn tree(v: &PtpSync, inp: &SyncIn, parents: &HashMap<String, (String, bool)>) -> Vec<TreeNode> {
    let mut nodes = Vec::new();
    let Some(g) = &v.grandmaster else { return nodes };
    let mut gm = TreeNode::new("gm".into(), "gm", g.name.clone().unwrap_or_else(|| g.identity.clone()), None, "ok");
    gm.clock_class = g.clock_class;
    gm.time_source.clone_from(&g.time_source);
    gm.domain = Some(g.domain);
    nodes.push(gm);
    let by_id: HashMap<&str, &SwitchPtpIn> = inp.switches.iter().map(|s| (s.id.as_str(), s)).collect();
    let inv_name = |id: &str| inp.inventory.iter().find(|x| x.0 == id).map(|x| x.1.clone());
    // Switches named as parents but not read (offline, no PTP MIB): shown under the grandmaster.
    let mut extra: Vec<String> = parents.values().map(|p| p.0.clone()).filter(|p| p != "gm" && !by_id.contains_key(p.as_str())).collect();
    extra.sort();
    extra.dedup();
    for id in extra {
        let name = inv_name(&id).unwrap_or_else(|| id.clone());
        nodes.push(TreeNode::new(format!("s:{id}"), "switch", name, Some("gm".into()), "unknown"));
    }
    let mut sws: Vec<&SwitchPtpIn> = inp.switches.iter().collect();
    sws.sort_by(|a, b| a.name.cmp(&b.name));
    for s in &sws {
        let (p, known) = parents.get(&s.id).cloned().unwrap_or_else(|| ("gm".into(), false));
        let parent = if p == "gm" { "gm".to_string() } else { format!("s:{p}") };
        let mut n = TreeNode::new(format!("s:{}", s.id), "switch", s.name.clone(), Some(parent), if known { "ok" } else { "unknown" });
        n.port = s.view.slave.as_ref().map(|x| x.port.clone());
        n.offset_ns = s.view.offset_ns;
        n.clock_type = Some(s.view.clock_type);
        n.domain = Some(s.view.domain);
        nodes.push(n);
    }
    // Devices on the master ports (this machine left out).
    for r in v.ports.iter().filter(|r| !r.strata) {
        let Some(name) = r.port.device.clone() else { continue };
        let link = match r.port.activity {
            Some("mute") => "bad",
            Some("none") => "idle",
            _ if r.port.domain != g.domain => "bad",
            _ => "ok",
        };
        let mut n = TreeNode::new(format!("d:{}:{}", r.switch_id, r.port.number), "device", name, Some(format!("s:{}", r.switch_id)), link);
        n.port = Some(r.port.at.port.clone());
        n.activity = r.port.activity;
        n.domain = Some(r.port.domain);
        nodes.push(n);
    }
    // Without a switch read: the clock that talks to this machine, between the grandmaster and it.
    if inp.switches.is_empty() {
        if let Some(t) = v.talker.as_ref().filter(|t| t.role == "boundary") {
            let name = t.at.as_ref().map(|p| if p.port.is_empty() { p.switch_name.clone() } else { format!("{} · {}", p.switch_name, p.port) }).or_else(|| t.name.clone()).unwrap_or_else(|| t.ip.to_string());
            let mut n = TreeNode::new("talker".into(), "clock", name, Some("gm".into()), "ok");
            n.domain = Some(t.domain);
            nodes.push(n);
        }
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_net::ptpmib::PtpPortMib;

    fn sw(id: &str, name: &str) -> Switch {
        let mut s: Switch = serde_json::from_value(serde_json::json!({"id": id, "name": name, "groups": [{"kind": "sfp", "count": 52, "naming": "Eth1/{n}"}]})).unwrap();
        s.normalize();
        s
    }

    fn ifs() -> Ifs {
        let names: HashMap<u32, String> = (1..=52).map(|n| (1000 + n, format!("Ethernet1/{n}"))).collect();
        Ifs { ports: names.keys().map(|i| (*i, (*i - 1000) as u16)).collect(), names }
    }

    fn port(number: u32, name: &str, state: PortState, rx: u64, tx: u64) -> PtpPortMib {
        PtpPortMib { number, name: name.into(), state: Some(state), log_announce: Some(0), log_sync: Some(-3), log_min_delay_req: Some(-3), rx: Some(rx), tx: Some(tx), ..PtpPortMib::default() }
    }

    fn clock(identity: &str, parent: &str, steps: u16, ports: Vec<PtpPortMib>) -> PtpClockMib {
        let mut c = PtpClockMib::new(127, ClockType::Boundary, 0);
        c.identity = Some(identity.into());
        c.parent = Some(parent.into());
        c.gm = Some(GM.into());
        c.gm_class = Some(6);
        c.gm_accuracy = Some(0x31);
        c.steps_removed = Some(steps);
        c.offset_ns = Some(-80.0);
        c.mean_path_delay_ns = Some(6900.0);
        c.ports = ports;
        c
    }

    const GM: &str = "EC:46:70:FF:FE:00:1A:2B";
    const CORE: &str = "00:3A:9C:FF:FE:11:22:33";
    const A: &str = "70:69:5A:FF:FE:EE:6F:79";

    #[test]
    fn activity_and_mute() {
        let mut t = Track::default();
        assert_eq!(t.activity(), None);
        t.update(Some(100), Some(10), 0);
        assert_eq!(t.activity(), None);
        t.update(Some(220), Some(40), 15_000);
        assert_eq!((t.activity(), t.rx_ps, t.last_rx), (Some("active"), Some(8.0), Some(15_000)));
        t.update(Some(220), Some(70), 30_000);
        assert_eq!((t.activity(), t.last_rx), (Some("mute"), Some(15_000)));
        // A counter that went back (restart): no rate, still mute until it moves.
        t.update(Some(5), Some(0), 45_000);
        assert_eq!(t.activity(), None);
        t.update(Some(5), Some(0), 60_000);
        assert_eq!(t.activity(), Some("mute"));
        // Never asked.
        let mut n = Track::default();
        n.update(Some(0), Some(0), 0);
        n.update(Some(0), Some(30), 15_000);
        assert_eq!(n.activity(), Some("none"));
        // No counters at all.
        let mut z = Track::default();
        z.update(None, None, 0);
        z.update(None, None, 15_000);
        assert_eq!(z.activity(), None);
    }

    fn mem_a(now: i64, rx35: u64) -> PtpMem {
        let mut m = PtpMem::default();
        let ports = |rx35| vec![port(1, "Ethernet1/48", PortState::Slave, 0, 0), port(2, "Ethernet1/12", PortState::Master, 0, 0), port(3, "Ethernet1/35", PortState::Master, rx35, 0), port(4, "Ethernet1/31", PortState::Master, 0, 0), port(5, "Ethernet1/20", PortState::Disabled, 0, 0)];
        m.update(vec![clock(A, CORE, 2, ports(0))], now - 30_000);
        m.update(vec![clock(A, CORE, 2, ports(80))], now - 15_000);
        m.update(vec![clock(A, CORE, 2, ports(rx35))], now);
        m
    }

    #[test]
    fn switch_view_maps_ports_and_devices() {
        let core = sw("c", "SW-Core-01");
        let mut a = sw("a", "SW-Plateau-A");
        a.ports[47].peer = Some(super::super::model::PortRef { id: "c".into(), port: 1 });
        a.ports[11].host = Some(Ipv4Addr::new(10, 10, 1, 12));
        a.ports[34].label = "enregistreur-01".into();
        let addresses: HashMap<Ipv4Addr, AddrInfo> = [
            (Ipv4Addr::new(10, 10, 1, 12), AddrInfo { name: Some("cobalt-bidi4-01".into()), mac: None }),
            (Ipv4Addr::new(10, 99, 0, 47), AddrInfo { name: Some("gm-meinberg-01".into()), mac: Some("ec:46:70:00:1a:2b".into()) }),
        ]
        .into();
        let switches = vec![core, a.clone()];
        let names = Names { switches: &switches, addresses: &addresses, nmos: &[] };
        let m = mem_a(100_000, 80);
        let v = switch_view(&m, &a, &ifs(), &names).unwrap();
        assert_eq!((v.clock_type, v.identity.as_deref(), v.steps_removed), ("boundary", Some(A), Some(2)));
        assert_eq!(v.slave.as_ref().map(|s| (s.n, s.port.as_str())), Some((Some(48), "Ethernet1/48")));
        let g = v.gm.as_ref().unwrap();
        // The grandmaster named by the MAC of its identity; an accuracy code beyond the table gives none.
        assert_eq!((g.name.as_deref(), g.ip, g.accuracy_ns), (Some("gm-meinberg-01"), Some(Ipv4Addr::new(10, 99, 0, 47)), None));
        let order: Vec<(Option<u16>, &str)> = v.ports.iter().map(|p| (p.at.n, p.state)).collect();
        assert_eq!(order, vec![(Some(48), "slave"), (Some(12), "master"), (Some(31), "master"), (Some(35), "master"), (Some(20), "disabled")]);
        assert_eq!((v.ports[0].device.as_deref(), v.ports[0].peer_switch.as_deref()), (Some("SW-Core-01 · Eth1/1"), Some("c")));
        assert_eq!(v.ports[1].device.as_deref(), Some("cobalt-bidi4-01"));
        assert_eq!(v.ports[3].device.as_deref(), Some("enregistreur-01"));
        // Eth1/35 asked, then stopped: mute since the read that last saw it ask.
        assert_eq!((v.ports[3].activity, v.ports[3].mute_since), (Some("mute"), Some(85_000)));
        assert_eq!(v.ports[2].activity, Some("none"));
        assert_eq!(v.ports[0].activity, None);
        assert_eq!(v.corrections.len(), 3);
        // The alert: only after the delay, only with the link up.
        assert!(mutes(&v, |_| true, 60.0, 100_000).is_empty());
        assert_eq!(mutes(&v, |_| true, 60.0, 146_000), vec![(35, "enregistreur-01".to_string())]);
        assert!(mutes(&v, |n| n != 35, 60.0, 146_000).is_empty());
        // Still asking: not mute.
        let busy = switch_view(&mem_a(100_000, 160), &a, &ifs(), &names).unwrap();
        assert_eq!(busy.ports[3].activity, Some("active"));
        // No PTP read: no view.
        assert!(switch_view(&PtpMem::default(), &a, &ifs(), &names).is_none());
    }

    fn input(id: &str, name: &str, identity: &str, parent: &str, steps: u16, ports: Vec<PtpPortV>, slave: Option<u16>) -> SwitchPtpIn {
        let slave = slave.map(|n| PortV { n: Some(n), port: format!("Ethernet1/{n}") });
        SwitchPtpIn {
            id: id.into(),
            name: name.into(),
            ip: None,
            view: PtpSwitchV {
                domain: 127,
                clock_type: "boundary",
                identity: Some(identity.into()),
                steps_removed: Some(steps),
                parent: Some(parent.into()),
                gm: Some(GmV { identity: GM.into(), clock_class: Some(6), ..GmV::default() }),
                offset_ns: Some(-50.0),
                slave,
                corrections: vec![(1, -40.0), (2, -50.0)],
                ports,
                ..PtpSwitchV::default()
            },
        }
    }

    fn pv(number: u32, n: u16, state: &'static str, device: Option<&str>, peer: Option<&str>, activity: Option<&'static str>) -> PtpPortV {
        PtpPortV {
            at: PortV { n: Some(n), port: format!("Ethernet1/{n}") },
            number,
            domain: 127,
            state,
            log_announce: Some(0),
            log_sync: Some(-3),
            log_delay_req: Some(-3),
            announce_timeout: None,
            delay_mech: None,
            rx_ps: None,
            tx_ps: None,
            activity,
            mute_since: None,
            device: device.map(String::from),
            device_ip: None,
            peer_switch: peer.map(String::from),
        }
    }

    fn announcer(clock: &str, gm: &str, steps: u16, port: u16) -> ptp::Announcer {
        ptp::Announcer {
            ip: Ipv4Addr::new(10, 10, 1, 1),
            domain: 127,
            clock: clock.into(),
            port,
            gm: gm.into(),
            priority1: 128,
            clock_class: 6,
            clock_accuracy: 0x21,
            accuracy_ns: Some(100.0),
            variance: 0,
            priority2: 128,
            steps_removed: steps,
            time_source: "gps".into(),
            utc_offset: 37,
            first_seen: 0,
            last_seen: 0,
            count: 1,
            rate: 1.0,
            log_announce: Some(0),
            log_sync: Some(-3),
            log_delay_req: Some(-3),
        }
    }

    fn scene() -> Vec<SwitchPtpIn> {
        vec![
            input("c", "SW-Core-01", CORE, GM, 1, vec![pv(1, 10, "slave", Some("gm-meinberg-01"), None, None), pv(2, 1, "master", Some("SW-Plateau-A · Eth1/48"), Some("a"), None)], Some(10)),
            input(
                "a",
                "SW-Plateau-A",
                A,
                CORE,
                2,
                vec![
                    pv(1, 48, "slave", Some("SW-Core-01 · Eth1/1"), Some("c"), None),
                    pv(2, 12, "master", Some("cobalt-bidi4-01"), None, Some("active")),
                    pv(3, 35, "master", Some("enregistreur-01"), None, Some("mute")),
                    pv(31, 31, "master", None, None, Some("none")),
                    pv(4, 33, "master", Some("multiviewer-01"), None, Some("none")),
                ],
                Some(48),
            ),
            // Its slave port's peer is unknown: found by its parent identity.
            input("b", "SW-Plateau-B", "11:11:11:FF:FE:11:11:11", CORE, 2, vec![pv(1, 48, "slave", None, None, None)], Some(48)),
        ]
    }

    #[test]
    fn grandmaster_and_talker_are_told_apart() {
        let sws = scene();
        let a = announcer(A, GM, 2, 31);
        let inv = vec![("c".to_string(), "SW-Core-01".to_string(), None, None), ("a".into(), "SW-Plateau-A".into(), Some(Ipv4Addr::new(10, 99, 0, 21)), None)];
        let inp = SyncIn { talker: Some((&a, true)), switches: &sws, inventory: &inv, talker_name: None, strata_at: None, gm_at: None, gm_name: Some("gm-meinberg-01".into()), gm_ip: Some(Ipv4Addr::new(10, 99, 0, 47)) };
        let v = sync_view(&inp);
        let g = v.grandmaster.as_ref().unwrap();
        assert_eq!((g.identity.as_str(), g.name.as_deref(), g.via, g.clock_class, g.time_source.as_deref()), (GM, Some("gm-meinberg-01"), "ptp", Some(6), Some("gps")));
        // Plugged on the core's slave port; reached from this machine's switch through the core.
        assert_eq!(g.at.as_ref().map(|p| (p.switch_name.as_str(), p.n)), Some(("SW-Core-01", Some(10))));
        assert_eq!(g.port_state, Some("slave"));
        assert_eq!(g.path.iter().map(|p| (p.switch_name.as_str(), p.n)).collect::<Vec<_>>(), vec![("SW-Plateau-A", Some(48)), ("SW-Core-01", Some(10))]);
        let t = v.talker.as_ref().unwrap();
        assert_eq!((t.role, t.via, t.steps_removed), ("boundary", Some("snmp"), 2));
        assert_eq!(t.at.as_ref().map(|p| (p.switch_name.as_str(), p.n)), Some(("SW-Plateau-A", Some(31))));
        assert_eq!(t.state, Some("master"));
        // Rows: master edge ports only; this machine's port marked.
        let rows: Vec<(&str, Option<u16>, bool)> = v.ports.iter().map(|r| (r.switch_name.as_str(), r.port.at.n, r.strata)).collect();
        assert_eq!(rows, vec![("SW-Plateau-A", Some(12), false), ("SW-Plateau-A", Some(31), true), ("SW-Plateau-A", Some(33), false), ("SW-Plateau-A", Some(35), false)]);
        assert_eq!(v.switch_sync.len(), 3);

        // Without SNMP: the sender is told by its address; the dataset is still the grandmaster's.
        let none: Vec<SwitchPtpIn> = Vec::new();
        let strata_at = Place { switch_id: "a".into(), switch_name: "SW-Plateau-A".into(), n: Some(31), port: "Eth1/31".into() };
        let inp = SyncIn { talker: Some((&a, true)), switches: &none, inventory: &inv, talker_name: None, strata_at: Some(strata_at.clone()), gm_at: None, gm_name: None, gm_ip: None };
        let v = sync_view(&inp);
        let t = v.talker.as_ref().unwrap();
        assert_eq!((t.via, t.at.as_ref()), (Some("mac"), Some(&strata_at)));
        assert_eq!(v.grandmaster.as_ref().map(|g| (g.identity.as_str(), g.name.clone())), Some((GM, None)));
        let v = sync_view(&SyncIn { strata_at: None, ..inp.clone() });
        assert_eq!(v.talker.as_ref().map(|t| (t.via, t.at.clone())), Some((None, None)));
        let inp = SyncIn { strata_at: Some(strata_at.clone()), ..inp };
        let a2 = ptp::Announcer { ip: Ipv4Addr::new(10, 99, 0, 21), ..a.clone() };
        let v = sync_view(&SyncIn { talker: Some((&a2, true)), ..inp.clone() });
        let t = v.talker.as_ref().unwrap();
        assert_eq!((t.via, t.at.as_ref()), (Some("ip"), Some(&strata_at)));
        assert_eq!(v.tree.iter().map(|n| (n.id.as_str(), n.parent.as_deref())).collect::<Vec<_>>(), vec![("gm", None), ("talker", Some("gm"))]);

        // The grandmaster heard directly (no boundary clock).
        let d = announcer(GM, GM, 0, 1);
        let v = sync_view(&SyncIn { talker: Some((&d, true)), switches: &none, inventory: &inv, talker_name: Some("gm-meinberg-01".into()), strata_at: None, gm_at: Some(strata_at.clone()), gm_name: None, gm_ip: None });
        assert_eq!(v.talker.as_ref().map(|t| (t.role, t.at.clone())), Some(("gm", Some(strata_at))));
        // Nothing heard, SNMP only: the switches' parent dataset.
        let v = sync_view(&SyncIn { talker: None, switches: &sws, inventory: &inv, talker_name: None, strata_at: None, gm_at: None, gm_name: None, gm_ip: None });
        assert_eq!(v.grandmaster.as_ref().map(|g| (g.via, g.clock_class)), Some(("snmp", Some(6))));
        assert!(v.talker.is_none());
    }

    #[test]
    fn clock_tree() {
        let sws = scene();
        let a = announcer(A, GM, 2, 31);
        let inv = vec![("c".to_string(), "SW-Core-01".to_string(), None, None)];
        let v = sync_view(&SyncIn { talker: Some((&a, true)), switches: &sws, inventory: &inv, talker_name: None, strata_at: None, gm_at: None, gm_name: None, gm_ip: None });
        let edges: Vec<(&str, &str, Option<&str>)> = v.tree.iter().map(|n| (n.id.as_str(), n.link, n.parent.as_deref())).collect();
        assert_eq!(
            edges,
            vec![
                ("gm", "ok", None),
                ("s:c", "ok", Some("gm")),
                ("s:a", "ok", Some("s:c")),
                ("s:b", "ok", Some("s:c")),
                ("d:a:2", "ok", Some("s:a")),
                ("d:a:4", "idle", Some("s:a")),
                ("d:a:3", "bad", Some("s:a")),
            ]
        );
        assert_eq!(v.tree[0].name, GM);
        // A loop in the data is broken at the grandmaster; a parent switch not read is shown under it.
        let mut looped = scene();
        looped[0].view.ports[0].peer_switch = Some("a".into());
        looped[0].view.parent = Some(A.into());
        looped[0].view.steps_removed = Some(3);
        let p = parents(&looped, Some(GM));
        assert_eq!(p["c"].0, "a");
        assert_eq!(p["a"], ("gm".to_string(), false));
        let mut orphan = scene();
        orphan.remove(0);
        let v = sync_view(&SyncIn { talker: Some((&a, true)), switches: &orphan, inventory: &inv, talker_name: None, strata_at: None, gm_at: None, gm_name: None, gm_ip: None });
        assert!(v.tree.iter().any(|n| n.id == "s:c" && n.name == "SW-Core-01" && n.parent.as_deref() == Some("gm") && n.link == "unknown"));
        assert!(v.tree.iter().any(|n| n.id == "s:a" && n.parent.as_deref() == Some("s:c")));
        // B: its parent identity is the core's, which is not read: under the grandmaster, guessed.
        assert!(v.tree.iter().any(|n| n.id == "s:b" && n.parent.as_deref() == Some("gm") && n.link == "unknown"));
    }
}
