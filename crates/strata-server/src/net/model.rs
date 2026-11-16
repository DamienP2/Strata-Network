//! The network inventory as stored and sent (see `docs/api-network.md`).

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};
use strata_net::scan::Port;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastScan {
    pub at: i64,
    pub secs: f32,
    pub up: u64,
    pub stopped: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Network {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub range: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<Ipv4Addr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<String>,
    #[serde(default)]
    pub schedule_mins: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan: Option<LastScan>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortRef {
    pub id: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub ip: Ipv4Addr,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub reserved: bool,
    #[serde(default)]
    pub watched: bool,
    #[serde(default)]
    pub up: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_source: Option<String>,
    #[serde(default)]
    pub ports: Vec<Port>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
    #[serde(default)]
    pub is_new: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Ipv4Addr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch: Option<PortRef>,
}

impl Address {
    pub fn new(ip: Ipv4Addr) -> Address {
        Address {
            ip,
            label: String::new(),
            kind: None,
            notes: String::new(),
            reserved: false,
            watched: false,
            up: false,
            via: None,
            rtt_ms: None,
            mac: None,
            vendor: None,
            name: None,
            name_source: None,
            ports: Vec::new(),
            first_seen: None,
            last_seen: None,
            is_new: false,
            parent: None,
            switch: None,
        }
    }

    /// Nothing worth keeping: never seen and nothing set by hand.
    pub fn is_blank(&self) -> bool {
        self.label.is_empty()
            && self.kind.is_none()
            && self.notes.is_empty()
            && !self.reserved
            && !self.watched
            && self.last_seen.is_none()
            && self.parent.is_none()
            && self.switch.is_none()
    }

    pub fn state(&self) -> &'static str {
        if self.up {
            "used"
        } else if self.last_seen.is_some() {
            "offline"
        } else if self.reserved || !self.label.is_empty() {
            "reserved"
        } else {
            "free"
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vlan {
    pub id: u16,
    pub name: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subnet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<Ipv4Addr>,
    #[serde(default)]
    pub notes: String,
}

fn default_color() -> String {
    "#8fa3ff".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lldp {
    pub name: String,
    pub port: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchPort {
    pub n: u16,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "access")]
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
    #[serde(default)]
    pub tagged: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<Ipv4Addr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<PortRef>,
    #[serde(default)]
    pub learnt: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lldp: Option<Lldp>,
    /// Alert when its link goes down (online switches).
    #[serde(default)]
    pub watched: bool,
}

fn access() -> String {
    "access".into()
}

impl SwitchPort {
    pub fn new(n: u16) -> SwitchPort {
        SwitchPort { n, name: String::new(), label: String::new(), mode: access(), vlan: None, tagged: vec![], host: None, peer: None, learnt: vec![], lldp: None, watched: false }
    }
}

/// The kind of cage of a group of ports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortKind {
    #[default]
    Rj45,
    Sfp,
    Qsfp,
}

/// Ports of the same kind, side by side on the faceplate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortGroup {
    pub kind: PortKind,
    pub count: u16,
    /// Interface name with `{n}` for the number: "Gi1/0/{n}".
    #[serde(default)]
    pub naming: String,
    /// First number; absent = after the previous group when it has the same naming, else 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u16>,
}

impl PortGroup {
    pub fn new(kind: PortKind, count: u16, naming: &str) -> PortGroup {
        PortGroup { kind, count, naming: naming.into(), start: None }
    }
}

/// Most ports on one switch.
pub const MAX_PORTS: u16 = 256;

/// The first number of each group (see `PortGroup::start`).
pub fn group_starts(groups: &[PortGroup]) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::with_capacity(groups.len());
    for (i, g) in groups.iter().enumerate() {
        let start = match (g.start, i) {
            (Some(s), _) => s as u32,
            (None, 0) => 1,
            (None, _) if groups[i - 1].naming == g.naming => out[i - 1] + groups[i - 1].count as u32,
            _ => 1,
        };
        out.push(start);
    }
    out
}

/// The interface names of the faceplate, in order, with the kind of each port.
pub fn port_names(groups: &[PortGroup]) -> Vec<(String, PortKind)> {
    let starts = group_starts(groups);
    let mut out = Vec::new();
    for (g, start) in groups.iter().zip(starts) {
        let naming = if g.naming.contains("{n}") { g.naming.clone() } else { format!("{}{{n}}", g.naming) };
        for i in 0..g.count as u32 {
            out.push((naming.replace("{n}", &(start + i).to_string()), g.kind));
        }
    }
    out
}

/// Groups as they are kept: trimmed naming, no empty group, at most `MAX_PORTS` in all.
pub fn clean_groups(groups: &mut Vec<PortGroup>) {
    let mut left = MAX_PORTS;
    groups.retain_mut(|g| {
        g.naming = g.naming.trim().to_string();
        g.count = g.count.min(left);
        left -= g.count;
        g.count > 0
    });
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Switch {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<Ipv4Addr>,
    #[serde(default)]
    pub group: String,
    /// The faceplate: groups of ports, in order.
    #[serde(default)]
    pub groups: Vec<PortGroup>,
    /// Before port groups: copper ports, then SFP ports (read to migrate, never written).
    #[serde(default, skip_serializing)]
    pub copper: Option<u16>,
    #[serde(default, skip_serializing)]
    pub sfp: Option<u16>,
    #[serde(default, skip_serializing)]
    pub naming: Option<String>,
    #[serde(default, skip_serializing)]
    pub sfp_naming: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snmp: Option<String>,
    #[serde(default)]
    pub ports: Vec<SwitchPort>,
    /// Read live (state, counters, VLAN setup) through `snmp`.
    #[serde(default)]
    pub online: bool,
    /// Seconds between two live reads: 5, 10, 30 or 60.
    #[serde(default = "default_online_secs")]
    pub online_secs: u32,
    /// Alerts raised while the switch is online.
    #[serde(default)]
    pub rules: AlertRules,
}

/// One alert rule: on or off, and its limit (unused by some rules).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub on: bool,
    #[serde(default)]
    pub value: f32,
}

const fn rule(on: bool, value: f32) -> Rule {
    Rule { on, value }
}

/// The alert rules of an online switch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlertRules {
    /// Link down on a watched port.
    pub link_down: Rule,
    /// More than `value` link drops in 10 minutes.
    pub flap: Rule,
    /// More than `value` errors per minute on a port.
    pub errors: Rule,
    /// More than `value` discarded packets a second (in + out) on a port, over 1 minute.
    pub discards: Rule,
    /// Optical receive power below `value` dBm.
    pub optic_rx: Rule,
    /// Port load above `value` % for 1 minute.
    pub saturation: Rule,
    /// Power supply or fan fault, or a temperature above `value` °C.
    pub health: Rule,
    /// A new difference between the switch and the inventory.
    pub drift: Rule,
    /// A CDP / LLDP neighbour appeared or disappeared.
    pub neighbours: Rule,
    /// PoE power used above `value` % of the budget.
    pub poe: Rule,
}

impl Default for AlertRules {
    fn default() -> Self {
        AlertRules {
            link_down: rule(true, 0.0),
            flap: rule(true, 3.0),
            errors: rule(true, 100.0),
            discards: rule(true, 10.0),
            optic_rx: rule(true, -10.0),
            saturation: rule(true, 90.0),
            health: rule(true, 60.0),
            drift: rule(true, 0.0),
            neighbours: rule(false, 0.0),
            poe: rule(true, 85.0),
        }
    }
}

impl AlertRules {
    /// Limits kept in sensible bounds.
    pub fn clamp(&mut self) {
        self.flap.value = self.flap.value.round().clamp(1.0, 1000.0);
        self.errors.value = self.errors.value.round().clamp(1.0, 1e9);
        self.discards.value = ((self.discards.value * 10.0).round() / 10.0).clamp(0.1, 1e9);
        self.optic_rx.value = self.optic_rx.value.clamp(-40.0, 10.0);
        self.saturation.value = self.saturation.value.clamp(1.0, 100.0);
        self.health.value = self.health.value.clamp(20.0, 150.0);
        self.poe.value = self.poe.value.clamp(1.0, 100.0);
    }
}

pub const ONLINE_SECS: [u32; 4] = [5, 10, 30, 60];

fn default_online_secs() -> u32 {
    10
}

impl Switch {
    /// A switch saved before port groups: copper then SFP become two groups. True when it changed.
    pub fn migrate(&mut self) -> bool {
        let legacy = self.copper.is_some() || self.sfp.is_some() || self.naming.is_some() || self.sfp_naming.is_some();
        let (copper, sfp, naming, sfp_naming) = (self.copper.take(), self.sfp.take(), self.naming.take(), self.sfp_naming.take());
        if !self.groups.is_empty() || !legacy {
            return false;
        }
        self.groups = vec![
            PortGroup::new(PortKind::Rj45, copper.unwrap_or(24), naming.as_deref().unwrap_or("Gi1/0/{n}")),
            PortGroup::new(PortKind::Sfp, sfp.unwrap_or(0), sfp_naming.as_deref().unwrap_or("Te1/1/{n}")),
        ];
        self.groups.retain(|g| g.count > 0);
        true
    }

    pub fn port_count(&self) -> u16 {
        self.groups.iter().map(|g| g.count).sum()
    }

    /// Makes `ports` match the faceplate: one entry per port, in order, names filled.
    pub fn normalize(&mut self) {
        self.migrate();
        clean_groups(&mut self.groups);
        if !ONLINE_SECS.contains(&self.online_secs) {
            self.online_secs = default_online_secs();
        }
        let names = port_names(&self.groups);
        let mut old: std::collections::HashMap<u16, SwitchPort> = self.ports.drain(..).map(|p| (p.n, p)).collect();
        for (i, (name, _)) in names.into_iter().enumerate() {
            let n = i as u16 + 1;
            let mut p = old.remove(&n).unwrap_or_else(|| SwitchPort::new(n));
            if !matches!(p.mode.as_str(), "access" | "trunk" | "off") {
                p.mode = access();
            }
            p.tagged.sort_unstable();
            p.tagged.dedup();
            p.name = name;
            self.ports.push(p);
        }
    }
}

/// A faceplate to start from: built in, or saved by a user.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchModel {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub brand: String,
    pub model: String,
    pub groups: Vec<PortGroup>,
    /// Texts that recognise it in sysDescr or the ENTITY model (built-in models only).
    #[serde(default, skip_serializing)]
    pub matches: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Poll {
    pub at: i64,
    pub ok: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnmpAgent {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub ip: Ipv4Addr,
    #[serde(default = "snmp_port")]
    pub port: u16,
    #[serde(default)]
    pub community: String,
    /// SNMP v3 user (the community is then unused).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v3: Option<strata_net::snmpv3::V3>,
    #[serde(default)]
    pub read_arp: bool,
    #[serde(default)]
    pub read_multicast: bool,
    #[serde(default)]
    pub read_switch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_poll: Option<Poll>,
}

fn snmp_port() -> u16 {
    161
}

pub const MASKED: &str = "••••••••";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McastSettings {
    #[serde(default)]
    pub iface: String,
    #[serde(default = "check_max")]
    pub check_max_mbps: u64,
    /// Labels by "group:port:source".
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
    /// Group names, plan, alert rules, PTP changes (`mflows`).
    #[serde(flatten)]
    pub more: super::mflows::McastMore,
}

impl Default for McastSettings {
    fn default() -> Self {
        McastSettings { iface: String::new(), check_max_mbps: check_max(), labels: Default::default(), more: Default::default() }
    }
}

fn check_max() -> u64 {
    100
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub id: String,
    pub at: i64,
    pub kind: String,
    pub ip: Ipv4Addr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_id: Option<String>,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub seen: bool,
    /// Switch alerts: the switch, the faceplate port, and the measure with its limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub at: i64,
    pub user: String,
    pub action: String,
    pub target: String,
    #[serde(default)]
    pub detail: String,
}

pub const KINDS: &[&str] = &[
    "computer", "server", "laptop", "phone", "printer", "camera", "monitor", "audio", "video", "switch", "router", "firewall", "wifi", "storage", "iot", "other",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn switch(groups: Vec<PortGroup>) -> Switch {
        Switch {
            id: "a".into(),
            name: "sw".into(),
            model: String::new(),
            ip: None,
            group: String::new(),
            groups,
            copper: None,
            sfp: None,
            naming: None,
            sfp_naming: None,
            snmp: None,
            ports: vec![],
            online: false,
            online_secs: 10,
            rules: AlertRules::default(),
        }
    }

    #[test]
    fn faceplate_keeps_ports_and_names_them() {
        let mut s = switch(vec![PortGroup::new(PortKind::Rj45, 4, "Gi1/0/{n}"), PortGroup::new(PortKind::Sfp, 2, "Te1/1/{n}")]);
        s.ports = vec![SwitchPort { label: "PTZ".into(), ..SwitchPort::new(2) }];
        s.normalize();
        assert_eq!(s.ports.len(), 6);
        assert_eq!(s.ports[1].label, "PTZ");
        assert_eq!(s.ports[0].name, "Gi1/0/1");
        assert_eq!(s.ports[5].name, "Te1/1/2");
        s.groups[0].count = 2;
        s.normalize();
        assert_eq!(s.ports.len(), 4);
        assert_eq!(s.ports[3].name, "Te1/1/2");
    }

    #[test]
    fn old_switches_become_groups() {
        let mut s: Switch = serde_json::from_value(serde_json::json!({
            "id": "b", "name": "sw", "copper": 24, "sfp": 4, "naming": "Gi1/0/{n}", "sfpNaming": "Te1/1/{n}",
            "ports": [{"n": 3, "label": "PTZ", "mode": "access", "vlan": 20}, {"n": 26, "label": "uplink", "mode": "trunk", "tagged": [10, 20]}]
        }))
        .unwrap();
        assert!(s.migrate());
        assert_eq!(s.groups, vec![PortGroup::new(PortKind::Rj45, 24, "Gi1/0/{n}"), PortGroup::new(PortKind::Sfp, 4, "Te1/1/{n}")]);
        assert!(!s.migrate());
        s.normalize();
        assert_eq!(s.ports.len(), 28);
        assert_eq!((s.ports[2].label.as_str(), s.ports[2].vlan), ("PTZ", Some(20)));
        assert_eq!((s.ports[25].name.as_str(), s.ports[25].label.as_str()), ("Te1/1/2", "uplink"));
        // Written without the old fields; an empty SFP group is skipped.
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("copper").is_none() && v.get("sfpNaming").is_none());
        let mut c: Switch = serde_json::from_value(serde_json::json!({"name": "c", "copper": 8, "sfp": 0})).unwrap();
        c.normalize();
        assert_eq!(c.groups, vec![PortGroup::new(PortKind::Rj45, 8, "Gi1/0/{n}")]);
    }

    #[test]
    fn group_numbers_follow_on_with_the_same_naming() {
        let nexus = [PortGroup::new(PortKind::Sfp, 48, "Eth1/{n}"), PortGroup::new(PortKind::Qsfp, 12, "Eth1/{n}")];
        let names = port_names(&nexus);
        assert_eq!(names.len(), 60);
        assert_eq!(names[47], ("Eth1/48".to_string(), PortKind::Sfp));
        assert_eq!(names[48], ("Eth1/49".to_string(), PortKind::Qsfp));
        assert_eq!(names[59].0, "Eth1/60");
        let catalyst = [PortGroup::new(PortKind::Rj45, 48, "Gi1/0/{n}"), PortGroup::new(PortKind::Sfp, 4, "Te1/1/{n}")];
        assert_eq!(port_names(&catalyst)[48].0, "Te1/1/1");
        let juniper = [PortGroup { start: Some(0), ..PortGroup::new(PortKind::Rj45, 48, "ge-0/0/{n}") }, PortGroup { start: Some(0), ..PortGroup::new(PortKind::Sfp, 4, "xe-0/2/{n}") }];
        let j = port_names(&juniper);
        assert_eq!((j[0].0.as_str(), j[47].0.as_str(), j[48].0.as_str()), ("ge-0/0/0", "ge-0/0/47", "xe-0/2/0"));
        // A naming without {n} gets the number at the end; three groups with one naming count on.
        let netgear = [PortGroup::new(PortKind::Rj45, 24, "1/0/"), PortGroup::new(PortKind::Rj45, 2, "1/0/"), PortGroup::new(PortKind::Sfp, 4, "1/0/")];
        assert_eq!(port_names(&netgear)[29].0, "1/0/30");
        assert_eq!(group_starts(&[PortGroup::new(PortKind::Rj45, 4, "a{n}"), PortGroup { start: Some(9), ..PortGroup::new(PortKind::Sfp, 2, "a{n}") }, PortGroup::new(PortKind::Sfp, 2, "a{n}")]), vec![1, 9, 11]);
    }

    #[test]
    fn at_most_256_ports() {
        let mut s = switch(vec![PortGroup::new(PortKind::Rj45, 200, "{n}"), PortGroup::new(PortKind::Sfp, 100, "{n}"), PortGroup::new(PortKind::Qsfp, 4, "{n}")]);
        s.normalize();
        assert_eq!(s.port_count(), 256);
        assert_eq!(s.groups.len(), 2);
        assert_eq!(s.groups[1].count, 56);
        assert_eq!(s.ports.len(), 256);
        assert_eq!(s.ports[255].name, "256");
        let mut big = switch(vec![PortGroup::new(PortKind::Sfp, 256, "Eth1/{n}")]);
        big.normalize();
        assert_eq!(big.ports.len(), 256);
    }

    #[test]
    fn address_states() {
        let mut a = Address::new(Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(a.state(), "free");
        a.label = "PTZ".into();
        assert_eq!(a.state(), "reserved");
        a.last_seen = Some(1);
        assert_eq!(a.state(), "offline");
        a.up = true;
        assert_eq!(a.state(), "used");
    }
}
