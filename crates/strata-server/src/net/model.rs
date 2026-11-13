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
}

fn access() -> String {
    "access".into()
}

impl SwitchPort {
    pub fn new(n: u16) -> SwitchPort {
        SwitchPort { n, name: String::new(), label: String::new(), mode: access(), vlan: None, tagged: vec![], host: None, peer: None, learnt: vec![], lldp: None }
    }
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
    #[serde(default = "default_copper")]
    pub copper: u16,
    #[serde(default)]
    pub sfp: u16,
    #[serde(default = "default_naming")]
    pub naming: String,
    #[serde(default = "default_sfp_naming")]
    pub sfp_naming: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snmp: Option<String>,
    #[serde(default)]
    pub ports: Vec<SwitchPort>,
}

fn default_copper() -> u16 {
    24
}
fn default_naming() -> String {
    "Gi1/0/{n}".into()
}
fn default_sfp_naming() -> String {
    "Te1/1/{n}".into()
}

impl Switch {
    /// Makes `ports` match the faceplate: one entry per port, in order, names filled.
    pub fn normalize(&mut self) {
        self.copper = self.copper.min(96);
        self.sfp = self.sfp.min(48);
        let total = self.copper + self.sfp;
        let mut old: std::collections::HashMap<u16, SwitchPort> = self.ports.drain(..).map(|p| (p.n, p)).collect();
        for n in 1..=total {
            let mut p = old.remove(&n).unwrap_or_else(|| SwitchPort::new(n));
            if !matches!(p.mode.as_str(), "access" | "trunk" | "off") {
                p.mode = access();
            }
            p.tagged.sort_unstable();
            p.tagged.dedup();
            p.name = self.port_name(n);
            self.ports.push(p);
        }
    }

    pub fn port_name(&self, n: u16) -> String {
        if n <= self.copper {
            self.naming.replace("{n}", &n.to_string())
        } else {
            self.sfp_naming.replace("{n}", &(n - self.copper).to_string())
        }
    }
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
}

impl Default for McastSettings {
    fn default() -> Self {
        McastSettings { iface: String::new(), check_max_mbps: check_max(), labels: Default::default() }
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

    #[test]
    fn faceplate_keeps_ports_and_names_them() {
        let mut s = Switch {
            id: "a".into(),
            name: "sw".into(),
            model: String::new(),
            ip: None,
            group: String::new(),
            copper: 4,
            sfp: 2,
            naming: default_naming(),
            sfp_naming: default_sfp_naming(),
            snmp: None,
            ports: vec![SwitchPort { label: "PTZ".into(), ..SwitchPort::new(2) }],
        };
        s.normalize();
        assert_eq!(s.ports.len(), 6);
        assert_eq!(s.ports[1].label, "PTZ");
        assert_eq!(s.ports[0].name, "Gi1/0/1");
        assert_eq!(s.ports[5].name, "Te1/1/2");
        s.copper = 2;
        s.normalize();
        assert_eq!(s.ports.len(), 4);
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
